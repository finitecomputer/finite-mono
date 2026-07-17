use crate::*;

impl BrainStore {
    pub fn mark_shared_folder_source(
        &mut self,
        vault_id: &VaultId,
        folder_id: &FolderId,
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
                reason: "shared folder sources must be restricted folders".to_owned(),
            });
        }
        if stored.setup_incomplete_folder_ids.contains(folder_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder source setup must be complete".to_owned(),
            });
        }
        self.conn.execute(
            "UPDATE folders SET shared_folder_source = 1 WHERE vault_id = ?1 AND id = ?2",
            params![vault_id.as_str(), folder_id.as_str()],
        )?;
        Ok(())
    }

    /// Create a Shared Folder Invitation from a source Folder to a destination Organization admin.
    #[allow(clippy::too_many_arguments)]
    pub fn create_shared_folder_invitation(
        &mut self,
        source_vault_id: &VaultId,
        source_folder_id: &FolderId,
        destination_vault_id: &VaultId,
        id: &str,
        destination_admin_npub: &UserId,
        created_by_npub: &UserId,
        accept_path: &str,
        grant: &FolderKeyGrantMetadata,
        created_at: &str,
    ) -> Result<StoredSharedFolderInvitation, StoreError> {
        let source = self.load_vault(source_vault_id)?;
        let source_folder = source
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == *source_folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: source_folder_id.to_string(),
            })?;
        if !has_vault_operational_authority(&source, created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder invitations require source vault operational authority"
                    .to_owned(),
            });
        }
        if !source_folder.shared_folder_source
            || source_folder.access != FolderAccessMode::Restricted
        {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder invitations require a restricted shared folder source"
                    .to_owned(),
            });
        }
        if source
            .setup_incomplete_folder_ids
            .contains(source_folder_id)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder source setup must be complete".to_owned(),
            });
        }
        let destination = self.load_vault(destination_vault_id)?;
        if destination.vault.kind != VaultKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder destination must be an organization vault".to_owned(),
            });
        }
        if !destination.vault.admins.contains(destination_admin_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder invitation target must be a destination vault admin"
                    .to_owned(),
            });
        }
        validate_link_id("shared_folder_invitation_id", id)?;
        validate_grant_metadata(grant)?;
        validate_grant_issuer(
            &source.vault,
            grant,
            source
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
        )?;
        if grant.folder_id != *source_folder_id
            || grant.key_version != source_folder.current_key_version
            || grant.recipient_npub != *destination_admin_npub
            || grant.issuer_npub != *created_by_npub
        {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder invitation grant must match source folder, key version, issuer, and destination admin"
                    .to_owned(),
            });
        }
        let access_change_event_json =
            grant
                .access_change_event_json
                .clone()
                .ok_or_else(|| StoreError::BrokenInvariant {
                    reason: "shared folder invitation requires an access-change event".to_owned(),
                })?;

        self.conn
            .execute(
                r#"
                INSERT INTO shared_folder_invitations (
                    id, source_vault_id, source_folder_id, destination_vault_id,
                    destination_admin_npub, created_by_npub, status, current_key_version,
                    accept_path, created_at, updated_at, grant_id, grant_wrapped_event_json,
                    access_change_event_json
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9, ?9, ?10, ?11, ?12)
                "#,
                params![
                    id,
                    source_vault_id.as_str(),
                    source_folder_id.as_str(),
                    destination_vault_id.as_str(),
                    destination_admin_npub.as_str(),
                    created_by_npub.as_str(),
                    source_folder.current_key_version,
                    accept_path,
                    created_at,
                    grant.id,
                    grant.wrapped_event_json,
                    access_change_event_json
                ],
            )
            .map_err(map_insert_error("shared_folder_invitation_id", id))?;

        self.load_shared_folder_invitation(id)
    }

    /// Load a Shared Folder Invitation.
    pub fn load_shared_folder_invitation(
        &self,
        invitation_id: &str,
    ) -> Result<StoredSharedFolderInvitation, StoreError> {
        self.conn
            .query_row(
                r#"
                SELECT id, source_vault_id, source_folder_id, destination_vault_id,
                       destination_admin_npub, created_by_npub, status, current_key_version,
                       accept_path, created_at, updated_at, accepted_at, grant_id,
                       grant_wrapped_event_json, access_change_event_json
                FROM shared_folder_invitations
                WHERE id = ?1
                "#,
                params![invitation_id],
                shared_folder_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "shared folder invitation",
            })
    }

    /// List Shared Folder Invitations for one Vault as source or destination,
    /// newest first, bounded by MAX_LINK_LIST_ROWS.
    pub fn list_shared_folder_invitations(
        &self,
        vault_id: &VaultId,
        direction: SharedFolderDirection,
    ) -> Result<Vec<StoredSharedFolderInvitation>, StoreError> {
        self.require_vault_exists(vault_id)?;
        let column = match direction {
            SharedFolderDirection::Source => "source_vault_id",
            SharedFolderDirection::Destination => "destination_vault_id",
        };
        let mut stmt = self.conn.prepare(&format!(
            r#"
            SELECT id, source_vault_id, source_folder_id, destination_vault_id,
                   destination_admin_npub, created_by_npub, status, current_key_version,
                   accept_path, created_at, updated_at, accepted_at, grant_id,
                   grant_wrapped_event_json, access_change_event_json
            FROM shared_folder_invitations
            WHERE {column} = ?1
            ORDER BY created_at DESC, id
            LIMIT ?2
            "#
        ))?;
        let rows = stmt.query_map(
            params![vault_id.as_str(), MAX_LINK_LIST_ROWS],
            shared_folder_invitation_from_row,
        )?;
        let mut invitations = Vec::new();
        for row in rows {
            invitations.push(row?);
        }
        Ok(invitations)
    }

    /// List Shared Folder Connections for one Vault as source or destination,
    /// newest first, bounded by MAX_LINK_LIST_ROWS. Members are included per connection.
    pub fn list_shared_folder_connections(
        &self,
        vault_id: &VaultId,
        direction: SharedFolderDirection,
    ) -> Result<Vec<StoredSharedFolderConnection>, StoreError> {
        self.require_vault_exists(vault_id)?;
        let column = match direction {
            SharedFolderDirection::Source => "source_vault_id",
            SharedFolderDirection::Destination => "destination_vault_id",
        };
        let connection_ids = {
            let mut stmt = self.conn.prepare(&format!(
                r#"
                SELECT id
                FROM shared_folder_connections
                WHERE {column} = ?1
                ORDER BY created_at DESC, id
                LIMIT ?2
                "#
            ))?;
            let rows = stmt.query_map(params![vault_id.as_str(), MAX_LINK_LIST_ROWS], |row| {
                row.get::<_, String>(0)
            })?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };
        let mut connections = Vec::new();
        for connection_id in connection_ids {
            connections.push(self.load_shared_folder_connection(&connection_id)?);
        }
        Ok(connections)
    }

    /// Revoke a pending or accepted Shared Folder Invitation delivery handle.
    pub fn revoke_shared_folder_invitation(
        &mut self,
        invitation_id: &str,
        actor_npub: &UserId,
        updated_at: &str,
    ) -> Result<StoredSharedFolderInvitation, StoreError> {
        let invitation = self.load_shared_folder_invitation(invitation_id)?;
        let source = self.load_vault(&invitation.source_vault_id)?;
        if !has_vault_operational_authority(&source, actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder invitation revocation requires source vault operational authority"
                    .to_owned(),
            });
        }
        self.conn.execute(
            "UPDATE shared_folder_invitations SET status = 'revoked', updated_at = ?2 WHERE id = ?1",
            params![invitation_id, updated_at],
        )?;
        self.load_shared_folder_invitation(invitation_id)
    }

    /// Accept a Shared Folder Invitation, creating/reusing connection and Organization Mount.
    pub fn accept_shared_folder_invitation(
        &mut self,
        invitation_id: &str,
        destination_admin_npub: &UserId,
        connection_id: &str,
        mount_id: &str,
        now: &str,
    ) -> Result<StoredSharedFolderInvitation, StoreError> {
        let mut invitation = self.load_shared_folder_invitation(invitation_id)?;
        if invitation.destination_admin_npub != *destination_admin_npub {
            return Err(StoreError::UnavailableLink {
                kind: "shared folder invitation",
            });
        }
        if invitation.status == LinkStatus::Accepted {
            invitation.duplicate_accept = true;
            return Ok(invitation);
        }
        if invitation.status != LinkStatus::Pending {
            return Err(StoreError::UnavailableLink {
                kind: "shared folder invitation",
            });
        }

        let source = self.load_vault(&invitation.source_vault_id)?;
        let source_folder = source
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == invitation.source_folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: invitation.source_folder_id.to_string(),
            })?;
        if !source_folder.shared_folder_source
            || source_folder.access != FolderAccessMode::Restricted
            || source
                .setup_incomplete_folder_ids
                .contains(&invitation.source_folder_id)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder invitation source is not usable".to_owned(),
            });
        }
        validate_grant_metadata(&invitation.folder_key_grant)?;
        validate_grant_issuer(
            &source.vault,
            &invitation.folder_key_grant,
            source
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
        )?;
        if invitation.folder_key_grant.key_version != source_folder.current_key_version {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder invitation grant key version must match source folder"
                    .to_owned(),
            });
        }

        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO shared_folder_connections (
                id, source_vault_id, source_folder_id, destination_vault_id,
                destination_admin_npub, status, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6)
            ON CONFLICT(source_vault_id, source_folder_id, destination_vault_id)
            DO UPDATE SET status = 'active', updated_at = excluded.updated_at
            "#,
            params![
                connection_id,
                invitation.source_vault_id.as_str(),
                invitation.source_folder_id.as_str(),
                invitation.destination_vault_id.as_str(),
                destination_admin_npub.as_str(),
                now
            ],
        )?;
        tx.execute(
            r#"
            INSERT INTO organization_folder_mounts (
                id, organization_vault_id, source_vault_id, source_folder_id, connection_id,
                display_name, display_parent_folder_id, created_by_npub, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?8)
            ON CONFLICT(organization_vault_id, source_vault_id, source_folder_id)
            DO UPDATE SET connection_id = excluded.connection_id, updated_at = excluded.updated_at
            "#,
            params![
                mount_id,
                invitation.destination_vault_id.as_str(),
                invitation.source_vault_id.as_str(),
                invitation.source_folder_id.as_str(),
                connection_id,
                source_folder.name.as_str(),
                destination_admin_npub.as_str(),
                now
            ],
        )?;
        insert_member_if_missing(&tx, &invitation.source_vault_id, destination_admin_npub)?;
        insert_folder_access_if_missing(
            &tx,
            &invitation.source_vault_id,
            &invitation.source_folder_id,
            destination_admin_npub,
        )?;
        insert_grant_or_ignore(
            &tx,
            &invitation.source_vault_id,
            &invitation.folder_key_grant,
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO shared_folder_connection_members (connection_id, member_npub, created_at) VALUES (?1, ?2, ?3)",
            params![connection_id, destination_admin_npub.as_str(), now],
        )?;
        tx.execute(
            "UPDATE shared_folder_invitations SET status = 'accepted', updated_at = ?2, accepted_at = ?2 WHERE id = ?1 AND status = 'pending'",
            params![invitation_id, now],
        )?;
        tx.commit()?;

        self.load_shared_folder_invitation(invitation_id)
    }

    /// Load a Shared Folder Connection.
    pub fn load_shared_folder_connection(
        &self,
        connection_id: &str,
    ) -> Result<StoredSharedFolderConnection, StoreError> {
        let members = self.load_connection_members(connection_id)?;
        self.conn
            .query_row(
                r#"
                SELECT id, source_vault_id, source_folder_id, destination_vault_id,
                       destination_admin_npub, status, created_at, updated_at
                FROM shared_folder_connections
                WHERE id = ?1
                "#,
                params![connection_id],
                |row| shared_folder_connection_from_row(row, members),
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "shared folder connection",
            })
    }

    /// Load Organization Folder Mounts for one destination Vault.
    pub fn load_organization_folder_mounts(
        &self,
        organization_vault_id: &VaultId,
    ) -> Result<Vec<StoredOrganizationFolderMount>, StoreError> {
        self.require_vault_exists(organization_vault_id)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, organization_vault_id, source_vault_id, source_folder_id,
                   connection_id, display_name, display_parent_folder_id,
                   created_by_npub, created_at, updated_at
            FROM organization_folder_mounts
            WHERE organization_vault_id = ?1
            ORDER BY id
            "#,
        )?;
        let rows = stmt.query_map(
            params![organization_vault_id.as_str()],
            organization_mount_from_row,
        )?;
        let mut mounts = Vec::new();
        for row in rows {
            mounts.push(row?);
        }
        Ok(mounts)
    }

    /// Project Organization Folder Mounts as client-visible source-backed Folders.
    pub fn mounted_folder_projection(
        &self,
        organization_vault_id: &VaultId,
        actor_npub: &UserId,
    ) -> Result<Vec<MountedFolderProjection>, StoreError> {
        let mounts = self.load_organization_folder_mounts(organization_vault_id)?;
        let mut projections = Vec::new();
        for mount in mounts {
            let connection = self.load_shared_folder_connection(&mount.connection_id)?;
            let state = if connection.status == SharedFolderConnectionStatus::Revoked {
                MountedFolderState::Revoked
            } else if self.actor_has_current_source_access_and_grant(
                &mount.source_vault_id,
                &mount.source_folder_id,
                actor_npub,
            )? {
                MountedFolderState::Available
            } else {
                MountedFolderState::Locked
            };
            projections.push(MountedFolderProjection {
                mount_id: mount.id,
                organization_vault_id: mount.organization_vault_id,
                source_vault_id: mount.source_vault_id,
                source_folder_id: mount.source_folder_id,
                connection_id: mount.connection_id,
                display_name: mount.display_name,
                display_parent_folder_id: mount.display_parent_folder_id,
                state,
            });
        }
        Ok(projections)
    }

    /// Add a destination Organization member to a Shared Folder Connection.
    pub fn add_shared_folder_connection_member(
        &mut self,
        connection_id: &str,
        actor_npub: &UserId,
        target_npub: &UserId,
        grant: &FolderKeyGrantMetadata,
        created_at: &str,
    ) -> Result<StoredSharedFolderConnection, StoreError> {
        let connection = self.load_shared_folder_connection(connection_id)?;
        self.validate_destination_admin_for_connection(&connection, actor_npub)?;
        self.validate_destination_member(&connection.destination_vault_id, target_npub)?;
        let source = self.load_vault(&connection.source_vault_id)?;
        let source_folder = source
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == connection.source_folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: connection.source_folder_id.to_string(),
            })?;
        validate_connection_grant(
            grant,
            &connection.source_folder_id,
            source_folder.current_key_version,
            actor_npub,
            target_npub,
        )?;

        let tx = self.conn.transaction()?;
        insert_member_if_missing(&tx, &connection.source_vault_id, target_npub)?;
        insert_folder_access_if_missing(
            &tx,
            &connection.source_vault_id,
            &connection.source_folder_id,
            target_npub,
        )?;
        insert_grant(&tx, &connection.source_vault_id, grant)?;
        tx.execute(
            "INSERT OR IGNORE INTO shared_folder_connection_members (connection_id, member_npub, created_at) VALUES (?1, ?2, ?3)",
            params![connection_id, target_npub.as_str(), created_at],
        )?;
        tx.commit()?;

        self.load_shared_folder_connection(connection_id)
    }

    /// Remove one destination member from a Shared Folder Connection with source key rotation.
    #[allow(clippy::too_many_arguments)]
    pub fn remove_shared_folder_connection_member(
        &mut self,
        connection_id: &str,
        actor_npub: &UserId,
        target_npub: &UserId,
        new_key_version: u32,
        grants: &[FolderKeyGrantMetadata],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        updated_at: &str,
    ) -> Result<StoredSharedFolderConnection, StoreError> {
        let connection = self.load_shared_folder_connection(connection_id)?;
        self.validate_destination_admin_for_connection(&connection, actor_npub)?;
        if target_npub == &connection.destination_admin_npub {
            return Err(StoreError::BrokenInvariant {
                reason: "destination admin access must be kept while the connection is active"
                    .to_owned(),
            });
        }
        if !connection.member_npubs.contains(target_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "connection member does not exist".to_owned(),
            });
        }
        let removed_user_ids = BTreeSet::from([target_npub.clone()]);
        let rotation = SharedFolderAccessRemoval {
            removed_user_ids: &removed_user_ids,
            new_key_version,
            grants,
            reencrypted_records,
            updated_at,
        };
        self.rotate_shared_folder_access_removal(
            &connection,
            actor_npub,
            rotation,
            |tx| {
                tx.execute(
                    "DELETE FROM shared_folder_connection_members WHERE connection_id = ?1 AND member_npub = ?2",
                    params![connection_id, target_npub.as_str()],
                )?;
                Ok(())
            },
        )?;
        self.load_shared_folder_connection(connection_id)
    }

    /// Revoke a Shared Folder Connection and remove all participating destination access.
    pub fn revoke_shared_folder_connection(
        &mut self,
        connection_id: &str,
        actor_npub: &UserId,
        new_key_version: u32,
        grants: &[FolderKeyGrantMetadata],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        updated_at: &str,
    ) -> Result<StoredSharedFolderConnection, StoreError> {
        let connection = self.load_shared_folder_connection(connection_id)?;
        let source = self.load_vault(&connection.source_vault_id)?;
        if !has_vault_operational_authority(&source, actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "shared folder connection revocation requires source vault operational authority"
                    .to_owned(),
            });
        }
        let rotation = SharedFolderAccessRemoval {
            removed_user_ids: &connection.member_npubs,
            new_key_version,
            grants,
            reencrypted_records,
            updated_at,
        };
        self.rotate_shared_folder_access_removal(
            &connection,
            actor_npub,
            rotation,
            |tx| {
                tx.execute(
                    "UPDATE shared_folder_connections SET status = 'revoked', updated_at = ?2 WHERE id = ?1",
                    params![connection_id, updated_at],
                )?;
                Ok(())
            },
        )?;
        self.load_shared_folder_connection(connection_id)
    }
}
