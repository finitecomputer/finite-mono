use crate::*;

pub(crate) async fn bootstrap_personal_vault_for_agent_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Json<BootstrapPersonalVaultForAgentResponse>, ApiError> {
    let agent_actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: BootstrapPersonalVaultForAgentRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(request.vault_id.clone())?;
    let folder = Folder {
        id: FolderId::new(request.folder_id)?,
        name: DisplayName::new("folder_name", request.folder_name)?,
        role: finite_brain_core::FolderRole::Folder,
        access: FolderAccessMode::Restricted,
        parent_folder_id: None,
        path: SafeRelativePath::new("folder_path", request.folder_path)?,
        current_key_version: 1,
        shared_folder_source: false,
    };
    let authorization_event = event_from_value(request.bootstrap_authorization)?;
    let authorization = validate_personal_vault_bootstrap_authorization_event(
        &authorization_event,
        &agent_actor,
        &vault_id,
        &folder.id,
        state.auth_now_unix_seconds(),
    )?;
    let owner_identity = resolve_and_record_identity(&state, &authorization.owner_npub)?;
    let owner_npub = UserId::new(owner_identity.npub)?;
    let agent_npub = UserId::new(agent_actor.clone())?;
    let (access_change_event, access_change_payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        owner_npub.as_str(),
        AdminAccessAction::SetFolderAccessMode,
        Some(&folder.id),
        None,
        Some(1),
    )?;
    let output = bootstrap_personal_vault(vault_id.as_str(), request.name, owner_npub.to_string())?;
    validate_bootstrap_grant_requests(&request.bootstrap_grants, &output.required_key_grants)?;
    let created_at = server_timestamp(&state);
    let bootstrap_grants = bootstrap_grant_requests_to_metadata(
        &request.bootstrap_grants,
        owner_npub.as_str(),
        &created_at,
    )?;
    let workspace_grants = grant_requests_to_metadata(
        &request.workspace_grants,
        &folder.id,
        owner_npub.as_str(),
        Some(access_change_event.as_json()),
        &created_at,
    )?;
    let mut sync_records = workspace_grants
        .iter()
        .map(folder_key_grant_sync_record)
        .collect::<Result<Vec<_>, _>>()?;
    sync_records.push(admin_access_change_sync_record(
        owner_npub.as_str(),
        &access_change_event,
        &access_change_payload,
    )?);
    let delegation_id = generated_link_id(
        "brain-email-access-delegation",
        &[vault_id.as_str(), owner_npub.as_str(), agent_npub.as_str()],
        12,
    );
    let outcome = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.bootstrap_personal_agent_workspace(&BootstrapPersonalAgentWorkspaceInput {
            authorization_id: authorization.authorization_id,
            authorization_event_id: authorization_event.id.to_hex(),
            authorization_expires_at: authorization.expires_at,
            vault: output,
            bootstrap_grants,
            pairing: EnsurePersonalAgentWorkspaceInput {
                delegation_id,
                vault_id: vault_id.clone(),
                owner_npub,
                agent_npub: agent_npub.clone(),
                folder,
                grants: workspace_grants,
                sync_records,
                created_at,
            },
            consumed_at: server_timestamp(&state),
        })?
    };

    let mut vault = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        let mounted = store.mounted_folder_projection(&vault_id, &agent_npub)?;
        let mut response = metadata_response_for_actor(stored, mounted, agent_npub.as_str());
        enrich_metadata_identities(&store, &mut response)?;
        response
    };
    vault
        .identities
        .sort_by(|left, right| left.npub.cmp(&right.npub));
    Ok(Json(BootstrapPersonalVaultForAgentResponse {
        vault,
        pairing: agent_workspace_pairing_response(outcome.delegation, false),
    }))
}

pub(crate) async fn ensure_agent_workspace_pairing_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<AgentWorkspacePairingResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: EnsureAgentWorkspacePairingRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let agent_identity = resolve_and_record_identity(&state, &request.agent_npub)?;
    let agent_npub = UserId::new(agent_identity.npub)?;
    let owner_npub = UserId::new(actor.clone())?;
    let folder = Folder {
        id: FolderId::new(request.folder_id)?,
        name: DisplayName::new("folder_name", request.name)?,
        role: finite_brain_core::FolderRole::Folder,
        access: FolderAccessMode::Restricted,
        parent_folder_id: None,
        path: SafeRelativePath::new("folder_path", request.path)?,
        current_key_version: 1,
        shared_folder_source: false,
    };

    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        if stored.vault.kind != VaultKind::Personal
            || stored.vault.owner_user_id.as_ref() != Some(&owner_npub)
        {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "Personal Vault owner access required",
            ));
        }
    }

    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        &actor,
        AdminAccessAction::SetFolderAccessMode,
        Some(&folder.id),
        None,
        Some(1),
    )?;
    let created_at = server_timestamp(&state);
    let grants = grant_requests_to_metadata(
        &request.grants,
        &folder.id,
        &actor,
        Some(event.as_json()),
        &created_at,
    )?;
    let mut sync_records = grants
        .iter()
        .map(folder_key_grant_sync_record)
        .collect::<Result<Vec<_>, _>>()?;
    sync_records.push(admin_access_change_sync_record(&actor, &event, &payload)?);
    let delegation_id = generated_link_id(
        "brain-email-access-delegation",
        &[vault_id.as_str(), owner_npub.as_str(), agent_npub.as_str()],
        12,
    );
    let outcome = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.ensure_personal_agent_workspace(&EnsurePersonalAgentWorkspaceInput {
            delegation_id,
            vault_id: vault_id.clone(),
            owner_npub,
            agent_npub,
            folder,
            grants: grants.clone(),
            sync_records,
            created_at,
        })?
    };

    Ok(Json(agent_workspace_pairing_response(
        outcome.delegation,
        outcome.duplicate,
    )))
}

pub(crate) async fn expand_agent_workspace_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, agent_npub, folder_id)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> Result<Json<AgentWorkspacePairingResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: ExpandAgentWorkspaceRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let owner_npub = UserId::new(actor.clone())?;
    let agent_identity = resolve_and_record_identity(&state, &agent_npub)?;
    let agent_npub = UserId::new(agent_identity.npub)?;
    let current_key_version = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        if stored.vault.kind != VaultKind::Personal
            || stored.vault.owner_user_id.as_ref() != Some(&owner_npub)
        {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "Personal Vault owner access required",
            ));
        }
        folder_current_key_version(&stored, &folder_id)?
    };
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        &actor,
        AdminAccessAction::GrantFolderAccess,
        Some(&folder_id),
        Some(agent_npub.as_str()),
        Some(current_key_version),
    )?;
    let changed_at = server_timestamp(&state);
    let mut grant_request = request.grant;
    grant_request.recipient_npub = agent_npub.to_string();
    let grant = grant_request_to_metadata(
        &grant_request,
        &folder_id,
        &actor,
        Some(event.as_json()),
        &changed_at,
    )?;
    let sync_records = vec![
        folder_key_grant_sync_record(&grant)?,
        admin_access_change_sync_record(&actor, &event, &payload)?,
    ];
    let delegation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.expand_personal_agent_workspace(&ExpandPersonalAgentWorkspaceInput {
            vault_id,
            owner_npub,
            agent_npub,
            folder_id,
            grant,
            sync_records,
            changed_at,
        })?
    };
    Ok(Json(agent_workspace_pairing_response(delegation, false)))
}

pub(crate) async fn revoke_agent_workspace_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, agent_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<AgentWorkspacePairingResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: RevokeAgentWorkspaceRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let owner_npub = UserId::new(actor.clone())?;
    let agent_identity = resolve_and_record_identity(&state, &agent_npub)?;
    let agent_npub = UserId::new(agent_identity.npub)?;
    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        if stored.vault.kind != VaultKind::Personal
            || stored.vault.owner_user_id.as_ref() != Some(&owner_npub)
        {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "Personal Vault owner access required",
            ));
        }
    }

    let changed_at = server_timestamp(&state);
    let mut rotations = Vec::with_capacity(request.folders.len());
    for folder_request in request.folders {
        let folder_id = FolderId::new(folder_request.folder_id)?;
        let (event, payload) = validate_admin_access_change_value(
            folder_request.access_change_event,
            &vault_id,
            &actor,
            AdminAccessAction::RemoveFolderAccess,
            Some(&folder_id),
            Some(agent_npub.as_str()),
            Some(folder_request.new_key_version),
        )?;
        let grants = grant_requests_to_metadata(
            &folder_request.grants,
            &folder_id,
            &actor,
            Some(event.as_json()),
            &changed_at,
        )?;
        let reencrypted_records = rotation_records_from_requests(
            &vault_id,
            &folder_id,
            &actor,
            folder_request.new_key_version,
            folder_request.reencrypted_records,
        )?;
        let mut sync_records = grants
            .iter()
            .map(folder_key_grant_sync_record)
            .collect::<Result<Vec<_>, _>>()?;
        sync_records.push(admin_access_change_sync_record(&actor, &event, &payload)?);
        rotations.push(RevokePersonalAgentFolderInput {
            folder_id,
            new_key_version: folder_request.new_key_version,
            grants,
            reencrypted_records,
            sync_records,
        });
    }

    let delegation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.revoke_personal_agent_workspace(&RevokePersonalAgentWorkspaceInput {
            vault_id,
            owner_npub,
            agent_npub,
            folders: rotations,
            changed_at,
        })?
    };
    Ok(Json(agent_workspace_pairing_response(delegation, false)))
}

pub(crate) async fn list_agent_workspace_pairings_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
) -> Result<Json<AgentWorkspacePairingListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let vault_id = VaultId::new(vault_id)?;
    let pairings = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        if stored.vault.kind != VaultKind::Personal
            || stored.vault.owner_user_id.as_ref() != Some(&actor)
        {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "Personal Vault owner access required",
            ));
        }
        store.list_brain_email_access_delegations(&vault_id)?
    };
    Ok(Json(AgentWorkspacePairingListResponse {
        pairings: pairings
            .into_iter()
            .map(|delegation| agent_workspace_pairing_response(delegation, false))
            .collect(),
    }))
}

fn agent_workspace_pairing_response(
    delegation: BrainEmailAccessDelegation,
    duplicate: bool,
) -> AgentWorkspacePairingResponse {
    AgentWorkspacePairingResponse {
        delegation_id: delegation.id,
        vault_id: delegation.vault_id.to_string(),
        owner_npub: delegation.owner_npub.to_string(),
        agent_npub: delegation.agent_npub.to_string(),
        workspace_folder_id: delegation.workspace_folder_id.to_string(),
        scope: AgentWorkspaceScopeResponse {
            folder_ids: delegation
                .folder_ids
                .iter()
                .map(ToString::to_string)
                .collect(),
            permission: "read_write".to_owned(),
        },
        status: delegation.status,
        created_by_npub: delegation.created_by_npub.to_string(),
        created_at: delegation.created_at,
        updated_at: delegation.updated_at,
        audit: delegation
            .audit
            .into_iter()
            .map(|audit| AgentWorkspacePairingAuditResponse {
                id: audit.id,
                action: audit.action,
                actor_npub: audit.actor_npub.to_string(),
                subject_npub: audit.subject_npub.to_string(),
                folder_ids: audit
                    .folder_ids
                    .into_iter()
                    .map(|folder_id| folder_id.to_string())
                    .collect(),
                occurred_at: audit.occurred_at,
            })
            .collect(),
        duplicate,
    }
}
