use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use finitechat_blob::{
    BlobDescriptor, PreparedAttachmentUpload, decrypt_attachment_ciphertext,
    finish_attachment_upload, prepare_attachment_upload, sha256_hex,
};
use finitechat_proto::{
    AttachmentBlobMetadataV1, AttachmentBlobReferenceV1, DeviceRef, MAX_ATTACHMENT_PLAINTEXT_BYTES,
};
use finitechat_store::SqliteDeliveryStore;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::chat::{
    CHAT_ATTACHMENT_BLOB_METADATA_KEY, ChatBootstrapData, ChatCapability, ChatInboxEvent,
    ChatMachine, ChatMessage, ChatMessageAttachmentInput, ChatMessagePage, ChatProjectAgent,
    ChatSite, ChatSkill, ChatSlashCommand, ChatThread, ChatUser, CreateChatThreadRequest,
    GatewayAckRequest, GatewayEditMessageRequest, GatewayMessageEvent,
    GatewayOutboundMessageRequest, GatewayPollRequest, GatewayPollResponse, SendChatMessageRequest,
};
use crate::finite_chat::{
    FiniteChatAppendInput, FiniteChatEventPayload, FiniteChatMessagePayload, FiniteChatRoomSeed,
    finitechat_device, finitechat_ensure_local_room_ready, finitechat_protocol_object_id,
    finitechat_push_app_event_to_store,
};

const DEFAULT_THREAD_TITLE: &str = "New chat";
const MAX_PROVISIONAL_THREAD_TITLE_CHARS: usize = 64;
const DEFAULT_MAX_CHAT_ATTACHMENT_BYTES: usize = MAX_ATTACHMENT_PLAINTEXT_BYTES as usize;
const MAX_CHAT_ATTACHMENT_BYTES_ENV: &str = "FINITE_CHAT_MAX_ATTACHMENT_BYTES";
const DEFAULT_BOOTSTRAP_MESSAGE_LIMIT: usize = 80;
const DEFAULT_MESSAGE_PAGE_LIMIT: usize = 80;
const MAX_MESSAGE_PAGE_LIMIT: usize = 200;
const FINITE_CHAT_MIRROR_ENABLED_ENV: &str = "FINITE_CHAT_MIRROR_ENABLED";
const FINITE_CHAT_MIRROR_DB_FILE: &str = "finitechat.sqlite3";
const ATTACHMENT_BLOBS_DIR: &str = "attachment-blobs";
const FINITE_CHAT_LOCAL_BLOB_URL_PREFIX: &str = "finitechat+local-blob://sha256/";
const MAX_CHAT_ATTACHMENTS_PER_MESSAGE: usize = 32;
const ATTACHMENT_BLOB_BACKFILL_BATCH_SIZE: usize = 128;
const MAX_ATTACHMENT_BLOB_BACKFILLS_PER_OPEN: usize = 4096;
const MAX_ATTACHMENT_BLOB_BACKFILL_MESSAGE_UPDATES: usize = 64;
const MAX_RELAY_CHAT_INBOX_SEQ: u64 = i64::MAX as u64;

pub struct ChatRuntime {
    store: ChatStore,
}

pub struct ChatRuntimeHealth {
    pub ok: bool,
    pub state_dir: PathBuf,
    pub db_path: PathBuf,
    pub machine_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatListMessagesRequest {
    #[serde(rename = "threadId")]
    pub thread_id: String,
    pub limit: Option<usize>,
    pub before: Option<String>,
}

impl ChatListMessagesRequest {
    pub fn limit(&self) -> usize {
        self.limit
            .unwrap_or(DEFAULT_MESSAGE_PAGE_LIMIT)
            .clamp(1, MAX_MESSAGE_PAGE_LIMIT)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatSendMessageRequest {
    #[serde(rename = "threadId")]
    pub thread_id: String,
    pub message: SendChatMessageRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatGetAttachmentRequest {
    #[serde(rename = "attachmentId")]
    pub attachment_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatAttachmentData {
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "dataBase64")]
    pub data_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatAttachmentBlobUpload {
    pub sha256: String,
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatLogSyncCursor {
    pub updated_at: String,
    pub message_id: String,
}

struct ChatStore {
    conn: Connection,
    state_dir: PathBuf,
    attachments_dir: PathBuf,
    attachment_blob_store: LocalAttachmentBlobStore,
    config: RuntimeConfig,
    finitechat_mirror: Option<FiniteChatMirror>,
}

struct StoredAttachment {
    path: PathBuf,
    name: String,
    mime_type: String,
    blob_reference: Option<AttachmentBlobReferenceV1>,
}

struct LegacyAttachmentBlobBackfill {
    id: String,
    name: String,
    mime_type: String,
    path: PathBuf,
}

struct LocalAttachmentBlobStore {
    root: PathBuf,
}

#[derive(Clone)]
struct RuntimeConfig {
    user_id: String,
    user_email: String,
    user_name: String,
    machine_id: String,
    agent_id: String,
    agent_name: String,
    agent_purpose: String,
    hermes_profile_name: String,
    hermes_api_port: u16,
    workspace_ref: String,
}

struct FiniteChatMirror {
    db_path: PathBuf,
    seed: FiniteChatRoomSeed,
}

enum MirrorWriteOutcome {
    Appended { message_id: String, seq: u64 },
    Skipped,
}

impl ChatRuntime {
    pub fn open(state_dir: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            store: ChatStore::open(resolve_state_dir(state_dir)?)?,
        })
    }

    #[cfg(test)]
    fn open_with_finitechat_mirror(state_dir: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            store: ChatStore::open_with_mirror_enabled(resolve_state_dir(state_dir)?, true)?,
        })
    }

    pub fn health(&self) -> ChatRuntimeHealth {
        ChatRuntimeHealth {
            ok: true,
            state_dir: self.store.state_dir.clone(),
            db_path: self.store.state_dir.join("chat.sqlite"),
            machine_id: self.store.config.machine_id.clone(),
            agent_id: self.store.config.agent_id.clone(),
        }
    }

    pub fn bootstrap(&self) -> Result<ChatBootstrapData> {
        self.store.bootstrap()
    }

    pub fn slash_commands(&self) -> Result<Vec<ChatSlashCommand>> {
        load_hermes_slash_commands()
    }

    pub fn threads(&self) -> Result<Vec<ChatThread>> {
        self.store.threads()
    }

    pub fn create_thread(&self, input: CreateChatThreadRequest) -> Result<ChatThread> {
        self.store.create_thread(
            &self.store.config.agent_id,
            input.thread_id.as_deref(),
            input.title.as_deref(),
        )
    }

    pub fn messages_for_thread(&self, thread_id: &str) -> Result<Vec<ChatMessage>> {
        self.store.messages_for_thread(thread_id)
    }

    pub fn message_page_for_thread(
        &self,
        input: &ChatListMessagesRequest,
    ) -> Result<ChatMessagePage> {
        self.store
            .message_page_for_thread(&input.thread_id, input.limit(), input.before.as_deref())
    }

    pub fn chat_log_messages_after(
        &self,
        after_updated_at: Option<&str>,
        after_message_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ChatMessage>> {
        self.store
            .chat_log_messages_after(after_updated_at, after_message_id, limit)
    }

    pub fn relay_chat_log_sync_cursor(
        &self,
        project_agent_id: &str,
    ) -> Result<Option<ChatLogSyncCursor>> {
        self.store.relay_chat_log_sync_cursor(project_agent_id)
    }

    pub fn advance_relay_chat_log_sync_cursor(
        &self,
        project_agent_id: &str,
        cursor: &ChatLogSyncCursor,
    ) -> Result<()> {
        self.store
            .advance_relay_chat_log_sync_cursor(project_agent_id, cursor)
    }

    pub fn send_user_message(&self, input: ChatSendMessageRequest) -> Result<ChatMessage> {
        if input.message.body.trim().is_empty() && input.message.attachments.is_empty() {
            bail!("message body or attachment is required");
        }
        self.store
            .create_user_message(&input.thread_id, input.message)
    }

    pub fn relay_chat_inbox_cursor(&self, project_agent_id: &str) -> Result<u64> {
        self.store.relay_chat_inbox_cursor(project_agent_id)
    }

    pub fn apply_relay_chat_inbox_event(&self, event: &ChatInboxEvent) -> Result<()> {
        self.store.apply_relay_chat_inbox_event(event)
    }

    pub fn attachment(&self, attachment_id: &str) -> Result<ChatAttachmentData> {
        self.store.attachment_data(attachment_id)
    }

    pub fn attachment_blobs_for_messages(
        &self,
        messages: &[ChatMessage],
    ) -> Result<Vec<ChatAttachmentBlobUpload>> {
        self.store.attachment_blobs_for_messages(messages)
    }

    pub fn gateway_poll(&self, input: &GatewayPollRequest) -> Result<GatewayPollResponse> {
        Ok(GatewayPollResponse {
            events: self.store.poll_gateway_events(input)?,
        })
    }

    pub fn gateway_ack(&self, input: &GatewayAckRequest) -> Result<()> {
        self.store.ack_gateway_event(input)
    }

    pub fn gateway_send(&self, input: GatewayOutboundMessageRequest) -> Result<ChatMessage> {
        self.store.create_agent_message(input)
    }

    pub fn gateway_edit(
        &self,
        message_id: &str,
        input: GatewayEditMessageRequest,
    ) -> Result<ChatMessage> {
        self.store.edit_agent_message(message_id, input)
    }

    pub fn recover_interrupted_agent_messages(&self) -> Result<usize> {
        self.store.recover_interrupted_agent_messages()
    }
}

impl ChatStore {
    fn open(state_dir: PathBuf) -> Result<Self> {
        Self::open_with_mirror_enabled(state_dir, finitechat_mirror_enabled())
    }

    fn open_with_mirror_enabled(
        state_dir: PathBuf,
        finitechat_mirror_enabled: bool,
    ) -> Result<Self> {
        fs::create_dir_all(&state_dir)
            .with_context(|| format!("failed to create {}", state_dir.display()))?;
        let attachments_dir = state_dir.join("attachments");
        fs::create_dir_all(&attachments_dir)
            .with_context(|| format!("failed to create {}", attachments_dir.display()))?;
        let attachment_blob_store =
            LocalAttachmentBlobStore::open(state_dir.join(ATTACHMENT_BLOBS_DIR))?;
        let db_path = state_dir.join("chat.sqlite");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("failed to open {}", db_path.display()))?;
        let config = runtime_config();
        let finitechat_mirror =
            FiniteChatMirror::open(&state_dir, &config, finitechat_mirror_enabled)
                .context("failed to open finitechat mirror")?;
        let store = Self {
            conn,
            state_dir,
            attachments_dir,
            attachment_blob_store,
            config,
            finitechat_mirror,
        };
        store.migrate()?;
        store.backfill_attachment_blobs()?;
        store.ensure_default_thread()?;
        store.backfill_provisional_thread_titles()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS threads (
              id TEXT PRIMARY KEY,
              project_agent_id TEXT NOT NULL,
              created_by TEXT NOT NULL,
              title TEXT NOT NULL,
              created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS messages (
              id TEXT PRIMARY KEY,
              thread_id TEXT NOT NULL,
              sender_type TEXT NOT NULL,
              kind TEXT NOT NULL,
              status TEXT NOT NULL,
              body TEXT NOT NULL,
              metadata_json TEXT NOT NULL DEFAULT '{}',
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS gateway_inbox (
              id TEXT PRIMARY KEY,
              agent_id TEXT NOT NULL,
              thread_id TEXT NOT NULL,
              message_id TEXT NOT NULL,
              user_id TEXT NOT NULL,
              user_name TEXT NOT NULL,
              body TEXT NOT NULL,
              metadata_json TEXT NOT NULL DEFAULT '{}',
              created_at TEXT NOT NULL,
              claimed_at TEXT,
              acknowledged_at TEXT
            );

            CREATE TABLE IF NOT EXISTS attachments (
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL,
              mime_type TEXT NOT NULL,
              size_bytes INTEGER NOT NULL,
              path TEXT NOT NULL,
              blob_reference_json TEXT,
              created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS finitechat_mirror_failures (
              id TEXT PRIMARY KEY,
              message_id TEXT NOT NULL,
              operation TEXT NOT NULL,
              error TEXT NOT NULL,
              created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS relay_chat_inbox_cursors (
              project_agent_id TEXT PRIMARY KEY,
              last_seq INTEGER NOT NULL CHECK (last_seq >= 0),
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS relay_chat_log_sync_cursors (
              project_agent_id TEXT PRIMARY KEY,
              updated_at TEXT NOT NULL,
              message_id TEXT NOT NULL CHECK (length(message_id) > 0),
              advanced_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS messages_thread_created_idx
              ON messages(thread_id, created_at);
            CREATE INDEX IF NOT EXISTS gateway_inbox_agent_pending_idx
              ON gateway_inbox(agent_id, acknowledged_at, claimed_at, created_at);
            CREATE INDEX IF NOT EXISTS finitechat_mirror_failures_message_idx
              ON finitechat_mirror_failures(message_id, created_at);
            "#,
        )?;
        self.apply_schema_migrations()?;
        Ok(())
    }

    fn apply_schema_migrations(&self) -> Result<()> {
        let attachment_columns = sqlite_table_columns(&self.conn, "attachments")?;
        if !attachment_columns.contains("blob_reference_json") {
            self.conn.execute(
                "ALTER TABLE attachments ADD COLUMN blob_reference_json TEXT",
                [],
            )?;
        }
        Ok(())
    }

    fn backfill_attachment_blobs(&self) -> Result<usize> {
        let mut backfilled = 0_usize;
        loop {
            if backfilled >= MAX_ATTACHMENT_BLOB_BACKFILLS_PER_OPEN {
                let remaining = self.legacy_attachment_blob_backfill_count()?;
                if remaining > 0 {
                    bail!(
                        "attachment blob backfill has {remaining} remaining rows after the {} row startup limit",
                        MAX_ATTACHMENT_BLOB_BACKFILLS_PER_OPEN
                    );
                }
                break;
            }
            let limit = (MAX_ATTACHMENT_BLOB_BACKFILLS_PER_OPEN - backfilled)
                .min(ATTACHMENT_BLOB_BACKFILL_BATCH_SIZE);
            let candidates = self.legacy_attachment_blob_backfill_candidates(limit)?;
            if candidates.is_empty() {
                break;
            }
            assert!(candidates.len() <= ATTACHMENT_BLOB_BACKFILL_BATCH_SIZE);
            for candidate in candidates {
                self.backfill_attachment_blob(candidate)?;
                backfilled += 1;
            }
        }
        assert!(backfilled <= MAX_ATTACHMENT_BLOB_BACKFILLS_PER_OPEN);
        Ok(backfilled)
    }

    fn legacy_attachment_blob_backfill_count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            r#"
            SELECT COUNT(*)
            FROM attachments
            WHERE blob_reference_json IS NULL OR trim(blob_reference_json) = ''
            "#,
            [],
            |row| row.get(0),
        )?;
        if count < 0 {
            bail!("legacy attachment blob backfill count is negative");
        }
        Ok(count as usize)
    }

    fn legacy_attachment_blob_backfill_candidates(
        &self,
        limit: usize,
    ) -> Result<Vec<LegacyAttachmentBlobBackfill>> {
        assert!(limit > 0);
        assert!(limit <= ATTACHMENT_BLOB_BACKFILL_BATCH_SIZE);
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, name, mime_type, path
            FROM attachments
            WHERE blob_reference_json IS NULL OR trim(blob_reference_json) = ''
            ORDER BY created_at, id
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let path: String = row.get(3)?;
            Ok(LegacyAttachmentBlobBackfill {
                id: row.get(0)?,
                name: row.get(1)?,
                mime_type: row.get(2)?,
                path: PathBuf::from(path),
            })
        })?;
        let candidates = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        assert!(candidates.len() <= limit);
        Ok(candidates)
    }

    fn backfill_attachment_blob(&self, candidate: LegacyAttachmentBlobBackfill) -> Result<()> {
        assert!(!candidate.id.is_empty());
        assert!(!candidate.name.trim().is_empty());
        assert!(!candidate.mime_type.trim().is_empty());
        let plaintext = fs::read(&candidate.path).with_context(|| {
            format!(
                "failed to read legacy attachment {} at {}",
                candidate.id,
                candidate.path.display()
            )
        })?;
        if plaintext.is_empty() {
            bail!("legacy attachment {} is empty", candidate.id);
        }
        let max_attachment_bytes = chat_max_attachment_bytes();
        if plaintext.len() > max_attachment_bytes {
            bail!(
                "legacy attachment {} is {} and exceeds the {} chat attachment limit",
                candidate.id,
                format_bytes(plaintext.len()),
                format_bytes(max_attachment_bytes)
            );
        }
        let blob_reference =
            self.attachment_blob_store
                .upload(&candidate.name, &candidate.mime_type, &plaintext)?;
        let blob_reference_json = serde_json::to_string(&blob_reference)?;

        self.conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;
        let result = (|| -> Result<usize> {
            let changed_messages =
                self.backfill_message_attachment_blob_reference(&candidate.id, &blob_reference)?;
            let updated = self.conn.execute(
                r#"
                UPDATE attachments
                SET blob_reference_json = ?1
                WHERE id = ?2
                  AND (blob_reference_json IS NULL OR trim(blob_reference_json) = '')
                "#,
                params![blob_reference_json, candidate.id],
            )?;
            assert!(updated <= 1);
            Ok(changed_messages)
        })();
        match result {
            Ok(changed_messages) => {
                self.conn.execute_batch("COMMIT")?;
                assert!(changed_messages <= MAX_ATTACHMENT_BLOB_BACKFILL_MESSAGE_UPDATES);
                Ok(())
            }
            Err(error) => {
                self.conn.execute_batch("ROLLBACK")?;
                Err(error)
            }
        }
    }

    fn backfill_message_attachment_blob_reference(
        &self,
        attachment_id: &str,
        reference: &AttachmentBlobReferenceV1,
    ) -> Result<usize> {
        assert!(!attachment_id.is_empty());
        reference.validate_limits()?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, metadata_json
            FROM messages
            WHERE instr(metadata_json, ?1) > 0
            ORDER BY updated_at, id
            LIMIT ?2
            "#,
        )?;
        let limit = MAX_ATTACHMENT_BLOB_BACKFILL_MESSAGE_UPDATES + 1;
        let rows = stmt.query_map(params![attachment_id, limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let messages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        if messages.len() > MAX_ATTACHMENT_BLOB_BACKFILL_MESSAGE_UPDATES {
            bail!(
                "attachment {attachment_id} appears in more than {} message metadata rows",
                MAX_ATTACHMENT_BLOB_BACKFILL_MESSAGE_UPDATES
            );
        }

        let mut changed = 0_usize;
        for (message_id, raw_metadata) in messages {
            let mut metadata: Value = serde_json::from_str(&raw_metadata)
                .with_context(|| format!("message {message_id} has invalid metadata JSON"))?;
            if !insert_attachment_blob_reference(&mut metadata, attachment_id, reference)? {
                continue;
            }
            let metadata_json = serde_json::to_string(&metadata)?;
            let updated_at = now_iso()?;
            let updated = self.conn.execute(
                "UPDATE messages SET metadata_json = ?1, updated_at = ?2 WHERE id = ?3",
                params![metadata_json, updated_at, message_id],
            )?;
            assert_eq!(updated, 1);
            changed += 1;
        }
        assert!(changed <= MAX_ATTACHMENT_BLOB_BACKFILL_MESSAGE_UPDATES);
        Ok(changed)
    }

    fn ensure_default_thread(&self) -> Result<()> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM threads WHERE project_agent_id = ?1",
            params![self.config.agent_id],
            |row| row.get(0),
        )?;
        if count == 0 {
            let title = env::var("FINITE_CHAT_DEFAULT_THREAD_TITLE")
                .unwrap_or_else(|_| DEFAULT_THREAD_TITLE.to_string());
            let _ = self.create_thread(&self.config.agent_id, None, Some(&title))?;
        }
        Ok(())
    }

    fn bootstrap(&self) -> Result<ChatBootstrapData> {
        self.ensure_default_thread()?;
        self.backfill_provisional_thread_titles()?;
        let threads = self.threads()?;
        let mut messages = Vec::new();
        for thread in &threads {
            let mut page = self
                .message_page_for_thread(&thread.id, DEFAULT_BOOTSTRAP_MESSAGE_LIMIT, None)?
                .messages;
            messages.append(&mut page);
        }
        assert!(messages.len() <= threads.len() * DEFAULT_BOOTSTRAP_MESSAGE_LIMIT);
        Ok(ChatBootstrapData {
            users: vec![ChatUser {
                id: self.config.user_id.clone(),
                email: self.config.user_email.clone(),
                name: self.config.user_name.clone(),
                disabled_at: None,
            }],
            machines: vec![ChatMachine {
                id: self.config.machine_id.clone(),
                owner_type: "user".to_string(),
                owner_id: self.config.user_id.clone(),
                kind: "hosted".to_string(),
                state: "running".to_string(),
                last_seen_at: Some(now_iso()?),
            }],
            project_agents: vec![ChatProjectAgent {
                id: self.config.agent_id.clone(),
                owner_type: "machine".to_string(),
                owner_id: self.config.machine_id.clone(),
                machine_id: self.config.machine_id.clone(),
                name: self.config.agent_name.clone(),
                purpose: self.config.agent_purpose.clone(),
                hermes_profile_name: self.config.hermes_profile_name.clone(),
                hermes_profile_ref: self.config.hermes_profile_name.clone(),
                hermes_api_port: self.config.hermes_api_port,
                hermes_api_base_url: format!("http://127.0.0.1:{}", self.config.hermes_api_port),
                workspace_ref: self.config.workspace_ref.clone(),
                google_bot_identity: None,
                replicated_from: None,
                archived_at: None,
            }],
            threads,
            messages,
            sites: Vec::<ChatSite>::new(),
            skills: Vec::<ChatSkill>::new(),
            capabilities: Vec::<ChatCapability>::new(),
        })
    }

    fn threads(&self) -> Result<Vec<ChatThread>> {
        let mut statement = self.conn.prepare(
            r#"
            SELECT
              id,
              project_agent_id,
              created_by,
              title,
              created_at,
              COALESCE(
                (
                  SELECT MAX(messages.updated_at)
                  FROM messages
                  WHERE messages.thread_id = threads.id
                ),
                threads.created_at
              ) AS last_activity_at,
              (
                SELECT COUNT(*)
                FROM messages
                WHERE messages.thread_id = threads.id
              ) AS message_count
            FROM threads
            ORDER BY
              last_activity_at DESC,
              threads.created_at DESC,
              threads.id ASC
            "#,
        )?;
        let rows = statement.query_map([], thread_from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn messages_for_thread(&self, thread_id: &str) -> Result<Vec<ChatMessage>> {
        let mut statement = self.conn.prepare(
            r#"
            SELECT id, thread_id, sender_type, kind, status, body, metadata_json, created_at, updated_at
            FROM messages
            WHERE thread_id = ?1
            ORDER BY created_at ASC, rowid ASC
            "#,
        )?;
        let rows = statement.query_map(params![thread_id], message_from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn message_page_for_thread(
        &self,
        thread_id: &str,
        limit: usize,
        before: Option<&str>,
    ) -> Result<ChatMessagePage> {
        if !self.thread_exists(thread_id)? {
            bail!("thread {thread_id} not found");
        }
        let fetch_limit = limit.saturating_add(1) as i64;
        let mut messages = if let Some(before) = before {
            let mut statement = self.conn.prepare(
                r#"
                SELECT id, thread_id, sender_type, kind, status, body, metadata_json, created_at, updated_at
                FROM messages
                WHERE thread_id = ?1
                  AND created_at < ?2
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?3
                "#,
            )?;
            let rows =
                statement.query_map(params![thread_id, before, fetch_limit], message_from_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            let mut statement = self.conn.prepare(
                r#"
                SELECT id, thread_id, sender_type, kind, status, body, metadata_json, created_at, updated_at
                FROM messages
                WHERE thread_id = ?1
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?2
                "#,
            )?;
            let rows = statement.query_map(params![thread_id, fetch_limit], message_from_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        messages.reverse();
        let has_more = messages.len() > limit;
        if has_more {
            messages.remove(0);
        }
        let next_before = has_more
            .then(|| messages.first().map(|message| message.created_at.clone()))
            .flatten();

        Ok(ChatMessagePage {
            messages,
            has_more,
            next_before,
        })
    }

    fn chat_log_messages_after(
        &self,
        after_updated_at: Option<&str>,
        after_message_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ChatMessage>> {
        let limit = limit.clamp(1, MAX_MESSAGE_PAGE_LIMIT);
        let rows = match (after_updated_at, after_message_id) {
            (Some(after_updated_at), Some(after_message_id)) => {
                let mut statement = self.conn.prepare(
                    r#"
                    SELECT id, thread_id, sender_type, kind, status, body, metadata_json, created_at, updated_at
                    FROM messages
                    WHERE updated_at > ?1 OR (updated_at = ?1 AND id > ?2)
                    ORDER BY updated_at ASC, id ASC
                    LIMIT ?3
                    "#,
                )?;
                statement
                    .query_map(
                        params![after_updated_at, after_message_id, limit as i64],
                        message_from_row,
                    )?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            }
            _ => {
                let mut statement = self.conn.prepare(
                    r#"
                    SELECT id, thread_id, sender_type, kind, status, body, metadata_json, created_at, updated_at
                    FROM messages
                    ORDER BY updated_at ASC, id ASC
                    LIMIT ?1
                    "#,
                )?;
                statement
                    .query_map(params![limit as i64], message_from_row)?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            }
        };
        assert!(rows.len() <= limit);
        Ok(rows)
    }

    fn relay_chat_log_sync_cursor(
        &self,
        project_agent_id: &str,
    ) -> Result<Option<ChatLogSyncCursor>> {
        self.validate_agent(project_agent_id)?;
        let cursor = self
            .conn
            .query_row(
                r#"
                SELECT updated_at, message_id
                FROM relay_chat_log_sync_cursors
                WHERE project_agent_id = ?1
                "#,
                params![project_agent_id],
                |row| {
                    Ok(ChatLogSyncCursor {
                        updated_at: row.get(0)?,
                        message_id: row.get(1)?,
                    })
                },
            )
            .optional()?;
        if let Some(cursor) = cursor.as_ref() {
            validate_chat_log_sync_cursor(cursor)?;
        }
        Ok(cursor)
    }

    fn advance_relay_chat_log_sync_cursor(
        &self,
        project_agent_id: &str,
        cursor: &ChatLogSyncCursor,
    ) -> Result<()> {
        self.validate_agent(project_agent_id)?;
        validate_chat_log_sync_cursor(cursor)?;
        let updated = self.conn.execute(
            r#"
            UPDATE relay_chat_log_sync_cursors
            SET updated_at = ?1, message_id = ?2, advanced_at = ?3
            WHERE project_agent_id = ?4
              AND (updated_at < ?1 OR (updated_at = ?1 AND message_id < ?2))
            "#,
            params![
                cursor.updated_at.as_str(),
                cursor.message_id.as_str(),
                now_iso()?,
                project_agent_id,
            ],
        )?;
        if updated == 0 {
            self.conn.execute(
                r#"
                INSERT OR IGNORE INTO relay_chat_log_sync_cursors
                  (project_agent_id, updated_at, message_id, advanced_at)
                VALUES (?1, ?2, ?3, ?4)
                "#,
                params![
                    project_agent_id,
                    cursor.updated_at.as_str(),
                    cursor.message_id.as_str(),
                    now_iso()?,
                ],
            )?;
        }
        let stored = self.relay_chat_log_sync_cursor(project_agent_id)?;
        assert!(stored.is_some());
        assert!(
            stored
                .as_ref()
                .is_some_and(|stored| chat_log_sync_cursor_at_or_after(stored, cursor))
        );
        Ok(())
    }

    fn create_thread(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
        title: Option<&str>,
    ) -> Result<ChatThread> {
        if agent_id != self.config.agent_id {
            bail!("unknown agent_id {agent_id}");
        }
        let thread_id = thread_id
            .map(validate_thread_id)
            .transpose()?
            .map(ToOwned::to_owned);
        if let Some(thread_id) = thread_id.as_deref()
            && let Some(existing) = self.thread_by_id(thread_id)?
        {
            let requested_title = normalized_thread_title(title);
            if existing.title != requested_title && !thread_title_is_auto(&requested_title) {
                bail!("thread {thread_id} already exists with a different title");
            }
            return Ok(existing);
        }
        let created_at = now_iso()?;
        let thread = ChatThread {
            id: thread_id.unwrap_or_else(|| id_with_prefix("thread")),
            project_agent_id: agent_id.to_string(),
            created_by: self.config.user_id.clone(),
            title: normalized_thread_title(title),
            created_at: created_at.clone(),
            last_activity_at: created_at.clone(),
            message_count: 0,
        };
        self.conn.execute(
            "INSERT INTO threads (id, project_agent_id, created_by, title, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                thread.id,
                thread.project_agent_id,
                thread.created_by,
                thread.title,
                created_at,
            ],
        )?;
        Ok(thread)
    }

    fn thread_by_id(&self, thread_id: &str) -> Result<Option<ChatThread>> {
        let mut statement = self.conn.prepare(
            r#"
            SELECT
              threads.id,
              threads.project_agent_id,
              threads.created_by,
              threads.title,
              threads.created_at,
              COALESCE(
                (SELECT MAX(messages.updated_at) FROM messages WHERE messages.thread_id = threads.id),
                threads.created_at
              ) AS last_activity_at,
              (SELECT COUNT(*) FROM messages WHERE messages.thread_id = threads.id) AS message_count
            FROM threads
            WHERE threads.id = ?1
            "#,
        )?;
        statement
            .query_row(params![thread_id], thread_from_row)
            .optional()
            .map_err(Into::into)
    }

    fn thread_exists(&self, thread_id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM threads WHERE id = ?1",
                params![thread_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    fn create_user_message(
        &self,
        thread_id: &str,
        input: SendChatMessageRequest,
    ) -> Result<ChatMessage> {
        if !self.thread_exists(thread_id)? {
            bail!("thread {thread_id} not found");
        }
        let now = now_iso()?;
        let message_id = input
            .client_message_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| id_with_prefix("msg"));
        let attachments = self.save_attachments(&input.attachments)?;
        self.maybe_title_thread_from_first_message(thread_id, &input.body)?;
        let metadata = if attachments.is_empty() {
            json!({})
        } else {
            json!({ "attachments": attachments })
        };

        let message = ChatMessage {
            id: message_id.clone(),
            thread_id: thread_id.to_string(),
            sender_type: "user".to_string(),
            kind: "message".to_string(),
            status: "complete".to_string(),
            body: input.body,
            metadata: metadata.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        self.insert_message(&message)?;
        self.conn.execute(
            r#"
            INSERT INTO gateway_inbox
              (id, agent_id, thread_id, message_id, user_id, user_name, body, metadata_json, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                id_with_prefix("evt"),
                self.config.agent_id,
                thread_id,
                message_id,
                self.config.user_id,
                self.config.user_name,
                message.body,
                serde_json::to_string(&metadata)?,
                now,
            ],
        )?;
        self.try_mirror_message(&message, "chat.send_message")?;
        Ok(message)
    }

    fn relay_chat_inbox_cursor(&self, project_agent_id: &str) -> Result<u64> {
        self.validate_agent(project_agent_id)?;
        self.relay_chat_inbox_cursor_unchecked(project_agent_id)
    }

    fn relay_chat_inbox_cursor_unchecked(&self, project_agent_id: &str) -> Result<u64> {
        assert!(!project_agent_id.is_empty());
        let cursor = self
            .conn
            .query_row(
                "SELECT last_seq FROM relay_chat_inbox_cursors WHERE project_agent_id = ?1",
                params![project_agent_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        sqlite_seq_to_u64(cursor, "relay chat inbox cursor")
    }

    fn apply_relay_chat_inbox_event(&self, event: &ChatInboxEvent) -> Result<()> {
        self.validate_relay_chat_inbox_event(event)?;
        self.conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;
        let result = self.apply_relay_chat_inbox_event_inner(event);
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                let cursor = self.relay_chat_inbox_cursor_unchecked(&event.project_agent_id)?;
                assert!(cursor >= event.seq);
                Ok(())
            }
            Err(error) => {
                self.conn.execute_batch("ROLLBACK")?;
                Err(error)
            }
        }
    }

    fn apply_relay_chat_inbox_event_inner(&self, event: &ChatInboxEvent) -> Result<()> {
        assert!(event.seq > 0);
        assert!(!event.project_agent_id.is_empty());
        let current = self.relay_chat_inbox_cursor_unchecked(&event.project_agent_id)?;
        if event.seq <= current {
            return Ok(());
        }

        match event.kind.as_str() {
            "conversation.create" | "conversation.update" => {
                let conversation = event
                    .conversation
                    .as_ref()
                    .ok_or_else(|| anyhow!("conversation inbox event missing conversation"))?;
                self.apply_relay_chat_conversation(conversation)?;
            }
            "chat.message" => {
                let message = event
                    .message
                    .as_ref()
                    .ok_or_else(|| anyhow!("chat inbox message event missing message"))?;
                self.apply_relay_chat_user_message(event, message)?;
            }
            _ => bail!("unsupported relay chat inbox event {}", event.kind),
        }

        self.advance_relay_chat_inbox_cursor(&event.project_agent_id, event.seq)?;
        Ok(())
    }

    fn validate_relay_chat_inbox_event(&self, event: &ChatInboxEvent) -> Result<()> {
        self.validate_agent(&event.project_agent_id)?;
        if event.seq == 0 {
            bail!("relay chat inbox event seq must be positive");
        }
        if event.seq > MAX_RELAY_CHAT_INBOX_SEQ {
            bail!(
                "relay chat inbox event seq {} exceeds sqlite integer range",
                event.seq
            );
        }
        validate_thread_id(&event.conversation_id)?;
        match event.kind.as_str() {
            "conversation.create" | "conversation.update" => {
                let conversation = event.conversation.as_ref().ok_or_else(|| {
                    anyhow!("relay chat inbox conversation event missing conversation")
                })?;
                if conversation.id != event.conversation_id {
                    bail!("relay chat inbox conversation id mismatch");
                }
                if conversation.project_agent_id != event.project_agent_id {
                    bail!("relay chat inbox conversation project agent mismatch");
                }
            }
            "chat.message" => {
                if event.message.is_none() {
                    bail!("relay chat inbox message event missing message");
                }
            }
            _ => bail!("unsupported relay chat inbox event {}", event.kind),
        }
        Ok(())
    }

    fn apply_relay_chat_conversation(&self, conversation: &ChatThread) -> Result<()> {
        self.validate_agent(&conversation.project_agent_id)?;
        validate_thread_id(&conversation.id)?;
        if conversation.title.trim().is_empty() {
            bail!("relay chat conversation title is required");
        }
        if conversation.created_at.trim().is_empty() {
            bail!("relay chat conversation created_at is required");
        }
        if let Some(existing) = self.thread_by_id(&conversation.id)? {
            assert_eq!(existing.id, conversation.id);
            if existing.project_agent_id != conversation.project_agent_id {
                bail!("relay chat conversation conflicts with existing project agent");
            }
            self.conn.execute(
                "UPDATE threads SET title = ?1 WHERE id = ?2",
                params![conversation.title, conversation.id],
            )?;
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO threads (id, project_agent_id, created_by, title, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                conversation.id,
                conversation.project_agent_id,
                conversation.created_by,
                conversation.title,
                conversation.created_at,
            ],
        )?;
        assert!(self.thread_exists(&conversation.id)?);
        Ok(())
    }

    fn apply_relay_chat_user_message(
        &self,
        event: &ChatInboxEvent,
        message: &ChatMessage,
    ) -> Result<()> {
        assert_eq!(event.kind, "chat.message");
        if message.id.trim().is_empty() {
            bail!("relay chat message id is required");
        }
        if message.thread_id != event.conversation_id {
            bail!("relay chat message conversation does not match event conversation");
        }
        if message.sender_type != "user" {
            bail!("relay chat inbox only accepts user messages");
        }
        if message.kind != "message" {
            bail!("relay chat inbox only accepts durable user messages");
        }
        if !self.thread_exists(&message.thread_id)? {
            bail!("relay chat conversation {} not found", message.thread_id);
        }
        if let Some(existing) = self.message(&message.id)? {
            if chat_messages_same_logical_user_request(&existing, message) {
                self.update_relay_chat_message_metadata_if_newer(&existing, message)?;
                let gateway_message = if existing.updated_at > message.updated_at {
                    &existing
                } else {
                    message
                };
                self.ensure_gateway_inbox_for_relay_message(event, gateway_message)?;
                return Ok(());
            }
            bail!(
                "relay chat message {} conflicts with local message",
                message.id
            );
        }

        self.insert_message(message)?;
        self.ensure_gateway_inbox_for_relay_message(event, message)?;
        Ok(())
    }

    fn update_relay_chat_message_metadata_if_newer(
        &self,
        existing: &ChatMessage,
        incoming: &ChatMessage,
    ) -> Result<()> {
        assert_eq!(existing.id, incoming.id);
        assert!(chat_messages_same_logical_user_request(existing, incoming));
        if incoming.updated_at <= existing.updated_at {
            return Ok(());
        }

        let metadata_json = serde_json::to_string(&incoming.metadata)?;
        self.conn.execute(
            r#"
            UPDATE messages
            SET metadata_json = ?1, updated_at = ?2
            WHERE id = ?3 AND updated_at < ?2
            "#,
            params![metadata_json, incoming.updated_at, incoming.id],
        )?;
        self.conn.execute(
            r#"
            UPDATE gateway_inbox
            SET metadata_json = ?1
            WHERE message_id = ?2
              AND claimed_at IS NULL
              AND acknowledged_at IS NULL
            "#,
            params![metadata_json, incoming.id],
        )?;
        Ok(())
    }

    fn ensure_gateway_inbox_for_relay_message(
        &self,
        event: &ChatInboxEvent,
        message: &ChatMessage,
    ) -> Result<()> {
        assert!(event.seq > 0);
        assert!(!message.id.is_empty());
        let inbox_id = finitechat_protocol_object_id(
            "relay-inbox",
            &format!(
                "{}:{}:{}",
                event.project_agent_id, event.conversation_id, message.id
            ),
        );
        self.conn.execute(
            r#"
            INSERT OR IGNORE INTO gateway_inbox
              (id, agent_id, thread_id, message_id, user_id, user_name, body, metadata_json, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                inbox_id,
                event.project_agent_id,
                message.thread_id,
                message.id,
                self.config.user_id,
                self.config.user_name,
                message.body,
                serde_json::to_string(&message.metadata)?,
                message.created_at,
            ],
        )?;
        Ok(())
    }

    fn advance_relay_chat_inbox_cursor(&self, project_agent_id: &str, seq: u64) -> Result<()> {
        assert!(!project_agent_id.is_empty());
        assert!(seq > 0);
        let seq_i64 = u64_to_sqlite_seq(seq, "relay chat inbox seq")?;
        let updated = self.conn.execute(
            r#"
            UPDATE relay_chat_inbox_cursors
            SET last_seq = ?1, updated_at = ?2
            WHERE project_agent_id = ?3 AND last_seq < ?1
            "#,
            params![seq_i64, now_iso()?, project_agent_id],
        )?;
        if updated == 0 {
            self.conn.execute(
                r#"
                INSERT OR IGNORE INTO relay_chat_inbox_cursors
                  (project_agent_id, last_seq, updated_at)
                VALUES (?1, ?2, ?3)
                "#,
                params![project_agent_id, seq_i64, now_iso()?],
            )?;
        }
        let cursor = self.relay_chat_inbox_cursor_unchecked(project_agent_id)?;
        assert!(cursor >= seq);
        Ok(())
    }

    fn maybe_title_thread_from_first_message(&self, thread_id: &str, body: &str) -> Result<()> {
        let Some(next_title) = provisional_thread_title(body) else {
            return Ok(());
        };
        let Some((current_title, user_message_count)) = self
            .conn
            .query_row(
                r#"
                SELECT
                  title,
                  (SELECT COUNT(*) FROM messages WHERE thread_id = ?1 AND sender_type = 'user')
                FROM threads
                WHERE id = ?1
                "#,
                params![thread_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
        else {
            bail!("thread {thread_id} not found");
        };

        if user_message_count == 0 && thread_title_is_auto(&current_title) {
            self.conn.execute(
                "UPDATE threads SET title = ?1 WHERE id = ?2",
                params![next_title, thread_id],
            )?;
        }
        Ok(())
    }

    fn backfill_provisional_thread_titles(&self) -> Result<()> {
        let threads = {
            let mut statement = self.conn.prepare(
                r#"
                SELECT
                  threads.id,
                  threads.title,
                  (
                    SELECT messages.body
                    FROM messages
                    WHERE messages.thread_id = threads.id
                      AND messages.sender_type = 'user'
                    ORDER BY messages.created_at ASC
                    LIMIT 1
                  ) AS first_user_body
                FROM threads
                "#,
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        for (thread_id, current_title, first_user_body) in threads {
            if !thread_title_is_auto(&current_title) {
                continue;
            }
            let Some(first_user_body) = first_user_body else {
                continue;
            };
            let Some(next_title) = provisional_thread_title(&first_user_body) else {
                continue;
            };
            self.conn.execute(
                "UPDATE threads SET title = ?1 WHERE id = ?2",
                params![next_title, thread_id],
            )?;
        }

        Ok(())
    }

    fn create_agent_message(&self, input: GatewayOutboundMessageRequest) -> Result<ChatMessage> {
        self.validate_agent(&input.agent_id)?;
        if !self.thread_exists(&input.thread_id)? {
            bail!("thread {} not found", input.thread_id);
        }
        let kind = input.kind().to_string();
        let status = input.status().to_string();
        let mut metadata = self.store_agent_attachment_paths(input.metadata)?;
        if let Some(reply_to) = input.reply_to_message_id {
            if let Value::Object(ref mut map) = metadata {
                map.insert("reply_to_message_id".to_string(), Value::String(reply_to));
            } else {
                metadata = json!({ "reply_to_message_id": reply_to });
            }
        }
        let now = now_iso()?;
        let message = ChatMessage {
            id: input.message_id.unwrap_or_else(|| id_with_prefix("msg")),
            thread_id: input.thread_id,
            sender_type: "agent".to_string(),
            kind,
            status,
            body: input.body,
            metadata,
            created_at: now.clone(),
            updated_at: now,
        };
        self.insert_message(&message)?;
        self.try_mirror_message(&message, "gateway.send")?;
        Ok(message)
    }

    fn edit_agent_message(
        &self,
        message_id: &str,
        input: GatewayEditMessageRequest,
    ) -> Result<ChatMessage> {
        self.validate_agent(&input.agent_id)?;
        let current = self
            .message(message_id)?
            .ok_or_else(|| anyhow!("message {message_id} not found"))?;
        if current.sender_type != "agent" {
            bail!("message {message_id} is not an agent message");
        }
        let status = input.status().to_string();
        let metadata = if input.metadata.is_null() {
            current.metadata
        } else {
            input.metadata
        };
        let now = now_iso()?;
        self.conn.execute(
            "UPDATE messages SET body = ?1, status = ?2, metadata_json = ?3, updated_at = ?4 WHERE id = ?5",
            params![
                input.body,
                status,
                serde_json::to_string(&metadata)?,
                now,
                message_id,
            ],
        )?;
        self.message(message_id)?
            .ok_or_else(|| anyhow!("message {message_id} not found after edit"))
    }

    fn recover_interrupted_agent_messages(&self) -> Result<usize> {
        let now = now_iso()?;
        let changed = self.conn.execute(
            r#"
            UPDATE messages
            SET
              status = 'error',
              body = CASE
                WHEN kind = 'status' THEN 'Interrupted: the agent gateway restarted before this turn completed.'
                ELSE body
              END,
              updated_at = ?1
            WHERE sender_type = 'agent'
              AND status IN ('queued', 'running')
            "#,
            params![now],
        )?;
        Ok(changed)
    }

    fn store_agent_attachment_paths(&self, metadata: Value) -> Result<Value> {
        let mut metadata = metadata;
        let Some(attachments) = metadata
            .get_mut("attachments")
            .and_then(Value::as_array_mut)
        else {
            return Ok(metadata);
        };

        for attachment in attachments {
            let Value::Object(map) = attachment else {
                continue;
            };
            let Some(raw_path) = map
                .get("path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
            else {
                continue;
            };

            let path = PathBuf::from(&raw_path);
            if !path.is_file() {
                bail!("agent attachment path {} is not a file", raw_path);
            }
            let name = map
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    path.file_name()
                        .map(|value| value.to_string_lossy().to_string())
                })
                .unwrap_or_else(|| "attachment".to_string());
            let mime_type = map
                .get("mime_type")
                .or_else(|| map.get("mimeType"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("application/octet-stream")
                .to_string();
            let data = fs::read(&path)
                .with_context(|| format!("failed to read agent attachment {}", path.display()))?;
            let stored = self.save_attachment_bytes(&name, &mime_type, &data)?;
            *attachment = stored;
        }

        Ok(metadata)
    }

    fn poll_gateway_events(&self, input: &GatewayPollRequest) -> Result<Vec<GatewayMessageEvent>> {
        self.validate_agent(&input.agent_id)?;
        let now = now_iso()?;
        let stale_claim_cutoff = OffsetDateTime::now_utc()
            .checked_sub(time::Duration::seconds(60))
            .unwrap_or_else(OffsetDateTime::now_utc)
            .format(&Rfc3339)?;
        let mut statement = self.conn.prepare(
            r#"
            SELECT id, agent_id, thread_id, message_id, user_id, user_name, body, metadata_json, created_at
            FROM gateway_inbox
            WHERE agent_id = ?1
              AND acknowledged_at IS NULL
              AND (claimed_at IS NULL OR claimed_at < ?2)
            ORDER BY created_at ASC
            LIMIT ?3
            "#,
        )?;
        let rows = statement.query_map(
            params![input.agent_id, stale_claim_cutoff, input.limit() as i64],
            |row| {
                let metadata_json: String = row.get(7)?;
                Ok(GatewayMessageEvent {
                    id: row.get(0)?,
                    project_agent_id: row.get(1)?,
                    thread_id: row.get(2)?,
                    message_id: row.get(3)?,
                    user_id: row.get(4)?,
                    user_name: row.get(5)?,
                    body: row.get(6)?,
                    metadata: parse_metadata(&metadata_json),
                    created_at: row.get(8)?,
                })
            },
        )?;
        let mut events = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        for event in &mut events {
            event.metadata = self.gateway_metadata_with_attachment_paths(event.metadata.clone())?;
        }
        for event in &events {
            self.conn.execute(
                "UPDATE gateway_inbox SET claimed_at = ?1 WHERE id = ?2 AND acknowledged_at IS NULL",
                params![now, event.id],
            )?;
        }
        Ok(events)
    }

    fn gateway_metadata_with_attachment_paths(&self, mut metadata: Value) -> Result<Value> {
        let Some(attachments) = metadata
            .get_mut("attachments")
            .and_then(Value::as_array_mut)
        else {
            return Ok(metadata);
        };

        for attachment in attachments {
            let Value::Object(map) = attachment else {
                continue;
            };
            let Some(id) = map.get("id").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            map.remove(CHAT_ATTACHMENT_BLOB_METADATA_KEY);
            let path = self
                .conn
                .query_row(
                    "SELECT path FROM attachments WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if let Some(path) = path {
                map.insert("path".to_string(), Value::String(path));
            }
        }

        Ok(metadata)
    }

    fn ack_gateway_event(&self, input: &GatewayAckRequest) -> Result<()> {
        self.validate_agent(&input.agent_id)?;
        self.conn.execute(
            "UPDATE gateway_inbox SET acknowledged_at = ?1 WHERE id = ?2 AND agent_id = ?3",
            params![now_iso()?, input.event_id, input.agent_id],
        )?;
        Ok(())
    }

    fn try_mirror_message(&self, message: &ChatMessage, operation: &'static str) -> Result<()> {
        let Some(mirror) = &self.finitechat_mirror else {
            return Ok(());
        };
        assert!(!operation.is_empty());
        assert!(!message.id.is_empty());
        match mirror.append_chat_message(message) {
            Ok(MirrorWriteOutcome::Appended {
                message_id: finitechat_message_id,
                seq,
            }) => {
                assert!(!finitechat_message_id.is_empty());
                assert!(seq > 0);
                Ok(())
            }
            Ok(MirrorWriteOutcome::Skipped) => Ok(()),
            Err(error) => self.record_finitechat_mirror_failure(message, operation, &error),
        }
    }

    fn record_finitechat_mirror_failure(
        &self,
        message: &ChatMessage,
        operation: &'static str,
        error: &anyhow::Error,
    ) -> Result<()> {
        assert!(!message.id.is_empty());
        assert!(!operation.is_empty());
        self.conn.execute(
            r#"
            INSERT INTO finitechat_mirror_failures
              (id, message_id, operation, error, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                id_with_prefix("finitechat_mirror_failure"),
                message.id,
                operation,
                error.to_string(),
                now_iso()?,
            ],
        )?;
        Ok(())
    }

    fn validate_agent(&self, agent_id: &str) -> Result<()> {
        if agent_id == self.config.agent_id {
            Ok(())
        } else {
            bail!("unknown agent_id {agent_id}")
        }
    }

    fn insert_message(&self, message: &ChatMessage) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO messages
              (id, thread_id, sender_type, kind, status, body, metadata_json, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                message.id,
                message.thread_id,
                message.sender_type,
                message.kind,
                message.status,
                message.body,
                serde_json::to_string(&message.metadata)?,
                message.created_at,
                message.updated_at,
            ],
        )?;
        Ok(())
    }

    fn message(&self, message_id: &str) -> Result<Option<ChatMessage>> {
        self.conn
            .query_row(
                r#"
                SELECT id, thread_id, sender_type, kind, status, body, metadata_json, created_at, updated_at
                FROM messages
                WHERE id = ?1
                "#,
                params![message_id],
                message_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn save_attachments(&self, attachments: &[ChatMessageAttachmentInput]) -> Result<Vec<Value>> {
        if attachments.len() > MAX_CHAT_ATTACHMENTS_PER_MESSAGE {
            bail!(
                "chat message contains {} attachments, max is {}",
                attachments.len(),
                MAX_CHAT_ATTACHMENTS_PER_MESSAGE
            );
        }
        let mut saved = Vec::new();
        let max_attachment_bytes = chat_max_attachment_bytes();
        for attachment in attachments {
            validate_attachment_size(
                &attachment.name,
                &attachment.data_base64,
                max_attachment_bytes,
            )?;
            let data = BASE64_STANDARD
                .decode(attachment.data_base64.as_bytes())
                .with_context(|| format!("attachment {} is not valid base64", attachment.name))?;
            saved.push(self.save_attachment_bytes(
                &attachment.name,
                &attachment.mime_type,
                &data,
            )?);
        }
        Ok(saved)
    }

    fn attachment_blobs_for_messages(
        &self,
        messages: &[ChatMessage],
    ) -> Result<Vec<ChatAttachmentBlobUpload>> {
        let mut seen_sha256 = BTreeSet::new();
        let mut blobs = Vec::new();
        for message in messages {
            let references = attachment_blob_references_from_metadata(&message.metadata)?;
            for reference in references {
                let sha256 = local_blob_sha_from_url(&reference.url)?;
                if sha256 != reference.ciphertext_sha256 {
                    bail!("attachment blob URL hash does not match reference hash");
                }
                if !seen_sha256.insert(sha256.clone()) {
                    continue;
                }
                let ciphertext = self.attachment_blob_store.ciphertext(&sha256)?;
                if sha256_hex(&ciphertext) != sha256 {
                    bail!("attachment blob hash mismatch for {sha256}");
                }
                blobs.push(ChatAttachmentBlobUpload { sha256, ciphertext });
            }
        }
        assert!(blobs.len() <= messages.len() * MAX_CHAT_ATTACHMENTS_PER_MESSAGE);
        Ok(blobs)
    }

    fn save_attachment_bytes(&self, name: &str, mime_type: &str, data: &[u8]) -> Result<Value> {
        let max_attachment_bytes = chat_max_attachment_bytes();
        if data.is_empty() {
            bail!("attachment {name} is empty");
        }
        if data.len() > max_attachment_bytes {
            bail!(
                "attachment {} is {} and exceeds the {} chat attachment limit",
                name,
                format_bytes(data.len()),
                format_bytes(max_attachment_bytes)
            );
        }
        let blob_reference = self.attachment_blob_store.upload(name, mime_type, data)?;
        let blob_reference_json = serde_json::to_string(&blob_reference)?;
        let id = id_with_prefix("att");
        let safe_name = sanitize_file_name(name);
        let path = self.attachments_dir.join(format!("{id}-{safe_name}"));
        fs::write(&path, data).with_context(|| format!("failed to write {}", path.display()))?;
        let now = now_iso()?;
        self.conn.execute(
            r#"
            INSERT INTO attachments
              (id, name, mime_type, size_bytes, path, blob_reference_json, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                id,
                name,
                mime_type,
                data.len() as i64,
                path.display().to_string(),
                blob_reference_json,
                now,
            ],
        )?;
        let mut metadata = json!({
            "id": id,
            "type": attachment_kind(mime_type),
            "name": name,
            "mime_type": mime_type,
            "size_bytes": data.len() as u64,
            "url": format!("/attachments/{id}"),
        });
        let Some(metadata_map) = metadata.as_object_mut() else {
            bail!("attachment metadata must be a JSON object");
        };
        metadata_map.insert(
            CHAT_ATTACHMENT_BLOB_METADATA_KEY.to_string(),
            serde_json::to_value(&blob_reference)?,
        );
        assert!(metadata_map.contains_key(CHAT_ATTACHMENT_BLOB_METADATA_KEY));
        Ok(metadata)
    }

    fn attachment_data(&self, attachment_id: &str) -> Result<ChatAttachmentData> {
        let attachment = self.attachment_record(attachment_id)?;
        if attachment.name.trim().is_empty() {
            bail!("attachment {attachment_id} has an empty filename");
        }
        if attachment.mime_type.trim().is_empty() {
            bail!("attachment {attachment_id} has an empty MIME type");
        }
        let bytes = if let Some(reference) = &attachment.blob_reference {
            self.attachment_blob_store.download(reference)?
        } else {
            fs::read(&attachment.path).with_context(|| {
                format!("failed to read attachment {}", attachment.path.display())
            })?
        };
        if bytes.is_empty() {
            bail!("attachment {attachment_id} is empty");
        }
        Ok(ChatAttachmentData {
            name: attachment.name,
            mime_type: attachment.mime_type,
            data_base64: BASE64_STANDARD.encode(bytes),
        })
    }

    fn attachment_record(&self, attachment_id: &str) -> Result<StoredAttachment> {
        let Some((path, name, mime_type, blob_reference_json)) = self
            .conn
            .query_row(
                "SELECT path, name, mime_type, blob_reference_json FROM attachments WHERE id = ?1",
                params![attachment_id],
                |row| {
                    let path: String = row.get(0)?;
                    let name: String = row.get(1)?;
                    let mime_type: String = row.get(2)?;
                    let blob_reference_json: Option<String> = row.get(3)?;
                    Ok((path, name, mime_type, blob_reference_json))
                },
            )
            .optional()?
        else {
            bail!("attachment not found");
        };
        let blob_reference =
            parse_attachment_blob_reference(attachment_id, blob_reference_json.as_deref())?;
        Ok(StoredAttachment {
            path: PathBuf::from(path),
            name,
            mime_type,
            blob_reference,
        })
    }
}

impl LocalAttachmentBlobStore {
    fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        Ok(Self { root })
    }

    fn upload(
        &self,
        name: &str,
        mime_type: &str,
        plaintext: &[u8],
    ) -> Result<AttachmentBlobReferenceV1> {
        assert!(!plaintext.is_empty());
        let metadata = attachment_blob_metadata(name, mime_type)?;
        let prepared = prepare_attachment_upload(plaintext, metadata)
            .with_context(|| format!("failed to prepare encrypted blob for attachment {name}"))?;
        self.put_prepared(&prepared)
    }

    fn download(&self, reference: &AttachmentBlobReferenceV1) -> Result<Vec<u8>> {
        let sha256 = local_blob_sha_from_url(&reference.url)?;
        if sha256 != reference.ciphertext_sha256 {
            bail!("attachment blob URL hash does not match reference hash");
        }
        let ciphertext = self.ciphertext(&sha256)?;
        let downloaded = decrypt_attachment_ciphertext(reference, &ciphertext)
            .with_context(|| format!("failed to decrypt attachment blob {}", reference.url))?;
        assert_eq!(downloaded.reference, *reference);
        assert!(!downloaded.plaintext.is_empty());
        Ok(downloaded.plaintext)
    }

    fn put_prepared(
        &self,
        prepared: &PreparedAttachmentUpload,
    ) -> Result<AttachmentBlobReferenceV1> {
        assert!(!prepared.ciphertext.is_empty());
        assert_eq!(prepared.ciphertext.len() as u64, prepared.ciphertext_size);
        let sha256 = prepared.ciphertext_sha256.as_str();
        let path = self.blob_path(sha256)?;
        if path.is_file() {
            let existing = fs::read(&path)
                .with_context(|| format!("failed to read attachment blob {}", path.display()))?;
            if existing != prepared.ciphertext {
                bail!("attachment blob hash collision for {sha256}");
            }
        } else {
            fs::write(&path, &prepared.ciphertext)
                .with_context(|| format!("failed to write attachment blob {}", path.display()))?;
        }
        let descriptor = BlobDescriptor {
            url: local_blob_url(sha256)?,
            sha256: sha256.to_string(),
            size_bytes: prepared.ciphertext_size,
        };
        let reference = finish_attachment_upload(prepared, descriptor)
            .context("failed to finish encrypted attachment blob reference")?;
        assert_eq!(reference.ciphertext_sha256, prepared.ciphertext_sha256);
        Ok(reference)
    }

    fn blob_path(&self, sha256: &str) -> Result<PathBuf> {
        validate_blob_sha256(sha256)?;
        Ok(self.root.join(sha256))
    }

    fn ciphertext(&self, sha256: &str) -> Result<Vec<u8>> {
        let path = self.blob_path(sha256)?;
        fs::read(&path)
            .with_context(|| format!("failed to read attachment blob {}", path.display()))
    }
}

impl FiniteChatMirror {
    fn open(state_dir: &Path, config: &RuntimeConfig, enabled: bool) -> Result<Option<Self>> {
        if !enabled {
            return Ok(None);
        }
        let mirror = Self {
            db_path: state_dir.join(FINITE_CHAT_MIRROR_DB_FILE),
            seed: finitechat_room_seed(config),
        };
        mirror.seed.validate()?;
        let mut store = mirror.open_store()?;
        mirror.ensure_room_ready(&mut store)?;
        Ok(Some(mirror))
    }

    fn open_store(&self) -> Result<SqliteDeliveryStore> {
        SqliteDeliveryStore::open(&self.db_path).map_err(Into::into)
    }

    fn append_chat_message(&self, message: &ChatMessage) -> Result<MirrorWriteOutcome> {
        assert!(!message.id.is_empty());
        if !message_is_finitechat_mirrorable(message) {
            return Ok(MirrorWriteOutcome::Skipped);
        }
        let sender = finitechat_sender_for_message(&self.seed, message).ok_or_else(|| {
            anyhow!(
                "unsupported finitechat mirror sender {}",
                message.sender_type
            )
        })?;
        let mut store = self.open_store()?;
        self.ensure_room_ready(&mut store)?;
        let room = store
            .room(&self.seed.room_id)?
            .ok_or_else(|| anyhow!("finitechat mirror room missing after setup"))?;
        assert!(room.current_epoch > 0);
        let accepted = finitechat_push_app_event_to_store(
            &mut store,
            FiniteChatAppendInput {
                room_id: self.seed.room_id.clone(),
                mls_group_id: self.seed.mls_group_id.clone(),
                epoch: room.current_epoch,
                sender,
                conversation_id: Some(message.thread_id.clone()),
                idempotency_key: finitechat_protocol_object_id("mirror-msg", &message.id),
                payload: FiniteChatEventPayload::ChatMessage(FiniteChatMessagePayload {
                    message_id: Some(message.id.clone()),
                    sender_type: Some(message.sender_type.clone()),
                    kind: Some(message.kind.clone()),
                    status: Some(message.status.clone()),
                    body: message.body.clone(),
                    metadata: message.metadata.clone(),
                    created_at: Some(message.created_at.clone()),
                    updated_at: Some(message.updated_at.clone()),
                }),
            },
        )?;
        Ok(MirrorWriteOutcome::Appended {
            message_id: accepted.message_id,
            seq: accepted.seq,
        })
    }

    fn ensure_room_ready(&self, store: &mut SqliteDeliveryStore) -> Result<()> {
        finitechat_ensure_local_room_ready(store, &self.seed).map_err(Into::into)
    }
}

fn message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessage> {
    let metadata_json: String = row.get(6)?;
    Ok(ChatMessage {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        sender_type: row.get(2)?,
        kind: row.get(3)?,
        status: row.get(4)?,
        body: row.get(5)?,
        metadata: parse_metadata(&metadata_json),
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn thread_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatThread> {
    Ok(ChatThread {
        id: row.get(0)?,
        project_agent_id: row.get(1)?,
        created_by: row.get(2)?,
        title: row.get(3)?,
        created_at: row.get(4)?,
        last_activity_at: row.get(5)?,
        message_count: row.get::<_, i64>(6)?.max(0) as usize,
    })
}

fn parse_metadata(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!({}))
}

fn parse_attachment_blob_reference(
    attachment_id: &str,
    raw: Option<&str>,
) -> Result<Option<AttachmentBlobReferenceV1>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let reference: AttachmentBlobReferenceV1 = serde_json::from_str(raw)
        .with_context(|| format!("attachment {attachment_id} has invalid blob reference"))?;
    reference
        .validate_limits()
        .with_context(|| format!("attachment {attachment_id} blob reference exceeds limits"))?;
    Ok(Some(reference))
}

fn attachment_blob_references_from_metadata(
    metadata: &Value,
) -> Result<Vec<AttachmentBlobReferenceV1>> {
    let Some(attachments) = metadata.get("attachments").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if attachments.len() > MAX_CHAT_ATTACHMENTS_PER_MESSAGE {
        bail!(
            "chat message contains {} attachments, max is {}",
            attachments.len(),
            MAX_CHAT_ATTACHMENTS_PER_MESSAGE
        );
    }
    let mut references = Vec::new();
    for attachment in attachments {
        let Some(reference_value) = attachment.get(CHAT_ATTACHMENT_BLOB_METADATA_KEY) else {
            continue;
        };
        let reference: AttachmentBlobReferenceV1 = serde_json::from_value(reference_value.clone())
            .context("attachment has invalid finitechat blob reference")?;
        reference
            .validate_limits()
            .context("attachment blob reference exceeds limits")?;
        references.push(reference);
    }
    assert!(references.len() <= attachments.len());
    Ok(references)
}

fn insert_attachment_blob_reference(
    metadata: &mut Value,
    attachment_id: &str,
    reference: &AttachmentBlobReferenceV1,
) -> Result<bool> {
    assert!(!attachment_id.is_empty());
    reference.validate_limits()?;
    let Some(attachments) = metadata
        .get_mut("attachments")
        .and_then(Value::as_array_mut)
    else {
        return Ok(false);
    };
    if attachments.len() > MAX_CHAT_ATTACHMENTS_PER_MESSAGE {
        bail!(
            "chat message contains {} attachments, max is {}",
            attachments.len(),
            MAX_CHAT_ATTACHMENTS_PER_MESSAGE
        );
    }
    let reference_value = serde_json::to_value(reference)?;
    let mut changed = false;
    for attachment in attachments {
        let Some(attachment_object) = attachment.as_object_mut() else {
            bail!("chat attachment metadata must be a JSON object");
        };
        let Some(id) = attachment_object.get("id").and_then(Value::as_str) else {
            continue;
        };
        if id != attachment_id {
            continue;
        }
        attachment_object.insert(
            CHAT_ATTACHMENT_BLOB_METADATA_KEY.to_string(),
            reference_value.clone(),
        );
        changed = true;
    }
    Ok(changed)
}

fn attachment_blob_metadata(name: &str, mime_type: &str) -> Result<AttachmentBlobMetadataV1> {
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

fn local_blob_url(sha256: &str) -> Result<String> {
    validate_blob_sha256(sha256)?;
    Ok(format!("{FINITE_CHAT_LOCAL_BLOB_URL_PREFIX}{sha256}"))
}

fn local_blob_sha_from_url(url: &str) -> Result<String> {
    let Some(sha256) = url.strip_prefix(FINITE_CHAT_LOCAL_BLOB_URL_PREFIX) else {
        bail!("attachment blob URL uses unsupported storage: {url}");
    };
    validate_blob_sha256(sha256)?;
    Ok(sha256.to_string())
}

fn validate_blob_sha256(sha256: &str) -> Result<()> {
    if sha256.len() != 64 {
        bail!(
            "attachment blob sha256 has {} chars, expected 64",
            sha256.len()
        );
    }
    if !sha256
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("attachment blob sha256 must be lowercase hex");
    }
    Ok(())
}

fn sqlite_table_columns(conn: &Connection, table_name: &str) -> Result<BTreeSet<String>> {
    assert!(!table_name.is_empty());
    assert!(
        table_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    );
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>("name"))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    Ok(rows)
}

fn finitechat_mirror_enabled() -> bool {
    env::var(FINITE_CHAT_MIRROR_ENABLED_ENV)
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "on"))
}

fn finitechat_room_seed(config: &RuntimeConfig) -> FiniteChatRoomSeed {
    assert!(!config.user_id.is_empty());
    assert!(!config.machine_id.is_empty());
    assert!(!config.agent_id.is_empty());
    let room_key = format!("{}:{}", config.machine_id, config.agent_id);
    FiniteChatRoomSeed {
        room_id: finitechat_protocol_object_id("room", &room_key),
        mls_group_id: finitechat_protocol_object_id("mls", &room_key),
        user_device: finitechat_device(
            env::var("FINITE_CHAT_USER_ACCOUNT_ID").unwrap_or_else(|_| config.user_id.clone()),
            env::var("FINITE_CHAT_USER_DEVICE_ID").unwrap_or_else(|_| "dashboard".to_string()),
        ),
        runtime_device: finitechat_device(
            env::var("FINITE_CHAT_RUNTIME_ACCOUNT_ID")
                .unwrap_or_else(|_| format!("runtime:{}", config.agent_id)),
            env::var("FINITE_CHAT_RUNTIME_DEVICE_ID").unwrap_or_else(|_| "finitec".to_string()),
        ),
    }
}

fn finitechat_sender_for_message(
    seed: &FiniteChatRoomSeed,
    message: &ChatMessage,
) -> Option<DeviceRef> {
    match message.sender_type.as_str() {
        "user" => Some(seed.user_device.clone()),
        "agent" => Some(seed.runtime_device.clone()),
        _ => None,
    }
}

fn message_is_finitechat_mirrorable(message: &ChatMessage) -> bool {
    message.kind == "message"
        && (!message.body.trim().is_empty()
            || message
                .metadata
                .get("attachments")
                .and_then(Value::as_array)
                .is_some_and(|attachments| !attachments.is_empty()))
}

fn chat_messages_same_logical_user_request(left: &ChatMessage, right: &ChatMessage) -> bool {
    left.id == right.id
        && left.thread_id == right.thread_id
        && left.sender_type == right.sender_type
        && left.kind == right.kind
        && left.status == right.status
        && left.body == right.body
}

fn u64_to_sqlite_seq(seq: u64, label: &str) -> Result<i64> {
    if seq > MAX_RELAY_CHAT_INBOX_SEQ {
        bail!("{label} exceeds sqlite integer range");
    }
    Ok(seq as i64)
}

fn sqlite_seq_to_u64(seq: i64, label: &str) -> Result<u64> {
    if seq < 0 {
        bail!("{label} is negative");
    }
    Ok(seq as u64)
}

fn validate_chat_log_sync_cursor(cursor: &ChatLogSyncCursor) -> Result<()> {
    if cursor.updated_at.trim().is_empty() {
        bail!("chat log sync cursor updated_at is required");
    }
    OffsetDateTime::parse(&cursor.updated_at, &Rfc3339)
        .context("chat log sync cursor updated_at is invalid")?;
    if cursor.message_id.trim().is_empty() {
        bail!("chat log sync cursor message_id is required");
    }
    if cursor.message_id.len() > 128 {
        bail!("chat log sync cursor message_id is too long");
    }
    Ok(())
}

fn chat_log_sync_cursor_at_or_after(left: &ChatLogSyncCursor, right: &ChatLogSyncCursor) -> bool {
    left.updated_at > right.updated_at
        || (left.updated_at == right.updated_at && left.message_id >= right.message_id)
}

pub fn resolve_state_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(path) = env::var_os("FINITE_CHAT_STATE_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(home) = env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".finite/chat"));
    }
    Ok(PathBuf::from(".state/finite-chat"))
}

fn load_hermes_slash_commands() -> Result<Vec<ChatSlashCommand>> {
    let script = r#"
import json
from hermes_cli.commands import COMMAND_REGISTRY

never_expose = {"sethome", "restart", "update", "commands"}
commands = []
for command in COMMAND_REGISTRY:
    name = getattr(command, "name", "")
    if not name or name in never_expose:
        continue
    if bool(getattr(command, "gateway_only", False)):
        continue
    commands.append({
        "name": name,
        "description": getattr(command, "description", ""),
        "category": getattr(command, "category", ""),
        "aliases": list(getattr(command, "aliases", ()) or ()),
        "args_hint": getattr(command, "args_hint", ""),
        "subcommands": list(getattr(command, "subcommands", ()) or ()),
        "cli_only": bool(getattr(command, "cli_only", False)),
        "gateway_only": bool(getattr(command, "gateway_only", False)),
    })
print(json.dumps(commands))
"#;
    let output = hermes_python_command(script)
        .context("failed to create Hermes slash command loader")?
        .output()
        .context("failed to load Hermes slash commands")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Hermes slash command registry unavailable: {}",
            stderr.trim()
        );
    }

    serde_json::from_slice(&output.stdout).context("failed to parse Hermes slash command registry")
}

fn hermes_python_command(script: &str) -> Result<Command> {
    for env_name in ["FINITE_HERMES_PYTHON", "HERMES_PYTHON"] {
        if let Ok(python) = env::var(env_name) {
            if python.trim().is_empty() {
                continue;
            }
            let mut command = Command::new(python);
            command.args(["-c", script]);
            return Ok(command);
        }
    }

    let repo = env::var("FINITE_HERMES_REPO")
        .map(PathBuf::from)
        .ok()
        .or_else(|| sibling_hermes_repo().ok())
        .filter(|path| path.join("hermes_cli").is_dir());

    if let Some(repo) = repo {
        for relative in [".venv/bin/python", "venv/bin/python"] {
            let python = repo.join(relative);
            if python.is_file() {
                let mut command = Command::new(python);
                command.current_dir(&repo).args(["-c", script]);
                return Ok(command);
            }
        }
        if command_exists("uv") {
            let mut command = Command::new("uv");
            command
                .current_dir(&repo)
                .args(["run", "python", "-c", script]);
            return Ok(command);
        }
    }

    for program in ["python3", "python"] {
        if command_exists(program) {
            let mut command = Command::new(program);
            command.args(["-c", script]);
            return Ok(command);
        }
    }

    bail!(
        "Python with hermes_cli.commands import support was not found. Set FINITE_HERMES_PYTHON or FINITE_HERMES_REPO."
    )
}

fn sibling_hermes_repo() -> Result<PathBuf> {
    let cwd = env::current_dir()?;
    Ok(cwd
        .parent()
        .map(|parent| parent.join("hermes-agent"))
        .unwrap_or_else(|| PathBuf::from("../hermes-agent")))
}

fn command_exists(program: &str) -> bool {
    Command::new("sh")
        .args(["-lc", &format!("command -v {program}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn runtime_config() -> RuntimeConfig {
    let machine_id = env::var("FINITE_MACHINE_ID")
        .or_else(|_| env::var("FC_WORKLOAD_ID"))
        .or_else(|_| env::var("MACHINE_ID"))
        .unwrap_or_else(|_| "local-machine".to_string());
    let agent_id = env::var("FINITE_AGENT_ID").unwrap_or_else(|_| format!("agent_{machine_id}"));
    let user_email =
        env::var("FINITE_USER_EMAIL").unwrap_or_else(|_| "local@finite.computer".to_string());
    let user_name = env::var("FINITE_USER_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| display_name_from_email(&user_email));
    RuntimeConfig {
        user_id: env::var("FINITE_USER_ID").unwrap_or_else(|_| "user_local".to_string()),
        user_email,
        user_name,
        machine_id: machine_id.clone(),
        agent_id,
        agent_name: env::var("FINITE_AGENT_NAME")
            .unwrap_or_else(|_| display_name_from_id(&machine_id)),
        agent_purpose: env::var("FINITE_AGENT_PURPOSE")
            .unwrap_or_else(|_| "Primary Hermes agent for this machine.".to_string()),
        hermes_profile_name: env::var("FINITE_HERMES_PROFILE").unwrap_or(machine_id),
        hermes_api_port: env::var("FINITE_HERMES_API_PORT")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(8642),
        workspace_ref: env::var("FINITE_AGENT_WORKSPACE")
            .or_else(|_| env::var("PWD"))
            .unwrap_or_else(|_| "/home/node".to_string()),
    }
}

fn now_iso() -> Result<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

fn provisional_thread_title(body: &str) -> Option<String> {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }

    if collapsed.chars().count() <= MAX_PROVISIONAL_THREAD_TITLE_CHARS {
        return Some(collapsed);
    }

    let mut title = collapsed
        .chars()
        .take(MAX_PROVISIONAL_THREAD_TITLE_CHARS.saturating_sub(3))
        .collect::<String>();
    title = title.trim_end().to_string();
    Some(format!("{title}..."))
}

fn thread_title_is_auto(title: &str) -> bool {
    let trimmed = title.trim();
    trimmed.is_empty() || trimmed == DEFAULT_THREAD_TITLE || trimmed == "Untitled"
}

fn normalized_thread_title(title: Option<&str>) -> String {
    title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_THREAD_TITLE)
        .to_string()
}

fn validate_thread_id(thread_id: &str) -> Result<&str> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        bail!("thread id is required");
    }
    if thread_id.len() > 128 {
        bail!("thread id is too long");
    }
    if !thread_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        bail!("thread id contains invalid characters");
    }
    Ok(thread_id)
}

fn id_with_prefix(prefix: &str) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp_nanos();
    let random = rand::random::<u64>();
    format!("{prefix}_{now:x}{random:x}")
}

fn display_name_from_email(email: &str) -> String {
    email
        .split('@')
        .next()
        .unwrap_or("Account")
        .split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_name_from_id(id: &str) -> String {
    id.split(['-', '_'])
        .filter(|part| !part.is_empty() && *part != "finite")
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sanitize_file_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

fn chat_max_attachment_bytes() -> usize {
    let configured = env::var(MAX_CHAT_ATTACHMENT_BYTES_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_CHAT_ATTACHMENT_BYTES);
    configured.min(DEFAULT_MAX_CHAT_ATTACHMENT_BYTES)
}

fn validate_attachment_size(name: &str, data_base64: &str, max_bytes: usize) -> Result<()> {
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

fn attachment_kind(mime_type: &str) -> &str {
    if mime_type.starts_with("image/") {
        "image"
    } else if mime_type.starts_with("audio/") {
        "audio"
    } else if mime_type.starts_with("video/") {
        "video"
    } else {
        "file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::SendChatMessageRequest;
    use crate::finite_chat::FiniteChatEventPayload;
    use finitechat_proto::{DecryptedApplicationEventV1, DurableAppEventKind, LogEntryKind};
    use finitechat_store::SqliteDeliveryStore;
    use tempfile::tempdir;

    #[test]
    fn attachment_limit_rejects_oversized_base64_before_decode() {
        let encoded = BASE64_STANDARD.encode(vec![7_u8; 16]);
        let err = validate_attachment_size("too-large.bin", &encoded, 8)
            .expect_err("oversized attachment should be rejected")
            .to_string();
        assert!(err.contains("too-large.bin"));
        assert!(err.contains("chat attachment limit"));
    }

    #[test]
    fn attachment_blob_rejects_empty_files() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();

        let err = runtime
            .send_user_message(ChatSendMessageRequest {
                thread_id,
                message: SendChatMessageRequest {
                    body: "empty".to_string(),
                    client_message_id: Some("msg_empty_attachment".to_string()),
                    attachments: vec![ChatMessageAttachmentInput {
                        name: "empty.txt".to_string(),
                        mime_type: "text/plain".to_string(),
                        data_base64: BASE64_STANDARD.encode(Vec::<u8>::new()),
                    }],
                },
            })
            .expect_err("empty attachment should fail")
            .to_string();
        assert!(err.contains("empty.txt"));
        assert!(err.contains("empty"));
    }

    #[test]
    fn create_thread_accepts_explicit_id_idempotently() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");

        let first = runtime
            .create_thread(CreateChatThreadRequest {
                thread_id: Some("topic-1".to_string()),
                title: Some("Deploys".to_string()),
            })
            .expect("explicit create");
        let second = runtime
            .create_thread(CreateChatThreadRequest {
                thread_id: Some("topic-1".to_string()),
                title: Some("Deploys".to_string()),
            })
            .expect("explicit create retry");

        assert_eq!(first, second);
        assert_eq!(first.id, "topic-1");
        let conflict = runtime
            .create_thread(CreateChatThreadRequest {
                thread_id: Some("topic-1".to_string()),
                title: Some("Different".to_string()),
            })
            .expect_err("same explicit id with different title must fail")
            .to_string();
        assert!(conflict.contains("different title"));
    }

    #[test]
    fn create_thread_explicit_id_replay_tolerates_default_title_after_auto_title() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");

        let thread = runtime
            .create_thread(CreateChatThreadRequest {
                thread_id: Some("topic-auto".to_string()),
                title: Some(DEFAULT_THREAD_TITLE.to_string()),
            })
            .expect("explicit create");
        runtime
            .send_user_message(ChatSendMessageRequest {
                thread_id: thread.id.clone(),
                message: SendChatMessageRequest {
                    body: "please rename me from first message".to_string(),
                    client_message_id: Some("msg-auto-title".to_string()),
                    attachments: vec![],
                },
            })
            .expect("send first user message");

        let replay = runtime
            .create_thread(CreateChatThreadRequest {
                thread_id: Some("topic-auto".to_string()),
                title: Some(DEFAULT_THREAD_TITLE.to_string()),
            })
            .expect("default title create replay");
        assert_eq!(replay.id, "topic-auto");
        assert_ne!(replay.title, DEFAULT_THREAD_TITLE);
    }

    #[test]
    fn relay_chat_inbox_projects_user_message_once_and_persists_cursor() {
        let dir = tempdir().expect("tempdir");
        let state_dir = dir.path().join("chat");
        let runtime = ChatRuntime::open(Some(state_dir.clone())).expect("chat runtime");
        let agent_id = runtime.health().agent_id;
        let conversation = relay_inbox_thread("topic-1", &agent_id, "Deploys");
        runtime
            .apply_relay_chat_inbox_event(&ChatInboxEvent {
                seq: 1,
                kind: "conversation.create".to_string(),
                project_agent_id: agent_id.clone(),
                conversation_id: "topic-1".to_string(),
                conversation: Some(conversation),
                message: None,
            })
            .expect("conversation create projects");
        runtime
            .apply_relay_chat_inbox_event(&ChatInboxEvent {
                seq: 2,
                kind: "chat.message".to_string(),
                project_agent_id: agent_id.clone(),
                conversation_id: "topic-1".to_string(),
                conversation: None,
                message: Some(relay_inbox_message("msg-1", "topic-1", "hello from relay")),
            })
            .expect("message projects");
        runtime
            .apply_relay_chat_inbox_event(&ChatInboxEvent {
                seq: 2,
                kind: "chat.message".to_string(),
                project_agent_id: agent_id.clone(),
                conversation_id: "topic-1".to_string(),
                conversation: None,
                message: Some(relay_inbox_message("msg-1", "topic-1", "hello from relay")),
            })
            .expect("message replay is idempotent");

        let gateway = runtime
            .gateway_poll(&GatewayPollRequest {
                agent_id: agent_id.clone(),
                limit: Some(10),
                timeout_secs: None,
            })
            .expect("gateway poll");
        assert_eq!(gateway.events.len(), 1);
        assert_eq!(gateway.events[0].message_id, "msg-1");
        assert_eq!(gateway.events[0].body, "hello from relay");

        let reopened = ChatRuntime::open(Some(state_dir)).expect("reopen chat runtime");
        assert_eq!(
            reopened
                .relay_chat_inbox_cursor(&agent_id)
                .expect("cursor survives restart"),
            2
        );
    }

    #[test]
    fn relay_chat_inbox_accepts_metadata_enrichment_and_timestamp_replay() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let agent_id = runtime.health().agent_id;
        let conversation = relay_inbox_thread("topic-blob", &agent_id, "Blob test");
        runtime
            .apply_relay_chat_inbox_event(&ChatInboxEvent {
                seq: 1,
                kind: "conversation.create".to_string(),
                project_agent_id: agent_id.clone(),
                conversation_id: "topic-blob".to_string(),
                conversation: Some(conversation),
                message: None,
            })
            .expect("conversation create projects");

        let mut local = relay_inbox_message("msg-blob", "topic-blob", "inspect this attachment");
        local.metadata = json!({
            "attachments": [{
                "id": "att-blob",
                "finitechat_blob": {
                    "scheme": "finitechat.attachment.blob.v1",
                    "url": "finitechat+local-blob://sha256/abc"
                }
            }]
        });
        local.created_at = "2026-05-22T00:00:03Z".to_string();
        local.updated_at = "2026-05-22T00:00:03Z".to_string();
        runtime
            .store
            .insert_message(&local)
            .expect("seed richer local message");

        runtime
            .apply_relay_chat_inbox_event(&ChatInboxEvent {
                seq: 2,
                kind: "chat.message".to_string(),
                project_agent_id: agent_id.clone(),
                conversation_id: "topic-blob".to_string(),
                conversation: None,
                message: Some(relay_inbox_message(
                    "msg-blob",
                    "topic-blob",
                    "inspect this attachment",
                )),
            })
            .expect("older metadata replay must not block the cursor");

        let gateway = runtime
            .gateway_poll(&GatewayPollRequest {
                agent_id: agent_id.clone(),
                limit: Some(10),
                timeout_secs: None,
            })
            .expect("gateway poll");
        assert_eq!(gateway.events.len(), 1);
        assert_eq!(gateway.events[0].message_id, "msg-blob");
        assert_eq!(
            gateway.events[0].metadata["attachments"][0]["id"],
            json!("att-blob")
        );
        assert!(
            gateway.events[0].metadata["attachments"][0]
                .get(CHAT_ATTACHMENT_BLOB_METADATA_KEY)
                .is_none()
        );
        assert_eq!(runtime.relay_chat_inbox_cursor(&agent_id).unwrap(), 2);
        assert_eq!(
            runtime
                .store
                .message("msg-blob")
                .expect("read message")
                .expect("message exists")
                .metadata,
            local.metadata
        );
    }

    #[test]
    fn relay_chat_inbox_rejects_same_message_id_with_different_body() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let agent_id = runtime.health().agent_id;
        let conversation = relay_inbox_thread("topic-conflict", &agent_id, "Conflict test");
        runtime
            .apply_relay_chat_inbox_event(&ChatInboxEvent {
                seq: 1,
                kind: "conversation.create".to_string(),
                project_agent_id: agent_id.clone(),
                conversation_id: "topic-conflict".to_string(),
                conversation: Some(conversation),
                message: None,
            })
            .expect("conversation create projects");
        runtime
            .store
            .insert_message(&relay_inbox_message(
                "msg-conflict",
                "topic-conflict",
                "original body",
            ))
            .expect("seed conflicting local message");

        let err = runtime
            .apply_relay_chat_inbox_event(&ChatInboxEvent {
                seq: 2,
                kind: "chat.message".to_string(),
                project_agent_id: agent_id.clone(),
                conversation_id: "topic-conflict".to_string(),
                conversation: None,
                message: Some(relay_inbox_message(
                    "msg-conflict",
                    "topic-conflict",
                    "changed body",
                )),
            })
            .expect_err("different body must remain a hard conflict")
            .to_string();
        assert!(err.contains("conflicts with local message"));
        assert_eq!(runtime.relay_chat_inbox_cursor(&agent_id).unwrap(), 1);
    }

    #[test]
    fn relay_chat_inbox_rejects_message_before_conversation() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let agent_id = runtime.health().agent_id;
        let err = runtime
            .apply_relay_chat_inbox_event(&ChatInboxEvent {
                seq: 1,
                kind: "chat.message".to_string(),
                project_agent_id: agent_id.clone(),
                conversation_id: "topic-missing".to_string(),
                conversation: None,
                message: Some(relay_inbox_message(
                    "msg-missing-thread",
                    "topic-missing",
                    "should fail",
                )),
            })
            .expect_err("message before conversation must fail")
            .to_string();
        assert!(err.contains("conversation topic-missing not found"));
        assert_eq!(runtime.relay_chat_inbox_cursor(&agent_id).unwrap(), 0);
    }

    #[test]
    fn gateway_poll_includes_local_attachment_path_but_transcript_does_not() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let health = runtime.health();
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();

        let message = runtime
            .send_user_message(ChatSendMessageRequest {
                thread_id,
                message: SendChatMessageRequest {
                    body: "please inspect this image".to_string(),
                    client_message_id: Some("msg_test_attachment".to_string()),
                    attachments: vec![ChatMessageAttachmentInput {
                        name: "screenshot.png".to_string(),
                        mime_type: "image/png".to_string(),
                        data_base64: BASE64_STANDARD.encode([0_u8, 1, 2, 3]),
                    }],
                },
            })
            .expect("send message");

        let transcript_attachment = message
            .metadata
            .get("attachments")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_object)
            .expect("transcript attachment metadata");
        assert!(transcript_attachment.get("url").is_some());
        assert!(transcript_attachment.get("path").is_none());
        assert!(
            transcript_attachment
                .get(CHAT_ATTACHMENT_BLOB_METADATA_KEY)
                .is_some()
        );

        let events = runtime
            .gateway_poll(&GatewayPollRequest {
                agent_id: health.agent_id,
                limit: Some(10),
                timeout_secs: None,
            })
            .expect("gateway poll")
            .events;
        let gateway_attachment = events[0]
            .metadata
            .get("attachments")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_object)
            .expect("gateway attachment metadata");
        assert!(
            gateway_attachment
                .get(CHAT_ATTACHMENT_BLOB_METADATA_KEY)
                .is_none()
        );
        let path = gateway_attachment
            .get("path")
            .and_then(Value::as_str)
            .expect("gateway path");
        assert!(PathBuf::from(path).is_file());
    }

    #[test]
    fn attachment_download_survives_plaintext_file_removal_when_blob_exists() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();
        let original = [7_u8, 8, 9, 10];

        let message = runtime
            .send_user_message(ChatSendMessageRequest {
                thread_id,
                message: SendChatMessageRequest {
                    body: "persist this".to_string(),
                    client_message_id: Some("msg_blob_persist".to_string()),
                    attachments: vec![ChatMessageAttachmentInput {
                        name: "payload.bin".to_string(),
                        mime_type: "application/octet-stream".to_string(),
                        data_base64: BASE64_STANDARD.encode(original),
                    }],
                },
            })
            .expect("send message");
        let attachment_id = message.metadata["attachments"][0]["id"]
            .as_str()
            .expect("attachment id");
        let attachment = runtime
            .store
            .attachment_record(attachment_id)
            .expect("attachment record");
        assert!(attachment.blob_reference.is_some());
        fs::remove_file(&attachment.path).expect("remove plaintext compatibility file");

        let data = runtime.attachment(attachment_id).expect("attachment data");
        assert_eq!(data.name, "payload.bin");
        assert_eq!(data.mime_type, "application/octet-stream");
        assert_eq!(
            BASE64_STANDARD.decode(data.data_base64).expect("base64"),
            original
        );
    }

    #[test]
    fn attachment_blob_uploads_are_read_from_blob_store_not_plaintext_file() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();
        let original = [42_u8, 43, 44, 45];

        let message = runtime
            .send_user_message(ChatSendMessageRequest {
                thread_id,
                message: SendChatMessageRequest {
                    body: "sync this blob".to_string(),
                    client_message_id: Some("msg_blob_sync".to_string()),
                    attachments: vec![ChatMessageAttachmentInput {
                        name: "sync.bin".to_string(),
                        mime_type: "application/octet-stream".to_string(),
                        data_base64: BASE64_STANDARD.encode(original),
                    }],
                },
            })
            .expect("send message");
        let attachment_id = message.metadata["attachments"][0]["id"]
            .as_str()
            .expect("attachment id");
        let attachment = runtime
            .store
            .attachment_record(attachment_id)
            .expect("attachment record");
        fs::remove_file(&attachment.path).expect("remove plaintext compatibility file");

        let blobs = runtime
            .attachment_blobs_for_messages(&[message])
            .expect("attachment blobs");
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].sha256.len(), 64);
        assert!(!blobs[0].ciphertext.is_empty());
    }

    #[test]
    fn legacy_attachment_rows_backfill_blob_references_on_open() {
        let dir = tempdir().expect("tempdir");
        let state_dir = dir.path().join("chat");
        let runtime = ChatRuntime::open(Some(state_dir.clone())).expect("chat runtime");
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();
        let created_at = "2026-05-22T00:00:00Z";
        let attachment_id = "att_legacy_blob_backfill";
        let message_id = "msg_legacy_blob_backfill";
        let plaintext = b"legacy attachment bytes";
        let legacy_path = runtime.store.attachments_dir.join("legacy-backfill.bin");
        fs::write(&legacy_path, plaintext).expect("legacy plaintext file");
        runtime
            .store
            .conn
            .execute(
                r#"
                INSERT INTO attachments
                  (id, name, mime_type, size_bytes, path, blob_reference_json, created_at)
                VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)
                "#,
                params![
                    attachment_id,
                    "legacy-backfill.bin",
                    "application/octet-stream",
                    plaintext.len() as i64,
                    legacy_path.display().to_string(),
                    created_at,
                ],
            )
            .expect("insert legacy attachment");
        let legacy_metadata = json!({
            "attachments": [{
                "id": attachment_id,
                "type": "file",
                "name": "legacy-backfill.bin",
                "mime_type": "application/octet-stream",
                "size_bytes": plaintext.len(),
                "url": format!("/attachments/{attachment_id}")
            }]
        });
        runtime
            .store
            .conn
            .execute(
                r#"
                INSERT INTO messages
                  (id, thread_id, sender_type, kind, status, body, metadata_json, created_at, updated_at)
                VALUES (?1, ?2, 'user', 'message', 'sent', 'legacy attachment', ?3, ?4, ?4)
                "#,
                params![
                    message_id,
                    thread_id,
                    serde_json::to_string(&legacy_metadata).expect("metadata json"),
                    created_at,
                ],
            )
            .expect("insert legacy message");
        drop(runtime);

        let runtime = ChatRuntime::open(Some(state_dir)).expect("reopen chat runtime");
        let attachment = runtime
            .store
            .attachment_record(attachment_id)
            .expect("backfilled attachment record");
        assert!(attachment.blob_reference.is_some());
        fs::remove_file(&legacy_path).expect("remove plaintext compatibility file");
        let data = runtime.attachment(attachment_id).expect("blob-backed data");
        assert_eq!(
            BASE64_STANDARD.decode(data.data_base64).expect("base64"),
            plaintext
        );

        let message = runtime
            .messages_for_thread(&thread_id)
            .expect("messages")
            .into_iter()
            .find(|message| message.id == message_id)
            .expect("legacy message");
        assert!(
            message.metadata["attachments"][0]
                .get(CHAT_ATTACHMENT_BLOB_METADATA_KEY)
                .is_some()
        );
        let blobs = runtime
            .attachment_blobs_for_messages(&[message])
            .expect("attachment blobs");
        assert_eq!(blobs.len(), 1);
        assert!(!blobs[0].ciphertext.is_empty());
    }

    #[test]
    fn gateway_send_stores_agent_media_paths_as_transcript_attachments() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let health = runtime.health();
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();
        let image_path = dir.path().join("generated.png");
        fs::write(&image_path, [137_u8, 80, 78, 71]).expect("write image");

        let message = runtime
            .gateway_send(GatewayOutboundMessageRequest {
                agent_id: health.agent_id,
                thread_id,
                body: "".to_string(),
                kind: Some("media".to_string()),
                status: Some("complete".to_string()),
                metadata: json!({
                    "attachments": [{
                        "type": "image",
                        "path": image_path.display().to_string(),
                        "name": "generated.png",
                        "mime_type": "image/png"
                    }]
                }),
                reply_to_message_id: None,
                message_id: None,
            })
            .expect("gateway send");

        let attachment = message
            .metadata
            .get("attachments")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_object)
            .expect("transcript attachment metadata");
        assert_eq!(
            attachment.get("name").and_then(Value::as_str),
            Some("generated.png")
        );
        assert_eq!(
            attachment.get("mime_type").and_then(Value::as_str),
            Some("image/png")
        );
        assert_eq!(
            attachment.get("type").and_then(Value::as_str),
            Some("image")
        );
        assert!(attachment.get("path").is_none());
        assert!(attachment.get(CHAT_ATTACHMENT_BLOB_METADATA_KEY).is_some());

        let attachment_id = attachment
            .get("id")
            .and_then(Value::as_str)
            .expect("attachment id");
        let data = runtime.attachment(attachment_id).expect("attachment data");
        assert_eq!(data.name, "generated.png");
        assert_eq!(data.mime_type, "image/png");
        assert_eq!(
            BASE64_STANDARD.decode(data.data_base64).expect("base64"),
            [137_u8, 80, 78, 71]
        );
    }

    #[test]
    fn recover_interrupted_agent_messages_closes_stale_running_rows() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let health = runtime.health();
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();

        let status = runtime
            .gateway_send(GatewayOutboundMessageRequest {
                agent_id: health.agent_id.clone(),
                thread_id: thread_id.clone(),
                body: "Hermes is working".to_string(),
                kind: Some("status".to_string()),
                status: Some("running".to_string()),
                metadata: json!({}),
                reply_to_message_id: None,
                message_id: Some("msg_running_status".to_string()),
            })
            .expect("gateway send status");
        let tool = runtime
            .gateway_send(GatewayOutboundMessageRequest {
                agent_id: health.agent_id,
                thread_id: thread_id.clone(),
                body: "browser_navigate".to_string(),
                kind: Some("tool".to_string()),
                status: Some("running".to_string()),
                metadata: json!({}),
                reply_to_message_id: None,
                message_id: Some("msg_running_tool".to_string()),
            })
            .expect("gateway send tool");

        assert_eq!(
            runtime
                .recover_interrupted_agent_messages()
                .expect("recover interrupted"),
            2
        );

        let messages = runtime
            .messages_for_thread(&thread_id)
            .expect("messages for thread");
        let recovered_status = messages
            .iter()
            .find(|message| message.id == status.id)
            .expect("status message");
        assert_eq!(recovered_status.status, "error");
        assert!(recovered_status.body.contains("gateway restarted"));

        let recovered_tool = messages
            .iter()
            .find(|message| message.id == tool.id)
            .expect("tool message");
        assert_eq!(recovered_tool.status, "error");
        assert_eq!(recovered_tool.body, "browser_navigate");
    }

    #[test]
    fn bootstrap_and_message_list_return_bounded_pages() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();

        for index in 0..95 {
            runtime
                .send_user_message(ChatSendMessageRequest {
                    thread_id: thread_id.clone(),
                    message: SendChatMessageRequest {
                        body: format!("message {index:02}"),
                        client_message_id: Some(format!("msg_page_test_{index:02}")),
                        attachments: vec![],
                    },
                })
                .expect("send message");
        }

        let bootstrap = runtime.bootstrap().expect("bootstrap");
        let thread = bootstrap
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .expect("thread");
        assert_eq!(thread.message_count, 95);
        assert_eq!(bootstrap.messages.len(), DEFAULT_BOOTSTRAP_MESSAGE_LIMIT);
        assert_eq!(bootstrap.messages[0].body, "message 15");
        assert_eq!(bootstrap.messages[79].body, "message 94");

        let latest = runtime
            .message_page_for_thread(&ChatListMessagesRequest {
                thread_id: thread_id.clone(),
                limit: Some(20),
                before: None,
            })
            .expect("latest page");
        assert!(latest.has_more);
        assert_eq!(latest.messages.len(), 20);
        assert_eq!(latest.messages[0].body, "message 75");
        assert_eq!(latest.messages[19].body, "message 94");

        let older = runtime
            .message_page_for_thread(&ChatListMessagesRequest {
                thread_id,
                limit: Some(20),
                before: latest.next_before,
            })
            .expect("older page");
        assert!(older.has_more);
        assert_eq!(older.messages[0].body, "message 55");
        assert_eq!(older.messages[19].body, "message 74");
    }

    #[test]
    fn bootstrap_includes_recent_messages_for_each_thread() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let first_thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();
        let second_thread_id = runtime
            .create_thread(CreateChatThreadRequest {
                thread_id: None,
                title: Some("Second topic".to_string()),
            })
            .expect("create second thread")
            .id;

        runtime
            .send_user_message(ChatSendMessageRequest {
                thread_id: first_thread_id.clone(),
                message: SendChatMessageRequest {
                    body: "first thread message".to_string(),
                    client_message_id: Some("msg_first_thread".to_string()),
                    attachments: vec![],
                },
            })
            .expect("send first thread message");
        runtime
            .send_user_message(ChatSendMessageRequest {
                thread_id: second_thread_id.clone(),
                message: SendChatMessageRequest {
                    body: "second thread message".to_string(),
                    client_message_id: Some("msg_second_thread".to_string()),
                    attachments: vec![],
                },
            })
            .expect("send second thread message");

        let bootstrap = runtime.bootstrap().expect("bootstrap");
        assert!(bootstrap.messages.iter().any(
            |message| message.thread_id == first_thread_id && message.id == "msg_first_thread"
        ));
        assert!(
            bootstrap
                .messages
                .iter()
                .any(|message| message.thread_id == second_thread_id
                    && message.id == "msg_second_thread")
        );
    }

    #[test]
    fn chat_log_messages_after_pages_by_stable_update_cursor() {
        let dir = tempdir().expect("tempdir");
        let runtime = ChatRuntime::open(Some(dir.path().join("chat"))).expect("chat runtime");
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();

        for index in 0..3 {
            runtime
                .send_user_message(ChatSendMessageRequest {
                    thread_id: thread_id.clone(),
                    message: SendChatMessageRequest {
                        body: format!("sync message {index}"),
                        client_message_id: Some(format!("msg_sync_cursor_{index}")),
                        attachments: vec![],
                    },
                })
                .expect("send message");
        }

        let first = runtime
            .chat_log_messages_after(None, None, 2)
            .expect("first sync page");
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].id, "msg_sync_cursor_0");
        assert_eq!(first[1].id, "msg_sync_cursor_1");

        let second = runtime
            .chat_log_messages_after(Some(&first[1].updated_at), Some(&first[1].id), 2)
            .expect("second sync page");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, "msg_sync_cursor_2");
    }

    #[test]
    fn relay_chat_log_sync_cursor_persists_and_does_not_regress() {
        let dir = tempdir().expect("tempdir");
        let state_dir = dir.path().join("chat");
        let runtime = ChatRuntime::open(Some(state_dir.clone())).expect("chat runtime");
        let agent_id = runtime.health().agent_id;
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();

        for index in 0..2 {
            runtime
                .send_user_message(ChatSendMessageRequest {
                    thread_id: thread_id.clone(),
                    message: SendChatMessageRequest {
                        body: format!("relay log cursor {index}"),
                        client_message_id: Some(format!("msg_relay_log_cursor_{index}")),
                        attachments: vec![],
                    },
                })
                .expect("send message");
        }

        let messages = runtime
            .chat_log_messages_after(None, None, 10)
            .expect("sync messages");
        assert_eq!(messages.len(), 2);
        let first = ChatLogSyncCursor {
            updated_at: messages[0].updated_at.clone(),
            message_id: messages[0].id.clone(),
        };
        let second = ChatLogSyncCursor {
            updated_at: messages[1].updated_at.clone(),
            message_id: messages[1].id.clone(),
        };
        assert!(chat_log_sync_cursor_at_or_after(&second, &first));

        runtime
            .advance_relay_chat_log_sync_cursor(&agent_id, &second)
            .expect("advance cursor");
        runtime
            .advance_relay_chat_log_sync_cursor(&agent_id, &first)
            .expect("older cursor does not regress");
        let reopened = ChatRuntime::open(Some(state_dir)).expect("reopen chat runtime");
        assert_eq!(
            reopened
                .relay_chat_log_sync_cursor(&agent_id)
                .expect("cursor read"),
            Some(second.clone())
        );
        let remaining = reopened
            .chat_log_messages_after(Some(&second.updated_at), Some(&second.message_id), 10)
            .expect("remaining messages after cursor");
        assert!(remaining.is_empty());
    }

    #[test]
    fn finitechat_mirror_is_disabled_by_default() {
        let dir = tempdir().expect("tempdir");
        let state_dir = dir.path().join("chat");
        let runtime = ChatRuntime::open(Some(state_dir.clone())).expect("chat runtime");
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();

        runtime
            .send_user_message(ChatSendMessageRequest {
                thread_id,
                message: SendChatMessageRequest {
                    body: "current chat still works".to_string(),
                    client_message_id: Some("msg_no_mirror".to_string()),
                    attachments: vec![],
                },
            })
            .expect("send message");

        assert!(!state_dir.join(FINITE_CHAT_MIRROR_DB_FILE).exists());
        assert_eq!(
            finitechat_mirror_failure_count(&runtime).expect("failure count"),
            0
        );
    }

    #[test]
    fn finitechat_mirror_persists_user_and_agent_messages_without_changing_chat_runtime() {
        let dir = tempdir().expect("tempdir");
        let state_dir = dir.path().join("chat");
        let runtime = ChatRuntime::open_with_finitechat_mirror(Some(state_dir.clone()))
            .expect("chat runtime");
        let health = runtime.health();
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();

        let user = runtime
            .send_user_message(ChatSendMessageRequest {
                thread_id: thread_id.clone(),
                message: SendChatMessageRequest {
                    body: "mirror this user message".to_string(),
                    client_message_id: Some("msg_mirror_user".to_string()),
                    attachments: vec![],
                },
            })
            .expect("send user message");
        let agent = runtime
            .gateway_send(GatewayOutboundMessageRequest {
                agent_id: health.agent_id.clone(),
                thread_id: thread_id.clone(),
                body: "mirror this agent reply".to_string(),
                kind: Some("message".to_string()),
                status: Some("complete".to_string()),
                metadata: json!({ "reply_to_message_id": user.id }),
                reply_to_message_id: None,
                message_id: Some("msg_mirror_agent".to_string()),
            })
            .expect("gateway send");

        let messages = runtime
            .messages_for_thread(&thread_id)
            .expect("messages for thread");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].id, "msg_mirror_user");
        assert_eq!(messages[1].id, agent.id);

        let mirror_messages =
            mirrored_chat_messages(&state_dir, &runtime).expect("mirrored chat messages");
        assert_eq!(mirror_messages.len(), 2);
        assert_eq!(
            mirror_messages[0].message_id.as_deref(),
            Some("msg_mirror_user")
        );
        assert_eq!(mirror_messages[0].sender_type.as_deref(), Some("user"));
        assert_eq!(mirror_messages[0].body, "mirror this user message");
        assert_eq!(
            mirror_messages[1].message_id.as_deref(),
            Some("msg_mirror_agent")
        );
        assert_eq!(mirror_messages[1].sender_type.as_deref(), Some("agent"));
        assert_eq!(mirror_messages[1].body, "mirror this agent reply");
        assert_eq!(
            finitechat_mirror_failure_count(&runtime).expect("failure count"),
            0
        );
    }

    #[test]
    fn finitechat_mirror_survives_runtime_reopen() {
        let dir = tempdir().expect("tempdir");
        let state_dir = dir.path().join("chat");
        let thread_id = {
            let runtime = ChatRuntime::open_with_finitechat_mirror(Some(state_dir.clone()))
                .expect("chat runtime");
            let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
                .id
                .clone();
            runtime
                .send_user_message(ChatSendMessageRequest {
                    thread_id: thread_id.clone(),
                    message: SendChatMessageRequest {
                        body: "first mirrored message".to_string(),
                        client_message_id: Some("msg_mirror_reopen_user".to_string()),
                        attachments: vec![],
                    },
                })
                .expect("send message");
            thread_id
        };

        let runtime = ChatRuntime::open_with_finitechat_mirror(Some(state_dir.clone()))
            .expect("reopen chat runtime");
        let health = runtime.health();
        runtime
            .gateway_send(GatewayOutboundMessageRequest {
                agent_id: health.agent_id,
                thread_id: thread_id.clone(),
                body: "second mirrored message".to_string(),
                kind: Some("message".to_string()),
                status: Some("complete".to_string()),
                metadata: json!({}),
                reply_to_message_id: None,
                message_id: Some("msg_mirror_reopen_agent".to_string()),
            })
            .expect("gateway send");

        let mirror_messages =
            mirrored_chat_messages(&state_dir, &runtime).expect("mirrored chat messages");
        assert_eq!(mirror_messages.len(), 2);
        assert_eq!(
            mirror_messages
                .iter()
                .map(|message| message.message_id.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("msg_mirror_reopen_user"),
                Some("msg_mirror_reopen_agent")
            ]
        );
    }

    #[test]
    fn finitechat_mirror_skips_non_message_status_rows() {
        let dir = tempdir().expect("tempdir");
        let state_dir = dir.path().join("chat");
        let runtime = ChatRuntime::open_with_finitechat_mirror(Some(state_dir.clone()))
            .expect("chat runtime");
        let health = runtime.health();
        let thread_id = runtime.bootstrap().expect("bootstrap").threads[0]
            .id
            .clone();

        runtime
            .gateway_send(GatewayOutboundMessageRequest {
                agent_id: health.agent_id,
                thread_id,
                body: "thinking".to_string(),
                kind: Some("status".to_string()),
                status: Some("running".to_string()),
                metadata: json!({}),
                reply_to_message_id: None,
                message_id: Some("msg_mirror_status_skip".to_string()),
            })
            .expect("gateway send");

        let mirror_messages =
            mirrored_chat_messages(&state_dir, &runtime).expect("mirrored chat messages");
        assert!(mirror_messages.is_empty());
        assert_eq!(
            finitechat_mirror_failure_count(&runtime).expect("failure count"),
            0
        );
    }

    fn mirrored_chat_messages(
        state_dir: &Path,
        runtime: &ChatRuntime,
    ) -> Result<Vec<FiniteChatMessagePayload>> {
        let seed = finitechat_room_seed(&runtime.store.config);
        let store = SqliteDeliveryStore::open(state_dir.join(FINITE_CHAT_MIRROR_DB_FILE))?;
        let room = store
            .room(&seed.room_id)?
            .ok_or_else(|| anyhow!("mirror room missing"))?;
        assert!(room.device_active_at_head(&seed.user_device));
        assert!(room.device_active_at_head(&seed.runtime_device));
        let mut messages = Vec::new();
        for entry in room
            .log
            .iter()
            .filter(|entry| entry.kind == LogEntryKind::Application)
        {
            let event: DecryptedApplicationEventV1 =
                serde_json::from_slice(&entry.envelope.payload)?;
            if event.kind != DurableAppEventKind::ChatMessage {
                continue;
            }
            let payload: FiniteChatEventPayload = serde_json::from_slice(&event.payload)?;
            let FiniteChatEventPayload::ChatMessage(message) = payload else {
                continue;
            };
            messages.push(message);
        }
        Ok(messages)
    }

    fn finitechat_mirror_failure_count(runtime: &ChatRuntime) -> Result<i64> {
        runtime
            .store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM finitechat_mirror_failures",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    fn relay_inbox_thread(id: &str, project_agent_id: &str, title: &str) -> ChatThread {
        ChatThread {
            id: id.to_string(),
            project_agent_id: project_agent_id.to_string(),
            created_by: "hosted-web-user-paul".to_string(),
            title: title.to_string(),
            created_at: "2026-05-22T00:00:00Z".to_string(),
            last_activity_at: "2026-05-22T00:00:00Z".to_string(),
            message_count: 0,
        }
    }

    fn relay_inbox_message(id: &str, thread_id: &str, body: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            thread_id: thread_id.to_string(),
            sender_type: "user".to_string(),
            kind: "message".to_string(),
            status: "complete".to_string(),
            body: body.to_string(),
            metadata: json!({}),
            created_at: "2026-05-22T00:00:01Z".to_string(),
            updated_at: "2026-05-22T00:00:01Z".to_string(),
        }
    }
}
