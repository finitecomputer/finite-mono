use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as _;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

pub mod workos_fixture;
use workos_fixture::{
    CLIENT_ID as WORKOS_FIXTURE_CLIENT_ID, CUSTOMER_EMAIL as WORKOS_FIXTURE_CUSTOMER_EMAIL,
    CUSTOMER_SUBJECT as WORKOS_FIXTURE_CUSTOMER_SUBJECT, FixturePaths,
    OPERATOR_ORG_ID as WORKOS_FIXTURE_OPERATOR_ORG_ID, prepare as prepare_workos_fixture,
};

#[derive(Debug, Clone, Copy)]
pub enum ProcessComposeMode {
    Tui,
    Headless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedProcess {
    ProcessCompose,
    WorkosFixture,
    RustBuild,
    Postgres,
    Core,
    FiniteChat,
    HostedWebDevice,
    FiniteSites,
    FiniteIdentity,
    FiniteBrain,
    RuntimeImage,
    FinitePrivateLimiter,
    AppleNetworkProbe,
    RuntimeArtifact,
    Runner,
    DashboardDeps,
    Dashboard,
}

impl ManagedProcess {
    const ALL: [Self; 17] = [
        Self::ProcessCompose,
        Self::WorkosFixture,
        Self::RustBuild,
        Self::Postgres,
        Self::Core,
        Self::FiniteChat,
        Self::HostedWebDevice,
        Self::FiniteSites,
        Self::FiniteIdentity,
        Self::FiniteBrain,
        Self::RuntimeImage,
        Self::FinitePrivateLimiter,
        Self::AppleNetworkProbe,
        Self::RuntimeArtifact,
        Self::Runner,
        Self::DashboardDeps,
        Self::Dashboard,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::ProcessCompose => "process-compose",
            Self::WorkosFixture => "workos-fixture",
            Self::RustBuild => "rust-build",
            Self::Postgres => "postgres",
            Self::Core => "core",
            Self::FiniteChat => "finitechat",
            Self::HostedWebDevice => "hosted-web-device",
            Self::FiniteSites => "finitesites",
            Self::FiniteIdentity => "finite-identity",
            Self::FiniteBrain => "finite-brain",
            Self::RuntimeImage => "runtime-image",
            Self::FinitePrivateLimiter => "finite-private-limiter",
            Self::AppleNetworkProbe => "apple-network-probe",
            Self::RuntimeArtifact => "runtime-artifact",
            Self::Runner => "runner",
            Self::DashboardDeps => "dashboard-deps",
            Self::Dashboard => "dashboard",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackProfile {
    AppleSaas,
    ServicesOnly,
}

impl StackProfile {
    fn includes_runtime(self) -> bool {
        matches!(self, Self::AppleSaas)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::AppleSaas => "apple-saas",
            Self::ServicesOnly => "services-only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InferenceMode {
    ChainedLimiter,
    DirectKeyOverride,
    Missing,
}

impl InferenceMode {
    fn from_environment() -> Self {
        if nonempty_env("FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY") {
            Self::ChainedLimiter
        } else if nonempty_env("FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE") {
            Self::DirectKeyOverride
        } else {
            Self::Missing
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppleHostAccess {
    runtime_host: String,
    bind_host: String,
    source: &'static str,
}

impl Default for AppleHostAccess {
    fn default() -> Self {
        Self {
            runtime_host: "host.container.internal".to_string(),
            bind_host: "127.0.0.1".to_string(),
            source: "unverified default",
        }
    }
}

const RUNTIME_ARTIFACT_ID_PREFIX: &str = "devfinity-runtime";
const RUNTIME_IMAGE_REF: &str = "finite-agent-runtime:devfinity";
const RUNNER_ID: &str = "devfinity-apple-runner";
const RUNNER_CLASS: &str = "apple_container";
const RUNNER_SOURCE_HOST_ID: &str = "devfinity-apple";
const DEVFINITY_RUNNER_CREDENTIAL_ID: &str = "devfinity-apple-current";
const DEVFINITY_RUNNER_TOKEN_ENV: &str = "FC_CORE_RUNNER_CREDENTIAL_TOKEN_DEVFINITY_APPLE_CURRENT";
const DEVFINITY_RUNNER_TOKEN: &str = "devfinity-runner-route-token";
const DEVFINITY_USAGE_TOKEN: &str = "devfinity-finite-private-usage-token";
const MACOS_UNIX_SOCKET_PATH_MAX: usize = 103;

fn devfinity_runner_credentials_json() -> String {
    serde_json::json!([{
        "credentialId": DEVFINITY_RUNNER_CREDENTIAL_ID,
        "tokenEnv": DEVFINITY_RUNNER_TOKEN_ENV,
        "runnerId": RUNNER_ID,
        "runnerClasses": [RUNNER_CLASS],
        "sourceHostId": RUNNER_SOURCE_HOST_ID,
        "revoked": false,
    }])
    .to_string()
}

impl std::fmt::Display for ManagedProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Stack {
    repo_root: PathBuf,
    state_dir: PathBuf,
    run_dir: PathBuf,
    logs_dir: PathBuf,
    pids_dir: PathBuf,
    process_compose_file: PathBuf,
    process_compose_control_dir: PathBuf,
    process_compose_socket: PathBuf,
    ports: Ports,
    core_token: String,
    hosted_web_device_token: String,
    sites_viewer_session_token: String,
    profile: StackProfile,
    fresh_services_state: bool,
    inference_mode: InferenceMode,
    apple_host_access: AppleHostAccess,
    apple_container_name_prefix: String,
}

#[derive(Debug, Clone)]
struct Ports {
    core: u16,
    dashboard: u16,
    postgres: u16,
    finitechat: u16,
    hosted_web_device: u16,
    finitesites: u16,
    finite_identity: u16,
    finite_brain: u16,
    finite_private_limiter: u16,
    workos_fixture: u16,
    runtime_agent: u16,
}

impl Stack {
    pub fn new(state_dir: PathBuf) -> Result<Self> {
        let repo_root = std::env::current_dir().context("failed to read current directory")?;
        let state_dir = absolute_path(&repo_root, &state_dir);
        let run_dir = state_dir.join("runs").join("default");
        let logs_dir = run_dir.join("logs");
        let pids_dir = run_dir.join("pids");
        let process_compose_control_dir = process_compose_control_dir(&run_dir);
        let runtime_agent_port = optional_env_u16("DEVFINITY_RUNTIME_AGENT_PORT", 18080)?;
        let apple_container_name_prefix = std::env::var("DEVFINITY_APPLE_CONTAINER_NAME_PREFIX")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "finite-devfinity".to_string());
        Ok(Self {
            repo_root,
            process_compose_file: run_dir.join("process-compose.yaml"),
            process_compose_socket: process_compose_control_dir.join("pc.sock"),
            process_compose_control_dir,
            state_dir,
            run_dir,
            logs_dir,
            pids_dir,
            ports: Ports {
                core: 14200,
                dashboard: 13002,
                postgres: 15432,
                finitechat: 18787,
                hosted_web_device: 38918,
                finitesites: 18789,
                finite_identity: 18788,
                finite_brain: 18790,
                finite_private_limiter: 18002,
                workos_fixture: 14199,
                runtime_agent: runtime_agent_port,
            },
            core_token: "devfinity-core-service-token".to_string(),
            hosted_web_device_token: "devfinity-hosted-web-device-token".to_string(),
            sites_viewer_session_token:
                "dededededededededededededededededededededededededededededededede".to_string(),
            profile: StackProfile::AppleSaas,
            fresh_services_state: false,
            inference_mode: InferenceMode::from_environment(),
            apple_host_access: AppleHostAccess::default(),
            apple_container_name_prefix,
        })
    }

    pub fn with_profile(mut self, profile: StackProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn with_fresh_services_state(mut self, fresh: bool) -> Self {
        self.fresh_services_state = fresh;
        self
    }

    /// Prepare the host-only prerequisites needed to generate an accurate
    /// Apple Container stack. This never installs software and never invokes
    /// sudo. The official host DNS bridge remains an explicit developer choice;
    /// when it is absent we derive the vmnet gateway that Apple assigned.
    pub fn prepare_host_environment(&mut self, dry_run: bool) -> Result<()> {
        if !self.profile.includes_runtime() {
            if self.fresh_services_state {
                return Ok(());
            }
            return Ok(());
        }
        if self.fresh_services_state {
            bail!("--fresh is limited to the isolated services-only smoke profile");
        }
        if std::env::consts::OS != "macos" || std::env::consts::ARCH != "aarch64" {
            bail!(
                "the default devfinity SaaS profile requires Apple silicon and macOS 26; use --services-only for the portable service profile"
            );
        }
        ensure_apple_container_cli()?;

        if dry_run {
            if !apple_container_system_running()? {
                bail!(
                    "Apple Container services are stopped; run `container system start` before --dry-run (devfinity starts them automatically for a real run)"
                );
            }
        } else {
            run_required(
                Command::new("container").args(["system", "start"]),
                "start Apple Container services",
            )?;
            if !apple_container_system_running()? {
                bail!(
                    "Apple Container services did not report running after `container system start`"
                );
            }
        }

        self.apple_host_access = detect_apple_host_access()?;

        if !dry_run {
            if self.inference_mode == InferenceMode::Missing {
                bail!(
                    "chat-capable local SaaS requires inference credentials. Set FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY (preferred: real local admission and per-runtime keys), or explicitly set FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE. Secrets are inherited by the relevant process and are never written to devfinity config or logs"
                );
            }
            // Apple Container 1.1 reports `builder is not running` with exit 0,
            // while `builder start` itself is idempotent. Invoke the operation
            // directly instead of inferring state from the exit code.
            run_required(
                Command::new("container")
                    .args(["builder", "start", "--cpus", "8", "--memory", "8G"]),
                "start the Apple Container image builder",
            )?;
        }
        Ok(())
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            &self.state_dir,
            &self.run_dir,
            &self.logs_dir,
            &self.pids_dir,
            &self.postgres_dir(),
            &self.core_dir(),
            &self.dashboard_dir(),
            &self.finitechat_dir(),
            &self.hosted_web_device_dir(),
            &self.finitesites_dir(),
            &self.finite_identity_dir(),
            &self.finite_brain_dir(),
            &self.finite_home_dir(),
            &self.runtime_image_dir(),
            &self.runner_dir(),
            &self.workos_fixture_dir(),
        ] {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        Ok(())
    }

    pub fn write_files(&self) -> Result<()> {
        self.ensure_dirs()?;
        self.write_secret_files()?;
        self.write_env_file()?;
        self.write_postgres_script()?;
        fs::write(&self.process_compose_file, self.process_compose_yaml())
            .with_context(|| format!("failed to write {}", self.process_compose_file.display()))?;
        fs::write(self.run_dir.join("urls.txt"), self.urls_text()).with_context(|| {
            format!(
                "failed to write {}",
                self.run_dir.join("urls.txt").display()
            )
        })?;
        Ok(())
    }

    fn write_secret_files(&self) -> Result<()> {
        self.remove_secret_files();
        fs::create_dir_all(self.secrets_dir())
            .with_context(|| format!("failed to create {}", self.secrets_dir().display()))?;
        #[cfg(unix)]
        fs::set_permissions(self.secrets_dir(), fs::Permissions::from_mode(0o700))?;

        let fixture = FixturePaths::new(self.workos_fixture_dir());
        prepare_workos_fixture(&fixture, &self.workos_fixture_url())?;
        let workos_api_key = fs::read_to_string(&fixture.api_key)?;
        let customer_token = fs::read_to_string(&fixture.customer_token)?;
        let runner_credentials_json = devfinity_runner_credentials_json();
        let identity_operator_token = random_local_secret()?;
        write_mode_600(
            &self.core_secret_file(),
            format!(
                "export FC_CORE_API_TOKEN={}\nexport FC_CORE_RUNNER_CREDENTIALS_JSON={}\nexport {}={}\nexport FC_FINITE_PRIVATE_USAGE_API_TOKEN={}\nexport WORKOS_API_KEY={}\n",
                shell_quote(&self.core_token),
                shell_quote(&runner_credentials_json),
                DEVFINITY_RUNNER_TOKEN_ENV,
                shell_quote(DEVFINITY_RUNNER_TOKEN),
                shell_quote(DEVFINITY_USAGE_TOKEN),
                shell_quote(workos_api_key.trim())
            ).as_bytes(),
        )?;
        write_mode_600(
            &self.runner_auth_secret_file(),
            format!(
                "export FC_CORE_RUNNER_API_TOKEN={}\n",
                shell_quote(DEVFINITY_RUNNER_TOKEN)
            )
            .as_bytes(),
        )?;
        write_mode_600(
            &self.identity_authority_secret_file(),
            format!(
                "export FINITE_IDENTITY_OPERATOR_TOKEN={}\n",
                shell_quote(&identity_operator_token)
            )
            .as_bytes(),
        )?;
        write_mode_600(
            &self.limiter_auth_secret_file(),
            format!(
                "export FC_FINITE_PRIVATE_USAGE_API_TOKEN={}\n",
                shell_quote(DEVFINITY_USAGE_TOKEN)
            )
            .as_bytes(),
        )?;
        write_mode_600(
            &self.dashboard_auth_secret_file(),
            format!(
                "export FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN={}\nexport FC_CORE_API_TOKEN={}\n",
                shell_quote(customer_token.trim()),
                shell_quote(&self.core_token)
            )
            .as_bytes(),
        )?;

        if !self.profile.includes_runtime() {
            return Ok(());
        }

        match self.inference_mode {
            InferenceMode::ChainedLimiter => {
                let value = required_secret_env("FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY")?;
                write_mode_600(
                    &self.limiter_secret_file(),
                    format!(
                        "export FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY={}\n",
                        shell_quote(&value)
                    )
                    .as_bytes(),
                )?;
            }
            InferenceMode::DirectKeyOverride => {
                let value = required_secret_env("FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE")?;
                write_mode_600(
                    &self.runner_secret_file(),
                    format!(
                        "export FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE={}\n",
                        shell_quote(&value)
                    )
                    .as_bytes(),
                )?;
            }
            InferenceMode::Missing => {}
        }
        Ok(())
    }

    fn remove_secret_files(&self) {
        for path in [
            self.limiter_secret_file(),
            self.runner_secret_file(),
            self.core_secret_file(),
            self.runner_auth_secret_file(),
            self.limiter_auth_secret_file(),
            self.dashboard_auth_secret_file(),
            self.identity_authority_secret_file(),
        ] {
            remove_file_best_effort(&path);
        }
        if self.secrets_dir().exists()
            && let Err(error) = fs::remove_dir(self.secrets_dir())
        {
            eprintln!(
                "failed to remove empty devfinity secret directory {}: {error}",
                self.secrets_dir().display()
            );
        }
    }

    pub fn write_env_file(&self) -> Result<()> {
        fs::write(self.run_dir.join("env"), self.env_exports())
            .with_context(|| format!("failed to write {}", self.run_dir.join("env").display()))
    }

    pub fn print_summary(&self) {
        println!("devfinity local stack");
        println!("  profile:    {}", self.profile.as_str());
        println!("  state:      {}", self.run_dir.display());
        println!("  logs:       {}", self.logs_dir.display());
        println!("  config:     {}", self.process_compose_file.display());
        println!("  socket:     {}", self.process_compose_socket.display());
        println!("  dashboard:  {}", self.dashboard_url());
        println!("  core:       {}", self.core_url());
        println!("  chat:       {}", self.finitechat_url());
        println!("  web device: {}", self.hosted_web_device_url());
        println!("  sites api:  {}", self.finitesites_api_url());
        println!("  brain:      {}", self.finite_brain_url());
        println!(
            "  sites base: http://*.sites.localhost:{}",
            self.ports.finitesites
        );
        if self.profile.includes_runtime() {
            println!(
                "  runtime:    http://127.0.0.1:{}",
                self.ports.runtime_agent
            );
            println!(
                "  host route: {} ({})",
                self.apple_host_access.runtime_host, self.apple_host_access.source
            );
            println!("  image:      {RUNTIME_IMAGE_REF}");
        }
        println!();
        println!("  env file:   {}", self.run_dir.join("env").display());
        println!("  urls file:  {}", self.run_dir.join("urls.txt").display());
        println!();
        println!("Stop the stack by quitting process-compose or pressing Ctrl-C.");
        println!("Run `devfinity cleanup` if a previous stack left orphaned processes behind.");
    }

    pub fn env_exports(&self) -> String {
        let mut out = String::new();
        for (key, value) in self.env_values() {
            let _ = writeln!(out, "export {key}={}", shell_quote(&value));
        }
        out
    }

    pub fn run_process_compose_up(
        &self,
        mode: ProcessComposeMode,
        dry_run: bool,
    ) -> Result<ExitCode> {
        self.ensure_process_compose_available()?;
        ensure_private_dir(&self.process_compose_control_dir)?;
        if !dry_run {
            self.prepare_for_start()?;
        }
        let mut command = self.process_compose_up_command();
        if matches!(mode, ProcessComposeMode::Headless) {
            command.arg("--tui=false");
        }
        if dry_run {
            command.arg("--dry-run");
        }
        command.arg("up");
        let result =
            run_status_with_pid_file(command, &self.pid_file(ManagedProcess::ProcessCompose));
        self.remove_secret_files();
        remove_file_best_effort(&self.process_compose_socket);
        self.remove_process_compose_control_dir();
        result
    }

    pub fn run_wrapped_command(&self, command: &[String]) -> Result<ExitCode> {
        if command.is_empty() {
            bail!("wrapped command cannot be empty");
        }

        self.ensure_process_compose_available()?;
        self.prepare_for_start()?;
        let mut guard = self.start_process_compose_headless()?;
        // Cold-cache CI needs a bigger window: the stack's cargo processes may
        // still be compiling when a warm-cache 180s would already have expired.
        let ready_timeout = std::env::var("DEVFINITY_READY_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or_else(|| {
                if self.profile.includes_runtime() {
                    // A cold canonical image build compiles the Rust CLIs and
                    // installs Hermes inside Apple Container's builder VM.
                    1_800
                } else {
                    180
                }
            });
        let outcome =
            match self.wait_for_services_ready(Duration::from_secs(ready_timeout), &mut guard) {
                Ok(()) => self.run_stack_command(command),
                Err(error) => Err(error),
            };

        if let Err(error) = guard.shutdown() {
            eprintln!("devfinity cleanup after wrapped command failed: {error:#}");
        }
        self.remove_secret_files();

        outcome
    }

    pub fn prepare_for_start(&self) -> Result<()> {
        self.ensure_postgres_not_running()?;
        if self.fresh_services_state {
            if self.profile != StackProfile::ServicesOnly {
                bail!("fresh state is only supported by the services-only smoke profile");
            }
            for dir in [
                self.postgres_dir(),
                self.core_dir(),
                self.finitechat_dir(),
                self.hosted_web_device_dir(),
                self.finitesites_dir(),
                self.finite_identity_dir(),
                self.finite_brain_dir(),
                self.finite_home_dir(),
            ] {
                if dir.exists() {
                    fs::remove_dir_all(&dir)
                        .with_context(|| format!("failed to remove {}", dir.display()))?;
                }
            }
            self.ensure_dirs()?;
        }
        Ok(())
    }

    pub fn cleanup(&self) -> Result<ExitCode> {
        if self.process_compose_socket.exists() && self.process_compose_file.exists() {
            if self.process_compose_available() {
                let mut command = self.process_compose_control_command();
                command.arg("down");
                match command.status() {
                    Ok(status) if status.success() => {
                        println!("process-compose stack stopped");
                    }
                    Ok(status) => {
                        eprintln!("process-compose down exited with {status}; continuing cleanup");
                    }
                    Err(error) => {
                        eprintln!(
                            "failed to run process-compose down: {error}; continuing cleanup"
                        );
                    }
                }
            } else {
                eprintln!("process-compose not found; skipping process-compose down");
            }
        } else {
            println!("no devfinity process-compose socket found");
        }

        self.cleanup_managed_processes();
        self.remove_secret_files();

        let process_compose_pid_file = self.pid_file(ManagedProcess::ProcessCompose);
        for path in [&self.process_compose_socket, &process_compose_pid_file] {
            if path.exists()
                && let Err(error) = fs::remove_file(path)
            {
                eprintln!("failed to remove {}: {error}", path.display());
            }
        }
        self.remove_process_compose_control_dir();

        println!("devfinity cleanup complete");
        Ok(ExitCode::SUCCESS)
    }

    fn remove_process_compose_control_dir(&self) {
        match fs::remove_dir(&self.process_compose_control_dir) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => eprintln!(
                "failed to remove {}: {error}",
                self.process_compose_control_dir.display()
            ),
        }
    }

    pub fn status(&self) -> Result<ExitCode> {
        println!("devfinity status");
        println!("  state:  {}", self.run_dir.display());
        println!("  logs:   {}", self.logs_dir.display());
        println!("  config: {}", self.process_compose_file.display());
        println!(
            "  socket: {} ({})",
            self.process_compose_socket.display(),
            if self.process_compose_socket.exists() {
                "present"
            } else {
                "missing"
            }
        );
        println!();

        let table = match process_table() {
            Ok(table) => table,
            Err(error) => {
                eprintln!("failed to inspect process table: {error}");
                Vec::new()
            }
        };

        println!("processes:");
        for status in self.managed_process_statuses(&table) {
            println!(
                "  {:<16} {:<10} {}",
                status.process, status.state, status.detail
            );
        }
        println!();

        println!("services:");
        for check in self.service_checks() {
            println!(
                "  {:<16} {:<9} {}",
                check.process, check.state, check.detail
            );
        }

        Ok(ExitCode::SUCCESS)
    }

    fn process_compose_yaml(&self) -> String {
        let mut yaml = String::new();
        let _ = writeln!(yaml, "version: \"0.5\"");
        let _ = writeln!(
            yaml,
            "log_location: {}",
            yaml_string(
                &self
                    .logs_dir
                    .join("process-compose.log")
                    .display()
                    .to_string()
            )
        );
        let _ = writeln!(yaml, "log_level: info");
        let _ = writeln!(yaml, "processes:");
        self.write_rust_build(&mut yaml);
        self.write_workos_fixture(&mut yaml);
        self.write_postgres(&mut yaml);
        self.write_core(&mut yaml);
        self.write_finitechat(&mut yaml);
        self.write_hosted_web_device(&mut yaml);
        self.write_finitesites(&mut yaml);
        self.write_finite_identity(&mut yaml);
        self.write_finite_brain(&mut yaml);
        if self.profile.includes_runtime() {
            self.write_runtime_image(&mut yaml);
            if self.inference_mode == InferenceMode::ChainedLimiter {
                self.write_finite_private_limiter(&mut yaml);
            }
            self.write_apple_network_probe(&mut yaml);
            self.write_runtime_artifact(&mut yaml);
            self.write_runner(&mut yaml);
        }
        self.write_dashboard_deps(&mut yaml);
        self.write_dashboard(&mut yaml);
        yaml
    }

    fn write_rust_build(&self, yaml: &mut String) {
        let process = ManagedProcess::RustBuild;
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Build Rust service binaries",
            &self.repo_root,
            process,
        );
        let build_command = if self.profile.includes_runtime() {
            "cargo build -p finite-saas-core && cargo build -p finitechat-server && cargo build -p finitechat-hosted-device && cargo build -p finitesitesd && cargo build -p finite-identity --bin finite-identityd && cargo build -p finite-brain-app && cargo build -p finite-saas-local && cargo build -p finite-saas-runner"
        } else {
            "cargo build -p finite-saas-core && cargo build -p finitechat-server && cargo build -p finitechat-hosted-device && cargo build -p finitesitesd && cargo build -p finite-identity --bin finite-identityd && cargo build -p finite-brain-app"
        };
        self.write_managed_command(
            yaml,
            process,
            // One build per package, matching each service's `cargo run -p`
            // resolution: a combined `-p A -p B -p C` build unifies features
            // across the packages (resolver 2), producing artifacts the
            // per-package runs don't reuse — on a cold cache every service
            // then recompiles its whole dep stack inside the readiness window.
            &[String::from(build_command)],
            &[],
        );
        let _ = writeln!(yaml, "    availability:");
        let _ = writeln!(yaml, "      restart: exit_on_failure");
    }

    fn write_postgres(&self, yaml: &mut String) {
        let process = ManagedProcess::Postgres;
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Local Postgres for finite-saas-core",
            &self.repo_root,
            process,
        );
        self.write_managed_command(
            yaml,
            process,
            &[format!(
                "exec bash {}",
                shell_quote(&self.postgres_script_path().display().to_string())
            )],
            &[],
        );
        let _ = writeln!(yaml, "    readiness_probe:");
        let _ = writeln!(yaml, "      exec:");
        let _ = writeln!(
            yaml,
            "        command: {}",
            yaml_string(&format!(
                "psql -h 127.0.0.1 -p {} -U postgres -d finite_saas_core -tAc 'select 1' >/dev/null",
                self.ports.postgres
            ))
        );
        self.write_probe_timing(yaml, 3, 2, 5, 30);
    }

    fn write_workos_fixture(&self, yaml: &mut String) {
        let process = ManagedProcess::WorkosFixture;
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Local read-only WorkOS JWKS and user fixture",
            &self.repo_root,
            process,
        );
        self.write_managed_command(
            yaml,
            process,
            &[format!(
                "exec cargo run -p devfinity -- workos-fixture --listen 127.0.0.1:{} --state-dir {}",
                self.ports.workos_fixture,
                shell_quote(&self.workos_fixture_dir().display().to_string())
            )],
            &[],
        );
        self.write_http_probe(
            yaml,
            &format!("/sso/jwks/{WORKOS_FIXTURE_CLIENT_ID}"),
            self.ports.workos_fixture,
            1,
            2,
            3,
            45,
        );
        let _ = writeln!(yaml, "    availability:");
        let _ = writeln!(yaml, "      restart: always");
    }

    fn write_postgres_script(&self) -> Result<()> {
        let script = self.postgres_script_path();
        let contents = format!(
            r#"#!/usr/bin/env bash
set -euo pipefail

export PGDATA={pgdata}
database=finite_saas_core
port={port}

mkdir -p "$PGDATA"

if [ ! -s "$PGDATA/PG_VERSION" ]; then
  initdb -D "$PGDATA" --username=postgres --auth=trust --no-locale --encoding=UTF8
fi

# TCP only: the nixpkgs default socket dir (/run/postgresql) is not writable
# on CI runners, and run-dir paths exceed the 103-byte unix socket limit on
# macOS. Everything in this stack connects via 127.0.0.1.
postgres -D "$PGDATA" -h 127.0.0.1 -p "$port" -c unix_socket_directories='' &
postgres_pid=$!

shutdown() {{
  set +e
  kill "$postgres_pid" >/dev/null 2>&1 || true
  wait "$postgres_pid" >/dev/null 2>&1 || true
}}
trap shutdown INT TERM

until pg_isready -h 127.0.0.1 -p "$port" -U postgres >/dev/null 2>&1; do
  if ! kill -0 "$postgres_pid" >/dev/null 2>&1; then
    wait "$postgres_pid"
    exit $?
  fi
  sleep 0.2
done

if ! psql -h 127.0.0.1 -p "$port" -U postgres -d postgres -tAc "select 1 from pg_database where datname = '$database'" | grep -q 1; then
  createdb -h 127.0.0.1 -p "$port" -U postgres "$database"
fi

wait "$postgres_pid"
"#,
            pgdata = shell_quote(&self.postgres_data_dir().display().to_string()),
            port = self.ports.postgres
        );

        fs::write(&script, contents)
            .with_context(|| format!("failed to write {}", script.display()))
    }

    fn write_core(&self, yaml: &mut String) {
        let process = ManagedProcess::Core;
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(yaml, "Finite SaaS Core API", &self.repo_root, process);
        self.write_managed_command(
            yaml,
            process,
            &[
                format!(
                    ". {}",
                    shell_quote(&self.core_secret_file().display().to_string())
                ),
                String::from("exec cargo run -p finite-saas-core -- serve"),
            ],
            &[],
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::Postgres);
        let _ = writeln!(yaml, "        condition: process_healthy");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::WorkosFixture);
        let _ = writeln!(yaml, "        condition: process_healthy");
        self.write_environment(
            yaml,
            &[
                ("FC_CORE_DATABASE_URL", self.database_url()),
                ("FC_CORE_BIND", format!("127.0.0.1:{}", self.ports.core)),
                ("WORKOS_CLIENT_ID", WORKOS_FIXTURE_CLIENT_ID.to_string()),
                ("WORKOS_API_BASE_URL", self.workos_fixture_url()),
                (
                    "WORKOS_JWKS_URL",
                    format!(
                        "{}/sso/jwks/{}",
                        self.workos_fixture_url(),
                        WORKOS_FIXTURE_CLIENT_ID
                    ),
                ),
                ("WORKOS_ISSUER", self.workos_fixture_url()),
                (
                    "FC_WORKOS_OPERATOR_ORG_ID",
                    WORKOS_FIXTURE_OPERATOR_ORG_ID.to_string(),
                ),
                (
                    "FC_CORE_RUNTIME_ENV_JSON",
                    serde_json::json!({
                        "FINITE_SITES_API": self.finitesites_api_url(),
                        "FINITE_BRAIN_SERVER_URL": self.runtime_finite_brain_url(),
                        "FINITE_BRAIN_PUBLIC_BASE_URL": self.dashboard_origin(),
                        "FINITE_BRAIN_DEVELOPMENT_HTTP_HOST": self.apple_host_access.runtime_host,
                    })
                    .to_string(),
                ),
                (
                    "FC_CORE_AGENT_CREATION_PLACEMENT_JSON",
                    serde_json::json!({
                        "runnerClass": RUNNER_CLASS,
                        "runtimeResourceClass": "vcpu4_memory8_gib",
                    })
                    .to_string(),
                ),
            ],
        );
        self.write_http_probe(yaml, "/healthz", self.ports.core, 2, 2, 3, 45);
    }

    fn write_finitechat(&self, yaml: &mut String) {
        let process = ManagedProcess::FiniteChat;
        let sqlite = self.finitechat_dir().join("server.sqlite3");
        let command = format!(
            "cargo run -p finitechat-server -- serve {}:{} --sqlite {}",
            self.service_bind_host(),
            self.ports.finitechat,
            shell_quote(&sqlite.display().to_string())
        );
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Local Finite Chat delivery server",
            &self.repo_root,
            process,
        );
        self.write_managed_command(yaml, process, &[format!("exec {command}")], &[]);
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        self.write_http_probe_host(
            yaml,
            self.service_probe_host(),
            "/health",
            self.ports.finitechat,
            1,
            2,
            3,
            45,
        );
        let _ = writeln!(yaml, "    availability:");
        let _ = writeln!(yaml, "      restart: always");
    }

    fn write_hosted_web_device(&self, yaml: &mut String) {
        let process = ManagedProcess::HostedWebDevice;
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Local Finite Chat Hosted Web Device",
            &self.repo_root,
            process,
        );
        self.write_managed_command(
            yaml,
            process,
            &[
                format!(
                    ". {}",
                    shell_quote(&self.identity_authority_secret_file().display().to_string())
                ),
                String::from("exec cargo run -p finitechat-hosted-device"),
            ],
            &[],
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::FiniteChat);
        let _ = writeln!(yaml, "        condition: process_healthy");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::FiniteIdentity);
        let _ = writeln!(yaml, "        condition: process_healthy");
        self.write_environment(
            yaml,
            &[
                (
                    "FINITECHAT_HOSTED_BIND",
                    format!("127.0.0.1:{}", self.ports.hosted_web_device),
                ),
                (
                    "FINITECHAT_HOSTED_DATA_ROOT",
                    self.hosted_web_device_dir().display().to_string(),
                ),
                (
                    "FINITECHAT_HOSTED_API_TOKEN",
                    self.hosted_web_device_token.clone(),
                ),
                ("FINITECHAT_SERVER_URL", self.finitechat_url()),
                ("FINITE_IDENTITY_AUTHORITY", self.finite_identity_url()),
            ],
        );
        self.write_http_probe(yaml, "/healthz", self.ports.hosted_web_device, 1, 2, 3, 45);
        let _ = writeln!(yaml, "    availability:");
        let _ = writeln!(yaml, "      restart: always");
    }

    fn write_finitesites(&self, yaml: &mut String) {
        let process = ManagedProcess::FiniteSites;
        let data = self.finitesites_dir();
        let command = format!(
            concat!(
                "cargo run -p finitesitesd -- serve ",
                "--data {} ",
                "--listen {}:{} ",
                "--api-url {} ",
                "--base-domain sites.localhost ",
                "--document-base-domain docs.sites.localhost ",
                "--git-url {} ",
                "--site-port {} ",
                "--app-runner none"
            ),
            shell_quote(&data.display().to_string()),
            if self.profile.includes_runtime() {
                "0.0.0.0"
            } else {
                "127.0.0.1"
            },
            self.ports.finitesites,
            shell_quote(&self.finitesites_api_url()),
            shell_quote(&self.finitesites_api_url()),
            self.ports.finitesites
        );
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(yaml, "Local Finite Sites server", &self.repo_root, process);
        self.write_managed_command(yaml, process, &[format!("exec {command}")], &[]);
        self.write_environment(
            yaml,
            &[(
                "FINITE_SITES_VIEWER_SESSION_TOKEN",
                self.sites_viewer_session_token.clone(),
            )],
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        self.write_http_probe(yaml, "/api/v1/healthz", self.ports.finitesites, 1, 2, 3, 45);
    }

    fn write_finite_brain(&self, yaml: &mut String) {
        let process = ManagedProcess::FiniteBrain;
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Local FiniteBrain API and first-party Product Client",
            &self.repo_root,
            process,
        );
        self.write_managed_command(
            yaml,
            process,
            &[
                format!(
                    ". {}",
                    shell_quote(&self.core_secret_file().display().to_string())
                ),
                format!(
                    ". {}",
                    shell_quote(&self.identity_authority_secret_file().display().to_string())
                ),
                String::from("exec cargo run -p finite-brain-app"),
            ],
            &[],
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::FiniteIdentity);
        let _ = writeln!(yaml, "        condition: process_healthy");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::Core);
        let _ = writeln!(yaml, "        condition: process_healthy");
        self.write_environment(
            yaml,
            &[
                (
                    "FINITE_BRAIN_ADDR",
                    format!("{}:{}", self.service_bind_host(), self.ports.finite_brain),
                ),
                ("FINITE_BRAIN_PUBLIC_BASE_URL", self.dashboard_origin()),
                (
                    "FINITE_BRAIN_DB",
                    self.finite_brain_dir()
                        .join("finite-brain.sqlite3")
                        .display()
                        .to_string(),
                ),
                ("FINITE_IDENTITY_AUTHORITY", self.finite_identity_url()),
                ("FC_CORE_API_BASE_URL", self.core_url()),
            ],
        );
        self.write_http_probe_host(
            yaml,
            self.service_probe_host(),
            "/health",
            self.ports.finite_brain,
            1,
            2,
            3,
            45,
        );
    }

    fn write_finite_identity(&self, yaml: &mut String) {
        let process = ManagedProcess::FiniteIdentity;
        let command = vec![
            format!(
                ". {}",
                shell_quote(&self.identity_authority_secret_file().display().to_string())
            ),
            format!(
                concat!(
                    "exec cargo run -p finite-identity --bin finite-identityd -- serve ",
                    "--data {} --external-base-url {} --listen 127.0.0.1:{} ",
                    "--finite-vip-domain finite.vip ",
                    "--mailer dev --dev-print-email-tokens yes"
                ),
                shell_quote(&self.finite_identity_dir().display().to_string()),
                shell_quote(&self.finite_identity_url()),
                self.ports.finite_identity,
            ),
        ];

        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Local Finite Identity authority",
            &self.repo_root,
            process,
        );
        self.write_managed_command(yaml, process, &command, &[]);
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        self.write_http_probe(yaml, "/health", self.ports.finite_identity, 1, 2, 3, 45);
    }

    fn write_runtime_image(&self, yaml: &mut String) {
        let process = ManagedProcess::RuntimeImage;
        let report = self.runtime_image_dir().join("build-report.json");
        let context = self.runtime_image_dir().join("context");
        let command = format!(
            concat!(
                "exec python3 finitecomputer-v2/scripts/build_runtime_image.py ",
                "--engine apple-container ",
                "--image-ref {} ",
                "--context-dir {} ",
                "--report {}"
            ),
            shell_quote(RUNTIME_IMAGE_REF),
            shell_quote(&context.display().to_string()),
            shell_quote(&report.display().to_string()),
        );
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Build the canonical Agent Runtime with Apple Container",
            &self.repo_root,
            process,
        );
        self.write_managed_command(yaml, process, &[command], &[]);
        let _ = writeln!(yaml, "    availability:");
        let _ = writeln!(yaml, "      restart: exit_on_failure");
    }

    fn write_finite_private_limiter(&self, yaml: &mut String) {
        let process = ManagedProcess::FinitePrivateLimiter;
        let source_secret = format!(
            ". {}",
            shell_quote(&self.limiter_secret_file().display().to_string())
        );
        let command = format!(
            concat!(
                "exec cargo run -p finite-saas-local -- finite-private-limiter-up ",
                "--listen-addr {} ",
                "--core-url {} ",
                "--dashboard-url {} ",
                "--agent-host {}"
            ),
            shell_quote(&format!(
                "{}:{}",
                self.service_bind_host(),
                self.ports.finite_private_limiter
            )),
            shell_quote(&self.core_url()),
            shell_quote(&self.dashboard_url()),
            shell_quote(&self.apple_host_access.runtime_host),
        );
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Local Finite Private admission and inference chain",
            &self.repo_root,
            process,
        );
        self.write_managed_command(
            yaml,
            process,
            &[
                format!(
                    ". {}",
                    shell_quote(&self.limiter_auth_secret_file().display().to_string())
                ),
                source_secret,
                command,
            ],
            &[],
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::Core);
        let _ = writeln!(yaml, "        condition: process_healthy");
        self.write_http_probe_host(
            yaml,
            self.service_probe_host(),
            "/health",
            self.ports.finite_private_limiter,
            1,
            2,
            3,
            60,
        );
    }

    fn write_apple_network_probe(&self, yaml: &mut String) {
        let process = ManagedProcess::AppleNetworkProbe;
        let mut urls = vec![
            format!("{}/health", self.runtime_finitechat_url()),
            format!("{}/api/v1/healthz", self.finitesites_api_url()),
        ];
        if self.inference_mode == InferenceMode::ChainedLimiter {
            urls.push(format!("{}/health", self.runtime_limiter_root_url()));
        }
        let probe_script = urls
            .iter()
            .map(|url| format!("curl -fsS --max-time 5 {} >/dev/null", shell_quote(url)))
            .collect::<Vec<_>>()
            .join(" && ");
        let command = format!(
            "exec container run --rm --name devfinity-host-network-probe --entrypoint /bin/bash {} -lc {}",
            shell_quote(RUNTIME_IMAGE_REF),
            shell_quote(&probe_script),
        );
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Prove the Agent Runtime can reach local host services",
            &self.repo_root,
            process,
        );
        self.write_managed_command(
            yaml,
            process,
            &[
                String::from(
                    "container delete --force devfinity-host-network-probe >/dev/null 2>&1 || true",
                ),
                command,
            ],
            &[],
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RuntimeImage);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::FiniteChat);
        let _ = writeln!(yaml, "        condition: process_healthy");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::FiniteSites);
        let _ = writeln!(yaml, "        condition: process_healthy");
        if self.inference_mode == InferenceMode::ChainedLimiter {
            let _ = writeln!(yaml, "      {}:", ManagedProcess::FinitePrivateLimiter);
            let _ = writeln!(yaml, "        condition: process_healthy");
        }
        let _ = writeln!(yaml, "    availability:");
        let _ = writeln!(yaml, "      restart: exit_on_failure");
    }

    fn write_runtime_artifact(&self, yaml: &mut String) {
        let process = ManagedProcess::RuntimeArtifact;
        let report = self.runtime_image_dir().join("build-report.json");
        let runner_artifact_env = self.runtime_image_dir().join("runner-artifact.sh");
        let command = String::from(concat!(
            "cargo run -p finite-saas-core -- runtime-artifact-upsert ",
            "--artifact-id \"$artifact_id\" ",
            "--kind oci_image ",
            "--reference \"$reference\" ",
            "--version-label devfinity-worktree ",
            "--state-schema-version runtime-state-v1 ",
            "--hermes-source-ref hermes-agent==0.18.2 ",
            "--promoted"
        ));
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Register the locally built Runtime as a promoted Core artifact",
            &self.repo_root,
            process,
        );
        self.write_managed_command(
            yaml,
            process,
            &[
                format!(
                    "digest_hex=$(jq -er '.image_metadata.digest | select(test(\"^sha256:[0-9a-f]{{64}}$\")) | sub(\"^sha256:\"; \"\")' {})",
                    shell_quote(&report.display().to_string())
                ),
                format!(
                    "artifact_id={}-\"$digest_hex\"",
                    shell_quote(RUNTIME_ARTIFACT_ID_PREFIX)
                ),
                String::from("digest=\"sha256:$digest_hex\""),
                format!("reference={}@\"$digest\"", shell_quote(RUNTIME_IMAGE_REF)),
                command,
                String::from("umask 077"),
                format!(
                    "printf 'export FC_RUNNER_RUNTIME_ARTIFACT_ID=%s\\n' \"$artifact_id\" > {}",
                    shell_quote(&runner_artifact_env.display().to_string())
                ),
            ],
            &[],
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::Core);
        let _ = writeln!(yaml, "        condition: process_healthy");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::AppleNetworkProbe);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        self.write_environment(yaml, &[("FC_CORE_DATABASE_URL", self.database_url())]);
        let _ = writeln!(yaml, "    availability:");
        let _ = writeln!(yaml, "      restart: exit_on_failure");
    }

    fn write_runner(&self, yaml: &mut String) {
        let process = ManagedProcess::Runner;
        let mut command = vec![
            format!(
                ". {}",
                shell_quote(&self.runner_auth_secret_file().display().to_string())
            ),
            format!(
                ". {}",
                shell_quote(&self.identity_authority_secret_file().display().to_string())
            ),
            format!(
                ". {}",
                shell_quote(
                    &self
                        .runtime_image_dir()
                        .join("runner-artifact.sh")
                        .display()
                        .to_string()
                )
            ),
        ];
        if self.inference_mode == InferenceMode::DirectKeyOverride {
            command.push(format!(
                ". {}",
                shell_quote(&self.runner_secret_file().display().to_string())
            ));
        }
        command.push("exec cargo run -p finite-saas-runner -- serve".to_string());

        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Real local Runner backed by Apple Container",
            &self.repo_root,
            process,
        );
        self.write_managed_command(yaml, process, &command, &[]);
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RuntimeArtifact);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::FiniteIdentity);
        let _ = writeln!(yaml, "        condition: process_healthy");
        self.write_environment(
            yaml,
            &[
                ("FC_RUNNER_CLASS", RUNNER_CLASS.to_string()),
                ("FC_CORE_URL", self.core_url()),
                ("FINITE_IDENTITY_AUTHORITY", self.finite_identity_url()),
                ("FC_RUNNER_ID", RUNNER_ID.to_string()),
                (
                    "FC_RUNNER_SOURCE_HOST_ID",
                    RUNNER_SOURCE_HOST_ID.to_string(),
                ),
                (
                    "FC_RUNNER_WORK_ROOT",
                    self.runner_dir().display().to_string(),
                ),
                (
                    "FC_RUNNER_FINITECHAT_SERVER_URL",
                    self.runtime_finitechat_url(),
                ),
                (
                    "FC_RUNNER_RUNTIME_ENV_JSON",
                    serde_json::json!({
                        "FINITE_SITES_API": self.finitesites_api_url(),
                        "FINITE_BRAIN_SERVER_URL": self.runtime_finite_brain_url(),
                        "FINITE_BRAIN_PUBLIC_BASE_URL": self.dashboard_origin(),
                        "FINITE_BRAIN_DEVELOPMENT_HTTP_HOST": self.apple_host_access.runtime_host,
                    })
                    .to_string(),
                ),
                (
                    "FC_RUNNER_APPLE_CONTAINER_NAME_PREFIX",
                    self.apple_container_name_prefix.clone(),
                ),
                (
                    "FC_RUNNER_APPLE_CONTAINER_LOCAL_IMAGE_REFERENCE",
                    RUNTIME_IMAGE_REF.to_string(),
                ),
                (
                    "FC_RUNNER_APPLE_CONTAINER_HOST_PORT",
                    self.ports.runtime_agent.to_string(),
                ),
                (
                    "FC_RUNNER_APPLE_CONTAINER_CONTAINER_PORT",
                    "8080".to_string(),
                ),
                ("FC_RUNNER_MAX_SANDBOXES", "1".to_string()),
                ("FC_RUNNER_IDLE_INTERVAL_MS", "1000".to_string()),
                (
                    "FC_RUNNER_FINITE_PRIVATE_BASE_URL",
                    if self.inference_mode == InferenceMode::ChainedLimiter {
                        format!("{}/v1", self.runtime_limiter_root_url())
                    } else {
                        std::env::var("FC_RUNNER_FINITE_PRIVATE_BASE_URL")
                            .ok()
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or_else(|| {
                                "https://kimi-k2-6.finite.containers.tinfoil.dev/v1".to_string()
                            })
                    },
                ),
            ],
        );
        // Runner performs a synchronous provider/artifact preflight before it
        // enters its retrying serve loop. Surface a static wiring failure as a
        // failed local stack instead of leaving a launch form backed by no
        // worker.
        let _ = writeln!(yaml, "    availability:");
        let _ = writeln!(yaml, "      restart: exit_on_failure");
    }

    fn write_dashboard(&self, yaml: &mut String) {
        let process = ManagedProcess::Dashboard;
        let dashboard_dir = self.repo_root.join("finitecomputer-v2/apps/dashboard");
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(yaml, "Finite dashboard dev server", &dashboard_dir, process);
        self.write_managed_command(
            yaml,
            process,
            &[
                format!(
                    ". {}",
                    shell_quote(&self.dashboard_auth_secret_file().display().to_string())
                ),
                format!(
                    "exec npm run dev -- --hostname 127.0.0.1 --port {}",
                    self.ports.dashboard
                ),
            ],
            &[],
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::DashboardDeps);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::Core);
        let _ = writeln!(yaml, "        condition: process_healthy");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::HostedWebDevice);
        let _ = writeln!(yaml, "        condition: process_healthy");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::FiniteBrain);
        let _ = writeln!(yaml, "        condition: process_healthy");
        if self.profile.includes_runtime() {
            let _ = writeln!(yaml, "      {}:", ManagedProcess::RuntimeArtifact);
            let _ = writeln!(yaml, "        condition: process_completed_successfully");
            let _ = writeln!(yaml, "      {}:", ManagedProcess::Runner);
            let _ = writeln!(yaml, "        condition: process_started");
        }
        let mut dashboard_environment = vec![
            ("FC_WORKOS_AUTH_ENABLED", "0".to_string()),
            ("FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH", "1".to_string()),
            (
                "FC_WORKOS_OPERATOR_ORG_ID",
                WORKOS_FIXTURE_OPERATOR_ORG_ID.to_string(),
            ),
            (
                "FC_DASHBOARD_DEV_EMAIL",
                WORKOS_FIXTURE_CUSTOMER_EMAIL.to_string(),
            ),
            (
                "FC_DASHBOARD_DEV_WORKOS_USER_ID",
                WORKOS_FIXTURE_CUSTOMER_SUBJECT.to_string(),
            ),
            ("FC_CORE_BASE_URL", self.core_url()),
            ("FC_HOSTED_WEB_DEVICE_URL", self.hosted_web_device_url()),
            ("FC_BRAIN_UPSTREAM_URL", self.finite_brain_url()),
            ("FC_BRAIN_PUBLIC_ORIGIN", self.dashboard_origin()),
            (
                "FC_SITES_UPSTREAM_URL",
                format!("http://127.0.0.1:{}", self.ports.finitesites),
            ),
            ("FC_SITES_ALLOW_LOCAL_OUTPUTS", "1".to_string()),
            (
                "FINITE_SITES_VIEWER_SESSION_TOKEN",
                self.sites_viewer_session_token.clone(),
            ),
            // Keep the long-lived local dev server isolated from production
            // and browser-test build artifacts. Next can otherwise combine
            // incompatible manifests and serve every App Router path as 404.
            ("NEXT_DIST_DIR", ".next-devfinity".to_string()),
            (
                "FINITECHAT_HOSTED_API_TOKEN",
                self.hosted_web_device_token.clone(),
            ),
            (
                "NEXT_PUBLIC_WORKOS_REDIRECT_URI",
                format!("http://127.0.0.1:{}/callback", self.ports.dashboard),
            ),
            (
                "WORKOS_COOKIE_PASSWORD",
                "devfinity-local-cookie-password-2026".to_string(),
            ),
        ];
        for name in [
            "GOOGLE_WORKSPACE_CLIENT_ID",
            "GOOGLE_WORKSPACE_CLIENT_SECRET",
        ] {
            if let Ok(value) = std::env::var(name)
                && !value.trim().is_empty()
            {
                dashboard_environment.push((name, value));
            }
        }
        self.write_environment(yaml, &dashboard_environment);
        self.write_http_probe(yaml, "/dashboard", self.ports.dashboard, 5, 5, 5, 120);
    }

    fn write_dashboard_deps(&self, yaml: &mut String) {
        let process = ManagedProcess::DashboardDeps;
        let dashboard_dir = self.repo_root.join("finitecomputer-v2/apps/dashboard");
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(
            yaml,
            "Install dashboard npm dependencies",
            &dashboard_dir,
            process,
        );
        self.write_managed_command(
            yaml,
            process,
            &[
                String::from(
                    "if [ ! -x node_modules/.bin/next ] || [ ! -f node_modules/.package-lock.json ] || find package.json package-lock.json -newer node_modules/.package-lock.json -print -quit | grep -q .; then",
                ),
                String::from("  npm ci"),
                String::from("else"),
                String::from("  echo \"dashboard dependencies already installed\""),
                String::from("fi"),
            ],
            &[],
        );
        let _ = writeln!(yaml, "    availability:");
        let _ = writeln!(yaml, "      restart: exit_on_failure");
    }

    fn write_process_header(
        &self,
        yaml: &mut String,
        description: &str,
        working_dir: &Path,
        process: ManagedProcess,
    ) {
        let _ = writeln!(yaml, "    description: {}", yaml_string(description));
        let _ = writeln!(
            yaml,
            "    working_dir: {}",
            yaml_string(&working_dir.display().to_string())
        );
        let _ = writeln!(
            yaml,
            "    log_location: {}",
            yaml_string(
                &self
                    .logs_dir
                    .join(format!("{process}.log"))
                    .display()
                    .to_string()
            )
        );
    }

    fn write_managed_command(
        &self,
        yaml: &mut String,
        process: ManagedProcess,
        command_lines: &[String],
        teardown_lines: &[String],
    ) {
        let pid_file = self.pid_file(process);
        let _ = writeln!(yaml, "    command: |");
        let _ = writeln!(yaml, "      set -eu");
        let _ = writeln!(
            yaml,
            "      mkdir -p {}",
            shell_quote(&self.pids_dir.display().to_string())
        );
        let _ = writeln!(yaml, "      export DEVFINITY_MANAGED_PROCESS=1");
        let _ = writeln!(
            yaml,
            "      export DEVFINITY_PROCESS={}",
            shell_quote(process.as_str())
        );
        let _ = writeln!(
            yaml,
            "      export DEVFINITY_RUN_DIR={}",
            shell_quote(&self.run_dir.display().to_string())
        );
        let _ = writeln!(yaml, "      (");
        for line in command_lines {
            let _ = writeln!(yaml, "        {line}");
        }
        let _ = writeln!(yaml, "      ) &");
        let _ = writeln!(yaml, "      child=$!");
        let _ = writeln!(
            yaml,
            "      printf '%s\\n' \"$child\" > {}",
            shell_quote(&pid_file.display().to_string())
        );
        let _ = writeln!(yaml, "      teardown() {{");
        let _ = writeln!(yaml, "        set +e");
        for line in teardown_lines {
            let _ = writeln!(yaml, "        {line}");
        }
        let _ = writeln!(yaml, "      }}");
        let _ = writeln!(yaml, "      cleanup() {{");
        let _ = writeln!(yaml, "        teardown");
        let _ = writeln!(yaml, "        kill \"$child\" >/dev/null 2>&1 || true");
        let _ = writeln!(yaml, "        wait \"$child\" >/dev/null 2>&1 || true");
        let _ = writeln!(
            yaml,
            "        rm -f {}",
            shell_quote(&pid_file.display().to_string())
        );
        let _ = writeln!(yaml, "        exit 143");
        let _ = writeln!(yaml, "      }}");
        let _ = writeln!(yaml, "      trap cleanup INT TERM");
        let _ = writeln!(yaml, "      set +e");
        let _ = writeln!(yaml, "      wait \"$child\"");
        let _ = writeln!(yaml, "      status=$?");
        let _ = writeln!(yaml, "      teardown");
        let _ = writeln!(
            yaml,
            "      rm -f {}",
            shell_quote(&pid_file.display().to_string())
        );
        let _ = writeln!(yaml, "      exit \"$status\"");
    }

    fn write_environment(&self, yaml: &mut String, env: &[(&str, String)]) {
        let _ = writeln!(yaml, "    environment:");
        for (key, value) in env {
            let _ = writeln!(yaml, "      - {}", yaml_string(&format!("{key}={value}")));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_http_probe(
        &self,
        yaml: &mut String,
        path: &str,
        port: u16,
        initial_delay: u64,
        period: u64,
        timeout: u64,
        failures: u64,
    ) {
        self.write_http_probe_host(
            yaml,
            "127.0.0.1",
            path,
            port,
            initial_delay,
            period,
            timeout,
            failures,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn write_http_probe_host(
        &self,
        yaml: &mut String,
        host: &str,
        path: &str,
        port: u16,
        initial_delay: u64,
        period: u64,
        timeout: u64,
        failures: u64,
    ) {
        let _ = writeln!(yaml, "    readiness_probe:");
        let _ = writeln!(yaml, "      http_get:");
        let _ = writeln!(yaml, "        host: {}", yaml_string(host));
        let _ = writeln!(yaml, "        scheme: http");
        let _ = writeln!(yaml, "        path: {}", yaml_string(path));
        let _ = writeln!(yaml, "        port: {port}");
        self.write_probe_timing(yaml, initial_delay, period, timeout, failures);
    }

    fn write_probe_timing(
        &self,
        yaml: &mut String,
        initial_delay: u64,
        period: u64,
        timeout: u64,
        failures: u64,
    ) {
        let _ = writeln!(yaml, "      initial_delay_seconds: {initial_delay}");
        let _ = writeln!(yaml, "      period_seconds: {period}");
        let _ = writeln!(yaml, "      timeout_seconds: {timeout}");
        let _ = writeln!(yaml, "      failure_threshold: {failures}");
    }

    fn process_compose_up_command(&self) -> Command {
        let mut command = Command::new("process-compose");
        command
            .arg("--disable-dotenv")
            .arg("--config")
            .arg(&self.process_compose_file)
            .args(self.process_compose_control_args());
        scrub_inference_secrets(&mut command);
        command
    }

    fn process_compose_control_command(&self) -> Command {
        let mut command = Command::new("process-compose");
        command.args(self.process_compose_control_args());
        scrub_inference_secrets(&mut command);
        command
    }

    fn process_compose_control_args(&self) -> Vec<std::ffi::OsString> {
        vec![
            "--use-uds".into(),
            "--unix-socket".into(),
            self.process_compose_socket.clone().into_os_string(),
            "--ordered-shutdown".into(),
            "--log-file".into(),
            self.logs_dir
                .join("process-compose-supervisor.log")
                .into_os_string(),
        ]
    }

    fn ensure_process_compose_available(&self) -> Result<()> {
        if self.process_compose_available() {
            return Ok(());
        }
        bail!("`process-compose version` failed; run `nix develop` or install process-compose")
    }

    fn process_compose_available(&self) -> bool {
        let status = Command::new("process-compose")
            .arg("version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        matches!(status, Ok(status) if status.success())
    }

    fn ensure_postgres_not_running(&self) -> Result<()> {
        let pid_file = self.pid_file(ManagedProcess::Postgres);
        if let Some(pid) = read_pid_file(&pid_file)?
            && process_alive(pid)
        {
            bail!(
                "devfinity postgres pid {pid} from {} is still running; run `devfinity cleanup` before starting a new stack",
                pid_file.display()
            );
        }

        if connect_tcp("127.0.0.1", self.ports.postgres).is_ok() {
            bail!(
                "tcp 127.0.0.1:{} is already accepting connections; stop the existing service or run `devfinity cleanup` before starting devfinity",
                self.ports.postgres
            );
        }

        Ok(())
    }

    fn start_process_compose_headless(&self) -> Result<ProcessComposeGuard<'_>> {
        self.ensure_process_compose_available()?;
        ensure_private_dir(&self.process_compose_control_dir)?;
        let mut command = self.process_compose_up_command();
        command.arg("--tui=false");
        command.arg("up");

        let pid_file = self.pid_file(ManagedProcess::ProcessCompose);
        if let Some(parent) = pid_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        println!("starting devfinity stack in headless mode");
        let mut child = command
            .spawn()
            .with_context(|| format!("failed to run {:?}", command))?;
        if let Err(error) = fs::write(&pid_file, format!("{}\n", child.id())) {
            let _ = child.kill();
            bail!("failed to write {}: {error}", pid_file.display());
        }

        Ok(ProcessComposeGuard {
            stack: self,
            child,
            pid_file,
            shutdown_complete: false,
        })
    }

    fn wait_for_services_ready(
        &self,
        timeout: Duration,
        guard: &mut ProcessComposeGuard<'_>,
    ) -> Result<()> {
        let started = Instant::now();
        let mut last_report = Instant::now() - Duration::from_secs(5);
        loop {
            if let Some(status) = guard
                .child
                .try_wait()
                .context("failed to check process-compose status")?
            {
                bail!("process-compose exited before devfinity became ready: {status}");
            }

            let checks = self.service_checks();
            let pending = pending_service_checks(&checks);
            if pending.is_empty() {
                println!("devfinity stack is ready");
                return Ok(());
            }

            if started.elapsed() >= timeout {
                bail!(
                    "devfinity stack did not become ready within {}s: {}",
                    timeout.as_secs(),
                    pending.join(", ")
                );
            }

            if last_report.elapsed() >= Duration::from_secs(5) {
                println!("waiting for devfinity stack: {}", pending.join(", "));
                last_report = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(750));
        }
    }

    fn run_stack_command(&self, command: &[String]) -> Result<ExitCode> {
        let program = &command[0];
        let args = &command[1..];
        println!("running devfinity command: {}", shell_words(command));
        let mut child_command = Command::new(program);
        child_command
            .args(args)
            .current_dir(&self.repo_root)
            .envs(self.env_values());
        scrub_inference_secrets(&mut child_command);
        let status = child_command.status().with_context(|| {
            format!("failed to run devfinity command `{}`", shell_words(command))
        })?;
        Ok(status_to_exit_code(status))
    }

    fn cleanup_managed_processes(&self) {
        let table = match process_table() {
            Ok(table) => table,
            Err(error) => {
                eprintln!("failed to inspect process table: {error}");
                return;
            }
        };

        // Cleanup must not depend on the credential/profile selected by the
        // current shell. A developer may unset the chained-limiter key before
        // recovering an orphaned stack, but its protected pid file still gives
        // us an exact and safely verifiable process identity.
        for spec in self.process_specs(ManagedProcess::ALL) {
            self.cleanup_pid_file(&spec, &table);
        }
    }

    fn cleanup_pid_file(&self, spec: &ManagedProcessSpec, table: &[ProcessInfo]) {
        let pid = match read_pid_file(&spec.pid_file) {
            Ok(Some(pid)) => pid,
            Ok(None) => return,
            Err(error) => {
                eprintln!("failed to read {}: {error}", spec.pid_file.display());
                return;
            }
        };

        let Some(root) = table.iter().find(|process| process.pid == pid) else {
            remove_file_best_effort(&spec.pid_file);
            return;
        };

        if !process_matches(root, &spec.expected_fragments) {
            eprintln!(
                "not killing pid {} from {} because it no longer looks like devfinity {}: {}",
                pid,
                spec.pid_file.display(),
                spec.process,
                root.command
            );
            return;
        }

        let mut pids = descendant_pids(table, pid);
        pids.push(pid);
        pids.sort_unstable();
        pids.dedup();
        pids.retain(|candidate| *candidate != std::process::id());

        if pids.is_empty() {
            remove_file_best_effort(&spec.pid_file);
            return;
        }

        pids.reverse();
        println!(
            "stopping devfinity {} process tree: {:?}",
            spec.process, pids
        );
        terminate_processes(&pids);
        remove_file_best_effort(&spec.pid_file);
    }

    fn managed_process_statuses(&self, table: &[ProcessInfo]) -> Vec<ManagedProcessRuntimeStatus> {
        self.managed_process_specs()
            .into_iter()
            .map(|spec| {
                let pid = match read_pid_file(&spec.pid_file) {
                    Ok(Some(pid)) => pid,
                    Ok(None) => {
                        return ManagedProcessRuntimeStatus::new(
                            spec.process,
                            "stopped",
                            format!("no pid file ({})", spec.pid_file.display()),
                        );
                    }
                    Err(error) => {
                        return ManagedProcessRuntimeStatus::new(
                            spec.process,
                            "unknown",
                            format!("invalid pid file {}: {error}", spec.pid_file.display()),
                        );
                    }
                };

                let Some(process) = table.iter().find(|process| process.pid == pid) else {
                    return ManagedProcessRuntimeStatus::new(
                        spec.process,
                        "stale",
                        format!("pid {pid} is not running"),
                    );
                };

                if process_matches(process, &spec.expected_fragments) {
                    ManagedProcessRuntimeStatus::new(
                        spec.process,
                        "running",
                        format!("pid {pid}: {}", process.command),
                    )
                } else {
                    ManagedProcessRuntimeStatus::new(
                        spec.process,
                        "mismatch",
                        format!("pid {pid}: {}", process.command),
                    )
                }
            })
            .collect()
    }

    fn managed_process_specs(&self) -> Vec<ManagedProcessSpec> {
        self.process_specs(self.enabled_processes())
    }

    fn process_specs(
        &self,
        processes: impl IntoIterator<Item = ManagedProcess>,
    ) -> Vec<ManagedProcessSpec> {
        processes
            .into_iter()
            .map(|process| {
                let expected_fragments = match process {
                    ManagedProcess::ProcessCompose => vec![
                        ManagedProcess::ProcessCompose.as_str().to_string(),
                        self.process_compose_file.display().to_string(),
                    ],
                    ManagedProcess::WorkosFixture => {
                        vec![String::from("devfinity"), String::from("workos-fixture")]
                    }
                    ManagedProcess::RustBuild => vec![String::from("cargo"), String::from("build")],
                    ManagedProcess::Postgres => vec![
                        String::from("bash"),
                        self.postgres_script_path().display().to_string(),
                    ],
                    ManagedProcess::Core => {
                        vec![String::from("finite-saas-core"), String::from("serve")]
                    }
                    ManagedProcess::FiniteChat => vec![
                        String::from("finitechat-server"),
                        self.finitechat_dir().display().to_string(),
                    ],
                    ManagedProcess::HostedWebDevice => {
                        vec![String::from("finitechat-hosted-device")]
                    }
                    ManagedProcess::FiniteSites => vec![
                        String::from("finitesitesd"),
                        self.finitesites_dir().display().to_string(),
                    ],
                    ManagedProcess::FiniteIdentity => {
                        vec![String::from("finite-identityd"), String::from("serve")]
                    }
                    ManagedProcess::FiniteBrain => vec![String::from("finite-brain")],
                    ManagedProcess::RuntimeImage => vec![
                        String::from("python3"),
                        String::from("build_runtime_image.py"),
                        String::from("apple-container"),
                    ],
                    ManagedProcess::FinitePrivateLimiter => vec![
                        String::from("finite-saas-local"),
                        String::from("finite-private-limiter-up"),
                    ],
                    ManagedProcess::AppleNetworkProbe => vec![
                        String::from("container"),
                        String::from("devfinity-host-network-probe"),
                    ],
                    ManagedProcess::RuntimeArtifact => vec![
                        String::from("finite-saas-core"),
                        String::from("runtime-artifact-upsert"),
                    ],
                    ManagedProcess::Runner => {
                        vec![String::from("finite-saas-runner"), String::from("serve")]
                    }
                    ManagedProcess::DashboardDeps => vec![String::from("npm"), String::from("ci")],
                    ManagedProcess::Dashboard => vec![
                        String::from("npm"),
                        String::from("run"),
                        String::from("dev"),
                        self.ports.dashboard.to_string(),
                    ],
                };
                ManagedProcessSpec::new(process, self.pid_file(process), expected_fragments)
            })
            .collect()
    }

    fn enabled_processes(&self) -> Vec<ManagedProcess> {
        ManagedProcess::ALL
            .into_iter()
            .filter(|process| {
                if matches!(
                    process,
                    ManagedProcess::RuntimeImage
                        | ManagedProcess::AppleNetworkProbe
                        | ManagedProcess::RuntimeArtifact
                        | ManagedProcess::Runner
                ) {
                    return self.profile.includes_runtime();
                }
                if *process == ManagedProcess::FinitePrivateLimiter {
                    return self.profile.includes_runtime()
                        && self.inference_mode == InferenceMode::ChainedLimiter;
                }
                true
            })
            .collect()
    }

    fn service_checks(&self) -> Vec<ServiceCheck> {
        let mut checks = vec![
            check_tcp_service(ManagedProcess::Postgres, "127.0.0.1", self.ports.postgres),
            check_http_service(
                ManagedProcess::Core,
                "127.0.0.1",
                self.ports.core,
                "/healthz",
            ),
            check_http_service(
                ManagedProcess::FiniteChat,
                self.service_probe_host(),
                self.ports.finitechat,
                "/health",
            ),
            check_http_service(
                ManagedProcess::HostedWebDevice,
                "127.0.0.1",
                self.ports.hosted_web_device,
                "/healthz",
            ),
            check_http_service(
                ManagedProcess::FiniteSites,
                "127.0.0.1",
                self.ports.finitesites,
                "/api/v1/healthz",
            ),
            check_http_service(
                ManagedProcess::FiniteIdentity,
                "127.0.0.1",
                self.ports.finite_identity,
                "/health",
            ),
            check_http_service(
                ManagedProcess::FiniteBrain,
                self.service_probe_host(),
                self.ports.finite_brain,
                "/health",
            ),
            check_http_service(
                ManagedProcess::Dashboard,
                "127.0.0.1",
                self.ports.dashboard,
                "/dashboard",
            ),
        ];
        if self.profile.includes_runtime() && self.inference_mode == InferenceMode::ChainedLimiter {
            checks.push(check_http_service(
                ManagedProcess::FinitePrivateLimiter,
                self.service_probe_host(),
                self.ports.finite_private_limiter,
                "/health",
            ));
        }
        checks
    }

    fn urls_text(&self) -> String {
        let mut urls = format!(
            concat!(
                "profile={}\n",
                "dashboard={}\n",
                "core={}\n",
                "finitechat={}\n",
                "hosted_web_device={}\n",
                "finitesites_api={}\n",
                "finitesites_base=http://*.sites.localhost:{}\n",
                "finite_identity={}\n",
                "finite_brain={}\n"
            ),
            self.profile.as_str(),
            self.dashboard_url(),
            self.core_url(),
            self.finitechat_url(),
            self.hosted_web_device_url(),
            self.finitesites_api_url(),
            self.ports.finitesites,
            self.finite_identity_url(),
            self.finite_brain_url()
        );
        if self.profile.includes_runtime() {
            let _ = writeln!(
                urls,
                "runtime=http://127.0.0.1:{}",
                self.ports.runtime_agent
            );
            let _ = writeln!(urls, "runtime_image={RUNTIME_IMAGE_REF}");
            let _ = writeln!(urls, "runtime_artifact_prefix={RUNTIME_ARTIFACT_ID_PREFIX}");
        }
        urls
    }

    fn core_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.core)
    }

    fn dashboard_url(&self) -> String {
        format!("{}/dashboard", self.dashboard_origin())
    }

    fn dashboard_origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.dashboard)
    }

    fn finitechat_url(&self) -> String {
        format!(
            "http://{}:{}",
            self.service_probe_host(),
            self.ports.finitechat
        )
    }

    fn runtime_finitechat_url(&self) -> String {
        format!(
            "http://{}:{}",
            self.apple_host_access.runtime_host, self.ports.finitechat
        )
    }

    fn runtime_limiter_root_url(&self) -> String {
        format!(
            "http://{}:{}",
            self.apple_host_access.runtime_host, self.ports.finite_private_limiter
        )
    }

    fn service_bind_host(&self) -> String {
        if self.profile.includes_runtime() {
            self.apple_host_access.bind_host.clone()
        } else {
            "127.0.0.1".to_string()
        }
    }

    fn service_probe_host(&self) -> &'static str {
        "127.0.0.1"
    }

    fn hosted_web_device_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.hosted_web_device)
    }

    fn finite_brain_url(&self) -> String {
        format!(
            "http://{}:{}",
            self.service_probe_host(),
            self.ports.finite_brain
        )
    }

    fn finite_identity_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.finite_identity)
    }

    fn runtime_finite_brain_url(&self) -> String {
        format!(
            "http://{}:{}",
            self.apple_host_access.runtime_host, self.ports.finite_brain
        )
    }

    fn finitesites_api_url(&self) -> String {
        let host = if self.profile.includes_runtime() {
            self.apple_host_access.runtime_host.as_str()
        } else {
            "127.0.0.1"
        };
        format!("http://{host}:{}", self.ports.finitesites)
    }

    fn database_url(&self) -> String {
        format!(
            "postgres://postgres:finite-local@127.0.0.1:{}/finite_saas_core",
            self.ports.postgres
        )
    }

    fn postgres_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::Postgres)
    }

    fn postgres_data_dir(&self) -> PathBuf {
        self.postgres_dir().join("data")
    }

    fn postgres_script_path(&self) -> PathBuf {
        self.run_dir.join("run-postgres.sh")
    }

    fn core_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::Core)
    }

    fn dashboard_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::Dashboard)
    }

    fn finitechat_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::FiniteChat)
    }

    fn hosted_web_device_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::HostedWebDevice)
    }

    fn finitesites_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::FiniteSites)
    }

    fn finite_brain_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::FiniteBrain)
    }

    fn finite_identity_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::FiniteIdentity)
    }

    fn finite_home_dir(&self) -> PathBuf {
        self.run_dir.join("finite-home")
    }

    fn runtime_image_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::RuntimeImage)
    }

    fn runner_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::Runner)
    }

    fn workos_fixture_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::WorkosFixture)
    }

    fn workos_fixture_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.workos_fixture)
    }

    fn secrets_dir(&self) -> PathBuf {
        self.run_dir.join("secrets")
    }

    fn limiter_secret_file(&self) -> PathBuf {
        self.secrets_dir().join("finite-private-limiter.sh")
    }

    fn runner_secret_file(&self) -> PathBuf {
        self.secrets_dir().join("runner.sh")
    }

    fn core_secret_file(&self) -> PathBuf {
        self.secrets_dir().join("core.sh")
    }
    fn runner_auth_secret_file(&self) -> PathBuf {
        self.secrets_dir().join("runner-auth.sh")
    }
    fn limiter_auth_secret_file(&self) -> PathBuf {
        self.secrets_dir().join("limiter-auth.sh")
    }
    fn dashboard_auth_secret_file(&self) -> PathBuf {
        self.secrets_dir().join("dashboard-auth.sh")
    }
    fn identity_authority_secret_file(&self) -> PathBuf {
        self.secrets_dir().join("identity-authority.sh")
    }

    fn process_state_dir(&self, process: ManagedProcess) -> PathBuf {
        self.run_dir.join(process.as_str())
    }

    fn pid_file(&self, process: ManagedProcess) -> PathBuf {
        self.pids_dir.join(format!("{process}.pid"))
    }

    fn env_values(&self) -> Vec<(&'static str, String)> {
        let mut values = vec![
            ("DEVFINITY_STATE_DIR", self.run_dir.display().to_string()),
            (
                "DEVFINITY_PROCESS_COMPOSE_FILE",
                self.process_compose_file.display().to_string(),
            ),
            (
                "DEVFINITY_PROCESS_COMPOSE_SOCKET",
                self.process_compose_socket.display().to_string(),
            ),
            ("DEVFINITY_LOGS_DIR", self.logs_dir.display().to_string()),
            ("DEVFINITY_PIDS_DIR", self.pids_dir.display().to_string()),
            ("DEVFINITY_POSTGRES_PORT", self.ports.postgres.to_string()),
            ("FC_WORKOS_AUTH_ENABLED", "0".to_string()),
            ("FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH", "1".to_string()),
            (
                "FC_WORKOS_OPERATOR_ORG_ID",
                WORKOS_FIXTURE_OPERATOR_ORG_ID.to_string(),
            ),
            (
                "FC_DASHBOARD_DEV_EMAIL",
                WORKOS_FIXTURE_CUSTOMER_EMAIL.to_string(),
            ),
            (
                "FC_DASHBOARD_DEV_WORKOS_USER_ID",
                WORKOS_FIXTURE_CUSTOMER_SUBJECT.to_string(),
            ),
            ("FC_CORE_URL", self.core_url()),
            ("FC_CORE_BASE_URL", self.core_url()),
            ("FC_CORE_DATABASE_URL", self.database_url()),
            ("FC_DASHBOARD_URL", self.dashboard_url()),
            ("FINITECHAT_SERVER_URL", self.finitechat_url()),
            (
                "FC_RUNNER_FINITECHAT_SERVER_URL",
                if self.profile.includes_runtime() {
                    self.runtime_finitechat_url()
                } else {
                    self.finitechat_url()
                },
            ),
            ("FC_HOSTED_WEB_DEVICE_URL", self.hosted_web_device_url()),
            (
                "FINITECHAT_HOSTED_BIND",
                format!("127.0.0.1:{}", self.ports.hosted_web_device),
            ),
            (
                "FINITECHAT_HOSTED_DATA_ROOT",
                self.hosted_web_device_dir().display().to_string(),
            ),
            (
                "FINITECHAT_HOSTED_API_TOKEN",
                self.hosted_web_device_token.clone(),
            ),
            ("FINITE_SITES_API", self.finitesites_api_url()),
            (
                "FC_SITES_UPSTREAM_URL",
                format!("http://127.0.0.1:{}", self.ports.finitesites),
            ),
            ("FC_SITES_ALLOW_LOCAL_OUTPUTS", "1".to_string()),
            (
                "FINITE_SITES_VIEWER_SESSION_TOKEN",
                self.sites_viewer_session_token.clone(),
            ),
            ("FINITE_BRAIN_URL", self.finite_brain_url()),
            ("FINITE_HOME", self.finite_home_dir().display().to_string()),
            ("DEVFINITY_PROFILE", self.profile.as_str().to_string()),
        ];
        if self.profile.includes_runtime() {
            values.push((
                "DEVFINITY_RUNTIME_URL",
                format!("http://127.0.0.1:{}", self.ports.runtime_agent),
            ));
        }
        values
    }
}

struct ProcessComposeGuard<'a> {
    stack: &'a Stack,
    child: Child,
    pid_file: PathBuf,
    shutdown_complete: bool,
}

impl ProcessComposeGuard<'_> {
    fn shutdown(&mut self) -> Result<()> {
        if self.shutdown_complete {
            return Ok(());
        }
        self.shutdown_complete = true;

        if self.stack.process_compose_socket.exists() && self.stack.process_compose_available() {
            let mut command = self.stack.process_compose_control_command();
            command.arg("down");
            run_best_effort(&mut command, "stop devfinity process-compose stack");
        }

        if wait_child_exit(&mut self.child, Duration::from_secs(10))?.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.stack.cleanup_managed_processes();
        self.stack.remove_secret_files();
        remove_file_best_effort(&self.stack.process_compose_socket);
        self.stack.remove_process_compose_control_dir();
        remove_file_best_effort(&self.pid_file);
        Ok(())
    }
}

impl Drop for ProcessComposeGuard<'_> {
    fn drop(&mut self) {
        if !self.shutdown_complete
            && let Err(error) = self.shutdown()
        {
            eprintln!("failed to shut down devfinity process-compose: {error:#}");
        }
    }
}

#[derive(Debug, Clone)]
struct ManagedProcessSpec {
    process: ManagedProcess,
    pid_file: PathBuf,
    expected_fragments: Vec<String>,
}

impl ManagedProcessSpec {
    fn new(process: ManagedProcess, pid_file: PathBuf, expected_fragments: Vec<String>) -> Self {
        Self {
            process,
            pid_file,
            expected_fragments,
        }
    }
}

#[derive(Debug, Clone)]
struct ManagedProcessRuntimeStatus {
    process: ManagedProcess,
    state: &'static str,
    detail: String,
}

impl ManagedProcessRuntimeStatus {
    fn new(process: ManagedProcess, state: &'static str, detail: String) -> Self {
        Self {
            process,
            state,
            detail,
        }
    }
}

#[derive(Debug, Clone)]
struct ServiceCheck {
    process: ManagedProcess,
    state: &'static str,
    detail: String,
}

impl ServiceCheck {
    fn new(process: ManagedProcess, state: &'static str, detail: String) -> Self {
        Self {
            process,
            state,
            detail,
        }
    }

    fn is_ready(&self) -> bool {
        matches!(self.state, "open" | "healthy")
    }
}

#[derive(Debug, Clone)]
struct ProcessInfo {
    pid: u32,
    ppid: u32,
    command: String,
}

fn process_table() -> Result<Vec<ProcessInfo>> {
    let output = Command::new("ps")
        .args(["axww", "-o", "pid=", "-o", "ppid=", "-o", "command="])
        .output()
        .context("failed to run ps")?;
    if !output.status.success() {
        bail!("ps exited with {}", output.status);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut processes = Vec::new();
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let Some(pid) = parts.next().and_then(|value| value.parse().ok()) else {
            continue;
        };
        let Some(ppid) = parts.next().and_then(|value| value.parse().ok()) else {
            continue;
        };
        let command = parts.collect::<Vec<_>>().join(" ");
        processes.push(ProcessInfo { pid, ppid, command });
    }
    Ok(processes)
}

fn read_pid_file(path: &Path) -> Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }
    let contents =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse()
        .map(Some)
        .with_context(|| format!("invalid pid in {}", path.display()))
}

fn process_matches(process: &ProcessInfo, expected_fragments: &[String]) -> bool {
    expected_fragments
        .iter()
        .all(|fragment| process.command.contains(fragment))
}

fn descendant_pids(table: &[ProcessInfo], root_pid: u32) -> Vec<u32> {
    let mut descendants = Vec::new();
    let mut stack = vec![root_pid];
    while let Some(parent) = stack.pop() {
        for process in table.iter().filter(|process| process.ppid == parent) {
            descendants.push(process.pid);
            stack.push(process.pid);
        }
    }
    descendants
}

fn terminate_processes(pids: &[u32]) {
    signal_processes(pids, "TERM");
    std::thread::sleep(std::time::Duration::from_millis(750));

    let alive: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|pid| process_alive(*pid))
        .collect();
    if !alive.is_empty() {
        signal_processes(&alive, "KILL");
    }
}

fn signal_processes(pids: &[u32], signal: &str) {
    for pid in pids {
        let status = Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(pid.to_string())
            .status();
        match status {
            Ok(status) if status.success() => {}
            Ok(status) => eprintln!("kill -{signal} {pid} exited with {status}"),
            Err(error) => eprintln!("failed to run kill -{signal} {pid}: {error}"),
        }
    }
}

fn process_alive(pid: u32) -> bool {
    let status = Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    matches!(status, Ok(status) if status.success())
}

fn remove_file_best_effort(path: &Path) {
    if path.exists()
        && let Err(error) = fs::remove_file(path)
    {
        eprintln!("failed to remove {}: {error}", path.display());
    }
}

fn check_tcp_service(process: ManagedProcess, host: &str, port: u16) -> ServiceCheck {
    match connect_tcp(host, port) {
        Ok(_) => ServiceCheck::new(process, "open", format!("tcp {host}:{port} accepted")),
        Err(error) => ServiceCheck::new(process, "down", format!("tcp {host}:{port}: {error}")),
    }
}

fn check_http_service(process: ManagedProcess, host: &str, port: u16, path: &str) -> ServiceCheck {
    match http_status_line(host, port, path) {
        Ok(status_line) => {
            let state = if http_status_is_ok(&status_line) {
                "healthy"
            } else {
                "unhealthy"
            };
            ServiceCheck::new(
                process,
                state,
                format!("http://{host}:{port}{path} {status_line}"),
            )
        }
        Err(error) => ServiceCheck::new(
            process,
            "down",
            format!("http://{host}:{port}{path}: {error}"),
        ),
    }
}

fn connect_tcp(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let addr: SocketAddr = format!("{host}:{port}").parse().map_err(|error| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{error}"))
    })?;
    TcpStream::connect_timeout(&addr, Duration::from_millis(500))
}

fn http_status_line(host: &str, port: u16, path: &str) -> std::io::Result<String> {
    let mut stream = connect_tcp(host, port)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response
        .lines()
        .next()
        .unwrap_or("no HTTP status line")
        .trim()
        .to_string())
}

fn http_status_is_ok(status_line: &str) -> bool {
    status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .is_some_and(|code| (200..400).contains(&code))
}

fn pending_service_checks(checks: &[ServiceCheck]) -> Vec<String> {
    checks
        .iter()
        .filter(|check| !check.is_ready())
        .map(|check| format!("{} {}", check.process, check.state))
        .collect()
}

fn absolute_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn run_status_with_pid_file(mut command: Command, pid_file: &Path) -> Result<ExitCode> {
    if let Some(parent) = pid_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to run {:?}", command))?;
    if let Err(error) = fs::write(pid_file, format!("{}\n", child.id())) {
        let _ = child.kill();
        bail!("failed to write {}: {error}", pid_file.display());
    }

    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {:?}", command))?;
    remove_file_best_effort(pid_file);
    Ok(status_to_exit_code(status))
}

fn status_to_exit_code(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) if (0..=255).contains(&code) => ExitCode::from(code as u8),
        _ => ExitCode::FAILURE,
    }
}

fn wait_child_exit(child: &mut Child, timeout: Duration) -> Result<Option<ExitStatus>> {
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .context("failed to wait for child process")?
        {
            return Ok(Some(status));
        }
        if started.elapsed() >= timeout {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run_best_effort(command: &mut Command, label: &str) {
    match command.status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("{label} exited with {status}"),
        Err(error) => eprintln!("failed to {label}: {error}"),
    }
}

fn run_required(command: &mut Command, label: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to {label}"))?;
    if !status.success() {
        bail!("failed to {label}: command exited with {status}");
    }
    Ok(())
}

fn command_stdout(command: &mut Command, label: &str) -> Result<String> {
    let output = command
        .output()
        .with_context(|| format!("failed to {label}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to {label}: {}", stderr.trim());
    }
    String::from_utf8(output.stdout)
        .with_context(|| format!("invalid UTF-8 while trying to {label}"))
}

fn ensure_apple_container_cli() -> Result<()> {
    let version = command_stdout(
        Command::new("container").arg("--version"),
        "run `container --version`",
    )
    .context(
        "Apple Container is required. Install the signed Apple `container` package, then run `container system start`",
    )?;
    let supported = version
        .split_whitespace()
        .find_map(|word| {
            let mut parts = word.split('.');
            let major = parts.next()?.parse::<u64>().ok()?;
            let minor = parts.next()?.parse::<u64>().ok()?;
            Some(major > 1 || (major == 1 && minor >= 1))
        })
        .unwrap_or(false);
    if !supported {
        bail!(
            "Apple Container 1.1 or newer is required; found {}",
            version.trim()
        );
    }

    let macos = command_stdout(
        Command::new("sw_vers").arg("-productVersion"),
        "read the macOS version",
    )?;
    let major = macos
        .trim()
        .split('.')
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    if major < 26 {
        bail!("Apple Container local SaaS requires macOS 26 or newer; found {macos:?}");
    }
    Ok(())
}

fn apple_container_system_running() -> Result<bool> {
    let status = command_stdout(
        Command::new("container").args(["system", "status", "--format", "json"]),
        "read Apple Container service status",
    )?;
    let value: serde_json::Value = serde_json::from_str(&status)
        .context("Apple Container returned invalid service-status JSON")?;
    Ok(value.get("status").and_then(serde_json::Value::as_str) == Some("running"))
}

fn detect_apple_host_access() -> Result<AppleHostAccess> {
    let domains = command_stdout(
        Command::new("container").args(["system", "dns", "list", "--quiet"]),
        "list Apple Container host DNS domains",
    )?;
    if domains.lines().any(|line| {
        line.trim_end_matches('.')
            .eq_ignore_ascii_case("host.container.internal")
    }) {
        return Ok(AppleHostAccess {
            runtime_host: "host.container.internal".to_string(),
            bind_host: "127.0.0.1".to_string(),
            source: "official Apple host DNS bridge",
        });
    }

    let network = command_stdout(
        Command::new("container").args(["network", "inspect", "default"]),
        "inspect the Apple Container default network",
    )?;
    let parsed: serde_json::Value = serde_json::from_str(&network)
        .context("Apple Container returned invalid default-network JSON")?;
    let gateway = parsed
        .get(0)
        .and_then(|network| network.get("status"))
        .and_then(|status| status.get("ipv4Gateway"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Apple Container did not report a default vmnet gateway. Configure the official bridge explicitly with `sudo container system dns create host.container.internal --localhost 203.0.113.113`, then rerun devfinity. Apple notes that this disables Private Relay and the packet-filter rule must be recreated after a reboot"
            )
        })?;
    let address = gateway
        .parse::<std::net::IpAddr>()
        .with_context(|| format!("Apple Container reported invalid gateway address {gateway:?}"))?;
    if !address.is_ipv4() || address.is_loopback() || address.is_unspecified() {
        bail!("Apple Container reported unusable vmnet gateway {gateway:?}");
    }
    Ok(AppleHostAccess {
        runtime_host: gateway.to_string(),
        bind_host: "0.0.0.0".to_string(),
        source: "Apple default-network gateway; runtime probe pending",
    })
}

fn shell_words(words: &[String]) -> String {
    words
        .iter()
        .map(|word| shell_quote(word))
        .collect::<Vec<_>>()
        .join(" ")
}

fn random_local_secret() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("local credential generation failed: {error:?}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn nonempty_env(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
}

fn optional_env_u16(name: &str, default: u16) -> Result<u16> {
    let Some(value) = std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(default);
    };
    let parsed = value
        .parse::<u16>()
        .with_context(|| format!("{name} must be an integer from 1 through 65535"))?;
    if parsed == 0 {
        bail!("{name} must be an integer from 1 through 65535");
    }
    Ok(parsed)
}

fn required_secret_env(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{name} is required for the selected inference mode"))
}

fn scrub_inference_secrets(command: &mut Command) {
    command.env_remove("FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY");
    command.env_remove("FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE");
}

fn write_mode_600(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    remove_file_best_effort(&temporary);
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary).with_context(|| {
        format!(
            "failed to create protected runtime file {}",
            temporary.display()
        )
    })?;
    file.write_all(bytes).with_context(|| {
        format!(
            "failed to write protected runtime file {}",
            temporary.display()
        )
    })?;
    file.sync_all().with_context(|| {
        format!(
            "failed to sync protected runtime file {}",
            temporary.display()
        )
    })?;
    #[cfg(unix)]
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(&temporary, path).with_context(|| {
        format!(
            "failed to activate protected runtime file {}",
            path.display()
        )
    })?;
    Ok(())
}

fn process_compose_control_dir(run_dir: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    run_dir.hash(&mut hasher);
    let directory_name = format!("devfinity-pc-{:016x}", hasher.finish());
    let preferred = std::env::temp_dir().join(&directory_name);
    if unix_socket_path_len(&preferred.join("pc.sock")) <= MACOS_UNIX_SOCKET_PATH_MAX {
        return preferred;
    }

    // TMPDIR can itself be arbitrarily deep. `/tmp` is a short, standard
    // fallback on Unix; the private per-state directory below prevents other
    // users from accessing the control socket.
    PathBuf::from("/tmp").join(directory_name)
}

fn unix_socket_path_len(path: &Path) -> usize {
    path.to_string_lossy().len()
}

fn ensure_private_dir(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)
                .with_context(|| format!("failed to inspect {}", path.display()))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                bail!(
                    "refusing to use process-compose control path {} because it is not a directory",
                    path.display()
                );
            }
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to create protected directory {}", path.display())
            });
        }
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to protect directory {}", path.display()))?;
    Ok(())
}

fn yaml_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_credential_metadata_matches_local_runner_identity() {
        let metadata: serde_json::Value =
            serde_json::from_str(&devfinity_runner_credentials_json()).unwrap();

        assert_eq!(
            metadata,
            serde_json::json!([{
                "credentialId": "devfinity-apple-current",
                "tokenEnv": "FC_CORE_RUNNER_CREDENTIAL_TOKEN_DEVFINITY_APPLE_CURRENT",
                "runnerId": "devfinity-apple-runner",
                "runnerClasses": ["apple_container"],
                "sourceHostId": "devfinity-apple",
                "revoked": false,
            }])
        );
    }

    #[test]
    fn core_uses_bound_runner_keyring_while_runner_keeps_process_token_env() {
        let state_dir = std::env::temp_dir().join(format!(
            "devfinity-test-runner-credential-wiring-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&state_dir);
        let stack = Stack::new(state_dir.clone())
            .unwrap()
            .with_profile(StackProfile::ServicesOnly);
        stack.ensure_dirs().unwrap();
        stack.write_secret_files().unwrap();

        let core_exports = fs::read_to_string(stack.core_secret_file()).unwrap();
        let runner_exports = fs::read_to_string(stack.runner_auth_secret_file()).unwrap();
        assert!(core_exports.contains(&format!(
            "export FC_CORE_RUNNER_CREDENTIALS_JSON={}\n",
            shell_quote(&devfinity_runner_credentials_json())
        )));
        assert!(core_exports.contains(&format!(
            "export {DEVFINITY_RUNNER_TOKEN_ENV}={}\n",
            shell_quote(DEVFINITY_RUNNER_TOKEN)
        )));
        assert!(!core_exports.contains("export FC_CORE_RUNNER_API_TOKEN="));
        assert_eq!(
            runner_exports,
            format!(
                "export FC_CORE_RUNNER_API_TOKEN={}\n",
                shell_quote(DEVFINITY_RUNNER_TOKEN)
            )
        );

        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn dashboard_secret_includes_core_service_token_without_core_only_credentials() {
        let state_dir = std::env::temp_dir().join(format!(
            "devfinity-test-dashboard-credential-wiring-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&state_dir);
        let stack = Stack::new(state_dir.clone())
            .unwrap()
            .with_profile(StackProfile::ServicesOnly);
        stack.ensure_dirs().unwrap();
        stack.write_secret_files().unwrap();

        let dashboard_exports = fs::read_to_string(stack.dashboard_auth_secret_file()).unwrap();
        assert!(dashboard_exports.contains("export FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN="));
        assert!(dashboard_exports.contains("export FC_CORE_API_TOKEN="));
        assert!(!dashboard_exports.contains("FC_CORE_RUNNER_CREDENTIALS_JSON"));
        assert!(!dashboard_exports.contains("FC_FINITE_PRIVATE_USAGE_API_TOKEN"));
        assert!(!dashboard_exports.contains("WORKOS_API_KEY"));

        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn env_exports_are_shell_quoted() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn managed_process_display_respects_format_width() {
        assert_eq!(format!("{:<16}", ManagedProcess::Core), "core            ");
    }

    #[test]
    fn generated_yaml_contains_core_services() {
        let mut stack = Stack::new(PathBuf::from(".local-state/devfinity")).unwrap();
        stack.ports.runtime_agent = 18081;
        stack.apple_container_name_prefix = "finite-devfinity-test".to_string();
        let yaml = stack.process_compose_yaml();
        assert!(yaml.contains("rust-build:"));
        assert!(yaml.contains("postgres:"));
        assert!(yaml.contains("core:"));
        assert!(yaml.contains("finitechat:"));
        assert!(yaml.contains("hosted-web-device:"));
        assert!(
            yaml.contains("finitechat:\n    description: \"Local Finite Chat delivery server\"")
        );
        assert_eq!(yaml.matches("restart: always").count(), 3);
        assert!(yaml.contains("workos-fixture:"));
        assert!(yaml.contains("workos-fixture --listen 127.0.0.1:14199"));
        assert!(yaml.contains("finitesites:"));
        assert!(yaml.contains("finite-brain:"));
        assert!(yaml.contains("finite-identity:"));
        assert!(yaml.contains("finite-identityd -- serve"));
        assert!(yaml.contains("FINITE_IDENTITY_AUTHORITY=http://127.0.0.1:18788"));
        assert!(yaml.contains("secrets/identity-authority.sh"));
        assert!(!yaml.contains("FINITE_IDENTITY_OPERATOR_TOKEN="));
        assert!(!yaml.contains("--operator-token"));
        assert!(yaml.contains("cargo run -p finite-brain-app"));
        assert!(yaml.contains("FINITE_BRAIN_PUBLIC_BASE_URL=http://127.0.0.1:13002"));
        assert!(
            yaml.contains("FINITE_BRAIN_SERVER_URL\\\":\\\"http://host.container.internal:18790")
        );
        assert!(yaml.contains("FINITE_BRAIN_PUBLIC_BASE_URL\\\":\\\"http://127.0.0.1:13002"));
        assert!(
            yaml.contains("FINITE_BRAIN_DEVELOPMENT_HTTP_HOST\\\":\\\"host.container.internal")
        );
        assert!(yaml.contains("FC_BRAIN_UPSTREAM_URL=http://127.0.0.1:18790"));
        assert!(yaml.contains("FC_BRAIN_PUBLIC_ORIGIN=http://127.0.0.1:13002"));
        assert!(yaml.contains("FC_SITES_UPSTREAM_URL=http://127.0.0.1:18789"));
        assert!(yaml.contains("FC_SITES_ALLOW_LOCAL_OUTPUTS=1"));
        assert!(
            yaml.contains(
                "FINITE_SITES_VIEWER_SESSION_TOKEN=dededededededededededededededededededededededededededededededede"
            )
        );
        assert!(yaml.contains("NEXT_DIST_DIR=.next-devfinity"));
        assert!(yaml.contains("--listen 0.0.0.0:18789"));
        assert!(yaml.contains("--api-url 'http://host.container.internal:18789'"));
        assert!(yaml.contains("--git-url 'http://host.container.internal:18789'"));
        assert!(yaml.contains("dashboard-deps:"));
        assert!(yaml.contains("dashboard:"));
        assert!(yaml.contains("runtime-image:"));
        assert!(yaml.contains("--engine apple-container"));
        assert!(yaml.contains("apple-network-probe:"));
        assert!(yaml.contains("runtime-artifact:"));
        assert!(yaml.contains("runtime-artifact-upsert"));
        assert!(yaml.contains(".image_metadata.digest"));
        assert!(yaml.contains("digest_hex=$(jq"));
        assert!(yaml.contains("artifact_id='devfinity-runtime'-\"$digest_hex\""));
        assert!(yaml.contains("runner-artifact.sh"));
        assert!(yaml.contains("--promoted"));
        assert!(yaml.contains("runner:"));
        assert!(yaml.contains("finite-saas-runner -- serve"));
        assert!(yaml.contains("FC_RUNNER_CLASS=apple_container"));
        assert!(yaml.contains("FC_RUNNER_APPLE_CONTAINER_NAME_PREFIX=finite-devfinity-test"));
        assert!(yaml.contains("FC_RUNNER_APPLE_CONTAINER_HOST_PORT=18081"));
        assert!(yaml.contains(
            "FC_RUNNER_APPLE_CONTAINER_LOCAL_IMAGE_REFERENCE=finite-agent-runtime:devfinity"
        ));
        assert!(!yaml.contains("FC_RUNNER_RUNTIME_ARTIFACT_ID=devfinity-runtime"));
        assert!(yaml.contains("FC_CORE_AGENT_CREATION_PLACEMENT_JSON="));
        assert!(!yaml.contains("FC_DASHBOARD_DEFAULT_RUNNER_CLASS"));
        assert!(!yaml.contains("FC_DASHBOARD_RUNNER_CLASSES"));
        assert!(yaml.contains("FC_RUNNER_RUNTIME_ENV_JSON="));
        assert!(yaml.contains("FC_CORE_RUNTIME_ENV_JSON="));
        assert!(yaml.contains("FINITE_SITES_API"));
        assert!(yaml.contains("FINITE_BRAIN_SERVER_URL"));
        assert!(!yaml.contains("FC_DASHBOARD_DEV_LAUNCH_CODE"));
        assert!(!yaml.contains("FC_CORE_RUNNER_API_TOKEN="));
        assert!(!yaml.contains("FC_FINITE_PRIVATE_USAGE_API_TOKEN="));
        assert!(yaml.contains("npm ci"));
        assert!(
            yaml.contains("dashboard-deps:\n        condition: process_completed_successfully")
        );
        assert!(yaml.contains("process_completed_successfully"));
        assert!(yaml.contains("process_healthy"));
        assert!(yaml.contains("DEVFINITY_MANAGED_PROCESS=1"));
        assert!(yaml.contains("pids/core.pid"));
        assert!(yaml.contains("run-postgres.sh"));
        assert!(yaml.contains("psql -h 127.0.0.1"));
        assert!(yaml.contains("cargo run -p finitechat-hosted-device"));
        assert!(yaml.contains("FINITECHAT_HOSTED_DATA_ROOT="));
        assert!(yaml.contains("FC_HOSTED_WEB_DEVICE_URL="));
        assert!(yaml.contains("FINITECHAT_HOSTED_API_TOKEN="));
        assert!(yaml.contains("hosted-web-device:\n        condition: process_healthy"));
        assert!(yaml.contains("finitesites:\n        condition: process_healthy"));
        assert!(!yaml.contains("postgres:16-alpine"));
        assert!(!yaml.contains("fpk_"));
    }

    #[test]
    fn gateway_fallback_separates_host_bind_and_runtime_addresses() {
        let mut stack = Stack::new(PathBuf::from(".local-state/devfinity")).unwrap();
        stack.apple_host_access = AppleHostAccess {
            runtime_host: "192.168.67.1".to_string(),
            bind_host: "0.0.0.0".to_string(),
            source: "test gateway",
        };

        let yaml = stack.process_compose_yaml();
        let finite_brain = yaml
            .split("  finite-brain:\n")
            .nth(1)
            .and_then(|tail| tail.split("\n  runtime-image:\n").next())
            .unwrap();

        assert!(finite_brain.contains("FINITE_BRAIN_ADDR=0.0.0.0:18790"));
        assert!(finite_brain.contains("host: \"127.0.0.1\""));
        assert!(yaml.contains("FC_BRAIN_UPSTREAM_URL=http://127.0.0.1:18790"));
        assert!(yaml.contains("FINITE_BRAIN_SERVER_URL\\\":\\\"http://192.168.67.1:18790"));
    }

    #[test]
    fn ordinary_start_preserves_previous_postgres_data() {
        let state_dir =
            std::env::temp_dir().join(format!("devfinity-test-prepare-{}", std::process::id()));
        let _ = fs::remove_dir_all(&state_dir);

        let mut stack = Stack::new(state_dir.clone()).unwrap();
        stack.ports.postgres = 0;
        let data_dir = stack.postgres_data_dir();
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("sentinel"), "stale").unwrap();

        stack.prepare_for_start().unwrap();

        assert_eq!(
            fs::read_to_string(data_dir.join("sentinel")).unwrap(),
            "stale"
        );
        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn explicit_fresh_services_profile_resets_service_state_only() {
        let state_dir = std::env::temp_dir().join(format!(
            "devfinity-test-fresh-services-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&state_dir);

        let mut stack = Stack::new(state_dir.clone())
            .unwrap()
            .with_profile(StackProfile::ServicesOnly)
            .with_fresh_services_state(true);
        stack.ports.postgres = 0;
        stack.ensure_dirs().unwrap();
        fs::create_dir_all(stack.postgres_data_dir()).unwrap();
        fs::write(stack.postgres_data_dir().join("sentinel"), "stale").unwrap();
        fs::write(stack.finite_identity_dir().join("sentinel"), "stale").unwrap();
        fs::write(stack.finite_brain_dir().join("sentinel"), "stale").unwrap();
        fs::write(stack.runtime_image_dir().join("sentinel"), "preserve").unwrap();
        fs::write(stack.runner_dir().join("sentinel"), "preserve").unwrap();

        stack.prepare_for_start().unwrap();

        assert!(!stack.postgres_data_dir().join("sentinel").exists());
        assert!(!stack.finite_identity_dir().join("sentinel").exists());
        assert!(!stack.finite_brain_dir().join("sentinel").exists());
        assert_eq!(
            fs::read_to_string(stack.runtime_image_dir().join("sentinel")).unwrap(),
            "preserve"
        );
        assert_eq!(
            fs::read_to_string(stack.runner_dir().join("sentinel")).unwrap(),
            "preserve"
        );
        let _ = fs::remove_dir_all(state_dir);
    }

    #[test]
    fn services_only_yaml_has_no_runtime_provider_processes() {
        let stack = Stack::new(PathBuf::from(".local-state/devfinity"))
            .unwrap()
            .with_profile(StackProfile::ServicesOnly);
        let yaml = stack.process_compose_yaml();

        assert!(!yaml.contains("runtime-image:"));
        assert!(!yaml.contains("runtime-artifact:"));
        assert!(!yaml.contains("apple-network-probe:"));
        assert!(!yaml.contains("finite-saas-runner -- serve"));
        assert!(yaml.contains("dashboard:"));
    }

    #[test]
    fn recovery_specs_include_optional_limiter_after_key_is_unset() {
        let mut stack = Stack::new(PathBuf::from(".local-state/devfinity")).unwrap();
        stack.inference_mode = InferenceMode::Missing;

        assert!(
            !stack
                .managed_process_specs()
                .iter()
                .any(|spec| spec.process == ManagedProcess::FinitePrivateLimiter)
        );
        assert!(
            stack
                .process_specs(ManagedProcess::ALL)
                .iter()
                .any(|spec| spec.process == ManagedProcess::FinitePrivateLimiter)
        );
    }

    #[test]
    fn inference_secrets_are_referenced_only_by_protected_owner_file() {
        let mut chained = Stack::new(PathBuf::from(".local-state/devfinity")).unwrap();
        chained.inference_mode = InferenceMode::ChainedLimiter;
        let chained_yaml = chained.process_compose_yaml();
        assert!(chained_yaml.contains("finite-private-limiter:"));
        assert!(chained_yaml.contains("secrets/finite-private-limiter.sh"));
        assert!(!chained_yaml.contains("FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY"));

        let mut direct = Stack::new(PathBuf::from(".local-state/devfinity")).unwrap();
        direct.inference_mode = InferenceMode::DirectKeyOverride;
        let direct_yaml = direct.process_compose_yaml();
        assert!(direct_yaml.contains("secrets/runner.sh"));
        assert!(!direct_yaml.contains("FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE"));
        assert!(!direct.env_exports().contains("API_KEY_OVERRIDE"));
    }

    #[cfg(unix)]
    #[test]
    fn protected_runtime_files_are_mode_600() {
        let path = std::env::temp_dir().join(format!("devfinity-mode-600-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        write_mode_600(&path, b"export TEST='<redacted>'\n").unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = fs::remove_file(path);
    }

    #[cfg(unix)]
    #[test]
    fn supervisor_environment_scrubs_inference_secrets() {
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "test -z \"${FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY:-}\" && test -z \"${FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE:-}\"",
            ])
            .env("FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY", "must-not-leak")
            .env(
                "FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE",
                "must-not-leak",
            );
        scrub_inference_secrets(&mut command);
        assert!(command.status().unwrap().success());
    }

    #[test]
    fn process_matching_requires_all_expected_fragments() {
        let process = ProcessInfo {
            pid: 1,
            ppid: 0,
            command: "cargo run -p finite-saas-core -- serve".to_string(),
        };

        assert!(process_matches(
            &process,
            &[
                "cargo".to_string(),
                "finite-saas-core".to_string(),
                "serve".to_string()
            ],
        ));
        assert!(!process_matches(
            &process,
            &["cargo".to_string(), "finitesitesd".to_string()],
        ));
    }

    #[test]
    fn core_process_spec_matches_started_binary() {
        let stack = Stack::new(PathBuf::from(".local-state/devfinity")).unwrap();
        let spec = stack
            .managed_process_specs()
            .into_iter()
            .find(|spec| spec.process == ManagedProcess::Core)
            .unwrap();
        let process = ProcessInfo {
            pid: 1,
            ppid: 0,
            command: "target/debug/finite-saas-core serve".to_string(),
        };

        assert!(process_matches(&process, &spec.expected_fragments));
    }

    #[test]
    fn descendant_pids_are_recursive() {
        let table = vec![
            ProcessInfo {
                pid: 10,
                ppid: 1,
                command: "parent".to_string(),
            },
            ProcessInfo {
                pid: 11,
                ppid: 10,
                command: "child".to_string(),
            },
            ProcessInfo {
                pid: 12,
                ppid: 11,
                command: "grandchild".to_string(),
            },
            ProcessInfo {
                pid: 20,
                ppid: 1,
                command: "unrelated".to_string(),
            },
        ];

        let mut descendants = descendant_pids(&table, 10);
        descendants.sort_unstable();
        assert_eq!(descendants, vec![11, 12]);
    }

    #[test]
    fn process_compose_socket_is_short_and_state_specific() {
        let long_segment = "nested-state-directory-".repeat(12);
        let first = Stack::new(PathBuf::from(format!("/{long_segment}/first"))).unwrap();
        let second = Stack::new(PathBuf::from(format!("/{long_segment}/second"))).unwrap();

        assert!(
            unix_socket_path_len(&first.process_compose_socket) <= MACOS_UNIX_SOCKET_PATH_MAX,
            "socket path is too long: {}",
            first.process_compose_socket.display()
        );
        assert!(!first.process_compose_socket.starts_with(&first.run_dir));
        assert_ne!(first.process_compose_socket, second.process_compose_socket);
        assert!(first.env_values().iter().any(|(name, value)| {
            *name == "DEVFINITY_PROCESS_COMPOSE_SOCKET"
                && value == &first.process_compose_socket.display().to_string()
        }));
    }

    #[test]
    fn process_compose_control_directory_is_private_and_removable() {
        let state_dir = std::env::temp_dir().join(format!(
            "devfinity-test-control-directory-{}",
            std::process::id()
        ));
        let stack = Stack::new(state_dir).unwrap();
        let _ = fs::remove_dir_all(&stack.process_compose_control_dir);

        ensure_private_dir(&stack.process_compose_control_dir).unwrap();
        assert!(stack.process_compose_control_dir.is_dir());
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&stack.process_compose_control_dir)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        fs::write(&stack.process_compose_socket, b"test").unwrap();
        remove_file_best_effort(&stack.process_compose_socket);
        stack.remove_process_compose_control_dir();
        assert!(!stack.process_compose_control_dir.exists());
    }

    #[test]
    fn process_compose_control_args_do_not_include_config() {
        let stack = Stack::new(PathBuf::from(".local-state/devfinity")).unwrap();
        let args = stack
            .process_compose_control_args()
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(args.contains(&"--use-uds".to_string()));
        assert!(args.contains(&"--unix-socket".to_string()));
        assert!(!args.contains(&"--config".to_string()));
        assert!(!args.contains(&"--disable-dotenv".to_string()));

        let up_args = stack
            .process_compose_up_command()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(up_args.contains(&"--disable-dotenv".to_string()));
    }
}
