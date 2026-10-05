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
    pub(crate) descriptions: DescriptionCoverage,
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

/// Core identity description coverage for this page: `checked`,
/// `notConfigured`, `notNeeded`, `unavailable`, or `unsupported`.
/// `checkedAt` is Core's source time, separate from the Brain snapshot.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DescriptionCoverage {
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
    pub(crate) description: IdentityDescriptionView,
    /// A stored public NIP-05 alias for this key, with its stored time. A
    /// cached claim, not rechecked by the report and never a mailbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) stored_nip05: Option<StoredAliasView>,
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

/// Permitted identity description of one exact key. `state`: `resolved`,
/// `unknown`, `ambiguous`, `notShared` or `unavailable`. Names never grant
/// or explain access; absent fields are never fabricated.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IdentityDescriptionView {
    pub(crate) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    /// `human` or `agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) account_email: Option<String>,
    /// Agents only: `active`, `offboarding` or `retired`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) lifecycle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) responsible_account: Option<ResponsibleAccountView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<DescriptionSourceView>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResponsibleAccountView {
    pub(crate) email: String,
    pub(crate) source: String,
    pub(crate) observed_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) human_public_keys_hex: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DescriptionSourceView {
    pub(crate) kind: String,
    pub(crate) observed_at: String,
    pub(crate) revision: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredAliasView {
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) stored_at: Option<String>,
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

/// Who issued a current grant and when, from the stored grant row and its
/// provenance. The wrapped key itself is encrypted to the recipient; its
/// presence never proves the recipient decrypted it.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrantEvidence {
    pub(crate) grant_id: String,
    pub(crate) issued_by: String,
    pub(crate) issued_at: String,
    pub(crate) provenance: ProvenanceView,
}
