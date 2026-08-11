use crate::*;

fn brain_invitation_id(id: &str) -> bool {
    id.starts_with("invitation-")
}

fn invitation_json<T: serde::Serialize>(value: T) -> Result<Json<serde_json::Value>, ApiError> {
    serde_json::to_value(value).map(Json).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invitation response serialization failed",
        )
    })
}

pub(crate) async fn list_account_invitations_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<AccountInvitationsQuery>,
) -> Result<Json<AccountInvitationInboxResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state, &headers, &method, &uri, None,
    )?)?;
    let invitations = {
        let store = state.store.lock().map_err(lock_error)?;
        let mut items = Vec::new();
        for (invitation, hidden) in store.list_account_invitations(&actor, query.include_hidden)? {
            let mut response = brain_invitation_response(invitation);
            attach_invitation_public_url(&state, &mut response);
            enrich_brain_invitation_identities(&store, &mut response)?;
            items.push(AccountInvitationInboxItemResponse {
                invitation: response,
                hidden,
            });
        }
        items
    };
    Ok(Json(AccountInvitationInboxResponse { invitations }))
}

pub(crate) async fn set_account_invitation_visibility_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invitation_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<AccountInvitationInboxItemResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: SetAccountInvitationVisibilityRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let now = server_timestamp(&state);
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.set_account_invitation_hidden(&invitation_id, &actor, request.hidden, &now)?;
        store.load_brain_invitation(&invitation_id)?
    };
    let mut response = brain_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(AccountInvitationInboxItemResponse {
        invitation: response,
        hidden: request.hidden,
    }))
}

pub(crate) async fn get_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invitation_id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !brain_invitation_id(&invitation_id) {
        return get_share_link_handler(
            State(state),
            headers,
            method,
            OriginalUri(uri),
            AxumPath(invitation_id),
        )
        .await
        .and_then(|Json(value)| invitation_json(value));
    }
    let actor = UserId::new(validate_request_auth(
        &state, &headers, &method, &uri, None,
    )?)?;
    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        let invitation = store.load_brain_invitation(&invitation_id)?;
        let stored = store.load_brain(&invitation.brain_id)?;
        let is_target = invitation.user_id.as_ref() == Some(&actor)
            || store
                .load_cohort_invitation_plan(&invitation_id)?
                .is_some_and(|plan| {
                    plan.participants
                        .iter()
                        .any(|participant| participant.npub == actor)
                });
        if !is_target {
            ensure_brain_admin(&stored, actor.as_str())?;
        }
        invitation
    };
    let mut response = brain_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
    }
    invitation_json(response)
}

pub(crate) async fn accept_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invitation_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !brain_invitation_id(&invitation_id) {
        return accept_share_link_handler(
            State(state),
            headers,
            method,
            OriginalUri(uri),
            AxumPath(invitation_id),
        )
        .await
        .and_then(|Json(value)| invitation_json(value));
    }
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        (!body.is_empty()).then_some(&body),
    )?)?;
    let narrowing: AcceptAccountCohortInvitationRequest = if body.is_empty() {
        AcceptAccountCohortInvitationRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?
    };
    let now = server_timestamp(&state);
    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain_invitation(&invitation_id)?
    };
    let departures = if invitation.target_kind == BrainInvitationTargetKind::AccountCohort {
        permanent_departures_for_cohort_invitation(&state, &invitation_id).await?
    } else {
        BTreeMap::new()
    };
    validate_acceptance_narrowing(&narrowing.removed_participants, &departures)?;
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        if invitation.target_kind == BrainInvitationTargetKind::AccountCohort {
            store.accept_account_cohort_invitation_by_code(
                &invitation.invite_code,
                &actor,
                &departures,
                &now,
            )?
        } else {
            store.accept_brain_invitation_by_code(&invitation.invite_code, &actor, &now)?
        }
    };
    let mut response = brain_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
    }
    invitation_json(response)
}

pub(crate) async fn revoke_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invitation_id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !brain_invitation_id(&invitation_id) {
        return revoke_share_link_handler(
            State(state),
            headers,
            method,
            OriginalUri(uri),
            AxumPath(invitation_id),
        )
        .await
        .and_then(|Json(value)| invitation_json(value));
    }
    let actor = UserId::new(validate_request_auth(
        &state, &headers, &method, &uri, None,
    )?)?;
    let now = server_timestamp(&state);
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let invitation = store.load_brain_invitation(&invitation_id)?;
        store.revoke_brain_invitation(&invitation.brain_id, &invitation_id, &actor, &now)?
    };
    let mut response = brain_invitation_response(invitation);
    attach_invitation_public_url(&state, &mut response);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_brain_invitation_identities(&store, &mut response)?;
    }
    invitation_json(response)
}

pub(crate) async fn create_share_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<FolderInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateFolderInvitationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let recipient_identity = resolve_and_record_identity(&state, &request.recipient_npub).await?;
    let recipient = UserId::new(recipient_identity.npub.clone())?;
    let current_key_version = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        folder_current_key_version(&stored, &folder_id)?
    };
    let (event, _) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::GrantFolderAccess,
        Some(&folder_id),
        Some(recipient.as_str()),
        Some(current_key_version),
    )?;
    let created_at = server_timestamp(&state);
    let mut grant_request = request.grant;
    grant_request.recipient_npub = recipient.as_str().to_owned();
    let grant = grant_request_to_metadata(
        &grant_request,
        &folder_id,
        &actor,
        Some(event.as_json()),
        &created_at,
    )?;
    let actor_user_id = UserId::new(actor.clone())?;
    let id = generated_link_id(
        "folder-invitation",
        &[
            brain_id.as_str(),
            folder_id.as_str(),
            recipient.as_str(),
            actor_user_id.as_str(),
            request.expires_at.as_str(),
            created_at.as_str(),
        ],
        16,
    );
    let accept_path = format!("/v1/invitations/{id}/accept");

    let share_link = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.create_share_link(
            &brain_id,
            &folder_id,
            &id,
            &recipient,
            &actor_user_id,
            &request.expires_at,
            &accept_path,
            &grant,
            &created_at,
        )?
    };
    let mut response = share_link_response(share_link);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_share_link_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn create_folder_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Ok(request) = serde_json::from_slice::<CreateBrainInvitationRequest>(&body)
        && request.folder_only
    {
        let selected = selected_folder_ids(&request.initial_folder_access)?;
        let expected_folder_id = FolderId::new(folder_id.clone())?;
        if selected.as_slice() != [expected_folder_id] {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Folder Email Invite Bootstrap path and initialFolderAccess must identify the same single Folder",
            ));
        }
        return create_brain_invitation_handler(
            State(state),
            headers,
            method,
            OriginalUri(uri),
            AxumPath(brain_id),
            body,
        )
        .await
        .and_then(|Json(value)| invitation_json(value));
    }

    create_share_link_handler(
        State(state),
        headers,
        method,
        OriginalUri(uri),
        AxumPath((brain_id, folder_id)),
        body,
    )
    .await
    .and_then(|Json(value)| invitation_json(value))
}

pub(crate) async fn list_folder_share_links_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
) -> Result<Json<FolderInvitationListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let invitations = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let mut npub_responses = store
            .list_folder_share_links(&brain_id, &folder_id)?
            .into_iter()
            .map(share_link_response)
            .collect::<Vec<_>>();
        for response in &mut npub_responses {
            enrich_share_link_identities(&store, response)?;
        }
        let mut email_responses = store
            .list_brain_invitations(&brain_id)?
            .into_iter()
            .filter(|invitation| {
                invitation.folder_only
                    && invitation.initial_folder_access.as_slice() == [folder_id.clone()]
            })
            .map(brain_invitation_response)
            .collect::<Vec<_>>();
        for response in &mut email_responses {
            enrich_brain_invitation_identities(&store, response)?;
            attach_invitation_public_url(&state, response);
        }
        npub_responses
            .into_iter()
            .map(|response| FolderInvitationResourceResponse::Npub(Box::new(response)))
            .chain(
                email_responses
                    .into_iter()
                    .map(|response| FolderInvitationResourceResponse::Email(Box::new(response))),
            )
            .collect()
    };
    Ok(Json(FolderInvitationListResponse { invitations }))
}

pub(crate) async fn get_share_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(share_link_id): AxumPath<String>,
) -> Result<Json<FolderInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let share_link = {
        let store = state.store.lock().map_err(lock_error)?;
        let share_link = store.load_share_link(&share_link_id)?;
        if share_link.recipient_npub == actor {
            store.load_available_share_link(&share_link_id, &actor, &now)?
        } else {
            let stored = store.load_brain(&share_link.brain_id)?;
            if ensure_brain_admin(&stored, actor.as_str()).is_err() {
                return Err(StoreError::UnavailableLink {
                    kind: "Folder Invitation",
                }
                .into());
            }
            share_link
        }
    };
    let mut response = share_link_response(share_link);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_share_link_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn accept_share_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(share_link_id): AxumPath<String>,
) -> Result<Json<FolderInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let share_link = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let current = store.load_share_link(&share_link_id)?;
        let control_record = folder_key_grant_sync_record(&current.folder_key_grant)?;
        store.accept_share_link(
            &share_link_id,
            &actor,
            std::slice::from_ref(&control_record),
            &now,
        )?
    };
    let mut response = share_link_response(share_link);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_share_link_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn revoke_share_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(share_link_id): AxumPath<String>,
) -> Result<Json<FolderInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let share_link = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.revoke_share_link(&share_link_id, &actor, &now)?
    };
    let mut response = share_link_response(share_link);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_share_link_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn create_shared_folder_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((source_brain_id, source_folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<MountOfferResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateMountOfferRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let source_brain_id = BrainId::new(source_brain_id)?;
    let source_folder_id = FolderId::new(source_folder_id)?;
    let destination_brain_id = BrainId::new(request.destination_brain_id)?;
    let destination_admin_identity =
        resolve_and_record_identity(&state, &request.destination_controller_npub).await?;
    let destination_admin = UserId::new(destination_admin_identity.npub.clone())?;
    let current_key_version = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&source_brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        folder_current_key_version(&stored, &source_folder_id)?
    };
    let (event, _) = validate_admin_access_change_value(
        request.access_change_event,
        &source_brain_id,
        &actor,
        AdminAccessAction::GrantFolderAccess,
        Some(&source_folder_id),
        Some(destination_admin.as_str()),
        Some(current_key_version),
    )?;
    let created_at = server_timestamp(&state);
    let mut grant_request = request.grant;
    grant_request.recipient_npub = destination_admin.as_str().to_owned();
    let grant = grant_request_to_metadata(
        &grant_request,
        &source_folder_id,
        &actor,
        Some(event.as_json()),
        &created_at,
    )?;
    let actor_user_id = UserId::new(actor)?;
    let id = generated_link_id(
        "mount-offer",
        &[
            source_brain_id.as_str(),
            source_folder_id.as_str(),
            destination_brain_id.as_str(),
            destination_admin.as_str(),
            created_at.as_str(),
        ],
        16,
    );
    let accept_path = format!("/v1/mount-offers/{id}/accept");
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.create_shared_folder_invitation(
            &source_brain_id,
            &source_folder_id,
            &destination_brain_id,
            &id,
            &destination_admin,
            &actor_user_id,
            &accept_path,
            &grant,
            &request.expires_at,
            &created_at,
        )?
    };
    let mut response = shared_folder_invitation_response(invitation);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_shared_folder_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn get_shared_folder_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invitation_id): AxumPath<String>,
) -> Result<Json<MountOfferResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        let invitation = store.load_shared_folder_invitation(&invitation_id)?;
        let source = store.load_brain(&invitation.source_brain_id)?;
        if invitation.destination_admin_npub.as_str() != actor
            && ensure_brain_admin(&source, &actor).is_err()
        {
            return Err(StoreError::UnavailableLink {
                kind: "shared folder invitation",
            }
            .into());
        }
        invitation
    };
    let mut response = shared_folder_invitation_response(invitation);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_shared_folder_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn accept_shared_folder_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invitation_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<MountOfferResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let request = if body.is_empty() {
        AcceptMountOfferRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?
    };
    let (invitation, connection_id, initial_participants) = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let invitation = store.load_shared_folder_invitation(&invitation_id)?;
        let destination = store.load_brain(&invitation.destination_brain_id)?;
        let mut initial_participants = BTreeSet::from([actor.clone()]);
        if destination.brain.kind == BrainKind::Personal {
            if let Some(owner) = destination.brain.owner_user_id {
                initial_participants.insert(owner);
            }
            if let Some(agent) = destination.personal_agent {
                initial_participants.insert(agent.agent_npub);
            }
        }
        let supplemental_grants = grant_requests_to_metadata(
            &request.grants,
            &invitation.source_folder_id,
            actor.as_str(),
            None,
            &now,
        )?;
        let connection_id = shared_folder_connection_id(
            &invitation.source_brain_id,
            &invitation.source_folder_id,
            &invitation.destination_brain_id,
        );
        let mount_id = folder_mount_id(
            &invitation.destination_brain_id,
            &invitation.source_brain_id,
            &invitation.source_folder_id,
        );
        let control_records = std::iter::once(&invitation.folder_key_grant)
            .chain(&supplemental_grants)
            .map(folder_key_grant_sync_record)
            .collect::<Result<Vec<_>, _>>()?;
        let invitation = store.accept_shared_folder_invitation(
            &invitation_id,
            &actor,
            &connection_id,
            &mount_id,
            &supplemental_grants,
            &control_records,
            &now,
        )?;
        (
            invitation,
            connection_id,
            initial_participants
                .into_iter()
                .map(|participant| participant.to_string())
                .collect::<Vec<_>>(),
        )
    };
    let mut response = shared_folder_invitation_response(invitation);
    response.mount_id = Some(connection_id);
    response.initial_participant_npubs = initial_participants;
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_shared_folder_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn revoke_shared_folder_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(invitation_id): AxumPath<String>,
) -> Result<Json<MountOfferResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.revoke_shared_folder_invitation(&invitation_id, &actor, &now)?
    };
    let mut response = shared_folder_invitation_response(invitation);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_shared_folder_invitation_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn revoke_shared_folder_connection_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(connection_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<MountResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let actor = UserId::new(actor)?;
    let request: RevokeMountRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let now = server_timestamp(&state);
    let connection = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let connection = store.load_shared_folder_connection(&connection_id)?;
        let grants = grant_requests_to_metadata(
            &request.grants,
            &connection.source_folder_id,
            actor.as_str(),
            None,
            &now,
        )?;
        let reencrypted_records = rotation_records_from_requests(
            &connection.source_brain_id,
            &connection.source_folder_id,
            actor.as_str(),
            request.new_key_version,
            request.reencrypted_records,
        )?;
        let control_records = grants
            .iter()
            .map(folder_key_grant_sync_record)
            .collect::<Result<Vec<_>, _>>()?;
        store.revoke_shared_folder_connection(
            &connection_id,
            &actor,
            request.new_key_version,
            &grants,
            &control_records,
            &reencrypted_records,
            &now,
        )?
    };
    let mut response = shared_folder_connection_response(connection);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_shared_folder_connection_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn get_shared_folder_connection_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(connection_id): AxumPath<String>,
) -> Result<Json<MountResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let connection = {
        let store = state.store.lock().map_err(lock_error)?;
        let connection = store.load_shared_folder_connection(&connection_id)?;
        let source = store.load_brain(&connection.source_brain_id)?;
        let destination = store.load_brain(&connection.destination_brain_id)?;
        if ensure_brain_admin(&source, &actor).is_err()
            && ensure_brain_admin(&destination, &actor).is_err()
        {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "mount controller access required",
            ));
        }
        connection
    };
    let mut response = shared_folder_connection_response(connection);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_shared_folder_connection_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn add_mount_participant_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((connection_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<MountResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: AddMountParticipantRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub)?;
    let now = server_timestamp(&state);
    let connection = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let current = store.load_shared_folder_connection(&connection_id)?;
        let mut request_grant = request.grant;
        request_grant.recipient_npub = target.as_str().to_owned();
        let grant = grant_request_to_metadata(
            &request_grant,
            &current.source_folder_id,
            actor.as_str(),
            None,
            &now,
        )?;
        let control_record = folder_key_grant_sync_record(&grant)?;
        store.add_shared_folder_connection_member(
            &connection_id,
            &actor,
            &target,
            &grant,
            std::slice::from_ref(&control_record),
            &now,
        )?
    };
    let mut response = shared_folder_connection_response(connection);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_shared_folder_connection_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn remove_mount_participant_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((connection_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<MountResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: RemoveMountParticipantRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub)?;
    let now = server_timestamp(&state);
    let connection = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let current = store.load_shared_folder_connection(&connection_id)?;
        let grants = grant_requests_to_metadata(
            &request.grants,
            &current.source_folder_id,
            actor.as_str(),
            None,
            &now,
        )?;
        let records = rotation_records_from_requests(
            &current.source_brain_id,
            &current.source_folder_id,
            actor.as_str(),
            request.new_key_version,
            request.reencrypted_records,
        )?;
        let control_records = grants
            .iter()
            .map(folder_key_grant_sync_record)
            .collect::<Result<Vec<_>, _>>()?;
        store.remove_shared_folder_connection_member(
            &connection_id,
            &actor,
            &target,
            request.new_key_version,
            &grants,
            &control_records,
            &records,
            &now,
        )?
    };
    let mut response = shared_folder_connection_response(connection);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_shared_folder_connection_identities(&store, &mut response)?;
    }
    Ok(Json(response))
}

pub(crate) async fn list_mount_offers_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
) -> Result<Json<Vec<MountOfferResponse>>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let offers = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let mut offers = store
            .list_shared_folder_invitations(&brain_id, SharedFolderDirection::Source)?
            .into_iter()
            .filter(|offer| offer.source_folder_id == folder_id)
            .map(shared_folder_invitation_response)
            .collect::<Vec<_>>();
        for response in &mut offers {
            enrich_shared_folder_invitation_identities(&store, response)?;
        }
        offers
    };
    Ok(Json(offers))
}

pub(crate) async fn list_brain_mount_offers_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<MountOfferListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let (outgoing, incoming) = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let mut outgoing = store
            .list_shared_folder_invitations(&brain_id, SharedFolderDirection::Source)?
            .into_iter()
            .map(shared_folder_invitation_response)
            .collect::<Vec<_>>();
        let mut incoming = store
            .list_shared_folder_invitations(&brain_id, SharedFolderDirection::Destination)?
            .into_iter()
            .map(shared_folder_invitation_response)
            .collect::<Vec<_>>();
        for response in outgoing.iter_mut().chain(incoming.iter_mut()) {
            enrich_shared_folder_invitation_identities(&store, response)?;
        }
        (outgoing, incoming)
    };
    Ok(Json(MountOfferListResponse { outgoing, incoming }))
}

pub(crate) async fn list_shared_folder_connections_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<MountListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let (outgoing, incoming) = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let mut outgoing = store
            .list_shared_folder_connections(&brain_id, SharedFolderDirection::Source)?
            .into_iter()
            .map(shared_folder_connection_response)
            .collect::<Vec<_>>();
        let mut incoming = store
            .list_shared_folder_connections(&brain_id, SharedFolderDirection::Destination)?
            .into_iter()
            .map(shared_folder_connection_response)
            .collect::<Vec<_>>();
        for response in outgoing.iter_mut().chain(incoming.iter_mut()) {
            enrich_shared_folder_connection_identities(&store, response)?;
        }
        (outgoing, incoming)
    };
    Ok(Json(MountListResponse { outgoing, incoming }))
}
