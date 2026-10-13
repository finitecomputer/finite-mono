//! Direct transport for the Hermes sidecar.
//!
//! With `FINITECHAT_CHAT_TRANSPORT=direct` the sidecar keeps the agent's room
//! log in plaintext SQLite instead of an MLS Device, and serves the hosted
//! device's app API (`/v1/app/*`) to the person's web session through the
//! `finitechat` Hermes dashboard plugin. Hermes and agentd keep their
//! existing loopback contracts unchanged.
//!
//! On first start the sidecar folds the agent's existing MLS Device into the
//! log once: a final sync, then every stored decrypted event with its
//! sequence, message id, sender and time. The MLS store is only read. A fold
//! that cannot complete leaves the sidecar in MLS mode for this start.

use std::collections::HashSet;

use axum::body::to_bytes;
use axum::extract::Request as AxumRequest;
use axum::extract::{FromRequest, Multipart};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use base64::Engine as _;
use finitechat_core::{
    AppAcceptedActivity, AppBridgeAppliedEvent, AppProfileSummary, AppState, AppView,
    DIRECT_WEB_DEVICE_ID, DirectEventDraft, DirectFoldExport, DirectLastView, DirectLog,
    DirectRequesterContext, DirectRoom, DirectRoomEngine, DirectSelection,
};
use finitechat_hermes::{HermesAttachmentKindV1, HermesAttachmentV1};
use finitechat_proto::{
    AttachmentBlobMetadataV1, DecryptedApplicationEventV1, DurableAppEventKind,
    RuntimeCommandJsonPayloadV1, RuntimeCommandPayloadKindV1, RuntimeCommandRequestV1,
    RuntimeCommandResultV1, RuntimeCommandTargetV1, RuntimeCommandTerminalStatusV1,
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use super::*;

pub(super) const CHAT_TRANSPORT_ENV: &str = "FINITECHAT_CHAT_TRANSPORT";
/// Opt-in token streaming: running edits become ephemeral drafts.
const DIRECT_STREAMING_ENV: &str = "FINITECHAT_DIRECT_STREAMING";
const DIRECT_DIR: &str = "direct";
const DIRECT_LOG_FILE: &str = "room-log.sqlite3";
const DIRECT_BLOB_DIR: &str = "blobs";
const DIRECT_FOLD_STATUS_FILE: &str = "fold-status.json";
pub(super) const DIRECT_BLOB_URL_PREFIX: &str = "finitechat-direct:/blobs/";
const DIRECT_SCHEMA_VERSION: i64 = 1;
const AGENT_ID_ENV: &str = "FINITE_AGENT_ID";
const REQUESTER_EMAIL_HEADER: &str = "x-finite-requester-email";
const SITES_REQUESTER_ASSERTION_HEADER: &str = "x-finite-sites-requester-assertion";
const ROOM_NAME_ENV: &str = "FINITECHAT_HERMES_ROOM_NAME";
const AUTHORIZATION_HEADER: &str = "x-finite-chat-authorization";
const AUTHORIZATION_KIND: u16 = 27_235;
const AUTHORIZATION_WINDOW_SECS: u64 = 120;
const AUTHORIZATION_REPLAY_SECS: u64 = 600;
const MAX_APP_JSON_BODY_BYTES: usize = 1024 * 1024;
const MAX_ATTACHMENTS_PER_MESSAGE: usize = 8;
const MAX_ATTACHMENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_ATTACHMENT_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_MULTIPART_BODY_BYTES: usize = MAX_ATTACHMENT_TOTAL_BYTES + 1024 * 1024;
const APP_UPDATE_WAIT_MILLIS: u64 = 30_000;
const DEFAULT_RUNTIME_COMMAND_WAIT_MILLIS: u64 = 45_000;
const MAX_RUNTIME_COMMAND_WAIT_MILLIS: u64 = 60_000;
const OWNER_CLAIM_COMMAND: &str = "agent.owner.claim";
const FOLD_SYNC_ROUNDS: usize = 20;

pub(super) fn direct_transport_requested() -> bool {
    std::env::var(CHAT_TRANSPORT_ENV)
        .map(|value| value.trim().eq_ignore_ascii_case("direct"))
        .unwrap_or(false)
}

fn direct_dir(home_dir: &Path) -> PathBuf {
    home_dir.join(DIRECT_DIR)
}

fn direct_streaming_enabled() -> bool {
    std::env::var(DIRECT_STREAMING_ENV).is_ok_and(|value| value.trim() == "1")
}

pub(super) fn direct_blob_path(home_dir: &Path, ciphertext_sha256: &str) -> PathBuf {
    // Content addresses are hex; anything else cannot name a stored blob.
    let name = if ciphertext_sha256.len() == 64
        && ciphertext_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        ciphertext_sha256
    } else {
        "invalid"
    };
    direct_dir(home_dir).join(DIRECT_BLOB_DIR).join(name)
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn random_hex<const N: usize>() -> Result<String, CliError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes)
        .map_err(|error| CliError::Hermes(format!("random bytes failed: {error}")))?;
    Ok(hex::encode(bytes))
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn core_error(error: CliError) -> FiniteChatCoreError {
    FiniteChatCoreError::Client {
        reason: error.to_string(),
    }
}

/// How the log came to exist. Reported by `/v1/app/info`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct DirectOrigin {
    state: &'static str,
    source: Option<&'static str>,
    folded_at_unix_seconds: Option<u64>,
}

/// The plaintext room log. One connection, guarded by the runtime's lock.
struct DirectStore {
    conn: Connection,
}

impl DirectStore {
    fn path(home_dir: &Path) -> PathBuf {
        direct_dir(home_dir).join(DIRECT_LOG_FILE)
    }

    fn exists(home_dir: &Path) -> bool {
        Self::path(home_dir).is_file()
    }

    fn open(path: &Path) -> Result<Self, CliError> {
        let conn = Connection::open(path)
            .map_err(|error| CliError::Hermes(format!("direct log open failed: {error}")))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS direct_meta (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS direct_rooms (
               room_id TEXT PRIMARY KEY,
               display_name TEXT NOT NULL,
               picture TEXT,
               read_seq INTEGER NOT NULL DEFAULT 0,
               next_seq INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS direct_members (
               room_id TEXT NOT NULL REFERENCES direct_rooms(room_id),
               account_id TEXT NOT NULL,
               PRIMARY KEY (room_id, account_id)
             );
             CREATE TABLE IF NOT EXISTS direct_events (
               row_id INTEGER PRIMARY KEY AUTOINCREMENT,
               room_id TEXT NOT NULL REFERENCES direct_rooms(room_id),
               seq INTEGER NOT NULL,
               message_id TEXT NOT NULL,
               sender_account_id TEXT NOT NULL,
               sender_device_id TEXT NOT NULL,
               timestamp_unix_seconds INTEGER NOT NULL,
               plaintext BLOB NOT NULL,
               origin TEXT NOT NULL,
               idempotency_key TEXT,
               UNIQUE (room_id, seq),
               UNIQUE (room_id, message_id)
             );
             CREATE UNIQUE INDEX IF NOT EXISTS direct_events_idempotency
               ON direct_events(room_id, idempotency_key) WHERE idempotency_key IS NOT NULL;
             CREATE TABLE IF NOT EXISTS direct_profiles (
               account_id TEXT PRIMARY KEY,
               profile_json TEXT NOT NULL
             );",
        )
        .map_err(|error| CliError::Hermes(format!("direct log schema failed: {error}")))?;
        Ok(Self { conn })
    }

    fn meta(&self, key: &str) -> Result<Option<String>, CliError> {
        self.conn
            .query_row(
                "SELECT value FROM direct_meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)
    }

    /// The person's view of everything durable, including where they were.
    fn load_engine(
        &self,
        owner: DeviceRef,
        agent: DeviceRef,
    ) -> Result<DirectRoomEngine, CliError> {
        let mut engine = DirectRoomEngine::new(owner, agent);
        for (room, read_seq) in self.rooms()? {
            let room_id = room.room_id.clone();
            engine.upsert_room(room);
            engine.set_read_seq(&room_id, read_seq);
        }
        for profile in self.profiles()? {
            engine.upsert_profile(profile);
        }
        for event in self.all_events()? {
            engine.apply(event);
        }
        // A view that no longer parses only loses the landing place.
        engine.set_last_view(
            self.meta("last_view")?
                .and_then(|value| serde_json::from_str(&value).ok()),
        );
        Ok(engine)
    }

    fn set_last_view(&self, view: Option<&DirectLastView>) -> Result<(), CliError> {
        match view {
            Some(view) => self.set_meta(
                "last_view",
                &serde_json::to_string(view).map_err(CliError::Serialize)?,
            ),
            None => self
                .conn
                .execute("DELETE FROM direct_meta WHERE key = 'last_view'", [])
                .map(|_| ())
                .map_err(sql_error),
        }
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<(), CliError> {
        self.conn
            .execute(
                "INSERT INTO direct_meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(|_| ())
            .map_err(sql_error)
    }

    fn insert_room(&self, room: &DirectRoom, next_seq: u64, read_seq: u64) -> Result<(), CliError> {
        self.conn
            .execute(
                "INSERT INTO direct_rooms(room_id, display_name, picture, read_seq, next_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    room.room_id,
                    room.display_name,
                    room.picture,
                    read_seq as i64,
                    next_seq as i64
                ],
            )
            .map(|_| ())
            .map_err(sql_error)
    }

    fn insert_member(&self, room_id: &str, account_id: &str) -> Result<(), CliError> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO direct_members(room_id, account_id) VALUES (?1, ?2)",
                params![room_id, account_id],
            )
            .map(|_| ())
            .map_err(sql_error)
    }

    fn insert_event(
        &self,
        event: &StoredAppEvent,
        origin: &str,
        idempotency_key: Option<&str>,
    ) -> Result<(), CliError> {
        self.conn
            .execute(
                "INSERT INTO direct_events(room_id, seq, message_id, sender_account_id,
                   sender_device_id, timestamp_unix_seconds, plaintext, origin, idempotency_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    event.room_id,
                    event.seq as i64,
                    event.message_id,
                    event.sender.account_id,
                    event.sender.device_id,
                    event.timestamp_unix_seconds as i64,
                    event.plaintext,
                    origin,
                    idempotency_key
                ],
            )
            .map(|_| ())
            .map_err(sql_error)
    }

    fn event_for_idempotency_key(
        &self,
        room_id: &str,
        key: &str,
    ) -> Result<Option<StoredAppEvent>, CliError> {
        self.conn
            .query_row(
                "SELECT room_id, seq, message_id, sender_account_id, sender_device_id,
                   plaintext, timestamp_unix_seconds
                 FROM direct_events WHERE room_id = ?1 AND idempotency_key = ?2",
                params![room_id, key],
                stored_event_from_row,
            )
            .optional()
            .map_err(sql_error)
    }

    fn take_next_seq(&self, room_id: &str) -> Result<u64, CliError> {
        let seq: i64 = self
            .conn
            .query_row(
                "UPDATE direct_rooms SET next_seq = next_seq + 1 WHERE room_id = ?1
                 RETURNING next_seq - 1",
                params![room_id],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        Ok(seq as u64)
    }

    fn rooms(&self) -> Result<Vec<(DirectRoom, u64)>, CliError> {
        let mut statement = self
            .conn
            .prepare("SELECT room_id, display_name, picture, read_seq FROM direct_rooms ORDER BY room_id")
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    DirectRoom {
                        room_id: row.get(0)?,
                        display_name: row.get(1)?,
                        picture: row.get(2)?,
                    },
                    row.get::<_, i64>(3)? as u64,
                ))
            })
            .map_err(sql_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)
    }

    fn set_read_seq(&self, room_id: &str, seq: u64) -> Result<(), CliError> {
        self.conn
            .execute(
                "UPDATE direct_rooms SET read_seq = ?2 WHERE room_id = ?1",
                params![room_id, seq as i64],
            )
            .map(|_| ())
            .map_err(sql_error)
    }

    fn all_events(&self) -> Result<Vec<StoredAppEvent>, CliError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT room_id, seq, message_id, sender_account_id, sender_device_id,
                   plaintext, timestamp_unix_seconds
                 FROM direct_events ORDER BY room_id, seq",
            )
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], stored_event_from_row)
            .map_err(sql_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)
    }

    fn recent_events(&self, limit: u32) -> Result<Vec<StoredAppEvent>, CliError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT room_id, seq, message_id, sender_account_id, sender_device_id,
                   plaintext, timestamp_unix_seconds
                 FROM (SELECT * FROM direct_events ORDER BY row_id DESC LIMIT ?1)
                 ORDER BY row_id ASC",
            )
            .map_err(sql_error)?;
        let rows = statement
            .query_map(params![limit as i64], stored_event_from_row)
            .map_err(sql_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)
    }

    fn events_after_row(&self, row_id: i64) -> Result<(Vec<StoredAppEvent>, i64), CliError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT room_id, seq, message_id, sender_account_id, sender_device_id,
                   plaintext, timestamp_unix_seconds, row_id
                 FROM direct_events WHERE row_id > ?1 ORDER BY row_id ASC",
            )
            .map_err(sql_error)?;
        let mut last = row_id;
        let rows = statement
            .query_map(params![row_id], |row| {
                Ok((stored_event_from_row(row)?, row.get::<_, i64>(7)?))
            })
            .map_err(sql_error)?;
        let mut events = Vec::new();
        for row in rows {
            let (event, id) = row.map_err(sql_error)?;
            last = last.max(id);
            events.push(event);
        }
        Ok((events, last))
    }

    fn max_row_id(&self) -> Result<i64, CliError> {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(row_id), 0) FROM direct_events",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)
    }

    fn profiles(&self) -> Result<Vec<AppProfileSummary>, CliError> {
        let mut statement = self
            .conn
            .prepare("SELECT profile_json FROM direct_profiles ORDER BY account_id")
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sql_error)?;
        let mut profiles = Vec::new();
        for row in rows {
            if let Ok(profile) = serde_json::from_str::<AppProfileSummary>(&row.map_err(sql_error)?)
            {
                profiles.push(profile);
            }
        }
        Ok(profiles)
    }

    fn upsert_profile(&self, profile: &AppProfileSummary) -> Result<(), CliError> {
        self.conn
            .execute(
                "INSERT INTO direct_profiles(account_id, profile_json) VALUES (?1, ?2)
                 ON CONFLICT(account_id) DO UPDATE SET profile_json = excluded.profile_json",
                params![
                    profile.account_id,
                    serde_json::to_string(profile).map_err(CliError::Serialize)?
                ],
            )
            .map(|_| ())
            .map_err(sql_error)
    }
}

fn stored_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredAppEvent> {
    Ok(StoredAppEvent {
        room_id: row.get(0)?,
        seq: row.get::<_, i64>(1)? as u64,
        message_id: row.get(2)?,
        sender: DeviceRef {
            account_id: row.get(3)?,
            device_id: row.get(4)?,
        },
        plaintext: row.get(5)?,
        timestamp_unix_seconds: row.get::<_, i64>(6)? as u64,
    })
}

fn sql_error(error: rusqlite::Error) -> CliError {
    CliError::Hermes(format!("direct log: {error}"))
}

/// Appends through the store; the engine applies what it returns.
struct StoreLog<'a> {
    store: &'a DirectStore,
}

impl DirectLog for StoreLog<'_> {
    fn append(&mut self, draft: DirectEventDraft) -> Result<StoredAppEvent, FiniteChatCoreError> {
        if let Some(key) = &draft.idempotency_key
            && let Some(existing) = self
                .store
                .event_for_idempotency_key(&draft.room_id, key)
                .map_err(core_error)?
        {
            return Ok(existing);
        }
        let seq = self
            .store
            .take_next_seq(&draft.room_id)
            .map_err(core_error)?;
        let timestamp_unix_seconds = now_unix_seconds();
        let message_id = format!(
            "direct-{}",
            &sha256_hex_bytes(
                format!(
                    "{}\0{seq}\0{}\0{timestamp_unix_seconds}\0{}",
                    draft.room_id,
                    draft.sender.account_id,
                    sha256_hex_bytes(&draft.plaintext)
                )
                .as_bytes()
            )[..40]
        );
        let event = StoredAppEvent {
            room_id: draft.room_id,
            seq,
            message_id,
            sender: draft.sender,
            plaintext: draft.plaintext,
            timestamp_unix_seconds,
        };
        self.store
            .insert_event(&event, "direct", draft.idempotency_key.as_deref())
            .map_err(core_error)?;
        Ok(event)
    }

    fn generate_object_id(&mut self, prefix: &str) -> Result<String, FiniteChatCoreError> {
        random_hex::<8>()
            .map(|hex| format!("{prefix}-{hex}"))
            .map_err(core_error)
    }
}

struct DirectInner {
    store: DirectStore,
    engine: DirectRoomEngine,
    polled_row_id: i64,
}

/// The direct backend behind the sidecar's loopback and app APIs.
pub(super) struct DirectRuntime {
    home_dir: PathBuf,
    server_url: String,
    agent: DeviceRef,
    agent_id: Option<String>,
    origin: DirectOrigin,
    inner: Mutex<DirectInner>,
    updates: (Mutex<u64>, Condvar),
    seen_authorizations: Mutex<BTreeMap<String, u64>>,
    on_append: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl DirectRuntime {
    /// Open the log, folding the agent's MLS Device on first start.
    pub(super) fn open_or_fold(home: &AgentHome) -> Result<Arc<Self>, CliError> {
        let home_dir = home.dir.clone();
        if !DirectStore::exists(&home_dir) {
            fold_agent_device(home)?;
        } else {
            refuse_if_mls_advanced_after_fold(home)?;
        }
        Self::open(home)
    }

    fn open(home: &AgentHome) -> Result<Arc<Self>, CliError> {
        let store = DirectStore::open(&DirectStore::path(&home.dir))?;
        let owner_account = store.meta("owner_account_id")?.unwrap_or_default();
        let agent = DeviceRef {
            account_id: home.config.account_id.clone(),
            device_id: home.config.device_id.clone(),
        };
        let owner = DeviceRef {
            account_id: owner_account,
            device_id: DIRECT_WEB_DEVICE_ID.to_owned(),
        };
        let engine = store.load_engine(owner, agent.clone())?;
        let polled_row_id = store.max_row_id()?;
        let origin = DirectOrigin {
            state: match store.meta("origin")?.as_deref() {
                Some("fold") => "migrated",
                _ => "fresh",
            },
            source: store
                .meta("origin")?
                .filter(|value| value == "fold")
                .map(|_| "agent-store"),
            folded_at_unix_seconds: store
                .meta("created_at")?
                .and_then(|value| value.parse().ok()),
        };
        Ok(Arc::new(Self {
            home_dir: home.dir.clone(),
            server_url: home.config.server_url.clone(),
            agent,
            agent_id: std::env::var(AGENT_ID_ENV)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            origin,
            inner: Mutex::new(DirectInner {
                store,
                engine,
                polled_row_id,
            }),
            updates: (Mutex::new(0), Condvar::new()),
            seen_authorizations: Mutex::new(BTreeMap::new()),
            on_append: Mutex::new(None),
        }))
    }

    pub(super) fn set_on_append(&self, callback: Box<dyn Fn() + Send + Sync>) {
        if let Ok(mut slot) = self.on_append.lock() {
            *slot = Some(callback);
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, DirectInner>, CliError> {
        self.inner
            .lock()
            .map_err(|_| CliError::Hermes("direct log lock poisoned".to_owned()))
    }

    fn notify(&self) {
        if let Ok(mut generation) = self.updates.0.lock() {
            *generation = generation.wrapping_add(1);
            self.updates.1.notify_all();
        }
        if let Ok(callback) = self.on_append.lock()
            && let Some(callback) = callback.as_ref()
        {
            callback();
        }
    }

    fn generation(&self) -> u64 {
        self.updates.0.lock().map(|value| *value).unwrap_or(0)
    }

    fn wait_for_change(&self, observed: u64, timeout_millis: u64) {
        let Ok(guard) = self.updates.0.lock() else {
            return;
        };
        let _ = self.updates.1.wait_timeout_while(
            guard,
            Duration::from_millis(timeout_millis.max(1)),
            |generation| *generation == observed,
        );
    }

    /// Run one mutation against the engine and its log, then wake readers.
    fn mutate<T>(
        &self,
        f: impl FnOnce(&mut DirectRoomEngine, &mut StoreLog<'_>) -> Result<T, FiniteChatCoreError>,
    ) -> Result<T, CliError> {
        let result = {
            let mut inner = self.lock()?;
            let DirectInner { store, engine, .. } = &mut *inner;
            store
                .conn
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(sql_error)?;
            let last_view = engine.last_view().cloned();
            let mut log = StoreLog { store };
            match f(engine, &mut log).and_then(|value| {
                if engine.last_view() != last_view.as_ref() {
                    store
                        .set_last_view(engine.last_view())
                        .map_err(core_error)?;
                }
                Ok(value)
            }) {
                Ok(value) => {
                    store.conn.execute_batch("COMMIT").map_err(sql_error)?;
                    Ok(value)
                }
                Err(error) => {
                    let _ = store.conn.execute_batch("ROLLBACK");
                    // The engine applied events the store just discarded:
                    // rebuild from what is durable.
                    drop(inner);
                    self.reload()?;
                    Err(CliError::from(error))
                }
            }
        };
        if result.is_ok() {
            self.notify();
        }
        result
    }

    fn reload(&self) -> Result<(), CliError> {
        let mut inner = self.lock()?;
        let owner = DeviceRef {
            account_id: inner.store.meta("owner_account_id")?.unwrap_or_default(),
            device_id: DIRECT_WEB_DEVICE_ID.to_owned(),
        };
        inner.engine = inner.store.load_engine(owner, self.agent.clone())?;
        Ok(())
    }

    fn single_room_id(&self) -> Result<Option<String>, CliError> {
        let inner = self.lock()?;
        let rooms = inner.engine.room_ids();
        Ok((rooms.len() == 1).then(|| rooms[0].clone()))
    }

    fn owner_account_id(&self) -> Result<String, CliError> {
        Ok(self.lock()?.engine.owner().account_id.clone())
    }

    /// Accept a person's request signature; see the module docs.
    fn authorize(
        &self,
        headers: &HeaderMap,
        method: &str,
        path_and_query: &str,
        body: &[u8],
    ) -> Result<String, AppApiError> {
        let header = headers
            .get(AUTHORIZATION_HEADER)
            .and_then(|value| value.to_str().ok())
            .ok_or(AppApiError::Unauthorized("chat authorization is required"))?;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(header.trim().trim_end_matches('='))
            .map_err(|_| AppApiError::Unauthorized("chat authorization is malformed"))?;
        let event: nostr::Event = serde_json::from_slice(&decoded)
            .map_err(|_| AppApiError::Unauthorized("chat authorization is malformed"))?;
        event
            .verify()
            .map_err(|_| AppApiError::Unauthorized("chat authorization signature is invalid"))?;
        if event.kind.as_u16() != AUTHORIZATION_KIND || !event.content.is_empty() {
            return Err(AppApiError::Unauthorized(
                "chat authorization kind is invalid",
            ));
        }
        let now = now_unix_seconds();
        let created = event.created_at.as_secs();
        if created.abs_diff(now) > AUTHORIZATION_WINDOW_SECS {
            return Err(AppApiError::Unauthorized("chat authorization expired"));
        }
        let agent_id = self
            .agent_id
            .as_deref()
            .ok_or(AppApiError::Unauthorized("agent id is not configured"))?;
        let expected_url = format!("finitechat-direct://{agent_id}{path_and_query}");
        let mut url = None;
        let mut signed_method = None;
        let mut payload = None;
        for tag in event.tags.iter() {
            match tag.as_slice() {
                [name, value] if name == "u" => url = Some(value.as_str()),
                [name, value] if name == "method" => signed_method = Some(value.as_str()),
                [name, value] if name == "payload" => payload = Some(value.as_str()),
                _ => {}
            }
        }
        if url != Some(expected_url.as_str()) || signed_method != Some(method) {
            return Err(AppApiError::Unauthorized(
                "chat authorization is for another request",
            ));
        }
        if method == "POST" && payload != Some(sha256_hex_bytes(body).as_str()) {
            return Err(AppApiError::Unauthorized(
                "chat authorization body does not match",
            ));
        }
        let signer = event.pubkey.to_hex();
        self.authorize_signer(&signer)?;
        let mut seen = self
            .seen_authorizations
            .lock()
            .map_err(|_| AppApiError::Internal("authorization cache poisoned".to_owned()))?;
        seen.retain(|_, at| now.saturating_sub(*at) <= AUTHORIZATION_REPLAY_SECS);
        if seen.insert(event.sig.to_string(), now).is_some() {
            return Err(AppApiError::Unauthorized(
                "chat authorization was already used",
            ));
        }
        Ok(signer)
    }

    /// The signer must be the person this log projects for. A fresh room
    /// with no known person accepts its first verified signer, as owner
    /// claim does.
    fn authorize_signer(&self, signer: &str) -> Result<(), AppApiError> {
        let inner = self
            .lock()
            .map_err(|error| AppApiError::Internal(error.to_string()))?;
        let owner = inner.engine.owner().account_id.clone();
        if owner == signer {
            return Ok(());
        }
        if !owner.is_empty() {
            return Err(AppApiError::Forbidden(
                "this person cannot use this agent's chat",
            ));
        }
        let claimable = owner_npubs_from_env();
        if !claimable.is_empty() && !claimable.contains(signer) {
            return Err(AppApiError::Forbidden(
                "this person cannot use this agent's chat",
            ));
        }
        inner
            .store
            .set_meta("owner_account_id", signer)
            .map_err(|error| AppApiError::Internal(error.to_string()))?;
        let rooms = inner.engine.room_ids();
        for room_id in &rooms {
            inner
                .store
                .insert_member(room_id, signer)
                .map_err(|error| AppApiError::Internal(error.to_string()))?;
        }
        drop(inner);
        self.reload()
            .map_err(|error| AppApiError::Internal(error.to_string()))?;
        Ok(())
    }

    fn app_state(&self, view: AppView, status: &str) -> Result<AppState, CliError> {
        let inner = self.lock()?;
        let mut state = inner
            .engine
            .state_for_view(view, status, now_ms())
            .map_err(CliError::from)?;
        state.identity.account_secret_hex.clear();
        Ok(state)
    }

    fn view_for_selection(&self, selection: &DirectSelection, limit: Option<u32>) -> AppView {
        AppView {
            room_id: selection.room_id.clone(),
            topic_id: selection.topic_id.clone(),
            chat_id: selection.chat_id.clone(),
            limit,
            oldest_message_id: selection.oldest_message_id.clone(),
        }
    }

    fn info(&self) -> Result<Value, CliError> {
        let inner = self.lock()?;
        let rooms = inner.engine.room_ids();
        Ok(json!({
            "transport": "direct",
            "version": 1,
            "agent_id": self.agent_id,
            "agent_account_id": self.agent.account_id,
            "rooms": rooms,
            "canonical_room_id": (rooms.len() == 1).then(|| rooms[0].clone()),
            "migration": self.origin,
        }))
    }

    fn store_attachment(
        &self,
        attachment: OutboundAttachment,
    ) -> Result<HermesAttachmentV1, CliError> {
        let filename = attachment.filename.trim().to_owned();
        let mime_type = attachment.mime_type.trim().to_owned();
        let metadata = AttachmentBlobMetadataV1 {
            mime_type: mime_type.clone(),
            filename: filename.clone(),
            dimensions: None,
        };
        let prepared = finitechat_blob::prepare_attachment_upload(&attachment.bytes, metadata)
            .map_err(|error| CliError::Hermes(format!("attachment preparation failed: {error}")))?;
        let ciphertext_sha256 = sha256_hex(&prepared.ciphertext);
        let blob_dir = direct_dir(&self.home_dir).join(DIRECT_BLOB_DIR);
        fs::create_dir_all(&blob_dir).map_err(|error| CliError::Hermes(error.to_string()))?;
        write_private(blob_dir.join(&ciphertext_sha256), &prepared.ciphertext)?;
        let reference = finitechat_blob::finish_attachment_upload(
            &prepared,
            finitechat_blob::BlobDescriptor {
                url: format!("{DIRECT_BLOB_URL_PREFIX}{ciphertext_sha256}"),
                sha256: ciphertext_sha256.clone(),
                size_bytes: prepared.ciphertext_size,
            },
        )
        .map_err(|error| CliError::Hermes(format!("attachment reference failed: {error}")))?;
        // Hermes reads inbound files from its cache; fill it now.
        let cache = hermes_attachment_cache_path(&self.home_dir, &reference);
        if let Some(parent) = cache.parent() {
            fs::create_dir_all(parent).map_err(|error| CliError::Hermes(error.to_string()))?;
        }
        write_private(cache.clone(), &attachment.bytes)?;
        Ok(HermesAttachmentV1 {
            kind: match attachment.kind {
                ChatMediaKind::Image => HermesAttachmentKindV1::Image,
                ChatMediaKind::VoiceNote => HermesAttachmentKindV1::Audio,
                ChatMediaKind::Video => HermesAttachmentKindV1::Video,
                ChatMediaKind::File => HermesAttachmentKindV1::File,
            },
            name: filename,
            mime_type,
            path: None,
            url: Some(reference.url.clone()),
            blob: Some(reference),
        })
    }

    fn attachment_bytes(
        &self,
        room_id: &str,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<(AttachmentBlobReferenceV1, Vec<u8>), AppApiError> {
        let reference = self
            .lock()
            .map_err(|error| AppApiError::Internal(error.to_string()))?
            .engine
            .attachment_reference(room_id, message_id, attachment_id)
            .ok_or(AppApiError::NotFound("attachment is not available"))?;
        let path = materialize_blob_attachment(&self.home_dir, &self.server_url, &reference)
            .map_err(|error| match error {
                AttachmentMaterializationError::Retryable(reason) => {
                    AppApiError::Unavailable(reason)
                }
                AttachmentMaterializationError::Permanent(reason) => AppApiError::Gone(reason),
            })?;
        let bytes = fs::read(path).map_err(|error| AppApiError::Internal(error.to_string()))?;
        Ok((reference, bytes))
    }

    fn send_runtime_command(
        &self,
        input: RuntimeCommandInput,
    ) -> Result<RuntimeCommandOutput, AppApiError> {
        if input.reuse_succeeded_owner_claim
            && input.command == OWNER_CLAIM_COMMAND
            && let Some(result) = self.find_succeeded_owner_claim(&input)?
        {
            return runtime_command_output(result);
        }
        let request_id = format!(
            "runtime-{}",
            random_hex::<16>().map_err(|error| AppApiError::Internal(error.to_string()))?
        );
        let body = serde_json::to_vec(&input.body)
            .map_err(|error| AppApiError::BadRequest(error.to_string()))?;
        let request = RuntimeCommandRequestV1 {
            payload_kind: RuntimeCommandPayloadKindV1::Request,
            request_id: request_id.clone(),
            command: input.command.clone(),
            target: RuntimeCommandTargetV1 {
                account_id: input.target_account_id.clone(),
                device_id: None,
            },
            resource_key: input.resource_key.clone(),
            body: RuntimeCommandJsonPayloadV1 {
                schema: input.schema.clone(),
                json_payload: body,
            },
        };
        request
            .validate_structure()
            .map_err(|error| AppApiError::BadRequest(error.to_string()))?;
        let payload = serde_json::to_vec(&request)
            .map_err(|error| AppApiError::Internal(error.to_string()))?;
        let plaintext = encode_runtime_event(
            DurableAppEventKind::RuntimeCommandRequest,
            input.conversation_id.clone(),
            &payload,
        )
        .map_err(|error| AppApiError::BadRequest(error.to_string()))?;
        let room_id = input.room_id.clone();
        self.mutate(|engine, log| engine.append_owner_event(log, &room_id, plaintext))
            .map_err(AppApiError::from_cli)?;
        let wait = Duration::from_millis(
            input
                .wait_millis
                .clamp(1_000, MAX_RUNTIME_COMMAND_WAIT_MILLIS),
        );
        let started = Instant::now();
        loop {
            let observed = self.generation();
            if let Some(result) = self.find_runtime_command_result(&input, &request_id)? {
                return runtime_command_output(result);
            }
            let elapsed = started.elapsed();
            if elapsed >= wait {
                return Err(AppApiError::Timeout(
                    "The agent did not respond in time. Try again.".to_owned(),
                ));
            }
            self.wait_for_change(observed, (wait - elapsed).as_millis() as u64);
        }
    }

    fn runtime_events(&self) -> Result<Vec<StoredAppEvent>, AppApiError> {
        self.lock()
            .map_err(|error| AppApiError::Internal(error.to_string()))?
            .store
            .recent_events(HERMES_STORED_EVENT_RECOVERY_LIMIT)
            .map_err(|error| AppApiError::Internal(error.to_string()))
    }

    fn find_succeeded_owner_claim(
        &self,
        input: &RuntimeCommandInput,
    ) -> Result<Option<RuntimeCommandResultV1>, AppApiError> {
        let owner = self
            .owner_account_id()
            .map_err(|error| AppApiError::Internal(error.to_string()))?;
        let mut matching = HashSet::new();
        for stored in self.runtime_events()? {
            if stored.room_id != input.room_id {
                continue;
            }
            let Ok(event) =
                serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext)
            else {
                continue;
            };
            if event.conversation_id.as_deref() != input.conversation_id.as_deref() {
                continue;
            }
            match event.kind {
                DurableAppEventKind::RuntimeCommandRequest if stored.sender.account_id == owner => {
                    let Ok(request) =
                        serde_json::from_slice::<RuntimeCommandRequestV1>(&event.payload)
                    else {
                        continue;
                    };
                    let body_matches = serde_json::from_slice::<Value>(&request.body.json_payload)
                        .is_ok_and(|body| body == input.body);
                    if request.validate_structure().is_ok()
                        && request.command == OWNER_CLAIM_COMMAND
                        && request.target.account_id == input.target_account_id
                        && request.target.device_id.is_none()
                        && request.resource_key == input.resource_key
                        && request.body.schema == input.schema
                        && body_matches
                    {
                        matching.insert(request.request_id);
                    }
                }
                DurableAppEventKind::RuntimeCommandResult
                    if stored.sender.account_id == input.target_account_id =>
                {
                    let Ok(result) =
                        serde_json::from_slice::<RuntimeCommandResultV1>(&event.payload)
                    else {
                        continue;
                    };
                    if result.validate_structure().is_ok()
                        && result.status == RuntimeCommandTerminalStatusV1::Succeeded
                        && matching.contains(&result.request_id)
                    {
                        return Ok(Some(result));
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn find_runtime_command_result(
        &self,
        input: &RuntimeCommandInput,
        request_id: &str,
    ) -> Result<Option<RuntimeCommandResultV1>, AppApiError> {
        for stored in self.runtime_events()? {
            if stored.room_id != input.room_id
                || stored.sender.account_id != input.target_account_id
            {
                continue;
            }
            let Ok(event) =
                serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext)
            else {
                continue;
            };
            if event.kind != DurableAppEventKind::RuntimeCommandResult
                || event.conversation_id.as_deref() != input.conversation_id.as_deref()
            {
                continue;
            }
            let Ok(result) = serde_json::from_slice::<RuntimeCommandResultV1>(&event.payload)
            else {
                continue;
            };
            if result.request_id == request_id && result.validate_structure().is_ok() {
                return Ok(Some(result));
            }
        }
        Ok(None)
    }
}

fn encode_runtime_event(
    kind: DurableAppEventKind,
    conversation_id: Option<String>,
    payload: &[u8],
) -> Result<Vec<u8>, CliError> {
    let event = DecryptedApplicationEventV1 {
        kind,
        conversation_id,
        segment_id: None,
        payload: payload.to_vec(),
    };
    event
        .validate_limits()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    serde_json::to_vec(&event).map_err(CliError::Serialize)
}

fn owner_npubs_from_env() -> BTreeSet<String> {
    std::env::var(OWNER_NPUBS_ENV)
        .ok()
        .and_then(|raw| parse_welcome_allowlist(&raw).ok())
        .unwrap_or_default()
}

impl SidecarRuntime for DirectRuntime {
    fn transport(&self) -> &'static str {
        "direct"
    }

    fn as_direct(&self) -> Option<&DirectRuntime> {
        Some(self)
    }

    fn start(&self) -> Result<AppState, CliError> {
        self.state()
    }

    fn state(&self) -> Result<AppState, CliError> {
        self.app_state(AppView::default(), "ready")
    }

    fn poll_once(&self) -> Result<AppBridgeSync, CliError> {
        let mut inner = self.lock()?;
        let (events, last) = inner.store.events_after_row(inner.polled_row_id)?;
        inner.polled_row_id = last;
        Ok(AppBridgeSync {
            joined_account_ids: Vec::new(),
            events: events
                .into_iter()
                .map(|event| AppBridgeAppliedEvent {
                    room_id: event.room_id,
                    seq: event.seq,
                    message_id: event.message_id,
                    sender_account_id: event.sender.account_id,
                    sender_device_id: event.sender.device_id,
                    plaintext: event.plaintext,
                })
                .collect(),
        })
    }

    fn wait_for_update(&self, timeout_millis: u64) -> Result<AppBridgeSync, CliError> {
        let observed = self.generation();
        let sync = self.poll_once()?;
        if !sync.events.is_empty() {
            return Ok(sync);
        }
        self.wait_for_change(observed, timeout_millis);
        self.poll_once()
    }

    fn recent_events(&self, limit: u32) -> Result<Vec<StoredAppEvent>, CliError> {
        self.lock()?.store.recent_events(limit)
    }

    fn send_encoded_chat_message(
        &self,
        room_id: String,
        plaintext: Vec<u8>,
        _preview: String,
    ) -> Result<AppSentMessage, CliError> {
        if direct_streaming_enabled() {
            let draft = self.lock()?.engine.set_draft(
                &room_id,
                &self.agent,
                &plaintext,
                now_unix_seconds(),
            );
            if let Some((message_id, seq)) = draft {
                self.notify();
                return Ok(AppSentMessage { message_id, seq });
            }
        }
        let event =
            self.mutate(|engine, log| engine.append_agent_event(log, &room_id, plaintext))?;
        Ok(AppSentMessage {
            message_id: event.message_id,
            seq: event.seq,
        })
    }

    fn upload_bridge_attachment(
        &self,
        _room_id: String,
        attachment: OutboundAttachment,
    ) -> Result<HermesAttachmentV1, CliError> {
        self.store_attachment(attachment)
    }

    fn append_ephemeral_activity(
        &self,
        input: AppBridgeActivityInput,
    ) -> Result<AppAcceptedActivity, CliError> {
        let accepted = {
            let mut inner = self.lock()?;
            inner.engine.apply_activity(&input, &self.agent, now_ms())
        };
        self.notify();
        Ok(accepted)
    }

    fn send_runtime_command_result(
        &self,
        room_id: String,
        conversation_id: Option<String>,
        payload: Vec<u8>,
    ) -> Result<String, CliError> {
        let plaintext = encode_runtime_event(
            DurableAppEventKind::RuntimeCommandResult,
            conversation_id,
            &payload,
        )?;
        self.mutate(|engine, log| engine.append_agent_event(log, &room_id, plaintext))
            .map(|event| event.message_id)
    }

    fn send_runtime_state_snapshot(
        &self,
        room_id: String,
        conversation_id: Option<String>,
        payload: Vec<u8>,
    ) -> Result<String, CliError> {
        let plaintext = encode_runtime_event(
            DurableAppEventKind::RuntimeStateSnapshot,
            conversation_id,
            &payload,
        )?;
        self.mutate(|engine, log| engine.append_agent_event(log, &room_id, plaintext))
            .map(|event| event.message_id)
    }

    fn room_sync_cursors(&self) -> Result<Vec<AppRoomSyncCursor>, CliError> {
        Ok(Vec::new())
    }
}

/// Fold the agent's MLS Device into a new direct log, once.
fn fold_agent_device(home: &AgentHome) -> Result<(), CliError> {
    let dir = direct_dir(&home.dir);
    fs::create_dir_all(&dir).map_err(|error| CliError::Hermes(error.to_string()))?;
    let has_mls_store = home.dir.join(STORE_FILE).is_file();
    let export = if has_mls_store {
        Some(final_sync_and_export(home)?)
    } else {
        None
    };
    let staging = dir.join(format!("{DIRECT_LOG_FILE}.fold-{}", std::process::id()));
    let _ = fs::remove_file(&staging);
    let report = {
        let store = DirectStore::open(&staging)?;
        store
            .conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(sql_error)?;
        let report = match &export {
            Some(export) if !export.rooms.is_empty() => write_fold(&store, home, export)?,
            _ => write_fresh_room(&store, home)?,
        };
        store.conn.execute_batch("COMMIT").map_err(sql_error)?;
        report
    };
    fs::rename(&staging, DirectStore::path(&home.dir))
        .map_err(|error| CliError::Hermes(format!("direct log install failed: {error}")))?;
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{suffix}", staging.display()));
    }
    // What the person sees right after the fold, as hashes, so operators can
    // compare it with the hosted device from the host's /data mount without
    // entering the agent.
    let mut report = report;
    let folded = DirectRuntime::open(home)?;
    let digests = {
        let inner = folded.lock()?;
        inner
            .engine
            .room_ids()
            .into_iter()
            .map(|room_id| inner.engine.digest(&room_id))
            .collect::<Vec<_>>()
    };
    report["digests"] = serde_json::to_value(digests).map_err(CliError::Serialize)?;
    write_private(
        dir.join(DIRECT_FOLD_STATUS_FILE),
        &serde_json::to_string_pretty(&report).map_err(CliError::Serialize)?,
    )?;
    eprintln!(
        "finitechat direct transport: {} room log created ({})",
        report["origin"].as_str().unwrap_or("direct"),
        report["summary"].as_str().unwrap_or_default()
    );
    Ok(())
}

/// A folded log is only valid while the MLS Device it came from stays where
/// the fold left it. If an older image ran MLS chat again (a rollback), the
/// room moved on without the log: refuse rather than serve a log that misses
/// those messages. Moving `direct/` aside makes the next start fold again.
fn refuse_if_mls_advanced_after_fold(home: &AgentHome) -> Result<(), CliError> {
    if !home.dir.join(STORE_FILE).is_file() {
        return Ok(());
    }
    let recorded = {
        let store = DirectStore::open(&DirectStore::path(&home.dir))?;
        if store.meta("origin")?.as_deref() != Some("fold") {
            return Ok(());
        }
        store
            .meta("fold_cursors")?
            .and_then(|value| serde_json::from_str::<BTreeMap<String, u64>>(&value).ok())
            .unwrap_or_default()
    };
    let runtime = FiniteChatRuntime::open_for_class(
        agent_runtime_open_options(home),
        finitechat_core::CommandClass::ReadOnly,
    )?;
    let advanced = runtime
        .room_sync_cursors()?
        .into_iter()
        .filter(|cursor| {
            recorded
                .get(&cursor.room_id)
                .is_some_and(|folded| cursor.last_applied_seq > *folded)
        })
        .map(|cursor| cursor.room_id)
        .collect::<Vec<_>>();
    if advanced.is_empty() {
        Ok(())
    } else {
        Err(CliError::Hermes(format!(
            "direct log refused: MLS chat advanced after the fold in {}; move {}/{DIRECT_DIR} aside to fold again",
            advanced.join(", "),
            home.dir.display()
        )))
    }
}

/// Sync the MLS Device until the server has nothing newer, then export.
fn final_sync_and_export(home: &AgentHome) -> Result<DirectFoldExport, CliError> {
    let runtime = open_agent_runtime(home)?;
    runtime.dispatch_and_wait(AppAction::StartRuntime)?;
    for _ in 0..FOLD_SYNC_ROUNDS {
        let sync = runtime.agent_bridge_poll_once()?;
        if sync.events.is_empty() {
            break;
        }
    }
    let behind = runtime
        .room_sync_cursors()?
        .into_iter()
        .filter(|cursor| cursor.behind_server.is_some())
        .map(|cursor| cursor.room_id)
        .collect::<Vec<_>>();
    if !behind.is_empty() {
        return Err(CliError::Hermes(format!(
            "direct fold refused: rooms are behind the chat server: {}",
            behind.join(", ")
        )));
    }
    Ok(runtime.export_direct_fold()?)
}

fn write_fold(
    store: &DirectStore,
    home: &AgentHome,
    export: &DirectFoldExport,
) -> Result<Value, CliError> {
    let agent_account = &home.config.account_id;
    let mut humans = BTreeSet::new();
    let mut rooms = Vec::new();
    let mut event_count = 0usize;
    for room in &export.rooms {
        if !room.connected {
            continue;
        }
        for member in &room.member_account_ids {
            if member != agent_account {
                humans.insert(member.clone());
            }
        }
        let max_seq = room
            .events
            .iter()
            .map(|event| event.seq)
            .max()
            .unwrap_or_default()
            .max(room.last_applied_seq);
        store.insert_room(
            &DirectRoom {
                room_id: room.room_id.clone(),
                display_name: room.display_name.clone(),
                picture: room.picture.clone(),
            },
            max_seq + 1,
            max_seq,
        )?;
        for member in &room.member_account_ids {
            store.insert_member(&room.room_id, member)?;
        }
        for event in &room.events {
            store.insert_event(event, "fold", None)?;
        }
        event_count += room.events.len();
        rooms.push(json!({
            "room_id": room.room_id,
            "events": room.events.len(),
            "last_applied_seq": room.last_applied_seq,
            "members": room.member_account_ids.len(),
        }));
    }
    // The log projects for one person. A room shared by several people
    // needs an explicit owner from Core; fail closed rather than pick one.
    let owner = match humans.len() {
        0 => String::new(),
        1 => humans.into_iter().next().unwrap_or_default(),
        _ => {
            let owners = owner_npubs_from_env();
            let mut matching = humans.intersection(&owners);
            match (matching.next(), matching.next()) {
                (Some(owner), None) => owner.clone(),
                _ => {
                    return Err(CliError::Hermes(
                        "direct fold refused: the room has several people and no single owner"
                            .to_owned(),
                    ));
                }
            }
        }
    };
    store.set_meta("schema_version", &DIRECT_SCHEMA_VERSION.to_string())?;
    store.set_meta("origin", "fold")?;
    store.set_meta("created_at", &now_unix_seconds().to_string())?;
    store.set_meta("owner_account_id", &owner)?;
    let cursors = export
        .rooms
        .iter()
        .map(|room| (room.room_id.clone(), room.last_applied_seq))
        .collect::<BTreeMap<_, _>>();
    store.set_meta(
        "fold_cursors",
        &serde_json::to_string(&cursors).map_err(CliError::Serialize)?,
    )?;
    for profile in &export.profiles {
        store.upsert_profile(profile)?;
    }
    Ok(json!({
        "origin": "fold",
        "source": "agent-store",
        "owner_account_id": owner,
        "rooms": rooms,
        "summary": format!("{} rooms, {event_count} events", export.rooms.len()),
    }))
}

fn write_fresh_room(store: &DirectStore, home: &AgentHome) -> Result<Value, CliError> {
    let room_id = format!("room-{}", random_hex::<8>()?);
    let display_name = std::env::var(ROOM_NAME_ENV)
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Agent".to_owned());
    store.insert_room(
        &DirectRoom {
            room_id: room_id.clone(),
            display_name,
            picture: None,
        },
        1,
        0,
    )?;
    store.insert_member(&room_id, &home.config.account_id)?;
    let owners = owner_npubs_from_env();
    let owner = if owners.len() == 1 {
        owners.into_iter().next().unwrap_or_default()
    } else {
        String::new()
    };
    if !owner.is_empty() {
        store.insert_member(&room_id, &owner)?;
    }
    store.set_meta("schema_version", &DIRECT_SCHEMA_VERSION.to_string())?;
    store.set_meta("origin", "fresh")?;
    store.set_meta("created_at", &now_unix_seconds().to_string())?;
    store.set_meta("owner_account_id", &owner)?;
    // Home exists from the start, as on a paired Device.
    let owner_ref = DeviceRef {
        account_id: owner.clone(),
        device_id: DIRECT_WEB_DEVICE_ID.to_owned(),
    };
    let agent = DeviceRef {
        account_id: home.config.account_id.clone(),
        device_id: home.config.device_id.clone(),
    };
    let mut engine = DirectRoomEngine::new(owner_ref, agent);
    engine.upsert_room(DirectRoom {
        room_id: room_id.clone(),
        display_name: String::new(),
        picture: None,
    });
    engine
        .ensure_home_topic(&room_id, &mut StoreLog { store })
        .map_err(CliError::from)?;
    Ok(json!({
        "origin": "fresh",
        "source": null,
        "owner_account_id": owner,
        "rooms": [{ "room_id": room_id }],
        "summary": "new room",
    }))
}

/// Records why a direct fold did not run, for operators and `/readyz`.
pub(super) fn record_fold_failure(home_dir: &Path, error: &CliError) {
    let dir = direct_dir(home_dir);
    if fs::create_dir_all(&dir).is_ok() {
        let _ = write_private(
            dir.join(DIRECT_FOLD_STATUS_FILE),
            json!({
                "origin": "pending",
                "error": error.to_string(),
                "at_unix_seconds": now_unix_seconds(),
            })
            .to_string(),
        );
    }
    eprintln!("finitechat direct transport: fold pending, serving MLS chat: {error}");
}

pub(super) fn read_fold_status(home_dir: &Path) -> Option<Value> {
    fs::read(direct_dir(home_dir).join(DIRECT_FOLD_STATUS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

// ---- app API -------------------------------------------------------------

#[derive(Debug)]
enum AppApiError {
    BadRequest(String),
    Unauthorized(&'static str),
    Forbidden(&'static str),
    NotFound(&'static str),
    Gone(String),
    TooLarge(String),
    Timeout(String),
    Unavailable(String),
    Internal(String),
    Core(String),
}

impl AppApiError {
    fn from_cli(error: CliError) -> Self {
        match error {
            CliError::Core(error) => Self::Core(error.to_string()),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl IntoResponse for AppApiError {
    fn into_response(self) -> Response {
        let (status, kind, retryable, message) = match self {
            Self::BadRequest(message) => {
                (StatusCode::BAD_REQUEST, "invalid_request", false, message)
            }
            Self::Core(message) => (StatusCode::BAD_REQUEST, "client", false, message),
            Self::Unauthorized(message) => (
                StatusCode::UNAUTHORIZED,
                "chat_authorization_invalid",
                false,
                message.to_owned(),
            ),
            Self::Forbidden(message) => (
                StatusCode::FORBIDDEN,
                "forbidden",
                false,
                message.to_owned(),
            ),
            Self::NotFound(message) => (
                StatusCode::NOT_FOUND,
                "not_found",
                false,
                message.to_owned(),
            ),
            Self::Gone(message) => (StatusCode::GONE, "attachment_unavailable", false, message),
            Self::TooLarge(message) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                false,
                message,
            ),
            Self::Timeout(message) => (StatusCode::GATEWAY_TIMEOUT, "agent_timeout", true, message),
            Self::Unavailable(message) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                true,
                message,
            ),
            Self::Internal(message) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal", true, message)
            }
        };
        (
            status,
            Json(json!({ "error": message, "error_kind": kind, "retryable": retryable })),
        )
            .into_response()
    }
}

type AppResult<T> = Result<T, AppApiError>;

fn direct_runtime(state: &HermesServiceState) -> AppResult<&DirectRuntime> {
    state
        .runtime
        .as_direct()
        .ok_or(AppApiError::NotFound("direct chat is not enabled"))
}

pub(super) fn direct_app_router() -> Router<HermesServiceState> {
    Router::new()
        .route("/v1/app/info", get(app_info))
        .route("/v1/app/state", get(app_state_route))
        .route("/v1/app/updates", get(app_updates_route))
        .route("/v1/app/actions", post(app_actions_route))
        .route("/v1/app/new-chat", post(app_new_chat_route))
        .route("/v1/app/attachments", post(app_attachments_upload_route))
        .route(
            "/v1/app/attachments/{room_id}/{message_id}/{attachment_id}",
            get(app_attachment_route),
        )
        .route("/v1/app/runtime-commands", post(app_runtime_commands_route))
        .route("/v1/app/digest", get(app_digest_route))
}

fn path_and_query(uri: &axum::http::Uri) -> String {
    uri.path_and_query()
        .map(|value| value.as_str().to_owned())
        .unwrap_or_else(|| uri.path().to_owned())
}

async fn read_body(
    request: AxumRequest,
    limit: usize,
) -> AppResult<(axum::http::request::Parts, Bytes)> {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, limit).await.map_err(|_| {
        AppApiError::TooLarge(format!("request body must be at most {limit} bytes"))
    })?;
    Ok((parts, bytes))
}

async fn app_info(State(state): State<HermesServiceState>) -> AppResult<Json<Value>> {
    let runtime = direct_runtime(&state)?;
    runtime
        .info()
        .map(Json)
        .map_err(|error| AppApiError::Internal(error.to_string()))
}

fn authorized_get(
    state: &HermesServiceState,
    headers: &HeaderMap,
    uri: &axum::http::Uri,
) -> AppResult<()> {
    direct_runtime(state)?.authorize(headers, "GET", &path_and_query(uri), &[])?;
    Ok(())
}

fn canonical_view(runtime: &DirectRuntime, mut view: AppView) -> AppResult<AppView> {
    if view.room_id.is_none()
        && (view.topic_id.is_some() || view.chat_id.is_some() || view.oldest_message_id.is_some())
    {
        view.room_id = runtime
            .single_room_id()
            .map_err(|error| AppApiError::Internal(error.to_string()))?;
    }
    Ok(view)
}

async fn app_state_route(
    State(state): State<HermesServiceState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Query(view): Query<AppView>,
) -> AppResult<Json<AppState>> {
    authorized_get(&state, &headers, &uri)?;
    let runtime = direct_runtime(&state)?;
    let view = canonical_view(runtime, view)?;
    runtime
        .app_state(view, "ready")
        .map(Json)
        .map_err(AppApiError::from_cli)
}

async fn app_updates_route(
    State(state): State<HermesServiceState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Query(view): Query<AppView>,
) -> AppResult<Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>>> {
    authorized_get(&state, &headers, &uri)?;
    let view = canonical_view(direct_runtime(&state)?, view)?;
    let runtime = Arc::clone(&state.runtime);
    let stream = stream::unfold((runtime, view, true), |(runtime, view, first)| async move {
        let next_runtime = Arc::clone(&runtime);
        let next_view = view.clone();
        let event = tokio::task::spawn_blocking(move || {
            let direct = next_runtime.as_direct()?;
            if !first {
                let observed = direct.generation();
                direct.wait_for_change(observed, APP_UPDATE_WAIT_MILLIS);
            }
            Some(direct.app_state(next_view, "ready"))
        })
        .await;
        let event = match event {
            Ok(Some(Ok(app))) => serde_json::to_string(&app)
                .map(|data| {
                    SseEvent::default()
                        .event("state")
                        .id(app.rev.to_string())
                        .data(data)
                })
                .unwrap_or_else(|error| sse_error(error.to_string())),
            Ok(Some(Err(error))) => sse_error(error.to_string()),
            Ok(None) => return None,
            Err(error) => sse_error(error.to_string()),
        };
        Some((Ok(event), (runtime, view, false)))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn sse_error(message: String) -> SseEvent {
    SseEvent::default()
        .event("error")
        .data(json!({ "error": message }).to_string())
}

fn requester_from_headers(headers: &HeaderMap) -> AppResult<Option<DirectRequesterContext>> {
    let read = |name: &str| {
        headers
            .get(name)
            .map(|value| value.to_str().map(str::trim).map(str::to_owned))
            .transpose()
            .map_err(|_| AppApiError::BadRequest("requester headers are invalid".to_owned()))
    };
    match (
        read(REQUESTER_EMAIL_HEADER)?,
        read(SITES_REQUESTER_ASSERTION_HEADER)?,
    ) {
        (None, None) => Ok(None),
        (Some(email), Some(sites_assertion))
            if !email.is_empty()
                && email.len() <= 320
                && !sites_assertion.is_empty()
                && sites_assertion.len() <= 512
                && !email.chars().any(char::is_control)
                && !sites_assertion.chars().any(char::is_control) =>
        {
            Ok(Some(DirectRequesterContext {
                email,
                sites_assertion,
            }))
        }
        _ => Err(AppApiError::BadRequest(
            "requester headers are invalid".to_owned(),
        )),
    }
}

async fn app_actions_route(
    State(state): State<HermesServiceState>,
    request: AxumRequest,
) -> AppResult<Json<AppState>> {
    let (parts, body) = read_body(request, MAX_APP_JSON_BODY_BYTES).await?;
    let runtime = direct_runtime(&state)?;
    runtime.authorize(&parts.headers, "POST", &path_and_query(&parts.uri), &body)?;
    let action: AppAction = serde_json::from_slice(&body)
        .map_err(|error| AppApiError::BadRequest(format!("invalid action: {error}")))?;
    let requester = requester_from_headers(&parts.headers)?;
    let runtime = Arc::clone(&state.runtime);
    let state = tokio::task::spawn_blocking(move || -> Result<AppState, AppApiError> {
        let direct = runtime
            .as_direct()
            .ok_or(AppApiError::NotFound("direct chat is not enabled"))?;
        let selection = direct
            .mutate(|engine, log| {
                let selection = engine.execute(action, requester.as_ref(), log)?;
                if let Some(room_id) = &selection.room_id
                    && selection.status == "read"
                {
                    let seq = engine.read_seq(room_id);
                    log.store.set_read_seq(room_id, seq).map_err(core_error)?;
                }
                Ok(selection)
            })
            .map_err(AppApiError::from_cli)?;
        let view = direct.view_for_selection(&selection, None);
        direct
            .app_state(view, &selection.status)
            .map_err(AppApiError::from_cli)
    })
    .await
    .map_err(|error| AppApiError::Internal(error.to_string()))??;
    Ok(Json(state))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewChatInput {
    #[serde(default)]
    project_id: Option<String>,
    room_id: String,
    topic_id: String,
    #[serde(default)]
    reason: Option<String>,
    intent_key: String,
}

async fn app_new_chat_route(
    State(state): State<HermesServiceState>,
    request: AxumRequest,
) -> AppResult<Json<AppState>> {
    let (parts, body) = read_body(request, MAX_APP_JSON_BODY_BYTES).await?;
    let runtime = direct_runtime(&state)?;
    runtime.authorize(&parts.headers, "POST", &path_and_query(&parts.uri), &body)?;
    let input: NewChatInput = serde_json::from_slice(&body)
        .map_err(|error| AppApiError::BadRequest(format!("invalid new chat: {error}")))?;
    let _ = input.project_id;
    let runtime = Arc::clone(&state.runtime);
    let state = tokio::task::spawn_blocking(move || -> Result<AppState, AppApiError> {
        let direct = runtime
            .as_direct()
            .ok_or(AppApiError::NotFound("direct chat is not enabled"))?;
        let selection = direct
            .mutate(|engine, log| {
                engine.new_chat(
                    log,
                    &input.room_id,
                    &input.topic_id,
                    input.reason,
                    &input.intent_key,
                )
            })
            .map_err(AppApiError::from_cli)?;
        let view = direct.view_for_selection(&selection, None);
        direct
            .app_state(view, &selection.status)
            .map_err(AppApiError::from_cli)
    })
    .await
    .map_err(|error| AppApiError::Internal(error.to_string()))??;
    Ok(Json(state))
}

#[derive(Default)]
struct UploadForm {
    room_id: Option<String>,
    topic_id: Option<String>,
    chat_id: Option<String>,
    caption: Option<String>,
    reply_to_message_id: Option<String>,
    attachments: Vec<OutboundAttachment>,
    total_bytes: usize,
}

async fn app_attachments_upload_route(
    State(state): State<HermesServiceState>,
    request: AxumRequest,
) -> AppResult<Json<AppState>> {
    let (parts, body) = read_body(request, MAX_MULTIPART_BODY_BYTES).await?;
    let runtime = direct_runtime(&state)?;
    runtime.authorize(&parts.headers, "POST", &path_and_query(&parts.uri), &body)?;
    let rebuilt = AxumRequest::from_parts(parts, axum::body::Body::from(body));
    let mut multipart = Multipart::from_request(rebuilt, &())
        .await
        .map_err(|_| AppApiError::BadRequest("missing or invalid multipart boundary".to_owned()))?;
    let mut form = UploadForm::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppApiError::BadRequest(error.body_text()))?
    {
        let name = field.name().unwrap_or_default().to_owned();
        match name.as_str() {
            "file" | "files" | "attachments" => {
                if form.attachments.len() >= MAX_ATTACHMENTS_PER_MESSAGE {
                    return Err(AppApiError::TooLarge(format!(
                        "at most {MAX_ATTACHMENTS_PER_MESSAGE} files are allowed"
                    )));
                }
                let filename = normalize_upload_filename(field.file_name())?;
                let mime_type = normalize_upload_mime_type(field.content_type());
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|error| AppApiError::BadRequest(error.body_text()))?;
                if bytes.len() > MAX_ATTACHMENT_BYTES {
                    return Err(AppApiError::TooLarge(format!(
                        "each file must be at most {MAX_ATTACHMENT_BYTES} bytes"
                    )));
                }
                form.total_bytes += bytes.len();
                if form.total_bytes > MAX_ATTACHMENT_TOTAL_BYTES {
                    return Err(AppApiError::TooLarge(format!(
                        "files must total at most {MAX_ATTACHMENT_TOTAL_BYTES} bytes"
                    )));
                }
                form.attachments.push(OutboundAttachment {
                    kind: upload_kind_for_mime(&mime_type),
                    filename,
                    mime_type,
                    bytes: bytes.to_vec(),
                });
            }
            "room_id" | "topic_id" | "chat_id" | "caption" | "reply_to_message_id" => {
                let value = field
                    .text()
                    .await
                    .map_err(|error| AppApiError::BadRequest(error.body_text()))?;
                let slot = match name.as_str() {
                    "room_id" => &mut form.room_id,
                    "topic_id" => &mut form.topic_id,
                    "chat_id" => &mut form.chat_id,
                    "caption" => &mut form.caption,
                    _ => &mut form.reply_to_message_id,
                };
                if slot.replace(value).is_some() {
                    return Err(AppApiError::BadRequest(format!("{name} must appear once")));
                }
            }
            _ => {
                return Err(AppApiError::BadRequest(format!("unexpected field {name}")));
            }
        }
    }
    let room_id = form
        .room_id
        .clone()
        .ok_or_else(|| AppApiError::BadRequest("room_id is required".to_owned()))?;
    if form.attachments.is_empty() {
        return Err(AppApiError::BadRequest(
            "at least one file is required".to_owned(),
        ));
    }
    let runtime = Arc::clone(&state.runtime);
    let state = tokio::task::spawn_blocking(move || -> Result<AppState, AppApiError> {
        let direct = runtime
            .as_direct()
            .ok_or(AppApiError::NotFound("direct chat is not enabled"))?;
        let mut stored = Vec::new();
        for attachment in form.attachments {
            stored.push(
                direct
                    .store_attachment(attachment)
                    .map_err(AppApiError::from_cli)?,
            );
        }
        let caption = form.caption.unwrap_or_default();
        let selection = direct
            .mutate(|engine, log| {
                engine.send_attachments(
                    log,
                    &room_id,
                    form.topic_id.as_deref(),
                    form.chat_id.as_deref(),
                    &caption,
                    stored,
                    form.reply_to_message_id.as_deref(),
                )
            })
            .map_err(AppApiError::from_cli)?;
        let view = direct.view_for_selection(&selection, None);
        direct
            .app_state(view, &selection.status)
            .map_err(AppApiError::from_cli)
    })
    .await
    .map_err(|error| AppApiError::Internal(error.to_string()))??;
    Ok(Json(state))
}

fn normalize_upload_filename(value: Option<&str>) -> AppResult<String> {
    let filename = value
        .ok_or_else(|| AppApiError::BadRequest("each file must include a filename".to_owned()))?
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    if filename.is_empty() || filename.len() > 255 || filename.chars().any(char::is_control) {
        return Err(AppApiError::BadRequest(
            "attachment filename is invalid".to_owned(),
        ));
    }
    Ok(filename.to_owned())
}

fn normalize_upload_mime_type(value: Option<&str>) -> String {
    let value = value.unwrap_or_default().trim().to_ascii_lowercase();
    let mut parts = value.split('/');
    let token = |token: &str| {
        !token.is_empty()
            && token.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                    )
            })
    };
    let (top, sub) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    if value.len() <= 255 && parts.next().is_none() && token(top) && token(sub) {
        value
    } else {
        "application/octet-stream".to_owned()
    }
}

fn upload_kind_for_mime(mime_type: &str) -> ChatMediaKind {
    if mime_type.starts_with("image/") {
        ChatMediaKind::Image
    } else if mime_type.starts_with("video/") {
        ChatMediaKind::Video
    } else if mime_type.starts_with("audio/") {
        ChatMediaKind::VoiceNote
    } else {
        ChatMediaKind::File
    }
}

async fn app_attachment_route(
    State(state): State<HermesServiceState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    AxumPath((room_id, message_id, attachment_id)): AxumPath<(String, String, String)>,
) -> AppResult<Response> {
    authorized_get(&state, &headers, &uri)?;
    let runtime = Arc::clone(&state.runtime);
    let (reference, bytes) = tokio::task::spawn_blocking(move || {
        runtime
            .as_direct()
            .ok_or(AppApiError::NotFound("direct chat is not enabled"))?
            .attachment_bytes(&room_id, &message_id, &attachment_id)
    })
    .await
    .map_err(|error| AppApiError::Internal(error.to_string()))??;
    let mime_type = reference.metadata.mime_type.as_str();
    let inline = matches!(
        mime_type,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "audio/mpeg"
            | "audio/mp4"
            | "audio/ogg"
            | "audio/wav"
            | "audio/webm"
            | "video/mp4"
            | "video/ogg"
            | "video/webm"
            | "video/quicktime"
    );
    let filename = safe_download_filename(&reference.metadata.filename);
    let disposition = format!(
        "{}; filename=\"{filename}\"",
        if inline { "inline" } else { "attachment" }
    );
    let content_type = if inline {
        mime_type
    } else {
        "application/octet-stream"
    };
    Ok((
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, "private, no-store".to_owned()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            (header::CONTENT_SECURITY_POLICY, "sandbox".to_owned()),
        ],
        bytes,
    )
        .into_response())
}

fn safe_download_filename(filename: &str) -> String {
    let safe = filename
        .chars()
        .take(128)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let safe = safe.trim_matches([' ', '.']).to_owned();
    if safe.is_empty() {
        "attachment".to_owned()
    } else {
        safe
    }
}

#[derive(Debug, Deserialize)]
struct RuntimeCommandInput {
    room_id: String,
    #[serde(default)]
    conversation_id: Option<String>,
    target_account_id: String,
    command: String,
    #[serde(default)]
    resource_key: Option<String>,
    schema: String,
    body: Value,
    #[serde(default)]
    reuse_succeeded_owner_claim: bool,
    #[serde(default = "default_runtime_command_wait")]
    wait_millis: u64,
}

fn default_runtime_command_wait() -> u64 {
    DEFAULT_RUNTIME_COMMAND_WAIT_MILLIS
}

#[derive(Debug, Serialize)]
struct RuntimeCommandOutput {
    request_id: String,
    status: RuntimeCommandTerminalStatusV1,
    body: Option<Value>,
    error: Option<finitechat_proto::RuntimeCommandErrorV1>,
}

fn runtime_command_output(result: RuntimeCommandResultV1) -> AppResult<RuntimeCommandOutput> {
    let body = result
        .body
        .as_ref()
        .map(|body| serde_json::from_slice::<Value>(&body.json_payload))
        .transpose()
        .map_err(|error| AppApiError::Internal(error.to_string()))?;
    Ok(RuntimeCommandOutput {
        request_id: result.request_id,
        status: result.status,
        body,
        error: result.error,
    })
}

async fn app_runtime_commands_route(
    State(state): State<HermesServiceState>,
    request: AxumRequest,
) -> AppResult<Json<RuntimeCommandOutput>> {
    let (parts, body) = read_body(request, MAX_APP_JSON_BODY_BYTES).await?;
    let runtime = direct_runtime(&state)?;
    runtime.authorize(&parts.headers, "POST", &path_and_query(&parts.uri), &body)?;
    let input: RuntimeCommandInput = serde_json::from_slice(&body)
        .map_err(|error| AppApiError::BadRequest(format!("invalid runtime command: {error}")))?;
    let runtime = Arc::clone(&state.runtime);
    let output = tokio::task::spawn_blocking(move || {
        runtime
            .as_direct()
            .ok_or(AppApiError::NotFound("direct chat is not enabled"))?
            .send_runtime_command(input)
    })
    .await
    .map_err(|error| AppApiError::Internal(error.to_string()))??;
    Ok(Json(output))
}

#[derive(Debug, Deserialize)]
struct DigestQuery {
    room_id: Option<String>,
}

async fn app_digest_route(
    State(state): State<HermesServiceState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Query(query): Query<DigestQuery>,
) -> AppResult<Json<Value>> {
    authorized_get(&state, &headers, &uri)?;
    let runtime = direct_runtime(&state)?;
    let room_id = match query.room_id {
        Some(room_id) => room_id,
        None => runtime
            .single_room_id()
            .map_err(|error| AppApiError::Internal(error.to_string()))?
            .ok_or_else(|| AppApiError::BadRequest("room_id is required".to_owned()))?,
    };
    let digest = runtime
        .lock()
        .map_err(|error| AppApiError::Internal(error.to_string()))?
        .engine
        .digest(&room_id);
    serde_json::to_value(digest)
        .map(Json)
        .map_err(|error| AppApiError::Internal(error.to_string()))
}

/// Operator digest of the direct log, for comparing with the hosted device.
pub(super) fn cmd_direct_digest<W: Write>(home_dir: &Path, output: &mut W) -> Result<(), CliError> {
    let home = load_home(home_dir)?;
    if !DirectStore::exists(home_dir) {
        return Err(CliError::Hermes(
            "no direct room log on this agent".to_owned(),
        ));
    }
    let runtime = DirectRuntime::open(&home)?;
    let inner = runtime.lock()?;
    let digests = inner
        .engine
        .room_ids()
        .into_iter()
        .map(|room_id| inner.engine.digest(&room_id))
        .collect::<Vec<_>>();
    crate::write_pretty_json(
        output,
        &json!({
            "transport": "direct",
            "origin": runtime.origin,
            "owner_account_id": inner.engine.owner().account_id,
            "rooms": digests,
        }),
    )
}
