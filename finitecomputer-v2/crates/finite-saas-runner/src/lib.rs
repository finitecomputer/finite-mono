use finite_saas_core::{
    AgentCreationLease, AgentCreationRequest, CompleteAgentCreationRequestInput,
    CompleteRuntimeControlRequestInput, FailAgentCreationRequestInput,
    FailRuntimeControlRequestInput, LeaseRuntimeControlRequestInput, ProviderOperationEnvelope,
    ProviderOperationTransition, ProviderRuntimeHandleEnvelope,
    ProvisionFinitePrivateRuntimeKeyInput, ProvisionFinitePrivateRuntimeKeyResult,
    RegisterAgentCreationRuntimeInput, RelayHeartbeat, RenewRuntimeControlRequestInput,
    RetryRuntimeControlRequestInput, RunnerClass, RunnerLeaseCapacity, RuntimeArtifact,
    RuntimeArtifactKind, RuntimeBootIntent, RuntimeCapabilitiesEnvelope, RuntimeCapabilitiesV1,
    RuntimeControlKind, RuntimeControlLease, RuntimeControlRequest, RuntimePlacement,
    RuntimeResourceClass, RuntimeRetirementSnapshotReceipt, RuntimeSpecEnvelope, RuntimeSpecV1,
    RuntimeSummaryStatus, api::RecordProviderOperationTransitionRequest,
    runtime_relay_token_hash as hash_runtime_relay_token,
};
#[cfg(test)]
use finite_saas_core::{FinitePrivateApiKey, RuntimeEndpointContractV1};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

mod apple_container;
mod kata;
pub mod phala;
mod phala_inventory;
pub mod retirement;

pub use apple_container::{AppleContainerConfig, AppleContainerLaunchPlan, AppleContainerLauncher};
pub use kata::{KataConfig, KataLaunchPlan, KataLauncher, KataRetirementConfig};
pub use phala::{PhalaConfig, PhalaLauncher};

const DEFAULT_RUNTIME_READY_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_RUNTIME_READY_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_LAUNCH_TIMEOUT: Duration = Duration::from_secs(300);
const RUNTIME_RETIREMENT_LEASE_SECONDS: i64 = 60 * 60;
// The deployed limiter domain keeps the historical kimi-k2-6 name but now
// serves glm-5-2 (see docs/service-dependencies.md, Finite Private Routing
// Debt). Do not rename the URL as a cosmetic change.
pub const DEFAULT_FINITE_PRIVATE_BASE_URL: &str =
    "https://kimi-k2-6.finite.containers.tinfoil.dev/v1";
pub const DEFAULT_FINITE_PRIVATE_MODEL: &str = "glm-5-2";
pub const DEFAULT_FINITE_PRIVATE_SPECIALIZATION_BUNDLE: &str = "aeon-multimodal";
pub const DEFAULT_FINITECHAT_SERVER_URL: &str = "https://chat.finite.computer";
pub const DEFAULT_FINITE_AGENT_PICTURE_URL: &str =
    "https://avatars.githubusercontent.com/u/274919006?v=4";
const FINITE_PRIVATE_PROFILE_ID: &str = "finite-private";
const FINITE_SPECIALIZATION_BUNDLE_ENV: &str = "FINITE_SPECIALIZATION_BUNDLE";
const FINITE_SPECIALIZATION_WORKER_API_KEY_ENV: &str = "FINITE_SPECIALIZATION_WORKER_API_KEY";
const DEFAULT_DOCKER_CONTAINER_PORT: u16 = 8080;
const MAX_RUNTIME_ENVIRONMENT_ENTRIES: usize = 64;
const MAX_RUNTIME_ENVIRONMENT_KEY_BYTES: usize = 128;
const MAX_RUNTIME_ENVIRONMENT_VALUE_BYTES: usize = 4 * 1024;
const MAX_RUNTIME_ENVIRONMENT_TOTAL_BYTES: usize = 32 * 1024;
const MAX_RUNTIME_SECRET_ENVIRONMENT_ENTRIES: usize = 64;
const MAX_RUNTIME_SECRET_ENVIRONMENT_VALUE_BYTES: usize = 16 * 1024;
const MAX_RUNTIME_SECRET_ENVIRONMENT_TOTAL_BYTES: usize = 128 * 1024;

pub(crate) fn state_preserving_runtime_capabilities(
    runtime_upgrade: bool,
) -> RuntimeCapabilitiesEnvelope {
    RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
        restart: true,
        recover_known_good_chat: false,
        runtime_upgrade,
        stop: true,
        runtime_retirement: false,
    })
}

#[cfg(test)]
pub(crate) fn kata_runtime_capabilities() -> RuntimeCapabilitiesEnvelope {
    kata_runtime_capabilities_with_retirement(false)
}

pub(crate) fn kata_runtime_capabilities_with_retirement(
    runtime_retirement: bool,
) -> RuntimeCapabilitiesEnvelope {
    RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
        restart: true,
        recover_known_good_chat: true,
        runtime_upgrade: true,
        stop: true,
        runtime_retirement,
    })
}

fn artifact_bounded_upgrade_runtime_capabilities(
    mut capabilities: RuntimeCapabilitiesEnvelope,
    target_artifact: Option<&RuntimeArtifact>,
) -> RuntimeCapabilitiesEnvelope {
    let RuntimeCapabilitiesEnvelope::V1(bounded) = &mut capabilities;
    bounded.recover_known_good_chat &=
        target_artifact.is_some_and(|artifact| artifact.recover_known_good_chat);
    capabilities
}

fn wait_with_captured_output(
    mut child: Child,
    program: &Path,
    timeout: Duration,
) -> Result<Output, RunnerError> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| RunnerError::CommandExecution {
            program: program.display().to_string(),
            message: "failed to capture stdout".to_string(),
        })?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| RunnerError::CommandExecution {
            program: program.display().to_string(),
            message: "failed to capture stderr".to_string(),
        })?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(RunnerError::CommandTimedOut {
                        program: program.display().to_string(),
                        timeout_secs: timeout.as_secs(),
                    });
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                // A wait failure does not imply the child has exited. Tear it
                // down before joining the pipe readers so this path cannot
                // leak a provider process or block forever on EOF.
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(RunnerError::CommandExecution {
                    program: program.display().to_string(),
                    message: error.to_string(),
                });
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| RunnerError::CommandExecution {
            program: program.display().to_string(),
            message: "stdout reader panicked".to_string(),
        })?
        .map_err(|error| RunnerError::CommandExecution {
            program: program.display().to_string(),
            message: error.to_string(),
        })?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| RunnerError::CommandExecution {
            program: program.display().to_string(),
            message: "stderr reader panicked".to_string(),
        })?
        .map_err(|error| RunnerError::CommandExecution {
            program: program.display().to_string(),
            message: error.to_string(),
        })?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("runner id is required")]
    MissingRunnerId,
    #[error("lease token is required")]
    MissingLeaseToken,
    #[error("source host id is required")]
    MissingSourceHostId,
    #[error("runner work root is required")]
    MissingWorkRoot,
    #[error("Docker binary is required")]
    MissingDockerBinary,
    #[error("Apple Container binary is required")]
    MissingAppleContainerBinary,
    #[error("nerdctl binary is required")]
    MissingNerdctlBinary,
    #[error("Kata runtime binary is required")]
    MissingKataRuntimeBinary,
    #[error("Phala API key is required")]
    MissingPhalaApiKey,
    #[error("Enclavia CLI binary is required")]
    MissingEnclaviaBinary,
    #[error("Enclavia enclave id is required")]
    MissingEnclaviaEnclaveId,
    #[error("Finite Chat server URL is required")]
    MissingFinitechatServerUrl,
    #[error("Agent Identity Authority configuration is invalid")]
    InvalidAgentIdentityAuthorityConfig,
    #[error("Agent Identity binding failed: {0}")]
    AgentIdentityBinding(String),
    #[error("Docker host port must be between 1 and 65535")]
    InvalidDockerHostPort,
    #[error("Apple Container host port must be between 1 and 65535")]
    InvalidAppleContainerHostPort,
    #[error("runtime artifact reference is required")]
    MissingRuntimeArtifactReference,
    #[error("invalid opaque runtime environment: {0}")]
    InvalidRuntimeEnvironment(String),
    #[error("Core request failed: {0}")]
    CoreRequest(String),
    #[error("Core returned HTTP {status}: {body}")]
    CoreStatus { status: u16, body: String },
    #[error("Core response was invalid JSON: {0}")]
    CoreJson(String),
    #[error("runtime launch failed: {0}")]
    RuntimeLaunch(String),
    #[error("failed to execute command {program}: {message}")]
    CommandExecution { program: String, message: String },
    #[error("command {program} timed out after {timeout_secs}s")]
    CommandTimedOut { program: String, timeout_secs: u64 },
}

#[derive(Clone)]
pub struct AgentIdentityAuthorityConfig {
    pub base_url: String,
    pub operator_token: String,
    pub timeout: Duration,
}

impl fmt::Debug for AgentIdentityAuthorityConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentIdentityAuthorityConfig")
            .field("base_url", &self.base_url)
            .field("operator_token", &"[redacted]")
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl AgentIdentityAuthorityConfig {
    fn normalized(mut self) -> Result<Self, RunnerError> {
        self.base_url = self.base_url.trim().trim_end_matches('/').to_string();
        self.operator_token = self.operator_token.trim().to_string();
        if !(self.base_url.starts_with("https://") || self.base_url.starts_with("http://"))
            || self.base_url.contains(char::is_whitespace)
            || self.operator_token.is_empty()
            || self.timeout.is_zero()
        {
            return Err(RunnerError::InvalidAgentIdentityAuthorityConfig);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum RunOnceOutcome {
    Idle,
    CapacityUnavailable {
        reason: String,
        runner_capacity: RunnerLeaseCapacity,
    },
    Launched {
        request_id: String,
        runtime_id: Option<String>,
    },
    LaunchFailed {
        request_id: String,
        failure_message: String,
    },
    RuntimeRestarted {
        request_id: String,
        runtime_id: String,
    },
    RuntimeRestartFailed {
        request_id: String,
        failure_message: String,
    },
    RuntimeRecoveredKnownGoodChat {
        request_id: String,
        runtime_id: String,
    },
    RuntimeUpgraded {
        request_id: String,
        runtime_id: String,
    },
    RuntimeRecoveryFailed {
        request_id: String,
        failure_message: String,
    },
    RuntimeUpgradeFailed {
        request_id: String,
        failure_message: String,
    },
    RuntimeStopped {
        request_id: String,
        runtime_id: String,
    },
    RuntimeStopFailed {
        request_id: String,
        failure_message: String,
    },
    RuntimeDestroyed {
        request_id: String,
        runtime_id: String,
    },
    RuntimeDestroyFailed {
        request_id: String,
        failure_message: String,
    },
}

fn identity_transport_error(error: ureq::Error) -> RunnerError {
    let message = match error {
        ureq::Error::Status(status, _) => format!("Identity Authority returned HTTP {status}"),
        ureq::Error::Transport(error) => format!("Identity Authority transport error: {error}"),
    };
    RunnerError::AgentIdentityBinding(message)
}

#[derive(Debug, Clone)]
pub struct AgentCreationRunner<Q, L, T> {
    queue: Q,
    launcher: L,
    lease_tokens: T,
    runner_id: String,
    lease_seconds: i64,
    runtime_ready_timeout: Duration,
    runtime_ready_interval: Duration,
    default_finite_private: Option<FinitePrivateRuntimeDefaults>,
    runtime_environment: BTreeMap<String, String>,
    runtime_secret_environment: BTreeMap<String, String>,
    agent_identity_authority: Option<AgentIdentityAuthorityConfig>,
}

impl<Q, L, T> AgentCreationRunner<Q, L, T>
where
    Q: AgentCreationQueue,
    L: RuntimeLauncher,
    T: LeaseTokenSource,
{
    pub fn new(
        queue: Q,
        launcher: L,
        lease_tokens: T,
        runner_id: impl Into<String>,
        lease_seconds: i64,
    ) -> Result<Self, RunnerError> {
        let runner_id = runner_id.into();
        if runner_id.trim().is_empty() {
            return Err(RunnerError::MissingRunnerId);
        }
        Ok(Self {
            queue,
            launcher,
            lease_tokens,
            runner_id,
            lease_seconds,
            runtime_ready_timeout: DEFAULT_RUNTIME_READY_TIMEOUT,
            runtime_ready_interval: DEFAULT_RUNTIME_READY_INTERVAL,
            default_finite_private: None,
            runtime_environment: BTreeMap::new(),
            runtime_secret_environment: BTreeMap::new(),
            agent_identity_authority: None,
        })
    }

    pub fn with_runtime_ready_polling(mut self, timeout: Duration, interval: Duration) -> Self {
        self.runtime_ready_timeout = timeout;
        self.runtime_ready_interval = interval;
        self
    }

    pub fn with_agent_identity_authority(
        mut self,
        config: AgentIdentityAuthorityConfig,
    ) -> Result<Self, RunnerError> {
        self.agent_identity_authority = Some(config.normalized()?);
        Ok(self)
    }

    pub fn with_default_finite_private_inference(
        mut self,
        defaults: FinitePrivateRuntimeDefaults,
    ) -> Self {
        self.default_finite_private = Some(defaults);
        self
    }

    /// Carry provider-neutral, non-secret RuntimeSpec environment through the
    /// shared launch path. Adapters transport it without interpreting which
    /// product owns a key.
    pub fn with_runtime_environment(
        mut self,
        environment: BTreeMap<String, String>,
    ) -> Result<Self, RunnerError> {
        validate_runtime_environment(&environment)?;
        validate_runtime_environment_disjoint(&environment, &self.runtime_secret_environment)?;
        self.runtime_environment = environment;
        Ok(self)
    }

    /// Carry operator-selected secret references through the same generic
    /// RuntimeSpec launch path as every other adapter. Values are never
    /// interpreted by Runner and diagnostics expose keys only.
    pub fn with_runtime_secret_environment(
        mut self,
        environment: BTreeMap<String, String>,
    ) -> Result<Self, RunnerError> {
        validate_runtime_secret_environment(&environment)?;
        validate_runtime_environment_disjoint(&self.runtime_environment, &environment)?;
        self.runtime_secret_environment = environment;
        Ok(self)
    }

    pub fn run_once(&mut self) -> Result<RunOnceOutcome, RunnerError> {
        self.launcher.validate_ready()?;
        let lease_token = self.lease_tokens.next_lease_token()?;
        let source_host_id = self.launcher.source_host_id().map(str::to_string);
        let runtime_capabilities = self.launcher.runtime_capabilities();
        let mut runner_capacity = self.launcher.runner_capacity();
        runner_capacity.runtime_capabilities = Some(runtime_capabilities.clone());
        if runner_capacity.runner_classes.is_empty() {
            return Ok(RunOnceOutcome::CapacityUnavailable {
                reason: "runner advertises no classes".to_string(),
                runner_capacity,
            });
        }
        if let Some(lease) = self.queue.lease_runtime_control(
            &self.runner_id,
            &lease_token,
            self.lease_seconds,
            source_host_id.as_deref(),
            Some(&runner_capacity),
        )? {
            return self.run_runtime_control(lease, lease_token);
        }
        if let Some(reason) = runner_capacity.agent_creation_rejection_reason() {
            return Ok(RunOnceOutcome::CapacityUnavailable {
                reason: reason.to_string(),
                runner_capacity,
            });
        }
        let Some(lease) = self.queue.lease_agent_creation(
            &self.runner_id,
            &lease_token,
            self.lease_seconds,
            Some(&runner_capacity),
        )?
        else {
            return Ok(RunOnceOutcome::Idle);
        };

        let request_id = lease.request.id.clone();
        let launch_options = match self.runtime_launch_options(&lease, &lease_token) {
            Ok(options) => options,
            Err(error) => {
                let failure_message = error.to_string();
                self.queue.fail_agent_creation(
                    &request_id,
                    FailAgentCreationRequestInput {
                        request_id: request_id.clone(),
                        runner_id: self.runner_id.clone(),
                        lease_token,
                        failure_message: failure_message.clone(),
                        provisioned_finite_private_api_key_id: None,
                        now: None,
                    },
                )?;
                return Ok(RunOnceOutcome::LaunchFailed {
                    request_id,
                    failure_message,
                });
            }
        };
        let launch = {
            let mut provider_operation_journal = QueueProviderOperationJournal {
                queue: &mut self.queue,
                request_id: &request_id,
                runner_id: &self.runner_id,
                lease_token: &lease_token,
            };
            self.launcher.launch_with_provider_operation(
                &lease,
                &launch_options,
                &mut provider_operation_journal,
            )
        };
        match launch {
            Ok(facts) => {
                let launch_result = self.queue.register_agent_creation_runtime(
                    &request_id,
                    RegisterAgentCreationRuntimeInput {
                        request_id: request_id.clone(),
                        runner_id: self.runner_id.clone(),
                        lease_token: lease_token.clone(),
                        source_host_id: facts.source_host_id.clone(),
                        source_machine_id: facts.source_machine_id.clone(),
                        runtime_artifact_id: facts.runtime_artifact_id.clone(),
                        state_schema_version: facts.state_schema_version.clone(),
                        provider_runtime_handle: facts.provider_runtime_handle.clone(),
                        contact_endpoint: facts.contact_endpoint.clone(),
                        runtime_relay_token_hash: facts.runtime_relay_token_hash.clone(),
                        display_name: facts.display_name.clone(),
                        hostname: facts.hostname.clone(),
                        runtime_host: facts.runtime_host.clone(),
                        runtime_status: Some(RuntimeSummaryStatus::Unknown),
                        active_inference_profile: facts.active_inference_profile.clone(),
                        hermes_available: facts.hermes_available,
                        published_app_urls: facts.published_app_urls.clone(),
                        runtime_capabilities: Some(runtime_capabilities.clone()),
                        now: None,
                    },
                );
                let launch_result = match launch_result {
                    Ok(_) => match self.wait_for_launch_readiness(&facts.source_machine_id) {
                        Ok(()) => match self.bind_agent_identity(&lease, &facts) {
                            Ok(()) => self.queue.complete_agent_creation(
                                &request_id,
                                CompleteAgentCreationRequestInput {
                                    request_id: request_id.clone(),
                                    runner_id: self.runner_id.clone(),
                                    lease_token: lease_token.clone(),
                                    source_host_id: facts.source_host_id.clone(),
                                    source_machine_id: facts.source_machine_id.clone(),
                                    runtime_artifact_id: facts.runtime_artifact_id.clone(),
                                    state_schema_version: facts.state_schema_version.clone(),
                                    provider_runtime_handle: facts.provider_runtime_handle.clone(),
                                    contact_endpoint: facts.contact_endpoint.clone(),
                                    display_name: facts.display_name.clone(),
                                    hostname: facts.hostname.clone(),
                                    runtime_host: facts.runtime_host.clone(),
                                    runtime_status: Some(RuntimeSummaryStatus::Online),
                                    active_inference_profile: facts
                                        .active_inference_profile
                                        .clone(),
                                    hermes_available: facts.hermes_available,
                                    published_app_urls: facts.published_app_urls.clone(),
                                    runtime_capabilities: Some(runtime_capabilities),
                                    now: None,
                                },
                            ),
                            Err(error) => Err(error),
                        },
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(error),
                };
                match launch_result {
                    Ok(completed) => Ok(RunOnceOutcome::Launched {
                        request_id,
                        runtime_id: completed.request.agent_runtime_id,
                    }),
                    Err(error) => {
                        let failure_message = error.to_string();
                        let cleanup_error = self.launcher.cleanup_failed_launch(&facts).err();
                        self.queue.fail_agent_creation(
                            &request_id,
                            FailAgentCreationRequestInput {
                                request_id: request_id.clone(),
                                runner_id: self.runner_id.clone(),
                                lease_token,
                                failure_message: failure_message.clone(),
                                provisioned_finite_private_api_key_id: provisioned_key_to_revoke(
                                    &launch_options,
                                ),
                                now: None,
                            },
                        )?;
                        if let Some(error) = cleanup_error {
                            eprintln!("warning: failed to clean up failed runtime launch: {error}");
                        }
                        Ok(RunOnceOutcome::LaunchFailed {
                            request_id,
                            failure_message,
                        })
                    }
                }
            }
            Err(error) => {
                let failure_message = error.to_string();
                self.queue.fail_agent_creation(
                    &request_id,
                    FailAgentCreationRequestInput {
                        request_id: request_id.clone(),
                        runner_id: self.runner_id.clone(),
                        lease_token,
                        failure_message: failure_message.clone(),
                        provisioned_finite_private_api_key_id: provisioned_key_to_revoke(
                            &launch_options,
                        ),
                        now: None,
                    },
                )?;
                Ok(RunOnceOutcome::LaunchFailed {
                    request_id,
                    failure_message,
                })
            }
        }
    }

    fn bind_agent_identity(
        &self,
        lease: &AgentCreationLease,
        facts: &RuntimeLaunchFacts,
    ) -> Result<(), RunnerError> {
        let Some(agent_email) = lease.project.agent_email.as_deref() else {
            return Ok(());
        };
        let config = self.agent_identity_authority.as_ref().ok_or_else(|| {
            RunnerError::AgentIdentityBinding(
                "Identity Authority is not configured for this managed agent email".to_string(),
            )
        })?;
        let contact_endpoint = facts
            .contact_endpoint
            .as_deref()
            .filter(|url| {
                (url.starts_with("https://") || url.starts_with("http://"))
                    && !url.contains(char::is_whitespace)
            })
            .ok_or_else(|| {
                RunnerError::AgentIdentityBinding(
                    "runtime did not publish a valid contact endpoint".to_string(),
                )
            })?;
        let agent = ureq::AgentBuilder::new().timeout(config.timeout).build();
        let contact: serde_json::Value = agent
            .get(contact_endpoint)
            .set("Accept", "application/json")
            .call()
            .map_err(identity_transport_error)?
            .into_json()
            .map_err(|error| RunnerError::AgentIdentityBinding(error.to_string()))?;
        let agent_npub = contact
            .get("agent_npub")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| value.starts_with("npub1") && value.len() <= 256)
            .ok_or_else(|| {
                RunnerError::AgentIdentityBinding(
                    "runtime contact document has no Agent Principal".to_string(),
                )
            })?;
        let response: serde_json::Value = agent
            .post(&format!(
                "{}/api/v1/operator/agent-email-bindings",
                config.base_url
            ))
            .set("Accept", "application/json")
            .set("X-Finite-Operator-Token", &config.operator_token)
            .send_json(serde_json::json!({
                "email": agent_email,
                "agent_npub": agent_npub,
            }))
            .map_err(identity_transport_error)?
            .into_json()
            .map_err(|error| RunnerError::AgentIdentityBinding(error.to_string()))?;
        if response.get("email").and_then(serde_json::Value::as_str) != Some(agent_email)
            || response
                .get("agent_npub")
                .and_then(serde_json::Value::as_str)
                != Some(agent_npub)
        {
            return Err(RunnerError::AgentIdentityBinding(
                "Identity Authority returned a mismatched binding".to_string(),
            ));
        }
        Ok(())
    }

    fn run_runtime_control(
        &mut self,
        lease: RuntimeControlLease,
        lease_token: String,
    ) -> Result<RunOnceOutcome, RunnerError> {
        match lease.request.kind {
            RuntimeControlKind::Restart => {
                self.run_runtime_control_operation(lease, lease_token, RuntimeControlKind::Restart)
            }
            RuntimeControlKind::RecoverKnownGoodChatRuntime => self.run_runtime_control_operation(
                lease,
                lease_token,
                RuntimeControlKind::RecoverKnownGoodChatRuntime,
            ),
            RuntimeControlKind::Upgrade => {
                self.run_runtime_control_operation(lease, lease_token, RuntimeControlKind::Upgrade)
            }
            RuntimeControlKind::Stop => {
                self.run_runtime_control_operation(lease, lease_token, RuntimeControlKind::Stop)
            }
            RuntimeControlKind::Destroy => {
                self.run_runtime_control_operation(lease, lease_token, RuntimeControlKind::Destroy)
            }
        }
    }

    fn run_runtime_control_operation(
        &mut self,
        lease: RuntimeControlLease,
        lease_token: String,
        kind: RuntimeControlKind,
    ) -> Result<RunOnceOutcome, RunnerError> {
        let request_id = lease.request.id.clone();
        if !self.launcher.runtime_capabilities().supports(kind) {
            let failure_message =
                format!("runtime control {kind:?} is not advertised by the runner adapter");
            self.queue.fail_runtime_control(
                &request_id,
                FailRuntimeControlRequestInput {
                    request_id: request_id.clone(),
                    runner_id: self.runner_id.clone(),
                    lease_token,
                    failure_message: failure_message.clone(),
                    now: None,
                },
            )?;
            return Ok(runtime_control_failed_outcome(
                kind,
                request_id,
                failure_message,
            ));
        }
        let source_machine_id = lease.request.source_machine_id.clone();
        let previous_heartbeat = match kind {
            RuntimeControlKind::Restart
            | RuntimeControlKind::RecoverKnownGoodChatRuntime
            | RuntimeControlKind::Upgrade => self
                .queue
                .runtime_heartbeat_for_machine(&source_machine_id)?
                .map(|heartbeat| heartbeat.last_seen_at),
            RuntimeControlKind::Stop | RuntimeControlKind::Destroy => None,
        };
        let runtime_spec = control_runtime_spec(&lease, self.launcher.runner_class())?;
        let desired_environment = runtime_spec
            .map(|spec| spec.environment.clone())
            .unwrap_or_else(|| self.runtime_environment.clone());
        let desired_secret_environment = if kind == RuntimeControlKind::Upgrade {
            runtime_spec
                .map(|spec| {
                    resolve_runtime_secret_environment(spec, &self.runtime_secret_environment)
                })
                .transpose()?
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let restart_options = RuntimeRestartOptions::new(desired_environment)?
            .with_secret_environment(desired_secret_environment)?;
        let operation_result: Result<RuntimeControlCompletionFacts, RunnerError> = match kind {
            RuntimeControlKind::Restart => self
                .launcher
                .restart_runtime(&lease, &restart_options)
                .map(|()| RuntimeControlCompletionFacts::None),
            RuntimeControlKind::RecoverKnownGoodChatRuntime => self
                .launcher
                .recover_known_good_chat_runtime(&lease, &restart_options)
                .map(|()| RuntimeControlCompletionFacts::None),
            RuntimeControlKind::Upgrade => self
                .launcher
                .upgrade_runtime(&lease, &restart_options)
                .map(RuntimeControlCompletionFacts::Upgrade),
            RuntimeControlKind::Stop => self
                .launcher
                .stop_runtime(&lease)
                .map(|()| RuntimeControlCompletionFacts::None),
            RuntimeControlKind::Destroy => {
                let runner_id = self.runner_id.clone();
                let lease_token_for_renewal = lease_token.clone();
                // Establish Core's maximum bounded lease before synchronous
                // local manifest/ZIP work. Borg operations below renew this
                // same lease every 30 seconds.
                let lease_seconds = RUNTIME_RETIREMENT_LEASE_SECONDS;
                let queue = &mut self.queue;
                let mut renew_lease = || {
                    queue
                        .renew_runtime_control(
                            &request_id,
                            RenewRuntimeControlRequestInput {
                                request_id: request_id.clone(),
                                runner_id: runner_id.clone(),
                                lease_token: lease_token_for_renewal.clone(),
                                lease_seconds: Some(lease_seconds),
                                now: None,
                            },
                        )
                        .map(|_| ())
                };
                self.launcher
                    .retire_runtime(&lease, &mut renew_lease)
                    .map(|receipt| RuntimeControlCompletionFacts::Retirement(Box::new(receipt)))
            }
        };

        match operation_result {
            Ok(completion_facts) => match self.wait_for_runtime_control_readiness(
                kind,
                &source_machine_id,
                previous_heartbeat.as_deref(),
            ) {
                Ok(()) => {
                    let upgrade_facts = match &completion_facts {
                        RuntimeControlCompletionFacts::Upgrade(facts) => Some(facts),
                        RuntimeControlCompletionFacts::None
                        | RuntimeControlCompletionFacts::Retirement(_) => None,
                    };
                    let retirement_snapshot = match &completion_facts {
                        RuntimeControlCompletionFacts::Retirement(receipt) => {
                            Some((**receipt).clone())
                        }
                        RuntimeControlCompletionFacts::None
                        | RuntimeControlCompletionFacts::Upgrade(_) => None,
                    };
                    let runtime_capabilities = (kind == RuntimeControlKind::Upgrade).then(|| {
                        artifact_bounded_upgrade_runtime_capabilities(
                            self.launcher.runtime_capabilities(),
                            lease.target_runtime_artifact.as_ref(),
                        )
                    });
                    let completed = self.queue.complete_runtime_control(
                        &request_id,
                        CompleteRuntimeControlRequestInput {
                            request_id: request_id.clone(),
                            runner_id: self.runner_id.clone(),
                            lease_token,
                            runtime_artifact_id: upgrade_facts
                                .as_ref()
                                .map(|facts| facts.runtime_artifact_id.clone()),
                            state_schema_version: upgrade_facts
                                .as_ref()
                                .map(|facts| facts.state_schema_version.clone()),
                            runtime_capabilities,
                            runtime_host: upgrade_facts
                                .as_ref()
                                .map(|facts| facts.runtime_host.clone()),
                            published_app_urls: upgrade_facts
                                .as_ref()
                                .map(|facts| facts.published_app_urls.clone()),
                            retirement_snapshot,
                            now: None,
                        },
                    )?;
                    Ok(runtime_control_success_outcome(
                        kind,
                        request_id,
                        completed.agent_runtime_id,
                    ))
                }
                Err(error) => {
                    let failure_message = error.to_string();
                    self.record_runtime_control_failure(
                        kind,
                        &request_id,
                        &lease_token,
                        &failure_message,
                    )?;
                    Ok(runtime_control_failed_outcome(
                        kind,
                        request_id,
                        failure_message,
                    ))
                }
            },
            Err(error) => {
                let failure_message = error.to_string();
                self.record_runtime_control_failure(
                    kind,
                    &request_id,
                    &lease_token,
                    &failure_message,
                )?;
                Ok(runtime_control_failed_outcome(
                    kind,
                    request_id,
                    failure_message,
                ))
            }
        }
    }

    fn record_runtime_control_failure(
        &mut self,
        kind: RuntimeControlKind,
        request_id: &str,
        lease_token: &str,
        failure_message: &str,
    ) -> Result<(), RunnerError> {
        if kind == RuntimeControlKind::Destroy {
            self.queue.retry_runtime_control(
                request_id,
                RetryRuntimeControlRequestInput {
                    request_id: request_id.to_string(),
                    runner_id: self.runner_id.clone(),
                    lease_token: lease_token.to_string(),
                    failure_message: failure_message.to_string(),
                    now: None,
                },
            )?;
        } else {
            self.queue.fail_runtime_control(
                request_id,
                FailRuntimeControlRequestInput {
                    request_id: request_id.to_string(),
                    runner_id: self.runner_id.clone(),
                    lease_token: lease_token.to_string(),
                    failure_message: failure_message.to_string(),
                    now: None,
                },
            )?;
        }
        Ok(())
    }

    fn wait_for_runtime_control_readiness(
        &mut self,
        kind: RuntimeControlKind,
        source_machine_id: &str,
        previous_heartbeat: Option<&str>,
    ) -> Result<(), RunnerError> {
        match kind {
            RuntimeControlKind::Restart
            | RuntimeControlKind::RecoverKnownGoodChatRuntime
            | RuntimeControlKind::Upgrade => {
                self.wait_for_restart_readiness(source_machine_id, previous_heartbeat)
            }
            RuntimeControlKind::Stop | RuntimeControlKind::Destroy => Ok(()),
        }
    }

    fn runtime_launch_options(
        &mut self,
        lease: &AgentCreationLease,
        lease_token: &str,
    ) -> Result<RuntimeLaunchOptions, RunnerError> {
        let runtime_spec = creation_runtime_spec(lease, self.launcher.runner_class())?;
        let environment = runtime_spec
            .map(|spec| spec.environment.clone())
            .unwrap_or_else(|| self.runtime_environment.clone());
        let secret_environment = if let Some(spec) = runtime_spec {
            resolve_runtime_secret_environment(spec, &self.runtime_secret_environment)?
        } else {
            self.runtime_secret_environment.clone()
        };
        validate_runtime_environment(&environment)?;
        validate_runtime_secret_environment(&secret_environment)?;
        validate_runtime_environment_disjoint(&environment, &secret_environment)?;
        let mut options = RuntimeLaunchOptions {
            profile_picture_url: lease.request.profile_picture_url.clone(),
            environment,
            secret_environment,
            ..RuntimeLaunchOptions::default()
        };
        let requires_finite_private = runtime_spec.is_some_and(|spec| {
            spec.secret_references
                .iter()
                .any(|reference| reference == "FINITE_PRIVATE_API_KEY")
        });
        let Some(defaults) = self.default_finite_private.clone() else {
            if requires_finite_private {
                return Err(RunnerError::InvalidRuntimeEnvironment(
                    "RuntimeSpec Finite Private secret reference could not be resolved".to_string(),
                ));
            }
            return Ok(options);
        };
        if runtime_spec.is_some() && !requires_finite_private {
            return Err(RunnerError::InvalidRuntimeEnvironment(
                "RuntimeSpec omitted the Finite Private secret reference".to_string(),
            ));
        }
        let specialization_bundle = specialization_bundle_for_finite_private_profile(&defaults)?;
        if let Some(raw_api_key) = defaults
            .api_key_override
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            options.finite_private = Some(FinitePrivateLaunchKey {
                api_key_id: "operator-override".to_string(),
                raw_api_key: raw_api_key.to_string(),
                base_url: defaults.base_url,
                model: defaults.model,
                revoke_on_launch_failure: false,
                specialization_bundle,
            });
            return Ok(options);
        }
        let source = self.launcher.planned_source(lease);
        let source_host_id = source
            .as_ref()
            .map(|value| value.source_host_id.clone())
            .or_else(|| self.launcher.source_host_id().map(str::to_string));
        let key = self.queue.provision_finite_private_runtime_key(
            &lease.request.id,
            ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: self.runner_id.clone(),
                lease_token: lease_token.to_string(),
                source_host_id,
                source_machine_id: source.as_ref().map(|value| value.source_machine_id.clone()),
                now: None,
            },
        )?;
        options.finite_private = Some(FinitePrivateLaunchKey {
            api_key_id: key.api_key.id,
            raw_api_key: key.raw_api_key,
            base_url: defaults.base_url,
            model: defaults.model,
            revoke_on_launch_failure: true,
            specialization_bundle,
        });
        Ok(options)
    }

    fn wait_for_runtime_heartbeat(&mut self, source_machine_id: &str) -> Result<(), RunnerError> {
        self.wait_for_runtime_heartbeat_after(source_machine_id, None)
    }

    fn wait_for_launch_readiness(&mut self, source_machine_id: &str) -> Result<(), RunnerError> {
        if self.launcher.uses_core_runtime_heartbeat() {
            self.wait_for_runtime_heartbeat(source_machine_id)
        } else {
            Ok(())
        }
    }

    fn wait_for_restart_readiness(
        &mut self,
        source_machine_id: &str,
        previous_last_seen_at: Option<&str>,
    ) -> Result<(), RunnerError> {
        if self.launcher.uses_core_runtime_heartbeat() {
            self.wait_for_runtime_heartbeat_after(source_machine_id, previous_last_seen_at)
        } else {
            Ok(())
        }
    }

    fn wait_for_runtime_heartbeat_after(
        &mut self,
        source_machine_id: &str,
        previous_last_seen_at: Option<&str>,
    ) -> Result<(), RunnerError> {
        let started = Instant::now();
        loop {
            if let Some(heartbeat) = self
                .queue
                .runtime_heartbeat_for_machine(source_machine_id)?
                && previous_last_seen_at
                    .map(|previous| heartbeat.last_seen_at != previous)
                    .unwrap_or(true)
            {
                return Ok(());
            }
            if started.elapsed() >= self.runtime_ready_timeout {
                let heartbeat_description = if previous_last_seen_at.is_some() {
                    "new relay heartbeat"
                } else {
                    "relay heartbeat"
                };
                return Err(RunnerError::RuntimeLaunch(format!(
                    "runtime did not publish a {heartbeat_description} within {}s",
                    self.runtime_ready_timeout.as_secs()
                )));
            }
            thread::sleep(self.runtime_ready_interval);
        }
    }
}

pub trait AgentCreationQueue {
    fn lease_runtime_control(
        &mut self,
        runner_id: &str,
        lease_token: &str,
        lease_seconds: i64,
        source_host_id: Option<&str>,
        runner_capacity: Option<&RunnerLeaseCapacity>,
    ) -> Result<Option<RuntimeControlLease>, RunnerError>;

    fn complete_runtime_control(
        &mut self,
        request_id: &str,
        input: CompleteRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError>;

    fn fail_runtime_control(
        &mut self,
        request_id: &str,
        input: FailRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError>;

    fn renew_runtime_control(
        &mut self,
        request_id: &str,
        input: RenewRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError>;

    fn retry_runtime_control(
        &mut self,
        request_id: &str,
        input: RetryRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError>;

    fn lease_agent_creation(
        &mut self,
        runner_id: &str,
        lease_token: &str,
        lease_seconds: i64,
        runner_capacity: Option<&RunnerLeaseCapacity>,
    ) -> Result<Option<AgentCreationLease>, RunnerError>;

    fn complete_agent_creation(
        &mut self,
        request_id: &str,
        input: CompleteAgentCreationRequestInput,
    ) -> Result<AgentCreationLease, RunnerError>;

    fn register_agent_creation_runtime(
        &mut self,
        request_id: &str,
        input: RegisterAgentCreationRuntimeInput,
    ) -> Result<AgentCreationLease, RunnerError>;

    fn record_provider_operation_transition(
        &mut self,
        request_id: &str,
        input: RecordProviderOperationTransitionRequest,
    ) -> Result<ProviderOperationEnvelope, RunnerError>;

    fn runtime_heartbeat_for_machine(
        &mut self,
        source_machine_id: &str,
    ) -> Result<Option<RelayHeartbeat>, RunnerError>;

    fn provision_finite_private_runtime_key(
        &mut self,
        request_id: &str,
        input: ProvisionFinitePrivateRuntimeKeyInput,
    ) -> Result<ProvisionFinitePrivateRuntimeKeyResult, RunnerError>;

    fn fail_agent_creation(
        &mut self,
        request_id: &str,
        input: FailAgentCreationRequestInput,
    ) -> Result<AgentCreationRequest, RunnerError>;
}

pub trait ProviderOperationJournal {
    fn record(
        &mut self,
        correlation_id: &str,
        placement: RuntimePlacement,
        transition: ProviderOperationTransition,
    ) -> Result<ProviderOperationEnvelope, RunnerError>;
}

struct QueueProviderOperationJournal<'a, Q> {
    queue: &'a mut Q,
    request_id: &'a str,
    runner_id: &'a str,
    lease_token: &'a str,
}

impl<Q> ProviderOperationJournal for QueueProviderOperationJournal<'_, Q>
where
    Q: AgentCreationQueue,
{
    fn record(
        &mut self,
        correlation_id: &str,
        placement: RuntimePlacement,
        transition: ProviderOperationTransition,
    ) -> Result<ProviderOperationEnvelope, RunnerError> {
        self.queue.record_provider_operation_transition(
            self.request_id,
            RecordProviderOperationTransitionRequest {
                runner_id: self.runner_id.to_string(),
                lease_token: self.lease_token.to_string(),
                correlation_id: correlation_id.to_string(),
                placement,
                transition,
            },
        )
    }
}

pub trait RuntimeLauncher {
    fn validate_ready(&self) -> Result<(), RunnerError>;
    fn runtime_capabilities(&self) -> RuntimeCapabilitiesEnvelope;
    fn runner_class(&self) -> RunnerClass {
        RunnerClass::LocalDocker
    }
    fn uses_core_runtime_heartbeat(&self) -> bool {
        true
    }
    fn runner_capacity(&self) -> RunnerLeaseCapacity {
        RunnerLeaseCapacity::default()
    }
    fn source_host_id(&self) -> Option<&str> {
        None
    }
    fn planned_source(&self, _lease: &AgentCreationLease) -> Option<RuntimeSourceIdentity> {
        None
    }
    fn restart_runtime(
        &mut self,
        _lease: &RuntimeControlLease,
        _options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        Err(RunnerError::RuntimeLaunch(
            "runtime restart is not supported by this launcher".to_string(),
        ))
    }
    fn recover_known_good_chat_runtime(
        &mut self,
        _lease: &RuntimeControlLease,
        _options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        Err(RunnerError::RuntimeLaunch(
            "runtime known-good chat recovery is not supported by this launcher".to_string(),
        ))
    }
    fn upgrade_runtime(
        &mut self,
        _lease: &RuntimeControlLease,
        _options: &RuntimeRestartOptions,
    ) -> Result<RuntimeUpgradeFacts, RunnerError> {
        Err(RunnerError::RuntimeLaunch(
            "runtime upgrade is not supported by this launcher".to_string(),
        ))
    }
    fn stop_runtime(&mut self, _lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        Err(RunnerError::RuntimeLaunch(
            "runtime stop is not supported by this launcher".to_string(),
        ))
    }
    fn destroy_runtime(&mut self, _lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        Err(RunnerError::RuntimeLaunch(
            "runtime destroy is not supported by this launcher".to_string(),
        ))
    }
    fn retire_runtime(
        &mut self,
        _lease: &RuntimeControlLease,
        _renew_lease: &mut dyn FnMut() -> Result<(), RunnerError>,
    ) -> Result<RuntimeRetirementSnapshotReceipt, RunnerError> {
        Err(RunnerError::RuntimeLaunch(
            "runtime retirement is not supported by this launcher".to_string(),
        ))
    }
    fn launch(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError>;
    fn launch_with_provider_operation(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
        _journal: &mut dyn ProviderOperationJournal,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        self.launch(lease, options)
    }
    fn cleanup_failed_launch(&mut self, _facts: &RuntimeLaunchFacts) -> Result<(), RunnerError> {
        Ok(())
    }
}

impl<L> RuntimeLauncher for Box<L>
where
    L: RuntimeLauncher + ?Sized,
{
    fn validate_ready(&self) -> Result<(), RunnerError> {
        (**self).validate_ready()
    }

    fn runtime_capabilities(&self) -> RuntimeCapabilitiesEnvelope {
        (**self).runtime_capabilities()
    }

    fn runner_class(&self) -> RunnerClass {
        (**self).runner_class()
    }

    fn uses_core_runtime_heartbeat(&self) -> bool {
        (**self).uses_core_runtime_heartbeat()
    }

    fn runner_capacity(&self) -> RunnerLeaseCapacity {
        (**self).runner_capacity()
    }

    fn source_host_id(&self) -> Option<&str> {
        (**self).source_host_id()
    }

    fn planned_source(&self, lease: &AgentCreationLease) -> Option<RuntimeSourceIdentity> {
        (**self).planned_source(lease)
    }

    fn restart_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        (**self).restart_runtime(lease, options)
    }

    fn recover_known_good_chat_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        (**self).recover_known_good_chat_runtime(lease, options)
    }

    fn upgrade_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        options: &RuntimeRestartOptions,
    ) -> Result<RuntimeUpgradeFacts, RunnerError> {
        (**self).upgrade_runtime(lease, options)
    }

    fn stop_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        (**self).stop_runtime(lease)
    }

    fn destroy_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        (**self).destroy_runtime(lease)
    }

    fn retire_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        renew_lease: &mut dyn FnMut() -> Result<(), RunnerError>,
    ) -> Result<RuntimeRetirementSnapshotReceipt, RunnerError> {
        (**self).retire_runtime(lease, renew_lease)
    }

    fn launch(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        (**self).launch(lease, options)
    }

    fn launch_with_provider_operation(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
        journal: &mut dyn ProviderOperationJournal,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        (**self).launch_with_provider_operation(lease, options, journal)
    }

    fn cleanup_failed_launch(&mut self, facts: &RuntimeLaunchFacts) -> Result<(), RunnerError> {
        (**self).cleanup_failed_launch(facts)
    }
}

pub trait LeaseTokenSource {
    fn next_lease_token(&mut self) -> Result<String, RunnerError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RandomLeaseTokenSource;

impl LeaseTokenSource for RandomLeaseTokenSource {
    fn next_lease_token(&mut self) -> Result<String, RunnerError> {
        let mut bytes = [0_u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        Ok(hex::encode(bytes))
    }
}

fn runtime_control_success_outcome(
    kind: RuntimeControlKind,
    request_id: String,
    runtime_id: String,
) -> RunOnceOutcome {
    match kind {
        RuntimeControlKind::Restart => RunOnceOutcome::RuntimeRestarted {
            request_id,
            runtime_id,
        },
        RuntimeControlKind::RecoverKnownGoodChatRuntime => {
            RunOnceOutcome::RuntimeRecoveredKnownGoodChat {
                request_id,
                runtime_id,
            }
        }
        RuntimeControlKind::Upgrade => RunOnceOutcome::RuntimeUpgraded {
            request_id,
            runtime_id,
        },
        RuntimeControlKind::Stop => RunOnceOutcome::RuntimeStopped {
            request_id,
            runtime_id,
        },
        RuntimeControlKind::Destroy => RunOnceOutcome::RuntimeDestroyed {
            request_id,
            runtime_id,
        },
    }
}

fn runtime_control_failed_outcome(
    kind: RuntimeControlKind,
    request_id: String,
    failure_message: String,
) -> RunOnceOutcome {
    match kind {
        RuntimeControlKind::Restart => RunOnceOutcome::RuntimeRestartFailed {
            request_id,
            failure_message,
        },
        RuntimeControlKind::RecoverKnownGoodChatRuntime => RunOnceOutcome::RuntimeRecoveryFailed {
            request_id,
            failure_message,
        },
        RuntimeControlKind::Upgrade => RunOnceOutcome::RuntimeUpgradeFailed {
            request_id,
            failure_message,
        },
        RuntimeControlKind::Stop => RunOnceOutcome::RuntimeStopFailed {
            request_id,
            failure_message,
        },
        RuntimeControlKind::Destroy => RunOnceOutcome::RuntimeDestroyFailed {
            request_id,
            failure_message,
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLaunchFacts {
    pub source_host_id: String,
    pub source_machine_id: String,
    pub runtime_artifact_id: Option<String>,
    pub state_schema_version: Option<String>,
    #[serde(default)]
    pub provider_runtime_handle: Option<ProviderRuntimeHandleEnvelope>,
    #[serde(default)]
    pub contact_endpoint: Option<String>,
    pub runtime_relay_token_hash: String,
    pub display_name: Option<String>,
    pub hostname: Option<String>,
    pub runtime_host: Option<String>,
    pub runtime_status: RuntimeSummaryStatus,
    pub active_inference_profile: Option<String>,
    pub hermes_available: Option<bool>,
    pub published_app_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpgradeFacts {
    pub runtime_artifact_id: String,
    pub state_schema_version: String,
    pub runtime_host: String,
    pub published_app_urls: Vec<String>,
}

enum RuntimeControlCompletionFacts {
    None,
    Upgrade(RuntimeUpgradeFacts),
    Retirement(Box<RuntimeRetirementSnapshotReceipt>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSourceIdentity {
    pub source_host_id: String,
    pub source_machine_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinitePrivateRuntimeDefaults {
    pub base_url: String,
    pub model: String,
    pub api_key_override: Option<String>,
    pub specialization_bundle: Option<SpecializationBundleRuntimeDefaults>,
}

impl Default for FinitePrivateRuntimeDefaults {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_FINITE_PRIVATE_BASE_URL.to_string(),
            model: DEFAULT_FINITE_PRIVATE_MODEL.to_string(),
            api_key_override: None,
            specialization_bundle: None,
        }
    }
}

fn specialization_bundle_for_finite_private_profile(
    defaults: &FinitePrivateRuntimeDefaults,
) -> Result<Option<SpecializationBundleRuntimeDefaults>, RunnerError> {
    // This is a profile decision, never a host, customer, or agent-name decision.
    if defaults.model != DEFAULT_FINITE_PRIVATE_MODEL {
        return Ok(None);
    }
    defaults
        .specialization_bundle
        .clone()
        .map(|mut specialization_bundle| {
            if specialization_bundle.bundle_id != DEFAULT_FINITE_PRIVATE_SPECIALIZATION_BUNDLE {
                return Err(RunnerError::InvalidRuntimeEnvironment(format!(
                    "unsupported Finite Private specialization bundle {:?}",
                    specialization_bundle.bundle_id
                )));
            }
            specialization_bundle.worker_api_key =
                specialization_bundle.worker_api_key.trim().to_owned();
            if specialization_bundle.worker_api_key.is_empty()
                || specialization_bundle.worker_api_key.len()
                    > MAX_RUNTIME_SECRET_ENVIRONMENT_VALUE_BYTES
            {
                return Err(RunnerError::InvalidRuntimeEnvironment(
                    "Finite Private specialization worker credential is empty or oversized"
                        .to_string(),
                ));
            }
            Ok(specialization_bundle)
        })
        .transpose()
}

#[derive(Clone, PartialEq, Eq)]
pub struct SpecializationBundleRuntimeDefaults {
    pub bundle_id: String,
    pub worker_api_key: String,
}

impl std::fmt::Debug for SpecializationBundleRuntimeDefaults {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SpecializationBundleRuntimeDefaults")
            .field("bundle_id", &self.bundle_id)
            .field("worker_api_key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct RuntimeLaunchOptions {
    pub finite_private: Option<FinitePrivateLaunchKey>,
    /// Public profile metadata selected before launch. It is not a secret and
    /// stays provider-neutral.
    pub profile_picture_url: Option<String>,
    /// Bounded non-secret values from the provider-neutral RuntimeSpec.
    pub environment: BTreeMap<String, String>,
    /// Bounded secret values resolved by the operator-side launch boundary.
    /// Adapters transport these opaquely and never log their values.
    pub secret_environment: BTreeMap<String, String>,
}

/// Provider-neutral desired environment carried through state-preserving
/// Runtime lifecycle operations. Public values may be reconciled by replacement;
/// secret values are populated only for an explicit image upgrade from a
/// Core-bound RuntimeSpec. Runtime-contract variables remain owned by the
/// existing Runtime.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RuntimeRestartOptions {
    environment: BTreeMap<String, String>,
    secret_environment: BTreeMap<String, String>,
}

fn runtime_spec_v1(envelope: &RuntimeSpecEnvelope) -> &RuntimeSpecV1 {
    match envelope {
        RuntimeSpecEnvelope::V1(spec) => spec,
    }
}

fn creation_runtime_spec(
    lease: &AgentCreationLease,
    runner_class: RunnerClass,
) -> Result<Option<&RuntimeSpecV1>, RunnerError> {
    let Some(envelope) = lease.request.runtime_spec.as_ref() else {
        return Ok(None);
    };
    let spec = runtime_spec_v1(envelope);
    let expected_runtime_id = lease.request.agent_runtime_id.as_deref().ok_or_else(|| {
        RunnerError::RuntimeLaunch(
            "Core-bound RuntimeSpec did not reserve an Agent Runtime id".to_string(),
        )
    })?;
    let placement = lease
        .request
        .placement
        .or(lease.project.placement)
        .ok_or_else(|| {
            RunnerError::RuntimeLaunch(
                "Core-bound RuntimeSpec did not have persisted placement".to_string(),
            )
        })?;
    validate_runtime_spec_contract(spec, runner_class)?;
    if spec.operation_id != lease.request.id
        || spec.project_id != lease.project.id
        || spec.agent_runtime_id != expected_runtime_id
        || spec.placement != placement
        || lease.request.runner_class != runner_class
        || lease.request.desired_runtime_artifact_id.as_deref()
            != Some(spec.runtime_artifact_id.as_str())
        || spec.boot_intent != RuntimeBootIntent::Normal
    {
        return Err(RunnerError::RuntimeLaunch(
            "Core-bound RuntimeSpec did not match its creation lease".to_string(),
        ));
    }
    Ok(Some(spec))
}

fn control_runtime_spec(
    lease: &RuntimeControlLease,
    runner_class: RunnerClass,
) -> Result<Option<&RuntimeSpecV1>, RunnerError> {
    let Some(envelope) = lease.runtime_spec.as_ref() else {
        return Ok(None);
    };
    let spec = runtime_spec_v1(envelope);
    let placement = lease.runtime.placement.ok_or_else(|| {
        RunnerError::RuntimeLaunch(
            "Core-bound lifecycle RuntimeSpec did not have persisted placement".to_string(),
        )
    })?;
    validate_runtime_spec_contract(spec, runner_class)?;
    let expected_artifact_id = if lease.request.kind == RuntimeControlKind::Upgrade {
        lease.request.target_runtime_artifact_id.as_deref()
    } else {
        lease.runtime.runtime_artifact_id.as_deref()
    };
    let expected_schema = if lease.request.kind == RuntimeControlKind::Upgrade {
        lease
            .target_runtime_artifact
            .as_ref()
            .map(|artifact| artifact.state_schema_version.as_str())
    } else {
        lease.runtime.state_schema_version.as_deref()
    };
    let expected_boot_intent = match lease.request.kind {
        RuntimeControlKind::RecoverKnownGoodChatRuntime => RuntimeBootIntent::RecoverKnownGood,
        RuntimeControlKind::Restart
        | RuntimeControlKind::Upgrade
        | RuntimeControlKind::Stop
        | RuntimeControlKind::Destroy => RuntimeBootIntent::Normal,
    };
    if spec.operation_id != lease.request.id
        || spec.project_id != lease.runtime.project_id
        || spec.agent_runtime_id != lease.runtime.id
        || spec.placement != placement
        || expected_artifact_id != Some(spec.runtime_artifact_id.as_str())
        || expected_schema != Some(spec.state_schema_version.as_str())
        || spec.boot_intent != expected_boot_intent
    {
        return Err(RunnerError::RuntimeLaunch(
            "Core-bound RuntimeSpec did not match its lifecycle lease".to_string(),
        ));
    }
    Ok(Some(spec))
}

fn validate_runtime_spec_contract(
    spec: &RuntimeSpecV1,
    runner_class: RunnerClass,
) -> Result<(), RunnerError> {
    let expected_resource_class = match runner_class {
        RunnerClass::Kata => Some(RuntimeResourceClass::Vcpu4Memory8Gib),
        RunnerClass::Phala => Some(RuntimeResourceClass::Vcpu2Memory4Gib),
        RunnerClass::LocalDocker | RunnerClass::AppleContainer | RunnerClass::Enclavia => None,
    };
    if spec.placement.runner_class != runner_class
        || expected_resource_class
            .is_some_and(|resource_class| spec.placement.runtime_resource_class != resource_class)
        || !runtime_spec_image_is_immutable(&spec.runtime_image_digest)
        || spec.durable_state_id.trim().is_empty()
        || spec.endpoints.service_port != DEFAULT_DOCKER_CONTAINER_PORT
        || spec.endpoints.health_path != "/healthz"
        || spec.endpoints.contact_path != "/contact"
    {
        return Err(RunnerError::RuntimeLaunch(
            "Core-bound RuntimeSpec violated the Runner contract".to_string(),
        ));
    }
    validate_runtime_environment(&spec.environment)?;
    let mut references = std::collections::BTreeSet::new();
    for reference in &spec.secret_references {
        if !valid_runtime_environment_key(reference)
            || spec.environment.contains_key(reference)
            || !references.insert(reference)
            || (reference != "FINITE_PRIVATE_API_KEY" && !secret_runtime_environment_key(reference))
        {
            return Err(RunnerError::InvalidRuntimeEnvironment(
                "RuntimeSpec secret references were invalid or duplicated".to_string(),
            ));
        }
    }
    Ok(())
}

fn runtime_spec_image_is_immutable(reference: &str) -> bool {
    let Some((repository, digest)) = reference.trim().rsplit_once("@sha256:") else {
        return false;
    };
    !repository.is_empty()
        && digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl RuntimeRestartOptions {
    pub fn new(environment: BTreeMap<String, String>) -> Result<Self, RunnerError> {
        validate_runtime_environment(&environment)?;
        Ok(Self {
            environment,
            secret_environment: BTreeMap::new(),
        })
    }

    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    pub fn with_secret_environment(
        mut self,
        secret_environment: BTreeMap<String, String>,
    ) -> Result<Self, RunnerError> {
        validate_runtime_secret_environment(&secret_environment)?;
        validate_runtime_environment_disjoint(&self.environment, &secret_environment)?;
        self.secret_environment = secret_environment;
        Ok(self)
    }

    pub fn secret_environment(&self) -> &BTreeMap<String, String> {
        &self.secret_environment
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct FinitePrivateLaunchKey {
    pub api_key_id: String,
    pub raw_api_key: String,
    pub base_url: String,
    pub model: String,
    pub revoke_on_launch_failure: bool,
    /// Optional automatic profile activation. A missing host credential must
    /// never prevent a normal Finite Private agent launch.
    pub specialization_bundle: Option<SpecializationBundleRuntimeDefaults>,
}

fn provisioned_key_to_revoke(options: &RuntimeLaunchOptions) -> Option<String> {
    options
        .finite_private
        .as_ref()
        .filter(|key| key.revoke_on_launch_failure)
        .map(|key| key.api_key_id.clone())
}

impl std::fmt::Debug for RuntimeLaunchOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeLaunchOptions")
            .field("finite_private", &self.finite_private)
            .field("has_profile_picture", &self.profile_picture_url.is_some())
            .field(
                "environment_keys",
                &self.environment.keys().collect::<Vec<_>>(),
            )
            .field(
                "secret_environment_keys",
                &self.secret_environment.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl std::fmt::Debug for RuntimeRestartOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeRestartOptions")
            .field(
                "environment_keys",
                &self.environment.keys().collect::<Vec<_>>(),
            )
            .field(
                "secret_environment_keys",
                &self.secret_environment.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Reconcile only the explicitly desired opaque keys. Existing Runtime-contract
/// values, provider settings, and credentials are retained byte-for-byte so
/// compute replacement does not silently erase them.
fn merge_desired_runtime_environment(
    mut existing: Vec<(String, String)>,
    options: &RuntimeRestartOptions,
) -> Vec<(String, String)> {
    for (key, value) in &mut existing {
        if let Some(desired) = options.environment().get(key) {
            *value = desired.clone();
        }
    }
    for (key, value) in options.environment() {
        if !existing.iter().any(|(existing_key, _)| existing_key == key) {
            existing.push((key.clone(), value.clone()));
        }
    }
    for (key, value) in options.secret_environment() {
        if let Some((_, existing_value)) = existing
            .iter_mut()
            .rfind(|(existing_key, _)| existing_key == key)
        {
            *existing_value = value.clone();
        } else {
            existing.push((key.clone(), value.clone()));
        }
    }
    existing
}

fn resolve_runtime_secret_environment(
    spec: &RuntimeSpecV1,
    available: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, RunnerError> {
    let mut resolved = BTreeMap::new();
    for reference in &spec.secret_references {
        if reference == "FINITE_PRIVATE_API_KEY" {
            continue;
        }
        let value = available.get(reference).cloned().ok_or_else(|| {
            RunnerError::InvalidRuntimeEnvironment(format!(
                "RuntimeSpec secret reference {reference} is unavailable"
            ))
        })?;
        resolved.insert(reference.clone(), value);
    }
    validate_runtime_secret_environment(&resolved)?;
    validate_runtime_environment_disjoint(&spec.environment, &resolved)?;
    Ok(resolved)
}

fn validate_runtime_environment(environment: &BTreeMap<String, String>) -> Result<(), RunnerError> {
    if environment.len() > MAX_RUNTIME_ENVIRONMENT_ENTRIES {
        return Err(RunnerError::InvalidRuntimeEnvironment(format!(
            "at most {MAX_RUNTIME_ENVIRONMENT_ENTRIES} entries are allowed"
        )));
    }
    let mut total_bytes = 0usize;
    for (key, value) in environment {
        if !valid_runtime_environment_key(key) {
            return Err(RunnerError::InvalidRuntimeEnvironment(format!(
                "{key:?} is not a bounded uppercase environment name"
            )));
        }
        if reserved_runtime_environment_key(key) {
            return Err(RunnerError::InvalidRuntimeEnvironment(format!(
                "{key} is owned by the Runtime contract"
            )));
        }
        if secret_runtime_environment_key(key) {
            return Err(RunnerError::InvalidRuntimeEnvironment(format!(
                "{key} looks secret-bearing; use a secret reference instead"
            )));
        }
        if value.is_empty()
            || value.len() > MAX_RUNTIME_ENVIRONMENT_VALUE_BYTES
            || value.contains('\0')
        {
            return Err(RunnerError::InvalidRuntimeEnvironment(format!(
                "{key} has an empty, oversized, or NUL-containing value"
            )));
        }
        total_bytes = total_bytes
            .saturating_add(key.len())
            .saturating_add(value.len());
    }
    if total_bytes > MAX_RUNTIME_ENVIRONMENT_TOTAL_BYTES {
        return Err(RunnerError::InvalidRuntimeEnvironment(format!(
            "values exceed the {MAX_RUNTIME_ENVIRONMENT_TOTAL_BYTES}-byte total limit"
        )));
    }
    Ok(())
}

fn validate_runtime_secret_environment(
    environment: &BTreeMap<String, String>,
) -> Result<(), RunnerError> {
    if environment.len() > MAX_RUNTIME_SECRET_ENVIRONMENT_ENTRIES {
        return Err(RunnerError::InvalidRuntimeEnvironment(format!(
            "at most {MAX_RUNTIME_SECRET_ENVIRONMENT_ENTRIES} secret entries are allowed"
        )));
    }
    let mut total_bytes = 0usize;
    for (key, value) in environment {
        if !valid_runtime_environment_key(key) {
            return Err(RunnerError::InvalidRuntimeEnvironment(format!(
                "{key:?} is not a bounded uppercase secret environment name"
            )));
        }
        if reserved_runtime_environment_key(key) {
            return Err(RunnerError::InvalidRuntimeEnvironment(format!(
                "{key} is owned by the Runtime contract"
            )));
        }
        if value.is_empty()
            || value.len() > MAX_RUNTIME_SECRET_ENVIRONMENT_VALUE_BYTES
            || value.contains('\0')
            || value
                .chars()
                .any(|character| matches!(character, '\n' | '\r'))
        {
            return Err(RunnerError::InvalidRuntimeEnvironment(format!(
                "{key} has an empty, oversized, or multiline secret value"
            )));
        }
        total_bytes = total_bytes
            .saturating_add(key.len())
            .saturating_add(value.len());
    }
    if total_bytes > MAX_RUNTIME_SECRET_ENVIRONMENT_TOTAL_BYTES {
        return Err(RunnerError::InvalidRuntimeEnvironment(format!(
            "secret values exceed the {MAX_RUNTIME_SECRET_ENVIRONMENT_TOTAL_BYTES}-byte total limit"
        )));
    }
    Ok(())
}

fn validate_runtime_environment_disjoint(
    environment: &BTreeMap<String, String>,
    secret_environment: &BTreeMap<String, String>,
) -> Result<(), RunnerError> {
    if let Some(key) = environment
        .keys()
        .find(|key| secret_environment.contains_key(*key))
    {
        return Err(RunnerError::InvalidRuntimeEnvironment(format!(
            "{key} appears in both public and secret Runtime environment"
        )));
    }
    Ok(())
}

fn valid_runtime_environment_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_RUNTIME_ENVIRONMENT_KEY_BYTES
        && key.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
        })
}

fn reserved_runtime_environment_key(key: &str) -> bool {
    matches!(
        key,
        "FINITE_SERVER_URL"
            | "FINITECHAT_SERVER_URL"
            | "FINITE_AGENT_BOOT_INTENT_JSON"
            | "FINITE_AGENT_STATE_ROOT"
            | "FINITECHAT_HOME"
            | "FINITE_HOME"
            | "HERMES_HOME"
            | "FINITECHAT_WORKSPACE"
            | "FINITE_AGENT_HTTP_HOST"
            | "FINITE_AGENT_HTTP_PORT"
            | "FINITECHAT_HERMES_AGENT_DEVICE_ID"
            | "FINITE_AGENT_ID"
            | "FINITE_AGENT_NAME"
            | "FINITECHAT_HERMES_AGENT_NAME"
            | "FINITECHAT_HERMES_ROOM_NAME"
            | "FINITECHAT_HERMES_AGENT_PICTURE_URL"
            | "FINITECHAT_HERMES_INBOUND_STREAM"
            | "FINITECHAT_ALLOW_ALL_USERS"
            | "FINITE_ALLOW_ALL_USERS"
            | "GATEWAY_ALLOW_ALL_USERS"
            | "FINITE_DEFAULT_INFERENCE_PROFILE"
            | "FINITE_PRIVATE_MODEL"
            | "FINITE_PRIVATE_BASE_URL"
            | "FINITE_PRIVATE_API_KEY"
            | "FINITECHAT_HERMES_MODEL"
            | "FINITECHAT_HERMES_PROVIDER"
            | "FINITECHAT_HERMES_BASE_URL"
            | "FINITECHAT_HERMES_API_MODE"
            | "FINITE_SPECIALIZATION_BUNDLE"
            | "FINITE_SPECIALIZATION_WORKER_API_KEY"
            | "OPENAI_API_KEY"
    )
}

fn secret_runtime_environment_key(key: &str) -> bool {
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"]
        .iter()
        .any(|part| key.split('_').any(|segment| segment == *part))
}

impl std::fmt::Debug for FinitePrivateLaunchKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FinitePrivateLaunchKey")
            .field("api_key_id", &self.api_key_id)
            .field("raw_api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("revoke_on_launch_failure", &self.revoke_on_launch_failure)
            .field("specialization_bundle", &self.specialization_bundle)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct CoreHttpAgentCreationQueue {
    base_url: String,
    api_token: String,
}

impl CoreHttpAgentCreationQueue {
    pub fn new(
        base_url: impl Into<String>,
        api_token: impl Into<String>,
    ) -> Result<Self, RunnerError> {
        let base_url = base_url.into().trim().trim_end_matches('/').to_string();
        let api_token = api_token.into().trim().to_string();
        if base_url.is_empty() {
            return Err(RunnerError::CoreRequest("Core URL is required".to_string()));
        }
        if api_token.is_empty() {
            return Err(RunnerError::CoreRequest(
                "Core API token is required".to_string(),
            ));
        }
        Ok(Self {
            base_url,
            api_token,
        })
    }

    fn post_json<T, B>(&self, path: &str, body: &B) -> Result<T, RunnerError>
    where
        T: serde::de::DeserializeOwned,
        B: Serialize,
    {
        let url = format!("{}{}", self.base_url, path);
        let response = ureq::post(&url)
            .set("authorization", &format!("Bearer {}", self.api_token))
            .send_json(serde_json::to_value(body).map_err(|error| {
                RunnerError::CoreRequest(format!("failed to encode request body: {error}"))
            })?);
        decode_core_response(response)
    }

    pub fn runtime_artifact(&self, artifact_id: &str) -> Result<RuntimeArtifact, RunnerError> {
        let artifact_id = artifact_id.trim();
        if artifact_id.is_empty() {
            return Err(RunnerError::MissingRuntimeArtifactReference);
        }
        let url = format!(
            "{}/api/core/v1/runtime-artifacts/{}",
            self.base_url, artifact_id
        );
        let response = ureq::get(&url)
            .set("authorization", &format!("Bearer {}", self.api_token))
            .call();
        decode_core_response(response)
    }
}

impl AgentCreationQueue for CoreHttpAgentCreationQueue {
    fn lease_runtime_control(
        &mut self,
        runner_id: &str,
        lease_token: &str,
        lease_seconds: i64,
        source_host_id: Option<&str>,
        runner_capacity: Option<&RunnerLeaseCapacity>,
    ) -> Result<Option<RuntimeControlLease>, RunnerError> {
        self.post_json(
            "/api/core/v1/runtime-control-requests/lease",
            &LeaseRuntimeControlRequestInput {
                runner_id: runner_id.to_string(),
                lease_token: lease_token.to_string(),
                lease_seconds: Some(lease_seconds),
                source_host_id: source_host_id.map(str::to_string),
                runner_capacity: runner_capacity.cloned(),
                now: None,
            },
        )
    }

    fn renew_runtime_control(
        &mut self,
        request_id: &str,
        input: RenewRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        self.post_json(
            &format!("/api/core/v1/runtime-control-requests/{}/renew", request_id),
            &input,
        )
    }

    fn retry_runtime_control(
        &mut self,
        request_id: &str,
        input: RetryRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        self.post_json(
            &format!("/api/core/v1/runtime-control-requests/{}/retry", request_id),
            &input,
        )
    }

    fn complete_runtime_control(
        &mut self,
        request_id: &str,
        input: CompleteRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        self.post_json(
            &format!(
                "/api/core/v1/runtime-control-requests/{}/complete",
                request_id
            ),
            &input,
        )
    }

    fn fail_runtime_control(
        &mut self,
        request_id: &str,
        input: FailRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        self.post_json(
            &format!("/api/core/v1/runtime-control-requests/{}/fail", request_id),
            &input,
        )
    }

    fn lease_agent_creation(
        &mut self,
        runner_id: &str,
        lease_token: &str,
        lease_seconds: i64,
        runner_capacity: Option<&RunnerLeaseCapacity>,
    ) -> Result<Option<AgentCreationLease>, RunnerError> {
        self.post_json(
            "/api/core/v1/agent-creation-requests/lease",
            &finite_saas_core::LeaseAgentCreationRequestInput {
                runner_id: runner_id.to_string(),
                source_host_id: None,
                lease_token: lease_token.to_string(),
                lease_seconds: Some(lease_seconds),
                runner_capacity: runner_capacity.cloned(),
                now: None,
            },
        )
    }

    fn complete_agent_creation(
        &mut self,
        request_id: &str,
        input: CompleteAgentCreationRequestInput,
    ) -> Result<AgentCreationLease, RunnerError> {
        self.post_json(
            &format!(
                "/api/core/v1/agent-creation-requests/{}/complete",
                request_id
            ),
            &input,
        )
    }

    fn register_agent_creation_runtime(
        &mut self,
        request_id: &str,
        input: RegisterAgentCreationRuntimeInput,
    ) -> Result<AgentCreationLease, RunnerError> {
        self.post_json(
            &format!(
                "/api/core/v1/agent-creation-requests/{}/runtime",
                request_id
            ),
            &input,
        )
    }

    fn record_provider_operation_transition(
        &mut self,
        request_id: &str,
        input: RecordProviderOperationTransitionRequest,
    ) -> Result<ProviderOperationEnvelope, RunnerError> {
        self.post_json(
            &format!(
                "/api/core/v1/agent-creation-requests/{}/provider-operation/transitions",
                request_id
            ),
            &input,
        )
    }

    fn runtime_heartbeat_for_machine(
        &mut self,
        source_machine_id: &str,
    ) -> Result<Option<RelayHeartbeat>, RunnerError> {
        let url = format!(
            "{}/api/finite/v1/machines/{}/heartbeat",
            self.base_url, source_machine_id
        );
        let response = ureq::get(&url)
            .set("authorization", &format!("Bearer {}", self.api_token))
            .call();
        match response {
            Ok(response) => response
                .into_json()
                .map(Some)
                .map_err(|error| RunnerError::CoreJson(error.to_string())),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(error) => decode_core_response::<RelayHeartbeat>(Err(error)).map(Some),
        }
    }

    fn provision_finite_private_runtime_key(
        &mut self,
        request_id: &str,
        input: ProvisionFinitePrivateRuntimeKeyInput,
    ) -> Result<ProvisionFinitePrivateRuntimeKeyResult, RunnerError> {
        self.post_json(
            &format!(
                "/api/core/v1/agent-creation-requests/{}/finite-private-key",
                request_id
            ),
            &input,
        )
    }

    fn fail_agent_creation(
        &mut self,
        request_id: &str,
        input: FailAgentCreationRequestInput,
    ) -> Result<AgentCreationRequest, RunnerError> {
        self.post_json(
            &format!("/api/core/v1/agent-creation-requests/{}/fail", request_id),
            &input,
        )
    }
}

fn decode_core_response<T>(response: Result<ureq::Response, ureq::Error>) -> Result<T, RunnerError>
where
    T: serde::de::DeserializeOwned,
{
    match response {
        Ok(response) => response
            .into_json()
            .map_err(|error| RunnerError::CoreJson(error.to_string())),
        Err(ureq::Error::Status(status, response)) => {
            let body = response
                .into_string()
                .unwrap_or_else(|_| "<unreadable response body>".to_string());
            Err(RunnerError::CoreStatus { status, body })
        }
        Err(error) => Err(RunnerError::CoreRequest(error.to_string())),
    }
}

#[derive(Debug, Clone)]
pub struct DockerConfig {
    pub docker_bin: PathBuf,
    pub source_host_id: String,
    pub image: String,
    pub runtime_artifact_id: Option<String>,
    pub runtime_artifact_kind: Option<RuntimeArtifactKind>,
    pub runtime_state_schema_version: Option<String>,
    pub work_root: PathBuf,
    pub finitechat_server_url: String,
    pub agent_picture_url: String,
    pub host_port: u16,
    pub container_port: u16,
    pub public_base_url: Option<String>,
    pub pull_policy: Option<String>,
    pub max_container_count: Option<u32>,
    pub drain_new_leases: bool,
    pub available_memory_bytes: Option<u64>,
    pub command_timeout: Duration,
    pub launch_timeout: Duration,
    pub readiness_timeout: Duration,
    pub readiness_interval: Duration,
}

impl DockerConfig {
    pub fn validate(&self) -> Result<(), RunnerError> {
        if self.docker_bin.as_os_str().is_empty() {
            return Err(RunnerError::MissingDockerBinary);
        }
        if self.source_host_id.trim().is_empty() {
            return Err(RunnerError::MissingSourceHostId);
        }
        if self.image.trim().is_empty() {
            return Err(RunnerError::MissingRuntimeArtifactReference);
        }
        if self.work_root.as_os_str().is_empty() {
            return Err(RunnerError::MissingWorkRoot);
        }
        if self.finitechat_server_url.trim().is_empty() {
            return Err(RunnerError::MissingFinitechatServerUrl);
        }
        if self.host_port == 0 || self.container_port == 0 {
            return Err(RunnerError::InvalidDockerHostPort);
        }
        if let Some(kind) = self.runtime_artifact_kind
            && kind != RuntimeArtifactKind::OciImage
        {
            return Err(RunnerError::RuntimeLaunch(format!(
                "Docker launcher requires an OCI image artifact, got {}",
                kind.as_str()
            )));
        }
        if let Some(policy) = self.pull_policy.as_deref() {
            match policy.trim() {
                "" | "always" | "missing" | "never" => {}
                other => {
                    return Err(RunnerError::RuntimeLaunch(format!(
                        "invalid Docker pull policy {other:?}; use always, missing, or never"
                    )));
                }
            }
        }
        Ok(())
    }

    fn public_base_url(&self) -> String {
        self.public_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.trim_end_matches('/').to_string())
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", self.host_port))
    }
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            docker_bin: PathBuf::from("docker"),
            source_host_id: String::new(),
            image: String::new(),
            runtime_artifact_id: None,
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: None,
            work_root: PathBuf::new(),
            finitechat_server_url: DEFAULT_FINITECHAT_SERVER_URL.to_string(),
            agent_picture_url: DEFAULT_FINITE_AGENT_PICTURE_URL.to_string(),
            host_port: 18080,
            container_port: DEFAULT_DOCKER_CONTAINER_PORT,
            public_base_url: None,
            pull_policy: Some("missing".to_string()),
            max_container_count: None,
            drain_new_leases: false,
            available_memory_bytes: None,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            launch_timeout: DEFAULT_LAUNCH_TIMEOUT,
            readiness_timeout: DEFAULT_RUNTIME_READY_TIMEOUT,
            readiness_interval: DEFAULT_RUNTIME_READY_INTERVAL,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DockerLauncher {
    config: DockerConfig,
}

impl DockerLauncher {
    pub fn new(config: DockerConfig) -> Self {
        Self { config }
    }

    pub fn plan_launch(&self, lease: &AgentCreationLease) -> DockerLaunchPlan {
        docker_launch_plan(&self.config, lease)
    }

    fn run_command(&self, command: PlannedCommand, timeout: Duration) -> Result<(), RunnerError> {
        let _ = self.run_command_capture(command, timeout)?;
        Ok(())
    }

    fn run_command_capture(
        &self,
        command: PlannedCommand,
        timeout: Duration,
    ) -> Result<String, RunnerError> {
        let mut process = Command::new(&command.program);
        process
            .args(&command.args)
            .envs(command.env.iter().map(|(key, value)| (key, value)));
        if let Some(cwd) = command.cwd.as_ref() {
            process.current_dir(cwd);
        }
        let child = process
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| RunnerError::CommandExecution {
                program: command.program.display().to_string(),
                message: error.to_string(),
            })?;
        let output = wait_with_captured_output(child, &command.program, timeout)?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if output.status.success() {
            return Ok(stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(RunnerError::CommandExecution {
            program: command.program.display().to_string(),
            message: format!(
                "exit status {} stdout={stdout} stderr={stderr}",
                output.status
            ),
        })
    }

    fn remove_container_if_present(&self, container_name: &str) {
        let _ = self.run_command(
            PlannedCommand {
                program: self.config.docker_bin.clone(),
                cwd: None,
                args: vec![
                    OsString::from("rm"),
                    OsString::from("-f"),
                    OsString::from(container_name),
                ],
                env: Vec::new(),
            },
            self.config.command_timeout,
        );
    }

    fn wait_for_runtime_http(&self, plan: &DockerLaunchPlan) -> Result<(), RunnerError> {
        wait_for_http_json_ready(
            &plan.health_url,
            "Docker runtime /healthz",
            self.config.readiness_timeout,
            self.config.readiness_interval,
        )
    }
}

impl RuntimeLauncher for DockerLauncher {
    fn runtime_capabilities(&self) -> RuntimeCapabilitiesEnvelope {
        state_preserving_runtime_capabilities(false)
    }

    fn runner_class(&self) -> RunnerClass {
        RunnerClass::LocalDocker
    }

    fn validate_ready(&self) -> Result<(), RunnerError> {
        self.config.validate()
    }

    fn uses_core_runtime_heartbeat(&self) -> bool {
        false
    }

    fn runner_capacity(&self) -> RunnerLeaseCapacity {
        RunnerLeaseCapacity {
            runner_classes: vec![self.runner_class()],
            draining: self.config.drain_new_leases,
            max_sandbox_count: self.config.max_container_count,
            active_sandbox_count: active_docker_container_count(&self.config),
            available_memory_bytes: self.config.available_memory_bytes,
            runtime_capabilities: Some(self.runtime_capabilities()),
        }
    }

    fn source_host_id(&self) -> Option<&str> {
        Some(&self.config.source_host_id)
    }

    fn planned_source(&self, lease: &AgentCreationLease) -> Option<RuntimeSourceIdentity> {
        let plan = self.plan_launch(lease);
        Some(RuntimeSourceIdentity {
            source_host_id: self.config.source_host_id.clone(),
            source_machine_id: plan.container_name,
        })
    }

    fn restart_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        _options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        self.validate_ready()?;
        if lease.runtime.source_host_id != self.config.source_host_id {
            return Err(RunnerError::RuntimeLaunch(format!(
                "runtime belongs to source host {}, not {}",
                lease.runtime.source_host_id, self.config.source_host_id
            )));
        }
        if lease.runtime.source_machine_id != lease.request.source_machine_id {
            return Err(RunnerError::RuntimeLaunch(format!(
                "runtime source machine {} did not match restart request {}",
                lease.runtime.source_machine_id, lease.request.source_machine_id
            )));
        }
        self.run_command(
            PlannedCommand {
                program: self.config.docker_bin.clone(),
                cwd: None,
                args: vec![
                    OsString::from("restart"),
                    OsString::from(&lease.request.source_machine_id),
                ],
                env: Vec::new(),
            },
            self.config.command_timeout,
        )?;
        let plan = docker_launch_plan_for_source_machine(
            &self.config,
            &lease.request.source_machine_id,
            &lease.runtime.host_facts.display_name,
            &lease.runtime.project_id,
        );
        self.wait_for_runtime_http(&plan)
    }

    fn recover_known_good_chat_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        self.restart_runtime(lease, options)
    }

    fn stop_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        self.validate_ready()?;
        if lease.runtime.source_host_id != self.config.source_host_id {
            return Err(RunnerError::RuntimeLaunch(format!(
                "runtime belongs to source host {}, not {}",
                lease.runtime.source_host_id, self.config.source_host_id
            )));
        }
        if lease.runtime.source_machine_id != lease.request.source_machine_id {
            return Err(RunnerError::RuntimeLaunch(format!(
                "runtime source machine {} did not match stop request {}",
                lease.runtime.source_machine_id, lease.request.source_machine_id
            )));
        }
        self.run_command(
            PlannedCommand {
                program: self.config.docker_bin.clone(),
                cwd: None,
                args: vec![
                    OsString::from("stop"),
                    OsString::from(&lease.request.source_machine_id),
                ],
                env: Vec::new(),
            },
            self.config.command_timeout,
        )
    }

    fn destroy_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        self.validate_ready()?;
        if lease.runtime.source_host_id != self.config.source_host_id {
            return Err(RunnerError::RuntimeLaunch(format!(
                "runtime belongs to source host {}, not {}",
                lease.runtime.source_host_id, self.config.source_host_id
            )));
        }
        if lease.runtime.source_machine_id != lease.request.source_machine_id {
            return Err(RunnerError::RuntimeLaunch(format!(
                "runtime source machine {} did not match destroy request {}",
                lease.runtime.source_machine_id, lease.request.source_machine_id
            )));
        }
        self.run_command(
            PlannedCommand {
                program: self.config.docker_bin.clone(),
                cwd: None,
                args: vec![
                    OsString::from("rm"),
                    OsString::from("-f"),
                    OsString::from(&lease.request.source_machine_id),
                ],
                env: Vec::new(),
            },
            self.config.command_timeout,
        )?;
        // Runtime destruction tears down replaceable compute only. Durable
        // user state has an independent recovery lifecycle and is never
        // purged as a side effect of a runtime-control request.
        Ok(())
    }

    fn launch(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        self.validate_ready()?;
        let plan = self.plan_launch(lease);
        std::fs::create_dir_all(&plan.state_root)
            .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;

        self.remove_container_if_present(&plan.container_name);
        self.run_command(
            docker_run_command(&self.config, &plan, lease, options),
            self.config.launch_timeout,
        )?;
        self.wait_for_runtime_http(&plan)?;

        let runtime_bootstrap_token = random_runtime_bootstrap_token();
        let runtime_relay_token_hash = hash_runtime_relay_token(&runtime_bootstrap_token)
            .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;

        Ok(RuntimeLaunchFacts {
            source_host_id: self.config.source_host_id.clone(),
            source_machine_id: plan.container_name,
            runtime_artifact_id: self.config.runtime_artifact_id.clone(),
            state_schema_version: self.config.runtime_state_schema_version.clone(),
            provider_runtime_handle: None,
            contact_endpoint: Some(plan.contact_url.clone()),
            runtime_relay_token_hash,
            display_name: Some(lease.project.display_name.clone()),
            hostname: None,
            runtime_host: Some(plan.public_base_url),
            runtime_status: RuntimeSummaryStatus::Online,
            active_inference_profile: options
                .finite_private
                .as_ref()
                .map(|_| FINITE_PRIVATE_PROFILE_ID.to_string()),
            hermes_available: Some(true),
            published_app_urls: vec![plan.contact_url],
        })
    }

    fn cleanup_failed_launch(&mut self, facts: &RuntimeLaunchFacts) -> Result<(), RunnerError> {
        self.run_command(
            PlannedCommand {
                program: self.config.docker_bin.clone(),
                cwd: None,
                args: vec![
                    OsString::from("rm"),
                    OsString::from("-f"),
                    OsString::from(&facts.source_machine_id),
                ],
                env: Vec::new(),
            },
            self.config.command_timeout,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerLaunchPlan {
    pub container_name: String,
    pub state_root: PathBuf,
    pub public_base_url: String,
    pub health_url: String,
    pub contact_url: String,
    pub host_port: u16,
    pub container_port: u16,
}

fn docker_launch_plan(config: &DockerConfig, lease: &AgentCreationLease) -> DockerLaunchPlan {
    docker_launch_plan_for_source_machine(
        config,
        &source_machine_name_for_request_id(&lease.request.id),
        &lease.project.display_name,
        &lease.project.id,
    )
}

fn docker_launch_plan_for_source_machine(
    config: &DockerConfig,
    source_machine_id: &str,
    _display_name: &str,
    _project_id: &str,
) -> DockerLaunchPlan {
    let container_name = sanitize_sandbox_name(source_machine_id);
    let public_base_url = config.public_base_url();
    DockerLaunchPlan {
        state_root: config.work_root.join("docker").join(&container_name),
        health_url: format!("{public_base_url}/healthz"),
        contact_url: format!("{public_base_url}/contact"),
        public_base_url,
        host_port: config.host_port,
        container_port: config.container_port,
        container_name,
    }
}

fn docker_run_command(
    config: &DockerConfig,
    plan: &DockerLaunchPlan,
    lease: &AgentCreationLease,
    options: &RuntimeLaunchOptions,
) -> PlannedCommand {
    let mut args = vec![
        OsString::from("run"),
        OsString::from("-d"),
        OsString::from("--name"),
        OsString::from(&plan.container_name),
        OsString::from("--restart"),
        OsString::from("unless-stopped"),
        OsString::from("-p"),
        OsString::from(format!(
            "127.0.0.1:{}:{}",
            plan.host_port, plan.container_port
        )),
        OsString::from("-v"),
        OsString::from(format!("{}:/data", plan.state_root.display())),
        OsString::from("--label"),
        OsString::from("computer.finite.v2.runtime=true"),
        OsString::from("--label"),
        OsString::from(format!(
            "computer.finite.v2.source_host_id={}",
            config.source_host_id
        )),
        OsString::from("--label"),
        OsString::from(format!(
            "computer.finite.v2.source_machine_id={}",
            plan.container_name
        )),
        OsString::from("--label"),
        OsString::from(format!(
            "computer.finite.v2.project_id={}",
            lease.project.id
        )),
    ];
    if let Some(policy) = config
        .pull_policy
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(OsString::from("--pull"));
        args.push(OsString::from(policy));
    }
    let env = docker_runtime_env(config, plan, lease, options)
        .into_iter()
        .map(|(key, value)| {
            args.push(OsString::from("--env"));
            args.push(OsString::from(&key));
            (OsString::from(key), OsString::from(value))
        })
        .collect();
    args.push(OsString::from(config.image.trim()));

    PlannedCommand {
        program: config.docker_bin.clone(),
        cwd: None,
        args,
        env,
    }
}

fn docker_runtime_env(
    config: &DockerConfig,
    plan: &DockerLaunchPlan,
    lease: &AgentCreationLease,
    options: &RuntimeLaunchOptions,
) -> Vec<(String, String)> {
    docker_equivalent_runtime_env(
        DockerEquivalentRuntimeEnv {
            finitechat_server_url: &config.finitechat_server_url,
            agent_picture_url: &config.agent_picture_url,
            agent_http_port: plan.container_port,
            agent_device_id: "agent",
            agent_home: "/data/agent",
            hermes_home: "/data/agent/hermes-home",
            workspace: "/data/workspace",
        },
        lease,
        options,
    )
}

struct DockerEquivalentRuntimeEnv<'a> {
    finitechat_server_url: &'a str,
    agent_picture_url: &'a str,
    agent_http_port: u16,
    agent_device_id: &'a str,
    agent_home: &'a str,
    hermes_home: &'a str,
    workspace: &'a str,
}

fn docker_equivalent_runtime_env(
    env: DockerEquivalentRuntimeEnv<'_>,
    lease: &AgentCreationLease,
    options: &RuntimeLaunchOptions,
) -> Vec<(String, String)> {
    let agent_id = stable_project_agent_id(&lease.project.id);
    let agent_name = lease.project.display_name.clone();
    let mut entries = vec![
        (
            "FINITE_SERVER_URL".to_string(),
            env.finitechat_server_url.to_string(),
        ),
        (
            "FINITECHAT_SERVER_URL".to_string(),
            env.finitechat_server_url.to_string(),
        ),
        ("FINITECHAT_HOME".to_string(), env.agent_home.to_string()),
        // Shared Finite identity contract: identity.json lives on the durable
        // mount so every Finite tool in the runtime finds the same key.
        ("FINITE_HOME".to_string(), env.agent_home.to_string()),
        ("HERMES_HOME".to_string(), env.hermes_home.to_string()),
        (
            "FINITECHAT_WORKSPACE".to_string(),
            env.workspace.to_string(),
        ),
        ("FINITE_AGENT_HTTP_HOST".to_string(), "0.0.0.0".to_string()),
        (
            "FINITE_AGENT_HTTP_PORT".to_string(),
            env.agent_http_port.to_string(),
        ),
        (
            "FINITECHAT_HERMES_AGENT_DEVICE_ID".to_string(),
            env.agent_device_id.to_string(),
        ),
        ("FINITE_AGENT_ID".to_string(), agent_id),
        ("FINITE_AGENT_NAME".to_string(), agent_name.clone()),
        (
            "FINITECHAT_HERMES_AGENT_NAME".to_string(),
            agent_name.clone(),
        ),
        ("FINITECHAT_HERMES_ROOM_NAME".to_string(), agent_name),
        (
            "FINITECHAT_HERMES_AGENT_PICTURE_URL".to_string(),
            options
                .profile_picture_url
                .as_deref()
                .unwrap_or(env.agent_picture_url)
                .to_string(),
        ),
        (
            "FINITECHAT_HERMES_INBOUND_STREAM".to_string(),
            "1".to_string(),
        ),
        ("FINITECHAT_ALLOW_ALL_USERS".to_string(), "true".to_string()),
        ("FINITE_ALLOW_ALL_USERS".to_string(), "true".to_string()),
        ("GATEWAY_ALLOW_ALL_USERS".to_string(), "true".to_string()),
    ];

    if let Some(finite_private) = options.finite_private.as_ref() {
        entries.extend([
            (
                "FINITE_DEFAULT_INFERENCE_PROFILE".to_string(),
                FINITE_PRIVATE_PROFILE_ID.to_string(),
            ),
            (
                "FINITE_PRIVATE_MODEL".to_string(),
                finite_private.model.clone(),
            ),
            (
                "FINITE_PRIVATE_BASE_URL".to_string(),
                finite_private.base_url.clone(),
            ),
            (
                "FINITE_PRIVATE_API_KEY".to_string(),
                finite_private.raw_api_key.clone(),
            ),
            (
                "FINITECHAT_HERMES_MODEL".to_string(),
                finite_private.model.clone(),
            ),
            (
                "FINITECHAT_HERMES_PROVIDER".to_string(),
                "custom".to_string(),
            ),
            (
                "FINITECHAT_HERMES_BASE_URL".to_string(),
                finite_private.base_url.clone(),
            ),
            (
                "FINITECHAT_HERMES_API_MODE".to_string(),
                "chat_completions".to_string(),
            ),
            (
                "OPENAI_API_KEY".to_string(),
                finite_private.raw_api_key.clone(),
            ),
        ]);
        if let Some(specialization_bundle) = finite_private.specialization_bundle.as_ref() {
            entries.extend([
                (
                    FINITE_SPECIALIZATION_BUNDLE_ENV.to_string(),
                    specialization_bundle.bundle_id.clone(),
                ),
                (
                    FINITE_SPECIALIZATION_WORKER_API_KEY_ENV.to_string(),
                    specialization_bundle.worker_api_key.clone(),
                ),
            ]);
        }
    }

    entries.extend(
        options
            .environment
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    entries.extend(
        options
            .secret_environment
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );

    entries
}

fn wait_for_http_json_ready(
    url: &str,
    name: &str,
    timeout: Duration,
    interval: Duration,
) -> Result<(), RunnerError> {
    let started = Instant::now();
    loop {
        let last_error = match ureq::get(url)
            .timeout(interval.max(Duration::from_millis(250)))
            .call()
        {
            Ok(response) => match response.into_json::<serde_json::Value>() {
                Ok(value) => {
                    if value
                        .get("ready")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        return Ok(());
                    }
                    value
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("ready=false")
                        .to_string()
                }
                Err(error) => format!("invalid JSON: {error}"),
            },
            Err(ureq::Error::Status(status, response)) => {
                let body = response
                    .into_string()
                    .unwrap_or_else(|_| "<unreadable response body>".to_string());
                format!("HTTP {status}: {body}")
            }
            Err(error) => error.to_string(),
        };
        if started.elapsed() >= timeout {
            return Err(RunnerError::RuntimeLaunch(format!(
                "{name} did not become ready within {}s: {last_error}",
                timeout.as_secs()
            )));
        }
        thread::sleep(interval);
    }
}

fn active_docker_container_count(config: &DockerConfig) -> Option<u32> {
    let output = Command::new(&config.docker_bin)
        .args([
            "ps",
            "--filter",
            "label=computer.finite.v2.runtime=true",
            "--filter",
            &format!(
                "label=computer.finite.v2.source_host_id={}",
                config.source_host_id
            ),
            "--format",
            "{{.ID}}",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count() as u32,
    )
}

#[derive(Debug, Clone)]
pub struct EnclaviaConfig {
    pub enclavia_bin: PathBuf,
    pub docker_bin: PathBuf,
    pub source_host_id: String,
    pub image: String,
    pub runtime_artifact_id: Option<String>,
    pub runtime_artifact_kind: Option<RuntimeArtifactKind>,
    pub runtime_state_schema_version: Option<String>,
    pub finitechat_server_url: String,
    pub agent_picture_url: String,
    pub enclave_id: String,
    pub pull_policy: Option<String>,
    pub max_enclave_count: Option<u32>,
    pub drain_new_leases: bool,
    pub available_memory_bytes: Option<u64>,
    pub command_timeout: Duration,
    pub launch_timeout: Duration,
    pub readiness_timeout: Duration,
    pub readiness_interval: Duration,
}

impl EnclaviaConfig {
    pub fn validate(&self) -> Result<(), RunnerError> {
        if self.enclavia_bin.as_os_str().is_empty() {
            return Err(RunnerError::MissingEnclaviaBinary);
        }
        if self.docker_bin.as_os_str().is_empty() {
            return Err(RunnerError::MissingDockerBinary);
        }
        if self.source_host_id.trim().is_empty() {
            return Err(RunnerError::MissingSourceHostId);
        }
        if self.image.trim().is_empty() {
            return Err(RunnerError::MissingRuntimeArtifactReference);
        }
        if self.finitechat_server_url.trim().is_empty() {
            return Err(RunnerError::MissingFinitechatServerUrl);
        }
        if self.enclave_id.trim().is_empty() {
            return Err(RunnerError::MissingEnclaviaEnclaveId);
        }
        if let Some(kind) = self.runtime_artifact_kind
            && kind != RuntimeArtifactKind::OciImage
        {
            return Err(RunnerError::RuntimeLaunch(format!(
                "Enclavia launcher requires an OCI image artifact, got {}",
                kind.as_str()
            )));
        }
        if let Some(policy) = self.pull_policy.as_deref() {
            match policy.trim() {
                "" | "always" | "missing" | "never" => {}
                other => {
                    return Err(RunnerError::RuntimeLaunch(format!(
                        "invalid Enclavia pull policy {other:?}; use always, missing, or never"
                    )));
                }
            }
        }
        Ok(())
    }
}

impl Default for EnclaviaConfig {
    fn default() -> Self {
        Self {
            enclavia_bin: PathBuf::from("enclavia"),
            docker_bin: PathBuf::from("docker"),
            source_host_id: String::new(),
            image: String::new(),
            runtime_artifact_id: None,
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: None,
            finitechat_server_url: DEFAULT_FINITECHAT_SERVER_URL.to_string(),
            agent_picture_url: DEFAULT_FINITE_AGENT_PICTURE_URL.to_string(),
            enclave_id: String::new(),
            pull_policy: Some("missing".to_string()),
            max_enclave_count: Some(1),
            drain_new_leases: false,
            available_memory_bytes: None,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            launch_timeout: Duration::from_secs(900),
            readiness_timeout: DEFAULT_RUNTIME_READY_TIMEOUT,
            readiness_interval: DEFAULT_RUNTIME_READY_INTERVAL,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnclaviaLauncher {
    config: EnclaviaConfig,
}

impl EnclaviaLauncher {
    pub fn new(config: EnclaviaConfig) -> Self {
        Self { config }
    }

    pub fn plan_launch(&self, _lease: &AgentCreationLease) -> EnclaviaLaunchPlan {
        enclavia_launch_plan(&self.config)
    }

    fn run_enclavia_capture(
        &self,
        args: Vec<OsString>,
        stdin: Option<&str>,
        timeout: Duration,
    ) -> Result<String, RunnerError> {
        let mut command = Command::new(&self.config.enclavia_bin);
        command
            .args(&args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| RunnerError::CommandExecution {
                program: self.config.enclavia_bin.display().to_string(),
                message: error.to_string(),
            })?;

        if let Some(input) = stdin
            && let Some(mut child_stdin) = child.stdin.take()
            && let Err(error) = child_stdin.write_all(input.as_bytes())
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RunnerError::CommandExecution {
                program: self.config.enclavia_bin.display().to_string(),
                message: format!("failed to write command stdin: {error}"),
            });
        }

        let output = wait_with_captured_output(child, &self.config.enclavia_bin, timeout)?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if output.status.success() {
            return Ok(stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(RunnerError::CommandExecution {
            program: self.config.enclavia_bin.display().to_string(),
            message: format!(
                "exit status {} stdout={stdout} stderr={stderr}",
                output.status
            ),
        })
    }

    fn run_enclavia_json(
        &self,
        args: Vec<OsString>,
        stdin: Option<&str>,
        timeout: Duration,
    ) -> Result<serde_json::Value, RunnerError> {
        let stdout = self.run_enclavia_capture(args, stdin, timeout)?;
        serde_json::from_str(&stdout).map_err(|error| {
            RunnerError::RuntimeLaunch(format!("invalid Enclavia JSON: {error}: {stdout}"))
        })
    }

    fn run_docker_status(&self, args: Vec<OsString>, timeout: Duration) -> Result<(), RunnerError> {
        let mut child = Command::new(&self.config.docker_bin)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| RunnerError::CommandExecution {
                program: self.config.docker_bin.display().to_string(),
                message: error.to_string(),
            })?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        return Ok(());
                    }
                    return Err(RunnerError::CommandExecution {
                        program: self.config.docker_bin.display().to_string(),
                        message: format!("exit status {status}"),
                    });
                }
                Ok(None) => {
                    if started.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(RunnerError::CommandTimedOut {
                            program: self.config.docker_bin.display().to_string(),
                            timeout_secs: timeout.as_secs(),
                        });
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(error) => {
                    return Err(RunnerError::CommandExecution {
                        program: self.config.docker_bin.display().to_string(),
                        message: error.to_string(),
                    });
                }
            }
        }
    }

    fn ensure_local_image(&self) -> Result<(), RunnerError> {
        let image = self.config.image.trim();
        match self
            .config
            .pull_policy
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
        {
            "" | "missing" => {
                let present = self
                    .run_docker_status(
                        vec![
                            OsString::from("image"),
                            OsString::from("inspect"),
                            OsString::from(image),
                        ],
                        self.config.command_timeout,
                    )
                    .is_ok();
                if !present {
                    self.run_docker_status(
                        vec![
                            OsString::from("pull"),
                            OsString::from("-q"),
                            OsString::from(image),
                        ],
                        self.config.launch_timeout,
                    )?;
                }
                Ok(())
            }
            "always" => self.run_docker_status(
                vec![
                    OsString::from("pull"),
                    OsString::from("-q"),
                    OsString::from(image),
                ],
                self.config.launch_timeout,
            ),
            "never" => Ok(()),
            other => Err(RunnerError::RuntimeLaunch(format!(
                "invalid Enclavia pull policy {other:?}; use always, missing, or never"
            ))),
        }
    }

    fn set_runtime_secrets(
        &self,
        plan: &EnclaviaLaunchPlan,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<(), RunnerError> {
        if options.finite_private.is_none() {
            return Err(RunnerError::RuntimeLaunch(
                "Enclavia runtime launch requires a Finite Private key".to_string(),
            ));
        }

        let env = enclavia_runtime_env(&self.config, plan, lease, options);
        let mut positional = Vec::new();
        let mut sensitive = Vec::new();
        for (key, value) in env {
            if enclavia_env_value_uses_stdin(&key) {
                sensitive.push((key, value));
            } else {
                positional.push((key, value));
            }
        }

        if !positional.is_empty() {
            let mut args = vec![
                OsString::from("--json"),
                OsString::from("secret"),
                OsString::from("set"),
                OsString::from(&plan.enclave_id),
            ];
            for (key, value) in positional {
                args.push(OsString::from(format!("{key}={value}")));
            }
            let _ = self.run_enclavia_json(args, None, self.config.command_timeout)?;
        }

        for (key, value) in sensitive {
            let _ = self.run_enclavia_json(
                vec![
                    OsString::from("--json"),
                    OsString::from("secret"),
                    OsString::from("set"),
                    OsString::from(&plan.enclave_id),
                    OsString::from("--from-stdin"),
                    OsString::from("--name"),
                    OsString::from(key),
                ],
                Some(&value),
                self.config.command_timeout,
            )?;
        }

        Ok(())
    }

    fn push_image(&self, plan: &EnclaviaLaunchPlan) -> Result<serde_json::Value, RunnerError> {
        self.run_enclavia_json(
            vec![
                OsString::from("--json"),
                OsString::from("push"),
                OsString::from(self.config.image.trim()),
                OsString::from(&plan.enclave_id),
            ],
            None,
            self.config.launch_timeout,
        )
    }

    fn status(&self, enclave_id: &str) -> Result<serde_json::Value, RunnerError> {
        self.run_enclavia_json(
            vec![
                OsString::from("--json"),
                OsString::from("enclave"),
                OsString::from("status"),
                OsString::from(enclave_id),
            ],
            None,
            self.config.command_timeout,
        )
    }

    fn wait_for_running(&self, enclave_id: &str) -> Result<serde_json::Value, RunnerError> {
        let started = Instant::now();
        loop {
            let last_status = match self.status(enclave_id) {
                Ok(status) => {
                    let status_name = status
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    match status_name.as_str() {
                        "running" => return Ok(status),
                        "error" => {
                            let message = status
                                .get("error_message")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("no error_message returned");
                            return Err(RunnerError::RuntimeLaunch(format!(
                                "Enclavia enclave {enclave_id} entered error: {message}"
                            )));
                        }
                        _ => {}
                    }
                    status_name
                }
                Err(error) => error.to_string(),
            };
            if started.elapsed() >= self.config.launch_timeout {
                return Err(RunnerError::RuntimeLaunch(format!(
                    "Enclavia enclave {enclave_id} did not reach running within {}s: {last_status}",
                    self.config.launch_timeout.as_secs()
                )));
            }
            thread::sleep(self.config.readiness_interval);
        }
    }

    fn wait_for_runtime_http(&self, endpoint: &EnclaviaEndpoint) -> Result<(), RunnerError> {
        wait_for_http_json_ready(
            &endpoint.health_url,
            "Enclavia runtime /proxy/healthz",
            self.config.readiness_timeout,
            self.config.readiness_interval,
        )
    }

    fn run_lifecycle(&self, command: &str, source_machine_id: &str) -> Result<(), RunnerError> {
        let _ = self.run_enclavia_json(
            vec![
                OsString::from("--json"),
                OsString::from("enclave"),
                OsString::from(command),
                OsString::from(source_machine_id),
            ],
            None,
            self.config.command_timeout,
        )?;
        Ok(())
    }
}

impl RuntimeLauncher for EnclaviaLauncher {
    fn runtime_capabilities(&self) -> RuntimeCapabilitiesEnvelope {
        state_preserving_runtime_capabilities(false)
    }

    fn runner_class(&self) -> RunnerClass {
        RunnerClass::Enclavia
    }

    fn validate_ready(&self) -> Result<(), RunnerError> {
        self.config.validate()
    }

    fn uses_core_runtime_heartbeat(&self) -> bool {
        false
    }

    fn runner_capacity(&self) -> RunnerLeaseCapacity {
        RunnerLeaseCapacity {
            runner_classes: vec![self.runner_class()],
            draining: self.config.drain_new_leases,
            max_sandbox_count: self.config.max_enclave_count,
            active_sandbox_count: active_enclavia_enclave_count(&self.config),
            available_memory_bytes: self.config.available_memory_bytes,
            runtime_capabilities: Some(self.runtime_capabilities()),
        }
    }

    fn source_host_id(&self) -> Option<&str> {
        Some(&self.config.source_host_id)
    }

    fn planned_source(&self, lease: &AgentCreationLease) -> Option<RuntimeSourceIdentity> {
        let plan = self.plan_launch(lease);
        Some(RuntimeSourceIdentity {
            source_host_id: self.config.source_host_id.clone(),
            source_machine_id: plan.enclave_id,
        })
    }

    fn restart_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        _options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        self.validate_ready()?;
        ensure_runtime_belongs_to_source(
            &lease.runtime.source_host_id,
            &self.config.source_host_id,
        )?;
        ensure_runtime_control_source_matches(
            &lease.runtime.source_machine_id,
            &lease.request.source_machine_id,
            "restart",
        )?;
        self.run_lifecycle("restart", &lease.request.source_machine_id)?;
        let status = self.wait_for_running(&lease.request.source_machine_id)?;
        let endpoint = enclavia_endpoint_from_status(&status, &lease.request.source_machine_id);
        self.wait_for_runtime_http(&endpoint)
    }

    fn recover_known_good_chat_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        self.restart_runtime(lease, options)
    }

    fn stop_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        self.validate_ready()?;
        ensure_runtime_belongs_to_source(
            &lease.runtime.source_host_id,
            &self.config.source_host_id,
        )?;
        ensure_runtime_control_source_matches(
            &lease.runtime.source_machine_id,
            &lease.request.source_machine_id,
            "stop",
        )?;
        self.run_lifecycle("stop", &lease.request.source_machine_id)
    }

    fn destroy_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        self.validate_ready()?;
        ensure_runtime_belongs_to_source(
            &lease.runtime.source_host_id,
            &self.config.source_host_id,
        )?;
        ensure_runtime_control_source_matches(
            &lease.runtime.source_machine_id,
            &lease.request.source_machine_id,
            "destroy",
        )?;
        self.run_lifecycle("destroy", &lease.request.source_machine_id)
    }

    fn launch(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        self.validate_ready()?;
        let plan = self.plan_launch(lease);
        self.ensure_local_image()?;
        self.set_runtime_secrets(&plan, lease, options)?;
        let push = self.push_image(&plan)?;
        if push
            .get("staged")
            .and_then(serde_json::Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false)
        {
            return Err(RunnerError::RuntimeLaunch(
                "Enclavia push staged an upgrade instead of launching; upgrade confirmation is not supported by this runner yet".to_string(),
            ));
        }

        let status = self.wait_for_running(&plan.enclave_id)?;
        let endpoint = enclavia_endpoint_from_status(&status, &plan.enclave_id);
        self.wait_for_runtime_http(&endpoint)?;

        let runtime_bootstrap_token = random_runtime_bootstrap_token();
        let runtime_relay_token_hash = hash_runtime_relay_token(&runtime_bootstrap_token)
            .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;

        Ok(RuntimeLaunchFacts {
            source_host_id: self.config.source_host_id.clone(),
            source_machine_id: endpoint.enclave_id.clone(),
            runtime_artifact_id: self.config.runtime_artifact_id.clone(),
            state_schema_version: self.config.runtime_state_schema_version.clone(),
            provider_runtime_handle: None,
            contact_endpoint: Some(endpoint.contact_url.clone()),
            runtime_relay_token_hash,
            display_name: Some(lease.project.display_name.clone()),
            hostname: Some(endpoint.hostname.clone()),
            runtime_host: Some(endpoint.public_base_url.clone()),
            runtime_status: RuntimeSummaryStatus::Online,
            active_inference_profile: options
                .finite_private
                .as_ref()
                .map(|_| FINITE_PRIVATE_PROFILE_ID.to_string()),
            hermes_available: Some(true),
            published_app_urls: vec![endpoint.contact_url],
        })
    }

    fn cleanup_failed_launch(&mut self, _facts: &RuntimeLaunchFacts) -> Result<(), RunnerError> {
        // The Enclavia lane is a pre-created single-enclave test target. Keep a
        // failed-but-booted enclave around for logs and dashboard inspection.
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnclaviaLaunchPlan {
    pub enclave_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EnclaviaEndpoint {
    enclave_id: String,
    hostname: String,
    public_base_url: String,
    health_url: String,
    contact_url: String,
}

fn enclavia_launch_plan(config: &EnclaviaConfig) -> EnclaviaLaunchPlan {
    EnclaviaLaunchPlan {
        enclave_id: config.enclave_id.trim().to_string(),
    }
}

fn enclavia_runtime_env(
    config: &EnclaviaConfig,
    plan: &EnclaviaLaunchPlan,
    lease: &AgentCreationLease,
    options: &RuntimeLaunchOptions,
) -> Vec<(String, String)> {
    docker_equivalent_runtime_env(
        DockerEquivalentRuntimeEnv {
            finitechat_server_url: &config.finitechat_server_url,
            agent_picture_url: &config.agent_picture_url,
            agent_http_port: DEFAULT_DOCKER_CONTAINER_PORT,
            agent_device_id: &plan.enclave_id,
            agent_home: "/data/agent",
            hermes_home: "/data/agent/hermes-home",
            workspace: "/data/workspace",
        },
        lease,
        options,
    )
}

fn enclavia_env_value_uses_stdin(key: &str) -> bool {
    matches!(key, "FINITE_PRIVATE_API_KEY" | "OPENAI_API_KEY")
        || key.ends_with("_KEY")
        || key.ends_with("_SECRET")
        || key.ends_with("_TOKEN")
        || key.ends_with("_PASSWORD")
        || key.ends_with("_CREDENTIAL")
}

fn enclavia_endpoint_from_status(
    status: &serde_json::Value,
    fallback_enclave_id: &str,
) -> EnclaviaEndpoint {
    let enclave_id = status
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_enclave_id)
        .to_string();
    enclavia_endpoint(&enclave_id)
}

fn enclavia_endpoint(enclave_id: &str) -> EnclaviaEndpoint {
    let enclave_id = enclave_id.trim().to_string();
    let hostname = format!("{enclave_id}.enclaves.beta.enclavia.io");
    let public_base_url = format!("https://{hostname}/proxy");
    EnclaviaEndpoint {
        enclave_id,
        hostname,
        health_url: format!("{public_base_url}/healthz"),
        contact_url: format!("{public_base_url}/contact"),
        public_base_url,
    }
}

fn active_enclavia_enclave_count(config: &EnclaviaConfig) -> Option<u32> {
    let output = Command::new(&config.enclavia_bin)
        .args(["--json", "enclave", "status", config.enclave_id.trim()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    match status
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
    {
        "waiting_for_image" => Some(0),
        _ => Some(1),
    }
}

fn ensure_runtime_belongs_to_source(
    runtime_source_host_id: &str,
    expected_source_host_id: &str,
) -> Result<(), RunnerError> {
    if runtime_source_host_id != expected_source_host_id {
        return Err(RunnerError::RuntimeLaunch(format!(
            "runtime belongs to source host {runtime_source_host_id}, not {expected_source_host_id}"
        )));
    }
    Ok(())
}

fn ensure_runtime_control_source_matches(
    runtime_source_machine_id: &str,
    request_source_machine_id: &str,
    operation: &str,
) -> Result<(), RunnerError> {
    if runtime_source_machine_id != request_source_machine_id {
        return Err(RunnerError::RuntimeLaunch(format!(
            "runtime source machine {runtime_source_machine_id} did not match {operation} request {request_source_machine_id}"
        )));
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
pub struct PlannedCommand {
    pub program: PathBuf,
    pub cwd: Option<PathBuf>,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
}

impl std::fmt::Debug for PlannedCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut redacted_args = Vec::with_capacity(self.args.len());
        let mut redact_next = false;
        for arg in &self.args {
            if redact_next {
                redacted_args.push(OsString::from("<redacted>"));
                redact_next = false;
                continue;
            }
            redacted_args.push(arg.clone());
            if arg.as_os_str() == std::ffi::OsStr::new("--secret") {
                redact_next = true;
            }
        }
        formatter
            .debug_struct("PlannedCommand")
            .field("program", &self.program)
            .field("cwd", &self.cwd)
            .field("args", &redacted_args)
            .field(
                "env_keys",
                &self.env.iter().map(|(key, _)| key).collect::<Vec<_>>(),
            )
            .finish()
    }
}

fn write_secret_file(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;

    #[cfg(unix)]
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub(crate) fn random_runtime_bootstrap_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn source_machine_name_for_request_id(request_id: &str) -> String {
    let suffix = request_id
        .strip_prefix("agent_request_")
        .unwrap_or(request_id);
    sanitize_sandbox_name(&format!("finite-agent_{suffix}"))
}

fn stable_project_agent_id(project_id: &str) -> String {
    format!("agent_{}", sanitize_sandbox_name(project_id))
}

fn sanitize_sandbox_name(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            result.push(ch);
        } else {
            result.push('-');
        }
    }
    if result.len() > 63 {
        result.truncate(63);
    }
    result.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use finite_saas_core::{
        AgentCreationRequest, AgentCreationRequestStatus, AgentRuntime, HostOwnedRuntimeFacts,
        Project, RuntimeControlRequestStatus,
    };
    use std::collections::VecDeque;

    fn specialization_bundle_defaults() -> SpecializationBundleRuntimeDefaults {
        SpecializationBundleRuntimeDefaults {
            bundle_id: DEFAULT_FINITE_PRIVATE_SPECIALIZATION_BUNDLE.to_owned(),
            worker_api_key: "specialization-worker-secret".to_owned(),
        }
    }

    fn finite_private_defaults() -> FinitePrivateRuntimeDefaults {
        FinitePrivateRuntimeDefaults {
            specialization_bundle: Some(specialization_bundle_defaults()),
            ..FinitePrivateRuntimeDefaults::default()
        }
    }

    #[test]
    fn specialization_bundle_is_scoped_only_to_the_glm_finite_private_profile() {
        let configured_glm_profile = finite_private_defaults();
        let glm_bundle = specialization_bundle_for_finite_private_profile(&configured_glm_profile)
            .unwrap()
            .expect("the canonical Finite Private GLM profile should activate AEON");
        assert_eq!(
            glm_bundle.bundle_id,
            DEFAULT_FINITE_PRIVATE_SPECIALIZATION_BUNDLE
        );

        let other_finite_private_profile = FinitePrivateRuntimeDefaults {
            model: "another-finite-private-model".to_owned(),
            specialization_bundle: Some(SpecializationBundleRuntimeDefaults {
                bundle_id: "not-validated-for-this-profile".to_owned(),
                worker_api_key: "unused-for-non-glm".to_owned(),
            }),
            ..FinitePrivateRuntimeDefaults::default()
        };
        assert!(
            specialization_bundle_for_finite_private_profile(&other_finite_private_profile)
                .unwrap()
                .is_none(),
            "specialization admission must depend on the Finite Private GLM profile, not a runner host or user"
        );
    }

    #[test]
    fn captured_provider_output_is_drained_before_the_child_exits() {
        let child = Command::new("sh")
            .arg("-c")
            .arg(
                "awk 'BEGIN { for (i = 0; i < 50000; i++) print \"stdout\" }'; \
                 awk 'BEGIN { for (i = 0; i < 50000; i++) print \"stderr\" }' >&2",
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let output =
            wait_with_captured_output(child, Path::new("sh"), Duration::from_secs(5)).unwrap();

        assert!(output.status.success());
        assert!(output.stdout.len() > 64 * 1024);
        assert!(output.stderr.len() > 64 * 1024);
    }

    #[test]
    fn runner_id_is_required() {
        let error = AgentCreationRunner::new(
            FakeQueue::idle(),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            " ",
            300,
        )
        .unwrap_err();
        assert!(matches!(error, RunnerError::MissingRunnerId));
    }

    #[test]
    fn adapters_advertise_only_proven_runtime_controls() {
        let state_preserving = state_preserving_runtime_capabilities(false);

        assert_eq!(
            DockerLauncher::new(DockerConfig::default()).runtime_capabilities(),
            state_preserving
        );
        assert_eq!(
            AppleContainerLauncher::new(AppleContainerConfig::default()).runtime_capabilities(),
            state_preserving
        );
        assert_eq!(
            EnclaviaLauncher::new(EnclaviaConfig::default()).runtime_capabilities(),
            state_preserving
        );
        assert_eq!(
            PhalaLauncher::new(PhalaConfig::default()).runtime_capabilities(),
            state_preserving
        );
        assert_eq!(
            KataLauncher::new(KataConfig::default()).runtime_capabilities(),
            kata_runtime_capabilities()
        );
    }

    #[test]
    fn run_once_does_not_lease_when_launcher_is_not_ready() {
        let mut runner = AgentCreationRunner::new(
            FakeQueue::idle(),
            FakeLauncher::not_ready("missing template"),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let error = runner.run_once().unwrap_err();
        assert!(error.to_string().contains("missing template"));
        assert_eq!(runner.queue.leases.len(), 0);
    }

    #[test]
    fn run_once_returns_idle_without_launching_when_core_has_no_work() {
        let mut runner = AgentCreationRunner::new(
            FakeQueue::idle(),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();
        assert_eq!(outcome, RunOnceOutcome::Idle);
        assert_eq!(
            runner.queue.leases,
            vec![("runner-1".to_string(), "lease-1".to_string(), 300)]
        );
        assert_eq!(runner.launcher.launch_count, 0);
    }

    #[test]
    fn run_once_with_no_advertised_classes_claims_no_work() {
        let capacity = RunnerLeaseCapacity {
            runtime_capabilities: Some(state_preserving_runtime_capabilities(false)),
            ..RunnerLeaseCapacity::default()
        };
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(sample_lease("agent_request_123")),
            FakeLauncher::ready(RuntimeLaunchFacts::sample())
                .with_runner_capacity(capacity.clone()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::CapacityUnavailable {
                reason: "runner advertises no classes".to_string(),
                runner_capacity: capacity,
            }
        );
        assert!(runner.queue.runtime_control_leases.is_empty());
        assert!(runner.queue.leases.is_empty());
        assert_eq!(runner.launcher.launch_count, 0);
    }

    #[test]
    fn run_once_reports_capacity_without_agent_lease_when_runner_is_draining() {
        let capacity = RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::LocalDocker],
            draining: true,
            max_sandbox_count: Some(4),
            active_sandbox_count: Some(2),
            available_memory_bytes: Some(8 * 1024 * 1024 * 1024),
            runtime_capabilities: Some(state_preserving_runtime_capabilities(false)),
        };
        let mut runner = AgentCreationRunner::new(
            FakeQueue::idle(),
            FakeLauncher::ready(RuntimeLaunchFacts::sample())
                .with_runner_capacity(capacity.clone()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::CapacityUnavailable {
                reason: "runner is draining".to_string(),
                runner_capacity: capacity.clone(),
            }
        );
        assert_eq!(
            runner.queue.runtime_control_capacities,
            vec![Some(capacity)]
        );
        assert!(runner.queue.leases.is_empty());
        assert_eq!(runner.launcher.launch_count, 0);
    }

    #[test]
    fn run_once_reports_capacity_without_agent_lease_when_sandbox_limit_is_full() {
        let capacity = RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::LocalDocker],
            draining: false,
            max_sandbox_count: Some(2),
            active_sandbox_count: Some(2),
            available_memory_bytes: Some(1024 * 1024 * 1024),
            runtime_capabilities: Some(state_preserving_runtime_capabilities(false)),
        };
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(sample_lease("agent_request_123")),
            FakeLauncher::ready(RuntimeLaunchFacts::sample())
                .with_runner_capacity(capacity.clone()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::CapacityUnavailable {
                reason: "runner sandbox capacity is full".to_string(),
                runner_capacity: capacity,
            }
        );
        assert!(runner.queue.leases.is_empty());
        assert_eq!(runner.launcher.launch_count, 0);
    }

    #[test]
    fn run_once_restarts_runtime_control_request_before_launch_work() {
        let runtime_control = sample_runtime_control_lease("runtime_ctl_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control.clone()).with_next_heartbeat(
                RelayHeartbeat {
                    last_seen_at: "2026-05-25T13:00:10Z".to_string(),
                    ..sample_heartbeat("oslo-agent-001")
                },
            ),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_runtime_environment(BTreeMap::from([(
            "FINITE_SITES_API".to_string(),
            "http://192.168.64.1:18789".to_string(),
        )]))
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::RuntimeRestarted {
                request_id: runtime_control.request.id.clone(),
                runtime_id: runtime_control.runtime.id.clone(),
            }
        );
        assert_eq!(
            runner.launcher.restarted,
            vec!["oslo-agent-001".to_string()]
        );
        assert_eq!(
            runner.launcher.restart_options[0].environment(),
            &BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "http://192.168.64.1:18789".to_string(),
            )])
        );
        assert!(runner.queue.leases.is_empty());
        assert_eq!(runner.queue.runtime_control_leases.len(), 1);
        assert_eq!(
            runner.queue.runtime_control_leases[0],
            (
                "runner-1".to_string(),
                "lease-1".to_string(),
                300,
                Some("oslo-host-1".to_string())
            )
        );
        assert_eq!(runner.queue.completed_runtime_control.len(), 1);
        assert_eq!(
            runner.queue.heartbeat_checks,
            vec!["oslo-agent-001".to_string(), "oslo-agent-001".to_string()]
        );
        assert!(runner.queue.failed_runtime_control.is_empty());
    }

    #[test]
    fn runtime_restart_uses_the_persisted_spec_environment_not_runner_defaults() {
        let mut runtime_control = sample_runtime_control_lease("runtime_ctl_123");
        let placement = RuntimePlacement {
            runner_class: RunnerClass::LocalDocker,
            runtime_resource_class: RuntimeResourceClass::Vcpu4Memory8Gib,
        };
        runtime_control.runtime.placement = Some(placement);
        runtime_control.runtime_spec = Some(sample_runtime_spec(
            &runtime_control.request.id,
            RunnerClass::LocalDocker,
            BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "https://api.finite.chat".to_string(),
            )]),
            vec!["FINITE_PRIVATE_API_KEY".to_string(), "FAL_KEY".to_string()],
        ));
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control).with_next_heartbeat(
                RelayHeartbeat {
                    last_seen_at: "2026-05-25T13:00:10Z".to_string(),
                    ..sample_heartbeat("oslo-agent-001")
                },
            ),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()).with_runner_capacity(
                RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::LocalDocker],
                    ..RunnerLeaseCapacity::default()
                },
            ),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_runtime_environment(BTreeMap::from([(
            "FINITE_SITES_API".to_string(),
            "https://runner-default.invalid".to_string(),
        )]))
        .unwrap()
        .with_runtime_secret_environment(BTreeMap::from([(
            "FAL_KEY".to_string(),
            "restart-must-not-refresh".to_string(),
        )]))
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::RuntimeRestarted { .. }));
        assert_eq!(
            runner.launcher.restart_options[0].environment(),
            &BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "https://api.finite.chat".to_string(),
            )])
        );
        assert!(
            runner.launcher.restart_options[0]
                .secret_environment()
                .is_empty(),
            "ordinary restart must not refresh Runtime secrets"
        );
    }

    #[test]
    fn run_once_refuses_unadvertised_known_good_recovery_from_an_older_core() {
        let runtime_control = sample_runtime_control_lease_with_kind(
            "runtime_ctl_123",
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
        );
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control.clone()).with_next_heartbeat(
                RelayHeartbeat {
                    last_seen_at: "2026-05-25T13:00:10Z".to_string(),
                    ..sample_heartbeat("oslo-agent-001")
                },
            ),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::RuntimeRecoveryFailed {
                request_id: runtime_control.request.id.clone(),
                failure_message: "runtime control RecoverKnownGoodChatRuntime is not advertised by the runner adapter".to_string(),
            }
        );
        assert!(runner.launcher.restarted.is_empty());
        assert!(runner.launcher.recovered.is_empty());
        assert!(runner.queue.leases.is_empty());
        assert!(runner.queue.completed_runtime_control.is_empty());
        assert_eq!(runner.queue.failed_runtime_control.len(), 1);
    }

    #[test]
    fn run_once_dispatches_core_bound_known_good_recovery_to_kata() {
        let mut runtime_control = sample_runtime_control_lease_with_kind(
            "runtime_ctl_recovery",
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
        );
        let placement = RuntimePlacement {
            runner_class: RunnerClass::Kata,
            runtime_resource_class: RuntimeResourceClass::Vcpu4Memory8Gib,
        };
        runtime_control.runtime.placement = Some(placement);
        let mut runtime_spec = sample_runtime_spec(
            &runtime_control.request.id,
            RunnerClass::Kata,
            BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "https://api.finite.chat".to_string(),
            )]),
            vec!["FINITE_PRIVATE_API_KEY".to_string()],
        );
        let RuntimeSpecEnvelope::V1(spec) = &mut runtime_spec;
        spec.boot_intent = RuntimeBootIntent::RecoverKnownGood;
        runtime_control.runtime_spec = Some(runtime_spec);

        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control.clone()),
            FakeLauncher::ready(RuntimeLaunchFacts::sample())
                .for_kata()
                .without_core_heartbeat(),
            FixedLeaseTokens::new(["lease-recovery"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::RuntimeRecoveredKnownGoodChat {
                request_id: runtime_control.request.id,
                runtime_id: runtime_control.runtime.id,
            }
        );
        assert_eq!(runner.launcher.recovered, vec!["oslo-agent-001"]);
        assert_eq!(
            runner.launcher.restart_options[0].environment(),
            &BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "https://api.finite.chat".to_string(),
            )])
        );
        assert_eq!(runner.queue.completed_runtime_control.len(), 1);
        assert_eq!(
            runner.queue.completed_runtime_control[0].runtime_capabilities, None,
            "recovery must not rewrite persisted capabilities"
        );
        assert!(runner.queue.failed_runtime_control.is_empty());
    }

    #[test]
    fn run_once_upgrades_only_the_core_bound_artifact_and_reports_actual_facts() {
        let mut runtime_control = sample_runtime_upgrade_lease("runtime_ctl_upgrade");
        let placement = RuntimePlacement {
            runner_class: RunnerClass::Kata,
            runtime_resource_class: RuntimeResourceClass::Vcpu4Memory8Gib,
        };
        runtime_control.runtime.placement = Some(placement);
        let mut runtime_spec = sample_runtime_spec(
            &runtime_control.request.id,
            RunnerClass::Kata,
            BTreeMap::new(),
            vec!["FINITE_PRIVATE_API_KEY".to_string(), "FAL_KEY".to_string()],
        );
        let RuntimeSpecEnvelope::V1(spec) = &mut runtime_spec;
        spec.runtime_artifact_id = "artifact-v2".to_string();
        spec.runtime_image_digest = runtime_control
            .target_runtime_artifact
            .as_ref()
            .unwrap()
            .reference
            .clone();
        runtime_control.runtime_spec = Some(runtime_spec);
        runtime_control
            .target_runtime_artifact
            .as_mut()
            .unwrap()
            .recover_known_good_chat = true;
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control.clone()),
            FakeLauncher::ready(RuntimeLaunchFacts::sample())
                .for_kata()
                .without_core_heartbeat(),
            FixedLeaseTokens::new(["lease-upgrade"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_runtime_secret_environment(BTreeMap::from([(
            "FAL_KEY".to_string(),
            "fal-added-on-upgrade".to_string(),
        )]))
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::RuntimeUpgraded {
                request_id: runtime_control.request.id,
                runtime_id: "runtime_123".to_string(),
            }
        );
        assert_eq!(runner.launcher.upgraded, vec!["oslo-agent-001"]);
        assert_eq!(
            runner.launcher.restart_options[0].secret_environment(),
            &BTreeMap::from([("FAL_KEY".to_string(), "fal-added-on-upgrade".to_string(),)])
        );
        assert_eq!(runner.queue.completed_runtime_control.len(), 1);
        let completion = &runner.queue.completed_runtime_control[0];
        assert_eq!(
            completion.runtime_artifact_id.as_deref(),
            Some("artifact-v2")
        );
        assert_eq!(completion.state_schema_version.as_deref(), Some("state-v1"));
        assert_eq!(
            completion.runtime_capabilities,
            Some(kata_runtime_capabilities()),
            "successful Kata upgrade refreshes the exact advertised envelope"
        );
        assert_eq!(
            completion.runtime_host.as_deref(),
            Some("http://127.0.0.1:41002")
        );
        assert_eq!(
            completion.published_app_urls.as_deref(),
            Some(["http://127.0.0.1:41002/contact".to_string()].as_slice())
        );
        assert!(runner.queue.failed_runtime_control.is_empty());
    }

    #[test]
    fn upgrade_capabilities_are_bounded_by_exact_target_artifact() {
        let legacy = sample_runtime_upgrade_lease("runtime_ctl_legacy");
        let bounded = artifact_bounded_upgrade_runtime_capabilities(
            kata_runtime_capabilities(),
            legacy.target_runtime_artifact.as_ref(),
        );
        assert!(!bounded.v1().recover_known_good_chat);

        let mut capable = legacy.target_runtime_artifact.unwrap();
        capable.recover_known_good_chat = true;
        let bounded = artifact_bounded_upgrade_runtime_capabilities(
            kata_runtime_capabilities(),
            Some(&capable),
        );
        assert!(bounded.v1().recover_known_good_chat);
    }

    #[test]
    fn run_once_stops_runtime_control_request_without_waiting_for_heartbeat() {
        let runtime_control =
            sample_runtime_control_lease_with_kind("runtime_ctl_123", RuntimeControlKind::Stop);
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control.clone()),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::RuntimeStopped {
                request_id: runtime_control.request.id.clone(),
                runtime_id: runtime_control.runtime.id.clone(),
            }
        );
        assert_eq!(runner.launcher.stopped, vec!["oslo-agent-001".to_string()]);
        assert!(runner.queue.heartbeat_checks.is_empty());
        assert_eq!(runner.queue.completed_runtime_control.len(), 1);
        assert_eq!(
            runner.queue.completed_runtime_control[0].runtime_capabilities, None,
            "non-upgrade lifecycle operations preserve persisted capabilities"
        );
        assert!(runner.queue.failed_runtime_control.is_empty());
    }

    #[test]
    fn run_once_refuses_unadvertised_retirement_from_an_older_core() {
        let runtime_control =
            sample_runtime_control_lease_with_kind("runtime_ctl_123", RuntimeControlKind::Destroy);
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control.clone()),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::RuntimeDestroyFailed {
                request_id: runtime_control.request.id.clone(),
                failure_message: "runtime control Destroy is not advertised by the runner adapter"
                    .to_string(),
            }
        );
        assert!(runner.launcher.destroyed.is_empty());
        assert!(runner.queue.heartbeat_checks.is_empty());
        assert!(runner.queue.completed_runtime_control.is_empty());
        assert_eq!(runner.queue.failed_runtime_control.len(), 1);
    }

    #[test]
    fn run_once_renews_and_completes_retirement_with_typed_receipt() {
        let runtime_control =
            sample_runtime_control_lease_with_kind("runtime_ctl_123", RuntimeControlKind::Destroy);
        let receipt = RuntimeRetirementSnapshotReceipt {
            schema: finite_saas_core::RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA.to_string(),
            request_id: runtime_control.request.id.clone(),
            project_id: runtime_control.request.project_id.clone(),
            agent_runtime_id: runtime_control.request.agent_runtime_id.clone(),
            durable_state_id: "state-123".to_string(),
            runtime_artifact_id: "artifact-v1".to_string(),
            backend: finite_saas_core::RUNTIME_RETIREMENT_BACKEND_BORG.to_string(),
            locator: finite_saas_core::runtime_retirement_archive_locator(
                &runtime_control.request.id,
            ),
            zip_bytes: 10,
            zip_sha256: "a".repeat(64),
            manifest_sha256: "b".repeat(64),
            created_at: "2026-05-25T13:00:00Z".to_string(),
            verified_at: "2026-05-25T13:01:00Z".to_string(),
            recovery_authority_id: "finite-assisted-v1".to_string(),
            retention_policy: finite_saas_core::RUNTIME_RETIREMENT_RETENTION_INDEFINITE.to_string(),
        };
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control.clone()),
            FakeLauncher::ready(RuntimeLaunchFacts::sample())
                .for_kata()
                .with_retirement_result(Ok(receipt.clone())),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::RuntimeDestroyed {
                request_id: runtime_control.request.id,
                runtime_id: runtime_control.runtime.id,
            }
        );
        assert_eq!(runner.queue.renewed_runtime_control.len(), 1);
        assert_eq!(
            runner.queue.renewed_runtime_control[0].lease_seconds,
            Some(RUNTIME_RETIREMENT_LEASE_SECONDS)
        );
        assert_eq!(
            runner.queue.completed_runtime_control[0].retirement_snapshot,
            Some(receipt)
        );
        assert!(runner.queue.retried_runtime_control.is_empty());
        assert!(runner.queue.failed_runtime_control.is_empty());
    }

    #[test]
    fn retirement_failure_requeues_same_request_instead_of_terminal_failure() {
        let runtime_control =
            sample_runtime_control_lease_with_kind("runtime_ctl_123", RuntimeControlKind::Destroy);
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control.clone()),
            FakeLauncher::ready(RuntimeLaunchFacts::sample())
                .for_kata()
                .with_retirement_result(Err("synthetic archive failure".to_string())),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::RuntimeDestroyFailed {
                request_id: runtime_control.request.id.clone(),
                failure_message: "runtime launch failed: synthetic archive failure".to_string(),
            }
        );
        assert_eq!(runner.queue.retried_runtime_control.len(), 1);
        assert_eq!(
            runner.queue.retried_runtime_control[0].request_id,
            runtime_control.request.id
        );
        assert!(runner.queue.failed_runtime_control.is_empty());
        assert!(runner.queue.completed_runtime_control.is_empty());
    }

    #[test]
    fn run_once_fails_restart_when_runtime_does_not_publish_new_heartbeat() {
        let runtime_control = sample_runtime_control_lease("runtime_ctl_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control.clone()),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_runtime_ready_polling(Duration::from_millis(0), Duration::from_millis(1));

        let outcome = runner.run_once().unwrap();

        assert!(matches!(
            outcome,
            RunOnceOutcome::RuntimeRestartFailed { .. }
        ));
        assert_eq!(
            runner.launcher.restarted,
            vec!["oslo-agent-001".to_string()]
        );
        assert!(runner.queue.completed_runtime_control.is_empty());
        assert_eq!(runner.queue.failed_runtime_control.len(), 1);
        assert_eq!(
            runner.queue.failed_runtime_control[0].failure_message,
            "runtime launch failed: runtime did not publish a new relay heartbeat within 0s"
        );
    }

    #[test]
    fn run_once_completes_only_after_launcher_returns_runtime_facts() {
        let lease = sample_lease("agent_request_123");
        let provider_runtime_handle: ProviderRuntimeHandleEnvelope =
            serde_json::from_value(serde_json::json!({
                "schema": "provider_runtime_handle.v1",
                "handle": {
                    "runnerClass": "phala",
                    "opaque": {
                        "schema": "phala_runtime_handle.v1",
                        "handle": {
                            "cvmId": "cvm_fixture_01",
                            "appId": "app_fixture_01"
                        }
                    }
                }
            }))
            .unwrap();
        let mut facts = RuntimeLaunchFacts::sample();
        facts.provider_runtime_handle = Some(provider_runtime_handle.clone());
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease.clone()),
            FakeLauncher::ready(facts),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();
        assert_eq!(
            outcome,
            RunOnceOutcome::Launched {
                request_id: lease.request.id.clone(),
                runtime_id: Some("runtime-from-core".to_string()),
            }
        );
        assert_eq!(runner.launcher.launch_count, 1);
        assert_eq!(runner.queue.registered.len(), 1);
        assert_eq!(
            runner.queue.registered[0].runtime_relay_token_hash,
            "hash-runtime-token"
        );
        assert_eq!(
            runner.queue.registered[0].provider_runtime_handle,
            Some(provider_runtime_handle.clone())
        );
        assert_eq!(
            runner.queue.registered[0].contact_endpoint.as_deref(),
            Some("http://oslo-host-1/contact")
        );
        assert_eq!(
            runner.queue.heartbeat_checks,
            vec!["finite-agent_123".to_string()]
        );
        assert_eq!(runner.queue.completed.len(), 1);
        assert_eq!(
            runner.queue.completed[0].source_machine_id,
            "finite-agent_123"
        );
        assert_eq!(
            runner.queue.completed[0].runtime_status,
            Some(RuntimeSummaryStatus::Online)
        );
        assert_eq!(
            runner.queue.completed[0].provider_runtime_handle,
            Some(provider_runtime_handle)
        );
        assert_eq!(
            runner.queue.completed[0].contact_endpoint.as_deref(),
            Some("http://oslo-host-1/contact")
        );
        assert!(runner.queue.failed.is_empty());
    }

    #[test]
    fn run_once_binds_canonical_agent_email_before_completion() {
        use std::sync::mpsc;

        const AGENT_NPUB: &str = "npub1wvxx6jqkqkfmfp8xy6avumvsqyl8fdfzt2x98vrrwgl9w444cgkscsfp48";
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, received) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut contact, _) = listener.accept().unwrap();
            let request = read_http_request(&mut contact);
            assert!(request.starts_with("GET /contact "));
            let body = serde_json::json!({ "agent_npub": AGENT_NPUB }).to_string();
            write_http_json(&mut contact, 200, &body);

            let (mut identity, _) = listener.accept().unwrap();
            let request = read_http_request(&mut identity);
            assert!(request.starts_with("POST /api/v1/operator/agent-email-bindings "));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("x-finite-operator-token: identity-operator-token")
            );
            assert!(request.contains("\"email\":\"oslo-agent@finite.vip\""));
            assert!(request.contains(&format!("\"agent_npub\":\"{AGENT_NPUB}\"")));
            sent.send(request).unwrap();
            let body = serde_json::json!({
                "email": "oslo-agent@finite.vip",
                "agent_npub": AGENT_NPUB,
                "nip05": "oslo-agent@finite.vip",
            })
            .to_string();
            write_http_json(&mut identity, 200, &body);
        });

        let mut lease = sample_lease("agent_request_123");
        lease.project.agent_email = Some("oslo-agent@finite.vip".to_string());
        let mut facts = RuntimeLaunchFacts::sample();
        facts.contact_endpoint = Some(format!("http://{address}/contact"));
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease),
            FakeLauncher::ready(facts),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_agent_identity_authority(AgentIdentityAuthorityConfig {
            base_url: format!("http://{address}"),
            operator_token: "identity-operator-token".to_string(),
            timeout: Duration::from_secs(1),
        })
        .unwrap();

        let outcome = runner.run_once().unwrap();
        assert!(matches!(outcome, RunOnceOutcome::Launched { .. }));
        assert_eq!(runner.queue.completed.len(), 1);
        assert!(received.recv_timeout(Duration::from_secs(1)).is_ok());
        server.join().unwrap();
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 1024];
            let bytes = stream.read(&mut chunk).unwrap();
            assert!(bytes > 0, "HTTP request ended before its body was complete");
            request.extend_from_slice(&chunk[..bytes]);
            assert!(
                request.len() <= 16 * 1024,
                "HTTP request fixture is too large"
            );

            let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                return String::from_utf8(request).unwrap();
            }
        }
    }

    #[test]
    fn run_once_fails_closed_when_managed_agent_email_cannot_be_registered() {
        let mut lease = sample_lease("agent_request_123");
        lease.project.agent_email = Some("oslo-agent@finite.vip".to_string());
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::LaunchFailed { .. }));
        assert!(runner.queue.completed.is_empty());
        assert_eq!(runner.queue.failed.len(), 1);
        assert!(
            runner.queue.failed[0]
                .failure_message
                .contains("Identity Authority is not configured")
        );
    }

    fn write_http_json(stream: &mut impl Write, status: u16, body: &str) {
        write!(
            stream,
            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    }

    #[test]
    fn run_once_uses_only_the_core_bound_spec_and_resolves_every_secret_reference() {
        let lease = sample_spec_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()).with_runner_capacity(
                RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::LocalDocker],
                    ..RunnerLeaseCapacity::default()
                },
            ),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_runtime_environment(BTreeMap::from([(
            "FINITE_SITES_API".to_string(),
            "https://runner-default.invalid".to_string(),
        )]))
        .unwrap()
        .with_runtime_secret_environment(BTreeMap::from([
            ("FAL_KEY".to_string(), "fal-value".to_string()),
            ("XAI_API_KEY".to_string(), "unused-value".to_string()),
        ]))
        .unwrap()
        .with_default_finite_private_inference(finite_private_defaults());

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::Launched { .. }));
        let options = &runner.launcher.launch_options[0];
        assert_eq!(
            options.environment,
            BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "https://api.finite.chat".to_string(),
            )])
        );
        assert_eq!(
            options.secret_environment,
            BTreeMap::from([("FAL_KEY".to_string(), "fal-value".to_string())])
        );
        assert_eq!(
            options.finite_private.as_ref().unwrap().raw_api_key,
            "fpk_live_test"
        );
        assert_eq!(runner.queue.provisioned.len(), 1);
    }

    #[test]
    fn run_once_rejects_a_core_bound_spec_for_another_runner_class() {
        let mut lease = sample_spec_lease("agent_request_123");
        lease.request.runner_class = RunnerClass::Kata;
        let RuntimeSpecEnvelope::V1(spec) = lease.request.runtime_spec.as_mut().unwrap();
        spec.placement =
            RuntimePlacement::for_hosting_tier(finite_saas_core::HostingTier::Standard);
        lease.request.placement = Some(spec.placement);
        lease.project.placement = Some(spec.placement);
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()).with_runner_capacity(
                RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::LocalDocker],
                    ..RunnerLeaseCapacity::default()
                },
            ),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::LaunchFailed { .. }));
        assert_eq!(runner.launcher.launch_count, 0);
        assert_eq!(runner.queue.failed.len(), 1);
        assert!(
            runner.queue.failed[0]
                .failure_message
                .contains("RuntimeSpec violated the Runner contract")
        );
    }

    #[test]
    fn run_once_provisions_default_finite_private_key_before_launch() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease.clone()),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_default_finite_private_inference(finite_private_defaults());

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::Launched { .. }));
        assert_eq!(runner.queue.provisioned.len(), 1);
        assert_eq!(runner.queue.provisioned[0].request_id, lease.request.id);
        assert_eq!(
            runner.queue.provisioned[0].source_machine_id.as_deref(),
            Some("finite-agent_123")
        );
        assert_eq!(runner.launcher.launch_options.len(), 1);
        let finite_private = runner.launcher.launch_options[0]
            .finite_private
            .as_ref()
            .expect("finite private key should be passed to launcher");
        assert_eq!(finite_private.raw_api_key, "fpk_live_test");
        assert_eq!(finite_private.base_url, DEFAULT_FINITE_PRIVATE_BASE_URL);
        assert_eq!(finite_private.model, DEFAULT_FINITE_PRIVATE_MODEL);
        assert_eq!(finite_private.model, "glm-5-2");
        let specialization_bundle = finite_private
            .specialization_bundle
            .as_ref()
            .expect("configured specialization should be passed to launcher");
        assert_eq!(
            specialization_bundle.bundle_id,
            DEFAULT_FINITE_PRIVATE_SPECIALIZATION_BUNDLE
        );
        assert_eq!(
            specialization_bundle.worker_api_key,
            "specialization-worker-secret"
        );
        assert!(!format!("{finite_private:?}").contains("specialization-worker-secret"));
        assert_eq!(runner.queue.registered.len(), 1);
        assert_eq!(
            runner.queue.registered[0]
                .active_inference_profile
                .as_deref(),
            Some("finite-private")
        );
        assert!(runner.queue.failed.is_empty());
    }

    #[test]
    fn run_once_binds_key_to_launcher_host_when_provider_machine_is_not_known_yet() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()).without_planned_source(),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_default_finite_private_inference(finite_private_defaults());

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::Launched { .. }));
        assert_eq!(runner.queue.provisioned.len(), 1);
        assert_eq!(
            runner.queue.provisioned[0].source_host_id.as_deref(),
            Some("oslo-host-1")
        );
        assert_eq!(runner.queue.provisioned[0].source_machine_id, None);
    }

    #[test]
    fn run_once_uses_operator_finite_private_override_without_core_provisioning() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_default_finite_private_inference(FinitePrivateRuntimeDefaults {
            api_key_override: Some("fpk_operator_valid".to_string()),
            ..finite_private_defaults()
        });

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::Launched { .. }));
        assert!(runner.queue.provisioned.is_empty());
        let finite_private = runner.launcher.launch_options[0]
            .finite_private
            .as_ref()
            .expect("finite private key should be passed to launcher");
        assert_eq!(finite_private.api_key_id, "operator-override");
        assert_eq!(finite_private.raw_api_key, "fpk_operator_valid");
        assert!(!finite_private.revoke_on_launch_failure);
    }

    #[test]
    fn run_once_fails_closed_when_default_finite_private_key_cannot_be_issued() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease.clone()).with_provision_error("core unavailable"),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_default_finite_private_inference(finite_private_defaults());

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::LaunchFailed {
                request_id: lease.request.id,
                failure_message: "Core request failed: core unavailable".to_string(),
            }
        );
        assert_eq!(runner.launcher.launch_count, 0);
        assert_eq!(runner.queue.failed.len(), 1);
        assert!(
            runner.queue.failed[0]
                .provisioned_finite_private_api_key_id
                .is_none()
        );
        assert!(runner.queue.registered.is_empty());
    }

    #[test]
    fn run_once_launches_without_specialization_when_credential_is_missing() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_default_finite_private_inference(FinitePrivateRuntimeDefaults::default());

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::Launched { .. }));
        assert_eq!(runner.launcher.launch_count, 1);
        assert_eq!(runner.queue.provisioned.len(), 1);
        assert!(
            runner.launcher.launch_options[0]
                .finite_private
                .as_ref()
                .expect("Finite Private key should be passed to launcher")
                .specialization_bundle
                .is_none()
        );
        assert!(runner.queue.failed.is_empty());
    }

    #[test]
    fn run_once_fails_closed_when_configured_specialization_is_invalid() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_default_finite_private_inference(FinitePrivateRuntimeDefaults {
            specialization_bundle: Some(SpecializationBundleRuntimeDefaults {
                bundle_id: DEFAULT_FINITE_PRIVATE_SPECIALIZATION_BUNDLE.to_owned(),
                worker_api_key: " ".to_owned(),
            }),
            ..FinitePrivateRuntimeDefaults::default()
        });

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::LaunchFailed { .. }));
        assert_eq!(runner.launcher.launch_count, 0);
        assert!(runner.queue.provisioned.is_empty());
        assert!(
            runner.queue.failed[0]
                .failure_message
                .contains("specialization worker credential is empty or oversized")
        );
    }

    #[test]
    fn run_once_revokes_default_finite_private_key_when_launch_fails() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease.clone()),
            FakeLauncher::launch_error("docker run failed"),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_default_finite_private_inference(finite_private_defaults());

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::LaunchFailed { .. }));
        assert_eq!(runner.queue.failed.len(), 1);
        assert_eq!(
            runner.queue.failed[0]
                .provisioned_finite_private_api_key_id
                .as_deref(),
            Some("fp_key_123")
        );
    }

    #[test]
    fn run_once_does_not_revoke_operator_finite_private_override_when_launch_fails() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease),
            FakeLauncher::launch_error("docker run failed"),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_default_finite_private_inference(FinitePrivateRuntimeDefaults {
            api_key_override: Some("fpk_operator_valid".to_string()),
            ..finite_private_defaults()
        });

        let outcome = runner.run_once().unwrap();

        assert!(matches!(outcome, RunOnceOutcome::LaunchFailed { .. }));
        assert!(runner.queue.provisioned.is_empty());
        assert!(
            runner.queue.failed[0]
                .provisioned_finite_private_api_key_id
                .is_none()
        );
    }

    #[test]
    fn run_once_records_failure_when_launch_fails_after_lease() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease.clone()),
            FakeLauncher::launch_error("docker run failed"),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap();

        let outcome = runner.run_once().unwrap();
        assert_eq!(
            outcome,
            RunOnceOutcome::LaunchFailed {
                request_id: lease.request.id.clone(),
                failure_message: "runtime launch failed: docker run failed".to_string(),
            }
        );
        assert!(runner.queue.completed.is_empty());
        assert_eq!(runner.queue.failed.len(), 1);
        assert_eq!(
            runner.queue.failed[0].failure_message,
            "runtime launch failed: docker run failed"
        );
    }

    #[test]
    fn run_once_records_failure_when_runtime_never_heartbeats() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease.clone()).without_heartbeat(),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_runtime_ready_polling(Duration::from_millis(0), Duration::from_millis(1));

        let outcome = runner.run_once().unwrap();
        assert_eq!(
            outcome,
            RunOnceOutcome::LaunchFailed {
                request_id: lease.request.id.clone(),
                failure_message:
                    "runtime launch failed: runtime did not publish a relay heartbeat within 0s"
                        .to_string(),
            }
        );
        assert_eq!(runner.queue.registered.len(), 1);
        assert_eq!(
            runner.queue.heartbeat_checks,
            vec!["finite-agent_123".to_string()]
        );
        assert!(runner.queue.completed.is_empty());
        assert_eq!(runner.queue.failed.len(), 1);
    }

    #[test]
    fn run_once_allows_launcher_owned_readiness_without_core_heartbeat() {
        let lease = sample_lease("agent_request_123");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease.clone()).without_heartbeat(),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()).without_core_heartbeat(),
            FixedLeaseTokens::new(["lease-1"]),
            "runner-1",
            300,
        )
        .unwrap()
        .with_runtime_ready_polling(Duration::from_millis(0), Duration::from_millis(1));

        let outcome = runner.run_once().unwrap();

        assert_eq!(
            outcome,
            RunOnceOutcome::Launched {
                request_id: lease.request.id,
                runtime_id: Some("runtime-from-core".to_string()),
            }
        );
        assert_eq!(runner.queue.registered.len(), 1);
        assert!(runner.queue.heartbeat_checks.is_empty());
        assert_eq!(runner.queue.completed.len(), 1);
    }

    #[test]
    fn production_runtime_image_uses_data_mount_and_finitechat_entrypoint() {
        let dockerfile = read_repo_file("deploy/finite-computer/images/runtime.Dockerfile");

        assert!(dockerfile.contains("ENV FINITECHAT_HOME=/data/agent"));
        assert!(dockerfile.contains("ENV FINITE_HOME=/data/agent"));
        assert!(dockerfile.contains("ENV HERMES_HOME=/data/agent/hermes-home"));
        assert!(dockerfile.contains("ENV FINITECHAT_WORKSPACE=/data/workspace"));
        assert!(dockerfile.contains("ENV FBRAIN_CONFIG_DIR=/data/agent/fbrain"));
        assert!(dockerfile.contains("ENV FBRAIN_WORKING_TREE_ROOT=/data/workspace/finitebrain"));
        assert!(dockerfile.contains("ENTRYPOINT [\"/opt/agent-entrypoint.sh\"]"));
        assert!(!dockerfile.contains("finitechat-entrypoint.sh"));
        assert!(!dockerfile.contains("/finite-state"));
    }

    #[test]
    fn bundled_finitebrain_skill_uses_runtime_brain_and_durable_paths() {
        let bundled =
            read_repo_file("../finite-skills/skills/software-development/finitebrain/SKILL.md");
        let package = read_repo_file("../finite-brain/skills/finitebrain/SKILL.md");

        assert_eq!(bundled, package);
        assert!(bundled.contains("FINITE_BRAIN_SERVER_URL"));
        assert!(bundled.contains("FBRAIN_CONFIG_DIR"));
        assert!(bundled.contains("FBRAIN_WORKING_TREE_ROOT"));
        assert!(!bundled.contains("SERVER=\"https://finite.computer\""));
        assert!(!bundled.contains("TREE=\"$HOME/finitebrain/$VAULT\""));
    }

    #[test]
    fn production_runtime_healthcheck_uses_only_the_authoritative_readiness_endpoint() {
        let healthcheck = read_repo_file("deploy/finite-computer/runtime-template/healthcheck.sh");

        assert!(!healthcheck.contains("source "));
        assert!(healthcheck.contains("http://${agent_http_host}:${agent_http_port}/healthz"));
        assert!(healthcheck.contains("exec curl -fsS --max-time 4"));
        assert!(!healthcheck.contains("/runtime/bin/"));
        assert!(!healthcheck.contains("finitechat identity"));
        assert!(!healthcheck.contains("finite-agentd status"));
        assert!(!healthcheck.contains("/runtime/env/runtime.env"));
    }

    #[test]
    fn production_runtime_healthcheck_targets_the_configured_loopback_service() {
        let temp = tempfile::tempdir().unwrap();
        let fake_curl = temp.path().join("curl");
        let args_file = temp.path().join("curl-args");
        std::fs::write(
            &fake_curl,
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > \"$PROBE_ARGS_FILE\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake_curl, std::fs::Permissions::from_mode(0o755)).unwrap();

        let existing_path = std::env::var_os("PATH").unwrap_or_default();
        let mut path = std::ffi::OsString::from(temp.path());
        path.push(":");
        path.push(existing_path);
        let output = Command::new("bash")
            .arg(repo_path(
                "deploy/finite-computer/runtime-template/healthcheck.sh",
            ))
            .env("PATH", path)
            .env("PROBE_ARGS_FILE", &args_file)
            .env("FINITE_AGENT_HTTP_HEALTH_HOST", "127.0.0.9")
            .env("FINITE_AGENT_HTTP_PORT", "18080")
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "healthcheck failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(args_file).unwrap(),
            "-fsS\n--max-time\n4\nhttp://127.0.0.9:18080/healthz\n"
        );
    }

    #[test]
    fn production_runtime_template_scripts_parse_as_bash() {
        let relative_path = "deploy/finite-computer/runtime-template/healthcheck.sh";
        let output = Command::new("bash")
            .arg("-n")
            .arg(repo_path(relative_path))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} failed bash -n: {}",
            relative_path,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn production_runner_systemd_preserves_provider_runtimes() {
        let unit = read_repo_file("../infra/hosts/lat1/systemd/finite-saas-runner.service");

        assert!(unit.contains("KillMode=process"));
    }

    #[test]
    fn runner_binary_advertises_one_worker_class_without_product_backend_switching() {
        let main_rs = read_repo_file("crates/finite-saas-runner/src/main.rs");

        assert!(main_rs.contains(r#"optional_env("FC_RUNNER_CLASS", "local_docker")"#));
        assert!(main_rs.contains(r#""kata" =>"#));
        assert!(main_rs.contains(r#""enclavia" =>"#));
        assert!(!main_rs.contains("FC_RUNNER_BACKEND"));
    }

    #[test]
    fn docker_plan_mounts_durable_state_and_targets_hosted_finitechat() {
        let config = DockerConfig {
            source_host_id: "local-docker".to_string(),
            image: "ghcr.io/finitecomputer/finite-chat-hermes-runtime:local".to_string(),
            runtime_artifact_id: Some("artifact-v1".to_string()),
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: Some("state-v1".to_string()),
            work_root: PathBuf::from("/var/lib/finite/runner"),
            host_port: 18081,
            ..DockerConfig::default()
        };
        config.validate().unwrap();
        let lease = sample_lease("agent_request_abc.123");
        let plan = docker_launch_plan(&config, &lease);
        let options = RuntimeLaunchOptions {
            finite_private: Some(FinitePrivateLaunchKey {
                api_key_id: "fp_key_123".to_string(),
                raw_api_key: "fpk_live_test".to_string(),
                base_url: DEFAULT_FINITE_PRIVATE_BASE_URL.to_string(),
                model: DEFAULT_FINITE_PRIVATE_MODEL.to_string(),
                revoke_on_launch_failure: true,
                specialization_bundle: Some(specialization_bundle_defaults()),
            }),
            profile_picture_url: None,
            environment: BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "http://192.168.64.1:18789".to_string(),
            )]),
            secret_environment: BTreeMap::new(),
        };

        assert_eq!(plan.container_name, "finite-agent_abc-123");
        assert_eq!(
            plan.state_root,
            PathBuf::from("/var/lib/finite/runner/docker/finite-agent_abc-123")
        );
        assert_eq!(plan.public_base_url, "http://127.0.0.1:18081");
        assert_eq!(plan.health_url, "http://127.0.0.1:18081/healthz");
        assert_eq!(plan.contact_url, "http://127.0.0.1:18081/contact");

        let env = docker_runtime_env(&config, &plan, &lease, &options);
        assert_env(&env, "FINITE_SERVER_URL", DEFAULT_FINITECHAT_SERVER_URL);
        assert_env(&env, "FINITECHAT_HOME", "/data/agent");
        assert_env(&env, "FINITE_HOME", "/data/agent");
        assert_env(&env, "FINITECHAT_WORKSPACE", "/data/workspace");
        assert_env(&env, "FINITECHAT_HERMES_AGENT_DEVICE_ID", "agent");
        assert_env(&env, "FINITECHAT_HERMES_PROVIDER", "custom");
        assert_env(&env, "FINITECHAT_HERMES_MODEL", "glm-5-2");
        assert_env(&env, "FINITE_PRIVATE_MODEL", "glm-5-2");
        // The endpoint domain keeps the historical kimi name; the served model
        // is glm-5-2.
        assert_env(
            &env,
            "FINITECHAT_HERMES_BASE_URL",
            "https://kimi-k2-6.finite.containers.tinfoil.dev/v1",
        );
        assert_env(&env, "FINITE_PRIVATE_API_KEY", "fpk_live_test");
        assert_env(&env, "OPENAI_API_KEY", "fpk_live_test");
        assert_env(
            &env,
            FINITE_SPECIALIZATION_BUNDLE_ENV,
            DEFAULT_FINITE_PRIVATE_SPECIALIZATION_BUNDLE,
        );
        assert_env(
            &env,
            FINITE_SPECIALIZATION_WORKER_API_KEY_ENV,
            "specialization-worker-secret",
        );
        let mut options_without_specialization = options.clone();
        options_without_specialization
            .finite_private
            .as_mut()
            .expect("Finite Private key should be present")
            .specialization_bundle = None;
        let env_without_specialization =
            docker_runtime_env(&config, &plan, &lease, &options_without_specialization);
        assert!(
            env_without_specialization
                .iter()
                .all(|(key, _)| key != FINITE_SPECIALIZATION_BUNDLE_ENV)
        );
        assert!(
            env_without_specialization
                .iter()
                .all(|(key, _)| key != FINITE_SPECIALIZATION_WORKER_API_KEY_ENV)
        );
        assert_env(&env, "FINITE_SITES_API", "http://192.168.64.1:18789");
        assert!(
            env.iter()
                .all(|(key, _)| key.as_str() != "OPENROUTER_API_KEY")
        );

        let command = docker_run_command(&config, &plan, &lease, &options);
        let args = os_strings_to_strings(&command.args);
        assert!(args.windows(2).any(|pair| pair == ["--pull", "missing"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-p", "127.0.0.1:18081:8080"])
        );
        assert!(args.windows(2).any(|pair| {
            pair == [
                "-v",
                "/var/lib/finite/runner/docker/finite-agent_abc-123:/data",
            ]
        }));
        assert!(
            args.windows(2)
                .any(|pair| { pair == ["--env", "FINITECHAT_HERMES_BASE_URL"] })
        );
        assert!(args.iter().all(|arg| !arg.contains("fpk_live_test")));
        assert!(!format!("{command:?}").contains("fpk_live_test"));
        assert_eq!(
            args.last().map(String::as_str),
            Some("ghcr.io/finitecomputer/finite-chat-hermes-runtime:local")
        );
    }

    #[test]
    fn kata_plan_is_microvm_isolated_durable_and_keeps_secrets_out_of_argv() {
        let config = KataConfig {
            nerdctl_bin: PathBuf::from("/run/current-system/sw/bin/nerdctl"),
            kata_runtime_bin: PathBuf::from("/run/current-system/sw/bin/kata-runtime"),
            source_host_id: "finite-lat-1".to_string(),
            image: "ghcr.io/finitecomputer/finite-agent-runtime:prod@sha256:abc123".to_string(),
            runtime_artifact_id: Some("artifact-prod".to_string()),
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: Some("runtime-state-v1".to_string()),
            work_root: PathBuf::from("/var/lib/finite-saas-runner"),
            ..KataConfig::default()
        };
        config.validate().unwrap();
        let lease = sample_lease("agent_request_ABC.123");
        let options = RuntimeLaunchOptions {
            finite_private: Some(FinitePrivateLaunchKey {
                api_key_id: "fp_key_prod".to_string(),
                raw_api_key: "fpk_must_never_reach_argv".to_string(),
                base_url: DEFAULT_FINITE_PRIVATE_BASE_URL.to_string(),
                model: DEFAULT_FINITE_PRIVATE_MODEL.to_string(),
                revoke_on_launch_failure: true,
                specialization_bundle: Some(specialization_bundle_defaults()),
            }),
            profile_picture_url: Some("https://chat.finite.computer/blobs/profile".to_string()),
            environment: BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "http://10.88.0.1:8789".to_string(),
            )]),
            secret_environment: BTreeMap::from([(
                "FAL_KEY".to_string(),
                "fal_must_never_reach_argv".to_string(),
            )]),
        };
        let plan = kata::kata_launch_plan(&config, &lease);
        assert_eq!(plan.container_name, "finite-kata-abc-123");
        assert_eq!(
            plan.state_root,
            PathBuf::from("/var/lib/finite-saas-runner/kata/finite-kata-abc-123")
        );
        assert_eq!(
            plan.env_file,
            PathBuf::from(
                "/var/lib/finite-saas-runner/kata-metadata/finite-kata-abc-123/launch.env"
            )
        );

        let command =
            kata::kata_run_command(&config, &plan, &lease, &RuntimeLaunchOptions::default());
        let args = os_strings_to_strings(&command.args);
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--namespace", "finite"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--runtime", "io.containerd.kata.v2"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--publish", "127.0.0.1::8080"])
        );
        assert!(args.windows(2).any(|pair| {
            pair == [
                "--volume",
                "/var/lib/finite-saas-runner/kata/finite-kata-abc-123:/data",
            ]
        }));
        assert!(args.windows(2).any(|pair| {
            pair == [
                "--env-file",
                "/var/lib/finite-saas-runner/kata-metadata/finite-kata-abc-123/launch.env",
            ]
        }));
        assert!(args.windows(2).any(|pair| {
            pair == [
                "--label",
                "computer.finite.v2.runtime_artifact_id=artifact-prod",
            ]
        }));
        assert!(args.iter().all(|arg| !arg.contains("fpk_must_never")));
        assert!(args.iter().all(|arg| !arg.contains("fal_must_never")));
        assert!(!format!("{command:?}").contains("fpk_must_never"));
        assert!(!format!("{command:?}").contains("fal_must_never"));
        assert_eq!(args.last().map(String::as_str), Some(config.image.as_str()));

        let env = kata::kata_runtime_env(&config, &plan, &lease, &options);
        assert_env(&env, "FINITE_PRIVATE_API_KEY", "fpk_must_never_reach_argv");
        assert_env(
            &env,
            "FINITECHAT_HERMES_AGENT_PICTURE_URL",
            "https://chat.finite.computer/blobs/profile",
        );
        assert_env(&env, "FINITE_SITES_API", "http://10.88.0.1:8789");
        assert_env(&env, "FAL_KEY", "fal_must_never_reach_argv");

        let temp = tempfile::tempdir().unwrap();
        let env_file = temp.path().join("launch.env");
        kata::write_kata_env_file(&env_file, &env).unwrap();
        let metadata = std::fs::metadata(&env_file).unwrap();
        #[cfg(unix)]
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert!(
            std::fs::read_to_string(env_file)
                .unwrap()
                .contains("FINITE_PRIVATE_API_KEY=fpk_must_never_reach_argv")
        );
    }

    #[test]
    fn opaque_runtime_environment_is_bounded_non_secret_and_cannot_override_contract() {
        let valid = BTreeMap::from([(
            "FINITE_SITES_API".to_string(),
            "http://192.168.64.1:18789".to_string(),
        )]);
        validate_runtime_environment(&valid).unwrap();
        let restart_options = RuntimeRestartOptions::new(valid.clone()).unwrap();

        for key in [
            "FINITECHAT_SERVER_URL",
            "FINITE_AGENT_BOOT_INTENT_JSON",
            "FINITE_AGENT_STATE_ROOT",
            "FINITE_HOME",
            "OPENAI_API_KEY",
            "GOOGLE_OAUTH_TOKEN",
            "lowercase",
        ] {
            let invalid = BTreeMap::from([(key.to_string(), "value".to_string())]);
            assert!(validate_runtime_environment(&invalid).is_err(), "{key}");
        }
        assert!(
            !format!(
                "{:?}",
                RuntimeLaunchOptions {
                    finite_private: None,
                    profile_picture_url: None,
                    environment: valid,
                    secret_environment: BTreeMap::from([(
                        "FAL_KEY".to_string(),
                        "fal_debug_secret".to_string(),
                    )]),
                }
            )
            .contains("192.168.64.1")
        );
        assert!(
            !format!(
                "{:?}",
                RuntimeLaunchOptions {
                    secret_environment: BTreeMap::from([(
                        "FAL_KEY".to_string(),
                        "fal_debug_secret".to_string(),
                    )]),
                    ..RuntimeLaunchOptions::default()
                }
            )
            .contains("fal_debug_secret")
        );
        assert!(!format!("{restart_options:?}").contains("192.168.64.1"));
        let secret_restart_options = RuntimeRestartOptions::default()
            .with_secret_environment(BTreeMap::from([(
                "FAL_KEY".to_string(),
                "fal_refresh_secret".to_string(),
            )]))
            .unwrap();
        assert!(!format!("{secret_restart_options:?}").contains("fal_refresh_secret"));
        assert!(format!("{secret_restart_options:?}").contains("FAL_KEY"));
        assert!(
            RuntimeRestartOptions::new(BTreeMap::from([(
                "GOOGLE_OAUTH_TOKEN".to_string(),
                "must-not-cross-this-boundary".to_string(),
            )]))
            .is_err()
        );
    }

    #[test]
    fn opaque_runtime_secret_environment_is_bounded_disjoint_and_value_redacted() {
        let secrets = BTreeMap::from([
            ("FAL_KEY".to_string(), "fal_test_secret".to_string()),
            ("XAI_API_KEY".to_string(), "xai_test_secret".to_string()),
        ]);
        validate_runtime_secret_environment(&secrets).unwrap();
        validate_runtime_environment_disjoint(&BTreeMap::new(), &secrets).unwrap();

        for key in ["FINITE_PRIVATE_API_KEY", "OPENAI_API_KEY", "lowercase"] {
            let invalid = BTreeMap::from([(key.to_string(), "secret".to_string())]);
            assert!(
                validate_runtime_secret_environment(&invalid).is_err(),
                "{key}"
            );
        }
        assert!(
            validate_runtime_environment_disjoint(
                &BTreeMap::from([("SENTRY_DSN".to_string(), "public".to_string())]),
                &BTreeMap::from([("SENTRY_DSN".to_string(), "secret".to_string())]),
            )
            .is_err()
        );
        let debug = format!(
            "{:?}",
            RuntimeLaunchOptions {
                secret_environment: secrets,
                ..RuntimeLaunchOptions::default()
            }
        );
        assert!(debug.contains("FAL_KEY"));
        assert!(!debug.contains("fal_test_secret"));
        assert!(!debug.contains("xai_test_secret"));
    }

    #[test]
    fn docker_runtime_readiness_depends_only_on_generic_health() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let bytes_read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..bytes_read]);
            assert!(request.starts_with("GET /healthz "));

            let body = r#"{"ready":true}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let config = DockerConfig {
            host_port: address.port(),
            readiness_timeout: Duration::from_secs(1),
            readiness_interval: Duration::from_millis(10),
            ..DockerConfig::default()
        };
        let launcher = DockerLauncher::new(config.clone());
        let public_base_url = format!("http://{address}");
        let plan = DockerLaunchPlan {
            container_name: "finite-agent-health-only".to_string(),
            state_root: config.work_root.join("docker/finite-agent-health-only"),
            health_url: format!("{public_base_url}/healthz"),
            contact_url: format!("{public_base_url}/contact"),
            public_base_url,
            host_port: address.port(),
            container_port: config.container_port,
        };

        launcher.wait_for_runtime_http(&plan).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn apple_container_plan_is_native_durable_and_keeps_secrets_out_of_argv() {
        let config = AppleContainerConfig {
            container_bin: PathBuf::from("/usr/local/bin/container"),
            source_host_id: "devfinity-apple".to_string(),
            image: "finite-agent-runtime:devfinity".to_string(),
            runtime_artifact_id: Some("devfinity-runtime".to_string()),
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: Some("runtime-state-v1".to_string()),
            work_root: PathBuf::from("/tmp/devfinity/runner"),
            finitechat_server_url: "http://192.168.64.1:18787".to_string(),
            name_prefix: "finite-devfinity".to_string(),
            host_port: 18080,
            ..AppleContainerConfig::default()
        };
        config.validate().unwrap();
        let lease = sample_lease("agent_request_ABC.123");
        let options = RuntimeLaunchOptions {
            finite_private: Some(FinitePrivateLaunchKey {
                api_key_id: "fp_key_local".to_string(),
                raw_api_key: "fpk_super_secret_local_value".to_string(),
                base_url: "http://192.168.64.1:18002/v1".to_string(),
                model: DEFAULT_FINITE_PRIVATE_MODEL.to_string(),
                revoke_on_launch_failure: true,
                specialization_bundle: Some(specialization_bundle_defaults()),
            }),
            profile_picture_url: None,
            environment: BTreeMap::new(),
            secret_environment: BTreeMap::from([(
                "FAL_KEY".to_string(),
                "fal_phala_secret".to_string(),
            )]),
        };

        let plan = apple_container::apple_container_launch_plan(&config, &lease);
        assert_eq!(plan.container_name, "finite-devfinity-abc-123");
        assert_eq!(
            plan.state_root,
            PathBuf::from("/tmp/devfinity/runner/apple-container/finite-devfinity-abc-123")
        );
        assert_eq!(plan.health_url, "http://127.0.0.1:18080/healthz");
        assert_eq!(plan.contact_url, "http://127.0.0.1:18080/contact");

        let command =
            apple_container::apple_container_run_command(&config, &plan, &lease, &options);
        let args = apple_container::apple_command_args(&command);
        let env_keys = apple_container::apple_command_env_keys(&command);
        let debug = format!("{command:?}");

        assert!(
            args.windows(2)
                .any(|pair| pair == ["--publish", "127.0.0.1:18080:8080/tcp"])
        );
        assert!(args.windows(2).any(|pair| {
            pair == [
                "--volume",
                "/tmp/devfinity/runner/apple-container/finite-devfinity-abc-123:/data",
            ]
        }));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--platform", "linux/arm64"])
        );
        assert!(args.windows(2).any(|pair| {
            pair == [
                "--label",
                "computer.finite.v2.runtime_artifact_id=devfinity-runtime",
            ]
        }));
        assert!(args.windows(2).any(|pair| {
            pair == [
                "--label",
                "computer.finite.v2.state_schema_version=runtime-state-v1",
            ]
        }));
        assert!(!args.iter().any(|arg| arg == "--restart" || arg == "--pull"));
        assert!(env_keys.iter().any(|key| key == "FINITE_PRIVATE_API_KEY"));
        assert!(env_keys.iter().any(|key| key == "OPENAI_API_KEY"));
        assert!(env_keys.iter().any(|key| key == "FAL_KEY"));
        assert!(args.iter().all(|arg| !arg.contains("fpk_super_secret")));
        assert!(args.iter().all(|arg| !arg.contains("fal_phala_secret")));
        assert!(!debug.contains("fpk_super_secret"));
        assert!(!debug.contains("fal_phala_secret"));
        assert_eq!(args.last().map(String::as_str), Some(config.image.as_str()));
    }

    #[test]
    fn apple_container_rosetta_requires_amd64() {
        let config = AppleContainerConfig {
            source_host_id: "local".to_string(),
            image: "runtime:local".to_string(),
            work_root: PathBuf::from("/tmp/runner"),
            rosetta: true,
            platform: Some("linux/arm64".to_string()),
            ..AppleContainerConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn enclavia_plan_targets_precreated_enclave_and_proxy_urls() {
        let config = EnclaviaConfig {
            enclavia_bin: PathBuf::from("/usr/local/bin/enclavia"),
            source_host_id: "enclavia-test".to_string(),
            image: "ghcr.io/finitecomputer/finite-agent-runtime:canary@sha256:abc123".to_string(),
            runtime_artifact_id: Some("artifact-v1".to_string()),
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: Some("state-v1".to_string()),
            enclave_id: "1d2c3b4a-5e6f-7a8b-9c0d-1e2f3a4b5c6d".to_string(),
            ..EnclaviaConfig::default()
        };
        config.validate().unwrap();

        let lease = sample_lease("agent_request_ABC.123");
        let plan = enclavia_launch_plan(&config);
        let endpoint = enclavia_endpoint(&plan.enclave_id);
        let options = RuntimeLaunchOptions {
            finite_private: Some(FinitePrivateLaunchKey {
                api_key_id: "fp_key_123".to_string(),
                raw_api_key: "fpk_live_test".to_string(),
                base_url: DEFAULT_FINITE_PRIVATE_BASE_URL.to_string(),
                model: DEFAULT_FINITE_PRIVATE_MODEL.to_string(),
                revoke_on_launch_failure: true,
                specialization_bundle: Some(specialization_bundle_defaults()),
            }),
            profile_picture_url: None,
            environment: BTreeMap::new(),
            secret_environment: BTreeMap::new(),
        };
        let env = enclavia_runtime_env(&config, &plan, &lease, &options);

        assert_eq!(plan.enclave_id, "1d2c3b4a-5e6f-7a8b-9c0d-1e2f3a4b5c6d");
        assert_eq!(
            endpoint.public_base_url,
            "https://1d2c3b4a-5e6f-7a8b-9c0d-1e2f3a4b5c6d.enclaves.beta.enclavia.io/proxy"
        );
        assert_eq!(
            endpoint.health_url,
            "https://1d2c3b4a-5e6f-7a8b-9c0d-1e2f3a4b5c6d.enclaves.beta.enclavia.io/proxy/healthz"
        );
        assert_eq!(
            endpoint.contact_url,
            "https://1d2c3b4a-5e6f-7a8b-9c0d-1e2f3a4b5c6d.enclaves.beta.enclavia.io/proxy/contact"
        );
        assert_env(&env, "FINITECHAT_HOME", "/data/agent");
        assert_env(&env, "FINITE_HOME", "/data/agent");
        assert_env(&env, "HERMES_HOME", "/data/agent/hermes-home");
        assert_env(&env, "FINITECHAT_WORKSPACE", "/data/workspace");
        assert_env(
            &env,
            "FINITECHAT_HERMES_AGENT_DEVICE_ID",
            "1d2c3b4a-5e6f-7a8b-9c0d-1e2f3a4b5c6d",
        );
        assert_env(&env, "FINITE_PRIVATE_API_KEY", "fpk_live_test");
        assert!(env.len() <= 32, "Enclavia secret cap is 32 rows");
    }

    #[test]
    fn enclavia_secret_argv_policy_keeps_raw_keys_on_stdin() {
        assert!(enclavia_env_value_uses_stdin("FINITE_PRIVATE_API_KEY"));
        assert!(enclavia_env_value_uses_stdin("OPENAI_API_KEY"));
        assert!(enclavia_env_value_uses_stdin("FAL_KEY"));
        assert!(enclavia_env_value_uses_stdin("XAI_API_KEY"));
        assert!(!enclavia_env_value_uses_stdin("FINITECHAT_SERVER_URL"));
        assert!(!enclavia_env_value_uses_stdin("FINITE_AGENT_NAME"));
    }

    #[derive(Debug)]
    struct FixedLeaseTokens {
        tokens: VecDeque<String>,
    }

    impl<const N: usize> From<[&str; N]> for FixedLeaseTokens {
        fn from(tokens: [&str; N]) -> Self {
            Self::new(tokens)
        }
    }

    impl FixedLeaseTokens {
        fn new<const N: usize>(tokens: [&str; N]) -> Self {
            Self {
                tokens: tokens.iter().map(|token| token.to_string()).collect(),
            }
        }
    }

    impl LeaseTokenSource for FixedLeaseTokens {
        fn next_lease_token(&mut self) -> Result<String, RunnerError> {
            self.tokens
                .pop_front()
                .ok_or(RunnerError::MissingLeaseToken)
        }
    }

    #[derive(Debug)]
    struct FakeQueue {
        next_lease: Option<AgentCreationLease>,
        next_runtime_control_lease: Option<RuntimeControlLease>,
        heartbeat: Option<RelayHeartbeat>,
        next_heartbeat: Option<RelayHeartbeat>,
        provision_error: Option<String>,
        leases: Vec<(String, String, i64)>,
        lease_capacities: Vec<Option<RunnerLeaseCapacity>>,
        runtime_control_leases: Vec<(String, String, i64, Option<String>)>,
        runtime_control_capacities: Vec<Option<RunnerLeaseCapacity>>,
        completed_runtime_control: Vec<CompleteRuntimeControlRequestInput>,
        failed_runtime_control: Vec<FailRuntimeControlRequestInput>,
        renewed_runtime_control: Vec<RenewRuntimeControlRequestInput>,
        retried_runtime_control: Vec<RetryRuntimeControlRequestInput>,
        provisioned: Vec<ProvisionFinitePrivateRuntimeKeyInput>,
        registered: Vec<RegisterAgentCreationRuntimeInput>,
        provider_operation_transitions: Vec<RecordProviderOperationTransitionRequest>,
        heartbeat_checks: Vec<String>,
        completed: Vec<CompleteAgentCreationRequestInput>,
        failed: Vec<FailAgentCreationRequestInput>,
    }

    impl FakeQueue {
        fn idle() -> Self {
            Self {
                next_lease: None,
                next_runtime_control_lease: None,
                heartbeat: Some(sample_heartbeat("finite-agent_123")),
                next_heartbeat: None,
                provision_error: None,
                leases: Vec::new(),
                lease_capacities: Vec::new(),
                runtime_control_leases: Vec::new(),
                runtime_control_capacities: Vec::new(),
                completed_runtime_control: Vec::new(),
                failed_runtime_control: Vec::new(),
                renewed_runtime_control: Vec::new(),
                retried_runtime_control: Vec::new(),
                provisioned: Vec::new(),
                registered: Vec::new(),
                provider_operation_transitions: Vec::new(),
                heartbeat_checks: Vec::new(),
                completed: Vec::new(),
                failed: Vec::new(),
            }
        }

        fn with_lease(lease: AgentCreationLease) -> Self {
            Self {
                next_lease: Some(lease),
                next_runtime_control_lease: None,
                heartbeat: Some(sample_heartbeat("finite-agent_123")),
                next_heartbeat: None,
                provision_error: None,
                leases: Vec::new(),
                lease_capacities: Vec::new(),
                runtime_control_leases: Vec::new(),
                runtime_control_capacities: Vec::new(),
                completed_runtime_control: Vec::new(),
                failed_runtime_control: Vec::new(),
                renewed_runtime_control: Vec::new(),
                retried_runtime_control: Vec::new(),
                provisioned: Vec::new(),
                registered: Vec::new(),
                provider_operation_transitions: Vec::new(),
                heartbeat_checks: Vec::new(),
                completed: Vec::new(),
                failed: Vec::new(),
            }
        }

        fn with_runtime_control_lease(lease: RuntimeControlLease) -> Self {
            Self {
                next_lease: None,
                next_runtime_control_lease: Some(lease),
                heartbeat: Some(sample_heartbeat("oslo-agent-001")),
                next_heartbeat: None,
                provision_error: None,
                leases: Vec::new(),
                lease_capacities: Vec::new(),
                runtime_control_leases: Vec::new(),
                runtime_control_capacities: Vec::new(),
                completed_runtime_control: Vec::new(),
                failed_runtime_control: Vec::new(),
                renewed_runtime_control: Vec::new(),
                retried_runtime_control: Vec::new(),
                provisioned: Vec::new(),
                registered: Vec::new(),
                provider_operation_transitions: Vec::new(),
                heartbeat_checks: Vec::new(),
                completed: Vec::new(),
                failed: Vec::new(),
            }
        }

        fn without_heartbeat(mut self) -> Self {
            self.heartbeat = None;
            self
        }

        fn with_next_heartbeat(mut self, heartbeat: RelayHeartbeat) -> Self {
            self.next_heartbeat = Some(heartbeat);
            self
        }

        fn with_provision_error(mut self, message: &str) -> Self {
            self.provision_error = Some(message.to_string());
            self
        }
    }

    impl AgentCreationQueue for FakeQueue {
        fn lease_runtime_control(
            &mut self,
            runner_id: &str,
            lease_token: &str,
            lease_seconds: i64,
            source_host_id: Option<&str>,
            runner_capacity: Option<&RunnerLeaseCapacity>,
        ) -> Result<Option<RuntimeControlLease>, RunnerError> {
            self.runtime_control_leases.push((
                runner_id.to_string(),
                lease_token.to_string(),
                lease_seconds,
                source_host_id.map(str::to_string),
            ));
            self.runtime_control_capacities
                .push(runner_capacity.cloned());
            Ok(self.next_runtime_control_lease.take())
        }

        fn complete_runtime_control(
            &mut self,
            _request_id: &str,
            input: CompleteRuntimeControlRequestInput,
        ) -> Result<RuntimeControlRequest, RunnerError> {
            self.completed_runtime_control.push(input);
            Ok(sample_runtime_control_lease("runtime_ctl_123").request)
        }

        fn fail_runtime_control(
            &mut self,
            _request_id: &str,
            input: FailRuntimeControlRequestInput,
        ) -> Result<RuntimeControlRequest, RunnerError> {
            self.failed_runtime_control.push(input);
            Ok(sample_runtime_control_lease("runtime_ctl_123").request)
        }

        fn renew_runtime_control(
            &mut self,
            _request_id: &str,
            input: RenewRuntimeControlRequestInput,
        ) -> Result<RuntimeControlRequest, RunnerError> {
            self.renewed_runtime_control.push(input);
            Ok(sample_runtime_control_lease("runtime_ctl_123").request)
        }

        fn retry_runtime_control(
            &mut self,
            _request_id: &str,
            input: RetryRuntimeControlRequestInput,
        ) -> Result<RuntimeControlRequest, RunnerError> {
            self.retried_runtime_control.push(input);
            Ok(sample_runtime_control_lease("runtime_ctl_123").request)
        }

        fn lease_agent_creation(
            &mut self,
            runner_id: &str,
            lease_token: &str,
            lease_seconds: i64,
            runner_capacity: Option<&RunnerLeaseCapacity>,
        ) -> Result<Option<AgentCreationLease>, RunnerError> {
            self.leases.push((
                runner_id.to_string(),
                lease_token.to_string(),
                lease_seconds,
            ));
            self.lease_capacities.push(runner_capacity.cloned());
            Ok(self.next_lease.take())
        }

        fn complete_agent_creation(
            &mut self,
            _request_id: &str,
            input: CompleteAgentCreationRequestInput,
        ) -> Result<AgentCreationLease, RunnerError> {
            self.completed.push(input);
            let mut lease = sample_lease("agent_request_123");
            lease.request.agent_runtime_id = Some("runtime-from-core".to_string());
            Ok(lease)
        }

        fn register_agent_creation_runtime(
            &mut self,
            _request_id: &str,
            input: RegisterAgentCreationRuntimeInput,
        ) -> Result<AgentCreationLease, RunnerError> {
            self.registered.push(input);
            Ok(sample_lease("agent_request_123"))
        }

        fn record_provider_operation_transition(
            &mut self,
            _request_id: &str,
            input: RecordProviderOperationTransitionRequest,
        ) -> Result<ProviderOperationEnvelope, RunnerError> {
            self.provider_operation_transitions.push(input);
            Err(RunnerError::CoreRequest(
                "fake queue has no provider-operation state machine".to_string(),
            ))
        }

        fn runtime_heartbeat_for_machine(
            &mut self,
            source_machine_id: &str,
        ) -> Result<Option<RelayHeartbeat>, RunnerError> {
            self.heartbeat_checks.push(source_machine_id.to_string());
            if self.heartbeat_checks.len() > 1
                && let Some(heartbeat) = self.next_heartbeat.take()
            {
                self.heartbeat = Some(heartbeat);
            }
            Ok(self.heartbeat.clone())
        }

        fn provision_finite_private_runtime_key(
            &mut self,
            _request_id: &str,
            input: ProvisionFinitePrivateRuntimeKeyInput,
        ) -> Result<ProvisionFinitePrivateRuntimeKeyResult, RunnerError> {
            if let Some(error) = self.provision_error.as_ref() {
                return Err(RunnerError::CoreRequest(error.clone()));
            }
            self.provisioned.push(input);
            Ok(sample_finite_private_key())
        }

        fn fail_agent_creation(
            &mut self,
            _request_id: &str,
            input: FailAgentCreationRequestInput,
        ) -> Result<AgentCreationRequest, RunnerError> {
            self.failed.push(input);
            Ok(sample_lease("agent_request_123").request)
        }
    }

    #[derive(Debug)]
    struct FakeLauncher {
        ready_error: Option<String>,
        launch_result: Result<RuntimeLaunchFacts, String>,
        restart_result: Result<(), String>,
        launch_count: usize,
        launch_options: Vec<RuntimeLaunchOptions>,
        restart_options: Vec<RuntimeRestartOptions>,
        restarted: Vec<String>,
        recovered: Vec<String>,
        upgraded: Vec<String>,
        stopped: Vec<String>,
        destroyed: Vec<String>,
        retired: Vec<String>,
        retirement_result: Option<Result<RuntimeRetirementSnapshotReceipt, String>>,
        runner_capacity: RunnerLeaseCapacity,
        runner_class: RunnerClass,
        uses_core_heartbeat: bool,
        planned_source: Option<RuntimeSourceIdentity>,
    }

    impl FakeLauncher {
        fn ready(facts: RuntimeLaunchFacts) -> Self {
            Self {
                ready_error: None,
                launch_result: Ok(facts),
                restart_result: Ok(()),
                launch_count: 0,
                launch_options: Vec::new(),
                restart_options: Vec::new(),
                restarted: Vec::new(),
                recovered: Vec::new(),
                upgraded: Vec::new(),
                stopped: Vec::new(),
                destroyed: Vec::new(),
                retired: Vec::new(),
                retirement_result: None,
                runner_capacity: RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::LocalDocker],
                    ..RunnerLeaseCapacity::default()
                },
                runner_class: RunnerClass::LocalDocker,
                uses_core_heartbeat: true,
                planned_source: Some(RuntimeSourceIdentity {
                    source_host_id: "oslo-host-1".to_string(),
                    source_machine_id: "finite-agent_123".to_string(),
                }),
            }
        }

        fn not_ready(message: &str) -> Self {
            Self {
                ready_error: Some(message.to_string()),
                launch_result: Ok(RuntimeLaunchFacts::sample()),
                restart_result: Ok(()),
                launch_count: 0,
                launch_options: Vec::new(),
                restart_options: Vec::new(),
                restarted: Vec::new(),
                recovered: Vec::new(),
                upgraded: Vec::new(),
                stopped: Vec::new(),
                destroyed: Vec::new(),
                retired: Vec::new(),
                retirement_result: None,
                runner_capacity: RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::LocalDocker],
                    ..RunnerLeaseCapacity::default()
                },
                runner_class: RunnerClass::LocalDocker,
                uses_core_heartbeat: true,
                planned_source: Some(RuntimeSourceIdentity {
                    source_host_id: "oslo-host-1".to_string(),
                    source_machine_id: "finite-agent_123".to_string(),
                }),
            }
        }

        fn launch_error(message: &str) -> Self {
            Self {
                ready_error: None,
                launch_result: Err(message.to_string()),
                restart_result: Ok(()),
                launch_count: 0,
                launch_options: Vec::new(),
                restart_options: Vec::new(),
                restarted: Vec::new(),
                recovered: Vec::new(),
                upgraded: Vec::new(),
                stopped: Vec::new(),
                destroyed: Vec::new(),
                retired: Vec::new(),
                retirement_result: None,
                runner_capacity: RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::LocalDocker],
                    ..RunnerLeaseCapacity::default()
                },
                runner_class: RunnerClass::LocalDocker,
                uses_core_heartbeat: true,
                planned_source: Some(RuntimeSourceIdentity {
                    source_host_id: "oslo-host-1".to_string(),
                    source_machine_id: "finite-agent_123".to_string(),
                }),
            }
        }

        fn with_runner_capacity(mut self, runner_capacity: RunnerLeaseCapacity) -> Self {
            self.runner_capacity = runner_capacity;
            self
        }

        fn without_core_heartbeat(mut self) -> Self {
            self.uses_core_heartbeat = false;
            self
        }

        fn without_planned_source(mut self) -> Self {
            self.planned_source = None;
            self
        }

        fn for_kata(mut self) -> Self {
            self.runner_class = RunnerClass::Kata;
            self.runner_capacity.runner_classes = vec![RunnerClass::Kata];
            self
        }

        fn with_retirement_result(
            mut self,
            result: Result<RuntimeRetirementSnapshotReceipt, String>,
        ) -> Self {
            self.retirement_result = Some(result);
            self
        }
    }

    impl RuntimeLauncher for FakeLauncher {
        fn validate_ready(&self) -> Result<(), RunnerError> {
            if let Some(message) = &self.ready_error {
                return Err(RunnerError::RuntimeLaunch(message.clone()));
            }
            Ok(())
        }

        fn runtime_capabilities(&self) -> RuntimeCapabilitiesEnvelope {
            if self.runner_class == RunnerClass::Kata {
                kata_runtime_capabilities_with_retirement(self.retirement_result.is_some())
            } else {
                state_preserving_runtime_capabilities(false)
            }
        }

        fn runner_class(&self) -> RunnerClass {
            self.runner_class
        }

        fn uses_core_runtime_heartbeat(&self) -> bool {
            self.uses_core_heartbeat
        }

        fn runner_capacity(&self) -> RunnerLeaseCapacity {
            self.runner_capacity.clone()
        }

        fn source_host_id(&self) -> Option<&str> {
            Some("oslo-host-1")
        }

        fn planned_source(&self, _lease: &AgentCreationLease) -> Option<RuntimeSourceIdentity> {
            self.planned_source.clone()
        }

        fn restart_runtime(
            &mut self,
            lease: &RuntimeControlLease,
            options: &RuntimeRestartOptions,
        ) -> Result<(), RunnerError> {
            self.restarted.push(lease.runtime.source_machine_id.clone());
            self.restart_options.push(options.clone());
            self.restart_result
                .clone()
                .map_err(RunnerError::RuntimeLaunch)
        }

        fn recover_known_good_chat_runtime(
            &mut self,
            lease: &RuntimeControlLease,
            options: &RuntimeRestartOptions,
        ) -> Result<(), RunnerError> {
            self.recovered.push(lease.runtime.source_machine_id.clone());
            self.restart_options.push(options.clone());
            self.restart_result
                .clone()
                .map_err(RunnerError::RuntimeLaunch)
        }

        fn upgrade_runtime(
            &mut self,
            lease: &RuntimeControlLease,
            options: &RuntimeRestartOptions,
        ) -> Result<RuntimeUpgradeFacts, RunnerError> {
            self.upgraded.push(lease.runtime.source_machine_id.clone());
            self.restart_options.push(options.clone());
            self.restart_result
                .clone()
                .map_err(RunnerError::RuntimeLaunch)?;
            let target = lease
                .target_runtime_artifact
                .as_ref()
                .ok_or_else(|| RunnerError::RuntimeLaunch("missing upgrade target".to_string()))?;
            Ok(RuntimeUpgradeFacts {
                runtime_artifact_id: target.id.clone(),
                state_schema_version: target.state_schema_version.clone(),
                runtime_host: "http://127.0.0.1:41002".to_string(),
                published_app_urls: vec!["http://127.0.0.1:41002/contact".to_string()],
            })
        }

        fn stop_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
            self.stopped.push(lease.runtime.source_machine_id.clone());
            self.restart_result
                .clone()
                .map_err(RunnerError::RuntimeLaunch)
        }

        fn destroy_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
            self.destroyed.push(lease.runtime.source_machine_id.clone());
            self.restart_result
                .clone()
                .map_err(RunnerError::RuntimeLaunch)
        }

        fn retire_runtime(
            &mut self,
            lease: &RuntimeControlLease,
            renew_lease: &mut dyn FnMut() -> Result<(), RunnerError>,
        ) -> Result<RuntimeRetirementSnapshotReceipt, RunnerError> {
            self.retired.push(lease.runtime.source_machine_id.clone());
            renew_lease()?;
            self.retirement_result
                .clone()
                .unwrap_or_else(|| Err("retirement not configured".to_string()))
                .map_err(RunnerError::RuntimeLaunch)
        }

        fn launch(
            &mut self,
            _lease: &AgentCreationLease,
            options: &RuntimeLaunchOptions,
        ) -> Result<RuntimeLaunchFacts, RunnerError> {
            self.launch_count += 1;
            self.launch_options.push(options.clone());
            self.launch_result
                .clone()
                .map_err(RunnerError::RuntimeLaunch)
        }
    }

    fn sample_finite_private_key() -> ProvisionFinitePrivateRuntimeKeyResult {
        ProvisionFinitePrivateRuntimeKeyResult {
            grant: finite_saas_core::FinitePrivateGrant {
                id: "fp_grant_123".to_string(),
                user_id: "user_123".to_string(),
                limit_profile_id: "finite-private-generous-v2".to_string(),
                status: finite_saas_core::FinitePrivateGrantStatus::Active,
                current_window_started_at: None,
                current_window_used_units: 0,
                burst_window_epoch: 0,
                created_at: "2026-05-25T13:00:00Z".to_string(),
                updated_at: "2026-05-25T13:00:00Z".to_string(),
            },
            api_key: FinitePrivateApiKey {
                id: "fp_key_123".to_string(),
                grant_id: "fp_grant_123".to_string(),
                project_id: Some("project_123".to_string()),
                agent_runtime_id: Some("runtime_123".to_string()),
                key_hash: "hash-fpk".to_string(),
                status: finite_saas_core::FinitePrivateApiKeyStatus::Active,
                created_at: "2026-05-25T13:00:00Z".to_string(),
                updated_at: "2026-05-25T13:00:00Z".to_string(),
            },
            raw_api_key: "fpk_live_test".to_string(),
        }
    }

    impl RuntimeLaunchFacts {
        fn sample() -> Self {
            Self {
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "finite-agent_123".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://oslo-host-1/contact".to_string()),
                runtime_relay_token_hash: "hash-runtime-token".to_string(),
                display_name: Some("Oslo Agent".to_string()),
                hostname: None,
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: RuntimeSummaryStatus::Online,
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
            }
        }
    }

    fn sample_lease(request_id: &str) -> AgentCreationLease {
        AgentCreationLease {
            project: Project {
                id: "project_123".to_string(),
                customer_org_id: "org_123".to_string(),
                owner_user_id: "user_123".to_string(),
                display_name: "Oslo Agent".to_string(),
                agent_email: None,
                import_candidate_id: None,
                hosting_tier: None,
                placement: None,
                created_at: "2026-05-25T12:00:00Z".to_string(),
                updated_at: "2026-05-25T12:00:00Z".to_string(),
            },
            request: AgentCreationRequest {
                id: request_id.to_string(),
                customer_org_id: "org_123".to_string(),
                owner_user_id: "user_123".to_string(),
                project_id: "project_123".to_string(),
                idempotency_key: "browser-submit-1".to_string(),
                display_name: "Oslo Agent".to_string(),
                runner_class: RunnerClass::LocalDocker,
                hosting_tier: None,
                placement: None,
                desired_runtime_artifact_id: None,
                runtime_spec: None,
                profile_picture_url: None,
                status: AgentCreationRequestStatus::Launching,
                requested_launch_code: Some("launch_code_record_123".to_string()),
                agent_runtime_id: None,
                runner_id: Some("runner-1".to_string()),
                lease_token: Some("lease-1".to_string()),
                lease_expires_at: Some("2026-05-25T13:10:00Z".to_string()),
                failure_message: None,
                created_at: "2026-05-25T12:00:00Z".to_string(),
                updated_at: "2026-05-25T13:00:00Z".to_string(),
            },
            provider_operation: None,
            in_flight_capacity_reservation: None,
        }
    }

    fn sample_runtime_spec(
        operation_id: &str,
        runner_class: RunnerClass,
        environment: BTreeMap<String, String>,
        secret_references: Vec<String>,
    ) -> RuntimeSpecEnvelope {
        RuntimeSpecEnvelope::V1(RuntimeSpecV1 {
            operation_id: operation_id.to_string(),
            project_id: "project_123".to_string(),
            agent_runtime_id: "runtime_123".to_string(),
            placement: RuntimePlacement {
                runner_class,
                runtime_resource_class: match runner_class {
                    RunnerClass::Phala => RuntimeResourceClass::Vcpu2Memory4Gib,
                    RunnerClass::LocalDocker
                    | RunnerClass::AppleContainer
                    | RunnerClass::Kata
                    | RunnerClass::Enclavia => RuntimeResourceClass::Vcpu4Memory8Gib,
                },
            },
            runtime_artifact_id: "artifact-v1".to_string(),
            runtime_image_digest: format!(
                "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                "a".repeat(64)
            ),
            state_schema_version: "state-v1".to_string(),
            durable_state_id: "runtime_123".to_string(),
            endpoints: RuntimeEndpointContractV1 {
                service_port: DEFAULT_DOCKER_CONTAINER_PORT,
                health_path: "/healthz".to_string(),
                contact_path: "/contact".to_string(),
            },
            boot_intent: RuntimeBootIntent::Normal,
            environment,
            secret_references,
        })
    }

    fn sample_spec_lease(request_id: &str) -> AgentCreationLease {
        let mut lease = sample_lease(request_id);
        let placement = RuntimePlacement {
            runner_class: RunnerClass::LocalDocker,
            runtime_resource_class: RuntimeResourceClass::Vcpu4Memory8Gib,
        };
        lease.project.placement = Some(placement);
        lease.request.runner_class = RunnerClass::LocalDocker;
        lease.request.placement = Some(placement);
        lease.request.agent_runtime_id = Some("runtime_123".to_string());
        lease.request.desired_runtime_artifact_id = Some("artifact-v1".to_string());
        lease.request.runtime_spec = Some(sample_runtime_spec(
            request_id,
            RunnerClass::LocalDocker,
            BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "https://api.finite.chat".to_string(),
            )]),
            vec!["FINITE_PRIVATE_API_KEY".to_string(), "FAL_KEY".to_string()],
        ));
        lease
    }

    fn sample_runtime_control_lease(request_id: &str) -> RuntimeControlLease {
        sample_runtime_control_lease_with_kind(request_id, RuntimeControlKind::Restart)
    }

    fn sample_runtime_upgrade_lease(request_id: &str) -> RuntimeControlLease {
        let mut lease =
            sample_runtime_control_lease_with_kind(request_id, RuntimeControlKind::Upgrade);
        lease.request.target_runtime_artifact_id = Some("artifact-v2".to_string());
        lease.target_runtime_artifact = Some(RuntimeArtifact {
            id: "artifact-v2".to_string(),
            kind: RuntimeArtifactKind::OciImage,
            reference: format!(
                "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
                "b".repeat(64)
            ),
            version_label: "v2".to_string(),
            source_git_sha: Some("git-v2".to_string()),
            finitec_version: None,
            hermes_source_ref: Some("0.18.2".to_string()),
            finite_platform_plugin_ref: Some("plugin-v2".to_string()),
            state_schema_version: "state-v1".to_string(),
            base_image: None,
            recover_known_good_chat: false,
            created_at: "2026-05-25T13:00:00Z".to_string(),
            promoted_at: Some("2026-05-25T13:01:00Z".to_string()),
            retired_at: None,
        });
        lease
    }

    fn sample_runtime_control_lease_with_kind(
        request_id: &str,
        kind: RuntimeControlKind,
    ) -> RuntimeControlLease {
        RuntimeControlLease {
            request: RuntimeControlRequest {
                id: request_id.to_string(),
                project_id: "project_123".to_string(),
                agent_runtime_id: "runtime_123".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                requested_by_user_id: "user_123".to_string(),
                kind,
                target_runtime_artifact_id: None,
                status: RuntimeControlRequestStatus::Running,
                runner_id: Some("runner-1".to_string()),
                lease_token: Some("lease-1".to_string()),
                lease_expires_at: Some("2026-05-25T13:10:00Z".to_string()),
                failure_message: None,
                created_at: "2026-05-25T13:00:00Z".to_string(),
                updated_at: "2026-05-25T13:00:00Z".to_string(),
                completed_at: None,
            },
            runtime: AgentRuntime {
                id: "runtime_123".to_string(),
                project_id: "project_123".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                source_import_key: "source-import-key".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                placement: None,
                provider_runtime_handle: None,
                provider_runtime_handle_history: Vec::new(),
                contact_endpoint: None,
                runtime_capabilities: None,
                host_facts: HostOwnedRuntimeFacts {
                    display_name: "Oslo Agent".to_string(),
                    hostname: None,
                    runtime_host: "oslo-host-1".to_string(),
                    runtime_status: RuntimeSummaryStatus::Online,
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                },
                created_at: "2026-05-25T12:00:00Z".to_string(),
                updated_at: "2026-05-25T13:00:00Z".to_string(),
            },
            runtime_spec: None,
            target_runtime_artifact: None,
        }
    }

    fn sample_heartbeat(machine_id: &str) -> RelayHeartbeat {
        RelayHeartbeat {
            ok: true,
            machine_id: machine_id.to_string(),
            last_seen_at: "2026-05-25T13:00:05Z".to_string(),
        }
    }

    fn assert_env(entries: &[(String, String)], key: &str, expected: &str) {
        assert_eq!(
            entries
                .iter()
                .find_map(|(entry_key, value)| (entry_key == key).then_some(value.as_str())),
            Some(expected),
            "missing or mismatched env {key}"
        );
    }

    fn os_strings_to_strings(values: &[OsString]) -> Vec<String> {
        values
            .iter()
            .map(|value| value.to_string_lossy().to_string())
            .collect()
    }

    fn read_repo_file(relative_path: &str) -> String {
        std::fs::read_to_string(repo_path(relative_path)).unwrap()
    }

    fn repo_path(relative_path: &str) -> PathBuf {
        let runner_manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = runner_manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("runner crate should live under crates/");
        repo_root.join(relative_path)
    }
}
