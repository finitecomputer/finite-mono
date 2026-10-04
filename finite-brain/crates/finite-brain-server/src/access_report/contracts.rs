//! Wire contract for `GET /v1/brains/{brain_id}/access-report`.
//!
//! Names are labels with evidence, never authority. Roles, entitlements, and
//! grants come from one Brain authority snapshot identified by
//! `authorityFingerprint`; every page of one report carries the same
//! fingerprint, and page cursors are bound to it.

use serde::{Deserialize, Serialize};

pub(crate) const ACCESS_REPORT_VERSION: &str = "finite-brain-access-report-v1";

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccessReportResponse {
    pub(crate) version: String,
    pub(crate) brain_id: String,
    pub(crate) brain_kind: String,
    pub(crate) brain_name: String,
    pub(crate) authority_sequence: u64,
    /// Digest of current roles, Folders, explicit access and its sources,
    /// current grants, and Mounts. Changes even when no record is appended.
    pub(crate) authority_fingerprint: String,
    pub(crate) checked_at: String,
    pub(crate) acting_key: ActingKey,
    pub(crate) coverage: AccessCoverage,
    pub(crate) current_access_complete: bool,
    pub(crate) directory: DirectoryCoverage,
    pub(crate) totals: AccessTotals,
    /// Folders from other Brains mounted into this Brain.
    pub(crate) incoming_mounts: Vec<IncomingMountView>,
    pub(crate) page: AccessPage,
    pub(crate) identities: Vec<AccessIdentityRow>,
}

/// One Folder mounted into this Brain from a source Brain. Source-Brain
/// access by routes other than this Mount is reported by the source Brain.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IncomingMountView {
    pub(crate) mount_id: String,
    pub(crate) display_name: String,
    pub(crate) source_brain_id: String,
    pub(crate) source_folder_id: String,
    /// `active`, `revoked`, or `missing`.
    pub(crate) connection_status: String,
    /// `complete` (every participant checked) or `withheld` (bound reached).
    pub(crate) participant_detail: String,
    pub(crate) participants: usize,
}

/// One row's participation in an incoming Mount: only facts the Mount
/// created. `state`: `ready`, `grantMissing`, or `mountAccessMissing`.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IncomingMountEntry {
    pub(crate) mount_id: String,
    pub(crate) display_name: String,
    pub(crate) source_brain_id: String,
    pub(crate) source_folder_id: String,
    pub(crate) mount_access: bool,
    pub(crate) current_grant: String,
    pub(crate) state: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActingKey {
    pub(crate) npub: String,
    pub(crate) brain_role: String,
}

/// Per-scope coverage: `complete`, `unverified`, or `unsupported`.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScopeCoverage {
    pub(crate) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccessCoverage {
    pub(crate) members: ScopeCoverage,
    pub(crate) guests: ScopeCoverage,
    pub(crate) mounts: ScopeCoverage,
    pub(crate) current_grants: ScopeCoverage,
    pub(crate) access_history: ScopeCoverage,
}

/// Directory name evidence for this page: `checked`, `notConfigured`,
/// `notNeeded`, `unavailable`, or `unsupported`.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectoryCoverage {
    pub(crate) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checked_at: Option<String>,
    pub(crate) checked_keys: usize,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccessTotals {
    pub(crate) identities: usize,
    pub(crate) revocation_incomplete: usize,
    pub(crate) grants_missing: usize,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccessPage {
    pub(crate) limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) next: Option<String>,
    pub(crate) returned: usize,
}

/// One exact key. Two runtimes sharing a key are one row; distinct keys
/// never merge, whatever their names.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccessIdentityRow {
    pub(crate) npub: String,
    pub(crate) hex: String,
    pub(crate) brain_role: String,
    pub(crate) role_sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) membership: Option<ProvenanceView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) participation: Option<ParticipationView>,
    pub(crate) name: NameEvidence,
    pub(crate) identity_type: IdentityTypeEvidence,
    pub(crate) folders: Vec<FolderAccessEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) incoming_mounts: Vec<IncomingMountEntry>,
    pub(crate) missing_current_grants: Vec<String>,
    pub(crate) revocation_incomplete: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProvenanceView {
    pub(crate) origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) delegated_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) origin_ref: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParticipationView {
    pub(crate) kind: String,
    pub(crate) recorded_at: String,
}

/// Name evidence: `verified` (active Identity Directory binding for the exact key),
/// `domainClaimed` (domain publishes a name-to-key claim, without key-holder confirmation),
/// `storedNotRechecked` (stored label; recheck unavailable), or `unknown`.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NameEvidence {
    pub(crate) state: String,
    pub(crate) display: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) matched_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) stored_verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) additional_names: Vec<String>,
    #[serde(default)]
    pub(crate) more_names: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

/// `managedAgent` only from an authoritative managed-agent binding for this
/// exact key; everything else is `notConfirmed`.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IdentityTypeEvidence {
    pub(crate) value: String,
    pub(crate) label: String,
    pub(crate) evidence: String,
}

/// Folder state: `ready` (entitled, current grant present), `grantMissing`
/// (entitled, no current grant), or `revocationIncomplete` (current grant
/// present without entitlement). `entitlementSources` come from Brain roles
/// and recorded `folder_access_sources`: `explicitAccess` (direct),
/// `invitationAccess`, `mount:<connection>` (active Mount),
/// `inactiveMount:<connection>` (retained access whose Mount is no longer
/// active), or `explicitAccessUnsourced` (no source recorded).
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FolderAccessEntry {
    pub(crate) folder_id: String,
    pub(crate) path: String,
    pub(crate) access_mode: String,
    pub(crate) key_version: u32,
    pub(crate) entitled: bool,
    pub(crate) entitlement_sources: Vec<String>,
    pub(crate) current_grant: String,
    pub(crate) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) grant: Option<GrantEvidence>,
}

/// Who issued a current grant and when, from the stored grant row and any
/// signed access-change stored with it. The wrapped key itself is encrypted
/// to the recipient; its presence never proves the recipient decrypted it.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrantEvidence {
    pub(crate) grant_id: String,
    pub(crate) issued_by: String,
    pub(crate) issued_at: String,
    pub(crate) provenance: ProvenanceView,
    /// `verified` (a valid signed grant-folder-access event for exactly this
    /// Brain, Folder, recipient, and key version), `mismatched` (validly
    /// signed but describing something else), `unreadable` (does not parse
    /// or verify), or `notStored`.
    pub(crate) signed_audit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) signed_audit_actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) signed_audit_at: Option<String>,
}
