use crate::*;

impl BrainStore {
    /// Atomically converge an Organization Brain target to member+admin and
    /// apply the client-prepared current Folder Key Grants it supplied.
    ///
    /// The caller has already classified omitted/stale snapshot entries. This
    /// boundary validates every opaque grant and commits the useful state in
    /// one SQLite transaction; no Folder Key is ever interpreted here.
    pub fn ensure_organization_admin_with_grants(
        &mut self,
        brain_id: &BrainId,
        target: &UserId,
        grants: &[(FolderKeyGrantMetadata, SyncRecordInput, SyncRecordInput)],
        admin_record: Option<&SyncRecordInput>,
    ) -> Result<(), StoreError> {
        let mut stored = self.load_brain(brain_id)?;
        if stored.brain.kind != BrainKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "organization collaboration requires an organization brain".to_owned(),
            });
        }
        let target_was_member = stored
            .brain
            .members
            .iter()
            .any(|member| member.user_id == *target);
        let target_was_admin = stored.brain.admins.iter().any(|admin| admin == target);
        if !target_was_member {
            stored.brain.members.push(BrainMember {
                user_id: target.clone(),
                folder_access: BTreeSet::new(),
            });
        }
        if !target_was_admin {
            stored.brain.admins.push(target.clone());
        }

        for (grant, _, _) in grants {
            let folder = stored
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == grant.folder_id)
                .ok_or_else(|| StoreError::MissingFolder {
                    folder_id: grant.folder_id.to_string(),
                })?;
            validate_grant_metadata(grant)?;
            validate_grant_issuer(
                &stored.brain,
                grant,
                None,
                has_brain_operational_authority(&stored, &grant.issuer_npub),
            )?;
            if grant.key_version != folder.current_key_version {
                return Err(StoreError::BrokenInvariant {
                    reason: "collaboration grant key version must match current Folder key version"
                        .to_owned(),
                });
            }
            if grant.recipient_npub != *target {
                return Err(StoreError::BrokenInvariant {
                    reason: "collaboration grant recipient must match target".to_owned(),
                });
            }
            if folder.access == FolderAccessMode::Owner {
                return Err(StoreError::BrokenInvariant {
                    reason: "owner Folders do not accept collaboration grants".to_owned(),
                });
            }
        }

        let tx = self.conn.transaction()?;
        if !target_was_member {
            insert_member_if_missing(&tx, brain_id, target)?;
        }
        if !target_was_admin {
            tx.execute(
                "INSERT OR IGNORE INTO brain_admins (brain_id, user_id) VALUES (?1, ?2)",
                params![brain_id.as_str(), target.as_str()],
            )?;
        }
        for (grant, record, access_record) in grants {
            let already_ready = stored.grants.iter().any(|existing| {
                existing.folder_id == grant.folder_id
                    && existing.key_version == grant.key_version
                    && existing.recipient_npub == grant.recipient_npub
            });
            if already_ready {
                continue;
            }
            let folder = stored
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == grant.folder_id)
                .expect("grant folder validated above");
            if folder.access == FolderAccessMode::Restricted {
                insert_folder_access_if_missing(&tx, brain_id, &grant.folder_id, target)?;
                insert_folder_access_source(
                    &tx,
                    brain_id,
                    &grant.folder_id,
                    target,
                    "direct",
                    "collaborator",
                    &grant.created_at,
                )?;
            }
            insert_grant(&tx, brain_id, grant)?;
            sync_records::validate_sync_conflict(&tx, brain_id, record)?;
            let sequence = sync_records::next_sequence(&tx, brain_id)?;
            sync_records::insert_sync_record(&tx, brain_id, sequence, record)?;
            sync_records::validate_sync_conflict(&tx, brain_id, access_record)?;
            let sequence = sync_records::next_sequence(&tx, brain_id)?;
            sync_records::insert_sync_record(&tx, brain_id, sequence, access_record)?;
        }
        if !target_was_admin && let Some(record) = admin_record {
            sync_records::validate_sync_conflict(&tx, brain_id, record)?;
            let sequence = sync_records::next_sequence(&tx, brain_id)?;
            sync_records::insert_sync_record(&tx, brain_id, sequence, record)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn create_folder(
        &mut self,
        brain_id: &BrainId,
        folder: &Folder,
        access_user_ids: &BTreeSet<UserId>,
        grants: &[FolderKeyGrantMetadata],
    ) -> Result<(), StoreError> {
        self.create_folder_with_control_records(brain_id, folder, access_user_ids, grants, &[])
    }

    /// Create a Folder and append its signed grant/admin records atomically.
    pub fn create_folder_with_control_records(
        &mut self,
        brain_id: &BrainId,
        folder: &Folder,
        access_user_ids: &BTreeSet<UserId>,
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
    ) -> Result<(), StoreError> {
        let was_deleted = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM deleted_folder_identities WHERE brain_id = ?1 AND folder_id = ?2)",
            params![brain_id.as_str(), folder.id.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        if was_deleted {
            return Err(StoreError::BrokenInvariant {
                reason: "deleted Folder identities cannot be reused".to_owned(),
            });
        }
        if folder.current_key_version != 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "new folders must start at key version 1".to_owned(),
            });
        }

        let brain = self.load_core_brain(brain_id)?;
        if brain
            .owner_user_id
            .as_ref()
            .is_some_and(|owner| access_user_ids.contains(owner))
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain owner cannot be an ordinary Folder Guest".to_owned(),
            });
        }
        self.validate_folder_request(&brain, folder, access_user_ids, grants)?;

        let tx = self.conn.transaction()?;
        insert_folder(&tx, brain_id, folder, false)?;
        insert_folder_access(&tx, brain_id, &folder.id, access_user_ids)?;
        for grant in grants {
            insert_grant(&tx, brain_id, grant)?;
        }
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.commit()?;
        Ok(())
    }

    /// Insert an empty legacy Folder that can later be repaired by Finish Setup.
    pub fn insert_setup_incomplete_folder_for_repair(
        &mut self,
        brain_id: &BrainId,
        folder: &Folder,
        access_user_ids: &BTreeSet<UserId>,
    ) -> Result<(), StoreError> {
        let was_deleted = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM deleted_folder_identities WHERE brain_id = ?1 AND folder_id = ?2)",
            params![brain_id.as_str(), folder.id.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        if was_deleted {
            return Err(StoreError::BrokenInvariant {
                reason: "deleted Folder identities cannot be reused".to_owned(),
            });
        }
        validate_hierarchy(&self.conn, brain_id, folder)?;

        let tx = self.conn.transaction()?;
        insert_folder(&tx, brain_id, folder, true)?;
        insert_folder_access(&tx, brain_id, &folder.id, access_user_ids)?;
        tx.commit()?;
        Ok(())
    }

    /// Finish setup for an empty Folder by writing the required current grants.
    pub fn finish_folder_setup(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        grants: &[FolderKeyGrantMetadata],
    ) -> Result<(), StoreError> {
        self.finish_folder_setup_with_control_records(brain_id, folder_id, grants, &[])
    }

    /// Finish Folder setup and append its signed grant/admin records atomically.
    pub fn finish_folder_setup_with_control_records(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
    ) -> Result<(), StoreError> {
        let stored = self.load_brain(brain_id)?;
        let folder = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == *folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: folder_id.to_string(),
            })?;

        if !stored.setup_incomplete_folder_ids.contains(folder_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "folder setup is already complete".to_owned(),
            });
        }
        if self
            .load_current_objects(brain_id)?
            .iter()
            .any(|object| object.folder_id == *folder_id)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "finish setup only supports empty folders".to_owned(),
            });
        }

        let access_user_ids = stored
            .folder_access
            .get(folder_id)
            .cloned()
            .unwrap_or_default();
        let personal_agent = stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub);
        let mut required =
            required_recipients(&stored.brain, folder, &access_user_ids, personal_agent)?;
        extend_account_agent_recipients(&mut required, &stored, folder_id);
        validate_folder_grants(
            &stored.brain,
            folder,
            &required,
            grants,
            personal_agent,
            grants
                .iter()
                .all(|grant| has_brain_operational_authority(&stored, &grant.issuer_npub)),
        )?;

        let tx = self.conn.transaction()?;
        for grant in grants {
            insert_grant(&tx, brain_id, grant)?;
        }
        tx.execute(
            "UPDATE folders SET setup_incomplete = 0 WHERE brain_id = ?1 AND id = ?2",
            params![brain_id.as_str(), folder_id.as_str()],
        )?;
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        tx.commit()?;
        Ok(())
    }

    /// Grant the current Folder Key and append its signed control records atomically.
    ///
    /// Explicit Folder grants are orthogonal to native access mode. Targets
    /// without Brain Membership become Guests scoped to this Folder.
    pub fn grant_folder_access_with_control_records(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        user_id: &UserId,
        grant: &FolderKeyGrantMetadata,
        control_records: &[SyncRecordInput],
    ) -> Result<GrantFolderAccessOutcome, StoreError> {
        let stored = self.load_brain(brain_id)?;
        let folder = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == *folder_id)
            .cloned()
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: folder_id.to_string(),
            })?;
        if stored.brain.owner_user_id.as_ref() == Some(user_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain owner cannot be an ordinary Folder Guest".to_owned(),
            });
        }
        validate_grant_metadata(grant)?;
        validate_grant_issuer(
            &stored.brain,
            grant,
            stored
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
            has_brain_operational_authority(&stored, &grant.issuer_npub),
        )?;
        if grant.folder_id != *folder_id {
            return Err(StoreError::BrokenInvariant {
                reason: "grant folder id must match folder metadata".to_owned(),
            });
        }
        if grant.key_version != folder.current_key_version {
            return Err(StoreError::BrokenInvariant {
                reason: "grant key version must match folder current key version".to_owned(),
            });
        }
        if grant.recipient_npub != *user_id {
            return Err(StoreError::BrokenInvariant {
                reason: "grant recipient must match folder access target".to_owned(),
            });
        }

        let current_access = stored
            .folder_access
            .get(folder_id)
            .cloned()
            .unwrap_or_default();
        let mut effective_access = required_recipients(
            &stored.brain,
            &folder,
            &current_access,
            stored
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
        )?;
        extend_account_agent_recipients(&mut effective_access, &stored, folder_id);
        let current_grant_exists = stored.grants.iter().any(|existing| {
            existing.folder_id == *folder_id
                && existing.key_version == folder.current_key_version
                && existing.recipient_npub == *user_id
        });
        let is_admin = stored.brain.admins.iter().any(|admin| admin == user_id);
        let is_member = stored
            .brain
            .members
            .iter()
            .any(|member| member.user_id == *user_id);
        let has_native_access = match folder.access {
            FolderAccessMode::Owner => stored.brain.owner_user_id.as_ref() == Some(user_id),
            FolderAccessMode::AdminOnly => is_admin,
            FolderAccessMode::AllMembers => is_admin || is_member,
            FolderAccessMode::Restricted => is_admin,
        };
        let direct_source_exists = folder_access_has_source(
            &self.conn,
            brain_id,
            folder_id,
            user_id,
            "direct",
            "folder-access",
        )?;
        if effective_access.contains(user_id)
            && current_grant_exists
            && (has_native_access || direct_source_exists)
        {
            return Ok(GrantFolderAccessOutcome::AlreadyHasAccess);
        }

        validate_folder_grant_control_records(folder_id, grant, control_records)?;

        let inserts_access_row = !has_native_access && !current_access.contains(user_id);

        let tx = self.conn.transaction()?;
        if inserts_access_row {
            tx.execute(
                "INSERT INTO folder_access (brain_id, folder_id, user_id) VALUES (?1, ?2, ?3)",
                params![brain_id.as_str(), folder_id.as_str(), user_id.as_str()],
            )?;
        }
        if !has_native_access {
            insert_folder_access_source(
                &tx,
                brain_id,
                folder_id,
                user_id,
                "direct",
                "folder-access",
                &grant.created_at,
            )?;
        }
        if !current_grant_exists {
            insert_grant(&tx, brain_id, grant)?;
        }
        for input in control_records {
            sync_records::validate_sync_conflict(&tx, brain_id, input)?;
            let sequence = sync_records::next_sequence(&tx, brain_id)?;
            sync_records::insert_sync_record(&tx, brain_id, sequence, input)?;
        }
        tx.commit()?;
        Ok(GrantFolderAccessOutcome::Granted)
    }

    /// Restore one ready Personal Brain peer Agent to a previously excluded
    /// Folder. The human intent and grant become durable in the same commit.
    #[allow(clippy::too_many_arguments)]
    pub fn restore_personal_agent_folder_access_with_control_records(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        agent_npub: &UserId,
        grant: &FolderKeyGrantMetadata,
        control_records: &[SyncRecordInput],
        intent: &AuthenticatedHumanIntentRecord,
        updated_at: &str,
    ) -> Result<(), StoreError> {
        let stored = self.load_brain(brain_id)?;
        let owner =
            stored
                .brain
                .owner_user_id
                .as_ref()
                .ok_or_else(|| StoreError::BrokenInvariant {
                    reason: "peer Agent restoration requires a Personal Brain".to_owned(),
                })?;
        if stored.brain.kind != BrainKind::Personal
            || intent.human_npub != *owner
            || intent.acting_agent_npub == *agent_npub
            || intent.target_agent_npub != *agent_npub
            || intent.operation != "restore"
            || intent.scope_kind != "folder"
            || intent.folder_id.as_ref() != Some(folder_id)
            || !stored
                .personal_brain_agents
                .iter()
                .any(|agent| agent.agent_npub == *agent_npub && agent.status == "ready")
            || !stored
                .account_agent_exclusions
                .contains(&(agent_npub.clone(), folder_id.to_string()))
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain peer Agent restoration is not authorized for this scope"
                    .to_owned(),
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
        validate_grant_metadata(grant)?;
        if grant.folder_id != *folder_id
            || grant.key_version != folder.current_key_version
            || grant.recipient_npub != *agent_npub
            || grant.issuer_npub != intent.acting_agent_npub
            || !has_brain_operational_authority(&stored, &grant.issuer_npub)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "peer Agent restoration grant does not match current authority and key"
                    .to_owned(),
            });
        }
        if control_records.len() != 2 {
            return Err(StoreError::BrokenInvariant {
                reason: "peer Agent restoration requires grant and access-change records"
                    .to_owned(),
            });
        }
        validate_folder_key_grant_control_records(
            std::slice::from_ref(grant),
            &control_records[..1],
        )?;
        let tx = self.conn.transaction()?;
        consume_authenticated_human_intent(&tx, brain_id, intent)?;
        let changed = tx.execute(
            r#"
            UPDATE account_access_cohort_exclusions
            SET active = 0, updated_at = ?4
            WHERE participant_npub = ?2 AND folder_id = ?3 AND active = 1
              AND cohort_id IN (
                  SELECT id FROM account_access_cohorts
                  WHERE brain_id = ?1 AND status = 'active'
              )
            "#,
            params![
                brain_id.as_str(),
                agent_npub.as_str(),
                folder_id.as_str(),
                updated_at
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::BrokenInvariant {
                reason: "peer Agent Folder exclusion was already cleared".to_owned(),
            });
        }
        insert_grant(&tx, brain_id, grant)?;
        sync_records::append_sync_records(&tx, brain_id, control_records)?;
        for cohort in stored.account_access_cohorts.iter().filter(|cohort| {
            cohort.status == "active"
                && cohort.participants.iter().any(|participant| {
                    participant.npub == *agent_npub && participant.relationship == "account_agent"
                })
        }) {
            tx.execute(
                r#"
                INSERT INTO account_access_cohort_audit (
                    id, cohort_id, action, actor_npub, anchoring_human_npub,
                    detail_json, occurred_at
                ) VALUES (?1, ?2, 'participant_folder_restored', ?3, ?4, ?5, ?6)
                ON CONFLICT(id) DO NOTHING
                "#,
                params![
                    format!(
                        "audit-{}-{}-{}-restored",
                        cohort.cohort_id, folder_id, intent.event_id
                    ),
                    cohort.cohort_id,
                    intent.acting_agent_npub.as_str(),
                    intent.human_npub.as_str(),
                    serde_json::json!({
                        "folderId": folder_id.as_str(),
                        "participantNpub": agent_npub.as_str(),
                        "humanIntentEventId": intent.event_id,
                    })
                    .to_string(),
                    updated_at,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove explicit Folder access by rotating the Folder Key and re-encrypting live objects.
    #[allow(clippy::too_many_arguments)]
    pub fn rotate_folder_key_for_access_removal(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        removed_user_id: &UserId,
        new_key_version: u32,
        grants: &[FolderKeyGrantMetadata],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        updated_at: &str,
    ) -> Result<(), StoreError> {
        self.rotate_folder_key_for_access_removal_with_control_records(
            brain_id,
            folder_id,
            removed_user_id,
            new_key_version,
            grants,
            reencrypted_records,
            updated_at,
            &[],
        )
    }

    /// Remove explicit Folder access and append signed grant/admin records atomically.
    #[allow(clippy::too_many_arguments)]
    pub fn rotate_folder_key_for_access_removal_with_control_records(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        removed_user_id: &UserId,
        new_key_version: u32,
        grants: &[FolderKeyGrantMetadata],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        updated_at: &str,
        control_records: &[SyncRecordInput],
    ) -> Result<(), StoreError> {
        self.rotate_folder_key_for_access_removal_with_control_records_and_intent(
            brain_id,
            folder_id,
            removed_user_id,
            new_key_version,
            grants,
            reencrypted_records,
            updated_at,
            control_records,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rotate_folder_key_for_access_removal_with_control_records_and_intent(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        removed_user_id: &UserId,
        new_key_version: u32,
        grants: &[FolderKeyGrantMetadata],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        updated_at: &str,
        control_records: &[SyncRecordInput],
        authenticated_human_intent: Option<&AuthenticatedHumanIntentRecord>,
    ) -> Result<(), StoreError> {
        validate_folder_rotation_fanout(
            FolderRotationOperation::FolderAccessRemoval,
            [FolderRotationFanout {
                grants: grants.len(),
                reencrypted_records: reencrypted_records.len(),
            }],
        )?;
        let stored = self.load_brain(brain_id)?;
        let folder = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == *folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: folder_id.to_string(),
            })?;
        if new_key_version != folder.current_key_version + 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "folder access removal must rotate to the next key version".to_owned(),
            });
        }
        let mut remaining_access = stored
            .folder_access
            .get(folder_id)
            .cloned()
            .unwrap_or_default();
        let had_explicit_access = remaining_access.remove(removed_user_id);
        if folder_access_has_mount_source(&self.conn, brain_id, folder_id, removed_user_id)? {
            return Err(StoreError::BrokenInvariant {
                reason: "Folder access target still receives this Folder through an active Mount; remove the target from that Mount before rotating explicit Folder access"
                    .to_owned(),
            });
        }

        let applicable_cohort_ids = stored
            .account_access_cohorts
            .iter()
            .filter(|cohort| {
                cohort.status == "active"
                    && (cohort.scope_kind == "brain"
                        || cohort.folder_id.as_ref() == Some(folder_id))
                    && cohort.participants.iter().any(|participant| {
                        participant.npub == *removed_user_id
                            && participant.relationship == "account_agent"
                            && participant.status == "active"
                    })
            })
            .map(|cohort| cohort.cohort_id.clone())
            .collect::<Vec<_>>();
        if !had_explicit_access && applicable_cohort_ids.is_empty() {
            return Err(StoreError::BrokenInvariant {
                reason: "folder access target does not currently have access".to_owned(),
            });
        }

        let mut rotated_folder = folder.clone();
        rotated_folder.current_key_version = new_key_version;
        let mut post_removal = stored.clone();
        if !applicable_cohort_ids.is_empty() {
            post_removal
                .account_agent_exclusions
                .insert((removed_user_id.clone(), folder_id.to_string()));
        }
        let personal_agent = stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub);
        let mut required = required_recipients(
            &stored.brain,
            &rotated_folder,
            &remaining_access,
            personal_agent,
        )?;
        extend_account_agent_recipients(&mut required, &post_removal, folder_id);
        validate_folder_grants(
            &stored.brain,
            &rotated_folder,
            &required,
            grants,
            personal_agent,
            grants
                .iter()
                .all(|grant| has_brain_operational_authority(&stored, &grant.issuer_npub)),
        )?;

        let live_objects = self
            .load_current_objects(brain_id)?
            .into_iter()
            .filter(|object| object.folder_id == *folder_id && !object.deleted)
            .collect::<Vec<_>>();
        validate_rotation_records(&live_objects, reencrypted_records)?;

        let tx = self.conn.transaction()?;
        if let Some(intent) = authenticated_human_intent {
            consume_authenticated_human_intent(&tx, brain_id, intent)?;
        }
        tx.execute(
            "DELETE FROM folder_access WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3",
            params![
                brain_id.as_str(),
                folder_id.as_str(),
                removed_user_id.as_str()
            ],
        )?;
        for cohort_id in &applicable_cohort_ids {
            tx.execute(
                r#"
                INSERT INTO account_access_cohort_exclusions (
                    cohort_id, participant_npub, folder_id, reason, active,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, 'targeted_folder_revocation', 1, ?4, ?4)
                ON CONFLICT(cohort_id, participant_npub, folder_id) DO UPDATE SET
                    reason = excluded.reason,
                    active = 1,
                    updated_at = excluded.updated_at
                "#,
                params![
                    cohort_id,
                    removed_user_id.as_str(),
                    folder_id.as_str(),
                    updated_at
                ],
            )?;
            tx.execute(
                r#"
                INSERT INTO account_access_cohort_audit (
                    id, cohort_id, action, actor_npub, anchoring_human_npub,
                    detail_json, occurred_at
                )
                SELECT ?1, ?2, 'participant_folder_revoked', ?3, human_npub,
                       ?4, ?5
                FROM account_access_cohorts WHERE id = ?2
                ON CONFLICT(id) DO NOTHING
                "#,
                params![
                    format!(
                        "audit-{cohort_id}-{}-{}-revoked-v{new_key_version}",
                        folder_id, removed_user_id
                    ),
                    cohort_id,
                    grants
                        .first()
                        .map(|grant| grant.issuer_npub.as_str())
                        .unwrap_or(removed_user_id.as_str()),
                    serde_json::json!({
                        "folderId": folder_id.as_str(),
                        "participantNpub": removed_user_id.as_str(),
                        "newKeyVersion": new_key_version,
                    })
                    .to_string(),
                    updated_at,
                ],
            )?;
        }
        tx.execute(
            "UPDATE folders SET current_key_version = ?3 WHERE brain_id = ?1 AND id = ?2",
            params![brain_id.as_str(), folder_id.as_str(), new_key_version],
        )?;
        invalidate_pending_email_bootstraps_for_rotated_folder(
            &tx, brain_id, folder_id, updated_at,
        )?;
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

    /// Remove a Member and every readable Folder relationship in one transaction.
    pub fn remove_member_with_rotations(
        &mut self,
        brain_id: &BrainId,
        actor_user_id: &UserId,
        removed_user_id: &UserId,
        rotations: &[MemberFolderRotation],
        mount_rotations: &[MemberMountRotation],
        updated_at: &str,
    ) -> Result<(), StoreError> {
        self.remove_member_with_rotations_and_control_records(
            brain_id,
            actor_user_id,
            removed_user_id,
            rotations,
            mount_rotations,
            updated_at,
            &BTreeMap::new(),
        )
    }

    pub fn member_removal_participants(
        &self,
        brain_id: &BrainId,
        target: &UserId,
    ) -> Result<BTreeSet<UserId>, StoreError> {
        let stored = self.load_brain(brain_id)?;
        self.member_removal_participants_preserving_independent(&stored, target)
    }

    pub fn member_removal_access_plan(
        &self,
        brain_id: &BrainId,
        target: &UserId,
    ) -> Result<MemberRemovalAccessPlan, StoreError> {
        let stored = self.load_brain(brain_id)?;
        self.member_removal_access_plan_for_stored(&stored, target)
    }

    /// Remove a Member and append every affected Brain's signed control records atomically.
    #[allow(clippy::too_many_arguments)]
    pub fn remove_member_with_rotations_and_control_records(
        &mut self,
        brain_id: &BrainId,
        actor_user_id: &UserId,
        removed_user_id: &UserId,
        rotations: &[MemberFolderRotation],
        mount_rotations: &[MemberMountRotation],
        updated_at: &str,
        control_records_by_brain: &BTreeMap<BrainId, Vec<SyncRecordInput>>,
    ) -> Result<(), StoreError> {
        validate_folder_rotation_fanout(
            FolderRotationOperation::MemberRemoval,
            rotations
                .iter()
                .map(|rotation| FolderRotationFanout {
                    grants: rotation.grants.len(),
                    reencrypted_records: rotation.reencrypted_records.len(),
                })
                .chain(mount_rotations.iter().map(|rotation| FolderRotationFanout {
                    grants: rotation.grants.len(),
                    reencrypted_records: rotation.reencrypted_records.len(),
                })),
        )?;
        let stored = self.load_brain(brain_id)?;
        let access_plan = self.member_removal_access_plan_for_stored(&stored, removed_user_id)?;
        let removed_user_ids = access_plan.removed_members;
        let folder_access_removals = access_plan.folder_access_removals;
        if removed_user_ids
            .iter()
            .any(|removed| stored.brain.admins.contains(removed))
        {
            return Err(StoreError::BrokenInvariant {
                reason: "remove admin role before removing member".to_owned(),
            });
        }
        if removed_user_ids.iter().any(|removed| {
            !stored
                .brain
                .members
                .iter()
                .any(|member| member.user_id == *removed)
        }) {
            return Err(StoreError::BrokenInvariant {
                reason: "brain member does not exist".to_owned(),
            });
        }
        let mut post_removal_brain = stored.brain.clone();
        post_removal_brain
            .members
            .retain(|member| !removed_user_ids.contains(&member.user_id));
        let mut post_removal = stored.clone();
        post_removal.brain = post_removal_brain.clone();
        for (folder_id, removed_access) in &folder_access_removals {
            if let Some(access) = post_removal.folder_access.get_mut(folder_id) {
                access.retain(|participant| !removed_access.contains(participant));
            }
        }
        for cohort in &mut post_removal.account_access_cohorts {
            if cohort.status == "active"
                && cohort.scope_kind == "brain"
                && cohort.participants.iter().any(|participant| {
                    participant.relationship == "human"
                        && participant.status == "active"
                        && participant.npub == *removed_user_id
                })
            {
                cohort.status = "revoked".to_owned();
                for participant in &mut cohort.participants {
                    participant.status = "revoked".to_owned();
                }
            }
        }
        let personal_agent = stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub);
        let mut expected_folders = BTreeSet::new();
        for folder in &stored.brain.folders {
            let access = stored
                .folder_access
                .get(&folder.id)
                .cloned()
                .unwrap_or_default();
            if required_recipients(&stored.brain, folder, &access, personal_agent)?
                .iter()
                .any(|recipient| removed_user_ids.contains(recipient))
                || folder_access_removals
                    .get(&folder.id)
                    .is_some_and(|participants| !participants.is_empty())
            {
                expected_folders.insert(folder.id.clone());
            }
        }
        let supplied_folders = rotations
            .iter()
            .map(|rotation| rotation.folder_id.clone())
            .collect::<BTreeSet<_>>();
        if expected_folders != supplied_folders || supplied_folders.len() != rotations.len() {
            return Err(StoreError::BrokenInvariant {
                reason: "member removal requires exactly one rotation for every readable Folder"
                    .to_owned(),
            });
        }
        let mut expected_mounts = BTreeSet::new();
        for removed in &removed_user_ids {
            expected_mounts.extend(
                self.list_active_destination_mounts_for_participant(brain_id, removed)?
                    .into_iter()
                    .map(|connection| connection.id),
            );
        }
        let supplied_mounts = mount_rotations
            .iter()
            .map(|rotation| rotation.connection_id.clone())
            .collect::<BTreeSet<_>>();
        if expected_mounts != supplied_mounts || supplied_mounts.len() != mount_rotations.len() {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "member removal requires exactly one rotation for every active Mount participation"
                        .to_owned(),
            });
        }
        let current_objects = self.load_current_objects(brain_id)?;
        for rotation in rotations {
            let folder = stored
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == rotation.folder_id)
                .ok_or_else(|| StoreError::MissingFolder {
                    folder_id: rotation.folder_id.to_string(),
                })?;
            if rotation.new_key_version != folder.current_key_version + 1 {
                return Err(StoreError::BrokenInvariant {
                    reason: "member removal must rotate to each Folder's next key version"
                        .to_owned(),
                });
            }
            let mut remaining_access = stored
                .folder_access
                .get(&rotation.folder_id)
                .cloned()
                .unwrap_or_default();
            if let Some(removed_access) = folder_access_removals.get(&rotation.folder_id) {
                remaining_access.retain(|recipient| !removed_access.contains(recipient));
            }
            let mut rotated_folder = folder.clone();
            rotated_folder.current_key_version = rotation.new_key_version;
            let mut required = required_recipients(
                &post_removal_brain,
                &rotated_folder,
                &remaining_access,
                personal_agent,
            )?;
            extend_account_agent_recipients(&mut required, &post_removal, &folder.id);
            validate_folder_grants(
                &post_removal_brain,
                &rotated_folder,
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
                .filter(|object| object.folder_id == rotation.folder_id && !object.deleted)
                .cloned()
                .collect::<Vec<_>>();
            validate_rotation_records(&live_objects, &rotation.reencrypted_records)?;
        }
        let mut mounted_removals = Vec::with_capacity(mount_rotations.len());
        let mut rotated_sources = BTreeSet::new();
        for rotation in mount_rotations {
            let connection = self.load_shared_folder_connection(&rotation.connection_id)?;
            if connection.destination_brain_id != *brain_id
                || connection.status != SharedFolderConnectionStatus::Active
                || !connection
                    .member_npubs
                    .iter()
                    .any(|member| removed_user_ids.contains(member))
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "member removal Mount rotation does not match active destination participation"
                        .to_owned(),
                });
            }
            let must_revoke = removed_user_ids.contains(&connection.destination_admin_npub);
            if rotation.revoke_mount != must_revoke {
                return Err(StoreError::BrokenInvariant {
                    reason: if must_revoke {
                        "removing a Mount's destination controller must revoke the Mount"
                    } else {
                        "removing a non-controller Member must preserve the Mount"
                    }
                    .to_owned(),
                });
            }
            if !rotated_sources.insert((
                connection.source_brain_id.clone(),
                connection.source_folder_id.clone(),
            )) {
                return Err(StoreError::BrokenInvariant {
                    reason: "member removal cannot rotate the same mounted source Folder twice"
                        .to_owned(),
                });
            }
            if connection.source_brain_id == *brain_id
                && supplied_folders.contains(&connection.source_folder_id)
            {
                return Err(StoreError::BrokenInvariant {
                    reason:
                        "member removal cannot rotate one Folder as both native and mounted access"
                            .to_owned(),
                });
            }
            let removed = if must_revoke {
                connection.managed_access_npubs.clone()
            } else {
                connection
                    .managed_access_npubs
                    .intersection(&removed_user_ids)
                    .cloned()
                    .collect()
            };
            if removed.is_empty() {
                mounted_removals.push((connection, removed));
                continue;
            }
            let source = self.load_brain(&connection.source_brain_id)?;
            let folder = source
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == connection.source_folder_id)
                .ok_or_else(|| StoreError::MissingFolder {
                    folder_id: connection.source_folder_id.to_string(),
                })?;
            if rotation.new_key_version != folder.current_key_version + 1 {
                return Err(StoreError::BrokenInvariant {
                    reason: "member removal Mount rotation must use the next key version"
                        .to_owned(),
                });
            }
            let mut remaining_access = source
                .folder_access
                .get(&connection.source_folder_id)
                .cloned()
                .unwrap_or_default();
            for target in &removed {
                if !remaining_access.remove(target) {
                    return Err(StoreError::BrokenInvariant {
                        reason: "member removal Mount target does not have source Folder access"
                            .to_owned(),
                    });
                }
            }
            let mut rotated_folder = folder.clone();
            rotated_folder.current_key_version = rotation.new_key_version;
            let mut required = required_recipients(
                &source.brain,
                &rotated_folder,
                &remaining_access,
                source
                    .personal_agent
                    .as_ref()
                    .map(|relationship| &relationship.agent_npub),
            )?;
            extend_account_agent_recipients(&mut required, &source, &folder.id);
            validate_connection_rotation_grants(
                &rotated_folder,
                &required,
                &rotation.grants,
                actor_user_id,
            )?;
            let live_objects = self
                .load_current_objects(&connection.source_brain_id)?
                .into_iter()
                .filter(|object| object.folder_id == connection.source_folder_id && !object.deleted)
                .collect::<Vec<_>>();
            validate_rotation_records(&live_objects, &rotation.reencrypted_records)?;
            mounted_removals.push((connection, removed));
        }

        let tx = self.conn.transaction()?;
        let revoked_invitation_sources = stored
            .account_access_cohorts
            .iter()
            .filter(|cohort| {
                cohort.status == "active"
                    && cohort.scope_kind == "brain"
                    && cohort.participants.iter().any(|participant| {
                        participant.relationship == "human"
                            && participant.status == "active"
                            && participant.npub == *removed_user_id
                    })
                    && cohort.provenance_kind == "invitation"
            })
            .map(|cohort| cohort.provenance_id.clone())
            .collect::<BTreeSet<_>>();
        for source_id in &revoked_invitation_sources {
            tx.execute(
                "DELETE FROM folder_access_sources WHERE brain_id = ?1 AND source_kind = 'invitation' AND source_id = ?2",
                params![brain_id.as_str(), source_id],
            )?;
        }
        tx.execute(
            r#"
            DELETE FROM folder_access
            WHERE brain_id = ?1
              AND NOT EXISTS (
                  SELECT 1 FROM folder_access_sources source
                  WHERE source.brain_id = folder_access.brain_id
                    AND source.folder_id = folder_access.folder_id
                    AND source.user_id = folder_access.user_id
              )
            "#,
            params![brain_id.as_str()],
        )?;
        for rotation in rotations {
            tx.execute(
                "DELETE FROM folder_access WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3",
                params![
                    brain_id.as_str(),
                    rotation.folder_id.as_str(),
                    removed_user_id.as_str()
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
                updated_at,
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
        for (rotation, (connection, removed)) in mount_rotations.iter().zip(mounted_removals.iter())
        {
            if removed.is_empty() {
                if rotation.revoke_mount {
                    tx.execute(
                        "UPDATE shared_folder_connections SET status = 'revoked', updated_at = ?2 WHERE id = ?1",
                        params![rotation.connection_id, updated_at],
                    )?;
                } else {
                    tx.execute(
                        "DELETE FROM shared_folder_connection_members WHERE connection_id = ?1 AND member_npub = ?2",
                        params![rotation.connection_id, removed_user_id.as_str()],
                    )?;
                }
                continue;
            }
            for target in removed {
                tx.execute(
                    "DELETE FROM folder_access WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3",
                    params![
                        connection.source_brain_id.as_str(),
                        connection.source_folder_id.as_str(),
                        target.as_str()
                    ],
                )?;
            }
            tx.execute(
                "UPDATE folders SET current_key_version = ?3 WHERE brain_id = ?1 AND id = ?2",
                params![
                    connection.source_brain_id.as_str(),
                    connection.source_folder_id.as_str(),
                    rotation.new_key_version
                ],
            )?;
            invalidate_pending_email_bootstraps_for_rotated_folder(
                &tx,
                &connection.source_brain_id,
                &connection.source_folder_id,
                updated_at,
            )?;
            for grant in &rotation.grants {
                insert_grant(&tx, &connection.source_brain_id, grant)?;
            }
            for record in &rotation.reencrypted_records {
                let input = SyncRecordInput::FolderObjectRevision(record.clone());
                sync_records::validate_sync_input(&input)?;
                sync_records::validate_sync_conflict(&tx, &connection.source_brain_id, &input)?;
                let sequence = sync_records::next_sequence(&tx, &connection.source_brain_id)?;
                sync_records::insert_sync_record(
                    &tx,
                    &connection.source_brain_id,
                    sequence,
                    &input,
                )?;
                sync_records::project_sync_record(&tx, &connection.source_brain_id, &input)?;
            }
            if rotation.revoke_mount {
                tx.execute(
                    "UPDATE shared_folder_connections SET status = 'revoked', updated_at = ?2 WHERE id = ?1",
                    params![rotation.connection_id, updated_at],
                )?;
            } else {
                for removed in &removed_user_ids {
                    tx.execute(
                        "DELETE FROM shared_folder_connection_members WHERE connection_id = ?1 AND member_npub = ?2",
                        params![rotation.connection_id, removed.as_str()],
                    )?;
                }
            }
        }
        for removed in &removed_user_ids {
            tx.execute(
                "DELETE FROM brain_member_independent_sources WHERE brain_id = ?1 AND user_id = ?2",
                params![brain_id.as_str(), removed.as_str()],
            )?;
            tx.execute(
                "DELETE FROM folder_access WHERE brain_id = ?1 AND user_id = ?2",
                params![brain_id.as_str(), removed.as_str()],
            )?;
            tx.execute(
                "DELETE FROM brain_members WHERE brain_id = ?1 AND user_id = ?2",
                params![brain_id.as_str(), removed.as_str()],
            )?;
        }
        let removed_human_cohort_ids = stored
            .account_access_cohorts
            .iter()
            .filter(|cohort| {
                cohort.status == "active"
                    && cohort.scope_kind == "brain"
                    && cohort.participants.iter().any(|participant| {
                        participant.relationship == "human"
                            && participant.status == "active"
                            && participant.npub == *removed_user_id
                    })
            })
            .map(|cohort| cohort.cohort_id.clone())
            .collect::<Vec<_>>();
        for cohort_id in &removed_human_cohort_ids {
            tx.execute(
                "UPDATE account_access_cohorts SET status = 'revoked', updated_at = ?2 WHERE id = ?1",
                params![cohort_id, updated_at],
            )?;
            tx.execute(
                "UPDATE account_access_cohort_participants SET status = 'revoked', exclusion_reason = 'anchoring_human_removed', updated_at = ?2 WHERE cohort_id = ?1",
                params![cohort_id, updated_at],
            )?;
            tx.execute(
                "UPDATE human_anchored_agent_authorities SET status = 'revoked', updated_at = ?2 WHERE cohort_id = ?1",
                params![cohort_id, updated_at],
            )?;
        }
        tx.execute(
            r#"
            UPDATE account_access_cohort_participants
            SET status = 'revoked', exclusion_reason = 'targeted_brain_revocation',
                updated_at = ?3
            WHERE participant_npub = ?2
              AND relationship = 'account_agent'
              AND cohort_id IN (
                  SELECT id FROM account_access_cohorts
                  WHERE brain_id = ?1 AND status = 'active'
              )
            "#,
            params![brain_id.as_str(), removed_user_id.as_str(), updated_at],
        )?;
        tx.execute(
            r#"
            UPDATE human_anchored_agent_authorities
            SET status = 'revoked', updated_at = ?3
            WHERE brain_id = ?1 AND agent_npub = ?2 AND status = 'active'
            "#,
            params![brain_id.as_str(), removed_user_id.as_str(), updated_at],
        )?;
        tx.execute(
            r#"
            INSERT OR IGNORE INTO account_access_cohort_exclusions (
                cohort_id, participant_npub, folder_id, reason, active,
                created_at, updated_at
            )
            SELECT participant.cohort_id, participant.participant_npub, '',
                   'targeted_brain_revocation', 1, ?3, ?3
            FROM account_access_cohort_participants participant
            JOIN account_access_cohorts cohort ON cohort.id = participant.cohort_id
            WHERE cohort.brain_id = ?1
              AND participant.participant_npub = ?2
              AND participant.relationship = 'account_agent'
            "#,
            params![brain_id.as_str(), removed_user_id.as_str(), updated_at],
        )?;
        for (record_brain_id, control_records) in control_records_by_brain {
            sync_records::append_sync_records(&tx, record_brain_id, control_records)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn member_removal_participants_preserving_independent(
        &self,
        stored: &StoredBrain,
        target: &UserId,
    ) -> Result<BTreeSet<UserId>, StoreError> {
        let target_cohorts = stored
            .account_access_cohorts
            .iter()
            .filter(|cohort| {
                cohort.status == "active"
                    && cohort.scope_kind == "brain"
                    && cohort.participants.iter().any(|participant| {
                        participant.relationship == "human"
                            && participant.status == "active"
                            && participant.npub == *target
                    })
            })
            .map(|cohort| cohort.cohort_id.as_str())
            .collect::<BTreeSet<_>>();
        let candidates = member_removal_participants(stored, target);
        let mut removed = BTreeSet::new();
        for participant in candidates {
            if &participant == target {
                removed.insert(participant);
                continue;
            }
            let independent = self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM brain_member_independent_sources WHERE brain_id = ?1 AND user_id = ?2)",
                    params![stored.brain.id.as_str(), participant.as_str()],
                    |row| row.get::<_, bool>(0),
                )?;
            let other_cohort = stored.account_access_cohorts.iter().any(|cohort| {
                cohort.status == "active"
                    && cohort.scope_kind == "brain"
                    && !target_cohorts.contains(cohort.cohort_id.as_str())
                    && cohort.participants.iter().any(|candidate| {
                        candidate.relationship == "account_agent"
                            && candidate.status == "active"
                            && candidate.npub == participant
                    })
            });
            let independent_admin = stored.brain.admins.contains(&participant);
            if !independent && !other_cohort && !independent_admin {
                removed.insert(participant);
            }
        }
        Ok(removed)
    }

    fn member_removal_access_plan_for_stored(
        &self,
        stored: &StoredBrain,
        target: &UserId,
    ) -> Result<MemberRemovalAccessPlan, StoreError> {
        let removed_members =
            self.member_removal_participants_preserving_independent(stored, target)?;
        let target_cohorts = stored
            .account_access_cohorts
            .iter()
            .filter(|cohort| {
                cohort.status == "active"
                    && cohort.scope_kind == "brain"
                    && cohort.participants.iter().any(|participant| {
                        participant.relationship == "human"
                            && participant.status == "active"
                            && participant.npub == *target
                    })
            })
            .collect::<Vec<_>>();
        let retained_cohort_agents = target_cohorts
            .iter()
            .flat_map(|cohort| &cohort.participants)
            .filter(|participant| {
                participant.relationship == "account_agent"
                    && participant.status == "active"
                    && !removed_members.contains(&participant.npub)
            })
            .map(|participant| participant.npub.clone())
            .collect::<BTreeSet<_>>();
        let revoked_invitation_sources = target_cohorts
            .iter()
            .filter(|cohort| cohort.provenance_kind == "invitation")
            .map(|cohort| cohort.provenance_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut statement = self.conn.prepare(
            "SELECT folder_id, user_id, source_kind, source_id FROM folder_access_sources WHERE brain_id = ?1",
        )?;
        let sources = statement
            .query_map(params![stored.brain.id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut removals = BTreeMap::<FolderId, BTreeSet<UserId>>::new();
        for (folder_id, participants) in &stored.folder_access {
            for participant in participants {
                if removed_members.contains(participant) {
                    removals
                        .entry(folder_id.clone())
                        .or_default()
                        .insert(participant.clone());
                    continue;
                }
                if !retained_cohort_agents.contains(participant) {
                    continue;
                }
                let matching = sources.iter().filter(|(source_folder, source_user, _, _)| {
                    source_folder == folder_id.as_str() && source_user == participant.as_str()
                });
                let mut revoked = false;
                let mut retained = false;
                for (_, _, source_kind, source_id) in matching {
                    if source_kind == "invitation"
                        && revoked_invitation_sources.contains(source_id.as_str())
                    {
                        revoked = true;
                    } else {
                        retained = true;
                    }
                }
                if revoked && !retained {
                    removals
                        .entry(folder_id.clone())
                        .or_default()
                        .insert(participant.clone());
                }
            }
        }
        Ok(MemberRemovalAccessPlan {
            removed_members,
            folder_access_removals: removals,
        })
    }
}

fn member_removal_participants(stored: &StoredBrain, target: &UserId) -> BTreeSet<UserId> {
    let mut removed = BTreeSet::from([target.clone()]);
    for cohort in &stored.account_access_cohorts {
        let target_is_human = cohort.status == "active"
            && cohort.scope_kind == "brain"
            && cohort.participants.iter().any(|participant| {
                participant.relationship == "human"
                    && participant.status == "active"
                    && participant.npub == *target
            });
        if target_is_human {
            removed.extend(
                cohort
                    .participants
                    .iter()
                    .filter(|participant| participant.status == "active")
                    .map(|participant| participant.npub.clone()),
            );
        }
    }
    removed
}

fn validate_folder_grant_control_records(
    folder_id: &FolderId,
    grant: &FolderKeyGrantMetadata,
    control_records: &[SyncRecordInput],
) -> Result<(), StoreError> {
    if control_records.len() != 2 {
        return Err(StoreError::BrokenInvariant {
            reason: "Folder access grant requires one Folder Key Grant record and one access-change record"
                .to_owned(),
        });
    }
    let expected_types = [
        SyncRecordType::FolderKeyGrant,
        SyncRecordType::BrainAdminAccessChange,
    ];
    for (input, expected_type) in control_records.iter().zip(expected_types) {
        sync_records::validate_sync_input(input)?;
        let SyncRecordInput::Control(record) = input else {
            return Err(StoreError::BrokenInvariant {
                reason: "Folder access grant records must be control records".to_owned(),
            });
        };
        if record.record_type != expected_type
            || record.folder_id.as_ref() != Some(folder_id)
            || record.actor_npub != grant.issuer_npub
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Folder access grant control records do not match the signed mutation"
                    .to_owned(),
            });
        }
    }
    Ok(())
}
