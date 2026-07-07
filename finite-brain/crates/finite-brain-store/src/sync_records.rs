//! Store-local sync append-log and current-projection helpers.

use finite_brain_core::{FolderId, ObjectId, UserId, VaultId};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::{
    APP_SPECIFIC_KIND, CurrentEncryptedObject, CurrentObjectRow, MAX_PULL_LIMIT,
    NIP59_GIFT_WRAP_KIND, StoreError, StoredSyncRecord, SyncPull, SyncRecordInput, SyncRecordType,
    current_timestamp, to_from_sql_error,
};

pub(super) fn validate_sync_input(input: &SyncRecordInput) -> Result<(), StoreError> {
    if input.record_event_id().trim().is_empty() {
        return Err(StoreError::InvalidRecord {
            reason: "record event id is required".to_owned(),
        });
    }
    if input.payload_json().trim().is_empty() {
        return Err(StoreError::InvalidRecord {
            reason: "payload JSON is required".to_owned(),
        });
    }
    let expected_kind = match input {
        SyncRecordInput::Control(record)
            if record.record_type == SyncRecordType::FolderKeyGrant =>
        {
            NIP59_GIFT_WRAP_KIND
        }
        _ => APP_SPECIFIC_KIND,
    };
    if input.record_event_kind() != expected_kind {
        return Err(StoreError::InvalidRecord {
            reason: format!(
                "expected event kind {expected_kind}, got {}",
                input.record_event_kind()
            ),
        });
    }

    if let SyncRecordInput::Control(record) = input
        && matches!(
            record.record_type,
            SyncRecordType::FolderObjectRevision | SyncRecordType::FolderObjectTombstone
        )
    {
        return Err(StoreError::InvalidRecord {
            reason: "object sync types must use object record inputs".to_owned(),
        });
    }

    Ok(())
}

pub(super) fn existing_sequence(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    event_id: &str,
) -> Result<Option<u64>, StoreError> {
    tx.query_row(
        "SELECT sequence FROM vault_record_index WHERE vault_id = ?1 AND record_event_id = ?2",
        params![vault_id.as_str(), event_id],
        |row| row.get::<_, u64>(0),
    )
    .optional()
    .map_err(StoreError::from)
}

pub(super) fn next_sequence(tx: &Transaction<'_>, vault_id: &VaultId) -> Result<u64, StoreError> {
    let next = tx.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM vault_record_index WHERE vault_id = ?1",
        params![vault_id.as_str()],
        |row| row.get::<_, u64>(0),
    )?;
    Ok(next)
}

pub(super) fn validate_sync_conflict(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    input: &SyncRecordInput,
) -> Result<(), StoreError> {
    if let Some(folder_id) = input.folder_id() {
        ensure_folder_exists_tx(tx, vault_id, folder_id)?;
    }

    match input {
        SyncRecordInput::FolderObjectRevision(record) => {
            let current = current_object_tx(tx, vault_id, &record.folder_id, &record.object_id)?;
            if record.base_revision.is_none() {
                if record.revision != 1 {
                    return Err(StoreError::InvalidRecord {
                        reason: "create revision must be 1".to_owned(),
                    });
                }
                if current.is_some() {
                    return Err(StoreError::Conflict {
                        reason: "object already exists".to_owned(),
                        current_revision: current.map(|object| object.revision),
                    });
                }
                return Ok(());
            }

            let base_revision = record
                .base_revision
                .ok_or_else(|| StoreError::InvalidRecord {
                    reason: "update and move require baseRevision".to_owned(),
                })?;
            let current = current.ok_or_else(|| StoreError::Conflict {
                reason: "object does not exist".to_owned(),
                current_revision: None,
            })?;
            if current.deleted {
                return Err(StoreError::Conflict {
                    reason: "object is deleted".to_owned(),
                    current_revision: Some(current.revision),
                });
            }
            if base_revision != current.revision {
                return Err(StoreError::Conflict {
                    reason: "baseRevision does not match current folder object revision".to_owned(),
                    current_revision: Some(current.revision),
                });
            }
            if record.revision != base_revision + 1 {
                return Err(StoreError::InvalidRecord {
                    reason: "revision must advance baseRevision by one".to_owned(),
                });
            }
        }
        SyncRecordInput::FolderObjectTombstone(record) => {
            let current = current_object_tx(tx, vault_id, &record.folder_id, &record.object_id)?
                .ok_or_else(|| StoreError::Conflict {
                    reason: "object does not exist".to_owned(),
                    current_revision: None,
                })?;
            if current.deleted {
                return Err(StoreError::Conflict {
                    reason: "object is already deleted".to_owned(),
                    current_revision: Some(current.revision),
                });
            }
            if record.base_revision != current.revision {
                return Err(StoreError::Conflict {
                    reason: "baseRevision does not match current folder object revision".to_owned(),
                    current_revision: Some(current.revision),
                });
            }
            if record.revision != record.base_revision + 1 {
                return Err(StoreError::InvalidRecord {
                    reason: "tombstone revision must advance baseRevision by one".to_owned(),
                });
            }
        }
        SyncRecordInput::Control(_) => {}
    }
    Ok(())
}

pub(super) fn insert_sync_record(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    sequence: u64,
    input: &SyncRecordInput,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO vault_record_index (
            vault_id, sequence, record_event_id, record_type, folder_id, object_id, revision,
            actor_npub, client_created_at, payload_json, accepted_at, record_event_kind
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            vault_id.as_str(),
            sequence,
            input.record_event_id(),
            input.record_type().as_str(),
            input.folder_id().map(FolderId::as_str),
            input.object_id().map(ObjectId::as_str),
            input.revision(),
            input.actor_npub().as_str(),
            input.client_created_at(),
            input.payload_json(),
            current_timestamp(),
            input.record_event_kind()
        ],
    )?;
    Ok(())
}

pub(super) fn project_sync_record(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    input: &SyncRecordInput,
) -> Result<(), StoreError> {
    match input {
        SyncRecordInput::FolderObjectRevision(record) => upsert_current_object(
            tx,
            vault_id,
            ProjectionUpdate {
                folder_id: &record.folder_id,
                object_id: &record.object_id,
                payload_json: &record.payload_json,
                revision: record.revision,
                updated_at: &record.client_created_at,
                deleted: false,
            },
        ),
        SyncRecordInput::FolderObjectTombstone(record) => upsert_current_object(
            tx,
            vault_id,
            ProjectionUpdate {
                folder_id: &record.folder_id,
                object_id: &record.object_id,
                payload_json: &record.payload_json,
                revision: record.revision,
                updated_at: &record.client_created_at,
                deleted: true,
            },
        ),
        SyncRecordInput::Control(_) => Ok(()),
    }
}

pub(super) fn project_stored_record(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    record: &StoredSyncRecord,
) -> Result<(), StoreError> {
    match record.record_type {
        SyncRecordType::FolderObjectRevision | SyncRecordType::FolderObjectTombstone => {
            let folder_id =
                record
                    .folder_id
                    .as_ref()
                    .ok_or_else(|| StoreError::BrokenInvariant {
                        reason: "object sync record is missing folder id".to_owned(),
                    })?;
            let object_id =
                record
                    .object_id
                    .as_ref()
                    .ok_or_else(|| StoreError::BrokenInvariant {
                        reason: "object sync record is missing object id".to_owned(),
                    })?;
            let revision = record.revision.ok_or_else(|| StoreError::BrokenInvariant {
                reason: "object sync record is missing revision".to_owned(),
            })?;
            upsert_current_object(
                tx,
                vault_id,
                ProjectionUpdate {
                    folder_id,
                    object_id,
                    payload_json: &record.payload_json,
                    revision,
                    updated_at: &record.client_created_at,
                    deleted: record.record_type == SyncRecordType::FolderObjectTombstone,
                },
            )
        }
        SyncRecordType::FolderKeyGrant | SyncRecordType::VaultAdminAccessChange => Ok(()),
    }
}

pub(super) fn load_sync_records_tx(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
) -> Result<Vec<StoredSyncRecord>, StoreError> {
    let mut stmt = tx.prepare(
        r#"
        SELECT sequence, record_event_id, record_type, folder_id, object_id, revision,
               actor_npub, client_created_at, payload_json, accepted_at, record_event_kind
        FROM vault_record_index
        WHERE vault_id = ?1
        ORDER BY sequence
        "#,
    )?;
    let rows = stmt.query_map(params![vault_id.as_str()], stored_sync_record_from_row)?;

    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

pub(super) fn load_sync_records(
    conn: &Connection,
    vault_id: &VaultId,
) -> Result<Vec<StoredSyncRecord>, StoreError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT sequence, record_event_id, record_type, folder_id, object_id, revision,
               actor_npub, client_created_at, payload_json, accepted_at, record_event_kind
        FROM vault_record_index
        WHERE vault_id = ?1
        ORDER BY sequence
        "#,
    )?;
    let rows = stmt.query_map(params![vault_id.as_str()], stored_sync_record_from_row)?;

    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

pub(super) fn pull_sync_records(
    conn: &Connection,
    vault_id: &VaultId,
    after_sequence: u64,
    requested_limit: u64,
    latest_sequence: u64,
) -> Result<SyncPull, StoreError> {
    let limit = requested_limit.clamp(1, MAX_PULL_LIMIT);
    let fetch_limit = limit + 1;
    let mut stmt = conn.prepare(
        r#"
        SELECT sequence, record_event_id, record_type, folder_id, object_id, revision,
               actor_npub, client_created_at, payload_json, accepted_at, record_event_kind
        FROM vault_record_index
        WHERE vault_id = ?1 AND sequence > ?2
        ORDER BY sequence
        LIMIT ?3
        "#,
    )?;
    let rows = stmt.query_map(
        params![vault_id.as_str(), after_sequence, fetch_limit],
        stored_sync_record_from_row,
    )?;

    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    let has_more = records.len() as u64 > limit;
    if has_more {
        records.truncate(limit as usize);
    }
    let next_sequence = records
        .last()
        .map_or(latest_sequence, |record| record.sequence);

    Ok(SyncPull {
        vault_id: vault_id.clone(),
        after_sequence,
        latest_sequence,
        count: records.len(),
        records,
        has_more,
        next_sequence,
    })
}

fn ensure_folder_exists_tx(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    folder_id: &FolderId,
) -> Result<(), StoreError> {
    let exists = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE vault_id = ?1 AND id = ?2)",
        params![vault_id.as_str(), folder_id.as_str()],
        |row| row.get::<_, bool>(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::MissingFolder {
            folder_id: folder_id.to_string(),
        })
    }
}

fn current_object_tx(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    folder_id: &FolderId,
    object_id: &ObjectId,
) -> Result<Option<CurrentEncryptedObject>, StoreError> {
    tx.query_row(
        r#"
        SELECT folder_id, object_id, payload_json, revision, updated_at, deleted
        FROM current_encrypted_vault_objects
        WHERE vault_id = ?1 AND folder_id = ?2 AND object_id = ?3
        "#,
        params![vault_id.as_str(), folder_id.as_str(), object_id.as_str()],
        |row| {
            Ok(CurrentObjectRow {
                folder_id: row.get(0)?,
                object_id: row.get(1)?,
                payload_json: row.get(2)?,
                revision: row.get(3)?,
                updated_at: row.get(4)?,
                deleted: row.get(5)?,
            })
        },
    )
    .optional()?
    .map(CurrentObjectRow::try_into_current_object)
    .transpose()
}

struct ProjectionUpdate<'a> {
    folder_id: &'a FolderId,
    object_id: &'a ObjectId,
    payload_json: &'a str,
    revision: u64,
    updated_at: &'a str,
    deleted: bool,
}

fn upsert_current_object(
    tx: &Transaction<'_>,
    vault_id: &VaultId,
    update: ProjectionUpdate<'_>,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO current_encrypted_vault_objects (
            vault_id, folder_id, object_id, payload_json, revision, updated_at, deleted
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(vault_id, folder_id, object_id) DO UPDATE SET
            payload_json = excluded.payload_json,
            revision = excluded.revision,
            updated_at = excluded.updated_at,
            deleted = excluded.deleted
        "#,
        params![
            vault_id.as_str(),
            update.folder_id.as_str(),
            update.object_id.as_str(),
            update.payload_json,
            update.revision,
            update.updated_at,
            update.deleted
        ],
    )?;
    Ok(())
}

fn stored_sync_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSyncRecord> {
    let record_type = row.get::<_, String>(2)?;
    let folder_id = row.get::<_, Option<String>>(3)?;
    let object_id = row.get::<_, Option<String>>(4)?;
    Ok(StoredSyncRecord {
        sequence: row.get(0)?,
        record_event_id: row.get(1)?,
        record_type: SyncRecordType::try_from(record_type.as_str()).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        folder_id: folder_id
            .map(FolderId::new)
            .transpose()
            .map_err(to_from_sql_error(3, rusqlite::types::Type::Text))?,
        object_id: object_id
            .map(ObjectId::new)
            .transpose()
            .map_err(to_from_sql_error(4, rusqlite::types::Type::Text))?,
        revision: row.get(5)?,
        actor_npub: UserId::new(row.get::<_, String>(6)?)
            .map_err(to_from_sql_error(6, rusqlite::types::Type::Text))?,
        client_created_at: row.get(7)?,
        payload_json: row.get(8)?,
        accepted_at: row.get(9)?,
        record_event_kind: row.get(10)?,
    })
}
