use crate::*;

const BRAIN_INVITATION_SELECT: &str = r#"
    SELECT id, brain_id, user_id, status, invite_code, accept_path,
           initial_folder_access_json, created_by_npub, expires_at,
           created_at, updated_at, accepted_at, target_kind, invited_email,
           invite_unwrap_npub, bootstrap_payload_hash, bootstrap_wrapped_event_json,
           bootstrap_authorization_event_json, claimed_by_npub, bootstrap_scope_json
    FROM brain_invitations
"#;

impl BrainStore {
    /// Create one npub-bound singleton Brain Invitation.
    #[allow(clippy::too_many_arguments)]
    pub fn create_brain_invitation(
        &mut self,
        brain_id: &BrainId,
        id: &str,
        user_id: &UserId,
        invite_code: &str,
        accept_path: &str,
        initial_folder_access: &[FolderId],
        created_by_npub: &UserId,
        expires_at: &str,
        created_at: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let brain = self.load_core_brain(brain_id)?;
        if brain.kind != BrainKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "brain invitations require an organization brain".to_owned(),
            });
        }
        if !brain.admins.contains(created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "brain invitations must be created by a brain admin".to_owned(),
            });
        }
        if self.member_exists(brain_id, user_id)? {
            return Err(StoreError::BrokenInvariant {
                reason: "target is already a brain member".to_owned(),
            });
        }
        validate_link_id("brain_invitation_id", id)?;
        validate_link_id("invite_code", invite_code)?;
        validate_link_timestamp("expiresAt", expires_at)?;
        for folder_id in initial_folder_access {
            ensure_folder_exists(&self.conn, brain_id, folder_id)?;
        }
        let initial_folder_access_json = folder_id_vec_json(initial_folder_access)?;

        self.conn
            .execute(
                r#"
                INSERT INTO brain_invitations (
                    id, brain_id, user_id, target_kind, status, invite_code, accept_path,
                    initial_folder_access_json, created_by_npub, expires_at,
                    created_at, updated_at, bootstrap_scope_json
                )
                VALUES (?1, ?2, ?3, 'npub', 'pending', ?4, ?5, ?6, ?7, ?8, ?9, ?9, '[]')
                "#,
                params![
                    id,
                    brain_id.as_str(),
                    user_id.as_str(),
                    invite_code,
                    accept_path,
                    initial_folder_access_json,
                    created_by_npub.as_str(),
                    expires_at,
                    created_at
                ],
            )
            .map_err(map_insert_error("brain_invitation_id", id))?;

        self.load_brain_invitation(id)
    }

    /// Create one email-targeted Brain Invitation with encrypted bootstrap material.
    #[allow(clippy::too_many_arguments)]
    pub fn create_email_brain_invitation(
        &mut self,
        brain_id: &BrainId,
        id: &str,
        invited_email: &str,
        invite_unwrap_npub: &UserId,
        bootstrap_payload_hash: &str,
        bootstrap_wrapped_event_json: &str,
        bootstrap_authorization_event_json: &str,
        invite_code: &str,
        accept_path: &str,
        selected_restricted_folder_access: &[FolderId],
        created_by_npub: &UserId,
        expires_at: &str,
        created_at: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let brain = self.load_core_brain(brain_id)?;
        if brain.kind != BrainKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "email brain invitations require an organization brain".to_owned(),
            });
        }
        if !brain.admins.contains(created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "email brain invitations must be created by a brain admin".to_owned(),
            });
        }
        validate_link_id("brain_invitation_id", id)?;
        validate_link_id("invite_code", invite_code)?;
        validate_link_timestamp("expiresAt", expires_at)?;
        let invited_email = canonical_invited_email(invited_email)?;
        validate_required_text("bootstrapPayloadHash", bootstrap_payload_hash)?;
        validate_required_text("bootstrapWrappedEventJson", bootstrap_wrapped_event_json)?;
        validate_required_text(
            "bootstrapAuthorizationEventJson",
            bootstrap_authorization_event_json,
        )?;
        let bootstrap_scope = email_bootstrap_scope(&brain, selected_restricted_folder_access)?;
        let initial_folder_access = bootstrap_scope
            .iter()
            .map(|scope| scope.folder_id.clone())
            .collect::<Vec<_>>();
        let initial_folder_access_json = folder_id_vec_json(&initial_folder_access)?;
        let bootstrap_scope_json = serde_json::to_string(&bootstrap_scope).map_err(|error| {
            StoreError::BrokenInvariant {
                reason: format!("email bootstrap scope did not serialize: {error}"),
            }
        })?;

        self.conn.execute(
            r#"
            UPDATE brain_invitations
            SET status = 'revoked',
                bootstrap_wrapped_event_json = NULL,
                updated_at = ?3
            WHERE brain_id = ?1
              AND target_kind = 'email_bootstrap'
              AND invited_email = ?2
              AND status = 'pending'
            "#,
            params![brain_id.as_str(), invited_email, created_at],
        )?;

        self.conn
            .execute(
                r#"
                INSERT INTO brain_invitations (
                    id, brain_id, user_id, target_kind, invited_email, invite_unwrap_npub,
                    bootstrap_payload_hash, bootstrap_wrapped_event_json,
                    bootstrap_authorization_event_json, bootstrap_scope_json,
                    status, invite_code, accept_path, initial_folder_access_json,
                    created_by_npub, expires_at, created_at, updated_at
                )
                VALUES (
                    ?1, ?2, NULL, 'email_bootstrap', ?3, ?4,
                    ?5, ?6, ?7, ?8,
                    'pending', ?9, ?10, ?11, ?12, ?13, ?14, ?14
                )
                "#,
                params![
                    id,
                    brain_id.as_str(),
                    invited_email,
                    invite_unwrap_npub.as_str(),
                    bootstrap_payload_hash,
                    bootstrap_wrapped_event_json,
                    bootstrap_authorization_event_json,
                    bootstrap_scope_json,
                    invite_code,
                    accept_path,
                    initial_folder_access_json,
                    created_by_npub.as_str(),
                    expires_at,
                    created_at
                ],
            )
            .map_err(map_insert_error("brain_invitation_id", id))?;

        self.load_brain_invitation(id)
    }

    /// Load one Brain Invitation by id.
    pub fn load_brain_invitation(
        &self,
        invitation_id: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        self.conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE id = ?1"),
                params![invitation_id],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invitation",
            })
    }

    /// Load one Brain Invitation by invite code without applying recipient availability rules.
    pub fn load_brain_invitation_by_code(
        &self,
        invite_code: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        self.conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE invite_code = ?1"),
                params![invite_code],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invitation",
            })
    }

    /// List Brain Invitations for one Brain, newest first, bounded by MAX_LINK_LIST_ROWS.
    pub fn list_brain_invitations(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<StoredBrainInvitation>, StoreError> {
        self.require_brain_exists(brain_id)?;
        let query = format!(
            "{BRAIN_INVITATION_SELECT} WHERE brain_id = ?1 ORDER BY created_at DESC, id LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&query)?;
        let rows = stmt.query_map(
            params![brain_id.as_str(), MAX_LINK_LIST_ROWS],
            brain_invitation_from_row,
        )?;
        let mut invitations = Vec::new();
        for row in rows {
            invitations.push(row?);
        }
        Ok(invitations)
    }

    fn tombstone_email_bootstrap_ciphertext(
        &mut self,
        invitation_id: &str,
        updated_at: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            r#"
            UPDATE brain_invitations
            SET bootstrap_wrapped_event_json = NULL,
                updated_at = ?2
            WHERE id = ?1 AND target_kind = 'email_bootstrap'
            "#,
            params![invitation_id, updated_at],
        )?;
        Ok(())
    }

    /// Load a pending Brain Invitation by invite code for its target user only.
    pub fn load_available_brain_invitation_by_code(
        &self,
        invite_code: &str,
        user_id: &UserId,
        now: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let invitation = self
            .conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE invite_code = ?1"),
                params![invite_code],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invitation",
            })?;
        ensure_invitation_available(&invitation, user_id, now)?;
        Ok(invitation)
    }

    /// Revoke a Brain Invitation delivery handle. Accepted membership is unchanged.
    pub fn revoke_brain_invitation(
        &mut self,
        brain_id: &BrainId,
        invitation_id: &str,
        actor_npub: &UserId,
        updated_at: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let brain = self.load_core_brain(brain_id)?;
        if !brain.admins.contains(actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "brain invitation revocation requires a brain admin".to_owned(),
            });
        }
        let invitation = self.load_brain_invitation(invitation_id)?;
        if invitation.brain_id != *brain_id {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        self.conn.execute(
            r#"
            UPDATE brain_invitations
            SET status = 'revoked',
                bootstrap_wrapped_event_json = CASE
                    WHEN target_kind = 'email_bootstrap' THEN NULL
                    ELSE bootstrap_wrapped_event_json
                END,
                updated_at = ?3
            WHERE brain_id = ?1 AND id = ?2
            "#,
            params![brain_id.as_str(), invitation_id, updated_at],
        )?;
        self.load_brain_invitation(invitation_id)
    }

    /// Accept a pending Brain Invitation, adding the target as a member exactly once.
    pub fn accept_brain_invitation_by_code(
        &mut self,
        invite_code: &str,
        user_id: &UserId,
        now: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let mut invitation = self
            .conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE invite_code = ?1"),
                params![invite_code],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invitation",
            })?;

        if invitation.target_kind != BrainInvitationTargetKind::Npub
            || invitation.user_id.as_ref() != Some(user_id)
        {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        if invitation.status == LinkStatus::Accepted {
            invitation.duplicate_accept = true;
            return Ok(invitation);
        }
        ensure_invitation_available(&invitation, user_id, now)?;
        let already_member = self.member_exists(&invitation.brain_id, user_id)?;
        let brain = self.load_core_brain(&invitation.brain_id)?;
        let restricted_initial_folder_access = invitation
            .initial_folder_access
            .iter()
            .filter(|folder_id| {
                brain.folders.iter().any(|folder| {
                    folder.id == **folder_id && folder.access == FolderAccessMode::Restricted
                })
            })
            .cloned()
            .collect::<Vec<_>>();

        let tx = self.conn.transaction()?;
        insert_member_if_missing(&tx, &invitation.brain_id, user_id)?;
        for folder_id in restricted_initial_folder_access {
            insert_folder_access_if_missing(&tx, &invitation.brain_id, &folder_id, user_id)?;
        }
        tx.execute(
            r#"
            UPDATE brain_invitations
            SET status = 'accepted', updated_at = ?3, accepted_at = ?3
            WHERE brain_id = ?1 AND id = ?2 AND status = 'pending'
            "#,
            params![invitation.brain_id.as_str(), invitation.id, now],
        )?;
        tx.commit()?;

        let mut invitation = self.load_brain_invitation(&invitation.id)?;
        invitation.duplicate_accept = already_member;
        Ok(invitation)
    }

    /// Claim a pending Email Invite Bootstrap into durable npub-bound access.
    pub fn claim_email_brain_invitation_by_code(
        &mut self,
        invite_code: &str,
        invited_email: &str,
        claimant: &UserId,
        grants: &[FolderKeyGrantMetadata],
        now: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let mut invitation = self
            .conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE invite_code = ?1"),
                params![invite_code],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invitation",
            })?;

        if invitation.target_kind != BrainInvitationTargetKind::EmailBootstrap {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        if invitation.status == LinkStatus::Accepted {
            if invitation.claimed_by_npub.as_ref() == Some(claimant) {
                invitation.duplicate_accept = true;
                return Ok(invitation);
            }
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        if invitation.status != LinkStatus::Pending {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        if timestamp_expired(&invitation.expires_at, now) {
            self.tombstone_email_bootstrap_ciphertext(&invitation.id, now)?;
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        let invited_email = canonical_invited_email(invited_email)?;
        if invitation.invited_email.as_deref() != Some(invited_email.as_str()) {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }

        let stored = self.load_brain(&invitation.brain_id)?;
        if email_bootstrap_scope_stale(&stored.brain, &invitation.bootstrap_scope)? {
            self.tombstone_email_bootstrap_ciphertext(&invitation.id, now)?;
            return Err(StoreError::BrokenInvariant {
                reason: "email bootstrap scope is stale for current Folder Key versions".to_owned(),
            });
        }
        validate_email_claim_grants(&stored.brain, &invitation.bootstrap_scope, claimant, grants)?;
        let restricted_scope = invitation
            .bootstrap_scope
            .iter()
            .filter(|scope| scope.access == FolderAccessMode::Restricted)
            .map(|scope| scope.folder_id.clone())
            .collect::<Vec<_>>();

        let tx = self.conn.transaction()?;
        insert_member_if_missing(&tx, &invitation.brain_id, claimant)?;
        for folder_id in restricted_scope {
            insert_folder_access_if_missing(&tx, &invitation.brain_id, &folder_id, claimant)?;
        }
        for grant in grants {
            insert_grant(&tx, &invitation.brain_id, grant)?;
        }
        tx.execute(
            r#"
            UPDATE brain_invitations
            SET status = 'accepted',
                user_id = ?3,
                claimed_by_npub = ?3,
                bootstrap_wrapped_event_json = NULL,
                updated_at = ?4,
                accepted_at = ?4
            WHERE brain_id = ?1 AND id = ?2 AND status = 'pending'
            "#,
            params![
                invitation.brain_id.as_str(),
                invitation.id,
                claimant.as_str(),
                now
            ],
        )?;
        tx.commit()?;

        self.load_brain_invitation(&invitation.id)
    }

    /// Create one npub-bound singleton Share Link for a restricted Folder.
    #[allow(clippy::too_many_arguments)]
    pub fn create_share_link(
        &mut self,
        brain_id: &BrainId,
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
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "share links require brain operational authority".to_owned(),
            });
        }
        let folder = stored
            .brain
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
        validate_grant_issuer(
            &stored.brain,
            grant,
            stored
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
        )?;
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
                    id, brain_id, folder_id, recipient_npub, created_by_npub, status,
                    accept_path, expires_at, created_at, updated_at, grant_id,
                    grant_key_version, grant_wrapped_event_json, access_change_event_json,
                    create_personal_mount
                )
                VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, ?8, ?8, ?9, ?10, ?11, ?12, ?13)
                "#,
                params![
                    id,
                    brain_id.as_str(),
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
                SELECT id, brain_id, folder_id, recipient_npub, created_by_npub, status,
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
        brain_id: &BrainId,
        folder_id: &FolderId,
    ) -> Result<Vec<StoredShareLink>, StoreError> {
        ensure_folder_exists(&self.conn, brain_id, folder_id)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, brain_id, folder_id, recipient_npub, created_by_npub, status,
                   accept_path, expires_at, created_at, updated_at, accepted_at,
                   grant_id, grant_key_version, grant_wrapped_event_json,
                   access_change_event_json, create_personal_mount, personal_mount_id
            FROM share_links
            WHERE brain_id = ?1 AND folder_id = ?2
            ORDER BY created_at DESC, id
            LIMIT ?3
            "#,
        )?;
        let rows = stmt.query_map(
            params![brain_id.as_str(), folder_id.as_str(), MAX_LINK_LIST_ROWS],
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
        let stored = self.load_brain(&share_link.brain_id)?;
        if !has_brain_operational_authority(&stored, actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "share link revocation requires brain operational authority".to_owned(),
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

        let stored = self.load_brain(&share_link.brain_id)?;
        let folder = stored
            .brain
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
        validate_grant_issuer(
            &stored.brain,
            &share_link.folder_key_grant,
            stored
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
        )?;
        if share_link.folder_key_grant.key_version != folder.current_key_version {
            return Err(StoreError::BrokenInvariant {
                reason: "share link grant key version must match folder current key version"
                    .to_owned(),
            });
        }

        let tx = self.conn.transaction()?;
        insert_member_if_missing(&tx, &share_link.brain_id, recipient_npub)?;
        tx.execute(
            "INSERT INTO folder_access (brain_id, folder_id, user_id) VALUES (?1, ?2, ?3)",
            params![
                share_link.brain_id.as_str(),
                share_link.folder_id.as_str(),
                recipient_npub.as_str()
            ],
        )?;
        insert_grant(&tx, &share_link.brain_id, &share_link.folder_key_grant)?;

        let personal_mount_id = if share_link.create_personal_mount {
            let mount_id =
                personal_mount_id(recipient_npub, &share_link.brain_id, &share_link.folder_id);
            tx.execute(
                r#"
                INSERT INTO personal_folder_mounts (
                    id, owner_npub, source_brain_id, source_folder_id, display_name,
                    display_parent_folder_id, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?6)
                ON CONFLICT(owner_npub, source_brain_id, source_folder_id) DO UPDATE SET
                    updated_at = excluded.updated_at
                "#,
                params![
                    mount_id,
                    recipient_npub.as_str(),
                    share_link.brain_id.as_str(),
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
