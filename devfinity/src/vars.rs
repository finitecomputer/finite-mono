use std::fmt::Write as _;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::topology::{DevfinityStack, shell_quote};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackPaths {
    pub repo_root: PathBuf,
    pub state_dir: PathBuf,
    pub run_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub control_dir: PathBuf,
    pub ready_file: PathBuf,
    pub error_file: PathBuf,
    pub postgres_dir: PathBuf,
    pub postgres_data_dir: PathBuf,
    pub postgres_script: PathBuf,
    pub core_dir: PathBuf,
    pub finitechat_dir: PathBuf,
    pub finitesites_dir: PathBuf,
    pub finite_home_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackPorts {
    pub core: u16,
    pub postgres: u16,
    pub finitechat: u16,
    pub finitesites: u16,
}

impl StackPorts {
    pub fn base() -> Self {
        Self {
            core: 14200,
            postgres: 15432,
            finitechat: 18787,
            finitesites: 18789,
        }
    }

    pub fn allocate() -> Result<Self> {
        let listeners = (0..4)
            .map(|_| TcpListener::bind(("127.0.0.1", 0)).context("failed to allocate local port"))
            .collect::<Result<Vec<_>>>()?;
        let ports = listeners
            .iter()
            .map(|listener| {
                listener
                    .local_addr()
                    .context("failed to read allocated local port")
                    .map(|addr| addr.port())
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            core: ports[0],
            postgres: ports[1],
            finitechat: ports[2],
            finitesites: ports[3],
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackEnv {
    pub devfinity_state_dir: String,
    pub devfinity_logs_dir: String,
    pub devfinity_ready_file: String,
    pub devfinity_error_file: String,
    pub devfinity_postgres_port: String,
    pub fc_core_url: String,
    pub fc_core_base_url: String,
    pub fc_core_api_token: String,
    pub fc_core_database_url: String,
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
            ("DEVFINITY_READY_FILE", self.devfinity_ready_file.clone()),
            ("DEVFINITY_ERROR_FILE", self.devfinity_error_file.clone()),
            (
                "DEVFINITY_POSTGRES_PORT",
                self.devfinity_postgres_port.clone(),
            ),
            ("FC_CORE_URL", self.fc_core_url.clone()),
            ("FC_CORE_BASE_URL", self.fc_core_base_url.clone()),
            ("FC_CORE_API_TOKEN", self.fc_core_api_token.clone()),
            ("FC_CORE_DATABASE_URL", self.fc_core_database_url.clone()),
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

    pub fn apply_to_current_process(&self) {
        for (key, value) in self.values() {
            // Devfinity fixtures are single-threaded during environment setup.
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevfinityVars {
    pub paths: StackPaths,
    pub ports: StackPorts,
    pub env: StackEnv,
}

impl DevfinityVars {
    pub fn from_stack(stack: &DevfinityStack) -> Self {
        let paths = stack.paths();
        let env = stack.env();
        let ports = stack.ports().clone();
        Self { paths, ports, env }
    }

    pub fn write_env_file(&self) -> Result<()> {
        fs::write(self.paths.run_dir.join("env"), self.env.exports()).with_context(|| {
            format!(
                "failed to write {}",
                self.paths.run_dir.join("env").display()
            )
        })
    }

    pub fn mark_ready(&self) -> Result<()> {
        remove_marker_best_effort(&self.paths.error_file);
        fs::write(&self.paths.ready_file, "READY\n")
            .with_context(|| format!("failed to write {}", self.paths.ready_file.display()))
    }

    pub fn mark_error(&self) -> Result<()> {
        remove_marker_best_effort(&self.paths.ready_file);
        fs::write(&self.paths.error_file, "ERROR\n")
            .with_context(|| format!("failed to write {}", self.paths.error_file.display()))
    }

    pub fn clear_markers(&self) {
        remove_marker_best_effort(&self.paths.ready_file);
        remove_marker_best_effort(&self.paths.error_file);
    }

    pub fn apply_env_to_current_process(&self) {
        self.env.apply_to_current_process();
    }
}

impl DevfinityStack {
    pub fn vars(&self) -> DevfinityVars {
        DevfinityVars::from_stack(self)
    }

    pub fn env(&self) -> StackEnv {
        let core_url = self.core_url();
        let finitechat_url = self.finitechat_url();
        let paths = self.paths();
        StackEnv {
            devfinity_state_dir: paths.run_dir.display().to_string(),
            devfinity_logs_dir: paths.logs_dir.display().to_string(),
            devfinity_ready_file: paths.ready_file.display().to_string(),
            devfinity_error_file: paths.error_file.display().to_string(),
            devfinity_postgres_port: self.ports.postgres.to_string(),
            fc_core_url: core_url.clone(),
            fc_core_base_url: core_url,
            fc_core_api_token: self.core_token.clone(),
            fc_core_database_url: self.database_url(),
            finitechat_server_url: finitechat_url.clone(),
            fc_runner_finitechat_server_url: finitechat_url,
            finite_sites_api: self.finitesites_api_url(),
            finite_home: self.finite_home_dir().display().to_string(),
        }
    }

    pub fn write_env_file(&self) -> Result<()> {
        self.vars().write_env_file()
    }

    pub fn env_exports(&self) -> String {
        self.env().exports()
    }

    pub fn env_values(&self) -> Vec<(&'static str, String)> {
        self.env().values()
    }

    pub fn apply_env_to_current_process(&self) {
        self.vars().apply_env_to_current_process();
    }

    pub(crate) fn clear_run_markers(&self) {
        self.vars().clear_markers();
    }

    pub(crate) fn mark_ready(&self) -> Result<()> {
        self.vars().mark_ready()
    }

    pub(crate) fn mark_error(&self) -> Result<()> {
        self.vars().mark_error()
    }
}

pub(crate) fn unique_run_name(prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{}-{millis}", std::process::id())
}

fn remove_marker_best_effort(path: &PathBuf) {
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocated_ports_are_distinct() {
        let ports = StackPorts::allocate().expect("ports");
        let mut values = vec![
            ports.core,
            ports.postgres,
            ports.finitechat,
            ports.finitesites,
        ];
        values.sort_unstable();
        values.dedup();
        assert_eq!(values.len(), 4);
    }
}
