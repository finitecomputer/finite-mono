//! Bounded Git subprocess IO. The caller owns the stdout destination; stderr is
//! counted and discarded. Every exit kills the process group and reaps its leader.

use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::{BackupError, limits};

pub(super) fn git(repo: &Path, args: &[&str]) -> Result<Command, BackupError> {
    let repo = repo.canonicalize()?;
    let mut command = Command::new("git");
    command
        .current_dir(&repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_TEMPLATE_DIR")
        .args(["--git-dir"])
        .arg(repo)
        .args([
            "--no-replace-objects",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "-c",
            "init.templateDir=/dev/null",
        ])
        .args(args);
    Ok(command)
}

#[cfg(unix)]
pub(super) fn run(
    mut command: Command,
    stdout: &mut dyn Write,
    stdout_limit: u64,
) -> Result<(), BackupError> {
    use std::os::unix::process::CommandExt;

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let started = Instant::now();
    let mut child = ProcessGroup {
        child: command.spawn()?,
        stopped: false,
    };
    let mut out = child.child.stdout.take().expect("stdout is piped");
    let mut err = child.child.stderr.take().expect("stderr is piped");
    nonblocking(&out)?;
    nonblocking(&err)?;
    let mut out_done = false;
    let mut err_done = false;
    let mut out_bytes = 0;
    let mut err_bytes = 0;
    let mut status = None;
    let mut buffer = [0; limits::BACKUP_PROCESS_BUFFER_BYTES as usize];
    // The deadline bounds even a child that never writes or closes its pipes.
    loop {
        if started.elapsed() >= Duration::from_secs(limits::MAX_BACKUP_GIT_SECONDS) {
            return Err(BackupError::Invalid("Git recovery command timed out"));
        }
        let out_progress = drain(
            &mut out,
            stdout,
            &mut out_bytes,
            stdout_limit,
            &mut out_done,
            &mut buffer,
            "Git stdout exceeds limit",
        )?;
        let err_progress = drain(
            &mut err,
            &mut std::io::sink(),
            &mut err_bytes,
            limits::MAX_BACKUP_GIT_STDERR_BYTES,
            &mut err_done,
            &mut buffer,
            "Git stderr exceeds limit",
        )?;
        if status.is_none() {
            status = child.child.try_wait()?;
            if status.is_some() {
                // A successful leader must not leave background descendants alive.
                child.stop();
            }
        }
        if let Some(status) = status
            && out_done
            && err_done
        {
            return if status.success() {
                Ok(())
            } else {
                Err(BackupError::Invalid("Git recovery command failed"))
            };
        }
        if !out_progress && !err_progress {
            let mut pipes = [
                libc::pollfd {
                    fd: if out_done { -1 } else { out.as_raw_fd() },
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: if err_done { -1 } else { err.as_raw_fd() },
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // Both descriptors remain owned by out/err while poll borrows them.
            if unsafe {
                libc::poll(
                    pipes.as_mut_ptr(),
                    pipes.len() as libc::nfds_t,
                    limits::BACKUP_PROCESS_POLL_MILLIS as libc::c_int,
                )
            } == -1
            {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    return Err(error.into());
                }
            }
        }
    }
}

#[cfg(not(unix))]
pub(super) fn run(_: Command, _: &mut dyn Write, _: u64) -> Result<(), BackupError> {
    Err(BackupError::Invalid(
        "bounded Git recovery requires Unix process groups",
    ))
}

#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> std::io::Result<()> {
    let fd = pipe.as_raw_fd();
    // The live pipe owns fd throughout both fcntl calls.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn drain(
    pipe: &mut impl Read,
    sink: &mut dyn Write,
    total: &mut u64,
    limit: u64,
    done: &mut bool,
    buffer: &mut [u8],
    error: &'static str,
) -> Result<bool, BackupError> {
    if *done {
        return Ok(false);
    }
    match pipe.read(buffer) {
        Ok(0) => {
            *done = true;
            Ok(false)
        }
        Ok(count) => {
            if count as u64 > limit.saturating_sub(*total) {
                return Err(BackupError::Invalid(error));
            }
            sink.write_all(&buffer[..count])?;
            *total += count as u64;
            Ok(true)
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
struct ProcessGroup {
    child: Child,
    stopped: bool,
}

#[cfg(unix)]
impl ProcessGroup {
    fn stop(&mut self) {
        if !self.stopped {
            // process_group(0) made the child PID its group ID. Negative IDs
            // target the whole group, including pack-objects and pipe holders.
            unsafe {
                libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
            }
            let _ = self.child.wait();
            self.stopped = true;
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.stop();
    }
}
