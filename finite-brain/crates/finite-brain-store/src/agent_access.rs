use crate::*;

const MAX_AGENT_WORKSPACE_PAIRINGS_PER_VAULT: i64 = 100;
const MAX_AGENT_DELEGATED_FOLDERS: usize = 100;
const MAX_DELEGATION_AUDIT_ROWS: i64 = MAX_AGENT_DELEGATED_FOLDERS as i64 + 2;

impl BrainStore {
    /// Atomically consume an owner authorization and create the user-owned Personal Vault plus
    /// its first restricted Agent Workspace.
    pub fn bootstrap_personal_agent_workspace(
        &mut self,
        input: &BootstrapPersonalAgentWorkspaceInput,
    ) -> Result<BootstrapPersonalAgentWorkspaceOutcome, StoreError> {
        if input.vault.vault.folders.len() > MAX_BOOTSTRAP_FOLDERS {
            return Err(StoreError::BrokenInvariant {
                reason: format!("bootstrap folder count exceeds limit {MAX_BOOTSTRAP_FOLDERS}"),
            });
        }
        if input.bootstrap_grants.len() > MAX_BOOTSTRAP_GRANTS {
            return Err(StoreError::BrokenInvariant {
                reason: format!("bootstrap grant count exceeds limit {MAX_BOOTSTRAP_GRANTS}"),
            });
        }
        validate_bootstrap_output(&input.vault)?;
        validate_required_grants(
            &input.vault.vault,
            &input.vault.required_key_grants,
            &input.bootstrap_grants,
        )?;
        let owner_npub = input.vault.vault.owner_user_id.as_ref().ok_or_else(|| {
            StoreError::BrokenInvariant {
                reason: "agent-first bootstrap requires a Personal Vault owner".to_owned(),
            }
        })?;
        if input.vault.vault.kind != VaultKind::Personal
            || input.pairing.vault_id != input.vault.vault.id
            || &input.pairing.owner_npub != owner_npub
            || input.pairing.agent_npub == *owner_npub
        {
            return Err(StoreError::BrokenInvariant {
                reason: "agent-first bootstrap identities do not match the Personal Vault"
                    .to_owned(),
            });
        }
        validate_pairing_sync_records(&input.pairing)?;
        if input.pairing.folder.access != FolderAccessMode::Restricted
            || input.pairing.folder.parent_folder_id.is_some()
            || input.pairing.folder.current_key_version != 1
        {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "initial Agent Workspace must be a top-level restricted Folder at key version 1"
                        .to_owned(),
            });
        }

        let authorization_used = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM personal_vault_bootstrap_authorizations WHERE authorization_id = ?1 OR authorization_event_id = ?2)",
            params![input.authorization_id, input.authorization_event_id],
            |row| row.get::<_, bool>(0),
        )?;
        if authorization_used {
            return Err(StoreError::BrokenInvariant {
                reason: "Personal Vault Bootstrap Authorization was already consumed".to_owned(),
            });
        }
        let existing_personal_vault_id = self
            .conn
            .query_row(
                "SELECT id FROM vaults WHERE kind = 'personal' AND owner_user_id = ?1",
                params![owner_npub.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if existing_personal_vault_id
            .as_deref()
            .is_some_and(|vault_id| vault_id != input.pairing.vault_id.as_str())
        {
            return Err(StoreError::BrokenInvariant {
                reason: "agent-first bootstrap names a different Personal Vault".to_owned(),
            });
        }
        let pairing_count = self.conn.query_row(
            "SELECT COUNT(*) FROM brain_email_access_delegations WHERE vault_id = ?1",
            params![input.pairing.vault_id.as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        if pairing_count >= MAX_AGENT_WORKSPACE_PAIRINGS_PER_VAULT {
            return Err(StoreError::BrokenInvariant {
                reason: format!(
                    "Personal Vault Agent Workspace pairing count exceeds limit {MAX_AGENT_WORKSPACE_PAIRINGS_PER_VAULT}"
                ),
            });
        }

        let access_user_ids = BTreeSet::from([input.pairing.agent_npub.clone()]);
        let mut paired_vault = if existing_personal_vault_id.is_some() {
            self.load_core_vault(&input.pairing.vault_id)?
        } else {
            input.vault.vault.clone()
        };
        paired_vault.members.push(VaultMember {
            user_id: input.pairing.agent_npub.clone(),
            folder_access: BTreeSet::from([input.pairing.folder.id.clone()]),
        });
        self.validate_folder_request(
            &paired_vault,
            &input.pairing.folder,
            &access_user_ids,
            &input.pairing.grants,
        )?;

        let scope_json = serde_json::to_string(&vec![input.pairing.folder.id.to_string()])
            .map_err(|error| StoreError::InvalidRecord {
                reason: format!("delegation scope did not serialize: {error}"),
            })?;
        let audit_id = format!("{}-created", input.pairing.delegation_id);
        let tx = self.conn.transaction()?;
        if existing_personal_vault_id.is_none() {
            insert_vault(&tx, &input.vault.vault)?;
            insert_members_and_admins(&tx, &input.vault.vault)?;
            for folder in &input.vault.vault.folders {
                insert_folder(&tx, &input.vault.vault.id, folder, false)?;
            }
            for grant in &input.bootstrap_grants {
                insert_grant(&tx, &input.vault.vault.id, grant)?;
            }
        }
        tx.execute(
            "INSERT INTO vault_members (vault_id, user_id) VALUES (?1, ?2)",
            params![
                input.pairing.vault_id.as_str(),
                input.pairing.agent_npub.as_str()
            ],
        )?;
        insert_folder(&tx, &input.pairing.vault_id, &input.pairing.folder, false)?;
        insert_folder_access(
            &tx,
            &input.pairing.vault_id,
            &input.pairing.folder.id,
            &access_user_ids,
        )?;
        for grant in &input.pairing.grants {
            insert_grant(&tx, &input.pairing.vault_id, grant)?;
        }
        tx.execute(
            r#"
            INSERT INTO brain_email_access_delegations (
                id, vault_id, owner_npub, agent_npub, workspace_folder_id,
                scope_json, status, created_by_npub, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?3, ?7, ?7)
            "#,
            params![
                input.pairing.delegation_id,
                input.pairing.vault_id.as_str(),
                input.pairing.owner_npub.as_str(),
                input.pairing.agent_npub.as_str(),
                input.pairing.folder.id.as_str(),
                scope_json,
                input.pairing.created_at,
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
                input.pairing.delegation_id,
                input.pairing.owner_npub.as_str(),
                input.pairing.agent_npub.as_str(),
                scope_json,
                input.pairing.created_at,
            ],
        )?;
        for record in &input.pairing.sync_records {
            sync_records::validate_sync_input(record)?;
            sync_records::validate_sync_conflict(&tx, &input.pairing.vault_id, record)?;
            let sequence = sync_records::next_sequence(&tx, &input.pairing.vault_id)?;
            sync_records::insert_sync_record(&tx, &input.pairing.vault_id, sequence, record)?;
            sync_records::project_sync_record(&tx, &input.pairing.vault_id, record)?;
        }
        tx.execute(
            r#"
            INSERT INTO personal_vault_bootstrap_authorizations (
                authorization_id, authorization_event_id, owner_npub, agent_npub,
                vault_id, workspace_folder_id, expires_at, consumed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                input.authorization_id,
                input.authorization_event_id,
                input.pairing.owner_npub.as_str(),
                input.pairing.agent_npub.as_str(),
                input.pairing.vault_id.as_str(),
                input.pairing.folder.id.as_str(),
                input.authorization_expires_at,
                input.consumed_at,
            ],
        )?;
        tx.commit()?;

        let delegation = self
            .load_brain_email_access_delegation(&input.pairing.vault_id, &input.pairing.agent_npub)?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "created Brain Email Access Delegation could not be reloaded".to_owned(),
            })?;
        Ok(BootstrapPersonalAgentWorkspaceOutcome { delegation })
    }

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

    /// Add one restricted Folder to an active agent delegation under Personal Vault owner
    /// authority, including its current encrypted grant, audit entry, and sync records.
    pub fn expand_personal_agent_workspace(
        &mut self,
        input: &ExpandPersonalAgentWorkspaceInput,
    ) -> Result<BrainEmailAccessDelegation, StoreError> {
        let stored = self.load_vault(&input.vault_id)?;
        if stored.vault.kind != VaultKind::Personal
            || stored.vault.owner_user_id.as_ref() != Some(&input.owner_npub)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Agent Workspace expansion requires the Personal Vault owner".to_owned(),
            });
        }
        let delegation = self
            .load_brain_email_access_delegation(&input.vault_id, &input.agent_npub)?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "active Brain Email Access Delegation is required".to_owned(),
            })?;
        if delegation.status != "active" {
            return Err(StoreError::BrokenInvariant {
                reason: "active Brain Email Access Delegation is required".to_owned(),
            });
        }
        if delegation.folder_ids.len() >= MAX_AGENT_DELEGATED_FOLDERS {
            return Err(StoreError::BrokenInvariant {
                reason: format!(
                    "Agent Workspace Folder scope exceeds limit {MAX_AGENT_DELEGATED_FOLDERS}"
                ),
            });
        }
        if delegation.folder_ids.contains(&input.folder_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "Agent Principal already has this delegated Folder scope".to_owned(),
            });
        }
        let folder = stored
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == input.folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: input.folder_id.to_string(),
            })?;
        if folder.access != FolderAccessMode::Restricted {
            return Err(StoreError::BrokenInvariant {
                reason: "Agent Workspace expansion requires a restricted Folder".to_owned(),
            });
        }
        if stored
            .folder_access
            .get(&input.folder_id)
            .is_some_and(|members| members.contains(&input.agent_npub))
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Agent Principal already has Folder access outside this delegation"
                    .to_owned(),
            });
        }
        validate_grant_metadata(&input.grant)?;
        validate_grant_issuer(&stored.vault, &input.grant)?;
        if input.grant.folder_id != input.folder_id
            || input.grant.key_version != folder.current_key_version
            || input.grant.recipient_npub != input.agent_npub
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Agent Workspace expansion grant does not match the Folder scope"
                    .to_owned(),
            });
        }
        validate_agent_scope_change_sync_records(
            &input.sync_records,
            &input.owner_npub,
            &input.folder_id,
            1,
        )?;

        let mut folder_ids = delegation.folder_ids.clone();
        folder_ids.push(input.folder_id.clone());
        folder_ids.sort();
        folder_ids.dedup();
        let scope_json = serde_json::to_string(
            &folder_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )
        .map_err(|error| StoreError::InvalidRecord {
            reason: format!("delegation scope did not serialize: {error}"),
        })?;
        let audit_id = format!("{}-scope-expanded-{}", delegation.id, input.folder_id);
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO folder_access (vault_id, folder_id, user_id) VALUES (?1, ?2, ?3)",
            params![
                input.vault_id.as_str(),
                input.folder_id.as_str(),
                input.agent_npub.as_str()
            ],
        )?;
        insert_grant(&tx, &input.vault_id, &input.grant)?;
        tx.execute(
            "UPDATE brain_email_access_delegations SET scope_json = ?3, updated_at = ?4 WHERE vault_id = ?1 AND agent_npub = ?2 AND status = 'active'",
            params![
                input.vault_id.as_str(),
                input.agent_npub.as_str(),
                scope_json,
                input.changed_at,
            ],
        )?;
        tx.execute(
            r#"
            INSERT INTO brain_email_access_delegation_audit (
                id, delegation_id, action, actor_npub, subject_npub, scope_json, occurred_at
            ) VALUES (?1, ?2, 'scope_expanded', ?3, ?4, ?5, ?6)
            "#,
            params![
                audit_id,
                delegation.id,
                input.owner_npub.as_str(),
                input.agent_npub.as_str(),
                scope_json,
                input.changed_at,
            ],
        )?;
        for record in &input.sync_records {
            sync_records::validate_sync_input(record)?;
            sync_records::validate_sync_conflict(&tx, &input.vault_id, record)?;
            let sequence = sync_records::next_sequence(&tx, &input.vault_id)?;
            sync_records::insert_sync_record(&tx, &input.vault_id, sequence, record)?;
            sync_records::project_sync_record(&tx, &input.vault_id, record)?;
        }
        tx.commit()?;
        self.load_brain_email_access_delegation(&input.vault_id, &input.agent_npub)?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "expanded Brain Email Access Delegation could not be reloaded".to_owned(),
            })
    }

    /// Revoke an active agent delegation and rotate every Folder in its current scope in one
    /// transaction. Previous ciphertext and keys cannot be erased from the former recipient.
    pub fn revoke_personal_agent_workspace(
        &mut self,
        input: &RevokePersonalAgentWorkspaceInput,
    ) -> Result<BrainEmailAccessDelegation, StoreError> {
        let stored = self.load_vault(&input.vault_id)?;
        if stored.vault.kind != VaultKind::Personal
            || stored.vault.owner_user_id.as_ref() != Some(&input.owner_npub)
        {
            return Err(StoreError::BrokenInvariant {
                reason: "Agent Workspace revocation requires the Personal Vault owner".to_owned(),
            });
        }
        let delegation = self
            .load_brain_email_access_delegation(&input.vault_id, &input.agent_npub)?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "active Brain Email Access Delegation is required".to_owned(),
            })?;
        if delegation.status != "active" || delegation.owner_npub != input.owner_npub {
            return Err(StoreError::BrokenInvariant {
                reason: "active Brain Email Access Delegation is required".to_owned(),
            });
        }

        let mut expected_folder_ids = delegation.folder_ids.clone();
        expected_folder_ids.sort();
        expected_folder_ids.dedup();
        let mut provided_folder_ids = input
            .folders
            .iter()
            .map(|folder| folder.folder_id.clone())
            .collect::<Vec<_>>();
        let provided_count = provided_folder_ids.len();
        provided_folder_ids.sort();
        provided_folder_ids.dedup();
        if provided_count != provided_folder_ids.len() || provided_folder_ids != expected_folder_ids
        {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "Agent Workspace revocation must rotate every delegated Folder exactly once"
                        .to_owned(),
            });
        }

        let live_objects = self.load_current_objects(&input.vault_id)?;
        for rotation in &input.folders {
            let folder = stored
                .vault
                .folders
                .iter()
                .find(|folder| folder.id == rotation.folder_id)
                .ok_or_else(|| StoreError::MissingFolder {
                    folder_id: rotation.folder_id.to_string(),
                })?;
            if folder.access != FolderAccessMode::Restricted
                || rotation.new_key_version != folder.current_key_version + 1
            {
                return Err(StoreError::BrokenInvariant {
                    reason: "Agent Workspace revocation requires the next key version for every restricted Folder"
                        .to_owned(),
                });
            }
            let mut remaining_access = stored
                .folder_access
                .get(&rotation.folder_id)
                .cloned()
                .unwrap_or_default();
            if !remaining_access.remove(&input.agent_npub) {
                return Err(StoreError::BrokenInvariant {
                    reason: "Agent Principal does not have every delegated Folder access"
                        .to_owned(),
                });
            }
            let mut rotated_folder = folder.clone();
            rotated_folder.current_key_version = rotation.new_key_version;
            let required = required_recipients(&stored.vault, &rotated_folder, &remaining_access)?;
            validate_folder_grants(&stored.vault, &rotated_folder, &required, &rotation.grants)?;
            let folder_objects = live_objects
                .iter()
                .filter(|object| object.folder_id == rotation.folder_id && !object.deleted)
                .cloned()
                .collect::<Vec<_>>();
            validate_rotation_records(&folder_objects, &rotation.reencrypted_records)?;
            validate_agent_scope_change_sync_records(
                &rotation.sync_records,
                &input.owner_npub,
                &rotation.folder_id,
                rotation.grants.len(),
            )?;
        }

        let scope_json = serde_json::to_string(
            &expected_folder_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )
        .map_err(|error| StoreError::InvalidRecord {
            reason: format!("delegation scope did not serialize: {error}"),
        })?;
        let tx = self.conn.transaction()?;
        for rotation in &input.folders {
            tx.execute(
                "DELETE FROM folder_access WHERE vault_id = ?1 AND folder_id = ?2 AND user_id = ?3",
                params![
                    input.vault_id.as_str(),
                    rotation.folder_id.as_str(),
                    input.agent_npub.as_str()
                ],
            )?;
            tx.execute(
                "UPDATE folders SET current_key_version = ?3 WHERE vault_id = ?1 AND id = ?2",
                params![
                    input.vault_id.as_str(),
                    rotation.folder_id.as_str(),
                    rotation.new_key_version
                ],
            )?;
            invalidate_pending_email_bootstraps_for_rotated_folder(
                &tx,
                &input.vault_id,
                &rotation.folder_id,
                &input.changed_at,
            )?;
            for grant in &rotation.grants {
                insert_grant(&tx, &input.vault_id, grant)?;
            }
            for record in &rotation.reencrypted_records {
                let input_record = SyncRecordInput::FolderObjectRevision(record.clone());
                sync_records::validate_sync_input(&input_record)?;
                sync_records::validate_sync_conflict(&tx, &input.vault_id, &input_record)?;
                let sequence = sync_records::next_sequence(&tx, &input.vault_id)?;
                sync_records::insert_sync_record(&tx, &input.vault_id, sequence, &input_record)?;
                sync_records::project_sync_record(&tx, &input.vault_id, &input_record)?;
            }
            for record in &rotation.sync_records {
                sync_records::validate_sync_input(record)?;
                sync_records::validate_sync_conflict(&tx, &input.vault_id, record)?;
                let sequence = sync_records::next_sequence(&tx, &input.vault_id)?;
                sync_records::insert_sync_record(&tx, &input.vault_id, sequence, record)?;
                sync_records::project_sync_record(&tx, &input.vault_id, record)?;
            }
        }
        let has_remaining_scope = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM folder_access WHERE vault_id = ?1 AND user_id = ?2)",
            params![input.vault_id.as_str(), input.agent_npub.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        if !has_remaining_scope {
            tx.execute(
                "DELETE FROM vault_members WHERE vault_id = ?1 AND user_id = ?2",
                params![input.vault_id.as_str(), input.agent_npub.as_str()],
            )?;
        }
        tx.execute(
            "UPDATE brain_email_access_delegations SET status = 'revoked', updated_at = ?3, revoked_at = ?3 WHERE vault_id = ?1 AND agent_npub = ?2 AND status = 'active'",
            params![
                input.vault_id.as_str(),
                input.agent_npub.as_str(),
                input.changed_at,
            ],
        )?;
        tx.execute(
            r#"
            INSERT INTO brain_email_access_delegation_audit (
                id, delegation_id, action, actor_npub, subject_npub, scope_json, occurred_at
            ) VALUES (?1, ?2, 'revoked', ?3, ?4, ?5, ?6)
            "#,
            params![
                format!("{}-revoked", delegation.id),
                delegation.id,
                input.owner_npub.as_str(),
                input.agent_npub.as_str(),
                scope_json,
                input.changed_at,
            ],
        )?;
        tx.commit()?;

        self.load_brain_email_access_delegation(&input.vault_id, &input.agent_npub)?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "revoked Brain Email Access Delegation could not be reloaded".to_owned(),
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
                SELECT id, owner_npub, workspace_folder_id, scope_json, status,
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
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            id,
            owner_npub,
            workspace_folder_id,
            scope_json,
            status,
            created_by,
            created_at,
            updated_at,
        )) = row
        else {
            return Ok(None);
        };
        let folder_ids = serde_json::from_str::<Vec<String>>(&scope_json)
            .map_err(|error| StoreError::BrokenInvariant {
                reason: format!("stored delegation scope is invalid: {error}"),
            })?
            .into_iter()
            .map(FolderId::new)
            .collect::<Result<Vec<_>, _>>()?;
        let audit = load_delegation_audit(&self.conn, &id)?;
        Ok(Some(BrainEmailAccessDelegation {
            id,
            vault_id: vault_id.clone(),
            owner_npub: UserId::new(owner_npub)?,
            agent_npub: agent_npub.clone(),
            workspace_folder_id: FolderId::new(workspace_folder_id)?,
            folder_ids,
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

fn validate_agent_scope_change_sync_records(
    records: &[SyncRecordInput],
    owner_npub: &UserId,
    folder_id: &FolderId,
    expected_grants: usize,
) -> Result<(), StoreError> {
    if records.len() != expected_grants + 1 {
        return Err(StoreError::BrokenInvariant {
            reason: "Agent Workspace scope change sync record set is incomplete".to_owned(),
        });
    }
    let mut grant_records = 0;
    let mut access_records = 0;
    for record in records {
        let SyncRecordInput::Control(record) = record else {
            return Err(StoreError::BrokenInvariant {
                reason: "Agent Workspace scope changes only accept control sync records".to_owned(),
            });
        };
        if record.actor_npub != *owner_npub || record.folder_id.as_ref() != Some(folder_id) {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "Agent Workspace scope change records must be owner-authored for the Folder"
                        .to_owned(),
            });
        }
        match record.record_type {
            SyncRecordType::FolderKeyGrant => grant_records += 1,
            SyncRecordType::VaultAdminAccessChange => access_records += 1,
            _ => {
                return Err(StoreError::BrokenInvariant {
                    reason: "Agent Workspace scope change received an unrelated sync record"
                        .to_owned(),
                });
            }
        }
    }
    if grant_records != expected_grants || access_records != 1 {
        return Err(StoreError::BrokenInvariant {
            reason: "Agent Workspace scope change sync record set is incomplete".to_owned(),
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
        ORDER BY occurred_at,
                 CASE action
                     WHEN 'created' THEN 0
                     WHEN 'scope_expanded' THEN 1
                     WHEN 'revoked' THEN 2
                 END,
                 id
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
