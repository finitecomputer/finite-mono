use finitechat_delivery::{
    HttpKeyPackageId, HttpKeyPackagePublication, HttpSyncPage, MAX_HTTP_SYNC_PAGE_ENTRIES,
};
use finitechat_http::{
    AckWelcomeRequest, AckWelcomeResponse, BootstrapAccountRoomRequest,
    BootstrapAccountRoomResponse, ClaimKeyPackageForAccountRequest, ClaimKeyPackageRequest,
    ClaimWelcomesRequest, FiniteAccountRoomCommitProjection, GetEphemeralActivitiesRequest,
    GetEphemeralActivitiesResponse, GetNostrProfilesRequest, GetNostrProfilesResponse,
    GroupSyncRequest, HttpClaimedWelcome, HttpKeyPackageInventory, KeyPackageInventoryRequest,
    ListAccountRoomDirectoryRequest, ListAccountRoomDirectoryResponse, NostrProfileRecord,
    PublishKeyPackageResponse, PushPlatform, PutNostrProfileRequest, PutNostrProfileResponse,
    RegisterPushTokenRequest, RegisterPushTokenResponse, RemovePushTokenRequest,
    RemovePushTokenResponse, RevokeDeviceRequest, RevokeDeviceResponse, SaveAccountRoomRequest,
    SaveAccountRoomResponse, SyncHintEvent, SyncStreamRequest, SyncWaitRequest, SyncWaitResponse,
};
use finitechat_mls::{
    ExpectedDeviceCredential, FiniteDeviceCredentialV1, MlsCredentialError, NOSTR_PUBLIC_KEY_BYTES,
    NOSTR_SECRET_KEY_BYTES, NostrPublicKey, NostrSecretKey,
};
use finitechat_proto::{
    AccountRoomRecord, AppendApplicationEventRequest, AppendEventRequest,
    ApplicationDeliveryPolicy, ClaimKeyPackageResult, CommitAccepted, CreateRoomRequest,
    EngineError, EventAccepted, KeyPackageInventory, ListAccountRoomsPage, ListAccountRoomsRequest,
    SubmitCommitRequest, SyncEventsPage, UploadKeyPackageRequest, WelcomeRecord,
    delivery_member_id_for_device, envelope, lease_token_for,
};
use finitechat_proto::{AppendEphemeralActivityRequest, EphemeralActivityAccepted};
use finitechat_proto::{
    DecryptedApplicationEventV1, DeviceLinkBootstrapEventV2, DeviceLinkBootstrapProfileV2,
    DeviceLinkBootstrapRoomV2, DeviceLinkBootstrapSelectionV2, DeviceLinkBootstrapV2,
    DurableAppEventKind, FINITECHAT_DEVICE_LINK_BOOTSTRAP_EVENT_V2, message_id_for_bytes,
};
use finitechat_proto::{
    DeviceRef, KeyPackageId, LogEntryKind, MAX_ACCOUNT_ID_BYTES,
    MAX_ACCOUNT_ROOM_DISCOVERY_RESULTS, MAX_DEVICE_ID_BYTES, MAX_ENVELOPE_PAYLOAD_BYTES,
    MAX_IDEMPOTENCY_KEY_BYTES, MAX_KEY_PACKAGE_PAYLOAD_BYTES, MAX_KEY_PACKAGES_PER_DEVICE,
    MAX_MLS_GROUP_ID_BYTES, MAX_OBJECT_ID_BYTES, MAX_RATCHET_TREE_PAYLOAD_BYTES, MAX_ROOM_ID_BYTES,
    MAX_STAGED_WELCOMES_PER_COMMIT, MAX_WELCOME_CLAIMS_PER_REQUEST, MAX_WELCOME_PAYLOAD_BYTES,
    MembershipAddV1, MembershipDeltaV1, MembershipRemoveV1, MessageId, MlsGroupId,
    ProtocolLimitError, RoomId, RoomLogEntry, StagedWelcomeV1, WelcomeId, WelcomeState,
    validate_bytes_len, validate_bytes_non_empty, validate_idempotency_key, validate_item_count,
    validate_mls_group_id, validate_room_id, validate_string_bytes,
};
use finitechat_transport::engine::KeyPackage as HttpKeyPackage;
use finitechat_transport::{
    GroupId as HttpGroupId, MemberId as HttpMemberId, MessageId as HttpMessageId,
};
use openmls::prelude::tls_codec::{Deserialize as _, Serialize as _};
use openmls::prelude::{
    AeadType, Ciphersuite, CredentialWithKey, GroupId, KeyPackage, KeyPackageIn, LeafNodeIndex,
    LeafNodeParameters, MlsGroup, MlsGroupCreateConfig, MlsMessageBodyIn, MlsMessageIn,
    MlsMessageOut, OpenMlsCrypto, OpenMlsProvider, OpenMlsRand, ProcessedMessageContent,
    ProtocolMessage, ProtocolVersion, RatchetTreeIn, StagedCommit, StagedWelcome, Welcome,
};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub mod rejected_entry_diagnostic;

pub const FINITECHAT_CIPHERSUITE: Ciphersuite =
    Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

const CLIENT_STORE_KEY_DERIVATION_DOMAIN: &[u8] = b"finitechat.client-store-key.v1";
const CLIENT_STATE_SNAPSHOT_MAGIC: &[u8] = b"finitechat.client-state-snapshot.v1";
const CLIENT_APP_MESSAGE_AAD_DOMAIN: &[u8] = b"finitechat.client-app-message.v1";
const CLIENT_APP_EVENT_AAD_DOMAIN: &[u8] = b"finitechat.client-app-event.v1";
const CLIENT_APP_OUTBOX_AAD_DOMAIN: &[u8] = b"finitechat.client-app-outbox.v1";
const CLIENT_APP_ROOM_AAD_DOMAIN: &[u8] = b"finitechat.client-app-room.v1";
const CLIENT_APP_STATE_AAD_DOMAIN: &[u8] = b"finitechat.client-app-state.v1";
const CLIENT_APP_PROFILE_AAD_DOMAIN: &[u8] = b"finitechat.client-app-profile.v1";
const CLIENT_DEVICE_LINK_BOOTSTRAP_MANIFEST_AAD_DOMAIN: &[u8] =
    b"finitechat.client-device-link-bootstrap-manifest.v1";
const CLIENT_DEVICE_LINK_BOOTSTRAP_CHUNK_AAD_DOMAIN: &[u8] =
    b"finitechat.client-device-link-bootstrap-chunk.v1";
const DEVICE_LINK_BOOTSTRAP_MANIFEST_DIGEST_DOMAIN: &[u8] =
    b"finitechat.device-link-bootstrap-manifest.v2";
const LEGACY_DEVICE_LINK_BOOTSTRAP_REQUEST_EVENT_V2: &str =
    "finitechat.device-link.bootstrap-request.v2";
const LEGACY_CLIENT_STATE_SNAPSHOT_VERSION: u16 = 8;
const CLIENT_STATE_SNAPSHOT_VERSION: u16 = 9;
const CLIENT_STORE_KEY_BYTES: usize = 32;
const CLIENT_STORE_NONCE_BYTES: usize = 12;
const CLIENT_STORE_AEAD_TAG_BYTES: u32 = 16;
const MAX_PERSISTED_ROOMS: u32 = 1024;
const MAX_ROOM_SERVER_URL_BYTES: u32 = 2048;
const ACTIVITY_EXPORTER_LABEL: &str = "finite-activity-v1";
const ACTIVITY_NONCE_BYTES: usize = 12;
const MAX_PENDING_CLIENT_WELCOMES: u32 = MAX_WELCOME_CLAIMS_PER_REQUEST;
const MAX_PENDING_KEY_PACKAGE_UPLOADS: u32 = MAX_KEY_PACKAGES_PER_DEVICE;
const MAX_LINK_FANOUTS: u32 = 16;
const MAX_LINK_FANOUT_ROOMS: u32 = MAX_ACCOUNT_ROOM_DISCOVERY_RESULTS;
const MAX_RUNTIME_SYNC_PAGES_PER_ROOM: u32 = 64;
const PENDING_COMMIT_SYNC_OVERLAP: u64 = 8192;
const MAX_RUNTIME_LINK_FANOUT_DISCOVERY_PAGES_PER_TICK: u32 = MAX_LINK_FANOUT_ROOMS;
const MAX_RUNTIME_LINK_FANOUT_COMMITS_PER_TICK: u32 = MAX_LINK_FANOUT_ROOMS;
const MAX_OPENMLS_STORAGE_RECORDS: u32 = 8192;
const MAX_CLIENT_SIGNER_PUBLIC_KEY_BYTES: u32 = MAX_OBJECT_ID_BYTES;
const MAX_CLIENT_CREDENTIAL_IDENTITY_BYTES: u32 = 1024;
const MAX_OPENMLS_STORAGE_KEY_BYTES: u32 = 4 * 1024;
const MAX_OPENMLS_STORAGE_VALUE_BYTES: u32 = 8 * 1024 * 1024;
const MAX_CLIENT_STATE_PLAINTEXT_BYTES: u32 = 32 * 1024 * 1024;
const MAX_CLIENT_STATE_CIPHERTEXT_BYTES: u32 =
    MAX_CLIENT_STATE_PLAINTEXT_BYTES + CLIENT_STORE_AEAD_TAG_BYTES;
const MAX_APP_MESSAGE_CIPHERTEXT_BYTES: u32 =
    MAX_ENVELOPE_PAYLOAD_BYTES + CLIENT_STORE_AEAD_TAG_BYTES;
const MAX_APP_EVENT_CIPHERTEXT_BYTES: u32 =
    MAX_ENVELOPE_PAYLOAD_BYTES + CLIENT_STORE_AEAD_TAG_BYTES;
const MAX_APP_OUTBOX_METADATA_PLAINTEXT_BYTES: u32 = MAX_ENVELOPE_PAYLOAD_BYTES + 8 * 1024;
const MAX_APP_OUTBOX_METADATA_CIPHERTEXT_BYTES: u32 =
    MAX_APP_OUTBOX_METADATA_PLAINTEXT_BYTES + CLIENT_STORE_AEAD_TAG_BYTES;
const MAX_APP_ROOM_DISPLAY_NAME_BYTES: u32 = 256;
const MAX_APP_ROOM_PICTURE_BYTES: u32 = 2 * 1024;
const MAX_APP_ROOM_STATUS_BYTES: u32 = 512;
const MAX_APP_ROOM_METADATA_PLAINTEXT_BYTES: u32 = 8192;
const MAX_APP_ROOM_METADATA_CIPHERTEXT_BYTES: u32 =
    MAX_APP_ROOM_METADATA_PLAINTEXT_BYTES + CLIENT_STORE_AEAD_TAG_BYTES;
const MAX_APP_STATE_METADATA_PLAINTEXT_BYTES: u32 = 8 * 1024 * 1024;
const MAX_APP_STATE_METADATA_CIPHERTEXT_BYTES: u32 =
    MAX_APP_STATE_METADATA_PLAINTEXT_BYTES + CLIENT_STORE_AEAD_TAG_BYTES;
const MAX_APP_PROFILE_NAME_BYTES: u32 = 128;
const MAX_APP_PROFILE_ABOUT_BYTES: u32 = 4 * 1024;
const MAX_APP_PROFILE_PICTURE_BYTES: u32 = 2 * 1024;
const MAX_APP_PROFILE_METADATA_PLAINTEXT_BYTES: u32 = 8192;
const MAX_APP_PROFILE_METADATA_CIPHERTEXT_BYTES: u32 =
    MAX_APP_PROFILE_METADATA_PLAINTEXT_BYTES + CLIENT_STORE_AEAD_TAG_BYTES;
const MAX_DEVICE_LINK_BOOTSTRAP_MANIFEST_PLAINTEXT_BYTES: u32 =
    finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_PAYLOAD_BYTES;
const MAX_DEVICE_LINK_BOOTSTRAP_MANIFEST_CIPHERTEXT_BYTES: u32 =
    MAX_DEVICE_LINK_BOOTSTRAP_MANIFEST_PLAINTEXT_BYTES + CLIENT_STORE_AEAD_TAG_BYTES;
const MAX_DEVICE_LINK_BOOTSTRAP_CHUNK_PLAINTEXT_BYTES: u32 =
    finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_PAYLOAD_BYTES;
const MAX_DEVICE_LINK_BOOTSTRAP_CHUNK_CIPHERTEXT_BYTES: u32 =
    MAX_DEVICE_LINK_BOOTSTRAP_CHUNK_PLAINTEXT_BYTES + CLIENT_STORE_AEAD_TAG_BYTES;
const MAX_PENDING_DEVICE_LINK_BOOTSTRAPS: u32 = 32;
// Message retention is complete. This is the API's addressability bound, not
// a product history window; callers must not silently prune durable history.
const MAX_STORED_APP_MESSAGES: u32 = u32::MAX;
const MAX_STORED_APP_OUTBOX_MESSAGES: u32 = 512;
const MAX_STORED_APP_PROFILES: u32 = 4_096;
const MAX_STORED_APP_REVOKED_DEVICES: u32 = 64;
const MAX_STORED_APP_CHAT_ARCHIVES: u32 = 5_000;
const U16_BYTES: usize = 2;
const U32_BYTES: usize = 4;
const U64_BYTES: usize = 8;
const LINK_FANOUT_STATUS_PENDING: u16 = 0;
const LINK_FANOUT_STATUS_PREPARED: u16 = 1;
const LINK_FANOUT_STATUS_DONE: u16 = 2;
const LOG_ENTRY_KIND_APPLICATION: u16 = 0;
const LOG_ENTRY_KIND_PROPOSAL: u16 = 1;
const LOG_ENTRY_KIND_COMMIT: u16 = 2;
const DEVICE_LINK_BOOTSTRAP_STATE_RECEIVING: i64 = 0;
const DEVICE_LINK_BOOTSTRAP_STATE_POISONED: i64 = 1;
const DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED: i64 = 2;
const DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED_POISONED: i64 = 3;

const _: () = {
    assert!(NOSTR_PUBLIC_KEY_BYTES == 32);
    assert!(CLIENT_STORE_KEY_BYTES == 32);
    assert!(CLIENT_STORE_NONCE_BYTES == 12);
    assert!(CLIENT_STORE_AEAD_TAG_BYTES == 16);
    assert!(MAX_PERSISTED_ROOMS > 0);
    assert!(MAX_PENDING_CLIENT_WELCOMES > 0);
    assert!(MAX_PENDING_KEY_PACKAGE_UPLOADS > 0);
    assert!(MAX_LINK_FANOUTS > 0);
    assert!(MAX_LINK_FANOUT_ROOMS > 0);
    assert!(MAX_RUNTIME_SYNC_PAGES_PER_ROOM > 0);
    assert!(MAX_RUNTIME_LINK_FANOUT_DISCOVERY_PAGES_PER_TICK > 0);
    assert!(MAX_RUNTIME_LINK_FANOUT_COMMITS_PER_TICK > 0);
    assert!(MAX_OPENMLS_STORAGE_RECORDS > 0);
    assert!(MAX_OPENMLS_STORAGE_KEY_BYTES > 0);
    assert!(MAX_OPENMLS_STORAGE_VALUE_BYTES > MAX_OPENMLS_STORAGE_KEY_BYTES);
    assert!(MAX_CLIENT_STATE_CIPHERTEXT_BYTES > MAX_CLIENT_STATE_PLAINTEXT_BYTES);
    assert!(MAX_APP_MESSAGE_CIPHERTEXT_BYTES > MAX_ENVELOPE_PAYLOAD_BYTES);
    assert!(MAX_APP_EVENT_CIPHERTEXT_BYTES > MAX_ENVELOPE_PAYLOAD_BYTES);
    assert!(MAX_APP_ROOM_METADATA_CIPHERTEXT_BYTES > MAX_APP_ROOM_METADATA_PLAINTEXT_BYTES);
    assert!(MAX_APP_PROFILE_METADATA_CIPHERTEXT_BYTES > MAX_APP_PROFILE_METADATA_PLAINTEXT_BYTES);
    assert!(
        MAX_DEVICE_LINK_BOOTSTRAP_MANIFEST_CIPHERTEXT_BYTES
            > MAX_DEVICE_LINK_BOOTSTRAP_MANIFEST_PLAINTEXT_BYTES
    );
    assert!(
        MAX_DEVICE_LINK_BOOTSTRAP_CHUNK_CIPHERTEXT_BYTES
            > MAX_DEVICE_LINK_BOOTSTRAP_CHUNK_PLAINTEXT_BYTES
    );
    assert!(MAX_PENDING_DEVICE_LINK_BOOTSTRAPS > 0);
    assert!(MAX_STORED_APP_MESSAGES > 0);
    assert!(MAX_STORED_APP_PROFILES > 0);
};

#[derive(Debug, Clone)]
pub struct FiniteChatDeviceConfig {
    pub account_secret_key: NostrSecretKey,
    pub device_id: String,
    pub now_unix_seconds: u64,
    pub credential_not_before_unix_seconds: u64,
    pub credential_not_after_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedApplicationEntry {
    pub plaintext: Vec<u8>,
    /// MLS-authenticated sender derived from the verified message credential.
    pub sender: DeviceRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommit {
    pub request: SubmitCommitRequest,
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiniteChatDeviceState {
    pub device_ref: DeviceRef,
    pub signer_public_key: Vec<u8>,
    pub credential_identity: Vec<u8>,
    pub rooms: Vec<PersistedRoomState>,
    pub pending_welcomes: Vec<PendingWelcomeState>,
    pub pending_welcome_acks: Vec<PendingWelcomeAckState>,
    pub pending_key_package_uploads: Vec<UploadKeyPackageRequest>,
    pub link_fanouts: Vec<LinkFanoutState>,
    pub openmls_storage_records: Vec<OpenMlsStorageRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedRoomState {
    pub room_id: RoomId,
    pub mls_group_id: MlsGroupId,
    pub last_applied_seq: u64,
    /// Which server hosts this room's ordered log. `None` means the
    /// device's home server (ADR 0005). Rooms activated from a Welcome
    /// record the room server that stored the Welcome so sync ticks can
    /// group rooms by server.
    pub server_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWelcomeState {
    pub welcome_id: WelcomeId,
    pub room_id: RoomId,
    pub commit_seq: u64,
    pub welcome_payload: Vec<u8>,
    pub ratchet_tree_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWelcomeAckState {
    pub welcome_id: WelcomeId,
    pub room_id: RoomId,
    pub commit_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkFanoutState {
    pub fanout_id: String,
    pub target_device: DeviceRef,
    pub after_room_id: Option<RoomId>,
    pub discovery_complete: bool,
    pub rooms: Vec<LinkFanoutRoomState>,
    pub bootstrap_export: LinkBootstrapExportState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LinkBootstrapExportState {
    #[default]
    WaitingForFanout,
    Exporting {
        canonical_selection: Option<DeviceLinkBootstrapSelectionV2>,
        rooms: Vec<LinkBootstrapRoomExport>,
    },
    Emitted {
        manifests: Vec<LinkBootstrapManifestReceipt>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkBootstrapRoomExport {
    pub room: DeviceLinkBootstrapRoomV2,
    pub canonical_selection: Option<DeviceLinkBootstrapSelectionV2>,
    pub profiles: Vec<DeviceLinkBootstrapProfileV2>,
    pub through_seq: u64,
    pub audit_after_seq: u64,
    pub audit_complete: bool,
    pub planning_after_seq: u64,
    pub planning_after_message_id: String,
    pub planning_complete: bool,
    pub chunks: Vec<LinkBootstrapChunkPlan>,
    pub total_history_events: u64,
    pub history_sha256: Option<String>,
    pub manifest_sha256: Option<String>,
    pub next_chunk_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkBootstrapChunkPlan {
    pub start_after_seq: u64,
    pub start_after_message_id: String,
    pub end_seq: u64,
    pub end_message_id: String,
    pub event_count: u32,
    pub chunk_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkBootstrapManifestReceipt {
    pub bootstrap_id: String,
    pub room_id: RoomId,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkFanoutRoomState {
    pub plan: LinkFanoutRoomPlan,
    pub claimed_key_package: Option<ClaimKeyPackageResult>,
    pub status: LinkFanoutRoomStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkFanoutRoomPlan {
    pub room_id: RoomId,
    pub key_package_id: KeyPackageId,
    pub welcome_id: WelcomeId,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkFanoutRoomStatus {
    Pending,
    Prepared { prepared: Box<PreparedCommit> },
    Done { accepted_seq: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenMlsStorageRecord {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppliedLogEntry {
    Application {
        plaintext: Vec<u8>,
        /// The MLS-authenticated sender (from the decrypted message's
        /// verified credential), not the server-claimed envelope sender.
        sender: DeviceRef,
    },
    Commit {
        sender: DeviceRef,
        epoch: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPackageReplenishmentPlan {
    pub inventory: KeyPackageInventory,
    pub target_available: u32,
    pub upload_requests: Vec<UploadKeyPackageRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSyncOptions {
    pub key_package_target_available: u32,
    pub max_sync_pages_per_room: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLinkFanoutOptions {
    pub max_discovery_pages_per_tick: u32,
    pub max_commit_rooms_per_tick: u32,
    pub max_completion_sync_pages_per_room: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeSyncReport {
    pub uploaded_key_packages: u32,
    pub claimed_welcomes: u32,
    pub activated_welcome_acks_sent: u32,
    pub sync_pages: u32,
    pub applied_entries: Vec<RuntimeAppliedEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAppliedEntry {
    pub room_id: RoomId,
    pub seq: u64,
    pub message_id: String,
    pub timestamp_unix_seconds: u64,
    pub entry: AppliedLogEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAppMessage {
    pub room_id: RoomId,
    pub seq: u64,
    pub message_id: MessageId,
    pub sender: DeviceRef,
    pub plaintext: Vec<u8>,
    pub timestamp_unix_seconds: u64,
}

impl StoredAppMessage {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_room_id(&self.room_id)?;
        validate_string_bytes(
            "app_message.message_id",
            &self.message_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        self.sender.validate_limits().map_err(ClientError::from)?;
        validate_bytes_len(
            "app_message.plaintext",
            self.plaintext.len(),
            MAX_ENVELOPE_PAYLOAD_BYTES,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAppEvent {
    pub room_id: RoomId,
    pub seq: u64,
    pub message_id: MessageId,
    pub sender: DeviceRef,
    pub plaintext: Vec<u8>,
    pub timestamp_unix_seconds: u64,
}

impl StoredAppEvent {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_room_id(&self.room_id)?;
        validate_string_bytes(
            "app_event.message_id",
            &self.message_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        self.sender.validate_limits().map_err(ClientError::from)?;
        validate_bytes_len(
            "app_event.plaintext",
            self.plaintext.len(),
            MAX_ENVELOPE_PAYLOAD_BYTES,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOutboundMessage {
    pub room_id: RoomId,
    pub message_id: MessageId,
    pub sender: DeviceRef,
    pub plaintext: Vec<u8>,
    pub local_state: StoredOutboundLocalState,
    pub server_delivery_state: StoredOutboundServerDeliveryState,
    pub append_request: AppendEventRequest,
    pub timestamp_unix_seconds: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredOutboundLocalState {
    Sending,
    #[default]
    Sent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredOutboundServerDeliveryState {
    #[default]
    Undelivered,
    Failed {
        reason: String,
    },
}

impl StoredOutboundLocalState {
    fn validate_limits(&self) -> Result<(), ClientError> {
        Ok(())
    }
}

impl StoredOutboundServerDeliveryState {
    fn validate_limits(&self) -> Result<(), ClientError> {
        match self {
            Self::Undelivered => Ok(()),
            Self::Failed { reason } => {
                validate_string_bytes(
                    "app_outbox.failure_reason",
                    reason,
                    MAX_APP_ROOM_STATUS_BYTES,
                )?;
                Ok(())
            }
        }
    }
}

impl StoredOutboundMessage {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_room_id(&self.room_id)?;
        validate_string_bytes(
            "app_outbox.message_id",
            &self.message_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        self.sender.validate_limits().map_err(ClientError::from)?;
        validate_bytes_len(
            "app_outbox.plaintext",
            self.plaintext.len(),
            MAX_ENVELOPE_PAYLOAD_BYTES,
        )?;
        self.local_state.validate_limits()?;
        self.server_delivery_state.validate_limits()?;
        self.append_request.validate_limits()?;
        if self.append_request.room_id != self.room_id {
            return Err(ClientError::OutboxRoomMismatch {
                expected: self.room_id.clone(),
                actual: self.append_request.room_id.clone(),
            });
        }
        let append_message_id = self
            .append_request
            .envelope
            .message_id()
            .map_err(ClientError::EnvelopeMessageId)?;
        if append_message_id != self.message_id {
            return Err(ClientError::OutboxMessageIdMismatch {
                expected: self.message_id.clone(),
                actual: append_message_id,
            });
        }
        if self.append_request.sender != self.sender {
            return Err(ClientError::OutboxSenderMismatch {
                expected: self.sender.clone(),
                actual: self.append_request.sender.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredOutboundMessageMetadataV1 {
    sender: DeviceRef,
    plaintext: Vec<u8>,
    local_state: StoredOutboundLocalState,
    server_delivery_state: StoredOutboundServerDeliveryState,
    append_request: AppendEventRequest,
    timestamp_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAppRoom {
    pub room_id: RoomId,
    pub display_name: String,
    pub picture: Option<String>,
    pub state: StoredAppRoomState,
    pub status: String,
    pub local_read_seq: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredAppRoomState {
    #[default]
    Connected,
    WaitingForApproval,
    Joining,
    UnavailableOnDevice,
}

impl StoredAppRoom {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_room_id(&self.room_id)?;
        validate_string_bytes(
            "app_room.display_name",
            &self.display_name,
            MAX_APP_ROOM_DISPLAY_NAME_BYTES,
        )?;
        validate_bytes_non_empty("app_room.display_name", self.display_name.len())?;
        validate_optional_app_room_picture("app_room.picture", self.picture.as_deref())?;
        validate_string_bytes("app_room.status", &self.status, MAX_APP_ROOM_STATUS_BYTES)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredAppRoomMetadataV1 {
    display_name: String,
    #[serde(default)]
    picture: Option<String>,
    state: StoredAppRoomState,
    status: String,
    local_read_seq: u64,
}

/// Durable proof that one exact V2 bootstrap manifest was verified and
/// published for this Device.
///
/// The receipt is intentionally independent from the device-state snapshot.
/// Callers can compare it with the source Device's expected receipt set after
/// a restart without decoding or rewriting older hosted snapshots.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StoredDeviceLinkBootstrapReceipt {
    pub bootstrap_id: String,
    pub room_id: RoomId,
    pub source: DeviceRef,
    pub target: DeviceRef,
    pub chunk_count: u32,
    pub total_history_events: u64,
    pub history_sha256: String,
    pub manifest_sha256: String,
}

impl StoredDeviceLinkBootstrapReceipt {
    pub fn from_bootstrap(
        source: &DeviceRef,
        bootstrap: &DeviceLinkBootstrapV2,
    ) -> Result<Self, ClientStoreError> {
        let manifest = StoredDeviceLinkBootstrapManifestV2::from_bootstrap(source, bootstrap)?;
        manifest.receipt()
    }
}

/// One encrypted, durable V2 transfer reconstructed from its manifest and
/// currently stored chunks. Incomplete transfers remain invisible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPendingDeviceLinkBootstrap {
    pub receipt: StoredDeviceLinkBootstrapReceipt,
    pub room: DeviceLinkBootstrapRoomV2,
    pub canonical_selection: Option<DeviceLinkBootstrapSelectionV2>,
    pub profiles: Vec<DeviceLinkBootstrapProfileV2>,
    pub earliest_envelope_seq: u64,
    pub chunks: BTreeMap<u32, Vec<DeviceLinkBootstrapEventV2>>,
}

impl StoredPendingDeviceLinkBootstrap {
    pub fn is_complete(&self) -> bool {
        self.chunks.len() == self.receipt.chunk_count as usize
            && (0..self.receipt.chunk_count).all(|index| self.chunks.contains_key(&index))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceLinkBootstrapStageOutcome {
    Pending {
        received_chunks: u32,
        expected_chunks: u32,
    },
    ExactDuplicate {
        received_chunks: u32,
        expected_chunks: u32,
    },
    Ready(StoredPendingDeviceLinkBootstrap),
    AlreadyCommitted(StoredDeviceLinkBootstrapReceipt),
    Poisoned,
    CapacityExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceLinkBootstrapCommitOutcome {
    Committed(StoredDeviceLinkBootstrapReceipt),
    AlreadyCommitted(StoredDeviceLinkBootstrapReceipt),
    Poisoned,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredDeviceLinkBootstrapManifestV2 {
    version: u16,
    bootstrap_id: String,
    source: DeviceRef,
    target: DeviceRef,
    chunk_count: u32,
    total_history_events: u64,
    history_sha256: String,
    room: DeviceLinkBootstrapRoomV2,
    canonical_selection: Option<DeviceLinkBootstrapSelectionV2>,
    profiles: Vec<DeviceLinkBootstrapProfileV2>,
}

impl StoredDeviceLinkBootstrapManifestV2 {
    fn from_bootstrap(
        source: &DeviceRef,
        bootstrap: &DeviceLinkBootstrapV2,
    ) -> Result<Self, ClientStoreError> {
        validate_device_link_bootstrap_manifest_fields(source, bootstrap)?;
        Ok(Self {
            version: bootstrap.version,
            bootstrap_id: bootstrap.bootstrap_id.clone(),
            source: source.clone(),
            target: bootstrap.target.clone(),
            chunk_count: bootstrap.chunk_count,
            total_history_events: bootstrap.total_history_events,
            history_sha256: bootstrap.history_sha256.clone(),
            room: bootstrap.room.clone(),
            canonical_selection: bootstrap.canonical_selection.clone(),
            profiles: bootstrap.profiles.clone(),
        })
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, ClientStoreError> {
        serde_json::to_vec(self).map_err(|_| ClientStoreError::EncodeDeviceLinkBootstrapManifest)
    }

    fn manifest_sha256(&self) -> Result<String, ClientStoreError> {
        let mut digest = Sha256::new();
        digest.update(DEVICE_LINK_BOOTSTRAP_MANIFEST_DIGEST_DOMAIN);
        digest.update(self.canonical_bytes()?);
        Ok(hex_lower(&digest.finalize()))
    }

    fn receipt(&self) -> Result<StoredDeviceLinkBootstrapReceipt, ClientStoreError> {
        Ok(StoredDeviceLinkBootstrapReceipt {
            bootstrap_id: self.bootstrap_id.clone(),
            room_id: self.room.room_id.clone(),
            source: self.source.clone(),
            target: self.target.clone(),
            chunk_count: self.chunk_count,
            total_history_events: self.total_history_events,
            history_sha256: self.history_sha256.clone(),
            manifest_sha256: self.manifest_sha256()?,
        })
    }
}

/// Hash one chunk using the canonical event encoding shared by V2 source
/// planning and target verification.
pub fn device_link_bootstrap_chunk_sha256(history: &[DeviceLinkBootstrapEventV2]) -> [u8; 32] {
    let mut digest = Sha256::new();
    for event in history {
        update_device_link_bootstrap_event_digest(&mut digest, event);
    }
    digest.finalize().into()
}

/// V2 transfer digest: hash each canonical chunk independently, concatenate
/// those raw 32-byte digests in chunk-index order, then hash that byte string.
///
/// An empty transfer still has exactly one empty chunk, so its value is
/// `SHA256(SHA256(empty))`, not `SHA256(empty)`.
pub fn device_link_bootstrap_history_sha256(chunks: &[Vec<DeviceLinkBootstrapEventV2>]) -> String {
    let mut digest = Sha256::new();
    for chunk in chunks {
        digest.update(device_link_bootstrap_chunk_sha256(chunk));
    }
    hex_lower(&digest.finalize())
}

fn update_device_link_bootstrap_event_digest(
    digest: &mut Sha256,
    event: &DeviceLinkBootstrapEventV2,
) {
    digest.update(event.seq.to_be_bytes());
    update_device_link_bootstrap_digest_field(digest, event.message_id.as_bytes());
    update_device_link_bootstrap_digest_field(digest, event.sender.account_id.as_bytes());
    update_device_link_bootstrap_digest_field(digest, event.sender.device_id.as_bytes());
    update_device_link_bootstrap_digest_field(digest, &event.plaintext);
    digest.update(event.timestamp_unix_seconds.to_be_bytes());
}

fn update_device_link_bootstrap_digest_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredAppState {
    pub selected_room_id: Option<RoomId>,
    pub selected_topic_id: Option<String>,
    pub selected_chat_id: Option<String>,
    pub paired_agent: Option<StoredPairedAgent>,
    pub revoked_devices: BTreeSet<DeviceRef>,
    pub chat_archives: Vec<StoredChatArchiveState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredChatArchiveState {
    pub room_id: RoomId,
    pub topic_id: String,
    pub chat_id: String,
    pub accepted_seq: u64,
    pub archived: bool,
}

impl StoredChatArchiveState {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_room_id(&self.room_id)?;
        validate_bytes_non_empty("app_state.chat_archive.topic_id", self.topic_id.len())?;
        validate_string_bytes(
            "app_state.chat_archive.topic_id",
            &self.topic_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("app_state.chat_archive.chat_id", self.chat_id.len())?;
        validate_string_bytes(
            "app_state.chat_archive.chat_id",
            &self.chat_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPairedAgent {
    pub agent_account_id: String,
    pub canonical_room_id: RoomId,
}

impl StoredAppState {
    fn validate_limits(&self) -> Result<(), ClientError> {
        if let Some(room_id) = &self.selected_room_id {
            validate_room_id(room_id)?;
        }
        if let Some(topic_id) = &self.selected_topic_id {
            validate_bytes_non_empty("app_state.selected_topic_id", topic_id.len())?;
            validate_string_bytes("app_state.selected_topic_id", topic_id, MAX_OBJECT_ID_BYTES)?;
        }
        if let Some(chat_id) = &self.selected_chat_id {
            validate_bytes_non_empty("app_state.selected_chat_id", chat_id.len())?;
            validate_string_bytes("app_state.selected_chat_id", chat_id, MAX_OBJECT_ID_BYTES)?;
        }
        if let Some(paired_agent) = &self.paired_agent {
            validate_bytes_non_empty(
                "app_state.paired_agent.agent_account_id",
                paired_agent.agent_account_id.len(),
            )?;
            validate_string_bytes(
                "app_state.paired_agent.agent_account_id",
                &paired_agent.agent_account_id,
                MAX_OBJECT_ID_BYTES,
            )?;
            validate_room_id(&paired_agent.canonical_room_id)?;
        }
        validate_item_count(
            "app_state.revoked_devices",
            self.revoked_devices.len(),
            MAX_STORED_APP_REVOKED_DEVICES,
        )?;
        for device in &self.revoked_devices {
            device.validate_limits()?;
        }
        validate_item_count(
            "app_state.chat_archives",
            self.chat_archives.len(),
            MAX_STORED_APP_CHAT_ARCHIVES,
        )?;
        for archive in &self.chat_archives {
            archive.validate_limits()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct StoredAppStateMetadataV1 {
    #[serde(deserialize_with = "deserialize_required_option")]
    selected_room_id: Option<RoomId>,
    #[serde(default)]
    selected_topic_id: Option<String>,
    #[serde(default)]
    selected_chat_id: Option<String>,
    #[serde(default)]
    paired_agent: Option<StoredPairedAgent>,
    revoked_devices: BTreeSet<DeviceRef>,
    #[serde(default)]
    chat_archives: Vec<StoredChatArchiveState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAppProfile {
    pub profile: NostrProfileRecord,
    pub stale: bool,
}

impl StoredAppProfile {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_nostr_profile_record(&self.profile)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredAppProfileMetadataV1 {
    profile: NostrProfileRecord,
    stale: bool,
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeLinkFanoutReport {
    pub discovery_pages: u32,
    pub queued_rooms: u32,
    pub claimed_key_packages: u32,
    pub prepared_commits: u32,
    pub submitted_commits: u32,
    pub completion_sync_pages: u32,
    pub completed_rooms: u32,
    pub applied_entries: Vec<RuntimeAppliedEntry>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomSyncCursor {
    pub room_id: RoomId,
    pub after_seq: u64,
    /// `None` = the device's home server (ADR 0005).
    pub server_url: Option<String>,
}

impl RuntimeSyncOptions {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_item_count(
            "runtime.key_package_target_available",
            self.key_package_target_available as usize,
            MAX_KEY_PACKAGES_PER_DEVICE,
        )?;
        validate_bytes_non_empty(
            "runtime.max_sync_pages_per_room",
            self.max_sync_pages_per_room as usize,
        )?;
        validate_item_count(
            "runtime.max_sync_pages_per_room",
            self.max_sync_pages_per_room as usize,
            MAX_RUNTIME_SYNC_PAGES_PER_ROOM,
        )?;
        Ok(())
    }
}

impl RuntimeLinkFanoutOptions {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_bytes_non_empty(
            "runtime_link_fanout.max_discovery_pages_per_tick",
            self.max_discovery_pages_per_tick as usize,
        )?;
        validate_item_count(
            "runtime_link_fanout.max_discovery_pages_per_tick",
            self.max_discovery_pages_per_tick as usize,
            MAX_RUNTIME_LINK_FANOUT_DISCOVERY_PAGES_PER_TICK,
        )?;
        validate_bytes_non_empty(
            "runtime_link_fanout.max_commit_rooms_per_tick",
            self.max_commit_rooms_per_tick as usize,
        )?;
        validate_item_count(
            "runtime_link_fanout.max_commit_rooms_per_tick",
            self.max_commit_rooms_per_tick as usize,
            MAX_RUNTIME_LINK_FANOUT_COMMITS_PER_TICK,
        )?;
        validate_bytes_non_empty(
            "runtime_link_fanout.max_completion_sync_pages_per_room",
            self.max_completion_sync_pages_per_room as usize,
        )?;
        validate_item_count(
            "runtime_link_fanout.max_completion_sync_pages_per_room",
            self.max_completion_sync_pages_per_room as usize,
            MAX_RUNTIME_SYNC_PAGES_PER_ROOM,
        )?;
        Ok(())
    }
}

impl RuntimeSyncReport {
    fn record_uploaded_key_package(&mut self) -> Result<(), ClientError> {
        self.uploaded_key_packages = self
            .uploaded_key_packages
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }

    fn record_claimed_welcomes(&mut self, count: usize) -> Result<(), ClientError> {
        let count = u32::try_from(count).map_err(|_| ClientError::RuntimeCounterOverflow)?;
        self.claimed_welcomes = self
            .claimed_welcomes
            .checked_add(count)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }

    fn record_welcome_ack(&mut self) -> Result<(), ClientError> {
        self.activated_welcome_acks_sent = self
            .activated_welcome_acks_sent
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }

    fn record_sync_page(&mut self) -> Result<(), ClientError> {
        self.sync_pages = self
            .sync_pages
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }
}

impl RuntimeLinkFanoutReport {
    fn record_discovery_page(&mut self) -> Result<(), ClientError> {
        self.discovery_pages = self
            .discovery_pages
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }

    fn record_queued_room(&mut self) -> Result<(), ClientError> {
        self.queued_rooms = self
            .queued_rooms
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }

    fn record_claimed_key_package(&mut self) -> Result<(), ClientError> {
        self.claimed_key_packages = self
            .claimed_key_packages
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }

    fn record_prepared_commit(&mut self) -> Result<(), ClientError> {
        self.prepared_commits = self
            .prepared_commits
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }

    fn record_submitted_commit(&mut self) -> Result<(), ClientError> {
        self.submitted_commits = self
            .submitted_commits
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }

    fn record_completion_sync_page(&mut self) -> Result<(), ClientError> {
        self.completion_sync_pages = self
            .completion_sync_pages
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }

    fn record_completed_room(&mut self) -> Result<(), ClientError> {
        self.completed_rooms = self
            .completed_rooms
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        Ok(())
    }
}

pub struct FiniteChatDevice {
    provider: OpenMlsRustCrypto,
    device_ref: DeviceRef,
    now_unix_seconds: u64,
    credential: FiniteDeviceCredentialV1,
    credential_with_key: CredentialWithKey,
    signer: SignatureKeyPair,
    groups: BTreeMap<RoomId, MlsGroup>,
    room_cursors: BTreeMap<RoomId, u64>,
    room_server_urls: BTreeMap<RoomId, String>,
    pending_welcomes: BTreeMap<WelcomeId, PendingWelcomeState>,
    pending_welcome_acks: BTreeMap<WelcomeId, PendingWelcomeAckState>,
    pending_key_package_uploads: BTreeMap<KeyPackageId, UploadKeyPackageRequest>,
    link_fanouts: BTreeMap<String, LinkFanoutState>,
}

impl FiniteChatDevice {
    pub fn new(config: FiniteChatDeviceConfig) -> Result<Self, ClientError> {
        let provider = OpenMlsRustCrypto::default();
        let signer = SignatureKeyPair::new(FINITECHAT_CIPHERSUITE.signature_algorithm())
            .map_err(|_| ClientError::CreateSigner)?;
        signer
            .store(provider.storage())
            .map_err(|_| ClientError::StoreSigner)?;

        let account_public_key = config.account_secret_key.public_key();
        let credential = FiniteDeviceCredentialV1::sign(
            &config.account_secret_key,
            config.device_id.clone(),
            signer.to_public_vec(),
            config.credential_not_before_unix_seconds,
            config.credential_not_after_unix_seconds,
        )?;
        credential.verify_expected(ExpectedDeviceCredential {
            account_public_key,
            device_id: &config.device_id,
            mls_leaf_signing_public_key: signer.public(),
            now_unix_seconds: config.now_unix_seconds,
        })?;

        let credential_with_key = credential.to_openmls_credential_with_key();
        let device_ref = DeviceRef {
            account_id: hex_lower(account_public_key.as_bytes()),
            device_id: config.device_id,
        };
        device_ref.validate_limits()?;

        let device = Self {
            provider,
            device_ref,
            now_unix_seconds: config.now_unix_seconds,
            credential,
            credential_with_key,
            signer,
            groups: BTreeMap::new(),
            room_cursors: BTreeMap::new(),
            room_server_urls: BTreeMap::new(),
            pending_welcomes: BTreeMap::new(),
            pending_welcome_acks: BTreeMap::new(),
            pending_key_package_uploads: BTreeMap::new(),
            link_fanouts: BTreeMap::new(),
        };
        debug_assert_eq!(
            device.credential_with_key.signature_key.as_slice(),
            device.signer.public()
        );
        Ok(device)
    }

    pub fn from_state(
        config: FiniteChatDeviceConfig,
        state: FiniteChatDeviceState,
    ) -> Result<Self, ClientError> {
        state.validate_limits()?;

        let provider = OpenMlsRustCrypto::default();
        {
            let mut values = provider
                .storage()
                .values
                .write()
                .map_err(|_| ClientError::OpenMlsStorageLock)?;
            values.clear();
            for record in &state.openmls_storage_records {
                values.insert(record.key.clone(), record.value.clone());
            }
        }

        let credential = FiniteDeviceCredentialV1::from_identity_bytes(&state.credential_identity)?;
        let account_public_key = config.account_secret_key.public_key();
        if credential.account_public_key() != account_public_key {
            return Err(ClientError::PersistedAccountMismatch);
        }
        if credential.device_id() != config.device_id {
            return Err(ClientError::PersistedDeviceMismatch);
        }
        credential.verify_expected(ExpectedDeviceCredential {
            account_public_key,
            device_id: &config.device_id,
            mls_leaf_signing_public_key: &state.signer_public_key,
            now_unix_seconds: config.now_unix_seconds,
        })?;

        let signer = SignatureKeyPair::read(
            provider.storage(),
            &state.signer_public_key,
            FINITECHAT_CIPHERSUITE.signature_algorithm(),
        )
        .ok_or(ClientError::MissingStoredSigner)?;
        if signer.public() != state.signer_public_key {
            return Err(ClientError::StoredSignerMismatch);
        }

        let credential_with_key = credential.to_openmls_credential_with_key();
        let device_ref = DeviceRef {
            account_id: hex_lower(account_public_key.as_bytes()),
            device_id: config.device_id,
        };
        if device_ref != state.device_ref {
            return Err(ClientError::PersistedDeviceMismatch);
        }

        let mut groups = BTreeMap::new();
        for room in &state.rooms {
            let group_id = GroupId::from_slice(room.mls_group_id.as_bytes());
            let group = MlsGroup::load(provider.storage(), &group_id)
                .map_err(|_| ClientError::LoadGroupState(room.room_id.clone()))?
                .ok_or_else(|| ClientError::MissingGroupState(room.room_id.clone()))?;
            if mls_group_id_string(group.group_id())? != room.mls_group_id {
                return Err(ClientError::PersistedGroupIdMismatch(room.room_id.clone()));
            }
            if groups.insert(room.room_id.clone(), group).is_some() {
                return Err(ClientError::DuplicatePersistedRoom(room.room_id.clone()));
            }
        }
        let room_server_urls = state
            .rooms
            .iter()
            .filter_map(|room| {
                room.server_url
                    .clone()
                    .map(|server_url| (room.room_id.clone(), server_url))
            })
            .collect::<BTreeMap<_, _>>();
        let room_cursors = state
            .rooms
            .iter()
            .map(|room| (room.room_id.clone(), room.last_applied_seq))
            .collect::<BTreeMap<_, _>>();
        let pending_welcomes = state
            .pending_welcomes
            .iter()
            .map(|welcome| (welcome.welcome_id.clone(), welcome.clone()))
            .collect::<BTreeMap<_, _>>();
        let pending_welcome_acks = state
            .pending_welcome_acks
            .iter()
            .map(|ack| (ack.welcome_id.clone(), ack.clone()))
            .collect::<BTreeMap<_, _>>();
        let pending_key_package_uploads = state
            .pending_key_package_uploads
            .iter()
            .map(|request| (request.key_package_id.clone(), request.clone()))
            .collect::<BTreeMap<_, _>>();
        let link_fanouts = state
            .link_fanouts
            .iter()
            .map(|fanout| (fanout.fanout_id.clone(), fanout.clone()))
            .collect::<BTreeMap<_, _>>();

        let device = Self {
            provider,
            device_ref,
            now_unix_seconds: config.now_unix_seconds,
            credential,
            credential_with_key,
            signer,
            groups,
            room_cursors,
            room_server_urls,
            pending_welcomes,
            pending_welcome_acks,
            pending_key_package_uploads,
            link_fanouts,
        };
        debug_assert_eq!(
            device.credential_with_key.signature_key.as_slice(),
            device.signer.public()
        );
        Ok(device)
    }

    pub fn export_state(&self) -> Result<FiniteChatDeviceState, ClientError> {
        let values = self
            .provider
            .storage()
            .values
            .read()
            .map_err(|_| ClientError::OpenMlsStorageLock)?;
        let mut openmls_storage_records = values
            .iter()
            .map(|(key, value)| OpenMlsStorageRecord {
                key: key.clone(),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        openmls_storage_records.sort_by(|left, right| left.key.cmp(&right.key));

        let rooms = self
            .groups
            .iter()
            .map(|(room_id, group)| {
                Ok(PersistedRoomState {
                    room_id: room_id.clone(),
                    mls_group_id: mls_group_id_string(group.group_id())?,
                    last_applied_seq: *self.room_cursors.get(room_id).unwrap_or(&0),
                    server_url: self.room_server_urls.get(room_id).cloned(),
                })
            })
            .collect::<Result<Vec<_>, ClientError>>()?;
        // self.groups is a BTreeMap, so rooms are already sorted by room_id.
        let pending_welcomes = self.pending_welcomes.values().cloned().collect::<Vec<_>>();
        let pending_welcome_acks = self
            .pending_welcome_acks
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let pending_key_package_uploads = self
            .pending_key_package_uploads
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let link_fanouts = self.link_fanouts.values().cloned().collect::<Vec<_>>();

        let state = FiniteChatDeviceState {
            device_ref: self.device_ref.clone(),
            signer_public_key: self.signer.public().to_vec(),
            credential_identity: self.credential.identity_bytes(),
            rooms,
            pending_welcomes,
            pending_welcome_acks,
            pending_key_package_uploads,
            link_fanouts,
            openmls_storage_records,
        };
        state.validate_limits()?;
        Ok(state)
    }

    pub fn device_ref(&self) -> &DeviceRef {
        &self.device_ref
    }

    pub fn set_now_unix_seconds(&mut self, now_unix_seconds: u64) {
        self.now_unix_seconds = now_unix_seconds;
    }

    pub fn create_group_state(
        &mut self,
        room_id: impl Into<RoomId>,
        mls_group_id: impl AsRef<str>,
    ) -> Result<(), ClientError> {
        let room_id = room_id.into();
        if self.groups.contains_key(&room_id) {
            return Err(ClientError::GroupAlreadyExists(room_id));
        }

        let group = MlsGroup::new_with_group_id(
            &self.provider,
            &self.signer,
            &openmls_group_config(),
            GroupId::from_slice(mls_group_id.as_ref().as_bytes()),
            self.credential_with_key.clone(),
        )
        .map_err(|_| ClientError::CreateGroup)?;
        self.groups.insert(room_id.clone(), group);
        self.room_cursors.insert(room_id, 0);
        Ok(())
    }

    pub fn upload_key_package_request(
        &self,
        key_package_id: impl Into<String>,
    ) -> Result<UploadKeyPackageRequest, ClientError> {
        self.build_upload_key_package_request(Some(key_package_id.into()))
    }

    pub fn upload_key_package_auto_id_request(
        &self,
    ) -> Result<UploadKeyPackageRequest, ClientError> {
        self.build_upload_key_package_request(None)
    }

    fn build_upload_key_package_request(
        &self,
        key_package_id: Option<String>,
    ) -> Result<UploadKeyPackageRequest, ClientError> {
        let key_package = KeyPackage::builder()
            .build(
                FINITECHAT_CIPHERSUITE,
                &self.provider,
                &self.signer,
                self.credential_with_key.clone(),
            )
            .map_err(|_| ClientError::BuildKeyPackage)?;
        let payload = key_package
            .key_package()
            .tls_serialize_detached()
            .map_err(|_| ClientError::SerializeKeyPackage)?;
        let key_package_ref = key_package
            .key_package()
            .hash_ref(self.provider.crypto())
            .map_err(|_| ClientError::HashKeyPackageRef)?;
        let key_package_hash = message_id_for_bytes(&payload);
        let key_package_id = key_package_id
            .unwrap_or_else(|| format!("kp_t{:020}_{key_package_hash}", self.now_unix_seconds));

        let request = UploadKeyPackageRequest {
            key_package_id,
            owner: self.device_ref.clone(),
            key_package_ref: hex_lower(key_package_ref.as_slice()),
            key_package_hash,
            key_package_payload: payload,
        };
        request.validate_limits()?;
        Ok(request)
    }

    pub fn key_package_replenishment_plan(
        &mut self,
        inventory: KeyPackageInventory,
        target_available: u32,
    ) -> Result<KeyPackageReplenishmentPlan, ClientError> {
        inventory.owner.validate_limits()?;
        if inventory.owner != self.device_ref {
            return Err(ClientError::KeyPackageInventoryOwnerMismatch {
                expected: self.device_ref.clone(),
                actual: inventory.owner,
            });
        }
        validate_item_count(
            "key_package_replenishment.target_available",
            target_available as usize,
            MAX_KEY_PACKAGES_PER_DEVICE,
        )?;
        validate_item_count(
            "key_package_replenishment.available",
            inventory.available as usize,
            MAX_KEY_PACKAGES_PER_DEVICE,
        )?;
        validate_item_count(
            "key_package_replenishment.leased",
            inventory.leased as usize,
            MAX_KEY_PACKAGES_PER_DEVICE,
        )?;
        if inventory.unconsumed() > u64::from(MAX_KEY_PACKAGES_PER_DEVICE) {
            return Err(ClientError::KeyPackageInventoryOverCap {
                available: inventory.available,
                leased: inventory.leased,
                max: MAX_KEY_PACKAGES_PER_DEVICE,
            });
        }

        validate_item_count(
            "key_package_replenishment.pending_uploads",
            self.pending_key_package_uploads.len(),
            MAX_PENDING_KEY_PACKAGE_UPLOADS,
        )?;
        let pending_upload_count = self.pending_key_package_uploads.len() as u32;
        let unconsumed_with_pending = inventory
            .unconsumed()
            .checked_add(u64::from(pending_upload_count))
            .ok_or(ClientError::KeyPackageInventoryOverCap {
                available: inventory.available,
                leased: inventory.leased,
                max: MAX_KEY_PACKAGES_PER_DEVICE,
            })?;
        if unconsumed_with_pending > u64::from(MAX_KEY_PACKAGES_PER_DEVICE) {
            return Err(ClientError::KeyPackagePendingUploadOverCap {
                available: inventory.available,
                leased: inventory.leased,
                pending: pending_upload_count,
                max: MAX_KEY_PACKAGES_PER_DEVICE,
            });
        }

        let missing_available = target_available
            .saturating_sub(inventory.available)
            .saturating_sub(pending_upload_count);
        let remaining_cap = u32::try_from(
            u64::from(MAX_KEY_PACKAGES_PER_DEVICE)
                .checked_sub(unconsumed_with_pending)
                .ok_or(ClientError::KeyPackageInventoryOverCap {
                    available: inventory.available,
                    leased: inventory.leased,
                    max: MAX_KEY_PACKAGES_PER_DEVICE,
                })?,
        )
        .map_err(|_| ClientError::KeyPackageInventoryOverCap {
            available: inventory.available,
            leased: inventory.leased,
            max: MAX_KEY_PACKAGES_PER_DEVICE,
        })?;
        let upload_count = missing_available.min(remaining_cap);
        let mut upload_requests =
            Vec::with_capacity(self.pending_key_package_uploads.len() + upload_count as usize);
        upload_requests.extend(self.pending_key_package_uploads());
        for _ in 0..upload_count {
            let request = self.upload_key_package_auto_id_request()?;
            self.store_pending_key_package_upload(request.clone())?;
            upload_requests.push(request);
        }

        debug_assert!(upload_requests.len() <= MAX_KEY_PACKAGES_PER_DEVICE as usize);
        debug_assert!(
            upload_requests
                .iter()
                .all(|request| request.owner == self.device_ref)
        );
        debug_assert_eq!(
            upload_requests.len(),
            self.pending_key_package_uploads.len()
        );
        Ok(KeyPackageReplenishmentPlan {
            inventory,
            target_available,
            upload_requests,
        })
    }

    pub fn prepare_add_member_commit(
        &mut self,
        room_id: &str,
        claimed_key_package: &ClaimKeyPackageResult,
        welcome_id: impl Into<WelcomeId>,
        idempotency_key: impl Into<String>,
    ) -> Result<PreparedCommit, ClientError> {
        let welcome_ids = [welcome_id.into()];
        self.prepare_add_members_commit(
            room_id,
            std::slice::from_ref(claimed_key_package),
            &welcome_ids,
            idempotency_key,
        )
    }

    pub fn prepare_add_members_commit(
        &mut self,
        room_id: &str,
        claimed_key_packages: &[ClaimKeyPackageResult],
        welcome_ids: &[WelcomeId],
        idempotency_key: impl Into<String>,
    ) -> Result<PreparedCommit, ClientError> {
        validate_room_id(room_id)?;
        let idempotency_key = idempotency_key.into();
        validate_idempotency_key(&idempotency_key)?;
        if claimed_key_packages.is_empty() {
            return Err(ClientError::EmptyWelcomeBatch);
        }
        if claimed_key_packages.len() != welcome_ids.len() {
            return Err(ClientError::WelcomeBatchCountMismatch {
                key_packages: claimed_key_packages.len(),
                welcome_ids: welcome_ids.len(),
            });
        }
        finitechat_proto::validate_item_count(
            "claimed_key_packages",
            claimed_key_packages.len(),
            MAX_STAGED_WELCOMES_PER_COMMIT,
        )?;

        let mut seen_devices = BTreeSet::<DeviceRef>::new();
        let mut seen_key_packages = BTreeSet::<KeyPackageId>::new();
        let mut seen_welcomes = BTreeSet::<WelcomeId>::new();
        for claimed_key_package in claimed_key_packages {
            claimed_key_package
                .owner
                .validate_limits()
                .map_err(ClientError::from)?;
            validate_string_bytes(
                "key_package_id",
                &claimed_key_package.key_package_id,
                MAX_OBJECT_ID_BYTES,
            )?;
            validate_string_bytes(
                "key_package_ref",
                &claimed_key_package.key_package_ref,
                MAX_OBJECT_ID_BYTES,
            )?;
            validate_string_bytes(
                "key_package_hash",
                &claimed_key_package.key_package_hash,
                MAX_OBJECT_ID_BYTES,
            )?;
            if !seen_devices.insert(claimed_key_package.owner.clone()) {
                return Err(ClientError::DuplicateWelcomeBatchDevice(
                    claimed_key_package.owner.clone(),
                ));
            }
            if !seen_key_packages.insert(claimed_key_package.key_package_id.clone()) {
                return Err(ClientError::DuplicateWelcomeBatchKeyPackage(
                    claimed_key_package.key_package_id.clone(),
                ));
            }
        }
        for welcome_id in welcome_ids {
            validate_string_bytes("welcome_id", welcome_id, MAX_OBJECT_ID_BYTES)?;
            if !seen_welcomes.insert(welcome_id.clone()) {
                return Err(ClientError::DuplicateWelcomeBatchWelcome(
                    welcome_id.clone(),
                ));
            }
        }

        let mut key_packages = Vec::with_capacity(claimed_key_packages.len());
        for claimed_key_package in claimed_key_packages {
            key_packages.push(verified_key_package_from_claim(
                &self.provider,
                claimed_key_package,
                self.now_unix_seconds,
            )?);
        }
        let provider = &self.provider;
        let signer = &self.signer;
        let sender = self.device_ref.clone();
        let group = self
            .groups
            .get_mut(room_id)
            .ok_or_else(|| ClientError::GroupNotFound(room_id.to_string()))?;
        if group.pending_commit().is_some() {
            return Err(ClientError::PendingCommitExists(room_id.to_string()));
        }

        let (commit_message, welcome_message, _group_info) = group
            .add_members(provider, signer, &key_packages)
            .map_err(|_| ClientError::AddMember)?;
        let commit_payload = mls_message_out_bytes(commit_message)?;
        let welcome_payload = mls_message_out_bytes(welcome_message)?;
        let ratchet_tree = group
            .pending_commit()
            .ok_or_else(|| ClientError::MissingPendingCommit(room_id.to_string()))?
            .export_ratchet_tree(provider.crypto(), group.export_ratchet_tree())
            .map_err(|_| ClientError::ExportPendingRatchetTree)?
            .ok_or(ClientError::ExportPendingRatchetTree)?;
        let ratchet_tree_payload = ratchet_tree
            .tls_serialize_detached()
            .map_err(|_| ClientError::SerializeRatchetTree)?;
        let expected_epoch = group.epoch().as_u64();
        let mls_group_id = mls_group_id_string(group.group_id())?;
        let commit_envelope = envelope(
            room_id.to_string(),
            mls_group_id,
            sender.clone(),
            expected_epoch,
            LogEntryKind::Commit,
            commit_payload,
        );
        let commit_message_id = commit_envelope
            .message_id()
            .map_err(ClientError::EnvelopeMessageId)?;
        let mut adds = Vec::with_capacity(claimed_key_packages.len());
        let mut staged_welcomes = Vec::with_capacity(claimed_key_packages.len());
        for (claimed_key_package, welcome_id) in claimed_key_packages.iter().zip(welcome_ids) {
            adds.push(MembershipAddV1 {
                device: claimed_key_package.owner.clone(),
                key_package_id: claimed_key_package.key_package_id.clone(),
                key_package_ref: claimed_key_package.key_package_ref.clone(),
                key_package_hash: claimed_key_package.key_package_hash.clone(),
                welcome_id: welcome_id.clone(),
            });
            staged_welcomes.push(StagedWelcomeV1 {
                welcome_id: welcome_id.clone(),
                welcome_payload: welcome_payload.clone(),
                ratchet_tree_payload: ratchet_tree_payload.clone(),
            });
        }
        debug_assert_eq!(adds.len(), staged_welcomes.len());
        debug_assert!(!adds.is_empty());
        let request = SubmitCommitRequest {
            room_id: room_id.to_string(),
            sender,
            expected_epoch,
            envelope: commit_envelope,
            membership_delta: MembershipDeltaV1 {
                base_epoch: expected_epoch,
                post_commit_epoch: expected_epoch + 1,
                commit_message_id: commit_message_id.clone(),
                adds,
                removes: vec![],
            },
            staged_welcomes,
            idempotency_key,
        };
        request.validate_limits()?;
        Ok(PreparedCommit {
            request,
            message_id: commit_message_id,
        })
    }

    pub fn prepare_remove_member_commit(
        &mut self,
        room_id: &str,
        removed_device: &DeviceRef,
        idempotency_key: impl Into<String>,
    ) -> Result<PreparedCommit, ClientError> {
        validate_room_id(room_id)?;
        removed_device.validate_limits()?;
        if removed_device == &self.device_ref {
            return Err(ClientError::CannotRemoveSelf);
        }
        let idempotency_key = idempotency_key.into();
        validate_idempotency_key(&idempotency_key)?;

        let provider = &self.provider;
        let signer = &self.signer;
        let sender = self.device_ref.clone();
        let now_unix_seconds = self.now_unix_seconds;
        let group = self
            .groups
            .get_mut(room_id)
            .ok_or_else(|| ClientError::GroupNotFound(room_id.to_string()))?;
        if group.pending_commit().is_some() {
            return Err(ClientError::PendingCommitExists(room_id.to_string()));
        }
        let removed_leaf_index =
            verified_member_leaf_index(group, removed_device, now_unix_seconds)?;

        let (commit_message, welcome_message, _group_info) = group
            .remove_members(provider, signer, &[removed_leaf_index])
            .map_err(|_| ClientError::RemoveMember)?;
        if welcome_message.is_some() {
            return Err(ClientError::UnexpectedWelcomeForNonAddCommit);
        }
        let commit_payload = mls_message_out_bytes(commit_message)?;
        let expected_epoch = group.epoch().as_u64();
        let mls_group_id = mls_group_id_string(group.group_id())?;
        let commit_envelope = envelope(
            room_id.to_string(),
            mls_group_id,
            sender.clone(),
            expected_epoch,
            LogEntryKind::Commit,
            commit_payload,
        );
        let commit_message_id = commit_envelope
            .message_id()
            .map_err(ClientError::EnvelopeMessageId)?;
        let request = SubmitCommitRequest {
            room_id: room_id.to_string(),
            sender,
            expected_epoch,
            envelope: commit_envelope,
            membership_delta: MembershipDeltaV1 {
                base_epoch: expected_epoch,
                post_commit_epoch: post_commit_epoch(expected_epoch)?,
                commit_message_id: commit_message_id.clone(),
                adds: vec![],
                removes: vec![MembershipRemoveV1 {
                    device: removed_device.clone(),
                    removed_leaf_index: removed_leaf_index.u32(),
                }],
            },
            staged_welcomes: vec![],
            idempotency_key,
        };
        request.validate_limits()?;
        Ok(PreparedCommit {
            request,
            message_id: commit_message_id,
        })
    }

    pub fn prepare_self_update_commit(
        &mut self,
        room_id: &str,
        idempotency_key: impl Into<String>,
    ) -> Result<PreparedCommit, ClientError> {
        validate_room_id(room_id)?;
        let idempotency_key = idempotency_key.into();
        validate_idempotency_key(&idempotency_key)?;

        let provider = &self.provider;
        let signer = &self.signer;
        let sender = self.device_ref.clone();
        let group = self
            .groups
            .get_mut(room_id)
            .ok_or_else(|| ClientError::GroupNotFound(room_id.to_string()))?;
        if group.pending_commit().is_some() {
            return Err(ClientError::PendingCommitExists(room_id.to_string()));
        }

        let (commit_message, welcome_message, _group_info) = group
            .self_update(provider, signer, LeafNodeParameters::default())
            .map_err(|_| ClientError::SelfUpdate)?
            .into_messages();
        if welcome_message.is_some() {
            return Err(ClientError::UnexpectedWelcomeForNonAddCommit);
        }
        let commit_payload = mls_message_out_bytes(commit_message)?;
        let expected_epoch = group.epoch().as_u64();
        let mls_group_id = mls_group_id_string(group.group_id())?;
        let commit_envelope = envelope(
            room_id.to_string(),
            mls_group_id,
            sender.clone(),
            expected_epoch,
            LogEntryKind::Commit,
            commit_payload,
        );
        let commit_message_id = commit_envelope
            .message_id()
            .map_err(ClientError::EnvelopeMessageId)?;
        let request = SubmitCommitRequest {
            room_id: room_id.to_string(),
            sender,
            expected_epoch,
            envelope: commit_envelope,
            membership_delta: MembershipDeltaV1 {
                base_epoch: expected_epoch,
                post_commit_epoch: post_commit_epoch(expected_epoch)?,
                commit_message_id: commit_message_id.clone(),
                adds: vec![],
                removes: vec![],
            },
            staged_welcomes: vec![],
            idempotency_key,
        };
        request.validate_limits()?;
        Ok(PreparedCommit {
            request,
            message_id: commit_message_id,
        })
    }

    pub fn merge_pending_commit_from_log(
        &mut self,
        room_id: &str,
        entries: &[RoomLogEntry],
        message_id: &str,
    ) -> Result<(), ClientError> {
        let sender = self.device_ref.clone();
        let observed = entries.iter().any(|entry| {
            entry.message_id == message_id
                && entry.kind == LogEntryKind::Commit
                && entry.sender == sender
        });
        if !observed {
            return Err(ClientError::PendingCommitNotObserved(
                message_id.to_string(),
            ));
        }

        let provider = &self.provider;
        let group = self
            .groups
            .get_mut(room_id)
            .ok_or_else(|| ClientError::GroupNotFound(room_id.to_string()))?;
        if group.pending_commit().is_none() {
            return Err(ClientError::MissingPendingCommit(room_id.to_string()));
        }
        group
            .merge_pending_commit(provider)
            .map_err(|_| ClientError::MergePendingCommit)?;
        debug_assert!(group.pending_commit().is_none());
        Ok(())
    }

    pub fn apply_log_entry(
        &mut self,
        room_id: &str,
        entry: &RoomLogEntry,
    ) -> Result<AppliedLogEntry, ClientError> {
        match entry.kind {
            LogEntryKind::Application => {
                let decrypted = self.decrypt_application_entry(room_id, entry)?;
                Ok(AppliedLogEntry::Application {
                    plaintext: decrypted.plaintext,
                    sender: decrypted.sender,
                })
            }
            LogEntryKind::Commit => {
                self.apply_commit_entry(room_id, entry)?;
                Ok(AppliedLogEntry::Commit {
                    sender: entry.sender.clone(),
                    epoch: post_commit_epoch(entry.epoch)?,
                })
            }
            LogEntryKind::Proposal => Err(ClientError::UnsupportedLogEntryKind(entry.kind)),
        }
    }

    pub fn apply_commit_entry(
        &mut self,
        room_id: &str,
        entry: &RoomLogEntry,
    ) -> Result<(), ClientError> {
        validate_log_entry_shape(room_id, entry, LogEntryKind::Commit)?;
        let post_commit_epoch = post_commit_epoch(entry.epoch)?;
        let own_device_ref = self.device_ref.clone();
        let now_unix_seconds = self.now_unix_seconds;
        let provider = &self.provider;
        let group = self
            .groups
            .get_mut(room_id)
            .ok_or_else(|| ClientError::GroupNotFound(room_id.to_string()))?;
        let current_epoch = group.epoch().as_u64();
        if current_epoch != entry.epoch {
            return Err(ClientError::UnexpectedCommitEpoch {
                room_id: room_id.to_string(),
                current_epoch,
                entry_epoch: entry.epoch,
            });
        }

        if entry.sender == own_device_ref {
            if group.pending_commit().is_none() {
                return Err(ClientError::OwnCommitWithoutPendingState(
                    entry.message_id.clone(),
                ));
            }
            group
                .merge_pending_commit(provider)
                .map_err(|_| ClientError::MergePendingCommit)?;
            if group.epoch().as_u64() != post_commit_epoch {
                return Err(ClientError::UnexpectedPostCommitEpoch {
                    room_id: room_id.to_string(),
                    expected_epoch: post_commit_epoch,
                    actual_epoch: group.epoch().as_u64(),
                });
            }
            debug_assert!(group.pending_commit().is_none());
            return Ok(());
        }

        if group.pending_commit().is_some() {
            group
                .clear_pending_commit(provider.storage())
                .map_err(|_| ClientError::ClearPendingCommit)?;
        }

        let processed = group
            .process_message(
                provider,
                protocol_message_from_bytes(&entry.envelope.payload)?,
            )
            .map_err(|error| ClientError::ProcessMessage {
                reason: format!("{error:?}"),
            })?;
        let ProcessedMessageContent::StagedCommitMessage(staged_commit) = processed.into_content()
        else {
            return Err(ClientError::UnexpectedMessage);
        };
        verify_staged_commit_credentials(now_unix_seconds, &staged_commit)?;
        group
            .merge_staged_commit(provider, *staged_commit)
            .map_err(|_| ClientError::MergeStagedCommit)?;
        if group.epoch().as_u64() != post_commit_epoch {
            return Err(ClientError::UnexpectedPostCommitEpoch {
                room_id: room_id.to_string(),
                expected_epoch: post_commit_epoch,
                actual_epoch: group.epoch().as_u64(),
            });
        }
        debug_assert!(group.pending_commit().is_none());
        Ok(())
    }

    pub fn activate_welcome(
        &mut self,
        room_id: impl Into<RoomId>,
        welcome_payload: &[u8],
        ratchet_tree_payload: &[u8],
    ) -> Result<(), ClientError> {
        let room_id = room_id.into();
        if self.groups.contains_key(&room_id) {
            return Err(ClientError::GroupAlreadyExists(room_id));
        }

        let group_config = openmls_group_config();
        let group = StagedWelcome::new_from_welcome(
            &self.provider,
            group_config.join_config(),
            welcome_from_bytes(welcome_payload)?,
            Some(ratchet_tree_from_bytes(ratchet_tree_payload)?),
        )
        .map_err(|_| ClientError::StageWelcome)?
        .into_group(&self.provider)
        .map_err(|_| ClientError::ActivateWelcome)?;
        self.verify_member_in_group(&group, &self.device_ref)?;
        self.groups.insert(room_id.clone(), group);
        self.room_cursors.insert(room_id, 0);
        Ok(())
    }

    pub fn last_applied_seq(&self, room_id: &str) -> Result<u64, ClientError> {
        validate_room_id(room_id)?;
        self.group(room_id)?;
        Ok(*self.room_cursors.get(room_id).unwrap_or(&0))
    }

    pub fn room_sync_cursors(&self) -> Vec<RoomSyncCursor> {
        self.groups
            .keys()
            .map(|room_id| RoomSyncCursor {
                room_id: room_id.clone(),
                after_seq: *self.room_cursors.get(room_id).unwrap_or(&0),
                server_url: self.room_server_urls.get(room_id).cloned(),
            })
            .collect()
    }

    /// Record which server hosts a room's ordered log. `None` means the
    /// home server. Rooms activated from a Welcome record the room server
    /// that stored the Welcome.
    pub fn set_room_server_url(
        &mut self,
        room_id: &str,
        server_url: Option<String>,
    ) -> Result<(), ClientError> {
        validate_room_id(room_id)?;
        self.group(room_id)?;
        if let Some(server_url) = &server_url {
            validate_string_bytes("room.server_url", server_url, MAX_ROOM_SERVER_URL_BYTES)?;
        }
        match server_url {
            Some(server_url) => {
                self.room_server_urls.insert(room_id.to_owned(), server_url);
            }
            None => {
                self.room_server_urls.remove(room_id);
            }
        }
        Ok(())
    }

    pub fn room_server_url(&self, room_id: &str) -> Option<&str> {
        self.room_server_urls.get(room_id).map(String::as_str)
    }

    pub fn pending_welcome_count(&self) -> usize {
        self.pending_welcomes.len()
    }

    pub fn pending_welcome_ack_count(&self) -> usize {
        self.pending_welcome_acks.len()
    }

    pub fn pending_key_package_upload_count(&self) -> usize {
        self.pending_key_package_uploads.len()
    }

    fn store_pending_welcome(&mut self, welcome: &WelcomeRecord) -> Result<(), ClientError> {
        if welcome.recipient != self.device_ref {
            return Err(ClientError::PendingWelcomeRecipientMismatch);
        }
        let pending = PendingWelcomeState::from_welcome_record(welcome)?;
        if self.pending_welcomes.contains_key(&pending.welcome_id) {
            return Err(ClientError::DuplicatePendingWelcome(pending.welcome_id));
        }
        self.pending_welcomes
            .insert(pending.welcome_id.clone(), pending);
        debug_assert!(!self.pending_welcomes.is_empty());
        Ok(())
    }

    fn pending_welcome_ids(&self) -> Vec<WelcomeId> {
        self.pending_welcomes.keys().cloned().collect()
    }

    fn pending_welcome(&self, welcome_id: &str) -> Result<PendingWelcomeState, ClientError> {
        validate_string_bytes("welcome_id", welcome_id, MAX_OBJECT_ID_BYTES)?;
        self.pending_welcomes
            .get(welcome_id)
            .cloned()
            .ok_or_else(|| ClientError::PendingWelcomeNotFound(welcome_id.to_string()))
    }

    fn clear_pending_welcome(&mut self, welcome_id: &str) -> Result<(), ClientError> {
        validate_string_bytes("welcome_id", welcome_id, MAX_OBJECT_ID_BYTES)?;
        if self.pending_welcomes.remove(welcome_id).is_none() {
            return Err(ClientError::PendingWelcomeNotFound(welcome_id.to_string()));
        }
        debug_assert!(!self.pending_welcomes.contains_key(welcome_id));
        Ok(())
    }

    fn activate_pending_welcome(
        &mut self,
        welcome_id: &str,
    ) -> Result<PendingWelcomeAckState, ClientError> {
        let pending = self.pending_welcome(welcome_id)?;
        self.activate_welcome(
            pending.room_id.clone(),
            &pending.welcome_payload,
            &pending.ratchet_tree_payload,
        )?;
        self.set_last_applied_seq(&pending.room_id, pending.commit_seq)?;
        self.clear_pending_welcome(welcome_id)?;
        let ack = PendingWelcomeAckState::from_pending_welcome(&pending)?;
        self.store_pending_welcome_ack(ack.clone())?;
        debug_assert!(!self.pending_welcomes.contains_key(welcome_id));
        debug_assert!(self.pending_welcome_acks.contains_key(welcome_id));
        Ok(ack)
    }

    fn store_pending_welcome_ack(
        &mut self,
        ack: PendingWelcomeAckState,
    ) -> Result<(), ClientError> {
        ack.validate_limits()?;
        if self.pending_welcomes.contains_key(&ack.welcome_id) {
            return Err(ClientError::PendingWelcomeAlsoNeedsAck(ack.welcome_id));
        }
        if self
            .pending_welcome_acks
            .insert(ack.welcome_id.clone(), ack.clone())
            .is_some()
        {
            return Err(ClientError::DuplicatePendingWelcomeAck(ack.welcome_id));
        }
        debug_assert!(self.pending_welcome_acks.contains_key(&ack.welcome_id));
        Ok(())
    }

    fn pending_welcome_acks(&self) -> Vec<PendingWelcomeAckState> {
        self.pending_welcome_acks.values().cloned().collect()
    }

    fn clear_pending_welcome_ack(&mut self, welcome_id: &str) -> Result<(), ClientError> {
        validate_string_bytes("welcome_ack.welcome_id", welcome_id, MAX_OBJECT_ID_BYTES)?;
        if self.pending_welcome_acks.remove(welcome_id).is_none() {
            return Err(ClientError::PendingWelcomeAckNotFound(
                welcome_id.to_string(),
            ));
        }
        debug_assert!(!self.pending_welcome_acks.contains_key(welcome_id));
        Ok(())
    }

    fn store_pending_key_package_upload(
        &mut self,
        request: UploadKeyPackageRequest,
    ) -> Result<(), ClientError> {
        request.validate_limits()?;
        if request.owner != self.device_ref {
            return Err(ClientError::PendingKeyPackageUploadOwnerMismatch {
                expected: self.device_ref.clone(),
                actual: request.owner,
            });
        }
        if self
            .pending_key_package_uploads
            .insert(request.key_package_id.clone(), request.clone())
            .is_some()
        {
            return Err(ClientError::DuplicatePendingKeyPackageUpload(
                request.key_package_id,
            ));
        }
        debug_assert!(
            self.pending_key_package_uploads
                .contains_key(&request.key_package_id)
        );
        Ok(())
    }

    fn pending_key_package_uploads(&self) -> Vec<UploadKeyPackageRequest> {
        self.pending_key_package_uploads.values().cloned().collect()
    }

    fn clear_pending_key_package_upload(
        &mut self,
        key_package_id: &str,
    ) -> Result<(), ClientError> {
        validate_string_bytes(
            "pending_key_package_upload.key_package_id",
            key_package_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        if self
            .pending_key_package_uploads
            .remove(key_package_id)
            .is_none()
        {
            return Err(ClientError::PendingKeyPackageUploadNotFound(
                key_package_id.to_string(),
            ));
        }
        debug_assert!(
            !self
                .pending_key_package_uploads
                .contains_key(key_package_id)
        );
        Ok(())
    }

    pub fn start_link_fanout(
        &mut self,
        fanout_id: impl Into<String>,
        target_device: DeviceRef,
    ) -> Result<(), ClientError> {
        let fanout_id = fanout_id.into();
        validate_string_bytes("link_fanout.fanout_id", &fanout_id, MAX_OBJECT_ID_BYTES)?;
        target_device.validate_limits()?;
        if target_device.account_id != self.device_ref.account_id {
            return Err(ClientError::LinkFanoutAccountMismatch {
                expected: self.device_ref.account_id.clone(),
                actual: target_device.account_id,
            });
        }
        if target_device == self.device_ref {
            return Err(ClientError::LinkFanoutCannotTargetSelf);
        }
        if self.link_fanouts.contains_key(&fanout_id) {
            return Err(ClientError::DuplicateLinkFanout(fanout_id));
        }
        let fanout = LinkFanoutState {
            fanout_id: fanout_id.clone(),
            target_device,
            after_room_id: None,
            discovery_complete: false,
            rooms: Vec::new(),
            bootstrap_export: LinkBootstrapExportState::WaitingForFanout,
        };
        fanout.validate_limits()?;
        self.link_fanouts.insert(fanout_id, fanout);
        debug_assert!(!self.link_fanouts.is_empty());
        Ok(())
    }

    pub fn queue_link_fanout_page(
        &mut self,
        fanout_id: &str,
        page: &ListAccountRoomsPage,
        plans: &[LinkFanoutRoomPlan],
    ) -> Result<(), ClientError> {
        page.validate_limits().map_err(ClientError::from)?;
        validate_item_count("link_fanout.page_plans", plans.len(), MAX_LINK_FANOUT_ROOMS)?;
        let target_device = self.link_fanout(fanout_id)?.target_device.clone();
        let mut plans_by_room = BTreeMap::<RoomId, LinkFanoutRoomPlan>::new();
        for plan in plans {
            plan.validate_limits()?;
            if plans_by_room
                .insert(plan.room_id.clone(), plan.clone())
                .is_some()
            {
                return Err(ClientError::DuplicateLinkFanoutRoom(plan.room_id.clone()));
            }
        }

        let mut queued = Vec::new();
        for room in &page.rooms {
            let target_already_current = room
                .devices
                .iter()
                .any(|device| device.device == target_device);
            if target_already_current {
                continue;
            }
            self.group(&room.room_id)?;
            let plan = plans_by_room
                .remove(&room.room_id)
                .ok_or_else(|| ClientError::MissingLinkFanoutRoomPlan(room.room_id.clone()))?;
            if plan.key_package_id.is_empty()
                || plan.welcome_id.is_empty()
                || plan.idempotency_key.is_empty()
            {
                return Err(ClientError::MissingLinkFanoutRoomPlan(plan.room_id));
            }
            queued.push(LinkFanoutRoomState {
                plan,
                claimed_key_package: None,
                status: LinkFanoutRoomStatus::Pending,
            });
        }
        if let Some((extra_room_id, _)) = plans_by_room.into_iter().next() {
            return Err(ClientError::UnexpectedLinkFanoutRoomPlan(extra_room_id));
        }

        let fanout = self.link_fanout_mut(fanout_id)?;
        let mut seen_rooms = fanout
            .rooms
            .iter()
            .map(|room| room.plan.room_id.clone())
            .collect::<BTreeSet<_>>();
        for room in queued {
            if !seen_rooms.insert(room.plan.room_id.clone()) {
                return Err(ClientError::DuplicateLinkFanoutRoom(
                    room.plan.room_id.clone(),
                ));
            }
            fanout.rooms.push(room);
        }
        fanout.after_room_id = page.next_after_room_id.clone();
        fanout.discovery_complete = !page.has_more;
        fanout.validate_limits()?;
        Ok(())
    }

    pub fn queue_claimed_link_fanout_page(
        &mut self,
        fanout_id: &str,
        page: &ListAccountRoomsPage,
        claimed_key_packages: &[ClaimKeyPackageResult],
    ) -> Result<(), ClientError> {
        page.validate_limits().map_err(ClientError::from)?;
        validate_item_count(
            "link_fanout.claimed_key_packages",
            claimed_key_packages.len(),
            MAX_LINK_FANOUT_ROOMS,
        )?;
        let fanout = self.link_fanout(fanout_id)?;
        let target_device = fanout.target_device.clone();
        let mut claims = claimed_key_packages.iter();
        let mut queued = Vec::new();
        for room in &page.rooms {
            let target_already_current = room
                .devices
                .iter()
                .any(|device| device.device == target_device);
            if target_already_current {
                continue;
            }
            self.group(&room.room_id)?;
            let Some(claimed_key_package) = claims.next() else {
                return Err(ClientError::MissingLinkFanoutClaim(room.room_id.clone()));
            };
            validate_claimed_key_package(claimed_key_package)?;
            if claimed_key_package.owner != target_device {
                return Err(ClientError::LinkFanoutClaimTargetMismatch {
                    expected: target_device.clone(),
                    actual: claimed_key_package.owner.clone(),
                });
            }
            let plan = LinkFanoutRoomPlan {
                room_id: room.room_id.clone(),
                key_package_id: claimed_key_package.key_package_id.clone(),
                welcome_id: link_fanout_derived_id(
                    "welcome",
                    fanout_id,
                    &room.room_id,
                    &claimed_key_package.key_package_id,
                )?,
                idempotency_key: link_fanout_derived_id(
                    "link",
                    fanout_id,
                    &room.room_id,
                    &claimed_key_package.key_package_id,
                )?,
            };
            plan.validate_limits()?;
            queued.push(LinkFanoutRoomState {
                plan,
                claimed_key_package: Some(claimed_key_package.clone()),
                status: LinkFanoutRoomStatus::Pending,
            });
        }
        if let Some(extra) = claims.next() {
            return Err(ClientError::UnexpectedLinkFanoutClaim(
                extra.key_package_id.clone(),
            ));
        }

        let fanout = self.link_fanout_mut(fanout_id)?;
        let mut seen_rooms = fanout
            .rooms
            .iter()
            .map(|room| room.plan.room_id.clone())
            .collect::<BTreeSet<_>>();
        for room in queued {
            if !seen_rooms.insert(room.plan.room_id.clone()) {
                return Err(ClientError::DuplicateLinkFanoutRoom(
                    room.plan.room_id.clone(),
                ));
            }
            fanout.rooms.push(room);
        }
        fanout.after_room_id = page.next_after_room_id.clone();
        fanout.discovery_complete = !page.has_more;
        fanout.validate_limits()?;
        Ok(())
    }

    pub fn prepare_link_fanout_room_commit(
        &mut self,
        fanout_id: &str,
        room_id: &str,
        claimed_key_package: &ClaimKeyPackageResult,
    ) -> Result<PreparedCommit, ClientError> {
        validate_room_id(room_id)?;
        let (target_device, plan) = {
            let fanout = self.link_fanout(fanout_id)?;
            let room = fanout
                .rooms
                .iter()
                .find(|room| room.plan.room_id == room_id)
                .ok_or_else(|| ClientError::LinkFanoutRoomNotFound(room_id.to_string()))?;
            match &room.status {
                LinkFanoutRoomStatus::Pending => {}
                LinkFanoutRoomStatus::Prepared { .. } if !self.has_pending_commit(room_id)? => {}
                _ => return Err(ClientError::LinkFanoutRoomNotPending(room_id.to_string())),
            }
            (fanout.target_device.clone(), room.plan.clone())
        };
        if claimed_key_package.owner != target_device {
            return Err(ClientError::LinkFanoutClaimTargetMismatch {
                expected: target_device,
                actual: claimed_key_package.owner.clone(),
            });
        }
        if claimed_key_package.key_package_id != plan.key_package_id {
            return Err(ClientError::LinkFanoutClaimPackageMismatch {
                expected: plan.key_package_id,
                actual: claimed_key_package.key_package_id.clone(),
            });
        }
        let prepared =
            self.prepare_link_fanout_room_commit_inner(fanout_id, room_id, claimed_key_package)?;
        Ok(prepared)
    }

    pub fn prepare_claimed_link_fanout_room_commit(
        &mut self,
        fanout_id: &str,
        room_id: &str,
    ) -> Result<PreparedCommit, ClientError> {
        let claimed_key_package = self
            .link_fanout_room(fanout_id, room_id)?
            .claimed_key_package
            .clone()
            .ok_or_else(|| ClientError::MissingLinkFanoutClaim(room_id.to_string()))?;
        self.prepare_link_fanout_room_commit_inner(fanout_id, room_id, &claimed_key_package)
    }

    fn prepare_link_fanout_room_commit_inner(
        &mut self,
        fanout_id: &str,
        room_id: &str,
        claimed_key_package: &ClaimKeyPackageResult,
    ) -> Result<PreparedCommit, ClientError> {
        validate_room_id(room_id)?;
        let (target_device, plan) = {
            let fanout = self.link_fanout(fanout_id)?;
            let room = fanout
                .rooms
                .iter()
                .find(|room| room.plan.room_id == room_id)
                .ok_or_else(|| ClientError::LinkFanoutRoomNotFound(room_id.to_string()))?;
            match &room.status {
                LinkFanoutRoomStatus::Pending => {}
                LinkFanoutRoomStatus::Prepared { .. } if !self.has_pending_commit(room_id)? => {}
                _ => return Err(ClientError::LinkFanoutRoomNotPending(room_id.to_string())),
            }
            (fanout.target_device.clone(), room.plan.clone())
        };
        if claimed_key_package.owner != target_device {
            return Err(ClientError::LinkFanoutClaimTargetMismatch {
                expected: target_device,
                actual: claimed_key_package.owner.clone(),
            });
        }
        if claimed_key_package.key_package_id != plan.key_package_id {
            return Err(ClientError::LinkFanoutClaimPackageMismatch {
                expected: plan.key_package_id,
                actual: claimed_key_package.key_package_id.clone(),
            });
        }
        let prepared = self.prepare_add_member_commit(
            &plan.room_id,
            claimed_key_package,
            plan.welcome_id.clone(),
            plan.idempotency_key.clone(),
        )?;
        prepared.validate_limits()?;
        let fanout = self.link_fanout_mut(fanout_id)?;
        let room = fanout
            .rooms
            .iter_mut()
            .find(|room| room.plan.room_id == room_id)
            .ok_or_else(|| ClientError::LinkFanoutRoomNotFound(room_id.to_string()))?;
        room.status = LinkFanoutRoomStatus::Prepared {
            prepared: Box::new(prepared.clone()),
        };
        fanout.validate_limits()?;
        Ok(prepared)
    }

    pub fn prepared_link_fanout_commit(
        &self,
        fanout_id: &str,
        room_id: &str,
    ) -> Result<PreparedCommit, ClientError> {
        let room = self.link_fanout_room(fanout_id, room_id)?;
        match &room.status {
            LinkFanoutRoomStatus::Prepared { prepared } => Ok((**prepared).clone()),
            _ => Err(ClientError::LinkFanoutRoomNotPrepared(room_id.to_string())),
        }
    }

    pub fn complete_link_fanout_room_from_log(
        &mut self,
        fanout_id: &str,
        room_id: &str,
        entry: &RoomLogEntry,
    ) -> Result<AppliedLogEntry, ClientError> {
        let prepared = self.prepared_link_fanout_commit(fanout_id, room_id)?;
        if entry.message_id != prepared.message_id {
            return Err(ClientError::LinkFanoutPreparedCommitMismatch {
                expected: prepared.message_id,
                actual: entry.message_id.clone(),
            });
        }
        let applied = self.apply_log_entry(room_id, entry)?;
        let fanout = self.link_fanout_mut(fanout_id)?;
        let room = fanout
            .rooms
            .iter_mut()
            .find(|room| room.plan.room_id == room_id)
            .ok_or_else(|| ClientError::LinkFanoutRoomNotFound(room_id.to_string()))?;
        room.claimed_key_package = None;
        room.status = LinkFanoutRoomStatus::Done {
            accepted_seq: entry.seq,
        };
        fanout.validate_limits()?;
        Ok(applied)
    }

    pub fn link_fanout_room_status(
        &self,
        fanout_id: &str,
        room_id: &str,
    ) -> Result<LinkFanoutRoomStatus, ClientError> {
        Ok(self.link_fanout_room(fanout_id, room_id)?.status.clone())
    }

    pub fn link_fanout_room_count(&self, fanout_id: &str) -> Result<usize, ClientError> {
        Ok(self.link_fanout(fanout_id)?.rooms.len())
    }

    pub fn link_fanout_bootstrap_export(
        &self,
        fanout_id: &str,
    ) -> Result<LinkBootstrapExportState, ClientError> {
        Ok(self.link_fanout(fanout_id)?.bootstrap_export.clone())
    }

    pub fn set_link_fanout_bootstrap_export(
        &mut self,
        fanout_id: &str,
        bootstrap_export: LinkBootstrapExportState,
    ) -> Result<(), ClientError> {
        validate_link_bootstrap_export(&bootstrap_export)?;
        let fanout = self.link_fanout_mut(fanout_id)?;
        fanout.bootstrap_export = bootstrap_export;
        fanout.validate_limits()
    }

    fn link_fanout_target_device(&self, fanout_id: &str) -> Result<DeviceRef, ClientError> {
        Ok(self.link_fanout(fanout_id)?.target_device.clone())
    }

    fn link_fanout_after_room_id(&self, fanout_id: &str) -> Result<Option<RoomId>, ClientError> {
        Ok(self.link_fanout(fanout_id)?.after_room_id.clone())
    }

    fn link_fanout_discovery_complete(&self, fanout_id: &str) -> Result<bool, ClientError> {
        Ok(self.link_fanout(fanout_id)?.discovery_complete)
    }

    fn pending_link_fanout_room_ids(&self, fanout_id: &str) -> Result<Vec<RoomId>, ClientError> {
        Ok(self
            .link_fanout(fanout_id)?
            .rooms
            .iter()
            .filter(|room| matches!(room.status, LinkFanoutRoomStatus::Pending))
            .map(|room| room.plan.room_id.clone())
            .collect())
    }

    fn prepared_link_fanout_room_ids(&self, fanout_id: &str) -> Result<Vec<RoomId>, ClientError> {
        Ok(self
            .link_fanout(fanout_id)?
            .rooms
            .iter()
            .filter(|room| matches!(room.status, LinkFanoutRoomStatus::Prepared { .. }))
            .map(|room| room.plan.room_id.clone())
            .collect())
    }

    pub fn link_fanout_is_complete(&self, fanout_id: &str) -> Result<bool, ClientError> {
        let fanout = self.link_fanout(fanout_id)?;
        Ok(fanout.discovery_complete
            && fanout
                .rooms
                .iter()
                .all(|room| matches!(room.status, LinkFanoutRoomStatus::Done { .. })))
    }

    fn link_fanout(&self, fanout_id: &str) -> Result<&LinkFanoutState, ClientError> {
        validate_string_bytes("link_fanout.fanout_id", fanout_id, MAX_OBJECT_ID_BYTES)?;
        self.link_fanouts
            .get(fanout_id)
            .ok_or_else(|| ClientError::LinkFanoutNotFound(fanout_id.to_string()))
    }

    fn link_fanout_mut(&mut self, fanout_id: &str) -> Result<&mut LinkFanoutState, ClientError> {
        validate_string_bytes("link_fanout.fanout_id", fanout_id, MAX_OBJECT_ID_BYTES)?;
        self.link_fanouts
            .get_mut(fanout_id)
            .ok_or_else(|| ClientError::LinkFanoutNotFound(fanout_id.to_string()))
    }

    fn link_fanout_room(
        &self,
        fanout_id: &str,
        room_id: &str,
    ) -> Result<&LinkFanoutRoomState, ClientError> {
        validate_room_id(room_id)?;
        self.link_fanout(fanout_id)?
            .rooms
            .iter()
            .find(|room| room.plan.room_id == room_id)
            .ok_or_else(|| ClientError::LinkFanoutRoomNotFound(room_id.to_string()))
    }

    pub fn create_application_request(
        &mut self,
        room_id: &str,
        plaintext: &[u8],
        idempotency_key: impl Into<String>,
    ) -> Result<AppendEventRequest, ClientError> {
        self.create_application_request_at(
            room_id,
            plaintext,
            idempotency_key,
            self.now_unix_seconds,
        )
    }

    pub fn create_application_request_at(
        &mut self,
        room_id: &str,
        plaintext: &[u8],
        idempotency_key: impl Into<String>,
        timestamp_unix_seconds: u64,
    ) -> Result<AppendEventRequest, ClientError> {
        let own_device_ref = self.device_ref.clone();
        let provider = &self.provider;
        let signer = &self.signer;
        let group = self
            .groups
            .get_mut(room_id)
            .ok_or_else(|| ClientError::GroupNotFound(room_id.to_string()))?;
        if group.pending_commit().is_some() {
            return Err(ClientError::PendingCommitMustBeMerged(room_id.to_string()));
        }
        let app_message = group
            .create_message(provider, signer, plaintext)
            .map_err(|_| ClientError::CreateApplicationMessage)?;
        let payload = mls_message_out_bytes(app_message)?;
        let request = AppendEventRequest {
            room_id: room_id.to_string(),
            sender: own_device_ref.clone(),
            envelope: envelope(
                room_id.to_string(),
                mls_group_id_string(group.group_id())?,
                own_device_ref,
                group.epoch().as_u64(),
                LogEntryKind::Application,
                payload,
            ),
            idempotency_key: idempotency_key.into(),
            timestamp_unix_seconds,
        };
        request.validate_limits()?;
        Ok(request)
    }

    pub fn decrypt_application_entry(
        &mut self,
        room_id: &str,
        entry: &RoomLogEntry,
    ) -> Result<DecryptedApplicationEntry, ClientError> {
        validate_log_entry_shape(room_id, entry, LogEntryKind::Application)?;
        let provider = &self.provider;
        let group = self
            .groups
            .get_mut(room_id)
            .ok_or_else(|| ClientError::GroupNotFound(room_id.to_string()))?;
        let processed = group
            .process_message(
                provider,
                protocol_message_from_bytes(&entry.envelope.payload)?,
            )
            .map_err(|error| ClientError::ProcessMessage {
                reason: format!("{error:?}"),
            })?;
        let credential = FiniteDeviceCredentialV1::from_credential(processed.credential().clone())?;
        let sender = DeviceRef {
            account_id: hex_lower(credential.account_public_key().as_bytes()),
            device_id: credential.device_id().to_owned(),
        };
        let ProcessedMessageContent::ApplicationMessage(message) = processed.into_content() else {
            return Err(ClientError::UnexpectedMessage);
        };
        Ok(DecryptedApplicationEntry {
            plaintext: message.into_bytes(),
            sender,
        })
    }

    pub fn verified_member_count(
        &self,
        room_id: &str,
        device: &DeviceRef,
    ) -> Result<u32, ClientError> {
        let group = self.group(room_id)?;
        let expected_account_public_key = account_public_key_from_device_ref(device)?;
        let mut count = 0u32;
        for member in group.members() {
            let credential = FiniteDeviceCredentialV1::from_credential(member.credential)?;
            // Device ids are only unique per account (DeviceRef is the
            // tuple); two accounts may share a device-id string.
            if credential.account_public_key() == expected_account_public_key
                && credential.device_id() == device.device_id
            {
                credential.verify_expected(ExpectedDeviceCredential {
                    account_public_key: expected_account_public_key,
                    device_id: &device.device_id,
                    mls_leaf_signing_public_key: &member.signature_key,
                    now_unix_seconds: self.now_unix_seconds,
                })?;
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn room_members(&self, room_id: &str) -> Result<Vec<DeviceRef>, ClientError> {
        let group = self.group(room_id)?;
        let mut members = Vec::new();
        for member in group.members() {
            let credential = FiniteDeviceCredentialV1::from_credential(member.credential)?;
            credential.verify_expected(ExpectedDeviceCredential {
                account_public_key: credential.account_public_key(),
                device_id: credential.device_id(),
                mls_leaf_signing_public_key: &member.signature_key,
                now_unix_seconds: self.now_unix_seconds,
            })?;
            members.push(DeviceRef {
                account_id: hex_lower(credential.account_public_key().as_bytes()),
                device_id: credential.device_id().to_owned(),
            });
        }
        members.sort_by(|left, right| {
            left.account_id
                .cmp(&right.account_id)
                .then_with(|| left.device_id.cmp(&right.device_id))
        });
        members.dedup();
        Ok(members)
    }

    fn random_bytes<const N: usize>(&self) -> Result<[u8; N], ClientError> {
        self.provider
            .rand()
            .random_array()
            .map_err(|_| ClientError::Randomness)
    }

    pub fn generate_object_id(&self, prefix: &str) -> Result<String, ClientError> {
        let bytes = self.random_bytes::<8>()?;
        Ok(format!("{prefix}-{}", hex_lower(&bytes)))
    }

    /// Verify that some member of the room carries a valid credential for
    /// the given account after activating a Welcome: a hostile rendezvous
    /// server that admits the device to a different group fails here.
    pub fn verify_room_member_account(
        &self,
        room_id: &str,
        account_id: &str,
    ) -> Result<(), ClientError> {
        let group = self.group(room_id)?;
        let expected = NostrPublicKey::from_bytes(decode_lower_hex_32(account_id)?)
            .map_err(ClientError::from)?;
        for member in group.members() {
            let Ok(credential) = FiniteDeviceCredentialV1::from_credential(member.credential)
            else {
                continue;
            };
            if credential.account_public_key() != expected {
                continue;
            }
            credential.verify_expected(ExpectedDeviceCredential {
                account_public_key: expected,
                device_id: credential.device_id(),
                mls_leaf_signing_public_key: &member.signature_key,
                now_unix_seconds: self.now_unix_seconds,
            })?;
            return Ok(());
        }
        Err(ClientError::AccountNotInRoom {
            room_id: room_id.to_owned(),
            account_id: account_id.to_owned(),
        })
    }

    /// Encrypt an ephemeral activity payload (typing/working indicators)
    /// under a key derived from the room's MLS exporter secret at the
    /// current epoch. Activities are disposable: a payload from another
    /// epoch simply fails to decrypt and is skipped (forward secrecy means
    /// old epoch secrets are gone by design).
    pub fn encrypt_activity_payload(
        &self,
        room_id: &str,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, ClientError> {
        let group = self.group(room_id)?;
        let epoch = group.epoch().as_u64();
        let key = group
            .export_secret(self.provider.crypto(), ACTIVITY_EXPORTER_LABEL, &[], 32)
            .map_err(|_| ClientError::ActivityCiphertext)?;
        let nonce: [u8; ACTIVITY_NONCE_BYTES] = self.random_bytes()?;
        let ciphertext = self
            .provider
            .crypto()
            .aead_encrypt(
                AeadType::Aes256Gcm,
                &key,
                plaintext,
                &nonce,
                room_id.as_bytes(),
            )
            .map_err(|_| ClientError::ActivityCiphertext)?;
        let mut out = Vec::with_capacity(8 + ACTIVITY_NONCE_BYTES + ciphertext.len());
        out.extend_from_slice(&epoch.to_be_bytes());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    pub fn decrypt_activity_payload(
        &self,
        room_id: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, ClientError> {
        let group = self.group(room_id)?;
        if payload.len() < 8 + ACTIVITY_NONCE_BYTES {
            return Err(ClientError::ActivityCiphertext);
        }
        let epoch = u64::from_be_bytes(payload[..8].try_into().expect("8 bytes"));
        if epoch != group.epoch().as_u64() {
            return Err(ClientError::ActivityEpochMismatch {
                payload_epoch: epoch,
                group_epoch: group.epoch().as_u64(),
            });
        }
        let key = group
            .export_secret(self.provider.crypto(), ACTIVITY_EXPORTER_LABEL, &[], 32)
            .map_err(|_| ClientError::ActivityCiphertext)?;
        let nonce = &payload[8..8 + ACTIVITY_NONCE_BYTES];
        self.provider
            .crypto()
            .aead_decrypt(
                AeadType::Aes256Gcm,
                &key,
                &payload[8 + ACTIVITY_NONCE_BYTES..],
                nonce,
                room_id.as_bytes(),
            )
            .map_err(|_| ClientError::ActivityCiphertext)
    }

    pub fn room_mls_group_id(&self, room_id: &str) -> Result<String, ClientError> {
        mls_group_id_string(self.group(room_id)?.group_id())
    }

    pub fn group_epoch(&self, room_id: &str) -> Result<u64, ClientError> {
        Ok(self.group(room_id)?.epoch().as_u64())
    }

    pub fn has_pending_commit(&self, room_id: &str) -> Result<bool, ClientError> {
        Ok(self.group(room_id)?.pending_commit().is_some())
    }

    fn verify_member_in_group(
        &self,
        group: &MlsGroup,
        device: &DeviceRef,
    ) -> Result<(), ClientError> {
        let expected_account_public_key = account_public_key_from_device_ref(device)?;
        let mut count = 0u32;
        for member in group.members() {
            let credential = FiniteDeviceCredentialV1::from_credential(member.credential)?;
            // Device ids are only unique per account (DeviceRef is the
            // tuple); two accounts may share a device-id string.
            if credential.account_public_key() == expected_account_public_key
                && credential.device_id() == device.device_id
            {
                credential.verify_expected(ExpectedDeviceCredential {
                    account_public_key: expected_account_public_key,
                    device_id: &device.device_id,
                    mls_leaf_signing_public_key: &member.signature_key,
                    now_unix_seconds: self.now_unix_seconds,
                })?;
                count += 1;
            }
        }
        if count == 1 {
            Ok(())
        } else {
            Err(ClientError::MemberCredentialMissing(device.clone()))
        }
    }

    fn group(&self, room_id: &str) -> Result<&MlsGroup, ClientError> {
        self.groups
            .get(room_id)
            .ok_or_else(|| ClientError::GroupNotFound(room_id.to_string()))
    }

    fn set_last_applied_seq(&mut self, room_id: &str, seq: u64) -> Result<(), ClientError> {
        validate_room_id(room_id)?;
        self.group(room_id)?;
        let current_seq = self.room_cursors.get(room_id).copied().unwrap_or(0);
        if seq < current_seq {
            return Err(ClientError::AppliedSeqRegression {
                room_id: room_id.to_string(),
                current_seq,
                attempted_seq: seq,
            });
        }
        self.room_cursors.insert(room_id.to_string(), seq);
        debug_assert!(self.room_cursors.contains_key(room_id));
        Ok(())
    }
}

impl FiniteChatDeviceState {
    fn validate_limits(&self) -> Result<(), ClientError> {
        self.device_ref.validate_limits()?;
        validate_bytes_non_empty("signer_public_key", self.signer_public_key.len())?;
        validate_bytes_len(
            "signer_public_key",
            self.signer_public_key.len(),
            MAX_CLIENT_SIGNER_PUBLIC_KEY_BYTES,
        )?;
        validate_bytes_non_empty("credential_identity", self.credential_identity.len())?;
        validate_bytes_len(
            "credential_identity",
            self.credential_identity.len(),
            MAX_CLIENT_CREDENTIAL_IDENTITY_BYTES,
        )?;
        validate_item_count("client_state.rooms", self.rooms.len(), MAX_PERSISTED_ROOMS)?;
        validate_item_count(
            "client_state.pending_welcomes",
            self.pending_welcomes.len(),
            MAX_PENDING_CLIENT_WELCOMES,
        )?;
        validate_item_count(
            "client_state.pending_welcome_acks",
            self.pending_welcome_acks.len(),
            MAX_PENDING_CLIENT_WELCOMES,
        )?;
        validate_item_count(
            "client_state.pending_key_package_uploads",
            self.pending_key_package_uploads.len(),
            MAX_PENDING_KEY_PACKAGE_UPLOADS,
        )?;
        validate_item_count(
            "client_state.link_fanouts",
            self.link_fanouts.len(),
            MAX_LINK_FANOUTS,
        )?;
        validate_item_count(
            "client_state.openmls_storage_records",
            self.openmls_storage_records.len(),
            MAX_OPENMLS_STORAGE_RECORDS,
        )?;
        for room in &self.rooms {
            room.validate_limits()?;
        }
        if self.openmls_storage_records.is_empty() {
            return Err(ClientError::MissingOpenMlsStorage);
        }
        let mut seen_storage_keys = BTreeSet::<Vec<u8>>::new();
        let mut seen_rooms = BTreeSet::<RoomId>::new();
        let mut seen_pending_welcomes = BTreeSet::<WelcomeId>::new();
        let mut seen_pending_welcome_acks = BTreeSet::<WelcomeId>::new();
        let mut seen_pending_key_package_uploads = BTreeSet::<KeyPackageId>::new();
        let mut seen_link_fanouts = BTreeSet::<String>::new();
        for room in &self.rooms {
            if !seen_rooms.insert(room.room_id.clone()) {
                return Err(ClientError::DuplicatePersistedRoom(room.room_id.clone()));
            }
        }
        for welcome in &self.pending_welcomes {
            welcome.validate_limits()?;
            if !seen_pending_welcomes.insert(welcome.welcome_id.clone()) {
                return Err(ClientError::DuplicatePendingWelcome(
                    welcome.welcome_id.clone(),
                ));
            }
        }
        for ack in &self.pending_welcome_acks {
            ack.validate_limits()?;
            if !seen_pending_welcome_acks.insert(ack.welcome_id.clone()) {
                return Err(ClientError::DuplicatePendingWelcomeAck(
                    ack.welcome_id.clone(),
                ));
            }
            if seen_pending_welcomes.contains(&ack.welcome_id) {
                return Err(ClientError::PendingWelcomeAlsoNeedsAck(
                    ack.welcome_id.clone(),
                ));
            }
            if !seen_rooms.contains(&ack.room_id) {
                return Err(ClientError::PendingWelcomeAckRoomMissing(
                    ack.room_id.clone(),
                ));
            }
        }
        for request in &self.pending_key_package_uploads {
            request.validate_limits()?;
            if request.owner != self.device_ref {
                return Err(ClientError::PendingKeyPackageUploadOwnerMismatch {
                    expected: self.device_ref.clone(),
                    actual: request.owner.clone(),
                });
            }
            if !seen_pending_key_package_uploads.insert(request.key_package_id.clone()) {
                return Err(ClientError::DuplicatePendingKeyPackageUpload(
                    request.key_package_id.clone(),
                ));
            }
        }
        for fanout in &self.link_fanouts {
            fanout.validate_limits()?;
            if fanout.target_device.account_id != self.device_ref.account_id {
                return Err(ClientError::LinkFanoutAccountMismatch {
                    expected: self.device_ref.account_id.clone(),
                    actual: fanout.target_device.account_id.clone(),
                });
            }
            if fanout.target_device == self.device_ref {
                return Err(ClientError::LinkFanoutCannotTargetSelf);
            }
            if !seen_link_fanouts.insert(fanout.fanout_id.clone()) {
                return Err(ClientError::DuplicateLinkFanout(fanout.fanout_id.clone()));
            }
        }
        for record in &self.openmls_storage_records {
            record.validate_limits()?;
            if !seen_storage_keys.insert(record.key.clone()) {
                return Err(ClientError::DuplicateOpenMlsStorageKey);
            }
        }
        debug_assert!(!self.signer_public_key.is_empty());
        debug_assert!(!self.credential_identity.is_empty());
        Ok(())
    }
}

impl PersistedRoomState {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_room_id(&self.room_id)?;
        validate_mls_group_id(&self.mls_group_id)?;
        if let Some(server_url) = &self.server_url {
            validate_string_bytes("room.server_url", server_url, MAX_ROOM_SERVER_URL_BYTES)?;
        }
        Ok(())
    }
}

impl PendingWelcomeState {
    fn from_welcome_record(welcome: &WelcomeRecord) -> Result<Self, ClientError> {
        let pending = Self {
            welcome_id: welcome.welcome_id.clone(),
            room_id: welcome.room_id.clone(),
            commit_seq: welcome.commit_seq,
            welcome_payload: welcome.welcome_payload.clone(),
            ratchet_tree_payload: welcome.ratchet_tree_payload.clone(),
        };
        pending.validate_limits()?;
        Ok(pending)
    }

    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_string_bytes("welcome_id", &self.welcome_id, MAX_OBJECT_ID_BYTES)?;
        validate_room_id(&self.room_id)?;
        validate_bytes_non_empty(
            "pending_welcome.welcome_payload",
            self.welcome_payload.len(),
        )?;
        validate_bytes_len(
            "pending_welcome.welcome_payload",
            self.welcome_payload.len(),
            MAX_WELCOME_PAYLOAD_BYTES,
        )?;
        validate_bytes_non_empty(
            "pending_welcome.ratchet_tree_payload",
            self.ratchet_tree_payload.len(),
        )?;
        validate_bytes_len(
            "pending_welcome.ratchet_tree_payload",
            self.ratchet_tree_payload.len(),
            MAX_RATCHET_TREE_PAYLOAD_BYTES,
        )?;
        Ok(())
    }
}

impl PendingWelcomeAckState {
    fn from_pending_welcome(pending: &PendingWelcomeState) -> Result<Self, ClientError> {
        let ack = Self {
            welcome_id: pending.welcome_id.clone(),
            room_id: pending.room_id.clone(),
            commit_seq: pending.commit_seq,
        };
        ack.validate_limits()?;
        Ok(ack)
    }

    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_string_bytes(
            "welcome_ack.welcome_id",
            &self.welcome_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_room_id(&self.room_id)?;
        Ok(())
    }
}

impl PreparedCommit {
    fn validate_limits(&self) -> Result<(), ClientError> {
        self.request.validate_limits().map_err(ClientError::from)?;
        validate_string_bytes(
            "prepared_commit.message_id",
            &self.message_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("prepared_commit.message_id", self.message_id.len())?;
        if self.request.membership_delta.commit_message_id != self.message_id {
            return Err(ClientError::PreparedCommitMessageIdMismatch);
        }
        if self
            .request
            .envelope
            .message_id()
            .map_err(ClientError::EnvelopeMessageId)?
            != self.message_id
        {
            return Err(ClientError::PreparedCommitMessageIdMismatch);
        }
        Ok(())
    }
}

impl LinkFanoutState {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_string_bytes(
            "link_fanout.fanout_id",
            &self.fanout_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("link_fanout.fanout_id", self.fanout_id.len())?;
        self.target_device.validate_limits()?;
        if let Some(after_room_id) = &self.after_room_id {
            validate_room_id(after_room_id)?;
        }
        validate_item_count("link_fanout.rooms", self.rooms.len(), MAX_LINK_FANOUT_ROOMS)?;
        let mut seen_rooms = BTreeSet::<RoomId>::new();
        for room in &self.rooms {
            room.validate_limits()?;
            if let Some(claimed_key_package) = &room.claimed_key_package {
                if matches!(room.status, LinkFanoutRoomStatus::Done { .. }) {
                    return Err(ClientError::UnexpectedLinkFanoutClaim(
                        claimed_key_package.key_package_id.clone(),
                    ));
                }
                if claimed_key_package.owner != self.target_device {
                    return Err(ClientError::LinkFanoutClaimTargetMismatch {
                        expected: self.target_device.clone(),
                        actual: claimed_key_package.owner.clone(),
                    });
                }
                if claimed_key_package.key_package_id != room.plan.key_package_id {
                    return Err(ClientError::LinkFanoutClaimPackageMismatch {
                        expected: room.plan.key_package_id.clone(),
                        actual: claimed_key_package.key_package_id.clone(),
                    });
                }
            }
            if !seen_rooms.insert(room.plan.room_id.clone()) {
                return Err(ClientError::DuplicateLinkFanoutRoom(
                    room.plan.room_id.clone(),
                ));
            }
        }
        validate_link_bootstrap_export(&self.bootstrap_export)?;
        Ok(())
    }
}

fn validate_link_bootstrap_export(export: &LinkBootstrapExportState) -> Result<(), ClientError> {
    let encoded = serde_json::to_vec(export)
        .map_err(|_| ClientError::InvalidClientState("device-link bootstrap export".to_owned()))?;
    validate_bytes_len(
        "link_fanout.bootstrap_export",
        encoded.len(),
        MAX_CLIENT_STATE_PLAINTEXT_BYTES,
    )?;
    match export {
        LinkBootstrapExportState::WaitingForFanout => {}
        LinkBootstrapExportState::Exporting { rooms, .. } => {
            validate_item_count(
                "link_fanout.bootstrap_rooms",
                rooms.len(),
                MAX_LINK_FANOUT_ROOMS,
            )?;
        }
        LinkBootstrapExportState::Emitted { manifests } => {
            validate_item_count(
                "link_fanout.bootstrap_manifests",
                manifests.len(),
                MAX_LINK_FANOUT_ROOMS,
            )?;
        }
    }
    Ok(())
}

impl LinkFanoutRoomState {
    fn validate_limits(&self) -> Result<(), ClientError> {
        self.plan.validate_limits()?;
        if let Some(claimed_key_package) = &self.claimed_key_package {
            validate_claimed_key_package(claimed_key_package)?;
        }
        self.status.validate_limits()?;
        Ok(())
    }
}

impl LinkFanoutRoomPlan {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_room_id(&self.room_id)?;
        validate_bytes_non_empty("link_fanout.key_package_id", self.key_package_id.len())?;
        validate_string_bytes(
            "link_fanout.key_package_id",
            &self.key_package_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("link_fanout.welcome_id", self.welcome_id.len())?;
        validate_string_bytes(
            "link_fanout.welcome_id",
            &self.welcome_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("link_fanout.idempotency_key", self.idempotency_key.len())?;
        validate_idempotency_key(&self.idempotency_key)?;
        Ok(())
    }
}

impl LinkFanoutRoomStatus {
    fn validate_limits(&self) -> Result<(), ClientError> {
        match self {
            Self::Pending => Ok(()),
            Self::Prepared { prepared } => prepared.validate_limits(),
            Self::Done { .. } => Ok(()),
        }
    }
}

impl OpenMlsStorageRecord {
    fn validate_limits(&self) -> Result<(), ClientError> {
        validate_bytes_non_empty("openmls_storage.key", self.key.len())?;
        validate_bytes_len(
            "openmls_storage.key",
            self.key.len(),
            MAX_OPENMLS_STORAGE_KEY_BYTES,
        )?;
        validate_bytes_non_empty("openmls_storage.value", self.value.len())?;
        validate_bytes_len(
            "openmls_storage.value",
            self.value.len(),
            MAX_OPENMLS_STORAGE_VALUE_BYTES,
        )?;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ClientStoreEncryptionKey {
    bytes: [u8; CLIENT_STORE_KEY_BYTES],
}

impl ClientStoreEncryptionKey {
    pub fn from_nostr_secret(
        account_secret_key: &NostrSecretKey,
        device_id: &str,
    ) -> Result<Self, ClientStoreError> {
        validate_bytes_non_empty("client_store.device_id", device_id.len())
            .map_err(ClientError::from)?;
        validate_string_bytes("client_store.device_id", device_id, MAX_DEVICE_ID_BYTES)
            .map_err(ClientError::from)?;
        let bytes = account_secret_key
            .derive_secret_32(CLIENT_STORE_KEY_DERIVATION_DOMAIN, device_id.as_bytes())
            .map_err(ClientError::from)?;
        debug_assert_eq!(bytes.len(), CLIENT_STORE_KEY_BYTES);
        Ok(Self { bytes })
    }

    fn as_bytes(&self) -> &[u8; CLIENT_STORE_KEY_BYTES] {
        &self.bytes
    }
}

impl std::fmt::Debug for ClientStoreEncryptionKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ClientStoreEncryptionKey(REDACTED)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteClientStoreOptions {
    pub encryption_key: ClientStoreEncryptionKey,
}

impl SqliteClientStoreOptions {
    pub fn from_nostr_secret(
        account_secret_key: &NostrSecretKey,
        device_id: &str,
    ) -> Result<Self, ClientStoreError> {
        Ok(Self {
            encryption_key: ClientStoreEncryptionKey::from_nostr_secret(
                account_secret_key,
                device_id,
            )?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyClientStoreTable {
    Profiles,
    Rooms,
    OpenMlsStorage,
}

impl LegacyClientStoreTable {
    fn name(self) -> &'static str {
        match self {
            Self::Profiles => "client_profiles",
            Self::Rooms => "client_rooms",
            Self::OpenMlsStorage => "client_openmls_storage",
        }
    }
}

#[derive(Debug)]
pub struct SqliteClientStore {
    db_path: PathBuf,
    conn: Connection,
    options: SqliteClientStoreOptions,
}

impl SqliteClientStore {
    pub fn open(
        db_path: impl AsRef<Path>,
        options: SqliteClientStoreOptions,
    ) -> Result<Self, ClientStoreError> {
        let db_path = db_path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|source| ClientStoreError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let conn = open_client_store_connection(&db_path)?;
        prepare_client_store_schema(&conn)?;
        Ok(Self {
            db_path,
            conn,
            options,
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn save_device_state(&mut self, device: &FiniteChatDevice) -> Result<(), ClientStoreError> {
        let state = device.export_state()?;
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| save_device_state_tx(tx, &state, &encryption_key))
    }

    pub fn save_device_state_and_app_messages(
        &mut self,
        device: &FiniteChatDevice,
        messages: &[StoredAppMessage],
    ) -> Result<(), ClientStoreError> {
        let state = device.export_state()?;
        let owner = state.device_ref.clone();
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| {
            save_device_state_tx(tx, &state, &encryption_key)?;
            save_app_messages_tx(tx, &encryption_key, &owner, messages)?;
            prune_app_messages_tx(tx, &owner, MAX_STORED_APP_MESSAGES)?;
            Ok(())
        })
    }

    pub fn save_device_state_and_app_messages_and_events(
        &mut self,
        device: &FiniteChatDevice,
        messages: &[StoredAppMessage],
        events: &[StoredAppEvent],
    ) -> Result<(), ClientStoreError> {
        let state = device.export_state()?;
        let owner = state.device_ref.clone();
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| {
            save_device_state_tx(tx, &state, &encryption_key)?;
            save_app_messages_tx(tx, &encryption_key, &owner, messages)?;
            save_app_events_tx(tx, &encryption_key, &owner, events)?;
            for event in events {
                if let Some((source, bootstrap)) =
                    device_link_bootstrap_from_stored_event(&owner, event)
                {
                    let _ = stage_device_link_bootstrap_chunk_tx(
                        tx,
                        &encryption_key,
                        &owner,
                        &source,
                        event.seq,
                        &bootstrap,
                    )?;
                }
            }
            prune_app_messages_tx(tx, &owner, MAX_STORED_APP_MESSAGES)?;
            Ok(())
        })
    }

    pub fn load_app_messages(
        &self,
        owner: &DeviceRef,
        limit: u32,
    ) -> Result<Vec<StoredAppMessage>, ClientStoreError> {
        validate_app_message_owner(owner)?;
        validate_app_message_limit(limit)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT room_id, seq, message_id, sender_account_id, sender_device_id, nonce, ciphertext, timestamp_unix_seconds
            FROM (
              SELECT
                rowid AS row_id,
                room_id,
                seq,
                message_id,
                sender_account_id,
                sender_device_id,
                nonce,
                ciphertext,
                timestamp_unix_seconds
              FROM client_app_messages
              WHERE account_id = ?1 AND device_id = ?2
              ORDER BY rowid DESC
              LIMIT ?3
            )
            ORDER BY row_id ASC
            "#,
        )?;
        let rows = stmt.query_map(
            params![&owner.account_id, &owner.device_id, i64::from(limit)],
            |row| {
                Ok((
                    row.get::<_, RoomId>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, MessageId>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )?;
        let mut messages = Vec::new();
        for row in rows {
            let (
                room_id,
                stored_seq,
                message_id,
                sender_account_id,
                sender_device_id,
                nonce,
                ciphertext,
                stored_timestamp_unix_seconds,
            ) = row?;
            let seq = sqlite_seq_to_u64(stored_seq)?;
            let timestamp_unix_seconds = sqlite_timestamp_to_u64(stored_timestamp_unix_seconds)?;
            let sender = DeviceRef {
                account_id: sender_account_id,
                device_id: sender_device_id,
            };
            let plaintext = decrypt_app_message_plaintext(
                &self.options.encryption_key,
                AppMessageIdentity {
                    owner,
                    room_id: &room_id,
                    seq,
                    message_id: &message_id,
                    sender: &sender,
                },
                &nonce,
                &ciphertext,
            )?;
            let message = StoredAppMessage {
                room_id,
                seq,
                message_id,
                sender,
                plaintext,
                timestamp_unix_seconds,
            };
            message.validate_limits()?;
            messages.push(message);
        }
        Ok(messages)
    }

    pub fn save_app_messages(
        &mut self,
        owner: &DeviceRef,
        messages: &[StoredAppMessage],
        max_messages: u32,
    ) -> Result<(), ClientStoreError> {
        validate_app_message_limit(max_messages)?;
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| {
            save_app_messages_tx(tx, &encryption_key, owner, messages)?;
            prune_app_messages_tx(tx, owner, max_messages)
        })
    }

    pub fn save_app_messages_and_events(
        &mut self,
        owner: &DeviceRef,
        messages: &[StoredAppMessage],
        events: &[StoredAppEvent],
        max_items: u32,
    ) -> Result<(), ClientStoreError> {
        validate_app_message_limit(max_items)?;
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| {
            save_app_messages_tx(tx, &encryption_key, owner, messages)?;
            save_app_events_tx(tx, &encryption_key, owner, events)?;
            prune_app_messages_tx(tx, owner, max_items)?;
            Ok(())
        })
    }

    pub fn load_app_events(
        &self,
        owner: &DeviceRef,
        limit: u32,
    ) -> Result<Vec<StoredAppEvent>, ClientStoreError> {
        validate_app_message_owner(owner)?;
        validate_app_event_limit(limit)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT room_id, seq, message_id, sender_account_id, sender_device_id, nonce, ciphertext, timestamp_unix_seconds
            FROM (
              SELECT
                rowid AS row_id,
                room_id,
                seq,
                message_id,
                sender_account_id,
                sender_device_id,
                nonce,
                ciphertext,
                timestamp_unix_seconds
              FROM client_app_events
              WHERE account_id = ?1 AND device_id = ?2
              ORDER BY rowid DESC
              LIMIT ?3
            )
            ORDER BY row_id ASC
            "#,
        )?;
        let rows = stmt.query_map(
            params![&owner.account_id, &owner.device_id, i64::from(limit)],
            |row| {
                Ok((
                    row.get::<_, RoomId>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, MessageId>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )?;
        let mut events = Vec::new();
        for row in rows {
            let (
                room_id,
                stored_seq,
                message_id,
                sender_account_id,
                sender_device_id,
                nonce,
                ciphertext,
                stored_timestamp_unix_seconds,
            ) = row?;
            let seq = sqlite_app_event_seq_to_u64(stored_seq)?;
            let timestamp_unix_seconds = sqlite_timestamp_to_u64(stored_timestamp_unix_seconds)?;
            let sender = DeviceRef {
                account_id: sender_account_id,
                device_id: sender_device_id,
            };
            let plaintext = decrypt_app_event_plaintext(
                &self.options.encryption_key,
                AppMessageIdentity {
                    owner,
                    room_id: &room_id,
                    seq,
                    message_id: &message_id,
                    sender: &sender,
                },
                &nonce,
                &ciphertext,
            )?;
            let event = StoredAppEvent {
                room_id,
                seq,
                message_id,
                sender,
                plaintext,
                timestamp_unix_seconds,
            };
            event.validate_limits()?;
            events.push(event);
        }
        Ok(events)
    }

    pub fn save_app_events(
        &mut self,
        owner: &DeviceRef,
        events: &[StoredAppEvent],
        max_events: u32,
    ) -> Result<(), ClientStoreError> {
        validate_app_event_limit(max_events)?;
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| {
            save_app_events_tx(tx, &encryption_key, owner, events)?;
            prune_app_events_tx(tx, owner, max_events)
        })
    }

    /// Persist canonical decrypted application events without pruning history.
    ///
    /// The in-memory transcript remains independently bounded. Durable event
    /// retention is intentionally complete because an existing account Device
    /// is the only party able to rewrap pre-join MLS history for a newly linked
    /// Device.
    pub fn import_app_events_atomically(
        &mut self,
        owner: &DeviceRef,
        events: &[StoredAppEvent],
    ) -> Result<(), ClientStoreError> {
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| import_app_events_tx(tx, &encryption_key, owner, events))
    }

    /// Durably stage one authenticated V2 bootstrap chunk.
    ///
    /// Runtime sync also invokes the same transaction helper while saving the
    /// advanced device cursor. Calling this from the later projection pass is
    /// therefore an exact, idempotent observation of already-durable state.
    pub fn stage_device_link_bootstrap_chunk(
        &mut self,
        owner: &DeviceRef,
        source: &DeviceRef,
        envelope_seq: u64,
        bootstrap: &DeviceLinkBootstrapV2,
    ) -> Result<DeviceLinkBootstrapStageOutcome, ClientStoreError> {
        validate_device_link_bootstrap_for_owner(owner, source, bootstrap)?;
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| {
            stage_device_link_bootstrap_chunk_tx(
                tx,
                &encryption_key,
                owner,
                source,
                envelope_seq,
                bootstrap,
            )
        })
    }

    /// Load every receiving transfer directly from encrypted staging.
    ///
    /// This is deliberately unrelated to the application-event cursor or its
    /// retention window. A complete result can be finalized with the source
    /// offline after any process restart.
    pub fn load_pending_device_link_bootstraps(
        &self,
        owner: &DeviceRef,
    ) -> Result<Vec<StoredPendingDeviceLinkBootstrap>, ClientStoreError> {
        validate_app_message_owner(owner)?;
        load_pending_device_link_bootstraps(&self.conn, &self.options.encryption_key, owner)
    }

    /// Atomically verify and publish one complete transfer.
    ///
    /// Imported history, authoritative room metadata, cached profiles, and
    /// the completed receipt share one SQLite commit. No caller can observe a
    /// receipt or authoritative room without the complete canonical history.
    pub fn commit_device_link_bootstrap_atomically(
        &mut self,
        owner: &DeviceRef,
        expected: &StoredDeviceLinkBootstrapReceipt,
        room: &StoredAppRoom,
        profiles: &[StoredAppProfile],
    ) -> Result<DeviceLinkBootstrapCommitOutcome, ClientStoreError> {
        validate_app_message_owner(owner)?;
        validate_device_link_bootstrap_receipt(expected)?;
        room.validate_limits()?;
        if room.room_id != expected.room_id {
            return Err(ClientStoreError::DeviceLinkBootstrapRoomMismatch {
                expected: expected.room_id.clone(),
                actual: room.room_id.clone(),
            });
        }
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| {
            commit_device_link_bootstrap_tx(tx, &encryption_key, owner, expected, room, profiles)
        })
    }

    pub fn poison_device_link_bootstrap(
        &mut self,
        owner: &DeviceRef,
        expected: &StoredDeviceLinkBootstrapReceipt,
    ) -> Result<(), ClientStoreError> {
        validate_app_message_owner(owner)?;
        validate_device_link_bootstrap_receipt(expected)?;
        self.with_transaction(|tx| {
            poison_device_link_bootstrap_tx(tx, owner, expected)?;
            Ok(())
        })
    }

    pub fn load_completed_device_link_bootstrap_receipts(
        &self,
        owner: &DeviceRef,
    ) -> Result<Vec<StoredDeviceLinkBootstrapReceipt>, ClientStoreError> {
        validate_app_message_owner(owner)?;
        load_completed_device_link_bootstrap_receipts(
            &self.conn,
            &self.options.encryption_key,
            owner,
        )
    }

    pub fn load_completed_device_link_bootstrap_receipt(
        &self,
        owner: &DeviceRef,
        room_id: &str,
        bootstrap_id: &str,
        source: &DeviceRef,
    ) -> Result<Option<StoredDeviceLinkBootstrapReceipt>, ClientStoreError> {
        validate_app_message_owner(owner)?;
        validate_room_id(room_id).map_err(ClientError::from)?;
        validate_string_bytes(
            "device_link_bootstrap.bootstrap_id",
            bootstrap_id,
            MAX_OBJECT_ID_BYTES,
        )
        .map_err(ClientError::from)?;
        source.validate_limits().map_err(ClientError::from)?;
        load_completed_device_link_bootstrap_receipt(
            &self.conn,
            &self.options.encryption_key,
            owner,
            room_id,
            bootstrap_id,
            source,
        )
    }

    pub fn max_app_event_seq_for_room(
        &self,
        owner: &DeviceRef,
        room_id: &str,
    ) -> Result<Option<u64>, ClientStoreError> {
        validate_app_message_owner(owner)?;
        validate_room_id(room_id).map_err(ClientError::from)?;
        let stored = self.conn.query_row(
            r#"
            SELECT MAX(seq)
            FROM client_app_events
            WHERE account_id = ?1 AND device_id = ?2 AND room_id = ?3
            "#,
            params![&owner.account_id, &owner.device_id, room_id],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        stored.map(sqlite_app_event_seq_to_u64).transpose()
    }

    pub fn has_app_event(
        &self,
        owner: &DeviceRef,
        room_id: &str,
        message_id: &str,
    ) -> Result<bool, ClientStoreError> {
        validate_app_message_owner(owner)?;
        validate_room_id(room_id).map_err(ClientError::from)?;
        validate_bytes_non_empty("app_event.message_id", message_id.len())
            .map_err(ClientError::from)?;
        validate_string_bytes("app_event.message_id", message_id, MAX_OBJECT_ID_BYTES)
            .map_err(ClientError::from)?;
        self.conn
            .query_row(
                r#"
                SELECT EXISTS(
                  SELECT 1
                  FROM client_app_events
                  WHERE account_id = ?1
                    AND device_id = ?2
                    AND room_id = ?3
                    AND message_id = ?4
                )
                "#,
                params![&owner.account_id, &owner.device_id, room_id, message_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(ClientStoreError::from)
    }

    pub fn has_app_event_identity(
        &self,
        owner: &DeviceRef,
        room_id: &str,
        seq: u64,
        message_id: &str,
        sender: &DeviceRef,
        timestamp_unix_seconds: u64,
    ) -> Result<bool, ClientStoreError> {
        validate_app_message_owner(owner)?;
        validate_room_id(room_id).map_err(ClientError::from)?;
        validate_bytes_non_empty("app_event.message_id", message_id.len())
            .map_err(ClientError::from)?;
        validate_string_bytes("app_event.message_id", message_id, MAX_OBJECT_ID_BYTES)
            .map_err(ClientError::from)?;
        sender.validate_limits().map_err(ClientError::from)?;
        self.conn
            .query_row(
                r#"
                SELECT EXISTS(
                  SELECT 1
                  FROM client_app_events
                  WHERE account_id = ?1
                    AND device_id = ?2
                    AND room_id = ?3
                    AND seq = ?4
                    AND message_id = ?5
                    AND sender_account_id = ?6
                    AND sender_device_id = ?7
                    AND timestamp_unix_seconds = ?8
                )
                "#,
                params![
                    &owner.account_id,
                    &owner.device_id,
                    room_id,
                    sqlite_app_event_seq_from_u64(seq)?,
                    message_id,
                    &sender.account_id,
                    &sender.device_id,
                    sqlite_timestamp_from_u64(timestamp_unix_seconds)?,
                ],
                |row| row.get::<_, bool>(0),
            )
            .map_err(ClientStoreError::from)
    }

    /// Load one deterministic page from the durable room history snapshot.
    ///
    /// `through_seq` freezes the upper boundary before bootstrap control events
    /// are appended. `(after_seq, after_message_id)` is an exclusive cursor so
    /// equal-sequence imported events cannot be skipped.
    pub fn load_app_events_for_room_page(
        &self,
        owner: &DeviceRef,
        room_id: &str,
        through_seq: u64,
        after_seq: u64,
        after_message_id: &str,
        limit: u32,
    ) -> Result<Vec<StoredAppEvent>, ClientStoreError> {
        validate_app_message_owner(owner)?;
        validate_room_id(room_id).map_err(ClientError::from)?;
        validate_string_bytes(
            "app_event_page.after_message_id",
            after_message_id,
            MAX_OBJECT_ID_BYTES,
        )
        .map_err(ClientError::from)?;
        validate_app_event_limit(limit)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
              room_id,
              seq,
              message_id,
              sender_account_id,
              sender_device_id,
              nonce,
              ciphertext,
              timestamp_unix_seconds
            FROM client_app_events
            WHERE account_id = ?1
              AND device_id = ?2
              AND room_id = ?3
              AND seq <= ?4
              AND (seq > ?5 OR (seq = ?5 AND message_id > ?6))
            ORDER BY seq ASC, message_id ASC
            LIMIT ?7
            "#,
        )?;
        let rows = stmt.query_map(
            params![
                &owner.account_id,
                &owner.device_id,
                room_id,
                sqlite_app_event_seq_from_u64(through_seq)?,
                sqlite_app_event_seq_from_u64(after_seq)?,
                after_message_id,
                i64::from(limit),
            ],
            |row| {
                Ok((
                    row.get::<_, RoomId>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, MessageId>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )?;
        let mut events = Vec::new();
        for row in rows {
            let (
                stored_room_id,
                stored_seq,
                message_id,
                sender_account_id,
                sender_device_id,
                nonce,
                ciphertext,
                stored_timestamp_unix_seconds,
            ) = row?;
            let seq = sqlite_app_event_seq_to_u64(stored_seq)?;
            let timestamp_unix_seconds = sqlite_timestamp_to_u64(stored_timestamp_unix_seconds)?;
            let sender = DeviceRef {
                account_id: sender_account_id,
                device_id: sender_device_id,
            };
            let plaintext = decrypt_app_event_plaintext(
                &self.options.encryption_key,
                AppMessageIdentity {
                    owner,
                    room_id: &stored_room_id,
                    seq,
                    message_id: &message_id,
                    sender: &sender,
                },
                &nonce,
                &ciphertext,
            )?;
            let event = StoredAppEvent {
                room_id: stored_room_id,
                seq,
                message_id,
                sender,
                plaintext,
                timestamp_unix_seconds,
            };
            event.validate_limits()?;
            events.push(event);
        }
        debug_assert!(events.len() <= limit as usize);
        Ok(events)
    }

    pub fn load_app_outbox(
        &self,
        owner: &DeviceRef,
    ) -> Result<Vec<StoredOutboundMessage>, ClientStoreError> {
        validate_app_message_owner(owner)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT room_id, message_id, nonce, ciphertext
            FROM client_app_outbox
            WHERE account_id = ?1 AND device_id = ?2
            ORDER BY rowid ASC
            "#,
        )?;
        let rows = stmt.query_map(params![&owner.account_id, &owner.device_id], |row| {
            Ok((
                row.get::<_, RoomId>(0)?,
                row.get::<_, MessageId>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?;
        let mut messages = Vec::new();
        for row in rows {
            let (room_id, message_id, nonce, ciphertext) = row?;
            let metadata = decrypt_app_outbox_metadata(
                &self.options.encryption_key,
                AppOutboxIdentity {
                    owner,
                    room_id: &room_id,
                    message_id: &message_id,
                },
                &nonce,
                &ciphertext,
            )?;
            let message = StoredOutboundMessage {
                room_id,
                message_id,
                sender: metadata.sender,
                plaintext: metadata.plaintext,
                local_state: metadata.local_state,
                server_delivery_state: metadata.server_delivery_state,
                append_request: metadata.append_request,
                timestamp_unix_seconds: metadata.timestamp_unix_seconds,
            };
            message.validate_limits()?;
            messages.push(message);
        }
        Ok(messages)
    }

    pub fn save_app_outbox(
        &mut self,
        owner: &DeviceRef,
        messages: &[StoredOutboundMessage],
    ) -> Result<(), ClientStoreError> {
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| {
            save_app_outbox_tx(tx, &encryption_key, owner, messages)?;
            prune_app_outbox_tx(tx, owner, MAX_STORED_APP_OUTBOX_MESSAGES)
        })
    }

    pub fn delete_app_outbox_message(
        &mut self,
        owner: &DeviceRef,
        room_id: &str,
        message_id: &str,
    ) -> Result<(), ClientStoreError> {
        validate_app_message_owner(owner)?;
        validate_room_id(room_id).map_err(ClientError::from)?;
        validate_string_bytes("app_outbox.message_id", message_id, MAX_OBJECT_ID_BYTES)
            .map_err(ClientError::from)?;
        self.with_transaction(|tx| {
            tx.execute(
                r#"
                DELETE FROM client_app_outbox
                WHERE account_id = ?1 AND device_id = ?2 AND room_id = ?3 AND message_id = ?4
                "#,
                params![&owner.account_id, &owner.device_id, room_id, message_id],
            )?;
            Ok(())
        })
    }

    pub fn load_app_rooms(
        &self,
        owner: &DeviceRef,
    ) -> Result<Vec<StoredAppRoom>, ClientStoreError> {
        validate_app_message_owner(owner)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT room_id, nonce, ciphertext
            FROM client_app_rooms
            WHERE account_id = ?1 AND device_id = ?2
            ORDER BY room_id ASC
            "#,
        )?;
        let rows = stmt.query_map(params![&owner.account_id, &owner.device_id], |row| {
            Ok((
                row.get::<_, RoomId>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        let mut rooms = Vec::new();
        for row in rows {
            let (room_id, nonce, ciphertext) = row?;
            validate_room_id(&room_id).map_err(ClientError::from)?;
            let metadata = decrypt_app_room_metadata(
                &self.options.encryption_key,
                AppRoomIdentity {
                    owner,
                    room_id: &room_id,
                },
                &nonce,
                &ciphertext,
            )?;
            let room = StoredAppRoom {
                room_id,
                display_name: metadata.display_name,
                picture: metadata.picture,
                state: metadata.state,
                status: metadata.status,
                local_read_seq: metadata.local_read_seq,
            };
            room.validate_limits()?;
            rooms.push(room);
        }
        Ok(rooms)
    }

    pub fn save_app_rooms(
        &mut self,
        owner: &DeviceRef,
        rooms: &[StoredAppRoom],
    ) -> Result<(), ClientStoreError> {
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| save_app_rooms_tx(tx, &encryption_key, owner, rooms))
    }

    pub fn load_app_state(&self, owner: &DeviceRef) -> Result<StoredAppState, ClientStoreError> {
        validate_app_message_owner(owner)?;
        let row = self
            .conn
            .query_row(
                r#"
                SELECT nonce, ciphertext
                FROM client_app_state
                WHERE account_id = ?1 AND device_id = ?2
                "#,
                params![&owner.account_id, &owner.device_id],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?;
        let Some((nonce, ciphertext)) = row else {
            return Ok(StoredAppState::default());
        };
        let metadata =
            decrypt_app_state_metadata(&self.options.encryption_key, owner, &nonce, &ciphertext)?;
        let state = StoredAppState {
            selected_room_id: metadata.selected_room_id,
            selected_topic_id: metadata.selected_topic_id,
            selected_chat_id: metadata.selected_chat_id,
            paired_agent: metadata.paired_agent,
            revoked_devices: metadata.revoked_devices,
            chat_archives: metadata.chat_archives,
        };
        state.validate_limits()?;
        Ok(state)
    }

    pub fn save_app_state(
        &mut self,
        owner: &DeviceRef,
        state: &StoredAppState,
    ) -> Result<(), ClientStoreError> {
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| save_app_state_tx(tx, &encryption_key, owner, state))
    }

    pub fn load_app_profiles(
        &self,
        owner: &DeviceRef,
    ) -> Result<Vec<StoredAppProfile>, ClientStoreError> {
        validate_app_message_owner(owner)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT profile_account_id, nonce, ciphertext
            FROM client_app_profiles
            WHERE account_id = ?1 AND device_id = ?2
            ORDER BY profile_account_id ASC
            "#,
        )?;
        let rows = stmt.query_map(params![&owner.account_id, &owner.device_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        let mut profiles = Vec::new();
        for row in rows {
            let (profile_account_id, nonce, ciphertext) = row?;
            decode_lower_hex_32(&profile_account_id)?;
            let metadata = decrypt_app_profile_metadata(
                &self.options.encryption_key,
                AppProfileIdentity {
                    owner,
                    profile_account_id: &profile_account_id,
                },
                &nonce,
                &ciphertext,
            )?;
            let profile = StoredAppProfile {
                profile: metadata.profile,
                stale: metadata.stale,
            };
            profile.validate_limits()?;
            profiles.push(profile);
        }
        Ok(profiles)
    }

    pub fn save_app_profiles(
        &mut self,
        owner: &DeviceRef,
        profiles: &[StoredAppProfile],
    ) -> Result<(), ClientStoreError> {
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| {
            save_app_profiles_tx(tx, &encryption_key, owner, profiles)?;
            prune_app_profiles_tx(tx, owner, MAX_STORED_APP_PROFILES)
        })
    }

    pub fn save_device_state_and_app_rooms(
        &mut self,
        device: &FiniteChatDevice,
        rooms: &[StoredAppRoom],
    ) -> Result<(), ClientStoreError> {
        let state = device.export_state()?;
        let owner = state.device_ref.clone();
        let encryption_key = self.options.encryption_key.clone();
        self.with_transaction(|tx| {
            save_device_state_tx(tx, &state, &encryption_key)?;
            save_app_rooms_tx(tx, &encryption_key, &owner, rooms)
        })
    }

    pub fn load_device(
        &self,
        config: FiniteChatDeviceConfig,
    ) -> Result<FiniteChatDevice, ClientStoreError> {
        let account_id = hex_lower(config.account_secret_key.public_key().as_bytes());
        let device_id = config.device_id.clone();
        let state = load_device_state(
            &self.conn,
            &self.options.encryption_key,
            &account_id,
            &device_id,
        )?
        .ok_or(ClientStoreError::DeviceStateNotFound {
            account_id,
            device_id,
        })?;
        Ok(FiniteChatDevice::from_state(config, state)?)
    }

    pub fn load_device_ids_for_account(
        &self,
        account_id: &str,
    ) -> Result<Vec<String>, ClientStoreError> {
        validate_string_bytes("account_id", account_id, MAX_ACCOUNT_ID_BYTES)
            .map_err(ClientError::from)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT device_id
            FROM client_device_states
            WHERE account_id = ?1
            ORDER BY device_id
            "#,
        )?;
        let rows = stmt.query_map(params![account_id], |row| row.get::<_, String>(0))?;
        let mut device_ids = Vec::new();
        for row in rows {
            device_ids.push(row?);
        }
        Ok(device_ids)
    }

    pub fn activate_welcome_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        welcome_id: impl Into<WelcomeId>,
        room_id: impl Into<RoomId>,
        welcome_payload: &[u8],
        ratchet_tree_payload: &[u8],
        commit_seq: u64,
    ) -> Result<(), ClientStoreError> {
        let welcome_id = welcome_id.into();
        let room_id = room_id.into();
        device.activate_welcome(room_id.clone(), welcome_payload, ratchet_tree_payload)?;
        device.set_last_applied_seq(&room_id, commit_seq)?;
        device.store_pending_welcome_ack(PendingWelcomeAckState {
            welcome_id,
            room_id,
            commit_seq,
        })?;
        self.save_device_state(device)
    }

    pub fn store_pending_welcome_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        welcome: &WelcomeRecord,
    ) -> Result<(), ClientStoreError> {
        device.store_pending_welcome(welcome)?;
        self.save_device_state(device)
    }

    pub fn activate_pending_welcome_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        welcome_id: &str,
    ) -> Result<u64, ClientStoreError> {
        let ack = device.activate_pending_welcome(welcome_id)?;
        self.save_device_state(device)?;
        Ok(ack.commit_seq)
    }

    pub fn clear_pending_welcome_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        welcome_id: &str,
    ) -> Result<(), ClientStoreError> {
        device.clear_pending_welcome(welcome_id)?;
        self.save_device_state(device)
    }

    pub fn clear_pending_welcome_ack_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        welcome_id: &str,
    ) -> Result<(), ClientStoreError> {
        device.clear_pending_welcome_ack(welcome_id)?;
        self.save_device_state(device)
    }

    pub fn clear_pending_key_package_upload_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        key_package_id: &str,
    ) -> Result<(), ClientStoreError> {
        device.clear_pending_key_package_upload(key_package_id)?;
        self.save_device_state(device)
    }

    pub fn start_link_fanout_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        fanout_id: impl Into<String>,
        target_device: DeviceRef,
    ) -> Result<(), ClientStoreError> {
        device.start_link_fanout(fanout_id, target_device)?;
        self.save_device_state(device)
    }

    pub fn queue_link_fanout_page_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        fanout_id: &str,
        page: &ListAccountRoomsPage,
        plans: &[LinkFanoutRoomPlan],
    ) -> Result<(), ClientStoreError> {
        device.queue_link_fanout_page(fanout_id, page, plans)?;
        self.save_device_state(device)
    }

    pub fn queue_claimed_link_fanout_page_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        fanout_id: &str,
        page: &ListAccountRoomsPage,
        claimed_key_packages: &[ClaimKeyPackageResult],
    ) -> Result<(), ClientStoreError> {
        device.queue_claimed_link_fanout_page(fanout_id, page, claimed_key_packages)?;
        self.save_device_state(device)
    }

    pub fn prepare_link_fanout_room_commit_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        fanout_id: &str,
        room_id: &str,
        claimed_key_package: &ClaimKeyPackageResult,
    ) -> Result<PreparedCommit, ClientStoreError> {
        let prepared =
            device.prepare_link_fanout_room_commit(fanout_id, room_id, claimed_key_package)?;
        self.save_device_state(device)?;
        Ok(prepared)
    }

    pub fn prepare_claimed_link_fanout_room_commit_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        fanout_id: &str,
        room_id: &str,
    ) -> Result<PreparedCommit, ClientStoreError> {
        let prepared = device.prepare_claimed_link_fanout_room_commit(fanout_id, room_id)?;
        self.save_device_state(device)?;
        Ok(prepared)
    }

    pub fn complete_link_fanout_room_from_log_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        fanout_id: &str,
        room_id: &str,
        entry: &RoomLogEntry,
    ) -> Result<Option<AppliedLogEntry>, ClientStoreError> {
        if entry.seq <= device.last_applied_seq(room_id)? {
            return Ok(None);
        }
        let applied = device.complete_link_fanout_room_from_log(fanout_id, room_id, entry)?;
        device.set_last_applied_seq(room_id, entry.seq)?;
        let app_messages = stored_app_message_from_applied(
            room_id,
            entry.seq,
            &entry.message_id,
            entry.timestamp_unix_seconds,
            &applied,
        )
        .into_iter()
        .collect::<Vec<_>>();
        let app_events = stored_app_event_from_applied(
            room_id,
            entry.seq,
            &entry.message_id,
            entry.timestamp_unix_seconds,
            &applied,
        )
        .into_iter()
        .collect::<Vec<_>>();
        self.save_device_state_and_app_messages_and_events(device, &app_messages, &app_events)?;
        Ok(Some(applied))
    }

    pub fn apply_log_entry_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        room_id: &str,
        entry: &RoomLogEntry,
    ) -> Result<Option<AppliedLogEntry>, ClientStoreError> {
        let before_seq = device.last_applied_seq(room_id)?;
        let applied = apply_log_entry_in_memory(device, room_id, entry)?;
        if device.last_applied_seq(room_id)? > before_seq {
            let app_messages = applied
                .as_ref()
                .and_then(|applied_entry| {
                    stored_app_message_from_applied(
                        room_id,
                        entry.seq,
                        &entry.message_id,
                        entry.timestamp_unix_seconds,
                        applied_entry,
                    )
                })
                .into_iter()
                .collect::<Vec<_>>();
            let app_events = applied
                .as_ref()
                .and_then(|applied_entry| {
                    stored_app_event_from_applied(
                        room_id,
                        entry.seq,
                        &entry.message_id,
                        entry.timestamp_unix_seconds,
                        applied_entry,
                    )
                })
                .into_iter()
                .collect::<Vec<_>>();
            self.save_device_state_and_app_messages_and_events(device, &app_messages, &app_events)?;
        }
        Ok(applied)
    }

    pub fn advance_room_cursor_and_save(
        &mut self,
        device: &mut FiniteChatDevice,
        room_id: &str,
        seq: u64,
    ) -> Result<(), ClientStoreError> {
        device.set_last_applied_seq(room_id, seq)?;
        self.save_device_state(device)
    }

    fn with_transaction<T>(
        &mut self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, ClientStoreError>,
    ) -> Result<T, ClientStoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    }
}

/// Apply one ordered log entry to the in-memory device without persisting.
///
/// The sync workers apply their bounded room tick in memory and save only
/// after every fetched page succeeds. The `seq <= last_applied_seq` guard
/// makes replay idempotent against the last complete room tick.
fn apply_log_entry_in_memory(
    device: &mut FiniteChatDevice,
    room_id: &str,
    entry: &RoomLogEntry,
) -> Result<Option<AppliedLogEntry>, ClientStoreError> {
    validate_room_id(room_id).map_err(ClientError::from)?;
    if entry.room_id != room_id {
        return Err(ClientError::LogEntryRoomMismatch {
            expected: room_id.to_string(),
            actual: entry.room_id.clone(),
        }
        .into());
    }
    let last_applied_seq = device.last_applied_seq(room_id)?;
    if entry.seq <= last_applied_seq {
        let own_pending_commit = entry.kind == LogEntryKind::Commit
            && entry.sender == *device.device_ref()
            && device.has_pending_commit(room_id)?;
        if own_pending_commit {
            return match device.apply_log_entry(room_id, entry) {
                Ok(applied) => Ok(Some(applied)),
                Err(ClientError::UnexpectedCommitEpoch { .. }) => Ok(None),
                Err(error) => Err(error.into()),
            };
        }
        return Ok(None);
    }
    // Own application messages cannot be decrypted by their sender (MLS);
    // they advance the cursor without producing an applied entry. Commits
    // are never skipped: own commits go through the pending-merge rule.
    if entry.kind == LogEntryKind::Application && entry.envelope.sender == *device.device_ref() {
        device.set_last_applied_seq(room_id, entry.seq)?;
        return Ok(None);
    }
    let applied = device.apply_log_entry(room_id, entry)?;
    device.set_last_applied_seq(room_id, entry.seq)?;
    Ok(Some(applied))
}

fn stored_app_message_from_applied(
    room_id: &str,
    seq: u64,
    message_id: &str,
    timestamp_unix_seconds: u64,
    applied: &AppliedLogEntry,
) -> Option<StoredAppMessage> {
    let AppliedLogEntry::Application { plaintext, sender } = applied else {
        return None;
    };
    Some(StoredAppMessage {
        room_id: room_id.to_owned(),
        seq,
        message_id: message_id.to_owned(),
        sender: sender.clone(),
        plaintext: plaintext.clone(),
        timestamp_unix_seconds,
    })
}

fn stored_app_event_from_applied(
    room_id: &str,
    seq: u64,
    message_id: &str,
    timestamp_unix_seconds: u64,
    applied: &AppliedLogEntry,
) -> Option<StoredAppEvent> {
    let AppliedLogEntry::Application { plaintext, sender } = applied else {
        return None;
    };
    Some(StoredAppEvent {
        room_id: room_id.to_owned(),
        seq,
        message_id: message_id.to_owned(),
        sender: sender.clone(),
        plaintext: plaintext.clone(),
        timestamp_unix_seconds,
    })
}

fn open_client_store_connection(db_path: &Path) -> Result<Connection, ClientStoreError> {
    let conn = Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = FULL;
        PRAGMA foreign_keys = ON;
        PRAGMA busy_timeout = 5000;
        "#,
    )?;
    Ok(conn)
}

pub trait RuntimeDelivery {
    type Error;

    fn key_package_inventory(
        &mut self,
        owner: &DeviceRef,
    ) -> Result<KeyPackageInventory, Self::Error>;

    fn upload_key_package(&mut self, request: UploadKeyPackageRequest) -> Result<(), Self::Error>;

    fn claim_key_package_for_device(
        &mut self,
        owner: &DeviceRef,
    ) -> Result<Option<ClaimKeyPackageResult>, Self::Error>;

    fn claim_key_package_for_account(
        &mut self,
        account_id: &str,
    ) -> Result<Option<ClaimKeyPackageResult>, Self::Error>;

    fn submit_commit(
        &mut self,
        request: SubmitCommitRequest,
    ) -> Result<CommitAccepted, Self::Error>;

    fn list_account_rooms(
        &mut self,
        request: ListAccountRoomsRequest,
    ) -> Result<ListAccountRoomsPage, Self::Error>;

    fn claim_welcomes(&mut self, device: &DeviceRef) -> Result<Vec<WelcomeRecord>, Self::Error>;

    fn ack_welcome(&mut self, welcome_id: &str) -> Result<(), Self::Error>;

    fn sync_events(
        &mut self,
        room_id: &str,
        requester: &DeviceRef,
        after_seq: u64,
    ) -> Result<SyncEventsPage, Self::Error>;
}

pub trait HttpRuntimeTransport {
    type Error;

    fn post_json<T, R>(&mut self, uri: &str, body: &T) -> Result<R, Self::Error>
    where
        T: Serialize,
        R: DeserializeOwned;
}

#[derive(Debug, Clone)]
pub struct ReqwestHttpRuntimeTransport {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl ReqwestHttpRuntimeTransport {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_client(base_url, reqwest::blocking::Client::new())
    }

    pub fn with_client(base_url: impl Into<String>, client: reqwest::blocking::Client) -> Self {
        Self {
            base_url: base_url.into(),
            client,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn route_url(&self, uri: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            uri.trim_start_matches('/')
        )
    }

    pub fn sync_stream(
        &mut self,
        request: &SyncStreamRequest,
    ) -> Result<ReqwestSyncHintStream, ReqwestHttpRuntimeTransportError> {
        let response = self
            .client
            .post(self.route_url("/sync/stream"))
            .json(request)
            .send()
            .map_err(ReqwestHttpRuntimeTransportError::Request)?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .map_err(ReqwestHttpRuntimeTransportError::Request)?;
            return Err(ReqwestHttpRuntimeTransportError::Server { status, body });
        }
        Ok(ReqwestSyncHintStream {
            response,
            buffer: String::new(),
        })
    }
}

impl HttpRuntimeTransport for ReqwestHttpRuntimeTransport {
    type Error = ReqwestHttpRuntimeTransportError;

    fn post_json<T, R>(&mut self, uri: &str, body: &T) -> Result<R, Self::Error>
    where
        T: Serialize,
        R: DeserializeOwned,
    {
        let response = self
            .client
            .post(self.route_url(uri))
            .json(body)
            .send()
            .map_err(ReqwestHttpRuntimeTransportError::Request)?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .map_err(ReqwestHttpRuntimeTransportError::Request)?;
            return Err(ReqwestHttpRuntimeTransportError::Server { status, body });
        }
        response
            .json()
            .map_err(ReqwestHttpRuntimeTransportError::Request)
    }
}

#[derive(Debug)]
pub enum ReqwestHttpRuntimeTransportError {
    Request(reqwest::Error),
    Server {
        status: reqwest::StatusCode,
        body: String,
    },
}

impl fmt::Display for ReqwestHttpRuntimeTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(error) => {
                write!(
                    formatter,
                    "HTTP runtime request failed: {}",
                    error_message_with_sources(error)
                )
            }
            Self::Server { status, body } => {
                write!(formatter, "server returned {status}: {body}")
            }
        }
    }
}

impl StdError for ReqwestHttpRuntimeTransportError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Request(error) => Some(error),
            Self::Server { .. } => None,
        }
    }
}

fn error_message_with_sources(error: &(dyn StdError + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(next) = source {
        let next_message = next.to_string();
        if !next_message.is_empty() && !message.contains(&next_message) {
            message.push_str(": ");
            message.push_str(&next_message);
        }
        source = next.source();
    }
    message
}

pub struct ReqwestSyncHintStream {
    response: reqwest::blocking::Response,
    buffer: String,
}

impl ReqwestSyncHintStream {
    pub fn next_hint(&mut self) -> Result<SyncHintEvent, ReqwestSyncHintStreamError> {
        loop {
            while !self.buffer.contains("\n\n") {
                let mut chunk = [0u8; 4096];
                let read = self
                    .response
                    .read(&mut chunk)
                    .map_err(ReqwestSyncHintStreamError::Read)?;
                if read == 0 {
                    return Err(ReqwestSyncHintStreamError::Ended);
                }
                let text = std::str::from_utf8(&chunk[..read])
                    .map_err(ReqwestSyncHintStreamError::Utf8)?;
                self.buffer.push_str(text);
            }

            let Some(split_at) = self.buffer.find("\n\n") else {
                continue;
            };
            let raw_event = self.buffer[..split_at].to_owned();
            self.buffer = self.buffer[split_at + 2..].to_owned();
            let data = raw_event
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                continue;
            }
            return serde_json::from_str(&data).map_err(ReqwestSyncHintStreamError::Json);
        }
    }
}

#[derive(Debug, Error)]
pub enum ReqwestSyncHintStreamError {
    #[error("SSE hint stream read failed: {0}")]
    Read(std::io::Error),
    #[error("SSE hint stream was not UTF-8: {0}")]
    Utf8(std::str::Utf8Error),
    #[error("SSE hint event JSON was malformed: {0}")]
    Json(serde_json::Error),
    #[error("SSE hint stream ended")]
    Ended,
}

#[derive(Debug, Clone)]
pub struct HttpRuntimeDelivery<T> {
    transport: T,
}

impl<T> HttpRuntimeDelivery<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn into_transport(self) -> T {
        self.transport
    }
}

impl<T: HttpRuntimeTransport> HttpRuntimeDelivery<T> {
    pub fn publish_account_room_record(
        &mut self,
        account_id: &str,
        record: &AccountRoomRecord,
    ) -> Result<(), HttpRuntimeDeliveryError<T::Error>> {
        let request = SaveAccountRoomRequest {
            account_id: account_id.to_owned(),
            room_id: record.room_id.clone(),
            record: serde_json::to_value(record)
                .map_err(|error| HttpRuntimeDeliveryError::Json(error.to_string()))?,
        };
        let _: SaveAccountRoomResponse = self.post_json("/account-rooms", &request)?;
        Ok(())
    }

    pub fn bootstrap_account_room(
        &mut self,
        request: &CreateRoomRequest,
    ) -> Result<(), HttpRuntimeDeliveryError<T::Error>> {
        let request = BootstrapAccountRoomRequest {
            room_id: request.room_id.clone(),
            mls_group_id: request.mls_group_id.clone(),
            creator: request.creator.clone(),
            protocol: request.protocol.clone(),
        };
        let _: BootstrapAccountRoomResponse =
            self.post_json("/account-rooms/bootstrap", &request)?;
        Ok(())
    }

    /// Long-poll wake hint (/sync/wait): purely advisory, never advances
    /// state. Returns when a watched room changes or wait_ms passes.
    pub fn sync_wait(
        &mut self,
        request: &SyncWaitRequest,
    ) -> Result<SyncWaitResponse, HttpRuntimeDeliveryError<T::Error>> {
        self.post_json("/sync/wait", request)
    }

    pub fn append_activity(
        &mut self,
        request: &AppendEphemeralActivityRequest,
    ) -> Result<EphemeralActivityAccepted, HttpRuntimeDeliveryError<T::Error>> {
        self.post_json("/activities", request)
    }

    pub fn get_ephemeral_activities(
        &mut self,
        request: &GetEphemeralActivitiesRequest,
    ) -> Result<GetEphemeralActivitiesResponse, HttpRuntimeDeliveryError<T::Error>> {
        self.post_json("/activities/get", request)
    }

    pub fn put_nostr_profile(
        &mut self,
        profile: &NostrProfileRecord,
    ) -> Result<PutNostrProfileResponse, HttpRuntimeDeliveryError<T::Error>> {
        self.post_json(
            "/profiles/nostr",
            &PutNostrProfileRequest {
                profile: profile.clone(),
            },
        )
    }

    pub fn get_nostr_profiles(
        &mut self,
        account_ids: Vec<String>,
        now_ms: u64,
    ) -> Result<GetNostrProfilesResponse, HttpRuntimeDeliveryError<T::Error>> {
        self.post_json(
            "/profiles/nostr/get",
            &GetNostrProfilesRequest {
                account_ids,
                now_ms,
            },
        )
    }

    pub fn revoke_device(
        &mut self,
        device: &DeviceRef,
    ) -> Result<RevokeDeviceResponse, HttpRuntimeDeliveryError<T::Error>> {
        self.post_json(
            "/devices/revoke",
            &RevokeDeviceRequest {
                device: device.clone(),
            },
        )
    }

    pub fn register_push_token(
        &mut self,
        device: &DeviceRef,
        platform: PushPlatform,
        token: String,
    ) -> Result<RegisterPushTokenResponse, HttpRuntimeDeliveryError<T::Error>> {
        self.post_json(
            "/push-tokens",
            &RegisterPushTokenRequest {
                device: device.clone(),
                platform,
                token,
            },
        )
    }

    pub fn remove_push_token(
        &mut self,
        device: &DeviceRef,
    ) -> Result<RemovePushTokenResponse, HttpRuntimeDeliveryError<T::Error>> {
        self.post_json(
            "/push-tokens/remove",
            &RemovePushTokenRequest {
                device: device.clone(),
                token: None,
            },
        )
    }

    pub fn append_event(
        &mut self,
        request: &AppendEventRequest,
        delivery_policy: ApplicationDeliveryPolicy,
    ) -> Result<EventAccepted, HttpRuntimeDeliveryError<T::Error>> {
        self.post_json(
            "/events",
            &AppendApplicationEventRequest {
                event: request.clone(),
                delivery_policy,
            },
        )
    }

    fn post_json<B, R>(
        &mut self,
        uri: &str,
        body: &B,
    ) -> Result<R, HttpRuntimeDeliveryError<T::Error>>
    where
        B: Serialize,
        R: DeserializeOwned,
    {
        self.transport
            .post_json(uri, body)
            .map_err(HttpRuntimeDeliveryError::Transport)
    }
}

impl HttpRuntimeDelivery<ReqwestHttpRuntimeTransport> {
    pub fn sync_stream(
        &mut self,
        request: &SyncStreamRequest,
    ) -> Result<ReqwestSyncHintStream, HttpRuntimeDeliveryError<ReqwestHttpRuntimeTransportError>>
    {
        self.transport
            .sync_stream(request)
            .map_err(HttpRuntimeDeliveryError::Transport)
    }
}

impl<T: HttpRuntimeTransport> RuntimeDelivery for HttpRuntimeDelivery<T> {
    type Error = HttpRuntimeDeliveryError<T::Error>;

    fn key_package_inventory(
        &mut self,
        owner: &DeviceRef,
    ) -> Result<KeyPackageInventory, Self::Error> {
        let owner_id = http_member_id_for_device(owner)?;
        let inventory: HttpKeyPackageInventory = self.post_json(
            "/key-packages/inventory",
            &KeyPackageInventoryRequest { owner: owner_id },
        )?;
        Ok(KeyPackageInventory {
            owner: owner.clone(),
            available: inventory.available,
            leased: inventory.claimed,
        })
    }

    fn upload_key_package(&mut self, request: UploadKeyPackageRequest) -> Result<(), Self::Error> {
        let publication = HttpKeyPackagePublication {
            key_package_id: HttpKeyPackageId::new(request.key_package_id.as_bytes().to_vec()),
            owner: http_member_id_for_device(&request.owner)?,
            key_package: HttpKeyPackage::new(
                serde_json::to_vec(&request)
                    .map_err(|error| HttpRuntimeDeliveryError::Json(error.to_string()))?,
            ),
        };
        let _: PublishKeyPackageResponse = self.post_json("/key-packages", &publication)?;
        Ok(())
    }

    fn claim_key_package_for_device(
        &mut self,
        owner: &DeviceRef,
    ) -> Result<Option<ClaimKeyPackageResult>, Self::Error> {
        let claimed: Option<finitechat_delivery::HttpClaimedKeyPackage> = self.post_json(
            "/key-packages/claim",
            &ClaimKeyPackageRequest {
                owner: http_member_id_for_device(owner)?,
            },
        )?;
        claimed
            .map(|claimed| {
                let result = claimed_key_package_result_from_http(claimed)?;
                if result.owner != *owner {
                    return Err(HttpRuntimeDeliveryError::KeyPackageOwnerMismatch {
                        expected: owner.clone(),
                        actual: result.owner,
                    });
                }
                Ok(result)
            })
            .transpose()
    }

    fn claim_key_package_for_account(
        &mut self,
        account_id: &str,
    ) -> Result<Option<ClaimKeyPackageResult>, Self::Error> {
        let claimed: Option<finitechat_delivery::HttpClaimedKeyPackage> = self.post_json(
            "/key-packages/claim-account",
            &ClaimKeyPackageForAccountRequest {
                account_id: account_id.to_owned(),
            },
        )?;
        claimed
            .map(|claimed| {
                let result = claimed_key_package_result_from_http(claimed)?;
                if result.owner.account_id != account_id {
                    return Err(HttpRuntimeDeliveryError::KeyPackageAccountMismatch {
                        expected: account_id.to_owned(),
                        actual: result.owner,
                    });
                }
                Ok(result)
            })
            .transpose()
    }

    fn submit_commit(
        &mut self,
        request: SubmitCommitRequest,
    ) -> Result<CommitAccepted, Self::Error> {
        request
            .validate_limits()
            .map_err(|error| HttpRuntimeDeliveryError::CommitValidation(error.to_string()))?;
        let message_id = request
            .envelope
            .message_id()
            .map_err(|error| HttpRuntimeDeliveryError::Json(error.to_string()))?;
        if request.envelope.kind != LogEntryKind::Commit {
            return Err(HttpRuntimeDeliveryError::CommitValidation(
                "commit request envelope must be a commit".to_owned(),
            ));
        }
        if request.envelope.epoch != request.expected_epoch {
            return Err(HttpRuntimeDeliveryError::CommitValidation(format!(
                "commit envelope epoch {} does not match expected epoch {}",
                request.envelope.epoch, request.expected_epoch
            )));
        }
        if request.envelope.sender != request.sender {
            return Err(HttpRuntimeDeliveryError::CommitValidation(
                "commit envelope sender does not match request sender".to_owned(),
            ));
        }
        request
            .membership_delta
            .validate_structure(request.expected_epoch, &message_id)
            .map_err(|error| HttpRuntimeDeliveryError::CommitValidation(error.to_string()))?;
        self.post_json("/commits", &request)
    }

    fn list_account_rooms(
        &mut self,
        request: ListAccountRoomsRequest,
    ) -> Result<ListAccountRoomsPage, Self::Error> {
        let response: ListAccountRoomDirectoryResponse = self.post_json(
            "/account-rooms/list",
            &ListAccountRoomDirectoryRequest {
                account_id: request.account_id.clone(),
                after_room_id: request.after_room_id.clone(),
                limit: request.limit as usize,
            },
        )?;
        let rooms = response
            .rooms
            .into_iter()
            .map(|record| {
                serde_json::from_value(record)
                    .map_err(|error| HttpRuntimeDeliveryError::Json(error.to_string()))
            })
            .collect::<Result<Vec<AccountRoomRecord>, _>>()?;
        let page = ListAccountRoomsPage {
            rooms,
            next_after_room_id: response.next_after_room_id,
            has_more: response.has_more,
        };
        page.validate_limits()
            .map_err(|error| HttpRuntimeDeliveryError::Json(error.to_string()))?;
        Ok(page)
    }

    fn claim_welcomes(&mut self, device: &DeviceRef) -> Result<Vec<WelcomeRecord>, Self::Error> {
        let claimed: Vec<HttpClaimedWelcome> = self.post_json(
            "/welcomes/claim",
            &ClaimWelcomesRequest {
                recipient: http_member_id_for_device(device)?,
                limit: MAX_WELCOME_CLAIMS_PER_REQUEST as usize,
            },
        )?;
        claimed
            .into_iter()
            .map(|claim| {
                let mut welcome: WelcomeRecord = serde_json::from_slice(&claim.message.payload)
                    .map_err(|error| HttpRuntimeDeliveryError::Json(error.to_string()))?;
                if claim.message.id.as_slice() != welcome.welcome_id.as_bytes() {
                    return Err(HttpRuntimeDeliveryError::WelcomeIdMismatch {
                        message_id: claim.message.id.as_slice().to_vec(),
                        welcome_id: welcome.welcome_id,
                    });
                }
                if welcome.recipient != *device {
                    return Err(HttpRuntimeDeliveryError::WelcomeRecipientMismatch {
                        expected: device.clone(),
                        actual: welcome.recipient,
                    });
                }
                welcome.state = WelcomeState::Claimed;
                Ok(welcome)
            })
            .collect()
    }

    fn ack_welcome(&mut self, welcome_id: &str) -> Result<(), Self::Error> {
        let _: AckWelcomeResponse = self.post_json(
            "/welcomes/ack",
            &AckWelcomeRequest {
                message_id: HttpMessageId::new(welcome_id.as_bytes().to_vec()),
            },
        )?;
        Ok(())
    }

    fn sync_events(
        &mut self,
        room_id: &str,
        requester: &DeviceRef,
        after_seq: u64,
    ) -> Result<SyncEventsPage, Self::Error> {
        let page: HttpSyncPage = self.post_json(
            "/sync/group",
            &GroupSyncRequest {
                group_id: http_group_id_for_room(room_id),
                after_seq,
                limit: MAX_HTTP_SYNC_PAGE_ENTRIES,
                requester: Some(http_member_id_for_device(requester)?),
            },
        )?;
        let entries = page
            .entries
            .into_iter()
            .map(|queued| {
                let mut entry = decode_http_room_log_entry(&queued.message.payload)?;
                if entry.room_id != room_id {
                    return Err(HttpRuntimeDeliveryError::RoomEntryMismatch {
                        expected: room_id.to_owned(),
                        actual: entry.room_id,
                    });
                }
                entry.seq = queued.seq;
                Ok(entry)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SyncEventsPage {
            entries,
            next_after_seq: page.next_after_seq,
            has_more: page.has_more,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum HttpRuntimeDeliveryError<E> {
    Transport(E),
    Json(String),
    WelcomeIdMismatch {
        message_id: Vec<u8>,
        welcome_id: String,
    },
    WelcomeRecipientMismatch {
        expected: DeviceRef,
        actual: DeviceRef,
    },
    KeyPackageIdMismatch {
        envelope_id: Vec<u8>,
        body_id: String,
    },
    KeyPackageOwnerMismatch {
        expected: DeviceRef,
        actual: DeviceRef,
    },
    KeyPackageAccountMismatch {
        expected: String,
        actual: DeviceRef,
    },
    RoomEntryMismatch {
        expected: String,
        actual: String,
    },
    CommitValidation(String),
}

impl<E: fmt::Display> fmt::Display for HttpRuntimeDeliveryError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "HTTP runtime transport failed: {error}"),
            Self::Json(error) => write!(formatter, "HTTP runtime JSON error: {error}"),
            Self::WelcomeIdMismatch {
                message_id,
                welcome_id,
            } => write!(
                formatter,
                "Welcome id mismatch: envelope {message_id:?}, payload {welcome_id}"
            ),
            Self::WelcomeRecipientMismatch { expected, actual } => write!(
                formatter,
                "Welcome recipient mismatch: expected {expected:?}, actual {actual:?}"
            ),
            Self::KeyPackageIdMismatch {
                envelope_id,
                body_id,
            } => write!(
                formatter,
                "KeyPackage id mismatch: envelope {envelope_id:?}, payload {body_id}"
            ),
            Self::KeyPackageOwnerMismatch { expected, actual } => write!(
                formatter,
                "KeyPackage owner mismatch: expected {expected:?}, actual {actual:?}"
            ),
            Self::KeyPackageAccountMismatch { expected, actual } => write!(
                formatter,
                "KeyPackage account mismatch: expected {expected}, actual {actual:?}"
            ),
            Self::RoomEntryMismatch { expected, actual } => write!(
                formatter,
                "room log entry mismatch: expected {expected}, actual {actual}"
            ),
            Self::CommitValidation(error) => {
                write!(formatter, "HTTP runtime commit validation failed: {error}")
            }
        }
    }
}

impl<E> std::error::Error for HttpRuntimeDeliveryError<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Json(_)
            | Self::WelcomeIdMismatch { .. }
            | Self::WelcomeRecipientMismatch { .. }
            | Self::KeyPackageIdMismatch { .. }
            | Self::KeyPackageOwnerMismatch { .. }
            | Self::KeyPackageAccountMismatch { .. }
            | Self::RoomEntryMismatch { .. }
            | Self::CommitValidation(_) => None,
        }
    }
}

fn decode_http_room_log_entry<E>(
    payload: &[u8],
) -> Result<RoomLogEntry, HttpRuntimeDeliveryError<E>> {
    if let Ok(projection) = serde_json::from_slice::<FiniteAccountRoomCommitProjection>(payload) {
        return Ok(projection.entry);
    }
    serde_json::from_slice(payload)
        .map_err(|error| HttpRuntimeDeliveryError::Json(error.to_string()))
}

fn claimed_key_package_result_from_http<E>(
    claimed: finitechat_delivery::HttpClaimedKeyPackage,
) -> Result<ClaimKeyPackageResult, HttpRuntimeDeliveryError<E>> {
    let request: UploadKeyPackageRequest = serde_json::from_slice(claimed.key_package.bytes())
        .map_err(|error| HttpRuntimeDeliveryError::Json(error.to_string()))?;
    if claimed.key_package_id.as_slice() != request.key_package_id.as_bytes() {
        return Err(HttpRuntimeDeliveryError::KeyPackageIdMismatch {
            envelope_id: claimed.key_package_id.as_slice().to_vec(),
            body_id: request.key_package_id,
        });
    }
    Ok(ClaimKeyPackageResult {
        lease_token: lease_token_for(&request.key_package_id, &request.owner),
        key_package_id: request.key_package_id,
        owner: request.owner,
        key_package_ref: request.key_package_ref,
        key_package_hash: request.key_package_hash,
        key_package_payload: request.key_package_payload,
    })
}

fn http_member_id_for_device<E>(
    device: &DeviceRef,
) -> Result<HttpMemberId, HttpRuntimeDeliveryError<E>> {
    Ok(HttpMemberId::new(delivery_member_id_for_device(device)))
}

fn http_group_id_for_room(room_id: &str) -> HttpGroupId {
    HttpGroupId::new(room_id.as_bytes().to_vec())
}

pub fn run_runtime_sync_tick<D: RuntimeDelivery>(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut D,
    options: &RuntimeSyncOptions,
) -> Result<RuntimeSyncReport, RuntimeWorkerError<D::Error>> {
    let mut report = run_runtime_sync_setup_tick(store, device, delivery, options)?;

    // Group rooms by server (ADR 0005): this tick talks to the home
    // server, so rooms pinned to another room server are synced by
    // `run_room_server_sync_tick` with that server's transport.
    for cursor in device.room_sync_cursors() {
        if cursor.server_url.is_some() {
            continue;
        }
        sync_room_pages(
            store,
            device,
            delivery,
            options,
            cursor.room_id,
            cursor.after_seq,
            &mut report,
        )?;
    }

    Ok(report)
}

/// Run the home-server work which is shared by every room sync, without
/// advancing a room. Callers which need room-level failure isolation can run
/// this once and then use [`run_room_sync_tick`] with a fresh persisted Device
/// candidate for each room.
pub fn run_runtime_sync_setup_tick<D: RuntimeDelivery>(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut D,
    options: &RuntimeSyncOptions,
) -> Result<RuntimeSyncReport, RuntimeWorkerError<D::Error>> {
    options.validate_limits()?;
    let mut report = RuntimeSyncReport::default();

    let inventory = delivery
        .key_package_inventory(device.device_ref())
        .map_err(RuntimeWorkerError::Delivery)?;
    let replenishment =
        device.key_package_replenishment_plan(inventory, options.key_package_target_available)?;
    let upload_requests = replenishment.upload_requests;
    if !upload_requests.is_empty() {
        store.save_device_state(device)?;
    }
    for request in upload_requests {
        delivery
            .upload_key_package(request.clone())
            .map_err(RuntimeWorkerError::Delivery)?;
        store.clear_pending_key_package_upload_and_save(device, &request.key_package_id)?;
        report.record_uploaded_key_package()?;
    }

    let claimed_welcomes = delivery
        .claim_welcomes(device.device_ref())
        .map_err(RuntimeWorkerError::Delivery)?;
    report.record_claimed_welcomes(claimed_welcomes.len())?;
    for welcome in claimed_welcomes {
        store.store_pending_welcome_and_save(device, &welcome)?;
    }

    activate_pending_welcomes(store, device)?;

    for ack in device.pending_welcome_acks() {
        delivery
            .ack_welcome(&ack.welcome_id)
            .map_err(RuntimeWorkerError::Delivery)?;
        store.clear_pending_welcome_ack_and_save(device, &ack.welcome_id)?;
        report.record_welcome_ack()?;
    }

    Ok(report)
}

/// Sync the rooms hosted on one specific room server (ADR 0005). The
/// caller provides a delivery bound to that server's address; welcomes are
/// claimed there too, because a room Welcome lives on the room's server.
pub fn run_room_server_sync_tick<D: RuntimeDelivery>(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut D,
    options: &RuntimeSyncOptions,
    server_url: &str,
) -> Result<RuntimeSyncReport, RuntimeWorkerError<D::Error>> {
    let mut report = run_room_server_sync_setup_tick(store, device, delivery, options)?;

    for cursor in device.room_sync_cursors() {
        if cursor.server_url.as_deref() != Some(server_url) {
            continue;
        }
        sync_room_pages(
            store,
            device,
            delivery,
            options,
            cursor.room_id,
            cursor.after_seq,
            &mut report,
        )?;
    }

    Ok(report)
}

/// Claim and activate Welcomes from one room server without advancing any
/// existing room on it.
pub fn run_room_server_sync_setup_tick<D: RuntimeDelivery>(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut D,
    options: &RuntimeSyncOptions,
) -> Result<RuntimeSyncReport, RuntimeWorkerError<D::Error>> {
    options.validate_limits()?;
    let mut report = RuntimeSyncReport::default();

    let claimed_welcomes = delivery
        .claim_welcomes(device.device_ref())
        .map_err(RuntimeWorkerError::Delivery)?;
    report.record_claimed_welcomes(claimed_welcomes.len())?;
    for welcome in claimed_welcomes {
        store.store_pending_welcome_and_save(device, &welcome)?;
    }

    activate_pending_welcomes(store, device)?;

    for ack in device.pending_welcome_acks() {
        delivery
            .ack_welcome(&ack.welcome_id)
            .map_err(RuntimeWorkerError::Delivery)?;
        store.clear_pending_welcome_ack_and_save(device, &ack.welcome_id)?;
        report.record_welcome_ack()?;
    }

    Ok(report)
}

/// Advance exactly one joined room. The caller chooses the transport for the
/// room's pinned server. On failure this function does not save the Device or
/// any application rows for the attempted room; callers must discard the
/// in-memory Device candidate because MLS processing may have changed it.
pub fn run_room_sync_tick<D: RuntimeDelivery>(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut D,
    options: &RuntimeSyncOptions,
    room_id: &str,
) -> Result<RuntimeSyncReport, RuntimeWorkerError<D::Error>> {
    options.validate_limits()?;
    let after_seq = device.last_applied_seq(room_id)?;
    let mut report = RuntimeSyncReport::default();
    sync_room_pages(
        store,
        device,
        delivery,
        options,
        room_id.to_owned(),
        after_seq,
        &mut report,
    )?;
    Ok(report)
}

fn activate_pending_welcomes(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
) -> Result<(), ClientStoreError> {
    for welcome_id in device.pending_welcome_ids() {
        match store.activate_pending_welcome_and_save(device, &welcome_id) {
            Ok(_) => {}
            Err(error) if pending_welcome_activation_failure_is_permanent(&error) => {
                store.clear_pending_welcome_and_save(device, &welcome_id)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn pending_welcome_activation_failure_is_permanent(error: &ClientStoreError) -> bool {
    matches!(
        error,
        ClientStoreError::Client(
            ClientError::ParseWelcome
                | ClientError::StageWelcome
                | ClientError::ActivateWelcome
                | ClientError::GroupAlreadyExists(_)
        )
    )
}

pub fn run_link_fanout_tick<D: RuntimeDelivery>(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut D,
    fanout_id: &str,
    options: &RuntimeLinkFanoutOptions,
) -> Result<RuntimeLinkFanoutReport, RuntimeWorkerError<D::Error>> {
    validate_string_bytes("link_fanout.fanout_id", fanout_id, MAX_OBJECT_ID_BYTES)
        .map_err(ClientError::from)?;
    options.validate_limits()?;
    let mut report = RuntimeLinkFanoutReport::default();

    run_link_fanout_discovery(store, device, delivery, fanout_id, options, &mut report)?;
    let room_ids =
        link_fanout_rooms_to_advance(device, fanout_id, options.max_commit_rooms_per_tick)?;
    for room_id in &room_ids.pending {
        store.prepare_claimed_link_fanout_room_commit_and_save(device, fanout_id, room_id)?;
        report.record_prepared_commit()?;
    }
    for room_id in &room_ids.prepared {
        if !device.has_pending_commit(room_id)? {
            store.prepare_claimed_link_fanout_room_commit_and_save(device, fanout_id, room_id)?;
            report.record_prepared_commit()?;
        }
    }
    for room_id in room_ids.all_prepared() {
        let prepared = device.prepared_link_fanout_commit(fanout_id, &room_id)?;
        let accepted = delivery
            .submit_commit(prepared.request.clone())
            .map_err(RuntimeWorkerError::Delivery)?;
        if accepted.message_id != prepared.message_id {
            return Err(ClientError::LinkFanoutPreparedCommitMismatch {
                expected: prepared.message_id,
                actual: accepted.message_id,
            }
            .into());
        }
        report.record_submitted_commit()?;
        complete_link_fanout_room_from_sync(
            store,
            device,
            delivery,
            fanout_id,
            &room_id,
            options,
            &mut report,
        )?;
    }

    report.complete = device.link_fanout_is_complete(fanout_id)?;
    Ok(report)
}

fn run_link_fanout_discovery<D: RuntimeDelivery>(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut D,
    fanout_id: &str,
    options: &RuntimeLinkFanoutOptions,
    report: &mut RuntimeLinkFanoutReport,
) -> Result<(), RuntimeWorkerError<D::Error>> {
    let target_device = device.link_fanout_target_device(fanout_id)?;
    let mut pages = 0u32;
    while pages < options.max_discovery_pages_per_tick
        && !device.link_fanout_discovery_complete(fanout_id)?
    {
        let after_room_id = device.link_fanout_after_room_id(fanout_id)?;
        let page = delivery
            .list_account_rooms(ListAccountRoomsRequest {
                account_id: device.device_ref().account_id.clone(),
                after_room_id: after_room_id.clone(),
                limit: 1,
            })
            .map_err(RuntimeWorkerError::Delivery)?;
        if page.has_more && page.next_after_room_id == after_room_id {
            return Err(ClientError::RuntimeLinkFanoutDiscoveryStalled {
                fanout_id: fanout_id.to_string(),
                after_room_id,
            }
            .into());
        }

        let needs_target = page.rooms.iter().any(|room| {
            !room
                .devices
                .iter()
                .any(|device| device.device == target_device)
        });
        let mut claimed_key_packages = Vec::new();
        if needs_target {
            let Some(claim) = delivery
                .claim_key_package_for_device(&target_device)
                .map_err(RuntimeWorkerError::Delivery)?
            else {
                break;
            };
            claimed_key_packages.push(claim);
        }

        store.queue_claimed_link_fanout_page_and_save(
            device,
            fanout_id,
            &page,
            &claimed_key_packages,
        )?;
        report.record_discovery_page()?;
        if !claimed_key_packages.is_empty() {
            report.record_claimed_key_package()?;
            report.record_queued_room()?;
        }
        pages = pages
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
    }
    debug_assert!(pages <= options.max_discovery_pages_per_tick);
    Ok(())
}

struct LinkFanoutRoomsToAdvance {
    prepared: Vec<RoomId>,
    pending: Vec<RoomId>,
}

impl LinkFanoutRoomsToAdvance {
    fn all_prepared(self) -> Vec<RoomId> {
        let mut room_ids = Vec::with_capacity(self.prepared.len() + self.pending.len());
        room_ids.extend(self.prepared);
        room_ids.extend(self.pending);
        room_ids
    }
}

fn link_fanout_rooms_to_advance(
    device: &FiniteChatDevice,
    fanout_id: &str,
    max_rooms: u32,
) -> Result<LinkFanoutRoomsToAdvance, ClientError> {
    let mut prepared = device.prepared_link_fanout_room_ids(fanout_id)?;
    prepared.truncate(max_rooms as usize);
    let remaining = max_rooms as usize - prepared.len();
    let mut pending = device.pending_link_fanout_room_ids(fanout_id)?;
    pending.truncate(remaining);
    debug_assert!(prepared.len() + pending.len() <= max_rooms as usize);
    Ok(LinkFanoutRoomsToAdvance { prepared, pending })
}

fn complete_link_fanout_room_from_sync<D: RuntimeDelivery>(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut D,
    fanout_id: &str,
    room_id: &str,
    options: &RuntimeLinkFanoutOptions,
    report: &mut RuntimeLinkFanoutReport,
) -> Result<(), RuntimeWorkerError<D::Error>> {
    let prepared = device.prepared_link_fanout_commit(fanout_id, room_id)?;
    let mut after_seq = device.last_applied_seq(room_id)?;
    let mut pages = 0u32;
    while pages < options.max_completion_sync_pages_per_room {
        let page = delivery
            .sync_events(room_id, device.device_ref(), after_seq)
            .map_err(RuntimeWorkerError::Delivery)?;
        report.record_completion_sync_page()?;
        pages = pages
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        if page.next_after_seq < after_seq {
            return Err(ClientError::RuntimeSyncCursorRegression {
                room_id: room_id.to_string(),
                current_seq: after_seq,
                next_after_seq: page.next_after_seq,
            }
            .into());
        }

        let mut dirty = false;
        let mut app_messages = Vec::new();
        let mut app_events = Vec::new();
        for entry in page.entries {
            let seq = entry.seq;
            if entry.message_id == prepared.message_id {
                // The completion path persists the whole device state,
                // including any entries applied in memory above.
                let message_id = entry.message_id.clone();
                if let Some(applied) = store.complete_link_fanout_room_from_log_and_save(
                    device, fanout_id, room_id, &entry,
                )? {
                    report.applied_entries.push(RuntimeAppliedEntry {
                        room_id: room_id.to_string(),
                        seq,
                        message_id,
                        timestamp_unix_seconds: entry.timestamp_unix_seconds,
                        entry: applied,
                    });
                    report.record_completed_room()?;
                }
                if !app_messages.is_empty() || !app_events.is_empty() {
                    store.save_app_messages_and_events(
                        device.device_ref(),
                        &app_messages,
                        &app_events,
                        MAX_STORED_APP_MESSAGES,
                    )?;
                }
                return Ok(());
            }
            let before_seq = device.last_applied_seq(room_id)?;
            if let Some(applied) = apply_log_entry_in_memory(device, room_id, &entry)? {
                dirty = true;
                if let Some(message) = stored_app_message_from_applied(
                    room_id,
                    seq,
                    &entry.message_id,
                    entry.timestamp_unix_seconds,
                    &applied,
                ) {
                    app_messages.push(message);
                }
                if let Some(event) = stored_app_event_from_applied(
                    room_id,
                    seq,
                    &entry.message_id,
                    entry.timestamp_unix_seconds,
                    &applied,
                ) {
                    app_events.push(event);
                }
                report.applied_entries.push(RuntimeAppliedEntry {
                    room_id: room_id.to_string(),
                    seq,
                    message_id: entry.message_id.clone(),
                    timestamp_unix_seconds: entry.timestamp_unix_seconds,
                    entry: applied,
                });
            } else if device.last_applied_seq(room_id)? > before_seq {
                dirty = true;
            }
        }

        if page.next_after_seq > device.last_applied_seq(room_id)? {
            device
                .set_last_applied_seq(room_id, page.next_after_seq)
                .map_err(ClientStoreError::from)?;
            dirty = true;
        }
        if dirty {
            store.save_device_state_and_app_messages_and_events(
                device,
                &app_messages,
                &app_events,
            )?;
        }
        if !page.has_more {
            break;
        }
        if page.next_after_seq == after_seq {
            return Err(ClientError::RuntimeSyncStalled {
                room_id: room_id.to_string(),
                after_seq,
            }
            .into());
        }
        after_seq = page.next_after_seq;
    }
    debug_assert!(pages <= options.max_completion_sync_pages_per_room);
    Ok(())
}

fn sync_room_pages<D: RuntimeDelivery>(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut D,
    options: &RuntimeSyncOptions,
    room_id: RoomId,
    mut after_seq: u64,
    report: &mut RuntimeSyncReport,
) -> Result<(), RuntimeWorkerError<D::Error>> {
    if device.has_pending_commit(&room_id)? {
        after_seq = after_seq.saturating_sub(PENDING_COMMIT_SYNC_OVERLAP);
    }
    let mut pages = 0u32;
    let mut dirty = false;
    let mut app_messages = Vec::new();
    let mut app_events = Vec::new();
    while pages < options.max_sync_pages_per_room {
        let page = delivery
            .sync_events(&room_id, device.device_ref(), after_seq)
            .map_err(RuntimeWorkerError::Delivery)?;
        report.record_sync_page()?;
        pages = pages
            .checked_add(1)
            .ok_or(ClientError::RuntimeCounterOverflow)?;
        if page.next_after_seq < after_seq {
            return Err(ClientError::RuntimeSyncCursorRegression {
                room_id,
                current_seq: after_seq,
                next_after_seq: page.next_after_seq,
            }
            .into());
        }

        for entry in page.entries {
            let seq = entry.seq;
            let message_id = entry.message_id.clone();
            let before_seq = device.last_applied_seq(&room_id)?;
            if let Some(applied) = apply_log_entry_in_memory(device, &room_id, &entry)? {
                dirty = true;
                if let Some(message) = stored_app_message_from_applied(
                    &room_id,
                    seq,
                    &message_id,
                    entry.timestamp_unix_seconds,
                    &applied,
                ) {
                    app_messages.push(message);
                }
                if let Some(event) = stored_app_event_from_applied(
                    &room_id,
                    seq,
                    &message_id,
                    entry.timestamp_unix_seconds,
                    &applied,
                ) {
                    app_events.push(event);
                }
                report.applied_entries.push(RuntimeAppliedEntry {
                    room_id: room_id.clone(),
                    seq,
                    message_id,
                    timestamp_unix_seconds: entry.timestamp_unix_seconds,
                    entry: applied,
                });
            } else if device.last_applied_seq(&room_id)? > before_seq {
                dirty = true;
            }
        }

        if page.next_after_seq > device.last_applied_seq(&room_id)? {
            device
                .set_last_applied_seq(&room_id, page.next_after_seq)
                .map_err(ClientStoreError::from)?;
            dirty = true;
        }
        if !page.has_more {
            break;
        }
        if page.next_after_seq == after_seq {
            return Err(ClientError::RuntimeSyncStalled { room_id, after_seq }.into());
        }
        after_seq = page.next_after_seq;
    }

    if dirty {
        store.save_device_state_and_app_messages_and_events(device, &app_messages, &app_events)?;
    }

    debug_assert!(pages <= options.max_sync_pages_per_room);
    Ok(())
}

#[derive(Debug)]
pub enum RuntimeWorkerError<E> {
    Delivery(E),
    Client(ClientError),
    ClientStore(ClientStoreError),
}

impl<E> From<ClientError> for RuntimeWorkerError<E> {
    fn from(error: ClientError) -> Self {
        Self::Client(error)
    }
}

impl<E> From<ClientStoreError> for RuntimeWorkerError<E> {
    fn from(error: ClientStoreError) -> Self {
        Self::ClientStore(error)
    }
}

impl<E: std::fmt::Display> std::fmt::Display for RuntimeWorkerError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Delivery(error) => write!(formatter, "delivery operation failed: {error}"),
            Self::Client(error) => write!(formatter, "{error}"),
            Self::ClientStore(error) => write!(formatter, "{error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for RuntimeWorkerError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Delivery(error) => Some(error),
            Self::Client(error) => Some(error),
            Self::ClientStore(error) => Some(error),
        }
    }
}

#[derive(Debug, Error)]
pub enum ClientStoreError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("failed to create sqlite client store directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("client state not found for {account_id}/{device_id}")]
    DeviceStateNotFound {
        account_id: String,
        device_id: String,
    },
    #[error("legacy unencrypted client store table {table:?} is reset-only; reset the store")]
    LegacyUnencryptedStoreTable { table: LegacyClientStoreTable },
    #[error("unsupported pre-release app projection table {table}: {reason}; reset the store")]
    LegacyAppProjectionSchema { table: String, reason: String },
    #[error("failed to generate encrypted client store nonce")]
    Randomness,
    #[error("failed to encrypt client state")]
    EncryptState,
    #[error("failed to decrypt client state")]
    DecryptState,
    #[error("encrypted client state nonce has {actual_bytes} bytes")]
    InvalidNonceLength { actual_bytes: usize },
    #[error("failed to encrypt stored app message")]
    EncryptAppMessage,
    #[error("failed to decrypt stored app message")]
    DecryptAppMessage,
    #[error("stored app message nonce has {actual_bytes} bytes")]
    InvalidAppMessageNonceLength { actual_bytes: usize },
    #[error("encrypted client state snapshot has malformed magic")]
    StateSnapshotMagic,
    #[error("encrypted client state snapshot version {0} is not supported")]
    StateSnapshotVersion(u16),
    #[error("encrypted client state snapshot is truncated")]
    StateSnapshotTruncated,
    #[error("encrypted client state snapshot has trailing bytes")]
    StateSnapshotTrailingBytes,
    #[error("encrypted client state snapshot has invalid UTF-8")]
    StateSnapshotUtf8,
    #[error("encrypted client state snapshot has invalid device-link bootstrap export")]
    DecodeLinkBootstrapExport,
    #[error("encrypted client state snapshot enum {field} has unknown value {value}")]
    StateSnapshotEnum { field: &'static str, value: u64 },
    #[error("encrypted client state snapshot length overflow")]
    StateSnapshotLengthOverflow,
    #[error("encrypted client state snapshot identity does not match lookup")]
    StateSnapshotIdentityMismatch,
    #[error("app message limit {limit} is outside 1..={max}")]
    InvalidAppMessageLimit { limit: u32, max: u32 },
    #[error("stored app message seq {seq} cannot be represented in sqlite")]
    StoredAppMessageSeqOutOfRange { seq: u64 },
    #[error("stored app message seq is negative: {seq}")]
    NegativeStoredAppMessageSeq { seq: i64 },
    #[error("stored app message count cannot be represented in sqlite")]
    StoredAppMessageCountOverflow,
    #[error("stored app event seq {seq} cannot be represented in sqlite")]
    StoredAppEventSeqOutOfRange { seq: u64 },
    #[error("stored app event seq is negative: {seq}")]
    NegativeStoredAppEventSeq { seq: i64 },
    #[error("stored app event count cannot be represented in sqlite")]
    StoredAppEventCountOverflow,
    #[error("stored app event {room_id}/{message_id} conflicts with canonical local history")]
    StoredAppEventConflict { room_id: String, message_id: String },
    #[error("stored app timestamp {timestamp} cannot be represented in sqlite")]
    StoredAppTimestampOutOfRange { timestamp: u64 },
    #[error("stored app timestamp is negative: {timestamp}")]
    NegativeStoredAppTimestamp { timestamp: i64 },
    #[error("encrypted app outbox nonce has {actual_bytes} bytes")]
    InvalidAppOutboxNonceLength { actual_bytes: usize },
    #[error("failed to encrypt app outbox metadata")]
    EncryptAppOutbox,
    #[error("failed to decrypt app outbox metadata")]
    DecryptAppOutbox,
    #[error("failed to encode app outbox metadata")]
    EncodeAppOutboxMetadata,
    #[error("failed to decode app outbox metadata")]
    DecodeAppOutboxMetadata,
    #[error("stored app outbox count cannot be represented in sqlite")]
    StoredAppOutboxCountOverflow,
    #[error("failed to encrypt stored app event")]
    EncryptAppEvent,
    #[error("failed to decrypt stored app event")]
    DecryptAppEvent,
    #[error("stored app event nonce has {actual_bytes} bytes")]
    InvalidAppEventNonceLength { actual_bytes: usize },
    #[error("encrypted app room nonce has {actual_bytes} bytes")]
    InvalidAppRoomNonceLength { actual_bytes: usize },
    #[error("failed to encrypt app room metadata")]
    EncryptAppRoom,
    #[error("failed to decrypt app room metadata")]
    DecryptAppRoom,
    #[error("failed to encode app room metadata")]
    EncodeAppRoomMetadata,
    #[error("failed to decode app room metadata")]
    DecodeAppRoomMetadata,
    #[error("encrypted app state nonce has {actual_bytes} bytes")]
    InvalidAppStateNonceLength { actual_bytes: usize },
    #[error("failed to encrypt app state metadata")]
    EncryptAppState,
    #[error("failed to decrypt app state metadata")]
    DecryptAppState,
    #[error("failed to encode app state metadata")]
    EncodeAppStateMetadata,
    #[error("failed to decode app state metadata")]
    DecodeAppStateMetadata,
    #[error("encrypted app profile nonce has {actual_bytes} bytes")]
    InvalidAppProfileNonceLength { actual_bytes: usize },
    #[error("failed to encrypt app profile metadata")]
    EncryptAppProfile,
    #[error("failed to decrypt app profile metadata")]
    DecryptAppProfile,
    #[error("failed to encode app profile metadata")]
    EncodeAppProfileMetadata,
    #[error("failed to decode app profile metadata")]
    DecodeAppProfileMetadata,
    #[error("stored app profile count cannot be represented in sqlite")]
    StoredAppProfileCountOverflow,
    #[error("failed to encode device-link bootstrap manifest")]
    EncodeDeviceLinkBootstrapManifest,
    #[error("failed to decode device-link bootstrap manifest")]
    DecodeDeviceLinkBootstrapManifest,
    #[error("failed to encrypt device-link bootstrap manifest")]
    EncryptDeviceLinkBootstrapManifest,
    #[error("failed to decrypt device-link bootstrap manifest")]
    DecryptDeviceLinkBootstrapManifest,
    #[error("device-link bootstrap manifest nonce has {actual_bytes} bytes")]
    InvalidDeviceLinkBootstrapManifestNonceLength { actual_bytes: usize },
    #[error("failed to encode device-link bootstrap chunk")]
    EncodeDeviceLinkBootstrapChunk,
    #[error("failed to decode device-link bootstrap chunk")]
    DecodeDeviceLinkBootstrapChunk,
    #[error("failed to encrypt device-link bootstrap chunk")]
    EncryptDeviceLinkBootstrapChunk,
    #[error("failed to decrypt device-link bootstrap chunk")]
    DecryptDeviceLinkBootstrapChunk,
    #[error("device-link bootstrap chunk nonce has {actual_bytes} bytes")]
    InvalidDeviceLinkBootstrapChunkNonceLength { actual_bytes: usize },
    #[error("device-link bootstrap state has unknown value {state}")]
    InvalidDeviceLinkBootstrapState { state: i64 },
    #[error("device-link bootstrap room mismatch: expected {expected}, actual {actual}")]
    DeviceLinkBootstrapRoomMismatch { expected: String, actual: String },
    #[error("device-link bootstrap receipt does not match its encrypted manifest")]
    DeviceLinkBootstrapReceiptMismatch,
    #[error("device-link bootstrap chunk index {index} cannot be represented in sqlite")]
    DeviceLinkBootstrapChunkIndexOutOfRange { index: u32 },
    #[error("device-link bootstrap chunk index is negative: {index}")]
    NegativeDeviceLinkBootstrapChunkIndex { index: i64 },
    #[error("device-link bootstrap chunk count cannot be represented in sqlite")]
    DeviceLinkBootstrapChunkCountOverflow,
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    MlsCredential(#[from] MlsCredentialError),
    #[error(transparent)]
    ProtocolLimit(#[from] ProtocolLimitError),
    #[error("engine rejected request: {0}")]
    Engine(#[from] EngineError),
    #[error("failed to derive envelope message id")]
    EnvelopeMessageId(#[source] serde_json::Error),
    #[error("invalid persisted client state: {0}")]
    InvalidClientState(String),
    #[error("stored outbox request room mismatch: expected {expected}, actual {actual}")]
    OutboxRoomMismatch { expected: String, actual: String },
    #[error("stored outbox request message id mismatch: expected {expected}, actual {actual}")]
    OutboxMessageIdMismatch { expected: String, actual: String },
    #[error("stored outbox request sender mismatch: expected {expected:?}, actual {actual:?}")]
    OutboxSenderMismatch {
        expected: DeviceRef,
        actual: DeviceRef,
    },
    #[error("failed to create OpenMLS signer")]
    CreateSigner,
    #[error("failed to store OpenMLS signer")]
    StoreSigner,
    #[error("failed to create OpenMLS group")]
    CreateGroup,
    #[error("failed to build OpenMLS KeyPackage")]
    BuildKeyPackage,
    #[error("failed to draw randomness from the crypto provider")]
    Randomness,
    #[error("failed to seal or open an ephemeral activity payload")]
    ActivityCiphertext,
    #[error("activity payload epoch {payload_epoch} does not match group epoch {group_epoch}")]
    ActivityEpochMismatch {
        payload_epoch: u64,
        group_epoch: u64,
    },
    #[error("room {room_id} has no verified member for account {account_id}")]
    AccountNotInRoom { room_id: String, account_id: String },
    #[error("failed to serialize OpenMLS KeyPackage")]
    SerializeKeyPackage,
    #[error("failed to parse OpenMLS KeyPackage")]
    ParseKeyPackage,
    #[error("claimed KeyPackage ref does not match payload")]
    KeyPackageRefMismatch,
    #[error("claimed KeyPackage hash does not match payload")]
    KeyPackageHashMismatch,
    #[error("KeyPackage inventory owner mismatch: expected {expected:?}, actual {actual:?}")]
    KeyPackageInventoryOwnerMismatch {
        expected: DeviceRef,
        actual: DeviceRef,
    },
    #[error(
        "KeyPackage inventory exceeds cap: {available} available and {leased} leased, max {max}"
    )]
    KeyPackageInventoryOverCap {
        available: u32,
        leased: u32,
        max: u32,
    },
    #[error(
        "KeyPackage replenishment exceeds cap: {available} available, {leased} leased, {pending} pending uploads, max {max}"
    )]
    KeyPackagePendingUploadOverCap {
        available: u32,
        leased: u32,
        pending: u32,
        max: u32,
    },
    #[error("failed to hash OpenMLS KeyPackage ref")]
    HashKeyPackageRef,
    #[error("failed to add OpenMLS member")]
    AddMember,
    #[error("failed to remove OpenMLS member")]
    RemoveMember,
    #[error("failed to create OpenMLS self-update commit")]
    SelfUpdate,
    #[error("remove-member commit cannot remove the sender")]
    CannotRemoveSelf,
    #[error("non-add commit unexpectedly produced a Welcome")]
    UnexpectedWelcomeForNonAddCommit,
    #[error("failed to serialize OpenMLS message")]
    SerializeMessage,
    #[error("failed to export pending ratchet tree")]
    ExportPendingRatchetTree,
    #[error("failed to serialize ratchet tree")]
    SerializeRatchetTree,
    #[error("failed to parse ratchet tree")]
    ParseRatchetTree,
    #[error("failed to parse Welcome")]
    ParseWelcome,
    #[error("failed to stage Welcome")]
    StageWelcome,
    #[error("failed to activate Welcome")]
    ActivateWelcome,
    #[error("failed to merge pending commit")]
    MergePendingCommit,
    #[error("failed to clear losing pending commit")]
    ClearPendingCommit,
    #[error("failed to merge staged remote commit")]
    MergeStagedCommit,
    #[error("failed to create application message")]
    CreateApplicationMessage,
    #[error("failed to parse protocol message")]
    ParseProtocolMessage,
    #[error("failed to process MLS message: {reason}")]
    ProcessMessage { reason: String },
    #[error("unexpected MLS message content")]
    UnexpectedMessage,
    #[error("group already exists: {0}")]
    GroupAlreadyExists(RoomId),
    #[error("group not found: {0}")]
    GroupNotFound(RoomId),
    #[error("pending commit already exists for group: {0}")]
    PendingCommitExists(RoomId),
    #[error("pending commit must be merged before sending application data: {0}")]
    PendingCommitMustBeMerged(RoomId),
    #[error("pending commit is missing for group: {0}")]
    MissingPendingCommit(RoomId),
    #[error("pending commit was not observed in the ordered server log: {0}")]
    PendingCommitNotObserved(String),
    #[error("Welcome batch must contain at least one KeyPackage")]
    EmptyWelcomeBatch,
    #[error("Welcome batch has {key_packages} KeyPackages but {welcome_ids} Welcome ids")]
    WelcomeBatchCountMismatch {
        key_packages: usize,
        welcome_ids: usize,
    },
    #[error("Welcome batch contains duplicate device: {0:?}")]
    DuplicateWelcomeBatchDevice(DeviceRef),
    #[error("Welcome batch contains duplicate KeyPackage: {0}")]
    DuplicateWelcomeBatchKeyPackage(KeyPackageId),
    #[error("Welcome batch contains duplicate Welcome id: {0}")]
    DuplicateWelcomeBatchWelcome(WelcomeId),
    #[error("pending Welcome already exists: {0}")]
    DuplicatePendingWelcome(WelcomeId),
    #[error("pending Welcome ack already exists: {0}")]
    DuplicatePendingWelcomeAck(WelcomeId),
    #[error("pending Welcome cannot also need ack: {0}")]
    PendingWelcomeAlsoNeedsAck(WelcomeId),
    #[error("pending Welcome ack room is missing: {0}")]
    PendingWelcomeAckRoomMissing(RoomId),
    #[error("pending Welcome not found: {0}")]
    PendingWelcomeNotFound(WelcomeId),
    #[error("pending Welcome ack not found: {0}")]
    PendingWelcomeAckNotFound(WelcomeId),
    #[error("pending Welcome recipient does not match this device")]
    PendingWelcomeRecipientMismatch,
    #[error("pending KeyPackage upload already exists: {0}")]
    DuplicatePendingKeyPackageUpload(KeyPackageId),
    #[error("pending KeyPackage upload not found: {0}")]
    PendingKeyPackageUploadNotFound(KeyPackageId),
    #[error("pending KeyPackage upload owner mismatch: expected {expected:?}, actual {actual:?}")]
    PendingKeyPackageUploadOwnerMismatch {
        expected: DeviceRef,
        actual: DeviceRef,
    },
    #[error("prepared commit message id does not match request")]
    PreparedCommitMessageIdMismatch,
    #[error("profile picture URL must be http(s): {0}")]
    ProfilePictureUrl(String),
    #[error("room picture URL must be http(s): {0}")]
    RoomPictureUrl(String),
    #[error(
        "profile expires_at_ms {expires_at_ms} must be greater than fetched_at_ms {fetched_at_ms}"
    )]
    ProfileExpiry {
        fetched_at_ms: u64,
        expires_at_ms: u64,
    },
    #[error("link fanout already exists: {0}")]
    DuplicateLinkFanout(String),
    #[error("link fanout not found: {0}")]
    LinkFanoutNotFound(String),
    #[error("link fanout room already exists: {0}")]
    DuplicateLinkFanoutRoom(RoomId),
    #[error("link fanout room not found: {0}")]
    LinkFanoutRoomNotFound(RoomId),
    #[error("missing link fanout room plan: {0}")]
    MissingLinkFanoutRoomPlan(RoomId),
    #[error("unexpected link fanout room plan: {0}")]
    UnexpectedLinkFanoutRoomPlan(RoomId),
    #[error("missing link fanout claimed KeyPackage for room: {0}")]
    MissingLinkFanoutClaim(RoomId),
    #[error("unexpected link fanout claimed KeyPackage: {0}")]
    UnexpectedLinkFanoutClaim(KeyPackageId),
    #[error("link fanout target account mismatch: expected {expected}, actual {actual}")]
    LinkFanoutAccountMismatch { expected: String, actual: String },
    #[error("link fanout cannot target the current device")]
    LinkFanoutCannotTargetSelf,
    #[error("link fanout room is not pending: {0}")]
    LinkFanoutRoomNotPending(RoomId),
    #[error("link fanout room is not prepared: {0}")]
    LinkFanoutRoomNotPrepared(RoomId),
    #[error(
        "link fanout claimed KeyPackage owner mismatch: expected {expected:?}, actual {actual:?}"
    )]
    LinkFanoutClaimTargetMismatch {
        expected: DeviceRef,
        actual: DeviceRef,
    },
    #[error("link fanout claimed KeyPackage id mismatch: expected {expected}, actual {actual}")]
    LinkFanoutClaimPackageMismatch {
        expected: KeyPackageId,
        actual: KeyPackageId,
    },
    #[error("link fanout prepared Commit mismatch: expected {expected}, actual {actual}")]
    LinkFanoutPreparedCommitMismatch { expected: String, actual: String },
    #[error("member credential missing or duplicated: {0:?}")]
    MemberCredentialMissing(DeviceRef),
    #[error("persisted client state account does not match config")]
    PersistedAccountMismatch,
    #[error("persisted client state device does not match config")]
    PersistedDeviceMismatch,
    #[error("persisted room has duplicate room id: {0}")]
    DuplicatePersistedRoom(RoomId),
    #[error("persisted room {0} has mismatched MLS group id")]
    PersistedGroupIdMismatch(RoomId),
    #[error("persisted OpenMLS storage is empty")]
    MissingOpenMlsStorage,
    #[error("persisted OpenMLS storage has duplicate key")]
    DuplicateOpenMlsStorageKey,
    #[error("failed to lock OpenMLS storage")]
    OpenMlsStorageLock,
    #[error("persisted signer is missing")]
    MissingStoredSigner,
    #[error("persisted signer does not match credential leaf key")]
    StoredSignerMismatch,
    #[error("failed to load persisted MLS group state: {0}")]
    LoadGroupState(RoomId),
    #[error("persisted MLS group state is missing: {0}")]
    MissingGroupState(RoomId),
    #[error("runtime worker counter overflow")]
    RuntimeCounterOverflow,
    #[error("runtime sync cursor regressed for {room_id}: {current_seq} -> {next_after_seq}")]
    RuntimeSyncCursorRegression {
        room_id: RoomId,
        current_seq: u64,
        next_after_seq: u64,
    },
    #[error("runtime sync stalled for {room_id} after seq {after_seq}")]
    RuntimeSyncStalled { room_id: RoomId, after_seq: u64 },
    #[error("runtime link fanout discovery stalled for {fanout_id} after room {after_room_id:?}")]
    RuntimeLinkFanoutDiscoveryStalled {
        fanout_id: String,
        after_room_id: Option<RoomId>,
    },
    #[error("log entry room mismatch: expected {expected}, actual {actual}")]
    LogEntryRoomMismatch { expected: RoomId, actual: RoomId },
    #[error("log entry envelope room mismatch: entry {entry_room}, envelope {envelope_room}")]
    LogEntryEnvelopeRoomMismatch {
        entry_room: RoomId,
        envelope_room: RoomId,
    },
    #[error("log entry kind mismatch: expected {expected:?}, actual {actual:?}")]
    LogEntryKindMismatch {
        expected: LogEntryKind,
        actual: LogEntryKind,
    },
    #[error("log entry envelope kind mismatch: entry {entry_kind:?}, envelope {envelope_kind:?}")]
    LogEntryEnvelopeKindMismatch {
        entry_kind: LogEntryKind,
        envelope_kind: LogEntryKind,
    },
    #[error(
        "log entry message id does not match envelope: entry {entry_message_id}, envelope {envelope_message_id}"
    )]
    LogEntryMessageIdMismatch {
        entry_message_id: String,
        envelope_message_id: String,
    },
    #[error("log entry sender does not match envelope")]
    LogEntrySenderMismatch,
    #[error("log entry epoch {entry_epoch} does not match envelope epoch {envelope_epoch}")]
    LogEntryEpochMismatch {
        entry_epoch: u64,
        envelope_epoch: u64,
    },
    #[error("unsupported log entry kind: {0:?}")]
    UnsupportedLogEntryKind(LogEntryKind),
    #[error("commit epoch mismatch for {room_id}: local {current_epoch}, entry {entry_epoch}")]
    UnexpectedCommitEpoch {
        room_id: RoomId,
        current_epoch: u64,
        entry_epoch: u64,
    },
    #[error("post-commit epoch overflow")]
    EpochOverflow,
    #[error(
        "post-commit epoch mismatch for {room_id}: expected {expected_epoch}, actual {actual_epoch}"
    )]
    UnexpectedPostCommitEpoch {
        room_id: RoomId,
        expected_epoch: u64,
        actual_epoch: u64,
    },
    #[error("own commit has no pending local state: {0}")]
    OwnCommitWithoutPendingState(String),
    #[error("account id is not a 32-byte lowercase hex Nostr public key: {0}")]
    MalformedAccountId(String),
    #[error("MLS group id is not valid UTF-8")]
    MlsGroupIdNotUtf8,
    #[error(
        "applied seq regression for {room_id}: current {current_seq}, attempted {attempted_seq}"
    )]
    AppliedSeqRegression {
        room_id: RoomId,
        current_seq: u64,
        attempted_seq: u64,
    },
}

fn verified_key_package_from_claim(
    provider: &OpenMlsRustCrypto,
    claimed_key_package: &ClaimKeyPackageResult,
    now_unix_seconds: u64,
) -> Result<KeyPackage, ClientError> {
    validate_claimed_key_package(claimed_key_package)?;
    let key_package_in =
        KeyPackageIn::tls_deserialize_exact(claimed_key_package.key_package_payload.as_slice())
            .map_err(|_| ClientError::ParseKeyPackage)?;
    let key_package = key_package_in
        .validate(provider.crypto(), ProtocolVersion::Mls10)
        .map_err(|_| ClientError::ParseKeyPackage)?;
    let key_package_ref = key_package
        .hash_ref(provider.crypto())
        .map_err(|_| ClientError::HashKeyPackageRef)?;
    if hex_lower(key_package_ref.as_slice()) != claimed_key_package.key_package_ref {
        return Err(ClientError::KeyPackageRefMismatch);
    }
    if message_id_for_bytes(&claimed_key_package.key_package_payload)
        != claimed_key_package.key_package_hash
    {
        return Err(ClientError::KeyPackageHashMismatch);
    }

    let credential =
        FiniteDeviceCredentialV1::from_credential(key_package.leaf_node().credential().clone())?;
    credential.verify_expected(ExpectedDeviceCredential {
        account_public_key: account_public_key_from_device_ref(&claimed_key_package.owner)?,
        device_id: &claimed_key_package.owner.device_id,
        mls_leaf_signing_public_key: key_package.leaf_node().signature_key().as_slice(),
        now_unix_seconds,
    })?;
    Ok(key_package)
}

fn validate_claimed_key_package(
    claimed_key_package: &ClaimKeyPackageResult,
) -> Result<(), ClientError> {
    claimed_key_package
        .owner
        .validate_limits()
        .map_err(ClientError::from)?;
    validate_string_bytes(
        "claimed_key_package.key_package_id",
        &claimed_key_package.key_package_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    validate_string_bytes(
        "claimed_key_package.key_package_ref",
        &claimed_key_package.key_package_ref,
        MAX_OBJECT_ID_BYTES,
    )?;
    validate_string_bytes(
        "claimed_key_package.key_package_hash",
        &claimed_key_package.key_package_hash,
        MAX_OBJECT_ID_BYTES,
    )?;
    validate_bytes_non_empty(
        "claimed_key_package.key_package_payload",
        claimed_key_package.key_package_payload.len(),
    )?;
    validate_bytes_len(
        "claimed_key_package.key_package_payload",
        claimed_key_package.key_package_payload.len(),
        MAX_KEY_PACKAGE_PAYLOAD_BYTES,
    )?;
    validate_string_bytes(
        "claimed_key_package.lease_token",
        &claimed_key_package.lease_token,
        MAX_OBJECT_ID_BYTES,
    )?;
    Ok(())
}

fn link_fanout_derived_id(
    prefix: &str,
    fanout_id: &str,
    room_id: &str,
    key_package_id: &str,
) -> Result<String, ClientError> {
    validate_string_bytes("link_fanout.id_prefix", prefix, MAX_OBJECT_ID_BYTES)?;
    validate_string_bytes("link_fanout.fanout_id", fanout_id, MAX_OBJECT_ID_BYTES)?;
    validate_room_id(room_id)?;
    validate_string_bytes(
        "link_fanout.key_package_id",
        key_package_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    let mut seed = Vec::with_capacity(
        prefix.len() + fanout_id.len() + room_id.len() + key_package_id.len() + 4,
    );
    seed.extend_from_slice(prefix.as_bytes());
    seed.push(0);
    seed.extend_from_slice(fanout_id.as_bytes());
    seed.push(0);
    seed.extend_from_slice(room_id.as_bytes());
    seed.push(0);
    seed.extend_from_slice(key_package_id.as_bytes());
    let value = format!("{prefix}_{}", message_id_for_bytes(&seed));
    validate_string_bytes("link_fanout.derived_id", &value, MAX_OBJECT_ID_BYTES)?;
    debug_assert!(value.starts_with(prefix));
    Ok(value)
}

fn verify_staged_commit_credentials(
    now_unix_seconds: u64,
    staged_commit: &StagedCommit,
) -> Result<(), ClientError> {
    for credential in staged_commit.credentials_to_verify() {
        let credential = FiniteDeviceCredentialV1::from_credential(credential.clone())?;
        credential.verify_expected(ExpectedDeviceCredential {
            account_public_key: credential.account_public_key(),
            device_id: credential.device_id(),
            mls_leaf_signing_public_key: credential.mls_leaf_signing_public_key(),
            now_unix_seconds,
        })?;
    }
    Ok(())
}

fn verified_member_leaf_index(
    group: &MlsGroup,
    device: &DeviceRef,
    now_unix_seconds: u64,
) -> Result<LeafNodeIndex, ClientError> {
    let expected_account_public_key = account_public_key_from_device_ref(device)?;
    let mut matched_index = None;
    for member in group.members() {
        let credential = FiniteDeviceCredentialV1::from_credential(member.credential)?;
        if credential.account_public_key() != expected_account_public_key
            || credential.device_id() != device.device_id
        {
            continue;
        }
        credential.verify_expected(ExpectedDeviceCredential {
            account_public_key: expected_account_public_key,
            device_id: &device.device_id,
            mls_leaf_signing_public_key: &member.signature_key,
            now_unix_seconds,
        })?;
        if matched_index.replace(member.index).is_some() {
            return Err(ClientError::MemberCredentialMissing(device.clone()));
        }
    }
    matched_index.ok_or_else(|| ClientError::MemberCredentialMissing(device.clone()))
}

fn openmls_group_config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .ciphersuite(FINITECHAT_CIPHERSUITE)
        .use_ratchet_tree_extension(false)
        .build()
}

fn welcome_from_bytes(bytes: &[u8]) -> Result<Welcome, ClientError> {
    let message = mls_message_in_from_bytes(bytes)?;
    let MlsMessageBodyIn::Welcome(welcome) = message.extract() else {
        return Err(ClientError::ParseWelcome);
    };
    Ok(welcome)
}

fn ratchet_tree_from_bytes(bytes: &[u8]) -> Result<RatchetTreeIn, ClientError> {
    if bytes.is_empty() {
        return Err(ClientError::ParseRatchetTree);
    }
    RatchetTreeIn::tls_deserialize_exact(bytes).map_err(|_| ClientError::ParseRatchetTree)
}

fn protocol_message_from_bytes(bytes: &[u8]) -> Result<ProtocolMessage, ClientError> {
    mls_message_in_from_bytes(bytes)?
        .try_into_protocol_message()
        .map_err(|_| ClientError::ParseProtocolMessage)
}

fn validate_log_entry_shape(
    room_id: &str,
    entry: &RoomLogEntry,
    expected_kind: LogEntryKind,
) -> Result<(), ClientError> {
    validate_room_id(room_id)?;
    if entry.room_id != room_id {
        return Err(ClientError::LogEntryRoomMismatch {
            expected: room_id.to_string(),
            actual: entry.room_id.clone(),
        });
    }
    if entry.envelope.room_id != entry.room_id {
        return Err(ClientError::LogEntryEnvelopeRoomMismatch {
            entry_room: entry.room_id.clone(),
            envelope_room: entry.envelope.room_id.clone(),
        });
    }
    if entry.kind != expected_kind {
        return Err(ClientError::LogEntryKindMismatch {
            expected: expected_kind,
            actual: entry.kind,
        });
    }
    if entry.envelope.kind != entry.kind {
        return Err(ClientError::LogEntryEnvelopeKindMismatch {
            entry_kind: entry.kind,
            envelope_kind: entry.envelope.kind,
        });
    }
    let envelope_message_id = entry
        .envelope
        .message_id()
        .map_err(ClientError::EnvelopeMessageId)?;
    if entry.message_id != envelope_message_id {
        return Err(ClientError::LogEntryMessageIdMismatch {
            entry_message_id: entry.message_id.clone(),
            envelope_message_id,
        });
    }
    if entry.sender != entry.envelope.sender {
        return Err(ClientError::LogEntrySenderMismatch);
    }
    if entry.epoch != entry.envelope.epoch {
        return Err(ClientError::LogEntryEpochMismatch {
            entry_epoch: entry.epoch,
            envelope_epoch: entry.envelope.epoch,
        });
    }
    debug_assert_eq!(entry.room_id, room_id);
    debug_assert_eq!(entry.kind, expected_kind);
    Ok(())
}

fn post_commit_epoch(epoch: u64) -> Result<u64, ClientError> {
    epoch.checked_add(1).ok_or(ClientError::EpochOverflow)
}

fn mls_message_out_bytes(message: MlsMessageOut) -> Result<Vec<u8>, ClientError> {
    let bytes = message
        .to_bytes()
        .map_err(|_| ClientError::SerializeMessage)?;
    debug_assert!(!bytes.is_empty());
    Ok(bytes)
}

fn mls_message_in_from_bytes(mut bytes: &[u8]) -> Result<MlsMessageIn, ClientError> {
    if bytes.is_empty() {
        return Err(ClientError::ParseProtocolMessage);
    }
    MlsMessageIn::tls_deserialize(&mut bytes).map_err(|_| ClientError::ParseProtocolMessage)
}

/// Generate a fresh account secret key from the crypto provider's RNG.
/// Used by agent onboarding (`hermes init`): each agent gets its own Nostr
/// identity that never needs a public relay (ADR 0006 §5).
pub fn generate_account_secret() -> Result<NostrSecretKey, ClientError> {
    let provider = OpenMlsRustCrypto::default();
    for _ in 0..8 {
        let bytes: [u8; NOSTR_SECRET_KEY_BYTES] = provider
            .rand()
            .random_array()
            .map_err(|_| ClientError::Randomness)?;
        if let Ok(secret) = NostrSecretKey::from_bytes(bytes) {
            return Ok(secret);
        }
    }
    Err(ClientError::Randomness)
}

fn account_public_key_from_device_ref(device: &DeviceRef) -> Result<NostrPublicKey, ClientError> {
    let bytes = decode_lower_hex_32(&device.account_id)?;
    NostrPublicKey::from_bytes(bytes).map_err(ClientError::from)
}

fn decode_lower_hex_32(value: &str) -> Result<[u8; NOSTR_PUBLIC_KEY_BYTES], ClientError> {
    if value.len() != NOSTR_PUBLIC_KEY_BYTES * 2 {
        return Err(ClientError::MalformedAccountId(value.to_string()));
    }
    let mut bytes = [0u8; NOSTR_PUBLIC_KEY_BYTES];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = decode_lower_hex_nibble(chunk[0])
            .ok_or_else(|| ClientError::MalformedAccountId(value.to_string()))?;
        let low = decode_lower_hex_nibble(chunk[1])
            .ok_or_else(|| ClientError::MalformedAccountId(value.to_string()))?;
        bytes[index] = (high << 4) | low;
    }
    Ok(bytes)
}

fn decode_lower_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn mls_group_id_string(group_id: &GroupId) -> Result<String, ClientError> {
    String::from_utf8(group_id.as_slice().to_vec()).map_err(|_| ClientError::MlsGroupIdNotUtf8)
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

fn prepare_client_store_schema(conn: &Connection) -> Result<(), ClientStoreError> {
    reject_legacy_client_store_tables(conn)?;
    reject_legacy_app_projection_schema(conn)?;
    create_current_client_store_schema(conn)?;
    Ok(())
}

fn create_current_client_store_schema(conn: &Connection) -> Result<(), ClientStoreError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS client_device_states (
          account_id TEXT NOT NULL,
          device_id TEXT NOT NULL,
          nonce BLOB NOT NULL,
          ciphertext BLOB NOT NULL,
          PRIMARY KEY (account_id, device_id)
        );

        CREATE TABLE IF NOT EXISTS client_app_messages (
          account_id TEXT NOT NULL,
          device_id TEXT NOT NULL,
          room_id TEXT NOT NULL,
          seq INTEGER NOT NULL,
          message_id TEXT NOT NULL,
          sender_account_id TEXT NOT NULL,
          sender_device_id TEXT NOT NULL,
          timestamp_unix_seconds INTEGER NOT NULL,
          nonce BLOB NOT NULL,
          ciphertext BLOB NOT NULL,
          PRIMARY KEY (account_id, device_id, room_id, message_id),
          FOREIGN KEY (account_id, device_id)
            REFERENCES client_device_states(account_id, device_id)
            ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS client_app_messages_owner_room_seq_idx
          ON client_app_messages(account_id, device_id, room_id, seq);

        CREATE INDEX IF NOT EXISTS client_app_messages_owner_idx
          ON client_app_messages(account_id, device_id);

        CREATE TABLE IF NOT EXISTS client_app_events (
          account_id TEXT NOT NULL,
          device_id TEXT NOT NULL,
          room_id TEXT NOT NULL,
          seq INTEGER NOT NULL,
          message_id TEXT NOT NULL,
          sender_account_id TEXT NOT NULL,
          sender_device_id TEXT NOT NULL,
          timestamp_unix_seconds INTEGER NOT NULL,
          nonce BLOB NOT NULL,
          ciphertext BLOB NOT NULL,
          PRIMARY KEY (account_id, device_id, room_id, message_id),
          FOREIGN KEY (account_id, device_id)
            REFERENCES client_device_states(account_id, device_id)
            ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS client_app_events_owner_room_seq_idx
          ON client_app_events(account_id, device_id, room_id, seq);

        CREATE INDEX IF NOT EXISTS client_app_events_owner_idx
          ON client_app_events(account_id, device_id);

        CREATE TABLE IF NOT EXISTS client_app_outbox (
          account_id TEXT NOT NULL,
          device_id TEXT NOT NULL,
          room_id TEXT NOT NULL,
          message_id TEXT NOT NULL,
          nonce BLOB NOT NULL,
          ciphertext BLOB NOT NULL,
          PRIMARY KEY (account_id, device_id, room_id, message_id),
          FOREIGN KEY (account_id, device_id)
            REFERENCES client_device_states(account_id, device_id)
            ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS client_app_outbox_owner_idx
          ON client_app_outbox(account_id, device_id);

        CREATE TABLE IF NOT EXISTS client_app_rooms (
          account_id TEXT NOT NULL,
          device_id TEXT NOT NULL,
          room_id TEXT NOT NULL,
          nonce BLOB NOT NULL,
          ciphertext BLOB NOT NULL,
          PRIMARY KEY (account_id, device_id, room_id),
          FOREIGN KEY (account_id, device_id)
            REFERENCES client_device_states(account_id, device_id)
            ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS client_app_rooms_owner_idx
          ON client_app_rooms(account_id, device_id);

        CREATE TABLE IF NOT EXISTS client_app_state (
          account_id TEXT NOT NULL,
          device_id TEXT NOT NULL,
          nonce BLOB NOT NULL,
          ciphertext BLOB NOT NULL,
          PRIMARY KEY (account_id, device_id),
          FOREIGN KEY (account_id, device_id)
            REFERENCES client_device_states(account_id, device_id)
            ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS client_app_profiles (
          account_id TEXT NOT NULL,
          device_id TEXT NOT NULL,
          profile_account_id TEXT NOT NULL,
          nonce BLOB NOT NULL,
          ciphertext BLOB NOT NULL,
          PRIMARY KEY (account_id, device_id, profile_account_id),
          FOREIGN KEY (account_id, device_id)
            REFERENCES client_device_states(account_id, device_id)
            ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS client_app_profiles_owner_idx
          ON client_app_profiles(account_id, device_id);

        CREATE TABLE IF NOT EXISTS client_device_link_bootstrap_transfers (
          account_id TEXT NOT NULL,
          device_id TEXT NOT NULL,
          room_id TEXT NOT NULL,
          bootstrap_id TEXT NOT NULL,
          source_account_id TEXT NOT NULL,
          source_device_id TEXT NOT NULL,
          state INTEGER NOT NULL
            CHECK (state IN (0, 1, 2, 3)),
          earliest_envelope_seq INTEGER NOT NULL,
          manifest_sha256 TEXT NOT NULL,
          manifest_nonce BLOB NOT NULL,
          manifest_ciphertext BLOB NOT NULL,
          PRIMARY KEY (
            account_id,
            device_id,
            room_id,
            bootstrap_id,
            source_account_id,
            source_device_id
          ),
          FOREIGN KEY (account_id, device_id)
            REFERENCES client_device_states(account_id, device_id)
            ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS client_device_link_bootstrap_transfers_owner_state_idx
          ON client_device_link_bootstrap_transfers(account_id, device_id, state);

        CREATE TABLE IF NOT EXISTS client_device_link_bootstrap_chunks (
          account_id TEXT NOT NULL,
          device_id TEXT NOT NULL,
          room_id TEXT NOT NULL,
          bootstrap_id TEXT NOT NULL,
          source_account_id TEXT NOT NULL,
          source_device_id TEXT NOT NULL,
          chunk_index INTEGER NOT NULL,
          chunk_sha256 BLOB NOT NULL,
          nonce BLOB,
          ciphertext BLOB,
          CHECK (
            (nonce IS NULL AND ciphertext IS NULL)
            OR (nonce IS NOT NULL AND ciphertext IS NOT NULL)
          ),
          PRIMARY KEY (
            account_id,
            device_id,
            room_id,
            bootstrap_id,
            source_account_id,
            source_device_id,
            chunk_index
          ),
          FOREIGN KEY (
            account_id,
            device_id,
            room_id,
            bootstrap_id,
            source_account_id,
            source_device_id
          ) REFERENCES client_device_link_bootstrap_transfers(
            account_id,
            device_id,
            room_id,
            bootstrap_id,
            source_account_id,
            source_device_id
          ) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS client_device_link_bootstrap_chunks_transfer_idx
          ON client_device_link_bootstrap_chunks(
            account_id,
            device_id,
            room_id,
            bootstrap_id,
            source_account_id,
            source_device_id,
            chunk_index
          );
        "#,
    )?;
    Ok(())
}

fn reject_legacy_app_projection_schema(conn: &Connection) -> Result<(), ClientStoreError> {
    reject_legacy_app_projection_table(
        conn,
        "client_app_messages",
        &[
            "account_id",
            "device_id",
            "room_id",
            "seq",
            "message_id",
            "sender_account_id",
            "sender_device_id",
            "timestamp_unix_seconds",
            "nonce",
            "ciphertext",
        ],
    )?;
    reject_legacy_app_projection_table(
        conn,
        "client_app_events",
        &[
            "account_id",
            "device_id",
            "room_id",
            "seq",
            "message_id",
            "sender_account_id",
            "sender_device_id",
            "timestamp_unix_seconds",
            "nonce",
            "ciphertext",
        ],
    )?;
    reject_legacy_app_projection_table(
        conn,
        "client_app_outbox",
        &[
            "account_id",
            "device_id",
            "room_id",
            "message_id",
            "nonce",
            "ciphertext",
        ],
    )?;
    reject_legacy_app_projection_table(
        conn,
        "client_app_rooms",
        &["account_id", "device_id", "room_id", "nonce", "ciphertext"],
    )?;
    reject_legacy_app_projection_table(
        conn,
        "client_app_state",
        &["account_id", "device_id", "nonce", "ciphertext"],
    )?;
    reject_legacy_app_projection_table(
        conn,
        "client_app_profiles",
        &[
            "account_id",
            "device_id",
            "profile_account_id",
            "nonce",
            "ciphertext",
        ],
    )
}

fn reject_legacy_app_projection_table(
    conn: &Connection,
    table: &str,
    required_columns: &[&str],
) -> Result<(), ClientStoreError> {
    if !sqlite_table_exists(conn, table)? {
        return Ok(());
    }
    let columns = sqlite_table_columns(conn, table)?;
    if columns.iter().any(|column| column == "plaintext") {
        return Err(ClientStoreError::LegacyAppProjectionSchema {
            table: table.to_owned(),
            reason: "plaintext column is unsupported".to_owned(),
        });
    }
    for column in required_columns {
        if !columns.iter().any(|found| found == column) {
            return Err(ClientStoreError::LegacyAppProjectionSchema {
                table: table.to_owned(),
                reason: format!("missing required column {column}"),
            });
        }
    }
    for column in &columns {
        if !required_columns.contains(&column.as_str()) {
            return Err(ClientStoreError::LegacyAppProjectionSchema {
                table: table.to_owned(),
                reason: format!("unsupported column {column}"),
            });
        }
    }
    if sqlite_table_column_default(conn, table, "timestamp_unix_seconds")?.is_some() {
        return Err(ClientStoreError::LegacyAppProjectionSchema {
            table: table.to_owned(),
            reason: "timestamp_unix_seconds default is unsupported".to_owned(),
        });
    }
    Ok(())
}

fn sqlite_table_column_default(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<Option<String>, ClientStoreError> {
    if !sqlite_table_exists(conn, table)? {
        return Ok(None);
    }
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(1)?, row.get(4)?)))?;
    for row in rows {
        let (found_column, default): (String, Option<String>) = row?;
        if found_column == column {
            return Ok(default);
        }
    }
    Ok(None)
}

fn sqlite_table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, ClientStoreError> {
    if !sqlite_table_exists(conn, table)? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = Vec::new();
    for row in rows {
        columns.push(row?);
    }
    Ok(columns)
}

#[cfg(test)]
fn sqlite_table_has_column(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, ClientStoreError> {
    if !sqlite_table_exists(conn, table)? {
        return Ok(false);
    }
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn sqlite_table_exists(conn: &Connection, table: &str) -> Result<bool, ClientStoreError> {
    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        params![table],
        |row| row.get::<_, bool>(0),
    )?;
    Ok(exists)
}

fn reject_legacy_client_store_tables(conn: &Connection) -> Result<(), ClientStoreError> {
    let tables = [
        LegacyClientStoreTable::OpenMlsStorage,
        LegacyClientStoreTable::Rooms,
        LegacyClientStoreTable::Profiles,
    ];
    for table in tables {
        if legacy_table_exists(conn, table)? {
            return Err(ClientStoreError::LegacyUnencryptedStoreTable { table });
        }
    }
    Ok(())
}

fn legacy_table_exists(
    conn: &Connection,
    table: LegacyClientStoreTable,
) -> Result<bool, ClientStoreError> {
    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        params![table.name()],
        |row| row.get::<_, bool>(0),
    )?;
    Ok(exists)
}

fn save_device_state_tx(
    tx: &Transaction<'_>,
    state: &FiniteChatDeviceState,
    encryption_key: &ClientStoreEncryptionKey,
) -> Result<(), ClientStoreError> {
    state.validate_limits()?;
    let sealed = encrypt_device_state(encryption_key, state)?;
    tx.execute(
        r#"
        INSERT INTO client_device_states (
          account_id, device_id, nonce, ciphertext
        ) VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(account_id, device_id) DO UPDATE SET
          nonce = excluded.nonce,
          ciphertext = excluded.ciphertext
        "#,
        params![
            state.device_ref.account_id,
            state.device_ref.device_id,
            sealed.nonce,
            sealed.ciphertext,
        ],
    )?;
    Ok(())
}

fn validate_app_message_owner(owner: &DeviceRef) -> Result<(), ClientStoreError> {
    owner.validate_limits().map_err(ClientError::from)?;
    Ok(())
}

fn validate_app_message_limit(limit: u32) -> Result<(), ClientStoreError> {
    if (1..=MAX_STORED_APP_MESSAGES).contains(&limit) {
        return Ok(());
    }
    Err(ClientStoreError::InvalidAppMessageLimit {
        limit,
        max: MAX_STORED_APP_MESSAGES,
    })
}

fn validate_app_event_limit(limit: u32) -> Result<(), ClientStoreError> {
    validate_app_message_limit(limit)
}

fn validate_optional_app_room_picture(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), ClientError> {
    validate_optional_profile_text(field, value, MAX_APP_ROOM_PICTURE_BYTES)?;
    if let Some(picture) = value
        && !picture.starts_with("http://")
        && !picture.starts_with("https://")
    {
        return Err(ClientError::RoomPictureUrl(picture.to_owned()));
    }
    Ok(())
}

fn validate_nostr_profile_record(profile: &NostrProfileRecord) -> Result<(), ClientError> {
    decode_lower_hex_32(&profile.account_id)?;
    validate_optional_profile_text(
        "profile.name",
        profile.name.as_deref(),
        MAX_APP_PROFILE_NAME_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.display_name",
        profile.display_name.as_deref(),
        MAX_APP_PROFILE_NAME_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.about",
        profile.about.as_deref(),
        MAX_APP_PROFILE_ABOUT_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.picture",
        profile.picture.as_deref(),
        MAX_APP_PROFILE_PICTURE_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.finite_role",
        profile.finite_role.as_deref(),
        MAX_APP_PROFILE_NAME_BYTES,
    )?;
    if let Some(picture) = &profile.picture
        && !picture.starts_with("http://")
        && !picture.starts_with("https://")
    {
        return Err(ClientError::ProfilePictureUrl(picture.clone()));
    }
    if profile.expires_at_ms <= profile.fetched_at_ms {
        return Err(ClientError::ProfileExpiry {
            fetched_at_ms: profile.fetched_at_ms,
            expires_at_ms: profile.expires_at_ms,
        });
    }
    Ok(())
}

fn validate_optional_profile_text(
    field: &'static str,
    value: Option<&str>,
    max_bytes: u32,
) -> Result<(), ClientError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_string_bytes(field, value, max_bytes)?;
    Ok(())
}

fn sqlite_seq_from_u64(seq: u64) -> Result<i64, ClientStoreError> {
    i64::try_from(seq).map_err(|_| ClientStoreError::StoredAppMessageSeqOutOfRange { seq })
}

fn sqlite_seq_to_u64(seq: i64) -> Result<u64, ClientStoreError> {
    u64::try_from(seq).map_err(|_| ClientStoreError::NegativeStoredAppMessageSeq { seq })
}

fn sqlite_app_event_seq_from_u64(seq: u64) -> Result<i64, ClientStoreError> {
    i64::try_from(seq).map_err(|_| ClientStoreError::StoredAppEventSeqOutOfRange { seq })
}

fn sqlite_app_event_seq_to_u64(seq: i64) -> Result<u64, ClientStoreError> {
    u64::try_from(seq).map_err(|_| ClientStoreError::NegativeStoredAppEventSeq { seq })
}

fn sqlite_timestamp_from_u64(timestamp: u64) -> Result<i64, ClientStoreError> {
    i64::try_from(timestamp)
        .map_err(|_| ClientStoreError::StoredAppTimestampOutOfRange { timestamp })
}

fn sqlite_timestamp_to_u64(timestamp: i64) -> Result<u64, ClientStoreError> {
    u64::try_from(timestamp).map_err(|_| ClientStoreError::NegativeStoredAppTimestamp { timestamp })
}

fn sqlite_device_link_chunk_index_from_u32(index: u32) -> Result<i64, ClientStoreError> {
    Ok(i64::from(index))
}

fn sqlite_device_link_chunk_index_to_u32(index: i64) -> Result<u32, ClientStoreError> {
    u32::try_from(index)
        .map_err(|_| ClientStoreError::NegativeDeviceLinkBootstrapChunkIndex { index })
}

#[derive(Clone, Copy)]
struct DeviceLinkBootstrapIdentity<'a> {
    owner: &'a DeviceRef,
    room_id: &'a str,
    bootstrap_id: &'a str,
    source: &'a DeviceRef,
}

fn device_link_bootstrap_identity_from_receipt<'a>(
    owner: &'a DeviceRef,
    receipt: &'a StoredDeviceLinkBootstrapReceipt,
) -> DeviceLinkBootstrapIdentity<'a> {
    DeviceLinkBootstrapIdentity {
        owner,
        room_id: &receipt.room_id,
        bootstrap_id: &receipt.bootstrap_id,
        source: &receipt.source,
    }
}

fn validate_device_link_bootstrap_identity(
    identity: DeviceLinkBootstrapIdentity<'_>,
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(identity.owner)?;
    validate_room_id(identity.room_id).map_err(ClientError::from)?;
    validate_bytes_non_empty(
        "device_link_bootstrap.bootstrap_id",
        identity.bootstrap_id.len(),
    )
    .map_err(ClientError::from)?;
    validate_string_bytes(
        "device_link_bootstrap.bootstrap_id",
        identity.bootstrap_id,
        MAX_OBJECT_ID_BYTES,
    )
    .map_err(ClientError::from)?;
    identity
        .source
        .validate_limits()
        .map_err(ClientError::from)?;
    Ok(())
}

fn validate_device_link_bootstrap_for_owner(
    owner: &DeviceRef,
    source: &DeviceRef,
    bootstrap: &DeviceLinkBootstrapV2,
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    source.validate_limits().map_err(ClientError::from)?;
    bootstrap.validate_limits().map_err(ClientError::from)?;
    if source.account_id != owner.account_id
        || source.device_id == owner.device_id
        || bootstrap.target != *owner
    {
        return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
    }
    let encoded = serde_json::to_vec(bootstrap)
        .map_err(|_| ClientStoreError::EncodeDeviceLinkBootstrapChunk)?;
    validate_bytes_len(
        "device_link_bootstrap.payload",
        encoded.len(),
        finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_PAYLOAD_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(())
}

fn validate_device_link_bootstrap_manifest_fields(
    source: &DeviceRef,
    bootstrap: &DeviceLinkBootstrapV2,
) -> Result<(), ClientStoreError> {
    source.validate_limits().map_err(ClientError::from)?;
    bootstrap
        .target
        .validate_limits()
        .map_err(ClientError::from)?;
    validate_bytes_non_empty(
        "device_link_bootstrap.bootstrap_id",
        bootstrap.bootstrap_id.len(),
    )
    .map_err(ClientError::from)?;
    validate_string_bytes(
        "device_link_bootstrap.bootstrap_id",
        &bootstrap.bootstrap_id,
        MAX_OBJECT_ID_BYTES,
    )
    .map_err(ClientError::from)?;
    if bootstrap.version != finitechat_proto::DEVICE_LINK_BOOTSTRAP_VERSION_V2
        || bootstrap.chunk_count == 0
        || bootstrap.chunk_count > finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_CHUNKS
        || bootstrap.total_history_events > finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_TOTAL_EVENTS
        || !is_lower_hex_sha256(&bootstrap.history_sha256)
    {
        return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
    }
    if bootstrap.total_history_events == 0 {
        if bootstrap.chunk_count != 1 {
            return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
        }
    } else if u64::from(bootstrap.chunk_count) > bootstrap.total_history_events
        || bootstrap.total_history_events
            > u64::from(bootstrap.chunk_count)
                * u64::from(finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_EVENTS)
    {
        return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
    }
    bootstrap
        .room
        .validate_limits()
        .map_err(ClientError::from)?;
    if let Some(selection) = &bootstrap.canonical_selection {
        selection.validate_limits().map_err(ClientError::from)?;
    }
    validate_item_count(
        "device_link_bootstrap.profiles",
        bootstrap.profiles.len(),
        finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_PROFILES,
    )
    .map_err(ClientError::from)?;
    for profile in &bootstrap.profiles {
        profile.validate_limits().map_err(ClientError::from)?;
    }
    Ok(())
}

fn validate_device_link_bootstrap_receipt(
    receipt: &StoredDeviceLinkBootstrapReceipt,
) -> Result<(), ClientStoreError> {
    validate_room_id(&receipt.room_id).map_err(ClientError::from)?;
    validate_bytes_non_empty(
        "device_link_bootstrap.bootstrap_id",
        receipt.bootstrap_id.len(),
    )
    .map_err(ClientError::from)?;
    validate_string_bytes(
        "device_link_bootstrap.bootstrap_id",
        &receipt.bootstrap_id,
        MAX_OBJECT_ID_BYTES,
    )
    .map_err(ClientError::from)?;
    receipt
        .source
        .validate_limits()
        .map_err(ClientError::from)?;
    receipt
        .target
        .validate_limits()
        .map_err(ClientError::from)?;
    if receipt.chunk_count == 0
        || receipt.chunk_count > finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_CHUNKS
        || receipt.total_history_events > finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_TOTAL_EVENTS
        || !is_lower_hex_sha256(&receipt.history_sha256)
        || !is_lower_hex_sha256(&receipt.manifest_sha256)
    {
        return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
    }
    Ok(())
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn device_link_bootstrap_from_stored_event(
    owner: &DeviceRef,
    event: &StoredAppEvent,
) -> Option<(DeviceRef, DeviceLinkBootstrapV2)> {
    if event.sender.account_id != owner.account_id || event.sender.device_id == owner.device_id {
        return None;
    }
    let app_event = serde_json::from_slice::<DecryptedApplicationEventV1>(&event.plaintext).ok()?;
    app_event.validate_limits().ok()?;
    match app_event.kind {
        DurableAppEventKind::Namespaced { name, policy }
            if name == FINITECHAT_DEVICE_LINK_BOOTSTRAP_EVENT_V2
                && policy == ApplicationDeliveryPolicy::NON_NOTIFYING => {}
        _ => return None,
    }
    if app_event.payload.len() > finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_PAYLOAD_BYTES as usize
    {
        return None;
    }
    let bootstrap = serde_json::from_slice::<DeviceLinkBootstrapV2>(&app_event.payload).ok()?;
    validate_device_link_bootstrap_for_owner(owner, &event.sender, &bootstrap).ok()?;
    (bootstrap.room.room_id == event.room_id).then(|| (event.sender.clone(), bootstrap))
}

#[derive(Debug)]
struct StoredDeviceLinkBootstrapTransferRow {
    state: i64,
    earliest_envelope_seq: u64,
    manifest_sha256: String,
    manifest: StoredDeviceLinkBootstrapManifestV2,
}

fn load_device_link_bootstrap_transfer_row(
    conn: &Connection,
    encryption_key: &ClientStoreEncryptionKey,
    identity: DeviceLinkBootstrapIdentity<'_>,
) -> Result<Option<StoredDeviceLinkBootstrapTransferRow>, ClientStoreError> {
    validate_device_link_bootstrap_identity(identity)?;
    let row = conn
        .query_row(
            r#"
            SELECT
              state,
              earliest_envelope_seq,
              manifest_sha256,
              manifest_nonce,
              manifest_ciphertext
            FROM client_device_link_bootstrap_transfers
            WHERE account_id = ?1
              AND device_id = ?2
              AND room_id = ?3
              AND bootstrap_id = ?4
              AND source_account_id = ?5
              AND source_device_id = ?6
            "#,
            params![
                &identity.owner.account_id,
                &identity.owner.device_id,
                identity.room_id,
                identity.bootstrap_id,
                &identity.source.account_id,
                &identity.source.device_id,
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((state, earliest_envelope_seq, manifest_sha256, nonce, ciphertext)) = row else {
        return Ok(None);
    };
    validate_device_link_bootstrap_state(state)?;
    if !is_lower_hex_sha256(&manifest_sha256) {
        return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
    }
    let manifest = decrypt_device_link_bootstrap_manifest(
        encryption_key,
        identity,
        &manifest_sha256,
        &nonce,
        &ciphertext,
    )?;
    if manifest.bootstrap_id != identity.bootstrap_id
        || manifest.room.room_id != identity.room_id
        || manifest.source != *identity.source
        || manifest.manifest_sha256()? != manifest_sha256
    {
        return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
    }
    Ok(Some(StoredDeviceLinkBootstrapTransferRow {
        state,
        earliest_envelope_seq: sqlite_app_event_seq_to_u64(earliest_envelope_seq)?,
        manifest_sha256,
        manifest,
    }))
}

fn validate_device_link_bootstrap_state(state: i64) -> Result<(), ClientStoreError> {
    match state {
        DEVICE_LINK_BOOTSTRAP_STATE_RECEIVING
        | DEVICE_LINK_BOOTSTRAP_STATE_POISONED
        | DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED
        | DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED_POISONED => Ok(()),
        state => Err(ClientStoreError::InvalidDeviceLinkBootstrapState { state }),
    }
}

fn poison_device_link_bootstrap_identity_tx(
    tx: &Transaction<'_>,
    identity: DeviceLinkBootstrapIdentity<'_>,
) -> Result<(), ClientStoreError> {
    tx.execute(
        r#"
        UPDATE client_device_link_bootstrap_transfers
        SET state = CASE
          WHEN state IN (?7, ?8) THEN ?8
          ELSE ?9
        END
        WHERE account_id = ?1
          AND device_id = ?2
          AND room_id = ?3
          AND bootstrap_id = ?4
          AND source_account_id = ?5
          AND source_device_id = ?6
        "#,
        params![
            &identity.owner.account_id,
            &identity.owner.device_id,
            identity.room_id,
            identity.bootstrap_id,
            &identity.source.account_id,
            &identity.source.device_id,
            DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED,
            DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED_POISONED,
            DEVICE_LINK_BOOTSTRAP_STATE_POISONED,
        ],
    )?;
    Ok(())
}

fn poison_device_link_bootstrap_tx(
    tx: &Transaction<'_>,
    owner: &DeviceRef,
    expected: &StoredDeviceLinkBootstrapReceipt,
) -> Result<(), ClientStoreError> {
    let identity = device_link_bootstrap_identity_from_receipt(owner, expected);
    tx.execute(
        r#"
        UPDATE client_device_link_bootstrap_transfers
        SET state = CASE
          WHEN state IN (?8, ?9) THEN ?9
          ELSE ?10
        END
        WHERE account_id = ?1
          AND device_id = ?2
          AND room_id = ?3
          AND bootstrap_id = ?4
          AND source_account_id = ?5
          AND source_device_id = ?6
          AND manifest_sha256 = ?7
        "#,
        params![
            &identity.owner.account_id,
            &identity.owner.device_id,
            identity.room_id,
            identity.bootstrap_id,
            &identity.source.account_id,
            &identity.source.device_id,
            &expected.manifest_sha256,
            DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED,
            DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED_POISONED,
            DEVICE_LINK_BOOTSTRAP_STATE_POISONED,
        ],
    )?;
    Ok(())
}

fn stage_device_link_bootstrap_chunk_tx(
    tx: &Transaction<'_>,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    source: &DeviceRef,
    envelope_seq: u64,
    bootstrap: &DeviceLinkBootstrapV2,
) -> Result<DeviceLinkBootstrapStageOutcome, ClientStoreError> {
    validate_device_link_bootstrap_for_owner(owner, source, bootstrap)?;
    let manifest = StoredDeviceLinkBootstrapManifestV2::from_bootstrap(source, bootstrap)?;
    let receipt = manifest.receipt()?;
    let identity = device_link_bootstrap_identity_from_receipt(owner, &receipt);
    let envelope_seq = sqlite_app_event_seq_from_u64(envelope_seq)?;
    let mut stored = load_device_link_bootstrap_transfer_row(tx, encryption_key, identity)?;

    if stored.is_none() {
        let active_count = tx.query_row(
            r#"
            SELECT COUNT(*)
            FROM client_device_link_bootstrap_transfers
            WHERE account_id = ?1
              AND device_id = ?2
              AND state IN (?3, ?4)
            "#,
            params![
                &owner.account_id,
                &owner.device_id,
                DEVICE_LINK_BOOTSTRAP_STATE_RECEIVING,
                DEVICE_LINK_BOOTSTRAP_STATE_POISONED,
            ],
            |row| row.get::<_, i64>(0),
        )?;
        if active_count >= i64::from(MAX_PENDING_DEVICE_LINK_BOOTSTRAPS) {
            return Ok(DeviceLinkBootstrapStageOutcome::CapacityExceeded);
        }
        let sealed = encrypt_device_link_bootstrap_manifest(
            encryption_key,
            identity,
            &receipt.manifest_sha256,
            &manifest,
        )?;
        tx.execute(
            r#"
            INSERT INTO client_device_link_bootstrap_transfers (
              account_id,
              device_id,
              room_id,
              bootstrap_id,
              source_account_id,
              source_device_id,
              state,
              earliest_envelope_seq,
              manifest_sha256,
              manifest_nonce,
              manifest_ciphertext
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                &owner.account_id,
                &owner.device_id,
                &receipt.room_id,
                &receipt.bootstrap_id,
                &receipt.source.account_id,
                &receipt.source.device_id,
                DEVICE_LINK_BOOTSTRAP_STATE_RECEIVING,
                envelope_seq,
                &receipt.manifest_sha256,
                &sealed.nonce,
                &sealed.ciphertext,
            ],
        )?;
        stored = Some(StoredDeviceLinkBootstrapTransferRow {
            state: DEVICE_LINK_BOOTSTRAP_STATE_RECEIVING,
            earliest_envelope_seq: sqlite_app_event_seq_to_u64(envelope_seq)?,
            manifest_sha256: receipt.manifest_sha256.clone(),
            manifest: manifest.clone(),
        });
    }

    let stored = stored.expect("new or existing bootstrap transfer row");
    if stored.manifest != manifest
        || stored.manifest_sha256 != receipt.manifest_sha256
        || stored.manifest.receipt()? != receipt
    {
        poison_device_link_bootstrap_identity_tx(tx, identity)?;
        return Ok(DeviceLinkBootstrapStageOutcome::Poisoned);
    }
    match stored.state {
        DEVICE_LINK_BOOTSTRAP_STATE_POISONED | DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED_POISONED => {
            return Ok(DeviceLinkBootstrapStageOutcome::Poisoned);
        }
        DEVICE_LINK_BOOTSTRAP_STATE_RECEIVING | DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED => {}
        state => return Err(ClientStoreError::InvalidDeviceLinkBootstrapState { state }),
    }

    let chunk_index = sqlite_device_link_chunk_index_from_u32(bootstrap.chunk_index)?;
    let candidate_chunk_sha256 = device_link_bootstrap_chunk_sha256(&bootstrap.history);
    let existing_chunk = tx
        .query_row(
            r#"
            SELECT chunk_sha256, nonce, ciphertext
            FROM client_device_link_bootstrap_chunks
            WHERE account_id = ?1
              AND device_id = ?2
              AND room_id = ?3
              AND bootstrap_id = ?4
              AND source_account_id = ?5
              AND source_device_id = ?6
              AND chunk_index = ?7
            "#,
            params![
                &owner.account_id,
                &owner.device_id,
                &receipt.room_id,
                &receipt.bootstrap_id,
                &receipt.source.account_id,
                &receipt.source.device_id,
                chunk_index,
            ],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Option<Vec<u8>>>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                ))
            },
        )
        .optional()?;

    let exact_duplicate = if let Some((stored_digest, nonce, ciphertext)) = existing_chunk {
        let digest_matches = stored_digest.as_slice() == candidate_chunk_sha256;
        let body_matches = match (nonce, ciphertext) {
            (Some(nonce), Some(ciphertext)) => {
                decrypt_device_link_bootstrap_chunk(
                    encryption_key,
                    identity,
                    &receipt.manifest_sha256,
                    bootstrap.chunk_index,
                    &nonce,
                    &ciphertext,
                )? == bootstrap.history
            }
            (None, None) => digest_matches,
            _ => false,
        };
        if !digest_matches || !body_matches {
            poison_device_link_bootstrap_identity_tx(tx, identity)?;
            return Ok(DeviceLinkBootstrapStageOutcome::Poisoned);
        }
        true
    } else {
        if stored.state == DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED {
            poison_device_link_bootstrap_identity_tx(tx, identity)?;
            return Ok(DeviceLinkBootstrapStageOutcome::Poisoned);
        }
        let sealed = encrypt_device_link_bootstrap_chunk(
            encryption_key,
            identity,
            &receipt.manifest_sha256,
            bootstrap.chunk_index,
            &bootstrap.history,
        )?;
        tx.execute(
            r#"
            INSERT INTO client_device_link_bootstrap_chunks (
              account_id,
              device_id,
              room_id,
              bootstrap_id,
              source_account_id,
              source_device_id,
              chunk_index,
              chunk_sha256,
              nonce,
              ciphertext
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                &owner.account_id,
                &owner.device_id,
                &receipt.room_id,
                &receipt.bootstrap_id,
                &receipt.source.account_id,
                &receipt.source.device_id,
                chunk_index,
                candidate_chunk_sha256.as_slice(),
                &sealed.nonce,
                &sealed.ciphertext,
            ],
        )?;
        false
    };

    if stored.state == DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED {
        return Ok(DeviceLinkBootstrapStageOutcome::AlreadyCommitted(receipt));
    }
    tx.execute(
        r#"
        UPDATE client_device_link_bootstrap_transfers
        SET earliest_envelope_seq = MIN(earliest_envelope_seq, ?7)
        WHERE account_id = ?1
          AND device_id = ?2
          AND room_id = ?3
          AND bootstrap_id = ?4
          AND source_account_id = ?5
          AND source_device_id = ?6
        "#,
        params![
            &owner.account_id,
            &owner.device_id,
            &receipt.room_id,
            &receipt.bootstrap_id,
            &receipt.source.account_id,
            &receipt.source.device_id,
            envelope_seq,
        ],
    )?;
    let received_chunks = tx.query_row(
        r#"
        SELECT COUNT(*)
        FROM client_device_link_bootstrap_chunks
        WHERE account_id = ?1
          AND device_id = ?2
          AND room_id = ?3
          AND bootstrap_id = ?4
          AND source_account_id = ?5
          AND source_device_id = ?6
        "#,
        params![
            &owner.account_id,
            &owner.device_id,
            &receipt.room_id,
            &receipt.bootstrap_id,
            &receipt.source.account_id,
            &receipt.source.device_id,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    let received_chunks = u32::try_from(received_chunks)
        .map_err(|_| ClientStoreError::DeviceLinkBootstrapChunkCountOverflow)?;
    if received_chunks == receipt.chunk_count {
        let ready = load_pending_device_link_bootstrap(tx, encryption_key, owner, &receipt)?
            .ok_or(ClientStoreError::DeviceLinkBootstrapReceiptMismatch)?;
        return Ok(DeviceLinkBootstrapStageOutcome::Ready(ready));
    }
    if received_chunks > receipt.chunk_count {
        poison_device_link_bootstrap_identity_tx(tx, identity)?;
        return Ok(DeviceLinkBootstrapStageOutcome::Poisoned);
    }
    if exact_duplicate {
        Ok(DeviceLinkBootstrapStageOutcome::ExactDuplicate {
            received_chunks,
            expected_chunks: receipt.chunk_count,
        })
    } else {
        Ok(DeviceLinkBootstrapStageOutcome::Pending {
            received_chunks,
            expected_chunks: receipt.chunk_count,
        })
    }
}

fn load_pending_device_link_bootstrap(
    conn: &Connection,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    receipt: &StoredDeviceLinkBootstrapReceipt,
) -> Result<Option<StoredPendingDeviceLinkBootstrap>, ClientStoreError> {
    let identity = device_link_bootstrap_identity_from_receipt(owner, receipt);
    let Some(row) = load_device_link_bootstrap_transfer_row(conn, encryption_key, identity)? else {
        return Ok(None);
    };
    if row.state != DEVICE_LINK_BOOTSTRAP_STATE_RECEIVING {
        return Ok(None);
    }
    if row.manifest.receipt()? != *receipt {
        return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
    }
    let mut stmt = conn.prepare(
        r#"
        SELECT chunk_index, chunk_sha256, nonce, ciphertext
        FROM client_device_link_bootstrap_chunks
        WHERE account_id = ?1
          AND device_id = ?2
          AND room_id = ?3
          AND bootstrap_id = ?4
          AND source_account_id = ?5
          AND source_device_id = ?6
        ORDER BY chunk_index ASC
        "#,
    )?;
    let rows = stmt.query_map(
        params![
            &owner.account_id,
            &owner.device_id,
            &receipt.room_id,
            &receipt.bootstrap_id,
            &receipt.source.account_id,
            &receipt.source.device_id,
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
            ))
        },
    )?;
    let mut chunks = BTreeMap::new();
    for row in rows {
        let (chunk_index, stored_digest, nonce, ciphertext) = row?;
        let chunk_index = sqlite_device_link_chunk_index_to_u32(chunk_index)?;
        let (Some(nonce), Some(ciphertext)) = (nonce, ciphertext) else {
            return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
        };
        let history = decrypt_device_link_bootstrap_chunk(
            encryption_key,
            identity,
            &receipt.manifest_sha256,
            chunk_index,
            &nonce,
            &ciphertext,
        )?;
        if stored_digest.as_slice() != device_link_bootstrap_chunk_sha256(&history) {
            return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
        }
        if chunks.insert(chunk_index, history).is_some() {
            return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
        }
    }
    Ok(Some(StoredPendingDeviceLinkBootstrap {
        receipt: receipt.clone(),
        room: row.manifest.room,
        canonical_selection: row.manifest.canonical_selection,
        profiles: row.manifest.profiles,
        earliest_envelope_seq: row.earliest_envelope_seq,
        chunks,
    }))
}

fn load_pending_device_link_bootstraps(
    conn: &Connection,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
) -> Result<Vec<StoredPendingDeviceLinkBootstrap>, ClientStoreError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT room_id, bootstrap_id, source_account_id, source_device_id
        FROM client_device_link_bootstrap_transfers
        WHERE account_id = ?1
          AND device_id = ?2
          AND state = ?3
        ORDER BY room_id, bootstrap_id, source_account_id, source_device_id
        "#,
    )?;
    let keys = stmt
        .query_map(
            params![
                &owner.account_id,
                &owner.device_id,
                DEVICE_LINK_BOOTSTRAP_STATE_RECEIVING,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    DeviceRef {
                        account_id: row.get::<_, String>(2)?,
                        device_id: row.get::<_, String>(3)?,
                    },
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut pending = Vec::with_capacity(keys.len());
    for (room_id, bootstrap_id, source) in keys {
        let identity = DeviceLinkBootstrapIdentity {
            owner,
            room_id: &room_id,
            bootstrap_id: &bootstrap_id,
            source: &source,
        };
        let row = load_device_link_bootstrap_transfer_row(conn, encryption_key, identity)?
            .ok_or(ClientStoreError::DeviceLinkBootstrapReceiptMismatch)?;
        let receipt = row.manifest.receipt()?;
        pending.push(
            load_pending_device_link_bootstrap(conn, encryption_key, owner, &receipt)?
                .ok_or(ClientStoreError::DeviceLinkBootstrapReceiptMismatch)?,
        );
    }
    Ok(pending)
}

fn validated_device_link_bootstrap_events(
    pending: &StoredPendingDeviceLinkBootstrap,
) -> Option<Vec<StoredAppEvent>> {
    if !pending.is_complete() {
        return None;
    }
    let mut transfer_digest = Sha256::new();
    let mut seen_message_ids = BTreeSet::new();
    let mut previous_identity = None::<(u64, String)>;
    let mut imported = Vec::new();
    for chunk_index in 0..pending.receipt.chunk_count {
        let chunk = pending.chunks.get(&chunk_index)?;
        transfer_digest.update(device_link_bootstrap_chunk_sha256(chunk));
        for history in chunk {
            history.validate_limits().ok()?;
            let identity = (history.seq, history.message_id.clone());
            if history.seq >= pending.earliest_envelope_seq
                || !seen_message_ids.insert(history.message_id.clone())
                || previous_identity
                    .as_ref()
                    .is_some_and(|previous| previous >= &identity)
            {
                return None;
            }
            previous_identity = Some(identity);
            let event =
                serde_json::from_slice::<DecryptedApplicationEventV1>(&history.plaintext).ok()?;
            event.validate_limits().ok()?;
            if matches!(
                event.kind,
                DurableAppEventKind::Namespaced { ref name, .. }
                    if name == FINITECHAT_DEVICE_LINK_BOOTSTRAP_EVENT_V2
                        || name == LEGACY_DEVICE_LINK_BOOTSTRAP_REQUEST_EVENT_V2
            ) {
                return None;
            }
            let stored = StoredAppEvent {
                room_id: pending.receipt.room_id.clone(),
                seq: history.seq,
                message_id: history.message_id.clone(),
                sender: history.sender.clone(),
                plaintext: history.plaintext.clone(),
                timestamp_unix_seconds: history.timestamp_unix_seconds,
            };
            stored.validate_limits().ok()?;
            imported.push(stored);
        }
    }
    if imported.len() as u64 != pending.receipt.total_history_events
        || hex_lower(&transfer_digest.finalize()) != pending.receipt.history_sha256
    {
        return None;
    }
    Some(imported)
}

fn commit_device_link_bootstrap_tx(
    tx: &Transaction<'_>,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    expected: &StoredDeviceLinkBootstrapReceipt,
    room: &StoredAppRoom,
    profiles: &[StoredAppProfile],
) -> Result<DeviceLinkBootstrapCommitOutcome, ClientStoreError> {
    let identity = device_link_bootstrap_identity_from_receipt(owner, expected);
    let Some(row) = load_device_link_bootstrap_transfer_row(tx, encryption_key, identity)? else {
        return Ok(DeviceLinkBootstrapCommitOutcome::Incomplete);
    };
    let stored_receipt = row.manifest.receipt()?;
    if stored_receipt != *expected || row.manifest_sha256 != expected.manifest_sha256 {
        poison_device_link_bootstrap_identity_tx(tx, identity)?;
        return Ok(DeviceLinkBootstrapCommitOutcome::Poisoned);
    }
    match row.state {
        DEVICE_LINK_BOOTSTRAP_STATE_POISONED | DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED_POISONED => {
            return Ok(DeviceLinkBootstrapCommitOutcome::Poisoned);
        }
        DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED => {
            return Ok(DeviceLinkBootstrapCommitOutcome::AlreadyCommitted(
                stored_receipt,
            ));
        }
        DEVICE_LINK_BOOTSTRAP_STATE_RECEIVING => {}
        state => return Err(ClientStoreError::InvalidDeviceLinkBootstrapState { state }),
    }
    if expected.target != *owner
        || expected.source.account_id != owner.account_id
        || room.display_name != row.manifest.room.display_name
        || room.picture != row.manifest.room.picture
    {
        poison_device_link_bootstrap_identity_tx(tx, identity)?;
        return Ok(DeviceLinkBootstrapCommitOutcome::Poisoned);
    }
    let pending = load_pending_device_link_bootstrap(tx, encryption_key, owner, expected)?
        .ok_or(ClientStoreError::DeviceLinkBootstrapReceiptMismatch)?;
    if !pending.is_complete() {
        return Ok(DeviceLinkBootstrapCommitOutcome::Incomplete);
    }
    let Some(imported) = validated_device_link_bootstrap_events(&pending) else {
        poison_device_link_bootstrap_identity_tx(tx, identity)?;
        return Ok(DeviceLinkBootstrapCommitOutcome::Poisoned);
    };
    if let Err(error) = validate_app_event_import_tx(tx, encryption_key, owner, &imported) {
        if matches!(error, ClientStoreError::StoredAppEventConflict { .. }) {
            poison_device_link_bootstrap_identity_tx(tx, identity)?;
            return Ok(DeviceLinkBootstrapCommitOutcome::Poisoned);
        }
        return Err(error);
    }

    save_app_events_tx(tx, encryption_key, owner, &imported)?;
    save_app_rooms_tx(tx, encryption_key, owner, std::slice::from_ref(room))?;
    save_app_profiles_tx(tx, encryption_key, owner, profiles)?;
    tx.execute(
        r#"
        DELETE FROM client_device_link_bootstrap_transfers
        WHERE account_id = ?1
          AND device_id = ?2
          AND room_id = ?3
          AND NOT (
            bootstrap_id = ?4
            AND source_account_id = ?5
            AND source_device_id = ?6
          )
        "#,
        params![
            &owner.account_id,
            &owner.device_id,
            &expected.room_id,
            &expected.bootstrap_id,
            &expected.source.account_id,
            &expected.source.device_id,
        ],
    )?;
    tx.execute(
        r#"
        UPDATE client_device_link_bootstrap_transfers
        SET state = ?7
        WHERE account_id = ?1
          AND device_id = ?2
          AND room_id = ?3
          AND bootstrap_id = ?4
          AND source_account_id = ?5
          AND source_device_id = ?6
        "#,
        params![
            &owner.account_id,
            &owner.device_id,
            &expected.room_id,
            &expected.bootstrap_id,
            &expected.source.account_id,
            &expected.source.device_id,
            DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED,
        ],
    )?;
    // Keep only the canonical per-chunk digests after publication. They are
    // enough to distinguish an exact replay from a conflicting duplicate
    // without retaining a second full copy of the imported room history.
    tx.execute(
        r#"
        UPDATE client_device_link_bootstrap_chunks
        SET nonce = NULL, ciphertext = NULL
        WHERE account_id = ?1
          AND device_id = ?2
          AND room_id = ?3
          AND bootstrap_id = ?4
          AND source_account_id = ?5
          AND source_device_id = ?6
        "#,
        params![
            &owner.account_id,
            &owner.device_id,
            &expected.room_id,
            &expected.bootstrap_id,
            &expected.source.account_id,
            &expected.source.device_id,
        ],
    )?;
    Ok(DeviceLinkBootstrapCommitOutcome::Committed(stored_receipt))
}

fn load_completed_device_link_bootstrap_receipts(
    conn: &Connection,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
) -> Result<Vec<StoredDeviceLinkBootstrapReceipt>, ClientStoreError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT room_id, bootstrap_id, source_account_id, source_device_id
        FROM client_device_link_bootstrap_transfers
        WHERE account_id = ?1
          AND device_id = ?2
          AND state IN (?3, ?4)
        ORDER BY room_id, bootstrap_id, source_account_id, source_device_id
        "#,
    )?;
    let keys = stmt
        .query_map(
            params![
                &owner.account_id,
                &owner.device_id,
                DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED,
                DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED_POISONED,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    DeviceRef {
                        account_id: row.get::<_, String>(2)?,
                        device_id: row.get::<_, String>(3)?,
                    },
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut receipts = Vec::with_capacity(keys.len());
    for (room_id, bootstrap_id, source) in keys {
        if let Some(receipt) = load_completed_device_link_bootstrap_receipt(
            conn,
            encryption_key,
            owner,
            &room_id,
            &bootstrap_id,
            &source,
        )? {
            receipts.push(receipt);
        }
    }
    receipts.sort();
    Ok(receipts)
}

fn load_completed_device_link_bootstrap_receipt(
    conn: &Connection,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    room_id: &str,
    bootstrap_id: &str,
    source: &DeviceRef,
) -> Result<Option<StoredDeviceLinkBootstrapReceipt>, ClientStoreError> {
    let identity = DeviceLinkBootstrapIdentity {
        owner,
        room_id,
        bootstrap_id,
        source,
    };
    let Some(row) = load_device_link_bootstrap_transfer_row(conn, encryption_key, identity)? else {
        return Ok(None);
    };
    if !matches!(
        row.state,
        DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED | DEVICE_LINK_BOOTSTRAP_STATE_COMMITTED_POISONED
    ) {
        return Ok(None);
    }
    Ok(Some(row.manifest.receipt()?))
}

fn save_app_messages_tx(
    tx: &Transaction<'_>,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    messages: &[StoredAppMessage],
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    let mut stmt = tx.prepare(
        r#"
        INSERT INTO client_app_messages (
          account_id,
          device_id,
          room_id,
          seq,
          message_id,
          sender_account_id,
          sender_device_id,
          timestamp_unix_seconds,
          nonce,
          ciphertext
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(account_id, device_id, room_id, message_id) DO UPDATE SET
          seq = excluded.seq,
          sender_account_id = excluded.sender_account_id,
          sender_device_id = excluded.sender_device_id,
          timestamp_unix_seconds = excluded.timestamp_unix_seconds,
          nonce = excluded.nonce,
          ciphertext = excluded.ciphertext
        "#,
    )?;
    for message in messages {
        message.validate_limits()?;
        let sealed = encrypt_app_message_plaintext(encryption_key, owner, message)?;
        stmt.execute(params![
            &owner.account_id,
            &owner.device_id,
            &message.room_id,
            sqlite_seq_from_u64(message.seq)?,
            &message.message_id,
            &message.sender.account_id,
            &message.sender.device_id,
            sqlite_timestamp_from_u64(message.timestamp_unix_seconds)?,
            &sealed.nonce,
            &sealed.ciphertext,
        ])?;
    }
    Ok(())
}

fn save_app_events_tx(
    tx: &Transaction<'_>,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    events: &[StoredAppEvent],
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    let mut stmt = tx.prepare(
        r#"
        INSERT INTO client_app_events (
          account_id,
          device_id,
          room_id,
          seq,
          message_id,
          sender_account_id,
          sender_device_id,
          timestamp_unix_seconds,
          nonce,
          ciphertext
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(account_id, device_id, room_id, message_id) DO UPDATE SET
          seq = excluded.seq,
          sender_account_id = excluded.sender_account_id,
          sender_device_id = excluded.sender_device_id,
          timestamp_unix_seconds = excluded.timestamp_unix_seconds,
          nonce = excluded.nonce,
          ciphertext = excluded.ciphertext
        "#,
    )?;
    for event in events {
        event.validate_limits()?;
        let sealed = encrypt_app_event_plaintext(encryption_key, owner, event)?;
        stmt.execute(params![
            &owner.account_id,
            &owner.device_id,
            &event.room_id,
            sqlite_app_event_seq_from_u64(event.seq)?,
            &event.message_id,
            &event.sender.account_id,
            &event.sender.device_id,
            sqlite_timestamp_from_u64(event.timestamp_unix_seconds)?,
            &sealed.nonce,
            &sealed.ciphertext,
        ])?;
    }
    Ok(())
}

fn import_app_events_tx(
    tx: &Transaction<'_>,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    events: &[StoredAppEvent],
) -> Result<(), ClientStoreError> {
    validate_app_event_import_tx(tx, encryption_key, owner, events)?;
    save_app_events_tx(tx, encryption_key, owner, events)
}

fn validate_app_event_import_tx(
    tx: &Transaction<'_>,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    events: &[StoredAppEvent],
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    let mut seen = BTreeSet::new();
    for event in events {
        event.validate_limits()?;
        if !seen.insert((event.room_id.clone(), event.message_id.clone())) {
            return Err(ClientStoreError::StoredAppEventConflict {
                room_id: event.room_id.clone(),
                message_id: event.message_id.clone(),
            });
        }
        let existing = tx
            .query_row(
                r#"
                SELECT
                  seq,
                  sender_account_id,
                  sender_device_id,
                  timestamp_unix_seconds,
                  nonce,
                  ciphertext
                FROM client_app_events
                WHERE account_id = ?1
                  AND device_id = ?2
                  AND room_id = ?3
                  AND message_id = ?4
                "#,
                params![
                    &owner.account_id,
                    &owner.device_id,
                    &event.room_id,
                    &event.message_id,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            stored_seq,
            sender_account_id,
            sender_device_id,
            stored_timestamp,
            nonce,
            ciphertext,
        )) = existing
        else {
            continue;
        };
        let seq = sqlite_app_event_seq_to_u64(stored_seq)?;
        let timestamp_unix_seconds = sqlite_timestamp_to_u64(stored_timestamp)?;
        let sender = DeviceRef {
            account_id: sender_account_id,
            device_id: sender_device_id,
        };
        let plaintext = decrypt_app_event_plaintext(
            encryption_key,
            AppMessageIdentity {
                owner,
                room_id: &event.room_id,
                seq,
                message_id: &event.message_id,
                sender: &sender,
            },
            &nonce,
            &ciphertext,
        )?;
        let canonical = StoredAppEvent {
            room_id: event.room_id.clone(),
            seq,
            message_id: event.message_id.clone(),
            sender,
            plaintext,
            timestamp_unix_seconds,
        };
        if canonical != *event {
            return Err(ClientStoreError::StoredAppEventConflict {
                room_id: event.room_id.clone(),
                message_id: event.message_id.clone(),
            });
        }
    }
    Ok(())
}

fn save_app_outbox_tx(
    tx: &Transaction<'_>,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    messages: &[StoredOutboundMessage],
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    let mut stmt = tx.prepare(
        r#"
        INSERT INTO client_app_outbox (
          account_id,
          device_id,
          room_id,
          message_id,
          nonce,
          ciphertext
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(account_id, device_id, room_id, message_id) DO UPDATE SET
          nonce = excluded.nonce,
          ciphertext = excluded.ciphertext
        "#,
    )?;
    for message in messages {
        message.validate_limits()?;
        let sealed = encrypt_app_outbox_metadata(encryption_key, owner, message)?;
        stmt.execute(params![
            &owner.account_id,
            &owner.device_id,
            &message.room_id,
            &message.message_id,
            &sealed.nonce,
            &sealed.ciphertext,
        ])?;
    }
    Ok(())
}

fn save_app_rooms_tx(
    tx: &Transaction<'_>,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    rooms: &[StoredAppRoom],
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    let mut stmt = tx.prepare(
        r#"
        INSERT INTO client_app_rooms (
          account_id,
          device_id,
          room_id,
          nonce,
          ciphertext
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(account_id, device_id, room_id) DO UPDATE SET
          nonce = excluded.nonce,
          ciphertext = excluded.ciphertext
        "#,
    )?;
    for room in rooms {
        room.validate_limits()?;
        let sealed = encrypt_app_room_metadata(encryption_key, owner, room)?;
        stmt.execute(params![
            &owner.account_id,
            &owner.device_id,
            &room.room_id,
            &sealed.nonce,
            &sealed.ciphertext,
        ])?;
    }
    Ok(())
}

fn save_app_state_tx(
    tx: &Transaction<'_>,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    state: &StoredAppState,
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    state.validate_limits()?;
    let sealed = encrypt_app_state_metadata(encryption_key, owner, state)?;
    tx.execute(
        r#"
        INSERT INTO client_app_state (
          account_id,
          device_id,
          nonce,
          ciphertext
        ) VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(account_id, device_id) DO UPDATE SET
          nonce = excluded.nonce,
          ciphertext = excluded.ciphertext
        "#,
        params![
            &owner.account_id,
            &owner.device_id,
            &sealed.nonce,
            &sealed.ciphertext,
        ],
    )?;
    Ok(())
}

fn save_app_profiles_tx(
    tx: &Transaction<'_>,
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    profiles: &[StoredAppProfile],
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    validate_item_count("app_profiles", profiles.len(), MAX_STORED_APP_PROFILES)
        .map_err(ClientError::from)?;
    let mut stmt = tx.prepare(
        r#"
        INSERT INTO client_app_profiles (
          account_id,
          device_id,
          profile_account_id,
          nonce,
          ciphertext
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(account_id, device_id, profile_account_id) DO UPDATE SET
          nonce = excluded.nonce,
          ciphertext = excluded.ciphertext
        "#,
    )?;
    for profile in profiles {
        profile.validate_limits()?;
        let sealed = encrypt_app_profile_metadata(encryption_key, owner, profile)?;
        stmt.execute(params![
            &owner.account_id,
            &owner.device_id,
            &profile.profile.account_id,
            &sealed.nonce,
            &sealed.ciphertext,
        ])?;
    }
    Ok(())
}

fn prune_app_messages_tx(
    tx: &Transaction<'_>,
    owner: &DeviceRef,
    max_messages: u32,
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    validate_app_message_limit(max_messages)?;
    let count = tx.query_row(
        r#"
        SELECT COUNT(*)
        FROM client_app_messages
        WHERE account_id = ?1 AND device_id = ?2
        "#,
        params![&owner.account_id, &owner.device_id],
        |row| row.get::<_, u64>(0),
    )?;
    let excess = count.saturating_sub(u64::from(max_messages));
    if excess == 0 {
        return Ok(());
    }
    let limit =
        i64::try_from(excess).map_err(|_| ClientStoreError::StoredAppMessageCountOverflow)?;
    tx.execute(
        r#"
        DELETE FROM client_app_messages
        WHERE rowid IN (
          SELECT rowid
          FROM client_app_messages
          WHERE account_id = ?1 AND device_id = ?2
          ORDER BY rowid ASC
          LIMIT ?3
        )
        "#,
        params![&owner.account_id, &owner.device_id, limit],
    )?;
    Ok(())
}

fn prune_app_events_tx(
    tx: &Transaction<'_>,
    owner: &DeviceRef,
    max_events: u32,
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    validate_app_event_limit(max_events)?;
    let count = tx.query_row(
        r#"
        SELECT COUNT(*)
        FROM client_app_events
        WHERE account_id = ?1 AND device_id = ?2
        "#,
        params![&owner.account_id, &owner.device_id],
        |row| row.get::<_, u64>(0),
    )?;
    let excess = count.saturating_sub(u64::from(max_events));
    if excess == 0 {
        return Ok(());
    }
    let limit = i64::try_from(excess).map_err(|_| ClientStoreError::StoredAppEventCountOverflow)?;
    tx.execute(
        r#"
        DELETE FROM client_app_events
        WHERE rowid IN (
          SELECT rowid
          FROM client_app_events
          WHERE account_id = ?1 AND device_id = ?2
          ORDER BY rowid ASC
          LIMIT ?3
        )
        "#,
        params![&owner.account_id, &owner.device_id, limit],
    )?;
    Ok(())
}

fn prune_app_outbox_tx(
    tx: &Transaction<'_>,
    owner: &DeviceRef,
    max_messages: u32,
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    let count = tx.query_row(
        r#"
        SELECT COUNT(*)
        FROM client_app_outbox
        WHERE account_id = ?1 AND device_id = ?2
        "#,
        params![&owner.account_id, &owner.device_id],
        |row| row.get::<_, u64>(0),
    )?;
    let excess = count.saturating_sub(u64::from(max_messages));
    if excess == 0 {
        return Ok(());
    }
    let limit =
        i64::try_from(excess).map_err(|_| ClientStoreError::StoredAppOutboxCountOverflow)?;
    tx.execute(
        r#"
        DELETE FROM client_app_outbox
        WHERE rowid IN (
          SELECT rowid
          FROM client_app_outbox
          WHERE account_id = ?1 AND device_id = ?2
          ORDER BY rowid ASC
          LIMIT ?3
        )
        "#,
        params![&owner.account_id, &owner.device_id, limit],
    )?;
    Ok(())
}

fn prune_app_profiles_tx(
    tx: &Transaction<'_>,
    owner: &DeviceRef,
    max_profiles: u32,
) -> Result<(), ClientStoreError> {
    validate_app_message_owner(owner)?;
    let count = tx.query_row(
        r#"
        SELECT COUNT(*)
        FROM client_app_profiles
        WHERE account_id = ?1 AND device_id = ?2
        "#,
        params![&owner.account_id, &owner.device_id],
        |row| row.get::<_, u64>(0),
    )?;
    let excess = count.saturating_sub(u64::from(max_profiles));
    if excess == 0 {
        return Ok(());
    }
    let limit =
        i64::try_from(excess).map_err(|_| ClientStoreError::StoredAppProfileCountOverflow)?;
    tx.execute(
        r#"
        DELETE FROM client_app_profiles
        WHERE rowid IN (
          SELECT rowid
          FROM client_app_profiles
          WHERE account_id = ?1 AND device_id = ?2
          ORDER BY rowid ASC
          LIMIT ?3
        )
        "#,
        params![&owner.account_id, &owner.device_id, limit],
    )?;
    Ok(())
}

fn load_device_state(
    conn: &Connection,
    encryption_key: &ClientStoreEncryptionKey,
    account_id: &str,
    device_id: &str,
) -> Result<Option<FiniteChatDeviceState>, ClientStoreError> {
    validate_string_bytes("account_id", account_id, MAX_ACCOUNT_ID_BYTES)
        .map_err(ClientError::from)?;
    validate_string_bytes("device_id", device_id, MAX_DEVICE_ID_BYTES)
        .map_err(ClientError::from)?;
    let sealed = conn
        .query_row(
            r#"
            SELECT nonce, ciphertext
            FROM client_device_states
            WHERE account_id = ?1 AND device_id = ?2
            "#,
            params![account_id, device_id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    let Some((nonce, ciphertext)) = sealed else {
        return Ok(None);
    };

    let state = decrypt_device_state(encryption_key, account_id, device_id, &nonce, &ciphertext)?;
    if state.device_ref.account_id != account_id || state.device_ref.device_id != device_id {
        return Err(ClientStoreError::StateSnapshotIdentityMismatch);
    }
    state.validate_limits()?;
    Ok(Some(state))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedClientState {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedAppMessage {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedAppEvent {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedAppOutbox {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedAppRoom {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedAppState {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedAppProfile {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedDeviceLinkBootstrap {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Clone, Copy)]
struct AppMessageIdentity<'a> {
    owner: &'a DeviceRef,
    room_id: &'a str,
    seq: u64,
    message_id: &'a str,
    sender: &'a DeviceRef,
}

#[derive(Clone, Copy)]
struct AppOutboxIdentity<'a> {
    owner: &'a DeviceRef,
    room_id: &'a str,
    message_id: &'a str,
}

#[derive(Clone, Copy)]
struct AppRoomIdentity<'a> {
    owner: &'a DeviceRef,
    room_id: &'a str,
}

#[derive(Clone, Copy)]
struct AppStateIdentity<'a> {
    owner: &'a DeviceRef,
}

#[derive(Clone, Copy)]
struct AppProfileIdentity<'a> {
    owner: &'a DeviceRef,
    profile_account_id: &'a str,
}

fn encrypt_device_state(
    encryption_key: &ClientStoreEncryptionKey,
    state: &FiniteChatDeviceState,
) -> Result<SealedClientState, ClientStoreError> {
    state.validate_limits()?;
    let plaintext = encode_device_state(state)?;
    let aad = client_store_aad(
        CLIENT_STATE_SNAPSHOT_VERSION,
        &state.device_ref.account_id,
        &state.device_ref.device_id,
    )?;
    let provider = OpenMlsRustCrypto::default();
    let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider
        .rand()
        .random_array()
        .map_err(|_| ClientStoreError::Randomness)?;
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            &plaintext,
            &nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::EncryptState)?;
    validate_bytes_len(
        "client_state.ciphertext",
        ciphertext.len(),
        MAX_CLIENT_STATE_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    debug_assert_eq!(nonce.len(), CLIENT_STORE_NONCE_BYTES);
    debug_assert!(!ciphertext.is_empty());
    Ok(SealedClientState {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn encrypt_app_message_plaintext(
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    message: &StoredAppMessage,
) -> Result<SealedAppMessage, ClientStoreError> {
    validate_app_message_owner(owner)?;
    message.validate_limits()?;
    let identity = AppMessageIdentity {
        owner,
        room_id: &message.room_id,
        seq: message.seq,
        message_id: &message.message_id,
        sender: &message.sender,
    };
    let aad = app_message_aad(identity)?;
    let provider = OpenMlsRustCrypto::default();
    let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider
        .rand()
        .random_array()
        .map_err(|_| ClientStoreError::Randomness)?;
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            &message.plaintext,
            &nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::EncryptAppMessage)?;
    validate_bytes_len(
        "app_message.ciphertext",
        ciphertext.len(),
        MAX_APP_MESSAGE_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(SealedAppMessage {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn encrypt_app_event_plaintext(
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    event: &StoredAppEvent,
) -> Result<SealedAppEvent, ClientStoreError> {
    validate_app_message_owner(owner)?;
    event.validate_limits()?;
    let identity = AppMessageIdentity {
        owner,
        room_id: &event.room_id,
        seq: event.seq,
        message_id: &event.message_id,
        sender: &event.sender,
    };
    let aad = app_event_aad(identity)?;
    let provider = OpenMlsRustCrypto::default();
    let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider
        .rand()
        .random_array()
        .map_err(|_| ClientStoreError::Randomness)?;
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            &event.plaintext,
            &nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::EncryptAppEvent)?;
    validate_bytes_len(
        "app_event.ciphertext",
        ciphertext.len(),
        MAX_APP_EVENT_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(SealedAppEvent {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn encrypt_app_outbox_metadata(
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    message: &StoredOutboundMessage,
) -> Result<SealedAppOutbox, ClientStoreError> {
    validate_app_message_owner(owner)?;
    message.validate_limits()?;
    let metadata = StoredOutboundMessageMetadataV1 {
        sender: message.sender.clone(),
        plaintext: message.plaintext.clone(),
        local_state: message.local_state.clone(),
        server_delivery_state: message.server_delivery_state.clone(),
        append_request: message.append_request.clone(),
        timestamp_unix_seconds: message.timestamp_unix_seconds,
    };
    let plaintext =
        serde_json::to_vec(&metadata).map_err(|_| ClientStoreError::EncodeAppOutboxMetadata)?;
    validate_bytes_len(
        "app_outbox.metadata",
        plaintext.len(),
        MAX_APP_OUTBOX_METADATA_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = app_outbox_aad(AppOutboxIdentity {
        owner,
        room_id: &message.room_id,
        message_id: &message.message_id,
    })?;
    let provider = OpenMlsRustCrypto::default();
    let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider
        .rand()
        .random_array()
        .map_err(|_| ClientStoreError::Randomness)?;
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            &plaintext,
            &nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::EncryptAppOutbox)?;
    validate_bytes_len(
        "app_outbox.ciphertext",
        ciphertext.len(),
        MAX_APP_OUTBOX_METADATA_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(SealedAppOutbox {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn encrypt_app_room_metadata(
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    room: &StoredAppRoom,
) -> Result<SealedAppRoom, ClientStoreError> {
    validate_app_message_owner(owner)?;
    room.validate_limits()?;
    let metadata = StoredAppRoomMetadataV1 {
        display_name: room.display_name.clone(),
        picture: room.picture.clone(),
        state: room.state,
        status: room.status.clone(),
        local_read_seq: room.local_read_seq,
    };
    let plaintext =
        serde_json::to_vec(&metadata).map_err(|_| ClientStoreError::EncodeAppRoomMetadata)?;
    validate_bytes_len(
        "app_room.metadata",
        plaintext.len(),
        MAX_APP_ROOM_METADATA_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = app_room_aad(AppRoomIdentity {
        owner,
        room_id: &room.room_id,
    })?;
    let provider = OpenMlsRustCrypto::default();
    let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider
        .rand()
        .random_array()
        .map_err(|_| ClientStoreError::Randomness)?;
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            &plaintext,
            &nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::EncryptAppRoom)?;
    validate_bytes_len(
        "app_room.ciphertext",
        ciphertext.len(),
        MAX_APP_ROOM_METADATA_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(SealedAppRoom {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn encrypt_app_state_metadata(
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    state: &StoredAppState,
) -> Result<SealedAppState, ClientStoreError> {
    validate_app_message_owner(owner)?;
    state.validate_limits()?;
    let metadata = StoredAppStateMetadataV1 {
        selected_room_id: state.selected_room_id.clone(),
        selected_topic_id: state.selected_topic_id.clone(),
        selected_chat_id: state.selected_chat_id.clone(),
        paired_agent: state.paired_agent.clone(),
        revoked_devices: state.revoked_devices.clone(),
        chat_archives: state.chat_archives.clone(),
    };
    let plaintext =
        serde_json::to_vec(&metadata).map_err(|_| ClientStoreError::EncodeAppStateMetadata)?;
    validate_bytes_len(
        "app_state.metadata",
        plaintext.len(),
        MAX_APP_STATE_METADATA_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = app_state_aad(AppStateIdentity { owner })?;
    let provider = OpenMlsRustCrypto::default();
    let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider
        .rand()
        .random_array()
        .map_err(|_| ClientStoreError::Randomness)?;
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            &plaintext,
            &nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::EncryptAppState)?;
    validate_bytes_len(
        "app_state.ciphertext",
        ciphertext.len(),
        MAX_APP_STATE_METADATA_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(SealedAppState {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn encrypt_app_profile_metadata(
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    profile: &StoredAppProfile,
) -> Result<SealedAppProfile, ClientStoreError> {
    validate_app_message_owner(owner)?;
    profile.validate_limits()?;
    let metadata = StoredAppProfileMetadataV1 {
        profile: profile.profile.clone(),
        stale: profile.stale,
    };
    let plaintext =
        serde_json::to_vec(&metadata).map_err(|_| ClientStoreError::EncodeAppProfileMetadata)?;
    validate_bytes_len(
        "app_profile.metadata",
        plaintext.len(),
        MAX_APP_PROFILE_METADATA_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = app_profile_aad(AppProfileIdentity {
        owner,
        profile_account_id: &profile.profile.account_id,
    })?;
    let provider = OpenMlsRustCrypto::default();
    let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider
        .rand()
        .random_array()
        .map_err(|_| ClientStoreError::Randomness)?;
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            &plaintext,
            &nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::EncryptAppProfile)?;
    validate_bytes_len(
        "app_profile.ciphertext",
        ciphertext.len(),
        MAX_APP_PROFILE_METADATA_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(SealedAppProfile {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn encrypt_device_link_bootstrap_manifest(
    encryption_key: &ClientStoreEncryptionKey,
    identity: DeviceLinkBootstrapIdentity<'_>,
    manifest_sha256: &str,
    manifest: &StoredDeviceLinkBootstrapManifestV2,
) -> Result<SealedDeviceLinkBootstrap, ClientStoreError> {
    let plaintext = manifest.canonical_bytes()?;
    validate_bytes_len(
        "device_link_bootstrap.manifest",
        plaintext.len(),
        MAX_DEVICE_LINK_BOOTSTRAP_MANIFEST_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = device_link_bootstrap_aad(
        CLIENT_DEVICE_LINK_BOOTSTRAP_MANIFEST_AAD_DOMAIN,
        identity,
        manifest_sha256,
        None,
    )?;
    let provider = OpenMlsRustCrypto::default();
    let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider
        .rand()
        .random_array()
        .map_err(|_| ClientStoreError::Randomness)?;
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            &plaintext,
            &nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::EncryptDeviceLinkBootstrapManifest)?;
    validate_bytes_len(
        "device_link_bootstrap.manifest_ciphertext",
        ciphertext.len(),
        MAX_DEVICE_LINK_BOOTSTRAP_MANIFEST_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(SealedDeviceLinkBootstrap {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn encrypt_device_link_bootstrap_chunk(
    encryption_key: &ClientStoreEncryptionKey,
    identity: DeviceLinkBootstrapIdentity<'_>,
    manifest_sha256: &str,
    chunk_index: u32,
    history: &[DeviceLinkBootstrapEventV2],
) -> Result<SealedDeviceLinkBootstrap, ClientStoreError> {
    let plaintext = serde_json::to_vec(history)
        .map_err(|_| ClientStoreError::EncodeDeviceLinkBootstrapChunk)?;
    validate_bytes_len(
        "device_link_bootstrap.chunk",
        plaintext.len(),
        MAX_DEVICE_LINK_BOOTSTRAP_CHUNK_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = device_link_bootstrap_aad(
        CLIENT_DEVICE_LINK_BOOTSTRAP_CHUNK_AAD_DOMAIN,
        identity,
        manifest_sha256,
        Some(chunk_index),
    )?;
    let provider = OpenMlsRustCrypto::default();
    let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider
        .rand()
        .random_array()
        .map_err(|_| ClientStoreError::Randomness)?;
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            &plaintext,
            &nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::EncryptDeviceLinkBootstrapChunk)?;
    validate_bytes_len(
        "device_link_bootstrap.chunk_ciphertext",
        ciphertext.len(),
        MAX_DEVICE_LINK_BOOTSTRAP_CHUNK_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(SealedDeviceLinkBootstrap {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn decrypt_device_state(
    encryption_key: &ClientStoreEncryptionKey,
    account_id: &str,
    device_id: &str,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<FiniteChatDeviceState, ClientStoreError> {
    if nonce.len() != CLIENT_STORE_NONCE_BYTES {
        return Err(ClientStoreError::InvalidNonceLength {
            actual_bytes: nonce.len(),
        });
    }
    validate_bytes_non_empty("client_state.ciphertext", ciphertext.len())
        .map_err(ClientError::from)?;
    validate_bytes_len(
        "client_state.ciphertext",
        ciphertext.len(),
        MAX_CLIENT_STATE_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let provider = OpenMlsRustCrypto::default();
    let mut plaintext = None;
    for version in [
        CLIENT_STATE_SNAPSHOT_VERSION,
        LEGACY_CLIENT_STATE_SNAPSHOT_VERSION,
    ] {
        let aad = client_store_aad(version, account_id, device_id)?;
        if let Ok(opened) = provider.crypto().aead_decrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            ciphertext,
            nonce,
            &aad,
        ) {
            plaintext = Some(opened);
            break;
        }
    }
    let plaintext = plaintext.ok_or(ClientStoreError::DecryptState)?;
    decode_device_state(&plaintext)
}

fn decrypt_app_message_plaintext(
    encryption_key: &ClientStoreEncryptionKey,
    identity: AppMessageIdentity<'_>,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, ClientStoreError> {
    if nonce.len() != CLIENT_STORE_NONCE_BYTES {
        return Err(ClientStoreError::InvalidAppMessageNonceLength {
            actual_bytes: nonce.len(),
        });
    }
    validate_bytes_non_empty("app_message.ciphertext", ciphertext.len())
        .map_err(ClientError::from)?;
    validate_bytes_len(
        "app_message.ciphertext",
        ciphertext.len(),
        MAX_APP_MESSAGE_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = app_message_aad(identity)?;
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            ciphertext,
            nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::DecryptAppMessage)?;
    validate_bytes_len(
        "app_message.plaintext",
        plaintext.len(),
        MAX_ENVELOPE_PAYLOAD_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(plaintext)
}

fn decrypt_app_event_plaintext(
    encryption_key: &ClientStoreEncryptionKey,
    identity: AppMessageIdentity<'_>,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, ClientStoreError> {
    if nonce.len() != CLIENT_STORE_NONCE_BYTES {
        return Err(ClientStoreError::InvalidAppEventNonceLength {
            actual_bytes: nonce.len(),
        });
    }
    validate_bytes_non_empty("app_event.ciphertext", ciphertext.len())
        .map_err(ClientError::from)?;
    validate_bytes_len(
        "app_event.ciphertext",
        ciphertext.len(),
        MAX_APP_EVENT_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = app_event_aad(identity)?;
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            ciphertext,
            nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::DecryptAppEvent)?;
    validate_bytes_len(
        "app_event.plaintext",
        plaintext.len(),
        MAX_ENVELOPE_PAYLOAD_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(plaintext)
}

fn decrypt_app_outbox_metadata(
    encryption_key: &ClientStoreEncryptionKey,
    identity: AppOutboxIdentity<'_>,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<StoredOutboundMessageMetadataV1, ClientStoreError> {
    if nonce.len() != CLIENT_STORE_NONCE_BYTES {
        return Err(ClientStoreError::InvalidAppOutboxNonceLength {
            actual_bytes: nonce.len(),
        });
    }
    validate_bytes_non_empty("app_outbox.ciphertext", ciphertext.len())
        .map_err(ClientError::from)?;
    validate_bytes_len(
        "app_outbox.ciphertext",
        ciphertext.len(),
        MAX_APP_OUTBOX_METADATA_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = app_outbox_aad(identity)?;
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            ciphertext,
            nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::DecryptAppOutbox)?;
    validate_bytes_len(
        "app_outbox.metadata",
        plaintext.len(),
        MAX_APP_OUTBOX_METADATA_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let metadata = serde_json::from_slice::<StoredOutboundMessageMetadataV1>(&plaintext)
        .map_err(|_| ClientStoreError::DecodeAppOutboxMetadata)?;
    metadata
        .sender
        .validate_limits()
        .map_err(ClientError::from)?;
    validate_bytes_len(
        "app_outbox.plaintext",
        metadata.plaintext.len(),
        MAX_ENVELOPE_PAYLOAD_BYTES,
    )
    .map_err(ClientError::from)?;
    metadata.local_state.validate_limits()?;
    metadata.server_delivery_state.validate_limits()?;
    metadata
        .append_request
        .validate_limits()
        .map_err(ClientError::from)?;
    Ok(metadata)
}

fn decrypt_app_room_metadata(
    encryption_key: &ClientStoreEncryptionKey,
    identity: AppRoomIdentity<'_>,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<StoredAppRoomMetadataV1, ClientStoreError> {
    if nonce.len() != CLIENT_STORE_NONCE_BYTES {
        return Err(ClientStoreError::InvalidAppRoomNonceLength {
            actual_bytes: nonce.len(),
        });
    }
    validate_bytes_non_empty("app_room.ciphertext", ciphertext.len()).map_err(ClientError::from)?;
    validate_bytes_len(
        "app_room.ciphertext",
        ciphertext.len(),
        MAX_APP_ROOM_METADATA_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = app_room_aad(identity)?;
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            ciphertext,
            nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::DecryptAppRoom)?;
    validate_bytes_len(
        "app_room.metadata",
        plaintext.len(),
        MAX_APP_ROOM_METADATA_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let metadata = serde_json::from_slice::<StoredAppRoomMetadataV1>(&plaintext)
        .map_err(|_| ClientStoreError::DecodeAppRoomMetadata)?;
    validate_string_bytes(
        "app_room.display_name",
        &metadata.display_name,
        MAX_APP_ROOM_DISPLAY_NAME_BYTES,
    )
    .map_err(ClientError::from)?;
    validate_bytes_non_empty("app_room.display_name", metadata.display_name.len())
        .map_err(ClientError::from)?;
    validate_string_bytes(
        "app_room.status",
        &metadata.status,
        MAX_APP_ROOM_STATUS_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(metadata)
}

fn decrypt_app_state_metadata(
    encryption_key: &ClientStoreEncryptionKey,
    owner: &DeviceRef,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<StoredAppStateMetadataV1, ClientStoreError> {
    if nonce.len() != CLIENT_STORE_NONCE_BYTES {
        return Err(ClientStoreError::InvalidAppStateNonceLength {
            actual_bytes: nonce.len(),
        });
    }
    validate_bytes_non_empty("app_state.ciphertext", ciphertext.len())
        .map_err(ClientError::from)?;
    validate_bytes_len(
        "app_state.ciphertext",
        ciphertext.len(),
        MAX_APP_STATE_METADATA_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = app_state_aad(AppStateIdentity { owner })?;
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            ciphertext,
            nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::DecryptAppState)?;
    validate_bytes_len(
        "app_state.metadata",
        plaintext.len(),
        MAX_APP_STATE_METADATA_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let metadata = serde_json::from_slice::<StoredAppStateMetadataV1>(&plaintext)
        .map_err(|_| ClientStoreError::DecodeAppStateMetadata)?;
    if let Some(room_id) = &metadata.selected_room_id {
        validate_room_id(room_id).map_err(ClientError::from)?;
    }
    if let Some(topic_id) = &metadata.selected_topic_id {
        validate_bytes_non_empty("app_state.selected_topic_id", topic_id.len())
            .map_err(ClientError::from)?;
        validate_string_bytes("app_state.selected_topic_id", topic_id, MAX_OBJECT_ID_BYTES)
            .map_err(ClientError::from)?;
    }
    if let Some(chat_id) = &metadata.selected_chat_id {
        validate_bytes_non_empty("app_state.selected_chat_id", chat_id.len())
            .map_err(ClientError::from)?;
        validate_string_bytes("app_state.selected_chat_id", chat_id, MAX_OBJECT_ID_BYTES)
            .map_err(ClientError::from)?;
    }
    validate_item_count(
        "app_state.revoked_devices",
        metadata.revoked_devices.len(),
        MAX_STORED_APP_REVOKED_DEVICES,
    )
    .map_err(ClientError::from)?;
    for device in &metadata.revoked_devices {
        device.validate_limits().map_err(ClientError::from)?;
    }
    validate_item_count(
        "app_state.chat_archives",
        metadata.chat_archives.len(),
        MAX_STORED_APP_CHAT_ARCHIVES,
    )
    .map_err(ClientError::from)?;
    for archive in &metadata.chat_archives {
        archive.validate_limits()?;
    }
    Ok(metadata)
}

fn decrypt_app_profile_metadata(
    encryption_key: &ClientStoreEncryptionKey,
    identity: AppProfileIdentity<'_>,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<StoredAppProfileMetadataV1, ClientStoreError> {
    if nonce.len() != CLIENT_STORE_NONCE_BYTES {
        return Err(ClientStoreError::InvalidAppProfileNonceLength {
            actual_bytes: nonce.len(),
        });
    }
    validate_bytes_non_empty("app_profile.ciphertext", ciphertext.len())
        .map_err(ClientError::from)?;
    validate_bytes_len(
        "app_profile.ciphertext",
        ciphertext.len(),
        MAX_APP_PROFILE_METADATA_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = app_profile_aad(identity)?;
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            ciphertext,
            nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::DecryptAppProfile)?;
    validate_bytes_len(
        "app_profile.metadata",
        plaintext.len(),
        MAX_APP_PROFILE_METADATA_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let metadata = serde_json::from_slice::<StoredAppProfileMetadataV1>(&plaintext)
        .map_err(|_| ClientStoreError::DecodeAppProfileMetadata)?;
    if metadata.profile.account_id != identity.profile_account_id {
        return Err(ClientStoreError::DecodeAppProfileMetadata);
    }
    validate_nostr_profile_record(&metadata.profile)?;
    Ok(metadata)
}

fn decrypt_device_link_bootstrap_manifest(
    encryption_key: &ClientStoreEncryptionKey,
    identity: DeviceLinkBootstrapIdentity<'_>,
    manifest_sha256: &str,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<StoredDeviceLinkBootstrapManifestV2, ClientStoreError> {
    if nonce.len() != CLIENT_STORE_NONCE_BYTES {
        return Err(
            ClientStoreError::InvalidDeviceLinkBootstrapManifestNonceLength {
                actual_bytes: nonce.len(),
            },
        );
    }
    validate_bytes_non_empty(
        "device_link_bootstrap.manifest_ciphertext",
        ciphertext.len(),
    )
    .map_err(ClientError::from)?;
    validate_bytes_len(
        "device_link_bootstrap.manifest_ciphertext",
        ciphertext.len(),
        MAX_DEVICE_LINK_BOOTSTRAP_MANIFEST_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = device_link_bootstrap_aad(
        CLIENT_DEVICE_LINK_BOOTSTRAP_MANIFEST_AAD_DOMAIN,
        identity,
        manifest_sha256,
        None,
    )?;
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            ciphertext,
            nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::DecryptDeviceLinkBootstrapManifest)?;
    validate_bytes_len(
        "device_link_bootstrap.manifest",
        plaintext.len(),
        MAX_DEVICE_LINK_BOOTSTRAP_MANIFEST_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    serde_json::from_slice(&plaintext)
        .map_err(|_| ClientStoreError::DecodeDeviceLinkBootstrapManifest)
}

fn decrypt_device_link_bootstrap_chunk(
    encryption_key: &ClientStoreEncryptionKey,
    identity: DeviceLinkBootstrapIdentity<'_>,
    manifest_sha256: &str,
    chunk_index: u32,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<DeviceLinkBootstrapEventV2>, ClientStoreError> {
    if nonce.len() != CLIENT_STORE_NONCE_BYTES {
        return Err(
            ClientStoreError::InvalidDeviceLinkBootstrapChunkNonceLength {
                actual_bytes: nonce.len(),
            },
        );
    }
    validate_bytes_non_empty("device_link_bootstrap.chunk_ciphertext", ciphertext.len())
        .map_err(ClientError::from)?;
    validate_bytes_len(
        "device_link_bootstrap.chunk_ciphertext",
        ciphertext.len(),
        MAX_DEVICE_LINK_BOOTSTRAP_CHUNK_CIPHERTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let aad = device_link_bootstrap_aad(
        CLIENT_DEVICE_LINK_BOOTSTRAP_CHUNK_AAD_DOMAIN,
        identity,
        manifest_sha256,
        Some(chunk_index),
    )?;
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            encryption_key.as_bytes(),
            ciphertext,
            nonce,
            &aad,
        )
        .map_err(|_| ClientStoreError::DecryptDeviceLinkBootstrapChunk)?;
    validate_bytes_len(
        "device_link_bootstrap.chunk",
        plaintext.len(),
        MAX_DEVICE_LINK_BOOTSTRAP_CHUNK_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let history = serde_json::from_slice::<Vec<DeviceLinkBootstrapEventV2>>(&plaintext)
        .map_err(|_| ClientStoreError::DecodeDeviceLinkBootstrapChunk)?;
    validate_item_count(
        "device_link_bootstrap.history",
        history.len(),
        finitechat_proto::MAX_DEVICE_LINK_BOOTSTRAP_EVENTS,
    )
    .map_err(ClientError::from)?;
    for event in &history {
        event.validate_limits().map_err(ClientError::from)?;
    }
    Ok(history)
}

fn client_store_aad(
    version: u16,
    account_id: &str,
    device_id: &str,
) -> Result<Vec<u8>, ClientStoreError> {
    validate_string_bytes("account_id", account_id, MAX_ACCOUNT_ID_BYTES)
        .map_err(ClientError::from)?;
    validate_string_bytes("device_id", device_id, MAX_DEVICE_ID_BYTES)
        .map_err(ClientError::from)?;
    let mut aad = Vec::with_capacity(
        CLIENT_STATE_SNAPSHOT_MAGIC.len()
            + U16_BYTES
            + U32_BYTES
            + account_id.len()
            + U32_BYTES
            + device_id.len(),
    );
    aad.extend_from_slice(CLIENT_STATE_SNAPSHOT_MAGIC);
    aad.extend_from_slice(&version.to_be_bytes());
    append_raw_len_prefixed(&mut aad, account_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, device_id.as_bytes())?;
    debug_assert!(aad.len() >= CLIENT_STATE_SNAPSHOT_MAGIC.len() + U16_BYTES);
    Ok(aad)
}

fn app_message_aad(identity: AppMessageIdentity<'_>) -> Result<Vec<u8>, ClientStoreError> {
    app_event_or_message_aad(
        CLIENT_APP_MESSAGE_AAD_DOMAIN,
        "app_message.message_id",
        identity,
    )
}

fn app_event_aad(identity: AppMessageIdentity<'_>) -> Result<Vec<u8>, ClientStoreError> {
    app_event_or_message_aad(
        CLIENT_APP_EVENT_AAD_DOMAIN,
        "app_event.message_id",
        identity,
    )
}

fn app_event_or_message_aad(
    domain: &[u8],
    message_id_field: &'static str,
    identity: AppMessageIdentity<'_>,
) -> Result<Vec<u8>, ClientStoreError> {
    validate_app_message_owner(identity.owner)?;
    validate_room_id(identity.room_id).map_err(ClientError::from)?;
    validate_string_bytes(message_id_field, identity.message_id, MAX_OBJECT_ID_BYTES)
        .map_err(ClientError::from)?;
    identity
        .sender
        .validate_limits()
        .map_err(ClientError::from)?;
    let mut aad = Vec::with_capacity(
        domain.len()
            + U32_BYTES
            + identity.owner.account_id.len()
            + U32_BYTES
            + identity.owner.device_id.len()
            + U32_BYTES
            + identity.room_id.len()
            + U64_BYTES
            + U32_BYTES
            + identity.message_id.len()
            + U32_BYTES
            + identity.sender.account_id.len()
            + U32_BYTES
            + identity.sender.device_id.len(),
    );
    aad.extend_from_slice(domain);
    append_raw_len_prefixed(&mut aad, identity.owner.account_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.owner.device_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.room_id.as_bytes())?;
    aad.extend_from_slice(&identity.seq.to_be_bytes());
    append_raw_len_prefixed(&mut aad, identity.message_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.sender.account_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.sender.device_id.as_bytes())?;
    Ok(aad)
}

fn app_outbox_aad(identity: AppOutboxIdentity<'_>) -> Result<Vec<u8>, ClientStoreError> {
    validate_app_message_owner(identity.owner)?;
    validate_room_id(identity.room_id).map_err(ClientError::from)?;
    validate_string_bytes(
        "app_outbox.message_id",
        identity.message_id,
        MAX_OBJECT_ID_BYTES,
    )
    .map_err(ClientError::from)?;
    let mut aad = Vec::with_capacity(
        CLIENT_APP_OUTBOX_AAD_DOMAIN.len()
            + U32_BYTES
            + identity.owner.account_id.len()
            + U32_BYTES
            + identity.owner.device_id.len()
            + U32_BYTES
            + identity.room_id.len()
            + U32_BYTES
            + identity.message_id.len(),
    );
    aad.extend_from_slice(CLIENT_APP_OUTBOX_AAD_DOMAIN);
    append_raw_len_prefixed(&mut aad, identity.owner.account_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.owner.device_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.room_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.message_id.as_bytes())?;
    Ok(aad)
}

fn app_room_aad(identity: AppRoomIdentity<'_>) -> Result<Vec<u8>, ClientStoreError> {
    validate_app_message_owner(identity.owner)?;
    validate_room_id(identity.room_id).map_err(ClientError::from)?;
    let mut aad = Vec::with_capacity(
        CLIENT_APP_ROOM_AAD_DOMAIN.len()
            + U32_BYTES
            + identity.owner.account_id.len()
            + U32_BYTES
            + identity.owner.device_id.len()
            + U32_BYTES
            + identity.room_id.len(),
    );
    aad.extend_from_slice(CLIENT_APP_ROOM_AAD_DOMAIN);
    append_raw_len_prefixed(&mut aad, identity.owner.account_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.owner.device_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.room_id.as_bytes())?;
    Ok(aad)
}

fn app_state_aad(identity: AppStateIdentity<'_>) -> Result<Vec<u8>, ClientStoreError> {
    validate_app_message_owner(identity.owner)?;
    let mut aad = Vec::with_capacity(
        CLIENT_APP_STATE_AAD_DOMAIN.len()
            + U32_BYTES
            + identity.owner.account_id.len()
            + U32_BYTES
            + identity.owner.device_id.len(),
    );
    aad.extend_from_slice(CLIENT_APP_STATE_AAD_DOMAIN);
    append_raw_len_prefixed(&mut aad, identity.owner.account_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.owner.device_id.as_bytes())?;
    Ok(aad)
}

fn app_profile_aad(identity: AppProfileIdentity<'_>) -> Result<Vec<u8>, ClientStoreError> {
    validate_app_message_owner(identity.owner)?;
    decode_lower_hex_32(identity.profile_account_id)?;
    let mut aad = Vec::with_capacity(
        CLIENT_APP_PROFILE_AAD_DOMAIN.len()
            + U32_BYTES
            + identity.owner.account_id.len()
            + U32_BYTES
            + identity.owner.device_id.len()
            + U32_BYTES
            + identity.profile_account_id.len(),
    );
    aad.extend_from_slice(CLIENT_APP_PROFILE_AAD_DOMAIN);
    append_raw_len_prefixed(&mut aad, identity.owner.account_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.owner.device_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.profile_account_id.as_bytes())?;
    Ok(aad)
}

fn device_link_bootstrap_aad(
    domain: &[u8],
    identity: DeviceLinkBootstrapIdentity<'_>,
    manifest_sha256: &str,
    chunk_index: Option<u32>,
) -> Result<Vec<u8>, ClientStoreError> {
    validate_device_link_bootstrap_identity(identity)?;
    if !is_lower_hex_sha256(manifest_sha256) {
        return Err(ClientStoreError::DeviceLinkBootstrapReceiptMismatch);
    }
    let mut aad = Vec::with_capacity(
        domain.len()
            + U32_BYTES
            + identity.owner.account_id.len()
            + U32_BYTES
            + identity.owner.device_id.len()
            + U32_BYTES
            + identity.room_id.len()
            + U32_BYTES
            + identity.bootstrap_id.len()
            + U32_BYTES
            + identity.source.account_id.len()
            + U32_BYTES
            + identity.source.device_id.len()
            + U32_BYTES
            + manifest_sha256.len()
            + U32_BYTES,
    );
    aad.extend_from_slice(domain);
    append_raw_len_prefixed(&mut aad, identity.owner.account_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.owner.device_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.room_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.bootstrap_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.source.account_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, identity.source.device_id.as_bytes())?;
    append_raw_len_prefixed(&mut aad, manifest_sha256.as_bytes())?;
    match chunk_index {
        Some(index) => {
            aad.push(1);
            aad.extend_from_slice(&index.to_be_bytes());
        }
        None => aad.push(0),
    }
    Ok(aad)
}

fn encode_device_state(state: &FiniteChatDeviceState) -> Result<Vec<u8>, ClientStoreError> {
    state.validate_limits()?;
    let mut out = Vec::with_capacity(encoded_device_state_len(state)?);
    out.extend_from_slice(CLIENT_STATE_SNAPSHOT_MAGIC);
    out.extend_from_slice(&CLIENT_STATE_SNAPSHOT_VERSION.to_be_bytes());
    append_string_field(
        &mut out,
        "account_id",
        &state.device_ref.account_id,
        MAX_ACCOUNT_ID_BYTES,
    )?;
    append_string_field(
        &mut out,
        "device_id",
        &state.device_ref.device_id,
        MAX_DEVICE_ID_BYTES,
    )?;
    append_bytes_field(
        &mut out,
        "signer_public_key",
        &state.signer_public_key,
        MAX_CLIENT_SIGNER_PUBLIC_KEY_BYTES,
    )?;
    append_bytes_field(
        &mut out,
        "credential_identity",
        &state.credential_identity,
        MAX_CLIENT_CREDENTIAL_IDENTITY_BYTES,
    )?;
    append_count(
        &mut out,
        "client_state.rooms",
        state.rooms.len(),
        MAX_PERSISTED_ROOMS,
    )?;
    for room in &state.rooms {
        room.validate_limits()?;
        append_string_field(
            &mut out,
            "room_id",
            &room.room_id,
            finitechat_proto::MAX_ROOM_ID_BYTES,
        )?;
        append_string_field(
            &mut out,
            "mls_group_id",
            &room.mls_group_id,
            finitechat_proto::MAX_MLS_GROUP_ID_BYTES,
        )?;
        out.extend_from_slice(&room.last_applied_seq.to_be_bytes());
        append_raw_len_prefixed(
            &mut out,
            room.server_url.as_deref().unwrap_or("").as_bytes(),
        )?;
    }
    append_count(
        &mut out,
        "client_state.pending_welcomes",
        state.pending_welcomes.len(),
        MAX_PENDING_CLIENT_WELCOMES,
    )?;
    for welcome in &state.pending_welcomes {
        welcome.validate_limits()?;
        append_string_field(
            &mut out,
            "welcome_id",
            &welcome.welcome_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        append_string_field(
            &mut out,
            "pending_welcome.room_id",
            &welcome.room_id,
            finitechat_proto::MAX_ROOM_ID_BYTES,
        )?;
        out.extend_from_slice(&welcome.commit_seq.to_be_bytes());
        append_bytes_field(
            &mut out,
            "pending_welcome.welcome_payload",
            &welcome.welcome_payload,
            MAX_WELCOME_PAYLOAD_BYTES,
        )?;
        append_bytes_field(
            &mut out,
            "pending_welcome.ratchet_tree_payload",
            &welcome.ratchet_tree_payload,
            MAX_RATCHET_TREE_PAYLOAD_BYTES,
        )?;
    }
    append_count(
        &mut out,
        "client_state.pending_welcome_acks",
        state.pending_welcome_acks.len(),
        MAX_PENDING_CLIENT_WELCOMES,
    )?;
    for ack in &state.pending_welcome_acks {
        ack.validate_limits()?;
        append_string_field(
            &mut out,
            "welcome_ack.welcome_id",
            &ack.welcome_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        append_string_field(
            &mut out,
            "welcome_ack.room_id",
            &ack.room_id,
            finitechat_proto::MAX_ROOM_ID_BYTES,
        )?;
        out.extend_from_slice(&ack.commit_seq.to_be_bytes());
    }
    append_count(
        &mut out,
        "client_state.pending_key_package_uploads",
        state.pending_key_package_uploads.len(),
        MAX_PENDING_KEY_PACKAGE_UPLOADS,
    )?;
    for request in &state.pending_key_package_uploads {
        request.validate_limits().map_err(ClientError::from)?;
        append_upload_key_package_request(&mut out, request)?;
    }
    append_count(
        &mut out,
        "client_state.link_fanouts",
        state.link_fanouts.len(),
        MAX_LINK_FANOUTS,
    )?;
    for fanout in &state.link_fanouts {
        append_link_fanout_state(&mut out, fanout)?;
    }
    append_count(
        &mut out,
        "client_state.openmls_storage_records",
        state.openmls_storage_records.len(),
        MAX_OPENMLS_STORAGE_RECORDS,
    )?;
    for record in &state.openmls_storage_records {
        record.validate_limits()?;
        append_bytes_field(
            &mut out,
            "openmls_storage.key",
            &record.key,
            MAX_OPENMLS_STORAGE_KEY_BYTES,
        )?;
        append_bytes_field(
            &mut out,
            "openmls_storage.value",
            &record.value,
            MAX_OPENMLS_STORAGE_VALUE_BYTES,
        )?;
    }
    validate_bytes_len(
        "client_state.plaintext",
        out.len(),
        MAX_CLIENT_STATE_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    debug_assert!(!out.is_empty());
    Ok(out)
}

fn decode_device_state(bytes: &[u8]) -> Result<FiniteChatDeviceState, ClientStoreError> {
    validate_bytes_non_empty("client_state.plaintext", bytes.len()).map_err(ClientError::from)?;
    validate_bytes_len(
        "client_state.plaintext",
        bytes.len(),
        MAX_CLIENT_STATE_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    let mut cursor = ClientStateCursor::new(bytes);
    cursor.take_magic()?;
    let version = cursor.take_u16()?;
    if version != CLIENT_STATE_SNAPSHOT_VERSION && version != LEGACY_CLIENT_STATE_SNAPSHOT_VERSION {
        return Err(ClientStoreError::StateSnapshotVersion(version));
    }
    let account_id = cursor.take_string("account_id", MAX_ACCOUNT_ID_BYTES)?;
    let device_id = cursor.take_string("device_id", MAX_DEVICE_ID_BYTES)?;
    let signer_public_key =
        cursor.take_vec("signer_public_key", MAX_CLIENT_SIGNER_PUBLIC_KEY_BYTES)?;
    let credential_identity =
        cursor.take_vec("credential_identity", MAX_CLIENT_CREDENTIAL_IDENTITY_BYTES)?;

    let room_count = cursor.take_count("client_state.rooms", MAX_PERSISTED_ROOMS)?;
    let mut rooms = Vec::with_capacity(room_count);
    for _ in 0..room_count {
        let room_id = cursor.take_string("room_id", finitechat_proto::MAX_ROOM_ID_BYTES)?;
        let mls_group_id =
            cursor.take_string("mls_group_id", finitechat_proto::MAX_MLS_GROUP_ID_BYTES)?;
        let last_applied_seq = cursor.take_u64()?;
        let server_url_bytes = cursor.take_vec("room.server_url", MAX_ROOM_SERVER_URL_BYTES)?;
        let server_url = if server_url_bytes.is_empty() {
            None
        } else {
            Some(
                String::from_utf8(server_url_bytes)
                    .map_err(|_| ClientStoreError::StateSnapshotUtf8)?,
            )
        };
        rooms.push(PersistedRoomState {
            room_id,
            mls_group_id,
            last_applied_seq,
            server_url,
        });
    }

    let pending_welcome_count =
        cursor.take_count("client_state.pending_welcomes", MAX_PENDING_CLIENT_WELCOMES)?;
    let mut pending_welcomes = Vec::with_capacity(pending_welcome_count);
    for _ in 0..pending_welcome_count {
        pending_welcomes.push(PendingWelcomeState {
            welcome_id: cursor.take_string("welcome_id", MAX_OBJECT_ID_BYTES)?,
            room_id: cursor.take_string(
                "pending_welcome.room_id",
                finitechat_proto::MAX_ROOM_ID_BYTES,
            )?,
            commit_seq: cursor.take_u64()?,
            welcome_payload: cursor
                .take_vec("pending_welcome.welcome_payload", MAX_WELCOME_PAYLOAD_BYTES)?,
            ratchet_tree_payload: cursor.take_vec(
                "pending_welcome.ratchet_tree_payload",
                MAX_RATCHET_TREE_PAYLOAD_BYTES,
            )?,
        });
    }

    let pending_welcome_ack_count = cursor.take_count(
        "client_state.pending_welcome_acks",
        MAX_PENDING_CLIENT_WELCOMES,
    )?;
    let mut pending_welcome_acks = Vec::with_capacity(pending_welcome_ack_count);
    for _ in 0..pending_welcome_ack_count {
        pending_welcome_acks.push(PendingWelcomeAckState {
            welcome_id: cursor.take_string("welcome_ack.welcome_id", MAX_OBJECT_ID_BYTES)?,
            room_id: cursor
                .take_string("welcome_ack.room_id", finitechat_proto::MAX_ROOM_ID_BYTES)?,
            commit_seq: cursor.take_u64()?,
        });
    }

    let pending_key_package_upload_count = cursor.take_count(
        "client_state.pending_key_package_uploads",
        MAX_PENDING_KEY_PACKAGE_UPLOADS,
    )?;
    let mut pending_key_package_uploads = Vec::with_capacity(pending_key_package_upload_count);
    for _ in 0..pending_key_package_upload_count {
        pending_key_package_uploads.push(cursor.take_upload_key_package_request()?);
    }

    let link_fanout_count = cursor.take_count("client_state.link_fanouts", MAX_LINK_FANOUTS)?;
    let mut link_fanouts = Vec::with_capacity(link_fanout_count);
    for _ in 0..link_fanout_count {
        link_fanouts.push(cursor.take_link_fanout_state(version)?);
    }

    let storage_count = cursor.take_count(
        "client_state.openmls_storage_records",
        MAX_OPENMLS_STORAGE_RECORDS,
    )?;
    let mut openmls_storage_records = Vec::with_capacity(storage_count);
    for _ in 0..storage_count {
        openmls_storage_records.push(OpenMlsStorageRecord {
            key: cursor.take_vec("openmls_storage.key", MAX_OPENMLS_STORAGE_KEY_BYTES)?,
            value: cursor.take_vec("openmls_storage.value", MAX_OPENMLS_STORAGE_VALUE_BYTES)?,
        });
    }
    cursor.finish()?;

    let state = FiniteChatDeviceState {
        device_ref: DeviceRef {
            account_id,
            device_id,
        },
        signer_public_key,
        credential_identity,
        rooms,
        pending_welcomes,
        pending_welcome_acks,
        pending_key_package_uploads,
        link_fanouts,
        openmls_storage_records,
    };
    state.validate_limits()?;
    Ok(state)
}

fn encoded_device_state_len(state: &FiniteChatDeviceState) -> Result<usize, ClientStoreError> {
    let mut len = CLIENT_STATE_SNAPSHOT_MAGIC.len() + U16_BYTES;
    len = checked_len_add(len, U32_BYTES + state.device_ref.account_id.len())?;
    len = checked_len_add(len, U32_BYTES + state.device_ref.device_id.len())?;
    len = checked_len_add(len, U32_BYTES + state.signer_public_key.len())?;
    len = checked_len_add(len, U32_BYTES + state.credential_identity.len())?;
    len = checked_len_add(len, U32_BYTES)?;
    for room in &state.rooms {
        len = checked_len_add(len, U32_BYTES + room.room_id.len())?;
        len = checked_len_add(len, U32_BYTES + room.mls_group_id.len())?;
        len = checked_len_add(len, U64_BYTES)?;
        len = checked_len_add(
            len,
            U32_BYTES + room.server_url.as_deref().unwrap_or("").len(),
        )?;
    }
    len = checked_len_add(len, U32_BYTES)?;
    for welcome in &state.pending_welcomes {
        len = checked_len_add(len, U32_BYTES + welcome.welcome_id.len())?;
        len = checked_len_add(len, U32_BYTES + welcome.room_id.len())?;
        len = checked_len_add(len, U64_BYTES)?;
        len = checked_len_add(len, U32_BYTES + welcome.welcome_payload.len())?;
        len = checked_len_add(len, U32_BYTES + welcome.ratchet_tree_payload.len())?;
    }
    len = checked_len_add(len, U32_BYTES)?;
    for ack in &state.pending_welcome_acks {
        len = checked_len_add(len, U32_BYTES + ack.welcome_id.len())?;
        len = checked_len_add(len, U32_BYTES + ack.room_id.len())?;
        len = checked_len_add(len, U64_BYTES)?;
    }
    len = checked_len_add(len, U32_BYTES)?;
    for request in &state.pending_key_package_uploads {
        len = checked_len_add(len, encoded_upload_key_package_request_len(request)?)?;
    }
    len = checked_len_add(len, U32_BYTES)?;
    for fanout in &state.link_fanouts {
        len = checked_len_add(len, encoded_link_fanout_state_len(fanout)?)?;
    }
    len = checked_len_add(len, U32_BYTES)?;
    for record in &state.openmls_storage_records {
        len = checked_len_add(len, U32_BYTES + record.key.len())?;
        len = checked_len_add(len, U32_BYTES + record.value.len())?;
    }
    validate_bytes_len(
        "client_state.plaintext",
        len,
        MAX_CLIENT_STATE_PLAINTEXT_BYTES,
    )
    .map_err(ClientError::from)?;
    Ok(len)
}

fn append_upload_key_package_request(
    out: &mut Vec<u8>,
    request: &UploadKeyPackageRequest,
) -> Result<(), ClientStoreError> {
    request.validate_limits().map_err(ClientError::from)?;
    append_string_field(
        out,
        "pending_key_package_upload.key_package_id",
        &request.key_package_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_device_ref(out, "pending_key_package_upload.owner", &request.owner)?;
    append_string_field(
        out,
        "pending_key_package_upload.key_package_ref",
        &request.key_package_ref,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_string_field(
        out,
        "pending_key_package_upload.key_package_hash",
        &request.key_package_hash,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_bytes_field(
        out,
        "pending_key_package_upload.key_package_payload",
        &request.key_package_payload,
        MAX_KEY_PACKAGE_PAYLOAD_BYTES,
    )?;
    Ok(())
}

fn encoded_upload_key_package_request_len(
    request: &UploadKeyPackageRequest,
) -> Result<usize, ClientStoreError> {
    request.validate_limits().map_err(ClientError::from)?;
    let mut len = U32_BYTES + request.key_package_id.len();
    len = checked_len_add(len, encoded_device_ref_len(&request.owner)?)?;
    len = checked_len_add(len, U32_BYTES + request.key_package_ref.len())?;
    len = checked_len_add(len, U32_BYTES + request.key_package_hash.len())?;
    len = checked_len_add(len, U32_BYTES + request.key_package_payload.len())?;
    Ok(len)
}

fn append_link_fanout_state(
    out: &mut Vec<u8>,
    fanout: &LinkFanoutState,
) -> Result<(), ClientStoreError> {
    fanout.validate_limits()?;
    append_string_field(
        out,
        "link_fanout.fanout_id",
        &fanout.fanout_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_device_ref(out, "link_fanout.target_device", &fanout.target_device)?;
    append_optional_string(
        out,
        "link_fanout.after_room_id",
        fanout.after_room_id.as_deref(),
        MAX_ROOM_ID_BYTES,
    )?;
    append_bool(out, fanout.discovery_complete);
    append_count(
        out,
        "link_fanout.rooms",
        fanout.rooms.len(),
        MAX_LINK_FANOUT_ROOMS,
    )?;
    for room in &fanout.rooms {
        append_link_fanout_room_state(out, room)?;
    }
    let bootstrap_export = serde_json::to_vec(&fanout.bootstrap_export)
        .map_err(|_| ClientError::InvalidClientState("device-link bootstrap export".to_owned()))?;
    append_bytes_field(
        out,
        "link_fanout.bootstrap_export",
        &bootstrap_export,
        MAX_CLIENT_STATE_PLAINTEXT_BYTES,
    )?;
    Ok(())
}

fn append_link_fanout_room_state(
    out: &mut Vec<u8>,
    room: &LinkFanoutRoomState,
) -> Result<(), ClientStoreError> {
    room.validate_limits()?;
    append_link_fanout_room_plan(out, &room.plan)?;
    append_bool(out, room.claimed_key_package.is_some());
    if let Some(claimed_key_package) = &room.claimed_key_package {
        append_claimed_key_package(out, claimed_key_package)?;
    }
    match &room.status {
        LinkFanoutRoomStatus::Pending => {
            out.extend_from_slice(&LINK_FANOUT_STATUS_PENDING.to_be_bytes());
        }
        LinkFanoutRoomStatus::Prepared { prepared } => {
            out.extend_from_slice(&LINK_FANOUT_STATUS_PREPARED.to_be_bytes());
            append_prepared_commit(out, prepared)?;
        }
        LinkFanoutRoomStatus::Done { accepted_seq } => {
            out.extend_from_slice(&LINK_FANOUT_STATUS_DONE.to_be_bytes());
            out.extend_from_slice(&accepted_seq.to_be_bytes());
        }
    }
    Ok(())
}

fn append_claimed_key_package(
    out: &mut Vec<u8>,
    claimed_key_package: &ClaimKeyPackageResult,
) -> Result<(), ClientStoreError> {
    validate_claimed_key_package(claimed_key_package)?;
    append_string_field(
        out,
        "claimed_key_package.key_package_id",
        &claimed_key_package.key_package_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_device_ref(out, "claimed_key_package.owner", &claimed_key_package.owner)?;
    append_string_field(
        out,
        "claimed_key_package.key_package_ref",
        &claimed_key_package.key_package_ref,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_string_field(
        out,
        "claimed_key_package.key_package_hash",
        &claimed_key_package.key_package_hash,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_bytes_field(
        out,
        "claimed_key_package.key_package_payload",
        &claimed_key_package.key_package_payload,
        MAX_KEY_PACKAGE_PAYLOAD_BYTES,
    )?;
    append_string_field(
        out,
        "claimed_key_package.lease_token",
        &claimed_key_package.lease_token,
        MAX_OBJECT_ID_BYTES,
    )?;
    Ok(())
}

fn append_link_fanout_room_plan(
    out: &mut Vec<u8>,
    plan: &LinkFanoutRoomPlan,
) -> Result<(), ClientStoreError> {
    plan.validate_limits()?;
    append_string_field(out, "link_fanout.room_id", &plan.room_id, MAX_ROOM_ID_BYTES)?;
    append_string_field(
        out,
        "link_fanout.key_package_id",
        &plan.key_package_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_string_field(
        out,
        "link_fanout.welcome_id",
        &plan.welcome_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_string_field(
        out,
        "link_fanout.idempotency_key",
        &plan.idempotency_key,
        MAX_IDEMPOTENCY_KEY_BYTES,
    )?;
    Ok(())
}

fn append_prepared_commit(
    out: &mut Vec<u8>,
    prepared: &PreparedCommit,
) -> Result<(), ClientStoreError> {
    prepared.validate_limits()?;
    append_submit_commit_request(out, &prepared.request)?;
    append_string_field(
        out,
        "prepared_commit.message_id",
        &prepared.message_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    Ok(())
}

fn append_submit_commit_request(
    out: &mut Vec<u8>,
    request: &SubmitCommitRequest,
) -> Result<(), ClientStoreError> {
    request.validate_limits().map_err(ClientError::from)?;
    append_string_field(
        out,
        "commit_request.room_id",
        &request.room_id,
        MAX_ROOM_ID_BYTES,
    )?;
    append_device_ref(out, "commit_request.sender", &request.sender)?;
    out.extend_from_slice(&request.expected_epoch.to_be_bytes());
    append_envelope(out, &request.envelope)?;
    append_membership_delta(out, &request.membership_delta)?;
    append_count(
        out,
        "commit_request.staged_welcomes",
        request.staged_welcomes.len(),
        MAX_STAGED_WELCOMES_PER_COMMIT,
    )?;
    for welcome in &request.staged_welcomes {
        append_staged_welcome(out, welcome)?;
    }
    append_string_field(
        out,
        "commit_request.idempotency_key",
        &request.idempotency_key,
        MAX_IDEMPOTENCY_KEY_BYTES,
    )?;
    Ok(())
}

fn append_membership_delta(
    out: &mut Vec<u8>,
    delta: &MembershipDeltaV1,
) -> Result<(), ClientStoreError> {
    delta.validate_limits().map_err(ClientError::from)?;
    out.extend_from_slice(&delta.base_epoch.to_be_bytes());
    out.extend_from_slice(&delta.post_commit_epoch.to_be_bytes());
    append_string_field(
        out,
        "membership_delta.commit_message_id",
        &delta.commit_message_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_count(
        out,
        "membership_delta.adds",
        delta.adds.len(),
        MAX_STAGED_WELCOMES_PER_COMMIT,
    )?;
    for add in &delta.adds {
        append_device_ref(out, "membership_delta.add.device", &add.device)?;
        append_string_field(
            out,
            "membership_delta.add.key_package_id",
            &add.key_package_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        append_string_field(
            out,
            "membership_delta.add.key_package_ref",
            &add.key_package_ref,
            MAX_OBJECT_ID_BYTES,
        )?;
        append_string_field(
            out,
            "membership_delta.add.key_package_hash",
            &add.key_package_hash,
            MAX_OBJECT_ID_BYTES,
        )?;
        append_string_field(
            out,
            "membership_delta.add.welcome_id",
            &add.welcome_id,
            MAX_OBJECT_ID_BYTES,
        )?;
    }
    append_count(
        out,
        "membership_delta.removes",
        delta.removes.len(),
        MAX_STAGED_WELCOMES_PER_COMMIT,
    )?;
    for remove in &delta.removes {
        append_device_ref(out, "membership_delta.remove.device", &remove.device)?;
        out.extend_from_slice(&remove.removed_leaf_index.to_be_bytes());
    }
    Ok(())
}

fn append_envelope(
    out: &mut Vec<u8>,
    envelope: &finitechat_proto::FiniteEnvelope,
) -> Result<(), ClientStoreError> {
    envelope.validate_limits().map_err(ClientError::from)?;
    append_string_field(
        out,
        "envelope.room_id",
        &envelope.room_id,
        MAX_ROOM_ID_BYTES,
    )?;
    append_string_field(
        out,
        "envelope.mls_group_id",
        &envelope.mls_group_id,
        MAX_MLS_GROUP_ID_BYTES,
    )?;
    out.extend_from_slice(&envelope.epoch.to_be_bytes());
    append_device_ref(out, "envelope.sender", &envelope.sender)?;
    out.extend_from_slice(&encode_log_entry_kind(envelope.kind).to_be_bytes());
    append_bytes_field(
        out,
        "envelope.payload",
        &envelope.payload,
        MAX_ENVELOPE_PAYLOAD_BYTES,
    )?;
    Ok(())
}

fn append_staged_welcome(
    out: &mut Vec<u8>,
    welcome: &StagedWelcomeV1,
) -> Result<(), ClientStoreError> {
    welcome.validate_limits().map_err(ClientError::from)?;
    append_string_field(
        out,
        "staged_welcome.welcome_id",
        &welcome.welcome_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    append_bytes_field(
        out,
        "staged_welcome.welcome_payload",
        &welcome.welcome_payload,
        MAX_WELCOME_PAYLOAD_BYTES,
    )?;
    append_bytes_field(
        out,
        "staged_welcome.ratchet_tree_payload",
        &welcome.ratchet_tree_payload,
        MAX_RATCHET_TREE_PAYLOAD_BYTES,
    )?;
    Ok(())
}

fn append_device_ref(
    out: &mut Vec<u8>,
    field: &str,
    device: &DeviceRef,
) -> Result<(), ClientStoreError> {
    device.validate_limits().map_err(ClientError::from)?;
    append_string_field(out, field, &device.account_id, MAX_ACCOUNT_ID_BYTES)?;
    append_string_field(out, field, &device.device_id, MAX_DEVICE_ID_BYTES)?;
    Ok(())
}

fn append_optional_string(
    out: &mut Vec<u8>,
    field: &str,
    value: Option<&str>,
    max_bytes: u32,
) -> Result<(), ClientStoreError> {
    append_bool(out, value.is_some());
    if let Some(value) = value {
        append_string_field(out, field, value, max_bytes)?;
    }
    Ok(())
}

fn append_bool(out: &mut Vec<u8>, value: bool) {
    let encoded = if value { 1u16 } else { 0u16 };
    out.extend_from_slice(&encoded.to_be_bytes());
}

fn encode_log_entry_kind(kind: LogEntryKind) -> u16 {
    match kind {
        LogEntryKind::Application => LOG_ENTRY_KIND_APPLICATION,
        LogEntryKind::Proposal => LOG_ENTRY_KIND_PROPOSAL,
        LogEntryKind::Commit => LOG_ENTRY_KIND_COMMIT,
    }
}

fn decode_log_entry_kind(value: u16) -> Result<LogEntryKind, ClientStoreError> {
    match value {
        LOG_ENTRY_KIND_APPLICATION => Ok(LogEntryKind::Application),
        LOG_ENTRY_KIND_PROPOSAL => Ok(LogEntryKind::Proposal),
        LOG_ENTRY_KIND_COMMIT => Ok(LogEntryKind::Commit),
        other => Err(ClientStoreError::StateSnapshotEnum {
            field: "log_entry_kind",
            value: u64::from(other),
        }),
    }
}

fn encoded_link_fanout_state_len(fanout: &LinkFanoutState) -> Result<usize, ClientStoreError> {
    let mut len = U32_BYTES + fanout.fanout_id.len();
    len = checked_len_add(len, encoded_device_ref_len(&fanout.target_device)?)?;
    len = checked_len_add(len, U16_BYTES)?;
    if let Some(after_room_id) = &fanout.after_room_id {
        len = checked_len_add(len, U32_BYTES + after_room_id.len())?;
    }
    len = checked_len_add(len, U16_BYTES)?;
    len = checked_len_add(len, U32_BYTES)?;
    for room in &fanout.rooms {
        len = checked_len_add(len, encoded_link_fanout_room_state_len(room)?)?;
    }
    let bootstrap_export = serde_json::to_vec(&fanout.bootstrap_export)
        .map_err(|_| ClientError::InvalidClientState("device-link bootstrap export".to_owned()))?;
    len = checked_len_add(len, U32_BYTES + bootstrap_export.len())?;
    Ok(len)
}

fn encoded_link_fanout_room_state_len(
    room: &LinkFanoutRoomState,
) -> Result<usize, ClientStoreError> {
    let mut len = encoded_link_fanout_room_plan_len(&room.plan)?;
    len = checked_len_add(len, U16_BYTES)?;
    if let Some(claimed_key_package) = &room.claimed_key_package {
        len = checked_len_add(len, encoded_claimed_key_package_len(claimed_key_package)?)?;
    }
    match &room.status {
        LinkFanoutRoomStatus::Pending => {}
        LinkFanoutRoomStatus::Prepared { prepared } => {
            len = checked_len_add(len, encoded_prepared_commit_len(prepared)?)?;
        }
        LinkFanoutRoomStatus::Done { .. } => {
            len = checked_len_add(len, U64_BYTES)?;
        }
    }
    Ok(len)
}

fn encoded_claimed_key_package_len(
    claimed_key_package: &ClaimKeyPackageResult,
) -> Result<usize, ClientStoreError> {
    validate_claimed_key_package(claimed_key_package)?;
    let mut len = U32_BYTES + claimed_key_package.key_package_id.len();
    len = checked_len_add(len, encoded_device_ref_len(&claimed_key_package.owner)?)?;
    len = checked_len_add(len, U32_BYTES + claimed_key_package.key_package_ref.len())?;
    len = checked_len_add(len, U32_BYTES + claimed_key_package.key_package_hash.len())?;
    len = checked_len_add(
        len,
        U32_BYTES + claimed_key_package.key_package_payload.len(),
    )?;
    len = checked_len_add(len, U32_BYTES + claimed_key_package.lease_token.len())?;
    Ok(len)
}

fn encoded_link_fanout_room_plan_len(plan: &LinkFanoutRoomPlan) -> Result<usize, ClientStoreError> {
    let mut len = U32_BYTES + plan.room_id.len();
    len = checked_len_add(len, U32_BYTES + plan.key_package_id.len())?;
    len = checked_len_add(len, U32_BYTES + plan.welcome_id.len())?;
    len = checked_len_add(len, U32_BYTES + plan.idempotency_key.len())?;
    Ok(len)
}

fn encoded_prepared_commit_len(prepared: &PreparedCommit) -> Result<usize, ClientStoreError> {
    let mut len = encoded_submit_commit_request_len(&prepared.request)?;
    len = checked_len_add(len, U32_BYTES + prepared.message_id.len())?;
    Ok(len)
}

fn encoded_submit_commit_request_len(
    request: &SubmitCommitRequest,
) -> Result<usize, ClientStoreError> {
    let mut len = U32_BYTES + request.room_id.len();
    len = checked_len_add(len, encoded_device_ref_len(&request.sender)?)?;
    len = checked_len_add(len, U64_BYTES)?;
    len = checked_len_add(len, encoded_envelope_len(&request.envelope)?)?;
    len = checked_len_add(
        len,
        encoded_membership_delta_len(&request.membership_delta)?,
    )?;
    len = checked_len_add(len, U32_BYTES)?;
    for welcome in &request.staged_welcomes {
        len = checked_len_add(len, encoded_staged_welcome_len(welcome)?)?;
    }
    len = checked_len_add(len, U32_BYTES + request.idempotency_key.len())?;
    Ok(len)
}

fn encoded_membership_delta_len(delta: &MembershipDeltaV1) -> Result<usize, ClientStoreError> {
    let mut len = U64_BYTES + U64_BYTES + U32_BYTES + delta.commit_message_id.len() + U32_BYTES;
    for add in &delta.adds {
        len = checked_len_add(len, encoded_device_ref_len(&add.device)?)?;
        len = checked_len_add(len, U32_BYTES + add.key_package_id.len())?;
        len = checked_len_add(len, U32_BYTES + add.key_package_ref.len())?;
        len = checked_len_add(len, U32_BYTES + add.key_package_hash.len())?;
        len = checked_len_add(len, U32_BYTES + add.welcome_id.len())?;
    }
    len = checked_len_add(len, U32_BYTES)?;
    for remove in &delta.removes {
        len = checked_len_add(len, encoded_device_ref_len(&remove.device)?)?;
        len = checked_len_add(len, U32_BYTES)?;
    }
    Ok(len)
}

fn encoded_envelope_len(
    envelope: &finitechat_proto::FiniteEnvelope,
) -> Result<usize, ClientStoreError> {
    let mut len = U32_BYTES + envelope.room_id.len();
    len = checked_len_add(len, U32_BYTES + envelope.mls_group_id.len())?;
    len = checked_len_add(len, U64_BYTES)?;
    len = checked_len_add(len, encoded_device_ref_len(&envelope.sender)?)?;
    len = checked_len_add(len, U16_BYTES)?;
    len = checked_len_add(len, U32_BYTES + envelope.payload.len())?;
    Ok(len)
}

fn encoded_staged_welcome_len(welcome: &StagedWelcomeV1) -> Result<usize, ClientStoreError> {
    let mut len = U32_BYTES + welcome.welcome_id.len();
    len = checked_len_add(len, U32_BYTES + welcome.welcome_payload.len())?;
    len = checked_len_add(len, U32_BYTES + welcome.ratchet_tree_payload.len())?;
    Ok(len)
}

fn encoded_device_ref_len(device: &DeviceRef) -> Result<usize, ClientStoreError> {
    let mut len = U32_BYTES + device.account_id.len();
    len = checked_len_add(len, U32_BYTES + device.device_id.len())?;
    Ok(len)
}

fn checked_len_add(left: usize, right: usize) -> Result<usize, ClientStoreError> {
    left.checked_add(right)
        .ok_or(ClientStoreError::StateSnapshotLengthOverflow)
}

fn append_string_field(
    out: &mut Vec<u8>,
    field: &str,
    value: &str,
    max_bytes: u32,
) -> Result<(), ClientStoreError> {
    validate_string_bytes(field, value, max_bytes).map_err(ClientError::from)?;
    append_raw_len_prefixed(out, value.as_bytes())
}

fn append_bytes_field(
    out: &mut Vec<u8>,
    field: &str,
    bytes: &[u8],
    max_bytes: u32,
) -> Result<(), ClientStoreError> {
    validate_bytes_len(field, bytes.len(), max_bytes).map_err(ClientError::from)?;
    append_raw_len_prefixed(out, bytes)
}

fn append_count(
    out: &mut Vec<u8>,
    field: &str,
    count: usize,
    max_items: u32,
) -> Result<(), ClientStoreError> {
    validate_item_count(field, count, max_items).map_err(ClientError::from)?;
    let count = u32::try_from(count).map_err(|_| ClientStoreError::StateSnapshotLengthOverflow)?;
    out.extend_from_slice(&count.to_be_bytes());
    Ok(())
}

fn append_raw_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ClientStoreError> {
    let len =
        u32::try_from(bytes.len()).map_err(|_| ClientStoreError::StateSnapshotLengthOverflow)?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

struct ClientStateCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ClientStateCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        debug_assert!(!bytes.is_empty());
        Self { bytes, offset: 0 }
    }

    fn take_magic(&mut self) -> Result<(), ClientStoreError> {
        let magic = self.take_bytes(CLIENT_STATE_SNAPSHOT_MAGIC.len())?;
        if magic == CLIENT_STATE_SNAPSHOT_MAGIC {
            Ok(())
        } else {
            Err(ClientStoreError::StateSnapshotMagic)
        }
    }

    fn take_u16(&mut self) -> Result<u16, ClientStoreError> {
        let bytes = self.take_bytes(U16_BYTES)?;
        Ok(u16::from_be_bytes(
            bytes
                .try_into()
                .map_err(|_| ClientStoreError::StateSnapshotTruncated)?,
        ))
    }

    fn take_u32(&mut self) -> Result<u32, ClientStoreError> {
        let bytes = self.take_bytes(U32_BYTES)?;
        Ok(u32::from_be_bytes(
            bytes
                .try_into()
                .map_err(|_| ClientStoreError::StateSnapshotTruncated)?,
        ))
    }

    fn take_u64(&mut self) -> Result<u64, ClientStoreError> {
        let bytes = self.take_bytes(U64_BYTES)?;
        Ok(u64::from_be_bytes(
            bytes
                .try_into()
                .map_err(|_| ClientStoreError::StateSnapshotTruncated)?,
        ))
    }

    fn take_count(&mut self, field: &str, max_items: u32) -> Result<usize, ClientStoreError> {
        let count = self.take_u32()? as usize;
        validate_item_count(field, count, max_items).map_err(ClientError::from)?;
        Ok(count)
    }

    fn take_string(&mut self, field: &str, max_bytes: u32) -> Result<String, ClientStoreError> {
        let bytes = self.take_vec(field, max_bytes)?;
        let value = String::from_utf8(bytes).map_err(|_| ClientStoreError::StateSnapshotUtf8)?;
        validate_string_bytes(field, &value, max_bytes).map_err(ClientError::from)?;
        Ok(value)
    }

    fn take_vec(&mut self, field: &str, max_bytes: u32) -> Result<Vec<u8>, ClientStoreError> {
        let len = self.take_u32()? as usize;
        validate_bytes_len(field, len, max_bytes).map_err(ClientError::from)?;
        Ok(self.take_bytes(len)?.to_vec())
    }

    fn take_upload_key_package_request(
        &mut self,
    ) -> Result<UploadKeyPackageRequest, ClientStoreError> {
        let request = UploadKeyPackageRequest {
            key_package_id: self.take_string(
                "pending_key_package_upload.key_package_id",
                MAX_OBJECT_ID_BYTES,
            )?,
            owner: self.take_device_ref("pending_key_package_upload.owner")?,
            key_package_ref: self.take_string(
                "pending_key_package_upload.key_package_ref",
                MAX_OBJECT_ID_BYTES,
            )?,
            key_package_hash: self.take_string(
                "pending_key_package_upload.key_package_hash",
                MAX_OBJECT_ID_BYTES,
            )?,
            key_package_payload: self.take_vec(
                "pending_key_package_upload.key_package_payload",
                MAX_KEY_PACKAGE_PAYLOAD_BYTES,
            )?,
        };
        request.validate_limits().map_err(ClientError::from)?;
        Ok(request)
    }

    fn take_link_fanout_state(
        &mut self,
        snapshot_version: u16,
    ) -> Result<LinkFanoutState, ClientStoreError> {
        let fanout_id = self.take_string("link_fanout.fanout_id", MAX_OBJECT_ID_BYTES)?;
        let target_device = self.take_device_ref("link_fanout.target_device")?;
        let after_room_id =
            self.take_optional_string("link_fanout.after_room_id", MAX_ROOM_ID_BYTES)?;
        let discovery_complete = self.take_bool()?;
        let rooms = {
            let count = self.take_count("link_fanout.rooms", MAX_LINK_FANOUT_ROOMS)?;
            let mut rooms = Vec::with_capacity(count);
            for _ in 0..count {
                rooms.push(self.take_link_fanout_room_state()?);
            }
            rooms
        };
        let bootstrap_export = if snapshot_version >= CLIENT_STATE_SNAPSHOT_VERSION {
            let encoded = self.take_vec(
                "link_fanout.bootstrap_export",
                MAX_CLIENT_STATE_PLAINTEXT_BYTES,
            )?;
            serde_json::from_slice(&encoded)
                .map_err(|_| ClientStoreError::DecodeLinkBootstrapExport)?
        } else {
            LinkBootstrapExportState::WaitingForFanout
        };
        let fanout = LinkFanoutState {
            fanout_id,
            target_device,
            after_room_id,
            discovery_complete,
            rooms,
            bootstrap_export,
        };
        fanout.validate_limits()?;
        Ok(fanout)
    }

    fn take_link_fanout_room_state(&mut self) -> Result<LinkFanoutRoomState, ClientStoreError> {
        let plan = self.take_link_fanout_room_plan()?;
        let claimed_key_package = if self.take_bool()? {
            Some(self.take_claimed_key_package()?)
        } else {
            None
        };
        let status = match self.take_u16()? {
            LINK_FANOUT_STATUS_PENDING => LinkFanoutRoomStatus::Pending,
            LINK_FANOUT_STATUS_PREPARED => LinkFanoutRoomStatus::Prepared {
                prepared: Box::new(self.take_prepared_commit()?),
            },
            LINK_FANOUT_STATUS_DONE => LinkFanoutRoomStatus::Done {
                accepted_seq: self.take_u64()?,
            },
            other => {
                return Err(ClientStoreError::StateSnapshotEnum {
                    field: "link_fanout.status",
                    value: u64::from(other),
                });
            }
        };
        let room = LinkFanoutRoomState {
            plan,
            claimed_key_package,
            status,
        };
        room.validate_limits()?;
        Ok(room)
    }

    fn take_claimed_key_package(&mut self) -> Result<ClaimKeyPackageResult, ClientStoreError> {
        let claimed_key_package = ClaimKeyPackageResult {
            key_package_id: self
                .take_string("claimed_key_package.key_package_id", MAX_OBJECT_ID_BYTES)?,
            owner: self.take_device_ref("claimed_key_package.owner")?,
            key_package_ref: self
                .take_string("claimed_key_package.key_package_ref", MAX_OBJECT_ID_BYTES)?,
            key_package_hash: self
                .take_string("claimed_key_package.key_package_hash", MAX_OBJECT_ID_BYTES)?,
            key_package_payload: self.take_vec(
                "claimed_key_package.key_package_payload",
                MAX_KEY_PACKAGE_PAYLOAD_BYTES,
            )?,
            lease_token: self
                .take_string("claimed_key_package.lease_token", MAX_OBJECT_ID_BYTES)?,
        };
        validate_claimed_key_package(&claimed_key_package)?;
        Ok(claimed_key_package)
    }

    fn take_link_fanout_room_plan(&mut self) -> Result<LinkFanoutRoomPlan, ClientStoreError> {
        let plan = LinkFanoutRoomPlan {
            room_id: self.take_string("link_fanout.room_id", MAX_ROOM_ID_BYTES)?,
            key_package_id: self.take_string("link_fanout.key_package_id", MAX_OBJECT_ID_BYTES)?,
            welcome_id: self.take_string("link_fanout.welcome_id", MAX_OBJECT_ID_BYTES)?,
            idempotency_key: self
                .take_string("link_fanout.idempotency_key", MAX_IDEMPOTENCY_KEY_BYTES)?,
        };
        plan.validate_limits()?;
        Ok(plan)
    }

    fn take_prepared_commit(&mut self) -> Result<PreparedCommit, ClientStoreError> {
        let prepared = PreparedCommit {
            request: self.take_submit_commit_request()?,
            message_id: self.take_string("prepared_commit.message_id", MAX_OBJECT_ID_BYTES)?,
        };
        prepared.validate_limits()?;
        Ok(prepared)
    }

    fn take_submit_commit_request(&mut self) -> Result<SubmitCommitRequest, ClientStoreError> {
        let request = SubmitCommitRequest {
            room_id: self.take_string("commit_request.room_id", MAX_ROOM_ID_BYTES)?,
            sender: self.take_device_ref("commit_request.sender")?,
            expected_epoch: self.take_u64()?,
            envelope: self.take_envelope()?,
            membership_delta: self.take_membership_delta()?,
            staged_welcomes: {
                let count = self.take_count(
                    "commit_request.staged_welcomes",
                    MAX_STAGED_WELCOMES_PER_COMMIT,
                )?;
                let mut welcomes = Vec::with_capacity(count);
                for _ in 0..count {
                    welcomes.push(self.take_staged_welcome()?);
                }
                welcomes
            },
            idempotency_key: self
                .take_string("commit_request.idempotency_key", MAX_IDEMPOTENCY_KEY_BYTES)?,
        };
        request.validate_limits().map_err(ClientError::from)?;
        Ok(request)
    }

    fn take_membership_delta(&mut self) -> Result<MembershipDeltaV1, ClientStoreError> {
        let base_epoch = self.take_u64()?;
        let post_commit_epoch = self.take_u64()?;
        let commit_message_id =
            self.take_string("membership_delta.commit_message_id", MAX_OBJECT_ID_BYTES)?;
        let add_count = self.take_count("membership_delta.adds", MAX_STAGED_WELCOMES_PER_COMMIT)?;
        let mut adds = Vec::with_capacity(add_count);
        for _ in 0..add_count {
            adds.push(MembershipAddV1 {
                device: self.take_device_ref("membership_delta.add.device")?,
                key_package_id: self
                    .take_string("membership_delta.add.key_package_id", MAX_OBJECT_ID_BYTES)?,
                key_package_ref: self
                    .take_string("membership_delta.add.key_package_ref", MAX_OBJECT_ID_BYTES)?,
                key_package_hash: self
                    .take_string("membership_delta.add.key_package_hash", MAX_OBJECT_ID_BYTES)?,
                welcome_id: self
                    .take_string("membership_delta.add.welcome_id", MAX_OBJECT_ID_BYTES)?,
            });
        }
        let remove_count =
            self.take_count("membership_delta.removes", MAX_STAGED_WELCOMES_PER_COMMIT)?;
        let mut removes = Vec::with_capacity(remove_count);
        for _ in 0..remove_count {
            removes.push(MembershipRemoveV1 {
                device: self.take_device_ref("membership_delta.remove.device")?,
                removed_leaf_index: self.take_u32()?,
            });
        }
        let delta = MembershipDeltaV1 {
            base_epoch,
            post_commit_epoch,
            commit_message_id,
            adds,
            removes,
        };
        delta.validate_limits().map_err(ClientError::from)?;
        Ok(delta)
    }

    fn take_envelope(&mut self) -> Result<finitechat_proto::FiniteEnvelope, ClientStoreError> {
        let envelope = finitechat_proto::FiniteEnvelope {
            room_id: self.take_string("envelope.room_id", MAX_ROOM_ID_BYTES)?,
            mls_group_id: self.take_string("envelope.mls_group_id", MAX_MLS_GROUP_ID_BYTES)?,
            epoch: self.take_u64()?,
            sender: self.take_device_ref("envelope.sender")?,
            kind: self.take_log_entry_kind()?,
            payload: self.take_vec("envelope.payload", MAX_ENVELOPE_PAYLOAD_BYTES)?,
        };
        envelope.validate_limits().map_err(ClientError::from)?;
        Ok(envelope)
    }

    fn take_staged_welcome(&mut self) -> Result<StagedWelcomeV1, ClientStoreError> {
        let welcome = StagedWelcomeV1 {
            welcome_id: self.take_string("staged_welcome.welcome_id", MAX_OBJECT_ID_BYTES)?,
            welcome_payload: self
                .take_vec("staged_welcome.welcome_payload", MAX_WELCOME_PAYLOAD_BYTES)?,
            ratchet_tree_payload: self.take_vec(
                "staged_welcome.ratchet_tree_payload",
                MAX_RATCHET_TREE_PAYLOAD_BYTES,
            )?,
        };
        welcome.validate_limits().map_err(ClientError::from)?;
        Ok(welcome)
    }

    fn take_device_ref(&mut self, field: &'static str) -> Result<DeviceRef, ClientStoreError> {
        let device = DeviceRef {
            account_id: self.take_string(field, MAX_ACCOUNT_ID_BYTES)?,
            device_id: self.take_string(field, MAX_DEVICE_ID_BYTES)?,
        };
        device.validate_limits().map_err(ClientError::from)?;
        Ok(device)
    }

    fn take_optional_string(
        &mut self,
        field: &str,
        max_bytes: u32,
    ) -> Result<Option<String>, ClientStoreError> {
        if self.take_bool()? {
            Ok(Some(self.take_string(field, max_bytes)?))
        } else {
            Ok(None)
        }
    }

    fn take_bool(&mut self) -> Result<bool, ClientStoreError> {
        match self.take_u16()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(ClientStoreError::StateSnapshotEnum {
                field: "bool",
                value: u64::from(other),
            }),
        }
    }

    fn take_log_entry_kind(&mut self) -> Result<LogEntryKind, ClientStoreError> {
        decode_log_entry_kind(self.take_u16()?)
    }

    fn take_bytes(&mut self, len: usize) -> Result<&'a [u8], ClientStoreError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(ClientStoreError::StateSnapshotLengthOverflow)?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(ClientStoreError::StateSnapshotTruncated)?;
        self.offset = end;
        debug_assert!(self.offset <= self.bytes.len());
        Ok(bytes)
    }

    fn finish(&self) -> Result<(), ClientStoreError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(ClientStoreError::StateSnapshotTrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    #[test]
    fn client_store_encryption_key_matches_pinned_vector() {
        // Pinned HKDF vector for the client-store key derivation domain
        // (`finitechat.client-store-key.v1`). The account secret now arrives
        // via the shared Finite identity file; existing encrypted stores
        // keyed off the same secret must keep opening, so this derivation
        // must never change for a given secret.
        let secret = NostrSecretKey::from_bytes([7; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let key = ClientStoreEncryptionKey::from_nostr_secret(&secret, "phone").unwrap();
        let mut key_hex = String::with_capacity(64);
        for byte in key.as_bytes() {
            key_hex.push_str(&format!("{byte:02x}"));
        }
        assert_eq!(
            key_hex,
            "cb0a531322f96b78a76cec704b201c2ccc4695855f78022a024e50e8349bb656"
        );
    }

    #[test]
    fn current_reader_opens_actual_predecessor_v8_encrypted_snapshot() {
        // This SQLite file was emitted by the unmodified V8 writer at
        // 5276a77d63a1b5069d21f3c2f5289947728f2784. Keep it static: encoding a
        // "legacy" value with today's writer would not prove staggered-version
        // compatibility.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("client.sqlite3");
        std::fs::write(
            &db_path,
            include_bytes!("../tests/fixtures/client-state-v8-predecessor.sqlite3"),
        )
        .unwrap();
        let secret = NostrSecretKey::from_bytes([42; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "v8-predecessor-device";
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let store = SqliteClientStore::open(&db_path, options).unwrap();
        let restored = store
            .load_device(FiniteChatDeviceConfig {
                account_secret_key: secret,
                device_id: device_id.to_owned(),
                now_unix_seconds: NOW,
                credential_not_before_unix_seconds: NOW.saturating_sub(60),
                credential_not_after_unix_seconds: NOW.saturating_add(600),
            })
            .unwrap();
        let fanout = restored.link_fanout("v8-predecessor-fanout").unwrap();
        assert_eq!(fanout.target_device.device_id, "v8-target-device");
        assert_eq!(
            fanout.bootstrap_export,
            LinkBootstrapExportState::WaitingForFanout,
            "V8 had no bootstrap-export field; V9 must supply the safe restart default"
        );
    }

    #[test]
    fn current_reader_opens_actual_candidate_v9_encrypted_snapshot() {
        // This SQLite file was emitted by the V9 candidate before the
        // expand/contract branches were restacked. Keep it static so the
        // release reader must remain compatible with state already produced
        // during candidate testing.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("client.sqlite3");
        std::fs::write(
            &db_path,
            include_bytes!("../tests/fixtures/client-state-v9-candidate.sqlite3"),
        )
        .unwrap();
        let secret = NostrSecretKey::from_bytes([43; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "v9-candidate-device";
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let store = SqliteClientStore::open(&db_path, options).unwrap();
        let restored = store
            .load_device(FiniteChatDeviceConfig {
                account_secret_key: secret,
                device_id: device_id.to_owned(),
                now_unix_seconds: NOW,
                credential_not_before_unix_seconds: NOW.saturating_sub(60),
                credential_not_after_unix_seconds: NOW.saturating_add(600),
            })
            .unwrap();
        let fanout = restored.link_fanout("v9-candidate-fanout").unwrap();
        assert_eq!(fanout.target_device.device_id, "v9-target-device");
        fanout.validate_limits().unwrap();
    }

    #[test]
    fn refreshed_device_clock_accepts_key_packages_created_after_runtime_open() {
        let alice_secret = NostrSecretKey::from_bytes([1; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let bob_secret = NostrSecretKey::from_bytes([2; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let mut alice = FiniteChatDevice::new(FiniteChatDeviceConfig {
            account_secret_key: alice_secret,
            device_id: "alice-phone".to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(600),
        })
        .unwrap();
        alice
            .create_group_state("room-clock", "mls-room-clock")
            .unwrap();

        let bob = FiniteChatDevice::new(FiniteChatDeviceConfig {
            account_secret_key: bob_secret,
            device_id: "bob-phone".to_owned(),
            now_unix_seconds: NOW + 120,
            credential_not_before_unix_seconds: NOW + 60,
            credential_not_after_unix_seconds: NOW + 600,
        })
        .unwrap();
        let bob_key_package = bob.upload_key_package_auto_id_request().unwrap();
        let claimed = ClaimKeyPackageResult {
            lease_token: "lease-clock".to_owned(),
            key_package_id: bob_key_package.key_package_id,
            owner: bob_key_package.owner,
            key_package_ref: bob_key_package.key_package_ref,
            key_package_hash: bob_key_package.key_package_hash,
            key_package_payload: bob_key_package.key_package_payload,
        };

        let stale_error = alice
            .prepare_add_member_commit(
                "room-clock",
                &claimed,
                "welcome-clock-stale",
                "direct-add-clock-stale",
            )
            .unwrap_err();
        assert!(
            matches!(
                stale_error,
                ClientError::MlsCredential(MlsCredentialError::CredentialNotYetValid)
            ),
            "expected stale clock to reject Bob's newer credential, got {stale_error:?}"
        );

        alice.set_now_unix_seconds(NOW + 120);
        let prepared = alice
            .prepare_add_member_commit(
                "room-clock",
                &claimed,
                "welcome-clock-fresh",
                "direct-add-clock-fresh",
            )
            .unwrap();
        assert_eq!(prepared.request.room_id, "room-clock");
    }

    #[test]
    fn sync_applies_own_pending_commit_even_when_cursor_already_advanced() {
        let alice_secret = NostrSecretKey::from_bytes([1; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let mut alice = FiniteChatDevice::new(FiniteChatDeviceConfig {
            account_secret_key: alice_secret,
            device_id: "alice-electron".to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(600),
        })
        .unwrap();
        let room_id = "room-stale-pending-commit";
        alice
            .create_group_state(room_id, "mls-room-stale-pending-commit")
            .unwrap();

        let prepared = alice
            .prepare_self_update_commit(room_id, "self-update-stale-pending")
            .unwrap();
        assert!(alice.has_pending_commit(room_id).unwrap());
        let seq = 7;
        alice.set_last_applied_seq(room_id, seq).unwrap();

        let entry = RoomLogEntry {
            room_id: room_id.to_owned(),
            seq,
            message_id: prepared.message_id.clone(),
            sender: alice.device_ref().clone(),
            kind: LogEntryKind::Commit,
            epoch: prepared.request.expected_epoch,
            envelope: prepared.request.envelope,
            idempotency_key: prepared.request.idempotency_key,
            timestamp_unix_seconds: NOW,
        };
        let applied = apply_log_entry_in_memory(&mut alice, room_id, &entry)
            .expect("old own commit should still merge pending state")
            .expect("own commit merge should be reported as applied");

        assert!(matches!(applied, AppliedLogEntry::Commit { .. }));
        assert!(
            !alice.has_pending_commit(room_id).unwrap(),
            "own commit at or behind the cursor must clear pending MLS state"
        );
    }

    #[test]
    fn runtime_sync_overlaps_when_pending_commit_cursor_already_advanced() {
        struct PendingCommitDelivery {
            entry: RoomLogEntry,
            requested_after_seq: Option<u64>,
        }

        impl RuntimeDelivery for PendingCommitDelivery {
            type Error = String;

            fn key_package_inventory(
                &mut self,
                owner: &DeviceRef,
            ) -> Result<KeyPackageInventory, Self::Error> {
                Ok(KeyPackageInventory {
                    owner: owner.clone(),
                    available: 0,
                    leased: 0,
                })
            }

            fn upload_key_package(
                &mut self,
                _request: UploadKeyPackageRequest,
            ) -> Result<(), Self::Error> {
                Ok(())
            }

            fn claim_key_package_for_device(
                &mut self,
                _owner: &DeviceRef,
            ) -> Result<Option<ClaimKeyPackageResult>, Self::Error> {
                unimplemented!("not used by runtime sync")
            }

            fn claim_key_package_for_account(
                &mut self,
                _account_id: &str,
            ) -> Result<Option<ClaimKeyPackageResult>, Self::Error> {
                unimplemented!("not used by runtime sync")
            }

            fn submit_commit(
                &mut self,
                _request: SubmitCommitRequest,
            ) -> Result<CommitAccepted, Self::Error> {
                unimplemented!("not used by runtime sync")
            }

            fn list_account_rooms(
                &mut self,
                _request: ListAccountRoomsRequest,
            ) -> Result<ListAccountRoomsPage, Self::Error> {
                unimplemented!("not used by runtime sync")
            }

            fn claim_welcomes(
                &mut self,
                _device: &DeviceRef,
            ) -> Result<Vec<WelcomeRecord>, Self::Error> {
                Ok(Vec::new())
            }

            fn ack_welcome(&mut self, _welcome_id: &str) -> Result<(), Self::Error> {
                Ok(())
            }

            fn sync_events(
                &mut self,
                room_id: &str,
                _requester: &DeviceRef,
                after_seq: u64,
            ) -> Result<SyncEventsPage, Self::Error> {
                self.requested_after_seq = Some(after_seq);
                let entries = (room_id == self.entry.room_id && after_seq < self.entry.seq)
                    .then(|| self.entry.clone())
                    .into_iter()
                    .collect::<Vec<_>>();
                Ok(SyncEventsPage {
                    entries,
                    next_after_seq: self.entry.seq,
                    has_more: false,
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let alice_secret = NostrSecretKey::from_bytes([1; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let mut alice = FiniteChatDevice::new(FiniteChatDeviceConfig {
            account_secret_key: alice_secret.clone(),
            device_id: "alice-electron".to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(600),
        })
        .unwrap();
        let room_id = "room-pending-overlap";
        alice
            .create_group_state(room_id, "mls-pending-overlap")
            .unwrap();
        let prepared = alice
            .prepare_self_update_commit(room_id, "self-update-overlap")
            .unwrap();
        let seq = 7;
        alice.set_last_applied_seq(room_id, seq).unwrap();

        let mut store = SqliteClientStore::open(
            dir.path().join("client.sqlite3"),
            SqliteClientStoreOptions::from_nostr_secret(&alice_secret, "alice-electron").unwrap(),
        )
        .unwrap();
        store.save_device_state(&alice).unwrap();
        let entry = RoomLogEntry {
            room_id: room_id.to_owned(),
            seq,
            message_id: prepared.message_id.clone(),
            sender: alice.device_ref().clone(),
            kind: LogEntryKind::Commit,
            epoch: prepared.request.expected_epoch,
            envelope: prepared.request.envelope,
            idempotency_key: prepared.request.idempotency_key,
            timestamp_unix_seconds: NOW,
        };
        let mut delivery = PendingCommitDelivery {
            entry,
            requested_after_seq: None,
        };
        let options = RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 1,
        };

        run_runtime_sync_tick(&mut store, &mut alice, &mut delivery, &options).unwrap();

        assert_eq!(delivery.requested_after_seq, Some(0));
        assert!(
            !alice.has_pending_commit(room_id).unwrap(),
            "runtime sync must recover a pending own commit even when the cursor was advanced"
        );
    }

    #[test]
    fn sqlite_client_store_persists_app_messages_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([7; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "phone";
        let config = FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        };
        let device = FiniteChatDevice::new(config).unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let db_path = dir.path().join("client.sqlite3");

        let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
        store.save_device_state(&device).unwrap();
        assert_eq!(
            store
                .load_device_ids_for_account(&owner.account_id)
                .unwrap(),
            vec![device_id.to_owned()]
        );
        let first = app_message(&owner, 1, "msg-1", "one");
        let second = app_message(&owner, 2, "msg-2", "two");
        store
            .save_app_messages(&owner, &[first.clone(), second.clone()], 10)
            .unwrap();
        assert!(!sqlite_table_has_column(&store.conn, "client_app_messages", "plaintext").unwrap());
        assert!(sqlite_table_has_column(&store.conn, "client_app_messages", "ciphertext").unwrap());
        assert!(
            sqlite_table_has_column(&store.conn, "client_app_messages", "timestamp_unix_seconds")
                .unwrap()
        );
        assert_eq!(
            sqlite_table_column_default(
                &store.conn,
                "client_app_messages",
                "timestamp_unix_seconds"
            )
            .unwrap(),
            None
        );
        assert_eq!(
            store.load_app_messages(&owner, 10).unwrap(),
            vec![first.clone(), second.clone()]
        );
        drop(store);

        let mut reopened = SqliteClientStore::open(&db_path, options).unwrap();
        assert_eq!(
            reopened.load_app_messages(&owner, 10).unwrap(),
            vec![first.clone(), second.clone()]
        );

        let replacement = StoredAppMessage {
            plaintext: b"one-edited".to_vec(),
            ..first.clone()
        };
        reopened
            .save_app_messages(&owner, std::slice::from_ref(&replacement), 10)
            .unwrap();
        assert_eq!(
            reopened.load_app_messages(&owner, 10).unwrap(),
            vec![replacement, second.clone()]
        );

        let third = app_message(&owner, 3, "msg-3", "three");
        reopened
            .save_app_messages(&owner, std::slice::from_ref(&third), 2)
            .unwrap();
        assert_eq!(
            reopened.load_app_messages(&owner, 10).unwrap(),
            vec![second, third]
        );
    }

    #[test]
    fn sqlite_client_store_persists_app_events_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([6; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "phone";
        let config = FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        };
        let device = FiniteChatDevice::new(config).unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let db_path = dir.path().join("client.sqlite3");

        let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
        store.save_device_state(&device).unwrap();
        let first = app_event(&owner, 1, "event-1", "one");
        let second = app_event(&owner, 2, "event-2", "two");
        store
            .save_app_events(&owner, &[first.clone(), second.clone()], 10)
            .unwrap();
        assert!(
            sqlite_table_has_column(&store.conn, "client_app_events", "timestamp_unix_seconds")
                .unwrap()
        );
        assert_eq!(
            sqlite_table_column_default(&store.conn, "client_app_events", "timestamp_unix_seconds")
                .unwrap(),
            None
        );
        assert_eq!(
            store.load_app_events(&owner, 10).unwrap(),
            vec![first.clone(), second.clone()]
        );
        drop(store);

        let mut reopened = SqliteClientStore::open(&db_path, options).unwrap();
        assert_eq!(
            reopened.load_app_events(&owner, 10).unwrap(),
            vec![first.clone(), second.clone()]
        );

        let replacement = StoredAppEvent {
            plaintext: b"one-edited".to_vec(),
            ..first.clone()
        };
        reopened
            .save_app_events(&owner, std::slice::from_ref(&replacement), 10)
            .unwrap();
        assert_eq!(
            reopened.load_app_events(&owner, 10).unwrap(),
            vec![replacement, second.clone()]
        );

        let third = app_event(&owner, 3, "event-3", "three");
        reopened
            .save_app_events(&owner, std::slice::from_ref(&third), 2)
            .unwrap();
        assert_eq!(
            reopened.load_app_events(&owner, 10).unwrap(),
            vec![second, third]
        );
    }

    #[test]
    fn production_combined_save_retains_events_beyond_transcript_cache_limit() {
        const FORMER_TRANSCRIPT_CACHE_LIMIT: u32 = 5_000;

        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([16; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "retention-phone";
        let device = FiniteChatDevice::new(FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        })
        .unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let mut store =
            SqliteClientStore::open(dir.path().join("client.sqlite3"), options).unwrap();
        store.save_device_state(&device).unwrap();
        let events = (0..FORMER_TRANSCRIPT_CACHE_LIMIT + 1)
            .map(|index| {
                app_event(
                    &owner,
                    u64::from(index) + 1,
                    &format!("event-{index:05}"),
                    "retained",
                )
            })
            .collect::<Vec<_>>();
        store
            .save_device_state_and_app_messages_and_events(&device, &[], &events)
            .unwrap();

        let first_page = store
            .load_app_events_for_room_page(
                &owner,
                "room-store",
                u64::from(FORMER_TRANSCRIPT_CACHE_LIMIT) + 1,
                0,
                "",
                FORMER_TRANSCRIPT_CACHE_LIMIT,
            )
            .unwrap();
        assert_eq!(first_page.len(), FORMER_TRANSCRIPT_CACHE_LIMIT as usize);
        let last = first_page.last().unwrap();
        let second_page = store
            .load_app_events_for_room_page(
                &owner,
                "room-store",
                u64::from(FORMER_TRANSCRIPT_CACHE_LIMIT) + 1,
                last.seq,
                &last.message_id,
                1,
            )
            .unwrap();
        assert_eq!(second_page.len(), 1);
        assert_eq!(second_page[0].message_id, "event-05000");
    }

    #[test]
    fn production_combined_save_retains_messages_beyond_former_cache_limit() {
        const MESSAGE_COUNT: u32 = 5_201;

        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([18; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "complete-history-phone";
        let device = FiniteChatDevice::new(FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        })
        .unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let mut store =
            SqliteClientStore::open(dir.path().join("client.sqlite3"), options).unwrap();
        let messages = (0..MESSAGE_COUNT)
            .map(|index| {
                app_message(
                    &owner,
                    u64::from(index) + 1,
                    &format!("message-{index:05}"),
                    "retained",
                )
            })
            .collect::<Vec<_>>();

        store
            .save_device_state_and_app_messages_and_events(&device, &messages, &[])
            .unwrap();

        let loaded = store
            .load_app_messages(&owner, MAX_STORED_APP_MESSAGES)
            .unwrap();
        assert_eq!(loaded.len(), MESSAGE_COUNT as usize);
        assert_eq!(loaded.first().unwrap().message_id, "message-00000");
        assert_eq!(loaded.last().unwrap().message_id, "message-05200");
    }

    #[test]
    fn atomic_event_import_replays_exact_duplicates_and_rolls_back_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([17; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "import-phone";
        let device = FiniteChatDevice::new(FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        })
        .unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let mut store =
            SqliteClientStore::open(dir.path().join("client.sqlite3"), options).unwrap();
        store.save_device_state(&device).unwrap();
        let canonical = app_event(&owner, 1, "event-1", "canonical");
        store
            .import_app_events_atomically(&owner, std::slice::from_ref(&canonical))
            .unwrap();
        store
            .import_app_events_atomically(&owner, std::slice::from_ref(&canonical))
            .unwrap();
        assert!(
            store
                .has_app_event_identity(
                    &owner,
                    &canonical.room_id,
                    canonical.seq,
                    &canonical.message_id,
                    &canonical.sender,
                    canonical.timestamp_unix_seconds,
                )
                .unwrap()
        );
        assert!(
            !store
                .has_app_event_identity(
                    &owner,
                    &canonical.room_id,
                    canonical.seq + 1,
                    &canonical.message_id,
                    &canonical.sender,
                    canonical.timestamp_unix_seconds,
                )
                .unwrap(),
            "message ID alone must not certify mismatched canonical history"
        );

        let new_event = app_event(&owner, 2, "event-2", "new");
        let conflicting = StoredAppEvent {
            plaintext: b"conflicting".to_vec(),
            ..canonical.clone()
        };
        assert!(matches!(
            store.import_app_events_atomically(&owner, &[new_event.clone(), conflicting]),
            Err(ClientStoreError::StoredAppEventConflict { .. })
        ));
        assert!(
            !store
                .has_app_event(&owner, "room-store", "event-2")
                .unwrap()
        );
        assert_eq!(store.load_app_events(&owner, 10).unwrap(), vec![canonical]);
    }

    #[test]
    fn device_link_bootstrap_staging_survives_cursor_advance_and_publishes_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([19; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "linked-phone";
        let config = FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        };
        let mut device = FiniteChatDevice::new(config).unwrap();
        device
            .create_group_state("room-bootstrap", "mls-room-bootstrap")
            .unwrap();
        device.set_last_applied_seq("room-bootstrap", 50).unwrap();
        let owner = device.device_ref().clone();
        let source = DeviceRef {
            account_id: owner.account_id.clone(),
            device_id: "source-phone".to_owned(),
        };
        let chunks = test_device_link_bootstrap_chunks(&owner, &source);
        let chunk_receipt =
            StoredDeviceLinkBootstrapReceipt::from_bootstrap(&source, &chunks[0]).unwrap();
        let mut manifest_only = chunks[0].clone();
        manifest_only.history.clear();
        assert_eq!(
            StoredDeviceLinkBootstrapReceipt::from_bootstrap(&source, &manifest_only).unwrap(),
            chunk_receipt,
            "source planning and target chunks must derive the same exact manifest receipt"
        );
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let db_path = dir.path().join("client.sqlite3");
        let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
        let first_control = test_device_link_bootstrap_control_event(&source, 50, &chunks[0]);

        // This is the production crash boundary: cursor/device state, control
        // event, and encrypted staging all enter one SQLite transaction.
        store
            .save_device_state_and_app_messages_and_events(
                &device,
                &[],
                std::slice::from_ref(&first_control),
            )
            .unwrap();
        assert!(store.load_app_rooms(&owner).unwrap().is_empty());
        assert!(
            !store
                .has_app_event(&owner, "room-bootstrap", "history-1")
                .unwrap()
        );
        assert!(matches!(
            store
                .stage_device_link_bootstrap_chunk(&owner, &source, 50, &chunks[0])
                .unwrap(),
            DeviceLinkBootstrapStageOutcome::ExactDuplicate {
                received_chunks: 1,
                expected_chunks: 2
            }
        ));
        assert!(
            !sqlite_table_has_column(
                &store.conn,
                "client_device_link_bootstrap_chunks",
                "plaintext"
            )
            .unwrap()
        );
        drop(store);

        let mut reopened = SqliteClientStore::open(&db_path, options).unwrap();
        let restored = reopened
            .load_device(FiniteChatDeviceConfig {
                account_secret_key: secret,
                device_id: device_id.to_owned(),
                now_unix_seconds: NOW,
                credential_not_before_unix_seconds: NOW.saturating_sub(60),
                credential_not_after_unix_seconds: NOW.saturating_add(60),
            })
            .unwrap();
        assert_eq!(restored.last_applied_seq("room-bootstrap").unwrap(), 50);
        let pending = reopened
            .load_pending_device_link_bootstraps(&owner)
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert!(!pending[0].is_complete());

        let ready = match reopened
            .stage_device_link_bootstrap_chunk(&owner, &source, 51, &chunks[1])
            .unwrap()
        {
            DeviceLinkBootstrapStageOutcome::Ready(ready) => ready,
            outcome => panic!("second chunk should complete staging, got {outcome:?}"),
        };
        let receipt = ready.receipt.clone();
        assert!(ready.is_complete());
        assert!(reopened.load_app_rooms(&owner).unwrap().is_empty());
        assert!(
            !reopened
                .has_app_event(&owner, "room-bootstrap", "history-1")
                .unwrap()
        );

        let outcome = reopened
            .commit_device_link_bootstrap_atomically(
                &owner,
                &receipt,
                &app_room("room-bootstrap", "Bootstrap Room"),
                &[],
            )
            .unwrap();
        assert_eq!(
            outcome,
            DeviceLinkBootstrapCommitOutcome::Committed(receipt.clone())
        );
        assert_eq!(
            reopened.load_app_rooms(&owner).unwrap(),
            vec![app_room("room-bootstrap", "Bootstrap Room")]
        );
        assert!(
            reopened
                .has_app_event(&owner, "room-bootstrap", "history-1")
                .unwrap()
        );
        assert!(
            reopened
                .has_app_event(&owner, "room-bootstrap", "history-2")
                .unwrap()
        );
        assert_eq!(
            reopened
                .load_completed_device_link_bootstrap_receipts(&owner)
                .unwrap(),
            vec![receipt.clone()]
        );
        assert_eq!(
            reopened
                .load_completed_device_link_bootstrap_receipt(
                    &owner,
                    "room-bootstrap",
                    "bootstrap-test",
                    &source,
                )
                .unwrap(),
            Some(receipt.clone())
        );
        assert!(matches!(
            reopened
                .stage_device_link_bootstrap_chunk(&owner, &source, 52, &chunks[1])
                .unwrap(),
            DeviceLinkBootstrapStageOutcome::AlreadyCommitted(found) if found == receipt
        ));
    }

    #[test]
    fn conflicting_device_link_bootstrap_duplicate_poison_is_durable() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([20; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "linked-phone";
        let device = FiniteChatDevice::new(FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        })
        .unwrap();
        let owner = device.device_ref().clone();
        let source = DeviceRef {
            account_id: owner.account_id.clone(),
            device_id: "source-phone".to_owned(),
        };
        let chunks = test_device_link_bootstrap_chunks(&owner, &source);
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let db_path = dir.path().join("client.sqlite3");
        let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
        store.save_device_state(&device).unwrap();
        assert!(matches!(
            store
                .stage_device_link_bootstrap_chunk(&owner, &source, 50, &chunks[0])
                .unwrap(),
            DeviceLinkBootstrapStageOutcome::Pending { .. }
        ));

        let mut conflicting = chunks[0].clone();
        conflicting.history[0].plaintext.push(b'!');
        assert!(matches!(
            store
                .stage_device_link_bootstrap_chunk(&owner, &source, 50, &conflicting)
                .unwrap(),
            DeviceLinkBootstrapStageOutcome::Poisoned
        ));
        drop(store);

        let mut reopened = SqliteClientStore::open(&db_path, options).unwrap();
        assert!(matches!(
            reopened
                .stage_device_link_bootstrap_chunk(&owner, &source, 50, &chunks[0])
                .unwrap(),
            DeviceLinkBootstrapStageOutcome::Poisoned
        ));
        assert!(
            reopened
                .load_pending_device_link_bootstraps(&owner)
                .unwrap()
                .is_empty()
        );
        assert!(
            reopened
                .load_completed_device_link_bootstrap_receipts(&owner)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn invalid_device_link_bootstrap_digest_never_publishes_partial_room() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([21; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "linked-phone";
        let device = FiniteChatDevice::new(FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        })
        .unwrap();
        let owner = device.device_ref().clone();
        let source = DeviceRef {
            account_id: owner.account_id.clone(),
            device_id: "source-phone".to_owned(),
        };
        let mut chunk = test_device_link_bootstrap_chunks(&owner, &source)[0].clone();
        chunk.chunk_count = 1;
        chunk.total_history_events = 1;
        chunk.history_sha256 = "0".repeat(64);
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let mut store =
            SqliteClientStore::open(dir.path().join("client.sqlite3"), options).unwrap();
        store.save_device_state(&device).unwrap();
        let ready = match store
            .stage_device_link_bootstrap_chunk(&owner, &source, 50, &chunk)
            .unwrap()
        {
            DeviceLinkBootstrapStageOutcome::Ready(ready) => ready,
            outcome => panic!("one chunk should be structurally ready, got {outcome:?}"),
        };
        assert_eq!(
            store
                .commit_device_link_bootstrap_atomically(
                    &owner,
                    &ready.receipt,
                    &app_room("room-bootstrap", "Bootstrap Room"),
                    &[],
                )
                .unwrap(),
            DeviceLinkBootstrapCommitOutcome::Poisoned
        );
        assert!(store.load_app_rooms(&owner).unwrap().is_empty());
        assert!(
            !store
                .has_app_event(&owner, "room-bootstrap", "history-1")
                .unwrap()
        );
        assert!(
            store
                .load_completed_device_link_bootstrap_receipts(&owner)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn sqlite_client_store_persists_app_outbox_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([11; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "phone";
        let config = FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        };
        let device = FiniteChatDevice::new(config).unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let db_path = dir.path().join("client.sqlite3");

        let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
        store.save_device_state(&device).unwrap();
        let pending = outbound_message(&owner, "pending body");
        store
            .save_app_outbox(&owner, std::slice::from_ref(&pending))
            .unwrap();
        assert_eq!(
            store.load_app_outbox(&owner).unwrap(),
            vec![pending.clone()]
        );
        drop(store);

        let mut reopened = SqliteClientStore::open(&db_path, options).unwrap();
        assert_eq!(
            reopened.load_app_outbox(&owner).unwrap(),
            vec![pending.clone()]
        );

        let failed = StoredOutboundMessage {
            server_delivery_state: StoredOutboundServerDeliveryState::Failed {
                reason: "network unavailable".to_owned(),
            },
            ..pending.clone()
        };
        reopened
            .save_app_outbox(&owner, std::slice::from_ref(&failed))
            .unwrap();
        assert_eq!(reopened.load_app_outbox(&owner).unwrap(), vec![failed]);

        reopened
            .delete_app_outbox_message(&owner, "room-store", &pending.message_id)
            .unwrap();
        assert!(reopened.load_app_outbox(&owner).unwrap().is_empty());
    }

    #[test]
    fn sqlite_client_store_rejects_legacy_outbox_metadata_shapes() {
        for case in ["missing timestamp", "legacy delivery state"] {
            let dir = tempfile::tempdir().unwrap();
            let secret = NostrSecretKey::from_bytes([15; NOSTR_SECRET_KEY_BYTES]).unwrap();
            let device_id = "phone";
            let config = FiniteChatDeviceConfig {
                account_secret_key: secret.clone(),
                device_id: device_id.to_owned(),
                now_unix_seconds: NOW,
                credential_not_before_unix_seconds: NOW.saturating_sub(60),
                credential_not_after_unix_seconds: NOW.saturating_add(60),
            };
            let device = FiniteChatDevice::new(config).unwrap();
            let owner = device.device_ref().clone();
            let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
            let db_path = dir.path().join("client.sqlite3");
            let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
            store.save_device_state(&device).unwrap();

            let message = outbound_message(&owner, "legacy pending body");
            let mut metadata = serde_json::to_value(StoredOutboundMessageMetadataV1 {
                sender: message.sender.clone(),
                plaintext: message.plaintext.clone(),
                local_state: message.local_state.clone(),
                server_delivery_state: message.server_delivery_state.clone(),
                append_request: message.append_request.clone(),
                timestamp_unix_seconds: message.timestamp_unix_seconds,
            })
            .unwrap();
            let metadata_object = metadata.as_object_mut().unwrap();
            match case {
                "missing timestamp" => {
                    metadata_object.remove("timestamp_unix_seconds");
                }
                "legacy delivery state" => {
                    metadata_object.remove("local_state");
                    metadata_object.remove("server_delivery_state");
                    metadata_object.remove("append_request");
                    metadata_object.insert(
                        "delivery_state".to_owned(),
                        serde_json::Value::String("Pending".to_owned()),
                    );
                }
                other => panic!("unknown outbox metadata case {other}"),
            }

            let sealed = encrypt_app_outbox_metadata_json(
                &options.encryption_key,
                &owner,
                &message.room_id,
                &message.message_id,
                &metadata,
            );
            store
                .conn
                .execute(
                    r#"
                    INSERT INTO client_app_outbox (
                      account_id,
                      device_id,
                      room_id,
                      message_id,
                      nonce,
                      ciphertext
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    "#,
                    params![
                        &owner.account_id,
                        &owner.device_id,
                        &message.room_id,
                        &message.message_id,
                        &sealed.nonce,
                        &sealed.ciphertext,
                    ],
                )
                .unwrap();

            let error = store.load_app_outbox(&owner).unwrap_err();
            assert!(
                matches!(error, ClientStoreError::DecodeAppOutboxMetadata),
                "{case} should fail closed, got {error}"
            );
        }
    }

    #[test]
    fn sqlite_client_store_rejects_encrypted_app_rows_without_timestamps() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([12; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "phone";
        let db_path = dir.path().join("client.sqlite3");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                r#"
                PRAGMA foreign_keys = ON;
                CREATE TABLE client_device_states (
                  account_id TEXT NOT NULL,
                  device_id TEXT NOT NULL,
                  nonce BLOB NOT NULL,
                  ciphertext BLOB NOT NULL,
                  PRIMARY KEY (account_id, device_id)
                );
                CREATE TABLE client_app_messages (
                  account_id TEXT NOT NULL,
                  device_id TEXT NOT NULL,
                  room_id TEXT NOT NULL,
                  seq INTEGER NOT NULL,
                  message_id TEXT NOT NULL,
                  sender_account_id TEXT NOT NULL,
                  sender_device_id TEXT NOT NULL,
                  nonce BLOB NOT NULL,
                  ciphertext BLOB NOT NULL,
                  PRIMARY KEY (account_id, device_id, room_id, message_id)
                );
                CREATE TABLE client_app_events (
                  account_id TEXT NOT NULL,
                  device_id TEXT NOT NULL,
                  room_id TEXT NOT NULL,
                  seq INTEGER NOT NULL,
                  message_id TEXT NOT NULL,
                  sender_account_id TEXT NOT NULL,
                  sender_device_id TEXT NOT NULL,
                  nonce BLOB NOT NULL,
                  ciphertext BLOB NOT NULL,
                  PRIMARY KEY (account_id, device_id, room_id, message_id)
                );
                "#,
            )
            .unwrap();
        }

        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let error = match SqliteClientStore::open(&db_path, options) {
            Ok(_) => panic!("pre-release app projection schema should be rejected"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error,
                ClientStoreError::LegacyAppProjectionSchema { ref table, ref reason }
                    if table == "client_app_messages"
                        && reason == "missing required column timestamp_unix_seconds"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn sqlite_client_store_rejects_defaulted_app_timestamps() {
        for table in ["client_app_messages", "client_app_events"] {
            let dir = tempfile::tempdir().unwrap();
            let secret = NostrSecretKey::from_bytes([18; NOSTR_SECRET_KEY_BYTES]).unwrap();
            let device_id = "phone";
            let db_path = dir.path().join("client.sqlite3");
            {
                let conn = Connection::open(&db_path).unwrap();
                conn.execute_batch(&format!(
                    r#"
                    PRAGMA foreign_keys = ON;
                    CREATE TABLE client_device_states (
                      account_id TEXT NOT NULL,
                      device_id TEXT NOT NULL,
                      nonce BLOB NOT NULL,
                      ciphertext BLOB NOT NULL,
                      PRIMARY KEY (account_id, device_id)
                    );
                    CREATE TABLE {table} (
                      account_id TEXT NOT NULL,
                      device_id TEXT NOT NULL,
                      room_id TEXT NOT NULL,
                      seq INTEGER NOT NULL,
                      message_id TEXT NOT NULL,
                      sender_account_id TEXT NOT NULL,
                      sender_device_id TEXT NOT NULL,
                      timestamp_unix_seconds INTEGER NOT NULL DEFAULT 0,
                      nonce BLOB NOT NULL,
                      ciphertext BLOB NOT NULL,
                      PRIMARY KEY (account_id, device_id, room_id, message_id)
                    );
                    "#
                ))
                .unwrap();
            }

            let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
            let error = match SqliteClientStore::open(&db_path, options) {
                Ok(_) => panic!("defaulted timestamp schema {table} should be rejected"),
                Err(error) => error,
            };
            assert!(
                matches!(
                    error,
                    ClientStoreError::LegacyAppProjectionSchema { table: ref found, ref reason }
                        if found == table
                            && reason == "timestamp_unix_seconds default is unsupported"
                ),
                "unexpected error for {table}: {error}"
            );
        }
    }

    #[test]
    fn sqlite_client_store_rejects_extra_app_projection_columns() {
        for (table, columns, primary_key) in [
            (
                "client_app_messages",
                &[
                    "room_id TEXT NOT NULL",
                    "seq INTEGER NOT NULL",
                    "message_id TEXT NOT NULL",
                    "sender_account_id TEXT NOT NULL",
                    "sender_device_id TEXT NOT NULL",
                    "timestamp_unix_seconds INTEGER NOT NULL",
                ][..],
                "account_id, device_id, room_id, message_id",
            ),
            (
                "client_app_events",
                &[
                    "room_id TEXT NOT NULL",
                    "seq INTEGER NOT NULL",
                    "message_id TEXT NOT NULL",
                    "sender_account_id TEXT NOT NULL",
                    "sender_device_id TEXT NOT NULL",
                    "timestamp_unix_seconds INTEGER NOT NULL",
                ][..],
                "account_id, device_id, room_id, message_id",
            ),
            (
                "client_app_outbox",
                &["room_id TEXT NOT NULL", "message_id TEXT NOT NULL"][..],
                "account_id, device_id, room_id, message_id",
            ),
            (
                "client_app_rooms",
                &["room_id TEXT NOT NULL"][..],
                "account_id, device_id, room_id",
            ),
            ("client_app_state", &[][..], "account_id, device_id"),
            (
                "client_app_profiles",
                &["profile_account_id TEXT NOT NULL"][..],
                "account_id, device_id, profile_account_id",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let secret = NostrSecretKey::from_bytes([19; NOSTR_SECRET_KEY_BYTES]).unwrap();
            let device_id = "phone";
            let db_path = dir.path().join("client.sqlite3");
            let mut table_columns = vec!["account_id TEXT NOT NULL", "device_id TEXT NOT NULL"];
            table_columns.extend_from_slice(columns);
            table_columns.extend_from_slice(&[
                "nonce BLOB NOT NULL",
                "ciphertext BLOB NOT NULL",
                "legacy_delivery_state TEXT",
            ]);
            let column_sql = table_columns.join(",\n                      ");
            {
                let conn = Connection::open(&db_path).unwrap();
                conn.execute_batch(&format!(
                    r#"
                    PRAGMA foreign_keys = ON;
                    CREATE TABLE client_device_states (
                      account_id TEXT NOT NULL,
                      device_id TEXT NOT NULL,
                      nonce BLOB NOT NULL,
                      ciphertext BLOB NOT NULL,
                      PRIMARY KEY (account_id, device_id)
                    );
                    CREATE TABLE {table} (
                      {column_sql},
                      PRIMARY KEY ({primary_key})
                    );
                    "#
                ))
                .unwrap();
            }

            let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
            let error = match SqliteClientStore::open(&db_path, options) {
                Ok(_) => panic!("extra app projection column on {table} should be rejected"),
                Err(error) => error,
            };
            assert!(
                matches!(
                    error,
                    ClientStoreError::LegacyAppProjectionSchema { table: ref found, ref reason }
                        if found == table && reason == "unsupported column legacy_delivery_state"
                ),
                "unexpected error for {table}: {error}"
            );
        }
    }

    #[test]
    fn sqlite_client_store_rejects_empty_legacy_unencrypted_tables() {
        for table in [
            LegacyClientStoreTable::OpenMlsStorage,
            LegacyClientStoreTable::Rooms,
            LegacyClientStoreTable::Profiles,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let secret = NostrSecretKey::from_bytes([13; NOSTR_SECRET_KEY_BYTES]).unwrap();
            let device_id = "phone";
            let db_path = dir.path().join("client.sqlite3");
            {
                let conn = Connection::open(&db_path).unwrap();
                conn.execute_batch(&format!(
                    "CREATE TABLE {} (id TEXT PRIMARY KEY);",
                    table.name()
                ))
                .unwrap();
            }

            let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
            let error = match SqliteClientStore::open(&db_path, options) {
                Ok(_) => panic!("legacy unencrypted table {table:?} should be rejected"),
                Err(error) => error,
            };
            assert!(
                matches!(
                    error,
                    ClientStoreError::LegacyUnencryptedStoreTable { table: found }
                        if found == table
                ),
                "unexpected error for {table:?}: {error}"
            );
        }
    }

    #[test]
    fn sqlite_client_store_rejects_legacy_room_metadata_shapes() {
        for (case, metadata) in [
            (
                "missing state",
                serde_json::json!({
                    "display_name": "Legacy Room",
                    "status": "connected",
                    "local_read_seq": 0
                }),
            ),
            (
                "missing status",
                serde_json::json!({
                    "display_name": "Legacy Room",
                    "state": "Connected",
                    "local_read_seq": 0
                }),
            ),
            (
                "missing local_read_seq",
                serde_json::json!({
                    "display_name": "Legacy Room",
                    "state": "Connected",
                    "status": "connected"
                }),
            ),
            (
                "legacy needs attention state",
                serde_json::json!({
                    "display_name": "Legacy Room",
                    "state": "NeedsAttention",
                    "status": "needs attention",
                    "local_read_seq": 0
                }),
            ),
            (
                "legacy offline state",
                serde_json::json!({
                    "display_name": "Legacy Room",
                    "state": "Offline",
                    "status": "offline",
                    "local_read_seq": 0
                }),
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let secret = NostrSecretKey::from_bytes([14; NOSTR_SECRET_KEY_BYTES]).unwrap();
            let device_id = "phone";
            let config = FiniteChatDeviceConfig {
                account_secret_key: secret.clone(),
                device_id: device_id.to_owned(),
                now_unix_seconds: NOW,
                credential_not_before_unix_seconds: NOW.saturating_sub(60),
                credential_not_after_unix_seconds: NOW.saturating_add(60),
            };
            let device = FiniteChatDevice::new(config).unwrap();
            let owner = device.device_ref().clone();
            let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
            let db_path = dir.path().join("client.sqlite3");
            let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
            store.save_device_state(&device).unwrap();

            let room_id = "room-legacy";
            let sealed =
                encrypt_app_room_metadata_json(&options.encryption_key, &owner, room_id, &metadata);
            store
                .conn
                .execute(
                    r#"
                    INSERT INTO client_app_rooms (
                      account_id,
                      device_id,
                      room_id,
                      nonce,
                      ciphertext
                    ) VALUES (?1, ?2, ?3, ?4, ?5)
                    "#,
                    params![
                        &owner.account_id,
                        &owner.device_id,
                        room_id,
                        &sealed.nonce,
                        &sealed.ciphertext,
                    ],
                )
                .unwrap();

            let error = store.load_app_rooms(&owner).unwrap_err();
            assert!(
                matches!(error, ClientStoreError::DecodeAppRoomMetadata),
                "{case} should fail closed, got {error}"
            );
        }
    }

    #[test]
    fn sqlite_client_store_persists_app_rooms_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([8; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "phone";
        let config = FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        };
        let device = FiniteChatDevice::new(config).unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let db_path = dir.path().join("client.sqlite3");

        let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
        store.save_device_state(&device).unwrap();
        let first = app_room("room-main", "Main Room");
        let second = StoredAppRoom {
            state: StoredAppRoomState::WaitingForApproval,
            status: "waiting for room admission".to_owned(),
            local_read_seq: 42,
            ..app_room("room-side", "Side Room")
        };
        store
            .save_app_rooms(&owner, &[second.clone(), first.clone()])
            .unwrap();
        assert_eq!(
            store.load_app_rooms(&owner).unwrap(),
            vec![first.clone(), second.clone()]
        );
        drop(store);

        let mut reopened = SqliteClientStore::open(&db_path, options).unwrap();
        assert_eq!(
            reopened.load_app_rooms(&owner).unwrap(),
            vec![first.clone(), second.clone()]
        );

        let renamed = StoredAppRoom {
            display_name: "Renamed Room".to_owned(),
            ..first
        };
        reopened
            .save_app_rooms(&owner, std::slice::from_ref(&renamed))
            .unwrap();
        assert_eq!(
            reopened.load_app_rooms(&owner).unwrap(),
            vec![renamed, second]
        );
    }

    #[test]
    fn sqlite_client_store_persists_app_state_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([9; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "phone";
        let config = FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        };
        let device = FiniteChatDevice::new(config).unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let db_path = dir.path().join("client.sqlite3");

        let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
        store.save_device_state(&device).unwrap();
        assert_eq!(
            store.load_app_state(&owner).unwrap(),
            StoredAppState::default()
        );

        let selected = StoredAppState {
            selected_room_id: Some("room-main".to_owned()),
            selected_topic_id: Some("home".to_owned()),
            selected_chat_id: Some("segment-main".to_owned()),
            paired_agent: Some(StoredPairedAgent {
                agent_account_id: "agent-account".to_owned(),
                canonical_room_id: "room-agent".to_owned(),
            }),
            revoked_devices: [DeviceRef {
                account_id: owner.account_id.clone(),
                device_id: "tablet".to_owned(),
            }]
            .into_iter()
            .collect(),
            chat_archives: vec![StoredChatArchiveState {
                room_id: "room-main".to_owned(),
                topic_id: "home".to_owned(),
                chat_id: "segment-archived".to_owned(),
                accepted_seq: 42,
                archived: true,
            }],
        };
        store.save_app_state(&owner, &selected).unwrap();
        assert_eq!(store.load_app_state(&owner).unwrap(), selected);
        drop(store);

        let mut reopened = SqliteClientStore::open(&db_path, options).unwrap();
        assert_eq!(reopened.load_app_state(&owner).unwrap(), selected);

        let cleared = StoredAppState {
            selected_room_id: None,
            selected_topic_id: None,
            selected_chat_id: None,
            paired_agent: None,
            revoked_devices: BTreeSet::new(),
            chat_archives: Vec::new(),
        };
        reopened.save_app_state(&owner, &cleared).unwrap();
        assert_eq!(reopened.load_app_state(&owner).unwrap(), cleared);
    }

    #[test]
    fn sqlite_client_store_rejects_legacy_app_state_metadata_shapes() {
        for case in ["missing selected room", "missing revoked devices"] {
            let dir = tempfile::tempdir().unwrap();
            let secret = NostrSecretKey::from_bytes([16; NOSTR_SECRET_KEY_BYTES]).unwrap();
            let device_id = "phone";
            let config = FiniteChatDeviceConfig {
                account_secret_key: secret.clone(),
                device_id: device_id.to_owned(),
                now_unix_seconds: NOW,
                credential_not_before_unix_seconds: NOW.saturating_sub(60),
                credential_not_after_unix_seconds: NOW.saturating_add(60),
            };
            let device = FiniteChatDevice::new(config).unwrap();
            let owner = device.device_ref().clone();
            let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
            let db_path = dir.path().join("client.sqlite3");
            let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
            store.save_device_state(&device).unwrap();

            let mut metadata = serde_json::to_value(StoredAppStateMetadataV1 {
                selected_room_id: Some("room-main".to_owned()),
                selected_topic_id: Some("home".to_owned()),
                selected_chat_id: Some("segment-main".to_owned()),
                paired_agent: Some(StoredPairedAgent {
                    agent_account_id: "agent-account".to_owned(),
                    canonical_room_id: "room-agent".to_owned(),
                }),
                revoked_devices: [DeviceRef {
                    account_id: owner.account_id.clone(),
                    device_id: "tablet".to_owned(),
                }]
                .into_iter()
                .collect(),
                chat_archives: Vec::new(),
            })
            .unwrap();
            let metadata_object = metadata.as_object_mut().unwrap();
            match case {
                "missing selected room" => {
                    metadata_object.remove("selected_room_id");
                }
                "missing revoked devices" => {
                    metadata_object.remove("revoked_devices");
                }
                other => panic!("unknown app-state metadata case {other}"),
            }

            let sealed =
                encrypt_app_state_metadata_json(&options.encryption_key, &owner, &metadata);
            store
                .conn
                .execute(
                    r#"
                    INSERT INTO client_app_state (
                      account_id,
                      device_id,
                      nonce,
                      ciphertext
                    ) VALUES (?1, ?2, ?3, ?4)
                    "#,
                    params![
                        &owner.account_id,
                        &owner.device_id,
                        &sealed.nonce,
                        &sealed.ciphertext,
                    ],
                )
                .unwrap();

            let error = store.load_app_state(&owner).unwrap_err();
            assert!(
                matches!(error, ClientStoreError::DecodeAppStateMetadata),
                "{case} should fail closed, got {error}"
            );
        }
    }

    #[test]
    fn sqlite_client_store_loads_metadata_written_before_paired_agent_field() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([17; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "hosted-web";
        let config = FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        };
        let device = FiniteChatDevice::new(config).unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let db_path = dir.path().join("client.sqlite3");
        let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
        store.save_device_state(&device).unwrap();

        // Keep this as literal predecessor output instead of serializing the
        // current StoredAppStateMetadataV1 and deleting a field. The latter
        // makes the fixture change with the candidate writer and recreates the
        // all-new writer/all-new reader blind spot that broke production.
        let metadata = serde_json::json!({
            "selected_room_id": "room-main",
            "selected_topic_id": "home",
            "selected_chat_id": "segment-main",
            "revoked_devices": [],
            "chat_archives": []
        });

        let sealed = encrypt_app_state_metadata_json(&options.encryption_key, &owner, &metadata);
        store
            .conn
            .execute(
                r#"
                INSERT INTO client_app_state (
                  account_id,
                  device_id,
                  nonce,
                  ciphertext
                ) VALUES (?1, ?2, ?3, ?4)
                "#,
                params![
                    &owner.account_id,
                    &owner.device_id,
                    &sealed.nonce,
                    &sealed.ciphertext,
                ],
            )
            .unwrap();

        assert_eq!(
            store.load_app_state(&owner).unwrap(),
            StoredAppState {
                selected_room_id: Some("room-main".to_owned()),
                selected_topic_id: Some("home".to_owned()),
                selected_chat_id: Some("segment-main".to_owned()),
                paired_agent: None,
                revoked_devices: BTreeSet::new(),
                chat_archives: Vec::new(),
            }
        );
    }

    #[test]
    fn sqlite_client_store_persists_app_profiles_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([10; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "phone";
        let config = FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        };
        let device = FiniteChatDevice::new(config).unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let db_path = dir.path().join("client.sqlite3");

        let alice = app_profile("alice", "Alice Finite");
        let bob = StoredAppProfile {
            stale: true,
            ..app_profile("bob", "Bob Cached")
        };

        let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
        store.save_device_state(&device).unwrap();
        store
            .save_app_profiles(&owner, &[bob.clone(), alice.clone()])
            .unwrap();
        assert_eq!(
            store.load_app_profiles(&owner).unwrap(),
            vec![alice.clone(), bob.clone()]
        );
        drop(store);

        let mut reopened = SqliteClientStore::open(&db_path, options).unwrap();
        assert_eq!(
            reopened.load_app_profiles(&owner).unwrap(),
            vec![alice.clone(), bob.clone()]
        );

        let updated = StoredAppProfile {
            profile: NostrProfileRecord {
                display_name: Some("Alice Updated".to_owned()),
                about: Some("updated profile".to_owned()),
                ..alice.profile.clone()
            },
            stale: false,
        };
        reopened
            .save_app_profiles(&owner, std::slice::from_ref(&updated))
            .unwrap();
        assert_eq!(
            reopened.load_app_profiles(&owner).unwrap(),
            vec![updated, bob]
        );
    }

    #[test]
    fn sqlite_client_store_rejects_legacy_app_profile_metadata_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([17; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "phone";
        let config = FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: NOW,
            credential_not_before_unix_seconds: NOW.saturating_sub(60),
            credential_not_after_unix_seconds: NOW.saturating_add(60),
        };
        let device = FiniteChatDevice::new(config).unwrap();
        let owner = device.device_ref().clone();
        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let db_path = dir.path().join("client.sqlite3");
        let mut store = SqliteClientStore::open(&db_path, options.clone()).unwrap();
        store.save_device_state(&device).unwrap();

        let profile = app_profile("alice", "Alice Finite");
        let mut metadata = serde_json::to_value(StoredAppProfileMetadataV1 {
            profile: profile.profile.clone(),
            stale: profile.stale,
        })
        .unwrap();
        metadata.as_object_mut().unwrap().remove("stale");
        let sealed = encrypt_app_profile_metadata_json(
            &options.encryption_key,
            &owner,
            &profile.profile.account_id,
            &metadata,
        );
        store
            .conn
            .execute(
                r#"
                INSERT INTO client_app_profiles (
                  account_id,
                  device_id,
                  profile_account_id,
                  nonce,
                  ciphertext
                ) VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![
                    &owner.account_id,
                    &owner.device_id,
                    &profile.profile.account_id,
                    &sealed.nonce,
                    &sealed.ciphertext,
                ],
            )
            .unwrap();

        let error = store.load_app_profiles(&owner).unwrap_err();
        assert!(
            matches!(error, ClientStoreError::DecodeAppProfileMetadata),
            "missing stale should fail closed, got {error}"
        );
    }

    #[test]
    fn sqlite_client_store_rejects_plaintext_app_messages() {
        let dir = tempfile::tempdir().unwrap();
        let secret = NostrSecretKey::from_bytes([9; NOSTR_SECRET_KEY_BYTES]).unwrap();
        let device_id = "phone";
        let owner = DeviceRef {
            account_id: hex_lower(secret.public_key().as_bytes()),
            device_id: device_id.to_owned(),
        };
        let db_path = dir.path().join("client.sqlite3");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                r#"
                PRAGMA foreign_keys = ON;
                CREATE TABLE client_device_states (
                  account_id TEXT NOT NULL,
                  device_id TEXT NOT NULL,
                  nonce BLOB NOT NULL,
                  ciphertext BLOB NOT NULL,
                  PRIMARY KEY (account_id, device_id)
                );
                CREATE TABLE client_app_messages (
                  account_id TEXT NOT NULL,
                  device_id TEXT NOT NULL,
                  room_id TEXT NOT NULL,
                  seq INTEGER NOT NULL,
                  message_id TEXT NOT NULL,
                  sender_account_id TEXT NOT NULL,
                  sender_device_id TEXT NOT NULL,
                  plaintext BLOB NOT NULL,
                  PRIMARY KEY (account_id, device_id, room_id, message_id),
                  FOREIGN KEY (account_id, device_id)
                    REFERENCES client_device_states(account_id, device_id)
                    ON DELETE CASCADE
                );
                "#,
            )
            .unwrap();
            conn.execute(
                r#"
                INSERT INTO client_device_states (account_id, device_id, nonce, ciphertext)
                VALUES (?1, ?2, X'00', X'00')
                "#,
                params![&owner.account_id, &owner.device_id],
            )
            .unwrap();
            conn.execute(
                r#"
                INSERT INTO client_app_messages (
                  account_id,
                  device_id,
                  room_id,
                  seq,
                  message_id,
                  sender_account_id,
                  sender_device_id,
                  plaintext
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                params![
                    &owner.account_id,
                    &owner.device_id,
                    "room-store",
                    7i64,
                    "msg-legacy",
                    &owner.account_id,
                    &owner.device_id,
                    b"legacy-message".as_slice(),
                ],
            )
            .unwrap();
        }

        let options = SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).unwrap();
        let error = match SqliteClientStore::open(&db_path, options) {
            Ok(_) => panic!("plaintext app projection schema should be rejected"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error,
                ClientStoreError::LegacyAppProjectionSchema { ref table, ref reason }
                    if table == "client_app_messages"
                        && reason == "plaintext column is unsupported"
            ),
            "unexpected error: {error}"
        );
    }

    fn test_device_link_bootstrap_chunks(
        owner: &DeviceRef,
        source: &DeviceRef,
    ) -> Vec<DeviceLinkBootstrapV2> {
        let history = [1_u64, 2]
            .into_iter()
            .map(|seq| DeviceLinkBootstrapEventV2 {
                seq,
                message_id: format!("history-{seq}"),
                sender: source.clone(),
                plaintext: serde_json::to_vec(&DecryptedApplicationEventV1 {
                    kind: DurableAppEventKind::ChatMessage,
                    conversation_id: Some("topic".to_owned()),
                    segment_id: Some("chat".to_owned()),
                    payload: format!("history {seq}").into_bytes(),
                })
                .unwrap(),
                timestamp_unix_seconds: NOW + seq,
            })
            .collect::<Vec<_>>();
        let chunk_history = vec![vec![history[0].clone()], vec![history[1].clone()]];
        let history_sha256 = device_link_bootstrap_history_sha256(&chunk_history);
        chunk_history
            .into_iter()
            .enumerate()
            .map(|(chunk_index, history)| DeviceLinkBootstrapV2 {
                version: finitechat_proto::DEVICE_LINK_BOOTSTRAP_VERSION_V2,
                bootstrap_id: "bootstrap-test".to_owned(),
                target: owner.clone(),
                chunk_index: chunk_index as u32,
                chunk_count: 2,
                total_history_events: 2,
                history_sha256: history_sha256.clone(),
                room: DeviceLinkBootstrapRoomV2 {
                    room_id: "room-bootstrap".to_owned(),
                    display_name: "Bootstrap Room".to_owned(),
                    picture: None,
                },
                canonical_selection: None,
                profiles: Vec::new(),
                history,
            })
            .collect()
    }

    fn test_device_link_bootstrap_control_event(
        source: &DeviceRef,
        seq: u64,
        bootstrap: &DeviceLinkBootstrapV2,
    ) -> StoredAppEvent {
        StoredAppEvent {
            room_id: bootstrap.room.room_id.clone(),
            seq,
            message_id: format!("control-{}", bootstrap.chunk_index),
            sender: source.clone(),
            plaintext: serde_json::to_vec(&DecryptedApplicationEventV1 {
                kind: DurableAppEventKind::Namespaced {
                    name: FINITECHAT_DEVICE_LINK_BOOTSTRAP_EVENT_V2.to_owned(),
                    policy: ApplicationDeliveryPolicy::NON_NOTIFYING,
                },
                conversation_id: None,
                segment_id: None,
                payload: serde_json::to_vec(bootstrap).unwrap(),
            })
            .unwrap(),
            timestamp_unix_seconds: NOW + seq,
        }
    }

    fn app_message(
        sender: &DeviceRef,
        seq: u64,
        message_id: &str,
        plaintext: &str,
    ) -> StoredAppMessage {
        StoredAppMessage {
            room_id: "room-store".to_owned(),
            seq,
            message_id: message_id.to_owned(),
            sender: sender.clone(),
            plaintext: plaintext.as_bytes().to_vec(),
            timestamp_unix_seconds: NOW + seq,
        }
    }

    fn app_event(
        sender: &DeviceRef,
        seq: u64,
        message_id: &str,
        plaintext: &str,
    ) -> StoredAppEvent {
        StoredAppEvent {
            room_id: "room-store".to_owned(),
            seq,
            message_id: message_id.to_owned(),
            sender: sender.clone(),
            plaintext: plaintext.as_bytes().to_vec(),
            timestamp_unix_seconds: NOW + seq,
        }
    }

    fn outbound_message(sender: &DeviceRef, plaintext: &str) -> StoredOutboundMessage {
        let envelope = envelope(
            "room-store".to_owned(),
            "mls-store".to_owned(),
            sender.clone(),
            1,
            LogEntryKind::Application,
            plaintext.as_bytes().to_vec(),
        );
        let message_id = envelope.message_id().unwrap();
        let append_request = AppendEventRequest {
            room_id: "room-store".to_owned(),
            sender: sender.clone(),
            envelope,
            idempotency_key: format!("idem-{message_id}"),
            timestamp_unix_seconds: NOW,
        };
        StoredOutboundMessage {
            room_id: "room-store".to_owned(),
            message_id,
            sender: sender.clone(),
            plaintext: plaintext.as_bytes().to_vec(),
            local_state: StoredOutboundLocalState::Sent,
            server_delivery_state: StoredOutboundServerDeliveryState::Undelivered,
            append_request,
            timestamp_unix_seconds: NOW,
        }
    }

    fn encrypt_app_outbox_metadata_json(
        encryption_key: &ClientStoreEncryptionKey,
        owner: &DeviceRef,
        room_id: &str,
        message_id: &str,
        metadata: &serde_json::Value,
    ) -> SealedAppOutbox {
        let plaintext = serde_json::to_vec(metadata).unwrap();
        let aad = app_outbox_aad(AppOutboxIdentity {
            owner,
            room_id,
            message_id,
        })
        .unwrap();
        let provider = OpenMlsRustCrypto::default();
        let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider.rand().random_array().unwrap();
        let ciphertext = provider
            .crypto()
            .aead_encrypt(
                AeadType::Aes256Gcm,
                encryption_key.as_bytes(),
                &plaintext,
                &nonce,
                &aad,
            )
            .unwrap();
        SealedAppOutbox {
            nonce: nonce.to_vec(),
            ciphertext,
        }
    }

    fn app_room(room_id: &str, display_name: &str) -> StoredAppRoom {
        StoredAppRoom {
            room_id: room_id.to_owned(),
            display_name: display_name.to_owned(),
            picture: None,
            state: StoredAppRoomState::Connected,
            status: "connected".to_owned(),
            local_read_seq: 0,
        }
    }

    fn encrypt_app_room_metadata_json(
        encryption_key: &ClientStoreEncryptionKey,
        owner: &DeviceRef,
        room_id: &str,
        metadata: &serde_json::Value,
    ) -> SealedAppRoom {
        let plaintext = serde_json::to_vec(metadata).unwrap();
        let aad = app_room_aad(AppRoomIdentity { owner, room_id }).unwrap();
        let provider = OpenMlsRustCrypto::default();
        let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider.rand().random_array().unwrap();
        let ciphertext = provider
            .crypto()
            .aead_encrypt(
                AeadType::Aes256Gcm,
                encryption_key.as_bytes(),
                &plaintext,
                &nonce,
                &aad,
            )
            .unwrap();
        SealedAppRoom {
            nonce: nonce.to_vec(),
            ciphertext,
        }
    }

    fn encrypt_app_state_metadata_json(
        encryption_key: &ClientStoreEncryptionKey,
        owner: &DeviceRef,
        metadata: &serde_json::Value,
    ) -> SealedAppState {
        let plaintext = serde_json::to_vec(metadata).unwrap();
        let aad = app_state_aad(AppStateIdentity { owner }).unwrap();
        let provider = OpenMlsRustCrypto::default();
        let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider.rand().random_array().unwrap();
        let ciphertext = provider
            .crypto()
            .aead_encrypt(
                AeadType::Aes256Gcm,
                encryption_key.as_bytes(),
                &plaintext,
                &nonce,
                &aad,
            )
            .unwrap();
        SealedAppState {
            nonce: nonce.to_vec(),
            ciphertext,
        }
    }

    fn app_profile(seed: &str, display_name: &str) -> StoredAppProfile {
        let account_id = match seed {
            "alice" => "000000000000000000000000000000000000000000000000000000000000000a",
            "bob" => "000000000000000000000000000000000000000000000000000000000000000b",
            other => panic!("unknown profile seed {other}"),
        };
        StoredAppProfile {
            profile: NostrProfileRecord {
                account_id: account_id.to_owned(),
                name: Some(display_name.to_ascii_lowercase().replace(' ', "_")),
                display_name: Some(display_name.to_owned()),
                about: None,
                picture: Some(format!("https://example.invalid/{seed}.png")),
                bot: None,
                finite_role: None,
                metadata_json: None,
                fetched_at_ms: NOW.saturating_mul(1000),
                expires_at_ms: NOW.saturating_mul(1000).saturating_add(60_000),
            },
            stale: false,
        }
    }

    fn encrypt_app_profile_metadata_json(
        encryption_key: &ClientStoreEncryptionKey,
        owner: &DeviceRef,
        profile_account_id: &str,
        metadata: &serde_json::Value,
    ) -> SealedAppProfile {
        let plaintext = serde_json::to_vec(metadata).unwrap();
        let aad = app_profile_aad(AppProfileIdentity {
            owner,
            profile_account_id,
        })
        .unwrap();
        let provider = OpenMlsRustCrypto::default();
        let nonce: [u8; CLIENT_STORE_NONCE_BYTES] = provider.rand().random_array().unwrap();
        let ciphertext = provider
            .crypto()
            .aead_encrypt(
                AeadType::Aes256Gcm,
                encryption_key.as_bytes(),
                &plaintext,
                &nonce,
                &aad,
            )
            .unwrap();
        SealedAppProfile {
            nonce: nonce.to_vec(),
            ciphertext,
        }
    }
}
