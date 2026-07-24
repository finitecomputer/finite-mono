use crate::*;

pub(crate) async fn create_share_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<ShareLinkResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateShareLinkRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let recipient_identity = resolve_and_record_identity(&state, &request.recipient_npub)?;
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
        "share-link",
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
    let accept_path = format!("/_admin/share-links/{id}/accept");

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
            request.create_personal_mount.unwrap_or(false),
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

pub(crate) async fn list_folder_share_links_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
) -> Result<Json<ShareLinkListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let share_links = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let mut responses = store
            .list_folder_share_links(&brain_id, &folder_id)?
            .into_iter()
            .map(share_link_response)
            .collect::<Vec<_>>();
        for response in &mut responses {
            enrich_share_link_identities(&store, response)?;
        }
        responses
    };
    Ok(Json(ShareLinkListResponse { share_links }))
}

pub(crate) async fn get_share_link_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(share_link_id): AxumPath<String>,
) -> Result<Json<ShareLinkResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let share_link = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_available_share_link(&share_link_id, &actor, &now)?
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
) -> Result<Json<ShareLinkResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let share_link = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let share_link = store.accept_share_link(&share_link_id, &actor, &now)?;
        append_folder_key_grant_record(
            &mut store,
            &share_link.brain_id,
            &share_link.folder_key_grant,
        )?;
        share_link
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
) -> Result<Json<ShareLinkResponse>, ApiError> {
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

pub(crate) async fn mark_shared_folder_source_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: MarkSharedFolderSourceRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let current_key_version = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        folder_current_key_version(&stored, &folder_id)?
    };
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::SetFolderAccessMode,
        Some(&folder_id),
        None,
        Some(current_key_version),
    )?;
    mutate_as_admin(state, brain_id, actor, event, payload, |store, brain_id| {
        store.mark_shared_folder_source(brain_id, &folder_id)
    })
    .map(Json)
}

pub(crate) async fn create_shared_folder_invitation_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((source_brain_id, source_folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<SharedFolderInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateSharedFolderInvitationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let source_brain_id = BrainId::new(source_brain_id)?;
    let source_folder_id = FolderId::new(source_folder_id)?;
    let destination_brain_id = BrainId::new(request.destination_brain_id)?;
    let destination_admin_identity =
        resolve_and_record_identity(&state, &request.destination_admin_npub)?;
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
        "shared-folder-invitation",
        &[
            source_brain_id.as_str(),
            source_folder_id.as_str(),
            destination_brain_id.as_str(),
            destination_admin.as_str(),
            created_at.as_str(),
        ],
        16,
    );
    let accept_path = format!("/_admin/shared-folder-invitations/{id}/accept");
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
) -> Result<Json<SharedFolderInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let invitation = {
        let store = state.store.lock().map_err(lock_error)?;
        let invitation = store.load_shared_folder_invitation(&invitation_id)?;
        if invitation.destination_admin_npub.as_str() != actor {
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
) -> Result<Json<SharedFolderInvitationResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let now = server_timestamp(&state);
    let invitation = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let invitation = store.load_shared_folder_invitation(&invitation_id)?;
        let connection_id = shared_folder_connection_id(
            &invitation.source_brain_id,
            &invitation.source_folder_id,
            &invitation.destination_brain_id,
        );
        let mount_id = organization_mount_id(
            &invitation.destination_brain_id,
            &invitation.source_brain_id,
            &invitation.source_folder_id,
        );
        let invitation = store.accept_shared_folder_invitation(
            &invitation_id,
            &actor,
            &connection_id,
            &mount_id,
            &now,
        )?;
        append_folder_key_grant_record(
            &mut store,
            &invitation.source_brain_id,
            &invitation.folder_key_grant,
        )?;
        invitation
    };
    let mut response = shared_folder_invitation_response(invitation);
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
) -> Result<Json<SharedFolderInvitationResponse>, ApiError> {
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

pub(crate) async fn update_shared_folder_connection_members_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(connection_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<SharedFolderConnectionResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let actor = UserId::new(actor)?;
    let request: UpdateSharedFolderConnectionMembersRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let target_identity = resolve_and_record_identity(&state, &request.target_npub)?;
    let target = UserId::new(target_identity.npub.clone())?;
    let now = server_timestamp(&state);
    let connection = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let connection = store.load_shared_folder_connection(&connection_id)?;
        match request.action.as_str() {
            "add" => {
                let mut grant = request.grant.clone().ok_or_else(|| {
                    ApiError::new(StatusCode::BAD_REQUEST, "grant is required for add")
                })?;
                grant.recipient_npub = target.as_str().to_owned();
                let grant = grant_request_to_metadata(
                    &grant,
                    &connection.source_folder_id,
                    actor.as_str(),
                    None,
                    &now,
                )?;
                let connection = store.add_shared_folder_connection_member(
                    &connection_id,
                    &actor,
                    &target,
                    &grant,
                    &now,
                )?;
                append_folder_key_grant_record(&mut store, &connection.source_brain_id, &grant)?;
                connection
            }
            "remove" => {
                let new_key_version = request.new_key_version.ok_or_else(|| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "newKeyVersion is required for remove",
                    )
                })?;
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
                    new_key_version,
                    request.reencrypted_records,
                )?;
                let connection = store.remove_shared_folder_connection_member(
                    &connection_id,
                    &actor,
                    &target,
                    new_key_version,
                    &grants,
                    &reencrypted_records,
                    &now,
                )?;
                for grant in &grants {
                    append_folder_key_grant_record(&mut store, &connection.source_brain_id, grant)?;
                }
                connection
            }
            _ => {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "action must be add or remove",
                ));
            }
        }
    };
    let mut response = shared_folder_connection_response(connection);
    {
        let store = state.store.lock().map_err(lock_error)?;
        enrich_shared_folder_connection_identities(&store, &mut response)?;
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
) -> Result<Json<SharedFolderConnectionResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let actor = UserId::new(actor)?;
    let request: RevokeSharedFolderConnectionRequest = serde_json::from_slice(&body)
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
        let connection = store.revoke_shared_folder_connection(
            &connection_id,
            &actor,
            request.new_key_version,
            &grants,
            &reencrypted_records,
            &now,
        )?;
        for grant in &grants {
            append_folder_key_grant_record(&mut store, &connection.source_brain_id, grant)?;
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

pub(crate) async fn list_shared_folder_invitations_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<SharedFolderInvitationListResponse>, ApiError> {
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
    Ok(Json(SharedFolderInvitationListResponse {
        outgoing,
        incoming,
    }))
}

pub(crate) async fn list_shared_folder_connections_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<SharedFolderConnectionListResponse>, ApiError> {
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
    Ok(Json(SharedFolderConnectionListResponse {
        outgoing,
        incoming,
    }))
}

pub(crate) async fn organization_folder_mounts_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<Vec<MountedFolderResponse>>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor = UserId::new(actor)?;
    let brain_id = BrainId::new(brain_id)?;
    let projections = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_metadata_visible(&stored, actor.as_str())?;
        store.mounted_folder_projection(&brain_id, &actor)?
    };
    Ok(Json(mounted_folder_responses(projections)))
}
