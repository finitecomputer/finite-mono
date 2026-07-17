use crate::*;

impl BrainStore {
    pub fn load_personal_agent(
        &self,
        vault_id: &VaultId,
    ) -> Result<Option<PersonalAgent>, StoreError> {
        let row = self
            .conn
            .query_row(
                r#"
                SELECT owner_npub, agent_npub, created_by_npub, created_at, updated_at
                FROM personal_agents
                WHERE vault_id = ?1 AND status = 'active'
                "#,
                params![vault_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        row.map(
            |(owner_npub, agent_npub, created_by_npub, created_at, updated_at)| {
                Ok(PersonalAgent {
                    vault_id: vault_id.clone(),
                    owner_npub: UserId::new(owner_npub)?,
                    agent_npub: UserId::new(agent_npub)?,
                    created_by_npub: UserId::new(created_by_npub)?,
                    created_at,
                    updated_at,
                })
            },
        )
        .transpose()
    }

    pub(crate) fn load_core_vault(&self, vault_id: &VaultId) -> Result<Vault, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT id, kind, name, owner_user_id FROM vaults WHERE id = ?1",
                params![vault_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::MissingVault {
                vault_id: vault_id.to_string(),
            })?;

        let kind = parse_vault_kind(&row.1)?;
        let mut vault = Vault {
            id: VaultId::new(row.0)?,
            kind,
            name: DisplayName::new("vault_name", row.2)?,
            owner_user_id: row.3.map(UserId::new).transpose()?,
            folders: self.load_folders(vault_id)?,
            members: self.load_members(vault_id)?,
            admins: self.load_admins(vault_id)?,
        };
        validate_loaded_vault(&vault)?;

        if vault.kind == VaultKind::Organization {
            let folder_access = self.load_folder_access(vault_id)?;
            for member in &mut vault.members {
                member.folder_access = folder_access
                    .iter()
                    .filter_map(|(folder_id, users)| {
                        users.contains(&member.user_id).then_some(folder_id.clone())
                    })
                    .collect();
            }
        }

        Ok(vault)
    }

    pub(crate) fn load_folders(&self, vault_id: &VaultId) -> Result<Vec<Folder>, StoreError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, name, role, access, parent_folder_id, path, current_key_version,
                   shared_folder_source
            FROM folders
            WHERE vault_id = ?1
            ORDER BY id
            "#,
        )?;
        let rows = stmt.query_map(params![vault_id.as_str()], |row| {
            Ok(StoredFolderRow {
                id: row.get(0)?,
                name: row.get(1)?,
                role: row.get(2)?,
                access: row.get(3)?,
                parent_folder_id: row.get(4)?,
                path: row.get(5)?,
                current_key_version: row.get(6)?,
                shared_folder_source: row.get(7)?,
            })
        })?;

        let mut folders = Vec::new();
        for row in rows {
            folders.push(row?.try_into_folder()?);
        }
        Ok(folders)
    }

    pub(crate) fn load_members(&self, vault_id: &VaultId) -> Result<Vec<VaultMember>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT user_id FROM vault_members WHERE vault_id = ?1 ORDER BY user_id")?;
        let rows = stmt.query_map(params![vault_id.as_str()], |row| row.get::<_, String>(0))?;

        let mut members = Vec::new();
        for row in rows {
            members.push(VaultMember {
                user_id: UserId::new(row?)?,
                folder_access: BTreeSet::new(),
            });
        }
        Ok(members)
    }

    pub(crate) fn load_admins(&self, vault_id: &VaultId) -> Result<Vec<UserId>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT user_id FROM vault_admins WHERE vault_id = ?1 ORDER BY user_id")?;
        let rows = stmt.query_map(params![vault_id.as_str()], |row| row.get::<_, String>(0))?;

        let mut admins = Vec::new();
        for row in rows {
            admins.push(UserId::new(row?)?);
        }
        Ok(admins)
    }

    pub(crate) fn load_folder_access(
        &self,
        vault_id: &VaultId,
    ) -> Result<BTreeMap<FolderId, BTreeSet<UserId>>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT folder_id, user_id FROM folder_access WHERE vault_id = ?1 ORDER BY folder_id, user_id",
        )?;
        let rows = stmt.query_map(params![vault_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut access = BTreeMap::new();
        for row in rows {
            let (folder_id, user_id) = row?;
            access
                .entry(FolderId::new(folder_id)?)
                .or_insert_with(BTreeSet::new)
                .insert(UserId::new(user_id)?);
        }
        Ok(access)
    }

    pub(crate) fn load_grants(
        &self,
        vault_id: &VaultId,
    ) -> Result<Vec<FolderKeyGrantMetadata>, StoreError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, folder_id, key_version, issuer_npub, recipient_npub, format,
                   wrapped_event_json, access_change_event_json, created_at
            FROM folder_key_grants
            WHERE vault_id = ?1
            ORDER BY folder_id, key_version, recipient_npub, id
            "#,
        )?;
        let rows = stmt.query_map(params![vault_id.as_str()], |row| {
            Ok(StoredGrantRow {
                id: row.get(0)?,
                folder_id: row.get(1)?,
                key_version: row.get(2)?,
                issuer_npub: row.get(3)?,
                recipient_npub: row.get(4)?,
                format: row.get(5)?,
                wrapped_event_json: row.get(6)?,
                access_change_event_json: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;

        let mut grants = Vec::new();
        for row in rows {
            grants.push(row?.try_into_grant()?);
        }
        Ok(grants)
    }

    pub(crate) fn load_setup_incomplete_folder_ids(
        &self,
        vault_id: &VaultId,
    ) -> Result<BTreeSet<FolderId>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM folders WHERE vault_id = ?1 AND setup_incomplete = 1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![vault_id.as_str()], |row| row.get::<_, String>(0))?;

        let mut ids = BTreeSet::new();
        for row in rows {
            ids.insert(FolderId::new(row?)?);
        }
        Ok(ids)
    }

    pub(crate) fn latest_sequence(&self, vault_id: &VaultId) -> Result<u64, StoreError> {
        let latest = self.conn.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM vault_record_index WHERE vault_id = ?1",
            params![vault_id.as_str()],
            |row| row.get::<_, u64>(0),
        )?;
        Ok(latest)
    }

    pub(crate) fn retention_floor(&self, vault_id: &VaultId) -> Result<u64, StoreError> {
        let floor = self
            .conn
            .query_row(
                "SELECT retention_floor FROM vault_sync_retention WHERE vault_id = ?1",
                params![vault_id.as_str()],
                |row| row.get::<_, u64>(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok(floor)
    }

    pub(crate) fn load_current_objects(
        &self,
        vault_id: &VaultId,
    ) -> Result<Vec<CurrentEncryptedObject>, StoreError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT folder_id, object_id, payload_json, revision, updated_at, deleted
            FROM current_encrypted_vault_objects
            WHERE vault_id = ?1
            ORDER BY folder_id, object_id
            "#,
        )?;
        let rows = stmt.query_map(params![vault_id.as_str()], |row| {
            Ok(CurrentObjectRow {
                folder_id: row.get(0)?,
                object_id: row.get(1)?,
                payload_json: row.get(2)?,
                revision: row.get(3)?,
                updated_at: row.get(4)?,
                deleted: row.get(5)?,
            })
        })?;

        let mut objects = Vec::new();
        for row in rows {
            objects.push(row?.try_into_current_object()?);
        }
        Ok(objects)
    }

    pub(crate) fn load_connection_members(
        &self,
        connection_id: &str,
    ) -> Result<BTreeSet<UserId>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT member_npub FROM shared_folder_connection_members WHERE connection_id = ?1 ORDER BY member_npub",
        )?;
        let rows = stmt.query_map(params![connection_id], |row| row.get::<_, String>(0))?;
        let mut members = BTreeSet::new();
        for row in rows {
            members.insert(UserId::new(row?)?);
        }
        Ok(members)
    }
}
