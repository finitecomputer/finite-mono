//! Folder Key rotation routes: explicit access removal and standalone rotation.
//!
//! Both routes accept the same client-prepared material and commit through
//! `BrainStore::rotate_folder_key`; the signed admin access-change event binds
//! the action, Folder, optional target and new key version.

use crate::*;
use finite_brain_store::{FolderKeyRotationRequest, FolderKeyRotationTarget};

/// Validated, server-shaped rotation material for one Folder.
struct PreparedRotation {
    new_key_version: u32,
    grants: Vec<FolderKeyGrantMetadata>,
    reencrypted_records: Vec<FolderObjectRevisionSyncRecord>,
    control_records: Vec<SyncRecordInput>,
    updated_at: String,
}

/// Authorize the admin, then verify the signed event for `action` before any
/// grant or record is parsed into store input. Fan-out is already bounded by
/// [`parse_rotation_body`].
fn prepare_rotation(
    state: &ServerState,
    actor: &str,
    brain_id: &BrainId,
    folder_id: &FolderId,
    target: Option<&UserId>,
    action: AdminAccessAction,
    request: FolderKeyRotationMaterialRequest,
) -> Result<PreparedRotation, ApiError> {
    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(brain_id)?;
        ensure_brain_admin(&stored, actor)?;
    }
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        brain_id,
        actor,
        action,
        Some(folder_id),
        target.map(UserId::as_str),
        Some(request.new_key_version),
    )?;
    let updated_at = server_timestamp(state);
    let grants = grant_requests_to_metadata(
        &request.grants,
        folder_id,
        actor,
        Some(event.as_json()),
        &updated_at,
    )?;
    let reencrypted_records = rotation_records_from_requests(
        brain_id,
        folder_id,
        actor,
        request.new_key_version,
        request.reencrypted_records,
    )?;
    let control_records = admin_mutation_control_records(&grants, actor, &event, &payload)?;
    Ok(PreparedRotation {
        new_key_version: request.new_key_version,
        grants,
        reencrypted_records,
        control_records,
        updated_at,
    })
}

impl PreparedRotation {
    fn store_request<'a>(&'a self, folder_id: &'a FolderId) -> FolderKeyRotationRequest<'a> {
        FolderKeyRotationRequest {
            folder_id,
            new_key_version: self.new_key_version,
            grants: &self.grants,
            reencrypted_records: &self.reencrypted_records,
            updated_at: &self.updated_at,
            control_records: &self.control_records,
        }
    }
}

/// Parse rotation material and bound its fan-out before any identity lookup,
/// alias write, cryptography or store read.
fn parse_rotation_body(
    body: &Bytes,
    operation: FolderRotationOperation,
) -> Result<FolderKeyRotationMaterialRequest, ApiError> {
    let request: FolderKeyRotationMaterialRequest = serde_json::from_slice(body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    validate_folder_rotation_fanout(
        operation,
        [FolderRotationFanout {
            grants: request.grants.len(),
            reencrypted_records: request.reencrypted_records.len(),
        }],
    )?;
    Ok(request)
}

/// Remove one key's Folder access: its explicit access row, or a current
/// Folder Key wrap it holds without entitlement. Rotates the Folder Key.
pub(crate) async fn remove_folder_access_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id, target_npub)): AxumPath<(String, String, String)>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request = parse_rotation_body(&body, FolderRotationOperation::FolderAccessRemoval)?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let prepared = prepare_rotation(
        &state,
        &actor,
        &brain_id,
        &folder_id,
        Some(&target),
        AdminAccessAction::RemoveFolderAccess,
        request,
    )?;

    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let response = run_as_admin(state, brain_id, actor, |store, brain_id| {
        store
            .rotate_folder_key(
                brain_id,
                FolderKeyRotationTarget::RemoveAccess(&target),
                prepared.store_request(&folder_id),
            )
            .map(|_| ())
    })?;
    notification_state.publish_access_update_for(&notification_brain_id, target.as_str());
    Ok(Json(response))
}

/// Rotate one Folder Key to its next version for exactly the required
/// recipients, with no removed target. Every key holding the current wrap
/// without entitlement is cut off.
pub(crate) async fn rotate_folder_key_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<RotateFolderKeyResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request = parse_rotation_body(&body, FolderRotationOperation::FolderKeyRotation)?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let prepared = prepare_rotation(
        &state,
        &actor,
        &brain_id,
        &folder_id,
        None,
        AdminAccessAction::RotateFolderKey,
        request,
    )?;

    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let mut outcome = None;
    let metadata = run_as_admin(state, brain_id, actor, |store, brain_id| {
        outcome = Some(store.rotate_folder_key(
            brain_id,
            FolderKeyRotationTarget::RequiredRecipients,
            prepared.store_request(&folder_id),
        )?);
        Ok(())
    })?;
    let outcome = outcome
        .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error"))?;
    // Recipients must refetch the new wrap and cut-off keys must observe the
    // loss; both are access changes for this Brain.
    notification_state.publish_access_update(&notification_brain_id);
    Ok(Json(RotateFolderKeyResponse {
        metadata,
        rotation: FolderKeyRotationReceipt {
            folder_id: outcome.folder_id.to_string(),
            previous_key_version: outcome.previous_key_version,
            new_key_version: outcome.new_key_version,
            recipient_npubs: outcome.recipients.iter().map(ToString::to_string).collect(),
            cut_off_npubs: outcome
                .cut_off_holders
                .iter()
                .map(ToString::to_string)
                .collect(),
        },
    }))
}
