//! Folder Key rotation routes: explicit access removal and standalone rotation.
//!
//! Both routes accept the same client-prepared material and commit through
//! `BrainStore::rotate_folder_key`; the signed admin access-change event binds
//! the action, Folder, optional target and new key version.

use crate::*;
use finite_brain_store::{FolderKeyRotationRequest, FolderKeyRotationTarget};

/// Rotation requests that may hold a large body at once. With the 64 MiB
/// rotation body limit this bounds buffered rotation request bodies at
/// 256 MiB (parsing and cryptography allocate on top of that).
pub(crate) const MAX_CONCURRENT_ROTATION_REQUESTS: usize = 4;

/// How long an admitted rotation request may take to deliver its body before
/// it is answered 408 and its permit is released.
pub(crate) const ROTATION_BODY_READ_DEADLINE: std::time::Duration =
    std::time::Duration::from_secs(finite_brain_core::ROTATION_BODY_READ_DEADLINE_SECONDS);

/// How long an authorized rotation request may wait, in arrival order, for a
/// permit before it is answered 503. Two body-read deadlines: a request
/// queued behind four stalled uploads gets the next released permit within
/// one deadline, ahead of any upload that arrives after it.
pub(crate) const ROTATION_ADMISSION_WAIT: std::time::Duration =
    std::time::Duration::from_secs(finite_brain_core::ROTATION_ADMISSION_WAIT_SECONDS);

/// Authorized rotation requests that may wait for a permit at once. Waiters
/// hold only their parsed header, never a body; beyond this they get 503.
pub(crate) const MAX_WAITING_ROTATION_REQUESTS: usize = 64;

/// Membership in the bounded set of requests waiting for a rotation permit;
/// leaving it (permit taken, wait expired or client gone) frees the slot.
struct WaitingForRotationPermit(Arc<std::sync::atomic::AtomicUsize>);

impl WaitingForRotationPermit {
    fn enter(waiting: &Arc<std::sync::atomic::AtomicUsize>) -> Result<Self, ApiError> {
        use std::sync::atomic::Ordering;
        if waiting.fetch_add(1, Ordering::AcqRel) >= MAX_WAITING_ROTATION_REQUESTS {
            waiting.fetch_sub(1, Ordering::AcqRel);
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "too many Folder Key rotation requests are waiting for admission; retry later",
            ));
        }
        Ok(Self(waiting.clone()))
    }
}

impl Drop for WaitingForRotationPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// The auth clock instant at which admission validated a rotation request's
/// signed header. Handlers validate the full request (with its body) against
/// this instant, so a request that was fresh when admitted stays valid while
/// it waits in the admission queue. Set only by admission, never by clients.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct RotationAdmittedAt(pub(crate) u64);

/// Who may occupy a rotation permit on a route. Keys are free, roles are not:
/// only a caller with this authority can hold one of the permits.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum RotationAuthority {
    /// An admin of the `{brain_id}` path Brain (owner or Personal Agent of a
    /// Personal Brain), exactly as `ensure_brain_admin` decides.
    BrainAdmin,
    /// Source or destination control of the `{connection_id}` Mount.
    MountRevocation,
    /// Destination control of the active `{connection_id}` Mount.
    MountParticipantRemoval,
}

#[derive(Clone)]
struct RotationAdmission {
    state: ServerState,
    authority: RotationAuthority,
}

/// Wrap the rotating method of a route (and only that method, so siblings on
/// the same path keep the ordinary 1 MiB cap). Before any body byte is read
/// the request must carry a valid signed header from a key with `authority`
/// over the path (403 otherwise); then it waits in arrival order, at most
/// [`ROTATION_ADMISSION_WAIT`] and among at most
/// [`MAX_WAITING_ROTATION_REQUESTS`] waiters (503 otherwise), for one of
/// [`MAX_CONCURRENT_ROTATION_REQUESTS`] permits, and must then deliver its
/// body within [`ROTATION_BODY_READ_DEADLINE`] (408) and
/// `MAX_FOLDER_ROTATION_REQUEST_BODY_BYTES` (413). The permit is held until
/// the handler has answered.
pub(crate) fn rotation_method(
    state: &ServerState,
    authority: RotationAuthority,
    method: axum::routing::MethodRouter<ServerState>,
) -> axum::routing::MethodRouter<ServerState> {
    method
        .layer(DefaultBodyLimit::max(
            finite_brain_core::MAX_FOLDER_ROTATION_REQUEST_BODY_BYTES,
        ))
        .layer(middleware::from_fn_with_state(
            RotationAdmission {
                state: state.clone(),
                authority,
            },
            admit_rotation_request,
        ))
}

async fn admit_rotation_request(
    State(admission): State<RotationAdmission>,
    AxumPath(params): AxumPath<BTreeMap<String, String>>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    match admit(admission, &params, request, next).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn admit(
    admission: RotationAdmission,
    params: &BTreeMap<String, String>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Result<Response, ApiError> {
    let state = &admission.state;
    // The handler repeats full authentication with the body hash (and the
    // replay cache) and the store repeats authority inside its transaction;
    // this pass only decides who may hold a permit while the body arrives.
    // Nested routers strip their prefix from `uri()`; the signed URL is the
    // original one, exactly as the handlers' `OriginalUri` sees it.
    let uri = request
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| request.uri().clone(), |original| original.0.clone());
    let admitted_at = state.auth_now_unix_seconds();
    let actor = authenticate_request_header_at(
        state,
        request.headers(),
        request.method(),
        &uri,
        admitted_at,
    )?;
    authorize_rotation_actor(state, admission.authority, params, &actor)?;
    refuse_rate_limited_signer(state, request.method(), &uri, &actor)?;
    let method = request.method().clone();
    // A request that leaves admission without reaching its handler is
    // charged to the signer's rate limit, so reconnecting is throttled.
    let charge_exit = |error: ApiError| {
        charge_rotation_admission_exit(state, &method, &uri, &actor);
        error
    };
    // Wait in arrival order (the semaphore is fair), so a stream of stalled
    // uploads that each time out and reconnect queues behind this request
    // instead of winning every refill.
    let _permit = {
        let _waiting = WaitingForRotationPermit::enter(&state.rotation_waiting_requests)
            .map_err(charge_exit)?;
        match tokio::time::timeout(
            state.rotation_admission_wait,
            state.rotation_request_permits.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) | Err(_) => {
                return Err(charge_exit(ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the server stayed busy with Folder Key rotation requests for the whole admission wait; retry later",
                )));
            }
        }
    };
    let (mut parts, body) = request.into_parts();
    parts.extensions.insert(RotationAdmittedAt(admitted_at));
    let body = match tokio::time::timeout(
        state.rotation_body_read_deadline,
        axum::body::to_bytes(
            body,
            finite_brain_core::MAX_FOLDER_ROTATION_REQUEST_BODY_BYTES,
        ),
    )
    .await
    {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "rotation request body exceeds the rotation request limit or was not fully received",
            ));
        }
        Err(_) => {
            return Err(charge_exit(ApiError::new(
                StatusCode::REQUEST_TIMEOUT,
                "rotation request body was not received before the deadline",
            )));
        }
    };
    Ok(next
        .run(axum::extract::Request::from_parts(
            parts,
            axum::body::Body::from(body),
        ))
        .await)
}

fn authorize_rotation_actor(
    state: &ServerState,
    authority: RotationAuthority,
    params: &BTreeMap<String, String>,
    actor: &str,
) -> Result<(), ApiError> {
    let param = |name: &str| {
        params.get(name).cloned().ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "rotation route parameter is missing",
            )
        })
    };
    let refuse = |error: StoreError| match error {
        StoreError::BrokenInvariant { reason } => ApiError::new(StatusCode::FORBIDDEN, reason),
        other => ApiError::from(other),
    };
    let store = state.store.lock().map_err(lock_error)?;
    match authority {
        RotationAuthority::BrainAdmin => {
            let stored = store.load_brain(&BrainId::new(param("brain_id")?)?)?;
            ensure_brain_admin(&stored, actor)
        }
        RotationAuthority::MountRevocation => store
            .authorize_mount_revocation(&param("connection_id")?, &UserId::new(actor)?)
            .map(|_| ())
            .map_err(refuse),
        RotationAuthority::MountParticipantRemoval => store
            .authorize_mount_participant_removal(&param("connection_id")?, &UserId::new(actor)?)
            .map(|_| ())
            .map_err(refuse),
    }
}

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
    Extension(admitted_at): Extension<RotationAdmittedAt>,
    body: Bytes,
) -> Result<Json<BrainMetadataResponse>, ApiError> {
    let actor =
        validate_admitted_rotation_auth(&state, &headers, &method, &uri, &body, admitted_at)?;
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
    Extension(admitted_at): Extension<RotationAdmittedAt>,
    body: Bytes,
) -> Result<Json<RotateFolderKeyResponse>, ApiError> {
    let actor =
        validate_admitted_rotation_auth(&state, &headers, &method, &uri, &body, admitted_at)?;
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
