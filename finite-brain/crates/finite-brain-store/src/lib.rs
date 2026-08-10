//! FiniteBrain SQLite store and transaction boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use finite_brain_core::{
    BRAIN_CAPACITY_ENVELOPE, BootstrapOutput, Brain, BrainId, BrainKind, BrainMember, CoreError,
    DisplayName, EmailInviteScopeError, EmailInviteScopeFolder, Folder, FolderAccessMode, FolderId,
    FolderKeyRecipientPolicy, FolderRole, FolderRotationFanout, FolderRotationOperation, ObjectId,
    RequiredFolderKeyGrant, SafeRelativePath, UserId, derive_email_invite_scope,
    required_folder_key_recipients, validate_folder_rotation_fanout,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

mod brains;
mod cohorts;
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
    /// Complete additive Personal Brain Agent Set. `personal_agent` remains the
    /// legacy singular compatibility projection until cutover.
    pub personal_brain_agents: Vec<StoredPersonalBrainAgent>,
    /// Explicit Guest access by Folder id, independent of native access mode.
    pub folder_access: BTreeMap<FolderId, BTreeSet<UserId>>,
    /// Stored Folder Key Grant metadata.
    pub grants: Vec<FolderKeyGrantMetadata>,
    /// Folders that still need current grants.
    pub setup_incomplete_folder_ids: BTreeSet<FolderId>,
    /// Exact pre-deletion readers allowed to observe each subtree tombstone.
    pub folder_deletion_audience: BTreeMap<String, BTreeSet<UserId>>,
    /// Active agent-to-human operational authority anchored in this Brain.
    pub human_anchored_agent_authorities: BTreeMap<UserId, UserId>,
    /// Durable account-cohort provenance and participant state. This is the
    /// authoritative source for explaining why a principal has access.
    pub account_access_cohorts: Vec<StoredAccountAccessCohort>,
    /// Active scoped exclusions keyed by participant and Folder. The empty
    /// Folder id sentinel represents a whole-Brain exclusion.
    pub account_agent_exclusions: BTreeSet<(UserId, String)>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredAccountAccessCohort {
    pub cohort_id: String,
    pub account_id: String,
    pub human_npub: UserId,
    pub human_email: String,
    pub scope_kind: String,
    pub folder_id: Option<FolderId>,
    pub provenance_kind: String,
    pub status: String,
    pub participants: Vec<StoredAccountAccessParticipant>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredAccountAccessParticipant {
    pub npub: UserId,
    pub relationship: String,
    pub nip05: String,
    pub display_name: String,
    pub status: String,
    pub exclusion_reason: Option<String>,
    pub excluded_folder_ids: Vec<String>,
}

/// One account-owned agent in a Personal Brain's desired/readiness set.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredPersonalBrainAgent {
    pub agent_npub: UserId,
    pub agent_nip05: String,
    pub display_name: String,
    pub status: String,
    pub roster_revision: u64,
    pub blocker: Option<String>,
}

/// Result of applying one fixed mailbox cohort to a restricted Folder.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum GrantAccountCohortFolderAccessOutcome {
    Granted,
    AlreadyApplied,
}

/// Read-only exact plan for removing mailbox-derived restricted-Folder access.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AccountCohortFolderRemovalPlan {
    pub cohort_ids: Vec<String>,
    pub source_origins: Vec<(String, String)>,
    pub participants: Vec<StoredCohortParticipant>,
    pub removed_participant_npubs: BTreeSet<UserId>,
    pub independently_retained_npubs: BTreeSet<UserId>,
    pub required_recipient_npubs: BTreeSet<UserId>,
    pub current_key_version: u32,
    pub new_key_version: u32,
}

/// One Personal Brain agent still needing current Folder grants.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PersonalBrainAgentAdmissionPlan {
    pub cohort_id: String,
    pub human_npub: UserId,
    pub human_email: String,
    pub roster_revision: u64,
    pub agents: Vec<StoredCohortParticipant>,
    pub folder_key_versions: Vec<(FolderId, u32)>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ApplyPermanentAgentDepartureOutcome {
    Applied,
    AlreadyApplied,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PermanentAgentDeparturePlan {
    pub account_id: String,
    pub human_email: String,
    pub agent_nip05: String,
    pub agent_npub: UserId,
    pub folders: Vec<PermanentAgentDepartureFolderPlan>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PermanentAgentDepartureFolderPlan {
    pub folder_id: FolderId,
    pub current_key_version: u32,
    pub new_key_version: u32,
    pub required_recipient_npubs: BTreeSet<UserId>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AuthenticatedHumanIntentRecord {
    pub event_id: String,
    pub human_npub: UserId,
    pub acting_agent_npub: UserId,
    pub target_agent_npub: UserId,
    pub operation: String,
    pub scope_kind: String,
    pub folder_id: Option<FolderId>,
    pub event_json: String,
    pub consumed_at: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PersonalAgentBrainAccessPlan {
    pub human_npub: UserId,
    pub human_email: String,
    pub target_agent_npub: UserId,
    pub operation: String,
    pub folders: Vec<PersonalAgentBrainAccessFolderPlan>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PersonalAgentBrainAccessFolderPlan {
    pub folder_id: FolderId,
    pub current_key_version: u32,
    pub new_key_version: u32,
    pub required_recipient_npubs: BTreeSet<UserId>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCohortReconciliationPlan {
    pub operation_id: String,
    pub brain_id: BrainId,
    pub account_id: String,
    pub human_npub: UserId,
    pub human_email: String,
    pub scope_kind: String,
    pub folder_id: Option<FolderId>,
    pub roster_revision: u64,
    pub participants: Vec<StoredCohortParticipant>,
    pub folders: Vec<AccountCohortReconciliationFolderPlan>,
    pub pending_invitations: Vec<AccountCohortReconciliationPendingInvitation>,
    pub expected_member_additions: Vec<UserId>,
    pub independent_agent_npubs: Vec<UserId>,
    pub capacity: AccountCohortReconciliationCapacity,
    pub blocker: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCohortReconciliationPendingInvitation {
    pub invitation_id: String,
    pub target_kind: String,
    pub scope_kind: String,
    pub folder_id: Option<FolderId>,
    pub expires_at: String,
    pub conversion_required: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCohortReconciliationFolderPlan {
    pub folder_id: FolderId,
    pub key_version: u32,
    pub current_grant_recipient_npubs: Vec<UserId>,
    pub missing_grant_recipient_npubs: Vec<UserId>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCohortReconciliationCapacity {
    pub members_after: usize,
    pub member_limit: usize,
    pub folder_access_entries_after: usize,
    pub folder_access_entry_limit: usize,
    pub folder_key_grants_after: usize,
    pub folder_key_grant_limit: usize,
    pub sync_records_after: usize,
    pub sync_record_limit: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CommitAccountCohortReconciliationOutcome {
    Committed,
    AlreadyCommitted,
}

impl StoredBrain {
    /// Active Folder-limited identities that do not hold Brain Membership.
    pub fn guest_user_ids(&self) -> BTreeSet<UserId> {
        let members = self
            .brain
            .members
            .iter()
            .map(|member| member.user_id.clone())
            .collect::<BTreeSet<_>>();
        let owner = self.brain.owner_user_id.as_ref();
        let personal_agents = self
            .personal_brain_agents
            .iter()
            .filter(|agent| agent.status == "ready")
            .map(|agent| &agent.agent_npub)
            .chain(
                self.personal_agent
                    .as_ref()
                    .map(|relationship| &relationship.agent_npub),
            )
            .collect::<BTreeSet<_>>();
        self.folder_access
            .values()
            .flat_map(BTreeSet::iter)
            .filter(|user| {
                !members.contains(*user) && owner != Some(*user) && !personal_agents.contains(*user)
            })
            .cloned()
            .collect()
    }
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
    /// Folder-limited identity without Brain Membership.
    Guest,
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
    /// Visible explicit Folder access entries.
    pub folders: Vec<EncryptedExportFolderAccess>,
}

/// Explicit Folder access entry.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EncryptedExportFolderAccess {
    /// Folder id.
    pub folder_id: FolderId,
    /// Visible users.
    pub user_ids: Vec<UserId>,
}

/// One Folder Key rotation prepared for atomic Member removal.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MemberFolderRotation {
    pub folder_id: FolderId,
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantMetadata>,
    pub reencrypted_records: Vec<FolderObjectRevisionSyncRecord>,
}

/// One mounted source Folder rotation prepared for atomic Member removal.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MemberMountRotation {
    pub connection_id: String,
    pub revoke_mount: bool,
    pub new_key_version: u32,
    pub grants: Vec<FolderKeyGrantMetadata>,
    pub reencrypted_records: Vec<FolderObjectRevisionSyncRecord>,
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
    /// Fixed human-plus-account-agent participant set addressed by mailbox.
    AccountCohort,
}

impl BrainInvitationTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Npub => "npub",
            Self::EmailBootstrap => "email_bootstrap",
            Self::AccountCohort => "account_cohort",
        }
    }
}

impl TryFrom<&str> for BrainInvitationTargetKind {
    type Error = StoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "npub" => Ok(Self::Npub),
            "email_bootstrap" => Ok(Self::EmailBootstrap),
            "account_cohort" => Ok(Self::AccountCohort),
            _ => Err(StoreError::BrokenInvariant {
                reason: format!("unknown brain invitation target kind {value}"),
            }),
        }
    }
}

/// Backward-compatible store name for the core-owned Email Invite Bootstrap scope item.
pub type EmailInviteBootstrapScopeFolder = EmailInviteScopeFolder;

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
    /// True when claim creates bounded Guest access instead of Brain Membership.
    pub folder_only: bool,
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

/// One immutable participant approved in an account-cohort invitation plan.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredCohortParticipant {
    pub relationship: String,
    pub name: String,
    pub nip05: String,
    pub npub: UserId,
}

/// Durable fixed participant set and authoritative facts behind one invitation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredCohortInvitationPlan {
    pub invitation_id: String,
    pub plan_id: String,
    pub account_id: String,
    pub human_email: String,
    pub roster_revision: u64,
    pub scope_kind: String,
    pub folder_id: Option<FolderId>,
    pub participants: Vec<StoredCohortParticipant>,
    pub exclusions_json: String,
    pub key_versions_json: String,
    pub actor_npub: UserId,
    pub created_at: String,
}

/// Authoritative account snapshot committed atomically during Brain bootstrap.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BootstrapAccountCohort {
    pub account_id: String,
    pub human_email: String,
    pub roster_revision: u64,
    pub participants: Vec<StoredCohortParticipant>,
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
    /// Offer expiry timestamp.
    pub expires_at: String,
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
    /// Participants whose source Folder Access was created by this Mount.
    pub managed_access_npubs: BTreeSet<UserId>,
}

/// Stored universal Folder Mount.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredFolderMount {
    /// Stable deterministic mount id.
    pub id: String,
    /// Destination Brain id.
    pub destination_brain_id: BrainId,
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

/// Historical Mount access whose original ownership cannot be reconstructed safely.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LegacyFolderAccessSourceRepair {
    /// Mount connection whose historical ownership is ambiguous.
    pub connection_id: String,
    /// Native source Brain.
    pub brain_id: BrainId,
    /// Native source Folder.
    pub folder_id: FolderId,
    /// Participant whose access requires an explicit ownership decision.
    pub user_id: UserId,
    /// Human-readable repair reason.
    pub reason: String,
    /// Timestamp inherited from the historical participant row.
    pub created_at: String,
}

/// A historical Personal Mount that could not be migrated without guessing.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LegacyPersonalMountRepair {
    /// Historical mount id.
    pub legacy_mount_id: String,
    /// Resolved destination Brain when one was unambiguous.
    pub destination_brain_id: Option<BrainId>,
    /// Human-readable repair reason.
    pub reason: String,
    /// Last migration-attempt timestamp inherited from the legacy row.
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
    /// Mount id.
    pub mount_id: String,
    /// Destination Brain id.
    pub destination_brain_id: BrainId,
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
    control_records: &'a [SyncRecordInput],
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
            personal_brain_agents: self.load_personal_brain_agents(brain_id)?,
            folder_access,
            grants: self.load_grants(brain_id)?,
            setup_incomplete_folder_ids: self.load_setup_incomplete_folder_ids(brain_id)?,
            folder_deletion_audience: self.load_folder_deletion_audience(brain_id)?,
            human_anchored_agent_authorities: self
                .load_human_anchored_agent_authorities(brain_id)?,
            account_access_cohorts: self.load_account_access_cohorts(brain_id)?,
            account_agent_exclusions: self.load_account_agent_exclusions(brain_id)?,
        })
    }

    fn load_account_access_cohorts(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<StoredAccountAccessCohort>, StoreError> {
        let mut cohort_statement = self.conn.prepare(
            r#"
            SELECT id, account_id, human_npub, human_email, scope_kind,
                   folder_id, provenance_kind, status
            FROM account_access_cohorts
            WHERE brain_id = ?1
            ORDER BY created_at, id
            "#,
        )?;
        let cohort_rows = cohort_statement
            .query_map(params![brain_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut participant_statement = self.conn.prepare(
            r#"
            SELECT participant_npub, relationship, nip05, display_name,
                   status, exclusion_reason
            FROM account_access_cohort_participants
            WHERE cohort_id = ?1
            ORDER BY relationship DESC, participant_npub
            "#,
        )?;
        let mut exclusion_statement = self.conn.prepare(
            r#"
            SELECT COALESCE(folder_id, '')
            FROM account_access_cohort_exclusions
            WHERE cohort_id = ?1 AND participant_npub = ?2 AND active = 1
            ORDER BY COALESCE(folder_id, '')
            "#,
        )?;
        let mut cohorts = Vec::with_capacity(cohort_rows.len());
        for (
            cohort_id,
            account_id,
            human_npub,
            human_email,
            scope_kind,
            folder_id,
            provenance_kind,
            status,
        ) in cohort_rows
        {
            let rows = participant_statement.query_map(params![&cohort_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?;
            let mut participants = Vec::new();
            for row in rows {
                let (npub, relationship, nip05, display_name, participant_status, reason) = row?;
                let excluded_folder_ids = exclusion_statement
                    .query_map(params![&cohort_id, &npub], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                participants.push(StoredAccountAccessParticipant {
                    npub: UserId::new(npub)?,
                    relationship,
                    nip05,
                    display_name,
                    status: participant_status,
                    exclusion_reason: reason,
                    excluded_folder_ids,
                });
            }
            cohorts.push(StoredAccountAccessCohort {
                cohort_id,
                account_id,
                human_npub: UserId::new(human_npub)?,
                human_email,
                scope_kind,
                folder_id: folder_id.map(FolderId::new).transpose()?,
                provenance_kind,
                status,
                participants,
            });
        }
        Ok(cohorts)
    }

    fn load_account_agent_exclusions(
        &self,
        brain_id: &BrainId,
    ) -> Result<BTreeSet<(UserId, String)>, StoreError> {
        let mut statement = self.conn.prepare(
            r#"
            SELECT exclusion.participant_npub, COALESCE(exclusion.folder_id, '')
            FROM account_access_cohort_exclusions exclusion
            JOIN account_access_cohorts cohort ON cohort.id = exclusion.cohort_id
            JOIN account_access_cohort_participants participant
              ON participant.cohort_id = exclusion.cohort_id
             AND participant.participant_npub = exclusion.participant_npub
            WHERE cohort.brain_id = ?1
              AND cohort.status = 'active'
              AND participant.relationship = 'account_agent'
              AND exclusion.active = 1
            ORDER BY exclusion.participant_npub, COALESCE(exclusion.folder_id, '')
            "#,
        )?;
        let rows = statement.query_map(params![brain_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut exclusions = BTreeSet::new();
        for row in rows {
            let (npub, folder_id) = row?;
            exclusions.insert((UserId::new(npub)?, folder_id));
        }
        Ok(exclusions)
    }

    fn load_personal_brain_agents(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<StoredPersonalBrainAgent>, StoreError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT agent_npub, agent_nip05, display_name, status,
                   roster_revision, blocker
            FROM personal_brain_agents
            WHERE brain_id = ?1
            ORDER BY created_at, agent_npub
            "#,
        )?;
        stmt.query_map(params![brain_id.as_str()], |row| {
            let npub = row.get::<_, String>(0)?;
            let revision = row.get::<_, i64>(4)?;
            Ok(StoredPersonalBrainAgent {
                agent_npub: UserId::new(npub).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
                agent_nip05: row.get(1)?,
                display_name: row.get(2)?,
                status: row.get(3)?,
                roster_revision: u64::try_from(revision)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, revision))?,
                blocker: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
    }

    fn load_human_anchored_agent_authorities(
        &self,
        brain_id: &BrainId,
    ) -> Result<BTreeMap<UserId, UserId>, StoreError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT authority.agent_npub, authority.human_npub
            FROM human_anchored_agent_authorities authority
            JOIN account_access_cohorts cohort ON cohort.id = authority.cohort_id
            WHERE authority.brain_id = ?1
              AND authority.status = 'active'
              AND cohort.status = 'active'
            ORDER BY authority.agent_npub, authority.created_at
            "#,
        )?;
        let rows = stmt.query_map(params![brain_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut authorities = BTreeMap::new();
        for row in rows {
            let (agent, human) = row?;
            authorities.insert(UserId::new(agent)?, UserId::new(human)?);
        }
        Ok(authorities)
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
        let is_personal_agent = is_ready_personal_agent(&stored, actor_npub);
        let is_member = stored
            .brain
            .members
            .iter()
            .any(|member| member.user_id == *actor_npub);
        let has_personal_folder_scope = stored.brain.kind != BrainKind::Personal
            || stored.brain.owner_user_id.as_ref() == Some(actor_npub)
            || is_personal_agent
            || is_member
            || stored
                .folder_access
                .values()
                .any(|users| users.contains(actor_npub));
        let is_guest = stored.guest_user_ids().contains(actor_npub);
        if (!brain_visible_to_actor(&stored.brain, actor_npub) && !is_personal_agent && !is_guest)
            || !has_personal_folder_scope
        {
            return Err(StoreError::BrokenInvariant {
                reason: "brain access required for encrypted export".to_owned(),
            });
        }
        let is_admin = stored.brain.admins.contains(actor_npub);
        let is_limited_guest = is_guest;
        let folders = stored
            .brain
            .folders
            .iter()
            .filter_map(|folder| {
                let accessible = folder_visible_to_actor(&stored, &folder.id, actor_npub);
                (!is_limited_guest || accessible).then(|| EncryptedExportFolder {
                    id: folder.id.clone(),
                    path: folder.path.clone(),
                    access: folder.access,
                    current_key_version: folder.current_key_version,
                    accessible,
                })
            })
            .collect::<Vec<_>>();
        let objects = self
            .load_current_objects(brain_id)?
            .into_iter()
            .filter_map(|object| {
                let accessible = folder_visible_to_actor(&stored, &object.folder_id, actor_npub);
                (!is_limited_guest || accessible).then(|| EncryptedExportObject {
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
        validate_hierarchy(&self.conn, &brain.id, folder)?;
        let personal_agent = self
            .load_personal_agent(&brain.id)?
            .map(|relationship| relationship.agent_npub);
        let personal_brain_agents = self
            .load_personal_brain_agents(&brain.id)?
            .into_iter()
            .filter(|agent| agent.status == "ready")
            .map(|agent| agent.agent_npub)
            .collect::<BTreeSet<_>>();
        let mut required =
            required_recipients(brain, folder, access_user_ids, personal_agent.as_ref())?;
        required.extend(personal_brain_agents.iter().cloned());
        let delegated_issuer = grants
            .first()
            .map(|grant| &grant.issuer_npub)
            .filter(|issuer| personal_brain_agents.contains(*issuer));
        let anchored_authorities = self.load_human_anchored_agent_authorities(&brain.id)?;
        for (agent, human) in &anchored_authorities {
            if required.contains(human) {
                required.insert(agent.clone());
            }
        }
        let allow_anchored_issuer = grants.iter().all(|grant| {
            anchored_authorities
                .get(&grant.issuer_npub)
                .is_some_and(|human| {
                    brain.owner_user_id.as_ref() == Some(human) || brain.admins.contains(human)
                })
        });
        validate_folder_grants(
            brain,
            folder,
            &required,
            grants,
            delegated_issuer.or(personal_agent.as_ref()),
            allow_anchored_issuer,
        )
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
            return Err(StoreError::UnavailableLink { kind: "Mount" });
        }
        let destination = self.load_brain(&connection.destination_brain_id)?;
        if !has_brain_operational_authority(&destination, actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "mount participant management requires destination brain control"
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
        let destination = self.load_brain(destination_brain_id)?;
        let is_owner = destination
            .brain
            .owner_user_id
            .as_ref()
            .is_some_and(|owner| owner == target_npub);
        let is_agent = destination
            .personal_agent
            .as_ref()
            .is_some_and(|agent| &agent.agent_npub == target_npub);
        let is_admin = destination.brain.admins.contains(target_npub);
        let is_member = destination
            .brain
            .members
            .iter()
            .any(|member| member.user_id == *target_npub);
        if is_owner || is_agent || is_admin || is_member {
            Ok(())
        } else {
            Err(StoreError::BrokenInvariant {
                reason: "mount participant must be governed by the destination brain".to_owned(),
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
        let mut required = required_recipients(
            &stored.brain,
            &rotated_folder,
            &remaining_access,
            stored
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
        )?;
        extend_account_agent_recipients(&mut required, &stored, &folder.id);
        validate_connection_rotation_grants(
            &rotated_folder,
            &required,
            rotation.grants,
            actor_npub,
        )?;
        validate_folder_key_grant_control_records(rotation.grants, rotation.control_records)?;
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
        sync_records::append_sync_records(
            &tx,
            &connection.source_brain_id,
            rotation.control_records,
        )?;
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

    fn as_storage_str(&self) -> &'static str {
        match self {
            Self::BrainAdminAccessChange => "vault_admin_access_change",
            _ => self.as_str(),
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
            "brain_admin_access_change" | "vault_admin_access_change" => {
                Ok(Self::BrainAdminAccessChange)
            }
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
        folder_only: row.get::<_, i64>(20)? != 0,
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
        expires_at: row.get(15)?,
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
        managed_access_npubs: BTreeSet::new(),
    })
}

fn folder_mount_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredFolderMount> {
    let display_parent_folder_id = row.get::<_, Option<String>>(6)?;
    Ok(StoredFolderMount {
        id: row.get(0)?,
        destination_brain_id: BrainId::new(row.get::<_, String>(1)?)
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
        return Err(StoreError::UnavailableLink {
            kind: "Folder Invitation",
        });
    }
    Ok(())
}

fn timestamp_expired(expires_at: &str, now: &str) -> bool {
    if expires_at.is_empty() {
        return false;
    }
    match (
        OffsetDateTime::parse(expires_at, &Rfc3339),
        OffsetDateTime::parse(now, &Rfc3339),
    ) {
        (Ok(expires_at), Ok(now)) => expires_at <= now,
        // Persisted malformed lifecycle state must fail closed rather than
        // making an invitation available indefinitely.
        _ => true,
    }
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
        reason: format!("{field} must be an RFC3339 timestamp"),
    })?;
    Ok(())
}

fn validate_bounded_offer_expiry(expires_at: &str, created_at: &str) -> Result<(), StoreError> {
    validate_link_timestamp("expiresAt", expires_at)?;
    validate_link_timestamp("createdAt", created_at)?;
    let expires =
        OffsetDateTime::parse(expires_at, &Rfc3339).map_err(|_| StoreError::BrokenInvariant {
            reason: "expiresAt must be an RFC3339 timestamp".to_owned(),
        })?;
    let created =
        OffsetDateTime::parse(created_at, &Rfc3339).map_err(|_| StoreError::BrokenInvariant {
            reason: "createdAt must be an RFC3339 timestamp".to_owned(),
        })?;
    let duration = expires - created;
    if duration < time::Duration::hours(1) || duration > time::Duration::days(30) {
        return Err(StoreError::BrokenInvariant {
            reason: "invitation expiry must be between one hour and thirty days".to_owned(),
        });
    }
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

fn insert_folder_access_source(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    folder_id: &FolderId,
    user_id: &UserId,
    source_kind: &str,
    source_id: &str,
    created_at: &str,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO folder_access_sources (
            brain_id, folder_id, user_id, source_kind, source_id, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            brain_id.as_str(),
            folder_id.as_str(),
            user_id.as_str(),
            source_kind,
            source_id,
            created_at
        ],
    )?;
    Ok(())
}

fn delete_folder_access_source(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    folder_id: &FolderId,
    user_id: &UserId,
    source_kind: &str,
    source_id: &str,
) -> Result<(), StoreError> {
    tx.execute(
        "DELETE FROM folder_access_sources
         WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3
           AND source_kind = ?4 AND source_id = ?5",
        params![
            brain_id.as_str(),
            folder_id.as_str(),
            user_id.as_str(),
            source_kind,
            source_id
        ],
    )?;
    Ok(())
}

fn delete_folder_access_sources_for_origin(
    tx: &Transaction<'_>,
    source_kind: &str,
    source_id: &str,
) -> Result<(), StoreError> {
    tx.execute(
        "DELETE FROM folder_access_sources WHERE source_kind = ?1 AND source_id = ?2",
        params![source_kind, source_id],
    )?;
    Ok(())
}

fn folder_access_has_source(
    conn: &Connection,
    brain_id: &BrainId,
    folder_id: &FolderId,
    user_id: &UserId,
    source_kind: &str,
    source_id: &str,
) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM folder_access_sources
            WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3
              AND source_kind = ?4 AND source_id = ?5
         )",
        params![
            brain_id.as_str(),
            folder_id.as_str(),
            user_id.as_str(),
            source_kind,
            source_id
        ],
        |row| row.get(0),
    )?)
}

fn folder_access_has_mount_source(
    conn: &Connection,
    brain_id: &BrainId,
    folder_id: &FolderId,
    user_id: &UserId,
) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM folder_access_sources
            WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3
              AND source_kind = 'mount'
         )",
        params![brain_id.as_str(), folder_id.as_str(), user_id.as_str()],
        |row| row.get(0),
    )?)
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
        validate_grant_issuer(brain, grant, None, false)?;
    }
    Ok(())
}

fn validate_folder_grants(
    brain: &Brain,
    folder: &Folder,
    required_recipients: &BTreeSet<UserId>,
    grants: &[FolderKeyGrantMetadata],
    personal_agent: Option<&UserId>,
    allow_operational_authority: bool,
) -> Result<(), StoreError> {
    let mut provided = BTreeSet::new();
    for grant in grants {
        validate_grant_metadata(grant)?;
        validate_grant_issuer(brain, grant, personal_agent, allow_operational_authority)?;
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
    allow_operational_authority: bool,
) -> Result<(), StoreError> {
    match brain.kind {
        BrainKind::Personal => {
            if brain.owner_user_id.as_ref() != Some(&grant.issuer_npub)
                && personal_agent != Some(&grant.issuer_npub)
                && !allow_operational_authority
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "personal brain grants must be issued by the owner or Personal Agent"
                        .to_owned(),
                });
            }
        }
        BrainKind::Organization => {
            if !brain.admins.contains(&grant.issuer_npub) && !allow_operational_authority {
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

fn validate_folder_key_grant_control_records(
    grants: &[FolderKeyGrantMetadata],
    control_records: &[SyncRecordInput],
) -> Result<(), StoreError> {
    if control_records.len() != grants.len() {
        return Err(StoreError::BrokenInvariant {
            reason: "sharing mutation requires one Folder Key Grant control record per grant"
                .to_owned(),
        });
    }
    for (grant, input) in grants.iter().zip(control_records) {
        sync_records::validate_sync_input(input)?;
        let SyncRecordInput::Control(record) = input else {
            return Err(StoreError::BrokenInvariant {
                reason: "sharing mutation Folder Key Grant records must be control records"
                    .to_owned(),
            });
        };
        if record.record_type != SyncRecordType::FolderKeyGrant
            || record.folder_id.as_ref() != Some(&grant.folder_id)
            || record.actor_npub != grant.issuer_npub
        {
            return Err(StoreError::BrokenInvariant {
                reason: "sharing mutation control records do not match their Folder Key Grants"
                    .to_owned(),
            });
        }
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
    folder_only: bool,
) -> Result<Vec<EmailInviteBootstrapScopeFolder>, StoreError> {
    let folders = brain
        .folders
        .iter()
        .map(|folder| EmailInviteBootstrapScopeFolder {
            folder_id: folder.id.clone(),
            access: folder.access,
            key_version: folder.current_key_version,
        })
        .collect::<Vec<_>>();
    derive_email_invite_scope(&folders, selected_restricted_folder_access, folder_only).map_err(
        |error| match error {
            EmailInviteScopeError::MissingFolder { folder_id } => {
                StoreError::MissingFolder { folder_id }
            }
            other => StoreError::BrokenInvariant {
                reason: other.to_string(),
            },
        },
    )
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

fn extend_account_agent_recipients(
    required: &mut BTreeSet<UserId>,
    stored: &StoredBrain,
    folder_id: &FolderId,
) {
    let excluded = |agent: &UserId| {
        stored
            .account_agent_exclusions
            .contains(&(agent.clone(), String::new()))
            || stored
                .account_agent_exclusions
                .contains(&(agent.clone(), folder_id.to_string()))
    };
    if stored.brain.kind == BrainKind::Personal {
        required.extend(
            stored
                .personal_brain_agents
                .iter()
                .filter(|agent| agent.status == "ready" && !excluded(&agent.agent_npub))
                .map(|agent| agent.agent_npub.clone()),
        );
    }
    for (agent, human) in &stored.human_anchored_agent_authorities {
        if required.contains(human) && !excluded(agent) {
            required.insert(agent.clone());
        }
    }
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
    if stored
        .account_agent_exclusions
        .contains(&(actor_npub.clone(), String::new()))
    {
        return false;
    }
    match stored.brain.kind {
        BrainKind::Personal => {
            stored.brain.owner_user_id.as_ref() == Some(actor_npub)
                || is_ready_personal_agent(stored, actor_npub)
                || stored
                    .human_anchored_agent_authorities
                    .get(actor_npub)
                    .is_some_and(|human| stored.brain.owner_user_id.as_ref() == Some(human))
        }
        BrainKind::Organization => {
            stored.brain.admins.contains(actor_npub)
                || stored
                    .human_anchored_agent_authorities
                    .get(actor_npub)
                    .is_some_and(|human| stored.brain.admins.contains(human))
        }
    }
}

fn is_ready_personal_agent(stored: &StoredBrain, actor_npub: &UserId) -> bool {
    stored
        .personal_brain_agents
        .iter()
        .any(|agent| agent.status == "ready" && agent.agent_npub == *actor_npub)
        || stored
            .personal_agent
            .as_ref()
            .is_some_and(|relationship| relationship.agent_npub == *actor_npub)
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
    let is_personal_agent = is_ready_personal_agent(stored, actor_npub);
    let is_member = stored
        .brain
        .members
        .iter()
        .any(|member| member.user_id == *actor_npub);

    let cohort_agent_excluded = stored
        .account_agent_exclusions
        .contains(&(actor_npub.clone(), String::new()))
        || stored
            .account_agent_exclusions
            .contains(&(actor_npub.clone(), folder_id.to_string()));

    if let Some(human) = stored.human_anchored_agent_authorities.get(actor_npub) {
        if cohort_agent_excluded {
            return false;
        }
        return folder_visible_to_actor(stored, folder_id, human);
    }

    if is_personal_agent {
        return !cohort_agent_excluded;
    }
    if stored
        .folder_access
        .get(folder_id)
        .is_some_and(|users| users.contains(actor_npub))
    {
        return true;
    }

    match folder.access {
        FolderAccessMode::Owner => is_owner,
        FolderAccessMode::AdminOnly => is_owner || is_admin,
        FolderAccessMode::AllMembers => is_owner || is_admin || is_member,
        FolderAccessMode::Restricted => is_owner || is_admin,
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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11)
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
        insert_folder_access_source(
            tx,
            brain_id,
            folder_id,
            user_id,
            "direct",
            "initial-folder-access",
            &current_timestamp(),
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

fn consume_authenticated_human_intent(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    intent: &AuthenticatedHumanIntentRecord,
) -> Result<(), StoreError> {
    if !matches!(intent.operation.as_str(), "restrict" | "restore")
        || !matches!(intent.scope_kind.as_str(), "brain" | "folder")
        || (intent.scope_kind == "brain") != intent.folder_id.is_none()
        || intent.event_id.trim().is_empty()
        || intent.event_json.trim().is_empty()
        || intent.acting_agent_npub == intent.target_agent_npub
    {
        return Err(StoreError::BrokenInvariant {
            reason: "Authenticated Human Intent is incomplete or ambiguous".to_owned(),
        });
    }
    let replayed = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM authenticated_human_intents WHERE event_id = ?1)",
        params![intent.event_id],
        |row| row.get::<_, bool>(0),
    )?;
    if replayed {
        return Err(StoreError::BrokenInvariant {
            reason: "Authenticated Human Intent was already consumed".to_owned(),
        });
    }
    tx.execute(
        r#"
        INSERT INTO authenticated_human_intents (
            event_id, brain_id, human_npub, acting_agent_npub,
            target_agent_npub, operation, scope_kind, folder_id,
            event_json, consumed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![
            intent.event_id,
            brain_id.as_str(),
            intent.human_npub.as_str(),
            intent.acting_agent_npub.as_str(),
            intent.target_agent_npub.as_str(),
            intent.operation,
            intent.scope_kind,
            intent.folder_id.as_ref().map(FolderId::as_str),
            intent.event_json,
            intent.consumed_at,
        ],
    )?;
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
        FolderRole::BrainOps => "vault_ops",
        FolderRole::General => "general",
        FolderRole::Folder => "folder",
    }
}

fn parse_folder_role(value: &str) -> Result<FolderRole, StoreError> {
    match value {
        "personal_home" => Ok(FolderRole::PersonalHome),
        "brain_ops" | "vault_ops" => Ok(FolderRole::BrainOps),
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
    fn human_anchored_agent_authority_follows_the_humans_current_admin_role() {
        let store = BrainStore::open_in_memory().unwrap();
        store
            .conn
            .execute(
                "INSERT INTO brains (id, kind, name, created_at) VALUES ('org', 'organization', 'Org', ?1)",
                params!["2026-08-10T00:00:00Z"],
            )
            .unwrap();
        for npub in ["npub-human", "npub-agent", "npub-second-admin"] {
            store
                .conn
                .execute(
                    "INSERT INTO brain_members (brain_id, user_id) VALUES ('org', ?1)",
                    params![npub],
                )
                .unwrap();
        }
        store
            .conn
            .execute(
                "INSERT INTO brain_admins (brain_id, user_id) VALUES ('org', 'npub-human')",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO brain_admins (brain_id, user_id) VALUES ('org', 'npub-second-admin')",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                r#"
                INSERT INTO account_access_cohorts (
                    id, brain_id, human_npub, human_email, scope_kind,
                    provenance_kind, provenance_id, roster_revision, status,
                    created_at, updated_at
                ) VALUES (
                    'cohort', 'org', 'npub-human', 'human@finite.vip', 'brain',
                    'bootstrap', 'bootstrap-org', 1, 'active', ?1, ?1
                )
                "#,
                params!["2026-08-10T00:00:00Z"],
            )
            .unwrap();
        store
            .conn
            .execute(
                r#"
                INSERT INTO human_anchored_agent_authorities (
                    cohort_id, brain_id, human_npub, agent_npub, status,
                    created_at, updated_at
                ) VALUES (
                    'cohort', 'org', 'npub-human', 'npub-agent', 'active', ?1, ?1
                )
                "#,
                params!["2026-08-10T00:00:00Z"],
            )
            .unwrap();

        let brain_id = BrainId::new("org").unwrap();
        let agent = UserId::new("npub-agent").unwrap();
        let stored = store.load_brain(&brain_id).unwrap();
        assert!(has_brain_operational_authority(&stored, &agent));
        assert!(!stored.brain.admins.contains(&agent));

        store
            .conn
            .execute(
                "DELETE FROM brain_admins WHERE brain_id = 'org' AND user_id = 'npub-human'",
                [],
            )
            .unwrap();
        let demoted = store.load_brain(&brain_id).unwrap();
        assert!(!has_brain_operational_authority(&demoted, &agent));
    }

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
    fn organization_collaboration_converges_membership_grants_and_folder_audit_records() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let target = UserId::new("npub-collaborator").unwrap();
        let grants = [
            grant(
                "grant-team-notes-collaborator",
                "team-notes",
                1,
                "npub-admin",
                target.as_str(),
            ),
            grant(
                "grant-private-project-collaborator",
                "private-project",
                1,
                "npub-admin",
                target.as_str(),
            ),
        ];
        let accepted = grants
            .iter()
            .map(|grant| {
                (
                    grant.clone(),
                    folder_access_control_record(
                        &format!("{}-grant", grant.id),
                        SyncRecordType::FolderKeyGrant,
                        grant.folder_id.as_str(),
                        "npub-admin",
                    ),
                    folder_access_control_record(
                        &format!("{}-access", grant.id),
                        SyncRecordType::BrainAdminAccessChange,
                        grant.folder_id.as_str(),
                        "npub-admin",
                    ),
                )
            })
            .collect::<Vec<_>>();
        let admin_record = SyncRecordInput::Control(ControlSyncRecord {
            record_event_id: "collaboration-admin".to_owned(),
            record_type: SyncRecordType::BrainAdminAccessChange,
            folder_id: None,
            actor_npub: UserId::new("npub-admin").unwrap(),
            client_created_at: "2026-06-23T00:00:00.000Z".to_owned(),
            payload_json: "{\"control\":true}".to_owned(),
            record_event_kind: APP_SPECIFIC_KIND,
        });

        store
            .ensure_organization_admin_with_grants(
                &brain_id,
                &target,
                &accepted,
                Some(&admin_record),
            )
            .unwrap();
        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            stored
                .brain
                .members
                .iter()
                .any(|member| member.user_id == target)
        );
        assert!(stored.brain.admins.contains(&target));
        assert_eq!(
            stored
                .grants
                .iter()
                .filter(|grant| grant.recipient_npub == target)
                .count(),
            2
        );
        assert_eq!(
            stored
                .folder_access
                .get(&FolderId::new("private-project").unwrap()),
            Some(&BTreeSet::from([target.clone()]))
        );

        let before_retry = store.sync_bootstrap(&brain_id).unwrap().latest_sequence;
        store
            .ensure_organization_admin_with_grants(
                &brain_id,
                &target,
                &accepted,
                Some(&admin_record),
            )
            .unwrap();
        assert_eq!(
            store.sync_bootstrap(&brain_id).unwrap().latest_sequence,
            before_retry,
            "retry must not duplicate grants or collaboration control records"
        );
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
                "/v1/brain-invitation-links/invite-initial-strategy0123456789ab/accept",
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
    fn explicit_guest_grant_is_orthogonal_to_admin_only_native_access() {
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
                .unwrap(),
            GrantFolderAccessOutcome::Granted
        );
        let stored = store.load_brain(&brain_id).unwrap();
        assert_eq!(
            stored
                .folder_access
                .get(&FolderId::new("admin-only").unwrap()),
            Some(&BTreeSet::from([member.clone()]))
        );
        assert!(folder_visible_to_actor(
            &stored,
            &FolderId::new("admin-only").unwrap(),
            &member
        ));
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
                "/v1/brain-invitation-links/invite-0123456789abcdef0123456789abcdef/accept",
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

        assert_eq!(
            store
                .revoke_brain_invitation(&brain_id, "invitation-target", &admin, now)
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "brain invitation"
            }
        );
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
                "/v1/brain-invitation-links/invite-email0123456789abcdef012345/accept",
                std::slice::from_ref(&restricted),
                false,
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
    fn failed_email_invitation_replacement_preserves_the_pending_invitation() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let restricted = FolderId::new("private-project").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let unwrap_npub = UserId::new("npub-unwrap").unwrap();
        let now = "2026-06-23T00:00:00.000Z";
        let original = store
            .create_email_brain_invitation(
                &brain_id,
                "pending-email-invitation",
                "guest@example.com",
                &unwrap_npub,
                "sha256-original",
                "{\"kind\":1059,\"original\":true}",
                "{\"kind\":30078}",
                "invite-original0123456789abcdef0123",
                "/v1/brain-invitation-links/invite-original0123456789abcdef0123/claim",
                std::slice::from_ref(&restricted),
                true,
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        store
            .create_brain_invitation(
                &brain_id,
                "already-used-id",
                &UserId::new("npub-target").unwrap(),
                "invite-other0123456789abcdef012345",
                "/v1/brain-invitation-links/invite-other0123456789abcdef012345/accept",
                &[],
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();

        assert!(
            store
                .create_email_brain_invitation(
                    &brain_id,
                    "already-used-id",
                    "guest@example.com",
                    &unwrap_npub,
                    "sha256-replacement",
                    "{\"kind\":1059,\"replacement\":true}",
                    "{\"kind\":30078}",
                    "invite-replacement0123456789abcdef",
                    "/v1/brain-invitation-links/invite-replacement0123456789abcdef/claim",
                    std::slice::from_ref(&restricted),
                    true,
                    &admin,
                    "2026-06-30T00:00:00.000Z",
                    now,
                )
                .is_err()
        );

        let preserved = store.load_brain_invitation(&original.id).unwrap();
        assert_eq!(preserved.status, LinkStatus::Pending);
        assert_eq!(
            preserved.bootstrap_wrapped_event_json.as_deref(),
            Some("{\"kind\":1059,\"original\":true}")
        );
    }

    #[test]
    fn failed_email_folder_claim_sync_append_rolls_back_access_and_acceptance() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let restricted = FolderId::new("private-project").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let unwrap_npub = UserId::new("npub-unwrap").unwrap();
        let claimant = UserId::new("npub-folder-guest").unwrap();
        let now = "2026-06-23T00:00:00.000Z";
        let invitation = store
            .create_email_brain_invitation(
                &brain_id,
                "atomic-folder-claim",
                "atomic@example.com",
                &unwrap_npub,
                "sha256-atomic",
                "{\"kind\":1059}",
                "{\"kind\":30078}",
                "invite-atomic0123456789abcdef012345",
                "/v1/brain-invitation-links/invite-atomic0123456789abcdef012345/claim",
                std::slice::from_ref(&restricted),
                true,
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        let grant = grant(
            "atomic-claim-grant",
            restricted.as_str(),
            1,
            claimant.as_str(),
            claimant.as_str(),
        );
        let duplicate_record = folder_key_grant_control_record(&grant, "duplicate-event");
        store
            .submit_sync_record(&brain_id, &duplicate_record)
            .unwrap();

        assert!(
            store
                .claim_email_brain_invitation_by_code_with_control_records(
                    "invite-atomic0123456789abcdef012345",
                    "atomic@example.com",
                    &claimant,
                    std::slice::from_ref(&grant),
                    std::slice::from_ref(&duplicate_record),
                    now,
                )
                .is_err()
        );

        let preserved = store.load_brain_invitation(&invitation.id).unwrap();
        assert_eq!(preserved.status, LinkStatus::Pending);
        assert!(preserved.bootstrap_wrapped_event_json.is_some());
        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            stored
                .folder_access
                .get(&restricted)
                .is_none_or(|users| !users.contains(&claimant))
        );
        assert!(!stored.grants.contains(&grant));
    }

    #[test]
    fn email_folder_invitation_claims_guest_access_to_exactly_one_folder() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let restricted = FolderId::new("private-project").unwrap();
        let team_notes = FolderId::new("team-notes").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let unwrap_npub = UserId::new("npub-unwrap").unwrap();
        let claimant = UserId::new("npub-folder-guest").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        let invitation = store
            .create_email_brain_invitation(
                &brain_id,
                "invitation-email-folder",
                "guest@example.com",
                &unwrap_npub,
                "sha256-folder-bootstrap",
                "{\"kind\":1059}",
                "{\"kind\":30078}",
                "invite-email-folder01234567890123",
                "/v1/brain-invitation-links/invite-email-folder01234567890123/claim",
                std::slice::from_ref(&restricted),
                true,
                &admin,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        assert!(invitation.folder_only);
        assert_eq!(invitation.initial_folder_access, vec![restricted.clone()]);
        assert_eq!(invitation.bootstrap_scope.len(), 1);

        let grant = grant(
            "claim-grant-folder-guest",
            restricted.as_str(),
            1,
            claimant.as_str(),
            claimant.as_str(),
        );
        store
            .claim_email_brain_invitation_by_code(
                "invite-email-folder01234567890123",
                "guest@example.com",
                &claimant,
                std::slice::from_ref(&grant),
                now,
            )
            .unwrap();

        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            stored
                .brain
                .members
                .iter()
                .all(|member| member.user_id != claimant)
        );
        assert_eq!(
            stored.folder_access.get(&restricted),
            Some(&BTreeSet::from([claimant.clone()]))
        );
        assert!(
            stored
                .folder_access
                .get(&team_notes)
                .is_none_or(|users| !users.contains(&claimant))
        );
        assert!(stored.grants.contains(&grant));
    }

    #[test]
    fn email_folder_invitation_preserves_an_existing_member_relationship() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let restricted = FolderId::new("private-project").unwrap();
        let unrelated = FolderId::new("unrelated-private").unwrap();
        let claimant = UserId::new("npub-existing-member").unwrap();
        let now = "2026-06-23T00:00:00.000Z";
        store.add_member(&brain_id, &claimant).unwrap();

        store
            .create_email_brain_invitation(
                &brain_id,
                "invitation-email-existing-member",
                "member@example.com",
                &UserId::new("npub-unwrap").unwrap(),
                "sha256-existing-member-bootstrap",
                "{\"kind\":1059}",
                "{\"kind\":30078}",
                "invite-email-existing-member012345",
                "/v1/brain-invitation-links/invite-email-existing-member012345/claim",
                std::slice::from_ref(&restricted),
                true,
                &UserId::new("npub-admin").unwrap(),
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        let grant = grant(
            "claim-grant-existing-member",
            restricted.as_str(),
            1,
            claimant.as_str(),
            claimant.as_str(),
        );
        store
            .claim_email_brain_invitation_by_code(
                "invite-email-existing-member012345",
                "member@example.com",
                &claimant,
                std::slice::from_ref(&grant),
                now,
            )
            .unwrap();

        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            stored
                .brain
                .members
                .iter()
                .any(|member| member.user_id == claimant)
        );
        assert!(!stored.guest_user_ids().contains(&claimant));
        assert_eq!(
            stored.folder_access.get(&restricted),
            Some(&BTreeSet::from([claimant.clone()]))
        );
        assert!(
            stored
                .folder_access
                .get(&unrelated)
                .is_none_or(|users| !users.contains(&claimant))
        );
        assert!(stored.grants.contains(&grant));
    }

    #[test]
    fn email_folder_invitation_preserves_native_access_mode_for_guest_access() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let folder_id = FolderId::new("team-notes").unwrap();
        let claimant = UserId::new("npub-all-members-guest").unwrap();
        let invitation = store
            .create_email_brain_invitation(
                &brain_id,
                "invitation-email-folder-all-members",
                "guest@example.com",
                &UserId::new("npub-unwrap").unwrap(),
                "sha256-folder-bootstrap",
                "{\"kind\":1059}",
                "{\"kind\":30078}",
                "invite-email-folder-all-members01",
                "/v1/brain-invitation-links/invite-email-folder-all-members01/claim",
                std::slice::from_ref(&folder_id),
                true,
                &UserId::new("npub-admin").unwrap(),
                "2026-06-30T00:00:00.000Z",
                "2026-06-23T00:00:00.000Z",
            )
            .unwrap();
        assert_eq!(invitation.bootstrap_scope.len(), 1);
        assert_eq!(
            invitation.bootstrap_scope[0].access,
            FolderAccessMode::AllMembers
        );

        let claim_grant = grant(
            "claim-grant-all-members-guest",
            folder_id.as_str(),
            1,
            claimant.as_str(),
            claimant.as_str(),
        );
        store
            .claim_email_brain_invitation_by_code(
                "invite-email-folder-all-members01",
                "guest@example.com",
                &claimant,
                std::slice::from_ref(&claim_grant),
                "2026-06-23T00:00:00.000Z",
            )
            .unwrap();

        let stored = store.load_brain(&brain_id).unwrap();
        assert_eq!(stored.guest_user_ids(), BTreeSet::from([claimant.clone()]));
        assert_eq!(
            stored.folder_access.get(&folder_id),
            Some(&BTreeSet::from([claimant.clone()]))
        );
        assert!(
            folder_visible_to_actor(&stored, &folder_id, &claimant),
            "an explicit Guest grant must be orthogonal to the Folder's native access mode"
        );
    }

    #[test]
    fn email_folder_invitation_grants_a_personal_owner_folder_without_changing_its_mode() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let brain_id = BrainId::new("personal-invite").unwrap();
        let owner = UserId::new("npub-personal-owner").unwrap();
        let agent = UserId::new("npub-personal-agent").unwrap();
        let claimant = UserId::new("npub-personal-guest").unwrap();
        bootstrap_personal_named(
            &mut store,
            brain_id.as_str(),
            owner.as_str(),
            agent.as_str(),
            "2026-06-23T00:00:00.000Z",
        );
        let folder = Folder {
            id: FolderId::new("private-notes").unwrap(),
            name: DisplayName::new("folder_name", "Private Notes").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Owner,
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Private Notes").unwrap(),
            current_key_version: 1,
        };
        store
            .create_folder(
                &brain_id,
                &folder,
                &BTreeSet::new(),
                &[
                    grant(
                        "grant-private-notes-owner",
                        folder.id.as_str(),
                        1,
                        owner.as_str(),
                        owner.as_str(),
                    ),
                    grant(
                        "grant-private-notes-agent",
                        folder.id.as_str(),
                        1,
                        owner.as_str(),
                        agent.as_str(),
                    ),
                ],
            )
            .unwrap();

        store
            .create_email_brain_invitation(
                &brain_id,
                "invitation-personal-owner-folder",
                "personal-guest@example.com",
                &UserId::new("npub-personal-unwrap").unwrap(),
                "sha256-personal-folder-bootstrap",
                "{\"kind\":1059}",
                "{\"kind\":30078}",
                "invite-personal-owner-folder0001",
                "/v1/brain-invitation-links/invite-personal-owner-folder0001/claim",
                std::slice::from_ref(&folder.id),
                true,
                &owner,
                "2026-06-30T00:00:00.000Z",
                "2026-06-23T00:00:00.000Z",
            )
            .unwrap();
        let claim_grant = grant(
            "claim-grant-personal-owner-folder",
            folder.id.as_str(),
            1,
            claimant.as_str(),
            claimant.as_str(),
        );
        store
            .claim_email_brain_invitation_by_code(
                "invite-personal-owner-folder0001",
                "personal-guest@example.com",
                &claimant,
                std::slice::from_ref(&claim_grant),
                "2026-06-23T00:00:00.000Z",
            )
            .unwrap();

        let stored = store.load_brain(&brain_id).unwrap();
        let unchanged = stored
            .brain
            .folders
            .iter()
            .find(|candidate| candidate.id == folder.id)
            .unwrap();
        assert_eq!(unchanged.access, FolderAccessMode::Owner);
        assert_eq!(stored.guest_user_ids(), BTreeSet::from([claimant.clone()]));
        assert!(folder_visible_to_actor(&stored, &folder.id, &claimant));
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
                        &format!("/v1/brain-invitation-links/{code}/claim"),
                        std::slice::from_ref(&restricted),
                        false,
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
            "2026-06-24T00:00:00.000Z",
        );
        store
            .conn
            .execute(
                "UPDATE brain_invitations SET expires_at = ?2 WHERE id = ?1",
                params![expired.id.as_str(), "2026-06-22T00:00:00.000Z"],
            )
            .unwrap();
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
                "/v1/brain-invitation-links/invite-email-rotation0123456789/claim",
                std::slice::from_ref(&restricted),
                false,
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
                    "/v1/brain-invitation-links/invite-existing-member0123456789abcdef/accept",
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
                "/v1/brain-invitation-links/invite-stale-member0123456789abcdef/accept",
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
    fn folder_invitation_accept_creates_guest_access_and_grant_once() {
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
                "/v1/share-links/share-link-recipient/accept",
                &grant,
                now,
            )
            .unwrap();
        assert_eq!(share_link.status, LinkStatus::Pending);
        assert_eq!(share_link.folder_key_grant, grant);

        assert_eq!(
            store
                .load_available_share_link("share-link-recipient", &wrong_user, now)
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "Folder Invitation"
            }
        );

        let control_record =
            folder_key_grant_control_record(&grant, "share-link-recipient-grant-record");
        let accepted = store
            .accept_share_link(
                "share-link-recipient",
                &recipient,
                std::slice::from_ref(&control_record),
                now,
            )
            .unwrap();
        assert_eq!(accepted.status, LinkStatus::Accepted);
        assert_eq!(accepted.accepted_at.as_deref(), Some(now));
        assert!(!accepted.duplicate_accept);

        let stored = store.load_brain(&brain_id).unwrap();
        assert!(
            !stored
                .brain
                .members
                .iter()
                .any(|member| member.user_id == recipient)
        );
        assert_eq!(stored.guest_user_ids(), BTreeSet::from([recipient.clone()]));
        assert_eq!(
            stored.folder_access.get(&folder_id),
            Some(&BTreeSet::from([recipient.clone()]))
        );
        assert!(stored.grants.iter().any(|stored_grant| {
            stored_grant.id == "grant-strategy-recipient"
                && stored_grant.recipient_npub == recipient
        }));

        let retry = store
            .accept_share_link(
                "share-link-recipient",
                &recipient,
                std::slice::from_ref(&control_record),
                now,
            )
            .unwrap();
        assert!(retry.duplicate_accept);

        assert_eq!(
            store
                .revoke_share_link("share-link-recipient", &admin, now)
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "Folder Invitation"
            }
        );
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
                "/v1/share-links/share-link-personal-agent/accept",
                &recipient_grant,
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
                    "/v1/brain-invitation-links/invite-bad-time/accept",
                    &[],
                    &admin,
                    "not-a-timestamp",
                    "2026-06-23T00:00:00.000Z",
                )
                .unwrap_err(),
            StoreError::BrokenInvariant {
                reason: "expiresAt must be an RFC3339 timestamp".to_owned()
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
                "/v1/brain-invitation-links/invite-revoked0123456789abcdef012345/accept",
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
                "/v1/brain-invitation-links/invite-expired0123456789abcdef012345/accept",
                &[],
                &admin,
                "2026-06-24T00:00:00.000Z",
                now,
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE brain_invitations SET expires_at = ?2 WHERE id = ?1",
                params!["invitation-expired", "2026-06-22T00:00:00.000Z"],
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
                "/v1/share-links/share-link-revoked/accept",
                &grant(
                    "grant-share-revoked",
                    "strategy",
                    1,
                    "npub-admin",
                    share_recipient.as_str(),
                ),
                now,
            )
            .unwrap();
        store
            .revoke_share_link("share-link-revoked", &admin, now)
            .unwrap();
        assert_eq!(
            store
                .accept_share_link("share-link-revoked", &share_recipient, &[], now)
                .unwrap_err(),
            StoreError::UnavailableLink {
                kind: "Folder Invitation"
            }
        );
    }

    #[test]
    fn invitation_expiry_compares_rfc3339_instants_not_text() {
        assert!(timestamp_expired(
            "2026-08-01T00:00:00+05:00",
            "2026-07-31T20:00:00Z"
        ));
        assert!(timestamp_expired(
            "2026-07-31T19:00:00.000Z",
            "2026-07-31T19:00:00Z"
        ));
        assert!(!timestamp_expired(
            "2026-07-31T19:00:00.001Z",
            "2026-07-31T19:00:00Z"
        ));
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

        let invitation = store
            .create_shared_folder_invitation(
                &source_brain_id,
                &source_folder_id,
                &destination_brain_id,
                "shared-folder-invitation-dest",
                &destination_admin,
                &source_admin,
                "/v1/shared-folder-invitations/shared-folder-invitation-dest/accept",
                &grant(
                    "grant-strategy-dest-admin-v1",
                    "strategy",
                    1,
                    "npub-admin",
                    destination_admin.as_str(),
                ),
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        assert_eq!(invitation.status, LinkStatus::Pending);

        let accepted = accept_mount_for_test(
            &mut store,
            "shared-folder-invitation-dest",
            &destination_admin,
            "shared-folder-connection-acme-dest",
            "organization-mount-dest-strategy",
            &[],
            now,
        )
        .unwrap();
        assert_eq!(accepted.status, LinkStatus::Accepted);
        assert!(!accepted.duplicate_accept);
        let retry = accept_mount_for_test(
            &mut store,
            "shared-folder-invitation-dest",
            &destination_admin,
            "shared-folder-connection-acme-dest",
            "organization-mount-dest-strategy",
            &[],
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
        let connection = add_mount_member_for_test(
            &mut store,
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

        let connection = remove_mount_member_for_test(
            &mut store,
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

        let connection = revoke_mount_for_test(
            &mut store,
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
    fn overlapping_mounts_preserve_access_until_the_last_entitlement_is_revoked() {
        let mut store = store_with_strategy_folder();
        bootstrap_org_named(&mut store, "dest-a", "Destination A", "npub-shared-admin");
        bootstrap_org_named(&mut store, "dest-b", "Destination B", "npub-shared-admin");
        let source_brain_id = BrainId::new("acme").unwrap();
        let source_folder_id = FolderId::new("strategy").unwrap();
        let source_admin = UserId::new("npub-admin").unwrap();
        let shared_admin = UserId::new("npub-shared-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        for (suffix, destination) in [("a", "dest-a"), ("b", "dest-b")] {
            store
                .create_shared_folder_invitation(
                    &source_brain_id,
                    &source_folder_id,
                    &BrainId::new(destination).unwrap(),
                    &format!("mount-offer-{suffix}"),
                    &shared_admin,
                    &source_admin,
                    &format!("/v1/mount-offers/mount-offer-{suffix}/accept"),
                    &grant(
                        &format!("grant-mount-{suffix}-v1"),
                        "strategy",
                        1,
                        source_admin.as_str(),
                        shared_admin.as_str(),
                    ),
                    "2026-06-30T00:00:00.000Z",
                    now,
                )
                .unwrap();
            accept_mount_for_test(
                &mut store,
                &format!("mount-offer-{suffix}"),
                &shared_admin,
                &format!("mount-{suffix}"),
                &format!("projection-{suffix}"),
                &[],
                now,
            )
            .unwrap();
        }

        let mount_a = store.load_shared_folder_connection("mount-a").unwrap();
        assert!(
            mount_a.managed_access_npubs.is_empty(),
            "Mount A must not claim exclusive ownership while Mount B still authorizes the identity"
        );
        revoke_mount_for_test(&mut store, "mount-a", &source_admin, 0, &[], &[], now).unwrap();
        let source = store.load_brain(&source_brain_id).unwrap();
        assert!(
            source
                .folder_access
                .get(&source_folder_id)
                .is_some_and(|users| users.contains(&shared_admin))
        );
        assert_eq!(
            source
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == source_folder_id)
                .unwrap()
                .current_key_version,
            1
        );
        assert_eq!(
            store
                .load_shared_folder_connection("mount-b")
                .unwrap()
                .status,
            SharedFolderConnectionStatus::Active
        );

        revoke_mount_for_test(
            &mut store,
            "mount-b",
            &source_admin,
            2,
            &[grant(
                "grant-strategy-source-admin-after-last-mount",
                "strategy",
                2,
                source_admin.as_str(),
                source_admin.as_str(),
            )],
            &[],
            now,
        )
        .unwrap();
        let source = store.load_brain(&source_brain_id).unwrap();
        assert!(
            !source
                .folder_access
                .get(&source_folder_id)
                .is_some_and(|users| users.contains(&shared_admin)),
            "the last Mount entitlement must remove access through key rotation"
        );
        assert_eq!(
            source
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == source_folder_id)
                .unwrap()
                .current_key_version,
            2
        );
    }

    #[test]
    fn migration_backfills_mount_provenance_for_pre_v19_connections() {
        let mut store = store_with_strategy_folder();
        bootstrap_org_named(&mut store, "dest", "Destination", "npub-dest-admin");
        let source_brain_id = BrainId::new("acme").unwrap();
        let source_folder_id = FolderId::new("strategy").unwrap();
        let destination_brain_id = BrainId::new("dest").unwrap();
        let source_admin = UserId::new("npub-admin").unwrap();
        let destination_admin = UserId::new("npub-dest-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        store
            .create_shared_folder_invitation(
                &source_brain_id,
                &source_folder_id,
                &destination_brain_id,
                "mount-offer-pre-v19",
                &destination_admin,
                &source_admin,
                "/v1/mount-offers/mount-offer-pre-v19/accept",
                &grant(
                    "grant-pre-v19-v1",
                    "strategy",
                    1,
                    source_admin.as_str(),
                    destination_admin.as_str(),
                ),
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        accept_mount_for_test(
            &mut store,
            "mount-offer-pre-v19",
            &destination_admin,
            "mount-pre-v19",
            "projection-pre-v19",
            &[],
            now,
        )
        .unwrap();

        store
            .conn
            .execute_batch(
                "DROP TABLE legacy_folder_access_source_repairs;
                 DROP TABLE folder_access_sources;
                 DELETE FROM schema_migrations WHERE version = 19;
                 UPDATE shared_folder_connection_members SET manages_folder_access = 1;",
            )
            .unwrap();
        store.apply_migrations().unwrap();

        let migrated = store
            .load_shared_folder_connection("mount-pre-v19")
            .unwrap();
        assert_eq!(
            migrated.managed_access_npubs,
            BTreeSet::from([destination_admin.clone()])
        );
        assert!(
            store
                .load_legacy_folder_access_source_repairs()
                .unwrap()
                .is_empty()
        );

        revoke_mount_for_test(
            &mut store,
            "mount-pre-v19",
            &source_admin,
            2,
            &[grant(
                "grant-pre-v19-source-v2",
                "strategy",
                2,
                source_admin.as_str(),
                source_admin.as_str(),
            )],
            &[],
            now,
        )
        .unwrap();
        let source = store.load_brain(&source_brain_id).unwrap();
        assert!(
            !source
                .folder_access
                .get(&source_folder_id)
                .is_some_and(|users| users.contains(&destination_admin))
        );
    }

    #[test]
    fn migration_preserves_and_surfaces_ambiguous_pre_v19_mount_access() {
        let mut store = store_with_strategy_folder();
        bootstrap_org_named(&mut store, "dest", "Destination", "npub-dest-admin");
        let source_brain_id = BrainId::new("acme").unwrap();
        let source_folder_id = FolderId::new("strategy").unwrap();
        let destination_brain_id = BrainId::new("dest").unwrap();
        let source_admin = UserId::new("npub-admin").unwrap();
        let destination_admin = UserId::new("npub-dest-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";

        store
            .create_shared_folder_invitation(
                &source_brain_id,
                &source_folder_id,
                &destination_brain_id,
                "mount-offer-ambiguous-pre-v19",
                &destination_admin,
                &source_admin,
                "/v1/mount-offers/mount-offer-ambiguous-pre-v19/accept",
                &grant(
                    "grant-ambiguous-pre-v19-v1",
                    "strategy",
                    1,
                    source_admin.as_str(),
                    destination_admin.as_str(),
                ),
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        accept_mount_for_test(
            &mut store,
            "mount-offer-ambiguous-pre-v19",
            &destination_admin,
            "mount-ambiguous-pre-v19",
            "projection-ambiguous-pre-v19",
            &[],
            now,
        )
        .unwrap();

        store
            .conn
            .execute_batch(
                "DROP TABLE legacy_folder_access_source_repairs;
                 DROP TABLE folder_access_sources;
                 DELETE FROM schema_migrations WHERE version = 19;
                 UPDATE shared_folder_connection_members SET manages_folder_access = 0;",
            )
            .unwrap();
        store.apply_migrations().unwrap();

        let migrated = store
            .load_shared_folder_connection("mount-ambiguous-pre-v19")
            .unwrap();
        assert!(
            migrated.managed_access_npubs.is_empty(),
            "ambiguous legacy access must not be treated as exclusively Mount-owned"
        );
        let repairs = store.load_legacy_folder_access_source_repairs().unwrap();
        assert_eq!(repairs.len(), 1);
        assert_eq!(
            repairs[0],
            LegacyFolderAccessSourceRepair {
                connection_id: "mount-ambiguous-pre-v19".to_owned(),
                brain_id: source_brain_id.clone(),
                folder_id: source_folder_id.clone(),
                user_id: destination_admin.clone(),
                reason: "legacy manages_folder_access=false cannot distinguish preexisting direct access from a pre-V17 Mount-owned grant".to_owned(),
                created_at: now.to_owned(),
            }
        );

        revoke_mount_for_test(
            &mut store,
            "mount-ambiguous-pre-v19",
            &source_admin,
            0,
            &[],
            &[],
            now,
        )
        .unwrap();
        let source = store.load_brain(&source_brain_id).unwrap();
        assert!(
            source
                .folder_access
                .get(&source_folder_id)
                .is_some_and(|users| users.contains(&destination_admin)),
            "ambiguous legacy access must remain until an operator resolves its provenance"
        );
    }

    #[test]
    fn legacy_personal_mounts_migrate_or_surface_explicit_repairs() {
        let mut store = store_with_strategy_folder();
        let now = "2026-06-23T00:00:00.000Z";
        bootstrap_personal_named(
            &mut store,
            "personal-destination",
            "npub-owner",
            "npub-agent",
            now,
        );
        store
            .conn
            .execute(
                r#"
                INSERT INTO personal_folder_mounts (
                    id, owner_npub, source_brain_id, source_folder_id, display_name,
                    display_parent_folder_id, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?6)
                "#,
                params![
                    "legacy-mount-resolvable",
                    "npub-owner",
                    "acme",
                    "strategy",
                    "Legacy Strategy",
                    now
                ],
            )
            .unwrap();
        store
            .conn
            .execute(
                r#"
                INSERT INTO personal_folder_mounts (
                    id, owner_npub, source_brain_id, source_folder_id, display_name,
                    display_parent_folder_id, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?6)
                "#,
                params![
                    "legacy-mount-needs-repair",
                    "npub-missing-owner",
                    "acme",
                    "strategy",
                    "Unresolved Strategy",
                    now
                ],
            )
            .unwrap();

        let tx = store.conn.transaction().unwrap();
        schema::migrate_legacy_personal_mounts(&tx).unwrap();
        schema::migrate_legacy_personal_mounts(&tx).unwrap();
        tx.commit().unwrap();

        let destination_brain_id = BrainId::new("personal-destination").unwrap();
        let mounts = store.load_folder_mounts(&destination_brain_id).unwrap();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].id, "legacy-mount-resolvable");
        assert_eq!(mounts[0].destination_brain_id, destination_brain_id);
        let connection = store
            .load_shared_folder_connection("legacy-personal-connection-legacy-mount-resolvable")
            .unwrap();
        assert_eq!(
            connection.member_npubs,
            BTreeSet::from([UserId::new("npub-owner").unwrap()])
        );
        assert_eq!(
            connection.managed_access_npubs,
            BTreeSet::from([UserId::new("npub-owner").unwrap()])
        );

        let repairs = store.load_legacy_personal_mount_repairs().unwrap();
        assert_eq!(repairs.len(), 1);
        assert_eq!(repairs[0].legacy_mount_id, "legacy-mount-needs-repair");
        assert!(repairs[0].reason.contains("has no Personal Brain"));
        let legacy_row_count: u64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM personal_folder_mounts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(legacy_row_count, 2, "legacy evidence must be preserved");
    }

    #[test]
    fn mount_acceptance_rolls_back_when_its_grant_record_cannot_commit() {
        let mut store = store_with_strategy_folder();
        bootstrap_org_named(&mut store, "dest", "Destination", "npub-dest-admin");
        let source_brain_id = BrainId::new("acme").unwrap();
        let source_folder_id = FolderId::new("strategy").unwrap();
        let destination_brain_id = BrainId::new("dest").unwrap();
        let source_admin = UserId::new("npub-admin").unwrap();
        let destination_admin = UserId::new("npub-dest-admin").unwrap();
        let now = "2026-06-23T00:00:00.000Z";
        let controller_grant = grant(
            "grant-atomic-mount-v1",
            "strategy",
            1,
            source_admin.as_str(),
            destination_admin.as_str(),
        );
        store
            .create_shared_folder_invitation(
                &source_brain_id,
                &source_folder_id,
                &destination_brain_id,
                "mount-offer-atomic",
                &destination_admin,
                &source_admin,
                "/v1/mount-offers/mount-offer-atomic/accept",
                &controller_grant,
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        let duplicate_record =
            folder_key_grant_control_record(&controller_grant, "duplicate-mount-grant-event");
        store
            .submit_sync_record(&source_brain_id, &duplicate_record)
            .unwrap();

        assert!(
            store
                .accept_shared_folder_invitation(
                    "mount-offer-atomic",
                    &destination_admin,
                    "mount-atomic",
                    "projection-atomic",
                    &[],
                    std::slice::from_ref(&duplicate_record),
                    now,
                )
                .is_err()
        );

        assert_eq!(
            store
                .load_shared_folder_invitation("mount-offer-atomic")
                .unwrap()
                .status,
            LinkStatus::Pending
        );
        let connection_count: u64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM shared_folder_connections WHERE id = 'mount-atomic'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(connection_count, 0);
        let source = store.load_brain(&source_brain_id).unwrap();
        assert!(
            !source
                .folder_access
                .get(&source_folder_id)
                .is_some_and(|users| users.contains(&destination_admin))
        );
        assert!(
            source
                .grants
                .iter()
                .all(|grant| grant.id != controller_grant.id)
        );
    }

    #[test]
    fn member_removal_atomically_removes_mount_participation() {
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
            .add_member(&destination_brain_id, &destination_member)
            .unwrap();
        store
            .create_shared_folder_invitation(
                &source_brain_id,
                &source_folder_id,
                &destination_brain_id,
                "mount-offer-member-removal",
                &destination_admin,
                &source_admin,
                "/v1/mount-offers/mount-offer-member-removal/accept",
                &grant(
                    "grant-member-removal-controller-v1",
                    "strategy",
                    1,
                    source_admin.as_str(),
                    destination_admin.as_str(),
                ),
                "2026-06-30T00:00:00.000Z",
                now,
            )
            .unwrap();
        accept_mount_for_test(
            &mut store,
            "mount-offer-member-removal",
            &destination_admin,
            "mount-member-removal",
            "projection-member-removal",
            &[],
            now,
        )
        .unwrap();
        add_mount_member_for_test(
            &mut store,
            "mount-member-removal",
            &destination_admin,
            &destination_member,
            &grant(
                "grant-member-removal-member-v1",
                "strategy",
                1,
                destination_admin.as_str(),
                destination_member.as_str(),
            ),
            now,
        )
        .unwrap();

        let mount_rotations = vec![MemberMountRotation {
            connection_id: "mount-member-removal".to_owned(),
            revoke_mount: false,
            new_key_version: 2,
            grants: vec![
                grant(
                    "grant-member-removal-source-v2",
                    "strategy",
                    2,
                    destination_admin.as_str(),
                    source_admin.as_str(),
                ),
                grant(
                    "grant-member-removal-controller-v2",
                    "strategy",
                    2,
                    destination_admin.as_str(),
                    destination_admin.as_str(),
                ),
            ],
            reencrypted_records: vec![],
        }];
        let control_records_by_brain = BTreeMap::from([
            (
                source_brain_id.clone(),
                mount_control_records(&mount_rotations[0].grants),
            ),
            (
                destination_brain_id.clone(),
                vec![brain_admin_control_record(
                    "member-removal-access-change",
                    destination_admin.as_str(),
                )],
            ),
        ]);
        store
            .remove_member_with_rotations_and_control_records(
                &destination_brain_id,
                &destination_admin,
                &destination_member,
                &[],
                &mount_rotations,
                now,
                &control_records_by_brain,
            )
            .unwrap();

        let destination = store.load_brain(&destination_brain_id).unwrap();
        assert!(
            !destination
                .brain
                .members
                .iter()
                .any(|member| member.user_id == destination_member)
        );
        let connection = store
            .load_shared_folder_connection("mount-member-removal")
            .unwrap();
        assert_eq!(connection.status, SharedFolderConnectionStatus::Active);
        assert!(!connection.member_npubs.contains(&destination_member));
        let source = store.load_brain(&source_brain_id).unwrap();
        assert!(
            !source
                .folder_access
                .get(&source_folder_id)
                .is_some_and(|users| users.contains(&destination_member))
        );
        assert_eq!(
            source
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == source_folder_id)
                .unwrap()
                .current_key_version,
            2
        );
        assert_eq!(
            store
                .sync_bootstrap(&source_brain_id)
                .unwrap()
                .control_records
                .iter()
                .filter(|record| record.record_type == SyncRecordType::FolderKeyGrant)
                .count(),
            4
        );
        assert_eq!(
            store
                .sync_bootstrap(&destination_brain_id)
                .unwrap()
                .control_records
                .iter()
                .filter(|record| record.record_type == SyncRecordType::BrainAdminAccessChange)
                .count(),
            1
        );
    }

    #[test]
    fn member_removal_mount_discovery_is_not_truncated_at_list_page_size() {
        let mut store = store_with_strategy_folder();
        bootstrap_org_named(&mut store, "dest", "Dest", "npub-dest-admin");
        let participant = UserId::new("npub-dest-member").unwrap();
        let now = "2026-06-23T00:00:00.000Z";
        for index in 0..=MAX_LINK_LIST_ROWS {
            let folder_id = format!("source-{index:03}");
            let connection_id = format!("mount-{index:03}");
            store
                .conn
                .execute(
                    "INSERT INTO folders (
                        brain_id, id, name, role, access, parent_folder_id,
                        parent_folder_key, path, current_key_version,
                        shared_folder_source, setup_incomplete, created_at
                     ) VALUES ('acme', ?1, ?1, 'folder', 'restricted', NULL, '', ?1, 1, 0, 0, ?2)",
                    params![folder_id, now],
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO shared_folder_connections (
                        id, source_brain_id, source_folder_id, destination_brain_id,
                        destination_admin_npub, status, created_at, updated_at
                     ) VALUES (?1, 'acme', ?2, 'dest', 'npub-dest-admin', 'active', ?3, ?3)",
                    params![connection_id, folder_id, now],
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO shared_folder_connection_members (
                        connection_id, member_npub, created_at
                     ) VALUES (?1, ?2, ?3)",
                    params![connection_id, participant.as_str(), now],
                )
                .unwrap();
        }

        let mounts = store
            .list_active_destination_mounts_for_participant(
                &BrainId::new("dest").unwrap(),
                &participant,
            )
            .unwrap();
        assert_eq!(mounts.len(), MAX_LINK_LIST_ROWS as usize + 1);
    }

    #[test]
    fn member_removal_rejects_rotation_fanout_before_loading_state() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let rotations = (0..=BRAIN_CAPACITY_ENVELOPE.folders
            + BRAIN_CAPACITY_ENVELOPE.shared_connections)
            .map(|index| MemberFolderRotation {
                folder_id: FolderId::new(format!("folder-{index}")).unwrap(),
                new_key_version: 2,
                grants: vec![],
                reencrypted_records: vec![],
            })
            .collect::<Vec<_>>();
        let error = store
            .remove_member_with_rotations(
                &BrainId::new("missing").unwrap(),
                &UserId::new("npub-admin").unwrap(),
                &UserId::new("npub-member").unwrap(),
                &rotations,
                &[],
                "2026-06-23T00:00:00.000Z",
            )
            .unwrap_err();
        assert!(matches!(
            error,
            StoreError::Core(CoreError::RotationFanoutLimitExceeded {
                resource: "Folder rotations",
                ..
            })
        ));
    }

    #[test]
    fn mount_offer_acceptance_supports_every_brain_kind_pair() {
        for source_personal in [false, true] {
            for destination_personal in [false, true] {
                let mut store = BrainStore::open_in_memory().unwrap();
                let now = "2026-06-23T00:00:00Z";
                let source_controller = UserId::new("npub-source-controller").unwrap();
                let destination_controller = UserId::new("npub-destination-controller").unwrap();

                if source_personal {
                    bootstrap_personal_named(
                        &mut store,
                        "source",
                        source_controller.as_str(),
                        "npub-source-agent",
                        now,
                    );
                } else {
                    bootstrap_org_named(&mut store, "source", "Source", source_controller.as_str());
                }
                if destination_personal {
                    bootstrap_personal_named(
                        &mut store,
                        "destination",
                        destination_controller.as_str(),
                        "npub-destination-agent",
                        now,
                    );
                } else {
                    bootstrap_org_named(
                        &mut store,
                        "destination",
                        "Destination",
                        destination_controller.as_str(),
                    );
                }

                let source_brain_id = BrainId::new("source").unwrap();
                let destination_brain_id = BrainId::new("destination").unwrap();
                let source_folder = Folder {
                    id: FolderId::new("shared").unwrap(),
                    name: DisplayName::new("folder_name", "Shared").unwrap(),
                    role: FolderRole::Folder,
                    access: FolderAccessMode::Restricted,
                    parent_folder_id: None,
                    path: SafeRelativePath::new("folder_path", "Shared").unwrap(),
                    current_key_version: 1,
                };
                let mut source_grants = vec![grant(
                    "grant-source-controller",
                    source_folder.id.as_str(),
                    1,
                    source_controller.as_str(),
                    source_controller.as_str(),
                )];
                if source_personal {
                    source_grants.push(grant(
                        "grant-source-agent",
                        source_folder.id.as_str(),
                        1,
                        source_controller.as_str(),
                        "npub-source-agent",
                    ));
                }
                store
                    .create_folder(
                        &source_brain_id,
                        &source_folder,
                        &BTreeSet::new(),
                        &source_grants,
                    )
                    .unwrap();
                let suffix = format!(
                    "{}-{}",
                    if source_personal { "personal" } else { "org" },
                    if destination_personal {
                        "personal"
                    } else {
                        "org"
                    }
                );
                let offer_id = format!("mount-offer-{suffix}");
                store
                    .create_shared_folder_invitation(
                        &source_brain_id,
                        &source_folder.id,
                        &destination_brain_id,
                        &offer_id,
                        &destination_controller,
                        &source_controller,
                        &format!("/v1/mount-offers/{offer_id}/accept"),
                        &grant(
                            &format!("grant-{suffix}-controller"),
                            source_folder.id.as_str(),
                            source_folder.current_key_version,
                            source_controller.as_str(),
                            destination_controller.as_str(),
                        ),
                        "2026-06-30T00:00:00Z",
                        now,
                    )
                    .unwrap();

                let supplemental = destination_personal.then(|| {
                    grant(
                        &format!("grant-{suffix}-agent"),
                        source_folder.id.as_str(),
                        source_folder.current_key_version,
                        destination_controller.as_str(),
                        "npub-destination-agent",
                    )
                });
                accept_mount_for_test(
                    &mut store,
                    &offer_id,
                    &destination_controller,
                    &format!("mount-{suffix}"),
                    &format!("projection-{suffix}"),
                    supplemental.as_slice(),
                    now,
                )
                .unwrap();

                let connection = store
                    .load_shared_folder_connection(&format!("mount-{suffix}"))
                    .unwrap();
                let mut expected = BTreeSet::from([destination_controller.clone()]);
                if destination_personal {
                    expected.insert(UserId::new("npub-destination-agent").unwrap());
                }
                assert_eq!(connection.member_npubs, expected, "{suffix}");
            }
        }
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
                    "/v1/invitations/invitation-dest-member/accept",
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
                    "/v1/shared-folder-invitations/shared-folder-invitation-lifecycle/accept",
                    &grant(
                        "grant-lifecycle-dest-admin-v1",
                        "strategy",
                        1,
                        "npub-admin",
                        destination_admin.as_str(),
                    ),
                    "2026-06-30T00:00:00.000Z",
                    now,
                )
                .unwrap();
            accept_mount_for_test(
                &mut store,
                "shared-folder-invitation-lifecycle",
                &destination_admin,
                "shared-folder-connection-lifecycle",
                "organization-mount-lifecycle",
                &[],
                now,
            )
            .unwrap();
            add_mount_member_for_test(
                &mut store,
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
                3
            );

            remove_mount_member_for_test(
                &mut store,
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

            assert_eq!(
                store
                    .encrypted_brain_export(&source_brain_id, &destination_member)
                    .unwrap_err(),
                StoreError::BrokenInvariant {
                    reason: "brain access required for encrypted export".to_owned()
                }
            );

            revoke_mount_for_test(
                &mut store,
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
                8
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
    fn access_removal_rolls_back_when_a_control_record_conflicts() {
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
                &folder_access_control_record(
                    "duplicate-access-change",
                    SyncRecordType::BrainAdminAccessChange,
                    "strategy",
                    "npub-admin",
                ),
            )
            .unwrap();
        let sequence_before = store.sync_bootstrap(&brain_id).unwrap().latest_sequence;
        let replacement_grant = grant(
            "grant-strategy-admin-v2",
            "strategy",
            2,
            "npub-admin",
            "npub-admin",
        );
        let control_records = [
            folder_key_grant_control_record(&replacement_grant, "replacement-grant-record"),
            folder_access_control_record(
                "duplicate-access-change",
                SyncRecordType::BrainAdminAccessChange,
                "strategy",
                "npub-admin",
            ),
        ];

        store
            .rotate_folder_key_for_access_removal_with_control_records(
                &brain_id,
                &folder_id,
                &member,
                2,
                std::slice::from_ref(&replacement_grant),
                &[],
                "2026-06-23T00:00:00.000Z",
                &control_records,
            )
            .unwrap_err();

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
        assert!(!stored.grants.contains(&replacement_grant));
        assert_eq!(
            store.sync_bootstrap(&brain_id).unwrap().latest_sequence,
            sequence_before
        );
    }

    #[test]
    fn folder_creation_rolls_back_when_a_control_record_conflicts() {
        let mut store = store_with_strategy_folder();
        let brain_id = BrainId::new("acme").unwrap();
        store
            .submit_sync_record(
                &brain_id,
                &folder_access_control_record(
                    "duplicate-create-change",
                    SyncRecordType::BrainAdminAccessChange,
                    "strategy",
                    "npub-admin",
                ),
            )
            .unwrap();
        let sequence_before = store.sync_bootstrap(&brain_id).unwrap().latest_sequence;
        let folder = Folder {
            id: FolderId::new("plans").unwrap(),
            name: DisplayName::new("folder_name", "Plans").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Restricted,
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Plans").unwrap(),
            current_key_version: 1,
        };
        let folder_grant = grant("grant-plans-admin", "plans", 1, "npub-admin", "npub-admin");
        let control_records = [
            folder_key_grant_control_record(&folder_grant, "plans-grant-record"),
            folder_access_control_record(
                "duplicate-create-change",
                SyncRecordType::BrainAdminAccessChange,
                "plans",
                "npub-admin",
            ),
        ];

        store
            .create_folder_with_control_records(
                &brain_id,
                &folder,
                &BTreeSet::new(),
                std::slice::from_ref(&folder_grant),
                &control_records,
            )
            .unwrap_err();

        assert!(
            store
                .load_brain(&brain_id)
                .unwrap()
                .brain
                .folders
                .iter()
                .all(|stored_folder| stored_folder.id != folder.id)
        );
        assert!(
            !store
                .load_brain(&brain_id)
                .unwrap()
                .grants
                .contains(&folder_grant)
        );
        assert_eq!(
            store.sync_bootstrap(&brain_id).unwrap().latest_sequence,
            sequence_before
        );
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

        store.add_member(&brain_id, &member).unwrap();
        assert!(store.member_exists(&brain_id, &member).unwrap());
    }

    #[test]
    fn personal_guest_is_removed_when_their_last_folder_scope_is_removed() {
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

        let before = store.load_brain(&brain_id).unwrap();
        assert_eq!(before.guest_user_ids(), BTreeSet::from([member.clone()]));
        assert!(
            !before
                .brain
                .members
                .iter()
                .any(|stored_member| stored_member.user_id == member)
        );

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
        assert!(stored.guest_user_ids().is_empty());
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
    fn member_add_rolls_back_when_its_control_record_conflicts() {
        let mut store = org_store_with_access_test_folders();
        let brain_id = BrainId::new("acme").unwrap();
        let member = UserId::new("npub-member").unwrap();
        let duplicate = brain_admin_control_record("duplicate-member-change", "npub-admin");
        store.submit_sync_record(&brain_id, &duplicate).unwrap();
        let sequence_before = store.sync_bootstrap(&brain_id).unwrap().latest_sequence;

        store
            .add_member_with_control_records(
                &brain_id,
                &member,
                &[brain_admin_control_record(
                    "duplicate-member-change",
                    "npub-admin",
                )],
            )
            .unwrap_err();

        assert!(!store.member_exists(&brain_id, &member).unwrap());
        assert_eq!(
            store.sync_bootstrap(&brain_id).unwrap().latest_sequence,
            sequence_before
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
                reason: "remove explicit Folder access before removing member".to_owned()
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
            store
                .create_brain_invitation(
                    &brain_id,
                    "restore-pending-invitation",
                    &UserId::new("npub-restore-invite").unwrap(),
                    "restore-pending-invite-code-000001",
                    "/v1/brain-invitation-links/restore-pending-invite-code-000001/accept",
                    &[],
                    &UserId::new("npub-admin").unwrap(),
                    "2026-06-30T00:00:00Z",
                    "2026-06-23T00:00:00Z",
                )
                .unwrap();
            store
                .conn
                .execute_batch(
                    r#"
                    INSERT INTO account_access_cohorts (
                        id, brain_id, human_npub, human_email, scope_kind, folder_id,
                        provenance_kind, provenance_id, roster_revision, status,
                        created_at, updated_at, account_id
                    ) VALUES (
                        'restore-cohort', 'acme', 'npub-human', 'human@finite.vip',
                        'brain', NULL, 'internal_beta_reconciliation', 'restore-source',
                        7, 'active', '2026-06-23T00:00:00Z',
                        '2026-06-23T00:00:00Z', 'account-restore'
                    );
                    INSERT INTO account_access_cohort_participants (
                        cohort_id, participant_npub, relationship, nip05, display_name,
                        status, created_at, updated_at
                    ) VALUES
                        ('restore-cohort', 'npub-human', 'human', 'human@finite.vip',
                         'Human', 'active', '2026-06-23T00:00:00Z', '2026-06-23T00:00:00Z'),
                        ('restore-cohort', 'npub-agent', 'account_agent', 'agent@finite.vip',
                         'Agent', 'excluded', '2026-06-23T00:00:00Z', '2026-06-23T00:00:00Z');
                    INSERT INTO human_anchored_agent_authorities (
                        cohort_id, brain_id, human_npub, agent_npub, status,
                        created_at, updated_at
                    ) VALUES (
                        'restore-cohort', 'acme', 'npub-human', 'npub-agent', 'active',
                        '2026-06-23T00:00:00Z', '2026-06-23T00:00:00Z'
                    );
                    INSERT INTO account_access_cohort_exclusions (
                        cohort_id, participant_npub, folder_id, reason, active,
                        created_at, updated_at
                    ) VALUES (
                        'restore-cohort', 'npub-agent', '', 'explicit_peer_restriction', 1,
                        '2026-06-23T00:00:00Z', '2026-06-23T00:00:00Z'
                    );
                    INSERT INTO account_access_cohort_audit (
                        id, cohort_id, action, actor_npub, anchoring_human_npub,
                        detail_json, occurred_at
                    ) VALUES (
                        'restore-audit', 'restore-cohort', 'participant_brain_restricted',
                        'npub-agent-actor', 'npub-human', '{"restored":true}',
                        '2026-06-23T00:00:00Z'
                    );
                    INSERT INTO personal_brain_agents (
                        brain_id, agent_npub, agent_nip05, display_name, status,
                        roster_revision, blocker, created_at, updated_at
                    ) VALUES (
                        'acme', 'npub-agent', 'agent@finite.vip', 'Agent', 'blocked',
                        7, 'current_folder_key_unavailable', '2026-06-23T00:00:00Z',
                        '2026-06-23T00:00:00Z'
                    );
                    "#,
                )
                .unwrap();
        }

        std::fs::copy(&source_db, &restored_db).unwrap();

        let mut restored = BrainStore::open(&restored_db).unwrap();
        let bootstrap = restored.sync_bootstrap(&brain_id).unwrap();
        assert_eq!(bootstrap.latest_sequence, 2);
        assert_eq!(bootstrap.object_count, 1);
        assert_eq!(bootstrap.objects[0].revision, 2);
        let restored_state = restored.load_brain(&brain_id).unwrap();
        assert_eq!(restored_state.account_access_cohorts.len(), 1);
        assert_eq!(restored_state.human_anchored_agent_authorities.len(), 1);
        assert_eq!(restored_state.account_agent_exclusions.len(), 1);
        assert_eq!(restored_state.personal_brain_agents.len(), 1);
        assert_eq!(
            restored.list_brain_invitations(&brain_id).unwrap()[0].status,
            LinkStatus::Pending
        );
        let restored_audit_count: i64 = restored
            .conn
            .query_row(
                "SELECT COUNT(*) FROM account_access_cohort_audit WHERE cohort_id = 'restore-cohort'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(restored_audit_count, 1);

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

    #[test]
    fn reconciliation_capacity_blocker_is_stable_and_does_not_create_exclusions() {
        let store = empty_org_store();
        let brain_id = BrainId::new("acme").unwrap();
        let human = UserId::new("npub-capacity-human").unwrap();
        store
            .conn
            .execute(
                "INSERT INTO brain_members (brain_id, user_id) VALUES (?1, ?2)",
                params![brain_id.as_str(), human.as_str()],
            )
            .unwrap();
        for index in 0..BRAIN_CAPACITY_ENVELOPE.members.saturating_sub(2) {
            store
                .conn
                .execute(
                    "INSERT INTO brain_members (brain_id, user_id) VALUES (?1, ?2)",
                    params![brain_id.as_str(), format!("npub-capacity-{index}")],
                )
                .unwrap();
        }
        let cohort = BootstrapAccountCohort {
            account_id: "account-capacity".to_owned(),
            human_email: "capacity@finite.vip".to_owned(),
            roster_revision: 9,
            participants: vec![
                StoredCohortParticipant {
                    relationship: "human".to_owned(),
                    name: "Capacity Human".to_owned(),
                    nip05: "capacity@finite.vip".to_owned(),
                    npub: human,
                },
                StoredCohortParticipant {
                    relationship: "account_agent".to_owned(),
                    name: "Capacity Agent".to_owned(),
                    nip05: "capacity-agent@finite.vip".to_owned(),
                    npub: UserId::new("npub-capacity-agent").unwrap(),
                },
            ],
        };
        let actor = UserId::new("npub-admin").unwrap();
        let first = store
            .plan_account_cohort_reconciliation(&brain_id, &cohort, None, &actor)
            .unwrap();
        let second = store
            .plan_account_cohort_reconciliation(&brain_id, &cohort, None, &actor)
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.blocker.as_deref(), Some("capacity_exceeded"));
        assert_eq!(
            first.capacity.members_after,
            BRAIN_CAPACITY_ENVELOPE.members + 1
        );
        let stored = store.load_brain(&brain_id).unwrap();
        assert!(stored.account_access_cohorts.is_empty());
        assert!(stored.account_agent_exclusions.is_empty());
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
            },
            Folder {
                id: FolderId::new("private-project").unwrap(),
                name: DisplayName::new("folder_name", "Private Project").unwrap(),
                role: FolderRole::Folder,
                access: FolderAccessMode::Restricted,
                parent_folder_id: None,
                path: SafeRelativePath::new("folder_path", "Private Project").unwrap(),
                current_key_version: 1,
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

    fn bootstrap_personal_named(
        store: &mut BrainStore,
        id: &str,
        owner: &str,
        agent: &str,
        now: &str,
    ) {
        let output = bootstrap_personal_brain(id, "Personal", owner).unwrap();
        let grants = grants_for_required(&output.required_key_grants, owner);
        store
            .create_personal_brain_bootstrap(
                &output,
                &grants,
                &UserId::new(agent).unwrap(),
                &UserId::new(owner).unwrap(),
                now,
            )
            .unwrap();
        let brain_id = BrainId::new(id).unwrap();
        let stored = store.load_brain(&brain_id).unwrap();
        for folder_id in stored.setup_incomplete_folder_ids {
            let folder = stored
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == folder_id)
                .unwrap();
            store
                .finish_folder_setup(
                    &brain_id,
                    &folder_id,
                    &[
                        grant(
                            &format!("grant-{id}-{folder_id}-owner"),
                            folder_id.as_str(),
                            folder.current_key_version,
                            owner,
                            owner,
                        ),
                        grant(
                            &format!("grant-{id}-{folder_id}-agent"),
                            folder_id.as_str(),
                            folder.current_key_version,
                            owner,
                            agent,
                        ),
                    ],
                )
                .unwrap();
        }
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

    fn mount_control_records(grants: &[FolderKeyGrantMetadata]) -> Vec<SyncRecordInput> {
        grants
            .iter()
            .map(|grant| {
                folder_key_grant_control_record(grant, &format!("{}-test-control-record", grant.id))
            })
            .collect()
    }

    fn accept_mount_for_test(
        store: &mut BrainStore,
        invitation_id: &str,
        destination_admin_npub: &UserId,
        connection_id: &str,
        mount_id: &str,
        supplemental_grants: &[FolderKeyGrantMetadata],
        now: &str,
    ) -> Result<StoredSharedFolderInvitation, StoreError> {
        let invitation = store.load_shared_folder_invitation(invitation_id)?;
        let all_grants = std::iter::once(invitation.folder_key_grant)
            .chain(supplemental_grants.iter().cloned())
            .collect::<Vec<_>>();
        let control_records = mount_control_records(&all_grants);
        store.accept_shared_folder_invitation(
            invitation_id,
            destination_admin_npub,
            connection_id,
            mount_id,
            supplemental_grants,
            &control_records,
            now,
        )
    }

    fn add_mount_member_for_test(
        store: &mut BrainStore,
        connection_id: &str,
        actor_npub: &UserId,
        target_npub: &UserId,
        grant: &FolderKeyGrantMetadata,
        now: &str,
    ) -> Result<StoredSharedFolderConnection, StoreError> {
        let control_records = mount_control_records(std::slice::from_ref(grant));
        store.add_shared_folder_connection_member(
            connection_id,
            actor_npub,
            target_npub,
            grant,
            &control_records,
            now,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn remove_mount_member_for_test(
        store: &mut BrainStore,
        connection_id: &str,
        actor_npub: &UserId,
        target_npub: &UserId,
        new_key_version: u32,
        grants: &[FolderKeyGrantMetadata],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        now: &str,
    ) -> Result<StoredSharedFolderConnection, StoreError> {
        let control_records = mount_control_records(grants);
        store.remove_shared_folder_connection_member(
            connection_id,
            actor_npub,
            target_npub,
            new_key_version,
            grants,
            &control_records,
            reencrypted_records,
            now,
        )
    }

    fn revoke_mount_for_test(
        store: &mut BrainStore,
        connection_id: &str,
        actor_npub: &UserId,
        new_key_version: u32,
        grants: &[FolderKeyGrantMetadata],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        now: &str,
    ) -> Result<StoredSharedFolderConnection, StoreError> {
        let control_records = mount_control_records(grants);
        store.revoke_shared_folder_connection(
            connection_id,
            actor_npub,
            new_key_version,
            grants,
            &control_records,
            reencrypted_records,
            now,
        )
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

    fn folder_key_grant_control_record(
        grant: &FolderKeyGrantMetadata,
        record_event_id: &str,
    ) -> SyncRecordInput {
        SyncRecordInput::Control(ControlSyncRecord {
            record_event_id: record_event_id.to_owned(),
            record_type: SyncRecordType::FolderKeyGrant,
            folder_id: Some(grant.folder_id.clone()),
            actor_npub: grant.issuer_npub.clone(),
            client_created_at: grant.created_at.clone(),
            payload_json: "{}".to_owned(),
            record_event_kind: NIP59_GIFT_WRAP_KIND,
        })
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

    fn brain_admin_control_record(event_id: &str, actor_npub: &str) -> SyncRecordInput {
        SyncRecordInput::Control(ControlSyncRecord {
            record_event_id: event_id.to_owned(),
            record_type: SyncRecordType::BrainAdminAccessChange,
            folder_id: None,
            actor_npub: UserId::new(actor_npub).unwrap(),
            client_created_at: "2026-06-23T00:00:00.000Z".to_owned(),
            payload_json: "{\"control\":true}".to_owned(),
            record_event_kind: APP_SPECIFIC_KIND,
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
