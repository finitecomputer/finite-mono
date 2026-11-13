//! Member removal pre-flight: which native Folders the removal must rotate.
//!
//! The client cannot see who holds each Folder's current key, and a removed
//! key that keeps a current wrap can still decrypt that Folder. Rather than
//! ship per-key grant detail in Brain metadata and repeat the permission rule
//! in every client, the server answers from the same store function the
//! removal enforces (`member_removal_folder_rotations`).

use crate::*;
use finite_brain_store::{MemberRemovalRotationReason, member_removal_folder_rotations};

pub(crate) async fn member_removal_plan_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, target_npub)): AxumPath<(String, String)>,
) -> Result<Json<MemberRemovalPlanResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let target = UserId::new(canonical_member_removal_target(&target_npub)?)?;
    let planned = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        member_removal_folder_rotations(&stored, &target)?
    };
    Ok(Json(MemberRemovalPlanResponse {
        brain_id: brain_id.to_string(),
        target_npub: target.to_string(),
        folder_rotations: planned
            .into_iter()
            .map(|rotation| MemberRemovalFolderRotationResponse {
                folder_id: rotation.folder_id.to_string(),
                current_key_version: rotation.current_key_version,
                reason: match rotation.reason {
                    MemberRemovalRotationReason::Readable => {
                        MemberRemovalRotationReasonResponse::Readable
                    }
                    MemberRemovalRotationReason::StaleCurrentGrant => {
                        MemberRemovalRotationReasonResponse::StaleCurrentGrant
                    }
                },
            })
            .collect(),
    }))
}

/// The plan is read-only, so it takes a public key (npub or hex) and never
/// resolves or records a NIP-05 identity; `fbrain` resolves the target first.
fn canonical_member_removal_target(value: &str) -> Result<String, ApiError> {
    NostrPublicKey::parse(value)
        .and_then(|public_key| public_key.to_npub())
        .map_err(|error| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("invalid member removal target: {error}"),
            )
        })
}
