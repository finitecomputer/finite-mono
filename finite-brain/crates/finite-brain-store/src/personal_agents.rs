use crate::*;

impl BrainStore {
    pub fn replace_personal_agent(
        &mut self,
        brain_id: &BrainId,
        owner_npub: &UserId,
        replacement_npub: Option<&UserId>,
        rotations: &[PersonalAgentFolderRotation],
        updated_at: &str,
    ) -> Result<(), StoreError> {
        validate_folder_rotation_fanout(
            FolderRotationOperation::PersonalAgent,
            rotations.iter().map(|rotation| FolderRotationFanout {
                grants: rotation.grants.len(),
                reencrypted_records: rotation.reencrypted_records.len(),
            }),
        )?;
        let stored = self.load_brain(brain_id)?;
        if stored.brain.kind != BrainKind::Personal
            || stored.brain.owner_user_id.as_ref() != Some(owner_npub)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "only the Personal Brain owner may replace its Personal Agent".to_owned(),
            });
        }
        let current = stored.personal_agent.as_ref();
        if current.is_none() && replacement_npub.is_none() {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Brain has no Personal Agent to remove".to_owned(),
            });
        }
        if replacement_npub == Some(owner_npub)
            || current.is_some_and(|current| replacement_npub == Some(&current.agent_npub))
        {
            return Err(StoreError::BrokenInvariant {
                reason: "replacement must be a distinct Agent Principal".to_owned(),
            });
        }
        let expected_ids = stored
            .brain
            .folders
            .iter()
            .map(|folder| folder.id.clone())
            .collect::<BTreeSet<_>>();
        let supplied_ids = rotations
            .iter()
            .map(|rotation| rotation.folder_id.clone())
            .collect::<BTreeSet<_>>();
        if expected_ids != supplied_ids || rotations.len() != expected_ids.len() {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Agent replacement must rotate every current Folder exactly once"
                    .to_owned(),
            });
        }
        for rotation in rotations {
            let folder = stored
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == rotation.folder_id)
                .expect("rotation set matched folders");
            if rotation.new_key_version != folder.current_key_version + 1 {
                return Err(StoreError::BrokenInvariant {
                    reason: "Personal Agent replacement must use the next Folder Key version"
                        .to_owned(),
                });
            }
            let mut rotated = folder.clone();
            rotated.current_key_version = rotation.new_key_version;
            let explicit_access = stored
                .folder_access
                .get(&folder.id)
                .cloned()
                .unwrap_or_default();
            let required =
                required_recipients(&stored.brain, &rotated, &explicit_access, replacement_npub)?;
            validate_folder_grants(
                &stored.brain,
                &rotated,
                &required,
                &rotation.grants,
                Some(owner_npub),
            )?;
            let live_objects = self
                .load_current_objects(brain_id)?
                .into_iter()
                .filter(|object| object.folder_id == folder.id && !object.deleted)
                .collect::<Vec<_>>();
            validate_rotation_records(&live_objects, &rotation.reencrypted_records)?;
            let grant_record_count = rotation
                .control_records
                .iter()
                .filter(|record| record.record_type == SyncRecordType::FolderKeyGrant)
                .count();
            let access_record_count = rotation
                .control_records
                .iter()
                .filter(|record| record.record_type == SyncRecordType::BrainAdminAccessChange)
                .count();
            if rotation.control_records.len() != rotation.grants.len() + 1
                || grant_record_count != rotation.grants.len()
                || access_record_count != 1
                || rotation.control_records.iter().any(|record| {
                    record.folder_id.as_ref() != Some(&rotation.folder_id)
                        || record.actor_npub != *owner_npub
                })
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "Personal Agent replacement requires one signed rotation control and one signed control per Folder Key Grant".to_owned(),
                });
            }
            for record in &rotation.control_records {
                sync_records::validate_sync_input(&SyncRecordInput::Control(record.clone()))?;
            }
        }

        let tx = self.conn.transaction()?;
        for rotation in rotations {
            tx.execute(
                "DELETE FROM folder_key_grants WHERE brain_id = ?1 AND folder_id = ?2",
                params![brain_id.as_str(), rotation.folder_id.as_str()],
            )?;
            tx.execute(
                "UPDATE folders SET current_key_version = ?3 WHERE brain_id = ?1 AND id = ?2",
                params![
                    brain_id.as_str(),
                    rotation.folder_id.as_str(),
                    rotation.new_key_version
                ],
            )?;
            for grant in &rotation.grants {
                insert_grant(&tx, brain_id, grant)?;
            }
            for record in &rotation.control_records {
                let input = SyncRecordInput::Control(record.clone());
                sync_records::validate_sync_conflict(&tx, brain_id, &input)?;
                let sequence = sync_records::next_sequence(&tx, brain_id)?;
                sync_records::insert_sync_record(&tx, brain_id, sequence, &input)?;
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
        let action = match (current, replacement_npub) {
            (None, Some(_)) => "established",
            (Some(_), Some(_)) => "replaced",
            (Some(_), None) => "revoked",
            (None, None) => unreachable!("vacant removal rejected above"),
        };
        if let Some(replacement) = replacement_npub {
            if current.is_some() {
                tx.execute(
                    "UPDATE personal_agents SET agent_npub = ?2, created_by_npub = ?3, updated_at = ?4 WHERE brain_id = ?1",
                    params![brain_id.as_str(), replacement.as_str(), owner_npub.as_str(), updated_at],
                )?;
            } else {
                tx.execute(
                    r#"INSERT INTO personal_agents (
                        brain_id, owner_npub, agent_npub, status,
                        created_by_npub, created_at, updated_at
                    ) VALUES (?1, ?2, ?3, 'active', ?2, ?4, ?4)"#,
                    params![
                        brain_id.as_str(),
                        owner_npub.as_str(),
                        replacement.as_str(),
                        updated_at,
                    ],
                )?;
            }
        } else {
            tx.execute(
                "DELETE FROM personal_agents WHERE brain_id = ?1",
                params![brain_id.as_str()],
            )?;
        }
        let audit_index = tx.query_row(
            "SELECT COUNT(*) FROM personal_agent_audit WHERE brain_id = ?1",
            params![brain_id.as_str()],
            |row| row.get::<_, u64>(0),
        )? + 1;
        let audit_id = format!("{}-{action}-{audit_index}", brain_id);
        tx.execute(
            r#"INSERT INTO personal_agent_audit (
                id, brain_id, action, actor_npub, previous_agent_npub, agent_npub, occurred_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                audit_id,
                brain_id.as_str(),
                action,
                owner_npub.as_str(),
                current.map(|relationship| relationship.agent_npub.as_str()),
                replacement_npub.map(UserId::as_str),
                updated_at,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
}
