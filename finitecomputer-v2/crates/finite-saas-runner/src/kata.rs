use super::*;
use serde::Deserialize;
use std::process::Output;

const KATA_PROVIDER_DIR: &str = "kata";
const KATA_METADATA_DIR: &str = "kata-metadata";
const DEFAULT_KATA_RUNTIME: &str = "io.containerd.kata.v2";

#[derive(Debug, Clone)]
pub struct KataConfig {
    pub nerdctl_bin: PathBuf,
    pub kata_runtime_bin: PathBuf,
    pub namespace: String,
    pub runtime: String,
    pub source_host_id: String,
    pub image: String,
    pub runtime_artifact_id: Option<String>,
    pub runtime_artifact_kind: Option<RuntimeArtifactKind>,
    pub runtime_state_schema_version: Option<String>,
    pub work_root: PathBuf,
    pub finitechat_server_url: String,
    pub agent_picture_url: String,
    pub name_prefix: String,
    pub container_port: u16,
    pub cpus: Option<u32>,
    pub memory: Option<String>,
    pub pull_policy: Option<String>,
    pub max_container_count: Option<u32>,
    pub drain_new_leases: bool,
    pub available_memory_bytes: Option<u64>,
    pub command_timeout: Duration,
    pub launch_timeout: Duration,
    pub readiness_timeout: Duration,
    pub readiness_interval: Duration,
    pub stop_timeout_secs: u64,
}

impl Default for KataConfig {
    fn default() -> Self {
        Self {
            nerdctl_bin: PathBuf::from("nerdctl"),
            kata_runtime_bin: PathBuf::from("kata-runtime"),
            namespace: "finite".to_string(),
            runtime: DEFAULT_KATA_RUNTIME.to_string(),
            source_host_id: String::new(),
            image: String::new(),
            runtime_artifact_id: None,
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: None,
            work_root: PathBuf::new(),
            finitechat_server_url: DEFAULT_FINITECHAT_SERVER_URL.to_string(),
            agent_picture_url: DEFAULT_FINITE_AGENT_PICTURE_URL.to_string(),
            name_prefix: "finite-kata".to_string(),
            container_port: DEFAULT_DOCKER_CONTAINER_PORT,
            cpus: Some(4),
            memory: Some("8G".to_string()),
            pull_policy: Some("missing".to_string()),
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

impl KataConfig {
    pub fn validate(&self) -> Result<(), RunnerError> {
        if self.nerdctl_bin.as_os_str().is_empty() {
            return Err(RunnerError::MissingNerdctlBinary);
        }
        if self.kata_runtime_bin.as_os_str().is_empty() {
            return Err(RunnerError::MissingKataRuntimeBinary);
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
        if self.container_port == 0 {
            return Err(RunnerError::InvalidDockerHostPort);
        }
        validate_identifier("Kata namespace", &self.namespace)?;
        validate_identifier("Kata runtime", &self.runtime)?;
        validate_identifier("Kata container prefix", &self.name_prefix)?;
        if let Some(kind) = self.runtime_artifact_kind
            && kind != RuntimeArtifactKind::OciImage
        {
            return Err(RunnerError::RuntimeLaunch(format!(
                "Kata launcher requires an OCI image artifact, got {}",
                kind.as_str()
            )));
        }
        if let Some(policy) = self.pull_policy.as_deref() {
            match policy.trim() {
                "" | "always" | "missing" | "never" => {}
                other => {
                    return Err(RunnerError::RuntimeLaunch(format!(
                        "invalid Kata pull policy {other:?}; use always, missing, or never"
                    )));
                }
            }
        }
        if self
            .memory
            .as_deref()
            .is_some_and(|memory| memory.trim().is_empty())
        {
            return Err(RunnerError::RuntimeLaunch(
                "Kata memory limit must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

fn validate_identifier(name: &str, value: &str) -> Result<(), RunnerError> {
    let value = value.trim();
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(RunnerError::RuntimeLaunch(format!(
            "{name} contains unsupported characters"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KataLaunchPlan {
    pub container_name: String,
    pub state_root: PathBuf,
    pub metadata_root: PathBuf,
    pub env_file: PathBuf,
    pub container_port: u16,
}

impl KataLaunchPlan {
    fn public_base_url(&self, host_port: u16) -> String {
        format!("http://127.0.0.1:{host_port}")
    }

    fn health_url(&self, host_port: u16) -> String {
        format!("{}/healthz", self.public_base_url(host_port))
    }

    fn contact_url(&self, host_port: u16) -> String {
        format!("{}/contact", self.public_base_url(host_port))
    }
}

#[derive(Debug, Clone)]
pub struct KataLauncher {
    config: KataConfig,
}

impl KataLauncher {
    pub fn new(config: KataConfig) -> Self {
        Self { config }
    }

    pub fn plan_launch(&self, lease: &AgentCreationLease) -> KataLaunchPlan {
        kata_launch_plan(&self.config, lease)
    }

    fn command(&self, args: Vec<OsString>) -> PlannedCommand {
        let mut namespaced = vec![
            OsString::from("--namespace"),
            OsString::from(self.config.namespace.trim()),
        ];
        namespaced.extend(args);
        PlannedCommand {
            program: self.config.nerdctl_bin.clone(),
            cwd: None,
            args: namespaced,
            env: Vec::new(),
        }
    }

    fn execute(&self, command: &PlannedCommand, timeout: Duration) -> Result<Output, RunnerError> {
        let mut process = Command::new(&command.program);
        process
            .args(&command.args)
            .envs(command.env.iter().map(|(key, value)| (key, value)));
        if let Some(cwd) = command.cwd.as_ref() {
            process.current_dir(cwd);
        }
        let mut child = process
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| RunnerError::CommandExecution {
                program: command.program.display().to_string(),
                message: error.to_string(),
            })?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if started.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(RunnerError::CommandTimedOut {
                            program: command.program.display().to_string(),
                            timeout_secs: timeout.as_secs(),
                        });
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(error) => {
                    return Err(RunnerError::CommandExecution {
                        program: command.program.display().to_string(),
                        message: error.to_string(),
                    });
                }
            }
        }
        child
            .wait_with_output()
            .map_err(|error| RunnerError::CommandExecution {
                program: command.program.display().to_string(),
                message: error.to_string(),
            })
    }

    fn run_checked(
        &self,
        command: PlannedCommand,
        timeout: Duration,
    ) -> Result<String, RunnerError> {
        let output = self.execute(&command, timeout)?;
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

    fn inspect(&self, container_name: &str) -> Result<Option<KataInspect>, RunnerError> {
        let command = self.command(vec![
            OsString::from("inspect"),
            OsString::from(container_name),
        ]);
        let output = self.execute(&command, self.config.command_timeout)?;
        if !output.status.success() {
            let combined = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .to_ascii_lowercase();
            if combined.contains("not found")
                || combined.contains("no such")
                || combined.contains("does not exist")
            {
                return Ok(None);
            }
            return Err(RunnerError::CommandExecution {
                program: command.program.display().to_string(),
                message: format!("container inspect failed: {combined}"),
            });
        }
        let records: Vec<KataInspect> =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                RunnerError::RuntimeLaunch(format!(
                    "nerdctl inspect returned invalid JSON for {container_name}: {error}"
                ))
            })?;
        records.into_iter().next().map(Some).ok_or_else(|| {
            RunnerError::RuntimeLaunch(format!(
                "nerdctl inspect returned no record for {container_name}"
            ))
        })
    }

    fn validate_owned(
        &self,
        plan: &KataLaunchPlan,
        project_id: &str,
        inspected: &KataInspect,
    ) -> Result<(), RunnerError> {
        let expected = [
            ("computer.finite.v2.runtime", "true"),
            (
                "computer.finite.v2.source_host_id",
                self.config.source_host_id.as_str(),
            ),
            (
                "computer.finite.v2.source_machine_id",
                plan.container_name.as_str(),
            ),
            ("computer.finite.v2.project_id", project_id),
        ];
        if expected.iter().any(|(key, value)| {
            inspected.config.labels.get(*key).map(String::as_str) != Some(*value)
        }) {
            return Err(RunnerError::RuntimeLaunch(format!(
                "refusing to manage Kata container {} because its ownership labels do not match",
                plan.container_name
            )));
        }
        Ok(())
    }

    fn remove_compute(&self, container_name: &str) -> Result<(), RunnerError> {
        self.run_checked(
            self.command(vec![
                OsString::from("rm"),
                OsString::from("--force"),
                OsString::from(container_name),
            ]),
            self.config.command_timeout,
        )?;
        Ok(())
    }

    fn remove_compute_if_present(
        &self,
        plan: &KataLaunchPlan,
        project_id: &str,
    ) -> Result<(), RunnerError> {
        let Some(inspected) = self.inspect(&plan.container_name)? else {
            return Ok(());
        };
        self.validate_owned(plan, project_id, &inspected)?;
        self.remove_compute(&plan.container_name)
    }

    fn host_port(&self, plan: &KataLaunchPlan) -> Result<u16, RunnerError> {
        let raw = self.run_checked(
            self.command(vec![
                OsString::from("port"),
                OsString::from(&plan.container_name),
                OsString::from(format!("{}/tcp", plan.container_port)),
            ]),
            self.config.command_timeout,
        )?;
        for line in raw.lines() {
            if let Some(port) = line.trim().rsplit(':').next()
                && let Ok(port) = port.parse::<u16>()
                && port != 0
            {
                return Ok(port);
            }
        }
        Err(RunnerError::RuntimeLaunch(format!(
            "Kata container {} did not publish its agent HTTP port on loopback",
            plan.container_name
        )))
    }

    fn wait_for_runtime_http(
        &self,
        plan: &KataLaunchPlan,
        host_port: u16,
    ) -> Result<(), RunnerError> {
        wait_for_http_json_ready(
            &plan.health_url(host_port),
            "Kata runtime /healthz",
            self.config.readiness_timeout,
            self.config.readiness_interval,
        )
    }

    fn validate_control(
        &self,
        lease: &RuntimeControlLease,
    ) -> Result<(KataLaunchPlan, KataInspect), RunnerError> {
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
        let plan =
            kata_launch_plan_for_source_machine(&self.config, &lease.request.source_machine_id);
        let inspected = self.inspect(&plan.container_name)?.ok_or_else(|| {
            RunnerError::RuntimeLaunch(format!(
                "owned Kata container {} does not exist",
                plan.container_name
            ))
        })?;
        self.validate_owned(&plan, &lease.runtime.project_id, &inspected)?;
        Ok((plan, inspected))
    }

    fn prepare_plan(&self, plan: &KataLaunchPlan) -> Result<(), RunnerError> {
        std::fs::create_dir_all(&plan.state_root)
            .and_then(|_| std::fs::create_dir_all(&plan.metadata_root))
            .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;
        #[cfg(unix)]
        for path in [&plan.state_root, &plan.metadata_root] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;
        }
        Ok(())
    }

    fn write_launch_environment(
        &self,
        plan: &KataLaunchPlan,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<(), RunnerError> {
        let env = kata_runtime_env(&self.config, plan, lease, options);
        write_kata_env_file(&plan.env_file, &env)
    }

    fn run_fresh(
        &self,
        plan: &KataLaunchPlan,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<u16, RunnerError> {
        self.prepare_plan(plan)?;
        self.remove_compute_if_present(plan, &lease.project.id)?;
        self.write_launch_environment(plan, lease, options)?;
        let launch_result = self.run_checked(
            kata_run_command(&self.config, plan, lease),
            self.config.launch_timeout,
        );
        let remove_env_result = std::fs::remove_file(&plan.env_file);
        if let Err(error) = launch_result {
            let _ = self.remove_compute(&plan.container_name);
            return Err(error);
        }
        if let Err(error) = remove_env_result {
            let _ = self.remove_compute(&plan.container_name);
            return Err(RunnerError::RuntimeLaunch(format!(
                "failed to remove the transient Kata environment file: {error}"
            )));
        }
        let host_port = match self.host_port(plan) {
            Ok(host_port) => host_port,
            Err(error) => {
                let _ = self.remove_compute(&plan.container_name);
                return Err(error);
            }
        };
        if let Err(error) = self.wait_for_runtime_http(plan, host_port) {
            let _ = self.remove_compute(&plan.container_name);
            return Err(error);
        }
        Ok(host_port)
    }
}

impl RuntimeLauncher for KataLauncher {
    fn runner_class(&self) -> RunnerClass {
        RunnerClass::Kata
    }

    fn validate_ready(&self) -> Result<(), RunnerError> {
        self.config.validate()?;
        self.run_checked(
            self.command(vec![OsString::from("info")]),
            self.config.command_timeout,
        )?;

        let command = PlannedCommand {
            program: self.config.kata_runtime_bin.clone(),
            cwd: None,
            args: vec![OsString::from("--version")],
            env: Vec::new(),
        };
        let output = self.execute(&command, self.config.command_timeout)?;
        if !output.status.success() {
            return Err(RunnerError::CommandExecution {
                program: command.program.display().to_string(),
                message: "Kata runtime version preflight failed".to_string(),
            });
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
            active_sandbox_count: active_kata_container_count(&self.config),
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
        let (plan, _) = self.validate_control(lease)?;
        self.run_checked(
            self.command(vec![
                OsString::from("restart"),
                OsString::from("--time"),
                OsString::from(self.config.stop_timeout_secs.to_string()),
                OsString::from(&plan.container_name),
            ]),
            self.config
                .command_timeout
                .max(Duration::from_secs(self.config.stop_timeout_secs + 5)),
        )?;
        let host_port = self.host_port(&plan)?;
        self.wait_for_runtime_http(&plan, host_port)
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
        let (plan, inspected) = self.validate_control(lease)?;
        if inspected.state.status == "running" {
            self.run_checked(
                self.command(vec![
                    OsString::from("stop"),
                    OsString::from("--time"),
                    OsString::from(self.config.stop_timeout_secs.to_string()),
                    OsString::from(&plan.container_name),
                ]),
                self.config
                    .command_timeout
                    .max(Duration::from_secs(self.config.stop_timeout_secs + 5)),
            )?;
        }
        Ok(())
    }

    fn destroy_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        self.validate_ready()?;
        let (plan, _) = self.validate_control(lease)?;
        // Destroy is deliberately compute-only. The durable state root is a
        // separate recovery boundary and must survive every runtime lifecycle
        // operation until an explicit, separately authorized data purge exists.
        self.remove_compute(&plan.container_name)
    }

    fn launch(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        self.validate_ready()?;
        let plan = self.plan_launch(lease);
        let host_port = self.run_fresh(&plan, lease, options)?;
        let runtime_bootstrap_token = random_runtime_bootstrap_token();
        let runtime_relay_token_hash = hash_runtime_relay_token(&runtime_bootstrap_token)
            .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;
        let public_base_url = plan.public_base_url(host_port);

        Ok(RuntimeLaunchFacts {
            source_host_id: self.config.source_host_id.clone(),
            source_machine_id: plan.container_name.clone(),
            runtime_artifact_id: self.config.runtime_artifact_id.clone(),
            state_schema_version: self.config.runtime_state_schema_version.clone(),
            runtime_relay_token_hash,
            display_name: Some(lease.project.display_name.clone()),
            hostname: None,
            runtime_host: Some(public_base_url),
            runtime_status: RuntimeSummaryStatus::Online,
            active_inference_profile: options
                .finite_private
                .as_ref()
                .map(|_| FINITE_PRIVATE_PROFILE_ID.to_string()),
            hermes_available: Some(true),
            published_app_urls: vec![plan.contact_url(host_port)],
        })
    }

    fn cleanup_failed_launch(&mut self, facts: &RuntimeLaunchFacts) -> Result<(), RunnerError> {
        if self.inspect(&facts.source_machine_id)?.is_some() {
            self.remove_compute(&facts.source_machine_id)?;
        }
        Ok(())
    }
}

pub(crate) fn kata_launch_plan(config: &KataConfig, lease: &AgentCreationLease) -> KataLaunchPlan {
    let request_suffix = lease
        .request
        .id
        .strip_prefix("agent_request_")
        .unwrap_or(&lease.request.id);
    let container_name =
        sanitize_sandbox_name(&format!("{}-{request_suffix}", config.name_prefix.trim()))
            .to_ascii_lowercase();
    kata_launch_plan_for_source_machine(config, &container_name)
}

fn kata_launch_plan_for_source_machine(
    config: &KataConfig,
    source_machine_id: &str,
) -> KataLaunchPlan {
    let container_name = sanitize_sandbox_name(source_machine_id).to_ascii_lowercase();
    let metadata_root = config
        .work_root
        .join(KATA_METADATA_DIR)
        .join(&container_name);
    KataLaunchPlan {
        state_root: config
            .work_root
            .join(KATA_PROVIDER_DIR)
            .join(&container_name),
        env_file: metadata_root.join("launch.env"),
        metadata_root,
        container_port: config.container_port,
        container_name,
    }
}

pub(crate) fn kata_run_command(
    config: &KataConfig,
    plan: &KataLaunchPlan,
    lease: &AgentCreationLease,
) -> PlannedCommand {
    let mut args = vec![
        OsString::from("--namespace"),
        OsString::from(config.namespace.trim()),
        OsString::from("run"),
        OsString::from("--detach"),
        OsString::from("--name"),
        OsString::from(&plan.container_name),
        OsString::from("--runtime"),
        OsString::from(config.runtime.trim()),
        OsString::from("--restart"),
        OsString::from("unless-stopped"),
        OsString::from("--publish"),
        OsString::from(format!("127.0.0.1::{}", plan.container_port)),
        OsString::from("--volume"),
        OsString::from(format!("{}:/data", plan.state_root.display())),
        OsString::from("--env-file"),
        plan.env_file.as_os_str().to_owned(),
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
    if let Some(runtime_artifact_id) = config
        .runtime_artifact_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.extend([
            OsString::from("--label"),
            OsString::from(format!(
                "computer.finite.v2.runtime_artifact_id={runtime_artifact_id}"
            )),
        ]);
    }
    if let Some(state_schema_version) = config
        .runtime_state_schema_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.extend([
            OsString::from("--label"),
            OsString::from(format!(
                "computer.finite.v2.state_schema_version={state_schema_version}"
            )),
        ]);
    }
    if let Some(policy) = config
        .pull_policy
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.extend([OsString::from("--pull"), OsString::from(policy)]);
    }
    if let Some(cpus) = config.cpus {
        args.extend([OsString::from("--cpus"), OsString::from(cpus.to_string())]);
    }
    if let Some(memory) = config
        .memory
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.extend([OsString::from("--memory"), OsString::from(memory)]);
    }
    args.push(OsString::from(config.image.trim()));
    PlannedCommand {
        program: config.nerdctl_bin.clone(),
        cwd: None,
        args,
        env: Vec::new(),
    }
}

pub(crate) fn kata_runtime_env(
    config: &KataConfig,
    plan: &KataLaunchPlan,
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

pub(crate) fn write_kata_env_file(
    path: &Path,
    environment: &[(String, String)],
) -> Result<(), RunnerError> {
    let mut rendered = Vec::new();
    for (key, value) in environment {
        if key.chars().any(|ch| matches!(ch, '\n' | '\r' | '\0'))
            || value.chars().any(|ch| matches!(ch, '\n' | '\r' | '\0'))
        {
            return Err(RunnerError::RuntimeLaunch(
                "runtime environment cannot be encoded safely for Kata".to_string(),
            ));
        }
        writeln!(&mut rendered, "{key}={value}")
            .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;
    }
    write_secret_file(path, &rendered)
        .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))
}

fn active_kata_container_count(config: &KataConfig) -> Option<u32> {
    let output = Command::new(&config.nerdctl_bin)
        .args([
            "--namespace",
            config.namespace.trim(),
            "ps",
            "--all",
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

#[derive(Debug, Deserialize)]
struct KataInspect {
    #[serde(rename = "Config")]
    config: KataInspectConfig,
    #[serde(rename = "State")]
    state: KataInspectState,
}

#[derive(Debug, Deserialize)]
struct KataInspectConfig {
    #[serde(rename = "Labels", default)]
    labels: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct KataInspectState {
    #[serde(rename = "Status", default)]
    status: String,
}
