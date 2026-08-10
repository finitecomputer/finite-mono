use finite_brain_core::{BrainKind, FolderAccessMode, FolderRole};
use serde::{Deserialize, Serialize};

/// Create Brain request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBrainRequest {
    pub brain_id: String,
    pub kind: CreateBrainKind,
    pub name: String,
    #[serde(default)]
    pub bootstrap_grants: Vec<CreateBrainFolderKeyGrantRequest>,
    #[serde(default)]
    pub personal_agent_email: Option<String>,
    #[serde(default)]
    pub personal_agent_npub: Option<String>,
    #[serde(default)]
    pub initial_agent_email: Option<String>,
    #[serde(default)]
    pub initial_agent_npub: Option<String>,
    #[serde(default)]
    pub requesting_user_npub: Option<String>,
}

/// Supported Brain creation kinds.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateBrainKind {
    Personal,
    Organization,
}

/// Client-generated current Folder Key Grant for initial Brain bootstrap.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBrainFolderKeyGrantRequest {
    pub folder_id: String,
    pub grant: FolderKeyGrantRequest,
}

/// Brain metadata response without plaintext Page content.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainMetadataResponse {
    pub brain_id: String,
    pub kind: BrainKind,
    pub name: String,
    pub owner_user_id: Option<String>,
    pub personal_agent: Option<PersonalAgentResponse>,
    #[serde(default)]
    pub personal_brain_agents: Vec<PersonalBrainAgentResponse>,
    #[serde(default)]
    pub human_anchored_agent_authorities: Vec<HumanAnchoredAgentAuthorityResponse>,
    /// Durable account-cohort provenance. Clients use this to label included
    /// agents and route human-cohort versus targeted-agent changes correctly.
    #[serde(default)]
    pub account_access_cohorts: Vec<AccountAccessCohortResponse>,
    pub members: Vec<String>,
    #[serde(default)]
    pub guests: Vec<String>,
    pub admins: Vec<String>,
    pub identities: Vec<IdentityResponse>,
    pub folders: Vec<FolderMetadataResponse>,
    pub mounted_folders: Vec<MountedFolderResponse>,
    pub grant_count: usize,
    /// Authoritative current-grant coverage for Organization Brain people.
    /// Populated only when the metadata requester is an Organization admin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collaborator_readiness: Vec<CollaboratorReadinessResponse>,
}

/// Brain role and authoritative current Folder Key Grant coverage for one
/// Organization Brain collaborator.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaboratorReadinessResponse {
    pub target_npub: String,
    pub brain_role: String,
    pub ready_count: usize,
    pub total_count: usize,
}

/// The one active Personal Agent relationship for a Personal Brain.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalAgentResponse {
    pub owner_npub: String,
    pub agent_npub: String,
    pub created_by_npub: String,
    pub created_at: String,
    pub updated_at: String,
}

/// One entry in the additive complete Personal Brain Agent Set.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalBrainAgentResponse {
    pub agent_npub: String,
    pub agent_nip05: String,
    pub display_name: String,
    pub status: String,
    pub roster_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

/// Explicit routine authority; this never represents ownership, recovery, or
/// whole-Brain destructive authority.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanAnchoredAgentAuthorityResponse {
    pub agent_npub: String,
    pub human_npub: String,
    pub scope: String,
    pub status: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountAccessCohortResponse {
    pub cohort_id: String,
    pub human_npub: String,
    pub human_email: String,
    pub scope_kind: String,
    pub folder_id: Option<String>,
    pub provenance_kind: String,
    pub status: String,
    pub participants: Vec<AccountAccessCohortParticipantResponse>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountAccessCohortParticipantResponse {
    pub npub: String,
    pub relationship: String,
    pub nip05: String,
    pub display_name: String,
    pub status: String,
    pub exclusion_reason: Option<String>,
    #[serde(default)]
    pub brain_access_excluded: bool,
    #[serde(default)]
    pub excluded_folder_ids: Vec<String>,
}

/// Display metadata for one canonical Nostr identity.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityResponse {
    pub npub: String,
    pub hex: String,
    pub display: String,
    pub nip05: Option<String>,
    pub relays: Vec<String>,
    pub verified_at: Option<String>,
}

/// Resolve a public identity input to canonical npub metadata.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveIdentityRequest {
    pub input: String,
}

/// Authenticated Brain switcher response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleBrainsResponse {
    pub brains: Vec<VisibleBrainResponse>,
}

/// Client-visible Brain summary.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleBrainResponse {
    pub brain_id: String,
    pub kind: BrainKind,
    pub name: String,
    pub role: String,
    pub invite_code: Option<String>,
}

/// Server-visible Folder metadata response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderMetadataResponse {
    pub id: String,
    pub name: String,
    pub role: FolderRole,
    pub access: FolderAccessMode,
    pub parent_folder_id: Option<String>,
    pub path: String,
    pub access_user_ids: Vec<String>,
    pub current_key_version: u32,
    pub setup_incomplete: bool,
}

/// Client-visible mounted Folder metadata response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MountedFolderResponse {
    pub mount_id: String,
    pub source_brain_id: String,
    pub source_folder_id: String,
    pub display_name: String,
    pub display_parent_folder_id: Option<String>,
    pub state: String,
}

/// Encrypted object write request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectWriteRequest {
    pub base_revision: Option<u64>,
    pub key_version: u32,
    pub cipher: String,
    pub ciphertext: String,
    pub revision_event: serde_json::Value,
}

/// Encrypted object tombstone request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDeleteRequest {
    pub base_revision: u64,
    pub tombstone_event: serde_json::Value,
}

/// Signed permanent deletion of one complete Folder subtree.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDeleteRequest {
    pub deletion_event: serde_json::Value,
    /// Exact Folder identities and object count shown by the confirming client.
    /// Both are mandatory and checked in the deletion transaction.
    pub expected_folder_ids: Vec<String>,
    pub expected_object_count: usize,
}

/// Counts and sync cursor returned after permanent Folder deletion.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDeleteResponse {
    pub sequence: u64,
    pub duplicate: bool,
    pub folder_count: usize,
    pub object_count: usize,
    pub deleted_folder_ids: Vec<String>,
}

/// Object write response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectWriteResponse {
    pub sequence: u64,
    pub duplicate: bool,
    pub revision: u64,
}

/// Current encrypted object response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectResponse {
    pub brain_id: String,
    pub folder_id: String,
    pub object_id: String,
    pub revision: u64,
    pub ciphertext: String,
    pub deleted: bool,
}

/// Encrypted Brain Export response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedBrainExportResponse {
    pub version: String,
    pub brain: ExportBrainSummaryResponse,
    pub folders: Vec<EncryptedExportFolderResponse>,
    pub objects: Vec<EncryptedExportObjectResponse>,
    pub key_grants: Vec<FolderKeyGrantResponse>,
    pub access_state: EncryptedExportAccessStateResponse,
}

/// Brain summary in an encrypted export.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportBrainSummaryResponse {
    pub id: String,
    pub kind: BrainKind,
    pub name: String,
    pub owner_user_id: Option<String>,
}

/// Folder entry in an encrypted export.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedExportFolderResponse {
    pub id: String,
    pub path: String,
    pub access: FolderAccessMode,
    pub current_key_version: u32,
    pub accessible: bool,
}

/// Object entry in an encrypted export.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedExportObjectResponse {
    pub folder_id: String,
    pub object_id: String,
    pub payload_json: Option<String>,
    pub revision: u64,
    pub updated_at: String,
    pub deleted: bool,
    pub opaque: bool,
}

/// Folder Key Grant metadata in an encrypted export.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderKeyGrantResponse {
    pub id: String,
    pub folder_id: String,
    pub key_version: u32,
    pub issuer_npub: String,
    pub recipient_npub: String,
    pub format: String,
    pub wrapped_event_json: String,
    pub access_change_event_json: Option<String>,
    pub created_at: String,
}

/// Access state in an encrypted export.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedExportAccessStateResponse {
    pub members: Vec<String>,
    pub admins: Vec<String>,
    pub folders: Vec<EncryptedExportFolderAccessResponse>,
}

/// Restricted Folder access state in an encrypted export.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedExportFolderAccessResponse {
    pub folder_id: String,
    pub user_ids: Vec<String>,
}

/// Sync bootstrap response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncBootstrapResponse {
    pub brain_id: String,
    pub latest_sequence: u64,
    pub objects: Vec<ObjectResponse>,
    pub object_count: usize,
    pub control_records: Vec<SyncRecordResponse>,
    pub current_state_kind: String,
}

/// Incremental sync record response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRecordResponse {
    pub sequence: u64,
    pub record_event_id: String,
    pub record_type: String,
    pub folder_id: Option<String>,
    pub object_id: Option<String>,
    pub revision: Option<u64>,
    pub actor_npub: String,
    pub client_created_at: String,
    pub payload_json: String,
    pub record_event_kind: u16,
}

/// Incremental sync pull response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPullResponse {
    pub brain_id: String,
    pub after_sequence: u64,
    pub latest_sequence: u64,
    pub records: Vec<SyncRecordResponse>,
    pub count: usize,
    pub has_more: bool,
    pub next_sequence: u64,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncRecordsQuery {
    pub(crate) after: Option<u64>,
    pub(crate) limit: Option<u64>,
}

/// Opaque Folder Key Grant metadata accepted by the server.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderKeyGrantRequest {
    pub id: String,
    pub key_version: u32,
    pub recipient_npub: String,
    pub wrapped_event_json: String,
    pub created_at: Option<String>,
}

/// Agent-first request. All authority and identity facts are derived server-side.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapPersonalBrainForAgentRequest {}

/// The converged user-owned Personal Brain and its Personal Agent relationship.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapPersonalBrainForAgentResponse {
    pub brain: BrainMetadataResponse,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalAgentFolderRotationRequest {
    pub folder_id: String,
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub reencrypted_records: Vec<RotationObjectRequest>,
    pub access_change_event: serde_json::Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacePersonalAgentRequest {
    pub agent_email: Option<String>,
    pub rotations: Vec<PersonalAgentFolderRotationRequest>,
}

/// Body for path-targeted admin mutations.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminEventRequest {
    pub access_change_event: serde_json::Value,
}

/// One Folder rotation included in atomic Member removal.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberFolderRotationRequest {
    pub folder_id: String,
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub reencrypted_records: Vec<RotationObjectRequest>,
}

/// One mounted source Folder rotation included in atomic Member removal.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberMountRotationRequest {
    pub mount_id: String,
    pub revoke_mount: bool,
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub reencrypted_records: Vec<RotationObjectRequest>,
}

/// Remove a Member and rotate every Folder the Member could read.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveMemberRequest {
    pub access_change_event: serde_json::Value,
    pub rotations: Vec<MemberFolderRotationRequest>,
    #[serde(default)]
    pub mount_rotations: Vec<MemberMountRotationRequest>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPermanentAgentDepartureRequest {
    pub human_email: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermanentAgentDepartureFolderResponse {
    pub folder_id: String,
    pub current_key_version: u32,
    pub new_key_version: u32,
    pub required_recipient_npubs: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPermanentAgentDepartureResponse {
    pub plan_id: String,
    pub fact_id: String,
    pub account_id: String,
    pub human_email: String,
    pub agent_nip05: String,
    pub agent_npub: String,
    pub departure_kind: String,
    pub occurred_at: String,
    pub folders: Vec<PermanentAgentDepartureFolderResponse>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPermanentAgentDepartureRequest {
    pub human_email: String,
    pub plan_id: String,
    pub rotations: Vec<MemberFolderRotationRequest>,
    pub access_change_event: serde_json::Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPermanentAgentDepartureResponse {
    pub fact_id: String,
    pub agent_npub: String,
    pub outcome: String,
    pub rotated_folder_ids: Vec<String>,
    pub metadata: BrainMetadataResponse,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPersonalAgentBrainAccessRequest {
    pub operation: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPersonalAgentBrainAccessResponse {
    pub plan_id: String,
    pub brain_id: String,
    pub human_email: String,
    pub target_agent_npub: String,
    pub operation: String,
    pub folders: Vec<PermanentAgentDepartureFolderResponse>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestrictPersonalAgentBrainAccessRequest {
    pub plan_id: String,
    pub rotations: Vec<MemberFolderRotationRequest>,
    pub access_change_event: serde_json::Value,
    pub authenticated_human_intent: serde_json::Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePersonalAgentBrainAccessRequest {
    pub plan_id: String,
    pub participant_grants: Vec<CreateBrainFolderKeyGrantRequest>,
    pub access_change_event: serde_json::Value,
    pub authenticated_human_intent: serde_json::Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalAgentBrainAccessResponse {
    pub outcome: String,
    pub target_agent_npub: String,
    pub operation: String,
    pub affected_folder_ids: Vec<String>,
    pub metadata: BrainMetadataResponse,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewAccountCohortReconciliationRequest {
    pub human_email: String,
    #[serde(default)]
    pub folder_id: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitAccountCohortReconciliationRequest {
    pub human_email: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    pub plan_id: String,
    pub participant_grants: Vec<CreateBrainFolderKeyGrantRequest>,
    pub access_change_event: serde_json::Value,
    pub backup_reference: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitAccountCohortReconciliationResponse {
    pub outcome: String,
    pub plan: finite_brain_store::AccountCohortReconciliationPlan,
    pub rollback_boundary: String,
    pub metadata: BrainMetadataResponse,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertPendingInvitationRequest {
    pub plan_id: String,
    pub participant_grants: Vec<CreateBrainFolderKeyGrantRequest>,
    #[serde(default)]
    pub approved_exclusions: Vec<String>,
    pub backup_reference: String,
}

/// Create Folder request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderRequest {
    pub folder_id: String,
    pub name: String,
    pub role: FolderRole,
    pub access: FolderAccessMode,
    pub parent_folder_id: Option<String>,
    pub path: String,
    pub access_user_ids: Vec<String>,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub access_change_event: serde_json::Value,
}

/// Finish setup request for setup-incomplete Folders.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishFolderSetupRequest {
    pub grants: Vec<FolderKeyGrantRequest>,
    pub access_change_event: serde_json::Value,
}

/// Grant access to one restricted Folder recipient.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantFolderAccessRequest {
    pub grant: FolderKeyGrantRequest,
    pub access_change_event: serde_json::Value,
    /// Short-lived human-signed authorization required only when one Personal
    /// Brain Agent restores another Personal Brain Agent's Folder access.
    #[serde(default)]
    pub authenticated_human_intent: Option<serde_json::Value>,
}

/// Result of granting one identity the current Folder Key.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantFolderAccessResponse {
    #[serde(flatten)]
    pub metadata: BrainMetadataResponse,
    pub outcome: GrantFolderAccessResponseOutcome,
}

/// One Folder/key-version entry in an Organization Brain collaboration snapshot.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationFolderSnapshot {
    pub folder_id: String,
    pub key_version: u32,
    pub path: String,
}

/// Client-prepared desired-state Organization Brain collaboration request.
///
/// The server receives only opaque wrapped grants. `folders` is the exact
/// inventory/key-version snapshot observed by the trusted client; grants may
/// intentionally omit entries whose source key was unavailable locally.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnsureOrganizationAdminRequest {
    pub target_npub: String,
    pub folders: Vec<CollaborationFolderSnapshot>,
    pub grants: Vec<CollaborationGrantRequest>,
    pub access_change_event: serde_json::Value,
}

/// One client-prepared wrapped grant tied to its Folder identity.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationGrantRequest {
    pub folder_id: String,
    #[serde(flatten)]
    pub grant: FolderKeyGrantRequest,
    /// A Folder-scoped signed access-change proof for this grant. A single
    /// Brain-level AddAdmin event is not sufficient evidence for Folder
    /// access and would make the audit stream semantically ambiguous.
    pub access_change_event: serde_json::Value,
}

/// Stable per-Folder desired-state outcome.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CollaborationFolderOutcome {
    Granted,
    AlreadyReady,
    MissingSourceKey,
    StaleVersion,
    Failed,
}

/// Public identity of a current Folder-key holder. The npub is safe to expose;
/// a verified NIP-05 is included when the server has one recorded, never any
/// key or grant plaintext.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationKeyHolder {
    pub npub: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// One safe Folder result in a collaboration receipt.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationFolderReceipt {
    pub folder_id: String,
    pub path: String,
    pub expected_key_version: u32,
    pub outcome: CollaborationFolderOutcome,
    pub reason: Option<String>,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_holders: Vec<CollaborationKeyHolder>,
}

/// Typed Organization Brain collaboration receipt shared by CLI and clients.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CollaborationReceiptState {
    Complete,
    Partial,
    Indeterminate,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnsureOrganizationAdminResponse {
    pub brain_id: String,
    pub target_npub: String,
    pub state: CollaborationReceiptState,
    pub brain_role: String,
    pub folders: Vec<CollaborationFolderReceipt>,
    pub ready_count: usize,
    pub total_count: usize,
    pub retryable: bool,
}

/// Stable machine-readable outcome for a Folder access grant.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GrantFolderAccessResponseOutcome {
    Granted,
    AlreadyHasAccess,
}

/// Re-encrypted object supplied during Folder Key rotation.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationObjectRequest {
    pub object_id: String,
    pub base_revision: Option<u64>,
    pub key_version: u32,
    pub cipher: String,
    pub ciphertext: String,
    pub revision_event: serde_json::Value,
}

/// Remove Folder access with required Folder Key rotation material.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveFolderAccessRequest {
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub reencrypted_records: Vec<RotationObjectRequest>,
    pub access_change_event: serde_json::Value,
    /// Short-lived human-signed authorization required only when one Personal
    /// Brain Agent restricts another Personal Brain Agent's Folder access.
    #[serde(default)]
    pub authenticated_human_intent: Option<serde_json::Value>,
}

/// Create Brain Invitation request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBrainInvitationRequest {
    /// Create one-Folder Guest access rather than Brain Membership.
    #[serde(default)]
    pub folder_only: bool,
    #[serde(default)]
    pub target_npub: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub target_email: Option<String>,
    #[serde(default)]
    pub initial_folder_access: Vec<String>,
    pub expires_at: String,
    #[serde(default)]
    pub invite_unwrap_npub: Option<String>,
    #[serde(default)]
    pub bootstrap_payload_hash: Option<String>,
    #[serde(default)]
    pub bootstrap_wrapped_event_json: Option<String>,
    #[serde(default)]
    pub bootstrap_authorization_event_json: Option<String>,
    /// Immutable account-cohort preflight plan being committed.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// One encrypted current Folder Key Grant per participant and Folder.
    #[serde(default)]
    pub participant_grants: Vec<CreateBrainFolderKeyGrantRequest>,
    /// Exact preflight exclusions explicitly approved by the inviter.
    /// Empty means no reduced participant set was authorized.
    #[serde(default)]
    pub approved_exclusions: Vec<String>,
}

/// Read-only plan for an account-cohort invitation.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewBrainInvitationRequest {
    pub target_email: String,
    #[serde(default)]
    pub folder_only: bool,
    #[serde(default)]
    pub initial_folder_access: Vec<String>,
    pub expires_at: String,
}

/// Resource scope cryptographically bound into an invitation plan.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPlanScopeResponse {
    pub kind: String,
    pub brain_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
}

/// One resolved principal in an account access cohort plan.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPlanParticipantResponse {
    pub relationship: String,
    pub name: String,
    pub nip05: String,
    pub npub: String,
    pub ready: bool,
}

/// One authoritative roster entry omitted from the proposed participant set.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPlanExclusionResponse {
    pub name: String,
    pub nip05: String,
    pub reason: String,
}

/// Current Folder Key version bound into a client-owned grant plan.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPlanKeyVersionResponse {
    pub folder_id: String,
    pub key_version: u32,
}

/// Capacity consequence of committing an invitation plan.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPlanCapacityResponse {
    pub fits: bool,
    pub resulting_members: usize,
    pub maximum_members: usize,
}

/// Stable, mutation-free account-cohort invitation preview.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewBrainInvitationResponse {
    pub plan_id: String,
    pub target_email: String,
    pub scope: InvitationPlanScopeResponse,
    pub roster_revision: u64,
    pub participants: Vec<InvitationPlanParticipantResponse>,
    pub excluded: Vec<InvitationPlanExclusionResponse>,
    pub key_versions: Vec<InvitationPlanKeyVersionResponse>,
    pub capacity: InvitationPlanCapacityResponse,
    pub expires_at: String,
}

/// Remove-only acceptance narrowing. The supplied principals must exactly
/// match the server's current permanent-departure proposal.
#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptAccountCohortInvitationRequest {
    #[serde(default)]
    pub removed_participants: Vec<String>,
}

/// Commit a current preflight as direct restricted-Folder access for the
/// addressed human and every included account agent.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantAccountCohortFolderAccessRequest {
    pub target_email: String,
    pub expires_at: String,
    pub plan_id: String,
    #[serde(default)]
    pub approved_exclusions: Vec<String>,
    pub participant_grants: Vec<CreateBrainFolderKeyGrantRequest>,
    pub access_change_event: serde_json::Value,
}

/// Participant-aware receipt for direct mailbox-addressed Folder access.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantAccountCohortFolderAccessResponse {
    pub brain_id: String,
    pub folder_id: String,
    pub target_email: String,
    pub outcome: String,
    pub participants: Vec<InvitationPlanParticipantResponse>,
    pub excluded: Vec<InvitationPlanExclusionResponse>,
    pub metadata: BrainMetadataResponse,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewAccountCohortFolderRemovalRequest {
    pub target_email: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewAccountCohortFolderRemovalResponse {
    pub plan_id: String,
    pub brain_id: String,
    pub folder_id: String,
    pub target_email: String,
    pub participants: Vec<InvitationPlanParticipantResponse>,
    pub removed_participant_npubs: Vec<String>,
    pub independently_retained_npubs: Vec<String>,
    pub required_recipient_npubs: Vec<String>,
    pub current_key_version: u32,
    pub new_key_version: u32,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveAccountCohortFolderAccessRequest {
    pub target_email: String,
    pub plan_id: String,
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub reencrypted_records: Vec<RotationObjectRequest>,
    pub access_change_event: serde_json::Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveAccountCohortFolderAccessResponse {
    pub brain_id: String,
    pub folder_id: String,
    pub target_email: String,
    pub removed_participant_npubs: Vec<String>,
    pub independently_retained_npubs: Vec<String>,
    pub new_key_version: u32,
    pub metadata: BrainMetadataResponse,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalBrainAgentAdmissionResponse {
    pub plan_id: String,
    pub brain_id: String,
    pub human_email: String,
    pub roster_revision: u64,
    pub status: String,
    pub agents: Vec<InvitationPlanParticipantResponse>,
    pub key_versions: Vec<InvitationPlanKeyVersionResponse>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitPersonalBrainAgentAdmissionsRequest {
    pub plan_id: String,
    pub participant_grants: Vec<CreateBrainFolderKeyGrantRequest>,
}

/// One Folder included in an Email Invite Bootstrap scope.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailInviteBootstrapScopeResponse {
    pub folder_id: String,
    pub access: FolderAccessMode,
    pub key_version: u32,
}

/// Brain Invitation response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainInvitationResponse {
    pub id: String,
    pub brain_id: String,
    pub target_kind: String,
    pub user_id: Option<String>,
    pub invited_email: Option<String>,
    pub invite_unwrap_npub: Option<String>,
    pub bootstrap_payload_hash: Option<String>,
    pub bootstrap_wrapped_event_json: Option<String>,
    pub bootstrap_authorization_event_json: Option<String>,
    pub bootstrap_scope: Vec<EmailInviteBootstrapScopeResponse>,
    pub folder_only: bool,
    pub claimed_by_npub: Option<String>,
    pub identities: Vec<IdentityResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<InvitationPlanParticipantResponse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<InvitationPlanExclusionResponse>,
    pub status: String,
    pub invite_code: String,
    pub accept_path: String,
    pub public_instructions_path: String,
    pub public_instructions_url: Option<String>,
    pub delivery_status: Option<String>,
    pub initial_folder_access: Vec<String>,
    pub expires_at: String,
    pub created_at: String,
    pub updated_at: String,
    pub accepted_at: Option<String>,
    pub duplicate_accept: bool,
}

/// One shared Account Invitation Inbox item.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInvitationInboxItemResponse {
    pub invitation: BrainInvitationResponse,
    pub hidden: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInvitationInboxResponse {
    pub invitations: Vec<AccountInvitationInboxItemResponse>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAccountInvitationVisibilityRequest {
    pub hidden: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountInvitationsQuery {
    #[serde(default)]
    pub(crate) include_hidden: bool,
}

/// Claim an Email Invite Bootstrap into npub-bound Brain access.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimEmailBrainInvitationRequest {
    pub email: String,
    pub email_proof_created_at: String,
    #[serde(default)]
    pub invite_unwrap_proof_event_json: Option<String>,
    #[serde(default)]
    pub grants: Vec<CreateBrainFolderKeyGrantRequest>,
}

/// Request authenticated, post-proof Invite Instructions for an Email Invite Bootstrap.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostProofInviteInstructionsRequest {
    pub email: String,
    pub email_proof_created_at: String,
}

/// Brain Invitation list response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainInvitationListResponse {
    pub invitations: Vec<BrainInvitationResponse>,
}

/// Create Folder Invitation request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderInvitationRequest {
    pub recipient_npub: String,
    pub grant: FolderKeyGrantRequest,
    pub access_change_event: serde_json::Value,
    pub expires_at: String,
}

/// Folder Invitation response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInvitationResponse {
    pub id: String,
    pub brain_id: String,
    pub folder_id: String,
    pub recipient_npub: String,
    pub created_by_npub: String,
    pub identities: Vec<IdentityResponse>,
    pub status: String,
    pub accept_path: String,
    pub expires_at: String,
    pub created_at: String,
    pub updated_at: String,
    pub accepted_at: Option<String>,
    pub grant_id: String,
    pub duplicate_accept: bool,
}

/// Folder Invitation list response for one Folder.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInvitationListResponse {
    pub invitations: Vec<FolderInvitationResourceResponse>,
}

/// One Folder Invitation, regardless of whether its target was already a
/// registered Member Identity or still needs Email Invite Bootstrap.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum FolderInvitationResourceResponse {
    Npub(Box<FolderInvitationResponse>),
    Email(Box<BrainInvitationResponse>),
}

/// Create Mount Offer request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMountOfferRequest {
    pub destination_brain_id: String,
    pub destination_controller_npub: String,
    pub grant: FolderKeyGrantRequest,
    pub access_change_event: serde_json::Value,
    pub expires_at: String,
}

/// Mount Offer response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MountOfferResponse {
    pub id: String,
    pub source_brain_id: String,
    pub source_folder_id: String,
    pub destination_brain_id: String,
    pub destination_controller_npub: String,
    pub created_by_npub: String,
    pub identities: Vec<IdentityResponse>,
    pub status: String,
    pub current_key_version: u32,
    pub accept_path: String,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: String,
    pub accepted_at: Option<String>,
    pub grant_id: String,
    pub grant: FolderKeyGrantResponse,
    pub duplicate_accept: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_participant_npubs: Vec<String>,
}

/// Mount Offer list response for one Brain, split by direction.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MountOfferListResponse {
    pub outgoing: Vec<MountOfferResponse>,
    pub incoming: Vec<MountOfferResponse>,
}

/// Accept a Mount Offer with grants for any additional Personal Brain participants.
#[derive(Debug, Clone, Eq, PartialEq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptMountOfferRequest {
    #[serde(default)]
    pub grants: Vec<FolderKeyGrantRequest>,
}

/// Add one destination-governed Mount Participant.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddMountParticipantRequest {
    pub grant: FolderKeyGrantRequest,
}

/// Remove one Mount Participant with atomic Folder Key rotation.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveMountParticipantRequest {
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub reencrypted_records: Vec<RotationObjectRequest>,
}

/// Mount response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MountResponse {
    pub id: String,
    pub source_brain_id: String,
    pub source_folder_id: String,
    pub destination_brain_id: String,
    pub destination_controller_npub: String,
    pub identities: Vec<IdentityResponse>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub participant_npubs: Vec<String>,
    pub managed_access_participant_npubs: Vec<String>,
}

/// Mount list response for one Brain, split by direction.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MountListResponse {
    pub outgoing: Vec<MountResponse>,
    pub incoming: Vec<MountResponse>,
}

/// Revoke Mount request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeMountRequest {
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub reencrypted_records: Vec<RotationObjectRequest>,
}
