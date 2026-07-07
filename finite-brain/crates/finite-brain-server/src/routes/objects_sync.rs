use crate::*;

pub(crate) async fn put_object_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, folder_id, object_id)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> Result<Json<ObjectWriteResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: ObjectWriteRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let operation = if request.base_revision.is_some() {
        FolderObjectOperation::Update
    } else {
        FolderObjectOperation::Create
    };
    accept_object_revision(
        state, vault_id, folder_id, object_id, actor, request, operation,
    )
    .map(Json)
}

pub(crate) async fn move_object_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, folder_id, object_id)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> Result<Json<ObjectWriteResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: ObjectWriteRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    accept_object_revision(
        state,
        vault_id,
        folder_id,
        object_id,
        actor,
        request,
        FolderObjectOperation::Move,
    )
    .map(Json)
}

pub(crate) async fn delete_object_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, folder_id, object_id)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> Result<Json<ObjectWriteResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: ObjectDeleteRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    accept_object_tombstone(state, vault_id, folder_id, object_id, actor, request).map(Json)
}

pub(crate) async fn get_object_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((vault_id, folder_id, object_id)): AxumPath<(String, String, String)>,
) -> Result<Json<ObjectResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let vault_id = VaultId::new(vault_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let object_id = ObjectId::new(object_id)?;
    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_vault(&vault_id)?
    };
    ensure_folder_visible(&stored, &folder_id, &actor)?;
    let bootstrap = {
        let store = state.store.lock().map_err(lock_error)?;
        store.sync_bootstrap(&vault_id)?
    };
    let object = bootstrap
        .objects
        .into_iter()
        .find(|object| object.folder_id == folder_id && object.object_id == object_id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "object not found"))?;
    if object.deleted {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "object not found"));
    }

    Ok(Json(ObjectResponse {
        vault_id: vault_id.to_string(),
        folder_id: object.folder_id.to_string(),
        object_id: object.object_id.as_str().to_owned(),
        revision: object.revision,
        ciphertext: object_ciphertext(&object.payload_json),
        deleted: object.deleted,
    }))
}

pub(crate) async fn sync_bootstrap_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
) -> Result<Json<SyncBootstrapResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let vault_id = VaultId::new(vault_id)?;
    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_vault(&vault_id)?
    };
    ensure_metadata_visible(&stored, &actor)?;
    let bootstrap = {
        let store = state.store.lock().map_err(lock_error)?;
        store.sync_bootstrap(&vault_id)?
    };
    let objects = bootstrap
        .objects
        .into_iter()
        .filter(|object| folder_visible(&stored, &object.folder_id, &actor))
        .map(|object| ObjectResponse {
            vault_id: vault_id.to_string(),
            folder_id: object.folder_id.to_string(),
            object_id: object.object_id.as_str().to_owned(),
            revision: object.revision,
            ciphertext: object_ciphertext(&object.payload_json),
            deleted: object.deleted,
        })
        .collect::<Vec<_>>();
    let control_records = bootstrap
        .control_records
        .into_iter()
        .filter(|record| record_visible(&stored, record, &actor))
        .map(sync_record_response)
        .collect::<Vec<_>>();

    Ok(Json(SyncBootstrapResponse {
        vault_id: vault_id.to_string(),
        latest_sequence: bootstrap.latest_sequence,
        object_count: objects.len(),
        objects,
        control_records,
        current_state_kind: bootstrap.current_state_kind.to_owned(),
    }))
}

pub(crate) async fn sync_records_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
    Query(query): Query<SyncRecordsQuery>,
) -> Result<Json<SyncPullResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let vault_id = VaultId::new(vault_id)?;
    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_vault(&vault_id)?
    };
    ensure_metadata_visible(&stored, &actor)?;
    let pull = {
        let store = state.store.lock().map_err(lock_error)?;
        let limit = query.limit.unwrap_or(100).clamp(1, MAX_SYNC_RECORDS_LIMIT);
        store.pull_sync_records(&vault_id, query.after.unwrap_or(0), limit)?
    };
    let records = pull
        .records
        .into_iter()
        .filter(|record| record_visible(&stored, record, &actor))
        .map(sync_record_response)
        .collect::<Vec<_>>();
    Ok(Json(SyncPullResponse {
        vault_id: vault_id.to_string(),
        after_sequence: pull.after_sequence,
        latest_sequence: pull.latest_sequence,
        count: records.len(),
        records,
        has_more: pull.has_more,
        next_sequence: pull.next_sequence,
    }))
}

pub(crate) async fn submit_sync_record_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(vault_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<ObjectWriteResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let value: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let record_type = value
        .get("recordType")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "recordType is required"))?;
    match record_type {
        "folder_object_revision" => {
            let request: ObjectWriteRequest = serde_json::from_value(value)
                .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid revision record"))?;
            let folder_id = request_field(&body, "folderId")?;
            let object_id = request_field(&body, "objectId")?;
            let operation = if request.base_revision.is_some() {
                FolderObjectOperation::Update
            } else {
                FolderObjectOperation::Create
            };
            accept_object_revision(
                state, vault_id, folder_id, object_id, actor, request, operation,
            )
            .map(Json)
        }
        "folder_object_tombstone" => {
            let request: ObjectDeleteRequest = serde_json::from_value(value)
                .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid tombstone record"))?;
            let folder_id = request_field(&body, "folderId")?;
            let object_id = request_field(&body, "objectId")?;
            accept_object_tombstone(state, vault_id, folder_id, object_id, actor, request).map(Json)
        }
        _ => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported recordType",
        )),
    }
}
