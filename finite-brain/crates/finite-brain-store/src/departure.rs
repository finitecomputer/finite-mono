use crate::*;

impl BrainStore {
    /// Last-applied Permanent Departure Fact revision. Always trails the facts
    /// whose revocations actually committed.
    pub fn departure_fact_cursor(&self) -> Result<i64, StoreError> {
        let cursor = self.conn.query_row(
            "SELECT last_applied_revision FROM brain_departure_fact_cursor WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(cursor)
    }

    /// Apply one resolved departure fact in revision order. Every revocation
    /// and the cursor advance commit in one transaction, so a crash can never
    /// move the cursor past an unapplied fact. Reapplying an already-covered
    /// revision is a no-op.
    pub fn apply_departure_fact(
        &mut self,
        application: &DepartureFactApplication,
    ) -> Result<DepartureFactOutcome, StoreError> {
        if application.fact_revision <= 0 {
            return Err(StoreError::BrokenInvariant {
                reason: "departure fact revision must be positive".to_owned(),
            });
        }
        if application.principal_ref.trim().is_empty() {
            return Err(StoreError::BrokenInvariant {
                reason: "departure fact principal reference is required".to_owned(),
            });
        }
        if self.departure_fact_cursor()? >= application.fact_revision {
            return Ok(DepartureFactOutcome::default());
        }

        // Plan every per-Brain revocation before opening the transaction. The
        // store is single-writer, so the loaded state cannot drift before the
        // transaction begins.
        let mut plans = Vec::new();
        if let Some(departed) = application.departed_npub.as_ref() {
            for brain_id in self.brains_holding_principal(departed)? {
                plans.push(self.plan_departure_revocation(&brain_id, departed)?);
            }
        }

        let tx = self.conn.transaction()?;
        let mut outcome = DepartureFactOutcome {
            applied: true,
            affected_brain_ids: BTreeSet::new(),
            revocations: 0,
        };
        for plan in &plans {
            let inserted = tx.execute(
                r#"
                INSERT OR IGNORE INTO brain_principal_revocations (
                    id, brain_id, departed_npub, principal_kind, principal_ref,
                    account_id, fact_revision, origin_kind, origin_ref, applied_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                "#,
                params![
                    format!(
                        "departure-{}-{}",
                        application.fact_revision,
                        plan.brain_id.as_str()
                    ),
                    plan.brain_id.as_str(),
                    application.departed_npub.as_ref().map(UserId::as_str),
                    application.principal_kind.as_str(),
                    application.principal_ref,
                    application.account_id,
                    application.fact_revision,
                    ProvenanceOriginKind::Departure.as_str(),
                    application.origin_ref(),
                    application.applied_at,
                ],
            )?;
            outcome.revocations += inserted;
            outcome.affected_brain_ids.insert(plan.brain_id.clone());

            for (folder_id, key_version) in &plan.pending_rotation_folders {
                mark_departure_pending_rotation(
                    &tx,
                    &plan.brain_id,
                    folder_id,
                    *key_version,
                    application.fact_revision,
                    &application.applied_at,
                )?;
            }
            for mount in &plan.mount_removals {
                if mount.revoke_mount {
                    tx.execute(
                        "UPDATE shared_folder_connections SET status = 'revoked', updated_at = ?2 WHERE id = ?1",
                        params![mount.connection_id, application.applied_at],
                    )?;
                }
                tx.execute(
                    "DELETE FROM shared_folder_connection_members WHERE connection_id = ?1 AND member_npub = ?2",
                    params![mount.connection_id, mount.member_npub.as_str()],
                )?;
                for target in &mount.managed_access_removed {
                    tx.execute(
                        "DELETE FROM folder_access WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3",
                        params![
                            mount.source_brain_id.as_str(),
                            mount.source_folder_id.as_str(),
                            target.as_str()
                        ],
                    )?;
                    pending_wraps::clear_pending_grant_wraps_for_folder_recipient(
                        &tx,
                        &mount.source_brain_id,
                        &mount.source_folder_id,
                        target,
                    )?;
                }
                if let Some(key_version) = mount.source_pending_key_version {
                    mark_departure_pending_rotation(
                        &tx,
                        &mount.source_brain_id,
                        &mount.source_folder_id,
                        key_version,
                        application.fact_revision,
                        &application.applied_at,
                    )?;
                    outcome
                        .affected_brain_ids
                        .insert(mount.source_brain_id.clone());
                }
            }
            tx.execute(
                "DELETE FROM brain_admins WHERE brain_id = ?1 AND user_id = ?2",
                params![
                    plan.brain_id.as_str(),
                    application
                        .departed_npub
                        .as_ref()
                        .expect("plans only exist for resolved principals")
                        .as_str()
                ],
            )?;
            tx.execute(
                "DELETE FROM personal_agents WHERE brain_id = ?1 AND agent_npub = ?2",
                params![
                    plan.brain_id.as_str(),
                    application
                        .departed_npub
                        .as_ref()
                        .expect("plans only exist for resolved principals")
                        .as_str()
                ],
            )?;
            tx.execute(
                "DELETE FROM folder_access WHERE brain_id = ?1 AND user_id = ?2",
                params![
                    plan.brain_id.as_str(),
                    application
                        .departed_npub
                        .as_ref()
                        .expect("plans only exist for resolved principals")
                        .as_str()
                ],
            )?;
            tx.execute(
                "DELETE FROM folder_key_grants WHERE brain_id = ?1 AND recipient_npub = ?2",
                params![
                    plan.brain_id.as_str(),
                    application
                        .departed_npub
                        .as_ref()
                        .expect("plans only exist for resolved principals")
                        .as_str()
                ],
            )?;
            // Pending invitations for a permanently departed Principal can
            // never legitimately complete; revoke them with the access.
            tx.execute(
                "UPDATE brain_invitations SET status = 'revoked', updated_at = ?3 \
                 WHERE brain_id = ?1 AND status = 'pending' \
                   AND (user_id = ?2 OR invited_email = ?4)",
                params![
                    plan.brain_id.as_str(),
                    application
                        .departed_npub
                        .as_ref()
                        .expect("plans only exist for resolved principals")
                        .as_str(),
                    application.applied_at,
                    application.principal_ref.trim().to_ascii_lowercase(),
                ],
            )?;
            tx.execute(
                "DELETE FROM brain_members WHERE brain_id = ?1 AND user_id = ?2",
                params![
                    plan.brain_id.as_str(),
                    application
                        .departed_npub
                        .as_ref()
                        .expect("plans only exist for resolved principals")
                        .as_str()
                ],
            )?;
            pending_wraps::clear_pending_grant_wraps_for_recipient(
                &tx,
                &plan.brain_id,
                application
                    .departed_npub
                    .as_ref()
                    .expect("plans only exist for resolved principals"),
            )?;
        }
        tx.execute(
            "UPDATE brain_departure_fact_cursor SET last_applied_revision = ?1, updated_at = ?2 WHERE id = 1",
            params![application.fact_revision, application.applied_at],
        )?;
        tx.commit()?;
        Ok(outcome)
    }

    /// Revocation ledger rows for one Brain, oldest departure first.
    pub fn departure_revocations(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<DepartureRevocationRecord>, StoreError> {
        let mut statement = self.conn.prepare(
            r#"
            SELECT id, brain_id, departed_npub, principal_kind, principal_ref,
                   account_id, fact_revision, origin_kind, origin_ref, applied_at
            FROM brain_principal_revocations
            WHERE brain_id = ?1
            ORDER BY fact_revision ASC, id ASC
            "#,
        )?;
        let rows = statement.query_map(params![brain_id.as_str()], |row| {
            let origin_kind = row.get::<_, String>(7)?;
            let principal_kind = row.get::<_, String>(3)?;
            Ok(DepartureRevocationRecord {
                id: row.get(0)?,
                brain_id: BrainId::new(row.get::<_, String>(1)?)
                    .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
                departed_npub: row
                    .get::<_, Option<String>>(2)?
                    .map(UserId::new)
                    .transpose()
                    .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?,
                principal_kind: DeparturePrincipalKind::try_from(principal_kind.as_str())
                    .map_err(to_store_from_sql_error(3, rusqlite::types::Type::Text))?,
                principal_ref: row.get(4)?,
                account_id: row.get(5)?,
                fact_revision: row.get(6)?,
                origin_kind: ProvenanceOriginKind::try_from(origin_kind.as_str())
                    .map_err(to_store_from_sql_error(7, rusqlite::types::Type::Text))?,
                origin_ref: row.get(8)?,
                applied_at: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Folders in one Brain still waiting for a client-driven re-wrap after a
    /// departure revocation.
    pub fn departure_pending_rotations(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<DeparturePendingRotation>, StoreError> {
        let mut statement = self.conn.prepare(
            r#"
            SELECT brain_id, folder_id, marked_at_revision, key_version
            FROM brain_departure_pending_rotations
            WHERE brain_id = ?1
            ORDER BY folder_id ASC
            "#,
        )?;
        let rows = statement.query_map(params![brain_id.as_str()], |row| {
            Ok(DeparturePendingRotation {
                brain_id: BrainId::new(row.get::<_, String>(0)?)
                    .map_err(to_from_sql_error(0, rusqlite::types::Type::Text))?,
                folder_id: FolderId::new(row.get::<_, String>(1)?)
                    .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
                marked_at_revision: row.get(2)?,
                key_version: row.get::<_, u32>(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Complete a departure-marked Folder Key rotation with grants and
    /// re-encrypted records prepared by a remaining admin's client. The server
    /// never sees plaintext keys; the authority for the rotation is the
    /// departure fact recorded on the pending marker, so no signed admin
    /// access-change event is required — only the per-grant control records.
    ///
    /// When the Folder already advanced past the marked version through
    /// another rotation path, the marker is simply cleared.
    #[allow(clippy::too_many_arguments)]
    pub fn complete_departure_rotation(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        new_key_version: u32,
        grants: &[FolderKeyGrantMetadata],
        reencrypted_records: &[FolderObjectRevisionSyncRecord],
        updated_at: &str,
        control_records: &[SyncRecordInput],
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
        let marked_version: u32 = self
            .conn
            .query_row(
                "SELECT key_version FROM brain_departure_pending_rotations WHERE brain_id = ?1 AND folder_id = ?2",
                params![brain_id.as_str(), folder_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "no departure rotation is pending for this Folder".to_owned(),
            })?;
        if marked_version < folder.current_key_version {
            let tx = self.conn.transaction()?;
            tx.execute(
                "DELETE FROM brain_departure_pending_rotations WHERE brain_id = ?1 AND folder_id = ?2",
                params![brain_id.as_str(), folder_id.as_str()],
            )?;
            tx.commit()?;
            return Ok(());
        }
        if new_key_version != folder.current_key_version + 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "departure rotation must rotate to the next key version".to_owned(),
            });
        }
        let access = stored
            .folder_access
            .get(folder_id)
            .cloned()
            .unwrap_or_default();
        let personal_agent = stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub);
        let mut rotated_folder = folder.clone();
        rotated_folder.current_key_version = new_key_version;
        let required =
            required_recipients(&stored.brain, &rotated_folder, &access, personal_agent)?;
        validate_folder_grants(
            &stored.brain,
            &rotated_folder,
            &required,
            grants,
            personal_agent,
        )?;
        let live_objects = self
            .load_current_objects(brain_id)?
            .into_iter()
            .filter(|object| object.folder_id == *folder_id && !object.deleted)
            .collect::<Vec<_>>();
        validate_rotation_records(&live_objects, reencrypted_records)?;
        validate_folder_key_grant_control_records(grants, control_records)?;

        let tx = self.conn.transaction()?;
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
        tx.execute(
            "DELETE FROM brain_departure_pending_rotations WHERE brain_id = ?1 AND folder_id = ?2",
            params![brain_id.as_str(), folder_id.as_str()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Look up the identity alias recorded for a NIP-05 identifier (Managed
    /// Agent Email or human mailbox) when brains were granted access.
    pub fn identity_alias_for_preferred_nip05(
        &self,
        nip05: &str,
    ) -> Result<Option<IdentityAlias>, StoreError> {
        let alias = self
            .conn
            .query_row(
                "SELECT npub, hex_public_key, preferred_nip05, nip05_verified_at, nip05_relays_json, updated_at FROM identity_aliases WHERE preferred_nip05 = ?1",
                params![nip05],
                identity_alias_from_row,
            )
            .optional()?;
        Ok(alias)
    }

    fn brains_holding_principal(&self, departed: &UserId) -> Result<Vec<BrainId>, StoreError> {
        let mut statement = self.conn.prepare(
            r#"
            SELECT brain_id FROM brain_members WHERE user_id = ?1
            UNION SELECT brain_id FROM brain_admins WHERE user_id = ?1
            UNION SELECT brain_id FROM folder_access WHERE user_id = ?1
            UNION SELECT brain_id FROM folder_key_grants WHERE recipient_npub = ?1
            UNION SELECT brain_id FROM personal_agents WHERE agent_npub = ?1
            UNION SELECT connections.destination_brain_id
                  FROM shared_folder_connection_members members
                  JOIN shared_folder_connections connections
                    ON connections.id = members.connection_id
                  WHERE members.member_npub = ?1 AND connections.status = 'active'
            ORDER BY 1
            "#,
        )?;
        let rows =
            statement.query_map(params![departed.as_str()], |row| row.get::<_, String>(0))?;
        rows.map(|brain_id| BrainId::new(brain_id?).map_err(StoreError::from))
            .collect()
    }

    fn plan_departure_revocation(
        &self,
        brain_id: &BrainId,
        departed: &UserId,
    ) -> Result<DepartureRevocationPlan, StoreError> {
        let stored = self.load_brain(brain_id)?;
        let personal_agent = stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub);
        let mut pending_rotation_folders = Vec::new();
        for folder in &stored.brain.folders {
            let access = stored
                .folder_access
                .get(&folder.id)
                .cloned()
                .unwrap_or_default();
            if required_recipients(&stored.brain, folder, &access, personal_agent)?
                .contains(departed)
            {
                pending_rotation_folders.push((folder.id.clone(), folder.current_key_version));
            }
        }
        let mut mount_removals = Vec::new();
        for connection in self.list_active_destination_mounts_for_participant(brain_id, departed)? {
            let revoke_mount = connection.destination_admin_npub == *departed;
            let managed_access_removed = if revoke_mount {
                connection.managed_access_npubs.clone()
            } else if connection.managed_access_npubs.contains(departed) {
                BTreeSet::from([departed.clone()])
            } else {
                BTreeSet::new()
            };
            let source_pending_key_version = if managed_access_removed.is_empty() {
                None
            } else {
                let source = self.load_brain(&connection.source_brain_id)?;
                let folder = source
                    .brain
                    .folders
                    .iter()
                    .find(|folder| folder.id == connection.source_folder_id)
                    .ok_or_else(|| StoreError::MissingFolder {
                        folder_id: connection.source_folder_id.to_string(),
                    })?;
                Some(folder.current_key_version)
            };
            mount_removals.push(DepartureMountRemoval {
                connection_id: connection.id.clone(),
                member_npub: departed.clone(),
                revoke_mount,
                managed_access_removed,
                source_brain_id: connection.source_brain_id.clone(),
                source_folder_id: connection.source_folder_id.clone(),
                source_pending_key_version,
            });
        }
        Ok(DepartureRevocationPlan {
            brain_id: brain_id.clone(),
            pending_rotation_folders,
            mount_removals,
        })
    }
}

#[derive(Debug)]
struct DepartureRevocationPlan {
    brain_id: BrainId,
    pending_rotation_folders: Vec<(FolderId, u32)>,
    mount_removals: Vec<DepartureMountRemoval>,
}

#[derive(Debug)]
struct DepartureMountRemoval {
    connection_id: String,
    member_npub: UserId,
    revoke_mount: bool,
    managed_access_removed: BTreeSet<UserId>,
    source_brain_id: BrainId,
    source_folder_id: FolderId,
    source_pending_key_version: Option<u32>,
}

fn mark_departure_pending_rotation(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    folder_id: &FolderId,
    key_version: u32,
    fact_revision: i64,
    updated_at: &str,
) -> Result<(), StoreError> {
    tx.execute(
        r#"
        INSERT INTO brain_departure_pending_rotations (
            brain_id, folder_id, marked_at_revision, key_version, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT (brain_id, folder_id) DO UPDATE SET
            marked_at_revision = excluded.marked_at_revision,
            key_version = excluded.key_version,
            updated_at = excluded.updated_at
        "#,
        params![
            brain_id.as_str(),
            folder_id.as_str(),
            fact_revision,
            key_version,
            updated_at
        ],
    )?;
    Ok(())
}
