//! Disposable, private display projection. Never consulted by authorization.
//!
//! The local exporter reads existing Core and hosted-account bindings. A missing,
//! stale, malformed, or ambiguous projection contributes no verified labels.
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

use finite_nostr::NostrPublicKey;
use serde::{Deserialize, Serialize};

// One snapshot covers one Brain's supported membership and guest envelope.
// Admins are members; owner and Personal Agent can be separate Principals.
pub const MAX_LABELS: usize = finite_brain_core::BRAIN_CAPACITY_ENVELOPE.members
    + finite_brain_core::BRAIN_CAPACITY_ENVELOPE.folder_access_entries
    + 2;
pub const MAX_PROJECTION_BYTES: u64 = MAX_LABELS as u64 * 1024 + 1024;
pub const MAX_LABEL_AGE_SECONDS: u64 = 3600;
pub const LABEL_REFRESH_INTERVAL_SECONDS: u64 = 300;

/// Authorized callers request one Brain, never arbitrary identity lookups.
/// The worker independently reads that Brain's roster from authoritative state.
#[cfg(unix)]
pub(crate) fn request_refresh(path: &Path, brain_id: &finite_brain_core::BrainId) {
    use std::os::unix::net::UnixDatagram;
    let Ok(socket) = UnixDatagram::unbound() else {
        return;
    };
    if socket.set_nonblocking(true).is_ok() {
        let _ = socket.send_to(brain_id.as_str().as_bytes(), path);
    }
}

#[cfg(not(unix))]
pub(crate) fn request_refresh(_path: &Path, _brain_id: &finite_brain_core::BrainId) {}

pub fn projection_path(
    directory: &Path,
    brain_id: &finite_brain_core::BrainId,
) -> std::path::PathBuf {
    // BrainId permits only ASCII alphanumeric, '-' and '_', at most 128 bytes.
    directory.join(format!("{}.json", brain_id.as_str()))
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelSource {
    HostedAccount,
    ManagedAgent,
}

/// Private display metadata, independent of public NIP-05 verification.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrincipalLabel {
    pub name: String,
    pub kind: PrincipalKind,
    pub source: LabelSource,
    pub observed_at: u64,
}

impl PrincipalLabel {
    pub fn display(&self) -> String {
        let kind = match self.kind {
            PrincipalKind::Human => "human",
            PrincipalKind::Agent => "agent",
        };
        format!("{} ({kind})", self.name)
    }
}

/// Fully resolved display data; source record identifiers stay in the worker.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabelProjection {
    pub version: u32,
    pub brain_id: String,
    pub generated_at: u64,
    pub labels: BTreeMap<String, PrincipalLabel>,
}

pub fn valid_label_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 254
        && value.trim() == value
        && !value.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}

impl LabelProjection {
    pub fn valid(&self, brain_id: &finite_brain_core::BrainId, now: u64) -> bool {
        if self.version != 1
            || self.brain_id != brain_id.as_str()
            || self.labels.len() > MAX_LABELS
            || self.generated_at > now
            || now - self.generated_at > MAX_LABEL_AGE_SECONDS
        {
            return false;
        }
        self.labels.iter().all(|(npub, label)| {
            NostrPublicKey::parse(npub)
                .ok()
                .and_then(|key| key.to_npub().ok())
                .as_ref()
                == Some(npub)
                && valid_label_name(&label.name)
                && label.observed_at == self.generated_at
                && matches!(
                    (label.kind, label.source),
                    (PrincipalKind::Human, LabelSource::HostedAccount)
                        | (PrincipalKind::Agent, LabelSource::ManagedAgent)
                )
        })
    }
}

pub(crate) fn read_labels(
    directory: &Path,
    brain_id: &finite_brain_core::BrainId,
    now: u64,
) -> Option<BTreeMap<String, PrincipalLabel>> {
    let file = File::open(projection_path(directory, brain_id)).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_PROJECTION_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_PROJECTION_BYTES {
        return None;
    }
    let projection: LabelProjection = serde_json::from_slice(&bytes).ok()?;
    if !projection.valid(brain_id, now) {
        return None;
    }
    Some(projection.labels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use finite_brain_core::BrainId;

    #[test]
    fn snapshots_are_brain_scoped_bounded_and_expire() {
        let brain = BrainId::new("acme").unwrap();
        let other = BrainId::new("other").unwrap();
        let npub = NostrPublicKey::parse(&"01".repeat(32))
            .unwrap()
            .to_npub()
            .unwrap();
        let valid = LabelProjection {
            version: 1,
            brain_id: brain.to_string(),
            generated_at: 100,
            labels: BTreeMap::from([(
                npub.clone(),
                PrincipalLabel {
                    name: "Alex".into(),
                    kind: PrincipalKind::Human,
                    source: LabelSource::HostedAccount,
                    observed_at: 100,
                },
            )]),
        };
        assert!(valid.valid(&brain, 100));
        assert!(!valid.valid(&other, 100));
        assert!(!valid.valid(&brain, 99));
        assert!(valid.valid(&brain, 100 + MAX_LABEL_AGE_SECONDS));
        assert!(!valid.valid(&brain, 101 + MAX_LABEL_AGE_SECONDS));
        let mut invalid = valid.clone();
        invalid.labels.get_mut(&npub).unwrap().kind = PrincipalKind::Agent;
        assert!(!invalid.valid(&brain, 100));
        invalid = valid.clone();
        invalid.labels.get_mut(&npub).unwrap().name = "Alex\nadmin".into();
        assert!(!invalid.valid(&brain, 100));
        invalid = valid.clone();
        invalid.labels.get_mut(&npub).unwrap().observed_at = 99;
        assert!(!invalid.valid(&brain, 100));
        let dir = tempfile::tempdir().unwrap();
        let path = projection_path(dir.path(), &brain);
        assert!(read_labels(dir.path(), &brain, 100).is_none());
        std::fs::write(&path, b"{broken}").unwrap();
        assert!(read_labels(dir.path(), &brain, 100).is_none());
        std::fs::write(&path, vec![b' '; MAX_PROJECTION_BYTES as usize + 1]).unwrap();
        assert!(read_labels(dir.path(), &brain, 100).is_none());
        std::fs::write(&path, serde_json::to_vec(&valid).unwrap()).unwrap();
        assert_eq!(read_labels(dir.path(), &brain, 100).unwrap().len(), 1);
        assert!(read_labels(dir.path(), &other, 100).is_none());
    }
}
