//! Permanent Organization Brain deletion over the signed API: a preview the
//! admin confirms, the deletion that rechecks it atomically, and the `410`
//! answer that tells former readers (and only them) the Brain is gone.

use axum::extract::Request;
use axum::middleware::Next;
use finite_brain_store::{
    BRAIN_DELETION_DIGEST_PREFIX, BrainDeletionOutcome, BrainDeletionReceipt, BrainDeletionRequest,
};

use crate::protected_routes::answered_request_signer;
use crate::*;

/// How long a signed `delete-brain` confirmation stays acceptable. An exact
/// retry of an accepted deletion is answered at any age.
pub(crate) const BRAIN_DELETION_CONFIRMATION_MAX_AGE_SECONDS: u64 = 10 * 60;

pub(crate) async fn brain_deletion_preview_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<BrainDeletionPreviewResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    spawn_store(state, move |state| {
        let brain_id = BrainId::new(brain_id)?;
        let server = server_identity(&state);
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_metadata_visible(&stored, &actor)?;
        ensure_direct_delete_authority(&stored, &actor)?;
        let preview = store.brain_deletion_preview(&brain_id, &UserId::new(actor)?, &server)?;
        Ok(Json(BrainDeletionPreviewResponse {
            server,
            brain_id: preview.brain_id.to_string(),
            name: preview.name,
            kind: "organization".to_owned(),
            folder_count: preview.folder_count,
            page_count: preview.object_count,
            member_count: preview.member_count,
            admin_count: preview.admin_count,
            guest_count: preview.guest_count,
            pending_invitation_count: preview.pending_invitation_count,
            open_invite_token_count: preview.open_invite_token_count,
            deletable: preview.blocking_mounts.is_empty(),
            blocking_mounts: preview
                .blocking_mounts
                .into_iter()
                .map(|mount| BlockingMountResponse {
                    connection_id: mount.connection_id,
                    direction: match mount.direction {
                        SharedFolderDirection::Source => "outgoing",
                        SharedFolderDirection::Destination => "incoming",
                    }
                    .to_owned(),
                    source_brain_id: mount.source_brain_id.to_string(),
                    source_folder_id: mount.source_folder_id.to_string(),
                    destination_brain_id: mount.destination_brain_id.to_string(),
                })
                .collect(),
            retired_mount_record_count: preview.retired_mount_record_count,
            state_digest: preview.state_digest,
            confirmation_max_age_seconds: BRAIN_DELETION_CONFIRMATION_MAX_AGE_SECONDS,
        }))
    })
    .await
}

pub(crate) async fn delete_brain_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<BrainDeleteResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: BrainDeleteRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    // The signed event is validated against the request alone, so a refusal
    // here never depends on whether the Brain exists.
    let (event, payload) = validate_admin_access_change_value(
        request.deletion_event,
        &brain_id,
        &actor,
        AdminAccessAction::DeleteBrain,
        None,
        None,
        None,
    )?;
    let state_digest = payload
        .note
        .filter(|note| is_state_digest(note))
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "delete-brain must sign the confirmed deletion preview stateDigest as its note",
            )
        })?;
    let now = state.auth_now_unix_seconds();
    let signed_at = event.created_at.as_secs();
    let confirmation_current =
        signed_at.saturating_add(BRAIN_DELETION_CONFIRMATION_MAX_AGE_SECONDS) >= now
            && signed_at <= now.saturating_add(state.max_auth_skew_seconds);
    let deletion_event_id = event.id.to_hex();
    let deletion_event_json = event.as_json();
    let deleted_at = server_timestamp(&state);
    let outcome = spawn_store(state.clone(), move |state| {
        let actor_id = UserId::new(actor.clone())?;
        let server = server_identity(&state);
        let mut store = state.store.lock().map_err(lock_error)?;
        if let Some(receipt) =
            store.brain_deletion_replay(&brain_id, &deletion_event_id, &actor_id)?
        {
            return Ok(BrainDeletionOutcome::Replayed { receipt });
        }
        let stored = store.load_brain(&brain_id)?;
        ensure_metadata_visible(&stored, &actor)?;
        ensure_direct_delete_authority(&stored, &actor)?;
        Ok(store.delete_organization_brain(&BrainDeletionRequest {
            brain_id: &brain_id,
            actor_npub: &actor_id,
            deletion_event_id: &deletion_event_id,
            deletion_event_json: &deletion_event_json,
            expected_state_digest: &state_digest,
            server_identity: &server,
            deleted_at: &deleted_at,
            confirmation_current,
        })?)
    })
    .await?;
    let (receipt, duplicate) = match outcome {
        BrainDeletionOutcome::Deleted { receipt, audience } => {
            state.publish_brain_deleted(&receipt.brain_id, &audience);
            (receipt, false)
        }
        BrainDeletionOutcome::Replayed { receipt } => (receipt, true),
    };
    Ok(Json(BrainDeleteResponse {
        receipt: receipt_response(receipt),
        duplicate,
    }))
}

/// Turn the unknown-Brain `404` of any Brain-scoped request into a `410 Gone`
/// deletion notice when, and only when, the signer held access to that Brain
/// when it was deleted. Everyone else keeps the unknown-Brain answer, so a
/// deleted Brain id reveals nothing to unrelated principals.
pub(crate) async fn deleted_brain_notice_middleware(
    State(state): State<ServerState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(brain_id) = brain_id_in_path(request.uri().path()) else {
        return next.run(request).await;
    };
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    let response = next.run(request).await;
    if response.status() != StatusCode::NOT_FOUND {
        return response;
    }
    let Some(signer) = answered_request_signer(&state, &headers, &method, &uri) else {
        return response;
    };
    let notice = UserId::new(signer).ok().and_then(|signer| {
        let store = state.store.lock().ok()?;
        store
            .deleted_brain_notice(&brain_id, &signer)
            .ok()
            .flatten()
    });
    match notice {
        Some(notice) => (
            StatusCode::GONE,
            Json(DeletedBrainResponse {
                error: format!(
                    "Brain {} was permanently deleted at {}",
                    notice.brain_id, notice.deleted_at
                ),
                code: "brain_deleted".to_owned(),
                brain_deleted: DeletedBrainNoticeResponse {
                    brain_id: notice.brain_id.to_string(),
                    deleted_at: notice.deleted_at,
                },
            }),
        )
            .into_response(),
        None => response,
    }
}

/// The Brain id of `/v1/brains/{id}[/...]` and `/v1/admin/brains/{id}/...`.
fn brain_id_in_path(path: &str) -> Option<BrainId> {
    let rest = path
        .strip_prefix("/v1/brains/")
        .or_else(|| path.strip_prefix("/v1/admin/brains/"))?;
    let id = rest.split('/').next()?;
    BrainId::new(id).ok()
}

fn is_state_digest(note: &str) -> bool {
    note.strip_prefix(BRAIN_DELETION_DIGEST_PREFIX)
        .is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
}

fn server_identity(state: &ServerState) -> String {
    state.public_base_url.trim_end_matches('/').to_owned()
}

fn receipt_response(receipt: BrainDeletionReceipt) -> BrainDeletionReceiptResponse {
    BrainDeletionReceiptResponse {
        brain_id: receipt.brain_id.to_string(),
        deletion_event_id: receipt.deletion_event_id,
        actor_npub: receipt.actor_npub.to_string(),
        deleted_at: receipt.deleted_at,
        state_digest: receipt.state_digest,
        folder_count: receipt.folder_count,
        page_count: receipt.object_count,
    }
}
