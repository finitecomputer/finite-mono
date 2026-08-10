use crate::*;

impl BrainStore {
    pub fn permanent_agent_departure_applied(
        &self,
        brain_id: &BrainId,
        fact_id: &str,
    ) -> Result<bool, StoreError> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM account_agent_departure_facts WHERE fact_id = ?1 AND brain_id = ?2 AND applied_at IS NOT NULL)",
            params![fact_id, brain_id.as_str()],
            |row| row.get::<_, bool>(0),
        )?)
    }

    pub fn applied_permanent_agent_departure(
        &self,
        brain_id: &BrainId,
        fact_id: &str,
    ) -> Result<Option<(UserId, Vec<FolderId>)>, StoreError> {
        let stored = self
            .conn
            .query_row(
                "SELECT agent_npub, result_json FROM account_agent_departure_facts WHERE fact_id = ?1 AND brain_id = ?2 AND applied_at IS NOT NULL",
                params![fact_id, brain_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((agent_npub, result_json)) = stored else {
            return Ok(None);
        };
        let result: serde_json::Value =
            serde_json::from_str(&result_json).map_err(|error| StoreError::BrokenInvariant {
                reason: format!("stored departure result is invalid: {error}"),
            })?;
        let folders = result["rotatedFolderIds"]
            .as_array()
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "stored departure result omits rotated Folder ids".to_owned(),
            })?
            .iter()
            .map(|folder| {
                FolderId::new(folder.as_str().unwrap_or_default().to_owned())
                    .map_err(StoreError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some((UserId::new(agent_npub)?, folders)))
    }

    pub fn plan_permanent_agent_departure(
        &self,
        brain_id: &BrainId,
        account_id: &str,
        human_email: &str,
        agent_nip05: &str,
        agent_npub: &UserId,
        actor: &UserId,
    ) -> Result<PermanentAgentDeparturePlan, StoreError> {
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, actor) {
            return Err(StoreError::BrokenInvariant {
                reason: "permanent Agent departure requires Brain operational authority".to_owned(),
            });
        }
        let normalized_human = human_email.trim().to_ascii_lowercase();
        let normalized_agent = agent_nip05.trim().to_ascii_lowercase();
        let participant_matches = stored.account_access_cohorts.iter().any(|cohort| {
            cohort.status == "active"
                && cohort.account_id == account_id
                && cohort
                    .human_email
                    .trim()
                    .eq_ignore_ascii_case(&normalized_human)
                && cohort.participants.iter().any(|participant| {
                    participant.npub == *agent_npub
                        && participant.relationship == "account_agent"
                        && participant
                            .nip05
                            .trim()
                            .eq_ignore_ascii_case(&normalized_agent)
                        && participant.status == "active"
                })
        });
        if !participant_matches {
            return Err(StoreError::BrokenInvariant {
                reason: "permanent departure fact does not match an active Brain account cohort"
                    .to_owned(),
            });
        }

        let mut post = stored.clone();
        post.brain
            .members
            .retain(|member| member.user_id != *agent_npub);
        post.brain.admins.retain(|admin| admin != agent_npub);
        post.folder_access
            .values_mut()
            .for_each(|users| users.retain(|user| user != agent_npub));
        post.personal_brain_agents
            .iter_mut()
            .filter(|agent| agent.agent_npub == *agent_npub)
            .for_each(|agent| agent.status = "revoked".to_owned());
        if post
            .personal_agent
            .as_ref()
            .is_some_and(|relationship| relationship.agent_npub == *agent_npub)
        {
            post.personal_agent = None;
        }
        post.human_anchored_agent_authorities.remove(agent_npub);

        let mut folders = Vec::new();
        for folder in stored
            .brain
            .folders
            .iter()
            .filter(|folder| folder_visible_to_actor(&stored, &folder.id, agent_npub))
        {
            let mut rotated = folder.clone();
            rotated.current_key_version += 1;
            let explicit = post
                .folder_access
                .get(&folder.id)
                .cloned()
                .unwrap_or_default();
            let personal_agent = post
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub);
            let mut required =
                required_recipients(&post.brain, &rotated, &explicit, personal_agent)?;
            extend_account_agent_recipients(&mut required, &post, &folder.id);
            folders.push(PermanentAgentDepartureFolderPlan {
                folder_id: folder.id.clone(),
                current_key_version: folder.current_key_version,
                new_key_version: rotated.current_key_version,
                required_recipient_npubs: required,
            });
        }
        Ok(PermanentAgentDeparturePlan {
            account_id: account_id.to_owned(),
            human_email: normalized_human,
            agent_nip05: normalized_agent,
            agent_npub: agent_npub.clone(),
            folders,
        })
    }

    /// Apply one replayable permanent-departure fact and rotate every Folder
    /// the departed Agent Principal could read. Temporary runtime state never
    /// enters this boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_permanent_agent_departure(
        &mut self,
        brain_id: &BrainId,
        fact_id: &str,
        account_id: &str,
        human_email: &str,
        agent_nip05: &str,
        agent_npub: &UserId,
        departure_kind: &str,
        occurred_at: &str,
        actor: &UserId,
        rotations: &[MemberFolderRotation],
        control_records: &[SyncRecordInput],
        now: &str,
    ) -> Result<ApplyPermanentAgentDepartureOutcome, StoreError> {
        validate_link_id("permanent_agent_departure_fact_id", fact_id)?;
        if !matches!(departure_kind, "unlinked" | "retired" | "deleted") {
            return Err(StoreError::BrokenInvariant {
                reason: "only permanent unlink, retirement, or deletion is a departure fact"
                    .to_owned(),
            });
        }
        if self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM account_agent_departure_facts WHERE fact_id = ?1 AND brain_id = ?2 AND applied_at IS NOT NULL)",
            params![fact_id, brain_id.as_str()],
            |row| row.get::<_, bool>(0),
        )? {
            return Ok(ApplyPermanentAgentDepartureOutcome::AlreadyApplied);
        }
        let plan = self.plan_permanent_agent_departure(
            brain_id,
            account_id,
            human_email,
            agent_nip05,
            agent_npub,
            actor,
        )?;
        let stored = self.load_brain(brain_id)?;
        let expected_folders = plan
            .folders
            .iter()
            .map(|folder| folder.folder_id.clone())
            .collect::<BTreeSet<_>>();
        let supplied_folders = rotations
            .iter()
            .map(|rotation| rotation.folder_id.clone())
            .collect::<BTreeSet<_>>();
        if expected_folders != supplied_folders || rotations.len() != expected_folders.len() {
            return Err(StoreError::BrokenInvariant {
                reason: "permanent Agent departure requires one rotation for every readable Folder"
                    .to_owned(),
            });
        }
        let mut post = stored.clone();
        post.brain
            .members
            .retain(|member| member.user_id != *agent_npub);
        post.brain.admins.retain(|admin| admin != agent_npub);
        post.folder_access
            .values_mut()
            .for_each(|users| users.retain(|user| user != agent_npub));
        post.personal_brain_agents
            .iter_mut()
            .filter(|agent| agent.agent_npub == *agent_npub)
            .for_each(|agent| agent.status = "revoked".to_owned());
        if post
            .personal_agent
            .as_ref()
            .is_some_and(|relationship| relationship.agent_npub == *agent_npub)
        {
            post.personal_agent = None;
        }
        post.human_anchored_agent_authorities.remove(agent_npub);
        let current_objects = self.load_current_objects(brain_id)?;
        let mut total_grants = 0usize;
        for rotation in rotations {
            let folder = stored
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == rotation.folder_id)
                .expect("rotation folder set was validated");
            if rotation.new_key_version != folder.current_key_version + 1 {
                return Err(StoreError::BrokenInvariant {
                    reason: "permanent Agent departure rotation must use the next key version"
                        .to_owned(),
                });
            }
            let mut rotated = folder.clone();
            rotated.current_key_version = rotation.new_key_version;
            let explicit = post
                .folder_access
                .get(&folder.id)
                .cloned()
                .unwrap_or_default();
            let personal_agent = post
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub);
            let mut required =
                required_recipients(&post.brain, &rotated, &explicit, personal_agent)?;
            extend_account_agent_recipients(&mut required, &post, &folder.id);
            validate_folder_grants(
                &post.brain,
                &rotated,
                &required,
                &rotation.grants,
                personal_agent,
                rotation
                    .grants
                    .iter()
                    .all(|grant| has_brain_operational_authority(&stored, &grant.issuer_npub)),
            )?;
            let live_objects = current_objects
                .iter()
                .filter(|object| object.folder_id == folder.id && !object.deleted)
                .cloned()
                .collect::<Vec<_>>();
            validate_rotation_records(&live_objects, &rotation.reencrypted_records)?;
            total_grants += rotation.grants.len();
        }
        if control_records.len() != total_grants + 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "permanent Agent departure requires all grant controls and one access-change control"
                    .to_owned(),
            });
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO account_agent_departure_facts (
                fact_id, brain_id, account_id, agent_nip05, agent_npub,
                departure_kind, occurred_at, applied_at, result_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(fact_id, brain_id) DO UPDATE SET
                applied_at = COALESCE(account_agent_departure_facts.applied_at, excluded.applied_at),
                result_json = COALESCE(account_agent_departure_facts.result_json, excluded.result_json)
            "#,
            params![
                fact_id,
                brain_id.as_str(),
                account_id,
                agent_nip05,
                agent_npub.as_str(),
                departure_kind,
                occurred_at,
                now,
                serde_json::json!({
                    "brainId": brain_id.as_str(),
                    "rotatedFolderIds": rotations.iter().map(|rotation| rotation.folder_id.as_str()).collect::<Vec<_>>()
                }).to_string(),
            ],
        )?;
        for rotation in rotations {
            tx.execute(
                "DELETE FROM folder_access WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3",
                params![
                    brain_id.as_str(),
                    rotation.folder_id.as_str(),
                    agent_npub.as_str()
                ],
            )?;
            tx.execute(
                "UPDATE folders SET current_key_version = ?3 WHERE brain_id = ?1 AND id = ?2",
                params![
                    brain_id.as_str(),
                    rotation.folder_id.as_str(),
                    rotation.new_key_version
                ],
            )?;
            invalidate_pending_email_bootstraps_for_rotated_folder(
                &tx,
                brain_id,
                &rotation.folder_id,
                now,
            )?;
            for grant in &rotation.grants {
                insert_grant(&tx, brain_id, grant)?;
            }
            for record in &rotation.reencrypted_records {
                let input = SyncRecordInput::FolderObjectRevision(record.clone());
                sync_records::validate_sync_input(&input)?;
                sync_records::validate_sync_conflict(&tx, brain_id, &input)?;
                let sequence = sync_records::next_sequence(&tx, brain_id)?;
                sync_records::insert_sync_record(&tx, brain_id, sequence, &input)?;
                sync_records::project_sync_record(&tx, brain_id, &input)?;
            }
        }
        tx.execute(
            "DELETE FROM folder_access WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), agent_npub.as_str()],
        )?;
        tx.execute(
            "DELETE FROM brain_members WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), agent_npub.as_str()],
        )?;
        tx.execute(
            "DELETE FROM brain_admins WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), agent_npub.as_str()],
        )?;
        tx.execute(
            "DELETE FROM personal_agents WHERE brain_id = ?1 AND agent_npub = ?2",
            params![brain_id.as_str(), agent_npub.as_str()],
        )?;
        tx.execute(
            "UPDATE personal_brain_agents SET status = 'revoked', blocker = 'permanent_agent_departure', updated_at = ?3 WHERE brain_id = ?1 AND agent_npub = ?2",
            params![brain_id.as_str(), agent_npub.as_str(), now],
        )?;
        tx.execute(
            "UPDATE account_access_cohort_participants SET status = 'revoked', exclusion_reason = 'permanent_agent_departure', updated_at = ?3 WHERE participant_npub = ?2 AND cohort_id IN (SELECT id FROM account_access_cohorts WHERE brain_id = ?1)",
            params![brain_id.as_str(), agent_npub.as_str(), now],
        )?;
        tx.execute(
            "UPDATE human_anchored_agent_authorities SET status = 'revoked', updated_at = ?3 WHERE brain_id = ?1 AND agent_npub = ?2",
            params![brain_id.as_str(), agent_npub.as_str(), now],
        )?;
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.commit()?;
        Ok(ApplyPermanentAgentDepartureOutcome::Applied)
    }

    pub fn personal_brain_human_email(
        &self,
        brain_id: &BrainId,
    ) -> Result<Option<String>, StoreError> {
        self.conn
            .query_row(
                "SELECT human_email FROM account_access_cohorts WHERE brain_id = ?1 AND scope_kind = 'brain' AND status = 'active' ORDER BY created_at LIMIT 1",
                params![brain_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn plan_personal_agent_brain_access(
        &self,
        brain_id: &BrainId,
        target_agent_npub: &UserId,
        operation: &str,
        actor: &UserId,
    ) -> Result<PersonalAgentBrainAccessPlan, StoreError> {
        if !matches!(operation, "restrict" | "restore") || actor == target_agent_npub {
            return Err(StoreError::BrokenInvariant {
                reason: "peer Agent Brain access operation is invalid".to_owned(),
            });
        }
        let stored = self.load_brain(brain_id)?;
        let owner = stored
            .brain
            .owner_user_id
            .as_ref()
            .filter(|_| stored.brain.kind == BrainKind::Personal)
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "peer Agent Brain access requires a Personal Brain".to_owned(),
            })?;
        if !has_brain_operational_authority(&stored, actor)
            || !stored
                .personal_brain_agents
                .iter()
                .any(|agent| agent.status == "ready" && agent.agent_npub == *actor)
            || !stored
                .personal_brain_agents
                .iter()
                .any(|agent| agent.status == "ready" && agent.agent_npub == *target_agent_npub)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "peer Agent Brain access requires two ready Personal Brain Agents"
                    .to_owned(),
            });
        }
        let is_excluded = stored
            .account_agent_exclusions
            .contains(&(target_agent_npub.clone(), String::new()));
        if (operation == "restrict" && is_excluded) || (operation == "restore" && !is_excluded) {
            return Err(StoreError::BrokenInvariant {
                reason: format!("peer Agent Brain access is already {operation}ed"),
            });
        }
        let human_email = self.personal_brain_human_email(brain_id)?.ok_or_else(|| {
            StoreError::BrokenInvariant {
                reason: "Personal Brain account cohort is missing".to_owned(),
            }
        })?;
        let mut post = stored.clone();
        if operation == "restrict" {
            post.account_agent_exclusions
                .insert((target_agent_npub.clone(), String::new()));
        }
        let mut folders = Vec::with_capacity(stored.brain.folders.len());
        for folder in &stored.brain.folders {
            if operation == "restore" {
                folders.push(PersonalAgentBrainAccessFolderPlan {
                    folder_id: folder.id.clone(),
                    current_key_version: folder.current_key_version,
                    new_key_version: folder.current_key_version,
                    required_recipient_npubs: BTreeSet::from([target_agent_npub.clone()]),
                });
                continue;
            }
            let mut rotated = folder.clone();
            rotated.current_key_version += 1;
            let explicit = post
                .folder_access
                .get(&folder.id)
                .cloned()
                .unwrap_or_default();
            let personal_agent = post
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub);
            let mut required =
                required_recipients(&post.brain, &rotated, &explicit, personal_agent)?;
            extend_account_agent_recipients(&mut required, &post, &folder.id);
            folders.push(PersonalAgentBrainAccessFolderPlan {
                folder_id: folder.id.clone(),
                current_key_version: folder.current_key_version,
                new_key_version: rotated.current_key_version,
                required_recipient_npubs: required,
            });
        }
        Ok(PersonalAgentBrainAccessPlan {
            human_npub: owner.clone(),
            human_email,
            target_agent_npub: target_agent_npub.clone(),
            operation: operation.to_owned(),
            folders,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restrict_personal_agent_brain_access(
        &mut self,
        brain_id: &BrainId,
        target_agent_npub: &UserId,
        actor: &UserId,
        rotations: &[MemberFolderRotation],
        control_records: &[SyncRecordInput],
        intent: &AuthenticatedHumanIntentRecord,
        now: &str,
    ) -> Result<(), StoreError> {
        let plan =
            self.plan_personal_agent_brain_access(brain_id, target_agent_npub, "restrict", actor)?;
        if intent.human_npub != plan.human_npub
            || intent.acting_agent_npub != *actor
            || intent.target_agent_npub != *target_agent_npub
            || intent.operation != "restrict"
            || intent.scope_kind != "brain"
            || intent.folder_id.is_some()
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Authenticated Human Intent does not authorize this Brain restriction"
                    .to_owned(),
            });
        }
        let expected = plan
            .folders
            .iter()
            .map(|folder| (folder.folder_id.clone(), folder))
            .collect::<BTreeMap<_, _>>();
        if rotations.len() != expected.len()
            || rotations
                .iter()
                .map(|rotation| &rotation.folder_id)
                .collect::<BTreeSet<_>>()
                .len()
                != expected.len()
        {
            return Err(StoreError::BrokenInvariant {
                reason: "peer Agent Brain restriction requires every Folder rotation".to_owned(),
            });
        }
        let stored = self.load_brain(brain_id)?;
        let current_objects = self.load_current_objects(brain_id)?;
        let mut all_grants = Vec::new();
        for rotation in rotations {
            let expected_folder =
                expected
                    .get(&rotation.folder_id)
                    .ok_or_else(|| StoreError::BrokenInvariant {
                        reason: "peer Agent Brain restriction contains an unexpected Folder"
                            .to_owned(),
                    })?;
            let folder = stored
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == rotation.folder_id)
                .expect("planned Folder exists");
            let recipients = rotation
                .grants
                .iter()
                .map(|grant| grant.recipient_npub.clone())
                .collect::<BTreeSet<_>>();
            if rotation.new_key_version != expected_folder.new_key_version
                || recipients != expected_folder.required_recipient_npubs
                || rotation.grants.len() != recipients.len()
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "peer Agent Brain restriction grants do not match the plan".to_owned(),
                });
            }
            let mut rotated = folder.clone();
            rotated.current_key_version = rotation.new_key_version;
            validate_folder_grants(
                &stored.brain,
                &rotated,
                &expected_folder.required_recipient_npubs,
                &rotation.grants,
                stored
                    .personal_agent
                    .as_ref()
                    .map(|relationship| &relationship.agent_npub),
                true,
            )?;
            let live_objects = current_objects
                .iter()
                .filter(|object| object.folder_id == rotation.folder_id && !object.deleted)
                .cloned()
                .collect::<Vec<_>>();
            validate_rotation_records(&live_objects, &rotation.reencrypted_records)?;
            all_grants.extend(rotation.grants.iter().cloned());
        }
        validate_access_mutation_control_records(&all_grants, control_records, actor)?;
        let cohort_ids = stored
            .account_access_cohorts
            .iter()
            .filter(|cohort| {
                cohort.status == "active"
                    && cohort.scope_kind == "brain"
                    && cohort.participants.iter().any(|participant| {
                        participant.npub == *target_agent_npub
                            && participant.relationship == "account_agent"
                            && participant.status == "active"
                    })
            })
            .map(|cohort| cohort.cohort_id.clone())
            .collect::<Vec<_>>();
        if cohort_ids.is_empty() {
            return Err(StoreError::BrokenInvariant {
                reason: "peer Agent has no active Personal Brain cohort".to_owned(),
            });
        }
        let tx = self.conn.transaction()?;
        consume_authenticated_human_intent(&tx, brain_id, intent)?;
        for rotation in rotations {
            tx.execute(
                "UPDATE folders SET current_key_version = ?3 WHERE brain_id = ?1 AND id = ?2",
                params![
                    brain_id.as_str(),
                    rotation.folder_id.as_str(),
                    rotation.new_key_version
                ],
            )?;
            invalidate_pending_email_bootstraps_for_rotated_folder(
                &tx,
                brain_id,
                &rotation.folder_id,
                now,
            )?;
            for grant in &rotation.grants {
                insert_grant(&tx, brain_id, grant)?;
            }
            for record in &rotation.reencrypted_records {
                let input = SyncRecordInput::FolderObjectRevision(record.clone());
                sync_records::validate_sync_input(&input)?;
                sync_records::validate_sync_conflict(&tx, brain_id, &input)?;
                let sequence = sync_records::next_sequence(&tx, brain_id)?;
                sync_records::insert_sync_record(&tx, brain_id, sequence, &input)?;
                sync_records::project_sync_record(&tx, brain_id, &input)?;
            }
        }
        for cohort_id in cohort_ids {
            tx.execute(
                r#"
                INSERT INTO account_access_cohort_exclusions (
                    cohort_id, participant_npub, folder_id, reason, active,
                    created_at, updated_at
                ) VALUES (?1, ?2, '', 'authenticated_human_restriction', 1, ?3, ?3)
                ON CONFLICT(cohort_id, participant_npub, folder_id) DO UPDATE SET
                    reason = excluded.reason, active = 1, updated_at = excluded.updated_at
                "#,
                params![cohort_id, target_agent_npub.as_str(), now],
            )?;
            insert_peer_agent_access_audit(
                &tx,
                &cohort_id,
                "participant_brain_restricted",
                actor,
                &plan.human_npub,
                target_agent_npub,
                intent,
                now,
            )?;
        }
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.commit()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore_personal_agent_brain_access(
        &mut self,
        brain_id: &BrainId,
        target_agent_npub: &UserId,
        actor: &UserId,
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
        intent: &AuthenticatedHumanIntentRecord,
        now: &str,
    ) -> Result<(), StoreError> {
        let plan =
            self.plan_personal_agent_brain_access(brain_id, target_agent_npub, "restore", actor)?;
        if intent.human_npub != plan.human_npub
            || intent.acting_agent_npub != *actor
            || intent.target_agent_npub != *target_agent_npub
            || intent.operation != "restore"
            || intent.scope_kind != "brain"
            || intent.folder_id.is_some()
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Authenticated Human Intent does not authorize this Brain restoration"
                    .to_owned(),
            });
        }
        let expected = plan
            .folders
            .iter()
            .map(|folder| {
                (
                    folder.folder_id.clone(),
                    (folder.current_key_version, target_agent_npub.clone()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        if grants.len() != expected.len()
            || grants
                .iter()
                .map(|grant| grant.folder_id.clone())
                .collect::<BTreeSet<_>>()
                .len()
                != expected.len()
        {
            return Err(StoreError::BrokenInvariant {
                reason: "peer Agent Brain restoration requires one current grant per Folder"
                    .to_owned(),
            });
        }
        let stored = self.load_brain(brain_id)?;
        for grant in grants {
            validate_grant_metadata(grant)?;
            let Some((version, recipient)) = expected.get(&grant.folder_id) else {
                return Err(StoreError::BrokenInvariant {
                    reason: "peer Agent Brain restoration contains an unexpected Folder".to_owned(),
                });
            };
            if grant.key_version != *version
                || grant.recipient_npub != *recipient
                || grant.issuer_npub != *actor
                || !has_brain_operational_authority(&stored, actor)
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "peer Agent Brain restoration grant is invalid".to_owned(),
                });
            }
        }
        validate_access_mutation_control_records(grants, control_records, actor)?;
        let cohort_ids = stored
            .account_access_cohorts
            .iter()
            .filter(|cohort| {
                cohort.status == "active"
                    && cohort.scope_kind == "brain"
                    && cohort.participants.iter().any(|participant| {
                        participant.npub == *target_agent_npub
                            && participant.relationship == "account_agent"
                            && participant.status == "active"
                    })
            })
            .map(|cohort| cohort.cohort_id.clone())
            .collect::<Vec<_>>();
        let tx = self.conn.transaction()?;
        consume_authenticated_human_intent(&tx, brain_id, intent)?;
        let changed = tx.execute(
            "UPDATE account_access_cohort_exclusions SET active = 0, updated_at = ?3 WHERE participant_npub = ?2 AND folder_id = '' AND active = 1 AND cohort_id IN (SELECT id FROM account_access_cohorts WHERE brain_id = ?1 AND status = 'active')",
            params![brain_id.as_str(), target_agent_npub.as_str(), now],
        )?;
        if changed == 0 {
            return Err(StoreError::BrokenInvariant {
                reason: "peer Agent Brain restriction was already cleared".to_owned(),
            });
        }
        for grant in grants {
            insert_grant(&tx, brain_id, grant)?;
        }
        for cohort_id in cohort_ids {
            insert_peer_agent_access_audit(
                &tx,
                &cohort_id,
                "participant_brain_restored",
                actor,
                &plan.human_npub,
                target_agent_npub,
                intent,
                now,
            )?;
        }
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.commit()?;
        Ok(())
    }

    /// Materialize fresh Core/Identity roster facts as the Personal Brain's
    /// desired set without changing current readers or Agent readiness.
    pub fn stage_personal_brain_agent_admissions(
        &mut self,
        brain_id: &BrainId,
        cohort: &BootstrapAccountCohort,
        now: &str,
    ) -> Result<PersonalBrainAgentAdmissionPlan, StoreError> {
        let stored = self.load_brain(brain_id)?;
        if stored.brain.kind != BrainKind::Personal {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain agent admission requires a Personal Brain".to_owned(),
            });
        }
        let owner =
            stored
                .brain
                .owner_user_id
                .as_ref()
                .ok_or_else(|| StoreError::BrokenInvariant {
                    reason: "Personal Brain owner is missing".to_owned(),
                })?;
        let human = cohort
            .participants
            .iter()
            .find(|participant| participant.relationship == "human")
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "Personal Brain roster requires its human participant".to_owned(),
            })?;
        if &human.npub != owner {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain roster human does not match its owner".to_owned(),
            });
        }
        let cohort_id = self
            .conn
            .query_row(
                "SELECT id FROM account_access_cohorts WHERE brain_id = ?1 AND scope_kind = 'brain' AND human_npub = ?2 AND status = 'active' ORDER BY created_at LIMIT 1",
                params![brain_id.as_str(), owner.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "Personal Brain has no reconciled account cohort".to_owned(),
            })?;
        let revision =
            i64::try_from(cohort.roster_revision).map_err(|_| StoreError::BrokenInvariant {
                reason: "roster revision exceeds SQLite range".to_owned(),
            })?;
        let tx = self.conn.transaction()?;
        for agent in cohort
            .participants
            .iter()
            .filter(|participant| participant.relationship == "account_agent")
        {
            tx.execute(
                r#"
                INSERT INTO personal_brain_agents (
                    brain_id, agent_npub, agent_nip05, display_name, status,
                    roster_revision, blocker, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, 'desired', ?5, NULL, ?6, ?6)
                ON CONFLICT(brain_id, agent_npub) DO UPDATE SET
                    agent_nip05 = excluded.agent_nip05,
                    display_name = excluded.display_name,
                    roster_revision = MAX(personal_brain_agents.roster_revision, excluded.roster_revision),
                    updated_at = excluded.updated_at
                "#,
                params![
                    brain_id.as_str(),
                    agent.npub.as_str(),
                    agent.nip05,
                    agent.name,
                    revision,
                    now,
                ],
            )?;
        }
        tx.commit()?;
        let current = self.load_personal_brain_agents(brain_id)?;
        let current_by_npub = current
            .into_iter()
            .map(|agent| (agent.agent_npub, agent.status))
            .collect::<BTreeMap<_, _>>();
        let agents = cohort
            .participants
            .iter()
            .filter(|participant| participant.relationship == "account_agent")
            .filter(|participant| {
                current_by_npub
                    .get(&participant.npub)
                    .is_some_and(|status| status == "desired" || status == "blocked")
            })
            .cloned()
            .collect();
        Ok(PersonalBrainAgentAdmissionPlan {
            cohort_id,
            human_npub: owner.clone(),
            human_email: cohort.human_email.clone(),
            roster_revision: cohort.roster_revision,
            agents,
            folder_key_versions: stored
                .brain
                .folders
                .iter()
                .map(|folder| (folder.id.clone(), folder.current_key_version))
                .collect(),
        })
    }

    /// Atomically make every planned Personal Brain agent ready after exact
    /// current Folder grants have been prepared by an existing reader.
    pub fn commit_personal_brain_agent_admissions(
        &mut self,
        brain_id: &BrainId,
        plan: &PersonalBrainAgentAdmissionPlan,
        actor: &UserId,
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
        now: &str,
    ) -> Result<(), StoreError> {
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, actor) {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain agent admission requires operational authority".to_owned(),
            });
        }
        let expected = plan
            .agents
            .iter()
            .flat_map(|agent| {
                plan.folder_key_versions
                    .iter()
                    .map(move |(folder, version)| (folder.clone(), *version, agent.npub.clone()))
            })
            .collect::<BTreeSet<_>>();
        let provided = grants
            .iter()
            .map(|grant| {
                (
                    grant.folder_id.clone(),
                    grant.key_version,
                    grant.recipient_npub.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        if expected != provided || grants.len() != expected.len() {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain admission grants must cover every planned agent and current Folder"
                    .to_owned(),
            });
        }
        for grant in grants {
            validate_grant_metadata(grant)?;
            if grant.issuer_npub != *actor {
                return Err(StoreError::BrokenInvariant {
                    reason: "Personal Brain admission grant issuer must be the acting reader"
                        .to_owned(),
                });
            }
        }
        validate_folder_key_grant_control_records(grants, control_records)?;
        let revision =
            i64::try_from(plan.roster_revision).map_err(|_| StoreError::BrokenInvariant {
                reason: "roster revision exceeds SQLite range".to_owned(),
            })?;
        let tx = self.conn.transaction()?;
        for agent in &plan.agents {
            let status = tx.query_row(
                "SELECT status FROM personal_brain_agents WHERE brain_id = ?1 AND agent_npub = ?2",
                params![brain_id.as_str(), agent.npub.as_str()],
                |row| row.get::<_, String>(0),
            )?;
            if status == "revoked" {
                return Err(StoreError::BrokenInvariant {
                    reason: "revoked Personal Brain agents require explicit restoration".to_owned(),
                });
            }
            tx.execute(
                "UPDATE personal_brain_agents SET status = 'ready', blocker = NULL, roster_revision = MAX(roster_revision, ?3), updated_at = ?4 WHERE brain_id = ?1 AND agent_npub = ?2",
                params![brain_id.as_str(), agent.npub.as_str(), revision, now],
            )?;
            tx.execute(
                r#"
                INSERT INTO account_access_cohort_participants (
                    cohort_id, participant_npub, relationship, nip05,
                    display_name, status, created_at, updated_at
                ) VALUES (?1, ?2, 'account_agent', ?3, ?4, 'active', ?5, ?5)
                ON CONFLICT(cohort_id, participant_npub) DO UPDATE SET
                    nip05 = excluded.nip05,
                    display_name = excluded.display_name,
                    status = 'active',
                    exclusion_reason = NULL,
                    updated_at = excluded.updated_at
                "#,
                params![
                    plan.cohort_id,
                    agent.npub.as_str(),
                    agent.nip05,
                    agent.name,
                    now
                ],
            )?;
            tx.execute(
                r#"
                INSERT INTO human_anchored_agent_authorities (
                    cohort_id, brain_id, human_npub, agent_npub, status,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)
                ON CONFLICT(cohort_id, agent_npub) DO UPDATE SET
                    status = 'active', updated_at = excluded.updated_at
                "#,
                params![
                    plan.cohort_id,
                    brain_id.as_str(),
                    plan.human_npub.as_str(),
                    agent.npub.as_str(),
                    now,
                ],
            )?;
        }
        for grant in grants {
            insert_grant(&tx, brain_id, grant)?;
        }
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.execute(
            r#"
            INSERT OR IGNORE INTO account_access_cohort_audit (
                id, cohort_id, action, actor_npub, anchoring_human_npub,
                detail_json, occurred_at
            ) VALUES (?1, ?2, 'personal_agents_admitted', ?3, ?4, ?5, ?6)
            "#,
            params![
                format!("audit-{}-personal-admit-r{}", plan.cohort_id, plan.roster_revision),
                plan.cohort_id,
                actor.as_str(),
                plan.human_npub.as_str(),
                serde_json::json!({
                    "agents": plan.agents.iter().map(|agent| agent.npub.as_str()).collect::<Vec<_>>(),
                    "folderCount": plan.folder_key_versions.len(),
                }).to_string(),
                now,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Plan removal of every active cohort-derived source for one mailbox and
    /// restricted Folder without mutating access or key state.
    pub fn plan_account_cohort_folder_access_removal(
        &self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        human_email: &str,
    ) -> Result<AccountCohortFolderRemovalPlan, StoreError> {
        let stored = self.load_brain(brain_id)?;
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
                reason: "mailbox Folder removal applies only to restricted Folders".to_owned(),
            });
        }
        let mut stmt = self.conn.prepare(
            r#"
            SELECT cohort.id, cohort.provenance_kind, cohort.provenance_id,
                   participant.participant_npub, participant.relationship,
                   participant.nip05, participant.display_name
            FROM account_access_cohorts cohort
            JOIN account_access_cohort_participants participant
              ON participant.cohort_id = cohort.id
            WHERE cohort.brain_id = ?1
              AND cohort.human_email = ?2
              AND cohort.status = 'active'
              AND participant.status = 'active'
            ORDER BY cohort.id, participant.participant_npub
            "#,
        )?;
        let rows = stmt.query_map(params![brain_id.as_str(), human_email], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        let mut cohort_ids = BTreeSet::new();
        let mut source_origins = BTreeSet::new();
        let mut participants = BTreeMap::<UserId, StoredCohortParticipant>::new();
        for row in rows {
            let (cohort_id, provenance_kind, provenance_id, npub, relationship, nip05, name) = row?;
            let source = match provenance_kind.as_str() {
                "mailbox_folder_access" => ("direct".to_owned(), cohort_id.clone()),
                "invitation" => ("invitation".to_owned(), provenance_id),
                _ => continue,
            };
            cohort_ids.insert(cohort_id);
            source_origins.insert(source);
            let npub = UserId::new(npub)?;
            participants
                .entry(npub.clone())
                .or_insert(StoredCohortParticipant {
                    relationship,
                    name,
                    nip05,
                    npub,
                });
        }
        if source_origins.is_empty() {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox has no cohort-derived access in this Folder".to_owned(),
            });
        }

        let mut removed = BTreeSet::new();
        let mut retained = BTreeSet::new();
        for participant in participants.values() {
            let mut sources = self.conn.prepare(
                "SELECT source_kind, source_id FROM folder_access_sources WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3",
            )?;
            let sources = sources
                .query_map(
                    params![
                        brain_id.as_str(),
                        folder_id.as_str(),
                        participant.npub.as_str()
                    ],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )?
                .collect::<Result<BTreeSet<_>, _>>()?;
            if !sources.iter().any(|source| source_origins.contains(source)) {
                continue;
            }
            let independent_source = sources
                .iter()
                .any(|source| !source_origins.contains(source));
            let native_access = stored.brain.owner_user_id.as_ref() == Some(&participant.npub)
                || stored.brain.admins.contains(&participant.npub)
                || is_ready_personal_agent(&stored, &participant.npub)
                || stored
                    .human_anchored_agent_authorities
                    .get(&participant.npub)
                    .is_some_and(|human| {
                        stored.brain.owner_user_id.as_ref() == Some(human)
                            || stored.brain.admins.contains(human)
                    });
            if independent_source || native_access {
                retained.insert(participant.npub.clone());
            } else {
                removed.insert(participant.npub.clone());
            }
        }
        let mut remaining_access = stored
            .folder_access
            .get(folder_id)
            .cloned()
            .unwrap_or_default();
        for participant in &removed {
            remaining_access.remove(participant);
        }
        let mut rotated = folder.clone();
        rotated.current_key_version = folder.current_key_version + 1;
        let personal_agent = stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub);
        let mut required =
            required_recipients(&stored.brain, &rotated, &remaining_access, personal_agent)?;
        extend_account_agent_recipients(&mut required, &stored, folder_id);
        Ok(AccountCohortFolderRemovalPlan {
            cohort_ids: cohort_ids.into_iter().collect(),
            source_origins: source_origins.into_iter().collect(),
            participants: participants.into_values().collect(),
            removed_participant_npubs: removed,
            independently_retained_npubs: retained,
            required_recipient_npubs: required,
            current_key_version: folder.current_key_version,
            new_key_version: folder.current_key_version + 1,
        })
    }

    /// Commit one previously reviewed mailbox Folder-removal plan with a
    /// single Folder Key rotation. Any independent source remains intact.
    #[allow(clippy::too_many_arguments)]
    pub fn remove_account_cohort_folder_access(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        plan: &AccountCohortFolderRemovalPlan,
        actor: &UserId,
        grants: &[FolderKeyGrantMetadata],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        control_records: &[SyncRecordInput],
        now: &str,
    ) -> Result<(), StoreError> {
        if plan.removed_participant_npubs.is_empty() {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder removal has no effective access to revoke".to_owned(),
            });
        }
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, actor) {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder removal requires Brain operational authority".to_owned(),
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
        if folder.current_key_version != plan.current_key_version
            || plan.new_key_version != folder.current_key_version + 1
        {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder removal plan is stale".to_owned(),
            });
        }
        let provided = grants
            .iter()
            .map(|grant| grant.recipient_npub.clone())
            .collect::<BTreeSet<_>>();
        if provided != plan.required_recipient_npubs
            || grants.len() != plan.required_recipient_npubs.len()
        {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder rotation grants must exactly match remaining readers"
                    .to_owned(),
            });
        }
        for grant in grants {
            validate_grant_metadata(grant)?;
            if grant.folder_id != *folder_id
                || grant.key_version != plan.new_key_version
                || grant.issuer_npub != *actor
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "mailbox Folder rotation grant does not match the reviewed plan"
                        .to_owned(),
                });
            }
        }
        if control_records.len() != grants.len() + 1 {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "mailbox Folder removal requires grant records plus one access-change record"
                        .to_owned(),
            });
        }
        validate_folder_key_grant_control_records(grants, &control_records[..grants.len()])?;
        let SyncRecordInput::Control(access_record) = &control_records[grants.len()] else {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder removal access-change must be a control record".to_owned(),
            });
        };
        sync_records::validate_sync_input(&control_records[grants.len()])?;
        if access_record.record_type != SyncRecordType::BrainAdminAccessChange
            || access_record.folder_id.as_ref() != Some(folder_id)
            || access_record.actor_npub != *actor
        {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder removal access-change record does not match the plan"
                    .to_owned(),
            });
        }
        let live_objects = self
            .load_current_objects(brain_id)?
            .into_iter()
            .filter(|object| object.folder_id == *folder_id && !object.deleted)
            .collect::<Vec<_>>();
        validate_rotation_records(&live_objects, reencrypted_records)?;

        let tx = self.conn.transaction()?;
        for participant in &plan.participants {
            for (source_kind, source_id) in &plan.source_origins {
                delete_folder_access_source(
                    &tx,
                    brain_id,
                    folder_id,
                    &participant.npub,
                    source_kind,
                    source_id,
                )?;
            }
        }
        for participant in &plan.removed_participant_npubs {
            tx.execute(
                "DELETE FROM folder_access WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3",
                params![brain_id.as_str(), folder_id.as_str(), participant.as_str()],
            )?;
        }
        for cohort_id in &plan.cohort_ids {
            for participant in &plan.participants {
                tx.execute(
                    r#"
                    INSERT INTO account_access_cohort_exclusions (
                        cohort_id, participant_npub, folder_id, reason, active,
                        created_at, updated_at
                    ) VALUES (?1, ?2, ?3, 'mailbox_folder_access_removed', 1, ?4, ?4)
                    ON CONFLICT(cohort_id, participant_npub, folder_id) DO UPDATE SET
                        reason = excluded.reason,
                        active = 1,
                        updated_at = excluded.updated_at
                    "#,
                    params![
                        cohort_id,
                        participant.npub.as_str(),
                        folder_id.as_str(),
                        now
                    ],
                )?;
            }
            tx.execute(
                r#"
                INSERT OR IGNORE INTO account_access_cohort_audit (
                    id, cohort_id, action, actor_npub, anchoring_human_npub,
                    detail_json, occurred_at
                )
                SELECT ?1, ?2, 'folder_access_removed', ?3, human_npub, ?4, ?5
                FROM account_access_cohorts WHERE id = ?2
                "#,
                params![
                    format!("audit-{cohort_id}-{folder_id}-remove-v{}", plan.new_key_version),
                    cohort_id,
                    actor.as_str(),
                    serde_json::json!({
                        "folderId": folder_id.as_str(),
                        "removedParticipants": plan.removed_participant_npubs.iter().map(UserId::as_str).collect::<Vec<_>>(),
                        "independentlyRetainedParticipants": plan.independently_retained_npubs.iter().map(UserId::as_str).collect::<Vec<_>>(),
                        "newKeyVersion": plan.new_key_version,
                    }).to_string(),
                    now,
                ],
            )?;
        }
        tx.execute(
            "UPDATE folders SET current_key_version = ?3 WHERE brain_id = ?1 AND id = ?2",
            params![brain_id.as_str(), folder_id.as_str(), plan.new_key_version],
        )?;
        invalidate_pending_email_bootstraps_for_rotated_folder(&tx, brain_id, folder_id, now)?;
        for grant in grants {
            insert_grant(&tx, brain_id, grant)?;
        }
        for record in reencrypted_records {
            let input = SyncRecordInput::FolderObjectRevision(record.clone());
            sync_records::validate_sync_input(&input)?;
            sync_records::validate_sync_conflict(&tx, brain_id, &input)?;
            let sequence = sync_records::next_sequence(&tx, brain_id)?;
            sync_records::insert_sync_record(&tx, brain_id, sequence, &input)?;
            sync_records::project_sync_record(&tx, brain_id, &input)?;
        }
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.commit()?;
        Ok(())
    }

    /// Atomically grant one fixed account cohort access to a restricted Folder.
    /// Folder Keys remain opaque; the caller supplies one recipient-wrapped
    /// grant and signed control record for every participant.
    #[allow(clippy::too_many_arguments)]
    pub fn grant_account_cohort_folder_access(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        operation_id: &str,
        cohort: &BootstrapAccountCohort,
        actor: &UserId,
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
        now: &str,
    ) -> Result<GrantAccountCohortFolderAccessOutcome, StoreError> {
        validate_link_id("cohort_folder_access_operation_id", operation_id)?;
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, actor) {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder access requires Brain operational authority".to_owned(),
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
                reason: "mailbox Folder access applies only to restricted Folders".to_owned(),
            });
        }
        let existing = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM account_access_cohorts WHERE brain_id = ?1 AND provenance_kind = 'mailbox_folder_access' AND provenance_id = ?2)",
            params![brain_id.as_str(), operation_id],
            |row| row.get::<_, bool>(0),
        )?;
        if existing {
            return Ok(GrantAccountCohortFolderAccessOutcome::AlreadyApplied);
        }

        let human = cohort
            .participants
            .iter()
            .find(|participant| participant.relationship == "human")
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "account cohort requires one human participant".to_owned(),
            })?;
        let participants = cohort
            .participants
            .iter()
            .map(|participant| participant.npub.clone())
            .collect::<BTreeSet<_>>();
        if participants.len() != cohort.participants.len()
            || cohort
                .participants
                .iter()
                .filter(|participant| participant.relationship == "human")
                .count()
                != 1
            || cohort.participants.iter().any(|participant| {
                participant.relationship != "human" && participant.relationship != "account_agent"
            })
        {
            return Err(StoreError::BrokenInvariant {
                reason: "account cohort participants are ambiguous".to_owned(),
            });
        }
        let provided = grants
            .iter()
            .map(|grant| grant.recipient_npub.clone())
            .collect::<BTreeSet<_>>();
        if provided != participants || grants.len() != participants.len() {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder grants must exactly match the fixed cohort".to_owned(),
            });
        }
        for grant in grants {
            validate_grant_metadata(grant)?;
            if grant.folder_id != *folder_id
                || grant.key_version != folder.current_key_version
                || grant.issuer_npub != *actor
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "mailbox Folder grant does not match the current Folder plan"
                        .to_owned(),
                });
            }
        }
        if control_records.len() != grants.len() + 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder access requires one grant record per participant and one access-change record".to_owned(),
            });
        }
        validate_folder_key_grant_control_records(grants, &control_records[..grants.len()])?;
        let SyncRecordInput::Control(access_record) = &control_records[grants.len()] else {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder access-change record must be a control record".to_owned(),
            });
        };
        sync_records::validate_sync_input(&control_records[grants.len()])?;
        if access_record.record_type != SyncRecordType::BrainAdminAccessChange
            || access_record.folder_id.as_ref() != Some(folder_id)
            || access_record.actor_npub != *actor
        {
            return Err(StoreError::BrokenInvariant {
                reason: "mailbox Folder access-change record does not match the operation"
                    .to_owned(),
            });
        }

        let cohort_id = format!("cohort-{operation_id}");
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO account_access_cohorts (
                id, brain_id, account_id, human_npub, human_email, scope_kind, folder_id,
                provenance_kind, provenance_id, roster_revision, status,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'folder', ?6,
                      'mailbox_folder_access', ?7, ?8, 'active', ?9, ?9)
            "#,
            params![
                cohort_id,
                brain_id.as_str(),
                cohort.account_id,
                human.npub.as_str(),
                cohort.human_email,
                folder_id.as_str(),
                operation_id,
                i64::try_from(cohort.roster_revision).map_err(|_| {
                    StoreError::BrokenInvariant {
                        reason: "cohort roster revision exceeds SQLite range".to_owned(),
                    }
                })?,
                now,
            ],
        )?;
        for participant in &cohort.participants {
            tx.execute(
                r#"
                INSERT INTO account_access_cohort_participants (
                    cohort_id, participant_npub, relationship, nip05,
                    display_name, status, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6)
                "#,
                params![
                    cohort_id,
                    participant.npub.as_str(),
                    participant.relationship,
                    participant.nip05,
                    participant.name,
                    now,
                ],
            )?;
            insert_folder_access_if_missing(&tx, brain_id, folder_id, &participant.npub)?;
            insert_folder_access_source(
                &tx,
                brain_id,
                folder_id,
                &participant.npub,
                "direct",
                &cohort_id,
                now,
            )?;
        }
        for grant in grants {
            insert_grant(&tx, brain_id, grant)?;
        }
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.execute(
            r#"
            INSERT INTO account_access_cohort_audit (
                id, cohort_id, action, actor_npub, anchoring_human_npub,
                detail_json, occurred_at
            ) VALUES (?1, ?2, 'folder_access_granted', ?3, ?4, ?5, ?6)
            "#,
            params![
                format!("audit-{operation_id}-grant"),
                cohort_id,
                actor.as_str(),
                human.npub.as_str(),
                serde_json::json!({
                    "folderId": folder_id.as_str(),
                    "participants": cohort.participants.iter().map(|participant| participant.npub.as_str()).collect::<Vec<_>>(),
                })
                .to_string(),
                now,
            ],
        )?;
        tx.commit()?;
        Ok(GrantAccountCohortFolderAccessOutcome::Granted)
    }

    pub fn plan_account_cohort_reconciliation(
        &self,
        brain_id: &BrainId,
        cohort: &BootstrapAccountCohort,
        folder_id: Option<&FolderId>,
        actor: &UserId,
    ) -> Result<AccountCohortReconciliationPlan, StoreError> {
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, actor) {
            return Err(StoreError::BrokenInvariant {
                reason: "cohort reconciliation requires Brain operational authority".to_owned(),
            });
        }
        let human = cohort
            .participants
            .iter()
            .find(|participant| participant.relationship == "human")
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "reconciliation cohort human is missing".to_owned(),
            })?;
        let is_member_scope = stored.brain.owner_user_id.as_ref() == Some(&human.npub)
            || stored
                .brain
                .members
                .iter()
                .any(|member| member.user_id == human.npub);
        let (scope_kind, selected_folder) = match (is_member_scope, folder_id) {
            (true, None) => ("brain", None),
            (true, Some(_)) => {
                return Err(StoreError::BrokenInvariant {
                    reason: "a Brain Member must reconcile as one complete Brain unit".to_owned(),
                });
            }
            (false, Some(folder_id))
                if stored
                    .folder_access
                    .get(folder_id)
                    .is_some_and(|users| users.contains(&human.npub)) =>
            {
                ("folder", Some(folder_id.clone()))
            }
            _ => {
                return Err(StoreError::BrokenInvariant {
                    reason: "human has no matching legacy Member or Folder-only Guest access"
                        .to_owned(),
                });
            }
        };
        let already_reconciled = stored.account_access_cohorts.iter().any(|candidate| {
            candidate.status == "active"
                && candidate.human_npub == human.npub
                && candidate.scope_kind == scope_kind
                && candidate.folder_id == selected_folder
        });
        let intended_folders = if let Some(folder_id) = selected_folder.as_ref() {
            stored
                .brain
                .folders
                .iter()
                .filter(|folder| folder.id == *folder_id)
                .collect::<Vec<_>>()
        } else {
            stored
                .brain
                .folders
                .iter()
                .filter(|folder| folder_visible_to_actor(&stored, &folder.id, &human.npub))
                .collect::<Vec<_>>()
        };
        let participant_npubs = cohort
            .participants
            .iter()
            .map(|participant| participant.npub.clone())
            .collect::<BTreeSet<_>>();
        let agent_npubs = cohort
            .participants
            .iter()
            .filter(|participant| participant.relationship == "account_agent")
            .map(|participant| participant.npub.clone())
            .collect::<BTreeSet<_>>();
        let mut independent_agents = BTreeSet::new();
        let mut folders = Vec::with_capacity(intended_folders.len());
        let mut missing_grants = 0usize;
        for folder in intended_folders {
            let current = stored
                .grants
                .iter()
                .filter(|grant| {
                    grant.folder_id == folder.id
                        && grant.key_version == folder.current_key_version
                        && participant_npubs.contains(&grant.recipient_npub)
                })
                .map(|grant| grant.recipient_npub.clone())
                .collect::<BTreeSet<_>>();
            let missing = participant_npubs
                .difference(&current)
                .cloned()
                .collect::<BTreeSet<_>>();
            missing_grants += missing.len();
            independent_agents.extend(
                agent_npubs
                    .iter()
                    .filter(|agent| {
                        stored
                            .folder_access
                            .get(&folder.id)
                            .is_some_and(|users| users.contains(*agent))
                            || current.contains(*agent)
                    })
                    .cloned(),
            );
            folders.push(AccountCohortReconciliationFolderPlan {
                folder_id: folder.id.clone(),
                key_version: folder.current_key_version,
                current_grant_recipient_npubs: current.into_iter().collect(),
                missing_grant_recipient_npubs: missing.into_iter().collect(),
            });
        }
        let expected_member_additions =
            if stored.brain.kind == BrainKind::Organization && scope_kind == "brain" {
                agent_npubs
                    .iter()
                    .filter(|agent| {
                        !stored
                            .brain
                            .members
                            .iter()
                            .any(|member| &member.user_id == *agent)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
        let folder_access_additions = if scope_kind == "folder" {
            cohort
                .participants
                .iter()
                .filter(|participant| {
                    !stored
                        .folder_access
                        .get(selected_folder.as_ref().expect("Folder scope"))
                        .is_some_and(|users| users.contains(&participant.npub))
                })
                .count()
        } else {
            0
        };
        let current_folder_access = stored
            .folder_access
            .values()
            .map(BTreeSet::len)
            .sum::<usize>();
        let current_sync = usize::try_from(self.latest_sequence(brain_id)?).unwrap_or(usize::MAX);
        let pending_invitations = {
            let mut statement = self.conn.prepare(
                r#"
                SELECT id, target_kind, folder_only, initial_folder_access_json, expires_at
                FROM brain_invitations
                WHERE brain_id = ?1
                  AND invited_email = ?2
                  AND status = 'pending'
                ORDER BY id
                "#,
            )?;
            let rows = statement.query_map(
                params![
                    brain_id.as_str(),
                    cohort.human_email.trim().to_ascii_lowercase()
                ],
                |row| {
                    let folder_only = row.get::<_, i64>("folder_only")? != 0;
                    let folders_json: String = row.get("initial_folder_access_json")?;
                    let folders = folder_id_vec_from_json(&folders_json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            folders_json.len(),
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                    Ok(AccountCohortReconciliationPendingInvitation {
                        invitation_id: row.get("id")?,
                        target_kind: row.get("target_kind")?,
                        scope_kind: if folder_only { "folder" } else { "brain" }.to_owned(),
                        folder_id: if folder_only {
                            folders.first().cloned()
                        } else {
                            None
                        },
                        expires_at: row.get("expires_at")?,
                        conversion_required: row.get::<_, String>("target_kind")?
                            == "email_bootstrap",
                    })
                },
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let capacity = AccountCohortReconciliationCapacity {
            members_after: stored
                .brain
                .members
                .len()
                .saturating_add(expected_member_additions.len()),
            member_limit: BRAIN_CAPACITY_ENVELOPE.members,
            folder_access_entries_after: current_folder_access
                .saturating_add(folder_access_additions),
            folder_access_entry_limit: BRAIN_CAPACITY_ENVELOPE.folder_access_entries,
            folder_key_grants_after: stored.grants.len().saturating_add(missing_grants),
            folder_key_grant_limit: BRAIN_CAPACITY_ENVELOPE.folder_key_grants,
            sync_records_after: current_sync
                .saturating_add(missing_grants)
                .saturating_add(1),
            sync_record_limit: BRAIN_CAPACITY_ENVELOPE.sync_records,
        };
        let blocker = if already_reconciled {
            Some("already_reconciled".to_owned())
        } else if capacity.members_after > capacity.member_limit
            || capacity.folder_access_entries_after > capacity.folder_access_entry_limit
            || capacity.folder_key_grants_after > capacity.folder_key_grant_limit
            || capacity.sync_records_after > capacity.sync_record_limit
        {
            Some("capacity_exceeded".to_owned())
        } else {
            None
        };
        Ok(AccountCohortReconciliationPlan {
            operation_id: String::new(),
            brain_id: brain_id.clone(),
            account_id: cohort.account_id.clone(),
            human_npub: human.npub.clone(),
            human_email: cohort.human_email.trim().to_ascii_lowercase(),
            scope_kind: scope_kind.to_owned(),
            folder_id: selected_folder,
            roster_revision: cohort.roster_revision,
            participants: cohort.participants.clone(),
            folders,
            pending_invitations,
            expected_member_additions,
            independent_agent_npubs: independent_agents.into_iter().collect(),
            capacity,
            blocker,
        })
    }

    pub fn committed_account_cohort_reconciliation(
        &self,
        operation_id: &str,
    ) -> Result<Option<AccountCohortReconciliationPlan>, StoreError> {
        let json = self
            .conn
            .query_row(
                "SELECT plan_json FROM account_cohort_reconciliation_plans WHERE operation_id = ?1 AND status = 'committed'",
                params![operation_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|json| {
            serde_json::from_str(&json).map_err(|error| StoreError::BrokenInvariant {
                reason: format!("stored reconciliation receipt is invalid: {error}"),
            })
        })
        .transpose()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit_account_cohort_reconciliation(
        &mut self,
        reviewed: &AccountCohortReconciliationPlan,
        cohort: &BootstrapAccountCohort,
        actor: &UserId,
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
        backup_reference: &str,
        now: &str,
    ) -> Result<CommitAccountCohortReconciliationOutcome, StoreError> {
        validate_link_id("cohort_reconciliation_operation_id", &reviewed.operation_id)?;
        if backup_reference.trim().is_empty()
            || backup_reference
                .chars()
                .any(|character| character.is_control())
        {
            return Err(StoreError::BrokenInvariant {
                reason: "reconciliation commit requires an explicit backup reference".to_owned(),
            });
        }
        if self
            .committed_account_cohort_reconciliation(&reviewed.operation_id)?
            .is_some()
        {
            return Ok(CommitAccountCohortReconciliationOutcome::AlreadyCommitted);
        }
        let mut fresh = self.plan_account_cohort_reconciliation(
            &reviewed.brain_id,
            cohort,
            reviewed.folder_id.as_ref(),
            actor,
        )?;
        fresh.operation_id = reviewed.operation_id.clone();
        if fresh != *reviewed || reviewed.blocker.is_some() {
            return Err(StoreError::BrokenInvariant {
                reason: "reconciliation plan is stale or blocked".to_owned(),
            });
        }
        let expected = reviewed
            .folders
            .iter()
            .flat_map(|folder| {
                folder
                    .missing_grant_recipient_npubs
                    .iter()
                    .map(move |recipient| {
                        (
                            folder.folder_id.clone(),
                            folder.key_version,
                            recipient.clone(),
                        )
                    })
            })
            .collect::<BTreeSet<_>>();
        let supplied = grants
            .iter()
            .map(|grant| {
                (
                    grant.folder_id.clone(),
                    grant.key_version,
                    grant.recipient_npub.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        if supplied != expected || grants.len() != expected.len() {
            return Err(StoreError::BrokenInvariant {
                reason: "reconciliation grants must exactly match the reviewed missing set"
                    .to_owned(),
            });
        }
        let stored_before = self.load_brain(&reviewed.brain_id)?;
        for grant in grants {
            validate_grant_metadata(grant)?;
            if grant.issuer_npub != *actor
                || !has_brain_operational_authority(&stored_before, actor)
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "reconciliation grant issuer lacks current authority".to_owned(),
                });
            }
        }
        validate_access_mutation_control_records(grants, control_records, actor)?;
        let cohort_id = format!("cohort-{}", reviewed.operation_id);
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO account_access_cohorts (
                id, brain_id, account_id, human_npub, human_email, scope_kind,
                folder_id, provenance_kind, provenance_id, roster_revision,
                status, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
                      'internal_beta_reconciliation', ?8, ?9, 'active', ?10, ?10)
            "#,
            params![
                cohort_id,
                reviewed.brain_id.as_str(),
                reviewed.account_id,
                reviewed.human_npub.as_str(),
                reviewed.human_email,
                reviewed.scope_kind,
                reviewed.folder_id.as_ref().map(FolderId::as_str),
                reviewed.operation_id,
                i64::try_from(reviewed.roster_revision).map_err(|_| {
                    StoreError::BrokenInvariant {
                        reason: "reconciliation roster revision exceeds SQLite range".to_owned(),
                    }
                })?,
                now,
            ],
        )?;
        for participant in &reviewed.participants {
            tx.execute(
                r#"
                INSERT INTO account_access_cohort_participants (
                    cohort_id, participant_npub, relationship, nip05,
                    display_name, status, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6)
                "#,
                params![
                    cohort_id,
                    participant.npub.as_str(),
                    participant.relationship,
                    participant.nip05,
                    participant.name,
                    now,
                ],
            )?;
            if participant.relationship != "account_agent" {
                continue;
            }
            if reviewed.scope_kind == "brain" {
                if stored_before.brain.kind == BrainKind::Organization {
                    tx.execute(
                        "INSERT OR IGNORE INTO brain_members (brain_id, user_id) VALUES (?1, ?2)",
                        params![reviewed.brain_id.as_str(), participant.npub.as_str()],
                    )?;
                } else {
                    tx.execute(
                        r#"
                        INSERT INTO personal_brain_agents (
                            brain_id, agent_npub, agent_nip05, display_name,
                            status, roster_revision, blocker, created_at, updated_at
                        ) VALUES (?1, ?2, ?3, ?4, 'ready', ?5, NULL, ?6, ?6)
                        ON CONFLICT(brain_id, agent_npub) DO UPDATE SET
                            agent_nip05 = excluded.agent_nip05,
                            display_name = excluded.display_name,
                            status = 'ready', roster_revision = excluded.roster_revision,
                            blocker = NULL, updated_at = excluded.updated_at
                        "#,
                        params![
                            reviewed.brain_id.as_str(),
                            participant.npub.as_str(),
                            participant.nip05,
                            participant.name,
                            i64::try_from(reviewed.roster_revision).map_err(|_| {
                                StoreError::BrokenInvariant {
                                    reason: "reconciliation roster revision exceeds SQLite range"
                                        .to_owned(),
                                }
                            })?,
                            now,
                        ],
                    )?;
                }
            } else if let Some(folder_id) = reviewed.folder_id.as_ref() {
                insert_folder_access_if_missing(
                    &tx,
                    &reviewed.brain_id,
                    folder_id,
                    &participant.npub,
                )?;
                insert_folder_access_source(
                    &tx,
                    &reviewed.brain_id,
                    folder_id,
                    &participant.npub,
                    "direct",
                    &cohort_id,
                    now,
                )?;
            }
            tx.execute(
                r#"
                INSERT INTO human_anchored_agent_authorities (
                    cohort_id, brain_id, human_npub, agent_npub,
                    status, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)
                "#,
                params![
                    cohort_id,
                    reviewed.brain_id.as_str(),
                    reviewed.human_npub.as_str(),
                    participant.npub.as_str(),
                    now,
                ],
            )?;
        }
        if let Some(folder_id) = reviewed.folder_id.as_ref() {
            insert_folder_access_source(
                &tx,
                &reviewed.brain_id,
                folder_id,
                &reviewed.human_npub,
                "direct",
                &cohort_id,
                now,
            )?;
        }
        for grant in grants {
            insert_grant(&tx, &reviewed.brain_id, grant)?;
        }
        sync_records::append_sync_records(&tx, &reviewed.brain_id, control_records)?;
        tx.execute(
            r#"
            INSERT INTO account_access_cohort_audit (
                id, cohort_id, action, actor_npub, anchoring_human_npub,
                detail_json, occurred_at
            ) VALUES (?1, ?2, 'internal_beta_reconciled', ?3, ?4, ?5, ?6)
            "#,
            params![
                format!("audit-{}-reconciled", reviewed.operation_id),
                cohort_id,
                actor.as_str(),
                reviewed.human_npub.as_str(),
                serde_json::json!({
                    "backupReference": backup_reference,
                    "scopeKind": reviewed.scope_kind,
                    "folderId": reviewed.folder_id.as_ref().map(FolderId::as_str),
                    "participantNpubs": reviewed.participants.iter().map(|participant| participant.npub.as_str()).collect::<Vec<_>>(),
                }).to_string(),
                now,
            ],
        )?;
        tx.execute(
            r#"
            INSERT INTO account_cohort_reconciliation_plans (
                operation_id, plan_json, status, created_at, committed_at
            ) VALUES (?1, ?2, 'committed', ?3, ?3)
            "#,
            params![
                reviewed.operation_id,
                serde_json::to_string(reviewed).map_err(|error| StoreError::BrokenInvariant {
                    reason: format!("could not serialize reconciliation receipt: {error}"),
                })?,
                now,
            ],
        )?;
        tx.commit()?;
        Ok(CommitAccountCohortReconciliationOutcome::Committed)
    }
}

fn validate_access_mutation_control_records(
    grants: &[FolderKeyGrantMetadata],
    control_records: &[SyncRecordInput],
    actor: &UserId,
) -> Result<(), StoreError> {
    if control_records.len() != grants.len() + 1 {
        return Err(StoreError::BrokenInvariant {
            reason:
                "peer Agent access change requires grant controls and one access-change control"
                    .to_owned(),
        });
    }
    validate_folder_key_grant_control_records(grants, &control_records[..grants.len()])?;
    sync_records::validate_sync_input(&control_records[grants.len()])?;
    let SyncRecordInput::Control(access_change) = &control_records[grants.len()] else {
        return Err(StoreError::BrokenInvariant {
            reason: "peer Agent access change proof must be a control record".to_owned(),
        });
    };
    if access_change.record_type != SyncRecordType::BrainAdminAccessChange
        || access_change.actor_npub != *actor
    {
        return Err(StoreError::BrokenInvariant {
            reason: "peer Agent access-change control does not match the acting Agent".to_owned(),
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_peer_agent_access_audit(
    tx: &Transaction<'_>,
    cohort_id: &str,
    action: &str,
    actor: &UserId,
    human: &UserId,
    target: &UserId,
    intent: &AuthenticatedHumanIntentRecord,
    now: &str,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO account_access_cohort_audit (
            id, cohort_id, action, actor_npub, anchoring_human_npub,
            detail_json, occurred_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(id) DO NOTHING
        "#,
        params![
            format!("audit-{cohort_id}-{}-{action}", intent.event_id),
            cohort_id,
            action,
            actor.as_str(),
            human.as_str(),
            serde_json::json!({
                "participantNpub": target.as_str(),
                "humanIntentEventId": intent.event_id,
            })
            .to_string(),
            now,
        ],
    )?;
    Ok(())
}
