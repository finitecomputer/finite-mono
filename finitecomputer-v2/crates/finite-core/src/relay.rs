use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::{
    Engine as _,
    engine::general_purpose::{
        STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD,
    },
};
use finitechat_blob::{
    BlobDescriptor, decrypt_attachment_ciphertext, finish_attachment_upload,
    prepare_attachment_upload, sha256_hex,
};
use finitechat_engine::{EngineError, ListAccountRoomsRequest};
use finitechat_proto::{
    AttachmentBlobMetadataV1, AttachmentBlobReferenceV1, DecryptedApplicationEventV1, DeviceRef,
    DurableAppEventKind, LogEntryKind, MAX_ATTACHMENT_CIPHERTEXT_BYTES,
    MAX_ATTACHMENT_PLAINTEXT_BYTES, MAX_OBJECT_ID_BYTES, MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS,
    MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES, RuntimeStateSnapshotV1,
};
use finitechat_store::{SqliteDeliveryStore, StoreError};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use time::Duration as TimeDuration;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::chat::{
    CHAT_ATTACHMENT_BLOB_METADATA_KEY, ChatInboxEvent, ChatInboxPage, ChatMessage,
    ChatMessageAttachmentInput, ChatMessagePage, ChatThread, SendChatMessageRequest,
};
use crate::chat_runtime::ChatAttachmentBlobUpload;
use crate::finite_chat::{
    FiniteChatAppendInput, FiniteChatBridgeError, FiniteChatCommandRequestPayload,
    FiniteChatCommandResultPayload, FiniteChatCommandStatus, FiniteChatConversationPayload,
    FiniteChatEventPayload, FiniteChatMessagePayload, FiniteChatRoomSeed, finitechat_device,
    finitechat_ensure_local_room_ready, finitechat_protocol_object_id,
    finitechat_push_app_event_to_store,
};

const DEFAULT_EVENT_TTL_SECS: u64 = 60;
const MAX_EVENT_TTL_SECS: u64 = 600;
const DEFAULT_STATUS_TTL_SECS: u64 = 120;
const MAX_RELAY_EVENTS: usize = 50;
const MAX_STATUS_TTL_SECS: u64 = 60 * 60;
const FINITE_CHAT_COMMAND_LEDGER_DB_FILE: &str = "finitechat-commands.sqlite3";
const FINITE_CHAT_CHAT_LEDGER_DB_FILE: &str = "finitechat-chat.sqlite3";
const FINITE_CHAT_BLOBS_DIR: &str = "finitechat-blobs";
const FINITE_CHAT_RELAY_BLOB_URL_PREFIX: &str = "finitechat+relay-blob://sha256/";
const MAX_COMMAND_LEDGER_SCAN_ENTRIES: usize = 100_000;
const MAX_CHAT_LEDGER_SCAN_ENTRIES: usize = 100_000;
const MAX_CHAT_LEDGER_SCAN_ROOMS: usize = 1_000;
const MAX_CHAT_LOG_SYNC_THREADS_PER_REQUEST: usize = 500;
const MAX_CHAT_LOG_SYNC_MESSAGES_PER_REQUEST: usize = 1_000;
const MAX_CHAT_THREAD_TITLE_CHARS: usize = 120;
const MAX_CHAT_MESSAGE_PAGE_LIMIT: usize = 200;
const MAX_CHAT_MESSAGE_BEFORE_CURSOR_BYTES: usize = 512;
const MAX_CHAT_STREAM_THREAD_METADATA: usize = 256;
const MAX_CHAT_STREAM_MESSAGE_THREADS: usize = 8;
const MAX_CHAT_STREAM_MESSAGES_PER_THREAD: usize = 80;
const MAX_CHAT_STREAM_MESSAGES_TOTAL: usize = 256;
const MAX_CHAT_INBOX_EVENTS: usize = 100;
const FINITE_CHAT_HOSTED_BRIDGE_STATE_DIR: &str = "finitechat-hosted-bridge-state";
const MAX_CHAT_ATTACHMENTS_PER_MESSAGE: usize = 32;
const CHAT_STREAM_CURSOR_PREFIX: &str = "v1.";
const MAX_CHAT_STREAM_CURSOR_BYTES: usize = 4096;
const MAX_CHAT_STREAM_CURSOR_ROOMS: usize = 64;
const CHAT_BOOTSTRAP_STATE_KEY: &str = "chat.bootstrap.metadata";
const CHAT_BOOTSTRAP_STATE_SCHEMA: &str = "finitecomputer.chat.bootstrap.metadata.v1";
const CHAT_BOOTSTRAP_METADATA_TTL_MS: u64 = 5 * 60 * 1_000;
const MAX_CHAT_BOOTSTRAP_PROJECT_AGENTS: usize = 64;
const CHAT_BOOTSTRAP_METADATA_KEYS: [&str; 6] = [
    "users",
    "machines",
    "project_agents",
    "sites",
    "skills",
    "capabilities",
];
const COMMAND_ENVELOPE_SCHEMA: &str = "finitecomputer.runtime.command.envelope.v1";
const COMMAND_LEASE_STATE_KEY: &str = "runtime.command.lease";
const COMMAND_LEASE_STATE_SCHEMA: &str = "finitecomputer.runtime.command.lease.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayEvent {
    pub id: String,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub lane: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge: Option<RelayBridgeDevice>,
    #[serde(
        rename = "commandEnvelope",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub command_envelope: Option<RelayCommandEnvelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<RelayCommandScope>,
    #[serde(default)]
    pub payload: Value,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayResult {
    #[serde(rename = "eventId")]
    pub event_id: String,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub output: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayHeartbeat {
    pub ok: bool,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "lastSeenAt")]
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayChatSnapshot {
    pub ok: bool,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(default)]
    pub snapshot: Value,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayChatStreamEvent {
    pub ok: bool,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(default)]
    pub snapshot: Value,
    pub cursor: String,
    pub reset: bool,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ChatStreamCursor {
    rooms: BTreeMap<String, u64>,
}

struct ChatStreamRoomUpdate {
    room_id: String,
    current_seq: u64,
    messages: Vec<ChatMessage>,
}

struct ChatStreamMaterialization {
    cursor: ChatStreamCursor,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayStatusSnapshot {
    pub ok: bool,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "stateKey")]
    pub state_key: String,
    pub schema: String,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub status: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(rename = "observedAt")]
    pub observed_at: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreRelayChatSnapshotInput {
    #[serde(default)]
    pub snapshot: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreRelayChatLogInput {
    #[serde(rename = "projectAgentId")]
    pub project_agent_id: String,
    #[serde(default)]
    pub threads: Vec<ChatThread>,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendRelayChatMessageInput {
    pub bridge: RelayBridgeDevice,
    pub message: SendChatMessageRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateRelayChatConversationInput {
    pub bridge: RelayBridgeDevice,
    #[serde(rename = "projectAgentId")]
    pub project_agent_id: String,
    #[serde(
        rename = "conversationId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub conversation_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateRelayChatConversationInput {
    pub bridge: RelayBridgeDevice,
    #[serde(rename = "projectAgentId")]
    pub project_agent_id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayChatLogAck {
    pub ok: bool,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "projectAgentId")]
    pub project_agent_id: String,
    pub stored: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayChatBlobAck {
    pub ok: bool,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub sha256: String,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRelayChatAttachmentUpload {
    pub metadata: Value,
    pub blob: ChatAttachmentBlobUpload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayChatAttachmentData {
    pub name: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelayChatAttachmentRecord {
    name: String,
    mime_type: String,
    reference: AttachmentBlobReferenceV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectedChatThread {
    thread: ChatThread,
    title_is_user_authored: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayBridgeDevice {
    #[serde(rename = "bridgeAccountId")]
    pub bridge_account_id: String,
    #[serde(rename = "bridgeDeviceId")]
    pub bridge_device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RelayCommandScope {
    pub machine_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_device_id: Option<String>,
}

impl RelayCommandScope {
    fn validate_for_machine(&self, machine_id: &str) -> Result<()> {
        assert!(!machine_id.is_empty());
        let scope_machine_id = safe_segment(&self.machine_id, "command scope machine id")?;
        if scope_machine_id.len() > MAX_OBJECT_ID_BYTES as usize {
            bail!(
                "command scope machine id exceeds {} bytes",
                MAX_OBJECT_ID_BYTES
            );
        }
        if scope_machine_id != machine_id {
            bail!("command scope machine id does not match relay machine id");
        }
        validate_optional_scope_id(&self.room_id, "command scope room id")?;
        validate_optional_scope_id(&self.conversation_id, "command scope conversation id")?;
        validate_optional_scope_id(&self.topic_id, "command scope topic id")?;
        validate_optional_scope_id(&self.project_id, "command scope project id")?;
        validate_optional_scope_id(&self.project_agent_id, "command scope project agent id")?;
        validate_optional_scope_id(&self.runtime_id, "command scope runtime id")?;
        validate_optional_scope_id(&self.target_device_id, "command scope target device id")?;
        validate_optional_scope_id(&self.actor_device_id, "command scope actor device id")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ChatMessageBeforeCursor {
    pub created_at: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreRelayStatusSnapshotInput {
    #[serde(rename = "stateKey")]
    pub state_key: String,
    pub schema: String,
    #[serde(default)]
    pub revision: Option<u64>,
    #[serde(rename = "ttlSecs")]
    pub ttl_secs: Option<u64>,
    pub ok: bool,
    #[serde(default)]
    pub status: Value,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateRelayEventInput {
    pub lane: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge: Option<RelayBridgeDevice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<RelayCommandScope>,
    #[serde(default)]
    pub payload: Value,
    #[serde(rename = "ttlSecs")]
    pub ttl_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreRelayResultInput {
    #[serde(rename = "eventId")]
    pub event_id: String,
    pub ok: bool,
    #[serde(default)]
    pub output: Value,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayEventsOutput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub events: Vec<RelayEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayAckOutput {
    pub ok: bool,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "eventId")]
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayResultAck {
    pub ok: bool,
    #[serde(rename = "eventId")]
    pub event_id: String,
}

#[derive(Debug, Clone)]
pub struct RelayStore {
    root: PathBuf,
    finitechat_command_ledger: FiniteChatCommandLedger,
    finitechat_chat_ledger: FiniteChatChatLedger,
}

pub fn prepare_relay_chat_attachment_upload(
    message_id: &str,
    index: usize,
    attachment: &ChatMessageAttachmentInput,
) -> Result<PreparedRelayChatAttachmentUpload> {
    assert!(!message_id.is_empty());
    let max_attachment_bytes = MAX_ATTACHMENT_PLAINTEXT_BYTES as usize;
    validate_relay_attachment_base64_size(
        &attachment.name,
        &attachment.data_base64,
        max_attachment_bytes,
    )?;
    let plaintext = BASE64_STANDARD
        .decode(attachment.data_base64.as_bytes())
        .with_context(|| format!("attachment {} is not valid base64", attachment.name))?;
    if plaintext.is_empty() {
        bail!("attachment {} is empty", attachment.name);
    }
    if plaintext.len() > max_attachment_bytes {
        bail!(
            "attachment {} is {} and exceeds the {} chat attachment limit",
            attachment.name,
            format_bytes(plaintext.len()),
            format_bytes(max_attachment_bytes)
        );
    }

    let metadata = relay_attachment_blob_metadata(&attachment.name, &attachment.mime_type)?;
    let prepared = prepare_attachment_upload(&plaintext, metadata).with_context(|| {
        format!(
            "failed to prepare encrypted blob for attachment {}",
            attachment.name
        )
    })?;
    let ciphertext = prepared.ciphertext.clone();
    let ciphertext_sha256 = prepared.ciphertext_sha256.clone();
    let descriptor = BlobDescriptor {
        url: relay_blob_url(&ciphertext_sha256)?,
        sha256: ciphertext_sha256.clone(),
        size_bytes: prepared.ciphertext_size,
    };
    let reference = finish_attachment_upload(&prepared, descriptor)
        .context("failed to finish encrypted relay attachment blob reference")?;
    let attachment_id = relay_attachment_id(message_id, index, &reference);
    let metadata = json!({
        "id": attachment_id,
        "type": attachment_kind(&attachment.mime_type),
        "name": attachment.name,
        "mime_type": attachment.mime_type,
        "size_bytes": plaintext.len() as u64,
        "url": format!("/attachments/{attachment_id}"),
        CHAT_ATTACHMENT_BLOB_METADATA_KEY: reference,
    });
    let blob = ChatAttachmentBlobUpload {
        sha256: ciphertext_sha256,
        ciphertext,
    };
    assert!(!blob.sha256.is_empty());
    assert!(!blob.ciphertext.is_empty());
    Ok(PreparedRelayChatAttachmentUpload { metadata, blob })
}

#[derive(Debug, Clone)]
struct FiniteChatCommandLedger {
    db_path: PathBuf,
}

#[derive(Debug, Clone)]
struct FiniteChatChatLedger {
    db_path: PathBuf,
}

impl RelayStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let finitechat_command_ledger = FiniteChatCommandLedger {
            db_path: root.join(FINITE_CHAT_COMMAND_LEDGER_DB_FILE),
        };
        let finitechat_chat_ledger = FiniteChatChatLedger {
            db_path: root.join(FINITE_CHAT_CHAT_LEDGER_DB_FILE),
        };
        Self {
            root,
            finitechat_command_ledger,
            finitechat_chat_ledger,
        }
    }

    pub fn heartbeat(&self, machine_id: &str) -> Result<RelayHeartbeat> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let now = now_iso()?;
        let heartbeat = RelayHeartbeat {
            ok: true,
            machine_id: machine_id.clone(),
            last_seen_at: now,
        };
        write_json_atomic(
            &self.machine_path(&machine_id, &["heartbeat.json"]),
            &heartbeat,
        )?;
        Ok(heartbeat)
    }

    pub fn read_heartbeat(&self, machine_id: &str) -> Result<Option<RelayHeartbeat>> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let path = self.machine_path(&machine_id, &["heartbeat.json"]);
        if !path.exists() {
            return Ok(None);
        }
        read_json_file(&path).map(Some)
    }

    pub fn store_chat_snapshot(
        &self,
        machine_id: &str,
        input: &StoreRelayChatSnapshotInput,
    ) -> Result<RelayChatSnapshot> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let _stored_state_count = self
            .finitechat_chat_ledger
            .store_chat_bootstrap_metadata(&machine_id, &input.snapshot)?;
        let snapshot = RelayChatSnapshot {
            ok: true,
            machine_id: machine_id.clone(),
            snapshot: input.snapshot.clone(),
            updated_at: now_iso()?,
        };
        assert_eq!(snapshot.machine_id, machine_id);
        Ok(snapshot)
    }

    pub fn read_chat_snapshot(&self, machine_id: &str) -> Result<Option<RelayChatSnapshot>> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        self.finitechat_chat_ledger
            .chat_bootstrap_snapshot(&machine_id)
    }

    pub fn chat_stream_snapshot(
        &self,
        machine_id: &str,
        bridge: &RelayBridgeDevice,
    ) -> Result<Option<RelayChatSnapshot>> {
        Ok(self
            .chat_stream_event(machine_id, bridge, None)?
            .map(|event| RelayChatSnapshot {
                ok: event.ok,
                machine_id: event.machine_id,
                snapshot: event.snapshot,
                updated_at: event.updated_at,
            }))
    }

    pub fn chat_stream_event(
        &self,
        machine_id: &str,
        bridge: &RelayBridgeDevice,
        since_cursor: Option<&str>,
    ) -> Result<Option<RelayChatStreamEvent>> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let _bridge_state_dir = self.ensure_hosted_bridge_state_dir(&machine_id, bridge)?;
        let base = self.read_chat_snapshot(&machine_id)?;
        let mut threads = self.finitechat_chat_ledger.chat_threads(&machine_id)?;
        let Some(base) = base else {
            if threads.is_empty() {
                return Ok(None);
            }
            bail!("chat bootstrap metadata is unavailable for {machine_id}");
        };
        let since = since_cursor
            .map(decode_chat_stream_cursor)
            .transpose()
            .context("invalid chat stream cursor")?;
        let mut reset = since.is_none();
        if threads.len() > MAX_CHAT_STREAM_THREAD_METADATA {
            threads.truncate(MAX_CHAT_STREAM_THREAD_METADATA);
        }

        let mut snapshot = base
            .snapshot
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("chat bootstrap metadata must be a JSON object"))?;
        let (messages, cursor) = if let Some(since) = since.as_ref() {
            let materialized = self.finitechat_chat_ledger.chat_stream_messages_after(
                &machine_id,
                &threads,
                since,
            )?;
            if materialized.messages.is_empty() && materialized.cursor == *since {
                return Ok(None);
            }
            if materialized.messages.len() > MAX_CHAT_STREAM_MESSAGES_TOTAL {
                reset = true;
                let messages = self
                    .finitechat_chat_ledger
                    .chat_stream_messages(&machine_id, &threads)?;
                let cursor = self
                    .finitechat_chat_ledger
                    .chat_stream_cursor(&machine_id, &threads)?;
                (messages, cursor)
            } else {
                (materialized.messages, materialized.cursor)
            }
        } else {
            let messages = self
                .finitechat_chat_ledger
                .chat_stream_messages(&machine_id, &threads)?;
            let cursor = self
                .finitechat_chat_ledger
                .chat_stream_cursor(&machine_id, &threads)?;
            (messages, cursor)
        };
        let cursor = encode_chat_stream_cursor(&cursor)?;
        assert!(threads.len() <= MAX_CHAT_STREAM_THREAD_METADATA);
        assert!(messages.len() <= MAX_CHAT_STREAM_MESSAGES_TOTAL);
        snapshot.insert("threads".to_string(), serde_json::to_value(threads)?);
        snapshot.insert("messages".to_string(), serde_json::to_value(messages)?);
        Ok(Some(RelayChatStreamEvent {
            ok: true,
            machine_id,
            snapshot: Value::Object(snapshot),
            cursor,
            reset,
            updated_at: now_iso()?,
        }))
    }

    pub fn store_chat_log(
        &self,
        machine_id: &str,
        input: &StoreRelayChatLogInput,
    ) -> Result<RelayChatLogAck> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let project_agent_id = safe_segment(&input.project_agent_id, "project agent id")?;
        if input.threads.len() > MAX_CHAT_LOG_SYNC_THREADS_PER_REQUEST {
            bail!(
                "chat log sync contains {} threads, max is {}",
                input.threads.len(),
                MAX_CHAT_LOG_SYNC_THREADS_PER_REQUEST
            );
        }
        if input.messages.len() > MAX_CHAT_LOG_SYNC_MESSAGES_PER_REQUEST {
            bail!(
                "chat log sync contains {} messages, max is {}",
                input.messages.len(),
                MAX_CHAT_LOG_SYNC_MESSAGES_PER_REQUEST
            );
        }
        let (stored, skipped) = self.finitechat_chat_ledger.append_entries(
            &machine_id,
            &project_agent_id,
            &input.threads,
            &input.messages,
        )?;
        assert!(stored + skipped <= input.threads.len() + input.messages.len());
        Ok(RelayChatLogAck {
            ok: true,
            machine_id,
            project_agent_id,
            stored,
            skipped,
        })
    }

    pub fn chat_inbox(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        after_seq: Option<u64>,
        limit: Option<usize>,
    ) -> Result<ChatInboxPage> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let project_agent_id = safe_segment(project_agent_id, "project agent id")?;
        let limit = limit
            .unwrap_or(MAX_CHAT_INBOX_EVENTS)
            .clamp(1, MAX_CHAT_INBOX_EVENTS);
        let after_seq = after_seq.unwrap_or(0);
        let events = self.finitechat_chat_ledger.chat_inbox(
            &machine_id,
            &project_agent_id,
            after_seq,
            limit,
        )?;
        let cursor = events.last().map(|event| event.seq).unwrap_or(after_seq);
        assert!(events.len() <= limit);
        assert!(cursor >= after_seq);
        Ok(ChatInboxPage {
            ok: true,
            machine_id,
            project_agent_id,
            cursor,
            events,
        })
    }

    pub fn send_chat_message(
        &self,
        machine_id: &str,
        conversation_id: &str,
        input: &SendRelayChatMessageInput,
    ) -> Result<ChatMessage> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let conversation_id = safe_segment(conversation_id, "conversation id")?;
        let _bridge_state_dir = self.ensure_hosted_bridge_state_dir(&machine_id, &input.bridge)?;
        if input.message.body.trim().is_empty() && input.message.attachments.is_empty() {
            bail!("message body or attachment is required");
        }
        let project_agent_id = self
            .finitechat_chat_ledger
            .chat_project_agent_id_for_conversation(&machine_id, &conversation_id)?
            .ok_or_else(|| anyhow::anyhow!("conversation {conversation_id} not found"))?;
        let message_id = input
            .message
            .client_message_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_segment(value, "client message id"))
            .transpose()?
            .unwrap_or_else(|| finitechat_protocol_object_id("msg", &relay_event_id()));
        let existing = self.finitechat_chat_ledger.chat_message_by_id(
            &machine_id,
            &project_agent_id,
            &message_id,
        )?;
        let message = match existing {
            Some(existing) => {
                validate_chat_send_retry_matches(&existing, &conversation_id, &input.message)?;
                existing
            }
            None => {
                let now = now_iso()?;
                let attachments = self.relay_chat_attachments(
                    &machine_id,
                    &message_id,
                    &input.message.attachments,
                )?;
                let metadata = if attachments.is_empty() {
                    json!({})
                } else {
                    json!({ "attachments": attachments })
                };
                let message = ChatMessage {
                    id: message_id.clone(),
                    thread_id: conversation_id.clone(),
                    sender_type: "user".to_string(),
                    kind: "message".to_string(),
                    status: "complete".to_string(),
                    body: input.message.body.clone(),
                    metadata,
                    created_at: now.clone(),
                    updated_at: now,
                };
                self.finitechat_chat_ledger.append_user_message(
                    &machine_id,
                    &project_agent_id,
                    &message,
                )?;
                message
            }
        };

        Ok(message)
    }

    pub fn store_chat_blob(
        &self,
        machine_id: &str,
        sha256: &str,
        ciphertext: &[u8],
    ) -> Result<RelayChatBlobAck> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let sha256 = safe_blob_sha256(sha256)?;
        if ciphertext.is_empty() {
            bail!("chat blob is empty");
        }
        if ciphertext.len() > MAX_ATTACHMENT_CIPHERTEXT_BYTES as usize {
            bail!(
                "chat blob is {} bytes, max is {}",
                ciphertext.len(),
                MAX_ATTACHMENT_CIPHERTEXT_BYTES
            );
        }
        let actual_sha256 = sha256_hex(ciphertext);
        if actual_sha256 != sha256 {
            bail!("chat blob sha256 mismatch");
        }
        let path = self.chat_blob_path(&sha256)?;
        if path.is_file() {
            let existing =
                fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
            if existing != ciphertext {
                bail!("chat blob hash collision for {sha256}");
            }
        } else {
            write_bytes_atomic(&path, ciphertext)?;
        }
        Ok(RelayChatBlobAck {
            ok: true,
            machine_id,
            sha256,
            size_bytes: ciphertext.len() as u64,
        })
    }

    pub fn read_chat_attachment(
        &self,
        machine_id: &str,
        attachment_id: &str,
        bridge: &RelayBridgeDevice,
    ) -> Result<Option<RelayChatAttachmentData>> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let attachment_id = safe_segment(attachment_id, "attachment id")?;
        let _bridge_state_dir = self.ensure_hosted_bridge_state_dir(&machine_id, bridge)?;
        let Some(record) = self
            .finitechat_chat_ledger
            .chat_attachment_record(&machine_id, &attachment_id)?
        else {
            return Ok(None);
        };
        let sha256 = safe_blob_sha256(&record.reference.ciphertext_sha256)?;
        let path = self.chat_blob_path(&sha256)?;
        let ciphertext =
            fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        let downloaded = decrypt_attachment_ciphertext(&record.reference, &ciphertext)
            .context("failed to decrypt chat attachment blob")?;
        assert_eq!(downloaded.reference, record.reference);
        if downloaded.plaintext.is_empty() {
            bail!("chat attachment {attachment_id} is empty");
        }
        Ok(Some(RelayChatAttachmentData {
            name: record.name,
            mime_type: record.mime_type,
            bytes: downloaded.plaintext,
        }))
    }

    pub fn chat_threads(
        &self,
        machine_id: &str,
        bridge: &RelayBridgeDevice,
    ) -> Result<Vec<ChatThread>> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let _bridge_state_dir = self.ensure_hosted_bridge_state_dir(&machine_id, bridge)?;
        self.finitechat_chat_ledger.chat_threads(&machine_id)
    }

    pub fn create_chat_conversation(
        &self,
        machine_id: &str,
        input: &CreateRelayChatConversationInput,
    ) -> Result<ChatThread> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let project_agent_id = safe_segment(&input.project_agent_id, "project agent id")?;
        let conversation_id = input
            .conversation_id
            .as_deref()
            .map(|value| safe_segment(value, "conversation id"))
            .transpose()?
            .unwrap_or_else(|| finitechat_protocol_object_id("topic", &relay_event_id()));
        let _bridge_state_dir = self.ensure_hosted_bridge_state_dir(&machine_id, &input.bridge)?;
        let now = now_iso()?;
        let requested = ChatThread {
            id: conversation_id.clone(),
            project_agent_id: project_agent_id.clone(),
            created_by: input.bridge.bridge_account_id.clone(),
            title: relay_chat_thread_title(input.title.as_deref())?,
            created_at: now.clone(),
            last_activity_at: now,
            message_count: 0,
        };
        let thread = self.finitechat_chat_ledger.append_conversation(
            &machine_id,
            &project_agent_id,
            &requested,
        )?;
        Ok(thread)
    }

    pub fn update_chat_conversation(
        &self,
        machine_id: &str,
        conversation_id: &str,
        input: &UpdateRelayChatConversationInput,
    ) -> Result<ChatThread> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let conversation_id = safe_segment(conversation_id, "conversation id")?;
        let project_agent_id = safe_segment(&input.project_agent_id, "project agent id")?;
        let _bridge_state_dir = self.ensure_hosted_bridge_state_dir(&machine_id, &input.bridge)?;
        let title = relay_chat_user_title(&input.title)?;
        let existing = self
            .finitechat_chat_ledger
            .chat_thread_by_id(&machine_id, &project_agent_id, &conversation_id)?
            .ok_or_else(|| anyhow::anyhow!("conversation not found"))?;
        assert_eq!(existing.id, conversation_id);
        assert_eq!(existing.project_agent_id, project_agent_id);
        if existing.title == title {
            return Ok(existing);
        }

        let updated = ChatThread {
            id: existing.id,
            project_agent_id: existing.project_agent_id,
            created_by: existing.created_by,
            title,
            created_at: existing.created_at,
            last_activity_at: existing.last_activity_at,
            message_count: existing.message_count,
        };
        let thread = self.finitechat_chat_ledger.append_conversation_update(
            &machine_id,
            &project_agent_id,
            &updated,
        )?;
        assert_eq!(thread.id, conversation_id);
        assert_eq!(thread.title, updated.title);
        Ok(thread)
    }

    pub fn chat_message_page(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        conversation_id: &str,
        bridge: &RelayBridgeDevice,
        limit: Option<usize>,
        before: Option<&str>,
    ) -> Result<ChatMessagePage> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let project_agent_id = safe_segment(project_agent_id, "project agent id")?;
        let conversation_id = safe_segment(conversation_id, "conversation id")?;
        let _bridge_state_dir = self.ensure_hosted_bridge_state_dir(&machine_id, bridge)?;
        let limit = limit.unwrap_or(80).clamp(1, MAX_CHAT_MESSAGE_PAGE_LIMIT);
        self.finitechat_chat_ledger.chat_message_page(
            &machine_id,
            &project_agent_id,
            &conversation_id,
            limit,
            before,
        )
    }

    pub fn chat_message_page_for_machine(
        &self,
        machine_id: &str,
        conversation_id: &str,
        bridge: &RelayBridgeDevice,
        limit: Option<usize>,
        before: Option<&str>,
    ) -> Result<ChatMessagePage> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let conversation_id = safe_segment(conversation_id, "conversation id")?;
        let _bridge_state_dir = self.ensure_hosted_bridge_state_dir(&machine_id, bridge)?;
        let limit = limit.unwrap_or(80).clamp(1, MAX_CHAT_MESSAGE_PAGE_LIMIT);
        let project_agent_id = self
            .finitechat_chat_ledger
            .chat_project_agent_id_for_conversation(&machine_id, &conversation_id)?
            .ok_or_else(|| anyhow::anyhow!("conversation {conversation_id} not found"))?;
        self.finitechat_chat_ledger.chat_message_page(
            &machine_id,
            &project_agent_id,
            &conversation_id,
            limit,
            before,
        )
    }

    fn ensure_hosted_bridge_state_dir(
        &self,
        machine_id: &str,
        bridge: &RelayBridgeDevice,
    ) -> Result<PathBuf> {
        let path = self.hosted_bridge_state_dir(machine_id, bridge)?;
        fs::create_dir_all(&path)
            .with_context(|| format!("failed to create {}", path.display()))?;
        assert!(path.is_dir());
        Ok(path)
    }

    fn hosted_bridge_state_dir(
        &self,
        machine_id: &str,
        bridge: &RelayBridgeDevice,
    ) -> Result<PathBuf> {
        assert!(!machine_id.is_empty());
        let account_id = safe_segment(&bridge.bridge_account_id, "bridge account id")?;
        let device_id = safe_segment(&bridge.bridge_device_id, "bridge device id")?;
        let path = self
            .root
            .join(FINITE_CHAT_HOSTED_BRIDGE_STATE_DIR)
            .join(account_id)
            .join(device_id)
            .join("machines")
            .join(machine_id);
        assert!(path.starts_with(&self.root));
        Ok(path)
    }

    fn relay_chat_attachments(
        &self,
        machine_id: &str,
        message_id: &str,
        attachments: &[ChatMessageAttachmentInput],
    ) -> Result<Vec<Value>> {
        assert!(!machine_id.is_empty());
        assert!(!message_id.is_empty());
        if attachments.len() > MAX_CHAT_ATTACHMENTS_PER_MESSAGE {
            bail!(
                "chat message contains {} attachments, max is {}",
                attachments.len(),
                MAX_CHAT_ATTACHMENTS_PER_MESSAGE
            );
        }
        let mut saved = Vec::with_capacity(attachments.len());
        for (index, attachment) in attachments.iter().enumerate() {
            saved.push(self.relay_chat_attachment(machine_id, message_id, index, attachment)?);
        }
        assert!(saved.len() <= MAX_CHAT_ATTACHMENTS_PER_MESSAGE);
        Ok(saved)
    }

    fn relay_chat_attachment(
        &self,
        machine_id: &str,
        message_id: &str,
        index: usize,
        attachment: &ChatMessageAttachmentInput,
    ) -> Result<Value> {
        assert!(!machine_id.is_empty());
        assert!(!message_id.is_empty());
        let prepared = prepare_relay_chat_attachment_upload(message_id, index, attachment)?;
        let _ack =
            self.store_chat_blob(machine_id, &prepared.blob.sha256, &prepared.blob.ciphertext)?;
        Ok(prepared.metadata)
    }

    fn chat_blob_path(&self, sha256: &str) -> Result<PathBuf> {
        let sha256 = safe_blob_sha256(sha256)?;
        let path = self.root.join(FINITE_CHAT_BLOBS_DIR).join(sha256);
        assert!(path.starts_with(&self.root));
        Ok(path)
    }

    pub fn store_status_snapshot(
        &self,
        machine_id: &str,
        input: &StoreRelayStatusSnapshotInput,
    ) -> Result<RelayStatusSnapshot> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let state_key = safe_kind(&input.state_key, "state key")?;
        let schema = safe_kind(&input.schema, "schema")?;
        validate_status_snapshot_label("state key", &state_key)?;
        validate_status_snapshot_label("schema", &schema)?;
        validate_status_snapshot_payload(&input.status)?;
        if let Some(error) = &input.error {
            validate_status_snapshot_error(error)?;
        }
        let now = now_iso()?;
        let ttl_secs = input
            .ttl_secs
            .unwrap_or(DEFAULT_STATUS_TTL_SECS)
            .clamp(1, MAX_STATUS_TTL_SECS);
        let snapshot = RelayStatusSnapshot {
            ok: input.ok,
            machine_id: machine_id.clone(),
            state_key: state_key.clone(),
            schema,
            revision: input.revision.unwrap_or(now_millis()),
            status: input.status.clone(),
            error: input.error.clone(),
            observed_at: now,
            expires_at: (OffsetDateTime::now_utc() + TimeDuration::seconds(ttl_secs as i64))
                .format(&Rfc3339)?,
        };
        assert!(!snapshot.state_key.is_empty());
        assert!(!snapshot.schema.is_empty());
        write_json_atomic(
            &self.machine_path(&machine_id, &["status", &format!("{state_key}.json")]),
            &snapshot,
        )?;
        Ok(snapshot)
    }

    pub fn read_status_snapshot(
        &self,
        machine_id: &str,
        state_key: &str,
    ) -> Result<Option<RelayStatusSnapshot>> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let state_key = safe_kind(state_key, "state key")?;
        validate_status_snapshot_label("state key", &state_key)?;
        let path = self.machine_path(&machine_id, &["status", &format!("{state_key}.json")]);
        if !path.exists() {
            return Ok(None);
        }
        read_json_file(&path).map(Some)
    }

    pub fn create_event(
        &self,
        machine_id: &str,
        input: &CreateRelayEventInput,
    ) -> Result<RelayEvent> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let lane = safe_kind(&input.lane, "lane")?;
        let input_kind = safe_kind(&input.kind, "kind")?;
        let kind = relay_normalized_kind(&lane, &input_kind);
        if relay_command_is_removed_chat_rpc(&kind) {
            bail!("{kind} moved to the finite chat room log");
        }
        let bridge = input.bridge.clone();
        if let Some(bridge) = &bridge {
            let _bridge_state_dir = self.ensure_hosted_bridge_state_dir(&machine_id, bridge)?;
        }
        let Some(scope) = input.scope.clone() else {
            bail!("{kind} requires explicit command scope");
        };
        scope.validate_for_machine(&machine_id)?;
        let now = now_iso()?;
        let ttl_secs = input
            .ttl_secs
            .unwrap_or(DEFAULT_EVENT_TTL_SECS)
            .clamp(1, MAX_EVENT_TTL_SECS);
        let mut event = RelayEvent {
            id: relay_event_id(),
            machine_id: machine_id.clone(),
            lane,
            kind,
            bridge,
            command_envelope: None,
            scope: Some(scope),
            payload: input.payload.clone(),
            created_at: now,
            expires_at: (OffsetDateTime::now_utc() + TimeDuration::seconds(ttl_secs as i64))
                .format(&Rfc3339)?,
        };
        event.command_envelope = Some(relay_command_envelope_from_event(&event)?);
        self.finitechat_command_ledger
            .append_command_request(&event)?;
        Ok(event)
    }

    pub fn claim_events(
        &self,
        machine_id: &str,
        after: Option<&str>,
        limit: Option<usize>,
    ) -> Result<RelayEventsOutput> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let after = after
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_segment(value, "event cursor"))
            .transpose()?
            .unwrap_or_default();
        let limit = limit.unwrap_or(10).clamp(1, MAX_RELAY_EVENTS);
        self.claim_finitechat_command_events(&machine_id, &after, limit)
    }

    pub fn ack_event(&self, machine_id: &str, event_id: &str) -> Result<RelayAckOutput> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let event_id = safe_segment(event_id, "event id")?;
        Ok(RelayAckOutput {
            ok: true,
            machine_id,
            event_id,
        })
    }

    pub fn store_result(
        &self,
        machine_id: &str,
        input: &StoreRelayResultInput,
    ) -> Result<RelayResultAck> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let event_id = safe_segment(&input.event_id, "event id")?;
        let result = RelayResult {
            event_id: event_id.clone(),
            machine_id,
            ok: input.ok,
            output: input.output.clone(),
            error: input.error.clone(),
            created_at: now_iso()?,
        };
        self.finitechat_command_ledger
            .append_command_result(&result)?;
        Ok(RelayResultAck { ok: true, event_id })
    }

    pub fn wait_result(&self, machine_id: &str, event_id: &str) -> Result<Option<RelayResult>> {
        let machine_id = safe_segment(machine_id, "machine id")?;
        let event_id = safe_segment(event_id, "event id")?;
        self.finitechat_command_ledger
            .command_result(&machine_id, &event_id)
    }

    fn claim_finitechat_command_events(
        &self,
        machine_id: &str,
        after: &str,
        limit: usize,
    ) -> Result<RelayEventsOutput> {
        assert!(!machine_id.is_empty());
        assert!(limit > 0);
        let pending = self
            .finitechat_command_ledger
            .pending_command_events(machine_id)?;
        let mut events = Vec::with_capacity(limit);
        for event in pending {
            if !after.is_empty() && event.id.as_str() <= after {
                continue;
            }
            if is_expired(&event.expires_at) {
                continue;
            }
            if !self
                .finitechat_command_ledger
                .append_command_lease(&event)?
            {
                continue;
            }
            events.push(event);
            if events.len() >= limit {
                break;
            }
        }
        assert!(events.len() <= limit);
        events.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(RelayEventsOutput {
            machine_id: machine_id.to_string(),
            events,
        })
    }

    fn machine_path(&self, machine_id: &str, parts: &[&str]) -> PathBuf {
        let mut path = self.root.join("machines").join(machine_id);
        for part in parts {
            path.push(part);
        }
        path
    }
}

impl FiniteChatCommandLedger {
    fn append_command_request(&self, event: &RelayEvent) -> Result<()> {
        assert!(!event.id.is_empty());
        assert!(!event.machine_id.is_empty());
        if self
            .command_projection(&event.machine_id)?
            .requests
            .contains_key(&event.id)
        {
            return Ok(());
        }
        let seed = finitechat_relay_room_seed(&event.machine_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let mut store = SqliteDeliveryStore::open(&self.db_path)?;
        finitechat_ensure_local_room_ready(&mut store, &seed)?;
        let room = store
            .room(&seed.room_id)?
            .ok_or_else(|| anyhow::anyhow!("finitechat command room missing"))?;
        assert!(room.current_epoch > 0);
        let envelope = relay_command_envelope_from_event(event)?;
        let command = envelope.command.clone();
        let args = serde_json::to_value(envelope)?;
        let accepted = finitechat_push_app_event_to_store(
            &mut store,
            FiniteChatAppendInput {
                room_id: seed.room_id,
                mls_group_id: seed.mls_group_id,
                epoch: room.current_epoch,
                sender: seed.user_device,
                conversation_id: None,
                idempotency_key: finitechat_protocol_object_id("relay-cmd-req", &event.id),
                payload: FiniteChatEventPayload::RuntimeCommandRequest(
                    FiniteChatCommandRequestPayload {
                        request_id: event.id.clone(),
                        command,
                        args,
                    },
                ),
            },
        )?;
        assert!(accepted.seq > 0);
        Ok(())
    }

    fn append_command_lease(&self, event: &RelayEvent) -> Result<bool> {
        assert!(!event.id.is_empty());
        assert!(!event.machine_id.is_empty());
        let now_ms = now_millis();
        if event_expires_at_ms(event)? <= now_ms {
            return Ok(false);
        }
        let projection = self.command_projection(&event.machine_id)?;
        if projection.results.contains_key(&event.id) {
            return Ok(false);
        }
        if projection
            .leases
            .get(&event.id)
            .is_some_and(|(_, lease)| lease.is_active_at(now_ms))
        {
            return Ok(false);
        }
        self.append_command_lease_at(event, now_ms)
    }

    fn append_command_lease_at(&self, event: &RelayEvent, claimed_at_ms: u64) -> Result<bool> {
        assert!(!event.id.is_empty());
        assert!(!event.machine_id.is_empty());
        let expires_at_ms = command_lease_expires_at_ms(event, claimed_at_ms)?;
        if expires_at_ms <= claimed_at_ms {
            return Ok(false);
        }
        let lease = RelayCommandLease {
            request_id: event.id.clone(),
            lease_id: relay_event_id(),
            worker_id: format!("finitec:{}", event.machine_id),
            claimed_at_ms,
            expires_at_ms,
        };
        lease.validate()?;
        let status_payload = serde_json::to_vec(&lease)?;
        if status_payload.len() > MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES as usize {
            bail!(
                "runtime command lease payload is {} bytes, max is {}",
                status_payload.len(),
                MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES
            );
        }
        let state = RuntimeStateSnapshotV1 {
            state_key: COMMAND_LEASE_STATE_KEY.to_string(),
            schema: COMMAND_LEASE_STATE_SCHEMA.to_string(),
            revision: claimed_at_ms,
            observed_at_ms: claimed_at_ms,
            expires_at_ms,
            status_payload,
        };
        state.validate_limits().map_err(anyhow::Error::from)?;
        let seed = finitechat_relay_room_seed(&event.machine_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let mut store = SqliteDeliveryStore::open(&self.db_path)?;
        finitechat_ensure_local_room_ready(&mut store, &seed)?;
        let room = store
            .room(&seed.room_id)?
            .ok_or_else(|| anyhow::anyhow!("finitechat command room missing"))?;
        assert!(room.current_epoch > 0);
        let accepted = finitechat_push_app_event_to_store(
            &mut store,
            FiniteChatAppendInput {
                room_id: seed.room_id,
                mls_group_id: seed.mls_group_id,
                epoch: room.current_epoch,
                sender: seed.runtime_device,
                conversation_id: None,
                idempotency_key: finitechat_protocol_object_id(
                    "relay-cmd-lease",
                    &format!("{}:{}", event.id, lease.lease_id),
                ),
                payload: FiniteChatEventPayload::RuntimeStateSnapshot(state),
            },
        )?;
        assert!(accepted.seq > 0);
        Ok(true)
    }

    fn append_command_result(&self, result: &RelayResult) -> Result<()> {
        assert!(!result.event_id.is_empty());
        assert!(!result.machine_id.is_empty());
        if self
            .command_result(&result.machine_id, &result.event_id)?
            .is_some()
        {
            return Ok(());
        }
        let seed = finitechat_relay_room_seed(&result.machine_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let mut store = SqliteDeliveryStore::open(&self.db_path)?;
        finitechat_ensure_local_room_ready(&mut store, &seed)?;
        let room = store
            .room(&seed.room_id)?
            .ok_or_else(|| anyhow::anyhow!("finitechat command room missing"))?;
        assert!(room.current_epoch > 0);
        let status = if result.ok {
            FiniteChatCommandStatus::Succeeded
        } else {
            FiniteChatCommandStatus::Failed
        };
        let output = if result.ok {
            result.output.clone()
        } else {
            json!({ "error": result.error.clone().unwrap_or_else(|| "relay command failed".to_string()) })
        };
        let append_result = finitechat_push_app_event_to_store(
            &mut store,
            FiniteChatAppendInput {
                room_id: seed.room_id,
                mls_group_id: seed.mls_group_id,
                epoch: room.current_epoch,
                sender: seed.runtime_device,
                conversation_id: None,
                idempotency_key: finitechat_protocol_object_id(
                    "relay-cmd-result",
                    &result.event_id,
                ),
                payload: FiniteChatEventPayload::RuntimeCommandResult(
                    FiniteChatCommandResultPayload {
                        request_id: result.event_id.clone(),
                        status,
                        result: output,
                    },
                ),
            },
        );
        let accepted = match append_result {
            Ok(accepted) => accepted,
            Err(error) if is_finitechat_idempotency_conflict(&error) => {
                if self
                    .command_result(&result.machine_id, &result.event_id)?
                    .is_some()
                {
                    return Ok(());
                }
                return Err(error.into());
            }
            Err(error) => return Err(error.into()),
        };
        assert!(accepted.seq > 0);
        Ok(())
    }

    fn pending_command_events(&self, machine_id: &str) -> Result<Vec<RelayEvent>> {
        assert!(!machine_id.is_empty());
        let projection = self.command_projection(machine_id)?;
        let now_ms = now_millis();

        let mut pending = projection
            .requests
            .into_iter()
            .filter_map(|(request_id, (seq, event))| {
                if projection.results.contains_key(&request_id) {
                    return None;
                }
                if projection
                    .leases
                    .get(&request_id)
                    .is_some_and(|(_, lease)| lease.is_active_at(now_ms))
                {
                    return None;
                }
                Some((seq, event))
            })
            .collect::<Vec<_>>();
        pending.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.id.cmp(&right.1.id))
        });
        Ok(pending.into_iter().map(|(_, event)| event).collect())
    }

    fn command_result(&self, machine_id: &str, event_id: &str) -> Result<Option<RelayResult>> {
        assert!(!machine_id.is_empty());
        assert!(!event_id.is_empty());
        let projection = self.command_projection(machine_id)?;
        Ok(projection
            .results
            .get(event_id)
            .map(|(_, result)| result.clone()))
    }

    fn command_projection(&self, machine_id: &str) -> Result<FiniteChatCommandProjection> {
        assert!(!machine_id.is_empty());
        let seed = finitechat_relay_room_seed(machine_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let store = SqliteDeliveryStore::open(&self.db_path)?;
        let Some(room) = store.room(&seed.room_id)? else {
            return Ok(FiniteChatCommandProjection::default());
        };
        assert!(room.current_epoch > 0);
        if room.log.len() > MAX_COMMAND_LEDGER_SCAN_ENTRIES {
            bail!(
                "finitechat command ledger has {} entries, max scan is {}",
                room.log.len(),
                MAX_COMMAND_LEDGER_SCAN_ENTRIES
            );
        }

        let mut projection = FiniteChatCommandProjection::default();
        for entry in room
            .log
            .iter()
            .filter(|entry| entry.kind == LogEntryKind::Application)
        {
            let app_event: DecryptedApplicationEventV1 =
                serde_json::from_slice(&entry.envelope.payload)?;
            match app_event.kind {
                DurableAppEventKind::RuntimeCommandRequest => {
                    let payload: FiniteChatEventPayload =
                        serde_json::from_slice(&app_event.payload)?;
                    let FiniteChatEventPayload::RuntimeCommandRequest(request) = payload else {
                        bail!("runtime command request payload kind mismatch");
                    };
                    let Some(relay_event) = relay_event_from_command_request(machine_id, &request)?
                    else {
                        continue;
                    };
                    projection
                        .requests
                        .entry(relay_event.id.clone())
                        .or_insert((entry.seq, relay_event));
                }
                DurableAppEventKind::RuntimeCommandResult => {
                    let payload: FiniteChatEventPayload =
                        serde_json::from_slice(&app_event.payload)?;
                    let FiniteChatEventPayload::RuntimeCommandResult(result) = payload else {
                        bail!("runtime command result payload kind mismatch");
                    };
                    let relay_result = relay_result_from_command_result(machine_id, &result)?;
                    projection
                        .results
                        .entry(relay_result.event_id.clone())
                        .or_insert((entry.seq, relay_result));
                }
                DurableAppEventKind::RuntimeStateSnapshot => {
                    let payload: FiniteChatEventPayload =
                        serde_json::from_slice(&app_event.payload)?;
                    let FiniteChatEventPayload::RuntimeStateSnapshot(snapshot) = payload else {
                        bail!("runtime state snapshot payload kind mismatch");
                    };
                    if snapshot.state_key != COMMAND_LEASE_STATE_KEY {
                        continue;
                    }
                    if snapshot.schema != COMMAND_LEASE_STATE_SCHEMA {
                        bail!(
                            "runtime command lease schema is {}, expected {}",
                            snapshot.schema,
                            COMMAND_LEASE_STATE_SCHEMA
                        );
                    }
                    snapshot.validate_limits().map_err(anyhow::Error::from)?;
                    let lease = relay_command_lease_from_snapshot(snapshot)?;
                    let should_replace = projection
                        .leases
                        .get(&lease.request_id)
                        .map(|(existing_seq, _)| entry.seq > *existing_seq)
                        .unwrap_or(true);
                    if should_replace {
                        projection
                            .leases
                            .insert(lease.request_id.clone(), (entry.seq, lease));
                    }
                }
                _ => {}
            }
        }
        assert!(projection.requests.len() <= room.log.len());
        assert!(projection.results.len() <= room.log.len());
        assert!(projection.leases.len() <= room.log.len());
        Ok(projection)
    }
}

impl FiniteChatChatLedger {
    fn store_chat_bootstrap_metadata(&self, machine_id: &str, snapshot: &Value) -> Result<usize> {
        assert!(!machine_id.is_empty());
        let project_agent_ids = chat_bootstrap_project_agent_ids(snapshot)?;
        let metadata = chat_bootstrap_metadata_value(machine_id, snapshot)?;
        let encoded = serde_json::to_vec(&metadata)?;
        if encoded.len() > MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES as usize {
            bail!(
                "chat bootstrap metadata is {} bytes, max is {}",
                encoded.len(),
                MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES
            );
        }
        let observed_at_ms = now_millis();
        let state = RuntimeStateSnapshotV1 {
            state_key: CHAT_BOOTSTRAP_STATE_KEY.to_string(),
            schema: CHAT_BOOTSTRAP_STATE_SCHEMA.to_string(),
            revision: observed_at_ms,
            observed_at_ms,
            expires_at_ms: observed_at_ms.saturating_add(CHAT_BOOTSTRAP_METADATA_TTL_MS),
            status_payload: encoded,
        };
        state.validate_limits().map_err(anyhow::Error::from)?;
        let mut store = SqliteDeliveryStore::open(&self.db_path)?;
        let mut stored = 0_usize;
        for project_agent_id in project_agent_ids
            .iter()
            .take(MAX_CHAT_BOOTSTRAP_PROJECT_AGENTS)
        {
            let seed = finitechat_chat_room_seed(machine_id, project_agent_id);
            seed.validate().map_err(anyhow::Error::from)?;
            finitechat_ensure_local_room_ready(&mut store, &seed)?;
            let room = store
                .room(&seed.room_id)?
                .ok_or_else(|| anyhow::anyhow!("finitechat chat room missing"))?;
            assert!(room.current_epoch > 0);
            let idempotency_key =
                finitechat_chat_bootstrap_idempotency_key(machine_id, project_agent_id, &metadata)?;
            if room
                .log
                .iter()
                .any(|entry| entry.idempotency_key == idempotency_key)
            {
                continue;
            }
            let accepted = finitechat_push_app_event_to_store(
                &mut store,
                FiniteChatAppendInput {
                    room_id: seed.room_id,
                    mls_group_id: seed.mls_group_id,
                    epoch: room.current_epoch,
                    sender: seed.runtime_device,
                    conversation_id: None,
                    idempotency_key,
                    payload: FiniteChatEventPayload::RuntimeStateSnapshot(state.clone()),
                },
            )?;
            assert!(accepted.seq > 0);
            stored += 1;
        }
        assert!(stored <= project_agent_ids.len());
        Ok(stored)
    }

    fn chat_bootstrap_snapshot(&self, machine_id: &str) -> Result<Option<RelayChatSnapshot>> {
        assert!(!machine_id.is_empty());
        let mut latest = None::<(u64, u64, Value)>;
        for room in self.chat_rooms()? {
            if room.log.len() > MAX_CHAT_LEDGER_SCAN_ENTRIES {
                bail!(
                    "finitechat chat ledger has {} entries, max scan is {}",
                    room.log.len(),
                    MAX_CHAT_LEDGER_SCAN_ENTRIES
                );
            }
            for entry in room
                .log
                .iter()
                .filter(|entry| entry.kind == LogEntryKind::Application)
            {
                let app_event: DecryptedApplicationEventV1 =
                    serde_json::from_slice(&entry.envelope.payload)?;
                if app_event.kind != DurableAppEventKind::RuntimeStateSnapshot {
                    continue;
                }
                let payload: FiniteChatEventPayload = serde_json::from_slice(&app_event.payload)?;
                let FiniteChatEventPayload::RuntimeStateSnapshot(snapshot) = payload else {
                    bail!("runtime state snapshot payload kind mismatch");
                };
                if snapshot.state_key != CHAT_BOOTSTRAP_STATE_KEY {
                    continue;
                }
                if snapshot.schema != CHAT_BOOTSTRAP_STATE_SCHEMA {
                    bail!(
                        "chat bootstrap metadata schema is {}, expected {}",
                        snapshot.schema,
                        CHAT_BOOTSTRAP_STATE_SCHEMA
                    );
                }
                snapshot.validate_limits().map_err(anyhow::Error::from)?;
                let metadata: Value = serde_json::from_slice(&snapshot.status_payload)
                    .context("chat bootstrap metadata payload is invalid JSON")?;
                if metadata.get("machine_id").and_then(Value::as_str) != Some(machine_id) {
                    continue;
                }
                let should_replace = latest
                    .as_ref()
                    .map(|(observed_at_ms, seq, _)| {
                        snapshot.observed_at_ms > *observed_at_ms
                            || (snapshot.observed_at_ms == *observed_at_ms && entry.seq > *seq)
                    })
                    .unwrap_or(true);
                if should_replace {
                    latest = Some((snapshot.observed_at_ms, entry.seq, metadata));
                }
            }
        }
        let Some((_, _, snapshot)) = latest else {
            return Ok(None);
        };
        Ok(Some(RelayChatSnapshot {
            ok: true,
            machine_id: machine_id.to_string(),
            snapshot,
            updated_at: now_iso()?,
        }))
    }

    fn chat_inbox(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<ChatInboxEvent>> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        assert!(limit > 0);
        assert!(limit <= MAX_CHAT_INBOX_EVENTS);
        let seed = finitechat_chat_room_seed(machine_id, project_agent_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let store = SqliteDeliveryStore::open(&self.db_path)?;
        let Some(room) = store.room(&seed.room_id)? else {
            return Ok(Vec::new());
        };
        assert!(room.current_epoch > 0);
        if room.log.len() > MAX_CHAT_LEDGER_SCAN_ENTRIES {
            bail!(
                "finitechat chat ledger has {} entries, max scan is {}",
                room.log.len(),
                MAX_CHAT_LEDGER_SCAN_ENTRIES
            );
        }

        let mut events = Vec::with_capacity(limit.min(room.log.len()));
        for entry in room
            .log
            .iter()
            .filter(|entry| entry.kind == LogEntryKind::Application)
        {
            if entry.seq <= after_seq {
                continue;
            }
            if events.len() >= limit {
                break;
            }
            if entry.sender != seed.user_device {
                continue;
            }
            let app_event: DecryptedApplicationEventV1 =
                serde_json::from_slice(&entry.envelope.payload)?;
            match app_event.kind {
                DurableAppEventKind::ConversationCreate
                | DurableAppEventKind::ConversationUpdate => {
                    let payload: FiniteChatEventPayload =
                        serde_json::from_slice(&app_event.payload)?;
                    let Some(conversation) =
                        chat_thread_from_finitechat_payload(machine_id, &app_event, payload)?
                    else {
                        continue;
                    };
                    events.push(ChatInboxEvent {
                        seq: entry.seq,
                        kind: durable_chat_event_kind_name(&app_event.kind),
                        project_agent_id: conversation.project_agent_id.clone(),
                        conversation_id: conversation.id.clone(),
                        conversation: Some(conversation),
                        message: None,
                    });
                }
                DurableAppEventKind::ChatMessage => {
                    let payload: FiniteChatEventPayload =
                        serde_json::from_slice(&app_event.payload)?;
                    let FiniteChatEventPayload::ChatMessage(message_payload) = payload else {
                        bail!("chat message payload kind mismatch");
                    };
                    let Some(conversation_id) = app_event.conversation_id.as_deref() else {
                        bail!("user chat inbox message is missing conversation id");
                    };
                    let message = chat_message_from_finitechat_payload(
                        entry.seq,
                        conversation_id,
                        message_payload,
                    );
                    if message.sender_type != "user" {
                        bail!("user chat inbox contained non-user message {}", message.id);
                    }
                    events.push(ChatInboxEvent {
                        seq: entry.seq,
                        kind: "chat.message".to_string(),
                        project_agent_id: project_agent_id.to_string(),
                        conversation_id: conversation_id.to_string(),
                        conversation: None,
                        message: Some(message),
                    });
                }
                _ => {}
            }
        }
        assert!(events.len() <= limit);
        Ok(events)
    }

    fn append_entries(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        threads: &[ChatThread],
        messages: &[ChatMessage],
    ) -> Result<(usize, usize)> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        assert!(threads.len() <= MAX_CHAT_LOG_SYNC_THREADS_PER_REQUEST);
        assert!(messages.len() <= MAX_CHAT_LOG_SYNC_MESSAGES_PER_REQUEST);
        let seed = finitechat_chat_room_seed(machine_id, project_agent_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let mut store = SqliteDeliveryStore::open(&self.db_path)?;
        finitechat_ensure_local_room_ready(&mut store, &seed)?;
        let room = store
            .room(&seed.room_id)?
            .ok_or_else(|| anyhow::anyhow!("finitechat chat room missing"))?;
        assert!(room.current_epoch > 0);
        let mut known_thread_keys =
            self.chat_thread_idempotency_keys(machine_id, project_agent_id)?;
        let mut known_message_keys =
            self.chat_message_idempotency_keys(machine_id, project_agent_id)?;
        let mut known_messages = self.chat_messages_by_id(machine_id, project_agent_id)?;
        let mut stored = 0_usize;
        let mut skipped = 0_usize;

        for thread in threads {
            if thread.project_agent_id != project_agent_id {
                skipped += 1;
                continue;
            }
            let payload = finitechat_conversation_payload_from_thread(machine_id, thread);
            let idempotency_key = finitechat_conversation_idempotency_key(&payload)?;
            if known_thread_keys.contains(&idempotency_key) {
                skipped += 1;
                continue;
            }
            let append_result = finitechat_push_app_event_to_store(
                &mut store,
                FiniteChatAppendInput {
                    room_id: seed.room_id.clone(),
                    mls_group_id: seed.mls_group_id.clone(),
                    epoch: room.current_epoch,
                    sender: seed.runtime_device.clone(),
                    conversation_id: Some(thread.id.clone()),
                    idempotency_key: idempotency_key.clone(),
                    payload: FiniteChatEventPayload::ConversationUpdate(payload),
                },
            );
            match append_result {
                Ok(accepted) => {
                    assert!(accepted.seq > 0);
                    known_thread_keys.insert(idempotency_key);
                    stored += 1;
                }
                Err(error) if is_finitechat_idempotency_conflict(&error) => {
                    if self
                        .chat_thread_idempotency_keys(machine_id, project_agent_id)?
                        .contains(&idempotency_key)
                    {
                        known_thread_keys.insert(idempotency_key);
                        skipped += 1;
                        continue;
                    }
                    return Err(error.into());
                }
                Err(error) => return Err(error.into()),
            }
        }

        for message in messages {
            if !message_is_finitechat_loggable(message) {
                skipped += 1;
                continue;
            }
            let Some(sender) = finitechat_chat_sender_for_message(&seed, message) else {
                skipped += 1;
                continue;
            };
            let payload = finitechat_chat_message_payload(message);
            let idempotency_key = finitechat_chat_message_idempotency_key(&payload)?;
            if known_message_keys.contains(&idempotency_key) {
                skipped += 1;
                continue;
            }
            if known_messages.get(&message.id).is_some_and(|known| {
                chat_messages_equal_for_projection(known, message)
                    || chat_messages_same_client_send(known, message)
            }) {
                skipped += 1;
                continue;
            }
            let append_result = finitechat_push_app_event_to_store(
                &mut store,
                FiniteChatAppendInput {
                    room_id: seed.room_id.clone(),
                    mls_group_id: seed.mls_group_id.clone(),
                    epoch: room.current_epoch,
                    sender,
                    conversation_id: Some(message.thread_id.clone()),
                    idempotency_key: idempotency_key.clone(),
                    payload: FiniteChatEventPayload::ChatMessage(payload),
                },
            );
            match append_result {
                Ok(accepted) => {
                    assert!(accepted.seq > 0);
                    known_message_keys.insert(idempotency_key);
                    known_messages.insert(message.id.clone(), message.clone());
                    stored += 1;
                }
                Err(error) if is_finitechat_idempotency_conflict(&error) => {
                    if self
                        .chat_message_idempotency_keys(machine_id, project_agent_id)?
                        .contains(&idempotency_key)
                    {
                        known_message_keys.insert(idempotency_key);
                        known_messages.insert(message.id.clone(), message.clone());
                        skipped += 1;
                        continue;
                    }
                    return Err(error.into());
                }
                Err(error) => return Err(error.into()),
            }
        }

        assert!(stored + skipped <= threads.len() + messages.len());
        Ok((stored, skipped))
    }

    fn append_conversation(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        thread: &ChatThread,
    ) -> Result<ChatThread> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        assert!(!thread.id.is_empty());
        if thread.project_agent_id != project_agent_id {
            bail!("conversation project agent does not match append target");
        }
        if let Some(existing) = self.chat_thread_by_id(machine_id, project_agent_id, &thread.id)? {
            validate_chat_conversation_retry_matches(&existing, thread)?;
            return Ok(existing);
        }
        let seed = finitechat_chat_room_seed(machine_id, project_agent_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let mut store = SqliteDeliveryStore::open(&self.db_path)?;
        finitechat_ensure_local_room_ready(&mut store, &seed)?;
        let room = store
            .room(&seed.room_id)?
            .ok_or_else(|| anyhow::anyhow!("finitechat chat room missing"))?;
        assert!(room.current_epoch > 0);
        let payload = finitechat_conversation_payload_from_thread(machine_id, thread);
        let idempotency_key = finitechat_conversation_idempotency_key(&payload)?;
        let append_result = finitechat_push_app_event_to_store(
            &mut store,
            FiniteChatAppendInput {
                room_id: seed.room_id,
                mls_group_id: seed.mls_group_id,
                epoch: room.current_epoch,
                sender: seed.user_device,
                conversation_id: Some(thread.id.clone()),
                idempotency_key,
                payload: FiniteChatEventPayload::ConversationCreate(payload),
            },
        );
        match append_result {
            Ok(accepted) => {
                assert!(accepted.seq > 0);
                Ok(thread.clone())
            }
            Err(error) if is_finitechat_idempotency_conflict(&error) => {
                let existing = self
                    .chat_thread_by_id(machine_id, project_agent_id, &thread.id)?
                    .ok_or(error)?;
                validate_chat_conversation_retry_matches(&existing, thread)?;
                Ok(existing)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn append_conversation_update(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        thread: &ChatThread,
    ) -> Result<ChatThread> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        assert!(!thread.id.is_empty());
        if thread.project_agent_id != project_agent_id {
            bail!("conversation update project agent does not match append target");
        }
        let seed = finitechat_chat_room_seed(machine_id, project_agent_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let mut store = SqliteDeliveryStore::open(&self.db_path)?;
        finitechat_ensure_local_room_ready(&mut store, &seed)?;
        let room = store
            .room(&seed.room_id)?
            .ok_or_else(|| anyhow::anyhow!("finitechat chat room missing"))?;
        assert!(room.current_epoch > 0);
        let payload = finitechat_conversation_payload_from_thread(machine_id, thread);
        let idempotency_key = finitechat_conversation_idempotency_key(&payload)?;
        let append_result = finitechat_push_app_event_to_store(
            &mut store,
            FiniteChatAppendInput {
                room_id: seed.room_id,
                mls_group_id: seed.mls_group_id,
                epoch: room.current_epoch,
                sender: seed.user_device,
                conversation_id: Some(thread.id.clone()),
                idempotency_key,
                payload: FiniteChatEventPayload::ConversationUpdate(payload),
            },
        );
        match append_result {
            Ok(accepted) => {
                assert!(accepted.seq > 0);
                Ok(thread.clone())
            }
            Err(error) if is_finitechat_idempotency_conflict(&error) => {
                let existing = self
                    .chat_thread_by_id(machine_id, project_agent_id, &thread.id)?
                    .ok_or(error)?;
                validate_chat_conversation_retry_matches(&existing, thread)?;
                Ok(existing)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn append_user_message(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        message: &ChatMessage,
    ) -> Result<()> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        assert!(!message.id.is_empty());
        if message.sender_type != "user" {
            bail!("finitechat-first send only accepts user messages");
        }
        if message.thread_id.trim().is_empty() {
            bail!("finitechat-first send needs conversation id");
        }
        let seed = finitechat_chat_room_seed(machine_id, project_agent_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let mut store = SqliteDeliveryStore::open(&self.db_path)?;
        finitechat_ensure_local_room_ready(&mut store, &seed)?;
        let room = store
            .room(&seed.room_id)?
            .ok_or_else(|| anyhow::anyhow!("finitechat chat room missing"))?;
        assert!(room.current_epoch > 0);
        let payload = finitechat_chat_message_payload(message);
        let idempotency_key = finitechat_chat_message_idempotency_key(&payload)?;
        let append_result = finitechat_push_app_event_to_store(
            &mut store,
            FiniteChatAppendInput {
                room_id: seed.room_id,
                mls_group_id: seed.mls_group_id,
                epoch: room.current_epoch,
                sender: seed.user_device,
                conversation_id: Some(message.thread_id.clone()),
                idempotency_key,
                payload: FiniteChatEventPayload::ChatMessage(payload),
            },
        );
        match append_result {
            Ok(accepted) => {
                assert!(accepted.seq > 0);
                Ok(())
            }
            Err(error) if is_finitechat_idempotency_conflict(&error) => {
                let existing =
                    self.chat_message_by_id(machine_id, project_agent_id, &message.id)?;
                if existing
                    .as_ref()
                    .is_some_and(|existing| chat_messages_same_client_send(existing, message))
                {
                    return Ok(());
                }
                Err(error.into())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn chat_message_page(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        conversation_id: &str,
        limit: usize,
        before: Option<&str>,
    ) -> Result<ChatMessagePage> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        assert!(!conversation_id.is_empty());
        assert!(limit > 0);
        let before = parse_chat_message_before_cursor(before)?;
        let mut messages = self
            .chat_messages(machine_id, project_agent_id)?
            .into_iter()
            .filter(|(_, message)| message.thread_id == conversation_id)
            .filter(|(_, message)| chat_message_is_before_cursor(message, before.as_ref()))
            .collect::<Vec<_>>();
        messages.sort_by(|left, right| {
            left.1
                .created_at
                .cmp(&right.1.created_at)
                .then_with(|| left.1.id.cmp(&right.1.id))
                .then_with(|| left.0.cmp(&right.0))
        });
        let start = messages.len().saturating_sub(limit.saturating_add(1));
        let mut page = messages
            .into_iter()
            .skip(start)
            .map(|(_, message)| message)
            .collect::<Vec<_>>();
        let has_more = page.len() > limit;
        if has_more {
            page.remove(0);
        }
        let next_before = has_more
            .then(|| page.first().map(chat_message_before_cursor))
            .flatten();
        assert!(page.len() <= limit);
        Ok(ChatMessagePage {
            messages: page,
            has_more,
            next_before,
        })
    }

    fn chat_message_by_id(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        message_id: &str,
    ) -> Result<Option<ChatMessage>> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        assert!(!message_id.is_empty());
        Ok(self
            .chat_messages(machine_id, project_agent_id)?
            .into_iter()
            .map(|(_, message)| message)
            .find(|message| message.id == message_id))
    }

    fn chat_attachment_record(
        &self,
        machine_id: &str,
        attachment_id: &str,
    ) -> Result<Option<RelayChatAttachmentRecord>> {
        assert!(!machine_id.is_empty());
        assert!(!attachment_id.is_empty());
        let mut seen_project_agents = BTreeSet::new();
        for thread in self.chat_threads(machine_id)? {
            if !seen_project_agents.insert(thread.project_agent_id.clone()) {
                continue;
            }
            for (_, message) in self.chat_messages(machine_id, &thread.project_agent_id)? {
                if let Some(record) =
                    chat_attachment_record_from_metadata(&message.metadata, attachment_id)?
                {
                    return Ok(Some(record));
                }
            }
        }
        Ok(None)
    }

    fn chat_stream_messages(
        &self,
        machine_id: &str,
        threads: &[ChatThread],
    ) -> Result<Vec<ChatMessage>> {
        assert!(!machine_id.is_empty());
        let mut messages = Vec::new();
        for thread in threads.iter().take(MAX_CHAT_STREAM_MESSAGE_THREADS) {
            let page = self.chat_message_page(
                machine_id,
                &thread.project_agent_id,
                &thread.id,
                MAX_CHAT_STREAM_MESSAGES_PER_THREAD,
                None,
            )?;
            messages.extend(page.messages);
            if messages.len()
                > MAX_CHAT_STREAM_MESSAGE_THREADS * MAX_CHAT_STREAM_MESSAGES_PER_THREAD
            {
                bail!("finitechat stream materialization exceeded bounded message count");
            }
        }
        messages.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        if messages.len() > MAX_CHAT_STREAM_MESSAGES_TOTAL {
            messages = messages.split_off(messages.len() - MAX_CHAT_STREAM_MESSAGES_TOTAL);
        }
        assert!(messages.len() <= MAX_CHAT_STREAM_MESSAGES_TOTAL);
        Ok(messages)
    }

    fn chat_stream_cursor(
        &self,
        machine_id: &str,
        threads: &[ChatThread],
    ) -> Result<ChatStreamCursor> {
        assert!(!machine_id.is_empty());
        let mut rooms = BTreeMap::new();
        let mut seen_project_agents = BTreeSet::new();
        for thread in threads.iter().take(MAX_CHAT_STREAM_MESSAGE_THREADS) {
            if !seen_project_agents.insert(thread.project_agent_id.clone()) {
                continue;
            }
            let update =
                self.chat_stream_room_update(machine_id, &thread.project_agent_id, u64::MAX)?;
            rooms.insert(update.room_id, update.current_seq);
            if rooms.len() > MAX_CHAT_STREAM_CURSOR_ROOMS {
                bail!(
                    "chat stream cursor has more than {} rooms",
                    MAX_CHAT_STREAM_CURSOR_ROOMS
                );
            }
        }
        Ok(ChatStreamCursor { rooms })
    }

    fn chat_stream_messages_after(
        &self,
        machine_id: &str,
        threads: &[ChatThread],
        since: &ChatStreamCursor,
    ) -> Result<ChatStreamMaterialization> {
        assert!(!machine_id.is_empty());
        if since.rooms.len() > MAX_CHAT_STREAM_CURSOR_ROOMS {
            bail!(
                "chat stream cursor has more than {} rooms",
                MAX_CHAT_STREAM_CURSOR_ROOMS
            );
        }
        let mut cursor = ChatStreamCursor {
            rooms: BTreeMap::new(),
        };
        let mut messages = Vec::new();
        let mut seen_project_agents = BTreeSet::new();
        for thread in threads.iter().take(MAX_CHAT_STREAM_MESSAGE_THREADS) {
            if !seen_project_agents.insert(thread.project_agent_id.clone()) {
                continue;
            }
            let seed = finitechat_chat_room_seed(machine_id, &thread.project_agent_id);
            let after_seq = since.rooms.get(&seed.room_id).copied().unwrap_or(0);
            let update =
                self.chat_stream_room_update(machine_id, &thread.project_agent_id, after_seq)?;
            cursor
                .rooms
                .insert(update.room_id.clone(), update.current_seq);
            messages.extend(update.messages);
            if cursor.rooms.len() > MAX_CHAT_STREAM_CURSOR_ROOMS {
                bail!(
                    "chat stream cursor has more than {} rooms",
                    MAX_CHAT_STREAM_CURSOR_ROOMS
                );
            }
            if messages.len() > MAX_CHAT_STREAM_MESSAGES_TOTAL {
                break;
            }
        }
        messages.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(ChatStreamMaterialization { cursor, messages })
    }

    fn chat_stream_room_update(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        after_seq: u64,
    ) -> Result<ChatStreamRoomUpdate> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        let seed = finitechat_chat_room_seed(machine_id, project_agent_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let store = SqliteDeliveryStore::open(&self.db_path)?;
        let Some(room) = store.room(&seed.room_id)? else {
            return Ok(ChatStreamRoomUpdate {
                room_id: seed.room_id,
                current_seq: 0,
                messages: Vec::new(),
            });
        };
        assert!(room.current_epoch > 0);
        if room.log.len() > MAX_CHAT_LEDGER_SCAN_ENTRIES {
            bail!(
                "finitechat chat ledger has {} entries, max scan is {}",
                room.log.len(),
                MAX_CHAT_LEDGER_SCAN_ENTRIES
            );
        }
        let mut by_id = BTreeMap::<String, (u64, ChatMessage)>::new();
        for entry in room
            .log
            .iter()
            .filter(|entry| entry.kind == LogEntryKind::Application)
        {
            if entry.seq <= after_seq {
                continue;
            }
            let app_event: DecryptedApplicationEventV1 =
                serde_json::from_slice(&entry.envelope.payload)?;
            if app_event.kind != DurableAppEventKind::ChatMessage {
                continue;
            }
            let payload: FiniteChatEventPayload = serde_json::from_slice(&app_event.payload)?;
            let FiniteChatEventPayload::ChatMessage(message) = payload else {
                bail!("chat message payload kind mismatch");
            };
            let Some(conversation_id) = app_event.conversation_id else {
                continue;
            };
            let chat_message =
                chat_message_from_finitechat_payload(entry.seq, &conversation_id, message);
            let should_replace = by_id
                .get(&chat_message.id)
                .map(|(existing_seq, _)| entry.seq > *existing_seq)
                .unwrap_or(true);
            if should_replace {
                by_id.insert(chat_message.id.clone(), (entry.seq, chat_message));
            }
        }
        let messages = by_id
            .into_values()
            .map(|(_, message)| message)
            .collect::<Vec<_>>();
        Ok(ChatStreamRoomUpdate {
            room_id: seed.room_id,
            current_seq: room.last_seq,
            messages,
        })
    }

    fn chat_threads(&self, machine_id: &str) -> Result<Vec<ChatThread>> {
        assert!(!machine_id.is_empty());
        let mut by_key = BTreeMap::<(String, String), ProjectedChatThread>::new();
        for room in self.chat_rooms()? {
            if room.log.len() > MAX_CHAT_LEDGER_SCAN_ENTRIES {
                bail!(
                    "finitechat chat ledger has {} entries, max scan is {}",
                    room.log.len(),
                    MAX_CHAT_LEDGER_SCAN_ENTRIES
                );
            }
            for entry in room
                .log
                .iter()
                .filter(|entry| entry.kind == LogEntryKind::Application)
            {
                let app_event: DecryptedApplicationEventV1 =
                    serde_json::from_slice(&entry.envelope.payload)?;
                match app_event.kind.clone() {
                    DurableAppEventKind::ConversationCreate
                    | DurableAppEventKind::ConversationUpdate => {
                        let payload: FiniteChatEventPayload =
                            serde_json::from_slice(&app_event.payload)?;
                        let Some(thread) =
                            chat_thread_from_finitechat_payload(machine_id, &app_event, payload)?
                        else {
                            continue;
                        };
                        let title_is_user_authored = conversation_event_locks_title(
                            machine_id,
                            app_event.kind.clone(),
                            &entry.sender,
                            &thread,
                        )?;
                        merge_projected_chat_thread(&mut by_key, thread, title_is_user_authored);
                    }
                    DurableAppEventKind::ConversationArchive => {
                        let payload: FiniteChatEventPayload =
                            serde_json::from_slice(&app_event.payload)?;
                        let Some(thread) =
                            chat_thread_from_finitechat_payload(machine_id, &app_event, payload)?
                        else {
                            continue;
                        };
                        by_key.remove(&(thread.project_agent_id, thread.id));
                    }
                    _ => {}
                }
            }
        }

        let mut threads = by_key
            .into_values()
            .map(|projected| projected.thread)
            .collect::<Vec<_>>();
        threads.sort_by(|left, right| {
            right
                .last_activity_at
                .cmp(&left.last_activity_at)
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(threads)
    }

    fn chat_project_agent_id_for_conversation(
        &self,
        machine_id: &str,
        conversation_id: &str,
    ) -> Result<Option<String>> {
        assert!(!machine_id.is_empty());
        assert!(!conversation_id.is_empty());
        Ok(self
            .chat_threads(machine_id)?
            .into_iter()
            .find(|thread| thread.id == conversation_id)
            .map(|thread| thread.project_agent_id))
    }

    fn chat_thread_by_id(
        &self,
        machine_id: &str,
        project_agent_id: &str,
        conversation_id: &str,
    ) -> Result<Option<ChatThread>> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        assert!(!conversation_id.is_empty());
        Ok(self.chat_threads(machine_id)?.into_iter().find(|thread| {
            thread.project_agent_id == project_agent_id && thread.id == conversation_id
        }))
    }

    fn chat_message_idempotency_keys(
        &self,
        machine_id: &str,
        project_agent_id: &str,
    ) -> Result<BTreeSet<String>> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        let seed = finitechat_chat_room_seed(machine_id, project_agent_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let store = SqliteDeliveryStore::open(&self.db_path)?;
        let Some(room) = store.room(&seed.room_id)? else {
            return Ok(BTreeSet::new());
        };
        if room.log.len() > MAX_CHAT_LEDGER_SCAN_ENTRIES {
            bail!(
                "finitechat chat ledger has {} entries, max scan is {}",
                room.log.len(),
                MAX_CHAT_LEDGER_SCAN_ENTRIES
            );
        }
        let mut keys = BTreeSet::new();
        for entry in room
            .log
            .iter()
            .filter(|entry| entry.kind == LogEntryKind::Application)
        {
            let app_event: DecryptedApplicationEventV1 =
                serde_json::from_slice(&entry.envelope.payload)?;
            if app_event.kind == DurableAppEventKind::ChatMessage {
                keys.insert(entry.idempotency_key.clone());
            }
        }
        assert!(keys.len() <= room.log.len());
        Ok(keys)
    }

    fn chat_messages_by_id(
        &self,
        machine_id: &str,
        project_agent_id: &str,
    ) -> Result<BTreeMap<String, ChatMessage>> {
        let messages = self
            .chat_messages(machine_id, project_agent_id)?
            .into_iter()
            .map(|(_, message)| (message.id.clone(), message))
            .collect::<BTreeMap<_, _>>();
        Ok(messages)
    }

    fn chat_thread_idempotency_keys(
        &self,
        machine_id: &str,
        project_agent_id: &str,
    ) -> Result<BTreeSet<String>> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        let seed = finitechat_chat_room_seed(machine_id, project_agent_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let store = SqliteDeliveryStore::open(&self.db_path)?;
        let Some(room) = store.room(&seed.room_id)? else {
            return Ok(BTreeSet::new());
        };
        if room.log.len() > MAX_CHAT_LEDGER_SCAN_ENTRIES {
            bail!(
                "finitechat chat ledger has {} entries, max scan is {}",
                room.log.len(),
                MAX_CHAT_LEDGER_SCAN_ENTRIES
            );
        }
        let mut keys = BTreeSet::new();
        for entry in room
            .log
            .iter()
            .filter(|entry| entry.kind == LogEntryKind::Application)
        {
            let app_event: DecryptedApplicationEventV1 =
                serde_json::from_slice(&entry.envelope.payload)?;
            if matches!(
                app_event.kind,
                DurableAppEventKind::ConversationCreate
                    | DurableAppEventKind::ConversationUpdate
                    | DurableAppEventKind::ConversationArchive
            ) {
                keys.insert(entry.idempotency_key.clone());
            }
        }
        assert!(keys.len() <= room.log.len());
        Ok(keys)
    }

    fn chat_messages(
        &self,
        machine_id: &str,
        project_agent_id: &str,
    ) -> Result<Vec<(u64, ChatMessage)>> {
        assert!(!machine_id.is_empty());
        assert!(!project_agent_id.is_empty());
        let seed = finitechat_chat_room_seed(machine_id, project_agent_id);
        seed.validate().map_err(anyhow::Error::from)?;
        let store = SqliteDeliveryStore::open(&self.db_path)?;
        let Some(room) = store.room(&seed.room_id)? else {
            return Ok(Vec::new());
        };
        assert!(room.current_epoch > 0);
        if room.log.len() > MAX_CHAT_LEDGER_SCAN_ENTRIES {
            bail!(
                "finitechat chat ledger has {} entries, max scan is {}",
                room.log.len(),
                MAX_CHAT_LEDGER_SCAN_ENTRIES
            );
        }

        let mut by_id = BTreeMap::<String, (u64, ChatMessage)>::new();
        for entry in room
            .log
            .iter()
            .filter(|entry| entry.kind == LogEntryKind::Application)
        {
            let app_event: DecryptedApplicationEventV1 =
                serde_json::from_slice(&entry.envelope.payload)?;
            if app_event.kind != DurableAppEventKind::ChatMessage {
                continue;
            }
            let payload: FiniteChatEventPayload = serde_json::from_slice(&app_event.payload)?;
            let FiniteChatEventPayload::ChatMessage(message) = payload else {
                bail!("chat message payload kind mismatch");
            };
            let Some(conversation_id) = app_event.conversation_id else {
                continue;
            };
            let chat_message =
                chat_message_from_finitechat_payload(entry.seq, &conversation_id, message);
            let should_replace = by_id
                .get(&chat_message.id)
                .map(|(existing_seq, _)| entry.seq > *existing_seq)
                .unwrap_or(true);
            if should_replace {
                by_id.insert(chat_message.id.clone(), (entry.seq, chat_message));
            }
        }
        assert!(by_id.len() <= room.log.len());
        Ok(by_id.into_values().collect())
    }

    fn chat_rooms(&self) -> Result<Vec<finitechat_engine::RoomRecord>> {
        let store = SqliteDeliveryStore::open(&self.db_path)?;
        let mut rooms = Vec::new();
        let mut after_room_id = None;
        loop {
            let page = store.list_account_rooms(ListAccountRoomsRequest {
                account_id: finitechat_chat_user_account_id(),
                after_room_id,
                limit: 100,
            })?;
            for room in page.rooms {
                if rooms.len() >= MAX_CHAT_LEDGER_SCAN_ROOMS {
                    bail!(
                        "finitechat chat ledger has at least {} rooms, max scan is {}",
                        rooms.len() + 1,
                        MAX_CHAT_LEDGER_SCAN_ROOMS
                    );
                }
                if let Some(room) = store.room(&room.room_id)? {
                    rooms.push(room);
                }
            }
            if !page.has_more {
                break;
            }
            after_room_id = page.next_after_room_id;
        }
        assert!(rooms.len() <= MAX_CHAT_LEDGER_SCAN_ROOMS);
        Ok(rooms)
    }
}

#[derive(Debug, Default)]
struct FiniteChatCommandProjection {
    requests: BTreeMap<String, (u64, RelayEvent)>,
    results: BTreeMap<String, (u64, RelayResult)>,
    leases: BTreeMap<String, (u64, RelayCommandLease)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RelayCommandLease {
    request_id: String,
    lease_id: String,
    worker_id: String,
    claimed_at_ms: u64,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RelayCommandEnvelope {
    pub schema: String,
    pub command: String,
    pub scope: RelayCommandScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge: Option<RelayBridgeDevice>,
    #[serde(default)]
    pub payload: Value,
    pub created_at: String,
    pub expires_at: String,
}

impl RelayCommandEnvelope {
    fn validate_for_machine(&self, machine_id: &str) -> Result<()> {
        assert!(!machine_id.is_empty());
        if self.schema != COMMAND_ENVELOPE_SCHEMA {
            bail!(
                "runtime command envelope schema is {}, expected {}",
                self.schema,
                COMMAND_ENVELOPE_SCHEMA
            );
        }
        let _command = safe_kind(&self.command, "runtime command")?;
        self.scope.validate_for_machine(machine_id)?;
        let created_at_ms = iso_to_millis(&self.created_at)?;
        let expires_at_ms = iso_to_millis(&self.expires_at)?;
        if expires_at_ms <= created_at_ms {
            bail!("runtime command envelope expires before it is created");
        }
        Ok(())
    }
}

impl RelayCommandLease {
    fn validate(&self) -> Result<()> {
        if self.request_id.is_empty() {
            bail!("runtime command lease request id is required");
        }
        if self.lease_id.is_empty() {
            bail!("runtime command lease id is required");
        }
        if self.worker_id.trim().is_empty() {
            bail!("runtime command lease worker id is required");
        }
        if self.expires_at_ms <= self.claimed_at_ms {
            bail!("runtime command lease expires before it starts");
        }
        if self.expires_at_ms - self.claimed_at_ms > MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS {
            bail!(
                "runtime command lease is {}ms, max is {}ms",
                self.expires_at_ms - self.claimed_at_ms,
                MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS
            );
        }
        Ok(())
    }

    fn is_active_at(&self, now_ms: u64) -> bool {
        assert!(self.expires_at_ms > self.claimed_at_ms);
        now_ms < self.expires_at_ms
    }
}

fn relay_event_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut bytes = [0_u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    format!("{millis:016x}-{}", hex::encode(bytes))
}

fn now_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.min(u128::from(u64::MAX)) as u64
}

fn finitechat_relay_room_seed(machine_id: &str) -> FiniteChatRoomSeed {
    assert!(!machine_id.is_empty());
    let room_key = format!("{machine_id}:relay");
    FiniteChatRoomSeed {
        room_id: finitechat_protocol_object_id("room", &room_key),
        mls_group_id: finitechat_protocol_object_id("mls", &room_key),
        user_device: finitechat_device(
            env::var("FINITE_CHAT_DASHBOARD_ACCOUNT_ID")
                .unwrap_or_else(|_| "dashboard:bridge".to_string()),
            env::var("FINITE_CHAT_DASHBOARD_DEVICE_ID")
                .unwrap_or_else(|_| "web-bridge".to_string()),
        ),
        runtime_device: finitechat_device(
            env::var("FINITE_CHAT_RUNTIME_ACCOUNT_ID")
                .unwrap_or_else(|_| format!("runtime:{machine_id}")),
            env::var("FINITE_CHAT_RUNTIME_DEVICE_ID").unwrap_or_else(|_| "finitec".to_string()),
        ),
    }
}

fn finitechat_chat_room_seed(machine_id: &str, project_agent_id: &str) -> FiniteChatRoomSeed {
    assert!(!machine_id.is_empty());
    assert!(!project_agent_id.is_empty());
    let room_key = format!("{machine_id}:{project_agent_id}");
    FiniteChatRoomSeed {
        room_id: finitechat_protocol_object_id("room", &room_key),
        mls_group_id: finitechat_protocol_object_id("mls", &room_key),
        user_device: finitechat_device(
            finitechat_chat_user_account_id(),
            env::var("FINITE_CHAT_USER_DEVICE_ID")
                .unwrap_or_else(|_| "relay-chat-ingest".to_string()),
        ),
        runtime_device: finitechat_device(
            env::var("FINITE_CHAT_RUNTIME_ACCOUNT_ID")
                .unwrap_or_else(|_| format!("runtime:{project_agent_id}")),
            env::var("FINITE_CHAT_RUNTIME_DEVICE_ID").unwrap_or_else(|_| "finitec".to_string()),
        ),
    }
}

fn finitechat_chat_user_account_id() -> String {
    env::var("FINITE_CHAT_USER_ACCOUNT_ID").unwrap_or_else(|_| "relay-chat-ingest".to_string())
}

fn finitechat_conversation_payload_from_thread(
    machine_id: &str,
    thread: &ChatThread,
) -> FiniteChatConversationPayload {
    assert!(!machine_id.is_empty());
    assert!(!thread.id.is_empty());
    assert!(!thread.project_agent_id.is_empty());
    FiniteChatConversationPayload {
        conversation_id: thread.id.clone(),
        machine_id: machine_id.to_string(),
        project_agent_id: thread.project_agent_id.clone(),
        created_by: thread.created_by.clone(),
        title: thread.title.clone(),
        created_at: thread.created_at.clone(),
        last_activity_at: thread.last_activity_at.clone(),
        message_count: thread.message_count,
        archived_at: None,
    }
}

fn finitechat_conversation_idempotency_key(
    payload: &FiniteChatConversationPayload,
) -> Result<String> {
    let encoded = serde_json::to_string(payload)?;
    Ok(finitechat_protocol_object_id("relay-chat-thread", &encoded))
}

fn conversation_event_locks_title(
    machine_id: &str,
    kind: DurableAppEventKind,
    sender: &DeviceRef,
    thread: &ChatThread,
) -> Result<bool> {
    assert!(!machine_id.is_empty());
    assert!(!thread.project_agent_id.is_empty());
    let seed = finitechat_chat_room_seed(machine_id, &thread.project_agent_id);
    seed.validate().map_err(anyhow::Error::from)?;
    let sender_is_user_device = sender == &seed.user_device;
    let title_is_locked = sender_is_user_device
        && (kind == DurableAppEventKind::ConversationUpdate
            || (kind == DurableAppEventKind::ConversationCreate
                && !relay_chat_thread_title_is_auto(&thread.title)));
    Ok(title_is_locked)
}

fn merge_projected_chat_thread(
    by_key: &mut BTreeMap<(String, String), ProjectedChatThread>,
    thread: ChatThread,
    title_is_user_authored: bool,
) {
    assert!(!thread.id.is_empty());
    assert!(!thread.project_agent_id.is_empty());
    let key = (thread.project_agent_id.clone(), thread.id.clone());
    let existing_title_is_user_authored = by_key
        .get(&key)
        .map(|projected| projected.title_is_user_authored)
        .unwrap_or(false);
    if let Some(projected) = by_key.get_mut(&key)
        && projected.title_is_user_authored
        && !title_is_user_authored
    {
        let user_title = projected.thread.title.clone();
        projected.thread = thread;
        projected.thread.title = user_title;
        assert!(projected.title_is_user_authored);
        return;
    }
    by_key.insert(
        key,
        ProjectedChatThread {
            thread,
            title_is_user_authored: existing_title_is_user_authored || title_is_user_authored,
        },
    );
}

fn finitechat_chat_message_payload(message: &ChatMessage) -> FiniteChatMessagePayload {
    assert!(!message.id.is_empty());
    FiniteChatMessagePayload {
        message_id: Some(message.id.clone()),
        sender_type: Some(message.sender_type.clone()),
        kind: Some(message.kind.clone()),
        status: Some(message.status.clone()),
        body: message.body.clone(),
        metadata: message.metadata.clone(),
        created_at: Some(message.created_at.clone()),
        updated_at: Some(message.updated_at.clone()),
    }
}

fn finitechat_chat_message_idempotency_key(payload: &FiniteChatMessagePayload) -> Result<String> {
    let message_id = payload
        .message_id
        .as_deref()
        .filter(|message_id| !message_id.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("finitechat chat message idempotency key needs message id")
        })?;
    let encoded = serde_json::to_string(payload)?;
    Ok(finitechat_protocol_object_id(
        "relay-chat-msg",
        &format!("{message_id}:{encoded}"),
    ))
}

fn relay_chat_thread_title(title: Option<&str>) -> Result<String> {
    let normalized = title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("New chat")
        .to_string();
    validate_relay_chat_title(&normalized)?;
    Ok(normalized)
}

fn relay_chat_user_title(title: &str) -> Result<String> {
    let normalized = title.trim().to_string();
    if normalized.is_empty() {
        bail!("conversation title is required");
    }
    validate_relay_chat_title(&normalized)?;
    Ok(normalized)
}

fn validate_relay_chat_title(title: &str) -> Result<()> {
    assert!(!title.is_empty());
    let char_count = title.chars().count();
    if char_count > MAX_CHAT_THREAD_TITLE_CHARS {
        bail!(
            "conversation title has {} characters, max is {}",
            char_count,
            MAX_CHAT_THREAD_TITLE_CHARS
        );
    }
    Ok(())
}

fn relay_chat_thread_title_is_auto(title: &str) -> bool {
    let trimmed = title.trim();
    trimmed.is_empty() || trimmed == "New chat" || trimmed == "Untitled"
}

fn validate_chat_conversation_retry_matches(
    existing: &ChatThread,
    requested: &ChatThread,
) -> Result<()> {
    assert!(!existing.id.is_empty());
    assert!(!requested.id.is_empty());
    if existing.id != requested.id {
        bail!("conversation retry id does not match accepted conversation");
    }
    if existing.project_agent_id != requested.project_agent_id {
        bail!("conversation retry project agent does not match accepted conversation");
    }
    if existing.title != requested.title {
        bail!("conversation retry title does not match accepted conversation");
    }
    Ok(())
}

fn validate_chat_send_retry_matches(
    existing: &ChatMessage,
    conversation_id: &str,
    input: &SendChatMessageRequest,
) -> Result<()> {
    assert!(!existing.id.is_empty());
    assert!(!conversation_id.is_empty());
    if existing.thread_id != conversation_id {
        bail!("client message id already exists in a different conversation");
    }
    if existing.sender_type != "user" || existing.kind != "message" {
        bail!("client message id already exists for a non-user message");
    }
    if existing.body != input.body {
        bail!("client message id retry body does not match accepted message");
    }
    let expected_attachment_count = input.attachments.len();
    let existing_attachment_count = existing
        .metadata
        .get("attachments")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if existing_attachment_count != expected_attachment_count {
        bail!("client message id retry attachments do not match accepted message");
    }
    Ok(())
}

fn chat_messages_same_client_send(left: &ChatMessage, right: &ChatMessage) -> bool {
    left.id == right.id
        && left.thread_id == right.thread_id
        && left.sender_type == right.sender_type
        && left.kind == right.kind
        && left.status == right.status
        && left.body == right.body
        && left.metadata == right.metadata
}

fn relay_attachment_id(
    message_id: &str,
    index: usize,
    reference: &AttachmentBlobReferenceV1,
) -> String {
    assert!(!message_id.is_empty());
    assert!(!reference.ciphertext_sha256.is_empty());
    finitechat_protocol_object_id(
        "att",
        &format!("{message_id}:{index}:{}", reference.ciphertext_sha256),
    )
}

fn relay_attachment_blob_metadata(name: &str, mime_type: &str) -> Result<AttachmentBlobMetadataV1> {
    let filename = name.trim();
    if filename.is_empty() {
        bail!("attachment filename is required");
    }
    let mime_type = mime_type.trim();
    if mime_type.is_empty() {
        bail!("attachment MIME type is required");
    }
    let metadata = AttachmentBlobMetadataV1 {
        mime_type: mime_type.to_string(),
        filename: filename.to_string(),
        dimensions: None,
    };
    metadata
        .validate_limits()
        .context("attachment blob metadata exceeds limits")?;
    Ok(metadata)
}

fn relay_blob_url(sha256: &str) -> Result<String> {
    let sha256 = safe_blob_sha256(sha256)?;
    Ok(format!("{FINITE_CHAT_RELAY_BLOB_URL_PREFIX}{sha256}"))
}

fn validate_relay_attachment_base64_size(
    name: &str,
    data_base64: &str,
    max_bytes: usize,
) -> Result<()> {
    assert!(max_bytes > 0);
    let max_base64_len = max_bytes.div_ceil(3).saturating_mul(4).saturating_add(4);
    if data_base64.len() > max_base64_len {
        bail!(
            "attachment {} exceeds the {} chat attachment limit",
            name,
            format_bytes(max_bytes)
        );
    }
    Ok(())
}

fn attachment_kind(mime_type: &str) -> &str {
    if mime_type.starts_with("image/") {
        "image"
    } else if mime_type.starts_with("video/") {
        "video"
    } else if mime_type.starts_with("audio/") {
        "audio"
    } else {
        "file"
    }
}

fn format_bytes(bytes: usize) -> String {
    const MIB: usize = 1024 * 1024;
    if bytes >= MIB {
        let whole = bytes / MIB;
        let fraction = ((bytes % MIB) * 10) / MIB;
        if fraction == 0 {
            format!("{whole} MiB")
        } else {
            format!("{whole}.{fraction} MiB")
        }
    } else {
        format!("{bytes} bytes")
    }
}

fn finitechat_chat_bootstrap_idempotency_key(
    machine_id: &str,
    project_agent_id: &str,
    metadata: &Value,
) -> Result<String> {
    assert!(!machine_id.is_empty());
    assert!(!project_agent_id.is_empty());
    let encoded = serde_json::to_string(metadata)?;
    Ok(finitechat_protocol_object_id(
        "relay-chat-bootstrap",
        &format!("{machine_id}:{project_agent_id}:{encoded}"),
    ))
}

fn chat_bootstrap_metadata_value(machine_id: &str, snapshot: &Value) -> Result<Value> {
    assert!(!machine_id.is_empty());
    let Some(source) = snapshot.as_object() else {
        bail!("chat bootstrap snapshot must be a JSON object");
    };
    let mut metadata = Map::new();
    metadata.insert(
        "machine_id".to_string(),
        Value::String(machine_id.to_string()),
    );
    for key in CHAT_BOOTSTRAP_METADATA_KEYS {
        let value = source
            .get(key)
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        if !value.is_array() {
            bail!("chat bootstrap metadata field {key} must be an array");
        }
        metadata.insert(key.to_string(), value);
    }
    let value = Value::Object(metadata);
    let encoded = serde_json::to_vec(&value)?;
    if encoded.len() > MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES as usize {
        bail!(
            "chat bootstrap metadata is {} bytes, max is {}",
            encoded.len(),
            MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES
        );
    }
    Ok(value)
}

fn chat_bootstrap_project_agent_ids(snapshot: &Value) -> Result<Vec<String>> {
    let Some(source) = snapshot.as_object() else {
        bail!("chat bootstrap snapshot must be a JSON object");
    };
    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    if let Some(project_agents) = source.get("project_agents") {
        let Some(project_agents) = project_agents.as_array() else {
            bail!("chat bootstrap metadata field project_agents must be an array");
        };
        if project_agents.len() > MAX_CHAT_BOOTSTRAP_PROJECT_AGENTS {
            bail!(
                "chat bootstrap has {} project agents, max is {}",
                project_agents.len(),
                MAX_CHAT_BOOTSTRAP_PROJECT_AGENTS
            );
        }
        for project_agent in project_agents {
            let Some(project_agent_id) = project_agent
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                bail!("chat bootstrap project agent is missing id");
            };
            let project_agent_id = safe_segment(project_agent_id, "project agent id")?;
            if seen.insert(project_agent_id.clone()) {
                ids.push(project_agent_id);
            }
        }
    }
    if ids.is_empty()
        && let Some(threads) = source.get("threads")
    {
        let Some(threads) = threads.as_array() else {
            bail!("chat bootstrap metadata field threads must be an array");
        };
        if threads.len() > MAX_CHAT_LOG_SYNC_THREADS_PER_REQUEST {
            bail!(
                "chat bootstrap has {} threads, max is {}",
                threads.len(),
                MAX_CHAT_LOG_SYNC_THREADS_PER_REQUEST
            );
        }
        for thread in threads {
            let Some(project_agent_id) = thread
                .get("project_agent_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let project_agent_id = safe_segment(project_agent_id, "project agent id")?;
            if seen.insert(project_agent_id.clone()) {
                ids.push(project_agent_id);
            }
        }
    }
    if ids.is_empty() {
        bail!("chat bootstrap needs at least one project agent id");
    }
    assert!(!ids.is_empty());
    assert!(ids.len() <= MAX_CHAT_BOOTSTRAP_PROJECT_AGENTS);
    Ok(ids)
}

fn chat_messages_equal_for_projection(left: &ChatMessage, right: &ChatMessage) -> bool {
    left.id == right.id
        && left.thread_id == right.thread_id
        && left.sender_type == right.sender_type
        && left.kind == right.kind
        && left.status == right.status
        && left.body == right.body
        && left.metadata == right.metadata
        && left.created_at == right.created_at
        && left.updated_at == right.updated_at
}

fn encode_chat_stream_cursor(cursor: &ChatStreamCursor) -> Result<String> {
    if cursor.rooms.len() > MAX_CHAT_STREAM_CURSOR_ROOMS {
        bail!(
            "chat stream cursor has more than {} rooms",
            MAX_CHAT_STREAM_CURSOR_ROOMS
        );
    }
    for room_id in cursor.rooms.keys() {
        if room_id.is_empty() {
            bail!("chat stream cursor room id is empty");
        }
        if room_id.len() > MAX_OBJECT_ID_BYTES as usize {
            bail!("chat stream cursor room id exceeds max object id bytes");
        }
    }
    let encoded = serde_json::to_vec(cursor)?;
    if encoded.len() > MAX_CHAT_STREAM_CURSOR_BYTES {
        bail!(
            "chat stream cursor is {} bytes, max is {}",
            encoded.len(),
            MAX_CHAT_STREAM_CURSOR_BYTES
        );
    }
    Ok(format!(
        "{CHAT_STREAM_CURSOR_PREFIX}{}",
        BASE64_URL_SAFE_NO_PAD.encode(encoded)
    ))
}

fn decode_chat_stream_cursor(raw: &str) -> Result<ChatStreamCursor> {
    let raw = raw.trim();
    let Some(encoded) = raw.strip_prefix(CHAT_STREAM_CURSOR_PREFIX) else {
        bail!("chat stream cursor uses unsupported version");
    };
    if encoded.len() > MAX_CHAT_STREAM_CURSOR_BYTES {
        bail!(
            "chat stream cursor is {} bytes, max is {}",
            encoded.len(),
            MAX_CHAT_STREAM_CURSOR_BYTES
        );
    }
    let decoded = BASE64_URL_SAFE_NO_PAD
        .decode(encoded.as_bytes())
        .context("chat stream cursor is not valid base64url")?;
    if decoded.len() > MAX_CHAT_STREAM_CURSOR_BYTES {
        bail!(
            "chat stream cursor is {} bytes, max is {}",
            decoded.len(),
            MAX_CHAT_STREAM_CURSOR_BYTES
        );
    }
    let cursor: ChatStreamCursor =
        serde_json::from_slice(&decoded).context("chat stream cursor is invalid JSON")?;
    if cursor.rooms.len() > MAX_CHAT_STREAM_CURSOR_ROOMS {
        bail!(
            "chat stream cursor has more than {} rooms",
            MAX_CHAT_STREAM_CURSOR_ROOMS
        );
    }
    for room_id in cursor.rooms.keys() {
        if room_id.is_empty() {
            bail!("chat stream cursor room id is empty");
        }
        if room_id.len() > MAX_OBJECT_ID_BYTES as usize {
            bail!("chat stream cursor room id exceeds max object id bytes");
        }
    }
    Ok(cursor)
}

fn finitechat_chat_sender_for_message(
    seed: &FiniteChatRoomSeed,
    message: &ChatMessage,
) -> Option<finitechat_proto::DeviceRef> {
    match message.sender_type.as_str() {
        "user" => Some(seed.user_device.clone()),
        "agent" => Some(seed.runtime_device.clone()),
        _ => None,
    }
}

fn message_is_finitechat_loggable(message: &ChatMessage) -> bool {
    assert!(!message.id.is_empty());
    if !matches!(message.sender_type.as_str(), "user" | "agent") {
        return false;
    }
    if message.kind == "status" && !matches!(message.status.as_str(), "error" | "cancelled") {
        return false;
    }
    !message.body.trim().is_empty() || chat_metadata_has_attachments(&message.metadata)
}

fn chat_metadata_has_attachments(metadata: &Value) -> bool {
    metadata
        .get("attachments")
        .and_then(Value::as_array)
        .is_some_and(|attachments| !attachments.is_empty())
}

fn chat_attachment_record_from_metadata(
    metadata: &Value,
    attachment_id: &str,
) -> Result<Option<RelayChatAttachmentRecord>> {
    assert!(!attachment_id.is_empty());
    let Some(attachments) = metadata.get("attachments").and_then(Value::as_array) else {
        return Ok(None);
    };
    if attachments.len() > MAX_CHAT_ATTACHMENTS_PER_MESSAGE {
        bail!(
            "chat message contains {} attachments, max is {}",
            attachments.len(),
            MAX_CHAT_ATTACHMENTS_PER_MESSAGE
        );
    }
    for attachment in attachments {
        let Value::Object(map) = attachment else {
            continue;
        };
        if map.get("id").and_then(Value::as_str) != Some(attachment_id) {
            continue;
        }
        let Some(reference_value) = map.get(CHAT_ATTACHMENT_BLOB_METADATA_KEY) else {
            return Ok(None);
        };
        let reference: AttachmentBlobReferenceV1 = serde_json::from_value(reference_value.clone())
            .context("attachment has invalid finitechat blob reference")?;
        reference
            .validate_limits()
            .context("attachment blob reference exceeds limits")?;
        let name = map
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(reference.metadata.filename.as_str())
            .to_string();
        let mime_type = map
            .get("mime_type")
            .or_else(|| map.get("mimeType"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(reference.metadata.mime_type.as_str())
            .to_string();
        if name.trim().is_empty() {
            bail!("chat attachment {attachment_id} has an empty filename");
        }
        if mime_type.trim().is_empty() {
            bail!("chat attachment {attachment_id} has an empty MIME type");
        }
        return Ok(Some(RelayChatAttachmentRecord {
            name,
            mime_type,
            reference,
        }));
    }
    Ok(None)
}

fn chat_message_from_finitechat_payload(
    seq: u64,
    conversation_id: &str,
    payload: FiniteChatMessagePayload,
) -> ChatMessage {
    assert!(seq > 0);
    assert!(!conversation_id.is_empty());
    let created_at = payload
        .created_at
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());
    let updated_at = payload
        .updated_at
        .clone()
        .unwrap_or_else(|| created_at.clone());
    let message = ChatMessage {
        id: payload
            .message_id
            .unwrap_or_else(|| format!("finitechat_seq_{seq}")),
        thread_id: conversation_id.to_string(),
        sender_type: payload.sender_type.unwrap_or_else(|| "agent".to_string()),
        kind: payload.kind.unwrap_or_else(|| "message".to_string()),
        status: payload.status.unwrap_or_else(|| "complete".to_string()),
        body: payload.body,
        metadata: payload.metadata,
        created_at,
        updated_at,
    };
    assert!(!message.id.is_empty());
    assert!(!message.thread_id.is_empty());
    message
}

fn durable_chat_event_kind_name(kind: &DurableAppEventKind) -> String {
    match kind {
        DurableAppEventKind::ConversationCreate => "conversation.create".to_string(),
        DurableAppEventKind::ConversationUpdate => "conversation.update".to_string(),
        DurableAppEventKind::ConversationArchive => "conversation.archive".to_string(),
        DurableAppEventKind::ConversationSegmentStart => "conversation.segment.start".to_string(),
        DurableAppEventKind::ChatMessage => "chat.message".to_string(),
        DurableAppEventKind::ChatEdit => "chat.edit".to_string(),
        DurableAppEventKind::ChatReaction => "chat.reaction".to_string(),
        DurableAppEventKind::ChatReceipt => "chat.receipt".to_string(),
        DurableAppEventKind::RuntimeStateSnapshot => "runtime.state.snapshot".to_string(),
        DurableAppEventKind::RuntimeCommandRequest => "runtime.command.request".to_string(),
        DurableAppEventKind::RuntimeCommandResult => "runtime.command.result".to_string(),
        DurableAppEventKind::RuntimeCommandCancel => "runtime.command.cancel".to_string(),
        DurableAppEventKind::Namespaced { name, .. } => name.clone(),
    }
}

fn chat_thread_from_finitechat_payload(
    machine_id: &str,
    app_event: &DecryptedApplicationEventV1,
    payload: FiniteChatEventPayload,
) -> Result<Option<ChatThread>> {
    assert!(!machine_id.is_empty());
    let conversation = match (app_event.kind.clone(), payload) {
        (
            DurableAppEventKind::ConversationCreate,
            FiniteChatEventPayload::ConversationCreate(payload),
        )
        | (
            DurableAppEventKind::ConversationUpdate,
            FiniteChatEventPayload::ConversationUpdate(payload),
        )
        | (
            DurableAppEventKind::ConversationArchive,
            FiniteChatEventPayload::ConversationArchive(payload),
        ) => payload,
        _ => bail!("conversation payload kind mismatch"),
    };
    let Some(conversation_id) = app_event.conversation_id.as_deref() else {
        bail!("conversation event missing conversation id");
    };
    if conversation.conversation_id != conversation_id {
        bail!("conversation payload id does not match app event conversation id");
    }
    if conversation.machine_id != machine_id {
        return Ok(None);
    }
    let thread = ChatThread {
        id: conversation.conversation_id,
        project_agent_id: conversation.project_agent_id,
        created_by: conversation.created_by,
        title: conversation.title,
        created_at: conversation.created_at,
        last_activity_at: conversation.last_activity_at,
        message_count: conversation.message_count,
    };
    assert!(!thread.id.is_empty());
    assert!(!thread.project_agent_id.is_empty());
    Ok(Some(thread))
}

fn parse_chat_message_before_cursor(
    before: Option<&str>,
) -> Result<Option<ChatMessageBeforeCursor>> {
    let Some(before) = before.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if before.len() > MAX_CHAT_MESSAGE_BEFORE_CURSOR_BYTES {
        bail!(
            "chat message before cursor has {} bytes, max is {}",
            before.len(),
            MAX_CHAT_MESSAGE_BEFORE_CURSOR_BYTES
        );
    }
    if before.starts_with('{') {
        let cursor: ChatMessageBeforeCursor = serde_json::from_str(before)?;
        if cursor.created_at.trim().is_empty() || cursor.id.trim().is_empty() {
            bail!("chat message before cursor must include created_at and id");
        }
        return Ok(Some(cursor));
    }
    Ok(Some(ChatMessageBeforeCursor {
        created_at: before.to_string(),
        id: String::new(),
    }))
}

fn chat_message_is_before_cursor(
    message: &ChatMessage,
    cursor: Option<&ChatMessageBeforeCursor>,
) -> bool {
    let Some(cursor) = cursor else {
        return true;
    };
    message.created_at.as_str() < cursor.created_at.as_str()
        || (!cursor.id.is_empty()
            && message.created_at == cursor.created_at
            && message.id.as_str() < cursor.id.as_str())
}

fn chat_message_before_cursor(message: &ChatMessage) -> String {
    assert!(!message.created_at.is_empty());
    assert!(!message.id.is_empty());
    serde_json::to_string(&ChatMessageBeforeCursor {
        created_at: message.created_at.clone(),
        id: message.id.clone(),
    })
    .expect("chat message before cursor is serializable")
}

fn relay_command_name(event: &RelayEvent) -> String {
    assert!(!event.lane.is_empty());
    assert!(!event.kind.is_empty());
    if event.kind.starts_with(&format!("{}.", event.lane)) {
        event.kind.clone()
    } else {
        format!("{}.{}", event.lane, event.kind)
    }
}

fn relay_normalized_kind(lane: &str, kind: &str) -> String {
    assert!(!lane.is_empty());
    assert!(!kind.is_empty());
    if kind.starts_with(&format!("{lane}.")) {
        kind.to_string()
    } else {
        format!("{lane}.{kind}")
    }
}

fn relay_command_is_removed_chat_rpc(kind: &str) -> bool {
    assert!(!kind.is_empty());
    matches!(kind, "chat.create_thread" | "chat.send_message")
}

fn relay_command_envelope_from_event(event: &RelayEvent) -> Result<RelayCommandEnvelope> {
    assert!(!event.id.is_empty());
    assert!(!event.machine_id.is_empty());
    if let Some(envelope) = &event.command_envelope {
        envelope.validate_for_machine(&event.machine_id)?;
        return Ok(envelope.clone());
    }
    let command = relay_command_name(event);
    let Some(scope) = &event.scope else {
        bail!("relay event {} is missing explicit command scope", event.id);
    };
    let envelope = RelayCommandEnvelope {
        schema: COMMAND_ENVELOPE_SCHEMA.to_string(),
        command,
        scope: scope.clone(),
        bridge: event.bridge.clone(),
        payload: event.payload.clone(),
        created_at: event.created_at.clone(),
        expires_at: event.expires_at.clone(),
    };
    envelope.validate_for_machine(&event.machine_id)?;
    Ok(envelope)
}

fn relay_event_from_command_request(
    machine_id: &str,
    request: &FiniteChatCommandRequestPayload,
) -> Result<Option<RelayEvent>> {
    assert!(!machine_id.is_empty());
    assert!(!request.request_id.is_empty());
    assert!(!request.command.is_empty());
    relay_event_from_envelope_command_request(machine_id, request)
}

fn relay_event_from_envelope_command_request(
    machine_id: &str,
    request: &FiniteChatCommandRequestPayload,
) -> Result<Option<RelayEvent>> {
    assert!(!machine_id.is_empty());
    assert!(!request.request_id.is_empty());
    let Some(args) = request.args.as_object() else {
        return Ok(None);
    };
    let Some(schema) = args.get("schema").and_then(Value::as_str) else {
        return Ok(None);
    };
    if schema != COMMAND_ENVELOPE_SCHEMA {
        bail!(
            "runtime command envelope schema is {}, expected {}",
            schema,
            COMMAND_ENVELOPE_SCHEMA
        );
    }
    let envelope: RelayCommandEnvelope = serde_json::from_value(request.args.clone())
        .context("runtime command envelope is invalid")?;
    envelope.validate_for_machine(machine_id)?;
    if envelope.command != request.command {
        bail!("runtime command request command does not match envelope command");
    }
    let (lane, kind) = relay_lane_kind_from_command(&envelope.command)?;
    let bridge = envelope.bridge.clone();
    let scope = envelope.scope.clone();
    let payload = envelope.payload.clone();
    let created_at = envelope.created_at.clone();
    let expires_at = envelope.expires_at.clone();
    Ok(Some(RelayEvent {
        id: safe_segment(&request.request_id, "request id")?,
        machine_id: safe_segment(machine_id, "machine id")?,
        lane,
        kind,
        bridge,
        command_envelope: Some(envelope),
        scope: Some(scope),
        payload,
        created_at,
        expires_at,
    }))
}

fn relay_result_from_command_result(
    machine_id: &str,
    result: &FiniteChatCommandResultPayload,
) -> Result<RelayResult> {
    assert!(!machine_id.is_empty());
    assert!(!result.request_id.is_empty());
    let (ok, output, error) = match result.status {
        FiniteChatCommandStatus::Succeeded => (true, result.result.clone(), None),
        FiniteChatCommandStatus::Failed => (
            false,
            Value::Null,
            Some(relay_error_from_command_result(
                &result.result,
                "relay command failed",
            )),
        ),
        FiniteChatCommandStatus::Cancelled => (
            false,
            Value::Null,
            Some(relay_error_from_command_result(
                &result.result,
                "relay command cancelled",
            )),
        ),
    };
    Ok(RelayResult {
        event_id: safe_segment(&result.request_id, "request id")?,
        machine_id: safe_segment(machine_id, "machine id")?,
        ok,
        output,
        error,
        created_at: now_iso()?,
    })
}

fn relay_command_lease_from_snapshot(
    snapshot: RuntimeStateSnapshotV1,
) -> Result<RelayCommandLease> {
    let lease: RelayCommandLease = serde_json::from_slice(&snapshot.status_payload)
        .context("runtime command lease payload is invalid JSON")?;
    lease.validate()?;
    if lease.claimed_at_ms != snapshot.observed_at_ms {
        bail!("runtime command lease claimed_at_ms does not match observed_at_ms");
    }
    if lease.expires_at_ms != snapshot.expires_at_ms {
        bail!("runtime command lease expires_at_ms does not match snapshot expires_at_ms");
    }
    Ok(lease)
}

fn command_lease_expires_at_ms(event: &RelayEvent, claimed_at_ms: u64) -> Result<u64> {
    assert!(!event.id.is_empty());
    let event_expires_at_ms = event_expires_at_ms(event)?;
    let max_expires_at_ms = claimed_at_ms.saturating_add(MAX_RUNTIME_STATE_SNAPSHOT_EXPIRY_MILLIS);
    Ok(event_expires_at_ms.min(max_expires_at_ms))
}

fn event_expires_at_ms(event: &RelayEvent) -> Result<u64> {
    assert!(!event.id.is_empty());
    iso_to_millis(&event.expires_at)
}

fn iso_to_millis(value: &str) -> Result<u64> {
    let parsed = OffsetDateTime::parse(value, &Rfc3339)
        .with_context(|| format!("failed to parse timestamp {value:?}"))?;
    let millis = parsed.unix_timestamp_nanos() / 1_000_000;
    if millis < 0 {
        return Ok(0);
    }
    Ok((millis as u128).min(u128::from(u64::MAX)) as u64)
}

fn relay_error_from_command_result(result: &Value, fallback: &str) -> String {
    assert!(!fallback.is_empty());
    result
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| result.as_str())
        .unwrap_or(fallback)
        .to_string()
}

fn is_finitechat_idempotency_conflict(error: &FiniteChatBridgeError) -> bool {
    assert!(!error.to_string().is_empty());
    matches!(
        error,
        FiniteChatBridgeError::Store(StoreError::Engine(EngineError::ConflictingIdempotencyKey))
            | FiniteChatBridgeError::Engine(EngineError::ConflictingIdempotencyKey)
    )
}

fn relay_lane_kind_from_command(command: &str) -> Result<(String, String)> {
    let kind = safe_kind(command, "command")?;
    let lane = command
        .split_once('.')
        .map(|(lane, _)| lane)
        .unwrap_or(command);
    Ok((safe_kind(lane, "lane")?, kind))
}

fn safe_segment(value: &str, label: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{label} is required");
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    {
        bail!("invalid {label}");
    }
    Ok(trimmed.to_string())
}

fn validate_optional_scope_id(value: &Option<String>, label: &str) -> Result<()> {
    if let Some(value) = value {
        let safe = safe_segment(value, label)?;
        if safe != *value {
            bail!("{label} must be trimmed");
        }
        if safe.len() > MAX_OBJECT_ID_BYTES as usize {
            bail!("{label} exceeds {} bytes", MAX_OBJECT_ID_BYTES);
        }
    }
    Ok(())
}

fn safe_blob_sha256(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.len() != 64 {
        bail!("chat blob sha256 has {} chars, expected 64", trimmed.len());
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("chat blob sha256 must be lowercase hex");
    }
    Ok(trimmed.to_string())
}

fn safe_kind(value: &str, label: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{label} is required");
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        bail!("invalid {label}");
    }
    Ok(trimmed.to_string())
}

fn validate_status_snapshot_payload(value: &Value) -> Result<()> {
    let len = serde_json::to_vec(value)?.len();
    if len > MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES as usize {
        bail!(
            "status snapshot payload exceeds {} bytes",
            MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES
        );
    }
    Ok(())
}

fn validate_status_snapshot_label(label: &str, value: &str) -> Result<()> {
    if value.len() > MAX_OBJECT_ID_BYTES as usize {
        bail!("{label} exceeds {} bytes", MAX_OBJECT_ID_BYTES);
    }
    Ok(())
}

fn validate_status_snapshot_error(value: &str) -> Result<()> {
    if value.len() > MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES as usize {
        bail!(
            "status snapshot error exceeds {} bytes",
            MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES
        );
    }
    Ok(())
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let tmp_path = path.with_extension(format!("json.{}.tmp", relay_event_id()));
    fs::write(
        &tmp_path,
        format!("{}\n", serde_json::to_string_pretty(value)?),
    )
    .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            path.display()
        )
    })
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        bail!("cannot write an empty blob");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let tmp_path = path.with_extension(format!("blob.{}.tmp", relay_event_id()));
    fs::write(&tmp_path, bytes)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            path.display()
        )
    })
}

fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    serde_json::from_str(&fs::read_to_string(path)?)
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn is_expired(expires_at: &str) -> bool {
    OffsetDateTime::parse(expires_at, &Rfc3339)
        .map(|value| value < OffsetDateTime::now_utc())
        .unwrap_or(true)
}

fn now_iso() -> Result<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use finitechat_blob::{BlobDescriptor, finish_attachment_upload, prepare_attachment_upload};
    use finitechat_proto::{
        AttachmentBlobMetadataV1, DecryptedApplicationEventV1, DurableAppEventKind, LogEntryKind,
    };
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn prepare_relay_chat_attachment_upload_returns_blob_and_protocol_metadata() {
        let input = ChatMessageAttachmentInput {
            name: "diagram.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: BASE64_STANDARD.encode(b"finite attachment bytes"),
        };
        let prepared = prepare_relay_chat_attachment_upload("msg-attachment", 0, &input).unwrap();
        let attachment_id = prepared.metadata.get("id").and_then(Value::as_str).unwrap();

        assert!(!attachment_id.is_empty());
        assert_eq!(prepared.metadata["name"], "diagram.png");
        assert_eq!(prepared.metadata["mime_type"], "image/png");
        assert_eq!(prepared.blob.sha256, sha256_hex(&prepared.blob.ciphertext));
        assert!(
            prepared
                .metadata
                .get(CHAT_ATTACHMENT_BLOB_METADATA_KEY)
                .is_some()
        );
    }

    #[test]
    fn relay_store_round_trips_command_events_through_finitechat() {
        let root = tempdir().unwrap();
        let db_path = root.path().join(FINITE_CHAT_COMMAND_LEDGER_DB_FILE);
        let store = RelayStore::new(root.path());

        let heartbeat = store.heartbeat("machine-1").unwrap();
        assert_eq!(heartbeat.machine_id, "machine-1");
        assert_eq!(store.read_heartbeat("machine-1").unwrap(), Some(heartbeat));
        assert!(store.read_heartbeat("machine-2").unwrap().is_none());

        let snapshot = store
            .store_chat_snapshot(
                "machine-1",
                &StoreRelayChatSnapshotInput {
                    snapshot: json!({
                        "project_agents": [{ "id": "agent_machine-1", "name": "Agent" }],
                        "threads": [],
                        "messages": []
                    }),
                },
            )
            .unwrap();
        assert_eq!(snapshot.machine_id, "machine-1");
        let stored_snapshot = store
            .read_chat_snapshot("machine-1")
            .unwrap()
            .expect("chat bootstrap metadata");
        assert_eq!(stored_snapshot.machine_id, "machine-1");
        assert_eq!(
            stored_snapshot.snapshot["project_agents"][0]["id"].as_str(),
            Some("agent_machine-1")
        );
        assert!(stored_snapshot.snapshot.get("messages").is_none());

        let status = store
            .store_status_snapshot(
                "machine-1",
                &StoreRelayStatusSnapshotInput {
                    state_key: "runtime.inference.status".to_string(),
                    schema: "finitecomputer.runtime.inference.status.v1".to_string(),
                    revision: Some(7),
                    ttl_secs: Some(60),
                    ok: true,
                    status: json!({ "configured": true, "activeProfile": "finite-private" }),
                    error: None,
                },
            )
            .unwrap();
        assert_eq!(status.machine_id, "machine-1");
        assert_eq!(status.state_key, "runtime.inference.status");
        assert_eq!(status.revision, 7);
        assert_eq!(
            store
                .read_status_snapshot("machine-1", "runtime.inference.status")
                .unwrap(),
            Some(status)
        );

        let event = store
            .create_event(
                "machine-1",
                &CreateRelayEventInput {
                    lane: "runtime".to_string(),
                    kind: "runtime.gateway.restart".to_string(),
                    bridge: None,
                    scope: Some(runtime_scope("machine-1")),
                    payload: json!({ "reason": "round-trip-test" }),
                    ttl_secs: Some(60),
                },
            )
            .unwrap();
        assert!(
            !root
                .path()
                .join("machines/machine-1/events")
                .join(format!("{}.json", event.id))
                .exists()
        );

        let events = store.claim_events("machine-1", None, Some(10)).unwrap();
        assert_eq!(events.events, vec![event.clone()]);
        assert!(!root.path().join("machines/machine-1/claimed").exists());

        let duplicate_claim = store.claim_events("machine-1", None, Some(10)).unwrap();
        assert!(duplicate_claim.events.is_empty());

        store
            .store_result(
                "machine-1",
                &StoreRelayResultInput {
                    event_id: event.id.clone(),
                    ok: true,
                    output: json!({ "status": 200 }),
                    error: None,
                },
            )
            .unwrap();
        store
            .store_result(
                "machine-1",
                &StoreRelayResultInput {
                    event_id: event.id.clone(),
                    ok: false,
                    output: Value::Null,
                    error: Some("duplicate worker should not overwrite".to_string()),
                },
            )
            .unwrap();

        let result = store.wait_result("machine-1", &event.id).unwrap().unwrap();
        assert!(result.ok);
        assert_eq!(result.output, json!({ "status": 200 }));
        assert!(
            !root
                .path()
                .join("machines/machine-1/results")
                .join(format!("{}.json", event.id))
                .exists()
        );

        let repeated = store.wait_result("machine-1", &event.id).unwrap().unwrap();
        assert!(repeated.ok);
        assert_eq!(repeated.output, json!({ "status": 200 }));

        store.ack_event("machine-1", &event.id).unwrap();
        let events = store.claim_events("machine-1", None, Some(10)).unwrap();
        assert!(events.events.is_empty());

        let payloads = finitechat_command_payloads(&db_path, "machine-1").unwrap();
        assert_eq!(payloads.len(), 2);
        let FiniteChatEventPayload::RuntimeCommandRequest(first_request) = &payloads[0] else {
            panic!("first command event should be a request");
        };
        assert_eq!(first_request.request_id, event.id);
        assert_eq!(first_request.command, "runtime.gateway.restart");
        let first_envelope: RelayCommandEnvelope =
            serde_json::from_value(first_request.args.clone()).unwrap();
        assert_eq!(first_envelope.schema, COMMAND_ENVELOPE_SCHEMA);
        assert_eq!(first_envelope.command, "runtime.gateway.restart");
        assert_eq!(first_envelope.scope.machine_id, "machine-1");
        assert_eq!(
            first_envelope.scope.runtime_id.as_deref(),
            Some("runtime:machine-1")
        );
        assert_eq!(
            first_envelope.payload.get("reason").and_then(Value::as_str),
            Some("round-trip-test")
        );

        let FiniteChatEventPayload::RuntimeCommandResult(first_result) = &payloads[1] else {
            panic!("second command event should be a result");
        };
        assert_eq!(first_result.request_id, event.id);
        assert_eq!(first_result.status, FiniteChatCommandStatus::Succeeded);
        assert_eq!(first_result.result, json!({ "status": 200 }));
    }

    #[test]
    fn relay_chat_log_pages_messages_from_finitechat_ledger() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        let other_bridge = relay_bridge_device("hosted-web-user-alice");
        assert_ne!(
            store.hosted_bridge_state_dir("machine-1", &bridge).unwrap(),
            store
                .hosted_bridge_state_dir("machine-1", &other_bridge)
                .unwrap()
        );
        let threads = vec![chat_thread(
            "thread-1",
            "agent_machine-1",
            "2026-05-22T00:03:00Z",
            3,
        )];
        let messages = vec![
            chat_message("msg-1", "thread-1", "2026-05-22T00:02:00Z", "one"),
            chat_message("msg-2", "thread-1", "2026-05-22T00:02:00Z", "two"),
            chat_message("msg-3", "thread-1", "2026-05-22T00:03:00Z", "three"),
            ChatMessage {
                id: "msg-status-running".to_string(),
                thread_id: "thread-1".to_string(),
                sender_type: "agent".to_string(),
                kind: "status".to_string(),
                status: "running".to_string(),
                body: "thinking".to_string(),
                metadata: json!({}),
                created_at: "2026-05-22T00:04:00Z".to_string(),
                updated_at: "2026-05-22T00:04:00Z".to_string(),
            },
        ];

        let ack = store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: threads.clone(),
                    messages: messages.clone(),
                },
            )
            .unwrap();
        assert_eq!(ack.stored, 4);
        assert_eq!(ack.skipped, 1);
        let chat_db = rusqlite::Connection::open(&store.finitechat_chat_ledger.db_path).unwrap();
        let legacy_keys = chat_db
            .execute(
                r#"
                UPDATE room_log_entries
                SET idempotency_key = 'legacy-relay-chat-msg-' || seq
                WHERE idempotency_key LIKE 'relay-chat-msg-%'
                "#,
                [],
            )
            .unwrap();
        assert_eq!(legacy_keys, 3);
        let legacy_duplicate = store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![],
                    messages: vec![messages[0].clone()],
                },
            )
            .unwrap();
        assert_eq!(legacy_duplicate.stored, 0);
        assert_eq!(legacy_duplicate.skipped, 1);

        let duplicate = store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads,
                    messages,
                },
            )
            .unwrap();
        assert_eq!(duplicate.stored, 0);
        assert_eq!(duplicate.skipped, 5);

        let mut updated_msg_2 =
            chat_message("msg-2", "thread-1", "2026-05-22T00:02:00Z", "two updated");
        updated_msg_2.updated_at = "2026-05-22T00:05:00Z".to_string();
        let updated = store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![],
                    messages: vec![updated_msg_2.clone()],
                },
            )
            .unwrap();
        assert_eq!(updated.stored, 1);
        assert_eq!(updated.skipped, 0);
        let duplicate_update = store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![],
                    messages: vec![updated_msg_2],
                },
            )
            .unwrap();
        assert_eq!(duplicate_update.stored, 0);
        assert_eq!(duplicate_update.skipped, 1);

        let threads = store.chat_threads("machine-1", &bridge).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "thread-1");
        assert_eq!(threads[0].message_count, 3);

        let latest = store
            .chat_message_page_for_machine("machine-1", "thread-1", &bridge, Some(2), None)
            .unwrap();
        assert!(latest.has_more);
        assert_eq!(
            latest
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["msg-2", "msg-3"]
        );
        assert_eq!(latest.messages[0].body, "two updated");
        assert_eq!(
            latest.next_before.as_deref(),
            Some(r#"{"created_at":"2026-05-22T00:02:00Z","id":"msg-2"}"#)
        );

        let older = store
            .chat_message_page(
                "machine-1",
                "agent_machine-1",
                "thread-1",
                &bridge,
                Some(2),
                latest.next_before.as_deref(),
            )
            .unwrap();
        assert!(!older.has_more);
        assert_eq!(older.messages.len(), 1);
        assert_eq!(older.messages[0].id, "msg-1");
    }

    #[test]
    fn relay_chat_attachment_reads_from_uploaded_blob_after_restart() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        let plaintext = b"render this attachment";
        let prepared = prepare_attachment_upload(
            plaintext,
            AttachmentBlobMetadataV1 {
                mime_type: "image/png".to_string(),
                filename: "render.png".to_string(),
                dimensions: None,
            },
        )
        .expect("prepare attachment");
        let reference = finish_attachment_upload(
            &prepared,
            BlobDescriptor {
                url: format!(
                    "finitechat+relay-blob://sha256/{}",
                    prepared.ciphertext_sha256
                ),
                sha256: prepared.ciphertext_sha256.clone(),
                size_bytes: prepared.ciphertext_size,
            },
        )
        .expect("finish attachment");

        let blob_ack = store
            .store_chat_blob(
                "machine-1",
                &reference.ciphertext_sha256,
                &prepared.ciphertext,
            )
            .expect("store blob");
        assert_eq!(blob_ack.size_bytes, prepared.ciphertext.len() as u64);
        let thread = chat_thread("thread-blob", "agent_machine-1", "2026-05-22T00:03:00Z", 1);
        let mut legacy_message =
            chat_message("msg-blob", "thread-blob", "2026-05-22T00:03:00Z", "");
        legacy_message.metadata = json!({
            "attachments": [{
                "id": "att_blob",
                "type": "image",
                "name": "render.png",
                "mime_type": "image/png",
                "size_bytes": plaintext.len(),
                "url": "/attachments/att_blob"
            }]
        });
        let legacy_ack = store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![thread],
                    messages: vec![legacy_message],
                },
            )
            .expect("store legacy chat log");
        assert_eq!(legacy_ack.stored, 2);

        let mut message = chat_message("msg-blob", "thread-blob", "2026-05-22T00:03:00Z", "");
        message.updated_at = "2026-05-22T00:04:00Z".to_string();
        let mut attachment = json!({
            "id": "att_blob",
            "type": "image",
            "name": "render.png",
            "mime_type": "image/png",
            "size_bytes": plaintext.len(),
            "url": "/attachments/att_blob"
        });
        attachment
            .as_object_mut()
            .expect("attachment object")
            .insert(
                CHAT_ATTACHMENT_BLOB_METADATA_KEY.to_string(),
                serde_json::to_value(reference).expect("reference json"),
            );
        message.metadata = json!({ "attachments": [attachment] });
        let ack = store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![],
                    messages: vec![message],
                },
            )
            .expect("store chat log");
        assert_eq!(ack.stored, 1);

        let reopened = RelayStore::new(root.path());
        let attachment = reopened
            .read_chat_attachment("machine-1", "att_blob", &bridge)
            .expect("read attachment")
            .expect("attachment exists");
        assert_eq!(attachment.name, "render.png");
        assert_eq!(attachment.mime_type, "image/png");
        assert_eq!(attachment.bytes, plaintext);
    }

    #[test]
    fn relay_chat_attachment_rejects_wrong_blob_hash() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let err = store
            .store_chat_blob("machine-1", &"0".repeat(64), b"wrong bytes")
            .expect_err("sha mismatch should fail")
            .to_string();
        assert!(err.contains("sha256 mismatch"));
    }

    #[test]
    fn relay_chat_stream_snapshot_materializes_from_finitechat_ledger() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        store
            .store_chat_snapshot(
                "machine-1",
                &StoreRelayChatSnapshotInput {
                    snapshot: json!({
                        "project_agents": [{ "id": "agent_machine-1", "name": "Agent" }],
                        "threads": [chat_thread("legacy-thread", "agent_machine-1", "2026-05-22T00:00:00Z", 1)],
                        "messages": [chat_message("legacy-msg", "legacy-thread", "2026-05-22T00:00:00Z", "legacy")]
                    }),
                },
            )
            .unwrap();
        assert!(
            !root
                .path()
                .join("machines/machine-1/chat/snapshot.json")
                .exists()
        );
        let bootstrap = store
            .read_chat_snapshot("machine-1")
            .unwrap()
            .expect("bootstrap from finitechat state");
        assert_eq!(
            bootstrap.snapshot["project_agents"][0]["id"].as_str(),
            Some("agent_machine-1")
        );
        assert!(bootstrap.snapshot.get("messages").is_none());

        store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![chat_thread(
                        "thread-1",
                        "agent_machine-1",
                        "2026-05-22T00:03:00Z",
                        2,
                    )],
                    messages: vec![
                        chat_message("msg-1", "thread-1", "2026-05-22T00:02:00Z", "one"),
                        chat_message("msg-2", "thread-1", "2026-05-22T00:03:00Z", "two"),
                    ],
                },
            )
            .unwrap();

        let snapshot = store
            .chat_stream_snapshot("machine-1", &bridge)
            .unwrap()
            .expect("stream snapshot");
        assert_eq!(
            snapshot.snapshot["project_agents"][0]["id"].as_str(),
            Some("agent_machine-1")
        );
        let threads: Vec<ChatThread> =
            serde_json::from_value(snapshot.snapshot["threads"].clone()).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "thread-1");
        let messages: Vec<ChatMessage> =
            serde_json::from_value(snapshot.snapshot["messages"].clone()).unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["msg-1", "msg-2"]
        );
        assert!(!messages.iter().any(|message| message.id == "legacy-msg"));
    }

    #[test]
    fn relay_chat_stream_requires_bootstrap_metadata_when_log_exists() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![chat_thread(
                        "thread-1",
                        "agent_machine-1",
                        "2026-05-22T00:03:00Z",
                        1,
                    )],
                    messages: vec![chat_message(
                        "msg-1",
                        "thread-1",
                        "2026-05-22T00:03:00Z",
                        "one",
                    )],
                },
            )
            .unwrap();

        assert!(store.read_chat_snapshot("machine-1").unwrap().is_none());
        let err = store
            .chat_stream_snapshot("machine-1", &bridge)
            .expect_err("stream must require bootstrap metadata")
            .to_string();
        assert!(err.contains("chat bootstrap metadata is unavailable"));
    }

    #[test]
    fn relay_chat_bootstrap_metadata_rejects_invalid_shape() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let non_array = store
            .store_chat_snapshot(
                "machine-1",
                &StoreRelayChatSnapshotInput {
                    snapshot: json!({ "project_agents": "agent_machine-1" }),
                },
            )
            .expect_err("project_agents must be an array")
            .to_string();
        assert!(non_array.contains("project_agents must be an array"));

        let missing_id = store
            .store_chat_snapshot(
                "machine-1",
                &StoreRelayChatSnapshotInput {
                    snapshot: json!({ "project_agents": [{ "name": "Agent" }] }),
                },
            )
            .expect_err("project agent id is required")
            .to_string();
        assert!(missing_id.contains("project agent is missing id"));
    }

    #[test]
    fn relay_chat_log_accepts_agent_message_updates() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        let thread = chat_thread("thread-1", "agent_machine-1", "2026-05-22T00:02:00Z", 1);
        let mut draft = chat_message("msg-agent-1", "thread-1", "2026-05-22T00:02:00Z", "draft");
        draft.sender_type = "agent".to_string();
        draft.status = "running".to_string();
        let mut complete = draft.clone();
        complete.body = "final answer".to_string();
        complete.status = "complete".to_string();
        complete.updated_at = "2026-05-22T00:03:00Z".to_string();

        let first = store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![thread],
                    messages: vec![draft],
                },
            )
            .unwrap();
        assert_eq!(first.stored, 2);
        assert_eq!(first.skipped, 0);
        let updated = store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![],
                    messages: vec![complete.clone()],
                },
            )
            .unwrap();
        assert_eq!(updated.stored, 1);
        assert_eq!(updated.skipped, 0);

        let page = store
            .chat_message_page_for_machine("machine-1", "thread-1", &bridge, Some(10), None)
            .unwrap();
        assert_eq!(page.messages, vec![complete]);
    }

    #[test]
    fn relay_chat_stream_event_resumes_from_cursor() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        store
            .store_chat_snapshot(
                "machine-1",
                &StoreRelayChatSnapshotInput {
                    snapshot: json!({
                        "project_agents": [{ "id": "agent_machine-1", "name": "Agent" }],
                        "users": [],
                        "machines": [],
                        "sites": [],
                        "skills": [],
                        "capabilities": []
                    }),
                },
            )
            .unwrap();
        store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![chat_thread(
                        "thread-1",
                        "agent_machine-1",
                        "2026-05-22T00:01:00Z",
                        1,
                    )],
                    messages: vec![chat_message(
                        "msg-1",
                        "thread-1",
                        "2026-05-22T00:01:00Z",
                        "one",
                    )],
                },
            )
            .unwrap();

        let initial = store
            .chat_stream_event("machine-1", &bridge, None)
            .unwrap()
            .expect("initial stream event");
        assert!(initial.reset);
        assert!(initial.cursor.starts_with(CHAT_STREAM_CURSOR_PREFIX));
        let messages: Vec<ChatMessage> =
            serde_json::from_value(initial.snapshot["messages"].clone()).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, "msg-1");
        assert!(
            store
                .chat_stream_event("machine-1", &bridge, Some(&initial.cursor))
                .unwrap()
                .is_none()
        );

        store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent_machine-1".to_string(),
                    threads: vec![chat_thread(
                        "thread-1",
                        "agent_machine-1",
                        "2026-05-22T00:02:00Z",
                        2,
                    )],
                    messages: vec![chat_message(
                        "msg-2",
                        "thread-1",
                        "2026-05-22T00:02:00Z",
                        "two",
                    )],
                },
            )
            .unwrap();
        let update = store
            .chat_stream_event("machine-1", &bridge, Some(&initial.cursor))
            .unwrap()
            .expect("delta stream event");
        assert!(!update.reset);
        assert_ne!(update.cursor, initial.cursor);
        let messages: Vec<ChatMessage> =
            serde_json::from_value(update.snapshot["messages"].clone()).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, "msg-2");
        assert!(
            store
                .chat_stream_event("machine-1", &bridge, Some(&update.cursor))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn relay_removed_chat_mutation_commands_are_rejected() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        let send = store
            .create_event(
                "machine-1",
                &CreateRelayEventInput {
                    lane: "chat".to_string(),
                    kind: "chat.send_message".to_string(),
                    bridge: Some(bridge.clone()),
                    scope: Some(RelayCommandScope {
                        machine_id: "machine-1".to_string(),
                        room_id: None,
                        conversation_id: Some("thread-1".to_string()),
                        topic_id: Some("topic-general".to_string()),
                        project_id: Some("project-alpha".to_string()),
                        project_agent_id: Some("agent-machine-1".to_string()),
                        runtime_id: Some("runtime:machine-1".to_string()),
                        target_device_id: None,
                        actor_device_id: Some(bridge.bridge_device_id.clone()),
                    }),
                    payload: json!({
                        "threadId": "thread-1",
                        "message": { "body": "hello", "attachments": [] }
                    }),
                    ttl_secs: Some(60),
                },
            )
            .expect_err("chat.send_message command should be rejected");
        assert!(
            send.to_string()
                .contains("moved to the finite chat room log")
        );

        let create = store
            .create_event(
                "machine-1",
                &CreateRelayEventInput {
                    lane: "chat".to_string(),
                    kind: "chat.create_thread".to_string(),
                    bridge: Some(bridge.clone()),
                    scope: Some(RelayCommandScope {
                        machine_id: "machine-1".to_string(),
                        room_id: None,
                        conversation_id: Some("thread-1".to_string()),
                        topic_id: Some("topic-general".to_string()),
                        project_id: Some("project-alpha".to_string()),
                        project_agent_id: Some("agent-machine-1".to_string()),
                        runtime_id: Some("runtime:machine-1".to_string()),
                        target_device_id: None,
                        actor_device_id: Some(bridge.bridge_device_id.clone()),
                    }),
                    payload: json!({
                        "threadId": "thread-1",
                        "topicId": "topic-general",
                        "projectId": "project-alpha",
                        "projectAgentId": "agent-machine-1",
                        "title": "Deploys"
                    }),
                    ttl_secs: Some(60),
                },
            )
            .expect_err("chat.create_thread command should be rejected");
        assert!(
            create
                .to_string()
                .contains("moved to the finite chat room log")
        );

        let claimed = store.claim_events("machine-1", None, Some(10)).unwrap();
        assert!(claimed.events.is_empty());
        assert!(!root.path().join("machines/machine-1/claimed").exists());
    }

    #[test]
    fn relay_create_chat_conversation_appends_finitechat_without_command_roundtrip() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        let input = CreateRelayChatConversationInput {
            bridge: bridge.clone(),
            project_agent_id: "agent-machine-1".to_string(),
            conversation_id: Some("topic-1".to_string()),
            title: Some("Deploys".to_string()),
        };

        let first = store.create_chat_conversation("machine-1", &input).unwrap();
        let second = store.create_chat_conversation("machine-1", &input).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.id, "topic-1");
        assert_eq!(first.title, "Deploys");
        assert_eq!(first.created_by, "hosted-web-user-paul");
        let threads = store.chat_threads("machine-1", &bridge).unwrap();
        assert_eq!(threads, vec![first]);

        let command_payloads =
            finitechat_command_payloads_or_empty(root.path(), "machine-1").unwrap();
        assert_eq!(command_payloads.len(), 0);

        let inbox = store
            .chat_inbox("machine-1", "agent-machine-1", None, Some(10))
            .unwrap();
        assert_eq!(inbox.events.len(), 1);
        assert_eq!(inbox.events[0].kind, "conversation.create");
        assert_eq!(inbox.events[0].conversation_id, "topic-1");
        assert_eq!(
            inbox.events[0].conversation.as_ref().unwrap().title,
            "Deploys"
        );
    }

    #[test]
    fn relay_update_chat_conversation_appends_non_notifying_title_update() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        let create = CreateRelayChatConversationInput {
            bridge: bridge.clone(),
            project_agent_id: "agent-machine-1".to_string(),
            conversation_id: Some("topic-1".to_string()),
            title: Some("New chat".to_string()),
        };
        store
            .create_chat_conversation("machine-1", &create)
            .unwrap();

        let update = UpdateRelayChatConversationInput {
            bridge: bridge.clone(),
            project_agent_id: "agent-machine-1".to_string(),
            title: " Deploy runbook ".to_string(),
        };
        let first = store
            .update_chat_conversation("machine-1", "topic-1", &update)
            .unwrap();
        let second = store
            .update_chat_conversation("machine-1", "topic-1", &update)
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(first.id, "topic-1");
        assert_eq!(first.title, "Deploy runbook");
        assert_eq!(
            store.chat_threads("machine-1", &bridge).unwrap()[0].title,
            "Deploy runbook"
        );

        let seed = finitechat_chat_room_seed("machine-1", "agent-machine-1");
        let chat_store =
            SqliteDeliveryStore::open(root.path().join(FINITE_CHAT_CHAT_LEDGER_DB_FILE)).unwrap();
        let room = chat_store.room(&seed.room_id).unwrap().unwrap();
        let updates = room
            .log
            .iter()
            .filter_map(|entry| {
                let event: DecryptedApplicationEventV1 =
                    serde_json::from_slice(&entry.envelope.payload).ok()?;
                if event.kind != DurableAppEventKind::ConversationUpdate {
                    return None;
                }
                let payload: FiniteChatEventPayload =
                    serde_json::from_slice(&event.payload).ok()?;
                Some((entry.message_id.clone(), payload))
            })
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 1);
        let FiniteChatEventPayload::ConversationUpdate(payload) = &updates[0].1 else {
            panic!("expected conversation update payload");
        };
        assert_eq!(payload.conversation_id, "topic-1");
        assert_eq!(payload.title, "Deploy runbook");
        let effect = chat_store
            .application_effect(&updates[0].0)
            .unwrap()
            .unwrap();
        assert!(!effect.creates_push());
        assert!(!effect.creates_unread());
    }

    #[test]
    fn runtime_sync_cannot_overwrite_user_renamed_conversation_title() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        store
            .create_chat_conversation(
                "machine-1",
                &CreateRelayChatConversationInput {
                    bridge: bridge.clone(),
                    project_agent_id: "agent-machine-1".to_string(),
                    conversation_id: Some("topic-1".to_string()),
                    title: Some("New chat".to_string()),
                },
            )
            .unwrap();
        store
            .update_chat_conversation(
                "machine-1",
                "topic-1",
                &UpdateRelayChatConversationInput {
                    bridge: bridge.clone(),
                    project_agent_id: "agent-machine-1".to_string(),
                    title: "User chosen title".to_string(),
                },
            )
            .unwrap();

        let mut runtime_thread =
            chat_thread("topic-1", "agent-machine-1", "2026-05-22T00:10:00Z", 3);
        runtime_thread.title = "Runtime stale auto title".to_string();
        store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent-machine-1".to_string(),
                    threads: vec![runtime_thread],
                    messages: vec![],
                },
            )
            .unwrap();

        let threads = store.chat_threads("machine-1", &bridge).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "topic-1");
        assert_eq!(threads[0].title, "User chosen title");
        assert_eq!(threads[0].message_count, 3);
        assert_eq!(threads[0].last_activity_at, "2026-05-22T00:10:00Z");
    }

    #[test]
    fn relay_update_chat_conversation_rejects_invalid_titles() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        store
            .create_chat_conversation(
                "machine-1",
                &CreateRelayChatConversationInput {
                    bridge: bridge.clone(),
                    project_agent_id: "agent-machine-1".to_string(),
                    conversation_id: Some("topic-1".to_string()),
                    title: Some("New chat".to_string()),
                },
            )
            .unwrap();

        let blank = store
            .update_chat_conversation(
                "machine-1",
                "topic-1",
                &UpdateRelayChatConversationInput {
                    bridge: bridge.clone(),
                    project_agent_id: "agent-machine-1".to_string(),
                    title: "   ".to_string(),
                },
            )
            .expect_err("blank title should be rejected");
        assert!(blank.to_string().contains("title is required"));

        let oversized = store
            .update_chat_conversation(
                "machine-1",
                "topic-1",
                &UpdateRelayChatConversationInput {
                    bridge,
                    project_agent_id: "agent-machine-1".to_string(),
                    title: "x".repeat(MAX_CHAT_THREAD_TITLE_CHARS + 1),
                },
            )
            .expect_err("oversized title should be rejected");
        assert!(oversized.to_string().contains("title has"));
    }

    #[test]
    fn relay_send_chat_message_appends_finitechat_without_command_roundtrip() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        let thread = chat_thread("thread-1", "agent-machine-1", "2026-05-22T00:03:00Z", 0);
        store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent-machine-1".to_string(),
                    threads: vec![thread],
                    messages: vec![],
                },
            )
            .unwrap();

        let message = store
            .send_chat_message(
                "machine-1",
                "thread-1",
                &SendRelayChatMessageInput {
                    bridge: bridge.clone(),
                    message: SendChatMessageRequest {
                        body: "hello finitechat first".to_string(),
                        client_message_id: Some("msg-finitechat-first".to_string()),
                        attachments: vec![],
                    },
                },
            )
            .unwrap();

        assert_eq!(message.id, "msg-finitechat-first");
        assert_eq!(message.thread_id, "thread-1");
        assert_eq!(message.sender_type, "user");
        let page = store
            .chat_message_page_for_machine("machine-1", "thread-1", &bridge, Some(10), None)
            .unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0], message);

        let command_payloads =
            finitechat_command_payloads_or_empty(root.path(), "machine-1").unwrap();
        assert_eq!(command_payloads.len(), 0);

        let inbox = store
            .chat_inbox("machine-1", "agent-machine-1", None, Some(10))
            .unwrap();
        assert_eq!(inbox.events.len(), 1);
        assert_eq!(inbox.events[0].kind, "chat.message");
        assert_eq!(inbox.events[0].conversation_id, "thread-1");
        assert_eq!(
            inbox.events[0].message.as_ref().unwrap().body,
            "hello finitechat first"
        );
    }

    #[test]
    fn relay_send_chat_message_retry_is_idempotent() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        let thread = chat_thread("thread-1", "agent-machine-1", "2026-05-22T00:03:00Z", 0);
        let input = SendRelayChatMessageInput {
            bridge: bridge.clone(),
            message: SendChatMessageRequest {
                body: "retry once".to_string(),
                client_message_id: Some("msg-retry".to_string()),
                attachments: vec![],
            },
        };
        store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent-machine-1".to_string(),
                    threads: vec![thread],
                    messages: vec![],
                },
            )
            .unwrap();

        let first = store
            .send_chat_message("machine-1", "thread-1", &input)
            .unwrap();
        let second = store
            .send_chat_message("machine-1", "thread-1", &input)
            .unwrap();

        assert_eq!(first, second);
        let page = store
            .chat_message_page_for_machine("machine-1", "thread-1", &bridge, Some(10), None)
            .unwrap();
        assert_eq!(page.messages.len(), 1);
        let command_payloads =
            finitechat_command_payloads_or_empty(root.path(), "machine-1").unwrap();
        assert_eq!(command_payloads.len(), 0);

        let mut runtime_synced_message = first.clone();
        runtime_synced_message.created_at = "2026-05-22T00:04:00Z".to_string();
        runtime_synced_message.updated_at = "2026-05-22T00:04:00Z".to_string();
        let ack = store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent-machine-1".to_string(),
                    threads: vec![],
                    messages: vec![runtime_synced_message],
                },
            )
            .unwrap();
        assert_eq!(ack.stored, 0);
        assert_eq!(ack.skipped, 1);
        let page = store
            .chat_message_page_for_machine("machine-1", "thread-1", &bridge, Some(10), None)
            .unwrap();
        assert_eq!(page.messages.len(), 1);
    }

    #[test]
    fn relay_chat_inbox_cursor_resumes_ordered_user_events() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        let conversation = store
            .create_chat_conversation(
                "machine-1",
                &CreateRelayChatConversationInput {
                    bridge: bridge.clone(),
                    project_agent_id: "agent-machine-1".to_string(),
                    conversation_id: Some("topic-1".to_string()),
                    title: Some("Deploys".to_string()),
                },
            )
            .unwrap();
        let message = store
            .send_chat_message(
                "machine-1",
                "topic-1",
                &SendRelayChatMessageInput {
                    bridge,
                    message: SendChatMessageRequest {
                        body: "ship it".to_string(),
                        client_message_id: Some("msg-ship-it".to_string()),
                        attachments: vec![],
                    },
                },
            )
            .unwrap();

        let first = store
            .chat_inbox("machine-1", "agent-machine-1", None, Some(1))
            .unwrap();
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.events[0].kind, "conversation.create");
        assert_eq!(first.events[0].conversation.as_ref(), Some(&conversation));

        let second = store
            .chat_inbox("machine-1", "agent-machine-1", Some(first.cursor), Some(10))
            .unwrap();
        assert_eq!(second.events.len(), 1);
        assert!(second.cursor > first.cursor);
        assert_eq!(second.events[0].kind, "chat.message");
        assert_eq!(second.events[0].message.as_ref(), Some(&message));

        let empty = store
            .chat_inbox(
                "machine-1",
                "agent-machine-1",
                Some(second.cursor),
                Some(10),
            )
            .unwrap();
        assert!(empty.events.is_empty());
        assert_eq!(empty.cursor, second.cursor);
    }

    #[test]
    fn relay_send_chat_message_stores_attachment_blob() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());
        let bridge = relay_bridge_device("hosted-web-user-paul");
        let thread = chat_thread("thread-1", "agent-machine-1", "2026-05-22T00:03:00Z", 0);
        store
            .store_chat_log(
                "machine-1",
                &StoreRelayChatLogInput {
                    project_agent_id: "agent-machine-1".to_string(),
                    threads: vec![thread],
                    messages: vec![],
                },
            )
            .unwrap();

        let message = store
            .send_chat_message(
                "machine-1",
                "thread-1",
                &SendRelayChatMessageInput {
                    bridge: bridge.clone(),
                    message: SendChatMessageRequest {
                        body: String::new(),
                        client_message_id: Some("msg-with-attachment".to_string()),
                        attachments: vec![ChatMessageAttachmentInput {
                            name: "note.txt".to_string(),
                            mime_type: "text/plain".to_string(),
                            data_base64: BASE64_STANDARD.encode(b"hello attachment"),
                        }],
                    },
                },
            )
            .unwrap();

        let attachment_id = message.metadata["attachments"][0]["id"]
            .as_str()
            .expect("attachment id");
        assert!(
            message.metadata["attachments"][0]
                .get(CHAT_ATTACHMENT_BLOB_METADATA_KEY)
                .is_some()
        );
        let attachment = store
            .read_chat_attachment("machine-1", attachment_id, &bridge)
            .unwrap()
            .expect("attachment");
        assert_eq!(attachment.name, "note.txt");
        assert_eq!(attachment.mime_type, "text/plain");
        assert_eq!(attachment.bytes, b"hello attachment");
    }

    #[test]
    fn relay_commands_ignore_legacy_bare_args_without_scope() {
        let request = FiniteChatCommandRequestPayload {
            request_id: "event-1".to_string(),
            command: "chat.send_message".to_string(),
            args: json!({
                "lane": "chat",
                "kind": "chat.send_message",
                "bridge": relay_bridge_device("hosted-web-user-paul"),
                "payload": {
                    "threadId": "thread-1",
                    "topicId": "topic-general",
                    "message": { "body": "hello", "attachments": [] }
                },
                "created_at": "2026-05-22T00:00:00Z",
                "expires_at": "2026-05-22T00:01:00Z"
            }),
        };

        let event = relay_event_from_command_request("machine-1", &request).unwrap();
        assert!(event.is_none());
    }

    #[test]
    fn relay_commands_claim_requests_from_finitechat_after_restart() {
        let root = tempdir().unwrap();
        let db_path = root.path().join(FINITE_CHAT_COMMAND_LEDGER_DB_FILE);
        let store = RelayStore::new(root.path());
        let event = store
            .create_event(
                "machine-1",
                &CreateRelayEventInput {
                    lane: "runtime".to_string(),
                    kind: "runtime.gateway.restart".to_string(),
                    bridge: None,
                    scope: Some(runtime_scope("machine-1")),
                    payload: json!({ "reason": "source-test" }),
                    ttl_secs: Some(60),
                },
            )
            .unwrap();
        assert!(
            !root
                .path()
                .join("machines/machine-1/events")
                .join(format!("{}.json", event.id))
                .exists()
        );

        let restarted = RelayStore::new(root.path());
        let claimed = restarted.claim_events("machine-1", None, Some(10)).unwrap();
        assert_eq!(claimed.events, vec![event.clone()]);
        assert!(!root.path().join("machines/machine-1/claimed").exists());
        assert!(
            restarted
                .claim_events("machine-1", None, Some(10))
                .unwrap()
                .events
                .is_empty()
        );
        let leases = finitechat_command_leases(&db_path, "machine-1").unwrap();
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].request_id, event.id);
        assert!(leases[0].expires_at_ms > leases[0].claimed_at_ms);

        restarted
            .store_result(
                "machine-1",
                &StoreRelayResultInput {
                    event_id: event.id.clone(),
                    ok: true,
                    output: json!({ "restarted": true }),
                    error: None,
                },
            )
            .unwrap();
        let result = restarted
            .wait_result("machine-1", &event.id)
            .unwrap()
            .unwrap();
        assert!(result.ok);
        assert_eq!(result.output, json!({ "restarted": true }));
        assert!(
            !root
                .path()
                .join("machines/machine-1/results")
                .join(format!("{}.json", event.id))
                .exists()
        );
        let repeat_result = restarted
            .wait_result("machine-1", &event.id)
            .unwrap()
            .unwrap();
        assert!(repeat_result.ok);
        assert_eq!(repeat_result.output, json!({ "restarted": true }));

        restarted
            .store_result(
                "machine-1",
                &StoreRelayResultInput {
                    event_id: event.id.clone(),
                    ok: true,
                    output: json!({ "restarted": true }),
                    error: None,
                },
            )
            .unwrap();
        restarted.ack_event("machine-1", &event.id).unwrap();

        let replayed = RelayStore::new(root.path());
        assert!(
            replayed
                .claim_events("machine-1", None, Some(10))
                .unwrap()
                .events
                .is_empty()
        );

        let payloads = finitechat_command_payloads(&db_path, "machine-1").unwrap();
        assert_eq!(payloads.len(), 2);
        let FiniteChatEventPayload::RuntimeCommandRequest(request) = &payloads[0] else {
            panic!("first command event should be a request");
        };
        assert_eq!(request.request_id, event.id);
        assert_eq!(request.command, "runtime.gateway.restart");
        let envelope: RelayCommandEnvelope = serde_json::from_value(request.args.clone()).unwrap();
        assert_eq!(envelope.schema, COMMAND_ENVELOPE_SCHEMA);
        assert_eq!(envelope.scope.machine_id, "machine-1");
        assert_eq!(
            envelope.payload.get("reason").and_then(Value::as_str),
            Some("source-test")
        );
        let FiniteChatEventPayload::RuntimeCommandResult(result) = &payloads[1] else {
            panic!("second command event should be a result");
        };
        assert_eq!(result.request_id, event.id);
        assert_eq!(result.status, FiniteChatCommandStatus::Succeeded);
        assert_eq!(result.result, json!({ "restarted": true }));
    }

    #[test]
    fn relay_commands_ignore_expired_finitechat_leases() {
        let root = tempdir().unwrap();
        let db_path = root.path().join(FINITE_CHAT_COMMAND_LEDGER_DB_FILE);
        let store = RelayStore::new(root.path());
        let event = store
            .create_event(
                "machine-1",
                &CreateRelayEventInput {
                    lane: "runtime".to_string(),
                    kind: "runtime.gateway.restart".to_string(),
                    bridge: None,
                    scope: Some(runtime_scope("machine-1")),
                    payload: json!({ "reason": "expired-lease-test" }),
                    ttl_secs: Some(60),
                },
            )
            .unwrap();
        store
            .finitechat_command_ledger
            .append_command_lease_at(&event, 1_000)
            .unwrap();
        let claimed = store.claim_events("machine-1", None, Some(10)).unwrap();
        assert_eq!(claimed.events, vec![event.clone()]);
        assert!(
            store
                .claim_events("machine-1", None, Some(10))
                .unwrap()
                .events
                .is_empty()
        );
        let leases = finitechat_command_leases(&db_path, "machine-1").unwrap();
        assert_eq!(leases.len(), 2);
        assert_eq!(leases[0].request_id, event.id);
        assert!(leases[0].expires_at_ms < now_millis());
        assert_eq!(leases[1].request_id, event.id);
        assert!(leases[1].expires_at_ms > now_millis());
    }

    #[test]
    fn relay_commands_project_failed_results_from_finitechat() {
        let root = tempdir().unwrap();
        let db_path = root.path().join(FINITE_CHAT_COMMAND_LEDGER_DB_FILE);
        let store = RelayStore::new(root.path());
        let event = store
            .create_event(
                "machine-1",
                &CreateRelayEventInput {
                    lane: "runtime".to_string(),
                    kind: "runtime.gateway.restart".to_string(),
                    bridge: None,
                    scope: Some(runtime_scope("machine-1")),
                    payload: json!({ "reason": "failure-test" }),
                    ttl_secs: Some(60),
                },
            )
            .unwrap();

        assert_eq!(
            store
                .claim_events("machine-1", None, Some(10))
                .unwrap()
                .events,
            vec![event.clone()]
        );
        store
            .store_result(
                "machine-1",
                &StoreRelayResultInput {
                    event_id: event.id.clone(),
                    ok: false,
                    output: Value::Null,
                    error: Some("gateway unavailable".to_string()),
                },
            )
            .unwrap();

        let result = store
            .wait_result("machine-1", &event.id)
            .unwrap()
            .expect("failed result should project from finitechat");
        assert!(!result.ok);
        assert_eq!(result.output, Value::Null);
        assert_eq!(result.error.as_deref(), Some("gateway unavailable"));
        assert!(
            !root
                .path()
                .join("machines/machine-1/results")
                .join(format!("{}.json", event.id))
                .exists()
        );

        let payloads = finitechat_command_payloads(&db_path, "machine-1").unwrap();
        assert_eq!(payloads.len(), 2);
        let FiniteChatEventPayload::RuntimeCommandResult(result) = &payloads[1] else {
            panic!("second command event should be a result");
        };
        assert_eq!(result.status, FiniteChatCommandStatus::Failed);
        assert_eq!(
            result.result.get("error").and_then(Value::as_str),
            Some("gateway unavailable")
        );
    }

    #[test]
    fn relay_status_snapshots_reject_invalid_or_oversized_data() {
        let root = tempdir().unwrap();
        let store = RelayStore::new(root.path());

        let invalid_key = store
            .store_status_snapshot(
                "machine-1",
                &StoreRelayStatusSnapshotInput {
                    state_key: "../runtime".to_string(),
                    schema: "finitecomputer.runtime.inference.status.v1".to_string(),
                    revision: Some(1),
                    ttl_secs: Some(60),
                    ok: true,
                    status: json!({}),
                    error: None,
                },
            )
            .expect_err("invalid state keys must be rejected");
        assert!(invalid_key.to_string().contains("invalid state key"));

        let oversized = store
            .store_status_snapshot(
                "machine-1",
                &StoreRelayStatusSnapshotInput {
                    state_key: "runtime.inference.status".to_string(),
                    schema: "finitecomputer.runtime.inference.status.v1".to_string(),
                    revision: Some(1),
                    ttl_secs: Some(60),
                    ok: true,
                    status: Value::String(
                        "x".repeat(MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES as usize + 1),
                    ),
                    error: None,
                },
            )
            .expect_err("oversized status snapshots must be rejected");
        assert!(
            oversized
                .to_string()
                .contains("status snapshot payload exceeds")
        );

        let oversized_error = store
            .store_status_snapshot(
                "machine-1",
                &StoreRelayStatusSnapshotInput {
                    state_key: "runtime.inference.status".to_string(),
                    schema: "finitecomputer.runtime.inference.status.v1".to_string(),
                    revision: Some(1),
                    ttl_secs: Some(60),
                    ok: false,
                    status: Value::Null,
                    error: Some("x".repeat(MAX_RUNTIME_STATE_SNAPSHOT_PAYLOAD_BYTES as usize + 1)),
                },
            )
            .expect_err("oversized status errors must be rejected");
        assert!(
            oversized_error
                .to_string()
                .contains("status snapshot error exceeds")
        );
    }

    fn finitechat_command_payloads(
        db_path: &Path,
        machine_id: &str,
    ) -> Result<Vec<FiniteChatEventPayload>> {
        let seed = finitechat_relay_room_seed(machine_id);
        let store = SqliteDeliveryStore::open(db_path)?;
        let room = store
            .room(&seed.room_id)?
            .ok_or_else(|| anyhow::anyhow!("command room missing"))?;
        assert!(room.device_active_at_head(&seed.user_device));
        assert!(room.device_active_at_head(&seed.runtime_device));
        let mut payloads = Vec::new();
        for entry in room
            .log
            .iter()
            .filter(|entry| entry.kind == LogEntryKind::Application)
        {
            let event: DecryptedApplicationEventV1 =
                serde_json::from_slice(&entry.envelope.payload)?;
            if !matches!(
                event.kind,
                DurableAppEventKind::RuntimeCommandRequest
                    | DurableAppEventKind::RuntimeCommandResult
            ) {
                continue;
            }
            payloads.push(serde_json::from_slice(&event.payload)?);
        }
        Ok(payloads)
    }

    fn finitechat_command_payloads_or_empty(
        root: &Path,
        machine_id: &str,
    ) -> Result<Vec<FiniteChatEventPayload>> {
        let db_path = root.join(FINITE_CHAT_COMMAND_LEDGER_DB_FILE);
        if !db_path.exists() {
            return Ok(Vec::new());
        }
        finitechat_command_payloads(&db_path, machine_id)
    }

    fn finitechat_command_leases(
        db_path: &Path,
        machine_id: &str,
    ) -> Result<Vec<RelayCommandLease>> {
        let seed = finitechat_relay_room_seed(machine_id);
        let store = SqliteDeliveryStore::open(db_path)?;
        let room = store
            .room(&seed.room_id)?
            .ok_or_else(|| anyhow::anyhow!("command room missing"))?;
        assert!(room.device_active_at_head(&seed.user_device));
        assert!(room.device_active_at_head(&seed.runtime_device));
        let mut leases = Vec::new();
        for entry in room
            .log
            .iter()
            .filter(|entry| entry.kind == LogEntryKind::Application)
        {
            let event: DecryptedApplicationEventV1 =
                serde_json::from_slice(&entry.envelope.payload)?;
            if event.kind != DurableAppEventKind::RuntimeStateSnapshot {
                continue;
            }
            let payload: FiniteChatEventPayload = serde_json::from_slice(&event.payload)?;
            let FiniteChatEventPayload::RuntimeStateSnapshot(snapshot) = payload else {
                continue;
            };
            if snapshot.state_key != COMMAND_LEASE_STATE_KEY {
                continue;
            }
            leases.push(relay_command_lease_from_snapshot(snapshot)?);
        }
        Ok(leases)
    }

    fn chat_message(id: &str, thread_id: &str, created_at: &str, body: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            thread_id: thread_id.to_string(),
            sender_type: "user".to_string(),
            kind: "message".to_string(),
            status: "complete".to_string(),
            body: body.to_string(),
            metadata: json!({}),
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
        }
    }

    fn chat_thread(
        id: &str,
        project_agent_id: &str,
        last_activity_at: &str,
        message_count: usize,
    ) -> ChatThread {
        ChatThread {
            id: id.to_string(),
            project_agent_id: project_agent_id.to_string(),
            created_by: "user-1".to_string(),
            title: "Thread 1".to_string(),
            created_at: "2026-05-22T00:01:00Z".to_string(),
            last_activity_at: last_activity_at.to_string(),
            message_count,
        }
    }

    fn relay_bridge_device(account_id: &str) -> RelayBridgeDevice {
        RelayBridgeDevice {
            bridge_account_id: account_id.to_string(),
            bridge_device_id: "dashboard-bridge-v1".to_string(),
        }
    }

    fn runtime_scope(machine_id: &str) -> RelayCommandScope {
        RelayCommandScope {
            machine_id: machine_id.to_string(),
            room_id: None,
            conversation_id: None,
            topic_id: None,
            project_id: None,
            project_agent_id: None,
            runtime_id: Some(format!("runtime:{machine_id}")),
            target_device_id: None,
            actor_device_id: None,
        }
    }
}
