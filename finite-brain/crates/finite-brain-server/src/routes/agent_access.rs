use crate::*;

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
    let folder_id = delegation.workspace_folder_id.to_string();
    AgentWorkspacePairingResponse {
        delegation_id: delegation.id,
        vault_id: delegation.vault_id.to_string(),
        owner_npub: delegation.owner_npub.to_string(),
        agent_npub: delegation.agent_npub.to_string(),
        workspace_folder_id: folder_id.clone(),
        scope: AgentWorkspaceScopeResponse {
            folder_ids: vec![folder_id],
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
