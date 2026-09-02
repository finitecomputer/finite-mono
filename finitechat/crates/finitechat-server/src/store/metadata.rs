//! Shared server-metadata tables on the normalized [`Store`].
//!
//! These tables predate the normalized delivery engine (they lived behind
//! the legacy op-log store's connection) but they were never part of the
//! op-log engine itself: pairing sessions, Nostr profiles, push tokens and
//! wakes, welcome claims, application-delivery effects, publish/claim
//! idempotency, the finite KeyPackage inventory, and blob objects are
//! current-state metadata written and read by the server layer directly.
//! They keep their historical `http_` names so a folded database needs no
//! metadata migration; only their accessors moved, from the deleted
//! `legacy_store.rs` into the normalized store.
//!
//! Everything here runs inside [`super::Store::read`] / [`super::Store::write`]
//! closures on the same single-writer connection that owns the delivery
//! tables, so a metadata row and a delivery append can commit atomically.

use std::collections::{BTreeMap, HashMap};
use std::time::SystemTime;

use finitechat_delivery::{HttpKeyPackageId, HttpKeyPackagePublication};
use finitechat_http::{
    HttpApplicationDeliveryEffect, HttpPairingSessionRecord, NostrProfileRecord, PushTokenRecord,
};
use finitechat_proto::DeviceMembership;
use finitechat_transport::MessageId;
use rusqlite::{Connection, OptionalExtension, params};

use crate::DurableStoreError;
use crate::state::{
    BlobBackend, BlobMeta, KeyPackageClaimIdempotencyRecord, KeyPackageInventoryRecord,
    PublishIdempotencyRecord, PushWakeOutboxRecord, WelcomeClaimRecord,
};

pub(crate) fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Create (or complete) the shared metadata schema on `conn`. Runs at every
/// boot; every statement is idempotent. This deliberately does NOT create the
/// legacy engine tables (`http_delivery_ops`, `http_state_snapshots_v2`,
/// `http_room_memberships`, `http_account_rooms`): a fresh database must stay
/// legacy-free so the fold gate ("legacy tables present") can tell a fresh
/// database from a pre-cutover one.
pub(crate) fn init_schema(conn: &Connection) -> Result<(), DurableStoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS http_push_tokens (
            device_key TEXT PRIMARY KEY,
            record_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS http_push_wakes (
            wake_id TEXT PRIMARY KEY,
            record_json TEXT NOT NULL
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
        );",
    )?;
    ensure_blob_content_type_column(conn)?;
    // Databases written before http_blob_meta existed carry payload rows
    // only; give each a meta row so boot never has to touch payloads.
    conn.execute(
        "INSERT OR IGNORE INTO http_blob_meta
             (sha256, size_bytes, content_type, backend, created_at_ms)
         SELECT sha256, size_bytes, content_type, 'sqlite', ?1
         FROM http_blob_objects",
        params![unix_now_ms()],
    )?;
    Ok(())
}

pub(crate) fn load_publish_idempotency(
    conn: &Connection,
) -> Result<HashMap<String, PublishIdempotencyRecord>, DurableStoreError> {
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

/// Insert one publish-idempotency row inside an existing transaction so it
/// commits atomically with the delivery entry it describes.
pub(crate) fn insert_publish_idempotency_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    idempotency_key: &str,
    record: &PublishIdempotencyRecord,
) -> Result<(), DurableStoreError> {
    transaction.execute(
        "INSERT INTO http_publish_idempotency (
            idempotency_key, fingerprint_json, receipt_json
        ) VALUES (?1, ?2, ?3)",
        params![
            idempotency_key,
            serde_json::to_string(&record.fingerprint)?,
            serde_json::to_string(&record.receipt)?
        ],
    )?;
    Ok(())
}

pub(crate) fn load_key_package_claim_idempotency(
    conn: &Connection,
) -> Result<HashMap<String, KeyPackageClaimIdempotencyRecord>, DurableStoreError> {
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

pub(crate) fn insert_key_package_claim_idempotency_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    idempotency_key: &str,
    record: &KeyPackageClaimIdempotencyRecord,
) -> Result<(), DurableStoreError> {
    transaction.execute(
        "INSERT INTO http_key_package_claim_idempotency (
            idempotency_key, fingerprint_json, response_json
        ) VALUES (?1, ?2, ?3)",
        params![
            idempotency_key,
            serde_json::to_string(&record.fingerprint)?,
            serde_json::to_string(&record.response)?
        ],
    )?;
    Ok(())
}

pub(crate) fn upsert_key_package_inventory(
    conn: &Connection,
    record: &KeyPackageInventoryRecord,
) -> Result<(), DurableStoreError> {
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
            serde_json::to_string(&record.state)?
        ],
    )?;
    Ok(())
}

/// Upsert one inventory row inside an existing transaction so the normalized
/// commit path consumes claimed KeyPackages atomically with the commit entry.
pub(crate) fn upsert_key_package_inventory_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    record: &KeyPackageInventoryRecord,
) -> Result<(), DurableStoreError> {
    upsert_key_package_inventory(transaction, record)
}

pub(crate) fn load_key_package_inventory(
    conn: &Connection,
) -> Result<HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>, DurableStoreError> {
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
                key_package: finitechat_transport::engine::KeyPackage::new(Vec::new()),
                state: serde_json::from_str(&state_json)?,
                finite_metadata: None,
            },
        );
    }
    enrich_inventory_with_durable_payloads(conn, &mut inventory)?;
    Ok(inventory)
}

/// Fill each inventory record's payload bytes (and derived finite metadata)
/// from the durable home in `sql_key_packages`. The shared inventory table
/// carries only the (id, owner, state) triple — the legacy engine rebuilt
/// bytes by replaying publish ops, so the normalized engine stores them in
/// the core KeyPackage table instead (see `upsert_key_package_payload`).
fn enrich_inventory_with_durable_payloads(
    conn: &Connection,
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
) -> Result<(), DurableStoreError> {
    if inventory.is_empty() {
        return Ok(());
    }
    let mut statement =
        conn.prepare("SELECT key_package_id, key_package_bytes FROM sql_key_packages")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    let mut payloads = HashMap::new();
    for row in rows {
        let (key_package_id, bytes) = row?;
        payloads.insert(HttpKeyPackageId::new(key_package_id), bytes);
    }
    for (key_package_id, record) in inventory.iter_mut() {
        let Some(bytes) = payloads.get(key_package_id) else {
            continue;
        };
        if !bytes.is_empty() {
            record.key_package = finitechat_transport::engine::KeyPackage::new(bytes.clone());
        }
        if record.finite_metadata.is_none() {
            record.finite_metadata =
                crate::state::finite_key_package_metadata(&HttpKeyPackagePublication {
                    key_package_id: record.key_package_id.clone(),
                    owner: record.owner.clone(),
                    key_package: record.key_package.clone(),
                });
        }
    }
    Ok(())
}

/// Persist the payload side of a wrapper KeyPackage record: the bytes (and
/// current state) live in `sql_key_packages`, the lease/consume state in
/// the shared inventory table. Called from the same transaction that
/// upserts the inventory rows so the two views cannot skew.
pub(crate) fn upsert_key_package_payload(
    transaction: &rusqlite::Transaction<'_>,
    record: &KeyPackageInventoryRecord,
) -> Result<(), DurableStoreError> {
    let state = match record.state {
        crate::state::KeyPackageInventoryState::Available => "available",
        crate::state::KeyPackageInventoryState::Claimed => "claimed",
        crate::state::KeyPackageInventoryState::Consumed => "consumed",
    };
    let source_json = record
        .key_package
        .source
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    transaction.execute(
        "INSERT INTO sql_key_packages
             (key_package_id, owner, key_package_bytes, key_package_source_json, state)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(key_package_id) DO UPDATE SET
            key_package_bytes = excluded.key_package_bytes,
            key_package_source_json = excluded.key_package_source_json,
            state = excluded.state",
        params![
            record.key_package_id.as_slice(),
            record.owner.as_slice(),
            &record.key_package.bytes,
            source_json,
            state,
        ],
    )?;
    Ok(())
}

pub(crate) fn upsert_pairing_session(
    conn: &Connection,
    record: &HttpPairingSessionRecord,
) -> Result<(), DurableStoreError> {
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
    conn: &Connection,
) -> Result<BTreeMap<String, HttpPairingSessionRecord>, DurableStoreError> {
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
    conn: &Connection,
    record: &NostrProfileRecord,
) -> Result<(), DurableStoreError> {
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
    conn: &Connection,
) -> Result<BTreeMap<String, NostrProfileRecord>, DurableStoreError> {
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

pub(crate) fn load_application_effects(
    conn: &Connection,
) -> Result<BTreeMap<String, HttpApplicationDeliveryEffect>, DurableStoreError> {
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

pub(crate) fn upsert_application_effect_in_transaction(
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
            serde_json::to_string(&effect.delivery_policy)?
        ],
    )?;
    Ok(())
}

pub(crate) fn upsert_welcome_claim(
    conn: &Connection,
    record: &WelcomeClaimRecord,
) -> Result<(), DurableStoreError> {
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
            serde_json::to_string(&record.state)?
        ],
    )?;
    Ok(())
}

pub(crate) fn load_welcome_claims(
    conn: &Connection,
) -> Result<HashMap<MessageId, WelcomeClaimRecord>, DurableStoreError> {
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

pub(crate) fn load_push_tokens(
    conn: &Connection,
) -> Result<BTreeMap<String, PushTokenRecord>, DurableStoreError> {
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

pub(crate) fn upsert_push_token(
    conn: &Connection,
    record: &PushTokenRecord,
) -> Result<(), DurableStoreError> {
    conn.execute(
        "INSERT INTO http_push_tokens (device_key, record_json)
         VALUES (?1, ?2)
         ON CONFLICT(device_key) DO UPDATE SET record_json = excluded.record_json",
        params![
            DeviceMembership::key(&record.device),
            serde_json::to_string(record)?
        ],
    )?;
    Ok(())
}

pub(crate) fn delete_push_token(
    conn: &Connection,
    device_key: &str,
) -> Result<(), DurableStoreError> {
    conn.execute(
        "DELETE FROM http_push_tokens WHERE device_key = ?1",
        params![device_key],
    )?;
    Ok(())
}

pub(crate) fn load_push_wakes(
    conn: &Connection,
) -> Result<BTreeMap<String, PushWakeOutboxRecord>, DurableStoreError> {
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

pub(crate) fn upsert_push_wake_in_transaction(
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

pub(crate) fn delete_push_wake(conn: &Connection, wake_id: &str) -> Result<(), DurableStoreError> {
    conn.execute(
        "DELETE FROM http_push_wakes WHERE wake_id = ?1",
        params![wake_id],
    )?;
    Ok(())
}

pub(crate) fn load_blob_meta(
    conn: &Connection,
) -> Result<BTreeMap<String, BlobMeta>, DurableStoreError> {
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
        let backend =
            BlobBackend::parse(&backend).ok_or_else(|| DurableStoreError::BlobObjectCorrupt {
                sha256: sha256.clone(),
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
    conn: &Connection,
    sha256: &str,
) -> Result<Option<Vec<u8>>, DurableStoreError> {
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
    transaction: &rusqlite::Transaction<'_>,
    sha256: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<(), DurableStoreError> {
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
    Ok(())
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
