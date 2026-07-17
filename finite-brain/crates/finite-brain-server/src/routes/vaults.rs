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

    let organization_requester = match request.kind {
        CreateVaultKind::Personal if request.requesting_user_npub.is_some() => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Organization Vault requester identity is only valid for an Organization Vault",
            ));
        }
        CreateVaultKind::Personal => None,
        CreateVaultKind::Organization => request
            .requesting_user_npub
            .as_deref()
            .map(canonical_requesting_user_npub)
            .transpose()?
            .map(UserId::new)
            .transpose()?,
    };

    let personal_agent = match request.kind {
        CreateVaultKind::Organization
            if request.personal_agent_email.is_some() || request.personal_agent_npub.is_some() =>
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Personal Agent identity is only valid for a Personal Vault",
            ));
        }
        CreateVaultKind::Organization => None,
        CreateVaultKind::Personal => {
            let email_identity = request
                .personal_agent_email
                .as_deref()
                .map(|email| resolve_managed_agent_email(&state, email))
                .transpose()?;
            let npub_identity = request
                .personal_agent_npub
                .as_deref()
                .map(|npub| resolve_identity_input(&state, npub))
                .transpose()?;
            if let (Some(email), Some(npub)) = (&email_identity, &npub_identity)
                && email.npub != npub.npub
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "personalAgentEmail and personalAgentNpub resolve to different Agent Principals",
                ));
            }
            let identity = email_identity.or(npub_identity);
            if let Some(identity) = identity.as_ref() {
                record_resolved_identity(&state, identity.clone())?;
            }
            identity
                .map(|identity| UserId::new(identity.npub))
                .transpose()?
        }
    };
    let requester_bootstrap = organization_requester.is_some();
    let output = match request.kind {
        CreateVaultKind::Personal => {
            bootstrap_personal_vault(request.vault_id, request.name, actor_npub.clone())?
        }
        CreateVaultKind::Organization => {
            if let Some(requester) = organization_requester {
                bootstrap_organization_vault_with_requester(
                    request.vault_id,
                    request.name,
                    actor_npub.clone(),
                    requester.as_str().to_owned(),
                )?
            } else {
                bootstrap_organization_vault(request.vault_id, request.name, actor_npub.clone())?
            }
        }
    };
    let vault_id = output.vault.id.clone();
    let grants = if request.bootstrap_grants.is_empty() {
        if requester_bootstrap {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Organization Vault requester bootstrap requires encrypted Folder Key Grants",
            ));
        }
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
        if let Some(agent_npub) = personal_agent.as_ref() {
            store.create_personal_vault_bootstrap(
                &output,
                &grants,
                agent_npub,
                &UserId::new(actor_npub.clone())?,
                &server_timestamp(&state),
            )?;
        } else {
            store.create_vault_bootstrap(&output, &grants)?;
        }
        store.load_vault(&vault_id)?
    };

    let mut response = metadata_response(stored);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_metadata_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

fn canonical_requesting_user_npub(value: &str) -> Result<String, ApiError> {
    let public_key = NostrPublicKey::parse(value).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid Organization Vault requester identity: {error}"),
        )
    })?;
    public_key.to_npub().map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid Organization Vault requester identity: {error}"),
        )
    })
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

    let mut response = metadata_response_for_actor(stored, mounted_folders, &actor_npub);
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
            attach_invitation_public_url(&state, response);
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
    let actor_user_id = UserId::new(actor.clone())?;
    let created_at = server_timestamp(&state);
    let target_input = invitation_target_input(&request)?;

    let npub_target = if let Ok(public_key) = NostrPublicKey::parse(&target_input) {
        Some(public_key.to_npub().map_err(nostr_identity_error)?)
    } else if finite_vip_email(&target_input) {
        resolve_and_record_identity(&state, &target_input)
            .ok()
            .map(|identity| identity.npub)
    } else {
        None
    };

    let invitation = if let Some(target_npub) = npub_target {
        let target = UserId::new(target_npub)?;
        let initial_folder_access = selected_folder_ids(&request.initial_folder_access)?;
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
    } else {
        if !email_like(&target_input) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invitation target must be npub, hex, active finite.vip NIP-05, or email",
            ));
        }
        let invited_email = canonical_email(&target_input)?;
        let invite_unwrap_npub = UserId::new(canonical_npub_from_public_key_input(
            request.invite_unwrap_npub.as_deref().ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "inviteUnwrapNpub is required for email bootstrap invitations",
                )
            })?,
        )?)?;
        let bootstrap_payload_hash = request
            .bootstrap_payload_hash
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "bootstrapPayloadHash is required for email bootstrap invitations",
                )
            })?;
        let bootstrap_wrapped_event_json = request
            .bootstrap_wrapped_event_json
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "bootstrapWrappedEventJson is required for email bootstrap invitations",
                )
            })?;
        let bootstrap_authorization_event_json = request
            .bootstrap_authorization_event_json
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "bootstrapAuthorizationEventJson is required for email bootstrap invitations",
                )
            })?;
        validate_folder_key_grant_wrapper(bootstrap_wrapped_event_json, &invite_unwrap_npub)?;
        let selected_restricted_folder_access =
            selected_folder_ids(&request.initial_folder_access)?;
        let id = generated_link_id(
            "invitation",
            &[
                vault_id.as_str(),
                invited_email.as_str(),
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
                invited_email.as_str(),
                actor_user_id.as_str(),
                request.expires_at.as_str(),
                created_at.as_str(),
                "code",
            ],
            16,
        );
        let accept_path = format!("/_admin/vault-invitation-links/{invite_code}/claim");
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        ensure_vault_admin(&stored, &actor)?;
        let scope = email_bootstrap_scope_for_vault(&stored, &selected_restricted_folder_access)?;
        validate_email_bootstrap_authorization(
            bootstrap_authorization_event_json,
            &actor,
            &vault_id,
            &invited_email,
            &invite_unwrap_npub,
            bootstrap_payload_hash,
            &request.expires_at,
            &scope,
        )?;
        store.create_email_vault_invitation(
            &vault_id,
            &id,
            &invited_email,
            &invite_unwrap_npub,
            bootstrap_payload_hash,
            bootstrap_wrapped_event_json,
            bootstrap_authorization_event_json,
            &invite_code,
            &accept_path,
            &selected_restricted_folder_access,
            &actor_user_id,
            &request.expires_at,
            &created_at,
        )?
    };

    let delivery_status = deliver_email_invitation(&state, &invitation)?;
    let mut response = vault_invitation_response(invitation);
    response.delivery_status = delivery_status;
    attach_invitation_public_url(&state, &mut response);
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
    attach_invitation_public_url(&state, &mut response);
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
    attach_invitation_public_url(&state, &mut response);
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
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_vault_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn public_vault_invitation_instructions_handler(
    State(state): State<ServerState>,
    AxumPath(invite_code): AxumPath<String>,
) -> Result<Response, ApiError> {
    {
        let store = state.store.lock().map_err(lock_error)?;
        let invitation = store.load_vault_invitation_by_code(&invite_code)?;
        if invitation.target_kind != VaultInvitationTargetKind::EmailBootstrap {
            return Err(StoreError::UnavailableLink {
                kind: "vault invitation",
            }
            .into());
        }
    }
    Ok(text_response(public_invite_instructions_text()))
}

pub(crate) async fn post_proof_vault_invitation_instructions_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request: PostProofInviteInstructionsRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let invitation = load_post_proof_email_invitation(
        &state,
        &headers,
        &method,
        &uri,
        &invite_code,
        &body,
        &request,
    )?;
    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_vault(&invitation.vault_id)?
    };
    Ok(text_response(post_proof_invite_instructions_text(
        &state,
        &invitation,
        &stored,
    )))
}

pub(crate) async fn post_proof_vault_invitation_bootstrap_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
    body: Bytes,
) -> Result<Json<VaultInvitationResponse>, ApiError> {
    let request: PostProofInviteInstructionsRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let invitation = load_post_proof_email_invitation(
        &state,
        &headers,
        &method,
        &uri,
        &invite_code,
        &body,
        &request,
    )?;
    if invitation.status == LinkStatus::Pending && invitation.bootstrap_wrapped_event_json.is_none()
    {
        return Err(StoreError::UnavailableLink {
            kind: "vault invitation",
        }
        .into());
    }
    let mut response = vault_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_vault_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

fn load_post_proof_email_invitation(
    state: &ServerState,
    headers: &HeaderMap,
    method: &Method,
    uri: &axum::http::Uri,
    invite_code: &str,
    body: &Bytes,
    request: &PostProofInviteInstructionsRequest,
) -> Result<StoredVaultInvitation, ApiError> {
    let actor = validate_request_auth(state, headers, method, uri, Some(body))?;
    let actor_user_id = UserId::new(actor)?;
    let invited_email = canonical_email(&request.email)?;
    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_vault_invitation_by_code(invite_code)?
    };
    if invitation.target_kind != VaultInvitationTargetKind::EmailBootstrap {
        return Err(StoreError::UnavailableLink {
            kind: "vault invitation",
        }
        .into());
    }
    if invitation.invited_email.as_deref() != Some(invited_email.as_str()) {
        return Err(StoreError::UnavailableLink {
            kind: "vault invitation",
        }
        .into());
    }
    if invitation.status == LinkStatus::Accepted
        && invitation.claimed_by_npub.as_ref() != Some(&actor_user_id)
    {
        return Err(StoreError::UnavailableLink {
            kind: "vault invitation",
        }
        .into());
    }
    validate_email_proof_window(
        &invitation,
        &request.email_proof_created_at,
        &server_timestamp(state),
    )?;
    verify_identity_authority_email_proof(state, invited_email.as_str(), &actor_user_id)?;
    Ok(invitation)
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
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_vault_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn claim_email_vault_invitation_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invite_code): AxumPath<String>,
    body: Bytes,
) -> Result<Json<VaultInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let actor_user_id = UserId::new(actor.clone())?;
    let request: ClaimEmailVaultInvitationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let now = server_timestamp(&state);
    let invited_email = canonical_email(&request.email)?;

    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_vault_invitation_by_code(&invite_code)?
    };
    if invitation.target_kind != VaultInvitationTargetKind::EmailBootstrap {
        return Err(StoreError::UnavailableLink {
            kind: "vault invitation",
        }
        .into());
    }
    if invitation.invited_email.as_deref() != Some(invited_email.as_str()) {
        return Err(StoreError::UnavailableLink {
            kind: "vault invitation",
        }
        .into());
    }

    let invitation = if invitation.status == LinkStatus::Accepted {
        if invitation.claimed_by_npub.as_ref() == Some(&actor_user_id) {
            let mut invitation = invitation;
            invitation.duplicate_accept = true;
            invitation
        } else {
            return Err(StoreError::UnavailableLink {
                kind: "vault invitation",
            }
            .into());
        }
    } else {
        validate_email_proof_window(&invitation, &request.email_proof_created_at, &now)?;
        verify_identity_authority_email_proof(&state, invited_email.as_str(), &actor_user_id)?;
        if let (Some(authorization), Some(invite_unwrap_npub), Some(payload_hash)) = (
            invitation.bootstrap_authorization_event_json.as_deref(),
            invitation.invite_unwrap_npub.as_ref(),
            invitation.bootstrap_payload_hash.as_deref(),
        ) {
            validate_email_bootstrap_authorization(
                authorization,
                invitation.created_by_npub.as_str(),
                &invitation.vault_id,
                invited_email.as_str(),
                invite_unwrap_npub,
                payload_hash,
                &invitation.expires_at,
                &invitation.bootstrap_scope,
            )?;
            let proof_event_json = request
                .invite_unwrap_proof_event_json
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "inviteUnwrapProofEventJson is required for email bootstrap claims",
                    )
                })?;
            validate_email_bootstrap_claim_proof(
                proof_event_json,
                invite_unwrap_npub,
                &invitation.vault_id,
                &invite_code,
                invited_email.as_str(),
                &actor_user_id,
                payload_hash,
                &request.email_proof_created_at,
            )?;
        } else {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "email bootstrap invitation is missing authorization metadata",
            ));
        }
        let grants = bootstrap_grant_requests_to_metadata(&request.grants, &actor, &now)?;
        let mut store = state.store.lock().map_err(lock_error)?;
        let invitation = store.claim_email_vault_invitation_by_code(
            &invite_code,
            invited_email.as_str(),
            &actor_user_id,
            &grants,
            &now,
        )?;
        if !invitation.duplicate_accept {
            for grant in &grants {
                append_folder_key_grant_record(&mut store, &invitation.vault_id, grant)?;
            }
        }
        invitation
    };

    let mut response = vault_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_vault_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}
