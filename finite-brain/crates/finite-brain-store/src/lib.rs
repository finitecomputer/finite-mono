//! FiniteBrain SQLite store and transaction boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::Path;

use finite_brain_core::{
    BootstrapOutput, CoreError, DisplayName, Folder, FolderAccessMode, FolderId, FolderRole,
    ObjectId, RequiredFolderKeyGrant, SafeRelativePath, UserId, Vault, VaultId, VaultKind,
    VaultMember,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

mod agent_access;
mod folder_access;
mod links;
mod loading;
mod schema;
mod shared_folders;
mod sync_records;
mod vaults;

const GRANT_FORMAT_NIP59: &str = "NIP-59";
const MAX_PULL_LIMIT: u64 = 1_000;
const MAX_BOOTSTRAP_FOLDERS: usize = 1_000;
const MAX_BOOTSTRAP_GRANTS: usize = 10_000;
const MAX_LINK_LIST_ROWS: i64 = 200;
const APP_SPECIFIC_KIND: u16 = 30_078;
const NIP59_GIFT_WRAP_KIND: u16 = 1_059;
const MIGRATION_TIMESTAMP: &str = "2026-06-23T00:00:00.000Z";

/// Returns the crate name used in workspace status surfaces.
pub fn crate_name() -> &'static str {
    "finite-brain-store"
}

/// Store-level validation and SQLite boundary errors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum StoreError {
    /// Core domain validation failed.
    Core(CoreError),
    /// SQLite returned an error.
    Database { message: String },
    /// A requested Vault does not exist.
    MissingVault { vault_id: String },
    /// A requested Folder does not exist.
    MissingFolder { folder_id: String },
    /// A stable id already exists in the scoped table.
    DuplicateId { field: &'static str, value: String },
    /// Grant metadata did not include a required current recipient.
    MissingRequiredGrant { recipient_user_id: String },
    /// Stored state would violate Vault, member, admin, access, or grant rules.
    BrokenInvariant { reason: String },
    /// A sync record is malformed or violates request semantics.
    InvalidRecord { reason: String },
    /// A sync record lost optimistic concurrency.
    Conflict {
        reason: String,
        current_revision: Option<u64>,
    },
    /// The client cursor is older than the retained floor.
    RebootstrapRequired { retention_floor: u64 },
    /// A singleton invitation or share link is unavailable to this actor.
    UnavailableLink { kind: &'static str },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => write!(f, "{error}"),
            Self::Database { message } => write!(f, "database error: {message}"),
            Self::MissingVault { vault_id } => write!(f, "missing vault: {vault_id}"),
            Self::MissingFolder { folder_id } => write!(f, "missing folder: {folder_id}"),
            Self::DuplicateId { field, value } => {
                write!(f, "duplicate id for {field}: {value}")
            }
            Self::MissingRequiredGrant { recipient_user_id } => {
                write!(f, "missing required grant for {recipient_user_id}")
            }
            Self::BrokenInvariant { reason } => write!(f, "broken invariant: {reason}"),
            Self::InvalidRecord { reason } => write!(f, "invalid record: {reason}"),
            Self::Conflict {
                reason,
                current_revision,
            } => write!(
                f,
                "sync conflict: {reason}; current revision: {current_revision:?}"
            ),
            Self::RebootstrapRequired { retention_floor } => {
                write!(
                    f,
                    "rebootstrap required from retention floor {retention_floor}"
                )
            }
            Self::UnavailableLink { kind } => write!(f, "{kind} unavailable"),
        }
    }
}

impl Error for StoreError {}

impl From<CoreError> for StoreError {
    fn from(value: CoreError) -> Self {
        Self::Core(value)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database {
            message: value.to_string(),
        }
    }
}

/// Stored Folder Key Grant metadata. The encrypted key remains opaque to the server.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FolderKeyGrantMetadata {
    /// Stable grant id.
    pub id: String,
    /// Folder id.
    pub folder_id: FolderId,
    /// Folder Key version.
    pub key_version: u32,
    /// Issuer npub.
    pub issuer_npub: UserId,
    /// Recipient npub.
    pub recipient_npub: UserId,
    /// Envelope format, currently `NIP-59`.
    pub format: String,
    /// Stored wrapped event JSON.
    pub wrapped_event_json: String,
    /// Optional signed admin access-change event JSON.
    pub access_change_event_json: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
}

/// Reloaded Vault state with store-only metadata attached.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredVault {
    /// Core Vault metadata.
    pub vault: Vault,
    /// The one active Personal Agent relationship, when occupied.
    pub personal_agent: Option<PersonalAgent>,
    /// Explicit restricted Folder access by Folder id.
    pub folder_access: BTreeMap<FolderId, BTreeSet<UserId>>,
    /// Stored Folder Key Grant metadata.
    pub grants: Vec<FolderKeyGrantMetadata>,
    /// Folders that still need current grants.
    pub setup_incomplete_folder_ids: BTreeSet<FolderId>,
}

/// One active Personal Agent relationship owned by a Personal Vault.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PersonalAgent {
    pub vault_id: VaultId,
    pub owner_npub: UserId,
    pub agent_npub: UserId,
    pub created_by_npub: UserId,
    pub created_at: String,
    pub updated_at: String,
}

/// Durable product-scoped delegation between a Personal Vault owner and one Agent Principal.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BrainEmailAccessDelegation {
    pub id: String,
    pub vault_id: VaultId,
    pub owner_npub: UserId,
    pub agent_npub: UserId,
    pub workspace_folder_id: FolderId,
    pub folder_ids: Vec<FolderId>,
    pub status: String,
    pub created_by_npub: UserId,
    pub created_at: String,
    pub updated_at: String,
    pub audit: Vec<BrainEmailAccessDelegationAudit>,
}

/// One durable explanation of a delegation lifecycle change.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BrainEmailAccessDelegationAudit {
    pub id: String,
    pub action: String,
    pub actor_npub: UserId,
    pub subject_npub: UserId,
    pub folder_ids: Vec<FolderId>,
    pub occurred_at: String,
}

/// Validated store input for the initial user-first Agent Workspace pairing.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EnsurePersonalAgentWorkspaceInput {
    pub delegation_id: String,
    pub vault_id: VaultId,
    pub owner_npub: UserId,
    pub agent_npub: UserId,
    pub folder: Folder,
    pub grants: Vec<FolderKeyGrantMetadata>,
    pub sync_records: Vec<SyncRecordInput>,
    pub created_at: String,
}

/// Result of creating or safely retrying one initial Agent Workspace pairing.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EnsurePersonalAgentWorkspaceOutcome {
    pub delegation: BrainEmailAccessDelegation,
    pub duplicate: bool,
}

/// Validated all-or-nothing input for the agent-first Personal Vault path.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BootstrapPersonalAgentWorkspaceInput {
    pub authorization_id: String,
    pub authorization_event_id: String,
    pub authorization_expires_at: u64,
    pub vault: BootstrapOutput,
    pub bootstrap_grants: Vec<FolderKeyGrantMetadata>,
    pub pairing: EnsurePersonalAgentWorkspaceInput,
    pub consumed_at: String,
}

/// Result of consuming one agent-first bootstrap authorization.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BootstrapPersonalAgentWorkspaceOutcome {
    pub delegation: BrainEmailAccessDelegation,
}

/// Owner-authorized expansion of an active Agent Workspace delegation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExpandPersonalAgentWorkspaceInput {
    pub vault_id: VaultId,
    pub owner_npub: UserId,
    pub agent_npub: UserId,
    pub folder_id: FolderId,
    pub grant: FolderKeyGrantMetadata,
    pub sync_records: Vec<SyncRecordInput>,
    pub changed_at: String,
}

/// One Folder Key rotation inside an all-or-nothing agent-delegation revocation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RevokePersonalAgentFolderInput {
    pub folder_id: FolderId,
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantMetadata>,
    pub reencrypted_records: Vec<FolderObjectRevisionSyncRecord>,
    pub sync_records: Vec<SyncRecordInput>,
}

/// Owner-authorized removal and key rotation for every Folder in an agent delegation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RevokePersonalAgentWorkspaceInput {
    pub vault_id: VaultId,
    pub owner_npub: UserId,
    pub agent_npub: UserId,
    pub folders: Vec<RevokePersonalAgentFolderInput>,
    pub changed_at: String,
}

/// Verified display metadata for one canonical Nostr identity.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IdentityAlias {
    /// Canonical NIP-19 public key.
    pub npub: UserId,
    /// Lowercase 64-character public key hex.
    pub hex_public_key: String,
    /// Preferred verified NIP-05 identifier.
    pub preferred_nip05: Option<String>,
    /// Timestamp when the NIP-05 binding was verified.
    pub nip05_verified_at: Option<String>,
    /// Relay hints from the verified NIP-05 document.
    pub nip05_relays: Vec<String>,
    /// Last time this alias row was refreshed.
    pub updated_at: String,
}

/// Vault summary visible to an authenticated actor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VisibleVault {
    /// Stable Vault id.
    pub id: VaultId,
    /// Vault kind.
    pub kind: VaultKind,
    /// Display name.
    pub name: String,
    /// Actor's relationship to this Vault.
    pub role: VisibleVaultRole,
    /// Pending invitation code when the actor has not accepted yet.
    pub invite_code: Option<String>,
}

/// Actor relationship used by client Vault switchers.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VisibleVaultRole {
    /// Personal Vault owner.
    Owner,
    /// Personal Vault's one fully trusted agent.
    PersonalAgent,
    /// Organization Vault admin.
    Admin,
    /// Organization Vault member.
    Member,
    /// Pending Organization Vault invitation.
    Invited,
}

/// Accepted sync record type.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SyncRecordType {
    /// Encrypted Folder Object create/update/move.
    FolderObjectRevision,
    /// Encrypted Folder Object tombstone/delete.
    FolderObjectTombstone,
    /// Folder Key Grant control record.
    FolderKeyGrant,
    /// Vault admin access-change control record.
    VaultAdminAccessChange,
}

/// Folder Object revision sync submission after crypto/signature validation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FolderObjectRevisionSyncRecord {
    /// Signed event id.
    pub record_event_id: String,
    /// Folder id.
    pub folder_id: FolderId,
    /// Object id.
    pub object_id: ObjectId,
    /// New revision.
    pub revision: u64,
    /// Client-observed base revision.
    pub base_revision: Option<u64>,
    /// Actor npub.
    pub actor_npub: UserId,
    /// Client payload timestamp.
    pub client_created_at: String,
    /// Exact encrypted request payload JSON.
    pub payload_json: String,
    /// Signed event kind.
    pub record_event_kind: u16,
}

/// Folder Object tombstone sync submission after crypto/signature validation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FolderObjectTombstoneSyncRecord {
    /// Signed event id.
    pub record_event_id: String,
    /// Folder id.
    pub folder_id: FolderId,
    /// Object id.
    pub object_id: ObjectId,
    /// New tombstone revision.
    pub revision: u64,
    /// Client-observed base revision.
    pub base_revision: u64,
    /// Actor npub.
    pub actor_npub: UserId,
    /// Client payload timestamp.
    pub client_created_at: String,
    /// Exact encrypted tombstone request payload JSON.
    pub payload_json: String,
    /// Signed event kind.
    pub record_event_kind: u16,
}

/// Non-object control record sync submission.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ControlSyncRecord {
    /// Signed event id.
    pub record_event_id: String,
    /// Control record type.
    pub record_type: SyncRecordType,
    /// Optional Folder id.
    pub folder_id: Option<FolderId>,
    /// Actor npub.
    pub actor_npub: UserId,
    /// Client payload timestamp.
    pub client_created_at: String,
    /// Exact control payload JSON.
    pub payload_json: String,
    /// Signed event kind.
    pub record_event_kind: u16,
}

/// Sync record submission.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SyncRecordInput {
    /// Folder Object revision.
    FolderObjectRevision(FolderObjectRevisionSyncRecord),
    /// Folder Object tombstone.
    FolderObjectTombstone(FolderObjectTombstoneSyncRecord),
    /// Control record.
    Control(ControlSyncRecord),
}

/// Result of accepting or retrying a sync record.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SubmitRecordOutcome {
    /// Vault-scoped sequence.
    pub sequence: u64,
    /// True when this event id was already accepted.
    pub duplicate: bool,
}

/// Stored accepted sync record.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredSyncRecord {
    /// Vault-scoped sequence.
    pub sequence: u64,
    /// Signed event id.
    pub record_event_id: String,
    /// Record type.
    pub record_type: SyncRecordType,
    /// Optional Folder id.
    pub folder_id: Option<FolderId>,
    /// Optional object id.
    pub object_id: Option<ObjectId>,
    /// Optional object revision.
    pub revision: Option<u64>,
    /// Actor npub.
    pub actor_npub: UserId,
    /// Client payload timestamp.
    pub client_created_at: String,
    /// Exact submitted payload JSON.
    pub payload_json: String,
    /// Server accepted timestamp.
    pub accepted_at: String,
    /// Signed event kind.
    pub record_event_kind: u16,
}

/// Current encrypted object projection.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CurrentEncryptedObject {
    /// Folder id.
    pub folder_id: FolderId,
    /// Object id.
    pub object_id: ObjectId,
    /// Current encrypted payload JSON.
    pub payload_json: String,
    /// Current revision.
    pub revision: u64,
    /// Projection update timestamp.
    pub updated_at: String,
    /// Whether the current projection is deleted.
    pub deleted: bool,
}

/// Encrypted Vault Export with actor-filtered visibility.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EncryptedVaultExport {
    /// Export version.
    pub version: String,
    /// Vault summary.
    pub vault: ExportVaultSummary,
    /// Folder metadata with actor accessibility.
    pub folders: Vec<EncryptedExportFolder>,
    /// Current encrypted object projection.
    pub objects: Vec<EncryptedExportObject>,
    /// Visible Folder Key Grants.
    pub key_grants: Vec<FolderKeyGrantMetadata>,
    /// Visible access state.
    pub access_state: EncryptedExportAccessState,
}

/// Vault summary in Encrypted Vault Export.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExportVaultSummary {
    /// Vault id.
    pub id: VaultId,
    /// Vault kind.
    pub kind: VaultKind,
    /// Vault name.
    pub name: DisplayName,
    /// Personal Vault owner, if any.
    pub owner_user_id: Option<UserId>,
}

/// Folder export entry.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EncryptedExportFolder {
    /// Folder id.
    pub id: FolderId,
    /// Folder display path.
    pub path: SafeRelativePath,
    /// Access mode.
    pub access: FolderAccessMode,
    /// Current key version.
    pub current_key_version: u32,
    /// Whether this is a Shared Folder Source.
    pub shared_folder_source: bool,
    /// Whether the actor can access current encrypted objects in this Folder.
    pub accessible: bool,
}

/// Object export entry. Inaccessible objects are opaque metadata only.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EncryptedExportObject {
    /// Folder id.
    pub folder_id: FolderId,
    /// Object id.
    pub object_id: ObjectId,
    /// Current encrypted payload JSON when accessible.
    pub payload_json: Option<String>,
    /// Current revision.
    pub revision: u64,
    /// Projection update timestamp.
    pub updated_at: String,
    /// Whether current projection is deleted.
    pub deleted: bool,
    /// True when payload is intentionally withheld.
    pub opaque: bool,
}

/// Actor-visible export access state.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EncryptedExportAccessState {
    /// Visible members.
    pub members: Vec<UserId>,
    /// Visible admins.
    pub admins: Vec<UserId>,
    /// Visible restricted Folder access entries.
    pub folders: Vec<EncryptedExportFolderAccess>,
}

/// Restricted Folder access entry.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EncryptedExportFolderAccess {
    /// Folder id.
    pub folder_id: FolderId,
    /// Visible users.
    pub user_ids: Vec<UserId>,
}

/// Current lifecycle state for Vault Invitations and Share Links.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LinkStatus {
    /// Link can still be accepted.
    Pending,
    /// Link was consumed by the target recipient.
    Accepted,
    /// Link delivery was revoked by an admin.
    Revoked,
}

/// Vault Invitation target routing mode.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VaultInvitationTargetKind {
    /// Existing concrete npub/hex/NIP-05 user target.
    Npub,
    /// Email-targeted bootstrap awaiting client-side claim into an npub.
    EmailBootstrap,
}

impl VaultInvitationTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Npub => "npub",
            Self::EmailBootstrap => "email_bootstrap",
        }
    }
}

impl TryFrom<&str> for VaultInvitationTargetKind {
    type Error = StoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "npub" => Ok(Self::Npub),
            "email_bootstrap" => Ok(Self::EmailBootstrap),
            _ => Err(StoreError::BrokenInvariant {
                reason: format!("unknown vault invitation target kind {value}"),
            }),
        }
    }
}

/// One folder included in an Email Invite Bootstrap authorization scope.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EmailInviteBootstrapScopeFolder {
    /// Folder id.
    pub folder_id: FolderId,
    /// Folder access mode at invite creation.
    pub access: FolderAccessMode,
    /// Exact Folder Key version authorized for bootstrap.
    pub key_version: u32,
}

/// Stored singleton Vault Invitation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredVaultInvitation {
    /// Stable invitation id.
    pub id: String,
    /// Vault id.
    pub vault_id: VaultId,
    /// Target routing mode.
    pub target_kind: VaultInvitationTargetKind,
    /// Target user npub for npub-bound invitations, or claimed npub after email bootstrap claim.
    pub user_id: Option<UserId>,
    /// Invited email for email bootstrap invitations.
    pub invited_email: Option<String>,
    /// Temporary Invite Unwrap npub for encrypted bootstrap material.
    pub invite_unwrap_npub: Option<UserId>,
    /// Server-visible hash of the client-created bootstrap payload.
    pub bootstrap_payload_hash: Option<String>,
    /// NIP-59-wrapped bootstrap payload ciphertext.
    pub bootstrap_wrapped_event_json: Option<String>,
    /// Admin-signed bootstrap authorization event JSON.
    pub bootstrap_authorization_event_json: Option<String>,
    /// Server-visible authorized folder scope and key versions.
    pub bootstrap_scope: Vec<EmailInviteBootstrapScopeFolder>,
    /// Claiming user npub after successful email bootstrap claim.
    pub claimed_by_npub: Option<UserId>,
    /// Lifecycle state.
    pub status: LinkStatus,
    /// Opaque singleton invite code.
    pub invite_code: String,
    /// User-facing accept path.
    pub accept_path: String,
    /// Initial Folder Access metadata only.
    pub initial_folder_access: Vec<FolderId>,
    /// Admin who created the invitation.
    pub created_by_npub: UserId,
    /// Expiry timestamp.
    pub expires_at: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update timestamp.
    pub updated_at: String,
    /// Acceptance timestamp when consumed.
    pub accepted_at: Option<String>,
    /// True when accept returned an already-consumed result for the same target.
    pub duplicate_accept: bool,
}

/// Stored npub-bound singleton Folder Share Link.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredShareLink {
    /// Stable share link id.
    pub id: String,
    /// Source Vault id.
    pub vault_id: VaultId,
    /// Source Folder id.
    pub folder_id: FolderId,
    /// Target user npub.
    pub recipient_npub: UserId,
    /// Admin who created the share link.
    pub created_by_npub: UserId,
    /// Lifecycle state.
    pub status: LinkStatus,
    /// User-facing accept path.
    pub accept_path: String,
    /// Expiry timestamp.
    pub expires_at: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update timestamp.
    pub updated_at: String,
    /// Acceptance timestamp when consumed.
    pub accepted_at: Option<String>,
    /// Folder Key Grant material to insert at accept time.
    pub folder_key_grant: FolderKeyGrantMetadata,
    /// Whether accept should create personal mount state.
    pub create_personal_mount: bool,
    /// Created personal mount id, if requested and accepted.
    pub personal_mount_id: Option<String>,
    /// True when accept returned an already-consumed result for the same target.
    pub duplicate_accept: bool,
}

/// Shared Folder Connection lifecycle state.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SharedFolderConnectionStatus {
    /// Connection is active.
    Active,
    /// Connection delivery/access has been revoked.
    Revoked,
}

/// Stored Shared Folder Invitation from a source Folder to a destination Organization Vault.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredSharedFolderInvitation {
    /// Stable invitation id.
    pub id: String,
    /// Source Vault id.
    pub source_vault_id: VaultId,
    /// Source Folder id.
    pub source_folder_id: FolderId,
    /// Destination Organization Vault id.
    pub destination_vault_id: VaultId,
    /// Destination admin npub.
    pub destination_admin_npub: UserId,
    /// Source admin who created the invitation.
    pub created_by_npub: UserId,
    /// Lifecycle state.
    pub status: LinkStatus,
    /// Source Folder Key version at creation.
    pub current_key_version: u32,
    /// Accept path.
    pub accept_path: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update timestamp.
    pub updated_at: String,
    /// Acceptance timestamp when consumed.
    pub accepted_at: Option<String>,
    /// Folder Key Grant material for the destination admin.
    pub folder_key_grant: FolderKeyGrantMetadata,
    /// True when accept returned an already-consumed result for the destination admin.
    pub duplicate_accept: bool,
}

/// Stored Shared Folder Connection.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredSharedFolderConnection {
    /// Stable deterministic connection id.
    pub id: String,
    /// Source Vault id.
    pub source_vault_id: VaultId,
    /// Source Folder id.
    pub source_folder_id: FolderId,
    /// Destination Organization Vault id.
    pub destination_vault_id: VaultId,
    /// Destination admin npub.
    pub destination_admin_npub: UserId,
    /// Lifecycle state.
    pub status: SharedFolderConnectionStatus,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update timestamp.
    pub updated_at: String,
    /// Participating destination members with source Folder Access.
    pub member_npubs: BTreeSet<UserId>,
}

/// Stored Organization Folder Mount.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredOrganizationFolderMount {
    /// Stable deterministic mount id.
    pub id: String,
    /// Destination Organization Vault id.
    pub organization_vault_id: VaultId,
    /// Source Vault id.
    pub source_vault_id: VaultId,
    /// Source Folder id.
    pub source_folder_id: FolderId,
    /// Connection id.
    pub connection_id: String,
    /// Display name in destination tree.
    pub display_name: String,
    /// Optional destination parent Folder id.
    pub display_parent_folder_id: Option<FolderId>,
    /// Destination admin who accepted/created the mount.
    pub created_by_npub: UserId,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update timestamp.
    pub updated_at: String,
}

/// Direction of a shared-folder relationship relative to one Vault.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SharedFolderDirection {
    /// The Vault is the source that shares one of its Folders.
    Source,
    /// The Vault is the destination that mounts a shared Folder.
    Destination,
}

/// Client-visible mounted Folder projection state.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MountedFolderState {
    /// Actor can open the mounted source Folder.
    Available,
    /// Mount exists, but the actor lacks source access or a current grant.
    Locked,
    /// Source connection has been revoked.
    Revoked,
}

/// Client-visible mounted Folder projection.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MountedFolderProjection {
    /// Organization mount id.
    pub mount_id: String,
    /// Destination Organization Vault id.
    pub organization_vault_id: VaultId,
    /// Source Vault id.
    pub source_vault_id: VaultId,
    /// Source Folder id.
    pub source_folder_id: FolderId,
    /// Connection id.
    pub connection_id: String,
    /// Display name.
    pub display_name: String,
    /// Optional destination parent Folder id.
    pub display_parent_folder_id: Option<FolderId>,
    /// Projection state for the actor.
    pub state: MountedFolderState,
}

struct SharedFolderAccessRemoval<'a> {
    removed_user_ids: &'a BTreeSet<UserId>,
    new_key_version: u32,
    grants: &'a [FolderKeyGrantMetadata],
    reencrypted_records: &'a [FolderObjectRevisionSyncRecord],
    updated_at: &'a str,
}

/// Bootstrap response data for rebuilding current encrypted state.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SyncBootstrap {
    /// Vault id.
    pub vault_id: VaultId,
    /// Latest accepted sequence.
    pub latest_sequence: u64,
    /// Current encrypted objects.
    pub objects: Vec<CurrentEncryptedObject>,
    /// Current control records needed to rebuild readable access state.
    pub control_records: Vec<StoredSyncRecord>,
    /// Object count.
    pub object_count: usize,
    /// Current state kind string.
    pub current_state_kind: &'static str,
}

/// Incremental sync pull result.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SyncPull {
    /// Vault id.
    pub vault_id: VaultId,
    /// Requested cursor.
    pub after_sequence: u64,
    /// Latest sequence at read time.
    pub latest_sequence: u64,
    /// Returned records.
    pub records: Vec<StoredSyncRecord>,
    /// Returned count.
    pub count: usize,
    /// Whether more records are available after `next_sequence`.
    pub has_more: bool,
    /// Cursor to use for the next pull.
    pub next_sequence: u64,
}

/// Narrow SQLite-backed authoritative store.
pub struct BrainStore {
    conn: Connection,
}

impl BrainStore {
    /// Open or create a SQLite store at `path` and apply migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// Open an in-memory SQLite store. Useful for fast unit tests only.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self { conn };
        store.apply_migrations()?;
        Ok(store)
    }

    pub fn load_vault(&self, vault_id: &VaultId) -> Result<StoredVault, StoreError> {
        let mut vault = self.load_core_vault(vault_id)?;
        let folder_access = self.load_folder_access(vault_id)?;
        for member in &mut vault.members {
            member.folder_access = folder_access
                .iter()
                .filter_map(|(folder_id, users)| {
                    users.contains(&member.user_id).then_some(folder_id.clone())
                })
                .collect();
        }

        Ok(StoredVault {
            vault,
            personal_agent: self.load_personal_agent(vault_id)?,
            folder_access,
            grants: self.load_grants(vault_id)?,
            setup_incomplete_folder_ids: self.load_setup_incomplete_folder_ids(vault_id)?,
        })
    }

    /// Upsert verified display metadata for a canonical Nostr identity.
    pub fn record_identity_alias(&mut self, alias: &IdentityAlias) -> Result<(), StoreError> {
        let relays_json = serde_json::to_string(&alias.nip05_relays).map_err(|error| {
            StoreError::InvalidRecord {
                reason: format!("identity alias relays did not serialize: {error}"),
            }
        })?;
        let tx = self.conn.transaction()?;
        if let Some(nip05) = &alias.preferred_nip05 {
            tx.execute(
                "DELETE FROM identity_aliases WHERE preferred_nip05 = ?1 AND npub <> ?2",
                params![nip05, alias.npub.as_str()],
            )?;
            tx.execute(
                r#"
                INSERT INTO identity_aliases (
                    npub, hex_public_key, preferred_nip05, nip05_verified_at,
                    nip05_relays_json, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(npub) DO UPDATE SET
                    hex_public_key = excluded.hex_public_key,
                    preferred_nip05 = excluded.preferred_nip05,
                    nip05_verified_at = excluded.nip05_verified_at,
                    nip05_relays_json = excluded.nip05_relays_json,
                    updated_at = excluded.updated_at
                "#,
                params![
                    alias.npub.as_str(),
                    alias.hex_public_key,
                    nip05,
                    alias.nip05_verified_at,
                    relays_json,
                    alias.updated_at,
                ],
            )?;
        } else {
            tx.execute(
                r#"
                INSERT INTO identity_aliases (
                    npub, hex_public_key, preferred_nip05, nip05_verified_at,
                    nip05_relays_json, updated_at
                ) VALUES (?1, ?2, NULL, NULL, ?3, ?4)
                ON CONFLICT(npub) DO UPDATE SET
                    hex_public_key = excluded.hex_public_key,
                    updated_at = excluded.updated_at
                "#,
                params![
                    alias.npub.as_str(),
                    alias.hex_public_key,
                    relays_json,
                    alias.updated_at,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Load known display metadata for canonical Nostr identities.
    pub fn load_identity_aliases(
        &self,
        npubs: &[UserId],
    ) -> Result<Vec<IdentityAlias>, StoreError> {
        let mut aliases = Vec::new();
        let mut statement = self.conn.prepare(
            r#"
            SELECT npub, hex_public_key, preferred_nip05, nip05_verified_at,
                   nip05_relays_json, updated_at
            FROM identity_aliases
            WHERE npub = ?1
            "#,
        )?;
        for npub in npubs {
            let alias = statement
                .query_row(params![npub.as_str()], identity_alias_from_row)
                .optional()?;
            if let Some(alias) = alias {
                aliases.push(alias);
            }
        }
        Ok(aliases)
    }

    /// Test/support helper for checking rollback behavior without exposing SQL.
    pub fn folder_exists(
        &self,
        vault_id: &VaultId,
        folder_id: &FolderId,
    ) -> Result<bool, StoreError> {
        let exists = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM folders WHERE vault_id = ?1 AND id = ?2)",
            params![vault_id.as_str(), folder_id.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        Ok(exists)
    }

    /// Test/support helper for checking grant rollback behavior without exposing SQL.
    pub fn grant_exists(&self, grant_id: &str) -> Result<bool, StoreError> {
        let exists = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM folder_key_grants WHERE id = ?1)",
            params![grant_id],
            |row| row.get::<_, bool>(0),
        )?;
        Ok(exists)
    }

    /// Accept a validated sync record, assign a Vault-scoped sequence, and update projections.
    pub fn submit_sync_record(
        &mut self,
        vault_id: &VaultId,
        input: &SyncRecordInput,
    ) -> Result<SubmitRecordOutcome, StoreError> {
        self.load_core_vault(vault_id)?;
        sync_records::validate_sync_input(input)?;

        let tx = self.conn.transaction()?;
        if let Some(sequence) =
            sync_records::existing_sequence(&tx, vault_id, input.record_event_id())?
        {
            tx.commit()?;
            return Ok(SubmitRecordOutcome {
                sequence,
                duplicate: true,
            });
        }

        sync_records::validate_sync_conflict(&tx, vault_id, input)?;
        let sequence = sync_records::next_sequence(&tx, vault_id)?;
        sync_records::insert_sync_record(&tx, vault_id, sequence, input)?;
        sync_records::project_sync_record(&tx, vault_id, input)?;
        tx.commit()?;

        Ok(SubmitRecordOutcome {
            sequence,
            duplicate: false,
        })
    }

    /// Return the current encrypted state for rebootstrap.
    pub fn sync_bootstrap(&self, vault_id: &VaultId) -> Result<SyncBootstrap, StoreError> {
        self.require_vault_exists(vault_id)?;
        let objects = self.load_current_objects(vault_id)?;
        let control_records = sync_records::load_sync_records(&self.conn, vault_id)?
            .into_iter()
            .filter(|record| {
                matches!(
                    record.record_type,
                    SyncRecordType::FolderKeyGrant | SyncRecordType::VaultAdminAccessChange
                )
            })
            .collect::<Vec<_>>();
        Ok(SyncBootstrap {
            vault_id: vault_id.clone(),
            latest_sequence: self.latest_sequence(vault_id)?,
            object_count: objects.len(),
            objects,
            control_records,
            current_state_kind: "current_encrypted_vault_state",
        })
    }

    /// Build an actor-filtered Encrypted Vault Export without decrypting content.
    pub fn encrypted_vault_export(
        &self,
        vault_id: &VaultId,
        actor_npub: &UserId,
    ) -> Result<EncryptedVaultExport, StoreError> {
        let stored = self.load_vault(vault_id)?;
        let has_personal_folder_scope = stored.vault.kind != VaultKind::Personal
            || stored.vault.owner_user_id.as_ref() == Some(actor_npub)
            || stored
                .folder_access
                .values()
                .any(|users| users.contains(actor_npub));
        if !vault_visible_to_actor(&stored.vault, actor_npub) || !has_personal_folder_scope {
            return Err(StoreError::BrokenInvariant {
                reason: "vault access required for encrypted export".to_owned(),
            });
        }
        let is_admin = stored.vault.admins.contains(actor_npub);
        let is_limited_personal_member = stored.vault.kind == VaultKind::Personal
            && stored.vault.owner_user_id.as_ref() != Some(actor_npub);
        let folders = stored
            .vault
            .folders
            .iter()
            .filter_map(|folder| {
                let accessible = folder_visible_to_actor(&stored, &folder.id, actor_npub);
                (!is_limited_personal_member || accessible).then(|| EncryptedExportFolder {
                    id: folder.id.clone(),
                    path: folder.path.clone(),
                    access: folder.access,
                    current_key_version: folder.current_key_version,
                    shared_folder_source: folder.shared_folder_source,
                    accessible,
                })
            })
            .collect::<Vec<_>>();
        let objects = self
            .load_current_objects(vault_id)?
            .into_iter()
            .filter_map(|object| {
                let accessible = folder_visible_to_actor(&stored, &object.folder_id, actor_npub);
                (!is_limited_personal_member || accessible).then(|| EncryptedExportObject {
                    folder_id: object.folder_id,
                    object_id: object.object_id,
                    payload_json: accessible.then_some(object.payload_json),
                    revision: object.revision,
                    updated_at: object.updated_at,
                    deleted: object.deleted,
                    opaque: !accessible,
                })
            })
            .collect::<Vec<_>>();
        let key_grants = stored
            .grants
            .iter()
            .filter(|grant| is_admin || grant.recipient_npub == *actor_npub)
            .cloned()
            .collect::<Vec<_>>();
        let access_state = export_access_state(&stored, actor_npub, is_admin);

        Ok(EncryptedVaultExport {
            version: "finite-vault-export-v1".to_owned(),
            vault: ExportVaultSummary {
                id: stored.vault.id,
                kind: stored.vault.kind,
                name: stored.vault.name,
                owner_user_id: stored.vault.owner_user_id,
            },
            folders,
            objects,
            key_grants,
            access_state,
        })
    }

    /// Pull accepted records after a cursor with bounded pagination.
    pub fn pull_sync_records(
        &self,
        vault_id: &VaultId,
        after_sequence: u64,
        limit: u64,
    ) -> Result<SyncPull, StoreError> {
        self.require_vault_exists(vault_id)?;
        let retention_floor = self.retention_floor(vault_id)?;
        if after_sequence < retention_floor {
            return Err(StoreError::RebootstrapRequired { retention_floor });
        }

        let latest_sequence = self.latest_sequence(vault_id)?;
        sync_records::pull_sync_records(
            &self.conn,
            vault_id,
            after_sequence,
            limit,
            latest_sequence,
        )
    }

    /// Set the retained cursor floor for a Vault.
    pub fn set_retention_floor(
        &mut self,
        vault_id: &VaultId,
        retention_floor: u64,
    ) -> Result<(), StoreError> {
        self.require_vault_exists(vault_id)?;
        self.conn.execute(
            r#"
            INSERT INTO vault_sync_retention (vault_id, retention_floor)
            VALUES (?1, ?2)
            ON CONFLICT(vault_id) DO UPDATE SET retention_floor = excluded.retention_floor
            "#,
            params![vault_id.as_str(), retention_floor],
        )?;
        Ok(())
    }

    /// Rebuild current encrypted object projection from the accepted append log.
    pub fn rebuild_current_projection(&mut self, vault_id: &VaultId) -> Result<(), StoreError> {
        self.require_vault_exists(vault_id)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM current_encrypted_vault_objects WHERE vault_id = ?1",
            params![vault_id.as_str()],
        )?;

        let records = sync_records::load_sync_records_tx(&tx, vault_id)?;
        for record in &records {
            sync_records::project_stored_record(&tx, vault_id, record)?;
        }

        tx.commit()?;
        Ok(())
    }

    fn require_vault_exists(&self, vault_id: &VaultId) -> Result<(), StoreError> {
        self.conn
            .query_row(
                "SELECT 1 FROM vaults WHERE id = ?1",
                params![vault_id.as_str()],
                |_| Ok(()),
            )
            .optional()?
            .ok_or_else(|| StoreError::MissingVault {
                vault_id: vault_id.to_string(),
            })
    }

    fn require_organization_vault(&self, vault_id: &VaultId) -> Result<(), StoreError> {
        let vault = self.load_core_vault(vault_id)?;
        if vault.kind != VaultKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "member/admin mutation requires an organization vault".to_owned(),
            });
        }
        Ok(())
    }

    fn member_exists(&self, vault_id: &VaultId, user_id: &UserId) -> Result<bool, StoreError> {
        let exists = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM vault_members WHERE vault_id = ?1 AND user_id = ?2)",
            params![vault_id.as_str(), user_id.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        Ok(exists)
    }

    fn member_has_restricted_access(
        &self,
        vault_id: &VaultId,
        user_id: &UserId,
    ) -> Result<bool, StoreError> {
        let exists = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM folder_access WHERE vault_id = ?1 AND user_id = ?2)",
            params![vault_id.as_str(), user_id.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        Ok(exists)
    }

    fn validate_folder_request(
        &self,
        vault: &Vault,
        folder: &Folder,
        access_user_ids: &BTreeSet<UserId>,
        grants: &[FolderKeyGrantMetadata],
    ) -> Result<(), StoreError> {
        if vault.kind == VaultKind::Personal
            && !matches!(
                folder.access,
                FolderAccessMode::Owner | FolderAccessMode::Restricted
            )
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Vault shared access requires a restricted Folder".to_owned(),
            });
        }
        validate_hierarchy(&self.conn, &vault.id, folder)?;
        validate_access_list_shape(folder, access_user_ids)?;
        validate_access_membership(vault, access_user_ids)?;
        let required = required_recipients(vault, folder, access_user_ids)?;
        validate_folder_grants(vault, folder, &required, grants)
    }

    fn actor_has_current_source_access_and_grant(
        &self,
        source_vault_id: &VaultId,
        source_folder_id: &FolderId,
        actor_npub: &UserId,
    ) -> Result<bool, StoreError> {
        let stored = self.load_vault(source_vault_id)?;
        let Some(folder) = stored
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == *source_folder_id)
        else {
            return Ok(false);
        };
        let has_access = stored.vault.admins.contains(actor_npub)
            || stored
                .folder_access
                .get(source_folder_id)
                .is_some_and(|users| users.contains(actor_npub));
        let has_grant = stored.grants.iter().any(|grant| {
            grant.folder_id == *source_folder_id
                && grant.key_version == folder.current_key_version
                && grant.recipient_npub == *actor_npub
        });
        Ok(has_access && has_grant)
    }

    fn validate_destination_admin_for_connection(
        &self,
        connection: &StoredSharedFolderConnection,
        actor_npub: &UserId,
    ) -> Result<(), StoreError> {
        if connection.status != SharedFolderConnectionStatus::Active {
            return Err(StoreError::UnavailableLink {
                kind: "shared folder connection",
            });
        }
        let destination = self.load_core_vault(&connection.destination_vault_id)?;
        if destination.kind != VaultKind::Organization || !destination.admins.contains(actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "connection member management requires a destination vault admin"
                    .to_owned(),
            });
        }
        Ok(())
    }

    fn validate_destination_member(
        &self,
        destination_vault_id: &VaultId,
        target_npub: &UserId,
    ) -> Result<(), StoreError> {
        let destination = self.load_core_vault(destination_vault_id)?;
        if destination
            .members
            .iter()
            .any(|member| member.user_id == *target_npub)
        {
            Ok(())
        } else {
            Err(StoreError::BrokenInvariant {
                reason: "connection target must be a destination vault member".to_owned(),
            })
        }
    }

    fn rotate_shared_folder_access_removal<F>(
        &mut self,
        connection: &StoredSharedFolderConnection,
        actor_npub: &UserId,
        rotation: SharedFolderAccessRemoval<'_>,
        after_rotation: F,
    ) -> Result<(), StoreError>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<(), StoreError>,
    {
        if rotation.removed_user_ids.is_empty() {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder access removal requires at least one target".to_owned(),
            });
        }
        let stored = self.load_vault(&connection.source_vault_id)?;
        let folder = stored
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == connection.source_folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: connection.source_folder_id.to_string(),
            })?;
        if rotation.new_key_version != folder.current_key_version + 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder access removal must rotate to the next key version"
                    .to_owned(),
            });
        }
        let mut remaining_access = stored
            .folder_access
            .get(&connection.source_folder_id)
            .cloned()
            .unwrap_or_default();
        for removed in rotation.removed_user_ids {
            if !remaining_access.remove(removed) {
                return Err(StoreError::BrokenInvariant {
                    reason: "shared folder removal target does not currently have access"
                        .to_owned(),
                });
            }
        }
        let mut rotated_folder = folder.clone();
        rotated_folder.current_key_version = rotation.new_key_version;
        let required = required_recipients(&stored.vault, &rotated_folder, &remaining_access)?;
        validate_connection_rotation_grants(
            &rotated_folder,
            &required,
            rotation.grants,
            actor_npub,
        )?;
        let live_objects = self
            .load_current_objects(&connection.source_vault_id)?
            .into_iter()
            .filter(|object| object.folder_id == connection.source_folder_id && !object.deleted)
            .collect::<Vec<_>>();
        validate_rotation_records(&live_objects, rotation.reencrypted_records)?;

        let tx = self.conn.transaction()?;
        for removed in rotation.removed_user_ids {
            tx.execute(
                "DELETE FROM folder_access WHERE vault_id = ?1 AND folder_id = ?2 AND user_id = ?3",
                params![
                    connection.source_vault_id.as_str(),
                    connection.source_folder_id.as_str(),
                    removed.as_str()
                ],
            )?;
        }
        tx.execute(
            "UPDATE folders SET current_key_version = ?3 WHERE vault_id = ?1 AND id = ?2",
            params![
                connection.source_vault_id.as_str(),
                connection.source_folder_id.as_str(),
                rotation.new_key_version
            ],
        )?;
        invalidate_pending_email_bootstraps_for_rotated_folder(
            &tx,
            &connection.source_vault_id,
            &connection.source_folder_id,
            rotation.updated_at,
        )?;
        for grant in rotation.grants {
            insert_grant(&tx, &connection.source_vault_id, grant)?;
        }
        for record in rotation.reencrypted_records {
            let input = SyncRecordInput::FolderObjectRevision(record.clone());
            sync_records::validate_sync_input(&input)?;
            sync_records::validate_sync_conflict(&tx, &connection.source_vault_id, &input)?;
            let sequence = sync_records::next_sequence(&tx, &connection.source_vault_id)?;
            sync_records::insert_sync_record(&tx, &connection.source_vault_id, sequence, &input)?;
            sync_records::project_sync_record(&tx, &connection.source_vault_id, &input)?;
        }
        after_rotation(&tx)?;
        tx.commit()?;
        Ok(())
    }
}

impl SyncRecordType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::FolderObjectRevision => "folder_object_revision",
            Self::FolderObjectTombstone => "folder_object_tombstone",
            Self::FolderKeyGrant => "folder_key_grant",
            Self::VaultAdminAccessChange => "vault_admin_access_change",
        }
    }
}

impl TryFrom<&str> for LinkStatus {
    type Error = StoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "accepted" => Ok(Self::Accepted),
            "revoked" => Ok(Self::Revoked),
            _ => Err(StoreError::BrokenInvariant {
                reason: format!("unknown link status: {value}"),
            }),
        }
    }
}

impl TryFrom<&str> for SharedFolderConnectionStatus {
    type Error = StoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            _ => Err(StoreError::BrokenInvariant {
                reason: format!("unknown shared folder connection status: {value}"),
            }),
        }
    }
}

impl TryFrom<&str> for SyncRecordType {
    type Error = StoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "folder_object_revision" => Ok(Self::FolderObjectRevision),
            "folder_object_tombstone" => Ok(Self::FolderObjectTombstone),
            "folder_key_grant" => Ok(Self::FolderKeyGrant),
            "vault_admin_access_change" => Ok(Self::VaultAdminAccessChange),
            _ => Err(StoreError::BrokenInvariant {
                reason: format!("unknown sync record type: {value}"),
            }),
        }
    }
}

impl SyncRecordInput {
    fn record_event_id(&self) -> &str {
        match self {
            Self::FolderObjectRevision(record) => &record.record_event_id,
            Self::FolderObjectTombstone(record) => &record.record_event_id,
            Self::Control(record) => &record.record_event_id,
        }
    }

    fn record_type(&self) -> SyncRecordType {
        match self {
            Self::FolderObjectRevision(_) => SyncRecordType::FolderObjectRevision,
            Self::FolderObjectTombstone(_) => SyncRecordType::FolderObjectTombstone,
            Self::Control(record) => record.record_type,
        }
    }

    fn folder_id(&self) -> Option<&FolderId> {
        match self {
            Self::FolderObjectRevision(record) => Some(&record.folder_id),
            Self::FolderObjectTombstone(record) => Some(&record.folder_id),
            Self::Control(record) => record.folder_id.as_ref(),
        }
    }

    fn object_id(&self) -> Option<&ObjectId> {
        match self {
            Self::FolderObjectRevision(record) => Some(&record.object_id),
            Self::FolderObjectTombstone(record) => Some(&record.object_id),
            Self::Control(_) => None,
        }
    }

    fn revision(&self) -> Option<u64> {
        match self {
            Self::FolderObjectRevision(record) => Some(record.revision),
            Self::FolderObjectTombstone(record) => Some(record.revision),
            Self::Control(_) => None,
        }
    }

    fn actor_npub(&self) -> &UserId {
        match self {
            Self::FolderObjectRevision(record) => &record.actor_npub,
            Self::FolderObjectTombstone(record) => &record.actor_npub,
            Self::Control(record) => &record.actor_npub,
        }
    }

    fn client_created_at(&self) -> &str {
        match self {
            Self::FolderObjectRevision(record) => &record.client_created_at,
            Self::FolderObjectTombstone(record) => &record.client_created_at,
            Self::Control(record) => &record.client_created_at,
        }
    }

    fn payload_json(&self) -> &str {
        match self {
            Self::FolderObjectRevision(record) => &record.payload_json,
            Self::FolderObjectTombstone(record) => &record.payload_json,
            Self::Control(record) => &record.payload_json,
        }
    }

    fn record_event_kind(&self) -> u16 {
        match self {
            Self::FolderObjectRevision(record) => record.record_event_kind,
            Self::FolderObjectTombstone(record) => record.record_event_kind,
            Self::Control(record) => record.record_event_kind,
        }
    }
}

#[derive(Debug)]
struct CurrentObjectRow {
    folder_id: String,
    object_id: String,
    payload_json: String,
    revision: u64,
    updated_at: String,
    deleted: bool,
}

impl CurrentObjectRow {
    fn try_into_current_object(self) -> Result<CurrentEncryptedObject, StoreError> {
        Ok(CurrentEncryptedObject {
            folder_id: FolderId::new(self.folder_id)?,
            object_id: ObjectId::new(self.object_id)?,
            payload_json: self.payload_json,
            revision: self.revision,
            updated_at: self.updated_at,
            deleted: self.deleted,
        })
    }
}

#[derive(Debug)]
struct StoredFolderRow {
    id: String,
    name: String,
    role: String,
    access: String,
    parent_folder_id: Option<String>,
    path: String,
    current_key_version: u32,
    shared_folder_source: bool,
}

impl StoredFolderRow {
    fn try_into_folder(self) -> Result<Folder, StoreError> {
        Ok(Folder {
            id: FolderId::new(self.id)?,
            name: DisplayName::new("folder_name", self.name)?,
            role: parse_folder_role(&self.role)?,
            access: parse_folder_access(&self.access)?,
            parent_folder_id: self.parent_folder_id.map(FolderId::new).transpose()?,
            path: SafeRelativePath::new("folder_path", self.path)?,
            current_key_version: self.current_key_version,
            shared_folder_source: self.shared_folder_source,
        })
    }
}

#[derive(Debug)]
struct StoredGrantRow {
    id: String,
    folder_id: String,
    key_version: u32,
    issuer_npub: String,
    recipient_npub: String,
    format: String,
    wrapped_event_json: String,
    access_change_event_json: Option<String>,
    created_at: String,
}

impl StoredGrantRow {
    fn try_into_grant(self) -> Result<FolderKeyGrantMetadata, StoreError> {
        Ok(FolderKeyGrantMetadata {
            id: self.id,
            folder_id: FolderId::new(self.folder_id)?,
            key_version: self.key_version,
            issuer_npub: UserId::new(self.issuer_npub)?,
            recipient_npub: UserId::new(self.recipient_npub)?,
            format: self.format,
            wrapped_event_json: self.wrapped_event_json,
            access_change_event_json: self.access_change_event_json,
            created_at: self.created_at,
        })
    }
}

fn identity_alias_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IdentityAlias> {
    let relays_json = row.get::<_, String>(4)?;
    let nip05_relays = serde_json::from_str::<Vec<String>>(&relays_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(IdentityAlias {
        npub: UserId::new(row.get::<_, String>(0)?)
            .map_err(to_from_sql_error(0, rusqlite::types::Type::Text))?,
        hex_public_key: row.get(1)?,
        preferred_nip05: row.get(2)?,
        nip05_verified_at: row.get(3)?,
        nip05_relays,
        updated_at: row.get(5)?,
    })
}

fn vault_invitation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredVaultInvitation> {
    let status = row.get::<_, String>(3)?;
    let initial_folder_access_json = row.get::<_, String>(6)?;
    let target_kind = row.get::<_, String>(12)?;
    let bootstrap_scope_json = row.get::<_, String>(19)?;
    Ok(StoredVaultInvitation {
        id: row.get(0)?,
        vault_id: VaultId::new(row.get::<_, String>(1)?)
            .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
        user_id: row
            .get::<_, Option<String>>(2)?
            .map(UserId::new)
            .transpose()
            .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?,
        target_kind: VaultInvitationTargetKind::try_from(target_kind.as_str())
            .map_err(to_store_from_sql_error(12, rusqlite::types::Type::Text))?,
        invited_email: row.get(13)?,
        invite_unwrap_npub: row
            .get::<_, Option<String>>(14)?
            .map(UserId::new)
            .transpose()
            .map_err(to_from_sql_error(14, rusqlite::types::Type::Text))?,
        bootstrap_payload_hash: row.get(15)?,
        bootstrap_wrapped_event_json: row.get(16)?,
        bootstrap_authorization_event_json: row.get(17)?,
        bootstrap_scope: serde_json::from_str(&bootstrap_scope_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                19,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        claimed_by_npub: row
            .get::<_, Option<String>>(18)?
            .map(UserId::new)
            .transpose()
            .map_err(to_from_sql_error(18, rusqlite::types::Type::Text))?,
        status: LinkStatus::try_from(status.as_str())
            .map_err(to_store_from_sql_error(3, rusqlite::types::Type::Text))?,
        invite_code: row.get(4)?,
        accept_path: row.get(5)?,
        initial_folder_access: folder_id_vec_from_json(&initial_folder_access_json)
            .map_err(to_from_sql_error(6, rusqlite::types::Type::Text))?,
        created_by_npub: UserId::new(row.get::<_, String>(7)?)
            .map_err(to_from_sql_error(7, rusqlite::types::Type::Text))?,
        expires_at: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        accepted_at: row.get(11)?,
        duplicate_accept: false,
    })
}

fn share_link_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredShareLink> {
    let status = row.get::<_, String>(5)?;
    let vault_id = VaultId::new(row.get::<_, String>(1)?)
        .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?;
    let folder_id = FolderId::new(row.get::<_, String>(2)?)
        .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?;
    let recipient_npub = UserId::new(row.get::<_, String>(3)?)
        .map_err(to_from_sql_error(3, rusqlite::types::Type::Text))?;
    let created_by_npub = UserId::new(row.get::<_, String>(4)?)
        .map_err(to_from_sql_error(4, rusqlite::types::Type::Text))?;
    Ok(StoredShareLink {
        id: row.get(0)?,
        vault_id: vault_id.clone(),
        folder_id: folder_id.clone(),
        recipient_npub: recipient_npub.clone(),
        created_by_npub: created_by_npub.clone(),
        status: LinkStatus::try_from(status.as_str())
            .map_err(to_store_from_sql_error(5, rusqlite::types::Type::Text))?,
        accept_path: row.get(6)?,
        expires_at: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        accepted_at: row.get(10)?,
        folder_key_grant: FolderKeyGrantMetadata {
            id: row.get(11)?,
            folder_id,
            key_version: row.get(12)?,
            issuer_npub: created_by_npub,
            recipient_npub,
            format: GRANT_FORMAT_NIP59.to_owned(),
            wrapped_event_json: row.get(13)?,
            access_change_event_json: Some(row.get(14)?),
            created_at: row.get(8)?,
        },
        create_personal_mount: row.get(15)?,
        personal_mount_id: row.get(16)?,
        duplicate_accept: false,
    })
}

fn shared_folder_invitation_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<StoredSharedFolderInvitation> {
    let status = row.get::<_, String>(6)?;
    let source_vault_id = VaultId::new(row.get::<_, String>(1)?)
        .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?;
    let source_folder_id = FolderId::new(row.get::<_, String>(2)?)
        .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?;
    let destination_admin_npub = UserId::new(row.get::<_, String>(4)?)
        .map_err(to_from_sql_error(4, rusqlite::types::Type::Text))?;
    let created_by_npub = UserId::new(row.get::<_, String>(5)?)
        .map_err(to_from_sql_error(5, rusqlite::types::Type::Text))?;
    let current_key_version = row.get(7)?;
    Ok(StoredSharedFolderInvitation {
        id: row.get(0)?,
        source_vault_id: source_vault_id.clone(),
        source_folder_id: source_folder_id.clone(),
        destination_vault_id: VaultId::new(row.get::<_, String>(3)?)
            .map_err(to_from_sql_error(3, rusqlite::types::Type::Text))?,
        destination_admin_npub: destination_admin_npub.clone(),
        created_by_npub: created_by_npub.clone(),
        status: LinkStatus::try_from(status.as_str())
            .map_err(to_store_from_sql_error(6, rusqlite::types::Type::Text))?,
        current_key_version,
        accept_path: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        accepted_at: row.get(11)?,
        folder_key_grant: FolderKeyGrantMetadata {
            id: row.get(12)?,
            folder_id: source_folder_id,
            key_version: current_key_version,
            issuer_npub: created_by_npub,
            recipient_npub: destination_admin_npub,
            format: GRANT_FORMAT_NIP59.to_owned(),
            wrapped_event_json: row.get(13)?,
            access_change_event_json: Some(row.get(14)?),
            created_at: row.get(9)?,
        },
        duplicate_accept: false,
    })
}

fn shared_folder_connection_from_row(
    row: &rusqlite::Row<'_>,
    member_npubs: BTreeSet<UserId>,
) -> rusqlite::Result<StoredSharedFolderConnection> {
    let status = row.get::<_, String>(5)?;
    Ok(StoredSharedFolderConnection {
        id: row.get(0)?,
        source_vault_id: VaultId::new(row.get::<_, String>(1)?)
            .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
        source_folder_id: FolderId::new(row.get::<_, String>(2)?)
            .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?,
        destination_vault_id: VaultId::new(row.get::<_, String>(3)?)
            .map_err(to_from_sql_error(3, rusqlite::types::Type::Text))?,
        destination_admin_npub: UserId::new(row.get::<_, String>(4)?)
            .map_err(to_from_sql_error(4, rusqlite::types::Type::Text))?,
        status: SharedFolderConnectionStatus::try_from(status.as_str())
            .map_err(to_store_from_sql_error(5, rusqlite::types::Type::Text))?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        member_npubs,
    })
}

fn organization_mount_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<StoredOrganizationFolderMount> {
    let display_parent_folder_id = row.get::<_, Option<String>>(6)?;
    Ok(StoredOrganizationFolderMount {
        id: row.get(0)?,
        organization_vault_id: VaultId::new(row.get::<_, String>(1)?)
            .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
        source_vault_id: VaultId::new(row.get::<_, String>(2)?)
            .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?,
        source_folder_id: FolderId::new(row.get::<_, String>(3)?)
            .map_err(to_from_sql_error(3, rusqlite::types::Type::Text))?,
        connection_id: row.get(4)?,
        display_name: row.get(5)?,
        display_parent_folder_id: display_parent_folder_id
            .map(FolderId::new)
            .transpose()
            .map_err(to_from_sql_error(6, rusqlite::types::Type::Text))?,
        created_by_npub: UserId::new(row.get::<_, String>(7)?)
            .map_err(to_from_sql_error(7, rusqlite::types::Type::Text))?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn ensure_invitation_available(
    invitation: &StoredVaultInvitation,
    user_id: &UserId,
    now: &str,
) -> Result<(), StoreError> {
    if invitation.target_kind != VaultInvitationTargetKind::Npub
        || invitation.user_id.as_ref() != Some(user_id)
        || invitation.status != LinkStatus::Pending
        || timestamp_expired(&invitation.expires_at, now)
    {
        return Err(StoreError::UnavailableLink {
            kind: "vault invitation",
        });
    }
    Ok(())
}

fn ensure_share_link_available(
    share_link: &StoredShareLink,
    recipient_npub: &UserId,
    now: &str,
) -> Result<(), StoreError> {
    if share_link.recipient_npub != *recipient_npub
        || share_link.status != LinkStatus::Pending
        || timestamp_expired(&share_link.expires_at, now)
    {
        return Err(StoreError::UnavailableLink { kind: "share link" });
    }
    Ok(())
}

fn timestamp_expired(expires_at: &str, now: &str) -> bool {
    !expires_at.is_empty() && expires_at <= now
}

fn validate_link_id(field: &'static str, value: &str) -> Result<(), StoreError> {
    if value.trim().is_empty() || value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(StoreError::BrokenInvariant {
            reason: format!("{field} must be non-empty and printable"),
        });
    }
    Ok(())
}

fn validate_link_timestamp(field: &'static str, value: &str) -> Result<(), StoreError> {
    if value.trim().is_empty() || value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(StoreError::BrokenInvariant {
            reason: format!("{field} must be non-empty and printable"),
        });
    }
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| StoreError::BrokenInvariant {
        reason: format!("{field} must be RFC3339/ISO 8601 UTC timestamp"),
    })?;
    Ok(())
}

fn folder_id_vec_json(folder_ids: &[FolderId]) -> Result<String, StoreError> {
    let values = folder_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    serde_json::to_string(&values).map_err(|error| StoreError::Database {
        message: error.to_string(),
    })
}

fn current_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| MIGRATION_TIMESTAMP.to_owned())
}

fn folder_id_vec_from_json(value: &str) -> Result<Vec<FolderId>, CoreError> {
    serde_json::from_str::<Vec<String>>(value)
        .map_err(|_| CoreError::InvalidId {
            field: "initial_folder_access",
            value: value.to_owned(),
        })?
        .into_iter()
        .map(FolderId::new)
        .collect()
}

fn ensure_folder_exists(
    conn: &Connection,
    vault_id: &VaultId,
    folder_id: &FolderId,
) -> Result<(), StoreError> {
    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE vault_id = ?1 AND id = ?2)",
        params![vault_id.as_str(), folder_id.as_str()],
        |row| row.get::<_, bool>(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::MissingFolder {
            folder_id: folder_id.to_string(),
        })
    }
}

fn insert_member_if_missing(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    user_id: &UserId,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO vault_members (vault_id, user_id) VALUES (?1, ?2)",
        params![vault_id.as_str(), user_id.as_str()],
    )?;
    Ok(())
}

fn insert_folder_access_if_missing(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    folder_id: &FolderId,
    user_id: &UserId,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO folder_access (vault_id, folder_id, user_id) VALUES (?1, ?2, ?3)",
        params![vault_id.as_str(), folder_id.as_str(), user_id.as_str()],
    )?;
    Ok(())
}

fn insert_grant_or_ignore(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    grant: &FolderKeyGrantMetadata,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT OR IGNORE INTO folder_key_grants (
            id, vault_id, folder_id, key_version, issuer_npub, recipient_npub, format,
            wrapped_event_json, access_change_event_json, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![
            grant.id,
            vault_id.as_str(),
            grant.folder_id.as_str(),
            grant.key_version,
            grant.issuer_npub.as_str(),
            grant.recipient_npub.as_str(),
            grant.format,
            grant.wrapped_event_json,
            grant.access_change_event_json,
            grant.created_at
        ],
    )?;
    Ok(())
}

fn personal_mount_id(
    owner_npub: &UserId,
    source_vault_id: &VaultId,
    source_folder_id: &FolderId,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(owner_npub.as_str());
    hasher.update(b"\n");
    hasher.update(source_vault_id.as_str());
    hasher.update(b"\n");
    hasher.update(source_folder_id.as_str());
    let hash = hasher.finalize();
    format!("personal-mount-{}", hex_prefix(&hash, 8))
}

fn hex_prefix(bytes: &[u8], len: usize) -> String {
    bytes
        .iter()
        .take(len)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn to_from_sql_error(
    column: usize,
    value_type: rusqlite::types::Type,
) -> impl FnOnce(CoreError) -> rusqlite::Error {
    move |error| rusqlite::Error::FromSqlConversionFailure(column, value_type, Box::new(error))
}

fn to_store_from_sql_error(
    column: usize,
    value_type: rusqlite::types::Type,
) -> impl FnOnce(StoreError) -> rusqlite::Error {
    move |error| rusqlite::Error::FromSqlConversionFailure(column, value_type, Box::new(error))
}

fn validate_bootstrap_output(output: &BootstrapOutput) -> Result<(), StoreError> {
    validate_loaded_vault(&output.vault)?;
    if output.vault.kind == VaultKind::Organization && output.vault.folders.is_empty() {
        return Err(StoreError::BrokenInvariant {
            reason: "organization bootstrap must create at least one folder".to_owned(),
        });
    }
    Ok(())
}

fn validate_loaded_vault(vault: &Vault) -> Result<(), StoreError> {
    match vault.kind {
        VaultKind::Personal => {
            let Some(owner) = vault.owner_user_id.as_ref() else {
                return Err(StoreError::BrokenInvariant {
                    reason: "personal vault must have an owner".to_owned(),
                });
            };
            if !vault.admins.is_empty()
                || vault.members.iter().any(|member| member.user_id == *owner)
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "personal vault owner is sole admin authority and cannot be an ordinary member"
                        .to_owned(),
                });
            }
        }
        VaultKind::Organization => {
            if vault.owner_user_id.is_some() || vault.admins.is_empty() {
                return Err(StoreError::BrokenInvariant {
                    reason: "organization vault must have admins and no owner".to_owned(),
                });
            }
            let members = vault
                .members
                .iter()
                .map(|member| member.user_id.clone())
                .collect::<BTreeSet<_>>();
            if vault.admins.iter().any(|admin| !members.contains(admin)) {
                return Err(StoreError::BrokenInvariant {
                    reason: "every vault admin must also be a member".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_required_grants(
    vault: &Vault,
    required: &[RequiredFolderKeyGrant],
    grants: &[FolderKeyGrantMetadata],
) -> Result<(), StoreError> {
    let provided = grants
        .iter()
        .map(|grant| {
            (
                grant.folder_id.clone(),
                grant.recipient_npub.clone(),
                grant.key_version,
            )
        })
        .collect::<BTreeSet<_>>();

    for required_grant in required {
        let key = (
            required_grant.folder_id.clone(),
            required_grant.recipient_user_id.clone(),
            required_grant.key_version,
        );
        if !provided.contains(&key) {
            return Err(StoreError::MissingRequiredGrant {
                recipient_user_id: required_grant.recipient_user_id.to_string(),
            });
        }
    }

    if grants.len() != required.len() || provided.len() != required.len() {
        return Err(StoreError::BrokenInvariant {
            reason: "bootstrap grants must exactly match required recipients".to_owned(),
        });
    }

    for grant in grants {
        validate_grant_metadata(grant)?;
        validate_grant_issuer(vault, grant)?;
    }
    Ok(())
}

fn validate_folder_grants(
    vault: &Vault,
    folder: &Folder,
    required_recipients: &BTreeSet<UserId>,
    grants: &[FolderKeyGrantMetadata],
) -> Result<(), StoreError> {
    let mut provided = BTreeSet::new();
    for grant in grants {
        validate_grant_metadata(grant)?;
        validate_grant_issuer(vault, grant)?;
        if grant.folder_id != folder.id {
            return Err(StoreError::BrokenInvariant {
                reason: "grant folder id must match folder metadata".to_owned(),
            });
        }
        if grant.key_version != folder.current_key_version {
            return Err(StoreError::BrokenInvariant {
                reason: "grant key version must match folder current key version".to_owned(),
            });
        }
        provided.insert(grant.recipient_npub.clone());
    }

    for recipient in required_recipients {
        if !provided.contains(recipient) {
            return Err(StoreError::MissingRequiredGrant {
                recipient_user_id: recipient.to_string(),
            });
        }
    }

    if &provided != required_recipients {
        return Err(StoreError::BrokenInvariant {
            reason: "grant recipients must exactly match required recipients".to_owned(),
        });
    }
    Ok(())
}

fn validate_grant_issuer(vault: &Vault, grant: &FolderKeyGrantMetadata) -> Result<(), StoreError> {
    match vault.kind {
        VaultKind::Personal => {
            if vault.owner_user_id.as_ref() != Some(&grant.issuer_npub) {
                return Err(StoreError::BrokenInvariant {
                    reason: "personal vault grants must be issued by the owner".to_owned(),
                });
            }
        }
        VaultKind::Organization => {
            if !vault.admins.contains(&grant.issuer_npub) {
                return Err(StoreError::BrokenInvariant {
                    reason: "organization folder grants must be issued by a vault admin".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_grant_metadata(grant: &FolderKeyGrantMetadata) -> Result<(), StoreError> {
    if grant.id.trim().is_empty() || grant.id.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(StoreError::BrokenInvariant {
            reason: "grant id must be non-empty and printable".to_owned(),
        });
    }
    if grant.format != GRANT_FORMAT_NIP59 {
        return Err(StoreError::BrokenInvariant {
            reason: "folder key grants must use NIP-59 format".to_owned(),
        });
    }
    if grant.wrapped_event_json.trim().is_empty() {
        return Err(StoreError::BrokenInvariant {
            reason: "folder key grant wrapped event JSON is required".to_owned(),
        });
    }
    Ok(())
}

fn canonical_invited_email(value: &str) -> Result<String, StoreError> {
    let value = value.trim().to_ascii_lowercase();
    let Some((local, domain)) = value.split_once('@') else {
        return Err(StoreError::BrokenInvariant {
            reason: "invited email must be an email address".to_owned(),
        });
    };
    if local.is_empty()
        || domain.is_empty()
        || value.chars().any(|c| c == '\0' || c.is_control())
        || value.len() > 320
    {
        return Err(StoreError::BrokenInvariant {
            reason: "invited email must be a printable email address".to_owned(),
        });
    }
    Ok(value)
}

fn validate_required_text(field: &'static str, value: &str) -> Result<(), StoreError> {
    if value.trim().is_empty() || value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(StoreError::BrokenInvariant {
            reason: format!("{field} is required"),
        });
    }
    Ok(())
}

fn email_bootstrap_scope(
    vault: &Vault,
    selected_restricted_folder_access: &[FolderId],
) -> Result<Vec<EmailInviteBootstrapScopeFolder>, StoreError> {
    let selected = selected_restricted_folder_access
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut seen_selected = BTreeSet::new();
    let mut included = BTreeSet::new();
    let mut scope = Vec::new();

    for folder in &vault.folders {
        let selected_folder = selected.contains(&folder.id);
        if selected_folder {
            seen_selected.insert(folder.id.clone());
        }
        let include = match folder.access {
            FolderAccessMode::AllMembers => true,
            FolderAccessMode::Restricted => selected_folder,
            FolderAccessMode::Owner | FolderAccessMode::AdminOnly => {
                if selected_folder {
                    return Err(StoreError::BrokenInvariant {
                        reason:
                            "email bootstrap initial folder access supports all-members and restricted folders only"
                                .to_owned(),
                    });
                }
                false
            }
        };
        if include && included.insert(folder.id.clone()) {
            scope.push(EmailInviteBootstrapScopeFolder {
                folder_id: folder.id.clone(),
                access: folder.access,
                key_version: folder.current_key_version,
            });
        }
    }

    if seen_selected != selected {
        let missing = selected
            .difference(&seen_selected)
            .next()
            .map(ToString::to_string)
            .unwrap_or_else(|| "unknown".to_owned());
        return Err(StoreError::MissingFolder { folder_id: missing });
    }

    Ok(scope)
}

fn validate_email_claim_grants(
    vault: &Vault,
    scope: &[EmailInviteBootstrapScopeFolder],
    claimant: &UserId,
    grants: &[FolderKeyGrantMetadata],
) -> Result<(), StoreError> {
    let required = scope
        .iter()
        .map(|item| (item.folder_id.clone(), item.key_version))
        .collect::<BTreeSet<_>>();
    let provided = grants
        .iter()
        .map(|grant| (grant.folder_id.clone(), grant.key_version))
        .collect::<BTreeSet<_>>();
    if provided != required || grants.len() != scope.len() {
        return Err(StoreError::BrokenInvariant {
            reason: "claim grants must exactly match the email bootstrap scope".to_owned(),
        });
    }

    for item in scope {
        let folder = vault
            .folders
            .iter()
            .find(|folder| folder.id == item.folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: item.folder_id.to_string(),
            })?;
        if folder.current_key_version != item.key_version {
            return Err(StoreError::BrokenInvariant {
                reason: "email bootstrap scope is stale for current Folder Key versions".to_owned(),
            });
        }
    }

    for grant in grants {
        validate_grant_metadata(grant)?;
        if grant.recipient_npub != *claimant || grant.issuer_npub != *claimant {
            return Err(StoreError::BrokenInvariant {
                reason: "claim grants must be issued to and by the claiming npub".to_owned(),
            });
        }
    }

    Ok(())
}

fn email_bootstrap_scope_stale(
    vault: &Vault,
    scope: &[EmailInviteBootstrapScopeFolder],
) -> Result<bool, StoreError> {
    for item in scope {
        let folder = vault
            .folders
            .iter()
            .find(|folder| folder.id == item.folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: item.folder_id.to_string(),
            })?;
        if folder.current_key_version != item.key_version {
            return Ok(true);
        }
    }
    Ok(false)
}

fn invalidate_pending_email_bootstraps_for_rotated_folder(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    folder_id: &FolderId,
    updated_at: &str,
) -> Result<(), StoreError> {
    let mut statement = tx.prepare(
        r#"
        SELECT id, bootstrap_scope_json
        FROM vault_invitations
        WHERE vault_id = ?1
          AND target_kind = 'email_bootstrap'
          AND status = 'pending'
          AND bootstrap_wrapped_event_json IS NOT NULL
        "#,
    )?;
    let invitations = statement
        .query_map(params![vault_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    for (invitation_id, scope_json) in invitations {
        let scope: Vec<EmailInviteBootstrapScopeFolder> = serde_json::from_str(&scope_json)
            .map_err(|error| StoreError::BrokenInvariant {
                reason: format!("stored email bootstrap scope JSON is invalid: {error}"),
            })?;
        if scope.iter().any(|item| item.folder_id == *folder_id) {
            tx.execute(
                r#"
                UPDATE vault_invitations
                SET status = 'revoked',
                    bootstrap_wrapped_event_json = NULL,
                    updated_at = ?2
                WHERE id = ?1
                "#,
                params![invitation_id, updated_at],
            )?;
        }
    }
    Ok(())
}

fn validate_connection_grant(
    grant: &FolderKeyGrantMetadata,
    folder_id: &FolderId,
    current_key_version: u32,
    issuer_npub: &UserId,
    recipient_npub: &UserId,
) -> Result<(), StoreError> {
    validate_grant_metadata(grant)?;
    if grant.folder_id != *folder_id
        || grant.key_version != current_key_version
        || grant.issuer_npub != *issuer_npub
        || grant.recipient_npub != *recipient_npub
    {
        return Err(StoreError::BrokenInvariant {
            reason:
                "connection grant must match folder, current key version, issuer, and recipient"
                    .to_owned(),
        });
    }
    Ok(())
}

fn validate_connection_rotation_grants(
    folder: &Folder,
    required_recipients: &BTreeSet<UserId>,
    grants: &[FolderKeyGrantMetadata],
    issuer_npub: &UserId,
) -> Result<(), StoreError> {
    let mut provided = BTreeSet::new();
    for grant in grants {
        validate_grant_metadata(grant)?;
        if grant.folder_id != folder.id
            || grant.key_version != folder.current_key_version
            || grant.issuer_npub != *issuer_npub
        {
            return Err(StoreError::BrokenInvariant {
                reason: "connection rotation grants must match folder, key version, and issuer"
                    .to_owned(),
            });
        }
        provided.insert(grant.recipient_npub.clone());
    }
    if &provided != required_recipients {
        return Err(StoreError::BrokenInvariant {
            reason: "connection rotation grants must exactly match remaining recipients".to_owned(),
        });
    }
    Ok(())
}

fn validate_access_list_shape(
    folder: &Folder,
    access_user_ids: &BTreeSet<UserId>,
) -> Result<(), StoreError> {
    if folder.access != FolderAccessMode::Restricted && !access_user_ids.is_empty() {
        return Err(StoreError::BrokenInvariant {
            reason: "explicit folder access users are only valid for restricted folders".to_owned(),
        });
    }
    Ok(())
}

fn validate_hierarchy(
    conn: &Connection,
    vault_id: &VaultId,
    folder: &Folder,
) -> Result<(), StoreError> {
    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE vault_id = ?1 AND id = ?2)",
        params![vault_id.as_str(), folder.id.as_str()],
        |row| row.get::<_, bool>(0),
    )?;
    if exists {
        return Err(StoreError::DuplicateId {
            field: "folder_id",
            value: folder.id.to_string(),
        });
    }

    if let Some(parent_id) = &folder.parent_folder_id {
        let parent_exists = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM folders WHERE vault_id = ?1 AND id = ?2)",
            params![vault_id.as_str(), parent_id.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        if !parent_exists {
            return Err(StoreError::MissingFolder {
                folder_id: parent_id.to_string(),
            });
        }
    }

    Ok(())
}

fn validate_access_membership(
    vault: &Vault,
    access_user_ids: &BTreeSet<UserId>,
) -> Result<(), StoreError> {
    let members = vault
        .members
        .iter()
        .map(|member| member.user_id.clone())
        .collect::<BTreeSet<_>>();
    for user_id in access_user_ids {
        if !members.contains(user_id) {
            return Err(StoreError::BrokenInvariant {
                reason: format!("folder access user is not a vault member: {user_id}"),
            });
        }
    }
    Ok(())
}

fn required_recipients(
    vault: &Vault,
    folder: &Folder,
    access_user_ids: &BTreeSet<UserId>,
) -> Result<BTreeSet<UserId>, StoreError> {
    let mut recipients = BTreeSet::new();
    match folder.access {
        FolderAccessMode::Owner => {
            let owner = vault
                .owner_user_id
                .clone()
                .ok_or_else(|| StoreError::BrokenInvariant {
                    reason: "owner access requires a personal vault owner".to_owned(),
                })?;
            recipients.insert(owner);
        }
        FolderAccessMode::AdminOnly => {
            recipients.extend(vault.admins.iter().cloned());
        }
        FolderAccessMode::AllMembers => {
            recipients.extend(vault.admins.iter().cloned());
            recipients.extend(vault.members.iter().map(|member| member.user_id.clone()));
        }
        FolderAccessMode::Restricted => {
            if let Some(owner) = vault.owner_user_id.clone() {
                recipients.insert(owner);
            }
            recipients.extend(vault.admins.iter().cloned());
            recipients.extend(access_user_ids.iter().cloned());
        }
    }

    if recipients.is_empty() {
        return Err(StoreError::BrokenInvariant {
            reason: "current folder key must have at least one recipient".to_owned(),
        });
    }
    Ok(recipients)
}

fn vault_visible_to_actor(vault: &Vault, actor_npub: &UserId) -> bool {
    match vault.kind {
        VaultKind::Personal => {
            vault
                .owner_user_id
                .as_ref()
                .is_some_and(|owner| owner == actor_npub)
                || vault
                    .members
                    .iter()
                    .any(|member| member.user_id == *actor_npub)
        }
        VaultKind::Organization => vault
            .members
            .iter()
            .any(|member| member.user_id == *actor_npub),
    }
}

fn folder_visible_to_actor(
    stored: &StoredVault,
    folder_id: &FolderId,
    actor_npub: &UserId,
) -> bool {
    let Some(folder) = stored
        .vault
        .folders
        .iter()
        .find(|folder| folder.id == *folder_id)
    else {
        return false;
    };
    let is_owner = stored
        .vault
        .owner_user_id
        .as_ref()
        .is_some_and(|owner| owner == actor_npub);
    let is_admin = stored.vault.admins.contains(actor_npub);
    let is_member = stored
        .vault
        .members
        .iter()
        .any(|member| member.user_id == *actor_npub);

    match folder.access {
        FolderAccessMode::Owner => is_owner,
        FolderAccessMode::AdminOnly => is_owner || is_admin,
        FolderAccessMode::AllMembers => {
            is_owner || is_admin || (stored.vault.kind == VaultKind::Organization && is_member)
        }
        FolderAccessMode::Restricted => {
            is_owner
                || is_admin
                || stored
                    .folder_access
                    .get(folder_id)
                    .is_some_and(|users| users.contains(actor_npub))
        }
    }
}

fn export_access_state(
    stored: &StoredVault,
    actor_npub: &UserId,
    is_admin: bool,
) -> EncryptedExportAccessState {
    if is_admin {
        return EncryptedExportAccessState {
            members: stored
                .vault
                .members
                .iter()
                .map(|member| member.user_id.clone())
                .collect(),
            admins: stored.vault.admins.clone(),
            folders: stored
                .folder_access
                .iter()
                .map(|(folder_id, users)| EncryptedExportFolderAccess {
                    folder_id: folder_id.clone(),
                    user_ids: users.iter().cloned().collect(),
                })
                .collect(),
        };
    }

    EncryptedExportAccessState {
        members: stored
            .vault
            .members
            .iter()
            .filter(|member| member.user_id == *actor_npub)
            .map(|member| member.user_id.clone())
            .collect(),
        admins: Vec::new(),
        folders: stored
            .folder_access
            .iter()
            .filter(|(_, users)| users.contains(actor_npub))
            .map(|(folder_id, _)| EncryptedExportFolderAccess {
                folder_id: folder_id.clone(),
                user_ids: vec![actor_npub.clone()],
            })
            .collect(),
    }
}

fn validate_rotation_records(
    live_objects: &[CurrentEncryptedObject],
    reencrypted_records: &[FolderObjectRevisionSyncRecord],
) -> Result<(), StoreError> {
    let live_by_object_id = live_objects
        .iter()
        .map(|object| (object.object_id.clone(), object))
        .collect::<BTreeMap<_, _>>();
    let reencrypted_by_object_id = reencrypted_records
        .iter()
        .map(|record| (record.object_id.clone(), record))
        .collect::<BTreeMap<_, _>>();

    if live_by_object_id.len() != live_objects.len()
        || reencrypted_by_object_id.len() != reencrypted_records.len()
        || live_by_object_id.keys().collect::<Vec<_>>()
            != reencrypted_by_object_id.keys().collect::<Vec<_>>()
    {
        return Err(StoreError::BrokenInvariant {
            reason: "folder key rotation must re-encrypt every live object exactly once".to_owned(),
        });
    }

    for (object_id, live_object) in live_by_object_id {
        let record = reencrypted_by_object_id
            .get(&object_id)
            .expect("object id sets were already checked");
        if record.folder_id != live_object.folder_id
            || record.base_revision != Some(live_object.revision)
            || record.revision != live_object.revision + 1
        {
            return Err(StoreError::BrokenInvariant {
                reason: "folder key rotation records must advance each live object by one revision"
                    .to_owned(),
            });
        }
    }

    Ok(())
}

fn insert_vault(tx: &Transaction<'_>, vault: &Vault) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO vaults (id, kind, name, owner_user_id, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            vault.id.as_str(),
            vault_kind_str(vault.kind),
            vault.name.as_str(),
            vault.owner_user_id.as_ref().map(UserId::as_str),
            current_timestamp()
        ],
    )
    .map_err(map_insert_error("vault_id", vault.id.as_str()))?;
    Ok(())
}

fn insert_members_and_admins(tx: &Transaction<'_>, vault: &Vault) -> Result<(), StoreError> {
    for member in &vault.members {
        tx.execute(
            "INSERT INTO vault_members (vault_id, user_id) VALUES (?1, ?2)",
            params![vault.id.as_str(), member.user_id.as_str()],
        )?;
    }
    for admin in &vault.admins {
        tx.execute(
            "INSERT INTO vault_admins (vault_id, user_id) VALUES (?1, ?2)",
            params![vault.id.as_str(), admin.as_str()],
        )?;
    }
    Ok(())
}

fn insert_folder(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    folder: &Folder,
    setup_incomplete: bool,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO folders (
            vault_id, id, name, role, access, parent_folder_id, parent_folder_key, path,
            current_key_version, shared_folder_source, setup_incomplete, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            vault_id.as_str(),
            folder.id.as_str(),
            folder.name.as_str(),
            folder_role_str(folder.role),
            folder_access_str(folder.access),
            folder.parent_folder_id.as_ref().map(FolderId::as_str),
            folder
                .parent_folder_id
                .as_ref()
                .map_or("", FolderId::as_str),
            folder.path.as_str(),
            folder.current_key_version,
            folder.shared_folder_source,
            setup_incomplete,
            current_timestamp()
        ],
    )
    .map_err(map_insert_error("folder_id", folder.id.as_str()))?;
    Ok(())
}

fn insert_folder_access(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    folder_id: &FolderId,
    access_user_ids: &BTreeSet<UserId>,
) -> Result<(), StoreError> {
    for user_id in access_user_ids {
        tx.execute(
            "INSERT INTO folder_access (vault_id, folder_id, user_id) VALUES (?1, ?2, ?3)",
            params![vault_id.as_str(), folder_id.as_str(), user_id.as_str()],
        )?;
    }
    Ok(())
}

fn insert_grant(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    grant: &FolderKeyGrantMetadata,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO folder_key_grants (
            id, vault_id, folder_id, key_version, issuer_npub, recipient_npub, format,
            wrapped_event_json, access_change_event_json, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![
            grant.id,
            vault_id.as_str(),
            grant.folder_id.as_str(),
            grant.key_version,
            grant.issuer_npub.as_str(),
            grant.recipient_npub.as_str(),
            grant.format,
            grant.wrapped_event_json,
            grant.access_change_event_json,
            grant.created_at
        ],
    )
    .map_err(map_insert_error("folder_key_grant_id", &grant.id))?;
    Ok(())
}

fn map_insert_error(
    field: &'static str,
    value: &str,
) -> impl FnOnce(rusqlite::Error) -> StoreError {
    let value = value.to_owned();
    move |error| match error {
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(inner.code, rusqlite::ErrorCode::ConstraintViolation) =>
        {
            StoreError::DuplicateId { field, value }
        }
        other => StoreError::from(other),
    }
}

fn vault_kind_str(kind: VaultKind) -> &'static str {
    match kind {
        VaultKind::Personal => "personal",
        VaultKind::Organization => "organization",
    }
}

fn parse_vault_kind(value: &str) -> Result<VaultKind, StoreError> {
    match value {
        "personal" => Ok(VaultKind::Personal),
        "organization" => Ok(VaultKind::Organization),
        _ => Err(StoreError::BrokenInvariant {
            reason: format!("unknown vault kind: {value}"),
        }),
    }
}

fn folder_role_str(role: FolderRole) -> &'static str {
    match role {
        FolderRole::PersonalHome => "personal_home",
        FolderRole::VaultOps => "vault_ops",
        FolderRole::General => "general",
        FolderRole::Folder => "folder",
    }
}

fn parse_folder_role(value: &str) -> Result<FolderRole, StoreError> {
    match value {
        "personal_home" => Ok(FolderRole::PersonalHome),
        "vault_ops" => Ok(FolderRole::VaultOps),
        "general" => Ok(FolderRole::General),
        "folder" => Ok(FolderRole::Folder),
        _ => Err(StoreError::BrokenInvariant {
            reason: format!("unknown folder role: {value}"),
        }),
    }
}

fn folder_access_str(access: FolderAccessMode) -> &'static str {
    match access {
        FolderAccessMode::Owner => "owner",
        FolderAccessMode::AdminOnly => "admin_only",
        FolderAccessMode::AllMembers => "all_members",
        FolderAccessMode::Restricted => "restricted",
    }
}

fn parse_folder_access(value: &str) -> Result<FolderAccessMode, StoreError> {
    match value {
        "owner" => Ok(FolderAccessMode::Owner),
        "admin_only" => Ok(FolderAccessMode::AdminOnly),
        "all_members" => Ok(FolderAccessMode::AllMembers),
        "restricted" => Ok(FolderAccessMode::Restricted),
        _ => Err(StoreError::BrokenInvariant {
            reason: format!("unknown folder access mode: {value}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finite_brain_core::{bootstrap_organization_vault, bootstrap_personal_vault};
    use tempfile::TempDir;

    #[test]
    fn exposes_store_crate_name() {
        assert_eq!(crate_name(), "finite-brain-store");
    }

    #[test]
    fn persists_and_reloads_personal_bootstrap() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("vault-sync.sqlite3");
        let output = bootstrap_personal_vault("personal", "Austin", "npub-owner").unwrap();
        let grants = grants_for_required(&output.required_key_grants, "npub-owner");

        {
            let mut store = BrainStore::open(&db).unwrap();
            store.create_vault_bootstrap(&output, &grants).unwrap();
        }

        let store = BrainStore::open(&db).unwrap();
        let stored = store
            .load_vault(&VaultId::new("personal").unwrap())
            .unwrap();

        assert_eq!(stored.vault.kind, VaultKind::Personal);
        assert_eq!(
            stored.vault.owner_user_id,
            Some(UserId::new("npub-owner").unwrap())
        );
        assert!(stored.vault.folders.is_empty());
        assert!(stored.folder_access.is_empty());
        assert!(stored.grants.is_empty());
        assert_same_grants(&stored.grants, &grants);
        assert!(stored.setup_incomplete_folder_ids.is_empty());
    }

    #[test]
    fn persists_and_reloads_organization_bootstrap() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("vault-sync.sqlite3");
        let output = bootstrap_organization_vault("acme", "Acme", "npub-admin").unwrap();
        let grants = grants_for_required(&output.required_key_grants, "npub-admin");

        {
            let mut store = BrainStore::open(&db).unwrap();
            store.create_vault_bootstrap(&output, &grants).unwrap();
        }

        let store = BrainStore::open(&db).unwrap();
        let stored = store.load_vault(&VaultId::new("acme").unwrap()).unwrap();

        assert_eq!(stored.vault.kind, VaultKind::Organization);
        assert_eq!(stored.vault.members.len(), 1);
        assert_eq!(
            stored.vault.admins,
            vec![UserId::new("npub-admin").unwrap()]
        );
        assert_eq!(
            stored
                .vault
                .folders
                .iter()
                .map(|folder| folder.id.to_string())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["getting-started".to_owned(), "restricted".to_owned()])
        );
        let restricted = FolderId::new("restricted").unwrap();
        assert_eq!(
            stored
                .vault
                .folders
                .iter()
                .find(|folder| folder.id == restricted)
                .map(|folder| folder.access),
            Some(FolderAccessMode::Restricted)
        );
        assert!(!stored.folder_access.contains_key(&restricted));
        assert!(stored.grants.iter().any(|grant| {
            grant.folder_id == restricted
                && grant.recipient_npub == UserId::new("npub-admin").unwrap()
        }));
        assert_same_grants(&stored.grants, &grants);
    }

    #[test]
    fn bootstrap_rejects_oversized_batches_before_deep_validation() {
        let mut output = bootstrap_organization_vault("acme", "Acme", "npub-admin").unwrap();
        output.vault.folders = vec![strategy_folder(); MAX_BOOTSTRAP_FOLDERS + 1];
        let mut store = BrainStore::open_in_memory().unwrap();

        assert_eq!(
            store.create_vault_bootstrap(&output, &[]).unwrap_err(),
            StoreError::BrokenInvariant {
                reason: format!("bootstrap folder count exceeds limit {MAX_BOOTSTRAP_FOLDERS}")
            }
        );
    }

    #[test]
    fn creates_restricted_folder_with_required_grants_transactionally() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&vault_id, &member).unwrap();

        let folder = strategy_folder();
        let access_user_ids = BTreeSet::from([member.clone()]);
        let grants = vec![
            grant(
                "grant-strategy-admin",
                "strategy",
                1,
                "npub-admin",
                "npub-admin",
            ),
            grant(
                "grant-strategy-member",
                "strategy",
                1,
                "npub-admin",
                member.as_str(),
            ),
        ];

        store
            .create_folder(&vault_id, &folder, &access_user_ids, &grants)
            .unwrap();
        let stored = store.load_vault(&vault_id).unwrap();

        assert!(stored.vault.folders.iter().any(|stored| stored == &folder));
        assert_eq!(
            stored.folder_access.get(&folder.id),
            Some(&BTreeSet::from([member]))
        );
        for expected_grant in grants {
            assert!(stored.grants.contains(&expected_grant));
        }
    }

    #[test]
    fn grants_restricted_folder_access_with_current_recipient_grant() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&vault_id, &member).unwrap();

        store
            .grant_folder_access(
                &vault_id,
                &FolderId::new("strategy").unwrap(),
                &member,
                &grant(
                    "grant-strategy-member",
                    "strategy",
                    1,
                    "npub-admin",
                    member.as_str(),
                ),
            )
            .unwrap();

        let stored = store.load_vault(&vault_id).unwrap();
        assert_eq!(
            stored
                .folder_access
                .get(&FolderId::new("strategy").unwrap()),
            Some(&BTreeSet::from([member.clone()]))
        );
        assert!(stored.grants.iter().any(|grant| {
            grant.folder_id == FolderId::new("strategy").unwrap()
                && grant.key_version == 1
                && grant.recipient_npub == member
        }));
    }

    #[test]
    fn grants_restricted_folder_key_after_invitation_access_metadata() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let member = UserId::new("npub-invited-member").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        store
            .create_vault_invitation(
                &vault_id,
                "invitation-initial-strategy",
                &member,
                "invite-initial-strategy0123456789ab",
                "/_admin/vault-invitation-links/invite-initial-strategy0123456789ab/accept",
                std::slice::from_ref(&folder_id),
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        store
            .accept_vault_invitation_by_code("invite-initial-strategy0123456789ab", &member, now)
            .unwrap();

        let stored = store.load_vault(&vault_id).unwrap();
        assert_eq!(
            stored.folder_access.get(&folder_id),
            Some(&BTreeSet::from([member.clone()]))
        );
        assert!(!stored.grants.iter().any(|grant| {
            grant.folder_id == folder_id && grant.key_version == 1 && grant.recipient_npub == member
        }));

        store
            .grant_folder_access(
                &vault_id,
                &folder_id,
                &member,
                &grant(
                    "grant-strategy-invited-member",
                    "strategy",
                    1,
                    "npub-admin",
                    member.as_str(),
                ),
            )
            .unwrap();

        let stored = store.load_vault(&vault_id).unwrap();
        assert!(stored.grants.iter().any(|grant| {
            grant.folder_id == folder_id && grant.key_version == 1 && grant.recipient_npub == member
        }));
    }

    #[test]
    fn grants_all_members_folder_key_without_restricted_access_row() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&vault_id, &member).unwrap();

        store
            .grant_folder_access(
                &vault_id,
                &FolderId::new("getting-started").unwrap(),
                &member,
                &grant(
                    "grant-getting-started-member",
                    "getting-started",
                    1,
                    "npub-admin",
                    member.as_str(),
                ),
            )
            .unwrap();

        let stored = store.load_vault(&vault_id).unwrap();
        assert!(
            !stored
                .folder_access
                .contains_key(&FolderId::new("getting-started").unwrap())
        );
        assert!(stored.grants.iter().any(|grant| {
            grant.folder_id == FolderId::new("getting-started").unwrap()
                && grant.key_version == 1
                && grant.recipient_npub == member
        }));
    }

    #[test]
    fn grants_admin_only_folder_key_to_existing_admin_without_access_row() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let admin_only = admin_only_folder();
        store
            .create_folder(
                &vault_id,
                &admin_only,
                &BTreeSet::new(),
                &[grant(
                    "grant-admin-only-admin",
                    "admin-only",
                    1,
                    "npub-admin",
                    "npub-admin",
                )],
            )
            .unwrap();
        let admin = UserId::new("npub-second-admin").unwrap();
        store.add_member(&vault_id, &admin).unwrap();
        store.add_admin(&vault_id, &admin).unwrap();

        store
            .grant_folder_access(
                &vault_id,
                &FolderId::new("admin-only").unwrap(),
                &admin,
                &grant(
                    "grant-admin-only-second-admin",
                    "admin-only",
                    1,
                    "npub-admin",
                    admin.as_str(),
                ),
            )
            .unwrap();

        let stored = store.load_vault(&vault_id).unwrap();
        assert!(
            !stored
                .folder_access
                .contains_key(&FolderId::new("admin-only").unwrap())
        );
        assert!(stored.grants.iter().any(|grant| {
            grant.folder_id == FolderId::new("admin-only").unwrap()
                && grant.key_version == 1
                && grant.recipient_npub == admin
        }));
    }

    #[test]
    fn rejects_admin_only_folder_key_grant_to_non_admin() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let admin_only = admin_only_folder();
        store
            .create_folder(
                &vault_id,
                &admin_only,
                &BTreeSet::new(),
                &[grant(
                    "grant-admin-only-admin",
                    "admin-only",
                    1,
                    "npub-admin",
                    "npub-admin",
                )],
            )
            .unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&vault_id, &member).unwrap();

        assert_eq!(
            store
                .grant_folder_access(
                    &vault_id,
                    &FolderId::new("admin-only").unwrap(),
                    &member,
                    &grant(
                        "grant-admin-only-member",
                        "admin-only",
                        1,
                        "npub-admin",
                        member.as_str(),
                    ),
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "admin-only folder grants require a vault admin target".to_owned()
            }
        );
    }

    #[test]
    fn vault_invitation_is_single_user_single_use_and_retry_safe() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let restricted = FolderId::new("restricted").unwrap();
        let target = UserId::new("npub-target").unwrap();
        let wrong_user = UserId::new("npub-wrong").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        let invitation = store
            .create_vault_invitation(
                &vault_id,
                "invitation-target",
                &target,
                "invite-0123456789abcdef0123456789abcdef",
                "/_admin/vault-invitation-links/invite-0123456789abcdef0123456789abcdef/accept",
                std::slice::from_ref(&restricted),
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        assert_eq!(invitation.status, LinkStatus::Pending);
        assert_eq!(invitation.initial_folder_access, vec![restricted.clone()]);

        assert_eq!(
            store
                .load_available_vault_invitation_by_code(
                    "invite-0123456789abcdef0123456789abcdef",
                    &wrong_user,
                    now,
                )
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "vault invitation"
            }
        );
        assert_eq!(
            store
                .load_available_vault_invitation_by_code(
                    "invite-0123456789abcdef0123456789abcdef",
                    &target,
                    "2026-07-01T00:00:00.000Z",
                )
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "vault invitation"
            }
        );

        let accepted = store
            .accept_vault_invitation_by_code(
                "invite-0123456789abcdef0123456789abcdef",
                &target,
                now,
            )
            .unwrap();
        assert_eq!(accepted.status, LinkStatus::Accepted);
        assert_eq!(accepted.accepted_at.as_deref(), Some(now));
        assert!(!accepted.duplicate_accept);
        let stored = store.load_vault(&vault_id).unwrap();
        assert!(
            stored
                .vault
                .members
                .iter()
                .any(|member| member.user_id == target)
        );
        assert_eq!(
            stored.folder_access.get(&restricted),
            Some(&BTreeSet::from([target.clone()]))
        );

        let retry = store
            .accept_vault_invitation_by_code(
                "invite-0123456789abcdef0123456789abcdef",
                &target,
                now,
            )
            .unwrap();
        assert_eq!(retry.status, LinkStatus::Accepted);
        assert!(retry.duplicate_accept);

        let revoked = store
            .revoke_vault_invitation(&vault_id, "invitation-target", &admin, now)
            .unwrap();
        assert_eq!(revoked.status, LinkStatus::Revoked);
        let stored = store.load_vault(&vault_id).unwrap();
        assert!(
            stored
                .vault
                .members
                .iter()
                .any(|member| member.user_id == target)
        );
    }

    #[test]
    fn email_vault_invitation_claims_membership_access_and_grants_atomically() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let restricted = FolderId::new("restricted").unwrap();
        let getting_started = FolderId::new("getting-started").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let unwrap_npub = UserId::new("npub-unwrap").unwrap();
        let claimant = UserId::new("npub-claimant").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        let invitation = store
            .create_email_vault_invitation(
                &vault_id,
                "invitation-email",
                " Friend@Example.COM ",
                &unwrap_npub,
                "sha256-bootstrap-payload",
                "{\"kind\":1059}",
                "{\"kind\":30078}",
                "invite-email0123456789abcdef012345",
                "/_admin/vault-invitation-links/invite-email0123456789abcdef012345/accept",
                std::slice::from_ref(&restricted),
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();

        assert_eq!(
            invitation.target_kind,
            VaultInvitationTargetKind::EmailBootstrap
        );
        assert_eq!(invitation.user_id, None);
        assert_eq!(
            invitation.invited_email.as_deref(),
            Some("friend@example.com")
        );
        assert_eq!(invitation.invite_unwrap_npub, Some(unwrap_npub.clone()));
        assert_eq!(
            invitation.initial_folder_access,
            vec![getting_started.clone(), restricted.clone()]
        );
        assert_eq!(
            invitation.bootstrap_scope,
            vec![
                EmailInviteBootstrapScopeFolder {
                    folder_id: getting_started.clone(),
                    access: FolderAccessMode::AllMembers,
                    key_version: 1,
                },
                EmailInviteBootstrapScopeFolder {
                    folder_id: restricted.clone(),
                    access: FolderAccessMode::Restricted,
                    key_version: 1,
                },
            ]
        );

        assert_eq!(
            store
                .claim_email_vault_invitation_by_code(
                    "invite-email0123456789abcdef012345",
                    "friend@example.com",
                    &claimant,
                    &[grant(
                        "claim-grant-getting-started",
                        "getting-started",
                        1,
                        "npub-claimant",
                        "npub-claimant",
                    )],
                    now,
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "claim grants must exactly match the email bootstrap scope".to_owned()
            }
        );
        let stored = store.load_vault(&vault_id).unwrap();
        assert!(
            !stored
                .vault
                .members
                .iter()
                .any(|member| member.user_id == claimant)
        );

        let claim_grants = vec![
            grant(
                "claim-grant-getting-started",
                "getting-started",
                1,
                "npub-claimant",
                "npub-claimant",
            ),
            grant(
                "claim-grant-restricted",
                "restricted",
                1,
                "npub-claimant",
                "npub-claimant",
            ),
        ];
        let claimed = store
            .claim_email_vault_invitation_by_code(
                "invite-email0123456789abcdef012345",
                "friend@example.com",
                &claimant,
                &claim_grants,
                now,
            )
            .unwrap();
        assert_eq!(claimed.status, LinkStatus::Accepted);
        assert_eq!(claimed.user_id, Some(claimant.clone()));
        assert_eq!(claimed.claimed_by_npub, Some(claimant.clone()));
        assert_eq!(claimed.bootstrap_wrapped_event_json, None);
        assert!(!claimed.duplicate_accept);

        let stored = store.load_vault(&vault_id).unwrap();
        assert!(
            stored
                .vault
                .members
                .iter()
                .any(|member| member.user_id == claimant)
        );
        assert_eq!(
            stored.folder_access.get(&restricted),
            Some(&BTreeSet::from([claimant.clone()]))
        );
        for grant in claim_grants {
            assert!(stored.grants.contains(&grant));
        }

        let retry = store
            .claim_email_vault_invitation_by_code(
                "invite-email0123456789abcdef012345",
                "friend@example.com",
                &claimant,
                &[],
                now,
            )
            .unwrap();
        assert!(retry.duplicate_accept);
        assert_eq!(
            store
                .claim_email_vault_invitation_by_code(
                    "invite-email0123456789abcdef012345",
                    "friend@example.com",
                    &UserId::new("npub-other-claimant").unwrap(),
                    &[],
                    now,
                )
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "vault invitation"
            }
        );
    }

    #[test]
    fn email_vault_invitation_terminal_states_tombstone_bootstrap_ciphertext() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let restricted = FolderId::new("restricted").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let unwrap_npub = UserId::new("npub-unwrap").unwrap();
        let claimant = UserId::new("npub-claimant").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        let create_invite =
            |store: &mut BrainStore, id: &str, code: &str, email: &str, expires_at: &str| {
                store
                    .create_email_vault_invitation(
                        &vault_id,
                        id,
                        email,
                        &unwrap_npub,
                        "sha256-bootstrap-payload",
                        "{\"kind\":1059}",
                        "{\"kind\":30078}",
                        code,
                        &format!("/_admin/vault-invitation-links/{code}/claim"),
                        std::slice::from_ref(&restricted),
                        &admin,
                        expires_at,
                        now,
                    )
                    .unwrap()
            };

        let revoked = create_invite(
            &mut store,
            "invitation-email-revoked",
            "invite-email-revoked012345678901",
            "revoked@example.com",
            "2026-06-30T00:00:00.000Z",
        );
        store
            .revoke_vault_invitation(&vault_id, &revoked.id, &admin, "2026-06-24T00:00:00.000Z")
            .unwrap();
        assert_eq!(
            store
                .load_vault_invitation(&revoked.id)
                .unwrap()
                .bootstrap_wrapped_event_json,
            None
        );

        let superseded_old = create_invite(
            &mut store,
            "invitation-email-superseded-old",
            "invite-email-supersedeold123456",
            "superseded@example.com",
            "2026-06-30T00:00:00.000Z",
        );
        let superseded_new = create_invite(
            &mut store,
            "invitation-email-superseded-new",
            "invite-email-supersedenew123456",
            "superseded@example.com",
            "2026-06-30T00:00:00.000Z",
        );
        let superseded_old = store.load_vault_invitation(&superseded_old.id).unwrap();
        assert_eq!(superseded_old.status, LinkStatus::Revoked);
        assert_eq!(superseded_old.bootstrap_wrapped_event_json, None);
        assert_eq!(superseded_new.status, LinkStatus::Pending);
        assert!(superseded_new.bootstrap_wrapped_event_json.is_some());

        let expired = create_invite(
            &mut store,
            "invitation-email-expired",
            "invite-email-expired012345678901",
            "expired@example.com",
            "2026-06-22T00:00:00.000Z",
        );
        assert!(matches!(
            store.claim_email_vault_invitation_by_code(
                "invite-email-expired012345678901",
                "expired@example.com",
                &claimant,
                &[],
                now,
            ),
            Err(StoreError::UnavailableLink { .. })
        ));
        assert_eq!(
            store
                .load_vault_invitation(&expired.id)
                .unwrap()
                .bootstrap_wrapped_event_json,
            None
        );

        let stale = create_invite(
            &mut store,
            "invitation-email-stale",
            "invite-email-stale01234567890123",
            "stale@example.com",
            "2026-06-30T00:00:00.000Z",
        );
        store
            .conn
            .execute(
                "UPDATE folders SET current_key_version = 2 WHERE vault_id = ?1 AND id = ?2",
                params![vault_id.as_str(), restricted.as_str()],
            )
            .unwrap();
        assert_eq!(
            store
                .claim_email_vault_invitation_by_code(
                    "invite-email-stale01234567890123",
                    "stale@example.com",
                    &claimant,
                    &[
                        grant(
                            "claim-grant-getting-started-stale",
                            "getting-started",
                            1,
                            "npub-claimant",
                            "npub-claimant",
                        ),
                        grant(
                            "claim-grant-restricted-stale",
                            "restricted",
                            1,
                            "npub-claimant",
                            "npub-claimant",
                        ),
                    ],
                    now,
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "email bootstrap scope is stale for current Folder Key versions".to_owned()
            }
        );
        assert_eq!(
            store
                .load_vault_invitation(&stale.id)
                .unwrap()
                .bootstrap_wrapped_event_json,
            None
        );
    }

    #[test]
    fn folder_key_rotation_invalidates_pending_email_bootstrap() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let restricted = FolderId::new("restricted").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let member = UserId::new("npub-member").unwrap();
        let unwrap_npub = UserId::new("npub-unwrap").unwrap();
        let now = "2026-06-23T00:00:00.000Z";
        store.add_member(&vault_id, &member).unwrap();
        store
            .grant_folder_access(
                &vault_id,
                &restricted,
                &member,
                &grant(
                    "grant-restricted-member-rotation",
                    "restricted",
                    1,
                    "npub-admin",
                    member.as_str(),
                ),
            )
            .unwrap();
        let invitation = store
            .create_email_vault_invitation(
                &vault_id,
                "invitation-email-rotation",
                "rotation@example.com",
                &unwrap_npub,
                "sha256-bootstrap-payload",
                "{\"kind\":1059}",
                "{\"kind\":30078}",
                "invite-email-rotation0123456789",
                "/_admin/vault-invitation-links/invite-email-rotation0123456789/claim",
                std::slice::from_ref(&restricted),
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        assert_eq!(invitation.status, LinkStatus::Pending);
        assert!(invitation.bootstrap_wrapped_event_json.is_some());
        let reencrypted_records = store
            .load_current_objects(&vault_id)
            .unwrap()
            .into_iter()
            .filter(|object| object.folder_id == restricted && !object.deleted)
            .enumerate()
            .map(|(index, object)| FolderObjectRevisionSyncRecord {
                record_event_id: format!("event-email-bootstrap-rotation-{index}"),
                folder_id: object.folder_id,
                object_id: object.object_id,
                revision: object.revision + 1,
                base_revision: Some(object.revision),
                actor_npub: admin.clone(),
                client_created_at: now.to_owned(),
                payload_json: object.payload_json,
                record_event_kind: APP_SPECIFIC_KIND,
            })
            .collect::<Vec<_>>();

        store
            .rotate_folder_key_for_access_removal(
                &vault_id,
                &restricted,
                &member,
                2,
                &[grant(
                    "grant-restricted-admin-v2",
                    "restricted",
                    2,
                    "npub-admin",
                    "npub-admin",
                )],
                &reencrypted_records,
                "2026-06-24T00:00:00.000Z",
            )
            .unwrap();

        let invalidated = store.load_vault_invitation(&invitation.id).unwrap();
        assert_eq!(invalidated.status, LinkStatus::Revoked);
        assert_eq!(invalidated.bootstrap_wrapped_event_json, None);
        assert_eq!(invalidated.updated_at, "2026-06-24T00:00:00.000Z");
    }

    #[test]
    fn vault_invitation_handles_existing_members_without_stale_invites() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let existing_member = UserId::new("npub-existing-member").unwrap();
        let stale_target = UserId::new("npub-stale-target").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        store.add_member(&vault_id, &existing_member).unwrap();
        assert_eq!(
            store
                .create_vault_invitation(
                    &vault_id,
                    "invitation-existing-member",
                    &existing_member,
                    "invite-existing-member0123456789abcdef",
                    "/_admin/vault-invitation-links/invite-existing-member0123456789abcdef/accept",
                    &[],
                    &admin,
                    "2026-06-30T00:00:00.000Z",
                    now,
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "target is already a vault member".to_owned()
            }
        );

        store
            .create_vault_invitation(
                &vault_id,
                "invitation-stale-member",
                &stale_target,
                "invite-stale-member0123456789abcdef",
                "/_admin/vault-invitation-links/invite-stale-member0123456789abcdef/accept",
                &[],
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        store.add_member(&vault_id, &stale_target).unwrap();

        let visible = store.list_visible_vaults(&stale_target).unwrap();
        assert!(visible.iter().any(|vault| vault.id == vault_id));
        assert!(!visible.iter().any(|vault| {
            vault.id == vault_id
                && vault.role == VisibleVaultRole::Invited
                && vault.invite_code.is_some()
        }));

        let accepted = store
            .accept_vault_invitation_by_code(
                "invite-stale-member0123456789abcdef",
                &stale_target,
                now,
            )
            .unwrap();
        assert_eq!(accepted.status, LinkStatus::Accepted);
        assert!(accepted.duplicate_accept);
        assert_eq!(accepted.accepted_at.as_deref(), Some(now));
    }

    #[test]
    fn share_link_accept_creates_member_access_grant_and_optional_mount_once() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let recipient = UserId::new("npub-recipient").unwrap();
        let wrong_user = UserId::new("npub-wrong").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";
        let grant = grant(
            "grant-strategy-recipient",
            "strategy",
            1,
            "npub-admin",
            recipient.as_str(),
        );

        let share_link = store
            .create_share_link(
                &vault_id,
                &folder_id,
                "share-link-recipient",
                &recipient,
                &admin,
                "2026-06-30T00:00:00.000Z",
                "/_admin/share-links/share-link-recipient/accept",
                &grant,
                true,
                now,
            )
            .unwrap();
        assert_eq!(share_link.status, LinkStatus::Pending);
        assert_eq!(share_link.folder_key_grant, grant);

        assert_eq!(
            store
                .load_available_share_link("share-link-recipient", &wrong_user, now)
                .unwrap_err(),
            StoreError::UnavailableLink { kind: "share link" }
        );

        let accepted = store
            .accept_share_link("share-link-recipient", &recipient, now)
            .unwrap();
        assert_eq!(accepted.status, LinkStatus::Accepted);
        assert_eq!(accepted.accepted_at.as_deref(), Some(now));
        assert!(accepted.personal_mount_id.is_some());
        assert!(!accepted.duplicate_accept);

        let stored = store.load_vault(&vault_id).unwrap();
        assert!(
            stored
                .vault
                .members
                .iter()
                .any(|member| member.user_id == recipient)
        );
        assert_eq!(
            stored.folder_access.get(&folder_id),
            Some(&BTreeSet::from([recipient.clone()]))
        );
        assert!(stored.grants.iter().any(|stored_grant| {
            stored_grant.id == "grant-strategy-recipient"
                && stored_grant.recipient_npub == recipient
        }));

        let retry = store
            .accept_share_link("share-link-recipient", &recipient, now)
            .unwrap();
        assert!(retry.duplicate_accept);

        let revoked = store
            .revoke_share_link("share-link-recipient", &admin, now)
            .unwrap();
        assert_eq!(revoked.status, LinkStatus::Revoked);
        let stored = store.load_vault(&vault_id).unwrap();
        assert_eq!(
            stored.folder_access.get(&folder_id),
            Some(&BTreeSet::from([recipient]))
        );
    }

    #[test]
    fn encrypted_vault_export_filters_payloads_grants_and_access_state() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&vault_id, &member).unwrap();
        store
            .create_folder(
                &vault_id,
                &strategy_folder(),
                &BTreeSet::new(),
                &[grant(
                    "grant-strategy-admin",
                    "strategy",
                    1,
                    "npub-admin",
                    "npub-admin",
                )],
            )
            .unwrap();
        store
            .submit_sync_record(
                &vault_id,
                &revision_record_for(
                    "getting-started",
                    "event-getting-started-create",
                    "obj_000000000101",
                    1,
                    None,
                    "getting-started payload",
                ),
            )
            .unwrap();
        store
            .submit_sync_record(
                &vault_id,
                &revision_record_for(
                    "strategy",
                    "event-strategy-create",
                    "obj_000000000102",
                    1,
                    None,
                    "restricted payload",
                ),
            )
            .unwrap();

        let member_export = store.encrypted_vault_export(&vault_id, &member).unwrap();
        assert_eq!(member_export.version, "finite-vault-export-v1");
        assert!(member_export.key_grants.is_empty());
        assert_eq!(member_export.access_state.members, vec![member.clone()]);
        assert!(member_export.access_state.admins.is_empty());
        let getting_started = member_export
            .objects
            .iter()
            .find(|object| object.folder_id == FolderId::new("getting-started").unwrap())
            .unwrap();
        assert!(!getting_started.opaque);
        assert!(
            getting_started
                .payload_json
                .as_ref()
                .unwrap()
                .contains("getting-started")
        );
        let strategy = member_export
            .objects
            .iter()
            .find(|object| object.folder_id == FolderId::new("strategy").unwrap())
            .unwrap();
        assert!(strategy.opaque);
        assert!(strategy.payload_json.is_none());
        assert!(
            !member_export
                .folders
                .iter()
                .find(|folder| folder.id == FolderId::new("strategy").unwrap())
                .unwrap()
                .accessible
        );

        let admin_export = store.encrypted_vault_export(&vault_id, &admin).unwrap();
        assert!(admin_export.key_grants.len() >= 3);
        assert!(admin_export.access_state.admins.contains(&admin));
        assert!(
            admin_export
                .objects
                .iter()
                .find(|object| object.folder_id == FolderId::new("strategy").unwrap())
                .unwrap()
                .payload_json
                .as_ref()
                .unwrap()
                .contains("restricted")
        );
    }

    #[test]
    fn link_timestamps_must_be_rfc3339() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let target = UserId::new("npub-target").unwrap();

        assert_eq!(
            store
                .create_vault_invitation(
                    &vault_id,
                    "invitation-bad-time",
                    &target,
                    "invite-bad-time",
                    "/_admin/vault-invitation-links/invite-bad-time/accept",
                    &[],
                    &admin,
                    "not-a-timestamp",
                    "2026-06-23T00:00:00.000Z",
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "expiresAt must be RFC3339/ISO 8601 UTC timestamp".to_owned()
            }
        );
    }

    #[test]
    fn pending_revoked_and_expired_links_cannot_be_accepted() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";
        let invite_target = UserId::new("npub-invite-target").unwrap();
        store
            .create_vault_invitation(
                &vault_id,
                "invitation-revoked",
                &invite_target,
                "invite-revoked0123456789abcdef012345",
                "/_admin/vault-invitation-links/invite-revoked0123456789abcdef012345/accept",
                &[],
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        store
            .revoke_vault_invitation(&vault_id, "invitation-revoked", &admin, now)
            .unwrap();
        assert_eq!(
            store
                .accept_vault_invitation_by_code(
                    "invite-revoked0123456789abcdef012345",
                    &invite_target,
                    now,
                )
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "vault invitation"
            }
        );

        let expired_target = UserId::new("npub-expired-target").unwrap();
        store
            .create_vault_invitation(
                &vault_id,
                "invitation-expired",
                &expired_target,
                "invite-expired0123456789abcdef012345",
                "/_admin/vault-invitation-links/invite-expired0123456789abcdef012345/accept",
                &[],
                &admin,
                "2026-01-01T00:00:00.000Z",
                now,
            )
            .unwrap();
        assert_eq!(
            store
                .accept_vault_invitation_by_code(
                    "invite-expired0123456789abcdef012345",
                    &expired_target,
                    now,
                )
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "vault invitation"
            }
        );

        let share_recipient = UserId::new("npub-share-revoked").unwrap();
        store
            .create_share_link(
                &vault_id,
                &folder_id,
                "share-link-revoked",
                &share_recipient,
                &admin,
                "2026-06-30T00:00:00.000Z",
                "/_admin/share-links/share-link-revoked/accept",
                &grant(
                    "grant-share-revoked",
                    "strategy",
                    1,
                    "npub-admin",
                    share_recipient.as_str(),
                ),
                false,
                now,
            )
            .unwrap();
        store
            .revoke_share_link("share-link-revoked", &admin, now)
            .unwrap();
        assert_eq!(
            store
                .accept_share_link("share-link-revoked", &share_recipient, now)
                .unwrap_err(),
            StoreError::UnavailableLink { kind: "share link" }
        );
    }

    #[test]
    fn shared_folder_connection_mount_projection_and_delegated_member_rotation() {
        let mut store = store_with_strategy_folder();
        bootstrap_org_named(&mut store, "dest", "Dest", "npub-dest-admin");
        let source_vault_id = VaultId::new("acme").unwrap();
        let source_folder_id = FolderId::new("strategy").unwrap();
        let destination_vault_id = VaultId::new("dest").unwrap();
        let source_admin = UserId::new("npub-admin").unwrap();
        let destination_admin = UserId::new("npub-dest-admin").unwrap();
        let destination_member = UserId::new("npub-dest-member").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        store
            .mark_shared_folder_source(&source_vault_id, &source_folder_id)
            .unwrap();
        let source = store.load_vault(&source_vault_id).unwrap();
        assert!(
            source
                .vault
                .folders
                .iter()
                .find(|folder| folder.id == source_folder_id)
                .unwrap()
                .shared_folder_source
        );

        let invitation = store
            .create_shared_folder_invitation(
                &source_vault_id,
                &source_folder_id,
                &destination_vault_id,
                "shared-folder-invitation-dest",
                &destination_admin,
                &source_admin,
                "/_admin/shared-folder-invitations/shared-folder-invitation-dest/accept",
                &grant(
                    "grant-strategy-dest-admin-v1",
                    "strategy",
                    1,
                    "npub-admin",
                    destination_admin.as_str(),
                ),
                now,
            )
            .unwrap();
        assert_eq!(invitation.status, LinkStatus::Pending);

        let accepted = store
            .accept_shared_folder_invitation(
                "shared-folder-invitation-dest",
                &destination_admin,
                "shared-folder-connection-acme-dest",
                "organization-mount-dest-strategy",
                now,
            )
            .unwrap();
        assert_eq!(accepted.status, LinkStatus::Accepted);
        assert!(!accepted.duplicate_accept);
        let retry = store
            .accept_shared_folder_invitation(
                "shared-folder-invitation-dest",
                &destination_admin,
                "shared-folder-connection-acme-dest",
                "organization-mount-dest-strategy",
                now,
            )
            .unwrap();
        assert_eq!(retry.status, LinkStatus::Accepted);
        assert!(retry.duplicate_accept);
        let connection = store
            .load_shared_folder_connection("shared-folder-connection-acme-dest")
            .unwrap();
        assert_eq!(connection.status, SharedFolderConnectionStatus::Active);
        assert_eq!(
            connection.member_npubs,
            BTreeSet::from([destination_admin.clone()])
        );
        let source = store.load_vault(&source_vault_id).unwrap();
        assert_eq!(
            source.folder_access.get(&source_folder_id),
            Some(&BTreeSet::from([destination_admin.clone()]))
        );
        assert_eq!(
            store
                .mounted_folder_projection(&destination_vault_id, &destination_admin)
                .unwrap()[0]
                .state,
            MountedFolderState::Available
        );

        store
            .add_member(&destination_vault_id, &destination_member)
            .unwrap();
        let connection = store
            .add_shared_folder_connection_member(
                "shared-folder-connection-acme-dest",
                &destination_admin,
                &destination_member,
                &grant(
                    "grant-strategy-dest-member-v1",
                    "strategy",
                    1,
                    destination_admin.as_str(),
                    destination_member.as_str(),
                ),
                now,
            )
            .unwrap();
        assert!(connection.member_npubs.contains(&destination_member));
        assert_eq!(
            store
                .mounted_folder_projection(&destination_vault_id, &destination_member)
                .unwrap()[0]
                .state,
            MountedFolderState::Available
        );

        let connection = store
            .remove_shared_folder_connection_member(
                "shared-folder-connection-acme-dest",
                &destination_admin,
                &destination_member,
                2,
                &[
                    grant(
                        "grant-strategy-source-admin-v2",
                        "strategy",
                        2,
                        destination_admin.as_str(),
                        source_admin.as_str(),
                    ),
                    grant(
                        "grant-strategy-dest-admin-v2",
                        "strategy",
                        2,
                        destination_admin.as_str(),
                        destination_admin.as_str(),
                    ),
                ],
                &[],
                now,
            )
            .unwrap();
        assert!(!connection.member_npubs.contains(&destination_member));
        assert_eq!(
            store
                .mounted_folder_projection(&destination_vault_id, &destination_member)
                .unwrap()[0]
                .state,
            MountedFolderState::Locked
        );

        let connection = store
            .revoke_shared_folder_connection(
                "shared-folder-connection-acme-dest",
                &source_admin,
                3,
                &[grant(
                    "grant-strategy-source-admin-v3",
                    "strategy",
                    3,
                    source_admin.as_str(),
                    source_admin.as_str(),
                )],
                &[],
                now,
            )
            .unwrap();
        assert_eq!(connection.status, SharedFolderConnectionStatus::Revoked);
        assert_eq!(
            store
                .mounted_folder_projection(&destination_vault_id, &destination_admin)
                .unwrap()[0]
                .state,
            MountedFolderState::Revoked
        );
    }

    #[test]
    fn sqlite_full_lifecycle_invite_share_sync_revoke_and_filter_visibility() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("finite-brain.sqlite3");
        let source_vault_id = VaultId::new("acme").unwrap();
        let source_folder_id = FolderId::new("strategy").unwrap();
        let destination_vault_id = VaultId::new("dest").unwrap();
        let source_admin = UserId::new("npub-admin").unwrap();
        let destination_admin = UserId::new("npub-dest-admin").unwrap();
        let destination_member = UserId::new("npub-dest-member").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        {
            let mut store = BrainStore::open(&db).unwrap();
            bootstrap_org_and_strategy_folder(&mut store);
            bootstrap_org_named(&mut store, "dest", "Dest", "npub-dest-admin");

            store
                .create_vault_invitation(
                    &destination_vault_id,
                    "invitation-dest-member",
                    &destination_member,
                    "invite-dest-member",
                    "/_admin/invitations/invitation-dest-member/accept",
                    &[],
                    &destination_admin,
                    "2026-06-30T00:00:00.000Z",
                    now,
                )
                .unwrap();
            store
                .accept_vault_invitation_by_code("invite-dest-member", &destination_member, now)
                .unwrap();

            store
                .mark_shared_folder_source(&source_vault_id, &source_folder_id)
                .unwrap();
            store
                .submit_sync_record(
                    &source_vault_id,
                    &revision_record(
                        "event-lifecycle-create",
                        "obj_000000000101",
                        1,
                        None,
                        "shared",
                    ),
                )
                .unwrap();

            store
                .create_shared_folder_invitation(
                    &source_vault_id,
                    &source_folder_id,
                    &destination_vault_id,
                    "shared-folder-invitation-lifecycle",
                    &destination_admin,
                    &source_admin,
                    "/_admin/shared-folder-invitations/shared-folder-invitation-lifecycle/accept",
                    &grant(
                        "grant-lifecycle-dest-admin-v1",
                        "strategy",
                        1,
                        "npub-admin",
                        destination_admin.as_str(),
                    ),
                    now,
                )
                .unwrap();
            store
                .accept_shared_folder_invitation(
                    "shared-folder-invitation-lifecycle",
                    &destination_admin,
                    "shared-folder-connection-lifecycle",
                    "organization-mount-lifecycle",
                    now,
                )
                .unwrap();
            store
                .add_shared_folder_connection_member(
                    "shared-folder-connection-lifecycle",
                    &destination_admin,
                    &destination_member,
                    &grant(
                        "grant-lifecycle-dest-member-v1",
                        "strategy",
                        1,
                        destination_admin.as_str(),
                        destination_member.as_str(),
                    ),
                    now,
                )
                .unwrap();
        }

        {
            let mut store = BrainStore::open(&db).unwrap();
            let member_projection = store
                .mounted_folder_projection(&destination_vault_id, &destination_member)
                .unwrap();
            assert_eq!(member_projection[0].state, MountedFolderState::Available);

            let member_export = store
                .encrypted_vault_export(&source_vault_id, &destination_member)
                .unwrap();
            let shared_object = member_export
                .objects
                .iter()
                .find(|object| object.folder_id == source_folder_id)
                .unwrap();
            assert_eq!(
                shared_object.payload_json.as_deref(),
                Some("{\"body\":\"shared\"}")
            );
            assert_eq!(
                store
                    .sync_bootstrap(&source_vault_id)
                    .unwrap()
                    .latest_sequence,
                1
            );

            store
                .remove_shared_folder_connection_member(
                    "shared-folder-connection-lifecycle",
                    &destination_admin,
                    &destination_member,
                    2,
                    &[
                        grant(
                            "grant-lifecycle-source-admin-v2",
                            "strategy",
                            2,
                            destination_admin.as_str(),
                            source_admin.as_str(),
                        ),
                        grant(
                            "grant-lifecycle-dest-admin-v2",
                            "strategy",
                            2,
                            destination_admin.as_str(),
                            destination_admin.as_str(),
                        ),
                    ],
                    &[revision_record_struct(
                        "event-lifecycle-reencrypt-member",
                        "strategy",
                        "obj_000000000101",
                        2,
                        Some(1),
                        "shared-v2",
                    )],
                    now,
                )
                .unwrap();
            let locked_projection = store
                .mounted_folder_projection(&destination_vault_id, &destination_member)
                .unwrap();
            assert_eq!(locked_projection[0].state, MountedFolderState::Locked);

            let filtered_export = store
                .encrypted_vault_export(&source_vault_id, &destination_member)
                .unwrap();
            let filtered_object = filtered_export
                .objects
                .iter()
                .find(|object| object.folder_id == source_folder_id)
                .unwrap();
            assert!(filtered_object.payload_json.is_none());
            assert!(filtered_object.opaque);

            store
                .revoke_shared_folder_connection(
                    "shared-folder-connection-lifecycle",
                    &source_admin,
                    3,
                    &[grant(
                        "grant-lifecycle-source-admin-v3",
                        "strategy",
                        3,
                        source_admin.as_str(),
                        source_admin.as_str(),
                    )],
                    &[revision_record_struct(
                        "event-lifecycle-reencrypt-admin",
                        "strategy",
                        "obj_000000000101",
                        3,
                        Some(2),
                        "shared-v3",
                    )],
                    now,
                )
                .unwrap();
            let revoked_projection = store
                .mounted_folder_projection(&destination_vault_id, &destination_admin)
                .unwrap();
            assert_eq!(revoked_projection[0].state, MountedFolderState::Revoked);
            assert_eq!(
                store
                    .sync_bootstrap(&source_vault_id)
                    .unwrap()
                    .latest_sequence,
                3
            );
        }
    }

    #[test]
    fn removing_restricted_folder_access_requires_rotation_and_reencrypts_live_objects() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&vault_id, &member).unwrap();
        store
            .grant_folder_access(
                &vault_id,
                &folder_id,
                &member,
                &grant(
                    "grant-strategy-member",
                    "strategy",
                    1,
                    "npub-admin",
                    member.as_str(),
                ),
            )
            .unwrap();
        store
            .submit_sync_record(
                &vault_id,
                &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
            )
            .unwrap();

        store
            .rotate_folder_key_for_access_removal(
                &vault_id,
                &folder_id,
                &member,
                2,
                &[grant(
                    "grant-strategy-admin-v2",
                    "strategy",
                    2,
                    "npub-admin",
                    "npub-admin",
                )],
                &[revision_record_struct(
                    "event-reencrypt-1",
                    "strategy",
                    "obj_000000000001",
                    2,
                    Some(1),
                    "reencrypted",
                )],
                "2026-06-23T00:00:00.000Z",
            )
            .unwrap();

        let stored = store.load_vault(&vault_id).unwrap();
        let folder = stored
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .unwrap();
        assert_eq!(folder.current_key_version, 2);
        assert_eq!(
            stored
                .folder_access
                .get(&folder_id)
                .cloned()
                .unwrap_or_default(),
            BTreeSet::new()
        );
        assert!(stored.grants.iter().any(|grant| {
            grant.folder_id == folder_id
                && grant.key_version == 2
                && grant.recipient_npub.as_str() == "npub-admin"
        }));

        let bootstrap = store.sync_bootstrap(&vault_id).unwrap();
        assert_eq!(bootstrap.latest_sequence, 2);
        assert_eq!(bootstrap.objects[0].revision, 2);
        assert_eq!(
            bootstrap.objects[0].payload_json,
            "{\"body\":\"reencrypted\"}"
        );
    }

    #[test]
    fn access_removal_rotation_rolls_back_when_reencryption_or_grants_are_incomplete() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&vault_id, &member).unwrap();
        store
            .grant_folder_access(
                &vault_id,
                &folder_id,
                &member,
                &grant(
                    "grant-strategy-member",
                    "strategy",
                    1,
                    "npub-admin",
                    member.as_str(),
                ),
            )
            .unwrap();
        store
            .submit_sync_record(
                &vault_id,
                &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
            )
            .unwrap();

        assert_eq!(
            store
                .rotate_folder_key_for_access_removal(
                    &vault_id,
                    &folder_id,
                    &member,
                    2,
                    &[grant(
                        "grant-strategy-admin-v2",
                        "strategy",
                        2,
                        "npub-admin",
                        "npub-admin",
                    )],
                    &[],
                    "2026-06-23T00:00:00.000Z",
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "folder key rotation must re-encrypt every live object exactly once"
                    .to_owned()
            }
        );

        assert_eq!(
            store
                .rotate_folder_key_for_access_removal(
                    &vault_id,
                    &folder_id,
                    &member,
                    2,
                    &[grant(
                        "grant-strategy-admin",
                        "strategy",
                        2,
                        "npub-admin",
                        "npub-admin",
                    )],
                    &[revision_record_struct(
                        "event-reencrypt-1",
                        "strategy",
                        "obj_000000000001",
                        2,
                        Some(1),
                        "reencrypted",
                    )],
                    "2026-06-23T00:00:00.000Z",
                )
                .unwrap_err(),
            StoreError::DuplicateId {
                field: "folder_key_grant_id",
                value: "grant-strategy-admin".to_owned()
            }
        );

        let stored = store.load_vault(&vault_id).unwrap();
        let folder = stored
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .unwrap();
        assert_eq!(folder.current_key_version, 1);
        assert_eq!(
            stored.folder_access.get(&folder_id),
            Some(&BTreeSet::from([member]))
        );
        assert_eq!(store.sync_bootstrap(&vault_id).unwrap().latest_sequence, 1);
    }

    #[test]
    fn rejects_missing_required_grant_without_partial_folder() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&vault_id, &member).unwrap();

        let folder = strategy_folder();
        let access_user_ids = BTreeSet::from([member]);
        let grants = vec![grant(
            "grant-strategy-admin",
            "strategy",
            1,
            "npub-admin",
            "npub-admin",
        )];

        assert_eq!(
            store
                .create_folder(&vault_id, &folder, &access_user_ids, &grants)
                .unwrap_err(),
            StoreError::MissingRequiredGrant {
                recipient_user_id: "npub-member".to_owned()
            }
        );
        assert!(!store.folder_exists(&vault_id, &folder.id).unwrap());
    }

    #[test]
    fn rolls_back_folder_creation_when_grant_insert_fails() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        assert!(
            store
                .grant_exists("grant-getting-started-npub-admin")
                .unwrap()
        );

        let folder = strategy_folder();
        let grants = vec![grant(
            "grant-getting-started-npub-admin",
            "strategy",
            1,
            "npub-admin",
            "npub-admin",
        )];

        assert!(matches!(
            store
                .create_folder(&vault_id, &folder, &BTreeSet::new(), &grants)
                .unwrap_err(),
            StoreError::DuplicateId {
                field: "folder_key_grant_id",
                ..
            }
        ));
        assert!(!store.folder_exists(&vault_id, &folder.id).unwrap());
    }

    #[test]
    fn detects_and_repairs_setup_incomplete_folder_across_restart() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("vault-sync.sqlite3");
        let vault_id = VaultId::new("acme").unwrap();
        let folder = strategy_folder();
        let grants = vec![grant(
            "grant-strategy-admin",
            "strategy",
            1,
            "npub-admin",
            "npub-admin",
        )];

        {
            let mut store = BrainStore::open(&db).unwrap();
            let output = bootstrap_organization_vault("acme", "Acme", "npub-admin").unwrap();
            let bootstrap_grants = grants_for_required(&output.required_key_grants, "npub-admin");
            store
                .create_vault_bootstrap(&output, &bootstrap_grants)
                .unwrap();
            store
                .insert_setup_incomplete_folder_for_repair(&vault_id, &folder, &BTreeSet::new())
                .unwrap();
        }

        {
            let mut store = BrainStore::open(&db).unwrap();
            let stored = store.load_vault(&vault_id).unwrap();
            assert_eq!(
                stored.setup_incomplete_folder_ids,
                BTreeSet::from([folder.id.clone()])
            );

            store
                .finish_folder_setup(&vault_id, &folder.id, &grants)
                .unwrap();
        }

        let store = BrainStore::open(&db).unwrap();
        let stored = store.load_vault(&vault_id).unwrap();
        assert!(stored.setup_incomplete_folder_ids.is_empty());
        assert!(stored.grants.contains(&grants[0]));
    }

    #[test]
    fn finish_setup_rejects_non_empty_setup_incomplete_folder() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let folder = strategy_folder();
        store
            .insert_setup_incomplete_folder_for_repair(&vault_id, &folder, &BTreeSet::new())
            .unwrap();
        store
            .submit_sync_record(
                &vault_id,
                &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
            )
            .unwrap();

        assert_eq!(
            store
                .finish_folder_setup(
                    &vault_id,
                    &folder.id,
                    &[grant(
                        "grant-strategy-admin",
                        "strategy",
                        1,
                        "npub-admin",
                        "npub-admin",
                    )],
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "finish setup only supports empty folders".to_owned()
            }
        );
    }

    #[test]
    fn rejects_invalid_hierarchy_duplicate_ids_and_admin_invariants() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();

        let mut missing_parent = strategy_folder();
        missing_parent.parent_folder_id = Some(FolderId::new("missing").unwrap());
        missing_parent.path = SafeRelativePath::new("folder_path", "Missing/Strategy").unwrap();
        assert_eq!(
            store
                .create_folder(
                    &vault_id,
                    &missing_parent,
                    &BTreeSet::new(),
                    &[grant(
                        "grant-missing-parent",
                        "strategy",
                        1,
                        "npub-admin",
                        "npub-admin"
                    )],
                )
                .unwrap_err(),
            StoreError::MissingFolder {
                folder_id: "missing".to_owned()
            }
        );

        let folder = strategy_folder();
        let grants = vec![grant(
            "grant-strategy-admin",
            "strategy",
            1,
            "npub-admin",
            "npub-admin",
        )];
        store
            .create_folder(&vault_id, &folder, &BTreeSet::new(), &grants)
            .unwrap();
        assert_eq!(
            store
                .create_folder(
                    &vault_id,
                    &folder,
                    &BTreeSet::new(),
                    &[grant(
                        "grant-strategy-admin-2",
                        "strategy",
                        1,
                        "npub-admin",
                        "npub-admin"
                    )],
                )
                .unwrap_err(),
            StoreError::DuplicateId {
                field: "folder_id",
                value: "strategy".to_owned()
            }
        );

        assert_eq!(
            store
                .add_admin(&vault_id, &UserId::new("npub-non-member").unwrap())
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "vault admin must already be a vault member".to_owned()
            }
        );

        let bad_issuer_folder = Folder {
            id: FolderId::new("bad-issuer-strategy").unwrap(),
            name: DisplayName::new("folder_name", "Bad Issuer Strategy").unwrap(),
            path: SafeRelativePath::new("folder_path", "getting-started/Bad Issuer Strategy")
                .unwrap(),
            ..strategy_folder()
        };
        assert_eq!(
            store
                .create_folder(
                    &vault_id,
                    &bad_issuer_folder,
                    &BTreeSet::new(),
                    &[grant(
                        "grant-bad-issuer",
                        "bad-issuer-strategy",
                        1,
                        "npub-non-admin",
                        "npub-admin"
                    )],
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "organization folder grants must be issued by a vault admin".to_owned()
            }
        );
        assert!(
            !store
                .folder_exists(&vault_id, &bad_issuer_folder.id)
                .unwrap()
        );
    }

    #[test]
    fn rejects_unscoped_personal_member_mutation() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_vault("personal", "Austin", "npub-owner").unwrap();
        let grants = grants_for_required(&output.required_key_grants, "npub-owner");
        store.create_vault_bootstrap(&output, &grants).unwrap();
        let vault_id = VaultId::new("personal").unwrap();
        let member = UserId::new("npub-member").unwrap();

        assert_eq!(
            store.add_member(&vault_id, &member).unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "member/admin mutation requires an organization vault".to_owned()
            }
        );
    }

    #[test]
    fn personal_member_is_removed_when_their_last_folder_scope_is_removed() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_vault("personal", "Austin", "npub-owner").unwrap();
        let grants = grants_for_required(&output.required_key_grants, "npub-owner");
        store.create_vault_bootstrap(&output, &grants).unwrap();
        let vault_id = VaultId::new("personal").unwrap();
        let member = UserId::new("npub-member").unwrap();
        let folder = Folder {
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Strategy").unwrap(),
            ..strategy_folder()
        };
        store
            .create_folder(
                &vault_id,
                &folder,
                &BTreeSet::from([member.clone()]),
                &[
                    grant(
                        "grant-personal-strategy-owner",
                        "strategy",
                        1,
                        "npub-owner",
                        "npub-owner",
                    ),
                    grant(
                        "grant-personal-strategy-member",
                        "strategy",
                        1,
                        "npub-owner",
                        member.as_str(),
                    ),
                ],
            )
            .unwrap();

        store
            .rotate_folder_key_for_access_removal(
                &vault_id,
                &folder.id,
                &member,
                2,
                &[grant(
                    "grant-personal-strategy-owner-v2",
                    "strategy",
                    2,
                    "npub-owner",
                    "npub-owner",
                )],
                &[],
                "2026-07-13T00:00:00.000Z",
            )
            .unwrap();

        let stored = store.load_vault(&vault_id).unwrap();
        assert!(
            !stored
                .vault
                .members
                .iter()
                .any(|stored_member| stored_member.user_id == member)
        );
        assert!(store.list_visible_vaults(&member).unwrap().is_empty());
    }

    #[test]
    fn removes_members_and_admins_without_breaking_admin_invariant() {
        let mut store = bootstrapped_org_store();
        let vault_id = VaultId::new("acme").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&vault_id, &member).unwrap();
        store.add_admin(&vault_id, &member).unwrap();

        store.remove_admin(&vault_id, &member).unwrap();
        assert_eq!(
            store
                .remove_admin(&vault_id, &UserId::new("npub-admin").unwrap())
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "organization vault must keep at least one admin".to_owned()
            }
        );

        store.remove_member(&vault_id, &member).unwrap();
        let stored = store.load_vault(&vault_id).unwrap();
        assert!(
            !stored
                .vault
                .members
                .iter()
                .any(|stored| stored.user_id == member)
        );
    }

    #[test]
    fn removing_member_requires_admin_and_restricted_access_cleanup_first() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        assert_eq!(
            store.remove_member(&vault_id, &admin).unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "remove admin role before removing member".to_owned()
            }
        );

        let member = UserId::new("npub-member").unwrap();
        store.add_member(&vault_id, &member).unwrap();
        store
            .grant_folder_access(
                &vault_id,
                &FolderId::new("strategy").unwrap(),
                &member,
                &grant(
                    "grant-strategy-member",
                    "strategy",
                    1,
                    "npub-admin",
                    member.as_str(),
                ),
            )
            .unwrap();

        assert_eq!(
            store.remove_member(&vault_id, &member).unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "remove restricted folder access before removing member".to_owned()
            }
        );
    }

    #[test]
    fn sync_create_update_and_delete_updates_current_projection() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let object_id = "obj_000000000001";

        assert_eq!(
            store
                .submit_sync_record(
                    &vault_id,
                    &revision_record("event-create-1", object_id, 1, None, "create")
                )
                .unwrap(),
            SubmitRecordOutcome {
                sequence: 1,
                duplicate: false
            }
        );
        assert_eq!(
            store
                .submit_sync_record(
                    &vault_id,
                    &revision_record("event-update-1", object_id, 2, Some(1), "update")
                )
                .unwrap()
                .sequence,
            2
        );
        assert_eq!(
            store
                .submit_sync_record(
                    &vault_id,
                    &tombstone_record("event-delete-1", object_id, 3, 2)
                )
                .unwrap()
                .sequence,
            3
        );

        let bootstrap = store.sync_bootstrap(&vault_id).unwrap();
        assert_eq!(bootstrap.latest_sequence, 3);
        assert_eq!(bootstrap.object_count, 1);
        assert_eq!(bootstrap.objects[0].revision, 3);
        assert!(bootstrap.objects[0].deleted);
        assert_eq!(bootstrap.objects[0].payload_json, "{\"body\":\"delete\"}");
    }

    #[test]
    fn sync_duplicate_event_returns_existing_sequence() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let record = revision_record("event-create-duplicate", "obj_000000000001", 1, None, "one");

        assert_eq!(
            store.submit_sync_record(&vault_id, &record).unwrap(),
            SubmitRecordOutcome {
                sequence: 1,
                duplicate: false
            }
        );
        assert_eq!(
            store.submit_sync_record(&vault_id, &record).unwrap(),
            SubmitRecordOutcome {
                sequence: 1,
                duplicate: true
            }
        );

        let pull = store.pull_sync_records(&vault_id, 0, 10).unwrap();
        assert_eq!(pull.count, 1);
        assert_eq!(pull.latest_sequence, 1);
    }

    #[test]
    fn sync_rejects_stale_base_revision_and_existing_create() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let object_id = "obj_000000000001";

        store
            .submit_sync_record(
                &vault_id,
                &revision_record("event-create-1", object_id, 1, None, "create"),
            )
            .unwrap();
        store
            .submit_sync_record(
                &vault_id,
                &revision_record("event-update-wins", object_id, 2, Some(1), "winner"),
            )
            .unwrap();

        assert_eq!(
            store
                .submit_sync_record(
                    &vault_id,
                    &revision_record("event-update-loses", object_id, 2, Some(1), "loser"),
                )
                .unwrap_err(),
            StoreError::Conflict {
                reason: "baseRevision does not match current folder object revision".to_owned(),
                current_revision: Some(2)
            }
        );
        assert_eq!(
            store
                .submit_sync_record(
                    &vault_id,
                    &revision_record("event-create-again", object_id, 1, None, "again"),
                )
                .unwrap_err(),
            StoreError::Conflict {
                reason: "object already exists".to_owned(),
                current_revision: Some(2)
            }
        );
        assert_eq!(store.sync_bootstrap(&vault_id).unwrap().latest_sequence, 2);
    }

    #[test]
    fn sync_rejects_non_monotonic_revision() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        let object_id = "obj_000000000001";

        store
            .submit_sync_record(
                &vault_id,
                &revision_record("event-create-1", object_id, 1, None, "create"),
            )
            .unwrap();

        assert_eq!(
            store
                .submit_sync_record(
                    &vault_id,
                    &revision_record("event-update-bad", object_id, 3, Some(1), "bad"),
                )
                .unwrap_err(),
            StoreError::InvalidRecord {
                reason: "revision must advance baseRevision by one".to_owned()
            }
        );
    }

    #[test]
    fn sync_pull_paginates_with_next_sequence() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();

        for (index, object_id) in ["obj_000000000001", "obj_000000000002", "obj_000000000003"]
            .into_iter()
            .enumerate()
        {
            store
                .submit_sync_record(
                    &vault_id,
                    &revision_record(
                        &format!("event-create-page-{index}"),
                        object_id,
                        1,
                        None,
                        object_id,
                    ),
                )
                .unwrap();
        }

        let first = store.pull_sync_records(&vault_id, 0, 2).unwrap();
        assert_eq!(first.count, 2);
        assert!(first.has_more);
        assert_eq!(first.next_sequence, 2);
        assert_eq!(first.latest_sequence, 3);

        let second = store
            .pull_sync_records(&vault_id, first.next_sequence, 2)
            .unwrap();
        assert_eq!(second.count, 1);
        assert!(!second.has_more);
        assert_eq!(second.next_sequence, 3);
        assert_eq!(second.records[0].sequence, 3);
    }

    #[test]
    fn sync_pull_caps_large_client_limits() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();

        for index in 1..=(MAX_PULL_LIMIT + 2) {
            let object_id = format!("obj_{index:012}");
            store
                .submit_sync_record(
                    &vault_id,
                    &revision_record(
                        &format!("event-capped-page-{index}"),
                        &object_id,
                        1,
                        None,
                        &object_id,
                    ),
                )
                .unwrap();
        }

        let pull = store.pull_sync_records(&vault_id, 0, u64::MAX).unwrap();
        assert_eq!(pull.count, MAX_PULL_LIMIT as usize);
        assert!(pull.has_more);
        assert_eq!(pull.next_sequence, MAX_PULL_LIMIT);
        assert_eq!(pull.latest_sequence, MAX_PULL_LIMIT + 2);
    }

    #[test]
    fn sync_cursor_expiry_requires_rebootstrap() {
        let mut store = store_with_strategy_folder();
        let vault_id = VaultId::new("acme").unwrap();
        store
            .submit_sync_record(
                &vault_id,
                &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
            )
            .unwrap();
        store.set_retention_floor(&vault_id, 1).unwrap();

        assert_eq!(
            store.pull_sync_records(&vault_id, 0, 10).unwrap_err(),
            StoreError::RebootstrapRequired { retention_floor: 1 }
        );
        assert_eq!(store.pull_sync_records(&vault_id, 1, 10).unwrap().count, 0);
    }

    #[test]
    fn sync_projection_survives_restart_and_can_rebuild() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("vault-sync.sqlite3");
        let vault_id = VaultId::new("acme").unwrap();

        {
            let mut store = BrainStore::open(&db).unwrap();
            bootstrap_org_and_strategy_folder(&mut store);
            store
                .submit_sync_record(
                    &vault_id,
                    &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
                )
                .unwrap();
        }

        {
            let mut store = BrainStore::open(&db).unwrap();
            assert_eq!(store.sync_bootstrap(&vault_id).unwrap().object_count, 1);
            store
                .conn
                .execute(
                    "DELETE FROM current_encrypted_vault_objects WHERE vault_id = ?1",
                    params![vault_id.as_str()],
                )
                .unwrap();
            assert_eq!(store.sync_bootstrap(&vault_id).unwrap().object_count, 0);

            store.rebuild_current_projection(&vault_id).unwrap();
            let bootstrap = store.sync_bootstrap(&vault_id).unwrap();
            assert_eq!(bootstrap.latest_sequence, 1);
            assert_eq!(bootstrap.object_count, 1);
            assert_eq!(bootstrap.objects[0].revision, 1);
            assert!(!bootstrap.objects[0].deleted);
        }
    }

    #[test]
    fn sqlite_backup_copy_restores_append_log_and_can_rebuild_projection() {
        let temp = TempDir::new().unwrap();
        let source_db = temp.path().join("source.sqlite3");
        let restored_db = temp.path().join("restored.sqlite3");
        let vault_id = VaultId::new("acme").unwrap();
        let object_id = "obj_000000000001";

        {
            let mut store = BrainStore::open(&source_db).unwrap();
            bootstrap_org_and_strategy_folder(&mut store);
            store
                .submit_sync_record(
                    &vault_id,
                    &revision_record("event-create-backup", object_id, 1, None, "create"),
                )
                .unwrap();
            store
                .submit_sync_record(
                    &vault_id,
                    &revision_record("event-update-backup", object_id, 2, Some(1), "update"),
                )
                .unwrap();
        }

        std::fs::copy(&source_db, &restored_db).unwrap();

        let mut restored = BrainStore::open(&restored_db).unwrap();
        let bootstrap = restored.sync_bootstrap(&vault_id).unwrap();
        assert_eq!(bootstrap.latest_sequence, 2);
        assert_eq!(bootstrap.object_count, 1);
        assert_eq!(bootstrap.objects[0].revision, 2);

        restored
            .conn
            .execute(
                "DELETE FROM current_encrypted_vault_objects WHERE vault_id = ?1",
                params![vault_id.as_str()],
            )
            .unwrap();
        assert_eq!(restored.sync_bootstrap(&vault_id).unwrap().object_count, 0);

        restored.rebuild_current_projection(&vault_id).unwrap();
        let rebuilt = restored.sync_bootstrap(&vault_id).unwrap();
        assert_eq!(rebuilt.latest_sequence, 2);
        assert_eq!(rebuilt.object_count, 1);
        assert_eq!(rebuilt.objects[0].payload_json, "{\"body\":\"update\"}");
    }

    fn bootstrapped_org_store() -> BrainStore {
        let mut store = BrainStore::open_in_memory().unwrap();
        bootstrap_org(&mut store);
        store
    }

    fn store_with_strategy_folder() -> BrainStore {
        let mut store = BrainStore::open_in_memory().unwrap();
        bootstrap_org_and_strategy_folder(&mut store);
        store
    }

    fn bootstrap_org_and_strategy_folder(store: &mut BrainStore) {
        bootstrap_org(store);
        let vault_id = VaultId::new("acme").unwrap();
        store
            .create_folder(
                &vault_id,
                &strategy_folder(),
                &BTreeSet::new(),
                &[grant(
                    "grant-strategy-admin",
                    "strategy",
                    1,
                    "npub-admin",
                    "npub-admin",
                )],
            )
            .unwrap();
    }

    fn bootstrap_org(store: &mut BrainStore) {
        let output = bootstrap_organization_vault("acme", "Acme", "npub-admin").unwrap();
        let grants = grants_for_required(&output.required_key_grants, "npub-admin");
        store.create_vault_bootstrap(&output, &grants).unwrap();
    }

    fn bootstrap_org_named(store: &mut BrainStore, id: &str, name: &str, admin: &str) {
        let output = bootstrap_organization_vault(id, name, admin).unwrap();
        let grants = grants_for_required(&output.required_key_grants, admin);
        store.create_vault_bootstrap(&output, &grants).unwrap();
    }

    fn strategy_folder() -> Folder {
        Folder {
            id: FolderId::new("strategy").unwrap(),
            name: DisplayName::new("folder_name", "Strategy").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Restricted,
            parent_folder_id: Some(FolderId::new("getting-started").unwrap()),
            path: SafeRelativePath::new("folder_path", "getting-started/Strategy").unwrap(),
            current_key_version: 1,
            shared_folder_source: false,
        }
    }

    fn admin_only_folder() -> Folder {
        Folder {
            id: FolderId::new("admin-only").unwrap(),
            name: DisplayName::new("folder_name", "admin-only").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::AdminOnly,
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "admin-only").unwrap(),
            current_key_version: 1,
            shared_folder_source: false,
        }
    }

    fn grants_for_required(
        required: &[RequiredFolderKeyGrant],
        issuer: &str,
    ) -> Vec<FolderKeyGrantMetadata> {
        required
            .iter()
            .map(|required| {
                grant(
                    &format!(
                        "grant-{}-{}",
                        required.folder_id,
                        required.recipient_user_id.as_str()
                    ),
                    required.folder_id.as_str(),
                    required.key_version,
                    issuer,
                    required.recipient_user_id.as_str(),
                )
            })
            .collect()
    }

    fn assert_same_grants(actual: &[FolderKeyGrantMetadata], expected: &[FolderKeyGrantMetadata]) {
        assert_eq!(actual.len(), expected.len());
        for grant in expected {
            assert!(actual.contains(grant), "missing grant: {grant:?}");
        }
    }

    fn grant(
        id: &str,
        folder_id: &str,
        key_version: u32,
        issuer: &str,
        recipient: &str,
    ) -> FolderKeyGrantMetadata {
        FolderKeyGrantMetadata {
            id: id.to_owned(),
            folder_id: FolderId::new(folder_id).unwrap(),
            key_version,
            issuer_npub: UserId::new(issuer).unwrap(),
            recipient_npub: UserId::new(recipient).unwrap(),
            format: GRANT_FORMAT_NIP59.to_owned(),
            wrapped_event_json: "{\"kind\":1059}".to_owned(),
            access_change_event_json: Some("{\"kind\":30078}".to_owned()),
            created_at: "2026-06-23T00:00:00.000Z".to_owned(),
        }
    }

    fn revision_record(
        event_id: &str,
        object_id: &str,
        revision: u64,
        base_revision: Option<u64>,
        body: &str,
    ) -> SyncRecordInput {
        SyncRecordInput::FolderObjectRevision(revision_record_struct(
            event_id,
            "strategy",
            object_id,
            revision,
            base_revision,
            body,
        ))
    }

    fn revision_record_struct(
        event_id: &str,
        folder_id: &str,
        object_id: &str,
        revision: u64,
        base_revision: Option<u64>,
        body: &str,
    ) -> FolderObjectRevisionSyncRecord {
        FolderObjectRevisionSyncRecord {
            record_event_id: event_id.to_owned(),
            folder_id: FolderId::new(folder_id).unwrap(),
            object_id: ObjectId::new(object_id).unwrap(),
            revision,
            base_revision,
            actor_npub: UserId::new("npub-admin").unwrap(),
            client_created_at: "2026-06-23T00:00:00.000Z".to_owned(),
            payload_json: format!("{{\"body\":\"{body}\"}}"),
            record_event_kind: APP_SPECIFIC_KIND,
        }
    }

    fn revision_record_for(
        folder_id: &str,
        event_id: &str,
        object_id: &str,
        revision: u64,
        base_revision: Option<u64>,
        body: &str,
    ) -> SyncRecordInput {
        SyncRecordInput::FolderObjectRevision(revision_record_struct(
            event_id,
            folder_id,
            object_id,
            revision,
            base_revision,
            body,
        ))
    }

    fn tombstone_record(
        event_id: &str,
        object_id: &str,
        revision: u64,
        base_revision: u64,
    ) -> SyncRecordInput {
        SyncRecordInput::FolderObjectTombstone(FolderObjectTombstoneSyncRecord {
            record_event_id: event_id.to_owned(),
            folder_id: FolderId::new("strategy").unwrap(),
            object_id: ObjectId::new(object_id).unwrap(),
            revision,
            base_revision,
            actor_npub: UserId::new("npub-admin").unwrap(),
            client_created_at: "2026-06-23T00:00:01.000Z".to_owned(),
            payload_json: "{\"body\":\"delete\"}".to_owned(),
            record_event_kind: APP_SPECIFIC_KIND,
        })
    }
}
