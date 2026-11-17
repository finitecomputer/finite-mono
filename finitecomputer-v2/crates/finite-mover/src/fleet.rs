//! What the mover knows of the old fleet: the roster it pushes, and each
//! old agent's exact binding, which every command needs: Core's runtime,
//! the runner host and machine it runs on, and its durable tree there.
//!
//! M1 reads both from Core's database (a read-only role). A file holds
//! them too, for tests and for a hand-run roster.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::contract::Roster;

/// One old agent's exact binding, as Core's `*-exact` commands check it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// Core's project id: the old agent's id on fragment's wire.
    pub old: String,
    pub agent_runtime_id: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    /// `<work root>/kata/<durable_state_id>` on its runner host.
    pub durable_state_id: String,
    /// The npub its runtime signs with, as Core pinned it: the key a
    /// brain it owns names as its owner.
    #[serde(default)]
    pub agent_npub: Option<String>,
    /// Its person's keys Core knows, as hex: those a brain its person owns
    /// names. With `agent_npub`, the host reader places each brain's owner.
    #[serde(default)]
    pub person_keys: Vec<String>,
    /// Its retirement's receipt, once Core retired it: a retire command
    /// asked again marks it without retiring it twice.
    #[serde(default)]
    pub retired_receipt: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum FleetError {
    #[error("the fleet could not be read: {0}")]
    Unreadable(String),
}

pub trait Fleet {
    /// Every live old agent and its owner, now.
    fn roster(&self) -> Result<Roster, FleetError>;
    /// `old`'s binding, or `None` if Core has no runtime for it.
    fn binding(&self, old: &str) -> Result<Option<Binding>, FleetError>;
}

/// A fleet written out by hand or by a test: `{roster, bindings: {old: …}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetFile {
    pub roster: Roster,
    #[serde(default)]
    pub bindings: BTreeMap<String, Binding>,
}

/// A fleet read from a file, again on every call.
pub struct FileFleet {
    path: PathBuf,
}

impl FileFleet {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    fn read(&self) -> Result<FleetFile, FleetError> {
        let bytes = std::fs::read(&self.path)
            .map_err(|error| FleetError::Unreadable(format!("{}: {error}", self.path.display())))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| FleetError::Unreadable(format!("{}: {error}", self.path.display())))
    }
}

impl Fleet for FileFleet {
    fn roster(&self) -> Result<Roster, FleetError> {
        Ok(self.read()?.roster)
    }

    fn binding(&self, old: &str) -> Result<Option<Binding>, FleetError> {
        Ok(self.read()?.bindings.remove(old))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file written for an older mover may name each binding's owned
    /// brains (`"owned_brains": []` on the canary's): read and ignored.
    #[test]
    fn an_older_files_owned_brains_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fleet.json");
        for owned in [
            serde_json::json!([]),
            serde_json::json!([{ "id": "b1", "name": "Notes" }]),
        ] {
            let file = serde_json::json!({
                "roster": { "snapshot": "s", "observed_ms": 1, "people": [] },
                "bindings": { "proj_a": {
                    "old": "proj_a", "agent_runtime_id": "rt_a", "source_host_id": "lat-3",
                    "source_machine_id": "m_a", "durable_state_id": "rt_a",
                    "owned_brains": owned,
                } },
            });
            std::fs::write(&path, file.to_string()).unwrap();
            let binding = FileFleet::new(&path).binding("proj_a").unwrap().unwrap();
            assert_eq!(
                (binding.durable_state_id.as_str(), binding.person_keys.len()),
                ("rt_a", 0)
            );
        }
    }
}
