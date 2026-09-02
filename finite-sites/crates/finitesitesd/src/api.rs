//! Control-plane API. Every mutation is authenticated with NIP-98 against
//! the exact URL and method received, and bodies are bound by payload hash.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, OriginalUri, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};

use finitesites_engine::EngineError;
use finitesites_proto::dto::{
    ApiErrorBody, AuthRegisterResponse, ERROR_GIT_REPOSITORY_SETUP_FAILED, ERROR_GIT_UNAVAILABLE,
    GitAuthRequest, GitAuthResponse, HostedRequesterAssertionRequest,
    HostedRequesterAssertionResponse, ProjectGrantRequest, ProjectGrantResponse,
    ProjectInitRequest, ProjectInitResponse, ProjectListResponse, ProjectOutputSharingResponse,
    ProjectRevokeRequest, ProjectRevokeResponse, ProjectStatusResponse, SharingRequest,
    SitesAuthorizedKeyRegisterRequest, SitesAuthorizedKeyResponse, SitesAuthorizedKeyRevokeRequest,
};
use finitesites_proto::limits::{
    MAX_API_BODY_BYTES, MAX_AUTH_HEADER_BYTES, MAX_VIEWER_RETURN_TO_BYTES,
};

use crate::mailer::ProjectCollaboratorInvite;
use crate::server::{AppState, now_unix};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/healthz", get(healthz))
        .route("/api/v1/auth/register", post(register_auth))
        .route("/api/v1/email-auth/request", post(request_email_login))
        .route("/api/v1/email-auth/redeem", post(redeem_email_login))
        .route(
            "/api/v1/sites-authorized-keys/register",
            post(register_sites_authorized_key),
        )
        .route(
            "/api/v1/sites-authorized-keys/revoke",
            post(revoke_sites_authorized_key),
        )
        .route("/api/v1/projects", get(list_projects))
        .route("/api/v1/projects/init", post(init_project))
        .route("/api/v1/projects/{slug}", get(project_status))
        .route("/api/v1/projects/{slug}/grant", post(grant_project))
        .route("/api/v1/projects/{slug}/revoke", post(revoke_project))
        .route("/api/v1/projects/{slug}/git-auth", post(auth_git))
        .route(
            "/api/v1/projects/{slug}/outputs/{output_id}/sharing",
            post(share_project_output),
        )
        .route(
            "/internal/v1/hosted-requester-assertions",
            post(create_hosted_requester_assertion),
        )
        .layer(DefaultBodyLimit::max(MAX_API_BODY_BYTES as usize))
        .fallback(api_not_found)
        .with_state(state)
}

// ---- error mapping -----------------------------------------------------------

pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            code,
            message: message.into(),
        }
    }

    fn unauthorized(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    fn bad_request(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    fn unavailable(message: impl Into<String>) -> ApiError {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            message,
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorBody {
            error: self.code.to_string(),
            message: self.message,
        };
        (self.status, Json(body)).into_response()
    }
}

impl From<EngineError> for ApiError {
    fn from(error: EngineError) -> ApiError {
        let message = error.to_string();
        match error {
            EngineError::NotAllowlisted => {
                ApiError::new(StatusCode::FORBIDDEN, "not_allowlisted", message)
            }
            EngineError::NotAuthorized => {
                ApiError::new(StatusCode::FORBIDDEN, "not_authorized", message)
            }
            EngineError::RequesterEmailRequired => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "requester_email_required",
                message,
            ),
            EngineError::NameTaken => ApiError::new(StatusCode::CONFLICT, "name_taken", message),
            EngineError::SiteNotFound
            | EngineError::ProjectNotFound
            | EngineError::OutputNotFound => {
                ApiError::new(StatusCode::NOT_FOUND, "not_found", message)
            }
            EngineError::TooManySites
            | EngineError::TooManyShares
            | EngineError::TooManyEmailKeys
            | EngineError::TooManyProjectCollaborators => {
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "limit_exceeded", message)
            }
            EngineError::Validation(_) | EngineError::Proto(_) => {
                ApiError::new(StatusCode::BAD_REQUEST, "validation_failed", message)
            }
            EngineError::Conflict(_) => ApiError::new(StatusCode::CONFLICT, "conflict", message),
            EngineError::Blob(inner) => match inner {
                finitesites_blob::BlobError::TooLarge { .. }
                | finitesites_blob::BlobError::HashMismatch { .. } => {
                    ApiError::new(StatusCode::BAD_REQUEST, "validation_failed", message)
                }
                _ => internal_error("blob storage failure"),
            },
            EngineError::Store(_) => internal_error("registry failure"),
        }
    }
}

fn internal_error(message: &'static str) -> ApiError {
    // Internal details go to the operator log, not the wire.
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
}

// ---- auth helper ----------------------------------------------------------------

/// Verify the NIP-98 Authorization header against the request actually
/// received and return the signer's pubkey hex. The freshness window comes
/// from the shared `finite-authn` policy table.
fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    method: &str,
    original_uri: &OriginalUri,
    body: Option<&[u8]>,
) -> Result<String, ApiError> {
    let header_value = headers
        .get(header::AUTHORIZATION)
        .ok_or_else(|| ApiError::unauthorized("missing Authorization header"))?
        .to_str()
        .map_err(|_| ApiError::unauthorized("malformed Authorization header"))?;
    let path_and_query = original_uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let url = format!("{}{}", state.api_url, path_and_query);
    let policy = finite_authn::AuthPolicy::default();
    finitesites_proto::nip98::verify_auth_header(
        header_value,
        &url,
        method,
        body,
        now_unix(),
        policy.nip98_max_skew_seconds,
    )
    .map_err(|error| match error {
        finite_authn::AuthnError::AuthRejected(reason) => {
            ApiError::unauthorized(format!("auth rejected: {reason}"))
        }
        other => ApiError::unauthorized(other.to_string()),
    })
}

fn parse_json_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|error| ApiError::bad_request(format!("invalid json: {error}")))
}

async fn create_hosted_requester_assertion(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<HostedRequesterAssertionResponse>, ApiError> {
    let expected_token = state
        .viewer_session_service_token
        .as_deref()
        .ok_or_else(|| ApiError::unavailable("hosted requester assertions are not configured"))?;
    authenticate_viewer_session_service(&headers, expected_token)?;
    let request: HostedRequesterAssertionRequest = parse_json_body(&body)?;
    let mut engine = state.engine.lock().expect("engine mutex never poisoned");
    engine
        .create_hosted_requester_assertion(&request, now_unix())
        .map(Json)
        .map_err(ApiError::from)
}

fn authenticate_viewer_session_service(
    headers: &HeaderMap,
    expected_token: &str,
) -> Result<(), ApiError> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= MAX_AUTH_HEADER_BYTES as usize)
        .ok_or_else(|| ApiError::unauthorized("viewer session authorization required"))?;
    let supplied = raw
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::unauthorized("viewer session authorization required"))?;
    if !constant_time_eq(supplied.as_bytes(), expected_token.as_bytes()) {
        return Err(ApiError::unauthorized(
            "viewer session authorization required",
        ));
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut difference = left.len() ^ right.len();
    for index in 0..max_len {
        difference |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

pub(crate) fn valid_return_to(return_to: &str) -> bool {
    !return_to.is_empty()
        && return_to.len() <= MAX_VIEWER_RETURN_TO_BYTES as usize
        && return_to.starts_with('/')
        && !return_to.starts_with("//")
        && !return_to.contains('\\')
        && return_to.is_ascii()
        && !return_to.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
}

/// Best-effort client identity for rate limiting. Spoofable headers only
/// weaken the per-IP budget; the per-email budget still binds.
fn client_key(headers: &HeaderMap) -> String {
    let from_header = headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 64);
    from_header.unwrap_or("direct").to_string()
}

/// Engine errors that indicate operator-side failure also go to stderr.
fn log_if_internal(error: &EngineError) {
    let is_internal = matches!(
        error,
        EngineError::Store(_) | EngineError::Blob(finitesites_blob::BlobError::Io(_))
    );
    if is_internal {
        eprintln!("finitesitesd internal error: {error}");
    }
}

// ---- handlers -------------------------------------------------------------------

async fn healthz() -> Response {
    git_health_response(crate::git::preflight_git_dependency()).into_response()
}

fn git_health_response(preflight: Result<(), String>) -> (StatusCode, Json<serde_json::Value>) {
    match preflight {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ok": false,
                "error": ERROR_GIT_UNAVAILABLE,
            })),
        ),
    }
}

async fn api_not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "unknown api route")
}

async fn request_email_login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<finitesites_proto::dto::EmailLoginResponse>, ApiError> {
    let request: finitesites_proto::dto::EmailLoginRequest = parse_json_body(&body)?;
    let now = now_unix();
    let ip_key = format!("email-login-ip:{}", client_key(&headers));
    let email_key = format!(
        "email-login-email:{}",
        request.email.trim().to_ascii_lowercase()
    );
    let ip_allowed =
        state
            .login_limiter
            .check_and_record(&ip_key, crate::limiter::MAX_LINKS_PER_IP, now);
    let email_allowed =
        state
            .login_limiter
            .check_and_record(&email_key, crate::limiter::MAX_LINKS_PER_EMAIL, now);
    if !ip_allowed || !email_allowed {
        return Ok(Json(finitesites_proto::dto::EmailLoginResponse {
            email: request.email.trim().to_ascii_lowercase(),
        }));
    }

    let token = {
        let mut engine = state.engine.lock().expect("engine mutex never poisoned");
        engine
            .request_email_login(&request.email, now)
            .map_err(ApiError::from)?
    };
    if let Err(error) = state
        .mailer
        .send_email_login_token(&token.email, &token.token)
    {
        eprintln!("finitesitesd mail error: {error}");
        return Err(internal_error("mail delivery failure"));
    }
    Ok(Json(finitesites_proto::dto::EmailLoginResponse {
        email: token.email,
    }))
}

async fn register_auth(
    State(state): State<Arc<AppState>>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AuthRegisterResponse>, ApiError> {
    let actor = authenticate(&state, &headers, "POST", &original_uri, Some(&body))?;
    if !body.is_empty() {
        return Err(ApiError::bad_request("auth register takes no JSON body"));
    }
    let mut engine = state.engine.lock().expect("engine mutex never poisoned");
    let response = engine
        .register_publishing_principal(&actor, now_unix())
        .map_err(|error| {
            log_if_internal(&error);
            ApiError::from(error)
        })?;
    Ok(Json(response))
}

async fn redeem_email_login(
    State(state): State<Arc<AppState>>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<finitesites_proto::dto::EmailRedeemResponse>, ApiError> {
    let actor = authenticate(&state, &headers, "POST", &original_uri, Some(&body))?;
    let request: finitesites_proto::dto::EmailRedeemRequest = parse_json_body(&body)?;
    let mut engine = state.engine.lock().expect("engine mutex never poisoned");
    let outcome = engine
        .redeem_email_login(&actor, &request.email, &request.token, now_unix())
        .map_err(|error| {
            log_if_internal(&error);
            ApiError::from(error)
        })?;
    Ok(Json(finitesites_proto::dto::EmailRedeemResponse {
        email: outcome.email,
        pubkey: actor,
        linked_to_native_principal: outcome.linked_to_native_principal,
    }))
}

async fn register_sites_authorized_key(
    State(state): State<Arc<AppState>>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<SitesAuthorizedKeyResponse>, ApiError> {
    let actor = authenticate(&state, &headers, "POST", &original_uri, Some(&body))?;
    let request: SitesAuthorizedKeyRegisterRequest = parse_json_body(&body)?;
    let mut engine = state.engine.lock().expect("engine mutex never poisoned");
    let email = engine
        .consume_email_proof(&request.email, &request.token, now_unix())
        .map_err(|error| {
            log_if_internal(&error);
            ApiError::unauthorized("email proof was invalid, expired, or already used")
        })?;
    engine
        .register_sites_authorized_key(&actor, &email, now_unix())
        .map(Json)
        .map_err(ApiError::from)
}

async fn revoke_sites_authorized_key(
    State(state): State<Arc<AppState>>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<SitesAuthorizedKeyResponse>, ApiError> {
    // Any holder of a fresh mailbox proof may revoke a key from that
    // mailbox's keyset; the NIP-98 signature only proves a live operator.
    let _actor = authenticate(&state, &headers, "POST", &original_uri, Some(&body))?;
    let request: SitesAuthorizedKeyRevokeRequest = parse_json_body(&body)?;
    let mut engine = state.engine.lock().expect("engine mutex never poisoned");
    let email = engine
        .consume_email_proof(&request.email, &request.token, now_unix())
        .map_err(|error| {
            log_if_internal(&error);
            ApiError::unauthorized("email proof was invalid, expired, or already used")
        })?;
    engine
        .revoke_sites_authorized_key(&email, &request.target_npub, now_unix())
        .map(Json)
        .map_err(ApiError::from)
}

async fn init_project(
    State(state): State<Arc<AppState>>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ProjectInitResponse>, ApiError> {
    let owner = authenticate(&state, &headers, "POST", &original_uri, Some(&body))?;
    let mut request: ProjectInitRequest = parse_json_body(&body)?;
    if let Err(error) = crate::git::preflight_git_dependency() {
        eprintln!("finitesitesd Git dependency unavailable before project init: {error}");
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ERROR_GIT_UNAVAILABLE,
            "Git publishing is temporarily unavailable; no Project Init state changed. Wait for service health to recover, then retry this request once.",
        ));
    }
    let resolved_owner_email = {
        let mut engine = state.engine.lock().expect("engine mutex never poisoned");
        engine
            .resolve_project_owner_email(&owner, &request, now_unix())
            .map_err(ApiError::from)?
    };
    request.owner_email = Some(resolved_owner_email);
    let git_remote_url = git_remote_url(&state, &request.config.project.slug);
    let mut engine = state.engine.lock().expect("engine mutex never poisoned");
    let response = engine
        .init_project(&owner, &request, git_remote_url, now_unix())
        .map_err(|error| {
            log_if_internal(&error);
            ApiError::from(error)
        })?;
    drop(engine);
    if !response.dry_run
        && let Some(project_id) = response.project_id.as_deref()
        && let Err(error) = crate::git::ensure_bare_project_repo(
            &state.data_dir,
            project_id,
            &state.git_hook_helper_path,
        )
    {
        eprintln!("finitesitesd project repo setup failed: {error}");
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ERROR_GIT_REPOSITORY_SETUP_FAILED,
            "Project registry state was saved, but Git repository setup failed. After an operator repairs the Git dependency or repository storage, replay this exact Project Init request once; replay repairs the repository without creating a duplicate Project.",
        ));
    }
    Ok(Json(response))
}

async fn list_projects(
    State(state): State<Arc<AppState>>,
    original_uri: OriginalUri,
    headers: HeaderMap,
) -> Result<Json<ProjectListResponse>, ApiError> {
    let actor = authenticate(&state, &headers, "GET", &original_uri, None)?;
    let engine = state.engine.lock().expect("engine mutex never poisoned");
    let response = engine
        .project_list(&actor, &state.git_base_url)
        .map_err(|error| {
            log_if_internal(&error);
            ApiError::from(error)
        })?;
    Ok(Json(response))
}

async fn project_status(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    original_uri: OriginalUri,
    headers: HeaderMap,
) -> Result<Json<ProjectStatusResponse>, ApiError> {
    let actor = authenticate(&state, &headers, "GET", &original_uri, None)?;
    let git_remote_url = git_remote_url(&state, &slug);
    let engine = state.engine.lock().expect("engine mutex never poisoned");
    let response = engine
        .project_status(&actor, &slug, git_remote_url)
        .map_err(|error| {
            log_if_internal(&error);
            ApiError::from(error)
        })?;
    Ok(Json(response))
}

async fn grant_project(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    Query(query): Query<InviteQuery>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ProjectGrantResponse>, ApiError> {
    let owner = authenticate(&state, &headers, "POST", &original_uri, Some(&body))?;
    let request: ProjectGrantRequest = parse_json_body(&body)?;
    if query.send_invites && request.email.trim().is_empty() {
        return Err(ApiError::bad_request(
            "native npub collaborators do not use email invites",
        ));
    }
    let mut response = {
        let mut engine = state.engine.lock().expect("engine mutex never poisoned");
        engine
            .grant_project(&owner, &slug, &request, now_unix())
            .map_err(|error| {
                log_if_internal(&error);
                ApiError::from(error)
            })?
    };
    if query.send_invites {
        send_project_collaborator_invite(&state, &mut response)?;
    }
    Ok(Json(response))
}

async fn revoke_project(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ProjectRevokeResponse>, ApiError> {
    let owner = authenticate(&state, &headers, "POST", &original_uri, Some(&body))?;
    let request: ProjectRevokeRequest = parse_json_body(&body)?;
    let mut engine = state.engine.lock().expect("engine mutex never poisoned");
    let response = engine
        .revoke_project(&owner, &slug, &request, now_unix())
        .map_err(|error| {
            log_if_internal(&error);
            ApiError::from(error)
        })?;
    Ok(Json(response))
}

async fn auth_git(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<GitAuthResponse>, ApiError> {
    let actor = authenticate(&state, &headers, "POST", &original_uri, Some(&body))?;
    let request: GitAuthRequest = parse_json_body(&body)?;
    let git_remote_url = git_remote_url(&state, &slug);
    let mut engine = state.engine.lock().expect("engine mutex never poisoned");
    let (locally_authorized, sites_key_record_exists, email_linked) = match request.email.as_deref()
    {
        Some(email) => (
            engine
                .actor_has_sites_email_key(&actor, email)
                .map_err(ApiError::from)?,
            engine
                .actor_has_sites_email_key_record(&actor, email)
                .map_err(ApiError::from)?,
            engine
                .actor_has_linked_email(&actor, email)
                .map_err(ApiError::from)?,
        ),
        None => (false, false, false),
    };
    // Grant satisfaction is a local lookup only: a verified Email Link between
    // the mailbox and the signer's native Principal. A revoked Sites key
    // record stays a tombstone and fails closed in `mint_git_credential`.
    let verified_email = request
        .email
        .as_deref()
        .filter(|_| email_linked && !locally_authorized && !sites_key_record_exists);
    let response = match verified_email {
        Some(email) => engine
            .mint_git_credential_for_verified_email(
                &actor,
                &slug,
                email,
                git_remote_url,
                now_unix(),
            )
            .map_err(|error| {
                log_if_internal(&error);
                ApiError::from(error)
            })?,
        None => engine
            .mint_git_credential(
                &actor,
                &slug,
                request.email.as_deref(),
                git_remote_url,
                now_unix(),
            )
            .map_err(|error| {
                log_if_internal(&error);
                ApiError::from(error)
            })?,
    };
    Ok(Json(response))
}

fn git_remote_url(state: &AppState, slug: &str) -> String {
    format!("{}/{}.git", state.git_base_url, slug)
}

/// Viewer sharing is access management, not delivery: the share row itself
/// is the invitation (the added viewer authenticates through the Auth Gate
/// at the output URL). Emailed sign-in links were deleted with the
/// magic-link viewing flow; `invited_emails` stays on the wire, always
/// empty, for older clients.
async fn share_project_output(
    State(state): State<Arc<AppState>>,
    Path((slug, output_id)): Path<(String, String)>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ProjectOutputSharingResponse>, ApiError> {
    let actor = authenticate(&state, &headers, "POST", &original_uri, Some(&body))?;
    let request: SharingRequest = parse_json_body(&body)?;
    let outcome = {
        let mut engine = state.engine.lock().expect("engine mutex never poisoned");
        engine
            .set_project_output_sharing(&actor, &slug, &output_id, &request, now_unix())
            .map_err(|error| {
                log_if_internal(&error);
                ApiError::from(error)
            })?
    };
    Ok(Json(ProjectOutputSharingResponse {
        project_slug: slug,
        output_id,
        visibility: outcome.response.visibility,
        shared_emails: outcome.response.shared_emails,
        shared_npubs: outcome.response.shared_npubs,
        invited_emails: Vec::new(),
    }))
}

#[derive(serde::Deserialize, Default)]
struct InviteQuery {
    #[serde(default)]
    send_invites: bool,
}

fn send_project_collaborator_invite(
    state: &AppState,
    response: &mut ProjectGrantResponse,
) -> Result<(), ApiError> {
    let token = {
        let mut engine = state.engine.lock().expect("engine mutex never poisoned");
        engine
            .request_email_login(&response.collaborator.email, now_unix())
            .map_err(|error| {
                log_if_internal(&error);
                ApiError::from(error)
            })?
    };

    let git_remote_url = git_remote_url_for_base(&state.git_base_url, &response.project_slug);
    state
        .mailer
        .send_project_collaborator_invite(&ProjectCollaboratorInvite {
            email: &token.email,
            project_slug: &response.project_slug,
            role: &response.collaborator.role,
            api_url: &state.api_url,
            git_remote_url: &git_remote_url,
            email_login_token: &token.token,
            outputs: &[],
        })
        .map_err(|error| {
            eprintln!("finitesitesd project collaborator invite mail error: {error}");
            internal_error("mail delivery failure")
        })?;
    response.invited_emails = vec![token.email];
    Ok(())
}

fn git_remote_url_for_base(base: &str, slug: &str) -> String {
    format!("{base}/{slug}.git")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_is_unavailable_when_git_preflight_fails() {
        let (status, Json(body)) = git_health_response(Err("missing git".to_string()));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], ERROR_GIT_UNAVAILABLE);
    }

    #[test]
    fn healthy_response_keeps_the_stable_success_body() {
        let (status, Json(body)) = git_health_response(Ok(()));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({ "ok": true }));
    }
}
