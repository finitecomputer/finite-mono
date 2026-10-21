//! Exact-Brain operator attribution. These records describe existing roster
//! keys; they are never account associations or access authority.
mod manifest;
mod wire;
pub use manifest::*;
pub use wire::*;

use super::*;

pub const DESCRIPTIONS_VERSION_V3: &str = "finite-core-brain-descriptions-v3";
pub const SCOPED_MANIFEST_VERSION: &str = "finite-core-brain-scoped-records-v1";
pub const OPERATOR_SOURCE: &str = "operatorScopedRecord";
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

pub fn valid_evidence_time(value: &str) -> bool {
    (20..=40).contains(&value.len())
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(17..19) != Some(b"60")
        && value.ends_with('Z')
        && crate::parse_time(value).is_ok_and(|time| time.offset().is_utc() && time.year() >= 1)
}

pub fn valid_annotation_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.trim() == value
        && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

pub fn sha256(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests;
