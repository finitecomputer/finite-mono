//! Uploading one old agent's stage: the zip `finite-mover-host stream`
//! writes, sent in parts to finite.chat, which writes it to its stage
//! bucket ([`crate::sink`]). The stage's id is `<old>.<phase>.<attempt>`;
//! `<old>` is Core's opaque project id, so no id holds a name or an email.
//!
//! - **The refusal, at the source.** The stream is read as it goes
//!   ([`crate::zip_scan`]): every entry's name goes through the host
//!   reader's own classifier before the part holding it is sent. One secret
//!   path refuses the whole stage: none of that file's bytes, nor anything
//!   after it, leaves the mover. Parts already sent hold only entries that
//!   passed (and at most the start of the refused one's header), and the
//!   stage is never completed.
//! - **Resume.** `GET …/parts` lists what finite.chat holds. Each part is
//!   re-read from the new stream and skipped when its size and SHA-256
//!   match; a part that differs is sent again in its place.
//! - **Stale attempts.** finite.chat takes parts only for the stage its
//!   current command names, while it stages: any other is refused
//!   (403 or 409), and the run stops.
//! - **One run at a time.** A run holds a lock on its old agent from its
//!   first call to its complete, so two runs never interleave their parts.
//! - **The end.** The central directory must list exactly the entries
//!   streamed. Then `complete` makes the zip whole and writes
//!   `manifest.json` last.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use finite_mover_host::stage::SUMMARY_ENTRY;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::sink::{Completed, SinkError, StageSink};
use crate::zip_scan::{ScanError, Scanned, Scanner};
use crate::zip_tail::{self, Tail, TailError};

pub const MANIFEST_FORMAT: &str = "finite-stage/1";
/// The design's layout name for a finite.computer agent's tree.
pub const LAYOUT: &str = "finite-computer";
/// R2's least part, every part but the last (fragment's `PART_MIN_BYTES`).
pub const PART_BYTES_MIN: usize = 5 * 1024 * 1024;
/// Every part but the last has this size (R2 requires equal parts). It is
/// fragment's `PART_MAX_BYTES`, under a request's body limit.
pub const PART_BYTES: usize = 64 * 1024 * 1024;
/// R2's limit; 640 GiB of 64 MiB parts (fragment's `PARTS_MAX`).
pub const PARTS_MAX: u32 = 10_000;
/// How much of the stream's end is kept to read its central directory:
/// about 300,000 entries.
pub const TAIL_BYTES: usize = 64 * 1024 * 1024;
/// The longest old agent id (fragment's `OLD_ID_MAX_BYTES`; Core's
/// project ids are 28 bytes).
const OLD_MAX: usize = 64;
/// `manifest.json` is read whole: at most this, and at most this many
/// `left` lines (fragment's `MANIFEST_MAX_BYTES`, `MANIFEST_LEFT_MAX`).
pub const MANIFEST_MAX_BYTES: usize = 256 * 1024;
pub const MANIFEST_LEFT_MAX: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// The person said yes.
    P1,
    /// Archived at the cutoff.
    P2,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::P1 => "p1",
            Phase::P2 => "p2",
        }
    }
}

/// Which stage: one attempt at one old agent, in one phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageTarget {
    phase: Phase,
    old: String,
    attempt: u32,
}

impl StageTarget {
    pub fn new(phase: Phase, old: &str, attempt: u32) -> Result<Self, StageError> {
        let valid = !old.is_empty()
            && old.len() <= OLD_MAX
            && old
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            && attempt >= 1;
        if !valid {
            return Err(StageError::InvalidTarget);
        }
        Ok(Self {
            phase,
            old: old.to_string(),
            attempt,
        })
    }

    /// fragment's `stage_id`: `<old>.<phase>.<attempt>`.
    pub fn stage_id(&self) -> String {
        format!("{}.{}.{}", self.old, self.phase.as_str(), self.attempt)
    }
}

/// `manifest.json`: what a staged attempt holds. The shape is fragment's
/// `fragment_proto::migrations::Manifest`, which the platform and the
/// importer read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// `finite-stage/1`.
    pub format: String,
    pub old: String,
    pub phase: Phase,
    pub attempt: u32,
    /// How the zip's tree is laid out, for the importer.
    pub layout: String,
    pub zip_bytes: u64,
    /// The zip's SHA-256, hex.
    pub zip_sha256: String,
    /// The staged files' bytes, uncompressed, and how many there are: the
    /// zip's entries but its summary.
    pub bytes: u64,
    pub files: u64,
    /// What stayed behind, largest first, at most `MANIFEST_LEFT_MAX`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<LeftBehind>,
    /// Files left behind past the list, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_more: Option<u64>,
}

/// One path that stayed behind: a file, or a directory that stayed whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeftBehind {
    pub path: String,
    pub bytes: u64,
    /// The classifier's class: `secret`, `cache` or `runtime`.
    pub why: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error("a stage target is a phase, an old agent id of [A-Za-z0-9_-] and an attempt from 1")]
    InvalidTarget,
    /// A refusal of this stage (403, 409) or a failed call: the parts
    /// finite.chat took are kept for a resume.
    #[error(transparent)]
    Sink(#[from] SinkError),
    #[error("reading the stage stream failed; the parts sent are kept for a resume: {0}")]
    Read(io::Error),
    #[error("the stage stream is empty")]
    Empty,
    #[error("the stage is larger than {PARTS_MAX} parts")]
    TooLarge,
    #[error("the stage was refused whole: {path} is a secret path")]
    Refused { path: String },
    #[error("the stage was refused whole: {path} is not a safe name")]
    UnsafeName { path: String },
    #[error("the stage is not a stage the mover can check: {0}")]
    Malformed(ScanError),
    #[error("the stage is not a stage the mover can check: {0}")]
    Unreadable(#[from] TailError),
    #[error("the stage's central directory does not list the entries streamed")]
    DirectoryMismatch,
    #[error("the stage has no summary entry")]
    NoSummary,
    #[error("finite.chat completed {got} bytes; the stream was {sent}")]
    CompletedMismatch { sent: u64, got: u64 },
    #[error("another run is staging this old agent")]
    Busy,
    #[error("the stage lock could not be taken: {0}")]
    Lock(io::Error),
}

impl From<ScanError> for StageError {
    fn from(error: ScanError) -> Self {
        match error {
            ScanError::Secret(path) => StageError::Refused { path },
            ScanError::Unsafe(path) => StageError::UnsafeName { path },
            other => StageError::Malformed(other),
        }
    }
}

/// Send `stream` to finite.chat as `target`'s stage, in parts of
/// `part_bytes`, and complete it with its manifest. Returns the manifest
/// and finite.chat's answer, whose `manifest_sha256` the `staged` report
/// names.
///
/// `locks` is a directory only the mover writes, on the one host it runs
/// on: a run holds `<phase>-<old>.lock` there until it returns, and a
/// second run of the same old agent meanwhile is `Busy`.
pub fn upload_stage(
    sink: &impl StageSink,
    target: &StageTarget,
    stream: impl Read,
    part_bytes: usize,
    locks: &Path,
) -> Result<(Manifest, Completed), StageError> {
    assert!(
        (PART_BYTES_MIN..=PART_BYTES).contains(&part_bytes),
        "parts are 5 to 64 MiB"
    );
    let _held = lock(locks, target)?;
    let stage = target.stage_id();
    let sent = send_parts(sink, &stage, stream, part_bytes)?;
    let summary = check(&sent)?;
    let mut manifest = Manifest {
        format: MANIFEST_FORMAT.to_string(),
        old: target.old.clone(),
        phase: target.phase,
        attempt: target.attempt,
        layout: LAYOUT.to_string(),
        zip_bytes: sent.tail.total(),
        zip_sha256: sent.sha256.clone(),
        bytes: summary.bytes,
        files: summary.files,
        left: summary.left,
        left_more: (summary.left_more > 0).then_some(summary.left_more),
    };
    fit_manifest(&mut manifest, summary.left_files);
    let completed = sink.complete(&stage, &manifest)?;
    if completed.zip_bytes != manifest.zip_bytes {
        return Err(StageError::CompletedMismatch {
            sent: manifest.zip_bytes,
            got: completed.zip_bytes,
        });
    }
    Ok((manifest, completed))
}

/// Keep the manifest's JSON within `MANIFEST_MAX_BYTES`: the smallest
/// `left` lines move into `left_more` until it fits. `left_files` is each
/// line's file count.
fn fit_manifest(manifest: &mut Manifest, mut left_files: Vec<u64>) {
    while serde_json::to_vec(manifest)
        .expect("a manifest serializes")
        .len()
        > MANIFEST_MAX_BYTES
    {
        if manifest.left.pop().is_none() {
            return;
        }
        let more = left_files.pop().unwrap_or(1);
        manifest.left_more = Some(manifest.left_more.unwrap_or(0) + more);
    }
}

/// The old agent's lock, held while the returned file is open.
fn lock(locks: &Path, target: &StageTarget) -> Result<File, StageError> {
    let path = locks.join(format!("{}-{}.lock", target.phase.as_str(), target.old));
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(StageError::Lock)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(StageError::Busy),
        Err(std::fs::TryLockError::Error(error)) => Err(StageError::Lock(error)),
    }
}

struct Sent {
    scanned: Vec<Scanned>,
    tail: Tail,
    sha256: String,
}

fn send_parts(
    sink: &impl StageSink,
    stage: &str,
    mut stream: impl Read,
    part_bytes: usize,
) -> Result<Sent, StageError> {
    let held: BTreeMap<u32, _> = sink
        .parts(stage)?
        .into_iter()
        .map(|part| (part.n, part))
        .collect();
    let mut scanner = Scanner::default();
    let mut tail = Tail::new(TAIL_BYTES);
    let mut hash = Sha256::new();
    let mut chunk = vec![0; part_bytes];
    let mut filled = read_full(&mut stream, &mut chunk).map_err(StageError::Read)?;
    if filled == 0 {
        return Err(StageError::Empty);
    }
    let mut n = 1u32;
    loop {
        if n > PARTS_MAX {
            return Err(StageError::TooLarge);
        }
        let body = &chunk[..filled];
        // Every name this part holds or begins is checked before it goes.
        scanner.feed(body)?;
        tail.push(body);
        hash.update(body);
        let sha256 = hex::encode(Sha256::digest(body));
        let same = held.get(&n).is_some_and(|part| {
            part.bytes == filled as u64 && part.sha256.eq_ignore_ascii_case(&sha256)
        });
        if !same {
            let taken = sink.put_part(stage, n, body, &sha256)?;
            if taken.n != n
                || taken.bytes != filled as u64
                || !taken.sha256.eq_ignore_ascii_case(&sha256)
            {
                return Err(SinkError::Unreadable {
                    operation: "put part",
                }
                .into());
            }
        }
        if filled < part_bytes {
            break;
        }
        filled = read_full(&mut stream, &mut chunk).map_err(StageError::Read)?;
        if filled == 0 {
            break;
        }
        n += 1;
    }
    let (scanned, _directory) = scanner.finish()?;
    Ok(Sent {
        scanned,
        tail,
        sha256: hex::encode(hash.finalize()),
    })
}

fn read_full(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

struct CheckedSummary {
    files: u64,
    bytes: u64,
    left: Vec<LeftBehind>,
    left_files: Vec<u64>,
    left_more: u64,
}

/// The end: the central directory lists exactly what streamed past (every
/// name of which was checked before it went), and the summary is read.
fn check(sent: &Sent) -> Result<CheckedSummary, StageError> {
    let entries = zip_tail::entries(&sent.tail)?;
    let listed: Vec<Scanned> = entries
        .iter()
        .map(|entry| Scanned {
            name: entry.name.clone(),
            offset: entry.offset,
            size: entry.size,
            crc: entry.crc,
        })
        .collect();
    if listed != sent.scanned {
        return Err(StageError::DirectoryMismatch);
    }
    let summary = entries
        .iter()
        .find(|entry| entry.name == SUMMARY_ENTRY)
        .ok_or(StageError::NoSummary)?;
    let bytes = entries
        .iter()
        .filter(|entry| entry.name != SUMMARY_ENTRY)
        .map(|entry| entry.size)
        .sum();
    let host: HostSummary = serde_json::from_slice(zip_tail::data(&sent.tail, summary)?)
        .map_err(|_| StageError::NoSummary)?;
    let listed = host.left.len().min(MANIFEST_LEFT_MAX);
    let left_more = host.left_more.files
        + host.left[listed..]
            .iter()
            .map(|line| line.files)
            .sum::<u64>();
    let (left, left_files) = host
        .left
        .into_iter()
        .take(listed)
        .map(|line| {
            let behind = LeftBehind {
                path: line.path,
                bytes: line.bytes,
                why: line.class,
            };
            (behind, line.files)
        })
        .unzip();
    Ok(CheckedSummary {
        files: entries.len() as u64 - 1,
        bytes,
        left,
        left_files,
        left_more,
    })
}

/// The parts of `finite-mover-host`'s summary the manifest carries.
#[derive(Deserialize)]
struct HostSummary {
    left: Vec<HostLeft>,
    left_more: HostTally,
}

#[derive(Deserialize)]
struct HostLeft {
    path: String,
    class: String,
    files: u64,
    bytes: u64,
}

#[derive(Deserialize)]
struct HostTally {
    files: u64,
}

#[cfg(test)]
mod tests;
