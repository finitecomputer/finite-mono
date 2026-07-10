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
        self.hermes_tx
            .send(ProcessAction::Restart)
            .await
            .map_err(|_| AgentdError::Supervisor("Hermes supervisor stopped".to_owned()))
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
