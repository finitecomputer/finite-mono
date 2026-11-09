//! Uploading one old agent's stage: the zip `finite-mover-host stream`
//! writes, into the stage bucket as
//!
//! ```text
//! <phase>/<old>/<attempt>/stage.zip       resumable multipart upload
//! <phase>/<old>/<attempt>/manifest.json   written last; its presence means staged
//! ```
//!
//! `<old>` is Core's opaque project id: no key holds a name or an email.
//!
//! - **Resume.** A stopped upload is found again by its key. Each part is
//!   re-read from the new stream and skipped when its MD5 matches what the
//!   bucket holds; a part that differs is uploaded again in place.
//! - **Stale attempts.** An attempt never writes once a later attempt of the
//!   same agent has begun; an earlier attempt's unfinished upload is aborted.
//! - **The refusal.** Before the upload completes, every name in the zip's
//!   central directory goes through the host reader's own classifier. One
//!   secret path refuses the whole stage: the upload is aborted, so nothing
//!   of it can be read, and no manifest is written.

use std::collections::BTreeMap;
use std::io::{self, Read};

use finite_mover_host::classify::{BrainScope, Class, EntryKind, classify};
use finite_mover_host::stage::SUMMARY_ENTRY;
use md5::{Digest as _, Md5};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::bucket::{Bucket, BucketError, Upload};
use crate::zip_tail::{self, Tail, TailError};

pub const MANIFEST_FORMAT: &str = "finite-stage/1";
/// The design's layout name for a finite.computer agent's tree.
pub const LAYOUT: &str = "finite-computer";
/// R2 and S3 refuse a part under 5 MiB, except the last.
pub const PART_BYTES_MIN: usize = 5 * 1024 * 1024;
/// Every part but the last has this size (R2 requires equal parts).
pub const PART_BYTES: usize = 64 * 1024 * 1024;
/// S3's limit; 640 GiB of 64 MiB parts.
pub const PARTS_MAX: u32 = 10_000;
/// How much of the stream's end is kept to read its central directory:
/// about 300,000 entries.
pub const TAIL_BYTES: usize = 64 * 1024 * 1024;
const OLD_MAX: usize = 128;

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

    fn agent_prefix(&self) -> String {
        format!("{}/{}/", self.phase.as_str(), self.old)
    }

    fn prefix(&self) -> String {
        format!("{}{}/", self.agent_prefix(), self.attempt)
    }

    pub fn zip_key(&self) -> String {
        format!("{}stage.zip", self.prefix())
    }

    pub fn manifest_key(&self) -> String {
        format!("{}manifest.json", self.prefix())
    }

    /// The attempt a key under this agent's prefix belongs to.
    fn attempt_of(&self, key: &str) -> Option<u32> {
        key.strip_prefix(&self.agent_prefix())?
            .split('/')
            .next()?
            .parse()
            .ok()
    }
}

/// `manifest.json`: what a staged attempt holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub phase: Phase,
    pub old: String,
    pub attempt: u32,
    pub layout: String,
    pub zip: ZipFacts,
    /// Staged files in the zip, its summary not counted.
    pub files: u64,
    /// The host reader's summary, verbatim: each class's files and bytes,
    /// and what stayed behind and why.
    pub summary: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZipFacts {
    pub key: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error("a stage target is a phase, an old agent id of [A-Za-z0-9_-] and an attempt from 1")]
    InvalidTarget,
    #[error("attempt {newer} of this agent has already begun; this attempt is stale")]
    StaleAttempt { newer: u32 },
    #[error(transparent)]
    Bucket(#[from] BucketError),
    #[error("reading the stage stream failed; the parts uploaded are kept for a resume: {0}")]
    Read(io::Error),
    #[error("the stage stream is empty")]
    Empty,
    #[error("the stage is larger than {PARTS_MAX} parts")]
    TooLarge,
    #[error("the stage is not a stage the mover can check: {0}")]
    Unreadable(#[from] TailError),
    #[error("the stage has no summary entry")]
    NoSummary,
    #[error("the stage was refused whole: {path} is a secret path")]
    Refused { path: String },
    #[error("the stage was refused whole: {path} is not a safe name")]
    UnsafeName { path: String },
}

/// The manifest of `target` if that attempt is already staged.
pub fn staged_manifest(
    bucket: &impl Bucket,
    target: &StageTarget,
) -> Result<Option<Manifest>, StageError> {
    let Some(bytes) = bucket.get(&target.manifest_key())? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|_| {
        StageError::Bucket(BucketError::Unreadable {
            operation: "manifest",
        })
    })
}

/// Upload `stream` as `target`'s stage and write its manifest last.
/// Returns the stored manifest unread if the attempt is already staged.
pub fn upload_stage(
    bucket: &impl Bucket,
    target: &StageTarget,
    stream: impl Read,
    part_bytes: usize,
) -> Result<Manifest, StageError> {
    assert!(part_bytes >= PART_BYTES_MIN, "parts are at least 5 MiB");
    refuse_if_stale(bucket, target)?;
    if let Some(manifest) = staged_manifest(bucket, target)? {
        return Ok(manifest);
    }
    let key = target.zip_key();
    let upload_id = resumable_upload(bucket, target, &key)?;
    let uploaded = match upload_parts(bucket, &key, &upload_id, stream, part_bytes) {
        Ok(uploaded) => uploaded,
        Err(error @ StageError::TooLarge) => {
            bucket.abort(&key, &upload_id)?;
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    let summary = match check(&uploaded.tail) {
        Ok(summary) => summary,
        Err(error @ (StageError::Refused { .. } | StageError::UnsafeName { .. })) => {
            // The parts may hold the refused file: drop them all.
            bucket.abort(&key, &upload_id)?;
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    bucket.complete(&key, &upload_id, &uploaded.parts)?;
    let manifest = Manifest {
        format: MANIFEST_FORMAT.to_string(),
        phase: target.phase,
        old: target.old.clone(),
        attempt: target.attempt,
        layout: LAYOUT.to_string(),
        zip: ZipFacts {
            key,
            bytes: uploaded.tail.total(),
            sha256: uploaded.sha256,
        },
        files: summary.files,
        summary: summary.json,
    };
    let body = serde_json::to_vec_pretty(&manifest).expect("a manifest serializes");
    bucket.put(&target.manifest_key(), &body, "application/json")?;
    Ok(manifest)
}

/// Refuse if a later attempt has begun; abort earlier unfinished uploads.
fn refuse_if_stale(bucket: &impl Bucket, target: &StageTarget) -> Result<(), StageError> {
    let prefix = target.agent_prefix();
    let objects = bucket.list_keys(&prefix)?;
    let uploads = bucket.uploads(&prefix)?;
    let newer = objects
        .iter()
        .chain(uploads.iter().map(|upload| &upload.key))
        .filter_map(|key| target.attempt_of(key))
        .filter(|attempt| *attempt > target.attempt)
        .max();
    if let Some(newer) = newer {
        return Err(StageError::StaleAttempt { newer });
    }
    for upload in uploads {
        if target
            .attempt_of(&upload.key)
            .is_some_and(|attempt| attempt < target.attempt)
        {
            bucket.abort(&upload.key, &upload.id)?;
        }
    }
    Ok(())
}

/// This attempt's unfinished upload, or a new one. Extra uploads of the
/// same key (from racing starts) are aborted.
fn resumable_upload(
    bucket: &impl Bucket,
    target: &StageTarget,
    key: &str,
) -> Result<String, StageError> {
    let mut mine: Vec<Upload> = bucket
        .uploads(&target.prefix())?
        .into_iter()
        .filter(|upload| upload.key == key)
        .collect();
    let Some(first) = mine.first().cloned() else {
        return Ok(bucket.create_upload(key)?);
    };
    for extra in mine.drain(1..) {
        bucket.abort(key, &extra.id)?;
    }
    Ok(first.id)
}

struct Uploaded {
    parts: Vec<(u32, String)>,
    tail: Tail,
    sha256: String,
}

fn upload_parts(
    bucket: &impl Bucket,
    key: &str,
    upload_id: &str,
    mut stream: impl Read,
    part_bytes: usize,
) -> Result<Uploaded, StageError> {
    let existing: BTreeMap<u32, _> = bucket
        .parts(key, upload_id)?
        .into_iter()
        .map(|part| (part.number, part))
        .collect();
    let mut tail = Tail::new(TAIL_BYTES);
    let mut hash = Sha256::new();
    let mut parts = Vec::new();
    let mut chunk = vec![0; part_bytes];
    let mut filled = read_full(&mut stream, &mut chunk).map_err(StageError::Read)?;
    if filled == 0 {
        return Err(StageError::Empty);
    }
    let mut number = 1u32;
    loop {
        if number > PARTS_MAX {
            return Err(StageError::TooLarge);
        }
        let body = &chunk[..filled];
        tail.push(body);
        hash.update(body);
        let md5 = hex::encode(Md5::digest(body));
        let etag = match existing.get(&number) {
            Some(part) if part.size == filled as u64 && part.etag.eq_ignore_ascii_case(&md5) => {
                part.etag.clone()
            }
            _ => bucket.upload_part(key, upload_id, number, body)?,
        };
        parts.push((number, etag));
        if filled < part_bytes {
            break;
        }
        filled = read_full(&mut stream, &mut chunk).map_err(StageError::Read)?;
        if filled == 0 {
            break;
        }
        number += 1;
    }
    Ok(Uploaded {
        parts,
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
    json: serde_json::Value,
}

/// The second check: every name through the host's classifier.
fn check(tail: &Tail) -> Result<CheckedSummary, StageError> {
    let entries = zip_tail::entries(tail)?;
    let mut summary = None;
    for entry in &entries {
        if entry.name == SUMMARY_ENTRY {
            summary = Some(entry);
            continue;
        }
        let parts: Vec<&str> = entry.name.split('/').collect();
        let unsafe_name = entry.name.starts_with('/')
            || parts.iter().any(|part| {
                part.is_empty() || *part == "." || *part == ".." || part.contains(['\\', '\0'])
            });
        if unsafe_name {
            return Err(StageError::UnsafeName {
                path: entry.name.clone(),
            });
        }
        let bytes: Vec<&[u8]> = parts.iter().map(|part| part.as_bytes()).collect();
        if classify(&bytes, EntryKind::File, BrainScope::None).class == Class::Secret {
            return Err(StageError::Refused {
                path: entry.name.clone(),
            });
        }
    }
    let summary = summary.ok_or(StageError::NoSummary)?;
    let json = serde_json::from_slice(zip_tail::data(tail, summary)?)
        .map_err(|_| StageError::NoSummary)?;
    Ok(CheckedSummary {
        files: entries.len() as u64 - 1,
        json,
    })
}

#[cfg(test)]
mod tests;
