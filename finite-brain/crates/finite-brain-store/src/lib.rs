//! FiniteBrain SQLite store and transaction boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use finite_brain_core::{
    BRAIN_CAPACITY_ENVELOPE, BootstrapOutput, Brain, BrainId, BrainKind, BrainMember, CoreError,
    DisplayName, Folder, FolderAccessMode, FolderId, FolderKeyRecipientPolicy, FolderRole,
    FolderRotationFanout, FolderRotationOperation, ObjectId, RequiredFolderKeyGrant,
    SafeRelativePath, UserId, required_folder_key_recipients, validate_folder_rotation_fanout,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

mod brains;
mod folder_access;
mod folder_deletion;
mod links;
mod loading;
mod personal_agents;
mod schema;
mod shared_folders;
mod sync_records;

const GRANT_FORMAT_NIP59: &str = "NIP-59";
const MAX_PULL_LIMIT: u64 = 1_000;
const MAX_BOOTSTRAP_FOLDERS: usize = BRAIN_CAPACITY_ENVELOPE.folders;
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
    /// A requested Brain does not exist.
    MissingBrain { brain_id: String },
    /// A requested Folder does not exist.
    MissingFolder { folder_id: String },
    /// A stable id already exists in the scoped table.
    DuplicateId { field: &'static str, value: String },
    /// Grant metadata did not include a required current recipient.
    MissingRequiredGrant { recipient_user_id: String },
    /// Stored state would violate Brain, member, admin, access, or grant rules.
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
    /// A mutation would exceed the governed accepted-state envelope.
    CapacityExceeded {
        limit: String,
        max: usize,
        current: usize,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => write!(f, "{error}"),
            Self::Database { message } => write!(f, "database error: {message}"),
            Self::MissingBrain { brain_id } => write!(f, "missing brain: {brain_id}"),
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
            Self::CapacityExceeded {
                limit,
                max,
                current,
            } => write!(
                f,
                "capacity exceeded for {limit}: current {current}, maximum {max}"
            ),
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
        if let Some((limit, max)) = parse_capacity_error(&value.to_string()) {
            return Self::CapacityExceeded {
                limit,
                max,
                current: max.saturating_add(1),
            };
        }
        Self::Database {
            message: value.to_string(),
        }
    }
}

fn parse_capacity_error(message: &str) -> Option<(String, usize)> {
    let marker = "finite_capacity:";
    let encoded = message.split(marker).nth(1)?;
    let mut parts = encoded.split(':');
    let limit = parts.next()?.to_owned();
    let max = parts.next()?.split_whitespace().next()?.parse().ok()?;
    Some((limit, max))
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

/// Reloaded Brain state with store-only metadata attached.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredBrain {
    /// Core Brain metadata.
    pub brain: Brain,
    /// The one active Personal Agent relationship, when occupied.
    pub personal_agent: Option<PersonalAgent>,
    /// Explicit restricted Folder access by Folder id.
    pub folder_access: BTreeMap<FolderId, BTreeSet<UserId>>,
    /// Stored Folder Key Grant metadata.
    pub grants: Vec<FolderKeyGrantMetadata>,
    /// Folders that still need current grants.
    pub setup_incomplete_folder_ids: BTreeSet<FolderId>,
    /// Exact pre-deletion readers allowed to observe each subtree tombstone.
    pub folder_deletion_audience: BTreeMap<String, BTreeSet<UserId>>,
}

/// One active Personal Agent relationship owned by a Personal Brain.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PersonalAgent {
    pub brain_id: BrainId,
    pub owner_npub: UserId,
    pub agent_npub: UserId,
    pub created_by_npub: UserId,
    pub created_at: String,
    pub updated_at: String,
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

/// Brain summary visible to an authenticated actor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VisibleBrain {
    /// Stable Brain id.
    pub id: BrainId,
    /// Brain kind.
    pub kind: BrainKind,
    /// Display name.
    pub name: String,
    /// Actor's relationship to this Brain.
    pub role: VisibleBrainRole,
    /// Pending invitation code when the actor has not accepted yet.
    pub invite_code: Option<String>,
}

/// Actor relationship used by client Brain switchers.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VisibleBrainRole {
    /// Personal Brain owner.
    Owner,
    /// Personal Brain's one fully trusted agent.
    PersonalAgent,
    /// Organization Brain admin.
    Admin,
    /// Organization Brain member.
    Member,
    /// Pending Organization Brain invitation.
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
    /// Brain admin access-change control record.
    BrainAdminAccessChange,
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
    /// Brain-scoped sequence.
    pub sequence: u64,
    /// True when this event id was already accepted.
    pub duplicate: bool,
}

/// Result of granting one identity the current Folder Key.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum GrantFolderAccessOutcome {
    /// Access and its current-version key grant were added.
    Granted,
    /// The identity already had effective access and the current-version grant.
    AlreadyHasAccess,
}

/// Result of atomically deleting one Folder subtree.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FolderSubtreeDeletion {
    pub sequence: u64,
    pub duplicate: bool,
    pub folder_count: usize,
    pub object_count: usize,
    pub deleted_folder_ids: Vec<FolderId>,
    /// Content-free deterministic accounting for the bounded delete transaction.
    pub work: FolderDeletionWork,
}

/// Deterministic work counters for one direct Folder subtree deletion.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct FolderDeletionWork {
    pub descendants_visited: usize,
    pub objects_collected: usize,
    pub audience_collected: usize,
    pub invitations_scanned: usize,
    pub invitations_deleted: usize,
    pub mutation_statements: usize,
    pub max_statement_parameters: usize,
    pub retry_attempts: usize,
}

/// Optional HTTP-signed scope shown to a user before destructive confirmation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FolderDeletionExpectation {
    pub folder_ids: BTreeSet<FolderId>,
    pub object_count: usize,
}

/// Retained facts needed to validate an exact retry after a Folder is gone.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FolderDeletionReplay {
    pub deletion_event_id: String,
    pub actor_npub: UserId,
    pub root_key_version: u32,
    pub folder_count: usize,
    pub object_count: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PersonalAgentFolderRotation {
    pub folder_id: FolderId,
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantMetadata>,
    pub reencrypted_records: Vec<FolderObjectRevisionSyncRecord>,
    pub control_records: Vec<ControlSyncRecord>,
}

/// Stored accepted sync record.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredSyncRecord {
    /// Brain-scoped sequence.
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

/// Encrypted Brain Export with actor-filtered visibility.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EncryptedBrainExport {
    /// Export version.
    pub version: String,
    /// Brain summary.
    pub brain: ExportBrainSummary,
    /// Folder metadata with actor accessibility.
    pub folders: Vec<EncryptedExportFolder>,
    /// Current encrypted object projection.
    pub objects: Vec<EncryptedExportObject>,
    /// Visible Folder Key Grants.
    pub key_grants: Vec<FolderKeyGrantMetadata>,
    /// Visible access state.
    pub access_state: EncryptedExportAccessState,
}

/// Brain summary in Encrypted Brain Export.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExportBrainSummary {
    /// Brain id.
    pub id: BrainId,
    /// Brain kind.
    pub kind: BrainKind,
    /// Brain name.
    pub name: DisplayName,
    /// Personal Brain owner, if any.
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

/// Current lifecycle state for Brain Invitations and Share Links.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LinkStatus {
    /// Link can still be accepted.
    Pending,
    /// Link was consumed by the target recipient.
    Accepted,
    /// Link delivery was revoked by an admin.
    Revoked,
}

/// Brain Invitation target routing mode.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BrainInvitationTargetKind {
    /// Existing concrete npub/hex/NIP-05 user target.
    Npub,
    /// Email-targeted bootstrap awaiting client-side claim into an npub.
    EmailBootstrap,
}

impl BrainInvitationTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Npub => "npub",
            Self::EmailBootstrap => "email_bootstrap",
        }
    }
}

impl TryFrom<&str> for BrainInvitationTargetKind {
    type Error = StoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "npub" => Ok(Self::Npub),
            "email_bootstrap" => Ok(Self::EmailBootstrap),
            _ => Err(StoreError::BrokenInvariant {
                reason: format!("unknown brain invitation target kind {value}"),
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

/// Stored singleton Brain Invitation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredBrainInvitation {
    /// Stable invitation id.
    pub id: String,
    /// Brain id.
    pub brain_id: BrainId,
    /// Target routing mode.
    pub target_kind: BrainInvitationTargetKind,
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
    /// Source Brain id.
    pub brain_id: BrainId,
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

/// Stored Shared Folder Invitation from a source Folder to a destination Organization Brain.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredSharedFolderInvitation {
    /// Stable invitation id.
    pub id: String,
    /// Source Brain id.
    pub source_brain_id: BrainId,
    /// Source Folder id.
    pub source_folder_id: FolderId,
    /// Destination Organization Brain id.
    pub destination_brain_id: BrainId,
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
    /// Source Brain id.
    pub source_brain_id: BrainId,
    /// Source Folder id.
    pub source_folder_id: FolderId,
    /// Destination Organization Brain id.
    pub destination_brain_id: BrainId,
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
    /// Destination Organization Brain id.
    pub organization_brain_id: BrainId,
    /// Source Brain id.
    pub source_brain_id: BrainId,
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

/// Direction of a shared-folder relationship relative to one Brain.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SharedFolderDirection {
    /// The Brain is the source that shares one of its Folders.
    Source,
    /// The Brain is the destination that mounts a shared Folder.
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
    /// Destination Organization Brain id.
    pub organization_brain_id: BrainId,
    /// Source Brain id.
    pub source_brain_id: BrainId,
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
    /// Brain id.
    pub brain_id: BrainId,
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
    /// Brain id.
    pub brain_id: BrainId,
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
        conn.busy_timeout(Duration::from_secs(5))?;
        let mut store = Self { conn };
        store.apply_migrations()?;
        Ok(store)
    }

    pub fn load_brain(&self, brain_id: &BrainId) -> Result<StoredBrain, StoreError> {
        let mut brain = self.load_core_brain(brain_id)?;
        let folder_access = self.load_folder_access(brain_id)?;
        for member in &mut brain.members {
            member.folder_access = folder_access
                .iter()
                .filter_map(|(folder_id, users)| {
                    users.contains(&member.user_id).then_some(folder_id.clone())
                })
                .collect();
        }

        Ok(StoredBrain {
            brain,
            personal_agent: self.load_personal_agent(brain_id)?,
            folder_access,
            grants: self.load_grants(brain_id)?,
            setup_incomplete_folder_ids: self.load_setup_incomplete_folder_ids(brain_id)?,
            folder_deletion_audience: self.load_folder_deletion_audience(brain_id)?,
        })
    }

    fn load_folder_deletion_audience(
        &self,
        brain_id: &BrainId,
    ) -> Result<BTreeMap<String, BTreeSet<UserId>>, StoreError> {
        let mut stmt = self.conn.prepare(
            r#"SELECT deletion_event_id, actor_npub
               FROM folder_deletion_audience
               WHERE brain_id = ?1
               ORDER BY deletion_event_id, actor_npub"#,
        )?;
        let rows = stmt.query_map(params![brain_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut audience = BTreeMap::<String, BTreeSet<UserId>>::new();
        for row in rows {
            let (event_id, actor_npub) = row?;
            audience
                .entry(event_id)
                .or_default()
                .insert(UserId::new(actor_npub)?);
        }
        Ok(audience)
    }

    /// Upsert verified display metadata for a canonical Nostr identity.
    pub fn record_identity_alias(&mut self, alias: &IdentityAlias) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        upsert_identity_alias(&tx, alias)?;
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
        brain_id: &BrainId,
        folder_id: &FolderId,
    ) -> Result<bool, StoreError> {
        let exists = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM folders WHERE brain_id = ?1 AND id = ?2)",
            params![brain_id.as_str(), folder_id.as_str()],
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

    /// Accept a validated sync record, assign a Brain-scoped sequence, and update projections.
    pub fn submit_sync_record(
        &mut self,
        brain_id: &BrainId,
        input: &SyncRecordInput,
    ) -> Result<SubmitRecordOutcome, StoreError> {
        self.load_core_brain(brain_id)?;
        sync_records::validate_sync_input(input)?;

        let tx = self.conn.transaction()?;
        if let Some(sequence) =
            sync_records::existing_sequence(&tx, brain_id, input.record_event_id())?
        {
            tx.commit()?;
            return Ok(SubmitRecordOutcome {
                sequence,
                duplicate: true,
            });
        }

        sync_records::validate_sync_conflict(&tx, brain_id, input)?;
        let sequence = sync_records::next_sequence(&tx, brain_id)?;
        sync_records::insert_sync_record(&tx, brain_id, sequence, input)?;
        sync_records::project_sync_record(&tx, brain_id, input)?;
        tx.commit()?;

        Ok(SubmitRecordOutcome {
            sequence,
            duplicate: false,
        })
    }

    /// Return the current encrypted state for rebootstrap.
    pub fn sync_bootstrap(&self, brain_id: &BrainId) -> Result<SyncBootstrap, StoreError> {
        self.require_brain_exists(brain_id)?;
        let objects = self.load_current_objects(brain_id)?;
        let control_records = sync_records::load_sync_records(&self.conn, brain_id)?
            .into_iter()
            .filter(|record| {
                matches!(
                    record.record_type,
                    SyncRecordType::FolderKeyGrant | SyncRecordType::BrainAdminAccessChange
                )
            })
            .collect::<Vec<_>>();
        Ok(SyncBootstrap {
            brain_id: brain_id.clone(),
            latest_sequence: self.latest_sequence(brain_id)?,
            object_count: objects.len(),
            objects,
            control_records,
            current_state_kind: "current_encrypted_brain_state",
        })
    }

    /// Build an actor-filtered Encrypted Brain Export without decrypting content.
    pub fn encrypted_brain_export(
        &self,
        brain_id: &BrainId,
        actor_npub: &UserId,
    ) -> Result<EncryptedBrainExport, StoreError> {
        let stored = self.load_brain(brain_id)?;
        let is_personal_agent = stored
            .personal_agent
            .as_ref()
            .is_some_and(|relationship| relationship.agent_npub == *actor_npub);
        let has_personal_folder_scope = stored.brain.kind != BrainKind::Personal
            || stored.brain.owner_user_id.as_ref() == Some(actor_npub)
            || is_personal_agent
            || stored
                .folder_access
                .values()
                .any(|users| users.contains(actor_npub));
        if (!brain_visible_to_actor(&stored.brain, actor_npub) && !is_personal_agent)
            || !has_personal_folder_scope
        {
            return Err(StoreError::BrokenInvariant {
                reason: "brain access required for encrypted export".to_owned(),
            });
        }
        let is_admin = stored.brain.admins.contains(actor_npub);
        let is_limited_personal_member = stored.brain.kind == BrainKind::Personal
            && stored.brain.owner_user_id.as_ref() != Some(actor_npub)
            && !is_personal_agent;
        let folders = stored
            .brain
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
            .load_current_objects(brain_id)?
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

        Ok(EncryptedBrainExport {
            version: "finite-brain-export-v1".to_owned(),
            brain: ExportBrainSummary {
                id: stored.brain.id,
                kind: stored.brain.kind,
                name: stored.brain.name,
                owner_user_id: stored.brain.owner_user_id,
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
        brain_id: &BrainId,
        after_sequence: u64,
        limit: u64,
    ) -> Result<SyncPull, StoreError> {
        self.require_brain_exists(brain_id)?;
        let retention_floor = self.retention_floor(brain_id)?;
        if after_sequence < retention_floor {
            return Err(StoreError::RebootstrapRequired { retention_floor });
        }

        let latest_sequence = self.latest_sequence(brain_id)?;
        sync_records::pull_sync_records(
            &self.conn,
            brain_id,
            after_sequence,
            limit,
            latest_sequence,
        )
    }

    /// Set the retained cursor floor for a Brain.
    pub fn set_retention_floor(
        &mut self,
        brain_id: &BrainId,
        retention_floor: u64,
    ) -> Result<(), StoreError> {
        self.require_brain_exists(brain_id)?;
        self.conn.execute(
            r#"
            INSERT INTO brain_sync_retention (brain_id, retention_floor)
            VALUES (?1, ?2)
            ON CONFLICT(brain_id) DO UPDATE SET retention_floor = excluded.retention_floor
            "#,
            params![brain_id.as_str(), retention_floor],
        )?;
        Ok(())
    }

    /// Rebuild current encrypted object projection from the accepted append log.
    pub fn rebuild_current_projection(&mut self, brain_id: &BrainId) -> Result<(), StoreError> {
        self.require_brain_exists(brain_id)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM current_encrypted_brain_objects WHERE brain_id = ?1",
            params![brain_id.as_str()],
        )?;

        let records = sync_records::load_sync_records_tx(&tx, brain_id)?;
        for record in &records {
            sync_records::project_stored_record(&tx, brain_id, record)?;
        }

        tx.commit()?;
        Ok(())
    }

    fn require_brain_exists(&self, brain_id: &BrainId) -> Result<(), StoreError> {
        self.conn
            .query_row(
                "SELECT 1 FROM brains WHERE id = ?1",
                params![brain_id.as_str()],
                |_| Ok(()),
            )
            .optional()?
            .ok_or_else(|| StoreError::MissingBrain {
                brain_id: brain_id.to_string(),
            })
    }

    fn require_organization_brain(&self, brain_id: &BrainId) -> Result<(), StoreError> {
        let brain = self.load_core_brain(brain_id)?;
        if brain.kind != BrainKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "member/admin mutation requires an organization brain".to_owned(),
            });
        }
        Ok(())
    }

    fn member_exists(&self, brain_id: &BrainId, user_id: &UserId) -> Result<bool, StoreError> {
        let exists = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM brain_members WHERE brain_id = ?1 AND user_id = ?2)",
            params![brain_id.as_str(), user_id.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        Ok(exists)
    }

    fn member_has_restricted_access(
        &self,
        brain_id: &BrainId,
        user_id: &UserId,
    ) -> Result<bool, StoreError> {
        let exists = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM folder_access WHERE brain_id = ?1 AND user_id = ?2)",
            params![brain_id.as_str(), user_id.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        Ok(exists)
    }

    fn validate_folder_request(
        &self,
        brain: &Brain,
        folder: &Folder,
        access_user_ids: &BTreeSet<UserId>,
        grants: &[FolderKeyGrantMetadata],
    ) -> Result<(), StoreError> {
        if brain.kind == BrainKind::Personal
            && !matches!(
                folder.access,
                FolderAccessMode::Owner | FolderAccessMode::Restricted
            )
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain shared access requires a restricted Folder".to_owned(),
            });
        }
        validate_hierarchy(&self.conn, &brain.id, folder)?;
        validate_access_list_shape(folder, access_user_ids)?;
        validate_access_membership(brain, access_user_ids)?;
        let personal_agent = self
            .load_personal_agent(&brain.id)?
            .map(|relationship| relationship.agent_npub);
        let required =
            required_recipients(brain, folder, access_user_ids, personal_agent.as_ref())?;
        validate_folder_grants(brain, folder, &required, grants, personal_agent.as_ref())
    }

    fn actor_has_current_source_access_and_grant(
        &self,
        source_brain_id: &BrainId,
        source_folder_id: &FolderId,
        actor_npub: &UserId,
    ) -> Result<bool, StoreError> {
        let stored = self.load_brain(source_brain_id)?;
        let Some(folder) = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == *source_folder_id)
        else {
            return Ok(false);
        };
        let has_access = stored.brain.admins.contains(actor_npub)
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
        let destination = self.load_core_brain(&connection.destination_brain_id)?;
        if destination.kind != BrainKind::Organization || !destination.admins.contains(actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "connection member management requires a destination brain admin"
                    .to_owned(),
            });
        }
        Ok(())
    }

    fn validate_destination_member(
        &self,
        destination_brain_id: &BrainId,
        target_npub: &UserId,
    ) -> Result<(), StoreError> {
        let destination = self.load_core_brain(destination_brain_id)?;
        if destination
            .members
            .iter()
            .any(|member| member.user_id == *target_npub)
        {
            Ok(())
        } else {
            Err(StoreError::BrokenInvariant {
                reason: "connection target must be a destination brain member".to_owned(),
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
        let stored = self.load_brain(&connection.source_brain_id)?;
        let folder = stored
            .brain
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
        let required = required_recipients(
            &stored.brain,
            &rotated_folder,
            &remaining_access,
            stored
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
        )?;
        validate_connection_rotation_grants(
            &rotated_folder,
            &required,
            rotation.grants,
            actor_npub,
        )?;
        let live_objects = self
            .load_current_objects(&connection.source_brain_id)?
            .into_iter()
            .filter(|object| object.folder_id == connection.source_folder_id && !object.deleted)
            .collect::<Vec<_>>();
        validate_rotation_records(&live_objects, rotation.reencrypted_records)?;

        let tx = self.conn.transaction()?;
        for removed in rotation.removed_user_ids {
            tx.execute(
                "DELETE FROM folder_access WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3",
                params![
                    connection.source_brain_id.as_str(),
                    connection.source_folder_id.as_str(),
                    removed.as_str()
                ],
            )?;
        }
        tx.execute(
            "UPDATE folders SET current_key_version = ?3 WHERE brain_id = ?1 AND id = ?2",
            params![
                connection.source_brain_id.as_str(),
                connection.source_folder_id.as_str(),
                rotation.new_key_version
            ],
        )?;
        invalidate_pending_email_bootstraps_for_rotated_folder(
            &tx,
            &connection.source_brain_id,
            &connection.source_folder_id,
            rotation.updated_at,
        )?;
        for grant in rotation.grants {
            insert_grant(&tx, &connection.source_brain_id, grant)?;
        }
        for record in rotation.reencrypted_records {
            let input = SyncRecordInput::FolderObjectRevision(record.clone());
            sync_records::validate_sync_input(&input)?;
            sync_records::validate_sync_conflict(&tx, &connection.source_brain_id, &input)?;
            let sequence = sync_records::next_sequence(&tx, &connection.source_brain_id)?;
            sync_records::insert_sync_record(&tx, &connection.source_brain_id, sequence, &input)?;
            sync_records::project_sync_record(&tx, &connection.source_brain_id, &input)?;
        }
        after_rotation(&tx)?;
        tx.commit()?;
        Ok(())
    }
}

fn upsert_identity_alias(tx: &Transaction<'_>, alias: &IdentityAlias) -> Result<(), StoreError> {
    let relays_json =
        serde_json::to_string(&alias.nip05_relays).map_err(|error| StoreError::InvalidRecord {
            reason: format!("identity alias relays did not serialize: {error}"),
        })?;
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
    Ok(())
}

impl SyncRecordType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::FolderObjectRevision => "folder_object_revision",
            Self::FolderObjectTombstone => "folder_object_tombstone",
            Self::FolderKeyGrant => "folder_key_grant",
            Self::BrainAdminAccessChange => "brain_admin_access_change",
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
            "brain_admin_access_change" => Ok(Self::BrainAdminAccessChange),
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

fn brain_invitation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredBrainInvitation> {
    let status = row.get::<_, String>(3)?;
    let initial_folder_access_json = row.get::<_, String>(6)?;
    let target_kind = row.get::<_, String>(12)?;
    let bootstrap_scope_json = row.get::<_, String>(19)?;
    Ok(StoredBrainInvitation {
        id: row.get(0)?,
        brain_id: BrainId::new(row.get::<_, String>(1)?)
            .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
        user_id: row
            .get::<_, Option<String>>(2)?
            .map(UserId::new)
            .transpose()
            .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?,
        target_kind: BrainInvitationTargetKind::try_from(target_kind.as_str())
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
    let brain_id = BrainId::new(row.get::<_, String>(1)?)
        .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?;
    let folder_id = FolderId::new(row.get::<_, String>(2)?)
        .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?;
    let recipient_npub = UserId::new(row.get::<_, String>(3)?)
        .map_err(to_from_sql_error(3, rusqlite::types::Type::Text))?;
    let created_by_npub = UserId::new(row.get::<_, String>(4)?)
        .map_err(to_from_sql_error(4, rusqlite::types::Type::Text))?;
    Ok(StoredShareLink {
        id: row.get(0)?,
        brain_id: brain_id.clone(),
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
    let source_brain_id = BrainId::new(row.get::<_, String>(1)?)
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
        source_brain_id: source_brain_id.clone(),
        source_folder_id: source_folder_id.clone(),
        destination_brain_id: BrainId::new(row.get::<_, String>(3)?)
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
        source_brain_id: BrainId::new(row.get::<_, String>(1)?)
            .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
        source_folder_id: FolderId::new(row.get::<_, String>(2)?)
            .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?,
        destination_brain_id: BrainId::new(row.get::<_, String>(3)?)
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
        organization_brain_id: BrainId::new(row.get::<_, String>(1)?)
            .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
        source_brain_id: BrainId::new(row.get::<_, String>(2)?)
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
    invitation: &StoredBrainInvitation,
    user_id: &UserId,
    now: &str,
) -> Result<(), StoreError> {
    if invitation.target_kind != BrainInvitationTargetKind::Npub
        || invitation.user_id.as_ref() != Some(user_id)
        || invitation.status != LinkStatus::Pending
        || timestamp_expired(&invitation.expires_at, now)
    {
        return Err(StoreError::UnavailableLink {
            kind: "brain invitation",
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
    brain_id: &BrainId,
    folder_id: &FolderId,
) -> Result<(), StoreError> {
    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE brain_id = ?1 AND id = ?2)",
        params![brain_id.as_str(), folder_id.as_str()],
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
    brain_id: &BrainId,
    user_id: &UserId,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO brain_members (brain_id, user_id) VALUES (?1, ?2)",
        params![brain_id.as_str(), user_id.as_str()],
    )?;
    Ok(())
}

fn insert_folder_access_if_missing(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    folder_id: &FolderId,
    user_id: &UserId,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO folder_access (brain_id, folder_id, user_id) VALUES (?1, ?2, ?3)",
        params![brain_id.as_str(), folder_id.as_str(), user_id.as_str()],
    )?;
    Ok(())
}

fn insert_grant_or_ignore(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    grant: &FolderKeyGrantMetadata,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT OR IGNORE INTO folder_key_grants (
            id, brain_id, folder_id, key_version, issuer_npub, recipient_npub, format,
            wrapped_event_json, access_change_event_json, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![
            grant.id,
            brain_id.as_str(),
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
    source_brain_id: &BrainId,
    source_folder_id: &FolderId,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(owner_npub.as_str());
    hasher.update(b"\n");
    hasher.update(source_brain_id.as_str());
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
    validate_loaded_brain(&output.brain)
}

fn validate_loaded_brain(brain: &Brain) -> Result<(), StoreError> {
    match brain.kind {
        BrainKind::Personal => {
            let Some(owner) = brain.owner_user_id.as_ref() else {
                return Err(StoreError::BrokenInvariant {
                    reason: "personal brain must have an owner".to_owned(),
                });
            };
            if !brain.admins.is_empty()
                || brain.members.iter().any(|member| member.user_id == *owner)
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "personal brain owner is sole admin authority and cannot be an ordinary member"
                        .to_owned(),
                });
            }
        }
        BrainKind::Organization => {
            if brain.owner_user_id.is_some() || brain.admins.is_empty() {
                return Err(StoreError::BrokenInvariant {
                    reason: "organization brain must have admins and no owner".to_owned(),
                });
            }
            let members = brain
                .members
                .iter()
                .map(|member| member.user_id.clone())
                .collect::<BTreeSet<_>>();
            if brain.admins.iter().any(|admin| !members.contains(admin)) {
                return Err(StoreError::BrokenInvariant {
                    reason: "every brain admin must also be a member".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_required_grants(
    brain: &Brain,
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
        validate_grant_issuer(brain, grant, None)?;
    }
    Ok(())
}

fn validate_folder_grants(
    brain: &Brain,
    folder: &Folder,
    required_recipients: &BTreeSet<UserId>,
    grants: &[FolderKeyGrantMetadata],
    personal_agent: Option<&UserId>,
) -> Result<(), StoreError> {
    let mut provided = BTreeSet::new();
    for grant in grants {
        validate_grant_metadata(grant)?;
        validate_grant_issuer(brain, grant, personal_agent)?;
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

fn validate_grant_issuer(
    brain: &Brain,
    grant: &FolderKeyGrantMetadata,
    personal_agent: Option<&UserId>,
) -> Result<(), StoreError> {
    match brain.kind {
        BrainKind::Personal => {
            if brain.owner_user_id.as_ref() != Some(&grant.issuer_npub)
                && personal_agent != Some(&grant.issuer_npub)
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "personal brain grants must be issued by the owner or Personal Agent"
                        .to_owned(),
                });
            }
        }
        BrainKind::Organization => {
            if !brain.admins.contains(&grant.issuer_npub) {
                return Err(StoreError::BrokenInvariant {
                    reason: "organization folder grants must be issued by a brain admin".to_owned(),
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
    brain: &Brain,
    selected_restricted_folder_access: &[FolderId],
) -> Result<Vec<EmailInviteBootstrapScopeFolder>, StoreError> {
    let selected = selected_restricted_folder_access
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut seen_selected = BTreeSet::new();
    let mut included = BTreeSet::new();
    let mut scope = Vec::new();

    for folder in &brain.folders {
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
    brain: &Brain,
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
        let folder = brain
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
    brain: &Brain,
    scope: &[EmailInviteBootstrapScopeFolder],
) -> Result<bool, StoreError> {
    for item in scope {
        let folder = brain
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
    brain_id: &BrainId,
    folder_id: &FolderId,
    updated_at: &str,
) -> Result<(), StoreError> {
    let mut statement = tx.prepare(
        r#"
        SELECT id, bootstrap_scope_json
        FROM brain_invitations
        WHERE brain_id = ?1
          AND target_kind = 'email_bootstrap'
          AND status = 'pending'
          AND bootstrap_wrapped_event_json IS NOT NULL
        "#,
    )?;
    let invitations = statement
        .query_map(params![brain_id.as_str()], |row| {
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
                UPDATE brain_invitations
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
    brain_id: &BrainId,
    folder: &Folder,
) -> Result<(), StoreError> {
    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE brain_id = ?1 AND id = ?2)",
        params![brain_id.as_str(), folder.id.as_str()],
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
            "SELECT EXISTS(SELECT 1 FROM folders WHERE brain_id = ?1 AND id = ?2)",
            params![brain_id.as_str(), parent_id.as_str()],
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
    brain: &Brain,
    access_user_ids: &BTreeSet<UserId>,
) -> Result<(), StoreError> {
    let members = brain
        .members
        .iter()
        .map(|member| member.user_id.clone())
        .collect::<BTreeSet<_>>();
    for user_id in access_user_ids {
        if !members.contains(user_id) {
            return Err(StoreError::BrokenInvariant {
                reason: format!("folder access user is not a brain member: {user_id}"),
            });
        }
    }
    Ok(())
}

fn required_recipients(
    brain: &Brain,
    folder: &Folder,
    access_user_ids: &BTreeSet<UserId>,
    personal_agent: Option<&UserId>,
) -> Result<BTreeSet<UserId>, StoreError> {
    let members = brain
        .members
        .iter()
        .map(|member| member.user_id.clone())
        .collect::<Vec<_>>();
    required_folder_key_recipients(FolderKeyRecipientPolicy {
        brain_kind: brain.kind,
        folder_access: folder.access,
        owner_user_id: brain.owner_user_id.as_ref(),
        admins: &brain.admins,
        members: &members,
        explicit_access_user_ids: access_user_ids,
        personal_agent_npub: personal_agent,
    })
    .map_err(StoreError::from)
}

fn brain_visible_to_actor(brain: &Brain, actor_npub: &UserId) -> bool {
    match brain.kind {
        BrainKind::Personal => {
            brain
                .owner_user_id
                .as_ref()
                .is_some_and(|owner| owner == actor_npub)
                || brain
                    .members
                    .iter()
                    .any(|member| member.user_id == *actor_npub)
        }
        BrainKind::Organization => brain
            .members
            .iter()
            .any(|member| member.user_id == *actor_npub),
    }
}

pub(crate) fn has_brain_operational_authority(stored: &StoredBrain, actor_npub: &UserId) -> bool {
    match stored.brain.kind {
        BrainKind::Personal => {
            stored.brain.owner_user_id.as_ref() == Some(actor_npub)
                || stored
                    .personal_agent
                    .as_ref()
                    .is_some_and(|relationship| relationship.agent_npub == *actor_npub)
        }
        BrainKind::Organization => stored.brain.admins.contains(actor_npub),
    }
}

fn folder_visible_to_actor(
    stored: &StoredBrain,
    folder_id: &FolderId,
    actor_npub: &UserId,
) -> bool {
    let Some(folder) = stored
        .brain
        .folders
        .iter()
        .find(|folder| folder.id == *folder_id)
    else {
        return false;
    };
    let is_owner = stored
        .brain
        .owner_user_id
        .as_ref()
        .is_some_and(|owner| owner == actor_npub);
    let is_admin = stored.brain.admins.contains(actor_npub);
    let is_personal_agent = stored
        .personal_agent
        .as_ref()
        .is_some_and(|relationship| relationship.agent_npub == *actor_npub);
    let is_member = stored
        .brain
        .members
        .iter()
        .any(|member| member.user_id == *actor_npub);

    if is_personal_agent {
        return true;
    }

    match folder.access {
        FolderAccessMode::Owner => is_owner,
        FolderAccessMode::AdminOnly => is_owner || is_admin,
        FolderAccessMode::AllMembers => {
            is_owner || is_admin || (stored.brain.kind == BrainKind::Organization && is_member)
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
    stored: &StoredBrain,
    actor_npub: &UserId,
    is_admin: bool,
) -> EncryptedExportAccessState {
    if is_admin {
        return EncryptedExportAccessState {
            members: stored
                .brain
                .members
                .iter()
                .map(|member| member.user_id.clone())
                .collect(),
            admins: stored.brain.admins.clone(),
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
            .brain
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

fn insert_brain(tx: &Transaction<'_>, brain: &Brain) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO brains (id, kind, name, owner_user_id, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            brain.id.as_str(),
            brain_kind_str(brain.kind),
            brain.name.as_str(),
            brain.owner_user_id.as_ref().map(UserId::as_str),
            current_timestamp()
        ],
    )
    .map_err(map_brain_insert_error(brain))?;
    Ok(())
}

fn map_brain_insert_error(brain: &Brain) -> impl FnOnce(rusqlite::Error) -> StoreError + '_ {
    move |error| match error {
        rusqlite::Error::SqliteFailure(inner, message)
            if matches!(inner.code, rusqlite::ErrorCode::ConstraintViolation)
                && brain.kind == BrainKind::Personal
                && message
                    .as_deref()
                    .is_some_and(|message| message.contains("brains.owner_user_id")) =>
        {
            StoreError::BrokenInvariant {
                reason: "user already has a personal brain".to_owned(),
            }
        }
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(inner.code, rusqlite::ErrorCode::ConstraintViolation) =>
        {
            StoreError::DuplicateId {
                field: "brain_id",
                value: brain.id.to_string(),
            }
        }
        other => StoreError::from(other),
    }
}

fn insert_members_and_admins(tx: &Transaction<'_>, brain: &Brain) -> Result<(), StoreError> {
    for member in &brain.members {
        tx.execute(
            "INSERT INTO brain_members (brain_id, user_id) VALUES (?1, ?2)",
            params![brain.id.as_str(), member.user_id.as_str()],
        )?;
    }
    for admin in &brain.admins {
        tx.execute(
            "INSERT INTO brain_admins (brain_id, user_id) VALUES (?1, ?2)",
            params![brain.id.as_str(), admin.as_str()],
        )?;
    }
    Ok(())
}

fn insert_folder(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    folder: &Folder,
    setup_incomplete: bool,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO folders (
            brain_id, id, name, role, access, parent_folder_id, parent_folder_key, path,
            current_key_version, shared_folder_source, setup_incomplete, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            brain_id.as_str(),
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
    brain_id: &BrainId,
    folder_id: &FolderId,
    access_user_ids: &BTreeSet<UserId>,
) -> Result<(), StoreError> {
    for user_id in access_user_ids {
        tx.execute(
            "INSERT INTO folder_access (brain_id, folder_id, user_id) VALUES (?1, ?2, ?3)",
            params![brain_id.as_str(), folder_id.as_str(), user_id.as_str()],
        )?;
    }
    Ok(())
}

fn insert_grant(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    grant: &FolderKeyGrantMetadata,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO folder_key_grants (
            id, brain_id, folder_id, key_version, issuer_npub, recipient_npub, format,
            wrapped_event_json, access_change_event_json, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![
            grant.id,
            brain_id.as_str(),
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
    move |error| {
        if parse_capacity_error(&error.to_string()).is_some() {
            return StoreError::from(error);
        }
        match error {
            rusqlite::Error::SqliteFailure(inner, _)
                if matches!(inner.code, rusqlite::ErrorCode::ConstraintViolation) =>
            {
                StoreError::DuplicateId { field, value }
            }
            other => StoreError::from(other),
        }
    }
}

fn brain_kind_str(kind: BrainKind) -> &'static str {
    match kind {
        BrainKind::Personal => "personal",
        BrainKind::Organization => "organization",
    }
}

fn parse_brain_kind(value: &str) -> Result<BrainKind, StoreError> {
    match value {
        "personal" => Ok(BrainKind::Personal),
        "organization" => Ok(BrainKind::Organization),
        _ => Err(StoreError::BrokenInvariant {
            reason: format!("unknown brain kind: {value}"),
        }),
    }
}

fn folder_role_str(role: FolderRole) -> &'static str {
    match role {
        FolderRole::PersonalHome => "personal_home",
        FolderRole::BrainOps => "brain_ops",
        FolderRole::General => "general",
        FolderRole::Folder => "folder",
    }
}

fn parse_folder_role(value: &str) -> Result<FolderRole, StoreError> {
    match value {
        "personal_home" => Ok(FolderRole::PersonalHome),
        "brain_ops" => Ok(FolderRole::BrainOps),
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
    use finite_brain_core::{
        MAX_FOLDER_ACCESS_REMOVAL_GRANTS, MAX_PERSONAL_AGENT_ROTATION_FOLDERS,
        bootstrap_organization_brain, bootstrap_personal_brain,
    };
    use std::sync::{Arc, Barrier};
    use tempfile::TempDir;

    #[test]
    fn exposes_store_crate_name() {
        assert_eq!(crate_name(), "finite-brain-store");
    }

    #[test]
    fn persists_and_reloads_personal_bootstrap() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("brain-sync.sqlite3");
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();
        let grants = grants_for_required(&output.required_key_grants, "npub-owner");
        let aliases = [
            IdentityAlias {
                npub: UserId::new("npub-owner").unwrap(),
                hex_public_key: "hex-owner".to_owned(),
                preferred_nip05: Some("owner@finite.computer".to_owned()),
                nip05_verified_at: Some("2026-06-23T00:00:00Z".to_owned()),
                nip05_relays: Vec::new(),
                updated_at: "2026-06-23T00:00:00Z".to_owned(),
            },
            IdentityAlias {
                npub: UserId::new("npub-agent").unwrap(),
                hex_public_key: "hex-agent".to_owned(),
                preferred_nip05: Some("agent@finite.vip".to_owned()),
                nip05_verified_at: Some("2026-06-23T00:00:00Z".to_owned()),
                nip05_relays: Vec::new(),
                updated_at: "2026-06-23T00:00:00Z".to_owned(),
            },
        ];

        {
            let mut store = BrainStore::open(&db).unwrap();
            store
                .create_personal_brain_bootstrap_with_identities(
                    &output,
                    &grants,
                    &UserId::new("npub-agent").unwrap(),
                    &UserId::new("npub-owner").unwrap(),
                    "2026-06-23T00:00:00Z",
                    &aliases,
                )
                .unwrap();
        }

        let store = BrainStore::open(&db).unwrap();
        let stored = store
            .load_brain(&BrainId::new("personal").unwrap())
            .unwrap();

        assert_eq!(stored.brain.kind, BrainKind::Personal);
        assert_eq!(
            stored.brain.owner_user_id,
            Some(UserId::new("npub-owner").unwrap())
        );
        assert!(stored.brain.folders.is_empty());
        assert_eq!(
            stored.personal_agent.unwrap().agent_npub,
            UserId::new("npub-agent").unwrap()
        );
        assert!(stored.folder_access.is_empty());
        assert!(stored.grants.is_empty());
        assert_same_grants(&stored.grants, &grants);
        assert!(stored.setup_incomplete_folder_ids.is_empty());
        assert_eq!(
            store
                .load_identity_aliases(&[
                    UserId::new("npub-owner").unwrap(),
                    UserId::new("npub-agent").unwrap(),
                ])
                .unwrap(),
            aliases
        );
    }

    #[test]
    fn database_allows_only_one_personal_brain_per_owner_across_connections() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("one-personal-brain.sqlite3");
        let first = BrainStore::open(&db).unwrap();
        let second = BrainStore::open(&db).unwrap();

        first
            .conn
            .execute(
                "INSERT INTO brains (id, kind, name, owner_user_id, created_at) VALUES (?1, 'personal', ?2, ?3, ?4)",
                params!["personal-first", "First", "npub-owner", "2026-07-19T00:00:00Z"],
            )
            .unwrap();

        let error = second
            .conn
            .execute(
                "INSERT INTO brains (id, kind, name, owner_user_id, created_at) VALUES (?1, 'personal', ?2, ?3, ?4)",
                params!["personal-second", "Second", "npub-owner", "2026-07-19T00:00:01Z"],
            )
            .unwrap_err();
        assert!(
            matches!(
                error,
                rusqlite::Error::SqliteFailure(inner, _)
                    if inner.code == rusqlite::ErrorCode::ConstraintViolation
            ),
            "the database must enforce one Personal Brain per owner: {error}"
        );
    }

    #[test]
    fn competing_personal_bootstraps_leave_one_brain_and_one_truthful_loser() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("competing-personal-brain.sqlite3");
        let first_store = BrainStore::open(&db).unwrap();
        let second_store = BrainStore::open(&db).unwrap();
        let barrier = Arc::new(Barrier::new(2));

        let results = std::thread::scope(|scope| {
            let first_barrier = Arc::clone(&barrier);
            let first = scope.spawn(move || {
                let mut store = first_store;
                let output =
                    bootstrap_personal_brain("personal-first", "First", "npub-owner").unwrap();
                first_barrier.wait();
                store.create_personal_brain_bootstrap(
                    &output,
                    &[],
                    &UserId::new("npub-agent-first").unwrap(),
                    &UserId::new("npub-owner").unwrap(),
                    "2026-07-19T00:00:00Z",
                )
            });
            let second_barrier = Arc::clone(&barrier);
            let second = scope.spawn(move || {
                let mut store = second_store;
                let output =
                    bootstrap_personal_brain("personal-second", "Second", "npub-owner").unwrap();
                second_barrier.wait();
                store.create_personal_brain_bootstrap(
                    &output,
                    &[],
                    &UserId::new("npub-agent-second").unwrap(),
                    &UserId::new("npub-owner").unwrap(),
                    "2026-07-19T00:00:01Z",
                )
            });
            [first.join().unwrap(), second.join().unwrap()]
        });

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(results.iter().any(|result| {
            matches!(
                result,
                Err(StoreError::BrokenInvariant { reason })
                    if reason == "user already has a personal brain"
            )
        }));

        let store = BrainStore::open(&db).unwrap();
        let count = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM brains WHERE kind = 'personal' AND owner_user_id = ?1",
                params!["npub-owner"],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        for table in ["personal_agents", "personal_agent_audit"] {
            let count = store
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(
                count, 1,
                "the losing bootstrap must leave no partial {table} row"
            );
        }
    }

    #[test]
    fn ordinary_brain_bootstrap_cannot_create_a_vacant_personal_agent_role() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();

        assert_eq!(
            store.create_brain_bootstrap(&output, &[]).unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "Personal Brain bootstrap requires a Personal Agent".to_owned(),
            }
        );
        assert!(
            matches!(
                store.load_brain(&output.brain.id),
                Err(StoreError::MissingBrain { .. })
            ),
            "a rejected vacant bootstrap must not create a Brain"
        );
    }

    #[test]
    fn personal_bootstrap_rolls_back_brain_and_agent_when_identity_alias_insert_fails() {
        let mut store = BrainStore::open_in_memory().unwrap();
        store
            .record_identity_alias(&IdentityAlias {
                npub: UserId::new("npub-existing").unwrap(),
                hex_public_key: "hex-owner".to_owned(),
                preferred_nip05: Some("existing@finite.vip".to_owned()),
                nip05_verified_at: Some("2026-06-23T00:00:00Z".to_owned()),
                nip05_relays: Vec::new(),
                updated_at: "2026-06-23T00:00:00Z".to_owned(),
            })
            .unwrap();
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();
        let aliases = [
            IdentityAlias {
                npub: UserId::new("npub-owner").unwrap(),
                hex_public_key: "hex-owner".to_owned(),
                preferred_nip05: Some("owner@finite.computer".to_owned()),
                nip05_verified_at: Some("2026-06-23T00:00:00Z".to_owned()),
                nip05_relays: Vec::new(),
                updated_at: "2026-06-23T00:00:00Z".to_owned(),
            },
            IdentityAlias {
                npub: UserId::new("npub-agent").unwrap(),
                hex_public_key: "hex-agent".to_owned(),
                preferred_nip05: Some("agent@finite.vip".to_owned()),
                nip05_verified_at: Some("2026-06-23T00:00:00Z".to_owned()),
                nip05_relays: Vec::new(),
                updated_at: "2026-06-23T00:00:00Z".to_owned(),
            },
        ];

        assert!(
            store
                .create_personal_brain_bootstrap_with_identities(
                    &output,
                    &[],
                    &UserId::new("npub-agent").unwrap(),
                    &UserId::new("npub-owner").unwrap(),
                    "2026-06-23T00:00:00Z",
                    &aliases,
                )
                .is_err()
        );
        assert!(matches!(
            store.load_brain(&output.brain.id),
            Err(StoreError::MissingBrain { .. })
        ));
        assert!(
            store
                .load_personal_agent(&output.brain.id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn organization_bootstrap_rolls_back_brain_when_identity_alias_insert_fails() {
        let mut store = BrainStore::open_in_memory().unwrap();
        store
            .record_identity_alias(&IdentityAlias {
                npub: UserId::new("npub-existing").unwrap(),
                hex_public_key: "hex-owner".to_owned(),
                preferred_nip05: Some("existing@finite.vip".to_owned()),
                nip05_verified_at: Some("2026-06-23T00:00:00Z".to_owned()),
                nip05_relays: Vec::new(),
                updated_at: "2026-06-23T00:00:00Z".to_owned(),
            })
            .unwrap();
        let output = finite_brain_core::bootstrap_organization_brain_with_requester(
            "acme",
            "Acme Brain",
            "npub-owner",
            "npub-agent",
        )
        .unwrap();
        let aliases = [
            IdentityAlias {
                npub: UserId::new("npub-owner").unwrap(),
                hex_public_key: "hex-owner".to_owned(),
                preferred_nip05: Some("owner@finite.computer".to_owned()),
                nip05_verified_at: Some("2026-06-23T00:00:00Z".to_owned()),
                nip05_relays: Vec::new(),
                updated_at: "2026-06-23T00:00:00Z".to_owned(),
            },
            IdentityAlias {
                npub: UserId::new("npub-agent").unwrap(),
                hex_public_key: "hex-agent".to_owned(),
                preferred_nip05: Some("agent@finite.vip".to_owned()),
                nip05_verified_at: Some("2026-06-23T00:00:00Z".to_owned()),
                nip05_relays: Vec::new(),
                updated_at: "2026-06-23T00:00:00Z".to_owned(),
            },
        ];

        assert!(
            store
                .create_brain_bootstrap_with_identities(&output, &[], &aliases)
                .is_err()
        );
        assert!(matches!(
            store.load_brain(&output.brain.id),
            Err(StoreError::MissingBrain { .. })
        ));
    }

    #[test]
    fn exact_organization_bootstrap_retry_returns_the_existing_brain() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = finite_brain_core::bootstrap_organization_brain_with_requester(
            "acme",
            "Acme Brain",
            "npub-owner",
            "npub-agent",
        )
        .unwrap();

        store.create_brain_bootstrap(&output, &[]).unwrap();
        store.create_brain_bootstrap(&output, &[]).unwrap();

        let stored = store.load_brain(&output.brain.id).unwrap();
        assert_eq!(stored.brain.id, output.brain.id);
        assert_eq!(stored.brain.name, output.brain.name);
        assert_eq!(stored.brain.members.len(), 2);
        assert_eq!(stored.brain.admins.len(), 2);
        assert!(stored.grants.is_empty());
    }

    #[test]
    fn reused_organization_brain_id_cannot_claim_a_different_bootstrap() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let first = bootstrap_organization_brain("acme", "Acme Brain", "npub-first").unwrap();
        let conflicting =
            bootstrap_organization_brain("acme", "Different Brain", "npub-second").unwrap();

        store.create_brain_bootstrap(&first, &[]).unwrap();
        let error = store.create_brain_bootstrap(&conflicting, &[]).unwrap_err();

        assert_eq!(
            error,
            StoreError::DuplicateId {
                field: "brain_id",
                value: "acme".to_owned(),
            }
        );
        assert_eq!(
            store.load_brain(&first.brain.id).unwrap().brain,
            first.brain
        );
    }

    #[test]
    fn persists_and_reloads_organization_bootstrap() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("brain-sync.sqlite3");
        let output = bootstrap_organization_brain("acme", "Acme", "npub-admin").unwrap();
        let grants = grants_for_required(&output.required_key_grants, "npub-admin");

        {
            let mut store = BrainStore::open(&db).unwrap();
            store.create_brain_bootstrap(&output, &grants).unwrap();
        }

        let store = BrainStore::open(&db).unwrap();
        let stored = store.load_brain(&BrainId::new("acme").unwrap()).unwrap();

        assert_eq!(stored.brain.kind, BrainKind::Organization);
        assert_eq!(stored.brain.members.len(), 1);
        assert_eq!(
            stored.brain.admins,
            vec![UserId::new("npub-admin").unwrap()]
        );
        assert!(stored.brain.folders.is_empty());
        assert!(stored.folder_access.is_empty());
        assert!(stored.grants.is_empty());
        assert_same_grants(&stored.grants, &grants);
    }

    #[test]
    fn bootstrap_rejects_oversized_batches_before_deep_validation() {
        let mut output = bootstrap_organization_brain("acme", "Acme", "npub-admin").unwrap();
        output.brain.folders = vec![strategy_folder(); MAX_BOOTSTRAP_FOLDERS + 1];
        let mut store = BrainStore::open_in_memory().unwrap();

        assert_eq!(
            store.create_brain_bootstrap(&output, &[]).unwrap_err(),
            StoreError::CapacityExceeded {
                limit: "brain_folders".to_owned(),
                max: MAX_BOOTSTRAP_FOLDERS,
                current: MAX_BOOTSTRAP_FOLDERS + 1,
            }
        );
    }

    #[test]
    fn creates_restricted_folder_with_required_grants_transactionally() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&brain_id, &member).unwrap();

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
            .create_folder(&brain_id, &folder, &access_user_ids, &grants)
            .unwrap();
        let stored = store.load_brain(&brain_id).unwrap();

        assert!(stored.brain.folders.iter().any(|stored| stored == &folder));
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
        let brain_id = BrainId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&brain_id, &member).unwrap();
        let before_sequence = store.latest_sequence(&brain_id).unwrap();
        let new_grant = grant(
            "grant-strategy-member",
            "strategy",
            1,
            "npub-admin",
            member.as_str(),
        );

        store
            .grant_folder_access(&brain_id, &folder_id, &member, &new_grant)
            .unwrap();

        let stored = store.load_brain(&brain_id).unwrap();
        assert_eq!(
            stored.folder_access.get(&folder_id),
            Some(&BTreeSet::from([member.clone()]))
        );
        assert!(stored.grants.iter().any(|grant| {
            grant.folder_id == folder_id && grant.key_version == 1 && grant.recipient_npub == member
        }));
        assert_eq!(
            store.latest_sequence(&brain_id).unwrap(),
            before_sequence + 2
        );
        let bootstrap = store.sync_bootstrap(&brain_id).unwrap();
        assert!(bootstrap.control_records.iter().any(|record| {
            record.record_event_id == "grant-strategy-member-key-record"
                && record.record_type == SyncRecordType::FolderKeyGrant
        }));
        assert!(bootstrap.control_records.iter().any(|record| {
            record.record_event_id == "grant-strategy-member-access-record"
                && record.record_type == SyncRecordType::BrainAdminAccessChange
        }));
    }

    #[test]
    fn grants_restricted_folder_key_after_invitation_access_metadata() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let member = UserId::new("npub-invited-member").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        store
            .create_brain_invitation(
                &brain_id,
                "invitation-initial-strategy",
                &member,
                "invite-initial-strategy0123456789ab",
                "/_admin/brain-invitation-links/invite-initial-strategy0123456789ab/accept",
                std::slice::from_ref(&folder_id),
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        store
            .accept_brain_invitation_by_code("invite-initial-strategy0123456789ab", &member, now)
            .unwrap();

        let stored = store.load_brain(&brain_id).unwrap();
        assert_eq!(
            stored.folder_access.get(&folder_id),
            Some(&BTreeSet::from([member.clone()]))
        );
        assert!(!stored.grants.iter().any(|grant| {
            grant.folder_id == folder_id && grant.key_version == 1 && grant.recipient_npub == member
        }));

        store
            .grant_folder_access(
                &brain_id,
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

        let stored = store.load_brain(&brain_id).unwrap();
        assert!(stored.grants.iter().any(|grant| {
            grant.folder_id == folder_id && grant.key_version == 1 && grant.recipient_npub == member
        }));
    }

    #[test]
    fn grants_all_members_folder_key_without_restricted_access_row() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&brain_id, &member).unwrap();

        store
            .grant_folder_access(
                &brain_id,
                &FolderId::new("team-notes").unwrap(),
                &member,
                &grant(
                    "grant-team-notes-member",
                    "team-notes",
                    1,
                    "npub-admin",
                    member.as_str(),
                ),
            )
            .unwrap();

        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            !stored
                .folder_access
                .contains_key(&FolderId::new("team-notes").unwrap())
        );
        assert!(stored.grants.iter().any(|grant| {
            grant.folder_id == FolderId::new("team-notes").unwrap()
                && grant.key_version == 1
                && grant.recipient_npub == member
        }));
    }

    #[test]
    fn grants_admin_only_folder_key_to_existing_admin_without_access_row() {
        let mut store = empty_org_store();
        let brain_id = BrainId::new("acme").unwrap();
        let admin_only = admin_only_folder();
        store
            .create_folder(
                &brain_id,
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
        store.add_member(&brain_id, &admin).unwrap();
        store.add_admin(&brain_id, &admin).unwrap();

        store
            .grant_folder_access(
                &brain_id,
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

        let stored = store.load_brain(&brain_id).unwrap();
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
    fn redundant_current_folder_key_grant_is_an_idempotent_no_op() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let folder_id = FolderId::new("team-notes").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let before = store.load_brain(&brain_id).unwrap();
        let before_sequence = store.latest_sequence(&brain_id).unwrap();

        let outcome = store
            .grant_folder_access(
                &brain_id,
                &folder_id,
                &admin,
                &grant(
                    "grant-team-notes-admin-retry",
                    "team-notes",
                    1,
                    "npub-admin",
                    admin.as_str(),
                ),
            )
            .unwrap();

        assert_eq!(outcome, GrantFolderAccessOutcome::AlreadyHasAccess);
        let after = store.load_brain(&brain_id).unwrap();
        assert_eq!(after.folder_access, before.folder_access);
        assert_eq!(after.grants, before.grants);
        assert_eq!(store.latest_sequence(&brain_id).unwrap(), before_sequence);
    }

    #[test]
    fn folder_grant_rolls_back_authority_when_a_control_record_conflicts() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();
        let brain_id = output.brain.id.clone();
        store
            .create_personal_brain_bootstrap(
                &output,
                &[],
                &UserId::new("npub-agent").unwrap(),
                &UserId::new("npub-owner").unwrap(),
                "2026-06-23T00:00:00Z",
            )
            .unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let folder = Folder {
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Strategy").unwrap(),
            ..strategy_folder()
        };
        store
            .create_folder(
                &brain_id,
                &folder,
                &BTreeSet::new(),
                &[
                    grant(
                        "grant-personal-owner",
                        "strategy",
                        1,
                        "npub-owner",
                        "npub-owner",
                    ),
                    grant(
                        "grant-personal-agent",
                        "strategy",
                        1,
                        "npub-owner",
                        "npub-agent",
                    ),
                ],
            )
            .unwrap();
        let member = UserId::new("npub-member").unwrap();
        let before = store.load_brain(&brain_id).unwrap();
        let before_sequence = store.latest_sequence(&brain_id).unwrap();
        let colliding = folder_access_control_record(
            "event-colliding-access-change",
            SyncRecordType::BrainAdminAccessChange,
            "strategy",
            "npub-owner",
        );
        store.submit_sync_record(&brain_id, &colliding).unwrap();
        let sequence_with_collision = store.latest_sequence(&brain_id).unwrap();
        assert_eq!(sequence_with_collision, before_sequence + 1);

        let new_grant = grant(
            "grant-strategy-member-atomic",
            "strategy",
            1,
            "npub-owner",
            member.as_str(),
        );
        let records = [
            folder_access_control_record(
                "event-new-folder-key-grant",
                SyncRecordType::FolderKeyGrant,
                "strategy",
                "npub-owner",
            ),
            colliding,
        ];

        store
            .grant_folder_access_with_control_records(
                &brain_id, &folder_id, &member, &new_grant, &records,
            )
            .unwrap_err();

        let after = store.load_brain(&brain_id).unwrap();
        assert_eq!(after.brain.members, before.brain.members);
        assert_eq!(after.folder_access, before.folder_access);
        assert_eq!(after.grants, before.grants);
        assert_eq!(
            store.latest_sequence(&brain_id).unwrap(),
            sequence_with_collision
        );
        assert!(
            store
                .sync_bootstrap(&brain_id)
                .unwrap()
                .control_records
                .iter()
                .all(|record| record.record_event_id != "event-new-folder-key-grant")
        );
    }

    #[test]
    fn rejects_admin_only_folder_key_grant_to_non_admin() {
        let mut store = empty_org_store();
        let brain_id = BrainId::new("acme").unwrap();
        let admin_only = admin_only_folder();
        store
            .create_folder(
                &brain_id,
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
        store.add_member(&brain_id, &member).unwrap();

        assert_eq!(
            store
                .grant_folder_access(
                    &brain_id,
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
                reason: "admin-only folder grants require a brain admin target".to_owned()
            }
        );
    }

    #[test]
    fn brain_invitation_is_single_user_single_use_and_retry_safe() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let restricted = FolderId::new("private-project").unwrap();
        let target = UserId::new("npub-target").unwrap();
        let wrong_user = UserId::new("npub-wrong").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        let invitation = store
            .create_brain_invitation(
                &brain_id,
                "invitation-target",
                &target,
                "invite-0123456789abcdef0123456789abcdef",
                "/_admin/brain-invitation-links/invite-0123456789abcdef0123456789abcdef/accept",
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
                .load_available_brain_invitation_by_code(
                    "invite-0123456789abcdef0123456789abcdef",
                    &wrong_user,
                    now,
                )
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "brain invitation"
            }
        );
        assert_eq!(
            store
                .load_available_brain_invitation_by_code(
                    "invite-0123456789abcdef0123456789abcdef",
                    &target,
                    "2026-07-01T00:00:00.000Z",
                )
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "brain invitation"
            }
        );

        let accepted = store
            .accept_brain_invitation_by_code(
                "invite-0123456789abcdef0123456789abcdef",
                &target,
                now,
            )
            .unwrap();
        assert_eq!(accepted.status, LinkStatus::Accepted);
        assert_eq!(accepted.accepted_at.as_deref(), Some(now));
        assert!(!accepted.duplicate_accept);
        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            stored
                .brain
                .members
                .iter()
                .any(|member| member.user_id == target)
        );
        assert_eq!(
            stored.folder_access.get(&restricted),
            Some(&BTreeSet::from([target.clone()]))
        );

        let retry = store
            .accept_brain_invitation_by_code(
                "invite-0123456789abcdef0123456789abcdef",
                &target,
                now,
            )
            .unwrap();
        assert_eq!(retry.status, LinkStatus::Accepted);
        assert!(retry.duplicate_accept);

        let revoked = store
            .revoke_brain_invitation(&brain_id, "invitation-target", &admin, now)
            .unwrap();
        assert_eq!(revoked.status, LinkStatus::Revoked);
        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            stored
                .brain
                .members
                .iter()
                .any(|member| member.user_id == target)
        );
    }

    #[test]
    fn email_brain_invitation_claims_membership_access_and_grants_atomically() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let restricted = FolderId::new("private-project").unwrap();
        let team_notes = FolderId::new("team-notes").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let unwrap_npub = UserId::new("npub-unwrap").unwrap();
        let claimant = UserId::new("npub-claimant").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        let invitation = store
            .create_email_brain_invitation(
                &brain_id,
                "invitation-email",
                " Friend@Example.COM ",
                &unwrap_npub,
                "sha256-bootstrap-payload",
                "{\"kind\":1059}",
                "{\"kind\":30078}",
                "invite-email0123456789abcdef012345",
                "/_admin/brain-invitation-links/invite-email0123456789abcdef012345/accept",
                std::slice::from_ref(&restricted),
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();

        assert_eq!(
            invitation.target_kind,
            BrainInvitationTargetKind::EmailBootstrap
        );
        assert_eq!(invitation.user_id, None);
        assert_eq!(
            invitation.invited_email.as_deref(),
            Some("friend@example.com")
        );
        assert_eq!(invitation.invite_unwrap_npub, Some(unwrap_npub.clone()));
        assert_eq!(
            invitation.initial_folder_access,
            vec![restricted.clone(), team_notes.clone()]
        );
        assert_eq!(
            invitation.bootstrap_scope,
            vec![
                EmailInviteBootstrapScopeFolder {
                    folder_id: restricted.clone(),
                    access: FolderAccessMode::Restricted,
                    key_version: 1,
                },
                EmailInviteBootstrapScopeFolder {
                    folder_id: team_notes.clone(),
                    access: FolderAccessMode::AllMembers,
                    key_version: 1,
                },
            ]
        );

        assert_eq!(
            store
                .claim_email_brain_invitation_by_code(
                    "invite-email0123456789abcdef012345",
                    "friend@example.com",
                    &claimant,
                    &[grant(
                        "claim-grant-team-notes",
                        "team-notes",
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
        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            !stored
                .brain
                .members
                .iter()
                .any(|member| member.user_id == claimant)
        );

        let claim_grants = vec![
            grant(
                "claim-grant-team-notes",
                "team-notes",
                1,
                "npub-claimant",
                "npub-claimant",
            ),
            grant(
                "claim-grant-private-project",
                "private-project",
                1,
                "npub-claimant",
                "npub-claimant",
            ),
        ];
        let claimed = store
            .claim_email_brain_invitation_by_code(
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

        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            stored
                .brain
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
            .claim_email_brain_invitation_by_code(
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
                .claim_email_brain_invitation_by_code(
                    "invite-email0123456789abcdef012345",
                    "friend@example.com",
                    &UserId::new("npub-other-claimant").unwrap(),
                    &[],
                    now,
                )
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "brain invitation"
            }
        );
    }

    #[test]
    fn email_brain_invitation_terminal_states_tombstone_bootstrap_ciphertext() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let restricted = FolderId::new("private-project").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let unwrap_npub = UserId::new("npub-unwrap").unwrap();
        let claimant = UserId::new("npub-claimant").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        let create_invite =
            |store: &mut BrainStore, id: &str, code: &str, email: &str, expires_at: &str| {
                store
                    .create_email_brain_invitation(
                        &brain_id,
                        id,
                        email,
                        &unwrap_npub,
                        "sha256-bootstrap-payload",
                        "{\"kind\":1059}",
                        "{\"kind\":30078}",
                        code,
                        &format!("/_admin/brain-invitation-links/{code}/claim"),
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
            .revoke_brain_invitation(&brain_id, &revoked.id, &admin, "2026-06-24T00:00:00.000Z")
            .unwrap();
        assert_eq!(
            store
                .load_brain_invitation(&revoked.id)
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
        let superseded_old = store.load_brain_invitation(&superseded_old.id).unwrap();
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
            store.claim_email_brain_invitation_by_code(
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
                .load_brain_invitation(&expired.id)
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
                "UPDATE folders SET current_key_version = 2 WHERE brain_id = ?1 AND id = ?2",
                params![brain_id.as_str(), restricted.as_str()],
            )
            .unwrap();
        assert_eq!(
            store
                .claim_email_brain_invitation_by_code(
                    "invite-email-stale01234567890123",
                    "stale@example.com",
                    &claimant,
                    &[
                        grant(
                            "claim-grant-team-notes-stale",
                            "team-notes",
                            1,
                            "npub-claimant",
                            "npub-claimant",
                        ),
                        grant(
                            "claim-grant-private-project-stale",
                            "private-project",
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
                .load_brain_invitation(&stale.id)
                .unwrap()
                .bootstrap_wrapped_event_json,
            None
        );
    }

    #[test]
    fn folder_key_rotation_invalidates_pending_email_bootstrap() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let restricted = FolderId::new("private-project").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let member = UserId::new("npub-member").unwrap();
        let unwrap_npub = UserId::new("npub-unwrap").unwrap();
        let now = "2026-06-23T00:00:00.000Z";
        store.add_member(&brain_id, &member).unwrap();
        store
            .grant_folder_access(
                &brain_id,
                &restricted,
                &member,
                &grant(
                    "grant-private-project-member-rotation",
                    "private-project",
                    1,
                    "npub-admin",
                    member.as_str(),
                ),
            )
            .unwrap();
        let invitation = store
            .create_email_brain_invitation(
                &brain_id,
                "invitation-email-rotation",
                "rotation@example.com",
                &unwrap_npub,
                "sha256-bootstrap-payload",
                "{\"kind\":1059}",
                "{\"kind\":30078}",
                "invite-email-rotation0123456789",
                "/_admin/brain-invitation-links/invite-email-rotation0123456789/claim",
                std::slice::from_ref(&restricted),
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        assert_eq!(invitation.status, LinkStatus::Pending);
        assert!(invitation.bootstrap_wrapped_event_json.is_some());
        let reencrypted_records = store
            .load_current_objects(&brain_id)
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
                &brain_id,
                &restricted,
                &member,
                2,
                &[grant(
                    "grant-private-project-admin-v2",
                    "private-project",
                    2,
                    "npub-admin",
                    "npub-admin",
                )],
                &reencrypted_records,
                "2026-06-24T00:00:00.000Z",
            )
            .unwrap();

        let invalidated = store.load_brain_invitation(&invitation.id).unwrap();
        assert_eq!(invalidated.status, LinkStatus::Revoked);
        assert_eq!(invalidated.bootstrap_wrapped_event_json, None);
        assert_eq!(invalidated.updated_at, "2026-06-24T00:00:00.000Z");
    }

    #[test]
    fn brain_invitation_handles_existing_members_without_stale_invites() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let existing_member = UserId::new("npub-existing-member").unwrap();
        let stale_target = UserId::new("npub-stale-target").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        store.add_member(&brain_id, &existing_member).unwrap();
        assert_eq!(
            store
                .create_brain_invitation(
                    &brain_id,
                    "invitation-existing-member",
                    &existing_member,
                    "invite-existing-member0123456789abcdef",
                    "/_admin/brain-invitation-links/invite-existing-member0123456789abcdef/accept",
                    &[],
                    &admin,
                    "2026-06-30T00:00:00.000Z",
                    now,
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "target is already a brain member".to_owned()
            }
        );

        store
            .create_brain_invitation(
                &brain_id,
                "invitation-stale-member",
                &stale_target,
                "invite-stale-member0123456789abcdef",
                "/_admin/brain-invitation-links/invite-stale-member0123456789abcdef/accept",
                &[],
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        store.add_member(&brain_id, &stale_target).unwrap();

        let visible = store.list_visible_brains(&stale_target).unwrap();
        assert!(visible.iter().any(|brain| brain.id == brain_id));
        assert!(!visible.iter().any(|brain| {
            brain.id == brain_id
                && brain.role == VisibleBrainRole::Invited
                && brain.invite_code.is_some()
        }));

        let accepted = store
            .accept_brain_invitation_by_code(
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
        let brain_id = BrainId::new("acme").unwrap();
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
                &brain_id,
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

        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            stored
                .brain
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
        let stored = store.load_brain(&brain_id).unwrap();
        assert_eq!(
            stored.folder_access.get(&folder_id),
            Some(&BTreeSet::from([recipient]))
        );
    }

    #[test]
    fn personal_agent_can_share_a_restricted_personal_brain_folder() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();
        let owner = UserId::new("npub-owner").unwrap();
        let agent = UserId::new("npub-agent").unwrap();
        let recipient = UserId::new("npub-recipient").unwrap();
        let brain_id = output.brain.id.clone();
        store
            .create_personal_brain_bootstrap(&output, &[], &agent, &owner, "2026-06-23T00:00:00Z")
            .unwrap();
        let folder = Folder {
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Strategy").unwrap(),
            ..strategy_folder()
        };
        store
            .create_folder(
                &brain_id,
                &folder,
                &BTreeSet::new(),
                &[
                    grant(
                        "grant-personal-owner",
                        "strategy",
                        1,
                        agent.as_str(),
                        owner.as_str(),
                    ),
                    grant(
                        "grant-personal-agent",
                        "strategy",
                        1,
                        agent.as_str(),
                        agent.as_str(),
                    ),
                ],
            )
            .unwrap();
        let recipient_grant = grant(
            "grant-personal-recipient",
            "strategy",
            1,
            agent.as_str(),
            recipient.as_str(),
        );

        let share = store
            .create_share_link(
                &brain_id,
                &folder.id,
                "share-link-personal-agent",
                &recipient,
                &agent,
                "2026-06-30T00:00:00Z",
                "/_admin/share-links/share-link-personal-agent/accept",
                &recipient_grant,
                true,
                "2026-06-23T00:00:00Z",
            )
            .unwrap();

        assert_eq!(share.created_by_npub, agent);
        assert_eq!(share.status, LinkStatus::Pending);
    }

    #[test]
    fn vacant_personal_agent_role_requires_only_the_owner_folder_grant() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();
        let owner = UserId::new("npub-owner").unwrap();
        let agent = UserId::new("npub-agent").unwrap();
        let brain_id = output.brain.id.clone();
        store
            .create_personal_brain_bootstrap(&output, &[], &agent, &owner, "2026-06-23T00:00:00Z")
            .unwrap();
        store
            .replace_personal_agent(&brain_id, &owner, None, &[], "2026-06-23T00:01:00Z")
            .unwrap();
        let folder = Folder {
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Private").unwrap(),
            access: FolderAccessMode::Owner,
            ..strategy_folder()
        };
        let owner_grant = grant(
            "grant-personal-owner",
            "strategy",
            1,
            owner.as_str(),
            owner.as_str(),
        );

        store
            .create_folder(&brain_id, &folder, &BTreeSet::new(), &[owner_grant])
            .unwrap();

        let stored = store.load_brain(&brain_id).unwrap();
        assert!(stored.personal_agent.is_none());
        assert_eq!(stored.grants.len(), 1);
        assert_eq!(stored.grants[0].recipient_npub, owner);
    }

    #[test]
    fn folder_subtree_deletion_is_atomic_and_folder_identities_stay_dead() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let child = Folder {
            id: FolderId::new("strategy-child").unwrap(),
            name: DisplayName::new("folder_name", "Child").unwrap(),
            parent_folder_id: Some(FolderId::new("strategy").unwrap()),
            path: SafeRelativePath::new("folder_path", "Strategy/Child").unwrap(),
            ..strategy_folder()
        };
        store
            .create_folder(
                &brain_id,
                &child,
                &BTreeSet::new(),
                &[grant(
                    "grant-strategy-child-admin",
                    "strategy-child",
                    1,
                    admin.as_str(),
                    admin.as_str(),
                )],
            )
            .unwrap();

        assert_eq!(
            store
                .delete_folder_subtree(
                    &brain_id,
                    &FolderId::new("strategy").unwrap(),
                    &admin,
                    2,
                    "event-delete-strategy",
                    r#"{"recordType":"folder_subtree_tombstone"}"#,
                    "2026-06-23T00:00:00Z",
                    APP_SPECIFIC_KIND,
                    None,
                )
                .unwrap_err(),
            StoreError::Conflict {
                reason: "Folder Key version changed before deletion".to_owned(),
                current_revision: Some(1),
            }
        );
        assert!(store.folder_exists(&brain_id, &child.id).unwrap());

        let stale_confirmation = FolderDeletionExpectation {
            folder_ids: [
                FolderId::new("strategy").unwrap(),
                FolderId::new("strategy-child").unwrap(),
            ]
            .into_iter()
            .collect(),
            object_count: 1,
        };
        assert_eq!(
            store
                .delete_folder_subtree(
                    &brain_id,
                    &FolderId::new("strategy").unwrap(),
                    &admin,
                    1,
                    "event-delete-stale-confirmation",
                    r#"{"recordType":"folder_subtree_tombstone"}"#,
                    "2026-06-23T00:00:00Z",
                    APP_SPECIFIC_KIND,
                    Some(&stale_confirmation),
                )
                .unwrap_err(),
            StoreError::Conflict {
                reason: "Folder subtree changed after destructive confirmation".to_owned(),
                current_revision: None,
            }
        );
        assert!(store.folder_exists(&brain_id, &child.id).unwrap());

        let deleted = store
            .delete_folder_subtree(
                &brain_id,
                &FolderId::new("strategy").unwrap(),
                &admin,
                1,
                "event-delete-strategy",
                r#"{"recordType":"folder_subtree_tombstone"}"#,
                "2026-06-23T00:00:00Z",
                APP_SPECIFIC_KIND,
                None,
            )
            .unwrap();
        assert_eq!(deleted.folder_count, 2);
        assert_eq!(
            deleted.work,
            FolderDeletionWork {
                descendants_visited: 2,
                objects_collected: 0,
                audience_collected: 1,
                invitations_scanned: 0,
                invitations_deleted: 0,
                mutation_statements: 8,
                max_statement_parameters: 10,
                retry_attempts: 0,
            }
        );
        let retry = store
            .delete_folder_subtree(
                &brain_id,
                &FolderId::new("strategy").unwrap(),
                &admin,
                1,
                "event-delete-strategy",
                r#"{"recordType":"folder_subtree_tombstone"}"#,
                "2026-06-23T00:00:00Z",
                APP_SPECIFIC_KIND,
                None,
            )
            .unwrap();
        assert!(retry.duplicate);
        assert_eq!(retry.work, FolderDeletionWork::default());
        assert_eq!(retry.folder_count, deleted.folder_count);
        assert_eq!(retry.object_count, deleted.object_count);
        assert!(!store.folder_exists(&brain_id, &child.id).unwrap());
        assert_eq!(
            store
                .create_folder(
                    &brain_id,
                    &strategy_folder(),
                    &BTreeSet::new(),
                    &[grant(
                        "grant-recreated-strategy",
                        "strategy",
                        1,
                        admin.as_str(),
                        admin.as_str(),
                    )],
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "deleted Folder identities cannot be reused".to_owned(),
            }
        );
    }

    #[test]
    fn folder_depth_accepts_exact_boundary_and_rejects_one_over_without_mutation() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let mut parent = FolderId::new("strategy").unwrap();

        for depth in 3..=BRAIN_CAPACITY_ENVELOPE.folder_depth {
            let id = FolderId::new(format!("depth-{depth}")).unwrap();
            let folder = Folder {
                id: id.clone(),
                name: DisplayName::new("folder_name", format!("Depth {depth}")).unwrap(),
                parent_folder_id: Some(parent.clone()),
                path: SafeRelativePath::new("folder_path", format!("Strategy/depth-{depth}"))
                    .unwrap(),
                ..strategy_folder()
            };
            store
                .create_folder(
                    &brain_id,
                    &folder,
                    &BTreeSet::new(),
                    &[grant(
                        &format!("grant-depth-{depth}"),
                        id.as_str(),
                        1,
                        admin.as_str(),
                        admin.as_str(),
                    )],
                )
                .unwrap();
            parent = id;
        }

        let one_over_depth = BRAIN_CAPACITY_ENVELOPE.folder_depth + 1;
        let one_over = Folder {
            id: FolderId::new(format!("depth-{one_over_depth}")).unwrap(),
            name: DisplayName::new("folder_name", format!("Depth {one_over_depth}")).unwrap(),
            parent_folder_id: Some(parent),
            path: SafeRelativePath::new("folder_path", format!("Strategy/depth-{one_over_depth}"))
                .unwrap(),
            ..strategy_folder()
        };
        assert_eq!(
            store
                .create_folder(
                    &brain_id,
                    &one_over,
                    &BTreeSet::new(),
                    &[grant(
                        "grant-depth-one-over",
                        one_over.id.as_str(),
                        1,
                        admin.as_str(),
                        admin.as_str(),
                    )],
                )
                .unwrap_err(),
            StoreError::CapacityExceeded {
                limit: "folder_depth".to_owned(),
                max: BRAIN_CAPACITY_ENVELOPE.folder_depth,
                current: one_over_depth,
            }
        );
        assert!(!store.folder_exists(&brain_id, &one_over.id).unwrap());
        let accepted_depth: usize = store
            .conn
            .query_row(
                "WITH RECURSIVE ancestors(id, depth) AS (
                    SELECT ?1, 1
                    UNION ALL
                    SELECT f.parent_folder_id, ancestors.depth + 1
                    FROM folders f
                    JOIN ancestors ON f.brain_id = ?2 AND f.id = ancestors.id
                    WHERE f.parent_folder_id IS NOT NULL
                 ) SELECT MAX(depth) FROM ancestors",
                params![
                    format!("depth-{}", BRAIN_CAPACITY_ENVELOPE.folder_depth),
                    brain_id.as_str()
                ],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(accepted_depth, BRAIN_CAPACITY_ENVELOPE.folder_depth);
    }

    #[test]
    fn folder_subtree_deletion_fails_closed_on_corrupt_invitation_scope() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let invitee = UserId::new("npub-invitee").unwrap();
        store
            .create_brain_invitation(
                &brain_id,
                "invite-corrupt-scope",
                &invitee,
                "invite-code-corrupt-scope",
                "/invite/corrupt-scope",
                &[FolderId::new("strategy").unwrap()],
                &admin,
                "2026-07-01T00:00:00Z",
                "2026-06-23T00:00:00Z",
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE brain_invitations SET initial_folder_access_json = '{' WHERE id = ?1",
                params!["invite-corrupt-scope"],
            )
            .unwrap();

        assert_eq!(
            store
                .delete_folder_subtree(
                    &brain_id,
                    &FolderId::new("strategy").unwrap(),
                    &admin,
                    1,
                    "event-delete-corrupt-scope",
                    r#"{"recordType":"folder_subtree_tombstone"}"#,
                    "2026-06-23T00:00:00Z",
                    APP_SPECIFIC_KIND,
                    None,
                )
                .unwrap_err(),
            StoreError::InvalidRecord {
                reason: "stored Brain Invitation Folder scope is invalid".to_owned(),
            }
        );
        assert!(
            store
                .folder_exists(&brain_id, &FolderId::new("strategy").unwrap())
                .unwrap()
        );
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM brain_invitations WHERE id = ?1",
                    params!["invite-corrupt-scope"],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );
        assert!(
            store
                .folder_deletion_replay(&brain_id, &FolderId::new("strategy").unwrap())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn empty_personal_brain_agent_replacement_is_owner_only_and_atomic() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();
        let owner = UserId::new("npub-owner").unwrap();
        let old_agent = UserId::new("npub-old-agent").unwrap();
        let new_agent = UserId::new("npub-new-agent").unwrap();
        store
            .create_personal_brain_bootstrap(
                &output,
                &[],
                &old_agent,
                &owner,
                "2026-06-23T00:00:00Z",
            )
            .unwrap();

        store
            .replace_personal_agent(
                &output.brain.id,
                &owner,
                Some(&new_agent),
                &[],
                "2026-06-23T00:01:00Z",
            )
            .unwrap();
        assert_eq!(
            store
                .load_personal_agent(&output.brain.id)
                .unwrap()
                .unwrap()
                .agent_npub,
            new_agent
        );
        assert!(
            store
                .replace_personal_agent(
                    &output.brain.id,
                    &old_agent,
                    None,
                    &[],
                    "2026-06-23T00:02:00Z",
                )
                .is_err()
        );
    }

    #[test]
    fn personal_agent_replacement_preserves_every_required_folder_recipient() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();
        let brain_id = output.brain.id.clone();
        let owner = UserId::new("npub-owner").unwrap();
        let old_agent = UserId::new("npub-old-agent").unwrap();
        let new_agent = UserId::new("npub-new-agent").unwrap();
        let collaborator = UserId::new("npub-collaborator").unwrap();
        store
            .create_personal_brain_bootstrap(
                &output,
                &[],
                &old_agent,
                &owner,
                "2026-06-23T00:00:00Z",
            )
            .unwrap();
        let folder = Folder {
            access: FolderAccessMode::Restricted,
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Strategy").unwrap(),
            ..strategy_folder()
        };
        store
            .create_folder(
                &brain_id,
                &folder,
                &BTreeSet::new(),
                &[
                    grant(
                        "grant-owner-v1",
                        "strategy",
                        1,
                        owner.as_str(),
                        owner.as_str(),
                    ),
                    grant(
                        "grant-agent-v1",
                        "strategy",
                        1,
                        owner.as_str(),
                        old_agent.as_str(),
                    ),
                ],
            )
            .unwrap();
        store
            .grant_folder_access(
                &brain_id,
                &folder.id,
                &collaborator,
                &grant(
                    "grant-collaborator-v1",
                    "strategy",
                    1,
                    owner.as_str(),
                    collaborator.as_str(),
                ),
            )
            .unwrap();

        let grants = vec![
            grant(
                "grant-owner-v2",
                "strategy",
                2,
                owner.as_str(),
                owner.as_str(),
            ),
            grant(
                "grant-agent-v2",
                "strategy",
                2,
                owner.as_str(),
                new_agent.as_str(),
            ),
            grant(
                "grant-collaborator-v2",
                "strategy",
                2,
                owner.as_str(),
                collaborator.as_str(),
            ),
        ];
        let rotation_for = |grants: Vec<FolderKeyGrantMetadata>| {
            let mut control_records = grants
                .iter()
                .map(|grant| {
                    let SyncRecordInput::Control(record) = folder_access_control_record(
                        &format!("{}-control", grant.id),
                        SyncRecordType::FolderKeyGrant,
                        "strategy",
                        owner.as_str(),
                    ) else {
                        unreachable!()
                    };
                    record
                })
                .collect::<Vec<_>>();
            let SyncRecordInput::Control(access_record) = folder_access_control_record(
                &format!("event-replace-agent-{}", grants.len()),
                SyncRecordType::BrainAdminAccessChange,
                "strategy",
                owner.as_str(),
            ) else {
                unreachable!()
            };
            control_records.push(access_record);
            PersonalAgentFolderRotation {
                folder_id: folder.id.clone(),
                new_key_version: 2,
                grants,
                reencrypted_records: vec![],
                control_records,
            }
        };

        let before = store.load_brain(&brain_id).unwrap();
        let incomplete = vec![grants[0].clone(), grants[1].clone()];
        assert_eq!(
            store
                .replace_personal_agent(
                    &brain_id,
                    &owner,
                    Some(&new_agent),
                    &[rotation_for(incomplete)],
                    "2026-06-23T00:01:00Z",
                )
                .unwrap_err(),
            StoreError::MissingRequiredGrant {
                recipient_user_id: collaborator.to_string(),
            }
        );
        assert_eq!(store.load_brain(&brain_id).unwrap(), before);

        let mut excessive = grants.clone();
        excessive.push(grant(
            "grant-unrequired-v2",
            "strategy",
            2,
            owner.as_str(),
            "npub-unrequired",
        ));
        assert_eq!(
            store
                .replace_personal_agent(
                    &brain_id,
                    &owner,
                    Some(&new_agent),
                    &[rotation_for(excessive)],
                    "2026-06-23T00:01:00Z",
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "grant recipients must exactly match required recipients".to_owned(),
            }
        );
        assert_eq!(store.load_brain(&brain_id).unwrap(), before);

        store
            .replace_personal_agent(
                &brain_id,
                &owner,
                Some(&new_agent),
                &[rotation_for(grants)],
                "2026-06-23T00:01:00Z",
            )
            .unwrap();

        let stored = store.load_brain(&brain_id).unwrap();
        let current_recipients = stored
            .grants
            .iter()
            .filter(|grant| grant.folder_id.as_str() == "strategy" && grant.key_version == 2)
            .map(|grant| grant.recipient_npub.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            current_recipients,
            BTreeSet::from([owner, new_agent, collaborator])
        );
        assert!(!current_recipients.contains(&old_agent));
    }

    #[test]
    fn rotation_fanout_limits_reject_before_store_mutation() {
        let mut personal_store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();
        let owner = UserId::new("npub-owner").unwrap();
        let old_agent = UserId::new("npub-old-agent").unwrap();
        personal_store
            .create_personal_brain_bootstrap(
                &output,
                &[],
                &old_agent,
                &owner,
                "2026-06-23T00:00:00Z",
            )
            .unwrap();
        let before = personal_store.load_brain(&output.brain.id).unwrap();
        let excessive_rotations = (0..=MAX_PERSONAL_AGENT_ROTATION_FOLDERS)
            .map(|index| PersonalAgentFolderRotation {
                folder_id: FolderId::new(format!("folder-{index}")).unwrap(),
                new_key_version: 2,
                grants: vec![],
                reencrypted_records: vec![],
                control_records: vec![],
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            personal_store.replace_personal_agent(
                &output.brain.id,
                &owner,
                None,
                &excessive_rotations,
                "2026-06-23T00:01:00Z",
            ),
            Err(StoreError::Core(CoreError::RotationFanoutLimitExceeded {
                resource: "Folder rotations",
                ..
            }))
        ));
        assert_eq!(personal_store.load_brain(&output.brain.id).unwrap(), before);

        let mut access_store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let member = UserId::new("npub-member").unwrap();
        let before = access_store.load_brain(&brain_id).unwrap();
        let before_sequence = access_store.latest_sequence(&brain_id).unwrap();
        let excessive_grants = (0..=MAX_FOLDER_ACCESS_REMOVAL_GRANTS)
            .map(|index| {
                grant(
                    &format!("grant-limit-{index}"),
                    "strategy",
                    2,
                    "npub-admin",
                    "npub-admin",
                )
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            access_store.rotate_folder_key_for_access_removal(
                &brain_id,
                &folder_id,
                &member,
                2,
                &excessive_grants,
                &[],
                "2026-06-23T00:01:00Z",
            ),
            Err(StoreError::Core(CoreError::RotationFanoutLimitExceeded {
                resource: "grants per Folder rotation",
                ..
            }))
        ));
        assert_eq!(access_store.load_brain(&brain_id).unwrap(), before);
        assert_eq!(
            access_store.latest_sequence(&brain_id).unwrap(),
            before_sequence
        );
    }

    #[test]
    fn encrypted_brain_export_filters_payloads_grants_and_access_state() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&brain_id, &member).unwrap();
        store
            .create_folder(
                &brain_id,
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
                &brain_id,
                &revision_record_for(
                    "team-notes",
                    "event-team-notes-create",
                    "obj_000000000101",
                    1,
                    None,
                    "team-notes payload",
                ),
            )
            .unwrap();
        store
            .submit_sync_record(
                &brain_id,
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

        let member_export = store.encrypted_brain_export(&brain_id, &member).unwrap();
        assert_eq!(member_export.version, "finite-brain-export-v1");
        assert!(member_export.key_grants.is_empty());
        assert_eq!(member_export.access_state.members, vec![member.clone()]);
        assert!(member_export.access_state.admins.is_empty());
        let team_notes_export = member_export
            .objects
            .iter()
            .find(|object| object.folder_id == FolderId::new("team-notes").unwrap())
            .unwrap();
        assert!(!team_notes_export.opaque);
        assert!(
            team_notes_export
                .payload_json
                .as_ref()
                .unwrap()
                .contains("team-notes")
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

        let admin_export = store.encrypted_brain_export(&brain_id, &admin).unwrap();
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
        let brain_id = BrainId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let target = UserId::new("npub-target").unwrap();

        assert_eq!(
            store
                .create_brain_invitation(
                    &brain_id,
                    "invitation-bad-time",
                    &target,
                    "invite-bad-time",
                    "/_admin/brain-invitation-links/invite-bad-time/accept",
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
        let brain_id = BrainId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";
        let invite_target = UserId::new("npub-invite-target").unwrap();
        store
            .create_brain_invitation(
                &brain_id,
                "invitation-revoked",
                &invite_target,
                "invite-revoked0123456789abcdef012345",
                "/_admin/brain-invitation-links/invite-revoked0123456789abcdef012345/accept",
                &[],
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        store
            .revoke_brain_invitation(&brain_id, "invitation-revoked", &admin, now)
            .unwrap();
        assert_eq!(
            store
                .accept_brain_invitation_by_code(
                    "invite-revoked0123456789abcdef012345",
                    &invite_target,
                    now,
                )
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "brain invitation"
            }
        );

        let expired_target = UserId::new("npub-expired-target").unwrap();
        store
            .create_brain_invitation(
                &brain_id,
                "invitation-expired",
                &expired_target,
                "invite-expired0123456789abcdef012345",
                "/_admin/brain-invitation-links/invite-expired0123456789abcdef012345/accept",
                &[],
                &admin,
                "2026-01-01T00:00:00.000Z",
                now,
            )
            .unwrap();
        assert_eq!(
            store
                .accept_brain_invitation_by_code(
                    "invite-expired0123456789abcdef012345",
                    &expired_target,
                    now,
                )
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "brain invitation"
            }
        );

        let share_recipient = UserId::new("npub-share-revoked").unwrap();
        store
            .create_share_link(
                &brain_id,
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
        let source_brain_id = BrainId::new("acme").unwrap();
        let source_folder_id = FolderId::new("strategy").unwrap();
        let destination_brain_id = BrainId::new("dest").unwrap();
        let source_admin = UserId::new("npub-admin").unwrap();
        let destination_admin = UserId::new("npub-dest-admin").unwrap();
        let destination_member = UserId::new("npub-dest-member").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        store
            .mark_shared_folder_source(&source_brain_id, &source_folder_id)
            .unwrap();
        let source = store.load_brain(&source_brain_id).unwrap();
        assert!(
            source
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == source_folder_id)
                .unwrap()
                .shared_folder_source
        );

        let invitation = store
            .create_shared_folder_invitation(
                &source_brain_id,
                &source_folder_id,
                &destination_brain_id,
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
        let source = store.load_brain(&source_brain_id).unwrap();
        assert_eq!(
            source.folder_access.get(&source_folder_id),
            Some(&BTreeSet::from([destination_admin.clone()]))
        );
        assert_eq!(
            store
                .mounted_folder_projection(&destination_brain_id, &destination_admin)
                .unwrap()[0]
                .state,
            MountedFolderState::Available
        );

        store
            .add_member(&destination_brain_id, &destination_member)
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
                .mounted_folder_projection(&destination_brain_id, &destination_member)
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
                .mounted_folder_projection(&destination_brain_id, &destination_member)
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
                .mounted_folder_projection(&destination_brain_id, &destination_admin)
                .unwrap()[0]
                .state,
            MountedFolderState::Revoked
        );
    }

    #[test]
    fn sqlite_full_lifecycle_invite_share_sync_revoke_and_filter_visibility() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("finite-brain.sqlite3");
        let source_brain_id = BrainId::new("acme").unwrap();
        let source_folder_id = FolderId::new("strategy").unwrap();
        let destination_brain_id = BrainId::new("dest").unwrap();
        let source_admin = UserId::new("npub-admin").unwrap();
        let destination_admin = UserId::new("npub-dest-admin").unwrap();
        let destination_member = UserId::new("npub-dest-member").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        {
            let mut store = BrainStore::open(&db).unwrap();
            bootstrap_org_and_strategy_folder(&mut store);
            bootstrap_org_named(&mut store, "dest", "Dest", "npub-dest-admin");

            store
                .create_brain_invitation(
                    &destination_brain_id,
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
                .accept_brain_invitation_by_code("invite-dest-member", &destination_member, now)
                .unwrap();

            store
                .mark_shared_folder_source(&source_brain_id, &source_folder_id)
                .unwrap();
            store
                .submit_sync_record(
                    &source_brain_id,
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
                    &source_brain_id,
                    &source_folder_id,
                    &destination_brain_id,
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
                .mounted_folder_projection(&destination_brain_id, &destination_member)
                .unwrap();
            assert_eq!(member_projection[0].state, MountedFolderState::Available);

            let member_export = store
                .encrypted_brain_export(&source_brain_id, &destination_member)
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
                    .sync_bootstrap(&source_brain_id)
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
                .mounted_folder_projection(&destination_brain_id, &destination_member)
                .unwrap();
            assert_eq!(locked_projection[0].state, MountedFolderState::Locked);

            let filtered_export = store
                .encrypted_brain_export(&source_brain_id, &destination_member)
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
                .mounted_folder_projection(&destination_brain_id, &destination_admin)
                .unwrap();
            assert_eq!(revoked_projection[0].state, MountedFolderState::Revoked);
            assert_eq!(
                store
                    .sync_bootstrap(&source_brain_id)
                    .unwrap()
                    .latest_sequence,
                3
            );
        }
    }

    #[test]
    fn removing_restricted_folder_access_requires_rotation_and_reencrypts_live_objects() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&brain_id, &member).unwrap();
        store
            .grant_folder_access(
                &brain_id,
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
                &brain_id,
                &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
            )
            .unwrap();

        store
            .rotate_folder_key_for_access_removal(
                &brain_id,
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

        let stored = store.load_brain(&brain_id).unwrap();
        let folder = stored
            .brain
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

        let bootstrap = store.sync_bootstrap(&brain_id).unwrap();
        assert_eq!(bootstrap.latest_sequence, 4);
        assert_eq!(bootstrap.objects[0].revision, 2);
        assert_eq!(
            bootstrap.objects[0].payload_json,
            "{\"body\":\"reencrypted\"}"
        );
    }

    #[test]
    fn access_removal_rotation_rolls_back_when_reencryption_or_grants_are_incomplete() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let folder_id = FolderId::new("strategy").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&brain_id, &member).unwrap();
        store
            .grant_folder_access(
                &brain_id,
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
                &brain_id,
                &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
            )
            .unwrap();

        assert_eq!(
            store
                .rotate_folder_key_for_access_removal(
                    &brain_id,
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
                    &brain_id,
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

        let stored = store.load_brain(&brain_id).unwrap();
        let folder = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .unwrap();
        assert_eq!(folder.current_key_version, 1);
        assert_eq!(
            stored.folder_access.get(&folder_id),
            Some(&BTreeSet::from([member]))
        );
        assert_eq!(store.sync_bootstrap(&brain_id).unwrap().latest_sequence, 3);
    }

    #[test]
    fn rejects_missing_required_grant_without_partial_folder() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&brain_id, &member).unwrap();

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
                .create_folder(&brain_id, &folder, &access_user_ids, &grants)
                .unwrap_err(),
            StoreError::MissingRequiredGrant {
                recipient_user_id: "npub-member".to_owned()
            }
        );
        assert!(!store.folder_exists(&brain_id, &folder.id).unwrap());
    }

    #[test]
    fn rolls_back_folder_creation_when_grant_insert_fails() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        assert!(store.grant_exists("grant-team-notes-npub-admin").unwrap());

        let folder = strategy_folder();
        let grants = vec![grant(
            "grant-team-notes-npub-admin",
            "strategy",
            1,
            "npub-admin",
            "npub-admin",
        )];

        assert!(matches!(
            store
                .create_folder(&brain_id, &folder, &BTreeSet::new(), &grants)
                .unwrap_err(),
            StoreError::DuplicateId {
                field: "folder_key_grant_id",
                ..
            }
        ));
        assert!(!store.folder_exists(&brain_id, &folder.id).unwrap());
    }

    #[test]
    fn detects_and_repairs_setup_incomplete_folder_across_restart() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("brain-sync.sqlite3");
        let brain_id = BrainId::new("acme").unwrap();
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
            let output = bootstrap_organization_brain("acme", "Acme", "npub-admin").unwrap();
            let bootstrap_grants = grants_for_required(&output.required_key_grants, "npub-admin");
            store
                .create_brain_bootstrap(&output, &bootstrap_grants)
                .unwrap();
            add_access_test_folders(&mut store);
            store
                .insert_setup_incomplete_folder_for_repair(&brain_id, &folder, &BTreeSet::new())
                .unwrap();
        }

        {
            let mut store = BrainStore::open(&db).unwrap();
            let stored = store.load_brain(&brain_id).unwrap();
            assert_eq!(
                stored.setup_incomplete_folder_ids,
                BTreeSet::from([folder.id.clone()])
            );

            store
                .finish_folder_setup(&brain_id, &folder.id, &grants)
                .unwrap();
        }

        let store = BrainStore::open(&db).unwrap();
        let stored = store.load_brain(&brain_id).unwrap();
        assert!(stored.setup_incomplete_folder_ids.is_empty());
        assert!(stored.grants.contains(&grants[0]));
    }

    #[test]
    fn finish_setup_rejects_non_empty_setup_incomplete_folder() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let folder = strategy_folder();
        store
            .insert_setup_incomplete_folder_for_repair(&brain_id, &folder, &BTreeSet::new())
            .unwrap();
        store
            .submit_sync_record(
                &brain_id,
                &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
            )
            .unwrap();

        assert_eq!(
            store
                .finish_folder_setup(
                    &brain_id,
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
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();

        let mut missing_parent = strategy_folder();
        missing_parent.parent_folder_id = Some(FolderId::new("missing").unwrap());
        missing_parent.path = SafeRelativePath::new("folder_path", "Missing/Strategy").unwrap();
        assert_eq!(
            store
                .create_folder(
                    &brain_id,
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
            .create_folder(&brain_id, &folder, &BTreeSet::new(), &grants)
            .unwrap();
        assert_eq!(
            store
                .create_folder(
                    &brain_id,
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
                .add_admin(&brain_id, &UserId::new("npub-non-member").unwrap())
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "brain admin must already be a brain member".to_owned()
            }
        );

        let bad_issuer_folder = Folder {
            id: FolderId::new("bad-issuer-strategy").unwrap(),
            name: DisplayName::new("folder_name", "Bad Issuer Strategy").unwrap(),
            path: SafeRelativePath::new("folder_path", "team-notes/Bad Issuer Strategy").unwrap(),
            ..strategy_folder()
        };
        assert_eq!(
            store
                .create_folder(
                    &brain_id,
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
                reason: "organization folder grants must be issued by a brain admin".to_owned()
            }
        );
        assert!(
            !store
                .folder_exists(&brain_id, &bad_issuer_folder.id)
                .unwrap()
        );
    }

    #[test]
    fn rejects_unscoped_personal_member_mutation() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();
        let grants = grants_for_required(&output.required_key_grants, "npub-owner");
        store
            .create_personal_brain_bootstrap(
                &output,
                &grants,
                &UserId::new("npub-agent").unwrap(),
                &UserId::new("npub-owner").unwrap(),
                "2026-06-23T00:00:00Z",
            )
            .unwrap();
        let brain_id = BrainId::new("personal").unwrap();
        let member = UserId::new("npub-member").unwrap();

        assert_eq!(
            store.add_member(&brain_id, &member).unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "member/admin mutation requires an organization brain".to_owned()
            }
        );
    }

    #[test]
    fn personal_member_is_removed_when_their_last_folder_scope_is_removed() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_personal_brain("personal", "Austin", "npub-owner").unwrap();
        let grants = grants_for_required(&output.required_key_grants, "npub-owner");
        store
            .create_personal_brain_bootstrap(
                &output,
                &grants,
                &UserId::new("npub-agent").unwrap(),
                &UserId::new("npub-owner").unwrap(),
                "2026-06-23T00:00:00Z",
            )
            .unwrap();
        let brain_id = BrainId::new("personal").unwrap();
        let member = UserId::new("npub-member").unwrap();
        let folder = Folder {
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Strategy").unwrap(),
            ..strategy_folder()
        };
        store
            .create_folder(
                &brain_id,
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
                    grant(
                        "grant-personal-strategy-agent",
                        "strategy",
                        1,
                        "npub-owner",
                        "npub-agent",
                    ),
                ],
            )
            .unwrap();

        store
            .rotate_folder_key_for_access_removal(
                &brain_id,
                &folder.id,
                &member,
                2,
                &[
                    grant(
                        "grant-personal-strategy-owner-v2",
                        "strategy",
                        2,
                        "npub-owner",
                        "npub-owner",
                    ),
                    grant(
                        "grant-personal-strategy-agent-v2",
                        "strategy",
                        2,
                        "npub-owner",
                        "npub-agent",
                    ),
                ],
                &[],
                "2026-07-13T00:00:00.000Z",
            )
            .unwrap();

        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            !stored
                .brain
                .members
                .iter()
                .any(|stored_member| stored_member.user_id == member)
        );
        assert!(store.list_visible_brains(&member).unwrap().is_empty());
    }

    #[test]
    fn removes_members_and_admins_without_breaking_admin_invariant() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let member = UserId::new("npub-member").unwrap();
        store.add_member(&brain_id, &member).unwrap();
        store.add_admin(&brain_id, &member).unwrap();

        store.remove_admin(&brain_id, &member).unwrap();
        assert_eq!(
            store
                .remove_admin(&brain_id, &UserId::new("npub-admin").unwrap())
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "organization brain must keep at least one admin".to_owned()
            }
        );

        store.remove_member(&brain_id, &member).unwrap();
        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            !stored
                .brain
                .members
                .iter()
                .any(|stored| stored.user_id == member)
        );
    }

    #[test]
    fn removing_member_requires_admin_and_restricted_access_cleanup_first() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        assert_eq!(
            store.remove_member(&brain_id, &admin).unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "remove admin role before removing member".to_owned()
            }
        );

        let member = UserId::new("npub-member").unwrap();
        store.add_member(&brain_id, &member).unwrap();
        store
            .grant_folder_access(
                &brain_id,
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
            store.remove_member(&brain_id, &member).unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "remove restricted folder access before removing member".to_owned()
            }
        );
    }

    #[test]
    fn sync_create_update_and_delete_updates_current_projection() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let object_id = "obj_000000000001";

        assert_eq!(
            store
                .submit_sync_record(
                    &brain_id,
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
                    &brain_id,
                    &revision_record("event-update-1", object_id, 2, Some(1), "update")
                )
                .unwrap()
                .sequence,
            2
        );
        assert_eq!(
            store
                .submit_sync_record(
                    &brain_id,
                    &tombstone_record("event-delete-1", object_id, 3, 2)
                )
                .unwrap()
                .sequence,
            3
        );

        let bootstrap = store.sync_bootstrap(&brain_id).unwrap();
        assert_eq!(bootstrap.latest_sequence, 3);
        assert_eq!(bootstrap.object_count, 1);
        assert_eq!(bootstrap.objects[0].revision, 3);
        assert!(bootstrap.objects[0].deleted);
        assert_eq!(bootstrap.objects[0].payload_json, "{\"body\":\"delete\"}");
    }

    #[test]
    fn sync_duplicate_event_returns_existing_sequence() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let record = revision_record("event-create-duplicate", "obj_000000000001", 1, None, "one");

        assert_eq!(
            store.submit_sync_record(&brain_id, &record).unwrap(),
            SubmitRecordOutcome {
                sequence: 1,
                duplicate: false
            }
        );
        assert_eq!(
            store.submit_sync_record(&brain_id, &record).unwrap(),
            SubmitRecordOutcome {
                sequence: 1,
                duplicate: true
            }
        );

        let pull = store.pull_sync_records(&brain_id, 0, 10).unwrap();
        assert_eq!(pull.count, 1);
        assert_eq!(pull.latest_sequence, 1);
    }

    #[test]
    fn sync_rejects_stale_base_revision_and_existing_create() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let object_id = "obj_000000000001";

        store
            .submit_sync_record(
                &brain_id,
                &revision_record("event-create-1", object_id, 1, None, "create"),
            )
            .unwrap();
        store
            .submit_sync_record(
                &brain_id,
                &revision_record("event-update-wins", object_id, 2, Some(1), "winner"),
            )
            .unwrap();

        assert_eq!(
            store
                .submit_sync_record(
                    &brain_id,
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
                    &brain_id,
                    &revision_record("event-create-again", object_id, 1, None, "again"),
                )
                .unwrap_err(),
            StoreError::Conflict {
                reason: "object already exists".to_owned(),
                current_revision: Some(2)
            }
        );
        assert_eq!(store.sync_bootstrap(&brain_id).unwrap().latest_sequence, 2);
    }

    #[test]
    fn sync_rejects_non_monotonic_revision() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        let object_id = "obj_000000000001";

        store
            .submit_sync_record(
                &brain_id,
                &revision_record("event-create-1", object_id, 1, None, "create"),
            )
            .unwrap();

        assert_eq!(
            store
                .submit_sync_record(
                    &brain_id,
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
        let brain_id = BrainId::new("acme").unwrap();

        for (index, object_id) in ["obj_000000000001", "obj_000000000002", "obj_000000000003"]
            .into_iter()
            .enumerate()
        {
            store
                .submit_sync_record(
                    &brain_id,
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

        let first = store.pull_sync_records(&brain_id, 0, 2).unwrap();
        assert_eq!(first.count, 2);
        assert!(first.has_more);
        assert_eq!(first.next_sequence, 2);
        assert_eq!(first.latest_sequence, 3);

        let second = store
            .pull_sync_records(&brain_id, first.next_sequence, 2)
            .unwrap();
        assert_eq!(second.count, 1);
        assert!(!second.has_more);
        assert_eq!(second.next_sequence, 3);
        assert_eq!(second.records[0].sequence, 3);
    }

    #[test]
    fn sync_pull_caps_large_client_limits() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();

        for index in 1..=(MAX_PULL_LIMIT + 2) {
            let object_id = format!("obj_{index:012}");
            store
                .submit_sync_record(
                    &brain_id,
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

        let pull = store.pull_sync_records(&brain_id, 0, u64::MAX).unwrap();
        assert_eq!(pull.count, MAX_PULL_LIMIT as usize);
        assert!(pull.has_more);
        assert_eq!(pull.next_sequence, MAX_PULL_LIMIT);
        assert_eq!(pull.latest_sequence, MAX_PULL_LIMIT + 2);
    }

    #[test]
    fn sync_cursor_expiry_requires_rebootstrap() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        store
            .submit_sync_record(
                &brain_id,
                &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
            )
            .unwrap();
        store.set_retention_floor(&brain_id, 1).unwrap();

        assert_eq!(
            store.pull_sync_records(&brain_id, 0, 10).unwrap_err(),
            StoreError::RebootstrapRequired { retention_floor: 1 }
        );
        assert_eq!(store.pull_sync_records(&brain_id, 1, 10).unwrap().count, 0);
    }

    #[test]
    fn sync_projection_survives_restart_and_can_rebuild() {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("brain-sync.sqlite3");
        let brain_id = BrainId::new("acme").unwrap();

        {
            let mut store = BrainStore::open(&db).unwrap();
            bootstrap_org_and_strategy_folder(&mut store);
            store
                .submit_sync_record(
                    &brain_id,
                    &revision_record("event-create-1", "obj_000000000001", 1, None, "create"),
                )
                .unwrap();
        }

        {
            let mut store = BrainStore::open(&db).unwrap();
            assert_eq!(store.sync_bootstrap(&brain_id).unwrap().object_count, 1);
            store
                .conn
                .execute(
                    "DELETE FROM current_encrypted_brain_objects WHERE brain_id = ?1",
                    params![brain_id.as_str()],
                )
                .unwrap();
            assert_eq!(store.sync_bootstrap(&brain_id).unwrap().object_count, 0);

            store.rebuild_current_projection(&brain_id).unwrap();
            let bootstrap = store.sync_bootstrap(&brain_id).unwrap();
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
        let brain_id = BrainId::new("acme").unwrap();
        let object_id = "obj_000000000001";

        {
            let mut store = BrainStore::open(&source_db).unwrap();
            bootstrap_org_and_strategy_folder(&mut store);
            store
                .submit_sync_record(
                    &brain_id,
                    &revision_record("event-create-backup", object_id, 1, None, "create"),
                )
                .unwrap();
            store
                .submit_sync_record(
                    &brain_id,
                    &revision_record("event-update-backup", object_id, 2, Some(1), "update"),
                )
                .unwrap();
        }

        std::fs::copy(&source_db, &restored_db).unwrap();

        let mut restored = BrainStore::open(&restored_db).unwrap();
        let bootstrap = restored.sync_bootstrap(&brain_id).unwrap();
        assert_eq!(bootstrap.latest_sequence, 2);
        assert_eq!(bootstrap.object_count, 1);
        assert_eq!(bootstrap.objects[0].revision, 2);

        restored
            .conn
            .execute(
                "DELETE FROM current_encrypted_brain_objects WHERE brain_id = ?1",
                params![brain_id.as_str()],
            )
            .unwrap();
        assert_eq!(restored.sync_bootstrap(&brain_id).unwrap().object_count, 0);

        restored.rebuild_current_projection(&brain_id).unwrap();
        let rebuilt = restored.sync_bootstrap(&brain_id).unwrap();
        assert_eq!(rebuilt.latest_sequence, 2);
        assert_eq!(rebuilt.object_count, 1);
        assert_eq!(rebuilt.objects[0].payload_json, "{\"body\":\"update\"}");
    }

    fn empty_org_store() -> BrainStore {
        let mut store = BrainStore::open_in_memory().unwrap();
        bootstrap_org(&mut store);
        store
    }

    fn org_store_with_access_test_folders() -> BrainStore {
        let mut store = empty_org_store();
        add_access_test_folders(&mut store);
        store
    }

    fn store_with_strategy_folder() -> BrainStore {
        let mut store = BrainStore::open_in_memory().unwrap();
        bootstrap_org_and_strategy_folder(&mut store);
        store
    }

    fn bootstrap_org_and_strategy_folder(store: &mut BrainStore) {
        bootstrap_org(store);
        add_access_test_folders(store);
        let brain_id = BrainId::new("acme").unwrap();
        store
            .create_folder(
                &brain_id,
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
        let output = bootstrap_organization_brain("acme", "Acme", "npub-admin").unwrap();
        let grants = grants_for_required(&output.required_key_grants, "npub-admin");
        store.create_brain_bootstrap(&output, &grants).unwrap();
    }

    fn add_access_test_folders(store: &mut BrainStore) {
        let brain_id = BrainId::new("acme").unwrap();
        for folder in [
            Folder {
                id: FolderId::new("team-notes").unwrap(),
                name: DisplayName::new("folder_name", "Team Notes").unwrap(),
                role: FolderRole::General,
                access: FolderAccessMode::AllMembers,
                parent_folder_id: None,
                path: SafeRelativePath::new("folder_path", "Team Notes").unwrap(),
                current_key_version: 1,
                shared_folder_source: false,
            },
            Folder {
                id: FolderId::new("private-project").unwrap(),
                name: DisplayName::new("folder_name", "Private Project").unwrap(),
                role: FolderRole::Folder,
                access: FolderAccessMode::Restricted,
                parent_folder_id: None,
                path: SafeRelativePath::new("folder_path", "Private Project").unwrap(),
                current_key_version: 1,
                shared_folder_source: false,
            },
        ] {
            store
                .create_folder(
                    &brain_id,
                    &folder,
                    &BTreeSet::new(),
                    &[grant(
                        &format!("grant-{}-npub-admin", folder.id),
                        folder.id.as_str(),
                        1,
                        "npub-admin",
                        "npub-admin",
                    )],
                )
                .unwrap();
        }
    }

    fn bootstrap_org_named(store: &mut BrainStore, id: &str, name: &str, admin: &str) {
        let output = bootstrap_organization_brain(id, name, admin).unwrap();
        let grants = grants_for_required(&output.required_key_grants, admin);
        store.create_brain_bootstrap(&output, &grants).unwrap();
    }

    fn strategy_folder() -> Folder {
        Folder {
            id: FolderId::new("strategy").unwrap(),
            name: DisplayName::new("folder_name", "Strategy").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Restricted,
            parent_folder_id: Some(FolderId::new("team-notes").unwrap()),
            path: SafeRelativePath::new("folder_path", "Team Notes/Strategy").unwrap(),
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

    trait BrainStoreFolderGrantTestExt {
        fn grant_folder_access(
            &mut self,
            brain_id: &BrainId,
            folder_id: &FolderId,
            user_id: &UserId,
            grant: &FolderKeyGrantMetadata,
        ) -> Result<GrantFolderAccessOutcome, StoreError>;
    }

    impl BrainStoreFolderGrantTestExt for BrainStore {
        fn grant_folder_access(
            &mut self,
            brain_id: &BrainId,
            folder_id: &FolderId,
            user_id: &UserId,
            grant: &FolderKeyGrantMetadata,
        ) -> Result<GrantFolderAccessOutcome, StoreError> {
            let records = [
                folder_access_control_record(
                    &format!("{}-key-record", grant.id),
                    SyncRecordType::FolderKeyGrant,
                    folder_id.as_str(),
                    grant.issuer_npub.as_str(),
                ),
                folder_access_control_record(
                    &format!("{}-access-record", grant.id),
                    SyncRecordType::BrainAdminAccessChange,
                    folder_id.as_str(),
                    grant.issuer_npub.as_str(),
                ),
            ];
            self.grant_folder_access_with_control_records(
                brain_id, folder_id, user_id, grant, &records,
            )
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

    fn folder_access_control_record(
        event_id: &str,
        record_type: SyncRecordType,
        folder_id: &str,
        actor_npub: &str,
    ) -> SyncRecordInput {
        SyncRecordInput::Control(ControlSyncRecord {
            record_event_id: event_id.to_owned(),
            record_type,
            folder_id: Some(FolderId::new(folder_id).unwrap()),
            actor_npub: UserId::new(actor_npub).unwrap(),
            client_created_at: "2026-06-23T00:00:00.000Z".to_owned(),
            payload_json: "{\"control\":true}".to_owned(),
            record_event_kind: match record_type {
                SyncRecordType::FolderKeyGrant => NIP59_GIFT_WRAP_KIND,
                _ => APP_SPECIFIC_KIND,
            },
        })
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
