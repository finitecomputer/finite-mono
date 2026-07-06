use std::error::Error;
use std::fmt;

use finitechat_engine::{
    AppendApplicationEventRequest, AppendEventRequest, CreateRoomRequest, DeliveryService,
    EngineError, EventAccepted, SubmitCommitRequest, UploadKeyPackageRequest, envelope,
};
use finitechat_proto::{
    DecryptedApplicationEventV1, DeviceRef, DurableAppEventKind, LogEntryKind, MembershipAddV1,
    MembershipDeltaV1, ProtocolLimitError, RuntimeStateSnapshotV1, StagedWelcomeV1,
    validate_idempotency_key, validate_mls_group_id, validate_room_id,
};
use finitechat_store::{SqliteDeliveryStore, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiniteChatRoomSeed {
    pub room_id: String,
    pub mls_group_id: String,
    pub user_device: DeviceRef,
    pub runtime_device: DeviceRef,
}

impl FiniteChatRoomSeed {
    pub fn validate(&self) -> Result<(), FiniteChatBridgeError> {
        ensure_non_empty("room_id", &self.room_id)?;
        ensure_non_empty("mls_group_id", &self.mls_group_id)?;
        validate_room_id(&self.room_id)?;
        validate_mls_group_id(&self.mls_group_id)?;
        self.user_device.validate_limits()?;
        self.runtime_device.validate_limits()?;
        if self.user_device == self.runtime_device {
            return Err(FiniteChatBridgeError::SameDeviceForUserAndRuntime);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FiniteChatAppendInput {
    pub room_id: String,
    pub mls_group_id: String,
    pub epoch: u64,
    pub sender: DeviceRef,
    pub conversation_id: Option<String>,
    pub idempotency_key: String,
    pub payload: FiniteChatEventPayload,
}

impl FiniteChatAppendInput {
    pub fn validate(&self) -> Result<(), FiniteChatBridgeError> {
        ensure_non_empty("room_id", &self.room_id)?;
        ensure_non_empty("mls_group_id", &self.mls_group_id)?;
        ensure_non_empty("idempotency_key", &self.idempotency_key)?;
        validate_room_id(&self.room_id)?;
        validate_mls_group_id(&self.mls_group_id)?;
        validate_idempotency_key(&self.idempotency_key)?;
        self.sender.validate_limits()?;
        self.payload.validate()?;
        if self.payload.requires_conversation_id() && self.conversation_id.is_none() {
            return Err(FiniteChatBridgeError::MissingConversationId {
                kind: self.payload.kind_name(),
            });
        }
        if let Some(conversation_id) = &self.conversation_id {
            ensure_non_empty("conversation_id", conversation_id)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "payload_type", content = "payload", rename_all = "snake_case")]
pub enum FiniteChatEventPayload {
    ConversationCreate(FiniteChatConversationPayload),
    ConversationUpdate(FiniteChatConversationPayload),
    ConversationArchive(FiniteChatConversationPayload),
    ChatMessage(FiniteChatMessagePayload),
    ConversationSegmentStart(FiniteChatSegmentStartPayload),
    RuntimeStateSnapshot(RuntimeStateSnapshotV1),
    RuntimeCommandRequest(FiniteChatCommandRequestPayload),
    RuntimeCommandResult(FiniteChatCommandResultPayload),
}

impl FiniteChatEventPayload {
    fn kind(&self) -> DurableAppEventKind {
        match self {
            Self::ConversationCreate(_) => DurableAppEventKind::ConversationCreate,
            Self::ConversationUpdate(_) => DurableAppEventKind::ConversationUpdate,
            Self::ConversationArchive(_) => DurableAppEventKind::ConversationArchive,
            Self::ChatMessage(_) => DurableAppEventKind::ChatMessage,
            Self::ConversationSegmentStart(_) => DurableAppEventKind::ConversationSegmentStart,
            Self::RuntimeStateSnapshot(_) => DurableAppEventKind::RuntimeStateSnapshot,
            Self::RuntimeCommandRequest(_) => DurableAppEventKind::RuntimeCommandRequest,
            Self::RuntimeCommandResult(_) => DurableAppEventKind::RuntimeCommandResult,
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            Self::ConversationCreate(_) => "conversation.create",
            Self::ConversationUpdate(_) => "conversation.update",
            Self::ConversationArchive(_) => "conversation.archive",
            Self::ChatMessage(_) => "chat.message",
            Self::ConversationSegmentStart(_) => "conversation.segment.start",
            Self::RuntimeStateSnapshot(_) => "runtime.state.snapshot",
            Self::RuntimeCommandRequest(_) => "runtime.command.request",
            Self::RuntimeCommandResult(_) => "runtime.command.result",
        }
    }

    fn requires_conversation_id(&self) -> bool {
        matches!(
            self,
            Self::ConversationCreate(_)
                | Self::ConversationUpdate(_)
                | Self::ConversationArchive(_)
                | Self::ChatMessage(_)
                | Self::ConversationSegmentStart(_)
        )
    }

    fn validate(&self) -> Result<(), FiniteChatBridgeError> {
        match self {
            Self::ConversationCreate(payload)
            | Self::ConversationUpdate(payload)
            | Self::ConversationArchive(payload) => payload.validate(),
            Self::ChatMessage(payload) => payload.validate(),
            Self::ConversationSegmentStart(payload) => payload.validate(),
            Self::RuntimeStateSnapshot(snapshot) => {
                ensure_non_empty("runtime_state.state_key", &snapshot.state_key)?;
                ensure_non_empty("runtime_state.schema", &snapshot.schema)?;
                if snapshot.expires_at_ms <= snapshot.observed_at_ms {
                    return Err(FiniteChatBridgeError::InvalidTimeRange {
                        start_field: "runtime_state.observed_at_ms",
                        end_field: "runtime_state.expires_at_ms",
                    });
                }
                snapshot.validate_limits().map_err(Into::into)
            }
            Self::RuntimeCommandRequest(payload) => payload.validate(),
            Self::RuntimeCommandResult(payload) => payload.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiniteChatConversationPayload {
    pub conversation_id: String,
    pub machine_id: String,
    pub project_agent_id: String,
    pub created_by: String,
    pub title: String,
    pub created_at: String,
    pub last_activity_at: String,
    pub message_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
}

impl FiniteChatConversationPayload {
    fn validate(&self) -> Result<(), FiniteChatBridgeError> {
        ensure_non_empty("conversation.conversation_id", &self.conversation_id)?;
        ensure_non_empty("conversation.machine_id", &self.machine_id)?;
        ensure_non_empty("conversation.project_agent_id", &self.project_agent_id)?;
        ensure_non_empty("conversation.created_by", &self.created_by)?;
        ensure_not_blank("conversation.title", &self.title)?;
        ensure_not_blank("conversation.created_at", &self.created_at)?;
        ensure_not_blank("conversation.last_activity_at", &self.last_activity_at)?;
        if let Some(archived_at) = &self.archived_at {
            ensure_not_blank("conversation.archived_at", archived_at)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FiniteChatMessagePayload {
    pub message_id: Option<String>,
    pub sender_type: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub body: String,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl FiniteChatMessagePayload {
    fn validate(&self) -> Result<(), FiniteChatBridgeError> {
        if let Some(message_id) = &self.message_id {
            ensure_non_empty("chat.message.message_id", message_id)?;
        }
        if let Some(sender_type) = &self.sender_type {
            ensure_not_blank("chat.message.sender_type", sender_type)?;
        }
        if let Some(kind) = &self.kind {
            ensure_not_blank("chat.message.kind", kind)?;
        }
        if let Some(status) = &self.status {
            ensure_not_blank("chat.message.status", status)?;
        }
        if let Some(created_at) = &self.created_at {
            ensure_not_blank("chat.message.created_at", created_at)?;
        }
        if let Some(updated_at) = &self.updated_at {
            ensure_not_blank("chat.message.updated_at", updated_at)?;
        }
        if !self.body.trim().is_empty() || metadata_has_attachments(&self.metadata) {
            return Ok(());
        }
        Err(FiniteChatBridgeError::BlankField("chat.message.body"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiniteChatSegmentStartPayload {
    pub segment_id: String,
    pub reason: String,
}

impl FiniteChatSegmentStartPayload {
    fn validate(&self) -> Result<(), FiniteChatBridgeError> {
        ensure_non_empty("conversation.segment_id", &self.segment_id)?;
        ensure_not_blank("conversation.segment_reason", &self.reason)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FiniteChatCommandRequestPayload {
    pub request_id: String,
    pub command: String,
    #[serde(default)]
    pub args: Value,
}

impl FiniteChatCommandRequestPayload {
    fn validate(&self) -> Result<(), FiniteChatBridgeError> {
        ensure_non_empty("runtime.command.request_id", &self.request_id)?;
        ensure_not_blank("runtime.command.command", &self.command)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FiniteChatCommandStatus {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FiniteChatCommandResultPayload {
    pub request_id: String,
    pub status: FiniteChatCommandStatus,
    #[serde(default)]
    pub result: Value,
}

impl FiniteChatCommandResultPayload {
    fn validate(&self) -> Result<(), FiniteChatBridgeError> {
        ensure_non_empty("runtime.command.request_id", &self.request_id)
    }
}

#[derive(Debug)]
pub enum FiniteChatBridgeError {
    EmptyField(&'static str),
    BlankField(&'static str),
    InvalidTimeRange {
        start_field: &'static str,
        end_field: &'static str,
    },
    MissingConversationId {
        kind: &'static str,
    },
    SameDeviceForUserAndRuntime,
    MissingRoomDuringSetup,
    MissingRuntimeKeyPackage,
    Protocol(ProtocolLimitError),
    Engine(EngineError),
    Store(StoreError),
    Json(serde_json::Error),
}

impl fmt::Display for FiniteChatBridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "{field} is empty"),
            Self::BlankField(field) => write!(f, "{field} is blank"),
            Self::InvalidTimeRange {
                start_field,
                end_field,
            } => write!(f, "{end_field} must be greater than {start_field}"),
            Self::MissingConversationId { kind } => {
                write!(f, "{kind} requires a conversation_id")
            }
            Self::SameDeviceForUserAndRuntime => {
                write!(f, "user device and runtime device must be distinct")
            }
            Self::MissingRoomDuringSetup => write!(f, "finitechat room was missing during setup"),
            Self::MissingRuntimeKeyPackage => {
                write!(
                    f,
                    "runtime key package unavailable for finitechat room setup"
                )
            }
            Self::Protocol(error) => write!(f, "finitechat protocol limit failed: {error}"),
            Self::Engine(error) => write!(f, "finitechat engine failed: {error}"),
            Self::Store(error) => write!(f, "finitechat store failed: {error}"),
            Self::Json(error) => write!(f, "finitechat json codec failed: {error}"),
        }
    }
}

impl Error for FiniteChatBridgeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Protocol(error) => Some(error),
            Self::Engine(error) => Some(error),
            Self::Store(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::EmptyField(_)
            | Self::BlankField(_)
            | Self::InvalidTimeRange { .. }
            | Self::MissingConversationId { .. }
            | Self::SameDeviceForUserAndRuntime
            | Self::MissingRoomDuringSetup
            | Self::MissingRuntimeKeyPackage => None,
        }
    }
}

impl From<ProtocolLimitError> for FiniteChatBridgeError {
    fn from(value: ProtocolLimitError) -> Self {
        Self::Protocol(value)
    }
}

impl From<EngineError> for FiniteChatBridgeError {
    fn from(value: EngineError) -> Self {
        Self::Engine(value)
    }
}

impl From<StoreError> for FiniteChatBridgeError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

impl From<serde_json::Error> for FiniteChatBridgeError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub fn finitechat_device(account_id: impl Into<String>, device_id: impl Into<String>) -> DeviceRef {
    DeviceRef {
        account_id: account_id.into(),
        device_id: device_id.into(),
    }
}

pub fn finitechat_protocol_object_id(prefix: &str, raw: &str) -> String {
    const MAX_PROTOCOL_OBJECT_ID_BYTES: usize = finitechat_proto::MAX_OBJECT_ID_BYTES as usize;
    const HASH_HEX_BYTES: usize = 32;
    assert!(!prefix.is_empty());
    assert!(prefix.len() + 1 + HASH_HEX_BYTES <= MAX_PROTOCOL_OBJECT_ID_BYTES);
    let candidate = format!("{prefix}-{raw}");
    if !raw.is_empty() && candidate.len() <= MAX_PROTOCOL_OBJECT_ID_BYTES {
        return candidate;
    }
    let digest = Sha256::digest(raw.as_bytes());
    let digest_hex = hex::encode(digest);
    let id = format!("{prefix}-{}", &digest_hex[..HASH_HEX_BYTES]);
    assert!(!id.is_empty());
    assert!(id.len() <= MAX_PROTOCOL_OBJECT_ID_BYTES);
    id
}

pub fn finitechat_ensure_local_room_ready(
    store: &mut SqliteDeliveryStore,
    seed: &FiniteChatRoomSeed,
) -> Result<(), FiniteChatBridgeError> {
    seed.validate()?;
    if let Some(room) = store.room(&seed.room_id)? {
        if room.device_active_at_head(&seed.user_device)
            && room.device_active_at_head(&seed.runtime_device)
        {
            return Ok(());
        }
        finitechat_ack_runtime_welcomes(store, seed)?;
        let room = store
            .room(&seed.room_id)?
            .ok_or(FiniteChatBridgeError::MissingRoomDuringSetup)?;
        if room.device_active_at_head(&seed.user_device)
            && room.device_active_at_head(&seed.runtime_device)
        {
            return Ok(());
        }
        finitechat_add_runtime_device(store, seed, room.current_epoch)?;
        return Ok(());
    }

    store.create_room(CreateRoomRequest {
        room_id: seed.room_id.clone(),
        mls_group_id: seed.mls_group_id.clone(),
        creator: seed.user_device.clone(),
    })?;
    finitechat_add_runtime_device(store, seed, 0)
}

pub fn finitechat_encode_app_event(
    input: &FiniteChatAppendInput,
) -> Result<Vec<u8>, FiniteChatBridgeError> {
    input.validate()?;
    let event = DecryptedApplicationEventV1 {
        kind: input.payload.kind(),
        conversation_id: input.conversation_id.clone(),
        payload: serde_json::to_vec(&input.payload)?,
    };
    event.validate_limits()?;
    let encoded = serde_json::to_vec(&event)?;
    assert!(!encoded.is_empty());
    Ok(encoded)
}

pub fn finitechat_push_app_event(
    service: &mut DeliveryService,
    input: FiniteChatAppendInput,
) -> Result<EventAccepted, FiniteChatBridgeError> {
    input.validate()?;
    let delivery_policy = input.payload.kind().delivery_policy();
    let encoded = finitechat_encode_app_event(&input)?;
    let accepted = service.append_application_event(AppendApplicationEventRequest {
        event: AppendEventRequest {
            room_id: input.room_id.clone(),
            sender: input.sender.clone(),
            envelope: envelope(
                input.room_id,
                input.mls_group_id,
                input.sender,
                input.epoch,
                LogEntryKind::Application,
                encoded,
            ),
            idempotency_key: input.idempotency_key,
        },
        delivery_policy,
    })?;
    assert!(accepted.seq > 0);
    assert!(!accepted.message_id.is_empty());
    Ok(accepted)
}

pub fn finitechat_push_app_event_to_store(
    store: &mut SqliteDeliveryStore,
    input: FiniteChatAppendInput,
) -> Result<EventAccepted, FiniteChatBridgeError> {
    input.validate()?;
    let delivery_policy = input.payload.kind().delivery_policy();
    let encoded = finitechat_encode_app_event(&input)?;
    let accepted = store.append_application_event(AppendApplicationEventRequest {
        event: AppendEventRequest {
            room_id: input.room_id.clone(),
            sender: input.sender.clone(),
            envelope: envelope(
                input.room_id,
                input.mls_group_id,
                input.sender,
                input.epoch,
                LogEntryKind::Application,
                encoded,
            ),
            idempotency_key: input.idempotency_key,
        },
        delivery_policy,
    })?;
    assert!(accepted.seq > 0);
    assert!(!accepted.message_id.is_empty());
    Ok(accepted)
}

fn finitechat_add_runtime_device(
    store: &mut SqliteDeliveryStore,
    seed: &FiniteChatRoomSeed,
    expected_epoch: u64,
) -> Result<(), FiniteChatBridgeError> {
    assert!(store.room(&seed.room_id)?.is_some());
    store.upload_key_package(UploadKeyPackageRequest {
        key_package_id: finitechat_bridge_id("kp"),
        owner: seed.runtime_device.clone(),
        key_package_ref: finitechat_bridge_id("kp_ref"),
        key_package_hash: finitechat_bridge_id("kp_hash"),
        key_package_payload: b"finitecomputer local mirror runtime key package".to_vec(),
    })?;
    let claimed = store
        .claim_key_package_for_device(&seed.runtime_device)?
        .ok_or(FiniteChatBridgeError::MissingRuntimeKeyPackage)?;
    let commit = envelope(
        seed.room_id.clone(),
        seed.mls_group_id.clone(),
        seed.user_device.clone(),
        expected_epoch,
        LogEntryKind::Commit,
        b"add finitecomputer runtime mirror device".to_vec(),
    );
    let commit_message_id = commit.message_id()?;
    let welcome_id = finitechat_bridge_id("welcome");
    store.submit_commit(SubmitCommitRequest {
        room_id: seed.room_id.clone(),
        sender: seed.user_device.clone(),
        expected_epoch,
        envelope: commit,
        membership_delta: MembershipDeltaV1 {
            base_epoch: expected_epoch,
            post_commit_epoch: expected_epoch.saturating_add(1),
            commit_message_id,
            adds: vec![MembershipAddV1 {
                device: seed.runtime_device.clone(),
                key_package_id: claimed.key_package_id,
                key_package_ref: claimed.key_package_ref,
                key_package_hash: claimed.key_package_hash,
                welcome_id: welcome_id.clone(),
            }],
            removes: vec![],
        },
        staged_welcomes: vec![StagedWelcomeV1 {
            welcome_id,
            welcome_payload: b"finitecomputer local mirror welcome".to_vec(),
            ratchet_tree_payload: b"finitecomputer local mirror ratchet tree".to_vec(),
        }],
        idempotency_key: finitechat_bridge_id("add_runtime"),
    })?;
    finitechat_ack_runtime_welcomes(store, seed)?;
    let room = store
        .room(&seed.room_id)?
        .ok_or(FiniteChatBridgeError::MissingRoomDuringSetup)?;
    assert!(room.device_active_at_head(&seed.user_device));
    assert!(room.device_active_at_head(&seed.runtime_device));
    Ok(())
}

fn finitechat_ack_runtime_welcomes(
    store: &mut SqliteDeliveryStore,
    seed: &FiniteChatRoomSeed,
) -> Result<(), FiniteChatBridgeError> {
    let welcomes = store.claim_welcomes(&seed.runtime_device)?;
    assert!(welcomes.len() <= finitechat_proto::MAX_WELCOME_CLAIMS_PER_REQUEST as usize);
    for welcome in welcomes {
        if welcome.room_id == seed.room_id {
            store.ack_welcome(&welcome.welcome_id, true)?;
        } else {
            store.release_welcome_claim(&welcome.welcome_id)?;
        }
    }
    Ok(())
}

fn finitechat_bridge_id(prefix: &str) -> String {
    assert!(!prefix.is_empty());
    let now = OffsetDateTime::now_utc().unix_timestamp_nanos();
    let random = rand::random::<u64>();
    let id = format!("{prefix}_{now:x}{random:x}");
    assert!(!id.is_empty());
    id
}

fn ensure_non_empty(field: &'static str, value: &str) -> Result<(), FiniteChatBridgeError> {
    if value.is_empty() {
        return Err(FiniteChatBridgeError::EmptyField(field));
    }
    Ok(())
}

fn ensure_not_blank(field: &'static str, value: &str) -> Result<(), FiniteChatBridgeError> {
    ensure_non_empty(field, value)?;
    if value.trim().is_empty() {
        return Err(FiniteChatBridgeError::BlankField(field));
    }
    Ok(())
}

fn metadata_has_attachments(metadata: &Value) -> bool {
    metadata
        .get("attachments")
        .and_then(Value::as_array)
        .is_some_and(|attachments| !attachments.is_empty())
}

#[cfg(test)]
mod tests {
    use finitechat_engine::{
        CreateRoomRequest, SubmitCommitRequest, UploadKeyPackageRequest, envelope,
    };
    use finitechat_proto::{
        ApplicationDeliveryPolicy, LogEntryKind, MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES,
        MembershipAddV1, MembershipDeltaV1, StagedWelcomeV1,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn finitechat_smoke_maps_messages_state_and_commands_without_dashboard_changes() {
        let seed = room_seed();
        let mut service = service_with_active_runtime(&seed);

        let chat = finitechat_push_app_event(
            &mut service,
            FiniteChatAppendInput {
                room_id: seed.room_id.clone(),
                mls_group_id: seed.mls_group_id.clone(),
                epoch: 1,
                sender: seed.user_device.clone(),
                conversation_id: Some("topic-general".to_string()),
                idempotency_key: "chat-message-1".to_string(),
                payload: FiniteChatEventPayload::ChatMessage(FiniteChatMessagePayload {
                    message_id: None,
                    sender_type: None,
                    kind: None,
                    status: None,
                    body: "restart the gateway if Hermes is down".to_string(),
                    metadata: json!({}),
                    created_at: Some("2026-05-22T00:00:00Z".to_string()),
                    updated_at: Some("2026-05-22T00:00:00Z".to_string()),
                }),
            },
        )
        .expect("chat message append succeeds");
        let chat_effect = service
            .application_effect(&chat.message_id)
            .expect("chat message effect is recorded");
        assert_eq!(
            chat_effect.delivery_policy,
            ApplicationDeliveryPolicy::USER_VISIBLE_MESSAGE
        );
        assert!(chat_effect.creates_push());
        assert!(chat_effect.creates_unread());
        assert!(!chat_effect.creates_command_inbox_work());

        let snapshot = finitechat_push_app_event(
            &mut service,
            FiniteChatAppendInput {
                room_id: seed.room_id.clone(),
                mls_group_id: seed.mls_group_id.clone(),
                epoch: 1,
                sender: seed.runtime_device.clone(),
                conversation_id: None,
                idempotency_key: "runtime-state-1".to_string(),
                payload: FiniteChatEventPayload::RuntimeStateSnapshot(RuntimeStateSnapshotV1 {
                    state_key: "runtime.gateway".to_string(),
                    schema: "finitecomputer.runtime.gateway.v1".to_string(),
                    revision: 1,
                    observed_at_ms: 1_000,
                    expires_at_ms: 31_000,
                    status_payload: json!({
                        "state": "down",
                        "restart_supported": true
                    })
                    .to_string()
                    .into_bytes(),
                }),
            },
        )
        .expect("runtime state append succeeds");
        let snapshot_effect = service
            .application_effect(&snapshot.message_id)
            .expect("runtime state effect is recorded");
        assert_eq!(
            snapshot_effect.delivery_policy,
            ApplicationDeliveryPolicy::NON_NOTIFYING
        );
        assert!(!snapshot_effect.creates_push());
        assert!(!snapshot_effect.creates_unread());
        assert!(!snapshot_effect.creates_command_inbox_work());

        let command = finitechat_push_app_event(
            &mut service,
            FiniteChatAppendInput {
                room_id: seed.room_id.clone(),
                mls_group_id: seed.mls_group_id.clone(),
                epoch: 1,
                sender: seed.user_device.clone(),
                conversation_id: Some("topic-general".to_string()),
                idempotency_key: "runtime-command-1".to_string(),
                payload: FiniteChatEventPayload::RuntimeCommandRequest(
                    FiniteChatCommandRequestPayload {
                        request_id: "restart-gateway-1".to_string(),
                        command: "finitecomputer.runtime.gateway.restart".to_string(),
                        args: json!({}),
                    },
                ),
            },
        )
        .expect("runtime command request append succeeds");
        let command_effect = service
            .application_effect(&command.message_id)
            .expect("runtime command effect is recorded");
        assert_eq!(
            command_effect.delivery_policy,
            ApplicationDeliveryPolicy::RUNTIME_COMMAND_REQUEST
        );
        assert!(command_effect.creates_push());
        assert!(!command_effect.creates_unread());
        assert!(command_effect.creates_command_inbox_work());
        assert_eq!(service.push_outbox_len(), 2);
        assert_eq!(service.unread_len(), 1);
        assert_eq!(service.command_inbox_len(), 1);

        let room = service.room(&seed.room_id).expect("room remains present");
        assert_eq!(room.last_seq, 4);
        assert_eq!(room.current_epoch, 1);
        assert_eq!(
            decode_app_event(&room.log[1].envelope.payload).kind,
            DurableAppEventKind::ChatMessage
        );
        assert_eq!(
            decode_app_event(&room.log[2].envelope.payload).kind,
            DurableAppEventKind::RuntimeStateSnapshot
        );
        assert_eq!(
            decode_app_event(&room.log[3].envelope.payload).kind,
            DurableAppEventKind::RuntimeCommandRequest
        );
    }

    #[test]
    fn chat_message_requires_conversation_id() {
        let error = finitechat_encode_app_event(&FiniteChatAppendInput {
            room_id: "room-a".to_string(),
            mls_group_id: "mls-a".to_string(),
            epoch: 0,
            sender: finitechat_device("user-npub", "phone"),
            conversation_id: None,
            idempotency_key: "chat-message-1".to_string(),
            payload: FiniteChatEventPayload::ChatMessage(FiniteChatMessagePayload {
                message_id: None,
                sender_type: None,
                kind: None,
                status: None,
                body: "hello".to_string(),
                metadata: json!({}),
                created_at: None,
                updated_at: None,
            }),
        })
        .expect_err("missing conversation id is rejected");

        assert!(matches!(
            error,
            FiniteChatBridgeError::MissingConversationId {
                kind: "chat.message"
            }
        ));
    }

    #[test]
    fn conversation_update_is_non_notifying_and_requires_conversation_id() {
        let seed = room_seed();
        let mut service = service_with_active_runtime(&seed);
        let update = finitechat_push_app_event(
            &mut service,
            FiniteChatAppendInput {
                room_id: seed.room_id.clone(),
                mls_group_id: seed.mls_group_id.clone(),
                epoch: 1,
                sender: seed.runtime_device.clone(),
                conversation_id: Some("thread-1".to_string()),
                idempotency_key: "conversation-update-1".to_string(),
                payload: FiniteChatEventPayload::ConversationUpdate(
                    FiniteChatConversationPayload {
                        conversation_id: "thread-1".to_string(),
                        machine_id: "machine-1".to_string(),
                        project_agent_id: "agent_machine-1".to_string(),
                        created_by: "user-1".to_string(),
                        title: "General".to_string(),
                        created_at: "2026-05-22T00:00:00Z".to_string(),
                        last_activity_at: "2026-05-22T00:01:00Z".to_string(),
                        message_count: 1,
                        archived_at: None,
                    },
                ),
            },
        )
        .expect("conversation update append succeeds");
        let effect = service
            .application_effect(&update.message_id)
            .expect("conversation update effect is recorded");
        assert_eq!(
            effect.delivery_policy,
            ApplicationDeliveryPolicy::NON_NOTIFYING
        );
        assert!(!effect.creates_push());
        assert!(!effect.creates_unread());

        let error = finitechat_encode_app_event(&FiniteChatAppendInput {
            room_id: "room-a".to_string(),
            mls_group_id: "mls-a".to_string(),
            epoch: 0,
            sender: finitechat_device("runtime-npub", "finitec"),
            conversation_id: None,
            idempotency_key: "conversation-update-2".to_string(),
            payload: FiniteChatEventPayload::ConversationUpdate(FiniteChatConversationPayload {
                conversation_id: "thread-1".to_string(),
                machine_id: "machine-1".to_string(),
                project_agent_id: "agent_machine-1".to_string(),
                created_by: "user-1".to_string(),
                title: "General".to_string(),
                created_at: "2026-05-22T00:00:00Z".to_string(),
                last_activity_at: "2026-05-22T00:01:00Z".to_string(),
                message_count: 1,
                archived_at: None,
            }),
        })
        .expect_err("missing conversation id is rejected");

        assert!(matches!(
            error,
            FiniteChatBridgeError::MissingConversationId {
                kind: "conversation.update"
            }
        ));
    }

    #[test]
    fn invalid_runtime_state_snapshot_is_rejected_before_append() {
        let error = finitechat_encode_app_event(&FiniteChatAppendInput {
            room_id: "room-a".to_string(),
            mls_group_id: "mls-a".to_string(),
            epoch: 0,
            sender: finitechat_device("runtime-npub", "finitec"),
            conversation_id: None,
            idempotency_key: "runtime-state-1".to_string(),
            payload: FiniteChatEventPayload::RuntimeStateSnapshot(RuntimeStateSnapshotV1 {
                state_key: "runtime.gateway".to_string(),
                schema: "finitecomputer.runtime.gateway.v1".to_string(),
                revision: 1,
                observed_at_ms: 2_000,
                expires_at_ms: 1_000,
                status_payload: vec![b'x'; MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES as usize + 1],
            }),
        })
        .expect_err("invalid snapshot is rejected");

        assert!(matches!(
            error,
            FiniteChatBridgeError::InvalidTimeRange {
                start_field: "runtime_state.observed_at_ms",
                end_field: "runtime_state.expires_at_ms"
            }
        ));
    }

    #[test]
    fn command_request_requires_nonblank_command() {
        let error = finitechat_encode_app_event(&FiniteChatAppendInput {
            room_id: "room-a".to_string(),
            mls_group_id: "mls-a".to_string(),
            epoch: 0,
            sender: finitechat_device("user-npub", "phone"),
            conversation_id: Some("topic-general".to_string()),
            idempotency_key: "runtime-command-1".to_string(),
            payload: FiniteChatEventPayload::RuntimeCommandRequest(
                FiniteChatCommandRequestPayload {
                    request_id: "restart-gateway-1".to_string(),
                    command: "   ".to_string(),
                    args: json!({}),
                },
            ),
        })
        .expect_err("blank command is rejected");

        assert!(matches!(
            error,
            FiniteChatBridgeError::BlankField("runtime.command.command")
        ));
    }

    fn service_with_active_runtime(seed: &FiniteChatRoomSeed) -> DeliveryService {
        seed.validate().expect("seed is valid");
        let mut service = DeliveryService::new();
        service
            .create_room(CreateRoomRequest {
                room_id: seed.room_id.clone(),
                mls_group_id: seed.mls_group_id.clone(),
                creator: seed.user_device.clone(),
            })
            .expect("room create succeeds");
        activate_runtime_device(&mut service, seed).expect("runtime device activates");
        service
    }

    fn activate_runtime_device(
        service: &mut DeliveryService,
        seed: &FiniteChatRoomSeed,
    ) -> Result<(), FiniteChatBridgeError> {
        service.upload_key_package(UploadKeyPackageRequest {
            key_package_id: "kp-runtime-1".to_string(),
            owner: seed.runtime_device.clone(),
            key_package_ref: "ref-runtime-1".to_string(),
            key_package_hash: "hash-runtime-1".to_string(),
            key_package_payload: b"opaque runtime key package".to_vec(),
        })?;
        let claimed = service.claim_key_package("kp-runtime-1")?;
        let commit = envelope(
            seed.room_id.clone(),
            seed.mls_group_id.clone(),
            seed.user_device.clone(),
            0,
            LogEntryKind::Commit,
            b"add runtime device".to_vec(),
        );
        let commit_message_id = commit.message_id()?;
        service.submit_commit(SubmitCommitRequest {
            room_id: seed.room_id.clone(),
            sender: seed.user_device.clone(),
            expected_epoch: 0,
            envelope: commit,
            membership_delta: MembershipDeltaV1 {
                base_epoch: 0,
                post_commit_epoch: 1,
                commit_message_id,
                adds: vec![MembershipAddV1 {
                    device: seed.runtime_device.clone(),
                    key_package_id: claimed.key_package_id,
                    key_package_ref: claimed.key_package_ref,
                    key_package_hash: claimed.key_package_hash,
                    welcome_id: "welcome-runtime-1".to_string(),
                }],
                removes: vec![],
            },
            staged_welcomes: vec![StagedWelcomeV1 {
                welcome_id: "welcome-runtime-1".to_string(),
                welcome_payload: b"opaque welcome".to_vec(),
                ratchet_tree_payload: b"opaque ratchet tree".to_vec(),
            }],
            idempotency_key: "add-runtime-device-1".to_string(),
        })?;
        let welcomes = service.claim_welcomes(&seed.runtime_device)?;
        assert_eq!(welcomes.len(), 1);
        service.ack_welcome(&welcomes[0].welcome_id, true)?;
        assert!(
            service
                .room(&seed.room_id)
                .expect("room exists")
                .device_active_at_head(&seed.runtime_device)
        );
        Ok(())
    }

    fn room_seed() -> FiniteChatRoomSeed {
        FiniteChatRoomSeed {
            room_id: "room-machine-agent-1".to_string(),
            mls_group_id: "mls-machine-agent-1".to_string(),
            user_device: finitechat_device("user-npub", "phone"),
            runtime_device: finitechat_device("runtime-npub", "finitec-daemon"),
        }
    }

    fn decode_app_event(bytes: &[u8]) -> DecryptedApplicationEventV1 {
        assert!(!bytes.is_empty());
        serde_json::from_slice(bytes).expect("app event decodes")
    }
}
