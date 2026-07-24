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
    pub shared_folder_source: bool,
    pub access_user_ids: Vec<String>,
    pub current_key_version: u32,
    pub setup_incomplete: bool,
}

/// Client-visible mounted Folder metadata response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MountedFolderResponse {
    pub mount_id: String,
    pub organization_brain_id: String,
    pub source_brain_id: String,
    pub source_folder_id: String,
    pub connection_id: String,
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
    pub shared_folder_source: bool,
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

/// Add/remove member/admin request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminTargetRequest {
    pub target_npub: String,
    pub access_change_event: serde_json::Value,
}

/// Body for path-targeted admin mutations.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminEventRequest {
    pub access_change_event: serde_json::Value,
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
    pub shared_folder_source: Option<bool>,
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
    pub target_npub: String,
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

/// Create Share Link request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateShareLinkRequest {
    pub recipient_npub: String,
    pub grant: FolderKeyGrantRequest,
    pub access_change_event: serde_json::Value,
    pub expires_at: String,
    pub create_personal_mount: Option<bool>,
}

/// Share Link response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareLinkResponse {
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
    pub create_personal_mount: bool,
    pub personal_mount_id: Option<String>,
    pub duplicate_accept: bool,
}

/// Share Link list response for one Folder.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareLinkListResponse {
    pub share_links: Vec<ShareLinkResponse>,
}

/// Mark Shared Folder Source request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkSharedFolderSourceRequest {
    pub access_change_event: serde_json::Value,
}

/// Create Shared Folder Invitation request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSharedFolderInvitationRequest {
    pub destination_brain_id: String,
    pub destination_admin_npub: String,
    pub grant: FolderKeyGrantRequest,
    pub access_change_event: serde_json::Value,
}

/// Shared Folder Invitation response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolderInvitationResponse {
    pub id: String,
    pub source_brain_id: String,
    pub source_folder_id: String,
    pub destination_brain_id: String,
    pub destination_admin_npub: String,
    pub created_by_npub: String,
    pub identities: Vec<IdentityResponse>,
    pub status: String,
    pub current_key_version: u32,
    pub accept_path: String,
    pub created_at: String,
    pub updated_at: String,
    pub accepted_at: Option<String>,
    pub grant_id: String,
    pub duplicate_accept: bool,
}

/// Shared Folder Connection response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolderConnectionResponse {
    pub id: String,
    pub source_brain_id: String,
    pub source_folder_id: String,
    pub destination_brain_id: String,
    pub destination_admin_npub: String,
    pub identities: Vec<IdentityResponse>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub member_npubs: Vec<String>,
}

/// Shared Folder Invitation list response for one Brain, split by direction.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolderInvitationListResponse {
    pub outgoing: Vec<SharedFolderInvitationResponse>,
    pub incoming: Vec<SharedFolderInvitationResponse>,
}

/// Shared Folder Connection list response for one Brain, split by direction.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolderConnectionListResponse {
    pub outgoing: Vec<SharedFolderConnectionResponse>,
    pub incoming: Vec<SharedFolderConnectionResponse>,
}

/// Update Shared Folder Connection members request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSharedFolderConnectionMembersRequest {
    pub action: String,
    pub target_npub: String,
    pub grant: Option<FolderKeyGrantRequest>,
    pub new_key_version: Option<u32>,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub reencrypted_records: Vec<RotationObjectRequest>,
}

/// Revoke Shared Folder Connection request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeSharedFolderConnectionRequest {
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub reencrypted_records: Vec<RotationObjectRequest>,
}
