use crate::*;
use finite_brain_store::{ApprovalRequestStatus, StoredBrainApprovalRequest};

/// How long a pending Approval request (and its nonce) stays signable.
const APPROVAL_REQUEST_TTL_SECONDS: u64 = 15 * 60;

/// The exact unsigned action payload stored on one Approval request; the
/// human's hosted key holder adds `humanNpub` and signs it (ADR-0046).
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnsignedBrainApprovalPayload {
    pub version: String,
    pub action: String,
    pub brain_id: String,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub target_npubs: Vec<String>,
    pub nonce: String,
    pub expires_at: u64,
}

pub(crate) fn approval_request_response(
    request: &StoredBrainApprovalRequest,
) -> Result<ApprovalRequestResponse, ApiError> {
    let payload: serde_json::Value = serde_json::from_str(&request.payload_json).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "stored approval request payload is corrupt",
        )
    })?;
    Ok(ApprovalRequestResponse {
        id: request.id.clone(),
        brain_id: request.brain_id.to_string(),
        action: request.action.clone(),
        payload,
        nonce: request.nonce.clone(),
        expires_at: request.expires_at_unix,
        requested_by_npub: request.requested_by_npub.to_string(),
        status: request.status.as_str().to_owned(),
        approval_event_id: request.approval_event_id.clone(),
        resolved_by_npub: request.resolved_by_npub.as_ref().map(ToString::to_string),
        created_at: request.created_at.clone(),
        resulting_invitations: request.result_invitations.clone(),
        updated_at: request.updated_at.clone(),
    })
}

/// Attach pending Approval cards for admins reading Brain metadata.
pub(crate) fn attach_pending_approvals(
    store: &BrainStore,
    response: &mut BrainMetadataResponse,
    brain_id: &BrainId,
) -> Result<(), ApiError> {
    let requests = store.list_brain_approval_requests(brain_id)?;
    response.pending_approvals = requests
        .iter()
        .filter(|request| request.status == ApprovalRequestStatus::Pending)
        .map(approval_request_response)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(())
}

/// The requester must belong to the Brain: member, owner, or Personal Agent.
/// Guests and outsiders cannot file Approval requests.
fn ensure_approval_requester(stored: &StoredBrain, actor_npub: &str) -> Result<(), ApiError> {
    let is_member = stored
        .brain
        .members
        .iter()
        .any(|member| member.user_id.as_str() == actor_npub);
    let is_owner = stored
        .brain
        .owner_user_id
        .as_ref()
        .is_some_and(|owner| owner.as_str() == actor_npub);
    let is_personal_agent = stored
        .personal_agent
        .as_ref()
        .is_some_and(|relationship| relationship.agent_npub.as_str() == actor_npub);
    if is_member || is_owner || is_personal_agent {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "brain membership is required to request an approval",
        ))
    }
}

/// The Approval signer must hold Brain admin standing: the Personal Brain
/// owner or an Organization Brain admin. Personal Agents and ordinary members
/// cannot produce Tier-3/4 signatures.
fn ensure_approval_signer_standing(
    stored: &StoredBrain,
    signer_npub: &str,
) -> Result<(), ApiError> {
    let is_owner = stored.brain.kind == BrainKind::Personal
        && stored
            .brain
            .owner_user_id
            .as_ref()
            .is_some_and(|owner| owner.as_str() == signer_npub);
    let is_admin = stored
        .brain
        .admins
        .iter()
        .any(|admin| admin.as_str() == signer_npub);
    if is_owner || is_admin {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "approval signer does not hold brain admin standing",
        ))
    }
}

/// Validate the requested action shape. Returns the normalized target npubs.
/// Validation is purely local (auth kernel): no authority calls, no account
/// bindings — just the stored Brain shape.
fn validate_approval_request_action(
    stored: &StoredBrain,
    action: &str,
    plan_id: Option<&str>,
    target_npubs: &[String],
) -> Result<Vec<String>, ApiError> {
    match action {
        finite_brain_core::BRAIN_APPROVAL_ACTION_DELEGATION_GRANT => {
            if plan_id.is_some() {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "delegation-grant approval requests cannot name an invitation plan",
                ));
            }
            if stored.brain.kind != BrainKind::Organization {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "delegation grants require an organization brain",
                ));
            }
            if target_npubs.is_empty()
                || target_npubs.len() > finite_brain_core::MAX_BRAIN_APPROVAL_TARGETS
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "delegation-grant approval requests require 1-100 target Principals",
                ));
            }
            let mut seen = BTreeSet::new();
            let mut normalized = Vec::with_capacity(target_npubs.len());
            for target in target_npubs {
                let npub = NostrPublicKey::parse(target)
                    .and_then(|public_key| public_key.to_npub())
                    .map_err(|error| {
                        ApiError::new(
                            StatusCode::BAD_REQUEST,
                            format!("invalid approval target npub: {error}"),
                        )
                    })?;
                if !seen.insert(npub.clone()) {
                    return Err(ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "delegation-grant approval requests cannot repeat a target Principal",
                    ));
                }
                if stored
                    .brain
                    .admins
                    .iter()
                    .any(|admin| admin.as_str() == npub)
                {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "delegation target is already a brain admin",
                    ));
                }
                normalized.push(npub);
            }
            Ok(normalized)
        }
        _ => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported approval action",
        )),
    }
}

pub(crate) async fn create_approval_request_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<ApprovalRequestResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: ApprovalRequestCreateRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let actor_user_id = UserId::new(actor)?;
    let target_npubs = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_approval_requester(&stored, actor_user_id.as_str())?;
        validate_approval_request_action(
            &stored,
            &request.action,
            request.plan_id.as_deref(),
            &request.target_npubs,
        )?
    };

    let created_at = server_timestamp(&state);
    let expires_at_unix = state.auth_now_unix_seconds() + APPROVAL_REQUEST_TTL_SECONDS;
    // Nonces are deterministic over unique request parts; on the vanishingly
    // rare collision the UNIQUE constraint answers 409 and the caller retries.
    let mut last_error = None;
    for attempt in 0..3_u8 {
        let attempt = attempt.to_string();
        let nonce = generated_link_id(
            "nonce",
            &[
                brain_id.as_str(),
                request.action.as_str(),
                actor_user_id.as_str(),
                created_at.as_str(),
                attempt.as_str(),
            ],
            16,
        )
        .replace("nonce-", "");
        let id = generated_link_id(
            "approval",
            &[
                brain_id.as_str(),
                request.action.as_str(),
                actor_user_id.as_str(),
                nonce.as_str(),
                created_at.as_str(),
            ],
            16,
        );
        let payload = UnsignedBrainApprovalPayload {
            version: finite_brain_core::BRAIN_APPROVAL_VERSION.to_owned(),
            action: request.action.clone(),
            brain_id: brain_id.to_string(),
            plan_id: request.plan_id.clone(),
            target_npubs: target_npubs.clone(),
            nonce: nonce.clone(),
            expires_at: expires_at_unix,
        };
        let payload_json = serde_json::to_string(&payload).map_err(|_| {
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
        })?;
        let stored_request = StoredBrainApprovalRequest {
            id,
            brain_id: brain_id.clone(),
            action: request.action.clone(),
            payload_json,
            nonce,
            expires_at_unix,
            requested_by_npub: actor_user_id.clone(),
            status: ApprovalRequestStatus::Pending,
            approval_event_id: None,
            resolved_by_npub: None,
            created_at: created_at.clone(),
            updated_at: created_at.clone(),
            result_invitations: None,
        };
        let created = {
            let mut store = state.store.lock().map_err(lock_error)?;
            store.create_brain_approval_request(&stored_request)
        };
        match created {
            Ok(created) => return Ok(Json(approval_request_response(&created)?)),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.map(ApiError::from).unwrap_or_else(|| {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }))
}

pub(crate) async fn list_approval_requests_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<ApprovalRequestListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let requests = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        let admin = ensure_brain_admin(&stored, &actor).is_ok();
        if !admin {
            ensure_approval_requester(&stored, &actor)?;
        }
        store
            .list_brain_approval_requests(&brain_id)?
            .into_iter()
            .filter(|request| admin || request.requested_by_npub.as_str() == actor)
            .collect::<Vec<_>>()
    };
    let requests = requests
        .iter()
        .map(approval_request_response)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ApprovalRequestListResponse { requests }))
}

pub(crate) async fn deny_approval_request_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, request_id)): AxumPath<(String, String)>,
) -> Result<Json<ApprovalRequestResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let actor_user_id = UserId::new(actor)?;
    let updated_at = server_timestamp(&state);
    let resolved = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        let request = store.load_brain_approval_request(&request_id)?;
        if request.brain_id != brain_id {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "approval request not found",
            ));
        }
        let is_requester = request.requested_by_npub == actor_user_id;
        if !is_requester {
            ensure_brain_admin(&stored, actor_user_id.as_str())?;
        }
        store.resolve_brain_approval_request(
            &request_id,
            ApprovalRequestStatus::Denied,
            None,
            &actor_user_id,
            &updated_at,
        )?
    };
    Ok(Json(approval_request_response(&resolved)?))
}

pub(crate) async fn submit_approval_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    // Any authenticated Principal may relay a signed approval; the artifact
    // itself is the authority, and every check below fails closed.
    let _relayer = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: ApprovalSubmissionRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let event = Event::from_json(&request.approval_event_json).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "approval artifact event JSON did not parse",
        )
    })?;
    verify_event_integrity(&event).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "approval artifact signature is invalid",
        )
    })?;
    let signer_npub = NostrPublicKey::from_protocol(event.pubkey)
        .to_npub()
        .map_err(|error| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("approval signer npub is invalid: {error}"),
            )
        })?;
    let payload = finite_brain_core::validate_brain_approval_event(&event, &signer_npub)
        .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, error.to_string()))?;
    if payload.brain_id != brain_id.as_str() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "approval artifact is scoped to a different brain",
        ));
    }
    if payload.expires_at <= state.auth_now_unix_seconds() {
        return Err(ApiError::new(
            StatusCode::GONE,
            "approval artifact has expired",
        ));
    }

    // Bind the artifact to its pending request when one is named.
    let approval_request = if let Some(request_id) = request.request_id.as_deref() {
        let request = {
            let store = state.store.lock().map_err(lock_error)?;
            store.load_brain_approval_request(request_id)?
        };
        if request.brain_id != brain_id {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "approval request not found",
            ));
        }
        if request.status != ApprovalRequestStatus::Pending {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "approval request is already resolved",
            ));
        }
        if request.expires_at_unix <= state.auth_now_unix_seconds() {
            return Err(ApiError::new(
                StatusCode::GONE,
                "approval request has expired",
            ));
        }
        let stored_payload: UnsignedBrainApprovalPayload =
            serde_json::from_str(&request.payload_json).map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "stored approval request payload is corrupt",
                )
            })?;
        if stored_payload.action != payload.action
            || stored_payload.plan_id != payload.plan_id
            || stored_payload.target_npubs != payload.target_npubs
            || stored_payload.nonce != payload.nonce
            || stored_payload.expires_at != payload.expires_at
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "approval artifact does not match the pending request",
            ));
        }
        Some(request)
    } else {
        None
    };

    let signer_user_id = UserId::new(signer_npub.clone())?;
    let approval_event_id = event.id.to_hex();
    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_approval_signer_standing(&stored, &signer_npub)?;
    }

    match payload.action.as_str() {
        finite_brain_core::BRAIN_APPROVAL_ACTION_DELEGATION_GRANT => {
            let applied_at = server_timestamp(&state);
            let targets = payload.target_npubs.clone();
            {
                let mut store = state.store.lock().map_err(lock_error)?;
                let stored = store.load_brain(&brain_id)?;
                if stored.brain.kind != BrainKind::Organization {
                    return Err(ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "delegation grants require an organization brain",
                    ));
                }
                if store.approval_nonce_seen(&brain_id, &payload.nonce)? {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "approval nonce was already applied",
                    ));
                }
                for target in &targets {
                    let target_id = UserId::new(target.clone())?;
                    let provenance = finite_brain_store::MemberProvenance::approval(
                        signer_user_id.clone(),
                        approval_event_id.clone(),
                    );
                    store.grant_admin_with_provenance(&brain_id, &target_id, &provenance)?;
                }
                store.record_brain_approval_nonce(
                    &brain_id,
                    &payload.nonce,
                    &approval_event_id,
                    &signer_user_id,
                    finite_brain_core::BRAIN_APPROVAL_ACTION_DELEGATION_GRANT,
                    &applied_at,
                )?;
                if let Some(request) = approval_request.as_ref() {
                    store.resolve_brain_approval_request(
                        &request.id,
                        ApprovalRequestStatus::Approved,
                        Some(approval_event_id.as_str()),
                        &signer_user_id,
                        &applied_at,
                    )?;
                }
            }
            for target in &targets {
                state.publish_access_update_for(&brain_id, target);
            }
            Ok(Json(ApprovalSubmissionResponse {
                status: "applied".to_owned(),
                action: payload.action.clone(),
                approval_event_id,
                request_id: approval_request.map(|request| request.id),
                result: serde_json::json!({ "grantedNpubs": targets }),
            })
            .into_response())
        }
        _ => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported approval action",
        )),
    }
}
