use super::{SubstrateClient, SubstrateConnection, SubstrateError, protocol as api};
use crate::*;
use finite_saas_core::ProviderRuntimeHandleV1;
use serde::{Deserialize, Serialize};

pub struct SubstrateConfig {
    pub connection: SubstrateConnection,
    pub atespace: String,
    pub base_template: String,
    pub source_host_id: String,
    pub finitechat_server_url: String,
    /// Private service ingress, projecting /runtimes/<runtime-id>/ to agentd.
    pub runtime_origin: String,
    pub draining: bool,
    pub egress_hosts: Vec<String>,
    pub egress_cidrs: Vec<String>,
}

pub struct SubstrateLauncher {
    config: SubstrateConfig,
    client: SubstrateClient,
    executor: tokio::runtime::Runtime,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Handle {
    atespace: String,
    actor: String,
    uid: String,
}

fn failed(error: impl std::fmt::Display) -> RunnerError {
    RunnerError::RuntimeLaunchPending(format!("Substrate: {error}"))
}

fn validate_base(template: &api::ActorTemplate) -> Result<(), RunnerError> {
    let [container] = template.containers.as_slice() else {
        return Err(failed(
            "base template must contain only the canonical agent container",
        ));
    };
    if container.name != "agent"
        || !container.volume_mounts.iter().any(|mount| {
            mount.mount_path == "/data"
                && template.volumes.iter().any(|volume| {
                    volume.name == mount.name && volume.external_volume_template.is_some()
                })
        })
    {
        return Err(failed("canonical agent requires a CSI-backed /data volume"));
    }
    // gVisor's DATA checkpoint currently requires a durable-dir mount even
    // when all authoritative state lives on CSI. The base supplies an empty
    // checkpoint directory; it is not a second copy of agent data.
    if !container.volume_mounts.iter().any(|mount| {
        template
            .volumes
            .iter()
            .any(|volume| volume.name == mount.name && volume.durable_dir.is_some())
    }) {
        return Err(failed(
            "base template requires a durable-dir checkpoint mount",
        ));
    }
    Ok(())
}

impl SubstrateLauncher {
    pub fn new(config: SubstrateConfig) -> Result<Self, RunnerError> {
        let client = SubstrateClient::new(config.connection.clone()).map_err(failed)?;
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failed)?;
        Ok(Self {
            config,
            client,
            executor,
        })
    }

    fn reference(&self, name: &str) -> api::ObjectRef {
        api::ObjectRef {
            atespace: self.config.atespace.clone(),
            name: name.into(),
        }
    }

    pub(super) fn actor_name(runtime_id: &str) -> Result<String, RunnerError> {
        let suffix = runtime_id
            .strip_prefix("runtime_")
            .filter(|suffix| {
                suffix.len() == 20
                    && suffix
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            })
            .ok_or_else(|| failed("expected a Core-generated runtime id"))?;
        // Injective for Core's new-runtime IDs; the shared ingress applies the
        // same mapping without a second routing registry or per-agent proxy.
        Ok(format!("runtime-{suffix}"))
    }

    fn artifact_template(
        &self,
        runtime_id: &str,
        artifact_id: &str,
    ) -> Result<api::ObjectRef, RunnerError> {
        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(artifact_id.as_bytes()));
        Ok(self.reference(&format!(
            "{}-{}",
            Self::actor_name(runtime_id)?,
            &digest[..32]
        )))
    }

    // Both creation and lifecycle recovery require a current Core lease.
    // Revert discards execution only after verifying CSI data and cold boot.
    fn revert_to_durable_data(
        &self,
        actor: api::ObjectRef,
        template: api::ObjectRef,
    ) -> Result<(), RunnerError> {
        let template = self
            .executor
            .block_on(self.client.template(template))
            .map_err(failed)?
            .ok_or_else(|| failed("actor template missing"))?;
        validate_base(&template)?;
        let snapshots = template
            .snapshots_config
            .as_ref()
            .ok_or_else(|| failed("snapshot policy missing"))?;
        if snapshots.on_pause != api::SnapshotContentScope::Data as i32
            || snapshots.on_commit != api::SnapshotContentScope::Data as i32
            || snapshots.on_resume.as_ref().map(|resume| resume.from_data)
                != Some(api::ResumeSource::ColdBoot as i32)
        {
            return Err(failed("execution recovery requires CSI data and cold boot"));
        }
        self.executor
            .block_on(self.client.revert(actor))
            .map_err(failed)?;
        Ok(())
    }

    fn suspend_for_restart(
        &self,
        lease: &RuntimeControlLease,
    ) -> Result<api::ObjectRef, RunnerError> {
        let (actor, current) = self.control_actor(lease)?;
        let state = current.status.as_ref().map(|status| status.state());
        if matches!(
            state,
            Some(
                api::ActorState::Crashed
                    | api::ActorState::Reverting
                    | api::ActorState::Resuming
                    | api::ActorState::Suspending
            )
        ) {
            self.revert_to_durable_data(
                actor.clone(),
                current
                    .actor_template
                    .ok_or_else(|| failed("actor template missing"))?,
            )?;
        } else {
            self.executor
                .block_on(self.client.stop(actor.clone()))
                .map_err(failed)?;
        }
        Ok(actor)
    }

    fn control_actor(
        &self,
        lease: &RuntimeControlLease,
    ) -> Result<(api::ObjectRef, api::Actor), RunnerError> {
        control_runtime_spec(lease, RunnerClass::Substrate)?
            .ok_or_else(|| failed("missing RuntimeSpec"))?;
        let Some(ProviderRuntimeHandleEnvelope::V1(envelope)) =
            &lease.runtime.provider_runtime_handle
        else {
            return Err(failed("missing provider handle"));
        };
        if envelope.runner_class != RunnerClass::Substrate {
            return Err(failed("wrong runner class"));
        }
        let handle: Handle = serde_json::from_value(envelope.opaque.clone())
            .map_err(|_| failed("invalid provider handle"))?;
        if handle.atespace != self.config.atespace
            || handle.actor != Self::actor_name(&lease.runtime.id)?
            || lease.runtime.source_host_id != self.config.source_host_id
        {
            return Err(failed("provider handle does not match assignment"));
        }
        let reference = self.reference(&handle.actor);
        let actor = self
            .executor
            .block_on(self.client.actor(reference.clone()))
            .map_err(failed)?
            .ok_or_else(|| failed("actor missing; restore is required"))?;
        if actor
            .metadata
            .as_ref()
            .map(|metadata| metadata.uid.as_str())
            != Some(handle.uid.as_str())
        {
            return Err(failed("actor identity changed"));
        }
        if !matches!(
            lease.request.kind,
            RuntimeControlKind::Upgrade | RuntimeControlKind::Stop
        ) && actor.actor_template
            != Some(
                self.artifact_template(
                    &lease.runtime.id,
                    lease
                        .runtime
                        .runtime_artifact_id
                        .as_deref()
                        .ok_or_else(|| failed("current artifact missing"))?,
                )?,
            )
        {
            return Err(failed(
                "uncommitted template change requires upgrade reconciliation",
            ));
        }
        Ok((reference, actor))
    }
}

impl RuntimeLauncher for SubstrateLauncher {
    fn enqueue_recovery(
        &mut self,
        queue: &mut dyn AgentCreationQueue,
    ) -> Result<bool, RunnerError> {
        let mut page_token = String::new();
        // Bound each scan to 100,000 actors, with one provider read per page
        // instead of a connection per healthy runtime. Core admits at most
        // one recovery per cycle and remains the only lifecycle authority.
        for _ in 0..100 {
            let page = self
                .executor
                .block_on(self.client.actors(&self.config.atespace, page_token))
                .map_err(failed)?;
            for actor in page.actors {
                if actor
                    .status
                    .is_none_or(|status| status.state() != api::ActorState::Crashed)
                {
                    continue;
                }
                let Some(metadata) = actor.metadata else {
                    continue;
                };
                let Some(suffix) = metadata.name.strip_prefix("runtime-") else {
                    continue;
                };
                let runtime_id = format!("runtime_{suffix}");
                if Self::actor_name(&runtime_id).is_ok()
                    && queue.request_runtime_recovery(&runtime_id)?
                {
                    return Ok(true);
                }
            }
            if page.next_page_token.is_empty() {
                return Ok(false);
            }
            page_token = page.next_page_token;
        }
        Err(failed("recovery actor inventory exceeded 100 pages"))
    }

    fn reconciles_interrupted_launch(&self) -> bool {
        true
    }

    fn existing_inference_key(
        &self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<Option<FinitePrivateLaunchKey>, RunnerError> {
        let spec = creation_runtime_spec(lease, RunnerClass::Substrate)?
            .ok_or_else(|| failed("missing RuntimeSpec"))?;
        let reference =
            self.artifact_template(&spec.agent_runtime_id, &spec.runtime_artifact_id)?;
        let Some(template) = self
            .executor
            .block_on(self.client.template(reference))
            .map_err(failed)?
        else {
            return Ok(None);
        };
        let [container] = template.containers.as_slice() else {
            return Err(failed("invalid installed template"));
        };
        let env: BTreeMap<_, _> = container
            .env
            .iter()
            .map(|e| (e.name.as_str(), e.value.as_str()))
            .collect();
        if !options
            .secret_environment
            .contains_key("FINITE_CORE_CREDENTIAL")
            || container.image != spec.runtime_image_digest
            || env.get("FINITE_CORE_CREDENTIAL").copied()
                != options
                    .secret_environment
                    .get("FINITE_CORE_CREDENTIAL")
                    .map(String::as_str)
        {
            return Err(failed("installed bootstrap does not match this creation"));
        }
        let packed: BTreeMap<String, String> = serde_json::from_str(
            env.get("FINITE_BOOTSTRAP_ENV_JSON")
                .ok_or_else(|| failed("installed bootstrap missing"))?,
        )
        .map_err(|_| failed("invalid installed bootstrap"))?;
        if packed.get("FINITE_AGENT_ID") != Some(&stable_project_agent_id(&lease.project.id)) {
            return Err(failed("installed agent identity mismatch"));
        }
        let required = |key: &str| {
            packed
                .get(key)
                .cloned()
                .ok_or_else(|| failed("installed inference credential missing"))
        };
        Ok(Some(FinitePrivateLaunchKey {
            api_key_id: required("FINITE_BOOTSTRAP_INFERENCE_KEY_ID")?,
            raw_api_key: required("FINITE_PRIVATE_API_KEY")?,
            base_url: required("FINITE_PRIVATE_BASE_URL")?,
            model: required("FINITE_PRIVATE_MODEL")?,
            revoke_on_launch_failure: false,
        }))
    }

    fn validate_ready(&self) -> Result<(), RunnerError> {
        let template = self
            .executor
            .block_on(
                self.client
                    .template(self.reference(&self.config.base_template)),
            )
            .map_err(failed)?
            .ok_or_else(|| failed("base template is unavailable"))?;
        validate_base(&template)
    }

    fn runner_capacity(&self) -> RunnerLeaseCapacity {
        RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Substrate],
            draining: self.config.draining,
            ..Default::default()
        }
    }

    fn runner_class(&self) -> RunnerClass {
        RunnerClass::Substrate
    }
    fn source_host_id(&self) -> Option<&str> {
        Some(&self.config.source_host_id)
    }
    fn runtime_capabilities(&self) -> RuntimeCapabilitiesEnvelope {
        let RuntimeCapabilitiesEnvelope::V1(mut capabilities) =
            state_preserving_runtime_capabilities(true);
        capabilities.native_hermes_chat = true;
        RuntimeCapabilitiesEnvelope::V1(capabilities)
    }
    fn planned_source(&self, lease: &AgentCreationLease) -> Option<RuntimeSourceIdentity> {
        Some(RuntimeSourceIdentity {
            source_host_id: self.config.source_host_id.clone(),
            source_machine_id: Self::actor_name(lease.request.agent_runtime_id.as_deref()?).ok()?,
        })
    }

    fn launch(
        &mut self,
        _lease: &AgentCreationLease,
        _options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        Err(failed(
            "creation requires the Core provider-operation journal",
        ))
    }

    fn launch_with_provider_operation(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
        journal: &mut dyn ProviderOperationJournal,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        let spec = creation_runtime_spec(lease, RunnerClass::Substrate)?
            .ok_or_else(|| failed("missing RuntimeSpec"))?;
        let name = Self::actor_name(&spec.agent_runtime_id)?;
        let reference = self.reference(&name);
        let template_reference =
            self.artifact_template(&spec.agent_runtime_id, &spec.runtime_artifact_id)?;
        journal
            .record(
                &name,
                spec.placement,
                ProviderOperationTransition::CorrelationReserved,
            )
            .map_err(failed)?;
        journal
            .record(
                &name,
                spec.placement,
                ProviderOperationTransition::ProvisionStarted,
            )
            .map_err(failed)?;
        let mut actor = self
            .executor
            .block_on(self.client.actor(reference.clone()))
            .map_err(failed)?;
        if actor.is_none() {
            let mut template = self
                .executor
                .block_on(
                    self.client
                        .template(self.reference(&self.config.base_template)),
                )
                .map_err(failed)?
                .ok_or_else(|| failed("base template is unavailable"))?;
            validate_base(&template)?;
            let (cpu, memory) = match spec.placement.runtime_resource_class {
                RuntimeResourceClass::Vcpu4Memory8Gib => ("4", "8Gi"),
                RuntimeResourceClass::Vcpu2Memory4Gib => ("2", "4Gi"),
            };
            template.resources = Some(api::Resources {
                limits: [("cpu", cpu), ("memory", memory)]
                    .into_iter()
                    .map(|(name, quantity)| api::Limits {
                        name: name.into(),
                        quantity: quantity.into(),
                    })
                    .collect(),
            });
            template.containers[0].resources = template.resources.clone();
            template.metadata = Some(api::ResourceMetadata {
                atespace: self.config.atespace.clone(),
                name: template_reference.name.clone(),
                ..Default::default()
            });
            template.status = None;
            if template.volumes.iter().any(|v| v.name == "finite-identity") {
                return Err(failed("base template reserves finite-identity"));
            }
            template.volumes.push(api::Volume {
                name: "finite-identity".into(),
                system_info: Some(api::SystemInfoVolumeSource {
                    data_sources: vec![api::SystemInfoDataSource {
                        actor_metadata: Some(api::ActorMetadataDataSource {
                            items: vec![api::ActorMetadataItem {
                                field: api::ActorMetadataField::Atespace as i32,
                                path: "atespace".into(),
                            }],
                        }),
                        ..Default::default()
                    }],
                }),
                ..Default::default()
            });
            let mut bootstrap_options = options.clone();
            // Mutable public flags are fetched by the runtime, not mirrored in
            // its immutable provider template. Core credentials bootstrap that pull.
            bootstrap_options
                .environment
                .retain(|key, _| key == "FINITE_CORE_URL");
            let mut environment: BTreeMap<_, _> = docker_equivalent_runtime_env(
                DockerEquivalentRuntimeEnv {
                    finitechat_server_url: &self.config.finitechat_server_url,
                    agent_picture_url: DEFAULT_FINITE_AGENT_PICTURE_URL,
                    agent_http_port: spec.endpoints.service_port,
                    agent_device_id: "agent",
                    agent_home: "/data/agent",
                    hermes_home: "/data/agent/hermes-home",
                    workspace: "/data/workspace",
                },
                lease,
                &bootstrap_options,
            )
            .into_iter()
            .collect();
            let core_url = environment
                .remove("FINITE_CORE_URL")
                .ok_or_else(|| failed("Core bootstrap URL required"))?;
            let credential = environment
                .remove("FINITE_CORE_CREDENTIAL")
                .ok_or_else(|| failed("Core bootstrap credential required"))?;
            if let Some(key) = &options.finite_private {
                environment.insert(
                    "FINITE_BOOTSTRAP_INFERENCE_KEY_ID".into(),
                    key.api_key_id.clone(),
                );
            }
            let packed = serde_json::to_string(&environment).map_err(failed)?;
            if packed.len() > 32768 {
                return Err(failed(
                    "bootstrap environment exceeds Substrate's value limit",
                ));
            }
            let container = &mut template.containers[0];
            container.volume_mounts.push(api::VolumeMount {
                name: "finite-identity".into(),
                mount_path: "/run/ate".into(),
            });
            container.readyz = Some(api::ContainerReadyz {
                http_get: Some(api::HttpGetAction {
                    path: "/healthz".into(),
                    port: 8080,
                }),
                timeout_seconds: 180,
            });
            container.image = spec.runtime_image_digest.clone();
            container.command = vec!["/runtime/bin/finite-agentd".into()];
            container.args = vec!["bootstrap".into()];
            container.env = [
                ("FINITE_CORE_URL", core_url),
                ("FINITE_CORE_CREDENTIAL", credential),
                ("FINITE_BOOTSTRAP_ENV_JSON", packed),
            ]
            .into_iter()
            .map(|(name, value)| api::EnvVar {
                name: name.into(),
                value,
            })
            .collect();
            // Cold boot from durable data re-reads current Core flags on restart.
            let snapshots = template
                .snapshots_config
                .as_mut()
                .ok_or_else(|| failed("snapshot policy required"))?;
            snapshots.on_pause = api::SnapshotContentScope::Data as i32;
            snapshots.on_commit = api::SnapshotContentScope::Data as i32;
            snapshots.on_resume = Some(api::OnResumeConfig {
                from_data: api::ResumeSource::ColdBoot as i32,
            });
            self.executor
                .block_on(self.client.ensure_template(template))
                .map_err(failed)?;
            actor = Some(
                self.executor
                    .block_on(self.client.ensure_actor(api::Actor {
                        metadata: Some(api::ResourceMetadata {
                            atespace: self.config.atespace.clone(),
                            name: name.clone(),
                            ..Default::default()
                        }),
                        actor_template: Some(template_reference.clone()),
                        ..Default::default()
                    }))
                    .map_err(failed)?,
            );
        }
        let actor = actor.ok_or_else(|| failed(SubstrateError::Conflict))?;
        if actor.actor_template.as_ref() != Some(&template_reference) {
            return Err(failed("actor template changed"));
        }
        let uid = actor
            .metadata
            .ok_or_else(|| failed("actor metadata missing"))?
            .uid;
        let handle = ProviderRuntimeHandleEnvelope::V1(ProviderRuntimeHandleV1 {
            runner_class: RunnerClass::Substrate,
            opaque: serde_json::to_value(Handle {
                atespace: self.config.atespace.clone(),
                actor: name.clone(),
                uid: uid.clone(),
            })
            .map_err(failed)?,
        });
        journal
            .record(
                &name,
                spec.placement,
                ProviderOperationTransition::Provisioned {
                    provider_facts: serde_json::json!({"uid":uid}),
                },
            )
            .map_err(failed)?;
        journal
            .record(
                &name,
                spec.placement,
                ProviderOperationTransition::CommitStarted,
            )
            .map_err(failed)?;
        self.executor
            .block_on(self.client.ensure_egress(
                reference.clone(),
                self.config.egress_hosts.clone(),
                self.config.egress_cidrs.clone(),
            ))
            .map_err(failed)?;
        if matches!(
            actor.status.as_ref().map(|status| status.state()),
            Some(api::ActorState::Crashed | api::ActorState::Reverting)
        ) {
            // A worker may die after provider creation but before Core completion.
            // Reconcile the same journaled actor; Resume alone rejects CRASHED.
            self.revert_to_durable_data(reference.clone(), template_reference)?;
        }
        self.executor
            .block_on(self.client.resume(reference))
            .map_err(|error| match error {
                SubstrateError::Rpc {
                    code: tonic::Code::ResourceExhausted,
                    ..
                } => RunnerError::RuntimeCapacityUnavailable,
                error => failed(error),
            })?;
        let base = format!(
            "{}/runtimes/{}",
            self.config.runtime_origin.trim_end_matches('/'),
            spec.agent_runtime_id
        );
        wait_for_http_json_ready(
            &format!("{base}/healthz"),
            &name,
            DEFAULT_RUNTIME_READY_TIMEOUT,
            DEFAULT_RUNTIME_READY_INTERVAL,
        )
        .map_err(failed)?;
        Ok(RuntimeLaunchFacts {
            source_host_id: self.config.source_host_id.clone(),
            source_machine_id: name.clone(),
            runtime_artifact_id: Some(spec.runtime_artifact_id.clone()),
            state_schema_version: Some(spec.state_schema_version.clone()),
            provider_runtime_handle: Some(handle),
            contact_endpoint: Some(format!("{base}/contact")),
            display_name: Some(lease.project.display_name.clone()),
            hostname: None,
            runtime_host: Some(base),
            runtime_status: RuntimeSummaryStatus::Online,
            active_inference_profile: options
                .finite_private
                .as_ref()
                .map(|_| FINITE_PRIVATE_PROFILE_ID.into()),
            hermes_available: Some(true),
            published_app_urls: vec![],
        })
    }

    fn upgrade_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        options: &RuntimeRestartOptions,
    ) -> Result<RuntimeUpgradeFacts, RunnerError> {
        let spec = control_runtime_spec(lease, RunnerClass::Substrate)?
            .ok_or_else(|| failed("missing RuntimeSpec"))?;
        let target = lease
            .target_runtime_artifact
            .as_ref()
            .ok_or_else(|| failed("missing upgrade target"))?;
        if target.kind != RuntimeArtifactKind::OciImage
            || target.id != spec.runtime_artifact_id
            || target.reference != spec.runtime_image_digest
            || target.state_schema_version != spec.state_schema_version
            || lease.runtime.state_schema_version.as_deref() != Some(&target.state_schema_version)
            || options.core_bootstrap.is_some()
        {
            return Err(failed(
                "upgrade target or Core enrollment conflicts with existing runtime",
            ));
        }
        let (reference, current) = self.control_actor(lease)?;
        // Core's current/target artifact IDs name both immutable templates.
        // A lost completion can retry or restore the prior image without a
        // second journal, a renamed actor, or replacement storage.
        let previous_ref = self.artifact_template(
            &lease.runtime.id,
            lease
                .runtime
                .runtime_artifact_id
                .as_deref()
                .ok_or_else(|| failed("current artifact missing"))?,
        )?;
        let target_ref = self.artifact_template(&lease.runtime.id, &target.id)?;
        let current_ref = current
            .actor_template
            .clone()
            .ok_or_else(|| failed("actor template missing"))?;
        if current_ref != previous_ref && current_ref != target_ref {
            return Err(failed(
                "actor template conflicts with Core artifact assignment",
            ));
        }
        let mut template = self
            .executor
            .block_on(self.client.template(previous_ref.clone()))
            .map_err(failed)?
            .ok_or_else(|| failed("previous artifact template missing"))?;
        validate_base(&template)?;
        // Preserve the provider-owned storage layout and bootstrap identity.
        // Public flags are fetched from Core on the next cold boot.
        template.metadata = Some(api::ResourceMetadata {
            atespace: self.config.atespace.clone(),
            name: target_ref.name.clone(),
            ..Default::default()
        });
        template.status = None;
        let container = &mut template.containers[0];
        container.image = target.reference.clone();
        let packed = container
            .env
            .iter_mut()
            .find(|entry| entry.name == "FINITE_BOOTSTRAP_ENV_JSON")
            .ok_or_else(|| failed("bootstrap environment missing"))?;
        let mut environment: BTreeMap<String, String> = serde_json::from_str(&packed.value)
            .map_err(|_| failed("invalid bootstrap environment"))?;
        environment.extend(options.secret_environment.clone());
        packed.value = serde_json::to_string(&environment).map_err(failed)?;
        if packed.value.len() > 32768 {
            return Err(failed(
                "bootstrap environment exceeds Substrate's value limit",
            ));
        }
        // Construct and validate the target before interrupting the agent.
        self.executor
            .block_on(self.client.ensure_template(template))
            .map_err(failed)?;
        if current_ref != target_ref {
            self.suspend_for_restart(lease)?;
            let (_, suspended) = self.control_actor(lease)?;
            if suspended.actor_template != Some(previous_ref.clone()) {
                return Err(failed("actor template changed during upgrade"));
            }
            self.executor
                .block_on(self.client.replace_template(suspended, target_ref.clone()))
                .map_err(failed)?;
        }
        let runtime_host = format!(
            "{}/runtimes/{}",
            self.config.runtime_origin.trim_end_matches('/'),
            lease.runtime.id
        );
        let resume = self
            .executor
            .block_on(self.client.resume(reference.clone()))
            .map_err(failed)
            .and_then(|_| {
                wait_for_http_json_ready(
                    &format!("{runtime_host}/healthz"),
                    &reference.name,
                    DEFAULT_RUNTIME_READY_TIMEOUT,
                    DEFAULT_RUNTIME_READY_INTERVAL,
                )
            });
        if let Err(error) = resume {
            // Restore compute only. Never rewind CSI state or claim rollback of
            // application writes. Both images must have the same state schema.
            self.suspend_for_restart(lease)?;
            let (_, suspended) = self.control_actor(lease)?;
            if suspended.actor_template != Some(target_ref) {
                return Err(failed("actor template changed before image recovery"));
            }
            self.executor
                .block_on(self.client.replace_template(suspended, previous_ref))
                .map_err(failed)?;
            self.executor
                .block_on(self.client.resume(reference.clone()))
                .map_err(failed)?;
            wait_for_http_json_ready(
                &format!("{runtime_host}/healthz"),
                &reference.name,
                DEFAULT_RUNTIME_READY_TIMEOUT,
                DEFAULT_RUNTIME_READY_INTERVAL,
            )?;
            return Err(error);
        }
        Ok(RuntimeUpgradeFacts {
            runtime_artifact_id: target.id.clone(),
            state_schema_version: target.state_schema_version.clone(),
            runtime_host,
            published_app_urls: lease.runtime.host_facts.published_app_urls.clone(),
        })
    }

    fn stop_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        let (actor, _) = self.control_actor(lease)?;
        self.executor
            .block_on(self.client.stop(actor))
            .map_err(failed)?;
        Ok(())
    }

    fn restart_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        _options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        let actor = if lease.request.requested_by_user_id.is_none() {
            let (reference, current) = self.control_actor(lease)?;
            match current.status.as_ref().map(|status| status.state()) {
                // A delayed observation must not interrupt a healthy actor.
                // Suspended/Resuming also cover an interrupted recovery lease.
                Some(
                    api::ActorState::Running
                    | api::ActorState::Suspended
                    | api::ActorState::Resuming,
                ) => reference,
                Some(api::ActorState::Crashed | api::ActorState::Reverting) => {
                    self.suspend_for_restart(lease)?
                }
                _ => {
                    return Err(failed(
                        "automatic recovery encountered an unexpected actor state",
                    ));
                }
            }
        } else {
            self.suspend_for_restart(lease)?
        };
        self.executor
            .block_on(self.client.resume(actor))
            .map_err(failed)?;
        wait_for_http_json_ready(
            &format!(
                "{}/runtimes/{}/healthz",
                self.config.runtime_origin.trim_end_matches('/'),
                lease.runtime.id
            ),
            &Self::actor_name(&lease.runtime.id)?,
            DEFAULT_RUNTIME_READY_TIMEOUT,
            DEFAULT_RUNTIME_READY_INTERVAL,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_on_data_requires_external_storage() {
        let mut template = api::ActorTemplate {
            containers: vec![api::Container {
                name: "agent".into(),
                volume_mounts: vec![api::VolumeMount {
                    name: "data".into(),
                    mount_path: "/data".into(),
                }],
                ..Default::default()
            }],
            volumes: vec![api::Volume {
                name: "data".into(),
                durable_dir: Some(Default::default()),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(validate_base(&template).is_err());
        template.volumes[0].durable_dir = None;
        template.volumes[0].external_volume_template = Some(api::ExternalVolumeTemplate {
            capacity: "10Gi".into(),
            storage_class_name: "persistent-csi".into(),
        });
        assert!(validate_base(&template).is_err());
        template.volumes.push(api::Volume {
            name: "checkpoint".into(),
            durable_dir: Some(Default::default()),
            ..Default::default()
        });
        template.containers[0].volume_mounts.push(api::VolumeMount {
            name: "checkpoint".into(),
            mount_path: "/run/finite-checkpoint".into(),
        });
        assert!(validate_base(&template).is_ok());
        template.containers[0].volume_mounts[0].mount_path = "/scratch".into();
        assert!(validate_base(&template).is_err());
    }
}
