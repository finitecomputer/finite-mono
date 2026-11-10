//! Where a stage goes: finite.chat's `Migrations`, through the mover's own
//! routes (fragment `docs/migrations.md`; FIN-463, "the mover uploads
//! through finite.chat"). The old side holds no bucket credential: R2 has
//! no write-only permission, so a token here could read every stage.
//! [`crate::migrations`] speaks these calls over HTTPS; tests use a fake.

use serde::{Deserialize, Serialize};

use crate::stage::Manifest;

/// One part finite.chat has taken.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub n: u32,
    pub bytes: u64,
    /// The part's SHA-256, hex.
    pub sha256: String,
    pub etag: String,
}

/// `GET …/parts`: the parts taken so far, for a resume.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parts {
    pub parts: Vec<Part>,
}

/// `POST …/complete` with `{manifest}`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Complete {
    pub manifest: Manifest,
}

/// Its answer: the stage is whole and its manifest written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completed {
    pub manifest_sha256: String,
    pub zip_bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    /// 403: this is not the stage the old agent's current command names.
    #[error("finite.chat did not command this stage")]
    NotCommanded,
    /// 409: the move is not staging (or archiving) now.
    #[error("finite.chat is not staging this agent now")]
    NotStaging,
    #[error("finite.chat answered {status} to {operation}")]
    Refused {
        operation: &'static str,
        status: u16,
    },
    #[error("finite.chat could not be reached for {operation}: {reason}")]
    Unreachable {
        operation: &'static str,
        reason: String,
    },
    #[error("finite.chat's answer to {operation} could not be read")]
    Unreadable { operation: &'static str },
}

/// The mover's three stage calls, all for one stage id
/// (`<old>.<phase>.<attempt>`).
pub trait StageSink {
    fn parts(&self, stage: &str) -> Result<Vec<Part>, SinkError>;
    /// Upload (or replace) part `n`. `sha256` is the body's, hex.
    fn put_part(&self, stage: &str, n: u32, body: &[u8], sha256: &str) -> Result<Part, SinkError>;
    /// Make the zip whole from parts 1 to n and write `manifest.json` last.
    /// Completing again gives the same answer.
    fn complete(&self, stage: &str, manifest: &Manifest) -> Result<Completed, SinkError>;
}
