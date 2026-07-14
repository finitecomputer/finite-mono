use crate::*;

const MAX_AGENT_WORKSPACE_PAIRINGS_PER_VAULT: i64 = 100;
const MAX_DELEGATION_AUDIT_ROWS: i64 = 100;

impl BrainStore {
    /// Atomically establish the initial restricted Agent Workspace and its durable delegation.
    pub fn ensure_personal_agent_workspace(
        &mut self,
        input: &EnsurePersonalAgentWorkspaceInput,
    ) -> Result<EnsurePersonalAgentWorkspaceOutcome, StoreError> {
        let vault = self.load_core_vault(&input.vault_id)?;
        if vault.kind != VaultKind::Personal
            || vault.owner_user_id.as_ref() != Some(&input.owner_npub)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "agent workspace pairing requires the Personal Vault owner".to_owned(),
            });
        }
        if input.agent_npub == input.owner_npub {
            return Err(StoreError::BrokenInvariant {
                reason: "agent workspace pairing requires a distinct Agent Principal".to_owned(),
            });
        }
        validate_pairing_sync_records(input)?;

        if let Some(delegation) =
            self.load_brain_email_access_delegation(&input.vault_id, &input.agent_npub)?
        {
            let stored = self.load_vault(&input.vault_id)?;
            let stored_folder = stored
                .vault
                .folders
                .iter()
                .find(|folder| folder.id == input.folder.id);
            let stored_access = stored.folder_access.get(&input.folder.id);
            let requested_access = BTreeSet::from([input.agent_npub.clone()]);
            let stored_grants = stored
                .grants
                .iter()
                .filter(|grant| {
                    grant.folder_id == input.folder.id
                        && grant.key_version == input.folder.current_key_version
                })
                .map(|grant| (grant.recipient_npub.clone(), grant.clone()))
                .collect::<BTreeMap<_, _>>();
            let requested_grants = input
                .grants
                .iter()
                .map(|grant| (grant.recipient_npub.clone(), grant.clone()))
                .collect::<BTreeMap<_, _>>();
            let sync_records_match = input.sync_records.iter().try_fold(
                true,
                |all_match, record| -> Result<bool, StoreError> {
                    let exists = self.conn.query_row(
                        "SELECT EXISTS(SELECT 1 FROM vault_record_index WHERE vault_id = ?1 AND record_event_id = ?2)",
                        params![input.vault_id.as_str(), record.record_event_id()],
                        |row| row.get::<_, bool>(0),
                    )?;
                    Ok(all_match && exists)
                },
            )?;
            if delegation.id != input.delegation_id
                || delegation.owner_npub != input.owner_npub
                || delegation.workspace_folder_id != input.folder.id
                || delegation.status != "active"
                || stored_folder != Some(&input.folder)
                || stored_access != Some(&requested_access)
                || !stored
                    .vault
                    .members
                    .iter()
                    .any(|member| member.user_id == input.agent_npub)
                || stored_grants != requested_grants
                || !sync_records_match
            {
                return Err(StoreError::BrokenInvariant {
                    reason:
                        "existing Brain Email Access Delegation does not match requested pairing"
                            .to_owned(),
                });
            }
            return Ok(EnsurePersonalAgentWorkspaceOutcome {
                delegation,
                duplicate: true,
            });
        }

        if vault
            .members
            .iter()
            .any(|member| member.user_id == input.agent_npub)
        {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "Agent Principal is already a Personal Vault member without this delegation"
                        .to_owned(),
            });
        }
        let pairing_count = self.conn.query_row(
            "SELECT COUNT(*) FROM brain_email_access_delegations WHERE vault_id = ?1",
            params![input.vault_id.as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        if pairing_count >= MAX_AGENT_WORKSPACE_PAIRINGS_PER_VAULT {
            return Err(StoreError::BrokenInvariant {
                reason: format!(
                    "Personal Vault Agent Workspace pairing count exceeds limit {MAX_AGENT_WORKSPACE_PAIRINGS_PER_VAULT}"
                ),
            });
        }
        if input.folder.access != FolderAccessMode::Restricted
            || input.folder.parent_folder_id.is_some()
            || input.folder.current_key_version != 1
        {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "initial Agent Workspace must be a top-level restricted Folder at key version 1"
                        .to_owned(),
            });
        }

        let access_user_ids = BTreeSet::from([input.agent_npub.clone()]);
        let mut paired_vault = vault.clone();
        paired_vault.members.push(VaultMember {
            user_id: input.agent_npub.clone(),
            folder_access: BTreeSet::from([input.folder.id.clone()]),
        });
        self.validate_folder_request(
            &paired_vault,
            &input.folder,
            &access_user_ids,
            &input.grants,
        )?;

        let scope_json =
            serde_json::to_string(&vec![input.folder.id.to_string()]).map_err(|error| {
                StoreError::InvalidRecord {
                    reason: format!("delegation scope did not serialize: {error}"),
                }
            })?;
        let audit_id = format!("{}-created", input.delegation_id);
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO vault_members (vault_id, user_id) VALUES (?1, ?2)",
            params![input.vault_id.as_str(), input.agent_npub.as_str()],
        )?;
        insert_folder(&tx, &input.vault_id, &input.folder, false)?;
        insert_folder_access(&tx, &input.vault_id, &input.folder.id, &access_user_ids)?;
        for grant in &input.grants {
            insert_grant(&tx, &input.vault_id, grant)?;
        }
        tx.execute(
            r#"
            INSERT INTO brain_email_access_delegations (
                id, vault_id, owner_npub, agent_npub, workspace_folder_id,
                scope_json, status, created_by_npub, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?3, ?7, ?7)
            "#,
            params![
                input.delegation_id,
                input.vault_id.as_str(),
                input.owner_npub.as_str(),
                input.agent_npub.as_str(),
                input.folder.id.as_str(),
                scope_json,
                input.created_at,
            ],
        )?;
        tx.execute(
            r#"
            INSERT INTO brain_email_access_delegation_audit (
                id, delegation_id, action, actor_npub, subject_npub,
                scope_json, occurred_at
            ) VALUES (?1, ?2, 'created', ?3, ?4, ?5, ?6)
            "#,
            params![
                audit_id,
                input.delegation_id,
                input.owner_npub.as_str(),
                input.agent_npub.as_str(),
                scope_json,
                input.created_at,
            ],
        )?;
        for record in &input.sync_records {
            sync_records::validate_sync_input(record)?;
            if sync_records::existing_sequence(&tx, &input.vault_id, record.record_event_id())?
                .is_some()
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "Agent Workspace sync record already exists outside this pairing"
                        .to_owned(),
                });
            }
            sync_records::validate_sync_conflict(&tx, &input.vault_id, record)?;
            let sequence = sync_records::next_sequence(&tx, &input.vault_id)?;
            sync_records::insert_sync_record(&tx, &input.vault_id, sequence, record)?;
            sync_records::project_sync_record(&tx, &input.vault_id, record)?;
        }
        tx.commit()?;

        let delegation = self
            .load_brain_email_access_delegation(&input.vault_id, &input.agent_npub)?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "created Brain Email Access Delegation could not be reloaded".to_owned(),
            })?;
        Ok(EnsurePersonalAgentWorkspaceOutcome {
            delegation,
            duplicate: false,
        })
    }

    pub fn list_brain_email_access_delegations(
        &self,
        vault_id: &VaultId,
    ) -> Result<Vec<BrainEmailAccessDelegation>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT agent_npub FROM brain_email_access_delegations WHERE vault_id = ?1 ORDER BY created_at, id LIMIT ?2",
        )?;
        let agent_npubs = statement
            .query_map(
                params![vault_id.as_str(), MAX_AGENT_WORKSPACE_PAIRINGS_PER_VAULT],
                |row| row.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);

        agent_npubs
            .into_iter()
            .map(|agent_npub| {
                let agent_npub = UserId::new(agent_npub)?;
                self.load_brain_email_access_delegation(vault_id, &agent_npub)?
                    .ok_or_else(|| StoreError::BrokenInvariant {
                        reason: "listed Brain Email Access Delegation could not be reloaded"
                            .to_owned(),
                    })
            })
            .collect()
    }

    fn load_brain_email_access_delegation(
        &self,
        vault_id: &VaultId,
        agent_npub: &UserId,
    ) -> Result<Option<BrainEmailAccessDelegation>, StoreError> {
        let row = self
            .conn
            .query_row(
                r#"
                SELECT id, owner_npub, workspace_folder_id, status,
                       created_by_npub, created_at, updated_at
                FROM brain_email_access_delegations
                WHERE vault_id = ?1 AND agent_npub = ?2
                "#,
                params![vault_id.as_str(), agent_npub.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, owner_npub, workspace_folder_id, status, created_by, created_at, updated_at)) =
            row
        else {
            return Ok(None);
        };
        let audit = load_delegation_audit(&self.conn, &id)?;
        Ok(Some(BrainEmailAccessDelegation {
            id,
            vault_id: vault_id.clone(),
            owner_npub: UserId::new(owner_npub)?,
            agent_npub: agent_npub.clone(),
            workspace_folder_id: FolderId::new(workspace_folder_id)?,
            status,
            created_by_npub: UserId::new(created_by)?,
            created_at,
            updated_at,
            audit,
        }))
    }
}

fn validate_pairing_sync_records(
    input: &EnsurePersonalAgentWorkspaceInput,
) -> Result<(), StoreError> {
    if input.sync_records.len() != input.grants.len() + 1 {
        return Err(StoreError::BrokenInvariant {
            reason: "Agent Workspace pairing requires one sync record per grant and one access-change record"
                .to_owned(),
        });
    }
    let mut grant_record_count = 0;
    let mut access_change_count = 0;
    for record in &input.sync_records {
        let SyncRecordInput::Control(record) = record else {
            return Err(StoreError::BrokenInvariant {
                reason: "Agent Workspace pairing only accepts control sync records".to_owned(),
            });
        };
        if record.folder_id.as_ref() != Some(&input.folder.id)
            || record.actor_npub != input.owner_npub
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Agent Workspace sync records must be owner-authored for the paired Folder"
                    .to_owned(),
            });
        }
        match record.record_type {
            SyncRecordType::FolderKeyGrant => grant_record_count += 1,
            SyncRecordType::VaultAdminAccessChange => access_change_count += 1,
            _ => {
                return Err(StoreError::BrokenInvariant {
                    reason: "Agent Workspace pairing received an unrelated sync record".to_owned(),
                });
            }
        }
    }
    if grant_record_count != input.grants.len() || access_change_count != 1 {
        return Err(StoreError::BrokenInvariant {
            reason: "Agent Workspace pairing sync record set is incomplete".to_owned(),
        });
    }
    Ok(())
}

fn load_delegation_audit(
    conn: &Connection,
    delegation_id: &str,
) -> Result<Vec<BrainEmailAccessDelegationAudit>, StoreError> {
    let mut statement = conn.prepare(
        r#"
        SELECT id, action, actor_npub, subject_npub, scope_json, occurred_at
        FROM brain_email_access_delegation_audit
        WHERE delegation_id = ?1
        ORDER BY occurred_at, id
        LIMIT ?2
        "#,
    )?;
    let rows = statement.query_map(params![delegation_id, MAX_DELEGATION_AUDIT_ROWS], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut audit = Vec::new();
    for row in rows {
        let (id, action, actor_npub, subject_npub, scope_json, occurred_at) = row?;
        let folder_ids = serde_json::from_str::<Vec<String>>(&scope_json)
            .map_err(|error| StoreError::BrokenInvariant {
                reason: format!("stored delegation scope is invalid: {error}"),
            })?
            .into_iter()
            .map(FolderId::new)
            .collect::<Result<Vec<_>, _>>()?;
        audit.push(BrainEmailAccessDelegationAudit {
            id,
            action,
            actor_npub: UserId::new(actor_npub)?,
            subject_npub: UserId::new(subject_npub)?,
            folder_ids,
            occurred_at,
        });
    }
    Ok(audit)
}
