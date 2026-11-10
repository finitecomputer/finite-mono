//! Every program the mover starts (Core's CLI, a host's reader) runs in a
//! process group of its own, watched: once the mover has waited on it
//! longer than its limit, the whole group is asked to stop (TERM), then
//! killed. One stuck program never holds the mailbox, whose lock would
//! keep every later run from doing anything.

use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};

/// How long a group asked to stop has before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(10);

/// Start `command` as the leader of a process group of its own.
pub fn spawn_alone(command: &mut Command) -> std::io::Result<Child> {
    command.process_group(0).spawn()
}

/// Kill `child`'s whole group (its leader not yet waited for, so the
/// group's id is still its own).
pub fn kill_group(child: &Child) {
    if let Some(group) = group_of(child) {
        let _ = kill_process_group(group, Signal::KILL);
    }
}

fn group_of(child: &Child) -> Option<Pid> {
    i32::try_from(child.id()).ok().and_then(Pid::from_raw)
}

#[derive(Default)]
struct Watch {
    /// Since when the mover has been waiting on the program, if it is.
    waiting: Option<Instant>,
    /// The mover is done with the program.
    done: bool,
    fired: bool,
}

type Shared = Arc<(Mutex<Watch>, Condvar)>;

/// Stops a program's group once the mover has waited on it past `limit`.
/// Time the mover spends elsewhere (sending a part, say) doesn't count.
pub struct Watchdog {
    shared: Shared,
    thread: Option<JoinHandle<()>>,
}

impl Watchdog {
    /// Watch `child` (started by `spawn_alone`), not yet waiting.
    pub fn watch(child: &Child, limit: Duration) -> Self {
        let group = group_of(child);
        let shared: Shared = Arc::new((Mutex::new(Watch::default()), Condvar::new()));
        let theirs = shared.clone();
        let thread = std::thread::spawn(move || {
            let (lock, wake) = &*theirs;
            let mut watch = lock.lock().expect("the watch");
            loop {
                if watch.done {
                    return;
                }
                watch = match watch.waiting {
                    None => wake.wait(watch).expect("the watch"),
                    Some(since) => {
                        let left = limit.saturating_sub(since.elapsed());
                        if left.is_zero() {
                            break;
                        }
                        wake.wait_timeout(watch, left).expect("the watch").0
                    }
                };
            }
            watch.fired = true;
            let Some(group) = group else { return };
            let _ = kill_process_group(group, Signal::TERM);
            let deadline = Instant::now() + STOP_GRACE;
            while !watch.done {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    // Still not done with it: its leader isn't waited for,
                    // so the group is still this program's.
                    let _ = kill_process_group(group, Signal::KILL);
                    return;
                }
                watch = wake.wait_timeout(watch, left).expect("the watch").0;
            }
        });
        Self {
            shared,
            thread: Some(thread),
        }
    }

    fn watch_state(&self) -> MutexGuard<'_, Watch> {
        self.shared.0.lock().expect("the watch")
    }

    /// The mover starts (or stops) waiting on the program.
    pub fn waiting(&self, waiting: bool) {
        self.watch_state().waiting = waiting.then(Instant::now);
        self.shared.1.notify_all();
    }

    /// Whether the program was stopped for going past its limit.
    pub fn fired(&self) -> bool {
        self.watch_state().fired
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.watch_state().done = true;
        self.shared.1.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
