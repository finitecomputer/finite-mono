use crate::*;

impl BrainStore {
    /// Create one npub-bound singleton Vault Invitation.
    #[allow(clippy::too_many_arguments)]
    pub fn create_vault_invitation(
        &mut self,
        vault_id: &VaultId,
        id: &str,
        user_id: &UserId,
        invite_code: &str,
        accept_path: &str,
        initial_folder_access: &[FolderId],
        created_by_npub: &UserId,
        expires_at: &str,
        created_at: &str,
    ) -> Result<StoredVaultInvitation, StoreError> {
        let vault = self.load_core_vault(vault_id)?;
        if vault.kind != VaultKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "vault invitations require an organization vault".to_owned(),
            });
        }
        if !vault.admins.contains(created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "vault invitations must be created by a vault admin".to_owned(),
            });
        }
        if self.member_exists(vault_id, user_id)? {
            return Err(StoreError::BrokenInvariant {
                reason: "target is already a vault member".to_owned(),
            });
        }
        validate_link_id("vault_invitation_id", id)?;
        validate_link_id("invite_code", invite_code)?;
        validate_link_timestamp("expiresAt", expires_at)?;
        for folder_id in initial_folder_access {
            ensure_folder_exists(&self.conn, vault_id, folder_id)?;
        }
        let initial_folder_access_json = folder_id_vec_json(initial_folder_access)?;

        self.conn
            .execute(
                r#"
                INSERT INTO vault_invitations (
                    id, vault_id, user_id, status, invite_code, accept_path,
                    initial_folder_access_json, created_by_npub, expires_at,
                    created_at, updated_at
                )
                VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                "#,
                params![
                    id,
                    vault_id.as_str(),
                    user_id.as_str(),
                    invite_code,
                    accept_path,
                    initial_folder_access_json,
                    created_by_npub.as_str(),
                    expires_at,
                    created_at
                ],
            )
            .map_err(map_insert_error("vault_invitation_id", id))?;

        self.load_vault_invitation(id)
    }

    /// Load one Vault Invitation by id.
    pub fn load_vault_invitation(
        &self,
        invitation_id: &str,
    ) -> Result<StoredVaultInvitation, StoreError> {
        self.conn
            .query_row(
                r#"
                SELECT id, vault_id, user_id, status, invite_code, accept_path,
                       initial_folder_access_json, created_by_npub, expires_at,
                       created_at, updated_at, accepted_at
                FROM vault_invitations
                WHERE id = ?1
                "#,
                params![invitation_id],
                vault_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "vault invitation",
            })
    }

    /// List Vault Invitations for one Vault, newest first, bounded by MAX_LINK_LIST_ROWS.
    pub fn list_vault_invitations(
        &self,
        vault_id: &VaultId,
    ) -> Result<Vec<StoredVaultInvitation>, StoreError> {
        self.require_vault_exists(vault_id)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, vault_id, user_id, status, invite_code, accept_path,
                   initial_folder_access_json, created_by_npub, expires_at,
                   created_at, updated_at, accepted_at
            FROM vault_invitations
            WHERE vault_id = ?1
            ORDER BY created_at DESC, id
            LIMIT ?2
            "#,
        )?;
        let rows = stmt.query_map(
            params![vault_id.as_str(), MAX_LINK_LIST_ROWS],
            vault_invitation_from_row,
        )?;
        let mut invitations = Vec::new();
        for row in rows {
            invitations.push(row?);
        }
        Ok(invitations)
    }

    /// Load a pending Vault Invitation by invite code for its target user only.
    pub fn load_available_vault_invitation_by_code(
        &self,
        invite_code: &str,
        user_id: &UserId,
        now: &str,
    ) -> Result<StoredVaultInvitation, StoreError> {
        let invitation = self
            .conn
            .query_row(
                r#"
                SELECT id, vault_id, user_id, status, invite_code, accept_path,
                       initial_folder_access_json, created_by_npub, expires_at,
                       created_at, updated_at, accepted_at
                FROM vault_invitations
                WHERE invite_code = ?1
                "#,
                params![invite_code],
                vault_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "vault invitation",
            })?;
        ensure_invitation_available(&invitation, user_id, now)?;
        Ok(invitation)
    }

    /// Revoke a Vault Invitation delivery handle. Accepted membership is unchanged.
    pub fn revoke_vault_invitation(
        &mut self,
        vault_id: &VaultId,
        invitation_id: &str,
        actor_npub: &UserId,
        updated_at: &str,
    ) -> Result<StoredVaultInvitation, StoreError> {
        let vault = self.load_core_vault(vault_id)?;
        if !vault.admins.contains(actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "vault invitation revocation requires a vault admin".to_owned(),
            });
        }
        let invitation = self.load_vault_invitation(invitation_id)?;
        if invitation.vault_id != *vault_id {
            return Err(StoreError::UnavailableLink {
                kind: "vault invitation",
            });
        }
        self.conn.execute(
            "UPDATE vault_invitations SET status = 'revoked', updated_at = ?3 WHERE vault_id = ?1 AND id = ?2",
            params![vault_id.as_str(), invitation_id, updated_at],
        )?;
        self.load_vault_invitation(invitation_id)
    }

    /// Accept a pending Vault Invitation, adding the target as a member exactly once.
    pub fn accept_vault_invitation_by_code(
        &mut self,
        invite_code: &str,
        user_id: &UserId,
        now: &str,
    ) -> Result<StoredVaultInvitation, StoreError> {
        let mut invitation = self
            .conn
            .query_row(
                r#"
                SELECT id, vault_id, user_id, status, invite_code, accept_path,
                       initial_folder_access_json, created_by_npub, expires_at,
                       created_at, updated_at, accepted_at
                FROM vault_invitations
                WHERE invite_code = ?1
                "#,
                params![invite_code],
                vault_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "vault invitation",
            })?;

        if invitation.user_id != *user_id {
            return Err(StoreError::UnavailableLink {
                kind: "vault invitation",
            });
        }
        if invitation.status == LinkStatus::Accepted {
            invitation.duplicate_accept = true;
            return Ok(invitation);
        }
        ensure_invitation_available(&invitation, user_id, now)?;
        let already_member = self.member_exists(&invitation.vault_id, user_id)?;

        let tx = self.conn.transaction()?;
        insert_member_if_missing(&tx, &invitation.vault_id, user_id)?;
        tx.execute(
            r#"
            UPDATE vault_invitations
            SET status = 'accepted', updated_at = ?3, accepted_at = ?3
            WHERE vault_id = ?1 AND id = ?2 AND status = 'pending'
            "#,
            params![invitation.vault_id.as_str(), invitation.id, now],
        )?;
        tx.commit()?;

        let mut invitation = self.load_vault_invitation(&invitation.id)?;
        invitation.duplicate_accept = already_member;
        Ok(invitation)
    }

    /// Create one npub-bound singleton Share Link for a restricted Folder.
    #[allow(clippy::too_many_arguments)]
    pub fn create_share_link(
        &mut self,
        vault_id: &VaultId,
        folder_id: &FolderId,
        id: &str,
        recipient_npub: &UserId,
        created_by_npub: &UserId,
        expires_at: &str,
        accept_path: &str,
        grant: &FolderKeyGrantMetadata,
        create_personal_mount: bool,
        created_at: &str,
    ) -> Result<StoredShareLink, StoreError> {
        let stored = self.load_vault(vault_id)?;
        if stored.vault.kind != VaultKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "share links require an organization source vault".to_owned(),
            });
        }
        if !stored.vault.admins.contains(created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "share links must be created by a vault admin".to_owned(),
            });
        }
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
                reason: "share links require a restricted folder".to_owned(),
            });
        }
        validate_link_id("share_link_id", id)?;
        validate_link_timestamp("expiresAt", expires_at)?;
        validate_grant_metadata(grant)?;
        validate_grant_issuer(&stored.vault, grant)?;
        if grant.folder_id != *folder_id
            || grant.key_version != folder.current_key_version
            || grant.recipient_npub != *recipient_npub
            || grant.issuer_npub != *created_by_npub
        {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "share link grant must match folder, current key version, issuer, and recipient"
                        .to_owned(),
            });
        }
        let access_change_event_json =
            grant
                .access_change_event_json
                .clone()
                .ok_or_else(|| StoreError::BrokenInvariant {
                    reason: "share link requires an access-change event".to_owned(),
                })?;

        self.conn
            .execute(
                r#"
                INSERT INTO share_links (
                    id, vault_id, folder_id, recipient_npub, created_by_npub, status,
                    accept_path, expires_at, created_at, updated_at, grant_id,
                    grant_key_version, grant_wrapped_event_json, access_change_event_json,
                    create_personal_mount
                )
                VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, ?8, ?8, ?9, ?10, ?11, ?12, ?13)
                "#,
                params![
                    id,
                    vault_id.as_str(),
                    folder_id.as_str(),
                    recipient_npub.as_str(),
                    created_by_npub.as_str(),
                    accept_path,
                    expires_at,
                    created_at,
                    grant.id,
                    grant.key_version,
                    grant.wrapped_event_json,
                    access_change_event_json,
                    create_personal_mount
                ],
            )
            .map_err(map_insert_error("share_link_id", id))?;

        self.load_share_link(id)
    }

    /// Load one Share Link by id.
    pub fn load_share_link(&self, share_link_id: &str) -> Result<StoredShareLink, StoreError> {
        self.conn
            .query_row(
                r#"
                SELECT id, vault_id, folder_id, recipient_npub, created_by_npub, status,
                       accept_path, expires_at, created_at, updated_at, accepted_at,
                       grant_id, grant_key_version, grant_wrapped_event_json,
                       access_change_event_json, create_personal_mount, personal_mount_id
                FROM share_links
                WHERE id = ?1
                "#,
                params![share_link_id],
                share_link_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink { kind: "share link" })
    }

    /// List Share Links for one Folder, newest first, bounded by MAX_LINK_LIST_ROWS.
    pub fn list_folder_share_links(
        &self,
        vault_id: &VaultId,
        folder_id: &FolderId,
    ) -> Result<Vec<StoredShareLink>, StoreError> {
        ensure_folder_exists(&self.conn, vault_id, folder_id)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, vault_id, folder_id, recipient_npub, created_by_npub, status,
                   accept_path, expires_at, created_at, updated_at, accepted_at,
                   grant_id, grant_key_version, grant_wrapped_event_json,
                   access_change_event_json, create_personal_mount, personal_mount_id
            FROM share_links
            WHERE vault_id = ?1 AND folder_id = ?2
            ORDER BY created_at DESC, id
            LIMIT ?3
            "#,
        )?;
        let rows = stmt.query_map(
            params![vault_id.as_str(), folder_id.as_str(), MAX_LINK_LIST_ROWS],
            share_link_from_row,
        )?;
        let mut share_links = Vec::new();
        for row in rows {
            share_links.push(row?);
        }
        Ok(share_links)
    }

    /// Load a pending Share Link for its recipient only.
    pub fn load_available_share_link(
        &self,
        share_link_id: &str,
        recipient_npub: &UserId,
        now: &str,
    ) -> Result<StoredShareLink, StoreError> {
        let share_link = self.load_share_link(share_link_id)?;
        ensure_share_link_available(&share_link, recipient_npub, now)?;
        Ok(share_link)
    }

    /// Revoke a Share Link delivery handle. Accepted access is unchanged.
    pub fn revoke_share_link(
        &mut self,
        share_link_id: &str,
        actor_npub: &UserId,
        updated_at: &str,
    ) -> Result<StoredShareLink, StoreError> {
        let share_link = self.load_share_link(share_link_id)?;
        let vault = self.load_core_vault(&share_link.vault_id)?;
        if !vault.admins.contains(actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "share link revocation requires a vault admin".to_owned(),
            });
        }
        self.conn.execute(
            "UPDATE share_links SET status = 'revoked', updated_at = ?2 WHERE id = ?1",
            params![share_link_id, updated_at],
        )?;
        self.load_share_link(share_link_id)
    }

    /// Accept a pending Share Link, creating membership, restricted access, grant, and optional mount state.
    pub fn accept_share_link(
        &mut self,
        share_link_id: &str,
        recipient_npub: &UserId,
        now: &str,
    ) -> Result<StoredShareLink, StoreError> {
        let mut share_link = self.load_share_link(share_link_id)?;
        if share_link.recipient_npub != *recipient_npub {
            return Err(StoreError::UnavailableLink { kind: "share link" });
        }
        if share_link.status == LinkStatus::Accepted {
            share_link.duplicate_accept = true;
            return Ok(share_link);
        }
        ensure_share_link_available(&share_link, recipient_npub, now)?;

        let stored = self.load_vault(&share_link.vault_id)?;
        let folder = stored
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == share_link.folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: share_link.folder_id.to_string(),
            })?;
        if folder.access != FolderAccessMode::Restricted {
            return Err(StoreError::BrokenInvariant {
                reason: "share links require a restricted folder".to_owned(),
            });
        }
        validate_grant_metadata(&share_link.folder_key_grant)?;
        validate_grant_issuer(&stored.vault, &share_link.folder_key_grant)?;
        if share_link.folder_key_grant.key_version != folder.current_key_version {
            return Err(StoreError::BrokenInvariant {
                reason: "share link grant key version must match folder current key version"
                    .to_owned(),
            });
        }

        let tx = self.conn.transaction()?;
        insert_member_if_missing(&tx, &share_link.vault_id, recipient_npub)?;
        tx.execute(
            "INSERT INTO folder_access (vault_id, folder_id, user_id) VALUES (?1, ?2, ?3)",
            params![
                share_link.vault_id.as_str(),
                share_link.folder_id.as_str(),
                recipient_npub.as_str()
            ],
        )?;
        insert_grant(&tx, &share_link.vault_id, &share_link.folder_key_grant)?;

        let personal_mount_id = if share_link.create_personal_mount {
            let mount_id =
                personal_mount_id(recipient_npub, &share_link.vault_id, &share_link.folder_id);
            tx.execute(
                r#"
                INSERT INTO personal_folder_mounts (
                    id, owner_npub, source_vault_id, source_folder_id, display_name,
                    display_parent_folder_id, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?6)
                ON CONFLICT(owner_npub, source_vault_id, source_folder_id) DO UPDATE SET
                    updated_at = excluded.updated_at
                "#,
                params![
                    mount_id,
                    recipient_npub.as_str(),
                    share_link.vault_id.as_str(),
                    share_link.folder_id.as_str(),
                    folder.name.as_str(),
                    now
                ],
            )?;
            Some(mount_id)
        } else {
            None
        };
        tx.execute(
            r#"
            UPDATE share_links
            SET status = 'accepted', updated_at = ?2, accepted_at = ?2, personal_mount_id = ?3
            WHERE id = ?1 AND status = 'pending'
            "#,
            params![share_link_id, now, personal_mount_id],
        )?;
        tx.commit()?;

        self.load_share_link(share_link_id)
    }
}
