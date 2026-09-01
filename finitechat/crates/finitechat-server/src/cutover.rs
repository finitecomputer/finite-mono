//! One-time fold of the legacy op-log engine's state into the normalized
//! tables (chat store swap, PR 1) — and the minimal legacy READER it feeds.
//!
//! This module is TRANSITIONAL. It exists only to move a pre-cutover
//! database onto the normalized engine, and PR 2 (`cleanup/chat-store-
//! delete-old`) deletes it wholesale before 2026-09-25. Two things live
//! here:
//!
//! * The READER: exactly the legacy boot path production ran until the
//!   cutover — v2 snapshot load, op-log tail load and replay, the #770
//!   room-projection reconciliation, and the legacy-table reads the seed
//!   transplants from. It never serves: no request path reaches it. Its
//!   only caller is the fold. It writes only the legacy reconciliation
//!   rows (`http_room_memberships` / `http_account_rooms`), exactly as the
//!   legacy boot did, so a failed fold leaves the input in the state the
//!   next boot's fold expects.
//! * The FOLD: the sanctioned full-history recovery action (ADR 0003)
//!   wearing a migration's clothes. It transplants the reader's fully
//!   booted state into the normalized tables inside ONE guarded boot
//!   transaction. The steady-state normalized engine never reads
//!   `http_state_snapshots_v2`, `http_room_memberships`,
//!   `http_account_rooms`, or `http_delivery_ops` again; those tables are
//!   frozen migration input from the moment the fold marker commits.
//!
//! What was DELETED with the serving engine (and must stay deleted): the
//! op-log append paths, v2 snapshot writing, snapshot-horizon pruning, and
//! the legacy projection upserts from live request handling. Nothing in
//! this module is reachable from a request path.
//!
//! Guarding and idempotence: the fold marker (`server_meta.op_log_fold_complete`)
//! is set inside the same transaction as the data. A crash mid-fold rolls
//! everything back; the next boot re-folds. The marker is re-checked inside
//! the write transaction so two racing boots cannot double-fold (the second
//! sees the marker under the write lock and skips).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use finitechat_delivery::{
    HttpDeliveryPlane, HttpDeliveryService, HttpKeyPackageId, HttpKeyPackagePublication,
    HttpPublishTarget, HttpSequence,
};
use finitechat_proto::{DeviceMembership, DeviceRef, LogEntryKind, WelcomeRecord};
use finitechat_transport::transport::TransportMessage;
use finitechat_transport::{MemberId, MessageId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::DurableStoreError;
use crate::FiniteAccountRoomCommitProjection;
use crate::finite_delivery_limits;
use crate::projections::HttpRoomMembershipProjection;
use crate::projections::apply_room_membership_delta;
use crate::state::{
    AccountRoomDirectoryMutation, KeyPackageInventoryRecord, KeyPackageInventoryState,
    WelcomeClaimRecord, WelcomeClaimState, activate_account_room_device_in_directory,
    apply_account_room_membership_delta, consume_key_packages_from_persisted_message,
    finite_key_package_metadata, mark_next_key_package_claimed, retire_older_finite_key_packages,
};
use crate::store::Store;
use crate::store::delivery::insert_entry;
use crate::store::room_state::{RoomStateCheckpoint, mark_fold_complete, save_checkpoint};
use crate::store::{db_to_u64, u64_to_db};

/// Date recorded in the fold marker for the rollback-window banner and
/// `scripts/finite-status` (chat store swap PR 1).
pub(crate) const CHAT_ENGINE_CUTOVER_DATE: &str = "2026-08-31";

/// Row counts and sample results asserted before the fold transaction
/// commits. Kept in the report so operators (and tests) can see what the
/// fold moved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FoldReport {
    pub(crate) routes: usize,
    pub(crate) entries: usize,
    pub(crate) commit_epochs: usize,
    pub(crate) key_packages: usize,
    pub(crate) directory_rows: usize,
    pub(crate) revoked_devices: usize,
    pub(crate) checkpoint_rooms: usize,
    pub(crate) sampled_routes: usize,
    pub(crate) sampled_directory_rows: usize,
}

/// How many routes the sampled-fold assertion inspects (first, last, and
/// evenly spaced between; all of them when there are fewer).
const SAMPLED_ROUTES: usize = 32;

/// The legacy engine's fully-booted state, extracted for the one-time fold.
/// The RAM delivery core's queues plus the post-reconciliation room maps —
/// exactly the state the old engine would have served, transplanted into
/// the normalized tables.
pub(crate) struct LegacyEngineSeed {
    pub(crate) routes: Vec<finitechat_delivery::HttpRouteSnapshot>,
    pub(crate) key_packages: Vec<finitechat_delivery::HttpKeyPackageSeed>,
    pub(crate) rooms: BTreeMap<String, HttpRoomMembershipProjection>,
    pub(crate) directory: BTreeMap<String, BTreeMap<String, Value>>,
    pub(crate) revoked_devices: BTreeSet<String>,
    /// The finite KeyPackage inventory rebuilt from snapshot + op log. The
    /// fold re-seeds the shared inventory table from this so the cutover
    /// cannot inherit a lagging cache row.
    pub(crate) inventory: Vec<KeyPackageInventoryRecord>,
}

/// Everything the legacy boot otherwise derived by replaying the full
/// operation log. Snapshotting it made legacy startup snapshot + tail
/// replay; the fold reader decodes it the same way.
#[derive(Serialize, Deserialize)]
pub(crate) struct DurableStateSnapshot {
    service: HttpDeliveryService,
    // Stored as a list: JSON maps need string keys, and the record carries
    // its own id.
    key_package_inventory: Vec<KeyPackageInventoryRecord>,
    revoked_devices: BTreeSet<String>,
}

/// One persisted operation of the legacy op log. Reader-side only: the
/// serving engine that appended these is gone; the fold replays them one
/// final time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PersistedOperation {
    PublishMessage {
        target: HttpPublishTarget,
        message: TransportMessage,
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

/// One group publish observed during boot replay, with the per-room seq the
/// service assigned it. Drives the #770 reconciliation below.
#[derive(Clone, Debug)]
pub(crate) struct ReplayedGroupPublish {
    pub(crate) room_id: String,
    pub(crate) message: finitechat_transport::transport::TransportMessage,
    pub(crate) seq: HttpSequence,
}

/// Fold the legacy engine's state at `path` into the normalized tables of
/// `store` (opened on the same file) unless the fold marker is already set.
/// Returns `None` when the fold already happened or the database is fresh
/// (no legacy tables): there is nothing to move.
pub(crate) fn fold_if_needed(
    store: &Store,
    path: &Path,
) -> Result<Option<FoldReport>, DurableStoreError> {
    if store
        .read(|conn| Ok(crate::store::room_state::fold_complete(conn)?))
        .map_err(fold_error)?
    {
        return Ok(None);
    }
    if !store
        .read(|conn| Ok(legacy_tables_present(conn)?))
        .map_err(fold_error)?
    {
        // Fresh (or post-deletion) database: the normalized tables are the
        // only state there ever was. The gate is table EXISTENCE, not
        // row counts — a database written by the legacy binary always
        // carries `http_delivery_ops` (its boot created it), and this
        // build never creates it.
        return Ok(None);
    }

    // Boot the legacy engine one final time. This is deliberately the same
    // code path production boots always ran — snapshot plus op-log tail
    // plus the #770 room reconciliation — so the transplanted state is the
    // state the old engine would have served.
    let seed = boot_legacy_seed(path)?;

    // Precompute the shared-inventory re-seed rows outside the transaction:
    // pure serialization, so the fold transaction only moves data.
    let inventory_rows = seed
        .inventory
        .iter()
        .map(|record| {
            Ok((
                serde_json::to_string(&record.key_package_id)?,
                serde_json::to_string(&record.owner)?,
                serde_json::to_string(&record.state)?,
            ))
        })
        .collect::<Result<Vec<(String, String, String)>, DurableStoreError>>()?;

    fold_seed_in_one_transaction(store, &seed, &inventory_rows)
}

/// Do the pre-cutover tables exist in `conn`? `http_delivery_ops` is the
/// sentinel: every legacy-engine boot created it, and this build never
/// does.
fn legacy_tables_present(conn: &Connection) -> Result<bool, DurableStoreError> {
    let exists: i64 = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'http_delivery_ops')",
        [],
        |row| row.get(0),
    )?;
    Ok(exists == 1)
}

/// Boot the legacy READER at `path` and extract the fold's seed. This is
/// the legacy `from_sqlite_path` reduced to what the fold transplants: the
/// snapshot+tail service rebuild, the inventory/revocation rebuild, and the
/// #770 reconciliation over the durable projection rows. Loads that never
/// fed the fold (pairing sessions, profiles, push, blobs, idempotency
/// caches) are skipped — the normalized boot loads them itself right after
/// the fold, through its own connection.
fn boot_legacy_seed(path: &Path) -> Result<LegacyEngineSeed, DurableStoreError> {
    let mut conn = open_legacy_connection(path)?;
    let (mut service, mut inventory, mut revoked_devices, snapshot_seq) =
        match load_state_snapshot(&conn)? {
            Some((seq, snapshot)) => (
                snapshot.service,
                snapshot
                    .key_package_inventory
                    .into_iter()
                    .map(|record| (record.key_package_id.clone(), record))
                    .collect(),
                snapshot.revoked_devices,
                seq,
            ),
            None => (
                HttpDeliveryService::with_limits(finite_delivery_limits()),
                HashMap::new(),
                BTreeSet::new(),
                0,
            ),
        };
    let operations = load_operations_after(&conn, snapshot_seq)?;
    let mut replayed_group_publishes = Vec::new();
    for operation in operations.iter().cloned() {
        replay_operation(&mut service, operation, &mut replayed_group_publishes)?;
    }
    apply_operations_to_key_package_inventory(&mut inventory, &operations);
    apply_operations_to_revoked_devices(&mut revoked_devices, &operations);
    if snapshot_seq == 0
        && !key_package_inventory_cache_matches(&load_key_package_inventory(&conn)?, &inventory)
    {
        for record in inventory.values() {
            upsert_key_package_inventory(&conn, record)?;
        }
    }
    let mut directory = load_account_room_directory(&conn)?;
    let mut rooms = load_room_memberships(&conn)?;
    let welcome_claims = load_welcome_claims(&conn)?;
    // The op log is authoritative: when the durable projection rows lag
    // the replayed tail (the frozen-table boot observed on lat2,
    // 2026-08-29), reconcile them from the same entries the service was
    // rebuilt from, or every current-epoch publish is rejected against a
    // stale epoch/last_seq and nothing durable advances again.
    reconcile_room_projections_with_replayed_log(
        &mut rooms,
        &mut directory,
        &replayed_group_publishes,
        &welcome_claims,
        &mut conn,
    )?;
    Ok(LegacyEngineSeed {
        routes: service.route_snapshots(),
        key_packages: service.key_package_snapshots(),
        rooms,
        directory,
        revoked_devices,
        inventory: inventory.values().cloned().collect(),
    })
}

/// Open the legacy database for the reader. Same durability PRAGMAs the
/// legacy store used: the reconciliation persist is a real write. The
/// reader creates NO tables — it only ever runs on databases the legacy
/// engine already shaped (see [`legacy_tables_present`]).
fn open_legacy_connection(path: &Path) -> Result<Connection, DurableStoreError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(conn)
}

fn load_state_snapshot(
    conn: &Connection,
) -> Result<Option<(i64, DurableStateSnapshot)>, DurableStoreError> {
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
    // The uncompressed v1 table stopped being written when v2 landed and
    // is no longer minted, but databases booted by older builds still
    // carry it. A v1 row with no v2 successor is ambiguous, not empty:
    // the old cross-generation MIN() prune may have cut the op log down
    // to that row's horizon, so replaying from op zero could silently
    // discard history. Fail closed (the fold refuses, nothing is mutated)
    // and let an operator recover — e.g. boot a v2-writing build once to
    // mint a successor snapshot, or restore from backup.
    let legacy_seq: Option<i64> = if conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'http_state_snapshots')",
        [],
        |row| row.get::<_, i64>(0),
    )? == 1
    {
        conn.query_row(
            "SELECT last_op_seq FROM http_state_snapshots WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
    } else {
        None
    };
    if let Some(last_op_seq) = legacy_seq {
        return Err(DurableStoreError::LegacySnapshotWithoutV2Successor { last_op_seq });
    }
    Ok(None)
}

fn load_operations_after(
    conn: &Connection,
    after_seq: i64,
) -> Result<Vec<PersistedOperation>, DurableStoreError> {
    let mut statement =
        conn.prepare("SELECT body_json FROM http_delivery_ops WHERE seq > ?1 ORDER BY seq ASC")?;
    let rows = statement.query_map(params![after_seq], |row| row.get::<_, String>(0))?;
    let mut operations = Vec::new();
    for row in rows {
        operations.push(serde_json::from_str(&row?)?);
    }
    Ok(operations)
}

fn load_key_package_inventory(
    conn: &Connection,
) -> Result<HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>, DurableStoreError> {
    crate::store::metadata::load_key_package_inventory(conn)
}

fn upsert_key_package_inventory(
    conn: &Connection,
    record: &KeyPackageInventoryRecord,
) -> Result<(), DurableStoreError> {
    crate::store::metadata::upsert_key_package_inventory(conn, record)
}

fn load_account_room_directory(
    conn: &Connection,
) -> Result<BTreeMap<String, BTreeMap<String, Value>>, DurableStoreError> {
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

fn load_room_memberships(
    conn: &Connection,
) -> Result<BTreeMap<String, HttpRoomMembershipProjection>, DurableStoreError> {
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

fn load_welcome_claims(
    conn: &Connection,
) -> Result<HashMap<MessageId, WelcomeClaimRecord>, DurableStoreError> {
    crate::store::metadata::load_welcome_claims(conn)
}

/// Persist one boot-reconciliation pass atomically: every repaired
/// `http_room_memberships` row and every `http_account_rooms`
/// delete/upsert moves in a single SQLite transaction. A crash or SQLite error
/// between the two tables must never advance the membership watermark
/// past directory writes a later boot would skip
/// (`publish.seq <= projection.last_seq`), which would strand the
/// directory stale forever.
fn persist_room_reconciliation(
    conn: &mut Connection,
    projections: &[HttpRoomMembershipProjection],
    directory_mutation: &AccountRoomDirectoryMutation,
) -> Result<(), DurableStoreError> {
    let transaction = conn.transaction()?;
    for projection in projections {
        transaction.execute(
            "INSERT INTO http_room_memberships (room_id, projection_json)
                 VALUES (?1, ?2)
                 ON CONFLICT(room_id) DO UPDATE SET
                    projection_json = excluded.projection_json",
            params![projection.room_id, serde_json::to_string(projection)?],
        )?;
    }
    for (account_id, room_id) in &directory_mutation.deletes {
        transaction.execute(
            "DELETE FROM http_account_rooms WHERE account_id = ?1 AND room_id = ?2",
            params![account_id, room_id],
        )?;
    }
    for record in &directory_mutation.upserts {
        transaction.execute(
            "INSERT INTO http_account_rooms (account_id, room_id, record_json)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, room_id) DO UPDATE SET
                    record_json = excluded.record_json",
            params![
                record.account_id,
                record.room_id,
                serde_json::to_string(&record.record)?
            ],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

/// Boot-time repair for durable Finite room state that lags the replayed
/// op log: replay each room's group entries above the projection row's
/// `last_seq` through the same semantics the live path applied
/// (membership deltas for typed commits — to both the projection and the
/// account-room directory — head advances for everything else), then
/// activate intervals whose Welcomes were already ACKED durably. The
/// service rebuilt from the op log stays authoritative for heads; after
/// this the transplanted service head, the projection, the directory, and
/// future seq assignment agree, so current-epoch clients can publish and
/// the normalized engine accepts them post-fold.
fn reconcile_room_projections_with_replayed_log(
    rooms: &mut BTreeMap<String, HttpRoomMembershipProjection>,
    directory: &mut BTreeMap<String, BTreeMap<String, Value>>,
    replayed: &[ReplayedGroupPublish],
    welcome_claims: &HashMap<MessageId, WelcomeClaimRecord>,
    conn: &mut Connection,
) -> Result<(), DurableStoreError> {
    let mut changed: BTreeSet<String> = BTreeSet::new();
    let mut directory_mutation = AccountRoomDirectoryMutation::default();
    for publish in replayed {
        let Some(projection) = rooms.get(&publish.room_id) else {
            // Rooms without a durable row keep using the typed bootstrap
            // path, which derives its projection from the log already.
            continue;
        };
        if publish.seq <= projection.last_seq {
            // The durable rows already reflect everything through the
            // projection's head: the live paths persisted the projection
            // and directory rows in one transaction. Replaying below the
            // watermark could resurrect state that later durable writes
            // (e.g. a leave) legitimately removed.
            continue;
        }
        // Typed commit payloads carry the membership delta the live path
        // applied at this seq; every other entry only advances the head.
        let commit =
            serde_json::from_slice::<FiniteAccountRoomCommitProjection>(&publish.message.payload)
                .ok()
                .filter(|commit| {
                    commit.entry.room_id == publish.room_id
                        && commit.entry.kind == LogEntryKind::Commit
                });
        // The account-room directory mirrors the same delta (the live
        // commit path updated both maps in one durable transaction).
        if let Some(commit) = &commit
            && let Ok(mutation) = apply_account_room_membership_delta(
                directory,
                &publish.room_id,
                &projection.mls_group_id,
                commit.membership_delta.post_commit_epoch,
                &commit.membership_delta,
                publish.seq,
            )
        {
            directory_mutation.deletes.extend(mutation.deletes);
            directory_mutation.upserts.extend(mutation.upserts);
        }
        if let Some(commit) = commit {
            let mls_group_id = projection.mls_group_id.clone();
            if let Err(error) = apply_room_membership_delta(
                rooms,
                &publish.room_id,
                &mls_group_id,
                &commit.entry.sender,
                commit.entry.epoch,
                &commit.membership_delta,
                publish.seq,
            ) {
                // The frozen row predates the replayed window's epoch
                // chain (or refuses one of its deltas). Fall back to the
                // bootstrap rule — the log is authoritative for heads —
                // so the fold continues; membership intervals stay at
                // their frozen state and the skew is loud.
                eprintln!(
                    "finitechat-server: room {} projection replay broke at seq {} ({:?}); advancing head only",
                    publish.room_id, publish.seq, error
                );
                let observed_epoch = commit.entry.epoch.saturating_add(1);
                if let Some(projection) = rooms.get_mut(&publish.room_id) {
                    projection.last_seq = publish.seq;
                    projection.current_epoch = projection.current_epoch.max(observed_epoch);
                }
            }
        } else if let Some(projection) = rooms.get_mut(&publish.room_id) {
            projection.last_seq = publish.seq;
        }
        changed.insert(publish.room_id.clone());
    }
    // Welcome ACKs are durable delivery events, not group-log entries:
    // the delta replay above re-creates added intervals (and directory
    // rows) as pending, so activate the ones whose Welcomes were already
    // acked before this boot. Mere claims stay pending, exactly like the
    // live claim/ack routes.
    for claim in welcome_claims.values() {
        if claim.state != WelcomeClaimState::Acked {
            continue;
        }
        let Ok(welcome) = serde_json::from_slice::<WelcomeRecord>(&claim.message.payload) else {
            continue;
        };
        if claim.message.id.as_slice() != welcome.welcome_id.as_bytes() {
            continue;
        }
        let Some(projection) = rooms.get_mut(&welcome.room_id) else {
            continue;
        };
        if projection.activate_interval(&welcome.recipient, welcome.commit_seq) {
            changed.insert(welcome.room_id.clone());
        }
        // Mirror the ack path's directory activation for the recipient.
        if let Some(record) = activate_account_room_device_in_directory(
            directory,
            &welcome.recipient,
            &welcome.room_id,
        ) {
            directory_mutation.upserts.push(record);
        }
    }
    // One transaction moves both tables together — the same coupling the
    // live commit path had. Failing closed here is the safe outcome.
    let repaired_projections = changed
        .into_iter()
        .filter_map(|room_id| rooms.get(&room_id).cloned())
        .collect::<Vec<_>>();
    persist_room_reconciliation(conn, &repaired_projections, &directory_mutation)?;
    Ok(())
}

/// Replay one persisted operation into the rebuilt service. Group publishes
/// are also recorded (room, message, assigned seq) so the reconciliation
/// above can compare the durable projections with the log the service was
/// rebuilt from.
fn replay_operation(
    service: &mut HttpDeliveryService,
    operation: PersistedOperation,
    replayed_group_publishes: &mut Vec<ReplayedGroupPublish>,
) -> Result<(), DurableStoreError> {
    match operation {
        PersistedOperation::PublishMessage {
            target, message, ..
        } => {
            let receipt = service.publish(target.clone(), message.clone())?;
            if let HttpPublishTarget::Group { group_id, .. } = &target
                && let Ok(room_id) = String::from_utf8(group_id.as_slice().to_vec())
            {
                replayed_group_publishes.push(ReplayedGroupPublish {
                    room_id,
                    message,
                    seq: receipt.seq,
                });
            }
        }
        // KeyPackage lease/reclaim/consume state is rebuilt in the finite
        // wrapper inventory; Finite Chat's core store has no claimed lease
        // state.
        PersistedOperation::PublishKeyPackage { .. } => {}
        PersistedOperation::RevokeDevice { .. } => {}
        PersistedOperation::ClaimKeyPackage { .. }
        | PersistedOperation::ClaimKeyPackages { .. }
        | PersistedOperation::ExpireKeyPackageLease { .. } => {}
    }
    Ok(())
}

fn apply_operations_to_revoked_devices(
    revoked: &mut BTreeSet<String>,
    operations: &[PersistedOperation],
) {
    for operation in operations {
        if let PersistedOperation::RevokeDevice { device } = operation {
            revoked.insert(DeviceMembership::key(device));
        }
    }
}

fn apply_operations_to_key_package_inventory(
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

fn key_package_inventory_cache_matches(
    cached: &HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    rebuilt: &HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
) -> bool {
    cached.len() == rebuilt.len()
        && rebuilt.iter().all(|(key_package_id, rebuilt_record)| {
            cached.get(key_package_id).is_some_and(|cached_record| {
                cached_record.owner == rebuilt_record.owner
                    && cached_record.state == rebuilt_record.state
            })
        })
}

fn fold_seed_in_one_transaction(
    store: &Store,
    seed: &LegacyEngineSeed,
    inventory_rows: &[(String, String, String)],
) -> Result<Option<FoldReport>, DurableStoreError> {
    store
        .write(|tx| {
            // Re-check the marker under the write lock so a racing boot that
            // folded between our read and this transaction is a no-op here.
            if crate::store::room_state::fold_complete(tx)? {
                return Ok(None);
            }

            let mut report = FoldReport::default();
            for route in &seed.routes {
                let plane = match route.plane {
                    HttpDeliveryPlane::Group => "group",
                    HttpDeliveryPlane::Inbox => "inbox",
                };
                tx.execute(
                    "INSERT INTO delivery_routes (plane, route_key, last_seq)
                     VALUES (?1, ?2, 0)",
                    params![plane, &route.route_key],
                )?;
                let route_id = tx.last_insert_rowid();
                let head = route.entries.last().map_or(0, |entry| entry.seq);
                for queued in &route.entries {
                    insert_entry(tx, route_id, queued.seq, &queued.message, digest_of(queued))?;
                    report.entries += 1;
                }
                // The legacy service records only WHICH source epochs already
                // admitted a commit, not the per-entry seq (that detail is
                // pruned with the ops below the snapshot horizon). Admission
                // checks read existence only, so the epochs seed at the route
                // head — an upper bound, truthful about ordering, and exact
                // for every epoch admitted after the last snapshot.
                for epoch in &route.accepted_commit_epochs {
                    tx.execute(
                        "INSERT INTO group_commit_epochs (route_id, source_epoch, seq)
                         VALUES (?1, ?2, ?3)",
                        params![route_id, u64_to_db(epoch.0)?, u64_to_db(head)?],
                    )?;
                    report.commit_epochs += 1;
                }
                tx.execute(
                    "UPDATE delivery_routes SET last_seq = ?2 WHERE route_id = ?1",
                    params![route_id, u64_to_db(head)?],
                )?;
                report.routes += 1;
            }
            let mut folded_key_package_ids: BTreeSet<Vec<u8>> = seed
                .key_packages
                .iter()
                .map(|key_package| key_package.key_package_id.as_slice().to_vec())
                .collect();
            report.key_packages = folded_key_package_ids.len();
            for key_package in &seed.key_packages {
                let source_json = key_package
                    .key_package
                    .source
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
                tx.execute(
                    "INSERT INTO sql_key_packages
                         (key_package_id, owner, key_package_bytes, key_package_source_json, state)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        key_package.key_package_id.as_slice(),
                        key_package.owner.as_slice(),
                        &key_package.key_package.bytes,
                        source_json,
                        if key_package.consumed {
                            "consumed"
                        } else {
                            "available"
                        },
                    ],
                )?;
            }
            // The service rebuild does NOT replay `PublishKeyPackage` ops
            // (the core store holds no wrapper lease state), so the snapshots
            // above only cover packages that predate the legacy v2 snapshot
            // horizon. Packages published AFTER that horizon exist only in
            // the replayed wrapper inventory — with their payload bytes. Give
            // every such record a durable payload row too, or the re-seeded
            // inventory row would be an id/owner/state triple with no bytes:
            // the normalized boot would enrich it to an empty payload and a
            // claim could return empty bytes (review #799). Records already
            // folded from the service snapshots keep their row untouched —
            // for overlapping ids both views carry the same publication
            // bytes, and the wrapper state stays authoritative in the shared
            // inventory table.
            for record in &seed.inventory {
                if !folded_key_package_ids.insert(record.key_package_id.as_slice().to_vec()) {
                    continue;
                }
                report.key_packages += 1;
                crate::store::metadata::upsert_key_package_payload(tx, record)?;
            }
            for (account_id, rooms) in &seed.directory {
                for (room_id, record) in rooms {
                    tx.execute(
                        "INSERT INTO account_room_directory (account_id, room_id, record_json)
                         VALUES (?1, ?2, ?3)",
                        params![account_id, room_id, record.to_string()],
                    )?;
                    report.directory_rows += 1;
                }
            }
            for device_key in &seed.revoked_devices {
                tx.execute(
                    "INSERT INTO revoked_devices (device_key) VALUES (?1)",
                    params![device_key],
                )?;
                report.revoked_devices += 1;
            }
            // The finite KeyPackage inventory table is shared state, but the
            // legacy boot treats it as a repair cache for the snapshot+log
            // replay. Re-seed it wholesale from the replayed state inside
            // the fold transaction so the normalized engine (which treats
            // the table as current state) cannot inherit a lagging row.
            tx.execute("DELETE FROM http_key_package_inventory", [])?;
            for (key_package_id_json, owner_json, state_json) in inventory_rows {
                tx.execute(
                    "INSERT INTO http_key_package_inventory (
                        key_package_id_json, owner_json, state_json
                    ) VALUES (?1, ?2, ?3)",
                    params![key_package_id_json, owner_json, state_json],
                )?;
            }
            let checkpoint = RoomStateCheckpoint {
                rooms: seed.rooms.clone(),
            };
            save_checkpoint(tx, &checkpoint)?;
            report.checkpoint_rooms = seed.rooms.len();

            assert_fold(tx, &mut report, seed)?;

            mark_fold_complete(tx, CHAT_ENGINE_CUTOVER_DATE)?;
            Ok(Some(report))
        })
        .map_err(fold_error)
}

/// Row counts plus a deterministic sampled fold, asserted INSIDE the
/// transaction before the marker commits. Any mismatch aborts the whole fold
/// (the transaction rolls back; boot fails closed).
fn assert_fold(
    tx: &rusqlite::Transaction<'_>,
    report: &mut FoldReport,
    seed: &LegacyEngineSeed,
) -> Result<(), DurableStoreError> {
    report.sampled_routes = 0;
    report.sampled_directory_rows = 0;
    let count = |sql: &str| -> Result<i64, DurableStoreError> {
        Ok(tx.query_row(sql, [], |row| row.get(0))?)
    };
    let checks = [
        ("delivery_routes", report.routes),
        ("delivery_entries", report.entries),
        ("group_commit_epochs", report.commit_epochs),
        ("sql_key_packages", report.key_packages),
        ("account_room_directory", report.directory_rows),
        ("revoked_devices", report.revoked_devices),
    ];
    for (table, expected) in checks {
        let actual = count(&format!("SELECT COUNT(*) FROM {table}"))?;
        if usize::try_from(actual).unwrap_or(usize::MAX) != expected {
            return Err(DurableStoreError::FoldAssertionFailed {
                details: format!(
                    "row count mismatch folding into {table}: expected {expected}, found {actual}"
                ),
            });
        }
    }

    // Every re-seeded inventory row must have a durable payload home in
    // `sql_key_packages`, and wherever the replay carries payload bytes they
    // must be the stored bytes. An inventory row without a payload row is
    // exactly the empty-claim class this cutover exists to close, so a
    // mismatch aborts the whole fold (review #799).
    for record in &seed.inventory {
        let stored: Option<Vec<u8>> = tx
            .query_row(
                "SELECT key_package_bytes FROM sql_key_packages WHERE key_package_id = ?1",
                params![record.key_package_id.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(stored_bytes) = stored else {
            return Err(DurableStoreError::FoldAssertionFailed {
                details: format!(
                    "key package {:?} re-seed into the inventory has no durable payload row",
                    record.key_package_id
                ),
            });
        };
        let replayed_bytes = record.key_package.bytes();
        if !replayed_bytes.is_empty() && stored_bytes != replayed_bytes {
            return Err(DurableStoreError::FoldAssertionFailed {
                details: format!(
                    "key package {:?} payload differs from the replayed publication bytes",
                    record.key_package_id
                ),
            });
        }
    }

    // Sampled fold: first, last, and evenly spaced routes; every sampled
    // route's entries must match the legacy service's queue byte-for-byte
    // (seq, message id, digest) and its head must equal the queue head.
    let route_samples = sample_indexes(seed.routes.len(), SAMPLED_ROUTES);
    report.sampled_routes = route_samples.len();
    for index in route_samples {
        let route = &seed.routes[index];
        let plane = match route.plane {
            HttpDeliveryPlane::Group => "group",
            HttpDeliveryPlane::Inbox => "inbox",
        };
        let route_id: i64 = tx.query_row(
            "SELECT route_id FROM delivery_routes WHERE plane = ?1 AND route_key = ?2",
            params![plane, &route.route_key],
            |row| row.get(0),
        )?;
        let stored_head: i64 = tx.query_row(
            "SELECT last_seq FROM delivery_routes WHERE route_id = ?1",
            params![route_id],
            |row| row.get(0),
        )?;
        let expected_head = route.entries.last().map_or(0, |entry| entry.seq);
        if db_to_u64(stored_head)? != expected_head {
            return Err(DurableStoreError::FoldAssertionFailed {
                details: format!(
                    "folded head mismatch for {:?}: expected {expected_head}, found {stored_head}",
                    route.route_key
                ),
            });
        }
        let mut statement = tx.prepare(
            "SELECT seq, message_id, digest FROM delivery_entries
             WHERE route_id = ?1 ORDER BY seq",
        )?;
        let folded_rows = statement
            .query_map(params![route_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if folded_rows.len() != route.entries.len() {
            return Err(DurableStoreError::FoldAssertionFailed {
                details: format!(
                    "folded entry count mismatch for {:?}: expected {}, found {}",
                    route.route_key,
                    route.entries.len(),
                    folded_rows.len()
                ),
            });
        }
        for (folded, queued) in folded_rows.iter().zip(&route.entries) {
            if db_to_u64(folded.0)? != queued.seq
                || folded.1 != queued.message.id.as_slice()
                || folded.2.as_slice() != digest_of(queued).as_slice()
            {
                return Err(DurableStoreError::FoldAssertionFailed {
                    details: format!(
                        "folded entry mismatch for {:?} at seq {}",
                        route.route_key, queued.seq
                    ),
                });
            }
        }
    }

    // Sampled directory rows must round-trip byte-equal against the legacy
    // engine's post-reconciliation directory map.
    let flat_directory = seed
        .directory
        .iter()
        .flat_map(|(account_id, rooms)| {
            rooms
                .iter()
                .map(move |(room_id, record)| (account_id.clone(), room_id.clone(), record))
        })
        .collect::<Vec<_>>();
    let directory_samples = sample_indexes(flat_directory.len(), SAMPLED_ROUTES);
    report.sampled_directory_rows = directory_samples.len();
    for index in directory_samples {
        let (account_id, room_id, record) = &flat_directory[index];
        let stored: String = tx
            .query_row(
                "SELECT record_json FROM account_room_directory
             WHERE account_id = ?1 AND room_id = ?2",
                params![account_id, room_id],
                |row| row.get(0),
            )
            .map_err(|error| DurableStoreError::FoldAssertionFailed {
                details: format!(
                    "sampled directory row {account_id}/{room_id} missing after fold: {error}"
                ),
            })?;
        if stored != record.to_string() {
            return Err(DurableStoreError::FoldAssertionFailed {
                details: format!(
                    "sampled directory row {account_id}/{room_id} differs from the legacy engine's map"
                ),
            });
        }
    }
    Ok(())
}

fn sample_indexes(len: usize, cap: usize) -> Vec<usize> {
    if len == 0 {
        return Vec::new();
    }
    if len <= cap {
        return (0..len).collect();
    }
    let mut indexes = Vec::with_capacity(cap);
    for step in 0..cap {
        indexes.push(step * (len - 1) / (cap - 1));
    }
    indexes.dedup();
    indexes
}

fn digest_of(queued: &finitechat_delivery::HttpQueuedDelivery) -> [u8; 32] {
    finitechat_delivery::digest_transport_message(&queued.message)
}

/// Widen the folded-transaction error so a fold failure names itself in logs
/// (StoreWriteError::Domain cannot happen here; every failure is storage).
fn fold_error(error: crate::store::StoreWriteError) -> DurableStoreError {
    match error {
        crate::store::StoreWriteError::Store(error) => error,
        crate::store::StoreWriteError::Domain(error) => DurableStoreError::Replay(error),
    }
}
