//! Permanent Organization Brain deletion: one store transaction that rechecks
//! the confirmed preview, removes everything the Brain owns, and leaves a
//! small permanent deletion record outside the Brain's foreign-key graph.

use rusqlite::types::ValueRef;
use sha2::{Digest, Sha256};

use crate::folder_key_rotation::begin_authoritative_write;
use crate::schema::LEGACY_DELEGATION_DELETION_SCOPE;
use crate::*;

/// Prefix of a Brain deletion state digest.
pub const BRAIN_DELETION_DIGEST_PREFIX: &str = "sha256:";
const BRAIN_DELETION_DIGEST_DOMAIN: &str = "finite-brain-deletion-state-v1";
/// Rows any one table may contribute to a deletion preview. Every scoped
/// table is already bounded by the Brain capacity envelope; this refuses a
/// preview over a Brain that somehow exceeds it instead of hashing without end.
const MAX_DIGEST_ROWS_PER_TABLE: usize = 200_000;

/// One set of rows a Brain deletion removes and its preview digest covers.
struct ScopedRows {
    table: &'static str,
    /// SQL predicate selecting this Brain's rows; `?1` is the Brain id.
    predicate: &'static str,
    /// Large ciphertext columns left out of the digest. The row's id,
    /// revision or event id in the same row already pins their contents.
    digest_excludes: &'static [&'static str],
}

/// Everything a Brain owns, plus the cross-Brain Mount artifacts that name
/// it. The preview digest hashes these rows and deletion must leave none of
/// them. A schema test requires every table to be listed here or in
/// [`BRAIN_DELETION_UNSCOPED_TABLES`].
const BRAIN_DELETION_SCOPE: &[ScopedRows] = &[
    scoped("brains", "id = ?1"),
    scoped("brain_members", "brain_id = ?1"),
    scoped("brain_admins", "brain_id = ?1"),
    scoped("folders", "brain_id = ?1"),
    scoped("folder_access", "brain_id = ?1"),
    scoped("folder_access_sources", "brain_id = ?1"),
    ScopedRows {
        table: "folder_key_grants",
        predicate: "brain_id = ?1",
        digest_excludes: &["wrapped_event_json", "access_change_event_json"],
    },
    scoped("brain_pending_grant_wraps", "brain_id = ?1"),
    ScopedRows {
        table: "current_encrypted_brain_objects",
        predicate: "brain_id = ?1",
        digest_excludes: &["payload_json"],
    },
    ScopedRows {
        table: "brain_record_index",
        predicate: "brain_id = ?1",
        digest_excludes: &["payload_json"],
    },
    scoped("brain_sync_retention", "brain_id = ?1"),
    scoped("brain_capacity_counts", "brain_id = ?1"),
    ScopedRows {
        table: "deleted_folder_identities",
        predicate: "brain_id = ?1",
        digest_excludes: &["payload_json"],
    },
    scoped("deleted_object_identities", "brain_id = ?1"),
    scoped("folder_deletion_audience", "brain_id = ?1"),
    scoped("brain_invitations", "brain_id = ?1"),
    scoped("brain_invite_tokens", "brain_id = ?1"),
    scoped("brain_invitation_plans", "brain_id = ?1"),
    scoped("brain_approval_requests", "brain_id = ?1"),
    scoped("brain_approval_nonces", "brain_id = ?1"),
    scoped("brain_principal_revocations", "brain_id = ?1"),
    scoped("brain_departure_pending_rotations", "brain_id = ?1"),
    scoped("share_links", "brain_id = ?1"),
    scoped("personal_agents", "brain_id = ?1"),
    scoped("personal_agent_audit", "brain_id = ?1"),
    legacy_delegation_scope(0),
    legacy_delegation_scope(1),
    legacy_delegation_scope(2),
    scoped(
        "shared_folder_invitations",
        "source_brain_id = ?1 OR destination_brain_id = ?1",
    ),
    scoped(
        "shared_folder_connections",
        "source_brain_id = ?1 OR destination_brain_id = ?1",
    ),
    scoped(
        "shared_folder_connection_members",
        concat!(
            "connection_id IN (SELECT id FROM shared_folder_connections ",
            "WHERE source_brain_id = ?1 OR destination_brain_id = ?1)"
        ),
    ),
    scoped(
        "folder_mounts",
        "source_brain_id = ?1 OR destination_brain_id = ?1",
    ),
    scoped("personal_folder_mounts", "source_brain_id = ?1"),
    scoped(
        "legacy_folder_access_source_repairs",
        concat!(
            "brain_id = ?1 OR connection_id IN (SELECT id FROM shared_folder_connections ",
            "WHERE source_brain_id = ?1 OR destination_brain_id = ?1)"
        ),
    ),
    scoped(
        "legacy_personal_mount_migrations",
        "destination_brain_id = ?1",
    ),
];

/// Tables that hold no Brain-owned rows: global state, and the permanent
/// deletion record itself.
#[cfg(test)]
pub(crate) const BRAIN_DELETION_UNSCOPED_TABLES: &[&str] = &[
    "schema_migrations",
    "identity_aliases",
    "brain_departure_fact_cursor",
    "deleted_brains",
    "deleted_brain_audience",
];

const fn legacy_delegation_scope(index: usize) -> ScopedRows {
    let (table, predicate) = LEGACY_DELEGATION_DELETION_SCOPE[index];
    scoped(table, predicate)
}

const fn scoped(table: &'static str, predicate: &'static str) -> ScopedRows {
    ScopedRows {
        table,
        predicate,
        digest_excludes: &[],
    }
}

/// Server-computed scope of a permanent Organization Brain deletion. The
/// admin confirms `state_digest`; the deletion transaction recomputes it and
/// refuses any change, including changes that leave every count the same.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BrainDeletionPreview {
    pub brain_id: BrainId,
    pub name: String,
    pub folder_count: usize,
    /// Live Folder objects (pages, plus legacy assets).
    pub object_count: usize,
    pub member_count: usize,
    pub admin_count: usize,
    pub guest_count: usize,
    /// Pending Brain and Folder invitations.
    pub pending_invitation_count: usize,
    /// Invite Tokens that are neither redeemed nor revoked.
    pub open_invite_token_count: usize,
    /// Active cross-Brain Mounts. Deletion is refused while any remain.
    pub blocking_mounts: Vec<BlockingMount>,
    /// Pending Mount Offers and revoked Mount records naming this Brain in
    /// either direction. Deletion removes them so none can restore access.
    pub retired_mount_record_count: usize,
    pub state_digest: String,
}

/// An active Mount that must be revoked before the Brain can be deleted.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BlockingMount {
    pub connection_id: String,
    pub direction: SharedFolderDirection,
    pub source_brain_id: BrainId,
    pub source_folder_id: FolderId,
    pub destination_brain_id: BrainId,
}

/// One confirmed, signed request to delete an Organization Brain.
#[derive(Debug, Clone)]
pub struct BrainDeletionRequest<'a> {
    pub brain_id: &'a BrainId,
    pub actor_npub: &'a UserId,
    pub deletion_event_id: &'a str,
    pub deletion_event_json: &'a str,
    /// The digest the admin confirmed, as signed in the deletion event.
    pub expected_state_digest: &'a str,
    /// Identity of this Brain server, bound into the digest.
    pub server_identity: &'a str,
    pub deleted_at: &'a str,
    /// False when the signed confirmation is older than the server accepts.
    /// Only a new deletion is refused for it; an exact retry still answers.
    pub confirmation_current: bool,
}

/// Permanent receipt for one accepted Brain deletion.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BrainDeletionReceipt {
    pub brain_id: BrainId,
    pub deletion_event_id: String,
    pub actor_npub: UserId,
    pub deleted_at: String,
    pub state_digest: String,
    pub folder_count: usize,
    pub object_count: usize,
}

/// Result of a Brain deletion request.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BrainDeletionOutcome {
    /// The Brain was deleted now. `audience` held access at deletion.
    Deleted {
        receipt: BrainDeletionReceipt,
        audience: Vec<UserId>,
    },
    /// An exact retry of the accepted deletion; nothing changed.
    Replayed { receipt: BrainDeletionReceipt },
}

/// What a principal that held access learns about a deleted Brain.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DeletedBrainNotice {
    pub brain_id: BrainId,
    pub deleted_at: String,
}

impl BrainStore {
    /// Scope and state digest of deleting `brain_id`, read from one
    /// snapshot without loading object or record ciphertext.
    pub fn brain_deletion_preview(
        &self,
        brain_id: &BrainId,
        actor_npub: &UserId,
        server_identity: &str,
    ) -> Result<BrainDeletionPreview, StoreError> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Deferred)?;
        let stored = self.load_brain(brain_id)?;
        refuse_unless_deletable_by(&stored, actor_npub)?;
        let folder_count = stored.brain.folders.len();
        let object_count = count(
            &tx,
            "SELECT COUNT(*) FROM current_encrypted_brain_objects WHERE brain_id = ?1 AND deleted = 0",
            brain_id,
        )?;
        let pending_invitation_count = count(
            &tx,
            "SELECT COUNT(*) FROM brain_invitations WHERE brain_id = ?1 AND status = 'pending'",
            brain_id,
        )? + count(
            &tx,
            "SELECT COUNT(*) FROM share_links WHERE brain_id = ?1 AND status = 'pending'",
            brain_id,
        )?;
        let open_invite_token_count = count(
            &tx,
            "SELECT COUNT(*) FROM brain_invite_tokens WHERE brain_id = ?1 AND redeemed_at IS NULL AND revoked_at IS NULL",
            brain_id,
        )?;
        let retired_mount_record_count = count(
            &tx,
            "SELECT COUNT(*) FROM shared_folder_connections WHERE (source_brain_id = ?1 OR destination_brain_id = ?1) AND status <> 'active'",
            brain_id,
        )? + count(
            &tx,
            "SELECT COUNT(*) FROM shared_folder_invitations WHERE (source_brain_id = ?1 OR destination_brain_id = ?1) AND status = 'pending'",
            brain_id,
        )?;
        let preview = BrainDeletionPreview {
            brain_id: brain_id.clone(),
            name: stored.brain.name.as_str().to_owned(),
            folder_count,
            object_count,
            member_count: stored.brain.members.len(),
            admin_count: stored.brain.admins.len(),
            guest_count: stored.guest_user_ids().len(),
            pending_invitation_count,
            open_invite_token_count,
            blocking_mounts: active_mounts(&tx, brain_id)?,
            retired_mount_record_count,
            state_digest: brain_state_digest(&tx, brain_id, server_identity)?,
        };
        tx.commit()?;
        Ok(preview)
    }

    /// Delete an Organization Brain, or answer an exact retry of an accepted
    /// deletion with its receipt. Authority, Brain kind, active Mounts,
    /// confirmation age and the confirmed state digest are all rechecked
    /// inside the one write transaction that deletes; any failure changes
    /// nothing.
    pub fn delete_organization_brain(
        &mut self,
        request: &BrainDeletionRequest<'_>,
    ) -> Result<BrainDeletionOutcome, StoreError> {
        let brain_id = request.brain_id;
        let tx = begin_authoritative_write(&self.conn)?;
        if let Some(receipt) = load_deletion_receipt(&tx, brain_id)? {
            return if receipt.deletion_event_id == request.deletion_event_id
                && receipt.actor_npub == *request.actor_npub
            {
                Ok(BrainDeletionOutcome::Replayed { receipt })
            } else {
                Err(StoreError::MissingBrain {
                    brain_id: brain_id.to_string(),
                })
            };
        }
        let stored = self.load_brain(brain_id)?;
        refuse_unless_deletable_by(&stored, request.actor_npub)?;
        let blocking = active_mounts(&tx, brain_id)?;
        if !blocking.is_empty() {
            return Err(StoreError::Conflict {
                reason: active_mounts_reason(&blocking),
                current_revision: None,
            });
        }
        if brain_state_digest(&tx, brain_id, request.server_identity)?
            != request.expected_state_digest
        {
            return Err(StoreError::Conflict {
                reason: "Brain changed after its deletion preview; nothing was deleted. Review a fresh preview and confirm again".to_owned(),
                current_revision: None,
            });
        }
        if !request.confirmation_current {
            return Err(StoreError::Conflict {
                reason: "Brain deletion confirmation expired; nothing was deleted. Review a fresh preview and confirm again".to_owned(),
                current_revision: None,
            });
        }

        let folder_count = stored.brain.folders.len();
        let object_count = count(
            &tx,
            "SELECT COUNT(*) FROM current_encrypted_brain_objects WHERE brain_id = ?1 AND deleted = 0",
            brain_id,
        )?;
        let audience = deletion_audience(&stored);
        tx.execute(
            r#"INSERT INTO deleted_brains (
                brain_id, deletion_event_id, actor_npub, deleted_at, state_digest,
                folder_count, object_count, deletion_event_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![
                brain_id.as_str(),
                request.deletion_event_id,
                request.actor_npub.as_str(),
                request.deleted_at,
                request.expected_state_digest,
                folder_count,
                object_count,
                request.deletion_event_json,
            ],
        )?;
        for reader in &audience {
            tx.execute(
                "INSERT INTO deleted_brain_audience (brain_id, actor_npub) VALUES (?1, ?2)",
                params![brain_id.as_str(), reader.as_str()],
            )?;
        }
        remove_brain_rows(&tx, brain_id)?;
        tx.commit()?;
        let receipt = load_deletion_receipt(&self.conn, brain_id)?.ok_or_else(|| {
            StoreError::BrokenInvariant {
                reason: "Brain deletion record was not written".to_owned(),
            }
        })?;
        Ok(BrainDeletionOutcome::Deleted {
            receipt,
            audience: audience.into_iter().collect(),
        })
    }

    /// The receipt of an accepted deletion, when `deletion_event_id` by
    /// `actor_npub` is exactly the event that deleted `brain_id`.
    pub fn brain_deletion_replay(
        &self,
        brain_id: &BrainId,
        deletion_event_id: &str,
        actor_npub: &UserId,
    ) -> Result<Option<BrainDeletionReceipt>, StoreError> {
        Ok(
            load_deletion_receipt(&self.conn, brain_id)?.filter(|receipt| {
                receipt.deletion_event_id == deletion_event_id && receipt.actor_npub == *actor_npub
            }),
        )
    }

    /// The deletion notice for `actor_npub`, only when it held access to
    /// `brain_id` when the Brain was deleted.
    pub fn deleted_brain_notice(
        &self,
        brain_id: &BrainId,
        actor_npub: &UserId,
    ) -> Result<Option<DeletedBrainNotice>, StoreError> {
        let deleted_at = self
            .conn
            .query_row(
                r#"SELECT d.deleted_at
                   FROM deleted_brains d
                   JOIN deleted_brain_audience a ON a.brain_id = d.brain_id
                   WHERE d.brain_id = ?1 AND a.actor_npub = ?2"#,
                params![brain_id.as_str(), actor_npub.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(deleted_at.map(|deleted_at| DeletedBrainNotice {
            brain_id: brain_id.clone(),
            deleted_at,
        }))
    }
}

/// Only a current admin of an Organization Brain may delete it. A Personal
/// Brain is refused for everyone, its owner and Personal Agent included.
fn refuse_unless_deletable_by(stored: &StoredBrain, actor_npub: &UserId) -> Result<(), StoreError> {
    if stored.brain.kind != BrainKind::Organization {
        return Err(StoreError::BrokenInvariant {
            reason: "Personal Brains cannot be deleted; only Organization Brains can".to_owned(),
        });
    }
    if !stored.brain.admins.contains(actor_npub) {
        return Err(StoreError::Conflict {
            reason: "Brain deletion requires current Brain admin standing; nothing was deleted"
                .to_owned(),
            current_revision: None,
        });
    }
    Ok(())
}

/// Every principal with access when the Brain is deleted: Members (admins
/// are Members) and Folder-limited guests.
fn deletion_audience(stored: &StoredBrain) -> BTreeSet<UserId> {
    let mut audience = stored
        .brain
        .members
        .iter()
        .map(|member| member.user_id.clone())
        .collect::<BTreeSet<_>>();
    audience.extend(stored.brain.admins.iter().cloned());
    audience.extend(stored.folder_access.values().flatten().cloned());
    audience
}

fn active_mounts(conn: &Connection, brain_id: &BrainId) -> Result<Vec<BlockingMount>, StoreError> {
    let mut stmt = conn.prepare(
        r#"SELECT id, source_brain_id, source_folder_id, destination_brain_id
           FROM shared_folder_connections
           WHERE (source_brain_id = ?1 OR destination_brain_id = ?1) AND status = 'active'
           ORDER BY id"#,
    )?;
    let rows = stmt.query_map(params![brain_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut mounts = Vec::new();
    for row in rows {
        let (connection_id, source, folder, destination) = row?;
        let source_brain_id = BrainId::new(source)?;
        mounts.push(BlockingMount {
            connection_id,
            direction: if source_brain_id == *brain_id {
                SharedFolderDirection::Source
            } else {
                SharedFolderDirection::Destination
            },
            source_brain_id,
            source_folder_id: FolderId::new(folder)?,
            destination_brain_id: BrainId::new(destination)?,
        });
    }
    Ok(mounts)
}

fn active_mounts_reason(mounts: &[BlockingMount]) -> String {
    let ids = mounts
        .iter()
        .map(|mount| mount.connection_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Brain has {} active cross-Brain Mount(s) ({ids}); revoke them before deleting the Brain. Nothing was deleted",
        mounts.len()
    )
}

fn count(conn: &Connection, sql: &str, brain_id: &BrainId) -> Result<usize, StoreError> {
    Ok(conn.query_row(sql, params![brain_id.as_str()], |row| {
        row.get::<_, usize>(0)
    })?)
}

fn load_deletion_receipt(
    conn: &Connection,
    brain_id: &BrainId,
) -> Result<Option<BrainDeletionReceipt>, StoreError> {
    let row = conn
        .query_row(
            r#"SELECT deletion_event_id, actor_npub, deleted_at, state_digest,
                      folder_count, object_count
               FROM deleted_brains WHERE brain_id = ?1"#,
            params![brain_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, usize>(4)?,
                    row.get::<_, usize>(5)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(deletion_event_id, actor, deleted_at, state_digest, folder_count, object_count)| {
            Ok(BrainDeletionReceipt {
                brain_id: brain_id.clone(),
                deletion_event_id,
                actor_npub: UserId::new(actor)?,
                deleted_at,
                state_digest,
                folder_count,
                object_count,
            })
        },
    )
    .transpose()
}

/// SHA-256 over every scoped row of the Brain, bound to this server and
/// Brain id. Rows are hashed column by column with type tags and lengths, in
/// a fixed table order and sorted by their own values, so the digest changes
/// whenever any scoped row is added, removed or altered.
fn brain_state_digest(
    conn: &Connection,
    brain_id: &BrainId,
    server_identity: &str,
) -> Result<String, StoreError> {
    let mut hasher = Sha256::new();
    for part in [
        BRAIN_DELETION_DIGEST_DOMAIN,
        server_identity,
        brain_id.as_str(),
    ] {
        hash_bytes(&mut hasher, part.as_bytes());
    }
    for scope in BRAIN_DELETION_SCOPE {
        let columns = digest_columns(conn, scope)?;
        hash_bytes(&mut hasher, scope.table.as_bytes());
        let order = (1..=columns.len())
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {} FROM \"{}\" WHERE {} ORDER BY {order} LIMIT {}",
            columns
                .iter()
                .map(|column| format!("\"{column}\""))
                .collect::<Vec<_>>()
                .join(", "),
            scope.table,
            scope.predicate,
            MAX_DIGEST_ROWS_PER_TABLE + 1,
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(params![brain_id.as_str()])?;
        let mut row_count = 0_usize;
        while let Some(row) = rows.next()? {
            row_count += 1;
            if row_count > MAX_DIGEST_ROWS_PER_TABLE {
                return Err(StoreError::CapacityExceeded {
                    limit: format!("brain_deletion_preview:{}", scope.table),
                    max: MAX_DIGEST_ROWS_PER_TABLE,
                    current: row_count,
                });
            }
            hasher.update([0x1e]);
            for index in 0..columns.len() {
                hash_value(&mut hasher, row.get_ref(index)?);
            }
        }
        hasher.update([0x1d]);
        hasher.update((row_count as u64).to_be_bytes());
    }
    let digest = hasher.finalize();
    Ok(format!(
        "{BRAIN_DELETION_DIGEST_PREFIX}{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

fn digest_columns(conn: &Connection, scope: &ScopedRows) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")?;
    let rows = stmt.query_map(params![scope.table], |row| row.get::<_, String>(0))?;
    let mut columns = Vec::new();
    for row in rows {
        let column = row?;
        if !scope.digest_excludes.contains(&column.as_str()) {
            columns.push(column);
        }
    }
    if columns.is_empty() {
        return Err(StoreError::BrokenInvariant {
            reason: format!("Brain deletion scope table {} has no columns", scope.table),
        });
    }
    Ok(columns)
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn hash_value(hasher: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hasher.update(b"n"),
        ValueRef::Integer(value) => {
            hasher.update(b"i");
            hasher.update(value.to_be_bytes());
        }
        ValueRef::Real(value) => {
            hasher.update(b"r");
            hasher.update(value.to_bits().to_be_bytes());
        }
        ValueRef::Text(bytes) => {
            hasher.update(b"t");
            hash_bytes(hasher, bytes);
        }
        ValueRef::Blob(bytes) => {
            hasher.update(b"b");
            hash_bytes(hasher, bytes);
        }
    }
}

/// Delete every scoped row in an order the RESTRICT foreign keys accept,
/// then prove none remain. Runs inside the caller's write transaction.
fn remove_brain_rows(tx: &Connection, brain_id: &BrainId) -> Result<(), StoreError> {
    let id = params![brain_id.as_str()];
    // Capacity counters first: every later counter trigger for this Brain
    // becomes a no-op instead of failing on counter drift.
    tx.execute("DELETE FROM brain_capacity_counts WHERE brain_id = ?1", id)?;
    tx.execute(
        "DELETE FROM folder_mounts WHERE source_brain_id = ?1 OR destination_brain_id = ?1",
        id,
    )?;
    tx.execute(
        "DELETE FROM shared_folder_connections WHERE source_brain_id = ?1 OR destination_brain_id = ?1",
        id,
    )?;
    tx.execute(
        "DELETE FROM shared_folder_invitations WHERE source_brain_id = ?1 OR destination_brain_id = ?1",
        id,
    )?;
    tx.execute(
        "DELETE FROM personal_folder_mounts WHERE source_brain_id = ?1",
        id,
    )?;
    // The audit rows cascade with their delegations.
    for (table, predicate) in [
        LEGACY_DELEGATION_DELETION_SCOPE[0],
        LEGACY_DELEGATION_DELETION_SCOPE[2],
    ] {
        tx.execute(&format!("DELETE FROM {table} WHERE {predicate}"), id)?;
    }
    tx.execute("DELETE FROM brain_record_index WHERE brain_id = ?1", id)?;
    // Folders reference their parent with ON DELETE RESTRICT: remove leaves
    // first, one bounded pass per hierarchy level.
    for _ in 0..=BRAIN_CAPACITY_ENVELOPE.folder_depth {
        let removed = tx.execute(
            r#"DELETE FROM folders
               WHERE brain_id = ?1
                 AND id NOT IN (
                     SELECT parent_folder_id FROM folders
                     WHERE brain_id = ?1 AND parent_folder_id IS NOT NULL
                 )"#,
            id,
        )?;
        if removed == 0 {
            break;
        }
    }
    tx.execute("DELETE FROM brains WHERE id = ?1", id)?;
    for scope in BRAIN_DELETION_SCOPE {
        let remaining = count(
            tx,
            &format!(
                "SELECT COUNT(*) FROM \"{}\" WHERE {}",
                scope.table, scope.predicate
            ),
            brain_id,
        )?;
        if remaining > 0 {
            return Err(StoreError::BrokenInvariant {
                reason: format!(
                    "Brain deletion left {remaining} row(s) in {}; nothing was deleted",
                    scope.table
                ),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn brain_deletion_scope_tables() -> Vec<&'static str> {
    BRAIN_DELETION_SCOPE
        .iter()
        .map(|scope| scope.table)
        .collect()
}

/// Every scoped row of `brain_id`, every column included, as sorted text.
/// Tests compare these dumps to prove other Brains are left byte-identical.
#[cfg(test)]
pub(crate) fn brain_scoped_rows(conn: &Connection, brain_id: &BrainId) -> Vec<String> {
    let mut dump = Vec::new();
    for scope in BRAIN_DELETION_SCOPE {
        let sql = format!(
            "SELECT * FROM \"{}\" WHERE {}",
            scope.table, scope.predicate
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let columns = stmt.column_count();
        let mut rows = stmt.query(params![brain_id.as_str()]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let values = (0..columns)
                .map(|index| format!("{:?}", row.get_ref(index).unwrap()))
                .collect::<Vec<_>>()
                .join("|");
            dump.push(format!("{}|{values}", scope.table));
        }
    }
    dump.sort();
    dump
}
