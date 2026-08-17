use crate::*;

impl BrainStore {
    /// Pending Folder Key wraps for one Brain, ordered by Folder then
    /// recipient. Key-holding clients use this to discover who still needs a
    /// wrapped current Folder Key; it is a delivery hint, never a gate.
    pub fn pending_grant_wraps(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<PendingGrantWrap>, StoreError> {
        let mut statement = self.conn.prepare(
            r#"
            SELECT brain_id, folder_id, recipient_npub, key_version, reason, created_at
            FROM brain_pending_grant_wraps
            WHERE brain_id = ?1
            ORDER BY folder_id ASC, recipient_npub ASC, key_version ASC
            "#,
        )?;
        let rows = statement.query_map(params![brain_id.as_str()], |row| {
            let reason = row.get::<_, String>(4)?;
            Ok(PendingGrantWrap {
                brain_id: BrainId::new(row.get::<_, String>(0)?)
                    .map_err(to_from_sql_error(0, rusqlite::types::Type::Text))?,
                folder_id: FolderId::new(row.get::<_, String>(1)?)
                    .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
                recipient_npub: UserId::new(row.get::<_, String>(2)?)
                    .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?,
                key_version: row.get::<_, u32>(3)?,
                reason: PendingGrantWrapReason::try_from(reason.as_str())
                    .map_err(to_store_from_sql_error(4, rusqlite::types::Type::Text))?,
                created_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Complete pending grant wraps for one Folder with grants prepared by a
    /// key-holding client. The server never sees plaintext keys; the recorded
    /// markers are the delivery list, so no signed admin access-change event
    /// is required — only the per-grant Folder Key Grant control records,
    /// exactly like the email claim and departure replay paths.
    ///
    /// Validation fails closed: every marker for the Folder must sit at the
    /// current key version (a rotation underneath the pending wrap is an
    /// error, never a silent clear), and the supplied grants must match the
    /// marked recipients exactly. When no markers remain — another client
    /// completed first, or another grant path already covered the recipients
    /// — the call is a no-op so replays are safe.
    pub fn complete_pending_grant_wraps(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        recipient_npubs: &[UserId],
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
    ) -> Result<usize, StoreError> {
        let stored = self.load_brain(brain_id)?;
        let folder = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == *folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: folder_id.to_string(),
            })?;
        let markers = self
            .pending_grant_wraps(brain_id)?
            .into_iter()
            .filter(|wrap| wrap.folder_id == *folder_id)
            .collect::<Vec<_>>();
        if markers.is_empty() {
            return Ok(0);
        }
        if markers
            .iter()
            .any(|wrap| wrap.key_version != folder.current_key_version)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "pending grant wrap markers drifted from the current Folder key version"
                    .to_owned(),
            });
        }
        let expected = markers
            .iter()
            .map(|wrap| wrap.recipient_npub.clone())
            .collect::<BTreeSet<_>>();
        let supplied = recipient_npubs.iter().cloned().collect::<BTreeSet<_>>();
        if supplied != expected || recipient_npubs.len() != expected.len() {
            return Err(StoreError::BrokenInvariant {
                reason: "pending grant wrap completion must name the marked recipients exactly"
                    .to_owned(),
            });
        }
        if grants.len() != expected.len() {
            return Err(StoreError::BrokenInvariant {
                reason: "pending grant wrap completion requires one grant per marked recipient"
                    .to_owned(),
            });
        }
        let personal_agent = stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub);
        for grant in grants {
            validate_grant_metadata(grant)?;
            validate_grant_issuer(&stored.brain, grant, personal_agent)?;
            if grant.folder_id != *folder_id {
                return Err(StoreError::BrokenInvariant {
                    reason: "grant folder id must match the pending wrap Folder".to_owned(),
                });
            }
            if grant.key_version != folder.current_key_version {
                return Err(StoreError::BrokenInvariant {
                    reason: "grant key version must match folder current key version".to_owned(),
                });
            }
            if !expected.contains(&grant.recipient_npub) {
                return Err(StoreError::BrokenInvariant {
                    reason: "grant recipient must be a marked pending wrap recipient".to_owned(),
                });
            }
        }
        validate_folder_key_grant_control_records(grants, control_records)?;

        let tx = self.conn.transaction()?;
        for grant in grants {
            insert_grant(&tx, brain_id, grant)?;
        }
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.execute(
            "DELETE FROM brain_pending_grant_wraps WHERE brain_id = ?1 AND folder_id = ?2",
            params![brain_id.as_str(), folder_id.as_str()],
        )?;
        tx.commit()?;
        Ok(grants.len())
    }
}

/// Record one pending-wrap marker for a (Brain, Folder, recipient) pair at
/// the Folder's current key version. Idempotent per key version, and skipped
/// when the recipient already holds a grant at this version.
pub(crate) fn mark_pending_grant_wrap(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    folder_id: &FolderId,
    recipient_npub: &UserId,
    key_version: u32,
    reason: PendingGrantWrapReason,
    created_at: &str,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT OR IGNORE INTO brain_pending_grant_wraps (
            brain_id, folder_id, recipient_npub, key_version, reason, created_at
        )
        SELECT ?1, ?2, ?3, ?4, ?5, ?6
        WHERE NOT EXISTS (
            SELECT 1 FROM folder_key_grants
            WHERE brain_id = ?1 AND folder_id = ?2 AND recipient_npub = ?3
              AND key_version = ?4
        )
        "#,
        params![
            brain_id.as_str(),
            folder_id.as_str(),
            recipient_npub.as_str(),
            key_version,
            reason.as_str(),
            created_at
        ],
    )?;
    Ok(())
}

/// A committed grant satisfies every pending wrap for its recipient at or
/// below the grant's key version, whichever path wrote the grant.
pub(crate) fn clear_pending_grant_wraps_for_grant(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    grant: &FolderKeyGrantMetadata,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        DELETE FROM brain_pending_grant_wraps
        WHERE brain_id = ?1 AND folder_id = ?2 AND recipient_npub = ?3
          AND key_version <= ?4
        "#,
        params![
            brain_id.as_str(),
            grant.folder_id.as_str(),
            grant.recipient_npub.as_str(),
            grant.key_version
        ],
    )?;
    Ok(())
}

/// Drop every pending wrap owed to a Principal leaving the Brain.
pub(crate) fn clear_pending_grant_wraps_for_recipient(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    recipient_npub: &UserId,
) -> Result<(), StoreError> {
    tx.execute(
        "DELETE FROM brain_pending_grant_wraps WHERE brain_id = ?1 AND recipient_npub = ?2",
        params![brain_id.as_str(), recipient_npub.as_str()],
    )?;
    Ok(())
}

/// Drop pending wraps owed to a Principal for one Folder, e.g. when their
/// explicit access to that Folder is removed.
pub(crate) fn clear_pending_grant_wraps_for_folder_recipient(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    folder_id: &FolderId,
    recipient_npub: &UserId,
) -> Result<(), StoreError> {
    tx.execute(
        "DELETE FROM brain_pending_grant_wraps WHERE brain_id = ?1 AND folder_id = ?2 AND recipient_npub = ?3",
        params![
            brain_id.as_str(),
            folder_id.as_str(),
            recipient_npub.as_str()
        ],
    )?;
    Ok(())
}

/// Drop pending wraps of one reason owed to a Principal, e.g. commit-time
/// 'invitation' markers when the pending invitation is revoked.
pub(crate) fn clear_pending_grant_wraps_for_reason(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    recipient_npub: &UserId,
    reason: PendingGrantWrapReason,
) -> Result<(), StoreError> {
    tx.execute(
        "DELETE FROM brain_pending_grant_wraps WHERE brain_id = ?1 AND recipient_npub = ?2 AND reason = ?3",
        params![
            brain_id.as_str(),
            recipient_npub.as_str(),
            reason.as_str()
        ],
    )?;
    Ok(())
}
