use std::collections::HashMap;
use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, FromRequest, Multipart, Path as AxumPath, Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, HeaderValue,
};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use finite_identity::{FiniteIdentity, IdentityPaths};
use finitechat_core::{
    AppAction, AppState, ChatMediaAttachment, ChatMediaKind, FiniteChatCoreError,
    FiniteChatRuntime, OpenOptions, OutboundAttachment,
};
use futures_util::{Stream, StreamExt};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const WORKOS_USER_HEADER: &str = "x-finite-workos-user-id";
const CREATED_BY: &str = concat!("finitechat-hosted-device/", env!("CARGO_PKG_VERSION"));
const DEFAULT_UPDATE_TIMEOUT_MILLIS: u64 = 30_000;
const MAX_USER_ID_BYTES: usize = 512;
/// The hosted browser surface is deliberately narrower than the core's wire
/// limit. A bounded fan-out keeps one HTTP request from monopolizing a Device.
pub const MAX_HOSTED_ATTACHMENTS_PER_MESSAGE: usize = 8;
/// Matches the v1 encrypted attachment plaintext ceiling.
pub const MAX_HOSTED_ATTACHMENT_BYTES: usize = 32 * 1024 * 1024;
/// Allows a small batch without allowing the per-file ceiling to multiply by
/// the full file-count ceiling in one allocation.
pub const MAX_HOSTED_ATTACHMENT_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_MULTIPART_OVERHEAD_BYTES: usize = 1024 * 1024;
pub const MAX_HOSTED_MULTIPART_BODY_BYTES: usize =
    MAX_HOSTED_ATTACHMENT_TOTAL_BYTES + MAX_MULTIPART_OVERHEAD_BYTES;
const MAX_MULTIPART_TEXT_FIELD_BYTES: usize = 16 * 1024;
const MAX_ATTACHMENT_FILENAME_BYTES: usize = 255;
const MAX_ATTACHMENT_MIME_TYPE_BYTES: usize = 128;

#[derive(Clone, Debug)]
pub struct HostedDeviceConfig {
    pub data_root: PathBuf,
    pub server_url: String,
    pub api_token: String,
}

#[derive(Clone)]
struct HostedDeviceState {
    config: HostedDeviceConfig,
    runtimes: Arc<Mutex<HashMap<String, Arc<FiniteChatRuntime>>>>,
}

impl HostedDeviceState {
    fn user_root(&self, user_id: &str) -> PathBuf {
        self.config
            .data_root
            .join("users")
            .join(user_storage_id(user_id))
    }

    fn chat_data_dir(&self, user_id: &str) -> PathBuf {
        self.user_root(user_id).join("chat")
    }

    fn runtime_for(&self, user_id: &str) -> Result<Arc<FiniteChatRuntime>, HostedDeviceError> {
        let mut runtimes = self
            .runtimes
            .lock()
            .map_err(|_| HostedDeviceError::LockPoisoned)?;
        if let Some(runtime) = runtimes.get(user_id) {
            return Ok(Arc::clone(runtime));
        }

        let user_root = self.user_root(user_id);
        let finite_home = user_root.join("finite-home");
        let identity_paths = IdentityPaths::with_finite_home(&finite_home);
        let identity = FiniteIdentity::load_or_generate(&identity_paths, CREATED_BY)?;
        let account_secret_hex = hex::encode(identity.expose_secret_bytes());
        let chat_data = user_root.join("chat");
        let runtime = FiniteChatRuntime::open(OpenOptions {
            data_dir: chat_data.to_string_lossy().into_owned(),
            server_url: self.config.server_url.clone(),
            device_id: "hosted-web".to_owned(),
            account_secret_hex: Some(account_secret_hex),
            now_unix_seconds: None,
        })?;
        runtimes.insert(user_id.to_owned(), Arc::clone(&runtime));
        Ok(runtime)
    }
}

#[derive(Debug, Error)]
pub enum HostedDeviceError {
    #[error("hosted device authorization is required")]
    Unauthorized,
    #[error("verified WorkOS user id is required")]
    MissingUser,
    #[error("invalid WorkOS user id")]
    InvalidUser,
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
    #[error("hosted device runtime cache lock poisoned")]
    LockPoisoned,
    #[error("hosted device task failed: {0}")]
    Task(String),
    #[error(transparent)]
    Core(#[from] FiniteChatCoreError),
    #[error(transparent)]
    Identity(#[from] finite_identity::Error),
    #[error(transparent)]
    Serialize(#[from] serde_json::Error),
}

impl IntoResponse for HostedDeviceError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::MissingUser | Self::InvalidUser | Self::InvalidMultipart(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::AttachmentNotFound => StatusCode::NOT_FOUND,
            Self::AttachmentUnavailable => StatusCode::BAD_GATEWAY,
            Self::Core(FiniteChatCoreError::Client { .. }) => StatusCode::BAD_REQUEST,
            Self::Core(FiniteChatCoreError::Profile { .. }) => StatusCode::BAD_REQUEST,
            Self::Core(FiniteChatCoreError::ServerRejected { .. })
            | Self::Core(FiniteChatCoreError::Delivery { .. }) => StatusCode::BAD_GATEWAY,
            Self::Core(_)
            | Self::Identity(_)
            | Self::Serialize(_)
            | Self::UnsafeAttachmentPath
            | Self::LockPoisoned
            | Self::Task(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    cached_devices: usize,
}

pub fn app(config: HostedDeviceConfig) -> Router {
    let state = HostedDeviceState {
        config,
        runtimes: Arc::new(Mutex::new(HashMap::new())),
    };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/app/state", get(app_state))
        .route("/v1/app/actions", post(dispatch_action))
        .route("/v1/app/updates", get(app_updates))
        .route(
            "/v1/app/attachments",
            post(upload_attachments).layer(DefaultBodyLimit::max(MAX_HOSTED_MULTIPART_BODY_BYTES)),
        )
        .route(
            "/v1/app/attachments/{room_id}/{message_id}/{attachment_id}",
            get(download_attachment),
        )
        .with_state(state)
}

async fn healthz(State(state): State<HostedDeviceState>) -> Json<HealthResponse> {
    let cached_devices = state
        .runtimes
        .lock()
        .map(|runtimes| runtimes.len())
        .unwrap_or_default();
    Json(HealthResponse {
        status: "ok",
        cached_devices,
    })
}

async fn app_state(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
) -> Result<Json<AppState>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    let runtime = state.runtime_for(&user_id)?;
    Ok(Json(redacted_state(runtime.state()?)))
}

async fn dispatch_action(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
    Json(action): Json<AppAction>,
) -> Result<Json<AppState>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    let runtime = state.runtime_for(&user_id)?;
    let next = tokio::task::spawn_blocking(move || runtime.dispatch_and_wait(action))
        .await
        .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(redacted_state(next)))
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
    State(state): State<HostedDeviceState>,
    request: Request,
) -> Result<Json<AppState>, HostedDeviceError> {
    // Authenticate from trusted proxy headers before the request body is
    // parsed or buffered.
    let user_id = authorized_user(&state, request.headers())?;
    if request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_HOSTED_MULTIPART_BODY_BYTES)
    {
        return Err(HostedDeviceError::PayloadTooLarge(format!(
            "multipart body must be at most {MAX_HOSTED_MULTIPART_BODY_BYTES} bytes"
        )));
    }

    let mut multipart = Multipart::from_request(request, &state)
        .await
        .map_err(|_| {
            HostedDeviceError::InvalidMultipart("missing or invalid boundary".to_owned())
        })?;
    let mut form = AttachmentUploadForm::default();
    while let Some(mut field) = multipart.next_field().await.map_err(map_multipart_error)? {
        let field_name = field.name().unwrap_or_default().to_owned();
        match field_name.as_str() {
            "file" | "files" | "attachments" => {
                if form.attachments.len() >= MAX_HOSTED_ATTACHMENTS_PER_MESSAGE {
                    return Err(HostedDeviceError::PayloadTooLarge(format!(
                        "at most {MAX_HOSTED_ATTACHMENTS_PER_MESSAGE} files are allowed"
                    )));
                }
                let filename = normalize_attachment_filename(field.file_name())?;
                let mime_type = normalize_attachment_mime_type(field.content_type());
                let kind = attachment_kind_for_mime(&mime_type);
                let mut bytes = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(map_multipart_error)? {
                    if bytes.len().saturating_add(chunk.len()) > MAX_HOSTED_ATTACHMENT_BYTES {
                        return Err(HostedDeviceError::PayloadTooLarge(format!(
                            "each file must be at most {MAX_HOSTED_ATTACHMENT_BYTES} bytes"
                        )));
                    }
                    if form.total_attachment_bytes.saturating_add(chunk.len())
                        > MAX_HOSTED_ATTACHMENT_TOTAL_BYTES
                    {
                        return Err(HostedDeviceError::PayloadTooLarge(format!(
                            "files must total at most {MAX_HOSTED_ATTACHMENT_TOTAL_BYTES} bytes"
                        )));
                    }
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
                return Err(HostedDeviceError::InvalidMultipart(format!(
                    "unsupported field '{field_name}'"
                )));
            }
        }
    }

    let room_id = required_text_field("room_id", form.room_id)?;
    if form.attachments.is_empty() {
        return Err(HostedDeviceError::InvalidMultipart(
            "at least one file is required".to_owned(),
        ));
    }
    let topic_id = optional_text_field(form.topic_id);
    let chat_id = optional_text_field(form.chat_id);
    if topic_id.is_some() != chat_id.is_some() {
        return Err(HostedDeviceError::InvalidMultipart(
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

    let runtime = state.runtime_for(&user_id)?;
    let next = tokio::task::spawn_blocking(move || runtime.dispatch_and_wait(action))
        .await
        .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(redacted_state(next)))
}

async fn read_single_text_field(
    duplicate: bool,
    field_name: String,
    field: &mut axum::extract::multipart::Field<'_>,
) -> Result<String, HostedDeviceError> {
    if duplicate {
        return Err(HostedDeviceError::InvalidMultipart(format!(
            "field '{field_name}' must appear only once"
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(map_multipart_error)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_MULTIPART_TEXT_FIELD_BYTES {
            return Err(HostedDeviceError::PayloadTooLarge(format!(
                "field '{field_name}' is too large"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| {
        HostedDeviceError::InvalidMultipart(format!("field '{field_name}' must be UTF-8"))
    })
}

fn required_text_field(
    field_name: &str,
    value: Option<String>,
) -> Result<String, HostedDeviceError> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            HostedDeviceError::InvalidMultipart(format!("field '{field_name}' is required"))
        })
}

fn optional_text_field(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn normalize_attachment_filename(value: Option<&str>) -> Result<String, HostedDeviceError> {
    let value = value.ok_or_else(|| {
        HostedDeviceError::InvalidMultipart("each file must include a filename".to_owned())
    })?;
    let filename = value.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if filename.is_empty()
        || filename.len() > MAX_ATTACHMENT_FILENAME_BYTES
        || filename.chars().any(char::is_control)
    {
        return Err(HostedDeviceError::InvalidMultipart(
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

fn map_multipart_error(error: axum::extract::multipart::MultipartError) -> HostedDeviceError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        HostedDeviceError::PayloadTooLarge("multipart body exceeded its limit".to_owned())
    } else {
        HostedDeviceError::InvalidMultipart(error.body_text())
    }
}

async fn download_attachment(
    State(state): State<HostedDeviceState>,
    AxumPath((room_id, message_id, attachment_id)): AxumPath<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    let runtime = state.runtime_for(&user_id)?;
    let initial_state = runtime.state()?;
    let mut attachment =
        attachment_from_state(&initial_state, &room_id, &message_id, &attachment_id)
            .ok_or(HostedDeviceError::AttachmentNotFound)?;
    let cache_root = state.chat_data_dir(&user_id).join("attachments");

    let initial_path = attachment.local_path.clone();
    // Never serve an out-of-cache path. A legacy attachment may still carry a
    // sender-local path alongside a valid encrypted blob reference, so give
    // the core download action one chance to materialize the safe cache copy.
    let mut served = match read_cached_attachment(cache_root.clone(), initial_path).await {
        Ok(served) => served,
        Err(HostedDeviceError::UnsafeAttachmentPath) => None,
        Err(error) => return Err(error),
    };
    if served.is_none() {
        let action = AppAction::DownloadAttachment {
            room_id: room_id.clone(),
            message_id: message_id.clone(),
            attachment_id: attachment_id.clone(),
        };
        let download_runtime = Arc::clone(&runtime);
        let next = tokio::task::spawn_blocking(move || download_runtime.dispatch_and_wait(action))
            .await
            .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
        attachment = attachment_from_state(&next, &room_id, &message_id, &attachment_id)
            .ok_or(HostedDeviceError::AttachmentUnavailable)?;
        served = read_cached_attachment(cache_root, attachment.local_path.clone()).await?;
    }
    let bytes = served.ok_or(HostedDeviceError::AttachmentUnavailable)?;
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
) -> Result<Option<Vec<u8>>, HostedDeviceError> {
    let Some(local_path) = local_path else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || {
        let cache_root = match fs::canonicalize(cache_root) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(HostedDeviceError::AttachmentUnavailable),
        };
        let path = match fs::canonicalize(local_path) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(HostedDeviceError::AttachmentUnavailable),
        };
        if !path.starts_with(&cache_root) {
            return Err(HostedDeviceError::UnsafeAttachmentPath);
        }
        let metadata = fs::metadata(&path).map_err(|_| HostedDeviceError::AttachmentUnavailable)?;
        if !metadata.is_file() {
            return Err(HostedDeviceError::AttachmentUnavailable);
        }
        if metadata.len() > MAX_HOSTED_ATTACHMENT_BYTES as u64 {
            return Err(HostedDeviceError::PayloadTooLarge(
                "cached attachment exceeds the serving limit".to_owned(),
            ));
        }
        fs::read(path)
            .map(Some)
            .map_err(|_| HostedDeviceError::AttachmentUnavailable)
    })
    .await
    .map_err(|error| HostedDeviceError::Task(error.to_string()))?
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
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "audio/mpeg"
            | "audio/mp4"
            | "audio/ogg"
            | "audio/wav"
            | "audio/webm"
            | "video/mp4"
            | "video/ogg"
            | "video/webm"
            | "video/quicktime"
    )
}

async fn app_updates(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    let runtime = state.runtime_for(&user_id)?;
    // Flush a state event immediately. Waiting for the first remote update (or
    // the SSE keepalive interval) makes a healthy idle Device look disconnected
    // every time the dashboard opens or reconnects.
    let initial = state_event(&redacted_state(runtime.state()?)).unwrap_or_else(error_event);
    let initial_stream = futures_util::stream::once(async move { Ok(initial) });
    let stream = futures_util::stream::unfold(runtime, |runtime| async move {
        let next_runtime = Arc::clone(&runtime);
        let update = tokio::task::spawn_blocking(move || {
            next_runtime
                .wait_for_update(DEFAULT_UPDATE_TIMEOUT_MILLIS)
                .or_else(|_| next_runtime.state())
        })
        .await;
        let event = match update {
            Ok(Ok(state)) => state_event(&redacted_state(state)).unwrap_or_else(error_event),
            Ok(Err(error)) => error_event(error),
            Err(error) => error_event(error),
        };
        Some((Ok(event), runtime))
    });
    Ok(Sse::new(initial_stream.chain(stream)).keep_alive(KeepAlive::default()))
}

fn authorized_user(
    state: &HostedDeviceState,
    headers: &HeaderMap,
) -> Result<String, HostedDeviceError> {
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(HostedDeviceError::Unauthorized)?;
    let supplied = authorization
        .strip_prefix("Bearer ")
        .ok_or(HostedDeviceError::Unauthorized)?;
    if !constant_time_eq(supplied.as_bytes(), state.config.api_token.as_bytes()) {
        return Err(HostedDeviceError::Unauthorized);
    }
    let user_id = headers
        .get(WORKOS_USER_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(HostedDeviceError::MissingUser)?
        .trim();
    if user_id.is_empty()
        || user_id.len() > MAX_USER_ID_BYTES
        || user_id.chars().any(char::is_control)
    {
        return Err(HostedDeviceError::InvalidUser);
    }
    Ok(user_id.to_owned())
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

fn user_storage_id(user_id: &str) -> String {
    let digest = Sha256::digest(user_id.as_bytes());
    hex::encode(digest)
}

fn redacted_state(mut state: AppState) -> AppState {
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
    state
}

fn state_event(state: &AppState) -> Result<Event, serde_json::Error> {
    Ok(Event::default()
        .event("state")
        .id(state.rev.to_string())
        .data(serde_json::to_string(state)?))
}

fn error_event(error: impl ToString) -> Event {
    Event::default()
        .event("error")
        .data(json!({ "error": error.to_string() }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn attachment_reader_confines_files_to_the_users_cache() {
        let root = TempDir::new().unwrap();
        let cache = root.path().join("chat/attachments");
        fs::create_dir_all(&cache).unwrap();
        let inside = cache.join("inside.bin");
        fs::write(&inside, b"inside").unwrap();
        assert_eq!(
            read_cached_attachment(cache.clone(), Some(inside.to_string_lossy().into_owned()))
                .await
                .unwrap(),
            Some(b"inside".to_vec())
        );

        let outside = root.path().join("outside.bin");
        fs::write(&outside, b"outside").unwrap();
        assert!(matches!(
            read_cached_attachment(cache, Some(outside.to_string_lossy().into_owned())).await,
            Err(HostedDeviceError::UnsafeAttachmentPath)
        ));
    }
}
