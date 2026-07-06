//! Shared runtime delivery types and helpers.
//!
//! These request/record types and pure helpers form the vocabulary between
//! the runtime client, the HTTP route layer, and the CLI. They were extracted
//! from the retired in-memory `finitechat-engine` delivery service when the
//! Finite Chat HTTP path became the only delivery implementation.

use crate::{
    AccountId, ApplicationDeliveryPolicy, ConversationId, DeviceId, DeviceRef, Epoch,
    FiniteEnvelope, IdempotencyKey, KeyPackageHash, KeyPackageId, KeyPackageRef, KeyPackageState,
    LeaseToken, LogEntryKind, MAX_ACCOUNT_DEVICES_PER_ROOM, MAX_ACCOUNT_ROOM_DISCOVERY_RESULTS,
    MAX_ENVELOPE_PAYLOAD_BYTES, MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS,
    MAX_KEY_PACKAGE_PAYLOAD_BYTES, MAX_OBJECT_ID_BYTES, MAX_STAGED_WELCOMES_PER_COMMIT,
    MAX_SYNC_PAGE_ENTRIES, MembershipDeltaError, MembershipDeltaV1, MessageId, MlsGroupId,
    ProtocolLimitError, RoomId, RoomLogEntry, RoomStatus, Seq, StagedWelcomeV1, WelcomeId,
    WelcomeState, validate_bytes_len, validate_bytes_non_empty, validate_idempotency_key,
    validate_item_count, validate_mls_group_id, validate_room_id, validate_string_bytes,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceMembership {
    pub device: DeviceRef,
    pub intervals: Vec<MembershipInterval>,
}

impl DeviceMembership {
    pub fn key(device: &DeviceRef) -> String {
        device_membership_key(device)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipInterval {
    pub start_seq: Seq,
    pub end_seq: Option<Seq>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyPackageInventory {
    pub owner: DeviceRef,
    pub available: u32,
    pub leased: u32,
}

impl KeyPackageInventory {
    pub fn unconsumed(&self) -> u64 {
        u64::from(self.available) + u64::from(self.leased)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WelcomeRecord {
    pub welcome_id: WelcomeId,
    pub room_id: RoomId,
    pub commit_seq: Seq,
    pub recipient: DeviceRef,
    pub sender: DeviceRef,
    pub key_package_id: KeyPackageId,
    pub join_epoch: Epoch,
    pub state: WelcomeState,
    pub lease_token: Option<LeaseToken>,
    pub welcome_payload: Vec<u8>,
    pub ratchet_tree_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateRoomRequest {
    pub room_id: RoomId,
    pub mls_group_id: MlsGroupId,
    pub creator: DeviceRef,
    #[serde(default)]
    pub protocol: crate::RoomProtocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadKeyPackageRequest {
    pub key_package_id: KeyPackageId,
    pub owner: DeviceRef,
    pub key_package_ref: KeyPackageRef,
    pub key_package_hash: KeyPackageHash,
    pub key_package_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendEventRequest {
    pub room_id: RoomId,
    pub sender: DeviceRef,
    pub envelope: FiniteEnvelope,
    pub idempotency_key: IdempotencyKey,
    #[serde(default)]
    pub timestamp_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendApplicationEventRequest {
    pub event: AppendEventRequest,
    pub delivery_policy: ApplicationDeliveryPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationDeliveryEffect {
    pub room_id: RoomId,
    pub seq: Seq,
    pub message_id: MessageId,
    pub sender: DeviceRef,
    pub delivery_policy: ApplicationDeliveryPolicy,
}

impl ApplicationDeliveryEffect {
    pub fn creates_push(&self) -> bool {
        self.delivery_policy.creates_push()
    }

    pub fn creates_unread(&self) -> bool {
        self.delivery_policy.creates_unread()
    }

    pub fn creates_command_inbox_work(&self) -> bool {
        self.delivery_policy.creates_command_inbox_work()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendEphemeralActivityRequest {
    pub room_id: RoomId,
    pub mls_group_id: MlsGroupId,
    pub epoch: Epoch,
    pub sender: DeviceRef,
    pub conversation_id: Option<ConversationId>,
    pub payload: Vec<u8>,
    pub received_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EphemeralActivityAccepted {
    pub route_key: String,
    pub cached_events_for_route: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EphemeralActivityRecord {
    pub room_id: RoomId,
    pub mls_group_id: MlsGroupId,
    pub epoch: Epoch,
    pub sender: DeviceRef,
    pub conversation_id: Option<ConversationId>,
    pub payload: Vec<u8>,
    pub received_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitCommitRequest {
    pub room_id: RoomId,
    pub sender: DeviceRef,
    pub expected_epoch: Epoch,
    pub envelope: FiniteEnvelope,
    pub membership_delta: MembershipDeltaV1,
    #[serde(default)]
    pub staged_welcomes: Vec<StagedWelcomeV1>,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimKeyPackageResult {
    pub key_package_id: KeyPackageId,
    pub owner: DeviceRef,
    pub key_package_ref: KeyPackageRef,
    pub key_package_hash: KeyPackageHash,
    pub key_package_payload: Vec<u8>,
    pub lease_token: LeaseToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitAccepted {
    pub seq: Seq,
    pub message_id: MessageId,
    pub released_welcomes: Vec<WelcomeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventAccepted {
    pub seq: Seq,
    pub message_id: MessageId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncEventsPage {
    pub entries: Vec<RoomLogEntry>,
    pub next_after_seq: Seq,
    pub has_more: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSyncProjection {
    room_id: Option<RoomId>,
    server_cursor: Seq,
    highest_stream_hint: Seq,
    applied_message_ids: Vec<MessageId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncProjectionApplyResult {
    pub applied_entries: u32,
    pub server_cursor: Seq,
    pub needs_more_pull: bool,
}

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SyncProjectionError {
    #[error("sync projection room mismatch: expected {expected}, actual {actual}")]
    RoomMismatch { expected: RoomId, actual: RoomId },
    #[error("sync page cursor {next_after_seq} is behind projection cursor {server_cursor}")]
    CursorRewind {
        server_cursor: Seq,
        next_after_seq: Seq,
    },
    #[error("sync page entry {entry_seq} is not after projection cursor {server_cursor}")]
    EntryAtOrBeforeCursor { server_cursor: Seq, entry_seq: Seq },
    #[error("sync page entry order regressed from {previous_seq} to {entry_seq}")]
    EntryOrderRegression { previous_seq: Seq, entry_seq: Seq },
    #[error("sync page entry {entry_seq} is beyond page cursor {next_after_seq}")]
    EntryBeyondPageCursor { entry_seq: Seq, next_after_seq: Seq },
    #[error("sync page entry room mismatch: expected {expected}, actual {actual}")]
    EntryRoomMismatch { expected: RoomId, actual: RoomId },
    #[error(transparent)]
    ProtocolLimit(#[from] ProtocolLimitError),
}

impl RoomSyncProjection {
    pub fn server_cursor(&self) -> Seq {
        self.server_cursor
    }

    pub fn highest_stream_hint(&self) -> Seq {
        self.highest_stream_hint
    }

    pub fn needs_pull(&self) -> bool {
        self.highest_stream_hint > self.server_cursor
    }

    pub fn applied_message_ids(&self) -> &[MessageId] {
        &self.applied_message_ids
    }

    pub fn observe_stream_hint(
        &mut self,
        room_id: &str,
        seq: Seq,
    ) -> Result<bool, SyncProjectionError> {
        self.ensure_room(room_id)?;
        if seq > self.highest_stream_hint {
            self.highest_stream_hint = seq;
        }
        assert!(self.highest_stream_hint >= self.server_cursor);
        Ok(self.needs_pull())
    }

    pub fn apply_page(
        &mut self,
        room_id: &str,
        page: &SyncEventsPage,
    ) -> Result<SyncProjectionApplyResult, SyncProjectionError> {
        self.ensure_room(room_id)?;
        validate_item_count(
            "sync_page.entries",
            page.entries.len(),
            MAX_SYNC_PAGE_ENTRIES,
        )?;
        if page.next_after_seq < self.server_cursor {
            return Err(SyncProjectionError::CursorRewind {
                server_cursor: self.server_cursor,
                next_after_seq: page.next_after_seq,
            });
        }

        let mut previous_seq = self.server_cursor;
        for entry in &page.entries {
            entry.envelope.validate_limits()?;
            if entry.room_id != room_id {
                return Err(SyncProjectionError::EntryRoomMismatch {
                    expected: room_id.to_string(),
                    actual: entry.room_id.clone(),
                });
            }
            if entry.seq <= self.server_cursor {
                return Err(SyncProjectionError::EntryAtOrBeforeCursor {
                    server_cursor: self.server_cursor,
                    entry_seq: entry.seq,
                });
            }
            if entry.seq <= previous_seq {
                return Err(SyncProjectionError::EntryOrderRegression {
                    previous_seq,
                    entry_seq: entry.seq,
                });
            }
            if entry.seq > page.next_after_seq {
                return Err(SyncProjectionError::EntryBeyondPageCursor {
                    entry_seq: entry.seq,
                    next_after_seq: page.next_after_seq,
                });
            }
            previous_seq = entry.seq;
        }

        for entry in &page.entries {
            self.applied_message_ids.push(entry.message_id.clone());
        }
        self.server_cursor = page.next_after_seq;
        if self.highest_stream_hint < self.server_cursor {
            self.highest_stream_hint = self.server_cursor;
        }
        assert!(self.highest_stream_hint >= self.server_cursor);
        Ok(SyncProjectionApplyResult {
            applied_entries: page.entries.len() as u32,
            server_cursor: self.server_cursor,
            needs_more_pull: page.has_more || self.needs_pull(),
        })
    }

    fn ensure_room(&mut self, room_id: &str) -> Result<(), SyncProjectionError> {
        validate_room_id(room_id)?;
        match &self.room_id {
            Some(existing) if existing != room_id => Err(SyncProjectionError::RoomMismatch {
                expected: existing.clone(),
                actual: room_id.to_string(),
            }),
            Some(_) => Ok(()),
            None => {
                self.room_id = Some(room_id.to_string());
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListAccountRoomsRequest {
    pub account_id: AccountId,
    pub after_room_id: Option<RoomId>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRoomDevice {
    pub device: DeviceRef,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRoomRecord {
    pub room_id: RoomId,
    pub mls_group_id: MlsGroupId,
    pub current_epoch: Epoch,
    pub last_seq: Seq,
    pub status: RoomStatus,
    pub devices: Vec<AccountRoomDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListAccountRoomsPage {
    pub rooms: Vec<AccountRoomRecord>,
    pub next_after_room_id: Option<RoomId>,
    pub has_more: bool,
}

impl CreateRoomRequest {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_room_id(&self.room_id)?;
        validate_mls_group_id(&self.mls_group_id)?;
        self.creator.validate_limits()?;
        Ok(())
    }
}

impl ListAccountRoomsRequest {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_string_bytes("account_id", &self.account_id, crate::MAX_ACCOUNT_ID_BYTES)?;
        if let Some(after_room_id) = &self.after_room_id {
            validate_room_id(after_room_id)?;
        }
        crate::validate_item_count(
            "account_room_discovery.limit",
            self.limit as usize,
            MAX_ACCOUNT_ROOM_DISCOVERY_RESULTS,
        )?;
        validate_bytes_non_empty("account_room_discovery.limit", self.limit as usize)?;
        Ok(())
    }
}

impl AccountRoomDevice {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        self.device.validate_limits()?;
        Ok(())
    }
}

impl AccountRoomRecord {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_room_id(&self.room_id)?;
        validate_mls_group_id(&self.mls_group_id)?;
        crate::validate_item_count(
            "account_room.devices",
            self.devices.len(),
            MAX_ACCOUNT_DEVICES_PER_ROOM,
        )?;
        validate_bytes_non_empty("account_room.devices", self.devices.len())?;
        for device in &self.devices {
            device.validate_limits()?;
        }
        Ok(())
    }
}

impl ListAccountRoomsPage {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        crate::validate_item_count(
            "account_room_discovery.rooms",
            self.rooms.len(),
            MAX_ACCOUNT_ROOM_DISCOVERY_RESULTS,
        )?;
        for room in &self.rooms {
            room.validate_limits()?;
        }
        if let Some(next_after_room_id) = &self.next_after_room_id {
            validate_room_id(next_after_room_id)?;
        }
        Ok(())
    }
}

impl UploadKeyPackageRequest {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_string_bytes("key_package_id", &self.key_package_id, MAX_OBJECT_ID_BYTES)?;
        self.owner.validate_limits()?;
        validate_string_bytes(
            "key_package_ref",
            &self.key_package_ref,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_string_bytes(
            "key_package_hash",
            &self.key_package_hash,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("key_package_payload", self.key_package_payload.len())?;
        validate_bytes_len(
            "key_package_payload",
            self.key_package_payload.len(),
            MAX_KEY_PACKAGE_PAYLOAD_BYTES,
        )?;
        Ok(())
    }
}

impl AppendEventRequest {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_room_id(&self.room_id)?;
        self.sender.validate_limits()?;
        self.envelope.validate_limits()?;
        validate_idempotency_key(&self.idempotency_key)?;
        Ok(())
    }
}

impl AppendApplicationEventRequest {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        self.event.validate_limits()
    }
}

impl AppendEphemeralActivityRequest {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_room_id(&self.room_id)?;
        validate_mls_group_id(&self.mls_group_id)?;
        self.sender.validate_limits()?;
        if let Some(conversation_id) = &self.conversation_id {
            validate_string_bytes("conversation_id", conversation_id, MAX_OBJECT_ID_BYTES)?;
        }
        validate_bytes_non_empty("ephemeral_activity.payload", self.payload.len())?;
        validate_bytes_len(
            "ephemeral_activity.payload",
            self.payload.len(),
            MAX_ENVELOPE_PAYLOAD_BYTES,
        )?;
        Ok(())
    }
}

impl SubmitCommitRequest {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_room_id(&self.room_id)?;
        self.sender.validate_limits()?;
        self.envelope.validate_limits()?;
        self.membership_delta.validate_limits()?;
        crate::validate_item_count(
            "staged_welcomes",
            self.staged_welcomes.len(),
            MAX_STAGED_WELCOMES_PER_COMMIT,
        )?;
        for staged_welcome in &self.staged_welcomes {
            staged_welcome.validate_limits()?;
        }
        validate_idempotency_key(&self.idempotency_key)?;
        Ok(())
    }
}

pub type LinkSessionId = String;

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EngineError {
    #[error("room already exists: {0}")]
    RoomAlreadyExists(RoomId),
    #[error("room not found: {0}")]
    RoomNotFound(RoomId),
    #[error("room is not open")]
    RoomNotOpen,
    #[error("key package already exists: {0}")]
    KeyPackageAlreadyExists(KeyPackageId),
    #[error("key package not found: {0}")]
    KeyPackageNotFound(KeyPackageId),
    #[error("key package {key_package_id} is {state:?}")]
    KeyPackageUnavailable {
        key_package_id: KeyPackageId,
        state: KeyPackageState,
    },
    #[error(
        "key package inventory is full for {owner:?}: {available} available and {leased} leased, max {max}"
    )]
    KeyPackageInventoryFull {
        owner: DeviceRef,
        available: u32,
        leased: u32,
        max: u32,
    },
    #[error("key package owner mismatch: {0}")]
    KeyPackageOwnerMismatch(KeyPackageId),
    #[error("key package ref or hash mismatch: {0}")]
    KeyPackageRefMismatch(KeyPackageId),
    #[error("duplicate key package in commit: {0}")]
    DuplicateKeyPackage(KeyPackageId),
    #[error("device is already current or pending in room: {0:?}")]
    DeviceAlreadyInRoom(DeviceRef),
    #[error("device not found: {0:?}")]
    DeviceNotFound(DeviceRef),
    #[error("device is revoked: {0:?}")]
    DeviceRevoked(DeviceRef),
    #[error("duplicate message id in room log: {0}")]
    DuplicateMessageId(MessageId),
    #[error("welcome not found: {0}")]
    WelcomeNotFound(WelcomeId),
    #[error("welcome already exists: {0}")]
    WelcomeAlreadyExists(WelcomeId),
    #[error("welcome is not claimed: {0}")]
    WelcomeNotClaimed(WelcomeId),
    #[error("duplicate welcome id in commit: {0}")]
    DuplicateWelcomeId(WelcomeId),
    #[error("commit add is missing staged Welcome bytes: {0}")]
    MissingStagedWelcome(WelcomeId),
    #[error("staged Welcome does not match any commit add: {0}")]
    UnexpectedStagedWelcome(WelcomeId),
    #[error("wrong epoch: expected {expected}, actual {actual}")]
    WrongEpoch { expected: Epoch, actual: Epoch },
    #[error("wrong envelope kind: expected {expected:?}, actual {actual:?}")]
    WrongEnvelopeKind {
        expected: LogEntryKind,
        actual: LogEntryKind,
    },
    #[error("envelope room does not match request")]
    EnvelopeRoomMismatch,
    #[error("envelope MLS group does not match room")]
    EnvelopeGroupMismatch,
    #[error("envelope sender does not match request")]
    EnvelopeSenderMismatch,
    #[error("sender is not active: {0:?}")]
    SenderNotActive(DeviceRef),
    #[error("ephemeral activity expiry must be after receipt")]
    EphemeralActivityAlreadyExpired,
    #[error("ephemeral activity expiry window {actual_millis}ms exceeds max {max_millis}ms")]
    EphemeralActivityExpiryTooLong { max_millis: u64, actual_millis: u64 },
    #[error("reporter was not a member for offending seq: {0:?}")]
    ReporterNotInInterval(DeviceRef),
    #[error("conflicting idempotency key")]
    ConflictingIdempotencyKey,
    #[error("link session already exists: {0}")]
    LinkSessionAlreadyExists(LinkSessionId),
    #[error("link session not found: {0}")]
    LinkSessionNotFound(LinkSessionId),
    #[error("link session has a conflicting payload")]
    LinkSessionConflict,
    #[error("link session is closed")]
    LinkSessionClosed,
    #[error("link session is not ready")]
    LinkSessionNotReady,
    #[error("bad link session claim token")]
    BadLinkSessionClaimToken,
    #[error("runtime worker counter overflow")]
    RuntimeCounterOverflow,
    #[error("direct room cannot add third account: {0}")]
    DirectRoomThirdAccount(AccountId),
    #[error(transparent)]
    ProtocolLimit(#[from] ProtocolLimitError),
    #[error(transparent)]
    MembershipDelta(#[from] MembershipDeltaError),
    #[error("json serialization failed: {0}")]
    Json(String),
}

impl From<serde_json::Error> for EngineError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error.to_string())
    }
}

pub fn staged_welcomes_by_id<'a>(
    delta: &MembershipDeltaV1,
    staged_welcomes: &'a [StagedWelcomeV1],
) -> Result<BTreeMap<WelcomeId, &'a StagedWelcomeV1>, EngineError> {
    crate::validate_item_count(
        "staged_welcomes",
        staged_welcomes.len(),
        MAX_STAGED_WELCOMES_PER_COMMIT,
    )?;

    let mut by_id = BTreeMap::new();
    for staged_welcome in staged_welcomes {
        staged_welcome.validate_limits()?;
        if by_id
            .insert(staged_welcome.welcome_id.clone(), staged_welcome)
            .is_some()
        {
            return Err(EngineError::DuplicateWelcomeId(
                staged_welcome.welcome_id.clone(),
            ));
        }
    }

    let mut expected_ids = BTreeSet::new();
    for add in &delta.adds {
        if !expected_ids.insert(add.welcome_id.clone()) {
            return Err(EngineError::DuplicateWelcomeId(add.welcome_id.clone()));
        }
        if !by_id.contains_key(&add.welcome_id) {
            return Err(EngineError::MissingStagedWelcome(add.welcome_id.clone()));
        }
    }

    for welcome_id in by_id.keys() {
        if !expected_ids.contains(welcome_id) {
            return Err(EngineError::UnexpectedStagedWelcome(welcome_id.clone()));
        }
    }

    debug_assert_eq!(by_id.len(), expected_ids.len());
    debug_assert!(
        delta
            .adds
            .iter()
            .all(|add| by_id.contains_key(&add.welcome_id))
    );
    Ok(by_id)
}

fn length_prefixed(value: &str) -> String {
    format!("{}:{value}", value.len())
}

fn device_membership_key(device: &DeviceRef) -> String {
    format!(
        "{}\u{1f}{}",
        length_prefixed(&device.account_id),
        length_prefixed(&device.device_id)
    )
}

pub fn validate_activity_expiry(
    received_at_ms: u64,
    expires_at_ms: u64,
) -> Result<(), EngineError> {
    if expires_at_ms <= received_at_ms {
        return Err(EngineError::EphemeralActivityAlreadyExpired);
    }
    let window = expires_at_ms - received_at_ms;
    if window > MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS {
        return Err(EngineError::EphemeralActivityExpiryTooLong {
            max_millis: MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS,
            actual_millis: window,
        });
    }
    Ok(())
}

pub fn ephemeral_activity_route_key(
    room_id: &str,
    conversation_id: Option<&str>,
    sender: &DeviceRef,
) -> String {
    let conversation = conversation_id.unwrap_or("");
    format!(
        "{}|{}|{}|{}",
        length_prefixed(room_id),
        length_prefixed(conversation),
        length_prefixed(&sender.account_id),
        length_prefixed(&sender.device_id)
    )
}

pub fn lease_token_for(id: &str, device: &DeviceRef) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"finitechat-lease-v1");
    hasher.update(id.as_bytes());
    hasher.update(device.account_id.as_bytes());
    hasher.update(device.device_id.as_bytes());
    hex_lower(&hasher.finalize())
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

pub fn device(account_id: impl Into<AccountId>, device_id: impl Into<DeviceId>) -> DeviceRef {
    DeviceRef {
        account_id: account_id.into(),
        device_id: device_id.into(),
    }
}

pub fn envelope(
    room_id: impl Into<RoomId>,
    group_id: impl Into<MlsGroupId>,
    sender: DeviceRef,
    epoch: Epoch,
    kind: LogEntryKind,
    payload: impl Into<Vec<u8>>,
) -> FiniteEnvelope {
    FiniteEnvelope {
        room_id: room_id.into(),
        mls_group_id: group_id.into(),
        epoch,
        sender,
        kind,
        payload: payload.into(),
    }
}
