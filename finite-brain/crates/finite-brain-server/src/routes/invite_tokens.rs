use crate::*;

/// Raw Invite Tokens carry this prefix so the capability is self-identifying
/// in URLs, emails, and logs. The hash covers the full prefixed token.
const INVITE_TOKEN_PREFIX: &str = "fbit-";
const INVITE_TOKEN_DEFAULT_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
const MAX_INVITE_TOKEN_LEN: usize = 160;

/// Create one capability Invite Token. The raw token is returned exactly
/// once; only its SHA-256 hash is stored. When `email` is present the link is
/// also sent through the configured invite mailer — delivery only, never
/// identity (auth kernel rules 2 and 3).
pub(crate) async fn create_brain_invite_token_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<CreateBrainInviteTokenResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: CreateBrainInviteTokenRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let role = BrainInviteTokenRole::try_from(request.role.as_str())?;
    let email = request.email.as_deref().map(canonical_email).transpose()?;
    let brain_id = BrainId::new(brain_id)?;
    let actor_user_id = UserId::new(actor.clone())?;
    let now_unix = state.auth_now_unix_seconds();
    let created_at = server_timestamp(&state);
    let expires_at = match request.expires_at.as_deref() {
        Some(expires_at) => expires_at.trim().to_owned(),
        None => {
            format_unix_timestamp(now_unix + INVITE_TOKEN_DEFAULT_TTL_SECONDS).ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invite token expiry is out of range",
                )
            })?
        }
    };

    let token = format!("{INVITE_TOKEN_PREFIX}{}", generate_capability_token());
    let token_hash = sha256_hex(&token);
    let (stored, created) = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        let created = store.create_brain_invite_token(
            &brain_id,
            &token_hash,
            role,
            &actor_user_id,
            &expires_at,
            &created_at,
        )?;
        (stored, created)
    };

    let url = invite_token_url(&state, &token);
    let delivery_status = match email.as_deref() {
        Some(email) => deliver_invite_token_email(&state, &stored, &actor, email, &url),
        None => "manual".to_owned(),
    };

    Ok(Json(CreateBrainInviteTokenResponse {
        token_id: created.token_hash,
        token,
        url,
        brain_id: brain_id.to_string(),
        role: role.as_str().to_owned(),
        expires_at: created.expires_at,
        created_at: created.created_at,
        delivery_status,
    }))
}

/// List Invite Tokens for one Brain, newest first. Never carries raw tokens.
pub(crate) async fn list_brain_invite_tokens_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
) -> Result<Json<BrainInviteTokenListResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let brain_id = BrainId::new(brain_id)?;
    let now = server_timestamp(&state);
    let invite_tokens = {
        let store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        store
            .list_brain_invite_tokens(&brain_id)?
            .into_iter()
            .map(|token| invite_token_response(token, &now))
            .collect::<Vec<_>>()
    };
    Ok(Json(BrainInviteTokenListResponse { invite_tokens }))
}

/// Revoke a pending Invite Token. Redeemed membership is unchanged.
pub(crate) async fn revoke_brain_invite_token_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<BrainInviteTokenResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: RevokeBrainInviteTokenRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let brain_id = BrainId::new(brain_id)?;
    let actor_user_id = UserId::new(actor.clone())?;
    let now = server_timestamp(&state);
    let token = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        store.revoke_brain_invite_token(&brain_id, request.token_id.trim(), &actor_user_id, &now)?
    };
    Ok(Json(invite_token_response(token, &now)))
}

/// Redeem one capability Invite Token: the signer's npub gains Brain
/// Membership (plus Brain Admin standing for an admin-role token) with
/// inviter provenance, and pending-wrap markers ask key-holding clients to
/// deliver the Folder Key Grants — the same claim mechanics as npub-target
/// invitation accept. Single-use, expiry, and revocation fail closed; a
/// same-npub re-present is idempotent.
pub(crate) async fn redeem_brain_invite_token_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Json<RedeemBrainInviteTokenResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: RedeemBrainInviteTokenRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let actor_user_id = UserId::new(actor)?;
    let token = normalize_invite_token(&request.token)?;
    let token_hash = sha256_hex(&token);
    let now = server_timestamp(&state);
    let (redeemed, brain_display_name) = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let redeemed = store.redeem_brain_invite_token(&token_hash, &actor_user_id, &now)?;
        let brain_display_name = store.load_brain(&redeemed.brain_id)?.brain.name.to_string();
        (redeemed, brain_display_name)
    };
    if !redeemed.duplicate_redeem {
        state.publish_access_update_for(&redeemed.brain_id, actor_user_id.as_str());
    }
    Ok(Json(RedeemBrainInviteTokenResponse {
        token_id: redeemed.token_hash,
        brain_id: redeemed.brain_id.to_string(),
        brain_display_name,
        role: redeemed.role.as_str().to_owned(),
        inviter_npub: redeemed.inviter_npub.to_string(),
        redeemed_by_npub: actor_user_id.to_string(),
        redeemed_at: redeemed.redeemed_at,
        duplicate_redeem: redeemed.duplicate_redeem,
    }))
}

fn invite_token_url(state: &ServerState, token: &str) -> String {
    format!(
        "{}/v1/invite-tokens/redeem#{token}",
        state.public_base_url.trim_end_matches('/')
    )
}

fn invite_token_response(token: StoredBrainInviteToken, now: &str) -> BrainInviteTokenResponse {
    let status = invite_token_status(&token, now).to_owned();
    BrainInviteTokenResponse {
        token_id: token.token_hash,
        brain_id: token.brain_id.to_string(),
        role: token.role.as_str().to_owned(),
        inviter_npub: token.inviter_npub.to_string(),
        status,
        created_at: token.created_at,
        expires_at: token.expires_at,
        redeemed_by_npub: token.redeemed_by_npub.map(|npub| npub.to_string()),
        redeemed_at: token.redeemed_at,
        revoked_at: token.revoked_at,
    }
}

fn invite_token_status(token: &StoredBrainInviteToken, now: &str) -> &'static str {
    if token.redeemed_by_npub.is_some() {
        "redeemed"
    } else if token.revoked_at.is_some() {
        "revoked"
    } else if timestamp_expired(&token.expires_at, now) {
        "expired"
    } else {
        "pending"
    }
}

/// Validate the raw token shape before hashing: the canonical prefix plus at
/// least 24 url-safe characters of token body.
fn normalize_invite_token(raw: &str) -> Result<String, ApiError> {
    let token = raw.trim();
    let Some(body) = token.strip_prefix(INVITE_TOKEN_PREFIX) else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invite token must start with {INVITE_TOKEN_PREFIX}"),
        ));
    };
    let valid = token.len() <= MAX_INVITE_TOKEN_LEN
        && body.len() >= 24
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invite token must be at least 24 url-safe characters",
        ));
    }
    Ok(token.to_owned())
}

/// Email copy serves two audiences without confusing either: one short human
/// paragraph, then one explicit agent instruction line.
fn deliver_invite_token_email(
    state: &ServerState,
    stored: &StoredBrain,
    inviter_npub: &str,
    email: &str,
    url: &str,
) -> String {
    let Some(mailer) = state.invite_mailer.as_ref() else {
        return "not_configured".to_owned();
    };
    let payload = BrainInviteEmail {
        to: email.to_owned(),
        subject: format!("Finite Brain invitation: {}", stored.brain.name),
        text: format!(
            "{inviter_npub} invited you to the Finite brain \"{brain}\". \
             Open this link with your agent, or use the CLI command below.\n\n\
             {url}\n\n\
             Agent instruction: fbrain invite-accept {url}\n\n\
             The link is a single-use capability: the first key to redeem it joins the \
             brain. If you were not expecting this invitation, ignore this email; the \
             inviter can revoke the link at any time.",
            brain = stored.brain.name,
        ),
    };
    match mailer(&payload) {
        Ok(()) => "sent".to_owned(),
        // The token is already durable; a delivery error is reported, not
        // fatal, so a retry does not mint a second capability.
        Err(_) => "failed".to_owned(),
    }
}
