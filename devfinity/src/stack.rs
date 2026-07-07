use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Command, ExitCode, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::process::{ProcessHandle, ProcessManager, ProcessSpec};
use crate::topology::{DevfinityStack, shell_quote};

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackRunMode {
    Foreground,
    Headless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedProcess {
    Postgres,
    Core,
    FiniteChat,
    FiniteSites,
}

impl ManagedProcess {
    fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Core => "core",
            Self::FiniteChat => "finitechat",
            Self::FiniteSites => "finitesites",
        }
    }

    fn all() -> &'static [ManagedProcess] {
        &[
            Self::Postgres,
            Self::Core,
            Self::FiniteChat,
            Self::FiniteSites,
        ]
    }
}

impl fmt::Display for ManagedProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

impl DevfinityStack {
    pub fn write_files(&self) -> Result<()> {
        self.ensure_dirs()?;
        self.clear_run_markers();
        self.write_env_file()?;
        self.write_postgres_script()?;
        fs::write(self.run_dir.join("urls.txt"), self.urls_text()).with_context(|| {
            format!(
                "failed to write {}",
                self.run_dir.join("urls.txt").display()
            )
        })?;
        Ok(())
    }

    pub fn run_up(&self, mode: StackRunMode, dry_run: bool) -> Result<ExitCode> {
        self.write_files()?;
        self.validate_command_plan()?;
        if dry_run {
            println!("devfinity dry run ok");
            return Ok(ExitCode::SUCCESS);
        }

        let mut running = self.start()?;

        match mode {
            StackRunMode::Foreground => println!(
                "devfinity stack is ready; logs: {}; press Ctrl-C to stop",
                self.logs_dir.display()
            ),
            StackRunMode::Headless => println!(
                "devfinity stack is ready in headless mode; logs: {}; press Ctrl-C to stop",
                self.logs_dir.display()
            ),
        }

        running.wait_until_shutdown()
    }

    pub fn run_wrapped_command(&self, command: &[String]) -> Result<ExitCode> {
        if command.is_empty() {
            bail!("wrapped command cannot be empty");
        }

        let mut running = self.start()?;
        let outcome = self.run_stack_command(command);

        if let Err(error) = running.shutdown() {
            eprintln!("devfinity cleanup after wrapped command failed: {error:#}");
        }

        outcome
    }

    pub fn start(&self) -> Result<RunningDevfinityStack> {
        let outcome = (|| {
            self.write_files()?;
            self.validate_command_plan()?;
            self.prepare_for_start()?;
            let mut running = self.start_managed_stack()?;
            self.wait_for_services_ready(Duration::from_secs(180), &mut running)?;
            self.mark_ready()?;
            Ok(running)
        })();

        if outcome.is_err() {
            let _ = self.mark_error();
        }

        outcome
    }

    pub fn prepare_for_start(&self) -> Result<()> {
        self.ensure_ports_free()?;
        let data_dir = self.postgres_data_dir();
        if data_dir.exists() {
            fs::remove_dir_all(&data_dir)
                .with_context(|| format!("failed to remove {}", data_dir.display()))?;
        }
        Ok(())
    }

    pub fn cleanup(&self) -> Result<ExitCode> {
        self.process_manager().cleanup_stale_processes()?;
        Ok(ExitCode::SUCCESS)
    }

    pub fn status(&self) -> Result<ExitCode> {
        println!("devfinity status");
        println!("  state:   {}", self.run_dir.display());
        println!("  logs:    {}", self.logs_dir.display());
        println!("  control: {}", self.control_dir().display());
        println!("  env:     {}", self.run_dir.join("env").display());
        println!("  urls:    {}", self.run_dir.join("urls.txt").display());
        println!();

        println!("processes:");
        let names = ManagedProcess::all()
            .iter()
            .map(|process| process.as_str())
            .collect::<Vec<_>>();
        for status in self.process_manager().statuses(&names) {
            match (status.pid, status.running) {
                (Some(pid), true) => println!("  {:<16} running pid={pid}", status.name),
                (Some(pid), false) => println!("  {:<16} stopped pid={pid}", status.name),
                (None, _) => println!(
                    "  {:<16} stopped ({})",
                    status.name,
                    status.pid_file.display()
                ),
            }
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

    fn process_manager(&self) -> ProcessManager {
        ProcessManager::new(self.control_dir(), self.logs_dir.clone())
    }

    fn start_managed_stack(&self) -> Result<RunningDevfinityStack> {
        let manager = self.process_manager();
        self.run_rust_build(&manager)?;

        let mut running = RunningDevfinityStack::new();
        running.spawn(&manager, self.postgres_spec())?;
        self.wait_for_postgres_ready(Duration::from_secs(45), &mut running)?;

        running.spawn(&manager, self.core_spec())?;
        running.spawn(&manager, self.finitechat_spec())?;
        running.spawn(&manager, self.finitesites_spec())?;
        self.wait_for_core_ready(Duration::from_secs(60), &mut running)?;

        Ok(running)
    }

    fn run_rust_build(&self, manager: &ProcessManager) -> Result<()> {
        let mut command = Command::new("cargo");
        command
            .arg("build")
            .arg("-p")
            .arg("finite-saas-core")
            .arg("-p")
            .arg("finitechat-server")
            .arg("-p")
            .arg("finitesitesd")
            .current_dir(&self.repo_root);
        let status = manager.run_command("rust-build", &mut command)?;
        ensure_status_success("rust-build", status, &self.logs_dir.join("rust-build.log"))
    }

    fn wait_for_postgres_ready(
        &self,
        timeout: Duration,
        running: &mut RunningDevfinityStack,
    ) -> Result<()> {
        let started = Instant::now();
        loop {
            running.check_processes()?;
            if self.postgres_ready() {
                return Ok(());
            }
            if started.elapsed() >= timeout {
                bail!(
                    "postgres did not become ready within {}s; see {}",
                    timeout.as_secs(),
                    self.logs_dir.join("postgres.log").display()
                );
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    fn wait_for_core_ready(
        &self,
        timeout: Duration,
        running: &mut RunningDevfinityStack,
    ) -> Result<()> {
        let started = Instant::now();
        loop {
            running.check_processes()?;
            if check_http_service(
                ManagedProcess::Core,
                "127.0.0.1",
                self.ports.core,
                "/healthz",
            )
            .is_ready()
            {
                return Ok(());
            }
            if started.elapsed() >= timeout {
                bail!(
                    "core did not become ready within {}s; see {}",
                    timeout.as_secs(),
                    self.logs_dir.join("core.log").display()
                );
            }
            std::thread::sleep(Duration::from_millis(750));
        }
    }

    fn wait_for_services_ready(
        &self,
        timeout: Duration,
        running: &mut RunningDevfinityStack,
    ) -> Result<()> {
        let started = Instant::now();
        let mut last_report = Instant::now() - Duration::from_secs(5);
        loop {
            running.check_processes()?;

            let checks = self.service_checks();
            let pending = pending_service_checks(&checks);
            if pending.is_empty() {
                println!("all devfinity services are ready");
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
        ]
    }

    fn ensure_ports_free(&self) -> Result<()> {
        for (name, port) in [
            ("postgres", self.ports.postgres),
            ("core", self.ports.core),
            ("finitechat", self.ports.finitechat),
            ("finitesites", self.ports.finitesites),
        ] {
            if connect_tcp("127.0.0.1", port).is_ok() {
                bail!(
                    "{name} port 127.0.0.1:{port} is already accepting connections; stop the existing service or run `devfinity cleanup` before starting devfinity"
                );
            }
        }
        Ok(())
    }

    fn validate_command_plan(&self) -> Result<()> {
        for command in [
            "bash",
            "cargo",
            "initdb",
            "postgres",
            "pg_isready",
            "psql",
            "createdb",
        ] {
            ensure_command_available(command)?;
        }
        Ok(())
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

    fn postgres_ready(&self) -> bool {
        let port = self.ports.postgres.to_string();
        Command::new("psql")
            .arg("-h")
            .arg("127.0.0.1")
            .arg("-p")
            .arg(port)
            .arg("-U")
            .arg("postgres")
            .arg("-d")
            .arg("finite_saas_core")
            .arg("-tAc")
            .arg("select 1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn postgres_spec(&self) -> ProcessSpec {
        ProcessSpec::new(
            ManagedProcess::Postgres.as_str(),
            "bash",
            self.repo_root.clone(),
            self.logs_dir.join("postgres.log"),
        )
        .args([self.postgres_script_path().display().to_string()])
        .env(self.process_env())
    }

    fn core_spec(&self) -> ProcessSpec {
        ProcessSpec::new(
            ManagedProcess::Core.as_str(),
            self.repo_root
                .join("target/debug/finite-saas-core")
                .display()
                .to_string(),
            self.repo_root.clone(),
            self.logs_dir.join("core.log"),
        )
        .args(["serve"])
        .env(
            self.process_env()
                .into_iter()
                .chain([
                    (
                        "FC_CORE_BIND".to_string(),
                        format!("127.0.0.1:{}", self.ports.core),
                    ),
                    ("FC_CORE_DATABASE_URL".to_string(), self.database_url()),
                    ("FC_CORE_API_TOKEN".to_string(), self.core_token.clone()),
                ])
                .collect::<Vec<_>>(),
        )
    }

    fn finitechat_spec(&self) -> ProcessSpec {
        ProcessSpec::new(
            ManagedProcess::FiniteChat.as_str(),
            self.repo_root
                .join("target/debug/finitechat-server")
                .display()
                .to_string(),
            self.repo_root.clone(),
            self.logs_dir.join("finitechat.log"),
        )
        .args([
            "serve".to_string(),
            format!("127.0.0.1:{}", self.ports.finitechat),
            "--sqlite".to_string(),
            self.finitechat_dir()
                .join("server.sqlite3")
                .display()
                .to_string(),
        ])
        .env(self.process_env())
    }

    fn finitesites_spec(&self) -> ProcessSpec {
        let port = self.ports.finitesites.to_string();
        ProcessSpec::new(
            ManagedProcess::FiniteSites.as_str(),
            self.repo_root
                .join("target/debug/finitesitesd")
                .display()
                .to_string(),
            self.repo_root.clone(),
            self.logs_dir.join("finitesites.log"),
        )
        .args([
            "serve".to_string(),
            "--data".to_string(),
            self.finitesites_dir().display().to_string(),
            "--listen".to_string(),
            format!("127.0.0.1:{port}"),
            "--api-url".to_string(),
            format!("http://127.0.0.1:{port}"),
            "--base-domain".to_string(),
            "sites.localhost".to_string(),
            "--document-base-domain".to_string(),
            "docs.sites.localhost".to_string(),
            "--git-url".to_string(),
            format!("http://git.sites.localhost:{port}"),
            "--site-port".to_string(),
            port,
            "--app-runner".to_string(),
            "none".to_string(),
        ])
        .env(self.process_env())
    }

    fn process_env(&self) -> Vec<(String, String)> {
        self.env_values()
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }
}

#[derive(Debug)]
pub struct RunningDevfinityStack {
    handles: Vec<ProcessHandle>,
}

impl RunningDevfinityStack {
    fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }

    fn spawn(&mut self, manager: &ProcessManager, spec: ProcessSpec) -> Result<()> {
        let handle = manager.spawn(spec)?;
        println!("starting {} pid={}", handle.name(), handle.pid());
        self.handles.push(handle);
        Ok(())
    }

    fn check_processes(&mut self) -> Result<()> {
        for handle in &mut self.handles {
            if let Some(status) = handle.try_exit_status()? {
                bail!(
                    "{} exited before devfinity finished: {status}; see {}",
                    handle.name(),
                    handle.log_path().display()
                );
            }
        }
        Ok(())
    }

    fn wait_until_shutdown(&mut self) -> Result<ExitCode> {
        install_shutdown_signal_handlers();
        while !SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
            self.check_processes()?;
            std::thread::sleep(Duration::from_millis(500));
        }
        println!("shutdown requested; stopping devfinity stack");
        self.shutdown()?;
        Ok(ExitCode::SUCCESS)
    }

    pub fn shutdown(&mut self) -> Result<()> {
        let mut first_error = None;
        for handle in self.handles.iter_mut().rev() {
            if let Err(error) = handle.shutdown() {
                eprintln!("failed to stop {}: {error:#}", handle.name());
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for RunningDevfinityStack {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            eprintln!("failed to shut down devfinity stack: {error:#}");
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

fn ensure_status_success(name: &str, status: ExitStatus, log_path: &Path) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    bail!("{name} exited with {status}; see {}", log_path.display())
}

fn ensure_command_available(command: &str) -> Result<()> {
    let check = Command::new("bash")
        .arg("-lc")
        .arg(format!("command -v {}", shell_quote(command)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to check whether `{command}` is available"))?;
    if check.success() {
        return Ok(());
    }
    bail!("`{command}` was not found; run devfinity through the pinned Nix environment")
}

fn status_to_exit_code(status: ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) if (0..=255).contains(&code) => ExitCode::from(code as u8),
        _ => ExitCode::FAILURE,
    }
}

fn shell_words(words: &[String]) -> String {
    words
        .iter()
        .map(|word| shell_quote(word))
        .collect::<Vec<_>>()
        .join(" ")
}

fn install_shutdown_signal_handlers() {
    SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
    install_signal_handler(2);
    install_signal_handler(15);
}

#[cfg(unix)]
fn install_signal_handler(signal_number: i32) {
    unsafe extern "C" {
        fn signal(signal: i32, handler: extern "C" fn(i32)) -> extern "C" fn(i32);
    }

    extern "C" fn request_shutdown(_: i32) {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    }

    unsafe {
        let _ = signal(signal_number, request_shutdown);
    }
}

#[cfg(not(unix))]
fn install_signal_handler(_: i32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn write_files_does_not_generate_process_compose_config() {
        let repo = std::env::temp_dir().join(format!("devfinity-repo-{}", now_millis()));
        let state = repo.join(".state");
        fs::create_dir_all(&repo).expect("repo dir");

        let stack = DevfinityStack::new_with_repo_root(repo.clone(), state).expect("stack");
        stack.write_files().expect("write files");

        assert!(stack.paths().run_dir.join("env").exists());
        assert!(stack.paths().run_dir.join("urls.txt").exists());
        assert!(stack.paths().postgres_script.exists());
        assert!(!stack.paths().run_dir.join("process-compose.yaml").exists());

        let _ = fs::remove_dir_all(repo);
    }

    fn now_millis() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0)
    }
}
