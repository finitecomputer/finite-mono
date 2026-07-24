use crate::*;

impl BrainStore {
    pub fn folder_deletion_replay(
        &self,
        brain_id: &BrainId,
        folder_id: &FolderId,
    ) -> Result<Option<FolderDeletionReplay>, StoreError> {
        self.conn
            .query_row(
                r#"SELECT deletion_event_id, actor_npub, root_key_version,
                          folder_count, object_count
                   FROM deleted_folder_identities
                   WHERE brain_id = ?1 AND folder_id = ?2"#,
                params![brain_id.as_str(), folder_id.as_str()],
                |row| {
                    Ok(FolderDeletionReplay {
                        deletion_event_id: row.get(0)?,
                        actor_npub: UserId::new(row.get::<_, String>(1)?)
                            .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
                        root_key_version: row.get(2)?,
                        folder_count: row.get(3)?,
                        object_count: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn delete_folder_subtree(
        &mut self,
        brain_id: &BrainId,
        root_folder_id: &FolderId,
        actor_npub: &UserId,
        expected_key_version: u32,
        deletion_event_id: &str,
        payload_json: &str,
        deleted_at: &str,
        record_event_kind: u16,
    ) -> Result<FolderSubtreeDeletion, StoreError> {
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "Folder deletion requires brain destructive authority".to_owned(),
            });
        }
        if let Some(existing) = self.folder_deletion_replay(brain_id, root_folder_id)? {
            if existing.deletion_event_id != deletion_event_id
                || existing.actor_npub != *actor_npub
                || existing.root_key_version != expected_key_version
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "Folder identity was already permanently deleted".to_owned(),
                });
            }
            let sequence = self.conn.query_row(
                "SELECT sequence FROM brain_record_index WHERE brain_id = ?1 AND record_event_id = ?2",
                params![brain_id.as_str(), deletion_event_id],
                |row| row.get::<_, u64>(0),
            )?;
            return Ok(FolderSubtreeDeletion {
                sequence,
                duplicate: true,
                folder_count: existing.folder_count,
                object_count: existing.object_count,
            });
        }

        let root_folder = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == *root_folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: root_folder_id.to_string(),
            })?;
        if root_folder.current_key_version != expected_key_version {
            return Err(StoreError::Conflict {
                reason: "Folder Key version changed before deletion".to_owned(),
                current_revision: Some(u64::from(root_folder.current_key_version)),
            });
        }

        let mut subtree = BTreeSet::from([root_folder_id.clone()]);
        loop {
            let before = subtree.len();
            for folder in &stored.brain.folders {
                if folder
                    .parent_folder_id
                    .as_ref()
                    .is_some_and(|parent| subtree.contains(parent))
                {
                    subtree.insert(folder.id.clone());
                }
            }
            if subtree.len() == before {
                break;
            }
        }
        let mut folders = stored
            .brain
            .folders
            .iter()
            .filter(|folder| subtree.contains(&folder.id))
            .cloned()
            .collect::<Vec<_>>();
        folders.sort_by_key(|folder| {
            let mut depth = 0_usize;
            let mut parent = folder.parent_folder_id.as_ref();
            while let Some(parent_id) = parent {
                depth += 1;
                parent = stored
                    .brain
                    .folders
                    .iter()
                    .find(|candidate| candidate.id == *parent_id)
                    .and_then(|candidate| candidate.parent_folder_id.as_ref());
            }
            std::cmp::Reverse(depth)
        });
        let objects = self
            .load_current_objects(brain_id)?
            .into_iter()
            .filter(|object| subtree.contains(&object.folder_id))
            .collect::<Vec<_>>();
        let live_object_count = objects.iter().filter(|object| !object.deleted).count();

        let tx = self.conn.transaction()?;
        let sequence = sync_records::next_sequence(&tx, brain_id)?;

        for object in &objects {
            tx.execute(
                r#"INSERT INTO deleted_object_identities (
                    brain_id, folder_id, object_id, root_folder_id,
                    deletion_event_id, actor_npub, deleted_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(brain_id, folder_id, object_id) DO NOTHING"#,
                params![
                    brain_id.as_str(),
                    object.folder_id.as_str(),
                    object.object_id.as_str(),
                    root_folder_id.as_str(),
                    deletion_event_id,
                    actor_npub.as_str(),
                    deleted_at,
                ],
            )?;
        }
        for folder in &folders {
            tx.execute(
                r#"INSERT INTO deleted_folder_identities (
                    brain_id, folder_id, root_folder_id, deletion_event_id,
                    actor_npub, deleted_at, payload_json, root_key_version,
                    folder_count, object_count
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
                params![
                    brain_id.as_str(),
                    folder.id.as_str(),
                    root_folder_id.as_str(),
                    deletion_event_id,
                    actor_npub.as_str(),
                    deleted_at,
                    payload_json,
                    expected_key_version,
                    folders.len(),
                    live_object_count,
                ],
            )?;
        }

        let mut invitation_ids = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT id, initial_folder_access_json FROM brain_invitations WHERE brain_id = ?1 AND status = 'pending'",
            )?;
            let rows = stmt.query_map(params![brain_id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, scope_json) = row?;
                let scope = serde_json::from_str::<Vec<String>>(&scope_json).map_err(|_| {
                    StoreError::InvalidRecord {
                        reason: "stored Brain Invitation Folder scope is invalid".to_owned(),
                    }
                })?;
                if scope
                    .iter()
                    .any(|folder_id| subtree.iter().any(|id| id.as_str() == folder_id))
                {
                    invitation_ids.push(id);
                }
            }
        }
        for invitation_id in invitation_ids {
            tx.execute(
                "DELETE FROM brain_invitations WHERE id = ?1",
                params![invitation_id],
            )?;
        }
        for folder in &folders {
            tx.execute(
                "DELETE FROM brain_record_index WHERE brain_id = ?1 AND folder_id = ?2",
                params![brain_id.as_str(), folder.id.as_str()],
            )?;
        }
        for folder in &folders {
            tx.execute(
                "DELETE FROM folders WHERE brain_id = ?1 AND id = ?2",
                params![brain_id.as_str(), folder.id.as_str()],
            )?;
        }
        tx.execute(
            r#"INSERT INTO brain_record_index (
                brain_id, sequence, record_event_id, record_type, folder_id, object_id,
                revision, actor_npub, client_created_at, payload_json, accepted_at,
                record_event_kind
            ) VALUES (?1, ?2, ?3, 'brain_admin_access_change', NULL, NULL, NULL, ?4, ?5, ?6, ?5, ?7)"#,
            params![
                brain_id.as_str(),
                sequence,
                deletion_event_id,
                actor_npub.as_str(),
                deleted_at,
                payload_json,
                record_event_kind,
            ],
        )?;
        tx.commit()?;

        Ok(FolderSubtreeDeletion {
            sequence,
            duplicate: false,
            folder_count: folders.len(),
            object_count: live_object_count,
        })
    }
}
