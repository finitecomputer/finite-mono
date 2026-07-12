use super::*;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::process::Output;

const KATA_PROVIDER_DIR: &str = "kata";
const KATA_METADATA_DIR: &str = "kata-metadata";
const DEFAULT_KATA_RUNTIME: &str = "io.containerd.kata.v2";
const RUNTIME_ENVIRONMENT_KEYS_LABEL: &str = "computer.finite.v2.runtime_environment_keys";
const RUNTIME_SECRET_ENVIRONMENT_KEYS_LABEL: &str =
    "computer.finite.v2.runtime_secret_environment_keys";

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
        let child = process
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| RunnerError::CommandExecution {
                program: command.program.display().to_string(),
                message: error.to_string(),
            })?;
        wait_with_captured_output(child, &command.program, timeout)
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
        if !inspected.mounts.iter().any(|mount| {
            mount.destination == Path::new("/data")
                && mount.source == plan.state_root
                && mount.read_write
        }) {
            return Err(RunnerError::RuntimeLaunch(format!(
                "refusing to manage Kata container {} because its durable /data bind does not match {}",
                plan.container_name,
                plan.state_root.display()
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

    fn wait_for_agent_npub(
        &self,
        plan: &KataLaunchPlan,
        host_port: u16,
    ) -> Result<String, RunnerError> {
        let url = plan.contact_url(host_port);
        let started = Instant::now();
        loop {
            let result = ureq::get(&url)
                .timeout(
                    self.config
                        .readiness_interval
                        .max(Duration::from_millis(250)),
                )
                .call()
                .map_err(|error| error.to_string())
                .and_then(|response| {
                    response
                        .into_json::<serde_json::Value>()
                        .map_err(|error| error.to_string())
                })
                .and_then(|value| parse_agent_npub(&value).map_err(|error| error.to_string()));
            match result {
                Ok(npub) => return Ok(npub),
                Err(last_error) if started.elapsed() >= self.config.readiness_timeout => {
                    return Err(RunnerError::RuntimeLaunch(format!(
                        "Kata runtime /contact did not expose its Agent Principal within {}s: {last_error}",
                        self.config.readiness_timeout.as_secs()
                    )));
                }
                Err(_) => thread::sleep(self.config.readiness_interval),
            }
        }
    }

    fn start_compute(&self, container_name: &str) -> Result<(), RunnerError> {
        self.run_checked(
            self.command(vec![
                OsString::from("start"),
                OsString::from(container_name),
            ]),
            self.config.command_timeout,
        )?;
        Ok(())
    }

    fn stop_compute(&self, container_name: &str) -> Result<(), RunnerError> {
        self.run_checked(
            self.command(vec![
                OsString::from("stop"),
                OsString::from("--time"),
                OsString::from(self.config.stop_timeout_secs.to_string()),
                OsString::from(container_name),
            ]),
            self.graceful_stop_command_timeout(),
        )?;
        Ok(())
    }

    fn graceful_stop_command_timeout(&self) -> Duration {
        // `nerdctl --time` is the guest's graceful-stop allowance, not the
        // complete CLI operation budget. Leave the full ordinary command
        // allowance after that grace for containerd/Kata acknowledgement and
        // process teardown; otherwise the outer watchdog can kill nerdctl
        // while the canonical container has already exited.
        Duration::from_secs(self.config.stop_timeout_secs)
            .saturating_add(self.config.command_timeout)
    }

    fn rename_compute(&self, from: &str, to: &str) -> Result<(), RunnerError> {
        self.run_checked(
            self.command(vec![
                OsString::from("rename"),
                OsString::from(from),
                OsString::from(to),
            ]),
            self.config.command_timeout,
        )?;
        Ok(())
    }

    fn validate_upgrade_auxiliary(
        &self,
        inspected: &KataInspect,
        project_id: &str,
        canonical_name: &str,
        request_id: &str,
    ) -> Result<(), RunnerError> {
        let expected = [
            ("computer.finite.v2.runtime", "true"),
            (
                "computer.finite.v2.source_host_id",
                self.config.source_host_id.as_str(),
            ),
            ("computer.finite.v2.source_machine_id", canonical_name),
            ("computer.finite.v2.project_id", project_id),
            ("computer.finite.v2.upgrade_request_id", request_id),
        ];
        if expected.iter().any(|(key, value)| {
            inspected.config.labels.get(*key).map(String::as_str) != Some(*value)
        }) {
            return Err(RunnerError::RuntimeLaunch(
                "refusing to manage a Kata upgrade helper whose ownership labels do not match"
                    .to_string(),
            ));
        }
        Ok(())
    }

    fn restore_previous_compute(
        &self,
        canonical_plan: &KataLaunchPlan,
        old_npub: &str,
    ) -> Result<(), RunnerError> {
        let inspected = self
            .inspect(&canonical_plan.container_name)?
            .ok_or_else(|| {
                RunnerError::RuntimeLaunch(format!(
                    "previous Kata compute {} disappeared during rollback",
                    canonical_plan.container_name
                ))
            })?;
        if inspected.state.status != "running" {
            self.start_compute(&canonical_plan.container_name)?;
        }
        let host_port = self.host_port(canonical_plan)?;
        self.wait_for_runtime_http(canonical_plan, host_port)?;
        let restored_npub = self.wait_for_agent_npub(canonical_plan, host_port)?;
        if restored_npub != old_npub {
            return Err(RunnerError::RuntimeLaunch(
                "previous Kata compute restarted with a different Agent Principal".to_string(),
            ));
        }
        Ok(())
    }

    fn discard_candidate_and_restore(
        &self,
        candidate_name: &str,
        canonical_plan: &KataLaunchPlan,
        old_npub: &str,
    ) -> Result<(), RunnerError> {
        if self.inspect(candidate_name)?.is_some() {
            // `rm --force` must complete before the old Runtime is started;
            // two live containers must never write the same /data bind.
            self.remove_compute(candidate_name)?;
        }
        self.restore_previous_compute(canonical_plan, old_npub)
    }

    fn restore_rollback_after_adopted_target_failure(
        &self,
        canonical_plan: &KataLaunchPlan,
        rollback_name: &str,
        expected_old_npub: Option<&str>,
    ) -> Result<(), RunnerError> {
        // The target canonical must be removed before the old handle is made
        // canonical and started: both bind the same durable /data read-write.
        self.remove_compute(&canonical_plan.container_name)?;
        self.rename_compute(rollback_name, &canonical_plan.container_name)?;
        match expected_old_npub {
            Some(expected) => self.restore_previous_compute(canonical_plan, expected),
            None => {
                self.start_compute(&canonical_plan.container_name)?;
                let host_port = self.host_port(canonical_plan)?;
                self.wait_for_runtime_http(canonical_plan, host_port)?;
                self.wait_for_agent_npub(canonical_plan, host_port)
                    .map(|_| ())
            }
        }
    }

    fn validate_upgrade_data_mount(
        &self,
        inspected: &KataInspect,
        canonical_plan: &KataLaunchPlan,
    ) -> Result<(), RunnerError> {
        if inspected.mounts.iter().any(|mount| {
            mount.destination == Path::new("/data")
                && mount.source == canonical_plan.state_root
                && mount.read_write
        }) {
            return Ok(());
        }
        Err(RunnerError::RuntimeLaunch(
            "refusing to reconcile a Kata upgrade helper with a mismatched /data bind".to_string(),
        ))
    }

    /// Reconcile only operation-scoped provider handles before requiring the
    /// canonical name. A process can die after either rename syscall, so the
    /// absence of the canonical handle is a recoverable state when the stopped
    /// old container and candidate both prove ownership of this exact request.
    fn reconcile_interrupted_upgrade(
        &self,
        canonical_plan: &KataLaunchPlan,
        project_id: &str,
        request_id: &str,
        target: &RuntimeArtifact,
    ) -> Result<(), RunnerError> {
        let canonical_name = canonical_plan.container_name.as_str();
        let candidate_name = kata_upgrade_helper_name(canonical_name, "candidate", request_id);
        let rollback_name = kata_upgrade_helper_name(canonical_name, "rollback", request_id);
        let canonical = self.inspect(canonical_name)?;
        let candidate = self.inspect(&candidate_name)?;
        let rollback = self.inspect(&rollback_name)?;

        if let Some(candidate) = candidate.as_ref() {
            self.validate_upgrade_auxiliary(candidate, project_id, canonical_name, request_id)?;
            self.validate_upgrade_data_mount(candidate, canonical_plan)?;
        }
        if let Some(rollback) = rollback.as_ref() {
            // The old canonical keeps its original ownership labels across a
            // rename; it deliberately does not gain an upgrade-request label.
            self.validate_owned(canonical_plan, project_id, rollback)?;
        }

        match canonical {
            None => {
                let Some(_rollback) = rollback else {
                    return Err(RunnerError::RuntimeLaunch(format!(
                        "Kata canonical handle {canonical_name} is missing and no owned rollback handle exists"
                    )));
                };
                if candidate.is_some() {
                    // Candidate must be gone before the old compute can be
                    // restored: never permit two writers on the same /data.
                    self.remove_compute(&candidate_name)?;
                }
                self.rename_compute(&rollback_name, canonical_name)?;
            }
            Some(canonical) => {
                self.validate_owned(canonical_plan, project_id, &canonical)?;
                if rollback.is_some() {
                    let canonical_is_target = canonical.config.image == target.reference
                        && canonical
                            .config
                            .labels
                            .get("computer.finite.v2.runtime_artifact_id")
                            .map(String::as_str)
                            == Some(target.id.as_str());
                    if !canonical_is_target {
                        return Err(RunnerError::RuntimeLaunch(
                            "refusing an ambiguous Kata upgrade topology with both old canonical and rollback handles"
                                .to_string(),
                        ));
                    }
                    // This topology is reachable only after the verified
                    // candidate->canonical rename. Bind it to this operation,
                    // then the normal idempotent target path can health-check
                    // it and retire the stopped old handle.
                    self.validate_upgrade_auxiliary(
                        &canonical,
                        project_id,
                        canonical_name,
                        request_id,
                    )?;
                }
            }
        }
        Ok(())
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
            kata_run_command(&self.config, plan, lease, options),
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
            self.graceful_stop_command_timeout(),
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

    fn upgrade_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        options: &RuntimeRestartOptions,
    ) -> Result<RuntimeUpgradeFacts, RunnerError> {
        self.validate_ready()?;
        let target = lease.target_runtime_artifact.as_ref().ok_or_else(|| {
            RunnerError::RuntimeLaunch(
                "Core did not bind a target artifact to the runtime upgrade lease".to_string(),
            )
        })?;
        if lease.request.target_runtime_artifact_id.as_deref() != Some(target.id.as_str())
            || target.kind != RuntimeArtifactKind::OciImage
            || lease.runtime.state_schema_version.as_deref()
                != Some(target.state_schema_version.as_str())
        {
            return Err(RunnerError::RuntimeLaunch(
                "Core-bound runtime upgrade target is incompatible with this Kata Runtime"
                    .to_string(),
            ));
        }

        let canonical_plan =
            kata_launch_plan_for_source_machine(&self.config, &lease.request.source_machine_id);
        self.reconcile_interrupted_upgrade(
            &canonical_plan,
            &lease.runtime.project_id,
            &lease.request.id,
            target,
        )?;
        let (canonical_plan, mut inspected) = self.validate_control(lease)?;
        self.prepare_plan(&canonical_plan)?;
        let canonical_name = canonical_plan.container_name.clone();
        let candidate_name =
            kata_upgrade_helper_name(&canonical_name, "candidate", &lease.request.id);
        let rollback_name =
            kata_upgrade_helper_name(&canonical_name, "rollback", &lease.request.id);

        // Completion can be retried after Core or Runner restarts. If the
        // canonical handle already names the exact target image, verify it and
        // return the actual endpoint facts without replacing it again.
        if inspected.config.image == target.reference
            && inspected
                .config
                .labels
                .get("computer.finite.v2.runtime_artifact_id")
                .map(String::as_str)
                == Some(target.id.as_str())
        {
            let rollback = self.inspect(&rollback_name)?;
            if let Some(rollback) = rollback.as_ref() {
                self.validate_owned(&canonical_plan, &lease.runtime.project_id, rollback)?;
            }
            let expected_old_npub = if rollback.is_some() {
                Some(read_kata_upgrade_expected_npub(
                    &canonical_plan,
                    &lease.request.id,
                ))
            } else {
                None
            };
            let verification = (|| {
                let host_port = self.host_port(&canonical_plan)?;
                self.wait_for_runtime_http(&canonical_plan, host_port)?;
                let actual_npub = self.wait_for_agent_npub(&canonical_plan, host_port)?;
                if let Some(Ok(expected)) = expected_old_npub.as_ref()
                    && actual_npub != *expected
                {
                    return Err(RunnerError::RuntimeLaunch(
                        "interrupted Kata upgrade changed Agent Principal".to_string(),
                    ));
                }
                if let Some(Err(error)) = expected_old_npub.as_ref() {
                    return Err(RunnerError::RuntimeLaunch(error.to_string()));
                }
                Ok(host_port)
            })();
            let host_port = match verification {
                Ok(host_port) => host_port,
                Err(error) if rollback.is_some() => {
                    let restore = self.restore_rollback_after_adopted_target_failure(
                        &canonical_plan,
                        &rollback_name,
                        expected_old_npub
                            .as_ref()
                            .and_then(|result| result.as_ref().ok())
                            .map(String::as_str),
                    );
                    remove_kata_upgrade_expected_npub(&canonical_plan, &lease.request.id);
                    return Err(runtime_upgrade_failure(error, restore.err()));
                }
                Err(error) => return Err(error),
            };
            if rollback.is_some() {
                self.remove_compute(&rollback_name)?;
                remove_kata_upgrade_expected_npub(&canonical_plan, &lease.request.id);
            }
            return Ok(RuntimeUpgradeFacts {
                runtime_artifact_id: target.id.clone(),
                state_schema_version: target.state_schema_version.clone(),
                runtime_host: canonical_plan.public_base_url(host_port),
                published_app_urls: vec![canonical_plan.contact_url(host_port)],
            });
        }

        // A candidate from an interrupted pre-swap attempt is never adopted
        // blindly. The canonical old Runtime remains authoritative, so remove
        // only a helper whose operation-scoped labels and /data bind match.
        let candidate_plan =
            kata_upgrade_plan(&canonical_plan, candidate_name.clone(), &lease.request.id);
        if let Some(candidate) = self.inspect(&candidate_name)? {
            self.validate_upgrade_auxiliary(
                &candidate,
                &lease.runtime.project_id,
                &canonical_name,
                &lease.request.id,
            )?;
            if !candidate.mounts.iter().any(|mount| {
                mount.destination == Path::new("/data")
                    && mount.source == canonical_plan.state_root
                    && mount.read_write
            }) {
                return Err(RunnerError::RuntimeLaunch(
                    "refusing to remove a Kata upgrade candidate with a mismatched /data bind"
                        .to_string(),
                ));
            }
            self.remove_compute(&candidate_name)?;
        }

        if inspected.state.status != "running" {
            self.start_compute(&canonical_name)?;
            inspected = self.inspect(&canonical_name)?.ok_or_else(|| {
                RunnerError::RuntimeLaunch(
                    "owned Kata Runtime disappeared before upgrade".to_string(),
                )
            })?;
        }
        let old_host_port = self.host_port(&canonical_plan)?;
        let old_npub = self.wait_for_agent_npub(&canonical_plan, old_host_port)?;
        // Persist before the first destructive provider operation. A retry
        // after candidate->canonical can then prove the target still exposes
        // the pre-upgrade Agent Principal before deleting the old handle.
        write_kata_upgrade_expected_npub(&canonical_plan, &lease.request.id, &old_npub)?;
        let replacement_environment = kata_upgrade_environment(&inspected.config, options)?;

        // Pull before user-visible downtime. The candidate run below uses
        // --pull=never, proving it starts the exact Core-bound artifact already
        // present on the host.
        self.run_checked(
            self.command(vec![
                OsString::from("pull"),
                OsString::from("--quiet"),
                OsString::from(target.reference.trim()),
            ]),
            self.config.launch_timeout,
        )?;
        write_kata_env_file(&candidate_plan.env_file, &replacement_environment.entries)?;
        if let Err(error) = self.stop_compute(&canonical_name) {
            let _ = std::fs::remove_file(&candidate_plan.env_file);
            let restore = self.restore_previous_compute(&canonical_plan, &old_npub);
            return Err(runtime_upgrade_failure(error, restore.err()));
        }

        let candidate_launch = self.run_checked(
            kata_upgrade_run_command(
                &self.config,
                &candidate_plan,
                &canonical_name,
                &lease.runtime.project_id,
                &lease.request.id,
                target,
                &replacement_environment,
            ),
            self.config.launch_timeout,
        );
        let remove_env = std::fs::remove_file(&candidate_plan.env_file);
        if let Err(error) = candidate_launch {
            let rollback =
                self.discard_candidate_and_restore(&candidate_name, &canonical_plan, &old_npub);
            return Err(runtime_upgrade_failure(error, rollback.err()));
        }
        if let Err(error) = remove_env {
            let rollback =
                self.discard_candidate_and_restore(&candidate_name, &canonical_plan, &old_npub);
            return Err(runtime_upgrade_failure(
                RunnerError::RuntimeLaunch(format!(
                    "failed to remove transient Kata upgrade environment: {error}"
                )),
                rollback.err(),
            ));
        }

        let candidate_host_port = match self.host_port(&candidate_plan) {
            Ok(port) => port,
            Err(error) => {
                let rollback =
                    self.discard_candidate_and_restore(&candidate_name, &canonical_plan, &old_npub);
                return Err(runtime_upgrade_failure(error, rollback.err()));
            }
        };
        let candidate_ready = self
            .wait_for_runtime_http(&candidate_plan, candidate_host_port)
            .and_then(|()| self.wait_for_agent_npub(&candidate_plan, candidate_host_port));
        match candidate_ready {
            Ok(candidate_npub) if candidate_npub == old_npub => {}
            Ok(_) => {
                let rollback =
                    self.discard_candidate_and_restore(&candidate_name, &canonical_plan, &old_npub);
                return Err(runtime_upgrade_failure(
                    RunnerError::RuntimeLaunch(
                        "Kata upgrade candidate exposed a different Agent Principal".to_string(),
                    ),
                    rollback.err(),
                ));
            }
            Err(error) => {
                let rollback =
                    self.discard_candidate_and_restore(&candidate_name, &canonical_plan, &old_npub);
                return Err(runtime_upgrade_failure(error, rollback.err()));
            }
        }

        // Keep the old container intact and stopped until the target image has
        // passed both readiness and identity checks. Renaming is the provider
        // handle swap; no Core destroy/offboarding path is involved.
        if let Err(error) = self.rename_compute(&canonical_name, &rollback_name) {
            let rollback =
                self.discard_candidate_and_restore(&candidate_name, &canonical_plan, &old_npub);
            return Err(runtime_upgrade_failure(error, rollback.err()));
        }
        if let Err(error) = self.rename_compute(&candidate_name, &canonical_name) {
            let restore_name = self.rename_compute(&rollback_name, &canonical_name);
            let restore = restore_name.and_then(|()| {
                self.discard_candidate_and_restore(&candidate_name, &canonical_plan, &old_npub)
            });
            return Err(runtime_upgrade_failure(error, restore.err()));
        }

        let post_swap = (|| {
            let (plan, replacement) = self.validate_control(lease)?;
            if replacement.config.image != target.reference
                || replacement
                    .config
                    .labels
                    .get("computer.finite.v2.runtime_artifact_id")
                    .map(String::as_str)
                    != Some(target.id.as_str())
            {
                return Err(RunnerError::RuntimeLaunch(
                    "Kata canonical handle did not resolve to the requested upgrade artifact"
                        .to_string(),
                ));
            }
            let host_port = self.host_port(&plan)?;
            self.wait_for_runtime_http(&plan, host_port)?;
            let npub = self.wait_for_agent_npub(&plan, host_port)?;
            if npub != old_npub {
                return Err(RunnerError::RuntimeLaunch(
                    "Kata canonical handle changed Agent Principal after upgrade".to_string(),
                ));
            }
            Ok((plan, host_port))
        })();

        let (upgraded_plan, upgraded_host_port) = match post_swap {
            Ok(result) => result,
            Err(error) => {
                let _ = self.stop_compute(&canonical_name);
                let move_target = self.rename_compute(&canonical_name, &candidate_name);
                let move_old =
                    move_target.and_then(|()| self.rename_compute(&rollback_name, &canonical_name));
                let restore = move_old.and_then(|()| {
                    self.discard_candidate_and_restore(&candidate_name, &canonical_plan, &old_npub)
                });
                return Err(runtime_upgrade_failure(error, restore.err()));
            }
        };

        // Availability wins over cleanup: a stopped old container is not part
        // of the durable Recovery Set. If cleanup alone fails, keep the healthy
        // target canonical and surface an operator warning rather than rolling
        // user compute backward after the verified swap.
        if let Err(error) = self.remove_compute(&rollback_name) {
            eprintln!(
                "warning: Kata runtime upgrade succeeded but old compute cleanup failed: {error}"
            );
        } else {
            remove_kata_upgrade_expected_npub(&canonical_plan, &lease.request.id);
        }

        Ok(RuntimeUpgradeFacts {
            runtime_artifact_id: target.id.clone(),
            state_schema_version: target.state_schema_version.clone(),
            runtime_host: upgraded_plan.public_base_url(upgraded_host_port),
            published_app_urls: vec![upgraded_plan.contact_url(upgraded_host_port)],
        })
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
                self.graceful_stop_command_timeout(),
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
    options: &RuntimeLaunchOptions,
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
    append_runtime_environment_contract_labels(
        &mut args,
        options.environment.keys(),
        options.secret_environment.keys(),
    );
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

fn append_runtime_environment_contract_labels<'a>(
    args: &mut Vec<OsString>,
    public_keys: impl IntoIterator<Item = &'a String>,
    secret_keys: impl IntoIterator<Item = &'a String>,
) {
    for (label, keys) in [
        (
            RUNTIME_ENVIRONMENT_KEYS_LABEL,
            public_keys.into_iter().cloned().collect::<Vec<_>>(),
        ),
        (
            RUNTIME_SECRET_ENVIRONMENT_KEYS_LABEL,
            secret_keys.into_iter().cloned().collect::<Vec<_>>(),
        ),
    ] {
        if !keys.is_empty() {
            args.extend([
                OsString::from("--label"),
                OsString::from(format!("{label}={}", keys.join(","))),
            ]);
        }
    }
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

fn parse_agent_npub(value: &serde_json::Value) -> Result<String, RunnerError> {
    value
        .get("agent_npub")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|npub| npub.starts_with("npub1") && npub.len() <= 256)
        .map(str::to_string)
        .ok_or_else(|| {
            RunnerError::RuntimeLaunch(
                "Kata runtime /contact did not contain a valid Agent Principal".to_string(),
            )
        })
}

fn kata_inspected_environment(entries: &[String]) -> Result<Vec<(String, String)>, RunnerError> {
    let mut environment = Vec::with_capacity(entries.len());
    for entry in entries {
        let (key, value) = entry.split_once('=').ok_or_else(|| {
            RunnerError::RuntimeLaunch(
                "Kata inspect returned a malformed environment entry".to_string(),
            )
        })?;
        if key.is_empty() {
            return Err(RunnerError::RuntimeLaunch(
                "Kata inspect returned an empty environment key".to_string(),
            ));
        }
        // HOSTNAME is generated by the container runtime. Carrying the old
        // sandbox id into replacement compute would create false identity.
        if key != "HOSTNAME" {
            environment.push((key.to_string(), value.to_string()));
        }
    }
    Ok(environment)
}

struct KataUpgradeEnvironment {
    entries: Vec<(String, String)>,
    public_keys: BTreeSet<String>,
    secret_keys: BTreeSet<String>,
}

fn runtime_environment_label_keys(
    labels: &BTreeMap<String, String>,
    label: &str,
) -> BTreeSet<String> {
    labels
        .get(label)
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|key| valid_runtime_environment_key(key))
        .map(str::to_string)
        .collect()
}

fn kata_upgrade_environment(
    inspected: &KataInspectConfig,
    options: &RuntimeRestartOptions,
) -> Result<KataUpgradeEnvironment, RunnerError> {
    let mut public_keys =
        runtime_environment_label_keys(&inspected.labels, RUNTIME_ENVIRONMENT_KEYS_LABEL);
    let mut secret_keys =
        runtime_environment_label_keys(&inspected.labels, RUNTIME_SECRET_ENVIRONMENT_KEYS_LABEL);
    let existing = kata_inspected_environment(&inspected.environment)?;
    let retained = existing
        .into_iter()
        .filter(|(key, _)| {
            let secret = secret_runtime_environment_key(key);
            if secret {
                secret_keys.insert(key.clone());
            }
            reserved_runtime_environment_key(key)
                || secret
                || public_keys.contains(key)
                || secret_keys.contains(key)
        })
        .collect();
    public_keys.extend(options.environment().keys().cloned());
    let entries = merge_desired_runtime_environment(retained, options);
    Ok(KataUpgradeEnvironment {
        entries,
        public_keys,
        secret_keys,
    })
}

fn runtime_upgrade_failure(error: RunnerError, rollback: Option<RunnerError>) -> RunnerError {
    match rollback {
        Some(rollback) => RunnerError::RuntimeLaunch(format!(
            "runtime upgrade failed ({error}); restoring the previous compute also failed ({rollback})"
        )),
        None => error,
    }
}

fn kata_upgrade_helper_name(canonical_name: &str, role: &str, request_id: &str) -> String {
    let suffix = request_id
        .strip_prefix("runtime_ctl_")
        .unwrap_or(request_id);
    let suffix = suffix
        .chars()
        .rev()
        .take(10)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let reserved = role.len() + suffix.len() + 2;
    let base_len = 63usize.saturating_sub(reserved).min(canonical_name.len());
    sanitize_sandbox_name(&format!("{}-{role}-{suffix}", &canonical_name[..base_len]))
        .to_ascii_lowercase()
}

fn kata_upgrade_plan(
    canonical: &KataLaunchPlan,
    container_name: String,
    request_id: &str,
) -> KataLaunchPlan {
    KataLaunchPlan {
        container_name,
        state_root: canonical.state_root.clone(),
        metadata_root: canonical.metadata_root.clone(),
        env_file: canonical
            .metadata_root
            .join(format!("upgrade-{}.env", sanitize_sandbox_name(request_id))),
        container_port: canonical.container_port,
    }
}

fn kata_upgrade_expected_npub_path(canonical: &KataLaunchPlan, request_id: &str) -> PathBuf {
    canonical.metadata_root.join(format!(
        "upgrade-{}.expected-npub",
        sanitize_sandbox_name(request_id)
    ))
}

fn write_kata_upgrade_expected_npub(
    canonical: &KataLaunchPlan,
    request_id: &str,
    npub: &str,
) -> Result<(), RunnerError> {
    write_secret_file(
        &kata_upgrade_expected_npub_path(canonical, request_id),
        npub.as_bytes(),
    )
    .map_err(|error| {
        RunnerError::RuntimeLaunch(format!(
            "failed to persist the pre-upgrade Agent Principal: {error}"
        ))
    })
}

fn read_kata_upgrade_expected_npub(
    canonical: &KataLaunchPlan,
    request_id: &str,
) -> Result<String, RunnerError> {
    let value = std::fs::read_to_string(kata_upgrade_expected_npub_path(canonical, request_id))
        .map_err(|error| {
            RunnerError::RuntimeLaunch(format!(
                "cannot verify interrupted Kata upgrade identity: {error}"
            ))
        })?;
    let value = value.trim();
    if !value.starts_with("npub1") || value.len() > 256 {
        return Err(RunnerError::RuntimeLaunch(
            "interrupted Kata upgrade stored an invalid Agent Principal".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn remove_kata_upgrade_expected_npub(canonical: &KataLaunchPlan, request_id: &str) {
    let _ = std::fs::remove_file(kata_upgrade_expected_npub_path(canonical, request_id));
}

fn kata_upgrade_run_command(
    config: &KataConfig,
    plan: &KataLaunchPlan,
    canonical_name: &str,
    project_id: &str,
    request_id: &str,
    target: &RuntimeArtifact,
    environment: &KataUpgradeEnvironment,
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
            "computer.finite.v2.source_machine_id={canonical_name}"
        )),
        OsString::from("--label"),
        OsString::from(format!("computer.finite.v2.project_id={project_id}")),
        OsString::from("--label"),
        OsString::from(format!(
            "computer.finite.v2.runtime_artifact_id={}",
            target.id
        )),
        OsString::from("--label"),
        OsString::from(format!(
            "computer.finite.v2.state_schema_version={}",
            target.state_schema_version
        )),
        OsString::from("--label"),
        OsString::from(format!(
            "computer.finite.v2.upgrade_request_id={request_id}"
        )),
    ];
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
    append_runtime_environment_contract_labels(
        &mut args,
        environment.public_keys.iter(),
        environment.secret_keys.iter(),
    );
    // Pull happens before stopping the existing Runtime. The candidate must
    // use exactly the Core-bound immutable reference and cannot consult a tag.
    args.extend([
        OsString::from("--pull"),
        OsString::from("never"),
        OsString::from(target.reference.trim()),
    ]);
    PlannedCommand {
        program: config.nerdctl_bin.clone(),
        cwd: None,
        args,
        env: Vec::new(),
    }
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

#[derive(Deserialize)]
struct KataInspect {
    #[serde(rename = "Config")]
    config: KataInspectConfig,
    #[serde(rename = "State")]
    state: KataInspectState,
    #[serde(rename = "Mounts", default)]
    mounts: Vec<KataInspectMount>,
}

#[derive(Deserialize)]
struct KataInspectConfig {
    #[serde(rename = "Labels", default)]
    labels: BTreeMap<String, String>,
    #[serde(rename = "Image", default)]
    image: String,
    #[serde(rename = "Env", default)]
    environment: Vec<String>,
}

#[derive(Deserialize)]
struct KataInspectState {
    #[serde(rename = "Status", default)]
    status: String,
}

#[derive(Deserialize)]
struct KataInspectMount {
    #[serde(rename = "Source")]
    source: PathBuf,
    #[serde(rename = "Destination")]
    destination: PathBuf,
    #[serde(rename = "RW", default)]
    read_write: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use finite_saas_core::{
        AgentRuntime, HostOwnedRuntimeFacts, RuntimeControlRequest, RuntimeControlRequestStatus,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct TestHttpServer {
        port: u16,
        stop: Arc<AtomicBool>,
        contact_requests: Arc<AtomicUsize>,
        health_requests: Arc<AtomicUsize>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl TestHttpServer {
        fn start(npub: &str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let port = listener.local_addr().unwrap().port();
            let stop = Arc::new(AtomicBool::new(false));
            let stop_thread = stop.clone();
            let contact_requests = Arc::new(AtomicUsize::new(0));
            let contact_requests_thread = contact_requests.clone();
            let health_requests = Arc::new(AtomicUsize::new(0));
            let health_requests_thread = health_requests.clone();
            let npub = npub.to_string();
            let thread = std::thread::spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let mut request = [0_u8; 2048];
                            let count = stream.read(&mut request).unwrap_or_default();
                            let request = String::from_utf8_lossy(&request[..count]);
                            let body = if request.contains(" /contact ") {
                                contact_requests_thread.fetch_add(1, Ordering::Relaxed);
                                format!(r#"{{"agent_npub":"{npub}"}}"#)
                            } else {
                                health_requests_thread.fetch_add(1, Ordering::Relaxed);
                                r#"{"ready":true}"#.to_string()
                            };
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = stream.write_all(response.as_bytes());
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                port,
                stop,
                contact_requests,
                health_requests,
                thread: Some(thread),
            }
        }
    }

    impl Drop for TestHttpServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn write_executable(path: &Path, contents: &str) {
        std::fs::write(path, contents).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn fake_nerdctl(root: &Path) -> PathBuf {
        let bin = root.join("nerdctl-fake");
        write_executable(
            &bin,
            r#"#!/bin/sh
set -eu
root="$(dirname "$0")/fake-state"
mkdir -p "$root"
if [ "${1:-}" = "--namespace" ]; then shift 2; fi
cmd="${1:-}"; shift || true
printf '%s %s\n' "$cmd" "$*" >> "$root/commands.log"
field() { cat "$root/$1.$2"; }
write_field() { printf '%s' "$3" > "$root/$1.$2"; }
case "$cmd" in
  info|pull) exit 0 ;;
  inspect)
    name="$1"
    if [ ! -f "$root/$name.image" ]; then echo "not found" >&2; exit 1; fi
    image="$(field "$name" image)"; status="$(field "$name" status)"
    artifact="$(field "$name" artifact)"; schema="$(field "$name" schema)"
    project="$(field "$name" project)"; source="$(field "$name" source)"
    mount="$(field "$name" mount)"; request="$(field "$name" request)"
    printf '[{"Config":{"Labels":{"computer.finite.v2.runtime":"true","computer.finite.v2.source_host_id":"finite-lat-1","computer.finite.v2.source_machine_id":"%s","computer.finite.v2.project_id":"%s","computer.finite.v2.runtime_artifact_id":"%s","computer.finite.v2.state_schema_version":"%s","computer.finite.v2.upgrade_request_id":"%s"},"Image":"%s","Env":["FINITE_PRIVATE_API_KEY=secret-kept","FINITE_SITES_API=https://sites.example","HOSTNAME=old-id"]},"State":{"Status":"%s"},"Mounts":[{"Source":"%s","Destination":"/data","RW":true}]}]\n' "$source" "$project" "$artifact" "$schema" "$request" "$image" "$status" "$mount"
    ;;
  port)
    name="$1"; printf '127.0.0.1:%s\n' "$(field "$name" port)" ;;
  start)
    name="$1"; write_field "$name" status running ;;
  stop)
    for name in "$@"; do :; done
    write_field "$name" status exited
    if [ -f "$root/fail-stop-after-exit" ]; then
      echo "injected stop failure" >&2
      exit 42
    fi
    if [ -f "$root/timeout-stop-after-exit" ]; then
      exec sleep 10
    fi
    ;;
  rm)
    for name in "$@"; do :; done
    rm -f "$root/$name.image" "$root/$name.status" "$root/$name.artifact" "$root/$name.schema" "$root/$name.project" "$root/$name.source" "$root/$name.mount" "$root/$name.request" "$root/$name.port"
    ;;
  rename)
    from="$1"; to="$2"
    for suffix in image status artifact schema project source mount request port; do
      if [ -f "$root/$from.$suffix" ]; then mv "$root/$from.$suffix" "$root/$to.$suffix"; fi
    done
    ;;
  run)
    name=""; volume=""; project=""; source=""; artifact=""; schema=""; request=""; image=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --detach) shift ;;
        --name) name="$2"; shift 2 ;;
        --volume) volume="${2%:/data}"; shift 2 ;;
        --label)
          case "$2" in
            computer.finite.v2.project_id=*) project="${2#*=}" ;;
            computer.finite.v2.source_machine_id=*) source="${2#*=}" ;;
            computer.finite.v2.runtime_artifact_id=*) artifact="${2#*=}" ;;
            computer.finite.v2.state_schema_version=*) schema="${2#*=}" ;;
            computer.finite.v2.upgrade_request_id=*) request="${2#*=}" ;;
          esac
          shift 2 ;;
        --runtime|--restart|--publish|--env-file|--cpus|--memory|--pull) shift 2 ;;
        *) image="$1"; shift ;;
      esac
    done
    write_field "$name" image "$image"; write_field "$name" status running
    write_field "$name" artifact "$artifact"; write_field "$name" schema "$schema"
    write_field "$name" project "$project"; write_field "$name" source "$source"
    write_field "$name" mount "$volume"; write_field "$name" request "$request"
    write_field "$name" port "$(cat "$root/candidate-port")"
    ;;
  *) echo "unsupported fake command: $cmd" >&2; exit 2 ;;
esac
"#,
        );
        bin
    }

    fn target_artifact() -> RuntimeArtifact {
        RuntimeArtifact {
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
            created_at: "2026-07-10T00:00:00Z".to_string(),
            promoted_at: Some("2026-07-10T00:01:00Z".to_string()),
            retired_at: None,
        }
    }

    fn upgrade_lease(request_id: &str) -> RuntimeControlLease {
        RuntimeControlLease {
            request: RuntimeControlRequest {
                id: request_id.to_string(),
                project_id: "project-1".to_string(),
                agent_runtime_id: "runtime-1".to_string(),
                source_host_id: "finite-lat-1".to_string(),
                source_machine_id: "finite-kata-upgrade-agent".to_string(),
                requested_by_user_id: "admin-1".to_string(),
                kind: RuntimeControlKind::Upgrade,
                target_runtime_artifact_id: Some("artifact-v2".to_string()),
                status: RuntimeControlRequestStatus::Running,
                runner_id: Some("kata-runner".to_string()),
                lease_token: Some("lease".to_string()),
                lease_expires_at: None,
                failure_message: None,
                created_at: "2026-07-10T00:00:00Z".to_string(),
                updated_at: "2026-07-10T00:00:00Z".to_string(),
                completed_at: None,
            },
            runtime: AgentRuntime {
                id: "runtime-1".to_string(),
                project_id: "project-1".to_string(),
                source_host_id: "finite-lat-1".to_string(),
                source_machine_id: "finite-kata-upgrade-agent".to_string(),
                source_import_key: "finite-lat-1/finite-kata-upgrade-agent".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                host_facts: HostOwnedRuntimeFacts {
                    display_name: "Upgrade Agent".to_string(),
                    hostname: None,
                    runtime_host: "http://127.0.0.1:1".to_string(),
                    runtime_status: RuntimeSummaryStatus::Online,
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                },
                created_at: "2026-07-10T00:00:00Z".to_string(),
                updated_at: "2026-07-10T00:00:00Z".to_string(),
            },
            target_runtime_artifact: Some(target_artifact()),
        }
    }

    fn write_fake_container(
        state: &Path,
        name: &str,
        image: &str,
        artifact: &str,
        request: &str,
        port: u16,
        mount: &Path,
    ) {
        for (field, value) in [
            ("image", image.to_string()),
            ("status", "running".to_string()),
            ("artifact", artifact.to_string()),
            ("schema", "state-v1".to_string()),
            ("project", "project-1".to_string()),
            ("source", "finite-kata-upgrade-agent".to_string()),
            ("mount", mount.display().to_string()),
            ("request", request.to_string()),
            ("port", port.to_string()),
        ] {
            std::fs::write(state.join(format!("{name}.{field}")), value).unwrap();
        }
    }

    fn test_launcher(
        temp: &tempfile::TempDir,
        candidate_port: u16,
    ) -> (KataLauncher, KataLaunchPlan, PathBuf) {
        let fake_state = temp.path().join("fake-state");
        std::fs::create_dir_all(&fake_state).unwrap();
        std::fs::write(
            fake_state.join("candidate-port"),
            candidate_port.to_string(),
        )
        .unwrap();
        let nerdctl = fake_nerdctl(temp.path());
        let kata_runtime = temp.path().join("kata-runtime-fake");
        write_executable(&kata_runtime, "#!/bin/sh\nexit 0\n");
        let config = KataConfig {
            nerdctl_bin: nerdctl,
            kata_runtime_bin: kata_runtime,
            source_host_id: "finite-lat-1".to_string(),
            image: "unused-global-image".to_string(),
            runtime_artifact_id: Some("unused-global-artifact".to_string()),
            runtime_state_schema_version: Some("state-v1".to_string()),
            work_root: temp.path().join("runner"),
            command_timeout: Duration::from_secs(2),
            launch_timeout: Duration::from_secs(2),
            readiness_timeout: Duration::from_secs(2),
            readiness_interval: Duration::from_millis(10),
            ..KataConfig::default()
        };
        let launcher = KataLauncher::new(config);
        let plan =
            kata_launch_plan_for_source_machine(&launcher.config, "finite-kata-upgrade-agent");
        std::fs::create_dir_all(&plan.state_root).unwrap();
        std::fs::create_dir_all(&plan.metadata_root).unwrap();
        std::fs::write(plan.state_root.join("identity-marker"), "same-agent").unwrap();
        (launcher, plan, fake_state)
    }

    #[test]
    fn kata_upgrade_swaps_exact_image_preserves_data_and_retry_cleans_old_helper() {
        let old_server = TestHttpServer::start("npub1sameagent");
        let candidate_server = TestHttpServer::start("npub1sameagent");
        let temp = tempfile::tempdir().unwrap();
        let (mut launcher, plan, fake_state) = test_launcher(&temp, candidate_server.port);
        write_fake_container(
            &fake_state,
            &plan.container_name,
            "ghcr.io/finitecomputer/agent-runtime:v1@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifact-v1",
            "",
            old_server.port,
            &plan.state_root,
        );
        let lease = upgrade_lease("runtime_ctl_upgrade_success");

        let facts = launcher
            .upgrade_runtime(&lease, &RuntimeRestartOptions::default())
            .unwrap();
        assert_eq!(facts.runtime_artifact_id, "artifact-v2");
        assert_eq!(
            std::fs::read_to_string(plan.state_root.join("identity-marker")).unwrap(),
            "same-agent"
        );
        assert_eq!(
            std::fs::read_to_string(fake_state.join(format!("{}.image", plan.container_name)))
                .unwrap(),
            target_artifact().reference
        );
        let commands_before_retry =
            std::fs::read_to_string(fake_state.join("commands.log")).unwrap();
        assert!(commands_before_retry.contains("pull --quiet"));
        assert!(commands_before_retry.contains("run --detach"));
        assert!(commands_before_retry.matches("rename ").count() >= 2);

        let rollback_name =
            kata_upgrade_helper_name(&plan.container_name, "rollback", &lease.request.id);
        write_fake_container(
            &fake_state,
            &rollback_name,
            "old-image",
            "artifact-v1",
            "",
            old_server.port,
            &plan.state_root,
        );
        write_kata_upgrade_expected_npub(&plan, &lease.request.id, "npub1sameagent").unwrap();
        launcher
            .upgrade_runtime(&lease, &RuntimeRestartOptions::default())
            .unwrap();
        assert!(!fake_state.join(format!("{rollback_name}.image")).exists());
        let commands_after_retry =
            std::fs::read_to_string(fake_state.join("commands.log")).unwrap();
        assert_eq!(
            commands_after_retry.matches("run --detach").count(),
            commands_before_retry.matches("run --detach").count(),
            "exact Config.Image retry must not launch replacement compute again"
        );
    }

    #[test]
    fn kata_stop_outer_timeout_includes_grace_and_full_command_budget() {
        let launcher = KataLauncher::new(KataConfig {
            command_timeout: Duration::from_secs(35),
            stop_timeout_secs: 30,
            ..KataConfig::default()
        });
        assert_eq!(
            launcher.graceful_stop_command_timeout(),
            Duration::from_secs(65)
        );
    }

    #[test]
    fn kata_upgrade_stop_failure_restarts_and_verifies_old_canonical() {
        let old_server = TestHttpServer::start("npub1sameagent");
        let temp = tempfile::tempdir().unwrap();
        let (mut launcher, plan, fake_state) = test_launcher(&temp, old_server.port);
        let old_image = "ghcr.io/finitecomputer/agent-runtime:v1@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        write_fake_container(
            &fake_state,
            &plan.container_name,
            old_image,
            "artifact-v1",
            "",
            old_server.port,
            &plan.state_root,
        );
        std::fs::write(fake_state.join("fail-stop-after-exit"), "1").unwrap();

        let error = launcher
            .upgrade_runtime(
                &upgrade_lease("runtime_ctl_upgrade_stop_failure"),
                &RuntimeRestartOptions::default(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("injected stop failure"));
        assert_eq!(
            std::fs::read_to_string(fake_state.join(format!("{}.status", plan.container_name)))
                .unwrap(),
            "running"
        );
        assert_eq!(
            std::fs::read_to_string(fake_state.join(format!("{}.image", plan.container_name)))
                .unwrap(),
            old_image
        );
        let commands = std::fs::read_to_string(fake_state.join("commands.log")).unwrap();
        let stop = commands
            .find(&format!("stop --time 30 {}", plan.container_name))
            .unwrap();
        let restart = commands
            .find(&format!("start {}", plan.container_name))
            .unwrap();
        assert!(
            stop < restart,
            "old canonical must restart after stop failure"
        );
        assert!(
            !commands.contains("run --detach"),
            "candidate takeover must not begin after stop failure"
        );
        assert!(
            old_server.health_requests.load(Ordering::Relaxed) >= 1,
            "restored canonical must pass health readiness"
        );
        assert!(
            old_server.contact_requests.load(Ordering::Relaxed) >= 2,
            "restored canonical must re-prove the original Agent Principal"
        );
    }

    #[test]
    fn kata_upgrade_stop_timeout_restarts_and_verifies_old_canonical() {
        let old_server = TestHttpServer::start("npub1sameagent");
        let temp = tempfile::tempdir().unwrap();
        let (mut launcher, plan, fake_state) = test_launcher(&temp, old_server.port);
        launcher.config.command_timeout = Duration::from_secs(1);
        launcher.config.stop_timeout_secs = 0;
        let old_image = "ghcr.io/finitecomputer/agent-runtime:v1@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        write_fake_container(
            &fake_state,
            &plan.container_name,
            old_image,
            "artifact-v1",
            "",
            old_server.port,
            &plan.state_root,
        );
        std::fs::write(fake_state.join("timeout-stop-after-exit"), "1").unwrap();

        let error = launcher
            .upgrade_runtime(
                &upgrade_lease("runtime_ctl_upgrade_stop_timeout"),
                &RuntimeRestartOptions::default(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert_eq!(
            std::fs::read_to_string(fake_state.join(format!("{}.status", plan.container_name)))
                .unwrap(),
            "running"
        );
        assert_eq!(
            std::fs::read_to_string(fake_state.join(format!("{}.image", plan.container_name)))
                .unwrap(),
            old_image
        );
        let commands = std::fs::read_to_string(fake_state.join("commands.log")).unwrap();
        let stop = commands
            .find(&format!("stop --time 0 {}", plan.container_name))
            .unwrap();
        let restart = commands
            .find(&format!("start {}", plan.container_name))
            .unwrap();
        assert!(
            stop < restart,
            "old canonical must restart after stop timeout"
        );
        assert!(
            !commands.contains("run --detach"),
            "candidate takeover must not begin after stop timeout"
        );
        assert!(
            old_server.health_requests.load(Ordering::Relaxed) >= 1,
            "restored canonical must pass health readiness"
        );
        assert!(
            old_server.contact_requests.load(Ordering::Relaxed) >= 2,
            "restored canonical must re-prove the original Agent Principal"
        );
    }

    #[test]
    fn kata_upgrade_identity_mismatch_removes_candidate_and_restarts_old_image() {
        let old_server = TestHttpServer::start("npub1sameagent");
        let candidate_server = TestHttpServer::start("npub1differentagent");
        let temp = tempfile::tempdir().unwrap();
        let (mut launcher, plan, fake_state) = test_launcher(&temp, candidate_server.port);
        let old_image = "ghcr.io/finitecomputer/agent-runtime:v1@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        write_fake_container(
            &fake_state,
            &plan.container_name,
            old_image,
            "artifact-v1",
            "",
            old_server.port,
            &plan.state_root,
        );
        let lease = upgrade_lease("runtime_ctl_upgrade_rollback");

        let error = launcher
            .upgrade_runtime(&lease, &RuntimeRestartOptions::default())
            .unwrap_err();
        assert!(error.to_string().contains("different Agent Principal"));
        assert_eq!(
            std::fs::read_to_string(fake_state.join(format!("{}.image", plan.container_name)))
                .unwrap(),
            old_image
        );
        assert_eq!(
            std::fs::read_to_string(fake_state.join(format!("{}.status", plan.container_name)))
                .unwrap(),
            "running"
        );
        let candidate_name =
            kata_upgrade_helper_name(&plan.container_name, "candidate", &lease.request.id);
        assert!(!fake_state.join(format!("{candidate_name}.image")).exists());
        assert_eq!(
            std::fs::read_to_string(plan.state_root.join("identity-marker")).unwrap(),
            "same-agent"
        );
    }

    #[test]
    fn kata_upgrade_plan_uses_same_data_and_never_places_secrets_in_argv() {
        let config = KataConfig {
            source_host_id: "finite-lat-1".to_string(),
            work_root: PathBuf::from("/var/lib/finite-saas-runner"),
            ..KataConfig::default()
        };
        let canonical = kata_launch_plan_for_source_machine(&config, "finite-kata-agent");
        let helper = kata_upgrade_plan(
            &canonical,
            kata_upgrade_helper_name("finite-kata-agent", "candidate", "runtime_ctl_123"),
            "runtime_ctl_123",
        );
        let inspected = KataInspectConfig {
            labels: BTreeMap::from([(
                RUNTIME_ENVIRONMENT_KEYS_LABEL.to_string(),
                "USER_VISIBLE_OVERRIDE".to_string(),
            )]),
            image: "old-image".to_string(),
            environment: vec![
                "FINITE_HOME=/data/agent".to_string(),
                "HERMES_HOME=/data/agent/hermes-home".to_string(),
                "FINITE_PRIVATE_API_KEY=secret-kept".to_string(),
                "USER_VISIBLE_OVERRIDE=keep-me".to_string(),
                "OLD_IMAGE_DEFAULT=must-not-leak".to_string(),
                "HOSTNAME=discard-me".to_string(),
            ],
        };
        let environment =
            kata_upgrade_environment(&inspected, &RuntimeRestartOptions::default()).unwrap();
        assert!(
            environment
                .entries
                .contains(&("FINITE_HOME".to_string(), "/data/agent".to_string()))
        );
        assert!(environment.entries.contains(&(
            "HERMES_HOME".to_string(),
            "/data/agent/hermes-home".to_string()
        )));
        assert!(environment.entries.contains(&(
            "FINITE_PRIVATE_API_KEY".to_string(),
            "secret-kept".to_string()
        )));
        assert!(
            environment
                .entries
                .contains(&("USER_VISIBLE_OVERRIDE".to_string(), "keep-me".to_string()))
        );
        assert!(
            environment
                .entries
                .iter()
                .all(|(key, _)| key != "OLD_IMAGE_DEFAULT" && key != "HOSTNAME")
        );

        let command = kata_upgrade_run_command(
            &config,
            &helper,
            &canonical.container_name,
            "project-1",
            "runtime_ctl_123",
            &target_artifact(),
            &environment,
        );
        let args = command
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| {
            pair == [
                "--volume",
                "/var/lib/finite-saas-runner/kata/finite-kata-agent:/data",
            ]
        }));
        assert!(args.windows(2).any(|pair| pair == ["--pull", "never"]));
        assert_eq!(args.last(), Some(&target_artifact().reference));
        assert!(args.iter().all(|value| !value.contains("secret-kept")));
    }

    #[test]
    fn kata_upgrade_retry_recovers_crash_after_canonical_moved_to_rollback() {
        let old_server = TestHttpServer::start("npub1sameagent");
        let candidate_server = TestHttpServer::start("npub1sameagent");
        let temp = tempfile::tempdir().unwrap();
        let (mut launcher, plan, fake_state) = test_launcher(&temp, candidate_server.port);
        let lease = upgrade_lease("runtime_ctl_crash_after_old_rename");
        let rollback_name =
            kata_upgrade_helper_name(&plan.container_name, "rollback", &lease.request.id);
        let candidate_name =
            kata_upgrade_helper_name(&plan.container_name, "candidate", &lease.request.id);
        write_fake_container(
            &fake_state,
            &rollback_name,
            "ghcr.io/finitecomputer/agent-runtime:v1@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifact-v1",
            "",
            old_server.port,
            &plan.state_root,
        );
        std::fs::write(fake_state.join(format!("{rollback_name}.status")), "exited").unwrap();
        write_fake_container(
            &fake_state,
            &candidate_name,
            &target_artifact().reference,
            "artifact-v2",
            &lease.request.id,
            candidate_server.port,
            &plan.state_root,
        );

        launcher
            .upgrade_runtime(&lease, &RuntimeRestartOptions::default())
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(fake_state.join(format!("{}.image", plan.container_name)))
                .unwrap(),
            target_artifact().reference
        );
        let commands = std::fs::read_to_string(fake_state.join("commands.log")).unwrap();
        let remove = commands
            .find(&format!("rm --force {candidate_name}"))
            .unwrap();
        let restore = commands
            .find(&format!("rename {rollback_name} {}", plan.container_name))
            .unwrap();
        assert!(
            remove < restore,
            "candidate must be removed before old /data writer is restored"
        );
    }

    #[test]
    fn kata_upgrade_retry_finishes_crash_after_candidate_became_canonical() {
        let target_server = TestHttpServer::start("npub1sameagent");
        let old_server = TestHttpServer::start("npub1sameagent");
        let temp = tempfile::tempdir().unwrap();
        let (mut launcher, plan, fake_state) = test_launcher(&temp, target_server.port);
        let lease = upgrade_lease("runtime_ctl_crash_after_target_rename");
        let rollback_name =
            kata_upgrade_helper_name(&plan.container_name, "rollback", &lease.request.id);
        write_fake_container(
            &fake_state,
            &plan.container_name,
            &target_artifact().reference,
            "artifact-v2",
            &lease.request.id,
            target_server.port,
            &plan.state_root,
        );
        write_fake_container(
            &fake_state,
            &rollback_name,
            "ghcr.io/finitecomputer/agent-runtime:v1@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifact-v1",
            "",
            old_server.port,
            &plan.state_root,
        );
        std::fs::write(fake_state.join(format!("{rollback_name}.status")), "exited").unwrap();
        write_kata_upgrade_expected_npub(&plan, &lease.request.id, "npub1sameagent").unwrap();

        launcher
            .upgrade_runtime(&lease, &RuntimeRestartOptions::default())
            .unwrap();
        assert!(!fake_state.join(format!("{rollback_name}.image")).exists());
        let commands = std::fs::read_to_string(fake_state.join("commands.log")).unwrap();
        assert!(!commands.contains("run --detach"));
    }

    #[test]
    fn kata_upgrade_retry_rejects_changed_identity_after_candidate_became_canonical() {
        let target_server = TestHttpServer::start("npub1differentagent");
        let old_server = TestHttpServer::start("npub1sameagent");
        let temp = tempfile::tempdir().unwrap();
        let (mut launcher, plan, fake_state) = test_launcher(&temp, target_server.port);
        let lease = upgrade_lease("runtime_ctl_crash_identity_mismatch");
        let rollback_name =
            kata_upgrade_helper_name(&plan.container_name, "rollback", &lease.request.id);
        let old_image = "ghcr.io/finitecomputer/agent-runtime:v1@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        write_fake_container(
            &fake_state,
            &plan.container_name,
            &target_artifact().reference,
            "artifact-v2",
            &lease.request.id,
            target_server.port,
            &plan.state_root,
        );
        write_fake_container(
            &fake_state,
            &rollback_name,
            old_image,
            "artifact-v1",
            "",
            old_server.port,
            &plan.state_root,
        );
        std::fs::write(fake_state.join(format!("{rollback_name}.status")), "exited").unwrap();
        write_kata_upgrade_expected_npub(&plan, &lease.request.id, "npub1sameagent").unwrap();

        let error = launcher
            .upgrade_runtime(&lease, &RuntimeRestartOptions::default())
            .unwrap_err();
        assert!(error.to_string().contains("changed Agent Principal"));
        assert_eq!(
            std::fs::read_to_string(fake_state.join(format!("{}.image", plan.container_name)))
                .unwrap(),
            old_image
        );
        assert_eq!(
            std::fs::read_to_string(fake_state.join(format!("{}.status", plan.container_name)))
                .unwrap(),
            "running"
        );
    }
}
