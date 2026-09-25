//! Disposable, private display projection. Never consulted by authorization.
//!
//! The local exporter reads existing Core and hosted-account bindings. A missing,
//! stale, malformed, or ambiguous projection contributes no verified labels.
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

use finite_nostr::NostrPublicKey;
use serde::{Deserialize, Serialize};

pub const MAX_LABELS: usize = 4096;
pub const MAX_PROJECTION_BYTES: u64 = 2 * 1024 * 1024;
// Revalidation is demand-driven every five minutes. During a source outage,
// display evidence expires after one hour; sharing is checked on every read.
pub const MAX_LABEL_AGE_SECONDS: u64 = 3600;
pub const LABEL_REFRESH_INTERVAL_SECONDS: u64 = 300;

/// Best-effort wakeup only. No identity, Brain ID or credentials cross this
/// socket; the confined worker discovers its targets from authoritative state.
/// Call only after authorizing the metadata request, outside the store lock.
#[cfg(unix)]
pub(crate) fn request_refresh(path: &Path) {
    use std::os::unix::net::UnixDatagram;
    let Ok(socket) = UnixDatagram::unbound() else {
        return;
    };
    if socket.set_nonblocking(true).is_ok() {
        // A missing worker or a full socket queue must never delay Brain reads.
        let _ = socket.send_to(b"refresh", path);
    }
}

#[cfg(not(unix))]
pub(crate) fn request_refresh(_path: &Path) {}

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

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabelBinding {
    pub npub: String,
    /// Stable source record identity. Used to reject conflicting owners, never
    /// returned in Brain metadata (in particular, never expose WorkOS subjects).
    pub source_id: String,
    pub label: PrincipalLabel,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabelProjection {
    pub version: u32,
    pub generated_at: u64,
    pub bindings: Vec<LabelBinding>,
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
    pub fn labels(&self, now: u64) -> Option<BTreeMap<String, PrincipalLabel>> {
        if self.version != 1
            || self.bindings.len() > MAX_LABELS
            || self.generated_at > now
            || now - self.generated_at > MAX_LABEL_AGE_SECONDS
        {
            return None;
        }
        let mut candidates: BTreeMap<String, Option<&LabelBinding>> = BTreeMap::new();
        for binding in &self.bindings {
            let canonical = NostrPublicKey::parse(&binding.npub).ok()?.to_npub().ok()?;
            if canonical != binding.npub
                || binding.label.observed_at != self.generated_at
                || !matches!(
                    (binding.label.kind, binding.label.source),
                    (PrincipalKind::Human, LabelSource::HostedAccount)
                        | (PrincipalKind::Agent, LabelSource::ManagedAgent)
                )
            {
                return None;
            }
            if !valid_label_name(&binding.label.name) || !valid_label_name(&binding.source_id) {
                // An unusable source still claims this key. Reject this key,
                // including any other candidate, without hiding unrelated labels.
                candidates.insert(binding.npub.clone(), None);
                continue;
            }
            candidates
                .entry(binding.npub.clone())
                .and_modify(|prior| {
                    if *prior != Some(binding) {
                        *prior = None;
                    }
                })
                .or_insert(Some(binding));
        }
        Some(
            candidates
                .into_iter()
                .filter_map(|(npub, binding)| binding.map(|binding| (npub, binding.label.clone())))
                .collect(),
        )
    }
}

pub(crate) fn read_labels(path: &Path, now: u64) -> Option<BTreeMap<String, PrincipalLabel>> {
    let file = File::open(path).ok()?;
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
    serde_json::from_slice::<LabelProjection>(&bytes)
        .ok()?
        .labels(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(key: u8, name: &str, kind: PrincipalKind) -> LabelBinding {
        let npub = NostrPublicKey::parse(&format!("{key:02x}").repeat(32))
            .unwrap()
            .to_npub()
            .unwrap();
        LabelBinding {
            npub,
            source_id: format!("source-{key}"),
            label: PrincipalLabel {
                name: name.to_owned(),
                kind,
                observed_at: 100,
                source: match kind {
                    PrincipalKind::Human => LabelSource::HostedAccount,
                    PrincipalKind::Agent => LabelSource::ManagedAgent,
                },
            },
        }
    }

    #[test]
    fn labels_distinguish_same_name_and_reject_conflicting_owners() {
        let human = binding(1, "Alex", PrincipalKind::Human);
        let agent = binding(2, "Alex", PrincipalKind::Agent);
        let mut projection = LabelProjection {
            version: 1,
            generated_at: 100,
            bindings: vec![human.clone(), agent.clone(), human.clone()],
        };
        let labels = projection.labels(101).unwrap();
        assert_eq!(labels[&human.npub].display(), "Alex (human)");
        assert_eq!(labels[&agent.npub].display(), "Alex (agent)");
        let mut conflict = human.clone();
        conflict.source_id = "different-owner".to_owned();
        projection.bindings.extend([conflict, human]);
        let labels = projection.labels(101).unwrap();
        assert_eq!(labels.len(), 1);
        assert!(labels.contains_key(&agent.npub));
    }

    #[test]
    fn projection_fails_closed_on_staleness_bad_types_and_oversized_input() {
        let valid = LabelProjection {
            version: 1,
            generated_at: 100,
            bindings: vec![binding(1, "Alex", PrincipalKind::Human)],
        };
        assert!(valid.labels(99).is_none());
        assert!(valid.labels(100 + MAX_LABEL_AGE_SECONDS + 1).is_none());
        assert!(valid.labels(100 + MAX_LABEL_AGE_SECONDS).is_some());
        let mut invalid = valid.clone();
        invalid.bindings[0].label.kind = PrincipalKind::Agent;
        assert!(invalid.labels(100).is_none());
        invalid = valid.clone();
        invalid.bindings[0].label.name = "Alex\nadmin".to_owned();
        invalid.bindings.push(valid.bindings[0].clone());
        invalid
            .bindings
            .push(binding(2, "Other", PrincipalKind::Agent));
        let labels = invalid.labels(100).unwrap();
        assert!(!labels.contains_key(&valid.bindings[0].npub));
        assert_eq!(labels.len(), 1);
        invalid = valid.clone();
        invalid.bindings[0].npub = "npub-invalid".to_owned();
        assert!(invalid.labels(100).is_none());
        invalid = valid.clone();
        invalid.bindings[0].label.observed_at = 99;
        assert!(invalid.labels(100).is_none());
        invalid = valid.clone();
        invalid.bindings = vec![valid.bindings[0].clone(); MAX_LABELS + 1];
        assert!(invalid.labels(100).is_none());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("labels.json");
        assert!(read_labels(&path, 100).is_none());
        std::fs::write(&path, b"{broken}").unwrap();
        assert!(read_labels(&path, 100).is_none());
        std::fs::write(&path, vec![b' '; MAX_PROJECTION_BYTES as usize + 1]).unwrap();
        assert!(read_labels(&path, 100).is_none());
        std::fs::write(&path, serde_json::to_vec(&valid).unwrap()).unwrap();
        assert_eq!(read_labels(&path, 100).unwrap().len(), 1);
    }
}
