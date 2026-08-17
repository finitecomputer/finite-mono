use crate::*;

pub(crate) async fn brain_updates_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
) -> Result<impl IntoResponse, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let mut updates = state.brain_updates.subscribe();
    let stream_state = state.clone();
    let stream = async_stream::stream! {
        yield Ok::<SseEvent, Infallible>(SseEvent::default().event("ready").data("{}"));
        loop {
            match updates.recv().await {
                Ok(update) if update.notify_npubs.iter().any(|npub| npub == &actor)
                    || stream_state.actor_can_see_brain(&actor, &update.brain_id) => {
                    let data = serde_json::to_string(&update).unwrap_or_else(|_| "{}".to_owned());
                    yield Ok(SseEvent::default().event("brain_update").data(data));
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => break,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub(crate) async fn put_object_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id, object_id)): AxumPath<(String, String, String)>,
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
    let notification_brain_id = brain_id.clone();
    let response = accept_object_revision(
        state.clone(),
        brain_id,
        folder_id,
        object_id,
        actor,
        request,
        operation,
    )?;
    if !response.duplicate {
        state.publish_brain_update(
            notification_brain_id,
            response.sequence,
            BrainUpdateReason::ContentUpdated,
        );
    }
    Ok(Json(response))
}

pub(crate) async fn move_object_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id, object_id)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> Result<Json<ObjectWriteResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: ObjectWriteRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let notification_brain_id = brain_id.clone();
    let response = accept_object_revision(
        state.clone(),
        brain_id,
        folder_id,
        object_id,
        actor,
        request,
        FolderObjectOperation::Move,
    )?;
    if !response.duplicate {
        state.publish_brain_update(
            notification_brain_id,
            response.sequence,
            BrainUpdateReason::ContentUpdated,
        );
    }
    Ok(Json(response))
}

pub(crate) async fn delete_object_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id, object_id)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> Result<Json<ObjectWriteResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: ObjectDeleteRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let notification_brain_id = brain_id.clone();
    let response = accept_object_tombstone(
        state.clone(),
        brain_id,
        folder_id,
        object_id,
        actor,
        request,
    )?;
    if !response.duplicate {
        state.publish_brain_update(
            notification_brain_id,
            response.sequence,
            BrainUpdateReason::ContentUpdated,
        );
    }
    Ok(Json(response))
}

pub(crate) async fn get_object_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id, object_id)): AxumPath<(String, String, String)>,
) -> Result<Json<ObjectResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let object_id = ObjectId::new(object_id)?;
    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain(&brain_id)?
    };
    ensure_folder_visible(&stored, &folder_id, &actor)?;
    let bootstrap = {
        let store = state.store.lock().map_err(lock_error)?;
        store.sync_bootstrap(&brain_id)?
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
        brain_id: brain_id.to_string(),
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
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<SyncBootstrapResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain(&brain_id)?
    };
    ensure_metadata_visible(&stored, &actor)?;
    let bootstrap = {
        let store = state.store.lock().map_err(lock_error)?;
        store.sync_bootstrap(&brain_id)?
    };
    let objects = bootstrap
        .objects
        .into_iter()
        .filter(|object| folder_visible(&stored, &object.folder_id, &actor))
        .map(|object| ObjectResponse {
            brain_id: brain_id.to_string(),
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
    // Pending grant wraps are visible only to key-holding clients (Brain
    // admin standing): they are the ones who can open the current Folder
    // Keys and complete the wraps. Older clients ignore the field.
    let pending_wraps = if ensure_brain_admin(&stored, &actor).is_ok() {
        let store = state.store.lock().map_err(lock_error)?;
        store
            .pending_grant_wraps(&brain_id)?
            .into_iter()
            .map(pending_grant_wrap_response)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    Ok(Json(SyncBootstrapResponse {
        brain_id: brain_id.to_string(),
        latest_sequence: bootstrap.latest_sequence,
        object_count: objects.len(),
        objects,
        control_records,
        current_state_kind: bootstrap.current_state_kind.to_owned(),
        pending_wraps,
    }))
}

pub(crate) async fn sync_records_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    Query(query): Query<SyncRecordsQuery>,
) -> Result<Json<SyncPullResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain(&brain_id)?
    };
    ensure_metadata_visible(&stored, &actor)?;
    let pull = {
        let store = state.store.lock().map_err(lock_error)?;
        let limit = query.limit.unwrap_or(100).clamp(1, MAX_SYNC_RECORDS_LIMIT);
        store.pull_sync_records(&brain_id, query.after.unwrap_or(0), limit)?
    };
    let records = pull
        .records
        .into_iter()
        .filter(|record| record_visible(&stored, record, &actor))
        .map(sync_record_response)
        .collect::<Vec<_>>();
    Ok(Json(SyncPullResponse {
        brain_id: brain_id.to_string(),
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
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<ObjectWriteResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let value: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let record_type = value
        .get("recordType")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "recordType is required"))?;
    let notification_brain_id = brain_id.clone();
    let response = match record_type {
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
                state.clone(),
                brain_id,
                folder_id,
                object_id,
                actor,
                request,
                operation,
            )
        }
        "folder_object_tombstone" => {
            let request: ObjectDeleteRequest = serde_json::from_value(value)
                .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid tombstone record"))?;
            let folder_id = request_field(&body, "folderId")?;
            let object_id = request_field(&body, "objectId")?;
            accept_object_tombstone(
                state.clone(),
                brain_id,
                folder_id,
                object_id,
                actor,
                request,
            )
        }
        _ => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "unsupported recordType",
            ));
        }
    }?;
    if !response.duplicate {
        state.publish_brain_update(
            notification_brain_id,
            response.sequence,
            BrainUpdateReason::ContentUpdated,
        );
    }
    Ok(Json(response))
}
