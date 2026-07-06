use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy)]
pub enum ProcessComposeMode {
    Tui,
    Headless,
}

#[derive(Debug, Clone)]
pub struct Stack {
    repo_root: PathBuf,
    state_dir: PathBuf,
    run_dir: PathBuf,
    logs_dir: PathBuf,
    process_compose_file: PathBuf,
    process_compose_socket: PathBuf,
    postgres_container: String,
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
        Ok(Self {
            repo_root,
            process_compose_file: run_dir.join("process-compose.yaml"),
            process_compose_socket: run_dir.join("process-compose.sock"),
            state_dir,
            run_dir,
            logs_dir,
            postgres_container: "devfinity-postgres".to_string(),
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
        let env = [
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
            ("FC_CORE_URL", self.core_url()),
            ("FC_CORE_BASE_URL", self.core_url()),
            ("FC_CORE_API_TOKEN", self.core_token.clone()),
            ("FC_CORE_DATABASE_URL", self.database_url()),
            ("FC_DASHBOARD_URL", self.dashboard_url()),
            ("FINITECHAT_SERVER_URL", self.finitechat_url()),
            ("FC_RUNNER_FINITECHAT_SERVER_URL", self.finitechat_url()),
            ("FINITE_SITES_API", self.finitesites_api_url()),
            ("FINITE_HOME", self.finite_home_dir().display().to_string()),
        ];

        let mut out = String::new();
        for (key, value) in env {
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
        let mut command = self.process_compose_command();
        if matches!(mode, ProcessComposeMode::Headless) {
            command.arg("--tui=false");
        }
        if dry_run {
            command.arg("--dry-run");
        }
        command.arg("up");
        run_status(command)
    }

    pub fn cleanup(&self) -> Result<ExitCode> {
        if self.process_compose_socket.exists() && self.process_compose_file.exists() {
            if self.process_compose_available() {
                let mut command = self.process_compose_command();
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

        run_best_effort(
            Command::new("sh").arg("-c").arg(format!(
                "docker rm -f {} >/dev/null 2>&1 || true",
                self.postgres_container
            )),
            "remove devfinity Postgres container",
        );

        for path in [&self.process_compose_socket] {
            if path.exists() {
                if let Err(error) = fs::remove_file(path) {
                    eprintln!("failed to remove {}: {error}", path.display());
                }
            }
        }

        println!("devfinity cleanup complete");
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
        self.write_dashboard(&mut yaml);
        yaml
    }

    fn write_rust_build(&self, yaml: &mut String) {
        let _ = writeln!(yaml, "  rust-build:");
        self.write_process_header(
            yaml,
            "Build Rust service binaries",
            &self.repo_root,
            "rust-build",
        );
        let _ = writeln!(
            yaml,
            "    command: \"cargo build -p finite-saas-core -p finitechat-server -p finitesitesd\""
        );
        let _ = writeln!(yaml, "    availability:");
        let _ = writeln!(yaml, "      restart: exit_on_failure");
    }

    fn write_postgres(&self, yaml: &mut String) {
        let password = "finite-local";
        let _ = writeln!(yaml, "  postgres:");
        self.write_process_header(
            yaml,
            "Local Postgres for finite-saas-core",
            &self.repo_root,
            "postgres",
        );
        let _ = writeln!(yaml, "    command: |");
        let _ = writeln!(
            yaml,
            "      docker rm -f {} >/dev/null 2>&1 || true",
            self.postgres_container
        );
        let _ = writeln!(
            yaml,
            "      exec docker run --rm --name {} -e POSTGRES_PASSWORD={} -e POSTGRES_DB=finite_saas_core -p 127.0.0.1:{}:5432 postgres:16-alpine",
            self.postgres_container,
            shell_quote(password),
            self.ports.postgres
        );
        let _ = writeln!(yaml, "    readiness_probe:");
        let _ = writeln!(yaml, "      exec:");
        let _ = writeln!(
            yaml,
            "        command: \"docker exec {} pg_isready -U postgres -d finite_saas_core\"",
            self.postgres_container
        );
        self.write_probe_timing(yaml, 3, 2, 5, 30);
        let _ = writeln!(yaml, "    shutdown:");
        let _ = writeln!(
            yaml,
            "      command: \"docker stop {} >/dev/null 2>&1 || true\"",
            self.postgres_container
        );
        let _ = writeln!(yaml, "      timeout_seconds: 10");
    }

    fn write_core(&self, yaml: &mut String) {
        let _ = writeln!(yaml, "  core:");
        self.write_process_header(yaml, "Finite SaaS Core API", &self.repo_root, "core");
        let _ = writeln!(
            yaml,
            "    command: \"cargo run -p finite-saas-core -- serve\""
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      rust-build:");
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        let _ = writeln!(yaml, "      postgres:");
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
        let sqlite = self.finitechat_dir().join("server.sqlite3");
        let command = format!(
            "cargo run -p finitechat-server -- serve 127.0.0.1:{} --sqlite {}",
            self.ports.finitechat,
            shell_quote(&sqlite.display().to_string())
        );
        let _ = writeln!(yaml, "  finitechat:");
        self.write_process_header(
            yaml,
            "Local Finite Chat delivery server",
            &self.repo_root,
            "finitechat",
        );
        let _ = writeln!(yaml, "    command: {}", yaml_string(&command));
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      rust-build:");
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        self.write_http_probe(yaml, "/health", self.ports.finitechat, 1, 2, 3, 45);
    }

    fn write_finitesites(&self, yaml: &mut String) {
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
        let _ = writeln!(yaml, "  finitesites:");
        self.write_process_header(
            yaml,
            "Local Finite Sites server",
            &self.repo_root,
            "finitesites",
        );
        let _ = writeln!(yaml, "    command: {}", yaml_string(&command));
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      rust-build:");
        let _ = writeln!(yaml, "        condition: process_completed_successfully");
        self.write_http_probe(yaml, "/api/v1/healthz", self.ports.finitesites, 1, 2, 3, 45);
    }

    fn write_dashboard(&self, yaml: &mut String) {
        let dashboard_dir = self.repo_root.join("finitecomputer-v2/apps/dashboard");
        let _ = writeln!(yaml, "  dashboard:");
        self.write_process_header(
            yaml,
            "Finite dashboard dev server",
            &dashboard_dir,
            "dashboard",
        );
        let _ = writeln!(
            yaml,
            "    command: \"npm run dev -- --hostname 127.0.0.1 --port {}\"",
            self.ports.dashboard
        );
        let _ = writeln!(yaml, "    depends_on:");
        let _ = writeln!(yaml, "      core:");
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

    fn write_process_header(
        &self,
        yaml: &mut String,
        description: &str,
        working_dir: &Path,
        log_name: &str,
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
                    .join(format!("{log_name}.log"))
                    .display()
                    .to_string()
            )
        );
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

    fn process_compose_command(&self) -> Command {
        let mut command = Command::new("process-compose");
        command
            .arg("--config")
            .arg(&self.process_compose_file)
            .arg("--use-uds")
            .arg("--unix-socket")
            .arg(&self.process_compose_socket)
            .arg("--ordered-shutdown")
            .arg("--log-file")
            .arg(self.logs_dir.join("process-compose-supervisor.log"));
        command
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
        self.run_dir.join("postgres")
    }

    fn core_dir(&self) -> PathBuf {
        self.run_dir.join("core")
    }

    fn dashboard_dir(&self) -> PathBuf {
        self.run_dir.join("dashboard")
    }

    fn finitechat_dir(&self) -> PathBuf {
        self.run_dir.join("finitechat")
    }

    fn finitesites_dir(&self) -> PathBuf {
        self.run_dir.join("finitesites")
    }

    fn finite_home_dir(&self) -> PathBuf {
        self.run_dir.join("finite-home")
    }
}

fn absolute_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn run_status(mut command: Command) -> Result<ExitCode> {
    let status = command
        .status()
        .with_context(|| format!("failed to run {:?}", command))?;
    Ok(match status.code() {
        Some(code) if (0..=255).contains(&code) => ExitCode::from(code as u8),
        _ => ExitCode::FAILURE,
    })
}

fn run_best_effort(command: &mut Command, label: &str) {
    match command.status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("{label} exited with {status}"),
        Err(error) => eprintln!("failed to {label}: {error}"),
    }
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
    fn generated_yaml_contains_core_services() {
        let stack = Stack::new(PathBuf::from(".local-state/devfinity")).unwrap();
        let yaml = stack.process_compose_yaml();
        assert!(yaml.contains("rust-build:"));
        assert!(yaml.contains("postgres:"));
        assert!(yaml.contains("core:"));
        assert!(yaml.contains("finitechat:"));
        assert!(yaml.contains("finitesites:"));
        assert!(yaml.contains("dashboard:"));
        assert!(yaml.contains("process_completed_successfully"));
        assert!(yaml.contains("process_healthy"));
    }
}
