use super::*;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

const DEFAULT_APPLE_CONTAINER_NAME_PREFIX: &str = "finite-apple";
const APPLE_CONTAINER_PROVIDER_DIR: &str = "apple-container";
static APPLE_CONTAINER_PREFLIGHT: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct AppleContainerConfig {
    pub container_bin: PathBuf,
    pub source_host_id: String,
    pub image: String,
    /// Digest from Core's immutable artifact reference. Local Apple Container
    /// stores address freshly built images by tag, so an optional local image
    /// reference is accepted only when its inspected descriptor matches this.
    pub expected_image_descriptor_digest: Option<String>,
    pub runtime_artifact_id: Option<String>,
    pub runtime_artifact_kind: Option<RuntimeArtifactKind>,
    pub runtime_state_schema_version: Option<String>,
    pub work_root: PathBuf,
    pub finitechat_server_url: String,
    pub agent_picture_url: String,
    pub name_prefix: String,
    pub host_port: u16,
    pub container_port: u16,
    pub public_base_url: Option<String>,
    pub platform: Option<String>,
    pub rosetta: bool,
    pub cpus: Option<u32>,
    pub memory: Option<String>,
    pub max_container_count: Option<u32>,
    pub drain_new_leases: bool,
    pub available_memory_bytes: Option<u64>,
    pub command_timeout: Duration,
    pub launch_timeout: Duration,
    pub readiness_timeout: Duration,
    pub readiness_interval: Duration,
    pub stop_timeout_secs: u64,
}

impl AppleContainerConfig {
    pub fn validate(&self) -> Result<(), RunnerError> {
        if self.container_bin.as_os_str().is_empty() {
            return Err(RunnerError::MissingAppleContainerBinary);
        }
        if self.source_host_id.trim().is_empty() {
            return Err(RunnerError::MissingSourceHostId);
        }
        if self.image.trim().is_empty() {
            return Err(RunnerError::MissingRuntimeArtifactReference);
        }
        if self
            .expected_image_descriptor_digest
            .as_deref()
            .is_some_and(|digest| {
                let Some(hex) = digest.strip_prefix("sha256:") else {
                    return true;
                };
                hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(RunnerError::RuntimeLaunch(
                "Apple Container expected image descriptor must be a sha256 digest".to_string(),
            ));
        }
        if self.work_root.as_os_str().is_empty() {
            return Err(RunnerError::MissingWorkRoot);
        }
        if !self.work_root.is_absolute() {
            return Err(RunnerError::RuntimeLaunch(
                "Apple Container work root must be an absolute path".to_string(),
            ));
        }
        if self.finitechat_server_url.trim().is_empty() {
            return Err(RunnerError::MissingFinitechatServerUrl);
        }
        if self.host_port == 0 || self.container_port == 0 {
            return Err(RunnerError::InvalidAppleContainerHostPort);
        }
        let owned_name_prefix = self.owned_name_prefix();
        if owned_name_prefix.is_empty() {
            return Err(RunnerError::RuntimeLaunch(
                "Apple Container name prefix is empty after sanitization".to_string(),
            ));
        }
        if owned_name_prefix.len() > 32 {
            return Err(RunnerError::RuntimeLaunch(
                "Apple Container name prefix must be at most 32 characters".to_string(),
            ));
        }
        if let Some(kind) = self.runtime_artifact_kind
            && kind != RuntimeArtifactKind::OciImage
        {
            return Err(RunnerError::RuntimeLaunch(format!(
                "Apple Container launcher requires an OCI image artifact, got {}",
                kind.as_str()
            )));
        }
        if self.rosetta && self.platform.as_deref() != Some("linux/amd64") {
            return Err(RunnerError::RuntimeLaunch(
                "Apple Container Rosetta requires platform linux/amd64".to_string(),
            ));
        }
        if self.cpus == Some(0) {
            return Err(RunnerError::RuntimeLaunch(
                "Apple Container CPU count must be positive".to_string(),
            ));
        }
        if self
            .memory
            .as_deref()
            .is_some_and(|memory| memory.trim().is_empty())
        {
            return Err(RunnerError::RuntimeLaunch(
                "Apple Container memory must not be empty".to_string(),
            ));
        }
        Ok(())
    }

    fn owned_name_prefix(&self) -> String {
        let configured = self.name_prefix.trim();
        sanitize_sandbox_name(if configured.is_empty() {
            DEFAULT_APPLE_CONTAINER_NAME_PREFIX
        } else {
            configured
        })
        .to_ascii_lowercase()
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

impl Default for AppleContainerConfig {
    fn default() -> Self {
        Self {
            container_bin: PathBuf::from("container"),
            source_host_id: String::new(),
            image: String::new(),
            expected_image_descriptor_digest: None,
            runtime_artifact_id: None,
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: None,
            work_root: PathBuf::new(),
            finitechat_server_url: DEFAULT_FINITECHAT_SERVER_URL.to_string(),
            agent_picture_url: DEFAULT_FINITE_AGENT_PICTURE_URL.to_string(),
            name_prefix: DEFAULT_APPLE_CONTAINER_NAME_PREFIX.to_string(),
            host_port: 18080,
            container_port: DEFAULT_DOCKER_CONTAINER_PORT,
            public_base_url: None,
            platform: Some("linux/arm64".to_string()),
            rosetta: false,
            cpus: Some(4),
            memory: Some("4G".to_string()),
            max_container_count: None,
            drain_new_leases: false,
            available_memory_bytes: None,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            launch_timeout: DEFAULT_LAUNCH_TIMEOUT,
            readiness_timeout: DEFAULT_RUNTIME_READY_TIMEOUT,
            readiness_interval: DEFAULT_RUNTIME_READY_INTERVAL,
            stop_timeout_secs: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppleContainerLaunchPlan {
    pub container_name: String,
    pub state_root: PathBuf,
    pub public_base_url: String,
    pub health_url: String,
    pub contact_url: String,
    pub host_port: u16,
    pub container_port: u16,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AppleContainerCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
}

impl std::fmt::Debug for AppleContainerCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let env_keys = self.env.iter().map(|(key, _)| key).collect::<Vec<_>>();
        formatter
            .debug_struct("AppleContainerCommand")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("env_keys", &env_keys)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct AppleContainerLauncher {
    config: AppleContainerConfig,
}

impl AppleContainerLauncher {
    pub fn new(config: AppleContainerConfig) -> Self {
        Self { config }
    }

    pub fn plan_launch(&self, lease: &AgentCreationLease) -> AppleContainerLaunchPlan {
        apple_container_launch_plan(&self.config, lease)
    }

    fn run_command(
        &self,
        command: AppleContainerCommand,
        timeout: Duration,
    ) -> Result<String, RunnerError> {
        let output = execute_command(&command, timeout)?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if output.status.success() {
            return Ok(stdout);
        }
        Err(command_failure(&command, &output))
    }

    fn inspect_owned(
        &self,
        plan: &AppleContainerLaunchPlan,
        project_id: &str,
    ) -> Result<Option<AppleContainerState>, RunnerError> {
        Ok(self
            .inspect_owned_with_policy(plan, project_id, true)?
            .map(|runtime| runtime.state))
    }

    fn inspect_replaceable_compute(
        &self,
        plan: &AppleContainerLaunchPlan,
        project_id: &str,
    ) -> Result<Option<AppleContainerState>, RunnerError> {
        Ok(self
            .inspect_owned_with_policy(plan, project_id, false)?
            .map(|runtime| runtime.state))
    }

    fn inspect_owned_with_policy(
        &self,
        plan: &AppleContainerLaunchPlan,
        project_id: &str,
        require_current_spec: bool,
    ) -> Result<Option<OwnedAppleContainer>, RunnerError> {
        let command = AppleContainerCommand {
            program: self.config.container_bin.clone(),
            args: vec![
                OsString::from("inspect"),
                OsString::from(&plan.container_name),
            ],
            env: Vec::new(),
        };
        let output = execute_command(&command, self.config.command_timeout)?;
        if !output.status.success() {
            let combined = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .to_ascii_lowercase();
            if combined.contains("not found") || combined.contains("no such") {
                return Ok(None);
            }
            return Err(command_failure(&command, &output));
        }

        let payload: Vec<AppleContainerInspect> =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                RunnerError::RuntimeLaunch(format!(
                    "Apple Container inspect returned invalid JSON for {}: {error}",
                    plan.container_name
                ))
            })?;
        let inspected = payload.first().ok_or_else(|| {
            RunnerError::RuntimeLaunch(format!(
                "Apple Container inspect returned no record for {}",
                plan.container_name
            ))
        })?;
        validate_compute_ownership(&self.config, plan, project_id, inspected)?;
        if require_current_spec {
            validate_owned_container_spec(&self.config, plan, inspected)?;
        }
        Ok(Some(OwnedAppleContainer::from_inspect(inspected)?))
    }

    fn desired_image_descriptor_digest(&self) -> Result<String, RunnerError> {
        let command = AppleContainerCommand {
            program: self.config.container_bin.clone(),
            args: vec![
                OsString::from("image"),
                OsString::from("inspect"),
                OsString::from(self.config.image.trim()),
            ],
            env: Vec::new(),
        };
        let output = execute_command(&command, self.config.command_timeout)?;
        if !output.status.success() {
            return Err(command_failure(&command, &output));
        }
        let payload: Vec<AppleImageInspect> =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                RunnerError::RuntimeLaunch(format!(
                    "Apple Container image inspect returned invalid JSON for {}: {error}",
                    self.config.image
                ))
            })?;
        payload
            .first()
            .and_then(|image| image.configuration.descriptor.digest.as_deref())
            .map(str::trim)
            .filter(|digest| !digest.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                RunnerError::RuntimeLaunch(format!(
                    "Apple Container image inspect did not report a descriptor digest for {}",
                    self.config.image
                ))
            })
    }

    fn start(&self, container_name: &str) -> Result<(), RunnerError> {
        self.run_command(
            AppleContainerCommand {
                program: self.config.container_bin.clone(),
                args: vec![OsString::from("start"), OsString::from(container_name)],
                env: Vec::new(),
            },
            self.config.command_timeout,
        )?;
        Ok(())
    }

    fn stop(&self, container_name: &str) -> Result<(), RunnerError> {
        self.run_command(
            AppleContainerCommand {
                program: self.config.container_bin.clone(),
                args: vec![
                    OsString::from("stop"),
                    OsString::from("--signal"),
                    OsString::from("SIGTERM"),
                    OsString::from("--time"),
                    OsString::from(self.config.stop_timeout_secs.to_string()),
                    OsString::from(container_name),
                ],
                env: Vec::new(),
            },
            self.config
                .command_timeout
                .max(Duration::from_secs(self.config.stop_timeout_secs + 5)),
        )?;
        Ok(())
    }

    fn delete_compute(&self, container_name: &str) -> Result<(), RunnerError> {
        self.run_command(
            AppleContainerCommand {
                program: self.config.container_bin.clone(),
                args: vec![
                    OsString::from("delete"),
                    OsString::from("--force"),
                    OsString::from(container_name),
                ],
                env: Vec::new(),
            },
            self.config.command_timeout,
        )?;
        Ok(())
    }

    fn wait_for_runtime_http(&self, plan: &AppleContainerLaunchPlan) -> Result<(), RunnerError> {
        // Runtime management owns generic process readiness only. The product
        // contact URL is a published fact, while chat admission remains the
        // Finite Chat Device's concern.
        wait_for_http_json_ready(
            &plan.health_url,
            "Apple Container runtime /healthz",
            self.config.readiness_timeout,
            self.config.readiness_interval,
        )
    }

    fn launch_facts(
        &self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
        plan: AppleContainerLaunchPlan,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
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

    fn validate_control_ownership(
        &self,
        lease: &RuntimeControlLease,
    ) -> Result<AppleContainerLaunchPlan, RunnerError> {
        if lease.runtime.source_host_id != self.config.source_host_id {
            return Err(RunnerError::RuntimeLaunch(format!(
                "runtime belongs to source host {}, not {}",
                lease.runtime.source_host_id, self.config.source_host_id
            )));
        }
        if lease.runtime.source_machine_id != lease.request.source_machine_id {
            return Err(RunnerError::RuntimeLaunch(format!(
                "runtime source machine {} did not match control request {}",
                lease.runtime.source_machine_id, lease.request.source_machine_id
            )));
        }
        let plan = apple_container_launch_plan_for_source_machine(
            &self.config,
            &lease.request.source_machine_id,
        );
        if self
            .inspect_owned(&plan, &lease.runtime.project_id)?
            .is_none()
        {
            return Err(RunnerError::RuntimeLaunch(format!(
                "owned Apple Container {} does not exist",
                plan.container_name
            )));
        }
        Ok(plan)
    }
}

impl RuntimeLauncher for AppleContainerLauncher {
    fn runtime_capabilities(&self) -> RuntimeCapabilitiesEnvelope {
        state_preserving_runtime_capabilities(false)
    }

    fn runner_class(&self) -> RunnerClass {
        RunnerClass::AppleContainer
    }

    fn validate_ready(&self) -> Result<(), RunnerError> {
        self.config.validate()?;
        if APPLE_CONTAINER_PREFLIGHT.get().is_none() {
            let version = self.run_command(
                AppleContainerCommand {
                    program: self.config.container_bin.clone(),
                    args: vec![OsString::from("--version")],
                    env: Vec::new(),
                },
                self.config.command_timeout,
            )?;
            if !version.contains("version 1.1.") {
                return Err(RunnerError::RuntimeLaunch(format!(
                    "Apple Container 1.1.x is required; installed CLI reported {}",
                    version.trim()
                )));
            }
            let _ = APPLE_CONTAINER_PREFLIGHT.set(());
        }

        let status = self.run_command(
            AppleContainerCommand {
                program: self.config.container_bin.clone(),
                args: vec![
                    OsString::from("system"),
                    OsString::from("status"),
                    OsString::from("--format"),
                    OsString::from("json"),
                ],
                env: Vec::new(),
            },
            self.config.command_timeout,
        )?;
        let status: serde_json::Value = serde_json::from_str(&status).map_err(|error| {
            RunnerError::RuntimeLaunch(format!(
                "Apple Container system status returned invalid JSON: {error}"
            ))
        })?;
        if status.get("status").and_then(serde_json::Value::as_str) != Some("running") {
            return Err(RunnerError::RuntimeLaunch(
                "Apple Container services are not running; run `container system start`"
                    .to_string(),
            ));
        }
        if let Some(expected) = self.config.expected_image_descriptor_digest.as_deref() {
            let actual = self.desired_image_descriptor_digest()?;
            if actual != expected {
                return Err(RunnerError::RuntimeLaunch(format!(
                    "local Apple Container image descriptor {actual} does not match Core artifact {expected}"
                )));
            }
        }
        Ok(())
    }

    fn uses_core_runtime_heartbeat(&self) -> bool {
        false
    }

    fn runner_capacity(&self) -> RunnerLeaseCapacity {
        RunnerLeaseCapacity {
            runner_classes: vec![self.runner_class()],
            draining: self.config.drain_new_leases,
            max_sandbox_count: self.config.max_container_count,
            active_sandbox_count: active_owned_container_count(&self.config),
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
        options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        self.validate_ready()?;
        let plan = self.validate_control_ownership(lease)?;
        let inspected = self
            .inspect_owned_with_policy(&plan, &lease.runtime.project_id, true)?
            .ok_or_else(|| {
                RunnerError::RuntimeLaunch(format!(
                    "owned Apple Container {} disappeared before restart",
                    plan.container_name
                ))
            })?;
        let desired_digest = self.desired_image_descriptor_digest()?;
        if !apple_container_replacement_required(&inspected, &desired_digest, options) {
            if inspected.state == AppleContainerState::Running {
                self.stop(&plan.container_name)?;
            }
            self.start(&plan.container_name)?;
        } else {
            // Mutable tags are convenient only in the local development
            // provider. Replace compute when the resolved OCI descriptor has
            // changed or the bounded non-secret desired environment has
            // drifted. Carry the inspected environment opaquely, reconciling
            // only the desired map and keeping the exact same /data bind. No
            // product feature is interpreted, existing credentials are not
            // rotated, and no user-state directory is removed.
            let replacement_environment =
                merge_desired_runtime_environment(inspected.environment, options);
            if inspected.state == AppleContainerState::Running {
                self.stop(&plan.container_name)?;
            }
            self.delete_compute(&plan.container_name)?;
            if let Err(error) = self.run_command(
                apple_container_run_command_with_env(
                    &self.config,
                    &plan,
                    &lease.runtime.project_id,
                    replacement_environment,
                ),
                self.config.launch_timeout,
            ) {
                let _ = self.delete_compute(&plan.container_name);
                return Err(error);
            }
        }
        self.wait_for_runtime_http(&plan)
    }

    fn stop_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        self.validate_ready()?;
        let plan = self.validate_control_ownership(lease)?;
        if self.inspect_owned(&plan, &lease.runtime.project_id)?
            == Some(AppleContainerState::Running)
        {
            self.stop(&plan.container_name)?;
        }
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

        // A creation lease can be retried after the provider mutation but
        // before Core durably records completion. Recreate replaceable compute
        // on every retry so it cannot retain an earlier ephemeral key, vmnet
        // endpoint, resource spec, or mutable local image tag. The stable
        // source-machine id and /data bind keep this idempotent at the Runtime
        // and user-state boundary.
        if self
            .inspect_replaceable_compute(&plan, &lease.project.id)?
            .is_some()
        {
            self.delete_compute(&plan.container_name)?;
        }
        if let Err(error) = self.run_command(
            apple_container_run_command(&self.config, &plan, lease, options),
            self.config.launch_timeout,
        ) {
            // `container run --detach` can mutate the provider before its
            // client exits or times out. Remove only compute on a failed
            // ensure so the durable bind directory remains the source of
            // truth and capacity is not orphaned.
            let _ = self.delete_compute(&plan.container_name);
            return Err(error);
        }
        if let Err(error) = self.wait_for_runtime_http(&plan) {
            let cleanup = self.delete_compute(&plan.container_name);
            if let Err(cleanup) = cleanup {
                return Err(RunnerError::RuntimeLaunch(format!(
                    "{error}; additionally failed to remove unready Apple compute: {cleanup}"
                )));
            }
            return Err(error);
        }
        self.launch_facts(lease, options, plan)
    }

    fn cleanup_failed_launch(&mut self, facts: &RuntimeLaunchFacts) -> Result<(), RunnerError> {
        // Core revokes a newly provisioned inference key after this hook. Drop
        // only replaceable compute so a retry cannot adopt a VM configured
        // with that revoked key. The bind-mounted state root is deliberately
        // retained; the next ensure recreates compute around the same /data.
        self.delete_compute(&facts.source_machine_id)
    }
}

pub(crate) fn apple_container_launch_plan(
    config: &AppleContainerConfig,
    lease: &AgentCreationLease,
) -> AppleContainerLaunchPlan {
    let request_suffix = lease
        .request
        .id
        .strip_prefix("agent_request_")
        .unwrap_or(&lease.request.id);
    let container_name =
        sanitize_sandbox_name(&format!("{}-{request_suffix}", config.owned_name_prefix()))
            .to_ascii_lowercase();
    apple_container_launch_plan_for_source_machine(config, &container_name)
}

fn apple_container_launch_plan_for_source_machine(
    config: &AppleContainerConfig,
    source_machine_id: &str,
) -> AppleContainerLaunchPlan {
    let container_name = sanitize_sandbox_name(source_machine_id);
    let public_base_url = config.public_base_url();
    AppleContainerLaunchPlan {
        state_root: config
            .work_root
            .join(APPLE_CONTAINER_PROVIDER_DIR)
            .join(&container_name),
        health_url: format!("{public_base_url}/healthz"),
        contact_url: format!("{public_base_url}/contact"),
        public_base_url,
        host_port: config.host_port,
        container_port: config.container_port,
        container_name,
    }
}

pub(crate) fn apple_container_run_command(
    config: &AppleContainerConfig,
    plan: &AppleContainerLaunchPlan,
    lease: &AgentCreationLease,
    options: &RuntimeLaunchOptions,
) -> AppleContainerCommand {
    apple_container_run_command_with_env(
        config,
        plan,
        &lease.project.id,
        apple_container_runtime_env(config, plan, lease, options),
    )
}

fn apple_container_run_command_with_env(
    config: &AppleContainerConfig,
    plan: &AppleContainerLaunchPlan,
    project_id: &str,
    environment: Vec<(String, String)>,
) -> AppleContainerCommand {
    let mut args = vec![
        OsString::from("run"),
        OsString::from("--progress"),
        OsString::from("none"),
        OsString::from("--detach"),
        OsString::from("--name"),
        OsString::from(&plan.container_name),
        OsString::from("--publish"),
        OsString::from(format!(
            "127.0.0.1:{}:{}/tcp",
            plan.host_port, plan.container_port
        )),
        OsString::from("--volume"),
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
        OsString::from(format!("computer.finite.v2.project_id={project_id}")),
    ];
    if let Some(runtime_artifact_id) = config
        .runtime_artifact_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(OsString::from("--label"));
        args.push(OsString::from(format!(
            "computer.finite.v2.runtime_artifact_id={runtime_artifact_id}"
        )));
    }
    if let Some(state_schema_version) = config
        .runtime_state_schema_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(OsString::from("--label"));
        args.push(OsString::from(format!(
            "computer.finite.v2.state_schema_version={state_schema_version}"
        )));
    }
    if let Some(platform) = config
        .platform
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(OsString::from("--platform"));
        args.push(OsString::from(platform));
    }
    if config.rosetta {
        args.push(OsString::from("--rosetta"));
    }
    if let Some(cpus) = config.cpus {
        args.push(OsString::from("--cpus"));
        args.push(OsString::from(cpus.to_string()));
    }
    if let Some(memory) = config.memory.as_deref() {
        args.push(OsString::from("--memory"));
        args.push(OsString::from(memory));
    }

    let env = environment
        .into_iter()
        .map(|(key, value)| {
            args.push(OsString::from("--env"));
            args.push(OsString::from(&key));
            (OsString::from(key), OsString::from(value))
        })
        .collect();
    args.push(OsString::from(config.image.trim()));

    AppleContainerCommand {
        program: config.container_bin.clone(),
        args,
        env,
    }
}

pub(crate) fn apple_container_runtime_env(
    config: &AppleContainerConfig,
    plan: &AppleContainerLaunchPlan,
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

fn execute_command(
    command: &AppleContainerCommand,
    timeout: Duration,
) -> Result<Output, RunnerError> {
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .envs(command.env.iter().map(|(key, value)| (key, value)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = process
        .spawn()
        .map_err(|error| RunnerError::CommandExecution {
            program: command.program.display().to_string(),
            message: error.to_string(),
        })?;
    wait_with_captured_output(child, &command.program, timeout)
}

fn command_failure(command: &AppleContainerCommand, output: &Output) -> RunnerError {
    let stdout = redact_values(command, &String::from_utf8_lossy(&output.stdout));
    let stderr = redact_values(command, &String::from_utf8_lossy(&output.stderr));
    RunnerError::CommandExecution {
        program: command.program.display().to_string(),
        message: format!(
            "exit status {} stdout={stdout} stderr={stderr}",
            output.status
        ),
    }
}

fn redact_values(command: &AppleContainerCommand, value: &str) -> String {
    let mut values = command
        .env
        .iter()
        .map(|(_, secret)| secret.to_string_lossy().to_string())
        .filter(|secret| !secret.is_empty())
        .collect::<Vec<_>>();
    values.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    values.dedup();
    values
        .into_iter()
        .fold(value.to_string(), |output, secret| {
            output.replace(&secret, "<redacted>")
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppleContainerState {
    Running,
    Stopped,
}

struct OwnedAppleContainer {
    state: AppleContainerState,
    image_descriptor_digest: Option<String>,
    environment: Vec<(String, String)>,
}

fn apple_container_replacement_required(
    inspected: &OwnedAppleContainer,
    desired_image_descriptor_digest: &str,
    options: &RuntimeRestartOptions,
) -> bool {
    inspected.image_descriptor_digest.as_deref() != Some(desired_image_descriptor_digest)
        || options
            .environment()
            .iter()
            .any(|(desired_key, desired_value)| {
                inspected
                    .environment
                    .iter()
                    .rfind(|(current_key, _)| current_key == desired_key)
                    .is_none_or(|(_, current_value)| current_value != desired_value)
            })
}

impl OwnedAppleContainer {
    fn from_inspect(inspected: &AppleContainerInspect) -> Result<Self, RunnerError> {
        let mut environment =
            Vec::with_capacity(inspected.configuration.init_process.environment.len());
        for entry in &inspected.configuration.init_process.environment {
            let Some((key, value)) = entry.split_once('=') else {
                return Err(RunnerError::RuntimeLaunch(
                    "Apple Container inspect returned a malformed environment entry".to_string(),
                ));
            };
            if key.is_empty() {
                return Err(RunnerError::RuntimeLaunch(
                    "Apple Container inspect returned an empty environment key".to_string(),
                ));
            }
            environment.push((key.to_owned(), value.to_owned()));
        }
        Ok(Self {
            state: AppleContainerState::from_status(inspected.status.state.as_deref()),
            image_descriptor_digest: inspected.configuration.image.descriptor.digest.clone(),
            environment,
        })
    }
}

impl AppleContainerState {
    fn from_status(status: Option<&str>) -> Self {
        if status.is_some_and(|value| value.eq_ignore_ascii_case("running")) {
            Self::Running
        } else {
            Self::Stopped
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleContainerInspect {
    configuration: AppleContainerInspectConfiguration,
    #[serde(default)]
    status: AppleContainerInspectStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleContainerInspectConfiguration {
    image: AppleContainerInspectImage,
    #[serde(default)]
    init_process: AppleContainerInspectInitProcess,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    mounts: Vec<AppleContainerInspectMount>,
    #[serde(default)]
    published_ports: Vec<AppleContainerInspectPort>,
}

#[derive(Debug, Deserialize)]
struct AppleContainerInspectImage {
    reference: String,
    #[serde(default)]
    descriptor: AppleContainerDescriptor,
}

#[derive(Debug, Default, Deserialize)]
struct AppleContainerDescriptor {
    digest: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct AppleContainerInspectInitProcess {
    #[serde(default)]
    environment: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AppleImageInspect {
    configuration: AppleImageInspectConfiguration,
}

#[derive(Debug, Deserialize)]
struct AppleImageInspectConfiguration {
    #[serde(default)]
    descriptor: AppleContainerDescriptor,
}

#[derive(Debug, Default, Deserialize)]
struct AppleContainerInspectStatus {
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppleContainerInspectMount {
    source: String,
    destination: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleContainerInspectPort {
    container_port: u16,
    host_address: String,
    host_port: u16,
    #[serde(default)]
    proto: String,
}

fn validate_compute_ownership(
    config: &AppleContainerConfig,
    plan: &AppleContainerLaunchPlan,
    project_id: &str,
    inspected: &AppleContainerInspect,
) -> Result<(), RunnerError> {
    let labels = &inspected.configuration.labels;
    let required = [
        ("computer.finite.v2.runtime", "true"),
        (
            "computer.finite.v2.source_host_id",
            config.source_host_id.as_str(),
        ),
        (
            "computer.finite.v2.source_machine_id",
            plan.container_name.as_str(),
        ),
        ("computer.finite.v2.project_id", project_id),
    ];
    for (key, expected) in required {
        if labels.get(key).map(String::as_str) != Some(expected) {
            return Err(RunnerError::RuntimeLaunch(format!(
                "refusing to adopt Apple Container {}: ownership label {key} did not match",
                plan.container_name
            )));
        }
    }
    let expected_state_root = plan.state_root.to_string_lossy();
    if !inspected
        .configuration
        .mounts
        .iter()
        .any(|mount| mount.destination == "/data" && mount.source == expected_state_root.as_ref())
    {
        return Err(RunnerError::RuntimeLaunch(format!(
            "refusing to adopt Apple Container {}: durable /data mount does not match",
            plan.container_name
        )));
    }
    Ok(())
}

fn validate_owned_container_spec(
    config: &AppleContainerConfig,
    plan: &AppleContainerLaunchPlan,
    inspected: &AppleContainerInspect,
) -> Result<(), RunnerError> {
    if !image_references_equivalent(&inspected.configuration.image.reference, &config.image) {
        return Err(RunnerError::RuntimeLaunch(format!(
            "refusing to adopt Apple Container {}: image reference does not match the promoted artifact",
            plan.container_name
        )));
    }
    for (key, expected) in [
        (
            "computer.finite.v2.runtime_artifact_id",
            config.runtime_artifact_id.as_deref(),
        ),
        (
            "computer.finite.v2.state_schema_version",
            config.runtime_state_schema_version.as_deref(),
        ),
    ] {
        if let Some(expected) = expected
            && inspected.configuration.labels.get(key).map(String::as_str) != Some(expected)
        {
            return Err(RunnerError::RuntimeLaunch(format!(
                "refusing to adopt Apple Container {}: {key} does not match the promoted artifact",
                plan.container_name
            )));
        }
    }
    if !inspected.configuration.published_ports.iter().any(|port| {
        port.container_port == plan.container_port
            && port.host_port == plan.host_port
            && port.host_address == "127.0.0.1"
            && (port.proto.is_empty() || port.proto.eq_ignore_ascii_case("tcp"))
    }) {
        return Err(RunnerError::RuntimeLaunch(format!(
            "refusing to adopt Apple Container {}: loopback health port does not match",
            plan.container_name
        )));
    }
    Ok(())
}

fn image_references_equivalent(actual: &str, expected: &str) -> bool {
    let actual = actual.trim();
    let expected = expected.trim();
    actual == expected
        || actual.strip_prefix("docker.io/library/") == Some(expected)
        || expected.strip_prefix("docker.io/library/") == Some(actual)
}

fn active_owned_container_count(config: &AppleContainerConfig) -> Option<u32> {
    let command = AppleContainerCommand {
        program: config.container_bin.clone(),
        args: vec![
            OsString::from("list"),
            OsString::from("--all"),
            OsString::from("--quiet"),
        ],
        env: Vec::new(),
    };
    let output = execute_command(&command, config.command_timeout).ok()?;
    if !output.status.success() {
        return None;
    }
    let prefix = format!("{}-", config.owned_name_prefix());
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.trim().starts_with(&prefix))
            .count() as u32,
    )
}

#[cfg(test)]
pub(crate) fn apple_command_args(command: &AppleContainerCommand) -> Vec<String> {
    command
        .args
        .iter()
        .map(|value| value.to_string_lossy().to_string())
        .collect()
}

#[cfg(test)]
pub(crate) fn apple_command_env_keys(command: &AppleContainerCommand) -> Vec<String> {
    command
        .env
        .iter()
        .map(|(key, _)| key.to_string_lossy().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AppleContainerConfig {
        AppleContainerConfig {
            source_host_id: "devfinity-apple".to_string(),
            image: "finite-agent-runtime:devfinity".to_string(),
            work_root: PathBuf::from("/tmp/devfinity/runner"),
            name_prefix: "finite-devfinity".to_string(),
            ..AppleContainerConfig::default()
        }
    }

    fn plan() -> AppleContainerLaunchPlan {
        AppleContainerLaunchPlan {
            container_name: "finite-devfinity-request-1".to_string(),
            state_root: PathBuf::from(
                "/tmp/devfinity/runner/apple-container/finite-devfinity-request-1",
            ),
            public_base_url: "http://127.0.0.1:18080".to_string(),
            health_url: "http://127.0.0.1:18080/healthz".to_string(),
            contact_url: "http://127.0.0.1:18080/contact".to_string(),
            host_port: 18080,
            container_port: 8080,
        }
    }

    fn inspected_json(project_id: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([{
            "configuration": {
                "image": { "reference": "docker.io/library/finite-agent-runtime:devfinity" },
                "labels": {
                    "computer.finite.v2.runtime": "true",
                    "computer.finite.v2.source_host_id": "devfinity-apple",
                    "computer.finite.v2.source_machine_id": "finite-devfinity-request-1",
                    "computer.finite.v2.project_id": project_id
                },
                "mounts": [{
                    "source": "/tmp/devfinity/runner/apple-container/finite-devfinity-request-1",
                    "destination": "/data"
                }],
                "publishedPorts": [{
                    "containerPort": 8080,
                    "hostAddress": "127.0.0.1",
                    "hostPort": 18080,
                    "proto": "tcp"
                }],
                "initProcess": {
                    "environment": ["FINITE_PRIVATE_API_KEY=must-never-be-logged"]
                }
            },
            "status": { "state": "running" }
        }]))
        .unwrap()
    }

    #[test]
    fn inspect_projection_accepts_only_the_owned_durable_runtime() {
        let payload: Vec<AppleContainerInspect> =
            serde_json::from_slice(&inspected_json("project-1")).unwrap();
        validate_compute_ownership(&config(), &plan(), "project-1", &payload[0]).unwrap();
        validate_owned_container_spec(&config(), &plan(), &payload[0]).unwrap();
        assert_eq!(
            AppleContainerState::from_status(payload[0].status.state.as_deref()),
            AppleContainerState::Running
        );
    }

    #[test]
    fn inspect_projection_rejects_cross_project_adoption() {
        let payload: Vec<AppleContainerInspect> =
            serde_json::from_slice(&inspected_json("different-project")).unwrap();
        let error =
            validate_compute_ownership(&config(), &plan(), "project-1", &payload[0]).unwrap_err();
        assert!(error.to_string().contains("ownership label"));
        assert!(!error.to_string().contains("must-never-be-logged"));
    }

    #[test]
    fn owned_stale_compute_is_replaceable_but_not_adoptable() {
        let mut payload: serde_json::Value =
            serde_json::from_slice(&inspected_json("project-1")).unwrap();
        payload[0]["configuration"]["image"]["reference"] =
            serde_json::Value::String("finite-agent-runtime:stale".to_string());
        payload[0]["configuration"]["publishedPorts"][0]["hostPort"] =
            serde_json::Value::from(19999);
        let payload: Vec<AppleContainerInspect> = serde_json::from_value(payload).unwrap();

        validate_compute_ownership(&config(), &plan(), "project-1", &payload[0]).unwrap();
        let error = validate_owned_container_spec(&config(), &plan(), &payload[0]).unwrap_err();
        assert!(error.to_string().contains("image reference"));
    }

    #[test]
    fn image_replacement_retains_current_vars_and_reconciles_desired_generic_vars() {
        let mut payload: serde_json::Value =
            serde_json::from_slice(&inspected_json("project-1")).unwrap();
        payload[0]["configuration"]["initProcess"]["environment"] = serde_json::json!([
            "FINITE_PRIVATE_API_KEY=existing-inference-key",
            "OPENAI_API_KEY=existing-inference-key",
            "FINITECHAT_SERVER_URL=https://chat.example.test",
            "FINITE_SITES_API=http://old-gateway.test",
            "UNRELATED_SETTING=preserved"
        ]);
        let payload: Vec<AppleContainerInspect> = serde_json::from_value(payload).unwrap();
        let inspected = OwnedAppleContainer::from_inspect(&payload[0]).unwrap();
        let options = RuntimeRestartOptions::new(BTreeMap::from([
            (
                "FINITE_SITES_API".to_string(),
                "http://new-gateway.test".to_string(),
            ),
            (
                "ANOTHER_PRODUCT_URL".to_string(),
                "http://another-service.test".to_string(),
            ),
        ]))
        .unwrap();

        let environment = merge_desired_runtime_environment(inspected.environment, &options);
        let command =
            apple_container_run_command_with_env(&config(), &plan(), "project-1", environment);
        let replacement_environment = command
            .env
            .iter()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_string(),
                    value.to_string_lossy().to_string(),
                )
            })
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            replacement_environment.get("FINITE_PRIVATE_API_KEY"),
            Some(&"existing-inference-key".to_string())
        );
        assert_eq!(
            replacement_environment.get("OPENAI_API_KEY"),
            Some(&"existing-inference-key".to_string())
        );
        assert_eq!(
            replacement_environment.get("FINITECHAT_SERVER_URL"),
            Some(&"https://chat.example.test".to_string())
        );
        assert_eq!(
            replacement_environment.get("UNRELATED_SETTING"),
            Some(&"preserved".to_string())
        );
        assert_eq!(
            replacement_environment.get("FINITE_SITES_API"),
            Some(&"http://new-gateway.test".to_string())
        );
        assert_eq!(
            replacement_environment.get("ANOTHER_PRODUCT_URL"),
            Some(&"http://another-service.test".to_string())
        );
        assert!(!format!("{command:?}").contains("existing-inference-key"));
        assert!(
            apple_command_args(&command)
                .iter()
                .all(|argument| !argument.contains("existing-inference-key"))
        );
    }

    #[test]
    fn same_image_env_drift_requires_replacement_until_desired_keys_match() {
        let mut payload: serde_json::Value =
            serde_json::from_slice(&inspected_json("project-1")).unwrap();
        payload[0]["configuration"]["image"]["descriptor"]["digest"] =
            serde_json::Value::String("sha256:same-image".to_string());
        payload[0]["configuration"]["initProcess"]["environment"] = serde_json::json!([
            "FINITE_PRIVATE_API_KEY=existing-inference-key",
            "FINITE_SITES_API=http://old-gateway.test"
        ]);
        let payload: Vec<AppleContainerInspect> = serde_json::from_value(payload).unwrap();
        let inspected = OwnedAppleContainer::from_inspect(&payload[0]).unwrap();

        let changed = RuntimeRestartOptions::new(BTreeMap::from([(
            "FINITE_SITES_API".to_string(),
            "http://new-gateway.test".to_string(),
        )]))
        .unwrap();
        assert!(apple_container_replacement_required(
            &inspected,
            "sha256:same-image",
            &changed
        ));

        let missing = RuntimeRestartOptions::new(BTreeMap::from([(
            "ANOTHER_PRODUCT_URL".to_string(),
            "http://another-service.test".to_string(),
        )]))
        .unwrap();
        assert!(apple_container_replacement_required(
            &inspected,
            "sha256:same-image",
            &missing
        ));

        let matching = RuntimeRestartOptions::new(BTreeMap::from([(
            "FINITE_SITES_API".to_string(),
            "http://old-gateway.test".to_string(),
        )]))
        .unwrap();
        assert!(!apple_container_replacement_required(
            &inspected,
            "sha256:same-image",
            &matching
        ));
    }

    #[test]
    fn command_debug_and_failures_redact_all_injected_values() {
        let command = AppleContainerCommand {
            program: PathBuf::from("container"),
            args: vec![
                OsString::from("run"),
                OsString::from("--env"),
                OsString::from("TOKEN"),
            ],
            env: vec![
                (
                    OsString::from("FINITECHAT_HERMES_INBOUND_STREAM"),
                    OsString::from("1"),
                ),
                (
                    OsString::from("TOKEN"),
                    OsString::from("secret-value-containing-1"),
                ),
            ],
        };
        let debug = format!("{command:?}");
        let redacted = redact_values(&command, "failure echoed secret-value-containing-1");
        assert!(!debug.contains("secret-value-containing-1"));
        assert!(!redacted.contains("secret-value-containing"));
        assert_eq!(redacted, "failure echoed <redacted>");
    }
}
