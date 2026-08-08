//! `/data/shell/state.json`: the shell's durable record of staged, bad, and
//! flipped generations. Writes are atomic (tempfile + rename in the shell
//! dir). The state file is bookkeeping, not truth: the `current`/`previous`
//! symlinks are the truth, and boot reconciles state against them.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::ShellError;

pub const SHELL_STATE_SCHEMA: &str = "finite_shell_state.v1";

/// A staged, fully verified generation waiting for `flip`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedGeneration {
    pub version_label: String,
    pub artifact_id: String,
    pub tree_digest: String,
    pub tarball_sha256: String,
    pub min_shell_version: String,
    pub staged_at: String,
}

/// A generation that failed the flip health gate. Never re-stageable without
/// `force`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BadGeneration {
    pub version_label: String,
    pub reason: String,
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlipOutcome {
    InProgress,
    Success,
    /// The transition failed and the previous generation was verifiably
    /// restored (symlinks back, old agentd running again).
    RolledBack,
    /// The transition failed AND restoration failed or could not be
    /// verified: the machine may be running the unverified candidate or
    /// nothing at all. Surfaced prominently in `/healthz` (ready:false).
    FailedOpen,
    /// A flip was in progress when the shell died; boot reconciled from the
    /// symlinks.
    Interrupted,
}

/// One flip (or rollback) transition. `prior_previous` remembers what
/// `previous` pointed at before the flip so a health-gate rollback can
/// restore it exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlipRecord {
    pub from: Option<String>,
    pub to: String,
    pub at: String,
    pub outcome: FlipOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_previous: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The seed generation unpacked on first boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedRecord {
    pub version_label: String,
    pub tree_digest: String,
    pub recorded_at: String,
}

/// A crash-looping agentd (>= N restarts inside the window). The shell keeps
/// restarting; this is surfaced in `/healthz` and `status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashLoopRecord {
    pub restarts: usize,
    pub window_secs: u64,
    pub recorded_at: String,
}

/// What one autonomous channel-poll tick decided (plan M4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollAction {
    /// Head is current, absent, or a transition was already in progress.
    None,
    /// The head was staged but its flip did not start.
    Staged,
    /// The head was (or already had been) staged and its flip was started.
    FlipStarted,
    /// The head is on the bad list (a previous flip's health gate failed).
    SkippedBad,
    /// The head's payload requires a newer shell.
    SkippedMinShell,
    Error,
}

/// The most recent channel-poll outcome, recorded per tick and surfaced in
/// `/healthz`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollRecord {
    pub at: String,
    pub channel: String,
    pub head_version: Option<String>,
    pub action: PollAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellState {
    pub schema: String,
    #[serde(default)]
    pub staged: Option<StagedGeneration>,
    #[serde(default)]
    pub bad: Vec<BadGeneration>,
    #[serde(default)]
    pub last_flip: Option<FlipRecord>,
    #[serde(default)]
    pub seed: Option<SeedRecord>,
    #[serde(default)]
    pub agentd_crash_loop: Option<CrashLoopRecord>,
    #[serde(default)]
    pub last_poll: Option<PollRecord>,
    /// Anti-replay floor: the newest `generated_at` of any service directory
    /// this shell has acted on. The poller refuses documents strictly older
    /// than it (equal is fine — an idempotent re-serve is not a replay).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory_floor: Option<String>,
    /// The verified tree digest of the generation `current` points at,
    /// recorded at seed time and at every flip commit (from the staged
    /// record). Cleared when a transition retargets `current` at a
    /// generation whose digest is not on record (rollback/restore journal
    /// only labels): absent evidence is served as null, never guessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_digest: Option<String>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            schema: SHELL_STATE_SCHEMA.to_owned(),
            staged: None,
            bad: Vec::new(),
            last_flip: None,
            seed: None,
            agentd_crash_loop: None,
            last_poll: None,
            directory_floor: None,
            current_digest: None,
        }
    }
}

impl ShellState {
    pub fn is_bad(&self, version: &str) -> bool {
        self.bad.iter().any(|bad| bad.version_label == version)
    }
}

/// RFC 3339 UTC now, the one timestamp format in shell records.
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

/// Shared handle over the state file: every mutation persists atomically
/// before the lock is released, so readers never observe a state the disk
/// does not.
#[derive(Clone)]
pub struct SharedState {
    inner: Arc<Mutex<StateFile>>,
}

struct StateFile {
    path: PathBuf,
    state: ShellState,
}

impl SharedState {
    /// Load existing state (unreadable/corrupt state starts fresh: the
    /// symlinks are the truth and boot reconciles).
    pub fn load(path: &Path) -> Self {
        let state = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ShellState>(&bytes).ok())
            .filter(|state| state.schema == SHELL_STATE_SCHEMA)
            .unwrap_or_default();
        Self {
            inner: Arc::new(Mutex::new(StateFile {
                path: path.to_path_buf(),
                state,
            })),
        }
    }

    pub fn snapshot(&self) -> ShellState {
        self.inner
            .lock()
            .expect("state lock poisoned")
            .state
            .clone()
    }

    /// Mutate and persist atomically.
    pub fn update<R>(&self, mutate: impl FnOnce(&mut ShellState) -> R) -> Result<R, ShellError> {
        let mut file = self.inner.lock().expect("state lock poisoned");
        let result = mutate(&mut file.state);
        let path = file.path.clone();
        write_atomic_json(&path, &file.state)?;
        Ok(result)
    }
}

/// Atomic JSON write: tempfile in the destination directory, fsync, rename.
pub fn write_atomic_json(path: &Path, value: &impl Serialize) -> Result<(), ShellError> {
    let parent = path
        .parent()
        .ok_or_else(|| ShellError::Io(std::io::Error::other("state path has no parent")))?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| ShellError::Io(error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrips_and_updates_persist_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shell/state.json");
        let state = SharedState::load(&path);
        assert_eq!(state.snapshot(), ShellState::default());

        state
            .update(|state| {
                state.staged = Some(StagedGeneration {
                    version_label: "v2".to_owned(),
                    artifact_id: "payload".to_owned(),
                    tree_digest: "a".repeat(64),
                    tarball_sha256: "b".repeat(64),
                    min_shell_version: "0.1.0".to_owned(),
                    staged_at: "2026-08-06T00:00:00Z".to_owned(),
                });
                state.bad.push(BadGeneration {
                    version_label: "v3".to_owned(),
                    reason: "health gate failed".to_owned(),
                    at: "2026-08-06T00:00:00Z".to_owned(),
                });
            })
            .unwrap();

        let reloaded = SharedState::load(&path).snapshot();
        assert_eq!(reloaded.staged.as_ref().unwrap().version_label, "v2");
        assert!(reloaded.is_bad("v3"));
        assert!(!reloaded.is_bad("v2"));
    }

    #[test]
    fn a_corrupt_state_file_starts_fresh_instead_of_wedging_boot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, b"{not json").unwrap();
        assert_eq!(SharedState::load(&path).snapshot(), ShellState::default());
        fs::write(&path, br#"{"schema":"other.v9"}"#).unwrap();
        assert_eq!(SharedState::load(&path).snapshot(), ShellState::default());
    }
}
