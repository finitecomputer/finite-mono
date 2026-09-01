//! Normalized schema for the SQL delivery engine.
//!
//! This covers exactly what the [`finitechat_delivery::HttpDelivery`]
//! contract exercises: sequenced per-route delivery entries, commit-epoch
//! admission, and the KeyPackage lifecycle. The richer finite-metadata
//! columns (leases, device bindings, inventory counters) arrive in a later
//! PR when the server-side inventory merges in.
//!
//! The cutover schema (PR 1 of the chat store swap) adds the engine's own
//! durable homes for the state the legacy engine kept in
//! `http_state_snapshots_v2` / `http_room_memberships` /
//! `http_account_rooms`: `revoked_devices` (current state), and
//! `account_room_directory` (current state, also derived in-transaction with
//! commits). Room-membership projections live in exactly ONE durable
//! structure, `room_state_checkpoint` — a boot memo of the derivation
//! (per-room `last_seq` watermarks inside the snapshot) that is re-derived
//! from `delivery_entries` tails at every boot and never trusted as
//! authority on its own: a checkpoint that lags simply replays more entries,
//! and one that disagrees with the entries fails boot closed.
//!
//! `server_meta` carries `op_log_fold_complete` once the one-time
//! op-log fold (`crate::cutover`) has populated these tables from the legacy
//! engine's state.

use rusqlite::Connection;

/// Current engine schema version, written to `server_meta`.
const ENGINE_SCHEMA_VERSION: &str = "1";

/// Create every table and index this engine needs (idempotent).
pub(crate) fn migrate_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS server_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS delivery_routes (
            route_id INTEGER PRIMARY KEY,
            plane TEXT NOT NULL CHECK (plane IN ('group', 'inbox')),
            route_key BLOB NOT NULL,
            last_seq INTEGER NOT NULL DEFAULT 0 CHECK (last_seq >= 0),
            UNIQUE (plane, route_key)
        );
        CREATE TABLE IF NOT EXISTS delivery_entries (
            route_id INTEGER NOT NULL REFERENCES delivery_routes(route_id),
            seq INTEGER NOT NULL CHECK (seq > 0),
            message_id BLOB NOT NULL,
            digest BLOB NOT NULL,
            payload BLOB NOT NULL,
            ts INTEGER NOT NULL,
            causal_deps_json TEXT NOT NULL,
            source TEXT NOT NULL,
            envelope_kind INTEGER NOT NULL CHECK (envelope_kind IN (0, 1)),
            envelope_ref BLOB NOT NULL,
            PRIMARY KEY (route_id, seq),
            UNIQUE (route_id, message_id)
        );
        CREATE TABLE IF NOT EXISTS group_commit_epochs (
            route_id INTEGER NOT NULL REFERENCES delivery_routes(route_id),
            source_epoch INTEGER NOT NULL,
            seq INTEGER NOT NULL,
            PRIMARY KEY (route_id, source_epoch)
        );
        CREATE TABLE IF NOT EXISTS sql_key_packages (
            key_package_id BLOB PRIMARY KEY,
            owner BLOB NOT NULL,
            key_package_bytes BLOB NOT NULL,
            key_package_source_json TEXT,
            state TEXT NOT NULL CHECK (state IN ('available', 'claimed', 'consumed'))
        );
        CREATE INDEX IF NOT EXISTS idx_sql_kp_owner_state
            ON sql_key_packages(owner, state, key_package_id);
        CREATE TABLE IF NOT EXISTS revoked_devices (
            device_key TEXT PRIMARY KEY
        );
        CREATE TABLE IF NOT EXISTS account_room_directory (
            account_id TEXT NOT NULL,
            room_id TEXT NOT NULL,
            record_json TEXT NOT NULL,
            PRIMARY KEY(account_id, room_id)
        );
        CREATE TABLE IF NOT EXISTS room_state_checkpoint (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            state_zstd BLOB NOT NULL
        );
        ",
    )?;
    conn.execute(
        "INSERT INTO server_meta (key, value) VALUES ('engine_schema_version', ?1)
         ON CONFLICT (key) DO NOTHING",
        [ENGINE_SCHEMA_VERSION],
    )?;
    Ok(())
}

/// `server_meta` key set by the one-time op-log fold (see `crate::cutover`).
/// Its presence means the normalized tables are authoritative and the legacy
/// tables (`http_delivery_ops`, `http_state_snapshots_v2`,
/// `http_room_memberships`, `http_account_rooms`) are frozen migration input.
pub(crate) const FOLD_COMPLETE_KEY: &str = "op_log_fold_complete";
