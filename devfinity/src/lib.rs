use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy)]
pub enum ProcessComposeMode {
    Tui,
    Headless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedProcess {
    ProcessCompose,
    RustBuild,
    Postgres,
    Core,
    FiniteChat,
    FiniteSites,
    DashboardDeps,
    Dashboard,
}

impl ManagedProcess {
    const ALL: [Self; 8] = [
        Self::ProcessCompose,
        Self::RustBuild,
        Self::Postgres,
        Self::Core,
        Self::FiniteChat,
        Self::FiniteSites,
        Self::DashboardDeps,
        Self::Dashboard,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::ProcessCompose => "process-compose",
            Self::RustBuild => "rust-build",
            Self::Postgres => "postgres",
            Self::Core => "core",
            Self::FiniteChat => "finitechat",
            Self::FiniteSites => "finitesites",
            Self::DashboardDeps => "dashboard-deps",
            Self::Dashboard => "dashboard",
        }
    }
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
    process_compose_socket: PathBuf,
    ports: Ports,
    core_token: String,
}

#[derive(Debug, Clone)]
struct Ports {
    core: u16,
    dashboard: u16,
    postgres: u16,
    finitechat: u16,
    finitesites: u16,
}

impl Stack {
    pub fn new(state_dir: PathBuf) -> Result<Self> {
        let repo_root = std::env::current_dir().context("failed to read current directory")?;
        let state_dir = absolute_path(&repo_root, &state_dir);
        let run_dir = state_dir.join("runs").join("default");
        let logs_dir = run_dir.join("logs");
        let pids_dir = run_dir.join("pids");
        Ok(Self {
            repo_root,
            process_compose_file: run_dir.join("process-compose.yaml"),
            process_compose_socket: run_dir.join("process-compose.sock"),
            state_dir,
            run_dir,
            logs_dir,
            pids_dir,
            ports: Ports {
                core: 14200,
                dashboard: 13002,
                postgres: 15432,
                finitechat: 18787,
                finitesites: 18789,
            },
            core_token: "devfinity-core-token".to_string(),
        })
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
            &self.finitesites_dir(),
            &self.finite_home_dir(),
        ] {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        Ok(())
    }

    pub fn write_files(&self) -> Result<()> {
        self.ensure_dirs()?;
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

    pub fn write_env_file(&self) -> Result<()> {
        fs::write(self.run_dir.join("env"), self.env_exports())
            .with_context(|| format!("failed to write {}", self.run_dir.join("env").display()))
    }

    pub fn print_summary(&self) {
        println!("devfinity local stack");
        println!("  state:      {}", self.run_dir.display());
        println!("  logs:       {}", self.logs_dir.display());
        println!("  config:     {}", self.process_compose_file.display());
        println!("  socket:     {}", self.process_compose_socket.display());
        println!("  dashboard:  {}", self.dashboard_url());
        println!("  core:       {}", self.core_url());
        println!("  chat:       {}", self.finitechat_url());
        println!("  sites api:  {}", self.finitesites_api_url());
        println!(
            "  sites base: http://*.sites.localhost:{}",
            self.ports.finitesites
        );
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
        run_status_with_pid_file(command, &self.pid_file(ManagedProcess::ProcessCompose))
    }

    pub fn run_wrapped_command(&self, command: &[String]) -> Result<ExitCode> {
        if command.is_empty() {
            bail!("wrapped command cannot be empty");
        }

        self.ensure_process_compose_available()?;
        self.prepare_for_start()?;
        let mut guard = self.start_process_compose_headless()?;
        let outcome = match self.wait_for_services_ready(Duration::from_secs(180), &mut guard) {
            Ok(()) => self.run_stack_command(command),
            Err(error) => Err(error),
        };

        if let Err(error) = guard.shutdown() {
            eprintln!("devfinity cleanup after wrapped command failed: {error:#}");
        }

        outcome
    }

    pub fn prepare_for_start(&self) -> Result<()> {
        self.ensure_postgres_not_running()?;
        let data_dir = self.postgres_data_dir();
        if data_dir.exists() {
            fs::remove_dir_all(&data_dir)
                .with_context(|| format!("failed to remove {}", data_dir.display()))?;
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

        let process_compose_pid_file = self.pid_file(ManagedProcess::ProcessCompose);
        for path in [&self.process_compose_socket, &process_compose_pid_file] {
            if path.exists() {
                if let Err(error) = fs::remove_file(path) {
                    eprintln!("failed to remove {}: {error}", path.display());
                }
            }
        }

        println!("devfinity cleanup complete");
        Ok(ExitCode::SUCCESS)
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
        self.write_postgres(&mut yaml);
        self.write_core(&mut yaml);
        self.write_finitechat(&mut yaml);
        self.write_finitesites(&mut yaml);
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
        self.write_managed_command(
            yaml,
            process,
            &[String::from(
                "exec cargo build -p finite-saas-core -p finitechat-server -p finitesitesd",
            )],
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

postgres -D "$PGDATA" -h 127.0.0.1 -p "$port" &
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
            &[String::from("exec cargo run -p finite-saas-core -- serve")],
            &[],
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::Postgres);
        let _ = writeln!(yaml, "        condition: process_healthy");
        self.write_environment(
            yaml,
            &[
                ("FC_CORE_DATABASE_URL", self.database_url()),
                ("FC_CORE_API_TOKEN", self.core_token.clone()),
                ("FC_CORE_BIND", format!("127.0.0.1:{}", self.ports.core)),
            ],
        );
        self.write_http_probe(yaml, "/healthz", self.ports.core, 2, 2, 3, 45);
    }

    fn write_finitechat(&self, yaml: &mut String) {
        let process = ManagedProcess::FiniteChat;
        let sqlite = self.finitechat_dir().join("server.sqlite3");
        let command = format!(
            "cargo run -p finitechat-server -- serve 127.0.0.1:{} --sqlite {}",
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
        self.write_http_probe(yaml, "/health", self.ports.finitechat, 1, 2, 3, 45);
    }

    fn write_finitesites(&self, yaml: &mut String) {
        let process = ManagedProcess::FiniteSites;
        let data = self.finitesites_dir();
        let command = format!(
            concat!(
                "cargo run -p finitesitesd -- serve ",
                "--data {} ",
                "--listen 127.0.0.1:{} ",
                "--api-url http://127.0.0.1:{} ",
                "--base-domain sites.localhost ",
                "--document-base-domain docs.sites.localhost ",
                "--git-url http://git.sites.localhost:{} ",
                "--site-port {} ",
                "--app-runner none"
            ),
            shell_quote(&data.display().to_string()),
            self.ports.finitesites,
            self.ports.finitesites,
            self.ports.finitesites,
            self.ports.finitesites
        );
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(yaml, "Local Finite Sites server", &self.repo_root, process);
        self.write_managed_command(yaml, process, &[format!("exec {command}")], &[]);
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::RustBuild);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        self.write_http_probe(yaml, "/api/v1/healthz", self.ports.finitesites, 1, 2, 3, 45);
    }

    fn write_dashboard(&self, yaml: &mut String) {
        let process = ManagedProcess::Dashboard;
        let dashboard_dir = self.repo_root.join("finitecomputer-v2/apps/dashboard");
        let _ = writeln!(yaml, "  {process}:");
        self.write_process_header(yaml, "Finite dashboard dev server", &dashboard_dir, process);
        self.write_managed_command(
            yaml,
            process,
            &[format!(
                "exec npm run dev -- --hostname 127.0.0.1 --port {}",
                self.ports.dashboard
            )],
            &[],
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::DashboardDeps);
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      {}:", ManagedProcess::Core);
        let _ = writeln!(yaml, "        condition: process_healthy");
        self.write_environment(
            yaml,
            &[
                ("FC_WORKOS_AUTH_ENABLED", "0".to_string()),
                ("FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH", "1".to_string()),
                (
                    "FC_DASHBOARD_DEV_EMAIL",
                    "devfinity@finite.computer".to_string(),
                ),
                (
                    "FC_DASHBOARD_DEV_WORKOS_USER_ID",
                    "user_devfinity".to_string(),
                ),
                ("FC_CORE_BASE_URL", self.core_url()),
                ("FC_CORE_API_TOKEN", self.core_token.clone()),
                (
                    "NEXT_PUBLIC_WORKOS_REDIRECT_URI",
                    format!("http://127.0.0.1:{}/callback", self.ports.dashboard),
                ),
            ],
        );
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
        let _ = writeln!(yaml, "    readiness_probe:");
        let _ = writeln!(yaml, "      http_get:");
        let _ = writeln!(yaml, "        host: \"127.0.0.1\"");
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
            .arg("--config")
            .arg(&self.process_compose_file)
            .args(self.process_compose_control_args());
        command
    }

    fn process_compose_control_command(&self) -> Command {
        let mut command = Command::new("process-compose");
        command.args(self.process_compose_control_args());
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
        if let Some(pid) = read_pid_file(&pid_file)? {
            if process_alive(pid) {
                bail!(
                    "devfinity postgres pid {pid} from {} is still running; run `devfinity cleanup` before starting a new stack",
                    pid_file.display()
                );
            }
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
        let status = Command::new(program)
            .args(args)
            .current_dir(&self.repo_root)
            .envs(self.env_values())
            .status()
            .with_context(|| {
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

        for spec in self.managed_process_specs() {
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
        ManagedProcess::ALL
            .into_iter()
            .map(|process| {
                let expected_fragments = match process {
                    ManagedProcess::ProcessCompose => vec![
                        ManagedProcess::ProcessCompose.as_str().to_string(),
                        self.process_compose_file.display().to_string(),
                    ],
                    ManagedProcess::RustBuild => vec![String::from("cargo"), String::from("build")],
                    ManagedProcess::Postgres => vec![
                        String::from("bash"),
                        self.postgres_script_path().display().to_string(),
                    ],
                    ManagedProcess::Core => vec![
                        String::from("cargo"),
                        String::from("finite-saas-core"),
                        String::from("serve"),
                    ],
                    ManagedProcess::FiniteChat => vec![
                        String::from("finitechat-server"),
                        self.finitechat_dir().display().to_string(),
                    ],
                    ManagedProcess::FiniteSites => vec![
                        String::from("finitesitesd"),
                        self.finitesites_dir().display().to_string(),
                    ],
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

    fn service_checks(&self) -> Vec<ServiceCheck> {
        vec![
            check_tcp_service(ManagedProcess::Postgres, "127.0.0.1", self.ports.postgres),
            check_http_service(
                ManagedProcess::Core,
                "127.0.0.1",
                self.ports.core,
                "/healthz",
            ),
            check_http_service(
                ManagedProcess::FiniteChat,
                "127.0.0.1",
                self.ports.finitechat,
                "/health",
            ),
            check_http_service(
                ManagedProcess::FiniteSites,
                "127.0.0.1",
                self.ports.finitesites,
                "/api/v1/healthz",
            ),
            check_http_service(
                ManagedProcess::Dashboard,
                "127.0.0.1",
                self.ports.dashboard,
                "/dashboard",
            ),
        ]
    }

    fn urls_text(&self) -> String {
        format!(
            concat!(
                "dashboard={}\n",
                "core={}\n",
                "finitechat={}\n",
                "finitesites_api={}\n",
                "finitesites_base=http://*.sites.localhost:{}\n"
            ),
            self.dashboard_url(),
            self.core_url(),
            self.finitechat_url(),
            self.finitesites_api_url(),
            self.ports.finitesites
        )
    }

    fn core_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.core)
    }

    fn dashboard_url(&self) -> String {
        format!("http://127.0.0.1:{}/dashboard", self.ports.dashboard)
    }

    fn finitechat_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.finitechat)
    }

    fn finitesites_api_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.finitesites)
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

    fn finitesites_dir(&self) -> PathBuf {
        self.process_state_dir(ManagedProcess::FiniteSites)
    }

    fn finite_home_dir(&self) -> PathBuf {
        self.run_dir.join("finite-home")
    }

    fn process_state_dir(&self, process: ManagedProcess) -> PathBuf {
        self.run_dir.join(process.as_str())
    }

    fn pid_file(&self, process: ManagedProcess) -> PathBuf {
        self.pids_dir.join(format!("{process}.pid"))
    }

    fn env_values(&self) -> Vec<(&'static str, String)> {
        vec![
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
                "FC_DASHBOARD_DEV_EMAIL",
                "devfinity@finite.computer".to_string(),
            ),
            (
                "FC_DASHBOARD_DEV_WORKOS_USER_ID",
                "user_devfinity".to_string(),
            ),
            ("FC_CORE_URL", self.core_url()),
            ("FC_CORE_BASE_URL", self.core_url()),
            ("FC_CORE_API_TOKEN", self.core_token.clone()),
            ("FC_CORE_DATABASE_URL", self.database_url()),
            ("FC_DASHBOARD_URL", self.dashboard_url()),
            ("FINITECHAT_SERVER_URL", self.finitechat_url()),
            ("FC_RUNNER_FINITECHAT_SERVER_URL", self.finitechat_url()),
            ("FINITE_SITES_API", self.finitesites_api_url()),
            ("FINITE_HOME", self.finite_home_dir().display().to_string()),
        ]
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
        remove_file_best_effort(&self.stack.process_compose_socket);
        remove_file_best_effort(&self.pid_file);
        Ok(())
    }
}

impl Drop for ProcessComposeGuard<'_> {
    fn drop(&mut self) {
        if !self.shutdown_complete {
            if let Err(error) = self.shutdown() {
                eprintln!("failed to shut down devfinity process-compose: {error:#}");
            }
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
    if path.exists() {
        if let Err(error) = fs::remove_file(path) {
            eprintln!("failed to remove {}: {error}", path.display());
        }
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

fn shell_words(words: &[String]) -> String {
    words
        .iter()
        .map(|word| shell_quote(word))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
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
    fn env_exports_are_shell_quoted() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn managed_process_display_respects_format_width() {
        assert_eq!(format!("{:<16}", ManagedProcess::Core), "core            ");
    }

    #[test]
    fn generated_yaml_contains_core_services() {
        let stack = Stack::new(PathBuf::from(".local-state/devfinity")).unwrap();
        let yaml = stack.process_compose_yaml();
        assert!(yaml.contains("rust-build:"));
        assert!(yaml.contains("postgres:"));
        assert!(yaml.contains("core:"));
        assert!(yaml.contains("finitechat:"));
        assert!(yaml.contains("finitesites:"));
        assert!(yaml.contains("dashboard-deps:"));
        assert!(yaml.contains("dashboard:"));
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
        assert!(!yaml.contains("postgres:16-alpine"));
    }

    #[test]
    fn prepare_for_start_removes_previous_postgres_data() {
        let state_dir =
            std::env::temp_dir().join(format!("devfinity-test-prepare-{}", std::process::id()));
        let _ = fs::remove_dir_all(&state_dir);

        let mut stack = Stack::new(state_dir.clone()).unwrap();
        stack.ports.postgres = 0;
        let data_dir = stack.postgres_data_dir();
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("sentinel"), "stale").unwrap();

        stack.prepare_for_start().unwrap();

        assert!(!data_dir.exists());
        let _ = fs::remove_dir_all(state_dir);
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
    }
}
