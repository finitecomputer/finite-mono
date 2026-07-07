use crate::*;

impl BrainStore {
    pub fn create_vault_bootstrap(
        &mut self,
        output: &BootstrapOutput,
        grants: &[FolderKeyGrantMetadata],
    ) -> Result<(), StoreError> {
        if output.vault.folders.len() > MAX_BOOTSTRAP_FOLDERS {
            return Err(StoreError::BrokenInvariant {
                reason: format!("bootstrap folder count exceeds limit {MAX_BOOTSTRAP_FOLDERS}"),
            });
        }
        if grants.len() > MAX_BOOTSTRAP_GRANTS {
            return Err(StoreError::BrokenInvariant {
                reason: format!("bootstrap grant count exceeds limit {MAX_BOOTSTRAP_GRANTS}"),
            });
        }
        validate_bootstrap_output(output)?;
        validate_required_grants(&output.vault, &output.required_key_grants, grants)?;
        if let (VaultKind::Personal, Some(owner)) =
            (output.vault.kind, output.vault.owner_user_id.as_ref())
        {
            self.ensure_personal_vault_available(owner)?;
        }

        let tx = self.conn.transaction()?;
        insert_vault(&tx, &output.vault)?;
        insert_members_and_admins(&tx, &output.vault)?;
        for folder in &output.vault.folders {
            insert_folder(&tx, &output.vault.id, folder, false)?;
        }
        for grant in grants {
            insert_grant(&tx, &output.vault.id, grant)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_visible_vaults(&self, actor: &UserId) -> Result<Vec<VisibleVault>, StoreError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, kind, name, role, invite_code
            FROM (
                SELECT v.id, v.kind, v.name,
                       CASE
                           WHEN v.owner_user_id = ?1 THEN 'owner'
                           WHEN va.user_id IS NOT NULL THEN 'admin'
                           ELSE 'member'
                       END AS role,
                       NULL AS invite_code
                FROM vaults v
                LEFT JOIN vault_admins va
                  ON va.vault_id = v.id AND va.user_id = ?1
                LEFT JOIN vault_members vm
                  ON vm.vault_id = v.id AND vm.user_id = ?1
                WHERE v.owner_user_id = ?1 OR vm.user_id IS NOT NULL

                UNION ALL

                SELECT v.id, v.kind, v.name, 'invited' AS role, vi.invite_code
                FROM vault_invitations vi
                JOIN vaults v
                  ON v.id = vi.vault_id
                LEFT JOIN vault_members vm
                  ON vm.vault_id = v.id AND vm.user_id = ?1
                WHERE vi.user_id = ?1
                  AND vi.status = 'pending'
                  AND vi.expires_at > ?2
                  AND v.owner_user_id IS NULL
                  AND vm.user_id IS NULL
            )
            ORDER BY
              CASE kind WHEN 'personal' THEN 0 ELSE 1 END,
              lower(name),
              id
            "#,
        )?;
        let now = current_timestamp();
        let rows = stmt.query_map(params![actor.as_str(), now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;

        let mut vaults = Vec::new();
        for row in rows {
            let (id, kind, name, role, invite_code) = row?;
            vaults.push(VisibleVault {
                id: VaultId::new(id)?,
                kind: parse_vault_kind(&kind)?,
                name,
                role: match role.as_str() {
                    "owner" => VisibleVaultRole::Owner,
                    "admin" => VisibleVaultRole::Admin,
                    "member" => VisibleVaultRole::Member,
                    "invited" => VisibleVaultRole::Invited,
                    _ => {
                        return Err(StoreError::BrokenInvariant {
                            reason: format!("unknown visible vault role: {role}"),
                        });
                    }
                },
                invite_code,
            });
        }
        Ok(vaults)
    }

    /// Add an organization Vault Member.
    pub fn add_member(&mut self, vault_id: &VaultId, user_id: &UserId) -> Result<(), StoreError> {
        self.require_organization_vault(vault_id)?;
        self.conn.execute(
            "INSERT INTO vault_members (vault_id, user_id) VALUES (?1, ?2)",
            params![vault_id.as_str(), user_id.as_str()],
        )?;
        Ok(())
    }

    /// Add an organization Vault Admin. The user must already be a member.
    pub fn add_admin(&mut self, vault_id: &VaultId, user_id: &UserId) -> Result<(), StoreError> {
        self.require_organization_vault(vault_id)?;
        if !self.member_exists(vault_id, user_id)? {
            return Err(StoreError::BrokenInvariant {
                reason: "vault admin must already be a vault member".to_owned(),
            });
        }
        self.conn.execute(
            "INSERT INTO vault_admins (vault_id, user_id) VALUES (?1, ?2)",
            params![vault_id.as_str(), user_id.as_str()],
        )?;
        Ok(())
    }

    /// Remove an organization Vault Admin while preserving at least one admin.
    pub fn remove_admin(&mut self, vault_id: &VaultId, user_id: &UserId) -> Result<(), StoreError> {
        let vault = self.load_core_vault(vault_id)?;
        if vault.kind != VaultKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "member/admin mutation requires an organization vault".to_owned(),
            });
        }
        if !vault.admins.contains(user_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "vault admin does not exist".to_owned(),
            });
        }
        if vault.admins.len() == 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "organization vault must keep at least one admin".to_owned(),
            });
        }

        self.conn.execute(
            "DELETE FROM vault_admins WHERE vault_id = ?1 AND user_id = ?2",
            params![vault_id.as_str(), user_id.as_str()],
        )?;
        Ok(())
    }

    /// Remove an organization Vault Member after admin and restricted access cleanup.
    pub fn remove_member(
        &mut self,
        vault_id: &VaultId,
        user_id: &UserId,
    ) -> Result<(), StoreError> {
        let vault = self.load_core_vault(vault_id)?;
        if vault.kind != VaultKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "member/admin mutation requires an organization vault".to_owned(),
            });
        }
        if vault.admins.contains(user_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "remove admin role before removing member".to_owned(),
            });
        }
        if !vault
            .members
            .iter()
            .any(|member| &member.user_id == user_id)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "vault member does not exist".to_owned(),
            });
        }
        if self.member_has_restricted_access(vault_id, user_id)? {
            return Err(StoreError::BrokenInvariant {
                reason: "remove restricted folder access before removing member".to_owned(),
            });
        }

        self.conn.execute(
            "DELETE FROM vault_members WHERE vault_id = ?1 AND user_id = ?2",
            params![vault_id.as_str(), user_id.as_str()],
        )?;
        Ok(())
    }

    fn ensure_personal_vault_available(&self, owner: &UserId) -> Result<(), StoreError> {
        let exists = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM vaults WHERE kind = 'personal' AND owner_user_id = ?1)",
            params![owner.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        if exists {
            return Err(StoreError::BrokenInvariant {
                reason: "user already has a personal vault".to_owned(),
            });
        }
        Ok(())
    }
}
