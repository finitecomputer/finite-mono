use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, FromRequest, Multipart, Path as AxumPath, Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, HeaderValue,
};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use finite_brain_core::{
    BRAIN_IDENTITY_PROVIDER_VERSION, BrainEventAuthorizationIntent, BrainGrantIntent,
    BrainHttpAuthorizationIntent, FolderId, FolderKey, IssuedFolderKeyGrant,
    PERSONAL_VAULT_BOOTSTRAP_MAX_TTL_SECONDS, VaultId, issue_folder_key_grant,
    issue_personal_vault_bootstrap_authorization_event, sign_brain_event_template,
    validate_brain_event_authorization_intent, validate_brain_grant_intent,
    validate_brain_http_authorization_intent,
};
use finite_identity::{FiniteIdentity, IdentityPaths};
use finite_nostr::{NostrPublicKey, decrypt_nip44, encrypt_nip44};
use finitechat_core::device_link::{
    DEVICE_LINK_MAX_TTL_SECONDS, DeviceLinkEncryptInput, encrypt_device_link_payload,
};
use finitechat_core::{
    AppAction, AppProfileChatBootstrapInput, AppProfileChatBootstrapPreparedCommit, AppState,
    AppTopicSummary, ChatMediaAttachment, ChatMediaKind, FiniteChatCoreError, FiniteChatRuntime,
    OpenOptions, OutboundAttachment, account_id_from_npub,
};
use finitechat_http::{
    ExpireLinkSessionRequest, ExpireLinkSessionResponse, GetLinkSessionRequest,
    HttpLinkSessionRecord, HttpLinkSessionState, UploadLinkPayloadRequest,
};
use finitechat_proto::{
    ClaimKeyPackageResult, CreateRoomRequest, DecryptedApplicationEventV1, DurableAppEventKind,
    RuntimeCommandJsonPayloadV1, RuntimeCommandPayloadKindV1, RuntimeCommandRequestV1,
    RuntimeCommandResultV1, RuntimeCommandTargetV1, RuntimeCommandTerminalStatusV1,
};
use futures_util::{Stream, StreamExt};
use nostr::{Keys, SecretKey};
use openmls::prelude::{AeadType, OpenMlsCrypto, OpenMlsProvider, OpenMlsRand};
use openmls_rust_crypto::OpenMlsRustCrypto;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

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
const MAX_RUNTIME_COMMAND_WAIT_MILLIS: u64 = 60_000;
const RECENT_RUNTIME_EVENT_LIMIT: u32 = 512;
const OWNER_CLAIM_EVENT_LIMIT: u32 = 5_000;
const OWNER_CLAIM_COMMAND: &str = "agent.owner.claim";
pub const MAX_HOSTED_PROFILE_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const DEVICE_LINK_RECORD_VERSION: u16 = 1;
const DEVICE_LINK_CREATED_BY: &str = "finitechat-hosted-device";
const DEVICE_LINK_HTTP_TIMEOUT_SECS: u64 = 10;
const MAX_DEVICE_LINK_RECORD_BYTES: u64 = 64 * 1024;
const MAX_DEVICE_LINK_HTTP_RESPONSE_BYTES: u64 = 64 * 1024;
const MAX_DEVICE_LINK_REQUEST_BYTES: usize = 4 * 1024;
const AGENT_BINDING_VERSION: u16 = 1;
const AGENT_BINDING_NONCE_BYTES: usize = 12;
const MAX_AGENT_BINDING_REQUEST_BYTES: usize = 8 * 1024;
const MAX_BRAIN_BOOTSTRAP_AUTHORIZATION_REQUEST_BYTES: usize = 8 * 1024;
const MAX_BRAIN_IDENTITY_PROVIDER_REQUEST_BYTES: usize = 1024 * 1024;
const BRAIN_PUBLIC_ORIGIN_HEADER: &str = "x-finite-brain-public-origin";
// A prepared one-member MLS commit can contain a Welcome, ratchet tree, and
// envelope near their protocol ceilings. The sealed JSON v1 representation
// encodes byte vectors as integer arrays, so its bounded disk form needs more
// room than the 64 KiB device-link records.
const MAX_AGENT_BINDING_RECORD_BYTES: usize = 64 * 1024 * 1024;
const AGENT_BINDING_KEY_DOMAIN: &[u8] = b"finitechat.hosted-agent-binding-key.v1";
const AGENT_BINDING_AAD_DOMAIN: &[u8] = b"finitechat.hosted-agent-binding.v1";
const AGENT_BINDING_BOOTSTRAP_AAD_DOMAIN: &[u8] = b"finitechat.hosted-agent-binding-bootstrap.v1";
const AGENT_BINDING_AUTHORIZATION_AAD_DOMAIN: &[u8] =
    b"finitechat.hosted-agent-binding-authorization.v1";

#[derive(Clone, Debug)]
pub struct HostedDeviceConfig {
    pub data_root: PathBuf,
    /// URL used by this process for chat and link-service HTTP transport.
    pub server_url: String,
    /// Canonical public chat server identity bound into encrypted device links.
    pub public_url: String,
    pub api_token: String,
}

#[derive(Clone)]
struct HostedDeviceState {
    config: HostedDeviceConfig,
    runtimes: Arc<Mutex<HashMap<String, Arc<FiniteChatRuntime>>>>,
    device_links: Arc<Mutex<()>>,
    agent_binding_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    fail_final_agent_binding_persists: Arc<AtomicUsize>,
    fail_profile_bootstrap_room_creates_after_server_acceptance: Arc<AtomicUsize>,
    fail_profile_bootstrap_submits_after_device_save: Arc<AtomicUsize>,
    fixed_device_link_now_unix_seconds: Option<u64>,
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

    fn device_link_path(&self, user_id: &str, link_session_id: &str) -> PathBuf {
        let digest = Sha256::digest(link_session_id.as_bytes());
        self.user_root(user_id)
            .join("device-links")
            .join(format!("{}.json", hex::encode(digest)))
    }

    fn agent_binding_path(&self, user_id: &str, project_id: &str) -> PathBuf {
        let digest = Sha256::digest(project_id.as_bytes());
        self.user_root(user_id)
            .join("agent-bindings")
            .join(format!("{}.json", hex::encode(digest)))
    }

    fn agent_binding_bootstrap_path(&self, user_id: &str, project_id: &str) -> PathBuf {
        self.agent_binding_path(user_id, project_id)
            .with_extension("bootstrap.json")
    }

    fn agent_binding_authorization_path(&self, user_id: &str, project_id: &str) -> PathBuf {
        self.agent_binding_path(user_id, project_id)
            .with_extension("authorization.json")
    }

    fn agent_binding_lock(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> Result<Arc<Mutex<()>>, HostedDeviceError> {
        let key = format!(
            "{}:{}",
            user_storage_id(user_id),
            hex::encode(Sha256::digest(project_id))
        );
        let mut locks = self
            .agent_binding_locks
            .lock()
            .map_err(|_| HostedDeviceError::LockPoisoned)?;
        Ok(Arc::clone(
            locks.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))),
        ))
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
        let chat_data = user_root.join("chat");
        let identity_exists = path_exists(&identity_paths.identity_file())?;
        let store_exists = path_exists(&chat_data.join("client.sqlite3"))?;
        if identity_exists != store_exists {
            return Err(HostedDeviceError::IncompleteUserState);
        }
        let identity = if identity_exists {
            FiniteIdentity::load(&identity_paths)?
        } else {
            FiniteIdentity::load_or_generate(&identity_paths, CREATED_BY)?
        };
        let account_secret_hex = hex::encode(identity.expose_secret_bytes());
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
    #[error("device link was not found")]
    DeviceLinkNotFound,
    #[error("invalid device link: {0}")]
    InvalidDeviceLink(String),
    #[error("device link conflict: {0}")]
    DeviceLinkConflict(String),
    #[error("Finite Chat link service is unavailable: {0}")]
    DeviceLinkService(String),
    #[error("hosted device runtime cache lock poisoned")]
    LockPoisoned,
    #[error("hosted chat state is incomplete; recovery is required")]
    IncompleteUserState,
    #[error("canonical Agent conversation is not bound; recovery is required")]
    AgentBindingNotFound,
    #[error("canonical Agent conversation requires recovery: {0}")]
    AgentBindingInvalid(String),
    #[error("invalid Brain bootstrap authorization request: {0}")]
    InvalidBrainBootstrapAuthorization(String),
    #[error("Finite Chat Hosted Device setup is required before opening Brain")]
    BrainIdentitySetupRequired,
    #[error("invalid Brain Identity Provider request: {0}")]
    InvalidBrainIdentityProvider(String),
    #[error("new chat must stay in the canonical Agent conversation: {0}")]
    CanonicalChatConflict(String),
    #[error("hosted chat state could not be inspected: {0}")]
    Io(#[from] std::io::Error),
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
            Self::MissingUser
            | Self::InvalidUser
            | Self::InvalidMultipart(_)
            | Self::InvalidBrainBootstrapAuthorization(_)
            | Self::InvalidBrainIdentityProvider(_) => StatusCode::BAD_REQUEST,
            Self::BrainIdentitySetupRequired => StatusCode::PRECONDITION_REQUIRED,
            Self::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::AttachmentNotFound | Self::DeviceLinkNotFound | Self::AgentBindingNotFound => {
                StatusCode::NOT_FOUND
            }
            Self::DeviceLinkConflict(_) | Self::CanonicalChatConflict(_) => StatusCode::CONFLICT,
            Self::InvalidDeviceLink(_) => StatusCode::BAD_REQUEST,
            Self::DeviceLinkService(_) => StatusCode::BAD_GATEWAY,
            Self::AttachmentUnavailable => StatusCode::BAD_GATEWAY,
            Self::IncompleteUserState | Self::AgentBindingInvalid(_) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            Self::Core(FiniteChatCoreError::Client { .. }) => StatusCode::BAD_REQUEST,
            Self::Core(FiniteChatCoreError::Profile { .. }) => StatusCode::BAD_REQUEST,
            Self::Core(FiniteChatCoreError::ServerRejected { .. })
            | Self::Core(FiniteChatCoreError::Delivery { .. }) => StatusCode::BAD_GATEWAY,
            Self::Core(_)
            | Self::Identity(_)
            | Self::Serialize(_)
            | Self::UnsafeAttachmentPath
            | Self::LockPoisoned
            | Self::Io(_)
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
    app_with_test_options(config, None, 0, 0, 0)
}

/// Test seam for exercising expiry and restart behavior without sleeping.
/// Production always calls [`app`] and uses the system clock.
#[doc(hidden)]
pub fn app_with_fixed_device_link_now(config: HostedDeviceConfig, now_unix_seconds: u64) -> Router {
    app_with_test_options(config, Some(now_unix_seconds), 0, 0, 0)
}

/// Test seam for proving recovery when the final binding write fails after its
/// intended Room has been created. Production always calls [`app`].
#[doc(hidden)]
pub fn app_with_final_agent_binding_persist_failures(
    config: HostedDeviceConfig,
    failure_count: usize,
) -> Router {
    app_with_test_options(config, None, failure_count, 0, 0)
}

/// Test seam for proving recovery after the server accepted the exact Room
/// create request but before the Device saved the matching local MLS group.
#[doc(hidden)]
pub fn app_with_profile_bootstrap_room_create_failures(
    config: HostedDeviceConfig,
    failure_count: usize,
) -> Router {
    app_with_test_options(config, None, 0, failure_count, 0)
}

/// Test seam for proving recovery after pending MLS state is durable but the
/// exact prepared add-member request has not yet been submitted.
#[doc(hidden)]
pub fn app_with_profile_bootstrap_submit_failures(
    config: HostedDeviceConfig,
    failure_count: usize,
) -> Router {
    app_with_test_options(config, None, 0, 0, failure_count)
}

fn app_with_test_options(
    config: HostedDeviceConfig,
    fixed_device_link_now_unix_seconds: Option<u64>,
    fail_final_agent_binding_persists: usize,
    fail_profile_bootstrap_room_creates_after_server_acceptance: usize,
    fail_profile_bootstrap_submits_after_device_save: usize,
) -> Router {
    let state = HostedDeviceState {
        config,
        runtimes: Arc::new(Mutex::new(HashMap::new())),
        device_links: Arc::new(Mutex::new(())),
        agent_binding_locks: Arc::new(Mutex::new(HashMap::new())),
        fail_final_agent_binding_persists: Arc::new(AtomicUsize::new(
            fail_final_agent_binding_persists,
        )),
        fail_profile_bootstrap_room_creates_after_server_acceptance: Arc::new(AtomicUsize::new(
            fail_profile_bootstrap_room_creates_after_server_acceptance,
        )),
        fail_profile_bootstrap_submits_after_device_save: Arc::new(AtomicUsize::new(
            fail_profile_bootstrap_submits_after_device_save,
        )),
        fixed_device_link_now_unix_seconds,
    };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/app/state", get(app_state))
        .route("/v1/app/actions", post(dispatch_action))
        .route("/v1/app/new-chat", post(start_new_chat))
        .route(
            "/v1/app/agent-bindings/open",
            post(open_agent_binding).layer(DefaultBodyLimit::max(MAX_AGENT_BINDING_REQUEST_BYTES)),
        )
        .route(
            "/v1/app/agent-bindings/ensure",
            post(ensure_agent_binding)
                .layer(DefaultBodyLimit::max(MAX_AGENT_BINDING_REQUEST_BYTES)),
        )
        .route(
            "/v1/app/agent-bindings/authorize-bootstrap",
            post(authorize_agent_binding_bootstrap)
                .layer(DefaultBodyLimit::max(MAX_AGENT_BINDING_REQUEST_BYTES)),
        )
        .route(
            "/v1/brain/personal-vault-bootstrap-authorizations",
            post(issue_personal_vault_bootstrap_authorization).layer(DefaultBodyLimit::max(
                MAX_BRAIN_BOOTSTRAP_AUTHORIZATION_REQUEST_BYTES,
            )),
        )
        .route(
            "/v1/brain/identity-provider",
            post(execute_brain_identity_provider_operation).layer(DefaultBodyLimit::max(
                MAX_BRAIN_IDENTITY_PROVIDER_REQUEST_BYTES,
            )),
        )
        .route(
            "/v1/app/images",
            post(upload_profile_image).layer(DefaultBodyLimit::max(MAX_HOSTED_PROFILE_IMAGE_BYTES)),
        )
        .route("/v1/app/runtime-commands", post(dispatch_runtime_command))
        .route("/v1/app/updates", get(app_updates))
        .route(
            "/v1/device-links/approve",
            post(approve_device_link).layer(DefaultBodyLimit::max(MAX_DEVICE_LINK_REQUEST_BYTES)),
        )
        .route(
            "/v1/device-links/status",
            post(device_link_status).layer(DefaultBodyLimit::max(MAX_DEVICE_LINK_REQUEST_BYTES)),
        )
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

#[derive(Clone, Deserialize)]
struct DeviceLinkRequest {
    link_session_id: String,
    target_device_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct PendingDeviceLinkV1 {
    version: u16,
    link_session_id: String,
    target_device_id: String,
    pairing_public_key: String,
    account_id: String,
    server_url: String,
    issued_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
    encrypted_payload: Vec<u8>,
    fanout_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DeviceLinkStatusKind {
    AwaitingClaim,
    AwaitingKeyPackage,
    JoiningRooms,
    Ready,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct DeviceLinkResponse {
    link_session_id: String,
    target_device_id: String,
    status: DeviceLinkStatusKind,
    expires_at_unix_seconds: u64,
    room_count: u32,
    active_room_count: u32,
}

async fn approve_device_link(
    State(state): State<HostedDeviceState>,
    request: Request,
) -> Result<Json<DeviceLinkResponse>, HostedDeviceError> {
    let (user_id, input) = authenticated_device_link_request(&state, request).await?;
    let response =
        tokio::task::spawn_blocking(move || approve_device_link_for_user(&state, &user_id, input))
            .await
            .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(response))
}

async fn device_link_status(
    State(state): State<HostedDeviceState>,
    request: Request,
) -> Result<Json<DeviceLinkResponse>, HostedDeviceError> {
    let (user_id, input) = authenticated_device_link_request(&state, request).await?;
    let response = tokio::task::spawn_blocking(move || {
        let _guard = state
            .device_links
            .lock()
            .map_err(|_| HostedDeviceError::LockPoisoned)?;
        validate_device_link_request(&input)?;
        let pending = load_pending_device_link(&state, &user_id, &input.link_session_id)?
            .ok_or(HostedDeviceError::DeviceLinkNotFound)?;
        if pending.target_device_id != input.target_device_id {
            return Err(HostedDeviceError::DeviceLinkNotFound);
        }
        reconcile_device_link(&state, &user_id, pending)
    })
    .await
    .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(response))
}

async fn authenticated_device_link_request(
    state: &HostedDeviceState,
    request: Request,
) -> Result<(String, DeviceLinkRequest), HostedDeviceError> {
    // Authenticate before Axum parses or buffers the JSON body. Only the
    // dashboard's authenticated internal call may reach pairing work.
    let user_id = authorized_user(state, request.headers())?;
    let Json(input) = Json::<DeviceLinkRequest>::from_request(request, state)
        .await
        .map_err(|error| {
            if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                HostedDeviceError::PayloadTooLarge(
                    "device-link request exceeds its 4 KiB limit".to_owned(),
                )
            } else {
                HostedDeviceError::InvalidDeviceLink("request body must be valid JSON".to_owned())
            }
        })?;
    Ok((user_id, input))
}

fn approve_device_link_for_user(
    state: &HostedDeviceState,
    user_id: &str,
    input: DeviceLinkRequest,
) -> Result<DeviceLinkResponse, HostedDeviceError> {
    let _guard = state
        .device_links
        .lock()
        .map_err(|_| HostedDeviceError::LockPoisoned)?;
    validate_device_link_request(&input)?;

    if let Some(pending) = load_pending_device_link(state, user_id, &input.link_session_id)? {
        if pending.target_device_id != input.target_device_id {
            return Err(HostedDeviceError::DeviceLinkNotFound);
        }
        return reconcile_device_link(state, user_id, pending);
    }

    let session = get_link_session(state, &input.link_session_id)?
        .ok_or(HostedDeviceError::DeviceLinkNotFound)?;
    // A session which another account already approved must not become an
    // account-discovery oracle. Only its original per-user pending record may
    // resume it.
    if session.state != HttpLinkSessionState::Created {
        return Err(HostedDeviceError::DeviceLinkNotFound);
    }
    if session.link_session_id != input.link_session_id {
        return Err(HostedDeviceError::DeviceLinkConflict(
            "link service returned a different session".to_owned(),
        ));
    }

    let runtime = state.runtime_for(user_id)?;
    let identity = runtime.state()?.identity;
    let now = device_link_now(state)?;
    let expires_at_unix_seconds =
        now.checked_add(DEVICE_LINK_MAX_TTL_SECONDS)
            .ok_or_else(|| {
                HostedDeviceError::InvalidDeviceLink(
                    "expiry is outside the supported range".to_owned(),
                )
            })?;
    let public_url = normalized_link_server_url(&state.config.public_url)?;
    let encrypted_payload = encrypt_device_link_payload(DeviceLinkEncryptInput {
        account_secret_hex: identity.account_secret_hex,
        pairing_public_key: session.pairing_public_key.clone(),
        link_session_id: input.link_session_id.clone(),
        target_device_id: input.target_device_id.clone(),
        server_url: public_url.clone(),
        issued_at_unix_seconds: now,
        expires_at_unix_seconds,
    })?;
    let digest = Sha256::digest(
        format!(
            "{DEVICE_LINK_CREATED_BY}\0{user_id}\0{}",
            input.link_session_id
        )
        .as_bytes(),
    );
    let pending = PendingDeviceLinkV1 {
        version: DEVICE_LINK_RECORD_VERSION,
        link_session_id: input.link_session_id,
        target_device_id: input.target_device_id,
        pairing_public_key: session.pairing_public_key,
        account_id: identity.account_id,
        server_url: public_url,
        issued_at_unix_seconds: now,
        expires_at_unix_seconds,
        encrypted_payload,
        fanout_id: format!("device-link-{}", &hex::encode(digest)[..40]),
    };
    let pending = persist_pending_device_link(state, user_id, &pending)?;
    reconcile_device_link(state, user_id, pending)
}

fn reconcile_device_link(
    state: &HostedDeviceState,
    user_id: &str,
    pending: PendingDeviceLinkV1,
) -> Result<DeviceLinkResponse, HostedDeviceError> {
    validate_pending_device_link(state, &pending)?;
    let runtime = state.runtime_for(user_id)?;
    if runtime.state()?.identity.account_id != pending.account_id {
        return Err(HostedDeviceError::DeviceLinkConflict(
            "the approving account no longer matches this request".to_owned(),
        ));
    }

    if device_link_now(state)? > pending.expires_at_unix_seconds {
        let _ = expire_link_session(state, &pending.link_session_id);
        return Ok(device_link_response(
            &pending,
            DeviceLinkStatusKind::Expired,
            0,
            0,
        ));
    }

    let mut session = get_link_session(state, &pending.link_session_id)?
        .ok_or(HostedDeviceError::DeviceLinkNotFound)?;
    if session.link_session_id != pending.link_session_id
        || session.pairing_public_key != pending.pairing_public_key
    {
        return Err(HostedDeviceError::DeviceLinkConflict(
            "link session no longer matches the approved Device".to_owned(),
        ));
    }
    match session.state {
        HttpLinkSessionState::Created if session.encrypted_payload.is_some() => {
            return Err(HostedDeviceError::DeviceLinkConflict(
                "link session contains an unexpected payload".to_owned(),
            ));
        }
        HttpLinkSessionState::PayloadUploaded
        | HttpLinkSessionState::Claimed
        | HttpLinkSessionState::Delivered
            if session.encrypted_payload.as_deref()
                != Some(pending.encrypted_payload.as_slice()) =>
        {
            return Err(HostedDeviceError::DeviceLinkConflict(
                "link session payload no longer matches this approval".to_owned(),
            ));
        }
        _ => {}
    }

    if session.state == HttpLinkSessionState::Created {
        session = link_service_post(
            state,
            "/link-sessions/payload",
            &UploadLinkPayloadRequest {
                link_session_id: pending.link_session_id.clone(),
                encrypted_payload: pending.encrypted_payload.clone(),
            },
        )?;
    }

    match session.state {
        HttpLinkSessionState::Created | HttpLinkSessionState::PayloadUploaded => Ok(
            device_link_response(&pending, DeviceLinkStatusKind::AwaitingClaim, 0, 0),
        ),
        HttpLinkSessionState::Claimed => Ok(device_link_response(
            &pending,
            DeviceLinkStatusKind::AwaitingKeyPackage,
            0,
            0,
        )),
        HttpLinkSessionState::Delivered => {
            let report = runtime.link_device_and_wait(
                pending.fanout_id.clone(),
                pending.target_device_id.clone(),
            )?;
            let status = if report.fanout_complete && report.room_count == report.active_room_count
            {
                DeviceLinkStatusKind::Ready
            } else if report.room_count == 0 && !report.fanout_complete {
                DeviceLinkStatusKind::AwaitingKeyPackage
            } else {
                DeviceLinkStatusKind::JoiningRooms
            };
            Ok(device_link_response(
                &pending,
                status,
                report.room_count,
                report.active_room_count,
            ))
        }
        HttpLinkSessionState::Expired => Ok(device_link_response(
            &pending,
            DeviceLinkStatusKind::Expired,
            0,
            0,
        )),
    }
}

fn device_link_response(
    pending: &PendingDeviceLinkV1,
    status: DeviceLinkStatusKind,
    room_count: u32,
    active_room_count: u32,
) -> DeviceLinkResponse {
    DeviceLinkResponse {
        link_session_id: pending.link_session_id.clone(),
        target_device_id: pending.target_device_id.clone(),
        status,
        expires_at_unix_seconds: pending.expires_at_unix_seconds,
        room_count,
        active_room_count,
    }
}

fn validate_device_link_request(input: &DeviceLinkRequest) -> Result<(), HostedDeviceError> {
    // The crypto helper is the canonical validator. A fixed dummy signer and
    // pairing key would be wasteful here, so enforce its public token limits
    // before looking up any server state and let encryption re-check them.
    for (field, value) in [
        ("link session id", input.link_session_id.as_str()),
        ("target Device id", input.target_device_id.as_str()),
    ] {
        if value.is_empty()
            || value.len() > 256
            || value.trim() != value
            || value.chars().any(char::is_control)
        {
            return Err(HostedDeviceError::InvalidDeviceLink(format!(
                "{field} is invalid"
            )));
        }
    }
    if input.target_device_id == "hosted-web" {
        return Err(HostedDeviceError::InvalidDeviceLink(
            "target Device must be distinct from the Hosted Web Device".to_owned(),
        ));
    }
    Ok(())
}

fn validate_pending_device_link(
    state: &HostedDeviceState,
    pending: &PendingDeviceLinkV1,
) -> Result<(), HostedDeviceError> {
    if pending.version != DEVICE_LINK_RECORD_VERSION {
        return Err(HostedDeviceError::InvalidDeviceLink(
            "pending record version is unsupported".to_owned(),
        ));
    }
    validate_device_link_request(&DeviceLinkRequest {
        link_session_id: pending.link_session_id.clone(),
        target_device_id: pending.target_device_id.clone(),
    })?;
    if pending.pairing_public_key.is_empty()
        || pending.pairing_public_key.len() > 256
        || pending.account_id.is_empty()
        || pending.account_id.len() > 256
        || pending.encrypted_payload.is_empty()
        || pending.encrypted_payload.len() > 16 * 1024
        || pending.fanout_id.is_empty()
        || pending.fanout_id.len() > 256
        || pending.expires_at_unix_seconds <= pending.issued_at_unix_seconds
        || pending
            .expires_at_unix_seconds
            .saturating_sub(pending.issued_at_unix_seconds)
            > DEVICE_LINK_MAX_TTL_SECONDS
        || pending.server_url != normalized_link_server_url(&state.config.public_url)?
    {
        return Err(HostedDeviceError::InvalidDeviceLink(
            "pending record is malformed".to_owned(),
        ));
    }
    Ok(())
}

fn persist_pending_device_link(
    state: &HostedDeviceState,
    user_id: &str,
    pending: &PendingDeviceLinkV1,
) -> Result<PendingDeviceLinkV1, HostedDeviceError> {
    let path = state.device_link_path(user_id, &pending.link_session_id);
    if path.exists() {
        let existing = load_pending_device_link(state, user_id, &pending.link_session_id)?
            .ok_or(HostedDeviceError::DeviceLinkNotFound)?;
        if same_device_link_binding(&existing, pending) {
            return Ok(existing);
        }
        return Err(HostedDeviceError::DeviceLinkConflict(
            "link session is already bound to another Device".to_owned(),
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        HostedDeviceError::Task("device-link record has no parent directory".to_owned())
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        HostedDeviceError::Task(format!("could not create device-link directory: {error}"))
    })?;
    let encoded = serde_json::to_vec(pending)?;
    if encoded.len() as u64 > MAX_DEVICE_LINK_RECORD_BYTES {
        return Err(HostedDeviceError::InvalidDeviceLink(
            "pending record is too large".to_owned(),
        ));
    }
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|error| {
        HostedDeviceError::Task(format!(
            "device-link record nonce generation failed: {error}"
        ))
    })?;
    let temporary = parent.join(format!(".pending-{}.tmp", hex::encode(entropy)));
    let write_result = (|| -> Result<PendingDeviceLinkV1, HostedDeviceError> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|error| {
            HostedDeviceError::Task(format!("could not create device-link record: {error}"))
        })?;
        file.write_all(&encoded).map_err(|error| {
            HostedDeviceError::Task(format!("could not write device-link record: {error}"))
        })?;
        file.sync_all().map_err(|error| {
            HostedDeviceError::Task(format!("could not sync device-link record: {error}"))
        })?;
        match fs::hard_link(&temporary, &path) {
            Ok(()) => {
                fs::remove_file(&temporary).map_err(|error| {
                    HostedDeviceError::Task(format!(
                        "could not remove staged device-link record: {error}"
                    ))
                })?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&temporary);
                let existing = load_pending_device_link(state, user_id, &pending.link_session_id)?
                    .ok_or(HostedDeviceError::DeviceLinkNotFound)?;
                if same_device_link_binding(&existing, pending) {
                    return Ok(existing);
                }
                return Err(HostedDeviceError::DeviceLinkConflict(
                    "link session is already bound to another Device".to_owned(),
                ));
            }
            Err(error) => {
                return Err(HostedDeviceError::Task(format!(
                    "could not install device-link record: {error}"
                )));
            }
        }
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                HostedDeviceError::Task(format!("could not sync device-link directory: {error}"))
            })?;
        Ok(pending.clone())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn same_device_link_binding(left: &PendingDeviceLinkV1, right: &PendingDeviceLinkV1) -> bool {
    left.version == right.version
        && left.link_session_id == right.link_session_id
        && left.target_device_id == right.target_device_id
        && left.pairing_public_key == right.pairing_public_key
        && left.account_id == right.account_id
        && left.server_url == right.server_url
}

fn load_pending_device_link(
    state: &HostedDeviceState,
    user_id: &str,
    link_session_id: &str,
) -> Result<Option<PendingDeviceLinkV1>, HostedDeviceError> {
    let path = state.device_link_path(user_id, link_session_id);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(HostedDeviceError::Task(format!(
                "could not inspect device-link record: {error}"
            )));
        }
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_DEVICE_LINK_RECORD_BYTES {
        return Err(HostedDeviceError::InvalidDeviceLink(
            "pending record is not a safe regular file".to_owned(),
        ));
    }
    let encoded = fs::read(path).map_err(|error| {
        HostedDeviceError::Task(format!("could not read device-link record: {error}"))
    })?;
    let pending: PendingDeviceLinkV1 = serde_json::from_slice(&encoded)?;
    if pending.link_session_id != link_session_id {
        return Err(HostedDeviceError::InvalidDeviceLink(
            "pending record is bound to another session".to_owned(),
        ));
    }
    validate_pending_device_link(state, &pending)?;
    Ok(Some(pending))
}

fn get_link_session(
    state: &HostedDeviceState,
    link_session_id: &str,
) -> Result<Option<HttpLinkSessionRecord>, HostedDeviceError> {
    link_service_post(
        state,
        "/link-sessions/get",
        &GetLinkSessionRequest {
            link_session_id: link_session_id.to_owned(),
        },
    )
}

fn expire_link_session(
    state: &HostedDeviceState,
    link_session_id: &str,
) -> Result<ExpireLinkSessionResponse, HostedDeviceError> {
    link_service_post(
        state,
        "/link-sessions/expire",
        &ExpireLinkSessionRequest {
            link_session_id: link_session_id.to_owned(),
        },
    )
}

fn link_service_post<I: Serialize, O: DeserializeOwned>(
    state: &HostedDeviceState,
    path: &str,
    input: &I,
) -> Result<O, HostedDeviceError> {
    let base_url = normalized_link_server_url(&state.config.server_url)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(DEVICE_LINK_HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|_| {
            HostedDeviceError::DeviceLinkService(
                "link service HTTP client could not be created".to_owned(),
            )
        })?;
    let mut response = client
        .post(format!("{base_url}{path}"))
        .json(input)
        .send()
        .map_err(|_| {
            HostedDeviceError::DeviceLinkService("link service request failed".to_owned())
        })?;
    if !response.status().is_success() {
        return Err(HostedDeviceError::DeviceLinkService(format!(
            "link service returned HTTP {}",
            response.status().as_u16()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DEVICE_LINK_HTTP_RESPONSE_BYTES)
    {
        return Err(HostedDeviceError::DeviceLinkService(
            "link service response is too large".to_owned(),
        ));
    }
    let mut encoded = Vec::new();
    response
        .by_ref()
        .take(MAX_DEVICE_LINK_HTTP_RESPONSE_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|_| {
            HostedDeviceError::DeviceLinkService(
                "link service response could not be read".to_owned(),
            )
        })?;
    if encoded.len() as u64 > MAX_DEVICE_LINK_HTTP_RESPONSE_BYTES {
        return Err(HostedDeviceError::DeviceLinkService(
            "link service response is too large".to_owned(),
        ));
    }
    serde_json::from_slice(&encoded).map_err(|_| {
        HostedDeviceError::DeviceLinkService("link service returned invalid JSON".to_owned())
    })
}

fn normalized_link_server_url(value: &str) -> Result<String, HostedDeviceError> {
    let parsed = reqwest::Url::parse(value).map_err(|_| {
        HostedDeviceError::InvalidDeviceLink("chat server URL is invalid".to_owned())
    })?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(HostedDeviceError::InvalidDeviceLink(
            "chat server URL is invalid".to_owned(),
        ));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

fn device_link_now(state: &HostedDeviceState) -> Result<u64, HostedDeviceError> {
    if let Some(now) = state.fixed_device_link_now_unix_seconds {
        return Ok(now);
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| HostedDeviceError::Task(format!("system clock is invalid: {error}")))
}

#[derive(Debug, Deserialize)]
struct HostedRuntimeCommandRequest {
    room_id: String,
    #[serde(default)]
    conversation_id: Option<String>,
    target_account_id: String,
    command: String,
    #[serde(default)]
    resource_key: Option<String>,
    schema: String,
    body: Value,
    #[serde(default)]
    reuse_succeeded_owner_claim: bool,
    #[serde(default = "default_runtime_command_wait_millis")]
    wait_millis: u64,
}

#[derive(Debug, Serialize)]
struct HostedRuntimeCommandResponse {
    request_id: String,
    status: RuntimeCommandTerminalStatusV1,
    body: Option<Value>,
    error: Option<finitechat_proto::RuntimeCommandErrorV1>,
}

async fn dispatch_runtime_command(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
    Json(input): Json<HostedRuntimeCommandRequest>,
) -> Result<Json<HostedRuntimeCommandResponse>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    let runtime = state.runtime_for(&user_id)?;
    let response = tokio::task::spawn_blocking(move || send_runtime_command(&runtime, input))
        .await
        .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(response))
}

fn send_runtime_command(
    runtime: &FiniteChatRuntime,
    input: HostedRuntimeCommandRequest,
) -> Result<HostedRuntimeCommandResponse, HostedDeviceError> {
    if input.reuse_succeeded_owner_claim
        && input.command == OWNER_CLAIM_COMMAND
        && let Some(result) = find_succeeded_owner_claim(runtime, &input)?
    {
        return hosted_runtime_command_response(result);
    }
    let request_id = random_runtime_request_id()?;
    let body = serde_json::to_vec(&input.body)?;
    let request = RuntimeCommandRequestV1 {
        payload_kind: RuntimeCommandPayloadKindV1::Request,
        request_id: request_id.clone(),
        command: input.command,
        target: RuntimeCommandTargetV1 {
            account_id: input.target_account_id.clone(),
            device_id: None,
        },
        resource_key: input.resource_key,
        body: RuntimeCommandJsonPayloadV1 {
            schema: input.schema,
            json_payload: body,
        },
    };
    request
        .validate_structure()
        .map_err(|error| HostedDeviceError::Task(error.to_string()))?;
    runtime.send_runtime_command_request_and_wait(
        input.room_id.clone(),
        input.conversation_id.clone(),
        serde_json::to_vec(&request)?,
    )?;

    let wait_millis = input
        .wait_millis
        .clamp(1_000, MAX_RUNTIME_COMMAND_WAIT_MILLIS);
    let started = Instant::now();
    loop {
        if let Some(result) = find_runtime_command_result(
            runtime,
            &input.room_id,
            input.conversation_id.as_deref(),
            &input.target_account_id,
            &request_id,
        )? {
            return hosted_runtime_command_response(result);
        }
        let elapsed = started.elapsed();
        if elapsed >= Duration::from_millis(wait_millis) {
            return Err(HostedDeviceError::Task(
                "The agent did not respond in time. Try again.".to_owned(),
            ));
        }
        let remaining = Duration::from_millis(wait_millis).saturating_sub(elapsed);
        runtime.agent_bridge_wait_for_update(
            remaining
                .as_millis()
                .min(u128::from(DEFAULT_UPDATE_TIMEOUT_MILLIS)) as u64,
        )?;
    }
}

fn hosted_runtime_command_response(
    result: RuntimeCommandResultV1,
) -> Result<HostedRuntimeCommandResponse, HostedDeviceError> {
    let decoded_body = result
        .body
        .as_ref()
        .map(|body| serde_json::from_slice::<Value>(&body.json_payload))
        .transpose()?;
    Ok(HostedRuntimeCommandResponse {
        request_id: result.request_id,
        status: result.status,
        body: decoded_body,
        error: result.error,
    })
}

fn find_succeeded_owner_claim(
    runtime: &FiniteChatRuntime,
    input: &HostedRuntimeCommandRequest,
) -> Result<Option<RuntimeCommandResultV1>, HostedDeviceError> {
    let local_account_id = runtime.state()?.identity.account_id;
    let mut matching_requests = HashSet::new();

    for stored in runtime.recent_bridge_events(OWNER_CLAIM_EVENT_LIMIT)? {
        if stored.room_id != input.room_id {
            continue;
        }
        let Ok(event) = serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext)
        else {
            continue;
        };
        if event.conversation_id.as_deref() != input.conversation_id.as_deref() {
            continue;
        }
        match event.kind {
            DurableAppEventKind::RuntimeCommandRequest
                if stored.sender_account_id == local_account_id =>
            {
                let Ok(request) = serde_json::from_slice::<RuntimeCommandRequestV1>(&event.payload)
                else {
                    continue;
                };
                let body_matches = serde_json::from_slice::<Value>(&request.body.json_payload)
                    .is_ok_and(|body| body == input.body);
                if request.validate_structure().is_ok()
                    && request.command == OWNER_CLAIM_COMMAND
                    && request.target.account_id == input.target_account_id
                    && request.target.device_id.is_none()
                    && request.resource_key == input.resource_key
                    && request.body.schema == input.schema
                    && body_matches
                {
                    matching_requests.insert(request.request_id);
                }
            }
            DurableAppEventKind::RuntimeCommandResult
                if stored.sender_account_id == input.target_account_id =>
            {
                let Ok(result) = serde_json::from_slice::<RuntimeCommandResultV1>(&event.payload)
                else {
                    continue;
                };
                if result.validate_structure().is_ok()
                    && result.status == RuntimeCommandTerminalStatusV1::Succeeded
                    && matching_requests.contains(&result.request_id)
                {
                    return Ok(Some(result));
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

fn find_runtime_command_result(
    runtime: &FiniteChatRuntime,
    room_id: &str,
    conversation_id: Option<&str>,
    target_account_id: &str,
    request_id: &str,
) -> Result<Option<RuntimeCommandResultV1>, HostedDeviceError> {
    for stored in runtime.recent_bridge_events(RECENT_RUNTIME_EVENT_LIMIT)? {
        if stored.room_id != room_id || stored.sender_account_id != target_account_id {
            continue;
        }
        let Ok(event) = serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext)
        else {
            continue;
        };
        if event.kind != DurableAppEventKind::RuntimeCommandResult
            || event.conversation_id.as_deref() != conversation_id
        {
            continue;
        }
        let Ok(result) = serde_json::from_slice::<RuntimeCommandResultV1>(&event.payload) else {
            continue;
        };
        if result.request_id == request_id && result.validate_structure().is_ok() {
            return Ok(Some(result));
        }
    }
    Ok(None)
}

fn random_runtime_request_id() -> Result<String, HostedDeviceError> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|error| {
        HostedDeviceError::Task(format!("request id generation failed: {error}"))
    })?;
    Ok(format!("runtime-{}", hex::encode(entropy)))
}

fn random_brain_bootstrap_authorization_id() -> Result<String, HostedDeviceError> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|error| {
        HostedDeviceError::Task(format!(
            "Brain bootstrap authorization id generation failed: {error}"
        ))
    })?;
    Ok(format!("brain-bootstrap-{}", hex::encode(entropy)))
}

fn random_brain_grant_id() -> Result<String, HostedDeviceError> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|error| {
        HostedDeviceError::Task(format!(
            "Brain Folder Key Grant id generation failed: {error}"
        ))
    })?;
    Ok(format!("brain-grant-{}", hex::encode(entropy)))
}

const fn default_runtime_command_wait_millis() -> u64 {
    45_000
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

#[derive(Clone, Debug, Deserialize)]
struct OpenAgentBindingRequest {
    project_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct EnsureAgentBindingRequest {
    project_id: String,
    agent_npub: String,
    display_name: String,
}

#[derive(Clone, Debug, Deserialize)]
struct AuthorizeAgentBindingBootstrapRequest {
    project_id: String,
    creation_request_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssuePersonalVaultBootstrapAuthorizationRequest {
    agent_npub: String,
    vault_id: String,
    workspace_folder_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IssuePersonalVaultBootstrapAuthorizationResponse {
    authorization_id: String,
    owner_npub: String,
    agent_npub: String,
    vault_id: String,
    name: String,
    folder_id: String,
    folder_name: String,
    folder_path: String,
    workspace_folder_id: String,
    expires_at: u64,
    bootstrap_grants: Vec<BrainBootstrapFolderGrant>,
    workspace_grants: Vec<IssuedFolderKeyGrant>,
    bootstrap_authorization: nostr::Event,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrainBootstrapFolderGrant {
    folder_id: String,
    grant: IssuedFolderKeyGrant,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrainIdentityProviderRequest {
    version: String,
    operation: String,
    #[serde(default)]
    input: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenBrainGrantPayloadInput {
    #[serde(flatten)]
    grant: BrainGrantIntent,
    ciphertext: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WrapBrainGrantPayloadInput {
    #[serde(flatten)]
    grant: BrainGrantIntent,
    plaintext: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct HostedAgentBindingV1 {
    version: u16,
    project_id: String,
    human_account_id: String,
    agent_account_id: String,
    agent_npub: String,
    canonical_room_id: String,
    associated_room_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct HostedAgentBindingAuthorizationV1 {
    version: u16,
    project_id: String,
    human_account_id: String,
    creation_request_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct HostedAgentBindingBootstrapV1 {
    version: u16,
    project_id: String,
    human_account_id: String,
    agent_account_id: String,
    agent_npub: String,
    profile: finitechat_core::AppProfileSummary,
    display_name: String,
    intended_room_id: String,
    creation_request_id: String,
    #[serde(default)]
    room_create_request: Option<CreateRoomRequest>,
    #[serde(default)]
    claimed_key_package: Option<ClaimKeyPackageResult>,
    #[serde(default)]
    prepared_commit: Option<AppProfileChatBootstrapPreparedCommit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SealedHostedAgentBindingV1 {
    version: u16,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Serialize)]
struct HostedAgentBindingResponse {
    #[serde(flatten)]
    state: AppState,
    hosted_agent_binding: HostedAgentBindingV1,
}

#[derive(Serialize)]
struct AuthorizeAgentBindingBootstrapResponse {
    status: &'static str,
}

async fn execute_brain_identity_provider_operation(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
    Json(request): Json<BrainIdentityProviderRequest>,
) -> Result<Json<Value>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    let official_origin = headers
        .get(BRAIN_PUBLIC_ORIGIN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            HostedDeviceError::InvalidBrainIdentityProvider(
                "trusted Brain origin is required".to_owned(),
            )
        })?
        .to_owned();
    if request.version != BRAIN_IDENTITY_PROVIDER_VERSION {
        return Err(HostedDeviceError::InvalidBrainIdentityProvider(
            "unsupported provider version".to_owned(),
        ));
    }

    let response =
        tokio::task::spawn_blocking(move || -> Result<_, HostedDeviceError> {
            let keys = brain_user_keys_if_setup(&state, &user_id)?;
            match request.operation.as_str() {
                "identifyMember" => {
                    let public_key = NostrPublicKey::from_protocol(keys.public_key());
                    Ok(json!({
                        "publicKeyHex": public_key.to_hex(),
                        "npub": public_key.to_npub().map_err(|error| {
                            HostedDeviceError::InvalidBrainIdentityProvider(error.to_string())
                        })?,
                    }))
                }
                "authorizeHttpRequest" => {
                    let input: BrainHttpAuthorizationIntent = serde_json::from_value(request.input)
                        .map_err(|error| {
                            HostedDeviceError::InvalidBrainIdentityProvider(error.to_string())
                        })?;
                    validate_brain_http_authorization_intent(&input, &official_origin).map_err(
                        |error| HostedDeviceError::InvalidBrainIdentityProvider(error.to_string()),
                    )?;
                    let event = sign_brain_event_template(&keys, &input.event_template).map_err(
                        |error| HostedDeviceError::InvalidBrainIdentityProvider(error.to_string()),
                    )?;
                    serde_json::to_value(event).map_err(HostedDeviceError::from)
                }
                "authorizeBrainEvent" => {
                    let input: BrainEventAuthorizationIntent =
                        serde_json::from_value(request.input).map_err(|error| {
                            HostedDeviceError::InvalidBrainIdentityProvider(error.to_string())
                        })?;
                    validate_brain_event_authorization_intent(&input).map_err(|error| {
                        HostedDeviceError::InvalidBrainIdentityProvider(error.to_string())
                    })?;
                    let event = sign_brain_event_template(&keys, &input.event_template).map_err(
                        |error| HostedDeviceError::InvalidBrainIdentityProvider(error.to_string()),
                    )?;
                    serde_json::to_value(event).map_err(HostedDeviceError::from)
                }
                "openGrantPayload" => {
                    let input: OpenBrainGrantPayloadInput = serde_json::from_value(request.input)
                        .map_err(|error| {
                        HostedDeviceError::InvalidBrainIdentityProvider(error.to_string())
                    })?;
                    if input.ciphertext.is_empty() {
                        return Err(HostedDeviceError::InvalidBrainIdentityProvider(
                            "Brain grant ciphertext is required".to_owned(),
                        ));
                    }
                    let peer = validate_brain_grant_intent(&input.grant).map_err(|error| {
                        HostedDeviceError::InvalidBrainIdentityProvider(error.to_string())
                    })?;
                    let plaintext = decrypt_nip44(keys.secret_key(), peer, input.ciphertext)
                        .map_err(|error| {
                            HostedDeviceError::InvalidBrainIdentityProvider(error.to_string())
                        })?;
                    Ok(json!({ "plaintext": plaintext }))
                }
                "wrapGrantPayload" => {
                    let input: WrapBrainGrantPayloadInput = serde_json::from_value(request.input)
                        .map_err(|error| {
                        HostedDeviceError::InvalidBrainIdentityProvider(error.to_string())
                    })?;
                    if input.plaintext.is_empty() {
                        return Err(HostedDeviceError::InvalidBrainIdentityProvider(
                            "Brain grant plaintext is required".to_owned(),
                        ));
                    }
                    let peer = validate_brain_grant_intent(&input.grant).map_err(|error| {
                        HostedDeviceError::InvalidBrainIdentityProvider(error.to_string())
                    })?;
                    let ciphertext = encrypt_nip44(keys.secret_key(), peer, input.plaintext)
                        .map_err(|error| {
                            HostedDeviceError::InvalidBrainIdentityProvider(error.to_string())
                        })?;
                    Ok(json!({ "ciphertext": ciphertext }))
                }
                _ => Err(HostedDeviceError::InvalidBrainIdentityProvider(
                    "unsupported Brain Identity Provider operation".to_owned(),
                )),
            }
        })
        .await
        .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(response))
}

fn brain_user_keys_if_setup(
    state: &HostedDeviceState,
    user_id: &str,
) -> Result<Keys, HostedDeviceError> {
    let user_root = state.user_root(user_id);
    let identity_paths = IdentityPaths::with_finite_home(user_root.join("finite-home"));
    let identity_exists = path_exists(&identity_paths.identity_file())?;
    let store_exists = path_exists(&user_root.join("chat/client.sqlite3"))?;
    if !identity_exists && !store_exists {
        return Err(HostedDeviceError::BrainIdentitySetupRequired);
    }
    if identity_exists != store_exists {
        return Err(HostedDeviceError::IncompleteUserState);
    }
    let identity = FiniteIdentity::load(&identity_paths)?;
    let secret = SecretKey::from_slice(&identity.expose_secret_bytes()).map_err(|_| {
        HostedDeviceError::InvalidBrainIdentityProvider(
            "Hosted Device User Key is invalid".to_owned(),
        )
    })?;
    Ok(Keys::new(secret))
}

async fn issue_personal_vault_bootstrap_authorization(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
    Json(input): Json<IssuePersonalVaultBootstrapAuthorizationRequest>,
) -> Result<Json<IssuePersonalVaultBootstrapAuthorizationResponse>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    let response = tokio::task::spawn_blocking(move || -> Result<_, HostedDeviceError> {
        let vault_id = VaultId::new(input.vault_id).map_err(|error| {
            HostedDeviceError::InvalidBrainBootstrapAuthorization(error.to_string())
        })?;
        let workspace_folder_id = FolderId::new(input.workspace_folder_id).map_err(|error| {
            HostedDeviceError::InvalidBrainBootstrapAuthorization(error.to_string())
        })?;
        let runtime = state.runtime_for(&user_id)?;
        let identity = runtime.state()?.identity;
        let secret_bytes = hex::decode(&identity.account_secret_hex).map_err(|_| {
            HostedDeviceError::InvalidBrainBootstrapAuthorization(
                "Hosted Device User Key is invalid".to_owned(),
            )
        })?;
        let secret = SecretKey::from_slice(&secret_bytes).map_err(|_| {
            HostedDeviceError::InvalidBrainBootstrapAuthorization(
                "Hosted Device User Key is invalid".to_owned(),
            )
        })?;
        let owner_keys = Keys::new(secret);
        let owner_npub = finitechat_core::npub_from_account_id(identity.account_id)?;
        let created_at = device_link_now(&state)?;
        let created_at_rfc3339 = OffsetDateTime::from_unix_timestamp(created_at as i64)
            .map_err(|error| HostedDeviceError::Task(format!("system clock is invalid: {error}")))?
            .format(&Rfc3339)
            .map_err(|error| {
                HostedDeviceError::Task(format!("system clock is invalid: {error}"))
            })?;
        let authorization_id = random_brain_bootstrap_authorization_id()?;
        let event = issue_personal_vault_bootstrap_authorization_event(
            &owner_keys,
            authorization_id.clone(),
            input.agent_npub.clone(),
            &vault_id,
            &workspace_folder_id,
            created_at,
        )
        .map_err(|error| {
            HostedDeviceError::InvalidBrainBootstrapAuthorization(error.to_string())
        })?;
        let mut bootstrap_grants = Vec::with_capacity(2);
        for folder_id in [
            FolderId::new("getting-started"),
            FolderId::new("restricted"),
        ] {
            let folder_id = folder_id.map_err(|error| {
                HostedDeviceError::InvalidBrainBootstrapAuthorization(error.to_string())
            })?;
            let folder_key = FolderKey::generate();
            let grant = issue_folder_key_grant(
                &owner_keys,
                random_brain_grant_id()?,
                &vault_id,
                &folder_id,
                1,
                owner_npub.clone(),
                &folder_key,
                created_at_rfc3339.clone(),
                created_at,
            )
            .map_err(|error| {
                HostedDeviceError::InvalidBrainBootstrapAuthorization(error.to_string())
            })?;
            bootstrap_grants.push(BrainBootstrapFolderGrant {
                folder_id: folder_id.to_string(),
                grant,
            });
        }
        let workspace_key = FolderKey::generate();
        let mut workspace_grants = Vec::with_capacity(2);
        for recipient in [&owner_npub, &input.agent_npub] {
            let grant = issue_folder_key_grant(
                &owner_keys,
                random_brain_grant_id()?,
                &vault_id,
                &workspace_folder_id,
                1,
                recipient.clone(),
                &workspace_key,
                created_at_rfc3339.clone(),
                created_at,
            )
            .map_err(|error| {
                HostedDeviceError::InvalidBrainBootstrapAuthorization(error.to_string())
            })?;
            workspace_grants.push(grant);
        }
        Ok(IssuePersonalVaultBootstrapAuthorizationResponse {
            authorization_id,
            owner_npub,
            agent_npub: input.agent_npub,
            vault_id: vault_id.to_string(),
            name: "Personal Brain".to_owned(),
            folder_id: workspace_folder_id.to_string(),
            folder_name: "Agent Workspace".to_owned(),
            folder_path: "Agent Workspace".to_owned(),
            workspace_folder_id: workspace_folder_id.to_string(),
            expires_at: created_at + PERSONAL_VAULT_BOOTSTRAP_MAX_TTL_SECONDS,
            bootstrap_grants,
            workspace_grants,
            bootstrap_authorization: event,
        })
    })
    .await
    .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(response))
}

async fn open_agent_binding(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
    Json(input): Json<OpenAgentBindingRequest>,
) -> Result<Json<HostedAgentBindingResponse>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    validate_binding_field("project_id", &input.project_id)?;
    let response = tokio::task::spawn_blocking(move || -> Result<_, HostedDeviceError> {
        let runtime = state.runtime_for(&user_id)?;
        let binding = load_agent_binding(&state, &user_id, &input.project_id, &runtime)?
            .ok_or(HostedDeviceError::AgentBindingNotFound)?;
        open_validated_agent_binding(&runtime, &binding)
    })
    .await
    .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(response))
}

async fn authorize_agent_binding_bootstrap(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
    Json(input): Json<AuthorizeAgentBindingBootstrapRequest>,
) -> Result<Json<AuthorizeAgentBindingBootstrapResponse>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    validate_binding_field("project_id", &input.project_id)?;
    validate_binding_field("creation_request_id", &input.creation_request_id)?;
    let binding_lock = state.agent_binding_lock(&user_id, &input.project_id)?;
    let response = tokio::task::spawn_blocking(move || -> Result<_, HostedDeviceError> {
        let _binding_guard = binding_lock
            .lock()
            .map_err(|_| HostedDeviceError::LockPoisoned)?;
        let runtime = state.runtime_for(&user_id)?;
        if load_agent_binding(&state, &user_id, &input.project_id, &runtime)?.is_some() {
            return Ok(AuthorizeAgentBindingBootstrapResponse {
                status: "already_bound",
            });
        }
        if let Some(bootstrap) =
            load_agent_binding_bootstrap(&state, &user_id, &input.project_id, &runtime)?
        {
            if bootstrap.creation_request_id == input.creation_request_id {
                return Ok(AuthorizeAgentBindingBootstrapResponse {
                    status: "already_authorized",
                });
            }
            return Err(HostedDeviceError::AgentBindingInvalid(
                "a different Project creation request already started this binding bootstrap"
                    .to_owned(),
            ));
        }
        if let Some(authorization) =
            load_agent_binding_authorization(&state, &user_id, &input.project_id, &runtime)?
        {
            if authorization.creation_request_id == input.creation_request_id {
                return Ok(AuthorizeAgentBindingBootstrapResponse {
                    status: "already_authorized",
                });
            }
            return Err(HostedDeviceError::AgentBindingInvalid(
                "a different Project creation request already authorized this binding bootstrap"
                    .to_owned(),
            ));
        }
        let identity = runtime.state()?.identity;
        let authorization = HostedAgentBindingAuthorizationV1 {
            version: AGENT_BINDING_VERSION,
            project_id: input.project_id,
            human_account_id: identity.account_id,
            creation_request_id: input.creation_request_id,
        };
        persist_agent_binding_authorization(&state, &user_id, &authorization, &runtime)?;
        Ok(AuthorizeAgentBindingBootstrapResponse {
            status: "authorized",
        })
    })
    .await
    .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(response))
}

async fn ensure_agent_binding(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
    Json(input): Json<EnsureAgentBindingRequest>,
) -> Result<Json<HostedAgentBindingResponse>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    validate_binding_field("project_id", &input.project_id)?;
    validate_binding_field("agent_npub", &input.agent_npub)?;
    validate_binding_field("display_name", &input.display_name)?;
    let binding_lock = state.agent_binding_lock(&user_id, &input.project_id)?;
    let response = tokio::task::spawn_blocking(move || -> Result<_, HostedDeviceError> {
        let _binding_guard = binding_lock
            .lock()
            .map_err(|_| HostedDeviceError::LockPoisoned)?;
        let runtime = state.runtime_for(&user_id)?;
        if let Some(binding) = load_agent_binding(&state, &user_id, &input.project_id, &runtime)? {
            if !binding.agent_npub.eq_ignore_ascii_case(&input.agent_npub) {
                return Err(HostedDeviceError::AgentBindingInvalid(
                    "the observed Agent Principal changed".to_owned(),
                ));
            }
            return open_validated_agent_binding(&runtime, &binding);
        }

        let mut bootstrap = match load_agent_binding_bootstrap(
            &state,
            &user_id,
            &input.project_id,
            &runtime,
        )? {
            Some(bootstrap) => {
                validate_agent_binding_bootstrap(&bootstrap, &input, &runtime)?;
                bootstrap
            }
            None => {
                let authorization = load_agent_binding_authorization(
                    &state,
                    &user_id,
                    &input.project_id,
                    &runtime,
                )?
                .ok_or_else(|| {
                    HostedDeviceError::AgentBindingInvalid(
                        "first-time binding bootstrap was not authorized by Project creation"
                            .to_owned(),
                    )
                })?;
                if authorization.human_account_id != runtime.state()?.identity.account_id {
                    return Err(HostedDeviceError::AgentBindingInvalid(
                        "the Hosted Web Device identity changed after Project creation"
                            .to_owned(),
                    ));
                }
                let agent_account_id = account_id_from_npub(input.agent_npub.clone())?;
                if !runtime
                    .profile_chat_room_ids(agent_account_id.clone())?
                    .is_empty()
                {
                    return Err(HostedDeviceError::AgentBindingInvalid(
                        "an existing Agent conversation is unbound; automatic migration is disabled"
                            .to_owned(),
                    ));
                }
                let app = runtime.dispatch_and_wait(AppAction::ScanTarget {
                    value: input.agent_npub.clone(),
                })?;
                let profile = app
                    .profiles
                    .iter()
                    .find(|profile| profile.npub.eq_ignore_ascii_case(&input.agent_npub))
                    .cloned()
                    .ok_or_else(|| {
                        HostedDeviceError::AgentBindingInvalid(
                            "the Agent Principal could not be resolved".to_owned(),
                        )
                    })?;
                if profile.account_id != agent_account_id {
                    return Err(HostedDeviceError::AgentBindingInvalid(
                        "the resolved Agent Principal does not match the requested npub".to_owned(),
                    ));
                }
                let bootstrap = HostedAgentBindingBootstrapV1 {
                    version: AGENT_BINDING_VERSION,
                    project_id: input.project_id.clone(),
                    human_account_id: app.identity.account_id,
                    agent_account_id,
                    agent_npub: profile.npub.clone(),
                    profile,
                    display_name: input.display_name.clone(),
                    intended_room_id: generate_agent_binding_room_id()?,
                    creation_request_id: authorization.creation_request_id,
                    room_create_request: None,
                    claimed_key_package: None,
                    prepared_commit: None,
                };
                let bootstrap =
                    persist_agent_binding_bootstrap(&state, &user_id, &bootstrap, &runtime)?;
                remove_agent_binding_authorization(&state, &user_id, &input.project_id)?;
                bootstrap
            }
        };

        if bootstrap.room_create_request.is_none() {
            bootstrap.room_create_request = Some(
                runtime.plan_profile_chat_bootstrap_room_and_wait(
                    bootstrap.intended_room_id.clone(),
                )?,
            );
            persist_agent_binding_bootstrap_update(
                &state,
                &user_id,
                &bootstrap,
                &runtime,
            )?;
        }
        if bootstrap.claimed_key_package.is_none() {
            bootstrap.claimed_key_package = Some(
                runtime
                    .claim_profile_chat_key_package_and_wait(
                        bootstrap.agent_account_id.clone(),
                    )?
                    .ok_or_else(|| {
                        HostedDeviceError::AgentBindingInvalid(
                            "the Agent has no available KeyPackage for first-time bootstrap"
                                .to_owned(),
                        )
                    })?,
            );
            persist_agent_binding_bootstrap_update(
                &state,
                &user_id,
                &bootstrap,
                &runtime,
            )?;
        }
        let fail_after_room_server_acceptance = state
            .fail_profile_bootstrap_room_creates_after_server_acceptance
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok();
        let preparation = runtime.prepare_profile_chat_bootstrap_and_wait(
            AppProfileChatBootstrapInput {
                profile: bootstrap.profile.clone(),
                display_name: bootstrap.display_name.clone(),
                intended_room_id: bootstrap.intended_room_id.clone(),
                room_create_request: bootstrap
                    .room_create_request
                    .clone()
                    .expect("Room create request was persisted above"),
                claimed_key_package: bootstrap
                    .claimed_key_package
                    .clone()
                    .expect("claimed KeyPackage was persisted above"),
                persisted_prepared_commit: bootstrap.prepared_commit.clone(),
            },
            fail_after_room_server_acceptance,
        )?;
        let app = if preparation.complete {
            preparation.state
        } else {
            let prepared_commit = preparation.prepared_commit.ok_or_else(|| {
                HostedDeviceError::AgentBindingInvalid(
                    "profile chat bootstrap preparation omitted its submit request".to_owned(),
                )
            })?;
            bootstrap.prepared_commit = Some(prepared_commit.clone());
            persist_agent_binding_bootstrap_update(
                &state,
                &user_id,
                &bootstrap,
                &runtime,
            )?;
            let fail_after_device_save = state
                .fail_profile_bootstrap_submits_after_device_save
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok();
            runtime.submit_profile_chat_bootstrap_and_wait(
                bootstrap.agent_account_id.clone(),
                bootstrap.display_name.clone(),
                bootstrap.intended_room_id.clone(),
                prepared_commit,
                fail_after_device_save,
            )?
        };
        let room_ids = runtime.profile_chat_room_ids(bootstrap.agent_account_id.clone())?;
        if !matches!(room_ids.as_slice(), [room_id] if room_id == &bootstrap.intended_room_id)
            || app.selected_room_id.as_deref() != Some(bootstrap.intended_room_id.as_str())
        {
            return Err(HostedDeviceError::AgentBindingInvalid(
                "the durable bootstrap intent did not produce exactly its intended Agent conversation"
                    .to_owned(),
            ));
        }
        let binding = HostedAgentBindingV1 {
            version: AGENT_BINDING_VERSION,
            project_id: bootstrap.project_id,
            human_account_id: bootstrap.human_account_id,
            agent_account_id: bootstrap.agent_account_id,
            agent_npub: bootstrap.agent_npub,
            canonical_room_id: bootstrap.intended_room_id,
            associated_room_ids: Vec::new(),
        };
        persist_agent_binding(&state, &user_id, &binding, &runtime)?;
        remove_agent_binding_bootstrap(&state, &user_id, &binding.project_id)?;
        open_validated_agent_binding(&runtime, &binding)
    })
    .await
    .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(response))
}

fn open_validated_agent_binding(
    runtime: &FiniteChatRuntime,
    binding: &HostedAgentBindingV1,
) -> Result<HostedAgentBindingResponse, HostedDeviceError> {
    let state = runtime.state()?;
    if state.identity.account_id != binding.human_account_id {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the Hosted Web Device identity changed".to_owned(),
        ));
    }
    let room_ids = runtime.profile_chat_room_ids(binding.agent_account_id.clone())?;
    if !room_ids.contains(&binding.canonical_room_id) {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the canonical Room is missing or has unexpected members".to_owned(),
        ));
    }
    let state = runtime.dispatch_and_wait(AppAction::OpenRoom {
        room_id: binding.canonical_room_id.clone(),
    })?;
    Ok(HostedAgentBindingResponse {
        state: redacted_state(state),
        hosted_agent_binding: binding.clone(),
    })
}

fn validate_binding_field(name: &str, value: &str) -> Result<(), HostedDeviceError> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(HostedDeviceError::AgentBindingInvalid(format!(
            "{name} is invalid"
        )));
    }
    Ok(())
}

fn binding_key(runtime: &FiniteChatRuntime) -> Result<[u8; 32], HostedDeviceError> {
    let secret = hex::decode(runtime.state()?.identity.account_secret_hex).map_err(|_| {
        HostedDeviceError::AgentBindingInvalid("Device key material is invalid".to_owned())
    })?;
    let mut hasher = Sha256::new();
    hasher.update(AGENT_BINDING_KEY_DOMAIN);
    hasher.update(secret);
    Ok(hasher.finalize().into())
}

fn binding_aad(user_id: &str, project_id: &str) -> Vec<u8> {
    binding_record_aad(AGENT_BINDING_AAD_DOMAIN, user_id, project_id)
}

fn binding_bootstrap_aad(user_id: &str, project_id: &str) -> Vec<u8> {
    binding_record_aad(AGENT_BINDING_BOOTSTRAP_AAD_DOMAIN, user_id, project_id)
}

fn binding_authorization_aad(user_id: &str, project_id: &str) -> Vec<u8> {
    binding_record_aad(AGENT_BINDING_AUTHORIZATION_AAD_DOMAIN, user_id, project_id)
}

fn binding_record_aad(domain: &[u8], user_id: &str, project_id: &str) -> Vec<u8> {
    let mut aad = domain.to_vec();
    aad.extend_from_slice(user_storage_id(user_id).as_bytes());
    aad.push(0);
    aad.extend_from_slice(project_id.as_bytes());
    aad
}

fn load_agent_binding(
    state: &HostedDeviceState,
    user_id: &str,
    project_id: &str,
    runtime: &FiniteChatRuntime,
) -> Result<Option<HostedAgentBindingV1>, HostedDeviceError> {
    let path = state.agent_binding_path(user_id, project_id);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if bytes.len() > MAX_AGENT_BINDING_RECORD_BYTES {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the binding record is oversized".to_owned(),
        ));
    }
    let sealed: SealedHostedAgentBindingV1 = serde_json::from_slice(&bytes).map_err(|_| {
        HostedDeviceError::AgentBindingInvalid("the binding record is corrupt".to_owned())
    })?;
    if sealed.version != AGENT_BINDING_VERSION || sealed.nonce.len() != AGENT_BINDING_NONCE_BYTES {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the binding record version is unsupported".to_owned(),
        ));
    }
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            &binding_key(runtime)?,
            &sealed.ciphertext,
            &sealed.nonce,
            &binding_aad(user_id, project_id),
        )
        .map_err(|_| {
            HostedDeviceError::AgentBindingInvalid(
                "the binding record failed authentication".to_owned(),
            )
        })?;
    let binding: HostedAgentBindingV1 = serde_json::from_slice(&plaintext).map_err(|_| {
        HostedDeviceError::AgentBindingInvalid("the binding plaintext is invalid".to_owned())
    })?;
    if binding.version != AGENT_BINDING_VERSION || binding.project_id != project_id {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the binding record does not match this Project".to_owned(),
        ));
    }
    Ok(Some(binding))
}

fn load_agent_binding_bootstrap(
    state: &HostedDeviceState,
    user_id: &str,
    project_id: &str,
    runtime: &FiniteChatRuntime,
) -> Result<Option<HostedAgentBindingBootstrapV1>, HostedDeviceError> {
    let path = state.agent_binding_bootstrap_path(user_id, project_id);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if bytes.len() > MAX_AGENT_BINDING_RECORD_BYTES {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the bootstrap intent is oversized".to_owned(),
        ));
    }
    let sealed: SealedHostedAgentBindingV1 = serde_json::from_slice(&bytes).map_err(|_| {
        HostedDeviceError::AgentBindingInvalid("the bootstrap intent is corrupt".to_owned())
    })?;
    if sealed.version != AGENT_BINDING_VERSION || sealed.nonce.len() != AGENT_BINDING_NONCE_BYTES {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the bootstrap intent version is unsupported".to_owned(),
        ));
    }
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            &binding_key(runtime)?,
            &sealed.ciphertext,
            &sealed.nonce,
            &binding_bootstrap_aad(user_id, project_id),
        )
        .map_err(|_| {
            HostedDeviceError::AgentBindingInvalid(
                "the bootstrap intent failed authentication".to_owned(),
            )
        })?;
    let bootstrap: HostedAgentBindingBootstrapV1 =
        serde_json::from_slice(&plaintext).map_err(|_| {
            HostedDeviceError::AgentBindingInvalid("the bootstrap intent is invalid".to_owned())
        })?;
    if bootstrap.version != AGENT_BINDING_VERSION || bootstrap.project_id != project_id {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the bootstrap intent does not match this Project".to_owned(),
        ));
    }
    Ok(Some(bootstrap))
}

fn load_agent_binding_authorization(
    state: &HostedDeviceState,
    user_id: &str,
    project_id: &str,
    runtime: &FiniteChatRuntime,
) -> Result<Option<HostedAgentBindingAuthorizationV1>, HostedDeviceError> {
    let path = state.agent_binding_authorization_path(user_id, project_id);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if bytes.len() > MAX_AGENT_BINDING_RECORD_BYTES {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the bootstrap authorization is oversized".to_owned(),
        ));
    }
    let sealed: SealedHostedAgentBindingV1 = serde_json::from_slice(&bytes).map_err(|_| {
        HostedDeviceError::AgentBindingInvalid("the bootstrap authorization is corrupt".to_owned())
    })?;
    if sealed.version != AGENT_BINDING_VERSION || sealed.nonce.len() != AGENT_BINDING_NONCE_BYTES {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the bootstrap authorization version is unsupported".to_owned(),
        ));
    }
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            &binding_key(runtime)?,
            &sealed.ciphertext,
            &sealed.nonce,
            &binding_authorization_aad(user_id, project_id),
        )
        .map_err(|_| {
            HostedDeviceError::AgentBindingInvalid(
                "the bootstrap authorization failed authentication".to_owned(),
            )
        })?;
    let authorization: HostedAgentBindingAuthorizationV1 = serde_json::from_slice(&plaintext)
        .map_err(|_| {
            HostedDeviceError::AgentBindingInvalid(
                "the bootstrap authorization is invalid".to_owned(),
            )
        })?;
    if authorization.version != AGENT_BINDING_VERSION || authorization.project_id != project_id {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the bootstrap authorization does not match this Project".to_owned(),
        ));
    }
    Ok(Some(authorization))
}

fn validate_agent_binding_bootstrap(
    bootstrap: &HostedAgentBindingBootstrapV1,
    request: &EnsureAgentBindingRequest,
    runtime: &FiniteChatRuntime,
) -> Result<(), HostedDeviceError> {
    let identity = runtime.state()?.identity;
    if identity.account_id != bootstrap.human_account_id {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the Hosted Web Device identity changed during bootstrap".to_owned(),
        ));
    }
    if !bootstrap
        .agent_npub
        .eq_ignore_ascii_case(&request.agent_npub)
    {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "a different Agent Principal already has a pending bootstrap for this Project"
                .to_owned(),
        ));
    }
    let requested_account_id = account_id_from_npub(request.agent_npub.clone())?;
    if requested_account_id != bootstrap.agent_account_id
        || bootstrap.profile.account_id != bootstrap.agent_account_id
        || !bootstrap
            .profile
            .npub
            .eq_ignore_ascii_case(&bootstrap.agent_npub)
    {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the pending bootstrap Agent identity is inconsistent".to_owned(),
        ));
    }
    validate_binding_field("intended_room_id", &bootstrap.intended_room_id)
}

fn generate_agent_binding_room_id() -> Result<String, HostedDeviceError> {
    let provider = OpenMlsRustCrypto::default();
    let bytes: [u8; 8] = provider
        .rand()
        .random_array()
        .map_err(|_| HostedDeviceError::Task("binding Room id generation failed".to_owned()))?;
    Ok(format!("room-{}", hex::encode(bytes)))
}

fn persist_agent_binding_bootstrap(
    state: &HostedDeviceState,
    user_id: &str,
    bootstrap: &HostedAgentBindingBootstrapV1,
    runtime: &FiniteChatRuntime,
) -> Result<HostedAgentBindingBootstrapV1, HostedDeviceError> {
    let encoded = seal_agent_binding_record(
        user_id,
        &bootstrap.project_id,
        bootstrap,
        runtime,
        AgentBindingRecordKind::Bootstrap,
    )?;
    if write_atomic_durable_create_new(
        &state.agent_binding_bootstrap_path(user_id, &bootstrap.project_id),
        &encoded,
    )? {
        return Ok(bootstrap.clone());
    }
    let existing = load_agent_binding_bootstrap(state, user_id, &bootstrap.project_id, runtime)?
        .ok_or_else(|| {
            HostedDeviceError::AgentBindingInvalid(
                "the concurrently created bootstrap intent disappeared".to_owned(),
            )
        })?;
    if existing.project_id == bootstrap.project_id
        && existing.human_account_id == bootstrap.human_account_id
        && existing.agent_account_id == bootstrap.agent_account_id
        && existing
            .agent_npub
            .eq_ignore_ascii_case(&bootstrap.agent_npub)
        && existing.creation_request_id == bootstrap.creation_request_id
    {
        Ok(existing)
    } else {
        Err(HostedDeviceError::AgentBindingInvalid(
            "a conflicting bootstrap intent already exists for this Project".to_owned(),
        ))
    }
}

fn persist_agent_binding_bootstrap_update(
    state: &HostedDeviceState,
    user_id: &str,
    bootstrap: &HostedAgentBindingBootstrapV1,
    runtime: &FiniteChatRuntime,
) -> Result<(), HostedDeviceError> {
    let encoded = seal_agent_binding_record(
        user_id,
        &bootstrap.project_id,
        bootstrap,
        runtime,
        AgentBindingRecordKind::Bootstrap,
    )?;
    write_atomic_durable_replace(
        &state.agent_binding_bootstrap_path(user_id, &bootstrap.project_id),
        &encoded,
    )
}

fn persist_agent_binding_authorization(
    state: &HostedDeviceState,
    user_id: &str,
    authorization: &HostedAgentBindingAuthorizationV1,
    runtime: &FiniteChatRuntime,
) -> Result<HostedAgentBindingAuthorizationV1, HostedDeviceError> {
    let encoded = seal_agent_binding_record(
        user_id,
        &authorization.project_id,
        authorization,
        runtime,
        AgentBindingRecordKind::Authorization,
    )?;
    if write_atomic_durable_create_new(
        &state.agent_binding_authorization_path(user_id, &authorization.project_id),
        &encoded,
    )? {
        return Ok(authorization.clone());
    }
    let existing =
        load_agent_binding_authorization(state, user_id, &authorization.project_id, runtime)?
            .ok_or_else(|| {
                HostedDeviceError::AgentBindingInvalid(
                    "the concurrently created bootstrap authorization disappeared".to_owned(),
                )
            })?;
    if existing == *authorization {
        Ok(existing)
    } else {
        Err(HostedDeviceError::AgentBindingInvalid(
            "a conflicting bootstrap authorization already exists for this Project".to_owned(),
        ))
    }
}

fn remove_agent_binding_bootstrap(
    state: &HostedDeviceState,
    user_id: &str,
    project_id: &str,
) -> Result<(), HostedDeviceError> {
    let path = state.agent_binding_bootstrap_path(user_id, project_id);
    match fs::remove_file(&path) {
        Ok(()) => sync_parent_directory(&path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn remove_agent_binding_authorization(
    state: &HostedDeviceState,
    user_id: &str,
    project_id: &str,
) -> Result<(), HostedDeviceError> {
    let path = state.agent_binding_authorization_path(user_id, project_id);
    match fs::remove_file(&path) {
        Ok(()) => sync_parent_directory(&path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn persist_agent_binding(
    state: &HostedDeviceState,
    user_id: &str,
    binding: &HostedAgentBindingV1,
    runtime: &FiniteChatRuntime,
) -> Result<(), HostedDeviceError> {
    if state
        .fail_final_agent_binding_persists
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
            remaining.checked_sub(1)
        })
        .is_ok()
    {
        return Err(HostedDeviceError::Task(
            "injected final Agent binding persistence failure".to_owned(),
        ));
    }
    let encoded = seal_agent_binding_record(
        user_id,
        &binding.project_id,
        binding,
        runtime,
        AgentBindingRecordKind::Binding,
    )?;
    if write_atomic_durable_create_new(
        &state.agent_binding_path(user_id, &binding.project_id),
        &encoded,
    )? {
        return Ok(());
    }
    match load_agent_binding(state, user_id, &binding.project_id, runtime)? {
        Some(existing) if existing == *binding => Ok(()),
        Some(_) => Err(HostedDeviceError::AgentBindingInvalid(
            "a conflicting canonical binding already exists for this Project".to_owned(),
        )),
        None => Err(HostedDeviceError::AgentBindingInvalid(
            "the concurrently created canonical binding disappeared".to_owned(),
        )),
    }
}

#[derive(Clone, Copy)]
enum AgentBindingRecordKind {
    Binding,
    Bootstrap,
    Authorization,
}

fn seal_agent_binding_record<T: Serialize>(
    user_id: &str,
    project_id: &str,
    record: &T,
    runtime: &FiniteChatRuntime,
    kind: AgentBindingRecordKind,
) -> Result<Vec<u8>, HostedDeviceError> {
    let plaintext = serde_json::to_vec(record)?;
    let provider = OpenMlsRustCrypto::default();
    let nonce: [u8; AGENT_BINDING_NONCE_BYTES] = provider
        .rand()
        .random_array()
        .map_err(|_| HostedDeviceError::Task("binding nonce generation failed".to_owned()))?;
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            &binding_key(runtime)?,
            &plaintext,
            &nonce,
            &match kind {
                AgentBindingRecordKind::Binding => binding_aad(user_id, project_id),
                AgentBindingRecordKind::Bootstrap => binding_bootstrap_aad(user_id, project_id),
                AgentBindingRecordKind::Authorization => {
                    binding_authorization_aad(user_id, project_id)
                }
            },
        )
        .map_err(|_| HostedDeviceError::Task("binding encryption failed".to_owned()))?;
    let encoded = serde_json::to_vec(&SealedHostedAgentBindingV1 {
        version: AGENT_BINDING_VERSION,
        nonce: nonce.to_vec(),
        ciphertext,
    })?;
    if encoded.len() > MAX_AGENT_BINDING_RECORD_BYTES {
        return Err(HostedDeviceError::AgentBindingInvalid(
            "the sealed binding record exceeds its durable size limit".to_owned(),
        ));
    }
    Ok(encoded)
}

/// Atomically install a durable record without replacing an existing target.
/// A same-directory hard link gives create-if-absent semantics that `rename`
/// does not provide, while a unique `create_new` temp avoids cross-process
/// writers clobbering one another during an overlapping deploy.
fn write_atomic_durable_create_new(
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<bool, HostedDeviceError> {
    let parent = path
        .parent()
        .ok_or_else(|| HostedDeviceError::Task("invalid binding path".to_owned()))?;
    fs::create_dir_all(parent)?;
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|error| {
        HostedDeviceError::Task(format!("binding temporary name generation failed: {error}"))
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| HostedDeviceError::Task("invalid binding file name".to_owned()))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", hex::encode(entropy)));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let write_result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    let installed = match fs::hard_link(&temporary, path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
    };
    fs::remove_file(&temporary)?;
    if installed {
        sync_parent_directory(path)?;
    }
    Ok(installed)
}

fn write_atomic_durable_replace(
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<(), HostedDeviceError> {
    let parent = path
        .parent()
        .ok_or_else(|| HostedDeviceError::Task("invalid binding path".to_owned()))?;
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|error| {
        HostedDeviceError::Task(format!("binding temporary name generation failed: {error}"))
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| HostedDeviceError::Task("invalid binding file name".to_owned()))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", hex::encode(entropy)));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let write_result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    sync_parent_directory(path)
}

fn sync_parent_directory(path: &std::path::Path) -> Result<(), HostedDeviceError> {
    let parent = path
        .parent()
        .ok_or_else(|| HostedDeviceError::Task("invalid binding path".to_owned()))?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
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

#[derive(Deserialize)]
struct StartNewChatRequest {
    project_id: String,
    room_id: String,
    topic_id: String,
    reason: Option<String>,
    intent_key: String,
}

async fn start_new_chat(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
    Json(input): Json<StartNewChatRequest>,
) -> Result<Json<AppState>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    validate_binding_field("project_id", &input.project_id)?;
    let next = tokio::task::spawn_blocking(move || -> Result<_, HostedDeviceError> {
        let runtime = state.runtime_for(&user_id)?;
        let binding = load_agent_binding(&state, &user_id, &input.project_id, &runtime)?
            .ok_or(HostedDeviceError::AgentBindingNotFound)?;
        let opened = open_validated_agent_binding(&runtime, &binding)?;
        validate_new_chat_target(&opened.hosted_agent_binding, &input, &opened.state.topics)?;
        Ok(runtime.start_topic_chat_intent_and_wait(
            input.room_id,
            input.topic_id,
            input.reason,
            input.intent_key,
        )?)
    })
    .await
    .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    Ok(Json(redacted_state(next)))
}

fn validate_new_chat_target(
    binding: &HostedAgentBindingV1,
    input: &StartNewChatRequest,
    topics: &[AppTopicSummary],
) -> Result<(), HostedDeviceError> {
    if input.room_id != binding.canonical_room_id {
        return Err(HostedDeviceError::CanonicalChatConflict(
            "Room does not match the persisted binding".to_owned(),
        ));
    }
    if !topics
        .iter()
        .any(|topic| topic.room_id == binding.canonical_room_id && topic.topic_id == input.topic_id)
    {
        return Err(HostedDeviceError::CanonicalChatConflict(
            "Topic does not belong to the canonical Room".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct HostedProfileImageResponse {
    image_url: String,
}

async fn upload_profile_image(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<HostedProfileImageResponse>, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if body.is_empty() {
        return Err(HostedDeviceError::InvalidMultipart(
            "profile image is empty".to_owned(),
        ));
    }

    let runtime = state.runtime_for(&user_id)?;
    let next = tokio::task::spawn_blocking(move || {
        runtime.dispatch_and_wait(AppAction::UploadImage {
            bytes: body.to_vec(),
            content_type,
        })
    })
    .await
    .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    let image_url = next.flow.image_upload_url.ok_or_else(|| {
        HostedDeviceError::Task("profile image upload returned no URL".to_owned())
    })?;
    Ok(Json(HostedProfileImageResponse { image_url }))
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

fn path_exists(path: &std::path::Path) -> Result<bool, HostedDeviceError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
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

    fn test_binding() -> HostedAgentBindingV1 {
        HostedAgentBindingV1 {
            version: AGENT_BINDING_VERSION,
            project_id: "project-1".to_owned(),
            human_account_id: "human-1".to_owned(),
            agent_account_id: "agent-1".to_owned(),
            agent_npub: "npub1agent".to_owned(),
            canonical_room_id: "canonical-room".to_owned(),
            associated_room_ids: vec!["legacy-room".to_owned()],
        }
    }

    fn test_topic(room_id: &str, topic_id: &str) -> AppTopicSummary {
        AppTopicSummary {
            room_id: room_id.to_owned(),
            topic_id: topic_id.to_owned(),
            title: topic_id.to_owned(),
            description: None,
            last_message_preview: String::new(),
            unread_count: 0,
            message_count: 0,
            created_seq: 0,
            updated_seq: 0,
            archived: false,
            active_chat_id: None,
            chats: Vec::new(),
        }
    }

    fn test_new_chat(room_id: &str, topic_id: &str) -> StartNewChatRequest {
        StartNewChatRequest {
            project_id: "project-1".to_owned(),
            room_id: room_id.to_owned(),
            topic_id: topic_id.to_owned(),
            reason: None,
            intent_key: "intent-1".to_owned(),
        }
    }

    #[test]
    fn new_chat_target_requires_a_topic_in_the_bound_canonical_room() {
        let binding = test_binding();
        let canonical_topics = vec![test_topic("canonical-room", "home")];

        assert!(
            validate_new_chat_target(
                &binding,
                &test_new_chat("canonical-room", "home"),
                &canonical_topics,
            )
            .is_ok()
        );
        assert!(matches!(
            validate_new_chat_target(
                &binding,
                &test_new_chat("legacy-room", "legacy-topic"),
                &[test_topic("legacy-room", "legacy-topic")],
            ),
            Err(HostedDeviceError::CanonicalChatConflict(_))
        ));
        assert!(matches!(
            validate_new_chat_target(
                &binding,
                &test_new_chat("canonical-room", "legacy-topic"),
                &[test_topic("legacy-room", "legacy-topic")],
            ),
            Err(HostedDeviceError::CanonicalChatConflict(_))
        ));
    }

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
