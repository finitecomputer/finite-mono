//! Core, as the mover changes it: only through Core's own operator CLI,
//! one `*-exact` subcommand at a time, each bound to the runtime, host and
//! machine the mover read (finite-mono #1146; Core keeps no listener for
//! the mover). On lat-2 it runs through sudo for these subcommands alone.

use std::process::{Command, Stdio};

use crate::contract::{Mark, short};
use crate::fleet::Binding;

#[derive(Debug, Clone)]
pub struct CoreCli {
    /// The command that runs Core's CLI, such as `["sudo", "-n",
    /// "/run/wrappers/bin/finite-mover-core"]`: the subcommand follows.
    pub program: Vec<String>,
    /// The operator Core records as acting (an audit fact, not a secret).
    pub admin_email: String,
    pub admin_workos_user_id: String,
    /// How long a stop, retirement or restart is waited for.
    pub wait_seconds: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("Core's {subcommand} could not be run: {why}")]
    NotRun {
        subcommand: &'static str,
        why: String,
    },
    #[error("Core's {subcommand} failed ({status}): {why}")]
    Failed {
        subcommand: &'static str,
        status: String,
        why: String,
    },
    #[error("Core's {subcommand} answered nothing the mover can read")]
    Unreadable { subcommand: &'static str },
}

impl CoreCli {
    fn bound(&self, binding: &Binding) -> Vec<String> {
        [
            ("--project-id", binding.old.as_str()),
            ("--expected-agent-runtime-id", &binding.agent_runtime_id),
            ("--expected-source-host-id", &binding.source_host_id),
            ("--expected-source-machine-id", &binding.source_machine_id),
            ("--admin-email", &self.admin_email),
            ("--admin-workos-user-id", &self.admin_workos_user_id),
        ]
        .into_iter()
        .flat_map(|(flag, value)| [flag.to_string(), value.to_string()])
        .collect()
    }

    fn waited(&self, binding: &Binding) -> Vec<String> {
        let mut args = self.bound(binding);
        args.extend([
            "--wait-timeout-seconds".to_string(),
            self.wait_seconds.to_string(),
        ]);
        args
    }

    /// Run `subcommand`, answering its standard output.
    fn run(&self, subcommand: &'static str, args: Vec<String>) -> Result<String, CoreError> {
        let (program, prefix) = self.program.split_first().ok_or(CoreError::NotRun {
            subcommand,
            why: "no Core command is configured".into(),
        })?;
        let output = Command::new(program)
            .args(prefix)
            .arg(subcommand)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| CoreError::NotRun {
                subcommand,
                why: error.to_string(),
            })?;
        if !output.status.success() {
            // Core's errors name ids and states, never a credential.
            let stderr = String::from_utf8_lossy(&output.stderr);
            let last = stderr
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            return Err(CoreError::Failed {
                subcommand,
                status: output.status.to_string(),
                why: short(last.trim()),
            });
        }
        String::from_utf8(output.stdout).map_err(|_| CoreError::Unreadable { subcommand })
    }

    /// Stop the runtime and wait until Core says it stopped. A stopped
    /// runtime stops again as nothing.
    pub fn stop(&self, binding: &Binding) -> Result<(), CoreError> {
        self.run("runtime-stop-exact", self.waited(binding))
            .map(drop)
    }

    /// Retire it through Core's verified Recovery Snapshot, answering the
    /// receipt: the retirement request's id, which keys its snapshot.
    pub fn retire(&self, binding: &Binding) -> Result<String, CoreError> {
        let subcommand = "runtime-retire-exact";
        let out = self.run(subcommand, self.waited(binding))?;
        last_json_id(&out).ok_or(CoreError::Unreadable { subcommand })
    }

    /// Mark it moved (`migrated` or `archived`), so the roster never offers
    /// it again and finite.computer sends its person on.
    pub fn mark_moved(&self, binding: &Binding, mark: Mark) -> Result<(), CoreError> {
        let mut args = self.bound(binding);
        args.extend(["--moved".to_string(), mark.as_str().to_string()]);
        self.run("runtime-mark-moved-exact", args).map(drop)
    }

    /// Start it again (staff gave it back) and wait until it is up.
    pub fn restart(&self, binding: &Binding) -> Result<(), CoreError> {
        self.run("runtime-restart-exact", self.waited(binding))
            .map(drop)
    }
}

/// The `id` of the last JSON object on `out`'s lines (Core prints the
/// control request it waited on).
fn last_json_id(out: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(out.trim()).ok().or_else(|| {
        out.lines()
            .rev()
            .find_map(|line| serde_json::from_str(line.trim()).ok())
    })?;
    value
        .get("id")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(short)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_receipt_is_the_request_core_printed() {
        assert_eq!(
            last_json_id("{\n  \"id\": \"rcr_1\",\n  \"status\": \"stopped\"\n}\n").as_deref(),
            Some("rcr_1")
        );
        assert_eq!(
            last_json_id("noise\n{\"id\":\"rcr_2\"}\n").as_deref(),
            Some("rcr_2")
        );
        assert_eq!(last_json_id("{\"status\":\"stopped\"}"), None);
        assert_eq!(last_json_id(""), None);
    }
}
