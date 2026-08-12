//! SQLite-backed implementation of the upstream delivery contract.
//!
//! [`SqlDelivery`] implements [`finitechat_delivery::HttpDelivery`] directly
//! against the normalized tables in [`super::schema`]: every publish runs in
//! one `BEGIN IMMEDIATE` transaction, sequences are allocated from the route
//! head (`delivery_routes.last_seq`) under the write lock, and duplicate
//! detection persists the canonical
//! [`finitechat_delivery::digest_transport_message`] digest. Sync reads run
//! on the store's `query_only` pool.
//!
//! The engine is proven by the upstream conformance suite in this module's
//! unit tests (file-backed with real restarts, plus in-memory).

use finitechat_delivery::{
    HttpClaimedKeyPackage, HttpCommitAdmission, HttpDelivery, HttpDeliveryLimits,
    HttpDeliveryPlane, HttpKeyPackageId, HttpKeyPackagePublication, HttpPublishReceipt,
    HttpPublishTarget, HttpQueuedDelivery, HttpSequence, HttpServerError, HttpSyncPage,
    MAX_HTTP_ID_BYTES, MAX_HTTP_KEY_PACKAGE_BYTES, MAX_HTTP_MESSAGE_CAUSAL_DEPS,
    MAX_HTTP_MESSAGE_PAYLOAD_BYTES, MAX_HTTP_SOURCE_BYTES, MAX_HTTP_SYNC_PAGE_ENTRIES,
    MAX_HTTP_TRANSPORT_GROUP_ID_BYTES, digest_transport_message,
};
use finitechat_transport::engine::{KeyPackage, KeyPackageSource};
use finitechat_transport::transport::{
    Timestamp, TransportEnvelope, TransportMessage, TransportSource,
};
use finitechat_transport::{GroupId, MemberId, MessageId};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{Store, StoreTxError, StoreWriteError, db_to_u64, u64_to_db};

/// `delivery_routes.plane` value for group routes.
const PLANE_GROUP: &str = "group";
/// `delivery_routes.plane` value for inbox routes.
const PLANE_INBOX: &str = "inbox";

/// `delivery_entries.envelope_kind` for [`TransportEnvelope::GroupMessage`]
/// (`envelope_ref` holds the transport group id).
const ENVELOPE_KIND_GROUP_MESSAGE: i64 = 0;
/// `delivery_entries.envelope_kind` for [`TransportEnvelope::Welcome`]
/// (`envelope_ref` holds the recipient member id).
const ENVELOPE_KIND_WELCOME: i64 = 1;

/// The normalized-SQLite delivery engine.
///
/// The inherent methods return [`StoreWriteError`] so callers (the eventual
/// server layer) can distinguish delivery-contract rejections (`Domain`) from
/// storage failures (`Store`). The [`HttpDelivery`] trait impl is a thin
/// adapter over them for the conformance suite.
pub(crate) struct SqlDelivery {
    store: Store,
    limits: HttpDeliveryLimits,
}

impl SqlDelivery {
    pub(crate) fn new(store: Store, limits: HttpDeliveryLimits) -> Self {
        Self { store, limits }
    }

    pub(crate) fn limits(&self) -> HttpDeliveryLimits {
        self.limits
    }

    pub(crate) fn publish(
        &self,
        target: HttpPublishTarget,
        message: TransportMessage,
    ) -> Result<HttpPublishReceipt, StoreWriteError> {
        let limits = self.limits;
        self.store.write(move |tx| {
            // Check order mirrors HttpDeliveryService::publish (D:405-428).
            validate_transport_message(&message)?;
            validate_target_matches_message(&target, &message)?;
            let digest = digest_transport_message(&message);
            match target {
                HttpPublishTarget::Group {
                    group_id,
                    transport_group_id,
                    commit_admission,
                } => {
                    validate_group_id(&group_id)?;
                    validate_transport_group_id(&transport_group_id)?;
                    publish_group(tx, limits, &group_id, commit_admission, &message, digest)
                }
                HttpPublishTarget::Inbox { recipient } => {
                    validate_member_id("recipient", &recipient)?;
                    publish_inbox(tx, limits, &recipient, &message, digest)
                }
            }
        })
    }

    pub(crate) fn sync_group(
        &self,
        group_id: &GroupId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, StoreWriteError> {
        // Validation order mirrors HttpDeliveryService::sync_group
        // (D:430-442): ids first, then the page limit, then route lookup.
        validate_group_id(group_id).map_err(StoreWriteError::Domain)?;
        validate_page_limit(limit).map_err(StoreWriteError::Domain)?;
        self.store
            .read(|conn| sync_route(conn, PLANE_GROUP, group_id.as_slice(), after_seq, limit))
    }

    pub(crate) fn sync_inbox(
        &self,
        recipient: &MemberId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, StoreWriteError> {
        // Mirrors HttpDeliveryService::sync_inbox (D:444-456).
        validate_member_id("recipient", recipient).map_err(StoreWriteError::Domain)?;
        validate_page_limit(limit).map_err(StoreWriteError::Domain)?;
        self.store
            .read(|conn| sync_route(conn, PLANE_INBOX, recipient.as_slice(), after_seq, limit))
    }

    pub(crate) fn publish_key_package(
        &self,
        publication: HttpKeyPackagePublication,
    ) -> Result<(), StoreWriteError> {
        let limits = self.limits;
        self.store.write(move |tx| {
            // Mirrors HttpDeliveryService::publish_key_package (D:458-491).
            validate_key_package_publication(&publication)?;
            let existing: Option<(Vec<u8>, Vec<u8>)> = tx
                .query_row(
                    "SELECT owner, key_package_bytes FROM sql_key_packages
                     WHERE key_package_id = ?1",
                    params![publication.key_package_id.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(StoreTxError::Sqlite)?;
            if let Some((owner, bytes)) = existing {
                // Idempotent when the stored record matches the publication.
                // KeyPackage equality is bytes-only (finitechat-transport
                // engine.rs:39-45), so the optional source provenance is
                // deliberately not part of the match, exactly like
                // key_package_record_matches (D:873-878).
                if owner == publication.owner.as_slice() && bytes == publication.key_package.bytes {
                    return Ok(());
                }
                return Err(HttpServerError::ConflictingKeyPackage {
                    key_package_id: publication.key_package_id.clone(),
                }
                .into());
            }
            // Cap counts every unconsumed package for the owner (D:471-481);
            // 'claimed' exists for a later lease phase and counts as
            // unconsumed.
            let unconsumed: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM sql_key_packages
                     WHERE owner = ?1 AND state != 'consumed'",
                    params![publication.owner.as_slice()],
                    |row| row.get(0),
                )
                .map_err(StoreTxError::Sqlite)?;
            if db_to_u64(unconsumed)? >= limits.max_key_packages_per_account as u64 {
                return Err(HttpServerError::KeyPackageInventoryFull {
                    owner: publication.owner.clone(),
                    max: limits.max_key_packages_per_account,
                }
                .into());
            }
            let source_json = publication
                .key_package
                .source
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            tx.execute(
                "INSERT INTO sql_key_packages
                     (key_package_id, owner, key_package_bytes, key_package_source_json, state)
                 VALUES (?1, ?2, ?3, ?4, 'available')",
                params![
                    publication.key_package_id.as_slice(),
                    publication.owner.as_slice(),
                    publication.key_package.bytes,
                    source_json,
                ],
            )
            .map_err(StoreTxError::Sqlite)?;
            Ok(())
        })
    }

    pub(crate) fn claim_key_package(
        &self,
        owner: &MemberId,
    ) -> Result<Option<HttpClaimedKeyPackage>, StoreWriteError> {
        self.store.write(|tx| {
            // Mirrors HttpDeliveryService::claim_key_package (D:493-519).
            validate_member_id("owner", owner)?;
            // Deterministic claim order: the smallest available id. SQLite
            // orders BLOBs by memcmp, which matches the reference's
            // Vec<u8> lexicographic min (D:498-505).
            let selected: Option<(Vec<u8>, Vec<u8>, Option<String>)> = tx
                .query_row(
                    "SELECT key_package_id, key_package_bytes, key_package_source_json
                     FROM sql_key_packages
                     WHERE owner = ?1 AND state = 'available'
                     ORDER BY key_package_id
                     LIMIT 1",
                    params![owner.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(StoreTxError::Sqlite)?;
            let Some((key_package_id, bytes, source_json)) = selected else {
                return Ok(None);
            };
            tx.execute(
                "UPDATE sql_key_packages SET state = 'consumed' WHERE key_package_id = ?1",
                params![key_package_id],
            )
            .map_err(StoreTxError::Sqlite)?;
            let source: Option<KeyPackageSource> = source_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
            Ok(Some(HttpClaimedKeyPackage {
                key_package_id: HttpKeyPackageId::new(key_package_id),
                owner: owner.clone(),
                key_package: KeyPackage { bytes, source },
            }))
        })
    }
}

/// [`HttpDelivery`] adapter over the inherent methods.
///
/// The delivery contract has no infrastructure-failure variant, so a
/// [`StoreWriteError::Store`] here is fatal — the same stance the existing
/// conformance adapter takes in `tests/http_conformance.rs`. The eventual
/// server layer calls the inherent methods instead and maps `Store` to its
/// existing 500 path.
impl HttpDelivery for SqlDelivery {
    fn publish(
        &mut self,
        target: HttpPublishTarget,
        message: TransportMessage,
    ) -> Result<HttpPublishReceipt, HttpServerError> {
        expect_domain(SqlDelivery::publish(self, target, message))
    }

    fn sync_group(
        &self,
        group_id: &GroupId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, HttpServerError> {
        expect_domain(SqlDelivery::sync_group(self, group_id, after_seq, limit))
    }

    fn sync_inbox(
        &self,
        recipient: &MemberId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, HttpServerError> {
        expect_domain(SqlDelivery::sync_inbox(self, recipient, after_seq, limit))
    }

    fn publish_key_package(
        &mut self,
        publication: HttpKeyPackagePublication,
    ) -> Result<(), HttpServerError> {
        expect_domain(SqlDelivery::publish_key_package(self, publication))
    }

    fn claim_key_package(
        &mut self,
        owner: &MemberId,
    ) -> Result<Option<HttpClaimedKeyPackage>, HttpServerError> {
        expect_domain(SqlDelivery::claim_key_package(self, owner))
    }
}

fn expect_domain<T>(result: Result<T, StoreWriteError>) -> Result<T, HttpServerError> {
    match result {
        Ok(value) => Ok(value),
        Err(StoreWriteError::Domain(error)) => Err(error),
        Err(StoreWriteError::Store(error)) => panic!("SQL delivery store failure: {error}"),
    }
}

/// One `delivery_routes` row: the queue head for a (plane, route_key) pair.
struct Route {
    route_id: i64,
    last_seq: HttpSequence,
}

fn publish_group(
    tx: &Transaction<'_>,
    limits: HttpDeliveryLimits,
    group_id: &GroupId,
    commit_admission: Option<HttpCommitAdmission>,
    message: &TransportMessage,
    digest: [u8; 32],
) -> Result<HttpPublishReceipt, StoreTxError> {
    let route = match lookup_route(tx, PLANE_GROUP, group_id.as_slice())? {
        Some(route) => {
            // Existing route check order mirrors GroupQueue::check_append
            // (D:574-606): duplicate replay first, then commit-epoch
            // admission, then queue space.
            if let Some(receipt) = replay_or_reject_duplicate(
                tx,
                &route,
                HttpDeliveryPlane::Group,
                &message.id,
                digest,
            )? {
                return Ok(receipt);
            }
            if let Some(admission) = commit_admission {
                let taken: Option<i64> = tx
                    .query_row(
                        "SELECT seq FROM group_commit_epochs
                         WHERE route_id = ?1 AND source_epoch = ?2",
                        params![route.route_id, u64_to_db(admission.source_epoch.0)?],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(StoreTxError::Sqlite)?;
                if taken.is_some() {
                    return Err(HttpServerError::StaleEpoch {
                        source_epoch: admission.source_epoch,
                    }
                    .into());
                }
            }
            ensure_queue_has_space(
                HttpDeliveryPlane::Group,
                route.last_seq,
                limits.max_queue_entries_per_route,
            )?;
            route
        }
        None => {
            // New route: enforce the group cap before creating (D:532-536).
            let groups = count_routes(tx, PLANE_GROUP)?;
            if groups >= limits.max_groups as u64 {
                return Err(HttpServerError::GroupLimitExceeded {
                    max: limits.max_groups,
                }
                .into());
            }
            create_route(tx, PLANE_GROUP, group_id.as_slice())?
        }
    };
    let seq = route.last_seq + 1;
    insert_entry(tx, route.route_id, seq, message, digest)?;
    if let Some(admission) = commit_admission {
        tx.execute(
            "INSERT INTO group_commit_epochs (route_id, source_epoch, seq) VALUES (?1, ?2, ?3)",
            params![
                route.route_id,
                u64_to_db(admission.source_epoch.0)?,
                u64_to_db(seq)?
            ],
        )
        .map_err(StoreTxError::Sqlite)?;
    }
    bump_route_head(tx, route.route_id, seq)?;
    Ok(HttpPublishReceipt {
        message_id: message.id.clone(),
        plane: HttpDeliveryPlane::Group,
        seq,
        duplicate: false,
    })
}

fn publish_inbox(
    tx: &Transaction<'_>,
    limits: HttpDeliveryLimits,
    recipient: &MemberId,
    message: &TransportMessage,
    digest: [u8; 32],
) -> Result<HttpPublishReceipt, StoreTxError> {
    let route = match lookup_route(tx, PLANE_INBOX, recipient.as_slice())? {
        Some(route) => {
            // Mirrors InboxQueue::check_append (D:652-674).
            if let Some(receipt) = replay_or_reject_duplicate(
                tx,
                &route,
                HttpDeliveryPlane::Inbox,
                &message.id,
                digest,
            )? {
                return Ok(receipt);
            }
            ensure_queue_has_space(
                HttpDeliveryPlane::Inbox,
                route.last_seq,
                limits.max_queue_entries_per_route,
            )?;
            route
        }
        None => {
            // New route: enforce the inbox cap before creating (D:553-557).
            let inboxes = count_routes(tx, PLANE_INBOX)?;
            if inboxes >= limits.max_recipient_inboxes as u64 {
                return Err(HttpServerError::InboxLimitExceeded {
                    max: limits.max_recipient_inboxes,
                }
                .into());
            }
            create_route(tx, PLANE_INBOX, recipient.as_slice())?
        }
    };
    let seq = route.last_seq + 1;
    insert_entry(tx, route.route_id, seq, message, digest)?;
    bump_route_head(tx, route.route_id, seq)?;
    Ok(HttpPublishReceipt {
        message_id: message.id.clone(),
        plane: HttpDeliveryPlane::Inbox,
        seq,
        duplicate: false,
    })
}

fn lookup_route(
    conn: &Connection,
    plane: &str,
    route_key: &[u8],
) -> Result<Option<Route>, StoreTxError> {
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT route_id, last_seq FROM delivery_routes
             WHERE plane = ?1 AND route_key = ?2",
            params![plane, route_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(StoreTxError::Sqlite)?;
    match row {
        Some((route_id, last_seq)) => Ok(Some(Route {
            route_id,
            last_seq: db_to_u64(last_seq)?,
        })),
        None => Ok(None),
    }
}

fn count_routes(tx: &Transaction<'_>, plane: &str) -> Result<u64, StoreTxError> {
    let count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM delivery_routes WHERE plane = ?1",
            [plane],
            |row| row.get(0),
        )
        .map_err(StoreTxError::Sqlite)?;
    Ok(db_to_u64(count)?)
}

fn create_route(
    tx: &Transaction<'_>,
    plane: &str,
    route_key: &[u8],
) -> Result<Route, StoreTxError> {
    tx.execute(
        "INSERT INTO delivery_routes (plane, route_key, last_seq) VALUES (?1, ?2, 0)",
        params![plane, route_key],
    )
    .map_err(StoreTxError::Sqlite)?;
    Ok(Route {
        route_id: tx.last_insert_rowid(),
        last_seq: 0,
    })
}

/// Sequences are dense from 1, so the route head doubles as the entry count
/// for the queue-space check (D:823-836).
fn ensure_queue_has_space(
    plane: HttpDeliveryPlane,
    last_seq: HttpSequence,
    max_entries: usize,
) -> Result<(), HttpServerError> {
    if last_seq < max_entries as u64 {
        Ok(())
    } else {
        Err(HttpServerError::QueueFull {
            plane,
            max: max_entries,
        })
    }
}

/// Digest-exact duplicates replay the original receipt; a different digest
/// under the same id is a conflict (D:769-787).
fn replay_or_reject_duplicate(
    tx: &Transaction<'_>,
    route: &Route,
    plane: HttpDeliveryPlane,
    message_id: &MessageId,
    digest: [u8; 32],
) -> Result<Option<HttpPublishReceipt>, StoreTxError> {
    let existing: Option<(i64, Vec<u8>)> = tx
        .query_row(
            "SELECT seq, digest FROM delivery_entries
             WHERE route_id = ?1 AND message_id = ?2",
            params![route.route_id, message_id.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(StoreTxError::Sqlite)?;
    let Some((seq, existing_digest)) = existing else {
        return Ok(None);
    };
    if existing_digest == digest {
        Ok(Some(HttpPublishReceipt {
            message_id: message_id.clone(),
            plane,
            seq: db_to_u64(seq)?,
            duplicate: true,
        }))
    } else {
        Err(HttpServerError::ConflictingMessageId {
            message_id: message_id.clone(),
        }
        .into())
    }
}

fn insert_entry(
    tx: &Transaction<'_>,
    route_id: i64,
    seq: HttpSequence,
    message: &TransportMessage,
    digest: [u8; 32],
) -> Result<(), StoreTxError> {
    let (envelope_kind, envelope_ref): (i64, &[u8]) = match &message.envelope {
        TransportEnvelope::GroupMessage { transport_group_id } => {
            (ENVELOPE_KIND_GROUP_MESSAGE, transport_group_id.as_slice())
        }
        TransportEnvelope::Welcome { recipient } => (ENVELOPE_KIND_WELCOME, recipient.as_slice()),
    };
    // The JSON array matches the serde encoding of Vec<MessageId>.
    let causal_deps_json = serde_json::to_string(&message.causal_deps)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    tx.execute(
        "INSERT INTO delivery_entries
             (route_id, seq, message_id, digest, payload, ts,
              causal_deps_json, source, envelope_kind, envelope_ref)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            route_id,
            u64_to_db(seq)?,
            message.id.as_slice(),
            digest.as_slice(),
            message.payload,
            u64_to_db(message.timestamp.0)?,
            causal_deps_json,
            message.source.0,
            envelope_kind,
            envelope_ref,
        ],
    )
    .map_err(StoreTxError::Sqlite)?;
    Ok(())
}

fn bump_route_head(
    tx: &Transaction<'_>,
    route_id: i64,
    seq: HttpSequence,
) -> Result<(), StoreTxError> {
    tx.execute(
        "UPDATE delivery_routes SET last_seq = ?2 WHERE route_id = ?1",
        params![route_id, u64_to_db(seq)?],
    )
    .map_err(StoreTxError::Sqlite)?;
    Ok(())
}

/// Page semantics mirror the reference `sync_page` (D:797-821): entries
/// strictly after `after_seq` in seq order, the cursor points at the last
/// returned entry (or stays at `after_seq` for an empty page), and
/// `has_more` is set exactly when a full page has a successor.
fn sync_route(
    conn: &Connection,
    plane: &str,
    route_key: &[u8],
    after_seq: HttpSequence,
    limit: usize,
) -> Result<HttpSyncPage, StoreTxError> {
    let Some(route) = lookup_route(conn, plane, route_key)? else {
        // Unknown routes sync as empty pages (D:438-439, D:452-453).
        return Ok(HttpSyncPage {
            entries: Vec::new(),
            next_after_seq: after_seq,
            has_more: false,
        });
    };
    let mut statement = conn
        .prepare(
            "SELECT seq, message_id, payload, ts, causal_deps_json,
                    source, envelope_kind, envelope_ref
             FROM delivery_entries
             WHERE route_id = ?1 AND seq > ?2
             ORDER BY seq
             LIMIT ?3",
        )
        .map_err(StoreTxError::Sqlite)?;
    let rows = statement
        .query_map(
            params![
                route.route_id,
                u64_to_db(after_seq)?,
                u64_to_db(limit as u64)?
            ],
            row_to_queued_delivery,
        )
        .map_err(StoreTxError::Sqlite)?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.map_err(StoreTxError::Sqlite)?);
    }
    let next_after_seq = entries.last().map_or(after_seq, |entry| entry.seq);
    let has_more = entries.len() == limit && next_after_seq < route.last_seq;
    Ok(HttpSyncPage {
        entries,
        next_after_seq,
        has_more,
    })
}

/// Reconstruct a stored [`TransportMessage`]: `envelope_kind` 0 is a
/// GroupMessage whose `envelope_ref` is the transport group id, 1 is a
/// Welcome whose `envelope_ref` is the recipient member id.
fn row_to_queued_delivery(row: &rusqlite::Row<'_>) -> rusqlite::Result<HttpQueuedDelivery> {
    let seq: i64 = row.get(0)?;
    let message_id: Vec<u8> = row.get(1)?;
    let payload: Vec<u8> = row.get(2)?;
    let ts: i64 = row.get(3)?;
    let causal_deps_json: String = row.get(4)?;
    let source: String = row.get(5)?;
    let envelope_kind: i64 = row.get(6)?;
    let envelope_ref: Vec<u8> = row.get(7)?;
    let causal_deps: Vec<MessageId> = serde_json::from_str(&causal_deps_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let envelope = match envelope_kind {
        ENVELOPE_KIND_GROUP_MESSAGE => TransportEnvelope::GroupMessage {
            transport_group_id: envelope_ref,
        },
        ENVELOPE_KIND_WELCOME => TransportEnvelope::Welcome {
            recipient: MemberId::new(envelope_ref),
        },
        other => return Err(rusqlite::Error::IntegralValueOutOfRange(6, other)),
    };
    Ok(HttpQueuedDelivery {
        seq: db_to_u64(seq)?,
        message: TransportMessage {
            id: MessageId::new(message_id),
            payload,
            timestamp: Timestamp(db_to_u64(ts)?),
            causal_deps,
            source: TransportSource(source),
            envelope,
        },
    })
}

// --- Mirrored validation ----------------------------------------------------
//
// finitechat-delivery keeps its validators private ("D:<line>" refers to
// finitechat/crates/finitechat-delivery/src/lib.rs); its only public entry
// point, HttpDeliveryService::check_publish, requires a reference-service
// instance and does not cover sync/claim validation. These mirrors reproduce
// the checks bit-for-bit — field names included, since HttpServerError
// carries them — so SqlDelivery rejects exactly what the reference rejects.

/// Mirrors `validate_transport_message` (D:880-904).
fn validate_transport_message(message: &TransportMessage) -> Result<(), HttpServerError> {
    validate_message_id("message.id", &message.id)?;
    validate_non_empty_len(
        "message.payload",
        message.payload.len(),
        MAX_HTTP_MESSAGE_PAYLOAD_BYTES,
    )?;
    validate_non_empty_len(
        "message.source",
        message.source.0.len(),
        MAX_HTTP_SOURCE_BYTES,
    )?;
    validate_item_count(
        "message.causal_deps",
        message.causal_deps.len(),
        MAX_HTTP_MESSAGE_CAUSAL_DEPS,
    )?;
    for dep in &message.causal_deps {
        validate_message_id("message.causal_deps", dep)?;
    }
    match &message.envelope {
        TransportEnvelope::GroupMessage { transport_group_id } => {
            validate_transport_group_id(transport_group_id)
        }
        TransportEnvelope::Welcome { recipient } => {
            validate_member_id("welcome.recipient", recipient)
        }
    }
}

/// Mirrors `validate_target_matches_message` (D:838-859).
fn validate_target_matches_message(
    target: &HttpPublishTarget,
    message: &TransportMessage,
) -> Result<(), HttpServerError> {
    match (target, &message.envelope) {
        (
            HttpPublishTarget::Group {
                transport_group_id, ..
            },
            TransportEnvelope::GroupMessage {
                transport_group_id: message_group_id,
            },
        ) if transport_group_id == message_group_id => Ok(()),
        (
            HttpPublishTarget::Inbox { recipient },
            TransportEnvelope::Welcome {
                recipient: message_recipient,
            },
        ) if recipient == message_recipient => Ok(()),
        _ => Err(HttpServerError::PublishTargetMismatch),
    }
}

/// Mirrors `validate_key_package_publication` (D:861-871).
fn validate_key_package_publication(
    publication: &HttpKeyPackagePublication,
) -> Result<(), HttpServerError> {
    validate_non_empty_len(
        "key_package_id",
        publication.key_package_id.as_slice().len(),
        MAX_HTTP_ID_BYTES,
    )?;
    validate_member_id("owner", &publication.owner)?;
    validate_non_empty_len(
        "key_package.bytes",
        publication.key_package.bytes.len(),
        MAX_HTTP_KEY_PACKAGE_BYTES,
    )
}

/// Mirrors `validate_group_id` (D:906-908).
fn validate_group_id(group_id: &GroupId) -> Result<(), HttpServerError> {
    validate_non_empty_len("group_id", group_id.as_slice().len(), MAX_HTTP_ID_BYTES)
}

/// Mirrors `validate_member_id` (D:910-912).
fn validate_member_id(field: &'static str, member_id: &MemberId) -> Result<(), HttpServerError> {
    validate_non_empty_len(field, member_id.as_slice().len(), MAX_HTTP_ID_BYTES)
}

/// Mirrors `validate_message_id` (D:914-916).
fn validate_message_id(field: &'static str, message_id: &MessageId) -> Result<(), HttpServerError> {
    validate_non_empty_len(field, message_id.as_slice().len(), MAX_HTTP_ID_BYTES)
}

/// Mirrors `validate_transport_group_id` (D:926-932).
fn validate_transport_group_id(transport_group_id: &[u8]) -> Result<(), HttpServerError> {
    validate_non_empty_len(
        "transport_group_id",
        transport_group_id.len(),
        MAX_HTTP_TRANSPORT_GROUP_ID_BYTES,
    )
}

/// Mirrors `validate_non_empty_len` (D:946-958).
fn validate_non_empty_len(
    field: &'static str,
    actual: usize,
    max: usize,
) -> Result<(), HttpServerError> {
    if actual == 0 {
        return Err(HttpServerError::Empty { field });
    }
    if actual > max {
        return Err(HttpServerError::TooLarge { field, actual, max });
    }
    Ok(())
}

/// Mirrors `validate_item_count` (D:960-970).
fn validate_item_count(
    field: &'static str,
    actual: usize,
    max: usize,
) -> Result<(), HttpServerError> {
    if actual <= max {
        Ok(())
    } else {
        Err(HttpServerError::TooLarge { field, actual, max })
    }
}

/// Mirrors `validate_page_limit` (D:972-981).
fn validate_page_limit(limit: usize) -> Result<(), HttpServerError> {
    if (1..=MAX_HTTP_SYNC_PAGE_ENTRIES).contains(&limit) {
        Ok(())
    } else {
        Err(HttpServerError::InvalidPageLimit {
            actual: limit,
            max: MAX_HTTP_SYNC_PAGE_ENTRIES,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Runs the upstream delivery conformance suite against [`SqlDelivery`].
    //!
    //! These live as unit tests (not an integration test) so the store
    //! modules can stay `pub(crate)` and the crate's external API is
    //! unchanged; dev-dependencies like `tempfile` are available here.

    use std::path::PathBuf;

    use finitechat_delivery::HttpDeliveryLimits;
    use finitechat_delivery::conformance::{self, HttpDeliveryHarness};

    use super::SqlDelivery;
    use crate::store::Store;

    /// File-backed harness: `restart` reopens the store from the same path,
    /// proving durability for the restart-survival checks.
    struct FileHarness {
        _dir: tempfile::TempDir,
        db_path: PathBuf,
        delivery: SqlDelivery,
    }

    impl FileHarness {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp dir for SQL delivery conformance");
            let db_path = dir.path().join("sql-delivery-conformance.sqlite3");
            let delivery = SqlDelivery::new(
                Store::open_file(&db_path).expect("open file-backed SQL delivery store"),
                HttpDeliveryLimits::default(),
            );
            Self {
                _dir: dir,
                db_path,
                delivery,
            }
        }
    }

    impl HttpDeliveryHarness for FileHarness {
        type Delivery = SqlDelivery;

        fn delivery(&mut self) -> &mut SqlDelivery {
            &mut self.delivery
        }

        fn restart(&mut self) -> bool {
            self.delivery = SqlDelivery::new(
                Store::open_file(&self.db_path).expect("reopen SQL delivery store after restart"),
                HttpDeliveryLimits::default(),
            );
            true
        }
    }

    /// In-memory harness: no durable state, so `restart` stays `false` and
    /// the suite skips its post-restart assertions.
    struct MemoryHarness {
        delivery: SqlDelivery,
    }

    impl MemoryHarness {
        fn new() -> Self {
            Self {
                delivery: SqlDelivery::new(
                    Store::open_in_memory().expect("open in-memory SQL delivery store"),
                    HttpDeliveryLimits::default(),
                ),
            }
        }
    }

    impl HttpDeliveryHarness for MemoryHarness {
        type Delivery = SqlDelivery;

        fn delivery(&mut self) -> &mut SqlDelivery {
            &mut self.delivery
        }
    }

    #[test]
    fn sql_delivery_passes_upstream_conformance_suite_on_file_store() {
        conformance::check_all(&mut FileHarness::new());
    }

    #[test]
    fn sql_delivery_passes_upstream_restart_conformance_alone() {
        conformance::check_state_survives_restart(&mut FileHarness::new());
    }

    #[test]
    fn sql_delivery_passes_upstream_conformance_suite_in_memory() {
        conformance::check_all(&mut MemoryHarness::new());
    }
}
