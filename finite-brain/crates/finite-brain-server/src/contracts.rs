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
    /// Pending human Approval cards (ADR-0046). Populated only when the
    /// metadata requester holds Brain admin standing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_approvals: Vec<ApprovalRequestResponse>,
    /// Folder Key wraps still owed to waiting recipients. Populated only
    /// when the metadata requester holds Brain admin standing, since those
    /// are the clients that can open the current Folder Keys and complete
    /// the wraps. Older clients ignore the field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_wraps: Vec<PendingGrantWrapResponse>,
    /// Pending viewer-session wraps (ephemeral browser keys waiting for a
    /// wrapped Folder Key). Populated only when the requester holds Brain
    /// admin standing; older clients ignore the field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_viewer_wraps: Vec<PendingViewerWrapResponse>,
}

/// One Folder Key wrap a key-holding client can complete for a waiting
/// recipient. Metadata only; no key material.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingGrantWrapResponse {
    pub folder_id: String,
    pub recipient_npub: String,
    pub key_version: u32,
    pub reason: String,
    pub created_at: String,
}

/// Request to mint a viewer session: the requesting principal (authenticated
/// by NIP-98) asks for the Folder Key wrapped to an ephemeral browser key.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateViewerSessionRequest {
    pub brain_id: String,
    pub folder_id: String,
    pub ephemeral_npub: String,
    #[serde(default)]
    pub requested_ttl_secs: Option<u64>,
}

/// Viewer session state for the browser poll and the admin access surface.
/// `wrappedKeyPayload` is NIP-44 ciphertext addressed to the ephemeral key;
/// the server stores and relays it blind.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerSessionResponse {
    pub id: String,
    pub brain_id: String,
    pub folder_id: String,
    pub ephemeral_npub: String,
    pub requester_npub: String,
    pub key_version: u32,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrapped_key_payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_by_npub: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
}

/// Viewer sessions of one Brain for the admin access surface.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerSessionListResponse {
    pub sessions: Vec<ViewerSessionResponse>,
}

/// One viewer-session wrap completion from a key-holding client: the
/// NIP-44 wrapped Folder Key addressed to the ephemeral npub.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteViewerWrapRequest {
    pub ephemeral_npub: String,
    pub key_version: u32,
    pub wrapped_key_payload: String,
}

/// Batch viewer-session wrap completion for one Folder.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteViewerWrapsRequest {
    #[serde(default)]
    pub wraps: Vec<CompleteViewerWrapRequest>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteViewerWrapsResponse {
    pub brain_id: String,
    pub folder_id: String,
    pub outcome: String,
    pub completed_count: usize,
    pub completed_ephemerals: Vec<String>,
}

/// Query parameters for the encrypted-read route. `after=0` is the initial
/// fetch (subject to the live-view size caps); `after>0` is a delta page.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize)]
pub struct FolderViewRecordsQuery {
    #[serde(default)]
    pub after: Option<u64>,
    #[serde(default)]
    pub limit: Option<u64>,
}

/// One encrypted Folder record for the viewer: sequence plus the record's
/// encrypted envelope. Object revisions carry ciphertext; tombstones do not.
/// Control records never appear.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerRecordResponse {
    pub sequence: u64,
    pub record_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ciphertext: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderViewRecordsResponse {
    pub brain_id: String,
    pub folder_id: String,
    pub after_sequence: u64,
    pub latest_sequence: u64,
    pub records: Vec<ViewerRecordResponse>,
    pub count: usize,
    pub has_more: bool,
    pub session_expires_at: String,
}

/// Pending viewer-session wrap marker for admin surfaces: the ephemeral
/// recipient plus the principal whose Folder access justified the request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingViewerWrapResponse {
    pub folder_id: String,
    pub ephemeral_npub: String,
    pub requester_npub: String,
    pub key_version: u32,
    pub created_at: String,
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
    /// Folder Key wraps still owed to waiting recipients. Populated only
    /// when the export requester holds Brain admin standing; older clients
    /// ignore the field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_wraps: Vec<PendingGrantWrapResponse>,
    /// Pending viewer-session wraps (ephemeral browser keys waiting for a
    /// wrapped Folder Key). Populated only when the requester holds Brain
    /// admin standing; older clients ignore the field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_viewer_wraps: Vec<PendingViewerWrapResponse>,
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
    /// Folder Key wraps still owed to waiting recipients. Populated only
    /// when the sync requester holds Brain admin standing; older clients
    /// ignore the field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_wraps: Vec<PendingGrantWrapResponse>,
    /// Pending viewer-session wraps (ephemeral browser keys waiting for a
    /// wrapped Folder Key). Populated only when the requester holds Brain
    /// admin standing; older clients ignore the field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_viewer_wraps: Vec<PendingViewerWrapResponse>,
}

/// Batch completion request for pending grant wraps: one opaque NIP-59 grant
/// per marked recipient, each carrying its recipient npub.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletePendingWrapsRequest {
    pub grants: Vec<FolderKeyGrantRequest>,
}

/// Receipt for a pending grant wrap completion.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletePendingWrapsResponse {
    pub brain_id: String,
    pub folder_id: String,
    pub outcome: String,
    pub completed_count: usize,
    pub completed_recipients: Vec<String>,
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

/// Ensure-access request (ADR-0046 onboarding repair): ensure the target's
/// Brain Membership and report their entitled Folder Key Grant state so the
/// caller can fill any gaps with wrapped grants.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnsureAccessRequest {
    pub target_npub: String,
    /// Signed AddMember proof. Required only when the Membership is missing;
    /// ignored when the target already holds Membership.
    #[serde(default)]
    pub access_change_event: Option<serde_json::Value>,
}

/// Stable machine-readable Folder grant state in an ensure-access receipt.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EnsureAccessGrantState {
    Present,
    Missing,
}

/// One entitled Folder's current grant state for the ensure-access target.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnsureAccessFolderStatus {
    pub folder_id: String,
    pub path: String,
    pub key_version: u32,
    pub grant: EnsureAccessGrantState,
}

/// Ensure-access receipt. Membership is completed server-side; missing Folder
/// Key Grants are left to the caller, who holds the Folder Keys.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnsureAccessResponse {
    pub brain_id: String,
    pub target_npub: String,
    /// `added` when this call created the Membership, `alreadyMember` when it
    /// already existed (including owner and admin principals).
    pub membership: String,
    /// The target's Brain Role after this call: `owner`, `admin`, or `member`.
    pub brain_role: String,
    /// One entry per Folder the target is entitled to read.
    pub folders: Vec<EnsureAccessFolderStatus>,
    pub missing_count: usize,
    /// `complete` when every entitled Folder has a current grant;
    /// `grantsMissing` when the caller still has wraps to deliver.
    pub state: String,
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
    /// Computed at read time: true when a pending Invitation's `expiresAt`
    /// is at or before the server's current time. Stored rows are never
    /// mutated to derive it.
    #[serde(default)]
    pub expired: bool,
    /// Set when a plan-linked acceptance observed roster narrowing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrowed: Option<NarrowedAcceptanceResponse>,
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

/// Invitation Preflight request: resolve one email target into an immutable plan.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPreflightRequest {
    pub target: String,
}

/// Resolved human Principal in an Invitation Plan.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPlanHuman {
    pub email: String,
    pub npub: Option<String>,
}

/// Resolved, grant-ready agent Principal in an Invitation Plan.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPlanAgent {
    pub managed_agent_email: String,
    pub agent_npub: Option<String>,
    pub status: String,
}

/// Participant excluded from an Invitation Plan with an explicit reason.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPlanExclusion {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub reason: String,
}

/// Invitation Preflight response: the immutable plan commit must match.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPreflightResponse {
    pub plan_id: String,
    pub plan_hash: String,
    pub human: InvitationPlanHuman,
    pub agents: Vec<InvitationPlanAgent>,
    pub roster_revision: Option<i64>,
    pub exclusions: Vec<InvitationPlanExclusion>,
    pub expires_at: String,
    /// Set when this plan replaces a stale plan at commit time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_plan_id: Option<String>,
}

/// Invitation Commit request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationCommitRequest {
    pub plan_id: String,
    pub plan_hash: String,
    /// Agent emails to exclude from the committed set (narrowing only).
    #[serde(default)]
    pub reduced_set: Option<Vec<String>>,
}

/// One per-principal Brain Invitation written by a commit.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedPrincipalInvitation {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub npub: String,
    pub invitation: BrainInvitationResponse,
}

/// One plan participant skipped at commit with an explicit reason.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSkippedPrincipal {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub reason: String,
}

/// Invitation Commit response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationCommitResponse {
    pub status: String,
    pub plan_id: String,
    pub roster_revision: Option<i64>,
    pub invitations: Vec<CommittedPrincipalInvitation>,
    pub skipped: Vec<CommitSkippedPrincipal>,
    /// Pending-but-expired invitations the commit revoked to supersede them
    /// with a fresh invitation for the same (Brain, target).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded_invitation_ids: Vec<String>,
}

/// Acceptance result note when the account roster narrowed the resolved set:
/// permanently departed participants are excluded, never added or substituted.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrowedAcceptanceResponse {
    pub roster_revision: Option<i64>,
    pub exclusions: Vec<InvitationPlanExclusion>,
}

/// Approval Request creation (ADR-0046): an agent or the UI asks a human key
/// holder to sign one scoped Brain action.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestCreateRequest {
    pub action: String,
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Invite-commit shorthand for requesters without admin standing: the
    /// server resolves this account email into a fresh invitation plan and
    /// files the request against it. Mutually exclusive with `plan_id`.
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub target_npubs: Vec<String>,
}

/// One stored human Approval request. `payload` is the exact unsigned action
/// payload the human's hosted key signs; the device fills in `humanNpub`.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestResponse {
    pub id: String,
    pub brain_id: String,
    pub action: String,
    pub payload: serde_json::Value,
    pub nonce: String,
    pub expires_at: u64,
    pub requested_by_npub: String,
    pub status: String,
    pub approval_event_id: Option<String>,
    pub resolved_by_npub: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Invitation ids an approved invite-commit produced. Lets members —
    /// whose agents cannot list org-brain invitations — observe their
    /// filing's outcome through `approvals list --all`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resulting_invitations: Option<Vec<String>>,
}

/// Approval Request list response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestListResponse {
    pub requests: Vec<ApprovalRequestResponse>,
}

/// Signed `finite-brain-approval-v1` artifact submitted for validation and
/// execution. `requestId` binds the artifact to one pending Approval Request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalSubmissionRequest {
    pub approval_event_json: String,
    #[serde(default)]
    pub request_id: Option<String>,
}

/// Approval submission outcome.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalSubmissionResponse {
    pub status: String,
    pub action: String,
    pub approval_event_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Action-specific result: an Invitation Commit response for
    /// 'invite-commit', or `{"grantedNpubs": [...]}` for 'delegation-grant'.
    pub result: serde_json::Value,
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

/// Invitee-scoped Brain Invitation summary: the pending npub-targeted
/// invitations addressed to the authenticated caller. Expired invitations
/// remain visible with `expired: true` so the invitee can ask for a re-invite
/// instead of seeing a silent disappearance.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyInvitationResponse {
    pub id: String,
    pub invite_code: String,
    pub brain_id: String,
    pub brain_display_name: String,
    pub inviter_display: String,
    pub folder_scope: Vec<String>,
    pub expires_at: String,
    /// Computed at read time: true when `expiresAt` is at or before the
    /// server's current time.
    #[serde(default)]
    pub expired: bool,
    pub public_instructions_url: Option<String>,
    pub origin_kind: String,
    pub origin_ref: Option<String>,
}

/// Invitee-scoped Brain Invitation list response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyInvitationListResponse {
    pub invitations: Vec<MyInvitationResponse>,
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

/// Folder-scoped invitation plan preview: the cohort resolution plus the
/// Folder the commit will grant Guest access to.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInvitationPreflightResponse {
    #[serde(flatten)]
    pub plan: InvitationPreflightResponse,
    pub folder_id: String,
    pub current_key_version: u32,
}

/// One per-principal Folder invitation inside a cohort Folder plan commit.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInvitationPlanParticipant {
    pub recipient_npub: String,
    pub grant: FolderKeyGrantRequest,
    pub access_change_event: serde_json::Value,
}

/// Cohort Folder plan commit: the committing key holder supplies one signed
/// access-change event and one wrapped Folder Key Grant per included
/// principal; the server fans the share links out atomically.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInvitationPlanCommitRequest {
    pub plan_id: String,
    pub plan_hash: String,
    pub expires_at: String,
    pub participants: Vec<FolderInvitationPlanParticipant>,
}

/// Folder plan commit result: one share link per included principal.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInvitationPlanCommitResponse {
    pub status: String,
    pub plan_id: String,
    pub invitations: Vec<FolderInvitationResponse>,
    pub duplicate_recipient_npubs: Vec<String>,
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
