//! Admin demotion routes.
//!
//! `POST .../roles/admin/{target}/demotion` removes the admin role and, in the
//! same store transaction, rotates every Folder Key the demoted key reached
//! only through that role. Each rotation carries its own signed
//! `rotate-folder-key` event, so the audit trail records the demotion and
//! every rotation it caused.
//!
//! `DELETE .../roles/admin/{target}` is the request shape older `fbrain`
//! releases send. It carries no rotations, so it succeeds only when the
//! demotion has nothing to rotate and otherwise refuses without changing
//! anything. An older server does not know the `demotion` route, so a newer
//! `fbrain` fails closed there instead of demoting without rotating.

use crate::*;
use finite_brain_store::{AdminDemotionRequest, FolderKeyRotationOutcome, MemberFolderRotation};

/// Remove the admin role with the rotations it requires, atomically.
pub(crate) async fn demote_admin_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<DemoteAdminResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: DemoteAdminRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    validate_folder_rotation_fanout(
        FolderRotationOperation::AdminDemotion,
        request
            .rotations
            .iter()
            .map(|rotation| FolderRotationFanout {
                grants: rotation.material.grants.len(),
                reencrypted_records: rotation.material.reencrypted_records.len(),
            }),
    )?;
    let brain_id = BrainId::new(brain_id)?;
    {
        let store = state.store.lock().map_err(lock_error)?;
        ensure_brain_admin(&store.load_brain(&brain_id)?, &actor)?;
    }
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::RemoveAdmin,
        None,
        Some(target.as_str()),
        None,
    )?;
    let updated_at = server_timestamp(&state);
    let mut control_records = vec![admin_access_change_sync_record(&actor, &event, &payload)?];
    let mut rotations = Vec::with_capacity(request.rotations.len());
    for rotation in request.rotations {
        let folder_id = FolderId::new(rotation.folder_id)?;
        let prepared = rotation_material(
            &actor,
            &brain_id,
            &folder_id,
            None,
            AdminAccessAction::RotateFolderKey,
            rotation.material,
            updated_at.clone(),
        )?;
        control_records.extend(prepared.control_records);
        rotations.push(MemberFolderRotation {
            folder_id,
            new_key_version: prepared.new_key_version,
            grants: prepared.grants,
            reencrypted_records: prepared.reencrypted_records,
        });
    }

    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let mut outcomes = Vec::new();
    let metadata = run_as_admin(state, brain_id, actor, |store, brain_id| {
        outcomes = store.remove_admin(
            brain_id,
            AdminDemotionRequest {
                removed_user_id: &target,
                rotations: &rotations,
                updated_at: &updated_at,
                control_records: &control_records,
            },
        )?;
        Ok(())
    })?;
    publish_demotion(
        &notification_state,
        &notification_brain_id,
        &target,
        &outcomes,
    );
    Ok(Json(DemoteAdminResponse {
        metadata,
        rotations: outcomes.iter().map(rotation_receipt).collect(),
    }))
}

/// Older `fbrain` request shape: the signed `remove-admin` event alone.
pub(crate) async fn remove_admin_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: AdminEventRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let target_identity = resolve_and_record_identity(&state, &target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
        &brain_id,
        &actor,
        AdminAccessAction::RemoveAdmin,
        None,
        Some(target.as_str()),
        None,
    )?;
    let control_records = admin_mutation_control_records(&[], &actor, &event, &payload)?;
    let updated_at = server_timestamp(&state);
    let notification_state = state.clone();
    let notification_brain_id = brain_id.clone();
    let mut required_rotations = None;
    let response = run_as_admin(state, brain_id, actor, |store, brain_id| {
        let result = store.remove_admin(
            brain_id,
            AdminDemotionRequest {
                removed_user_id: &target,
                rotations: &[],
                updated_at: &updated_at,
                control_records: &control_records,
            },
        );
        if let Err(StoreError::FolderRotationSetMismatch {
            required_folder_ids,
            ..
        }) = &result
        {
            required_rotations = Some(required_folder_ids.clone());
        }
        result.map(|_| ())
    })
    .map_err(|error| match required_rotations.take() {
        Some(folder_ids) => rotations_required_by_older_client(&target, &folder_ids),
        None => error,
    })?;
    publish_demotion(&notification_state, &notification_brain_id, &target, &[]);
    Ok(Json(response))
}

/// The older request shape cannot carry rotations. Say what to do instead of
/// only naming the mismatched sets; nothing was changed.
fn rotations_required_by_older_client(target: &UserId, folder_ids: &[String]) -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        format!(
            "revoking admin from {target} must also rotate the Folder Keys of [{}], which it holds only as an admin, and this request carried no rotations; nothing was changed. Upgrade fbrain and run `fbrain admin role revoke admin --target {target}` again",
            folder_ids.join(", ")
        ),
    )
}

/// The demoted key must observe its role change; when Folder Keys rotated,
/// every remaining recipient must also refetch the new wraps.
fn publish_demotion(
    state: &ServerState,
    brain_id: &BrainId,
    target: &UserId,
    outcomes: &[FolderKeyRotationOutcome],
) {
    if outcomes.is_empty() {
        state.publish_access_update_for(brain_id, target.as_str());
    } else {
        state.publish_access_update(brain_id);
    }
}
