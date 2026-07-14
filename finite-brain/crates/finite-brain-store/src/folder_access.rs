use crate::*;

impl BrainStore {
    pub fn create_folder(
        &mut self,
        vault_id: &VaultId,
        folder: &Folder,
        access_user_ids: &BTreeSet<UserId>,
        grants: &[FolderKeyGrantMetadata],
    ) -> Result<(), StoreError> {
        if folder.current_key_version != 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "new folders must start at key version 1".to_owned(),
            });
        }

        let mut vault = self.load_core_vault(vault_id)?;
        let adds_personal_members = vault.kind == VaultKind::Personal;
        if adds_personal_members {
            if vault
                .owner_user_id
                .as_ref()
                .is_some_and(|owner| access_user_ids.contains(owner))
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "Personal Vault owner cannot be an ordinary Folder member".to_owned(),
                });
            }
            for user_id in access_user_ids {
                if !vault
                    .members
                    .iter()
                    .any(|member| member.user_id == *user_id)
                {
                    vault.members.push(VaultMember {
                        user_id: user_id.clone(),
                        folder_access: BTreeSet::from([folder.id.clone()]),
                    });
                }
            }
        }
        self.validate_folder_request(&vault, folder, access_user_ids, grants)?;

        let tx = self.conn.transaction()?;
        if adds_personal_members {
            for user_id in access_user_ids {
                insert_member_if_missing(&tx, vault_id, user_id)?;
            }
        }
        insert_folder(&tx, vault_id, folder, false)?;
        insert_folder_access(&tx, vault_id, &folder.id, access_user_ids)?;
        for grant in grants {
            insert_grant(&tx, vault_id, grant)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Insert an empty legacy Folder that can later be repaired by Finish Setup.
    pub fn insert_setup_incomplete_folder_for_repair(
        &mut self,
        vault_id: &VaultId,
        folder: &Folder,
        access_user_ids: &BTreeSet<UserId>,
    ) -> Result<(), StoreError> {
        let vault = self.load_core_vault(vault_id)?;
        validate_hierarchy(&self.conn, vault_id, folder)?;
        validate_access_list_shape(folder, access_user_ids)?;
        validate_access_membership(&vault, access_user_ids)?;

        let tx = self.conn.transaction()?;
        insert_folder(&tx, vault_id, folder, true)?;
        insert_folder_access(&tx, vault_id, &folder.id, access_user_ids)?;
        tx.commit()?;
        Ok(())
    }

    /// Finish setup for an empty Folder by writing the required current grants.
    pub fn finish_folder_setup(
        &mut self,
        vault_id: &VaultId,
        folder_id: &FolderId,
        grants: &[FolderKeyGrantMetadata],
    ) -> Result<(), StoreError> {
        let stored = self.load_vault(vault_id)?;
        let folder = stored
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == *folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: folder_id.to_string(),
            })?;

        if !stored.setup_incomplete_folder_ids.contains(folder_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "folder setup is already complete".to_owned(),
            });
        }
        if self
            .load_current_objects(vault_id)?
            .iter()
            .any(|object| object.folder_id == *folder_id)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "finish setup only supports empty folders".to_owned(),
            });
        }

        let access_user_ids = stored
            .folder_access
            .get(folder_id)
            .cloned()
            .unwrap_or_default();
        let required = required_recipients(&stored.vault, folder, &access_user_ids)?;
        validate_folder_grants(&stored.vault, folder, &required, grants)?;

        let tx = self.conn.transaction()?;
        for grant in grants {
            insert_grant(&tx, vault_id, grant)?;
        }
        tx.execute(
            "UPDATE folders SET setup_incomplete = 0 WHERE vault_id = ?1 AND id = ?2",
            params![vault_id.as_str(), folder_id.as_str()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Grant the current Folder Key to one organization member.
    ///
    /// Restricted Folders also add the member to the Folder access list. All-members Folders
    /// already grant metadata access to every member, so this path only records the missing key.
    pub fn grant_folder_access(
        &mut self,
        vault_id: &VaultId,
        folder_id: &FolderId,
        user_id: &UserId,
        grant: &FolderKeyGrantMetadata,
    ) -> Result<(), StoreError> {
        let mut stored = self.load_vault(vault_id)?;
        let folder = stored
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == *folder_id)
            .cloned()
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: folder_id.to_string(),
            })?;
        let adds_personal_member = stored.vault.kind == VaultKind::Personal
            && !stored
                .vault
                .members
                .iter()
                .any(|member| member.user_id == *user_id);
        if adds_personal_member {
            if folder.access != FolderAccessMode::Restricted {
                return Err(StoreError::BrokenInvariant {
                    reason: "Personal Vault shared access requires a restricted Folder".to_owned(),
                });
            }
            if stored.vault.owner_user_id.as_ref() == Some(user_id) {
                return Err(StoreError::BrokenInvariant {
                    reason: "Personal Vault owner cannot be an ordinary Folder member".to_owned(),
                });
            }
            stored.vault.members.push(VaultMember {
                user_id: user_id.clone(),
                folder_access: BTreeSet::from([folder_id.clone()]),
            });
        }
        validate_access_membership(&stored.vault, &BTreeSet::from([user_id.clone()]))?;
        validate_grant_metadata(grant)?;
        validate_grant_issuer(&stored.vault, grant)?;
        if grant.folder_id != *folder_id {
            return Err(StoreError::BrokenInvariant {
                reason: "grant folder id must match folder metadata".to_owned(),
            });
        }
        if grant.key_version != folder.current_key_version {
            return Err(StoreError::BrokenInvariant {
                reason: "grant key version must match folder current key version".to_owned(),
            });
        }
        if grant.recipient_npub != *user_id {
            return Err(StoreError::BrokenInvariant {
                reason: "grant recipient must match folder access target".to_owned(),
            });
        }

        let inserts_access_row = match folder.access {
            FolderAccessMode::Restricted => {
                let current_access = stored
                    .folder_access
                    .get(folder_id)
                    .cloned()
                    .unwrap_or_default();
                if current_access.contains(user_id) {
                    if stored.grants.iter().any(|existing| {
                        existing.folder_id == *folder_id
                            && existing.key_version == folder.current_key_version
                            && existing.recipient_npub == *user_id
                    }) {
                        return Err(StoreError::BrokenInvariant {
                            reason: "folder key grant is already present".to_owned(),
                        });
                    }
                    false
                } else {
                    true
                }
            }
            FolderAccessMode::AllMembers => {
                if stored.grants.iter().any(|existing| {
                    existing.folder_id == *folder_id
                        && existing.key_version == folder.current_key_version
                        && existing.recipient_npub == *user_id
                }) {
                    return Err(StoreError::BrokenInvariant {
                        reason: "folder key grant is already present".to_owned(),
                    });
                }
                false
            }
            FolderAccessMode::AdminOnly => {
                if !stored.vault.admins.iter().any(|admin| admin == user_id) {
                    return Err(StoreError::BrokenInvariant {
                        reason: "admin-only folder grants require a vault admin target".to_owned(),
                    });
                }
                if stored.grants.iter().any(|existing| {
                    existing.folder_id == *folder_id
                        && existing.key_version == folder.current_key_version
                        && existing.recipient_npub == *user_id
                }) {
                    return Err(StoreError::BrokenInvariant {
                        reason: "folder key grant is already present".to_owned(),
                    });
                }
                false
            }
            FolderAccessMode::Owner => {
                return Err(StoreError::BrokenInvariant {
                    reason: "folder access grants require a restricted or all-members folder"
                        .to_owned(),
                });
            }
        };

        let tx = self.conn.transaction()?;
        if adds_personal_member {
            insert_member_if_missing(&tx, vault_id, user_id)?;
        }
        if inserts_access_row {
            tx.execute(
                "INSERT INTO folder_access (vault_id, folder_id, user_id) VALUES (?1, ?2, ?3)",
                params![vault_id.as_str(), folder_id.as_str(), user_id.as_str()],
            )?;
        }
        insert_grant(&tx, vault_id, grant)?;
        tx.commit()?;
        Ok(())
    }

    /// Remove restricted Folder access by rotating the Folder Key and re-encrypting live objects.
    #[allow(clippy::too_many_arguments)]
    pub fn rotate_folder_key_for_access_removal(
        &mut self,
        vault_id: &VaultId,
        folder_id: &FolderId,
        removed_user_id: &UserId,
        new_key_version: u32,
        grants: &[FolderKeyGrantMetadata],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        updated_at: &str,
    ) -> Result<(), StoreError> {
        let stored = self.load_vault(vault_id)?;
        let folder = stored
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == *folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: folder_id.to_string(),
            })?;
        if folder.access != FolderAccessMode::Restricted {
            return Err(StoreError::BrokenInvariant {
                reason: "folder access removal requires a restricted folder".to_owned(),
            });
        }
        if new_key_version != folder.current_key_version + 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "folder access removal must rotate to the next key version".to_owned(),
            });
        }
        let mut remaining_access = stored
            .folder_access
            .get(folder_id)
            .cloned()
            .unwrap_or_default();
        if !remaining_access.remove(removed_user_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "folder access target does not currently have access".to_owned(),
            });
        }
        let is_active_agent_workspace = self.conn.query_row(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM brain_email_access_delegations
                WHERE vault_id = ?1
                  AND agent_npub = ?2
                  AND workspace_folder_id = ?3
                  AND status = 'active'
            )
            "#,
            params![
                vault_id.as_str(),
                removed_user_id.as_str(),
                folder_id.as_str()
            ],
            |row| row.get::<_, bool>(0),
        )?;
        if is_active_agent_workspace {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "active Agent Workspace access must be removed through delegation revocation"
                        .to_owned(),
            });
        }

        let mut rotated_folder = folder.clone();
        rotated_folder.current_key_version = new_key_version;
        let required = required_recipients(&stored.vault, &rotated_folder, &remaining_access)?;
        validate_folder_grants(&stored.vault, &rotated_folder, &required, grants)?;

        let live_objects = self
            .load_current_objects(vault_id)?
            .into_iter()
            .filter(|object| object.folder_id == *folder_id && !object.deleted)
            .collect::<Vec<_>>();
        validate_rotation_records(&live_objects, reencrypted_records)?;

        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM folder_access WHERE vault_id = ?1 AND folder_id = ?2 AND user_id = ?3",
            params![
                vault_id.as_str(),
                folder_id.as_str(),
                removed_user_id.as_str()
            ],
        )?;
        if stored.vault.kind == VaultKind::Personal {
            let has_remaining_scope = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM folder_access WHERE vault_id = ?1 AND user_id = ?2)",
                params![vault_id.as_str(), removed_user_id.as_str()],
                |row| row.get::<_, bool>(0),
            )?;
            if !has_remaining_scope {
                tx.execute(
                    "DELETE FROM vault_members WHERE vault_id = ?1 AND user_id = ?2",
                    params![vault_id.as_str(), removed_user_id.as_str()],
                )?;
            }
        }
        tx.execute(
            "UPDATE folders SET current_key_version = ?3 WHERE vault_id = ?1 AND id = ?2",
            params![vault_id.as_str(), folder_id.as_str(), new_key_version],
        )?;
        invalidate_pending_email_bootstraps_for_rotated_folder(
            &tx, vault_id, folder_id, updated_at,
        )?;
        for grant in grants {
            insert_grant(&tx, vault_id, grant)?;
        }
        for record in reencrypted_records {
            let input = SyncRecordInput::FolderObjectRevision(record.clone());
            sync_records::validate_sync_input(&input)?;
            sync_records::validate_sync_conflict(&tx, vault_id, &input)?;
            let sequence = sync_records::next_sequence(&tx, vault_id)?;
            sync_records::insert_sync_record(&tx, vault_id, sequence, &input)?;
            sync_records::project_sync_record(&tx, vault_id, &input)?;
        }
        tx.commit()?;
        Ok(())
    }
}
