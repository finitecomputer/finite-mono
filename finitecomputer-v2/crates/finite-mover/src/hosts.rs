//! The runner hosts, as the mover reads them: `finite-mover-host` on each,
//! through the mover's ssh key, whose forced command runs the reader and
//! nothing else (finite-mono #1145). The mover names the operation; the
//! host's own configuration names the work root.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::contract::short;
use crate::fleet::Binding;

/// The most of a host reader's error output kept for a failure's why.
const STDERR_KEPT: usize = 4096;

/// How to reach each runner host's reader: its Core `source_host_id` →
/// the command that runs it there, such as `["ssh", "-i", "<key>", "-o",
/// "BatchMode=yes", "finite-mover@finite-lat-3"]`. The operation's words
/// follow.
#[derive(Debug, Clone, Default)]
pub struct HostCommands {
    pub hosts: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("no way to reach runner host {0} is configured")]
    Unknown(String),
    #[error("the host reader could not be started: {0}")]
    NotRun(String),
}

impl HostCommands {
    fn command(&self, binding: &Binding, words: &[String]) -> Result<Command, HostError> {
        let argv = self
            .hosts
            .get(&binding.source_host_id)
            .ok_or_else(|| HostError::Unknown(binding.source_host_id.clone()))?;
        let (program, prefix) = argv
            .split_first()
            .ok_or_else(|| HostError::Unknown(binding.source_host_id.clone()))?;
        let mut command = Command::new(program);
        command.args(prefix).args(words);
        Ok(command)
    }

    /// `sizes <durable_state_id> [--brain <id>]...`: the host reader's
    /// summary from metadata alone (it opens no file), as the roster counts.
    pub fn sizes(&self, binding: &Binding) -> Result<crate::contract::Sizes, HostError> {
        let mut words = vec!["sizes".to_string(), binding.durable_state_id.clone()];
        for brain in &binding.owned_brains {
            words.extend(["--brain".to_string(), brain.clone()]);
        }
        let output = self
            .command(binding, &words)?
            .stdin(Stdio::null())
            .output()
            .map_err(|error| HostError::NotRun(error.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let last = stderr
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            return Err(HostError::NotRun(short(&format!(
                "{}: {}",
                output.status,
                last.trim()
            ))));
        }
        serde_json::from_slice(&output.stdout)
            .ok()
            .and_then(|summary| crate::sizes::sizes_of(&summary))
            .ok_or_else(|| HostError::NotRun("the host reader's sizes could not be read".into()))
    }

    /// `stream <durable_state_id> [--brain <id>]...`: the stage, as the
    /// host reader writes it. Its failure, or its exit before the end, is a
    /// read error at the stream's end.
    pub fn stream(&self, binding: &Binding) -> Result<HostStream, HostError> {
        let mut words = vec!["stream".to_string(), binding.durable_state_id.clone()];
        for brain in &binding.owned_brains {
            words.extend(["--brain".to_string(), brain.clone()]);
        }
        let mut child = self
            .command(binding, &words)?
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| HostError::NotRun(error.to_string()))?;
        let stdout = child.stdout.take().expect("a piped stdout");
        let stderr = child.stderr.take().expect("a piped stderr");
        let kept = Arc::new(Mutex::new(Vec::new()));
        let drain = drain_stderr(stderr, kept.clone());
        Ok(HostStream {
            child,
            stdout,
            stderr: kept,
            drain: Some(drain),
            ended: false,
        })
    }
}

/// Keep the end of `stderr`, read to its end beside the stream.
fn drain_stderr(mut stderr: ChildStderr, kept: Arc<Mutex<Vec<u8>>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while let Ok(read) = stderr.read(&mut buffer) {
            if read == 0 {
                break;
            }
            let mut kept = kept.lock().expect("the kept stderr");
            kept.extend_from_slice(&buffer[..read]);
            if kept.len() > STDERR_KEPT {
                let excess = kept.len() - STDERR_KEPT;
                kept.drain(..excess);
            }
        }
    })
}

/// The host reader's output. At its end the reader must have exited 0;
/// otherwise the end is an error, so a cut stream is never taken whole.
pub struct HostStream {
    child: Child,
    stdout: ChildStdout,
    stderr: Arc<Mutex<Vec<u8>>>,
    drain: Option<JoinHandle<()>>,
    ended: bool,
}

impl Read for HostStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.ended {
            return Ok(0);
        }
        let read = self.stdout.read(buffer)?;
        if read > 0 || buffer.is_empty() {
            return Ok(read);
        }
        let status = self.child.wait()?;
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
        if !status.success() {
            let stderr = self.stderr.lock().expect("the kept stderr");
            let stderr = String::from_utf8_lossy(&stderr);
            let last = stderr
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            return Err(io::Error::other(short(&format!(
                "the host reader exited {status}: {}",
                last.trim()
            ))));
        }
        self.ended = true;
        Ok(0)
    }
}

impl Drop for HostStream {
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
