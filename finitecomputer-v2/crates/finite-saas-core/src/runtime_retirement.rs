//! Forward-only offboarding phases and verified retirement receipts.

use crate::{
    AgentRuntime, CoreError, CoreResult, RuntimeControlKind, RuntimeControlRequest,
    RuntimeSpecEnvelope, parse_time, runtime_spec_v1, wire_enum,
};
use serde::Deserialize;
use serde::Serialize;

wire_enum! {
/// The single forward-only offboarding state of a Runtime. Each phase is
/// written in the same transaction as the side effect it records and never
/// moves backward: a destroy request (`RetirementRequested`), a stored
/// verified retirement receipt (`ReceiptVerified`), recorded compute removal
/// (`ComputeRemoved`), the offboarding boundary (`LinkDeactivated`), and the
/// terminal departure record (`Archived`). No phase means the Runtime is
/// live. Purge User Data stays the separate retention-gated path (ADR 0001).
    OffboardingPhase {
    RetirementRequested => "retirement_requested",
    ReceiptVerified => "receipt_verified",
    ComputeRemoved => "compute_removed",
    LinkDeactivated => "link_deactivated",
    Archived => "archived",
    }
    parse: parse_offboarding_phase
}

impl OffboardingPhase {
    fn rank(self) -> u8 {
        match self {
            Self::RetirementRequested => 1,
            Self::ReceiptVerified => 2,
            Self::ComputeRemoved => 3,
            Self::LinkDeactivated => 4,
            Self::Archived => 5,
        }
    }

    /// True when `next` keeps the phase moving strictly forward. Restating
    /// the current phase is allowed so an idempotent replay never regresses.
    pub fn transition_allowed(current: Option<Self>, next: Self) -> bool {
        current.is_none_or(|current| current.rank() <= next.rank())
    }

    /// True when this phase has reached or passed `phase`.
    pub fn reached(self, phase: Self) -> bool {
        self.rank() >= phase.rank()
    }

    /// Derive the phase from the durable facts a pre-phase-machine Core
    /// recorded. Mirrors the 0020 backfill exactly; the pre-deploy census
    /// enumerates these flag combinations. A stored verified receipt proves
    /// the runner removed compute before completing the destroy, so receipt
    /// plus an active link is the half-retired ghost (`ComputeRemoved`) and
    /// receipt plus an inactive link is a completed retirement (`Archived`).
    /// An inactive link with the project's active link on another Runtime is
    /// a relocation leftover, not an offboarding (`None`).
    pub fn from_legacy_facts(
        has_verified_receipt: bool,
        destroy_request_active: bool,
        link_active: bool,
        any_link_exists: bool,
        project_has_active_link: bool,
    ) -> Option<Self> {
        if has_verified_receipt {
            return Some(if link_active {
                Self::ComputeRemoved
            } else {
                Self::Archived
            });
        }
        if destroy_request_active && link_active {
            return Some(Self::RetirementRequested);
        }
        if link_active || !any_link_exists || project_has_active_link {
            return None;
        }
        Some(Self::Archived)
    }
}

impl std::fmt::Display for OffboardingPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub const RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA: &str = "runtime_retirement_snapshot.v1";

pub const RUNTIME_RETIREMENT_BACKEND_BORG: &str = "borg";

pub const RUNTIME_RETIREMENT_RETENTION_INDEFINITE: &str = "indefinite_until_purge";

/// Restore-relevant facts produced only after an exact retirement ZIP has
/// been uploaded and read back successfully. Locators are opaque archive
/// names, never repository URLs or credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRetirementSnapshotReceipt {
    pub schema: String,
    pub request_id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub durable_state_id: String,
    pub runtime_artifact_id: String,
    pub backend: String,
    pub locator: String,
    pub zip_bytes: u64,
    pub zip_sha256: String,
    pub manifest_sha256: String,
    pub created_at: String,
    pub verified_at: String,
    pub recovery_authority_id: String,
    pub retention_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRetirementSnapshot {
    pub receipt: RuntimeRetirementSnapshotReceipt,
    pub stored_at: String,
}

pub fn runtime_retirement_archive_locator(request_id: &str) -> String {
    format!("retirement-{request_id}")
}

pub(crate) fn validate_runtime_retirement_snapshot_receipt(
    receipt: &RuntimeRetirementSnapshotReceipt,
    request: &RuntimeControlRequest,
    runtime: &AgentRuntime,
    runtime_spec: &RuntimeSpecEnvelope,
    now: &str,
) -> CoreResult<()> {
    let spec = runtime_spec_v1(runtime_spec);
    let created_at = parse_time(&receipt.created_at)
        .map_err(|_| CoreError::RuntimeRetirementSnapshotMismatch)?;
    let verified_at = parse_time(&receipt.verified_at)
        .map_err(|_| CoreError::RuntimeRetirementSnapshotMismatch)?;
    let now = parse_time(now).map_err(|_| CoreError::RuntimeRetirementSnapshotMismatch)?;
    let opaque_value_valid = |value: &str| {
        !value.is_empty()
            && value.len() <= 256
            && !value.chars().any(char::is_control)
            && !value.contains("//")
    };
    let hash_valid = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if request.kind != RuntimeControlKind::Destroy
        || receipt.schema != RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA
        || receipt.request_id != request.id
        || receipt.project_id != request.project_id
        || receipt.agent_runtime_id != request.agent_runtime_id
        || receipt.project_id != runtime.project_id
        || receipt.durable_state_id != spec.durable_state_id
        || receipt.runtime_artifact_id != spec.runtime_artifact_id
        || runtime.runtime_artifact_id.as_deref() != Some(receipt.runtime_artifact_id.as_str())
        || receipt.backend != RUNTIME_RETIREMENT_BACKEND_BORG
        || receipt.locator != runtime_retirement_archive_locator(&request.id)
        || receipt.zip_bytes == 0
        || receipt.zip_bytes > i64::MAX as u64
        || !hash_valid(&receipt.zip_sha256)
        || !hash_valid(&receipt.manifest_sha256)
        || !opaque_value_valid(&receipt.recovery_authority_id)
        || receipt.retention_policy != RUNTIME_RETIREMENT_RETENTION_INDEFINITE
        || created_at > verified_at
        || verified_at > now
    {
        return Err(CoreError::RuntimeRetirementSnapshotMismatch);
    }
    Ok(())
}
