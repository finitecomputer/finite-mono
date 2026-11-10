//! Every program the mover starts (Core's CLI, a host's reader) runs as the
//! leader of a process group of its own, watched. Once the mover has waited
//! on it past its limit, the group is asked to stop (TERM), and after a
//! grace the whole group is killed. One rule keeps that safe and complete:
//! the leader is not reaped until the group's cleanup is over, so the
//! group's id stays this program's and a kill can never reach another; and
//! a program that went past its limit is reported only once its group has
//! no live member. One stuck program never holds the mailbox, and no part
//! of it outlives its deadline.

use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustix::io::Errno;
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};

/// How long a group asked to stop has before it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(10);

/// How long a killed group has to be gone before it's reported as not.
const GONE_WITHIN: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("could not be started: {0}")]
    NotRun(io::Error),
    #[error("had not ended after {0} s, so it was stopped")]
    Stuck(u64),
    #[error("was killed, but part of it was still running {} s later", GONE_WITHIN.as_secs())]
    Lingering,
}

/// How a watched program ended.
pub struct Ended {
    pub status: ExitStatus,
    /// It went past its limit and its group was stopped.
    pub fired: bool,
}

#[derive(Default)]
struct Watch {
    /// Since when the mover has been waiting on the program, if it is.
    waiting: Option<Instant>,
    /// The mover is done waiting on the program.
    done: bool,
    /// The mover is ending it now: no grace.
    now: bool,
    fired: bool,
}

type Shared = Arc<(Mutex<Watch>, Condvar)>;

/// A program alone in its process group, stopped once the mover has waited
/// on it past its limit. Time the mover spends elsewhere (sending a part,
/// say) doesn't count.
pub struct Watched {
    child: Child,
    group: Pid,
    shared: Shared,
    thread: Option<JoinHandle<()>>,
    reaped: bool,
}

impl Watched {
    pub fn spawn(command: &mut Command, limit: Duration) -> io::Result<Self> {
        let child = command.process_group(0).spawn()?;
        let group = i32::try_from(child.id())
            .ok()
            .and_then(Pid::from_raw)
            .ok_or_else(|| io::Error::other("a child with no process id"))?;
        let shared: Shared = Arc::new((Mutex::new(Watch::default()), Condvar::new()));
        let theirs = shared.clone();
        let thread = std::thread::spawn(move || watch(&theirs, group, limit));
        Ok(Self {
            child,
            group,
            shared,
            thread: Some(thread),
            reaped: false,
        })
    }

    pub fn stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    fn state(&self) -> MutexGuard<'_, Watch> {
        self.shared.0.lock().expect("the watch")
    }

    /// The mover starts (or stops) waiting on the program.
    pub fn waiting(&self, waiting: bool) {
        self.state().waiting = waiting.then(Instant::now);
        self.shared.1.notify_all();
    }

    /// Wait for the leader to exit, leaving it unreaped.
    pub fn exited(&self) -> io::Result<()> {
        loop {
            match waitid(
                WaitId::Pid(self.group),
                WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
            ) {
                Ok(_) => return Ok(()),
                Err(Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// End it: if it went past its limit, its group is stopped and killed
    /// after the grace; with `now`, killed at once. Only then is the leader
    /// reaped, and a stopped group must be gone before this returns.
    pub fn end(&mut self, now: bool) -> Result<Ended, RunError> {
        {
            let mut watch = self.state();
            watch.done = true;
            watch.now = now;
        }
        self.shared.1.notify_all();
        if now {
            // Its leader isn't reaped: the group is still this program's.
            let _ = kill_process_group(self.group, Signal::KILL);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let fired = self.state().fired;
        let status = self.child.wait().map_err(RunError::NotRun)?;
        self.reaped = true;
        if (fired || now) && !gone(self.group) {
            return Err(RunError::Lingering);
        }
        Ok(Ended { status, fired })
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.end(true);
        }
    }
}

/// The watchdog: TERM once the mover has waited past `limit`, then KILL
/// after the grace, whatever the mover does meanwhile (the leader stays
/// unreaped until this returns).
fn watch(shared: &Shared, group: Pid, limit: Duration) {
    let (lock, wake) = &**shared;
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
    let _ = kill_process_group(group, Signal::TERM);
    let deadline = Instant::now() + STOP_GRACE;
    while !watch.now {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        watch = wake.wait_timeout(watch, left).expect("the watch").0;
    }
    let _ = kill_process_group(group, Signal::KILL);
}

/// Whether `group` has no live member left (its killed members may wait
/// to be reaped by their new parent), within `GONE_WITHIN`.
fn gone(group: Pid) -> bool {
    let deadline = Instant::now() + GONE_WITHIN;
    loop {
        if !has_live_member(group) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether a process not yet dead is in `group`, from `/proc`.
fn has_live_member(group: Pid) -> bool {
    let group = group.as_raw_nonzero().get().to_string();
    let Ok(processes) = std::fs::read_dir("/proc") else {
        return true;
    };
    processes.flatten().any(|process| {
        let Ok(stat) = std::fs::read_to_string(process.path().join("stat")) else {
            return false;
        };
        // `pid (comm) state ppid pgrp …`: comm may hold anything but the
        // last `)`.
        let Some((_, rest)) = stat.rsplit_once(')') else {
            return false;
        };
        let mut fields = rest.split_whitespace();
        let state = fields.next().unwrap_or("");
        let pgrp = fields.nth(1).unwrap_or("");
        pgrp == group && !matches!(state, "Z" | "X" | "x")
    })
}

/// Run `command` to its end, alone in its group, with no input, stopped
/// once it runs past `limit`, counted from its start to its pipes' end (a
/// program that lingers holding them is stopped too).
pub fn run_within(
    command: &mut Command,
    limit: Duration,
) -> Result<(Ended, Vec<u8>, Vec<u8>), RunError> {
    let mut watched = Watched::spawn(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        limit,
    )
    .map_err(RunError::NotRun)?;
    watched.waiting(true);
    let stdout = read_all(watched.stdout().expect("a piped stdout"));
    let stderr = read_all(watched.stderr().expect("a piped stderr"));
    watched.exited().map_err(RunError::NotRun)?;
    let (stdout, stderr) = (stdout.join(), stderr.join());
    let ended = watched.end(false)?;
    if ended.fired {
        return Err(RunError::Stuck(limit.as_secs()));
    }
    Ok((
        ended,
        stdout.unwrap_or_default(),
        stderr.unwrap_or_default(),
    ))
}

/// A pipe read to its end beside the program.
fn read_all(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}
