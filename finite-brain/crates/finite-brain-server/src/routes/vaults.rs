use crate::*;

pub(crate) async fn list_vaults_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<VisibleVaultsResponse>, ApiError> {
    let actor_npub = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor_npub)?;
    let vaults = {
        let store = state.store.lock().map_err(lock_error)?;
        store.list_visible_vaults(&actor)?
    };

    Ok(Json(visible_vaults_response(vaults)))
}

pub(crate) async fn create_vault_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Json<VaultMetadataResponse>, ApiError> {
    let actor_npub = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateVaultRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;

    let output = match request.kind {
        CreateVaultKind::Personal => {
            bootstrap_personal_vault(request.vault_id, request.name, actor_npub.clone())?
        }
        CreateVaultKind::Organization => {
            bootstrap_organization_vault(request.vault_id, request.name, actor_npub.clone())?
        }
    };
    let vault_id = output.vault.id.clone();
    let grants = if request.bootstrap_grants.is_empty() {
        grants_for_required(&output.required_key_grants, &vault_id, &actor_npub)
    } else {
        validate_bootstrap_grant_requests(&request.bootstrap_grants, &output.required_key_grants)?;
        bootstrap_grant_requests_to_metadata(
            &request.bootstrap_grants,
            &actor_npub,
            &server_timestamp(&state),
        )?
    };

    let stored = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.create_vault_bootstrap(&output, &grants)?;
        store.load_vault(&vault_id)?
    };

    let mut response = metadata_response(stored);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_metadata_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn vault_metadata_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
) -> Result<Json<VaultMetadataResponse>, ApiError> {
    let actor_npub = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let vault_id = VaultId::new(vault_id)?;

    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_vault(&vault_id)?
    };
    ensure_metadata_visible(&stored, &actor_npub)?;
    let mounted_folders = {
        let store = state.store.lock().map_err(lock_error)?;
        store.mounted_folder_projection(&vault_id, &UserId::new(actor_npub.clone())?)?
    };

    let mut response = metadata_response_with_mounts(stored, mounted_folders);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_metadata_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn encrypted_vault_export_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
) -> Result<Json<EncryptedVaultExportResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor_id = UserId::new(actor.clone())?;
    let vault_id = VaultId::new(vault_id)?;
    let export = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        ensure_metadata_visible(&stored, &actor)?;
        store.encrypted_vault_export(&vault_id, &actor_id)?
    };
    Ok(Json(encrypted_vault_export_response(export)))
}

pub(crate) async fn vault_search_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let vault_id = VaultId::new(vault_id)?;
    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        ensure_metadata_visible(&stored, &actor)?;
    }
    Err(ApiError::new(
        StatusCode::BAD_REQUEST,
        "plaintext search is client-side only over decrypted accessible content",
    ))
}

pub(crate) async fn add_member_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<VaultMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: AdminTargetRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let target_identity = resolve_and_record_identity(&state, &request.target_npub)?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        &actor,
        AdminAccessAction::AddMember,
        None,
        Some(target.as_str()),
        None,
    )?;
    mutate_as_admin(state, vault_id, actor, event, payload, |store, vault_id| {
        store.add_member(vault_id, &target)
    })
    .map(Json)
}

pub(crate) async fn remove_member_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<VaultMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: AdminEventRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub)?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        &actor,
        AdminAccessAction::RemoveMember,
        None,
        Some(target.as_str()),
        None,
    )?;
    mutate_as_admin(state, vault_id, actor, event, payload, |store, vault_id| {
        store.remove_member(vault_id, &target)
    })
    .map(Json)
}

pub(crate) async fn add_admin_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<VaultMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: AdminTargetRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let target_identity = resolve_and_record_identity(&state, &request.target_npub)?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        &actor,
        AdminAccessAction::AddAdmin,
        None,
        Some(target.as_str()),
        None,
    )?;
    mutate_as_admin(state, vault_id, actor, event, payload, |store, vault_id| {
        store.add_admin(vault_id, &target)
    })
    .map(Json)
}

pub(crate) async fn remove_admin_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<VaultMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: AdminEventRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub)?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        &actor,
        AdminAccessAction::RemoveAdmin,
        None,
        Some(target.as_str()),
        None,
    )?;
    mutate_as_admin(state, vault_id, actor, event, payload, |store, vault_id| {
        store.remove_admin(vault_id, &target)
    })
    .map(Json)
}

pub(crate) async fn list_vault_invitations_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
) -> Result<Json<VaultInvitationListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let vault_id = VaultId::new(vault_id)?;
    let invitations = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        ensure_vault_admin(&stored, &actor)?;
        let mut responses = store
            .list_vault_invitations(&vault_id)?
            .into_iter()
            .map(vault_invitation_response)
            .collect::<Vec<_>>();
        for response in &mut responses {
            enrich_vault_invitation_identities(&store, response)?;
        }
        responses
    };
    Ok(Json(VaultInvitationListResponse { invitations }))
}

pub(crate) async fn create_vault_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<VaultInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateVaultInvitationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let target_identity = resolve_and_record_identity(&state, &request.target_npub)?;
    let target = UserId::new(target_identity.npub.clone())?;
    let initial_folder_access = request
        .initial_folder_access
        .into_iter()
        .map(FolderId::new)
        .collect::<Result<Vec<_>, _>>()?;
    let actor_user_id = UserId::new(actor.clone())?;
    let created_at = server_timestamp(&state);
    let id = generated_link_id(
        "invitation",
        &[
            vault_id.as_str(),
            target.as_str(),
            actor_user_id.as_str(),
            request.expires_at.as_str(),
            created_at.as_str(),
        ],
        16,
    );
    let invite_code = generated_link_id(
        "invite",
        &[
            vault_id.as_str(),
            target.as_str(),
            actor_user_id.as_str(),
            request.expires_at.as_str(),
            created_at.as_str(),
            "code",
        ],
        16,
    );
    let accept_path = format!("/_admin/vault-invitation-links/{invite_code}/accept");

    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        ensure_vault_admin(&stored, &actor)?;
        store.create_vault_invitation(
            &vault_id,
            &id,
            &target,
            &invite_code,
            &accept_path,
            &initial_folder_access,
            &actor_user_id,
            &request.expires_at,
            &created_at,
        )?
    };

    let mut response = vault_invitation_response(invitation);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_vault_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn revoke_vault_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, invitation_id)): AxumPath<(String, String)>,
) -> Result<Json<VaultInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let vault_id = VaultId::new(vault_id)?;
    let actor_user_id = UserId::new(actor)?;
    let updated_at = server_timestamp(&state);
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.revoke_vault_invitation(&vault_id, &invitation_id, &actor_user_id, &updated_at)?
    };
    let mut response = vault_invitation_response(invitation);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_vault_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn accept_vault_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, invitation_id)): AxumPath<(String, String)>,
) -> Result<Json<VaultInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let vault_id = VaultId::new(vault_id)?;
    let now = server_timestamp(&state);
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let invitation = store.load_vault_invitation(&invitation_id)?;
        if invitation.vault_id != vault_id {
            return Err(StoreError::UnavailableLink {
                kind: "vault invitation",
            }
            .into());
        }
        store.accept_vault_invitation_by_code(&invitation.invite_code, &actor, &now)?
    };
    let mut response = vault_invitation_response(invitation);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_vault_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn get_vault_invitation_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
) -> Result<Json<VaultInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_available_vault_invitation_by_code(&invite_code, &actor, &now)?
    };
    let mut response = vault_invitation_response(invitation);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_vault_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn accept_vault_invitation_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
) -> Result<Json<VaultInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.accept_vault_invitation_by_code(&invite_code, &actor, &now)?
    };
    let mut response = vault_invitation_response(invitation);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_vault_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}
