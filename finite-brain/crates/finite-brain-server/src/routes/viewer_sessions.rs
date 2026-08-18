//! Viewer sessions for the brain:// live viewer (plan Phase 2).
//!
//! Two-tier model (owner ruling): a viewer session is a key-delivery
//! RECORD, not a grant. The entitlement is the requesting principal's
//! existing Folder access, re-checked on every read; the session row only
//! carries the NIP-44 wrapped Folder Key addressed to an ephemeral npub,
//! plus TTL and revocation key hygiene. Durable machine consumers (Sites
//! live views) will later arrive as real grant rows honored by the same
//! read route with zero route changes. The server stays blind: it never
//! sees a plaintext Folder Key.

use crate::*;

/// Default viewer session TTL (grill #1: 1h default).
pub(crate) const VIEWER_SESSION_DEFAULT_TTL_SECS: u64 = 3_600;
/// Maximum viewer session TTL (grill #1: 24h cap).
pub(crate) const VIEWER_SESSION_MAX_TTL_SECS: u64 = 86_400;
/// Initial-fetch record cap for the live viewer (grill #5).
pub(crate) const MAX_VIEWER_INITIAL_RECORDS: u64 = 2_000;
/// Initial-fetch ciphertext cap for the live viewer (grill #5).
pub(crate) const MAX_VIEWER_INITIAL_CIPHERTEXT_BYTES: u64 = 5 * 1024 * 1024;
/// Delta page cap for the encrypted-read route.
pub(crate) const MAX_VIEWER_RECORDS_LIMIT: u64 = 1_000;
/// Default delta page size.
const DEFAULT_VIEWER_RECORDS_LIMIT: u64 = 500;
/// Bound on the stored NIP-44 wrapped-key payload.
const MAX_VIEWER_WRAPPED_KEY_LEN: usize = 4_096;

fn viewer_session_response(session: StoredViewerSession, now: &str) -> ViewerSessionResponse {
    ViewerSessionResponse {
        id: session.id.clone(),
        brain_id: session.brain_id.to_string(),
        folder_id: session.folder_id.to_string(),
        ephemeral_npub: session.ephemeral_npub.to_string(),
        requester_npub: session.requester_npub.to_string(),
        key_version: session.key_version,
        status: session.status_at(now).as_str().to_owned(),
        wrapped_key_payload: session.wrapped_key_payload.clone(),
        completed_by_npub: session.completed_by_npub.map(|npub| npub.to_string()),
        created_at: session.created_at,
        expires_at: session.expires_at,
        revoked_at: session.revoked_at,
    }
}

/// POST /v1/viewer-session-requests — the requesting principal (NIP-98)
/// asks for the Folder Key wrapped to an ephemeral browser key. Any
/// principal holding Folder access may mint a session for themselves
/// (grill #2); the server records the request and marks the wrap a
/// key-holding client will complete.
pub(crate) async fn create_viewer_session_request_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Json<ViewerSessionResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: CreateViewerSessionRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(request.brain_id)?;
    let folder_id = FolderId::new(request.folder_id)?;
    let ephemeral_npub = parse_ephemeral_npub(&request.ephemeral_npub)?;
    let ttl = request
        .requested_ttl_secs
        .unwrap_or(VIEWER_SESSION_DEFAULT_TTL_SECS)
        .clamp(1, VIEWER_SESSION_MAX_TTL_SECS);

    let now_unix = state.auth_now_unix_seconds();
    let now = format_unix_timestamp(now_unix).unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned());
    let pending_deadline = format_unix_timestamp(now_unix + VIEWER_SESSION_MAX_TTL_SECS)
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned());

    let key_version = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        // Folder existence before visibility so legitimate requesters get
        // a precise 404; outsiders keep the generic 403.
        let key_version = folder_current_key_version(&stored, &folder_id)?;
        ensure_folder_visible(&stored, &folder_id, actor.as_str())?;
        key_version
    };
    let session = {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.create_viewer_session_request(ViewerSessionRequest {
            brain_id: brain_id.clone(),
            folder_id: folder_id.clone(),
            ephemeral_npub: ephemeral_npub.clone(),
            requester_npub: actor,
            key_version,
            requested_ttl_secs: ttl,
            now: now.clone(),
            pending_expires_at: pending_deadline,
        })?
    };
    Ok(Json(viewer_session_response(session, &now)))
}

/// GET /v1/viewer-session-requests/{request_id} — the browser polls this
/// with the ephemeral key (or the requester checks their own request) to
/// learn when the wrap lands. The response carries the wrapped-key
/// envelope only for those two principals; everyone else fails closed.
pub(crate) async fn get_viewer_session_request_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(request_id): AxumPath<String>,
) -> Result<Json<ViewerSessionResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state, &headers, &method, &uri, None,
    )?)?;
    let now = server_timestamp(&state);
    let session = {
        let store = state.store.lock().map_err(lock_error)?;
        let Some(session) = store.viewer_session(&request_id)? else {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "viewer session not found",
            ));
        };
        if actor != session.ephemeral_npub && actor != session.requester_npub {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "viewer session access required",
            ));
        }
        session
    };
    Ok(Json(viewer_session_response(session, &now)))
}

/// GET /v1/brains/{brain_id}/viewer-sessions — the admin access surface
/// behind `fbrain viewer-session list`.
pub(crate) async fn list_viewer_sessions_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<ViewerSessionListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let now = server_timestamp(&state);
    let sessions = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, actor.as_str())?;
        store.viewer_sessions_for_brain(&brain_id)?
    };
    Ok(Json(ViewerSessionListResponse {
        sessions: sessions
            .into_iter()
            .map(|session| viewer_session_response(session, &now))
            .collect(),
    }))
}

/// POST /v1/brains/{brain_id}/viewer-sessions/{session_id}/revoke — key
/// hygiene for Brain admins and session requesters. Revoking a session
/// never touches Folder access; revoking a person's viewing means
/// revoking the underlying Folder access.
pub(crate) async fn revoke_viewer_session_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, session_id)): AxumPath<(String, String)>,
) -> Result<Json<ViewerSessionResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state, &headers, &method, &uri, None,
    )?)?;
    let brain_id = BrainId::new(brain_id)?;
    let now = server_timestamp(&state);
    let session = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        let Some(existing) = store.viewer_session(&session_id)? else {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "viewer session not found",
            ));
        };
        if actor != existing.requester_npub {
            ensure_brain_admin(&stored, actor.as_str())?;
        }
        store.revoke_viewer_session(&brain_id, &session_id, &now)?
    };
    state.publish_access_update_for(&brain_id, session.ephemeral_npub.as_str());
    Ok(Json(viewer_session_response(session, &now)))
}

/// POST /v1/admin/brains/{brain_id}/folders/{folder_id}/viewer-session-wraps
/// — completion by a key-holding client (the agent daemon): one NIP-44
/// wrapped Folder Key per pending viewer-session marker. Mirrors the
/// grant-batch pending-wraps route's admin gating and fail-closed store
/// semantics, but lands in key-delivery records instead of grant rows.
pub(crate) async fn complete_pending_viewer_wraps_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    body: Bytes,
) -> Result<Json<CompleteViewerWrapsResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state,
        &headers,
        &method,
        &uri,
        Some(&body),
    )?)?;
    let request: CompleteViewerWrapsRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    if request.wraps.is_empty() || request.wraps.len() > BRAIN_CAPACITY_ENVELOPE.members {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "wraps must hold one entry per pending viewer session",
        ));
    }

    let now_unix = state.auth_now_unix_seconds();
    let mut prepared = Vec::with_capacity(request.wraps.len());
    {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, actor.as_str())?;
        let current = folder_current_key_version(&stored, &folder_id)?;
        for wrap in &request.wraps {
            let ephemeral_npub = parse_ephemeral_npub(&wrap.ephemeral_npub)?;
            if wrap.key_version != current {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "keyVersion does not match current Folder Key version",
                ));
            }
            validate_wrapped_key_payload(&wrap.wrapped_key_payload)?;
            prepared.push((
                ephemeral_npub,
                wrap.key_version,
                wrap.wrapped_key_payload.clone(),
            ));
        }
    }

    let mut completed_ephemerals = Vec::with_capacity(prepared.len());
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, actor.as_str())?;
        for (ephemeral_npub, key_version, payload) in &prepared {
            let Some(session) = store.viewer_session_for_recipient(
                &brain_id,
                &folder_id,
                ephemeral_npub,
                *key_version,
            )?
            else {
                return Err(StoreError::BrokenInvariant {
                    reason: "viewer session row missing for the marked recipient".to_owned(),
                }
                .into());
            };
            let ttl = session
                .requested_ttl_secs
                .clamp(1, VIEWER_SESSION_MAX_TTL_SECS);
            let expires_at = format_unix_timestamp(now_unix + ttl)
                .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned());
            store.complete_viewer_session(
                &brain_id,
                &folder_id,
                ViewerWrapCompletion {
                    ephemeral_npub: ephemeral_npub.clone(),
                    key_version: *key_version,
                    wrapped_key_payload: payload.clone(),
                    completed_by_npub: actor.clone(),
                    expires_at,
                },
            )?;
            completed_ephemerals.push(ephemeral_npub.to_string());
        }
    }
    for ephemeral in &completed_ephemerals {
        state.publish_access_update_for(&brain_id, ephemeral);
    }
    let completed_count = completed_ephemerals.len();
    Ok(Json(CompleteViewerWrapsResponse {
        brain_id: brain_id.to_string(),
        folder_id: folder_id.to_string(),
        outcome: if completed_count == 0 {
            "noPendingWraps"
        } else {
            "completed"
        }
        .to_owned(),
        completed_count,
        completed_ephemerals,
    }))
}

/// GET /v1/brains/{brain_id}/folders/{folder_id}/records — the
/// encrypted-read route. Auth tier (grill #7): an unexpired, unrevoked
/// viewer-session record whose requester's underlying Folder access still
/// holds. Machine grant rows will join this check later without route
/// changes. Returns only this Folder's object revisions and tombstones —
/// never control records — so no wrapped grants for other principals can
/// leak. The response is ciphertext the browser decrypts client-side.
pub(crate) async fn folder_view_records_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath((brain_id, folder_id)): AxumPath<(String, String)>,
    Query(query): Query<FolderViewRecordsQuery>,
) -> Result<Json<FolderViewRecordsResponse>, ApiError> {
    let actor = UserId::new(validate_request_auth(
        &state, &headers, &method, &uri, None,
    )?)?;
    let brain_id = BrainId::new(brain_id)?;
    let folder_id = FolderId::new(folder_id)?;
    let now = server_timestamp(&state);

    let session = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        let current = folder_current_key_version(&stored, &folder_id)?;
        // Session tier: an existing record distinguishes the honest
        // states; a stranger gets the generic failure.
        let Some(latest) = store.latest_viewer_session_for_actor(&brain_id, &folder_id, &actor)?
        else {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "viewer session required",
            ));
        };
        match latest.status_at(&now) {
            ViewerSessionStatus::Ready => {}
            ViewerSessionStatus::Revoked => {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "viewer session revoked",
                ));
            }
            ViewerSessionStatus::Expired | ViewerSessionStatus::Pending => {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "viewer session expired",
                ));
            }
        }
        // A rotation underneath the delivered wrap means the browser holds
        // a stale key; the honest state is expiry (re-request).
        if latest.key_version != current {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "viewer session expired",
            ));
        }
        // The route re-checks the requester's underlying Folder access on
        // every request: revoking their Folder access stops sessions
        // derived from it without touching session rows.
        ensure_folder_visible(&stored, &folder_id, latest.requester_npub.as_str())?;
        latest
    };

    let after = query.after.unwrap_or(0);
    let limit = if after == 0 {
        MAX_VIEWER_INITIAL_RECORDS
    } else {
        query
            .limit
            .unwrap_or(DEFAULT_VIEWER_RECORDS_LIMIT)
            .clamp(1, MAX_VIEWER_RECORDS_LIMIT)
    };
    if after == 0 {
        let (record_count, total_bytes) = {
            let store = state.store.lock().map_err(lock_error)?;
            store.folder_view_record_stats(&brain_id, &folder_id)?
        };
        if record_count > MAX_VIEWER_INITIAL_RECORDS
            || total_bytes > MAX_VIEWER_INITIAL_CIPHERTEXT_BYTES
        {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "folder too large for live view",
            ));
        }
    }
    let (records, latest_sequence) = {
        let store = state.store.lock().map_err(lock_error)?;
        store.pull_folder_view_records(&brain_id, &folder_id, after, limit)?
    };
    let has_more = records
        .last()
        .is_some_and(|record| record.sequence < latest_sequence);
    let count = records.len();
    let responses = records
        .into_iter()
        .map(|record| {
            let ciphertext = (record.record_type == SyncRecordType::FolderObjectRevision)
                .then(|| object_ciphertext(&record.payload_json));
            ViewerRecordResponse {
                sequence: record.sequence,
                record_type: match record.record_type {
                    SyncRecordType::FolderObjectRevision => "folder_object_revision",
                    SyncRecordType::FolderObjectTombstone => "folder_object_tombstone",
                    SyncRecordType::FolderKeyGrant | SyncRecordType::BrainAdminAccessChange => {
                        unreachable!("folder view pull filters to object records")
                    }
                }
                .to_owned(),
                object_id: record
                    .object_id
                    .map(|object_id| object_id.as_str().to_owned()),
                revision: record.revision,
                ciphertext,
            }
        })
        .collect::<Vec<_>>();
    Ok(Json(FolderViewRecordsResponse {
        brain_id: brain_id.to_string(),
        folder_id: folder_id.to_string(),
        after_sequence: after,
        latest_sequence,
        records: responses,
        count,
        has_more,
        session_expires_at: session.expires_at,
    }))
}

/// The ephemeral npub must be a real bech32 nostr public key; anything
/// else fails closed before touching the store.
fn parse_ephemeral_npub(value: &str) -> Result<UserId, ApiError> {
    NostrPublicKey::parse(value).map(|_| ()).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "ephemeralNpub must be a valid npub",
        )
    })?;
    UserId::new(value.to_owned()).map_err(ApiError::from)
}

/// The wrapped-key payload is opaque to the server; validate only shape
/// (non-empty base64 of bounded length) so garbage cannot be stored.
fn validate_wrapped_key_payload(payload: &str) -> Result<(), ApiError> {
    if payload.is_empty() || payload.len() > MAX_VIEWER_WRAPPED_KEY_LEN {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "wrappedKeyPayload must be non-empty base64 of bounded length",
        ));
    }
    if !payload
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/' || byte == b'=')
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "wrappedKeyPayload must be non-empty base64 of bounded length",
        ));
    }
    Ok(())
}
