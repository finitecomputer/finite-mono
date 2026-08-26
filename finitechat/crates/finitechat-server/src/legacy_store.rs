//! SQLite-backed durable delivery store: op log, snapshots, and replay.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Mutex, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

use finitechat_delivery::{
    HttpDeliveryService, HttpKeyPackageId, HttpKeyPackagePublication, HttpPublishTarget,
};
use finitechat_http::{
    HttpApplicationDeliveryEffect, HttpPairingSessionRecord, NostrProfileRecord, PushTokenRecord,
};
use finitechat_proto::{DeviceMembership, DeviceRef};
use finitechat_transport::engine::KeyPackage;
use finitechat_transport::{MemberId, MessageId};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::projections::HttpRoomMembershipProjection;
use crate::state::{
    AccountRoomDirectoryMutation, AccountRoomDirectoryRecord, BlobBackend, BlobMeta,
    DurableStateSnapshot, KeyPackageClaimIdempotencyRecord, KeyPackageInventoryRecord,
    KeyPackageInventoryState, PublishIdempotencyRecord, PublishMutation, PushWakeOutboxRecord,
    WelcomeClaimRecord, consume_key_packages_from_persisted_message, finite_key_package_metadata,
    mark_next_key_package_claimed, retire_older_finite_key_packages,
};
use crate::{DurableStoreError, SNAPSHOT_ZSTD_LEVEL};

const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const READINESS_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(5);

pub(crate) fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PersistedOperation {
    PublishMessage {
        target: HttpPublishTarget,
        message: finitechat_transport::transport::TransportMessage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        idempotency_key: Option<String>,
    },
    PublishKeyPackage {
        publication: HttpKeyPackagePublication,
    },
    RevokeDevice {
        device: DeviceRef,
    },
    ClaimKeyPackage {
        owner: MemberId,
    },
    ClaimKeyPackages {
        owners: Vec<MemberId>,
    },
    ExpireKeyPackageLease {
        key_package_id: HttpKeyPackageId,
    },
}

impl PersistedOperation {
    fn kind(&self) -> &'static str {
        match self {
            Self::PublishMessage { .. } => "publish_message",
            Self::PublishKeyPackage { .. } => "publish_key_package",
            Self::RevokeDevice { .. } => "revoke_device",
            Self::ClaimKeyPackage { .. } => "claim_key_package",
            Self::ClaimKeyPackages { .. } => "claim_key_packages",
            Self::ExpireKeyPackageLease { .. } => "expire_key_package_lease",
        }
    }
}

#[derive(Debug)]
pub(crate) struct SqliteHttpDeliveryStore {
    conn: Mutex<Connection>,
}

impl SqliteHttpDeliveryStore {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, rusqlite::Error> {
        let store = Self {
            conn: Mutex::new(Connection::open(path.as_ref())?),
        };
        let conn = store.connection();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
            PRAGMA synchronous = FULL;
            PRAGMA busy_timeout = 5000;
            CREATE TABLE IF NOT EXISTS http_delivery_ops (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                body_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_push_tokens (
                device_key TEXT PRIMARY KEY,
                record_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_push_wakes (
                wake_id TEXT PRIMARY KEY,
                record_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_state_snapshots (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                last_op_seq INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_state_snapshots_v2 (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                last_op_seq INTEGER NOT NULL,
                snapshot_zstd BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_publish_idempotency (
                idempotency_key TEXT PRIMARY KEY,
                fingerprint_json TEXT NOT NULL,
                receipt_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_key_package_claim_idempotency (
                idempotency_key TEXT PRIMARY KEY,
                fingerprint_json TEXT NOT NULL,
                response_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_key_package_inventory (
                key_package_id_json TEXT PRIMARY KEY,
                owner_json TEXT NOT NULL,
                state_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_pairing_sessions (
                pairing_session_id TEXT PRIMARY KEY,
                record_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_nostr_profiles (
                account_id TEXT PRIMARY KEY,
                record_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_account_rooms (
                account_id TEXT NOT NULL,
                room_id TEXT NOT NULL,
                record_json TEXT NOT NULL,
                PRIMARY KEY(account_id, room_id)
            );
            CREATE TABLE IF NOT EXISTS http_room_memberships (
                room_id TEXT PRIMARY KEY,
                projection_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_application_delivery_effects (
                message_id TEXT PRIMARY KEY,
                room_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                sender_json TEXT NOT NULL,
                delivery_policy_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_welcome_claims (
                message_id_json TEXT PRIMARY KEY,
                recipient_json TEXT NOT NULL,
                seq INTEGER NOT NULL,
                message_json TEXT NOT NULL,
                state_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_blob_objects (
                sha256 TEXT PRIMARY KEY,
                size_bytes INTEGER NOT NULL,
                content_type TEXT NOT NULL,
                ciphertext BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_blob_meta (
                sha256 TEXT PRIMARY KEY,
                size_bytes INTEGER NOT NULL,
                content_type TEXT NOT NULL,
                backend TEXT NOT NULL CHECK (backend IN ('sqlite','object')),
                created_at_ms INTEGER NOT NULL,
                migrated_at_ms INTEGER
            );
            CREATE TABLE IF NOT EXISTS http_readiness_probe (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                checked_at_ms INTEGER NOT NULL
            );",
        )?;
        ensure_blob_content_type_column(&conn)?;
        // Databases written before http_blob_meta existed carry payload rows
        // only; give each a meta row so boot never has to touch payloads.
        conn.execute(
            "INSERT OR IGNORE INTO http_blob_meta
                 (sha256, size_bytes, content_type, backend, created_at_ms)
             SELECT sha256, size_bytes, content_type, 'sqlite', ?1
             FROM http_blob_objects",
            params![unix_now_ms()],
        )?;
        drop(conn);
        Ok(store)
    }

    /// Commit and read back one service-owned row through the same SQLite
    /// connection and durability settings as user delivery writes.
    ///
    /// The row is health evidence, not Room or Device state. Keeping it in a
    /// dedicated singleton table makes the probe idempotent and ensures it can
    /// never appear in a user's encrypted delivery history.
    pub(crate) fn probe_readiness(&self, budget: Duration) -> Result<(), &'static str> {
        let started = Instant::now();
        let mut conn = loop {
            match self.conn.try_lock() {
                Ok(conn) => break conn,
                Err(TryLockError::WouldBlock) if started.elapsed() < budget => {
                    thread::sleep(READINESS_LOCK_POLL_INTERVAL);
                }
                Err(TryLockError::WouldBlock) => return Err("connection_contended"),
                Err(TryLockError::Poisoned(_)) => return Err("connection_lock_poisoned"),
            }
        };

        let remaining = budget.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("connection_contended");
        }
        if let Err(error) = conn.busy_timeout(remaining) {
            eprintln!("finitechat-server: readiness could not set SQLite timeout: {error}");
            return Err("timeout_configuration_failed");
        }

        let checked_at_ms = unix_now_ms();
        let write_result = (|| -> Result<i64, rusqlite::Error> {
            let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let _delivery_head: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM http_delivery_ops",
                [],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT INTO http_readiness_probe (id, checked_at_ms)
                 VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE
                 SET checked_at_ms = excluded.checked_at_ms",
                params![checked_at_ms],
            )?;
            transaction.commit()?;
            conn.query_row(
                "SELECT checked_at_ms FROM http_readiness_probe WHERE id = 1",
                [],
                |row| row.get(0),
            )
        })();
        let restore_result = conn.busy_timeout(SQLITE_BUSY_TIMEOUT);

        let observed = match write_result {
            Ok(observed) => observed,
            Err(error) => {
                eprintln!("finitechat-server: readiness SQLite commit failed: {error}");
                return Err("commit_failed");
            }
        };
        if let Err(error) = restore_result {
            eprintln!("finitechat-server: readiness could not restore SQLite timeout: {error}");
            return Err("timeout_restore_failed");
        }
        if observed != checked_at_ms {
            eprintln!(
                "finitechat-server: readiness SQLite read-back mismatch (wrote {checked_at_ms}, observed {observed})"
            );
            return Err("read_back_mismatch");
        }
        Ok(())
    }

    pub(crate) fn append_operation(
        &self,
        operation: &PersistedOperation,
    ) -> Result<(), DurableStoreError> {
        let body_json = serde_json::to_string(operation)?;
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
            params![operation.kind(), body_json],
        )?;
        Ok(())
    }

    pub(crate) fn append_publish_mutation(
        &self,
        operation: Option<&PersistedOperation>,
        idempotency: Option<(&str, &PublishIdempotencyRecord)>,
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        if let Some(operation) = operation {
            transaction.execute(
                "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
                params![operation.kind(), serde_json::to_string(operation)?],
            )?;
        }
        if let Some((idempotency_key, record)) = idempotency {
            transaction.execute(
                "INSERT INTO http_publish_idempotency (
                    idempotency_key,
                    fingerprint_json,
                    receipt_json
                ) VALUES (?1, ?2, ?3)",
                params![
                    idempotency_key,
                    serde_json::to_string(&record.fingerprint)?,
                    serde_json::to_string(&record.receipt)?,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn append_submit_commit_mutation(
        &self,
        publish_mutations: &[PublishMutation],
        account_room_mutation: &AccountRoomDirectoryMutation,
        room_membership_projection: &HttpRoomMembershipProjection,
        key_package_inventory_mutation: &[KeyPackageInventoryRecord],
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        for mutation in publish_mutations {
            if let Some(operation) = &mutation.operation {
                transaction.execute(
                    "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
                    params![operation.kind(), serde_json::to_string(operation)?],
                )?;
            }
            transaction.execute(
                "INSERT INTO http_publish_idempotency (
                    idempotency_key,
                    fingerprint_json,
                    receipt_json
                ) VALUES (?1, ?2, ?3)",
                params![
                    mutation.idempotency_key,
                    serde_json::to_string(&mutation.record.fingerprint)?,
                    serde_json::to_string(&mutation.record.receipt)?,
                ],
            )?;
        }
        for (account_id, room_id) in &account_room_mutation.deletes {
            transaction.execute(
                "DELETE FROM http_account_rooms WHERE account_id = ?1 AND room_id = ?2",
                params![account_id, room_id],
            )?;
        }
        for record in &account_room_mutation.upserts {
            transaction.execute(
                "INSERT INTO http_account_rooms (account_id, room_id, record_json)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, room_id) DO UPDATE SET
                    record_json = excluded.record_json",
                params![
                    record.account_id,
                    record.room_id,
                    serde_json::to_string(&record.record)?,
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO http_room_memberships (room_id, projection_json)
             VALUES (?1, ?2)
             ON CONFLICT(room_id) DO UPDATE SET
                projection_json = excluded.projection_json",
            params![
                room_membership_projection.room_id,
                serde_json::to_string(room_membership_projection)?,
            ],
        )?;
        for record in key_package_inventory_mutation {
            upsert_key_package_inventory_in_transaction(&transaction, record)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn append_application_event_mutation(
        &self,
        publish_mutation: Option<&PublishMutation>,
        room_membership_projection: Option<&HttpRoomMembershipProjection>,
        effect: Option<&HttpApplicationDeliveryEffect>,
        push_wake: Option<&PushWakeOutboxRecord>,
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        if let Some(mutation) = publish_mutation {
            if let Some(operation) = &mutation.operation {
                transaction.execute(
                    "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
                    params![operation.kind(), serde_json::to_string(operation)?],
                )?;
            }
            transaction.execute(
                "INSERT INTO http_publish_idempotency (
                    idempotency_key,
                    fingerprint_json,
                    receipt_json
                ) VALUES (?1, ?2, ?3)",
                params![
                    mutation.idempotency_key,
                    serde_json::to_string(&mutation.record.fingerprint)?,
                    serde_json::to_string(&mutation.record.receipt)?,
                ],
            )?;
        }
        if let Some(projection) = room_membership_projection {
            transaction.execute(
                "INSERT INTO http_room_memberships (room_id, projection_json)
                 VALUES (?1, ?2)
                 ON CONFLICT(room_id) DO UPDATE SET
                    projection_json = excluded.projection_json",
                params![projection.room_id, serde_json::to_string(projection)?],
            )?;
        }
        if let Some(effect) = effect {
            upsert_application_effect_in_transaction(&transaction, effect)?;
        }
        if let Some(push_wake) = push_wake {
            upsert_push_wake_in_transaction(&transaction, push_wake)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn append_key_package_claim_mutation(
        &self,
        operation: Option<&PersistedOperation>,
        idempotency: Option<(&str, &KeyPackageClaimIdempotencyRecord)>,
        inventory_records: &[KeyPackageInventoryRecord],
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        if let Some(operation) = operation {
            transaction.execute(
                "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
                params![operation.kind(), serde_json::to_string(operation)?],
            )?;
        }
        if let Some((idempotency_key, record)) = idempotency {
            transaction.execute(
                "INSERT INTO http_key_package_claim_idempotency (
                    idempotency_key,
                    fingerprint_json,
                    response_json
                ) VALUES (?1, ?2, ?3)",
                params![
                    idempotency_key,
                    serde_json::to_string(&record.fingerprint)?,
                    serde_json::to_string(&record.response)?,
                ],
            )?;
        }
        for record in inventory_records {
            upsert_key_package_inventory_in_transaction(&transaction, record)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn append_key_package_inventory_operation(
        &self,
        operation: &PersistedOperation,
        inventory_record: &KeyPackageInventoryRecord,
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        transaction.execute(
            "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
            params![operation.kind(), serde_json::to_string(operation)?],
        )?;
        upsert_key_package_inventory_in_transaction(&transaction, inventory_record)?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn load_operations_after(
        &self,
        after_seq: i64,
    ) -> Result<Vec<PersistedOperation>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn
            .prepare("SELECT body_json FROM http_delivery_ops WHERE seq > ?1 ORDER BY seq ASC")?;
        let rows = statement.query_map(params![after_seq], |row| row.get::<_, String>(0))?;
        let mut operations = Vec::new();
        for row in rows {
            operations.push(serde_json::from_str(&row?)?);
        }
        Ok(operations)
    }

    pub(crate) fn max_operation_seq(&self) -> Result<i64, DurableStoreError> {
        let conn = self.connection();
        let max: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM http_delivery_ops",
            [],
            |row| row.get(0),
        )?;
        Ok(max)
    }

    pub(crate) fn load_push_tokens(
        &self,
    ) -> Result<BTreeMap<String, PushTokenRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare("SELECT device_key, record_json FROM http_push_tokens")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut tokens = BTreeMap::new();
        for row in rows {
            let (key, json) = row?;
            tokens.insert(key, serde_json::from_str(&json)?);
        }
        Ok(tokens)
    }

    pub(crate) fn load_push_wakes(
        &self,
    ) -> Result<BTreeMap<String, PushWakeOutboxRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement =
            conn.prepare("SELECT wake_id, record_json FROM http_push_wakes ORDER BY wake_id ASC")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut wakes = BTreeMap::new();
        for row in rows {
            let (wake_id, json) = row?;
            wakes.insert(wake_id, serde_json::from_str(&json)?);
        }
        Ok(wakes)
    }

    pub(crate) fn load_blob_meta(&self) -> Result<BTreeMap<String, BlobMeta>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT sha256, size_bytes, content_type, backend
             FROM http_blob_meta
             ORDER BY sha256 ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut meta = BTreeMap::new();
        for row in rows {
            let (sha256, size_bytes, content_type, backend) = row?;
            let backend = BlobBackend::parse(&backend).ok_or_else(|| {
                DurableStoreError::BlobObjectCorrupt {
                    sha256: sha256.clone(),
                }
            })?;
            meta.insert(
                sha256,
                BlobMeta {
                    size_bytes,
                    content_type,
                    backend,
                },
            );
        }
        Ok(meta)
    }

    pub(crate) fn load_blob_payload(
        &self,
        sha256: &str,
    ) -> Result<Option<Vec<u8>>, DurableStoreError> {
        let conn = self.connection();
        let bytes = conn
            .query_row(
                "SELECT ciphertext FROM http_blob_objects WHERE sha256 = ?1",
                params![sha256],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        Ok(bytes)
    }

    pub(crate) fn insert_blob_object(
        &self,
        sha256: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        transaction.execute(
            "INSERT INTO http_blob_objects (sha256, size_bytes, content_type, ciphertext)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(sha256) DO NOTHING",
            params![sha256, bytes.len() as u64, content_type, bytes],
        )?;
        transaction.execute(
            "INSERT INTO http_blob_meta
                 (sha256, size_bytes, content_type, backend, created_at_ms)
             VALUES (?1, ?2, ?3, 'sqlite', ?4)
             ON CONFLICT(sha256) DO NOTHING",
            params![sha256, bytes.len() as u64, content_type, unix_now_ms()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn upsert_push_token(
        &self,
        record: &PushTokenRecord,
    ) -> Result<(), DurableStoreError> {
        let json = serde_json::to_string(record)?;
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_push_tokens (device_key, record_json)
             VALUES (?1, ?2)
             ON CONFLICT(device_key) DO UPDATE SET record_json = excluded.record_json",
            params![DeviceMembership::key(&record.device), json],
        )?;
        Ok(())
    }

    pub(crate) fn delete_push_token(&self, device_key: &str) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM http_push_tokens WHERE device_key = ?1",
            params![device_key],
        )?;
        Ok(())
    }

    pub(crate) fn upsert_push_wakes(
        &self,
        records: &[PushWakeOutboxRecord],
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        for record in records {
            upsert_push_wake_in_transaction(&transaction, record)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn delete_push_wake(&self, wake_id: &str) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM http_push_wakes WHERE wake_id = ?1",
            params![wake_id],
        )?;
        Ok(())
    }

    pub(crate) fn load_state_snapshot(
        &self,
    ) -> Result<Option<(i64, DurableStateSnapshot)>, DurableStoreError> {
        let conn = self.connection();
        let v2 = conn
            .query_row(
                "SELECT last_op_seq, snapshot_zstd FROM http_state_snapshots_v2 WHERE id = 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?;
        if let Some((seq, compressed)) = v2 {
            let snapshot = serde_json::from_reader(zstd::Decoder::new(compressed.as_slice())?)?;
            return Ok(Some((seq, snapshot)));
        }
        // Uncompressed rows written before the v2 table existed.
        let row = conn
            .query_row(
                "SELECT last_op_seq, snapshot_json FROM http_state_snapshots WHERE id = 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        match row {
            Some((seq, json)) => Ok(Some((seq, serde_json::from_str(&json)?))),
            None => Ok(None),
        }
    }

    pub(crate) fn save_state_snapshot(
        &self,
        last_op_seq: i64,
        snapshot: &DurableStateSnapshot,
    ) -> Result<(), DurableStoreError> {
        // The plain-JSON encoding outgrew SQLite's 1e9-byte value cap in
        // production, so every save failed; compression keeps the row far
        // under the cap, and streaming into the encoder avoids materializing
        // the uncompressed document.
        let mut encoder = zstd::Encoder::new(Vec::new(), SNAPSHOT_ZSTD_LEVEL)?;
        serde_json::to_writer(&mut encoder, snapshot)?;
        let compressed = encoder.finish()?;
        let conn = self.connection();
        let prune_horizon: Option<i64> = conn.query_row(
            "SELECT MIN(last_op_seq) FROM (
                 SELECT last_op_seq FROM http_state_snapshots WHERE id = 1
                 UNION ALL
                 SELECT last_op_seq FROM http_state_snapshots_v2 WHERE id = 1
             )",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        conn.execute(
            "INSERT INTO http_state_snapshots_v2 (id, last_op_seq, snapshot_zstd)
             VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
                 last_op_seq = excluded.last_op_seq,
                 snapshot_zstd = excluded.snapshot_zstd
             WHERE excluded.last_op_seq >= http_state_snapshots_v2.last_op_seq",
            params![last_op_seq, compressed],
        )?;
        // Ops at or below every retained snapshot's horizon can never be
        // replayed again. The MIN across both snapshot generations keeps a
        // still-present legacy row (and a rollback build that boots from it)
        // fully replayable.
        if let Some(horizon) = prune_horizon {
            conn.execute(
                "DELETE FROM http_delivery_ops WHERE seq <= ?1",
                params![horizon.min(last_op_seq)],
            )?;
        }
        Ok(())
    }

    pub(crate) fn load_publish_idempotency(
        &self,
    ) -> Result<HashMap<String, PublishIdempotencyRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT idempotency_key, fingerprint_json, receipt_json FROM http_publish_idempotency",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut idempotency = HashMap::new();
        for row in rows {
            let (key, fingerprint_json, receipt_json) = row?;
            idempotency.insert(
                key,
                PublishIdempotencyRecord {
                    fingerprint: serde_json::from_str(&fingerprint_json)?,
                    receipt: serde_json::from_str(&receipt_json)?,
                },
            );
        }
        Ok(idempotency)
    }

    pub(crate) fn load_key_package_claim_idempotency(
        &self,
    ) -> Result<HashMap<String, KeyPackageClaimIdempotencyRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT idempotency_key, fingerprint_json, response_json
             FROM http_key_package_claim_idempotency",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut idempotency = HashMap::new();
        for row in rows {
            let (key, fingerprint_json, response_json) = row?;
            idempotency.insert(
                key,
                KeyPackageClaimIdempotencyRecord {
                    fingerprint: serde_json::from_str(&fingerprint_json)?,
                    response: serde_json::from_str(&response_json)?,
                },
            );
        }
        Ok(idempotency)
    }

    pub(crate) fn upsert_key_package_inventory(
        &self,
        record: &KeyPackageInventoryRecord,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_key_package_inventory (
                key_package_id_json,
                owner_json,
                state_json
            ) VALUES (?1, ?2, ?3)
            ON CONFLICT(key_package_id_json) DO UPDATE SET
                owner_json = excluded.owner_json,
                state_json = excluded.state_json",
            params![
                serde_json::to_string(&record.key_package_id)?,
                serde_json::to_string(&record.owner)?,
                serde_json::to_string(&record.state)?,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn load_key_package_inventory(
        &self,
    ) -> Result<HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT key_package_id_json, owner_json, state_json FROM http_key_package_inventory",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut inventory = HashMap::new();
        for row in rows {
            let (key_package_id_json, owner_json, state_json) = row?;
            let key_package_id: HttpKeyPackageId = serde_json::from_str(&key_package_id_json)?;
            inventory.insert(
                key_package_id.clone(),
                KeyPackageInventoryRecord {
                    key_package_id,
                    owner: serde_json::from_str(&owner_json)?,
                    key_package: KeyPackage::new(Vec::new()),
                    state: serde_json::from_str(&state_json)?,
                    finite_metadata: None,
                },
            );
        }
        Ok(inventory)
    }

    pub(crate) fn upsert_pairing_session(
        &self,
        record: &HttpPairingSessionRecord,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_pairing_sessions (pairing_session_id, record_json)
             VALUES (?1, ?2)
             ON CONFLICT(pairing_session_id) DO UPDATE SET
                record_json = excluded.record_json",
            params![record.pairing_session_id, serde_json::to_string(record)?],
        )?;
        Ok(())
    }

    pub(crate) fn load_pairing_sessions(
        &self,
    ) -> Result<BTreeMap<String, HttpPairingSessionRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT pairing_session_id, record_json
             FROM http_pairing_sessions
             ORDER BY pairing_session_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut sessions = BTreeMap::new();
        for row in rows {
            let (pairing_session_id, record_json) = row?;
            sessions.insert(pairing_session_id, serde_json::from_str(&record_json)?);
        }
        Ok(sessions)
    }

    pub(crate) fn upsert_nostr_profile(
        &self,
        record: &NostrProfileRecord,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_nostr_profiles (account_id, record_json)
             VALUES (?1, ?2)
             ON CONFLICT(account_id) DO UPDATE SET
                record_json = excluded.record_json",
            params![record.account_id, serde_json::to_string(record)?],
        )?;
        Ok(())
    }

    pub(crate) fn load_nostr_profiles(
        &self,
    ) -> Result<BTreeMap<String, NostrProfileRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT account_id, record_json
             FROM http_nostr_profiles
             ORDER BY account_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut profiles = BTreeMap::new();
        for row in rows {
            let (account_id, record_json) = row?;
            profiles.insert(account_id, serde_json::from_str(&record_json)?);
        }
        Ok(profiles)
    }

    pub(crate) fn upsert_account_room(
        &self,
        record: &AccountRoomDirectoryRecord,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_account_rooms (account_id, room_id, record_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id, room_id) DO UPDATE SET
                record_json = excluded.record_json",
            params![
                record.account_id,
                record.room_id,
                serde_json::to_string(&record.record)?,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn load_account_room_directory(
        &self,
    ) -> Result<BTreeMap<String, BTreeMap<String, Value>>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT account_id, room_id, record_json
             FROM http_account_rooms
             ORDER BY account_id ASC, room_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut directory = BTreeMap::new();
        for row in rows {
            let (account_id, room_id, record_json) = row?;
            directory
                .entry(account_id)
                .or_insert_with(BTreeMap::new)
                .insert(room_id, serde_json::from_str(&record_json)?);
        }
        Ok(directory)
    }

    pub(crate) fn delete_account_room(
        &self,
        account_id: &str,
        room_id: &str,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM http_account_rooms WHERE account_id = ?1 AND room_id = ?2",
            params![account_id, room_id],
        )?;
        Ok(())
    }

    pub(crate) fn upsert_room_membership(
        &self,
        projection: &HttpRoomMembershipProjection,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_room_memberships (room_id, projection_json)
             VALUES (?1, ?2)
             ON CONFLICT(room_id) DO UPDATE SET
                projection_json = excluded.projection_json",
            params![&projection.room_id, serde_json::to_string(projection)?,],
        )?;
        Ok(())
    }

    pub(crate) fn upsert_room_repair_state(
        &self,
        projection: &HttpRoomMembershipProjection,
        account_records: &[AccountRoomDirectoryRecord],
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        transaction.execute(
            "INSERT INTO http_room_memberships (room_id, projection_json)
             VALUES (?1, ?2)
             ON CONFLICT(room_id) DO UPDATE SET
                projection_json = excluded.projection_json",
            params![projection.room_id, serde_json::to_string(projection)?],
        )?;
        for record in account_records {
            transaction.execute(
                "INSERT INTO http_account_rooms (account_id, room_id, record_json)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, room_id) DO UPDATE SET
                    record_json = excluded.record_json",
                params![
                    record.account_id,
                    record.room_id,
                    serde_json::to_string(&record.record)?,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn load_room_memberships(
        &self,
    ) -> Result<BTreeMap<String, HttpRoomMembershipProjection>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT room_id, projection_json
             FROM http_room_memberships
             ORDER BY room_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut rooms = BTreeMap::new();
        for row in rows {
            let (room_id, projection_json) = row?;
            rooms.insert(room_id, serde_json::from_str(&projection_json)?);
        }
        Ok(rooms)
    }

    pub(crate) fn load_application_effects(
        &self,
    ) -> Result<BTreeMap<String, HttpApplicationDeliveryEffect>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT message_id, room_id, seq, sender_json, delivery_policy_json
             FROM http_application_delivery_effects
             ORDER BY message_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut effects = BTreeMap::new();
        for row in rows {
            let (message_id, room_id, seq, sender_json, delivery_policy_json) = row?;
            effects.insert(
                message_id.clone(),
                HttpApplicationDeliveryEffect {
                    room_id,
                    seq,
                    message_id,
                    sender: serde_json::from_str(&sender_json)?,
                    delivery_policy: serde_json::from_str(&delivery_policy_json)?,
                },
            );
        }
        Ok(effects)
    }

    pub(crate) fn upsert_welcome_claim(
        &self,
        record: &WelcomeClaimRecord,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_welcome_claims (
                message_id_json,
                recipient_json,
                seq,
                message_json,
                state_json
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(message_id_json) DO UPDATE SET
                recipient_json = excluded.recipient_json,
                seq = excluded.seq,
                message_json = excluded.message_json,
                state_json = excluded.state_json",
            params![
                serde_json::to_string(&record.message.id)?,
                serde_json::to_string(&record.recipient)?,
                record.seq,
                serde_json::to_string(&record.message)?,
                serde_json::to_string(&record.state)?,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn load_welcome_claims(
        &self,
    ) -> Result<HashMap<MessageId, WelcomeClaimRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT message_id_json, recipient_json, seq, message_json, state_json
             FROM http_welcome_claims",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut claims = HashMap::new();
        for row in rows {
            let (message_id_json, recipient_json, seq, message_json, state_json) = row?;
            let message_id = serde_json::from_str(&message_id_json)?;
            claims.insert(
                message_id,
                WelcomeClaimRecord {
                    recipient: serde_json::from_str(&recipient_json)?,
                    seq,
                    message: serde_json::from_str(&message_json)?,
                    state: serde_json::from_str(&state_json)?,
                },
            );
        }
        Ok(claims)
    }

    fn connection(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .expect("HTTP delivery store connection mutex")
    }
}

fn ensure_blob_content_type_column(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut statement = conn.prepare("PRAGMA table_info(http_blob_objects)")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == "content_type" {
            return Ok(());
        }
    }
    conn.execute(
        "ALTER TABLE http_blob_objects
         ADD COLUMN content_type TEXT NOT NULL DEFAULT 'application/octet-stream'",
        [],
    )?;
    Ok(())
}

fn upsert_key_package_inventory_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    record: &KeyPackageInventoryRecord,
) -> Result<(), DurableStoreError> {
    transaction.execute(
        "INSERT INTO http_key_package_inventory (
            key_package_id_json,
            owner_json,
            state_json
        ) VALUES (?1, ?2, ?3)
        ON CONFLICT(key_package_id_json) DO UPDATE SET
            owner_json = excluded.owner_json,
            state_json = excluded.state_json",
        params![
            serde_json::to_string(&record.key_package_id)?,
            serde_json::to_string(&record.owner)?,
            serde_json::to_string(&record.state)?,
        ],
    )?;
    Ok(())
}

fn upsert_application_effect_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    effect: &HttpApplicationDeliveryEffect,
) -> Result<(), DurableStoreError> {
    transaction.execute(
        "INSERT INTO http_application_delivery_effects (
            message_id,
            room_id,
            seq,
            sender_json,
            delivery_policy_json
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(message_id) DO NOTHING",
        params![
            &effect.message_id,
            &effect.room_id,
            effect.seq,
            serde_json::to_string(&effect.sender)?,
            serde_json::to_string(&effect.delivery_policy)?,
        ],
    )?;
    Ok(())
}

fn upsert_push_wake_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    record: &PushWakeOutboxRecord,
) -> Result<(), DurableStoreError> {
    transaction.execute(
        "INSERT INTO http_push_wakes (wake_id, record_json)
         VALUES (?1, ?2)
         ON CONFLICT(wake_id) DO UPDATE SET record_json = excluded.record_json",
        params![&record.wake_id, serde_json::to_string(record)?],
    )?;
    Ok(())
}

pub(crate) fn replay_operation(
    service: &mut HttpDeliveryService,
    operation: PersistedOperation,
) -> Result<(), DurableStoreError> {
    match operation {
        PersistedOperation::PublishMessage {
            target, message, ..
        } => {
            service.publish(target, message)?;
        }
        // KeyPackage lease/reclaim/consume state is rebuilt in the finite wrapper
        // inventory; Finite Chat's core store has no claimed lease state.
        PersistedOperation::PublishKeyPackage { .. } => {}
        PersistedOperation::RevokeDevice { .. } => {}
        PersistedOperation::ClaimKeyPackage { .. }
        | PersistedOperation::ClaimKeyPackages { .. }
        | PersistedOperation::ExpireKeyPackageLease { .. } => {}
    }
    Ok(())
}

pub(crate) fn apply_operations_to_revoked_devices(
    revoked: &mut BTreeSet<String>,
    operations: &[PersistedOperation],
) {
    for operation in operations {
        if let PersistedOperation::RevokeDevice { device } = operation {
            revoked.insert(DeviceMembership::key(device));
        }
    }
}

pub(crate) fn apply_operations_to_key_package_inventory(
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    operations: &[PersistedOperation],
) {
    for operation in operations {
        match operation {
            PersistedOperation::PublishKeyPackage { publication } => {
                if let Some(metadata) = finite_key_package_metadata(publication) {
                    retire_older_finite_key_packages(
                        inventory,
                        &metadata.owner,
                        &publication.key_package_id,
                    );
                }
                let record = inventory
                    .entry(publication.key_package_id.clone())
                    .or_insert_with(|| KeyPackageInventoryRecord {
                        key_package_id: publication.key_package_id.clone(),
                        owner: publication.owner.clone(),
                        key_package: publication.key_package.clone(),
                        state: KeyPackageInventoryState::Available,
                        finite_metadata: finite_key_package_metadata(publication),
                    });
                if record.key_package.bytes().is_empty() {
                    record.key_package = publication.key_package.clone();
                }
                if record.finite_metadata.is_none() {
                    record.finite_metadata = finite_key_package_metadata(publication);
                }
            }
            PersistedOperation::ClaimKeyPackage { owner } => {
                mark_next_key_package_claimed(inventory, owner);
            }
            PersistedOperation::ClaimKeyPackages { owners } => {
                for owner in owners {
                    mark_next_key_package_claimed(inventory, owner);
                }
            }
            PersistedOperation::ExpireKeyPackageLease { key_package_id } => {
                if let Some(record) = inventory.get_mut(key_package_id)
                    && record.state == KeyPackageInventoryState::Claimed
                {
                    record.state = KeyPackageInventoryState::Available;
                }
            }
            PersistedOperation::PublishMessage { message, .. } => {
                consume_key_packages_from_persisted_message(inventory, message);
            }
            PersistedOperation::RevokeDevice { .. } => {}
        }
    }
}
