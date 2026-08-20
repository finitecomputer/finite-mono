use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use finitechat_blob::{
    BlobDescriptor, BlossomDownloadHttpResponse, BlossomUploadHttpResponse,
    finish_blossom_download_http_response, finish_blossom_upload_http_response,
    prepare_attachment_upload, prepare_blossom_download_http_request,
    prepare_blossom_upload_http_request,
};
use finitechat_client::{
    AppliedLogEntry, ClientError, ClientStoreError, DeviceLinkBootstrapCommitOutcome,
    DeviceLinkBootstrapStageOutcome, FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery,
    HttpRuntimeDeliveryError, LinkBootstrapChunkPlan, LinkBootstrapExportState,
    LinkBootstrapManifestReceipt, LinkBootstrapRoomExport, LinkFanoutRoomStatus, PreparedCommit,
    ReqwestHttpRuntimeTransport, ReqwestHttpRuntimeTransportError, RuntimeDelivery,
    RuntimeLinkFanoutOptions, RuntimeSyncOptions, RuntimeWorkerError, SqliteClientStore,
    SqliteClientStoreOptions, StoredAppEvent, StoredAppMessage, StoredAppProfile, StoredAppRoom,
    StoredAppRoomState, StoredAppState, StoredChatArchiveState, StoredDeviceLinkBootstrapReceipt,
    StoredOutboundLocalState, StoredOutboundMessage, StoredOutboundServerDeliveryState,
    StoredPairedAgent, StoredPendingDeviceLinkBootstrap, device_link_bootstrap_chunk_sha256,
    generate_account_secret, run_link_fanout_tick, run_room_server_sync_setup_tick,
    run_room_sync_tick, run_runtime_sync_setup_tick,
};
use finitechat_hermes::{
    HermesAttachmentKindV1, HermesAttachmentV1, HermesMessagePayloadV1, HermesMessageStatusV1,
    HermesSendKindV1,
};
use finitechat_http::{
    FINITECHAT_SERVER_CONTRACT_VERSION, GetEphemeralActivitiesRequest, HealthResponse,
    PushPlatform, SyncHintEvent, SyncStreamRequest, SyncWaitInbox, SyncWaitRoom,
};
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::{
    AppendEphemeralActivityRequest, ApplicationDeliveryPolicy, AttachmentBlobMetadataV1,
    AttachmentBlobReferenceV1, ChatArchiveV1, ChatReactionV1, ChatReceiptStateV1, ChatReceiptV1,
    ChatRenameV1, ClaimKeyPackageResult, ConversationMetadataV1, ConversationProjection,
    ConversationProjectionEntry, ConversationProjectionEventContext, ConversationSegmentStartV1,
    CreateRoomRequest, DEVICE_LINK_BOOTSTRAP_VERSION_V2, DecryptedApplicationEventV1,
    DecryptedEphemeralActivityV1, DeviceLinkBootstrapEventV2, DeviceLinkBootstrapProfileV2,
    DeviceLinkBootstrapRoomV2, DeviceLinkBootstrapSelectionV2, DeviceLinkBootstrapV2, DeviceRef,
    DurableAppEventKind, EphemeralActivityAccepted, EphemeralActivityActionV1,
    EphemeralActivityIngressContext, EphemeralActivityProjection, EphemeralActivityProjectionEntry,
    EventAccepted, FINITECHAT_ACTIVITY_KIND_THINKING, FINITECHAT_ACTIVITY_KIND_TYPING,
    FINITECHAT_ACTIVITY_KIND_WORKING, FINITECHAT_CHAT_ARCHIVE_EVENT_V1,
    FINITECHAT_CHAT_RENAME_EVENT_V1, FINITECHAT_DEVICE_LINK_BOOTSTRAP_EVENT_V2,
    GenericActivityKindV1, ListAccountRoomsRequest, LogEntryKind, MAX_CHAT_TITLE_BYTES,
    MAX_DEVICE_LINK_BOOTSTRAP_CHUNKS, MAX_DEVICE_LINK_BOOTSTRAP_EVENTS,
    MAX_DEVICE_LINK_BOOTSTRAP_PAYLOAD_BYTES, MAX_DEVICE_LINK_BOOTSTRAP_TOTAL_EVENTS,
    MAX_KEY_PACKAGES_PER_DEVICE, MAX_OBJECT_ID_BYTES, MAX_STAGED_WELCOMES_PER_COMMIT, RoomProtocol,
    RuntimeActivityClearV1, RuntimeCommandRequestV1, RuntimeCommandResultV1,
    RuntimeStateSnapshotV1, SubmitCommitRequest, delivery_member_id_for_device, nprofile_decode,
    npub_decode, npub_encode, nsec_decode, nsec_encode, validate_item_count, validate_string_bytes,
};
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::{OffsetDateTime, UtcOffset};

pub mod native_authkit;
pub mod native_device_link;
pub mod nip_ab;

const CLIENT_STORE_FILE: &str = "client.sqlite3";
const LEGACY_DEVICE_LINK_BOOTSTRAP_REQUEST_EVENT_V2: &str =
    "finitechat.device-link.bootstrap-request.v2";
const ATTACHMENT_CACHE_DIR: &str = "attachments";
const LOCAL_ROOM_CONNECTED_TEXT: &str = "Connected";
const LOCAL_ROOM_UNAVAILABLE_STATUS: &str = "room is not available on this device";
const LOCAL_ROOM_UNAVAILABLE_TEXT: &str = "Unavailable on this device";
// The transcript shown in AppState remains paged, but the encrypted local
// projection retains every message. This is an addressability bound only.
const MAX_APP_MESSAGES: usize = u32::MAX as usize;
const MAX_APP_MESSAGES_U32: u32 = u32::MAX;
const MAX_APP_CHAT_ARCHIVES: usize = 5_000;
const MAX_APP_CHAT_TITLES: usize = 5_000;
const DEVICE_LINK_BOOTSTRAP_STORE_PAGE_SIZE: u32 = 256;
const DEFAULT_TRANSCRIPT_WINDOW: usize = 50;
const MAX_TRANSCRIPT_PAGE_SIZE: u32 = 100;
const MAX_OUTBOX_DRAIN_PER_TICK: usize = 16;
const DEFAULT_KEY_PACKAGE_TARGET_AVAILABLE: u32 = 2;
const DEFAULT_MAX_SYNC_PAGES_PER_ROOM: u32 = 16;
const DEFAULT_CREDENTIAL_VALIDITY_SECONDS: u64 = 10 * 365 * 24 * 60 * 60;
const DEFAULT_APP_UPDATE_WAIT_MILLIS: u64 = 30_000;
const DEFAULT_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS: u64 = 30_000;
const SERVER_CONTRACT_HEALTH_TIMEOUT_SECS: u64 = 5;
const DEFAULT_PROFILE_CACHE_TTL_MS: u64 = 90 * 24 * 60 * 60 * 1000;
const MAX_PROFILE_DISPLAY_NAME_BYTES: u32 = 128;
const MAX_PROFILE_ABOUT_BYTES: u32 = 4 * 1024;
const MAX_PROFILE_PICTURE_BYTES: u32 = 2 * 1024;
const MAX_PUBLIC_IMAGE_UPLOAD_BYTES: usize = 8 * 1024 * 1024;
const MIN_APP_UPDATE_WAIT_MILLIS: u64 = 1_000;
const MAX_APP_UPDATE_WAIT_MILLIS: u64 = 60_000;
const MAX_ATTACHMENTS_PER_MESSAGE: u32 = 32;
const FINITECHAT_POLL_PAYLOAD_TYPE_V1: &str = "finitechat.chat.poll.v1";
const FINITECHAT_POLL_VOTE_EVENT_V1: &str = "chat.poll.vote.v1";
pub const HOME_TOPIC_ID: &str = "home";
pub const HOME_TOPIC_TITLE: &str = "Home";
pub const HOME_CHAT_ID: &str = "home-chat";
const MIN_POLL_OPTIONS: usize = 2;
const MAX_POLL_OPTIONS: u32 = 10;
const MAX_POLL_QUESTION_BYTES: u32 = 512;
const MAX_POLL_OPTION_BYTES: u32 = 160;
const TYPING_REFRESH_MIN_MILLIS: u64 = 10_000;
const FINITE_SITES_NATIVE_SESSION_PURPOSE: &str = "finite_site_view_session";
const FINITE_SITES_NATIVE_SESSION_PATH: &str = "/_finite/auth/native-session";
const FINITE_SITES_NATIVE_SESSION_METHOD: &str = "POST";
const FINITE_SITES_NIP98_KIND: u32 = 27235;
const FINITE_SITES_NIP98_AUTH_SCHEME: &str = "Nostr ";
const MAX_FINITE_SITES_NATIVE_SESSION_BODY_BYTES: usize = 4 * 1024;
const MAX_FINITE_SITES_NATIVE_RETURN_TO_BYTES: usize = 1024;
const MIN_FINITE_SITES_NATIVE_NONCE_BYTES: usize = 16;
const MAX_FINITE_SITES_NATIVE_NONCE_BYTES: usize = 128;
const MAX_FINITE_SITES_NATIVE_CLIENT_BYTES: usize = 64;

const _: () = {
    assert!(MAX_APP_MESSAGES > 0);
    assert!(MAX_APP_MESSAGES_U32 as usize == MAX_APP_MESSAGES);
    assert!(DEFAULT_TRANSCRIPT_WINDOW > 0);
    assert!(DEFAULT_TRANSCRIPT_WINDOW <= MAX_APP_MESSAGES);
    assert!(MAX_TRANSCRIPT_PAGE_SIZE > 0);
    assert!(MIN_POLL_OPTIONS > 0);
    assert!(MIN_POLL_OPTIONS <= MAX_POLL_OPTIONS as usize);
};

uniffi::setup_scaffolding!();

#[derive(Debug, Error, uniffi::Error)]
pub enum FiniteChatCoreError {
    #[error("filesystem error: {reason}")]
    Filesystem { reason: String },
    #[error("invalid account secret")]
    InvalidAccountSecret,
    #[error("client error: {reason}")]
    Client { reason: String },
    #[error("delivery error: {reason}")]
    Delivery { reason: String },
    #[error("server rejected delivery: {reason}")]
    ServerRejected { reason: String },
    #[error("server idempotency conflict: {reason}")]
    IdempotencyConflict { reason: String },
    #[error("store error: {reason}")]
    Store { reason: String },
    #[error("profile error: {reason}")]
    Profile { reason: String },
    #[error("lock poisoned")]
    LockPoisoned,
    #[error("this runtime opened the client store read-only and cannot change state")]
    ReadOnly,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct OpenOptions {
    pub data_dir: String,
    pub server_url: String,
    pub device_id: String,
    /// Explicit account secret (lowercase hex), for platforms that hold key
    /// material themselves (e.g. the iOS keychain identity). `None` resolves
    /// the shared Finite identity at `$FINITE_HOME/identity/identity.json`
    /// (or `~/.finite/identity/identity.json`), minting it if absent.
    pub account_secret_hex: Option<String>,
    pub now_unix_seconds: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct Identity {
    pub account_id: String,
    pub device_id: String,
    pub account_secret_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct NostrIdentityMaterial {
    pub account_secret_hex: String,
    pub account_id: String,
    pub npub: String,
    pub nsec: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct FiniteSitesNativeSessionProof {
    pub body_json: String,
    pub authorization_header: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FiniteSitesNativeSessionRequest {
    purpose: String,
    return_to: String,
    client: String,
    nonce: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct NostrHttpAuthEvent {
    id: String,
    pubkey: String,
    created_at: u64,
    kind: u32,
    tags: Vec<Vec<String>>,
    content: String,
    sig: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct BootstrapRoomResult {
    pub room_id: String,
    pub mls_group_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ChatReactionSummary {
    pub emoji: String,
    pub count: u32,
    pub reacted_by_me: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ChatReadReceiptSummary {
    pub delivered_count: u32,
    pub read_count: u32,
    pub display_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ChatPollOption {
    pub option_id: String,
    pub text: String,
    pub vote_count: u32,
    pub voted_by_me: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ChatPoll {
    pub question: String,
    pub options: Vec<ChatPollOption>,
    pub total_votes: u32,
    pub my_vote_option_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum ChatMediaKind {
    Image,
    VoiceNote,
    Video,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ChatMediaAttachment {
    pub attachment_id: String,
    pub url: Option<String>,
    pub mime_type: String,
    pub filename: String,
    pub kind: ChatMediaKind,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub local_path: Option<String>,
    /// Integer progress in 0..=1000. Kept integral so the FFI-visible
    /// projection stays Eq/Hash-friendly and deterministic across platforms.
    pub upload_progress_per_mille: Option<u32>,
    /// Integer progress in 0..=1000 for Rust-owned attachment download state.
    /// `Some(0)` means the verified download is in flight but no byte-level
    /// progress is available from the current blocking HTTP boundary yet.
    pub download_progress_per_mille: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ChatMediaGalleryState {
    pub room_id: String,
    pub items: Vec<ChatMediaGalleryItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ChatMediaGalleryItem {
    pub item_id: String,
    pub room_id: String,
    pub message_id: String,
    pub attachment_id: String,
    pub attachment: ChatMediaAttachment,
    pub sender_display_name: String,
    pub sender_npub: Option<String>,
    pub timestamp_unix_seconds: u64,
    pub display_timestamp: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct OutboundAttachment {
    pub filename: String,
    pub mime_type: String,
    pub kind: ChatMediaKind,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum OutboundLocalSendState {
    Sending,
    #[default]
    Sent,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum OutboundServerDeliveryState {
    #[default]
    Undelivered,
    Delivered,
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct OutboundDelivery {
    pub local_send: OutboundLocalSendState,
    pub server_delivery: OutboundServerDeliveryState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppOutboxDebugRow {
    pub room_id: String,
    pub message_id: String,
    pub sender_account_id: String,
    pub sender_device_id: String,
    pub local_state: String,
    pub server_delivery_state: String,
    pub append_request_room_id: String,
    pub append_request_message_id: String,
    pub append_request_sender_account_id: String,
    pub append_request_sender_device_id: String,
    pub idempotency_key_present: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum ChatMessageKind {
    #[default]
    Message,
    Status,
    Tool,
    Media,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum ChatMessageStatus {
    Running,
    #[default]
    Complete,
}

/// One outbound plain-text chat delivery: the body plus its optional reply
/// target and caller-authored application metadata.
struct OutboundChatText {
    text: String,
    reply_to_message_id: Option<String>,
    metadata_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ChatMessage {
    pub room_id: String,
    pub seq: u64,
    pub message_id: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub chat_id: Option<String>,
    pub sender_account_id: String,
    pub sender_device_id: String,
    #[serde(default)]
    pub sender_display_name: String,
    #[serde(default)]
    pub sender_npub: Option<String>,
    pub text: String,
    #[serde(default)]
    pub display_content: String,
    #[serde(default)]
    pub rich_text_json: String,
    /// Caller-authored application metadata carried by the message payload,
    /// serialized as a JSON object string. Derived at projection time from
    /// the payload's metadata map (minus requester credentials); renderers
    /// read the keys they know (`metadata.notify`, `metadata.approve`) and
    /// ignore the rest. Empty when absent.
    #[serde(default)]
    pub metadata_json: String,
    #[serde(default)]
    pub kind: ChatMessageKind,
    #[serde(default)]
    pub status: ChatMessageStatus,
    /// Hermes marks final or otherwise notify-worthy assistant deliveries
    /// with `metadata.notify=true`. Commentary and tool progress do not carry
    /// that marker.
    #[serde(default)]
    pub final_delivery: bool,
    #[serde(default)]
    pub edit_of_message_id: Option<String>,
    pub payload: Vec<u8>,
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
    #[serde(default)]
    pub is_mine: bool,
    #[serde(default)]
    pub outbound_delivery: Option<OutboundDelivery>,
    #[serde(default)]
    pub reactions: Vec<ChatReactionSummary>,
    #[serde(default)]
    pub media: Vec<ChatMediaAttachment>,
    #[serde(default)]
    pub read_receipt: Option<ChatReadReceiptSummary>,
    #[serde(default)]
    pub poll: Option<ChatPoll>,
    #[serde(default)]
    pub timestamp_unix_seconds: u64,
    #[serde(default)]
    pub display_timestamp: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct SyncResult {
    pub uploaded_key_packages: u32,
    pub claimed_welcomes: u32,
    pub activated_welcome_acks_sent: u32,
    pub sync_pages: u32,
    pub messages: Vec<ChatMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum AppRoomState {
    Connected,
    WaitingForApproval,
    Joining,
    UnavailableOnDevice,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppRoomSummary {
    pub room_id: String,
    pub display_name: String,
    pub picture: Option<String>,
    pub state: AppRoomState,
    pub status: String,
    pub user_status_text: String,
    pub last_message_preview: String,
    pub unread_count: u32,
    pub can_load_older: bool,
    pub is_agent_chat: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppChatSummary {
    pub chat_id: String,
    pub title: String,
    pub last_message_preview: String,
    pub unread_count: u32,
    pub message_count: u32,
    pub started_seq: u64,
    pub updated_seq: u64,
    pub active: bool,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppTopicSummary {
    pub room_id: String,
    pub topic_id: String,
    pub title: String,
    pub description: Option<String>,
    pub last_message_preview: String,
    pub unread_count: u32,
    pub message_count: u32,
    pub created_seq: u64,
    pub updated_seq: u64,
    pub archived: bool,
    pub active_chat_id: Option<String>,
    pub chats: Vec<AppChatSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppPairedAgent {
    pub agent_account_id: String,
    pub canonical_room_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppRoomDetailsState {
    pub room_id: String,
    pub display_name: String,
    pub picture: Option<String>,
    pub state: AppRoomState,
    pub status: String,
    pub user_status_text: String,
    pub media_item_count: u32,
    pub members: Vec<AppRoomMemberSummary>,
    pub devices: Vec<AppDeviceSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppRoomMemberSummary {
    pub account_id: String,
    pub device_id: String,
    pub npub: String,
    pub display_name: String,
    pub picture: Option<String>,
    pub current_device: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppProfileSummary {
    pub account_id: String,
    pub npub: String,
    pub display_name: String,
    pub about: Option<String>,
    pub picture: Option<String>,
    pub stale: bool,
    pub is_agent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppDeviceSummary {
    pub account_id: String,
    pub device_id: String,
    pub active: bool,
    pub current_device: bool,
    pub revoked: bool,
    pub room_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppTypingMember {
    pub room_id: String,
    #[serde(default)]
    pub topic_id: Option<String>,
    #[serde(default)]
    pub chat_id: Option<String>,
    pub account_id: String,
    pub device_id: String,
    pub display_name: String,
    pub picture: Option<String>,
    pub npub: Option<String>,
    pub activity_kind: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum AppScanTargetOutcome {
    #[default]
    None,
    Profile,
    Unavailable,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppFlowState {
    pub notice_text: Option<String>,
    pub notice_busy: bool,
    pub scan_in_flight: bool,
    pub scan_result: AppScanTargetOutcome,
    pub image_upload_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSentMessage {
    pub message_id: String,
    pub seq: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppAcceptedActivity {
    pub route_key: String,
    pub cached_events_for_route: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppBridgeActivityInput {
    pub room_id: String,
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub segment_id: Option<String>,
    pub activity_kind: String,
    pub activity_id: Option<String>,
    pub action: EphemeralActivityActionV1,
    pub payload: Vec<u8>,
    pub expires_in_millis: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppBridgeAppliedEvent {
    pub room_id: String,
    pub seq: u64,
    pub message_id: String,
    pub sender_account_id: String,
    pub sender_device_id: String,
    pub plaintext: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppBridgeSync {
    pub joined_account_ids: Vec<String>,
    pub events: Vec<AppBridgeAppliedEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLinkFanoutReport {
    pub fanout_id: String,
    pub target_account_id: String,
    pub target_device_id: String,
    pub fanout_complete: bool,
    pub room_count: u32,
    pub active_room_count: u32,
    #[serde(default)]
    pub bootstrap_manifests: Vec<LinkBootstrapManifestReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppDeviceLinkBootstrapReceipt {
    pub bootstrap_id: String,
    pub room_id: String,
    pub manifest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AppState {
    pub rev: u64,
    pub identity: Identity,
    pub rooms: Vec<AppRoomSummary>,
    pub paired_agent: Option<AppPairedAgent>,
    pub selected_room_id: Option<String>,
    pub topics: Vec<AppTopicSummary>,
    pub selected_topic_id: Option<String>,
    pub selected_chat_id: Option<String>,
    pub active_profile_id: Option<String>,
    pub status: String,
    pub toast: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub media_gallery: Option<ChatMediaGalleryState>,
    pub room_details: Option<AppRoomDetailsState>,
    pub profiles: Vec<AppProfileSummary>,
    pub devices: Vec<AppDeviceSummary>,
    pub typing_members: Vec<AppTypingMember>,
    #[serde(default)]
    pub device_link_bootstrap_receipts: Vec<AppDeviceLinkBootstrapReceipt>,
    pub flow: AppFlowState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppProfileChatBootstrapPreparedCommit {
    pub request: SubmitCommitRequest,
    pub message_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppProfileChatBootstrapPreparation {
    pub state: AppState,
    pub complete: bool,
    pub prepared_commit: Option<AppProfileChatBootstrapPreparedCommit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppProfileChatBootstrapInput {
    pub profile: AppProfileSummary,
    pub display_name: String,
    pub intended_room_id: String,
    pub room_create_request: CreateRoomRequest,
    pub claimed_key_package: ClaimKeyPackageResult,
    pub persisted_prepared_commit: Option<AppProfileChatBootstrapPreparedCommit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum AppAction {
    StartRuntime,
    StopRuntime,
    OpenRoom {
        room_id: String,
    },
    OpenTopic {
        room_id: String,
        topic_id: String,
    },
    OpenChat {
        room_id: String,
        topic_id: String,
        chat_id: String,
    },
    PairAgent {
        room_id: String,
    },
    StartHomeChat {
        text: Option<String>,
        intent_key: String,
    },
    RenameChat {
        room_id: String,
        topic_id: String,
        chat_id: String,
        title: String,
    },
    SetChatArchived {
        room_id: String,
        topic_id: String,
        chat_id: String,
        archived: bool,
    },
    CreateRoom {
        display_name: String,
    },
    CreateTopic {
        room_id: String,
        title: String,
    },
    StartTopicChat {
        room_id: String,
        topic_id: String,
        reason: Option<String>,
    },
    SaveProfile {
        display_name: String,
        about: String,
        picture: Option<String>,
    },
    UploadImage {
        bytes: Vec<u8>,
        content_type: String,
    },
    SaveRoomMetadata {
        room_id: String,
        display_name: String,
        picture: Option<String>,
    },
    StartProfileChat {
        profile: AppProfileSummary,
        display_name: String,
    },
    StartGroupChat {
        profiles: Vec<AppProfileSummary>,
        display_name: String,
    },
    AddRoomMembers {
        room_id: String,
        profiles: Vec<AppProfileSummary>,
    },
    ScanTarget {
        value: String,
    },
    SendMessage {
        room_id: String,
        text: String,
        /// JSON object of caller-authored application metadata carried onto
        /// the message payload's metadata map (e.g. `metadata.approve`).
        #[serde(default)]
        metadata_json: Option<String>,
    },
    SendTopicMessage {
        room_id: String,
        topic_id: String,
        text: String,
        #[serde(default)]
        metadata_json: Option<String>,
    },
    SendChatMessage {
        room_id: String,
        topic_id: String,
        chat_id: String,
        text: String,
        #[serde(default)]
        metadata_json: Option<String>,
    },
    SendReply {
        room_id: String,
        text: String,
        reply_to_message_id: String,
        #[serde(default)]
        metadata_json: Option<String>,
    },
    SendChatReply {
        room_id: String,
        topic_id: String,
        chat_id: String,
        text: String,
        reply_to_message_id: String,
        #[serde(default)]
        metadata_json: Option<String>,
    },
    SendAttachment {
        room_id: String,
        filename: String,
        mime_type: String,
        kind: ChatMediaKind,
        bytes: Vec<u8>,
        caption: String,
        reply_to_message_id: Option<String>,
    },
    SendAttachments {
        room_id: String,
        attachments: Vec<OutboundAttachment>,
        caption: String,
        reply_to_message_id: Option<String>,
    },
    SendChatAttachment {
        room_id: String,
        topic_id: String,
        chat_id: String,
        filename: String,
        mime_type: String,
        kind: ChatMediaKind,
        bytes: Vec<u8>,
        caption: String,
        reply_to_message_id: Option<String>,
    },
    SendChatAttachments {
        room_id: String,
        topic_id: String,
        chat_id: String,
        attachments: Vec<OutboundAttachment>,
        caption: String,
        reply_to_message_id: Option<String>,
    },
    SendPoll {
        room_id: String,
        question: String,
        options: Vec<String>,
    },
    SendChatPoll {
        room_id: String,
        topic_id: String,
        chat_id: String,
        question: String,
        options: Vec<String>,
    },
    VotePoll {
        room_id: String,
        message_id: String,
        option_id: String,
    },
    DownloadAttachment {
        room_id: String,
        message_id: String,
        attachment_id: String,
    },
    BeginDownloadAttachment {
        room_id: String,
        message_id: String,
        attachment_id: String,
    },
    LoadOlderMessages {
        room_id: String,
        before_message_id: String,
        limit: u32,
    },
    ReactToMessage {
        room_id: String,
        message_id: String,
        emoji: String,
    },
    MarkRoomRead {
        room_id: String,
    },
    RetryMessage {
        room_id: String,
        message_id: String,
    },
    SetTyping {
        room_id: String,
        is_typing: bool,
    },
    RefreshDevices,
    RevokeDevice {
        account_id: String,
        device_id: String,
    },
    SetPushToken {
        token: String,
    },
    RemovePushToken,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum AppUpdate {
    FullState(AppState),
}

#[uniffi::export(callback_interface)]
pub trait AppReconciler: Send + Sync + 'static {
    fn reconcile(&self, update: AppUpdate);
}

struct CoreState {
    data_dir: PathBuf,
    server_url: String,
    account_secret: NostrSecretKey,
    fixed_now_unix_seconds: Option<u64>,
    store: SqliteClientStore,
    device: FiniteChatDevice,
    /// A read-only core holds no writer lease and must never mutate the
    /// store; one-shot diagnostics open this way so they can run alongside a
    /// resident service that owns the store.
    read_only: bool,
}

#[derive(uniffi::Object)]
pub struct FiniteChatRuntime {
    command_tx: mpsc::Sender<AppRuntimeCommand>,
    shared_state: Arc<Mutex<AppState>>,
    reconciler: Arc<Mutex<Option<Box<dyn AppReconciler>>>>,
    listening: AtomicBool,
}

enum AppRuntimeCommand {
    Dispatch {
        action: AppAction,
        requester_context: Option<VerifiedRequesterContext>,
        response: Option<mpsc::SyncSender<Result<AppState, FiniteChatCoreError>>>,
    },
    WaitPlan {
        timeout_millis: u64,
        response: mpsc::SyncSender<Result<AppRuntimeWaitPlan, FiniteChatCoreError>>,
    },
    ApplySyncHint {
        event: SyncHintEvent,
        response: mpsc::SyncSender<Result<AppState, FiniteChatCoreError>>,
    },
    AgentBridgePoll {
        response: mpsc::SyncSender<Result<AppBridgeSync, FiniteChatCoreError>>,
    },
    AgentBridgeApplySyncHint {
        event: SyncHintEvent,
        response: mpsc::SyncSender<Result<AppBridgeSync, FiniteChatCoreError>>,
    },
    RecentBridgeEvents {
        limit: u32,
        response: mpsc::SyncSender<Result<Vec<AppBridgeAppliedEvent>, FiniteChatCoreError>>,
    },
    SendRuntimeEvent {
        room_id: String,
        conversation_id: Option<String>,
        kind: DurableAppEventKind,
        payload: Vec<u8>,
        response: mpsc::SyncSender<Result<String, FiniteChatCoreError>>,
    },
    SendEncodedChatMessage {
        room_id: String,
        app_event_plaintext: Vec<u8>,
        preview: String,
        response: mpsc::SyncSender<Result<AppSentMessage, FiniteChatCoreError>>,
    },
    UploadBridgeAttachment {
        room_id: String,
        attachment: OutboundAttachment,
        response: mpsc::SyncSender<Result<HermesAttachmentV1, FiniteChatCoreError>>,
    },
    AppendEphemeralActivity {
        input: AppBridgeActivityInput,
        response: mpsc::SyncSender<Result<AppAcceptedActivity, FiniteChatCoreError>>,
    },
    DebugOutbox {
        response: mpsc::SyncSender<Result<Vec<AppOutboxDebugRow>, FiniteChatCoreError>>,
    },
    LinkDevice {
        fanout_id: String,
        target_device_id: String,
        response: mpsc::SyncSender<Result<DeviceLinkFanoutReport, FiniteChatCoreError>>,
    },
    ProfileChatRooms {
        account_id: String,
        response: mpsc::SyncSender<Result<Vec<String>, FiniteChatCoreError>>,
    },
    ClaimProfileChatKeyPackage {
        account_id: String,
        response: mpsc::SyncSender<Result<Option<ClaimKeyPackageResult>, FiniteChatCoreError>>,
    },
    PlanProfileChatBootstrapRoom {
        intended_room_id: String,
        response: mpsc::SyncSender<Result<CreateRoomRequest, FiniteChatCoreError>>,
    },
    PrepareProfileChatBootstrap {
        input: Box<AppProfileChatBootstrapInput>,
        fail_after_room_server_acceptance: bool,
        response: mpsc::SyncSender<Result<AppProfileChatBootstrapPreparation, FiniteChatCoreError>>,
    },
    SubmitProfileChatBootstrap {
        agent_account_id: String,
        display_name: String,
        intended_room_id: String,
        prepared_commit: Box<AppProfileChatBootstrapPreparedCommit>,
        fail_after_device_save: bool,
        response: mpsc::SyncSender<Result<AppState, FiniteChatCoreError>>,
    },
    StartTopicChatIntent {
        room_id: String,
        topic_id: String,
        reason: Option<String>,
        intent_key: String,
        response: mpsc::SyncSender<Result<AppState, FiniteChatCoreError>>,
    },
    #[cfg(test)]
    TestLoadOutbox {
        response: mpsc::SyncSender<Result<Vec<StoredOutboundMessage>, FiniteChatCoreError>>,
    },
    #[cfg(test)]
    TestSaveOutbox {
        rows: Vec<StoredOutboundMessage>,
        response: mpsc::SyncSender<Result<(), FiniteChatCoreError>>,
    },
    #[cfg(test)]
    TestRevokedDevices {
        response: mpsc::SyncSender<Result<BTreeSet<String>, FiniteChatCoreError>>,
    },
    #[cfg(test)]
    TestSeedRoomState {
        room: StoredAppRoom,
        selected_room_id: Option<String>,
        response: mpsc::SyncSender<Result<(), FiniteChatCoreError>>,
    },
}

#[derive(Clone, Debug)]
struct VerifiedRequesterContext {
    email: String,
    sites_assertion: String,
}

struct AppRuntimeState {
    core: CoreState,
    app: AppState,
    chat_projection: ChatProjectionState,
    activity_projection: EphemeralActivityProjection,
    local_typing_leases: BTreeMap<String, u64>,
    loaded_message_counts: BTreeMap<String, usize>,
    local_read_seq: BTreeMap<String, u64>,
    profile_cache: BTreeMap<String, AppProfileSummary>,
    profile_records: BTreeMap<String, finitechat_http::NostrProfileRecord>,
    revoked_devices: BTreeSet<String>,
    downloading_attachments: BTreeSet<(String, String, String)>,
    bridge_seen_joined_account_ids: BTreeSet<String>,
    room_sync_failures: BTreeSet<String>,
    inbox_hint_after_seq: u64,
    profile_chat_bootstrap_preparations: BTreeMap<String, AppProfileChatBootstrapPreparedCommit>,
    /// False only while a fresh Device is using the provisional first-room
    /// selection. A device-link bootstrap may replace that provisional route
    /// once; explicit navigation and an accepted bootstrap both lock it.
    selection_is_explicit_or_bootstrapped: bool,
}

struct AcceptedDeviceLinkBootstrap {
    room_id: String,
    canonical_selection: Option<DeviceLinkBootstrapSelectionV2>,
}

struct SendAttachmentInput {
    room_id: String,
    conversation_id: Option<String>,
    chat_id: Option<String>,
    attachments: Vec<OutboundAttachment>,
    caption: String,
    reply_to_message_id: Option<String>,
}

struct PreparedOutboundMessage {
    chat_message: ChatMessage,
    stored_message: StoredOutboundMessage,
}

#[derive(Clone, Debug)]
struct AppRuntimeWaitPlan {
    server_url: String,
    request: SyncStreamRequest,
}

#[derive(Debug, Default)]
#[must_use = "a sync projection contains every canonical delta consumed while advancing room cursors"]
struct CoreSyncProjection {
    result: SyncResult,
    events: Vec<StoredAppEvent>,
    room_sync_failures: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct ChatProjectionState {
    messages: BTreeMap<(String, String), ChatMessage>,
    /// Local arrival order mirrors the encrypted SQLite projection cache's
    /// row insertion order. Room sequence numbers are canonical only within a
    /// room and must never be compared to choose a cross-room eviction.
    message_arrival_order: VecDeque<(String, String)>,
    conversations: ConversationProjection,
    chat_archives: BTreeMap<(String, String, String), ChatArchiveProjectionEntry>,
    chat_titles: BTreeMap<(String, String, String), ChatTitleProjectionEntry>,
    reaction_senders: BTreeSet<(String, String, String, String)>,
    poll_votes: BTreeMap<(String, String, String), String>,
    delivered_through: BTreeMap<(String, String), u64>,
    read_through: BTreeMap<(String, String), u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChatArchiveProjectionEntry {
    accepted_seq: u64,
    archived: bool,
    source: ChatArchiveProjectionSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChatArchiveProjectionSource {
    /// An actual `finitechat.chat.archive.v1` event consumed from the Room log.
    CanonicalEvent,
    /// A local restart or same-account bootstrap display fallback. This may
    /// initialize UI, but it never outranks or becomes proof of a Room event.
    CacheFallback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChatTitleProjectionEntry {
    accepted_seq: u64,
    title: String,
}

#[uniffi::export]
pub fn create_nostr_identity() -> Result<NostrIdentityMaterial, FiniteChatCoreError> {
    let secret = generate_account_secret().map_err(client_error)?;
    nostr_identity_from_secret(secret)
}

#[uniffi::export]
pub fn nostr_identity_from_nsec(
    nsec: String,
) -> Result<NostrIdentityMaterial, FiniteChatCoreError> {
    let trimmed = nsec.trim();
    let normalized = trimmed.strip_prefix("nostr:").unwrap_or(trimmed);
    let secret_hex = nsec_decode(normalized).map_err(|reason| FiniteChatCoreError::Client {
        reason: format!("invalid nsec: {reason}"),
    })?;
    let secret = parse_account_secret_hex(&secret_hex)?;
    nostr_identity_from_secret(secret)
}

#[uniffi::export]
pub fn nostr_identity_from_account_secret_hex(
    account_secret_hex: String,
) -> Result<NostrIdentityMaterial, FiniteChatCoreError> {
    let secret = parse_account_secret_hex(account_secret_hex.trim())?;
    nostr_identity_from_secret(secret)
}

#[uniffi::export]
pub fn npub_from_account_id(account_id: String) -> Result<String, FiniteChatCoreError> {
    npub_encode(account_id.trim()).map_err(profile_error)
}

#[uniffi::export]
pub fn account_id_from_npub(npub: String) -> Result<String, FiniteChatCoreError> {
    let trimmed = npub.trim();
    let normalized = strip_ascii_prefix(trimmed, "nostr:").unwrap_or(trimmed);
    profile_account_id_from_nip19(normalized).map_err(profile_error)
}

fn profile_account_id_from_nip19(value: &str) -> Result<String, String> {
    if starts_with_ascii_case_insensitive(value, "npub1") {
        return npub_decode(value);
    }
    if starts_with_ascii_case_insensitive(value, "nprofile1") {
        return nprofile_decode(value);
    }
    npub_decode(value)
}

fn explicit_profile_account_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if starts_with_ascii_case_insensitive(trimmed, "npub1")
        || starts_with_ascii_case_insensitive(trimmed, "nprofile1")
    {
        return profile_account_id_from_nip19(trimmed).ok();
    }
    strip_ascii_prefix(trimmed, "nostr:").and_then(|rest| {
        if starts_with_ascii_case_insensitive(rest, "npub1")
            || starts_with_ascii_case_insensitive(rest, "nprofile1")
        {
            profile_account_id_from_nip19(rest).ok()
        } else {
            None
        }
    })
}

fn embedded_profile_account_id(value: &str) -> Option<String> {
    profile_account_id_query_value(value).or_else(|| first_embedded_profile_account_id(value))
}

fn profile_account_id_query_value(value: &str) -> Option<String> {
    let (_, query_and_fragment) = value.split_once('?')?;
    let query = query_and_fragment
        .split_once('#')
        .map(|(query, _)| query)
        .unwrap_or(query_and_fragment);
    for pair in query.split('&') {
        let Some((key, raw_value)) = pair.split_once('=') else {
            continue;
        };
        if !key.eq_ignore_ascii_case("npub") && !key.eq_ignore_ascii_case("nprofile") {
            continue;
        }
        let candidate = take_profile_bech32_candidate(raw_value);
        if let Some(account_id) = explicit_profile_account_id(candidate) {
            return Some(account_id);
        }
    }
    None
}

fn first_embedded_profile_account_id(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let npub_start = lower.find("npub1");
    let nprofile_start = lower.find("nprofile1");
    let start = match (npub_start, nprofile_start) {
        (Some(left), Some(right)) => left.min(right),
        (Some(index), None) | (None, Some(index)) => index,
        (None, None) => return None,
    };
    explicit_profile_account_id(take_profile_bech32_candidate(&value[start..]))
}

fn take_profile_bech32_candidate(value: &str) -> &str {
    let separators = ['"', '\'', '<', '>', '&', '#', '?', '/', '\\'];
    value
        .trim()
        .split(|character: char| character.is_whitespace() || separators.contains(&character))
        .next()
        .unwrap_or("")
}

fn strip_ascii_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    if starts_with_ascii_case_insensitive(value, prefix) {
        Some(&value[prefix.len()..])
    } else {
        None
    }
}

fn starts_with_ascii_case_insensitive(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

#[uniffi::export]
pub fn finite_sites_native_viewer_session_proof(
    account_secret_hex: String,
    url: String,
    return_to: String,
    client: String,
    nonce: String,
    now_unix_seconds: u64,
) -> Result<FiniteSitesNativeSessionProof, FiniteChatCoreError> {
    let secret = parse_account_secret_hex(account_secret_hex.trim())?;
    validate_finite_sites_native_session_fields(&url, &return_to, &client, &nonce)?;
    let body = FiniteSitesNativeSessionRequest {
        purpose: FINITE_SITES_NATIVE_SESSION_PURPOSE.to_owned(),
        return_to,
        client,
        nonce,
    };
    let body_json = serde_json::to_string(&body).map_err(|error| FiniteChatCoreError::Client {
        reason: format!("failed to serialize finite-sites native session body: {error}"),
    })?;
    if body_json.len() > MAX_FINITE_SITES_NATIVE_SESSION_BODY_BYTES {
        return Err(FiniteChatCoreError::Client {
            reason: "finite-sites native session body is too large".to_owned(),
        });
    }
    let authorization_header = build_nip98_auth_header(
        &secret,
        &url,
        FINITE_SITES_NATIVE_SESSION_METHOD,
        Some(body_json.as_bytes()),
        now_unix_seconds,
    )?;
    Ok(FiniteSitesNativeSessionProof {
        body_json,
        authorization_header,
    })
}

#[uniffi::export]
impl FiniteChatRuntime {
    #[uniffi::constructor]
    pub fn open(options: OpenOptions) -> Result<Arc<Self>, FiniteChatCoreError> {
        Self::open_with_mode(options, false)
    }

    /// Open a diagnostics-only runtime: holds no writer lease on the client
    /// store, never mutates it, and rejects every dispatched action. Safe to
    /// run alongside a resident service that owns the store.
    #[uniffi::constructor]
    pub fn open_read_only(options: OpenOptions) -> Result<Arc<Self>, FiniteChatCoreError> {
        Self::open_with_mode(options, true)
    }
}

impl FiniteChatRuntime {
    fn open_with_mode(
        options: OpenOptions,
        read_only: bool,
    ) -> Result<Arc<Self>, FiniteChatCoreError> {
        let core = if read_only {
            CoreState::open_read_only(options)?
        } else {
            CoreState::open(options)?
        };
        let runtime_state = AppRuntimeState::new(core)?;
        let initial_state = runtime_state.app.clone();
        let shared_state = Arc::new(Mutex::new(initial_state.clone()));
        let reconciler = Arc::new(Mutex::new(None::<Box<dyn AppReconciler>>));
        let (command_tx, command_rx) = mpsc::channel();
        spawn_app_runtime_worker(
            runtime_state,
            command_rx,
            Arc::clone(&shared_state),
            Arc::clone(&reconciler),
        );
        Ok(Arc::new(Self {
            command_tx,
            shared_state,
            reconciler,
            listening: AtomicBool::new(false),
        }))
    }
}

#[uniffi::export]
impl FiniteChatRuntime {
    pub fn state(&self) -> Result<AppState, FiniteChatCoreError> {
        let state = self
            .shared_state
            .lock()
            .map_err(|_| FiniteChatCoreError::LockPoisoned)?;
        Ok(state.clone())
    }

    pub fn dispatch(&self, action: AppAction) -> Result<(), FiniteChatCoreError> {
        self.command_tx
            .send(AppRuntimeCommand::Dispatch {
                action,
                requester_context: None,
                response: None,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })
    }

    pub fn dispatch_and_wait(&self, action: AppAction) -> Result<AppState, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::Dispatch {
                action,
                requester_context: None,
                response: Some(response_tx),
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before completing command".to_owned(),
            })?
    }

    pub fn wait_for_update(&self, timeout_millis: u64) -> Result<AppState, FiniteChatCoreError> {
        let plan = self.wait_plan(timeout_millis)?;

        let event = {
            let mut delivery = delivery_for(&plan.server_url);
            let mut stream = delivery.sync_stream(&plan.request).map_err(runtime_error)?;
            stream.next_hint().map_err(runtime_error)?
        };

        self.apply_sync_hint_and_wait(event)
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn AppReconciler>) {
        if self
            .listening
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        if let Ok(mut slot) = self.reconciler.lock() {
            *slot = Some(reconciler);
            if let Ok(state) = self.shared_state.lock()
                && let Some(listener) = slot.as_ref()
            {
                listener.reconcile(AppUpdate::FullState(state.clone()));
            }
        }
    }
}

impl FiniteChatRuntime {
    /// Dispatch a Hosted Web action carrying server-verified requester
    /// context. This intentionally stays outside the UniFFI surface.
    pub fn dispatch_and_wait_with_requester_context(
        &self,
        action: AppAction,
        email: String,
        sites_assertion: String,
    ) -> Result<AppState, FiniteChatCoreError> {
        let email = normalize_bounded_non_empty_string("requester.email", &email, 320)?;
        let sites_assertion =
            normalize_bounded_non_empty_string("requester.sites_assertion", &sites_assertion, 512)?;
        if email.chars().any(char::is_control) || sites_assertion.chars().any(char::is_control) {
            return Err(FiniteChatCoreError::Client {
                reason: "requester context contains control characters".to_owned(),
            });
        }
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::Dispatch {
                action,
                requester_context: Some(VerifiedRequesterContext {
                    email,
                    sites_assertion,
                }),
                response: Some(response_tx),
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before completing command".to_owned(),
            })?
    }
}

impl FiniteChatRuntime {
    pub fn start_topic_chat_intent_and_wait(
        &self,
        room_id: String,
        topic_id: String,
        reason: Option<String>,
        intent_key: String,
    ) -> Result<AppState, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::StartTopicChatIntent {
                room_id,
                topic_id,
                reason,
                intent_key,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before starting a Chat".to_owned(),
            })?
    }

    pub fn profile_chat_room_ids(
        &self,
        account_id: String,
    ) -> Result<Vec<String>, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::ProfileChatRooms {
                account_id,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before inspecting profile rooms".to_owned(),
            })?
    }

    pub fn claim_profile_chat_key_package_and_wait(
        &self,
        account_id: String,
    ) -> Result<Option<ClaimKeyPackageResult>, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::ClaimProfileChatKeyPackage {
                account_id,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped while claiming a profile KeyPackage".to_owned(),
            })?
    }

    /// Generate the exact account-Room bootstrap request without contacting
    /// the server. The caller must durably record this request before passing
    /// it to `prepare_profile_chat_bootstrap_and_wait`.
    pub fn plan_profile_chat_bootstrap_room_and_wait(
        &self,
        intended_room_id: String,
    ) -> Result<CreateRoomRequest, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::PlanProfileChatBootstrapRoom {
                intended_room_id,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped while planning profile chat bootstrap Room"
                    .to_owned(),
            })?
    }

    /// Prepare the exact Room named by a durable bootstrap intent. This does
    /// not persist the MLS pending commit. The Room create request must already
    /// be durable; the caller must also durably record the returned commit
    /// request before calling `submit_profile_chat_bootstrap_and_wait`.
    pub fn prepare_profile_chat_bootstrap_and_wait(
        &self,
        input: AppProfileChatBootstrapInput,
        fail_after_room_server_acceptance: bool,
    ) -> Result<AppProfileChatBootstrapPreparation, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::PrepareProfileChatBootstrap {
                input: Box::new(input),
                fail_after_room_server_acceptance,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped while preparing profile chat bootstrap".to_owned(),
            })?
    }

    pub fn submit_profile_chat_bootstrap_and_wait(
        &self,
        agent_account_id: String,
        display_name: String,
        intended_room_id: String,
        prepared_commit: AppProfileChatBootstrapPreparedCommit,
        fail_after_device_save: bool,
    ) -> Result<AppState, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::SubmitProfileChatBootstrap {
                agent_account_id,
                display_name,
                intended_room_id,
                prepared_commit: Box::new(prepared_commit),
                fail_after_device_save,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped while submitting profile chat bootstrap".to_owned(),
            })?
    }

    /// Advance the crash-safe account-room fanout for one already-created
    /// Device. The target must use this runtime's account id and publish its
    /// own KeyPackages before fanout can make progress.
    pub fn link_device_and_wait(
        &self,
        fanout_id: String,
        target_device_id: String,
    ) -> Result<DeviceLinkFanoutReport, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::LinkDevice {
                fanout_id,
                target_device_id,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before advancing device linking".to_owned(),
            })?
    }

    fn wait_plan(&self, timeout_millis: u64) -> Result<AppRuntimeWaitPlan, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::WaitPlan {
                timeout_millis,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before preparing update wait".to_owned(),
            })?
    }

    fn apply_sync_hint_and_wait(
        &self,
        event: SyncHintEvent,
    ) -> Result<AppState, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::ApplySyncHint {
                event,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before applying update".to_owned(),
            })?
    }

    pub fn agent_bridge_poll_once(&self) -> Result<AppBridgeSync, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::AgentBridgePoll {
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before bridge poll".to_owned(),
            })?
    }

    pub fn agent_bridge_wait_for_update(
        &self,
        timeout_millis: u64,
    ) -> Result<AppBridgeSync, FiniteChatCoreError> {
        let plan = self.wait_plan(timeout_millis)?;

        let event = {
            let mut delivery = delivery_for(&plan.server_url);
            let mut stream = delivery.sync_stream(&plan.request).map_err(runtime_error)?;
            stream.next_hint().map_err(runtime_error)?
        };

        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::AgentBridgeApplySyncHint {
                event,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before bridge update".to_owned(),
            })?
    }

    pub fn recent_bridge_events(
        &self,
        limit: u32,
    ) -> Result<Vec<AppBridgeAppliedEvent>, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::RecentBridgeEvents {
                limit,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before reading recent events".to_owned(),
            })?
    }

    pub fn send_runtime_command_request_and_wait(
        &self,
        room_id: String,
        conversation_id: Option<String>,
        payload: Vec<u8>,
    ) -> Result<String, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::SendRuntimeEvent {
                room_id,
                conversation_id,
                kind: DurableAppEventKind::RuntimeCommandRequest,
                payload,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before sending runtime command".to_owned(),
            })?
    }

    pub fn send_runtime_command_result_and_wait(
        &self,
        room_id: String,
        conversation_id: Option<String>,
        payload: Vec<u8>,
    ) -> Result<String, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::SendRuntimeEvent {
                room_id,
                conversation_id,
                kind: DurableAppEventKind::RuntimeCommandResult,
                payload,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before sending runtime command result".to_owned(),
            })?
    }

    pub fn send_runtime_state_snapshot_and_wait(
        &self,
        room_id: String,
        conversation_id: Option<String>,
        payload: Vec<u8>,
    ) -> Result<String, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::SendRuntimeEvent {
                room_id,
                conversation_id,
                kind: DurableAppEventKind::RuntimeStateSnapshot,
                payload,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before sending runtime state".to_owned(),
            })?
    }

    pub fn send_encoded_chat_message_and_wait(
        &self,
        room_id: String,
        app_event_plaintext: Vec<u8>,
        preview: String,
    ) -> Result<AppSentMessage, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::SendEncodedChatMessage {
                room_id,
                app_event_plaintext,
                preview,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before sending encoded chat message".to_owned(),
            })?
    }

    /// Encrypt, upload, and cache one attachment for an agent bridge before
    /// the bridge appends the E2EE message that references it. Keeping this on
    /// the runtime actor preserves the room's pinned server selection and the
    /// same attachment verification path used by native clients.
    pub fn upload_bridge_attachment_and_wait(
        &self,
        room_id: String,
        attachment: OutboundAttachment,
    ) -> Result<HermesAttachmentV1, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::UploadBridgeAttachment {
                room_id,
                attachment,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before uploading bridge attachment".to_owned(),
            })?
    }

    pub fn append_ephemeral_activity_and_wait(
        &self,
        input: AppBridgeActivityInput,
    ) -> Result<AppAcceptedActivity, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::AppendEphemeralActivity {
                input,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before appending ephemeral activity".to_owned(),
            })?
    }

    pub fn app_outbox_debug_rows(&self) -> Result<Vec<AppOutboxDebugRow>, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::DebugOutbox {
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before debug snapshot".to_owned(),
            })?
    }

    #[cfg(test)]
    fn test_outbox(&self) -> Result<Vec<StoredOutboundMessage>, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::TestLoadOutbox {
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before test outbox snapshot".to_owned(),
            })?
    }

    #[cfg(test)]
    fn test_save_outbox(
        &self,
        rows: Vec<StoredOutboundMessage>,
    ) -> Result<(), FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::TestSaveOutbox {
                rows,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before test outbox write".to_owned(),
            })?
    }

    #[cfg(test)]
    fn test_revoked_devices(&self) -> Result<BTreeSet<String>, FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::TestRevokedDevices {
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before test revoked-device snapshot".to_owned(),
            })?
    }

    #[cfg(test)]
    fn test_seed_room_state(
        &self,
        room: StoredAppRoom,
        selected_room_id: Option<String>,
    ) -> Result<(), FiniteChatCoreError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::TestSeedRoomState {
                room,
                selected_room_id,
                response: response_tx,
            })
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor is stopped".to_owned(),
            })?;
        response_rx
            .recv()
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "runtime actor stopped before test room seed".to_owned(),
            })?
    }
}

fn spawn_app_runtime_worker(
    mut state: AppRuntimeState,
    command_rx: mpsc::Receiver<AppRuntimeCommand>,
    shared_state: Arc<Mutex<AppState>>,
    reconciler: Arc<Mutex<Option<Box<dyn AppReconciler>>>>,
) {
    thread::spawn(move || {
        publish_app_update(&state.app, &shared_state, &reconciler);
        while let Ok(command) = command_rx.recv() {
            match command {
                AppRuntimeCommand::Dispatch {
                    action,
                    requester_context,
                    response,
                } => {
                    if state.prepare_dispatch(&action) {
                        let snapshot = state.app.clone();
                        publish_app_update(&snapshot, &shared_state, &reconciler);
                    }
                    let completed_action = action.clone();
                    let result = match state.dispatch(action, requester_context.as_ref()) {
                        Ok(()) => {
                            let snapshot = state.app.clone();
                            publish_app_update(&snapshot, &shared_state, &reconciler);
                            Ok(snapshot)
                        }
                        Err(error) => {
                            if state.finish_failed_dispatch(&completed_action, &error) {
                                state.bump_rev();
                                let snapshot = state.app.clone();
                                publish_app_update(&snapshot, &shared_state, &reconciler);
                            }
                            Err(error)
                        }
                    };
                    let run_selection_housekeeping = result.is_ok()
                        && matches!(
                            completed_action,
                            AppAction::OpenRoom { .. }
                                | AppAction::OpenTopic { .. }
                                | AppAction::OpenChat { .. }
                        );
                    if let Some(response) = response {
                        let _ = response.send(result);
                    }
                    if run_selection_housekeeping {
                        let before = state.app.clone();
                        if let Err(error) = state.run_selection_housekeeping()
                            && std::env::var_os("FINITECHAT_DEBUG_SELECTION_HOUSEKEEPING").is_some()
                        {
                            eprintln!("finitechat selection housekeeping error: {error:?}");
                        }
                        if state.app != before {
                            state.bump_rev();
                            let snapshot = state.app.clone();
                            publish_app_update(&snapshot, &shared_state, &reconciler);
                        }
                    }
                }
                AppRuntimeCommand::WaitPlan {
                    timeout_millis,
                    response,
                } => {
                    let _ = response.send(Ok(state.wait_plan(timeout_millis)));
                }
                AppRuntimeCommand::ApplySyncHint { event, response } => {
                    let result = (|| {
                        if matches!(event, SyncHintEvent::Heartbeat) {
                            return Ok(state.app.clone());
                        }
                        state.apply_app_sync_hint(&event)?;
                        state.bump_rev();
                        let snapshot = state.app.clone();
                        publish_app_update(&snapshot, &shared_state, &reconciler);
                        Ok(snapshot)
                    })();
                    let _ = response.send(result);
                }
                AppRuntimeCommand::AgentBridgeApplySyncHint { event, response } => {
                    let result = state.agent_bridge_apply_sync_hint(event);
                    if result.as_ref().is_ok_and(|bridge| {
                        !bridge.joined_account_ids.is_empty() || !bridge.events.is_empty()
                    }) {
                        state.bump_rev();
                        let snapshot = state.app.clone();
                        publish_app_update(&snapshot, &shared_state, &reconciler);
                    }
                    let _ = response.send(result);
                }
                AppRuntimeCommand::AgentBridgePoll { response } => {
                    let result = (|| {
                        let bridge = state.agent_bridge_poll_once()?;
                        state.bump_rev();
                        let snapshot = state.app.clone();
                        publish_app_update(&snapshot, &shared_state, &reconciler);
                        Ok(bridge)
                    })();
                    let _ = response.send(result);
                }
                AppRuntimeCommand::RecentBridgeEvents { limit, response } => {
                    let _ = response.send(state.recent_bridge_events(limit));
                }
                AppRuntimeCommand::SendRuntimeEvent {
                    room_id,
                    conversation_id,
                    kind,
                    payload,
                    response,
                } => {
                    let _ = response.send(state.send_runtime_command_request(
                        room_id,
                        conversation_id,
                        kind,
                        payload,
                    ));
                }
                AppRuntimeCommand::SendEncodedChatMessage {
                    room_id,
                    app_event_plaintext,
                    preview,
                    response,
                } => {
                    let result = (|| {
                        let sent = state.send_encoded_chat_message(
                            room_id,
                            app_event_plaintext,
                            preview,
                        )?;
                        state.bump_rev();
                        let snapshot = state.app.clone();
                        publish_app_update(&snapshot, &shared_state, &reconciler);
                        Ok(sent)
                    })();
                    let _ = response.send(result);
                }
                AppRuntimeCommand::UploadBridgeAttachment {
                    room_id,
                    attachment,
                    response,
                } => {
                    let _ = response.send(state.upload_bridge_attachment(room_id, attachment));
                }
                AppRuntimeCommand::AppendEphemeralActivity { input, response } => {
                    let result = (|| {
                        let accepted = state.append_ephemeral_activity(input)?;
                        state.bump_rev();
                        let snapshot = state.app.clone();
                        publish_app_update(&snapshot, &shared_state, &reconciler);
                        Ok(accepted)
                    })();
                    let _ = response.send(result);
                }
                AppRuntimeCommand::DebugOutbox { response } => {
                    let _ = response.send(state.app_outbox_debug_rows());
                }
                AppRuntimeCommand::LinkDevice {
                    fanout_id,
                    target_device_id,
                    response,
                } => {
                    let result = state.link_device(fanout_id, target_device_id);
                    if result.is_ok() {
                        state.bump_rev();
                        let snapshot = state.app.clone();
                        publish_app_update(&snapshot, &shared_state, &reconciler);
                    }
                    let _ = response.send(result);
                }
                AppRuntimeCommand::ProfileChatRooms {
                    account_id,
                    response,
                } => {
                    let _ = response.send(Ok(state.profile_chat_room_ids(&account_id)));
                }
                AppRuntimeCommand::ClaimProfileChatKeyPackage {
                    account_id,
                    response,
                } => {
                    let _ = response.send(state.claim_profile_chat_key_package(account_id));
                }
                AppRuntimeCommand::PlanProfileChatBootstrapRoom {
                    intended_room_id,
                    response,
                } => {
                    let _ = response.send(state.plan_profile_chat_bootstrap_room(intended_room_id));
                }
                AppRuntimeCommand::PrepareProfileChatBootstrap {
                    input,
                    fail_after_room_server_acceptance,
                    response,
                } => {
                    let result = state
                        .prepare_profile_chat_bootstrap(*input, fail_after_room_server_acceptance)
                        .map(|mut preparation| {
                            state.bump_rev();
                            let snapshot = state.app.clone();
                            publish_app_update(&snapshot, &shared_state, &reconciler);
                            preparation.state = snapshot;
                            preparation
                        });
                    let _ = response.send(result);
                }
                AppRuntimeCommand::SubmitProfileChatBootstrap {
                    agent_account_id,
                    display_name,
                    intended_room_id,
                    prepared_commit,
                    fail_after_device_save,
                    response,
                } => {
                    let result = state
                        .submit_profile_chat_bootstrap(
                            agent_account_id,
                            display_name,
                            intended_room_id,
                            *prepared_commit,
                            fail_after_device_save,
                        )
                        .map(|()| {
                            state.bump_rev();
                            let snapshot = state.app.clone();
                            publish_app_update(&snapshot, &shared_state, &reconciler);
                            snapshot
                        });
                    let _ = response.send(result);
                }
                AppRuntimeCommand::StartTopicChatIntent {
                    room_id,
                    topic_id,
                    reason,
                    intent_key,
                    response,
                } => {
                    let result = state
                        .start_topic_chat_intent(room_id, topic_id, reason, intent_key)
                        .map(|()| {
                            state.bump_rev();
                            let snapshot = state.app.clone();
                            publish_app_update(&snapshot, &shared_state, &reconciler);
                            snapshot
                        });
                    let _ = response.send(result);
                }
                #[cfg(test)]
                AppRuntimeCommand::TestLoadOutbox { response } => {
                    let _ = response.send(state.test_outbox());
                }
                #[cfg(test)]
                AppRuntimeCommand::TestSaveOutbox { rows, response } => {
                    let _ = response.send(state.test_save_outbox(rows));
                }
                #[cfg(test)]
                AppRuntimeCommand::TestRevokedDevices { response } => {
                    let _ = response.send(Ok(state.revoked_devices.clone()));
                }
                #[cfg(test)]
                AppRuntimeCommand::TestSeedRoomState {
                    room,
                    selected_room_id,
                    response,
                } => {
                    let _ = response.send(state.test_seed_room_state(room, selected_room_id));
                }
            }
        }
    });
}

fn publish_app_update(
    snapshot: &AppState,
    shared_state: &Arc<Mutex<AppState>>,
    reconciler: &Arc<Mutex<Option<Box<dyn AppReconciler>>>>,
) {
    if let Ok(mut shared) = shared_state.lock() {
        *shared = snapshot.clone();
    }
    if let Ok(slot) = reconciler.lock()
        && let Some(listener) = slot.as_ref()
    {
        listener.reconcile(AppUpdate::FullState(snapshot.clone()));
    }
}

impl AppRuntimeState {
    fn new(mut core: CoreState) -> Result<Self, FiniteChatCoreError> {
        let identity = core.identity();
        let owner = core.device.device_ref().clone();
        let stored_app_state = core.store.load_app_state(&owner).map_err(store_error)?;
        let stored_rooms = core.store.load_app_rooms(&owner).map_err(store_error)?;
        let completed_link_bootstrap_receipts = core
            .store
            .load_completed_device_link_bootstrap_receipts(&owner)
            .map_err(store_error)?;
        let completed_link_bootstrap_room_ids = completed_link_bootstrap_receipts
            .iter()
            .map(|receipt| receipt.room_id.clone())
            .collect::<BTreeSet<_>>();
        let known_room_ids = core.known_room_ids().into_iter().collect::<BTreeSet<_>>();
        let mut authoritative_room_ids = stored_rooms
            .iter()
            .filter(|room| room.display_name != room.room_id)
            .map(|room| room.room_id.clone())
            .collect::<BTreeSet<_>>();
        authoritative_room_ids.extend(completed_link_bootstrap_room_ids.iter().cloned());
        let provisional_room_ids = known_room_ids
            .difference(&authoritative_room_ids)
            .filter(|room_id| {
                core.device.room_members(room_id).is_ok_and(|members| {
                    members.iter().any(|member| {
                        member.account_id == owner.account_id && member.device_id != owner.device_id
                    })
                })
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        let stored_messages = core
            .store
            .load_app_messages(&owner, MAX_APP_MESSAGES_U32)
            .map_err(store_error)?
            .into_iter()
            .filter(|message| !provisional_room_ids.contains(&message.room_id))
            .collect::<Vec<_>>();
        let all_stored_events = core
            .store
            .load_app_events(&owner, MAX_APP_MESSAGES_U32)
            .map_err(store_error)?;
        // Additive migration for control events saved by builds from before
        // target staging existed. Current runtime sync already stages each
        // chunk in the same SQLite transaction that advances the room cursor.
        // Read-only opens skip the migration: they never write.
        for event in all_stored_events
            .iter()
            .filter(|event| provisional_room_ids.contains(&event.room_id))
        {
            if core.read_only {
                break;
            }
            if let Some(bootstrap) = device_link_bootstrap_from_stored_event(event) {
                let _ = core
                    .store
                    .stage_device_link_bootstrap_chunk(&owner, &event.sender, event.seq, &bootstrap)
                    .map_err(store_error)?;
            }
        }
        let ready_link_bootstraps = core
            .store
            .load_pending_device_link_bootstraps(&owner)
            .map_err(store_error)?
            .into_iter()
            .filter(StoredPendingDeviceLinkBootstrap::is_complete)
            .collect::<Vec<_>>();
        let stored_events = all_stored_events
            .into_iter()
            .filter(|event| !provisional_room_ids.contains(&event.room_id))
            .collect::<Vec<_>>();
        let delivered_local_messages = stored_messages
            .iter()
            .filter(|message| message.sender == owner)
            .map(|message| (message.room_id.clone(), message.message_id.clone()))
            .collect::<BTreeSet<_>>();
        let mut chat_projection =
            ChatProjectionState::from_stored(stored_messages, stored_events, &owner);
        chat_projection.restore_chat_archives(&stored_app_state.chat_archives);
        let stored_outbox = core.store.load_app_outbox(&owner).map_err(store_error)?;
        let mut visible_outbox = Vec::new();
        for message in stored_outbox {
            if delivered_local_messages
                .contains(&(message.room_id.clone(), message.message_id.clone()))
            {
                // Read-only opens project the same visible state but leave the
                // durable cleanup to the writer.
                if !core.read_only {
                    core.store
                        .delete_app_outbox_message(&owner, &message.room_id, &message.message_id)
                        .map_err(store_error)?;
                }
            } else {
                visible_outbox.push(message);
            }
        }
        chat_projection.append_messages(
            visible_outbox
                .into_iter()
                .filter_map(|message| chat_message_from_outbox(message, &owner))
                .collect(),
            &owner,
        );
        let should_persist_chat_archive_repair =
            stored_app_state.chat_archives != chat_projection.stored_chat_archives();
        let all_messages = chat_projection.messages();
        let mut persisted_room_ids = BTreeSet::new();
        let mut local_read_seq = BTreeMap::new();
        let mut rooms = Vec::new();
        let mut repaired_room_ids = Vec::new();
        for stored_room in stored_rooms {
            let room_id = stored_room.room_id.clone();
            let has_mls_room = known_room_ids.contains(&room_id);
            let stored_state = app_room_state_from_stored(stored_room.state);
            let stored_status = stored_room.status.clone();
            persisted_room_ids.insert(room_id.clone());
            local_read_seq.insert(room_id.clone(), stored_room.local_read_seq);
            let room = app_room_from_stored(stored_room, has_mls_room);
            if room.state != stored_state || room.status != stored_status {
                repaired_room_ids.push(room_id);
            }
            rooms.push(room);
        }
        for room_id in known_room_ids {
            if !persisted_room_ids.contains(&room_id) {
                local_read_seq.entry(room_id.clone()).or_default();
                rooms.push(connected_app_room(&room_id, &room_id));
            }
        }
        sort_app_rooms(&mut rooms);
        apply_room_message_projection(&mut rooms, &all_messages, &local_read_seq);
        // Pre-bootstrap linked Devices persisted the first discovered room as
        // though it were a user choice. Their generic `room_id` label is the
        // migration signal that this selection is still provisional. Once a
        // bootstrap hydrates room metadata, subsequent persisted navigation is
        // treated as explicit and is never overridden.
        let selection_is_explicit_or_bootstrapped = stored_app_state
            .selected_room_id
            .as_ref()
            .and_then(|selected| rooms.iter().find(|room| room.room_id == *selected))
            .is_some_and(|room| {
                room.display_name != room.room_id
                    || completed_link_bootstrap_room_ids.contains(&room.room_id)
            });
        let selected_room_id = selected_room_id_from_stored(&rooms, &stored_app_state);
        let revoked_devices = stored_app_state
            .revoked_devices
            .iter()
            .map(|device| app_device_key(&device.account_id, &device.device_id))
            .collect();
        let should_persist_selected_room_repair =
            selected_room_id.is_some() && stored_app_state.selected_room_id != selected_room_id;
        let mut loaded_message_counts = BTreeMap::new();
        if let Some(room_id) = selected_room_id.clone() {
            loaded_message_counts.insert(room_id, DEFAULT_TRANSCRIPT_WINDOW);
        }
        let mut state = Self {
            core,
            app: AppState {
                rev: 0,
                identity,
                paired_agent: stored_app_state.paired_agent.map(|paired| AppPairedAgent {
                    agent_account_id: paired.agent_account_id,
                    canonical_room_id: paired.canonical_room_id,
                }),
                selected_room_id,
                topics: Vec::new(),
                selected_topic_id: None,
                selected_chat_id: None,
                rooms,
                active_profile_id: None,
                status: "ready".to_owned(),
                toast: None,
                messages: Vec::new(),
                media_gallery: None,
                room_details: None,
                profiles: Vec::new(),
                devices: Vec::new(),
                typing_members: Vec::new(),
                device_link_bootstrap_receipts: completed_link_bootstrap_receipts
                    .iter()
                    .map(app_device_link_bootstrap_receipt)
                    .collect(),
                flow: AppFlowState::default(),
            },
            chat_projection,
            activity_projection: EphemeralActivityProjection::default(),
            local_typing_leases: BTreeMap::new(),
            loaded_message_counts,
            local_read_seq,
            profile_cache: BTreeMap::new(),
            profile_records: BTreeMap::new(),
            revoked_devices,
            downloading_attachments: BTreeSet::new(),
            bridge_seen_joined_account_ids: BTreeSet::new(),
            room_sync_failures: BTreeSet::new(),
            inbox_hint_after_seq: 0,
            profile_chat_bootstrap_preparations: BTreeMap::new(),
            selection_is_explicit_or_bootstrapped,
        };
        for room_id in &authoritative_room_ids {
            state.replay_room_history_into_projection(room_id)?;
        }
        state.sync_chat_projection();
        if let Some(room_id) = state.app.selected_room_id.clone() {
            state.app.selected_topic_id = stored_app_state.selected_topic_id.clone();
            state.app.selected_chat_id = stored_app_state.selected_chat_id.clone();
            if state.app.selected_topic_id.is_none() && state.topic_exists(&room_id, HOME_TOPIC_ID)
            {
                state.app.selected_topic_id = Some(HOME_TOPIC_ID.to_owned());
                state.app.selected_chat_id =
                    state.default_chat_id_for_topic(&room_id, HOME_TOPIC_ID);
            }
            state.repair_selected_topic();
            state.sync_selected_room_messages();
        }
        for room_id in repaired_room_ids {
            state.persist_room_projection(&room_id)?;
        }
        if should_persist_selected_room_repair
            || should_persist_chat_archive_repair
            || stored_app_state.selected_topic_id != state.app.selected_topic_id
            || stored_app_state.selected_chat_id != state.app.selected_chat_id
        {
            state.persist_app_state()?;
        }
        state.load_profile_cache()?;
        if !state.core.read_only {
            let mut bootstrap_selection = None;
            for pending in ready_link_bootstraps {
                if let Some(accepted) = state.finish_link_device_bootstrap(pending)? {
                    bootstrap_selection = bootstrap_selection.or(accepted.canonical_selection);
                }
            }
            if let Some(selection) = bootstrap_selection {
                state.apply_link_device_bootstrap_selection(selection)?;
            }
        }
        Ok(state)
    }

    fn prepare_dispatch(&mut self, action: &AppAction) -> bool {
        match action {
            AppAction::ScanTarget { .. } => {
                self.app.flow.scan_in_flight = true;
                self.app.flow.notice_busy = true;
                self.app.flow.notice_text = None;
                self.app.flow.scan_result = AppScanTargetOutcome::None;
                true
            }
            AppAction::UploadImage { .. } => {
                self.app.flow.image_upload_url = None;
                true
            }
            _ => false,
        }
    }

    fn finish_failed_dispatch(&mut self, action: &AppAction, error: &FiniteChatCoreError) -> bool {
        match action {
            AppAction::ScanTarget { .. } => {
                self.app.flow.scan_in_flight = false;
                self.app.flow.notice_busy = false;
                self.app.flow.scan_result = AppScanTargetOutcome::Unavailable;
                self.app.flow.notice_text = Some(scan_target_failure_message(error));
                self.app.status = "scan unavailable".to_owned();
                true
            }
            _ => false,
        }
    }

    fn finish_scan_target(&mut self, outcome: AppScanTargetOutcome, notice_text: Option<String>) {
        self.app.flow.scan_in_flight = false;
        self.app.flow.notice_busy = false;
        self.app.flow.scan_result = outcome;
        self.app.flow.notice_text = notice_text;
    }

    fn dispatch(
        &mut self,
        action: AppAction,
        requester_context: Option<&VerifiedRequesterContext>,
    ) -> Result<(), FiniteChatCoreError> {
        if self.core.read_only {
            return Err(FiniteChatCoreError::ReadOnly);
        }
        self.app.toast = None;
        self.core.refresh_device_clock()?;
        match action {
            AppAction::StartRuntime => self.start_runtime()?,
            AppAction::StopRuntime => self.app.status = "stopped".to_owned(),
            AppAction::OpenRoom { room_id } => self.open_room(room_id)?,
            AppAction::OpenTopic { room_id, topic_id } => self.open_topic(room_id, topic_id)?,
            AppAction::OpenChat {
                room_id,
                topic_id,
                chat_id,
            } => self.open_chat(room_id, topic_id, chat_id)?,
            AppAction::PairAgent { room_id } => self.pair_agent(room_id)?,
            AppAction::StartHomeChat { text, intent_key } => {
                self.start_home_chat(text, intent_key)?
            }
            AppAction::RenameChat {
                room_id,
                topic_id,
                chat_id,
                title,
            } => self.rename_chat(room_id, topic_id, chat_id, title)?,
            AppAction::SetChatArchived {
                room_id,
                topic_id,
                chat_id,
                archived,
            } => self.set_chat_archived(room_id, topic_id, chat_id, archived)?,
            AppAction::CreateRoom { display_name } => self.create_room(display_name)?,
            AppAction::CreateTopic { room_id, title } => self.create_topic(room_id, title)?,
            AppAction::StartTopicChat {
                room_id,
                topic_id,
                reason,
            } => self.start_topic_chat(room_id, topic_id, reason)?,
            AppAction::SaveProfile {
                display_name,
                about,
                picture,
            } => self.save_profile(display_name, about, picture)?,
            AppAction::UploadImage {
                bytes,
                content_type,
            } => self.upload_image(bytes, content_type)?,
            AppAction::SaveRoomMetadata {
                room_id,
                display_name,
                picture,
            } => self.save_room_metadata(room_id, display_name, picture)?,
            AppAction::StartProfileChat {
                profile,
                display_name,
            } => self.start_profile_chat(profile, display_name)?,
            AppAction::StartGroupChat {
                profiles,
                display_name,
            } => self.start_group_chat(profiles, display_name)?,
            AppAction::AddRoomMembers { room_id, profiles } => {
                self.add_room_members(room_id, profiles)?
            }
            AppAction::ScanTarget { value } => self.scan_target(value)?,
            AppAction::SendMessage {
                room_id,
                text,
                metadata_json,
            } => self.send_message(room_id, text, metadata_json.as_deref(), requester_context)?,
            AppAction::SendTopicMessage {
                room_id,
                topic_id,
                text,
                metadata_json,
            } => self.send_topic_message(
                room_id,
                topic_id,
                text,
                metadata_json.as_deref(),
                requester_context,
            )?,
            AppAction::SendChatMessage {
                room_id,
                topic_id,
                chat_id,
                text,
                metadata_json,
            } => self.send_chat_message(
                room_id,
                topic_id,
                chat_id,
                OutboundChatText {
                    text,
                    reply_to_message_id: None,
                    metadata_json,
                },
                requester_context,
            )?,
            AppAction::SendReply {
                room_id,
                text,
                reply_to_message_id,
                metadata_json,
            } => self.send_reply(
                room_id,
                text,
                reply_to_message_id,
                metadata_json.as_deref(),
                requester_context,
            )?,
            AppAction::SendChatReply {
                room_id,
                topic_id,
                chat_id,
                text,
                reply_to_message_id,
                metadata_json,
            } => self.send_chat_reply(
                room_id,
                topic_id,
                chat_id,
                OutboundChatText {
                    text,
                    reply_to_message_id: Some(reply_to_message_id),
                    metadata_json,
                },
                requester_context,
            )?,
            AppAction::SendAttachment {
                room_id,
                filename,
                mime_type,
                kind,
                bytes,
                caption,
                reply_to_message_id,
            } => self.send_attachment(SendAttachmentInput {
                room_id,
                conversation_id: None,
                chat_id: None,
                attachments: vec![OutboundAttachment {
                    filename,
                    mime_type,
                    kind,
                    bytes,
                }],
                caption,
                reply_to_message_id,
            })?,
            AppAction::SendChatAttachment {
                room_id,
                topic_id,
                chat_id,
                filename,
                mime_type,
                kind,
                bytes,
                caption,
                reply_to_message_id,
            } => self.send_chat_attachment(SendAttachmentInput {
                room_id,
                conversation_id: Some(topic_id),
                chat_id: Some(chat_id),
                attachments: vec![OutboundAttachment {
                    filename,
                    mime_type,
                    kind,
                    bytes,
                }],
                caption,
                reply_to_message_id,
            })?,
            AppAction::SendAttachments {
                room_id,
                attachments,
                caption,
                reply_to_message_id,
            } => self.send_attachment(SendAttachmentInput {
                room_id,
                conversation_id: None,
                chat_id: None,
                attachments,
                caption,
                reply_to_message_id,
            })?,
            AppAction::SendChatAttachments {
                room_id,
                topic_id,
                chat_id,
                attachments,
                caption,
                reply_to_message_id,
            } => self.send_chat_attachment(SendAttachmentInput {
                room_id,
                conversation_id: Some(topic_id),
                chat_id: Some(chat_id),
                attachments,
                caption,
                reply_to_message_id,
            })?,
            AppAction::SendPoll {
                room_id,
                question,
                options,
            } => self.send_poll(room_id, question, options)?,
            AppAction::SendChatPoll {
                room_id,
                topic_id,
                chat_id,
                question,
                options,
            } => self.send_chat_poll(room_id, topic_id, chat_id, question, options)?,
            AppAction::VotePoll {
                room_id,
                message_id,
                option_id,
            } => self.vote_poll(room_id, message_id, option_id)?,
            AppAction::DownloadAttachment {
                room_id,
                message_id,
                attachment_id,
            } => self.download_attachment(room_id, message_id, attachment_id)?,
            AppAction::BeginDownloadAttachment {
                room_id,
                message_id,
                attachment_id,
            } => self.begin_download_attachment(room_id, message_id, attachment_id)?,
            AppAction::LoadOlderMessages {
                room_id,
                before_message_id,
                limit,
            } => self.load_older_messages(room_id, before_message_id, limit)?,
            AppAction::ReactToMessage {
                room_id,
                message_id,
                emoji,
            } => self.react_to_message(room_id, message_id, emoji)?,
            AppAction::MarkRoomRead { room_id } => self.mark_room_read(room_id)?,
            AppAction::RetryMessage {
                room_id,
                message_id,
            } => self.retry_message(room_id, message_id)?,
            AppAction::SetTyping { room_id, is_typing } => self.set_typing(room_id, is_typing)?,
            AppAction::RefreshDevices => self.refresh_devices()?,
            AppAction::RevokeDevice {
                account_id,
                device_id,
            } => self.revoke_device(account_id, device_id)?,
            AppAction::SetPushToken { token } => self.set_push_token(token)?,
            AppAction::RemovePushToken => self.remove_push_token()?,
        }
        self.bump_rev();
        Ok(())
    }

    fn upload_image(
        &mut self,
        bytes: Vec<u8>,
        content_type: String,
    ) -> Result<(), FiniteChatCoreError> {
        let image_url = self.core.upload_image_blob(&bytes, &content_type)?;
        self.app.flow.image_upload_url = Some(image_url);
        self.app.status = "image uploaded".to_owned();
        Ok(())
    }

    fn start_runtime(&mut self) -> Result<(), FiniteChatCoreError> {
        let runtime_result = self.runtime_tick();
        if let Err(error) = self.refresh_own_profile()
            && !online_action_failure(&error)
        {
            return Err(error);
        }
        match runtime_result {
            Ok(()) => Ok(()),
            Err(error @ FiniteChatCoreError::Delivery { .. }) => {
                if std::env::var_os("FINITECHAT_DEBUG_START_RUNTIME_DELIVERY").is_some() {
                    eprintln!("finitechat start_runtime delivery error: {error:?}");
                }
                self.app.status = "offline".to_owned();
                self.app.toast = Some("Showing saved chats. Connection will retry.".to_owned());
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn refresh_own_profile(&mut self) -> Result<(), FiniteChatCoreError> {
        let account_id = self.core.device.device_ref().account_id.clone();
        self.fetch_profiles(vec![account_id]).map(|_| ())
    }

    fn wait_plan(&self, timeout_millis: u64) -> AppRuntimeWaitPlan {
        let server_url = self.core.server_url.clone();
        let rooms = self
            .core
            .device
            .room_sync_cursors()
            .into_iter()
            .filter(|cursor| cursor.server_url.as_deref().unwrap_or(&server_url) == server_url)
            .map(|cursor| SyncWaitRoom {
                room_id: cursor.room_id,
                after_seq: cursor.after_seq,
            })
            .collect::<Vec<_>>();

        AppRuntimeWaitPlan {
            server_url,
            request: SyncStreamRequest {
                rooms,
                inbox: Some(SyncWaitInbox::new(
                    delivery_member_id_for_device(self.core.device.device_ref()),
                    self.inbox_hint_after_seq,
                )),
                heartbeat_ms: Some(normalize_app_update_wait_millis(timeout_millis)),
            },
        }
    }

    fn set_push_token(&mut self, token: String) -> Result<(), FiniteChatCoreError> {
        let token = token.trim().to_owned();
        let mut delivery = self.core.home_delivery();
        delivery
            .register_push_token(self.core.device.device_ref(), PushPlatform::Apns, token)
            .map_err(send_delivery_error)?;
        Ok(())
    }

    fn remove_push_token(&mut self) -> Result<(), FiniteChatCoreError> {
        let mut delivery = self.core.home_delivery();
        delivery
            .remove_push_token(self.core.device.device_ref())
            .map_err(delivery_error)?;
        Ok(())
    }

    fn apply_sync_hint(&mut self, event: &SyncHintEvent) {
        if let SyncHintEvent::InboxAdvanced { seq } = event {
            self.inbox_hint_after_seq = self.inbox_hint_after_seq.max(*seq);
        }
    }

    fn apply_app_sync_hint(&mut self, event: &SyncHintEvent) -> Result<(), FiniteChatCoreError> {
        // Room/activity hints name an exact, already-watched scope. Unknown
        // scopes and inbox work retain the full reconciliation path.
        match event {
            SyncHintEvent::RoomAdvanced { room_id, .. } if self.core.has_room(room_id) => {
                self.expire_ephemeral_activity()?;
                let synced = self.core.sync_room_with_projection(room_id)?;
                self.apply_targeted_sync_projection(room_id, synced)?;
            }
            SyncHintEvent::ActivityChanged { room_id, .. }
                if self.room_is_connected(room_id) && self.core.has_room(room_id) =>
            {
                self.refresh_ephemeral_activity_for_room(room_id)?;
                self.drain_undelivered_outbox(MAX_OUTBOX_DRAIN_PER_TICK)?;
                self.app.status = ready_status(self.room_sync_failures.len());
            }
            SyncHintEvent::Heartbeat => {}
            SyncHintEvent::RoomAdvanced { .. }
            | SyncHintEvent::ActivityChanged { .. }
            | SyncHintEvent::InboxAdvanced { .. } => self.runtime_tick()?,
        }
        self.apply_sync_hint(event);
        Ok(())
    }

    fn runtime_tick(&mut self) -> Result<(), FiniteChatCoreError> {
        self.refresh_ephemeral_activity_for_connected_rooms()?;
        let synced = self.core.sync_with_projection()?;
        self.room_sync_failures = synced.room_sync_failures.iter().cloned().collect();
        self.apply_projection_events(synced.events)?;
        self.append_messages(synced.result.messages);
        self.materialize_known_connected_rooms()?;
        self.drain_undelivered_outbox(MAX_OUTBOX_DRAIN_PER_TICK)?;
        self.app.status = ready_status(self.room_sync_failures.len());
        Ok(())
    }

    fn apply_targeted_sync_projection(
        &mut self,
        room_id: &str,
        synced: CoreSyncProjection,
    ) -> Result<(), FiniteChatCoreError> {
        if synced
            .room_sync_failures
            .iter()
            .any(|failed_room_id| failed_room_id == room_id)
        {
            self.room_sync_failures.insert(room_id.to_owned());
        } else {
            self.room_sync_failures.remove(room_id);
        }
        self.apply_projection_events(synced.events)?;
        self.append_messages(synced.result.messages);
        self.materialize_known_connected_rooms()?;
        self.drain_undelivered_outbox(MAX_OUTBOX_DRAIN_PER_TICK)?;
        self.app.status = ready_status(self.room_sync_failures.len());
        Ok(())
    }

    fn agent_bridge_poll_once(&mut self) -> Result<AppBridgeSync, FiniteChatCoreError> {
        self.agent_bridge_sync_after_change()
    }

    fn recent_bridge_events(
        &self,
        limit: u32,
    ) -> Result<Vec<AppBridgeAppliedEvent>, FiniteChatCoreError> {
        self.core
            .store
            .load_app_events(self.core.device.device_ref(), limit)
            .map(|events| {
                events
                    .iter()
                    .filter(|event| !is_device_link_control_event(&event.plaintext))
                    .map(app_bridge_event_from_stored_event)
                    .collect()
            })
            .map_err(store_error)
    }

    fn agent_bridge_apply_sync_hint(
        &mut self,
        event: SyncHintEvent,
    ) -> Result<AppBridgeSync, FiniteChatCoreError> {
        // The resident bridge deliberately keeps heartbeat reconciliation
        // full so a missed or dropped hint cannot strand durable Chat data.
        let bridge = match &event {
            SyncHintEvent::RoomAdvanced { room_id, .. } if self.core.has_room(room_id) => {
                self.agent_bridge_sync_room_after_change(room_id)?
            }
            SyncHintEvent::ActivityChanged { room_id, .. }
                if self.room_is_connected(room_id) && self.core.has_room(room_id) =>
            {
                self.refresh_ephemeral_activity_for_room(room_id)?;
                self.drain_undelivered_outbox(MAX_OUTBOX_DRAIN_PER_TICK)?;
                self.sync_selected_room_messages();
                self.app.status = ready_status(self.room_sync_failures.len());
                AppBridgeSync::default()
            }
            SyncHintEvent::RoomAdvanced { .. }
            | SyncHintEvent::ActivityChanged { .. }
            | SyncHintEvent::InboxAdvanced { .. }
            | SyncHintEvent::Heartbeat => self.agent_bridge_sync_after_change()?,
        };
        self.apply_sync_hint(&event);
        Ok(bridge)
    }

    fn agent_bridge_sync_after_change(&mut self) -> Result<AppBridgeSync, FiniteChatCoreError> {
        self.refresh_ephemeral_activity_for_connected_rooms()?;
        let synced = self.core.sync_with_projection()?;
        self.room_sync_failures = synced.room_sync_failures.iter().cloned().collect();
        let events = synced
            .events
            .iter()
            .filter(|event| !is_device_link_control_event(&event.plaintext))
            .map(app_bridge_event_from_stored_event)
            .collect::<Vec<_>>();
        self.apply_projection_events(synced.events)?;
        self.append_messages(synced.result.messages);
        self.materialize_known_connected_rooms()?;
        self.drain_undelivered_outbox(MAX_OUTBOX_DRAIN_PER_TICK)?;
        self.sync_selected_room_messages();
        self.app.status = ready_status(self.room_sync_failures.len());
        let joined_account_ids = self.bridge_unseen_joined_account_ids();
        Ok(AppBridgeSync {
            joined_account_ids,
            events,
        })
    }

    fn agent_bridge_sync_room_after_change(
        &mut self,
        room_id: &str,
    ) -> Result<AppBridgeSync, FiniteChatCoreError> {
        self.expire_ephemeral_activity()?;
        let synced = self.core.sync_room_with_projection(room_id)?;
        if synced
            .room_sync_failures
            .iter()
            .any(|failed_room_id| failed_room_id == room_id)
        {
            self.room_sync_failures.insert(room_id.to_owned());
        } else {
            self.room_sync_failures.remove(room_id);
        }
        let events = synced
            .events
            .iter()
            .filter(|event| !is_device_link_control_event(&event.plaintext))
            .map(app_bridge_event_from_stored_event)
            .collect::<Vec<_>>();
        self.apply_projection_events(synced.events)?;
        self.append_messages(synced.result.messages);
        self.materialize_known_connected_rooms()?;
        self.drain_undelivered_outbox(MAX_OUTBOX_DRAIN_PER_TICK)?;
        self.sync_selected_room_messages();
        self.app.status = ready_status(self.room_sync_failures.len());
        let joined_account_ids = self.bridge_unseen_joined_account_ids();
        Ok(AppBridgeSync {
            joined_account_ids,
            events,
        })
    }

    fn materialize_known_connected_rooms(&mut self) -> Result<(), FiniteChatCoreError> {
        let mut known_app_rooms = self
            .app
            .rooms
            .iter()
            .map(|room| room.room_id.clone())
            .collect::<BTreeSet<_>>();
        let mut stored_rooms = Vec::new();
        for room_id in self.core.known_room_ids() {
            if !known_app_rooms.insert(room_id.clone()) {
                continue;
            }
            let app_room = app_room_metadata(&room_id, None);
            self.upsert_room(
                &app_room.room_id,
                &app_room.display_name,
                None,
                AppRoomState::Connected,
                "connected",
            );
            stored_rooms.push(app_room);
        }
        if !stored_rooms.is_empty() {
            let owner = self.core.device.device_ref().clone();
            self.core
                .store
                .save_app_rooms(&owner, &stored_rooms)
                .map_err(store_error)?;
        }
        let connected_room_ids = self
            .app
            .rooms
            .iter()
            .filter(|room| {
                room.state == AppRoomState::Connected && self.core.has_room(&room.room_id)
            })
            .map(|room| room.room_id.clone())
            .collect::<Vec<_>>();
        for room_id in connected_room_ids {
            self.ensure_home_topic(&room_id)?;
        }
        if self.app.selected_room_id.is_none()
            && let Some(room_id) = self
                .app
                .rooms
                .iter()
                .filter(|room| {
                    room.state == AppRoomState::Connected && self.core.has_room(&room.room_id)
                })
                .map(|room| room.room_id.clone())
                .next()
        {
            self.app.selected_room_id = Some(room_id.clone());
            self.app.selected_topic_id = Some(HOME_TOPIC_ID.to_owned());
            self.app.selected_chat_id = self.default_chat_id_for_topic(&room_id, HOME_TOPIC_ID);
            self.loaded_message_counts
                .entry(room_id)
                .or_insert(DEFAULT_TRANSCRIPT_WINDOW);
            self.persist_app_state()?;
        }
        Ok(())
    }

    fn bridge_unseen_joined_account_ids(&mut self) -> Vec<String> {
        let own_account_id = self.core.device.device_ref().account_id.clone();
        let mut current = BTreeSet::new();
        for room in &self.app.rooms {
            if room.state != AppRoomState::Connected || !self.core.has_room(&room.room_id) {
                continue;
            }
            let Ok(members) = self.core.device.room_members(&room.room_id) else {
                continue;
            };
            for member in members {
                if member.account_id != own_account_id {
                    current.insert(member.account_id);
                }
            }
        }
        let joined = current
            .difference(&self.bridge_seen_joined_account_ids)
            .cloned()
            .collect::<Vec<_>>();
        self.bridge_seen_joined_account_ids.extend(current);
        joined
    }

    fn open_room(&mut self, room_id: String) -> Result<(), FiniteChatCoreError> {
        if self.app.selected_room_id.as_deref() == Some(room_id.as_str()) {
            // Re-opening the already-selected room is a read, not a selection
            // change: keep the current topic/chat selection so hosted-device
            // state reads cannot reset the chat out from under the user.
            return Ok(());
        }
        if self.room_is_connected(&room_id) {
            self.ensure_home_topic(&room_id)?;
        }
        self.app.selected_room_id = Some(room_id.clone());
        if self.topic_exists(&room_id, HOME_TOPIC_ID) {
            self.app.selected_topic_id = Some(HOME_TOPIC_ID.to_owned());
            self.app.selected_chat_id = self.default_chat_id_for_topic(&room_id, HOME_TOPIC_ID);
        } else {
            self.app.selected_topic_id = None;
            self.app.selected_chat_id = None;
        }
        self.loaded_message_counts
            .entry(room_id.clone())
            .or_insert(DEFAULT_TRANSCRIPT_WINDOW);
        if self.room_mut(&room_id).is_none() {
            self.upsert_room(
                &room_id,
                &room_id,
                None,
                AppRoomState::UnavailableOnDevice,
                LOCAL_ROOM_UNAVAILABLE_STATUS,
            );
        }
        self.persist_app_state()?;
        self.selection_is_explicit_or_bootstrapped = true;
        self.sync_selected_room_messages();
        Ok(())
    }

    fn open_topic(&mut self, room_id: String, topic_id: String) -> Result<(), FiniteChatCoreError> {
        if self.room(&room_id).is_none() {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not available"),
            });
        }
        if !self
            .app
            .topics
            .iter()
            .any(|topic| topic.room_id == room_id && topic.topic_id == topic_id)
        {
            return Err(FiniteChatCoreError::Client {
                reason: format!("topic '{topic_id}' is not available in room '{room_id}'"),
            });
        }
        self.app.selected_room_id = Some(room_id.clone());
        self.app.selected_chat_id = self.default_chat_id_for_topic(&room_id, &topic_id);
        self.app.selected_topic_id = Some(topic_id);
        self.loaded_message_counts
            .entry(room_id.clone())
            .or_insert(DEFAULT_TRANSCRIPT_WINDOW);
        self.persist_app_state()?;
        self.selection_is_explicit_or_bootstrapped = true;
        self.sync_selected_room_messages();
        Ok(())
    }

    fn open_chat(
        &mut self,
        room_id: String,
        topic_id: String,
        chat_id: String,
    ) -> Result<(), FiniteChatCoreError> {
        if !self.chat_exists(&room_id, &topic_id, &chat_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!(
                    "chat '{chat_id}' is not available in topic '{topic_id}' in room '{room_id}'"
                ),
            });
        }
        self.app.selected_room_id = Some(room_id.clone());
        self.app.selected_topic_id = Some(topic_id);
        self.app.selected_chat_id = Some(chat_id);
        self.loaded_message_counts
            .entry(room_id.clone())
            .or_insert(DEFAULT_TRANSCRIPT_WINDOW);
        self.persist_app_state()?;
        self.selection_is_explicit_or_bootstrapped = true;
        self.sync_selected_room_messages();
        Ok(())
    }

    fn run_selection_housekeeping(&mut self) -> Result<(), FiniteChatCoreError> {
        self.refresh_ephemeral_activity_for_connected_rooms()?;
        self.drain_undelivered_outbox(MAX_OUTBOX_DRAIN_PER_TICK)?;
        Ok(())
    }

    fn rename_chat(
        &mut self,
        room_id: String,
        topic_id: String,
        chat_id: String,
        title: String,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_chat_route(&room_id, &topic_id, &chat_id)?;
        let title = normalize_bounded_non_empty_string("chat title", &title, MAX_CHAT_TITLE_BYTES)?;
        let rename = ChatRenameV1 {
            topic_id: topic_id.clone(),
            chat_id: chat_id.clone(),
            title,
        };
        rename.validate_limits().map_err(client_error)?;
        let payload = serde_json::to_vec(&rename).map_err(client_error)?;
        let event = self.core.send_application_event_with_segment(
            &room_id,
            DurableAppEventKind::Namespaced {
                name: FINITECHAT_CHAT_RENAME_EVENT_V1.to_owned(),
                policy: ApplicationDeliveryPolicy::NON_NOTIFYING,
            },
            Some(topic_id.clone()),
            Some(chat_id.clone()),
            &payload,
            "chat-rename",
        )?;
        self.apply_projection_events(vec![event])?;
        self.app.status = "chat renamed".to_owned();
        Ok(())
    }

    fn set_chat_archived(
        &mut self,
        room_id: String,
        topic_id: String,
        chat_id: String,
        archived: bool,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_chat_route(&room_id, &topic_id, &chat_id)?;
        let archive = ChatArchiveV1 {
            topic_id: topic_id.clone(),
            chat_id: chat_id.clone(),
            archived,
        };
        archive.validate_limits().map_err(client_error)?;
        let payload = serde_json::to_vec(&archive).map_err(client_error)?;
        let event = self.core.send_application_event_with_segment(
            &room_id,
            DurableAppEventKind::Namespaced {
                name: FINITECHAT_CHAT_ARCHIVE_EVENT_V1.to_owned(),
                policy: ApplicationDeliveryPolicy::NON_NOTIFYING,
            },
            Some(topic_id.clone()),
            Some(chat_id.clone()),
            &payload,
            "chat-archive",
        )?;
        self.apply_projection_events(vec![event])?;
        self.app.status = if archived {
            "chat archived".to_owned()
        } else {
            "chat restored".to_owned()
        };
        Ok(())
    }

    fn create_room(&mut self, display_name: String) -> Result<(), FiniteChatCoreError> {
        let label = display_name.trim();
        if label.len() > MAX_PROFILE_DISPLAY_NAME_BYTES as usize {
            return Err(FiniteChatCoreError::Client {
                reason: format!(
                    "room display name must be at most {MAX_PROFILE_DISPLAY_NAME_BYTES} bytes"
                ),
            });
        }
        let room_id = self.core.generate_object_id("room")?;
        let display_name = if label.is_empty() {
            room_id.clone()
        } else {
            label.to_owned()
        };
        self.core
            .bootstrap_room(&room_id, Some(display_name.clone()))?;
        self.upsert_room(
            &room_id,
            &display_name,
            None,
            AppRoomState::Connected,
            "connected",
        );
        self.persist_room_projection(&room_id)?;
        self.ensure_home_topic(&room_id)?;
        self.app.selected_room_id = Some(room_id.clone());
        self.app.selected_topic_id = Some(HOME_TOPIC_ID.to_owned());
        self.app.selected_chat_id = self.default_chat_id_for_topic(&room_id, HOME_TOPIC_ID);
        self.persist_app_state()?;
        self.sync_selected_room_messages();
        self.app.status = "room created".to_owned();
        Ok(())
    }

    fn create_topic(&mut self, room_id: String, title: String) -> Result<(), FiniteChatCoreError> {
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to create topics"),
            });
        }
        let title = normalize_bounded_non_empty_string("topic title", &title, MAX_OBJECT_ID_BYTES)?;
        let topic_id = self.core.generate_object_id("topic")?;
        let metadata = ConversationMetadataV1 {
            title: Some(title),
            description: None,
            external_topic: None,
            skill_binding: None,
        };
        metadata.validate_limits().map_err(client_error)?;
        let payload = serde_json::to_vec(&metadata).map_err(client_error)?;
        let event = self.core.send_application_event(
            &room_id,
            DurableAppEventKind::ConversationCreate,
            Some(topic_id.clone()),
            &payload,
            "topic",
        )?;
        self.apply_projection_events(vec![event])?;
        self.app.selected_room_id = Some(room_id.clone());
        self.app.selected_topic_id = Some(topic_id.clone());
        self.app.selected_chat_id = None;
        self.loaded_message_counts
            .entry(room_id.clone())
            .or_insert(DEFAULT_TRANSCRIPT_WINDOW);
        self.start_topic_chat(room_id.clone(), topic_id.clone(), None)?;
        self.app.selected_topic_id = Some(topic_id);
        self.app.selected_chat_id =
            self.app
                .selected_topic_id
                .as_deref()
                .and_then(|selected_topic_id| {
                    self.default_chat_id_for_topic(&room_id, selected_topic_id)
                });
        self.persist_app_state()?;
        self.sync_selected_room_messages();
        self.app.status = "topic created".to_owned();
        Ok(())
    }

    fn start_topic_chat(
        &mut self,
        room_id: String,
        topic_id: String,
        reason: Option<String>,
    ) -> Result<(), FiniteChatCoreError> {
        let chat_id = self.append_topic_chat(&room_id, &topic_id, reason)?;
        self.app.selected_room_id = Some(room_id);
        self.app.selected_topic_id = Some(topic_id);
        self.app.selected_chat_id = Some(chat_id);
        self.persist_app_state()?;
        self.sync_selected_room_messages();
        self.app.status = "chat created".to_owned();
        Ok(())
    }

    fn pair_agent(&mut self, room_id: String) -> Result<(), FiniteChatCoreError> {
        let room = self
            .room(&room_id)
            .ok_or_else(|| FiniteChatCoreError::Client {
                reason: format!("agent room not found: {room_id}"),
            })?;
        if room.state != AppRoomState::Connected {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not an available agent"),
            });
        }
        let owner_account_id = self.app.identity.account_id.clone();
        let member_account_ids = self
            .core
            .device
            .room_members(&room_id)
            .map_err(client_error)?
            .into_iter()
            .map(|member| member.account_id)
            .collect::<BTreeSet<_>>();
        let agent_account_id = member_account_ids
            .iter()
            .find(|account_id| account_id.as_str() != owner_account_id)
            .filter(|_| member_account_ids.len() == 2)
            .filter(|account_id| {
                self.profile_cache
                    .get(*account_id)
                    .is_some_and(|profile| profile.is_agent)
            })
            .cloned()
            .ok_or_else(|| FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not an available direct agent"),
            })?;
        if !self
            .profile_chat_room_ids(&agent_account_id)
            .iter()
            .any(|candidate| candidate == &room_id)
        {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not the selected agent's direct room"),
            });
        }

        self.ensure_home_topic(&room_id)?;
        self.app.paired_agent = Some(AppPairedAgent {
            agent_account_id,
            canonical_room_id: room_id,
        });
        self.persist_app_state()?;
        self.app.status = "agent paired".to_owned();
        Ok(())
    }

    fn start_home_chat(
        &mut self,
        text: Option<String>,
        intent_key: String,
    ) -> Result<(), FiniteChatCoreError> {
        let paired = self
            .app
            .paired_agent
            .clone()
            .ok_or_else(|| FiniteChatCoreError::Client {
                reason: "choose an agent before starting a chat".to_owned(),
            })?;
        let room_id = paired.canonical_room_id;
        let trimmed = text.as_deref().map(str::trim);
        if trimmed.is_some_and(str::is_empty) {
            return Err(FiniteChatCoreError::Client {
                reason: "message must not be empty".to_owned(),
            });
        }
        let room = self
            .room(&room_id)
            .ok_or_else(|| FiniteChatCoreError::Client {
                reason: format!("agent room not found: {room_id}"),
            })?;
        if room.state != AppRoomState::Connected
            || !self
                .profile_chat_room_ids(&paired.agent_account_id)
                .iter()
                .any(|candidate| candidate == &room_id)
        {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not an available agent"),
            });
        }

        self.ensure_home_topic(&room_id)?;
        self.start_topic_chat_intent(room_id.clone(), HOME_TOPIC_ID.to_owned(), None, intent_key)?;
        let chat_id =
            self.app
                .selected_chat_id
                .clone()
                .ok_or_else(|| FiniteChatCoreError::Client {
                    reason: "new home chat did not select a chat".to_owned(),
                })?;
        let Some(trimmed) = trimmed else {
            self.app.status = "chat created".to_owned();
            return Ok(());
        };
        if self.app.messages.iter().any(|message| {
            message.room_id == room_id
                && message.conversation_id.as_deref() == Some(HOME_TOPIC_ID)
                && message.chat_id.as_deref() == Some(chat_id.as_str())
        }) {
            self.app.status = "chat started".to_owned();
            return Ok(());
        }
        self.send_chat_message(
            room_id,
            HOME_TOPIC_ID.to_owned(),
            chat_id,
            OutboundChatText {
                text: trimmed.to_owned(),
                reply_to_message_id: None,
                metadata_json: None,
            },
            None,
        )?;
        self.app.status = "chat started".to_owned();
        Ok(())
    }

    fn start_topic_chat_intent(
        &mut self,
        room_id: String,
        topic_id: String,
        reason: Option<String>,
        intent_key: String,
    ) -> Result<(), FiniteChatCoreError> {
        let intent_key = intent_key.trim();
        validate_string_bytes("new chat intent key", intent_key, MAX_OBJECT_ID_BYTES)
            .map_err(client_error)?;
        if intent_key.is_empty() {
            return Err(FiniteChatCoreError::Client {
                reason: "new chat intent key must not be empty".to_owned(),
            });
        }
        let digest = Sha256::digest(
            format!("finitechat.new-chat-intent.v1\0{room_id}\0{topic_id}\0{intent_key}")
                .as_bytes(),
        );
        let chat_id = format!("segment-{}", hex::encode(&digest[..20]));
        if !self.chat_exists(&room_id, &topic_id, &chat_id) {
            self.append_topic_chat_with_id(&room_id, &topic_id, reason, chat_id.clone())?;
        }
        self.app.selected_room_id = Some(room_id);
        self.app.selected_topic_id = Some(topic_id);
        self.app.selected_chat_id = Some(chat_id);
        self.persist_app_state()?;
        self.sync_selected_room_messages();
        self.app.status = "chat created".to_owned();
        Ok(())
    }

    fn append_topic_chat(
        &mut self,
        room_id: &str,
        topic_id: &str,
        reason: Option<String>,
    ) -> Result<String, FiniteChatCoreError> {
        if !self.room_is_connected(room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to start chats"),
            });
        }
        self.validate_topic(room_id, topic_id)?;
        let reason = reason
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let segment_id = self.core.generate_object_id("segment")?;
        self.append_topic_chat_with_id(room_id, topic_id, reason, segment_id)
    }

    fn append_topic_chat_with_id(
        &mut self,
        room_id: &str,
        topic_id: &str,
        reason: Option<String>,
        segment_id: String,
    ) -> Result<String, FiniteChatCoreError> {
        if !self.room_is_connected(room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to start chats"),
            });
        }
        self.validate_topic(room_id, topic_id)?;
        let reason = reason
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let segment = ConversationSegmentStartV1 { segment_id, reason };
        segment.validate_limits().map_err(client_error)?;
        let payload = serde_json::to_vec(&segment).map_err(client_error)?;
        let event = self.core.send_application_event(
            room_id,
            DurableAppEventKind::ConversationSegmentStart,
            Some(topic_id.to_owned()),
            &payload,
            "segment",
        )?;
        self.apply_projection_events(vec![event])?;
        Ok(segment.segment_id)
    }

    fn save_room_metadata(
        &mut self,
        room_id: String,
        display_name: String,
        picture: Option<String>,
    ) -> Result<(), FiniteChatCoreError> {
        let room_id = room_id.trim().to_owned();
        validate_string_bytes("room_id", &room_id, MAX_OBJECT_ID_BYTES).map_err(client_error)?;
        let display_name = normalize_required_profile_text("room name", display_name)?;
        validate_string_bytes(
            "room display name",
            &display_name,
            MAX_PROFILE_DISPLAY_NAME_BYTES,
        )
        .map_err(client_error)?;
        let picture = normalize_optional_room_picture_url(picture)?;
        if let Some(room) = self.room_mut(&room_id) {
            room.display_name = display_name;
            room.picture = picture;
            room.user_status_text = app_room_user_status_text(room);
        } else {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room not found: {room_id}"),
            });
        }
        sort_app_rooms(&mut self.app.rooms);
        self.persist_room_projection(&room_id)?;
        self.sync_selected_room_messages();
        self.app.status = "room saved".to_owned();
        Ok(())
    }

    fn start_profile_chat(
        &mut self,
        profile: AppProfileSummary,
        display_name: String,
    ) -> Result<(), FiniteChatCoreError> {
        let profile = normalize_profile_summary_hint(profile)?;
        let account_id = profile.account_id.clone();
        if account_id == self.app.identity.account_id {
            self.set_online_action_unavailable(
                "chat unavailable",
                "This profile is already signed in on this device",
            );
            return Ok(());
        }
        let label = display_name.trim();
        if label.len() > MAX_PROFILE_DISPLAY_NAME_BYTES as usize {
            return Err(FiniteChatCoreError::Client {
                reason: format!(
                    "room display name must be at most {MAX_PROFILE_DISPLAY_NAME_BYTES} bytes"
                ),
            });
        }
        let display_name = if label.is_empty() {
            format!("Chat with {}", short_account_label(&account_id))
        } else {
            label.to_owned()
        };
        let existing_room_ids = self.profile_chat_room_ids(&account_id);
        let existing_room_id = match existing_room_ids.as_slice() {
            [] => None,
            [room_id] => Some(room_id.clone()),
            _ => {
                return Err(FiniteChatCoreError::Client {
                    reason: "multiple direct Rooms exist for this profile; open one explicitly"
                        .to_owned(),
                });
            }
        };

        let mut profile = profile;
        if !profile_has_useful_name(&profile, &account_id)
            && let Some(display_name) =
                profile_name_hint_from_chat_label(&display_name, &account_id)
        {
            profile.display_name = display_name;
        }
        self.remember_profile_summary(profile)?;
        if let Some(room_id) = existing_room_id {
            self.ensure_home_topic(&room_id)?;
            let selected_route = if self.app.selected_room_id.as_deref() == Some(room_id.as_str()) {
                match (
                    self.app.selected_topic_id.clone(),
                    self.app.selected_chat_id.clone(),
                ) {
                    (Some(topic_id), Some(chat_id))
                        if self.chat_exists(&room_id, &topic_id, &chat_id) =>
                    {
                        Some((topic_id, chat_id))
                    }
                    _ => None,
                }
            } else {
                None
            };
            self.app.selected_room_id = Some(room_id.clone());
            if let Some((topic_id, chat_id)) = selected_route {
                self.app.selected_topic_id = Some(topic_id);
                self.app.selected_chat_id = Some(chat_id);
            } else {
                self.app.selected_topic_id = self
                    .topic_exists(&room_id, HOME_TOPIC_ID)
                    .then(|| HOME_TOPIC_ID.to_owned());
                self.app.selected_chat_id = self.default_chat_id_for_topic(&room_id, HOME_TOPIC_ID);
            }
            self.persist_app_state()?;
            self.sync_selected_room_messages();
            self.app.status = "chat opened".to_owned();
            self.app.toast = None;
            return Ok(());
        }

        let room_id = self.core.generate_object_id("room")?;
        self.create_profile_chat_room(account_id, display_name, room_id)
    }

    fn claim_profile_chat_key_package(
        &mut self,
        account_id: String,
    ) -> Result<Option<ClaimKeyPackageResult>, FiniteChatCoreError> {
        let mut delivery = self.core.home_delivery();
        delivery
            .claim_key_package_for_account(&account_id)
            .map_err(send_delivery_error)
    }

    fn plan_profile_chat_bootstrap_room(
        &mut self,
        intended_room_id: String,
    ) -> Result<CreateRoomRequest, FiniteChatCoreError> {
        validate_string_bytes("room_id", &intended_room_id, MAX_OBJECT_ID_BYTES)
            .map_err(client_error)?;
        let mls_group_id = if self.core.has_room(&intended_room_id) {
            self.core
                .device
                .room_mls_group_id(&intended_room_id)
                .map_err(client_error)?
        } else {
            self.core.generate_object_id("mls")?
        };
        let request = CreateRoomRequest {
            room_id: intended_room_id.clone(),
            mls_group_id,
            creator: self.core.device.device_ref().clone(),
            protocol: RoomProtocol::default(),
        };
        self.validate_profile_chat_bootstrap_room_create_request(&intended_room_id, &request)?;
        Ok(request)
    }

    fn prepare_profile_chat_bootstrap(
        &mut self,
        input: AppProfileChatBootstrapInput,
        fail_after_room_server_acceptance: bool,
    ) -> Result<AppProfileChatBootstrapPreparation, FiniteChatCoreError> {
        let AppProfileChatBootstrapInput {
            profile,
            display_name,
            intended_room_id,
            room_create_request,
            claimed_key_package,
            persisted_prepared_commit,
        } = input;
        validate_string_bytes("room_id", &intended_room_id, MAX_OBJECT_ID_BYTES)
            .map_err(client_error)?;
        self.validate_profile_chat_bootstrap_room_create_request(
            &intended_room_id,
            &room_create_request,
        )?;
        let profile = normalize_profile_summary_hint(profile)?;
        let account_id = profile.account_id.clone();
        if account_id == self.app.identity.account_id {
            return Err(FiniteChatCoreError::Client {
                reason: "a profile chat cannot target the current account".to_owned(),
            });
        }
        let label = display_name.trim();
        if label.len() > MAX_PROFILE_DISPLAY_NAME_BYTES as usize {
            return Err(FiniteChatCoreError::Client {
                reason: format!(
                    "room display name must be at most {MAX_PROFILE_DISPLAY_NAME_BYTES} bytes"
                ),
            });
        }
        let display_name = if label.is_empty() {
            format!("Chat with {}", short_account_label(&account_id))
        } else {
            label.to_owned()
        };
        if claimed_key_package.owner.account_id != account_id {
            return Err(FiniteChatCoreError::Client {
                reason: "the durable bootstrap KeyPackage targets a different Agent".to_owned(),
            });
        }

        if self.sync_and_validate_profile_chat_bootstrap_room(&account_id, &intended_room_id)? {
            self.profile_chat_bootstrap_preparations
                .remove(&intended_room_id);
            self.finalize_profile_chat_bootstrap(
                intended_room_id,
                display_name,
                "chat bootstrap resumed",
            )?;
            return Ok(AppProfileChatBootstrapPreparation {
                state: self.app.clone(),
                complete: true,
                prepared_commit: None,
            });
        }

        let mut profile = profile;
        if !profile_has_useful_name(&profile, &account_id)
            && let Some(display_name) =
                profile_name_hint_from_chat_label(&display_name, &account_id)
        {
            profile.display_name = display_name;
        }
        self.remember_profile_summary(profile)?;

        self.core.bootstrap_room_from_request(
            &room_create_request,
            Some(display_name),
            fail_after_room_server_acceptance,
        )?;
        self.materialize_known_connected_rooms()?;

        let prepared_commit = if self
            .core
            .device
            .has_pending_commit(&intended_room_id)
            .map_err(client_error)?
        {
            let prepared = persisted_prepared_commit
                .or_else(|| {
                    self.profile_chat_bootstrap_preparations
                        .get(&intended_room_id)
                        .cloned()
                })
                .ok_or_else(|| FiniteChatCoreError::Client {
                    reason: "the intended bootstrap Room has pending MLS state without its durable submit request"
                        .to_owned(),
                })?;
            self.validate_profile_chat_bootstrap_prepared_commit(
                &account_id,
                &intended_room_id,
                &prepared,
            )?;
            prepared
        } else {
            let welcome_id = self.core.generate_object_id("welcome")?;
            let idempotency_key = self.core.generate_object_id("direct-add")?;
            let prepared = self
                .core
                .device
                .prepare_add_member_commit(
                    &intended_room_id,
                    &claimed_key_package,
                    welcome_id,
                    idempotency_key,
                )
                .map_err(client_error)?;
            AppProfileChatBootstrapPreparedCommit {
                request: prepared.request,
                message_id: prepared.message_id,
            }
        };
        self.profile_chat_bootstrap_preparations
            .insert(intended_room_id, prepared_commit.clone());
        Ok(AppProfileChatBootstrapPreparation {
            state: self.app.clone(),
            complete: false,
            prepared_commit: Some(prepared_commit),
        })
    }

    fn validate_profile_chat_bootstrap_room_create_request(
        &self,
        intended_room_id: &str,
        request: &CreateRoomRequest,
    ) -> Result<(), FiniteChatCoreError> {
        request.validate_limits().map_err(client_error)?;
        request.protocol.validate_limits().map_err(client_error)?;
        if request.room_id != intended_room_id
            || request.creator != *self.core.device.device_ref()
            || request.protocol != RoomProtocol::default()
        {
            return Err(FiniteChatCoreError::Client {
                reason: "the durable profile chat Room create request is inconsistent".to_owned(),
            });
        }
        if self.core.has_room(intended_room_id)
            && self
                .core
                .device
                .room_mls_group_id(intended_room_id)
                .map_err(client_error)?
                != request.mls_group_id
        {
            return Err(FiniteChatCoreError::Client {
                reason:
                    "the durable profile chat Room create request does not match local MLS state"
                        .to_owned(),
            });
        }
        Ok(())
    }

    fn submit_profile_chat_bootstrap(
        &mut self,
        agent_account_id: String,
        display_name: String,
        intended_room_id: String,
        prepared_commit: AppProfileChatBootstrapPreparedCommit,
        fail_after_device_save: bool,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_profile_chat_bootstrap_prepared_commit(
            &agent_account_id,
            &intended_room_id,
            &prepared_commit,
        )?;
        // `prepare_profile_chat_bootstrap` already synchronized and validated
        // immediately before this command. Synchronizing again here would
        // reload the not-yet-durable pending commit before we save it.
        let current_room_ids = self.profile_chat_room_ids(&agent_account_id);
        if matches!(current_room_ids.as_slice(), [room_id] if room_id == &intended_room_id) {
            self.profile_chat_bootstrap_preparations
                .remove(&intended_room_id);
            return self.finalize_profile_chat_bootstrap(
                intended_room_id,
                display_name,
                "chat bootstrap resumed",
            );
        }
        if !current_room_ids.is_empty() {
            return Err(FiniteChatCoreError::Client {
                reason: "an existing direct Room prevents first-time profile chat bootstrap"
                    .to_owned(),
            });
        }
        if !self.core.has_room(&intended_room_id)
            || self
                .core
                .device
                .room_members(&intended_room_id)
                .map_err(client_error)?
                .iter()
                .any(|member| member.account_id != self.app.identity.account_id)
        {
            return Err(FiniteChatCoreError::Client {
                reason: "the intended bootstrap Room changed before its durable submit".to_owned(),
            });
        }
        if !self
            .core
            .device
            .has_pending_commit(&intended_room_id)
            .map_err(client_error)?
        {
            return Err(FiniteChatCoreError::Client {
                reason: "the prepared bootstrap commit is not pending; it must be prepared again"
                    .to_owned(),
            });
        }
        if let Some(in_memory) = self
            .profile_chat_bootstrap_preparations
            .get(&intended_room_id)
            && in_memory != &prepared_commit
        {
            return Err(FiniteChatCoreError::Client {
                reason: "the durable bootstrap submit request does not match live MLS state"
                    .to_owned(),
            });
        }
        self.core
            .store
            .save_device_state(&self.core.device)
            .map_err(store_error)?;
        if fail_after_device_save {
            return Err(FiniteChatCoreError::Client {
                reason: "injected profile chat bootstrap failure after saving pending MLS state"
                    .to_owned(),
            });
        }
        let accepted = {
            let mut delivery = self.core.home_delivery();
            delivery
                .submit_commit(prepared_commit.request.clone())
                .map_err(send_delivery_error)?
        };
        if accepted.message_id != prepared_commit.message_id {
            return Err(client_error(format!(
                "commit acceptance message id {} did not match prepared message id {}",
                accepted.message_id, prepared_commit.message_id
            )));
        }
        let synced = self.core.sync_with_projection()?;
        self.apply_projection_events(synced.events)?;
        self.append_messages(synced.result.messages);
        self.materialize_known_connected_rooms()?;
        let room_ids = self.profile_chat_room_ids(&agent_account_id);
        if !matches!(room_ids.as_slice(), [room_id] if room_id == &intended_room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: "the submitted bootstrap did not activate exactly its intended direct Room"
                    .to_owned(),
            });
        }
        self.profile_chat_bootstrap_preparations
            .remove(&intended_room_id);
        self.finalize_profile_chat_bootstrap(intended_room_id, display_name, "chat created")
    }

    /// Synchronize before deciding whether a Project bootstrap may proceed.
    /// Returns true only when the exact intended Room is already the sole
    /// direct Room for the Agent. A self-only intended Room is resumable;
    /// every other existing or uninspectable Room remains fail-closed.
    fn sync_and_validate_profile_chat_bootstrap_room(
        &mut self,
        agent_account_id: &str,
        intended_room_id: &str,
    ) -> Result<bool, FiniteChatCoreError> {
        let synced = self.core.sync_with_projection()?;
        self.apply_projection_events(synced.events)?;
        self.append_messages(synced.result.messages);
        self.materialize_known_connected_rooms()?;
        if self
            .app
            .rooms
            .iter()
            .any(|room| room.state != AppRoomState::Connected || !self.core.has_room(&room.room_id))
        {
            return Err(FiniteChatCoreError::Client {
                reason: "cannot prove profile Room absence while a retained Room is unavailable on this Device"
                    .to_owned(),
            });
        }
        let existing_room_ids = self.profile_chat_room_ids(agent_account_id);
        match existing_room_ids.as_slice() {
            [room_id] if room_id == intended_room_id => return Ok(true),
            [] => {}
            _ => {
                return Err(FiniteChatCoreError::Client {
                    reason: "an existing direct Room prevents first-time profile chat bootstrap"
                        .to_owned(),
                });
            }
        }
        if self.core.has_room(intended_room_id) {
            let members = self
                .core
                .device
                .room_members(intended_room_id)
                .map_err(client_error)?;
            if members
                .iter()
                .any(|member| member.account_id != self.app.identity.account_id)
            {
                return Err(FiniteChatCoreError::Client {
                    reason: "the intended bootstrap Room exists with unexpected members".to_owned(),
                });
            }
        } else if self.room(intended_room_id).is_some() {
            return Err(FiniteChatCoreError::Client {
                reason: "the intended bootstrap Room is retained without inspectable MLS state"
                    .to_owned(),
            });
        }
        Ok(false)
    }

    fn validate_profile_chat_bootstrap_prepared_commit(
        &self,
        agent_account_id: &str,
        intended_room_id: &str,
        prepared: &AppProfileChatBootstrapPreparedCommit,
    ) -> Result<(), FiniteChatCoreError> {
        prepared.request.validate_limits().map_err(client_error)?;
        let own_device = self.core.device.device_ref();
        let request = &prepared.request;
        let message_id = request.envelope.message_id().map_err(client_error)?;
        if request.room_id != intended_room_id
            || request.sender != *own_device
            || request.envelope.room_id != intended_room_id
            || request.envelope.sender != *own_device
            || request.membership_delta.commit_message_id != prepared.message_id
            || message_id != prepared.message_id
            || request.membership_delta.adds.len() != 1
            || request.staged_welcomes.len() != 1
            || request.membership_delta.adds[0].device.account_id != agent_account_id
        {
            return Err(FiniteChatCoreError::Client {
                reason: "the durable profile chat bootstrap submit request is inconsistent"
                    .to_owned(),
            });
        }
        Ok(())
    }

    fn finalize_profile_chat_bootstrap(
        &mut self,
        room_id: String,
        display_name: String,
        status: &str,
    ) -> Result<(), FiniteChatCoreError> {
        self.app.selected_room_id = Some(room_id.clone());
        self.upsert_room(
            &room_id,
            &display_name,
            None,
            AppRoomState::Connected,
            "connected",
        );
        self.ensure_home_topic(&room_id)?;
        if self
            .default_chat_id_for_topic(&room_id, HOME_TOPIC_ID)
            .is_none()
        {
            self.start_topic_chat(room_id.clone(), HOME_TOPIC_ID.to_owned(), None)?;
        }
        self.app.selected_topic_id = Some(HOME_TOPIC_ID.to_owned());
        self.app.selected_chat_id = self.default_chat_id_for_topic(&room_id, HOME_TOPIC_ID);
        self.persist_room_projection(&room_id)?;
        self.persist_app_state()?;
        self.sync_selected_room_messages();
        self.app.status = status.to_owned();
        self.app.toast = None;
        Ok(())
    }

    fn create_profile_chat_room(
        &mut self,
        account_id: String,
        display_name: String,
        room_id: String,
    ) -> Result<(), FiniteChatCoreError> {
        let claimed = {
            let mut delivery = self.core.home_delivery();
            match delivery.claim_key_package_for_account(&account_id) {
                Ok(Some(claimed)) => claimed,
                Ok(None) => {
                    self.set_online_action_unavailable(
                        "chat unavailable",
                        "Ask them to open Finite Chat, then try again",
                    );
                    return Ok(());
                }
                Err(error) => {
                    let error = send_delivery_error(error);
                    if online_action_failure(&error) {
                        self.set_online_action_unavailable(
                            "chat unavailable",
                            "Profile chat could not be created",
                        );
                        return Ok(());
                    }
                    return Err(error);
                }
            }
        };

        self.core
            .bootstrap_room(&room_id, Some(display_name.clone()))?;

        let welcome_id = self.core.generate_object_id("welcome")?;
        let idempotency_key = self.core.generate_object_id("direct-add")?;
        let prepared = self
            .core
            .device
            .prepare_add_member_commit(&room_id, &claimed, welcome_id, idempotency_key)
            .map_err(client_error)?;
        self.core
            .store
            .save_device_state(&self.core.device)
            .map_err(store_error)?;
        let accepted = {
            let mut delivery = self.core.home_delivery();
            match delivery.submit_commit(prepared.request) {
                Ok(accepted) => accepted,
                Err(error) => {
                    let error = send_delivery_error(error);
                    if online_action_failure(&error) {
                        self.set_online_action_unavailable(
                            "chat unavailable",
                            "Profile chat could not be created",
                        );
                        return Ok(());
                    }
                    return Err(error);
                }
            }
        };
        if accepted.message_id != prepared.message_id {
            return Err(client_error(format!(
                "commit acceptance message id {} did not match prepared message id {}",
                accepted.message_id, prepared.message_id
            )));
        }

        let synced = self.core.sync_with_projection()?;
        self.apply_projection_events(synced.events)?;
        self.append_messages(synced.result.messages);
        self.materialize_known_connected_rooms()?;
        self.app.selected_room_id = Some(room_id.clone());
        self.upsert_room(
            &room_id,
            &display_name,
            None,
            AppRoomState::Connected,
            "connected",
        );
        self.ensure_home_topic(&room_id)?;
        if self
            .default_chat_id_for_topic(&room_id, HOME_TOPIC_ID)
            .is_none()
        {
            self.start_topic_chat(room_id.clone(), HOME_TOPIC_ID.to_owned(), None)?;
        }
        self.app.selected_topic_id = Some(HOME_TOPIC_ID.to_owned());
        self.app.selected_chat_id = self.default_chat_id_for_topic(&room_id, HOME_TOPIC_ID);
        self.persist_room_projection(&room_id)?;
        self.persist_app_state()?;
        self.sync_selected_room_messages();
        self.app.status = "chat created".to_owned();
        Ok(())
    }

    fn start_group_chat(
        &mut self,
        profiles: Vec<AppProfileSummary>,
        display_name: String,
    ) -> Result<(), FiniteChatCoreError> {
        let mut seen = BTreeSet::new();
        let mut member_profiles = Vec::new();
        for profile in profiles {
            let profile = normalize_profile_summary_hint(profile)?;
            let account_id = profile.account_id.clone();
            if account_id.is_empty() {
                continue;
            }
            if seen.insert(account_id.clone()) {
                member_profiles.push(profile);
            }
        }
        let member_account_ids: Vec<String> = member_profiles
            .iter()
            .map(|profile| profile.account_id.clone())
            .collect();
        if member_account_ids.is_empty() {
            self.set_online_action_unavailable(
                "chat unavailable",
                "Choose at least one other person to start a chat",
            );
            return Ok(());
        }
        validate_item_count(
            "group.account_ids",
            member_account_ids.len(),
            MAX_STAGED_WELCOMES_PER_COMMIT,
        )
        .map_err(client_error)?;
        for profile in member_profiles {
            self.remember_profile_summary(profile)?;
        }

        let label = display_name.trim();
        if label.len() > MAX_PROFILE_DISPLAY_NAME_BYTES as usize {
            return Err(FiniteChatCoreError::Client {
                reason: format!(
                    "room display name must be at most {MAX_PROFILE_DISPLAY_NAME_BYTES} bytes"
                ),
            });
        }
        let display_name = if label.is_empty() {
            format!("Group with {}", short_account_label(&member_account_ids[0]))
        } else {
            label.to_owned()
        };

        let claimed: Result<Option<Vec<ClaimKeyPackageResult>>, FiniteChatCoreError> = {
            let mut delivery = self.core.home_delivery();
            let mut claimed = Vec::with_capacity(member_account_ids.len());
            let mut missing_package = false;
            for account_id in &member_account_ids {
                match delivery.claim_key_package_for_account(account_id) {
                    Ok(Some(package)) => claimed.push(package),
                    Ok(None) => {
                        missing_package = true;
                        break;
                    }
                    Err(error) => return Err(send_delivery_error(error)),
                }
            }
            if missing_package {
                Ok(None)
            } else {
                Ok(Some(claimed))
            }
        };
        let claimed = match claimed {
            Ok(Some(claimed)) => claimed,
            Ok(None) => {
                self.set_online_action_unavailable(
                    "chat unavailable",
                    "Ask everyone to open Finite Chat, then try again",
                );
                return Ok(());
            }
            Err(error) => {
                if online_action_failure(&error) {
                    self.set_online_action_unavailable(
                        "chat unavailable",
                        "Group chat could not be created",
                    );
                    return Ok(());
                }
                return Err(error);
            }
        };

        let room_id = self.core.generate_object_id("room")?;
        self.core
            .bootstrap_room(&room_id, Some(display_name.clone()))?;

        let mut welcome_ids = Vec::with_capacity(claimed.len());
        for _ in &claimed {
            welcome_ids.push(self.core.generate_object_id("welcome")?);
        }
        let idempotency_key = self.core.generate_object_id("group-add")?;
        let prepared = self
            .core
            .device
            .prepare_add_members_commit(&room_id, &claimed, &welcome_ids, idempotency_key)
            .map_err(client_error)?;
        if !self.submit_prepared_members_commit(
            prepared,
            "chat unavailable",
            "Group chat could not be created",
        )? {
            return Ok(());
        }

        let synced = self.core.sync_with_projection()?;
        self.apply_projection_events(synced.events)?;
        self.append_messages(synced.result.messages);
        self.materialize_known_connected_rooms()?;
        self.app.selected_room_id = Some(room_id.clone());
        self.upsert_room(
            &room_id,
            &display_name,
            None,
            AppRoomState::Connected,
            "connected",
        );
        self.ensure_home_topic(&room_id)?;
        if self
            .default_chat_id_for_topic(&room_id, HOME_TOPIC_ID)
            .is_none()
        {
            self.start_topic_chat(room_id.clone(), HOME_TOPIC_ID.to_owned(), None)?;
        }
        self.app.selected_topic_id = Some(HOME_TOPIC_ID.to_owned());
        self.app.selected_chat_id = self.default_chat_id_for_topic(&room_id, HOME_TOPIC_ID);
        self.persist_room_projection(&room_id)?;
        self.persist_app_state()?;
        self.sync_selected_room_messages();
        self.app.status = "chat created".to_owned();
        Ok(())
    }

    fn add_room_members(
        &mut self,
        room_id: String,
        profiles: Vec<AppProfileSummary>,
    ) -> Result<(), FiniteChatCoreError> {
        let room_id = room_id.trim().to_owned();
        validate_string_bytes("room_id", &room_id, MAX_OBJECT_ID_BYTES).map_err(client_error)?;
        if !self.room_is_connected(&room_id) {
            self.set_online_action_unavailable(
                "chat unavailable",
                "People could not be added to this chat",
            );
            return Ok(());
        }

        let mut seen = BTreeSet::new();
        let mut member_profiles = Vec::new();
        for profile in profiles {
            let profile = normalize_profile_summary_hint(profile)?;
            let account_id = profile.account_id.clone();
            if account_id.is_empty() {
                continue;
            }
            if seen.insert(account_id.clone()) {
                member_profiles.push(profile);
            }
        }
        let member_account_ids: Vec<String> = member_profiles
            .iter()
            .map(|profile| profile.account_id.clone())
            .collect();
        if member_account_ids.is_empty() {
            self.set_online_action_unavailable(
                "chat unavailable",
                "Choose at least one person to add",
            );
            return Ok(());
        }
        validate_item_count(
            "room_member.account_ids",
            member_account_ids.len(),
            MAX_STAGED_WELCOMES_PER_COMMIT,
        )
        .map_err(client_error)?;
        for profile in member_profiles {
            self.remember_profile_summary(profile)?;
        }

        let claimed: Result<Option<Vec<ClaimKeyPackageResult>>, FiniteChatCoreError> = {
            let mut delivery = self.core.home_delivery();
            let mut claimed = Vec::with_capacity(member_account_ids.len());
            let mut missing_package = false;
            let existing_members = self
                .core
                .device
                .room_members(&room_id)
                .map_err(client_error)?
                .into_iter()
                .collect::<BTreeSet<_>>();
            for account_id in &member_account_ids {
                let mut accepted_package = None;
                for _ in 0..MAX_KEY_PACKAGES_PER_DEVICE {
                    match delivery.claim_key_package_for_account(account_id) {
                        Ok(Some(package)) if existing_members.contains(&package.owner) => {
                            continue;
                        }
                        Ok(Some(package)) => {
                            accepted_package = Some(package);
                            break;
                        }
                        Ok(None) => {
                            break;
                        }
                        Err(error) => return Err(send_delivery_error(error)),
                    }
                }
                if let Some(package) = accepted_package {
                    claimed.push(package);
                } else {
                    missing_package = true;
                    break;
                }
            }
            if missing_package {
                Ok(None)
            } else {
                Ok(Some(claimed))
            }
        };
        let claimed = match claimed {
            Ok(Some(claimed)) => claimed,
            Ok(None) => {
                self.set_online_action_unavailable(
                    "chat unavailable",
                    "Ask everyone to open Finite Chat, then try again",
                );
                return Ok(());
            }
            Err(error) => {
                if online_action_failure(&error) {
                    self.set_online_action_unavailable(
                        "chat unavailable",
                        "People could not be added to this chat",
                    );
                    return Ok(());
                }
                return Err(error);
            }
        };

        let mut welcome_ids = Vec::with_capacity(claimed.len());
        for _ in &claimed {
            welcome_ids.push(self.core.generate_object_id("welcome")?);
        }
        let idempotency_key = self.core.generate_object_id("room-add")?;
        let prepared = self
            .core
            .device
            .prepare_add_members_commit(&room_id, &claimed, &welcome_ids, idempotency_key)
            .map_err(client_error)?;
        if !self.submit_prepared_members_commit(
            prepared,
            "chat unavailable",
            "People could not be added to this chat",
        )? {
            return Ok(());
        }

        let synced = self.core.sync_with_projection()?;
        self.apply_projection_events(synced.events)?;
        self.append_messages(synced.result.messages);
        self.materialize_known_connected_rooms()?;
        self.app.selected_room_id = Some(room_id.clone());
        if let Some(room) = self.room(&room_id).cloned() {
            self.upsert_room(
                &room_id,
                &room.display_name,
                None,
                AppRoomState::Connected,
                "connected",
            );
        }
        self.persist_room_projection(&room_id)?;
        self.persist_app_state()?;
        self.sync_selected_room_messages();
        self.app.status = "people added".to_owned();
        Ok(())
    }

    fn submit_prepared_members_commit(
        &mut self,
        prepared: PreparedCommit,
        unavailable_status: &str,
        unavailable_toast: &str,
    ) -> Result<bool, FiniteChatCoreError> {
        let PreparedCommit {
            request,
            message_id,
        } = prepared;
        self.core
            .store
            .save_device_state(&self.core.device)
            .map_err(store_error)?;
        let accepted = {
            let mut delivery = self.core.home_delivery();
            match delivery.submit_commit(request) {
                Ok(accepted) => accepted,
                Err(error) => {
                    let error = send_delivery_error(error);
                    if online_action_failure(&error) {
                        self.set_online_action_unavailable(unavailable_status, unavailable_toast);
                        return Ok(false);
                    }
                    return Err(error);
                }
            }
        };
        if accepted.message_id != message_id {
            return Err(client_error(format!(
                "commit acceptance message id {} did not match prepared message id {}",
                accepted.message_id, message_id
            )));
        }
        Ok(true)
    }

    fn set_online_action_unavailable(&mut self, status: &str, toast: &str) {
        self.app.status = status.to_owned();
        self.app.toast = Some(toast.to_owned());
    }

    fn scan_target(&mut self, value: String) -> Result<(), FiniteChatCoreError> {
        let trimmed = value.trim();
        if let Some(account_id) = explicit_profile_account_id(trimmed) {
            return self.scan_profile_account_id(account_id);
        }
        if let Some(account_id) = embedded_profile_account_id(trimmed) {
            return self.scan_profile_account_id(account_id);
        }
        Err(FiniteChatCoreError::Client {
            reason: "unsupported scan target; paste an npub or profile code".to_owned(),
        })
    }

    fn scan_profile_account_id(&mut self, account_id: String) -> Result<(), FiniteChatCoreError> {
        let found = match self.fetch_profiles(vec![account_id.clone()]) {
            Ok(found) => found,
            Err(error) => {
                if !online_action_failure(&error) {
                    return Err(error);
                }
                self.remember_placeholder_profile(&account_id)?;
                self.app.active_profile_id = Some(account_id);
                self.set_online_action_unavailable(
                    "profile details unavailable",
                    "Profile details unavailable; you can still start a chat",
                );
                self.finish_scan_target(
                    AppScanTargetOutcome::Profile,
                    Some("Profile opened.".to_owned()),
                );
                return Ok(());
            }
        };
        self.app.active_profile_id = Some(account_id.clone());
        if found {
            self.app.status = "profile loaded".to_owned();
            self.app.toast = None;
        } else {
            self.remember_placeholder_profile(&account_id)?;
            self.app.status = "profile not found".to_owned();
            self.app.toast = Some("No cached profile was available for that npub".to_owned());
        }
        self.finish_scan_target(
            AppScanTargetOutcome::Profile,
            Some("Profile opened.".to_owned()),
        );
        Ok(())
    }

    fn send_message(
        &mut self,
        room_id: String,
        text: String,
        metadata_json: Option<&str>,
        requester_context: Option<&VerifiedRequesterContext>,
    ) -> Result<(), FiniteChatCoreError> {
        self.send_message_with_reply(
            room_id,
            OutboundChatText {
                text,
                reply_to_message_id: None,
                metadata_json: metadata_json.map(str::to_owned),
            },
            requester_context,
        )
    }

    fn send_topic_message(
        &mut self,
        room_id: String,
        topic_id: String,
        text: String,
        metadata_json: Option<&str>,
        requester_context: Option<&VerifiedRequesterContext>,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_topic(&room_id, &topic_id)?;
        let chat_id = match self.default_chat_id_for_topic(&room_id, &topic_id) {
            Some(chat_id) => chat_id,
            None => {
                self.start_topic_chat(room_id.clone(), topic_id.clone(), None)?;
                self.default_chat_id_for_topic(&room_id, &topic_id)
                    .ok_or_else(|| FiniteChatCoreError::Client {
                        reason: format!(
                            "topic '{topic_id}' in room '{room_id}' has no available chat"
                        ),
                    })?
            }
        };
        self.send_chat_message(
            room_id,
            topic_id,
            chat_id,
            OutboundChatText {
                text,
                reply_to_message_id: None,
                metadata_json: metadata_json.map(str::to_owned),
            },
            requester_context,
        )
    }

    fn send_chat_message(
        &mut self,
        room_id: String,
        topic_id: String,
        chat_id: String,
        outbound: OutboundChatText,
        requester_context: Option<&VerifiedRequesterContext>,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_chat_route(&room_id, &topic_id, &chat_id)?;
        self.send_message_with_conversation_and_chat(
            room_id,
            Some(topic_id),
            Some(chat_id),
            outbound,
            requester_context,
        )
    }

    fn send_reply(
        &mut self,
        room_id: String,
        text: String,
        reply_to_message_id: String,
        metadata_json: Option<&str>,
        requester_context: Option<&VerifiedRequesterContext>,
    ) -> Result<(), FiniteChatCoreError> {
        let target_id = reply_to_message_id.trim();
        self.validate_reply_target(&room_id, target_id)?;
        self.send_message_with_reply(
            room_id,
            OutboundChatText {
                text,
                reply_to_message_id: Some(target_id.to_owned()),
                metadata_json: metadata_json.map(str::to_owned),
            },
            requester_context,
        )
    }

    fn send_chat_reply(
        &mut self,
        room_id: String,
        topic_id: String,
        chat_id: String,
        outbound: OutboundChatText,
        requester_context: Option<&VerifiedRequesterContext>,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_chat_route(&room_id, &topic_id, &chat_id)?;
        let target_id = outbound
            .reply_to_message_id
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();
        self.validate_reply_target(&room_id, &target_id)?;
        self.send_message_with_conversation_and_chat(
            room_id,
            Some(topic_id),
            Some(chat_id),
            OutboundChatText {
                reply_to_message_id: Some(target_id),
                ..outbound
            },
            requester_context,
        )
    }

    fn send_message_with_reply(
        &mut self,
        room_id: String,
        outbound: OutboundChatText,
        requester_context: Option<&VerifiedRequesterContext>,
    ) -> Result<(), FiniteChatCoreError> {
        let trimmed = outbound.text.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to send"),
            });
        }

        let reply_to_message_id =
            self.normalize_reply_target(&room_id, outbound.reply_to_message_id)?;
        let (topic_id, chat_id) = self.default_chat_route_for_room(&room_id)?;
        self.send_message_with_conversation_and_chat(
            room_id,
            Some(topic_id),
            Some(chat_id),
            OutboundChatText {
                text: trimmed.to_owned(),
                reply_to_message_id,
                ..outbound
            },
            requester_context,
        )
    }

    fn send_message_with_conversation_and_chat(
        &mut self,
        room_id: String,
        conversation_id: Option<String>,
        chat_id: Option<String>,
        outbound: OutboundChatText,
        requester_context: Option<&VerifiedRequesterContext>,
    ) -> Result<(), FiniteChatCoreError> {
        let trimmed = outbound.text.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to send"),
            });
        }

        let chat_payload = encode_text_message_payload_scoped(
            trimmed,
            outbound.reply_to_message_id.as_deref(),
            conversation_id.as_deref(),
            chat_id.as_deref(),
            outbound.metadata_json.as_deref(),
            requester_context,
        )?;
        let app_event_plaintext = encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            conversation_id.clone(),
            chat_id.clone(),
            &chat_payload,
        )?;
        self.send_chat_message_with_local_outbox(
            room_id,
            app_event_plaintext,
            trimmed.to_owned(),
            "sent",
        )?;
        Ok(())
    }

    fn send_encoded_chat_message(
        &mut self,
        room_id: String,
        app_event_plaintext: Vec<u8>,
        preview: String,
    ) -> Result<AppSentMessage, FiniteChatCoreError> {
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to send"),
            });
        }
        let (app_event_plaintext, selected_topic_id, selected_chat_id) =
            self.scope_unscoped_chat_event_to_default(&room_id, app_event_plaintext)?;
        let sent = self.send_chat_message_with_local_outbox_result(
            room_id.clone(),
            app_event_plaintext,
            preview,
            "sent",
        )?;
        if selected_topic_id.is_some() || selected_chat_id.is_some() {
            self.app.selected_room_id = Some(room_id);
            self.app.selected_topic_id = selected_topic_id;
            self.app.selected_chat_id = selected_chat_id;
            self.sync_selected_room_messages();
        }
        Ok(sent)
    }

    fn send_runtime_command_request(
        &mut self,
        room_id: String,
        conversation_id: Option<String>,
        kind: DurableAppEventKind,
        payload: Vec<u8>,
    ) -> Result<String, FiniteChatCoreError> {
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to send"),
            });
        }
        match kind {
            DurableAppEventKind::RuntimeCommandRequest => {
                let request = serde_json::from_slice::<RuntimeCommandRequestV1>(&payload)
                    .map_err(client_error)?;
                request.validate_structure().map_err(client_error)?;
            }
            DurableAppEventKind::RuntimeCommandResult => {
                let result = serde_json::from_slice::<RuntimeCommandResultV1>(&payload)
                    .map_err(client_error)?;
                result.validate_structure().map_err(client_error)?;
            }
            DurableAppEventKind::RuntimeStateSnapshot => {
                let snapshot = serde_json::from_slice::<RuntimeStateSnapshotV1>(&payload)
                    .map_err(client_error)?;
                snapshot.validate_limits().map_err(client_error)?;
            }
            _ => {
                return Err(FiniteChatCoreError::Client {
                    reason: "unsupported runtime event kind".to_owned(),
                });
            }
        }
        let event = self.core.send_application_event(
            &room_id,
            kind,
            conversation_id,
            &payload,
            "runtime-command",
        )?;
        Ok(event.message_id)
    }

    fn upload_bridge_attachment(
        &mut self,
        room_id: String,
        attachment: OutboundAttachment,
    ) -> Result<HermesAttachmentV1, FiniteChatCoreError> {
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to upload attachments"),
            });
        }
        self.core.upload_outbound_attachment(&room_id, attachment)
    }

    fn send_chat_message_with_local_outbox(
        &mut self,
        room_id: String,
        app_event_plaintext: Vec<u8>,
        preview: String,
        sent_status: &str,
    ) -> Result<(), FiniteChatCoreError> {
        self.send_chat_message_with_local_outbox_result(
            room_id,
            app_event_plaintext,
            preview,
            sent_status,
        )
        .map(|_| ())
    }

    fn send_chat_message_with_local_outbox_result(
        &mut self,
        room_id: String,
        app_event_plaintext: Vec<u8>,
        preview: String,
        sent_status: &str,
    ) -> Result<AppSentMessage, FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        let prepared = self
            .core
            .prepare_outbound_chat_message(&room_id, app_event_plaintext)?;
        let local_message_id = prepared.chat_message.message_id.clone();
        self.persist_outbox_message(&prepared.stored_message)?;
        self.chat_projection
            .append_messages(vec![prepared.chat_message.clone()], &owner);
        self.sync_chat_projection();

        let sent = match self
            .core
            .submit_outbox_message_with_acceptance(&prepared.stored_message)
        {
            Ok((accepted, projection)) => {
                self.remove_outbox_message(&room_id, &local_message_id)?;
                self.apply_projection_events(projection.events)?;
                self.append_messages(projection.result.messages);
                if let Some(room) = self.room_mut(&room_id) {
                    room.last_message_preview = preview;
                }
                self.app.status = sent_status.to_owned();
                AppSentMessage {
                    message_id: accepted.message_id,
                    seq: Some(accepted.seq),
                }
            }
            Err(FiniteChatCoreError::Delivery { .. }) => {
                self.app.status = sent_status.to_owned();
                AppSentMessage {
                    message_id: local_message_id,
                    seq: None,
                }
            }
            Err(FiniteChatCoreError::ServerRejected { reason }) => {
                let failed = failed_outbox_message(prepared.stored_message, reason);
                self.persist_outbox_message(&failed)?;
                if let Some(message) = chat_message_from_outbox(failed, &owner) {
                    self.chat_projection.append_messages(vec![message], &owner);
                    self.sync_chat_projection();
                }
                self.app.status = "delivery failed".to_owned();
                self.app.toast = Some("Message delivery failed. Retry when ready.".to_owned());
                AppSentMessage {
                    message_id: local_message_id,
                    seq: None,
                }
            }
            Err(error) => {
                return Err(error);
            }
        };
        Ok(sent)
    }

    fn send_chat_attachment(
        &mut self,
        input: SendAttachmentInput,
    ) -> Result<(), FiniteChatCoreError> {
        let topic_id =
            input
                .conversation_id
                .clone()
                .ok_or_else(|| FiniteChatCoreError::Client {
                    reason: "chat attachment must include a topic id".to_owned(),
                })?;
        let chat_id = input
            .chat_id
            .clone()
            .ok_or_else(|| FiniteChatCoreError::Client {
                reason: "chat attachment must include a chat id".to_owned(),
            })?;
        self.validate_chat_route(&input.room_id, &topic_id, &chat_id)?;
        self.send_attachment(input)
    }

    fn append_ephemeral_activity(
        &mut self,
        input: AppBridgeActivityInput,
    ) -> Result<AppAcceptedActivity, FiniteChatCoreError> {
        if !self.room_is_connected(&input.room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{}' is not ready to send activity", input.room_id),
            });
        }
        let accepted = self.core.append_ephemeral_activity(input)?;
        self.app.status = "activity sent".to_owned();
        Ok(AppAcceptedActivity {
            route_key: accepted.route_key,
            cached_events_for_route: accepted.cached_events_for_route,
        })
    }

    fn send_attachment(
        &mut self,
        mut input: SendAttachmentInput,
    ) -> Result<(), FiniteChatCoreError> {
        let room_id = input.room_id.clone();
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to send"),
            });
        }
        match (input.conversation_id.clone(), input.chat_id.clone()) {
            (Some(topic_id), Some(chat_id)) => {
                self.validate_chat_route(&room_id, &topic_id, &chat_id)?;
            }
            (None, None) => {
                let (topic_id, chat_id) = self.default_chat_route_for_room(&room_id)?;
                input.conversation_id = Some(topic_id);
                input.chat_id = Some(chat_id);
            }
            _ => {
                return Err(FiniteChatCoreError::Client {
                    reason: "attachment route must include both topic and chat ids".to_owned(),
                });
            }
        }
        if input.attachments.is_empty() {
            return Err(FiniteChatCoreError::Client {
                reason: "attachment message must include at least one attachment".to_owned(),
            });
        }
        validate_item_count(
            "attachments",
            input.attachments.len(),
            MAX_ATTACHMENTS_PER_MESSAGE,
        )
        .map_err(client_error)?;
        input.reply_to_message_id =
            self.normalize_reply_target(&room_id, input.reply_to_message_id)?;

        match self.core.send_attachment(input) {
            Ok(projection) => {
                self.apply_projection_events(projection.events)?;
                self.append_messages(projection.result.messages);
                self.app.status = "sent".to_owned();
            }
            Err(error) => {
                self.app.status = "attachment unavailable".to_owned();
                self.app.toast = Some(format!(
                    "Attachment upload failed: {}",
                    compact_error_reason(&error)
                ));
            }
        }
        Ok(())
    }

    fn send_poll(
        &mut self,
        room_id: String,
        question: String,
        options: Vec<String>,
    ) -> Result<(), FiniteChatCoreError> {
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to send"),
            });
        }
        let (topic_id, chat_id) = self.default_chat_route_for_room(&room_id)?;
        let chat_payload = encode_poll_message_payload(&question, options)?;
        // The poll payload was just encoded from `question`; projecting it
        // again would only round-trip that value through a JSON parse.
        let preview = question.clone();
        let app_event_plaintext = encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            Some(topic_id.clone()),
            Some(chat_id.clone()),
            &chat_payload,
        )?;
        self.send_chat_message_with_local_outbox(room_id, app_event_plaintext, preview, "sent")?;
        self.app.selected_topic_id = Some(topic_id);
        self.app.selected_chat_id = Some(chat_id);
        self.sync_selected_room_messages();
        Ok(())
    }

    fn send_chat_poll(
        &mut self,
        room_id: String,
        topic_id: String,
        chat_id: String,
        question: String,
        options: Vec<String>,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_chat_route(&room_id, &topic_id, &chat_id)?;
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to send"),
            });
        }
        let chat_payload = encode_poll_message_payload(&question, options)?;
        // The poll payload was just encoded from `question`; projecting it
        // again would only round-trip that value through a JSON parse.
        let preview = question.clone();
        let app_event_plaintext = encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            Some(topic_id.clone()),
            Some(chat_id.clone()),
            &chat_payload,
        )?;
        self.send_chat_message_with_local_outbox(room_id, app_event_plaintext, preview, "sent")?;
        self.app.selected_topic_id = Some(topic_id);
        self.app.selected_chat_id = Some(chat_id);
        self.sync_selected_room_messages();
        Ok(())
    }

    fn vote_poll(
        &mut self,
        room_id: String,
        message_id: String,
        option_id: String,
    ) -> Result<(), FiniteChatCoreError> {
        let option_id = option_id.trim().to_owned();
        if option_id.is_empty() {
            return Ok(());
        }
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to vote"),
            });
        }
        let Some(message) = self.chat_projection.message(&room_id, &message_id) else {
            return Err(FiniteChatCoreError::Client {
                reason: format!("message '{message_id}' is not available in room '{room_id}'"),
            });
        };
        let Some(poll) = &message.poll else {
            return Err(FiniteChatCoreError::Client {
                reason: format!("message '{message_id}' is not a poll"),
            });
        };
        if poll.my_vote_option_id.as_deref() == Some(option_id.as_str()) {
            return Ok(());
        }
        if !poll
            .options
            .iter()
            .any(|option| option.option_id == option_id)
        {
            return Err(FiniteChatCoreError::Client {
                reason: format!("poll option '{option_id}' is not available"),
            });
        }

        let event = self
            .core
            .send_poll_vote(&room_id, &message_id, &option_id)?;
        self.apply_projection_events(vec![event])?;
        self.app.status = "voted".to_owned();
        Ok(())
    }

    fn begin_download_attachment(
        &mut self,
        room_id: String,
        message_id: String,
        attachment_id: String,
    ) -> Result<(), FiniteChatCoreError> {
        self.download_attachment_reference(&room_id, &message_id, &attachment_id)?;
        let key = attachment_download_key(&room_id, &message_id, &attachment_id);
        self.downloading_attachments.insert(key);
        self.sync_selected_room_messages();
        self.app.status = "downloading attachment".to_owned();
        Ok(())
    }

    fn download_attachment(
        &mut self,
        room_id: String,
        message_id: String,
        attachment_id: String,
    ) -> Result<(), FiniteChatCoreError> {
        let reference =
            self.download_attachment_reference(&room_id, &message_id, &attachment_id)?;
        let key = attachment_download_key(&room_id, &message_id, &attachment_id);
        self.downloading_attachments.insert(key.clone());
        self.sync_selected_room_messages();
        let downloaded = self.core.download_attachment_blob(&reference);
        self.downloading_attachments.remove(&key);
        let path = match downloaded {
            Ok(path) => path,
            Err(error) => {
                self.sync_selected_room_messages();
                self.app.status = "download failed".to_owned();
                self.app.toast = Some(format!(
                    "Attachment download failed: {}",
                    compact_error_reason(&error)
                ));
                return Ok(());
            }
        };
        self.sync_chat_projection();
        let filename = reference.metadata.filename.trim();
        let display_name = if filename.is_empty() {
            "attachment"
        } else {
            filename
        };
        self.app.status = format!("downloaded {display_name}");
        debug_assert!(path.is_file());
        Ok(())
    }

    fn download_attachment_reference(
        &self,
        room_id: &str,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<AttachmentBlobReferenceV1, FiniteChatCoreError> {
        if !self.room_is_connected(room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to download attachments"),
            });
        }
        let Some(message) = self.chat_projection.message(room_id, message_id) else {
            return Err(FiniteChatCoreError::Client {
                reason: format!("message '{message_id}' is not available in room '{room_id}'"),
            });
        };
        attachment_reference_for_id(message, attachment_id).ok_or_else(|| {
            FiniteChatCoreError::Client {
                reason: format!(
                    "attachment '{attachment_id}' is not available on message '{message_id}'"
                ),
            }
        })
    }

    fn load_older_messages(
        &mut self,
        room_id: String,
        before_message_id: String,
        limit: u32,
    ) -> Result<(), FiniteChatCoreError> {
        if self.room(&room_id).is_none() {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not available"),
            });
        }

        if self.app.selected_room_id.as_deref() != Some(room_id.as_str()) {
            self.app.selected_room_id = Some(room_id.clone());
            self.loaded_message_counts
                .entry(room_id.clone())
                .or_insert(DEFAULT_TRANSCRIPT_WINDOW);
            self.persist_app_state()?;
            self.sync_selected_room_messages();
            return Ok(());
        }

        if let Some(oldest) = self.app.messages.first()
            && oldest.message_id != before_message_id
        {
            self.sync_selected_room_messages();
            return Ok(());
        }

        let page_size = normalized_transcript_page_size(limit);
        let current_count = self.loaded_message_count(&room_id);
        let total_count = if let (Some(topic_id), Some(chat_id)) = (
            self.app.selected_topic_id.as_deref(),
            self.app.selected_chat_id.as_deref(),
        ) {
            self.chat_projection
                .chat_message_count(&room_id, topic_id, chat_id)
        } else {
            self.app
                .selected_topic_id
                .as_deref()
                .map(|topic_id| self.chat_projection.topic_message_count(&room_id, topic_id))
                .unwrap_or_else(|| self.chat_projection.room_message_count(&room_id))
        };
        let next_count = current_count
            .saturating_add(page_size)
            .min(total_count)
            .min(MAX_APP_MESSAGES);
        self.loaded_message_counts.insert(
            room_id.clone(),
            next_count.max(DEFAULT_TRANSCRIPT_WINDOW.min(total_count)),
        );
        self.sync_selected_room_messages();
        self.app.status = "loaded older messages".to_owned();
        Ok(())
    }

    fn react_to_message(
        &mut self,
        room_id: String,
        message_id: String,
        emoji: String,
    ) -> Result<(), FiniteChatCoreError> {
        let emoji = emoji.trim().to_owned();
        if emoji.is_empty() {
            return Ok(());
        }
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to react"),
            });
        }
        let Some(message) = self
            .app
            .messages
            .iter()
            .find(|message| message.room_id == room_id && message.message_id == message_id)
        else {
            return Err(FiniteChatCoreError::Client {
                reason: format!("message '{message_id}' is not available in room '{room_id}'"),
            });
        };
        if message
            .reactions
            .iter()
            .any(|reaction| reaction.emoji == emoji && reaction.reacted_by_me)
        {
            return Ok(());
        }

        let projection = self.core.send_reaction(&room_id, &message_id, &emoji)?;
        self.apply_projection_events(projection.events)?;
        self.append_messages(projection.result.messages);
        self.app.status = "reacted".to_owned();
        Ok(())
    }

    fn mark_room_read(&mut self, room_id: String) -> Result<(), FiniteChatCoreError> {
        if self.room(&room_id).is_none() {
            return Ok(());
        }
        if let Some((_, seq)) = self.chat_projection.latest_peer_message(&room_id) {
            let current = self
                .local_read_seq
                .get(&room_id)
                .copied()
                .unwrap_or_default();
            if seq > current {
                self.local_read_seq.insert(room_id.clone(), seq);
                self.persist_room_projection(&room_id)?;
                self.sync_chat_projection();
            }
        }
        if !self.room_is_connected(&room_id) {
            return Ok(());
        }
        let owner = self.core.device.device_ref().clone();
        let Some((message_id, seq)) = self
            .chat_projection
            .latest_peer_message_needing_read_receipt(&room_id, &owner)
        else {
            return Ok(());
        };

        match self
            .core
            .send_read_receipt(&room_id, &message_id, seq, ChatReceiptStateV1::Read)
        {
            Ok(event) => self.apply_projection_events(vec![event])?,
            Err(FiniteChatCoreError::Delivery { .. }) => {}
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn set_typing(&mut self, room_id: String, is_typing: bool) -> Result<(), FiniteChatCoreError> {
        if !self.room_is_connected(&room_id) {
            return Ok(());
        }
        let now_ms = self.core.now_millis()?;
        let refresh_floor = now_ms.saturating_add(TYPING_REFRESH_MIN_MILLIS);

        if is_typing {
            if self
                .local_typing_leases
                .get(&room_id)
                .copied()
                .is_some_and(|expires_at_ms| expires_at_ms > refresh_floor)
            {
                return Ok(());
            }
            match self.core.send_typing_activity(&room_id, true, now_ms) {
                Ok(()) => {
                    self.local_typing_leases.insert(
                        room_id,
                        now_ms.saturating_add(
                            GenericActivityKindV1::Typing.recommended_expiry_millis(),
                        ),
                    );
                }
                Err(FiniteChatCoreError::Delivery { .. }) => {}
                Err(error) => return Err(error),
            }
            return Ok(());
        }

        if self.local_typing_leases.remove(&room_id).is_none() {
            return Ok(());
        }
        match self.core.send_typing_activity(&room_id, false, now_ms) {
            Ok(()) | Err(FiniteChatCoreError::Delivery { .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn retry_message(
        &mut self,
        room_id: String,
        message_id: String,
    ) -> Result<(), FiniteChatCoreError> {
        if !self.room_is_connected(&room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to retry sends"),
            });
        }
        let Some(message) = self.chat_projection.message(&room_id, &message_id).cloned() else {
            self.app.status = "message unavailable".to_owned();
            return Ok(());
        };
        let Some(outbound) = &message.outbound_delivery else {
            self.app.status = "message unavailable".to_owned();
            return Ok(());
        };
        match &outbound.server_delivery {
            OutboundServerDeliveryState::Failed { .. } => {}
            OutboundServerDeliveryState::Undelivered => {
                self.app.status = "sent".to_owned();
                return Ok(());
            }
            OutboundServerDeliveryState::Delivered => {
                self.app.status = "sent".to_owned();
                return Ok(());
            }
        }

        let owner = self.core.device.device_ref().clone();
        let Some(mut retrying) = self.load_outbox_message(&room_id, &message_id)? else {
            self.app.status = "message unavailable".to_owned();
            return Ok(());
        };
        retrying.local_state = StoredOutboundLocalState::Sending;
        retrying.server_delivery_state = StoredOutboundServerDeliveryState::Undelivered;
        self.persist_outbox_message(&retrying)?;
        self.chat_projection.append_messages(
            vec![
                chat_message_from_outbox(retrying.clone(), &owner).ok_or_else(|| {
                    FiniteChatCoreError::Client {
                        reason: "outbox retry row did not project as a transcript row".to_owned(),
                    }
                })?,
            ],
            &owner,
        );
        self.sync_chat_projection();

        retrying.local_state = StoredOutboundLocalState::Sent;
        match self.core.submit_outbox_message(&retrying) {
            Ok(projection) => {
                self.remove_outbox_message(&room_id, &message_id)?;
                self.apply_projection_events(projection.events)?;
                self.append_messages(projection.result.messages);
                self.app.status = "sent".to_owned();
            }
            Err(FiniteChatCoreError::Delivery { .. }) => {
                self.persist_outbox_message(&retrying)?;
                if let Some(message) = chat_message_from_outbox(retrying, &owner) {
                    self.chat_projection.append_messages(vec![message], &owner);
                    self.sync_chat_projection();
                }
                self.app.status = "sent".to_owned();
            }
            Err(FiniteChatCoreError::ServerRejected { reason }) => {
                let failed = failed_outbox_message(retrying, reason);
                self.persist_outbox_message(&failed)?;
                if let Some(message) = chat_message_from_outbox(failed, &owner) {
                    self.chat_projection.append_messages(vec![message], &owner);
                    self.sync_chat_projection();
                }
                self.app.status = "delivery failed".to_owned();
                self.app.toast = Some("Message delivery failed. Retry when ready.".to_owned());
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn drain_undelivered_outbox(&mut self, limit: usize) -> Result<(), FiniteChatCoreError> {
        if limit == 0 {
            return Ok(());
        }
        let owner = self.core.device.device_ref().clone();
        let messages = self
            .core
            .store
            .load_app_outbox(&owner)
            .map_err(store_error)?;
        let mut drained = 0usize;
        for message in messages {
            if drained >= limit {
                break;
            }
            if message.local_state != StoredOutboundLocalState::Sent {
                continue;
            }
            if message.server_delivery_state != StoredOutboundServerDeliveryState::Undelivered {
                continue;
            }
            if !self.room_is_connected(&message.room_id) {
                continue;
            }
            drained = drained.saturating_add(1);
            match self.core.submit_outbox_message(&message) {
                Ok(projection) => {
                    self.remove_outbox_message(&message.room_id, &message.message_id)?;
                    self.apply_projection_events(projection.events)?;
                    self.append_messages(projection.result.messages);
                }
                Err(FiniteChatCoreError::Delivery { .. }) => {
                    break;
                }
                Err(FiniteChatCoreError::ServerRejected { reason }) => {
                    let failed = failed_outbox_message(message, reason);
                    self.persist_outbox_message(&failed)?;
                    if let Some(projected) = chat_message_from_outbox(failed, &owner) {
                        self.chat_projection
                            .append_messages(vec![projected], &owner);
                        self.sync_chat_projection();
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn fetch_profiles(&mut self, account_ids: Vec<String>) -> Result<bool, FiniteChatCoreError> {
        let now_ms = self.core.now_millis()?;
        let mut delivery = self.core.home_delivery();
        let response = match delivery.get_nostr_profiles(account_ids.clone(), now_ms) {
            Ok(response) => response,
            Err(error) => {
                let found = account_ids
                    .iter()
                    .any(|account_id| self.profile_cache.contains_key(account_id));
                self.sync_profile_state();
                if found {
                    return Ok(true);
                }
                return Err(runtime_error(error));
            }
        };
        let mut found = false;
        let mut stored = Vec::new();
        for entry in response.profiles {
            found = true;
            stored.push(StoredAppProfile {
                profile: entry.profile.clone(),
                stale: entry.stale,
            });
            self.profile_records
                .insert(entry.profile.account_id.clone(), entry.profile.clone());
            self.profile_cache.insert(
                entry.profile.account_id.clone(),
                profile_from_record(entry.profile, entry.stale),
            );
        }
        if !stored.is_empty() {
            let owner = self.core.device.device_ref().clone();
            self.core
                .store
                .save_app_profiles(&owner, &stored)
                .map_err(store_error)?;
        }
        self.sync_profile_state();
        Ok(found)
    }

    fn load_profile_cache(&mut self) -> Result<(), FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        let now_ms = self.core.now_millis()?;
        let stored = self
            .core
            .store
            .load_app_profiles(&owner)
            .map_err(store_error)?;
        self.profile_cache.clear();
        self.profile_records.clear();
        for profile in stored {
            let stale = profile.stale || profile.profile.expires_at_ms <= now_ms;
            self.profile_records
                .insert(profile.profile.account_id.clone(), profile.profile.clone());
            self.profile_cache.insert(
                profile.profile.account_id.clone(),
                profile_from_record(profile.profile, stale),
            );
        }
        self.sync_profile_state();
        Ok(())
    }

    fn save_profile(
        &mut self,
        display_name: String,
        about: String,
        picture: Option<String>,
    ) -> Result<(), FiniteChatCoreError> {
        let display_name = normalize_required_profile_text("display name", display_name)?;
        let about = normalize_optional_profile_text(about);
        let picture = normalize_optional_profile_url(picture)?;
        let account_id = self.core.device.device_ref().account_id.clone();
        let now_ms = self.core.now_millis()?;
        let existing = self.profile_records.get(&account_id).cloned();
        let bot = existing.as_ref().and_then(|record| record.bot);
        let finite_role = existing
            .as_ref()
            .and_then(|record| record.finite_role.clone());
        let record = finitechat_http::NostrProfileRecord {
            account_id: account_id.clone(),
            name: Some(display_name.clone()),
            display_name: Some(display_name.clone()),
            about: about.clone(),
            picture: picture.clone(),
            bot,
            finite_role: finite_role.clone(),
            metadata_json: profile_metadata_json_for_edit(
                existing.as_ref(),
                Some(display_name.as_str()),
                about.as_deref(),
                picture.as_deref(),
                bot,
                finite_role.as_deref(),
            )?,
            fetched_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(DEFAULT_PROFILE_CACHE_TTL_MS),
        };
        let mut delivery = self.core.home_delivery();
        delivery.put_nostr_profile(&record).map_err(runtime_error)?;

        self.persist_profile_record(record, false)?;
        self.sync_profile_state();
        self.app.status = "profile saved".to_owned();
        Ok(())
    }

    fn persist_profile_record(
        &mut self,
        record: finitechat_http::NostrProfileRecord,
        stale: bool,
    ) -> Result<(), FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        let stored = StoredAppProfile {
            profile: record.clone(),
            stale,
        };
        self.core
            .store
            .save_app_profiles(&owner, std::slice::from_ref(&stored))
            .map_err(store_error)?;
        self.profile_cache.insert(
            record.account_id.clone(),
            profile_from_record(record.clone(), stale),
        );
        self.profile_records
            .insert(record.account_id.clone(), record);
        Ok(())
    }

    fn persist_profile(&mut self, profile: &AppProfileSummary) -> Result<(), FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        let stored = stored_profile_from_app(profile);
        self.profile_records
            .insert(stored.profile.account_id.clone(), stored.profile.clone());
        self.profile_cache
            .insert(profile.account_id.clone(), profile.clone());
        self.core
            .store
            .save_app_profiles(&owner, std::slice::from_ref(&stored))
            .map_err(store_error)
    }

    fn remember_placeholder_profile(
        &mut self,
        account_id: &str,
    ) -> Result<(), FiniteChatCoreError> {
        self.remember_profile_hint(account_id, None)
    }

    fn remember_profile_hint(
        &mut self,
        account_id: &str,
        display_name_hint: Option<&str>,
    ) -> Result<(), FiniteChatCoreError> {
        let mut profile = placeholder_profile(account_id);
        if let Some(display_name) =
            normalize_profile_display_name_hint(display_name_hint, account_id)
        {
            profile.display_name = display_name;
        }
        self.remember_profile_summary(profile)
    }

    fn remember_profile_summary(
        &mut self,
        profile: AppProfileSummary,
    ) -> Result<(), FiniteChatCoreError> {
        let profile = normalize_profile_summary_hint(profile)?;
        let account_id = profile.account_id.clone();
        let next = merge_profile_summary_hint(self.profile_cache.get(&account_id), profile);
        self.persist_profile(&next)?;
        self.sync_profile_state();
        Ok(())
    }

    fn sync_profile_state(&mut self) {
        self.app.profiles = self.profile_cache.values().cloned().collect();
        self.sync_agent_room_flags();
        self.sync_typing_members();
        self.sync_selected_room_messages();
    }

    fn sync_agent_room_flags(&mut self) {
        let own_account_id = self.core.device.device_ref().account_id.clone();
        let mut flags = BTreeMap::new();
        for room in &self.app.rooms {
            let connected_agent_member = self
                .core
                .device
                .room_members(&room.room_id)
                .ok()
                .into_iter()
                .flatten()
                .filter(|member| member.account_id != own_account_id)
                .any(|member| {
                    self.profile_cache
                        .get(&member.account_id)
                        .is_some_and(|profile| profile.is_agent)
                });
            flags.insert(room.room_id.clone(), connected_agent_member);
        }
        for room in &mut self.app.rooms {
            room.is_agent_chat = flags.get(&room.room_id).copied().unwrap_or(false);
        }
    }

    fn refresh_devices(&mut self) -> Result<(), FiniteChatCoreError> {
        let account_id = self.app.identity.account_id.clone();
        let mut delivery = self.core.home_delivery();
        let mut after_room_id = None;
        let mut devices = BTreeMap::<(String, String), AppDeviceSummary>::new();
        for _ in 0..16 {
            let page = match delivery.list_account_rooms(ListAccountRoomsRequest {
                account_id: account_id.clone(),
                after_room_id: after_room_id.clone(),
                limit: 100,
            }) {
                Ok(page) => page,
                Err(error) => {
                    self.set_online_action_unavailable(
                        "devices unavailable",
                        "Device list could not be refreshed",
                    );
                    debug_assert!(!error.to_string().is_empty());
                    return Ok(());
                }
            };
            for room in page.rooms {
                for room_device in room.devices {
                    if room_device.device.account_id != account_id {
                        continue;
                    }
                    let key = (
                        room_device.device.account_id.clone(),
                        room_device.device.device_id.clone(),
                    );
                    let revoked_key = app_device_key(&key.0, &key.1);
                    let entry = devices
                        .entry(key.clone())
                        .or_insert_with(|| AppDeviceSummary {
                            account_id: key.0.clone(),
                            device_id: key.1.clone(),
                            active: false,
                            current_device: self.app.identity.device_id == key.1,
                            revoked: self.revoked_devices.contains(&revoked_key),
                            room_count: 0,
                        });
                    entry.active |= room_device.active;
                    entry.revoked |= self.revoked_devices.contains(&revoked_key);
                    entry.room_count = entry.room_count.saturating_add(1);
                }
            }
            if !page.has_more {
                break;
            }
            let Some(next) = page.next_after_room_id else {
                break;
            };
            after_room_id = Some(next);
        }
        self.app.devices = devices.into_values().collect();
        self.sync_selected_room_details();
        self.app.status = "devices refreshed".to_owned();
        Ok(())
    }

    fn link_device(
        &mut self,
        fanout_id: String,
        target_device_id: String,
    ) -> Result<DeviceLinkFanoutReport, FiniteChatCoreError> {
        self.core.refresh_device_clock()?;
        validate_string_bytes("device_link.fanout_id", &fanout_id, MAX_OBJECT_ID_BYTES)
            .map_err(client_error)?;
        validate_string_bytes(
            "device_link.target_device_id",
            &target_device_id,
            MAX_OBJECT_ID_BYTES,
        )
        .map_err(client_error)?;
        let owner = self.core.device.device_ref().clone();
        if target_device_id == owner.device_id {
            return Err(FiniteChatCoreError::Client {
                reason: "linked Device must be distinct from the current Device".to_owned(),
            });
        }
        let target = DeviceRef {
            account_id: owner.account_id.clone(),
            device_id: target_device_id.clone(),
        };
        target.validate_limits().map_err(client_error)?;

        let existing = self
            .core
            .device
            .export_state()
            .map_err(client_error)?
            .link_fanouts
            .into_iter()
            .find(|fanout| fanout.fanout_id == fanout_id);
        let mut delivery = self.core.home_delivery();
        self.resume_interrupted_link_fanout_commits(&fanout_id, &mut delivery)?;
        match existing {
            Some(existing) if existing.target_device != target => {
                return Err(FiniteChatCoreError::Client {
                    reason: "device-link fanout id is already bound to another Device".to_owned(),
                });
            }
            Some(_) => {}
            None => {
                let (existing_room_count, _) =
                    device_room_counts(&mut delivery, &target).map_err(delivery_error)?;
                if existing_room_count > 0 {
                    return Err(FiniteChatCoreError::Client {
                        reason: "linked Device id already belongs to an enrolled Device; use a fresh Device id"
                            .to_owned(),
                    });
                }
                self.core
                    .store
                    .start_link_fanout_and_save(
                        &mut self.core.device,
                        fanout_id.clone(),
                        target.clone(),
                    )
                    .map_err(store_error)?;
            }
        }

        let fanout = run_link_fanout_tick(
            &mut self.core.store,
            &mut self.core.device,
            &mut delivery,
            &fanout_id,
            &RuntimeLinkFanoutOptions {
                max_discovery_pages_per_tick: 16,
                max_commit_rooms_per_tick: 4,
                max_completion_sync_pages_per_room: DEFAULT_MAX_SYNC_PAGES_PER_ROOM,
            },
        )
        .map_err(runtime_error)?;
        let bootstrap_export = if fanout.complete {
            self.advance_link_device_bootstrap_export(&fanout_id, &target)?
        } else {
            LinkBootstrapExportState::WaitingForFanout
        };
        let room_count = u32::try_from(
            self.core
                .device
                .link_fanout_room_count(&fanout_id)
                .map_err(client_error)?,
        )
        .map_err(|_| client_error("device-link room count overflow"))?;
        let bootstrap_manifests = match bootstrap_export {
            LinkBootstrapExportState::Emitted { manifests } => manifests,
            _ => Vec::new(),
        };
        let complete = fanout.complete && bootstrap_manifests.len() == room_count as usize;
        let active_room_count = if complete { room_count } else { 0 };
        self.app.status = if complete {
            "device linked".to_owned()
        } else if fanout.complete {
            "device joining".to_owned()
        } else {
            "waiting for linked device".to_owned()
        };
        Ok(DeviceLinkFanoutReport {
            fanout_id,
            target_account_id: owner.account_id,
            target_device_id,
            fanout_complete: complete,
            room_count,
            active_room_count,
            bootstrap_manifests,
        })
    }

    fn resume_interrupted_link_fanout_commits<D: RuntimeDelivery>(
        &mut self,
        requested_fanout_id: &str,
        delivery: &mut D,
    ) -> Result<(), FiniteChatCoreError>
    where
        D::Error: std::fmt::Display,
    {
        let state = self.core.device.export_state().map_err(client_error)?;
        let interrupted = state
            .link_fanouts
            .iter()
            .filter(|fanout| fanout.fanout_id != requested_fanout_id)
            .filter(|fanout| {
                fanout.rooms.iter().any(|room| {
                    matches!(room.status, LinkFanoutRoomStatus::Prepared { .. })
                        && self
                            .core
                            .device
                            .has_pending_commit(&room.plan.room_id)
                            .unwrap_or(false)
                })
            })
            .map(|fanout| fanout.fanout_id.clone())
            .take(4)
            .collect::<Vec<_>>();
        for fanout_id in interrupted {
            run_link_fanout_tick(
                &mut self.core.store,
                &mut self.core.device,
                delivery,
                &fanout_id,
                &RuntimeLinkFanoutOptions {
                    max_discovery_pages_per_tick: 1,
                    max_commit_rooms_per_tick: 1,
                    max_completion_sync_pages_per_room: DEFAULT_MAX_SYNC_PAGES_PER_ROOM,
                },
            )
            .map_err(runtime_error)?;
        }
        Ok(())
    }

    fn advance_link_device_bootstrap_export(
        &mut self,
        bootstrap_id: &str,
        target: &DeviceRef,
    ) -> Result<LinkBootstrapExportState, FiniteChatCoreError> {
        let mut export = self
            .core
            .device
            .link_fanout_bootstrap_export(bootstrap_id)
            .map_err(client_error)?;
        if matches!(export, LinkBootstrapExportState::WaitingForFanout) {
            export = LinkBootstrapExportState::Exporting {
                canonical_selection: self.canonical_agent_bootstrap_selection(),
                rooms: Vec::new(),
            };
            self.persist_link_bootstrap_export(bootstrap_id, export.clone())?;
            return Ok(export);
        }
        let LinkBootstrapExportState::Exporting {
            canonical_selection,
            rooms,
        } = &mut export
        else {
            return Ok(export);
        };

        let fanout = self
            .core
            .device
            .export_state()
            .map_err(client_error)?
            .link_fanouts
            .into_iter()
            .find(|fanout| fanout.fanout_id == bootstrap_id)
            .ok_or_else(|| client_error("device-link fanout disappeared during export"))?;
        if let Some(fanout_room) = fanout.rooms.iter().find(|room| {
            !rooms
                .iter()
                .any(|export| export.room.room_id == room.plan.room_id)
        }) {
            let LinkFanoutRoomStatus::Done { accepted_seq } = fanout_room.status else {
                return Err(client_error(
                    "device-link bootstrap began before membership fanout completed",
                ));
            };
            let room_id = &fanout_room.plan.room_id;
            let app_room = self
                .room(room_id)
                .cloned()
                .unwrap_or_else(|| connected_app_room(room_id, room_id));
            let member_account_ids = self
                .core
                .device
                .room_members(room_id)
                .map_err(client_error)?
                .into_iter()
                .map(|member| member.account_id)
                .collect::<BTreeSet<_>>();
            let profiles = member_account_ids
                .into_iter()
                .map(|account_id| {
                    self.profile_cache
                        .get(&account_id)
                        .cloned()
                        .unwrap_or_else(|| placeholder_profile(&account_id))
                })
                .map(device_link_bootstrap_profile_from_app)
                .collect();
            rooms.push(LinkBootstrapRoomExport {
                room: DeviceLinkBootstrapRoomV2 {
                    room_id: room_id.clone(),
                    display_name: app_room.display_name,
                    picture: app_room.picture,
                },
                canonical_selection: canonical_selection
                    .clone()
                    .filter(|selection| selection.room_id == *room_id),
                profiles,
                through_seq: accepted_seq,
                audit_after_seq: 0,
                audit_complete: false,
                planning_after_seq: 0,
                planning_after_message_id: String::new(),
                planning_complete: false,
                chunks: Vec::new(),
                total_history_events: 0,
                history_sha256: None,
                manifest_sha256: None,
                next_chunk_index: 0,
            });
            self.persist_link_bootstrap_export(bootstrap_id, export.clone())?;
            return Ok(export);
        }

        if let Some(index) = rooms.iter().position(|room| {
            !room.audit_complete
                || !room.planning_complete
                || room.next_chunk_index < room.chunks.len() as u32
        }) {
            let mut room = rooms[index].clone();
            self.advance_link_device_bootstrap_room(bootstrap_id, target, &mut room)?;
            rooms[index] = room;
            self.persist_link_bootstrap_export(bootstrap_id, export.clone())?;
            return Ok(export);
        }

        let manifests = rooms
            .iter()
            .map(|room| {
                Ok(LinkBootstrapManifestReceipt {
                    bootstrap_id: bootstrap_id.to_owned(),
                    room_id: room.room.room_id.clone(),
                    manifest_sha256: room
                        .manifest_sha256
                        .clone()
                        .ok_or_else(|| client_error("device-link manifest was not finalized"))?,
                })
            })
            .collect::<Result<Vec<_>, FiniteChatCoreError>>()?;
        export = LinkBootstrapExportState::Emitted { manifests };
        self.persist_link_bootstrap_export(bootstrap_id, export.clone())?;
        Ok(export)
    }

    fn persist_link_bootstrap_export(
        &mut self,
        bootstrap_id: &str,
        export: LinkBootstrapExportState,
    ) -> Result<(), FiniteChatCoreError> {
        self.core
            .device
            .set_link_fanout_bootstrap_export(bootstrap_id, export)
            .map_err(client_error)?;
        self.core
            .store
            .save_device_state(&self.core.device)
            .map_err(store_error)
    }

    fn advance_link_device_bootstrap_room(
        &mut self,
        bootstrap_id: &str,
        target: &DeviceRef,
        export: &mut LinkBootstrapRoomExport,
    ) -> Result<(), FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        if !export.audit_complete {
            let room_server_url = self.core.room_server_url(&export.room.room_id);
            let mut delivery = self.core.delivery_for(&room_server_url);
            let page = delivery
                .sync_events(&export.room.room_id, &owner, export.audit_after_seq)
                .map_err(delivery_error)?;
            if page.next_after_seq < export.audit_after_seq {
                return Err(client_error(
                    "device-link source history audit cursor regressed",
                ));
            }
            for entry in page
                .entries
                .iter()
                .take_while(|entry| entry.seq <= export.through_seq)
            {
                if entry.kind == LogEntryKind::Application
                    && !self
                        .core
                        .store
                        .has_app_event_identity(
                            &owner,
                            &export.room.room_id,
                            entry.seq,
                            &entry.message_id,
                            &entry.sender,
                            entry.timestamp_unix_seconds,
                        )
                        .map_err(store_error)?
                {
                    return Err(client_error(format!(
                        "device-link source cannot certify local plaintext for application event {} at seq {} from {}",
                        entry.message_id, entry.seq, entry.sender.device_id
                    )));
                }
            }
            let reached_cutoff = page
                .entries
                .last()
                .is_some_and(|entry| entry.seq >= export.through_seq)
                || page.next_after_seq >= export.through_seq
                || !page.has_more;
            if !reached_cutoff && page.next_after_seq == export.audit_after_seq {
                return Err(client_error("device-link source history audit stalled"));
            }
            export.audit_after_seq = page.next_after_seq.min(export.through_seq);
            export.audit_complete = reached_cutoff;
            return Ok(());
        }

        if !export.planning_complete {
            let start_after_seq = export.planning_after_seq;
            let start_after_message_id = export.planning_after_message_id.clone();
            let page = self
                .core
                .store
                .load_app_events_for_room_page(
                    &owner,
                    &export.room.room_id,
                    export.through_seq,
                    start_after_seq,
                    &start_after_message_id,
                    DEVICE_LINK_BOOTSTRAP_STORE_PAGE_SIZE,
                )
                .map_err(store_error)?;
            if page.is_empty() {
                if export.chunks.is_empty() {
                    export.chunks.push(LinkBootstrapChunkPlan {
                        start_after_seq,
                        start_after_message_id,
                        end_seq: export.planning_after_seq,
                        end_message_id: export.planning_after_message_id.clone(),
                        event_count: 0,
                        chunk_sha256: device_link_bootstrap_chunk_sha256_hex(&[]),
                    });
                }
                finalize_link_bootstrap_room_manifest(bootstrap_id, &owner, target, export)?;
                return Ok(());
            }
            let page_len = page.len();
            let mut history = Vec::new();
            let mut last_cursor = (start_after_seq, start_after_message_id.clone());
            let mut exhausted_page = true;
            for stored in page {
                if is_device_link_control_event(&stored.plaintext) {
                    last_cursor = (stored.seq, stored.message_id);
                    continue;
                }
                let candidate = device_link_history_event_from_stored(stored.clone())?;
                let mut sized_history = history.clone();
                sized_history.push(candidate.clone());
                let sizing = DeviceLinkBootstrapV2 {
                    version: DEVICE_LINK_BOOTSTRAP_VERSION_V2,
                    bootstrap_id: bootstrap_id.to_owned(),
                    target: target.clone(),
                    chunk_index: u32::MAX,
                    chunk_count: u32::MAX,
                    total_history_events: u64::MAX,
                    history_sha256: "0".repeat(64),
                    room: export.room.clone(),
                    canonical_selection: export.canonical_selection.clone(),
                    profiles: export.profiles.clone(),
                    history: sized_history,
                };
                if history.len() >= MAX_DEVICE_LINK_BOOTSTRAP_EVENTS as usize
                    || serde_json::to_vec(&sizing).map_err(client_error)?.len()
                        > MAX_DEVICE_LINK_BOOTSTRAP_PAYLOAD_BYTES as usize
                {
                    if history.is_empty() {
                        return Err(client_error(
                            "one history event cannot fit a device-link bootstrap chunk",
                        ));
                    }
                    exhausted_page = false;
                    break;
                }
                history.push(candidate);
                last_cursor = (stored.seq, stored.message_id);
            }
            export.planning_after_seq = last_cursor.0;
            export.planning_after_message_id = last_cursor.1.clone();
            if !history.is_empty() {
                export.total_history_events = export
                    .total_history_events
                    .checked_add(history.len() as u64)
                    .ok_or_else(|| client_error("device-link history count overflow"))?;
                if export.total_history_events > MAX_DEVICE_LINK_BOOTSTRAP_TOTAL_EVENTS {
                    return Err(client_error(
                        "device-link history exceeds the complete-transfer limit",
                    ));
                }
                export.chunks.push(LinkBootstrapChunkPlan {
                    start_after_seq,
                    start_after_message_id,
                    end_seq: last_cursor.0,
                    end_message_id: last_cursor.1,
                    event_count: history.len() as u32,
                    chunk_sha256: device_link_bootstrap_chunk_sha256_hex(&history),
                });
                if export.chunks.len() > MAX_DEVICE_LINK_BOOTSTRAP_CHUNKS as usize {
                    return Err(client_error(
                        "device-link history exceeds the complete-transfer chunk limit",
                    ));
                }
            }
            if exhausted_page && page_len < DEVICE_LINK_BOOTSTRAP_STORE_PAGE_SIZE as usize {
                finalize_link_bootstrap_room_manifest(bootstrap_id, &owner, target, export)?;
            }
            return Ok(());
        }

        let chunk_index = export.next_chunk_index as usize;
        let Some(plan) = export.chunks.get(chunk_index).cloned() else {
            return Ok(());
        };
        let mut history = Vec::new();
        let mut reached_end = plan.event_count == 0;
        if plan.event_count > 0 {
            let page = self
                .core
                .store
                .load_app_events_for_room_page(
                    &owner,
                    &export.room.room_id,
                    export.through_seq,
                    plan.start_after_seq,
                    &plan.start_after_message_id,
                    DEVICE_LINK_BOOTSTRAP_STORE_PAGE_SIZE,
                )
                .map_err(store_error)?;
            for stored in page {
                if !is_device_link_control_event(&stored.plaintext) {
                    history.push(device_link_history_event_from_stored(stored.clone())?);
                }
                if stored.seq == plan.end_seq && stored.message_id == plan.end_message_id {
                    reached_end = true;
                    break;
                }
            }
        }
        if !reached_end
            || history.len() != plan.event_count as usize
            || device_link_bootstrap_chunk_sha256_hex(&history) != plan.chunk_sha256
        {
            return Err(client_error(
                "device-link frozen history changed before chunk emission",
            ));
        }
        let bootstrap = DeviceLinkBootstrapV2 {
            version: DEVICE_LINK_BOOTSTRAP_VERSION_V2,
            bootstrap_id: bootstrap_id.to_owned(),
            target: target.clone(),
            chunk_index: export.next_chunk_index,
            chunk_count: export.chunks.len() as u32,
            total_history_events: export.total_history_events,
            history_sha256: export
                .history_sha256
                .clone()
                .ok_or_else(|| client_error("device-link history digest was not finalized"))?,
            room: export.room.clone(),
            canonical_selection: export.canonical_selection.clone(),
            profiles: export.profiles.clone(),
            history,
        };
        self.send_link_device_bootstrap(&export.room.room_id, &bootstrap)?;
        export.next_chunk_index = export.next_chunk_index.saturating_add(1);
        Ok(())
    }

    fn send_link_device_bootstrap(
        &mut self,
        room_id: &str,
        bootstrap: &DeviceLinkBootstrapV2,
    ) -> Result<(), FiniteChatCoreError> {
        bootstrap.validate_limits().map_err(client_error)?;
        let payload = serde_json::to_vec(bootstrap).map_err(client_error)?;
        if payload.len() > MAX_DEVICE_LINK_BOOTSTRAP_PAYLOAD_BYTES as usize {
            return Err(client_error("device-link bootstrap payload is too large"));
        }
        let idempotency_key = device_link_bootstrap_idempotency_key(
            &bootstrap.bootstrap_id,
            room_id,
            bootstrap.chunk_index,
        );
        self.core.send_application_event_with_idempotency_key(
            room_id,
            device_link_bootstrap_event_kind(),
            None,
            &payload,
            idempotency_key,
        )?;
        Ok(())
    }

    fn canonical_agent_bootstrap_selection(&self) -> Option<DeviceLinkBootstrapSelectionV2> {
        let paired_room_id = self
            .app
            .paired_agent
            .as_ref()
            .map(|paired| paired.canonical_room_id.clone());
        let agent_room_ids = self
            .app
            .rooms
            .iter()
            .filter(|room| room.state == AppRoomState::Connected && room.is_agent_chat)
            .map(|room| room.room_id.clone())
            .collect::<Vec<_>>();
        let selected_room_id = self.app.selected_room_id.as_ref();
        let room_id = paired_room_id
            .filter(|paired| agent_room_ids.contains(paired))
            .or_else(|| {
                selected_room_id
                    .filter(|selected| agent_room_ids.contains(selected))
                    .cloned()
            })
            .or_else(|| (agent_room_ids.len() == 1).then(|| agent_room_ids[0].clone()))?;
        let topic_id = if selected_room_id == Some(&room_id) {
            self.app.selected_topic_id.clone()
        } else {
            self.app
                .topics
                .iter()
                .find(|topic| topic.room_id == room_id && topic.topic_id == HOME_TOPIC_ID)
                .map(|topic| topic.topic_id.clone())
        }?;
        let chat_id = if selected_room_id == Some(&room_id)
            && self.app.selected_topic_id.as_ref() == Some(&topic_id)
        {
            self.app.selected_chat_id.clone()
        } else {
            self.default_chat_id_for_topic(&room_id, &topic_id)
        }?;
        self.chat_exists(&room_id, &topic_id, &chat_id)
            .then_some(DeviceLinkBootstrapSelectionV2 {
                room_id,
                topic_id,
                chat_id,
            })
    }

    fn revoke_device(
        &mut self,
        account_id: String,
        device_id: String,
    ) -> Result<(), FiniteChatCoreError> {
        if account_id == self.app.identity.account_id && device_id == self.app.identity.device_id {
            return Err(FiniteChatCoreError::Client {
                reason: "cannot revoke the current device from this device".to_owned(),
            });
        }
        let device = DeviceRef {
            account_id,
            device_id,
        };
        let mut delivery = self.core.home_delivery();
        if let Err(error) = delivery.revoke_device(&device) {
            self.set_online_action_unavailable("device unavailable", "Device could not be revoked");
            debug_assert!(!error.to_string().is_empty());
            return Ok(());
        }
        self.revoked_devices
            .insert(app_device_key(&device.account_id, &device.device_id));
        self.persist_app_state()?;
        self.refresh_devices()?;
        self.app.status = "device revoked".to_owned();
        Ok(())
    }

    fn append_messages(&mut self, messages: Vec<ChatMessage>) {
        if messages.is_empty() {
            return;
        }
        let owner = self.core.device.device_ref().clone();
        let selected_room_id = self.app.selected_room_id.clone();
        let selected_message_count = selected_room_id.as_ref().map_or(0, |room_id| {
            messages
                .iter()
                .filter(|message| message.room_id == *room_id)
                .count()
        });
        for message in &messages {
            self.clear_default_typing_for_sender(
                &message.room_id,
                &DeviceRef {
                    account_id: message.sender_account_id.clone(),
                    device_id: message.sender_device_id.clone(),
                },
            );
        }
        self.chat_projection.append_messages(messages, &owner);
        if let Some(room_id) = selected_room_id
            && selected_message_count > 0
        {
            let current_count = self.loaded_message_count(&room_id);
            if current_count > DEFAULT_TRANSCRIPT_WINDOW {
                let next_count = current_count
                    .saturating_add(selected_message_count)
                    .min(MAX_APP_MESSAGES);
                self.loaded_message_counts.insert(room_id, next_count);
            }
        }
        self.sync_chat_projection();
        self.sync_typing_members();
    }

    fn apply_projection_events(
        &mut self,
        events: Vec<StoredAppEvent>,
    ) -> Result<(), FiniteChatCoreError> {
        if events.is_empty() {
            return Ok(());
        }
        let owner = self.core.device.device_ref().clone();
        let mut projection_events = Vec::new();
        let mut bootstrap_selection = None;
        let mut completed_bootstrap_rooms = BTreeSet::new();
        for event in &events {
            if let Some(bootstrap) = device_link_bootstrap_from_stored_event(event)
                && let Some(accepted) = self.accept_link_device_bootstrap(event, bootstrap)?
            {
                completed_bootstrap_rooms.insert(accepted.room_id);
                bootstrap_selection = bootstrap_selection.or(accepted.canonical_selection);
            }
        }
        projection_events.extend(events.into_iter().filter(|event| {
            is_device_link_control_event(&event.plaintext)
                || completed_bootstrap_rooms.contains(&event.room_id)
                || !self.room_is_provisional_for_device_link(&event.room_id)
        }));
        let mut archive_state_changed = false;
        for event in projection_events {
            let parsed = ParsedAppEvent::decode(event);
            if let Some(app_event) = &parsed.envelope {
                let _ = self
                    .activity_projection
                    .clear_from_durable_application_event(
                        &parsed.event.room_id,
                        &parsed.event.sender,
                        app_event,
                    );
            }
            archive_state_changed |= self.chat_projection.apply_parsed_event(parsed, &owner);
        }
        self.sync_chat_projection();
        if archive_state_changed {
            self.persist_app_state()?;
        }
        if let Some(selection) = bootstrap_selection {
            self.apply_link_device_bootstrap_selection(selection)?;
        }
        self.sync_typing_members();
        Ok(())
    }

    fn accept_link_device_bootstrap(
        &mut self,
        envelope: &StoredAppEvent,
        bootstrap: DeviceLinkBootstrapV2,
    ) -> Result<Option<AcceptedDeviceLinkBootstrap>, FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        if envelope.sender.account_id != owner.account_id
            || bootstrap.target != owner
            || bootstrap.room.room_id != envelope.room_id
            || !self.core.has_room(&envelope.room_id)
        {
            return Ok(None);
        }
        let encoded = serde_json::to_vec(&bootstrap).map_err(client_error)?;
        if encoded.len() > MAX_DEVICE_LINK_BOOTSTRAP_PAYLOAD_BYTES as usize {
            return Ok(None);
        }
        match self
            .core
            .store
            .stage_device_link_bootstrap_chunk(&owner, &envelope.sender, envelope.seq, &bootstrap)
            .map_err(store_error)?
        {
            DeviceLinkBootstrapStageOutcome::Ready(pending) => {
                self.finish_link_device_bootstrap(pending)
            }
            DeviceLinkBootstrapStageOutcome::AlreadyCommitted(_) => {
                self.refresh_device_link_bootstrap_receipts()?;
                Ok(None)
            }
            DeviceLinkBootstrapStageOutcome::Pending { .. }
            | DeviceLinkBootstrapStageOutcome::ExactDuplicate { .. }
            | DeviceLinkBootstrapStageOutcome::Poisoned
            | DeviceLinkBootstrapStageOutcome::CapacityExceeded => Ok(None),
        }
    }

    fn finish_link_device_bootstrap(
        &mut self,
        pending: StoredPendingDeviceLinkBootstrap,
    ) -> Result<Option<AcceptedDeviceLinkBootstrap>, FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        if pending.receipt.target != owner
            || pending.receipt.source.account_id != owner.account_id
            || !self.core.has_room(&pending.receipt.room_id)
            || !pending.is_complete()
        {
            return Ok(None);
        }
        let member_account_ids = self
            .core
            .device
            .room_members(&pending.receipt.room_id)
            .map_err(client_error)?
            .into_iter()
            .map(|member| member.account_id)
            .collect::<BTreeSet<_>>();
        let profile_summaries = pending
            .profiles
            .iter()
            .filter(|profile| member_account_ids.contains(&profile.account_id))
            .cloned()
            .map(app_profile_from_device_link_bootstrap)
            .collect::<Vec<_>>();
        let stored_profiles = profile_summaries
            .iter()
            .map(stored_profile_from_app)
            .collect::<Vec<_>>();
        let stored_room = StoredAppRoom {
            room_id: pending.room.room_id.clone(),
            display_name: pending.room.display_name.clone(),
            picture: pending.room.picture.clone(),
            state: StoredAppRoomState::Connected,
            status: "connected".to_owned(),
            local_read_seq: self
                .local_read_seq
                .get(&pending.room.room_id)
                .copied()
                .unwrap_or_default(),
        };
        let canonical_selection = pending.canonical_selection.clone();
        match self
            .core
            .store
            .commit_device_link_bootstrap_atomically(
                &owner,
                &pending.receipt,
                &stored_room,
                &stored_profiles,
            )
            .map_err(store_error)?
        {
            DeviceLinkBootstrapCommitOutcome::Committed(receipt) => {
                self.upsert_room(
                    &pending.room.room_id,
                    &pending.room.display_name,
                    pending.room.picture,
                    AppRoomState::Connected,
                    "connected",
                );
                self.replay_room_history_into_projection(&pending.room.room_id)?;
                for (profile, stored) in profile_summaries.into_iter().zip(stored_profiles) {
                    self.profile_records
                        .insert(stored.profile.account_id.clone(), stored.profile);
                    self.profile_cache
                        .insert(profile.account_id.clone(), profile);
                }
                self.sync_profile_state();
                self.refresh_device_link_bootstrap_receipts()?;
                debug_assert!(self.app.device_link_bootstrap_receipts.iter().any(|found| {
                    found.bootstrap_id == receipt.bootstrap_id
                        && found.room_id == receipt.room_id
                        && found.manifest_sha256 == receipt.manifest_sha256
                }));
                self.sync_chat_projection();
                Ok(Some(AcceptedDeviceLinkBootstrap {
                    room_id: pending.room.room_id,
                    canonical_selection,
                }))
            }
            DeviceLinkBootstrapCommitOutcome::AlreadyCommitted(_) => {
                self.refresh_device_link_bootstrap_receipts()?;
                Ok(None)
            }
            DeviceLinkBootstrapCommitOutcome::Poisoned
            | DeviceLinkBootstrapCommitOutcome::Incomplete => Ok(None),
        }
    }

    fn refresh_device_link_bootstrap_receipts(&mut self) -> Result<(), FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        self.app.device_link_bootstrap_receipts = self
            .core
            .store
            .load_completed_device_link_bootstrap_receipts(&owner)
            .map_err(store_error)?
            .iter()
            .map(app_device_link_bootstrap_receipt)
            .collect();
        Ok(())
    }

    fn room_is_provisional_for_device_link(&self, room_id: &str) -> bool {
        if self
            .app
            .device_link_bootstrap_receipts
            .iter()
            .any(|receipt| receipt.room_id == room_id)
        {
            return false;
        }
        let owner = self.core.device.device_ref();
        self.room(room_id)
            .is_some_and(|room| room.display_name == room.room_id)
            && self.core.device.room_members(room_id).is_ok_and(|members| {
                members.iter().any(|member| {
                    member.account_id == owner.account_id && member.device_id != owner.device_id
                })
            })
    }

    fn replay_room_history_into_projection(
        &mut self,
        room_id: &str,
    ) -> Result<(), FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        let through_seq = self
            .core
            .store
            .max_app_event_seq_for_room(&owner, room_id)
            .map_err(store_error)?
            .unwrap_or(0);
        let mut after_seq = 0_u64;
        let mut after_message_id = String::new();
        let mut archive_state_changed = false;
        loop {
            let page = self
                .core
                .store
                .load_app_events_for_room_page(
                    &owner,
                    room_id,
                    through_seq,
                    after_seq,
                    &after_message_id,
                    DEVICE_LINK_BOOTSTRAP_STORE_PAGE_SIZE,
                )
                .map_err(store_error)?;
            if page.is_empty() {
                break;
            }
            let page_len = page.len();
            for event in page {
                after_seq = event.seq;
                after_message_id.clone_from(&event.message_id);
                if !is_device_link_control_event(&event.plaintext) {
                    archive_state_changed |= self
                        .chat_projection
                        .apply_parsed_event(ParsedAppEvent::decode(event), &owner);
                }
            }
            if page_len < DEVICE_LINK_BOOTSTRAP_STORE_PAGE_SIZE as usize {
                break;
            }
        }
        if archive_state_changed {
            self.persist_app_state()?;
        }
        Ok(())
    }

    fn apply_link_device_bootstrap_selection(
        &mut self,
        selection: DeviceLinkBootstrapSelectionV2,
    ) -> Result<(), FiniteChatCoreError> {
        if !self.room_is_connected(&selection.room_id)
            || !self.chat_exists(&selection.room_id, &selection.topic_id, &selection.chat_id)
        {
            return Ok(());
        }
        // Pairing and navigation are separate. A user-selected route must not
        // be overwritten, but the authenticated source's canonical direct
        // agent room still has to become the companion's paired agent.
        if self.app.paired_agent.is_none() {
            self.pair_agent(selection.room_id.clone())?;
        }
        if self.selection_is_explicit_or_bootstrapped {
            return Ok(());
        }
        self.app.selected_room_id = Some(selection.room_id.clone());
        self.app.selected_topic_id = Some(selection.topic_id);
        self.app.selected_chat_id = Some(selection.chat_id);
        self.loaded_message_counts
            .entry(selection.room_id)
            .or_insert(DEFAULT_TRANSCRIPT_WINDOW);
        self.persist_app_state()?;
        self.selection_is_explicit_or_bootstrapped = true;
        self.sync_selected_room_messages();
        Ok(())
    }

    fn sync_chat_projection(&mut self) {
        let messages = self.chat_projection.messages();
        apply_room_message_projection(&mut self.app.rooms, &messages, &self.local_read_seq);
        self.app.topics = self.chat_projection.topics(&self.local_read_seq);
        if let Some(room_id) = self.app.selected_room_id.clone()
            && self.app.selected_topic_id.is_none()
            && self.topic_exists(&room_id, HOME_TOPIC_ID)
        {
            self.app.selected_topic_id = Some(HOME_TOPIC_ID.to_owned());
            self.app.selected_chat_id = self.default_chat_id_for_topic(&room_id, HOME_TOPIC_ID);
        }
        self.repair_selected_topic();
        self.sync_selected_room_messages();
    }

    fn sync_selected_room_messages(&mut self) {
        let Some(room_id) = self.app.selected_room_id.clone() else {
            self.app.messages.clear();
            self.app.media_gallery = None;
            self.app.room_details = None;
            self.sync_transcript_load_state();
            return;
        };
        let count = self.loaded_message_count(&room_id);
        let mut messages = if let (Some(topic_id), Some(chat_id)) = (
            self.app.selected_topic_id.as_deref(),
            self.app.selected_chat_id.as_deref(),
        ) {
            self.chat_projection
                .messages_for_chat_window(&room_id, topic_id, chat_id, count)
        } else if let Some(topic_id) = self.app.selected_topic_id.as_deref() {
            self.chat_projection
                .messages_for_topic_window(&room_id, topic_id, count)
        } else {
            self.chat_projection
                .messages_for_room_window(&room_id, count)
        };
        self.core.apply_attachment_cache_paths(&mut messages);
        self.apply_attachment_download_progress(&mut messages);
        self.app.messages = messages;
        self.sync_selected_room_media_gallery(&room_id);
        self.sync_transcript_load_state();
        self.sync_selected_room_details();
    }

    fn sync_selected_room_media_gallery(&mut self, room_id: &str) {
        let mut messages = if let (Some(topic_id), Some(chat_id)) = (
            self.app.selected_topic_id.as_deref(),
            self.app.selected_chat_id.as_deref(),
        ) {
            self.chat_projection
                .visual_media_messages_for_chat(room_id, topic_id, chat_id)
        } else if let Some(topic_id) = self.app.selected_topic_id.as_deref() {
            self.chat_projection
                .visual_media_messages_for_topic(room_id, topic_id)
        } else {
            self.chat_projection.visual_media_messages_for_room(room_id)
        };
        self.core.apply_attachment_cache_paths(&mut messages);
        self.apply_attachment_download_progress(&mut messages);
        self.app.media_gallery = Some(ChatProjectionState::media_gallery_from_messages(
            room_id, &messages,
        ));
    }

    fn sync_selected_room_details(&mut self) {
        let Some(room_id) = self.app.selected_room_id.clone() else {
            self.app.room_details = None;
            return;
        };
        let Some(room) = self.room(&room_id).cloned() else {
            self.app.room_details = None;
            return;
        };
        let media_item_count = self
            .app
            .media_gallery
            .as_ref()
            .filter(|gallery| gallery.room_id == room_id)
            .map(|gallery| gallery.items.len().min(u32::MAX as usize) as u32)
            .unwrap_or_default();
        let members = if room.state == AppRoomState::Connected {
            self.core
                .device
                .room_members(&room_id)
                .map(|members| {
                    room_member_summaries(
                        members,
                        &self.profile_cache,
                        self.core.device.device_ref(),
                    )
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.app.room_details = Some(AppRoomDetailsState {
            room_id,
            display_name: room.display_name.clone(),
            picture: room.picture.clone(),
            state: room.state.clone(),
            status: room.status.clone(),
            user_status_text: app_room_user_status_text(&room),
            media_item_count,
            members,
            devices: self.app.devices.clone(),
        });
    }

    fn apply_attachment_download_progress(&self, messages: &mut [ChatMessage]) {
        for message in messages {
            for attachment in &mut message.media {
                let key = attachment_download_key(
                    &message.room_id,
                    &message.message_id,
                    &attachment.attachment_id,
                );
                if self.downloading_attachments.contains(&key) && attachment.local_path.is_none() {
                    attachment.download_progress_per_mille = Some(0);
                } else {
                    attachment.download_progress_per_mille = None;
                }
            }
        }
    }

    fn sync_transcript_load_state(&mut self) {
        let selected_room_id = self.app.selected_room_id.clone();
        let selected_can_load_older = selected_room_id.as_ref().is_some_and(|room_id| {
            let total_count = if let (Some(topic_id), Some(chat_id)) = (
                self.app.selected_topic_id.as_deref(),
                self.app.selected_chat_id.as_deref(),
            ) {
                self.chat_projection
                    .chat_message_count(room_id, topic_id, chat_id)
            } else {
                self.app
                    .selected_topic_id
                    .as_deref()
                    .map(|topic_id| self.chat_projection.topic_message_count(room_id, topic_id))
                    .unwrap_or_else(|| self.chat_projection.room_message_count(room_id))
            };
            total_count > self.loaded_message_count(room_id)
        });
        for room in &mut self.app.rooms {
            room.can_load_older = selected_room_id.as_deref() == Some(room.room_id.as_str())
                && selected_can_load_older;
        }
    }

    fn repair_selected_topic(&mut self) {
        let Some(room_id) = self.app.selected_room_id.clone() else {
            self.app.selected_topic_id = None;
            self.app.selected_chat_id = None;
            return;
        };
        let selected_topic = self.app.selected_topic_id.as_ref().and_then(|topic_id| {
            self.app.topics.iter().find(|topic| {
                topic.room_id == room_id && topic.topic_id == *topic_id && !topic.archived
            })
        });
        let Some(topic) = selected_topic else {
            self.app.selected_topic_id = None;
            self.app.selected_chat_id = None;
            return;
        };
        if self
            .app
            .selected_chat_id
            .as_ref()
            .is_some_and(|chat_id| topic.chats.iter().any(|chat| chat.chat_id == *chat_id))
        {
            return;
        }
        self.app.selected_chat_id = topic
            .active_chat_id
            .clone()
            .or_else(|| topic.chats.first().map(|chat| chat.chat_id.clone()));
    }

    fn refresh_ephemeral_activity_for_connected_rooms(
        &mut self,
    ) -> Result<(), FiniteChatCoreError> {
        let now_ms = self.expire_ephemeral_activity()?;
        let room_ids = self
            .app
            .rooms
            .iter()
            .filter(|room| room.state == AppRoomState::Connected)
            .filter(|room| self.core.has_room(&room.room_id))
            .map(|room| room.room_id.clone())
            .collect::<Vec<_>>();
        for room_id in room_ids {
            self.refresh_ephemeral_activity_for_room_at(&room_id, now_ms)?;
        }
        self.sync_typing_members();
        Ok(())
    }

    fn refresh_ephemeral_activity_for_room(
        &mut self,
        room_id: &str,
    ) -> Result<(), FiniteChatCoreError> {
        let now_ms = self.expire_ephemeral_activity()?;
        self.refresh_ephemeral_activity_for_room_at(room_id, now_ms)?;
        self.sync_typing_members();
        Ok(())
    }

    fn expire_ephemeral_activity(&mut self) -> Result<u64, FiniteChatCoreError> {
        let now_ms = self.core.now_millis()?;
        self.activity_projection
            .expire_at(now_ms)
            .map_err(client_error)?;
        self.local_typing_leases
            .retain(|_, expires_at_ms| *expires_at_ms > now_ms);
        self.sync_typing_members();
        Ok(now_ms)
    }

    fn refresh_ephemeral_activity_for_room_at(
        &mut self,
        room_id: &str,
        now_ms: u64,
    ) -> Result<(), FiniteChatCoreError> {
        let selected_topic_id = (self.app.selected_room_id.as_deref() == Some(room_id))
            .then(|| self.app.selected_topic_id.clone())
            .flatten();
        let scopes = std::iter::once(None)
            .chain(selected_topic_id.as_deref().map(Some))
            .collect::<Vec<_>>();
        for conversation_id in scopes {
            let records = match self
                .core
                .get_ephemeral_activities(room_id, conversation_id, now_ms)
            {
                Ok(records) => records,
                Err(FiniteChatCoreError::Delivery { .. }) => {
                    continue;
                }
                Err(error) => return Err(error),
            };
            for record in records {
                let Ok(plaintext) = self
                    .core
                    .device
                    .decrypt_activity_payload(&record.room_id, &record.payload)
                else {
                    continue;
                };
                let Ok(activity) =
                    serde_json::from_slice::<DecryptedEphemeralActivityV1>(&plaintext)
                else {
                    continue;
                };
                let context = EphemeralActivityIngressContext {
                    room_id: &record.room_id,
                    conversation_id: record.conversation_id.as_deref(),
                    sender: &record.sender,
                    received_at_ms: record.received_at_ms,
                    expires_at_ms: record.expires_at_ms,
                };
                let _ = self.activity_projection.apply(context, &activity);
            }
        }
        Ok(())
    }

    fn clear_default_typing_for_sender(&mut self, room_id: &str, sender: &DeviceRef) {
        let clear = RuntimeActivityClearV1 {
            activity_kind: FINITECHAT_ACTIVITY_KIND_TYPING.to_owned(),
            activity_id: None,
            conversation_id: None,
        };
        let _ = self
            .activity_projection
            .clear_from_durable_terminal(room_id, None, sender, &clear);
    }

    fn sync_typing_members(&mut self) {
        let owner = self.core.device.device_ref();
        let connected_rooms = self
            .app
            .rooms
            .iter()
            .filter(|room| room.state == AppRoomState::Connected)
            .map(|room| room.room_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut members = self
            .activity_projection
            .entries()
            .filter(|entry| connected_rooms.contains(entry.room_id.as_str()))
            .filter(|entry| is_chat_live_indicator_activity(&entry.activity_kind))
            .filter(|entry| entry.sender != *owner)
            .map(|entry| typing_member_from_activity(entry, &self.profile_cache))
            .collect::<Vec<_>>();
        members.sort_by(|left, right| {
            left.room_id
                .cmp(&right.room_id)
                .then_with(|| left.topic_id.cmp(&right.topic_id))
                .then_with(|| left.chat_id.cmp(&right.chat_id))
                .then_with(|| left.display_name.cmp(&right.display_name))
                .then_with(|| left.account_id.cmp(&right.account_id))
                .then_with(|| left.device_id.cmp(&right.device_id))
                .then_with(|| {
                    live_indicator_activity_priority(&left.activity_kind)
                        .cmp(&live_indicator_activity_priority(&right.activity_kind))
                })
        });
        let mut deduped = Vec::with_capacity(members.len());
        for member in members {
            let duplicate_sender = deduped
                .last()
                .map(|existing: &AppTypingMember| {
                    existing.room_id == member.room_id
                        && existing.topic_id == member.topic_id
                        && existing.chat_id == member.chat_id
                        && existing.account_id == member.account_id
                        && existing.device_id == member.device_id
                })
                .unwrap_or(false);
            if !duplicate_sender {
                deduped.push(member);
            }
        }
        self.app.typing_members = deduped;
    }

    fn loaded_message_count(&self, room_id: &str) -> usize {
        self.loaded_message_counts
            .get(room_id)
            .copied()
            .unwrap_or(DEFAULT_TRANSCRIPT_WINDOW)
            .min(MAX_APP_MESSAGES)
    }

    fn persist_room_projection(&mut self, room_id: &str) -> Result<(), FiniteChatCoreError> {
        // Read-only opens report the repaired projection but leave the
        // durable write to the writer.
        if self.core.read_only {
            return Ok(());
        }
        let Some(room) = self.room(room_id).cloned() else {
            return Ok(());
        };
        let local_read_seq = self
            .local_read_seq
            .get(room_id)
            .copied()
            .unwrap_or_default();
        let stored = stored_room_from_app(&room, local_read_seq);
        let owner = self.core.device.device_ref().clone();
        self.core
            .store
            .save_app_rooms(&owner, std::slice::from_ref(&stored))
            .map_err(store_error)
    }

    fn persist_app_state(&mut self) -> Result<(), FiniteChatCoreError> {
        // Read-only opens report the repaired projection but leave the
        // durable write to the writer.
        if self.core.read_only {
            return Ok(());
        }
        let owner = self.core.device.device_ref().clone();
        let stored = StoredAppState {
            selected_room_id: self.app.selected_room_id.clone(),
            selected_topic_id: self.app.selected_topic_id.clone(),
            selected_chat_id: self.app.selected_chat_id.clone(),
            paired_agent: self
                .app
                .paired_agent
                .as_ref()
                .map(|paired| StoredPairedAgent {
                    agent_account_id: paired.agent_account_id.clone(),
                    canonical_room_id: paired.canonical_room_id.clone(),
                }),
            revoked_devices: self.revoked_device_refs(),
            chat_archives: self.chat_projection.stored_chat_archives(),
        };
        self.core
            .store
            .save_app_state(&owner, &stored)
            .map_err(store_error)
    }

    fn revoked_device_refs(&self) -> BTreeSet<DeviceRef> {
        self.revoked_devices
            .iter()
            .filter_map(|key| {
                let (account_id, device_id) = key.split_once('/')?;
                Some(DeviceRef {
                    account_id: account_id.to_owned(),
                    device_id: device_id.to_owned(),
                })
            })
            .collect()
    }

    fn app_outbox_debug_rows(&self) -> Result<Vec<AppOutboxDebugRow>, FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        self.core
            .store
            .load_app_outbox(&owner)
            .map_err(store_error)?
            .into_iter()
            .map(app_outbox_debug_row)
            .collect()
    }

    #[cfg(test)]
    fn test_outbox(&self) -> Result<Vec<StoredOutboundMessage>, FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        self.core.store.load_app_outbox(&owner).map_err(store_error)
    }

    #[cfg(test)]
    fn test_save_outbox(
        &mut self,
        rows: Vec<StoredOutboundMessage>,
    ) -> Result<(), FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        self.core
            .store
            .save_app_outbox(&owner, &rows)
            .map_err(store_error)
    }

    #[cfg(test)]
    fn test_seed_room_state(
        &mut self,
        room: StoredAppRoom,
        selected_room_id: Option<String>,
    ) -> Result<(), FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        self.core
            .store
            .save_app_rooms(&owner, std::slice::from_ref(&room))
            .map_err(store_error)?;
        self.core
            .store
            .save_app_state(
                &owner,
                &StoredAppState {
                    selected_room_id,
                    selected_topic_id: None,
                    selected_chat_id: None,
                    paired_agent: None,
                    revoked_devices: BTreeSet::new(),
                    chat_archives: Vec::new(),
                },
            )
            .map_err(store_error)
    }

    fn persist_outbox_message(
        &mut self,
        message: &StoredOutboundMessage,
    ) -> Result<(), FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        self.core
            .store
            .save_app_outbox(&owner, std::slice::from_ref(message))
            .map_err(store_error)
    }

    fn load_outbox_message(
        &mut self,
        room_id: &str,
        message_id: &str,
    ) -> Result<Option<StoredOutboundMessage>, FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        let messages = self
            .core
            .store
            .load_app_outbox(&owner)
            .map_err(store_error)?;
        Ok(messages
            .into_iter()
            .find(|message| message.room_id == room_id && message.message_id == message_id))
    }

    fn remove_outbox_message(
        &mut self,
        room_id: &str,
        message_id: &str,
    ) -> Result<(), FiniteChatCoreError> {
        let owner = self.core.device.device_ref().clone();
        self.core
            .store
            .delete_app_outbox_message(&owner, room_id, message_id)
            .map_err(store_error)
    }

    fn upsert_room(
        &mut self,
        room_id: &str,
        display_name: &str,
        picture: Option<String>,
        state: AppRoomState,
        status: &str,
    ) {
        if let Some(index) = self
            .app
            .rooms
            .iter()
            .position(|room| room.room_id == room_id)
        {
            self.app.rooms[index].display_name = display_name.to_owned();
            if picture.is_some() {
                self.app.rooms[index].picture = picture;
            }
            self.app.rooms[index].state = state;
            self.app.rooms[index].status = status.to_owned();
            self.app.rooms[index].user_status_text =
                app_room_user_status_text(&self.app.rooms[index]);
            self.sync_agent_room_flags();
            sort_app_rooms(&mut self.app.rooms);
            return;
        }
        let user_status_text = app_room_user_status_text_from_parts(&state, status);
        self.app.rooms.push(AppRoomSummary {
            room_id: room_id.to_owned(),
            display_name: display_name.to_owned(),
            picture,
            state,
            status: status.to_owned(),
            user_status_text,
            last_message_preview: String::new(),
            unread_count: 0,
            can_load_older: false,
            is_agent_chat: false,
        });
        self.sync_agent_room_flags();
        sort_app_rooms(&mut self.app.rooms);
    }

    fn room_is_connected(&self, room_id: &str) -> bool {
        self.room(room_id)
            .is_some_and(|room| room.state == AppRoomState::Connected)
            && self.core.has_room(room_id)
    }

    fn ensure_home_topic(&mut self, room_id: &str) -> Result<(), FiniteChatCoreError> {
        if !self.room_is_connected(room_id) || self.topic_exists(room_id, HOME_TOPIC_ID) {
            return Ok(());
        }
        if self.room_is_provisional_for_device_link(room_id) {
            // A provisional device-link room inherits the home topic from the
            // bootstrap history once the transfer completes; firing here
            // would append (or, under the deterministic key, conflict on) a
            // duplicate ConversationCreate on every sync tick.
            return Ok(());
        }
        let metadata = ConversationMetadataV1 {
            title: Some(HOME_TOPIC_TITLE.to_owned()),
            description: None,
            external_topic: None,
            skill_binding: None,
        };
        metadata.validate_limits().map_err(client_error)?;
        let payload = serde_json::to_vec(&metadata).map_err(client_error)?;
        let idempotency_key = home_topic_idempotency_key(room_id, self.core.device.device_ref());
        match self.core.send_application_event_with_idempotency_key(
            room_id,
            DurableAppEventKind::ConversationCreate,
            Some(HOME_TOPIC_ID.to_owned()),
            &payload,
            idempotency_key.clone(),
        ) {
            Ok(event) => self.apply_projection_events(vec![event])?,
            Err(FiniteChatCoreError::IdempotencyConflict { .. }) => {
                // The deterministic key proves this device already published
                // the room's home topic; the local projection lost the entry
                // (e.g. the event cache was pruned). A sender cannot decrypt
                // its own MLS application entries, so the log cannot restore
                // it — adopt the deterministic metadata locally instead of
                // appending a second entry the room might never process. The
                // adoption is in-memory only: nothing fabricated lands in the
                // durable event history.
                let adopted = StoredAppEvent {
                    room_id: room_id.to_owned(),
                    seq: self
                        .core
                        .device
                        .last_applied_seq(room_id)
                        .map_err(client_error)?,
                    message_id: format!("adopted-{idempotency_key}"),
                    sender: self.core.device.device_ref().clone(),
                    plaintext: encode_application_event(
                        DurableAppEventKind::ConversationCreate,
                        Some(HOME_TOPIC_ID.to_owned()),
                        &payload,
                    )?,
                    timestamp_unix_seconds: self.core.now_unix_seconds()?,
                };
                self.apply_projection_events(vec![adopted])?;
                if !self.topic_exists(room_id, HOME_TOPIC_ID) {
                    return Err(FiniteChatCoreError::ServerRejected {
                        reason: format!(
                            "room '{room_id}' already holds this device's home topic entry, but the local projection could not adopt it"
                        ),
                    });
                }
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn default_chat_route_for_room(
        &mut self,
        room_id: &str,
    ) -> Result<(String, String), FiniteChatCoreError> {
        if !self.room_is_connected(room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' is not ready to send"),
            });
        }
        self.ensure_home_topic(room_id)?;
        let chat_id = self
            .default_chat_id_for_topic(room_id, HOME_TOPIC_ID)
            .unwrap_or_else(|| HOME_CHAT_ID.to_owned());
        Ok((HOME_TOPIC_ID.to_owned(), chat_id))
    }

    #[allow(clippy::type_complexity)]
    fn scope_unscoped_chat_event_to_default(
        &mut self,
        room_id: &str,
        app_event_plaintext: Vec<u8>,
    ) -> Result<(Vec<u8>, Option<String>, Option<String>), FiniteChatCoreError> {
        let DecodedAppEvent::ChatMessage(decoded) = decode_application_event(&app_event_plaintext)
        else {
            return Ok((app_event_plaintext, None, None));
        };
        let projection = chat_projection_payload(&decoded.payload);
        let has_topic = decoded
            .conversation_id
            .as_deref()
            .or(projection.conversation_id.as_deref())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let has_chat = decoded
            .segment_id
            .as_deref()
            .or(projection.chat_id.as_deref())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        if has_topic || has_chat {
            return Ok((app_event_plaintext, None, None));
        }
        let (topic_id, chat_id) = self.default_chat_route_for_room(room_id)?;
        let scoped = encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            Some(topic_id.clone()),
            Some(chat_id.clone()),
            &decoded.raw_payload,
        )?;
        Ok((scoped, Some(topic_id), Some(chat_id)))
    }

    fn topic_exists(&self, room_id: &str, topic_id: &str) -> bool {
        self.app
            .topics
            .iter()
            .any(|topic| topic.room_id == room_id && topic.topic_id == topic_id && !topic.archived)
    }

    fn chat_exists(&self, room_id: &str, topic_id: &str, chat_id: &str) -> bool {
        self.app
            .topics
            .iter()
            .find(|topic| topic.room_id == room_id && topic.topic_id == topic_id && !topic.archived)
            .is_some_and(|topic| topic.chats.iter().any(|chat| chat.chat_id == chat_id))
    }

    fn default_chat_id_for_topic(&self, room_id: &str, topic_id: &str) -> Option<String> {
        let topic = self.app.topics.iter().find(|topic| {
            topic.room_id == room_id && topic.topic_id == topic_id && !topic.archived
        })?;
        if let Some(active_chat_id) = topic.active_chat_id.as_deref() {
            let active_is_archived = topic
                .chats
                .iter()
                .any(|chat| chat.chat_id == active_chat_id && chat.archived);
            if !active_is_archived {
                return Some(active_chat_id.to_owned());
            }
        }
        topic
            .chats
            .iter()
            .filter(|chat| !chat.archived)
            .max_by_key(|chat| chat.updated_seq)
            .or_else(|| topic.chats.first())
            .map(|chat| chat.chat_id.clone())
    }

    fn profile_chat_room_ids(&self, account_id: &str) -> Vec<String> {
        let current_account_id = &self.app.identity.account_id;
        let matches_profile_chat = |room_id: &str| {
            let Some(room) = self.app.rooms.iter().find(|room| room.room_id == room_id) else {
                return false;
            };
            if room.state != AppRoomState::Connected || !self.core.has_room(&room.room_id) {
                return false;
            }
            let Ok(members) = self.core.device.room_members(&room.room_id) else {
                return false;
            };
            let member_account_ids = members
                .into_iter()
                .map(|member| member.account_id)
                .collect::<BTreeSet<_>>();
            member_account_ids.len() == 2
                && member_account_ids.contains(current_account_id)
                && member_account_ids.contains(account_id)
        };

        let mut room_ids = self
            .app
            .rooms
            .iter()
            .filter(|room| matches_profile_chat(&room.room_id))
            .map(|room| room.room_id.clone())
            .collect::<Vec<_>>();
        room_ids.sort();
        room_ids
    }

    fn room(&self, room_id: &str) -> Option<&AppRoomSummary> {
        self.app.rooms.iter().find(|room| room.room_id == room_id)
    }

    fn room_mut(&mut self, room_id: &str) -> Option<&mut AppRoomSummary> {
        self.app
            .rooms
            .iter_mut()
            .find(|room| room.room_id == room_id)
    }

    fn validate_topic(&self, room_id: &str, topic_id: &str) -> Result<(), FiniteChatCoreError> {
        if self
            .app
            .topics
            .iter()
            .any(|topic| topic.room_id == room_id && topic.topic_id == topic_id && !topic.archived)
        {
            Ok(())
        } else {
            Err(FiniteChatCoreError::Client {
                reason: format!("topic '{topic_id}' is not available in room '{room_id}'"),
            })
        }
    }

    fn validate_chat_route(
        &self,
        room_id: &str,
        topic_id: &str,
        chat_id: &str,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_topic(room_id, topic_id)?;
        if self.chat_exists(room_id, topic_id, chat_id) {
            Ok(())
        } else {
            Err(FiniteChatCoreError::Client {
                reason: format!(
                    "chat '{chat_id}' is not available in topic '{topic_id}' in room '{room_id}'"
                ),
            })
        }
    }

    fn normalize_reply_target(
        &self,
        room_id: &str,
        reply_to_message_id: Option<String>,
    ) -> Result<Option<String>, FiniteChatCoreError> {
        let Some(reply_to_message_id) = reply_to_message_id else {
            return Ok(None);
        };
        let target_id = reply_to_message_id.trim();
        self.validate_reply_target(room_id, target_id)?;
        Ok(Some(target_id.to_owned()))
    }

    fn validate_reply_target(
        &self,
        room_id: &str,
        target_id: &str,
    ) -> Result<(), FiniteChatCoreError> {
        if target_id.is_empty() {
            return Err(FiniteChatCoreError::Client {
                reason: "reply target message id cannot be empty".to_owned(),
            });
        }
        if !self.chat_projection.message_exists(room_id, target_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("reply target '{target_id}' is not available in room '{room_id}'"),
            });
        }
        Ok(())
    }

    fn bump_rev(&mut self) {
        self.app.rev = self.app.rev.saturating_add(1);
    }
}

fn profile_from_record(
    record: finitechat_http::NostrProfileRecord,
    stale: bool,
) -> AppProfileSummary {
    let display_name = record
        .display_name
        .clone()
        .or_else(|| record.name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| short_account_label(&record.account_id));
    AppProfileSummary {
        npub: npub_encode(&record.account_id).unwrap_or_else(|_| record.account_id.clone()),
        is_agent: profile_record_is_agent(&record),
        account_id: record.account_id,
        display_name,
        about: record.about,
        picture: record.picture,
        stale,
    }
}

fn placeholder_profile(account_id: &str) -> AppProfileSummary {
    AppProfileSummary {
        account_id: account_id.to_owned(),
        npub: npub_encode(account_id).unwrap_or_else(|_| account_id.to_owned()),
        display_name: short_account_label(account_id),
        about: None,
        picture: None,
        stale: true,
        is_agent: false,
    }
}

fn profile_record_is_agent(record: &finitechat_http::NostrProfileRecord) -> bool {
    record.bot.unwrap_or(false)
        || record
            .finite_role
            .as_deref()
            .is_some_and(|role| role.eq_ignore_ascii_case("agent"))
}

fn profile_name_hint_from_chat_label(label: &str, account_id: &str) -> Option<String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return None;
    }
    let chat_prefix = "Chat with ";
    if let Some(name) = trimmed.strip_prefix(chat_prefix) {
        return normalize_profile_display_name_hint(Some(name), account_id);
    }
    normalize_profile_display_name_hint(Some(trimmed), account_id)
}

fn normalize_profile_display_name_hint(
    display_name_hint: Option<&str>,
    account_id: &str,
) -> Option<String> {
    let display_name = display_name_hint?.trim();
    if display_name.is_empty() || display_name == account_id {
        return None;
    }
    Some(display_name.to_owned())
}

fn profile_has_useful_name(profile: &AppProfileSummary, account_id: &str) -> bool {
    let name = profile.display_name.trim();
    !name.is_empty() && name != short_account_label(account_id) && name != account_id
}

fn normalize_profile_summary_hint(
    profile: AppProfileSummary,
) -> Result<AppProfileSummary, FiniteChatCoreError> {
    let account_id = profile.account_id.trim().to_owned();
    validate_string_bytes("profile.account_id", &account_id, MAX_OBJECT_ID_BYTES)
        .map_err(client_error)?;

    let npub = profile.npub.trim();
    let npub = if npub.is_empty() {
        npub_encode(&account_id).unwrap_or_else(|_| account_id.clone())
    } else {
        npub.to_owned()
    };

    let display_name = profile.display_name.trim();
    let display_name = if display_name.is_empty() {
        short_account_label(&account_id)
    } else {
        display_name.to_owned()
    };
    validate_string_bytes(
        "profile.display_name",
        &display_name,
        MAX_PROFILE_DISPLAY_NAME_BYTES,
    )
    .map_err(client_error)?;

    let about = normalize_optional_profile_text_option(profile.about);
    if let Some(about) = &about {
        validate_string_bytes("profile.about", about, MAX_PROFILE_ABOUT_BYTES)
            .map_err(client_error)?;
    }

    let picture = normalize_optional_profile_url(profile.picture)?;
    if let Some(picture) = &picture {
        validate_string_bytes("profile.picture", picture, MAX_PROFILE_PICTURE_BYTES)
            .map_err(client_error)?;
    }

    Ok(AppProfileSummary {
        account_id,
        npub,
        display_name,
        about,
        picture,
        stale: profile.stale,
        is_agent: profile.is_agent,
    })
}

fn merge_profile_summary_hint(
    existing: Option<&AppProfileSummary>,
    incoming: AppProfileSummary,
) -> AppProfileSummary {
    let Some(existing) = existing else {
        return incoming;
    };

    let account_id = existing.account_id.clone();
    let incoming_has_name = profile_has_useful_name(&incoming, &account_id);
    let existing_has_name = profile_has_useful_name(existing, &account_id);
    AppProfileSummary {
        account_id,
        npub: if incoming.npub.trim().is_empty() || incoming.npub == incoming.account_id {
            existing.npub.clone()
        } else {
            incoming.npub
        },
        display_name: if incoming_has_name || !existing_has_name {
            incoming.display_name
        } else {
            existing.display_name.clone()
        },
        about: incoming.about.or_else(|| existing.about.clone()),
        picture: incoming.picture.or_else(|| existing.picture.clone()),
        stale: existing.stale && incoming.stale,
        is_agent: existing.is_agent || incoming.is_agent,
    }
}

fn normalize_required_profile_text(
    field: &str,
    value: String,
) -> Result<String, FiniteChatCoreError> {
    let trimmed = value.trim().to_owned();
    if trimmed.is_empty() {
        return Err(FiniteChatCoreError::Client {
            reason: format!("{field} cannot be empty"),
        });
    }
    Ok(trimmed)
}

fn normalize_optional_profile_text(value: String) -> Option<String> {
    let trimmed = value.trim().to_owned();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn normalize_optional_profile_text_option(value: Option<String>) -> Option<String> {
    value.and_then(normalize_optional_profile_text)
}

fn normalize_optional_profile_url(
    value: Option<String>,
) -> Result<Option<String>, FiniteChatCoreError> {
    normalize_optional_http_url("profile picture URL", value)
}

fn normalize_optional_room_picture_url(
    value: Option<String>,
) -> Result<Option<String>, FiniteChatCoreError> {
    normalize_optional_http_url("room picture URL", value)
}

fn normalize_optional_http_url(
    field: &str,
    value: Option<String>,
) -> Result<Option<String>, FiniteChatCoreError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim().to_owned();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let url = reqwest::Url::parse(&trimmed).map_err(|error| FiniteChatCoreError::Client {
        reason: format!("{field} is invalid: {error}"),
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(FiniteChatCoreError::Client {
            reason: format!("{field} must be http(s)"),
        });
    }
    Ok(Some(trimmed))
}

fn profile_metadata_json_for_edit(
    existing: Option<&finitechat_http::NostrProfileRecord>,
    display_name: Option<&str>,
    about: Option<&str>,
    picture: Option<&str>,
    bot: Option<bool>,
    finite_role: Option<&str>,
) -> Result<Option<String>, FiniteChatCoreError> {
    let mut object = existing
        .and_then(|record| record.metadata_json.as_deref())
        .map(profile_metadata_json_object)
        .transpose()?
        .unwrap_or_default();

    patch_profile_metadata_string(&mut object, "name", display_name);
    patch_profile_metadata_string(&mut object, "display_name", display_name);
    object.remove("displayName");
    patch_profile_metadata_string(&mut object, "about", about);
    patch_profile_metadata_string(&mut object, "picture", picture);
    object.remove("picture_url");
    if let Some(bot) = bot {
        object.insert("bot".to_owned(), serde_json::Value::Bool(bot));
    } else {
        object.remove("bot");
    }
    patch_profile_metadata_string(&mut object, "finite_role", finite_role);
    object.remove("finiteRole");

    serde_json::to_string(&serde_json::Value::Object(object))
        .map(Some)
        .map_err(|error| FiniteChatCoreError::Client {
            reason: format!("profile metadata could not be encoded: {error}"),
        })
}

fn profile_metadata_json_object(
    metadata_json: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, FiniteChatCoreError> {
    let value: serde_json::Value =
        serde_json::from_str(metadata_json).map_err(|error| FiniteChatCoreError::Client {
            reason: format!("cached profile metadata is invalid JSON: {error}"),
        })?;
    match value {
        serde_json::Value::Object(object) => Ok(object),
        _ => Err(FiniteChatCoreError::Client {
            reason: "cached profile metadata must be a JSON object".to_owned(),
        }),
    }
}

fn patch_profile_metadata_string(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        object.insert(key.to_owned(), serde_json::Value::String(value.to_owned()));
    } else {
        object.remove(key);
    }
}

fn normalize_image_upload_content_type(
    content_type: &str,
) -> Result<&'static str, FiniteChatCoreError> {
    match content_type.trim().to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => Ok("image/jpeg"),
        "image/png" => Ok("image/png"),
        "image/gif" => Ok("image/gif"),
        "image/webp" => Ok("image/webp"),
        other => Err(FiniteChatCoreError::Client {
            reason: format!("public image content type is not supported: {other}"),
        }),
    }
}

fn validate_image_upload(bytes: &[u8], content_type: &str) -> Result<(), FiniteChatCoreError> {
    if bytes.is_empty() {
        return Err(FiniteChatCoreError::Client {
            reason: "public image cannot be empty".to_owned(),
        });
    }
    if bytes.len() > MAX_PUBLIC_IMAGE_UPLOAD_BYTES {
        return Err(FiniteChatCoreError::Client {
            reason: format!(
                "public image is too large: {} bytes, max {MAX_PUBLIC_IMAGE_UPLOAD_BYTES}",
                bytes.len()
            ),
        });
    }
    if image_magic_matches(bytes, content_type) {
        return Ok(());
    }
    Err(FiniteChatCoreError::Client {
        reason: format!("profile image bytes do not match {content_type}"),
    })
}

fn image_magic_matches(bytes: &[u8], content_type: &str) -> bool {
    match content_type {
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

fn room_member_summaries(
    members: Vec<DeviceRef>,
    profile_cache: &BTreeMap<String, AppProfileSummary>,
    current_device: &DeviceRef,
) -> Vec<AppRoomMemberSummary> {
    let mut summaries = members
        .into_iter()
        .map(|member| {
            let profile_name = profile_cache
                .get(&member.account_id)
                .map(|profile| profile.display_name.trim())
                .filter(|name| !name.is_empty());
            let picture = profile_cache
                .get(&member.account_id)
                .and_then(|profile| profile.picture.clone());
            let is_current_device = &member == current_device;
            let display_name = if is_current_device {
                "You".to_owned()
            } else if let Some(profile_name) = profile_name {
                profile_name.to_owned()
            } else {
                short_account_label(&member.account_id)
            };
            AppRoomMemberSummary {
                npub: npub_encode(&member.account_id).unwrap_or_else(|_| member.account_id.clone()),
                current_device: is_current_device,
                account_id: member.account_id,
                device_id: member.device_id,
                display_name,
                picture,
            }
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        right
            .current_device
            .cmp(&left.current_device)
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.account_id.cmp(&right.account_id))
            .then_with(|| left.device_id.cmp(&right.device_id))
    });
    summaries
}

fn stored_profile_from_app(profile: &AppProfileSummary) -> StoredAppProfile {
    StoredAppProfile {
        profile: finitechat_http::NostrProfileRecord {
            account_id: profile.account_id.clone(),
            name: None,
            display_name: Some(profile.display_name.clone()),
            about: profile.about.clone(),
            picture: profile.picture.clone(),
            bot: profile.is_agent.then_some(true),
            finite_role: profile.is_agent.then(|| "agent".to_owned()),
            metadata_json: None,
            fetched_at_ms: 0,
            expires_at_ms: 1,
        },
        stale: profile.stale,
    }
}

fn short_account_label(account_id: &str) -> String {
    let prefix_len = account_id.len().min(8);
    format!("npub {}", &account_id[..prefix_len])
}

fn app_device_key(account_id: &str, device_id: &str) -> String {
    format!("{account_id}/{device_id}")
}

fn normalize_app_update_wait_millis(timeout_millis: u64) -> u64 {
    if timeout_millis == 0 {
        return DEFAULT_APP_UPDATE_WAIT_MILLIS;
    }
    timeout_millis.clamp(MIN_APP_UPDATE_WAIT_MILLIS, MAX_APP_UPDATE_WAIT_MILLIS)
}

fn normalized_transcript_page_size(limit: u32) -> usize {
    let limit = if limit == 0 {
        MAX_TRANSCRIPT_PAGE_SIZE
    } else {
        limit.min(MAX_TRANSCRIPT_PAGE_SIZE)
    };
    usize::try_from(limit).expect("u32 transcript page limit fits usize")
}

fn recover_or_create_device_state(
    data_dir: &Path,
    account_secret: &NostrSecretKey,
    requested_config: FiniteChatDeviceConfig,
    explicit_account_secret: bool,
) -> Result<(SqliteClientStore, FiniteChatDeviceConfig), FiniteChatCoreError> {
    let db_path = data_dir.join(CLIENT_STORE_FILE);
    let account_id = hex::encode(account_secret.public_key().as_bytes());
    let mut requested_store = SqliteClientStore::open(
        &db_path,
        SqliteClientStoreOptions::from_nostr_secret(account_secret, &requested_config.device_id)
            .map_err(store_error)?,
    )
    .map_err(store_error)?;
    let stored_device_ids = requested_store
        .load_device_ids_for_account(&account_id)
        .map_err(store_error)?;

    if stored_device_ids.is_empty() || explicit_account_secret {
        let device = FiniteChatDevice::new(requested_config.clone()).map_err(client_error)?;
        requested_store
            .save_device_state(&device)
            .map_err(store_error)?;
        return Ok((requested_store, requested_config));
    }

    if stored_device_ids.len() == 1 {
        let mut recovered_config = requested_config;
        recovered_config.device_id = stored_device_ids[0].clone();
        let recovered_store = SqliteClientStore::open(
            db_path,
            SqliteClientStoreOptions::from_nostr_secret(
                account_secret,
                &recovered_config.device_id,
            )
            .map_err(store_error)?,
        )
        .map_err(store_error)?;
        return Ok((recovered_store, recovered_config));
    }

    Err(FiniteChatCoreError::Client {
        reason: format!(
            "device state not found for requested device '{}'; stored devices for this account are: {}",
            requested_config.device_id,
            stored_device_ids.join(", ")
        ),
    })
}

impl CoreState {
    fn open(options: OpenOptions) -> Result<Self, FiniteChatCoreError> {
        Self::open_with_mode(options, false)
    }

    fn open_read_only(options: OpenOptions) -> Result<Self, FiniteChatCoreError> {
        Self::open_with_mode(options, true)
    }

    fn open_with_mode(options: OpenOptions, read_only: bool) -> Result<Self, FiniteChatCoreError> {
        let requested_device_id = options.device_id.trim().to_owned();
        if requested_device_id.is_empty() {
            return Err(FiniteChatCoreError::Client {
                reason: "device id cannot be empty".to_owned(),
            });
        }
        let explicit_account_secret = options.account_secret_hex.is_some();

        let data_dir = PathBuf::from(options.data_dir);
        if !read_only {
            fs::create_dir_all(&data_dir).map_err(|error| FiniteChatCoreError::Filesystem {
                reason: format!("failed to create {}: {error}", data_dir.display()),
            })?;
        }

        let account_secret = resolve_account_secret(options.account_secret_hex.as_deref())?;
        let fixed_now_unix_seconds = options.now_unix_seconds;
        let now = fixed_now_unix_seconds.unwrap_or_else(current_unix_seconds);
        let mut config = FiniteChatDeviceConfig {
            account_secret_key: account_secret.clone(),
            device_id: requested_device_id.clone(),
            now_unix_seconds: now,
            credential_not_before_unix_seconds: now.saturating_sub(60),
            credential_not_after_unix_seconds: now
                .saturating_add(DEFAULT_CREDENTIAL_VALIDITY_SECONDS),
        };
        let store_options =
            SqliteClientStoreOptions::from_nostr_secret(&account_secret, &config.device_id)
                .map_err(store_error)?;
        let mut store = if read_only {
            SqliteClientStore::open_read_only(data_dir.join(CLIENT_STORE_FILE), store_options)
                .map_err(store_error)?
        } else {
            SqliteClientStore::open(data_dir.join(CLIENT_STORE_FILE), store_options)
                .map_err(store_error)?
        };
        let device = match store.load_device(config.clone()) {
            Ok(device) => device,
            Err(finitechat_client::ClientStoreError::DeviceStateNotFound { .. }) => {
                if read_only {
                    return Err(FiniteChatCoreError::Store {
                        reason: format!(
                            "client store {} has no state for device '{}' and was opened read-only; nothing can be minted without the writer lease",
                            data_dir.join(CLIENT_STORE_FILE).display(),
                            requested_device_id,
                        ),
                    });
                }
                let (next_store, recovered_config) = recover_or_create_device_state(
                    &data_dir,
                    &account_secret,
                    config,
                    explicit_account_secret,
                )?;
                store = next_store;
                config = recovered_config;
                store.load_device(config.clone()).map_err(store_error)?
            }
            Err(error) => return Err(store_error(error)),
        };

        Ok(Self {
            data_dir,
            server_url: options.server_url,
            account_secret,
            fixed_now_unix_seconds,
            store,
            device,
            read_only,
        })
    }

    fn identity(&self) -> Identity {
        let device = self.device.device_ref();
        Identity {
            account_id: device.account_id.clone(),
            device_id: device.device_id.clone(),
            account_secret_hex: hex::encode(self.account_secret.as_bytes()),
        }
    }

    fn now_unix_seconds(&self) -> Result<u64, FiniteChatCoreError> {
        Ok(self
            .fixed_now_unix_seconds
            .unwrap_or_else(current_unix_seconds))
    }

    fn reload_persisted_device(&mut self) -> Result<(), FiniteChatCoreError> {
        let config = FiniteChatDeviceConfig {
            account_secret_key: self.account_secret.clone(),
            device_id: self.device.device_ref().device_id.clone(),
            now_unix_seconds: self.now_unix_seconds()?,
            credential_not_before_unix_seconds: 0,
            credential_not_after_unix_seconds: u64::MAX,
        };
        self.device = self.store.load_device(config).map_err(store_error)?;
        Ok(())
    }

    fn refresh_device_clock(&mut self) -> Result<(), FiniteChatCoreError> {
        let now = self.now_unix_seconds()?;
        self.device.set_now_unix_seconds(now);
        Ok(())
    }

    fn now_millis(&self) -> Result<u64, FiniteChatCoreError> {
        self.now_unix_seconds()?
            .checked_mul(1000)
            .ok_or_else(|| FiniteChatCoreError::Client {
                reason: "clock overflow".to_owned(),
            })
    }

    fn home_delivery(&self) -> HttpRuntimeDelivery<ReqwestHttpRuntimeTransport> {
        self.delivery_for(&self.server_url)
    }

    fn delivery_for(&self, server_url: &str) -> HttpRuntimeDelivery<ReqwestHttpRuntimeTransport> {
        delivery_for_with_client(server_url, shared_blocking_http_client())
    }

    fn generate_object_id(&mut self, prefix: &str) -> Result<String, FiniteChatCoreError> {
        self.device.generate_object_id(prefix).map_err(client_error)
    }

    fn known_room_ids(&self) -> Vec<String> {
        self.device
            .room_sync_cursors()
            .into_iter()
            .map(|cursor| cursor.room_id)
            .collect()
    }

    fn has_room(&self, room_id: &str) -> bool {
        self.device.room_mls_group_id(room_id).is_ok()
    }

    fn bootstrap_room(
        &mut self,
        room_id: &str,
        display_name: Option<String>,
    ) -> Result<BootstrapRoomResult, FiniteChatCoreError> {
        if self.has_room(room_id) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("room '{room_id}' already exists on this device"),
            });
        }
        let app_room = app_room_metadata(room_id, display_name.as_deref());
        let mls_group_id = self.generate_object_id("mls")?;
        verify_server_contract(&self.server_url)?;
        let mut delivery = self.home_delivery();
        delivery
            .bootstrap_account_room(&CreateRoomRequest {
                room_id: room_id.to_owned(),
                mls_group_id: mls_group_id.clone(),
                creator: self.device.device_ref().clone(),
                protocol: RoomProtocol::default(),
            })
            .map_err(delivery_error)?;
        self.device
            .create_group_state(room_id, &mls_group_id)
            .map_err(client_error)?;
        self.store
            .save_device_state_and_app_rooms(&self.device, std::slice::from_ref(&app_room))
            .map_err(store_error)?;
        Ok(BootstrapRoomResult {
            room_id: room_id.to_owned(),
            mls_group_id,
        })
    }

    /// Apply an already-durable account-Room bootstrap request. The server
    /// endpoint accepts an exact replay, so a crash after server acceptance
    /// can retry this request before reconstructing the same local MLS group.
    fn bootstrap_room_from_request(
        &mut self,
        request: &CreateRoomRequest,
        display_name: Option<String>,
        fail_after_server_acceptance: bool,
    ) -> Result<BootstrapRoomResult, FiniteChatCoreError> {
        request.validate_limits().map_err(client_error)?;
        request.protocol.validate_limits().map_err(client_error)?;
        if request.creator != *self.device.device_ref()
            || request.protocol != RoomProtocol::default()
        {
            return Err(FiniteChatCoreError::Client {
                reason: "durable Room bootstrap request does not belong to this Device".to_owned(),
            });
        }
        let already_local = self.has_room(&request.room_id);
        if already_local
            && self
                .device
                .room_mls_group_id(&request.room_id)
                .map_err(client_error)?
                != request.mls_group_id
        {
            return Err(FiniteChatCoreError::Client {
                reason: "durable Room bootstrap request does not match local MLS state".to_owned(),
            });
        }

        verify_server_contract(&self.server_url)?;
        let mut delivery = self.home_delivery();
        delivery
            .bootstrap_account_room(request)
            .map_err(delivery_error)?;
        if fail_after_server_acceptance && !already_local {
            return Err(FiniteChatCoreError::Client {
                reason: "injected profile chat bootstrap failure after Room server acceptance"
                    .to_owned(),
            });
        }
        if !already_local {
            self.device
                .create_group_state(&request.room_id, &request.mls_group_id)
                .map_err(client_error)?;
        }
        let app_room = app_room_metadata(&request.room_id, display_name.as_deref());
        self.store
            .save_device_state_and_app_rooms(&self.device, std::slice::from_ref(&app_room))
            .map_err(store_error)?;
        Ok(BootstrapRoomResult {
            room_id: request.room_id.clone(),
            mls_group_id: request.mls_group_id.clone(),
        })
    }

    fn send_attachment(
        &mut self,
        input: SendAttachmentInput,
    ) -> Result<CoreSyncProjection, FiniteChatCoreError> {
        let SendAttachmentInput {
            room_id,
            conversation_id,
            chat_id,
            attachments: input_attachments,
            caption,
            reply_to_message_id,
        } = input;
        if input_attachments.is_empty() {
            return Err(FiniteChatCoreError::Client {
                reason: "attachment message must include at least one attachment".to_owned(),
            });
        }
        validate_item_count(
            "attachments",
            input_attachments.len(),
            MAX_ATTACHMENTS_PER_MESSAGE,
        )
        .map_err(client_error)?;
        let mut attachments = Vec::with_capacity(input_attachments.len());
        for attachment in input_attachments {
            attachments.push(self.upload_outbound_attachment(&room_id, attachment)?);
        }
        let chat_payload = encode_attachment_message_payload(
            caption.trim(),
            attachments,
            reply_to_message_id.as_deref(),
            conversation_id.as_deref(),
            chat_id.as_deref(),
        )?;
        let mut projection =
            self.send_chat_payload(&room_id, conversation_id, chat_id, chat_payload)?;
        self.apply_attachment_cache_paths(&mut projection.result.messages);
        Ok(projection)
    }

    fn upload_outbound_attachment(
        &self,
        room_id: &str,
        attachment: OutboundAttachment,
    ) -> Result<HermesAttachmentV1, FiniteChatCoreError> {
        let filename = attachment.filename.trim().to_owned();
        let mime_type = attachment.mime_type.trim().to_owned();
        let metadata = AttachmentBlobMetadataV1 {
            mime_type: mime_type.clone(),
            filename: filename.clone(),
            dimensions: None,
        };
        metadata.validate_limits().map_err(client_error)?;
        let room_server_url = self.room_server_url(room_id);
        let reference =
            self.upload_attachment_blob(&room_server_url, &attachment.bytes, metadata)?;
        self.cache_attachment_plaintext(&reference, &attachment.bytes)?;
        Ok(HermesAttachmentV1 {
            kind: hermes_attachment_kind(&attachment.kind),
            name: filename,
            mime_type,
            path: None,
            url: Some(reference.url.clone()),
            blob: Some(reference),
        })
    }

    fn send_chat_payload(
        &mut self,
        room_id: &str,
        conversation_id: Option<String>,
        chat_id: Option<String>,
        chat_payload: Vec<u8>,
    ) -> Result<CoreSyncProjection, FiniteChatCoreError> {
        let app_event_plaintext = encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            conversation_id,
            chat_id,
            &chat_payload,
        )?;
        self.send_application_plaintext(room_id, app_event_plaintext)
    }

    fn send_application_plaintext(
        &mut self,
        room_id: &str,
        app_event_plaintext: Vec<u8>,
    ) -> Result<CoreSyncProjection, FiniteChatCoreError> {
        let prepared = self.prepare_outbound_chat_message(room_id, app_event_plaintext)?;
        self.submit_outbox_message(&prepared.stored_message)
    }

    fn prepare_outbound_chat_message(
        &mut self,
        room_id: &str,
        app_event_plaintext: Vec<u8>,
    ) -> Result<PreparedOutboundMessage, FiniteChatCoreError> {
        let idempotency_key = self
            .device
            .generate_object_id("msg")
            .map_err(client_error)?;
        let timestamp_unix_seconds = self.now_unix_seconds()?;
        let request = self
            .device
            .create_application_request_at(
                room_id,
                &app_event_plaintext,
                idempotency_key,
                timestamp_unix_seconds,
            )
            .map_err(|error| send_error(room_id, error))?;
        let sender = request.sender.clone();
        let message_id = request.envelope.message_id().map_err(client_error)?;
        self.store
            .save_device_state(&self.device)
            .map_err(store_error)?;

        let mut chat_message = project_chat_message(
            room_id.to_owned(),
            u64::MAX,
            message_id.clone(),
            sender.clone(),
            app_event_plaintext.clone(),
            request.timestamp_unix_seconds,
            self.device.device_ref(),
        )
        .ok_or_else(|| FiniteChatCoreError::Client {
            reason: "local chat message did not project as a transcript row".to_owned(),
        })?;
        chat_message.outbound_delivery = Some(outbound_undelivered());
        let stored_message = StoredOutboundMessage {
            room_id: room_id.to_owned(),
            message_id,
            sender,
            plaintext: app_event_plaintext,
            local_state: StoredOutboundLocalState::Sent,
            server_delivery_state: StoredOutboundServerDeliveryState::Undelivered,
            append_request: request,
            timestamp_unix_seconds: chat_message.timestamp_unix_seconds,
        };
        Ok(PreparedOutboundMessage {
            chat_message,
            stored_message,
        })
    }

    fn submit_outbox_message(
        &mut self,
        message: &StoredOutboundMessage,
    ) -> Result<CoreSyncProjection, FiniteChatCoreError> {
        self.submit_outbox_message_with_acceptance(message)
            .map(|(_, projection)| projection)
    }

    fn submit_outbox_message_with_acceptance(
        &mut self,
        message: &StoredOutboundMessage,
    ) -> Result<(EventAccepted, CoreSyncProjection), FiniteChatCoreError> {
        let room_server_url = self.room_server_url(&message.room_id);
        let mut delivery = self.delivery_for(&room_server_url);
        let accepted = match delivery.append_event(
            &message.append_request,
            DurableAppEventKind::ChatMessage.delivery_policy(),
        ) {
            Ok(accepted) => accepted,
            Err(error) => return Err(send_delivery_error(error)),
        };
        if accepted.message_id != message.message_id {
            return Err(FiniteChatCoreError::Client {
                reason: format!(
                    "server accepted message id '{}' but outbox expected '{}'",
                    accepted.message_id, message.message_id
                ),
            });
        }

        let message = project_chat_message(
            message.room_id.clone(),
            accepted.seq,
            message.message_id.clone(),
            message.sender.clone(),
            message.plaintext.clone(),
            message.timestamp_unix_seconds,
            self.device.device_ref(),
        )
        .ok_or_else(|| FiniteChatCoreError::Client {
            reason: "sent chat message did not project as a transcript row".to_owned(),
        })?;
        self.persist_chat_messages_and_events(std::slice::from_ref(&message))?;
        let mut projection = match self.sync_with_projection() {
            Ok(projection) => projection,
            Err(FiniteChatCoreError::Delivery { .. }) => CoreSyncProjection::default(),
            Err(error) => return Err(error),
        };
        projection.result.messages.insert(0, message);
        Ok((accepted, projection))
    }

    fn upload_attachment_blob(
        &self,
        server_url: &str,
        plaintext: &[u8],
        metadata: AttachmentBlobMetadataV1,
    ) -> Result<AttachmentBlobReferenceV1, FiniteChatCoreError> {
        let prepared = prepare_attachment_upload(plaintext, metadata).map_err(client_error)?;
        let request = prepare_blossom_upload_http_request(&prepared).map_err(client_error)?;
        let upload_url = format!("{}{}", server_url.trim_end_matches('/'), request.path);
        let response = shared_blocking_http_client()
            .put(upload_url)
            .header(CONTENT_TYPE, request.content_type)
            .body(request.body.to_vec())
            .send()
            .map_err(delivery_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(delivery_error(format!(
                "blob upload failed with status {status}"
            )));
        }
        let descriptor = response.json::<BlobDescriptor>().map_err(delivery_error)?;
        finish_blossom_upload_http_response(
            &prepared,
            BlossomUploadHttpResponse {
                status: status.as_u16(),
                descriptor,
            },
        )
        .map_err(client_error)
    }

    fn upload_image_blob(
        &self,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<String, FiniteChatCoreError> {
        let content_type = normalize_image_upload_content_type(content_type)?;
        validate_image_upload(bytes, content_type)?;
        let upload_url = format!("{}/upload", self.server_url.trim_end_matches('/'));
        let response = shared_blocking_http_client()
            .put(upload_url)
            .header(CONTENT_TYPE, content_type)
            .body(bytes.to_vec())
            .send()
            .map_err(delivery_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(delivery_error(format!(
                "image upload failed with status {status}"
            )));
        }
        let descriptor = response.json::<BlobDescriptor>().map_err(delivery_error)?;
        let expected_sha256 = sha256_hex(bytes);
        if descriptor.sha256 != expected_sha256 {
            return Err(delivery_error(format!(
                "image upload hash mismatch: expected {expected_sha256}, got {}",
                descriptor.sha256
            )));
        }
        if descriptor.size_bytes != bytes.len() as u64 {
            return Err(delivery_error(format!(
                "image upload size mismatch: expected {}, got {}",
                bytes.len(),
                descriptor.size_bytes
            )));
        }
        normalize_optional_http_url("image upload URL", Some(descriptor.url))?.ok_or_else(|| {
            FiniteChatCoreError::Client {
                reason: "image upload returned an empty URL".to_owned(),
            }
        })
    }

    fn download_attachment_blob(
        &self,
        reference: &AttachmentBlobReferenceV1,
    ) -> Result<PathBuf, FiniteChatCoreError> {
        if let Some(path) = self.cached_attachment_path(reference)? {
            return Ok(path);
        }

        let request = prepare_blossom_download_http_request(reference).map_err(client_error)?;
        let response = shared_blocking_http_client()
            .get(request.url)
            .send()
            .map_err(delivery_error)?;
        let status = response.status();
        let body = response.bytes().map_err(delivery_error)?;
        let downloaded = finish_blossom_download_http_response(
            reference,
            BlossomDownloadHttpResponse {
                status: status.as_u16(),
                body: body.as_ref(),
            },
        )
        .map_err(client_error)?;
        self.cache_attachment_plaintext(reference, &downloaded.plaintext)?;
        self.cached_attachment_path(reference)?
            .ok_or_else(|| FiniteChatCoreError::Filesystem {
                reason: "attachment cache write did not produce a readable file".to_owned(),
            })
    }

    fn apply_attachment_cache_paths(&self, messages: &mut [ChatMessage]) {
        for message in messages {
            let references = attachment_references_by_id(message);
            for attachment in &mut message.media {
                let Some(reference) = references.get(&attachment.attachment_id) else {
                    continue;
                };
                attachment.local_path = self
                    .cached_attachment_path(reference)
                    .ok()
                    .flatten()
                    .map(|path| path.to_string_lossy().into_owned());
            }
        }
    }

    fn cached_attachment_path(
        &self,
        reference: &AttachmentBlobReferenceV1,
    ) -> Result<Option<PathBuf>, FiniteChatCoreError> {
        let path = self.attachment_cache_path(reference);
        if !path.is_file() {
            return Ok(None);
        }
        let plaintext = fs::read(&path).map_err(|error| FiniteChatCoreError::Filesystem {
            reason: format!("failed to read {}: {error}", path.display()),
        })?;
        if attachment_plaintext_matches(reference, &plaintext) {
            return Ok(Some(path));
        }
        fs::remove_file(&path).map_err(|error| FiniteChatCoreError::Filesystem {
            reason: format!(
                "failed to remove corrupt attachment cache {}: {error}",
                path.display()
            ),
        })?;
        Ok(None)
    }

    fn cache_attachment_plaintext(
        &self,
        reference: &AttachmentBlobReferenceV1,
        plaintext: &[u8],
    ) -> Result<PathBuf, FiniteChatCoreError> {
        if !attachment_plaintext_matches(reference, plaintext) {
            return Err(FiniteChatCoreError::Client {
                reason: "attachment plaintext does not match encrypted reference".to_owned(),
            });
        }
        let path = self.attachment_cache_path(reference);
        self.write_attachment_cache_file(&path, plaintext)
    }

    fn write_attachment_cache_file(
        &self,
        path: &Path,
        plaintext: &[u8],
    ) -> Result<PathBuf, FiniteChatCoreError> {
        let Some(parent) = path.parent() else {
            return Err(FiniteChatCoreError::Filesystem {
                reason: format!("attachment cache path has no parent: {}", path.display()),
            });
        };
        fs::create_dir_all(parent).map_err(|error| FiniteChatCoreError::Filesystem {
            reason: format!("failed to create {}: {error}", parent.display()),
        })?;
        let tmp_path = path.with_file_name(format!(
            ".{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("attachment")
        ));
        fs::write(&tmp_path, plaintext).map_err(|error| FiniteChatCoreError::Filesystem {
            reason: format!("failed to write {}: {error}", tmp_path.display()),
        })?;
        fs::rename(&tmp_path, path).map_err(|error| FiniteChatCoreError::Filesystem {
            reason: format!(
                "failed to move {} to {}: {error}",
                tmp_path.display(),
                path.display()
            ),
        })?;
        Ok(path.to_path_buf())
    }

    fn attachment_cache_path(&self, reference: &AttachmentBlobReferenceV1) -> PathBuf {
        self.local_attachment_cache_path(&reference.plaintext_sha256, &reference.metadata.filename)
    }

    fn local_attachment_cache_path(&self, plaintext_sha256: &str, filename: &str) -> PathBuf {
        self.data_dir
            .join(ATTACHMENT_CACHE_DIR)
            .join(plaintext_sha256)
            .join(sanitized_attachment_filename(filename))
    }

    fn room_server_url(&self, room_id: &str) -> String {
        self.device
            .room_server_url(room_id)
            .map(str::to_owned)
            .unwrap_or_else(|| self.server_url.clone())
    }

    fn send_typing_activity(
        &mut self,
        room_id: &str,
        is_typing: bool,
        now_ms: u64,
    ) -> Result<(), FiniteChatCoreError> {
        let activity = DecryptedEphemeralActivityV1 {
            activity_kind: FINITECHAT_ACTIVITY_KIND_TYPING.to_owned(),
            activity_id: None,
            segment_id: None,
            action: if is_typing {
                EphemeralActivityActionV1::Set
            } else {
                EphemeralActivityActionV1::Clear
            },
            payload: if is_typing {
                br#"{}"#.to_vec()
            } else {
                Vec::new()
            },
        };
        activity.validate_limits().map_err(client_error)?;
        let plaintext = serde_json::to_vec(&activity).map_err(client_error)?;
        let request = AppendEphemeralActivityRequest {
            room_id: room_id.to_owned(),
            mls_group_id: self
                .device
                .room_mls_group_id(room_id)
                .map_err(client_error)?,
            epoch: self.device.group_epoch(room_id).map_err(client_error)?,
            sender: self.device.device_ref().clone(),
            conversation_id: None,
            payload: self
                .device
                .encrypt_activity_payload(room_id, &plaintext)
                .map_err(client_error)?,
            received_at_ms: now_ms,
            expires_at_ms: now_ms
                .saturating_add(GenericActivityKindV1::Typing.recommended_expiry_millis()),
        };
        let room_server_url = self.room_server_url(room_id);
        let mut delivery = self.delivery_for(&room_server_url);
        delivery.append_activity(&request).map_err(delivery_error)?;
        Ok(())
    }

    fn append_ephemeral_activity(
        &mut self,
        input: AppBridgeActivityInput,
    ) -> Result<EphemeralActivityAccepted, FiniteChatCoreError> {
        let AppBridgeActivityInput {
            room_id,
            conversation_id,
            segment_id,
            activity_kind,
            activity_id,
            action,
            payload,
            expires_in_millis,
        } = input;
        let activity = DecryptedEphemeralActivityV1 {
            activity_kind: activity_kind.clone(),
            activity_id,
            segment_id,
            action,
            payload,
        };
        activity.validate_limits().map_err(client_error)?;
        let plaintext = serde_json::to_vec(&activity).map_err(client_error)?;
        let now_ms = self.now_millis()?;
        let expires_in_millis = if expires_in_millis == 0 {
            GenericActivityKindV1::from_activity_kind(&activity_kind)
                .map(|kind| kind.recommended_expiry_millis())
                .unwrap_or(DEFAULT_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS)
        } else {
            expires_in_millis
        };
        let request = AppendEphemeralActivityRequest {
            room_id: room_id.clone(),
            mls_group_id: self
                .device
                .room_mls_group_id(&room_id)
                .map_err(client_error)?,
            epoch: self.device.group_epoch(&room_id).map_err(client_error)?,
            sender: self.device.device_ref().clone(),
            conversation_id,
            payload: self
                .device
                .encrypt_activity_payload(&room_id, &plaintext)
                .map_err(client_error)?,
            received_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(expires_in_millis),
        };
        let room_server_url = self.room_server_url(&room_id);
        let mut delivery = self.delivery_for(&room_server_url);
        delivery.append_activity(&request).map_err(delivery_error)
    }

    fn get_ephemeral_activities(
        &self,
        room_id: &str,
        conversation_id: Option<&str>,
        now_ms: u64,
    ) -> Result<Vec<finitechat_proto::EphemeralActivityRecord>, FiniteChatCoreError> {
        let request = GetEphemeralActivitiesRequest {
            room_id: room_id.to_owned(),
            conversation_id: conversation_id.map(ToOwned::to_owned),
            requester: self.device.device_ref().clone(),
            now_ms,
        };
        let room_server_url = self.room_server_url(room_id);
        let mut delivery = self.delivery_for(&room_server_url);
        delivery
            .get_ephemeral_activities(&request)
            .map(|response| response.records)
            .map_err(delivery_error)
    }

    fn send_application_event(
        &mut self,
        room_id: &str,
        kind: DurableAppEventKind,
        conversation_id: Option<String>,
        payload: &[u8],
        idempotency_prefix: &str,
    ) -> Result<StoredAppEvent, FiniteChatCoreError> {
        self.send_application_event_with_segment(
            room_id,
            kind,
            conversation_id,
            None,
            payload,
            idempotency_prefix,
        )
    }

    fn send_application_event_with_idempotency_key(
        &mut self,
        room_id: &str,
        kind: DurableAppEventKind,
        conversation_id: Option<String>,
        payload: &[u8],
        idempotency_key: String,
    ) -> Result<StoredAppEvent, FiniteChatCoreError> {
        let app_event_plaintext =
            encode_application_event_with_segment(kind.clone(), conversation_id, None, payload)?;
        let timestamp_unix_seconds = self.now_unix_seconds()?;
        let request = self
            .device
            .create_application_request_at(
                room_id,
                &app_event_plaintext,
                idempotency_key,
                timestamp_unix_seconds,
            )
            .map_err(|error| send_error(room_id, error))?;
        let sender = request.sender.clone();
        self.store
            .save_device_state(&self.device)
            .map_err(store_error)?;
        let room_server_url = self
            .device
            .room_server_url(room_id)
            .map(str::to_owned)
            .unwrap_or_else(|| self.server_url.clone());
        let mut delivery = self.delivery_for(&room_server_url);
        let accepted = match delivery.append_event(&request, kind.delivery_policy()) {
            Ok(accepted) => accepted,
            Err(error) if is_idempotency_conflict_rejection(&error) => {
                return Err(FiniteChatCoreError::IdempotencyConflict {
                    reason: format!(
                        "room '{room_id}' already holds a different entry under this idempotency key"
                    ),
                });
            }
            Err(error) => return Err(delivery_error(error)),
        };
        let event = StoredAppEvent {
            room_id: room_id.to_owned(),
            seq: accepted.seq,
            message_id: accepted.message_id,
            sender,
            plaintext: app_event_plaintext,
            timestamp_unix_seconds: request.timestamp_unix_seconds,
        };
        self.store
            .import_app_events_atomically(self.device.device_ref(), std::slice::from_ref(&event))
            .map_err(store_error)?;
        Ok(event)
    }

    fn send_application_event_with_segment(
        &mut self,
        room_id: &str,
        kind: DurableAppEventKind,
        conversation_id: Option<String>,
        segment_id: Option<String>,
        payload: &[u8],
        idempotency_prefix: &str,
    ) -> Result<StoredAppEvent, FiniteChatCoreError> {
        let app_event_plaintext = encode_application_event_with_segment(
            kind.clone(),
            conversation_id,
            segment_id,
            payload,
        )?;
        let idempotency_key = self
            .device
            .generate_object_id(idempotency_prefix)
            .map_err(client_error)?;
        let timestamp_unix_seconds = self.now_unix_seconds()?;
        let request = self
            .device
            .create_application_request_at(
                room_id,
                &app_event_plaintext,
                idempotency_key,
                timestamp_unix_seconds,
            )
            .map_err(|error| send_error(room_id, error))?;
        let sender = request.sender.clone();
        self.store
            .save_device_state(&self.device)
            .map_err(store_error)?;

        let room_server_url = self
            .device
            .room_server_url(room_id)
            .map(str::to_owned)
            .unwrap_or_else(|| self.server_url.clone());
        let mut delivery = self.delivery_for(&room_server_url);
        let accepted = delivery
            .append_event(&request, kind.delivery_policy())
            .map_err(delivery_error)?;
        let event = StoredAppEvent {
            room_id: room_id.to_owned(),
            seq: accepted.seq,
            message_id: accepted.message_id,
            sender,
            plaintext: app_event_plaintext,
            timestamp_unix_seconds: request.timestamp_unix_seconds,
        };
        self.store
            .import_app_events_atomically(self.device.device_ref(), std::slice::from_ref(&event))
            .map_err(store_error)?;
        Ok(event)
    }

    fn send_reaction(
        &mut self,
        room_id: &str,
        target_message_id: &str,
        emoji: &str,
    ) -> Result<CoreSyncProjection, FiniteChatCoreError> {
        let reaction = ChatReactionV1 {
            target_message_id: target_message_id.to_owned(),
            emoji: emoji.trim().to_owned(),
        };
        reaction.validate_limits().map_err(client_error)?;
        let reaction_payload = serde_json::to_vec(&reaction).map_err(client_error)?;
        let app_event_plaintext =
            encode_application_event(DurableAppEventKind::ChatReaction, None, &reaction_payload)?;
        let idempotency_key = self
            .device
            .generate_object_id("reaction")
            .map_err(client_error)?;
        let timestamp_unix_seconds = self.now_unix_seconds()?;
        let request = self
            .device
            .create_application_request_at(
                room_id,
                &app_event_plaintext,
                idempotency_key,
                timestamp_unix_seconds,
            )
            .map_err(|error| send_error(room_id, error))?;
        let sender = request.sender.clone();
        self.store
            .save_device_state(&self.device)
            .map_err(store_error)?;

        let room_server_url = self
            .device
            .room_server_url(room_id)
            .map(str::to_owned)
            .unwrap_or_else(|| self.server_url.clone());
        let mut delivery = self.delivery_for(&room_server_url);
        let accepted = delivery
            .append_event(
                &request,
                DurableAppEventKind::ChatReaction.delivery_policy(),
            )
            .map_err(delivery_error)?;
        let event = StoredAppEvent {
            room_id: room_id.to_owned(),
            seq: accepted.seq,
            message_id: accepted.message_id,
            sender,
            plaintext: app_event_plaintext,
            timestamp_unix_seconds: request.timestamp_unix_seconds,
        };
        self.store
            .import_app_events_atomically(self.device.device_ref(), std::slice::from_ref(&event))
            .map_err(store_error)?;
        let mut projection = match self.sync_with_projection() {
            Ok(projection) => projection,
            Err(FiniteChatCoreError::Delivery { .. }) => CoreSyncProjection::default(),
            Err(error) => return Err(error),
        };
        projection.include_event(event);
        Ok(projection)
    }

    fn send_read_receipt(
        &mut self,
        room_id: &str,
        target_message_id: &str,
        target_seq: u64,
        state: ChatReceiptStateV1,
    ) -> Result<StoredAppEvent, FiniteChatCoreError> {
        let receipt = ChatReceiptV1 {
            target_message_id: target_message_id.to_owned(),
            target_seq,
            state,
        };
        receipt.validate_limits().map_err(client_error)?;
        let receipt_payload = serde_json::to_vec(&receipt).map_err(client_error)?;
        let app_event_plaintext =
            encode_application_event(DurableAppEventKind::ChatReceipt, None, &receipt_payload)?;
        let idempotency_key = self
            .device
            .generate_object_id("receipt")
            .map_err(client_error)?;
        let timestamp_unix_seconds = self.now_unix_seconds()?;
        let request = self
            .device
            .create_application_request_at(
                room_id,
                &app_event_plaintext,
                idempotency_key,
                timestamp_unix_seconds,
            )
            .map_err(|error| send_error(room_id, error))?;
        let sender = request.sender.clone();
        self.store
            .save_device_state(&self.device)
            .map_err(store_error)?;

        let room_server_url = self
            .device
            .room_server_url(room_id)
            .map(str::to_owned)
            .unwrap_or_else(|| self.server_url.clone());
        let mut delivery = self.delivery_for(&room_server_url);
        let accepted = delivery
            .append_event(&request, DurableAppEventKind::ChatReceipt.delivery_policy())
            .map_err(delivery_error)?;
        let event = StoredAppEvent {
            room_id: room_id.to_owned(),
            seq: accepted.seq,
            message_id: accepted.message_id,
            sender,
            plaintext: app_event_plaintext,
            timestamp_unix_seconds: request.timestamp_unix_seconds,
        };
        self.store
            .import_app_events_atomically(self.device.device_ref(), std::slice::from_ref(&event))
            .map_err(store_error)?;
        Ok(event)
    }

    fn send_poll_vote(
        &mut self,
        room_id: &str,
        poll_message_id: &str,
        option_id: &str,
    ) -> Result<StoredAppEvent, FiniteChatCoreError> {
        let vote = ChatPollVoteV1 {
            poll_message_id: poll_message_id.to_owned(),
            option_id: option_id.trim().to_owned(),
        };
        validate_poll_vote(&vote)?;
        let vote_payload = serde_json::to_vec(&vote).map_err(client_error)?;
        let kind = poll_vote_event_kind();
        let app_event_plaintext = encode_application_event(kind.clone(), None, &vote_payload)?;
        let idempotency_key = self
            .device
            .generate_object_id("poll-vote")
            .map_err(client_error)?;
        let timestamp_unix_seconds = self.now_unix_seconds()?;
        let request = self
            .device
            .create_application_request_at(
                room_id,
                &app_event_plaintext,
                idempotency_key,
                timestamp_unix_seconds,
            )
            .map_err(|error| send_error(room_id, error))?;
        let sender = request.sender.clone();
        self.store
            .save_device_state(&self.device)
            .map_err(store_error)?;

        let room_server_url = self
            .device
            .room_server_url(room_id)
            .map(str::to_owned)
            .unwrap_or_else(|| self.server_url.clone());
        let mut delivery = self.delivery_for(&room_server_url);
        let accepted = delivery
            .append_event(&request, kind.delivery_policy())
            .map_err(delivery_error)?;
        let event = StoredAppEvent {
            room_id: room_id.to_owned(),
            seq: accepted.seq,
            message_id: accepted.message_id,
            sender,
            plaintext: app_event_plaintext,
            timestamp_unix_seconds: request.timestamp_unix_seconds,
        };
        self.store
            .import_app_events_atomically(self.device.device_ref(), std::slice::from_ref(&event))
            .map_err(store_error)?;
        Ok(event)
    }

    fn sync_room_with_projection(
        &mut self,
        room_id: &str,
    ) -> Result<CoreSyncProjection, FiniteChatCoreError> {
        let Some(cursor) = self
            .device
            .room_sync_cursors()
            .into_iter()
            .find(|cursor| cursor.room_id == room_id)
        else {
            return self.sync_with_projection();
        };
        let options = RuntimeSyncOptions {
            key_package_target_available: DEFAULT_KEY_PACKAGE_TARGET_AVAILABLE,
            max_sync_pages_per_room: DEFAULT_MAX_SYNC_PAGES_PER_ROOM,
        };
        let owner = self.device.device_ref().clone();
        let server_url = cursor.server_url.unwrap_or_else(|| self.server_url.clone());
        let mut delivery = self.delivery_for(&server_url);
        let mut projection = CoreSyncProjection::default();
        match run_room_sync_tick(
            &mut self.store,
            &mut self.device,
            &mut delivery,
            &options,
            room_id,
        ) {
            Ok(report) => projection.merge_report(report, &owner),
            Err(error) => {
                self.reload_persisted_device()?;
                let quarantined_room_failure = matches!(
                    &error,
                    RuntimeWorkerError::Client(_)
                        | RuntimeWorkerError::ClientStore(ClientStoreError::Client(_))
                );
                if quarantined_room_failure {
                    projection.room_sync_failures.push(room_id.to_owned());
                } else {
                    return Err(runtime_error(error));
                }
            }
        }
        Ok(projection)
    }

    fn sync_with_projection(&mut self) -> Result<CoreSyncProjection, FiniteChatCoreError> {
        let options = RuntimeSyncOptions {
            key_package_target_available: DEFAULT_KEY_PACKAGE_TARGET_AVAILABLE,
            max_sync_pages_per_room: DEFAULT_MAX_SYNC_PAGES_PER_ROOM,
        };
        let mut projection = CoreSyncProjection::default();

        let owner = self.device.device_ref().clone();
        let mut first_error = None;
        let mut home_delivery = self.home_delivery();
        match run_runtime_sync_setup_tick(
            &mut self.store,
            &mut self.device,
            &mut home_delivery,
            &options,
        ) {
            Ok(report) => projection.merge_report(report, &owner),
            Err(error) => {
                first_error.get_or_insert_with(|| runtime_error(error));
            }
        };

        let room_servers = self
            .device
            .room_sync_cursors()
            .into_iter()
            .filter_map(|cursor| cursor.server_url)
            .collect::<BTreeSet<_>>();
        for server_url in room_servers {
            let mut delivery = self.delivery_for(&server_url);
            match run_room_server_sync_setup_tick(
                &mut self.store,
                &mut self.device,
                &mut delivery,
                &options,
            ) {
                Ok(report) => projection.merge_report(report, &owner),
                Err(error) => {
                    first_error.get_or_insert_with(|| runtime_error(error));
                }
            }
        }

        // MLS failures are scoped to one room. A failed room attempt may have
        // changed the in-memory Device, so restore the last durable snapshot
        // before continuing to later rooms.
        // `run_room_sync_tick` persists only after the full bounded room tick
        // succeeds, so a failed room keeps its prior durable cursor and MLS
        // state while healthy rooms continue.
        for cursor in self.device.room_sync_cursors() {
            let server_url = cursor.server_url.unwrap_or_else(|| self.server_url.clone());
            let mut delivery = self.delivery_for(&server_url);
            match run_room_sync_tick(
                &mut self.store,
                &mut self.device,
                &mut delivery,
                &options,
                &cursor.room_id,
            ) {
                Ok(report) => projection.merge_report(report, &owner),
                Err(error) => {
                    self.reload_persisted_device()?;
                    let quarantined_room_failure = matches!(
                        &error,
                        RuntimeWorkerError::Client(_)
                            | RuntimeWorkerError::ClientStore(ClientStoreError::Client(_))
                    );
                    if quarantined_room_failure {
                        projection.room_sync_failures.push(cursor.room_id);
                    } else {
                        first_error.get_or_insert_with(|| runtime_error(error));
                    }
                }
            }
        }

        if let Some(error) = first_error
            && !projection.has_progress()
        {
            return Err(error);
        }

        Ok(projection)
    }

    fn persist_chat_messages_and_events(
        &mut self,
        messages: &[ChatMessage],
    ) -> Result<(), FiniteChatCoreError> {
        if messages.is_empty() {
            return Ok(());
        }
        let owner = self.device.device_ref().clone();
        let stored_messages = messages
            .iter()
            .map(stored_message_from_chat)
            .collect::<Vec<_>>();
        let stored_events = messages
            .iter()
            .map(stored_event_from_chat)
            .collect::<Vec<_>>();
        self.store
            .save_app_messages_and_events(
                &owner,
                &stored_messages,
                &stored_events,
                MAX_APP_MESSAGES_U32,
            )
            .map_err(store_error)
    }
}

impl CoreSyncProjection {
    fn include_event(&mut self, event: StoredAppEvent) {
        if !self.events.iter().any(|existing| {
            existing.room_id == event.room_id && existing.message_id == event.message_id
        }) {
            self.events.push(event);
            self.events.sort_by(|left, right| {
                left.room_id
                    .cmp(&right.room_id)
                    .then_with(|| left.seq.cmp(&right.seq))
                    .then_with(|| left.message_id.cmp(&right.message_id))
            });
        }
    }

    fn has_progress(&self) -> bool {
        self.result.uploaded_key_packages > 0
            || self.result.claimed_welcomes > 0
            || self.result.activated_welcome_acks_sent > 0
            || self.result.sync_pages > 0
            || !self.result.messages.is_empty()
            || !self.events.is_empty()
    }

    fn merge_report(&mut self, report: finitechat_client::RuntimeSyncReport, owner: &DeviceRef) {
        self.result.uploaded_key_packages = self
            .result
            .uploaded_key_packages
            .saturating_add(report.uploaded_key_packages);
        self.result.claimed_welcomes = self
            .result
            .claimed_welcomes
            .saturating_add(report.claimed_welcomes);
        self.result.activated_welcome_acks_sent = self
            .result
            .activated_welcome_acks_sent
            .saturating_add(report.activated_welcome_acks_sent);
        self.result.sync_pages = self.result.sync_pages.saturating_add(report.sync_pages);
        for entry in report.applied_entries {
            match entry.entry {
                AppliedLogEntry::Application { plaintext, sender } => {
                    if let Some(message) = project_chat_message(
                        entry.room_id.clone(),
                        entry.seq,
                        entry.message_id.clone(),
                        sender.clone(),
                        plaintext.clone(),
                        entry.timestamp_unix_seconds,
                        owner,
                    ) {
                        self.result.messages.push(message);
                    }
                    self.events.push(StoredAppEvent {
                        room_id: entry.room_id,
                        seq: entry.seq,
                        message_id: entry.message_id,
                        sender,
                        plaintext,
                        timestamp_unix_seconds: entry.timestamp_unix_seconds,
                    });
                }
                AppliedLogEntry::Commit { .. } => {}
            }
        }
    }
}

fn app_bridge_event_from_stored_event(event: &StoredAppEvent) -> AppBridgeAppliedEvent {
    AppBridgeAppliedEvent {
        room_id: event.room_id.clone(),
        seq: event.seq,
        message_id: event.message_id.clone(),
        sender_account_id: event.sender.account_id.clone(),
        sender_device_id: event.sender.device_id.clone(),
        plaintext: event.plaintext.clone(),
    }
}

#[cfg(test)]
fn chat_display_text(plaintext: &[u8]) -> String {
    chat_projection_payload_from_application_plaintext(plaintext)
        .map(|payload| payload.text)
        .unwrap_or_default()
}

// One decoded event is held at a time and moved, not stored in bulk, so
// the chat-message variant's size is not worth boxing every match over.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
enum DecodedAppEvent {
    ChatMessage(DecodedChatMessage),
    ChatReaction(ChatReactionV1),
    ChatReceipt(ChatReceiptV1),
    PollVote(ChatPollVoteV1),
    ChatArchive(ChatArchiveV1),
    ChatRename(ChatRenameV1),
    Ignored,
}

/// A decoded `finitechat` chat-message application event. This is the single
/// interpreted form of an application plaintext: `decode_application_event`
/// resolves the payload once, and every projection consumes these fields
/// instead of re-parsing the raw bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DecodedChatMessage {
    /// Conversation scoping from the application-event envelope.
    conversation_id: Option<String>,
    /// Segment scoping from the application-event envelope.
    segment_id: Option<String>,
    /// The envelope's payload bytes, retained for wire-faithful re-use
    /// (e.g. re-scoping an unscoped outbound event).
    raw_payload: Vec<u8>,
    /// The payload resolved to its typed form exactly once at decode time.
    payload: ChatMessagePayload,
}

/// Envelope scoping for one decoded chat-message event. Conversation
/// projection contexts are envelope-first while projected messages are
/// payload-first; carrying both values separately preserves that.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ChatEventScope {
    conversation_id: Option<String>,
    segment_id: Option<String>,
}

impl ChatEventScope {
    fn from_decoded(decoded: &DecodedAppEvent) -> Self {
        match decoded {
            DecodedAppEvent::ChatMessage(decoded) => Self {
                conversation_id: decoded.conversation_id.clone(),
                segment_id: decoded.segment_id.clone(),
            },
            _ => Self::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ChatMessagePayload {
    Poll(ChatPollPayloadV1),
    Hermes(HermesMessagePayloadV1),
    /// Anything else (non-JSON traffic, unknown payload types, payloads that
    /// fail validation): projected as plain lossy text.
    Raw(Vec<u8>),
}

/// Resolve a chat-message payload with a single JSON pass: parse the bytes
/// into one `Value`, dispatch on its `"type"` tag, and convert only the
/// matching typed form. Callers previously did this twice — the poll probe
/// and the hermes probe each fully parsed the payload just to read the tag.
fn resolve_chat_message_payload(payload_bytes: &[u8]) -> ChatMessagePayload {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(payload_bytes) else {
        return ChatMessagePayload::Raw(payload_bytes.to_vec());
    };
    if value.get("type").and_then(serde_json::Value::as_str)
        == Some(FINITECHAT_POLL_PAYLOAD_TYPE_V1)
    {
        return serde_json::from_value::<ChatPollPayloadV1>(value)
            .ok()
            .filter(|poll| validate_poll_payload(poll).is_ok())
            .map(ChatMessagePayload::Poll)
            .unwrap_or_else(|| ChatMessagePayload::Raw(payload_bytes.to_vec()));
    }
    match HermesMessagePayloadV1::decode_value(value) {
        Ok(Some(hermes)) => ChatMessagePayload::Hermes(hermes),
        Ok(None) | Err(_) => ChatMessagePayload::Raw(payload_bytes.to_vec()),
    }
}

struct ChatProjectionPayload {
    text: String,
    display_content: String,
    metadata: BTreeMap<String, serde_json::Value>,
    kind: ChatMessageKind,
    status: ChatMessageStatus,
    final_delivery: bool,
    edit_of_message_id: Option<String>,
    conversation_id: Option<String>,
    chat_id: Option<String>,
    reply_to_message_id: Option<String>,
    sender_name: Option<String>,
    media: Vec<ChatMediaAttachment>,
    poll: Option<ChatPoll>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ChatPollPayloadV1 {
    #[serde(rename = "type")]
    payload_type: String,
    question: String,
    options: Vec<ChatPollPayloadOptionV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ChatPollPayloadOptionV1 {
    option_id: String,
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ChatPollVoteV1 {
    poll_message_id: String,
    option_id: String,
}

fn project_chat_message(
    room_id: String,
    seq: u64,
    message_id: String,
    sender: DeviceRef,
    plaintext: Vec<u8>,
    timestamp_unix_seconds: u64,
    owner: &DeviceRef,
) -> Option<ChatMessage> {
    let event = StoredAppEvent {
        room_id,
        seq,
        message_id,
        sender,
        plaintext,
        timestamp_unix_seconds,
    };
    let decoded = decode_application_event(&event.plaintext);
    project_decoded_chat_message(event, decoded, owner)
}

/// Project a chat message from an already-decoded application event, so
/// callers on the replay path decode once instead of re-parsing the
/// envelope and payload per projection.
fn project_decoded_chat_message(
    event: StoredAppEvent,
    decoded: DecodedAppEvent,
    owner: &DeviceRef,
) -> Option<ChatMessage> {
    let DecodedAppEvent::ChatMessage(decoded_message) = decoded else {
        return None;
    };
    let StoredAppEvent {
        room_id,
        seq,
        message_id,
        sender,
        plaintext,
        timestamp_unix_seconds,
    } = event;
    let projection = chat_projection_for_decoded(decoded_message);
    // Product authorship is account-scoped: another Device enrolled under the
    // same Principal is still "you". Delivery state, however, belongs only to
    // the Device that actually authored this local outbound message.
    let is_mine = sender.account_id == owner.account_id;
    let authored_by_current_device = sender == *owner;
    let sender_npub = npub_encode(&sender.account_id).ok();
    let rich_text_json = chat_rich_text_json(chat_message_body_text(
        &projection.text,
        &projection.display_content,
    ));
    Some(ChatMessage {
        room_id,
        seq,
        message_id,
        conversation_id: projection.conversation_id,
        chat_id: projection.chat_id,
        sender_account_id: sender.account_id.clone(),
        sender_device_id: sender.device_id.clone(),
        sender_display_name: sender_display_name(
            &sender,
            projection.sender_name.as_deref(),
            is_mine,
        ),
        sender_npub,
        text: projection.text,
        display_content: projection.display_content,
        rich_text_json,
        metadata_json: chat_metadata_json(&projection.metadata),
        kind: projection.kind,
        status: projection.status,
        final_delivery: projection.final_delivery,
        edit_of_message_id: projection.edit_of_message_id,
        payload: plaintext,
        reply_to_message_id: projection.reply_to_message_id,
        is_mine,
        outbound_delivery: authored_by_current_device.then(outbound_delivered),
        reactions: Vec::new(),
        media: projection.media,
        read_receipt: None,
        poll: projection.poll,
        timestamp_unix_seconds,
        display_timestamp: display_timestamp(timestamp_unix_seconds),
    })
}

fn outbound_undelivered() -> OutboundDelivery {
    OutboundDelivery {
        local_send: OutboundLocalSendState::Sent,
        server_delivery: OutboundServerDeliveryState::Undelivered,
    }
}

fn outbound_delivered() -> OutboundDelivery {
    OutboundDelivery {
        local_send: OutboundLocalSendState::Sent,
        server_delivery: OutboundServerDeliveryState::Delivered,
    }
}

fn display_timestamp(timestamp_unix_seconds: u64) -> String {
    if timestamp_unix_seconds == 0 {
        return String::new();
    }
    let Ok(timestamp) = i64::try_from(timestamp_unix_seconds) else {
        return String::new();
    };
    let Ok(utc) = OffsetDateTime::from_unix_timestamp(timestamp) else {
        return String::new();
    };
    let datetime = utc.to_offset(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC));
    let hour = datetime.hour();
    let hour_12 = match hour % 12 {
        0 => 12,
        value => value,
    };
    let period = if hour < 12 { "AM" } else { "PM" };
    format!("{hour_12}:{:02} {period}", datetime.minute())
}

fn chat_message_body_text<'a>(text: &'a str, display_content: &'a str) -> &'a str {
    let display = display_content.trim();
    if !display.is_empty() {
        display
    } else {
        text.trim()
    }
}

fn chat_rich_text_json(body_text: &str) -> String {
    let body_text = body_text.trim();
    if body_text.is_empty() {
        return String::new();
    }
    hypernote_mdx::serialize_tree(&hypernote_mdx::parse(body_text))
}

#[cfg(test)]
fn chat_projection_payload_from_application_plaintext(
    plaintext: &[u8],
) -> Option<ChatProjectionPayload> {
    match decode_application_event(plaintext) {
        DecodedAppEvent::ChatMessage(decoded) => Some(chat_projection_for_decoded(decoded)),
        DecodedAppEvent::ChatReaction(_)
        | DecodedAppEvent::ChatReceipt(_)
        | DecodedAppEvent::PollVote(_)
        | DecodedAppEvent::ChatArchive(_)
        | DecodedAppEvent::ChatRename(_)
        | DecodedAppEvent::Ignored => None,
    }
}

/// Merge a decoded chat message into its projection fields: the payload's
/// own scoping wins, with the envelope's scoping as fallback, then default
/// scoping applies. This precedence is product behavior — the message's
/// conversation is payload-first while conversation projection contexts are
/// envelope-first — and must not be silently unified.
fn chat_projection_for_decoded(decoded: DecodedChatMessage) -> ChatProjectionPayload {
    let mut projection = chat_projection_payload(&decoded.payload);
    if projection.conversation_id.is_none() {
        projection.conversation_id = decoded.conversation_id;
    }
    if projection.chat_id.is_none() {
        projection.chat_id = decoded.segment_id;
    }
    apply_default_chat_projection_scope(&mut projection);
    projection
}

fn apply_default_chat_projection_scope(projection: &mut ChatProjectionPayload) {
    if projection
        .conversation_id
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
    {
        projection.conversation_id = Some(HOME_TOPIC_ID.to_owned());
    }
    if projection
        .chat_id
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
        && projection.conversation_id.as_deref() == Some(HOME_TOPIC_ID)
    {
        projection.chat_id = Some(HOME_CHAT_ID.to_owned());
    }
}

fn chat_projection_payload(payload: &ChatMessagePayload) -> ChatProjectionPayload {
    match payload {
        ChatMessagePayload::Poll(payload) => {
            let question = payload.question.clone();
            ChatProjectionPayload {
                display_content: question.clone(),
                text: question,
                metadata: BTreeMap::new(),
                kind: ChatMessageKind::Message,
                status: ChatMessageStatus::Complete,
                final_delivery: false,
                edit_of_message_id: None,
                conversation_id: None,
                chat_id: None,
                reply_to_message_id: None,
                sender_name: None,
                media: Vec::new(),
                poll: Some(chat_poll_from_payload(payload.clone())),
            }
        }
        ChatMessagePayload::Hermes(payload) => {
            let final_delivery = payload
                .metadata
                .get("notify")
                .and_then(serde_json::Value::as_bool)
                == Some(true);
            ChatProjectionPayload {
                display_content: payload.text.clone(),
                text: payload.text.clone(),
                metadata: surfaced_message_metadata(payload.metadata.clone()),
                kind: chat_message_kind(payload.kind),
                status: chat_message_status(payload.status),
                final_delivery,
                edit_of_message_id: payload.edit_of.clone(),
                conversation_id: payload.conversation_id.clone(),
                chat_id: payload.segment_id.clone(),
                reply_to_message_id: payload.reply_to_message_id.clone(),
                sender_name: payload.sender_name.clone(),
                media: payload
                    .attachments
                    .clone()
                    .into_iter()
                    .enumerate()
                    .map(|(index, attachment)| chat_media_attachment(index, attachment))
                    .collect(),
                poll: None,
            }
        }
        ChatMessagePayload::Raw(payload_bytes) => {
            let text = String::from_utf8_lossy(payload_bytes).into_owned();
            ChatProjectionPayload {
                display_content: text.clone(),
                text,
                metadata: BTreeMap::new(),
                kind: ChatMessageKind::Message,
                status: ChatMessageStatus::Complete,
                final_delivery: false,
                edit_of_message_id: None,
                conversation_id: None,
                chat_id: None,
                reply_to_message_id: None,
                sender_name: None,
                media: Vec::new(),
                poll: None,
            }
        }
    }
}

/// Payload metadata keys that never surface on the projected message.
/// Requester credentials are transport claims for the receiving service,
/// not message content for renderers.
fn surfaced_message_metadata(
    metadata: BTreeMap<String, serde_json::Value>,
) -> BTreeMap<String, serde_json::Value> {
    let mut surfaced = metadata;
    surfaced.remove(finitechat_hermes::HERMES_METADATA_REQUESTER_EMAIL);
    surfaced.remove(finitechat_hermes::HERMES_METADATA_SITES_REQUESTER_ASSERTION);
    surfaced
}

fn chat_metadata_json(metadata: &BTreeMap<String, serde_json::Value>) -> String {
    if metadata.is_empty() {
        return String::new();
    }
    serde_json::to_string(metadata).unwrap_or_default()
}

fn chat_message_kind(kind: HermesSendKindV1) -> ChatMessageKind {
    match kind {
        HermesSendKindV1::Message => ChatMessageKind::Message,
        HermesSendKindV1::Status => ChatMessageKind::Status,
        HermesSendKindV1::Tool => ChatMessageKind::Tool,
        HermesSendKindV1::Media => ChatMessageKind::Media,
    }
}

fn chat_message_status(status: HermesMessageStatusV1) -> ChatMessageStatus {
    match status {
        HermesMessageStatusV1::Running => ChatMessageStatus::Running,
        HermesMessageStatusV1::Complete => ChatMessageStatus::Complete,
    }
}

/// A stored application event paired with its single-decode interpretation.
/// `ChatProjectionState` consumes only this form, so application plaintext
/// is parsed exactly once per event, at the door — future callers cannot
/// hand the projection raw bytes to re-interpret.
struct ParsedAppEvent {
    event: StoredAppEvent,
    decoded: DecodedAppEvent,
    /// The envelope when the plaintext parses as an application event,
    /// regardless of limits validity, for consumers that need envelope
    /// fields the decoded form does not carry (conversation lifecycle
    /// payloads, durable-terminal activity clears).
    envelope: Option<DecryptedApplicationEventV1>,
}

impl ParsedAppEvent {
    fn decode(event: StoredAppEvent) -> Self {
        let (decoded, envelope) = decode_application_plaintext(&event.plaintext);
        Self {
            event,
            decoded,
            envelope,
        }
    }
}

fn decode_application_event(plaintext: &[u8]) -> DecodedAppEvent {
    decode_application_plaintext(plaintext).0
}

/// The single interpreter of application plaintext: parse the envelope at
/// most once and resolve the decoded form from it.
fn decode_application_plaintext(
    plaintext: &[u8],
) -> (DecodedAppEvent, Option<DecryptedApplicationEventV1>) {
    match serde_json::from_slice::<DecryptedApplicationEventV1>(plaintext) {
        Ok(envelope) => {
            let decoded = decoded_typed_application_event(&envelope);
            (decoded, Some(envelope))
        }
        Err(_) => (
            DecodedAppEvent::ChatMessage(DecodedChatMessage {
                conversation_id: None,
                segment_id: None,
                raw_payload: plaintext.to_vec(),
                // An envelope-less payload can still be a typed chat payload
                // (e.g. a bare hermes message); resolve it the same way.
                payload: resolve_chat_message_payload(plaintext),
            }),
            None,
        ),
    }
}

fn decoded_typed_application_event(event: &DecryptedApplicationEventV1) -> DecodedAppEvent {
    if event.validate_limits().is_err() {
        return DecodedAppEvent::Ignored;
    }
    match &event.kind {
        DurableAppEventKind::ChatMessage => {
            let raw_payload = event.payload.clone();
            let payload = resolve_chat_message_payload(&raw_payload);
            DecodedAppEvent::ChatMessage(DecodedChatMessage {
                conversation_id: event.conversation_id.clone(),
                segment_id: event.segment_id.clone(),
                raw_payload,
                payload,
            })
        }
        DurableAppEventKind::ChatReaction => {
            serde_json::from_slice::<ChatReactionV1>(&event.payload)
                .ok()
                .filter(|reaction| reaction.validate_limits().is_ok())
                .map(DecodedAppEvent::ChatReaction)
                .unwrap_or(DecodedAppEvent::Ignored)
        }
        DurableAppEventKind::ChatReceipt => serde_json::from_slice::<ChatReceiptV1>(&event.payload)
            .ok()
            .filter(|receipt| receipt.validate_limits().is_ok())
            .map(DecodedAppEvent::ChatReceipt)
            .unwrap_or(DecodedAppEvent::Ignored),
        DurableAppEventKind::Namespaced { name, policy }
            if name == FINITECHAT_POLL_VOTE_EVENT_V1
                && *policy == ApplicationDeliveryPolicy::NON_NOTIFYING =>
        {
            serde_json::from_slice::<ChatPollVoteV1>(&event.payload)
                .ok()
                .filter(|vote| validate_poll_vote(vote).is_ok())
                .map(DecodedAppEvent::PollVote)
                .unwrap_or(DecodedAppEvent::Ignored)
        }
        DurableAppEventKind::Namespaced { name, policy }
            if name == FINITECHAT_CHAT_ARCHIVE_EVENT_V1
                && *policy == ApplicationDeliveryPolicy::NON_NOTIFYING =>
        {
            serde_json::from_slice::<ChatArchiveV1>(&event.payload)
                .ok()
                .filter(|archive| archive.validate_limits().is_ok())
                .filter(|archive| {
                    event.conversation_id.as_deref() == Some(archive.topic_id.as_str())
                })
                .filter(|archive| event.segment_id.as_deref() == Some(archive.chat_id.as_str()))
                .map(DecodedAppEvent::ChatArchive)
                .unwrap_or(DecodedAppEvent::Ignored)
        }
        DurableAppEventKind::Namespaced { name, policy }
            if name == FINITECHAT_CHAT_RENAME_EVENT_V1
                && *policy == ApplicationDeliveryPolicy::NON_NOTIFYING =>
        {
            serde_json::from_slice::<ChatRenameV1>(&event.payload)
                .ok()
                .filter(|rename| rename.validate_limits().is_ok())
                .filter(|rename| event.conversation_id.as_deref() == Some(rename.topic_id.as_str()))
                .filter(|rename| event.segment_id.as_deref() == Some(rename.chat_id.as_str()))
                .map(|mut rename| {
                    rename.title = rename.title.trim().to_owned();
                    DecodedAppEvent::ChatRename(rename)
                })
                .unwrap_or(DecodedAppEvent::Ignored)
        }
        DurableAppEventKind::ConversationCreate
        | DurableAppEventKind::ConversationUpdate
        | DurableAppEventKind::ConversationArchive
        | DurableAppEventKind::ConversationSegmentStart
        | DurableAppEventKind::ChatEdit
        | DurableAppEventKind::RuntimeStateSnapshot
        | DurableAppEventKind::RuntimeCommandRequest
        | DurableAppEventKind::RuntimeCommandResult
        | DurableAppEventKind::RuntimeCommandCancel
        | DurableAppEventKind::StreamStart
        | DurableAppEventKind::StreamFinish
        | DurableAppEventKind::Namespaced { .. } => DecodedAppEvent::Ignored,
    }
}

fn device_link_bootstrap_event_kind() -> DurableAppEventKind {
    DurableAppEventKind::Namespaced {
        name: FINITECHAT_DEVICE_LINK_BOOTSTRAP_EVENT_V2.to_owned(),
        policy: ApplicationDeliveryPolicy::NON_NOTIFYING,
    }
}

fn typed_namespaced_payload(plaintext: &[u8], expected_name: &str) -> Option<Vec<u8>> {
    let event = serde_json::from_slice::<DecryptedApplicationEventV1>(plaintext).ok()?;
    event.validate_limits().ok()?;
    match event.kind {
        DurableAppEventKind::Namespaced { name, policy }
            if name == expected_name && policy == ApplicationDeliveryPolicy::NON_NOTIFYING =>
        {
            (event.conversation_id.is_none() && event.segment_id.is_none()).then_some(event.payload)
        }
        _ => None,
    }
}

fn device_link_bootstrap_from_stored_event(
    event: &StoredAppEvent,
) -> Option<DeviceLinkBootstrapV2> {
    let payload =
        typed_namespaced_payload(&event.plaintext, FINITECHAT_DEVICE_LINK_BOOTSTRAP_EVENT_V2)?;
    let bootstrap = serde_json::from_slice::<DeviceLinkBootstrapV2>(&payload).ok()?;
    (bootstrap.version == DEVICE_LINK_BOOTSTRAP_VERSION_V2
        && bootstrap.validate_limits().is_ok()
        && bootstrap.room.room_id == event.room_id)
        .then_some(bootstrap)
}

fn is_device_link_control_event(plaintext: &[u8]) -> bool {
    typed_namespaced_payload(plaintext, FINITECHAT_DEVICE_LINK_BOOTSTRAP_EVENT_V2).is_some()
        || typed_namespaced_payload(plaintext, LEGACY_DEVICE_LINK_BOOTSTRAP_REQUEST_EVENT_V2)
            .is_some()
}

fn device_link_history_event_from_stored(
    event: StoredAppEvent,
) -> Result<DeviceLinkBootstrapEventV2, FiniteChatCoreError> {
    let history = DeviceLinkBootstrapEventV2 {
        seq: event.seq,
        message_id: event.message_id,
        sender: event.sender,
        plaintext: event.plaintext,
        timestamp_unix_seconds: event.timestamp_unix_seconds,
    };
    history.validate_limits().map_err(client_error)?;
    Ok(history)
}

fn update_device_link_digest_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn device_link_bootstrap_chunk_sha256_hex(history: &[DeviceLinkBootstrapEventV2]) -> String {
    hex::encode(device_link_bootstrap_chunk_sha256(history))
}

fn device_link_bootstrap_history_sha256_from_chunks(
    chunks: &[LinkBootstrapChunkPlan],
) -> Result<String, FiniteChatCoreError> {
    let mut digest = Sha256::new();
    for chunk in chunks {
        let bytes = hex::decode(&chunk.chunk_sha256)
            .map_err(|_| client_error("device-link chunk digest is invalid"))?;
        if bytes.len() != 32 {
            return Err(client_error("device-link chunk digest is invalid"));
        }
        digest.update(bytes);
    }
    Ok(hex::encode(digest.finalize()))
}

fn finalize_link_bootstrap_room_manifest(
    bootstrap_id: &str,
    source: &DeviceRef,
    target: &DeviceRef,
    export: &mut LinkBootstrapRoomExport,
) -> Result<(), FiniteChatCoreError> {
    if export.chunks.is_empty() {
        export.chunks.push(LinkBootstrapChunkPlan {
            start_after_seq: export.planning_after_seq,
            start_after_message_id: export.planning_after_message_id.clone(),
            end_seq: export.planning_after_seq,
            end_message_id: export.planning_after_message_id.clone(),
            event_count: 0,
            chunk_sha256: device_link_bootstrap_chunk_sha256_hex(&[]),
        });
    }
    let history_sha256 = device_link_bootstrap_history_sha256_from_chunks(&export.chunks)?;
    let manifest_bootstrap = DeviceLinkBootstrapV2 {
        version: DEVICE_LINK_BOOTSTRAP_VERSION_V2,
        bootstrap_id: bootstrap_id.to_owned(),
        target: target.clone(),
        chunk_index: 0,
        chunk_count: export.chunks.len() as u32,
        total_history_events: export.total_history_events,
        history_sha256: history_sha256.clone(),
        room: export.room.clone(),
        canonical_selection: export.canonical_selection.clone(),
        profiles: export.profiles.clone(),
        history: if export.total_history_events == 0 {
            Vec::new()
        } else {
            // Manifest hashing excludes chunk payloads, but the shared helper
            // validates the wire shape first.
            vec![DeviceLinkBootstrapEventV2 {
                seq: 0,
                message_id: "manifest-placeholder".to_owned(),
                sender: source.clone(),
                plaintext: vec![0],
                timestamp_unix_seconds: 0,
            }]
        },
    };
    let receipt = StoredDeviceLinkBootstrapReceipt::from_bootstrap(source, &manifest_bootstrap)
        .map_err(store_error)?;
    export.history_sha256 = Some(history_sha256);
    export.manifest_sha256 = Some(receipt.manifest_sha256);
    export.planning_complete = true;
    Ok(())
}

fn device_link_bootstrap_idempotency_key(
    bootstrap_id: &str,
    room_id: &str,
    chunk_index: u32,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"finitechat.device-link-bootstrap.idempotency.v2");
    update_device_link_digest_field(&mut digest, bootstrap_id.as_bytes());
    update_device_link_digest_field(&mut digest, room_id.as_bytes());
    digest.update(chunk_index.to_be_bytes());
    format!("dlb-{}", hex::encode(digest.finalize()))
}

fn device_link_bootstrap_profile_from_app(
    profile: AppProfileSummary,
) -> DeviceLinkBootstrapProfileV2 {
    DeviceLinkBootstrapProfileV2 {
        account_id: profile.account_id,
        npub: profile.npub,
        display_name: profile.display_name,
        about: profile.about,
        picture: profile.picture,
        is_agent: profile.is_agent,
    }
}

fn app_profile_from_device_link_bootstrap(
    profile: DeviceLinkBootstrapProfileV2,
) -> AppProfileSummary {
    AppProfileSummary {
        account_id: profile.account_id,
        npub: profile.npub,
        display_name: profile.display_name,
        about: profile.about,
        picture: profile.picture,
        stale: false,
        is_agent: profile.is_agent,
    }
}

fn app_device_link_bootstrap_receipt(
    receipt: &StoredDeviceLinkBootstrapReceipt,
) -> AppDeviceLinkBootstrapReceipt {
    AppDeviceLinkBootstrapReceipt {
        bootstrap_id: receipt.bootstrap_id.clone(),
        room_id: receipt.room_id.clone(),
        manifest_sha256: receipt.manifest_sha256.clone(),
    }
}

fn conversation_id_from_decoded_event(event: &DecodedAppEvent) -> Option<String> {
    match event {
        DecodedAppEvent::ChatMessage(decoded) => {
            let payload_conversation_id = match &decoded.payload {
                ChatMessagePayload::Hermes(payload) => payload.conversation_id.clone(),
                ChatMessagePayload::Poll(_) | ChatMessagePayload::Raw(_) => None,
            };
            decoded.conversation_id.clone().or(payload_conversation_id)
        }
        DecodedAppEvent::ChatReaction(_)
        | DecodedAppEvent::ChatReceipt(_)
        | DecodedAppEvent::PollVote(_)
        | DecodedAppEvent::ChatArchive(_)
        | DecodedAppEvent::ChatRename(_)
        | DecodedAppEvent::Ignored => None,
    }
}

#[cfg(test)]
fn encode_text_message_payload(
    text: &str,
    reply_to_message_id: Option<&str>,
) -> Result<Vec<u8>, FiniteChatCoreError> {
    encode_text_message_payload_scoped(text, reply_to_message_id, None, None, None, None)
}

fn encode_text_message_payload_scoped(
    text: &str,
    reply_to_message_id: Option<&str>,
    conversation_id: Option<&str>,
    chat_id: Option<&str>,
    metadata_json: Option<&str>,
    requester_context: Option<&VerifiedRequesterContext>,
) -> Result<Vec<u8>, FiniteChatCoreError> {
    let mut metadata = match metadata_json.map(str::trim) {
        None | Some("") => BTreeMap::new(),
        Some(encoded) => serde_json::from_str::<BTreeMap<String, serde_json::Value>>(encoded)
            .map_err(|_| FiniteChatCoreError::Client {
                reason: "message metadata_json must be a JSON object".to_owned(),
            })?,
    };
    // The verified requester context is authoritative: it is inserted last so
    // caller metadata can never spoof the transport claims.
    if let Some(requester) = requester_context {
        metadata.insert(
            finitechat_hermes::HERMES_METADATA_REQUESTER_EMAIL.to_owned(),
            serde_json::Value::String(requester.email.clone()),
        );
        metadata.insert(
            finitechat_hermes::HERMES_METADATA_SITES_REQUESTER_ASSERTION.to_owned(),
            serde_json::Value::String(requester.sites_assertion.clone()),
        );
    }
    HermesMessagePayloadV1 {
        payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
        conversation_id: conversation_id.map(ToOwned::to_owned),
        segment_id: chat_id.map(ToOwned::to_owned),
        text: text.to_owned(),
        kind: finitechat_hermes::HermesSendKindV1::Message,
        status: finitechat_hermes::HermesMessageStatusV1::Complete,
        edit_of: None,
        attachments: Vec::new(),
        reply_to_message_id: reply_to_message_id.map(ToOwned::to_owned),
        sender_name: None,
        metadata,
    }
    .encode()
    .map_err(client_error)
}

fn encode_attachment_message_payload(
    caption: &str,
    attachments: Vec<HermesAttachmentV1>,
    reply_to_message_id: Option<&str>,
    conversation_id: Option<&str>,
    chat_id: Option<&str>,
) -> Result<Vec<u8>, FiniteChatCoreError> {
    validate_item_count(
        "attachments",
        attachments.len(),
        MAX_ATTACHMENTS_PER_MESSAGE,
    )
    .map_err(client_error)?;
    HermesMessagePayloadV1 {
        payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
        conversation_id: conversation_id.map(ToOwned::to_owned),
        segment_id: chat_id.map(ToOwned::to_owned),
        text: caption.to_owned(),
        kind: finitechat_hermes::HermesSendKindV1::Media,
        status: finitechat_hermes::HermesMessageStatusV1::Complete,
        edit_of: None,
        attachments,
        reply_to_message_id: reply_to_message_id.map(ToOwned::to_owned),
        sender_name: None,
        metadata: BTreeMap::new(),
    }
    .encode()
    .map_err(client_error)
}

fn encode_poll_message_payload(
    question: &str,
    options: Vec<String>,
) -> Result<Vec<u8>, FiniteChatCoreError> {
    let payload = ChatPollPayloadV1 {
        payload_type: FINITECHAT_POLL_PAYLOAD_TYPE_V1.to_owned(),
        question: normalize_bounded_non_empty_string(
            "chat_poll.question",
            question,
            MAX_POLL_QUESTION_BYTES,
        )?,
        options: normalized_poll_options(options)?,
    };
    validate_poll_payload(&payload)?;
    serde_json::to_vec(&payload).map_err(client_error)
}

fn normalized_poll_options(
    options: Vec<String>,
) -> Result<Vec<ChatPollPayloadOptionV1>, FiniteChatCoreError> {
    if options.len() < MIN_POLL_OPTIONS {
        return Err(FiniteChatCoreError::Client {
            reason: format!("poll must include at least {MIN_POLL_OPTIONS} options"),
        });
    }
    validate_item_count("chat_poll.options", options.len(), MAX_POLL_OPTIONS)
        .map_err(client_error)?;
    let mut normalized = Vec::with_capacity(options.len());
    for (index, option) in options.into_iter().enumerate() {
        normalized.push(ChatPollPayloadOptionV1 {
            option_id: poll_option_id(index),
            text: normalize_bounded_non_empty_string(
                "chat_poll.option",
                &option,
                MAX_POLL_OPTION_BYTES,
            )?,
        });
    }
    Ok(normalized)
}

fn validate_poll_payload(payload: &ChatPollPayloadV1) -> Result<(), FiniteChatCoreError> {
    if payload.payload_type != FINITECHAT_POLL_PAYLOAD_TYPE_V1 {
        return Err(FiniteChatCoreError::Client {
            reason: "poll payload type is not supported".to_owned(),
        });
    }
    normalize_bounded_non_empty_string(
        "chat_poll.question",
        &payload.question,
        MAX_POLL_QUESTION_BYTES,
    )?;
    if payload.options.len() < MIN_POLL_OPTIONS {
        return Err(FiniteChatCoreError::Client {
            reason: format!("poll must include at least {MIN_POLL_OPTIONS} options"),
        });
    }
    validate_item_count("chat_poll.options", payload.options.len(), MAX_POLL_OPTIONS)
        .map_err(client_error)?;
    let mut option_ids = BTreeSet::new();
    for option in &payload.options {
        normalize_bounded_non_empty_string(
            "chat_poll.option",
            &option.text,
            MAX_POLL_OPTION_BYTES,
        )?;
        normalize_bounded_non_empty_string(
            "chat_poll.option_id",
            &option.option_id,
            MAX_OBJECT_ID_BYTES,
        )?;
        if !option_ids.insert(option.option_id.clone()) {
            return Err(FiniteChatCoreError::Client {
                reason: format!("duplicate poll option id '{}'", option.option_id),
            });
        }
    }
    Ok(())
}

fn validate_poll_vote(vote: &ChatPollVoteV1) -> Result<(), FiniteChatCoreError> {
    normalize_bounded_non_empty_string(
        "chat_poll_vote.poll_message_id",
        &vote.poll_message_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    normalize_bounded_non_empty_string(
        "chat_poll_vote.option_id",
        &vote.option_id,
        MAX_OBJECT_ID_BYTES,
    )?;
    Ok(())
}

fn chat_poll_from_payload(payload: ChatPollPayloadV1) -> ChatPoll {
    ChatPoll {
        question: payload.question,
        options: payload
            .options
            .into_iter()
            .map(|option| ChatPollOption {
                option_id: option.option_id,
                text: option.text,
                vote_count: 0,
                voted_by_me: false,
            })
            .collect(),
        total_votes: 0,
        my_vote_option_id: None,
    }
}

fn poll_vote_event_kind() -> DurableAppEventKind {
    DurableAppEventKind::Namespaced {
        name: FINITECHAT_POLL_VOTE_EVENT_V1.to_owned(),
        policy: ApplicationDeliveryPolicy::NON_NOTIFYING,
    }
}

fn poll_option_id(index: usize) -> String {
    format!("option-{}", index + 1)
}

fn normalize_bounded_non_empty_string(
    field: &str,
    value: &str,
    max_bytes: u32,
) -> Result<String, FiniteChatCoreError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(FiniteChatCoreError::Client {
            reason: format!("{field} must not be empty"),
        });
    }
    validate_string_bytes(field, trimmed, max_bytes).map_err(client_error)?;
    Ok(trimmed.to_owned())
}

fn hermes_attachment_kind(kind: &ChatMediaKind) -> HermesAttachmentKindV1 {
    match kind {
        ChatMediaKind::Image => HermesAttachmentKindV1::Image,
        ChatMediaKind::VoiceNote => HermesAttachmentKindV1::Audio,
        ChatMediaKind::Video => HermesAttachmentKindV1::Video,
        ChatMediaKind::File => HermesAttachmentKindV1::File,
    }
}

fn encode_application_event(
    kind: DurableAppEventKind,
    conversation_id: Option<String>,
    payload: &[u8],
) -> Result<Vec<u8>, FiniteChatCoreError> {
    encode_application_event_with_segment(kind, conversation_id, None, payload)
}

fn encode_application_event_with_segment(
    kind: DurableAppEventKind,
    conversation_id: Option<String>,
    segment_id: Option<String>,
    payload: &[u8],
) -> Result<Vec<u8>, FiniteChatCoreError> {
    let event = DecryptedApplicationEventV1 {
        kind,
        conversation_id,
        segment_id,
        payload: payload.to_vec(),
    };
    event.validate_limits().map_err(client_error)?;
    serde_json::to_vec(&event).map_err(client_error)
}

fn chat_media_attachment(index: usize, attachment: HermesAttachmentV1) -> ChatMediaAttachment {
    let local_pending_upload =
        attachment.path.is_some() && attachment.url.is_none() && attachment.blob.is_none();
    let blob = attachment.blob;
    let dimensions = blob
        .as_ref()
        .and_then(|blob| blob.metadata.dimensions.as_ref());
    let attachment_id = blob
        .as_ref()
        .map(|blob| blob.plaintext_sha256.clone())
        .or_else(|| attachment.url.clone())
        .or_else(|| attachment.path.clone())
        .unwrap_or_else(|| format!("attachment-{index}"));
    let url = blob
        .as_ref()
        .map(|blob| blob.url.clone())
        .or(attachment.url);
    let mime_type = blob
        .as_ref()
        .map(|blob| blob.metadata.mime_type.clone())
        .filter(|mime_type| !mime_type.trim().is_empty())
        .unwrap_or(attachment.mime_type);
    let filename = blob
        .as_ref()
        .map(|blob| blob.metadata.filename.clone())
        .filter(|filename| !filename.trim().is_empty())
        .unwrap_or(attachment.name);
    ChatMediaAttachment {
        attachment_id,
        url,
        mime_type,
        filename,
        kind: match attachment.kind {
            HermesAttachmentKindV1::Image => ChatMediaKind::Image,
            HermesAttachmentKindV1::Video => ChatMediaKind::Video,
            HermesAttachmentKindV1::Audio => ChatMediaKind::VoiceNote,
            HermesAttachmentKindV1::File => ChatMediaKind::File,
        },
        width: dimensions.map(|dimensions| dimensions.width),
        height: dimensions.map(|dimensions| dimensions.height),
        local_path: attachment.path,
        upload_progress_per_mille: local_pending_upload.then_some(0),
        download_progress_per_mille: None,
    }
}

fn attachment_download_key(
    room_id: &str,
    message_id: &str,
    attachment_id: &str,
) -> (String, String, String) {
    (
        room_id.to_owned(),
        message_id.to_owned(),
        attachment_id.to_owned(),
    )
}

fn attachment_references_by_id(
    message: &ChatMessage,
) -> BTreeMap<String, AttachmentBlobReferenceV1> {
    let DecodedAppEvent::ChatMessage(decoded) = decode_application_event(&message.payload) else {
        return BTreeMap::new();
    };
    let ChatMessagePayload::Hermes(payload) = decoded.payload else {
        return BTreeMap::new();
    };

    let mut references = BTreeMap::new();
    for (index, attachment) in payload.attachments.into_iter().enumerate() {
        let projected = chat_media_attachment(index, attachment.clone());
        if let Some(reference) = attachment.blob {
            references.insert(projected.attachment_id, reference);
        }
    }
    references
}

fn attachment_reference_for_id(
    message: &ChatMessage,
    attachment_id: &str,
) -> Option<AttachmentBlobReferenceV1> {
    attachment_references_by_id(message).remove(attachment_id)
}

fn attachment_plaintext_matches(reference: &AttachmentBlobReferenceV1, plaintext: &[u8]) -> bool {
    plaintext.len() as u64 == reference.plaintext_size
        && sha256_hex(plaintext) == reference.plaintext_sha256
}

fn sanitized_attachment_filename(filename: &str) -> String {
    let trimmed = filename.trim();
    let mut out = String::with_capacity(trimmed.len().min(128));
    for ch in trimmed.chars() {
        if out.len() >= 128 {
            break;
        }
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let out = out.trim_matches('.');
    if out.is_empty() || out == "_" {
        return "attachment".to_owned();
    }
    if out == ".." {
        return "attachment".to_owned();
    }
    out.to_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn sender_display_name(sender: &DeviceRef, payload_name: Option<&str>, is_mine: bool) -> String {
    if is_mine {
        return "You".to_owned();
    }
    if let Some(name) = payload_name.map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_owned();
    }
    format!(
        "{} / {}",
        short_account_label(&sender.account_id),
        sender.device_id
    )
}

fn typing_member_from_activity(
    entry: &EphemeralActivityProjectionEntry,
    profile_cache: &BTreeMap<String, AppProfileSummary>,
) -> AppTypingMember {
    let npub = npub_encode(&entry.sender.account_id).ok();
    let profile_name = profile_cache
        .get(&entry.sender.account_id)
        .map(|profile| profile.display_name.trim())
        .filter(|display_name| !display_name.is_empty());
    let picture = profile_cache
        .get(&entry.sender.account_id)
        .and_then(|profile| profile.picture.clone());
    AppTypingMember {
        room_id: entry.room_id.clone(),
        topic_id: entry.conversation_id.clone(),
        chat_id: entry.segment_id.clone(),
        account_id: entry.sender.account_id.clone(),
        device_id: entry.sender.device_id.clone(),
        display_name: sender_display_name(&entry.sender, profile_name, false),
        picture,
        npub,
        activity_kind: entry.activity_kind.clone(),
    }
}

fn is_chat_live_indicator_activity(activity_kind: &str) -> bool {
    activity_kind == FINITECHAT_ACTIVITY_KIND_TYPING
        || activity_kind == FINITECHAT_ACTIVITY_KIND_THINKING
        || activity_kind == FINITECHAT_ACTIVITY_KIND_WORKING
}

fn live_indicator_activity_priority(activity_kind: &str) -> u8 {
    match activity_kind {
        FINITECHAT_ACTIVITY_KIND_WORKING => 0,
        FINITECHAT_ACTIVITY_KIND_THINKING => 1,
        FINITECHAT_ACTIVITY_KIND_TYPING => 2,
        _ => 3,
    }
}

fn chat_message_from_stored_decoded(
    message: StoredAppMessage,
    decoded: DecodedAppEvent,
    owner: &DeviceRef,
) -> Option<ChatMessage> {
    let event = StoredAppEvent {
        room_id: message.room_id,
        seq: message.seq,
        message_id: message.message_id,
        sender: message.sender,
        plaintext: message.plaintext,
        timestamp_unix_seconds: message.timestamp_unix_seconds,
    };
    project_decoded_chat_message(event, decoded, owner)
}

fn chat_message_from_outbox(
    message: StoredOutboundMessage,
    owner: &DeviceRef,
) -> Option<ChatMessage> {
    if message.sender != *owner {
        return None;
    }
    let local_send = match message.local_state {
        StoredOutboundLocalState::Sending => OutboundLocalSendState::Sending,
        StoredOutboundLocalState::Sent => OutboundLocalSendState::Sent,
    };
    let server_delivery = match message.server_delivery_state {
        StoredOutboundServerDeliveryState::Undelivered => OutboundServerDeliveryState::Undelivered,
        StoredOutboundServerDeliveryState::Failed { reason } => {
            OutboundServerDeliveryState::Failed { reason }
        }
    };
    let mut projected = project_chat_message(
        message.room_id,
        u64::MAX,
        message.message_id,
        message.sender,
        message.plaintext,
        message.timestamp_unix_seconds,
        owner,
    )?;
    projected.outbound_delivery = Some(OutboundDelivery {
        local_send,
        server_delivery,
    });
    Some(projected)
}

fn failed_outbox_message(
    mut message: StoredOutboundMessage,
    reason: String,
) -> StoredOutboundMessage {
    message.local_state = StoredOutboundLocalState::Sent;
    message.server_delivery_state = StoredOutboundServerDeliveryState::Failed { reason };
    message
}

fn app_outbox_debug_row(
    message: StoredOutboundMessage,
) -> Result<AppOutboxDebugRow, FiniteChatCoreError> {
    let append_request_message_id = message
        .append_request
        .envelope
        .message_id()
        .map_err(client_error)?;
    Ok(AppOutboxDebugRow {
        room_id: message.room_id,
        message_id: message.message_id,
        sender_account_id: message.sender.account_id,
        sender_device_id: message.sender.device_id,
        local_state: stored_outbound_local_state_label(&message.local_state).to_owned(),
        server_delivery_state: stored_outbound_server_delivery_state_label(
            &message.server_delivery_state,
        )
        .to_owned(),
        append_request_room_id: message.append_request.room_id,
        append_request_message_id,
        append_request_sender_account_id: message.append_request.sender.account_id,
        append_request_sender_device_id: message.append_request.sender.device_id,
        idempotency_key_present: !message.append_request.idempotency_key.is_empty(),
    })
}

fn stored_outbound_local_state_label(state: &StoredOutboundLocalState) -> &'static str {
    match state {
        StoredOutboundLocalState::Sending => "sending",
        StoredOutboundLocalState::Sent => "sent",
    }
}

fn stored_outbound_server_delivery_state_label(
    state: &StoredOutboundServerDeliveryState,
) -> &'static str {
    match state {
        StoredOutboundServerDeliveryState::Undelivered => "undelivered",
        StoredOutboundServerDeliveryState::Failed { .. } => "failed",
    }
}

#[cfg(test)]
fn chat_messages_from_stored(
    stored_messages: Vec<StoredAppMessage>,
    stored_events: Vec<StoredAppEvent>,
    owner: &DeviceRef,
) -> Vec<ChatMessage> {
    ChatProjectionState::from_stored(stored_messages, stored_events, owner).messages()
}

impl ChatProjectionState {
    fn from_stored(
        stored_messages: Vec<StoredAppMessage>,
        stored_events: Vec<StoredAppEvent>,
        owner: &DeviceRef,
    ) -> Self {
        let mut projection = Self::default();
        for message in stored_messages {
            // One decode per stored message drives the message projection
            // and the conversation projection; neither re-parses plaintext.
            let decoded = decode_application_event(&message.plaintext);
            let scope = ChatEventScope::from_decoded(&decoded);
            if let Some(projected) = chat_message_from_stored_decoded(message, decoded, owner) {
                projection.insert_message_scoped(projected, Some(scope), owner);
            }
        }
        for event in stored_events {
            let _ = projection.apply_parsed_event(ParsedAppEvent::decode(event), owner);
        }
        projection.trim_to_limit();
        projection
    }

    fn append_messages(&mut self, messages: Vec<ChatMessage>, owner: &DeviceRef) {
        for message in messages {
            self.insert_message(message, owner);
        }
        self.trim_to_limit();
    }

    /// Apply one application event to the projection. This is the only
    /// entry point for events, and it takes the pre-decoded
    /// [`ParsedAppEvent`] — nothing below this boundary parses plaintext.
    fn apply_parsed_event(&mut self, parsed: ParsedAppEvent, owner: &DeviceRef) -> bool {
        let ParsedAppEvent {
            event,
            decoded,
            envelope,
        } = parsed;
        let decoded_conversation_id = conversation_id_from_decoded_event(&decoded);
        let duplicate_projected_message = matches!(decoded, DecodedAppEvent::ChatMessage(_))
            && self
                .messages
                .get(&(event.room_id.clone(), event.message_id.clone()))
                .is_some_and(|message| message.seq == event.seq);
        if !duplicate_projected_message {
            self.apply_conversation_projection_from_decoded(
                &decoded,
                &event,
                envelope.as_ref(),
                decoded_conversation_id.as_deref(),
            );
        }
        let archive_changed = match decoded {
            // A duplicate (same room, message id, and seq) was already
            // projected from its stored message row with identical inputs;
            // re-running the projection would only repeat its parse work.
            DecodedAppEvent::ChatMessage(_) if duplicate_projected_message => false,
            DecodedAppEvent::ChatMessage(_) => {
                if let Some(message) = project_decoded_chat_message(event, decoded, owner) {
                    self.insert_message_record(message, owner);
                }
                false
            }
            DecodedAppEvent::ChatReaction(reaction) => {
                self.apply_reaction(&event.room_id, &event.sender, owner, reaction);
                false
            }
            DecodedAppEvent::ChatReceipt(receipt) => {
                self.apply_receipt(&event.room_id, &event.sender, receipt);
                false
            }
            DecodedAppEvent::PollVote(vote) => {
                self.apply_poll_vote(&event.room_id, &event.sender, owner, vote);
                false
            }
            DecodedAppEvent::ChatArchive(archive) => self.apply_chat_archive(
                &event.room_id,
                event.seq,
                archive,
                ChatArchiveProjectionSource::CanonicalEvent,
            ),
            DecodedAppEvent::ChatRename(rename) => {
                self.apply_chat_rename(&event.room_id, event.seq, rename);
                false
            }
            DecodedAppEvent::Ignored => false,
        };
        self.trim_to_limit();
        archive_changed
    }

    /// Drive the conversation projection from the single decode pass. A
    /// chat message needs only its decoded kind and scoping (the
    /// conversation projection's ChatMessage arm reads no payload).
    /// Conversation lifecycle kinds use the envelope carried by
    /// [`ParsedAppEvent`]; kinds the conversation projection ignores are
    /// skipped outright.
    fn apply_conversation_projection_from_decoded(
        &mut self,
        decoded: &DecodedAppEvent,
        event: &StoredAppEvent,
        envelope: Option<&DecryptedApplicationEventV1>,
        decoded_conversation_id: Option<&str>,
    ) {
        let (app_event, conversation_id) = match decoded {
            DecodedAppEvent::ChatMessage(decoded_message) => {
                let conversation_id = decoded_message
                    .conversation_id
                    .as_deref()
                    .or(decoded_conversation_id)
                    .map(str::to_owned);
                let app_event = DecryptedApplicationEventV1 {
                    kind: DurableAppEventKind::ChatMessage,
                    conversation_id: decoded_message.conversation_id.clone(),
                    segment_id: decoded_message.segment_id.clone(),
                    payload: Vec::new(),
                };
                (app_event, conversation_id)
            }
            DecodedAppEvent::ChatReaction(_)
            | DecodedAppEvent::ChatReceipt(_)
            | DecodedAppEvent::PollVote(_)
            | DecodedAppEvent::ChatArchive(_)
            | DecodedAppEvent::ChatRename(_) => return,
            DecodedAppEvent::Ignored => {
                let Some(app_event) = envelope else {
                    return;
                };
                let conversation_id = app_event
                    .conversation_id
                    .as_deref()
                    .or(decoded_conversation_id)
                    .map(str::to_owned);
                (app_event.clone(), conversation_id)
            }
        };
        let Some(conversation_id) = conversation_id else {
            return;
        };
        let context = ConversationProjectionEventContext {
            room_id: &event.room_id,
            accepted_seq: event.seq,
            conversation_id: Some(conversation_id.as_str()),
        };
        let _ = self.conversations.apply_event(context, &app_event);
    }

    fn restore_chat_archives(&mut self, archives: &[StoredChatArchiveState]) {
        for archive in archives {
            let _ = self.apply_chat_archive(
                &archive.room_id,
                archive.accepted_seq,
                ChatArchiveV1 {
                    topic_id: archive.topic_id.clone(),
                    chat_id: archive.chat_id.clone(),
                    archived: archive.archived,
                },
                ChatArchiveProjectionSource::CacheFallback,
            );
        }
    }

    fn stored_chat_archives(&self) -> Vec<StoredChatArchiveState> {
        self.chat_archives
            .iter()
            .map(
                |((room_id, topic_id, chat_id), archive)| StoredChatArchiveState {
                    room_id: room_id.clone(),
                    topic_id: topic_id.clone(),
                    chat_id: chat_id.clone(),
                    accepted_seq: archive.accepted_seq,
                    archived: archive.archived,
                },
            )
            .collect()
    }

    fn topics(&self, local_read_seq: &BTreeMap<String, u64>) -> Vec<AppTopicSummary> {
        let mut messages_by_topic = BTreeMap::<(String, String), Vec<&ChatMessage>>::new();
        for message in self.messages.values() {
            let Some(topic_id) = message.conversation_id.as_ref() else {
                continue;
            };
            messages_by_topic
                .entry((message.room_id.clone(), topic_id.clone()))
                .or_default()
                .push(message);
        }

        let mut topics = self
            .conversations
            .entries()
            .map(|entry| {
                let key = (entry.room_id.clone(), entry.conversation_id.clone());
                topic_summary_from_projection(
                    entry,
                    messages_by_topic.remove(&key).unwrap_or_default(),
                    local_read_seq,
                    &self.chat_archives,
                    &self.chat_titles,
                )
            })
            .collect::<Vec<_>>();

        for ((room_id, topic_id), messages) in messages_by_topic {
            topics.push(topic_summary_from_messages(
                room_id,
                topic_id,
                messages,
                local_read_seq,
                &self.chat_archives,
                &self.chat_titles,
            ));
        }

        topics.sort_by(topic_sort);
        topics
    }

    fn messages(&self) -> Vec<ChatMessage> {
        let mut messages = self.messages.values().cloned().collect::<Vec<_>>();
        messages.sort_by(message_sort);
        messages
    }

    fn messages_for_room_window(&self, room_id: &str, limit: usize) -> Vec<ChatMessage> {
        let mut messages = self
            .messages
            .values()
            .filter(|message| message.room_id == room_id)
            .cloned()
            .collect::<Vec<_>>();
        messages.sort_by(message_sort);
        if messages.len() > limit {
            messages.drain(0..messages.len() - limit);
        }
        messages
    }

    fn messages_for_topic_window(
        &self,
        room_id: &str,
        topic_id: &str,
        limit: usize,
    ) -> Vec<ChatMessage> {
        let mut messages = self
            .messages
            .values()
            .filter(|message| message.room_id == room_id)
            .filter(|message| message.conversation_id.as_deref() == Some(topic_id))
            .cloned()
            .collect::<Vec<_>>();
        messages.sort_by(message_sort);
        if messages.len() > limit {
            messages.drain(0..messages.len() - limit);
        }
        messages
    }

    fn messages_for_chat_window(
        &self,
        room_id: &str,
        topic_id: &str,
        chat_id: &str,
        limit: usize,
    ) -> Vec<ChatMessage> {
        let mut messages = self
            .messages
            .values()
            .filter(|message| message.room_id == room_id)
            .filter(|message| message.conversation_id.as_deref() == Some(topic_id))
            .filter(|message| message.chat_id.as_deref() == Some(chat_id))
            .cloned()
            .collect::<Vec<_>>();
        messages.sort_by(message_sort);
        if messages.len() > limit {
            messages.drain(0..messages.len() - limit);
        }
        messages
    }

    fn visual_media_messages_for_room(&self, room_id: &str) -> Vec<ChatMessage> {
        let mut messages = self
            .messages
            .values()
            .filter(|message| message.room_id == room_id)
            .filter(|message| {
                message.media.iter().any(|attachment| {
                    matches!(attachment.kind, ChatMediaKind::Image | ChatMediaKind::Video)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        messages.sort_by(message_sort);
        messages
    }

    fn visual_media_messages_for_topic(&self, room_id: &str, topic_id: &str) -> Vec<ChatMessage> {
        let mut messages = self
            .messages
            .values()
            .filter(|message| message.room_id == room_id)
            .filter(|message| message.conversation_id.as_deref() == Some(topic_id))
            .filter(|message| {
                message.media.iter().any(|attachment| {
                    matches!(attachment.kind, ChatMediaKind::Image | ChatMediaKind::Video)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        messages.sort_by(message_sort);
        messages
    }

    fn visual_media_messages_for_chat(
        &self,
        room_id: &str,
        topic_id: &str,
        chat_id: &str,
    ) -> Vec<ChatMessage> {
        let mut messages = self
            .messages
            .values()
            .filter(|message| message.room_id == room_id)
            .filter(|message| message.conversation_id.as_deref() == Some(topic_id))
            .filter(|message| message.chat_id.as_deref() == Some(chat_id))
            .filter(|message| {
                message.media.iter().any(|attachment| {
                    matches!(attachment.kind, ChatMediaKind::Image | ChatMediaKind::Video)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        messages.sort_by(message_sort);
        messages
    }

    fn media_gallery_from_messages(
        room_id: &str,
        messages: &[ChatMessage],
    ) -> ChatMediaGalleryState {
        let mut items = Vec::new();
        for message in messages {
            debug_assert_eq!(message.room_id, room_id);
            for attachment in &message.media {
                if !matches!(attachment.kind, ChatMediaKind::Image | ChatMediaKind::Video) {
                    continue;
                }
                items.push(ChatMediaGalleryItem {
                    item_id: chat_media_gallery_item_id(message, attachment),
                    room_id: message.room_id.clone(),
                    message_id: message.message_id.clone(),
                    attachment_id: attachment.attachment_id.clone(),
                    attachment: attachment.clone(),
                    sender_display_name: message.sender_display_name.clone(),
                    sender_npub: message.sender_npub.clone(),
                    timestamp_unix_seconds: message.timestamp_unix_seconds,
                    display_timestamp: message.display_timestamp.clone(),
                });
            }
        }
        ChatMediaGalleryState {
            room_id: room_id.to_owned(),
            items,
        }
    }

    fn room_message_count(&self, room_id: &str) -> usize {
        self.messages
            .values()
            .filter(|message| message.room_id == room_id)
            .count()
    }

    fn topic_message_count(&self, room_id: &str, topic_id: &str) -> usize {
        self.messages
            .values()
            .filter(|message| message.room_id == room_id)
            .filter(|message| message.conversation_id.as_deref() == Some(topic_id))
            .count()
    }

    fn chat_message_count(&self, room_id: &str, topic_id: &str, chat_id: &str) -> usize {
        self.messages
            .values()
            .filter(|message| message.room_id == room_id)
            .filter(|message| message.conversation_id.as_deref() == Some(topic_id))
            .filter(|message| message.chat_id.as_deref() == Some(chat_id))
            .count()
    }

    fn message_exists(&self, room_id: &str, message_id: &str) -> bool {
        self.messages
            .contains_key(&(room_id.to_owned(), message_id.to_owned()))
    }

    fn message(&self, room_id: &str, message_id: &str) -> Option<&ChatMessage> {
        self.messages
            .get(&(room_id.to_owned(), message_id.to_owned()))
    }

    fn insert_message(&mut self, message: ChatMessage, owner: &DeviceRef) {
        self.insert_message_scoped(message, None, owner);
    }

    /// Insert a chat message whose application event was already decoded.
    /// `scope` carries the envelope scoping so the conversation projection
    /// does not have to re-parse the plaintext; without it (live appends of
    /// already-projected messages) the envelope is parsed as before.
    fn insert_message_scoped(
        &mut self,
        message: ChatMessage,
        scope: Option<ChatEventScope>,
        owner: &DeviceRef,
    ) {
        let key = message_key(&message);
        let should_apply_conversation = message.seq != u64::MAX
            && self
                .messages
                .get(&key)
                .is_none_or(|existing| existing.seq != message.seq);
        if should_apply_conversation {
            self.apply_conversation_projection_from_message(&message, scope.as_ref());
        }
        self.insert_message_record(message, owner);
    }

    fn insert_message_record(&mut self, mut message: ChatMessage, owner: &DeviceRef) {
        if message.chat_id.is_none()
            && let Some(topic_id) = message.conversation_id.as_deref()
        {
            message.chat_id = self
                .conversations
                .get(&message.room_id, topic_id)
                .and_then(|topic| topic.active_segment_id.clone())
                .or_else(|| (topic_id == HOME_TOPIC_ID).then(|| HOME_CHAT_ID.to_owned()));
        }
        message.reactions = reaction_summaries_for_message(&message, &self.reaction_senders, owner);
        message.read_receipt =
            receipt_summary_for_message(&message, &self.delivered_through, &self.read_through);
        if let Some(poll) = &message.poll {
            message.poll = Some(poll_with_vote_summary(
                poll,
                &message.room_id,
                &message.message_id,
                &self.poll_votes,
                owner,
            ));
        }
        let key = message_key(&message);
        if !self.messages.contains_key(&key) {
            let latest_same_room_seq =
                self.message_arrival_order
                    .iter()
                    .rev()
                    .find_map(|candidate| {
                        self.messages
                            .get(candidate)
                            .filter(|existing| existing.room_id == message.room_id)
                            .map(|existing| existing.seq)
                    });
            if latest_same_room_seq.is_none_or(|seq| seq <= message.seq) {
                self.message_arrival_order.push_back(key.clone());
            } else {
                let position = self
                    .message_arrival_order
                    .iter()
                    .position(|candidate| {
                        self.messages.get(candidate).is_some_and(|existing| {
                            existing.room_id == message.room_id && existing.seq > message.seq
                        })
                    })
                    .unwrap_or(self.message_arrival_order.len());
                self.message_arrival_order.insert(position, key.clone());
            }
        }
        self.messages.insert(key, message);
    }

    fn apply_conversation_projection_from_message(
        &mut self,
        message: &ChatMessage,
        scope: Option<&ChatEventScope>,
    ) {
        let Some(conversation_id) = message.conversation_id.as_deref() else {
            return;
        };
        let app_event = match scope {
            // The conversation projection reads only the kind and scoping of
            // a chat-message envelope, both already decoded here; the empty
            // payload is never read for this kind. Rebuilding the envelope
            // avoids re-parsing the full plaintext per stored message.
            Some(scope) => DecryptedApplicationEventV1 {
                kind: DurableAppEventKind::ChatMessage,
                conversation_id: scope.conversation_id.clone(),
                segment_id: scope.segment_id.clone(),
                payload: Vec::new(),
            },
            None => {
                let Ok(app_event) =
                    serde_json::from_slice::<DecryptedApplicationEventV1>(&message.payload)
                else {
                    return;
                };
                app_event
            }
        };
        let context = ConversationProjectionEventContext {
            room_id: &message.room_id,
            accepted_seq: message.seq,
            conversation_id: app_event
                .conversation_id
                .as_deref()
                .or(Some(conversation_id)),
        };
        let _ = self.conversations.apply_event(context, &app_event);
    }

    fn latest_peer_message_needing_read_receipt(
        &self,
        room_id: &str,
        owner: &DeviceRef,
    ) -> Option<(String, u64)> {
        let owner_key = device_label(owner);
        let read_through = self
            .read_through
            .get(&(room_id.to_owned(), owner_key))
            .copied()
            .unwrap_or_default();
        self.messages
            .values()
            .filter(|message| message.room_id == room_id)
            .filter(|message| !message.is_mine)
            .filter(|message| message.seq > read_through)
            .max_by(|left, right| {
                left.seq
                    .cmp(&right.seq)
                    .then_with(|| left.message_id.cmp(&right.message_id))
            })
            .map(|message| (message.message_id.clone(), message.seq))
    }

    fn latest_peer_message(&self, room_id: &str) -> Option<(String, u64)> {
        self.messages
            .values()
            .filter(|message| message.room_id == room_id)
            .filter(|message| !message.is_mine)
            .max_by(|left, right| {
                left.seq
                    .cmp(&right.seq)
                    .then_with(|| left.message_id.cmp(&right.message_id))
            })
            .map(|message| (message.message_id.clone(), message.seq))
    }

    fn apply_reaction(
        &mut self,
        room_id: &str,
        sender: &DeviceRef,
        owner: &DeviceRef,
        reaction: ChatReactionV1,
    ) {
        let key = (room_id.to_owned(), reaction.target_message_id);
        let emoji = reaction.emoji.trim().to_owned();
        let sender_key = device_label(sender);
        if !self
            .reaction_senders
            .insert((key.0.clone(), key.1.clone(), emoji.clone(), sender_key))
        {
            return;
        }

        self.refresh_reactions_for_message(&key, owner);
    }

    fn apply_receipt(&mut self, room_id: &str, sender: &DeviceRef, receipt: ChatReceiptV1) {
        let target_key = (room_id.to_owned(), receipt.target_message_id);
        let Some(target) = self.messages.get(&target_key) else {
            return;
        };
        if target.seq != receipt.target_seq {
            return;
        }

        let receipt_key = (room_id.to_owned(), device_label(sender));
        match receipt.state {
            ChatReceiptStateV1::Delivered => {
                upsert_receipt_marker(&mut self.delivered_through, receipt_key, receipt.target_seq);
            }
            ChatReceiptStateV1::Read | ChatReceiptStateV1::Seen => {
                upsert_receipt_marker(&mut self.read_through, receipt_key, receipt.target_seq);
            }
        }
        self.refresh_receipts_for_room(room_id);
    }

    fn apply_poll_vote(
        &mut self,
        room_id: &str,
        sender: &DeviceRef,
        owner: &DeviceRef,
        vote: ChatPollVoteV1,
    ) {
        let message_key = (room_id.to_owned(), vote.poll_message_id.clone());
        let option_is_valid = self
            .messages
            .get(&message_key)
            .and_then(|message| message.poll.as_ref())
            .is_some_and(|poll| {
                poll.options
                    .iter()
                    .any(|option| option.option_id == vote.option_id)
            });
        if !option_is_valid {
            return;
        }

        self.poll_votes.insert(
            (
                room_id.to_owned(),
                vote.poll_message_id.clone(),
                device_label(sender),
            ),
            vote.option_id,
        );
        self.refresh_poll_for_message(&message_key, owner);
    }

    fn apply_chat_rename(&mut self, room_id: &str, accepted_seq: u64, rename: ChatRenameV1) {
        let key = (room_id.to_owned(), rename.topic_id, rename.chat_id);
        if !self.chat_titles.contains_key(&key) && self.chat_titles.len() >= MAX_APP_CHAT_TITLES {
            return;
        }
        let should_replace = self
            .chat_titles
            .get(&key)
            .is_none_or(|existing| accepted_seq > existing.accepted_seq);
        if should_replace {
            self.chat_titles.insert(
                key,
                ChatTitleProjectionEntry {
                    accepted_seq,
                    title: rename.title,
                },
            );
        }
    }

    fn apply_chat_archive(
        &mut self,
        room_id: &str,
        accepted_seq: u64,
        archive: ChatArchiveV1,
        source: ChatArchiveProjectionSource,
    ) -> bool {
        let key = (room_id.to_owned(), archive.topic_id, archive.chat_id);
        if !self.chat_archives.contains_key(&key)
            && self.chat_archives.len() >= MAX_APP_CHAT_ARCHIVES
        {
            return false;
        }
        let should_replace =
            self.chat_archives
                .get(&key)
                .is_none_or(|existing| match (existing.source, source) {
                    (
                        ChatArchiveProjectionSource::CacheFallback,
                        ChatArchiveProjectionSource::CanonicalEvent,
                    ) => true,
                    (
                        ChatArchiveProjectionSource::CanonicalEvent,
                        ChatArchiveProjectionSource::CacheFallback,
                    ) => false,
                    _ => accepted_seq > existing.accepted_seq,
                });
        if should_replace {
            self.chat_archives.insert(
                key,
                ChatArchiveProjectionEntry {
                    accepted_seq,
                    archived: archive.archived,
                    source,
                },
            );
        }
        should_replace
    }

    fn refresh_reactions_for_message(&mut self, key: &(String, String), owner: &DeviceRef) {
        let summaries = self
            .messages
            .get(key)
            .map(|message| reaction_summaries_for_message(message, &self.reaction_senders, owner));
        if let Some(summaries) = summaries
            && let Some(message) = self.messages.get_mut(key)
        {
            message.reactions = summaries;
        }
    }

    fn refresh_poll_for_message(&mut self, key: &(String, String), owner: &DeviceRef) {
        let poll = self
            .messages
            .get(key)
            .and_then(|message| message.poll.as_ref())
            .map(|poll| poll_with_vote_summary(poll, &key.0, &key.1, &self.poll_votes, owner));
        if let Some(poll) = poll
            && let Some(message) = self.messages.get_mut(key)
        {
            message.poll = Some(poll);
        }
    }

    fn refresh_receipts_for_room(&mut self, room_id: &str) {
        let keys = self
            .messages
            .keys()
            .filter(|(message_room_id, _)| message_room_id == room_id)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            let summary = self.messages.get(&key).and_then(|message| {
                receipt_summary_for_message(message, &self.delivered_through, &self.read_through)
            });
            if let Some(message) = self.messages.get_mut(&key) {
                message.read_receipt = summary;
            }
        }
    }

    fn trim_to_limit(&mut self) {
        // The derived-index cleanup below rebuilds key sets over every stored
        // message, so it must only run when an eviction actually happened.
        // `apply_event` calls this after each replayed event; doing the
        // rebuild unconditionally made boot replay O(messages²) (#596).
        let mut evicted = false;
        while self.messages.len() > MAX_APP_MESSAGES {
            let Some(key) = self.message_arrival_order.pop_front() else {
                break;
            };
            self.messages.remove(&key);
            evicted = true;
        }
        if !evicted {
            return;
        }
        let message_keys = self.messages.keys().cloned().collect::<BTreeSet<_>>();
        self.reaction_senders.retain(|(room_id, message_id, _, _)| {
            message_keys.contains(&(room_id.clone(), message_id.clone()))
        });
        self.poll_votes.retain(|(room_id, message_id, _), _| {
            message_keys.contains(&(room_id.clone(), message_id.clone()))
        });
        let message_rooms = message_keys
            .into_iter()
            .map(|(room_id, _)| room_id)
            .collect::<BTreeSet<_>>();
        self.delivered_through
            .retain(|(room_id, _), _| message_rooms.contains(room_id));
        self.read_through
            .retain(|(room_id, _), _| message_rooms.contains(room_id));
    }
}

fn upsert_receipt_marker(
    markers: &mut BTreeMap<(String, String), u64>,
    key: (String, String),
    target_seq: u64,
) {
    let entry = markers.entry(key).or_default();
    if target_seq > *entry {
        *entry = target_seq;
    }
}

fn reaction_summaries_for_message(
    message: &ChatMessage,
    reaction_senders: &BTreeSet<(String, String, String, String)>,
    owner: &DeviceRef,
) -> Vec<ChatReactionSummary> {
    let owner_key = device_label(owner);
    let mut reactions = BTreeMap::<String, (u32, bool)>::new();
    for (room_id, message_id, emoji, sender_key) in reaction_senders {
        if room_id != &message.room_id || message_id != &message.message_id {
            continue;
        }
        let entry = reactions.entry(emoji.clone()).or_default();
        entry.0 = entry.0.saturating_add(1);
        entry.1 |= sender_key == &owner_key;
    }
    reactions
        .into_iter()
        .map(|(emoji, (count, reacted_by_me))| ChatReactionSummary {
            emoji,
            count,
            reacted_by_me,
        })
        .collect()
}

fn poll_with_vote_summary(
    poll: &ChatPoll,
    room_id: &str,
    message_id: &str,
    poll_votes: &BTreeMap<(String, String, String), String>,
    owner: &DeviceRef,
) -> ChatPoll {
    let option_ids = poll
        .options
        .iter()
        .map(|option| option.option_id.clone())
        .collect::<BTreeSet<_>>();
    let mut counts = BTreeMap::<String, u32>::new();
    let owner_key = device_label(owner);
    let mut my_vote_option_id = None;
    for ((vote_room_id, poll_message_id, voter_key), option_id) in poll_votes {
        if vote_room_id != room_id || poll_message_id != message_id {
            continue;
        }
        if !option_ids.contains(option_id) {
            continue;
        }
        counts
            .entry(option_id.clone())
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        if voter_key == &owner_key {
            my_vote_option_id = Some(option_id.clone());
        }
    }
    let mut total_votes = 0u32;
    let options = poll
        .options
        .iter()
        .map(|option| {
            let vote_count = counts.remove(&option.option_id).unwrap_or_default();
            total_votes = total_votes.saturating_add(vote_count);
            ChatPollOption {
                option_id: option.option_id.clone(),
                text: option.text.clone(),
                vote_count,
                voted_by_me: my_vote_option_id.as_deref() == Some(option.option_id.as_str()),
            }
        })
        .collect();
    ChatPoll {
        question: poll.question.clone(),
        options,
        total_votes,
        my_vote_option_id,
    }
}

fn receipt_summary_for_message(
    message: &ChatMessage,
    delivered_through: &BTreeMap<(String, String), u64>,
    read_through: &BTreeMap<(String, String), u64>,
) -> Option<ChatReadReceiptSummary> {
    let sender_key = format!("{}/{}", message.sender_account_id, message.sender_device_id);
    let mut delivered = BTreeSet::new();
    let mut read = BTreeSet::new();
    for ((room_id, device), through_seq) in delivered_through {
        if room_id == &message.room_id && *through_seq >= message.seq && device != &sender_key {
            delivered.insert(device.clone());
        }
    }
    for ((room_id, device), through_seq) in read_through {
        if room_id == &message.room_id && *through_seq >= message.seq && device != &sender_key {
            read.insert(device.clone());
            delivered.insert(device.clone());
        }
    }
    let read_count = bounded_u32_count(read.len());
    let delivered_count = bounded_u32_count(delivered.len());
    if read_count == 0 && delivered_count == 0 {
        return None;
    }
    let display_text = if read_count > 0 {
        format!("Read by {read_count}")
    } else {
        format!("Delivered to {delivered_count}")
    };
    Some(ChatReadReceiptSummary {
        delivered_count,
        read_count,
        display_text,
    })
}

fn bounded_u32_count(count: usize) -> u32 {
    count.min(u32::MAX as usize) as u32
}

fn stored_message_from_chat(message: &ChatMessage) -> StoredAppMessage {
    StoredAppMessage {
        room_id: message.room_id.clone(),
        seq: message.seq,
        message_id: message.message_id.clone(),
        sender: DeviceRef {
            account_id: message.sender_account_id.clone(),
            device_id: message.sender_device_id.clone(),
        },
        plaintext: message.payload.clone(),
        timestamp_unix_seconds: message.timestamp_unix_seconds,
    }
}

fn stored_event_from_chat(message: &ChatMessage) -> StoredAppEvent {
    StoredAppEvent {
        room_id: message.room_id.clone(),
        seq: message.seq,
        message_id: message.message_id.clone(),
        sender: DeviceRef {
            account_id: message.sender_account_id.clone(),
            device_id: message.sender_device_id.clone(),
        },
        plaintext: message.payload.clone(),
        timestamp_unix_seconds: message.timestamp_unix_seconds,
    }
}

fn app_room_from_stored(room: StoredAppRoom, has_mls_room: bool) -> AppRoomSummary {
    let mut state = app_room_state_from_stored(room.state);
    let mut status = room.status;
    if matches!(
        state,
        AppRoomState::Connected | AppRoomState::UnavailableOnDevice
    ) && !has_mls_room
    {
        state = AppRoomState::UnavailableOnDevice;
        status = LOCAL_ROOM_UNAVAILABLE_STATUS.to_owned();
    } else if state == AppRoomState::UnavailableOnDevice && has_mls_room {
        state = AppRoomState::Connected;
        status = "connected".to_owned();
    }
    let user_status_text = app_room_user_status_text_from_parts(&state, &status);
    AppRoomSummary {
        room_id: room.room_id,
        display_name: room.display_name,
        picture: room.picture,
        state,
        status,
        user_status_text,
        last_message_preview: String::new(),
        unread_count: 0,
        can_load_older: false,
        is_agent_chat: false,
    }
}

fn connected_app_room(room_id: &str, display_name: &str) -> AppRoomSummary {
    AppRoomSummary {
        room_id: room_id.to_owned(),
        display_name: display_name.to_owned(),
        picture: None,
        state: AppRoomState::Connected,
        status: "connected".to_owned(),
        user_status_text: LOCAL_ROOM_CONNECTED_TEXT.to_owned(),
        last_message_preview: String::new(),
        unread_count: 0,
        can_load_older: false,
        is_agent_chat: false,
    }
}

fn app_room_user_status_text(room: &AppRoomSummary) -> String {
    app_room_user_status_text_from_parts(&room.state, &room.status)
}

fn app_room_user_status_text_from_parts(state: &AppRoomState, _status: &str) -> String {
    match state {
        AppRoomState::Connected => LOCAL_ROOM_CONNECTED_TEXT.to_owned(),
        AppRoomState::WaitingForApproval => "Waiting for approval".to_owned(),
        AppRoomState::Joining => "Joining".to_owned(),
        AppRoomState::UnavailableOnDevice => LOCAL_ROOM_UNAVAILABLE_TEXT.to_owned(),
    }
}

fn scan_target_failure_message(error: &FiniteChatCoreError) -> String {
    let raw = error.to_string();
    if raw.to_ascii_lowercase().contains("profile") {
        "That profile could not be opened. Check the npub and try again.".to_owned()
    } else {
        "That code is not a valid profile code.".to_owned()
    }
}

fn selected_room_id_from_stored(
    rooms: &[AppRoomSummary],
    stored: &StoredAppState,
) -> Option<String> {
    if let Some(selected) = stored.selected_room_id.as_ref()
        && rooms.iter().any(|room| room.room_id == *selected)
    {
        return Some(selected.clone());
    }
    rooms.first().map(|room| room.room_id.clone())
}

fn stored_room_from_app(room: &AppRoomSummary, local_read_seq: u64) -> StoredAppRoom {
    StoredAppRoom {
        room_id: room.room_id.clone(),
        display_name: room.display_name.clone(),
        picture: room.picture.clone(),
        state: stored_app_room_state(&room.state),
        status: room.status.clone(),
        local_read_seq,
    }
}

fn app_room_state_from_stored(state: StoredAppRoomState) -> AppRoomState {
    match state {
        StoredAppRoomState::Connected => AppRoomState::Connected,
        StoredAppRoomState::WaitingForApproval => AppRoomState::WaitingForApproval,
        StoredAppRoomState::Joining => AppRoomState::Joining,
        StoredAppRoomState::UnavailableOnDevice => AppRoomState::UnavailableOnDevice,
    }
}

fn stored_app_room_state(state: &AppRoomState) -> StoredAppRoomState {
    match state {
        AppRoomState::Connected => StoredAppRoomState::Connected,
        AppRoomState::WaitingForApproval => StoredAppRoomState::WaitingForApproval,
        AppRoomState::Joining => StoredAppRoomState::Joining,
        AppRoomState::UnavailableOnDevice => StoredAppRoomState::UnavailableOnDevice,
    }
}

fn app_room_metadata(room_id: &str, display_name: Option<&str>) -> StoredAppRoom {
    let display_name = display_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(room_id)
        .to_owned();
    StoredAppRoom {
        room_id: room_id.to_owned(),
        display_name,
        picture: None,
        state: StoredAppRoomState::Connected,
        status: "connected".to_owned(),
        local_read_seq: 0,
    }
}

fn apply_room_message_projection(
    rooms: &mut [AppRoomSummary],
    messages: &[ChatMessage],
    local_read_seq: &BTreeMap<String, u64>,
) {
    let mut previews = BTreeMap::new();
    let mut unread_counts = BTreeMap::<String, u32>::new();
    for message in messages {
        previews.insert(message.room_id.clone(), message_preview(message));
        let read_seq = local_read_seq
            .get(&message.room_id)
            .copied()
            .unwrap_or_default();
        if !message.is_mine && message.seq > read_seq {
            unread_counts
                .entry(message.room_id.clone())
                .and_modify(|count| *count = count.saturating_add(1))
                .or_insert(1);
        }
    }

    for room in rooms {
        room.last_message_preview = previews.remove(&room.room_id).unwrap_or_default();
        room.unread_count = unread_counts.remove(&room.room_id).unwrap_or_default();
    }
}

fn message_preview(message: &ChatMessage) -> String {
    let text = message.text.trim();
    if !text.is_empty() {
        return text.to_owned();
    }
    let Some(attachment) = message.media.first() else {
        return String::new();
    };
    if !attachment.filename.trim().is_empty() {
        return attachment.filename.clone();
    }
    match &attachment.kind {
        ChatMediaKind::Image => "Image".to_owned(),
        ChatMediaKind::VoiceNote => "Voice note".to_owned(),
        ChatMediaKind::Video => "Video".to_owned(),
        ChatMediaKind::File => "File".to_owned(),
    }
}

fn topic_summary_from_projection(
    entry: &ConversationProjectionEntry,
    messages: Vec<&ChatMessage>,
    local_read_seq: &BTreeMap<String, u64>,
    chat_archives: &BTreeMap<(String, String, String), ChatArchiveProjectionEntry>,
    chat_titles: &BTreeMap<(String, String, String), ChatTitleProjectionEntry>,
) -> AppTopicSummary {
    let metadata = entry.metadata.as_ref();
    let last_message_preview = latest_message_preview(&messages);
    let title = metadata
        .and_then(|metadata| metadata.title.as_deref())
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            metadata
                .and_then(|metadata| metadata.external_topic.as_ref())
                .and_then(|external| external.topic_name.as_deref())
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| topic_fallback_title(&entry.conversation_id, &last_message_preview));
    let latest_seq = messages
        .iter()
        .map(|message| message.seq)
        .max()
        .unwrap_or(entry.updated_seq);
    let active_chat_id = entry
        .active_segment_id
        .clone()
        .or_else(|| (entry.conversation_id == HOME_TOPIC_ID).then(|| HOME_CHAT_ID.to_owned()));
    let mut chats =
        chat_summaries_for_topic(entry, &messages, local_read_seq, chat_archives, chat_titles);
    ensure_default_home_chat(
        &entry.room_id,
        &entry.conversation_id,
        active_chat_id.as_deref(),
        &mut chats,
        local_read_seq,
        chat_archives,
        chat_titles,
    );
    AppTopicSummary {
        room_id: entry.room_id.clone(),
        topic_id: entry.conversation_id.clone(),
        title,
        description: metadata.and_then(|metadata| metadata.description.clone()),
        last_message_preview,
        unread_count: topic_unread_count(&entry.room_id, &messages, local_read_seq),
        message_count: messages.len().min(u32::MAX as usize) as u32,
        created_seq: entry.created_seq,
        updated_seq: entry.updated_seq.max(latest_seq),
        archived: entry.archived,
        active_chat_id,
        chats,
    }
}

fn topic_summary_from_messages(
    room_id: String,
    topic_id: String,
    messages: Vec<&ChatMessage>,
    local_read_seq: &BTreeMap<String, u64>,
    chat_archives: &BTreeMap<(String, String, String), ChatArchiveProjectionEntry>,
    chat_titles: &BTreeMap<(String, String, String), ChatTitleProjectionEntry>,
) -> AppTopicSummary {
    let last_message_preview = latest_message_preview(&messages);
    let created_seq = messages
        .iter()
        .map(|message| message.seq)
        .min()
        .unwrap_or_default();
    let updated_seq = messages
        .iter()
        .map(|message| message.seq)
        .max()
        .unwrap_or_default();
    let active_chat_id = (topic_id == HOME_TOPIC_ID).then(|| HOME_CHAT_ID.to_owned());
    let mut chats = message_only_chat_summaries(
        &room_id,
        &topic_id,
        &messages,
        local_read_seq,
        chat_archives,
        chat_titles,
    );
    ensure_default_home_chat(
        &room_id,
        &topic_id,
        active_chat_id.as_deref(),
        &mut chats,
        local_read_seq,
        chat_archives,
        chat_titles,
    );
    AppTopicSummary {
        room_id: room_id.clone(),
        topic_id: topic_id.clone(),
        title: topic_fallback_title(&topic_id, &last_message_preview),
        description: None,
        last_message_preview,
        unread_count: topic_unread_count(&room_id, &messages, local_read_seq),
        message_count: messages.len().min(u32::MAX as usize) as u32,
        created_seq,
        updated_seq,
        archived: false,
        active_chat_id,
        chats,
    }
}

fn ensure_default_home_chat(
    room_id: &str,
    topic_id: &str,
    active_chat_id: Option<&str>,
    chats: &mut Vec<AppChatSummary>,
    local_read_seq: &BTreeMap<String, u64>,
    chat_archives: &BTreeMap<(String, String, String), ChatArchiveProjectionEntry>,
    chat_titles: &BTreeMap<(String, String, String), ChatTitleProjectionEntry>,
) {
    if topic_id != HOME_TOPIC_ID || chats.iter().any(|chat| chat.chat_id == HOME_CHAT_ID) {
        return;
    }
    let context = ChatSummaryContext {
        room_id,
        topic_id,
        active_chat_id,
        local_read_seq,
        chat_archives,
        chat_titles,
    };
    chats.push(chat_summary_from_parts(&context, HOME_CHAT_ID, 0, 0, &[]));
    chats.sort_by(chat_sort);
}

fn chat_summaries_for_topic(
    entry: &ConversationProjectionEntry,
    messages: &[&ChatMessage],
    local_read_seq: &BTreeMap<String, u64>,
    chat_archives: &BTreeMap<(String, String, String), ChatArchiveProjectionEntry>,
    chat_titles: &BTreeMap<(String, String, String), ChatTitleProjectionEntry>,
) -> Vec<AppChatSummary> {
    let mut messages_by_chat = BTreeMap::<String, Vec<&ChatMessage>>::new();
    for message in messages {
        let Some(chat_id) = message.chat_id.as_ref() else {
            continue;
        };
        messages_by_chat
            .entry(chat_id.clone())
            .or_default()
            .push(*message);
    }

    let context = ChatSummaryContext {
        room_id: &entry.room_id,
        topic_id: &entry.conversation_id,
        active_chat_id: entry.active_segment_id.as_deref(),
        local_read_seq,
        chat_archives,
        chat_titles,
    };
    let mut chats = Vec::new();
    for (index, segment) in entry.segments.iter().enumerate() {
        let chat_messages = messages_by_chat
            .remove(&segment.segment_id)
            .unwrap_or_default();
        chats.push(chat_summary_from_parts(
            &context,
            &segment.segment_id,
            index,
            segment.started_seq,
            &chat_messages,
        ));
    }

    for (index, (chat_id, chat_messages)) in messages_by_chat.into_iter().enumerate() {
        let started_seq = chat_messages
            .iter()
            .map(|message| message.seq)
            .min()
            .unwrap_or(entry.created_seq);
        chats.push(chat_summary_from_parts(
            &context,
            &chat_id,
            entry.segments.len() + index,
            started_seq,
            &chat_messages,
        ));
    }

    chats.sort_by(chat_sort);
    chats
}

fn message_only_chat_summaries(
    room_id: &str,
    topic_id: &str,
    messages: &[&ChatMessage],
    local_read_seq: &BTreeMap<String, u64>,
    chat_archives: &BTreeMap<(String, String, String), ChatArchiveProjectionEntry>,
    chat_titles: &BTreeMap<(String, String, String), ChatTitleProjectionEntry>,
) -> Vec<AppChatSummary> {
    let mut messages_by_chat = BTreeMap::<String, Vec<&ChatMessage>>::new();
    for message in messages {
        let Some(chat_id) = message.chat_id.as_ref() else {
            continue;
        };
        messages_by_chat
            .entry(chat_id.clone())
            .or_default()
            .push(*message);
    }
    let context = ChatSummaryContext {
        room_id,
        topic_id,
        active_chat_id: None,
        local_read_seq,
        chat_archives,
        chat_titles,
    };
    let mut chats = messages_by_chat
        .into_iter()
        .enumerate()
        .map(|(index, (chat_id, chat_messages))| {
            let started_seq = chat_messages
                .iter()
                .map(|message| message.seq)
                .min()
                .unwrap_or_default();
            chat_summary_from_parts(&context, &chat_id, index, started_seq, &chat_messages)
        })
        .collect::<Vec<_>>();
    chats.sort_by(chat_sort);
    chats
}

struct ChatSummaryContext<'a> {
    room_id: &'a str,
    topic_id: &'a str,
    active_chat_id: Option<&'a str>,
    local_read_seq: &'a BTreeMap<String, u64>,
    chat_archives: &'a BTreeMap<(String, String, String), ChatArchiveProjectionEntry>,
    chat_titles: &'a BTreeMap<(String, String, String), ChatTitleProjectionEntry>,
}

fn chat_summary_from_parts(
    context: &ChatSummaryContext<'_>,
    chat_id: &str,
    _index: usize,
    started_seq: u64,
    messages: &[&ChatMessage],
) -> AppChatSummary {
    let last_message_preview = latest_message_preview(messages);
    let updated_seq = messages
        .iter()
        .map(|message| message.seq)
        .max()
        .unwrap_or(started_seq);
    AppChatSummary {
        chat_id: chat_id.to_owned(),
        title: chat_title(
            context.room_id,
            context.topic_id,
            chat_id,
            messages,
            context.chat_titles,
        ),
        last_message_preview,
        unread_count: topic_unread_count(context.room_id, messages, context.local_read_seq),
        message_count: messages.len().min(u32::MAX as usize) as u32,
        started_seq,
        updated_seq,
        active: context.active_chat_id == Some(chat_id),
        archived: context
            .chat_archives
            .get(&(
                context.room_id.to_owned(),
                context.topic_id.to_owned(),
                chat_id.to_owned(),
            ))
            .is_some_and(|entry| entry.archived),
    }
}

fn chat_title(
    room_id: &str,
    topic_id: &str,
    chat_id: &str,
    messages: &[&ChatMessage],
    chat_titles: &BTreeMap<(String, String, String), ChatTitleProjectionEntry>,
) -> String {
    let key = (room_id.to_owned(), topic_id.to_owned(), chat_id.to_owned());
    if let Some(explicit) = chat_titles.get(&key) {
        return explicit.title.clone();
    }

    messages
        .iter()
        .filter(|message| {
            matches!(
                message.kind,
                ChatMessageKind::Message | ChatMessageKind::Media
            ) && message.edit_of_message_id.is_none()
        })
        .filter_map(|message| {
            let preview = message_preview(message);
            (!preview.trim().is_empty()).then_some((message, preview))
        })
        .min_by(|(left, _), (right, _)| message_sort(left, right))
        .map(|(_, preview)| preview.chars().take(48).collect())
        .unwrap_or_else(|| "New chat".to_owned())
}

fn latest_message_preview(messages: &[&ChatMessage]) -> String {
    messages
        .iter()
        .max_by(|left, right| message_sort(left, right))
        .map(|message| message_preview(message))
        .unwrap_or_default()
}

fn topic_fallback_title(topic_id: &str, last_message_preview: &str) -> String {
    let preview = last_message_preview.trim();
    if !preview.is_empty() {
        return preview.chars().take(48).collect();
    }
    topic_id.to_owned()
}

fn topic_unread_count(
    room_id: &str,
    messages: &[&ChatMessage],
    local_read_seq: &BTreeMap<String, u64>,
) -> u32 {
    let read_seq = local_read_seq.get(room_id).copied().unwrap_or_default();
    messages
        .iter()
        .filter(|message| !message.is_mine && message.seq > read_seq)
        .count()
        .min(u32::MAX as usize) as u32
}

fn topic_sort(left: &AppTopicSummary, right: &AppTopicSummary) -> std::cmp::Ordering {
    right
        .updated_seq
        .cmp(&left.updated_seq)
        .then_with(|| left.room_id.cmp(&right.room_id))
        .then_with(|| left.title.cmp(&right.title))
        .then_with(|| left.topic_id.cmp(&right.topic_id))
}

fn chat_sort(left: &AppChatSummary, right: &AppChatSummary) -> std::cmp::Ordering {
    right
        .updated_seq
        .cmp(&left.updated_seq)
        .then_with(|| left.started_seq.cmp(&right.started_seq))
        .then_with(|| left.chat_id.cmp(&right.chat_id))
}

fn sort_app_rooms(rooms: &mut [AppRoomSummary]) {
    rooms.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.room_id.cmp(&right.room_id))
    });
}

fn message_sort(left: &ChatMessage, right: &ChatMessage) -> std::cmp::Ordering {
    left.seq
        .cmp(&right.seq)
        .then_with(|| left.room_id.cmp(&right.room_id))
        .then_with(|| left.message_id.cmp(&right.message_id))
}

fn message_key(message: &ChatMessage) -> (String, String) {
    (message.room_id.clone(), message.message_id.clone())
}

fn chat_media_gallery_item_id(message: &ChatMessage, attachment: &ChatMediaAttachment) -> String {
    format!(
        "{}|{}|{}",
        message.room_id, message.message_id, attachment.attachment_id
    )
}

/// Recorded as `created_by` in `identity.json` when this build mints the
/// shared Finite identity.
const FINITE_IDENTITY_CREATED_BY: &str = concat!("finitechat ", env!("CARGO_PKG_VERSION"));

/// Resolve the account secret per the Finite Identity Contract v1.
///
/// Callers that already hold key material (iOS keychain identities, tests)
/// pass it explicitly; it is held in memory only and never written to the
/// data dir. Otherwise the shared identity at `$FINITE_HOME/identity/`
/// (or `~/.finite/identity/`) is loaded, minting under the contract's
/// exclusive lock if this is the first Finite tool to run. Legacy per-store
/// `account-secret.hex` files are deliberately never read (hard cut).
fn resolve_account_secret(provided: Option<&str>) -> Result<NostrSecretKey, FiniteChatCoreError> {
    if let Some(secret) = provided {
        return parse_account_secret_hex(secret);
    }
    let paths = finite_identity::IdentityPaths::resolve().map_err(finite_identity_error)?;
    let identity =
        finite_identity::FiniteIdentity::load_or_generate(&paths, FINITE_IDENTITY_CREATED_BY)
            .map_err(finite_identity_error)?;
    NostrSecretKey::from_bytes(identity.expose_secret_bytes())
        .map_err(|_| FiniteChatCoreError::InvalidAccountSecret)
}

fn finite_identity_error(error: finite_identity::Error) -> FiniteChatCoreError {
    FiniteChatCoreError::Filesystem {
        reason: format!("finite identity: {error}"),
    }
}

fn parse_account_secret_hex(secret: &str) -> Result<NostrSecretKey, FiniteChatCoreError> {
    let bytes =
        hex::decode(secret.trim()).map_err(|_| FiniteChatCoreError::InvalidAccountSecret)?;
    let bytes: [u8; NOSTR_SECRET_KEY_BYTES] = bytes
        .try_into()
        .map_err(|_| FiniteChatCoreError::InvalidAccountSecret)?;
    NostrSecretKey::from_bytes(bytes).map_err(|_| FiniteChatCoreError::InvalidAccountSecret)
}

fn validate_finite_sites_native_session_fields(
    url: &str,
    return_to: &str,
    client: &str,
    nonce: &str,
) -> Result<(), FiniteChatCoreError> {
    validate_finite_sites_native_session_url(url)?;
    if !valid_finite_sites_return_to(return_to) {
        return Err(FiniteChatCoreError::Client {
            reason: "invalid finite-sites return path".to_owned(),
        });
    }
    if !valid_finite_sites_token_like(client, 1, MAX_FINITE_SITES_NATIVE_CLIENT_BYTES) {
        return Err(FiniteChatCoreError::Client {
            reason: "invalid finite-sites client id".to_owned(),
        });
    }
    if !valid_finite_sites_token_like(
        nonce,
        MIN_FINITE_SITES_NATIVE_NONCE_BYTES,
        MAX_FINITE_SITES_NATIVE_NONCE_BYTES,
    ) {
        return Err(FiniteChatCoreError::Client {
            reason: "invalid finite-sites native auth nonce".to_owned(),
        });
    }
    Ok(())
}

fn validate_finite_sites_native_session_url(url: &str) -> Result<(), FiniteChatCoreError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| FiniteChatCoreError::Client {
        reason: "invalid finite-sites native auth URL".to_owned(),
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(FiniteChatCoreError::Client {
            reason: "finite-sites native auth URL must be absolute HTTP(S)".to_owned(),
        });
    }
    if parsed.path() != FINITE_SITES_NATIVE_SESSION_PATH
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(FiniteChatCoreError::Client {
            reason: "finite-sites native auth URL must target the native session endpoint"
                .to_owned(),
        });
    }
    Ok(())
}

fn valid_finite_sites_return_to(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_FINITE_SITES_NATIVE_RETURN_TO_BYTES {
        return false;
    }
    if !value.starts_with('/') || value.starts_with("//") {
        return false;
    }
    !bytes.iter().any(|byte| byte.is_ascii_control())
}

fn valid_finite_sites_token_like(value: &str, min_len: usize, max_len: usize) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < min_len || bytes.len() > max_len {
        return false;
    }
    bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

fn build_nip98_auth_header(
    secret: &NostrSecretKey,
    url: &str,
    method: &str,
    body: Option<&[u8]>,
    now_unix_seconds: u64,
) -> Result<String, FiniteChatCoreError> {
    let mut tags = vec![
        vec!["u".to_owned(), url.to_owned()],
        vec!["method".to_owned(), method.to_owned()],
    ];
    if let Some(body) = body {
        let digest = Sha256::digest(body);
        tags.push(vec!["payload".to_owned(), hex::encode(digest)]);
    }
    let event = sign_nostr_auth_event(
        secret,
        now_unix_seconds,
        FINITE_SITES_NIP98_KIND,
        tags,
        String::new(),
    )?;
    let encoded =
        BASE64.encode(serde_json::to_vec(&event).expect("nostr auth event always serializes"));
    Ok(format!("{FINITE_SITES_NIP98_AUTH_SCHEME}{encoded}"))
}

fn sign_nostr_auth_event(
    secret: &NostrSecretKey,
    created_at: u64,
    kind: u32,
    tags: Vec<Vec<String>>,
    content: String,
) -> Result<NostrHttpAuthEvent, FiniteChatCoreError> {
    let pubkey = hex::encode(secret.public_key().as_bytes());
    let id_digest = nostr_event_id_digest(&pubkey, created_at, kind, &tags, &content)?;
    let signature = secret.sign_schnorr_digest(id_digest);
    Ok(NostrHttpAuthEvent {
        id: hex::encode(id_digest),
        pubkey,
        created_at,
        kind,
        tags,
        content,
        sig: hex::encode(signature),
    })
}

fn nostr_event_id_digest(
    pubkey: &str,
    created_at: u64,
    kind: u32,
    tags: &[Vec<String>],
    content: &str,
) -> Result<[u8; 32], FiniteChatCoreError> {
    let canonical = serde_json::json!([0, pubkey, created_at, kind, tags, content]);
    let serialized =
        serde_json::to_string(&canonical).map_err(|error| FiniteChatCoreError::Client {
            reason: format!("failed to serialize nostr auth event id: {error}"),
        })?;
    let digest = Sha256::digest(serialized.as_bytes());
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&digest);
    Ok(bytes)
}

fn nostr_identity_from_secret(
    secret: NostrSecretKey,
) -> Result<NostrIdentityMaterial, FiniteChatCoreError> {
    let account_secret_hex = hex::encode(secret.as_bytes());
    let account_id = hex::encode(secret.public_key().as_bytes());
    let npub = npub_encode(&account_id).map_err(profile_error)?;
    let nsec = nsec_encode(&account_secret_hex).map_err(profile_error)?;
    Ok(NostrIdentityMaterial {
        account_secret_hex,
        account_id,
        npub,
        nsec,
    })
}

/// Process-wide blocking HTTP client. `reqwest::blocking::Client::new()`
/// allocates a fresh connection pool (sockets, an event loop thread) and
/// PANICS if the builder fails — under fd exhaustion every per-call
/// construction both deepens the exhaustion and turns it into a panic storm
/// (2026-08-12: ~275k panics in 8 minutes from the hosted-device long-poll
/// loops). Construct once, clone the handle (it is an Arc internally).
fn shared_blocking_http_client() -> reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::blocking::Client::new).clone()
}

fn delivery_for(server_url: &str) -> HttpRuntimeDelivery<ReqwestHttpRuntimeTransport> {
    delivery_for_with_client(server_url, shared_blocking_http_client())
}

fn delivery_for_with_client(
    server_url: &str,
    client: reqwest::blocking::Client,
) -> HttpRuntimeDelivery<ReqwestHttpRuntimeTransport> {
    HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::with_client(server_url, client))
}

fn device_room_counts<D: RuntimeDelivery>(
    delivery: &mut D,
    target: &DeviceRef,
) -> Result<(u32, u32), D::Error> {
    let mut after_room_id = None;
    let mut room_count = 0_u32;
    let mut active_room_count = 0_u32;
    for _ in 0..16 {
        let page = delivery.list_account_rooms(ListAccountRoomsRequest {
            account_id: target.account_id.clone(),
            after_room_id: after_room_id.clone(),
            limit: 100,
        })?;
        for room in page.rooms {
            if let Some(device) = room.devices.iter().find(|device| device.device == *target) {
                room_count = room_count.saturating_add(1);
                if device.active {
                    active_room_count = active_room_count.saturating_add(1);
                }
            }
        }
        if !page.has_more {
            break;
        }
        let Some(next) = page.next_after_room_id else {
            break;
        };
        if after_room_id.as_ref() == Some(&next) {
            break;
        }
        after_room_id = Some(next);
    }
    Ok((room_count, active_room_count))
}

fn verify_server_contract(server_url: &str) -> Result<(), FiniteChatCoreError> {
    let health_url = format!("{}/health", server_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(SERVER_CONTRACT_HEALTH_TIMEOUT_SECS))
        .build()
        .map_err(delivery_error)?;
    let response = client.get(&health_url).send().map_err(delivery_error)?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(FiniteChatCoreError::ServerRejected {
            reason: format!(
                "server contract check failed at {health_url}: server returned {status}: {body}"
            ),
        });
    }
    let health = response.json::<HealthResponse>().map_err(delivery_error)?;
    validate_server_contract_health(server_url, &health)
}

fn validate_server_contract_health(
    server_url: &str,
    health: &HealthResponse,
) -> Result<(), FiniteChatCoreError> {
    if health.status != "ok" {
        return Err(FiniteChatCoreError::ServerRejected {
            reason: format!(
                "server {server_url} reported health status '{}'; expected ok",
                health.status
            ),
        });
    }
    match health.server_contract_version {
        Some(actual) if actual >= FINITECHAT_SERVER_CONTRACT_VERSION => Ok(()),
        Some(actual) => Err(FiniteChatCoreError::ServerRejected {
            reason: format!(
                "server {server_url} reports finitechat {} contract {actual}; this client requires server contract at least {}. Deploy a compatible finitechat-server before syncing rooms.",
                server_build_label(health),
                FINITECHAT_SERVER_CONTRACT_VERSION
            ),
        }),
        None => Err(FiniteChatCoreError::ServerRejected {
            reason: format!(
                "server {server_url} reports finitechat {} without a server_contract_version; this client expects contract {}. Deploy a compatible finitechat-server before syncing rooms.",
                server_build_label(health),
                FINITECHAT_SERVER_CONTRACT_VERSION
            ),
        }),
    }
}

fn server_build_label(health: &HealthResponse) -> String {
    let version = health
        .server_version
        .as_deref()
        .unwrap_or("unknown-version");
    let source = health
        .source_fingerprint
        .as_deref()
        .or(health.source_commit.as_deref())
        .unwrap_or("unknown-source");
    let dirty = match health.source_dirty {
        Some(true) => " dirty",
        Some(false) => "",
        None => " unknown-dirty",
    };
    format!("{version}@{source}{dirty}")
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn compact_error_reason(error: &FiniteChatCoreError) -> String {
    const MAX_REASON_CHARS: usize = 240;
    let mut reason = error.to_string();
    if reason.chars().count() <= MAX_REASON_CHARS {
        return reason;
    }
    reason = reason.chars().take(MAX_REASON_CHARS).collect();
    reason.push_str("...");
    reason
}

fn ready_status(room_sync_failure_count: usize) -> String {
    match room_sync_failure_count {
        0 => "ready".to_owned(),
        1 => "ready; 1 room needs repair".to_owned(),
        count => format!("ready; {count} rooms need repair"),
    }
}

fn online_action_failure(error: &FiniteChatCoreError) -> bool {
    matches!(
        error,
        FiniteChatCoreError::Delivery { .. } | FiniteChatCoreError::ServerRejected { .. }
    )
}

fn device_label(device: &DeviceRef) -> String {
    format!("{}/{}", device.account_id, device.device_id)
}

fn client_error(error: impl std::fmt::Display) -> FiniteChatCoreError {
    FiniteChatCoreError::Client {
        reason: error.to_string(),
    }
}

fn send_error(room_id: &str, error: ClientError) -> FiniteChatCoreError {
    match error {
        ClientError::GroupNotFound(_) => FiniteChatCoreError::Client {
            reason: format!(
                "this device has not created or joined room '{room_id}' yet; create the room on this device, or claim a Welcome before sending"
            ),
        },
        other => client_error(other),
    }
}

fn delivery_error(error: impl std::fmt::Display) -> FiniteChatCoreError {
    FiniteChatCoreError::Delivery {
        reason: error.to_string(),
    }
}

/// Deterministic idempotency key for one device's home-topic create in one
/// room. A double-fire by the same device (the poison-entry incident class:
/// two processes sharing one store, or a restarted writer whose local
/// projection predates the entry) dedupes at the server: an identical request
/// replays the original acceptance, and a ratchet-diverged one fails closed
/// with an idempotency conflict instead of appending an unprocessable second
/// entry. The sender is part of the key because a member that joined after
/// the topic was created cannot decrypt the earlier entry and must publish
/// its own ConversationCreate to learn the topic.
fn home_topic_idempotency_key(room_id: &str, sender: &DeviceRef) -> String {
    format!(
        "home-topic:{room_id}:{}:{}",
        sender.account_id, sender.device_id
    )
}

/// The server answers a reused idempotency key naming a different payload
/// with 409 `idempotency_conflict`; an exact replay returns the original
/// acceptance instead.
fn is_idempotency_conflict_rejection(
    error: &HttpRuntimeDeliveryError<ReqwestHttpRuntimeTransportError>,
) -> bool {
    matches!(
        error,
        HttpRuntimeDeliveryError::Transport(ReqwestHttpRuntimeTransportError::Server {
            status,
            body,
        }) if status.as_u16() == 409 && body.contains("\"idempotency_conflict\"")
    )
}

fn send_delivery_error(
    error: HttpRuntimeDeliveryError<ReqwestHttpRuntimeTransportError>,
) -> FiniteChatCoreError {
    match error {
        HttpRuntimeDeliveryError::Transport(ReqwestHttpRuntimeTransportError::Server {
            status,
            body,
        }) => FiniteChatCoreError::ServerRejected {
            reason: format!("server returned {status}: {body}"),
        },
        other => delivery_error(other),
    }
}

fn runtime_error(error: impl std::fmt::Display) -> FiniteChatCoreError {
    FiniteChatCoreError::Delivery {
        reason: error.to_string(),
    }
}

fn store_error(error: impl std::fmt::Display) -> FiniteChatCoreError {
    FiniteChatCoreError::Store {
        reason: error.to_string(),
    }
}

fn profile_error(error: impl std::fmt::Display) -> FiniteChatCoreError {
    FiniteChatCoreError::Profile {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finitechat_http::{
        ApplicationEffectRequest, GetNostrProfilesRequest, HttpApplicationDeliveryEffect,
        NostrProfileRecord, PutNostrProfileRequest,
    };
    use finitechat_server::{HttpServerState, http_router};
    use std::time::{Duration, Instant};

    const NOW: u64 = 1_800_000_000;

    #[test]
    fn application_envelope_parsing_stays_within_decode_authority() {
        // ChatProjectionState consumes ParsedAppEvent, so application
        // plaintext is decoded once at the boundary and never re-parsed
        // below it. This gate makes a NEW parse of the application envelope
        // a conscious, reviewed change: the production region of this file
        // may parse DecryptedApplicationEventV1 only in
        //
        //   - decode_application_plaintext (the decode authority),
        //   - typed_namespaced_payload (the namespaced-event authority),
        //   - apply_conversation_projection_from_message (the documented
        //     fallback for appending already-projected messages that have
        //     no carried scope).
        //
        // Adding a fourth site means re-introducing re-parsing; thread
        // ParsedAppEvent/ChatEventScope instead, or — if genuinely new —
        // update this count with a comment naming the new authority.
        let source = include_str!("lib.rs");
        let production = source
            .split("\nmod tests {")
            .next()
            .expect("tests module delimits the production region");
        let sites = production
            .match_indices("from_slice::<DecryptedApplicationEventV1>")
            .count();
        assert_eq!(
            sites, 3,
            "application envelope is parsed outside the sanctioned decode sites"
        );
    }

    #[test]
    fn text_payload_carries_requester_context_only_when_hosted_dispatch_supplies_it() {
        let plain =
            encode_text_message_payload_scoped("hello", None, None, None, None, None).unwrap();
        let plain = HermesMessagePayloadV1::decode(&plain).unwrap().unwrap();
        assert!(
            !plain
                .metadata
                .contains_key(finitechat_hermes::HERMES_METADATA_REQUESTER_EMAIL)
        );

        let requester = VerifiedRequesterContext {
            email: "paul@finite.vip".to_owned(),
            sites_assertion: "assertion-1".to_owned(),
        };
        let hosted =
            encode_text_message_payload_scoped("publish", None, None, None, None, Some(&requester))
                .unwrap();
        let hosted = HermesMessagePayloadV1::decode(&hosted).unwrap().unwrap();
        assert_eq!(
            hosted
                .metadata
                .get(finitechat_hermes::HERMES_METADATA_REQUESTER_EMAIL)
                .and_then(serde_json::Value::as_str),
            Some("paul@finite.vip")
        );
        assert_eq!(
            hosted
                .metadata
                .get(finitechat_hermes::HERMES_METADATA_SITES_REQUESTER_ASSERTION)
                .and_then(serde_json::Value::as_str),
            Some("assertion-1")
        );
    }

    #[test]
    fn message_metadata_round_trips_and_requester_claims_never_surface() {
        let metadata_json = r#"{"approve":{"service":"brain","requests":[{"brainId":"brain-1","requestId":"approval-1"}]}}"#;
        let requester = VerifiedRequesterContext {
            email: "paul@finite.vip".to_owned(),
            sites_assertion: "assertion-1".to_owned(),
        };
        let encoded = encode_text_message_payload_scoped(
            "approved",
            None,
            None,
            None,
            Some(metadata_json),
            Some(&requester),
        )
        .unwrap();

        let projected = chat_projection_payload(&resolve_chat_message_payload(&encoded));
        assert_eq!(
            projected.metadata.get("approve"),
            Some(&serde_json::json!({
                "service": "brain",
                "requests": [{"brainId": "brain-1", "requestId": "approval-1"}],
            }))
        );
        assert!(
            !projected
                .metadata
                .contains_key(finitechat_hermes::HERMES_METADATA_REQUESTER_EMAIL)
        );
        assert!(
            !projected
                .metadata
                .contains_key(finitechat_hermes::HERMES_METADATA_SITES_REQUESTER_ASSERTION)
        );
        assert_eq!(
            chat_metadata_json(&projected.metadata),
            serde_json::to_string(&projected.metadata).unwrap()
        );

        // A spoofed requester claim in caller metadata is overwritten by the
        // verified requester context.
        let spoofed = encode_text_message_payload_scoped(
            "publish",
            None,
            None,
            None,
            Some(
                &serde_json::json!({
                    finitechat_hermes::HERMES_METADATA_REQUESTER_EMAIL: "mallory@evil.example"
                })
                .to_string(),
            ),
            Some(&requester),
        )
        .unwrap();
        let spoofed = HermesMessagePayloadV1::decode(&spoofed).unwrap().unwrap();
        assert_eq!(
            spoofed
                .metadata
                .get(finitechat_hermes::HERMES_METADATA_REQUESTER_EMAIL)
                .and_then(serde_json::Value::as_str),
            Some("paul@finite.vip")
        );

        // Non-object metadata is rejected fail-closed rather than dropped.
        assert!(
            encode_text_message_payload_scoped("publish", None, None, None, Some("[1,2,3]"), None)
                .is_err()
        );
    }

    fn next_permutation(values: &mut [usize]) -> bool {
        let Some(pivot) = (1..values.len())
            .rev()
            .find(|&index| values[index - 1] < values[index])
            .map(|index| index - 1)
        else {
            return false;
        };
        let successor = (pivot + 1..values.len())
            .rev()
            .find(|&index| values[pivot] < values[index])
            .expect("a permutation pivot always has a successor");
        values.swap(pivot, successor);
        values[pivot + 1..].reverse();
        true
    }

    #[test]
    fn startup_finalizes_durable_device_link_staging_and_exposes_exact_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("linked-device");
        let options = with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "linked-device".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        });
        let mut core = CoreState::open(options.clone()).unwrap();
        core.device
            .create_group_state("room-durable-link", "mls-room-durable-link")
            .unwrap();
        core.store.save_device_state(&core.device).unwrap();
        let owner = core.device.device_ref().clone();
        let source = DeviceRef {
            account_id: owner.account_id.clone(),
            device_id: "source-device".to_owned(),
        };
        let history = vec![DeviceLinkBootstrapEventV2 {
            seq: 1,
            message_id: "history-before-link".to_owned(),
            sender: source.clone(),
            plaintext: encode_application_event(
                DurableAppEventKind::ChatMessage,
                None,
                &encode_text_message_payload("history before link", None).unwrap(),
            )
            .unwrap(),
            timestamp_unix_seconds: NOW,
        }];
        let mut transfer_digest = Sha256::new();
        transfer_digest.update(device_link_bootstrap_chunk_sha256(&history));
        let bootstrap = DeviceLinkBootstrapV2 {
            version: DEVICE_LINK_BOOTSTRAP_VERSION_V2,
            bootstrap_id: "durable-link-bootstrap".to_owned(),
            target: owner.clone(),
            chunk_index: 0,
            chunk_count: 1,
            total_history_events: 1,
            history_sha256: hex::encode(transfer_digest.finalize()),
            room: DeviceLinkBootstrapRoomV2 {
                room_id: "room-durable-link".to_owned(),
                display_name: "Durable linked room".to_owned(),
                picture: None,
            },
            canonical_selection: None,
            profiles: Vec::new(),
            history,
        };
        assert!(matches!(
            core.store
                .stage_device_link_bootstrap_chunk(&owner, &source, 10, &bootstrap)
                .unwrap(),
            DeviceLinkBootstrapStageOutcome::Ready(_)
        ));
        drop(core);

        let state = AppRuntimeState::new(CoreState::open(options).unwrap()).unwrap();
        let expected =
            StoredDeviceLinkBootstrapReceipt::from_bootstrap(&source, &bootstrap).unwrap();
        assert_eq!(
            state.app.device_link_bootstrap_receipts,
            vec![app_device_link_bootstrap_receipt(&expected)]
        );
        assert_eq!(
            state
                .app
                .rooms
                .iter()
                .find(|room| room.room_id == "room-durable-link")
                .unwrap()
                .display_name,
            "Durable linked room"
        );
        assert!(
            state
                .core
                .store
                .has_app_event(&owner, "room-durable-link", "history-before-link",)
                .unwrap()
        );
    }

    #[test]
    #[ignore = "timing harness; run explicitly in release mode"]
    fn app_runtime_idle_tick_with_full_projection_history() {
        const HISTORY_ITEMS: usize = 5_000;
        const SAMPLES: usize = 10;

        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let mut core = CoreState::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("app").to_string_lossy().into_owned(),
            server_url,
            device_id: "perf-electron".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let owner = core.device.device_ref().clone();
        let plaintext = encode_application_event(
            DurableAppEventKind::ChatMessage,
            None,
            &encode_text_message_payload("projection history", None).unwrap(),
        )
        .unwrap();
        let messages = (0..HISTORY_ITEMS)
            .map(|index| StoredAppMessage {
                room_id: "perf-room".to_owned(),
                seq: index as u64 + 1,
                message_id: format!("perf-message-{index}"),
                sender: owner.clone(),
                plaintext: plaintext.clone(),
                timestamp_unix_seconds: NOW + index as u64,
            })
            .collect::<Vec<_>>();
        let events = messages
            .iter()
            .map(|message| StoredAppEvent {
                room_id: message.room_id.clone(),
                seq: message.seq,
                message_id: message.message_id.clone(),
                sender: message.sender.clone(),
                plaintext: message.plaintext.clone(),
                timestamp_unix_seconds: message.timestamp_unix_seconds,
            })
            .collect::<Vec<_>>();
        core.store
            .save_app_messages_and_events(&owner, &messages, &events, MAX_APP_MESSAGES_U32)
            .unwrap();
        let mut state = AppRuntimeState::new(core).unwrap();

        state.runtime_tick().unwrap();
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            state.runtime_tick().unwrap();
            samples.push(started.elapsed());
        }
        samples.sort();
        let p50 = samples[samples.len() / 2];
        let total = samples.iter().copied().sum::<Duration>();
        println!(
            "idle runtime tick with {HISTORY_ITEMS} messages + {HISTORY_ITEMS} events: \
             n={SAMPLES} p50={p50:?} average={:?} max={:?}",
            total / SAMPLES as u32,
            samples.last().unwrap()
        );
    }

    #[test]
    #[ignore = "timing harness; run explicitly in release mode"]
    fn app_runtime_idle_tick_with_many_rooms() {
        const ROOMS: usize = 20;
        const SAMPLES: usize = 10;

        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let core = CoreState::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("app").to_string_lossy().into_owned(),
            server_url,
            device_id: "perf-many-rooms".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let mut state = AppRuntimeState::new(core).unwrap();
        for index in 0..ROOMS {
            state
                .create_room(format!("Performance room {index}"))
                .unwrap();
        }

        state.runtime_tick().unwrap();
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            state.runtime_tick().unwrap();
            samples.push(started.elapsed());
        }
        samples.sort();
        let p50 = samples[samples.len() / 2];
        let total = samples.iter().copied().sum::<Duration>();
        println!(
            "idle runtime tick with {ROOMS} rooms: n={SAMPLES} p50={p50:?} average={:?} max={:?}",
            total / SAMPLES as u32,
            samples.last().unwrap()
        );
    }

    #[test]
    #[ignore = "timing harness; run explicitly in release mode"]
    fn app_runtime_activity_hint_with_many_rooms() {
        const ROOMS: usize = 20;
        const SAMPLES: usize = 10;

        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let core = CoreState::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("app").to_string_lossy().into_owned(),
            server_url,
            device_id: "perf-targeted-hint".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let mut state = AppRuntimeState::new(core).unwrap();
        for index in 0..ROOMS {
            state
                .create_room(format!("Performance room {index}"))
                .unwrap();
        }
        let room_id = state.app.rooms[ROOMS / 2].room_id.clone();
        let hint = SyncHintEvent::ActivityChanged {
            room_id,
            received_at_ms: NOW * 1_000,
        };

        state.apply_app_sync_hint(&hint).unwrap();
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            state.apply_app_sync_hint(&hint).unwrap();
            samples.push(started.elapsed());
        }
        samples.sort();
        let p50 = samples[samples.len() / 2];
        let total = samples.iter().copied().sum::<Duration>();
        println!(
            "activity hint with {ROOMS} rooms: n={SAMPLES} p50={p50:?} average={:?} max={:?}",
            total / SAMPLES as u32,
            samples.last().unwrap()
        );
    }

    #[test]
    fn server_contract_check_rejects_old_health_without_contract_version() {
        let health = HealthResponse {
            status: "ok".to_owned(),
            server_contract_version: None,
            server_version: Some("0.1.0".to_owned()),
            source_fingerprint: None,
            source_commit: Some("00aa753093c9".to_owned()),
            source_branch: Some("HEAD".to_owned()),
            source_dirty: Some(false),
        };

        let error = validate_server_contract_health("https://chat.finite.computer", &health)
            .expect_err("old server health is rejected");

        assert!(matches!(error, FiniteChatCoreError::ServerRejected { .. }));
        assert!(error.to_string().contains("server_contract_version"));
        assert!(error.to_string().contains("00aa753093c9"));
    }

    #[test]
    fn server_contract_check_accepts_current_contract_version() {
        let health = HealthResponse {
            status: "ok".to_owned(),
            server_contract_version: Some(FINITECHAT_SERVER_CONTRACT_VERSION),
            server_version: Some("0.1.0".to_owned()),
            source_fingerprint: Some("nix-chat-source".to_owned()),
            source_commit: Some("9dd1e11ce6b".to_owned()),
            source_branch: Some("main".to_owned()),
            source_dirty: Some(false),
        };

        validate_server_contract_health("http://127.0.0.1:8787", &health)
            .expect("current server contract is accepted");
    }

    #[test]
    fn server_contract_check_accepts_newer_server_contract_version() {
        let health = HealthResponse {
            status: "ok".to_owned(),
            server_contract_version: Some(FINITECHAT_SERVER_CONTRACT_VERSION + 1),
            server_version: Some("0.1.0".to_owned()),
            source_fingerprint: Some("nix-future-source".to_owned()),
            source_commit: Some("future".to_owned()),
            source_branch: Some("main".to_owned()),
            source_dirty: Some(false),
        };

        validate_server_contract_health("http://127.0.0.1:8787", &health)
            .expect("newer server contract is accepted as long as it includes this contract");
    }

    /// Tests always pass an explicit account secret so they never touch the
    /// shared Finite identity in the developer's real `$HOME`/`$FINITE_HOME`.
    /// The secret is derived deterministically from the data dir so a store
    /// reopened at the same path recovers the same account, while separate
    /// stores (alice/bob) get distinct accounts.
    fn with_test_secret(mut options: OpenOptions) -> OpenOptions {
        if options.account_secret_hex.is_none() {
            options.account_secret_hex = Some(test_account_secret_hex(&options.data_dir));
        }
        options
    }

    /// For tests that must exercise the shared-identity (`None`) acquisition
    /// path itself — e.g. stored-device recovery, which only runs when no
    /// explicit secret is passed. Points FINITE_HOME at a process-wide
    /// throwaway directory, never the developer's real ~/.finite.
    fn ensure_test_finite_home() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let dir = tempfile::tempdir().expect("test FINITE_HOME tempdir");
            let path = dir.path().to_path_buf();
            std::mem::forget(dir);
            // SAFETY: set once; no other unit test in this binary reads
            // FINITE_HOME (they all pass explicit secrets).
            unsafe { std::env::set_var("FINITE_HOME", &path) };
        });
    }

    fn test_account_secret_hex(seed: &str) -> String {
        for counter in 0u32.. {
            let mut hasher = Sha256::new();
            hasher.update(b"finitechat.test.account-secret.v1");
            hasher.update(seed.as_bytes());
            hasher.update(counter.to_le_bytes());
            let bytes: [u8; NOSTR_SECRET_KEY_BYTES] = hasher.finalize().into();
            if NostrSecretKey::from_bytes(bytes).is_ok() {
                return hex::encode(bytes);
            }
        }
        unreachable!("some hash counter yields a valid secp256k1 secret key")
    }

    #[test]
    fn nostr_identity_helpers_round_trip_nsec_material() {
        let created = create_nostr_identity().unwrap();
        assert!(created.npub.starts_with("npub1"));
        assert!(created.nsec.starts_with("nsec1"));
        assert_eq!(created.account_secret_hex.len(), 64);
        assert_eq!(created.account_id.len(), 64);

        let restored = nostr_identity_from_nsec(created.nsec.clone()).unwrap();
        assert_eq!(restored, created);

        let restored_from_hex =
            nostr_identity_from_account_secret_hex(created.account_secret_hex.clone()).unwrap();
        assert_eq!(restored_from_hex, created);
    }

    #[test]
    fn nostr_identity_helpers_accept_nostr_prefixed_nsec() {
        let created = create_nostr_identity().unwrap();
        let restored = nostr_identity_from_nsec(format!("nostr:{}", created.nsec)).unwrap();
        assert_eq!(restored.account_id, created.account_id);
        assert_eq!(restored.account_secret_hex, created.account_secret_hex);
    }

    #[test]
    fn finite_sites_native_viewer_session_proof_matches_endpoint_contract() {
        let identity = create_nostr_identity().unwrap();
        let url = "https://finitechat-native-mockup.sites.test/_finite/auth/native-session";
        let proof = finite_sites_native_viewer_session_proof(
            identity.account_secret_hex.clone(),
            url.to_owned(),
            "/draft?view=full#top".to_owned(),
            "finite-chat-ios".to_owned(),
            "native-nonce-0000001".to_owned(),
            NOW,
        )
        .unwrap();

        assert_eq!(
            proof.body_json,
            r#"{"purpose":"finite_site_view_session","return_to":"/draft?view=full#top","client":"finite-chat-ios","nonce":"native-nonce-0000001"}"#
        );

        let event = decode_nip98_event(&proof.authorization_header);
        assert_eq!(event.pubkey, identity.account_id);
        assert_eq!(event.created_at, NOW);
        assert_eq!(event.kind, FINITE_SITES_NIP98_KIND);
        assert_eq!(event.content, "");
        assert_eq!(tag_value(&event, "u"), Some(url));
        assert_eq!(tag_value(&event, "method"), Some("POST"));
        let expected_payload = hex::encode(Sha256::digest(proof.body_json.as_bytes()));
        assert_eq!(
            tag_value(&event, "payload"),
            Some(expected_payload.as_str())
        );

        let expected_digest =
            nostr_event_id_digest(&event.pubkey, event.created_at, event.kind, &event.tags, "")
                .unwrap();
        assert_eq!(event.id, hex::encode(expected_digest));
        let signature: [u8; 64] = hex::decode(&event.sig).unwrap().try_into().unwrap();
        parse_account_secret_hex(&identity.account_secret_hex)
            .unwrap()
            .public_key()
            .verify_schnorr_digest(expected_digest, &signature)
            .unwrap();
    }

    #[test]
    fn finite_sites_native_viewer_session_proof_rejects_non_endpoint_or_external_redirects() {
        let identity = create_nostr_identity().unwrap();
        let valid_url = "https://site.test/_finite/auth/native-session".to_owned();
        let valid_return_to = "/".to_owned();
        let valid_client = "finite-chat-ios".to_owned();
        let valid_nonce = "native-nonce-0000001".to_owned();

        let wrong_path = finite_sites_native_viewer_session_proof(
            identity.account_secret_hex.clone(),
            "https://site.test/".to_owned(),
            valid_return_to.clone(),
            valid_client.clone(),
            valid_nonce.clone(),
            NOW,
        )
        .unwrap_err();
        assert!(wrong_path.to_string().contains("native session endpoint"));

        let bad_return = finite_sites_native_viewer_session_proof(
            identity.account_secret_hex.clone(),
            valid_url.clone(),
            "https://evil.test/".to_owned(),
            valid_client.clone(),
            valid_nonce.clone(),
            NOW,
        )
        .unwrap_err();
        assert!(bad_return.to_string().contains("return path"));

        let bad_nonce = finite_sites_native_viewer_session_proof(
            identity.account_secret_hex,
            valid_url,
            valid_return_to,
            valid_client,
            "short".to_owned(),
            NOW,
        )
        .unwrap_err();
        assert!(bad_nonce.to_string().contains("nonce"));
    }

    #[test]
    fn app_runtime_listener_receives_initial_and_dispatched_snapshots() {
        struct TestReconciler {
            tx: Mutex<std::sync::mpsc::Sender<AppUpdate>>,
        }

        impl AppReconciler for TestReconciler {
            fn reconcile(&self, update: AppUpdate) {
                let _ = self.tx.lock().unwrap().send(update);
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let runtime = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("app").to_string_lossy().into_owned(),
            server_url: "http://127.0.0.1:1".to_owned(),
            device_id: "listener-device".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();

        runtime.listen_for_updates(Box::new(TestReconciler { tx: Mutex::new(tx) }));

        let AppUpdate::FullState(initial) =
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(initial.rev, 0);

        runtime.dispatch(AppAction::StopRuntime).unwrap();

        let updated = loop {
            let AppUpdate::FullState(state) =
                rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
            if state.rev > initial.rev {
                break state;
            }
        };
        assert!(updated.rev > initial.rev);
        assert_eq!(updated.status, "stopped");
    }

    #[test]
    fn app_runtime_sessions_message_each_other_over_live_http() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-cli".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "App Runtime Flow".to_owned(),
            })
            .unwrap();
        let room_id = alice_state
            .selected_room_id
            .expect("created room is selected");
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");

        alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let bob_joined = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&bob_joined, &room_id).state,
            AppRoomState::Connected
        );

        alice
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "hello from cli".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let bob_sync = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            bob_sync
                .messages
                .iter()
                .any(|message| message.text == "hello from cli")
        );

        bob.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "hello from ios".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        let alice_sync = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            alice_sync
                .messages
                .iter()
                .any(|message| message.text == "hello from ios")
        );
    }

    #[test]
    fn app_runtime_add_uses_current_key_package_when_stale_inventory_exists() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_secret_hex = hex::encode([1_u8; 32]);
        let bob_secret_hex = hex::encode([2_u8; 32]);

        let stale_bob = CoreState::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("stale-bob").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: Some(bob_secret_hex.clone()),
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let stale_upload = stale_bob
            .device
            .upload_key_package_request("000-stale-welcome-decoy")
            .unwrap();
        let mut stale_delivery = delivery_for(&server_url);
        stale_delivery.upload_key_package(stale_upload).unwrap();
        drop(stale_bob);

        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-cli".to_owned(),
            account_secret_hex: Some(alice_secret_hex),
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: Some(bob_secret_hex),
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Stale Welcome Inventory".to_owned(),
            })
            .unwrap();
        let room_id = alice_state
            .selected_room_id
            .expect("created room is selected");
        let bob_account_id = bob.state().unwrap().identity.account_id;
        bob.dispatch_and_wait(AppAction::StartRuntime)
            .expect("bob publishes current key packages");
        alice
            .dispatch_and_wait(AppAction::AddRoomMembers {
                room_id: room_id.clone(),
                profiles: vec![test_profile(&bob_account_id, "Bob")],
            })
            .unwrap();

        alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let bob_joined = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&bob_joined, &room_id).state,
            AppRoomState::Connected,
            "direct MLS add must not select stale KeyPackages left by an old device state"
        );
    }

    #[test]
    fn app_runtime_reopens_app_created_chat() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let created = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "App Created".to_owned(),
            })
            .unwrap();
        let room_id = created
            .selected_room_id
            .as_deref()
            .expect("created room is selected")
            .to_owned();
        app.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "app message before relaunch".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        drop(app);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let local_snapshot = reopened.state().unwrap();
        assert_eq!(
            local_snapshot.selected_room_id.as_deref(),
            Some(room_id.as_str())
        );
        assert_eq!(
            app_room(&local_snapshot, &room_id).display_name,
            "App Created"
        );
        assert_eq!(
            app_room(&local_snapshot, &room_id).last_message_preview,
            "app message before relaunch"
        );
        assert!(
            local_snapshot
                .messages
                .iter()
                .any(|message| message.room_id == room_id
                    && message.text == "app message before relaunch"),
            "app cold launch must project a durable app-created transcript"
        );
    }

    #[test]
    fn app_runtime_models_chats_as_topic_children() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-desktop".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let created = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Topic Room".to_owned(),
            })
            .unwrap();
        let room_id = created.selected_room_id.expect("created room is selected");
        assert_eq!(created.selected_topic_id.as_deref(), Some(HOME_TOPIC_ID));
        let home_chat_id = created
            .selected_chat_id
            .as_deref()
            .expect("Home chat is selected")
            .to_owned();
        assert_eq!(home_chat_id, HOME_CHAT_ID);
        let home = created
            .topics
            .iter()
            .find(|topic| topic.room_id == room_id && topic.topic_id == HOME_TOPIC_ID)
            .expect("Home topic exists");
        assert_eq!(home.title, HOME_TOPIC_TITLE);
        assert_eq!(home.chats.len(), 1);
        assert_eq!(home.chats[0].chat_id, home_chat_id);

        let first_chat = app
            .dispatch_and_wait(AppAction::SendChatMessage {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                chat_id: home_chat_id.clone(),
                text: "first Home chat".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        assert_eq!(first_chat.messages.len(), 1);
        assert_eq!(first_chat.messages[0].text, "first Home chat");
        assert_eq!(
            first_chat.messages[0].chat_id.as_deref(),
            Some(home_chat_id.as_str())
        );

        let new_chat = app
            .dispatch_and_wait(AppAction::StartTopicChat {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                reason: Some("slash_new".to_owned()),
            })
            .unwrap();
        let second_chat_id = new_chat
            .selected_chat_id
            .as_deref()
            .expect("new chat selected")
            .to_owned();
        assert_ne!(second_chat_id, home_chat_id);
        assert!(new_chat.messages.is_empty());
        let empty_title = new_chat
            .topics
            .iter()
            .find(|topic| topic.topic_id == HOME_TOPIC_ID)
            .and_then(|topic| {
                topic
                    .chats
                    .iter()
                    .find(|chat| chat.chat_id == second_chat_id)
            })
            .map(|chat| chat.title.as_str());
        assert_eq!(empty_title, Some("New chat"));

        let second_chat = app
            .dispatch_and_wait(AppAction::SendChatMessage {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                chat_id: second_chat_id.clone(),
                text: "second Home chat".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        assert_eq!(second_chat.messages.len(), 1);
        assert_eq!(second_chat.messages[0].text, "second Home chat");
        assert_eq!(
            second_chat.messages[0].chat_id.as_deref(),
            Some(second_chat_id.as_str())
        );
        let initial_title = second_chat
            .topics
            .iter()
            .find(|topic| topic.topic_id == HOME_TOPIC_ID)
            .and_then(|topic| {
                topic
                    .chats
                    .iter()
                    .find(|chat| chat.chat_id == second_chat_id)
            })
            .map(|chat| chat.title.as_str());
        assert_eq!(initial_title, Some("second Home chat"));

        let later_second_chat = app
            .dispatch_and_wait(AppAction::SendChatMessage {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                chat_id: second_chat_id.clone(),
                text: "a later response must not rename this chat".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let stable_title = later_second_chat
            .topics
            .iter()
            .find(|topic| topic.topic_id == HOME_TOPIC_ID)
            .and_then(|topic| {
                topic
                    .chats
                    .iter()
                    .find(|chat| chat.chat_id == second_chat_id)
            })
            .map(|chat| chat.title.as_str());
        assert_eq!(stable_title, Some("second Home chat"));

        let reopened_first = app
            .dispatch_and_wait(AppAction::OpenChat {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                chat_id: home_chat_id.clone(),
            })
            .unwrap();
        assert_eq!(
            reopened_first.selected_chat_id.as_deref(),
            Some(home_chat_id.as_str())
        );
        assert_eq!(reopened_first.messages.len(), 1);
        assert_eq!(reopened_first.messages[0].text, "first Home chat");
        let home_parent_id = reopened_first.messages[0].message_id.clone();

        let home_reply = app
            .dispatch_and_wait(AppAction::SendChatReply {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                chat_id: home_chat_id.clone(),
                text: "reply in Home chat".to_owned(),
                reply_to_message_id: home_parent_id.clone(),
                metadata_json: None,
            })
            .unwrap();
        let reply = home_reply
            .messages
            .iter()
            .find(|message| message.text == "reply in Home chat")
            .expect("scoped reply projects in the selected chat");
        assert_eq!(reply.conversation_id.as_deref(), Some(HOME_TOPIC_ID));
        assert_eq!(reply.chat_id.as_deref(), Some(home_chat_id.as_str()));
        assert_eq!(
            reply.reply_to_message_id.as_deref(),
            Some(home_parent_id.as_str())
        );
        let DecodedAppEvent::ChatMessage(decoded) = decode_application_event(&reply.payload) else {
            panic!("scoped reply row must carry a chat message application event");
        };
        assert_eq!(decoded.conversation_id.as_deref(), Some(HOME_TOPIC_ID));
        assert_eq!(decoded.segment_id.as_deref(), Some(home_chat_id.as_str()));
        let ChatMessagePayload::Hermes(hermes) = decoded.payload else {
            panic!("scoped reply row must carry Hermes message payload");
        };
        assert_eq!(hermes.conversation_id.as_deref(), Some(HOME_TOPIC_ID));
        assert_eq!(hermes.segment_id.as_deref(), Some(home_chat_id.as_str()));
        assert_eq!(
            hermes.reply_to_message_id.as_deref(),
            Some(home_parent_id.as_str())
        );

        let topic = app
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "Build".to_owned(),
            })
            .unwrap();
        let build_topic_id = topic
            .selected_topic_id
            .as_deref()
            .expect("new topic selected")
            .to_owned();
        assert_ne!(build_topic_id, HOME_TOPIC_ID);
        let build_chat_id = topic
            .selected_chat_id
            .as_deref()
            .expect("new topic creates a first chat")
            .to_owned();
        let build_topic = topic
            .topics
            .iter()
            .find(|topic| topic.topic_id == build_topic_id)
            .expect("new topic appears in app state");
        assert_eq!(build_topic.title, "Build");
        assert_eq!(build_topic.chats.len(), 1);
        assert_eq!(build_topic.chats[0].chat_id, build_chat_id);

        let build_media = app
            .dispatch_and_wait(AppAction::SendChatAttachment {
                room_id: room_id.clone(),
                topic_id: build_topic_id.clone(),
                chat_id: build_chat_id.clone(),
                filename: "scope-proof.jpg".to_owned(),
                mime_type: "image/jpeg".to_owned(),
                kind: ChatMediaKind::Image,
                bytes: b"scoped media bytes".to_vec(),
                caption: "scoped media".to_owned(),
                reply_to_message_id: None,
            })
            .unwrap();
        assert_eq!(
            build_media.selected_topic_id.as_deref(),
            Some(build_topic_id.as_str())
        );
        assert_eq!(
            build_media.selected_chat_id.as_deref(),
            Some(build_chat_id.as_str())
        );
        let media_message = build_media
            .messages
            .iter()
            .find(|message| message.text == "scoped media")
            .expect("scoped attachment projects in the selected chat");
        assert_eq!(
            media_message.conversation_id.as_deref(),
            Some(build_topic_id.as_str())
        );
        assert_eq!(
            media_message.chat_id.as_deref(),
            Some(build_chat_id.as_str())
        );
        assert_eq!(media_message.media.len(), 1);
        let DecodedAppEvent::ChatMessage(decoded) =
            decode_application_event(&media_message.payload)
        else {
            panic!("scoped attachment row must carry a chat message application event");
        };
        assert_eq!(
            decoded.conversation_id.as_deref(),
            Some(build_topic_id.as_str())
        );
        assert_eq!(decoded.segment_id.as_deref(), Some(build_chat_id.as_str()));
        let ChatMessagePayload::Hermes(hermes) = decoded.payload else {
            panic!("scoped attachment row must carry Hermes message payload");
        };
        assert_eq!(
            hermes.conversation_id.as_deref(),
            Some(build_topic_id.as_str())
        );
        assert_eq!(hermes.segment_id.as_deref(), Some(build_chat_id.as_str()));

        let build_poll = app
            .dispatch_and_wait(AppAction::SendChatPoll {
                room_id: room_id.clone(),
                topic_id: build_topic_id.clone(),
                chat_id: build_chat_id.clone(),
                question: "Ship scoped rich messages?".to_owned(),
                options: vec!["Yes".to_owned(), "Also yes".to_owned()],
            })
            .unwrap();
        let poll_message = build_poll
            .messages
            .iter()
            .find(|message| message.poll.is_some())
            .expect("scoped poll projects in the selected chat");
        assert_eq!(
            poll_message.conversation_id.as_deref(),
            Some(build_topic_id.as_str())
        );
        assert_eq!(
            poll_message.chat_id.as_deref(),
            Some(build_chat_id.as_str())
        );
        let DecodedAppEvent::ChatMessage(decoded) = decode_application_event(&poll_message.payload)
        else {
            panic!("scoped poll row must carry a chat message application event");
        };
        assert_eq!(
            decoded.conversation_id.as_deref(),
            Some(build_topic_id.as_str())
        );
        assert_eq!(decoded.segment_id.as_deref(), Some(build_chat_id.as_str()));
        let ChatMessagePayload::Poll(poll) = decoded.payload else {
            panic!("scoped poll payload decodes")
        };
        assert_eq!(poll.question, "Ship scoped rich messages?");

        let reopened_home = app
            .dispatch_and_wait(AppAction::OpenChat {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                chat_id: home_chat_id.clone(),
            })
            .unwrap();
        assert!(
            reopened_home
                .messages
                .iter()
                .all(|message| message.chat_id.as_deref() == Some(home_chat_id.as_str())),
            "messages from the Build chat must not leak into the Home chat transcript"
        );
    }

    #[test]
    fn topic_and_named_chat_mutations_reject_invalid_inputs_without_projection_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let before = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Mutation Guard Room".to_owned(),
            })
            .unwrap();
        let room_id = before.selected_room_id.clone().unwrap();
        let home_chat_id = before.selected_chat_id.clone().unwrap();

        let empty_topic = app
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "   ".to_owned(),
            })
            .expect_err("empty topic titles must fail before generating durable state");
        assert!(
            empty_topic
                .to_string()
                .contains("topic title must not be empty")
        );

        app.dispatch_and_wait(AppAction::StartTopicChat {
            room_id: room_id.clone(),
            topic_id: "missing-topic".to_owned(),
            reason: None,
        })
        .expect_err("a chat cannot be started under a missing topic");
        app.dispatch_and_wait(AppAction::RenameChat {
            room_id: room_id.clone(),
            topic_id: HOME_TOPIC_ID.to_owned(),
            chat_id: "missing-chat".to_owned(),
            title: "Impossible rename".to_owned(),
        })
        .expect_err("a missing chat cannot acquire projection metadata");
        app.dispatch_and_wait(AppAction::SetChatArchived {
            room_id: room_id.clone(),
            topic_id: HOME_TOPIC_ID.to_owned(),
            chat_id: "missing-chat".to_owned(),
            archived: true,
        })
        .expect_err("a missing chat cannot acquire archive metadata");
        app.dispatch_and_wait(AppAction::RenameChat {
            room_id: room_id.clone(),
            topic_id: HOME_TOPIC_ID.to_owned(),
            chat_id: home_chat_id,
            title: "   ".to_owned(),
        })
        .expect_err("empty chat titles are rejected");

        let after = app.state().unwrap();
        assert_eq!(after.topics, before.topics);
        assert_eq!(after.selected_room_id, before.selected_room_id);
        assert_eq!(after.selected_topic_id, before.selected_topic_id);
        assert_eq!(after.selected_chat_id, before.selected_chat_id);
    }

    #[test]
    fn app_runtime_chat_rename_replays_and_syncs_to_another_device() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-electron".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let created = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Rename Room".to_owned(),
            })
            .unwrap();
        let room_id = created.selected_room_id.unwrap();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");
        bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();

        let topic_state = alice
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "Build".to_owned(),
            })
            .unwrap();
        let topic_id = topic_state.selected_topic_id.unwrap();
        let chat_id = topic_state.selected_chat_id.unwrap();
        let titled_from_first_message = alice
            .dispatch_and_wait(AppAction::SendChatMessage {
                room_id: room_id.clone(),
                topic_id: topic_id.clone(),
                chat_id: chat_id.clone(),
                text: "Implement the chats sidebar".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let fallback_title = titled_from_first_message
            .topics
            .iter()
            .find(|topic| topic.topic_id == topic_id)
            .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
            .map(|chat| chat.title.as_str());
        assert_eq!(fallback_title, Some("Implement the chats sidebar"));

        let elsewhere = alice
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "Elsewhere".to_owned(),
            })
            .unwrap();
        let elsewhere_topic_id = elsewhere.selected_topic_id.unwrap();
        let elsewhere_chat_id = elsewhere.selected_chat_id.unwrap();

        let renamed = alice
            .dispatch_and_wait(AppAction::RenameChat {
                room_id: room_id.clone(),
                topic_id: topic_id.clone(),
                chat_id: chat_id.clone(),
                title: "  SaaS chat polish  ".to_owned(),
            })
            .unwrap();
        let explicit_title = renamed
            .topics
            .iter()
            .find(|topic| topic.topic_id == topic_id)
            .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
            .map(|chat| chat.title.as_str());
        assert_eq!(explicit_title, Some("SaaS chat polish"));
        assert_eq!(
            renamed.selected_topic_id.as_deref(),
            Some(elsewhere_topic_id.as_str()),
            "renaming a background chat must not change the active topic"
        );
        assert_eq!(
            renamed.selected_chat_id.as_deref(),
            Some(elsewhere_chat_id.as_str()),
            "renaming a background chat must not change the active chat"
        );

        let bob_synced = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let bob_title = bob_synced
            .topics
            .iter()
            .find(|topic| topic.topic_id == topic_id)
            .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
            .map(|chat| chat.title.as_str());
        assert_eq!(
            bob_title,
            Some("SaaS chat polish"),
            "another Device must project the encrypted non-notifying rename"
        );

        assert!(
            alice
                .dispatch_and_wait(AppAction::RenameChat {
                    room_id: room_id.clone(),
                    topic_id: topic_id.clone(),
                    chat_id: chat_id.clone(),
                    title: "   ".to_owned(),
                })
                .unwrap_err()
                .to_string()
                .contains("must not be empty")
        );

        drop(alice);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        let replayed_title = reopened_state
            .topics
            .iter()
            .find(|topic| topic.topic_id == topic_id)
            .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
            .map(|chat| chat.title.as_str());
        assert_eq!(
            replayed_title,
            Some("SaaS chat polish"),
            "cold replay must not depend on Hermes or a live server"
        );
    }

    #[test]
    fn cached_archive_state_is_display_fallback_not_ordering_authority() {
        let mut projection = ChatProjectionState::default();
        projection.restore_chat_archives(&[StoredChatArchiveState {
            room_id: "room-main".to_owned(),
            topic_id: "topic-main".to_owned(),
            chat_id: "chat-main".to_owned(),
            accepted_seq: 50,
            archived: true,
        }]);
        assert!(
            projection.stored_chat_archives()[0].archived,
            "cache fallback may restore local display when canonical history was pruned"
        );
        assert!(projection.apply_chat_archive(
            "room-main",
            40,
            ChatArchiveV1 {
                topic_id: "topic-main".to_owned(),
                chat_id: "chat-main".to_owned(),
                archived: false,
            },
            ChatArchiveProjectionSource::CanonicalEvent,
        ));
        let canonical = projection.stored_chat_archives();
        assert_eq!(canonical[0].accepted_seq, 40);
        assert!(!canonical[0].archived);
        projection.restore_chat_archives(&[StoredChatArchiveState {
            room_id: "room-main".to_owned(),
            topic_id: "topic-main".to_owned(),
            chat_id: "chat-main".to_owned(),
            accepted_seq: 60,
            archived: true,
        }]);
        let after_confused_cache = projection.stored_chat_archives();
        assert_eq!(after_confused_cache[0].accepted_seq, 40);
        assert!(!after_confused_cache[0].archived);
    }

    #[test]
    fn named_chat_rename_and_archive_converge_across_every_observation_order() {
        #[derive(Clone, Copy)]
        enum Operation {
            Rename {
                accepted_seq: u64,
                title: &'static str,
            },
            Archive {
                accepted_seq: u64,
                archived: bool,
            },
        }

        let operations = [
            Operation::Rename {
                accepted_seq: 10,
                title: "Initial name",
            },
            Operation::Archive {
                accepted_seq: 15,
                archived: true,
            },
            Operation::Rename {
                accepted_seq: 20,
                title: "Canonical name",
            },
            Operation::Archive {
                accepted_seq: 25,
                archived: false,
            },
        ];
        let mut order = [0, 1, 2, 3];
        let mut checked = 0_u32;

        loop {
            let mut projection = ChatProjectionState::default();
            for index in order {
                let operation = operations[index];
                for _ in 0..2 {
                    match operation {
                        Operation::Rename {
                            accepted_seq,
                            title,
                        } => projection.apply_chat_rename(
                            "room-main",
                            accepted_seq,
                            ChatRenameV1 {
                                topic_id: "topic-main".to_owned(),
                                chat_id: "chat-main".to_owned(),
                                title: title.to_owned(),
                            },
                        ),
                        Operation::Archive {
                            accepted_seq,
                            archived,
                        } => {
                            let _ = projection.apply_chat_archive(
                                "room-main",
                                accepted_seq,
                                ChatArchiveV1 {
                                    topic_id: "topic-main".to_owned(),
                                    chat_id: "chat-main".to_owned(),
                                    archived,
                                },
                                ChatArchiveProjectionSource::CanonicalEvent,
                            );
                        }
                    }
                }
            }

            let title = projection
                .chat_titles
                .get(&(
                    "room-main".to_owned(),
                    "topic-main".to_owned(),
                    "chat-main".to_owned(),
                ))
                .expect("latest title is retained");
            assert_eq!(title.accepted_seq, 20, "observation order {order:?}");
            assert_eq!(title.title, "Canonical name", "observation order {order:?}");
            let archive = projection
                .chat_archives
                .get(&(
                    "room-main".to_owned(),
                    "topic-main".to_owned(),
                    "chat-main".to_owned(),
                ))
                .expect("latest archive state is retained");
            assert_eq!(archive.accepted_seq, 25, "observation order {order:?}");
            assert!(!archive.archived, "observation order {order:?}");
            assert_eq!(
                archive.source,
                ChatArchiveProjectionSource::CanonicalEvent,
                "observation order {order:?}"
            );
            checked += 1;
            if !next_permutation(&mut order) {
                break;
            }
        }

        assert_eq!(checked, 24);
    }

    #[test]
    fn app_runtime_chat_archive_is_durable_shared_organizational_state() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-electron".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let created = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Archive Room".to_owned(),
            })
            .unwrap();
        let room_id = created.selected_room_id.unwrap();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");
        bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();

        let topic_state = alice
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "Build".to_owned(),
            })
            .unwrap();
        let topic_id = topic_state.selected_topic_id.unwrap();
        let chat_id = topic_state.selected_chat_id.unwrap();

        let archived = alice
            .dispatch_and_wait(AppAction::SetChatArchived {
                room_id: room_id.clone(),
                topic_id: topic_id.clone(),
                chat_id: chat_id.clone(),
                archived: true,
            })
            .unwrap();
        assert_eq!(
            archived.selected_topic_id.as_deref(),
            Some(topic_id.as_str())
        );
        assert_eq!(archived.selected_chat_id.as_deref(), Some(chat_id.as_str()));
        assert!(
            archived
                .topics
                .iter()
                .find(|topic| topic.topic_id == topic_id)
                .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
                .is_some_and(|chat| chat.archived),
            "archiving the current chat must preserve selection and expose shared metadata"
        );

        let elsewhere = alice
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "Elsewhere".to_owned(),
            })
            .unwrap();
        let elsewhere_topic_id = elsewhere.selected_topic_id.unwrap();
        let elsewhere_chat_id = elsewhere.selected_chat_id.unwrap();
        let background_archived = alice
            .dispatch_and_wait(AppAction::SetChatArchived {
                room_id: room_id.clone(),
                topic_id: topic_id.clone(),
                chat_id: chat_id.clone(),
                archived: true,
            })
            .unwrap();
        assert_eq!(
            background_archived.selected_topic_id.as_deref(),
            Some(elsewhere_topic_id.as_str()),
            "archiving a background chat must not change the active topic"
        );
        assert_eq!(
            background_archived.selected_chat_id.as_deref(),
            Some(elsewhere_chat_id.as_str()),
            "archiving a background chat must not change the active chat"
        );

        let bob_synced = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            bob_synced
                .topics
                .iter()
                .find(|topic| topic.topic_id == topic_id)
                .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
                .is_some_and(|chat| chat.archived),
            "another Device must project the encrypted non-notifying archive event"
        );

        let after_message = alice
            .dispatch_and_wait(AppAction::SendChatMessage {
                room_id: room_id.clone(),
                topic_id: topic_id.clone(),
                chat_id: chat_id.clone(),
                text: "Archived chats remain usable.".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        assert!(
            after_message
                .topics
                .iter()
                .find(|topic| topic.topic_id == topic_id)
                .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
                .is_some_and(|chat| chat.archived),
            "new messages must not implicitly restore an archived chat"
        );
        assert_eq!(
            after_message.selected_topic_id.as_deref(),
            Some(elsewhere_topic_id.as_str()),
            "sending to a background chat must not change the active topic"
        );
        assert_eq!(
            after_message.selected_chat_id.as_deref(),
            Some(elsewhere_chat_id.as_str()),
            "sending to a background chat must not change the active chat"
        );

        drop(alice);
        let owner = DeviceRef {
            account_id: after_message.identity.account_id,
            device_id: "alice-hosted-web".to_owned(),
        };
        let account_secret =
            parse_account_secret_hex(&test_account_secret_hex(&alice_dir.to_string_lossy()))
                .unwrap();
        let store_options =
            SqliteClientStoreOptions::from_nostr_secret(&account_secret, &owner.device_id).unwrap();
        let mut app_store =
            SqliteClientStore::open(alice_dir.join(CLIENT_STORE_FILE), store_options).unwrap();
        let mut app_state = app_store.load_app_state(&owner).unwrap();
        app_state.chat_archives.clear();
        app_store.save_app_state(&owner, &app_state).unwrap();
        drop(app_store);

        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        assert!(
            alice
                .state()
                .unwrap()
                .topics
                .iter()
                .find(|topic| topic.topic_id == topic_id)
                .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
                .is_some_and(|chat| chat.archived),
            "startup must reconstruct archive state after a sync/apply crash boundary"
        );
        drop(alice);

        let app_db = rusqlite::Connection::open(alice_dir.join(CLIENT_STORE_FILE)).unwrap();
        app_db.execute("DELETE FROM client_app_events", []).unwrap();
        drop(app_db);
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        assert!(
            alice
                .state()
                .unwrap()
                .topics
                .iter()
                .find(|topic| topic.topic_id == topic_id)
                .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
                .is_some_and(|chat| chat.archived),
            "the encrypted current-value projection must survive event-cache pruning"
        );

        let restored = bob
            .dispatch_and_wait(AppAction::SetChatArchived {
                room_id: room_id.clone(),
                topic_id: topic_id.clone(),
                chat_id: chat_id.clone(),
                archived: false,
            })
            .unwrap();
        assert!(
            restored
                .topics
                .iter()
                .find(|topic| topic.topic_id == topic_id)
                .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
                .is_some_and(|chat| !chat.archived)
        );

        let alice_synced = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            alice_synced
                .topics
                .iter()
                .find(|topic| topic.topic_id == topic_id)
                .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
                .is_some_and(|chat| !chat.archived),
            "later accepted archive state must win on every Device"
        );

        drop(alice);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        assert!(
            reopened
                .state()
                .unwrap()
                .topics
                .iter()
                .find(|topic| topic.topic_id == topic_id)
                .and_then(|topic| topic.chats.iter().find(|chat| chat.chat_id == chat_id))
                .is_some_and(|chat| !chat.archived),
            "cold replay must recover the latest archive state without a live server"
        );
    }

    #[test]
    fn open_room_on_the_selected_room_preserves_topic_and_chat_selection() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let created = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Selection room".to_owned(),
            })
            .unwrap();
        let room_id = created.selected_room_id.unwrap();
        let topic_state = alice
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "Build".to_owned(),
            })
            .unwrap();
        let topic_id = topic_state.selected_topic_id.unwrap();
        let first_chat_id = topic_state.selected_chat_id.unwrap();
        let second_chat_id = alice
            .dispatch_and_wait(AppAction::StartTopicChat {
                room_id: room_id.clone(),
                topic_id: topic_id.clone(),
                reason: Some("second chat".to_owned()),
            })
            .unwrap()
            .selected_chat_id
            .unwrap();
        assert_ne!(first_chat_id, second_chat_id);
        // The topic's default chat is its active (most recently started) chat,
        // so the first chat is a non-default selection that a reset would lose.
        alice
            .dispatch_and_wait(AppAction::OpenChat {
                room_id: room_id.clone(),
                topic_id: topic_id.clone(),
                chat_id: first_chat_id.clone(),
            })
            .unwrap();

        let reopened = alice
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: room_id.clone(),
            })
            .unwrap();
        assert_eq!(reopened.selected_room_id.as_deref(), Some(room_id.as_str()));
        assert_eq!(
            reopened.selected_topic_id.as_deref(),
            Some(topic_id.as_str()),
            "re-opening the already-selected room must not reset the topic"
        );
        assert_eq!(
            reopened.selected_chat_id.as_deref(),
            Some(first_chat_id.as_str()),
            "re-opening the already-selected room must not reset the chat"
        );
    }

    #[test]
    fn open_room_on_a_different_room_still_resets_to_that_rooms_default_chat() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let room_one = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "First room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        let room_two = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Second room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        assert_ne!(room_one, room_two);

        let reopened = alice
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: room_one.clone(),
            })
            .unwrap();
        assert_eq!(
            reopened.selected_room_id.as_deref(),
            Some(room_one.as_str())
        );
        assert_eq!(
            reopened.selected_topic_id.as_deref(),
            Some(HOME_TOPIC_ID),
            "switching rooms must still reset to the room's Home topic"
        );
        assert_eq!(
            reopened.selected_chat_id.as_deref(),
            Some(HOME_CHAT_ID),
            "switching rooms must still reset to the Home topic's default chat"
        );
    }

    #[test]
    fn open_room_default_chat_skips_an_archived_active_chat() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let room_id = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Archived default room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        let archived_chat_id = alice
            .dispatch_and_wait(AppAction::StartTopicChat {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                reason: Some("soon to be archived".to_owned()),
            })
            .unwrap()
            .selected_chat_id
            .unwrap();
        alice
            .dispatch_and_wait(AppAction::SetChatArchived {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                chat_id: archived_chat_id.clone(),
                archived: true,
            })
            .unwrap();
        alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Elsewhere".to_owned(),
            })
            .unwrap();

        let reopened = alice
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: room_id.clone(),
            })
            .unwrap();
        assert_eq!(reopened.selected_topic_id.as_deref(), Some(HOME_TOPIC_ID));
        assert_eq!(
            reopened.selected_chat_id.as_deref(),
            Some(HOME_CHAT_ID),
            "the default chat pick must skip the archived active chat when a \
             non-archived chat exists"
        );
    }

    #[test]
    fn open_room_default_chat_keeps_an_archived_chat_when_every_chat_is_archived() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let room_id = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "All archived room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        alice
            .dispatch_and_wait(AppAction::SetChatArchived {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                chat_id: HOME_CHAT_ID.to_owned(),
                archived: true,
            })
            .unwrap();
        alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Elsewhere".to_owned(),
            })
            .unwrap();

        let reopened = alice
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: room_id.clone(),
            })
            .unwrap();
        assert_eq!(reopened.selected_topic_id.as_deref(), Some(HOME_TOPIC_ID));
        assert_eq!(
            reopened.selected_chat_id.as_deref(),
            Some(HOME_CHAT_ID),
            "an all-archived topic keeps its archived default chat"
        );
    }

    #[test]
    #[ignore = "slow full-history stress test; run `just chat-history-stress`"]
    fn late_same_account_device_converges_topics_named_chats_and_archives_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let hosted = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("hosted").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let agent = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("agent").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let created = hosted
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Convergence Room".to_owned(),
            })
            .unwrap();
        let room_id = created.selected_room_id.unwrap();
        add_runtime_member_named(&hosted, &agent, &room_id, "Agent");

        let build = hosted
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "Build".to_owned(),
            })
            .unwrap();
        let build_topic_id = build.selected_topic_id.unwrap();
        let first_build_chat_id = build.selected_chat_id.unwrap();
        hosted
            .dispatch_and_wait(AppAction::SendChatMessage {
                room_id: room_id.clone(),
                topic_id: build_topic_id.clone(),
                chat_id: first_build_chat_id.clone(),
                text: "First build context".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        hosted
            .dispatch_and_wait(AppAction::RenameChat {
                room_id: room_id.clone(),
                topic_id: build_topic_id.clone(),
                chat_id: first_build_chat_id.clone(),
                title: "Pinned build chat".to_owned(),
            })
            .unwrap();
        hosted
            .dispatch_and_wait(AppAction::SetChatArchived {
                room_id: room_id.clone(),
                topic_id: build_topic_id.clone(),
                chat_id: first_build_chat_id.clone(),
                archived: true,
            })
            .unwrap();

        let second_build = hosted
            .dispatch_and_wait(AppAction::StartTopicChat {
                room_id: room_id.clone(),
                topic_id: build_topic_id.clone(),
                reason: Some("new_context".to_owned()),
            })
            .unwrap();
        let second_build_chat_id = second_build.selected_chat_id.unwrap();
        hosted
            .dispatch_and_wait(AppAction::SendChatMessage {
                room_id: room_id.clone(),
                topic_id: build_topic_id.clone(),
                chat_id: second_build_chat_id.clone(),
                text: "Second build context".to_owned(),
                metadata_json: None,
            })
            .unwrap();

        let operations = hosted
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "Operations".to_owned(),
            })
            .unwrap();
        let operations_topic_id = operations.selected_topic_id.unwrap();
        let operations_chat_id = operations.selected_chat_id.unwrap();
        hosted
            .dispatch_and_wait(AppAction::RenameChat {
                room_id: room_id.clone(),
                topic_id: operations_topic_id.clone(),
                chat_id: operations_chat_id.clone(),
                title: "Production operations".to_owned(),
            })
            .unwrap();
        const TOTAL_HISTORY_STRESS_MESSAGES: u32 = 5_226;
        for index in 0..TOTAL_HISTORY_STRESS_MESSAGES {
            hosted
                .dispatch_and_wait(AppAction::SendChatMessage {
                    room_id: room_id.clone(),
                    topic_id: operations_topic_id.clone(),
                    chat_id: operations_chat_id.clone(),
                    text: format!("Busy operations history {index}"),
                    metadata_json: None,
                })
                .unwrap();
        }

        let hosted_identity = hosted.state().unwrap().identity;
        let account_secret_hex = hosted_identity.account_secret_hex;
        let linked_dir = dir.path().join("linked-ios");
        let linked = FiniteChatRuntime::open(OpenOptions {
            data_dir: linked_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "linked-ios".to_owned(),
            account_secret_hex: Some(account_secret_hex.clone()),
            now_unix_seconds: Some(NOW),
        })
        .unwrap();
        linked
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("late Device publishes KeyPackages");
        let (_, linked_state) = drive_device_link_to_exact_receipts(
            &hosted,
            &linked,
            "topic-hardening-link",
            "linked-ios",
        );
        let linked_db = rusqlite::Connection::open(linked_dir.join(CLIENT_STORE_FILE)).unwrap();
        let linked_event_count = linked_db
            .query_row("SELECT COUNT(*) FROM client_app_events", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap();
        assert!(
            linked_event_count > u64::from(TOTAL_HISTORY_STRESS_MESSAGES),
            "the linked Device must durably import all 5,226 messages across multiple chunks"
        );
        drop(linked_db);

        let linked_build = linked_state
            .topics
            .iter()
            .find(|topic| topic.topic_id == build_topic_id)
            .expect("late Device receives the Build topic");
        assert_eq!(linked_build.title, "Build");
        assert_eq!(linked_build.chats.len(), 2);
        let linked_first_build = linked_build
            .chats
            .iter()
            .find(|chat| chat.chat_id == first_build_chat_id)
            .expect("late Device receives the first Build chat");
        assert_eq!(linked_first_build.title, "Pinned build chat");
        assert!(linked_first_build.archived);
        assert!(
            linked_build
                .chats
                .iter()
                .any(|chat| chat.chat_id == second_build_chat_id),
            "late Device receives the newer Build chat boundary"
        );
        let linked_operations = linked_state
            .topics
            .iter()
            .find(|topic| topic.topic_id == operations_topic_id)
            .expect("late Device receives the Operations topic");
        assert_eq!(linked_operations.title, "Operations");
        assert!(linked_operations.chats.iter().any(
            |chat| chat.chat_id == operations_chat_id && chat.title == "Production operations"
        ));

        linked
            .dispatch_and_wait(AppAction::SetChatArchived {
                room_id: room_id.clone(),
                topic_id: build_topic_id.clone(),
                chat_id: first_build_chat_id.clone(),
                archived: false,
            })
            .unwrap();
        hosted
            .dispatch_and_wait(AppAction::RenameChat {
                room_id: room_id.clone(),
                topic_id: build_topic_id.clone(),
                chat_id: first_build_chat_id.clone(),
                title: "Final build chat".to_owned(),
            })
            .unwrap();

        let hosted_converged = hosted.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let linked_converged = linked.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        // One runtime tick is intentionally bounded. Prove repeated ordinary
        // ticks reach the source tail instead of silently treating a partial
        // page set as convergence.
        let agent_converged = (0..100)
            .find_map(|_| {
                let state = agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
                state
                    .topics
                    .iter()
                    .find(|topic| topic.topic_id == build_topic_id)
                    .and_then(|topic| {
                        topic
                            .chats
                            .iter()
                            .find(|chat| chat.chat_id == first_build_chat_id)
                    })
                    .is_some_and(|chat| chat.title == "Final build chat" && !chat.archived)
                    .then_some(state)
            })
            .expect("bounded ordinary sync converges after the 5,226-message gap");
        for (device, state) in [
            ("hosted", &hosted_converged),
            ("linked", &linked_converged),
            ("agent", &agent_converged),
        ] {
            let chat = state
                .topics
                .iter()
                .find(|topic| topic.topic_id == build_topic_id)
                .and_then(|topic| {
                    topic
                        .chats
                        .iter()
                        .find(|chat| chat.chat_id == first_build_chat_id)
                })
                .unwrap_or_else(|| panic!("{device} must retain the first Build chat"));
            assert_eq!(chat.title, "Final build chat", "{device} title");
            assert!(!chat.archived, "{device} archive state");
        }

        drop(linked);
        let reopened = FiniteChatRuntime::open(OpenOptions {
            data_dir: linked_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "linked-ios".to_owned(),
            account_secret_hex: Some(account_secret_hex),
            now_unix_seconds: Some(NOW),
        })
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        let reopened_chat = reopened_state
            .topics
            .iter()
            .find(|topic| topic.topic_id == build_topic_id)
            .and_then(|topic| {
                topic
                    .chats
                    .iter()
                    .find(|chat| chat.chat_id == first_build_chat_id)
            })
            .unwrap_or_else(|| {
                panic!(
                    "late Device cold restart retains named chat state; topics={:#?}",
                    reopened_state.topics
                )
            });
        assert_eq!(reopened_chat.title, "Final build chat");
        assert!(!reopened_chat.archived);
        assert!(
            reopened_state
                .topics
                .iter()
                .any(|topic| topic.topic_id == operations_topic_id),
            "late Device cold restart retains sibling topics"
        );
    }

    #[test]
    fn chat_projection_displays_hermes_payload_text() {
        let payload = HermesMessagePayloadV1 {
            payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: None,
            segment_id: None,
            text: "echo: hello from iOS".to_owned(),
            kind: finitechat_hermes::HermesSendKindV1::Message,
            status: finitechat_hermes::HermesMessageStatusV1::Complete,
            edit_of: None,
            attachments: Vec::new(),
            reply_to_message_id: None,
            sender_name: None,
            metadata: BTreeMap::new(),
        }
        .encode()
        .unwrap();

        assert_eq!(chat_display_text(&payload), "echo: hello from iOS");
        let wrapped =
            encode_application_event(DurableAppEventKind::ChatMessage, None, &payload).unwrap();
        assert_eq!(chat_display_text(&wrapped), "echo: hello from iOS");
        assert_eq!(chat_display_text(b"plain hello"), "plain hello");
    }

    #[test]
    fn chat_projection_preserves_hermes_presentation_and_old_state_defaults() {
        let payload = HermesMessagePayloadV1 {
            payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: Some("topic-build".to_owned()),
            segment_id: Some("segment-7".to_owned()),
            text: "Running cargo test".to_owned(),
            kind: HermesSendKindV1::Tool,
            status: HermesMessageStatusV1::Running,
            edit_of: Some("tool-message-1".to_owned()),
            attachments: Vec::new(),
            reply_to_message_id: None,
            sender_name: Some("Hermes".to_owned()),
            metadata: BTreeMap::new(),
        }
        .encode()
        .unwrap();
        let plaintext = encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            Some("topic-build".to_owned()),
            Some("segment-7".to_owned()),
            &payload,
        )
        .unwrap();
        let sender = DeviceRef {
            account_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            device_id: "hermes".to_owned(),
        };
        let owner = DeviceRef {
            account_id: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                .to_owned(),
            device_id: "hosted-web".to_owned(),
        };
        let message = project_chat_message(
            "room-main".to_owned(),
            8,
            "tool-message-2".to_owned(),
            sender,
            plaintext,
            NOW,
            &owner,
        )
        .unwrap();
        assert_eq!(message.kind, ChatMessageKind::Tool);
        assert_eq!(message.status, ChatMessageStatus::Running);
        assert!(!message.final_delivery);
        assert_eq!(
            message.edit_of_message_id.as_deref(),
            Some("tool-message-1")
        );

        let mut legacy_json = serde_json::to_value(&message).unwrap();
        let object = legacy_json.as_object_mut().unwrap();
        object.remove("kind");
        object.remove("status");
        object.remove("final_delivery");
        object.remove("edit_of_message_id");
        let legacy: ChatMessage = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(legacy.kind, ChatMessageKind::Message);
        assert_eq!(legacy.status, ChatMessageStatus::Complete);
        assert!(!legacy.final_delivery);
        assert_eq!(legacy.edit_of_message_id, None);

        let raw = project_chat_message(
            "room-main".to_owned(),
            9,
            "native-message".to_owned(),
            owner.clone(),
            b"native hello".to_vec(),
            NOW,
            &owner,
        )
        .unwrap();
        assert_eq!(raw.kind, ChatMessageKind::Message);
        assert_eq!(raw.status, ChatMessageStatus::Complete);
        assert!(!raw.final_delivery);
        assert_eq!(raw.edit_of_message_id, None);
    }

    #[test]
    fn same_account_other_device_is_mine_without_local_outbound_delivery() {
        let owner = DeviceRef {
            account_id: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                .to_owned(),
            device_id: "hosted-web".to_owned(),
        };
        let electron = DeviceRef {
            account_id: owner.account_id.clone(),
            device_id: "electron-alpha".to_owned(),
        };
        let message = project_chat_message(
            "room-main".to_owned(),
            10,
            "message-electron".to_owned(),
            electron,
            b"sent from Electron".to_vec(),
            NOW,
            &owner,
        )
        .unwrap();

        assert!(message.is_mine);
        assert_eq!(message.sender_display_name, "You");
        assert_eq!(message.sender_device_id, "electron-alpha");
        assert_eq!(message.outbound_delivery, None);

        let current_device = project_chat_message(
            "room-main".to_owned(),
            11,
            "message-hosted".to_owned(),
            owner.clone(),
            b"sent from Hosted Web".to_vec(),
            NOW,
            &owner,
        )
        .unwrap();
        assert!(current_device.is_mine);
        assert_eq!(current_device.outbound_delivery, Some(outbound_delivered()));
    }

    #[test]
    fn chat_projection_maps_notify_to_final_delivery_for_complete_responses() {
        let sender = DeviceRef {
            account_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            device_id: "hermes".to_owned(),
        };
        let owner = DeviceRef {
            account_id: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                .to_owned(),
            device_id: "hosted-web".to_owned(),
        };
        let project = |seq: u64,
                       text: &str,
                       kind: HermesSendKindV1,
                       metadata: BTreeMap<String, serde_json::Value>| {
            let payload = HermesMessagePayloadV1 {
                payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
                conversation_id: Some("topic-build".to_owned()),
                segment_id: Some("segment-7".to_owned()),
                text: text.to_owned(),
                kind,
                status: HermesMessageStatusV1::Complete,
                edit_of: None,
                attachments: Vec::new(),
                reply_to_message_id: None,
                sender_name: Some("Hermes".to_owned()),
                metadata,
            }
            .encode()
            .unwrap();
            project_chat_message(
                "room-main".to_owned(),
                seq,
                format!("message-{seq}"),
                sender.clone(),
                payload,
                NOW,
                &owner,
            )
            .unwrap()
        };

        let final_message = project(
            10,
            "Final answer",
            HermesSendKindV1::Message,
            BTreeMap::from([("notify".to_owned(), serde_json::Value::Bool(true))]),
        );
        let commentary = project(
            11,
            "Still working through it",
            HermesSendKindV1::Message,
            BTreeMap::new(),
        );
        let tool = project(
            12,
            "cargo test complete",
            HermesSendKindV1::Tool,
            BTreeMap::from([("notify".to_owned(), serde_json::Value::Bool(false))]),
        );

        assert!(final_message.final_delivery);
        assert!(!commentary.final_delivery);
        assert!(!tool.final_delivery);
    }

    #[test]
    fn chat_projection_ignores_reaction_app_events_as_messages() {
        let reaction = ChatReactionV1 {
            target_message_id: "message-1".to_owned(),
            emoji: "+1".to_owned(),
        };
        let payload = serde_json::to_vec(&reaction).unwrap();
        let event =
            encode_application_event(DurableAppEventKind::ChatReaction, None, &payload).unwrap();
        let sender = DeviceRef {
            account_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            device_id: "phone".to_owned(),
        };
        let owner = sender.clone();

        assert_eq!(chat_display_text(&event), "");
        assert!(
            project_chat_message(
                "room-main".to_owned(),
                8,
                "reaction-1".to_owned(),
                sender,
                event,
                NOW,
                &owner,
            )
            .is_none(),
            "typed reaction events must not become transcript rows"
        );
    }

    #[test]
    fn chat_projection_rebuilds_from_stored_app_events_without_message_cache() {
        let owner = DeviceRef {
            account_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            device_id: "phone".to_owned(),
        };
        let peer = DeviceRef {
            account_id: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                .to_owned(),
            device_id: "tablet".to_owned(),
        };
        let chat_payload = encode_text_message_payload("event sourced hello", None).unwrap();
        let chat_event =
            encode_application_event(DurableAppEventKind::ChatMessage, None, &chat_payload)
                .unwrap();
        let reaction = ChatReactionV1 {
            target_message_id: "message-1".to_owned(),
            emoji: "+1".to_owned(),
        };
        let reaction_payload = serde_json::to_vec(&reaction).unwrap();
        let reaction_event =
            encode_application_event(DurableAppEventKind::ChatReaction, None, &reaction_payload)
                .unwrap();
        let receipt = ChatReceiptV1 {
            target_message_id: "message-1".to_owned(),
            target_seq: 1,
            state: ChatReceiptStateV1::Read,
        };
        let receipt_payload = serde_json::to_vec(&receipt).unwrap();
        let receipt_event =
            encode_application_event(DurableAppEventKind::ChatReceipt, None, &receipt_payload)
                .unwrap();

        let messages = chat_messages_from_stored(
            Vec::new(),
            vec![
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 1,
                    message_id: "message-1".to_owned(),
                    sender: owner.clone(),
                    plaintext: chat_event,
                    timestamp_unix_seconds: NOW,
                },
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 2,
                    message_id: "reaction-1".to_owned(),
                    sender: peer.clone(),
                    plaintext: reaction_event,
                    timestamp_unix_seconds: NOW + 1,
                },
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 3,
                    message_id: "reaction-duplicate".to_owned(),
                    sender: peer.clone(),
                    plaintext: encode_application_event(
                        DurableAppEventKind::ChatReaction,
                        None,
                        &reaction_payload,
                    )
                    .unwrap(),
                    timestamp_unix_seconds: NOW + 2,
                },
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 4,
                    message_id: "reaction-owner".to_owned(),
                    sender: owner.clone(),
                    plaintext: encode_application_event(
                        DurableAppEventKind::ChatReaction,
                        None,
                        &reaction_payload,
                    )
                    .unwrap(),
                    timestamp_unix_seconds: NOW + 3,
                },
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 5,
                    message_id: "receipt-1".to_owned(),
                    sender: peer.clone(),
                    plaintext: receipt_event,
                    timestamp_unix_seconds: NOW + 4,
                },
            ],
            &owner,
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "event sourced hello");
        assert_eq!(
            messages[0].reactions,
            vec![ChatReactionSummary {
                emoji: "+1".to_owned(),
                count: 2,
                reacted_by_me: true,
            }]
        );
        assert_eq!(
            messages[0].read_receipt,
            Some(ChatReadReceiptSummary {
                delivered_count: 1,
                read_count: 1,
                display_text: "Read by 1".to_owned(),
            })
        );
    }

    #[test]
    fn chat_projection_retains_complete_history_across_rooms() {
        let owner = DeviceRef {
            account_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            device_id: "phone".to_owned(),
        };
        let plaintext = encode_application_event(
            DurableAppEventKind::ChatMessage,
            None,
            &encode_text_message_payload("retained message", None).unwrap(),
        )
        .unwrap();
        let project = |room_id: &str, seq: u64, message_id: String| {
            project_chat_message(
                room_id.to_owned(),
                seq,
                message_id,
                owner.clone(),
                plaintext.clone(),
                NOW.saturating_add(seq),
                &owner,
            )
            .unwrap()
        };
        let mut projection = ChatProjectionState::default();
        projection.append_messages(
            (0..5_000)
                .map(|index| {
                    project(
                        "mature-room",
                        index as u64 + 2,
                        format!("mature-message-{index}"),
                    )
                })
                .collect(),
            &owner,
        );

        projection.append_messages(
            vec![project("new-room", 1, "new-room-first-message".to_owned())],
            &owner,
        );

        assert!(
            projection.message_exists("new-room", "new-room-first-message"),
            "a fresh message must not be evicted merely because its room-local sequence is low"
        );
        assert!(
            projection.message_exists("mature-room", "mature-message-0"),
            "complete local history must not evict the oldest message"
        );
        assert!(
            projection.message_exists("mature-room", &format!("mature-message-{}", 5_000 - 1)),
            "the newest mature-room message should remain"
        );
        assert_eq!(projection.messages.len(), 5_001);
    }

    #[test]
    fn duplicate_message_projection_cannot_lower_topic_recency() {
        let owner = DeviceRef {
            account_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            device_id: "phone".to_owned(),
        };
        let topic_id = "topic-main";
        let first_chat_id = "chat-first";
        let second_chat_id = "chat-second";
        let message_plaintext = encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            Some(topic_id.to_owned()),
            Some(first_chat_id.to_owned()),
            &encode_text_message_payload("first message", None).unwrap(),
        )
        .unwrap();
        let message = project_chat_message(
            "room-main".to_owned(),
            10,
            "message-10".to_owned(),
            owner.clone(),
            message_plaintext,
            NOW,
            &owner,
        )
        .unwrap();
        let mut projection = ChatProjectionState::default();
        projection.append_messages(vec![message.clone()], &owner);

        let segment = ConversationSegmentStartV1 {
            segment_id: second_chat_id.to_owned(),
            reason: None,
        };
        projection.apply_parsed_event(
            ParsedAppEvent::decode(StoredAppEvent {
                room_id: "room-main".to_owned(),
                seq: 11,
                message_id: "segment-11".to_owned(),
                sender: owner.clone(),
                plaintext: encode_application_event(
                    DurableAppEventKind::ConversationSegmentStart,
                    Some(topic_id.to_owned()),
                    &serde_json::to_vec(&segment).unwrap(),
                )
                .unwrap(),
                timestamp_unix_seconds: NOW + 1,
            }),
            &owner,
        );
        projection.append_messages(vec![message], &owner);

        let topic = projection.conversations.get("room-main", topic_id).unwrap();
        assert_eq!(topic.updated_seq, 11);
        assert_eq!(topic.active_segment_id.as_deref(), Some(second_chat_id));
    }

    #[test]
    fn chat_projection_builds_app_topics_from_conversation_events_and_messages() {
        let owner = DeviceRef {
            account_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            device_id: "phone".to_owned(),
        };
        let peer = DeviceRef {
            account_id: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                .to_owned(),
            device_id: "tablet".to_owned(),
        };
        let metadata = ConversationMetadataV1 {
            title: Some("Build Electron".to_owned()),
            description: Some("Desktop daemon topic".to_owned()),
            external_topic: None,
            skill_binding: None,
        };
        let create_topic = encode_application_event(
            DurableAppEventKind::ConversationCreate,
            Some("topic-main".to_owned()),
            &serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        let topic_message = encode_application_event(
            DurableAppEventKind::ChatMessage,
            Some("topic-main".to_owned()),
            &encode_text_message_payload("hello topic", None).unwrap(),
        )
        .unwrap();
        let segment = ConversationSegmentStartV1 {
            segment_id: "segment-2".to_owned(),
            reason: Some("slash_new".to_owned()),
        };
        let start_segment = encode_application_event(
            DurableAppEventKind::ConversationSegmentStart,
            Some("topic-main".to_owned()),
            &serde_json::to_vec(&segment).unwrap(),
        )
        .unwrap();
        let message_only_topic = encode_application_event(
            DurableAppEventKind::ChatMessage,
            Some("topic-only".to_owned()),
            &encode_text_message_payload("message only topic", None).unwrap(),
        )
        .unwrap();
        let projection = ChatProjectionState::from_stored(
            Vec::new(),
            vec![
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 1,
                    message_id: "topic-create".to_owned(),
                    sender: owner.clone(),
                    plaintext: create_topic,
                    timestamp_unix_seconds: NOW,
                },
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 2,
                    message_id: "message-2".to_owned(),
                    sender: peer.clone(),
                    plaintext: topic_message,
                    timestamp_unix_seconds: NOW + 1,
                },
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 3,
                    message_id: "segment-2".to_owned(),
                    sender: owner,
                    plaintext: start_segment,
                    timestamp_unix_seconds: NOW + 2,
                },
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 4,
                    message_id: "message-4".to_owned(),
                    sender: peer,
                    plaintext: message_only_topic,
                    timestamp_unix_seconds: NOW + 3,
                },
            ],
            &DeviceRef {
                account_id: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                    .to_owned(),
                device_id: "desktop".to_owned(),
            },
        );

        let topics = projection.topics(&BTreeMap::new());
        let topic = topics
            .iter()
            .find(|topic| topic.topic_id == "topic-main")
            .expect("created topic projects");
        assert_eq!(topic.title, "Build Electron");
        assert_eq!(topic.description.as_deref(), Some("Desktop daemon topic"));
        assert_eq!(topic.last_message_preview, "hello topic");
        assert_eq!(topic.message_count, 1);
        assert_eq!(topic.unread_count, 1);
        assert_eq!(topic.created_seq, 1);
        assert_eq!(topic.updated_seq, 3);
        assert_eq!(topic.active_chat_id.as_deref(), Some("segment-2"));
        assert_eq!(topic.chats.len(), 1);
        assert_eq!(topic.chats[0].chat_id, "segment-2");
        assert_eq!(topic.chats[0].started_seq, 3);

        let message_only = topics
            .iter()
            .find(|topic| topic.topic_id == "topic-only")
            .expect("message-only topic projects");
        assert_eq!(message_only.title, "message only topic");
        assert_eq!(message_only.created_seq, 4);
        assert_eq!(message_only.updated_seq, 4);
        assert_eq!(message_only.message_count, 1);
    }

    #[test]
    fn chat_projection_rebuilds_poll_votes_and_survives_duplicate_message_append() {
        let owner = DeviceRef {
            account_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            device_id: "phone".to_owned(),
        };
        let peer = DeviceRef {
            account_id: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                .to_owned(),
            device_id: "tablet".to_owned(),
        };
        let poll_payload = encode_poll_message_payload(
            "Where should we meet?",
            vec!["Office".to_owned(), "Cafe".to_owned()],
        )
        .unwrap();
        let poll_event =
            encode_application_event(DurableAppEventKind::ChatMessage, None, &poll_payload)
                .unwrap();
        let vote = ChatPollVoteV1 {
            poll_message_id: "poll-1".to_owned(),
            option_id: "option-2".to_owned(),
        };
        let vote_payload = serde_json::to_vec(&vote).unwrap();
        let vote_event = encode_application_event(poll_vote_event_kind(), None, &vote_payload)
            .expect("poll vote event encodes");
        let mut projection = ChatProjectionState::from_stored(
            Vec::new(),
            vec![
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 1,
                    message_id: "poll-1".to_owned(),
                    sender: peer.clone(),
                    plaintext: poll_event.clone(),
                    timestamp_unix_seconds: NOW,
                },
                StoredAppEvent {
                    room_id: "room-main".to_owned(),
                    seq: 2,
                    message_id: "vote-1".to_owned(),
                    sender: owner.clone(),
                    plaintext: vote_event,
                    timestamp_unix_seconds: NOW + 1,
                },
            ],
            &owner,
        );
        let messages = projection.messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "Where should we meet?");
        assert_poll_message(
            &messages[0],
            "Where should we meet?",
            "option-2",
            1,
            1,
            true,
        );

        let duplicate = project_chat_message(
            "room-main".to_owned(),
            1,
            "poll-1".to_owned(),
            peer,
            poll_event,
            NOW,
            &owner,
        )
        .expect("poll projects as a transcript row");
        projection.append_messages(vec![duplicate], &owner);
        let messages = projection.messages();
        assert_poll_message(
            &messages[0],
            "Where should we meet?",
            "option-2",
            1,
            1,
            true,
        );
    }

    #[test]
    fn poll_payload_validation_rejects_unusable_shapes() {
        assert!(
            encode_poll_message_payload("Question?", vec!["Only one".to_owned()]).is_err(),
            "single-option polls are not actionable"
        );
        assert!(
            encode_poll_message_payload(
                "Question?",
                vec![
                    "1".to_owned(),
                    "2".to_owned(),
                    "3".to_owned(),
                    "4".to_owned(),
                    "5".to_owned(),
                    "6".to_owned(),
                    "7".to_owned(),
                    "8".to_owned(),
                    "9".to_owned(),
                    "10".to_owned(),
                    "11".to_owned(),
                ],
            )
            .is_err(),
            "poll options are explicitly bounded"
        );
        assert!(
            encode_poll_message_payload("Question?", vec!["Yes".to_owned(), " ".to_owned()])
                .is_err(),
            "blank options are rejected before send"
        );
    }

    #[test]
    fn chat_projection_maps_hermes_reply_sender_and_media() {
        use finitechat_proto::{
            AttachmentBlobEncryptionV1, AttachmentBlobMetadataV1, AttachmentBlobReferenceV1,
            AttachmentDimensionsV1,
        };

        let sender = DeviceRef {
            account_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            device_id: "phone".to_owned(),
        };
        let owner = DeviceRef {
            account_id: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                .to_owned(),
            device_id: "ios".to_owned(),
        };
        let payload = HermesMessagePayloadV1 {
            payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: Some("topic-main".to_owned()),
            segment_id: None,
            text: "photo from Hermes".to_owned(),
            kind: finitechat_hermes::HermesSendKindV1::Media,
            status: finitechat_hermes::HermesMessageStatusV1::Complete,
            edit_of: None,
            attachments: vec![HermesAttachmentV1 {
                kind: HermesAttachmentKindV1::Image,
                name: "ignored.jpg".to_owned(),
                mime_type: "application/octet-stream".to_owned(),
                path: Some("/tmp/local-preview.jpg".to_owned()),
                url: Some("https://cdn.invalid/fallback".to_owned()),
                blob: Some(AttachmentBlobReferenceV1 {
                    scheme: "finitechat.attachment.v1".to_owned(),
                    url: "https://blob.invalid/sha256".to_owned(),
                    ciphertext_sha256: "c".repeat(64),
                    plaintext_sha256: "p".repeat(64),
                    plaintext_size: 12,
                    ciphertext_size: 28,
                    encryption: AttachmentBlobEncryptionV1 {
                        algorithm: "AES-256-GCM".to_owned(),
                        key_hex: "00".repeat(32),
                        nonce_hex: "11".repeat(12),
                    },
                    metadata: AttachmentBlobMetadataV1 {
                        mime_type: "image/jpeg".to_owned(),
                        filename: "photo.jpg".to_owned(),
                        dimensions: Some(AttachmentDimensionsV1 {
                            width: 640,
                            height: 480,
                        }),
                    },
                }),
            }],
            reply_to_message_id: Some("message-parent".to_owned()),
            sender_name: Some("Hermes User".to_owned()),
            metadata: BTreeMap::new(),
        }
        .encode()
        .unwrap();

        let message = project_chat_message(
            "room-main".to_owned(),
            7,
            "message-7".to_owned(),
            sender,
            payload,
            NOW,
            &owner,
        )
        .expect("hermes chat payload should project");

        assert_eq!(message.conversation_id.as_deref(), Some("topic-main"));
        assert_eq!(message.text, "photo from Hermes");
        assert_eq!(message.display_content, "photo from Hermes");
        assert_eq!(
            message.reply_to_message_id.as_deref(),
            Some("message-parent")
        );
        assert_eq!(message.sender_display_name, "Hermes User");
        assert!(!message.is_mine);
        assert!(message.reactions.is_empty());
        assert!(message.read_receipt.is_none());
        assert_eq!(message.media.len(), 1);
        let media = &message.media[0];
        assert_eq!(media.kind, ChatMediaKind::Image);
        assert_eq!(media.url.as_deref(), Some("https://blob.invalid/sha256"));
        assert_eq!(media.mime_type, "image/jpeg");
        assert_eq!(media.filename, "photo.jpg");
        assert_eq!(media.width, Some(640));
        assert_eq!(media.height, Some(480));
        assert_eq!(media.local_path.as_deref(), Some("/tmp/local-preview.jpg"));
    }

    #[test]
    fn chat_projection_builds_hypernote_ast_for_hermes_markdown() {
        let sender = DeviceRef {
            account_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            device_id: "agent".to_owned(),
        };
        let owner = DeviceRef {
            account_id: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                .to_owned(),
            device_id: "ios".to_owned(),
        };
        let markdown = "**Hermes markdown**\n\n- item one\n- item two\n\n`inline code` and [Finite](https://finite.com)";
        let payload = HermesMessagePayloadV1 {
            payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: Some("topic-main".to_owned()),
            segment_id: None,
            text: markdown.to_owned(),
            kind: finitechat_hermes::HermesSendKindV1::Message,
            status: finitechat_hermes::HermesMessageStatusV1::Complete,
            edit_of: None,
            attachments: Vec::new(),
            reply_to_message_id: None,
            sender_name: Some("Hermes".to_owned()),
            metadata: BTreeMap::new(),
        }
        .encode()
        .unwrap();

        let message = project_chat_message(
            "room-main".to_owned(),
            8,
            "message-8".to_owned(),
            sender,
            payload,
            NOW,
            &owner,
        )
        .expect("hermes chat payload should project");

        assert_eq!(message.text, markdown);
        assert_eq!(message.display_content, markdown);
        assert!(!message.rich_text_json.is_empty());
        let ast: serde_json::Value =
            serde_json::from_str(&message.rich_text_json).expect("rich text JSON should parse");
        assert_eq!(ast["type"], "root");
        assert_eq!(ast["source"], markdown);

        let mut node_types = BTreeSet::new();
        collect_json_node_types(&ast, &mut node_types);
        for expected in ["strong", "list_unordered", "code_inline", "link"] {
            assert!(
                node_types.contains(expected),
                "expected Hypernote AST node type {expected}, got {node_types:?}"
            );
        }
    }

    fn collect_json_node_types(value: &serde_json::Value, out: &mut BTreeSet<String>) {
        if let Some(node_type) = value.get("type").and_then(serde_json::Value::as_str) {
            out.insert(node_type.to_owned());
        }
        if let Some(children) = value.get("children").and_then(serde_json::Value::as_array) {
            for child in children {
                collect_json_node_types(child, out);
            }
        }
    }

    #[test]
    fn app_create_room_requires_durable_server_success() {
        let dir = tempfile::tempdir().unwrap();
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: "http://127.0.0.1:1".to_owned(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let error = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "No Server".to_owned(),
            })
            .expect_err("server failure rejects room creation");
        assert!(
            error.to_string().contains("delivery error"),
            "unexpected error: {error}"
        );
        let state = app.state().unwrap();
        assert!(state.rooms.is_empty());
        assert_eq!(state.status, "ready");
    }

    #[test]
    fn app_create_room_rejects_oversized_display_name_before_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let error = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "x".repeat(MAX_PROFILE_DISPLAY_NAME_BYTES as usize + 1),
            })
            .expect_err("oversized room labels fail before network or storage side effects");
        assert!(matches!(error, FiniteChatCoreError::Client { .. }));
        assert!(app.state().unwrap().rooms.is_empty());
    }

    #[test]
    fn app_push_token_actions_register_remove_and_surface_server_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let registered = app
            .dispatch_and_wait(AppAction::SetPushToken {
                token: "  apns-token-alice  ".to_owned(),
            })
            .unwrap();
        assert_eq!(registered.status, "ready");
        app.dispatch_and_wait(AppAction::RemovePushToken).unwrap();

        let error = app
            .dispatch_and_wait(AppAction::SetPushToken {
                token: " ".to_owned(),
            })
            .expect_err("server rejects empty push tokens");
        match error {
            FiniteChatCoreError::ServerRejected { reason } => {
                assert!(reason.contains("push token must be 1..=4096 bytes"));
            }
            other => panic!("expected server rejection, got {other:?}"),
        }
    }

    #[test]
    fn app_runtime_windows_selected_room_transcript_and_loads_older() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let created = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Windowed Chat".to_owned(),
            })
            .unwrap();
        let room_id = created.rooms.first().unwrap().room_id.clone();
        let total_messages = DEFAULT_TRANSCRIPT_WINDOW + 25;
        let mut state = created;
        for index in 0..total_messages {
            state = app
                .dispatch_and_wait(AppAction::SendMessage {
                    room_id: room_id.clone(),
                    text: format!("message-{index:03}"),
                    metadata_json: None,
                })
                .unwrap();
        }

        assert_eq!(state.messages.len(), DEFAULT_TRANSCRIPT_WINDOW);
        assert_eq!(state.messages.first().unwrap().text, "message-025");
        assert_eq!(state.messages.last().unwrap().text, "message-074");
        assert!(app_room(&state, &room_id).can_load_older);

        let stale = app
            .dispatch_and_wait(AppAction::LoadOlderMessages {
                room_id: room_id.clone(),
                before_message_id: "not-the-current-oldest".to_owned(),
                limit: 25,
            })
            .unwrap();
        assert_eq!(stale.messages.len(), DEFAULT_TRANSCRIPT_WINDOW);
        assert_eq!(stale.messages.first().unwrap().text, "message-025");
        assert!(app_room(&stale, &room_id).can_load_older);

        let before_message_id = stale.messages.first().unwrap().message_id.clone();
        let loaded = app
            .dispatch_and_wait(AppAction::LoadOlderMessages {
                room_id: room_id.clone(),
                before_message_id,
                limit: 25,
            })
            .unwrap();
        assert_eq!(loaded.messages.len(), total_messages);
        assert_eq!(loaded.messages.first().unwrap().text, "message-000");
        assert_eq!(loaded.messages.last().unwrap().text, "message-074");
        assert!(!app_room(&loaded, &room_id).can_load_older);

        drop(app);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        assert_eq!(reopened_state.messages.len(), DEFAULT_TRANSCRIPT_WINDOW);
        assert_eq!(reopened_state.messages.first().unwrap().text, "message-025");
        assert_eq!(reopened_state.messages.last().unwrap().text, "message-074");
        assert!(app_room(&reopened_state, &room_id).can_load_older);
    }

    #[test]
    fn app_runtime_cold_relaunch_restores_saved_chat_before_offline_sync() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let created = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Persisted Chat".to_owned(),
            })
            .unwrap();
        let room_id = created.rooms.first().unwrap().room_id.clone();
        app.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "survives force close".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        drop(app);

        let relaunched = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let cached = relaunched.state().unwrap();
        assert_eq!(cached.rooms.len(), 1);
        assert_eq!(cached.rooms[0].room_id, room_id);
        assert_eq!(cached.selected_room_id.as_deref(), Some(room_id.as_str()));
        assert_eq!(cached.messages.len(), 1);
        assert_eq!(cached.messages[0].text, "survives force close");

        let offline = relaunched
            .dispatch_and_wait(AppAction::StartRuntime)
            .unwrap();
        assert_eq!(offline.status, "offline");
        assert_eq!(
            offline.toast.as_deref(),
            Some("Showing saved chats. Connection will retry.")
        );
        assert_eq!(offline.rooms.len(), 1);
        assert_eq!(offline.rooms[0].room_id, room_id);
        assert_eq!(offline.selected_room_id.as_deref(), Some(room_id.as_str()));
        assert_eq!(offline.messages.len(), 1);
        assert_eq!(offline.messages[0].text, "survives force close");
    }

    #[test]
    fn app_scan_npub_loads_server_backed_profile_cache() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let account_id =
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned();
        let npub = npub_encode(&account_id).unwrap();
        put_profile(
            &server_url,
            NostrProfileRecord {
                account_id: account_id.clone(),
                name: Some("alice".to_owned()),
                display_name: Some("Alice Finite".to_owned()),
                about: Some("profile cache test".to_owned()),
                picture: Some("https://example.invalid/alice.png".to_owned()),
                bot: None,
                finite_role: None,
                metadata_json: None,
                fetched_at_ms: NOW.saturating_mul(1000).saturating_sub(1_000),
                expires_at_ms: NOW.saturating_mul(1000).saturating_add(60_000),
            },
        );
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let state = app
            .dispatch_and_wait(AppAction::ScanTarget {
                value: npub.clone(),
            })
            .unwrap();
        assert_eq!(state.status, "profile loaded");
        assert_eq!(
            state.active_profile_id.as_deref(),
            Some(account_id.as_str())
        );
        assert_eq!(state.profiles.len(), 1);
        assert_eq!(state.profiles[0].account_id, account_id);
        assert_eq!(state.profiles[0].npub, npub);
        assert_eq!(state.profiles[0].display_name, "Alice Finite");
        assert_eq!(
            state.profiles[0].about.as_deref(),
            Some("profile cache test")
        );
        assert!(!state.profiles[0].stale);
        drop(app);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let cached = reopened.state().unwrap();
        assert_eq!(cached.profiles.len(), 1);
        assert_eq!(cached.profiles[0].display_name, "Alice Finite");
        assert!(!cached.profiles[0].stale);

        let scanned_offline = reopened
            .dispatch_and_wait(AppAction::ScanTarget {
                value: npub.clone(),
            })
            .unwrap();
        assert_eq!(scanned_offline.status, "profile loaded");
        assert_eq!(
            scanned_offline.active_profile_id.as_deref(),
            Some(account_id.as_str())
        );
        assert_eq!(scanned_offline.profiles[0].display_name, "Alice Finite");
    }

    #[test]
    fn app_start_runtime_loads_signed_in_profile_from_server() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let account_id = app.state().unwrap().identity.account_id;
        let npub = npub_encode(&account_id).unwrap();

        put_profile(
            &server_url,
            NostrProfileRecord {
                account_id: account_id.clone(),
                name: Some("alice".to_owned()),
                display_name: Some("Alice Real Npub".to_owned()),
                about: Some("signed-in profile".to_owned()),
                picture: Some("https://example.invalid/alice-real.png".to_owned()),
                bot: None,
                finite_role: None,
                metadata_json: None,
                fetched_at_ms: NOW.saturating_mul(1000).saturating_sub(1_000),
                expires_at_ms: NOW.saturating_mul(1000).saturating_add(60_000),
            },
        );

        let started = app.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(started.active_profile_id, None);
        let profile = started
            .profiles
            .iter()
            .find(|profile| profile.account_id == account_id)
            .expect("signed-in profile in app state");
        assert_eq!(profile.npub, npub);
        assert_eq!(profile.display_name, "Alice Real Npub");
        assert_eq!(profile.about.as_deref(), Some("signed-in profile"));
        assert_eq!(
            profile.picture.as_deref(),
            Some("https://example.invalid/alice-real.png")
        );
        assert!(!profile.stale);
    }

    #[test]
    fn app_save_profile_publishes_caches_and_persists_picture() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let account_id = app.state().unwrap().identity.account_id;

        let saved = app
            .dispatch_and_wait(AppAction::SaveProfile {
                display_name: "Alice Finite".to_owned(),
                about: "Encrypted chat operator".to_owned(),
                picture: Some("https://example.invalid/alice.png".to_owned()),
            })
            .unwrap();

        assert_eq!(saved.status, "profile saved");
        let profile = saved
            .profiles
            .iter()
            .find(|profile| profile.account_id == account_id)
            .expect("saved profile in app state");
        assert_eq!(profile.display_name, "Alice Finite");
        assert_eq!(profile.about.as_deref(), Some("Encrypted chat operator"));
        assert_eq!(
            profile.picture.as_deref(),
            Some("https://example.invalid/alice.png")
        );
        assert!(!profile.stale);

        let server_profiles = get_profiles(&server_url, vec![account_id.clone()]);
        assert_eq!(server_profiles.profiles.len(), 1);
        assert_eq!(
            server_profiles.profiles[0].profile.picture.as_deref(),
            Some("https://example.invalid/alice.png")
        );

        drop(app);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let cached = reopened.state().unwrap();
        let profile = cached
            .profiles
            .iter()
            .find(|profile| profile.account_id == account_id)
            .expect("saved profile survives offline relaunch");
        assert_eq!(
            profile.picture.as_deref(),
            Some("https://example.invalid/alice.png")
        );
    }

    #[test]
    fn app_upload_image_returns_public_blob_url_for_profile_and_room_save() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let account_id = app.state().unwrap().identity.account_id;
        let png = b"\x89PNG\r\n\x1a\nprofile image bytes".to_vec();

        let picture_url = app
            .dispatch_and_wait(AppAction::UploadImage {
                bytes: png.clone(),
                content_type: "image/png".to_owned(),
            })
            .unwrap();
        assert_eq!(picture_url.status, "image uploaded");
        let picture_url = picture_url
            .flow
            .image_upload_url
            .expect("image upload action returns blob URL");
        assert!(picture_url.starts_with(&format!("{server_url}/blobs/")));

        let response = reqwest::blocking::Client::new()
            .get(&picture_url)
            .send()
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("image/png")
        );
        assert_eq!(response.bytes().unwrap().as_ref(), png.as_slice());

        let saved = app
            .dispatch_and_wait(AppAction::SaveProfile {
                display_name: "Alice Finite".to_owned(),
                about: "Encrypted chat operator".to_owned(),
                picture: Some(picture_url.clone()),
            })
            .unwrap();
        let profile = saved
            .profiles
            .iter()
            .find(|profile| profile.account_id == account_id)
            .expect("saved profile in app state");
        assert_eq!(profile.picture.as_deref(), Some(picture_url.as_str()));

        let room_state = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Room Metadata".to_owned(),
            })
            .unwrap();
        let room_id = room_state
            .selected_room_id
            .as_deref()
            .expect("selected created room")
            .to_owned();
        let saved_room = app
            .dispatch_and_wait(AppAction::SaveRoomMetadata {
                room_id: room_id.clone(),
                display_name: "Room Metadata Saved".to_owned(),
                picture: Some(picture_url.clone()),
            })
            .unwrap();
        let room = app_room(&saved_room, &room_id);
        assert_eq!(room.display_name, "Room Metadata Saved");
        assert_eq!(room.picture.as_deref(), Some(picture_url.as_str()));
        let details = saved_room.room_details.as_ref().expect("room details");
        assert_eq!(details.display_name, "Room Metadata Saved");
        assert_eq!(details.picture.as_deref(), Some(picture_url.as_str()));
        drop(app);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        let room = app_room(&reopened_state, &room_id);
        assert_eq!(room.display_name, "Room Metadata Saved");
        assert_eq!(room.picture.as_deref(), Some(picture_url.as_str()));
    }

    #[test]
    fn app_upload_image_rejects_mismatched_content_type() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let error = app
            .dispatch_and_wait(AppAction::UploadImage {
                bytes: b"not an image".to_vec(),
                content_type: "image/png".to_owned(),
            })
            .unwrap_err();
        assert!(error.to_string().contains("do not match image/png"));
    }

    #[test]
    fn app_upload_image_rejects_oversized_public_image_before_http_upload() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let mut bytes = vec![0; MAX_PUBLIC_IMAGE_UPLOAD_BYTES + 1];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");

        let error = app
            .dispatch_and_wait(AppAction::UploadImage {
                bytes,
                content_type: "image/png".to_owned(),
            })
            .unwrap_err();
        assert!(error.to_string().contains("public image is too large"));
    }

    #[test]
    fn app_scan_nprofile_loads_server_backed_profile_cache() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let account_id =
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned();
        let npub = npub_encode(&account_id).unwrap();
        let nprofile = "nprofile1qqsqzg69v7y6hn00qy352euf40x77qfrg4ncn27dauqjx3t83x4ummcs22eux";
        put_profile(
            &server_url,
            NostrProfileRecord {
                account_id: account_id.clone(),
                name: Some("alice".to_owned()),
                display_name: Some("Alice Nprofile".to_owned()),
                about: Some("nprofile cache test".to_owned()),
                picture: None,
                bot: None,
                finite_role: None,
                metadata_json: None,
                fetched_at_ms: NOW.saturating_mul(1000).saturating_sub(1_000),
                expires_at_ms: NOW.saturating_mul(1000).saturating_add(60_000),
            },
        );
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let state = app
            .dispatch_and_wait(AppAction::ScanTarget {
                value: format!("nostr:{nprofile}"),
            })
            .unwrap();

        assert_eq!(state.status, "profile loaded");
        assert_eq!(
            state.active_profile_id.as_deref(),
            Some(account_id.as_str())
        );
        assert_eq!(state.profiles[0].account_id, account_id);
        assert_eq!(state.profiles[0].npub, npub);
        assert_eq!(state.profiles[0].display_name, "Alice Nprofile");
        assert!(!state.profiles[0].stale);
    }

    #[test]
    fn app_scan_missing_npub_surfaces_stale_profile_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let account_id =
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".to_owned();
        let npub = npub_encode(&account_id).unwrap();
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let state = app
            .dispatch_and_wait(AppAction::ScanTarget {
                value: format!("nostr:{npub}"),
            })
            .unwrap();
        assert_eq!(state.status, "profile not found");
        assert_eq!(
            state.toast.as_deref(),
            Some("No cached profile was available for that npub")
        );
        assert_eq!(
            state.active_profile_id.as_deref(),
            Some(account_id.as_str())
        );
        assert_eq!(state.profiles.len(), 1);
        assert_eq!(state.profiles[0].account_id, account_id);
        assert_eq!(state.profiles[0].npub, npub);
        assert!(state.profiles[0].stale);
        drop(app);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let cached = reopened.state().unwrap();
        assert_eq!(cached.profiles.len(), 1);
        assert_eq!(cached.profiles[0].account_id, account_id);
        assert!(cached.profiles[0].stale);
    }

    #[test]
    fn app_scan_profile_url_query_npub_loads_server_backed_profile_cache() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let account_id =
            "2222222222222222222222222222222222222222222222222222222222222222".to_owned();
        let npub = npub_encode(&account_id).unwrap();
        put_profile(
            &server_url,
            NostrProfileRecord {
                account_id: account_id.clone(),
                name: Some("alice-url".to_owned()),
                display_name: Some("Alice URL".to_owned()),
                about: None,
                picture: None,
                bot: None,
                finite_role: None,
                metadata_json: None,
                fetched_at_ms: NOW.saturating_mul(1000).saturating_sub(1_000),
                expires_at_ms: NOW.saturating_mul(1000).saturating_add(60_000),
            },
        );
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let state = app
            .dispatch_and_wait(AppAction::ScanTarget {
                value: format!("https://finite.computer/profile?npub={npub}&source=qr"),
            })
            .unwrap();

        assert_eq!(state.status, "profile loaded");
        assert_eq!(
            state.active_profile_id.as_deref(),
            Some(account_id.as_str())
        );
        assert_eq!(state.profiles[0].display_name, "Alice URL");
        assert_eq!(state.profiles[0].npub, npub);
    }

    #[test]
    fn app_scan_profile_url_query_nprofile_loads_server_backed_profile_cache() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let account_id =
            "2222222222222222222222222222222222222222222222222222222222222222".to_owned();
        let npub = npub_encode(&account_id).unwrap();
        let nprofile = "nprofile1qqszyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygsmjs029";
        put_profile(
            &server_url,
            NostrProfileRecord {
                account_id: account_id.clone(),
                name: Some("alice-url".to_owned()),
                display_name: Some("Alice URL Nprofile".to_owned()),
                about: None,
                picture: None,
                bot: None,
                finite_role: None,
                metadata_json: None,
                fetched_at_ms: NOW.saturating_mul(1000).saturating_sub(1_000),
                expires_at_ms: NOW.saturating_mul(1000).saturating_add(60_000),
            },
        );
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let state = app
            .dispatch_and_wait(AppAction::ScanTarget {
                value: format!("https://finite.computer/profile?nprofile={nprofile}&source=qr"),
            })
            .unwrap();

        assert_eq!(state.status, "profile loaded");
        assert_eq!(
            state.active_profile_id.as_deref(),
            Some(account_id.as_str())
        );
        assert_eq!(state.profiles[0].display_name, "Alice URL Nprofile");
        assert_eq!(state.profiles[0].npub, npub);
    }

    #[test]
    fn app_scan_embedded_npub_falls_back_to_profile_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let account_id =
            "3333333333333333333333333333333333333333333333333333333333333333".to_owned();
        let npub = npub_encode(&account_id).unwrap();
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let state = app
            .dispatch_and_wait(AppAction::ScanTarget {
                value: format!("Profile code: {npub}\nShared from Finite Chat"),
            })
            .unwrap();

        assert_eq!(state.status, "profile details unavailable");
        assert_eq!(
            state.active_profile_id.as_deref(),
            Some(account_id.as_str())
        );
        assert_eq!(state.profiles[0].account_id, account_id);
        assert_eq!(state.profiles[0].npub, npub);
        assert!(state.profiles[0].stale);
    }

    #[test]
    fn app_profile_scan_offline_without_cache_surfaces_stale_profile_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let account_id =
            "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210".to_owned();
        let npub = npub_encode(&account_id).unwrap();
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let state = app
            .dispatch_and_wait(AppAction::ScanTarget {
                value: npub.clone(),
            })
            .unwrap();
        assert_eq!(state.status, "profile details unavailable");
        assert_eq!(
            state.toast.as_deref(),
            Some("Profile details unavailable; you can still start a chat")
        );
        assert_eq!(
            state.active_profile_id.as_deref(),
            Some(account_id.as_str())
        );
        assert_eq!(state.profiles.len(), 1);
        assert_eq!(state.profiles[0].account_id, account_id);
        assert_eq!(state.profiles[0].npub, npub);
        assert!(state.profiles[0].stale);
        assert!(state.messages.is_empty());
        assert!(runtime_outbox(&app).is_empty());
        drop(app);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        assert_eq!(reopened_state.active_profile_id, None);
        assert_eq!(reopened_state.profiles.len(), 1);
        assert_eq!(reopened_state.profiles[0].account_id, account_id);
        assert!(reopened_state.profiles[0].stale);
    }

    #[test]
    fn app_runtime_adds_member_and_joiner_sends_without_protocol_actions() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let carol = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("carol").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "carol-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let carol_account_id = carol.state().unwrap().identity.account_id;

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Agent Room".to_owned(),
            })
            .unwrap();
        let room = alice_state.rooms.first().expect("created room");
        let room_id = room.room_id.clone();
        assert_eq!(room.display_name, "Agent Room");
        assert_eq!(room.state, AppRoomState::Connected);

        let bob_account_id = bob.state().unwrap().identity.account_id;
        add_runtime_member(&alice, &bob, &room_id, test_profile(&bob_account_id, "Bob"));
        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&bob_state, &room_id).state,
            AppRoomState::Connected
        );

        carol
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("carol publishes key packages");
        let bob_added_carol = bob
            .dispatch_and_wait(AppAction::AddRoomMembers {
                room_id: room_id.clone(),
                profiles: vec![test_profile(&carol_account_id, "Carol")],
            })
            .unwrap();
        assert_eq!(bob_added_carol.status, "people added");
        let carol_state = carol.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&carol_state, &room_id).state,
            AppRoomState::Connected
        );

        let sent = bob
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "hello from app actor".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        assert!(
            sent.messages
                .iter()
                .any(|message| message.text == "hello from app actor")
        );

        let alice_state = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            alice_state
                .messages
                .iter()
                .any(|message| message.text == "hello from app actor")
        );
        drop(alice);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        assert!(
            reopened_state
                .messages
                .iter()
                .any(|message| message.text == "hello from app actor"),
            "message projection should survive runtime reopen"
        );
        assert_eq!(
            app_room(&reopened_state, &room_id).last_message_preview,
            "hello from app actor"
        );
    }

    #[test]
    fn app_runtime_reopened_owner_adds_member_from_persisted_room() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-electron".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Persisted Welcome Room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state
            .selected_room_id
            .expect("created room is selected");
        drop(alice);

        let reopened_alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-electron".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob_account_id = bob.state().unwrap().identity.account_id;
        add_runtime_member(
            &reopened_alice,
            &bob,
            &room_id,
            test_profile(&bob_account_id, "Bob"),
        );
        let bob_joined = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&bob_joined, &room_id).state,
            AppRoomState::Connected
        );
    }

    #[test]
    fn app_runtime_agent_bridge_poll_observes_native_app_after_add() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let agent = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("agent").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("ios-app").to_string_lossy().into_owned(),
            server_url,
            device_id: "ios-hermes-media-sim".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let agent_state = agent
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Hermes Bridge Welcome".to_owned(),
            })
            .unwrap();
        let room_id = agent_state.rooms.first().unwrap().room_id.clone();
        let app_account_id = app.state().unwrap().identity.account_id;
        add_runtime_member(
            &agent,
            &app,
            &room_id,
            test_profile(&app_account_id, "iOS App"),
        );

        let bridge = agent.agent_bridge_poll_once().unwrap();
        assert_eq!(bridge.joined_account_ids.len(), 1);

        let app_state = app.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&app_state, &room_id).state,
            AppRoomState::Connected
        );

        agent
            .send_encoded_chat_message_and_wait(
                room_id.clone(),
                encode_text_message_payload("agent reply", None).unwrap(),
                "agent reply".to_owned(),
            )
            .unwrap();
        let bridge = agent.agent_bridge_poll_once().unwrap();
        assert!(
            bridge.events.is_empty(),
            "agent bridge poll should not surface its own sent message"
        );
    }

    #[test]
    fn app_runtime_agent_bridge_inbox_hint_activates_welcome_without_polling() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let agent = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("agent").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let user = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("hosted-web").to_string_lossy().into_owned(),
            server_url,
            device_id: "hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let agent_state = agent
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("agent publishes KeyPackages");
        assert!(agent_state.rooms.is_empty());
        assert_eq!(
            agent
                .wait_plan(5_000)
                .unwrap()
                .request
                .inbox
                .expect("every Device watches its own inbox")
                .after_seq,
            0
        );

        let waiting_agent = Arc::clone(&agent);
        let waiter = std::thread::spawn(move || waiting_agent.agent_bridge_wait_for_update(5_000));

        let agent_account_id = agent_state.identity.account_id;
        let user_state = user
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: test_profile(&agent_account_id, "Agent"),
                display_name: "Chat with Agent".to_owned(),
            })
            .expect("hosted Device adds the zero-room agent");
        let room_id = user_state.rooms.first().unwrap().room_id.clone();

        waiter
            .join()
            .expect("agent inbox waiter thread")
            .expect("inbox hint runs the normal sync path");
        let joined = agent.state().unwrap();
        assert_eq!(app_room(&joined, &room_id).state, AppRoomState::Connected);
        assert_eq!(
            agent
                .wait_plan(5_000)
                .unwrap()
                .request
                .inbox
                .expect("inbox watch remains active after Welcome activation")
                .after_seq,
            1
        );
    }

    #[test]
    fn app_runtime_agent_bridge_startup_poll_and_heartbeat_reconcile_typed_commands() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let agent_core = CoreState::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("agent").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let mut agent = AppRuntimeState::new(agent_core).unwrap();
        agent
            .start_runtime()
            .expect("agent publishes KeyPackages before the resident wait loop");
        let agent_account_id = agent.app.identity.account_id.clone();

        let hosted = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("hosted-web").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let hosted_state = hosted
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: test_profile(&agent_account_id, "Agent"),
                display_name: "Chat with Agent".to_owned(),
            })
            .expect("hosted Device creates the agent room");
        let room_id = hosted_state
            .selected_room_id
            .expect("agent room is selected");

        agent
            .agent_bridge_apply_sync_hint(SyncHintEvent::InboxAdvanced { seq: 1 })
            .expect("agent activates its Welcome before command delivery");

        let hosted_identity = hosted.state().unwrap().identity;
        let electron = FiniteChatRuntime::open(OpenOptions {
            data_dir: dir.path().join("electron").to_string_lossy().into_owned(),
            server_url,
            device_id: "electron-heartbeat".to_owned(),
            account_secret_hex: Some(hosted_identity.account_secret_hex),
            now_unix_seconds: Some(NOW),
        })
        .unwrap();
        electron
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("linked Device publishes a KeyPackage");
        let (fanout, _) = drive_device_link_to_exact_receipts(
            &hosted,
            &electron,
            "link-heartbeat-reconciliation",
            "electron-heartbeat",
        );
        assert!(fanout.fanout_complete);

        let before_seq = agent
            .wait_plan(5_000)
            .request
            .rooms
            .iter()
            .find(|room| room.room_id == room_id)
            .expect("joined room has a durable cursor")
            .after_seq;

        let runtime_request = |request_id: &str| RuntimeCommandRequestV1 {
            payload_kind: finitechat_proto::RuntimeCommandPayloadKindV1::Request,
            request_id: request_id.to_owned(),
            command: "agent.connections.status".to_owned(),
            target: finitechat_proto::RuntimeCommandTargetV1 {
                account_id: agent_account_id.clone(),
                device_id: None,
            },
            resource_key: Some("agent.connections".to_owned()),
            body: finitechat_proto::RuntimeCommandJsonPayloadV1 {
                schema: "finite.agent.empty.request.v1".to_owned(),
                json_payload: serde_json::to_vec(&serde_json::json!({})).unwrap(),
            },
        };
        let delivered_request_ids = |sync: &AppBridgeSync| {
            sync.events
                .iter()
                .filter_map(|stored| {
                    let event =
                        serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext)
                            .ok()?;
                    if event.kind != DurableAppEventKind::RuntimeCommandRequest {
                        return None;
                    }
                    serde_json::from_slice::<RuntimeCommandRequestV1>(&event.payload)
                        .ok()
                        .map(|request| request.request_id)
                })
                .collect::<Vec<_>>()
        };

        let startup_request_id = "request-before-startup-reconciliation";
        hosted
            .send_runtime_command_request_and_wait(
                room_id.clone(),
                None,
                serde_json::to_vec(&runtime_request(startup_request_id)).unwrap(),
            )
            .expect("hosted Device durably queues a typed command");
        assert_eq!(
            agent
                .wait_plan(5_000)
                .request
                .rooms
                .iter()
                .find(|room| room.room_id == room_id)
                .unwrap()
                .after_seq,
            before_seq,
            "the local Device remains behind until its resident sync reconciles"
        );

        let startup_recovered = agent
            .agent_bridge_poll_once()
            .expect("resident startup performs an immediate bounded reconciliation");
        assert!(
            delivered_request_ids(&startup_recovered)
                .iter()
                .any(|request_id| request_id == startup_request_id),
            "the startup reconciliation makes the queued typed command deliverable"
        );
        let heartbeat_before_seq = agent
            .wait_plan(5_000)
            .request
            .rooms
            .iter()
            .find(|room| room.room_id == room_id)
            .unwrap()
            .after_seq;
        assert!(
            heartbeat_before_seq > before_seq,
            "startup reconciliation advances through the membership commit"
        );

        let heartbeat_request_id = "request-before-heartbeat-reconciliation";
        hosted
            .send_runtime_command_request_and_wait(
                room_id.clone(),
                None,
                serde_json::to_vec(&runtime_request(heartbeat_request_id)).unwrap(),
            )
            .expect("hosted Device durably queues a second typed command");
        assert_eq!(
            agent
                .wait_plan(5_000)
                .request
                .rooms
                .iter()
                .find(|room| room.room_id == room_id)
                .unwrap()
                .after_seq,
            heartbeat_before_seq,
            "the second command remains queued until the next resident reconciliation"
        );

        let heartbeat_recovered = agent
            .agent_bridge_apply_sync_hint(SyncHintEvent::Heartbeat)
            .expect("a heartbeat performs the bounded durable reconciliation");
        assert!(
            delivered_request_ids(&heartbeat_recovered)
                .iter()
                .any(|request_id| request_id == heartbeat_request_id),
            "the heartbeat reconciliation makes the second typed command deliverable"
        );
        assert!(
            agent
                .wait_plan(5_000)
                .request
                .rooms
                .iter()
                .find(|room| room.room_id == room_id)
                .unwrap()
                .after_seq
                > heartbeat_before_seq,
            "heartbeat reconciliation advances the durable room cursor"
        );
    }

    #[test]
    fn app_runtime_room_hint_syncs_only_target_and_heartbeat_recovers_other_room() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let agent_options = with_test_secret(OpenOptions {
            data_dir: dir.path().join("agent").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        });
        let user_options = with_test_secret(OpenOptions {
            data_dir: dir.path().join("user").to_string_lossy().into_owned(),
            server_url,
            device_id: "user".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        });
        let agent_runtime = FiniteChatRuntime::open(agent_options.clone()).unwrap();
        let user_runtime = FiniteChatRuntime::open(user_options).unwrap();
        let user_account_id = user_runtime.state().unwrap().identity.account_id;

        let target_room_id = agent_runtime
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Targeted room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        add_runtime_member(
            &agent_runtime,
            &user_runtime,
            &target_room_id,
            test_profile(&user_account_id, "User"),
        );
        let recovery_room_id = agent_runtime
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Heartbeat recovery room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        add_runtime_member(
            &agent_runtime,
            &user_runtime,
            &recovery_room_id,
            test_profile(&user_account_id, "User"),
        );
        drop(agent_runtime);

        let agent_core = CoreState::open(agent_options.clone()).unwrap();
        let mut agent = AppRuntimeState::new(agent_core).unwrap();
        let recovery_before = agent
            .core
            .device
            .last_applied_seq(&recovery_room_id)
            .unwrap();

        user_runtime
            .send_encoded_chat_message_and_wait(
                target_room_id.clone(),
                encode_text_message_payload("targeted delivery", None).unwrap(),
                "targeted delivery".to_owned(),
            )
            .unwrap();
        user_runtime
            .send_encoded_chat_message_and_wait(
                recovery_room_id.clone(),
                encode_text_message_payload("heartbeat recovery", None).unwrap(),
                "heartbeat recovery".to_owned(),
            )
            .unwrap();

        let targeted = agent
            .agent_bridge_apply_sync_hint(SyncHintEvent::RoomAdvanced {
                room_id: target_room_id.clone(),
                seq: u64::MAX,
            })
            .expect("a precise room hint syncs its target");
        assert!(
            targeted
                .events
                .iter()
                .any(|event| event.room_id == target_room_id)
        );
        assert!(
            targeted
                .events
                .iter()
                .all(|event| event.room_id != recovery_room_id),
            "the precise room hint must not consume another room"
        );
        assert_eq!(
            agent
                .core
                .device
                .last_applied_seq(&recovery_room_id)
                .unwrap(),
            recovery_before,
            "the unhinted room keeps its durable cursor"
        );
        drop(agent);

        let reopened_core = CoreState::open(agent_options).unwrap();
        let mut reopened = AppRuntimeState::new(reopened_core).unwrap();
        let recovered = reopened
            .agent_bridge_apply_sync_hint(SyncHintEvent::Heartbeat)
            .expect("heartbeat performs full reconciliation after restart");
        assert!(
            recovered
                .events
                .iter()
                .any(|event| event.room_id == recovery_room_id)
        );
    }

    #[test]
    fn app_runtime_agent_bridge_quarantines_broken_room_and_delivers_fresh_room_command() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let agent_dir = dir.path().join("agent");
        let user_dir = dir.path().join("hosted-web");
        let agent_options = with_test_secret(OpenOptions {
            data_dir: agent_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        });
        let user_options = with_test_secret(OpenOptions {
            data_dir: user_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        });
        let agent_runtime = FiniteChatRuntime::open(agent_options.clone()).unwrap();
        let user_runtime = FiniteChatRuntime::open(user_options.clone()).unwrap();
        let user_account_id = user_runtime.state().unwrap().identity.account_id;

        let first_room = agent_runtime
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "First direct room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        add_runtime_member(
            &agent_runtime,
            &user_runtime,
            &first_room,
            test_profile(&user_account_id, "User"),
        );
        let second_room = agent_runtime
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Fresh recovery room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        add_runtime_member(
            &agent_runtime,
            &user_runtime,
            &second_room,
            test_profile(&user_account_id, "User"),
        );

        let mut room_ids = [first_room, second_room];
        room_ids.sort();
        let [broken_room_id, fresh_room_id] = room_ids;
        drop(agent_runtime);
        drop(user_runtime);

        let agent_core = CoreState::open(agent_options).unwrap();
        let mut agent = AppRuntimeState::new(agent_core).unwrap();
        let user_core = CoreState::open(user_options).unwrap();
        let mut user = AppRuntimeState::new(user_core).unwrap();

        let broken_before = agent.core.device.last_applied_seq(&broken_room_id).unwrap();
        let fresh_before = agent.core.device.last_applied_seq(&fresh_room_id).unwrap();

        let mut invalid = user
            .core
            .device
            .create_application_request(
                &broken_room_id,
                b"ciphertext deliberately corrupted after creation",
                "invalid-old-room-entry",
            )
            .unwrap();
        invalid.envelope.payload = vec![0xff; 32];
        user.core
            .store
            .save_device_state(&user.core.device)
            .unwrap();
        let mut delivery = delivery_for(&server_url);
        delivery
            .append_event(
                &invalid,
                DurableAppEventKind::RuntimeCommandRequest.delivery_policy(),
            )
            .expect("the opaque server stores the malformed MLS ciphertext");

        let request_id = "request-in-fresh-recovery-room";
        let request = RuntimeCommandRequestV1 {
            payload_kind: finitechat_proto::RuntimeCommandPayloadKindV1::Request,
            request_id: request_id.to_owned(),
            command: "agent.connections.status".to_owned(),
            target: finitechat_proto::RuntimeCommandTargetV1 {
                account_id: agent.app.identity.account_id.clone(),
                device_id: None,
            },
            resource_key: Some("agent.connections".to_owned()),
            body: finitechat_proto::RuntimeCommandJsonPayloadV1 {
                schema: "finite.agent.empty.request.v1".to_owned(),
                json_payload: serde_json::to_vec(&serde_json::json!({})).unwrap(),
            },
        };
        user.send_runtime_command_request(
            fresh_room_id.clone(),
            None,
            DurableAppEventKind::RuntimeCommandRequest,
            serde_json::to_vec(&request).unwrap(),
        )
        .expect("the fresh room accepts a typed runtime command");

        let bridge = agent
            .agent_bridge_poll_once()
            .expect("one invalid old room must not abort the fresh room");
        assert!(bridge.events.iter().any(|stored| {
            let Ok(event) =
                serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext)
            else {
                return false;
            };
            event.kind == DurableAppEventKind::RuntimeCommandRequest
                && serde_json::from_slice::<RuntimeCommandRequestV1>(&event.payload)
                    .is_ok_and(|request| request.request_id == request_id)
        }));
        assert_eq!(agent.app.status, "ready; 1 room needs repair");
        assert_eq!(
            agent.core.device.last_applied_seq(&broken_room_id).unwrap(),
            broken_before,
            "the rejected room keeps its durable cursor"
        );
        assert!(
            agent.core.device.last_applied_seq(&fresh_room_id).unwrap() > fresh_before,
            "the healthy room advances independently"
        );

        let reloaded = agent
            .core
            .store
            .load_device(FiniteChatDeviceConfig {
                account_secret_key: agent.core.account_secret.clone(),
                device_id: agent.core.device.device_ref().device_id.clone(),
                now_unix_seconds: NOW,
                credential_not_before_unix_seconds: 0,
                credential_not_after_unix_seconds: u64::MAX,
            })
            .unwrap();
        assert_eq!(
            reloaded.last_applied_seq(&broken_room_id).unwrap(),
            broken_before,
            "the failed room does not persist a partial advance"
        );
        assert!(reloaded.last_applied_seq(&fresh_room_id).unwrap() > fresh_before);

        let idle = agent
            .agent_bridge_poll_once()
            .expect("an idle healthy room keeps the quarantined bridge degraded-ready");
        assert!(idle.events.is_empty());
        assert_eq!(agent.app.status, "ready; 1 room needs repair");
    }

    #[test]
    fn app_runtime_failed_inbox_hint_sync_does_not_advance_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let core = CoreState::open(with_test_secret(OpenOptions {
            data_dir: dir
                .path()
                .join("offline-agent")
                .to_string_lossy()
                .into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "offline-agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let mut state = AppRuntimeState::new(core).unwrap();

        state
            .agent_bridge_apply_sync_hint(SyncHintEvent::InboxAdvanced { seq: 7 })
            .expect_err("offline full sync must fail");
        assert_eq!(
            state
                .wait_plan(5_000)
                .request
                .inbox
                .expect("offline Device still watches its inbox")
                .after_seq,
            0,
            "a reconnect must retry the same inbox hint until the pull/activate/ack tick succeeds"
        );
    }

    #[test]
    fn app_profile_chat_claims_key_package_and_sends_welcome_via_welcome() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let bob_account_id = bob.state().unwrap().identity.account_id;
        bob.dispatch_and_wait(AppAction::StartRuntime)
            .expect("bob publishes key packages");

        let alice_state = alice
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: test_profile_with_picture(
                    &bob_account_id,
                    "Bob",
                    "https://example.invalid/bob.png",
                ),
                display_name: "Chat with Bob".to_owned(),
            })
            .unwrap();
        assert_eq!(alice_state.status, "chat created");
        let room = alice_state.rooms.first().expect("direct room");
        assert_eq!(room.display_name, "Chat with Bob");
        assert_eq!(room.state, AppRoomState::Connected);
        let room_id = room.room_id.clone();
        let bob_profile = app_profile(&alice_state, &bob_account_id);
        assert_eq!(bob_profile.display_name, "Bob");
        assert_eq!(
            bob_profile.picture.as_deref(),
            Some("https://example.invalid/bob.png")
        );
        let details = alice_state
            .room_details
            .as_ref()
            .expect("direct room details");
        let bob_member = room_details_member(details, &bob_account_id, "bob-ios");
        assert_eq!(bob_member.display_name, "Bob");
        assert_eq!(
            bob_member.picture.as_deref(),
            Some("https://example.invalid/bob.png")
        );

        let topic_state = alice
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "Remember this chat".to_owned(),
            })
            .unwrap();
        let selected_topic_id = topic_state
            .selected_topic_id
            .expect("new topic is selected");
        let selected_chat_id = topic_state
            .selected_chat_id
            .expect("new topic's first chat is selected");
        let second_chat_state = alice
            .dispatch_and_wait(AppAction::StartTopicChat {
                room_id: room_id.clone(),
                topic_id: selected_topic_id.clone(),
                reason: Some("selection persistence regression".to_owned()),
            })
            .unwrap();
        let second_chat_id = second_chat_state
            .selected_chat_id
            .expect("second chat is selected");
        assert_ne!(second_chat_id, selected_chat_id);
        let selected_chat_state = alice
            .dispatch_and_wait(AppAction::OpenChat {
                room_id: room_id.clone(),
                topic_id: selected_topic_id.clone(),
                chat_id: selected_chat_id.clone(),
            })
            .unwrap();
        assert_eq!(
            selected_chat_state.selected_chat_id.as_deref(),
            Some(selected_chat_id.as_str())
        );

        let reopened_state = alice
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: test_profile(&bob_account_id, "Bob"),
                display_name: "Chat with Bob".to_owned(),
            })
            .unwrap();
        assert_eq!(reopened_state.status, "chat opened");
        assert_eq!(
            reopened_state.selected_room_id.as_deref(),
            Some(room_id.as_str())
        );
        assert_eq!(
            reopened_state.selected_topic_id.as_deref(),
            Some(selected_topic_id.as_str())
        );
        assert_eq!(
            reopened_state.selected_chat_id.as_deref(),
            Some(selected_chat_id.as_str())
        );
        assert_eq!(reopened_state.rooms.len(), 1);
        assert_eq!(
            app_room(&reopened_state, &room_id).state,
            AppRoomState::Connected
        );
        drop(alice);

        let reopened_alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let cold_state = reopened_alice.state().unwrap();
        assert_eq!(
            cold_state.selected_topic_id.as_deref(),
            Some(selected_topic_id.as_str())
        );
        assert_eq!(
            cold_state.selected_chat_id.as_deref(),
            Some(selected_chat_id.as_str())
        );
        let disk_reopened_state = reopened_alice
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: test_profile(&bob_account_id, "Bob"),
                display_name: "Chat with Bob".to_owned(),
            })
            .unwrap();
        assert_eq!(disk_reopened_state.status, "chat opened");
        assert_eq!(
            disk_reopened_state.selected_room_id.as_deref(),
            Some(room_id.as_str())
        );
        assert_eq!(
            disk_reopened_state.selected_topic_id.as_deref(),
            Some(selected_topic_id.as_str())
        );
        assert_eq!(
            disk_reopened_state.selected_chat_id.as_deref(),
            Some(selected_chat_id.as_str())
        );
        assert_eq!(
            app_room(&disk_reopened_state, &room_id).state,
            AppRoomState::Connected
        );

        let other_room_state = reopened_alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Another room".to_owned(),
            })
            .unwrap();
        assert_ne!(
            other_room_state.selected_room_id.as_deref(),
            Some(room_id.as_str())
        );
        let switched_room_state = reopened_alice
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: test_profile(&bob_account_id, "Bob"),
                display_name: "Chat with Bob".to_owned(),
            })
            .unwrap();
        assert_eq!(
            switched_room_state.selected_room_id.as_deref(),
            Some(room_id.as_str())
        );
        assert_eq!(
            switched_room_state.selected_topic_id.as_deref(),
            Some(HOME_TOPIC_ID)
        );
        assert_eq!(
            switched_room_state.selected_chat_id.as_deref(),
            Some(HOME_CHAT_ID)
        );

        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let bob_room = app_room(&bob_state, &room_id);
        assert_eq!(bob_room.state, AppRoomState::Connected);
        bob.dispatch_and_wait(AppAction::OpenRoom {
            room_id: room_id.clone(),
        })
        .unwrap();

        reopened_alice
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "hello direct".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            bob_state
                .messages
                .iter()
                .any(|message| message.text == "hello direct")
        );
    }

    #[test]
    fn app_profile_chat_rejects_ambiguous_direct_rooms_without_changing_selection() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob_account_id = bob.state().unwrap().identity.account_id;

        let first = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Old direct room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        add_runtime_member(&alice, &bob, &first, test_profile(&bob_account_id, "Bob"));
        let second = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Fresh direct room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        add_runtime_member(&alice, &bob, &second, test_profile(&bob_account_id, "Bob"));

        let selected_recovery_room = second;
        alice
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: selected_recovery_room.clone(),
            })
            .unwrap();
        let before = alice.state().unwrap();

        let error = alice
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: test_profile_with_picture(
                    &bob_account_id,
                    "A profile mutation that must not persist",
                    "https://example.invalid/must-not-persist.png",
                ),
                display_name: "Chat with Bob".to_owned(),
            })
            .unwrap_err();
        assert!(matches!(
            error,
            FiniteChatCoreError::Client { reason }
                if reason == "multiple direct Rooms exist for this profile; open one explicitly"
        ));
        let unchanged = alice.state().unwrap();
        assert_eq!(unchanged, before);
        assert_eq!(
            unchanged.selected_room_id.as_deref(),
            Some(selected_recovery_room.as_str())
        );
        assert_eq!(unchanged.rooms.len(), 2);
    }

    #[test]
    fn profile_chat_bootstrap_resumes_only_its_intended_room_and_rejects_any_other_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob_account_id = bob.state().unwrap().identity.account_id;
        bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let intended_room_id = "room-durable-bootstrap-intent".to_owned();

        let created = bootstrap_profile_chat_for_test(
            &alice,
            test_profile(&bob_account_id, "Bob"),
            "Chat with Bob",
            &intended_room_id,
        )
        .unwrap();
        assert_eq!(created.status, "chat created");
        assert_eq!(
            created.selected_room_id.as_deref(),
            Some(intended_room_id.as_str())
        );
        assert_eq!(created.rooms.len(), 1);

        let resumed = bootstrap_profile_chat_for_test(
            &alice,
            test_profile(&bob_account_id, "Bob"),
            "Ignored retry label",
            &intended_room_id,
        )
        .unwrap();
        assert_eq!(resumed.status, "chat bootstrap resumed");
        assert_eq!(
            resumed.selected_room_id.as_deref(),
            Some(intended_room_id.as_str())
        );
        assert_eq!(resumed.rooms.len(), 1);
        bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();

        let error = bootstrap_profile_chat_for_test(
            &alice,
            test_profile(&bob_account_id, "Bob"),
            "Chat with Bob",
            "room-different-bootstrap-intent",
        )
        .unwrap_err();
        assert!(matches!(
            error,
            FiniteChatCoreError::Client { reason }
                if reason == "an existing direct Room prevents first-time profile chat bootstrap"
        ));
        assert_eq!(alice.state().unwrap().rooms.len(), 1);
    }

    #[test]
    fn profile_chat_bootstrap_resumes_its_intended_self_only_room_without_creating_another() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let intended_room_id = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Durable intended Room".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        let bob_account_id = bob
            .dispatch_and_wait(AppAction::StartRuntime)
            .unwrap()
            .identity
            .account_id;

        let completed = bootstrap_profile_chat_for_test(
            &alice,
            test_profile(&bob_account_id, "Bob"),
            "Chat with Bob",
            &intended_room_id,
        )
        .unwrap();
        assert_eq!(completed.status, "chat created");
        assert_eq!(completed.rooms.len(), 1);
        assert_eq!(
            completed.selected_room_id.as_deref(),
            Some(intended_room_id.as_str())
        );
        assert_eq!(
            alice.profile_chat_room_ids(bob_account_id).unwrap(),
            vec![intended_room_id]
        );
    }

    #[test]
    fn profile_chat_bootstrap_fails_closed_when_retained_room_membership_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_data = dir.path().join("alice");
        let seeded_alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_data.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        seeded_alice
            .test_seed_room_state(
                StoredAppRoom {
                    room_id: "room-retained-without-mls".to_owned(),
                    display_name: "Retained Room".to_owned(),
                    picture: None,
                    state: StoredAppRoomState::UnavailableOnDevice,
                    status: LOCAL_ROOM_UNAVAILABLE_STATUS.to_owned(),
                    local_read_seq: 0,
                },
                None,
            )
            .unwrap();
        drop(seeded_alice);

        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_data.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob_account_id = bob.state().unwrap().identity.account_id;
        bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();

        let error = bootstrap_profile_chat_for_test(
            &alice,
            test_profile(&bob_account_id, "Bob"),
            "Chat with Bob",
            "room-authorized-intent",
        )
        .unwrap_err();
        assert!(matches!(
            error,
            FiniteChatCoreError::Client { reason }
                if reason == "cannot prove profile Room absence while a retained Room is unavailable on this Device"
        ));
        let state = alice.state().unwrap();
        assert_eq!(state.rooms.len(), 1);
        assert_eq!(state.rooms[0].state, AppRoomState::UnavailableOnDevice);
    }

    #[test]
    fn app_new_chat_intent_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("app").to_string_lossy().into_owned(),
            server_url,
            device_id: "hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let initial = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Agent".to_owned(),
            })
            .unwrap();
        let room_id = initial.selected_room_id.unwrap();
        let before = initial
            .topics
            .iter()
            .find(|topic| topic.room_id == room_id && topic.topic_id == HOME_TOPIC_ID)
            .unwrap()
            .chats
            .len();
        let first = app
            .start_topic_chat_intent_and_wait(
                room_id.clone(),
                HOME_TOPIC_ID.to_owned(),
                None,
                "browser-intent-1".to_owned(),
            )
            .unwrap();
        let first_chat_id = first.selected_chat_id.clone().unwrap();
        let retried = app
            .start_topic_chat_intent_and_wait(
                room_id.clone(),
                HOME_TOPIC_ID.to_owned(),
                None,
                "browser-intent-1".to_owned(),
            )
            .unwrap();
        assert_eq!(
            retried.selected_chat_id.as_deref(),
            Some(first_chat_id.as_str())
        );
        let chats = retried
            .topics
            .iter()
            .find(|topic| topic.room_id == room_id && topic.topic_id == HOME_TOPIC_ID)
            .unwrap()
            .chats
            .len();
        assert_eq!(chats, before + 1);
    }

    #[test]
    fn app_pairs_one_agent_and_starts_home_chats_with_or_without_a_first_message() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let user_options = with_test_secret(OpenOptions {
            data_dir: dir.path().join("user").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "user-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        });
        let user = FiniteChatRuntime::open(user_options.clone()).unwrap();
        let agent = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("agent").to_string_lossy().into_owned(),
            server_url,
            device_id: "agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let agent_state = agent
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("agent publishes key packages");
        let mut agent_profile = test_profile(&agent_state.identity.account_id, "Agent");
        agent_profile.is_agent = true;

        let connected = user
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: agent_profile,
                display_name: "Agent".to_owned(),
            })
            .expect("user creates an encrypted agent room");
        let room_id = connected
            .selected_room_id
            .clone()
            .expect("agent room is selected");
        assert!(app_room(&connected, &room_id).is_agent_chat);
        let before = connected
            .topics
            .iter()
            .find(|topic| topic.room_id == room_id && topic.topic_id == HOME_TOPIC_ID)
            .expect("agent room has a home topic")
            .chats
            .len();

        let paired = user
            .dispatch_and_wait(AppAction::PairAgent {
                room_id: room_id.clone(),
            })
            .expect("connected agent can be paired");
        assert_eq!(
            paired
                .paired_agent
                .as_ref()
                .map(|paired| paired.canonical_room_id.as_str()),
            Some(room_id.as_str()),
        );
        let paired_again = user
            .dispatch_and_wait(AppAction::PairAgent {
                room_id: room_id.clone(),
            })
            .expect("pairing the same direct agent is idempotent");
        assert_eq!(paired_again.paired_agent, paired.paired_agent);

        let empty_message_error = user
            .dispatch_and_wait(AppAction::StartHomeChat {
                text: Some("   ".to_owned()),
                intent_key: "ios-home-empty-message".to_owned(),
            })
            .expect_err("an explicit first message must not be blank");
        assert!(
            empty_message_error
                .to_string()
                .contains("message must not be empty")
        );

        let created = user
            .dispatch_and_wait(AppAction::StartHomeChat {
                text: None,
                intent_key: "ios-home-attachment-1".to_owned(),
            })
            .expect("home can create an empty chat before sending an attachment");
        let empty_chat_id = created
            .selected_chat_id
            .as_deref()
            .expect("empty chat selected");
        let created_home = created
            .topics
            .iter()
            .find(|topic| topic.room_id == room_id && topic.topic_id == HOME_TOPIC_ID)
            .expect("home topic remains available");
        assert_eq!(created_home.chats.len(), before + 1);
        assert!(!created.messages.iter().any(|message| {
            message.room_id == room_id && message.chat_id.as_deref() == Some(empty_chat_id)
        }));
        let created_again = user
            .dispatch_and_wait(AppAction::StartHomeChat {
                text: None,
                intent_key: "ios-home-attachment-1".to_owned(),
            })
            .expect("retry reuses the empty chat");
        let created_again_home = created_again
            .topics
            .iter()
            .find(|topic| topic.room_id == room_id && topic.topic_id == HOME_TOPIC_ID)
            .unwrap();
        assert_eq!(created_again_home.chats.len(), before + 1);
        assert_eq!(
            created_again.selected_chat_id.as_deref(),
            Some(empty_chat_id)
        );
        let attachment_bytes = b"home attachment".to_vec();
        let with_attachment = user
            .dispatch_and_wait(AppAction::SendChatAttachments {
                room_id: room_id.clone(),
                topic_id: HOME_TOPIC_ID.to_owned(),
                chat_id: empty_chat_id.to_owned(),
                attachments: vec![OutboundAttachment {
                    filename: "home.txt".to_owned(),
                    mime_type: "text/plain".to_owned(),
                    kind: ChatMediaKind::File,
                    bytes: attachment_bytes,
                }],
                caption: "from Home".to_owned(),
                reply_to_message_id: None,
            })
            .expect("the retained encrypted attachment path sends into the created chat");
        assert!(with_attachment.messages.iter().any(|message| {
            message.room_id == room_id
                && message.chat_id.as_deref() == Some(empty_chat_id)
                && message.text == "from Home"
                && message
                    .media
                    .iter()
                    .any(|media| media.filename == "home.txt")
        }));

        let started = user
            .dispatch_and_wait(AppAction::StartHomeChat {
                text: Some("  Help me ship this.  ".to_owned()),
                intent_key: "ios-home-submit-1".to_owned(),
            })
            .expect("home starts a new chat and sends its first message");
        let selected_chat_id = started
            .selected_chat_id
            .as_deref()
            .expect("new chat selected");
        let home = started
            .topics
            .iter()
            .find(|topic| topic.room_id == room_id && topic.topic_id == HOME_TOPIC_ID)
            .expect("home topic remains available");
        assert_eq!(home.chats.len(), before + 2);
        assert!(
            home.chats
                .iter()
                .any(|chat| chat.chat_id == selected_chat_id)
        );
        assert!(started.messages.iter().any(|message| {
            message.room_id == room_id
                && message.chat_id.as_deref() == Some(selected_chat_id)
                && message.text == "Help me ship this."
        }));
        let retried = user
            .dispatch_and_wait(AppAction::StartHomeChat {
                text: Some("Help me ship this.".to_owned()),
                intent_key: "ios-home-submit-1".to_owned(),
            })
            .expect("retry reuses the chat and first message");
        let retried_home = retried
            .topics
            .iter()
            .find(|topic| topic.room_id == room_id && topic.topic_id == HOME_TOPIC_ID)
            .unwrap();
        assert_eq!(retried_home.chats.len(), before + 2);
        assert_eq!(
            retried
                .messages
                .iter()
                .filter(|message| {
                    message.chat_id.as_deref() == Some(selected_chat_id)
                        && message.text == "Help me ship this."
                })
                .count(),
            1
        );

        drop(user);
        let reopened = FiniteChatRuntime::open(user_options)
            .expect("paired app state reopens from encrypted local storage")
            .state()
            .unwrap();
        assert_eq!(
            reopened
                .paired_agent
                .as_ref()
                .map(|paired| paired.canonical_room_id.as_str()),
            Some(room_id.as_str()),
        );
    }

    #[test]
    fn app_refuses_to_pair_a_non_agent_room() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("app").to_string_lossy().into_owned(),
            server_url,
            device_id: "ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let room_id = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Not an agent".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();

        let error = app
            .dispatch_and_wait(AppAction::PairAgent { room_id })
            .expect_err("ordinary rooms are outside the focused iOS product");
        assert!(matches!(
            error,
            FiniteChatCoreError::Client { reason }
                if reason.contains("is not an available")
        ));
    }

    #[test]
    fn app_profile_chat_without_available_key_package_does_not_create_room() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let account_id =
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_owned();

        let state = alice
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: test_profile(&account_id, "Missing"),
                display_name: "Chat with Missing".to_owned(),
            })
            .unwrap();
        assert_eq!(state.status, "chat unavailable");
        assert_eq!(
            state.toast.as_deref(),
            Some("Ask them to open Finite Chat, then try again")
        );
        assert!(state.rooms.is_empty());
        let profile = app_profile(&state, &account_id);
        assert_eq!(profile.display_name, "Missing");
        assert!(profile.stale);
        drop(alice);

        let reopened_alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened_alice.state().unwrap();
        let reopened_profile = app_profile(&reopened_state, &account_id);
        assert_eq!(reopened_profile.display_name, "Missing");
        assert!(reopened_profile.stale);
    }

    #[test]
    fn app_group_chat_claims_key_packages_and_sends_welcomes_via_welcome() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let carol = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("carol").to_string_lossy().into_owned(),
            server_url,
            device_id: "carol-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let bob_account_id = bob.state().unwrap().identity.account_id;
        let carol_account_id = carol.state().unwrap().identity.account_id;
        bob.dispatch_and_wait(AppAction::StartRuntime)
            .expect("bob publishes key packages");
        carol
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("carol publishes key packages");

        let alice_state = alice
            .dispatch_and_wait(AppAction::StartGroupChat {
                profiles: vec![
                    test_profile_with_picture(
                        &bob_account_id,
                        "Bob",
                        "https://example.invalid/bob.png",
                    ),
                    test_profile_with_picture(
                        &carol_account_id,
                        "Carol",
                        "https://example.invalid/carol.png",
                    ),
                ],
                display_name: "Weekend plans".to_owned(),
            })
            .unwrap();
        assert_eq!(alice_state.status, "chat created");
        let room = alice_state.rooms.first().expect("group room");
        assert_eq!(room.display_name, "Weekend plans");
        assert_eq!(room.state, AppRoomState::Connected);
        let room_id = room.room_id.clone();
        let details = alice_state
            .room_details
            .as_ref()
            .expect("group room details");
        assert_member_in_room_details(details, &alice_state.identity.account_id, "alice-ios", true);
        assert_member_in_room_details(details, &bob_account_id, "bob-ios", false);
        assert_member_in_room_details(details, &carol_account_id, "carol-ios", false);
        assert_eq!(
            room_details_member(details, &bob_account_id, "bob-ios")
                .picture
                .as_deref(),
            Some("https://example.invalid/bob.png")
        );
        assert_eq!(
            room_details_member(details, &carol_account_id, "carol-ios")
                .picture
                .as_deref(),
            Some("https://example.invalid/carol.png")
        );

        let direct_state = alice
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: test_profile(&bob_account_id, "Bob"),
                display_name: "Chat with Bob".to_owned(),
            })
            .unwrap();
        assert_eq!(direct_state.status, "chat created");
        assert_eq!(direct_state.rooms.len(), 2);
        let direct_room_id = direct_state
            .selected_room_id
            .as_deref()
            .expect("selected direct room");
        assert_ne!(direct_room_id, room_id);
        assert_eq!(
            app_room(&direct_state, direct_room_id).state,
            AppRoomState::Connected
        );

        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&bob_state, &room_id).state,
            AppRoomState::Connected
        );
        bob.dispatch_and_wait(AppAction::OpenRoom {
            room_id: room_id.clone(),
        })
        .unwrap();

        let carol_state = carol.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&carol_state, &room_id).state,
            AppRoomState::Connected
        );
        carol
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: room_id.clone(),
            })
            .unwrap();

        alice
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "hello group".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            bob_state
                .messages
                .iter()
                .any(|message| message.text == "hello group")
        );
        let carol_state = carol.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            carol_state
                .messages
                .iter()
                .any(|message| message.text == "hello group")
        );
    }

    #[test]
    fn app_group_chat_with_missing_key_package_does_not_create_room() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let bob_account_id = bob.state().unwrap().identity.account_id;
        bob.dispatch_and_wait(AppAction::StartRuntime)
            .expect("bob publishes key packages");
        let missing_account_id =
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_owned();

        let state = alice
            .dispatch_and_wait(AppAction::StartGroupChat {
                profiles: vec![
                    test_profile(&bob_account_id, "Bob"),
                    test_profile(&missing_account_id, "Missing"),
                ],
                display_name: "Broken group".to_owned(),
            })
            .unwrap();
        assert_eq!(state.status, "chat unavailable");
        assert_eq!(
            state.toast.as_deref(),
            Some("Ask everyone to open Finite Chat, then try again")
        );
        assert!(state.rooms.is_empty());
        assert!(app_profile(&state, &missing_account_id).stale);
    }

    #[test]
    fn app_add_room_members_claims_key_package_and_sends_welcome_via_welcome() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let carol = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("carol").to_string_lossy().into_owned(),
            server_url,
            device_id: "carol-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let bob_account_id = bob.state().unwrap().identity.account_id;
        let carol_account_id = carol.state().unwrap().identity.account_id;
        bob.dispatch_and_wait(AppAction::StartRuntime)
            .expect("bob publishes key packages");
        carol
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("carol publishes key packages");

        let alice_state = alice
            .dispatch_and_wait(AppAction::StartProfileChat {
                profile: test_profile(&bob_account_id, "Bob"),
                display_name: "Chat with Bob".to_owned(),
            })
            .unwrap();
        let room_id = alice_state
            .rooms
            .first()
            .expect("direct room")
            .room_id
            .clone();
        bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        bob.dispatch_and_wait(AppAction::OpenRoom {
            room_id: room_id.clone(),
        })
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::AddRoomMembers {
                room_id: room_id.clone(),
                profiles: vec![test_profile(&carol_account_id, "Carol")],
            })
            .unwrap();
        assert_eq!(alice_state.status, "people added");
        assert_eq!(
            app_room(&alice_state, &room_id).state,
            AppRoomState::Connected
        );
        let details = alice_state
            .room_details
            .as_ref()
            .expect("room details after adding member");
        assert_member_in_room_details(details, &alice_state.identity.account_id, "alice-ios", true);
        assert_member_in_room_details(details, &bob_account_id, "bob-ios", false);
        assert_member_in_room_details(details, &carol_account_id, "carol-ios", false);

        let carol_state = carol.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&carol_state, &room_id).state,
            AppRoomState::Connected
        );
        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&bob_state, &room_id).state,
            AppRoomState::Connected
        );
        carol
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: room_id.clone(),
            })
            .unwrap();

        alice
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "hello after add".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            bob_state
                .messages
                .iter()
                .any(|message| message.text == "hello after add")
        );
        let carol_state = carol.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            carol_state
                .messages
                .iter()
                .any(|message| message.text == "hello after add")
        );
    }

    #[test]
    fn app_add_room_members_after_nonjoining_member_still_adds_later_member() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let carol = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("carol").to_string_lossy().into_owned(),
            server_url,
            device_id: "carol-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let bob_account_id = bob.state().unwrap().identity.account_id;
        let carol_account_id = carol.state().unwrap().identity.account_id;
        bob.dispatch_and_wait(AppAction::StartRuntime)
            .expect("bob publishes key packages");
        carol
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("carol publishes key packages");

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Welcome regression".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().expect("room").room_id.clone();

        let alice_state = alice
            .dispatch_and_wait(AppAction::AddRoomMembers {
                room_id: room_id.clone(),
                profiles: vec![test_profile(&bob_account_id, "Bob")],
            })
            .unwrap();
        assert_eq!(alice_state.status, "people added");
        assert_eq!(
            app_room(&alice_state, &room_id).state,
            AppRoomState::Connected
        );

        let alice_state = alice
            .dispatch_and_wait(AppAction::AddRoomMembers {
                room_id: room_id.clone(),
                profiles: vec![test_profile(&carol_account_id, "Carol")],
            })
            .unwrap();
        assert_eq!(alice_state.status, "people added");
        assert_eq!(
            app_room(&alice_state, &room_id).state,
            AppRoomState::Connected
        );

        let carol_state = carol.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(
            app_room(&carol_state, &room_id).state,
            AppRoomState::Connected
        );
        carol
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: room_id.clone(),
            })
            .unwrap();
        carol
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "carol joined after bob stayed offline".to_owned(),
                metadata_json: None,
            })
            .unwrap();

        let alice_state = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            alice_state
                .messages
                .iter()
                .any(|message| message.text == "carol joined after bob stayed offline"),
            "a stale unclaimed Welcome must not block a later add"
        );
    }

    #[test]
    fn app_add_room_members_with_missing_key_package_keeps_existing_room() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Existing room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().expect("room").room_id.clone();
        let missing_account_id =
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_owned();

        let state = alice
            .dispatch_and_wait(AppAction::AddRoomMembers {
                room_id: room_id.clone(),
                profiles: vec![test_profile(&missing_account_id, "Missing")],
            })
            .unwrap();
        assert_eq!(state.status, "chat unavailable");
        assert_eq!(
            state.toast.as_deref(),
            Some("Ask everyone to open Finite Chat, then try again")
        );
        assert_eq!(state.rooms.len(), 1);
        assert_eq!(app_room(&state, &room_id).display_name, "Existing room");
        assert_eq!(app_room(&state, &room_id).state, AppRoomState::Connected);
        assert!(app_profile(&state, &missing_account_id).stale);
    }

    #[test]
    fn app_runtime_projects_typing_as_live_only_state() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob_account_id = bob.state().unwrap().identity.account_id;
        let bob_npub = npub_encode(&bob_account_id).unwrap();
        bob.dispatch_and_wait(AppAction::SaveProfile {
            display_name: "Bob Finite".to_owned(),
            about: String::new(),
            picture: Some("https://example.invalid/bob.png".to_owned()),
        })
        .unwrap();
        alice
            .dispatch_and_wait(AppAction::ScanTarget { value: bob_npub })
            .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Typing Room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        let alice_joined = add_runtime_member(
            &alice,
            &bob,
            &room_id,
            test_profile_with_picture(&bob_account_id, "Bob", "https://example.invalid/bob.png"),
        );
        let bob_member = alice_joined
            .room_details
            .as_ref()
            .and_then(|details| {
                details
                    .members
                    .iter()
                    .find(|member| member.account_id == bob_account_id)
            })
            .expect("bob member summary");
        assert_eq!(
            bob_member.picture.as_deref(),
            Some("https://example.invalid/bob.png")
        );

        bob.dispatch_and_wait(AppAction::SetTyping {
            room_id: room_id.clone(),
            is_typing: true,
        })
        .unwrap();
        let alice_state = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(alice_state.typing_members.len(), 1);
        assert_eq!(alice_state.typing_members[0].room_id, room_id);
        assert_eq!(alice_state.typing_members[0].device_id, "bob-ios");
        assert_eq!(
            alice_state.typing_members[0].activity_kind,
            FINITECHAT_ACTIVITY_KIND_TYPING
        );
        assert_eq!(
            alice_state.typing_members[0].picture.as_deref(),
            Some("https://example.invalid/bob.png")
        );

        drop(alice);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        assert!(
            reopened.state().unwrap().typing_members.is_empty(),
            "ephemeral typing must not be stored in the durable client transcript"
        );
        drop(reopened);

        bob.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "done typing".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        let alice_online = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let alice_state = alice_online
            .dispatch_and_wait(AppAction::StartRuntime)
            .unwrap();
        assert!(
            alice_state.typing_members.is_empty(),
            "durable chat messages clear stale typing from the same sender"
        );
        assert!(
            alice_state
                .messages
                .iter()
                .any(|message| message.text == "done typing")
        );
    }

    #[test]
    fn app_runtime_projects_hermes_working_as_live_indicator_state() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let hermes = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("hermes").to_string_lossy().into_owned(),
            server_url,
            device_id: "hermes-agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Hermes Room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        let topic_id = alice_state.selected_topic_id.clone().unwrap();
        let chat_id = alice_state.selected_chat_id.clone().unwrap();
        add_runtime_member_named(&alice, &hermes, &room_id, "Hermes");

        hermes
            .append_ephemeral_activity_and_wait(AppBridgeActivityInput {
                room_id: room_id.clone(),
                conversation_id: Some(topic_id.clone()),
                segment_id: Some(chat_id.clone()),
                activity_kind: FINITECHAT_ACTIVITY_KIND_WORKING.to_owned(),
                activity_id: None,
                action: EphemeralActivityActionV1::Set,
                payload: br#"{}"#.to_vec(),
                expires_in_millis: 15_000,
            })
            .unwrap();
        let alice_state = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(alice_state.typing_members.len(), 1);
        assert_eq!(alice_state.typing_members[0].room_id, room_id);
        assert_eq!(
            alice_state.typing_members[0].topic_id.as_deref(),
            Some(topic_id.as_str())
        );
        assert_eq!(
            alice_state.typing_members[0].chat_id.as_deref(),
            Some(chat_id.as_str())
        );
        assert_eq!(alice_state.typing_members[0].device_id, "hermes-agent");
        assert_eq!(
            alice_state.typing_members[0].activity_kind,
            FINITECHAT_ACTIVITY_KIND_WORKING
        );

        hermes
            .append_ephemeral_activity_and_wait(AppBridgeActivityInput {
                room_id: room_id.clone(),
                conversation_id: Some(topic_id),
                segment_id: Some(chat_id),
                activity_kind: FINITECHAT_ACTIVITY_KIND_WORKING.to_owned(),
                activity_id: None,
                action: EphemeralActivityActionV1::Clear,
                payload: Vec::new(),
                expires_in_millis: 15_000,
            })
            .unwrap();
        let alice_state = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            alice_state.typing_members.is_empty(),
            "Hermes working clears should remove the live indicator"
        );
    }

    #[test]
    fn app_start_runtime_returns_durable_chat_when_delivery_is_offline() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Local First".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        alice
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "saved before force close".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        drop(alice);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let local_snapshot = reopened.state().unwrap();
        assert_eq!(
            app_room(&local_snapshot, &room_id).display_name,
            "Local First"
        );
        assert_eq!(
            local_snapshot.selected_room_id.as_deref(),
            Some(room_id.as_str())
        );
        assert_eq!(
            app_room(&local_snapshot, &room_id).last_message_preview,
            "saved before force close"
        );
        let reopened_message = local_snapshot
            .messages
            .iter()
            .find(|message| message.text == "saved before force close")
            .expect("force-close reopen must render the durable local transcript before sync");
        assert_eq!(reopened_message.timestamp_unix_seconds, NOW);
        assert!(!reopened_message.display_timestamp.is_empty());

        let started = reopened.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(started.status, "offline");
        assert_eq!(
            started.toast.as_deref(),
            Some("Showing saved chats. Connection will retry.")
        );
        let started_message = started
            .messages
            .iter()
            .find(|message| message.text == "saved before force close")
            .expect("startup sync failure must not hide the durable local transcript");
        assert_eq!(started_message.timestamp_unix_seconds, NOW);
        assert!(!started_message.display_timestamp.is_empty());
        assert_eq!(app_room(&started, &room_id).state, AppRoomState::Connected);
    }

    #[test]
    fn app_offline_text_send_persists_undelivered_outbox_across_force_close() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Local Outbox".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        drop(alice);

        let offline = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let failed = offline
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "do not lose this".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        assert_eq!(failed.status, "sent");
        assert_eq!(failed.toast, None);
        let failed_message = failed
            .messages
            .iter()
            .find(|message| message.text == "do not lose this")
            .expect("undelivered local message projects immediately");
        assert_outbound_undelivered(failed_message);
        let local_message_id = failed_message.message_id.clone();
        drop(offline);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let local_snapshot = reopened.state().unwrap();
        let reopened_message = local_snapshot
            .messages
            .iter()
            .find(|message| message.text == "do not lose this")
            .expect("undelivered local message survives force-close reopen");
        assert_eq!(reopened_message.message_id, local_message_id);
        assert_outbound_undelivered(reopened_message);
        let outbox_rows = reopened.app_outbox_debug_rows().unwrap();
        assert_eq!(outbox_rows.len(), 1);
        let outbox_row = &outbox_rows[0];
        assert_eq!(outbox_row.room_id, room_id);
        assert_eq!(outbox_row.message_id, local_message_id);
        assert_eq!(outbox_row.sender_device_id, "alice-ios");
        assert_eq!(outbox_row.local_state, "sent");
        assert_eq!(outbox_row.server_delivery_state, "undelivered");
        assert_eq!(outbox_row.append_request_room_id, outbox_row.room_id);
        assert_eq!(outbox_row.append_request_message_id, outbox_row.message_id);
        assert_eq!(
            outbox_row.append_request_sender_device_id,
            outbox_row.sender_device_id
        );
        assert!(
            outbox_row.idempotency_key_present,
            "durable outbox row should retain retry idempotency material"
        );
        assert_eq!(
            app_room(&local_snapshot, &room_id).last_message_preview,
            "do not lose this"
        );
    }

    #[test]
    fn app_offline_text_send_auto_drains_after_force_close_without_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Retry Outbox".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        drop(alice);

        let offline = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let failed = offline
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "retry after force close".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let local_message_id = failed
            .messages
            .iter()
            .find(|message| message.text == "retry after force close")
            .expect("undelivered local message projects immediately")
            .message_id
            .clone();
        let stale_outbox_row = runtime_outbox(&offline)
            .into_iter()
            .find(|message| message.message_id == local_message_id)
            .expect("offline send should persist the outbox row");
        drop(offline);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let before_retry = reopened.state().unwrap();
        let before_message = before_retry
            .messages
            .iter()
            .find(|message| message.message_id == local_message_id)
            .expect("undelivered row should be visible before drain");
        assert_outbound_undelivered(before_message);

        let drained = reopened.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let accepted_messages = drained
            .messages
            .iter()
            .filter(|message| message.text == "retry after force close")
            .collect::<Vec<_>>();
        assert_eq!(accepted_messages.len(), 1);
        let accepted = accepted_messages[0];
        assert_eq!(accepted.message_id, local_message_id);
        assert_outbound_delivered(accepted);
        assert!(
            runtime_outbox(&reopened).is_empty(),
            "successful drain removes the exact undelivered outbox row"
        );
        reopened
            .test_save_outbox(vec![stale_outbox_row.clone()])
            .unwrap();
        assert_eq!(
            runtime_outbox(&reopened).len(),
            1,
            "test setup should recreate the stale accepted outbox row observed in the demo"
        );
        drop(reopened);

        let persisted_runtime = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let persisted = persisted_runtime.state().unwrap();
        let persisted_message = persisted
            .messages
            .iter()
            .find(|message| message.text == "retry after force close")
            .expect("accepted retry survives force-close reopen");
        assert_eq!(persisted_message.message_id, local_message_id);
        assert_outbound_delivered(persisted_message);
        assert!(
            runtime_outbox(&persisted_runtime).is_empty(),
            "force-close reopen deletes stale outbox rows once the same local message id is delivered"
        );
    }

    #[test]
    fn app_server_rejected_text_send_requires_explicit_retry_with_same_outbox_identity() {
        let dir = tempfile::tempdir().unwrap();
        let original_server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let empty_server_url = spawn_live_http_server(dir.path().join("empty-server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: original_server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Rejected Send".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        drop(alice);

        let rejected_runtime = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: empty_server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let rejected = rejected_runtime
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "retry only after server rejection".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        assert_eq!(rejected.status, "delivery failed");
        assert_eq!(
            rejected.toast.as_deref(),
            Some("Message delivery failed. Retry when ready.")
        );
        let rejected_message = rejected
            .messages
            .iter()
            .find(|message| message.text == "retry only after server rejection")
            .expect("server-rejected local message stays visible");
        assert_outbound_failed_contains(rejected_message, "room_membership_conflict");
        let local_message_id = rejected_message.message_id.clone();
        let failed_outbox = runtime_outbox(&rejected_runtime);
        assert_eq!(failed_outbox.len(), 1);
        let failed_row = failed_outbox[0].clone();
        assert_eq!(failed_row.room_id, room_id);
        assert_eq!(failed_row.message_id, local_message_id);
        assert_eq!(failed_row.local_state, StoredOutboundLocalState::Sent);
        assert!(matches!(
            failed_row.server_delivery_state,
            StoredOutboundServerDeliveryState::Failed { .. }
        ));
        let retry_idempotency_key = failed_row.append_request.idempotency_key.clone();
        let retry_append_request = failed_row.append_request.clone();
        assert_eq!(
            application_effect(&original_server_url, &local_message_id),
            None,
            "rejection on another configured server must not create an app effect on the original server"
        );
        drop(rejected_runtime);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: original_server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let before_retry = reopened.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let still_failed = before_retry
            .messages
            .iter()
            .find(|message| message.message_id == local_message_id)
            .expect("failed message survives force close");
        assert_outbound_failed_contains(still_failed, "room_membership_conflict");
        let failed_after_start = runtime_outbox(&reopened);
        assert_eq!(failed_after_start.len(), 1);
        assert_eq!(failed_after_start[0].append_request, retry_append_request);
        assert_eq!(
            failed_after_start[0].append_request.idempotency_key,
            retry_idempotency_key
        );
        assert_eq!(
            application_effect(&original_server_url, &local_message_id),
            None,
            "failed rows are excluded from automatic outbox drain"
        );

        let retried = reopened
            .dispatch_and_wait(AppAction::RetryMessage {
                room_id: room_id.clone(),
                message_id: local_message_id.clone(),
            })
            .unwrap();
        let delivered = retried
            .messages
            .iter()
            .find(|message| message.message_id == local_message_id)
            .expect("retry reuses the original visible bubble");
        assert_eq!(delivered.text, "retry only after server rejection");
        assert_outbound_delivered(delivered);
        assert!(
            runtime_outbox(&reopened).is_empty(),
            "successful retry removes the exact failed outbox row"
        );
        let effect = application_effect(&original_server_url, &local_message_id)
            .expect("retry creates one application delivery effect");
        assert_eq!(effect.room_id, room_id);
        assert_eq!(effect.message_id, local_message_id);
        assert_eq!(effect.sender.device_id, "alice-ios");
    }

    #[test]
    fn app_offline_attachment_send_fails_fast_without_outbox_or_bubble() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Offline Attachment Fail Fast".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        drop(alice);

        let caption = "offline media should not send";
        let plaintext = b"offline media must not create an outbox row".to_vec();
        let offline = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let failed = offline
            .dispatch_and_wait(AppAction::SendAttachment {
                room_id: room_id.clone(),
                filename: "offline-photo.jpg".to_owned(),
                mime_type: "image/jpeg".to_owned(),
                kind: ChatMediaKind::Image,
                bytes: plaintext.clone(),
                caption: caption.to_owned(),
                reply_to_message_id: None,
            })
            .unwrap();
        assert_eq!(failed.status, "attachment unavailable");
        assert!(
            failed
                .toast
                .as_deref()
                .is_some_and(|toast| { toast.starts_with("Attachment upload failed:") })
        );
        assert!(
            failed
                .messages
                .iter()
                .all(|message| message.text != caption)
        );
        assert_eq!(app_room(&failed, &room_id).last_message_preview, "");
        assert_eq!(app_room(&failed, &room_id).state, AppRoomState::Connected);
        assert!(
            runtime_outbox(&offline).is_empty(),
            "unreachable attachment upload must not create a durable outbox row"
        );
        drop(offline);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let local_snapshot = reopened.state().unwrap();
        assert_eq!(
            local_snapshot
                .messages
                .iter()
                .filter(|message| message.text == caption)
                .count(),
            0
        );
        assert_eq!(
            app_room(&local_snapshot, &room_id).state,
            AppRoomState::Connected
        );
        assert!(
            runtime_outbox(&reopened).is_empty(),
            "attachment fail-fast must survive force-close without durable outbox rows"
        );
    }

    #[test]
    fn app_reopens_last_selected_room_before_network_sync() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alpha = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Alpha".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        let zulu = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Zulu".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        assert_ne!(alpha, zulu);
        alice
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: zulu.clone(),
                text: "selected room survives force close".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        alice
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: zulu.clone(),
            })
            .unwrap();
        drop(alice);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let local_snapshot = reopened.state().unwrap();

        assert_eq!(
            local_snapshot.selected_room_id.as_deref(),
            Some(zulu.as_str())
        );
        assert!(
            local_snapshot
                .messages
                .iter()
                .any(|message| message.room_id == zulu
                    && message.text == "selected room survives force close"),
            "force-close reopen must restore the last selected room transcript before sync"
        );
        assert_eq!(app_room(&local_snapshot, &alpha).display_name, "Alpha");
    }

    #[test]
    fn app_reopens_unique_local_device_when_requested_device_id_is_stale() {
        // Stored-device recovery only runs on the shared-identity (`None`)
        // acquisition path: an explicit secret keeps the requested device id.
        ensure_test_finite_home();
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let data_dir = dir.path().join("stable-app-store");
        let app = FiniteChatRuntime::open(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "qt433".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        })
        .unwrap();

        let state = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Recovered".to_owned(),
            })
            .unwrap();
        let room_id = state.rooms.first().unwrap().room_id.clone();
        app.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "still here after stale config".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        drop(app);

        let reopened = FiniteChatRuntime::open(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "codex-persist-check".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        })
        .unwrap();
        let local_snapshot = reopened.state().unwrap();

        assert_eq!(local_snapshot.identity.device_id, "qt433");
        assert_eq!(
            app_room(&local_snapshot, &room_id).display_name,
            "Recovered"
        );
        assert!(
            local_snapshot
                .messages
                .iter()
                .any(|message| message.text == "still here after stale config"),
            "stale launch config must recover the durable local transcript before sync"
        );

        let started = reopened.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(started.status, "offline");
        assert!(
            started
                .messages
                .iter()
                .any(|message| message.text == "still here after stale config"),
            "offline startup after stale config recovery must keep the transcript visible"
        );
    }

    #[test]
    fn app_reopens_synced_peer_chat_offline_after_force_close() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let bob_dir = dir.path().join("bob");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: bob_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Force Close".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");

        bob.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "remote message before force close".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        let synced = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            synced
                .messages
                .iter()
                .any(|message| message.text == "remote message before force close")
        );
        drop(alice);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let local_snapshot = reopened.state().unwrap();
        assert_eq!(
            app_room(&local_snapshot, &room_id).display_name,
            "Force Close"
        );
        assert_eq!(
            app_room(&local_snapshot, &room_id).last_message_preview,
            "remote message before force close"
        );
        assert!(
            local_snapshot
                .messages
                .iter()
                .any(|message| message.text == "remote message before force close"),
            "force-close reopen must render synced peer messages from local SQLite before sync"
        );

        let started = reopened.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(started.status, "offline");
        assert!(
            started
                .messages
                .iter()
                .any(|message| message.text == "remote message before force close"),
            "offline startup must not clear a synced peer transcript"
        );
    }

    #[test]
    fn app_runtime_sends_reply_message_with_durable_target() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let created = app
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Replies".to_owned(),
            })
            .unwrap();
        let room_id = created.rooms.first().unwrap().room_id.clone();
        let parent_state = app
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "parent".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let parent_id = parent_state
            .messages
            .iter()
            .find(|message| message.text == "parent")
            .expect("parent message projects")
            .message_id
            .clone();

        let missing = app
            .dispatch_and_wait(AppAction::SendReply {
                room_id: room_id.clone(),
                text: "nope".to_owned(),
                reply_to_message_id: "missing-message".to_owned(),
                metadata_json: None,
            })
            .expect_err("unknown reply targets are rejected by Rust policy");
        assert!(
            missing.to_string().contains("reply target"),
            "unexpected missing-target error: {missing}"
        );

        let replied = app
            .dispatch_and_wait(AppAction::SendReply {
                room_id: room_id.clone(),
                text: "child".to_owned(),
                reply_to_message_id: parent_id.clone(),
                metadata_json: None,
            })
            .unwrap();
        let reply = replied
            .messages
            .iter()
            .find(|message| message.text == "child")
            .expect("reply message projects");
        assert_eq!(
            reply.reply_to_message_id.as_deref(),
            Some(parent_id.as_str())
        );

        let DecodedAppEvent::ChatMessage(decoded) = decode_application_event(&reply.payload) else {
            panic!("reply row must carry a chat message application event");
        };
        let ChatMessagePayload::Hermes(hermes) = decoded.payload else {
            panic!("reply row must carry Hermes message payload");
        };
        assert_eq!(
            hermes.reply_to_message_id.as_deref(),
            Some(parent_id.as_str())
        );

        let media_replied = app
            .dispatch_and_wait(AppAction::SendAttachment {
                room_id: room_id.clone(),
                filename: "reply-photo.jpg".to_owned(),
                mime_type: "image/jpeg".to_owned(),
                kind: ChatMediaKind::Image,
                bytes: b"reply image bytes".to_vec(),
                caption: "media child".to_owned(),
                reply_to_message_id: Some(parent_id.clone()),
            })
            .unwrap();
        let media_reply = media_replied
            .messages
            .iter()
            .find(|message| message.text == "media child" && !message.media.is_empty())
            .expect("media reply message projects");
        assert_eq!(
            media_reply.reply_to_message_id.as_deref(),
            Some(parent_id.as_str())
        );

        let DecodedAppEvent::ChatMessage(decoded) = decode_application_event(&media_reply.payload)
        else {
            panic!("media reply row must carry a chat message application event");
        };
        let ChatMessagePayload::Hermes(hermes) = decoded.payload else {
            panic!("media reply row must carry Hermes message payload");
        };
        assert_eq!(
            hermes.reply_to_message_id.as_deref(),
            Some(parent_id.as_str())
        );

        drop(app);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        let reopened_reply = reopened_state
            .messages
            .iter()
            .find(|message| message.text == "child")
            .expect("reply projection survives reopen");
        assert_eq!(
            reopened_reply.reply_to_message_id.as_deref(),
            Some(parent_id.as_str())
        );
        let reopened_media_reply = reopened_state
            .messages
            .iter()
            .find(|message| message.text == "media child" && !message.media.is_empty())
            .expect("media reply projection survives reopen");
        assert_eq!(
            reopened_media_reply.reply_to_message_id.as_deref(),
            Some(parent_id.as_str())
        );
    }

    #[test]
    fn app_runtime_sends_encrypted_attachment_blob_and_reopens_projection() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Media Room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        let plaintext = b"fake jpeg plaintext bytes".to_vec();

        let sent = alice
            .dispatch_and_wait(AppAction::SendAttachment {
                room_id: room_id.clone(),
                filename: "photo.jpg".to_owned(),
                mime_type: "image/jpeg".to_owned(),
                kind: ChatMediaKind::Image,
                bytes: plaintext.clone(),
                caption: String::new(),
                reply_to_message_id: None,
            })
            .unwrap();
        let message = sent
            .messages
            .iter()
            .find(|message| message.room_id == room_id && !message.media.is_empty())
            .expect("attachment message projects");
        assert_eq!(message.text, "");
        assert_eq!(message.media.len(), 1);
        let media = &message.media[0];
        assert_eq!(media.kind, ChatMediaKind::Image);
        assert_eq!(media.filename, "photo.jpg");
        assert_eq!(media.mime_type, "image/jpeg");
        assert_eq!(media.upload_progress_per_mille, None);
        let local_path = media
            .local_path
            .as_ref()
            .expect("sender caches uploaded attachment plaintext");
        assert_eq!(std::fs::read(local_path).unwrap(), plaintext);
        assert_eq!(app_room(&sent, &room_id).last_message_preview, "photo.jpg");

        let reference = attachment_reference_from_message(message);
        let url = media.url.as_ref().expect("projected blob URL");
        let ciphertext = reqwest::blocking::Client::new()
            .get(url)
            .send()
            .unwrap()
            .bytes()
            .unwrap();
        assert_ne!(ciphertext.as_ref(), plaintext.as_slice());
        let downloaded = finitechat_blob::finish_blossom_download_http_response(
            &reference,
            finitechat_blob::BlossomDownloadHttpResponse {
                status: 200,
                body: ciphertext.as_ref(),
            },
        )
        .unwrap();
        assert_eq!(downloaded.plaintext, plaintext);

        drop(alice);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        let reopened_message = reopened_state
            .messages
            .iter()
            .find(|message| message.room_id == room_id && !message.media.is_empty())
            .expect("attachment projection survives reopen");
        assert_eq!(
            reopened_state
                .messages
                .iter()
                .filter(|message| message.room_id == room_id && !message.media.is_empty())
                .count(),
            1
        );
        assert_eq!(reopened_message.media[0].filename, "photo.jpg");
        assert!(reopened_message.media[0].local_path.is_some());
        assert_eq!(reopened_message.media[0].upload_progress_per_mille, None);
        assert_eq!(
            app_room(&reopened_state, &room_id).last_message_preview,
            "photo.jpg"
        );
    }

    #[test]
    fn app_runtime_sends_multiple_attachments_as_one_durable_media_message() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Batch Media".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();

        let sent = alice
            .dispatch_and_wait(AppAction::SendAttachments {
                room_id: room_id.clone(),
                attachments: vec![
                    OutboundAttachment {
                        filename: "photo-a.jpg".to_owned(),
                        mime_type: "image/jpeg".to_owned(),
                        kind: ChatMediaKind::Image,
                        bytes: b"first fake jpeg".to_vec(),
                    },
                    OutboundAttachment {
                        filename: "clip-b.mov".to_owned(),
                        mime_type: "video/quicktime".to_owned(),
                        kind: ChatMediaKind::Video,
                        bytes: b"second fake movie".to_vec(),
                    },
                ],
                caption: "two files, one message".to_owned(),
                reply_to_message_id: None,
            })
            .unwrap();
        let message = sent
            .messages
            .iter()
            .find(|message| message.text == "two files, one message")
            .expect("batch media message projects");
        assert_eq!(message.media.len(), 2);
        assert_eq!(message.media[0].filename, "photo-a.jpg");
        assert_eq!(message.media[0].kind, ChatMediaKind::Image);
        assert_eq!(message.media[1].filename, "clip-b.mov");
        assert_eq!(message.media[1].kind, ChatMediaKind::Video);
        assert!(message.media.iter().all(|media| media.local_path.is_some()));

        let DecodedAppEvent::ChatMessage(decoded) = decode_application_event(&message.payload)
        else {
            panic!("batch media row must carry a chat message application event");
        };
        let ChatMessagePayload::Hermes(hermes) = decoded.payload else {
            panic!("batch media row must carry Hermes message payload");
        };
        assert_eq!(hermes.attachments.len(), 2);
        assert!(
            hermes
                .attachments
                .iter()
                .all(|attachment| attachment.blob.is_some())
        );

        drop(alice);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        let reopened_message = reopened_state
            .messages
            .iter()
            .find(|message| message.text == "two files, one message")
            .expect("batch media projection survives reopen");
        assert_eq!(reopened_message.media.len(), 2);
        assert!(
            reopened_message
                .media
                .iter()
                .all(|media| media.local_path.is_some())
        );
    }

    #[test]
    fn app_runtime_sends_voice_note_as_durable_audio_attachment() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Voice Notes".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();

        let voice_bytes = b"fake m4a voice note".to_vec();
        let sent = alice
            .dispatch_and_wait(AppAction::SendAttachment {
                room_id: room_id.clone(),
                filename: "voice_1725000123.m4a".to_owned(),
                mime_type: "audio/mp4".to_owned(),
                kind: ChatMediaKind::VoiceNote,
                bytes: voice_bytes,
                caption: "voice caption".to_owned(),
                reply_to_message_id: None,
            })
            .unwrap();
        let message = sent
            .messages
            .iter()
            .find(|message| message.text == "voice caption")
            .expect("voice media message projects");
        assert_eq!(message.media.len(), 1);
        assert_eq!(message.media[0].filename, "voice_1725000123.m4a");
        assert_eq!(message.media[0].mime_type, "audio/mp4");
        assert_eq!(message.media[0].kind, ChatMediaKind::VoiceNote);
        assert!(message.media[0].local_path.is_some());

        let DecodedAppEvent::ChatMessage(decoded) = decode_application_event(&message.payload)
        else {
            panic!("voice media row must carry a chat message application event");
        };
        let ChatMessagePayload::Hermes(hermes) = decoded.payload else {
            panic!("voice media row must carry Hermes message payload");
        };
        assert_eq!(hermes.attachments.len(), 1);
        assert_eq!(hermes.attachments[0].kind, HermesAttachmentKindV1::Audio);
        let blob = hermes.attachments[0]
            .blob
            .as_ref()
            .expect("voice media must use encrypted blob reference");
        assert_eq!(blob.metadata.filename, "voice_1725000123.m4a");
        assert_eq!(blob.metadata.mime_type, "audio/mp4");

        drop(alice);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        let reopened_message = reopened_state
            .messages
            .iter()
            .find(|message| message.text == "voice caption")
            .expect("voice projection survives offline reopen");
        assert_eq!(reopened_message.media[0].kind, ChatMediaKind::VoiceNote);
        assert!(reopened_message.media[0].local_path.is_some());
    }

    #[test]
    fn app_runtime_downloads_attachment_blob_to_verified_local_cache() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let bob_dir = dir.path().join("bob");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: bob_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Media Download".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");

        let plaintext = b"download me after sync".to_vec();
        alice
            .dispatch_and_wait(AppAction::SendAttachment {
                room_id: room_id.clone(),
                filename: "remote photo.jpg".to_owned(),
                mime_type: "image/jpeg".to_owned(),
                kind: ChatMediaKind::Image,
                bytes: plaintext.clone(),
                caption: "from alice".to_owned(),
                reply_to_message_id: None,
            })
            .unwrap();

        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let message = bob_state
            .messages
            .iter()
            .find(|message| message.room_id == room_id && !message.media.is_empty())
            .expect("receiver sees remote attachment");
        assert_eq!(message.text, "from alice");
        let attachment = message.media.first().unwrap();
        assert_eq!(attachment.filename, "remote photo.jpg");
        assert_eq!(attachment.local_path, None);
        assert_eq!(attachment.download_progress_per_mille, None);
        let message_id = message.message_id.clone();
        let attachment_id = attachment.attachment_id.clone();

        drop(bob);
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: bob_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_before_download = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let cache_miss_message = reopened_before_download
            .messages
            .iter()
            .find(|message| message.message_id == message_id)
            .expect("reopened receiver keeps delivered attachment projected");
        assert_eq!(cache_miss_message.media[0].local_path, None);
        assert_eq!(
            cache_miss_message.media[0].download_progress_per_mille,
            None
        );
        assert_eq!(cache_miss_message.outbound_delivery, None);

        let downloading_state = bob
            .dispatch_and_wait(AppAction::BeginDownloadAttachment {
                room_id: room_id.clone(),
                message_id: message_id.clone(),
                attachment_id: attachment_id.clone(),
            })
            .unwrap();
        let downloading_message = downloading_state
            .messages
            .iter()
            .find(|message| message.room_id == room_id && !message.media.is_empty())
            .expect("beginning a download keeps the attachment projected");
        assert_eq!(
            downloading_message.media[0].download_progress_per_mille,
            Some(0)
        );
        assert_eq!(downloading_message.media[0].local_path, None);

        let bob_state = bob
            .dispatch_and_wait(AppAction::DownloadAttachment {
                room_id: room_id.clone(),
                message_id,
                attachment_id,
            })
            .unwrap();
        let downloaded = bob_state
            .messages
            .iter()
            .find(|message| message.room_id == room_id && !message.media.is_empty())
            .expect("downloaded message remains projected");
        let local_path = downloaded.media[0]
            .local_path
            .as_ref()
            .expect("downloaded attachment projects verified local path");
        assert_eq!(downloaded.media[0].download_progress_per_mille, None);
        assert!(local_path.ends_with("remote_photo.jpg"));
        assert_eq!(std::fs::read(local_path).unwrap(), plaintext);

        drop(bob);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: bob_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        let reopened_message = reopened_state
            .messages
            .iter()
            .find(|message| message.room_id == room_id && !message.media.is_empty())
            .expect("cached attachment projection survives offline reopen");
        assert_eq!(
            std::fs::read(reopened_message.media[0].local_path.as_ref().unwrap()).unwrap(),
            plaintext
        );
    }

    #[test]
    fn app_runtime_media_gallery_is_all_history_and_downloads_outside_transcript_window() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let bob_dir = dir.path().join("bob");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: bob_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Gallery Window".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");

        let plaintext = b"old image outside visible transcript".to_vec();
        bob.dispatch_and_wait(AppAction::SendAttachment {
            room_id: room_id.clone(),
            filename: "old-photo.jpg".to_owned(),
            mime_type: "image/jpeg".to_owned(),
            kind: ChatMediaKind::Image,
            bytes: plaintext.clone(),
            caption: "old media".to_owned(),
            reply_to_message_id: None,
        })
        .unwrap();
        for index in 0..55 {
            bob.dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: format!("filler {index}"),
                metadata_json: None,
            })
            .unwrap();
        }

        let synced = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(synced.messages.len(), DEFAULT_TRANSCRIPT_WINDOW);
        assert!(
            synced
                .messages
                .iter()
                .all(|message| message.media.is_empty()),
            "the media message should be older than the selected transcript window"
        );
        let gallery = synced
            .media_gallery
            .as_ref()
            .expect("selected room must project a media gallery");
        assert_eq!(gallery.room_id, room_id);
        assert_eq!(gallery.items.len(), 1);
        let item = &gallery.items[0];
        assert_eq!(item.attachment.filename, "old-photo.jpg");
        assert_eq!(item.attachment.kind, ChatMediaKind::Image);
        assert_eq!(item.attachment.local_path, None);
        assert_eq!(
            item.item_id,
            format!("{}|{}|{}", room_id, item.message_id, item.attachment_id)
        );
        let details = synced
            .room_details
            .as_ref()
            .expect("selected room must project details");
        assert_eq!(details.room_id, room_id);
        assert_eq!(details.display_name, "Gallery Window");
        assert_eq!(details.user_status_text, "Connected");
        assert_eq!(details.media_item_count, 1);

        let downloaded = alice
            .dispatch_and_wait(AppAction::DownloadAttachment {
                room_id: room_id.clone(),
                message_id: item.message_id.clone(),
                attachment_id: item.attachment_id.clone(),
            })
            .unwrap();
        assert!(
            downloaded
                .messages
                .iter()
                .all(|message| message.media.is_empty()),
            "downloading old gallery media must not force-expand the transcript window"
        );
        let downloaded_item = downloaded
            .media_gallery
            .as_ref()
            .and_then(|gallery| gallery.items.first())
            .expect("gallery remains projected after download");
        let local_path = downloaded_item
            .attachment
            .local_path
            .as_ref()
            .expect("downloaded gallery item projects verified local path");
        assert_eq!(std::fs::read(local_path).unwrap(), plaintext);

        drop(alice);
        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let reopened_state = reopened.state().unwrap();
        assert_eq!(reopened_state.messages.len(), DEFAULT_TRANSCRIPT_WINDOW);
        assert!(
            reopened_state
                .messages
                .iter()
                .all(|message| message.media.is_empty())
        );
        let reopened_item = reopened_state
            .media_gallery
            .as_ref()
            .and_then(|gallery| gallery.items.first())
            .expect("gallery survives offline reopen from client SQLite projection");
        assert_eq!(reopened_item.attachment.filename, "old-photo.jpg");
        assert_eq!(
            std::fs::read(reopened_item.attachment.local_path.as_ref().unwrap()).unwrap(),
            plaintext
        );
        let reopened_details = reopened_state
            .room_details
            .as_ref()
            .expect("room details survive offline reopen");
        assert_eq!(reopened_details.room_id, room_id);
        assert_eq!(reopened_details.media_item_count, 1);
    }

    #[test]
    fn app_runtime_wait_for_update_uses_sse_hints_for_messages() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Hint Room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");

        bob.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "hello over app sse".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        let alice_state = alice.wait_for_update(1_000).unwrap();
        assert!(
            alice_state
                .messages
                .iter()
                .any(|message| message.text == "hello over app sse"),
            "receiver should sync the message after the room high-watermark hint"
        );
    }

    #[test]
    fn app_runtime_persists_remote_synced_message_for_force_close_relaunch() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Remote Persistence".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");

        bob.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "remote sync survives force close".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        let alice_synced = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            alice_synced
                .messages
                .iter()
                .any(|message| message.room_id == room_id
                    && message.text == "remote sync survives force close"),
            "receiver must render the synced remote message before relaunch"
        );
        drop(alice);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let local_snapshot = reopened.state().unwrap();
        assert_eq!(
            local_snapshot.selected_room_id.as_deref(),
            Some(room_id.as_str())
        );
        assert_eq!(
            app_room(&local_snapshot, &room_id).last_message_preview,
            "remote sync survives force close"
        );
        assert!(
            local_snapshot
                .messages
                .iter()
                .any(|message| message.room_id == room_id
                    && message.text == "remote sync survives force close"),
            "force-close reopen must restore remote synced messages from local SQLite before sync"
        );

        let offline = reopened.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(offline.status, "offline");
        assert!(
            offline
                .messages
                .iter()
                .any(|message| message.room_id == room_id
                    && message.text == "remote sync survives force close"),
            "offline startup must not hide locally persisted synced messages"
        );
    }

    #[test]
    fn app_runtime_polls_are_durable_and_votes_are_non_notifying() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let bob_dir = dir.path().join("bob");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: bob_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Poll Room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");

        let bob_state = bob
            .dispatch_and_wait(AppAction::SendPoll {
                room_id: room_id.clone(),
                question: "Lunch?".to_owned(),
                options: vec!["Tacos".to_owned(), "Sushi".to_owned()],
            })
            .unwrap();
        let poll_message_id = bob_state
            .messages
            .iter()
            .find(|message| message.poll.is_some())
            .expect("poll message projects")
            .message_id
            .clone();
        assert_eq!(
            app_room(&bob_state, &room_id).last_message_preview,
            "Lunch?"
        );

        let alice_state = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_poll(
            &alice_state,
            &poll_message_id,
            "Lunch?",
            "option-2",
            0,
            0,
            false,
        );
        let alice_state = alice
            .dispatch_and_wait(AppAction::VotePoll {
                room_id: room_id.clone(),
                message_id: poll_message_id.clone(),
                option_id: "option-2".to_owned(),
            })
            .unwrap();
        assert_poll(
            &alice_state,
            &poll_message_id,
            "Lunch?",
            "option-2",
            1,
            1,
            true,
        );

        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_poll(
            &bob_state,
            &poll_message_id,
            "Lunch?",
            "option-2",
            1,
            1,
            false,
        );
        assert_eq!(
            app_room(&bob_state, &room_id).unread_count,
            0,
            "poll votes are durable but must not create unread chat"
        );
        drop(bob);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: bob_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        assert_poll(
            &reopened.state().unwrap(),
            &poll_message_id,
            "Lunch?",
            "option-2",
            1,
            1,
            false,
        );
    }

    #[test]
    fn app_runtime_reactions_are_durable_and_live_projected() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Reaction Room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");

        let bob_state = bob
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "tap a reaction on this".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let target_message_id = bob_state
            .messages
            .iter()
            .find(|message| message.text == "tap a reaction on this")
            .expect("sent message projects")
            .message_id
            .clone();

        alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let alice_state = alice
            .dispatch_and_wait(AppAction::ReactToMessage {
                room_id: room_id.clone(),
                message_id: target_message_id.clone(),
                emoji: "👍".to_owned(),
            })
            .unwrap();
        assert_reaction(&alice_state, &target_message_id, "👍", 1, true);

        let alice_state = alice
            .dispatch_and_wait(AppAction::ReactToMessage {
                room_id: room_id.clone(),
                message_id: target_message_id.clone(),
                emoji: "👍".to_owned(),
            })
            .unwrap();
        assert_reaction(&alice_state, &target_message_id, "👍", 1, true);

        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_reaction(&bob_state, &target_message_id, "👍", 1, false);
        drop(alice);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        assert_reaction(
            &reopened.state().unwrap(),
            &target_message_id,
            "👍",
            1,
            true,
        );
    }

    #[test]
    fn app_runtime_actions_apply_every_event_consumed_by_their_internal_sync() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_options = with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        });
        let alice_runtime = FiniteChatRuntime::open(alice_options.clone()).unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("bob").to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let room_id = alice_runtime
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Complete Sync Delta".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        add_runtime_member_named(&alice_runtime, &bob, &room_id, "Bob");
        let alice_state = alice_runtime
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "reaction target".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let target_message_id = alice_state
            .messages
            .iter()
            .find(|message| message.text == "reaction target")
            .unwrap()
            .message_id
            .clone();
        bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        drop(alice_runtime);

        let mut alice = AppRuntimeState::new(CoreState::open(alice_options).unwrap()).unwrap();
        bob.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "concurrent message before reaction".to_owned(),
            metadata_json: None,
        })
        .unwrap();

        alice
            .react_to_message(room_id.clone(), target_message_id.clone(), "👍".to_owned())
            .unwrap();
        assert!(
            alice
                .app
                .messages
                .iter()
                .any(|message| message.text == "concurrent message before reaction"),
            "reaction sync must project a concurrent message before its durable cursor advances"
        );

        bob.dispatch_and_wait(AppAction::ReactToMessage {
            room_id: room_id.clone(),
            message_id: target_message_id.clone(),
            emoji: "🎯".to_owned(),
        })
        .unwrap();
        alice
            .send_message(
                room_id,
                "send while peer reaction is queued".to_owned(),
                None,
                None,
            )
            .unwrap();
        assert_reaction(&alice.app, &target_message_id, "🎯", 1, false);
    }

    #[test]
    fn app_runtime_read_receipts_are_durable_and_live_projected() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let bob_dir = dir.path().join("bob");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: bob_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Receipt Room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");

        let bob_state = bob
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "read me".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let target_message_id = bob_state
            .messages
            .iter()
            .find(|message| message.text == "read me")
            .expect("sent message projects")
            .message_id
            .clone();

        alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        alice
            .dispatch_and_wait(AppAction::MarkRoomRead {
                room_id: room_id.clone(),
            })
            .unwrap();
        alice
            .dispatch_and_wait(AppAction::MarkRoomRead {
                room_id: room_id.clone(),
            })
            .unwrap();

        let bob_state = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_read_receipt(&bob_state, &target_message_id, 1, 1, "Read by 1");
        drop(bob);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: bob_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        assert_read_receipt(
            &reopened.state().unwrap(),
            &target_message_id,
            1,
            1,
            "Read by 1",
        );
    }

    #[test]
    fn app_runtime_syncs_second_hermes_message_after_read_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let app_dir = dir.path().join("app");
        let agent = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("agent").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: app_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "ios-smoke".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let agent_state = agent
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Hermes Receipt Followup".to_owned(),
            })
            .unwrap();
        let room_id = agent_state.rooms.first().unwrap().room_id.clone();
        let app_account_id = app.state().unwrap().identity.account_id;
        add_runtime_member(
            &agent,
            &app,
            &room_id,
            test_profile(&app_account_id, "iOS App"),
        );

        agent
            .send_encoded_chat_message_and_wait(
                room_id.clone(),
                hermes_chat_event("first agent message"),
                "first agent message".to_owned(),
            )
            .unwrap();
        let first_sync = app.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            first_sync
                .messages
                .iter()
                .any(|message| message.text == "first agent message")
        );
        app.dispatch_and_wait(AppAction::MarkRoomRead {
            room_id: room_id.clone(),
        })
        .unwrap();
        agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        agent
            .send_encoded_chat_message_and_wait(
                room_id.clone(),
                hermes_chat_event("second agent message"),
                "second agent message".to_owned(),
            )
            .unwrap();
        let live_second_sync = app.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            live_second_sync
                .messages
                .iter()
                .any(|message| message.text == "second agent message"),
            "live app runtime should project the second Hermes message without a relaunch"
        );
        drop(app);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: app_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "ios-smoke".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let final_state = reopened.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            final_state
                .messages
                .iter()
                .any(|message| message.text == "second agent message"),
            "app should decrypt the second Hermes message after sending a read receipt"
        );
    }

    #[test]
    fn runtime_device_link_fanout_enrolls_same_account_device_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let hosted = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("hosted").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "hosted-web".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let agent = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("agent").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "agent".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let hosted_state = hosted
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Device Parity".to_owned(),
            })
            .unwrap();
        let room_id = hosted_state.rooms[0].room_id.clone();
        add_runtime_member_named(&hosted, &agent, &room_id, "Agent");

        let hosted_identity = hosted.state().unwrap().identity;
        let electron = FiniteChatRuntime::open(OpenOptions {
            data_dir: dir.path().join("electron").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "electron-alpha".to_owned(),
            account_secret_hex: Some(hosted_identity.account_secret_hex),
            now_unix_seconds: Some(NOW),
        })
        .unwrap();
        electron
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("Electron publishes KeyPackages");

        let (first, electron_state) =
            drive_device_link_to_exact_receipts(&hosted, &electron, "link-alpha", "electron-alpha");
        assert!(first.fanout_complete);
        assert_eq!(first.room_count, 1);
        assert_eq!(first.active_room_count, 1);
        assert_eq!(
            app_room(&electron_state, &room_id).state,
            AppRoomState::Connected
        );

        let completed = hosted
            .link_device_and_wait("link-alpha".to_owned(), "electron-alpha".to_owned())
            .unwrap();
        assert!(completed.fanout_complete);
        assert_eq!(completed.room_count, 1);
        assert_eq!(completed.active_room_count, 1);

        let mismatch = hosted
            .link_device_and_wait("link-alpha".to_owned(), "electron-other".to_owned())
            .unwrap_err();
        assert!(mismatch.to_string().contains("another Device"));

        electron
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "from the local Device".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        let hosted_state = hosted.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let message = hosted_state
            .messages
            .iter()
            .find(|message| message.text == "from the local Device")
            .unwrap();
        assert!(message.is_mine);
        assert_eq!(message.sender_device_id, "electron-alpha");
        assert_eq!(message.outbound_delivery, None);
        let agent_state = agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert!(
            agent_state
                .messages
                .iter()
                .any(|message| message.text == "from the local Device" && !message.is_mine)
        );

        drop(electron);
        let replacement = FiniteChatRuntime::open(OpenOptions {
            data_dir: dir
                .path()
                .join("electron-replacement")
                .to_string_lossy()
                .into_owned(),
            server_url: server_url.clone(),
            device_id: "electron-alpha".to_owned(),
            account_secret_hex: Some(hosted.state().unwrap().identity.account_secret_hex),
            now_unix_seconds: Some(NOW),
        })
        .unwrap();
        let replacement_state = replacement
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("replacement publishes its own cryptographic state");
        assert!(
            !replacement_state
                .rooms
                .iter()
                .any(|room| room.room_id == room_id),
            "replacement must not inherit the old Device generation's room"
        );

        let collision = hosted
            .link_device_and_wait("link-replacement".to_owned(), "electron-alpha".to_owned())
            .unwrap_err();
        assert!(
            collision
                .to_string()
                .contains("already belongs to an enrolled Device"),
            "a replacement cryptographic state must fail closed instead of reporting ready: {collision}"
        );

        let fresh_replacement = FiniteChatRuntime::open(OpenOptions {
            data_dir: dir
                .path()
                .join("electron-fresh-replacement")
                .to_string_lossy()
                .into_owned(),
            server_url,
            device_id: "electron-beta".to_owned(),
            account_secret_hex: Some(hosted.state().unwrap().identity.account_secret_hex),
            now_unix_seconds: Some(NOW),
        })
        .unwrap();
        fresh_replacement
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("fresh replacement publishes its own cryptographic state");
        let (fresh_link, fresh_state) = drive_device_link_to_exact_receipts(
            &hosted,
            &fresh_replacement,
            "link-fresh-replacement",
            "electron-beta",
        );
        assert!(fresh_link.fanout_complete);
        assert_eq!(
            app_room(&fresh_state, &room_id).state,
            AppRoomState::Connected
        );
    }

    #[test]
    fn device_link_export_restarts_at_every_boundary_and_terminal_polls_are_inert() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let source_options = with_test_secret(OpenOptions {
            data_dir: dir.path().join("source").to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "source-device".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        });
        let mut source = FiniteChatRuntime::open(source_options.clone()).unwrap();
        let room_id = source
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Restartable transfer".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        const HISTORY_MESSAGES: usize = 300;
        for index in 0..HISTORY_MESSAGES {
            source
                .dispatch_and_wait(AppAction::SendMessage {
                    room_id: room_id.clone(),
                    text: format!("frozen history {index}"),
                    metadata_json: None,
                })
                .unwrap();
        }

        let source_identity = source.state().unwrap().identity;
        let target_dir = dir.path().join("target");
        let target = FiniteChatRuntime::open(OpenOptions {
            data_dir: target_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "target-device".to_owned(),
            account_secret_hex: Some(source_identity.account_secret_hex),
            now_unix_seconds: Some(NOW),
        })
        .unwrap();
        target
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("target publishes KeyPackages");

        let first = source
            .link_device_and_wait(
                "restartable-bootstrap".to_owned(),
                "target-device".to_owned(),
            )
            .unwrap();
        assert!(!first.fanout_complete);
        drop(source);

        let frozen_through_seq = (0..20)
            .find_map(|_| {
                let reopened = FiniteChatRuntime::open(source_options.clone()).unwrap();
                reopened
                    .link_device_and_wait(
                        "restartable-bootstrap".to_owned(),
                        "target-device".to_owned(),
                    )
                    .unwrap();
                drop(reopened);
                let core = CoreState::open(source_options.clone()).unwrap();
                let frozen = match core
                    .device
                    .link_fanout_bootstrap_export("restartable-bootstrap")
                    .unwrap()
                {
                    LinkBootstrapExportState::Exporting { rooms, .. } => {
                        rooms.first().map(|room| room.through_seq)
                    }
                    LinkBootstrapExportState::WaitingForFanout
                    | LinkBootstrapExportState::Emitted { .. } => None,
                };
                drop(core);
                target
                    .dispatch_and_wait(AppAction::StartRuntime)
                    .expect("target advances while the source freezes its cutoff");
                frozen
            })
            .expect("source freezes the accepted membership sequence");
        source = FiniteChatRuntime::open(source_options.clone()).unwrap();
        source
            .dispatch_and_wait(AppAction::SendMessage {
                room_id: room_id.clone(),
                text: "ordinary sync after frozen enrollment snapshot".to_owned(),
                metadata_json: None,
            })
            .unwrap();
        drop(source);

        let (terminal, converged) = (0..100)
            .find_map(|_| {
                let reopened = FiniteChatRuntime::open(source_options.clone()).unwrap();
                let report = reopened
                    .link_device_and_wait(
                        "restartable-bootstrap".to_owned(),
                        "target-device".to_owned(),
                    )
                    .unwrap();
                drop(reopened);
                let target_state = target
                    .dispatch_and_wait(AppAction::StartRuntime)
                    .expect("target advances one bounded sync tick");
                let has_receipt = report.bootstrap_manifests.iter().all(|expected| {
                    target_state
                        .device_link_bootstrap_receipts
                        .iter()
                        .any(|actual| {
                            actual.bootstrap_id == expected.bootstrap_id
                                && actual.room_id == expected.room_id
                                && actual.manifest_sha256 == expected.manifest_sha256
                        })
                });
                (report.fanout_complete && has_receipt).then_some((report, target_state))
            })
            .expect("restarting every source tick still converges");
        assert_eq!(terminal.room_count, 1);
        assert_eq!(terminal.active_room_count, 1);
        assert_eq!(terminal.bootstrap_manifests.len(), 1);
        assert!(
            converged.messages.iter().any(|message| {
                message.text == "ordinary sync after frozen enrollment snapshot"
            }),
            "events accepted after the immutable cutoff arrive through ordinary MLS sync"
        );

        let target_db = rusqlite::Connection::open(target_dir.join(CLIENT_STORE_FILE)).unwrap();
        let imported_before = target_db
            .query_row("SELECT COUNT(*) FROM client_app_events", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap();
        assert!(imported_before >= HISTORY_MESSAGES as u64);
        let max_imported_seq = target_db
            .query_row("SELECT MAX(seq) FROM client_app_events", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap();
        assert!(
            max_imported_seq > frozen_through_seq,
            "ordinary post-cutoff events do not mutate the frozen bootstrap"
        );
        drop(target_db);

        source = FiniteChatRuntime::open(source_options).unwrap();
        for _ in 0..100 {
            assert_eq!(
                source
                    .link_device_and_wait(
                        "restartable-bootstrap".to_owned(),
                        "target-device".to_owned(),
                    )
                    .unwrap(),
                terminal
            );
        }
        let after_terminal_polls = target
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("terminal polling creates no additional target work");
        assert_eq!(
            after_terminal_polls.device_link_bootstrap_receipts,
            converged.device_link_bootstrap_receipts
        );
        let target_db = rusqlite::Connection::open(target_dir.join(CLIENT_STORE_FILE)).unwrap();
        let imported_after = target_db
            .query_row("SELECT COUNT(*) FROM client_app_events", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap();
        assert_eq!(imported_after, imported_before);
    }

    #[test]
    fn app_runtime_unread_counts_are_local_durable_and_offline_clearable() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice");
        let bob_dir = dir.path().join("bob");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let bob = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: bob_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "bob-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Unread Room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        add_runtime_member_named(&alice, &bob, &room_id, "Bob");

        bob.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "first unread".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        let alice_state = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(app_room(&alice_state, &room_id).unread_count, 1);

        let alice_state = alice
            .dispatch_and_wait(AppAction::MarkRoomRead {
                room_id: room_id.clone(),
            })
            .unwrap();
        assert_eq!(app_room(&alice_state, &room_id).unread_count, 0);

        bob.dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "second unread".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        let alice_state = alice.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        assert_eq!(app_room(&alice_state, &room_id).unread_count, 1);
        assert_eq!(
            app_room(&alice_state, &room_id).last_message_preview,
            "second unread"
        );
        drop(alice);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let offline_state = reopened.state().unwrap();
        assert_eq!(app_room(&offline_state, &room_id).unread_count, 1);

        let cleared = reopened
            .dispatch_and_wait(AppAction::MarkRoomRead {
                room_id: room_id.clone(),
            })
            .unwrap();
        assert_eq!(app_room(&cleared, &room_id).unread_count, 0);
        drop(reopened);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        assert_eq!(
            app_room(&reopened.state().unwrap(), &room_id).unread_count,
            0
        );
    }

    #[test]
    fn app_runtime_lists_and_revokes_same_account_devices() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice-phone");
        let alice = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-phone".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let alice_identity = alice.state().unwrap().identity;
        let tablet = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir
                .path()
                .join("alice-tablet")
                .to_string_lossy()
                .into_owned(),
            server_url: server_url.clone(),
            device_id: "alice-tablet".to_owned(),
            account_secret_hex: Some(alice_identity.account_secret_hex.clone()),
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let alice_state = alice
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Device Room".to_owned(),
            })
            .unwrap();
        let room_id = alice_state.rooms.first().unwrap().room_id.clone();
        add_runtime_member_named(&alice, &tablet, &room_id, "Alice Tablet");

        let devices = alice.dispatch_and_wait(AppAction::RefreshDevices).unwrap();
        assert_device(&devices, "alice-phone", true, true, false);
        assert_device(&devices, "alice-tablet", true, false, false);
        let details = devices
            .room_details
            .as_ref()
            .expect("selected room details include refreshed device projection");
        assert_eq!(details.room_id, room_id);
        assert_eq!(details.devices.len(), 2);
        assert_device_in_room_details(details, "alice-phone", true, true, false);
        assert_device_in_room_details(details, "alice-tablet", true, false, false);

        let devices = alice
            .dispatch_and_wait(AppAction::RevokeDevice {
                account_id: alice_identity.account_id,
                device_id: "alice-tablet".to_owned(),
            })
            .unwrap();
        assert_device(&devices, "alice-tablet", true, false, true);
        assert_device_in_room_details(
            devices
                .room_details
                .as_ref()
                .expect("selected room details include revoked device projection"),
            "alice-tablet",
            true,
            false,
            true,
        );
        drop(alice);

        let reopened = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: alice_dir.to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-phone".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let devices = reopened
            .dispatch_and_wait(AppAction::RefreshDevices)
            .unwrap();
        assert_device(&devices, "alice-tablet", true, false, true);
        assert_device_in_room_details(
            devices
                .room_details
                .as_ref()
                .expect("reopened room details preserve local revoked-device mark"),
            "alice-tablet",
            true,
            false,
            true,
        );
    }

    #[test]
    fn app_device_actions_offline_are_transient_only() {
        let dir = tempfile::tempdir().unwrap();
        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.path().join("alice").to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-phone".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();

        let refreshed = app.dispatch_and_wait(AppAction::RefreshDevices).unwrap();
        assert_eq!(refreshed.status, "devices unavailable");
        assert_eq!(
            refreshed.toast.as_deref(),
            Some("Device list could not be refreshed")
        );
        assert!(refreshed.devices.is_empty());
        assert!(refreshed.messages.is_empty());
        assert!(runtime_outbox(&app).is_empty());

        let account_id = refreshed.identity.account_id.clone();
        let revoked = app
            .dispatch_and_wait(AppAction::RevokeDevice {
                account_id,
                device_id: "alice-tablet".to_owned(),
            })
            .unwrap();
        assert_eq!(revoked.status, "device unavailable");
        assert_eq!(
            revoked.toast.as_deref(),
            Some("Device could not be revoked")
        );
        assert!(revoked.devices.is_empty());
        assert!(revoked.messages.is_empty());
        assert!(runtime_outbox(&app).is_empty());
        assert!(app.test_revoked_devices().unwrap().is_empty());
    }

    #[test]
    fn app_corrupted_state_keeps_room_unavailable_without_local_mls() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("alice");
        let room_id = "room-missing-local-mls".to_owned();
        let seeded = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        seeded
            .test_seed_room_state(
                StoredAppRoom {
                    room_id: room_id.clone(),
                    display_name: "Missing MLS Room".to_owned(),
                    picture: None,
                    state: StoredAppRoomState::UnavailableOnDevice,
                    status: LOCAL_ROOM_UNAVAILABLE_STATUS.to_owned(),
                    local_read_seq: 0,
                },
                Some(room_id.clone()),
            )
            .unwrap();
        drop(seeded);

        let app = FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: unavailable_http_server_url(),
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap();
        let initial = app.state().unwrap();
        let initial_room = app_room(&initial, &room_id);
        assert_eq!(initial_room.state, AppRoomState::UnavailableOnDevice);
        assert_eq!(initial_room.status, LOCAL_ROOM_UNAVAILABLE_STATUS);
        assert_eq!(initial.selected_room_id.as_deref(), Some(room_id.as_str()));

        let after_start = app.dispatch_and_wait(AppAction::StartRuntime).unwrap();
        let room = app_room(&after_start, &room_id);
        assert_eq!(room.state, AppRoomState::UnavailableOnDevice);
        assert_eq!(room.status, LOCAL_ROOM_UNAVAILABLE_STATUS);
        assert!(runtime_outbox(&app).is_empty());
    }

    #[test]
    fn app_stored_room_without_local_mls_projects_unavailable_on_device_status() {
        let room = app_room_from_stored(
            StoredAppRoom {
                room_id: "room_missing_local_mls".to_owned(),
                display_name: "Saved Room".to_owned(),
                picture: None,
                state: StoredAppRoomState::Connected,
                status: "connected".to_owned(),
                local_read_seq: 0,
            },
            false,
        );

        assert_eq!(room.state, AppRoomState::UnavailableOnDevice);
        assert_eq!(room.status, LOCAL_ROOM_UNAVAILABLE_STATUS);
        assert_eq!(room.user_status_text, LOCAL_ROOM_UNAVAILABLE_TEXT);

        let stale_room = app_room_from_stored(
            StoredAppRoom {
                room_id: "room_stale_missing_local_mls".to_owned(),
                display_name: "Stale Saved Room".to_owned(),
                picture: None,
                state: StoredAppRoomState::UnavailableOnDevice,
                status: LOCAL_ROOM_UNAVAILABLE_TEXT.to_owned(),
                local_read_seq: 0,
            },
            false,
        );

        assert_eq!(stale_room.state, AppRoomState::UnavailableOnDevice);
        assert_eq!(stale_room.status, LOCAL_ROOM_UNAVAILABLE_STATUS);
        assert_eq!(stale_room.user_status_text, LOCAL_ROOM_UNAVAILABLE_TEXT);

        let stale_but_available_room = app_room_from_stored(
            StoredAppRoom {
                room_id: "room_stale_but_available".to_owned(),
                display_name: "Available Saved Room".to_owned(),
                picture: None,
                state: StoredAppRoomState::UnavailableOnDevice,
                status: LOCAL_ROOM_UNAVAILABLE_TEXT.to_owned(),
                local_read_seq: 0,
            },
            true,
        );

        assert_eq!(stale_but_available_room.state, AppRoomState::Connected);
        assert_eq!(stale_but_available_room.status, "connected");
        assert_eq!(
            stale_but_available_room.user_status_text,
            LOCAL_ROOM_CONNECTED_TEXT
        );
    }

    fn hermes_chat_event(text: &str) -> Vec<u8> {
        let payload = HermesMessagePayloadV1 {
            payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: None,
            segment_id: None,
            text: text.to_owned(),
            kind: finitechat_hermes::HermesSendKindV1::Message,
            status: finitechat_hermes::HermesMessageStatusV1::Complete,
            edit_of: None,
            attachments: Vec::new(),
            reply_to_message_id: None,
            sender_name: None,
            metadata: Default::default(),
        }
        .encode()
        .unwrap();
        encode_application_event(DurableAppEventKind::ChatMessage, None, &payload).unwrap()
    }

    fn app_room<'a>(state: &'a AppState, room_id: &str) -> &'a AppRoomSummary {
        state
            .rooms
            .iter()
            .find(|room| room.room_id == room_id)
            .unwrap_or_else(|| panic!("missing app room {room_id}"))
    }

    fn app_profile<'a>(state: &'a AppState, account_id: &str) -> &'a AppProfileSummary {
        state
            .profiles
            .iter()
            .find(|profile| profile.account_id == account_id)
            .unwrap_or_else(|| panic!("missing app profile {account_id}"))
    }

    fn drive_device_link_to_exact_receipts(
        source: &Arc<FiniteChatRuntime>,
        target: &Arc<FiniteChatRuntime>,
        fanout_id: &str,
        target_device_id: &str,
    ) -> (DeviceLinkFanoutReport, AppState) {
        const MAX_TICKS: usize = 100_000;
        for _ in 0..MAX_TICKS {
            let report = source
                .link_device_and_wait(fanout_id.to_owned(), target_device_id.to_owned())
                .expect("source advances one bounded device-link tick");
            let target_state = target
                .dispatch_and_wait(AppAction::StartRuntime)
                .expect("target advances one bounded sync tick");
            let exact_receipts_present = report.fanout_complete
                && report.bootstrap_manifests.len() == report.room_count as usize
                && report.bootstrap_manifests.iter().all(|expected| {
                    target_state
                        .device_link_bootstrap_receipts
                        .iter()
                        .any(|actual| {
                            actual.bootstrap_id == expected.bootstrap_id
                                && actual.room_id == expected.room_id
                                && actual.manifest_sha256 == expected.manifest_sha256
                        })
                });
            if exact_receipts_present {
                return (report, target_state);
            }
        }
        panic!("device link did not converge to exact durable receipts within {MAX_TICKS} ticks");
    }

    fn add_runtime_member(
        owner: &Arc<FiniteChatRuntime>,
        member: &Arc<FiniteChatRuntime>,
        room_id: &str,
        profile: AppProfileSummary,
    ) -> AppState {
        member
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("member publishes key packages");
        let state = owner
            .dispatch_and_wait(AppAction::AddRoomMembers {
                room_id: room_id.to_owned(),
                profiles: vec![profile],
            })
            .expect("owner adds member through MLS add/welcome");
        let member_state = member
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("member claims Welcome");
        assert_eq!(
            app_room(&member_state, room_id).state,
            AppRoomState::Connected
        );
        state
    }

    fn add_runtime_member_named(
        owner: &Arc<FiniteChatRuntime>,
        member: &Arc<FiniteChatRuntime>,
        room_id: &str,
        display_name: &str,
    ) -> AppState {
        let account_id = member.state().unwrap().identity.account_id;
        add_runtime_member(
            owner,
            member,
            room_id,
            test_profile(&account_id, display_name),
        )
    }

    fn test_profile(account_id: &str, display_name: &str) -> AppProfileSummary {
        AppProfileSummary {
            account_id: account_id.to_owned(),
            npub: npub_encode(account_id).unwrap_or_else(|_| account_id.to_owned()),
            display_name: display_name.to_owned(),
            about: None,
            picture: None,
            stale: true,
            is_agent: false,
        }
    }

    fn bootstrap_profile_chat_for_test(
        runtime: &Arc<FiniteChatRuntime>,
        profile: AppProfileSummary,
        display_name: &str,
        intended_room_id: &str,
    ) -> Result<AppState, FiniteChatCoreError> {
        let claimed = runtime
            .claim_profile_chat_key_package_and_wait(profile.account_id.clone())?
            .ok_or_else(|| FiniteChatCoreError::Client {
                reason: "test Agent has no available KeyPackage".to_owned(),
            })?;
        let room_create_request =
            runtime.plan_profile_chat_bootstrap_room_and_wait(intended_room_id.to_owned())?;
        let preparation = runtime.prepare_profile_chat_bootstrap_and_wait(
            AppProfileChatBootstrapInput {
                profile: profile.clone(),
                display_name: display_name.to_owned(),
                intended_room_id: intended_room_id.to_owned(),
                room_create_request,
                claimed_key_package: claimed,
                persisted_prepared_commit: None,
            },
            false,
        )?;
        if preparation.complete {
            return Ok(preparation.state);
        }
        runtime.submit_profile_chat_bootstrap_and_wait(
            profile.account_id,
            display_name.to_owned(),
            intended_room_id.to_owned(),
            preparation
                .prepared_commit
                .expect("prepared bootstrap commit"),
            false,
        )
    }

    fn test_profile_with_picture(
        account_id: &str,
        display_name: &str,
        picture: &str,
    ) -> AppProfileSummary {
        let mut profile = test_profile(account_id, display_name);
        profile.picture = Some(picture.to_owned());
        profile.stale = false;
        profile
    }

    fn assert_outbound_undelivered(message: &ChatMessage) {
        let outbound = message
            .outbound_delivery
            .as_ref()
            .unwrap_or_else(|| panic!("missing outbound delivery on {}", message.message_id));
        assert_eq!(outbound.local_send, OutboundLocalSendState::Sent);
        assert_eq!(
            outbound.server_delivery,
            OutboundServerDeliveryState::Undelivered
        );
    }

    fn assert_outbound_delivered(message: &ChatMessage) {
        let outbound = message
            .outbound_delivery
            .as_ref()
            .unwrap_or_else(|| panic!("missing outbound delivery on {}", message.message_id));
        assert_eq!(outbound.local_send, OutboundLocalSendState::Sent);
        assert_eq!(
            outbound.server_delivery,
            OutboundServerDeliveryState::Delivered
        );
    }

    fn assert_outbound_failed_contains(message: &ChatMessage, expected_reason: &str) {
        let outbound = message
            .outbound_delivery
            .as_ref()
            .unwrap_or_else(|| panic!("missing outbound delivery on {}", message.message_id));
        assert_eq!(outbound.local_send, OutboundLocalSendState::Sent);
        match &outbound.server_delivery {
            OutboundServerDeliveryState::Failed { reason } => {
                assert!(
                    reason.contains(expected_reason),
                    "failure reason {reason:?} should contain {expected_reason:?}"
                );
            }
            other => panic!("expected failed outbound delivery, got {other:?}"),
        }
    }

    fn runtime_outbox(runtime: &FiniteChatRuntime) -> Vec<StoredOutboundMessage> {
        runtime.test_outbox().unwrap()
    }

    fn application_effect(
        server_url: &str,
        message_id: &str,
    ) -> Option<HttpApplicationDeliveryEffect> {
        let response = reqwest::blocking::Client::new()
            .post(format!("{server_url}/application-effects/get"))
            .json(&ApplicationEffectRequest {
                message_id: message_id.to_owned(),
            })
            .send()
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        response.json().unwrap()
    }

    fn assert_device(
        state: &AppState,
        device_id: &str,
        active: bool,
        current_device: bool,
        revoked: bool,
    ) {
        let device = state
            .devices
            .iter()
            .find(|device| device.device_id == device_id)
            .unwrap_or_else(|| panic!("missing device {device_id}"));
        assert_eq!(device.active, active);
        assert_eq!(device.current_device, current_device);
        assert_eq!(device.revoked, revoked);
        assert_eq!(device.room_count, 1);
    }

    fn assert_device_in_room_details(
        details: &AppRoomDetailsState,
        device_id: &str,
        active: bool,
        current_device: bool,
        revoked: bool,
    ) {
        let device = details
            .devices
            .iter()
            .find(|device| device.device_id == device_id)
            .unwrap_or_else(|| panic!("missing room details device {device_id}"));
        assert_eq!(device.active, active);
        assert_eq!(device.current_device, current_device);
        assert_eq!(device.revoked, revoked);
        assert_eq!(device.room_count, 1);
    }

    fn assert_member_in_room_details(
        details: &AppRoomDetailsState,
        account_id: &str,
        device_id: &str,
        current_device: bool,
    ) {
        let member = room_details_member(details, account_id, device_id);
        assert_eq!(member.current_device, current_device);
        assert!(!member.display_name.trim().is_empty());
        assert!(member.npub.starts_with("npub1") || member.npub == account_id);
    }

    fn room_details_member<'a>(
        details: &'a AppRoomDetailsState,
        account_id: &str,
        device_id: &str,
    ) -> &'a AppRoomMemberSummary {
        details
            .members
            .iter()
            .find(|member| member.account_id == account_id && member.device_id == device_id)
            .unwrap_or_else(|| panic!("missing room details member {account_id}/{device_id}"))
    }

    fn assert_reaction(
        state: &AppState,
        message_id: &str,
        emoji: &str,
        count: u32,
        reacted_by_me: bool,
    ) {
        let message = state
            .messages
            .iter()
            .find(|message| message.message_id == message_id)
            .unwrap_or_else(|| panic!("missing message {message_id}"));
        let reaction = message
            .reactions
            .iter()
            .find(|reaction| reaction.emoji == emoji)
            .unwrap_or_else(|| panic!("missing reaction {emoji} on {message_id}"));
        assert_eq!(reaction.count, count);
        assert_eq!(reaction.reacted_by_me, reacted_by_me);
    }

    fn assert_poll(
        state: &AppState,
        message_id: &str,
        question: &str,
        option_id: &str,
        option_votes: u32,
        total_votes: u32,
        voted_by_me: bool,
    ) {
        let message = state
            .messages
            .iter()
            .find(|message| message.message_id == message_id)
            .unwrap_or_else(|| panic!("missing poll message {message_id}"));
        assert_poll_message(
            message,
            question,
            option_id,
            option_votes,
            total_votes,
            voted_by_me,
        );
    }

    fn assert_poll_message(
        message: &ChatMessage,
        question: &str,
        option_id: &str,
        option_votes: u32,
        total_votes: u32,
        voted_by_me: bool,
    ) {
        let poll = message
            .poll
            .as_ref()
            .unwrap_or_else(|| panic!("missing poll on {}", message.message_id));
        assert_eq!(poll.question, question);
        assert_eq!(poll.total_votes, total_votes);
        assert_eq!(
            poll.my_vote_option_id.as_deref() == Some(option_id),
            voted_by_me
        );
        let option = poll
            .options
            .iter()
            .find(|option| option.option_id == option_id)
            .unwrap_or_else(|| panic!("missing poll option {option_id}"));
        assert_eq!(option.vote_count, option_votes);
        assert_eq!(option.voted_by_me, voted_by_me);
    }

    fn assert_read_receipt(
        state: &AppState,
        message_id: &str,
        delivered_count: u32,
        read_count: u32,
        display_text: &str,
    ) {
        let message = state
            .messages
            .iter()
            .find(|message| message.message_id == message_id)
            .unwrap_or_else(|| panic!("missing message {message_id}"));
        let receipt = message
            .read_receipt
            .as_ref()
            .unwrap_or_else(|| panic!("missing read receipt on {message_id}"));
        assert_eq!(receipt.delivered_count, delivered_count);
        assert_eq!(receipt.read_count, read_count);
        assert_eq!(receipt.display_text, display_text);
    }

    fn attachment_reference_from_message(message: &ChatMessage) -> AttachmentBlobReferenceV1 {
        let DecodedAppEvent::ChatMessage(decoded) = decode_application_event(&message.payload)
        else {
            panic!("expected chat message application event");
        };
        let ChatMessagePayload::Hermes(hermes) = decoded.payload else {
            panic!("Hermes payload");
        };
        hermes
            .attachments
            .first()
            .and_then(|attachment| attachment.blob.clone())
            .expect("blob reference")
    }

    fn put_profile(server_url: &str, profile: NostrProfileRecord) {
        let response = reqwest::blocking::Client::new()
            .post(format!("{server_url}/profiles/nostr"))
            .json(&PutNostrProfileRequest { profile })
            .send()
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    fn get_profiles(
        server_url: &str,
        account_ids: Vec<String>,
    ) -> finitechat_http::GetNostrProfilesResponse {
        let response = reqwest::blocking::Client::new()
            .post(format!("{server_url}/profiles/nostr/get"))
            .json(&GetNostrProfilesRequest {
                account_ids,
                now_ms: NOW.saturating_mul(1000),
            })
            .send()
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        response.json().unwrap()
    }

    fn decode_nip98_event(header: &str) -> NostrHttpAuthEvent {
        let encoded = header.strip_prefix(FINITE_SITES_NIP98_AUTH_SCHEME).unwrap();
        let raw = BASE64.decode(encoded).unwrap();
        serde_json::from_slice(&raw).unwrap()
    }

    fn tag_value<'a>(event: &'a NostrHttpAuthEvent, name: &str) -> Option<&'a str> {
        for tag in &event.tags {
            if tag.len() >= 2 && tag[0] == name {
                return Some(tag[1].as_str());
            }
        }
        None
    }

    fn spawn_live_http_server(path: impl AsRef<Path>) -> String {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let app = http_router(HttpServerState::from_sqlite_path(path).unwrap());
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(listener, app).await.unwrap();
            });
        });
        let server_url = format!("http://{addr}");
        wait_for_live_http_server(&server_url);
        server_url
    }

    fn unavailable_http_server_url() -> String {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}")
    }

    fn wait_for_live_http_server(server_url: &str) {
        let health_url = format!("{}/health", server_url.trim_end_matches('/'));
        let client = reqwest::blocking::Client::new();
        for _ in 0..100 {
            if client
                .get(&health_url)
                .send()
                .map(|response| response.status().is_success())
                .unwrap_or(false)
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("live HTTP test server did not become healthy at {health_url}");
    }

    /// Open a core with an explicit account secret, so two data dirs can hold
    /// the same logical device (the two-writers incident setup).
    fn open_test_core_with_account(
        dir: impl AsRef<Path>,
        server_url: &str,
        device_id: &str,
        account_seed: &str,
    ) -> CoreState {
        CoreState::open(OpenOptions {
            data_dir: dir.as_ref().to_string_lossy().into_owned(),
            server_url: server_url.to_owned(),
            device_id: device_id.to_owned(),
            account_secret_hex: Some(test_account_secret_hex(account_seed)),
            now_unix_seconds: Some(NOW),
        })
        .unwrap()
    }

    #[test]
    fn read_only_runtime_reports_persisted_state_and_rejects_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let options = with_test_secret(OpenOptions {
            data_dir: dir.path().join("app").to_string_lossy().into_owned(),
            server_url,
            device_id: "alice-ios".to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        });
        let writer = FiniteChatRuntime::open(options.clone()).unwrap();
        let created = writer
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Main".to_owned(),
            })
            .expect("writer creates a room");
        let room_id = created.selected_room_id.clone().expect("room selected");

        // The resident writer still holds the store and its lease; a
        // read-only runtime opens alongside it, reports persisted state, and
        // refuses every dispatched action.
        let reader = FiniteChatRuntime::open_read_only(options).unwrap();
        let state = reader.state().unwrap();
        assert!(state.rooms.iter().any(|room| room.room_id == room_id));
        let error = reader
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect_err("read-only runtime rejects actions");
        assert!(matches!(error, FiniteChatCoreError::ReadOnly));
    }

    /// Snapshot a data dir's client store through the SQLite backup API
    /// (WAL-safe), producing a second data dir with identical device state:
    /// the "two writers, one logical device" incident setup. The writer-lease
    /// sidecar is deliberately not copied.
    fn copy_client_store(data_dir: &Path, target_dir: &Path) {
        fs::create_dir_all(target_dir).unwrap();
        let source = rusqlite::Connection::open(data_dir.join(CLIENT_STORE_FILE)).unwrap();
        let mut target = rusqlite::Connection::open(target_dir.join(CLIENT_STORE_FILE)).unwrap();
        let backup = rusqlite::backup::Backup::new(&source, &mut target).unwrap();
        backup
            .run_to_completion(64, std::time::Duration::from_millis(0), None)
            .unwrap();
    }

    fn open_app_runtime_state_with_account(
        dir: impl AsRef<Path>,
        server_url: &str,
        device_id: &str,
        account_seed: &str,
    ) -> AppRuntimeState {
        AppRuntimeState::new(open_test_core_with_account(
            dir,
            server_url,
            device_id,
            account_seed,
        ))
        .unwrap()
    }

    fn room_application_entry_count(state: &AppRuntimeState, room_id: &str) -> usize {
        let owner = state.core.device.device_ref().clone();
        let page = state
            .core
            .home_delivery()
            .sync_events(room_id, &owner, 0)
            .unwrap();
        page.entries
            .iter()
            .filter(|entry| entry.kind == LogEntryKind::Application)
            .count()
    }

    /// The poison-entry regression: one logical device (same account+device
    /// identity, store snapshot from before the first fire) fires
    /// `ensure_home_topic` twice with a ratchet that has diverged from the
    /// snapshot. Both fires use the same deterministic idempotency key, so
    /// the second produces a different ciphertext under a key the server
    /// already holds: the server answers 409 idempotency_conflict and the
    /// stale writer adopts the existing topic locally instead of appending
    /// an application ciphertext the room can never process.
    ///
    /// Note the double-fire cannot be an exact replay: MLS message creation
    /// injects fresh randomness, so even a same-state re-fire differs from
    /// the recorded request and conflicts. Adopting locally is safe because
    /// the deterministic key proves the conflicting entry is this device's
    /// own earlier home-topic create — and a sender cannot decrypt its own
    /// MLS application entries, so the log cannot restore it.
    #[test]
    fn diverged_double_fired_home_topic_adopts_instead_of_poisoning_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let room_id = "room-home-topic-fail-closed";

        let mut alice = open_test_core_with_account(
            dir.path().join("alice"),
            &server_url,
            "alice-ios",
            "home-topic-fail-closed-alice",
        );
        alice
            .bootstrap_room(room_id, Some("Fail Closed".to_owned()))
            .unwrap();
        drop(alice);
        copy_client_store(&dir.path().join("alice"), &dir.path().join("alice-stale"));

        let mut first = open_app_runtime_state_with_account(
            dir.path().join("alice"),
            &server_url,
            "alice-ios",
            "home-topic-fail-closed-alice",
        );
        // Advance the ratchet past the snapshot before firing the home topic.
        first
            .core
            .send_application_event(
                room_id,
                DurableAppEventKind::ChatMessage,
                None,
                &encode_text_message_payload("first", None).unwrap(),
                "chat",
            )
            .unwrap();
        first.ensure_home_topic(room_id).unwrap();
        assert!(first.topic_exists(room_id, HOME_TOPIC_ID));

        let mut second = open_app_runtime_state_with_account(
            dir.path().join("alice-stale"),
            &server_url,
            "alice-ios",
            "home-topic-fail-closed-alice",
        );
        assert!(second.room_is_connected(room_id));
        assert!(!second.topic_exists(room_id, HOME_TOPIC_ID));
        second
            .ensure_home_topic(room_id)
            .expect("the ratchet-diverged double-fire adopts the existing topic");
        assert!(
            second.topic_exists(room_id, HOME_TOPIC_ID),
            "the stale writer adopts its own earlier home topic locally"
        );

        assert_eq!(
            room_application_entry_count(&first, room_id),
            2,
            "only the chat message and the single home topic entry exist"
        );
    }
}
