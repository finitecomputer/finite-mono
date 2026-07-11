use std::convert::Infallible;
use std::fmt;
use std::fs;
use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{
    DefaultBodyLimit, FromRequest, Multipart, Path as AxumPath, Query, Request, State,
};
use axum::http::StatusCode;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, HeaderValue,
};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use finitechat_core::{
    AppAction, AppState, ChatMediaAttachment, ChatMediaKind, FiniteChatCoreError,
    FiniteChatRuntime, OutboundAttachment, nostr_identity_from_account_secret_hex,
    nostr_identity_from_nsec,
};
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

pub mod device_link;

pub const DEFAULT_BIND: &str = "127.0.0.1:0";
pub const DEFAULT_SERVER_URL: &str = "https://chat.finite.computer";
pub const MAX_STARTUP_SECRETS_BYTES: usize = 2 * 1024;
pub const MAX_DAEMON_ATTACHMENTS_PER_MESSAGE: usize = 8;
pub const MAX_DAEMON_ATTACHMENT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_DAEMON_ATTACHMENT_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_MULTIPART_OVERHEAD_BYTES: usize = 1024 * 1024;
pub const MAX_DAEMON_MULTIPART_BODY_BYTES: usize =
    MAX_DAEMON_ATTACHMENT_TOTAL_BYTES + MAX_MULTIPART_OVERHEAD_BYTES;
const MAX_MULTIPART_TEXT_FIELD_BYTES: usize = 16 * 1024;
const MAX_ATTACHMENT_FILENAME_BYTES: usize = 255;
const MAX_ATTACHMENT_MIME_TYPE_BYTES: usize = 128;

const DEFAULT_UPDATE_TIMEOUT_MILLIS: u64 = 30_000;
const AUTH_TOKEN_HEX_BYTES: usize = 32;

#[derive(Clone)]
struct DaemonState {
    runtime: Arc<FiniteChatRuntime>,
    attachment_cache_root: Option<Arc<PathBuf>>,
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
    #[error("invalid multipart attachment request: {0}")]
    InvalidMultipart(String),
    #[error("attachment request is too large: {0}")]
    PayloadTooLarge(String),
    #[error("attachment was not found")]
    AttachmentNotFound,
    #[error("attachment is not available")]
    AttachmentUnavailable,
    #[error("attachment cache path is not safe to serve")]
    UnsafeAttachmentPath,
    #[error(transparent)]
    Core(#[from] FiniteChatCoreError),
    #[error("task failed: {0}")]
    Task(String),
    #[error("serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error(transparent)]
    DeviceLink(#[from] device_link::DeviceLinkBootstrapError),
}

/// Build the authenticated local daemon API around a Rust-owned app runtime.
/// The bearer is deliberately process-local and is never projected in
/// `AppState` or any response.
pub fn app(
    runtime: Arc<FiniteChatRuntime>,
    auth_token: impl Into<String>,
) -> Result<Router, DaemonError> {
    app_with_attachment_cache_root(runtime, auth_token, None)
}

pub fn app_with_data_dir(
    runtime: Arc<FiniteChatRuntime>,
    auth_token: impl Into<String>,
    data_dir: impl Into<PathBuf>,
) -> Result<Router, DaemonError> {
    app_with_attachment_cache_root(
        runtime,
        auth_token,
        Some(Arc::new(data_dir.into().join("attachments"))),
    )
}

fn app_with_attachment_cache_root(
    runtime: Arc<FiniteChatRuntime>,
    auth_token: impl Into<String>,
    attachment_cache_root: Option<Arc<PathBuf>>,
) -> Result<Router, DaemonError> {
    let auth_token = auth_token.into();
    validate_auth_token(&auth_token)?;
    let authorization = DaemonAuthorization {
        token: Arc::from(auth_token),
    };
    let state = DaemonState {
        runtime,
        attachment_cache_root,
    };
    Ok(Router::new()
        .route("/v1/healthz", get(healthz))
        .route("/v1/app/state", get(app_state))
        .route("/v1/app/actions", post(dispatch_action))
        .route(
            "/v1/app/attachments",
            post(upload_attachments).layer(DefaultBodyLimit::max(MAX_DAEMON_MULTIPART_BODY_BYTES)),
        )
        .route(
            "/v1/app/attachments/{room_id}/{message_id}/{attachment_id}",
            get(download_attachment),
        )
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
    dispatch_runtime_action(Arc::clone(&state.runtime), action).await
}

async fn dispatch_runtime_action(
    runtime: Arc<FiniteChatRuntime>,
    action: AppAction,
) -> Result<Json<AppState>, DaemonError> {
    let mut app = tokio::task::spawn_blocking(move || runtime.dispatch_and_wait(action))
        .await
        .map_err(|error| DaemonError::Task(error.to_string()))??;
    redact_app_state(&mut app);
    Ok(Json(app))
}

#[derive(Default)]
struct AttachmentUploadForm {
    room_id: Option<String>,
    topic_id: Option<String>,
    chat_id: Option<String>,
    caption: Option<String>,
    reply_to_message_id: Option<String>,
    attachments: Vec<OutboundAttachment>,
    total_attachment_bytes: usize,
}

async fn upload_attachments(
    State(state): State<DaemonState>,
    request: Request,
) -> Result<Json<AppState>, DaemonError> {
    let action = parse_attachment_upload(request).await?;
    dispatch_runtime_action(Arc::clone(&state.runtime), action).await
}

async fn parse_attachment_upload(request: Request) -> Result<AppAction, DaemonError> {
    if request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_DAEMON_MULTIPART_BODY_BYTES)
    {
        return Err(DaemonError::PayloadTooLarge(format!(
            "multipart body must be at most {MAX_DAEMON_MULTIPART_BODY_BYTES} bytes"
        )));
    }

    let mut multipart = Multipart::from_request(request, &())
        .await
        .map_err(|_| DaemonError::InvalidMultipart("missing or invalid boundary".to_owned()))?;
    let mut form = AttachmentUploadForm::default();
    while let Some(mut field) = multipart.next_field().await.map_err(map_multipart_error)? {
        let field_name = field.name().unwrap_or_default().to_owned();
        match field_name.as_str() {
            "file" | "files" | "attachments" => {
                if form.attachments.len() >= MAX_DAEMON_ATTACHMENTS_PER_MESSAGE {
                    return Err(DaemonError::PayloadTooLarge(format!(
                        "at most {MAX_DAEMON_ATTACHMENTS_PER_MESSAGE} files are allowed"
                    )));
                }
                let filename = normalize_attachment_filename(field.file_name())?;
                let mime_type = normalize_attachment_mime_type(field.content_type());
                let kind = attachment_kind_for_mime(&mime_type);
                let mut bytes = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(map_multipart_error)? {
                    validate_attachment_chunk_lengths(
                        bytes.len(),
                        form.total_attachment_bytes,
                        chunk.len(),
                    )?;
                    bytes.extend_from_slice(&chunk);
                    form.total_attachment_bytes += chunk.len();
                }
                form.attachments.push(OutboundAttachment {
                    filename,
                    mime_type,
                    kind,
                    bytes,
                });
            }
            "room_id" => {
                form.room_id = Some(
                    read_single_text_field(form.room_id.is_some(), field_name, &mut field).await?,
                );
            }
            "topic_id" => {
                form.topic_id = Some(
                    read_single_text_field(form.topic_id.is_some(), field_name, &mut field).await?,
                );
            }
            "chat_id" => {
                form.chat_id = Some(
                    read_single_text_field(form.chat_id.is_some(), field_name, &mut field).await?,
                );
            }
            "caption" => {
                form.caption = Some(
                    read_single_text_field(form.caption.is_some(), field_name, &mut field).await?,
                );
            }
            "reply_to_message_id" => {
                form.reply_to_message_id = Some(
                    read_single_text_field(
                        form.reply_to_message_id.is_some(),
                        field_name,
                        &mut field,
                    )
                    .await?,
                );
            }
            _ => {
                return Err(DaemonError::InvalidMultipart(format!(
                    "unsupported field '{field_name}'"
                )));
            }
        }
    }

    let room_id = required_text_field("room_id", form.room_id)?;
    if form.attachments.is_empty() {
        return Err(DaemonError::InvalidMultipart(
            "at least one file is required".to_owned(),
        ));
    }
    let topic_id = optional_text_field(form.topic_id);
    let chat_id = optional_text_field(form.chat_id);
    if topic_id.is_some() != chat_id.is_some() {
        return Err(DaemonError::InvalidMultipart(
            "topic_id and chat_id must be provided together".to_owned(),
        ));
    }
    let caption = form.caption.unwrap_or_default().trim().to_owned();
    let reply_to_message_id = optional_text_field(form.reply_to_message_id);
    let action = match (topic_id, chat_id) {
        (Some(topic_id), Some(chat_id)) => AppAction::SendChatAttachments {
            room_id,
            topic_id,
            chat_id,
            attachments: form.attachments,
            caption,
            reply_to_message_id,
        },
        (None, None) => AppAction::SendAttachments {
            room_id,
            attachments: form.attachments,
            caption,
            reply_to_message_id,
        },
        _ => unreachable!("paired attachment route fields were validated"),
    };
    Ok(action)
}

fn validate_attachment_chunk_lengths(
    file_bytes: usize,
    total_bytes: usize,
    chunk_bytes: usize,
) -> Result<(), DaemonError> {
    if file_bytes.saturating_add(chunk_bytes) > MAX_DAEMON_ATTACHMENT_BYTES {
        return Err(DaemonError::PayloadTooLarge(format!(
            "each file must be at most {MAX_DAEMON_ATTACHMENT_BYTES} bytes"
        )));
    }
    if total_bytes.saturating_add(chunk_bytes) > MAX_DAEMON_ATTACHMENT_TOTAL_BYTES {
        return Err(DaemonError::PayloadTooLarge(format!(
            "files must total at most {MAX_DAEMON_ATTACHMENT_TOTAL_BYTES} bytes"
        )));
    }
    Ok(())
}

async fn read_single_text_field(
    duplicate: bool,
    field_name: String,
    field: &mut axum::extract::multipart::Field<'_>,
) -> Result<String, DaemonError> {
    if duplicate {
        return Err(DaemonError::InvalidMultipart(format!(
            "field '{field_name}' must appear only once"
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(map_multipart_error)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_MULTIPART_TEXT_FIELD_BYTES {
            return Err(DaemonError::PayloadTooLarge(format!(
                "field '{field_name}' is too large"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map_err(|_| DaemonError::InvalidMultipart(format!("field '{field_name}' must be UTF-8")))
}

fn required_text_field(field_name: &str, value: Option<String>) -> Result<String, DaemonError> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DaemonError::InvalidMultipart(format!("field '{field_name}' is required")))
}

fn optional_text_field(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn normalize_attachment_filename(value: Option<&str>) -> Result<String, DaemonError> {
    let value = value.ok_or_else(|| {
        DaemonError::InvalidMultipart("each file must include a filename".to_owned())
    })?;
    let filename = value.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if filename.is_empty()
        || filename.len() > MAX_ATTACHMENT_FILENAME_BYTES
        || filename.chars().any(char::is_control)
    {
        return Err(DaemonError::InvalidMultipart(
            "attachment filename is invalid".to_owned(),
        ));
    }
    Ok(filename.to_owned())
}

fn normalize_attachment_mime_type(value: Option<&str>) -> String {
    let value = value.unwrap_or_default().trim().to_ascii_lowercase();
    let mut parts = value.split('/');
    let top = parts.next().unwrap_or_default();
    let subtype = parts.next().unwrap_or_default();
    let valid_token = |token: &str| {
        !token.is_empty()
            && token.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                    )
            })
    };
    if value.len() <= MAX_ATTACHMENT_MIME_TYPE_BYTES
        && parts.next().is_none()
        && valid_token(top)
        && valid_token(subtype)
    {
        value
    } else {
        "application/octet-stream".to_owned()
    }
}

fn attachment_kind_for_mime(mime_type: &str) -> ChatMediaKind {
    if mime_type.starts_with("image/") {
        ChatMediaKind::Image
    } else if mime_type.starts_with("video/") {
        ChatMediaKind::Video
    } else if mime_type.starts_with("audio/") {
        ChatMediaKind::VoiceNote
    } else {
        ChatMediaKind::File
    }
}

fn map_multipart_error(error: axum::extract::multipart::MultipartError) -> DaemonError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        DaemonError::PayloadTooLarge("multipart body exceeded its limit".to_owned())
    } else {
        DaemonError::InvalidMultipart(error.body_text())
    }
}

async fn download_attachment(
    State(state): State<DaemonState>,
    AxumPath((room_id, message_id, attachment_id)): AxumPath<(String, String, String)>,
) -> Result<Response, DaemonError> {
    let initial_state = state.runtime.state()?;
    let mut attachment =
        attachment_from_state(&initial_state, &room_id, &message_id, &attachment_id)
            .ok_or(DaemonError::AttachmentNotFound)?;
    let cache_root = state
        .attachment_cache_root
        .as_ref()
        .map(|root| root.as_ref().clone())
        .ok_or(DaemonError::AttachmentUnavailable)?;
    let mut served =
        match read_cached_attachment(cache_root.clone(), attachment.local_path.clone()).await {
            Ok(served) => served,
            Err(DaemonError::UnsafeAttachmentPath) => None,
            Err(error) => return Err(error),
        };
    if served.is_none() {
        let action = AppAction::DownloadAttachment {
            room_id: room_id.clone(),
            message_id: message_id.clone(),
            attachment_id: attachment_id.clone(),
        };
        let _ = dispatch_runtime_action(Arc::clone(&state.runtime), action).await?;
        let next = state.runtime.state()?;
        attachment = attachment_from_state(&next, &room_id, &message_id, &attachment_id)
            .ok_or(DaemonError::AttachmentUnavailable)?;
        served = read_cached_attachment(cache_root, attachment.local_path.clone()).await?;
    }
    let bytes = served.ok_or(DaemonError::AttachmentUnavailable)?;
    Ok(attachment_response(bytes, &attachment))
}

fn attachment_from_state(
    state: &AppState,
    room_id: &str,
    message_id: &str,
    attachment_id: &str,
) -> Option<ChatMediaAttachment> {
    state
        .messages
        .iter()
        .filter(|message| message.room_id == room_id && message.message_id == message_id)
        .flat_map(|message| message.media.iter())
        .find(|attachment| attachment.attachment_id == attachment_id)
        .cloned()
        .or_else(|| {
            state
                .media_gallery
                .as_ref()
                .filter(|gallery| gallery.room_id == room_id)
                .and_then(|gallery| {
                    gallery.items.iter().find(|item| {
                        item.message_id == message_id && item.attachment_id == attachment_id
                    })
                })
                .map(|item| item.attachment.clone())
        })
}

async fn read_cached_attachment(
    cache_root: PathBuf,
    local_path: Option<String>,
) -> Result<Option<Vec<u8>>, DaemonError> {
    let Some(local_path) = local_path else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || {
        let cache_root = match fs::canonicalize(cache_root) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(DaemonError::AttachmentUnavailable),
        };
        let path = match fs::canonicalize(local_path) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(DaemonError::AttachmentUnavailable),
        };
        if !path.starts_with(&cache_root) {
            return Err(DaemonError::UnsafeAttachmentPath);
        }
        let metadata = fs::metadata(&path).map_err(|_| DaemonError::AttachmentUnavailable)?;
        if !metadata.is_file() {
            return Err(DaemonError::AttachmentUnavailable);
        }
        if metadata.len() > MAX_DAEMON_ATTACHMENT_BYTES as u64 {
            return Err(DaemonError::PayloadTooLarge(
                "cached attachment exceeds the serving limit".to_owned(),
            ));
        }
        fs::read(path)
            .map(Some)
            .map_err(|_| DaemonError::AttachmentUnavailable)
    })
    .await
    .map_err(|error| DaemonError::Task(error.to_string()))?
}

fn attachment_response(bytes: Vec<u8>, attachment: &ChatMediaAttachment) -> Response {
    let content_length = bytes.len();
    let mime_type = normalize_attachment_mime_type(Some(&attachment.mime_type));
    let disposition = if is_safe_inline_mime(&mime_type) {
        "inline"
    } else {
        "attachment"
    };
    let filename = safe_disposition_filename(&attachment.filename);
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("{disposition}; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"attachment\"")),
    );
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&content_length.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("sandbox"),
    );
    response
}

fn safe_disposition_filename(filename: &str) -> String {
    let mut safe = filename
        .chars()
        .take(128)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    safe = safe.trim_matches([' ', '.']).to_owned();
    if safe.is_empty() {
        "attachment".to_owned()
    } else {
        safe
    }
}

fn is_safe_inline_mime(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
    )
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
    for message in &mut state.messages {
        for attachment in &mut message.media {
            attachment.local_path = None;
        }
    }
    if let Some(gallery) = &mut state.media_gallery {
        for item in &mut gallery.items {
            item.attachment.local_path = None;
        }
    }
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
            | Self::StartupSecretsRead
            | Self::InvalidMultipart(_) => StatusCode::BAD_REQUEST,
            Self::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::AttachmentNotFound => StatusCode::NOT_FOUND,
            Self::AttachmentUnavailable => StatusCode::BAD_GATEWAY,
            Self::Task(_)
            | Self::Serialize(_)
            | Self::DeviceLink(_)
            | Self::UnsafeAttachmentPath => StatusCode::INTERNAL_SERVER_ERROR,
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
    use axum::body::Body;

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

    #[test]
    fn attachment_byte_limits_check_boundaries_without_large_allocations() {
        assert!(validate_attachment_chunk_lengths(0, 0, MAX_DAEMON_ATTACHMENT_BYTES).is_ok());
        assert!(matches!(
            validate_attachment_chunk_lengths(0, 0, MAX_DAEMON_ATTACHMENT_BYTES + 1),
            Err(DaemonError::PayloadTooLarge(_))
        ));
        assert!(
            validate_attachment_chunk_lengths(0, MAX_DAEMON_ATTACHMENT_TOTAL_BYTES - 1, 1,).is_ok()
        );
        assert!(matches!(
            validate_attachment_chunk_lengths(0, MAX_DAEMON_ATTACHMENT_TOTAL_BYTES, 1,),
            Err(DaemonError::PayloadTooLarge(_))
        ));
        assert!(matches!(
            validate_attachment_chunk_lengths(usize::MAX, usize::MAX, 1),
            Err(DaemonError::PayloadTooLarge(_))
        ));
    }

    #[tokio::test]
    async fn multipart_parser_preserves_binary_bytes_and_chat_scope() {
        let boundary = "finitechat-daemon-unit-boundary";
        let plaintext = b"binary\0bytes\r\nnot-json";
        let mut body = Vec::new();
        for (name, value) in [
            ("room_id", "room-test"),
            ("topic_id", "topic-test"),
            ("chat_id", "chat-test"),
            ("caption", " Binary proof "),
        ] {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"proof.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(plaintext);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let action = parse_attachment_upload(
            Request::post("/v1/app/attachments")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
        let AppAction::SendChatAttachments {
            room_id,
            topic_id,
            chat_id,
            attachments,
            caption,
            reply_to_message_id,
        } = action
        else {
            panic!("paired chat fields must create SendChatAttachments");
        };
        assert_eq!(room_id, "room-test");
        assert_eq!(topic_id, "topic-test");
        assert_eq!(chat_id, "chat-test");
        assert_eq!(caption, "Binary proof");
        assert_eq!(reply_to_message_id, None);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].filename, "proof.bin");
        assert_eq!(attachments[0].mime_type, "application/octet-stream");
        assert_eq!(attachments[0].kind, ChatMediaKind::File);
        assert_eq!(attachments[0].bytes, plaintext);
    }

    #[tokio::test]
    async fn attachment_cache_reader_confines_canonical_paths() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("device/attachments");
        fs::create_dir_all(&cache).unwrap();
        let inside = cache.join("inside.bin");
        let outside = root.path().join("outside.bin");
        fs::write(&inside, b"inside").unwrap();
        fs::write(&outside, b"outside").unwrap();

        assert_eq!(
            read_cached_attachment(cache.clone(), Some(inside.to_string_lossy().into_owned()))
                .await
                .unwrap(),
            Some(b"inside".to_vec())
        );
        assert!(matches!(
            read_cached_attachment(cache, Some(outside.to_string_lossy().into_owned())).await,
            Err(DaemonError::UnsafeAttachmentPath)
        ));
    }
}
