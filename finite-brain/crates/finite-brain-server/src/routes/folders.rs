use crate::*;

pub(crate) async fn delete_folder_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<FolderDeleteResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: FolderDeleteRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let submitted_event = Event::from_json(request.deletion_event.to_string()).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "deletionEvent must be a valid signed Nostr event",
        )
    })?;
    let submitted_event_id = submitted_event.id.to_hex();
    let current_key_version = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        ensure_direct_delete_authority(&stored, &actor)?;
        if let Some(folder) = stored
            .vault
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
        {
            folder.current_key_version
        } else if let Some(replay) = store.folder_deletion_replay(&vault_id, &folder_id)? {
            if replay.deletion_event_id != submitted_event_id || replay.actor_npub.as_str() != actor
            {
                return Err(ApiError::from(StoreError::BrokenInvariant {
                    reason: "Folder identity was already permanently deleted".to_owned(),
                }));
            }
            replay.root_key_version
        } else {
            return Err(ApiError::from(StoreError::MissingFolder {
                folder_id: folder_id.to_string(),
            }));
        }
    };
    let (event, payload) = validate_admin_access_change_value(
        request.deletion_event,
        &vault_id,
        &actor,
        AdminAccessAction::DeleteFolder,
        Some(&folder_id),
        None,
        Some(current_key_version),
    )?;
    let event_id = event.id.to_hex();
    let deleted_at = payload.created_at.clone();
    let payload_json = serde_json::json!({
        "recordType": "folder_subtree_tombstone",
        "folderId": folder_id,
        "deletionEvent": event,
    })
    .to_string();
    let actor = UserId::new(actor)?;
    let outcome = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.delete_folder_subtree(
            &vault_id,
            &folder_id,
            &actor,
            current_key_version,
            &event_id,
            &payload_json,
            &deleted_at,
            APP_SPECIFIC_KIND,
        )?
    };
    Ok(Json(FolderDeleteResponse {
        sequence: outcome.sequence,
        duplicate: outcome.duplicate,
        folder_count: outcome.folder_count,
        object_count: outcome.object_count,
    }))
}

pub(crate) async fn create_folder_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<VaultMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateFolderRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let folder = Folder {
        id: FolderId::new(request.folder_id)?,
        name: DisplayName::new("folder_name", request.name)?,
        role: request.role,
        access: request.access,
        parent_folder_id: request.parent_folder_id.map(FolderId::new).transpose()?,
        path: SafeRelativePath::new("folder_path", request.path)?,
        current_key_version: 1,
        shared_folder_source: request.shared_folder_source.unwrap_or(false),
    };
    let access_user_ids = resolve_user_id_set(&state, request.access_user_ids)?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        &actor,
        AdminAccessAction::SetFolderAccessMode,
        Some(&folder.id),
        None,
        Some(1),
    )?;
    let event_json = event.as_json();
    let grant_created_at = server_timestamp(&state);
    let grants = grant_requests_to_metadata(
        &request.grants,
        &folder.id,
        &actor,
        Some(event_json),
        &grant_created_at,
    )?;

    mutate_as_admin_with_grants(
        state,
        vault_id,
        actor,
        event,
        payload,
        grants.clone(),
        |store, vault_id| store.create_folder(vault_id, &folder, &access_user_ids, &grants),
    )
    .map(Json)
}

pub(crate) async fn finish_folder_setup_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<VaultMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: FinishFolderSetupRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let current_key_version = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        ensure_vault_admin(&stored, &actor)?;
        folder_current_key_version(&stored, &folder_id)?
    };
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        &actor,
        AdminAccessAction::SetFolderAccessMode,
        Some(&folder_id),
        None,
        Some(current_key_version),
    )?;
    let event_json = event.as_json();
    let grant_created_at = server_timestamp(&state);
    let grants = grant_requests_to_metadata(
        &request.grants,
        &folder_id,
        &actor,
        Some(event_json),
        &grant_created_at,
    )?;

    mutate_as_admin_with_grants(
        state,
        vault_id,
        actor,
        event,
        payload,
        grants.clone(),
        |store, vault_id| store.finish_folder_setup(vault_id, &folder_id, &grants),
    )
    .map(Json)
}

pub(crate) async fn grant_folder_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<GrantFolderAccessResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: GrantFolderAccessRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let vault_id = VaultId::new(vault_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let target_identity = resolve_and_record_identity(&state, &request.target_npub)?;
    let target = UserId::new(target_identity.npub.clone())?;
    let current_key_version = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        ensure_vault_admin(&stored, &actor)?;
        folder_current_key_version(&stored, &folder_id)?
    };
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        &actor,
        AdminAccessAction::GrantFolderAccess,
        Some(&folder_id),
        Some(target.as_str()),
        Some(current_key_version),
    )?;
    let grant_created_at = server_timestamp(&state);
    let mut grant_request = request.grant;
    grant_request.recipient_npub = target.as_str().to_owned();
    let grant = grant_request_to_metadata(
        &grant_request,
        &folder_id,
        &actor,
        Some(event.as_json()),
        &grant_created_at,
    )?;
    let control_records = [
        folder_key_grant_sync_record(&grant)?,
        admin_access_change_sync_record(&actor, &event, &payload)?,
    ];

    let (metadata, outcome) = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        ensure_vault_admin(&stored, &actor)?;
        let outcome = store.grant_folder_access_with_control_records(
            &vault_id,
            &folder_id,
            &target,
            &grant,
            &control_records,
        )?;
        let stored = store.load_vault(&vault_id)?;
        let mut metadata = metadata_response(stored);
        enrich_metadata_identities(&store, &mut metadata)?;
        (metadata, outcome)
    };
    let outcome = match outcome {
        GrantFolderAccessOutcome::Granted => GrantFolderAccessResponseOutcome::Granted,
        GrantFolderAccessOutcome::AlreadyHasAccess => {
            GrantFolderAccessResponseOutcome::AlreadyHasAccess
        }
    };
    Ok(Json(GrantFolderAccessResponse { metadata, outcome }))
}

pub(crate) async fn remove_folder_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, folder_id, target_npub)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> Result<Json<VaultMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: RemoveFolderAccessRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    validate_folder_rotation_fanout(
        FolderRotationOperation::FolderAccessRemoval,
        [FolderRotationFanout {
            grants: request.grants.len(),
            reencrypted_records: request.reencrypted_records.len(),
        }],
    )?;
    let vault_id = VaultId::new(vault_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub)?;
    let target = UserId::new(target_identity.npub.clone())?;
    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_vault(&vault_id)?;
        ensure_vault_admin(&stored, &actor)?;
    }
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &vault_id,
        &actor,
        AdminAccessAction::RemoveFolderAccess,
        Some(&folder_id),
        Some(target.as_str()),
        Some(request.new_key_version),
    )?;
    let event_json = event.as_json();
    let grant_created_at = server_timestamp(&state);
    let updated_at = grant_created_at.clone();
    let grants = grant_requests_to_metadata(
        &request.grants,
        &folder_id,
        &actor,
        Some(event_json),
        &grant_created_at,
    )?;
    let mut reencrypted_records = Vec::new();
    for record in request.reencrypted_records {
        if record.key_version != request.new_key_version {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "rotation record keyVersion must match newKeyVersion",
            ));
        }
        let object_id = ObjectId::new(record.object_id)?;
        let write_request = ObjectWriteRequest {
            base_revision: record.base_revision,
            key_version: record.key_version,
            cipher: record.cipher,
            ciphertext: record.ciphertext,
            revision_event: record.revision_event,
        };
        let (record, _) = validate_object_revision_record(
            &vault_id,
            &folder_id,
            &object_id,
            &actor,
            write_request,
            FolderObjectOperation::Update,
        )?;
        reencrypted_records.push(record);
    }

    mutate_as_admin_with_grants(
        state,
        vault_id,
        actor,
        event,
        payload,
        grants.clone(),
        |store, vault_id| {
            store.rotate_folder_key_for_access_removal(
                vault_id,
                &folder_id,
                &target,
                request.new_key_version,
                &grants,
                &reencrypted_records,
                &updated_at,
            )
        },
    )
    .map(Json)
}
