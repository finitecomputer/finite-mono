use crate::links::invitation_pending_wrap_folders;
use crate::*;

const BRAIN_INVITE_TOKEN_SELECT: &str = r#"
    SELECT token_hash, brain_id, role, inviter_npub, created_at, expires_at,
           redeemed_by_npub, redeemed_at, revoked_at
    FROM brain_invite_tokens
"#;

impl BrainStore {
    /// Create one capability Invite Token. The store only ever sees the
    /// SHA-256 hash of the raw token; the raw token is a server-side concern.
    pub fn create_brain_invite_token(
        &mut self,
        brain_id: &BrainId,
        token_hash: &str,
        role: BrainInviteTokenRole,
        inviter_npub: &UserId,
        expires_at: &str,
        created_at: &str,
    ) -> Result<StoredBrainInviteToken, StoreError> {
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, inviter_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "brain invite tokens require brain operational authority".to_owned(),
            });
        }
        if role == BrainInviteTokenRole::Admin && stored.brain.kind != BrainKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "admin invite tokens require an organization brain".to_owned(),
            });
        }
        validate_invite_token_hash(token_hash)?;
        validate_bounded_offer_expiry(expires_at, created_at)?;

        self.conn
            .execute(
                r#"
                INSERT INTO brain_invite_tokens (
                    token_hash, brain_id, role, inviter_npub, created_at, expires_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    token_hash,
                    brain_id.as_str(),
                    role.as_str(),
                    inviter_npub.as_str(),
                    created_at,
                    expires_at
                ],
            )
            .map_err(map_insert_error("brain_invite_token_hash", token_hash))?;

        self.load_brain_invite_token(token_hash)
    }

    /// Load one Invite Token by hash.
    pub fn load_brain_invite_token(
        &self,
        token_hash: &str,
    ) -> Result<StoredBrainInviteToken, StoreError> {
        self.conn
            .query_row(
                &format!("{BRAIN_INVITE_TOKEN_SELECT} WHERE token_hash = ?1"),
                params![token_hash],
                brain_invite_token_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invite token",
            })
    }

    /// List Invite Tokens for one Brain, newest first, bounded by
    /// MAX_LINK_LIST_ROWS. Rows never carry raw token material.
    pub fn list_brain_invite_tokens(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<StoredBrainInviteToken>, StoreError> {
        self.require_brain_exists(brain_id)?;
        let query = format!(
            "{BRAIN_INVITE_TOKEN_SELECT} WHERE brain_id = ?1 ORDER BY created_at DESC, token_hash LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&query)?;
        let rows = stmt.query_map(
            params![brain_id.as_str(), MAX_LINK_LIST_ROWS],
            brain_invite_token_from_row,
        )?;
        let mut tokens = Vec::new();
        for row in rows {
            tokens.push(row?);
        }
        Ok(tokens)
    }

    /// Revoke a pending Invite Token. Redeemed membership is unchanged.
    pub fn revoke_brain_invite_token(
        &mut self,
        brain_id: &BrainId,
        token_hash: &str,
        actor_npub: &UserId,
        now: &str,
    ) -> Result<StoredBrainInviteToken, StoreError> {
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "brain invite token revocation requires brain operational authority"
                    .to_owned(),
            });
        }
        let token = self.load_brain_invite_token(token_hash)?;
        if token.brain_id != *brain_id
            || token.redeemed_by_npub.is_some()
            || token.revoked_at.is_some()
        {
            return Err(StoreError::UnavailableLink {
                kind: "brain invite token",
            });
        }
        self.conn.execute(
            r#"
            UPDATE brain_invite_tokens
            SET revoked_at = ?2
            WHERE token_hash = ?1 AND redeemed_by_npub IS NULL AND revoked_at IS NULL
            "#,
            params![token_hash, now],
        )?;
        self.load_brain_invite_token(token_hash)
    }

    /// Redeem one capability Invite Token: the presenting npub gains Brain
    /// Membership (with inviter provenance) and, for an admin-role token,
    /// Brain Admin standing. Folder Key delivery stays with key-holding
    /// clients through pending-wrap markers, exactly as on npub-target
    /// invitation accept. Single-use, expiry, and revocation fail closed;
    /// re-presenting a consumed token with the same npub returns the current
    /// state instead of an error.
    pub fn redeem_brain_invite_token(
        &mut self,
        token_hash: &str,
        redeemer: &UserId,
        now: &str,
    ) -> Result<StoredBrainInviteToken, StoreError> {
        let token = self.load_brain_invite_token(token_hash)?;
        if token.revoked_at.is_some() {
            return Err(StoreError::UnavailableLink {
                kind: "brain invite token",
            });
        }
        if let Some(redeemed_by) = token.redeemed_by_npub.as_ref() {
            if redeemed_by == redeemer {
                let mut token = token;
                token.duplicate_redeem = true;
                return Ok(token);
            }
            return Err(StoreError::UnavailableLink {
                kind: "brain invite token",
            });
        }
        if timestamp_expired(&token.expires_at, now) {
            return Err(StoreError::UnavailableLink {
                kind: "brain invite token",
            });
        }

        let already_member = self.member_exists(&token.brain_id, redeemer)?;
        let member_provenance = MemberProvenance::invitation(
            token.inviter_npub.clone(),
            format!("invite-token:{}", token.token_hash),
        );
        let brain = self.load_core_brain(&token.brain_id)?;
        // Member-role tokens follow npub-invitation accept semantics exactly:
        // wrap markers for every All-Members Folder. An admin-role token
        // redeems to Brain Admin standing, which is entitled to every Folder,
        // so every Folder gets a marker.
        let pending_wrap_folders = match token.role {
            BrainInviteTokenRole::Member => invitation_pending_wrap_folders(&brain, &[])
                .into_iter()
                .map(|folder| (folder.id.clone(), folder.current_key_version))
                .collect::<Vec<_>>(),
            BrainInviteTokenRole::Admin => brain
                .folders
                .iter()
                .map(|folder| (folder.id.clone(), folder.current_key_version))
                .collect::<Vec<_>>(),
        };

        let tx = self.conn.transaction()?;
        insert_member_with_provenance_if_missing(
            &tx,
            &token.brain_id,
            redeemer,
            &member_provenance,
        )?;
        if token.role == BrainInviteTokenRole::Admin {
            tx.execute(
                "INSERT OR IGNORE INTO brain_admins (brain_id, user_id) VALUES (?1, ?2)",
                params![token.brain_id.as_str(), redeemer.as_str()],
            )?;
        }
        for (folder_id, key_version) in pending_wrap_folders {
            pending_wraps::mark_pending_grant_wrap(
                &tx,
                &token.brain_id,
                &folder_id,
                redeemer,
                key_version,
                PendingGrantWrapReason::Accept,
                now,
            )?;
        }
        let updated = tx.execute(
            r#"
            UPDATE brain_invite_tokens
            SET redeemed_by_npub = ?2, redeemed_at = ?3
            WHERE token_hash = ?1 AND redeemed_by_npub IS NULL AND revoked_at IS NULL
            "#,
            params![token_hash, redeemer.as_str(), now],
        )?;
        if updated != 1 {
            // A concurrent redeem won the single-use race; fail closed.
            return Err(StoreError::UnavailableLink {
                kind: "brain invite token",
            });
        }
        tx.commit()?;

        let mut token = self.load_brain_invite_token(token_hash)?;
        token.duplicate_redeem = already_member;
        Ok(token)
    }
}

fn brain_invite_token_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<StoredBrainInviteToken, rusqlite::Error> {
    let role: String = row.get(2)?;
    let inviter_npub: String = row.get(3)?;
    let redeemed_by_npub: Option<String> = row.get(6)?;
    Ok(StoredBrainInviteToken {
        token_hash: row.get(0)?,
        brain_id: BrainId::new(row.get::<_, String>(1)?)
            .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
        role: BrainInviteTokenRole::try_from(role.as_str())
            .map_err(to_store_from_sql_error(2, rusqlite::types::Type::Text))?,
        inviter_npub: UserId::new(inviter_npub)
            .map_err(to_from_sql_error(3, rusqlite::types::Type::Text))?,
        created_at: row.get(4)?,
        expires_at: row.get(5)?,
        redeemed_by_npub: redeemed_by_npub
            .map(UserId::new)
            .transpose()
            .map_err(to_from_sql_error(6, rusqlite::types::Type::Text))?,
        redeemed_at: row.get(7)?,
        revoked_at: row.get(8)?,
        duplicate_redeem: false,
    })
}

fn validate_invite_token_hash(token_hash: &str) -> Result<(), StoreError> {
    let valid = token_hash.len() == 64
        && token_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !valid {
        return Err(StoreError::BrokenInvariant {
            reason: "brain invite token hash must be 64 lowercase hex characters".to_owned(),
        });
    }
    Ok(())
}
