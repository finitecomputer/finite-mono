use crate::*;

pub(crate) fn accept_object_revision(
    state: ServerState,
    brain_id: String,
    folder_id: String,
    object_id: String,
    actor_npub: String,
    request: ObjectWriteRequest,
    operation: FolderObjectOperation,
) -> Result<ObjectWriteResponse, ApiError> {
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let object_id = ObjectId::new(object_id)?;

    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain(&brain_id)?
    };
    ensure_folder_visible(&stored, &folder_id, &actor_npub)?;
    ensure_folder_key_version(&stored, &folder_id, request.key_version)?;
    let request_key_version = request.key_version;

    let (record, revision) = validate_object_revision_record(
        &brain_id,
        &folder_id,
        &object_id,
        &actor_npub,
        request,
        operation,
    )?;
    let outcome = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_folder_visible(&stored, &folder_id, &actor_npub)?;
        ensure_folder_key_version(&stored, &folder_id, request_key_version)?;
        store.submit_sync_record(&brain_id, &SyncRecordInput::FolderObjectRevision(record))?
    };

    Ok(ObjectWriteResponse {
        sequence: outcome.sequence,
        duplicate: outcome.duplicate,
        revision,
    })
}

pub(crate) fn validate_object_revision_record(
    brain_id: &BrainId,
    folder_id: &FolderId,
    object_id: &ObjectId,
    actor_npub: &str,
    request: ObjectWriteRequest,
    operation: FolderObjectOperation,
) -> Result<(FolderObjectRevisionSyncRecord, u64), ApiError> {
    if request.cipher != "AES-256-GCM" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "cipher must be AES-256-GCM",
        ));
    }
    let revision = request.base_revision.map_or(1, |base| base + 1);
    let base_revision = request.base_revision;
    let key_version = request.key_version;
    let cipher = request.cipher;
    let ciphertext = request.ciphertext;
    let event = event_from_value(request.revision_event)?;
    let expected = RevisionValidation {
        brain_id: brain_id.clone(),
        folder_id: folder_id.clone(),
        object_id: object_id.clone(),
        operation,
        revision,
        base_revision,
        key_version,
        envelope_json: ciphertext.clone(),
        author_npub: actor_npub.to_owned(),
        created_at: expected_created_at(&event)?,
    };
    let payload: FolderObjectRevisionPayload = validate_revision_event(&event, &expected)?;
    let payload_json = serde_json::json!({
        "recordType": "folder_object_revision",
        "folderId": folder_id.to_string(),
        "objectId": object_id.as_str(),
        "baseRevision": base_revision,
        "keyVersion": key_version,
        "cipher": cipher,
        "ciphertext": ciphertext,
        "revisionEvent": event,
    })
    .to_string();
    Ok((
        FolderObjectRevisionSyncRecord {
            record_event_id: event.id.to_hex(),
            folder_id: folder_id.clone(),
            object_id: object_id.clone(),
            revision,
            base_revision,
            actor_npub: UserId::new(actor_npub.to_owned())?,
            client_created_at: payload.created_at,
            payload_json,
            record_event_kind: event.kind.as_u16(),
        },
        revision,
    ))
}

pub(crate) fn rotation_records_from_requests(
    brain_id: &BrainId,
    folder_id: &FolderId,
    actor_npub: &str,
    new_key_version: u32,
    requests: Vec<RotationObjectRequest>,
) -> Result<Vec<FolderObjectRevisionSyncRecord>, ApiError> {
    let mut records = Vec::new();
    for request in requests {
        if request.key_version != new_key_version {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "rotation record keyVersion must match newKeyVersion",
            ));
        }
        let object_id = ObjectId::new(request.object_id)?;
        let write_request = ObjectWriteRequest {
            base_revision: request.base_revision,
            key_version: request.key_version,
            cipher: request.cipher,
            ciphertext: request.ciphertext,
            revision_event: request.revision_event,
        };
        let (record, _) = validate_object_revision_record(
            brain_id,
            folder_id,
            &object_id,
            actor_npub,
            write_request,
            FolderObjectOperation::Update,
        )?;
        records.push(record);
    }
    Ok(records)
}

pub(crate) fn accept_object_tombstone(
    state: ServerState,
    brain_id: String,
    folder_id: String,
    object_id: String,
    actor_npub: String,
    request: ObjectDeleteRequest,
) -> Result<ObjectWriteResponse, ApiError> {
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let object_id = ObjectId::new(object_id)?;
    let revision = request.base_revision + 1;
    let event = event_from_value(request.tombstone_event)?;

    let stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain(&brain_id)?
    };
    ensure_folder_visible(&stored, &folder_id, &actor_npub)?;
    ensure_direct_delete_authority(&stored, &actor_npub)?;

    let expected = TombstoneValidation {
        brain_id: brain_id.clone(),
        folder_id: folder_id.clone(),
        object_id: object_id.clone(),
        revision,
        base_revision: request.base_revision,
        author_npub: actor_npub.clone(),
        deleted_at: expected_created_at(&event)?,
    };
    let payload: FolderObjectTombstonePayload = validate_tombstone_event(&event, &expected)?;
    let payload_json = serde_json::json!({
        "recordType": "folder_object_tombstone",
        "folderId": folder_id.to_string(),
        "objectId": object_id.as_str(),
        "baseRevision": request.base_revision,
        "tombstoneEvent": event,
    })
    .to_string();
    let outcome = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_folder_visible(&stored, &folder_id, &actor_npub)?;
        ensure_direct_delete_authority(&stored, &actor_npub)?;
        store.submit_sync_record(
            &brain_id,
            &SyncRecordInput::FolderObjectTombstone(FolderObjectTombstoneSyncRecord {
                record_event_id: event.id.to_hex(),
                folder_id,
                object_id,
                revision,
                base_revision: request.base_revision,
                actor_npub: UserId::new(actor_npub)?,
                client_created_at: payload.deleted_at,
                payload_json,
                record_event_kind: event.kind.as_u16(),
            }),
        )?
    };

    Ok(ObjectWriteResponse {
        sequence: outcome.sequence,
        duplicate: outcome.duplicate,
        revision,
    })
}
