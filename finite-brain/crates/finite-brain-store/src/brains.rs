use crate::*;

impl BrainStore {
    pub fn create_personal_brain_bootstrap(
        &mut self,
        output: &BootstrapOutput,
        grants: &[FolderKeyGrantMetadata],
        agent_npub: &UserId,
        created_by_npub: &UserId,
        created_at: &str,
    ) -> Result<(), StoreError> {
        self.create_personal_brain_bootstrap_with_identities(
            output,
            grants,
            agent_npub,
            created_by_npub,
            created_at,
            &[],
        )
    }

    /// Atomically create a Personal Brain, its Personal Agent, and both verified display aliases.
    pub fn create_personal_brain_bootstrap_with_identities(
        &mut self,
        output: &BootstrapOutput,
        grants: &[FolderKeyGrantMetadata],
        agent_npub: &UserId,
        created_by_npub: &UserId,
        created_at: &str,
        identity_aliases: &[IdentityAlias],
    ) -> Result<(), StoreError> {
        validate_bootstrap_output(output)?;
        validate_required_grants(&output.brain, &output.required_key_grants, grants)?;
        if output.brain.kind != BrainKind::Personal {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Agent bootstrap requires a personal brain".to_owned(),
            });
        }
        let owner_npub =
            output
                .brain
                .owner_user_id
                .as_ref()
                .ok_or_else(|| StoreError::BrokenInvariant {
                    reason: "Personal Agent bootstrap requires a brain owner".to_owned(),
                })?;
        if owner_npub == agent_npub {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Agent must use a distinct Agent Principal".to_owned(),
            });
        }
        if created_by_npub != owner_npub && created_by_npub != agent_npub {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Agent bootstrap actor must be the owner or agent".to_owned(),
            });
        }
        if !identity_aliases.is_empty() {
            let alias_npubs = identity_aliases
                .iter()
                .map(|alias| alias.npub.clone())
                .collect::<BTreeSet<_>>();
            let alias_emails = identity_aliases
                .iter()
                .filter_map(|alias| alias.preferred_nip05.clone())
                .collect::<BTreeSet<_>>();
            if identity_aliases.len() != 2
                || alias_npubs != BTreeSet::from([owner_npub.clone(), agent_npub.clone()])
                || alias_emails.len() != 2
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "Personal Agent bootstrap identities must name the owner and agent with verified emails".to_owned(),
                });
            }
        }

        // Serialize Personal Brain creation before checking the one-owner invariant. The partial
        // unique index remains the final database guard for every writer.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_brain_id = tx
            .query_row(
                "SELECT id FROM brains WHERE kind = 'personal' AND owner_user_id = ?1",
                params![owner_npub.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(existing_brain_id) = existing_brain_id {
            if existing_brain_id != output.brain.id.as_str() {
                return Err(StoreError::BrokenInvariant {
                    reason: "user already has a personal brain".to_owned(),
                });
            }
            let existing_agent = tx
                .query_row(
                    "SELECT agent_npub FROM personal_agents WHERE brain_id = ?1 AND status = 'active'",
                    params![output.brain.id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            return match existing_agent {
                Some(existing_agent) if existing_agent == agent_npub.as_str() => Ok(()),
                Some(_) => Err(StoreError::BrokenInvariant {
                    reason: "personal brain already has a different personal agent".to_owned(),
                }),
                None => Err(StoreError::BrokenInvariant {
                    reason: "personal brain already exists without a personal agent".to_owned(),
                }),
            };
        }

        let audit_id = format!("{}-personal-agent-established", output.brain.id);
        insert_brain(&tx, &output.brain)?;
        insert_members_and_admins(&tx, &output.brain)?;
        for folder in &output.brain.folders {
            insert_folder(&tx, &output.brain.id, folder, false)?;
        }
        for grant in grants {
            insert_grant(&tx, &output.brain.id, grant)?;
        }
        for alias in identity_aliases {
            upsert_identity_alias(&tx, alias)?;
        }
        tx.execute(
            r#"
            INSERT INTO personal_agents (
                brain_id, owner_npub, agent_npub, status, created_by_npub,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, 'active', ?4, ?5, ?5)
            "#,
            params![
                output.brain.id.as_str(),
                owner_npub.as_str(),
                agent_npub.as_str(),
                created_by_npub.as_str(),
                created_at,
            ],
        )?;
        tx.execute(
            r#"
            INSERT INTO personal_agent_audit (
                id, brain_id, action, actor_npub, previous_agent_npub,
                agent_npub, occurred_at
            ) VALUES (?1, ?2, 'established', ?3, NULL, ?4, ?5)
            "#,
            params![
                audit_id,
                output.brain.id.as_str(),
                created_by_npub.as_str(),
                agent_npub.as_str(),
                created_at,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn create_brain_bootstrap(
        &mut self,
        output: &BootstrapOutput,
        grants: &[FolderKeyGrantMetadata],
    ) -> Result<(), StoreError> {
        if output.brain.folders.len() > MAX_BOOTSTRAP_FOLDERS {
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
        validate_required_grants(&output.brain, &output.required_key_grants, grants)?;
        if output.brain.kind == BrainKind::Personal {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain bootstrap requires a Personal Agent".to_owned(),
            });
        }

        let tx = self.conn.transaction()?;
        insert_brain(&tx, &output.brain)?;
        insert_members_and_admins(&tx, &output.brain)?;
        for folder in &output.brain.folders {
            insert_folder(&tx, &output.brain.id, folder, false)?;
        }
        for grant in grants {
            insert_grant(&tx, &output.brain.id, grant)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_visible_brains(&self, actor: &UserId) -> Result<Vec<VisibleBrain>, StoreError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, kind, name, role, invite_code
            FROM (
                SELECT v.id, v.kind, v.name,
                       CASE
                           WHEN v.owner_user_id = ?1 THEN 'owner'
                           WHEN pa.agent_npub = ?1 THEN 'personal_agent'
                           WHEN va.user_id IS NOT NULL THEN 'admin'
                           ELSE 'member'
                       END AS role,
                       NULL AS invite_code
                FROM brains v
                LEFT JOIN brain_admins va
                  ON va.brain_id = v.id AND va.user_id = ?1
                LEFT JOIN personal_agents pa
                  ON pa.brain_id = v.id AND pa.agent_npub = ?1 AND pa.status = 'active'
                LEFT JOIN brain_members vm
                  ON vm.brain_id = v.id AND vm.user_id = ?1
                WHERE v.owner_user_id = ?1
                   OR pa.agent_npub = ?1
                   OR (
                       vm.user_id IS NOT NULL
                       AND (
                           v.kind = 'organization'
                           OR EXISTS (
                               SELECT 1
                               FROM folder_access fa
                               WHERE fa.brain_id = v.id AND fa.user_id = ?1
                           )
                       )
                   )

                UNION ALL

                SELECT v.id, v.kind, v.name, 'invited' AS role, vi.invite_code
                FROM brain_invitations vi
                JOIN brains v
                  ON v.id = vi.brain_id
                LEFT JOIN brain_members vm
                  ON vm.brain_id = v.id AND vm.user_id = ?1
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

        let mut brains = Vec::new();
        for row in rows {
            let (id, kind, name, role, invite_code) = row?;
            brains.push(VisibleBrain {
                id: BrainId::new(id)?,
                kind: parse_brain_kind(&kind)?,
                name,
                role: match role.as_str() {
                    "owner" => VisibleBrainRole::Owner,
                    "personal_agent" => VisibleBrainRole::PersonalAgent,
                    "admin" => VisibleBrainRole::Admin,
                    "member" => VisibleBrainRole::Member,
                    "invited" => VisibleBrainRole::Invited,
                    _ => {
                        return Err(StoreError::BrokenInvariant {
                            reason: format!("unknown visible brain role: {role}"),
                        });
                    }
                },
                invite_code,
            });
        }
        Ok(brains)
    }

    /// Add an organization Brain Member.
    pub fn add_member(&mut self, brain_id: &BrainId, user_id: &UserId) -> Result<(), StoreError> {
        self.require_organization_brain(brain_id)?;
        self.conn.execute(
            "INSERT INTO brain_members (brain_id, user_id) VALUES (?1, ?2)",
            params![brain_id.as_str(), user_id.as_str()],
        )?;
        Ok(())
    }

    /// Add an organization Brain Admin. The user must already be a member.
    pub fn add_admin(&mut self, brain_id: &BrainId, user_id: &UserId) -> Result<(), StoreError> {
        self.require_organization_brain(brain_id)?;
        if !self.member_exists(brain_id, user_id)? {
            return Err(StoreError::BrokenInvariant {
                reason: "brain admin must already be a brain member".to_owned(),
            });
        }
        self.conn.execute(
            "INSERT INTO brain_admins (brain_id, user_id) VALUES (?1, ?2)",
            params![brain_id.as_str(), user_id.as_str()],
        )?;
        Ok(())
    }

    /// Remove an organization Brain Admin while preserving at least one admin.
    pub fn remove_admin(&mut self, brain_id: &BrainId, user_id: &UserId) -> Result<(), StoreError> {
        let brain = self.load_core_brain(brain_id)?;
        if brain.kind != BrainKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "member/admin mutation requires an organization brain".to_owned(),
            });
        }
        if !brain.admins.contains(user_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "brain admin does not exist".to_owned(),
            });
        }
        if brain.admins.len() == 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "organization brain must keep at least one admin".to_owned(),
            });
        }

        self.conn.execute(
            "DELETE FROM brain_admins WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), user_id.as_str()],
        )?;
        Ok(())
    }

    /// Remove an organization Brain Member after admin and restricted access cleanup.
    pub fn remove_member(
        &mut self,
        brain_id: &BrainId,
        user_id: &UserId,
    ) -> Result<(), StoreError> {
        let brain = self.load_core_brain(brain_id)?;
        if brain.kind != BrainKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "member/admin mutation requires an organization brain".to_owned(),
            });
        }
        if brain.admins.contains(user_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "remove admin role before removing member".to_owned(),
            });
        }
        if !brain
            .members
            .iter()
            .any(|member| &member.user_id == user_id)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "brain member does not exist".to_owned(),
            });
        }
        if self.member_has_restricted_access(brain_id, user_id)? {
            return Err(StoreError::BrokenInvariant {
                reason: "remove restricted folder access before removing member".to_owned(),
            });
        }

        self.conn.execute(
            "DELETE FROM brain_members WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), user_id.as_str()],
        )?;
        Ok(())
    }
}
