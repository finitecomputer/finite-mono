use crate::folder_key_rotation::{
    apply_folder_key_rotation, begin_authoritative_write, require_next_key_version,
    validate_folder_key_rotation_material,
};
use crate::member_removal::require_member_removal_rotation_set;
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
            validate_grant_issuer(&stored.brain, grant, None)?;
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
        let required =
            required_recipients(&stored.brain, folder, &access_user_ids, personal_agent)?;
        validate_folder_grants(&stored.brain, folder, &required, grants, personal_agent)?;

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
        let effective_access = required_recipients(
            &stored.brain,
            &folder,
            &current_access,
            stored
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
        )?;
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
            // Demotion can remove inherited access without deleting its key
            // grant. Preserve that wrap and publish only the new permission;
            // publishing the discarded replacement would diverge from export.
            if current_grant_exists && input.record_type() == SyncRecordType::FolderKeyGrant {
                continue;
            }
            sync_records::validate_sync_conflict(&tx, brain_id, input)?;
            let sequence = sync_records::next_sequence(&tx, brain_id)?;
            sync_records::insert_sync_record(&tx, brain_id, sequence, input)?;
        }
        tx.commit()?;
        Ok(GrantFolderAccessOutcome::Granted)
    }

    /// Open a Restricted Organization Folder to every Member. One way only.
    ///
    /// New readers receive the current Folder Key, so no content is
    /// re-encrypted. `grants` pairs each client-wrapped grant with its sync
    /// record and must cover every recipient of the opened Folder that lacks a
    /// current grant. Grants for current holders are skipped, so a client may
    /// wrap for every recipient. Explicit Folder access and child Folders are
    /// unchanged. The mode change, new grants and signed audit record commit
    /// together; a roster or key change since the client prepared the request
    /// is a conflict that writes nothing.
    pub fn open_folder_to_all_members_with_control_records(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        expected_key_version: u32,
        grants: &[(FolderKeyGrantMetadata, SyncRecordInput)],
        access_change_record: &SyncRecordInput,
    ) -> Result<OpenFolderToAllMembersOutcome, StoreError> {
        // Read the roster under the write lock so it cannot change before commit.
        let tx = begin_authoritative_write(&self.conn)?;
        let stored = self.load_brain(brain_id)?;
        if stored.brain.kind != BrainKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "only an Organization Brain Folder can be opened to all Members".to_owned(),
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
        match folder.access {
            FolderAccessMode::AllMembers => return Ok(OpenFolderToAllMembersOutcome::AlreadyOpen),
            FolderAccessMode::Restricted => {}
            FolderAccessMode::Owner | FolderAccessMode::AdminOnly => {
                return Err(StoreError::BrokenInvariant {
                    reason: "only a Restricted Folder can be opened to all Members".to_owned(),
                });
            }
        }
        if stored.setup_incomplete_folder_ids.contains(folder_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "finish Folder setup before opening it to all Members".to_owned(),
            });
        }
        if folder.current_key_version != expected_key_version {
            return Err(StoreError::Conflict {
                reason: "the Folder Key changed while the request was prepared; refresh and retry"
                    .to_owned(),
                current_revision: None,
            });
        }
        let opened = Folder {
            access: FolderAccessMode::AllMembers,
            ..folder.clone()
        };
        let access_user_ids = stored
            .folder_access
            .get(folder_id)
            .cloned()
            .unwrap_or_default();
        let required = required_recipients(&stored.brain, &opened, &access_user_ids, None)?;
        let holders = stored
            .grants
            .iter()
            .filter(|grant| {
                grant.folder_id == *folder_id && grant.key_version == folder.current_key_version
            })
            .map(|grant| grant.recipient_npub.clone())
            .collect::<BTreeSet<_>>();

        let mut supplied = BTreeSet::new();
        let mut new_grants = Vec::new();
        let mut new_records = Vec::new();
        for (grant, record) in grants {
            validate_grant_metadata(grant)?;
            validate_grant_issuer(&stored.brain, grant, None)?;
            if grant.folder_id != *folder_id || grant.key_version != folder.current_key_version {
                return Err(StoreError::BrokenInvariant {
                    reason: "grant must carry this Folder's current key version".to_owned(),
                });
            }
            if !required.contains(&grant.recipient_npub) {
                return Err(StoreError::Conflict {
                    reason: format!(
                        "{} is no longer a reader of the opened Folder; refresh and retry",
                        grant.recipient_npub
                    ),
                    current_revision: None,
                });
            }
            if !supplied.insert(grant.recipient_npub.clone()) {
                return Err(StoreError::BrokenInvariant {
                    reason: "each recipient may receive only one grant".to_owned(),
                });
            }
            if !holders.contains(&grant.recipient_npub) {
                new_grants.push(grant.clone());
                new_records.push(record.clone());
            }
        }
        if let Some(missing) = required
            .iter()
            .find(|recipient| !holders.contains(*recipient) && !supplied.contains(*recipient))
        {
            return Err(StoreError::Conflict {
                reason: format!(
                    "{missing} needs a grant for the opened Folder; the roster changed, refresh and retry"
                ),
                current_revision: None,
            });
        }
        validate_folder_key_grant_control_records(&new_grants, &new_records)?;
        let SyncRecordInput::Control(audit) = access_change_record else {
            return Err(StoreError::BrokenInvariant {
                reason: "opening a Folder requires a signed admin access-change record".to_owned(),
            });
        };
        if audit.record_type != SyncRecordType::BrainAdminAccessChange
            || audit.folder_id.as_ref() != Some(folder_id)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "the access-change record must name the opened Folder".to_owned(),
            });
        }

        let changed = tx.execute(
            "UPDATE folders SET access = 'all_members'
             WHERE brain_id = ?1 AND id = ?2 AND access = 'restricted' AND current_key_version = ?3",
            params![brain_id.as_str(), folder_id.as_str(), expected_key_version],
        )?;
        if changed != 1 {
            return Err(StoreError::Conflict {
                reason: "the Folder changed while it was being opened; refresh and retry"
                    .to_owned(),
                current_revision: None,
            });
        }
        for grant in &new_grants {
            insert_grant(&tx, brain_id, grant)?;
        }
        new_records.push(access_change_record.clone());
        sync_records::append_sync_records(&tx, brain_id, &new_records)?;
        tx.commit()?;
        Ok(OpenFolderToAllMembersOutcome::Opened {
            granted_recipients: new_grants
                .into_iter()
                .map(|grant| grant.recipient_npub)
                .collect(),
        })
    }

    /// Remove a Member and every Folder Key relationship in one transaction.
    ///
    /// `rotations` must cover exactly [`member_removal_folder_rotations`]: the
    /// Folders the Member reads and the Folders where it still holds the
    /// current key without entitlement.
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
        // Every roster, Mount and object read below happens inside the write
        // transaction, so validation and the write see one snapshot.
        let tx = begin_authoritative_write(&self.conn)?;
        let stored = self.load_brain(brain_id)?;
        let planned_rotations = member_removal_folder_rotations(&stored, removed_user_id)?;
        require_member_removal_rotation_set(&planned_rotations, rotations)?;
        let supplied_folders = rotations
            .iter()
            .map(|rotation| rotation.folder_id.clone())
            .collect::<BTreeSet<_>>();
        let mut post_removal_brain = stored.brain.clone();
        post_removal_brain
            .members
            .retain(|member| member.user_id != *removed_user_id);
        let personal_agent = stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub);
        let expected_mounts = self
            .list_active_destination_mounts_for_participant(brain_id, removed_user_id)?
            .into_iter()
            .map(|connection| connection.id)
            .collect::<BTreeSet<_>>();
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
        let mut previous_key_versions = Vec::with_capacity(rotations.len());
        for rotation in rotations {
            let folder = stored
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == rotation.folder_id)
                .ok_or_else(|| StoreError::MissingFolder {
                    folder_id: rotation.folder_id.to_string(),
                })?;
            let mut remaining_access = stored
                .folder_access
                .get(&rotation.folder_id)
                .cloned()
                .unwrap_or_default();
            remaining_access.remove(removed_user_id);
            validate_folder_key_rotation_material(
                &stored.brain,
                &post_removal_brain,
                folder,
                &remaining_access,
                personal_agent,
                rotation.new_key_version,
                &rotation.grants,
                &rotation.reencrypted_records,
                &current_objects,
            )?;
            previous_key_versions.push(folder.current_key_version);
        }
        let mut mounted_removals = Vec::with_capacity(mount_rotations.len());
        let mut rotated_sources = BTreeSet::new();
        for rotation in mount_rotations {
            let connection = self.load_shared_folder_connection(&rotation.connection_id)?;
            if connection.destination_brain_id != *brain_id
                || connection.status != SharedFolderConnectionStatus::Active
                || !connection.member_npubs.contains(removed_user_id)
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "member removal Mount rotation does not match active destination participation"
                        .to_owned(),
                });
            }
            let must_revoke = connection.destination_admin_npub == *removed_user_id;
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
            } else if connection.managed_access_npubs.contains(removed_user_id) {
                BTreeSet::from([removed_user_id.clone()])
            } else {
                BTreeSet::new()
            };
            if removed.is_empty() {
                mounted_removals.push((connection, removed, 0));
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
            require_next_key_version(folder, rotation.new_key_version)?;
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
            let required = required_recipients(
                &source.brain,
                &rotated_folder,
                &remaining_access,
                source
                    .personal_agent
                    .as_ref()
                    .map(|relationship| &relationship.agent_npub),
            )?;
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
            mounted_removals.push((connection, removed, folder.current_key_version));
        }

        for (rotation, previous_key_version) in rotations.iter().zip(previous_key_versions) {
            tx.execute(
                "DELETE FROM folder_access WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3",
                params![
                    brain_id.as_str(),
                    rotation.folder_id.as_str(),
                    removed_user_id.as_str()
                ],
            )?;
            apply_folder_key_rotation(
                &tx,
                brain_id,
                &rotation.folder_id,
                previous_key_version,
                rotation.new_key_version,
                &rotation.grants,
                &rotation.reencrypted_records,
                updated_at,
            )?;
        }
        for (rotation, (connection, removed, previous_key_version)) in
            mount_rotations.iter().zip(mounted_removals.iter())
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
                pending_wraps::clear_pending_grant_wraps_for_folder_recipient(
                    &tx,
                    &connection.source_brain_id,
                    &connection.source_folder_id,
                    target,
                )?;
            }
            apply_folder_key_rotation(
                &tx,
                &connection.source_brain_id,
                &connection.source_folder_id,
                *previous_key_version,
                rotation.new_key_version,
                &rotation.grants,
                &rotation.reencrypted_records,
                updated_at,
            )?;
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
        }
        tx.execute(
            "DELETE FROM folder_access WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), removed_user_id.as_str()],
        )?;
        tx.execute(
            "DELETE FROM brain_members WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), removed_user_id.as_str()],
        )?;
        pending_wraps::clear_pending_grant_wraps_for_recipient(&tx, brain_id, removed_user_id)?;
        for (record_brain_id, control_records) in control_records_by_brain {
            sync_records::append_sync_records(&tx, record_brain_id, control_records)?;
        }
        tx.commit()?;
        Ok(())
    }
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
