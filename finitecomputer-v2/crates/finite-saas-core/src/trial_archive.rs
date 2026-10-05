//! Reversible trial storage residency, separate from terminal offboarding.
use crate::RuntimeRetirementSnapshotReceipt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrialArchiveSnapshot {
    pub receipt: RuntimeRetirementSnapshotReceipt,
    /// The relocation tree hash is deliberately separate from the ZIP manifest hash.
    pub durable_state_manifest_sha256: String,
    pub agent_principal: String,
}
