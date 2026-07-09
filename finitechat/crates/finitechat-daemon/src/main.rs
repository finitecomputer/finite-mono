use std::convert::Infallible;
use std::env;
use std::io::{self, Read};
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use finitechat_core::{
    AppAction, AppState, FiniteChatCoreError, FiniteChatRuntime, OpenOptions,
    nostr_identity_from_account_secret_hex, nostr_identity_from_nsec,
};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use tower_http::cors::CorsLayer;

const DEFAULT_BIND: &str = "127.0.0.1:38917";
const DEFAULT_SERVER_URL: &str = "https://chat.finite.computer";
const DEFAULT_UPDATE_TIMEOUT_MILLIS: u64 = 30_000;

#[derive(Debug, Parser)]
#[command(name = "finitechatd")]
#[command(about = "Finite Chat local daemon for thin native and Electron clients")]
struct Args {
    #[arg(long, default_value = DEFAULT_BIND)]
    bind: SocketAddr,
    #[arg(long)]
    data_dir: Option<String>,
    #[arg(long)]
    server_url: Option<String>,
    #[arg(long)]
    device_id: Option<String>,
    /// Read an nsec or 64-hex account secret from stdin. This avoids leaking
    /// secrets through argv or shell history for desktop launches.
    #[arg(long)]
    account_secret_stdin: bool,
    #[arg(long)]
    now_unix_seconds: Option<u64>,
}

#[derive(Clone)]
struct DaemonState {
    runtime: Arc<FiniteChatRuntime>,
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

#[derive(Debug, Error)]
enum DaemonError {
    #[error("missing required option: {0}")]
    MissingOption(&'static str),
    #[error(transparent)]
    Core(#[from] FiniteChatCoreError),
    #[error("task failed: {0}")]
    Task(String),
    #[error("serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to read account secret from stdin: {0}")]
    AccountSecretStdin(String),
}

#[tokio::main]
async fn main() -> Result<(), DaemonError> {
    let args = Args::parse();
    let data_dir = args
        .data_dir
        .or_else(|| env::var("FINITECHAT_HOME").ok())
        .ok_or(DaemonError::MissingOption("--data-dir or FINITECHAT_HOME"))?;
    let server_url = args
        .server_url
        .or_else(|| env::var("FINITECHAT_SERVER_URL").ok())
        .unwrap_or_else(|| DEFAULT_SERVER_URL.to_owned());
    let device_id = args
        .device_id
        .or_else(|| env::var("FINITECHAT_DEVICE_ID").ok())
        .unwrap_or_else(|| "electron".to_owned());
    let account_secret_hex = resolve_account_secret_input(args.account_secret_stdin)?;
    let now_unix_seconds = args.now_unix_seconds.or_else(|| {
        env::var("FINITECHAT_FIXED_NOW_UNIX_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
    });
    let runtime = FiniteChatRuntime::open(OpenOptions {
        data_dir,
        server_url,
        device_id,
        account_secret_hex,
        now_unix_seconds,
    })?;
    let state = DaemonState { runtime };
    let app = Router::new()
        .route("/v1/healthz", get(healthz))
        .route("/v1/app/state", get(app_state))
        .route("/v1/app/actions", post(dispatch_action))
        .route("/v1/app/updates", get(app_updates))
        .layer(CorsLayer::permissive())
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .map_err(|error| DaemonError::Task(error.to_string()))?;
    axum::serve(listener, app)
        .await
        .map_err(|error| DaemonError::Task(error.to_string()))
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
    Sse::new(stream).keep_alive(KeepAlive::default())
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

fn resolve_account_secret_input(read_stdin: bool) -> Result<Option<String>, DaemonError> {
    if !read_stdin {
        return Ok(None);
    }
    let mut secret = String::new();
    io::stdin()
        .read_to_string(&mut secret)
        .map_err(|error| DaemonError::AccountSecretStdin(error.to_string()))?;
    normalize_account_secret_input(&secret).map(Some)
}

fn normalize_account_secret_input(raw: &str) -> Result<String, DaemonError> {
    let trimmed = raw.trim();
    let without_prefix = trimmed.strip_prefix("nostr:").unwrap_or(trimmed);
    let material = if without_prefix.starts_with("nsec1") {
        nostr_identity_from_nsec(without_prefix.to_owned())?
    } else {
        nostr_identity_from_account_secret_hex(without_prefix.to_owned())?
    };
    Ok(material.account_secret_hex)
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
            Self::MissingOption(_) => StatusCode::BAD_REQUEST,
            Self::AccountSecretStdin(_) => StatusCode::BAD_REQUEST,
            Self::Task(_) | Self::Serialize(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(json!({
            "error": self.to_string(),
        }));
        (status, body).into_response()
    }
}
