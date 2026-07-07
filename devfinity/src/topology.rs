use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct DevfinityStack {
    pub(crate) repo_root: PathBuf,
    pub(crate) state_dir: PathBuf,
    pub(crate) run_dir: PathBuf,
    pub(crate) logs_dir: PathBuf,
    pub(crate) control_dir: PathBuf,
    pub(crate) profile: StackProfile,
    pub(crate) ports: StackPorts,
    pub(crate) core_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackProfile {
    /// Base local profile: Core plus Chat plus Sites with native Postgres.
    Base,
}

impl StackProfile {
    pub fn base() -> Self {
        Self::Base
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Base => "Core plus Chat plus Sites",
        }
    }

    pub(crate) fn run_name(self) -> &'static str {
        match self {
            Self::Base => "default",
        }
    }
}

impl Default for StackProfile {
    fn default() -> Self {
        Self::Base
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackPaths {
    pub repo_root: PathBuf,
    pub state_dir: PathBuf,
    pub run_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub control_dir: PathBuf,
    pub postgres_dir: PathBuf,
    pub postgres_data_dir: PathBuf,
    pub postgres_script: PathBuf,
    pub core_dir: PathBuf,
    pub dashboard_dir: PathBuf,
    pub finitechat_dir: PathBuf,
    pub finitesites_dir: PathBuf,
    pub finite_home_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackPorts {
    pub core: u16,
    pub dashboard: u16,
    pub postgres: u16,
    pub finitechat: u16,
    pub finitesites: u16,
}

impl StackPorts {
    pub fn base() -> Self {
        Self {
            core: 14200,
            dashboard: 13002,
            postgres: 15432,
            finitechat: 18787,
            finitesites: 18789,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackEnv {
    pub devfinity_state_dir: String,
    pub devfinity_logs_dir: String,
    pub devfinity_postgres_port: String,
    pub fc_workos_auth_enabled: String,
    pub fc_dashboard_allow_dev_account_auth: String,
    pub fc_dashboard_dev_email: String,
    pub fc_dashboard_dev_workos_user_id: String,
    pub fc_core_url: String,
    pub fc_core_base_url: String,
    pub fc_core_api_token: String,
    pub fc_core_database_url: String,
    pub fc_dashboard_url: String,
    pub finitechat_server_url: String,
    pub fc_runner_finitechat_server_url: String,
    pub finite_sites_api: String,
    pub finite_home: String,
}

impl StackEnv {
    pub fn values(&self) -> Vec<(&'static str, String)> {
        vec![
            ("DEVFINITY_STATE_DIR", self.devfinity_state_dir.clone()),
            ("DEVFINITY_LOGS_DIR", self.devfinity_logs_dir.clone()),
            (
                "DEVFINITY_POSTGRES_PORT",
                self.devfinity_postgres_port.clone(),
            ),
            (
                "FC_WORKOS_AUTH_ENABLED",
                self.fc_workos_auth_enabled.clone(),
            ),
            (
                "FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH",
                self.fc_dashboard_allow_dev_account_auth.clone(),
            ),
            (
                "FC_DASHBOARD_DEV_EMAIL",
                self.fc_dashboard_dev_email.clone(),
            ),
            (
                "FC_DASHBOARD_DEV_WORKOS_USER_ID",
                self.fc_dashboard_dev_workos_user_id.clone(),
            ),
            ("FC_CORE_URL", self.fc_core_url.clone()),
            ("FC_CORE_BASE_URL", self.fc_core_base_url.clone()),
            ("FC_CORE_API_TOKEN", self.fc_core_api_token.clone()),
            ("FC_CORE_DATABASE_URL", self.fc_core_database_url.clone()),
            ("FC_DASHBOARD_URL", self.fc_dashboard_url.clone()),
            ("FINITECHAT_SERVER_URL", self.finitechat_server_url.clone()),
            (
                "FC_RUNNER_FINITECHAT_SERVER_URL",
                self.fc_runner_finitechat_server_url.clone(),
            ),
            ("FINITE_SITES_API", self.finite_sites_api.clone()),
            ("FINITE_HOME", self.finite_home.clone()),
        ]
    }

    pub fn exports(&self) -> String {
        let mut out = String::new();
        for (key, value) in self.values() {
            let _ = writeln!(out, "export {key}={}", shell_quote(&value));
        }
        out
    }
}

impl DevfinityStack {
    pub fn new(state_dir: PathBuf) -> Result<Self> {
        let repo_root = std::env::current_dir().context("failed to read current directory")?;
        Self::new_with_repo_root(repo_root, state_dir)
    }

    pub fn new_with_repo_root(repo_root: PathBuf, state_dir: PathBuf) -> Result<Self> {
        Self::with_profile_and_repo_root(StackProfile::Base, repo_root, state_dir)
    }

    pub fn with_profile(profile: StackProfile, state_dir: PathBuf) -> Result<Self> {
        let repo_root = std::env::current_dir().context("failed to read current directory")?;
        Self::with_profile_and_repo_root(profile, repo_root, state_dir)
    }

    pub fn with_profile_and_repo_root(
        profile: StackProfile,
        repo_root: PathBuf,
        state_dir: PathBuf,
    ) -> Result<Self> {
        let state_dir = absolute_path(&repo_root, &state_dir);
        let run_dir = state_dir.join("runs").join(profile.run_name());
        let logs_dir = run_dir.join("logs");
        let control_dir = run_dir.join("control");
        Ok(Self {
            repo_root,
            state_dir,
            run_dir,
            logs_dir,
            control_dir,
            profile,
            ports: StackPorts::base(),
            core_token: "devfinity-core-token".to_string(),
        })
    }

    pub fn profile(&self) -> StackProfile {
        self.profile
    }

    pub fn paths(&self) -> StackPaths {
        StackPaths {
            repo_root: self.repo_root.clone(),
            state_dir: self.state_dir.clone(),
            run_dir: self.run_dir.clone(),
            logs_dir: self.logs_dir.clone(),
            control_dir: self.control_dir.clone(),
            postgres_dir: self.postgres_dir(),
            postgres_data_dir: self.postgres_data_dir(),
            postgres_script: self.postgres_script_path(),
            core_dir: self.core_dir(),
            dashboard_dir: self.dashboard_dir(),
            finitechat_dir: self.finitechat_dir(),
            finitesites_dir: self.finitesites_dir(),
            finite_home_dir: self.finite_home_dir(),
        }
    }

    pub fn ports(&self) -> &StackPorts {
        &self.ports
    }

    pub fn ports_mut(&mut self) -> &mut StackPorts {
        &mut self.ports
    }

    pub fn env(&self) -> StackEnv {
        let core_url = self.core_url();
        let finitechat_url = self.finitechat_url();
        StackEnv {
            devfinity_state_dir: self.run_dir.display().to_string(),
            devfinity_logs_dir: self.logs_dir.display().to_string(),
            devfinity_postgres_port: self.ports.postgres.to_string(),
            fc_workos_auth_enabled: "0".to_string(),
            fc_dashboard_allow_dev_account_auth: "1".to_string(),
            fc_dashboard_dev_email: "devfinity@finite.computer".to_string(),
            fc_dashboard_dev_workos_user_id: "user_devfinity".to_string(),
            fc_core_url: core_url.clone(),
            fc_core_base_url: core_url,
            fc_core_api_token: self.core_token.clone(),
            fc_core_database_url: self.database_url(),
            fc_dashboard_url: self.dashboard_url(),
            finitechat_server_url: finitechat_url.clone(),
            fc_runner_finitechat_server_url: finitechat_url,
            finite_sites_api: self.finitesites_api_url(),
            finite_home: self.finite_home_dir().display().to_string(),
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        let paths = self.paths();
        for dir in [
            &paths.state_dir,
            &paths.run_dir,
            &paths.logs_dir,
            &paths.control_dir,
            &paths.postgres_dir,
            &paths.core_dir,
            &paths.dashboard_dir,
            &paths.finitechat_dir,
            &paths.finitesites_dir,
            &paths.finite_home_dir,
        ] {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
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
        println!("Stop the stack by pressing Ctrl-C.");
        println!("Run `devfinity cleanup` if a previous stack left orphaned processes behind.");
    }

    pub fn env_exports(&self) -> String {
        self.env().exports()
    }

    pub fn env_values(&self) -> Vec<(&'static str, String)> {
        self.env().values()
    }

    pub fn urls_text(&self) -> String {
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

    pub fn core_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.core)
    }

    pub fn dashboard_url(&self) -> String {
        format!("http://127.0.0.1:{}/dashboard", self.ports.dashboard)
    }

    pub fn finitechat_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.finitechat)
    }

    pub fn finitesites_api_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.ports.finitesites)
    }

    pub fn database_url(&self) -> String {
        format!(
            "postgres://postgres:finite-local@127.0.0.1:{}/finite_saas_core",
            self.ports.postgres
        )
    }

    pub(crate) fn postgres_dir(&self) -> PathBuf {
        self.process_state_dir("postgres")
    }

    pub(crate) fn postgres_data_dir(&self) -> PathBuf {
        self.postgres_dir().join("data")
    }

    pub(crate) fn postgres_script_path(&self) -> PathBuf {
        self.run_dir.join("run-postgres.sh")
    }

    pub(crate) fn core_dir(&self) -> PathBuf {
        self.process_state_dir("core")
    }

    pub(crate) fn dashboard_dir(&self) -> PathBuf {
        self.process_state_dir("dashboard")
    }

    pub(crate) fn finitechat_dir(&self) -> PathBuf {
        self.process_state_dir("finitechat")
    }

    pub(crate) fn finitesites_dir(&self) -> PathBuf {
        self.process_state_dir("finitesites")
    }

    pub(crate) fn finite_home_dir(&self) -> PathBuf {
        self.run_dir.join("finite-home")
    }

    pub(crate) fn control_dir(&self) -> PathBuf {
        self.control_dir.clone()
    }

    pub(crate) fn process_state_dir(&self, process: &str) -> PathBuf {
        self.run_dir.join(process)
    }
}

pub(crate) fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn absolute_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_exports_are_shell_quoted() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn base_profile_builds_stable_typed_topology() {
        let repo = PathBuf::from("/tmp/finite-mono");
        let stack = DevfinityStack::new_with_repo_root(repo.clone(), PathBuf::from(".state"))
            .expect("stack");
        let paths = stack.paths();

        assert_eq!(stack.profile(), StackProfile::Base);
        assert_eq!(stack.ports().core, 14200);
        assert_eq!(paths.repo_root, repo);
        assert_eq!(
            paths.run_dir,
            PathBuf::from("/tmp/finite-mono/.state/runs/default")
        );
        assert_eq!(
            paths.control_dir,
            PathBuf::from("/tmp/finite-mono/.state/runs/default/control")
        );
        assert_eq!(
            stack.env().fc_core_database_url,
            "postgres://postgres:finite-local@127.0.0.1:15432/finite_saas_core"
        );
    }
}
