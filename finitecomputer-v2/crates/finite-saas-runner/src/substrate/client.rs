use super::protocol::{self as api, control_client::ControlClient};
use std::{path::PathBuf, time::Duration};
use tonic::{
    Code,
    transport::{Certificate, Channel, ClientTlsConfig, Endpoint},
};

/// Files are mounted by Kubernetes projected service-account and trust volumes, not stored by Core.
#[derive(Clone, Debug)]
pub struct SubstrateConnection {
    pub endpoint: String,
    pub server_name: String,
    pub ca_file: PathBuf,
    pub token_file: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum SubstrateError {
    #[error("Substrate credential file could not be read")]
    Credentials(#[source] std::io::Error),
    #[error("Substrate requires an HTTPS endpoint and a TLS server name")]
    Configuration,
    #[error("Substrate transport failed")]
    Transport(#[from] tonic::transport::Error),
    // RPC descriptions may contain an echoed template, including secrets.
    #[error("Substrate {operation} failed ({code})")]
    Rpc { operation: &'static str, code: Code },
    #[error("Substrate existing resource conflicts with the requested identity")]
    Conflict,
}

/// Stateless control API access. Reconnect for each lifecycle operation so
/// rotated pod credentials are read again without an independent refresh loop.
pub struct SubstrateClient {
    connection: SubstrateConnection,
}

impl SubstrateClient {
    pub fn new(connection: SubstrateConnection) -> Result<Self, SubstrateError> {
        if !connection.endpoint.starts_with("https://") || connection.server_name.is_empty() {
            return Err(SubstrateError::Configuration);
        }
        Ok(Self { connection })
    }

    async fn connect(
        &self,
    ) -> Result<
        ControlClient<
            tonic::service::interceptor::InterceptedService<
                Channel,
                impl tonic::service::Interceptor,
            >,
        >,
        SubstrateError,
    > {
        let ca = std::fs::read(&self.connection.ca_file).map_err(SubstrateError::Credentials)?;
        let token = std::fs::read_to_string(&self.connection.token_file)
            .map_err(SubstrateError::Credentials)?;
        let mut authorization: tonic::metadata::MetadataValue<_> =
            format!("Bearer {}", token.trim())
                .parse()
                .map_err(|_| SubstrateError::Configuration)?;
        authorization.set_sensitive(true);
        let tls = ClientTlsConfig::new()
            .domain_name(&self.connection.server_name)
            .ca_certificate(Certificate::from_pem(ca));
        let channel = Endpoint::from_shared(self.connection.endpoint.clone())?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .tls_config(tls)?
            .connect()
            .await?;
        Ok(
            ControlClient::with_interceptor(channel, move |mut request: tonic::Request<()>| {
                request
                    .metadata_mut()
                    .insert("authorization", authorization.clone());
                Ok(request)
            })
            .max_decoding_message_size(4 * 1024 * 1024)
            .max_encoding_message_size(4 * 1024 * 1024),
        )
    }

    pub async fn actors(
        &self,
        atespace: &str,
        page_token: String,
    ) -> Result<api::ListActorsResponse, SubstrateError> {
        self.connect()
            .await?
            .list_actors(api::ListActorsRequest {
                atespace: atespace.into(),
                page_size: 1000,
                page_token,
            })
            .await
            .map(|response| response.into_inner())
            .map_err(|status| rpc("list actors", status))
    }

    pub async fn actor(&self, actor: api::ObjectRef) -> Result<Option<api::Actor>, SubstrateError> {
        match self
            .connect()
            .await?
            .get_actor(api::GetActorRequest { actor: Some(actor) })
            .await
        {
            Ok(response) => Ok(Some(response.into_inner())),
            Err(status) if status.code() == Code::NotFound => Ok(None),
            Err(status) => Err(rpc("get actor", status)),
        }
    }

    /// An ambiguous create result is recovered by reading the same immutable
    /// name. Never generate a second actor name after a timeout.
    pub async fn ensure_actor(&self, actor: api::Actor) -> Result<api::Actor, SubstrateError> {
        let metadata = actor
            .metadata
            .as_ref()
            .ok_or(SubstrateError::Configuration)?;
        let reference = api::ObjectRef {
            atespace: metadata.atespace.clone(),
            name: metadata.name.clone(),
        };
        match self
            .connect()
            .await?
            .create_actor(api::CreateActorRequest {
                actor: Some(actor.clone()),
            })
            .await
        {
            Ok(response) => Ok(response.into_inner()),
            Err(status) if status.code() == Code::AlreadyExists => {
                let existing = self
                    .actor(reference)
                    .await?
                    .ok_or(SubstrateError::Conflict)?;
                if existing.actor_template != actor.actor_template
                    || existing.source_tag != actor.source_tag
                    || existing.worker_selector != actor.worker_selector
                {
                    return Err(SubstrateError::Conflict);
                }
                Ok(existing)
            }
            Err(status) => Err(rpc("create actor", status)),
        }
    }

    pub async fn template(
        &self,
        template: api::ObjectRef,
    ) -> Result<Option<api::ActorTemplate>, SubstrateError> {
        match self
            .connect()
            .await?
            .get_actor_template(api::GetActorTemplateRequest {
                actor_template: Some(template),
            })
            .await
        {
            Ok(response) => Ok(Some(response.into_inner())),
            Err(status) if status.code() == Code::NotFound => Ok(None),
            Err(status) => Err(rpc("get template", status)),
        }
    }

    pub async fn ensure_template(
        &self,
        template: api::ActorTemplate,
    ) -> Result<api::ActorTemplate, SubstrateError> {
        let metadata = template
            .metadata
            .as_ref()
            .ok_or(SubstrateError::Configuration)?;
        let reference = api::ObjectRef {
            atespace: metadata.atespace.clone(),
            name: metadata.name.clone(),
        };
        match self
            .connect()
            .await?
            .create_actor_template(api::CreateActorTemplateRequest {
                actor_template: Some(template.clone()),
            })
            .await
        {
            Ok(response) => Ok(response.into_inner()),
            Err(status) if status.code() == Code::AlreadyExists => {
                let existing = self
                    .template(reference)
                    .await?
                    .ok_or(SubstrateError::Conflict)?;
                let mut comparable = existing.clone();
                comparable.metadata = template.metadata.clone();
                comparable.status = template.status.clone();
                if comparable != template {
                    return Err(SubstrateError::Conflict);
                }
                Ok(existing)
            }
            Err(status) => Err(rpc("create template", status)),
        }
    }

    /// Preserve an existing operator policy; never silently broaden it on retry.
    pub async fn ensure_egress(
        &self,
        actor: api::ObjectRef,
        hosts: Vec<String>,
        cidrs: Vec<String>,
    ) -> Result<(), SubstrateError> {
        if (hosts.is_empty() && cidrs.is_empty())
            || hosts.iter().any(|host| host.is_empty() || host == "*")
            || cidrs
                .iter()
                .any(|cidr| cidr.is_empty() || cidr.ends_with("/0"))
        {
            return Err(SubstrateError::Configuration);
        }
        let policy = api::EgressPolicy {
            metadata: Some(api::ResourceMetadata {
                atespace: actor.atespace.clone(),
                name: "default".into(),
                ..Default::default()
            }),
            rules: [
                (!hosts.is_empty()).then_some(api::EgressRule {
                    hostnames: Some(api::HostnameRule {
                        patterns: hosts,
                        effects: None,
                    }),
                    ..Default::default()
                }),
                (!cidrs.is_empty()).then_some(api::EgressRule {
                    cidrs: Some(api::CidrRule { cidrs }),
                    ..Default::default()
                }),
            ]
            .into_iter()
            .flatten()
            .collect(),
        };
        match self
            .connect()
            .await?
            .create_actor_egress_policy(api::CreateActorEgressPolicyRequest {
                actor: Some(actor),
                egress_policy: Some(policy),
            })
            .await
        {
            Ok(_) => Ok(()),
            Err(status) if status.code() == Code::AlreadyExists => Ok(()),
            Err(status) => Err(rpc("create egress policy", status)),
        }
    }

    /// Replace only the template reference using the observed actor UID/version.
    /// Upstream updates replace the whole object, so retain other fields. Resolve
    /// a lost response by reading the exact actor, never repeating stale metadata.
    pub async fn replace_template(
        &self,
        mut actor: api::Actor,
        template: api::ObjectRef,
    ) -> Result<api::Actor, SubstrateError> {
        actor.actor_template = Some(template);
        let metadata = actor
            .metadata
            .as_ref()
            .ok_or(SubstrateError::Configuration)?;
        let reference = api::ObjectRef {
            atespace: metadata.atespace.clone(),
            name: metadata.name.clone(),
        };
        match self
            .connect()
            .await?
            .update_actor(api::UpdateActorRequest {
                actor: Some(actor.clone()),
            })
            .await
        {
            Ok(response) => Ok(response.into_inner()),
            Err(status) => {
                if matches!(
                    status.code(),
                    Code::Unknown
                        | Code::Internal
                        | Code::Unavailable
                        | Code::DeadlineExceeded
                        | Code::Cancelled
                ) && let Ok(Some(observed)) = self.actor(reference).await
                    && observed
                        .metadata
                        .as_ref()
                        .is_some_and(|value| value.uid == metadata.uid)
                    && observed.actor_template == actor.actor_template
                    && observed.worker_selector == actor.worker_selector
                    && observed.source_tag == actor.source_tag
                {
                    return Ok(observed);
                }
                Err(rpc("replace actor template", status))
            }
        }
    }

    pub async fn resume(&self, actor: api::ObjectRef) -> Result<api::Actor, SubstrateError> {
        let mut client = self.connect().await?;
        let mut capacity_retries = 0;
        loop {
            match client
                .resume_actor(api::ResumeActorRequest {
                    actor: Some(actor.clone()),
                })
                .await
            {
                Ok(response) => return response.into_inner().actor.ok_or(SubstrateError::Conflict),
                // A replacement worker can register just after CRASHED becomes
                // visible. Allow at most 30 one-second capacity retries; other
                // failures (including ambiguous transport errors) remain caller-owned.
                Err(status)
                    if status.code() == Code::ResourceExhausted && capacity_retries < 30 =>
                {
                    capacity_retries += 1;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(status) => return Err(rpc("resume actor", status)),
            }
        }
    }

    pub async fn revert(&self, actor: api::ObjectRef) -> Result<api::Actor, SubstrateError> {
        self.connect()
            .await?
            .revert_actor(api::RevertActorRequest { actor: Some(actor) })
            .await
            .map_err(|status| rpc("revert actor execution", status))?
            .into_inner()
            .actor
            .ok_or(SubstrateError::Conflict)
    }

    /// Suspending retains durable data. Actor deletion also deletes its volumes
    /// upstream, so deletion is deliberately absent from this lifecycle client.
    pub async fn stop(&self, actor: api::ObjectRef) -> Result<api::Actor, SubstrateError> {
        self.connect()
            .await?
            .suspend_actor(api::SuspendActorRequest { actor: Some(actor) })
            .await
            .map_err(|status| rpc("suspend actor", status))?
            .into_inner()
            .actor
            .ok_or(SubstrateError::Conflict)
    }
}

fn rpc(operation: &'static str, status: tonic::Status) -> SubstrateError {
    SubstrateError::Rpc {
        operation,
        code: status.code(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_errors_do_not_disclose_echoed_credentials() {
        let error = rpc(
            "create actor",
            tonic::Status::invalid_argument("secret credential payload"),
        );
        assert_eq!(
            error.to_string(),
            "Substrate create actor failed (Client specified an invalid argument)"
        );
        assert!(!format!("{error:?}").contains("secret credential payload"));
    }
}

#[cfg(test)]
mod integration {
    use super::*;

    fn required(key: &str) -> String {
        std::env::var(key).expect(key)
    }

    fn test_client() -> SubstrateClient {
        SubstrateClient::new(SubstrateConnection {
            endpoint: required("FC_TEST_SUBSTRATE_ENDPOINT"),
            server_name: "api.ate-system.svc".into(),
            ca_file: required("FC_TEST_SUBSTRATE_CA_FILE").into(),
            token_file: required("FC_TEST_SUBSTRATE_TOKEN_FILE").into(),
        })
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires a disposable Substrate API and projected credentials"]
    async fn real_substrate_authenticated_read() {
        let client = test_client();
        let actor = client
            .actor(api::ObjectRef {
                atespace: required("FC_TEST_SUBSTRATE_ATESPACE"),
                name: required("FC_TEST_SUBSTRATE_ACTOR"),
            })
            .await
            .unwrap()
            .expect("fixture actor must exist");
        assert!(!actor.metadata.unwrap().uid.is_empty());
        assert!(actor.status.is_some());
    }
    #[tokio::test]
    #[ignore = "repoints an explicitly named suspended disposable actor, then restores its template"]
    async fn real_substrate_template_update_preserves_identity_and_rejects_stale_version() {
        let client = test_client();
        let reference = api::ObjectRef {
            atespace: required("FC_TEST_SUBSTRATE_ATESPACE"),
            name: required("FC_TEST_SUBSTRATE_ACTOR"),
        };
        let before = client.actor(reference.clone()).await.unwrap().unwrap();
        assert_eq!(
            before.status.as_ref().unwrap().state(),
            api::ActorState::Suspended
        );
        let original = before.actor_template.clone().unwrap();
        let target = api::ObjectRef {
            atespace: reference.atespace.clone(),
            name: format!("{}-cas-proof", reference.name),
        };
        let mut template = client.template(original.clone()).await.unwrap().unwrap();
        template.metadata = Some(api::ResourceMetadata {
            atespace: target.atespace.clone(),
            name: target.name.clone(),
            ..Default::default()
        });
        template.status = None;
        client.ensure_template(template).await.unwrap();
        let updated = client
            .replace_template(before.clone(), target.clone())
            .await
            .unwrap();
        // Attempt a real lost-update with the old resource version. Restore the
        // fixture using a fresh read before asserting results, even on regression.
        let stale = client
            .replace_template(before.clone(), original.clone())
            .await;
        let observed = client.actor(reference.clone()).await.unwrap().unwrap();
        let restored = client
            .replace_template(observed.clone(), original.clone())
            .await
            .unwrap();
        assert!(matches!(
            stale,
            Err(SubstrateError::Rpc {
                code: Code::Aborted,
                ..
            })
        ));
        assert_eq!(observed.actor_template, Some(target));
        assert_eq!(restored.actor_template, Some(original));
        for actor in [&updated, &observed, &restored] {
            assert_eq!(
                actor.metadata.as_ref().unwrap().uid,
                before.metadata.as_ref().unwrap().uid
            );
            assert_eq!(actor.worker_selector, before.worker_selector);
            assert_eq!(actor.source_tag, before.source_tag);
            let status = actor.status.as_ref().unwrap();
            assert_eq!(status.state(), api::ActorState::Suspended);
            assert_eq!(
                status.actor_volumes,
                before.status.as_ref().unwrap().actor_volumes
            );
        }
    }

    #[tokio::test]
    #[ignore = "mutates two explicitly named disposable actors; requires all worker capacity occupied"]
    async fn real_substrate_resume_waits_for_capacity() {
        let client = test_client();
        let target = api::ObjectRef {
            atespace: required("FC_TEST_SUBSTRATE_ATESPACE"),
            name: required("FC_TEST_SUBSTRATE_ACTOR"),
        };
        let holder = api::ObjectRef {
            atespace: target.atespace.clone(),
            name: required("FC_TEST_SUBSTRATE_CAPACITY_RELEASE_ACTOR"),
        };
        assert_ne!(target, holder);
        let before = client.actor(target.clone()).await.unwrap().unwrap();
        assert_eq!(
            before.status.as_ref().unwrap().state(),
            api::ActorState::Suspended
        );
        // Prove the actual provider refuses capacity before testing the retry.
        let refusal = client
            .connect()
            .await
            .unwrap()
            .resume_actor(api::ResumeActorRequest {
                actor: Some(target.clone()),
            })
            .await
            .unwrap_err();
        assert_eq!(refusal.code(), Code::ResourceExhausted);
        let (resumed, stopped) = tokio::join!(client.resume(target), async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            client.stop(holder).await
        });
        assert_eq!(
            stopped.unwrap().status.unwrap().state(),
            api::ActorState::Suspended
        );
        let resumed = resumed.unwrap();
        assert_eq!(resumed.metadata.unwrap().uid, before.metadata.unwrap().uid);
        assert_eq!(resumed.status.unwrap().state(), api::ActorState::Running);
    }
}
