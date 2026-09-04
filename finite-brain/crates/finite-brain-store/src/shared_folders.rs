use crate::*;

impl BrainStore {
    pub(crate) fn list_active_destination_mounts_for_participant(
        &self,
        destination_brain_id: &BrainId,
        participant: &UserId,
    ) -> Result<Vec<StoredSharedFolderConnection>, StoreError> {
        let limit = BRAIN_CAPACITY_ENVELOPE.mounts + 1;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT connections.id
            FROM shared_folder_connections connections
            JOIN shared_folder_connection_members participants
              ON participants.connection_id = connections.id
            WHERE connections.destination_brain_id = ?1
              AND connections.status = 'active'
              AND participants.member_npub = ?2
            ORDER BY connections.created_at DESC, connections.id ASC
            LIMIT ?3
            "#,
        )?;
        let ids = stmt
            .query_map(
                params![destination_brain_id.as_str(), participant.as_str(), limit],
                |row| row.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        if ids.len() > BRAIN_CAPACITY_ENVELOPE.mounts {
            return Err(StoreError::BrokenInvariant {
                reason: "active destination Mount participation exceeds the capacity envelope"
                    .to_owned(),
            });
        }
        ids.into_iter()
            .map(|id| self.load_shared_folder_connection(&id))
            .collect()
    }

    /// Create a Shared Folder Invitation from a source Folder to a destination Organization admin.
    #[allow(clippy::too_many_arguments)]
    pub fn create_shared_folder_invitation(
        &mut self,
        source_brain_id: &BrainId,
        source_folder_id: &FolderId,
        destination_brain_id: &BrainId,
        id: &str,
        destination_admin_npub: &UserId,
        created_by_npub: &UserId,
        accept_path: &str,
        grant: &FolderKeyGrantMetadata,
        expires_at: &str,
        created_at: &str,
    ) -> Result<StoredSharedFolderInvitation, StoreError> {
        let source = self.load_brain(source_brain_id)?;
        let source_folder = source
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == *source_folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: source_folder_id.to_string(),
            })?;
        if !has_brain_operational_authority(&source, created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "Mount Offers require source Brain operational authority".to_owned(),
            });
        }
        if source
            .setup_incomplete_folder_ids
            .contains(source_folder_id)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Mount Offer source Folder setup must be complete".to_owned(),
            });
        }
        let destination = self.load_brain(destination_brain_id)?;
        if !has_brain_operational_authority(&destination, destination_admin_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "mount offer target must control the destination brain".to_owned(),
            });
        }
        validate_link_id("shared_folder_invitation_id", id)?;
        validate_bounded_offer_expiry(expires_at, created_at)?;
        validate_grant_metadata(grant)?;
        validate_grant_issuer(
            &source.brain,
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
                reason: "Mount Offer grant must match source Folder, key version, issuer, and destination controller"
                    .to_owned(),
            });
        }
        let access_change_event_json =
            grant
                .access_change_event_json
                .clone()
                .ok_or_else(|| StoreError::BrokenInvariant {
                    reason: "Mount Offer requires an access-change event".to_owned(),
                })?;

        self.conn
            .execute(
                r#"
                INSERT INTO shared_folder_invitations (
                    id, source_brain_id, source_folder_id, destination_brain_id,
                    destination_admin_npub, created_by_npub, status, current_key_version,
                    accept_path, created_at, updated_at, grant_id, grant_wrapped_event_json,
                    access_change_event_json, expires_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9, ?9, ?10, ?11, ?12, ?13)
                "#,
                params![
                    id,
                    source_brain_id.as_str(),
                    source_folder_id.as_str(),
                    destination_brain_id.as_str(),
                    destination_admin_npub.as_str(),
                    created_by_npub.as_str(),
                    source_folder.current_key_version,
                    accept_path,
                    created_at,
                    grant.id,
                    grant.wrapped_event_json,
                    access_change_event_json,
                    expires_at
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
                SELECT id, source_brain_id, source_folder_id, destination_brain_id,
                       destination_admin_npub, created_by_npub, status, current_key_version,
                       accept_path, created_at, updated_at, accepted_at, grant_id,
                       grant_wrapped_event_json, access_change_event_json, expires_at
                FROM shared_folder_invitations
                WHERE id = ?1
                "#,
                params![invitation_id],
                shared_folder_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "Mount Offer",
            })
    }

    /// List Shared Folder Invitations for one Brain as source or destination,
    /// newest first, bounded by MAX_LINK_LIST_ROWS.
    pub fn list_shared_folder_invitations(
        &self,
        brain_id: &BrainId,
        direction: SharedFolderDirection,
    ) -> Result<Vec<StoredSharedFolderInvitation>, StoreError> {
        self.require_brain_exists(brain_id)?;
        let column = match direction {
            SharedFolderDirection::Source => "source_brain_id",
            SharedFolderDirection::Destination => "destination_brain_id",
        };
        let mut stmt = self.conn.prepare(&format!(
            r#"
            SELECT id, source_brain_id, source_folder_id, destination_brain_id,
                   destination_admin_npub, created_by_npub, status, current_key_version,
                   accept_path, created_at, updated_at, accepted_at, grant_id,
                   grant_wrapped_event_json, access_change_event_json, expires_at
            FROM shared_folder_invitations
            WHERE {column} = ?1
            ORDER BY created_at DESC, id
            LIMIT ?2
            "#
        ))?;
        let rows = stmt.query_map(
            params![brain_id.as_str(), MAX_LINK_LIST_ROWS],
            shared_folder_invitation_from_row,
        )?;
        let mut invitations = Vec::new();
        for row in rows {
            invitations.push(row?);
        }
        Ok(invitations)
    }

    /// List Shared Folder Connections for one Brain as source or destination,
    /// newest first, bounded by MAX_LINK_LIST_ROWS. Members are included per connection.
    pub fn list_shared_folder_connections(
        &self,
        brain_id: &BrainId,
        direction: SharedFolderDirection,
    ) -> Result<Vec<StoredSharedFolderConnection>, StoreError> {
        self.require_brain_exists(brain_id)?;
        let column = match direction {
            SharedFolderDirection::Source => "source_brain_id",
            SharedFolderDirection::Destination => "destination_brain_id",
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
            let rows = stmt.query_map(params![brain_id.as_str(), MAX_LINK_LIST_ROWS], |row| {
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
        if invitation.status != LinkStatus::Pending {
            return Err(StoreError::UnavailableLink {
                kind: "Mount Offer",
            });
        }
        let source = self.load_brain(&invitation.source_brain_id)?;
        if !has_brain_operational_authority(&source, actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "Mount Offer revocation requires source Brain operational authority"
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
    #[allow(clippy::too_many_arguments)]
    pub fn accept_shared_folder_invitation(
        &mut self,
        invitation_id: &str,
        destination_admin_npub: &UserId,
        connection_id: &str,
        mount_id: &str,
        supplemental_grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
        now: &str,
    ) -> Result<StoredSharedFolderInvitation, StoreError> {
        let mut invitation = self.load_shared_folder_invitation(invitation_id)?;
        if invitation.destination_admin_npub != *destination_admin_npub {
            return Err(StoreError::UnavailableLink {
                kind: "Mount Offer",
            });
        }
        if invitation.status == LinkStatus::Accepted {
            invitation.duplicate_accept = true;
            return Ok(invitation);
        }
        if invitation.status != LinkStatus::Pending
            || timestamp_expired(&invitation.expires_at, now)
        {
            return Err(StoreError::UnavailableLink {
                kind: "Mount Offer",
            });
        }

        let source = self.load_brain(&invitation.source_brain_id)?;
        let source_folder = source
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == invitation.source_folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: invitation.source_folder_id.to_string(),
            })?;
        if source
            .setup_incomplete_folder_ids
            .contains(&invitation.source_folder_id)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Mount Offer source Folder is not usable".to_owned(),
            });
        }
        validate_grant_metadata(&invitation.folder_key_grant)?;
        validate_grant_issuer(
            &source.brain,
            &invitation.folder_key_grant,
            source
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
        )?;
        if invitation.folder_key_grant.key_version != source_folder.current_key_version {
            return Err(StoreError::BrokenInvariant {
                reason: "Mount Offer grant key version must match source Folder".to_owned(),
            });
        }
        let destination = self.load_brain(&invitation.destination_brain_id)?;
        if !has_brain_operational_authority(&destination, destination_admin_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "mount acceptance requires destination brain control".to_owned(),
            });
        }
        let mut participants = BTreeSet::from([destination_admin_npub.clone()]);
        if destination.brain.kind == BrainKind::Personal {
            if let Some(owner) = destination.brain.owner_user_id.as_ref() {
                participants.insert(owner.clone());
            }
            if let Some(agent) = destination.personal_agent.as_ref() {
                participants.insert(agent.agent_npub.clone());
            }
        }
        let mut participant_grants = BTreeMap::from([(
            invitation.folder_key_grant.recipient_npub.clone(),
            invitation.folder_key_grant.clone(),
        )]);
        for grant in supplemental_grants {
            validate_grant_metadata(grant)?;
            if grant.folder_id != invitation.source_folder_id
                || grant.key_version != source_folder.current_key_version
                || grant.issuer_npub != *destination_admin_npub
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "mount participant grant does not match the source Folder or accepting controller"
                        .to_owned(),
                });
            }
            participant_grants.insert(grant.recipient_npub.clone(), grant.clone());
        }
        if participant_grants.keys().cloned().collect::<BTreeSet<_>>() != participants {
            return Err(StoreError::BrokenInvariant {
                reason: "mount acceptance requires one current Folder Key Grant for every initial participant"
                    .to_owned(),
            });
        }
        let all_grants = std::iter::once(&invitation.folder_key_grant)
            .chain(supplemental_grants)
            .cloned()
            .collect::<Vec<_>>();
        validate_folder_key_grant_control_records(&all_grants, control_records)?;

        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO shared_folder_connections (
                id, source_brain_id, source_folder_id, destination_brain_id,
                destination_admin_npub, status, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6)
            ON CONFLICT(source_brain_id, source_folder_id, destination_brain_id)
            DO UPDATE SET status = 'active', updated_at = excluded.updated_at
            "#,
            params![
                connection_id,
                invitation.source_brain_id.as_str(),
                invitation.source_folder_id.as_str(),
                invitation.destination_brain_id.as_str(),
                destination_admin_npub.as_str(),
                now
            ],
        )?;
        tx.execute(
            r#"
            INSERT INTO folder_mounts (
                id, destination_brain_id, source_brain_id, source_folder_id, connection_id,
                display_name, display_parent_folder_id, created_by_npub, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?8)
            ON CONFLICT(destination_brain_id, source_brain_id, source_folder_id)
            DO UPDATE SET connection_id = excluded.connection_id, updated_at = excluded.updated_at
            "#,
            params![
                mount_id,
                invitation.destination_brain_id.as_str(),
                invitation.source_brain_id.as_str(),
                invitation.source_folder_id.as_str(),
                connection_id,
                source_folder.name.as_str(),
                destination_admin_npub.as_str(),
                now
            ],
        )?;
        for participant in &participants {
            insert_folder_access_if_missing(
                &tx,
                &invitation.source_brain_id,
                &invitation.source_folder_id,
                participant,
            )?;
            insert_folder_access_source(
                &tx,
                &invitation.source_brain_id,
                &invitation.source_folder_id,
                participant,
                "mount",
                connection_id,
                now,
            )?;
            insert_grant_or_ignore(
                &tx,
                &invitation.source_brain_id,
                participant_grants
                    .get(participant)
                    .expect("participant grants checked above"),
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO shared_folder_connection_members
                 (connection_id, member_npub, created_at, manages_folder_access)
                 VALUES (?1, ?2, ?3, 0)",
                params![connection_id, participant.as_str(), now],
            )?;
        }
        sync_records::append_sync_records(&tx, &invitation.source_brain_id, control_records)?;
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
        let managed_access_npubs = self.load_connection_managed_access_members(connection_id)?;
        self.conn
            .query_row(
                r#"
                SELECT id, source_brain_id, source_folder_id, destination_brain_id,
                       destination_admin_npub, status, created_at, updated_at
                FROM shared_folder_connections
                WHERE id = ?1
                "#,
                params![connection_id],
                |row| {
                    let mut connection = shared_folder_connection_from_row(row, members)?;
                    connection.managed_access_npubs = managed_access_npubs;
                    Ok(connection)
                },
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink { kind: "Mount" })
    }

    /// Load Folder Mounts for one destination Brain.
    pub fn load_folder_mounts(
        &self,
        destination_brain_id: &BrainId,
    ) -> Result<Vec<StoredFolderMount>, StoreError> {
        self.require_brain_exists(destination_brain_id)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, destination_brain_id, source_brain_id, source_folder_id,
                   connection_id, display_name, display_parent_folder_id,
                   created_by_npub, created_at, updated_at
            FROM folder_mounts
            WHERE destination_brain_id = ?1
            ORDER BY id
            "#,
        )?;
        let rows = stmt.query_map(
            params![destination_brain_id.as_str()],
            folder_mount_from_row,
        )?;
        let mut mounts = Vec::new();
        for row in rows {
            mounts.push(row?);
        }
        Ok(mounts)
    }

    /// Project Folder Mounts as client-visible source-backed Folders.
    pub fn mounted_folder_projection(
        &self,
        destination_brain_id: &BrainId,
        actor_npub: &UserId,
    ) -> Result<Vec<MountedFolderProjection>, StoreError> {
        let mounts = self.load_folder_mounts(destination_brain_id)?;
        let mut projections = Vec::new();
        for mount in mounts {
            let connection = self.load_shared_folder_connection(&mount.connection_id)?;
            let state = if connection.status == SharedFolderConnectionStatus::Revoked {
                MountedFolderState::Revoked
            } else if self.actor_has_current_source_access_and_grant(
                &mount.source_brain_id,
                &mount.source_folder_id,
                actor_npub,
            )? {
                MountedFolderState::Available
            } else {
                MountedFolderState::Locked
            };
            projections.push(MountedFolderProjection {
                mount_id: mount.id,
                destination_brain_id: mount.destination_brain_id,
                source_brain_id: mount.source_brain_id,
                source_folder_id: mount.source_folder_id,
                connection_id: mount.connection_id,
                display_name: mount.display_name,
                display_parent_folder_id: mount.display_parent_folder_id,
                state,
            });
        }
        Ok(projections)
    }

    /// Add one destination-governed identity to a Shared Folder Connection.
    pub fn add_shared_folder_connection_member(
        &mut self,
        connection_id: &str,
        actor_npub: &UserId,
        target_npub: &UserId,
        grant: &FolderKeyGrantMetadata,
        control_records: &[SyncRecordInput],
        created_at: &str,
    ) -> Result<StoredSharedFolderConnection, StoreError> {
        let connection = self.load_shared_folder_connection(connection_id)?;
        self.validate_destination_admin_for_connection(&connection, actor_npub)?;
        self.validate_destination_member(&connection.destination_brain_id, target_npub)?;
        let source = self.load_brain(&connection.source_brain_id)?;
        let source_folder = source
            .brain
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
        validate_folder_key_grant_control_records(std::slice::from_ref(grant), control_records)?;

        let tx = self.conn.transaction()?;
        insert_folder_access_if_missing(
            &tx,
            &connection.source_brain_id,
            &connection.source_folder_id,
            target_npub,
        )?;
        insert_folder_access_source(
            &tx,
            &connection.source_brain_id,
            &connection.source_folder_id,
            target_npub,
            "mount",
            connection_id,
            created_at,
        )?;
        insert_grant_or_ignore(&tx, &connection.source_brain_id, grant)?;
        sync_records::append_sync_records(&tx, &connection.source_brain_id, control_records)?;
        tx.execute(
            "INSERT OR IGNORE INTO shared_folder_connection_members
             (connection_id, member_npub, created_at, manages_folder_access)
             VALUES (?1, ?2, ?3, 0)",
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
        control_records: &[SyncRecordInput],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        updated_at: &str,
    ) -> Result<StoredSharedFolderConnection, StoreError> {
        let connection = self.load_shared_folder_connection(connection_id)?;
        self.validate_destination_admin_for_connection(&connection, actor_npub)?;
        if target_npub == &connection.destination_admin_npub {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "destination controller must remain a participant while the Mount is active"
                        .to_owned(),
            });
        }
        if !connection.member_npubs.contains(target_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "Mount Participant does not exist".to_owned(),
            });
        }
        if !connection.managed_access_npubs.contains(target_npub) {
            validate_folder_key_grant_control_records(grants, control_records)?;
            let tx = self.conn.transaction()?;
            delete_folder_access_source(
                &tx,
                &connection.source_brain_id,
                &connection.source_folder_id,
                target_npub,
                "mount",
                connection_id,
            )?;
            tx.execute(
                "DELETE FROM shared_folder_connection_members
                 WHERE connection_id = ?1 AND member_npub = ?2",
                params![connection_id, target_npub.as_str()],
            )?;
            tx.commit()?;
            return self.load_shared_folder_connection(connection_id);
        }
        let removed_user_ids = BTreeSet::from([target_npub.clone()]);
        let rotation = SharedFolderAccessRemoval {
            removed_user_ids: &removed_user_ids,
            new_key_version,
            grants,
            control_records,
            reencrypted_records,
            updated_at,
        };
        self.rotate_shared_folder_access_removal(
            &connection,
            actor_npub,
            rotation,
            |tx| {
                delete_folder_access_source(
                    tx,
                    &connection.source_brain_id,
                    &connection.source_folder_id,
                    target_npub,
                    "mount",
                    connection_id,
                )?;
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
    #[allow(clippy::too_many_arguments)]
    pub fn revoke_shared_folder_connection(
        &mut self,
        connection_id: &str,
        actor_npub: &UserId,
        new_key_version: u32,
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        updated_at: &str,
    ) -> Result<StoredSharedFolderConnection, StoreError> {
        let connection = self.load_shared_folder_connection(connection_id)?;
        let source = self.load_brain(&connection.source_brain_id)?;
        let destination = self.load_brain(&connection.destination_brain_id)?;
        if !has_brain_operational_authority(&source, actor_npub)
            && !has_brain_operational_authority(&destination, actor_npub)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "mount revocation requires source or destination brain control".to_owned(),
            });
        }
        if connection.managed_access_npubs.is_empty() {
            validate_folder_key_grant_control_records(grants, control_records)?;
            let tx = self.conn.transaction()?;
            delete_folder_access_sources_for_origin(&tx, "mount", connection_id)?;
            tx.execute(
                "UPDATE shared_folder_connections
                 SET status = 'revoked', updated_at = ?2 WHERE id = ?1",
                params![connection_id, updated_at],
            )?;
            tx.commit()?;
            return self.load_shared_folder_connection(connection_id);
        }
        let rotation = SharedFolderAccessRemoval {
            removed_user_ids: &connection.managed_access_npubs,
            new_key_version,
            grants,
            control_records,
            reencrypted_records,
            updated_at,
        };
        self.rotate_shared_folder_access_removal(
            &connection,
            actor_npub,
            rotation,
            |tx| {
                delete_folder_access_sources_for_origin(tx, "mount", connection_id)?;
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
