use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Protocol version this build of finite chat speaks (ADR 0003 §1).
pub const PROTOCOL_VERSION_V1: u32 = 1;
/// Oldest room protocol version this build still accepts.
pub const MIN_SUPPORTED_PROTOCOL_VERSION: u32 = 1;
pub const MAX_REQUIRED_CAPABILITIES: u32 = 16;
pub const MAX_REQUIRED_CAPABILITY_BYTES: u32 = 64;

pub type AccountId = String;
pub type DeviceId = String;
pub type RoomId = String;
pub type MlsGroupId = String;
pub type MessageId = String;
pub type KeyPackageId = String;
pub type KeyPackageRef = String;
pub type KeyPackageHash = String;
pub type WelcomeId = String;
pub type LeaseToken = String;
pub type IdempotencyKey = String;
pub type ConversationId = String;
pub type ConversationSegmentId = String;
pub type RuntimeStateKey = String;
pub type RuntimeCommandRequestId = String;
pub type RuntimeCommandName = String;
pub type RuntimeCommandResourceKey = String;
pub type ActivityKind = String;
pub type ActivityId = String;
pub type AttachmentBlobUrl = String;
pub type AttachmentHash = String;
pub type Epoch = u64;
pub type Seq = u64;

pub const MESSAGE_ID_DOMAIN: &[u8] = b"finite-message-id-v1";
pub const MAX_ENVELOPE_PAYLOAD_BYTES: u32 = 256 * 1024;
pub const MAX_SYNC_PAGE_ENTRIES: u32 = 100;
pub const MAX_SYNC_PAGE_BYTES: u32 = 4 * 1024 * 1024;
pub const MAX_ACCOUNT_DEVICES_PER_ROOM: u32 = 32;
pub const MAX_KEY_PACKAGES_PER_DEVICE: u32 = 64;
pub const MAX_KEY_PACKAGE_PAYLOAD_BYTES: u32 = 64 * 1024;
pub const MAX_WELCOME_CLAIMS_PER_REQUEST: u32 = 32;
pub const MAX_STAGED_WELCOMES_PER_COMMIT: u32 = 32;
pub const MAX_ACCOUNT_ROOM_DISCOVERY_RESULTS: u32 = 256;
pub const MAX_WELCOME_PAYLOAD_BYTES: u32 = 1024 * 1024;
pub const MAX_RATCHET_TREE_PAYLOAD_BYTES: u32 = 1024 * 1024;
pub const MAX_LINK_SESSION_PAYLOAD_BYTES: u32 = 1024 * 1024;
pub const MAX_ATTACHMENT_PLAINTEXT_BYTES: u32 = 32 * 1024 * 1024;
pub const MAX_ATTACHMENT_CIPHERTEXT_BYTES: u32 = MAX_ATTACHMENT_PLAINTEXT_BYTES + 16;
pub const MAX_ATTACHMENT_BLOB_URL_BYTES: u32 = 2048;
pub const MAX_ATTACHMENT_FILENAME_BYTES: u32 = 255;
pub const MAX_ATTACHMENT_MIME_TYPE_BYTES: u32 = 128;
pub const MAX_ATTACHMENT_HASH_HEX_BYTES: u32 = 64;
pub const MAX_ATTACHMENT_KEY_HEX_BYTES: u32 = 64;
pub const MAX_ATTACHMENT_NONCE_HEX_BYTES: u32 = 24;
pub const MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES: u32 = 64 * 1024;
pub const MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS: u64 = 5 * 60 * 1000;
pub const MAX_DEVICE_LIVENESS_EXPIRY_MILLIS: u64 = 60 * 1000;
pub const MAX_RUNTIME_STATE_KEYS_PER_ROOM_DEVICE: u32 = 128;
pub const MAX_CONVERSATION_PROJECTION_ENTRIES: u32 = 4096;
pub const MAX_CONVERSATION_SEGMENTS_PER_CONVERSATION: u32 = 1024;
pub const MAX_CONVERSATION_METADATA_PAYLOAD_BYTES: u32 = 16 * 1024;
pub const MAX_CONVERSATION_SEGMENT_PAYLOAD_BYTES: u32 = 16 * 1024;
pub const MAX_RUNTIME_COMMAND_PAYLOAD_BYTES: u32 = 128 * 1024;
pub const MAX_RUNTIME_COMMAND_ERROR_MESSAGE_BYTES: u32 = 2048;
pub const MAX_RUNTIME_COMMAND_ACTIVITY_CLEARS: u32 = 16;
pub const MAX_RUNTIME_COMMAND_LEDGER_RECORDS: u32 = 1024;
pub const MAX_EPHEMERAL_ACTIVITY_DECRYPTED_PAYLOAD_BYTES: u32 = 64 * 1024;
pub const MAX_EPHEMERAL_ACTIVITY_PROJECTION_ENTRIES: u32 = 4096;
pub const MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS: u64 = 30 * 60 * 1000;
pub const MAX_EPHEMERAL_ACTIVITY_CACHE_ENTRIES_PER_ROUTE: u32 = 64;
pub const MAX_CHAT_REACTION_EMOJI_BYTES: u32 = 32;
pub const MAX_IDEMPOTENCY_KEY_BYTES: u32 = 128;
pub const MAX_ACCOUNT_ID_BYTES: u32 = 128;
pub const MAX_DEVICE_ID_BYTES: u32 = 128;
pub const MAX_ROOM_ID_BYTES: u32 = 128;
pub const MAX_MLS_GROUP_ID_BYTES: u32 = 128;
pub const MAX_OBJECT_ID_BYTES: u32 = 128;
pub const FINITECHAT_ATTACHMENT_BLOB_SCHEME_V1: &str = "finitechat.attachment.blob.v1";
pub const FINITECHAT_ATTACHMENT_BLOB_ENCRYPTION_AES256_GCM_V1: &str = "aes-256-gcm.v1";
pub const FINITECHAT_DEFAULT_ACTIVITY_ID: &str = "default";
pub const FINITECHAT_ACTIVITY_KIND_TYPING: &str = "typing";
pub const FINITECHAT_ACTIVITY_KIND_THINKING: &str = "thinking";
pub const FINITECHAT_ACTIVITY_KIND_WORKING: &str = "working";
pub const FINITECHAT_ACTIVITY_KIND_UPLOADING: &str = "uploading";
pub const FINITECHAT_ACTIVITY_KIND_RECORDING: &str = "recording";
pub const FINITECHAT_ACTIVITY_KIND_PRESENT: &str = "present";
pub const FINITECHAT_ACTIVITY_TYPING_EXPIRY_MILLIS: u64 = 30 * 1000;
pub const FINITECHAT_ACTIVITY_PRESENT_EXPIRY_MILLIS: u64 = 2 * 60 * 1000;
pub const FINITECHAT_ACTIVITY_WORKING_EXPIRY_MILLIS: u64 = 5 * 60 * 1000;

const _: () = {
    assert!(MAX_ENVELOPE_PAYLOAD_BYTES > 0);
    assert!(MAX_SYNC_PAGE_ENTRIES > 0);
    assert!(MAX_SYNC_PAGE_BYTES >= MAX_ENVELOPE_PAYLOAD_BYTES);
    assert!(MAX_KEY_PACKAGE_PAYLOAD_BYTES > 0);
    assert!(MAX_KEY_PACKAGE_PAYLOAD_BYTES < MAX_WELCOME_PAYLOAD_BYTES);
    assert!(MAX_WELCOME_CLAIMS_PER_REQUEST > 0);
    assert!(MAX_STAGED_WELCOMES_PER_COMMIT > 0);
    assert!(MAX_ACCOUNT_ROOM_DISCOVERY_RESULTS > 0);
    assert!(MAX_WELCOME_PAYLOAD_BYTES > 0);
    assert!(MAX_RATCHET_TREE_PAYLOAD_BYTES > 0);
    assert!(MAX_LINK_SESSION_PAYLOAD_BYTES > 0);
    assert!(MAX_CHAT_REACTION_EMOJI_BYTES > 0);
    assert!(MAX_ATTACHMENT_PLAINTEXT_BYTES > MAX_ENVELOPE_PAYLOAD_BYTES);
    assert!(MAX_ATTACHMENT_CIPHERTEXT_BYTES > MAX_ATTACHMENT_PLAINTEXT_BYTES);
    assert!(MAX_ATTACHMENT_BLOB_URL_BYTES >= MAX_OBJECT_ID_BYTES);
    assert!(MAX_ATTACHMENT_FILENAME_BYTES > 0);
    assert!(MAX_ATTACHMENT_MIME_TYPE_BYTES > 0);
    assert!(MAX_ATTACHMENT_HASH_HEX_BYTES == 64);
    assert!(MAX_ATTACHMENT_KEY_HEX_BYTES == 64);
    assert!(MAX_ATTACHMENT_NONCE_HEX_BYTES == 24);
    assert!(MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES > 0);
    assert!(MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS > 0);
    assert!(MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS <= MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS);
    assert!(MAX_DEVICE_LIVENESS_EXPIRY_MILLIS > 0);
    assert!(MAX_DEVICE_LIVENESS_EXPIRY_MILLIS <= MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS);
    assert!(MAX_RUNTIME_STATE_KEYS_PER_ROOM_DEVICE > 0);
    assert!(MAX_CONVERSATION_PROJECTION_ENTRIES > 0);
    assert!(MAX_CONVERSATION_SEGMENTS_PER_CONVERSATION > 0);
    assert!(MAX_CONVERSATION_METADATA_PAYLOAD_BYTES > 0);
    assert!(MAX_CONVERSATION_METADATA_PAYLOAD_BYTES <= MAX_ENVELOPE_PAYLOAD_BYTES);
    assert!(MAX_CONVERSATION_SEGMENT_PAYLOAD_BYTES > 0);
    assert!(MAX_RUNTIME_COMMAND_PAYLOAD_BYTES > 0);
    assert!(MAX_RUNTIME_COMMAND_PAYLOAD_BYTES < MAX_ENVELOPE_PAYLOAD_BYTES);
    assert!(MAX_RUNTIME_COMMAND_ERROR_MESSAGE_BYTES > 0);
    assert!(MAX_RUNTIME_COMMAND_ACTIVITY_CLEARS > 0);
    assert!(MAX_RUNTIME_COMMAND_LEDGER_RECORDS > 0);
    assert!(MAX_EPHEMERAL_ACTIVITY_DECRYPTED_PAYLOAD_BYTES > 0);
    assert!(MAX_EPHEMERAL_ACTIVITY_DECRYPTED_PAYLOAD_BYTES < MAX_ENVELOPE_PAYLOAD_BYTES);
    assert!(MAX_EPHEMERAL_ACTIVITY_PROJECTION_ENTRIES > 0);
    assert!(MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS > 0);
    assert!(MAX_EPHEMERAL_ACTIVITY_CACHE_ENTRIES_PER_ROUTE > 0);
    assert!(FINITECHAT_ACTIVITY_TYPING_EXPIRY_MILLIS > 0);
    assert!(FINITECHAT_ACTIVITY_PRESENT_EXPIRY_MILLIS > FINITECHAT_ACTIVITY_TYPING_EXPIRY_MILLIS);
    assert!(FINITECHAT_ACTIVITY_WORKING_EXPIRY_MILLIS > FINITECHAT_ACTIVITY_PRESENT_EXPIRY_MILLIS);
    assert!(FINITECHAT_ACTIVITY_WORKING_EXPIRY_MILLIS <= MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS);
    assert!(MAX_IDEMPOTENCY_KEY_BYTES > 0);
};

/// Per-room protocol slots (ADR 0003 §1): rolled out before any external
/// client exists so version negotiation has somewhere to live. Version 1
/// rooms carry no required capabilities; the fields exist so adding either
/// never changes the wire shape again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoomProtocol {
    pub protocol_version: u32,
    pub required_capabilities: Vec<String>,
}

impl Default for RoomProtocol {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION_V1,
            required_capabilities: Vec::new(),
        }
    }
}

impl RoomProtocol {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_item_count(
            "room_protocol.required_capabilities",
            self.required_capabilities.len(),
            MAX_REQUIRED_CAPABILITIES,
        )?;
        for capability in &self.required_capabilities {
            validate_string_bytes(
                "room_protocol.required_capabilities[]",
                capability,
                MAX_REQUIRED_CAPABILITY_BYTES,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DeviceRef {
    pub account_id: AccountId,
    pub device_id: DeviceId,
}

impl DeviceRef {
    pub fn new(account_id: impl Into<AccountId>, device_id: impl Into<DeviceId>) -> Self {
        Self {
            account_id: account_id.into(),
            device_id: device_id.into(),
        }
    }

    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_string_bytes("account_id", &self.account_id, MAX_ACCOUNT_ID_BYTES)?;
        validate_string_bytes("device_id", &self.device_id, MAX_DEVICE_ID_BYTES)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomStatus {
    Open,
    NeedsRepair,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogEntryKind {
    Application,
    Proposal,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiniteEnvelope {
    pub room_id: RoomId,
    pub mls_group_id: MlsGroupId,
    pub epoch: Epoch,
    pub sender: DeviceRef,
    pub kind: LogEntryKind,
    #[serde(with = "bytes_as_vec")]
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushPolicy {
    Default,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreadPolicy {
    Default,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandInboxPolicy {
    Create,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationDeliveryPolicy {
    pub push: PushPolicy,
    pub unread: UnreadPolicy,
    pub command_inbox: CommandInboxPolicy,
}

impl ApplicationDeliveryPolicy {
    pub const USER_VISIBLE_MESSAGE: Self = Self {
        push: PushPolicy::Default,
        unread: UnreadPolicy::Default,
        command_inbox: CommandInboxPolicy::Never,
    };

    pub const NON_NOTIFYING: Self = Self {
        push: PushPolicy::Never,
        unread: UnreadPolicy::Never,
        command_inbox: CommandInboxPolicy::Never,
    };

    pub const RUNTIME_COMMAND_REQUEST: Self = Self {
        push: PushPolicy::Default,
        unread: UnreadPolicy::Never,
        command_inbox: CommandInboxPolicy::Create,
    };

    pub const RUNTIME_COMMAND_RESULT: Self = Self {
        push: PushPolicy::Never,
        unread: UnreadPolicy::Never,
        command_inbox: CommandInboxPolicy::Never,
    };

    pub fn creates_push(self) -> bool {
        self.push == PushPolicy::Default
    }

    pub fn creates_unread(self) -> bool {
        self.unread == UnreadPolicy::Default
    }

    pub fn creates_command_inbox_work(self) -> bool {
        self.command_inbox == CommandInboxPolicy::Create
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DurableAppEventKind {
    ConversationCreate,
    ConversationUpdate,
    ConversationArchive,
    ConversationSegmentStart,
    ChatMessage,
    ChatEdit,
    ChatReaction,
    ChatReceipt,
    RuntimeStateSnapshot,
    RuntimeCommandRequest,
    RuntimeCommandResult,
    RuntimeCommandCancel,
    /// Durable anchor for a live agent token stream (ADR 0003 §6,
    /// reservation only). Transient deltas never enter the ordered log.
    StreamStart,
    /// Durable close of a stream; the payload carries the transcript hash
    /// the streamed deltas must verify against.
    StreamFinish,
    Namespaced {
        name: String,
        policy: ApplicationDeliveryPolicy,
    },
}

impl DurableAppEventKind {
    pub fn delivery_policy(&self) -> ApplicationDeliveryPolicy {
        match self {
            Self::ChatMessage => ApplicationDeliveryPolicy::USER_VISIBLE_MESSAGE,
            Self::RuntimeCommandRequest => ApplicationDeliveryPolicy::RUNTIME_COMMAND_REQUEST,
            Self::RuntimeCommandResult => ApplicationDeliveryPolicy::RUNTIME_COMMAND_RESULT,
            Self::ConversationSegmentStart
            | Self::ChatEdit
            | Self::ChatReaction
            | Self::ChatReceipt
            | Self::RuntimeStateSnapshot
            | Self::RuntimeCommandCancel
            | Self::StreamStart => ApplicationDeliveryPolicy::NON_NOTIFYING,
            // The finish is the user-visible artifact of a stream: it is the
            // message that gets pushed, not the transient deltas.
            Self::StreamFinish => ApplicationDeliveryPolicy::USER_VISIBLE_MESSAGE,
            Self::ConversationCreate | Self::ConversationUpdate | Self::ConversationArchive => {
                ApplicationDeliveryPolicy::NON_NOTIFYING
            }
            Self::Namespaced { policy, .. } => *policy,
        }
    }

    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        if let Self::Namespaced { name, .. } = self {
            validate_string_bytes(
                "durable_app_event.namespaced_kind",
                name,
                MAX_OBJECT_ID_BYTES,
            )?;
        }
        Ok(())
    }
}

/// Reserved stream-lane anchors (ADR 0003 §6). A stream is pinned to the
/// room epoch at `StreamStartV1`; a membership change mid-stream aborts the
/// stream. The transient delta transport (SSE keyed by room + conversation,
/// replay TTL <= 300 s) is future work — only these durable kinds and the
/// deltas-never-in-the-ordered-log rule are frozen now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamStartV1 {
    pub stream_id: String,
    pub conversation_id: ConversationId,
    /// Room epoch the stream is pinned to.
    pub epoch: Epoch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamFinishV1 {
    pub stream_id: String,
    pub conversation_id: ConversationId,
    /// Hash binding the durable final payload to the transient deltas that
    /// streamed; algorithm is deliberately opaque bytes at this layer.
    #[serde(with = "bytes_as_vec")]
    pub transcript_hash: Vec<u8>,
    /// The complete final text/content, so a client that missed the stream
    /// renders the same message as one that watched it.
    #[serde(with = "bytes_as_vec")]
    pub final_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecryptedApplicationEventV1 {
    pub kind: DurableAppEventKind,
    pub conversation_id: Option<ConversationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<ConversationSegmentId>,
    #[serde(with = "bytes_as_vec")]
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatReactionV1 {
    pub target_message_id: MessageId,
    pub emoji: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatReceiptStateV1 {
    Delivered,
    Read,
    Seen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatReceiptV1 {
    pub target_message_id: MessageId,
    pub target_seq: Seq,
    pub state: ChatReceiptStateV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStateSnapshotV1 {
    pub state_key: RuntimeStateKey,
    pub schema: String,
    pub revision: u64,
    pub observed_at_ms: u64,
    pub expires_at_ms: u64,
    #[serde(with = "bytes_as_vec")]
    pub status_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentDimensionsV1 {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentBlobMetadataV1 {
    pub mime_type: String,
    pub filename: String,
    pub dimensions: Option<AttachmentDimensionsV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentBlobEncryptionV1 {
    pub algorithm: String,
    pub key_hex: String,
    pub nonce_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentBlobReferenceV1 {
    pub scheme: String,
    pub url: AttachmentBlobUrl,
    pub ciphertext_sha256: AttachmentHash,
    pub plaintext_sha256: AttachmentHash,
    pub plaintext_size: u64,
    pub ciphertext_size: u64,
    pub encryption: AttachmentBlobEncryptionV1,
    pub metadata: AttachmentBlobMetadataV1,
}

impl AttachmentBlobMetadataV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("attachment.mime_type", self.mime_type.len())?;
        validate_string_bytes(
            "attachment.mime_type",
            &self.mime_type,
            MAX_ATTACHMENT_MIME_TYPE_BYTES,
        )?;
        validate_bytes_non_empty("attachment.filename", self.filename.len())?;
        validate_string_bytes(
            "attachment.filename",
            &self.filename,
            MAX_ATTACHMENT_FILENAME_BYTES,
        )?;
        if let Some(dimensions) = &self.dimensions {
            dimensions.validate_limits()?;
        }
        Ok(())
    }
}

impl AttachmentDimensionsV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        if self.width == 0 {
            return Err(ProtocolLimitError::BytesEmpty {
                field: "attachment.dimensions.width".to_string(),
            });
        }
        if self.height == 0 {
            return Err(ProtocolLimitError::BytesEmpty {
                field: "attachment.dimensions.height".to_string(),
            });
        }
        Ok(())
    }
}

impl AttachmentBlobEncryptionV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("attachment.encryption.algorithm", self.algorithm.len())?;
        validate_string_bytes(
            "attachment.encryption.algorithm",
            &self.algorithm,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_string_bytes(
            "attachment.encryption.key_hex",
            &self.key_hex,
            MAX_ATTACHMENT_KEY_HEX_BYTES,
        )?;
        validate_string_bytes(
            "attachment.encryption.nonce_hex",
            &self.nonce_hex,
            MAX_ATTACHMENT_NONCE_HEX_BYTES,
        )?;
        Ok(())
    }
}

impl AttachmentBlobReferenceV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("attachment.scheme", self.scheme.len())?;
        validate_string_bytes("attachment.scheme", &self.scheme, MAX_OBJECT_ID_BYTES)?;
        validate_bytes_non_empty("attachment.url", self.url.len())?;
        validate_string_bytes("attachment.url", &self.url, MAX_ATTACHMENT_BLOB_URL_BYTES)?;
        validate_string_bytes(
            "attachment.ciphertext_sha256",
            &self.ciphertext_sha256,
            MAX_ATTACHMENT_HASH_HEX_BYTES,
        )?;
        validate_string_bytes(
            "attachment.plaintext_sha256",
            &self.plaintext_sha256,
            MAX_ATTACHMENT_HASH_HEX_BYTES,
        )?;
        validate_size_limit(
            "attachment.plaintext",
            self.plaintext_size,
            MAX_ATTACHMENT_PLAINTEXT_BYTES,
        )?;
        validate_size_limit(
            "attachment.ciphertext",
            self.ciphertext_size,
            MAX_ATTACHMENT_CIPHERTEXT_BYTES,
        )?;
        self.encryption.validate_limits()?;
        self.metadata.validate_limits()?;
        Ok(())
    }
}

impl RuntimeStateSnapshotV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("runtime_state.state_key", self.state_key.len())?;
        validate_string_bytes(
            "runtime_state.state_key",
            &self.state_key,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("runtime_state.schema", self.schema.len())?;
        validate_string_bytes("runtime_state.schema", &self.schema, MAX_OBJECT_ID_BYTES)?;
        validate_runtime_state_snapshot_expiry(self.observed_at_ms, self.expires_at_ms)?;
        validate_bytes_non_empty("runtime_state.status_payload", self.status_payload.len())?;
        validate_bytes_len(
            "runtime_state.status_payload",
            self.status_payload.len(),
            MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES,
        )?;
        Ok(())
    }

    pub fn is_expired_at(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RuntimeStateProjectionError {
    #[error(
        "runtime state snapshot is missing for room {room_id}, source {source_device:?}, key {state_key}"
    )]
    Missing {
        room_id: RoomId,
        source_device: DeviceRef,
        state_key: RuntimeStateKey,
    },
    #[error("runtime state snapshot {state_key} has schema {actual:?}, expected {expected:?}")]
    WrongSchema {
        state_key: RuntimeStateKey,
        expected: String,
        actual: String,
    },
    #[error("runtime state snapshot {state_key} expired at {expires_at_ms}, now {now_ms}")]
    Expired {
        state_key: RuntimeStateKey,
        now_ms: u64,
        expires_at_ms: u64,
    },
    #[error("runtime state snapshot {state_key} has malformed payload")]
    MalformedPayload { state_key: RuntimeStateKey },
    #[error(transparent)]
    Protocol(#[from] ProtocolLimitError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStateProjectionEntry {
    pub room_id: RoomId,
    pub source: DeviceRef,
    pub accepted_seq: Seq,
    pub snapshot: RuntimeStateSnapshotV1,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStateProjection {
    entries: BTreeMap<String, RuntimeStateProjectionEntry>,
}

#[derive(Debug, Clone, Copy)]
pub struct ConversationProjectionEventContext<'a> {
    pub room_id: &'a str,
    pub accepted_seq: Seq,
    pub conversation_id: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSegmentStartV1 {
    pub segment_id: ConversationSegmentId,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationExternalTopicV1 {
    pub platform: String,
    pub chat_id: String,
    pub thread_id: Option<String>,
    pub topic_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSkillBindingV1 {
    pub namespace: String,
    pub skill_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMetadataV1 {
    pub title: Option<String>,
    pub description: Option<String>,
    pub external_topic: Option<ConversationExternalTopicV1>,
    pub skill_binding: Option<ConversationSkillBindingV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSegmentProjectionRecord {
    pub segment_id: ConversationSegmentId,
    pub started_seq: Seq,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationProjectionEntry {
    pub room_id: RoomId,
    pub conversation_id: ConversationId,
    pub created_seq: Seq,
    pub updated_seq: Seq,
    pub archived: bool,
    pub metadata: Option<ConversationMetadataV1>,
    pub active_segment_id: Option<ConversationSegmentId>,
    pub segments: Vec<ConversationSegmentProjectionRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationProjection {
    entries: BTreeMap<String, ConversationProjectionEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductTrustModeV1 {
    LocalDeviceE2ee,
    HostedTrustedServerClient,
    PlaintextArchive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductClientKindV1 {
    HostedWebBridge,
    NativeDevice,
    ElectronDaemon,
    RuntimeDevice,
    PlaintextArchive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceSecretLocationV1 {
    UserDevice,
    TrustedHostedServer,
    RuntimeHost,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductTrustDisclosureV1 {
    pub mode: ProductTrustModeV1,
    pub label: String,
    pub may_claim_e2ee: bool,
    pub stores_device_secrets_on_user_device: bool,
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationProjectionDecision {
    Ignored,
    Created,
    CreatedByMessage,
    Updated,
    Archived,
    SegmentStarted,
}

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ConversationProjectionError {
    #[error("conversation event {kind:?} requires conversation_id")]
    MissingConversationId { kind: DurableAppEventKind },
    #[error("conversation metadata payload is malformed")]
    MalformedMetadataPayload,
    #[error("conversation segment payload is malformed")]
    MalformedSegmentPayload,
    #[error("conversation segment id already exists: {segment_id}")]
    DuplicateSegment { segment_id: ConversationSegmentId },
    #[error("conversation projection capacity exceeded: max {max_records}")]
    CapacityExceeded { max_records: u32 },
    #[error(transparent)]
    Protocol(#[from] ProtocolLimitError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeCommandPayloadKindV1 {
    #[serde(rename = "runtime.command.request")]
    Request,
    #[serde(rename = "runtime.command.result")]
    Result,
    #[serde(rename = "runtime.command.cancel")]
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCommandTargetV1 {
    pub account_id: AccountId,
    pub device_id: Option<DeviceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCommandJsonPayloadV1 {
    pub schema: String,
    #[serde(with = "bytes_as_vec")]
    pub json_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeActivityClearV1 {
    pub activity_kind: String,
    pub activity_id: Option<String>,
    pub conversation_id: Option<ConversationId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCommandRequestV1 {
    #[serde(rename = "type")]
    pub payload_kind: RuntimeCommandPayloadKindV1,
    pub request_id: RuntimeCommandRequestId,
    pub command: RuntimeCommandName,
    pub target: RuntimeCommandTargetV1,
    pub resource_key: Option<RuntimeCommandResourceKey>,
    pub body: RuntimeCommandJsonPayloadV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCommandTerminalStatusV1 {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCommandErrorV1 {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCommandResultV1 {
    #[serde(rename = "type")]
    pub payload_kind: RuntimeCommandPayloadKindV1,
    pub request_id: RuntimeCommandRequestId,
    pub status: RuntimeCommandTerminalStatusV1,
    pub body: Option<RuntimeCommandJsonPayloadV1>,
    pub error: Option<RuntimeCommandErrorV1>,
    #[serde(default)]
    pub clears_activity: Vec<RuntimeActivityClearV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCommandCancelV1 {
    #[serde(rename = "type")]
    pub payload_kind: RuntimeCommandPayloadKindV1,
    pub request_id: RuntimeCommandRequestId,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCommandLedgerStatus {
    Pending,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCommandLedgerDecision {
    Recorded,
    Replayed,
    IgnoredTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCommandTerminalDecision {
    Recorded,
    Replayed,
    IgnoredAlreadyTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCommandLedgerRecord {
    pub room_id: RoomId,
    pub conversation_id: Option<ConversationId>,
    pub request_id: RuntimeCommandRequestId,
    pub command: RuntimeCommandName,
    pub sender: DeviceRef,
    pub target: RuntimeCommandTargetV1,
    pub original_message_id: MessageId,
    pub accepted_seq: Seq,
    pub resource_key: Option<RuntimeCommandResourceKey>,
    pub status: RuntimeCommandLedgerStatus,
    pub terminal_seq: Option<Seq>,
    pub terminal_message_id: Option<MessageId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCommandLedger {
    records: BTreeMap<String, RuntimeCommandLedgerRecord>,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeCommandIngressContext<'a> {
    pub room_id: &'a str,
    pub conversation_id: Option<&'a str>,
    pub accepted_seq: Seq,
    pub original_message_id: &'a str,
    pub sender: &'a DeviceRef,
    pub local_device: &'a DeviceRef,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeCommandTerminalContext<'a> {
    pub room_id: &'a str,
    pub conversation_id: Option<&'a str>,
    pub request_sender: &'a DeviceRef,
    pub accepted_seq: Seq,
    pub terminal_message_id: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EphemeralActivityActionV1 {
    Set,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenericActivityKindV1 {
    Typing,
    Thinking,
    Working,
    Uploading,
    Recording,
    Present,
}

impl GenericActivityKindV1 {
    pub fn from_activity_kind(activity_kind: &str) -> Option<Self> {
        match activity_kind {
            FINITECHAT_ACTIVITY_KIND_TYPING => Some(Self::Typing),
            FINITECHAT_ACTIVITY_KIND_THINKING => Some(Self::Thinking),
            FINITECHAT_ACTIVITY_KIND_WORKING => Some(Self::Working),
            FINITECHAT_ACTIVITY_KIND_UPLOADING => Some(Self::Uploading),
            FINITECHAT_ACTIVITY_KIND_RECORDING => Some(Self::Recording),
            FINITECHAT_ACTIVITY_KIND_PRESENT => Some(Self::Present),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typing => FINITECHAT_ACTIVITY_KIND_TYPING,
            Self::Thinking => FINITECHAT_ACTIVITY_KIND_THINKING,
            Self::Working => FINITECHAT_ACTIVITY_KIND_WORKING,
            Self::Uploading => FINITECHAT_ACTIVITY_KIND_UPLOADING,
            Self::Recording => FINITECHAT_ACTIVITY_KIND_RECORDING,
            Self::Present => FINITECHAT_ACTIVITY_KIND_PRESENT,
        }
    }

    pub fn recommended_expiry_millis(self) -> u64 {
        match self {
            Self::Typing => FINITECHAT_ACTIVITY_TYPING_EXPIRY_MILLIS,
            Self::Thinking | Self::Working => FINITECHAT_ACTIVITY_WORKING_EXPIRY_MILLIS,
            Self::Uploading | Self::Recording | Self::Present => {
                FINITECHAT_ACTIVITY_PRESENT_EXPIRY_MILLIS
            }
        }
    }
}

pub fn generic_activity_kind_v1(
    activity_kind: &str,
) -> Result<Option<GenericActivityKindV1>, ProtocolLimitError> {
    validate_bytes_non_empty("ephemeral_activity.kind", activity_kind.len())?;
    validate_string_bytes(
        "ephemeral_activity.kind",
        activity_kind,
        MAX_OBJECT_ID_BYTES,
    )?;
    let generic_kind = GenericActivityKindV1::from_activity_kind(activity_kind);
    Ok(generic_kind)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecryptedEphemeralActivityV1 {
    pub activity_kind: ActivityKind,
    pub activity_id: Option<ActivityId>,
    pub action: EphemeralActivityActionV1,
    #[serde(with = "bytes_as_vec")]
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct EphemeralActivityIngressContext<'a> {
    pub room_id: &'a str,
    pub conversation_id: Option<&'a str>,
    pub sender: &'a DeviceRef,
    pub received_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EphemeralActivityProjectionEntry {
    pub room_id: RoomId,
    pub conversation_id: Option<ConversationId>,
    pub sender: DeviceRef,
    pub activity_kind: ActivityKind,
    pub activity_id: ActivityId,
    #[serde(with = "bytes_as_vec")]
    pub payload: Vec<u8>,
    pub received_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EphemeralActivityProjection {
    entries: BTreeMap<String, EphemeralActivityProjectionEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EphemeralActivityProjectionDecision {
    Set,
    Refreshed,
    Cleared,
    ClearMiss,
}

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EphemeralActivityProjectionError {
    #[error("ephemeral activity expired before receipt")]
    AlreadyExpired,
    #[error("ephemeral activity expiry window {actual_millis}ms exceeds max {max_millis}ms")]
    ExpiryTooLong { max_millis: u64, actual_millis: u64 },
    #[error("ephemeral activity projection capacity exceeded: max {max_records}")]
    CapacityExceeded { max_records: u32 },
    #[error(transparent)]
    Protocol(#[from] ProtocolLimitError),
}

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RuntimeCommandPayloadError {
    #[error("runtime command payload kind {actual:?} does not match expected {expected:?}")]
    WrongPayloadKind {
        expected: RuntimeCommandPayloadKindV1,
        actual: RuntimeCommandPayloadKindV1,
    },
    #[error("runtime command result {request_id} is missing a body for success")]
    SuccessMissingBody { request_id: RuntimeCommandRequestId },
    #[error("runtime command result {request_id} is missing an error for failure")]
    FailureMissingError { request_id: RuntimeCommandRequestId },
    #[error(transparent)]
    Protocol(#[from] ProtocolLimitError),
}

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RuntimeCommandLedgerError {
    #[error("runtime command request id conflict for {request_id}")]
    ConflictingRequestId { request_id: RuntimeCommandRequestId },
    #[error("runtime command request not found for {request_id}")]
    RequestNotFound { request_id: RuntimeCommandRequestId },
    #[error(
        "runtime command terminal event for {request_id} at seq {terminal_seq} is not after request seq {request_seq}"
    )]
    TerminalBeforeRequest {
        request_id: RuntimeCommandRequestId,
        request_seq: Seq,
        terminal_seq: Seq,
    },
    #[error("runtime command status {status:?} is not terminal")]
    NonTerminalStatus { status: RuntimeCommandLedgerStatus },
    #[error("runtime command ledger capacity exceeded: max {max_records}")]
    CapacityExceeded { max_records: u32 },
    #[error(transparent)]
    Payload(#[from] RuntimeCommandPayloadError),
    #[error(transparent)]
    Protocol(#[from] ProtocolLimitError),
}

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RuntimeCommandActivityClearError {
    #[error(transparent)]
    Payload(#[from] RuntimeCommandPayloadError),
    #[error(transparent)]
    Projection(#[from] EphemeralActivityProjectionError),
}

impl RuntimeStateProjection {
    pub fn apply(&mut self, entry: RuntimeStateProjectionEntry) -> Result<(), ProtocolLimitError> {
        validate_room_id(&entry.room_id)?;
        entry.source.validate_limits()?;
        entry.snapshot.validate_limits()?;
        let key =
            runtime_state_projection_key(&entry.room_id, &entry.source, &entry.snapshot.state_key)?;
        let should_replace = self
            .entries
            .get(&key)
            .map(|current| {
                entry.snapshot.revision > current.snapshot.revision
                    || (entry.snapshot.revision == current.snapshot.revision
                        && entry.accepted_seq > current.accepted_seq)
            })
            .unwrap_or(true);
        if should_replace {
            self.entries.insert(key, entry);
        }
        Ok(())
    }

    pub fn get(
        &self,
        room_id: &str,
        source: &DeviceRef,
        state_key: &str,
    ) -> Option<&RuntimeStateProjectionEntry> {
        let key = runtime_state_projection_key(room_id, source, state_key).ok()?;
        self.entries.get(&key)
    }

    pub fn require_fresh(
        &self,
        room_id: &str,
        source: &DeviceRef,
        state_key: &str,
        expected_schema: &str,
        now_ms: u64,
    ) -> Result<&RuntimeStateProjectionEntry, RuntimeStateProjectionError> {
        validate_room_id(room_id)?;
        source.validate_limits()?;
        validate_string_bytes("runtime_state.state_key", state_key, MAX_OBJECT_ID_BYTES)?;
        validate_bytes_non_empty("runtime_state.schema", expected_schema.len())?;
        validate_string_bytes("runtime_state.schema", expected_schema, MAX_OBJECT_ID_BYTES)?;
        let entry = self.get(room_id, source, state_key).ok_or_else(|| {
            RuntimeStateProjectionError::Missing {
                room_id: room_id.to_string(),
                source_device: source.clone(),
                state_key: state_key.to_string(),
            }
        })?;
        if entry.snapshot.schema != expected_schema {
            return Err(RuntimeStateProjectionError::WrongSchema {
                state_key: state_key.to_string(),
                expected: expected_schema.to_string(),
                actual: entry.snapshot.schema.clone(),
            });
        }
        if entry.snapshot.is_expired_at(now_ms) {
            return Err(RuntimeStateProjectionError::Expired {
                state_key: state_key.to_string(),
                now_ms,
                expires_at_ms: entry.snapshot.expires_at_ms,
            });
        }
        Ok(entry)
    }

    pub fn require_fresh_json<T: DeserializeOwned>(
        &self,
        room_id: &str,
        source: &DeviceRef,
        state_key: &str,
        expected_schema: &str,
        now_ms: u64,
    ) -> Result<T, RuntimeStateProjectionError> {
        let entry = self.require_fresh(room_id, source, state_key, expected_schema, now_ms)?;
        serde_json::from_slice(&entry.snapshot.status_payload).map_err(|_| {
            RuntimeStateProjectionError::MalformedPayload {
                state_key: state_key.to_string(),
            }
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl ConversationProjectionEventContext<'_> {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_room_id(self.room_id)?;
        if let Some(conversation_id) = self.conversation_id {
            validate_bytes_non_empty("conversation_id", conversation_id.len())?;
            validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
        }
        Ok(())
    }
}

impl ProductTrustDisclosureV1 {
    pub fn for_mode(mode: ProductTrustModeV1) -> Self {
        match mode {
            ProductTrustModeV1::LocalDeviceE2ee => Self {
                mode,
                label: "end-to-end encrypted chat".to_string(),
                may_claim_e2ee: true,
                stores_device_secrets_on_user_device: true,
                read_only: false,
            },
            ProductTrustModeV1::HostedTrustedServerClient => Self {
                mode,
                label: "web chat".to_string(),
                may_claim_e2ee: false,
                stores_device_secrets_on_user_device: false,
                read_only: false,
            },
            ProductTrustModeV1::PlaintextArchive => Self {
                mode,
                label: "archived chat".to_string(),
                may_claim_e2ee: false,
                stores_device_secrets_on_user_device: false,
                read_only: true,
            },
        }
    }
}

impl ProductClientKindV1 {
    pub fn secret_location(self) -> DeviceSecretLocationV1 {
        match self {
            Self::HostedWebBridge => DeviceSecretLocationV1::TrustedHostedServer,
            Self::NativeDevice | Self::ElectronDaemon => DeviceSecretLocationV1::UserDevice,
            Self::RuntimeDevice => DeviceSecretLocationV1::RuntimeHost,
            Self::PlaintextArchive => DeviceSecretLocationV1::None,
        }
    }

    pub fn product_trust_mode(self) -> Option<ProductTrustModeV1> {
        match self {
            Self::HostedWebBridge => Some(ProductTrustModeV1::HostedTrustedServerClient),
            Self::NativeDevice | Self::ElectronDaemon => Some(ProductTrustModeV1::LocalDeviceE2ee),
            Self::PlaintextArchive => Some(ProductTrustModeV1::PlaintextArchive),
            // Runtime devices are chat participants, not user-facing product
            // surfaces. Their secrets stay on the runtime host and product copy
            // belongs to the controlling user client.
            Self::RuntimeDevice => None,
        }
    }

    pub fn is_server_side_bridge(self) -> bool {
        self == Self::HostedWebBridge
    }
}

impl ConversationSegmentStartV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("conversation_segment.segment_id", self.segment_id.len())?;
        validate_string_bytes(
            "conversation_segment.segment_id",
            &self.segment_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        if let Some(reason) = &self.reason {
            validate_bytes_non_empty("conversation_segment.reason", reason.len())?;
            validate_string_bytes("conversation_segment.reason", reason, MAX_OBJECT_ID_BYTES)?;
        }
        Ok(())
    }
}

impl ConversationExternalTopicV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("conversation.external_topic.platform", self.platform.len())?;
        validate_string_bytes(
            "conversation.external_topic.platform",
            &self.platform,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("conversation.external_topic.chat_id", self.chat_id.len())?;
        validate_string_bytes(
            "conversation.external_topic.chat_id",
            &self.chat_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        if let Some(thread_id) = &self.thread_id {
            validate_bytes_non_empty("conversation.external_topic.thread_id", thread_id.len())?;
            validate_string_bytes(
                "conversation.external_topic.thread_id",
                thread_id,
                MAX_OBJECT_ID_BYTES,
            )?;
        }
        if let Some(topic_name) = &self.topic_name {
            validate_bytes_non_empty("conversation.external_topic.topic_name", topic_name.len())?;
            validate_string_bytes(
                "conversation.external_topic.topic_name",
                topic_name,
                MAX_OBJECT_ID_BYTES,
            )?;
        }
        Ok(())
    }
}

impl ConversationSkillBindingV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("conversation.skill_binding.namespace", self.namespace.len())?;
        validate_string_bytes(
            "conversation.skill_binding.namespace",
            &self.namespace,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("conversation.skill_binding.skill_id", self.skill_id.len())?;
        validate_string_bytes(
            "conversation.skill_binding.skill_id",
            &self.skill_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        Ok(())
    }
}

impl ConversationMetadataV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        if let Some(title) = &self.title {
            validate_bytes_non_empty("conversation.metadata.title", title.len())?;
            validate_string_bytes("conversation.metadata.title", title, MAX_OBJECT_ID_BYTES)?;
        }
        if let Some(description) = &self.description {
            validate_bytes_non_empty("conversation.metadata.description", description.len())?;
            validate_string_bytes(
                "conversation.metadata.description",
                description,
                MAX_CONVERSATION_METADATA_PAYLOAD_BYTES,
            )?;
        }
        if let Some(external_topic) = &self.external_topic {
            external_topic.validate_limits()?;
        }
        if let Some(skill_binding) = &self.skill_binding {
            skill_binding.validate_limits()?;
        }
        Ok(())
    }
}

impl ConversationProjection {
    pub fn apply_event(
        &mut self,
        context: ConversationProjectionEventContext<'_>,
        event: &DecryptedApplicationEventV1,
    ) -> Result<ConversationProjectionDecision, ConversationProjectionError> {
        context.validate_limits()?;
        event.validate_limits()?;
        match event.kind {
            DurableAppEventKind::ConversationCreate => {
                let conversation_id = required_conversation_id(&context, &event.kind)?;
                let metadata = parse_conversation_metadata(&event.payload)?;
                self.ensure_entry(context.room_id, conversation_id, context.accepted_seq)?;
                let entry = self
                    .entry_mut(context.room_id, conversation_id)
                    .expect("conversation was ensured before update");
                entry.updated_seq = context.accepted_seq;
                entry.archived = false;
                entry.metadata = Some(metadata);
                Ok(ConversationProjectionDecision::Created)
            }
            DurableAppEventKind::ConversationUpdate => {
                let conversation_id = required_conversation_id(&context, &event.kind)?;
                let metadata = parse_conversation_metadata(&event.payload)?;
                self.ensure_entry(context.room_id, conversation_id, context.accepted_seq)?;
                let entry = self
                    .entry_mut(context.room_id, conversation_id)
                    .expect("conversation was ensured before update");
                entry.updated_seq = context.accepted_seq;
                entry.metadata = Some(metadata);
                Ok(ConversationProjectionDecision::Updated)
            }
            DurableAppEventKind::ConversationArchive => {
                let conversation_id = required_conversation_id(&context, &event.kind)?;
                self.ensure_entry(context.room_id, conversation_id, context.accepted_seq)?;
                let entry = self
                    .entry_mut(context.room_id, conversation_id)
                    .expect("conversation was ensured before archive");
                entry.updated_seq = context.accepted_seq;
                entry.archived = true;
                Ok(ConversationProjectionDecision::Archived)
            }
            DurableAppEventKind::ConversationSegmentStart => {
                let conversation_id = required_conversation_id(&context, &event.kind)?;
                let segment = parse_segment_start(&event.payload)?;
                self.ensure_entry(context.room_id, conversation_id, context.accepted_seq)?;
                let entry = self
                    .entry_mut(context.room_id, conversation_id)
                    .expect("conversation was ensured before segment");
                validate_item_count(
                    "conversation.segments",
                    entry.segments.len() + 1,
                    MAX_CONVERSATION_SEGMENTS_PER_CONVERSATION,
                )?;
                if entry
                    .segments
                    .iter()
                    .any(|record| record.segment_id == segment.segment_id)
                {
                    return Err(ConversationProjectionError::DuplicateSegment {
                        segment_id: segment.segment_id,
                    });
                }
                entry.segments.push(ConversationSegmentProjectionRecord {
                    segment_id: segment.segment_id.clone(),
                    started_seq: context.accepted_seq,
                });
                entry.active_segment_id = Some(segment.segment_id);
                entry.updated_seq = context.accepted_seq;
                assert!(
                    entry.segments.len() <= MAX_CONVERSATION_SEGMENTS_PER_CONVERSATION as usize
                );
                Ok(ConversationProjectionDecision::SegmentStarted)
            }
            DurableAppEventKind::ChatMessage => {
                if let Some(conversation_id) = context.conversation_id {
                    let existed = self.get(context.room_id, conversation_id).is_some();
                    self.ensure_entry(context.room_id, conversation_id, context.accepted_seq)?;
                    let entry = self
                        .entry_mut(context.room_id, conversation_id)
                        .expect("conversation was ensured before message");
                    entry.updated_seq = context.accepted_seq;
                    if existed {
                        Ok(ConversationProjectionDecision::Updated)
                    } else {
                        Ok(ConversationProjectionDecision::CreatedByMessage)
                    }
                } else {
                    Ok(ConversationProjectionDecision::Ignored)
                }
            }
            DurableAppEventKind::ChatEdit
            | DurableAppEventKind::ChatReaction
            | DurableAppEventKind::ChatReceipt
            | DurableAppEventKind::RuntimeStateSnapshot
            | DurableAppEventKind::RuntimeCommandRequest
            | DurableAppEventKind::RuntimeCommandResult
            | DurableAppEventKind::RuntimeCommandCancel
            | DurableAppEventKind::StreamStart
            | DurableAppEventKind::StreamFinish
            | DurableAppEventKind::Namespaced { .. } => Ok(ConversationProjectionDecision::Ignored),
        }
    }

    pub fn get(
        &self,
        room_id: &str,
        conversation_id: &str,
    ) -> Option<&ConversationProjectionEntry> {
        let key = conversation_projection_key(room_id, conversation_id).ok()?;
        self.entries.get(&key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = &ConversationProjectionEntry> {
        self.entries.values()
    }

    fn ensure_entry(
        &mut self,
        room_id: &str,
        conversation_id: &str,
        accepted_seq: Seq,
    ) -> Result<(), ConversationProjectionError> {
        let key = conversation_projection_key(room_id, conversation_id)?;
        if self.entries.contains_key(&key) {
            return Ok(());
        }
        if self.entries.len() >= MAX_CONVERSATION_PROJECTION_ENTRIES as usize {
            return Err(ConversationProjectionError::CapacityExceeded {
                max_records: MAX_CONVERSATION_PROJECTION_ENTRIES,
            });
        }
        self.entries.insert(
            key,
            ConversationProjectionEntry {
                room_id: room_id.to_string(),
                conversation_id: conversation_id.to_string(),
                created_seq: accepted_seq,
                updated_seq: accepted_seq,
                archived: false,
                metadata: None,
                active_segment_id: None,
                segments: Vec::new(),
            },
        );
        assert!(self.entries.len() <= MAX_CONVERSATION_PROJECTION_ENTRIES as usize);
        Ok(())
    }

    fn entry_mut(
        &mut self,
        room_id: &str,
        conversation_id: &str,
    ) -> Option<&mut ConversationProjectionEntry> {
        let key = conversation_projection_key(room_id, conversation_id).ok()?;
        self.entries.get_mut(&key)
    }
}

impl RuntimeCommandTargetV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("runtime_command.target.account_id", self.account_id.len())?;
        validate_string_bytes(
            "runtime_command.target.account_id",
            &self.account_id,
            MAX_ACCOUNT_ID_BYTES,
        )?;
        if let Some(device_id) = &self.device_id {
            validate_bytes_non_empty("runtime_command.target.device_id", device_id.len())?;
            validate_string_bytes(
                "runtime_command.target.device_id",
                device_id,
                MAX_DEVICE_ID_BYTES,
            )?;
        }
        Ok(())
    }

    pub fn matches_device(&self, device: &DeviceRef) -> bool {
        self.account_id == device.account_id
            && self
                .device_id
                .as_ref()
                .is_none_or(|device_id| *device_id == device.device_id)
    }
}

impl RuntimeCommandJsonPayloadV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("runtime_command.payload.schema", self.schema.len())?;
        validate_string_bytes(
            "runtime_command.payload.schema",
            &self.schema,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty(
            "runtime_command.payload.json_payload",
            self.json_payload.len(),
        )?;
        validate_bytes_len(
            "runtime_command.payload.json_payload",
            self.json_payload.len(),
            MAX_RUNTIME_COMMAND_PAYLOAD_BYTES,
        )?;
        Ok(())
    }
}

impl RuntimeActivityClearV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("runtime_activity_clear.kind", self.activity_kind.len())?;
        validate_string_bytes(
            "runtime_activity_clear.kind",
            &self.activity_kind,
            MAX_OBJECT_ID_BYTES,
        )?;
        if let Some(activity_id) = &self.activity_id {
            validate_bytes_non_empty("runtime_activity_clear.activity_id", activity_id.len())?;
            validate_string_bytes(
                "runtime_activity_clear.activity_id",
                activity_id,
                MAX_OBJECT_ID_BYTES,
            )?;
        }
        if let Some(conversation_id) = &self.conversation_id {
            validate_bytes_non_empty(
                "runtime_activity_clear.conversation_id",
                conversation_id.len(),
            )?;
            validate_string_bytes(
                "runtime_activity_clear.conversation_id",
                conversation_id,
                MAX_OBJECT_ID_BYTES,
            )?;
        }
        Ok(())
    }
}

impl RuntimeCommandRequestV1 {
    pub fn validate_structure(&self) -> Result<(), RuntimeCommandPayloadError> {
        if self.payload_kind != RuntimeCommandPayloadKindV1::Request {
            return Err(RuntimeCommandPayloadError::WrongPayloadKind {
                expected: RuntimeCommandPayloadKindV1::Request,
                actual: self.payload_kind,
            });
        }
        self.validate_limits()?;
        Ok(())
    }

    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("runtime_command.request_id", self.request_id.len())?;
        validate_string_bytes(
            "runtime_command.request_id",
            &self.request_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("runtime_command.command", self.command.len())?;
        validate_string_bytes(
            "runtime_command.command",
            &self.command,
            MAX_OBJECT_ID_BYTES,
        )?;
        self.target.validate_limits()?;
        if let Some(resource_key) = &self.resource_key {
            validate_bytes_non_empty("runtime_command.resource_key", resource_key.len())?;
            validate_string_bytes(
                "runtime_command.resource_key",
                resource_key,
                MAX_OBJECT_ID_BYTES,
            )?;
        }
        self.body.validate_limits()?;
        Ok(())
    }
}

impl RuntimeCommandErrorV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("runtime_command.error.code", self.code.len())?;
        validate_string_bytes(
            "runtime_command.error.code",
            &self.code,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("runtime_command.error.message", self.message.len())?;
        validate_string_bytes(
            "runtime_command.error.message",
            &self.message,
            MAX_RUNTIME_COMMAND_ERROR_MESSAGE_BYTES,
        )?;
        Ok(())
    }
}

impl RuntimeCommandResultV1 {
    pub fn validate_structure(&self) -> Result<(), RuntimeCommandPayloadError> {
        if self.payload_kind != RuntimeCommandPayloadKindV1::Result {
            return Err(RuntimeCommandPayloadError::WrongPayloadKind {
                expected: RuntimeCommandPayloadKindV1::Result,
                actual: self.payload_kind,
            });
        }
        self.validate_limits()?;
        match self.status {
            RuntimeCommandTerminalStatusV1::Succeeded if self.body.is_none() => {
                Err(RuntimeCommandPayloadError::SuccessMissingBody {
                    request_id: self.request_id.clone(),
                })
            }
            RuntimeCommandTerminalStatusV1::Failed if self.error.is_none() => {
                Err(RuntimeCommandPayloadError::FailureMissingError {
                    request_id: self.request_id.clone(),
                })
            }
            RuntimeCommandTerminalStatusV1::Succeeded
            | RuntimeCommandTerminalStatusV1::Failed
            | RuntimeCommandTerminalStatusV1::Cancelled => Ok(()),
        }
    }

    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("runtime_command.request_id", self.request_id.len())?;
        validate_string_bytes(
            "runtime_command.request_id",
            &self.request_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        if let Some(body) = &self.body {
            body.validate_limits()?;
        }
        if let Some(error) = &self.error {
            error.validate_limits()?;
        }
        validate_item_count(
            "runtime_command.clears_activity",
            self.clears_activity.len(),
            MAX_RUNTIME_COMMAND_ACTIVITY_CLEARS,
        )?;
        for clear in &self.clears_activity {
            clear.validate_limits()?;
        }
        Ok(())
    }
}

impl RuntimeCommandCancelV1 {
    pub fn validate_structure(&self) -> Result<(), RuntimeCommandPayloadError> {
        if self.payload_kind != RuntimeCommandPayloadKindV1::Cancel {
            return Err(RuntimeCommandPayloadError::WrongPayloadKind {
                expected: RuntimeCommandPayloadKindV1::Cancel,
                actual: self.payload_kind,
            });
        }
        self.validate_limits()?;
        Ok(())
    }

    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("runtime_command.request_id", self.request_id.len())?;
        validate_string_bytes(
            "runtime_command.request_id",
            &self.request_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        if let Some(reason) = &self.reason {
            validate_bytes_non_empty("runtime_command.cancel.reason", reason.len())?;
            validate_string_bytes("runtime_command.cancel.reason", reason, MAX_OBJECT_ID_BYTES)?;
        }
        Ok(())
    }
}

impl RuntimeCommandLedger {
    pub fn record_request(
        &mut self,
        context: RuntimeCommandIngressContext<'_>,
        request: &RuntimeCommandRequestV1,
    ) -> Result<RuntimeCommandLedgerDecision, RuntimeCommandLedgerError> {
        context.validate_limits()?;
        request.validate_structure()?;
        if !request.target.matches_device(context.local_device) {
            return Ok(RuntimeCommandLedgerDecision::IgnoredTarget);
        }

        let key = runtime_command_ledger_key(
            context.room_id,
            context.conversation_id,
            context.sender,
            &request.request_id,
        )?;
        if let Some(record) = self.records.get(&key) {
            if record.original_message_id == context.original_message_id
                && record.accepted_seq == context.accepted_seq
                && record.command == request.command
            {
                return Ok(RuntimeCommandLedgerDecision::Replayed);
            }
            return Err(RuntimeCommandLedgerError::ConflictingRequestId {
                request_id: request.request_id.clone(),
            });
        }
        if self.records.len() >= MAX_RUNTIME_COMMAND_LEDGER_RECORDS as usize {
            return Err(RuntimeCommandLedgerError::CapacityExceeded {
                max_records: MAX_RUNTIME_COMMAND_LEDGER_RECORDS,
            });
        }

        self.records.insert(
            key,
            RuntimeCommandLedgerRecord {
                room_id: context.room_id.to_string(),
                conversation_id: context.conversation_id.map(str::to_string),
                request_id: request.request_id.clone(),
                command: request.command.clone(),
                sender: context.sender.clone(),
                target: request.target.clone(),
                original_message_id: context.original_message_id.to_string(),
                accepted_seq: context.accepted_seq,
                resource_key: request.resource_key.clone(),
                status: RuntimeCommandLedgerStatus::Pending,
                terminal_seq: None,
                terminal_message_id: None,
            },
        );
        assert!(self.records.len() <= MAX_RUNTIME_COMMAND_LEDGER_RECORDS as usize);
        Ok(RuntimeCommandLedgerDecision::Recorded)
    }

    pub fn apply_result(
        &mut self,
        context: RuntimeCommandTerminalContext<'_>,
        result: &RuntimeCommandResultV1,
    ) -> Result<RuntimeCommandTerminalDecision, RuntimeCommandLedgerError> {
        context.validate_limits()?;
        result.validate_structure()?;
        let status = match result.status {
            RuntimeCommandTerminalStatusV1::Succeeded => RuntimeCommandLedgerStatus::Succeeded,
            RuntimeCommandTerminalStatusV1::Failed => RuntimeCommandLedgerStatus::Failed,
            RuntimeCommandTerminalStatusV1::Cancelled => RuntimeCommandLedgerStatus::Cancelled,
        };
        self.record_terminal(context, &result.request_id, status)
    }

    pub fn apply_cancel(
        &mut self,
        context: RuntimeCommandTerminalContext<'_>,
        cancel: &RuntimeCommandCancelV1,
    ) -> Result<RuntimeCommandTerminalDecision, RuntimeCommandLedgerError> {
        context.validate_limits()?;
        cancel.validate_structure()?;
        self.record_terminal(
            context,
            &cancel.request_id,
            RuntimeCommandLedgerStatus::Cancelled,
        )
    }

    pub fn pending_requests(&self) -> Vec<&RuntimeCommandLedgerRecord> {
        let mut pending = self
            .records
            .values()
            .filter(|record| record.status == RuntimeCommandLedgerStatus::Pending)
            .collect::<Vec<_>>();
        sort_runtime_command_records(&mut pending);
        assert!(pending.len() <= MAX_RUNTIME_COMMAND_LEDGER_RECORDS as usize);
        pending
    }

    pub fn ready_requests(&self) -> Vec<&RuntimeCommandLedgerRecord> {
        let pending = self.pending_requests();
        let mut locked_resources = BTreeSet::new();
        let mut ready = Vec::new();
        // `pending_requests` is ordered by accepted sequence and bounded by
        // `MAX_RUNTIME_COMMAND_LEDGER_RECORDS`.
        for record in pending {
            if let Some(resource_key) = &record.resource_key {
                debug_assert!(!resource_key.is_empty());
                let resource_scope = runtime_command_resource_scope_key(record);
                if locked_resources.contains(&resource_scope) {
                    continue;
                }
                locked_resources.insert(resource_scope);
            }
            ready.push(record);
        }
        assert!(ready.len() <= MAX_RUNTIME_COMMAND_LEDGER_RECORDS as usize);
        ready
    }

    pub fn get(
        &self,
        room_id: &str,
        conversation_id: Option<&str>,
        sender: &DeviceRef,
        request_id: &str,
    ) -> Option<&RuntimeCommandLedgerRecord> {
        let key = runtime_command_ledger_key(room_id, conversation_id, sender, request_id).ok()?;
        self.records.get(&key)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    fn record_terminal(
        &mut self,
        context: RuntimeCommandTerminalContext<'_>,
        request_id: &str,
        status: RuntimeCommandLedgerStatus,
    ) -> Result<RuntimeCommandTerminalDecision, RuntimeCommandLedgerError> {
        if status == RuntimeCommandLedgerStatus::Pending {
            return Err(RuntimeCommandLedgerError::NonTerminalStatus { status });
        }
        let key = runtime_command_ledger_key(
            context.room_id,
            context.conversation_id,
            context.request_sender,
            request_id,
        )?;
        let record = self.records.get_mut(&key).ok_or_else(|| {
            RuntimeCommandLedgerError::RequestNotFound {
                request_id: request_id.to_string(),
            }
        })?;
        if context.accepted_seq <= record.accepted_seq {
            return Err(RuntimeCommandLedgerError::TerminalBeforeRequest {
                request_id: request_id.to_string(),
                request_seq: record.accepted_seq,
                terminal_seq: context.accepted_seq,
            });
        }
        if record.status == RuntimeCommandLedgerStatus::Pending {
            record.status = status;
            record.terminal_seq = Some(context.accepted_seq);
            record.terminal_message_id = Some(context.terminal_message_id.to_string());
            assert_ne!(record.status, RuntimeCommandLedgerStatus::Pending);
            assert_eq!(record.terminal_seq, Some(context.accepted_seq));
            assert_eq!(
                record.terminal_message_id.as_deref(),
                Some(context.terminal_message_id)
            );
            return Ok(RuntimeCommandTerminalDecision::Recorded);
        }
        if record.status == status
            && record.terminal_seq == Some(context.accepted_seq)
            && record.terminal_message_id.as_deref() == Some(context.terminal_message_id)
        {
            Ok(RuntimeCommandTerminalDecision::Replayed)
        } else {
            Ok(RuntimeCommandTerminalDecision::IgnoredAlreadyTerminal)
        }
    }
}

impl RuntimeCommandIngressContext<'_> {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_room_id(self.room_id)?;
        if let Some(conversation_id) = self.conversation_id {
            validate_bytes_non_empty("conversation_id", conversation_id.len())?;
            validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
        }
        validate_bytes_non_empty("message_id", self.original_message_id.len())?;
        validate_string_bytes("message_id", self.original_message_id, MAX_OBJECT_ID_BYTES)?;
        self.sender.validate_limits()?;
        self.local_device.validate_limits()?;
        Ok(())
    }
}

impl RuntimeCommandTerminalContext<'_> {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_room_id(self.room_id)?;
        if let Some(conversation_id) = self.conversation_id {
            validate_bytes_non_empty("conversation_id", conversation_id.len())?;
            validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
        }
        self.request_sender.validate_limits()?;
        validate_bytes_non_empty(
            "runtime_command.terminal_message_id",
            self.terminal_message_id.len(),
        )?;
        validate_string_bytes(
            "runtime_command.terminal_message_id",
            self.terminal_message_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        Ok(())
    }
}

impl DecryptedEphemeralActivityV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        generic_activity_kind_v1(&self.activity_kind)?;
        if let Some(activity_id) = &self.activity_id {
            validate_bytes_non_empty("ephemeral_activity.activity_id", activity_id.len())?;
            validate_string_bytes(
                "ephemeral_activity.activity_id",
                activity_id,
                MAX_OBJECT_ID_BYTES,
            )?;
        }
        match self.action {
            EphemeralActivityActionV1::Set => {
                validate_bytes_non_empty("ephemeral_activity.payload", self.payload.len())?;
                validate_bytes_len(
                    "ephemeral_activity.payload",
                    self.payload.len(),
                    MAX_EPHEMERAL_ACTIVITY_DECRYPTED_PAYLOAD_BYTES,
                )?;
            }
            EphemeralActivityActionV1::Clear => {
                validate_bytes_len(
                    "ephemeral_activity.payload",
                    self.payload.len(),
                    MAX_EPHEMERAL_ACTIVITY_DECRYPTED_PAYLOAD_BYTES,
                )?;
            }
        }
        Ok(())
    }

    pub fn normalized_activity_id(&self) -> &str {
        self.activity_id
            .as_deref()
            .unwrap_or(FINITECHAT_DEFAULT_ACTIVITY_ID)
    }
}

impl EphemeralActivityIngressContext<'_> {
    pub fn validate_limits(&self) -> Result<(), EphemeralActivityProjectionError> {
        validate_room_id(self.room_id)?;
        if let Some(conversation_id) = self.conversation_id {
            validate_bytes_non_empty("conversation_id", conversation_id.len())?;
            validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
        }
        self.sender.validate_limits()?;
        validate_ephemeral_activity_expiry(self.received_at_ms, self.expires_at_ms)?;
        Ok(())
    }
}

impl EphemeralActivityProjection {
    pub fn apply(
        &mut self,
        context: EphemeralActivityIngressContext<'_>,
        activity: &DecryptedEphemeralActivityV1,
    ) -> Result<EphemeralActivityProjectionDecision, EphemeralActivityProjectionError> {
        context.validate_limits()?;
        activity.validate_limits()?;
        let key = ephemeral_activity_projection_key(
            context.room_id,
            context.conversation_id,
            context.sender,
            &activity.activity_kind,
            activity.normalized_activity_id(),
        )?;
        match activity.action {
            EphemeralActivityActionV1::Set => {
                let existed = self.entries.contains_key(&key);
                if !existed
                    && self.entries.len() >= MAX_EPHEMERAL_ACTIVITY_PROJECTION_ENTRIES as usize
                {
                    return Err(EphemeralActivityProjectionError::CapacityExceeded {
                        max_records: MAX_EPHEMERAL_ACTIVITY_PROJECTION_ENTRIES,
                    });
                }
                self.entries.insert(
                    key,
                    EphemeralActivityProjectionEntry {
                        room_id: context.room_id.to_string(),
                        conversation_id: context.conversation_id.map(str::to_string),
                        sender: context.sender.clone(),
                        activity_kind: activity.activity_kind.clone(),
                        activity_id: activity.normalized_activity_id().to_string(),
                        payload: activity.payload.clone(),
                        received_at_ms: context.received_at_ms,
                        expires_at_ms: context.expires_at_ms,
                    },
                );
                assert!(self.entries.len() <= MAX_EPHEMERAL_ACTIVITY_PROJECTION_ENTRIES as usize);
                if existed {
                    Ok(EphemeralActivityProjectionDecision::Refreshed)
                } else {
                    Ok(EphemeralActivityProjectionDecision::Set)
                }
            }
            EphemeralActivityActionV1::Clear => {
                if let Some(entry) = self.entries.get(&key) {
                    // Ephemeral packets are unordered hints. An older clear must
                    // not erase a newer refreshed activity for the same route.
                    if entry.received_at_ms > context.received_at_ms {
                        return Ok(EphemeralActivityProjectionDecision::ClearMiss);
                    }
                }
                if self.entries.remove(&key).is_some() {
                    Ok(EphemeralActivityProjectionDecision::Cleared)
                } else {
                    Ok(EphemeralActivityProjectionDecision::ClearMiss)
                }
            }
        }
    }

    pub fn clear_from_durable_terminal(
        &mut self,
        room_id: &str,
        conversation_id: Option<&str>,
        sender: &DeviceRef,
        clear: &RuntimeActivityClearV1,
    ) -> Result<bool, EphemeralActivityProjectionError> {
        validate_room_id(room_id)?;
        if let Some(conversation_id) = conversation_id {
            validate_bytes_non_empty("conversation_id", conversation_id.len())?;
            validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
        }
        sender.validate_limits()?;
        clear.validate_limits()?;
        let activity_id = clear
            .activity_id
            .as_deref()
            .unwrap_or(FINITECHAT_DEFAULT_ACTIVITY_ID);
        let key = ephemeral_activity_projection_key(
            room_id,
            clear.conversation_id.as_deref().or(conversation_id),
            sender,
            &clear.activity_kind,
            activity_id,
        )?;
        Ok(self.entries.remove(&key).is_some())
    }

    pub fn clear_from_runtime_command_result(
        &mut self,
        room_id: &str,
        conversation_id: Option<&str>,
        sender: &DeviceRef,
        result: &RuntimeCommandResultV1,
    ) -> Result<u32, RuntimeCommandActivityClearError> {
        result.validate_structure()?;
        let mut removed = 0u32;
        // `validate_structure` bounds this loop by
        // `MAX_RUNTIME_COMMAND_ACTIVITY_CLEARS`.
        for clear in &result.clears_activity {
            if self.clear_from_durable_terminal(room_id, conversation_id, sender, clear)? {
                removed += 1;
            }
        }
        assert!(removed <= MAX_RUNTIME_COMMAND_ACTIVITY_CLEARS);
        Ok(removed)
    }

    pub fn clear_from_durable_application_event(
        &mut self,
        room_id: &str,
        sender: &DeviceRef,
        event: &DecryptedApplicationEventV1,
    ) -> Result<u32, EphemeralActivityProjectionError> {
        validate_room_id(room_id)?;
        sender.validate_limits()?;
        event.validate_limits()?;
        match event.kind {
            DurableAppEventKind::ChatMessage => {
                let clear = RuntimeActivityClearV1 {
                    activity_kind: FINITECHAT_ACTIVITY_KIND_TYPING.to_string(),
                    activity_id: None,
                    conversation_id: None,
                };
                let removed = self.clear_from_durable_terminal(
                    room_id,
                    event.conversation_id.as_deref(),
                    sender,
                    &clear,
                )?;
                if removed { Ok(1) } else { Ok(0) }
            }
            _ => Ok(0),
        }
    }

    pub fn expire_at(&mut self, now_ms: u64) -> Result<u32, EphemeralActivityProjectionError> {
        let before = self.entries.len();
        self.entries.retain(|_, entry| entry.expires_at_ms > now_ms);
        let expired = before.saturating_sub(self.entries.len());
        u32::try_from(expired).map_err(|_| EphemeralActivityProjectionError::CapacityExceeded {
            max_records: u32::MAX,
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = &EphemeralActivityProjectionEntry> {
        self.entries.values()
    }

    pub fn get(
        &self,
        room_id: &str,
        conversation_id: Option<&str>,
        sender: &DeviceRef,
        activity_kind: &str,
        activity_id: Option<&str>,
    ) -> Option<&EphemeralActivityProjectionEntry> {
        let key = ephemeral_activity_projection_key(
            room_id,
            conversation_id,
            sender,
            activity_kind,
            activity_id.unwrap_or(FINITECHAT_DEFAULT_ACTIVITY_ID),
        )
        .ok()?;
        self.entries.get(&key)
    }

    pub fn collect_account_activity<'a>(
        &'a self,
        room_id: &str,
        conversation_id: Option<&str>,
        account_id: &str,
        activity_kind: &str,
        activity_id: Option<&str>,
        output: &mut Vec<&'a EphemeralActivityProjectionEntry>,
    ) -> Result<u32, EphemeralActivityProjectionError> {
        validate_room_id(room_id)?;
        if let Some(conversation_id) = conversation_id {
            validate_bytes_non_empty("conversation_id", conversation_id.len())?;
            validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
        }
        validate_string_bytes("account_id", account_id, MAX_ACCOUNT_ID_BYTES)?;
        generic_activity_kind_v1(activity_kind)?;
        let normalized_activity_id = activity_id.unwrap_or(FINITECHAT_DEFAULT_ACTIVITY_ID);
        validate_bytes_non_empty(
            "ephemeral_activity.activity_id",
            normalized_activity_id.len(),
        )?;
        validate_string_bytes(
            "ephemeral_activity.activity_id",
            normalized_activity_id,
            MAX_OBJECT_ID_BYTES,
        )?;

        output.clear();
        // The projection has a fixed entry cap, so this scan is visibly bounded.
        for entry in self.entries.values() {
            if entry.room_id == room_id
                && entry.conversation_id.as_deref() == conversation_id
                && entry.sender.account_id == account_id
                && entry.activity_kind == activity_kind
                && entry.activity_id == normalized_activity_id
            {
                output.push(entry);
            }
        }
        let count = u32::try_from(output.len()).map_err(|_| {
            EphemeralActivityProjectionError::CapacityExceeded {
                max_records: u32::MAX,
            }
        })?;
        assert!(count <= MAX_EPHEMERAL_ACTIVITY_PROJECTION_ENTRIES);
        Ok(count)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl DecryptedApplicationEventV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        self.kind.validate_limits()?;
        if let Some(conversation_id) = &self.conversation_id {
            validate_bytes_non_empty("conversation_id", conversation_id.len())?;
            validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
        }
        if let Some(segment_id) = &self.segment_id {
            validate_bytes_non_empty("segment_id", segment_id.len())?;
            validate_string_bytes("segment_id", segment_id, MAX_OBJECT_ID_BYTES)?;
        }
        let max_payload = if self.kind == DurableAppEventKind::RuntimeStateSnapshot {
            MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES
        } else {
            MAX_ENVELOPE_PAYLOAD_BYTES
        };
        validate_bytes_len("application_event.payload", self.payload.len(), max_payload)?;
        Ok(())
    }
}

impl ChatReactionV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty(
            "chat_reaction.target_message_id",
            self.target_message_id.len(),
        )?;
        validate_string_bytes(
            "chat_reaction.target_message_id",
            &self.target_message_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        let emoji = self.emoji.trim();
        validate_bytes_non_empty("chat_reaction.emoji", emoji.len())?;
        validate_string_bytes("chat_reaction.emoji", emoji, MAX_CHAT_REACTION_EMOJI_BYTES)?;
        Ok(())
    }
}

impl ChatReceiptV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty(
            "chat_receipt.target_message_id",
            self.target_message_id.len(),
        )?;
        validate_string_bytes(
            "chat_receipt.target_message_id",
            &self.target_message_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        if self.target_seq == 0 {
            return Err(ProtocolLimitError::BytesEmpty {
                field: "chat_receipt.target_seq".to_string(),
            });
        }
        Ok(())
    }
}

impl FiniteEnvelope {
    pub fn message_id(&self) -> Result<MessageId, serde_json::Error> {
        message_id_for_envelope(self)
    }

    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_room_id(&self.room_id)?;
        validate_mls_group_id(&self.mls_group_id)?;
        self.sender.validate_limits()?;
        validate_bytes_len(
            "envelope.payload",
            self.payload.len(),
            MAX_ENVELOPE_PAYLOAD_BYTES,
        )?;
        Ok(())
    }
}

pub fn message_id_for_envelope(envelope: &FiniteEnvelope) -> Result<MessageId, serde_json::Error> {
    let mut hasher = Sha256::new();
    hasher.update(MESSAGE_ID_DOMAIN);
    hasher.update(serde_json::to_vec(envelope)?);
    Ok(hex_lower(&hasher.finalize()))
}

pub fn message_id_for_bytes(bytes: &[u8]) -> MessageId {
    let mut hasher = Sha256::new();
    hasher.update(MESSAGE_ID_DOMAIN);
    hasher.update(bytes);
    hex_lower(&hasher.finalize())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipDeltaV1 {
    pub base_epoch: Epoch,
    pub post_commit_epoch: Epoch,
    pub commit_message_id: MessageId,
    #[serde(default)]
    pub adds: Vec<MembershipAddV1>,
    #[serde(default)]
    pub removes: Vec<MembershipRemoveV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipAddV1 {
    pub device: DeviceRef,
    pub key_package_id: KeyPackageId,
    pub key_package_ref: KeyPackageRef,
    pub key_package_hash: KeyPackageHash,
    pub welcome_id: WelcomeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipRemoveV1 {
    pub device: DeviceRef,
    pub removed_leaf_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedWelcomeV1 {
    pub welcome_id: WelcomeId,
    #[serde(with = "bytes_as_vec")]
    pub welcome_payload: Vec<u8>,
    #[serde(with = "bytes_as_vec")]
    pub ratchet_tree_payload: Vec<u8>,
}

impl StagedWelcomeV1 {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_string_bytes("welcome_id", &self.welcome_id, MAX_OBJECT_ID_BYTES)?;
        validate_bytes_non_empty("welcome_payload", self.welcome_payload.len())?;
        validate_bytes_len(
            "welcome_payload",
            self.welcome_payload.len(),
            MAX_WELCOME_PAYLOAD_BYTES,
        )?;
        validate_bytes_non_empty("ratchet_tree_payload", self.ratchet_tree_payload.len())?;
        validate_bytes_len(
            "ratchet_tree_payload",
            self.ratchet_tree_payload.len(),
            MAX_RATCHET_TREE_PAYLOAD_BYTES,
        )?;
        Ok(())
    }
}

impl MembershipDeltaV1 {
    pub fn validate_structure(
        &self,
        expected_epoch: Epoch,
        actual_commit_message_id: &str,
    ) -> Result<(), MembershipDeltaError> {
        if self.base_epoch != expected_epoch {
            return Err(MembershipDeltaError::WrongBaseEpoch {
                expected: expected_epoch,
                actual: self.base_epoch,
            });
        }
        if self.post_commit_epoch != self.base_epoch + 1 {
            return Err(MembershipDeltaError::WrongPostCommitEpoch {
                base: self.base_epoch,
                actual: self.post_commit_epoch,
            });
        }
        if self.commit_message_id != actual_commit_message_id {
            return Err(MembershipDeltaError::WrongCommitMessageId);
        }

        let mut add_devices = BTreeSet::new();
        for add in &self.adds {
            if !add_devices.insert(add.device.clone()) {
                return Err(MembershipDeltaError::DuplicateAdd(add.device.clone()));
            }
            if add.key_package_id.trim().is_empty()
                || add.key_package_ref.trim().is_empty()
                || add.key_package_hash.trim().is_empty()
                || add.welcome_id.trim().is_empty()
            {
                return Err(MembershipDeltaError::IncompleteAdd(add.device.clone()));
            }
        }

        let mut remove_devices = BTreeSet::new();
        for remove in &self.removes {
            if !remove_devices.insert(remove.device.clone()) {
                return Err(MembershipDeltaError::DuplicateRemove(remove.device.clone()));
            }
        }

        if let Some(device) = add_devices.intersection(&remove_devices).next() {
            return Err(MembershipDeltaError::AddAndRemoveSameDevice(
                (*device).clone(),
            ));
        }

        Ok(())
    }

    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        for add in &self.adds {
            add.device.validate_limits()?;
            validate_string_bytes("key_package_id", &add.key_package_id, MAX_OBJECT_ID_BYTES)?;
            validate_string_bytes("key_package_ref", &add.key_package_ref, MAX_OBJECT_ID_BYTES)?;
            validate_string_bytes(
                "key_package_hash",
                &add.key_package_hash,
                MAX_OBJECT_ID_BYTES,
            )?;
            validate_string_bytes("welcome_id", &add.welcome_id, MAX_OBJECT_ID_BYTES)?;
        }
        for remove in &self.removes {
            remove.device.validate_limits()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MembershipDeltaError {
    #[error("membership delta base epoch {actual} does not match expected epoch {expected}")]
    WrongBaseEpoch { expected: Epoch, actual: Epoch },
    #[error("membership delta post-commit epoch {actual} is not base epoch {base} + 1")]
    WrongPostCommitEpoch { base: Epoch, actual: Epoch },
    #[error("membership delta commit message id does not match submitted commit")]
    WrongCommitMessageId,
    #[error("membership delta adds device more than once: {0:?}")]
    DuplicateAdd(DeviceRef),
    #[error("membership delta removes device more than once: {0:?}")]
    DuplicateRemove(DeviceRef),
    #[error("membership delta adds and removes same device: {0:?}")]
    AddAndRemoveSameDevice(DeviceRef),
    #[error("membership delta add is missing key package or welcome fields: {0:?}")]
    IncompleteAdd(DeviceRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyPackageState {
    Available,
    Leased,
    Consumed,
    Released,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WelcomeState {
    Staged,
    Released,
    Claimed,
    Acked,
    Failed,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomLogEntry {
    pub room_id: RoomId,
    pub seq: Seq,
    pub message_id: MessageId,
    pub sender: DeviceRef,
    pub kind: LogEntryKind,
    pub epoch: Epoch,
    pub envelope: FiniteEnvelope,
    pub idempotency_key: IdempotencyKey,
    #[serde(default)]
    pub timestamp_unix_seconds: u64,
}

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ProtocolLimitError {
    #[error("{field} is empty")]
    BytesEmpty { field: String },
    #[error("{field} has {actual_bytes} bytes, max {max_bytes}")]
    BytesTooLong {
        field: String,
        max_bytes: u64,
        actual_bytes: u64,
    },
    #[error("{field} has {actual_items} items, max {max_items}")]
    TooManyItems {
        field: String,
        max_items: u64,
        actual_items: u64,
    },
    #[error("{end_field} must be after {start_field}")]
    InvalidTimeRange {
        start_field: String,
        end_field: String,
    },
    #[error("{field} has duration {actual_millis}ms, max {max_millis}ms")]
    DurationTooLong {
        field: String,
        max_millis: u64,
        actual_millis: u64,
    },
}

pub fn validate_room_id(room_id: &str) -> Result<(), ProtocolLimitError> {
    validate_string_bytes("room_id", room_id, MAX_ROOM_ID_BYTES)
}

pub fn validate_mls_group_id(mls_group_id: &str) -> Result<(), ProtocolLimitError> {
    validate_string_bytes("mls_group_id", mls_group_id, MAX_MLS_GROUP_ID_BYTES)
}

pub fn validate_idempotency_key(key: &str) -> Result<(), ProtocolLimitError> {
    validate_string_bytes("idempotency_key", key, MAX_IDEMPOTENCY_KEY_BYTES)
}

pub fn validate_string_bytes(
    field: &str,
    value: &str,
    max_bytes: u32,
) -> Result<(), ProtocolLimitError> {
    validate_bytes_len(field, value.len(), max_bytes)
}

pub fn validate_bytes_len(
    field: &str,
    actual_bytes: usize,
    max_bytes: u32,
) -> Result<(), ProtocolLimitError> {
    if actual_bytes <= max_bytes as usize {
        Ok(())
    } else {
        Err(ProtocolLimitError::BytesTooLong {
            field: field.to_string(),
            max_bytes: u64::from(max_bytes),
            actual_bytes: actual_bytes as u64,
        })
    }
}

pub fn validate_bytes_non_empty(
    field: &str,
    actual_bytes: usize,
) -> Result<(), ProtocolLimitError> {
    if actual_bytes > 0 {
        Ok(())
    } else {
        Err(ProtocolLimitError::BytesEmpty {
            field: field.to_string(),
        })
    }
}

pub fn validate_item_count(
    field: &str,
    actual_items: usize,
    max_items: u32,
) -> Result<(), ProtocolLimitError> {
    if actual_items <= max_items as usize {
        Ok(())
    } else {
        Err(ProtocolLimitError::TooManyItems {
            field: field.to_string(),
            max_items: u64::from(max_items),
            actual_items: actual_items as u64,
        })
    }
}

pub fn validate_size_limit(
    field: &str,
    actual_bytes: u64,
    max_bytes: u32,
) -> Result<(), ProtocolLimitError> {
    if actual_bytes == 0 {
        return Err(ProtocolLimitError::BytesEmpty {
            field: field.to_string(),
        });
    }
    if actual_bytes <= u64::from(max_bytes) {
        Ok(())
    } else {
        Err(ProtocolLimitError::BytesTooLong {
            field: field.to_string(),
            max_bytes: u64::from(max_bytes),
            actual_bytes,
        })
    }
}

fn runtime_state_projection_key(
    room_id: &str,
    source: &DeviceRef,
    state_key: &str,
) -> Result<String, ProtocolLimitError> {
    validate_room_id(room_id)?;
    source.validate_limits()?;
    validate_string_bytes("runtime_state.state_key", state_key, MAX_OBJECT_ID_BYTES)?;
    Ok(format!(
        "{}|{}|{}|{}",
        length_prefixed(room_id),
        length_prefixed(&source.account_id),
        length_prefixed(&source.device_id),
        length_prefixed(state_key)
    ))
}

fn conversation_projection_key(
    room_id: &str,
    conversation_id: &str,
) -> Result<String, ProtocolLimitError> {
    validate_room_id(room_id)?;
    validate_bytes_non_empty("conversation_id", conversation_id.len())?;
    validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
    Ok(format!(
        "{}|{}",
        length_prefixed(room_id),
        length_prefixed(conversation_id)
    ))
}

fn required_conversation_id<'a>(
    context: &'a ConversationProjectionEventContext<'_>,
    kind: &DurableAppEventKind,
) -> Result<&'a str, ConversationProjectionError> {
    context
        .conversation_id
        .ok_or_else(|| ConversationProjectionError::MissingConversationId { kind: kind.clone() })
}

fn parse_segment_start(
    payload: &[u8],
) -> Result<ConversationSegmentStartV1, ConversationProjectionError> {
    validate_bytes_non_empty("conversation_segment.payload", payload.len())?;
    validate_bytes_len(
        "conversation_segment.payload",
        payload.len(),
        MAX_CONVERSATION_SEGMENT_PAYLOAD_BYTES,
    )?;
    let segment = serde_json::from_slice::<ConversationSegmentStartV1>(payload)
        .map_err(|_| ConversationProjectionError::MalformedSegmentPayload)?;
    segment.validate_limits()?;
    Ok(segment)
}

fn parse_conversation_metadata(
    payload: &[u8],
) -> Result<ConversationMetadataV1, ConversationProjectionError> {
    validate_bytes_non_empty("conversation.metadata.payload", payload.len())?;
    validate_bytes_len(
        "conversation.metadata.payload",
        payload.len(),
        MAX_CONVERSATION_METADATA_PAYLOAD_BYTES,
    )?;
    let metadata = serde_json::from_slice::<ConversationMetadataV1>(payload)
        .map_err(|_| ConversationProjectionError::MalformedMetadataPayload)?;
    metadata.validate_limits()?;
    Ok(metadata)
}

fn runtime_command_ledger_key(
    room_id: &str,
    conversation_id: Option<&str>,
    sender: &DeviceRef,
    request_id: &str,
) -> Result<String, ProtocolLimitError> {
    validate_room_id(room_id)?;
    if let Some(conversation_id) = conversation_id {
        validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
    }
    sender.validate_limits()?;
    validate_string_bytes(
        "runtime_command.request_id",
        request_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    Ok(format!(
        "{}|{}|{}|{}|{}",
        length_prefixed(room_id),
        length_prefixed(conversation_id.unwrap_or("")),
        length_prefixed(&sender.account_id),
        length_prefixed(&sender.device_id),
        length_prefixed(request_id)
    ))
}

fn runtime_command_resource_scope_key(record: &RuntimeCommandLedgerRecord) -> String {
    let resource_key = record
        .resource_key
        .as_deref()
        .expect("resource scope is only built for keyed commands");
    // Conversation id is intentionally not part of this scope. A command in
    // topic A that mutates `hermes.config` must block a command in topic B that
    // mutates the same runtime resource.
    format!(
        "{}|{}|{}|{}",
        length_prefixed(&record.room_id),
        length_prefixed(&record.target.account_id),
        length_prefixed(record.target.device_id.as_deref().unwrap_or("")),
        length_prefixed(resource_key)
    )
}

fn sort_runtime_command_records(records: &mut [&RuntimeCommandLedgerRecord]) {
    records.sort_by(|left, right| {
        left.accepted_seq
            .cmp(&right.accepted_seq)
            .then_with(|| left.original_message_id.cmp(&right.original_message_id))
    });
}

fn ephemeral_activity_projection_key(
    room_id: &str,
    conversation_id: Option<&str>,
    sender: &DeviceRef,
    activity_kind: &str,
    activity_id: &str,
) -> Result<String, ProtocolLimitError> {
    validate_room_id(room_id)?;
    if let Some(conversation_id) = conversation_id {
        validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
    }
    sender.validate_limits()?;
    validate_string_bytes(
        "ephemeral_activity.kind",
        activity_kind,
        MAX_OBJECT_ID_BYTES,
    )?;
    validate_string_bytes(
        "ephemeral_activity.activity_id",
        activity_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    Ok(format!(
        "{}|{}|{}|{}|{}|{}",
        length_prefixed(room_id),
        length_prefixed(conversation_id.unwrap_or("")),
        length_prefixed(&sender.account_id),
        length_prefixed(&sender.device_id),
        length_prefixed(activity_kind),
        length_prefixed(activity_id)
    ))
}

fn validate_ephemeral_activity_expiry(
    received_at_ms: u64,
    expires_at_ms: u64,
) -> Result<(), EphemeralActivityProjectionError> {
    if expires_at_ms <= received_at_ms {
        return Err(EphemeralActivityProjectionError::AlreadyExpired);
    }
    let window = expires_at_ms - received_at_ms;
    if window > MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS {
        return Err(EphemeralActivityProjectionError::ExpiryTooLong {
            max_millis: MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS,
            actual_millis: window,
        });
    }
    Ok(())
}

fn validate_runtime_state_snapshot_expiry(
    observed_at_ms: u64,
    expires_at_ms: u64,
) -> Result<(), ProtocolLimitError> {
    if expires_at_ms <= observed_at_ms {
        return Err(ProtocolLimitError::InvalidTimeRange {
            start_field: "runtime_state.observed_at_ms".to_string(),
            end_field: "runtime_state.expires_at_ms".to_string(),
        });
    }
    let window = expires_at_ms - observed_at_ms;
    if window > MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS {
        return Err(ProtocolLimitError::DurationTooLong {
            field: "runtime_state.expiry_window_millis".to_string(),
            max_millis: MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS,
            actual_millis: window,
        });
    }
    Ok(())
}

fn length_prefixed(value: &str) -> String {
    format!("{}:{value}", value.len())
}

fn hex_lower(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(TABLE[(byte >> 4) as usize] as char);
        out.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    out
}

mod bytes_as_vec {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    /// Opaque payload bytes travel as base64 strings (1.33× the raw size).
    /// The previous JSON number-array form cost 3.6–4.5× on the wire and a
    /// per-element parse; reads stay tolerant of it for stored logs.
    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BytesRepr {
        Base64(String),
        Legacy(Vec<u8>),
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        match BytesRepr::deserialize(deserializer)? {
            BytesRepr::Base64(value) => base64::engine::general_purpose::STANDARD
                .decode(value.as_bytes())
                .map_err(D::Error::custom),
            BytesRepr::Legacy(bytes) => Ok(bytes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(account: &str, id: &str) -> DeviceRef {
        DeviceRef::new(account, id)
    }

    #[test]
    fn message_id_is_stable_for_same_envelope() {
        let envelope = FiniteEnvelope {
            room_id: "room_1".to_string(),
            mls_group_id: "group_1".to_string(),
            epoch: 0,
            sender: device("alice", "phone"),
            kind: LogEntryKind::Application,
            payload: b"hello".to_vec(),
        };
        assert_eq!(
            envelope.message_id().unwrap(),
            envelope.message_id().unwrap()
        );
    }

    #[test]
    fn membership_delta_rejects_duplicate_adds() {
        let delta = MembershipDeltaV1 {
            base_epoch: 0,
            post_commit_epoch: 1,
            commit_message_id: "commit".to_string(),
            adds: vec![
                MembershipAddV1 {
                    device: device("bob", "phone"),
                    key_package_id: "kp_1".to_string(),
                    key_package_ref: "ref".to_string(),
                    key_package_hash: "hash".to_string(),
                    welcome_id: "welcome_1".to_string(),
                },
                MembershipAddV1 {
                    device: device("bob", "phone"),
                    key_package_id: "kp_2".to_string(),
                    key_package_ref: "ref2".to_string(),
                    key_package_hash: "hash2".to_string(),
                    welcome_id: "welcome_2".to_string(),
                },
            ],
            removes: vec![],
        };
        assert_eq!(
            delta.validate_structure(0, "commit").unwrap_err(),
            MembershipDeltaError::DuplicateAdd(device("bob", "phone"))
        );
    }

    #[test]
    fn durable_app_event_defaults_match_push_and_inbox_policy() {
        assert_eq!(
            DurableAppEventKind::ChatMessage.delivery_policy(),
            ApplicationDeliveryPolicy::USER_VISIBLE_MESSAGE
        );
        assert_eq!(
            DurableAppEventKind::ChatReceipt.delivery_policy(),
            ApplicationDeliveryPolicy::NON_NOTIFYING
        );
        assert_eq!(
            DurableAppEventKind::ConversationSegmentStart.delivery_policy(),
            ApplicationDeliveryPolicy::NON_NOTIFYING
        );
        assert_eq!(
            DurableAppEventKind::RuntimeStateSnapshot.delivery_policy(),
            ApplicationDeliveryPolicy::NON_NOTIFYING
        );
        assert_eq!(
            DurableAppEventKind::RuntimeCommandRequest.delivery_policy(),
            ApplicationDeliveryPolicy::RUNTIME_COMMAND_REQUEST
        );
        assert_eq!(
            DurableAppEventKind::RuntimeCommandResult.delivery_policy(),
            ApplicationDeliveryPolicy::RUNTIME_COMMAND_RESULT
        );

        assert!(
            DurableAppEventKind::ChatMessage
                .delivery_policy()
                .creates_push()
        );
        assert!(
            DurableAppEventKind::ChatMessage
                .delivery_policy()
                .creates_unread()
        );
        assert!(
            DurableAppEventKind::RuntimeCommandRequest
                .delivery_policy()
                .creates_command_inbox_work()
        );
        assert!(
            !DurableAppEventKind::RuntimeStateSnapshot
                .delivery_policy()
                .creates_command_inbox_work()
        );
        assert!(
            !DurableAppEventKind::RuntimeCommandResult
                .delivery_policy()
                .creates_push()
        );
    }

    #[test]
    fn decrypted_application_event_rejects_empty_conversation_id() {
        let event = DecryptedApplicationEventV1 {
            kind: DurableAppEventKind::ChatMessage,
            conversation_id: Some(String::new()),
            segment_id: None,
            payload: b"hello".to_vec(),
        };

        assert_eq!(
            event.validate_limits().unwrap_err(),
            ProtocolLimitError::BytesEmpty {
                field: "conversation_id".to_string()
            }
        );
    }

    #[test]
    fn chat_reaction_rejects_empty_and_oversized_emoji() {
        let empty = ChatReactionV1 {
            target_message_id: "msg-1".to_owned(),
            emoji: "  ".to_owned(),
        };
        assert_eq!(
            empty.validate_limits().unwrap_err(),
            ProtocolLimitError::BytesEmpty {
                field: "chat_reaction.emoji".to_owned()
            }
        );

        let oversized = ChatReactionV1 {
            target_message_id: "msg-1".to_owned(),
            emoji: "a".repeat(MAX_CHAT_REACTION_EMOJI_BYTES as usize + 1),
        };
        assert!(matches!(
            oversized.validate_limits().unwrap_err(),
            ProtocolLimitError::BytesTooLong { field, .. } if field == "chat_reaction.emoji"
        ));
    }

    #[test]
    fn chat_receipt_rejects_empty_target_and_zero_sequence() {
        let empty_target = ChatReceiptV1 {
            target_message_id: String::new(),
            target_seq: 1,
            state: ChatReceiptStateV1::Read,
        };
        assert_eq!(
            empty_target.validate_limits().unwrap_err(),
            ProtocolLimitError::BytesEmpty {
                field: "chat_receipt.target_message_id".to_owned()
            }
        );

        let zero_seq = ChatReceiptV1 {
            target_message_id: "msg-1".to_owned(),
            target_seq: 0,
            state: ChatReceiptStateV1::Seen,
        };
        assert_eq!(
            zero_seq.validate_limits().unwrap_err(),
            ProtocolLimitError::BytesEmpty {
                field: "chat_receipt.target_seq".to_owned()
            }
        );
    }

    #[test]
    fn runtime_state_projection_replaces_by_revision_and_sequence() {
        let source = device("runtime_npub", "runtime_box");
        let mut projection = RuntimeStateProjection::default();

        projection
            .apply(runtime_state_entry(
                "room_1",
                source.clone(),
                "runtime.gateway",
                "finite.gateway.v1",
                1,
                10,
                br#"{"status":"down"}"#,
            ))
            .unwrap();
        projection
            .apply(runtime_state_entry(
                "room_1",
                source.clone(),
                "runtime.gateway",
                "finite.gateway.v1",
                1,
                9,
                br#"{"status":"older"}"#,
            ))
            .unwrap();
        assert_eq!(
            projection
                .get("room_1", &source, "runtime.gateway")
                .unwrap()
                .snapshot
                .status_payload,
            br#"{"status":"down"}"#
        );

        projection
            .apply(runtime_state_entry(
                "room_1",
                source.clone(),
                "runtime.gateway",
                "finite.gateway.v1",
                1,
                11,
                br#"{"status":"restarted"}"#,
            ))
            .unwrap();
        assert_eq!(
            projection
                .get("room_1", &source, "runtime.gateway")
                .unwrap()
                .snapshot
                .status_payload,
            br#"{"status":"restarted"}"#
        );

        projection
            .apply(runtime_state_entry(
                "room_1",
                source.clone(),
                "runtime.gateway",
                "finite.gateway.v1",
                2,
                8,
                br#"{"status":"live"}"#,
            ))
            .unwrap();
        let current = projection
            .get("room_1", &source, "runtime.gateway")
            .unwrap();
        assert_eq!(current.snapshot.revision, 2);
        assert_eq!(current.accepted_seq, 8);
        assert_eq!(current.snapshot.status_payload, br#"{"status":"live"}"#);
    }

    #[test]
    fn runtime_state_projection_preserves_unknown_schema_and_expiry() {
        let source = device("runtime_npub", "runtime_box");
        let mut projection = RuntimeStateProjection::default();

        projection
            .apply(runtime_state_entry(
                "room_1",
                source.clone(),
                "runtime.capabilities",
                "vendor.future-schema.v9",
                1,
                10,
                br#"{"unrecognized":true}"#,
            ))
            .unwrap();

        let current = projection
            .get("room_1", &source, "runtime.capabilities")
            .unwrap();
        assert_eq!(current.snapshot.schema, "vendor.future-schema.v9");
        assert_eq!(current.snapshot.status_payload, br#"{"unrecognized":true}"#);
        assert!(!current.snapshot.is_expired_at(1_999));
        assert!(current.snapshot.is_expired_at(2_000));
    }

    #[derive(Debug, serde::Deserialize, PartialEq, Eq)]
    struct GatewayStatus {
        status: String,
    }

    #[test]
    fn runtime_state_projection_requires_fresh_matching_schema() {
        let source = device("runtime_npub", "runtime_box");
        let mut projection = RuntimeStateProjection::default();
        projection
            .apply(runtime_state_entry(
                "room_1",
                source.clone(),
                "runtime.gateway",
                "finitecomputer.runtime.gateway.status.v1",
                1,
                10,
                br#"{"status":"down"}"#,
            ))
            .unwrap();

        let status: GatewayStatus = projection
            .require_fresh_json(
                "room_1",
                &source,
                "runtime.gateway",
                "finitecomputer.runtime.gateway.status.v1",
                1_999,
            )
            .unwrap();

        assert_eq!(
            status,
            GatewayStatus {
                status: "down".to_string()
            }
        );
    }

    #[test]
    fn runtime_state_projection_fails_loudly_for_missing_stale_wrong_or_malformed_status() {
        let source = device("runtime_npub", "runtime_box");
        let mut projection = RuntimeStateProjection::default();
        assert_eq!(
            projection
                .require_fresh(
                    "room_1",
                    &source,
                    "runtime.gateway",
                    "finitecomputer.runtime.gateway.status.v1",
                    1_500,
                )
                .unwrap_err(),
            RuntimeStateProjectionError::Missing {
                room_id: "room_1".to_string(),
                source_device: source.clone(),
                state_key: "runtime.gateway".to_string(),
            }
        );

        projection
            .apply(runtime_state_entry(
                "room_1",
                source.clone(),
                "runtime.gateway",
                "finitecomputer.runtime.gateway.status.v1",
                1,
                10,
                br#"{"status":"down"}"#,
            ))
            .unwrap();

        assert!(matches!(
            projection
                .require_fresh(
                    "room_1",
                    &source,
                    "runtime.gateway",
                    "finitecomputer.runtime.gateway.status.v1",
                    2_000,
                )
                .unwrap_err(),
            RuntimeStateProjectionError::Expired { .. }
        ));
        assert!(matches!(
            projection
                .require_fresh(
                    "room_1",
                    &source,
                    "runtime.gateway",
                    "finitecomputer.runtime.gateway.status.v2",
                    1_500,
                )
                .unwrap_err(),
            RuntimeStateProjectionError::WrongSchema { .. }
        ));

        projection
            .apply(runtime_state_entry(
                "room_1",
                source.clone(),
                "runtime.gateway",
                "finitecomputer.runtime.gateway.status.v1",
                2,
                11,
                b"not json",
            ))
            .unwrap();
        let err = projection
            .require_fresh_json::<GatewayStatus>(
                "room_1",
                &source,
                "runtime.gateway",
                "finitecomputer.runtime.gateway.status.v1",
                1_500,
            )
            .unwrap_err();
        assert_eq!(
            err,
            RuntimeStateProjectionError::MalformedPayload {
                state_key: "runtime.gateway".to_string()
            }
        );
    }

    #[test]
    fn runtime_state_snapshot_rejects_empty_key_schema_or_payload() {
        let source = device("runtime_npub", "runtime_box");
        let mut projection = RuntimeStateProjection::default();

        for (state_key, schema, payload) in [
            (
                "",
                "finitecomputer.runtime.gateway.status.v1",
                b"{}".as_slice(),
            ),
            ("runtime.gateway", "", b"{}".as_slice()),
            (
                "runtime.gateway",
                "finitecomputer.runtime.gateway.status.v1",
                b"".as_slice(),
            ),
        ] {
            assert!(matches!(
                projection
                    .apply(runtime_state_entry(
                        "room_1",
                        source.clone(),
                        state_key,
                        schema,
                        1,
                        10,
                        payload,
                    ))
                    .unwrap_err(),
                ProtocolLimitError::BytesEmpty { .. }
            ));
        }
    }

    #[test]
    fn runtime_state_snapshot_refresh_cadence_is_bounded() {
        let source = device("runtime_npub", "runtime_box");
        let mut projection = RuntimeStateProjection::default();
        let expired_at_observation =
            runtime_state_entry_with_expiry("room_1", source.clone(), 1_000, 1_000);
        let too_slow = runtime_state_entry_with_expiry(
            "room_1",
            source,
            1_000,
            1_000 + MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS + 1,
        );

        assert!(matches!(
            projection.apply(expired_at_observation).unwrap_err(),
            ProtocolLimitError::InvalidTimeRange { .. }
        ));
        assert!(matches!(
            projection.apply(too_slow).unwrap_err(),
            ProtocolLimitError::DurationTooLong {
                max_millis: MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS,
                actual_millis,
                ..
            } if actual_millis == MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS + 1
        ));
        assert!(projection.is_empty());
    }

    #[test]
    fn runtime_command_request_validates_kind_body_and_target_policy() {
        let local_runtime = device("runtime_npub", "runtime_box");
        let account_target = RuntimeCommandTargetV1 {
            account_id: "runtime_npub".to_string(),
            device_id: None,
        };
        let device_target = RuntimeCommandTargetV1 {
            account_id: "runtime_npub".to_string(),
            device_id: Some("runtime_box".to_string()),
        };
        let other_device_target = RuntimeCommandTargetV1 {
            account_id: "runtime_npub".to_string(),
            device_id: Some("gpu_worker".to_string()),
        };

        let request = runtime_command_request(
            "restart_1",
            "finitecomputer.runtime.gateway.restart",
            account_target.clone(),
            br#"{}"#,
        );
        request.validate_structure().unwrap();
        assert!(account_target.matches_device(&local_runtime));
        assert!(device_target.matches_device(&local_runtime));
        assert!(!other_device_target.matches_device(&local_runtime));

        let mut wrong_kind = request.clone();
        wrong_kind.payload_kind = RuntimeCommandPayloadKindV1::Result;
        assert!(matches!(
            wrong_kind.validate_structure().unwrap_err(),
            RuntimeCommandPayloadError::WrongPayloadKind { .. }
        ));

        let mut empty_schema = request;
        empty_schema.body.schema.clear();
        assert!(matches!(
            empty_schema.validate_structure().unwrap_err(),
            RuntimeCommandPayloadError::Protocol(ProtocolLimitError::BytesEmpty { field })
                if field == "runtime_command.payload.schema"
        ));
    }

    #[test]
    fn runtime_command_result_requires_terminal_shape_and_bounded_clears() {
        let ok_result = RuntimeCommandResultV1 {
            payload_kind: RuntimeCommandPayloadKindV1::Result,
            request_id: "restart_1".to_string(),
            status: RuntimeCommandTerminalStatusV1::Succeeded,
            body: Some(runtime_command_body(br#"{"status":"ok"}"#)),
            error: None,
            clears_activity: vec![RuntimeActivityClearV1 {
                activity_kind: "working".to_string(),
                activity_id: Some("restart_1".to_string()),
                conversation_id: Some("topic_1".to_string()),
            }],
        };
        ok_result.validate_structure().unwrap();

        let missing_success_body = RuntimeCommandResultV1 {
            body: None,
            ..ok_result.clone()
        };
        assert!(matches!(
            missing_success_body.validate_structure().unwrap_err(),
            RuntimeCommandPayloadError::SuccessMissingBody { .. }
        ));

        let missing_failure_error = RuntimeCommandResultV1 {
            status: RuntimeCommandTerminalStatusV1::Failed,
            body: None,
            error: None,
            clears_activity: Vec::new(),
            ..ok_result.clone()
        };
        assert!(matches!(
            missing_failure_error.validate_structure().unwrap_err(),
            RuntimeCommandPayloadError::FailureMissingError { .. }
        ));

        let too_many_clears = RuntimeCommandResultV1 {
            clears_activity: vec![
                RuntimeActivityClearV1 {
                    activity_kind: "working".to_string(),
                    activity_id: None,
                    conversation_id: None,
                };
                MAX_RUNTIME_COMMAND_ACTIVITY_CLEARS as usize + 1
            ],
            ..ok_result
        };
        assert!(matches!(
            too_many_clears.validate_structure().unwrap_err(),
            RuntimeCommandPayloadError::Protocol(ProtocolLimitError::TooManyItems { field, .. })
                if field == "runtime_command.clears_activity"
        ));
    }

    #[test]
    fn runtime_command_ledger_records_after_decrypted_target_policy() {
        let sender = device("alice_npub", "dashboard");
        let local_runtime = device("runtime_npub", "runtime_box");
        let targeted = runtime_command_request(
            "restart_1",
            "finitecomputer.runtime.gateway.restart",
            RuntimeCommandTargetV1 {
                account_id: "runtime_npub".to_string(),
                device_id: Some("runtime_box".to_string()),
            },
            br#"{}"#,
        );
        let not_for_local_device = runtime_command_request(
            "restart_2",
            "finitecomputer.runtime.gateway.restart",
            RuntimeCommandTargetV1 {
                account_id: "runtime_npub".to_string(),
                device_id: Some("other_device".to_string()),
            },
            br#"{}"#,
        );
        let mut ledger = RuntimeCommandLedger::default();

        assert_eq!(
            ledger
                .record_request(
                    RuntimeCommandIngressContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        accepted_seq: 12,
                        original_message_id: "message_1",
                        sender: &sender,
                        local_device: &local_runtime,
                    },
                    &targeted,
                )
                .unwrap(),
            RuntimeCommandLedgerDecision::Recorded
        );
        assert_eq!(
            ledger
                .record_request(
                    RuntimeCommandIngressContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        accepted_seq: 12,
                        original_message_id: "message_1",
                        sender: &sender,
                        local_device: &local_runtime,
                    },
                    &targeted,
                )
                .unwrap(),
            RuntimeCommandLedgerDecision::Replayed
        );
        assert_eq!(
            ledger
                .record_request(
                    RuntimeCommandIngressContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        accepted_seq: 13,
                        original_message_id: "message_2",
                        sender: &sender,
                        local_device: &local_runtime,
                    },
                    &targeted,
                )
                .unwrap_err(),
            RuntimeCommandLedgerError::ConflictingRequestId {
                request_id: "restart_1".to_string()
            }
        );
        assert_eq!(
            ledger
                .record_request(
                    RuntimeCommandIngressContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        accepted_seq: 14,
                        original_message_id: "message_3",
                        sender: &sender,
                        local_device: &local_runtime,
                    },
                    &not_for_local_device,
                )
                .unwrap(),
            RuntimeCommandLedgerDecision::IgnoredTarget
        );
        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger.pending_requests().len(), 1);
    }

    #[test]
    fn runtime_config_commands_serialize_per_resource() {
        let sender = device("alice_npub", "dashboard");
        let local_runtime = device("runtime_npub", "runtime_box");
        let mut ledger = RuntimeCommandLedger::default();
        let target = RuntimeCommandTargetV1 {
            account_id: "runtime_npub".to_string(),
            device_id: Some("runtime_box".to_string()),
        };
        let mut apply_config = runtime_command_request(
            "apply_config",
            "finitecomputer.runtime.inference.apply",
            target.clone(),
            br#"{}"#,
        );
        apply_config.resource_key = Some("hermes.config".to_string());
        let mut rollback_config = runtime_command_request(
            "rollback_config",
            "finitecomputer.runtime.inference.rollback",
            target.clone(),
            br#"{}"#,
        );
        rollback_config.resource_key = Some("hermes.config".to_string());
        let mut restart_gateway = runtime_command_request(
            "restart_gateway",
            "finitecomputer.runtime.gateway.restart",
            target,
            br#"{}"#,
        );
        restart_gateway.resource_key = Some("gateway.process".to_string());

        for (seq, message_id, request) in [
            (11, "message_apply_config", &apply_config),
            (12, "message_restart_gateway", &restart_gateway),
            (13, "message_rollback_config", &rollback_config),
        ] {
            ledger
                .record_request(
                    RuntimeCommandIngressContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        accepted_seq: seq,
                        original_message_id: message_id,
                        sender: &sender,
                        local_device: &local_runtime,
                    },
                    request,
                )
                .unwrap();
        }

        let ready = ledger.ready_requests();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].request_id, "apply_config");
        assert_eq!(ready[1].request_id, "restart_gateway");

        ledger
            .apply_result(
                RuntimeCommandTerminalContext {
                    room_id: "room_1",
                    conversation_id: Some("topic_1"),
                    request_sender: &sender,
                    accepted_seq: 20,
                    terminal_message_id: "result_apply_config",
                },
                &runtime_command_success_result("apply_config", br#"{"status":"ok"}"#),
            )
            .unwrap();
        let ready = ledger.ready_requests();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].request_id, "restart_gateway");
        assert_eq!(ready[1].request_id, "rollback_config");
    }

    #[test]
    fn runtime_bridge_commands_serialize_per_physical_resource() {
        let sender = device("alice_npub", "dashboard");
        let local_runtime = device("runtime_npub", "runtime_box");
        let target = RuntimeCommandTargetV1 {
            account_id: "runtime_npub".to_string(),
            device_id: Some("runtime_box".to_string()),
        };
        let mut reconnect_telegram = runtime_command_request(
            "reconnect_telegram",
            "finitecomputer.runtime.connection.telegram.reconnect",
            target.clone(),
            br#"{}"#,
        );
        reconnect_telegram.resource_key = Some("connection.telegram".to_string());
        let mut reconnect_matrix = runtime_command_request(
            "reconnect_matrix",
            "finitecomputer.runtime.connection.matrix.reconnect",
            target.clone(),
            br#"{}"#,
        );
        reconnect_matrix.resource_key = Some("connection.matrix".to_string());
        let mut rotate_telegram = runtime_command_request(
            "rotate_telegram",
            "finitecomputer.runtime.connection.telegram.rotate",
            target,
            br#"{}"#,
        );
        rotate_telegram.resource_key = Some("connection.telegram".to_string());
        let mut ledger = RuntimeCommandLedger::default();

        for (seq, message_id, request) in [
            (11, "message_reconnect_telegram", &reconnect_telegram),
            (12, "message_reconnect_matrix", &reconnect_matrix),
            (13, "message_rotate_telegram", &rotate_telegram),
        ] {
            ledger
                .record_request(
                    RuntimeCommandIngressContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        accepted_seq: seq,
                        original_message_id: message_id,
                        sender: &sender,
                        local_device: &local_runtime,
                    },
                    request,
                )
                .unwrap();
        }

        let ready = ledger.ready_requests();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].request_id, "reconnect_telegram");
        assert_eq!(ready[1].request_id, "reconnect_matrix");

        ledger
            .apply_result(
                RuntimeCommandTerminalContext {
                    room_id: "room_1",
                    conversation_id: Some("topic_1"),
                    request_sender: &sender,
                    accepted_seq: 20,
                    terminal_message_id: "result_reconnect_telegram",
                },
                &runtime_command_success_result("reconnect_telegram", br#"{"status":"ok"}"#),
            )
            .unwrap();
        let ready = ledger.ready_requests();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].request_id, "reconnect_matrix");
        assert_eq!(ready[1].request_id, "rotate_telegram");
    }

    #[test]
    fn unkeyed_runtime_commands_do_not_block_keyed_resources() {
        let sender = device("alice_npub", "dashboard");
        let local_runtime = device("runtime_npub", "runtime_box");
        let target = RuntimeCommandTargetV1 {
            account_id: "runtime_npub".to_string(),
            device_id: Some("runtime_box".to_string()),
        };
        let mut unkeyed = runtime_command_request(
            "read_status",
            "finitecomputer.runtime.status.refresh",
            target.clone(),
            br#"{}"#,
        );
        unkeyed.resource_key = None;
        let keyed = runtime_command_request(
            "restart_gateway",
            "finitecomputer.runtime.gateway.restart",
            target,
            br#"{}"#,
        );
        let mut ledger = RuntimeCommandLedger::default();

        for (seq, message_id, request) in [
            (11, "message_read_status", &unkeyed),
            (12, "message_restart_gateway", &keyed),
        ] {
            ledger
                .record_request(
                    RuntimeCommandIngressContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        accepted_seq: seq,
                        original_message_id: message_id,
                        sender: &sender,
                        local_device: &local_runtime,
                    },
                    request,
                )
                .unwrap();
        }

        let ready = ledger.ready_requests();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].request_id, "read_status");
        assert_eq!(ready[1].request_id, "restart_gateway");
    }

    #[test]
    fn runtime_command_result_is_idempotent_terminal_event() {
        let sender = device("alice_npub", "dashboard");
        let local_runtime = device("runtime_npub", "runtime_box");
        let mut ledger = RuntimeCommandLedger::default();
        let request = runtime_command_request(
            "restart_1",
            "finitecomputer.runtime.gateway.restart",
            RuntimeCommandTargetV1 {
                account_id: "runtime_npub".to_string(),
                device_id: Some("runtime_box".to_string()),
            },
            br#"{}"#,
        );
        ledger
            .record_request(
                RuntimeCommandIngressContext {
                    room_id: "room_1",
                    conversation_id: Some("topic_1"),
                    accepted_seq: 12,
                    original_message_id: "request_message_1",
                    sender: &sender,
                    local_device: &local_runtime,
                },
                &request,
            )
            .unwrap();
        let result = runtime_command_success_result("restart_1", br#"{"status":"live"}"#);

        assert_eq!(
            ledger
                .apply_result(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 20,
                        terminal_message_id: "result_message_1",
                    },
                    &result,
                )
                .unwrap(),
            RuntimeCommandTerminalDecision::Recorded
        );
        assert_eq!(
            ledger
                .apply_result(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 20,
                        terminal_message_id: "result_message_1",
                    },
                    &result,
                )
                .unwrap(),
            RuntimeCommandTerminalDecision::Replayed
        );
        assert_eq!(
            ledger
                .apply_result(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 21,
                        terminal_message_id: "result_message_late",
                    },
                    &result,
                )
                .unwrap(),
            RuntimeCommandTerminalDecision::IgnoredAlreadyTerminal
        );
        assert!(ledger.pending_requests().is_empty());
        let record = ledger
            .get("room_1", Some("topic_1"), &sender, "restart_1")
            .unwrap();
        assert_eq!(record.status, RuntimeCommandLedgerStatus::Succeeded);
        assert_eq!(record.terminal_seq, Some(20));
        assert_eq!(
            record.terminal_message_id.as_deref(),
            Some("result_message_1")
        );
    }

    #[test]
    fn runtime_command_cancel_races_with_result_first_terminal_wins() {
        let sender = device("alice_npub", "dashboard");
        let local_runtime = device("runtime_npub", "runtime_box");
        let mut ledger = RuntimeCommandLedger::default();

        for request_id in ["restart_cancel_first", "restart_result_first"] {
            let request = runtime_command_request(
                request_id,
                "finitecomputer.runtime.gateway.restart",
                RuntimeCommandTargetV1 {
                    account_id: "runtime_npub".to_string(),
                    device_id: Some("runtime_box".to_string()),
                },
                br#"{}"#,
            );
            ledger
                .record_request(
                    RuntimeCommandIngressContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        accepted_seq: 12,
                        original_message_id: request_id,
                        sender: &sender,
                        local_device: &local_runtime,
                    },
                    &request,
                )
                .unwrap();
        }

        let cancel = runtime_command_cancel("restart_cancel_first");
        let result =
            runtime_command_success_result("restart_cancel_first", br#"{"status":"live"}"#);
        assert_eq!(
            ledger
                .apply_cancel(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 20,
                        terminal_message_id: "cancel_message_1",
                    },
                    &cancel,
                )
                .unwrap(),
            RuntimeCommandTerminalDecision::Recorded
        );
        assert_eq!(
            ledger
                .apply_result(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 21,
                        terminal_message_id: "result_message_late",
                    },
                    &result,
                )
                .unwrap(),
            RuntimeCommandTerminalDecision::IgnoredAlreadyTerminal
        );
        assert_eq!(
            ledger
                .get("room_1", Some("topic_1"), &sender, "restart_cancel_first")
                .unwrap()
                .status,
            RuntimeCommandLedgerStatus::Cancelled
        );

        let result =
            runtime_command_success_result("restart_result_first", br#"{"status":"live"}"#);
        let cancel = runtime_command_cancel("restart_result_first");
        assert_eq!(
            ledger
                .apply_result(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 30,
                        terminal_message_id: "result_message_1",
                    },
                    &result,
                )
                .unwrap(),
            RuntimeCommandTerminalDecision::Recorded
        );
        assert_eq!(
            ledger
                .apply_cancel(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 31,
                        terminal_message_id: "cancel_message_late",
                    },
                    &cancel,
                )
                .unwrap(),
            RuntimeCommandTerminalDecision::IgnoredAlreadyTerminal
        );
        let record = ledger
            .get("room_1", Some("topic_1"), &sender, "restart_result_first")
            .unwrap();
        assert_eq!(record.status, RuntimeCommandLedgerStatus::Succeeded);
        assert_eq!(record.terminal_seq, Some(30));
        assert_eq!(
            record.terminal_message_id.as_deref(),
            Some("result_message_1")
        );
    }

    #[test]
    fn runtime_command_cancel_validates_kind_reason_and_known_request() {
        let sender = device("alice_npub", "dashboard");
        let mut ledger = RuntimeCommandLedger::default();
        let mut wrong_kind = runtime_command_cancel("restart_1");
        wrong_kind.payload_kind = RuntimeCommandPayloadKindV1::Result;

        assert!(matches!(
            ledger
                .apply_cancel(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 20,
                        terminal_message_id: "cancel_message_1",
                    },
                    &wrong_kind,
                )
                .unwrap_err(),
            RuntimeCommandLedgerError::Payload(RuntimeCommandPayloadError::WrongPayloadKind { .. })
        ));

        let mut bad_reason = runtime_command_cancel("restart_1");
        bad_reason.reason = Some(String::new());
        assert!(matches!(
            ledger
                .apply_cancel(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 20,
                        terminal_message_id: "cancel_message_1",
                    },
                    &bad_reason,
                )
                .unwrap_err(),
            RuntimeCommandLedgerError::Payload(RuntimeCommandPayloadError::Protocol(
                ProtocolLimitError::BytesEmpty { .. }
            ))
        ));

        assert_eq!(
            ledger
                .apply_cancel(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 20,
                        terminal_message_id: "cancel_message_1",
                    },
                    &runtime_command_cancel("restart_1"),
                )
                .unwrap_err(),
            RuntimeCommandLedgerError::RequestNotFound {
                request_id: "restart_1".to_string()
            }
        );
    }

    #[test]
    fn runtime_command_terminal_event_must_follow_request_sequence() {
        let sender = device("alice_npub", "dashboard");
        let local_runtime = device("runtime_npub", "runtime_box");
        let mut ledger = RuntimeCommandLedger::default();
        let request = runtime_command_request(
            "restart_1",
            "finitecomputer.runtime.gateway.restart",
            RuntimeCommandTargetV1 {
                account_id: "runtime_npub".to_string(),
                device_id: Some("runtime_box".to_string()),
            },
            br#"{}"#,
        );
        ledger
            .record_request(
                RuntimeCommandIngressContext {
                    room_id: "room_1",
                    conversation_id: Some("topic_1"),
                    accepted_seq: 12,
                    original_message_id: "request_message_1",
                    sender: &sender,
                    local_device: &local_runtime,
                },
                &request,
            )
            .unwrap();

        assert_eq!(
            ledger
                .apply_result(
                    RuntimeCommandTerminalContext {
                        room_id: "room_1",
                        conversation_id: Some("topic_1"),
                        request_sender: &sender,
                        accepted_seq: 12,
                        terminal_message_id: "result_message_1",
                    },
                    &runtime_command_success_result("restart_1", br#"{"status":"live"}"#),
                )
                .unwrap_err(),
            RuntimeCommandLedgerError::TerminalBeforeRequest {
                request_id: "restart_1".to_string(),
                request_seq: 12,
                terminal_seq: 12,
            }
        );
    }

    #[test]
    fn activity_projection_keeps_devices_separate_and_clear_scoped() {
        let phone = device("alice_npub", "phone");
        let laptop = device("alice_npub", "laptop");
        let mut projection = EphemeralActivityProjection::default();

        projection
            .apply(
                activity_context("room_1", Some("topic_1"), &phone, 1_000, 11_000),
                &activity_set("typing", None, br#"{"chars":3}"#),
            )
            .unwrap();
        projection
            .apply(
                activity_context("room_1", Some("topic_1"), &laptop, 1_000, 11_000),
                &activity_set("typing", None, br#"{"chars":1}"#),
            )
            .unwrap();

        assert_eq!(projection.len(), 2);
        assert_eq!(
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), &phone, 2_000, 12_000),
                    &activity_clear("typing", None),
                )
                .unwrap(),
            EphemeralActivityProjectionDecision::Cleared
        );
        assert!(
            projection
                .get("room_1", Some("topic_1"), &phone, "typing", None)
                .is_none()
        );
        assert!(
            projection
                .get("room_1", Some("topic_1"), &laptop, "typing", None)
                .is_some()
        );
    }

    #[test]
    fn activity_refresh_extends_matching_device_expiry() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();

        assert_eq!(
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), &runtime, 1_000, 11_000),
                    &activity_set("working", Some("run_1"), br#"{"pct":10}"#),
                )
                .unwrap(),
            EphemeralActivityProjectionDecision::Set
        );
        assert_eq!(
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), &runtime, 2_000, 22_000),
                    &activity_set("working", Some("run_1"), br#"{"pct":20}"#),
                )
                .unwrap(),
            EphemeralActivityProjectionDecision::Refreshed
        );
        let current = projection
            .get(
                "room_1",
                Some("topic_1"),
                &runtime,
                "working",
                Some("run_1"),
            )
            .unwrap();
        assert_eq!(current.expires_at_ms, 22_000);
        assert_eq!(current.payload, br#"{"pct":20}"#);
    }

    #[test]
    fn long_running_agent_activity_survives_refresh_without_push() {
        let runtime = device("runtime_npub", "box");
        let run_id = "run_restart_gateway_1";
        let mut projection = EphemeralActivityProjection::default();

        assert_eq!(
            projection
                .apply(
                    activity_context(
                        "room_1",
                        Some("topic_1"),
                        &runtime,
                        1_000,
                        1_000 + MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS,
                    ),
                    &activity_set("working", Some(run_id), br#"{"phase":"restart"}"#),
                )
                .unwrap(),
            EphemeralActivityProjectionDecision::Set
        );
        assert_eq!(
            projection
                .apply(
                    activity_context(
                        "room_1",
                        Some("topic_1"),
                        &runtime,
                        20 * 60 * 1_000,
                        50 * 60 * 1_000,
                    ),
                    &activity_set("working", Some(run_id), br#"{"phase":"waiting"}"#),
                )
                .unwrap(),
            EphemeralActivityProjectionDecision::Refreshed
        );
        let current = projection
            .get("room_1", Some("topic_1"), &runtime, "working", Some(run_id))
            .unwrap();

        assert_eq!(projection.len(), 1);
        assert_eq!(current.activity_id, run_id);
        assert_eq!(current.expires_at_ms, 50 * 60 * 1_000);
        assert_eq!(current.payload, br#"{"phase":"waiting"}"#);
        assert_eq!(projection.expire_at(50 * 60 * 1_000 - 1).unwrap(), 0);
        assert_eq!(projection.expire_at(50 * 60 * 1_000).unwrap(), 1);
        assert!(projection.is_empty());
    }

    #[test]
    fn durable_terminal_clear_is_sender_and_activity_scoped() {
        let runtime = device("runtime_npub", "box");
        let sibling = device("runtime_npub", "gpu");
        let mut projection = EphemeralActivityProjection::default();
        for sender in [&runtime, &sibling] {
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), sender, 1_000, 11_000),
                    &activity_set("working", Some("restart_1"), br#"{}"#),
                )
                .unwrap();
        }

        let removed = projection
            .clear_from_durable_terminal(
                "room_1",
                Some("topic_1"),
                &runtime,
                &RuntimeActivityClearV1 {
                    activity_kind: "working".to_string(),
                    activity_id: Some("restart_1".to_string()),
                    conversation_id: None,
                },
            )
            .unwrap();

        assert!(removed);
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    "working",
                    Some("restart_1"),
                )
                .is_none()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &sibling,
                    "working",
                    Some("restart_1"),
                )
                .is_some()
        );
    }

    #[test]
    fn activity_clear_does_not_remove_unrelated_kind() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();
        for kind in ["typing", "working"] {
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), &runtime, 1_000, 11_000),
                    &activity_set(kind, Some("shared_id"), br#"{}"#),
                )
                .unwrap();
        }

        assert_eq!(
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), &runtime, 2_000, 12_000),
                    &activity_clear("typing", Some("shared_id")),
                )
                .unwrap(),
            EphemeralActivityProjectionDecision::Cleared
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    "typing",
                    Some("shared_id"),
                )
                .is_none()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    "working",
                    Some("shared_id"),
                )
                .is_some()
        );
    }

    #[test]
    fn activity_clear_does_not_remove_different_activity_id() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();
        for activity_id in ["run_1", "run_2"] {
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), &runtime, 1_000, 11_000),
                    &activity_set("working", Some(activity_id), br#"{}"#),
                )
                .unwrap();
        }

        assert_eq!(
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), &runtime, 2_000, 12_000),
                    &activity_clear("working", Some("run_1")),
                )
                .unwrap(),
            EphemeralActivityProjectionDecision::Cleared
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    "working",
                    Some("run_1"),
                )
                .is_none()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    "working",
                    Some("run_2"),
                )
                .is_some()
        );
    }

    #[test]
    fn stale_agent_activity_clear_does_not_hide_newer_run() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();
        projection
            .apply(
                activity_context("room_1", Some("topic_1"), &runtime, 2_000, 12_000),
                &activity_set("working", Some("run_1"), br#"{"phase":"new"}"#),
            )
            .unwrap();

        assert_eq!(
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), &runtime, 1_500, 11_500),
                    &activity_clear("working", Some("run_1")),
                )
                .unwrap(),
            EphemeralActivityProjectionDecision::ClearMiss
        );
        let current = projection
            .get(
                "room_1",
                Some("topic_1"),
                &runtime,
                "working",
                Some("run_1"),
            )
            .unwrap();
        assert_eq!(current.received_at_ms, 2_000);
        assert_eq!(current.payload, br#"{"phase":"new"}"#);
    }

    #[test]
    fn conversation_id_does_not_authorize_cross_room_activity() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();
        for room_id in ["room_1", "room_2"] {
            projection
                .apply(
                    activity_context(room_id, Some("topic_1"), &runtime, 1_000, 11_000),
                    &activity_set("present", None, br#"{}"#),
                )
                .unwrap();
        }

        assert_eq!(
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), &runtime, 2_000, 12_000),
                    &activity_clear("present", None),
                )
                .unwrap(),
            EphemeralActivityProjectionDecision::Cleared
        );
        assert!(
            projection
                .get("room_1", Some("topic_1"), &runtime, "present", None)
                .is_none()
        );
        assert!(
            projection
                .get("room_2", Some("topic_1"), &runtime, "present", None)
                .is_some()
        );
    }

    #[test]
    fn reserved_activity_kinds_render_generically() {
        let reserved = [
            (
                FINITECHAT_ACTIVITY_KIND_TYPING,
                GenericActivityKindV1::Typing,
                FINITECHAT_ACTIVITY_TYPING_EXPIRY_MILLIS,
            ),
            (
                FINITECHAT_ACTIVITY_KIND_THINKING,
                GenericActivityKindV1::Thinking,
                FINITECHAT_ACTIVITY_WORKING_EXPIRY_MILLIS,
            ),
            (
                FINITECHAT_ACTIVITY_KIND_WORKING,
                GenericActivityKindV1::Working,
                FINITECHAT_ACTIVITY_WORKING_EXPIRY_MILLIS,
            ),
            (
                FINITECHAT_ACTIVITY_KIND_UPLOADING,
                GenericActivityKindV1::Uploading,
                FINITECHAT_ACTIVITY_PRESENT_EXPIRY_MILLIS,
            ),
            (
                FINITECHAT_ACTIVITY_KIND_RECORDING,
                GenericActivityKindV1::Recording,
                FINITECHAT_ACTIVITY_PRESENT_EXPIRY_MILLIS,
            ),
            (
                FINITECHAT_ACTIVITY_KIND_PRESENT,
                GenericActivityKindV1::Present,
                FINITECHAT_ACTIVITY_PRESENT_EXPIRY_MILLIS,
            ),
        ];

        for (activity_kind, expected_kind, expected_expiry_millis) in reserved {
            let generic_kind = generic_activity_kind_v1(activity_kind).unwrap().unwrap();
            assert_eq!(generic_kind, expected_kind);
            assert_eq!(generic_kind.as_str(), activity_kind);
            assert_eq!(
                generic_kind.recommended_expiry_millis(),
                expected_expiry_millis
            );
        }
    }

    #[test]
    fn unknown_namespaced_activity_kind_is_preserved() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();

        projection
            .apply(
                activity_context("room_1", Some("topic_1"), &runtime, 1_000, 11_000),
                &activity_set("finitecomputer.indexing", Some("job_1"), br#"{"pct":50}"#),
            )
            .unwrap();

        let current = projection
            .get(
                "room_1",
                Some("topic_1"),
                &runtime,
                "finitecomputer.indexing",
                Some("job_1"),
            )
            .unwrap();
        assert_eq!(current.activity_kind, "finitecomputer.indexing");
        assert_eq!(current.payload, br#"{"pct":50}"#);
    }

    #[test]
    fn app_specific_activity_kind_does_not_trigger_generic_ui() {
        for activity_kind in ["finitecomputer.indexing", "hermes.tool_calling"] {
            assert!(generic_activity_kind_v1(activity_kind).unwrap().is_none());
        }
    }

    #[test]
    fn present_without_conversation_id_is_room_scoped() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();

        projection
            .apply(
                activity_context("room_1", None, &runtime, 1_000, 11_000),
                &activity_set(FINITECHAT_ACTIVITY_KIND_PRESENT, None, br#"{}"#),
            )
            .unwrap();

        assert!(
            projection
                .get(
                    "room_1",
                    None,
                    &runtime,
                    FINITECHAT_ACTIVITY_KIND_PRESENT,
                    None
                )
                .is_some()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    FINITECHAT_ACTIVITY_KIND_PRESENT,
                    None,
                )
                .is_none()
        );
    }

    #[test]
    fn present_with_conversation_id_is_conversation_scoped() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();

        projection
            .apply(
                activity_context("room_1", Some("topic_1"), &runtime, 1_000, 11_000),
                &activity_set(FINITECHAT_ACTIVITY_KIND_PRESENT, None, br#"{}"#),
            )
            .unwrap();

        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    FINITECHAT_ACTIVITY_KIND_PRESENT,
                    None,
                )
                .is_some()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    None,
                    &runtime,
                    FINITECHAT_ACTIVITY_KIND_PRESENT,
                    None
                )
                .is_none()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_2"),
                    &runtime,
                    FINITECHAT_ACTIVITY_KIND_PRESENT,
                    None,
                )
                .is_none()
        );
    }

    #[test]
    fn activity_default_expiry_guidance_stays_within_v1_cap() {
        for generic_kind in [
            GenericActivityKindV1::Typing,
            GenericActivityKindV1::Thinking,
            GenericActivityKindV1::Working,
            GenericActivityKindV1::Uploading,
            GenericActivityKindV1::Recording,
            GenericActivityKindV1::Present,
        ] {
            let expiry_millis = generic_kind.recommended_expiry_millis();
            assert!(expiry_millis > 0);
            assert!(expiry_millis <= MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS);
        }
    }

    #[test]
    fn activity_projection_rolls_up_identity_for_normal_ui() {
        let phone = device("alice_npub", "phone");
        let laptop = device("alice_npub", "laptop");
        let bob_phone = device("bob_npub", "phone");
        let mut projection = EphemeralActivityProjection::default();
        for sender in [&phone, &laptop, &bob_phone] {
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), sender, 1_000, 11_000),
                    &activity_set(FINITECHAT_ACTIVITY_KIND_TYPING, None, br#"{}"#),
                )
                .unwrap();
        }

        let mut entries = Vec::with_capacity(2);
        let count = projection
            .collect_account_activity(
                "room_1",
                Some("topic_1"),
                "alice_npub",
                FINITECHAT_ACTIVITY_KIND_TYPING,
                None,
                &mut entries,
            )
            .unwrap();
        let device_ids = entries
            .iter()
            .map(|entry| entry.sender.device_id.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(count, 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(device_ids, BTreeSet::from(["laptop", "phone"]));
        assert_eq!(projection.len(), 3);
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &bob_phone,
                    FINITECHAT_ACTIVITY_KIND_TYPING,
                    None,
                )
                .is_some()
        );
    }

    #[test]
    fn durable_chat_message_clears_matching_default_typing() {
        let phone = device("alice_npub", "phone");
        let laptop = device("alice_npub", "laptop");
        let mut projection = EphemeralActivityProjection::default();
        for sender in [&phone, &laptop] {
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), sender, 1_000, 11_000),
                    &activity_set(FINITECHAT_ACTIVITY_KIND_TYPING, None, br#"{}"#),
                )
                .unwrap();
        }
        projection
            .apply(
                activity_context("room_1", Some("topic_1"), &phone, 1_000, 11_000),
                &activity_set(FINITECHAT_ACTIVITY_KIND_WORKING, Some("run_1"), br#"{}"#),
            )
            .unwrap();

        assert_eq!(
            projection
                .clear_from_durable_application_event(
                    "room_1",
                    &phone,
                    &application_event(DurableAppEventKind::ChatMessage, Some("topic_1"), b"hi"),
                )
                .unwrap(),
            1
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &phone,
                    FINITECHAT_ACTIVITY_KIND_TYPING,
                    None,
                )
                .is_none()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &laptop,
                    FINITECHAT_ACTIVITY_KIND_TYPING,
                    None,
                )
                .is_some()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &phone,
                    FINITECHAT_ACTIVITY_KIND_WORKING,
                    Some("run_1"),
                )
                .is_some()
        );
    }

    #[test]
    fn durable_command_result_clears_matching_working_activity() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();
        projection
            .apply(
                activity_context("room_1", Some("topic_1"), &runtime, 1_000, 11_000),
                &activity_set(
                    FINITECHAT_ACTIVITY_KIND_WORKING,
                    Some("restart_1"),
                    br#"{}"#,
                ),
            )
            .unwrap();
        let mut result = runtime_command_success_result("restart_1", br#"{"status":"live"}"#);
        result.clears_activity.push(RuntimeActivityClearV1 {
            activity_kind: FINITECHAT_ACTIVITY_KIND_WORKING.to_string(),
            activity_id: Some("restart_1".to_string()),
            conversation_id: None,
        });

        assert_eq!(
            projection
                .clear_from_runtime_command_result("room_1", Some("topic_1"), &runtime, &result)
                .unwrap(),
            1
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    FINITECHAT_ACTIVITY_KIND_WORKING,
                    Some("restart_1"),
                )
                .is_none()
        );
    }

    #[test]
    fn dropped_ephemeral_clear_is_repaired_by_durable_terminal_event() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();
        projection
            .apply(
                activity_context("room_1", Some("topic_1"), &runtime, 1_000, 11_000),
                &activity_set(
                    FINITECHAT_ACTIVITY_KIND_WORKING,
                    Some("restart_1"),
                    br#"{}"#,
                ),
            )
            .unwrap();
        let mut result = runtime_command_success_result("restart_1", br#"{"status":"live"}"#);
        result.clears_activity.push(RuntimeActivityClearV1 {
            activity_kind: FINITECHAT_ACTIVITY_KIND_WORKING.to_string(),
            activity_id: Some("restart_1".to_string()),
            conversation_id: None,
        });

        assert_eq!(
            projection
                .clear_from_runtime_command_result("room_1", Some("topic_1"), &runtime, &result)
                .unwrap(),
            1
        );
        assert!(projection.is_empty());
    }

    #[test]
    fn durable_terminal_clear_is_sender_scoped() {
        let runtime = device("runtime_npub", "box");
        let sibling = device("runtime_npub", "gpu");
        let mut projection = EphemeralActivityProjection::default();
        for sender in [&runtime, &sibling] {
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), sender, 1_000, 11_000),
                    &activity_set(
                        FINITECHAT_ACTIVITY_KIND_WORKING,
                        Some("restart_1"),
                        br#"{}"#,
                    ),
                )
                .unwrap();
        }

        assert!(
            projection
                .clear_from_durable_terminal(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    &RuntimeActivityClearV1 {
                        activity_kind: FINITECHAT_ACTIVITY_KIND_WORKING.to_string(),
                        activity_id: Some("restart_1".to_string()),
                        conversation_id: None,
                    },
                )
                .unwrap()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &sibling,
                    FINITECHAT_ACTIVITY_KIND_WORKING,
                    Some("restart_1"),
                )
                .is_some()
        );
    }

    #[test]
    fn durable_terminal_clear_does_not_remove_different_activity_id() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();
        for activity_id in ["restart_1", "restart_2"] {
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), &runtime, 1_000, 11_000),
                    &activity_set(
                        FINITECHAT_ACTIVITY_KIND_WORKING,
                        Some(activity_id),
                        br#"{}"#,
                    ),
                )
                .unwrap();
        }

        assert!(
            projection
                .clear_from_durable_terminal(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    &RuntimeActivityClearV1 {
                        activity_kind: FINITECHAT_ACTIVITY_KIND_WORKING.to_string(),
                        activity_id: Some("restart_1".to_string()),
                        conversation_id: None,
                    },
                )
                .unwrap()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    FINITECHAT_ACTIVITY_KIND_WORKING,
                    Some("restart_2"),
                )
                .is_some()
        );
    }

    #[test]
    fn runtime_command_result_clears_matching_activity() {
        let runtime = device("runtime_npub", "box");
        let sibling = device("runtime_npub", "gpu");
        let mut projection = EphemeralActivityProjection::default();
        for (sender, activity_id) in [
            (&runtime, "restart_1"),
            (&runtime, "restart_2"),
            (&sibling, "restart_1"),
        ] {
            projection
                .apply(
                    activity_context("room_1", Some("topic_1"), sender, 1_000, 11_000),
                    &activity_set("working", Some(activity_id), br#"{}"#),
                )
                .unwrap();
        }
        let mut result = runtime_command_success_result("restart_1", br#"{"status":"live"}"#);
        result.clears_activity.push(RuntimeActivityClearV1 {
            activity_kind: "working".to_string(),
            activity_id: Some("restart_1".to_string()),
            conversation_id: None,
        });

        assert_eq!(
            projection
                .clear_from_runtime_command_result("room_1", Some("topic_1"), &runtime, &result,)
                .unwrap(),
            1
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    "working",
                    Some("restart_1"),
                )
                .is_none()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    "working",
                    Some("restart_2"),
                )
                .is_some()
        );
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &sibling,
                    "working",
                    Some("restart_1"),
                )
                .is_some()
        );
    }

    #[test]
    fn runtime_command_result_clear_rejects_invalid_result_before_mutation() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();
        projection
            .apply(
                activity_context("room_1", Some("topic_1"), &runtime, 1_000, 11_000),
                &activity_set("working", Some("restart_1"), br#"{}"#),
            )
            .unwrap();
        let mut result = runtime_command_success_result("restart_1", br#"{"status":"live"}"#);
        result.payload_kind = RuntimeCommandPayloadKindV1::Request;
        result.clears_activity.push(RuntimeActivityClearV1 {
            activity_kind: "working".to_string(),
            activity_id: Some("restart_1".to_string()),
            conversation_id: None,
        });

        assert!(matches!(
            projection
                .clear_from_runtime_command_result("room_1", Some("topic_1"), &runtime, &result,)
                .unwrap_err(),
            RuntimeCommandActivityClearError::Payload(
                RuntimeCommandPayloadError::WrongPayloadKind { .. }
            )
        ));
        assert!(
            projection
                .get(
                    "room_1",
                    Some("topic_1"),
                    &runtime,
                    "working",
                    Some("restart_1"),
                )
                .is_some()
        );
    }

    #[test]
    fn activity_projection_expires_and_rejects_bad_lease_windows() {
        let runtime = device("runtime_npub", "box");
        let mut projection = EphemeralActivityProjection::default();
        projection
            .apply(
                activity_context("room_1", None, &runtime, 1_000, 11_000),
                &activity_set("finitecomputer.indexing", Some("job_1"), br#"{}"#),
            )
            .unwrap();

        assert_eq!(projection.expire_at(10_999).unwrap(), 0);
        assert_eq!(projection.expire_at(11_000).unwrap(), 1);
        assert!(projection.is_empty());

        assert_eq!(
            projection
                .apply(
                    activity_context("room_1", None, &runtime, 1_000, 1_000),
                    &activity_set("thinking", None, br#"{}"#),
                )
                .unwrap_err(),
            EphemeralActivityProjectionError::AlreadyExpired
        );
        assert!(matches!(
            projection
                .apply(
                    activity_context(
                        "room_1",
                        None,
                        &runtime,
                        1_000,
                        1_001 + MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS,
                    ),
                    &activity_set("thinking", None, br#"{}"#),
                )
                .unwrap_err(),
            EphemeralActivityProjectionError::ExpiryTooLong { .. }
        ));
    }

    #[test]
    fn topic_message_routes_by_conversation_id() {
        let mut projection = ConversationProjection::default();
        projection
            .apply_event(
                conversation_context("room_1", 1, Some("topic_agent")),
                &application_event(
                    DurableAppEventKind::ConversationCreate,
                    Some("topic_agent"),
                    b"{}",
                ),
            )
            .unwrap();

        let decision = projection
            .apply_event(
                conversation_context("room_1", 2, Some("topic_agent")),
                &application_event(
                    DurableAppEventKind::ChatMessage,
                    Some("topic_agent"),
                    b"hello",
                ),
            )
            .unwrap();
        let topic = projection.get("room_1", "topic_agent").unwrap();

        assert_eq!(decision, ConversationProjectionDecision::Updated);
        assert_eq!(topic.created_seq, 1);
        assert_eq!(topic.updated_seq, 2);
        assert!(!topic.archived);
    }

    #[test]
    fn first_message_lazily_materializes_missing_conversation() {
        let mut projection = ConversationProjection::default();

        let decision = projection
            .apply_event(
                conversation_context("room_1", 7, Some("topic_new")),
                &application_event(
                    DurableAppEventKind::ChatMessage,
                    Some("topic_new"),
                    b"hello",
                ),
            )
            .unwrap();
        let topic = projection.get("room_1", "topic_new").unwrap();

        assert_eq!(decision, ConversationProjectionDecision::CreatedByMessage);
        assert_eq!(projection.len(), 1);
        assert_eq!(topic.created_seq, 7);
        assert_eq!(topic.updated_seq, 7);
    }

    #[test]
    fn topic_create_is_conversation_create_with_topic_metadata() {
        let mut projection = ConversationProjection::default();
        let metadata = conversation_metadata_payload(ConversationMetadataV1 {
            title: Some("Ops".to_string()),
            description: Some("Runtime operations".to_string()),
            external_topic: Some(ConversationExternalTopicV1 {
                platform: "telegram".to_string(),
                chat_id: "-100123".to_string(),
                thread_id: Some("42".to_string()),
                topic_name: Some("Ops".to_string()),
            }),
            skill_binding: Some(ConversationSkillBindingV1 {
                namespace: "finitecomputer".to_string(),
                skill_id: "runtime-admin".to_string(),
            }),
        });

        let decision = projection
            .apply_event(
                conversation_context("room_1", 3, Some("topic_imported_telegram")),
                &application_event(
                    DurableAppEventKind::ConversationCreate,
                    Some("topic_imported_telegram"),
                    &metadata,
                ),
            )
            .unwrap();
        let topic = projection.get("room_1", "topic_imported_telegram").unwrap();
        let metadata = topic.metadata.as_ref().unwrap();

        assert_eq!(decision, ConversationProjectionDecision::Created);
        assert_eq!(topic.created_seq, 3);
        assert_eq!(topic.updated_seq, 3);
        assert_eq!(metadata.title.as_deref(), Some("Ops"));
        assert_eq!(
            metadata.external_topic.as_ref().unwrap().platform,
            "telegram"
        );
        assert_eq!(
            metadata.skill_binding.as_ref().unwrap().skill_id,
            "runtime-admin"
        );
    }

    #[test]
    fn telegram_thread_id_imports_to_topic_conversation_id() {
        let mut projection = ConversationProjection::default();
        let metadata = conversation_metadata_payload(ConversationMetadataV1 {
            title: Some("Telegram Topic".to_string()),
            description: None,
            external_topic: Some(ConversationExternalTopicV1 {
                platform: "telegram".to_string(),
                chat_id: "-100123".to_string(),
                thread_id: Some("31337".to_string()),
                topic_name: Some("Deploys".to_string()),
            }),
            skill_binding: None,
        });

        projection
            .apply_event(
                conversation_context("room_1", 5, Some("topic_finite_31337")),
                &application_event(
                    DurableAppEventKind::ConversationCreate,
                    Some("topic_finite_31337"),
                    &metadata,
                ),
            )
            .unwrap();
        let topic = projection.get("room_1", "topic_finite_31337").unwrap();
        let external = topic
            .metadata
            .as_ref()
            .unwrap()
            .external_topic
            .as_ref()
            .unwrap();

        assert_eq!(topic.conversation_id, "topic_finite_31337");
        assert_ne!(
            topic.conversation_id,
            external.thread_id.as_deref().unwrap()
        );
        assert_eq!(external.platform, "telegram");
        assert_eq!(external.chat_id, "-100123");
        assert_eq!(external.thread_id.as_deref(), Some("31337"));
        assert_eq!(external.topic_name.as_deref(), Some("Deploys"));
    }

    #[test]
    fn topic_skill_binding_is_encrypted_conversation_metadata() {
        let mut projection = ConversationProjection::default();
        let create = conversation_metadata_payload(ConversationMetadataV1 {
            title: Some("Coding".to_string()),
            description: None,
            external_topic: None,
            skill_binding: Some(ConversationSkillBindingV1 {
                namespace: "finitecomputer".to_string(),
                skill_id: "code-review".to_string(),
            }),
        });
        let update = conversation_metadata_payload(ConversationMetadataV1 {
            title: Some("Coding".to_string()),
            description: Some("Use implementation mode".to_string()),
            external_topic: None,
            skill_binding: Some(ConversationSkillBindingV1 {
                namespace: "finitecomputer".to_string(),
                skill_id: "implementation".to_string(),
            }),
        });

        projection
            .apply_event(
                conversation_context("room_1", 6, Some("topic_code")),
                &application_event(
                    DurableAppEventKind::ConversationCreate,
                    Some("topic_code"),
                    &create,
                ),
            )
            .unwrap();
        projection
            .apply_event(
                conversation_context("room_1", 7, Some("topic_code")),
                &application_event(
                    DurableAppEventKind::ConversationUpdate,
                    Some("topic_code"),
                    &update,
                ),
            )
            .unwrap();
        let topic = projection.get("room_1", "topic_code").unwrap();
        let binding = topic
            .metadata
            .as_ref()
            .unwrap()
            .skill_binding
            .as_ref()
            .unwrap();

        assert_eq!(topic.updated_seq, 7);
        assert_eq!(binding.namespace, "finitecomputer");
        assert_eq!(binding.skill_id, "implementation");
        assert_eq!(
            topic.metadata.as_ref().unwrap().description.as_deref(),
            Some("Use implementation mode")
        );
    }

    #[test]
    fn new_command_inside_topic_starts_segment_not_conversation() {
        let mut projection = ConversationProjection::default();
        projection
            .apply_event(
                conversation_context("room_1", 1, Some("topic_agent")),
                &application_event(
                    DurableAppEventKind::ConversationCreate,
                    Some("topic_agent"),
                    b"{}",
                ),
            )
            .unwrap();

        let decision = projection
            .apply_event(
                conversation_context("room_1", 4, Some("topic_agent")),
                &application_event(
                    DurableAppEventKind::ConversationSegmentStart,
                    Some("topic_agent"),
                    br#"{"segment_id":"segment_2","reason":"slash_new"}"#,
                ),
            )
            .unwrap();
        let topic = projection.get("room_1", "topic_agent").unwrap();

        assert_eq!(decision, ConversationProjectionDecision::SegmentStarted);
        assert_eq!(projection.len(), 1);
        assert_eq!(topic.active_segment_id.as_deref(), Some("segment_2"));
        assert_eq!(topic.segments.len(), 1);
        assert_eq!(topic.segments[0].started_seq, 4);
    }

    #[test]
    fn segment_boundary_rejects_missing_conversation_id_or_bad_payload() {
        let mut projection = ConversationProjection::default();
        assert!(matches!(
            projection
                .apply_event(
                    conversation_context("room_1", 4, None),
                    &application_event(
                        DurableAppEventKind::ConversationSegmentStart,
                        None,
                        br#"{"segment_id":"segment_2"}"#,
                    ),
                )
                .unwrap_err(),
            ConversationProjectionError::MissingConversationId { .. }
        ));
        assert_eq!(
            projection
                .apply_event(
                    conversation_context("room_1", 4, Some("topic_agent")),
                    &application_event(
                        DurableAppEventKind::ConversationSegmentStart,
                        Some("topic_agent"),
                        b"not-json",
                    ),
                )
                .unwrap_err(),
            ConversationProjectionError::MalformedSegmentPayload
        );
    }

    #[test]
    fn segment_boundary_is_projected_without_protocol_managed_prompt_state() {
        let mut projection = ConversationProjection::default();
        projection
            .apply_event(
                conversation_context("room_1", 1, Some("topic_agent")),
                &application_event(
                    DurableAppEventKind::ConversationCreate,
                    Some("topic_agent"),
                    br#"{"title":"Agent"}"#,
                ),
            )
            .unwrap();

        let decision = projection
            .apply_event(
                conversation_context("room_1", 8, Some("topic_agent")),
                &application_event(
                    DurableAppEventKind::ConversationSegmentStart,
                    Some("topic_agent"),
                    br#"{"segment_id":"segment_2","reason":"slash_new","prompt":"ignored by protocol","messages":[{"role":"user","content":"hi"}]}"#,
                ),
            )
            .unwrap();
        let topic = projection.get("room_1", "topic_agent").unwrap();

        assert_eq!(decision, ConversationProjectionDecision::SegmentStarted);
        assert_eq!(topic.active_segment_id.as_deref(), Some("segment_2"));
        assert_eq!(
            topic.segments,
            vec![ConversationSegmentProjectionRecord {
                segment_id: "segment_2".to_string(),
                started_seq: 8,
            }]
        );
        assert_eq!(
            topic.metadata.as_ref().unwrap().title.as_deref(),
            Some("Agent")
        );
    }

    #[test]
    fn conversation_metadata_rejects_missing_conversation_id_or_bad_payload() {
        let mut projection = ConversationProjection::default();
        assert!(matches!(
            projection
                .apply_event(
                    conversation_context("room_1", 4, None),
                    &application_event(DurableAppEventKind::ConversationCreate, None, b"{}"),
                )
                .unwrap_err(),
            ConversationProjectionError::MissingConversationId { .. }
        ));
        assert_eq!(
            projection
                .apply_event(
                    conversation_context("room_1", 5, Some("topic_agent")),
                    &application_event(
                        DurableAppEventKind::ConversationCreate,
                        Some("topic_agent"),
                        b"not-json",
                    ),
                )
                .unwrap_err(),
            ConversationProjectionError::MalformedMetadataPayload
        );
        assert!(projection.is_empty());
    }

    #[test]
    fn archiving_topic_does_not_archive_sibling_topic() {
        let mut projection = ConversationProjection::default();
        for (seq, topic) in [(1, "topic_agent"), (2, "topic_human")] {
            projection
                .apply_event(
                    conversation_context("room_1", seq, Some(topic)),
                    &application_event(DurableAppEventKind::ChatMessage, Some(topic), b"hello"),
                )
                .unwrap();
        }
        projection
            .apply_event(
                conversation_context("room_1", 3, Some("topic_agent")),
                &application_event(
                    DurableAppEventKind::ConversationArchive,
                    Some("topic_agent"),
                    b"{}",
                ),
            )
            .unwrap();

        assert!(projection.get("room_1", "topic_agent").unwrap().archived);
        assert!(!projection.get("room_1", "topic_human").unwrap().archived);
    }

    #[test]
    fn hosted_web_mode_is_not_labeled_e2ee() {
        let disclosure =
            ProductTrustDisclosureV1::for_mode(ProductTrustModeV1::HostedTrustedServerClient);

        assert!(!disclosure.may_claim_e2ee);
        assert!(!disclosure.stores_device_secrets_on_user_device);
        assert!(!disclosure.label.to_ascii_lowercase().contains("e2ee"));
        assert!(!disclosure.label.to_ascii_lowercase().contains("end-to-end"));
        assert!(!disclosure.read_only);
    }

    #[test]
    fn hosted_web_mode_uses_server_side_trusted_client() {
        let disclosure =
            ProductTrustDisclosureV1::for_mode(ProductTrustModeV1::HostedTrustedServerClient);

        assert_eq!(
            disclosure.mode,
            ProductTrustModeV1::HostedTrustedServerClient
        );
        assert_eq!(disclosure.label, "web chat");
        assert!(!disclosure.stores_device_secrets_on_user_device);
    }

    #[test]
    fn product_client_kinds_have_explicit_secret_locations() {
        assert_eq!(
            ProductClientKindV1::HostedWebBridge.secret_location(),
            DeviceSecretLocationV1::TrustedHostedServer
        );
        assert_eq!(
            ProductClientKindV1::NativeDevice.secret_location(),
            DeviceSecretLocationV1::UserDevice
        );
        assert_eq!(
            ProductClientKindV1::ElectronDaemon.secret_location(),
            DeviceSecretLocationV1::UserDevice
        );
        assert_eq!(
            ProductClientKindV1::RuntimeDevice.secret_location(),
            DeviceSecretLocationV1::RuntimeHost
        );
        assert_eq!(
            ProductClientKindV1::PlaintextArchive.secret_location(),
            DeviceSecretLocationV1::None
        );
    }

    #[test]
    fn native_and_electron_modes_keep_device_secrets_on_user_device() {
        for client_kind in [
            ProductClientKindV1::NativeDevice,
            ProductClientKindV1::ElectronDaemon,
        ] {
            let trust_mode = client_kind.product_trust_mode().unwrap();
            let disclosure = ProductTrustDisclosureV1::for_mode(trust_mode);

            assert_eq!(
                client_kind.secret_location(),
                DeviceSecretLocationV1::UserDevice
            );
            assert_eq!(trust_mode, ProductTrustModeV1::LocalDeviceE2ee);
            assert!(disclosure.may_claim_e2ee);
            assert!(disclosure.stores_device_secrets_on_user_device);
        }
    }

    #[test]
    fn runtime_device_keeps_device_secret_on_runtime_host() {
        assert_eq!(
            ProductClientKindV1::RuntimeDevice.secret_location(),
            DeviceSecretLocationV1::RuntimeHost
        );
        assert_eq!(
            ProductClientKindV1::RuntimeDevice.product_trust_mode(),
            None
        );
        assert!(!ProductClientKindV1::RuntimeDevice.is_server_side_bridge());
    }

    #[test]
    fn hosted_web_bridge_is_not_a_local_device_e2ee_surface() {
        let trust_mode = ProductClientKindV1::HostedWebBridge
            .product_trust_mode()
            .unwrap();
        let disclosure = ProductTrustDisclosureV1::for_mode(trust_mode);

        assert!(ProductClientKindV1::HostedWebBridge.is_server_side_bridge());
        assert_eq!(
            ProductClientKindV1::HostedWebBridge.secret_location(),
            DeviceSecretLocationV1::TrustedHostedServer
        );
        assert_eq!(trust_mode, ProductTrustModeV1::HostedTrustedServerClient);
        assert!(!disclosure.may_claim_e2ee);
        assert!(!disclosure.stores_device_secrets_on_user_device);
    }

    #[test]
    fn local_daemon_mode_keeps_device_secrets_local() {
        let disclosure = ProductTrustDisclosureV1::for_mode(ProductTrustModeV1::LocalDeviceE2ee);

        assert!(disclosure.may_claim_e2ee);
        assert!(disclosure.stores_device_secrets_on_user_device);
        assert!(disclosure.label.to_ascii_lowercase().contains("end-to-end"));
        assert!(!disclosure.read_only);
    }

    #[test]
    fn old_plaintext_chats_render_as_read_only_archive() {
        let disclosure = ProductTrustDisclosureV1::for_mode(ProductTrustModeV1::PlaintextArchive);

        assert!(!disclosure.may_claim_e2ee);
        assert!(!disclosure.stores_device_secrets_on_user_device);
        assert!(disclosure.read_only);
        assert!(disclosure.label.to_ascii_lowercase().contains("archive"));
    }

    #[test]
    fn runtime_bridge_state_projection_is_scoped_by_room_source_device_and_key() {
        let runtime_box = device("runtime_npub", "box");
        let runtime_gpu = device("runtime_npub", "gpu");
        let mut projection = RuntimeStateProjection::default();
        for (room_id, source, state_key, payload) in [
            (
                "room_agent_a",
                runtime_box.clone(),
                "runtime.connection.telegram",
                br#"{"topic":"alpha"}"#.as_slice(),
            ),
            (
                "room_agent_b",
                runtime_box.clone(),
                "runtime.connection.telegram",
                br#"{"topic":"beta"}"#.as_slice(),
            ),
            (
                "room_agent_a",
                runtime_gpu.clone(),
                "runtime.connection.telegram",
                br#"{"topic":"gpu"}"#.as_slice(),
            ),
            (
                "room_agent_a",
                runtime_box.clone(),
                "runtime.connection.matrix",
                br#"{"room":"matrix"}"#.as_slice(),
            ),
        ] {
            projection
                .apply(runtime_state_entry(
                    room_id,
                    source,
                    state_key,
                    "finitecomputer.bridge.status.v1",
                    1,
                    1,
                    payload,
                ))
                .unwrap();
        }

        assert_eq!(
            projection
                .get("room_agent_a", &runtime_box, "runtime.connection.telegram")
                .unwrap()
                .snapshot
                .status_payload,
            br#"{"topic":"alpha"}"#
        );
        assert_eq!(
            projection
                .get("room_agent_b", &runtime_box, "runtime.connection.telegram")
                .unwrap()
                .snapshot
                .status_payload,
            br#"{"topic":"beta"}"#
        );
        assert_eq!(
            projection
                .get("room_agent_a", &runtime_gpu, "runtime.connection.telegram")
                .unwrap()
                .snapshot
                .status_payload,
            br#"{"topic":"gpu"}"#
        );
        assert_eq!(
            projection
                .get("room_agent_a", &runtime_box, "runtime.connection.matrix")
                .unwrap()
                .snapshot
                .status_payload,
            br#"{"room":"matrix"}"#
        );
    }

    fn runtime_state_entry(
        room_id: &str,
        source: DeviceRef,
        state_key: &str,
        schema: &str,
        revision: u64,
        accepted_seq: Seq,
        payload: &[u8],
    ) -> RuntimeStateProjectionEntry {
        RuntimeStateProjectionEntry {
            room_id: room_id.to_string(),
            source,
            accepted_seq,
            snapshot: RuntimeStateSnapshotV1 {
                state_key: state_key.to_string(),
                schema: schema.to_string(),
                revision,
                observed_at_ms: 1_000,
                expires_at_ms: 2_000,
                status_payload: payload.to_vec(),
            },
        }
    }

    fn runtime_state_entry_with_expiry(
        room_id: &str,
        source: DeviceRef,
        observed_at_ms: u64,
        expires_at_ms: u64,
    ) -> RuntimeStateProjectionEntry {
        RuntimeStateProjectionEntry {
            room_id: room_id.to_string(),
            source,
            accepted_seq: 10,
            snapshot: RuntimeStateSnapshotV1 {
                state_key: "runtime.gateway".to_string(),
                schema: "finitecomputer.runtime.gateway.status.v1".to_string(),
                revision: 1,
                observed_at_ms,
                expires_at_ms,
                status_payload: br#"{"status":"live"}"#.to_vec(),
            },
        }
    }

    fn runtime_command_request(
        request_id: &str,
        command: &str,
        target: RuntimeCommandTargetV1,
        body: &[u8],
    ) -> RuntimeCommandRequestV1 {
        RuntimeCommandRequestV1 {
            payload_kind: RuntimeCommandPayloadKindV1::Request,
            request_id: request_id.to_string(),
            command: command.to_string(),
            target,
            resource_key: Some("hermes.config".to_string()),
            body: runtime_command_body(body),
        }
    }

    fn runtime_command_body(body: &[u8]) -> RuntimeCommandJsonPayloadV1 {
        RuntimeCommandJsonPayloadV1 {
            schema: "finitecomputer.runtime.command.body.v1".to_string(),
            json_payload: body.to_vec(),
        }
    }

    fn runtime_command_success_result(request_id: &str, body: &[u8]) -> RuntimeCommandResultV1 {
        RuntimeCommandResultV1 {
            payload_kind: RuntimeCommandPayloadKindV1::Result,
            request_id: request_id.to_string(),
            status: RuntimeCommandTerminalStatusV1::Succeeded,
            body: Some(runtime_command_body(body)),
            error: None,
            clears_activity: Vec::new(),
        }
    }

    fn runtime_command_cancel(request_id: &str) -> RuntimeCommandCancelV1 {
        RuntimeCommandCancelV1 {
            payload_kind: RuntimeCommandPayloadKindV1::Cancel,
            request_id: request_id.to_string(),
            reason: Some("user_requested".to_string()),
        }
    }

    fn activity_context<'a>(
        room_id: &'a str,
        conversation_id: Option<&'a str>,
        sender: &'a DeviceRef,
        received_at_ms: u64,
        expires_at_ms: u64,
    ) -> EphemeralActivityIngressContext<'a> {
        EphemeralActivityIngressContext {
            room_id,
            conversation_id,
            sender,
            received_at_ms,
            expires_at_ms,
        }
    }

    fn activity_set(
        activity_kind: &str,
        activity_id: Option<&str>,
        payload: &[u8],
    ) -> DecryptedEphemeralActivityV1 {
        DecryptedEphemeralActivityV1 {
            activity_kind: activity_kind.to_string(),
            activity_id: activity_id.map(str::to_string),
            action: EphemeralActivityActionV1::Set,
            payload: payload.to_vec(),
        }
    }

    fn activity_clear(
        activity_kind: &str,
        activity_id: Option<&str>,
    ) -> DecryptedEphemeralActivityV1 {
        DecryptedEphemeralActivityV1 {
            activity_kind: activity_kind.to_string(),
            activity_id: activity_id.map(str::to_string),
            action: EphemeralActivityActionV1::Clear,
            payload: Vec::new(),
        }
    }

    fn conversation_context<'a>(
        room_id: &'a str,
        accepted_seq: Seq,
        conversation_id: Option<&'a str>,
    ) -> ConversationProjectionEventContext<'a> {
        ConversationProjectionEventContext {
            room_id,
            accepted_seq,
            conversation_id,
        }
    }

    fn conversation_metadata_payload(metadata: ConversationMetadataV1) -> Vec<u8> {
        metadata.validate_limits().unwrap();
        serde_json::to_vec(&metadata).unwrap()
    }

    fn application_event(
        kind: DurableAppEventKind,
        conversation_id: Option<&str>,
        payload: &[u8],
    ) -> DecryptedApplicationEventV1 {
        DecryptedApplicationEventV1 {
            kind,
            conversation_id: conversation_id.map(str::to_string),
            segment_id: None,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn stream_kinds_have_reserved_policies_and_round_trip() {
        assert_eq!(
            DurableAppEventKind::StreamStart.delivery_policy(),
            ApplicationDeliveryPolicy::NON_NOTIFYING
        );
        assert_eq!(
            DurableAppEventKind::StreamFinish.delivery_policy(),
            ApplicationDeliveryPolicy::USER_VISIBLE_MESSAGE
        );
        let finish = StreamFinishV1 {
            stream_id: "stream-1".to_owned(),
            conversation_id: "conversation-1".to_owned(),
            transcript_hash: vec![0xAA; 32],
            final_payload: b"final text".to_vec(),
        };
        let decoded: StreamFinishV1 =
            serde_json::from_slice(&serde_json::to_vec(&finish).unwrap()).unwrap();
        assert_eq!(decoded, finish);
    }
}
mod runtime;
pub use runtime::*;
mod nostr;
pub use nostr::*;
