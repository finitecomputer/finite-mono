//! Core-owned bearer bootstrap for an exact runtime assignment.
//! Bootstrap is private launch material, never part of RuntimeSpec or runtime views.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// No Debug: launch credentials must not become diagnostic data.
pub struct ProvisionRuntimeCredential {
    pub creation_request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    /// Derived from the authenticated Runner, not request JSON.
    pub source_host_id: String,
}

// Serialize only for the dedicated, authenticated provisioning response.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBootstrapCredential {
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedRuntime {
    pub agent_runtime_id: String,
    pub project_id: String,
    pub creation_request_id: String,
}

pub(super) fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub(super) fn hex_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub(crate) fn new_secret() -> CoreResult<String> {
    let mut bytes = [0; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| CoreError::Store("runtime credential generation failed".into()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn validate_runtime_core_url(value: &str) -> CoreResult<String> {
    let url = reqwest::Url::parse(value).map_err(|_| CoreError::RuntimeSpecMismatch)?;
    let local = url
        .host_str()
        .is_some_and(|host| host == "localhost" || host == "127.0.0.1" || host == "[::1]");
    if (url.scheme() != "https" && !(url.scheme() == "http" && local))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

impl CoreStore {
    /// Core returns the same secret for retries of this creation, including a
    /// new live lease on the same host. It never rotates an installed secret as
    /// a side effect of retry. A revoked/replaced creation cannot be re-enrolled.
    pub async fn provision_runtime_credential(
        &self,
        input: ProvisionRuntimeCredential,
    ) -> CoreResult<RuntimeBootstrapCredential> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let request = locked_agent_creation_request(&*tx, &input.creation_request_id).await?;
        verify_agent_creation_lease_active(&*tx, &request, &input.runner_id, &input.lease_token)
            .await?;
        if request.relocation.is_some()
            || request
                .target_source_host_id
                .as_deref()
                .is_some_and(|host| host != input.source_host_id)
        {
            return Err(CoreError::ProviderOperationIdentityMismatch);
        }
        if let Some(runtime_id) = request.agent_runtime_id.as_deref() {
            let row = tx
                .query_opt(
                    "SELECT source_host_id FROM agent_runtimes WHERE id=$1 FOR UPDATE",
                    &[&runtime_id],
                )
                .await
                .map_err(store_error)?;
            if let Some(row) = row {
                if row.get::<_, String>(0) != input.source_host_id {
                    return Err(CoreError::ProviderOperationIdentityMismatch);
                }
                check_initial_runtime(&*tx, &request.project_id, runtime_id).await?;
            }
        }
        // Lock before wall-clock validation, even if only the pending row exists.
        let existing = tx.query_opt("SELECT bootstrap_secret, source_host_id, revoked FROM runtime_core_credentials WHERE creation_request_id=$1 FOR UPDATE", &[&request.id]).await.map_err(store_error)?;
        ensure_live_now(&*tx, &request.id).await?;
        let lease_hash = digest(
            request
                .lease_token
                .as_deref()
                .ok_or(CoreError::AgentCreationRequestLeaseConflict)?,
        );
        let secret = match existing {
            Some(row) => {
                if row.get::<_, bool>(2) || row.get::<_, String>(1) != input.source_host_id {
                    return Err(CoreError::ProviderOperationTransitionConflict);
                }
                let secret: String = row.get(0);
                tx.execute("UPDATE runtime_core_credentials SET lease_sha256=$2 WHERE creation_request_id=$1", &[&request.id, &lease_hash]).await.map_err(store_error)?;
                secret
            }
            None => {
                let secret = new_secret()?;
                tx.execute("INSERT INTO runtime_core_credentials (creation_request_id,source_host_id,bootstrap_secret,token_sha256,lease_sha256,owner_user_id) SELECT $1,$2,$3,$4,$5,p.owner_user_id FROM projects p JOIN agent_creation_requests q ON q.project_id=p.id WHERE q.id=$1", &[&request.id,&input.source_host_id,&secret,&digest(&secret),&lease_hash]).await.map_err(store_error)?;
                secret
            }
        };
        // If provisioning is retried after runtime registration, bind it now.
        bind_bootstrap(&*tx, &request.id).await?;
        self.finish(tx).await?;
        Ok(RuntimeBootstrapCredential { secret })
    }

    /// Observational reader. Mutation consumers use the transactional method below.
    pub async fn authenticate_runtime_credential(
        &self,
        secret: &str,
    ) -> CoreResult<Option<AuthenticatedRuntime>> {
        if !hex_secret(secret) {
            return Ok(None);
        }
        let client = self.connection().await?;
        authenticated(&**client, secret).await
    }

    pub async fn revoke_runtime_credential(
        &self,
        runtime_id: &str,
        expected_creation_request_id: &str,
    ) -> CoreResult<bool> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let changed = tx.execute("UPDATE runtime_core_credentials SET revoked=TRUE WHERE agent_runtime_id=$1 AND creation_request_id=$2 AND NOT revoked", &[&runtime_id,&expected_creation_request_id]).await.map_err(store_error)?;
        self.finish(tx).await?;
        Ok(changed == 1)
    }
}

async fn ensure_live_now<C: GenericClient + Sync>(client: &C, request_id: &str) -> CoreResult<()> {
    let live: bool = client.query_one("SELECT COALESCE(lease_expires_at>clock_timestamp(),FALSE) FROM agent_creation_requests WHERE id=$1", &[&request_id]).await.map_err(store_error)?.get(0);
    if !live {
        return Err(CoreError::AgentCreationRequestLeaseConflict);
    }
    Ok(())
}
async fn check_initial_runtime<C: GenericClient + Sync>(
    client: &C,
    project_id: &str,
    runtime_id: &str,
) -> CoreResult<()> {
    let allowed: bool = client.query_one("SELECT EXISTS(SELECT 1 FROM project_runtime_links WHERE project_id=$1 AND agent_runtime_id=$2 AND active) AND NOT EXISTS(SELECT 1 FROM runtime_control_requests WHERE agent_runtime_id=$2)", &[&project_id,&runtime_id]).await.map_err(store_error)?.get(0);
    if !allowed {
        return Err(CoreError::ProviderOperationTransitionConflict);
    }
    Ok(())
}

/// The creation row is locked by the caller. Validate submitted source facts
/// before runtime upsert, which preserves existing source fields on conflict.
pub(super) async fn validate_bootstrap_source<C: GenericClient + Sync>(
    client: &C,
    request_id: &str,
    host: &str,
    machine: &str,
) -> CoreResult<()> {
    let source = client.query_opt(
        "SELECT source_host_id,source_machine_id FROM runtime_core_credentials WHERE creation_request_id=$1",
        &[&request_id],
    ).await.map_err(store_error)?;
    if let Some(row) = source
        && (row.get::<_, String>(0) != host
            || row
                .get::<_, Option<String>>(1)
                .is_some_and(|value| value != machine))
    {
        return Err(CoreError::ProviderOperationIdentityMismatch);
    }
    Ok(())
}

/// Called by the actual runtime-registration and completion transactions.
/// Old launchers have no bootstrap row; those paths remain unchanged.
pub(super) async fn bind_bootstrap<C: GenericClient + Sync>(
    client: &C,
    request_id: &str,
) -> CoreResult<()> {
    let row = client
        .query_opt(
            "SELECT r.id,r.project_id,r.source_host_id,r.source_machine_id,q.lease_token
         FROM agent_creation_requests q JOIN agent_runtimes r ON r.id=q.agent_runtime_id
         WHERE q.id=$1",
            &[&request_id],
        )
        .await
        .map_err(store_error)?;
    let Some(row) = row else {
        return Ok(());
    };
    let runtime: String = row.get(0);
    client
        .query_one(
            "SELECT id FROM agent_runtimes WHERE id=$1 FOR UPDATE",
            &[&runtime],
        )
        .await
        .map_err(store_error)?;
    let origin = client.query_opt("SELECT source_host_id,lease_sha256,revoked,agent_runtime_id,source_machine_id FROM runtime_core_credentials WHERE creation_request_id=$1 FOR UPDATE", &[&request_id]).await.map_err(store_error)?;
    let Some(origin) = origin else {
        return Ok(());
    };
    if origin
        .get::<_, Option<String>>(3)
        .is_some_and(|id| id != runtime)
        || origin
            .get::<_, Option<String>>(4)
            .is_some_and(|machine| machine != row.get::<_, String>(3))
    {
        return Err(CoreError::ProviderOperationIdentityMismatch);
    }
    let lease: Option<String> = row.get(4);
    if origin.get::<_, bool>(2)
        || origin.get::<_, String>(0) != row.get::<_, String>(2)
        || lease.as_deref().map(digest).as_deref() != Some(origin.get::<_, String>(1).as_str())
    {
        return Err(CoreError::ProviderOperationTransitionConflict);
    }
    check_initial_runtime(client, &row.get::<_, String>(1), &runtime).await?;
    ensure_live_now(client, request_id).await?;
    client.execute("UPDATE runtime_core_credentials SET agent_runtime_id=$2,source_machine_id=$3 WHERE creation_request_id=$1", &[&request_id,&runtime,&row.get::<_, String>(3)]).await.map_err(store_error)?;
    Ok(())
}
pub(super) async fn authenticated<C: GenericClient + Sync>(
    client: &C,
    secret: &str,
) -> CoreResult<Option<AuthenticatedRuntime>> {
    let row = client.query_opt(
        "SELECT r.id,r.project_id,q.id,q.status,q.lease_token,c.lease_sha256 FROM runtime_core_credentials c
         JOIN agent_runtimes r ON r.id=c.agent_runtime_id
         JOIN agent_creation_requests q ON q.id=c.creation_request_id
         JOIN projects p ON p.id=r.project_id AND p.owner_user_id=c.owner_user_id
         WHERE c.token_sha256=$1 AND NOT c.revoked
           AND q.agent_runtime_id=r.id AND q.project_id=r.project_id
           AND c.source_host_id=r.source_host_id AND c.source_machine_id=r.source_machine_id
           AND EXISTS(SELECT 1 FROM project_runtime_links l WHERE l.project_id=r.project_id AND l.agent_runtime_id=r.id AND l.active)
           AND ((q.status='launching' AND q.lease_expires_at>clock_timestamp()
                 AND q.lease_token IS NOT NULL)
                OR (q.status='running' AND c.activated))",
        &[&digest(secret)],
    ).await.map_err(store_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.get::<_, String>(3) == "launching" {
        let lease: Option<String> = row.get(4);
        if lease.as_deref().map(digest).as_deref() != Some(row.get::<_, String>(5).as_str()) {
            return Ok(None);
        }
    }
    Ok(Some(AuthenticatedRuntime {
        agent_runtime_id: row.get(0),
        project_id: row.get(1),
        creation_request_id: row.get(2),
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_support::{TestDb, with_isolated_postgres};
    use crate::{RuntimeCapabilitiesEnvelope, RuntimeCapabilitiesV1};
    pub(crate) async fn requested(db: &TestDb) -> String {
        let code = db
            .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "runtime bootstrap".into(),
                code_count: 1,
                expires_in_hours: Some(24),
                hosting_tier: None,
                created_by_workos_user_id: "operator".into(),
                now: None,
            })
            .await
            .unwrap()
            .codes[0]
            .code
            .clone();
        let request = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "runtime-auth@finite.test".into(),
                workos_user_id: "runtime-auth-user".into(),
                display_name: "Runtime auth".into(),
                launch_code: code,
                idempotency_key: "runtime-auth-create".into(),
                now: None,
            })
            .await
            .unwrap()
            .request;
        db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: "auth-runner".into(),
            source_host_id: Some("auth-host".into()),
            lease_token: "test-launch-lease".into(),
            lease_seconds: Some(300),
            runner_capacity: None,
            now: None,
        })
        .await
        .unwrap()
        .unwrap();
        request.id
    }
    pub(crate) fn provision(request: &str) -> ProvisionRuntimeCredential {
        ProvisionRuntimeCredential {
            creation_request_id: request.into(),
            runner_id: "auth-runner".into(),
            source_host_id: "auth-host".into(),
            lease_token: "test-launch-lease".into(),
        }
    }
    pub(crate) async fn register(db: &TestDb, request: &str) -> String {
        register_named(db, request, "auth-machine")
            .await
            .unwrap()
            .request
            .agent_runtime_id
            .unwrap()
    }
    async fn register_named(
        db: &TestDb,
        request: &str,
        machine: &str,
    ) -> CoreResult<AgentCreationLease> {
        db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
            request_id: request.into(),
            runner_id: "auth-runner".into(),
            lease_token: "test-launch-lease".into(),
            source_host_id: "auth-host".into(),
            source_machine_id: machine.into(),
            runtime_artifact_id: Some("artifact-postgres-fixture".into()),
            state_schema_version: Some("state-v1".into()),
            provider_runtime_handle: None,
            contact_endpoint: None,
            runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                restart: true,
                stop: true,
                ..Default::default()
            })),
            display_name: None,
            hostname: None,
            runtime_host: None,
            runtime_status: None,
            active_inference_profile: None,
            hermes_available: None,
            published_app_urls: vec![],
            now: None,
        })
        .await
    }
    pub(crate) async fn complete(db: &TestDb, request: &str) -> CoreResult<AgentCreationLease> {
        db.complete_agent_creation_request(CompleteAgentCreationRequestInput {
            request_id: request.into(),
            runner_id: "auth-runner".into(),
            lease_token: "test-launch-lease".into(),
            source_host_id: "auth-host".into(),
            source_machine_id: "auth-machine".into(),
            runtime_artifact_id: Some("artifact-postgres-fixture".into()),
            state_schema_version: Some("state-v1".into()),
            provider_runtime_handle: None,
            contact_endpoint: None,
            runtime_capabilities: None,
            display_name: None,
            hostname: None,
            runtime_host: None,
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: None,
            hermes_available: None,
            published_app_urls: vec![],
            agent_npub: None,
            now: None,
        })
        .await
    }
    async fn expire(db: &TestDb, request: &str) {
        db.connection().await.unwrap().execute("UPDATE agent_creation_requests SET lease_expires_at=clock_timestamp()-INTERVAL '1 second' WHERE id=$1", &[&request]).await.unwrap();
    }
    #[tokio::test]
    async fn runtime_bootstrap_direct_completion_and_immutable_binding() {
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let secret = db
                .provision_runtime_credential(provision(&request))
                .await
                .unwrap()
                .secret;
            complete(&db, &request).await.unwrap();
            assert!(
                db.authenticate_runtime_credential(&secret)
                    .await
                    .unwrap()
                    .is_some()
            );
        })
        .await;
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let secret = db
                .provision_runtime_credential(provision(&request))
                .await
                .unwrap()
                .secret;
            let runtime = register(&db, &request).await;
            assert!(
                register_named(&db, &request, "different-machine")
                    .await
                    .is_err()
            );
            assert_eq!(
                db.authenticate_runtime_credential(&secret)
                    .await
                    .unwrap()
                    .unwrap()
                    .agent_runtime_id,
                runtime
            );
        })
        .await;
    }
    #[tokio::test]
    async fn runtime_bootstrap_origin_precedes_runtime_and_replays_after_reconnect() {
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let secret = db
                .provision_runtime_credential(provision(&request))
                .await
                .unwrap()
                .secret;
            assert!(
                db.authenticate_runtime_credential(&secret)
                    .await
                    .unwrap()
                    .is_none()
            );
            let runtime = register(&db, &request).await;
            let mut padded = provision(&request);
            padded.lease_token = "  test-launch-lease\n".into();
            assert!(
                db.provision_runtime_credential(padded)
                    .await
                    .unwrap()
                    .secret
                    == secret
            );
            assert!(
                db.provision_runtime_credential(provision(&request))
                    .await
                    .unwrap()
                    .secret
                    == secret
            );
            assert_eq!(
                db.authenticate_runtime_credential(&secret)
                    .await
                    .unwrap()
                    .unwrap()
                    .agent_runtime_id,
                runtime
            );
            complete(&db, &request).await.unwrap();
            assert!(
                db.authenticate_runtime_credential(&secret)
                    .await
                    .unwrap()
                    .is_some()
            );
            assert!(
                db.provision_runtime_credential(provision(&request))
                    .await
                    .is_err()
            );
        })
        .await;
    }
    #[tokio::test]
    async fn runtime_bootstrap_rejects_wrong_host_lease_and_expired_completion() {
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let secret = db
                .provision_runtime_credential(provision(&request))
                .await
                .unwrap()
                .secret;
            let mut wrong = provision(&request);
            wrong.source_host_id = "other-host".into();
            assert!(db.provision_runtime_credential(wrong).await.is_err());
            let mut wrong = provision(&request);
            wrong.lease_token = "wrong".into();
            assert!(db.provision_runtime_credential(wrong).await.is_err());
            register(&db, &request).await;
            expire(&db, &request).await;
            assert!(
                db.provision_runtime_credential(provision(&request))
                    .await
                    .is_err()
            );
            assert!(complete(&db, &request).await.is_err());
            assert!(
                db.authenticate_runtime_credential(&secret)
                    .await
                    .unwrap()
                    .is_none()
            );
        })
        .await;
    }
    #[tokio::test]
    async fn runtime_bootstrap_live_takeover_recovers_same_installed_secret() {
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let secret = db
                .provision_runtime_credential(provision(&request))
                .await
                .unwrap()
                .secret;
            register(&db, &request).await;
            expire(&db, &request).await;
            db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "auth-runner".into(),
                source_host_id: Some("auth-host".into()),
                lease_token: "new-lease".into(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
            assert!(
                db.authenticate_runtime_credential(&secret)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                db.provision_runtime_credential(provision(&request))
                    .await
                    .is_err()
            );
            let mut input = provision(&request);
            input.lease_token = "new-lease".into();
            assert!(db.provision_runtime_credential(input).await.unwrap().secret == secret);
            assert!(
                db.authenticate_runtime_credential(&secret)
                    .await
                    .unwrap()
                    .is_some()
            );
        })
        .await;
    }
    #[tokio::test]
    async fn runtime_bootstrap_offboarding_both_orders_and_control_fence() {
        for enroll in [true, false] {
            with_isolated_postgres(|db| async move {
                let request = requested(&db).await;
                let runtime = register(&db, &request).await;
                let secret = if enroll {
                    Some(
                        db.provision_runtime_credential(provision(&request))
                            .await
                            .unwrap()
                            .secret,
                    )
                } else {
                    None
                };
                let project_id = db.agent_runtime(&runtime).await.unwrap().project_id;
                db.admin_archive_unrecoverable_runtime(AdminArchiveUnrecoverableRuntimeInput {
                    admin_verified_email: "operator@finite.test".into(),
                    admin_workos_user_id: "operator".into(),
                    project_id,
                    expected_agent_runtime_id: runtime,
                    expected_source_host_id: "auth-host".into(),
                    expected_source_machine_id: "auth-machine".into(),
                    expected_owner_email: "runtime-auth@finite.test".into(),
                    operator_observed_compute_absent: true,
                    operator_observed_durable_state_absent: true,
                    owner_acknowledged_unrecoverable: true,
                    now: None,
                })
                .await
                .unwrap();
                assert!(
                    db.provision_runtime_credential(provision(&request))
                        .await
                        .is_err()
                );
                if let Some(secret) = secret {
                    assert!(
                        db.authenticate_runtime_credential(&secret)
                            .await
                            .unwrap()
                            .is_none()
                    );
                }
            })
            .await;
        }
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let runtime = register(&db, &request).await;
            db.request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "runtime-auth@finite.test".into(),
                workos_user_id: "runtime-auth-user".into(),
                project_id: db.agent_runtime(&runtime).await.unwrap().project_id,
                now: None,
            })
            .await
            .unwrap();
            assert!(
                db.provision_runtime_credential(provision(&request))
                    .await
                    .is_err()
            );
        })
        .await;
    }
    #[tokio::test]
    async fn creation_credential_survives_restart_and_stop_resume_without_reviving_revocation() {
        use crate::{RunnerClass, RunnerLeaseCapacity};
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let secret = db
                .provision_runtime_credential(provision(&request))
                .await
                .unwrap()
                .secret;
            let runtime = register(&db, &request).await;
            let project = complete(&db, &request).await.unwrap().project.id;
            for (index, stop) in [false, true, false, false].into_iter().enumerate() {
                // Explicitly revoke before the final restart: lifecycle success
                // must never resurrect a credential revoked by an operator.
                if index == 3 {
                    db.revoke_runtime_credential(&runtime, &request)
                        .await
                        .unwrap();
                }
                let input = RequestRuntimeRestartInput {
                    verified_email: "runtime-auth@finite.test".into(),
                    workos_user_id: "runtime-auth-user".into(),
                    project_id: project.clone(),
                    now: None,
                };
                let operation = if stop {
                    db.request_runtime_stop(input).await.unwrap()
                } else {
                    db.request_runtime_restart(input).await.unwrap()
                };
                let lease = db
                    .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                        runner_id: "auth-runner".into(),
                        lease_token: "control-lease".into(),
                        lease_seconds: Some(300),
                        source_host_id: Some("auth-host".into()),
                        runner_capacity: Some(RunnerLeaseCapacity {
                            runner_classes: vec![RunnerClass::Kata],
                            runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                                RuntimeCapabilitiesV1 {
                                    restart: true,
                                    stop: true,
                                    ..Default::default()
                                },
                            )),
                            ..Default::default()
                        }),
                        now: None,
                    })
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(lease.request.id, operation.id);
                db.complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: operation.id,
                    runner_id: "auth-runner".into(),
                    lease_token: "control-lease".into(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: None,
                    now: None,
                })
                .await
                .unwrap();
                assert_eq!(
                    db.authenticate_runtime_credential(&secret)
                        .await
                        .unwrap()
                        .is_some(),
                    !stop && index != 3
                );
            }
        })
        .await;
    }
    #[tokio::test]
    async fn runtime_bootstrap_real_http_requires_matching_runner_lease() {
        use crate::auth::test_support::{
            core_auth_with_runner_credentials, runner_credential_config,
        };
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let auth = core_auth_with_runner_credentials(
                "service",
                vec![
                    runner_credential_config(
                        "runner",
                        "runner-secret",
                        "auth-runner",
                        &[crate::RunnerClass::Kata],
                        "auth-host",
                        false,
                    ),
                    runner_credential_config(
                        "other",
                        "other-secret",
                        "other-runner",
                        &[crate::RunnerClass::Kata],
                        "other-host",
                        false,
                    ),
                ],
                "usage",
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!(
                "http://{}/api/core/v1/agent-creation-requests/{request}/runtime-credential",
                listener.local_addr().unwrap()
            );
            let app = crate::api::router(db.store.clone(), auth);
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let client = reqwest::Client::new();
            let body =
                serde_json::json!({"runnerId":"auth-runner","leaseToken":"test-launch-lease"});
            for bearer in ["service", "usage", "unknown", "other-secret"] {
                let response = client
                    .post(&url)
                    .bearer_auth(bearer)
                    .json(&body)
                    .send()
                    .await
                    .unwrap();
                assert!(
                    response.status() == reqwest::StatusCode::UNAUTHORIZED
                        || response.status() == reqwest::StatusCode::FORBIDDEN
                );
            }
            let response = client
                .post(&url)
                .bearer_auth("runner-secret")
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            let issued = response.json::<RuntimeBootstrapCredential>().await.unwrap();
            let replay = client
                .post(&url)
                .bearer_auth("runner-secret")
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<RuntimeBootstrapCredential>()
                .await
                .unwrap();
            assert!(issued.secret == replay.secret);
            assert!(
                db.authenticate_runtime_credential(&issued.secret)
                    .await
                    .unwrap()
                    .is_none()
            );
            register(&db, &request).await;
            complete(&db, &request).await.unwrap();
            assert!(
                db.authenticate_runtime_credential(&issued.secret)
                    .await
                    .unwrap()
                    .is_some()
            );
            server.abort();
        })
        .await;
    }
}
