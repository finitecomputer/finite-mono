//! Normalized schema for the SQL delivery engine.
//!
//! This covers exactly what the [`finitechat_delivery::HttpDelivery`]
//! contract exercises: sequenced per-route delivery entries, commit-epoch
//! admission, and the KeyPackage lifecycle. The richer finite-metadata
//! columns (leases, device bindings, inventory counters) arrive in a later
//! PR when the server-side inventory merges in.

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
        ",
    )?;
    conn.execute(
        "INSERT INTO server_meta (key, value) VALUES ('engine_schema_version', ?1)
         ON CONFLICT (key) DO NOTHING",
        [ENGINE_SCHEMA_VERSION],
    )?;
    Ok(())
}
