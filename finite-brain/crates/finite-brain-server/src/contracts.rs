use finite_brain_core::{FolderAccessMode, FolderRole, VaultKind};
use serde::{Deserialize, Serialize};

/// Create Vault request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVaultRequest {
    pub vault_id: String,
    pub kind: CreateVaultKind,
    pub name: String,
    #[serde(default)]
    pub bootstrap_grants: Vec<CreateVaultFolderKeyGrantRequest>,
    #[serde(default)]
    pub personal_agent_email: Option<String>,
    #[serde(default)]
    pub personal_agent_npub: Option<String>,
}

/// Supported Vault creation kinds.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateVaultKind {
    Personal,
    Organization,
}

/// Client-generated current Folder Key Grant for initial Vault bootstrap.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVaultFolderKeyGrantRequest {
    pub folder_id: String,
    pub grant: FolderKeyGrantRequest,
}

/// Vault metadata response without plaintext Page content.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultMetadataResponse {
    pub vault_id: String,
    pub kind: VaultKind,
    pub name: String,
    pub owner_user_id: Option<String>,
    pub personal_agent: Option<PersonalAgentResponse>,
    pub members: Vec<String>,
    pub admins: Vec<String>,
    pub identities: Vec<IdentityResponse>,
    pub folders: Vec<FolderMetadataResponse>,
    pub mounted_folders: Vec<MountedFolderResponse>,
    pub grant_count: usize,
}

/// The one active Personal Agent relationship for a Personal Vault.
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

/// Authenticated Vault switcher response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleVaultsResponse {
    pub vaults: Vec<VisibleVaultResponse>,
}

/// Client-visible Vault summary.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleVaultResponse {
    pub vault_id: String,
    pub kind: VaultKind,
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
    pub organization_vault_id: String,
    pub source_vault_id: String,
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
    pub vault_id: String,
    pub folder_id: String,
    pub object_id: String,
    pub revision: u64,
    pub ciphertext: String,
    pub deleted: bool,
}

/// Encrypted Vault Export response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedVaultExportResponse {
    pub version: String,
    pub vault: ExportVaultSummaryResponse,
    pub folders: Vec<EncryptedExportFolderResponse>,
    pub objects: Vec<EncryptedExportObjectResponse>,
    pub key_grants: Vec<FolderKeyGrantResponse>,
    pub access_state: EncryptedExportAccessStateResponse,
}

/// Vault summary in an encrypted export.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportVaultSummaryResponse {
    pub id: String,
    pub kind: VaultKind,
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
    pub vault_id: String,
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
    pub vault_id: String,
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

/// Owner-authorized creation or retry of one initial Agent Workspace pairing.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnsureAgentWorkspacePairingRequest {
    pub agent_npub: String,
    pub folder_id: String,
    pub name: String,
    pub path: String,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub access_change_event: serde_json::Value,
}

/// Durable Brain Email Access Delegation exposed to its Personal Vault owner.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkspacePairingResponse {
    pub delegation_id: String,
    pub vault_id: String,
    pub owner_npub: String,
    pub agent_npub: String,
    pub workspace_folder_id: String,
    pub scope: AgentWorkspaceScopeResponse,
    pub status: String,
    pub created_by_npub: String,
    pub created_at: String,
    pub updated_at: String,
    pub audit: Vec<AgentWorkspacePairingAuditResponse>,
    pub duplicate: bool,
}

/// Initial and current Folder scope for an Agent Workspace delegation.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkspaceScopeResponse {
    pub folder_ids: Vec<String>,
    pub permission: String,
}

/// Durable explanation of one delegation lifecycle action.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkspacePairingAuditResponse {
    pub id: String,
    pub action: String,
    pub actor_npub: String,
    pub subject_npub: String,
    pub folder_ids: Vec<String>,
    pub occurred_at: String,
}

/// Owner-visible current Agent Workspace pairings.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkspacePairingListResponse {
    pub pairings: Vec<AgentWorkspacePairingResponse>,
}

/// Agent-first request backed by one owner-signed Personal Vault bootstrap authorization.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapPersonalVaultForAgentRequest {
    pub vault_id: String,
    pub name: String,
    pub folder_id: String,
    pub folder_name: String,
    pub folder_path: String,
    pub bootstrap_grants: Vec<CreateVaultFolderKeyGrantRequest>,
    pub workspace_grants: Vec<FolderKeyGrantRequest>,
    pub bootstrap_authorization: serde_json::Value,
    pub access_change_event: serde_json::Value,
}

/// The converged user-owned Personal Vault and its initial Agent Workspace pairing.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapPersonalVaultForAgentResponse {
    pub vault: VaultMetadataResponse,
    pub pairing: AgentWorkspacePairingResponse,
}

/// Owner-authorized grant of one additional restricted Folder to a paired Agent Principal.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpandAgentWorkspaceRequest {
    pub grant: FolderKeyGrantRequest,
    pub access_change_event: serde_json::Value,
}

/// One delegated Folder rotation supplied during Agent Workspace revocation.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeAgentWorkspaceFolderRequest {
    pub folder_id: String,
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantRequest>,
    pub reencrypted_records: Vec<RotationObjectRequest>,
    pub access_change_event: serde_json::Value,
}

/// Revoke an Agent Principal's complete current Personal Vault Folder scope.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeAgentWorkspaceRequest {
    pub folders: Vec<RevokeAgentWorkspaceFolderRequest>,
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

/// Create Vault Invitation request.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVaultInvitationRequest {
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

/// Vault Invitation response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultInvitationResponse {
    pub id: String,
    pub vault_id: String,
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

/// Claim an Email Invite Bootstrap into npub-bound Vault access.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimEmailVaultInvitationRequest {
    pub email: String,
    pub email_proof_created_at: String,
    #[serde(default)]
    pub invite_unwrap_proof_event_json: Option<String>,
    #[serde(default)]
    pub grants: Vec<CreateVaultFolderKeyGrantRequest>,
}

/// Request authenticated, post-proof Invite Instructions for an Email Invite Bootstrap.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostProofInviteInstructionsRequest {
    pub email: String,
    pub email_proof_created_at: String,
}

/// Vault Invitation list response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultInvitationListResponse {
    pub invitations: Vec<VaultInvitationResponse>,
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
    pub vault_id: String,
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
    pub destination_vault_id: String,
    pub destination_admin_npub: String,
    pub grant: FolderKeyGrantRequest,
    pub access_change_event: serde_json::Value,
}

/// Shared Folder Invitation response.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolderInvitationResponse {
    pub id: String,
    pub source_vault_id: String,
    pub source_folder_id: String,
    pub destination_vault_id: String,
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
    pub source_vault_id: String,
    pub source_folder_id: String,
    pub destination_vault_id: String,
    pub destination_admin_npub: String,
    pub identities: Vec<IdentityResponse>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub member_npubs: Vec<String>,
}

/// Shared Folder Invitation list response for one Vault, split by direction.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolderInvitationListResponse {
    pub outgoing: Vec<SharedFolderInvitationResponse>,
    pub incoming: Vec<SharedFolderInvitationResponse>,
}

/// Shared Folder Connection list response for one Vault, split by direction.
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
