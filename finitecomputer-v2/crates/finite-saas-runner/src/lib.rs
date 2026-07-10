use finite_saas_core::{
    AgentCreationLease, AgentCreationRequest, CompleteAgentCreationRequestInput,
    CompleteRuntimeControlRequestInput, FailAgentCreationRequestInput,
    FailRuntimeControlRequestInput, LeaseRuntimeControlRequestInput,
    ProvisionFinitePrivateRuntimeKeyInput, ProvisionFinitePrivateRuntimeKeyResult,
    RegisterAgentCreationRuntimeInput, RelayHeartbeat, RunnerClass, RunnerLeaseCapacity,
    RuntimeArtifact, RuntimeArtifactKind, RuntimeControlKind, RuntimeControlLease,
    RuntimeControlRequest, RuntimeSummaryStatus,
    runtime_relay_token_hash as hash_runtime_relay_token,
};
#[cfg(test)]
use finite_saas_core::FinitePrivateApiKey;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
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

pub use apple_container::{AppleContainerConfig, AppleContainerLaunchPlan, AppleContainerLauncher};
pub use kata::{KataConfig, KataLaunchPlan, KataLauncher};

const DEFAULT_RUNTIME_READY_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_RUNTIME_READY_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_LAUNCH_TIMEOUT: Duration = Duration::from_secs(300);
// The deployed limiter domain keeps the historical kimi-k2-6 name but now
// serves glm-5-2 (see docs/service-dependencies.md, Finite Private Routing
// Debt). Do not rename the URL as a cosmetic change.
pub const DEFAULT_FINITE_PRIVATE_BASE_URL: &str =
    "https://kimi-k2-6.finite.containers.tinfoil.dev/v1";
pub const DEFAULT_FINITE_PRIVATE_MODEL: &str = "glm-5-2";
pub const DEFAULT_FINITECHAT_SERVER_URL: &str = "https://chat.finite.computer";
pub const DEFAULT_FINITE_AGENT_PICTURE_URL: &str =
    "https://avatars.githubusercontent.com/u/274919006?v=4";
const FINITE_PRIVATE_PROFILE_ID: &str = "finite-private";
const DEFAULT_DOCKER_CONTAINER_PORT: u16 = 8080;
const MAX_RUNTIME_ENVIRONMENT_ENTRIES: usize = 64;
const MAX_RUNTIME_ENVIRONMENT_KEY_BYTES: usize = 128;
const MAX_RUNTIME_ENVIRONMENT_VALUE_BYTES: usize = 4 * 1024;
const MAX_RUNTIME_ENVIRONMENT_TOTAL_BYTES: usize = 32 * 1024;
const MAX_RUNTIME_SECRET_ENVIRONMENT_ENTRIES: usize = 64;
const MAX_RUNTIME_SECRET_ENVIRONMENT_VALUE_BYTES: usize = 16 * 1024;
const MAX_RUNTIME_SECRET_ENVIRONMENT_TOTAL_BYTES: usize = 128 * 1024;

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
    #[error("Phala CLI binary is required")]
    MissingPhalaBinary,
    #[error("Phala API key is required")]
    MissingPhalaApiKey,
    #[error("Enclavia CLI binary is required")]
    MissingEnclaviaBinary,
    #[error("Enclavia enclave id is required")]
    MissingEnclaviaEnclaveId,
    #[error("Finite Chat server URL is required")]
    MissingFinitechatServerUrl,
    #[error("Docker host port must be between 1 and 65535")]
    InvalidDockerHostPort,
    #[error("Apple Container host port must be between 1 and 65535")]
    InvalidAppleContainerHostPort,
    #[error("runtime artifact reference is required")]
    MissingRuntimeArtifactReference,
    #[error("invalid opaque runtime environment: {0}")]
    InvalidRuntimeEnvironment(String),
    #[error("Phala instance type is required")]
    MissingPhalaInstanceType,
    #[error("Phala disk size is required")]
    MissingPhalaDiskSize,
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
        })
    }

    pub fn with_runtime_ready_polling(mut self, timeout: Duration, interval: Duration) -> Self {
        self.runtime_ready_timeout = timeout;
        self.runtime_ready_interval = interval;
        self
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
        let mut runner_capacity = self.launcher.runner_capacity();
        if runner_capacity.runner_classes.is_empty() {
            runner_capacity
                .runner_classes
                .push(self.launcher.runner_class());
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
        match self.launcher.launch(&lease, &launch_options) {
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
                        runtime_relay_token_hash: facts.runtime_relay_token_hash.clone(),
                        display_name: facts.display_name.clone(),
                        hostname: facts.hostname.clone(),
                        runtime_host: facts.runtime_host.clone(),
                        runtime_status: Some(RuntimeSummaryStatus::Unknown),
                        active_inference_profile: facts.active_inference_profile.clone(),
                        hermes_available: facts.hermes_available,
                        published_app_urls: facts.published_app_urls.clone(),
                        now: None,
                    },
                );
                let launch_result = match launch_result {
                    Ok(_) => match self.wait_for_launch_readiness(&facts.source_machine_id) {
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
                                display_name: facts.display_name.clone(),
                                hostname: facts.hostname.clone(),
                                runtime_host: facts.runtime_host.clone(),
                                runtime_status: Some(RuntimeSummaryStatus::Online),
                                active_inference_profile: facts.active_inference_profile.clone(),
                                hermes_available: facts.hermes_available,
                                published_app_urls: facts.published_app_urls.clone(),
                                now: None,
                            },
                        ),
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
        let restart_options = RuntimeRestartOptions::new(self.runtime_environment.clone())?;
        let operation_result: Result<Option<RuntimeUpgradeFacts>, RunnerError> = match kind {
            RuntimeControlKind::Restart => self
                .launcher
                .restart_runtime(&lease, &restart_options)
                .map(|()| None),
            RuntimeControlKind::RecoverKnownGoodChatRuntime => self
                .launcher
                .recover_known_good_chat_runtime(&lease, &restart_options)
                .map(|()| None),
            RuntimeControlKind::Upgrade => self
                .launcher
                .upgrade_runtime(&lease, &restart_options)
                .map(Some),
            RuntimeControlKind::Stop => self.launcher.stop_runtime(&lease).map(|()| None),
            RuntimeControlKind::Destroy => self.launcher.destroy_runtime(&lease).map(|()| None),
        };

        match operation_result {
            Ok(upgrade_facts) => match self.wait_for_runtime_control_readiness(
                kind,
                &source_machine_id,
                previous_heartbeat.as_deref(),
            ) {
                Ok(()) => {
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
                            runtime_host: upgrade_facts
                                .as_ref()
                                .map(|facts| facts.runtime_host.clone()),
                            published_app_urls: upgrade_facts
                                .as_ref()
                                .map(|facts| facts.published_app_urls.clone()),
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
                    Ok(runtime_control_failed_outcome(
                        kind,
                        request_id,
                        failure_message,
                    ))
                }
            },
            Err(error) => {
                let failure_message = error.to_string();
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
                Ok(runtime_control_failed_outcome(
                    kind,
                    request_id,
                    failure_message,
                ))
            }
        }
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
        let mut options = RuntimeLaunchOptions {
            profile_picture_url: lease.request.profile_picture_url.clone(),
            environment: self.runtime_environment.clone(),
            secret_environment: self.runtime_secret_environment.clone(),
            ..RuntimeLaunchOptions::default()
        };
        let Some(defaults) = self.default_finite_private.clone() else {
            return Ok(options);
        };
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
            });
            return Ok(options);
        }
        let source = self.launcher.planned_source(lease);
        let key = self.queue.provision_finite_private_runtime_key(
            &lease.request.id,
            ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: self.runner_id.clone(),
                lease_token: lease_token.to_string(),
                source_host_id: source.as_ref().map(|value| value.source_host_id.clone()),
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

pub trait RuntimeLauncher {
    fn validate_ready(&self) -> Result<(), RunnerError>;
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
    fn launch(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError>;
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

    fn launch(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        (**self).launch(lease, options)
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
}

impl Default for FinitePrivateRuntimeDefaults {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_FINITE_PRIVATE_BASE_URL.to_string(),
            model: DEFAULT_FINITE_PRIVATE_MODEL.to_string(),
            api_key_override: None,
        }
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

/// Provider-neutral desired environment carried through a state-preserving
/// Runtime restart. The map is intentionally limited to the same bounded,
/// non-secret opaque values accepted at launch; inference credentials and
/// Runtime-contract variables remain owned by the existing Runtime.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RuntimeRestartOptions {
    environment: BTreeMap<String, String>,
}

impl RuntimeRestartOptions {
    pub fn new(environment: BTreeMap<String, String>) -> Result<Self, RunnerError> {
        validate_runtime_environment(&environment)?;
        Ok(Self { environment })
    }

    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct FinitePrivateLaunchKey {
    pub api_key_id: String,
    pub raw_api_key: String,
    pub base_url: String,
    pub model: String,
    pub revoke_on_launch_failure: bool,
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
            .finish()
    }
}

/// Reconcile only the explicitly desired non-secret opaque keys. Existing
/// Runtime-contract values, provider settings, and credentials are retained
/// byte-for-byte so compute replacement does not silently rotate or erase
/// them.
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
    existing
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
pub struct PhalaConfig {
    pub phala_bin: PathBuf,
    pub api_key: String,
    pub source_host_id: String,
    pub image: String,
    pub runtime_artifact_id: Option<String>,
    pub runtime_artifact_kind: Option<RuntimeArtifactKind>,
    pub runtime_state_schema_version: Option<String>,
    pub work_root: PathBuf,
    pub finitechat_server_url: String,
    pub agent_picture_url: String,
    pub instance_type: String,
    pub disk_size: String,
    pub region: Option<String>,
    pub kms: String,
    pub public_logs: bool,
    pub public_sysinfo: bool,
    pub max_cvm_count: Option<u32>,
    pub drain_new_leases: bool,
    pub available_memory_bytes: Option<u64>,
    pub command_timeout: Duration,
    pub launch_timeout: Duration,
    pub readiness_timeout: Duration,
    pub readiness_interval: Duration,
}

impl PhalaConfig {
    pub fn validate(&self) -> Result<(), RunnerError> {
        if self.phala_bin.as_os_str().is_empty() {
            return Err(RunnerError::MissingPhalaBinary);
        }
        if self.api_key.trim().is_empty() {
            return Err(RunnerError::MissingPhalaApiKey);
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
        if self.instance_type.trim().is_empty() {
            return Err(RunnerError::MissingPhalaInstanceType);
        }
        if self.disk_size.trim().is_empty() {
            return Err(RunnerError::MissingPhalaDiskSize);
        }
        if let Some(kind) = self.runtime_artifact_kind
            && kind != RuntimeArtifactKind::OciImage
        {
            return Err(RunnerError::RuntimeLaunch(format!(
                "Phala launcher requires an OCI image artifact, got {}",
                kind.as_str()
            )));
        }
        Ok(())
    }
}

impl Default for PhalaConfig {
    fn default() -> Self {
        Self {
            phala_bin: PathBuf::from("phala"),
            api_key: String::new(),
            source_host_id: String::new(),
            image: String::new(),
            runtime_artifact_id: None,
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: None,
            work_root: PathBuf::new(),
            finitechat_server_url: DEFAULT_FINITECHAT_SERVER_URL.to_string(),
            agent_picture_url: DEFAULT_FINITE_AGENT_PICTURE_URL.to_string(),
            instance_type: "tdx.small".to_string(),
            disk_size: "40G".to_string(),
            region: None,
            kms: "phala".to_string(),
            public_logs: false,
            public_sysinfo: false,
            max_cvm_count: None,
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
pub struct PhalaLauncher {
    config: PhalaConfig,
}

impl PhalaLauncher {
    pub fn new(config: PhalaConfig) -> Self {
        Self { config }
    }

    pub fn plan_launch(&self, lease: &AgentCreationLease) -> PhalaLaunchPlan {
        phala_launch_plan(&self.config, lease)
    }

    fn run_command_capture(
        &self,
        args: Vec<OsString>,
        timeout: Duration,
    ) -> Result<String, RunnerError> {
        let child = Command::new(&self.config.phala_bin)
            .args(&args)
            .env("PHALA_CLOUD_API_KEY", self.config.api_key.trim())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| RunnerError::CommandExecution {
                program: self.config.phala_bin.display().to_string(),
                message: error.to_string(),
            })?;
        let output = wait_with_captured_output(child, &self.config.phala_bin, timeout)?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if output.status.success() {
            return Ok(stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(RunnerError::CommandExecution {
            program: self.config.phala_bin.display().to_string(),
            message: format!(
                "exit status {} stdout={stdout} stderr={stderr}",
                output.status
            ),
        })
    }

    fn run_command(&self, args: Vec<OsString>, timeout: Duration) -> Result<(), RunnerError> {
        let _ = self.run_command_capture(args, timeout)?;
        Ok(())
    }

    fn wait_for_runtime_http(&self, endpoint: &PhalaAppEndpoint) -> Result<(), RunnerError> {
        wait_for_http_json_ready(
            &endpoint.health_url,
            "Phala runtime /healthz",
            self.config.readiness_timeout,
            self.config.readiness_interval,
        )
    }

    fn wait_for_endpoint(&self, app_name: &str) -> Result<PhalaAppEndpoint, RunnerError> {
        let started = Instant::now();
        loop {
            match self.lookup_endpoint(app_name) {
                Ok(endpoint) => return Ok(endpoint),
                Err(error) => {
                    if started.elapsed() >= self.config.readiness_timeout {
                        return Err(error);
                    }
                }
            }
            thread::sleep(self.config.readiness_interval);
        }
    }

    fn lookup_endpoint(&self, app_name: &str) -> Result<PhalaAppEndpoint, RunnerError> {
        let apps_output = self.run_command_capture(
            vec![
                OsString::from("apps"),
                OsString::from("--search"),
                OsString::from(app_name),
                OsString::from("--json"),
            ],
            self.config.command_timeout,
        )?;
        match phala_app_from_apps_json(&apps_output, app_name, DEFAULT_DOCKER_CONTAINER_PORT)? {
            // Older `phala` versions embedded the CVM node (teepod) inside the
            // `apps` listing, so the endpoint is fully known after one call.
            PhalaAppLookup::Endpoint(endpoint) => Ok(endpoint),
            // The current `phala` CLI (>=1.1) only reports {appId, cvmName,
            // status, uptime} per app, so the node needed to build the public
            // hostname must be resolved with a follow-up `cvms get` call.
            PhalaAppLookup::NeedsCvm(app_id) => {
                let cvm_output = self.run_command_capture(
                    vec![
                        OsString::from("cvms"),
                        OsString::from("get"),
                        OsString::from(&app_id),
                        OsString::from("--json"),
                    ],
                    self.config.command_timeout,
                )?;
                phala_endpoint_from_cvm_json(
                    &cvm_output,
                    &app_id,
                    app_name,
                    DEFAULT_DOCKER_CONTAINER_PORT,
                )
            }
        }
    }
}

impl RuntimeLauncher for PhalaLauncher {
    fn runner_class(&self) -> RunnerClass {
        RunnerClass::Phala
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
            max_sandbox_count: self.config.max_cvm_count,
            active_sandbox_count: None,
            available_memory_bytes: self.config.available_memory_bytes,
        }
    }

    fn source_host_id(&self) -> Option<&str> {
        Some(&self.config.source_host_id)
    }

    fn planned_source(&self, lease: &AgentCreationLease) -> Option<RuntimeSourceIdentity> {
        let plan = self.plan_launch(lease);
        Some(RuntimeSourceIdentity {
            source_host_id: self.config.source_host_id.clone(),
            source_machine_id: plan.cvm_name,
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
            vec![
                OsString::from("cvms"),
                OsString::from("restart"),
                OsString::from(&lease.request.source_machine_id),
                OsString::from("--json"),
            ],
            self.config.command_timeout,
        )?;
        let endpoint = self.wait_for_endpoint(&lease.request.source_machine_id)?;
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
            vec![
                OsString::from("cvms"),
                OsString::from("stop"),
                OsString::from(&lease.request.source_machine_id),
                OsString::from("--json"),
            ],
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
            vec![
                OsString::from("cvms"),
                OsString::from("delete"),
                OsString::from(&lease.request.source_machine_id),
                OsString::from("--yes"),
                OsString::from("--json"),
            ],
            self.config.command_timeout,
        )
    }

    fn launch(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        self.validate_ready()?;
        let plan = self.plan_launch(lease);
        std::fs::create_dir_all(&plan.work_dir)
            .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;
        write_secret_file(
            &plan.compose_path,
            phala_compose(&self.config, &plan, lease, options).as_bytes(),
        )
        .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;
        write_secret_file(
            &plan.env_path,
            phala_env_file(options)
                .ok_or_else(|| {
                    RunnerError::RuntimeLaunch(
                        "Phala runtime launch requires a Finite Private key".to_string(),
                    )
                })?
                .as_bytes(),
        )
        .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;

        let mut args = vec![
            OsString::from("deploy"),
            OsString::from("--name"),
            OsString::from(&plan.cvm_name),
            OsString::from("--compose"),
            OsString::from(&plan.compose_path),
            OsString::from("--env"),
            OsString::from(&plan.env_path),
            OsString::from("--instance-type"),
            OsString::from(self.config.instance_type.trim()),
            OsString::from("--disk-size"),
            OsString::from(self.config.disk_size.trim()),
            OsString::from("--kms"),
            OsString::from(self.config.kms.trim()),
        ];
        if let Some(region) = self
            .config
            .region
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            args.push(OsString::from("--region"));
            args.push(OsString::from(region));
        }
        args.push(if self.config.public_logs {
            OsString::from("--public-logs")
        } else {
            OsString::from("--no-public-logs")
        });
        args.push(if self.config.public_sysinfo {
            OsString::from("--public-sysinfo")
        } else {
            OsString::from("--no-public-sysinfo")
        });
        args.push(OsString::from("--wait"));
        args.push(OsString::from("--json"));

        let _ = self.run_command_capture(args, self.config.launch_timeout)?;
        let endpoint = self.wait_for_endpoint(&plan.cvm_name)?;
        self.wait_for_runtime_http(&endpoint)?;

        let runtime_bootstrap_token = random_runtime_bootstrap_token();
        let runtime_relay_token_hash = hash_runtime_relay_token(&runtime_bootstrap_token)
            .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;

        Ok(RuntimeLaunchFacts {
            source_host_id: self.config.source_host_id.clone(),
            source_machine_id: plan.cvm_name,
            runtime_artifact_id: self.config.runtime_artifact_id.clone(),
            state_schema_version: self.config.runtime_state_schema_version.clone(),
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

    fn cleanup_failed_launch(&mut self, facts: &RuntimeLaunchFacts) -> Result<(), RunnerError> {
        self.run_command(
            vec![
                OsString::from("cvms"),
                OsString::from("delete"),
                OsString::from(&facts.source_machine_id),
                OsString::from("--yes"),
                OsString::from("--json"),
            ],
            self.config.command_timeout,
        )
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhalaLaunchPlan {
    pub cvm_name: String,
    pub work_dir: PathBuf,
    pub compose_path: PathBuf,
    pub env_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PhalaAppEndpoint {
    app_id: String,
    teepod_name: String,
    hostname: String,
    public_base_url: String,
    health_url: String,
    contact_url: String,
}

fn phala_launch_plan(config: &PhalaConfig, lease: &AgentCreationLease) -> PhalaLaunchPlan {
    let cvm_name = phala_cvm_name_for_request_id(&lease.request.id);
    let work_dir = config.work_root.join("phala").join(&cvm_name);
    PhalaLaunchPlan {
        compose_path: work_dir.join("docker-compose.yml"),
        env_path: work_dir.join("runtime.env"),
        work_dir,
        cvm_name,
    }
}

fn phala_compose(
    config: &PhalaConfig,
    plan: &PhalaLaunchPlan,
    lease: &AgentCreationLease,
    options: &RuntimeLaunchOptions,
) -> String {
    let mut env = docker_equivalent_runtime_env(
        DockerEquivalentRuntimeEnv {
            finitechat_server_url: &config.finitechat_server_url,
            agent_picture_url: &config.agent_picture_url,
            agent_http_port: DEFAULT_DOCKER_CONTAINER_PORT,
            agent_device_id: &plan.cvm_name,
            agent_home: "/data/agent",
            hermes_home: "/data/agent/hermes-home",
            workspace: "/data/workspace",
        },
        lease,
        options,
    );
    for (key, value) in &mut env {
        if matches!(key.as_str(), "FINITE_PRIVATE_API_KEY" | "OPENAI_API_KEY") {
            *value = "${FINITE_PRIVATE_API_KEY:?FINITE_PRIVATE_API_KEY is required}".to_string();
        } else if options.secret_environment.contains_key(key) {
            *value = format!("${{{key}:?{key} is required}}");
        }
    }

    let mut rendered = String::new();
    rendered.push_str("services:\n");
    rendered.push_str("  agent:\n");
    rendered.push_str("    image: ");
    rendered.push_str(&yaml_quote(config.image.trim()));
    rendered.push('\n');
    rendered.push_str("    container_name: ");
    rendered.push_str(&yaml_quote(&plan.cvm_name));
    rendered.push('\n');
    rendered.push_str("    restart: unless-stopped\n");
    rendered.push_str("    ports:\n");
    rendered.push_str("      - \"8080:8080\"\n");
    rendered.push_str("    volumes:\n");
    rendered.push_str("      - agent_state:/data\n");
    rendered.push_str("    environment:\n");
    for (key, value) in env {
        rendered.push_str("      ");
        rendered.push_str(&key);
        rendered.push_str(": ");
        rendered.push_str(&yaml_quote(&value));
        rendered.push('\n');
    }
    rendered.push_str("\nvolumes:\n");
    rendered.push_str("  agent_state:\n");
    rendered
}

fn phala_env_file(options: &RuntimeLaunchOptions) -> Option<String> {
    let finite_private = options.finite_private.as_ref()?;
    let mut rendered = String::new();
    rendered.push_str("# Generated by finite-saas-runner. Do not commit.\n");
    rendered.push_str("# Phala CLI seals these values before upload.\n");
    rendered.push_str("FINITE_PRIVATE_API_KEY=");
    rendered.push_str(&dotenv_quote(&finite_private.raw_api_key));
    rendered.push('\n');
    for (key, value) in &options.secret_environment {
        rendered.push_str(key);
        rendered.push('=');
        rendered.push_str(&dotenv_quote(value));
        rendered.push('\n');
    }
    Some(rendered)
}

/// Outcome of parsing a `phala apps --search <name> --json` response.
///
/// The Phala CLI's output schema changed across versions:
///
/// * Older builds returned a `dstack_apps` array whose entries embedded the
///   running CVM (via `current_cvm`/`cvms`), including the node/`teepod_name`.
///   For those we can build the full public endpoint from the single call.
/// * The current CLI (verified against `phala` 1.1.x) returns
///   `{"success":true,...,"items":[{"appId":..,"cvmName":..,"status":..,
///   "uptime":..}]}`. Those entries carry no node information, so we can only
///   learn the `appId` here and must resolve the node with a `cvms get` call.
enum PhalaAppLookup {
    Endpoint(PhalaAppEndpoint),
    NeedsCvm(String),
}

fn phala_app_from_apps_json(
    output: &str,
    app_name: &str,
    port: u16,
) -> Result<PhalaAppLookup, RunnerError> {
    let value: serde_json::Value = serde_json::from_str(output)
        .map_err(|error| RunnerError::RuntimeLaunch(format!("invalid Phala apps JSON: {error}")))?;
    // Accept every app-list container shape the CLI has used: `items` (>=1.1),
    // `dstack_apps` (older), or a bare top-level array.
    let apps = value
        .get("items")
        .and_then(serde_json::Value::as_array)
        .or_else(|| {
            value
                .get("dstack_apps")
                .and_then(serde_json::Value::as_array)
        })
        .or_else(|| value.as_array())
        .ok_or_else(|| {
            RunnerError::RuntimeLaunch("Phala apps JSON did not contain an app list".to_string())
        })?;
    // The CLI already filters to the searched app, so match by name against
    // both the new `cvmName` and the old `name`, falling back to the first row.
    let app = apps
        .iter()
        .find(|app| {
            let matches =
                |key: &str| app.get(key).and_then(serde_json::Value::as_str) == Some(app_name);
            matches("cvmName") || matches("name")
        })
        .or_else(|| apps.first())
        .ok_or_else(|| RunnerError::RuntimeLaunch(format!("Phala app {app_name} was not found")))?;
    // `appId` (>=1.1) or `app_id` (older).
    let app_id = app
        .get("appId")
        .or_else(|| app.get("app_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            RunnerError::RuntimeLaunch(format!("Phala app {app_name} did not include app_id"))
        })?;
    // If the listing still embeds the CVM node, finish in one call. Otherwise
    // defer to `cvms get <app_id>` to learn the node.
    match phala_teepod_name_from_value(app) {
        Some(teepod_name) => Ok(PhalaAppLookup::Endpoint(phala_app_endpoint(
            app_id,
            &teepod_name,
            port,
        ))),
        None => Ok(PhalaAppLookup::NeedsCvm(app_id.to_string())),
    }
}

/// Resolve the public endpoint from a `phala cvms get <app_id> --json` response.
///
/// The CLI wraps the CVM object as `{"success":true,"data":{..}}`; we also
/// accept a bare CVM object. The node lives under `teepod.name` (verified
/// against `phala` 1.1.x), with `current_cvm.teepod_name`/`teepod_name`
/// accepted for compatibility. `app_id` from the earlier `apps` call is used as
/// a fallback when the CVM payload omits it.
fn phala_endpoint_from_cvm_json(
    output: &str,
    app_id: &str,
    app_name: &str,
    port: u16,
) -> Result<PhalaAppEndpoint, RunnerError> {
    let value: serde_json::Value = serde_json::from_str(output)
        .map_err(|error| RunnerError::RuntimeLaunch(format!("invalid Phala cvm JSON: {error}")))?;
    let cvm = value
        .get("data")
        .filter(|data| !data.is_null())
        .unwrap_or(&value);
    let resolved_app_id = cvm
        .get("app_id")
        .or_else(|| cvm.get("appId"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(app_id);
    // Preferred: the CLI hands us the full public URL in `endpoints[].app`
    // (verified against `phala` 1.1.x `cvms get`), so use it verbatim rather
    // than reconstructing the hostname. Fall back to building it from the node
    // name only when no endpoint URL is present.
    if let Some(endpoint) = phala_endpoint_from_endpoints(cvm, resolved_app_id, port) {
        return Ok(endpoint);
    }
    let teepod_name = phala_teepod_name_from_value(cvm).ok_or_else(|| {
        RunnerError::RuntimeLaunch(format!(
            "Phala cvm for {app_name} did not include an app endpoint or node name"
        ))
    })?;
    Ok(phala_app_endpoint(resolved_app_id, &teepod_name, port))
}

/// Build the endpoint from the CLI-provided `endpoints[].app` URL. Picks the
/// entry whose URL targets our port, else the first non-empty `app` URL.
fn phala_endpoint_from_endpoints(
    cvm: &serde_json::Value,
    app_id: &str,
    port: u16,
) -> Option<PhalaAppEndpoint> {
    let endpoints = cvm.get("endpoints")?.as_array()?;
    let app_urls = || {
        endpoints
            .iter()
            .filter_map(|entry| entry.get("app").and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|url| !url.is_empty())
    };
    let port_marker = format!("-{port}.");
    let url = app_urls()
        .find(|url| url.contains(&port_marker))
        .or_else(|| app_urls().next())?;
    let public_base_url = url.trim_end_matches('/').to_string();
    let hostname = public_base_url
        .strip_prefix("https://")
        .or_else(|| public_base_url.strip_prefix("http://"))
        .unwrap_or(&public_base_url)
        .to_string();
    let teepod_name = phala_teepod_name_from_value(cvm).unwrap_or_default();
    Some(PhalaAppEndpoint {
        app_id: app_id.to_string(),
        teepod_name,
        health_url: format!("{public_base_url}/healthz"),
        contact_url: format!("{public_base_url}/contact"),
        hostname,
        public_base_url,
    })
}

/// Extract the node/`teepod` name from a Phala app or CVM object across the
/// several field layouts the CLI has emitted.
fn phala_teepod_name_from_value(value: &serde_json::Value) -> Option<String> {
    let clean = |candidate: Option<&serde_json::Value>| {
        candidate
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    // Current CLI (`phala` 1.1.x `cvms get`): `node_info: { name }`.
    if let Some(name) = clean(value.get("node_info").and_then(|node| node.get("name"))) {
        return Some(name);
    }
    // Current CLI: `teepod: { name }`.
    if let Some(name) = clean(value.get("teepod").and_then(|teepod| teepod.get("name"))) {
        return Some(name);
    }
    // Older listings: `current_cvm.teepod_name` or `cvms[0].teepod_name`.
    let current_cvm = value
        .get("current_cvm")
        .filter(|value| !value.is_null())
        .or_else(|| {
            value
                .get("cvms")
                .and_then(serde_json::Value::as_array)
                .and_then(|items| items.first())
        });
    if let Some(name) = clean(current_cvm.and_then(|cvm| cvm.get("teepod_name"))) {
        return Some(name);
    }
    // Flat `teepod_name` fallback.
    clean(value.get("teepod_name"))
}

fn phala_app_endpoint(app_id: &str, teepod_name: &str, port: u16) -> PhalaAppEndpoint {
    let teepod_name = teepod_name.trim();
    let hostname = format!("{app_id}-{port}.dstack-pha-{teepod_name}.phala.network");
    let public_base_url = format!("https://{hostname}");
    PhalaAppEndpoint {
        app_id: app_id.to_string(),
        teepod_name: teepod_name.to_string(),
        hostname,
        health_url: format!("{public_base_url}/healthz"),
        contact_url: format!("{public_base_url}/contact"),
        public_base_url,
    }
}

fn phala_cvm_name_for_request_id(request_id: &str) -> String {
    let suffix = request_id
        .strip_prefix("agent_request_")
        .unwrap_or(request_id);
    sanitize_phala_name(&format!("finite-agent-{suffix}"))
}

fn sanitize_phala_name(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push('-');
        }
    }
    while result.contains("--") {
        result = result.replace("--", "-");
    }
    if result.len() > 63 {
        result.truncate(63);
    }
    result.trim_matches('-').to_string()
}

fn yaml_quote(value: &str) -> String {
    let mut quoted = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

fn dotenv_quote(value: &str) -> String {
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
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

fn random_runtime_bootstrap_token() -> String {
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
    fn run_once_reports_capacity_without_agent_lease_when_runner_is_draining() {
        let capacity = RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::LocalDocker],
            draining: true,
            max_sandbox_count: Some(4),
            active_sandbox_count: Some(2),
            available_memory_bytes: Some(8 * 1024 * 1024 * 1024),
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
    fn run_once_recovers_known_good_chat_runtime_control_request() {
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
            RunOnceOutcome::RuntimeRecoveredKnownGoodChat {
                request_id: runtime_control.request.id.clone(),
                runtime_id: runtime_control.runtime.id.clone(),
            }
        );
        assert!(runner.launcher.restarted.is_empty());
        assert_eq!(
            runner.launcher.recovered,
            vec!["oslo-agent-001".to_string()]
        );
        assert!(runner.queue.leases.is_empty());
        assert_eq!(runner.queue.completed_runtime_control.len(), 1);
        assert!(runner.queue.failed_runtime_control.is_empty());
    }

    #[test]
    fn run_once_upgrades_only_the_core_bound_artifact_and_reports_actual_facts() {
        let runtime_control = sample_runtime_upgrade_lease("runtime_ctl_upgrade");
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_runtime_control_lease(runtime_control.clone()),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()).without_core_heartbeat(),
            FixedLeaseTokens::new(["lease-upgrade"]),
            "runner-1",
            300,
        )
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
        assert_eq!(runner.queue.completed_runtime_control.len(), 1);
        let completion = &runner.queue.completed_runtime_control[0];
        assert_eq!(
            completion.runtime_artifact_id.as_deref(),
            Some("artifact-v2")
        );
        assert_eq!(completion.state_schema_version.as_deref(), Some("state-v1"));
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
        assert!(runner.queue.failed_runtime_control.is_empty());
    }

    #[test]
    fn run_once_destroys_runtime_control_request_without_waiting_for_heartbeat() {
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
            RunOnceOutcome::RuntimeDestroyed {
                request_id: runtime_control.request.id.clone(),
                runtime_id: runtime_control.runtime.id.clone(),
            }
        );
        assert_eq!(
            runner.launcher.destroyed,
            vec!["oslo-agent-001".to_string()]
        );
        assert!(runner.queue.heartbeat_checks.is_empty());
        assert_eq!(runner.queue.completed_runtime_control.len(), 1);
        assert!(runner.queue.failed_runtime_control.is_empty());
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
        let mut runner = AgentCreationRunner::new(
            FakeQueue::with_lease(lease.clone()),
            FakeLauncher::ready(RuntimeLaunchFacts::sample()),
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
        assert!(runner.queue.failed.is_empty());
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
        .with_default_finite_private_inference(FinitePrivateRuntimeDefaults::default());

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
            ..FinitePrivateRuntimeDefaults::default()
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
        .with_default_finite_private_inference(FinitePrivateRuntimeDefaults::default());

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
        .with_default_finite_private_inference(FinitePrivateRuntimeDefaults::default());

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
            ..FinitePrivateRuntimeDefaults::default()
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
        assert!(dockerfile.contains("ENTRYPOINT [\"/opt/agent-entrypoint.sh\"]"));
        assert!(!dockerfile.contains("finitechat-entrypoint.sh"));
        assert!(!dockerfile.contains("/finite-state"));
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
    fn phala_plan_renders_docker_equivalent_compose_without_raw_secrets() {
        let config = PhalaConfig {
            phala_bin: PathBuf::from("/usr/local/bin/phala"),
            api_key: "phala_test_api_key".to_string(),
            source_host_id: "phala-prod".to_string(),
            image: "ghcr.io/finitecomputer/finite-agent-runtime:canary@sha256:abc123".to_string(),
            runtime_artifact_id: Some("artifact-v1".to_string()),
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: Some("state-v1".to_string()),
            work_root: PathBuf::from("/var/lib/finite/runner"),
            ..PhalaConfig::default()
        };
        config.validate().unwrap();
        let lease = sample_lease("agent_request_ABC.123");
        let plan = phala_launch_plan(&config, &lease);
        let options = RuntimeLaunchOptions {
            finite_private: Some(FinitePrivateLaunchKey {
                api_key_id: "fp_key_123".to_string(),
                raw_api_key: "fpk_live_test".to_string(),
                base_url: DEFAULT_FINITE_PRIVATE_BASE_URL.to_string(),
                model: DEFAULT_FINITE_PRIVATE_MODEL.to_string(),
                revoke_on_launch_failure: true,
            }),
            profile_picture_url: None,
            environment: BTreeMap::new(),
            secret_environment: BTreeMap::from([(
                "FAL_KEY".to_string(),
                "fal_phala_secret".to_string(),
            )]),
        };

        assert_eq!(plan.cvm_name, "finite-agent-abc-123");
        assert_eq!(
            plan.compose_path,
            PathBuf::from("/var/lib/finite/runner/phala/finite-agent-abc-123/docker-compose.yml")
        );

        let compose = phala_compose(&config, &plan, &lease, &options);
        assert!(
            compose.contains(
                "image: 'ghcr.io/finitecomputer/finite-agent-runtime:canary@sha256:abc123'"
            )
        );
        assert!(compose.contains("container_name: 'finite-agent-abc-123'"));
        assert!(compose.contains("- agent_state:/data"));
        assert!(compose.contains("FINITECHAT_HOME: '/data/agent'"));
        assert!(compose.contains("FINITE_HOME: '/data/agent'"));
        assert!(compose.contains("HERMES_HOME: '/data/agent/hermes-home'"));
        assert!(compose.contains("FINITECHAT_WORKSPACE: '/data/workspace'"));
        assert!(compose.contains("FINITECHAT_SERVER_URL: 'https://chat.finite.computer'"));
        assert!(compose.contains(
            "FINITE_PRIVATE_API_KEY: '${FINITE_PRIVATE_API_KEY:?FINITE_PRIVATE_API_KEY is required}'"
        ));
        assert!(compose.contains(
            "OPENAI_API_KEY: '${FINITE_PRIVATE_API_KEY:?FINITE_PRIVATE_API_KEY is required}'"
        ));
        assert!(compose.contains("FAL_KEY: '${FAL_KEY:?FAL_KEY is required}'"));
        assert!(!compose.contains("fpk_live_test"));
        assert!(!compose.contains("fal_phala_secret"));
        assert!(!compose.contains("phala_test_api_key"));

        let env_file = phala_env_file(&options).unwrap();
        assert!(env_file.contains("FINITE_PRIVATE_API_KEY=\"fpk_live_test\""));
        assert!(env_file.contains("FAL_KEY=\"fal_phala_secret\""));
        assert!(!env_file.contains("phala_test_api_key"));
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

    #[test]
    fn phala_apps_items_shape_resolves_app_id_for_cvm_lookup() {
        // Exact shape emitted by the installed `phala` CLI (>=1.1) for
        // `apps --search <name> --json`.
        let payload = r#"{
            "success": true,
            "page": 1,
            "pageSize": 50,
            "total": 1,
            "totalPages": 1,
            "items": [
                {
                    "appId": "b86bdd97e9575f178ec5ccfe6fab6e138781ea1c",
                    "cvmName": "finite-agent-abc-123",
                    "status": "running",
                    "uptime": "3m 17s"
                }
            ]
        }"#;

        match phala_app_from_apps_json(payload, "finite-agent-abc-123", 8080).unwrap() {
            PhalaAppLookup::NeedsCvm(app_id) => {
                assert_eq!(app_id, "b86bdd97e9575f178ec5ccfe6fab6e138781ea1c");
            }
            PhalaAppLookup::Endpoint(endpoint) => {
                panic!("items shape has no node; expected NeedsCvm, got {endpoint:?}")
            }
        }
    }

    #[test]
    fn phala_cvm_json_builds_endpoint_from_teepod_name() {
        // Exact shape emitted by `phala cvms get <app_id> --json` (>=1.1):
        // `{success, data: { ..., teepod: { name }, app_id }}`.
        let payload = r#"{
            "success": true,
            "data": {
                "id": 42,
                "name": "finite-agent-abc-123",
                "status": "running",
                "app_id": "b86bdd97e9575f178ec5ccfe6fab6e138781ea1c",
                "teepod_id": 5,
                "teepod": { "id": 5, "name": "prod5" },
                "app_url": "https://something-else",
                "instance_id": "i-1234"
            }
        }"#;

        let endpoint = phala_endpoint_from_cvm_json(
            payload,
            "b86bdd97e9575f178ec5ccfe6fab6e138781ea1c",
            "finite-agent-abc-123",
            8080,
        )
        .unwrap();

        assert_eq!(
            endpoint.public_base_url,
            "https://b86bdd97e9575f178ec5ccfe6fab6e138781ea1c-8080.dstack-pha-prod5.phala.network"
        );
        assert_eq!(
            endpoint.contact_url,
            "https://b86bdd97e9575f178ec5ccfe6fab6e138781ea1c-8080.dstack-pha-prod5.phala.network/contact"
        );
        assert_eq!(endpoint.teepod_name, "prod5");
    }

    #[test]
    fn phala_cvm_json_uses_provided_endpoint_url_and_node_info() {
        // Real `phala cvms get <app_id> --json` (1.1.x): NO `data` wrapper,
        // the node is under `node_info.name`, and the full public URL is handed
        // to us in `endpoints[].app`. We must use that URL verbatim.
        let payload = r#"{
            "success": true,
            "id": 99,
            "name": "finite-agent-7cc86b88",
            "app_id": "d6afaf5f4775f4774de28527d93fe9a06639cd3d",
            "status": "running",
            "node_info": { "object_type": "node", "id": 26, "name": "prod5", "status": "ONLINE" },
            "gateway": { "base_domain": "dstack-pha-prod5.phala.network" },
            "endpoints": [
                { "app": "https://d6afaf5f4775f4774de28527d93fe9a06639cd3d-8080.dstack-pha-prod5.phala.network", "instance": "" }
            ],
            "app_url": null
        }"#;

        let endpoint = phala_endpoint_from_cvm_json(
            payload,
            "d6afaf5f4775f4774de28527d93fe9a06639cd3d",
            "finite-agent-7cc86b88",
            8080,
        )
        .unwrap();

        assert_eq!(
            endpoint.public_base_url,
            "https://d6afaf5f4775f4774de28527d93fe9a06639cd3d-8080.dstack-pha-prod5.phala.network"
        );
        assert_eq!(
            endpoint.contact_url,
            "https://d6afaf5f4775f4774de28527d93fe9a06639cd3d-8080.dstack-pha-prod5.phala.network/contact"
        );
        assert_eq!(
            endpoint.health_url,
            "https://d6afaf5f4775f4774de28527d93fe9a06639cd3d-8080.dstack-pha-prod5.phala.network/healthz"
        );
        assert_eq!(endpoint.teepod_name, "prod5");
    }

    #[test]
    fn phala_endpoint_uses_app_id_port_and_teepod_name() {
        // Backward-compat: older CLI embedded the CVM node in the `dstack_apps`
        // listing, so the endpoint resolves from a single `apps` call.
        let payload = serde_json::json!({
            "dstack_apps": [
                {
                    "name": "finite-agent-abc-123",
                    "app_id": "b86bdd97e9575f178ec5ccfe6fab6e138781ea1c",
                    "current_cvm": {
                        "teepod_name": "prod5"
                    }
                }
            ]
        });

        let endpoint = match phala_app_from_apps_json(
            &serde_json::to_string(&payload).unwrap(),
            "finite-agent-abc-123",
            8080,
        )
        .unwrap()
        {
            PhalaAppLookup::Endpoint(endpoint) => endpoint,
            PhalaAppLookup::NeedsCvm(app_id) => {
                panic!("dstack_apps shape embeds the node; expected Endpoint, got {app_id}")
            }
        };

        assert_eq!(
            endpoint.public_base_url,
            "https://b86bdd97e9575f178ec5ccfe6fab6e138781ea1c-8080.dstack-pha-prod5.phala.network"
        );
        assert_eq!(
            endpoint.contact_url,
            "https://b86bdd97e9575f178ec5ccfe6fab6e138781ea1c-8080.dstack-pha-prod5.phala.network/contact"
        );
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
        provisioned: Vec<ProvisionFinitePrivateRuntimeKeyInput>,
        registered: Vec<RegisterAgentCreationRuntimeInput>,
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
                provisioned: Vec::new(),
                registered: Vec::new(),
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
                provisioned: Vec::new(),
                registered: Vec::new(),
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
                provisioned: Vec::new(),
                registered: Vec::new(),
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
        runner_capacity: RunnerLeaseCapacity,
        uses_core_heartbeat: bool,
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
                runner_capacity: RunnerLeaseCapacity::default(),
                uses_core_heartbeat: true,
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
                runner_capacity: RunnerLeaseCapacity::default(),
                uses_core_heartbeat: true,
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
                runner_capacity: RunnerLeaseCapacity::default(),
                uses_core_heartbeat: true,
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
    }

    impl RuntimeLauncher for FakeLauncher {
        fn validate_ready(&self) -> Result<(), RunnerError> {
            if let Some(message) = &self.ready_error {
                return Err(RunnerError::RuntimeLaunch(message.clone()));
            }
            Ok(())
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
            Some(RuntimeSourceIdentity {
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "finite-agent_123".to_string(),
            })
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
                limit_profile_id: "finite-private-generous".to_string(),
                status: finite_saas_core::FinitePrivateGrantStatus::Active,
                current_window_started_at: None,
                current_window_used_units: 0,
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
                import_candidate_id: None,
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
                runner_class: RunnerClass::Phala,
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
        }
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
