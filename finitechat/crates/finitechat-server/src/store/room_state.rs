//! Durable room state for the normalized engine.
//!
//! The room-membership projection has exactly ONE durable home here:
//! [`room_state_checkpoint`], a zstd-compressed snapshot of every
//! [`HttpRoomMembershipProjection`] whose per-room `last_seq` is the
//! watermark of the entries it already reflects. Boot re-derives the live
//! maps from the checkpoint plus the `delivery_entries` tails (see
//! `HttpServerState::boot_normalized`); the checkpoint is never trusted as
//! authority on its own — one that lags simply replays more entries, and one
//! that disagrees with the route heads fails boot closed.
//!
//! The account-room directory is current state (direct saves are primary
//! writes), with its commit-derived rows written in the same transaction as
//! the delivery publish that produced them.

use std::collections::{BTreeMap, BTreeSet};

use finitechat_delivery::{HttpQueuedDelivery, HttpSequence, MAX_HTTP_SYNC_PAGE_ENTRIES};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::projections::HttpRoomMembershipProjection;
use crate::state::AccountRoomDirectoryMutation;
use crate::{DurableStoreError, SNAPSHOT_ZSTD_LEVEL};

use super::schema::{FOLD_COMPLETE_KEY, FOLD_HEAD_KEY};

/// The single durable room-state structure: every room projection, each
/// carrying the `last_seq` watermark of the delivery entries it reflects.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct RoomStateCheckpoint {
    pub(crate) rooms: BTreeMap<String, HttpRoomMembershipProjection>,
}

/// Read the fold marker: `true` once the one-time op-log fold has populated
/// the normalized tables (see `crate::cutover`).
pub(crate) fn fold_complete(conn: &Connection) -> Result<bool, DurableStoreError> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM server_meta WHERE key = ?1",
            params![FOLD_COMPLETE_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value.is_some())
}

/// Set the fold marker inside the fold transaction.
pub(crate) fn mark_fold_complete(
    tx: &rusqlite::Transaction<'_>,
    iso_date: &str,
) -> Result<(), DurableStoreError> {
    tx.execute(
        "INSERT INTO server_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![FOLD_COMPLETE_KEY, iso_date],
    )?;
    Ok(())
}

/// The normalized delivery head: the total number of sequenced delivery
/// entries across every route (`SUM(last_seq)`). Route heads only ever
/// advance and routes are never deleted, so this scalar is strictly
/// monotone in the set of `(route, seq)` pairs a client can hold a cursor
/// for: two databases with the same head (one derived from the other) have
/// the same cursor frontier.
pub(crate) fn delivery_head(conn: &Connection) -> Result<HttpSequence, DurableStoreError> {
    let head: i64 = conn.query_row(
        "SELECT COALESCE(SUM(last_seq), 0) FROM delivery_routes",
        [],
        |row| row.get(0),
    )?;
    Ok(super::db_to_u64(head)?)
}

/// Record the pre-fold head inside the fold transaction (see
/// [`FOLD_HEAD_KEY`]).
pub(crate) fn record_fold_head(
    tx: &rusqlite::Transaction<'_>,
    head: HttpSequence,
) -> Result<(), DurableStoreError> {
    tx.execute(
        "INSERT INTO server_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![FOLD_HEAD_KEY, head.to_string()],
    )?;
    Ok(())
}

/// Read the recorded pre-fold head verbatim (`None` when the fold ran under
/// a build that did not record it; callers fail closed on that and on an
/// unparseable value).
pub(crate) fn load_fold_head(conn: &Connection) -> Result<Option<String>, DurableStoreError> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM server_meta WHERE key = ?1",
            params![FOLD_HEAD_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value)
}

pub(crate) fn load_checkpoint(
    conn: &Connection,
) -> Result<Option<RoomStateCheckpoint>, DurableStoreError> {
    let compressed: Option<Vec<u8>> = conn
        .query_row(
            "SELECT state_zstd FROM room_state_checkpoint WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(compressed) = compressed else {
        return Ok(None);
    };
    let checkpoint = serde_json::from_reader(zstd::Decoder::new(compressed.as_slice())?)?;
    Ok(Some(checkpoint))
}

/// Write the checkpoint inside a transaction (the caller owns the atomicity
/// coupling, e.g. leave-room writes the directory delete in the same tx).
pub(crate) fn save_checkpoint(
    tx: &rusqlite::Transaction<'_>,
    checkpoint: &RoomStateCheckpoint,
) -> Result<(), DurableStoreError> {
    let mut encoder = zstd::Encoder::new(Vec::new(), SNAPSHOT_ZSTD_LEVEL)?;
    serde_json::to_writer(&mut encoder, checkpoint)?;
    let compressed = encoder.finish()?;
    tx.execute(
        "INSERT INTO room_state_checkpoint (id, state_zstd) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET state_zstd = excluded.state_zstd",
        params![compressed],
    )?;
    Ok(())
}

pub(crate) fn load_directory(
    conn: &Connection,
) -> Result<BTreeMap<String, BTreeMap<String, Value>>, DurableStoreError> {
    let mut statement = conn.prepare(
        "SELECT account_id, room_id, record_json FROM account_room_directory
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

/// Apply one directory mutation (deletes then upserts) inside a transaction.
pub(crate) fn apply_directory_mutation(
    tx: &rusqlite::Transaction<'_>,
    mutation: &AccountRoomDirectoryMutation,
) -> Result<(), DurableStoreError> {
    for (account_id, room_id) in &mutation.deletes {
        tx.execute(
            "DELETE FROM account_room_directory WHERE account_id = ?1 AND room_id = ?2",
            params![account_id, room_id],
        )?;
    }
    for record in &mutation.upserts {
        tx.execute(
            "INSERT INTO account_room_directory (account_id, room_id, record_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id, room_id) DO UPDATE SET
                record_json = excluded.record_json",
            params![
                record.account_id,
                record.room_id,
                &record.record.to_string()
            ],
        )?;
    }
    Ok(())
}

pub(crate) fn load_revoked_devices(
    conn: &Connection,
) -> Result<BTreeSet<String>, DurableStoreError> {
    let mut statement = conn.prepare("SELECT device_key FROM revoked_devices")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut revoked = BTreeSet::new();
    for row in rows {
        revoked.insert(row?);
    }
    Ok(revoked)
}

/// Every group route whose key is a UTF-8 room id, with its current head.
/// Group route keys ARE Finite room ids (`group_id_for_room`); anything else
/// (non-UTF-8) is not a room and is skipped.
pub(crate) fn group_room_heads(
    conn: &Connection,
) -> Result<Vec<(String, HttpSequence)>, DurableStoreError> {
    let mut statement = conn.prepare(
        "SELECT route_key, last_seq FROM delivery_routes WHERE plane = 'group' ORDER BY route_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut heads = Vec::new();
    for row in rows {
        let (route_key, last_seq) = row?;
        let Ok(room_id) = String::from_utf8(route_key) else {
            continue;
        };
        heads.push((room_id, super::db_to_u64(last_seq)?));
    }
    Ok(heads)
}

/// All delivery entries for one group route strictly after `after_seq`, in
/// seq order (paged internally to bound statement row counts).
pub(crate) fn group_entries_after(
    conn: &Connection,
    room_id: &str,
    after_seq: HttpSequence,
) -> Result<Vec<HttpQueuedDelivery>, DurableStoreError> {
    let route_id: Option<i64> = conn
        .query_row(
            "SELECT route_id FROM delivery_routes WHERE plane = 'group' AND route_key = ?1",
            params![room_id.as_bytes()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(route_id) = route_id else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    let mut cursor = after_seq;
    loop {
        let mut statement = conn.prepare(
            "SELECT seq, message_id, payload, ts, causal_deps_json,
                    source, envelope_kind, envelope_ref
             FROM delivery_entries
             WHERE route_id = ?1 AND seq > ?2
             ORDER BY seq
             LIMIT ?3",
        )?;
        let rows = statement
            .query_map(
                params![
                    route_id,
                    super::u64_to_db(cursor)?,
                    MAX_HTTP_SYNC_PAGE_ENTRIES
                ],
                super::delivery::row_to_queued_delivery,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let page_len = rows.len();
        let next_cursor = rows.last().map_or(cursor, |entry| entry.seq);
        entries.extend(rows);
        if page_len < MAX_HTTP_SYNC_PAGE_ENTRIES || next_cursor <= cursor {
            return Ok(entries);
        }
        cursor = next_cursor;
    }
}
