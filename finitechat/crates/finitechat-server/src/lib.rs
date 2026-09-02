use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use finitechat_delivery::{
    HttpDeliveryLimits, HttpKeyPackageId, HttpSequence, HttpServerError, MAX_HTTP_SYNC_PAGE_ENTRIES,
};
pub use finitechat_http::{
    AckWelcomeRequest, AckWelcomeResponse,
    ApplicationEffectCountsResponse, ApplicationEffectRequest, BootstrapAccountRoomRequest,
    BootstrapAccountRoomResponse, ClaimKeyPackageForAccountRequest, ClaimKeyPackageRequest,
    ClaimKeyPackagesRequest, ClaimWelcomesRequest,
    CreatePairingSessionRequest, DeviceLivenessRecord, ErrorResponse, ExpireKeyPackageLeaseRequest,
    ExpireKeyPackageLeaseResponse, ExpirePairingSessionRequest, ExpirePairingSessionResponse,
    FINITECHAT_SERVER_CONTRACT_VERSION,
    FiniteAccountRoomCommitProjection, GetDeviceLivenessRequest, GetDeviceLivenessResponse,
    GetEphemeralActivitiesRequest, GetEphemeralActivitiesResponse,
    GetKeyPackageAvailabilityRequest, GetKeyPackageAvailabilityResponse, GetNostrProfilesRequest,
    GetNostrProfilesResponse, GetPairingSessionRequest, GroupSyncRequest, HealthResponse,
    HttpApplicationDeliveryEffect, HttpClaimedWelcome, HttpKeyPackageClaim,
    HttpKeyPackageInventory, HttpNipAbSourceDescriptorV1, HttpPairingEventRecord,
    HttpPairingSessionRecord, HttpPairingSessionState, InboxSyncRequest,
    KeyPackageAvailabilityEntry, KeyPackageInventoryRequest, LeaveRoomRequest, LeaveRoomResponse,
    ListAccountRoomDirectoryRequest, ListAccountRoomDirectoryResponse, NostrProfileCacheEntry,
    NostrProfileRecord, ObserveDeviceLivenessRequest, PublishKeyPackageResponse,
    PublishMessageRequest, PublishPairingCompleteRequest, PublishPairingOfferRequest,
    PublishPairingResponseRequest, PutNostrProfileRequest, PutNostrProfileResponse,
    ReportInvalidCommitRequest, ReportInvalidCommitResponse, RevokeDeviceRequest,
    RevokeDeviceResponse, SaveAccountRoomRequest, SaveAccountRoomResponse, SyncHintEvent,
    SyncStreamRequest, SyncWaitRequest, SyncWaitResponse, UpdateRoomAdminsRequest,
    UpdateRoomAdminsResponse,
};
use finitechat_proto::{DeviceRef, MAX_ATTACHMENT_CIPHERTEXT_BYTES, RoomStatus};
use finitechat_transport::{GroupId, MemberId, MessageId};
use thiserror::Error;

pub(crate) const MAX_HTTP_ACCOUNT_ROOM_ID_BYTES: usize = 128;
pub(crate) const PAIRING_PROTOCOL_VERSION: u16 = 1;
pub(crate) const PAIRING_EVENT_KIND: u16 = 24_134;
pub(crate) const PAIRING_SESSION_TTL_SECONDS: u64 = 120;
pub(crate) const MAX_PAIRING_EVENT_BYTES: u32 = 96 * 1024;
pub(crate) const MAX_PAIRING_EVENTS: usize = 8;
pub(crate) const MAX_HTTP_BLOB_UPLOAD_BODY_BYTES: usize = MAX_ATTACHMENT_CIPHERTEXT_BYTES as usize;
pub(crate) const MAX_KEY_PACKAGE_AVAILABILITY_BATCH: usize = MAX_HTTP_SYNC_PAGE_ENTRIES;
pub(crate) const MAX_NOSTR_PROFILE_BATCH: usize = 64;
pub(crate) const MAX_NOSTR_PROFILE_NAME_BYTES: usize = 128;
pub(crate) const MAX_NOSTR_PROFILE_ABOUT_BYTES: usize = 4 * 1024;
pub(crate) const MAX_NOSTR_PROFILE_PICTURE_BYTES: usize = 2 * 1024;
pub(crate) const MAX_NOSTR_PROFILE_METADATA_JSON_BYTES: usize = 16 * 1024;
pub(crate) const MAX_PUBLIC_IMAGE_BLOB_BYTES: usize = 8 * 1024 * 1024;

/// Capacity limits for the durable finite chat server.
///
/// The upstream defaults are sized for tests. These are sized for the current
/// product phase (hundreds of active users, dozens of long chats each); they
/// must be applied before op-log replay so reopening a large server never
/// trips a smaller cap than the one it was written under.
/// How many accepted operations may accumulate before the durable state
/// snapshot refreshes. Startup replays at most this many ops on top of the
/// snapshot.
pub(crate) const SNAPSHOT_INTERVAL_OPS: u64 = 4_096;
/// zstd level for durable-state snapshots: the default level compresses the
/// JSON several-fold at hundreds of MB/s, which is what bounds how long the
/// background snapshot thread runs.
pub(crate) const SNAPSHOT_ZSTD_LEVEL: i32 = 3;

pub fn finite_delivery_limits() -> HttpDeliveryLimits {
    HttpDeliveryLimits {
        max_groups: 65_536,
        max_recipient_inboxes: 65_536,
        max_queue_entries_per_route: 262_144,
        max_key_packages_per_account: 4_096,
    }
}

mod auth;
/// One-time op-log fold of a pre-cutover database onto the normalized
/// engine, plus the minimal legacy READER that feeds it. Transitional:
/// PR 2 (`cleanup/chat-store-delete-old`) deletes this module wholesale
/// before 2026-09-25.
mod cutover;
mod projections;
mod routes;
mod state;
// The normalized SQLite delivery engine — the only engine. There is no
// runtime flag and no second engine: a durable boot either folds a
// pre-cutover database (marker-gated, see `cutover`) or starts fresh.
mod store;
mod validate;

pub use cutover::{RollbackCheck, rollback_check};
pub use routes::http_router;
pub use state::{
    DEFAULT_RATE_LIMIT_PER_WINDOW, DEFAULT_RATE_LIMIT_WINDOW_SECONDS, HttpServerState,
    WelcomeClaimState,
};

/// Date the chat store swap cutover (PR 1) opened the rollback window. The
/// boot banner, `scripts/finite-status`'s amber line, and the dated
/// deletion-deadline test all key off dates in this module; PR 2 deletes
/// all of them.
pub const CHAT_ENGINE_ROLLOUT_SINCE: &str = "2026-08-31";

/// Print the transitional rollback-window banner on stderr. Every durable
/// server boot prints this while the legacy reader/fold code is still
/// compiled, so an operator ssh-ing onto a host cannot miss that the
/// database shape is mid-swap and the deletion PR is pending.
pub fn print_engine_rollout_banner() {
    eprintln!(
        "FINITECHAT SERVER — ROLLBACK WINDOW OPEN (since {CHAT_ENGINE_ROLLOUT_SINCE})\n\
         The normalized delivery engine is the only engine; the first boot\n\
         on a pre-cutover database runs the one-time op-log fold. There is\n\
         no engine flag: rolling the deploy back does NOT un-fold — restore\n\
         the pre-fold backup only if `finitechat-server rollback-check\n\
         --sqlite PATH` passes; otherwise roll forward (the restore is\n\
         refused once any post-fold write exists). The legacy reader and\n\
         fold code are deleted by cleanup/chat-store-delete-old before\n\
         2026-09-25."
    );
}

#[derive(Debug, Error)]
pub enum HttpServerConfigurationError {
    #[error("invalid Finite Chat public URL: {reason}")]
    InvalidPublicUrl { reason: String },
}

#[derive(Debug, Error)]
pub enum DurableStoreError {
    #[error("SQLite delivery store error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("delivery store JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("delivery store I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("persisted delivery operation failed replay: {0}")]
    Replay(#[from] HttpServerError),
    #[error("persisted blob object is corrupt: {sha256}")]
    BlobObjectCorrupt { sha256: String },
    #[error(
        "legacy uncompressed http_state_snapshots row at op {last_op_seq} has no v2 successor; \
         refusing to boot: the op log may have been pruned to that row's horizon, so silent \
         replay from op zero could discard history. Boot a v2-snapshot-writing build once to \
         mint a successor snapshot, or restore from backup."
    )]
    LegacySnapshotWithoutV2Successor { last_op_seq: i64 },
    #[error(
        "op-log fold assertion failed, nothing was committed: {details}. The fold rolled back; \
         boot refuses to switch engines on state it could not verify. Recovery: fix the \
         underlying store (restore from backup) or keep the legacy engine."
    )]
    FoldAssertionFailed { details: String },
    #[error(
        "room-state checkpoint diverges from the delivery entries: {details}. Boot refuses to \
         serve a projection it cannot re-derive. Recovery: restore the checkpoint from backup, \
         or re-derive room state from full history (ADR 0003) on a scratch copy."
    )]
    CheckpointDivergence { details: String },
}

#[derive(Debug)]
pub enum ServerHttpError {
    Delivery(HttpServerError),
    Unauthorized {
        reason: String,
    },
    IdempotencyConflict {
        idempotency_key: String,
    },
    InvalidIdempotencyKey,
    InvalidKeyPackageClaimBatch {
        actual: usize,
        max: usize,
    },
    InvalidKeyPackageLeaseRequest {
        reason: String,
    },
    InvalidDeviceRequest {
        reason: String,
    },
    DeviceRevoked {
        device: DeviceRef,
    },
    InvalidDeviceLivenessRequest {
        reason: String,
    },
    InvalidNostrProfileRequest {
        reason: String,
    },
    InvalidNostrProfileBatch {
        actual: usize,
        max: usize,
    },
    InvalidKeyPackageAvailabilityRequest {
        reason: String,
    },
    InvalidKeyPackageAvailabilityBatch {
        actual: usize,
        max: usize,
    },
    DeviceNotActive {
        device: DeviceRef,
    },
    DuplicateKeyPackageClaimOwner {
        owner: MemberId,
    },
    InventoryConflict {
        key_package_id: HttpKeyPackageId,
    },
    KeyPackageInventoryCountOverflow {
        field: &'static str,
        value: usize,
    },
    CounterOverflow,
    InvalidCommitRequest {
        reason: String,
    },
    InvalidRawCommitImport {
        room_id: String,
        reason: String,
    },
    InvalidEventRequest {
        reason: String,
    },
    DuplicateMessageId {
        message_id: MessageId,
    },
    InvalidActivityRequest {
        reason: String,
    },
    SenderNotActive {
        sender: DeviceRef,
    },
    CommitAuthorityRequired {
        sender: DeviceRef,
    },
    InvalidAdminChange {
        reason: String,
    },
    UnsupportedProtocolVersion {
        requested: u32,
        min: u32,
        max: u32,
    },
    InvalidRepairReport {
        reason: String,
    },
    ReporterNotInInterval {
        reporter: DeviceRef,
        offending_seq: HttpSequence,
    },
    RoomNotOpen {
        room_id: String,
        status: RoomStatus,
    },
    InvalidFanoutRequest {
        reason: String,
    },
    FanoutLimitExceeded {
        fanout_id: String,
        actual: usize,
        max: usize,
    },
    FanoutConflict {
        fanout_id: String,
        reason: String,
    },
    FanoutNotFound {
        fanout_id: String,
    },
    FanoutRoomNotFound {
        fanout_id: String,
        room_id: GroupId,
    },
    InvalidPairingSessionRequest {
        reason: String,
    },
    PairingSessionAlreadyExists {
        pairing_session_id: String,
    },
    PairingSessionNotFound {
        pairing_session_id: String,
    },
    PairingSessionConflict {
        pairing_session_id: String,
        reason: String,
    },
    PairingSessionClosed {
        pairing_session_id: String,
    },
    InvalidSyncRequest {
        reason: String,
    },
    InvalidAccountRoomRequest {
        reason: String,
    },
    AccountRoomBootstrapConflict {
        account_id: String,
        room_id: String,
        reason: String,
    },
    DirectRoomConflict {
        room_id: String,
        reason: String,
    },
    ProjectionJson(String),
    InvalidGroupSyncRequest {
        reason: String,
    },
    InvalidGroupSyncLimit {
        actual: usize,
        max: usize,
    },
    RoomMembershipConflict {
        room_id: String,
        reason: String,
    },
    InvalidAccountRoomListLimit {
        actual: usize,
        max: usize,
    },
    InvalidWelcomeClaimLimit {
        actual: usize,
        max: usize,
    },
    Store(DurableStoreError),
    WelcomeNotFound {
        message_id: MessageId,
    },
    InvalidBlobRequest {
        reason: String,
    },
    BlobNotFound {
        sha256: String,
    },
    BlobConflict {
        sha256: String,
    },
}

impl From<HttpServerError> for ServerHttpError {
    fn from(error: HttpServerError) -> Self {
        Self::Delivery(error)
    }
}

impl From<DurableStoreError> for ServerHttpError {
    fn from(error: DurableStoreError) -> Self {
        Self::Store(error)
    }
}

impl IntoResponse for ServerHttpError {
    fn into_response(self) -> Response {
        let (status, kind, error) = match self {
            Self::Delivery(error) => (
                status_for_error(&error),
                kind_for_error(&error).to_owned(),
                error.to_string(),
            ),
            Self::Store(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "delivery_store".to_owned(),
                error.to_string(),
            ),
            Self::Unauthorized { reason } => {
                (StatusCode::UNAUTHORIZED, "unauthorized".to_owned(), reason)
            }
            Self::IdempotencyConflict { idempotency_key } => (
                StatusCode::CONFLICT,
                "idempotency_conflict".to_owned(),
                format!("conflicting request for idempotency key '{idempotency_key}'"),
            ),
            Self::InvalidIdempotencyKey => (
                StatusCode::BAD_REQUEST,
                "invalid_idempotency_key".to_owned(),
                "idempotency key must not be empty".to_owned(),
            ),
            Self::InvalidKeyPackageClaimBatch { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_key_package_claim_batch".to_owned(),
                format!(
                    "KeyPackage claim batch must contain between 1 and {max} owners, got {actual}"
                ),
            ),
            Self::InvalidKeyPackageLeaseRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_key_package_lease_request".to_owned(),
                reason,
            ),
            Self::InvalidDeviceRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_device_request".to_owned(),
                reason,
            ),
            Self::DeviceRevoked { device } => (
                StatusCode::FORBIDDEN,
                "device_revoked".to_owned(),
                format!("device {device:?} is revoked"),
            ),
            Self::InvalidDeviceLivenessRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_device_liveness_request".to_owned(),
                reason,
            ),
            Self::InvalidNostrProfileRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_nostr_profile_request".to_owned(),
                reason,
            ),
            Self::InvalidNostrProfileBatch { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_nostr_profile_batch".to_owned(),
                format!(
                    "Nostr profile batch must contain between 1 and {max} accounts, got {actual}"
                ),
            ),
            Self::InvalidKeyPackageAvailabilityRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_key_package_availability_request".to_owned(),
                reason,
            ),
            Self::InvalidKeyPackageAvailabilityBatch { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_key_package_availability_batch".to_owned(),
                format!(
                    "KeyPackage availability batch must contain between 1 and {max} accounts, got {actual}"
                ),
            ),
            Self::DeviceNotActive { device } => (
                StatusCode::FORBIDDEN,
                "device_not_active".to_owned(),
                format!("device {device:?} is not active in any room"),
            ),
            Self::DuplicateKeyPackageClaimOwner { owner } => (
                StatusCode::BAD_REQUEST,
                "duplicate_key_package_claim_owner".to_owned(),
                format!("KeyPackage claim batch contains duplicate owner {owner:?}"),
            ),
            Self::InventoryConflict { key_package_id } => (
                StatusCode::CONFLICT,
                "key_package_inventory_conflict".to_owned(),
                format!("KeyPackage inventory has a conflicting owner for {key_package_id:?}"),
            ),
            Self::KeyPackageInventoryCountOverflow { field, value } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "key_package_inventory_count_overflow".to_owned(),
                format!("KeyPackage inventory field {field} does not fit in u32: {value}"),
            ),
            Self::CounterOverflow => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "counter_overflow".to_owned(),
                "counter value does not fit in u32".to_owned(),
            ),
            Self::InvalidCommitRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_commit_request".to_owned(),
                reason,
            ),
            Self::InvalidRawCommitImport { room_id, reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_raw_commit_import".to_owned(),
                format!("raw commit import for {room_id} is invalid: {reason}"),
            ),
            Self::InvalidEventRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_event_request".to_owned(),
                reason,
            ),
            Self::DuplicateMessageId { message_id } => (
                StatusCode::CONFLICT,
                "duplicate_message_id".to_owned(),
                format!("duplicate typed event message id {message_id}"),
            ),
            Self::InvalidActivityRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_activity_request".to_owned(),
                reason,
            ),
            Self::SenderNotActive { sender } => (
                StatusCode::FORBIDDEN,
                "sender_not_active".to_owned(),
                format!("sender {sender:?} is not active in the room"),
            ),
            Self::CommitAuthorityRequired { sender } => (
                StatusCode::FORBIDDEN,
                "commit_authority_required".to_owned(),
                format!(
                    "sender {sender:?} must be a room admin to change another account's membership"
                ),
            ),
            Self::InvalidAdminChange { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_admin_change".to_owned(),
                reason,
            ),
            Self::UnsupportedProtocolVersion {
                requested,
                min,
                max,
            } => (
                StatusCode::UPGRADE_REQUIRED,
                "unsupported_protocol_version".to_owned(),
                format!(
                    "room protocol version {requested} is outside the supported range {min}..={max}"
                ),
            ),
            Self::InvalidRepairReport { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_repair_report".to_owned(),
                reason,
            ),
            Self::ReporterNotInInterval {
                reporter,
                offending_seq,
            } => (
                StatusCode::FORBIDDEN,
                "reporter_not_in_interval".to_owned(),
                format!("reporter {reporter:?} was not a member for seq {offending_seq}"),
            ),
            Self::RoomNotOpen { room_id, status } => (
                StatusCode::CONFLICT,
                "room_not_open".to_owned(),
                format!("room {room_id} is {status:?}"),
            ),
            Self::InvalidFanoutRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_fanout_request".to_owned(),
                reason,
            ),
            Self::FanoutLimitExceeded {
                fanout_id,
                actual,
                max,
            } => (
                StatusCode::TOO_MANY_REQUESTS,
                "fanout_limit_exceeded".to_owned(),
                format!("fanout {fanout_id} has {actual} rooms, max {max}"),
            ),
            Self::FanoutConflict { fanout_id, reason } => (
                StatusCode::CONFLICT,
                "fanout_conflict".to_owned(),
                format!("fanout {fanout_id} conflict: {reason}"),
            ),
            Self::FanoutNotFound { fanout_id } => (
                StatusCode::NOT_FOUND,
                "fanout_not_found".to_owned(),
                format!("fanout {fanout_id} was not found"),
            ),
            Self::FanoutRoomNotFound { fanout_id, room_id } => (
                StatusCode::NOT_FOUND,
                "fanout_room_not_found".to_owned(),
                format!("fanout {fanout_id} has no room {room_id:?}"),
            ),
            Self::InvalidPairingSessionRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_pairing_session_request".to_owned(),
                reason,
            ),
            Self::PairingSessionAlreadyExists { pairing_session_id } => (
                StatusCode::CONFLICT,
                "pairing_session_already_exists".to_owned(),
                format!("pairing session {pairing_session_id} already exists"),
            ),
            Self::PairingSessionNotFound { pairing_session_id } => (
                StatusCode::NOT_FOUND,
                "pairing_session_not_found".to_owned(),
                format!("pairing session {pairing_session_id} was not found"),
            ),
            Self::PairingSessionConflict {
                pairing_session_id,
                reason,
            } => (
                StatusCode::CONFLICT,
                "pairing_session_conflict".to_owned(),
                format!("pairing session {pairing_session_id} conflict: {reason}"),
            ),
            Self::PairingSessionClosed { pairing_session_id } => (
                StatusCode::BAD_REQUEST,
                "pairing_session_closed".to_owned(),
                format!("pairing session {pairing_session_id} is closed"),
            ),
            Self::InvalidSyncRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_sync_request".to_owned(),
                reason,
            ),
            Self::InvalidAccountRoomRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_account_room_request".to_owned(),
                reason,
            ),
            Self::AccountRoomBootstrapConflict {
                account_id,
                room_id,
                reason,
            } => (
                StatusCode::CONFLICT,
                "account_room_bootstrap_conflict".to_owned(),
                format!("account-room bootstrap conflict for {account_id}/{room_id}: {reason}"),
            ),
            Self::DirectRoomConflict { room_id, reason } => (
                StatusCode::CONFLICT,
                "direct_room_conflict".to_owned(),
                format!("direct-room conflict for {room_id}: {reason}"),
            ),
            Self::ProjectionJson(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "finite_projection_json".to_owned(),
                error,
            ),
            Self::InvalidGroupSyncRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_group_sync_request".to_owned(),
                reason,
            ),
            Self::InvalidGroupSyncLimit { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_group_sync_limit".to_owned(),
                format!("group sync limit must be between 1 and {max}, got {actual}"),
            ),
            Self::RoomMembershipConflict { room_id, reason } => (
                StatusCode::CONFLICT,
                "room_membership_conflict".to_owned(),
                format!("room-membership projection conflict for {room_id}: {reason}"),
            ),
            Self::InvalidAccountRoomListLimit { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_account_room_list_limit".to_owned(),
                format!("account-room list limit must be between 1 and {max}, got {actual}"),
            ),
            Self::InvalidWelcomeClaimLimit { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_welcome_claim_limit".to_owned(),
                format!("welcome claim limit must be between 1 and {max}, got {actual}"),
            ),
            Self::WelcomeNotFound { message_id } => (
                StatusCode::NOT_FOUND,
                "welcome_not_found".to_owned(),
                format!("welcome {message_id} was not claimed"),
            ),
            Self::InvalidBlobRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_blob_request".to_owned(),
                reason,
            ),
            Self::BlobNotFound { sha256 } => (
                StatusCode::NOT_FOUND,
                "blob_not_found".to_owned(),
                format!("blob object {sha256} was not found"),
            ),
            Self::BlobConflict { sha256 } => (
                StatusCode::CONFLICT,
                "blob_conflict".to_owned(),
                format!("blob object {sha256} already exists with different bytes"),
            ),
        };
        let body = ErrorResponse { kind, error };
        (status, Json(body)).into_response()
    }
}

fn status_for_error(error: &HttpServerError) -> StatusCode {
    match error {
        HttpServerError::ConflictingMessageId { .. }
        | HttpServerError::StaleEpoch { .. }
        | HttpServerError::ConflictingKeyPackage { .. } => StatusCode::CONFLICT,
        HttpServerError::QueueFull { .. }
        | HttpServerError::GroupLimitExceeded { .. }
        | HttpServerError::InboxLimitExceeded { .. }
        | HttpServerError::KeyPackageInventoryFull { .. } => StatusCode::TOO_MANY_REQUESTS,
        HttpServerError::Empty { .. }
        | HttpServerError::TooLarge { .. }
        | HttpServerError::PublishTargetMismatch
        | HttpServerError::InvalidPageLimit { .. } => StatusCode::BAD_REQUEST,
    }
}

fn kind_for_error(error: &HttpServerError) -> &'static str {
    match error {
        HttpServerError::Empty { .. } => "empty",
        HttpServerError::TooLarge { .. } => "too_large",
        HttpServerError::PublishTargetMismatch => "publish_target_mismatch",
        HttpServerError::ConflictingMessageId { .. } => "conflicting_message_id",
        HttpServerError::StaleEpoch { .. } => "stale_epoch",
        HttpServerError::QueueFull { .. } => "queue_full",
        HttpServerError::GroupLimitExceeded { .. } => "group_limit_exceeded",
        HttpServerError::InboxLimitExceeded { .. } => "inbox_limit_exceeded",
        HttpServerError::InvalidPageLimit { .. } => "invalid_page_limit",
        HttpServerError::ConflictingKeyPackage { .. } => "conflicting_key_package",
        HttpServerError::KeyPackageInventoryFull { .. } => "key_package_inventory_full",
    }
}

#[cfg(test)]
mod engine_cutover_tripwires {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// PR 2 (`cleanup/chat-store-delete-old`) must land before this date.
    /// After it, this test fails CI until the transitional reader/fold
    /// module (`src/cutover.rs`) is deleted. Postponing the deletion means
    /// consciously editing this constant in review — that is the point.
    const LEGACY_READER_DELETION_DEADLINE: (i64, u32, u32) = (2026, 9, 25);

    fn deadline_unix_seconds() -> i64 {
        let (year, month, day) = LEGACY_READER_DELETION_DEADLINE;
        // Days since 1970-01-01 for the (year, month, day) Gregorian date
        // (Howard Hinnant's days_from_civil).
        let years = year - if month <= 2 { 1 } else { 0 };
        let era = if years >= 0 { years } else { years - 399 } / 400;
        let year_of_era = years - era * 400;
        let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
        let day_of_era =
            year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year as i64;
        let days = era * 146_097 + day_of_era - 719_468;
        days * 86_400
    }

    #[test]
    fn legacy_reader_module_is_deleted_by_the_cutover_deadline() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("wall clock after the unix epoch")
            .as_secs() as i64;
        if now < deadline_unix_seconds() {
            // Window still open: the transitional reader/fold is allowed to
            // exist.
            return;
        }
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("cutover.rs");
        assert!(
            !source.exists(),
            "the chat store swap deletion deadline ({:?}) has passed and \
             src/cutover.rs (the transitional legacy reader + fold) still exists: \
             land PR 2 (cleanup/chat-store-delete-old) or consciously move \
             LEGACY_READER_DELETION_DEADLINE in this test",
            LEGACY_READER_DELETION_DEADLINE
        );
    }

    #[test]
    fn legacy_engine_is_gone_from_the_source_tree() {
        // Single-deploy rework: the legacy SERVING engine was deleted in
        // PR 1 itself. This tripwire keeps it deleted.
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert!(
            !manifest.join("src").join("legacy_store.rs").exists(),
            "src/legacy_store.rs must not come back: the legacy serving engine \
             was deleted with the single-deploy cutover"
        );
    }

    #[test]
    fn rollback_window_dates_agree_across_the_tripwires() {
        // The boot banner, the fold marker, and finite-status's amber line
        // must all name the same window-open date or the operators stop
        // trusting any of them.
        assert_eq!(super::CHAT_ENGINE_ROLLOUT_SINCE, "2026-08-31");
        assert_eq!(super::cutover::CHAT_ENGINE_CUTOVER_DATE, "2026-08-31");
    }
}
