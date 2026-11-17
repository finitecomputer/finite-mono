//! The runner hosts, as the mover reads them: `finite-mover-host` on each,
//! through the mover's ssh key, whose forced command runs the reader and
//! nothing else (finite-mono #1145). The mover names the operation; the
//! host's own configuration names the work root.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{ChildStderr, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use finite_mover_host::brains::PERSON_KEYS_MAX;
use finite_mover_host::overrides::{OVERRIDES_MAX, parse_path};
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::child::{Watched, run_within};
use crate::contract::{HandoffItem, SecretKind, short};
use crate::fleet::Binding;

/// The most of a host reader's error output kept for a failure's why.
const STDERR_KEPT: usize = 4096;

/// How long the mover waits on a host reader that sends nothing before it
/// is stopped (a tree on a local disk sends steadily).
pub const HOST_IDLE: Duration = Duration::from_secs(600);

/// How to reach each runner host's reader: its Core `source_host_id` →
/// the command that runs it there, such as `["ssh", "-i", "<key>", "-o",
/// "BatchMode=yes", "finite-mover@finite-lat-3"]`. The operation's words
/// follow.
#[derive(Debug, Clone)]
pub struct HostCommands {
    pub hosts: BTreeMap<String, Vec<String>>,
    /// How long a reader may send nothing while the mover waits on it
    /// (`HOST_IDLE`).
    pub idle: Duration,
    /// The operator's overrides file ([`HostCommands::overrides`]).
    pub overrides: Option<PathBuf>,
}

/// The largest overrides file read.
const OVERRIDES_FILE_MAX: u64 = 64 * 1024;

/// One entry of the overrides file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Overrides {
    #[serde(default)]
    carry: Vec<String>,
    #[serde(default)]
    leave: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("no way to reach runner host {0} is configured")]
    Unknown(String),
    #[error("the host reader could not be started: {0}")]
    NotRun(String),
    #[error("the operator's overrides: {0}")]
    Overrides(String),
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

    /// The operator's overrides for old agent `old`, as the host reader's
    /// words (`--carry`, `--leave`, each path's UTF-8 in hex), read now
    /// from the overrides file: `{"<old>" or "*": {"carry": [<path>, ...],
    /// "leave": [<path>, ...]}}`, each path relative to the durable tree.
    /// No file changes nothing; one that can't be read, or a path the
    /// reader would refuse, is an error.
    pub fn overrides(&self, old: &str) -> Result<Vec<String>, HostError> {
        let Some(path) = &self.overrides else {
            return Ok(Vec::new());
        };
        let refused = |why: String| HostError::Overrides(format!("{}: {why}", path.display()));
        let file = match std::fs::File::open(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            other => other.map_err(|error| refused(error.to_string()))?,
        };
        let mut bytes = Vec::new();
        file.take(OVERRIDES_FILE_MAX + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| refused(error.to_string()))?;
        if bytes.len() as u64 > OVERRIDES_FILE_MAX {
            return Err(refused(format!("over {OVERRIDES_FILE_MAX} bytes")));
        }
        let entries: BTreeMap<String, Overrides> =
            serde_json::from_slice(&bytes).map_err(|error| refused(error.to_string()))?;
        let mut words = Vec::new();
        for entry in ["*", old].iter().filter_map(|key| entries.get(*key)) {
            for (flag, paths) in [("--carry", &entry.carry), ("--leave", &entry.leave)] {
                for path in paths {
                    parse_path(path).map_err(|error| refused(format!("{path:?}: {error}")))?;
                    words.extend([flag.to_string(), hex::encode(path)]);
                }
            }
        }
        if words.len() / 2 > OVERRIDES_MAX {
            return Err(refused(format!(
                "more than {OVERRIDES_MAX} paths for {old}"
            )));
        }
        Ok(words)
    }

    /// `sizes <durable_state_id> [--carry|--leave <hex path>]...`: the host
    /// reader's summary from metadata alone (it opens no file), as the
    /// roster counts. It answers only at its end, so it has `idle` to do so.
    pub fn sizes(&self, binding: &Binding) -> Result<crate::contract::Sizes, HostError> {
        let mut words = vec!["sizes".to_string(), binding.durable_state_id.clone()];
        words.extend(self.overrides(&binding.old)?);
        let (ended, stdout, stderr) = run_within(&mut self.command(binding, &words)?, self.idle)
            .map_err(|error| HostError::NotRun(error.to_string()))?;
        if !ended.status.success() {
            return Err(failed(&ended.status, &stderr));
        }
        serde_json::from_slice(&stdout)
            .ok()
            .and_then(|summary| crate::sizes::sizes_of(&summary))
            .ok_or_else(|| HostError::NotRun("the host reader's sizes could not be read".into()))
    }

    /// `secrets <durable_state_id> <kind>,...`: the stopped agent's
    /// secrets of `kinds` it has, read into memory alone (its answer's
    /// bytes are cleared once read). A kind it lacks is left out.
    pub fn secrets(
        &self,
        binding: &Binding,
        kinds: &[SecretKind],
    ) -> Result<Vec<HandoffItem>, HostError> {
        let named: Vec<&str> = kinds.iter().filter_map(|kind| kind.as_str()).collect();
        if named.is_empty() {
            return Ok(Vec::new());
        }
        let words = vec![
            "secrets".to_string(),
            binding.durable_state_id.clone(),
            named.join(","),
        ];
        let (ended, stdout, stderr) = run_within(&mut self.command(binding, &words)?, self.idle)
            .map_err(|error| HostError::NotRun(error.to_string()))?;
        let stdout = Zeroizing::new(stdout);
        if !ended.status.success() {
            return Err(failed(&ended.status, &stderr));
        }
        #[derive(serde::Deserialize)]
        struct Read {
            items: Vec<HandoffItem>,
        }
        // serde's own error could quote a value: never kept
        serde_json::from_slice::<Read>(&stdout)
            .map(|read| read.items)
            .map_err(|_| HostError::NotRun("the host reader's secrets could not be read".into()))
    }

    /// `stream <durable_state_id> --stage <stage id> [--agent-key <hex>]
    /// [--person-key <hex>]... [--carry|--leave <hex path>]...`: the stage,
    /// as the host reader writes it, with every brain and each one's name
    /// file, whose owner the keys place, and the operator's `overrides`
    /// (read before the agent was stopped). The reader names its SimpleX
    /// seal by the stage's id, and carries SimpleX only with one. Its
    /// failure, or its exit before the end, is a read error at the stream's
    /// end.
    pub fn stream(
        &self,
        binding: &Binding,
        stage: &str,
        overrides: &[String],
    ) -> Result<HostStream, HostError> {
        let mut words = vec![
            "stream".to_string(),
            binding.durable_state_id.clone(),
            "--stage".to_string(),
            stage.to_string(),
        ];
        words.extend(owner_keys(binding));
        words.extend(overrides.iter().cloned());
        let mut watched = Watched::spawn(
            self.command(binding, &words)?
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
            self.idle,
        )
        .map_err(|error| HostError::NotRun(error.to_string()))?;
        let stdout = watched.stdout().expect("a piped stdout");
        let stderr = watched.stderr().expect("a piped stderr");
        let kept = Arc::new(Mutex::new(Vec::new()));
        let drain = drain_stderr(stderr, kept.clone());
        Ok(HostStream {
            watched,
            stdout,
            stderr: kept,
            drain: Some(drain),
            idle: self.idle,
            ended: false,
        })
    }
}

/// A host reader that exited unsuccessfully: its status and the last line
/// it wrote to stderr (the reader puts no secret in an error).
/// The keys that place each brain's owner, as the host reader takes them:
/// 64 hex digits each, the agent's own and at most `PERSON_KEYS_MAX` of
/// its person's. A key that isn't one is left out, and a brain it would
/// have placed is `unknown`.
fn owner_keys(binding: &Binding) -> Vec<String> {
    let hex = |key: &str| nostr::PublicKey::parse(key).ok().map(|key| key.to_hex());
    let mut words = Vec::new();
    if let Some(agent) = binding.agent_npub.as_deref().and_then(hex) {
        words.extend(["--agent-key".to_string(), agent]);
    }
    let mut people: Vec<String> = binding
        .person_keys
        .iter()
        .filter_map(|key| hex(key))
        .collect();
    people.sort();
    people.dedup();
    people.truncate(PERSON_KEYS_MAX);
    for person in people {
        words.extend(["--person-key".to_string(), person]);
    }
    words
}

fn failed(status: &std::process::ExitStatus, stderr: &[u8]) -> HostError {
    let stderr = String::from_utf8_lossy(stderr);
    let last = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    HostError::NotRun(short(&format!("{status}: {}", last.trim())))
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
/// Dropped before its end, the reader's whole group is killed.
pub struct HostStream {
    watched: Watched,
    stdout: ChildStdout,
    stderr: Arc<Mutex<Vec<u8>>>,
    drain: Option<JoinHandle<()>>,
    idle: Duration,
    ended: bool,
}

impl Read for HostStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.ended {
            return Ok(0);
        }
        // Watched only while the mover waits on the reader.
        self.watched.waiting(true);
        let read = self.stdout.read(buffer);
        if !matches!(read, Ok(0)) || buffer.is_empty() {
            self.watched.waiting(false);
            return read;
        }
        // Its output ended: the reader's end, and its group's.
        let exited = self.watched.exited();
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
        self.watched.waiting(false);
        exited?;
        let ended = self
            .watched
            .end(false)
            .map_err(|error| io::Error::other(format!("the host reader {error}")))?;
        if ended.fired {
            return Err(io::Error::other(format!(
                "the host reader sent nothing for {} s, so it was stopped",
                self.idle.as_secs()
            )));
        }
        if !ended.status.success() {
            let stderr = self.stderr.lock().expect("the kept stderr");
            let stderr = String::from_utf8_lossy(&stderr);
            let last = stderr
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            return Err(io::Error::other(short(&format!(
                "the host reader exited {}: {}",
                ended.status,
                last.trim()
            ))));
        }
        self.ended = true;
        Ok(0)
    }
}
