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
        self.create_personal_brain_bootstrap_with_identities_and_cohort(
            output,
            grants,
            agent_npub,
            created_by_npub,
            created_at,
            identity_aliases,
            None,
        )
    }

    /// Atomically create the Personal Brain with the complete current account
    /// agent set while retaining the first agent as the legacy singular view.
    pub fn create_personal_brain_cohort_bootstrap_with_identities(
        &mut self,
        output: &BootstrapOutput,
        grants: &[FolderKeyGrantMetadata],
        created_by_npub: &UserId,
        created_at: &str,
        identity_aliases: &[IdentityAlias],
        cohort: &BootstrapAccountCohort,
    ) -> Result<(), StoreError> {
        let primary_agent = cohort
            .participants
            .iter()
            .find(|participant| participant.relationship == "account_agent")
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "Personal Brain cohort bootstrap requires at least one ready agent"
                    .to_owned(),
            })?;
        self.create_personal_brain_bootstrap_with_identities_and_cohort(
            output,
            grants,
            &primary_agent.npub,
            created_by_npub,
            created_at,
            identity_aliases,
            Some(cohort),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_personal_brain_bootstrap_with_identities_and_cohort(
        &mut self,
        output: &BootstrapOutput,
        grants: &[FolderKeyGrantMetadata],
        agent_npub: &UserId,
        created_by_npub: &UserId,
        created_at: &str,
        identity_aliases: &[IdentityAlias],
        cohort: Option<&BootstrapAccountCohort>,
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
        if !identity_aliases.is_empty() && cohort.is_none() {
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
        if let Some(cohort) = cohort {
            insert_bootstrap_account_cohort(
                &tx,
                &output.brain,
                cohort,
                "personal_bootstrap",
                created_at,
            )?;
            for participant in cohort
                .participants
                .iter()
                .filter(|participant| participant.relationship == "account_agent")
            {
                tx.execute(
                    r#"
                    INSERT INTO personal_brain_agents (
                        brain_id, agent_npub, agent_nip05, display_name, status,
                        roster_revision, created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, 'ready', ?5, ?6, ?6)
                    "#,
                    params![
                        output.brain.id.as_str(),
                        participant.npub.as_str(),
                        participant.nip05,
                        participant.name,
                        i64::try_from(cohort.roster_revision).map_err(|_| {
                            StoreError::BrokenInvariant {
                                reason: "roster revision exceeds SQLite integer range".to_owned(),
                            }
                        })?,
                        created_at,
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn create_brain_bootstrap(
        &mut self,
        output: &BootstrapOutput,
        grants: &[FolderKeyGrantMetadata],
    ) -> Result<(), StoreError> {
        self.create_brain_bootstrap_with_identities(output, grants, &[])
    }

    /// Atomically create an Organization Brain and its verified member aliases.
    pub fn create_brain_bootstrap_with_identities(
        &mut self,
        output: &BootstrapOutput,
        grants: &[FolderKeyGrantMetadata],
        identity_aliases: &[IdentityAlias],
    ) -> Result<(), StoreError> {
        self.create_brain_bootstrap_with_identities_and_cohort(
            output,
            grants,
            identity_aliases,
            None,
            &current_timestamp(),
        )
    }

    /// Atomically bootstrap an Organization Brain with one human admin and
    /// every current eligible account agent as a non-admin Member.
    pub fn create_organization_brain_cohort_bootstrap_with_identities(
        &mut self,
        output: &BootstrapOutput,
        grants: &[FolderKeyGrantMetadata],
        identity_aliases: &[IdentityAlias],
        cohort: &BootstrapAccountCohort,
        created_at: &str,
    ) -> Result<(), StoreError> {
        self.create_brain_bootstrap_with_identities_and_cohort(
            output,
            grants,
            identity_aliases,
            Some(cohort),
            created_at,
        )
    }

    fn create_brain_bootstrap_with_identities_and_cohort(
        &mut self,
        output: &BootstrapOutput,
        grants: &[FolderKeyGrantMetadata],
        identity_aliases: &[IdentityAlias],
        cohort: Option<&BootstrapAccountCohort>,
        created_at: &str,
    ) -> Result<(), StoreError> {
        if output.brain.folders.len() > MAX_BOOTSTRAP_FOLDERS {
            return Err(StoreError::CapacityExceeded {
                limit: "brain_folders".to_owned(),
                max: MAX_BOOTSTRAP_FOLDERS,
                current: output.brain.folders.len(),
            });
        }
        if grants.len() > MAX_BOOTSTRAP_GRANTS {
            return Err(StoreError::CapacityExceeded {
                limit: "folder_key_grants".to_owned(),
                max: MAX_BOOTSTRAP_GRANTS,
                current: grants.len(),
            });
        }
        validate_bootstrap_output(output)?;
        validate_required_grants(&output.brain, &output.required_key_grants, grants)?;
        if output.brain.kind == BrainKind::Personal {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain bootstrap requires a Personal Agent".to_owned(),
            });
        }
        if identity_aliases.iter().any(|alias| {
            !output
                .brain
                .members
                .iter()
                .any(|member| member.user_id == alias.npub)
        }) {
            return Err(StoreError::BrokenInvariant {
                reason: "Organization Brain bootstrap identities must belong to initial members"
                    .to_owned(),
            });
        }

        match self.load_brain(&output.brain.id) {
            Ok(existing)
                if equivalent_bootstrap_brain(&existing.brain, &output.brain)
                    && existing.grants == grants =>
            {
                return Ok(());
            }
            Ok(_) => {
                return Err(StoreError::DuplicateId {
                    field: "brain_id",
                    value: output.brain.id.to_string(),
                });
            }
            Err(StoreError::MissingBrain { .. }) => {}
            Err(error) => return Err(error),
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
        for alias in identity_aliases {
            upsert_identity_alias(&tx, alias)?;
        }
        if let Some(cohort) = cohort {
            insert_bootstrap_account_cohort(
                &tx,
                &output.brain,
                cohort,
                "organization_bootstrap",
                created_at,
            )?;
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
                           WHEN pa.agent_npub = ?1 OR pba.agent_npub = ?1 THEN 'personal_agent'
                           WHEN va.user_id IS NOT NULL THEN 'admin'
                           WHEN vm.user_id IS NOT NULL THEN 'member'
                           ELSE 'guest'
                       END AS role,
                       NULL AS invite_code
                FROM brains v
                LEFT JOIN brain_admins va
                  ON va.brain_id = v.id AND va.user_id = ?1
                LEFT JOIN personal_agents pa
                  ON pa.brain_id = v.id AND pa.agent_npub = ?1 AND pa.status = 'active'
                 AND NOT EXISTS (
                     SELECT 1 FROM account_access_cohort_exclusions exclusion
                     JOIN account_access_cohorts cohort ON cohort.id = exclusion.cohort_id
                     WHERE cohort.brain_id = v.id
                       AND exclusion.participant_npub = ?1
                       AND exclusion.folder_id = '' AND exclusion.active = 1
                 )
                LEFT JOIN personal_brain_agents pba
                  ON pba.brain_id = v.id AND pba.agent_npub = ?1 AND pba.status = 'ready'
                 AND NOT EXISTS (
                     SELECT 1 FROM account_access_cohort_exclusions exclusion
                     JOIN account_access_cohorts cohort ON cohort.id = exclusion.cohort_id
                     WHERE cohort.brain_id = v.id
                       AND exclusion.participant_npub = ?1
                       AND exclusion.folder_id = '' AND exclusion.active = 1
                 )
                LEFT JOIN brain_members vm
                  ON vm.brain_id = v.id AND vm.user_id = ?1
                WHERE v.owner_user_id = ?1
                   OR pa.agent_npub = ?1
                   OR pba.agent_npub = ?1
                   OR vm.user_id IS NOT NULL
                   OR EXISTS (
                       SELECT 1
                       FROM folder_access fa
                       WHERE fa.brain_id = v.id AND fa.user_id = ?1
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
                    "guest" => VisibleBrainRole::Guest,
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

    /// Add a Member to either Brain kind.
    pub fn add_member(&mut self, brain_id: &BrainId, user_id: &UserId) -> Result<(), StoreError> {
        self.add_member_with_control_records(brain_id, user_id, &[])
    }

    /// Add a Member and append its signed administrative record atomically.
    pub fn add_member_with_control_records(
        &mut self,
        brain_id: &BrainId,
        user_id: &UserId,
        control_records: &[SyncRecordInput],
    ) -> Result<(), StoreError> {
        self.load_core_brain(brain_id)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO brain_members (brain_id, user_id) VALUES (?1, ?2)",
            params![brain_id.as_str(), user_id.as_str()],
        )?;
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.commit()?;
        Ok(())
    }

    /// Add an Organization Brain Admin. The user must already be a member.
    pub fn add_admin(&mut self, brain_id: &BrainId, user_id: &UserId) -> Result<(), StoreError> {
        self.add_admin_with_control_records(brain_id, user_id, &[])
    }

    /// Add an Organization Brain Admin and append its signed record atomically.
    pub fn add_admin_with_control_records(
        &mut self,
        brain_id: &BrainId,
        user_id: &UserId,
        control_records: &[SyncRecordInput],
    ) -> Result<(), StoreError> {
        self.require_organization_brain(brain_id)?;
        if !self.member_exists(brain_id, user_id)? {
            return Err(StoreError::BrokenInvariant {
                reason: "brain admin must already be a brain member".to_owned(),
            });
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO brain_admins (brain_id, user_id) VALUES (?1, ?2)",
            params![brain_id.as_str(), user_id.as_str()],
        )?;
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.commit()?;
        Ok(())
    }

    /// Remove an Organization Brain Admin while preserving at least one admin.
    pub fn remove_admin(&mut self, brain_id: &BrainId, user_id: &UserId) -> Result<(), StoreError> {
        self.remove_admin_with_control_records(brain_id, user_id, &[])
    }

    /// Remove an Organization Brain Admin and append its signed record atomically.
    pub fn remove_admin_with_control_records(
        &mut self,
        brain_id: &BrainId,
        user_id: &UserId,
        control_records: &[SyncRecordInput],
    ) -> Result<(), StoreError> {
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

        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM brain_admins WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), user_id.as_str()],
        )?;
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.commit()?;
        Ok(())
    }

    /// Remove a Brain Member after role and restricted access cleanup.
    pub fn remove_member(
        &mut self,
        brain_id: &BrainId,
        user_id: &UserId,
    ) -> Result<(), StoreError> {
        let brain = self.load_core_brain(brain_id)?;
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
                reason: "remove explicit Folder access before removing member".to_owned(),
            });
        }

        self.conn.execute(
            "DELETE FROM brain_members WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), user_id.as_str()],
        )?;
        Ok(())
    }
}

fn insert_bootstrap_account_cohort(
    tx: &Transaction<'_>,
    brain: &Brain,
    cohort: &BootstrapAccountCohort,
    provenance_kind: &str,
    created_at: &str,
) -> Result<(), StoreError> {
    let human = cohort
        .participants
        .iter()
        .filter(|participant| participant.relationship == "human")
        .collect::<Vec<_>>();
    let participant_npubs = cohort
        .participants
        .iter()
        .map(|participant| participant.npub.clone())
        .collect::<BTreeSet<_>>();
    if human.len() != 1
        || participant_npubs.len() != cohort.participants.len()
        || cohort.account_id.trim().is_empty()
        || !human[0]
            .nip05
            .trim()
            .eq_ignore_ascii_case(cohort.human_email.trim())
    {
        return Err(StoreError::BrokenInvariant {
            reason: "bootstrap account cohort is incomplete or ambiguous".to_owned(),
        });
    }
    match brain.kind {
        BrainKind::Personal if brain.owner_user_id.as_ref() != Some(&human[0].npub) => {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain cohort human must be the owner".to_owned(),
            });
        }
        BrainKind::Organization
            if !brain.admins.contains(&human[0].npub)
                || brain.admins.len() != 1
                || cohort.participants.iter().any(|participant| {
                    !brain
                        .members
                        .iter()
                        .any(|member| member.user_id == participant.npub)
                }) =>
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Organization cohort bootstrap requires one human admin and all participants as members"
                    .to_owned(),
            });
        }
        _ => {}
    }
    let cohort_id = format!("cohort-{}-bootstrap", brain.id);
    tx.execute(
        r#"
        INSERT INTO account_access_cohorts (
            id, brain_id, account_id, human_npub, human_email, scope_kind, folder_id,
            provenance_kind, provenance_id, roster_revision, status,
            created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, 'brain', NULL, ?6, ?7, ?8, 'active', ?9, ?9)
        "#,
        params![
            cohort_id,
            brain.id.as_str(),
            cohort.account_id,
            human[0].npub.as_str(),
            cohort.human_email,
            provenance_kind,
            format!("bootstrap-{}", brain.id),
            i64::try_from(cohort.roster_revision).map_err(|_| StoreError::BrokenInvariant {
                reason: "roster revision exceeds SQLite integer range".to_owned(),
            })?,
            created_at,
        ],
    )?;
    for participant in &cohort.participants {
        tx.execute(
            r#"
            INSERT INTO account_access_cohort_participants (
                cohort_id, participant_npub, relationship, nip05, display_name,
                status, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6)
            "#,
            params![
                cohort_id,
                participant.npub.as_str(),
                participant.relationship,
                participant.nip05,
                participant.name,
                created_at,
            ],
        )?;
        if participant.relationship == "account_agent" {
            tx.execute(
                r#"
                INSERT INTO human_anchored_agent_authorities (
                    cohort_id, brain_id, human_npub, agent_npub, status,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)
                "#,
                params![
                    cohort_id,
                    brain.id.as_str(),
                    human[0].npub.as_str(),
                    participant.npub.as_str(),
                    created_at,
                ],
            )?;
        }
    }
    tx.execute(
        r#"
        INSERT INTO account_access_cohort_audit (
            id, cohort_id, action, actor_npub, anchoring_human_npub,
            detail_json, occurred_at
        ) VALUES (?1, ?2, 'bootstrap_committed', ?3, ?4, ?5, ?6)
        "#,
        params![
            format!("audit-{cohort_id}-bootstrap"),
            cohort_id,
            human[0].npub.as_str(),
            human[0].npub.as_str(),
            serde_json::json!({ "participantCount": cohort.participants.len() }).to_string(),
            created_at,
        ],
    )?;
    Ok(())
}

fn equivalent_bootstrap_brain(existing: &Brain, requested: &Brain) -> bool {
    let existing_members = existing
        .members
        .iter()
        .map(|member| (&member.user_id, &member.folder_access))
        .collect::<BTreeSet<_>>();
    let requested_members = requested
        .members
        .iter()
        .map(|member| (&member.user_id, &member.folder_access))
        .collect::<BTreeSet<_>>();
    existing.id == requested.id
        && existing.kind == requested.kind
        && existing.name == requested.name
        && existing.owner_user_id == requested.owner_user_id
        && existing.folders == requested.folders
        && existing_members == requested_members
        && existing.admins.iter().collect::<BTreeSet<_>>()
            == requested.admins.iter().collect::<BTreeSet<_>>()
}
