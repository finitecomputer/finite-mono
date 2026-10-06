use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tokio::sync::{RwLock, mpsc, oneshot};

use crate::AgentdError;

#[derive(Clone)]
pub struct ProcessSpec {
    pub name: &'static str,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
}

impl std::fmt::Debug for ProcessSpec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessSpec")
            .field("name", &self.name)
            .field("program", &self.program)
            .field("args", &self.args)
            .field(
                "environment_keys",
                &self.environment.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessState {
    Starting,
    Restarting,
    Running { pid: u32 },
    Unavailable { error: String },
    Exited { exit: String },
    Stopped,
}

impl ProcessState {
    fn tag(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Restarting => "restarting",
            Self::Running { .. } => "running",
            Self::Unavailable { .. } => "unavailable",
            Self::Exited { .. } => "exited",
            Self::Stopped => "stopped",
        }
    }

    fn pid(&self) -> Option<u32> {
        match self {
            Self::Running { pid } => Some(*pid),
            _ => None,
        }
    }

    fn last_exit(&self) -> Option<&str> {
        match self {
            Self::Unavailable { error } => Some(error),
            Self::Exited { exit } => Some(exit),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessStatus {
    pub state: ProcessState,
    pub restart_count: u64,
    pub updated_at_ms: u64,
}

impl ProcessStatus {
    pub fn pid(&self) -> Option<u32> {
        self.state.pid()
    }
}

// The serde impls are manual so the published wire shape stays byte-identical
// to the historical flat struct: every state keeps its `pid` and `last_exit`
// keys, serialized as null where the state carries no value. Deserialization
// fails closed on unknown states and on pid/last_exit combinations the
// supervisor never writes.
impl Serialize for ProcessStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut status = serializer.serialize_struct("ProcessStatus", 5)?;
        status.serialize_field("state", self.state.tag())?;
        status.serialize_field("pid", &self.state.pid())?;
        status.serialize_field("restart_count", &self.restart_count)?;
        status.serialize_field("last_exit", &self.state.last_exit())?;
        status.serialize_field("updated_at_ms", &self.updated_at_ms)?;
        status.end()
    }
}

impl<'de> Deserialize<'de> for ProcessStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct WireProcessStatus {
            state: String,
            pid: Option<u32>,
            restart_count: u64,
            last_exit: Option<String>,
            updated_at_ms: u64,
        }
        let wire = WireProcessStatus::deserialize(deserializer)?;
        let state = match (wire.state.as_str(), wire.pid, wire.last_exit) {
            ("starting", None, None) => ProcessState::Starting,
            ("restarting", None, None) => ProcessState::Restarting,
            ("running", Some(pid), None) => ProcessState::Running { pid },
            ("unavailable", None, Some(error)) => ProcessState::Unavailable { error },
            ("exited", None, Some(exit)) => ProcessState::Exited { exit },
            ("stopped", None, None) => ProcessState::Stopped,
            _ => {
                return Err(serde::de::Error::custom(
                    "process status state is unknown or inconsistent with pid/last_exit",
                ));
            }
        };
        Ok(Self {
            state,
            restart_count: wire.restart_count,
            updated_at_ms: wire.updated_at_ms,
        })
    }
}

impl Default for ProcessStatus {
    fn default() -> Self {
        Self {
            state: ProcessState::Starting,
            restart_count: 0,
            updated_at_ms: now_ms(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorStatus {
    pub processes: BTreeMap<String, ProcessStatus>,
}

/// Grace a restarted child gets between SIGTERM and SIGKILL.
pub(crate) const TERMINATE_GRACE: Duration = Duration::from_secs(10);
/// Shutdown graces. The whole stop must fit inside the runtime's stop window
/// (`docker stop --time 15` in the interruption smoke; Kata allows far more)
/// even when every child ignores SIGTERM: gateways get 8s to release their
/// leases, then the sidecar and health server 2s, each phase plus reap slack.
const GATEWAY_STOP_GRACE: Duration = Duration::from_secs(8);
const SERVICE_STOP_GRACE: Duration = Duration::from_secs(2);
const STOP_REAP_SLACK: Duration = Duration::from_secs(1);

#[derive(Debug)]
enum ProcessAction {
    Restart,
    /// Stop the slot for good after at most `grace` of SIGTERM drain; `done`
    /// fires once its child is reaped.
    Stop {
        grace: Duration,
        done: oneshot::Sender<()>,
    },
}

#[derive(Clone)]
pub struct SupervisorHandle {
    hermes_tx: mpsc::Sender<ProcessAction>,
    /// Shutdown phases, stopped in order. The gateways come first while the
    /// Finite Chat sidecar still serves: on SIGTERM the Hermes adapter
    /// releases its in-flight and queued inbox leases through the sidecar,
    /// and a lease it cannot release stays stranded until lease expiry.
    stop_phases: Arc<Vec<(Duration, Vec<mpsc::Sender<ProcessAction>>)>>,
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
                let state = self.status.read().await.processes.get("hermes").cloned();
                match state {
                    Some(status)
                        if matches!(status.state, ProcessState::Running { .. })
                            && status.restart_count > previous_restart_count =>
                    {
                        return Ok(());
                    }
                    Some(status) if status.state == ProcessState::Stopped => {
                        return Err(AgentdError::Supervisor(
                            "Hermes supervisor stopped during restart".to_owned(),
                        ));
                    }
                    _ => {}
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| {
            AgentdError::Supervisor(
                "Hermes did not return to running state after restart".to_owned(),
            )
        })?
    }

    pub async fn status(&self) -> SupervisorStatus {
        self.status.read().await.clone()
    }

    /// Stop every slot phase by phase and return once their children are
    /// reaped. Callers exit right after this, and dropping the runtime would
    /// SIGKILL any child still draining (`kill_on_drop`), so the wait is the
    /// graceful stop. Each phase is bounded by its grace plus reap slack.
    pub async fn shutdown(&self) {
        for (grace, phase) in self.stop_phases.iter() {
            let grace = *grace;
            let stops = phase.iter().map(|tx| async move {
                let (done, done_rx) = oneshot::channel();
                if tx.send(ProcessAction::Stop { grace, done }).await.is_ok() {
                    // A closed channel or dropped ack means the slot already
                    // returned; either way nothing is left to wait for.
                    let _ = done_rx.await;
                }
            });
            let bound = grace + STOP_REAP_SLACK;
            let _ = tokio::time::timeout(bound, futures_util::future::join_all(stops)).await;
        }
    }
}

pub fn start_supervisor(
    sidecar: ProcessSpec,
    health: ProcessSpec,
    hermes: ProcessSpec,
    simplex: Option<ProcessSpec>,
) -> SupervisorHandle {
    let status = Arc::new(RwLock::new(SupervisorStatus::default()));
    let (sidecar_tx, sidecar_rx) = mpsc::channel(4);
    let (health_tx, health_rx) = mpsc::channel(4);
    let (hermes_tx, hermes_rx) = mpsc::channel(4);

    tokio::spawn(supervise_process(sidecar, sidecar_rx, Arc::clone(&status)));
    tokio::spawn(supervise_process(health, health_rx, Arc::clone(&status)));
    tokio::spawn(supervise_process(hermes, hermes_rx, Arc::clone(&status)));

    let mut gateways = vec![hermes_tx.clone()];
    if let Some(spec) = simplex {
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(supervise_process(spec, rx, Arc::clone(&status)));
        gateways.push(tx);
    }
    SupervisorHandle {
        hermes_tx,
        stop_phases: Arc::new(vec![
            (GATEWAY_STOP_GRACE, gateways),
            (SERVICE_STOP_GRACE, vec![health_tx, sidecar_tx]),
        ]),
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
                    ProcessState::Starting
                } else {
                    ProcessState::Restarting
                },
                restart_count,
                updated_at_ms: now_ms(),
            },
        )
        .await;

        let (mut child, pid) = match spawn_process(&spec) {
            Ok(spawned) => spawned,
            Err(error) => {
                set_status(
                    &statuses,
                    spec.name,
                    ProcessStatus {
                        state: ProcessState::Unavailable {
                            error: error.to_string(),
                        },
                        restart_count,
                        updated_at_ms: now_ms(),
                    },
                )
                .await;
                if let Some(done) = backoff(retry_delay, &mut actions).await {
                    stop_slot(&statuses, spec.name, restart_count, done).await;
                    return;
                }
                retry_delay = (retry_delay * 2).min(Duration::from_secs(5));
                restart_count = restart_count.saturating_add(1);
                continue;
            }
        };
        set_status(
            &statuses,
            spec.name,
            ProcessStatus {
                state: ProcessState::Running { pid },
                restart_count,
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
                        state: ProcessState::Exited { exit },
                        restart_count,
                        updated_at_ms: now_ms(),
                    },
                ).await;
            }
            action = actions.recv() => {
                match action {
                    Some(ProcessAction::Restart) => {
                        terminate_child(&mut child, TERMINATE_GRACE).await;
                    }
                    Some(ProcessAction::Stop { grace, done }) => {
                        terminate_child(&mut child, grace).await;
                        stop_slot(&statuses, spec.name, restart_count, Some(done)).await;
                        return;
                    }
                    None => {
                        terminate_child(&mut child, TERMINATE_GRACE).await;
                        stop_slot(&statuses, spec.name, restart_count, None).await;
                        return;
                    }
                }
            }
        }
        restart_count = restart_count.saturating_add(1);
        if let Some(done) = backoff(retry_delay, &mut actions).await {
            stop_slot(&statuses, spec.name, restart_count, done).await;
            return;
        }
    }
}

/// Sleep out a respawn backoff unless the slot is stopped first. Returns
/// `Some(ack)` on stop (`Some(None)` when every handle is gone) so a stop
/// arriving between children never spawns another one. A restart request
/// here is already being honored by the pending respawn.
async fn backoff(
    delay: Duration,
    actions: &mut mpsc::Receiver<ProcessAction>,
) -> Option<Option<oneshot::Sender<()>>> {
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            () = &mut sleep => return None,
            action = actions.recv() => match action {
                Some(ProcessAction::Restart) => {}
                Some(ProcessAction::Stop { done, .. }) => return Some(Some(done)),
                None => return Some(None),
            },
        }
    }
}

async fn stop_slot(
    statuses: &Arc<RwLock<SupervisorStatus>>,
    name: &str,
    restart_count: u64,
    done: Option<oneshot::Sender<()>>,
) {
    set_status(
        statuses,
        name,
        ProcessStatus {
            state: ProcessState::Stopped,
            restart_count,
            updated_at_ms: now_ms(),
        },
    )
    .await;
    if let Some(done) = done {
        let _ = done.send(());
    }
}

pub(crate) fn spawn_process(spec: &ProcessSpec) -> Result<(Child, u32), AgentdError> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(&spec.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        // Each child leads its own process group so termination can signal
        // the whole tree: gateway scripts fork background loops, and a
        // straggler grandchild would otherwise survive a restart holding a
        // loopback port (PR 440 observed an orphaned finitechat bridge
        // keeping :37633, so the next bridge exited at bind).
        .process_group(0)
        .kill_on_drop(true);
    let child = command.spawn().map_err(AgentdError::from)?;
    let pid = child
        .id()
        .ok_or_else(|| AgentdError::Supervisor("spawned child has no pid".to_owned()))?;
    Ok((child, pid))
}

pub(crate) fn signal_group(pid: u32, signal: rustix::process::Signal) {
    // Signal the child's whole process group (it is spawned as group leader).
    // In-process syscall: the runtime image ships no `kill` binary (only the
    // sh builtin), so the old `Command::new("kill")` shell-out silently
    // signalled nothing — the child never saw SIGTERM and every "graceful"
    // drain was really a 10s stall followed by SIGKILL (PR 440, 83ef3024).
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process_group(pid, signal);
    }
}

pub(crate) async fn terminate_child(child: &mut Child, grace: Duration) {
    let pid = child.id();
    if let Some(pid) = pid {
        signal_group(pid, rustix::process::Signal::TERM);
    }
    if tokio::time::timeout(grace, child.wait()).await.is_err() {
        if let Some(pid) = pid {
            signal_group(pid, rustix::process::Signal::KILL);
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    // The direct child is gone; sweep the group once more so no orphaned
    // grandchild survives into this slot's next incarnation (the post-exit
    // sweep from PR 440's ec46243e, at agentd's own supervision boundary).
    if let Some(pid) = pid {
        signal_group(pid, rustix::process::Signal::KILL);
    }
}

async fn set_status(statuses: &Arc<RwLock<SupervisorStatus>>, name: &str, status: ProcessStatus) {
    statuses
        .write()
        .await
        .processes
        .insert(name.to_owned(), status);
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hermes_restart_leaves_simplex_running_and_shutdown_stops_it() {
        let handle = start_supervisor(
            sleeping_process("sidecar"),
            sleeping_process("health"),
            sleeping_process("hermes"),
            Some(sleeping_process("simplex")),
        );
        let simplex_pid = wait_for_running(&handle, "simplex").await.pid();
        handle.restart_hermes().await.unwrap();
        assert_eq!(
            handle.status().await.processes["simplex"].pid(),
            simplex_pid
        );
        handle.shutdown().await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if matches!(
                    handle.status().await.processes["simplex"].state,
                    ProcessState::Stopped
                ) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn restart_hermes_waits_for_a_new_running_process() {
        let handle = start_supervisor(
            sleeping_process("sidecar"),
            sleeping_process("health"),
            sleeping_process("hermes"),
            None,
        );
        let original_pid = wait_for_running(&handle, "hermes").await.pid().unwrap();

        handle.restart_hermes().await.unwrap();

        let restarted = handle.status().await.processes["hermes"].clone();
        assert!(matches!(restarted.state, ProcessState::Running { .. }));
        assert_eq!(restarted.restart_count, 1);
        assert_ne!(restarted.pid(), Some(original_pid));
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_delivers_a_real_sigterm_that_children_can_trap_and_drain_on() {
        // The graceful path agentd's own SIGTERM handler routes through:
        // supervisor.shutdown() must deliver SIGTERM in-process (the runtime
        // image has no `kill` binary) and give the child its bounded window
        // to finish work before any SIGKILL (PR 440, 83ef3024).
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("drained");
        let armed = dir.path().join("armed");
        let script = dir.path().join("trapper.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ntrap 'echo drained > {}; exit 0' TERM\necho armed > {}\nsleep 30 & wait\n",
                marker.display(),
                armed.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let handle = start_supervisor(
            ProcessSpec {
                name: "finitechat",
                program: script.clone(),
                args: Vec::new(),
                environment: BTreeMap::new(),
            },
            sleeping_process("health"),
            sleeping_process("hermes"),
            None,
        );
        wait_for_running(&handle, "finitechat").await;
        // Let the script arm its trap before signalling.
        tokio::time::timeout(Duration::from_secs(5), async {
            while !armed.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the stub must arm its trap");

        handle.shutdown().await;

        tokio::time::timeout(Duration::from_secs(5), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the child must observe SIGTERM and drain, not be SIGKILLed");
    }

    #[tokio::test]
    async fn the_post_exit_sweep_kills_orphaned_grandchildren() {
        // A gateway-forked straggler must not survive its supervisor slot:
        // the grandchild here ignores SIGTERM, so only the post-exit group
        // KILL sweep can remove it (the leaked bridge from PR 440's
        // ec46243e held :37633 exactly this way).
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");
        let script = dir.path().join("forker.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nsh -c 'trap \"\" TERM; while :; do sleep 1; done' &\necho $! > {}\nwait\n",
                pidfile.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let handle = start_supervisor(
            sleeping_process("finitechat"),
            sleeping_process("health"),
            ProcessSpec {
                name: "hermes",
                program: script.clone(),
                args: Vec::new(),
                environment: BTreeMap::new(),
            },
            None,
        );
        wait_for_running(&handle, "hermes").await;
        let grandchild = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(raw) = std::fs::read_to_string(&pidfile)
                    && let Ok(pid) = raw.trim().parse::<i32>()
                {
                    break rustix::process::Pid::from_raw(pid).expect("nonzero pid");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the stub must record its grandchild");

        handle.shutdown().await;

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                // A just-killed grandchild can linger as a zombie until init
                // reaps it; ESRCH is the only accepted terminal state.
                if rustix::process::test_kill_process(grandchild).is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the post-exit sweep must remove the orphaned grandchild");
    }

    fn script_process(name: &'static str, dir: &std::path::Path, body: &str) -> ProcessSpec {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(format!("{name}.sh"));
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        ProcessSpec {
            name,
            program: path,
            args: Vec::new(),
            environment: BTreeMap::new(),
        }
    }

    async fn wait_for_file(path: &std::path::Path) -> String {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(raw) = std::fs::read_to_string(path)
                    && !raw.is_empty()
                {
                    return raw;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{} was never written", path.display()))
    }

    fn shutdown_bound() -> Duration {
        GATEWAY_STOP_GRACE + SERVICE_STOP_GRACE + STOP_REAP_SLACK * 2
    }

    #[test]
    fn worst_case_shutdown_fits_the_smoke_stop_window() {
        // `docker stop --time 15` in hermes-chat-interruption-docker-smoke.py;
        // keep headroom for the entrypoint and container teardown.
        assert!(shutdown_bound() <= Duration::from_secs(13));
    }

    fn pid_is_gone(pid: u32) -> bool {
        rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid as i32).unwrap())
            .is_err()
    }

    #[tokio::test]
    async fn shutdown_drains_hermes_while_the_sidecar_serves_and_returns_after_reap() {
        // The graceful-stop smoke failure: shutdown only enqueued Stop, agentd
        // returned within ~150ms, and dropping the runtime SIGKILLed Hermes
        // before its adapter could release inbox leases through the sidecar.
        // Hermes here needs a second of drain and records whether the
        // sidecar was still alive when it finished.
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().display();
        let sidecar = script_process(
            "finitechat",
            dir.path(),
            &format!("echo $$ > {d}/sidecar.pid\nexec sleep 30\n"),
        );
        let hermes = script_process(
            "hermes",
            dir.path(),
            &format!(
                "trap 'sleep 1; if kill -0 \"$(cat {d}/sidecar.pid)\"; then echo alive; else echo gone; fi > {d}/drained; exit 0' TERM\n\
                 echo $$ > {d}/hermes.pid\nsleep 30 & wait\n"
            ),
        );
        let handle = start_supervisor(sidecar, sleeping_process("health"), hermes, None);
        wait_for_running(&handle, "hermes").await;
        let sidecar_pid: u32 = wait_for_file(&dir.path().join("sidecar.pid"))
            .await
            .trim()
            .parse()
            .unwrap();
        let hermes_pid: u32 = wait_for_file(&dir.path().join("hermes.pid"))
            .await
            .trim()
            .parse()
            .unwrap();

        tokio::time::timeout(shutdown_bound(), handle.shutdown())
            .await
            .expect("shutdown is bounded");

        // No polling: everything must already be settled when shutdown returns.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("drained"))
                .ok()
                .as_deref(),
            Some("alive\n"),
            "Hermes must finish draining before shutdown returns, with the sidecar still up"
        );
        assert!(pid_is_gone(hermes_pid));
        assert!(pid_is_gone(sidecar_pid));
        let status = handle.status().await;
        for name in ["finitechat", "health", "hermes"] {
            assert_eq!(
                status.processes[name].state,
                ProcessState::Stopped,
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn shutdown_kills_a_child_that_ignores_sigterm_within_the_bound() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().display();
        let hermes = script_process(
            "hermes",
            dir.path(),
            &format!("trap '' TERM\necho $$ > {d}/hermes.pid\nwhile :; do sleep 1; done\n"),
        );
        let handle = start_supervisor(
            sleeping_process("finitechat"),
            sleeping_process("health"),
            hermes,
            None,
        );
        let hermes_pid: u32 = wait_for_file(&dir.path().join("hermes.pid"))
            .await
            .trim()
            .parse()
            .unwrap();
        let started = std::time::Instant::now();
        handle.shutdown().await;
        let elapsed = started.elapsed();
        assert!(
            elapsed >= GATEWAY_STOP_GRACE,
            "SIGKILL came before the grace: {elapsed:?}"
        );
        assert!(
            elapsed < shutdown_bound(),
            "shutdown overran its bound: {elapsed:?}"
        );
        assert!(pid_is_gone(hermes_pid));
        assert_eq!(
            handle.status().await.processes["hermes"].state,
            ProcessState::Stopped
        );
    }

    #[tokio::test]
    async fn shutdown_stops_a_slot_stuck_retrying_an_unavailable_program() {
        let missing = ProcessSpec {
            name: "hermes",
            program: PathBuf::from("/nonexistent/finite-agentd-test-hermes"),
            args: Vec::new(),
            environment: BTreeMap::new(),
        };
        let handle = start_supervisor(
            sleeping_process("finitechat"),
            sleeping_process("health"),
            missing,
            None,
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while !matches!(
                handle
                    .status()
                    .await
                    .processes
                    .get("hermes")
                    .map(|s| &s.state),
                Some(ProcessState::Unavailable { .. })
            ) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(3), handle.shutdown())
            .await
            .expect("a retrying slot must take the stop instead of looping on");
        assert_eq!(
            handle.status().await.processes["hermes"].state,
            ProcessState::Stopped
        );
    }

    #[tokio::test]
    async fn shutdown_during_respawn_backoff_never_spawns_again() {
        let dir = tempfile::tempdir().unwrap();
        let spawns = dir.path().join("spawns");
        let hermes = script_process(
            "hermes",
            dir.path(),
            &format!("echo spawn >> {}\nexit 1\n", spawns.display()),
        );
        let handle = start_supervisor(
            sleeping_process("finitechat"),
            sleeping_process("health"),
            hermes,
            None,
        );
        wait_for_file(&spawns).await;
        tokio::time::timeout(Duration::from_secs(3), handle.shutdown())
            .await
            .expect("shutdown is bounded");
        let after_stop = std::fs::read_to_string(&spawns).unwrap();
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(
            std::fs::read_to_string(&spawns).unwrap(),
            after_stop,
            "respawned after stop"
        );
        assert_eq!(
            handle.status().await.processes["hermes"].state,
            ProcessState::Stopped
        );
    }

    #[tokio::test]
    async fn shutdown_racing_a_hermes_restart_stops_for_good() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().display();
        let hermes = script_process(
            "hermes",
            dir.path(),
            &format!("echo $$ >> {d}/hermes.pids\nexec sleep 30\n"),
        );
        let handle = start_supervisor(
            sleeping_process("finitechat"),
            sleeping_process("health"),
            hermes,
            None,
        );
        wait_for_file(&dir.path().join("hermes.pids")).await;
        let restarter = handle.clone();
        let restart = tokio::spawn(async move { restarter.restart_hermes().await });
        tokio::task::yield_now().await;
        tokio::time::timeout(shutdown_bound(), handle.shutdown())
            .await
            .expect("shutdown is bounded");
        // Either the restart completed before the stop or it reports the
        // stop; it must not sit out its 30s wait on a slot that is gone.
        tokio::time::timeout(Duration::from_secs(3), restart)
            .await
            .expect("restart must observe the stop")
            .unwrap()
            .ok();
        let pids = std::fs::read_to_string(dir.path().join("hermes.pids")).unwrap();
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(
            std::fs::read_to_string(dir.path().join("hermes.pids")).unwrap(),
            pids,
            "respawned after stop"
        );
        for pid in pids.lines() {
            assert!(
                pid_is_gone(pid.parse().unwrap()),
                "Hermes {pid} outlived shutdown"
            );
        }
        assert_eq!(
            handle.status().await.processes["hermes"].state,
            ProcessState::Stopped
        );
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
                    && matches!(status.state, ProcessState::Running { .. })
                {
                    return status.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

    // The serialized ProcessStatus shape is a cross-process contract: it ships
    // in the published `finite.agent.status.v1` snapshot and in status.json,
    // which `finite-agentd status --json` prints for external consumers. These
    // tests pin the exact bytes per state.
    fn assert_wire_shape(status: ProcessStatus, json: &str) {
        assert_eq!(serde_json::to_string(&status).unwrap(), json);
        assert_eq!(serde_json::from_str::<ProcessStatus>(json).unwrap(), status);
    }

    #[test]
    fn starting_process_status_keeps_the_published_wire_shape() {
        assert_wire_shape(
            ProcessStatus {
                state: ProcessState::Starting,
                restart_count: 0,
                updated_at_ms: 1_700_000_000_000,
            },
            r#"{"state":"starting","pid":null,"restart_count":0,"last_exit":null,"updated_at_ms":1700000000000}"#,
        );
    }

    #[test]
    fn restarting_process_status_keeps_the_published_wire_shape() {
        assert_wire_shape(
            ProcessStatus {
                state: ProcessState::Restarting,
                restart_count: 1,
                updated_at_ms: 1_700_000_000_000,
            },
            r#"{"state":"restarting","pid":null,"restart_count":1,"last_exit":null,"updated_at_ms":1700000000000}"#,
        );
    }

    #[test]
    fn running_process_status_keeps_the_published_wire_shape() {
        assert_wire_shape(
            ProcessStatus {
                state: ProcessState::Running { pid: 4242 },
                restart_count: 1,
                updated_at_ms: 1_700_000_000_000,
            },
            r#"{"state":"running","pid":4242,"restart_count":1,"last_exit":null,"updated_at_ms":1700000000000}"#,
        );
    }

    #[test]
    fn unavailable_process_status_keeps_the_published_wire_shape() {
        assert_wire_shape(
            ProcessStatus {
                state: ProcessState::Unavailable {
                    error: "program not found".to_owned(),
                },
                restart_count: 2,
                updated_at_ms: 1_700_000_000_000,
            },
            r#"{"state":"unavailable","pid":null,"restart_count":2,"last_exit":"program not found","updated_at_ms":1700000000000}"#,
        );
    }

    #[test]
    fn exited_process_status_keeps_the_published_wire_shape() {
        assert_wire_shape(
            ProcessStatus {
                state: ProcessState::Exited {
                    exit: "exit status: 1".to_owned(),
                },
                restart_count: 3,
                updated_at_ms: 1_700_000_000_000,
            },
            r#"{"state":"exited","pid":null,"restart_count":3,"last_exit":"exit status: 1","updated_at_ms":1700000000000}"#,
        );
    }

    #[test]
    fn stopped_process_status_keeps_the_published_wire_shape() {
        assert_wire_shape(
            ProcessStatus {
                state: ProcessState::Stopped,
                restart_count: 4,
                updated_at_ms: 1_700_000_000_000,
            },
            r#"{"state":"stopped","pid":null,"restart_count":4,"last_exit":null,"updated_at_ms":1700000000000}"#,
        );
    }

    #[test]
    fn process_status_deserialization_fails_closed_on_inconsistent_states() {
        assert!(
            serde_json::from_str::<ProcessStatus>(
                r#"{"state":"running","pid":null,"restart_count":0,"last_exit":null,"updated_at_ms":0}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<ProcessStatus>(
                r#"{"state":"stopped","pid":4242,"restart_count":0,"last_exit":null,"updated_at_ms":0}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<ProcessStatus>(
                r#"{"state":"runnign","pid":null,"restart_count":0,"last_exit":null,"updated_at_ms":0}"#
            )
            .is_err()
        );
    }
}
