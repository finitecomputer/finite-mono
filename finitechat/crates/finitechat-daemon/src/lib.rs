use std::convert::Infallible;
use std::fmt;
use std::io::Read;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use finitechat_core::{
    AppAction, AppState, FiniteChatCoreError, FiniteChatRuntime,
    nostr_identity_from_account_secret_hex, nostr_identity_from_nsec,
};
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

pub const DEFAULT_BIND: &str = "127.0.0.1:0";
pub const DEFAULT_SERVER_URL: &str = "https://chat.finite.computer";
pub const MAX_STARTUP_SECRETS_BYTES: usize = 2 * 1024;

const DEFAULT_UPDATE_TIMEOUT_MILLIS: u64 = 30_000;
const AUTH_TOKEN_HEX_BYTES: usize = 32;

#[derive(Clone)]
struct DaemonState {
    runtime: Arc<FiniteChatRuntime>,
}

#[derive(Clone)]
struct DaemonAuthorization {
    token: Arc<str>,
}

#[derive(Debug, Deserialize)]
struct UpdatesQuery {
    timeout_millis: Option<u64>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    rev: u64,
    account_id: String,
    device_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartupSecretsDocument {
    auth_token: String,
    #[serde(default)]
    account_secret: Option<String>,
}

/// Secrets supplied once by the supervising desktop process over the daemon's
/// stdin. The custom Debug representation intentionally never includes secret
/// material.
pub struct StartupSecrets {
    pub auth_token: String,
    pub account_secret_hex: Option<String>,
}

impl fmt::Debug for StartupSecrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StartupSecrets")
            .field("auth_token", &"<redacted>")
            .field(
                "account_secret_hex",
                &self.account_secret_hex.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("missing required option: {0}")]
    MissingOption(&'static str),
    #[error("daemon bind address must be loopback")]
    NonLoopbackBind,
    #[error("invalid daemon startup secrets")]
    InvalidStartupSecrets,
    #[error("daemon startup secrets are too large")]
    StartupSecretsTooLarge,
    #[error("failed to read daemon startup secrets")]
    StartupSecretsRead,
    #[error(transparent)]
    Core(#[from] FiniteChatCoreError),
    #[error("task failed: {0}")]
    Task(String),
    #[error("serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Build the authenticated local daemon API around a Rust-owned app runtime.
/// The bearer is deliberately process-local and is never projected in
/// `AppState` or any response.
pub fn app(
    runtime: Arc<FiniteChatRuntime>,
    auth_token: impl Into<String>,
) -> Result<Router, DaemonError> {
    let auth_token = auth_token.into();
    validate_auth_token(&auth_token)?;
    let authorization = DaemonAuthorization {
        token: Arc::from(auth_token),
    };
    let state = DaemonState { runtime };
    Ok(Router::new()
        .route("/v1/healthz", get(healthz))
        .route("/v1/app/state", get(app_state))
        .route("/v1/app/actions", post(dispatch_action))
        .route("/v1/app/updates", get(app_updates))
        .route_layer(middleware::from_fn_with_state(
            authorization,
            require_authorization,
        ))
        .with_state(state))
}

pub fn validate_loopback_bind(bind: SocketAddr) -> Result<(), DaemonError> {
    if bind.ip().is_loopback() {
        Ok(())
    } else {
        Err(DaemonError::NonLoopbackBind)
    }
}

/// Read exactly one bounded JSON startup document from stdin. Parsing and
/// validation failures are intentionally generic so malformed input cannot be
/// reflected into logs.
pub fn read_startup_secrets(reader: impl Read) -> Result<StartupSecrets, DaemonError> {
    let mut bytes = Vec::new();
    let mut bounded = reader.take((MAX_STARTUP_SECRETS_BYTES + 1) as u64);
    bounded
        .read_to_end(&mut bytes)
        .map_err(|_| DaemonError::StartupSecretsRead)?;
    if bytes.len() > MAX_STARTUP_SECRETS_BYTES {
        return Err(DaemonError::StartupSecretsTooLarge);
    }
    let document: StartupSecretsDocument =
        serde_json::from_slice(&bytes).map_err(|_| DaemonError::InvalidStartupSecrets)?;
    validate_auth_token(&document.auth_token)?;
    let account_secret_hex = document
        .account_secret
        .as_deref()
        .map(normalize_account_secret_input)
        .transpose()
        .map_err(|_| DaemonError::InvalidStartupSecrets)?;
    Ok(StartupSecrets {
        auth_token: document.auth_token,
        account_secret_hex,
    })
}

async fn require_authorization(
    State(authorization): State<DaemonAuthorization>,
    request: Request,
    next: Next,
) -> Response {
    let supplied = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if !supplied.is_some_and(|supplied| {
        constant_time_eq(supplied.as_bytes(), authorization.token.as_bytes())
    }) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "daemon authorization is required" })),
        )
            .into_response();
    }
    next.run(request).await
}

async fn healthz(State(state): State<DaemonState>) -> Result<Json<HealthResponse>, DaemonError> {
    let app = state.runtime.state()?;
    Ok(Json(HealthResponse {
        status: "ok",
        rev: app.rev,
        account_id: app.identity.account_id,
        device_id: app.identity.device_id,
    }))
}

async fn app_state(State(state): State<DaemonState>) -> Result<Json<AppState>, DaemonError> {
    Ok(Json(runtime_state(&state.runtime)?))
}

async fn dispatch_action(
    State(state): State<DaemonState>,
    Json(action): Json<AppAction>,
) -> Result<Json<AppState>, DaemonError> {
    let runtime = Arc::clone(&state.runtime);
    let mut app = tokio::task::spawn_blocking(move || runtime.dispatch_and_wait(action))
        .await
        .map_err(|error| DaemonError::Task(error.to_string()))??;
    redact_app_state(&mut app);
    Ok(Json(app))
}

async fn app_updates(
    State(state): State<DaemonState>,
    Query(query): Query<UpdatesQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let timeout_millis = query
        .timeout_millis
        .unwrap_or(DEFAULT_UPDATE_TIMEOUT_MILLIS)
        .max(1_000);
    let initial = match runtime_state(&state.runtime) {
        Ok(state) => state_event(&state).unwrap_or_else(error_event),
        Err(error) => error_event(error),
    };
    let initial_stream = futures_util::stream::once(async move { Ok(initial) });
    let stream = futures_util::stream::unfold(state.runtime, move |runtime| async move {
        let next_runtime = Arc::clone(&runtime);
        let update = tokio::task::spawn_blocking(move || {
            next_runtime
                .wait_for_update(timeout_millis)
                .or_else(|_| next_runtime.state())
        })
        .await;
        let event = match update {
            Ok(Ok(mut state)) => {
                redact_app_state(&mut state);
                state_event(&state).unwrap_or_else(error_event)
            }
            Ok(Err(error)) => error_event(error),
            Err(error) => error_event(error.to_string()),
        };
        Some((Ok(event), runtime))
    });
    Sse::new(initial_stream.chain(stream)).keep_alive(KeepAlive::default())
}

fn state_event(state: &AppState) -> Result<Event, serde_json::Error> {
    Ok(Event::default()
        .event("state")
        .id(state.rev.to_string())
        .data(serde_json::to_string(state)?))
}

fn runtime_state(runtime: &FiniteChatRuntime) -> Result<AppState, FiniteChatCoreError> {
    let mut state = runtime.state()?;
    redact_app_state(&mut state);
    Ok(state)
}

fn redact_app_state(state: &mut AppState) {
    state.identity.account_secret_hex.clear();
}

fn validate_auth_token(token: &str) -> Result<(), DaemonError> {
    if token.len() == AUTH_TOKEN_HEX_BYTES * 2
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(DaemonError::InvalidStartupSecrets)
    }
}

fn normalize_account_secret_input(raw: &str) -> Result<String, FiniteChatCoreError> {
    let trimmed = raw.trim();
    let without_prefix = trimmed.strip_prefix("nostr:").unwrap_or(trimmed);
    let material = if without_prefix.starts_with("nsec1") {
        nostr_identity_from_nsec(without_prefix.to_owned())?
    } else {
        nostr_identity_from_account_secret_hex(without_prefix.to_owned())?
    };
    Ok(material.account_secret_hex)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn error_event(error: impl ToString) -> Event {
    Event::default()
        .event("error")
        .data(json!({ "error": error.to_string() }).to_string())
}

impl IntoResponse for DaemonError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Core(FiniteChatCoreError::Client { .. }) => StatusCode::BAD_REQUEST,
            Self::Core(FiniteChatCoreError::Profile { .. }) => StatusCode::BAD_REQUEST,
            Self::Core(FiniteChatCoreError::ServerRejected { .. }) => StatusCode::BAD_GATEWAY,
            Self::Core(FiniteChatCoreError::Delivery { .. }) => StatusCode::BAD_GATEWAY,
            Self::Core(FiniteChatCoreError::Filesystem { .. }) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Core(FiniteChatCoreError::Store { .. }) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Core(FiniteChatCoreError::InvalidAccountSecret) => StatusCode::BAD_REQUEST,
            Self::Core(FiniteChatCoreError::LockPoisoned) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::MissingOption(_)
            | Self::NonLoopbackBind
            | Self::InvalidStartupSecrets
            | Self::StartupSecretsTooLarge
            | Self::StartupSecretsRead => StatusCode::BAD_REQUEST,
            Self::Task(_) | Self::Serialize(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(json!({
            "error": self.to_string(),
        }));
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn startup_secrets_are_bounded_normalized_and_redacted() {
        let secret = "0000000000000000000000000000000000000000000000000000000000000003";
        let input = format!("{{\"auth_token\":\"{TOKEN}\",\"account_secret\":\"nostr:{secret}\"}}");
        let parsed = read_startup_secrets(input.as_bytes()).unwrap();
        assert_eq!(parsed.auth_token, TOKEN);
        assert_eq!(parsed.account_secret_hex.as_deref(), Some(secret));
        let debug = format!("{parsed:?}");
        assert!(!debug.contains(TOKEN));
        assert!(!debug.contains(secret));
        assert!(debug.contains("<redacted>"));

        let oversized = vec![b'x'; MAX_STARTUP_SECRETS_BYTES + 1];
        assert!(matches!(
            read_startup_secrets(oversized.as_slice()),
            Err(DaemonError::StartupSecretsTooLarge)
        ));
    }

    #[test]
    fn startup_secrets_reject_weak_tokens_and_unknown_fields_without_echoing_values() {
        for input in [
            r#"{"auth_token":"weak"}"#,
            r#"{"auth_token":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","extra":"do-not-echo"}"#,
        ] {
            let error = read_startup_secrets(input.as_bytes()).unwrap_err();
            let message = error.to_string();
            assert_eq!(message, "invalid daemon startup secrets");
            assert!(!message.contains("weak"));
            assert!(!message.contains("do-not-echo"));
        }
    }

    #[test]
    fn bind_validation_accepts_only_loopback() {
        assert!(validate_loopback_bind("127.0.0.1:0".parse().unwrap()).is_ok());
        assert!(validate_loopback_bind("[::1]:0".parse().unwrap()).is_ok());
        assert!(matches!(
            validate_loopback_bind("0.0.0.0:0".parse().unwrap()),
            Err(DaemonError::NonLoopbackBind)
        ));
    }
}
