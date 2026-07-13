use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tokio::sync::{RwLock, mpsc};

use crate::AgentdError;

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub name: &'static str,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessStatus {
    pub state: String,
    pub pid: Option<u32>,
    pub restart_count: u64,
    pub last_exit: Option<String>,
    pub updated_at_ms: u64,
}

impl Default for ProcessStatus {
    fn default() -> Self {
        Self {
            state: "starting".to_owned(),
            pid: None,
            restart_count: 0,
            last_exit: None,
            updated_at_ms: now_ms(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorStatus {
    pub processes: BTreeMap<String, ProcessStatus>,
}

#[derive(Debug)]
enum ProcessAction {
    Restart,
    Stop,
}

#[derive(Clone)]
pub struct SupervisorHandle {
    hermes_tx: mpsc::Sender<ProcessAction>,
    all_txs: Arc<Vec<mpsc::Sender<ProcessAction>>>,
    status: Arc<RwLock<SupervisorStatus>>,
}

impl SupervisorHandle {
    pub async fn restart_hermes(&self) -> Result<(), AgentdError> {
        let previous_restart_count = self
            .status
            .read()
            .await
            .processes
            .get("hermes")
            .map(|status| status.restart_count)
            .unwrap_or(0);
        self.hermes_tx
            .send(ProcessAction::Restart)
            .await
            .map_err(|_| AgentdError::Supervisor("Hermes supervisor stopped".to_owned()))?;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let restarted = self
                    .status
                    .read()
                    .await
                    .processes
                    .get("hermes")
                    .is_some_and(|status| {
                        status.state == "running"
                            && status.restart_count > previous_restart_count
                            && status.pid.is_some()
                    });
                if restarted {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| {
            AgentdError::Supervisor(
                "Hermes did not return to running state after restart".to_owned(),
            )
        })
    }

    pub async fn status(&self) -> SupervisorStatus {
        self.status.read().await.clone()
    }

    pub async fn shutdown(&self) {
        for tx in self.all_txs.iter() {
            let _ = tx.send(ProcessAction::Stop).await;
        }
    }
}

pub fn start_supervisor(
    sidecar: ProcessSpec,
    health: ProcessSpec,
    hermes: ProcessSpec,
) -> SupervisorHandle {
    let status = Arc::new(RwLock::new(SupervisorStatus::default()));
    let (sidecar_tx, sidecar_rx) = mpsc::channel(4);
    let (health_tx, health_rx) = mpsc::channel(4);
    let (hermes_tx, hermes_rx) = mpsc::channel(4);

    tokio::spawn(supervise_process(sidecar, sidecar_rx, Arc::clone(&status)));
    tokio::spawn(supervise_process(health, health_rx, Arc::clone(&status)));
    tokio::spawn(supervise_process(hermes, hermes_rx, Arc::clone(&status)));

    SupervisorHandle {
        hermes_tx: hermes_tx.clone(),
        all_txs: Arc::new(vec![sidecar_tx, health_tx, hermes_tx]),
        status,
    }
}

async fn supervise_process(
    spec: ProcessSpec,
    mut actions: mpsc::Receiver<ProcessAction>,
    statuses: Arc<RwLock<SupervisorStatus>>,
) {
    let mut restart_count = 0u64;
    let mut retry_delay = Duration::from_millis(250);
    loop {
        set_status(
            &statuses,
            spec.name,
            ProcessStatus {
                state: if restart_count == 0 {
                    "starting".to_owned()
                } else {
                    "restarting".to_owned()
                },
                pid: None,
                restart_count,
                last_exit: None,
                updated_at_ms: now_ms(),
            },
        )
        .await;

        let mut child = match spawn_process(&spec) {
            Ok(child) => child,
            Err(error) => {
                set_status(
                    &statuses,
                    spec.name,
                    ProcessStatus {
                        state: "unavailable".to_owned(),
                        pid: None,
                        restart_count,
                        last_exit: Some(error.to_string()),
                        updated_at_ms: now_ms(),
                    },
                )
                .await;
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(5));
                restart_count = restart_count.saturating_add(1);
                continue;
            }
        };
        let pid = child.id();
        set_status(
            &statuses,
            spec.name,
            ProcessStatus {
                state: "running".to_owned(),
                pid,
                restart_count,
                last_exit: None,
                updated_at_ms: now_ms(),
            },
        )
        .await;
        retry_delay = Duration::from_millis(250);

        tokio::select! {
            result = child.wait() => {
                let exit = result
                    .map(|status| status.to_string())
                    .unwrap_or_else(|error| error.to_string());
                set_status(
                    &statuses,
                    spec.name,
                    ProcessStatus {
                        state: "exited".to_owned(),
                        pid: None,
                        restart_count,
                        last_exit: Some(exit),
                        updated_at_ms: now_ms(),
                    },
                ).await;
            }
            action = actions.recv() => {
                match action {
                    Some(ProcessAction::Restart) => {
                        terminate_child(&mut child).await;
                    }
                    Some(ProcessAction::Stop) | None => {
                        terminate_child(&mut child).await;
                        set_status(
                            &statuses,
                            spec.name,
                            ProcessStatus {
                                state: "stopped".to_owned(),
                                pid: None,
                                restart_count,
                                last_exit: None,
                                updated_at_ms: now_ms(),
                            },
                        ).await;
                        return;
                    }
                }
            }
        }
        restart_count = restart_count.saturating_add(1);
        tokio::time::sleep(retry_delay).await;
    }
}

fn spawn_process(spec: &ProcessSpec) -> Result<Child, AgentdError> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(&spec.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    command.spawn().map_err(AgentdError::from)
}

async fn terminate_child(child: &mut Child) {
    if let Some(pid) = child.id() {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .await;
    }
    if tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

async fn set_status(statuses: &Arc<RwLock<SupervisorStatus>>, name: &str, status: ProcessStatus) {
    statuses
        .write()
        .await
        .processes
        .insert(name.to_owned(), status);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn restart_hermes_waits_for_a_new_running_process() {
        let handle = start_supervisor(
            sleeping_process("sidecar"),
            sleeping_process("health"),
            sleeping_process("hermes"),
        );
        let original_pid = wait_for_running(&handle, "hermes").await.pid.unwrap();

        handle.restart_hermes().await.unwrap();

        let restarted = handle.status().await.processes["hermes"].clone();
        assert_eq!(restarted.state, "running");
        assert_eq!(restarted.restart_count, 1);
        assert_ne!(restarted.pid, Some(original_pid));
        handle.shutdown().await;
    }

    fn sleeping_process(name: &'static str) -> ProcessSpec {
        ProcessSpec {
            name,
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), "exec sleep 30".to_owned()],
            environment: BTreeMap::new(),
        }
    }

    async fn wait_for_running(handle: &SupervisorHandle, name: &str) -> ProcessStatus {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(status) = handle.status().await.processes.get(name)
                    && status.state == "running"
                {
                    return status.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
}
