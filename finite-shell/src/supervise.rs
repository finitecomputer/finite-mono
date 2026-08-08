//! The shell supervises exactly one child: the active generation's
//! `finite-agentd serve`. Unexpected exits restart it with exponential
//! backoff (1s..60s); a crash loop (>= 5 restarts in 10 minutes) is recorded
//! in state.json and surfaced in `/healthz`, but the shell keeps trying.
//! `stop()` is the flip's quiesce: SIGTERM, bounded wait, then SIGKILL.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

use crate::ShellError;
use crate::state::{CrashLoopRecord, SharedState, now_rfc3339};

/// What to run and how patiently to restart it.
#[derive(Debug, Clone)]
pub struct AgentdSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Set on top of the shell's own (inherited) environment.
    pub env: BTreeMap<String, String>,
    pub backoff_initial: Duration,
    pub backoff_max: Duration,
    pub crash_loop_window: Duration,
    pub crash_loop_restarts: usize,
}

/// A live snapshot of the supervised process, serialized into `status` and
/// `/healthz`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentdSupervisionStatus {
    pub state: String,
    pub pid: Option<u32>,
    pub restart_count: u64,
    pub last_exit: Option<String>,
    pub crash_looping: bool,
    pub updated_at_ms: u64,
}

impl Default for AgentdSupervisionStatus {
    fn default() -> Self {
        Self {
            state: "starting".to_owned(),
            pid: None,
            restart_count: 0,
            last_exit: None,
            crash_looping: false,
            updated_at_ms: now_ms(),
        }
    }
}

/// Children whose exit statuses belong to tokio's `Child::wait` (the
/// supervised agentd). The PID 1 zombie reaper must never `waitpid` these.
static SUPERVISED_CHILD_PIDS: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

/// PID 1's init duty, which tokio's child machinery does not cover: children
/// the shell adopts — a `container exec` session's backgrounded processes,
/// double-forked daemons, and old-generation stragglers swept by
/// [`terminate`]'s group KILL — are never `wait`ed on when they die, so they
/// sit in the process table as zombies forever. A zombie still satisfies
/// `kill -0`, so anything polling "is that pid gone yet" hangs (observed: a
/// smoke's backgrounded `fbrain daemon supervise` stayed `kill -0`-alive
/// after its SIGTERM because the dead process was never reaped). entrypoint.sh
/// used to do this implicitly: bash as PID 1 reaps every adopted child.
pub fn spawn_zombie_reaper() {
    if !rustix::process::getpid().is_init() {
        return;
    }
    tokio::spawn(async {
        let Ok(mut sigchld) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::child())
        else {
            return;
        };
        loop {
            sigchld.recv().await;
            reap_adopted_zombies();
        }
    });
}

fn reap_adopted_zombies() {
    // Holding the registry lock across the scan closes the race with
    // [`spawn`]: a just-spawned agentd is registered before this reaper can
    // observe it as a zombie.
    let supervised = SUPERVISED_CHILD_PIDS.lock().expect("registry poisoned");
    for pid in zombie_children_of_self() {
        if supervised.contains(&pid) {
            continue;
        }
        if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
            let _ = rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG);
        }
    }
}

fn zombie_children_of_self() -> Vec<u32> {
    let own_pid = std::process::id();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut zombies = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // `/proc/<pid>/stat` is "pid (comm) state ppid ..." and comm may
        // itself contain spaces or parentheses; parse after the last ')'.
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let (Some(state), Some(ppid)) = (fields.next(), fields.next()) else {
            continue;
        };
        if state == "Z" && ppid.parse() == Ok(own_pid) {
            zombies.push(pid);
        }
    }
    zombies
}

fn release_supervised_pid(pid: Option<u32>) {
    if let Some(pid) = pid {
        SUPERVISED_CHILD_PIDS
            .lock()
            .expect("registry poisoned")
            .remove(&pid);
    }
}

enum Action {
    Stop {
        quiesce: Duration,
        done: oneshot::Sender<()>,
    },
}

/// Handle to the one supervised agentd. Dropping it does not stop the child;
/// call [`AgentdSupervisor::stop`].
pub struct AgentdSupervisor {
    actions: mpsc::Sender<Action>,
    status: Arc<RwLock<AgentdSupervisionStatus>>,
}

impl AgentdSupervisor {
    pub fn status(&self) -> AgentdSupervisionStatus {
        self.status.read().expect("status lock poisoned").clone()
    }

    /// Quiesce: SIGTERM the child, wait up to `quiesce` for exit, then
    /// SIGKILL. Resolves once the supervision task has fully stopped.
    pub async fn stop(&self, quiesce: Duration) -> Result<(), ShellError> {
        let (done, finished) = oneshot::channel();
        if self
            .actions
            .send(Action::Stop { quiesce, done })
            .await
            .is_err()
        {
            // The task already stopped (e.g. an earlier stop).
            return Ok(());
        }
        finished
            .await
            .map_err(|_| ShellError::Supervisor("supervision task vanished during stop".to_owned()))
    }
}

/// Start supervising. Clears any recorded crash loop: a fresh start is a
/// fresh observation window.
pub fn start_agentd(spec: AgentdSpec, shared_state: SharedState) -> AgentdSupervisor {
    let status = Arc::new(RwLock::new(AgentdSupervisionStatus::default()));
    let (actions_tx, actions_rx) = mpsc::channel(4);
    let _ = shared_state.update(|state| state.agentd_crash_loop = None);
    tokio::spawn(supervise(
        spec,
        actions_rx,
        Arc::clone(&status),
        shared_state,
    ));
    AgentdSupervisor {
        actions: actions_tx,
        status,
    }
}

async fn supervise(
    spec: AgentdSpec,
    mut actions: mpsc::Receiver<Action>,
    status: Arc<RwLock<AgentdSupervisionStatus>>,
    shared_state: SharedState,
) {
    let mut restart_count = 0_u64;
    let mut backoff = spec.backoff_initial;
    let mut recent_exits: Vec<Instant> = Vec::new();
    let mut crash_looping = false;
    loop {
        set_status(&status, |snapshot| {
            snapshot.state = if restart_count == 0 {
                "starting".to_owned()
            } else {
                "restarting".to_owned()
            };
            snapshot.pid = None;
            snapshot.restart_count = restart_count;
        });
        let mut child = match spawn(&spec) {
            Ok(child) => child,
            Err(error) => {
                set_status(&status, |snapshot| {
                    snapshot.state = "unavailable".to_owned();
                    snapshot.last_exit = Some(error.to_string());
                });
                note_exit(
                    &spec,
                    &mut recent_exits,
                    &mut crash_looping,
                    &status,
                    &shared_state,
                );
                restart_count = restart_count.saturating_add(1);
                if wait_or_stop(&mut actions, backoff).await {
                    return;
                }
                backoff = (backoff * 2).min(spec.backoff_max);
                continue;
            }
        };
        let started_at = Instant::now();
        let child_pid = child.id();
        set_status(&status, |snapshot| {
            snapshot.state = "running".to_owned();
            snapshot.pid = child_pid;
        });

        tokio::select! {
            result = child.wait() => {
                // tokio reaped the status; the pid may now be reused freely.
                release_supervised_pid(child_pid);
                let exit = result
                    .map(|exit_status| exit_status.to_string())
                    .unwrap_or_else(|error| error.to_string());
                set_status(&status, |snapshot| {
                    snapshot.state = "exited".to_owned();
                    snapshot.pid = None;
                    snapshot.last_exit = Some(exit);
                });
                // A child that ran a while earns a fresh backoff.
                if started_at.elapsed() >= spec.backoff_max {
                    backoff = spec.backoff_initial;
                }
                note_exit(
                    &spec,
                    &mut recent_exits,
                    &mut crash_looping,
                    &status,
                    &shared_state,
                );
                restart_count = restart_count.saturating_add(1);
                if wait_or_stop(&mut actions, backoff).await {
                    return;
                }
                backoff = (backoff * 2).min(spec.backoff_max);
            }
            action = actions.recv() => {
                let Some(Action::Stop { quiesce, done }) = action else {
                    // All handles dropped: keep the child running until the
                    // process (PID 1) itself exits.
                    let _ = child.wait().await;
                    release_supervised_pid(child_pid);
                    return;
                };
                terminate(&mut child, quiesce).await;
                release_supervised_pid(child_pid);
                set_status(&status, |snapshot| {
                    snapshot.state = "stopped".to_owned();
                    snapshot.pid = None;
                });
                let _ = done.send(());
                return;
            }
        }
    }
}

/// Sleep for `delay` but honor a Stop that arrives mid-backoff. Returns true
/// when the task must exit.
async fn wait_or_stop(actions: &mut mpsc::Receiver<Action>, delay: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        action = actions.recv() => {
            if let Some(Action::Stop { done, .. }) = action {
                let _ = done.send(());
            }
            true
        }
    }
}

fn note_exit(
    spec: &AgentdSpec,
    recent_exits: &mut Vec<Instant>,
    crash_looping: &mut bool,
    status: &Arc<RwLock<AgentdSupervisionStatus>>,
    shared_state: &SharedState,
) {
    let now = Instant::now();
    recent_exits.push(now);
    recent_exits.retain(|at| now.duration_since(*at) <= spec.crash_loop_window);
    if recent_exits.len() >= spec.crash_loop_restarts && !*crash_looping {
        *crash_looping = true;
        set_status(status, |snapshot| snapshot.crash_looping = true);
        let record = CrashLoopRecord {
            restarts: recent_exits.len(),
            window_secs: spec.crash_loop_window.as_secs(),
            recorded_at: now_rfc3339(),
        };
        let _ = shared_state.update(|state| state.agentd_crash_loop = Some(record));
    }
}

fn spawn(spec: &AgentdSpec) -> Result<Child, ShellError> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(&spec.env)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        // agentd leads its own process group so quiesce can signal the whole
        // tree: gateway scripts fork background loops, and any straggler from
        // the old generation would otherwise survive a flip holding loopback
        // ports (observed: an orphaned finitechat bridge kept :37633, so the
        // new generation's bridge exited at bind and the flip looked broken).
        .process_group(0)
        .kill_on_drop(true);
    // Register under the reaper's lock BEFORE the child can be observed
    // exiting: the PID 1 zombie reaper must not steal this exit status from
    // tokio's `Child::wait`.
    let mut supervised = SUPERVISED_CHILD_PIDS.lock().expect("registry poisoned");
    let child = command.spawn().map_err(ShellError::from)?;
    if let Some(pid) = child.id() {
        supervised.insert(pid);
    }
    Ok(child)
}

fn signal_group(pid: u32, signal: rustix::process::Signal) {
    // Signal agentd's whole process group (it is spawned as group leader).
    // In-process syscall: the runtime image has no `kill` binary (only the
    // sh builtin), so shelling out silently signalled nothing — which is how
    // every generation's children were being orphaned across flips.
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process_group(pid, signal);
    }
}

async fn terminate(child: &mut Child, quiesce: Duration) {
    let pid = child.id();
    if let Some(pid) = pid {
        signal_group(pid, rustix::process::Signal::TERM);
    }
    if tokio::time::timeout(quiesce, child.wait()).await.is_err() {
        if let Some(pid) = pid {
            signal_group(pid, rustix::process::Signal::KILL);
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    // The direct child is gone; sweep the group once more so no orphaned
    // grandchild survives into the next generation.
    if let Some(pid) = pid {
        signal_group(pid, rustix::process::Signal::KILL);
    }
}

fn set_status(
    status: &Arc<RwLock<AgentdSupervisionStatus>>,
    mutate: impl FnOnce(&mut AgentdSupervisionStatus),
) {
    let mut snapshot = status.write().expect("status lock poisoned");
    mutate(&mut snapshot);
    snapshot.updated_at_ms = now_ms();
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::*;

    fn script_spec(dir: &Path, body: &str) -> AgentdSpec {
        let path = dir.join("stub-agentd");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        AgentdSpec {
            program: path,
            args: vec!["serve".to_owned()],
            env: BTreeMap::new(),
            backoff_initial: Duration::from_millis(10),
            backoff_max: Duration::from_millis(40),
            crash_loop_window: Duration::from_secs(600),
            crash_loop_restarts: 5,
        }
    }

    async fn wait_until(
        supervisor: &AgentdSupervisor,
        what: &str,
        predicate: impl Fn(&AgentdSupervisionStatus) -> bool,
    ) -> AgentdSupervisionStatus {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let status = supervisor.status();
                if predicate(&status) {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }

    #[tokio::test]
    async fn a_running_agentd_is_stopped_gracefully_on_quiesce() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("term-marker");
        let armed = dir.path().join("trap-armed");
        let spec = script_spec(
            dir.path(),
            &format!(
                "trap 'echo terminated > {}; exit 0' TERM\necho armed > {}\nsleep 60 & wait",
                marker.display(),
                armed.display()
            ),
        );
        let state = SharedState::load(&dir.path().join("state.json"));
        let supervisor = start_agentd(spec, state);
        wait_until(&supervisor, "running", |status| {
            status.state == "running" && status.pid.is_some()
        })
        .await;
        // Only quiesce once the script has installed its TERM trap —
        // otherwise the signal races shell startup and the default action
        // kills the script before it can write the marker.
        tokio::time::timeout(Duration::from_secs(10), async {
            while !armed.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the stub must arm its trap");

        supervisor.stop(Duration::from_secs(5)).await.unwrap();
        assert_eq!(supervisor.status().state, "stopped");
        // The trap ran: SIGTERM came first, not a kill.
        tokio::time::timeout(Duration::from_secs(10), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the child must observe SIGTERM");
    }

    #[tokio::test]
    async fn a_crash_looping_agentd_is_recorded_in_state_but_kept_restarting() {
        let dir = tempfile::tempdir().unwrap();
        let spec = script_spec(dir.path(), "exit 1");
        let state = SharedState::load(&dir.path().join("state.json"));
        let supervisor = start_agentd(spec, state.clone());

        let status = wait_until(&supervisor, "crash loop", |status| status.crash_looping).await;
        assert!(status.restart_count >= 4);
        let recorded = state.snapshot().agentd_crash_loop.expect("recorded");
        assert!(recorded.restarts >= 5);
        assert_eq!(recorded.window_secs, 600);

        // Still restarting after the loop was flagged.
        let before = supervisor.status().restart_count;
        wait_until(&supervisor, "keeps trying", |status| {
            status.restart_count > before
        })
        .await;
        supervisor.stop(Duration::from_secs(1)).await.unwrap();
    }

    /// The PID 1 init duty: an adopted child's death must be reaped, not
    /// left as a zombie that keeps satisfying `kill -0`, while the
    /// supervised agentd's exit status stays with tokio's `Child::wait`.
    /// Linux-only: it relies on `PR_SET_CHILD_SUBREAPER` and `/proc` to
    /// stand in for being PID 1.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn an_adopted_zombie_is_reaped_but_the_supervised_agentd_is_not_stolen() {
        use rustix::process::{Pid, Signal, kill_process, test_kill_process};

        rustix::process::set_child_subreaper(Some(rustix::process::getpid()))
            .expect("become a subreaper");

        // A supervised agentd stub, registered by spawn().
        let dir = tempfile::tempdir().unwrap();
        let spec = script_spec(dir.path(), "sleep 60 & wait");
        let state = SharedState::load(&dir.path().join("state.json"));
        let supervisor = start_agentd(spec, state);
        wait_until(&supervisor, "running", |status| {
            status.state == "running" && status.pid.is_some()
        })
        .await;

        // An orphan: sh backgrounds a sleep and exits, so the sleep is
        // adopted by this (subreaper) process — the shape of a `container
        // exec` session's backgrounded daemon.
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 600 & echo $!"])
            .output()
            .expect("spawn the orphan factory");
        let orphan_raw = String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse::<i32>()
            .expect("orphan pid");
        let orphan = Pid::from_raw(orphan_raw).expect("nonzero pid");
        let deadline = Instant::now() + Duration::from_secs(10);
        while fs::read_to_string(format!("/proc/{orphan_raw}/stat"))
            .ok()
            .and_then(|stat| {
                let (_, rest) = stat.rsplit_once(')')?;
                Some(rest.split_whitespace().nth(1)? == std::process::id().to_string())
            })
            != Some(true)
        {
            assert!(Instant::now() < deadline, "orphan was never adopted");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        kill_process(orphan, Signal::TERM).expect("terminate the orphan");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            reap_adopted_zombies();
            if test_kill_process(orphan).is_err() {
                break; // Fully reaped: the pid no longer exists.
            }
            assert!(
                Instant::now() < deadline,
                "the adopted zombie was never reaped"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // The supervised child was not stolen: a graceful stop still
        // observes the real exit.
        supervisor.stop(Duration::from_secs(5)).await.unwrap();
        assert_eq!(supervisor.status().state, "stopped");
    }

    #[tokio::test]
    async fn a_fresh_start_clears_the_recorded_crash_loop() {
        let dir = tempfile::tempdir().unwrap();
        let state = SharedState::load(&dir.path().join("state.json"));
        state
            .update(|shell_state| {
                shell_state.agentd_crash_loop = Some(CrashLoopRecord {
                    restarts: 9,
                    window_secs: 600,
                    recorded_at: now_rfc3339(),
                })
            })
            .unwrap();
        let spec = script_spec(dir.path(), "sleep 60 & wait");
        let supervisor = start_agentd(spec, state.clone());
        assert!(state.snapshot().agentd_crash_loop.is_none());
        supervisor.stop(Duration::from_secs(1)).await.unwrap();
    }
}
