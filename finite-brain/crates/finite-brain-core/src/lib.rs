//! FiniteBrain Portable v1 core domain and validation logic.

pub mod portability;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, OsRng, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use finite_nostr::{
    GiftWrapValidation, NostrPublicKey, build_rumor, open_gift_wrap, validate_gift_wrap,
    verify_event_integrity, wrap_rumor,
};
use nostr::event::FinalizeEvent;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

const RESERVED_TOP_LEVEL_NAMES: [&str; 3] = [".finitebrain", "_admin", ".git"];
const FOLDER_OBJECT_VERSION: &str = "finite-folder-object-v1";
const CIPHER_AES_256_GCM: &str = "AES-256-GCM";
const APP_SPECIFIC_KIND: u16 = 30_078;
const MAX_USER_ID_LEN: usize = 128;
const MAX_DISPLAY_NAME_LEN: usize = 128;
const MAX_SAFE_RELATIVE_PATH_LEN: usize = 1024;
const MAX_BRAIN_INVITE_BOOTSTRAP_FOLDERS: usize = 100;
/// Maximum number of Folder rotations accepted in one Personal Agent request.
pub const MAX_PERSONAL_AGENT_ROTATION_FOLDERS: usize = 100;
/// Maximum Folder Key Grants accepted for one Folder rotation.
pub const MAX_FOLDER_ROTATION_GRANTS: usize = 1_000;
/// Maximum re-encrypted object records accepted for one Folder rotation.
pub const MAX_FOLDER_ROTATION_RECORDS: usize = 1_000;
/// Maximum total Folder Key Grants accepted in one Personal Agent request.
pub const MAX_PERSONAL_AGENT_ROTATION_GRANTS: usize = 10_000;
/// Maximum total re-encrypted records accepted in one Personal Agent request.
pub const MAX_PERSONAL_AGENT_ROTATION_RECORDS: usize = 10_000;
/// Maximum Folder Key Grants accepted in one Folder access-removal request.
pub const MAX_FOLDER_ACCESS_REMOVAL_GRANTS: usize = MAX_FOLDER_ROTATION_GRANTS;
/// Maximum re-encrypted records accepted in one Folder access-removal request.
pub const MAX_FOLDER_ACCESS_REMOVAL_RECORDS: usize = MAX_FOLDER_ROTATION_RECORDS;

/// Versioned official Product Client identity boundary.
pub const BRAIN_IDENTITY_PROVIDER_VERSION: &str = "finite-brain-identity-provider-v1";

/// Unsigned Nostr event accepted only after a named Brain intent validates it.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct BrainEventTemplate {
    pub kind: u16,
    pub created_at: u64,
    pub tags: Vec<Vec<String>>,
    pub content: String,
}

/// Typed protected-request authorization presented to the Brain Identity Provider.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainHttpAuthorizationIntent {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub body_text: String,
    pub event_template: BrainEventTemplate,
}

/// Typed Brain event authorization presented to the Brain Identity Provider.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainEventAuthorizationIntent {
    pub intent: String,
    pub event_template: BrainEventTemplate,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrainEmailInviteAuthorizationPayload {
    version: String,
    vault_id: String,
    invited_email: String,
    invite_unwrap_npub: String,
    bootstrap_payload_hash: String,
    expires_at: String,
    folders: Vec<BrainEmailInviteAuthorizationFolder>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrainEmailInviteAuthorizationFolder {
    folder_id: String,
    access: FolderAccessMode,
    key_version: u32,
}

impl BrainEmailInviteAuthorizationPayload {
    fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"vaultId\":{},\"invitedEmail\":{},\"inviteUnwrapNpub\":{},\"bootstrapPayloadHash\":{},\"expiresAt\":{},\"folders\":{}}}",
            json_string(&self.version),
            json_string(&self.vault_id),
            json_string(&self.invited_email),
            json_string(&self.invite_unwrap_npub),
            json_string(&self.bootstrap_payload_hash),
            json_string(&self.expires_at),
            serde_json::to_string(&self.folders)
                .expect("serializing invite Folder scope cannot fail"),
        )
    }
}

/// Typed NIP-44 operation for a Brain-owned Folder Key or invitation grant.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainGrantIntent {
    pub purpose: String,
    pub vault_id: String,
    pub recipient_npub: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    #[serde(default)]
    pub key_version: Option<u32>,
}

/// Returns the crate name used in workspace status surfaces.
pub fn crate_name() -> &'static str {
    "finite-brain-core"
}

/// Core domain validation errors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CoreError {
    /// A stable id is empty or not path-safe.
    InvalidId { field: &'static str, value: String },
    /// A display name is empty or contains forbidden characters.
    InvalidName { field: &'static str, value: String },
    /// A path is not a safe relative path.
    InvalidPath { field: &'static str, value: String },
    /// A case-sensitive product identity collision occurred.
    Collision { field: &'static str, value: String },
    /// A folder hierarchy operation is invalid.
    InvalidHierarchy { reason: String },
    /// Bootstrap input is incomplete or violates the Vault kind rules.
    InvalidBootstrapInput { reason: String },
    /// Folder access metadata cannot produce a valid current key recipient set.
    InvalidAccessPolicy { reason: String },
    /// A key-rotation request exceeded a documented per-request work bound.
    RotationFanoutLimitExceeded {
        operation: &'static str,
        resource: &'static str,
        count: usize,
        max: usize,
    },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId { field, value } => {
                write!(f, "invalid id for {field}: {value}")
            }
            Self::InvalidName { field, value } => {
                write!(f, "invalid name for {field}: {value}")
            }
            Self::InvalidPath { field, value } => {
                write!(f, "invalid path for {field}: {value}")
            }
            Self::Collision { field, value } => {
                write!(f, "collision for {field}: {value}")
            }
            Self::InvalidHierarchy { reason } => write!(f, "invalid hierarchy: {reason}"),
            Self::InvalidBootstrapInput { reason } => {
                write!(f, "invalid bootstrap input: {reason}")
            }
            Self::InvalidAccessPolicy { reason } => {
                write!(f, "invalid Folder access policy: {reason}")
            }
            Self::RotationFanoutLimitExceeded {
                operation,
                resource,
                count,
                max,
            } => write!(
                f,
                "{operation} exceeds {resource} limit: {count} supplied, maximum {max}"
            ),
        }
    }
}

impl Error for CoreError {}

/// Stable Vault id.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct VaultId(String);

impl VaultId {
    /// Validate and create a Vault id.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        validate_stable_id("vault_id", value.into(), 1, 128).map(Self)
    }

    /// Borrow the normalized id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VaultId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable Folder id.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct FolderId(String);

impl FolderId {
    /// Validate and create a Folder id.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        validate_stable_id("folder_id", value.into(), 1, 128).map(Self)
    }

    /// Borrow the normalized id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable Folder Object id.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct ObjectId(String);

impl ObjectId {
    /// Validate and create a Folder Object id.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let normalized = validate_stable_id("object_id", value.into(), 16, 128)?;
        if normalized.contains('.') {
            return Err(CoreError::InvalidId {
                field: "object_id",
                value: normalized,
            });
        }
        Ok(Self(normalized))
    }

    /// Borrow the normalized id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Nostr user id as stored by FiniteBrain.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct UserId(String);

impl UserId {
    /// Validate and create a user id.
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let normalized = normalize_nfc(&value);
        if normalized.is_empty()
            || normalized.len() > MAX_USER_ID_LEN
            || contains_nul_or_control(&normalized)
        {
            return Err(CoreError::InvalidId {
                field: "user_id",
                value,
            });
        }
        Ok(Self(normalized))
    }

    /// Borrow the normalized id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// User-facing Folder or Vault display name.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct DisplayName(String);

impl DisplayName {
    /// Validate and normalize a display name.
    pub fn new(field: &'static str, value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let normalized = normalize_nfc(&value);
        if normalized.is_empty()
            || normalized.len() > MAX_DISPLAY_NAME_LEN
            || normalized.contains('/')
            || contains_nul_or_control(&normalized)
            || normalized == "."
            || normalized == ".."
        {
            return Err(CoreError::InvalidName { field, value });
        }
        Ok(Self(normalized))
    }

    /// Borrow the normalized display name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Safe relative path normalized to Unicode NFC.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct SafeRelativePath(String);

impl SafeRelativePath {
    /// Validate a Folder path or decrypted object path.
    pub fn new(field: &'static str, value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let normalized = normalize_nfc(&value);

        if normalized.is_empty()
            || normalized.len() > MAX_SAFE_RELATIVE_PATH_LEN
            || normalized.starts_with('/')
            || normalized.contains('\\')
            || contains_nul_or_control(&normalized)
        {
            return Err(CoreError::InvalidPath { field, value });
        }

        let segments = normalized.split('/').collect::<Vec<_>>();
        if segments
            .iter()
            .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
        {
            return Err(CoreError::InvalidPath { field, value });
        }

        if RESERVED_TOP_LEVEL_NAMES.contains(&segments[0]) {
            return Err(CoreError::InvalidPath { field, value });
        }

        Ok(Self(normalized))
    }

    /// Borrow the normalized path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SafeRelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Vault kind.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultKind {
    /// Personal Vault owned by one user.
    Personal,
    /// Organization Vault with members and admins.
    Organization,
}

/// Folder role.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderRole {
    /// Personal home folder.
    PersonalHome,
    /// Organization operations/admin folder.
    VaultOps,
    /// Organization general folder.
    General,
    /// Ordinary folder.
    Folder,
}

/// Binary access mode for a Folder.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderAccessMode {
    /// Personal Vault owner only.
    Owner,
    /// Organization Vault admins only.
    AdminOnly,
    /// Organization members and admins.
    AllMembers,
    /// Vault admins, the personal owner for personal Vaults, plus explicitly listed members.
    Restricted,
}

/// Vault member metadata.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct VaultMember {
    /// Member user id.
    pub user_id: UserId,
    /// Explicit restricted Folder Access entries.
    pub folder_access: BTreeSet<FolderId>,
}

/// Folder metadata.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    /// Stable folder id.
    pub id: FolderId,
    /// User-facing display name.
    pub name: DisplayName,
    /// Folder role.
    pub role: FolderRole,
    /// Binary access mode.
    pub access: FolderAccessMode,
    /// Optional parent Folder id.
    pub parent_folder_id: Option<FolderId>,
    /// Decorated Folder hierarchy path.
    pub path: SafeRelativePath,
    /// Current Folder Key version.
    pub current_key_version: u32,
    /// Whether this Folder is a shared-folder source.
    pub shared_folder_source: bool,
}

/// Folder Object metadata without encrypted bytes.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct FolderObject {
    /// Stable object id.
    pub object_id: ObjectId,
    /// Containing Folder id.
    pub folder_id: FolderId,
    /// Encrypted plaintext path.
    pub plaintext_path: SafeRelativePath,
}

/// Vault metadata.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Vault {
    /// Stable Vault id.
    pub id: VaultId,
    /// Vault kind.
    pub kind: VaultKind,
    /// User-facing Vault name.
    pub name: DisplayName,
    /// Personal Vault owner, if this is a personal Vault.
    pub owner_user_id: Option<UserId>,
    /// Folders in this Vault.
    pub folders: Vec<Folder>,
    /// Organization members.
    pub members: Vec<VaultMember>,
    /// Organization admins.
    pub admins: Vec<UserId>,
}

/// Typed inputs for computing the complete current Folder Key recipient set.
#[derive(Debug, Clone, Copy)]
pub struct FolderKeyRecipientPolicy<'a> {
    /// Personal Vault or Organization Vault semantics.
    pub vault_kind: VaultKind,
    /// Folder access mode.
    pub folder_access: FolderAccessMode,
    /// Sole owner for a Personal Vault.
    pub owner_user_id: Option<&'a UserId>,
    /// Organization Vault admins.
    pub admins: &'a [UserId],
    /// Organization Vault members.
    pub members: &'a [UserId],
    /// Explicit restricted-Folder recipients.
    pub explicit_access_user_ids: &'a BTreeSet<UserId>,
    /// Current Personal Agent, when the role is occupied.
    pub personal_agent_npub: Option<&'a UserId>,
}

/// Compute the canonical set of recipients for the current Folder Key.
pub fn required_folder_key_recipients(
    policy: FolderKeyRecipientPolicy<'_>,
) -> Result<BTreeSet<UserId>, CoreError> {
    let mut recipients = BTreeSet::new();

    if policy.vault_kind == VaultKind::Personal {
        let owner = policy
            .owner_user_id
            .ok_or_else(|| CoreError::InvalidAccessPolicy {
                reason: "Personal Vault Folder access requires an owner".to_owned(),
            })?;
        recipients.insert(owner.clone());
        recipients.extend(policy.personal_agent_npub.cloned());
    }

    match policy.folder_access {
        FolderAccessMode::Owner => {
            let owner = policy
                .owner_user_id
                .ok_or_else(|| CoreError::InvalidAccessPolicy {
                    reason: "owner access requires a Personal Vault owner".to_owned(),
                })?;
            recipients.insert(owner.clone());
        }
        FolderAccessMode::AdminOnly => {
            recipients.extend(policy.admins.iter().cloned());
        }
        FolderAccessMode::AllMembers => {
            recipients.extend(policy.admins.iter().cloned());
            recipients.extend(policy.members.iter().cloned());
        }
        FolderAccessMode::Restricted => {
            recipients.extend(policy.owner_user_id.cloned());
            recipients.extend(policy.admins.iter().cloned());
            recipients.extend(policy.explicit_access_user_ids.iter().cloned());
        }
    }

    if recipients.is_empty() {
        return Err(CoreError::InvalidAccessPolicy {
            reason: "current Folder Key must have at least one recipient".to_owned(),
        });
    }
    Ok(recipients)
}

/// One Folder's client-supplied key-rotation work counts.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct FolderRotationFanout {
    pub grants: usize,
    pub reencrypted_records: usize,
}

/// Rotation endpoint whose request fanout is being validated.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FolderRotationOperation {
    PersonalAgent,
    FolderAccessRemoval,
}

/// Validate key-rotation work before cryptography, record parsing, or durable mutation.
pub fn validate_folder_rotation_fanout(
    operation: FolderRotationOperation,
    rotations: impl IntoIterator<Item = FolderRotationFanout>,
) -> Result<(), CoreError> {
    let operation_name = match operation {
        FolderRotationOperation::PersonalAgent => "Personal Agent rotation",
        FolderRotationOperation::FolderAccessRemoval => "Folder access removal",
    };
    let max_rotations = match operation {
        FolderRotationOperation::PersonalAgent => MAX_PERSONAL_AGENT_ROTATION_FOLDERS,
        FolderRotationOperation::FolderAccessRemoval => 1,
    };
    let max_total_grants = match operation {
        FolderRotationOperation::PersonalAgent => MAX_PERSONAL_AGENT_ROTATION_GRANTS,
        FolderRotationOperation::FolderAccessRemoval => MAX_FOLDER_ACCESS_REMOVAL_GRANTS,
    };
    let max_total_records = match operation {
        FolderRotationOperation::PersonalAgent => MAX_PERSONAL_AGENT_ROTATION_RECORDS,
        FolderRotationOperation::FolderAccessRemoval => MAX_FOLDER_ACCESS_REMOVAL_RECORDS,
    };

    let mut rotation_count = 0usize;
    let mut total_grants = 0usize;
    let mut total_records = 0usize;
    for rotation in rotations {
        rotation_count = rotation_count.saturating_add(1);
        if rotation_count > max_rotations {
            return Err(CoreError::RotationFanoutLimitExceeded {
                operation: operation_name,
                resource: "Folder rotations",
                count: rotation_count,
                max: max_rotations,
            });
        }
        if rotation.grants > MAX_FOLDER_ROTATION_GRANTS {
            return Err(CoreError::RotationFanoutLimitExceeded {
                operation: operation_name,
                resource: "grants per Folder rotation",
                count: rotation.grants,
                max: MAX_FOLDER_ROTATION_GRANTS,
            });
        }
        if rotation.reencrypted_records > MAX_FOLDER_ROTATION_RECORDS {
            return Err(CoreError::RotationFanoutLimitExceeded {
                operation: operation_name,
                resource: "re-encrypted records per Folder rotation",
                count: rotation.reencrypted_records,
                max: MAX_FOLDER_ROTATION_RECORDS,
            });
        }
        total_grants = total_grants.saturating_add(rotation.grants);
        total_records = total_records.saturating_add(rotation.reencrypted_records);
        if total_grants > max_total_grants {
            return Err(CoreError::RotationFanoutLimitExceeded {
                operation: operation_name,
                resource: "aggregate grants",
                count: total_grants,
                max: max_total_grants,
            });
        }
        if total_records > max_total_records {
            return Err(CoreError::RotationFanoutLimitExceeded {
                operation: operation_name,
                resource: "aggregate re-encrypted records",
                count: total_records,
                max: max_total_records,
            });
        }
    }
    Ok(())
}

/// Required current Folder Key Grant recipient produced by bootstrap.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RequiredFolderKeyGrant {
    /// Folder receiving a grant.
    pub folder_id: FolderId,
    /// Recipient user id.
    pub recipient_user_id: UserId,
    /// Folder Key version.
    pub key_version: u32,
}

/// Bootstrap output for an initial Vault.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapOutput {
    /// Created Vault metadata.
    pub vault: Vault,
    /// Required current key grants.
    pub required_key_grants: Vec<RequiredFolderKeyGrant>,
}

/// Mutable pure-domain collection used to enforce hierarchy collisions.
#[derive(Debug, Clone, Default)]
pub struct VaultDraft {
    folders_by_id: BTreeMap<FolderId, Folder>,
    sibling_names: BTreeSet<(Option<FolderId>, DisplayName)>,
    object_paths: BTreeSet<(FolderId, SafeRelativePath)>,
    object_ids: BTreeSet<(FolderId, ObjectId)>,
}

impl VaultDraft {
    /// Add a Folder while enforcing id, parent, and sibling-name uniqueness.
    pub fn add_folder(&mut self, folder: Folder) -> Result<(), CoreError> {
        if self.folders_by_id.contains_key(&folder.id) {
            return Err(CoreError::Collision {
                field: "folder_id",
                value: folder.id.to_string(),
            });
        }

        if let Some(parent_id) = &folder.parent_folder_id
            && !self.folders_by_id.contains_key(parent_id)
        {
            return Err(CoreError::InvalidHierarchy {
                reason: format!("missing parent folder: {parent_id}"),
            });
        }

        let sibling_key = (folder.parent_folder_id.clone(), folder.name.clone());
        if !self.sibling_names.insert(sibling_key) {
            return Err(CoreError::Collision {
                field: "sibling_folder_name",
                value: folder.name.to_string(),
            });
        }

        self.folders_by_id.insert(folder.id.clone(), folder);
        Ok(())
    }

    /// Add a Folder Object while enforcing object id and page-path uniqueness per Folder.
    pub fn add_object(&mut self, object: FolderObject) -> Result<(), CoreError> {
        if !self.folders_by_id.contains_key(&object.folder_id) {
            return Err(CoreError::InvalidHierarchy {
                reason: format!("missing object folder: {}", object.folder_id),
            });
        }

        if !self
            .object_ids
            .insert((object.folder_id.clone(), object.object_id.clone()))
        {
            return Err(CoreError::Collision {
                field: "object_id",
                value: object.object_id.as_str().to_owned(),
            });
        }

        if !self
            .object_paths
            .insert((object.folder_id.clone(), object.plaintext_path.clone()))
        {
            return Err(CoreError::Collision {
                field: "object_path",
                value: object.plaintext_path.to_string(),
            });
        }

        Ok(())
    }

    /// Return folders in id order for deterministic tests/smoke output.
    pub fn folders(&self) -> Vec<Folder> {
        self.folders_by_id.values().cloned().collect()
    }
}

/// Build the initial personal Vault shape.
pub fn bootstrap_personal_vault(
    vault_id: impl Into<String>,
    name: impl Into<String>,
    owner_user_id: impl Into<String>,
) -> Result<BootstrapOutput, CoreError> {
    let vault_id = VaultId::new(vault_id)?;
    let name = DisplayName::new("vault_name", name)?;
    let owner_user_id = UserId::new(owner_user_id)?;

    let vault = Vault {
        id: vault_id,
        kind: VaultKind::Personal,
        name,
        owner_user_id: Some(owner_user_id),
        folders: Vec::new(),
        members: Vec::new(),
        admins: Vec::new(),
    };

    Ok(BootstrapOutput {
        vault,
        required_key_grants: Vec::new(),
    })
}

/// Build the initial organization Vault shape.
pub fn bootstrap_organization_vault(
    vault_id: impl Into<String>,
    name: impl Into<String>,
    admin_user_id: impl Into<String>,
) -> Result<BootstrapOutput, CoreError> {
    bootstrap_organization_vault_with_admins(vault_id, name, vec![admin_user_id.into()])
}

/// Build an organization Vault created by an agent for an authenticated human
/// requester. Both distinct Member Identities are initial admins.
pub fn bootstrap_organization_vault_with_requester(
    vault_id: impl Into<String>,
    name: impl Into<String>,
    creator_user_id: impl Into<String>,
    requesting_user_id: impl Into<String>,
) -> Result<BootstrapOutput, CoreError> {
    let creator_user_id = creator_user_id.into();
    let requesting_user_id = requesting_user_id.into();
    if creator_user_id == requesting_user_id {
        return Err(CoreError::InvalidBootstrapInput {
            reason: "organization Vault creator and requester must be distinct Member Identities"
                .to_owned(),
        });
    }
    bootstrap_organization_vault_with_admins(
        vault_id,
        name,
        vec![creator_user_id, requesting_user_id],
    )
}

fn bootstrap_organization_vault_with_admins(
    vault_id: impl Into<String>,
    name: impl Into<String>,
    admin_user_ids: Vec<String>,
) -> Result<BootstrapOutput, CoreError> {
    let vault_id = VaultId::new(vault_id)?;
    let name = DisplayName::new("vault_name", name)?;
    let admin_user_ids = admin_user_ids
        .into_iter()
        .map(UserId::new)
        .collect::<Result<Vec<_>, _>>()?;

    let vault = Vault {
        id: vault_id,
        kind: VaultKind::Organization,
        name,
        owner_user_id: None,
        folders: Vec::new(),
        members: admin_user_ids
            .iter()
            .map(|admin_user_id| VaultMember {
                user_id: admin_user_id.clone(),
                folder_access: BTreeSet::new(),
            })
            .collect(),
        admins: admin_user_ids,
    };

    Ok(BootstrapOutput {
        vault,
        required_key_grants: Vec::new(),
    })
}

/// Development-only deterministic bootstrap summary used by the smoke server.
pub fn smoke_bootstrap_summary() -> Result<BootstrapSmokeSummary, CoreError> {
    let personal =
        bootstrap_personal_vault("personal-smoke", "Personal Smoke", "npub-smoke-owner")?;
    let organization =
        bootstrap_organization_vault("org-smoke", "Organization Smoke", "npub-smoke-admin")?;

    Ok(BootstrapSmokeSummary {
        personal: BootstrapVaultSummary::from_output(&personal),
        organization: BootstrapVaultSummary::from_output(&organization),
    })
}

/// Development smoke summary for both bootstrap shapes.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapSmokeSummary {
    /// Personal Vault summary.
    pub personal: BootstrapVaultSummary,
    /// Organization Vault summary.
    pub organization: BootstrapVaultSummary,
}

/// Compact bootstrap summary safe to return from smoke endpoints.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapVaultSummary {
    /// Vault kind.
    pub kind: VaultKind,
    /// Folder ids created by bootstrap.
    pub folder_ids: Vec<String>,
    /// Number of current required Folder Key Grants.
    pub required_grants: usize,
    /// Admin count.
    pub admin_count: usize,
    /// Member count.
    pub member_count: usize,
}

impl BootstrapVaultSummary {
    fn from_output(output: &BootstrapOutput) -> Self {
        Self {
            kind: output.vault.kind,
            folder_ids: output
                .vault
                .folders
                .iter()
                .map(|folder| folder.id.to_string())
                .collect(),
            required_grants: output.required_key_grants.len(),
            admin_count: output.vault.admins.len(),
            member_count: output.vault.members.len(),
        }
    }
}

/// Folder Object crypto and signed-record validation errors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CryptoRecordError {
    /// Encrypted envelope is malformed or unsupported.
    InvalidEnvelope { reason: String },
    /// AES-GCM authentication failed for the expected AAD.
    AadMismatch,
    /// Envelope key version does not match the expected context.
    KeyVersionMismatch { expected: u32, actual: u32 },
    /// Signed payload ciphertext hash does not match the submitted envelope.
    CiphertextHashMismatch { expected: String, actual: String },
    /// Nostr event kind, id, signature, content, or tags did not match.
    EventMismatch { reason: String },
    /// Event signer did not match the payload actor.
    SignerMismatch { expected: String, actual: String },
    /// Operation/action is not allowed for this payload type.
    BadOperation { operation: String },
}

impl fmt::Display for CryptoRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnvelope { reason } => write!(f, "invalid envelope: {reason}"),
            Self::AadMismatch => f.write_str("folder object AAD mismatch"),
            Self::KeyVersionMismatch { expected, actual } => {
                write!(
                    f,
                    "folder key version mismatch: expected {expected}, got {actual}"
                )
            }
            Self::CiphertextHashMismatch { expected, actual } => write!(
                f,
                "ciphertext hash mismatch: expected {expected}, got {actual}"
            ),
            Self::EventMismatch { reason } => write!(f, "signed event mismatch: {reason}"),
            Self::SignerMismatch { expected, actual } => {
                write!(f, "signer mismatch: expected {expected}, got {actual}")
            }
            Self::BadOperation { operation } => write!(f, "bad operation: {operation}"),
        }
    }
}

impl Error for CryptoRecordError {}

impl From<CoreError> for CryptoRecordError {
    fn from(error: CoreError) -> Self {
        Self::EventMismatch {
            reason: error.to_string(),
        }
    }
}

/// AES-256-GCM Folder Key.
#[derive(Clone, Eq, PartialEq)]
pub struct FolderKey([u8; 32]);

impl fmt::Debug for FolderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FolderKey([redacted; 32])")
    }
}

impl FolderKey {
    /// Generate a random Folder Key.
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Import raw key bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Import a base64 raw AES-256 key.
    pub fn from_base64(value: &str) -> Result<Self, CryptoRecordError> {
        let bytes =
            BASE64_STANDARD
                .decode(value)
                .map_err(|_| CryptoRecordError::InvalidEnvelope {
                    reason: "folder key is not base64".to_owned(),
                })?;
        let bytes = bytes
            .try_into()
            .map_err(|_| CryptoRecordError::InvalidEnvelope {
                reason: "folder key must be 32 bytes".to_owned(),
            })?;
        Ok(Self(bytes))
    }

    /// Export raw key bytes as base64.
    pub fn to_base64(&self) -> String {
        BASE64_STANDARD.encode(self.0)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(&self.0).expect("FolderKey is exactly 32 bytes")
    }
}

/// Folder Object encryption context used as AES-GCM AAD.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FolderObjectAad {
    /// Vault id.
    pub vault_id: VaultId,
    /// Folder id.
    pub folder_id: FolderId,
    /// Object id.
    pub object_id: ObjectId,
    /// Folder Key version.
    pub key_version: u32,
}

impl FolderObjectAad {
    /// Build the canonical AAD JSON string.
    pub fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"vaultId\":{},\"folderId\":{},\"objectId\":{},\"keyVersion\":{}}}",
            json_string(FOLDER_OBJECT_VERSION),
            json_string(self.vault_id.as_str()),
            json_string(self.folder_id.as_str()),
            json_string(self.object_id.as_str()),
            self.key_version
        )
    }
}

/// `finite-folder-object-v1` encrypted envelope.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EncryptedFolderObjectEnvelope {
    /// Envelope version.
    pub version: String,
    /// Cipher name.
    pub cipher: String,
    /// Folder Key version.
    #[serde(rename = "keyVersion")]
    pub key_version: u32,
    /// Base64 12-byte AES-GCM nonce.
    pub nonce: String,
    /// Base64 AES-GCM ciphertext plus tag.
    pub ciphertext: String,
}

impl EncryptedFolderObjectEnvelope {
    /// Build the exact canonical envelope string used for hashing.
    pub fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"cipher\":{},\"keyVersion\":{},\"nonce\":{},\"ciphertext\":{}}}",
            json_string(&self.version),
            json_string(&self.cipher),
            self.key_version,
            json_string(&self.nonce),
            json_string(&self.ciphertext)
        )
    }

    /// Parse a canonical or ordinary JSON envelope.
    pub fn from_json(value: &str) -> Result<Self, CryptoRecordError> {
        serde_json::from_str(value).map_err(|_| CryptoRecordError::InvalidEnvelope {
            reason: "envelope JSON did not parse".to_owned(),
        })
    }
}

/// Encrypt plaintext bytes into a canonical Folder Object envelope with a fresh nonce.
pub fn encrypt_folder_object(
    key: &FolderKey,
    aad: &FolderObjectAad,
    plaintext: impl AsRef<[u8]>,
) -> Result<EncryptedFolderObjectEnvelope, CryptoRecordError> {
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce);
    encrypt_folder_object_with_nonce(key, aad, nonce, plaintext)
}

/// Encrypt plaintext bytes with a caller-provided nonce for deterministic vectors/tests.
pub fn encrypt_folder_object_with_nonce(
    key: &FolderKey,
    aad: &FolderObjectAad,
    nonce: [u8; 12],
    plaintext: impl AsRef<[u8]>,
) -> Result<EncryptedFolderObjectEnvelope, CryptoRecordError> {
    let aad_json = aad.canonical_json();
    let ciphertext = key
        .cipher()
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_ref(),
                aad: aad_json.as_bytes(),
            },
        )
        .map_err(|_| CryptoRecordError::InvalidEnvelope {
            reason: "encryption failed".to_owned(),
        })?;

    Ok(EncryptedFolderObjectEnvelope {
        version: FOLDER_OBJECT_VERSION.to_owned(),
        cipher: CIPHER_AES_256_GCM.to_owned(),
        key_version: aad.key_version,
        nonce: BASE64_STANDARD.encode(nonce),
        ciphertext: BASE64_STANDARD.encode(ciphertext),
    })
}

/// Open a `finite-folder-object-v1` envelope using expected AAD.
pub fn open_folder_object(
    key: &FolderKey,
    aad: &FolderObjectAad,
    envelope: &EncryptedFolderObjectEnvelope,
) -> Result<Vec<u8>, CryptoRecordError> {
    validate_envelope_header(aad, envelope)?;

    let nonce = BASE64_STANDARD.decode(&envelope.nonce).map_err(|_| {
        CryptoRecordError::InvalidEnvelope {
            reason: "nonce is not base64".to_owned(),
        }
    })?;
    let nonce: [u8; 12] = nonce
        .try_into()
        .map_err(|_| CryptoRecordError::InvalidEnvelope {
            reason: "nonce must be 12 bytes".to_owned(),
        })?;
    let ciphertext = BASE64_STANDARD.decode(&envelope.ciphertext).map_err(|_| {
        CryptoRecordError::InvalidEnvelope {
            reason: "ciphertext is not base64".to_owned(),
        }
    })?;
    let aad_json = aad.canonical_json();

    key.cipher()
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: aad_json.as_bytes(),
            },
        )
        .map_err(|_| CryptoRecordError::AadMismatch)
}

/// SHA-256 hex digest of an exact string.
pub fn sha256_hex(input: impl AsRef<[u8]>) -> String {
    let digest = Sha256::digest(input.as_ref());
    hex_encode(&digest)
}

/// Hash the exact serialized encrypted envelope string.
pub fn ciphertext_hash(envelope_json: &str) -> String {
    sha256_hex(envelope_json.as_bytes())
}

/// Folder Object revision operation.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FolderObjectOperation {
    /// Create a new object.
    Create,
    /// Update an existing object.
    Update,
    /// Move an existing object to a new plaintext path.
    Move,
}

impl FolderObjectOperation {
    /// String representation used in signed payloads and tags.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Move => "move",
        }
    }
}

impl TryFrom<&str> for FolderObjectOperation {
    type Error = CryptoRecordError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            "move" => Ok(Self::Move),
            _ => Err(CryptoRecordError::BadOperation {
                operation: value.to_owned(),
            }),
        }
    }
}

/// Signed Folder Object revision payload.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub struct FolderObjectRevisionPayload {
    /// Payload version.
    pub version: String,
    /// Vault id.
    #[serde(rename = "vaultId")]
    pub vault_id: String,
    /// Folder id.
    #[serde(rename = "folderId")]
    pub folder_id: String,
    /// Object id.
    #[serde(rename = "objectId")]
    pub object_id: String,
    /// Operation.
    pub operation: String,
    /// New revision number.
    pub revision: u64,
    /// Base revision.
    #[serde(rename = "baseRevision")]
    pub base_revision: Option<u64>,
    /// Folder Key version.
    #[serde(rename = "keyVersion")]
    pub key_version: u32,
    /// Cipher.
    pub cipher: String,
    /// Ciphertext hash.
    #[serde(rename = "ciphertextHash")]
    pub ciphertext_hash: String,
    /// Author npub.
    #[serde(rename = "authorNpub")]
    pub author_npub: String,
    /// Creation timestamp.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl FolderObjectRevisionPayload {
    /// Create a signed revision payload.
    pub fn new(input: &RevisionValidation) -> Self {
        Self {
            version: "finite-folder-object-revision-v1".to_owned(),
            vault_id: input.vault_id.to_string(),
            folder_id: input.folder_id.to_string(),
            object_id: input.object_id.as_str().to_owned(),
            operation: input.operation.as_str().to_owned(),
            revision: input.revision,
            base_revision: input.base_revision,
            key_version: input.key_version,
            cipher: CIPHER_AES_256_GCM.to_owned(),
            ciphertext_hash: ciphertext_hash(&input.envelope_json),
            author_npub: input.author_npub.clone(),
            created_at: input.created_at.clone(),
        }
    }

    /// Canonical JSON in spec field order.
    pub fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"vaultId\":{},\"folderId\":{},\"objectId\":{},\"operation\":{},\"revision\":{},\"baseRevision\":{},\"keyVersion\":{},\"cipher\":{},\"ciphertextHash\":{},\"authorNpub\":{},\"createdAt\":{}}}",
            json_string(&self.version),
            json_string(&self.vault_id),
            json_string(&self.folder_id),
            json_string(&self.object_id),
            json_string(&self.operation),
            self.revision,
            json_optional_u64(self.base_revision),
            self.key_version,
            json_string(&self.cipher),
            json_string(&self.ciphertext_hash),
            json_string(&self.author_npub),
            json_string(&self.created_at)
        )
    }
}

/// Expected values for validating a signed revision event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RevisionValidation {
    /// Vault id.
    pub vault_id: VaultId,
    /// Folder id.
    pub folder_id: FolderId,
    /// Object id.
    pub object_id: ObjectId,
    /// Operation.
    pub operation: FolderObjectOperation,
    /// New revision.
    pub revision: u64,
    /// Expected base revision.
    pub base_revision: Option<u64>,
    /// Folder Key version.
    pub key_version: u32,
    /// Exact serialized encrypted envelope string submitted in the request.
    pub envelope_json: String,
    /// Expected actor/author npub.
    pub author_npub: String,
    /// Expected payload timestamp.
    pub created_at: String,
}

/// Validate a signed create/update/move Folder Object revision event.
pub fn validate_revision_event(
    event: &Event,
    expected: &RevisionValidation,
) -> Result<FolderObjectRevisionPayload, CryptoRecordError> {
    validate_event_integrity(event)?;
    validate_revision_envelope(expected)?;
    let payload: FolderObjectRevisionPayload = parse_event_content(event)?;
    FolderObjectOperation::try_from(payload.operation.as_str())?;

    if payload.canonical_json() != event.content {
        return Err(CryptoRecordError::EventMismatch {
            reason: "revision payload is not canonical".to_owned(),
        });
    }

    let expected_payload = FolderObjectRevisionPayload::new(expected);
    if payload != expected_payload {
        if payload.ciphertext_hash != expected_payload.ciphertext_hash {
            return Err(CryptoRecordError::CiphertextHashMismatch {
                expected: expected_payload.ciphertext_hash,
                actual: payload.ciphertext_hash,
            });
        }
        return Err(CryptoRecordError::EventMismatch {
            reason: "revision payload fields differ from expected request".to_owned(),
        });
    }

    validate_signer(event, &payload.author_npub)?;
    require_exact_tags(event, revision_tags(expected))?;
    Ok(payload)
}

fn validate_revision_envelope(expected: &RevisionValidation) -> Result<(), CryptoRecordError> {
    let envelope = EncryptedFolderObjectEnvelope::from_json(&expected.envelope_json)?;
    let aad = FolderObjectAad {
        vault_id: expected.vault_id.clone(),
        folder_id: expected.folder_id.clone(),
        object_id: expected.object_id.clone(),
        key_version: expected.key_version,
    };
    validate_envelope_header(&aad, &envelope)
}

/// Signed Folder Object tombstone payload.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub struct FolderObjectTombstonePayload {
    /// Payload version.
    pub version: String,
    /// Vault id.
    #[serde(rename = "vaultId")]
    pub vault_id: String,
    /// Folder id.
    #[serde(rename = "folderId")]
    pub folder_id: String,
    /// Object id.
    #[serde(rename = "objectId")]
    pub object_id: String,
    /// Operation, always delete.
    pub operation: String,
    /// New revision number.
    pub revision: u64,
    /// Base revision.
    #[serde(rename = "baseRevision")]
    pub base_revision: u64,
    /// Author npub.
    #[serde(rename = "authorNpub")]
    pub author_npub: String,
    /// Deletion timestamp.
    #[serde(rename = "deletedAt")]
    pub deleted_at: String,
}

impl FolderObjectTombstonePayload {
    /// Create a signed tombstone payload.
    pub fn new(input: &TombstoneValidation) -> Self {
        Self {
            version: "finite-folder-object-tombstone-v1".to_owned(),
            vault_id: input.vault_id.to_string(),
            folder_id: input.folder_id.to_string(),
            object_id: input.object_id.as_str().to_owned(),
            operation: "delete".to_owned(),
            revision: input.revision,
            base_revision: input.base_revision,
            author_npub: input.author_npub.clone(),
            deleted_at: input.deleted_at.clone(),
        }
    }

    /// Canonical JSON in spec field order.
    pub fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"vaultId\":{},\"folderId\":{},\"objectId\":{},\"operation\":{},\"revision\":{},\"baseRevision\":{},\"authorNpub\":{},\"deletedAt\":{}}}",
            json_string(&self.version),
            json_string(&self.vault_id),
            json_string(&self.folder_id),
            json_string(&self.object_id),
            json_string(&self.operation),
            self.revision,
            self.base_revision,
            json_string(&self.author_npub),
            json_string(&self.deleted_at)
        )
    }
}

/// Expected values for validating a signed tombstone event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TombstoneValidation {
    /// Vault id.
    pub vault_id: VaultId,
    /// Folder id.
    pub folder_id: FolderId,
    /// Object id.
    pub object_id: ObjectId,
    /// New tombstone revision.
    pub revision: u64,
    /// Previous revision.
    pub base_revision: u64,
    /// Expected actor/author npub.
    pub author_npub: String,
    /// Expected deletion timestamp.
    pub deleted_at: String,
}

/// Validate a signed delete/tombstone event.
pub fn validate_tombstone_event(
    event: &Event,
    expected: &TombstoneValidation,
) -> Result<FolderObjectTombstonePayload, CryptoRecordError> {
    validate_event_integrity(event)?;
    let payload: FolderObjectTombstonePayload = parse_event_content(event)?;

    if payload.operation != "delete" {
        return Err(CryptoRecordError::BadOperation {
            operation: payload.operation,
        });
    }

    if payload.canonical_json() != event.content {
        return Err(CryptoRecordError::EventMismatch {
            reason: "tombstone payload is not canonical".to_owned(),
        });
    }

    let expected_payload = FolderObjectTombstonePayload::new(expected);
    if payload != expected_payload {
        return Err(CryptoRecordError::EventMismatch {
            reason: "tombstone payload fields differ from expected request".to_owned(),
        });
    }

    validate_signer(event, &payload.author_npub)?;
    require_exact_tags(event, tombstone_tags(expected))?;
    Ok(payload)
}

/// Admin access-change action.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AdminAccessAction {
    /// Add member.
    AddMember,
    /// Remove member.
    RemoveMember,
    /// Add admin.
    AddAdmin,
    /// Remove admin.
    RemoveAdmin,
    /// Grant restricted folder access.
    GrantFolderAccess,
    /// Remove restricted folder access.
    RemoveFolderAccess,
    /// Rotate a Folder Key.
    RotateFolderKey,
    /// Change Folder Access mode.
    SetFolderAccessMode,
    /// Permanently delete a Folder subtree.
    DeleteFolder,
}

impl AdminAccessAction {
    /// String representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AddMember => "add-member",
            Self::RemoveMember => "remove-member",
            Self::AddAdmin => "add-admin",
            Self::RemoveAdmin => "remove-admin",
            Self::GrantFolderAccess => "grant-folder-access",
            Self::RemoveFolderAccess => "remove-folder-access",
            Self::RotateFolderKey => "rotate-folder-key",
            Self::SetFolderAccessMode => "set-folder-access-mode",
            Self::DeleteFolder => "delete-folder",
        }
    }
}

impl TryFrom<&str> for AdminAccessAction {
    type Error = CryptoRecordError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "add-member" => Ok(Self::AddMember),
            "remove-member" => Ok(Self::RemoveMember),
            "add-admin" => Ok(Self::AddAdmin),
            "remove-admin" => Ok(Self::RemoveAdmin),
            "grant-folder-access" => Ok(Self::GrantFolderAccess),
            "remove-folder-access" => Ok(Self::RemoveFolderAccess),
            "rotate-folder-key" => Ok(Self::RotateFolderKey),
            "set-folder-access-mode" => Ok(Self::SetFolderAccessMode),
            "delete-folder" => Ok(Self::DeleteFolder),
            _ => Err(CryptoRecordError::BadOperation {
                operation: value.to_owned(),
            }),
        }
    }
}

/// Signed Vault admin access-change payload.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub struct AdminAccessChangePayload {
    /// Payload version.
    pub version: String,
    /// Vault id.
    #[serde(rename = "vaultId")]
    pub vault_id: String,
    /// Change id.
    #[serde(rename = "changeId")]
    pub change_id: String,
    /// Action.
    pub action: String,
    /// Admin npub.
    #[serde(rename = "adminNpub")]
    pub admin_npub: String,
    /// Optional folder id.
    #[serde(rename = "folderId")]
    pub folder_id: Option<String>,
    /// Optional target npub.
    #[serde(rename = "targetNpub")]
    pub target_npub: Option<String>,
    /// Optional key version.
    #[serde(rename = "keyVersion")]
    pub key_version: Option<u32>,
    /// Optional note.
    pub note: Option<String>,
    /// Creation timestamp.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl AdminAccessChangePayload {
    /// Create an access-change payload.
    pub fn new(input: &AdminAccessChangeValidation) -> Self {
        Self {
            version: "finite-vault-admin-access-change-v1".to_owned(),
            vault_id: input.vault_id.to_string(),
            change_id: input.change_id.clone(),
            action: input.action.as_str().to_owned(),
            admin_npub: input.admin_npub.clone(),
            folder_id: input.folder_id.as_ref().map(ToString::to_string),
            target_npub: input.target_npub.clone(),
            key_version: input.key_version,
            note: input.note.clone(),
            created_at: input.created_at.clone(),
        }
    }

    /// Canonical JSON in spec field order, omitting absent optional fields.
    pub fn canonical_json(&self) -> String {
        let mut fields = vec![
            format!("\"version\":{}", json_string(&self.version)),
            format!("\"vaultId\":{}", json_string(&self.vault_id)),
            format!("\"changeId\":{}", json_string(&self.change_id)),
            format!("\"action\":{}", json_string(&self.action)),
            format!("\"adminNpub\":{}", json_string(&self.admin_npub)),
        ];

        if let Some(folder_id) = &self.folder_id {
            fields.push(format!("\"folderId\":{}", json_string(folder_id)));
        }
        if let Some(target_npub) = &self.target_npub {
            fields.push(format!("\"targetNpub\":{}", json_string(target_npub)));
        }
        if let Some(key_version) = self.key_version {
            fields.push(format!("\"keyVersion\":{key_version}"));
        }
        if let Some(note) = &self.note {
            fields.push(format!("\"note\":{}", json_string(note)));
        }
        fields.push(format!("\"createdAt\":{}", json_string(&self.created_at)));

        format!("{{{}}}", fields.join(","))
    }
}

/// Expected values for validating an admin access-change event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AdminAccessChangeValidation {
    /// Vault id.
    pub vault_id: VaultId,
    /// Change id.
    pub change_id: String,
    /// Action.
    pub action: AdminAccessAction,
    /// Admin npub.
    pub admin_npub: String,
    /// Optional folder id.
    pub folder_id: Option<FolderId>,
    /// Optional target npub.
    pub target_npub: Option<String>,
    /// Optional key version.
    pub key_version: Option<u32>,
    /// Optional note.
    pub note: Option<String>,
    /// Expected timestamp.
    pub created_at: String,
}

/// Validate a signed Vault admin access-change event.
pub fn validate_admin_access_change_event(
    event: &Event,
    expected: &AdminAccessChangeValidation,
) -> Result<AdminAccessChangePayload, CryptoRecordError> {
    validate_event_integrity(event)?;
    let payload: AdminAccessChangePayload = parse_event_content(event)?;
    AdminAccessAction::try_from(payload.action.as_str())?;

    if payload.canonical_json() != event.content {
        return Err(CryptoRecordError::EventMismatch {
            reason: "access-change payload is not canonical".to_owned(),
        });
    }

    let expected_payload = AdminAccessChangePayload::new(expected);
    if payload != expected_payload {
        return Err(CryptoRecordError::EventMismatch {
            reason: "access-change payload fields differ from expected request".to_owned(),
        });
    }

    validate_signer(event, &payload.admin_npub)?;
    require_exact_tags(event, admin_access_change_tags(expected)?)?;
    Ok(payload)
}

/// Issue one canonical Vault admin access-change event from its typed validation contract.
pub fn issue_admin_access_change_event(
    admin_keys: &Keys,
    input: &AdminAccessChangeValidation,
    created_at_unix_seconds: u64,
) -> Result<Event, CryptoRecordError> {
    let signer_npub = NostrPublicKey::from_protocol(admin_keys.public_key())
        .to_npub()
        .map_err(|error| CryptoRecordError::EventMismatch {
            reason: error.to_string(),
        })?;
    if signer_npub != input.admin_npub {
        return brain_identity_provider_error(
            "access-change signer does not match the named Vault admin",
        );
    }
    let payload = AdminAccessChangePayload::new(input);
    let tags = admin_access_change_tags(input)?
        .into_iter()
        .map(|parts| {
            Tag::parse(parts).map_err(|error| CryptoRecordError::EventMismatch {
                reason: error.to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    EventBuilder::new(Kind::ApplicationSpecificData, payload.canonical_json())
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(created_at_unix_seconds))
        .finalize(admin_keys)
        .map_err(|error| CryptoRecordError::EventMismatch {
            reason: format!("access-change event could not be signed: {error}"),
        })
}

/// Validate an official Product Client request-signing intent before the user key signs it.
pub fn validate_brain_http_authorization_intent(
    input: &BrainHttpAuthorizationIntent,
    official_brain_origin: &str,
) -> Result<(), CryptoRecordError> {
    let method = input.method.to_ascii_uppercase();
    if !matches!(method.as_str(), "DELETE" | "GET" | "PATCH" | "POST" | "PUT") {
        return brain_identity_provider_error("Brain HTTP authorization method is unsupported");
    }
    let (origin, path) =
        absolute_http_url_parts(&input.url).ok_or_else(|| CryptoRecordError::EventMismatch {
            reason: "Brain HTTP authorization URL is invalid".to_owned(),
        })?;
    if origin != official_brain_origin.trim_end_matches('/') {
        return brain_identity_provider_error(
            "Brain HTTP authorization requires the official Brain origin",
        );
    }
    if path != "/_admin" && !path.starts_with("/_admin/") {
        return brain_identity_provider_error(
            "Brain HTTP authorization requires a protected Brain route",
        );
    }
    let template = &input.event_template;
    if template.kind != 27_235 || !template.content.is_empty() {
        return brain_identity_provider_error(
            "Brain HTTP authorization requires an empty kind 27235 event",
        );
    }
    let allowed_tags = BTreeSet::from(["method", "nonce", "payload", "u"]);
    if template.tags.iter().any(|tag| {
        tag.first()
            .is_none_or(|name| !allowed_tags.contains(name.as_str()))
    }) {
        return brain_identity_provider_error(
            "Brain HTTP authorization contains an unsupported tag",
        );
    }
    if single_template_tag(template, "u")? != input.url {
        return brain_identity_provider_error(
            "Brain HTTP authorization URL tag does not match its request",
        );
    }
    if single_template_tag(template, "method")? != method {
        return brain_identity_provider_error(
            "Brain HTTP authorization method tag does not match its request",
        );
    }
    let nonce = single_template_tag(template, "nonce")?;
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return brain_identity_provider_error("Brain HTTP authorization nonce tag is invalid");
    }
    let payload_tags = template_tag_values(template, "payload");
    if input.body_text.is_empty() {
        if !payload_tags.is_empty() {
            return brain_identity_provider_error(
                "Brain HTTP authorization without a body cannot include a payload tag",
            );
        }
    } else if payload_tags.len() != 1 || payload_tags[0] != sha256_hex(input.body_text.as_bytes()) {
        return brain_identity_provider_error(
            "Brain HTTP authorization payload tag does not match its request body",
        );
    }
    Ok(())
}

/// Validate one named Brain application-event intent before signing it.
pub fn validate_brain_event_authorization_intent(
    input: &BrainEventAuthorizationIntent,
    signer_npub: &str,
) -> Result<(), CryptoRecordError> {
    let template = &input.event_template;
    if template.kind != APP_SPECIFIC_KIND || template.content.is_empty() {
        return brain_identity_provider_error(
            "Brain event kind or content does not match its named intent",
        );
    }
    match input.intent.as_str() {
        "folder-object-revision" => {
            let payload: FolderObjectRevisionPayload = serde_json::from_str(&template.content)
                .map_err(|_| CryptoRecordError::EventMismatch {
                    reason: "Brain revision payload did not parse".to_owned(),
                })?;
            let operation = FolderObjectOperation::try_from(payload.operation.as_str())?;
            let vault_id = VaultId::new(payload.vault_id.clone())?;
            let folder_id = FolderId::new(payload.folder_id.clone())?;
            let object_id = ObjectId::new(payload.object_id.clone())?;
            if payload.version != "finite-folder-object-revision-v1"
                || payload.cipher != CIPHER_AES_256_GCM
                || payload.revision == 0
                || payload.key_version == 0
                || payload.ciphertext_hash.len() != 64
                || !payload
                    .ciphertext_hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                || payload.author_npub != signer_npub
                || payload.canonical_json() != template.content
            {
                return brain_identity_provider_error("Brain revision payload is invalid");
            }
            require_exact_template_tags(
                template,
                vec![
                    vec![
                        "d".to_owned(),
                        format!(
                            "finite-folder-object-revision:{vault_id}:{folder_id}:{}:{}",
                            object_id.as_str(),
                            payload.revision
                        ),
                    ],
                    vec!["vault".to_owned(), vault_id.to_string()],
                    vec!["folder".to_owned(), folder_id.to_string()],
                    vec!["object".to_owned(), object_id.as_str().to_owned()],
                    vec!["operation".to_owned(), operation.as_str().to_owned()],
                    vec!["keyVersion".to_owned(), payload.key_version.to_string()],
                ],
            )?;
        }
        "folder-object-tombstone" => {
            let payload: FolderObjectTombstonePayload = serde_json::from_str(&template.content)
                .map_err(|_| CryptoRecordError::EventMismatch {
                    reason: "Brain tombstone payload did not parse".to_owned(),
                })?;
            let vault_id = VaultId::new(payload.vault_id.clone())?;
            let folder_id = FolderId::new(payload.folder_id.clone())?;
            let object_id = ObjectId::new(payload.object_id.clone())?;
            if payload.version != "finite-folder-object-tombstone-v1"
                || payload.operation != "delete"
                || payload.revision == 0
                || payload.author_npub != signer_npub
                || payload.canonical_json() != template.content
            {
                return brain_identity_provider_error("Brain tombstone payload is invalid");
            }
            require_exact_template_tags(
                template,
                vec![
                    vec![
                        "d".to_owned(),
                        format!(
                            "finite-folder-object-tombstone:{vault_id}:{folder_id}:{}:{}",
                            object_id.as_str(),
                            payload.revision
                        ),
                    ],
                    vec!["vault".to_owned(), vault_id.to_string()],
                    vec!["folder".to_owned(), folder_id.to_string()],
                    vec!["object".to_owned(), object_id.as_str().to_owned()],
                    vec!["operation".to_owned(), "delete".to_owned()],
                ],
            )?;
        }
        "vault-access-change" => {
            let payload: AdminAccessChangePayload = serde_json::from_str(&template.content)
                .map_err(|_| CryptoRecordError::EventMismatch {
                    reason: "Brain access-change payload did not parse".to_owned(),
                })?;
            let action = AdminAccessAction::try_from(payload.action.as_str())?;
            let vault_id = VaultId::new(payload.vault_id.clone())?;
            let folder_id = payload
                .folder_id
                .as_ref()
                .map(|value| FolderId::new(value.clone()))
                .transpose()?;
            if let Some(target_npub) = &payload.target_npub {
                NostrPublicKey::parse(target_npub).map_err(|error| {
                    CryptoRecordError::EventMismatch {
                        reason: error.to_string(),
                    }
                })?;
            }
            if payload.version != "finite-vault-admin-access-change-v1"
                || payload.admin_npub != signer_npub
                || payload.canonical_json() != template.content
            {
                return brain_identity_provider_error("Brain access-change payload is invalid");
            }
            let expected = AdminAccessChangeValidation {
                vault_id,
                change_id: payload.change_id,
                action,
                admin_npub: payload.admin_npub,
                folder_id,
                target_npub: payload.target_npub,
                key_version: payload.key_version,
                note: payload.note,
                created_at: payload.created_at,
            };
            require_exact_template_tags(template, admin_access_change_tags(&expected)?)?;
        }
        "vault-invite-authorization" => {
            let payload: BrainEmailInviteAuthorizationPayload =
                serde_json::from_str(&template.content).map_err(|_| {
                    CryptoRecordError::EventMismatch {
                        reason: "Brain email-invite authorization payload did not parse".to_owned(),
                    }
                })?;
            if payload.folders.len() > MAX_BRAIN_INVITE_BOOTSTRAP_FOLDERS {
                return brain_identity_provider_error(
                    "Brain email-invite Folder scope exceeds the supported limit",
                );
            }
            let vault_id = VaultId::new(payload.vault_id.clone())?;
            NostrPublicKey::parse(&payload.invite_unwrap_npub).map_err(|error| {
                CryptoRecordError::EventMismatch {
                    reason: error.to_string(),
                }
            })?;
            for folder in &payload.folders {
                FolderId::new(folder.folder_id.clone())?;
                if folder.key_version == 0 {
                    return brain_identity_provider_error(
                        "Brain email-invite Folder Key version is invalid",
                    );
                }
            }
            if payload.version != "finite-email-invite-bootstrap-authorization-v1"
                || payload.invited_email.trim().is_empty()
                || payload.bootstrap_payload_hash.len() != "sha256:".len() + 64
                || !payload.bootstrap_payload_hash.starts_with("sha256:")
                || !payload.bootstrap_payload_hash["sha256:".len()..]
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                || payload.canonical_json() != template.content
            {
                return brain_identity_provider_error(
                    "Brain email-invite authorization payload is invalid",
                );
            }
            require_exact_template_tags(
                template,
                vec![
                    vec![
                        "d".to_owned(),
                        format!(
                            "finite-email-invite-bootstrap-authorization:{vault_id}:{}",
                            payload.invited_email
                        ),
                    ],
                    vec!["vault".to_owned(), vault_id.to_string()],
                    vec!["email".to_owned(), payload.invited_email],
                ],
            )?;
        }
        _ => return brain_identity_provider_error("unsupported Brain identity intent"),
    }
    Ok(())
}

fn require_exact_template_tags(
    template: &BrainEventTemplate,
    expected: Vec<Vec<String>>,
) -> Result<(), CryptoRecordError> {
    if template.tags != expected {
        return brain_identity_provider_error("Brain event tags differ from its typed payload");
    }
    Ok(())
}

/// Validate and parse the peer key for one bounded Brain grant operation.
pub fn validate_brain_grant_intent(
    input: &BrainGrantIntent,
) -> Result<NostrPublicKey, CryptoRecordError> {
    VaultId::new(input.vault_id.clone())?;
    let recipient = NostrPublicKey::parse(&input.recipient_npub).map_err(|error| {
        CryptoRecordError::EventMismatch {
            reason: error.to_string(),
        }
    })?;
    match input.purpose.as_str() {
        "folder-key-grant" => {
            let folder_id =
                input
                    .folder_id
                    .as_ref()
                    .ok_or_else(|| CryptoRecordError::EventMismatch {
                        reason: "Brain Folder Key Grant requires a Folder id".to_owned(),
                    })?;
            FolderId::new(folder_id.clone())?;
            if input.key_version.is_none_or(|version| version == 0) {
                return brain_identity_provider_error(
                    "Brain Folder Key Grant requires a positive key version",
                );
            }
        }
        "vault-invite-bootstrap" => {
            if input.folder_id.is_some() || input.key_version.is_some() {
                return brain_identity_provider_error(
                    "Brain invite bootstrap cannot name one Folder Key",
                );
            }
        }
        _ => return brain_identity_provider_error("unsupported Brain grant purpose"),
    }
    Ok(recipient)
}

/// Sign an already validated event template without exposing a general signer API.
pub fn sign_brain_event_template(
    keys: &Keys,
    template: &BrainEventTemplate,
) -> Result<Event, CryptoRecordError> {
    let tags = template
        .tags
        .iter()
        .cloned()
        .map(|parts| {
            Tag::parse(parts).map_err(|error| CryptoRecordError::EventMismatch {
                reason: format!("Brain event tag is invalid: {error}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    EventBuilder::new(Kind::from(template.kind), template.content.clone())
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(template.created_at))
        .finalize(keys)
        .map_err(|error| CryptoRecordError::EventMismatch {
            reason: format!("Brain event could not be signed: {error}"),
        })
}

fn brain_identity_provider_error<T>(reason: &str) -> Result<T, CryptoRecordError> {
    Err(CryptoRecordError::EventMismatch {
        reason: reason.to_owned(),
    })
}

fn template_tag_values<'a>(template: &'a BrainEventTemplate, name: &str) -> Vec<&'a str> {
    template
        .tags
        .iter()
        .filter_map(|tag| match tag.as_slice() {
            [tag_name, value, ..] if tag_name == name => Some(value.as_str()),
            _ => None,
        })
        .collect()
}

fn single_template_tag<'a>(
    template: &'a BrainEventTemplate,
    name: &str,
) -> Result<&'a str, CryptoRecordError> {
    let values = template_tag_values(template, name);
    if values.len() != 1 {
        return brain_identity_provider_error(&format!(
            "Brain event requires exactly one {name} tag"
        ));
    }
    Ok(values[0])
}

fn absolute_http_url_parts(value: &str) -> Option<(&str, &str)> {
    let scheme_end = value.find("://")?;
    if !matches!(&value[..scheme_end], "http" | "https") {
        return None;
    }
    let authority_start = scheme_end + 3;
    let path_start = value[authority_start..]
        .find('/')
        .map_or(value.len(), |index| authority_start + index);
    if path_start == authority_start {
        return None;
    }
    let origin = &value[..path_start];
    let path_and_suffix = if path_start == value.len() {
        "/"
    } else {
        &value[path_start..]
    };
    let path_end = path_and_suffix
        .find(['?', '#'])
        .unwrap_or(path_and_suffix.len());
    Some((origin, &path_and_suffix[..path_end]))
}

/// One client-generated encrypted Folder Key Grant ready for the Brain HTTP contract.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedFolderKeyGrant {
    pub id: String,
    pub key_version: u32,
    pub recipient_npub: String,
    pub wrapped_event_json: String,
    pub created_at: String,
}

/// Decrypted, fully validated Folder Key Grant payload returned to a trusted client.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FolderKeyGrantPayload {
    pub version: String,
    pub vault_id: String,
    pub folder_id: String,
    pub key_version: u32,
    pub folder_key: String,
    pub issuer_npub: String,
    pub recipient_npub: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrainInviteBootstrapPayload {
    version: String,
    vault_id: String,
    invited_email: String,
    invite_unwrap_npub: String,
    folders: Vec<BrainEmailInviteAuthorizationFolder>,
    grants: Vec<BrainInviteBootstrapGrant>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrainInviteBootstrapGrant {
    folder_id: String,
    grant: IssuedFolderKeyGrant,
}

impl BrainInviteBootstrapPayload {
    fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"vaultId\":{},\"invitedEmail\":{},\"inviteUnwrapNpub\":{},\"folders\":{},\"grants\":{}}}",
            json_string(&self.version),
            json_string(&self.vault_id),
            json_string(&self.invited_email),
            json_string(&self.invite_unwrap_npub),
            serde_json::to_string(&self.folders)
                .expect("serializing invite Folder scope cannot fail"),
            serde_json::to_string(&self.grants).expect("serializing invite grants cannot fail"),
        )
    }
}

impl FolderKeyGrantPayload {
    /// Canonical JSON signed inside the NIP-59 rumor.
    pub fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"vaultId\":{},\"folderId\":{},\"keyVersion\":{},\"folderKey\":{},\"issuerNpub\":{},\"recipientNpub\":{},\"createdAt\":{}}}",
            json_string(&self.version),
            json_string(&self.vault_id),
            json_string(&self.folder_id),
            self.key_version,
            json_string(&self.folder_key),
            json_string(&self.issuer_npub),
            json_string(&self.recipient_npub),
            json_string(&self.created_at),
        )
    }
}

/// Open one complete NIP-59 Folder Key Grant bound to the exact requested resource and recipient.
pub fn open_folder_key_grant(
    recipient_keys: &Keys,
    intent: &BrainGrantIntent,
    wrapped_event_json: &str,
) -> Result<FolderKeyGrantPayload, CryptoRecordError> {
    if intent.purpose != "folder-key-grant" {
        return brain_identity_provider_error("only Folder Key Grants can be opened");
    }
    let recipient = validate_brain_grant_intent(intent)?;
    if recipient != NostrPublicKey::from_protocol(recipient_keys.public_key()) {
        return brain_identity_provider_error(
            "Folder Key Grant recipient does not match the hosted member key",
        );
    }
    let wrapped: Event =
        serde_json::from_str(wrapped_event_json).map_err(|_| CryptoRecordError::EventMismatch {
            reason: "Folder Key Grant wrapper did not parse".to_owned(),
        })?;
    let opened = open_gift_wrap(
        recipient_keys,
        &wrapped,
        &GiftWrapValidation::new(recipient),
    )
    .map_err(|error| CryptoRecordError::EventMismatch {
        reason: format!("Folder Key Grant wrapper is invalid: {error}"),
    })?;
    if opened.rumor.kind != Kind::ApplicationSpecificData {
        return brain_identity_provider_error("Folder Key Grant rumor kind is invalid");
    }
    let payload: FolderKeyGrantPayload =
        serde_json::from_str(&opened.rumor.content).map_err(|_| {
            CryptoRecordError::EventMismatch {
                reason: "Folder Key Grant payload did not parse".to_owned(),
            }
        })?;
    let expected_folder_id = intent.folder_id.as_deref().expect("validated Folder id");
    let expected_key_version = intent.key_version.expect("validated key version");
    let issuer_npub =
        opened
            .sender
            .to_npub()
            .map_err(|error| CryptoRecordError::EventMismatch {
                reason: error.to_string(),
            })?;
    if payload.version != "finite-folder-key-grant-v1"
        || payload.vault_id != intent.vault_id
        || payload.folder_id != expected_folder_id
        || payload.key_version != expected_key_version
        || payload.recipient_npub != intent.recipient_npub
        || payload.issuer_npub != issuer_npub
        || payload.canonical_json() != opened.rumor.content
    {
        return brain_identity_provider_error(
            "Folder Key Grant payload does not match its requested resource",
        );
    }
    FolderKey::from_base64(&payload.folder_key)?;
    let actual_tags = opened
        .rumor
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();
    let expected_tags = vec![
        vec![
            "d".to_owned(),
            format!(
                "finite-folder-key-grant:{}:{expected_folder_id}:{expected_key_version}",
                intent.vault_id
            ),
        ],
        vec!["vault".to_owned(), intent.vault_id.clone()],
        vec!["folder".to_owned(), expected_folder_id.to_owned()],
        vec!["keyVersion".to_owned(), expected_key_version.to_string()],
    ];
    if actual_tags != expected_tags {
        return brain_identity_provider_error("Folder Key Grant tags differ from its payload");
    }
    Ok(payload)
}

/// Validate and wrap one complete Email Invite Bootstrap payload for its one-use unwrap key.
pub fn wrap_brain_invite_bootstrap(
    issuer_keys: &Keys,
    intent: &BrainGrantIntent,
    plaintext: &str,
    created_at_unix_seconds: u64,
) -> Result<Event, CryptoRecordError> {
    if intent.purpose != "vault-invite-bootstrap" {
        return brain_identity_provider_error("only an Email Invite Bootstrap can be wrapped");
    }
    let recipient = validate_brain_grant_intent(intent)?;
    let payload: BrainInviteBootstrapPayload =
        serde_json::from_str(plaintext).map_err(|_| CryptoRecordError::EventMismatch {
            reason: "Email Invite Bootstrap payload did not parse".to_owned(),
        })?;
    if payload.folders.len() > MAX_BRAIN_INVITE_BOOTSTRAP_FOLDERS
        || payload.grants.len() > MAX_BRAIN_INVITE_BOOTSTRAP_FOLDERS
    {
        return brain_identity_provider_error(
            "Email Invite Bootstrap Folder scope exceeds the supported limit",
        );
    }
    if payload.version != "finite-email-invite-bootstrap-payload-v1"
        || payload.vault_id != intent.vault_id
        || payload.invite_unwrap_npub != intent.recipient_npub
        || payload.invited_email.trim().is_empty()
        || payload.folders.len() != payload.grants.len()
        || payload.canonical_json() != plaintext
    {
        return brain_identity_provider_error(
            "Email Invite Bootstrap payload does not match its requested resource",
        );
    }
    let mut grants_by_folder = BTreeMap::new();
    for entry in &payload.grants {
        let folder_id = FolderId::new(entry.folder_id.clone())?;
        if grants_by_folder
            .insert(folder_id.to_string(), entry)
            .is_some()
        {
            return brain_identity_provider_error(
                "Email Invite Bootstrap contains duplicate Folder Key Grants",
            );
        }
    }
    let mut seen_folders = BTreeSet::new();
    for folder in &payload.folders {
        let folder_id = FolderId::new(folder.folder_id.clone())?;
        if folder.key_version == 0 || !seen_folders.insert(folder_id.to_string()) {
            return brain_identity_provider_error("Email Invite Bootstrap Folder scope is invalid");
        }
        let entry = grants_by_folder.get(folder_id.as_str()).ok_or_else(|| {
            CryptoRecordError::EventMismatch {
                reason: "Email Invite Bootstrap is missing a scoped Folder Key Grant".to_owned(),
            }
        })?;
        if entry.grant.id.is_empty()
            || entry.grant.key_version != folder.key_version
            || entry.grant.recipient_npub != intent.recipient_npub
        {
            return brain_identity_provider_error(
                "Email Invite Bootstrap Folder Key Grant metadata is invalid",
            );
        }
        let wrapped: Event =
            serde_json::from_str(&entry.grant.wrapped_event_json).map_err(|_| {
                CryptoRecordError::EventMismatch {
                    reason: "Email Invite Bootstrap Folder Key Grant did not parse".to_owned(),
                }
            })?;
        validate_gift_wrap(&wrapped, recipient).map_err(|error| {
            CryptoRecordError::EventMismatch {
                reason: format!("Email Invite Bootstrap Folder Key Grant is invalid: {error}"),
            }
        })?;
    }
    let issuer = NostrPublicKey::from_protocol(issuer_keys.public_key());
    let tags = vec![
        Tag::parse(vec![
            "d".to_owned(),
            format!("finite-email-invite-bootstrap:{}", intent.vault_id),
        ]),
        Tag::parse(vec!["vault".to_owned(), intent.vault_id.clone()]),
    ]
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|error| CryptoRecordError::EventMismatch {
        reason: error.to_string(),
    })?;
    let rumor = build_rumor(
        issuer,
        Kind::ApplicationSpecificData,
        tags,
        plaintext,
        created_at_unix_seconds,
    );
    wrap_rumor(issuer_keys, recipient, rumor).map_err(|error| CryptoRecordError::EventMismatch {
        reason: error.to_string(),
    })
}

/// Wrap a Folder Key for one recipient without exposing the key in the returned contract.
#[allow(clippy::too_many_arguments)]
pub fn issue_folder_key_grant(
    issuer_keys: &Keys,
    grant_id: impl Into<String>,
    vault_id: &VaultId,
    folder_id: &FolderId,
    key_version: u32,
    recipient_npub: impl Into<String>,
    folder_key: &FolderKey,
    created_at: impl Into<String>,
    created_at_unix_seconds: u64,
) -> Result<IssuedFolderKeyGrant, CryptoRecordError> {
    let issuer_npub = NostrPublicKey::from_protocol(issuer_keys.public_key())
        .to_npub()
        .map_err(|error| CryptoRecordError::EventMismatch {
            reason: error.to_string(),
        })?;
    let recipient_npub = recipient_npub.into();
    let recipient = NostrPublicKey::parse(&recipient_npub).map_err(|error| {
        CryptoRecordError::EventMismatch {
            reason: error.to_string(),
        }
    })?;
    let created_at = created_at.into();
    let content = FolderKeyGrantPayload {
        version: "finite-folder-key-grant-v1".to_owned(),
        vault_id: vault_id.to_string(),
        folder_id: folder_id.to_string(),
        key_version,
        folder_key: folder_key.to_base64(),
        issuer_npub,
        recipient_npub: recipient_npub.clone(),
        created_at: created_at.clone(),
    }
    .canonical_json();
    let tags = vec![
        vec![
            "d".to_owned(),
            format!("finite-folder-key-grant:{vault_id}:{folder_id}:{key_version}"),
        ],
        vec!["vault".to_owned(), vault_id.to_string()],
        vec!["folder".to_owned(), folder_id.to_string()],
        vec!["keyVersion".to_owned(), key_version.to_string()],
    ]
    .into_iter()
    .map(|parts| {
        Tag::parse(parts).map_err(|error| CryptoRecordError::EventMismatch {
            reason: error.to_string(),
        })
    })
    .collect::<Result<Vec<_>, _>>()?;
    let rumor = build_rumor(
        NostrPublicKey::from_protocol(issuer_keys.public_key()),
        Kind::ApplicationSpecificData,
        tags,
        content,
        created_at_unix_seconds,
    );
    let wrapped = wrap_rumor(issuer_keys, recipient, rumor).map_err(|error| {
        CryptoRecordError::EventMismatch {
            reason: error.to_string(),
        }
    })?;
    Ok(IssuedFolderKeyGrant {
        id: grant_id.into(),
        key_version,
        recipient_npub,
        wrapped_event_json: wrapped.as_json(),
        created_at,
    })
}

fn validate_envelope_header(
    aad: &FolderObjectAad,
    envelope: &EncryptedFolderObjectEnvelope,
) -> Result<(), CryptoRecordError> {
    if envelope.version != FOLDER_OBJECT_VERSION {
        return Err(CryptoRecordError::InvalidEnvelope {
            reason: "unsupported version".to_owned(),
        });
    }
    if envelope.cipher != CIPHER_AES_256_GCM {
        return Err(CryptoRecordError::InvalidEnvelope {
            reason: "unsupported cipher".to_owned(),
        });
    }
    if envelope.key_version != aad.key_version {
        return Err(CryptoRecordError::KeyVersionMismatch {
            expected: aad.key_version,
            actual: envelope.key_version,
        });
    }
    Ok(())
}

fn validate_event_integrity(event: &Event) -> Result<(), CryptoRecordError> {
    if event.kind != Kind::ApplicationSpecificData {
        return Err(CryptoRecordError::EventMismatch {
            reason: format!(
                "expected kind {APP_SPECIFIC_KIND}, got {}",
                event.kind.as_u16()
            ),
        });
    }
    verify_event_integrity(event).map_err(|error| CryptoRecordError::EventMismatch {
        reason: error.to_string(),
    })
}

fn validate_signer(event: &Event, expected_npub: &str) -> Result<(), CryptoRecordError> {
    let actual = NostrPublicKey::from_protocol(event.pubkey)
        .to_npub()
        .map_err(|error| CryptoRecordError::EventMismatch {
            reason: error.to_string(),
        })?;

    if actual != expected_npub {
        return Err(CryptoRecordError::SignerMismatch {
            expected: expected_npub.to_owned(),
            actual,
        });
    }

    Ok(())
}

fn parse_event_content<T>(event: &Event) -> Result<T, CryptoRecordError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(&event.content).map_err(|_| CryptoRecordError::EventMismatch {
        reason: "payload JSON did not parse".to_owned(),
    })
}

fn require_exact_tags(event: &Event, expected: Vec<Vec<String>>) -> Result<(), CryptoRecordError> {
    let actual = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();

    if actual != expected {
        return Err(CryptoRecordError::EventMismatch {
            reason: "event tags differ from payload".to_owned(),
        });
    }

    Ok(())
}

fn revision_tags(input: &RevisionValidation) -> Vec<Vec<String>> {
    vec![
        vec![
            "d".to_owned(),
            format!(
                "finite-folder-object-revision:{}:{}:{}:{}",
                input.vault_id,
                input.folder_id,
                input.object_id.as_str(),
                input.revision
            ),
        ],
        vec!["vault".to_owned(), input.vault_id.to_string()],
        vec!["folder".to_owned(), input.folder_id.to_string()],
        vec!["object".to_owned(), input.object_id.as_str().to_owned()],
        vec!["operation".to_owned(), input.operation.as_str().to_owned()],
        vec!["keyVersion".to_owned(), input.key_version.to_string()],
    ]
}

fn tombstone_tags(input: &TombstoneValidation) -> Vec<Vec<String>> {
    vec![
        vec![
            "d".to_owned(),
            format!(
                "finite-folder-object-tombstone:{}:{}:{}:{}",
                input.vault_id,
                input.folder_id,
                input.object_id.as_str(),
                input.revision
            ),
        ],
        vec!["vault".to_owned(), input.vault_id.to_string()],
        vec!["folder".to_owned(), input.folder_id.to_string()],
        vec!["object".to_owned(), input.object_id.as_str().to_owned()],
        vec!["operation".to_owned(), "delete".to_owned()],
    ]
}

fn admin_access_change_tags(
    input: &AdminAccessChangeValidation,
) -> Result<Vec<Vec<String>>, CryptoRecordError> {
    let mut tags = vec![
        vec![
            "d".to_owned(),
            format!(
                "finite-vault-admin-access-change:{}:{}",
                input.vault_id, input.change_id
            ),
        ],
        vec!["vault".to_owned(), input.vault_id.to_string()],
        vec!["action".to_owned(), input.action.as_str().to_owned()],
    ];

    if let Some(folder_id) = &input.folder_id {
        tags.push(vec!["folder".to_owned(), folder_id.to_string()]);
    }
    if let Some(target_npub) = &input.target_npub {
        let target_hex = NostrPublicKey::parse(target_npub)
            .map_err(|error| CryptoRecordError::EventMismatch {
                reason: error.to_string(),
            })?
            .to_hex();
        tags.push(vec!["p".to_owned(), target_hex]);
    }
    if let Some(key_version) = input.key_version {
        tags.push(vec!["keyVersion".to_owned(), key_version.to_string()]);
    }

    Ok(tags)
}

fn json_optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing string cannot fail")
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
fn root_folder(
    id: &str,
    name: &str,
    role: FolderRole,
    access: FolderAccessMode,
) -> Result<Folder, CoreError> {
    if RESERVED_TOP_LEVEL_NAMES.contains(&name) {
        return Err(CoreError::InvalidName {
            field: "folder_name",
            value: name.to_owned(),
        });
    }

    Ok(Folder {
        id: FolderId::new(id)?,
        name: DisplayName::new("folder_name", name)?,
        role,
        access,
        parent_folder_id: None,
        path: SafeRelativePath::new("folder_path", name)?,
        current_key_version: 1,
        shared_folder_source: false,
    })
}

fn validate_stable_id(
    field: &'static str,
    value: String,
    min_len: usize,
    max_len: usize,
) -> Result<String, CoreError> {
    let normalized = normalize_nfc(&value);
    let valid_len = (min_len..=max_len).contains(&normalized.len());
    let valid_chars = normalized
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');

    if !valid_len || !valid_chars {
        return Err(CoreError::InvalidId { field, value });
    }

    Ok(normalized)
}

fn normalize_nfc(value: &str) -> String {
    value.nfc().collect::<String>()
}

fn contains_nul_or_control(value: &str) -> bool {
    value.chars().any(|c| c == '\0' || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::event::FinalizeEvent;
    use nostr::{EventBuilder, Keys, Tag, Timestamp};

    #[test]
    fn exposes_core_crate_name() {
        assert_eq!(crate_name(), "finite-brain-core");
    }

    #[test]
    fn bootstraps_personal_vault() {
        let output = bootstrap_personal_vault("personal", "Austin", "npub-owner").unwrap();

        assert_eq!(output.vault.kind, VaultKind::Personal);
        assert_eq!(
            output.vault.owner_user_id,
            Some(UserId::new("npub-owner").unwrap())
        );
        assert!(output.vault.members.is_empty());
        assert!(output.vault.admins.is_empty());
        assert!(output.vault.folders.is_empty());
        assert!(output.required_key_grants.is_empty());
    }

    #[test]
    fn bootstraps_organization_vault() {
        let output = bootstrap_organization_vault("org", "Finite", "npub-admin").unwrap();

        assert_eq!(output.vault.kind, VaultKind::Organization);
        assert_eq!(output.vault.owner_user_id, None);
        assert_eq!(
            output.vault.admins,
            vec![UserId::new("npub-admin").unwrap()]
        );
        assert_eq!(output.vault.members.len(), 1);
        assert_eq!(
            output.vault.members[0].user_id,
            UserId::new("npub-admin").unwrap()
        );
        assert!(output.vault.folders.is_empty());
        assert!(output.required_key_grants.is_empty());
    }

    #[test]
    fn personal_folder_key_recipients_always_include_owner_and_current_agent() {
        let owner = UserId::new("npub-owner").unwrap();
        let agent = UserId::new("npub-agent").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let member = UserId::new("npub-member").unwrap();
        let explicit = UserId::new("npub-explicit").unwrap();

        for access in [
            FolderAccessMode::Owner,
            FolderAccessMode::AdminOnly,
            FolderAccessMode::AllMembers,
            FolderAccessMode::Restricted,
        ] {
            let recipients = required_folder_key_recipients(FolderKeyRecipientPolicy {
                vault_kind: VaultKind::Personal,
                folder_access: access,
                owner_user_id: Some(&owner),
                admins: std::slice::from_ref(&admin),
                members: std::slice::from_ref(&member),
                explicit_access_user_ids: &BTreeSet::from([explicit.clone()]),
                personal_agent_npub: Some(&agent),
            })
            .unwrap();

            assert!(recipients.contains(&owner), "owner missing for {access:?}");
            assert!(recipients.contains(&agent), "agent missing for {access:?}");
        }
    }

    #[test]
    fn vacant_personal_agent_role_grants_every_mode_to_owner() {
        let owner = UserId::new("npub-owner").unwrap();
        for access in [
            FolderAccessMode::Owner,
            FolderAccessMode::AdminOnly,
            FolderAccessMode::AllMembers,
            FolderAccessMode::Restricted,
        ] {
            assert_eq!(
                required_folder_key_recipients(FolderKeyRecipientPolicy {
                    vault_kind: VaultKind::Personal,
                    folder_access: access,
                    owner_user_id: Some(&owner),
                    admins: &[],
                    members: &[],
                    explicit_access_user_ids: &BTreeSet::new(),
                    personal_agent_npub: None,
                })
                .unwrap(),
                BTreeSet::from([owner.clone()])
            );
        }
    }

    #[test]
    fn organization_folder_key_recipients_preserve_access_modes() {
        let admin = UserId::new("npub-admin").unwrap();
        let member = UserId::new("npub-member").unwrap();
        let explicit = UserId::new("npub-explicit").unwrap();
        let explicit_users = BTreeSet::from([explicit.clone()]);
        let policy = |folder_access| FolderKeyRecipientPolicy {
            vault_kind: VaultKind::Organization,
            folder_access,
            owner_user_id: None,
            admins: std::slice::from_ref(&admin),
            members: std::slice::from_ref(&member),
            explicit_access_user_ids: &explicit_users,
            personal_agent_npub: None,
        };

        assert_eq!(
            required_folder_key_recipients(policy(FolderAccessMode::AdminOnly)).unwrap(),
            BTreeSet::from([admin.clone()])
        );
        assert_eq!(
            required_folder_key_recipients(policy(FolderAccessMode::AllMembers)).unwrap(),
            BTreeSet::from([admin.clone(), member.clone()])
        );
        assert_eq!(
            required_folder_key_recipients(policy(FolderAccessMode::Restricted)).unwrap(),
            BTreeSet::from([admin, explicit])
        );
    }

    #[test]
    fn bounds_personal_agent_rotation_fanout_by_folder_and_aggregate_work() {
        validate_folder_rotation_fanout(
            FolderRotationOperation::PersonalAgent,
            [FolderRotationFanout {
                grants: MAX_FOLDER_ROTATION_GRANTS,
                reencrypted_records: MAX_FOLDER_ROTATION_RECORDS,
            }],
        )
        .unwrap();

        assert!(matches!(
            validate_folder_rotation_fanout(
                FolderRotationOperation::PersonalAgent,
                (0..=MAX_PERSONAL_AGENT_ROTATION_FOLDERS).map(|_| FolderRotationFanout {
                    grants: 0,
                    reencrypted_records: 0,
                }),
            ),
            Err(CoreError::RotationFanoutLimitExceeded {
                resource: "Folder rotations",
                ..
            })
        ));
        assert!(matches!(
            validate_folder_rotation_fanout(
                FolderRotationOperation::PersonalAgent,
                [FolderRotationFanout {
                    grants: MAX_FOLDER_ROTATION_GRANTS + 1,
                    reencrypted_records: 0,
                }],
            ),
            Err(CoreError::RotationFanoutLimitExceeded {
                resource: "grants per Folder rotation",
                ..
            })
        ));
        assert!(matches!(
            validate_folder_rotation_fanout(
                FolderRotationOperation::PersonalAgent,
                (0..=MAX_PERSONAL_AGENT_ROTATION_GRANTS / MAX_FOLDER_ROTATION_GRANTS).map(|_| {
                    FolderRotationFanout {
                        grants: MAX_FOLDER_ROTATION_GRANTS,
                        reencrypted_records: 0,
                    }
                }),
            ),
            Err(CoreError::RotationFanoutLimitExceeded {
                resource: "aggregate grants",
                ..
            })
        ));
        assert!(matches!(
            validate_folder_rotation_fanout(
                FolderRotationOperation::PersonalAgent,
                (0..=MAX_PERSONAL_AGENT_ROTATION_RECORDS / MAX_FOLDER_ROTATION_RECORDS).map(|_| {
                    FolderRotationFanout {
                        grants: 0,
                        reencrypted_records: MAX_FOLDER_ROTATION_RECORDS,
                    }
                }),
            ),
            Err(CoreError::RotationFanoutLimitExceeded {
                resource: "aggregate re-encrypted records",
                ..
            })
        ));
    }

    #[test]
    fn bounds_folder_access_removal_rotation_fanout() {
        assert!(matches!(
            validate_folder_rotation_fanout(
                FolderRotationOperation::FolderAccessRemoval,
                [FolderRotationFanout {
                    grants: 0,
                    reencrypted_records: MAX_FOLDER_ACCESS_REMOVAL_RECORDS + 1,
                }],
            ),
            Err(CoreError::RotationFanoutLimitExceeded {
                resource: "re-encrypted records per Folder rotation",
                ..
            })
        ));
    }

    #[test]
    fn validates_paths_and_names() {
        let decomposed = "Cafe\u{301}/notes.md";
        let path = SafeRelativePath::new("page_path", decomposed).unwrap();
        assert_eq!(path.as_str(), "Café/notes.md");

        assert_eq!(
            SafeRelativePath::new("page_path", "/absolute").unwrap_err(),
            CoreError::InvalidPath {
                field: "page_path",
                value: "/absolute".to_owned()
            }
        );
        assert_eq!(
            SafeRelativePath::new("page_path", "a/../b").unwrap_err(),
            CoreError::InvalidPath {
                field: "page_path",
                value: "a/../b".to_owned()
            }
        );
        assert_eq!(
            SafeRelativePath::new("page_path", ".git/config").unwrap_err(),
            CoreError::InvalidPath {
                field: "page_path",
                value: ".git/config".to_owned()
            }
        );
        assert!(DisplayName::new("folder_name", "bad/name").is_err());
        assert!(DisplayName::new("folder_name", "bad\u{0}name").is_err());
        assert!(DisplayName::new("folder_name", "x".repeat(MAX_DISPLAY_NAME_LEN + 1)).is_err());
        assert!(SafeRelativePath::new("page_path", format!("{}.md", "x".repeat(1025))).is_err());
        assert!(UserId::new("x".repeat(MAX_USER_ID_LEN + 1)).is_err());
        assert!(ObjectId::new("too-short").is_err());
        assert!(ObjectId::new("object_id_with_extension.md").is_err());
    }

    #[test]
    fn folder_and_page_collisions_are_case_sensitive() {
        let mut draft = VaultDraft::default();
        let root = root_folder(
            "root",
            "Root",
            FolderRole::Folder,
            FolderAccessMode::Restricted,
        )
        .unwrap();
        draft.add_folder(root.clone()).unwrap();

        let duplicate = Folder {
            id: FolderId::new("other").unwrap(),
            name: root.name.clone(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Restricted,
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Root 2").unwrap(),
            current_key_version: 1,
            shared_folder_source: false,
        };
        assert_eq!(
            draft.add_folder(duplicate).unwrap_err(),
            CoreError::Collision {
                field: "sibling_folder_name",
                value: "Root".to_owned()
            }
        );

        draft
            .add_folder(Folder {
                id: FolderId::new("lower").unwrap(),
                name: DisplayName::new("folder_name", "root").unwrap(),
                role: FolderRole::Folder,
                access: FolderAccessMode::Restricted,
                parent_folder_id: None,
                path: SafeRelativePath::new("folder_path", "root").unwrap(),
                current_key_version: 1,
                shared_folder_source: false,
            })
            .unwrap();

        let object = FolderObject {
            object_id: ObjectId::new("object_0000000001").unwrap(),
            folder_id: root.id.clone(),
            plaintext_path: SafeRelativePath::new("page_path", "wiki/Intro.md").unwrap(),
        };
        draft.add_object(object.clone()).unwrap();
        assert_eq!(
            draft
                .add_object(FolderObject {
                    object_id: ObjectId::new("object_0000000002").unwrap(),
                    ..object
                })
                .unwrap_err(),
            CoreError::Collision {
                field: "object_path",
                value: "wiki/Intro.md".to_owned()
            }
        );
    }

    #[test]
    fn child_access_is_independent_from_parent_access() {
        let mut draft = VaultDraft::default();
        let parent = root_folder(
            "parent",
            "Parent",
            FolderRole::Folder,
            FolderAccessMode::AllMembers,
        )
        .unwrap();
        draft.add_folder(parent.clone()).unwrap();

        let child = Folder {
            id: FolderId::new("child").unwrap(),
            name: DisplayName::new("folder_name", "Child").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Restricted,
            parent_folder_id: Some(parent.id.clone()),
            path: SafeRelativePath::new("folder_path", "Parent/Child").unwrap(),
            current_key_version: 1,
            shared_folder_source: false,
        };
        draft.add_folder(child.clone()).unwrap();

        let folders = draft.folders();
        let stored_parent = folders
            .iter()
            .find(|folder| folder.id == parent.id)
            .unwrap();
        let stored_child = folders.iter().find(|folder| folder.id == child.id).unwrap();

        assert_eq!(stored_parent.access, FolderAccessMode::AllMembers);
        assert_eq!(stored_child.access, FolderAccessMode::Restricted);
        assert_ne!(stored_parent.access, stored_child.access);
    }

    #[test]
    fn rejects_invalid_hierarchy() {
        let mut draft = VaultDraft::default();
        let orphan = Folder {
            id: FolderId::new("orphan").unwrap(),
            name: DisplayName::new("folder_name", "Orphan").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Restricted,
            parent_folder_id: Some(FolderId::new("missing").unwrap()),
            path: SafeRelativePath::new("folder_path", "Missing/Orphan").unwrap(),
            current_key_version: 1,
            shared_folder_source: false,
        };

        assert_eq!(
            draft.add_folder(orphan).unwrap_err(),
            CoreError::InvalidHierarchy {
                reason: "missing parent folder: missing".to_owned()
            }
        );
    }

    #[test]
    fn smoke_bootstrap_summary_is_stable() {
        let summary = smoke_bootstrap_summary().unwrap();

        assert_eq!(summary.personal.kind, VaultKind::Personal);
        assert!(summary.personal.folder_ids.is_empty());
        assert_eq!(summary.personal.required_grants, 0);

        assert_eq!(summary.organization.kind, VaultKind::Organization);
        assert!(summary.organization.folder_ids.is_empty());
        assert_eq!(summary.organization.required_grants, 0);
        assert_eq!(summary.organization.admin_count, 1);
        assert_eq!(summary.organization.member_count, 1);
    }

    #[test]
    fn hashes_canonical_spec_vectors() {
        let request_body = r#"{"recordType":"folder_object_revision","folderId":"strategy","objectId":"obj_0123456789abcdef"}"#;
        assert_eq!(
            sha256_hex(request_body),
            "beb370cd8804a3a4e7b4764f1f7fdf4bac95895004513a19abee515a2b9c55e4"
        );

        let envelope = r#"{"version":"finite-folder-object-v1","cipher":"AES-256-GCM","keyVersion":1,"nonce":"AAAAAAAAAAAAAAAA","ciphertext":"AQIDBAUGBwgJCgsMDQ4PEA=="}"#;
        assert_eq!(
            ciphertext_hash(envelope),
            "9083fa9666f921de7da1d0b435903e98045b27a1065030dc6d4c841d2374b5bb"
        );
    }

    #[test]
    fn encrypts_and_opens_folder_object_with_aad() {
        let key = FolderKey::from_bytes([7; 32]);
        let aad = folder_object_aad(1);
        let plaintext = br#"{"path":"wiki/concepts/example.md","body":"hello"}"#;

        let envelope = encrypt_folder_object_with_nonce(&key, &aad, [0; 12], plaintext).unwrap();

        assert_eq!(envelope.version, FOLDER_OBJECT_VERSION);
        assert_eq!(envelope.cipher, CIPHER_AES_256_GCM);
        assert_eq!(envelope.key_version, 1);
        assert_eq!(envelope.nonce, "AAAAAAAAAAAAAAAA");
        assert_eq!(
            open_folder_object(&key, &aad, &envelope).unwrap(),
            plaintext
        );
        assert_eq!(
            EncryptedFolderObjectEnvelope::from_json(&envelope.canonical_json()).unwrap(),
            envelope
        );
    }

    #[test]
    fn rejects_wrong_folder_object_aad() {
        let key = FolderKey::from_bytes([7; 32]);
        let aad = folder_object_aad(1);
        let envelope = encrypt_folder_object_with_nonce(&key, &aad, [1; 12], b"hello").unwrap();
        let wrong_aad = FolderObjectAad {
            object_id: ObjectId::new("obj_aaaaaaaaaaaaaaaa").unwrap(),
            ..aad
        };

        assert_eq!(
            open_folder_object(&key, &wrong_aad, &envelope).unwrap_err(),
            CryptoRecordError::AadMismatch
        );
    }

    #[test]
    fn rejects_wrong_folder_key_version() {
        let key = FolderKey::from_bytes([7; 32]);
        let aad = folder_object_aad(1);
        let envelope = encrypt_folder_object_with_nonce(&key, &aad, [2; 12], b"hello").unwrap();
        let wrong_version = folder_object_aad(2);

        assert_eq!(
            open_folder_object(&key, &wrong_version, &envelope).unwrap_err(),
            CryptoRecordError::KeyVersionMismatch {
                expected: 2,
                actual: 1
            }
        );
    }

    #[test]
    fn validates_signed_create_update_and_move_revisions() {
        let keys = Keys::generate();
        let author_npub = npub(&keys);
        let envelope_json = sample_envelope_json();

        for (operation, revision, base_revision) in [
            (FolderObjectOperation::Create, 1, None),
            (FolderObjectOperation::Update, 2, Some(1)),
            (FolderObjectOperation::Move, 3, Some(2)),
        ] {
            let expected = revision_validation(
                operation,
                revision,
                base_revision,
                author_npub.clone(),
                envelope_json.clone(),
            );
            let payload = FolderObjectRevisionPayload::new(&expected);
            let event = sign_app_event(&keys, payload.canonical_json(), revision_tags(&expected));

            assert_eq!(validate_revision_event(&event, &expected).unwrap(), payload);
        }
    }

    #[test]
    fn rejects_revision_ciphertext_hash_mismatch() {
        let keys = Keys::generate();
        let expected = revision_validation(
            FolderObjectOperation::Create,
            1,
            None,
            npub(&keys),
            sample_envelope_json(),
        );
        let payload = FolderObjectRevisionPayload {
            ciphertext_hash: sha256_hex("different envelope"),
            ..FolderObjectRevisionPayload::new(&expected)
        };
        let event = sign_app_event(&keys, payload.canonical_json(), revision_tags(&expected));

        assert!(matches!(
            validate_revision_event(&event, &expected).unwrap_err(),
            CryptoRecordError::CiphertextHashMismatch { .. }
        ));
    }

    #[test]
    fn rejects_malformed_revision_payloads_and_bad_operations() {
        let keys = Keys::generate();
        let expected = revision_validation(
            FolderObjectOperation::Create,
            1,
            None,
            npub(&keys),
            sample_envelope_json(),
        );
        let malformed = sign_app_event(&keys, "{}".to_owned(), revision_tags(&expected));
        assert!(matches!(
            validate_revision_event(&malformed, &expected).unwrap_err(),
            CryptoRecordError::EventMismatch { .. }
        ));

        let bad_operation = FolderObjectRevisionPayload {
            operation: "delete".to_owned(),
            ..FolderObjectRevisionPayload::new(&expected)
        };
        let event = sign_app_event(
            &keys,
            bad_operation.canonical_json(),
            revision_tags(&expected),
        );

        assert_eq!(
            validate_revision_event(&event, &expected).unwrap_err(),
            CryptoRecordError::BadOperation {
                operation: "delete".to_owned()
            }
        );
    }

    #[test]
    fn rejects_revision_signer_mismatch_and_invalid_envelopes() {
        let author_keys = Keys::generate();
        let signer_keys = Keys::generate();
        let expected = revision_validation(
            FolderObjectOperation::Create,
            1,
            None,
            npub(&author_keys),
            sample_envelope_json(),
        );
        let payload = FolderObjectRevisionPayload::new(&expected);
        let event = sign_app_event(
            &signer_keys,
            payload.canonical_json(),
            revision_tags(&expected),
        );

        assert!(matches!(
            validate_revision_event(&event, &expected).unwrap_err(),
            CryptoRecordError::SignerMismatch { .. }
        ));

        let invalid_envelope = RevisionValidation {
            envelope_json: r#"{"bad":true}"#.to_owned(),
            ..expected
        };
        assert!(matches!(
            validate_revision_event(&event, &invalid_envelope).unwrap_err(),
            CryptoRecordError::InvalidEnvelope { .. }
        ));

        let key_version_mismatch = RevisionValidation {
            key_version: 2,
            ..revision_validation(
                FolderObjectOperation::Create,
                1,
                None,
                npub(&author_keys),
                sample_envelope_json(),
            )
        };
        assert_eq!(
            validate_revision_event(&event, &key_version_mismatch).unwrap_err(),
            CryptoRecordError::KeyVersionMismatch {
                expected: 2,
                actual: 1
            }
        );
    }

    #[test]
    fn validates_signed_tombstone() {
        let keys = Keys::generate();
        let expected = TombstoneValidation {
            vault_id: VaultId::new("acme").unwrap(),
            folder_id: FolderId::new("strategy").unwrap(),
            object_id: ObjectId::new("obj_0123456789abcdef").unwrap(),
            revision: 4,
            base_revision: 3,
            author_npub: npub(&keys),
            deleted_at: "2026-06-23T00:01:00.000Z".to_owned(),
        };
        let payload = FolderObjectTombstonePayload::new(&expected);
        let event = sign_app_event(&keys, payload.canonical_json(), tombstone_tags(&expected));

        assert_eq!(
            validate_tombstone_event(&event, &expected).unwrap(),
            payload
        );
    }

    #[test]
    fn validates_signed_admin_access_change() {
        let admin_keys = Keys::generate();
        let target_keys = Keys::generate();
        let expected = AdminAccessChangeValidation {
            vault_id: VaultId::new("acme").unwrap(),
            change_id: "change_0123456789abcdef".to_owned(),
            action: AdminAccessAction::GrantFolderAccess,
            admin_npub: npub(&admin_keys),
            folder_id: Some(FolderId::new("strategy").unwrap()),
            target_npub: Some(npub(&target_keys)),
            key_version: Some(2),
            note: Some("initial restricted access".to_owned()),
            created_at: "2026-06-23T00:02:00.000Z".to_owned(),
        };
        let payload = AdminAccessChangePayload::new(&expected);
        let event = sign_app_event(
            &admin_keys,
            payload.canonical_json(),
            admin_access_change_tags(&expected).unwrap(),
        );

        assert_eq!(
            validate_admin_access_change_event(&event, &expected).unwrap(),
            payload
        );
    }

    #[test]
    fn bounds_brain_invite_authorization_and_bootstrap_fanout() {
        let issuer = Keys::generate();
        let recipient = Keys::generate();
        let recipient_npub = npub(&recipient);
        let folders = (0..=MAX_BRAIN_INVITE_BOOTSTRAP_FOLDERS)
            .map(|index| BrainEmailInviteAuthorizationFolder {
                folder_id: format!("folder-{index}"),
                access: FolderAccessMode::Restricted,
                key_version: 1,
            })
            .collect::<Vec<_>>();
        let authorization = BrainEmailInviteAuthorizationPayload {
            version: "finite-email-invite-bootstrap-authorization-v1".to_owned(),
            vault_id: "personal".to_owned(),
            invited_email: "invitee@example.com".to_owned(),
            invite_unwrap_npub: recipient_npub.clone(),
            bootstrap_payload_hash: format!("sha256:{}", "1".repeat(64)),
            expires_at: "2026-07-14T00:00:00.000Z".to_owned(),
            folders: folders.clone(),
        };
        let authorization_error = validate_brain_event_authorization_intent(
            &BrainEventAuthorizationIntent {
                intent: "vault-invite-authorization".to_owned(),
                event_template: BrainEventTemplate {
                    kind: APP_SPECIFIC_KIND,
                    created_at: 1_784_000_000,
                    tags: Vec::new(),
                    content: authorization.canonical_json(),
                },
            },
            &npub(&issuer),
        )
        .unwrap_err();
        assert!(
            authorization_error
                .to_string()
                .contains("Folder scope exceeds the supported limit")
        );

        let grants = folders
            .iter()
            .map(|folder| {
                serde_json::json!({
                    "folderId": folder.folder_id,
                    "grant": {
                        "id": format!("grant-{}", folder.folder_id),
                        "keyVersion": 1,
                        "recipientNpub": recipient_npub,
                        "wrappedEventJson": "{}",
                        "createdAt": "2026-07-13T00:00:00.000Z"
                    }
                })
            })
            .collect::<Vec<_>>();
        let bootstrap = serde_json::json!({
            "version": "finite-email-invite-bootstrap-payload-v1",
            "vaultId": "personal",
            "invitedEmail": "invitee@example.com",
            "inviteUnwrapNpub": recipient_npub,
            "folders": folders,
            "grants": grants,
        })
        .to_string();
        let bootstrap_error = wrap_brain_invite_bootstrap(
            &issuer,
            &BrainGrantIntent {
                purpose: "vault-invite-bootstrap".to_owned(),
                vault_id: "personal".to_owned(),
                recipient_npub,
                folder_id: None,
                key_version: None,
            },
            &bootstrap,
            1_784_000_000,
        )
        .unwrap_err();
        assert!(
            bootstrap_error
                .to_string()
                .contains("Folder scope exceeds the supported limit")
        );
    }

    fn folder_object_aad(key_version: u32) -> FolderObjectAad {
        FolderObjectAad {
            vault_id: VaultId::new("acme").unwrap(),
            folder_id: FolderId::new("strategy").unwrap(),
            object_id: ObjectId::new("obj_0123456789abcdef").unwrap(),
            key_version,
        }
    }

    fn sample_envelope_json() -> String {
        let key = FolderKey::from_bytes([9; 32]);
        let aad = folder_object_aad(1);
        encrypt_folder_object_with_nonce(&key, &aad, [3; 12], b"encrypted page")
            .unwrap()
            .canonical_json()
    }

    fn revision_validation(
        operation: FolderObjectOperation,
        revision: u64,
        base_revision: Option<u64>,
        author_npub: String,
        envelope_json: String,
    ) -> RevisionValidation {
        RevisionValidation {
            vault_id: VaultId::new("acme").unwrap(),
            folder_id: FolderId::new("strategy").unwrap(),
            object_id: ObjectId::new("obj_0123456789abcdef").unwrap(),
            operation,
            revision,
            base_revision,
            key_version: 1,
            envelope_json,
            author_npub,
            created_at: "2026-06-23T00:00:00.000Z".to_owned(),
        }
    }

    fn sign_app_event(keys: &Keys, content: String, tags: Vec<Vec<String>>) -> Event {
        let tags = tags
            .into_iter()
            .map(|tag| Tag::parse(tag).unwrap())
            .collect::<Vec<_>>();

        EventBuilder::new(Kind::ApplicationSpecificData, content)
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(1_780_000_000))
            .finalize(keys)
            .unwrap()
    }

    fn npub(keys: &Keys) -> String {
        NostrPublicKey::from_protocol(keys.public_key())
            .to_npub()
            .unwrap()
    }
}
