//! FiniteBrain HTTP server and API surface.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, OriginalUri, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
#[cfg(test)]
use finite_brain_core::FolderRole;
use finite_brain_core::{
    AdminAccessAction, AdminAccessChangePayload, AdminAccessChangeValidation,
    BootstrapSmokeSummary, BrainId, BrainKind, CoreError, CryptoRecordError, DisplayName, Folder,
    FolderAccessMode, FolderId, FolderObjectOperation, FolderObjectRevisionPayload,
    FolderObjectTombstonePayload, FolderRotationFanout, FolderRotationOperation, ObjectId,
    RequiredFolderKeyGrant, RevisionValidation, SafeRelativePath, TombstoneValidation, UserId,
    bootstrap_organization_brain, bootstrap_organization_brain_with_requester,
    bootstrap_personal_brain, validate_admin_access_change_event, validate_folder_rotation_fanout,
    validate_revision_event, validate_tombstone_event,
};
use finite_brain_store::{
    BrainInvitationTargetKind, BrainStore, ControlSyncRecord, EmailInviteBootstrapScopeFolder,
    EncryptedBrainExport, FolderKeyGrantMetadata, FolderObjectRevisionSyncRecord,
    FolderObjectTombstoneSyncRecord, GrantFolderAccessOutcome, IdentityAlias, LinkStatus,
    MountedFolderProjection, MountedFolderState, PersonalAgentFolderRotation,
    SharedFolderConnectionStatus, SharedFolderDirection, StoreError, StoredBrain,
    StoredBrainInvitation, StoredShareLink, StoredSharedFolderConnection,
    StoredSharedFolderInvitation, StoredSyncRecord, SyncRecordInput, SyncRecordType, VisibleBrain,
    VisibleBrainRole,
};
use finite_nostr::{
    MAX_NIP05_DOCUMENT_BYTES, Nip05Identifier, Nip05WellKnownDocument, Nip05WellKnownRequest,
    NostrPrimitiveError, NostrPublicKey, validate_gift_wrap, verify_event_integrity,
};
use nostr::Event;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

mod contracts;
mod object_records;
mod protected_routes;
mod responses;

pub use contracts::*;
pub(crate) use object_records::*;
pub(crate) use responses::*;

use protected_routes::{
    cors_allowed_origins_from_public_base_url, cors_allowlist_middleware, validate_request_auth,
};

const DEFAULT_PUBLIC_BASE_URL: &str = "http://127.0.0.1:3015";
const DEFAULT_MAX_AUTH_SKEW_SECONDS: u64 = 60;
const DEFAULT_RATE_LIMIT_MAX_REQUESTS: u32 = 120;
const DEFAULT_RATE_LIMIT_WINDOW_SECONDS: u64 = 60;
const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;
const MAX_SYNC_RECORDS_LIMIT: u64 = 1_000;
const NOSTR_AUTHORIZATION_HEADER: &str = "x-nostr-authorization";
const FINITEBRAIN_NOSTR_HEADER: &str = "x-finitebrain-nostr";
const APP_SPECIFIC_KIND: u16 = 30_078;
const NIP05_CONNECT_TIMEOUT_SECONDS: u64 = 3;
const NIP05_READ_TIMEOUT_SECONDS: u64 = 5;
const FINITE_VIP_NIP05_PREFIX: &str = "https://finite.vip/";
const SECP256K1_ORDER_HEX: &str =
    "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141";

type Nip05Fetcher =
    Arc<dyn Fn(&Nip05WellKnownRequest) -> Result<Vec<u8>, String> + Send + Sync + 'static>;

fn normalized_smoke_nip07_secret(secret_hex: impl Into<String>) -> Result<String, String> {
    let value = secret_hex.into().trim().to_ascii_lowercase();
    if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err("FINITE_BRAIN_SMOKE_NIP07_SECRET must be 64 hex characters".to_owned());
    }
    if value.chars().all(|character| character == '0') || value.as_str() >= SECP256K1_ORDER_HEX {
        return Err("FINITE_BRAIN_SMOKE_NIP07_SECRET must be a valid secp256k1 secret".to_owned());
    }
    Ok(value)
}

fn normalized_smoke_email_proofs(value: impl AsRef<str>) -> Result<BTreeSet<String>, String> {
    let mut emails = BTreeSet::new();
    for raw in value.as_ref().split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        emails.insert(canonical_email(raw).map_err(|error| error.message)?);
    }
    if emails.is_empty() {
        return Err("FINITE_BRAIN_SMOKE_EMAIL_PROOFS must include at least one email".to_owned());
    }
    Ok(emails)
}

/// Development status returned by the first smoke path.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct HealthStatus {
    pub service: String,
    pub status: String,
    pub core_crate: String,
    pub store_crate: String,
}

/// Public Product Client runtime config.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductClientConfigResponse {
    pub public_base_url: String,
    pub auth_scheme: String,
    pub http_auth_kind: u16,
    pub default_brain_id: String,
}

/// Shared server state.
#[derive(Clone)]
pub struct ServerState {
    store: Arc<Mutex<BrainStore>>,
    public_base_url: Arc<str>,
    auth_now_unix_seconds: Option<u64>,
    max_auth_skew_seconds: u64,
    auth_replay_cache: Arc<Mutex<BTreeMap<String, u64>>>,
    rate_limit_hits: Arc<Mutex<BTreeMap<String, Vec<u64>>>>,
    rate_limit: RateLimitConfig,
    cors_allowed_origins: Arc<BTreeSet<String>>,
    nip05_fetcher: Nip05Fetcher,
    email_proof_verifier: Option<EmailProofVerifier>,
    invite_mailer: Option<BrainInviteMailer>,
    smoke_nip07_signer_secret: Option<Arc<str>>,
    agent_bootstrap_authorities: Option<AgentBootstrapAuthorities>,
}

type EmailProofVerifier = Arc<dyn Fn(&str, &UserId) -> Result<(), EmailProofFailure> + Send + Sync>;
type BrainInviteMailer = Arc<dyn Fn(&BrainInviteEmail) -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
struct AgentBootstrapAuthorities {
    core_base_url: Arc<str>,
    core_token: Arc<str>,
    identity_base_url: Arc<str>,
    identity_token: Arc<str>,
}

#[derive(Debug)]
enum EmailProofFailure {
    Authority(AuthorityFailure),
    Rejected(String),
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct AccountAgentPrincipals {
    owner_npub: UserId,
    agent_npub: UserId,
    owner_email: String,
    managed_agent_email: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityAgentResolutionResponse {
    agent_npub: String,
    managed_agent_email: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreAgentAccountResponse {
    workos_user_id: String,
    managed_agent_email: String,
    #[serde(default)]
    verified_email: Option<String>,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityUserResolutionResponse {
    workos_user_id: String,
    user_npub: String,
}

/// Server-visible Brain invitation email payload.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BrainInviteEmail {
    /// Recipient email address.
    pub to: String,
    /// Email subject.
    pub subject: String,
    /// Plaintext body.
    pub text: String,
}

#[derive(Debug, Clone, Copy)]
struct RateLimitConfig {
    max_requests: u32,
    window_seconds: u64,
}

impl ServerState {
    /// Build state around an existing store.
    pub fn new(store: BrainStore, public_base_url: impl Into<String>) -> Self {
        let public_base_url = public_base_url.into();
        let cors_allowed_origins = cors_allowed_origins_from_public_base_url(&public_base_url);
        Self {
            store: Arc::new(Mutex::new(store)),
            public_base_url: Arc::<str>::from(public_base_url),
            auth_now_unix_seconds: None,
            max_auth_skew_seconds: DEFAULT_MAX_AUTH_SKEW_SECONDS,
            auth_replay_cache: Arc::new(Mutex::new(BTreeMap::new())),
            rate_limit_hits: Arc::new(Mutex::new(BTreeMap::new())),
            rate_limit: RateLimitConfig {
                max_requests: DEFAULT_RATE_LIMIT_MAX_REQUESTS,
                window_seconds: DEFAULT_RATE_LIMIT_WINDOW_SECONDS,
            },
            cors_allowed_origins: Arc::new(cors_allowed_origins),
            nip05_fetcher: default_nip05_fetcher(),
            email_proof_verifier: None,
            invite_mailer: None,
            smoke_nip07_signer_secret: None,
            agent_bootstrap_authorities: None,
        }
    }

    /// Override the auth validation clock for deterministic tests.
    pub fn with_auth_clock(mut self, now_unix_seconds: u64, max_skew_seconds: u64) -> Self {
        self.auth_now_unix_seconds = Some(now_unix_seconds);
        self.max_auth_skew_seconds = max_skew_seconds;
        self
    }

    /// Override protected route rate limits for tests or deployments.
    pub fn with_rate_limit(mut self, max_requests: u32, window_seconds: u64) -> Self {
        self.rate_limit = RateLimitConfig {
            max_requests: max_requests.max(1),
            window_seconds: window_seconds.max(1),
        };
        self
    }

    /// Override CORS allowed origins.
    pub fn with_cors_allowed_origins(mut self, origins: impl IntoIterator<Item = String>) -> Self {
        self.cors_allowed_origins = Arc::new(origins.into_iter().collect());
        self
    }

    /// Verify email proof through a finite-identity Authority deployment.
    pub fn with_identity_authority_url(mut self, base_url: impl Into<String>) -> Self {
        let base_url = base_url.into().trim().trim_end_matches('/').to_owned();
        if !base_url.is_empty() {
            self.email_proof_verifier =
                Some(identity_authority_email_proof_verifier(base_url.clone()));
            self.nip05_fetcher = identity_authority_nip05_fetcher(base_url);
        }
        self
    }

    /// Configure the Core and Finite Identity facts required for agent-first bootstrap.
    pub fn with_agent_bootstrap_authorities(
        mut self,
        core_base_url: impl Into<String>,
        core_token: impl Into<String>,
        identity_base_url: impl Into<String>,
        identity_token: impl Into<String>,
    ) -> Self {
        let core_base_url = core_base_url.into().trim().trim_end_matches('/').to_owned();
        let core_token = core_token.into().trim().to_owned();
        let identity_base_url = identity_base_url
            .into()
            .trim()
            .trim_end_matches('/')
            .to_owned();
        let identity_token = identity_token.into().trim().to_owned();
        if !core_base_url.is_empty()
            && !core_token.is_empty()
            && !identity_base_url.is_empty()
            && !identity_token.is_empty()
        {
            self.agent_bootstrap_authorities = Some(AgentBootstrapAuthorities {
                core_base_url: Arc::from(core_base_url),
                core_token: Arc::from(core_token),
                identity_base_url: Arc::from(identity_base_url),
                identity_token: Arc::from(identity_token),
            });
        }
        self
    }

    /// Deliver Brain-owned Brain invitation emails through a local dev sink.
    pub fn with_dev_invite_mailer(mut self) -> Self {
        self.invite_mailer = Some(Arc::new(|email| {
            eprintln!(
                "finite-brain dev invite email\nTo: {}\nSubject: {}\n\n{}",
                email.to, email.subject, email.text
            );
            Ok(())
        }));
        self
    }

    /// Deliver Brain-owned Brain invitation emails through Resend.
    pub fn with_resend_invite_mailer(
        mut self,
        api_key: impl Into<String>,
        from: impl Into<String>,
    ) -> Self {
        self.invite_mailer = Some(resend_invite_mailer(api_key.into(), from.into()));
        self
    }

    /// Deliver Brain-owned Brain invitation emails through Postmark.
    pub fn with_postmark_invite_mailer(
        mut self,
        server_token: impl Into<String>,
        from: impl Into<String>,
    ) -> Self {
        self.invite_mailer = Some(postmark_invite_mailer(server_token.into(), from.into()));
        self
    }

    /// Enable a local Product Client NIP-07 shim for browser smoke tests.
    pub fn with_smoke_nip07_signer(
        mut self,
        secret_hex: impl Into<String>,
    ) -> Result<Self, String> {
        self.smoke_nip07_signer_secret =
            Some(Arc::<str>::from(normalized_smoke_nip07_secret(secret_hex)?));
        Ok(self)
    }

    /// Enable an explicit local email-proof allowlist for browser smoke tests.
    pub fn with_smoke_email_proofs(mut self, emails: impl AsRef<str>) -> Result<Self, String> {
        let allowed = Arc::new(normalized_smoke_email_proofs(emails)?);
        self.email_proof_verifier = Some(Arc::new(move |email, _actor| {
            let email = canonical_email(email)
                .map_err(|error| EmailProofFailure::Rejected(error.message))?;
            if allowed.contains(&email) {
                Ok(())
            } else {
                Err(EmailProofFailure::Rejected(format!(
                    "smoke email proof is not allowed for {email}"
                )))
            }
        }));
        Ok(self)
    }

    #[cfg(test)]
    fn with_email_proof_verifier(
        mut self,
        verifier: impl Fn(&str, &UserId) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.email_proof_verifier = Some(Arc::new(move |email, actor| {
            verifier(email, actor).map_err(EmailProofFailure::Rejected)
        }));
        self
    }

    #[cfg(test)]
    fn with_invite_mailer(
        mut self,
        mailer: impl Fn(&BrainInviteEmail) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.invite_mailer = Some(Arc::new(mailer));
        self
    }

    #[cfg(test)]
    fn with_nip05_fixture(mut self, url: String, document: impl Into<Vec<u8>>) -> Self {
        let document = Arc::new(document.into());
        self.nip05_fetcher = Arc::new(move |request| {
            if request.url == url {
                Ok((*document).clone())
            } else {
                Err(format!("unexpected NIP-05 URL {}", request.url))
            }
        });
        self
    }

    fn auth_now_unix_seconds(&self) -> u64 {
        self.auth_now_unix_seconds
            .unwrap_or_else(current_unix_seconds)
    }

    fn cors_origin_allowed(&self, origin: &str) -> bool {
        self.cors_allowed_origins.contains(origin)
    }
}

/// API error body.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct ApiErrorBody {
    pub error: String,
}

#[derive(Debug, Clone)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(value: StoreError) -> Self {
        match value {
            StoreError::Core(error) => Self::from(error),
            StoreError::MissingBrain { .. } | StoreError::MissingFolder { .. } => {
                Self::new(StatusCode::NOT_FOUND, value.to_string())
            }
            StoreError::DuplicateId { .. } | StoreError::Conflict { .. } => {
                Self::new(StatusCode::CONFLICT, value.to_string())
            }
            StoreError::MissingRequiredGrant { .. }
            | StoreError::BrokenInvariant { .. }
            | StoreError::InvalidRecord { .. } => {
                Self::new(StatusCode::BAD_REQUEST, value.to_string())
            }
            StoreError::RebootstrapRequired { .. } => {
                Self::new(StatusCode::GONE, value.to_string())
            }
            StoreError::UnavailableLink { .. } => {
                Self::new(StatusCode::NOT_FOUND, value.to_string())
            }
            StoreError::CapacityExceeded { .. } => {
                Self::new(StatusCode::PAYLOAD_TOO_LARGE, value.to_string())
            }
            StoreError::Database { .. } => {
                Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
            }
        }
    }
}

impl From<CoreError> for ApiError {
    fn from(value: CoreError) -> Self {
        let status = match &value {
            CoreError::RotationFanoutLimitExceeded { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        };
        Self::new(status, value.to_string())
    }
}

impl From<CryptoRecordError> for ApiError {
    fn from(value: CryptoRecordError) -> Self {
        Self::new(StatusCode::BAD_REQUEST, value.to_string())
    }
}

/// Returns the current process health status.
pub fn health_status() -> HealthStatus {
    HealthStatus {
        service: "finite-brain".to_owned(),
        status: "ok".to_owned(),
        core_crate: finite_brain_core::crate_name().to_owned(),
        store_crate: finite_brain_store::crate_name().to_owned(),
    }
}

/// Builds the development server router with an in-memory SQLite store.
pub fn router() -> Router {
    let store = BrainStore::open_in_memory().expect("in-memory store migration succeeds");
    router_with_state(ServerState::new(store, DEFAULT_PUBLIC_BASE_URL))
}

/// Build a router backed by an on-disk SQLite store.
pub fn router_with_sqlite_path(
    path: impl AsRef<Path>,
    public_base_url: impl Into<String>,
) -> Result<Router, StoreError> {
    Ok(router_with_state(server_state_with_sqlite_path(
        path,
        public_base_url,
    )?))
}

/// Build server state backed by an on-disk SQLite store.
pub fn server_state_with_sqlite_path(
    path: impl AsRef<Path>,
    public_base_url: impl Into<String>,
) -> Result<ServerState, StoreError> {
    Ok(ServerState::new(BrainStore::open(path)?, public_base_url))
}

/// Build a router backed by SQLite and an optional finite-identity Authority.
pub fn router_with_sqlite_path_and_identity_authority(
    path: impl AsRef<Path>,
    public_base_url: impl Into<String>,
    identity_authority_url: Option<String>,
) -> Result<Router, StoreError> {
    let mut state = server_state_with_sqlite_path(path, public_base_url)?;
    if let Some(url) = identity_authority_url {
        state = state.with_identity_authority_url(url);
    }
    Ok(router_with_state(state))
}

/// Build a router with explicit state.
pub fn router_with_state(state: ServerState) -> Router {
    let cors_state = state.clone();
    Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/smoke/bootstrap", get(bootstrap_smoke_handler))
        .route("/smoke/ui", get(smoke_ui_handler))
        .route("/smoke/ui.css", get(smoke_ui_css_handler))
        .route("/smoke/ui.js", get(smoke_ui_js_handler))
        .route("/client", get(product_client_handler))
        .route("/client/app.css", get(product_client_css_handler))
        .route("/client/app.js", get(product_client_js_handler))
        .route(
            "/client/fonts/funnel-display-500.ttf",
            get(product_client_funnel_display_500_font_handler),
        )
        .route(
            "/client/fonts/funnel-display-600.ttf",
            get(product_client_funnel_display_600_font_handler),
        )
        .route(
            "/client/fonts/funnel-display-700.ttf",
            get(product_client_funnel_display_700_font_handler),
        )
        .route(
            "/client/fonts/funnel-sans-400.ttf",
            get(product_client_funnel_sans_400_font_handler),
        )
        .route(
            "/client/fonts/funnel-sans-500.ttf",
            get(product_client_funnel_sans_500_font_handler),
        )
        .route(
            "/client/fonts/funnel-sans-600.ttf",
            get(product_client_funnel_sans_600_font_handler),
        )
        .route(
            "/client/fonts/funnel-sans-700.ttf",
            get(product_client_funnel_sans_700_font_handler),
        )
        .route(
            "/client/fonts/jetbrains-mono-400.ttf",
            get(product_client_jetbrains_mono_400_font_handler),
        )
        .route(
            "/client/fonts/jetbrains-mono-500.ttf",
            get(product_client_jetbrains_mono_500_font_handler),
        )
        .route(
            "/client/fonts/jetbrains-mono-600.ttf",
            get(product_client_jetbrains_mono_600_font_handler),
        )
        .route(
            "/client/smoke-nip07.js",
            get(product_client_smoke_nip07_js_handler),
        )
        .route("/client/config.json", get(product_client_config_handler))
        .route(
            "/_admin/brains",
            get(list_brains_handler).post(create_brain_handler),
        )
        .route("/_admin/identities/resolve", post(resolve_identity_handler))
        .route(
            "/_admin/brains/{brain_id}/metadata",
            get(brain_metadata_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/export",
            get(encrypted_brain_export_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/search",
            get(brain_search_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/members",
            post(add_member_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/members/{target_npub}",
            axum::routing::delete(remove_member_handler),
        )
        .route(
            "/_admin/personal-brain-bootstrap",
            post(bootstrap_personal_brain_for_agent_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/personal-agent",
            axum::routing::put(replace_personal_agent_handler),
        )
        .route("/_admin/brains/{brain_id}/admins", post(add_admin_handler))
        .route(
            "/_admin/brains/{brain_id}/collaborators/ensure-admin",
            post(ensure_organization_admin_handler)
                .layer(DefaultBodyLimit::max(MAX_COLLABORATION_REQUEST_BODY_BYTES)),
        )
        .route(
            "/_admin/brains/{brain_id}/admins/{target_npub}",
            axum::routing::delete(remove_admin_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/invitations",
            get(list_brain_invitations_handler).post(create_brain_invitation_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/invitations/{invitation_id}",
            axum::routing::delete(revoke_brain_invitation_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/invitations/{invitation_id}/accept",
            post(accept_brain_invitation_handler),
        )
        .route(
            "/_admin/brain-invitation-links/{invite_code}",
            get(get_brain_invitation_link_handler),
        )
        .route(
            "/_admin/brain-invitation-links/{invite_code}/llms.txt",
            get(public_brain_invitation_instructions_handler),
        )
        .route(
            "/_admin/brain-invitation-links/{invite_code}/instructions",
            post(post_proof_brain_invitation_instructions_handler),
        )
        .route(
            "/_admin/brain-invitation-links/{invite_code}/bootstrap",
            post(post_proof_brain_invitation_bootstrap_handler),
        )
        .route(
            "/_admin/brain-invitation-links/{invite_code}/accept",
            post(accept_brain_invitation_link_handler),
        )
        .route(
            "/_admin/brain-invitation-links/{invite_code}/claim",
            post(claim_email_brain_invitation_link_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/folders",
            post(create_folder_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/folders/{folder_id}/finish-setup",
            post(finish_folder_setup_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/folders/{folder_id}",
            axum::routing::delete(delete_folder_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/folders/{folder_id}/access",
            post(grant_folder_access_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/folders/{folder_id}/access/{target_npub}",
            axum::routing::delete(remove_folder_access_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/folders/{folder_id}/share-links",
            get(list_folder_share_links_handler).post(create_share_link_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/folders/{folder_id}/share-source",
            post(mark_shared_folder_source_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/folders/{folder_id}/shared-folder-invitations",
            post(create_shared_folder_invitation_handler),
        )
        .route(
            "/_admin/share-links/{share_link_id}",
            get(get_share_link_handler).delete(revoke_share_link_handler),
        )
        .route(
            "/_admin/share-links/{share_link_id}/accept",
            post(accept_share_link_handler),
        )
        .route(
            "/_admin/shared-folder-invitations/{invitation_id}",
            get(get_shared_folder_invitation_handler)
                .delete(revoke_shared_folder_invitation_handler),
        )
        .route(
            "/_admin/shared-folder-invitations/{invitation_id}/accept",
            post(accept_shared_folder_invitation_handler),
        )
        .route(
            "/_admin/shared-folder-connections/{connection_id}/members",
            axum::routing::patch(update_shared_folder_connection_members_handler),
        )
        .route(
            "/_admin/shared-folder-connections/{connection_id}",
            axum::routing::delete(revoke_shared_folder_connection_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/shared-folder-invitations",
            get(list_shared_folder_invitations_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/shared-folder-connections",
            get(list_shared_folder_connections_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/organization-folder-mounts",
            get(organization_folder_mounts_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/folders/{folder_id}/objects/{object_id}",
            get(get_object_handler)
                .put(put_object_handler)
                .delete(delete_object_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/folders/{folder_id}/objects/{object_id}/move",
            post(move_object_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/sync/bootstrap",
            get(sync_bootstrap_handler),
        )
        .route(
            "/_admin/brains/{brain_id}/sync/records",
            get(sync_records_handler).post(submit_sync_record_handler),
        )
        .layer(middleware::from_fn_with_state(
            cors_state,
            cors_allowlist_middleware,
        ))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .with_state(state)
}

mod routes;

use routes::*;

#[derive(Debug, Clone, Eq, PartialEq)]
struct ResolvedIdentity {
    public_key: NostrPublicKey,
    npub: String,
    hex: String,
    nip05: Option<String>,
    relays: Vec<String>,
}

fn default_nip05_fetcher() -> Nip05Fetcher {
    Arc::new(|request| fetch_nip05_document(request, &request.url))
}

fn identity_authority_nip05_fetcher(base_url: String) -> Nip05Fetcher {
    let internet = default_nip05_fetcher();
    Arc::new(move |request| {
        let Some(path_and_query) = request.url.strip_prefix(FINITE_VIP_NIP05_PREFIX) else {
            return internet(request);
        };
        fetch_nip05_document(request, &format!("{base_url}/{path_and_query}"))
    })
}

fn fetch_nip05_document(request: &Nip05WellKnownRequest, url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(NIP05_CONNECT_TIMEOUT_SECONDS))
        .timeout_read(Duration::from_secs(NIP05_READ_TIMEOUT_SECONDS))
        .redirects(0)
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| format!("NIP-05 lookup failed: {error}"))?;
    let mut bytes = Vec::new();
    let mut reader = response
        .into_reader()
        .take(request.max_response_bytes.saturating_add(1) as u64);
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| format!("NIP-05 response read failed: {error}"))?;
    if bytes.len() > request.max_response_bytes {
        return Err(format!(
            "NIP-05 document exceeded {} bytes",
            request.max_response_bytes
        ));
    }
    Ok(bytes)
}

async fn resolve_identity_input(
    state: &ServerState,
    input: &str,
) -> Result<ResolvedIdentity, ApiError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "identity input is required",
        ));
    }

    if let Ok(public_key) = NostrPublicKey::parse(input) {
        return resolved_identity(public_key, None, Vec::new());
    }

    let identifier = Nip05Identifier::parse(input).map_err(nostr_identity_error)?;
    let request = identifier.well_known_request();
    let fetcher = state.nip05_fetcher.clone();
    let document = run_authority_blocking("NIP-05 lookup", move || {
        fetcher(&request).map_err(|_| AuthorityFailure::Transport)
    })
    .await?;
    if document.len() > MAX_NIP05_DOCUMENT_BYTES {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("NIP-05 document exceeded {MAX_NIP05_DOCUMENT_BYTES} bytes"),
        ));
    }
    let document = Nip05WellKnownDocument::from_json(&document).map_err(nostr_identity_error)?;
    let verified = document
        .resolve(&identifier)
        .map_err(nostr_identity_error)?;
    resolved_identity(
        verified.public_key(),
        Some(verified.identifier().as_str().to_owned()),
        verified.relays().to_vec(),
    )
}

async fn resolve_and_record_identity(
    state: &ServerState,
    input: &str,
) -> Result<ResolvedIdentityResponse, ApiError> {
    let resolved = resolve_identity_input(state, input).await?;
    record_resolved_identity(state, resolved)
}

fn record_resolved_identity(
    state: &ServerState,
    resolved: ResolvedIdentity,
) -> Result<ResolvedIdentityResponse, ApiError> {
    let now = server_timestamp(state);
    let alias = IdentityAlias {
        npub: UserId::new(resolved.npub.clone())?,
        hex_public_key: resolved.hex.clone(),
        preferred_nip05: resolved.nip05.clone(),
        nip05_verified_at: resolved.nip05.as_ref().map(|_| now.clone()),
        nip05_relays: resolved.relays.clone(),
        updated_at: now.clone(),
    };
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        store.record_identity_alias(&alias)?;
    }

    Ok(ResolvedIdentityResponse {
        npub: resolved.npub.clone(),
        response: identity_response_from_resolved(resolved, alias.nip05_verified_at),
    })
}

async fn resolve_managed_agent_email(
    state: &ServerState,
    email: &str,
    expected_owner_npub: &UserId,
) -> Result<ResolvedIdentity, ApiError> {
    let managed_agent_email = canonical_email(email)?;
    let resolved = resolve_identity_input(state, &managed_agent_email).await?;
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Managed Agent resolution is not configured",
        )
    })?;
    let agent: IdentityAgentResolutionResponse = post_authority_json(
        &format!(
            "{}/api/v1/operator/brain/agent-resolution",
            authorities.identity_base_url
        ),
        "X-Finite-Operator-Token",
        &authorities.identity_token,
        &serde_json::json!({ "agentNpub": resolved.npub }),
        "Finite Identity Managed Agent resolution",
    )
    .await?;
    if agent.agent_npub != resolved.npub
        || canonical_email(&agent.managed_agent_email)? != managed_agent_email
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Finite Identity returned a mismatched Managed Agent",
        ));
    }
    let account: CoreAgentAccountResponse = post_authority_json(
        &format!(
            "{}/api/core/v1/brain/agent-account",
            authorities.core_base_url
        ),
        "Authorization",
        &format!("Bearer {}", authorities.core_token),
        &serde_json::json!({ "managedAgentEmail": managed_agent_email }),
        "Finite Core Managed Agent resolution",
    )
    .await?;
    if account.status != "active"
        || canonical_email(&account.managed_agent_email)? != managed_agent_email
        || account.workos_user_id.trim().is_empty()
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Finite Core returned an inactive or mismatched Managed Agent",
        ));
    }
    let owner: IdentityUserResolutionResponse = post_authority_json(
        &format!(
            "{}/api/v1/operator/brain/user-resolution",
            authorities.identity_base_url
        ),
        "X-Finite-Operator-Token",
        &authorities.identity_token,
        &serde_json::json!({ "workosUserId": account.workos_user_id }),
        "Finite Identity Managed Agent owner resolution",
    )
    .await?;
    if owner.workos_user_id != account.workos_user_id
        || UserId::new(owner.user_npub)? != *expected_owner_npub
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Managed Agent does not belong to the Personal Brain owner's account",
        ));
    }
    Ok(resolved)
}

async fn resolve_account_agent_principals(
    state: &ServerState,
    agent_npub: &UserId,
) -> Result<AccountAgentPrincipals, ApiError> {
    let authorities = state.agent_bootstrap_authorities.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
    })?;
    let agent: IdentityAgentResolutionResponse = post_authority_json(
        &format!(
            "{}/api/v1/operator/brain/agent-resolution",
            authorities.identity_base_url
        ),
        "X-Finite-Operator-Token",
        &authorities.identity_token,
        &serde_json::json!({ "agentNpub": agent_npub.as_str() }),
        "Finite Identity Agent Principal resolution",
    )
    .await?;
    let resolved_agent = UserId::new(agent.agent_npub)?;
    if resolved_agent != *agent_npub {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Finite Identity returned a mismatched Agent Principal",
        ));
    }
    let managed_agent_email = canonical_email(&agent.managed_agent_email)?;

    let account: CoreAgentAccountResponse = post_authority_json(
        &format!(
            "{}/api/core/v1/brain/agent-account",
            authorities.core_base_url
        ),
        "Authorization",
        &format!("Bearer {}", authorities.core_token),
        &serde_json::json!({ "managedAgentEmail": managed_agent_email }),
        "Finite Core account-agent resolution",
    )
    .await?;
    if account.status != "active"
        || account.managed_agent_email.trim().to_ascii_lowercase() != managed_agent_email
        || account.workos_user_id.trim().is_empty()
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Finite Core returned an inactive or mismatched account-agent association",
        ));
    }

    let owner: IdentityUserResolutionResponse = post_authority_json(
        &format!(
            "{}/api/v1/operator/brain/user-resolution",
            authorities.identity_base_url
        ),
        "X-Finite-Operator-Token",
        &authorities.identity_token,
        &serde_json::json!({ "workosUserId": account.workos_user_id }),
        "Finite Identity User Nostr Identity resolution",
    )
    .await?;
    if owner.workos_user_id != account.workos_user_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Finite Identity returned a mismatched WorkOS account",
        ));
    }
    let owner_npub = UserId::new(owner.user_npub)?;
    if owner_npub == *agent_npub {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "User and Agent Principals must be distinct",
        ));
    }

    let owner_email = account
        .verified_email
        .as_deref()
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "Finite Core account-agent association has no verified owner email",
            )
        })
        .and_then(canonical_email)?;

    Ok(AccountAgentPrincipals {
        owner_npub,
        agent_npub: agent_npub.clone(),
        owner_email,
        managed_agent_email,
    })
}

fn account_agent_identity_aliases(
    principals: &AccountAgentPrincipals,
    updated_at: &str,
) -> Result<[IdentityAlias; 2], ApiError> {
    let owner_key =
        NostrPublicKey::parse(principals.owner_npub.as_str()).map_err(nostr_identity_error)?;
    let agent_key =
        NostrPublicKey::parse(principals.agent_npub.as_str()).map_err(nostr_identity_error)?;
    Ok([
        IdentityAlias {
            npub: principals.owner_npub.clone(),
            hex_public_key: owner_key.to_hex(),
            preferred_nip05: Some(principals.owner_email.clone()),
            nip05_verified_at: Some(updated_at.to_owned()),
            nip05_relays: Vec::new(),
            updated_at: updated_at.to_owned(),
        },
        IdentityAlias {
            npub: principals.agent_npub.clone(),
            hex_public_key: agent_key.to_hex(),
            preferred_nip05: Some(principals.managed_agent_email.clone()),
            nip05_verified_at: Some(updated_at.to_owned()),
            nip05_relays: Vec::new(),
            updated_at: updated_at.to_owned(),
        },
    ])
}

const AUTHORITY_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const AUTHORITY_IO_TIMEOUT: Duration = Duration::from_secs(3);
const AUTHORITY_OVERALL_TIMEOUT: Duration = Duration::from_secs(5);
const AUTHORITY_RESPONSE_MAX_BYTES: u64 = 64 * 1024;
const AUTHORITY_MAX_CONCURRENCY: usize = 16;

fn authority_concurrency() -> &'static tokio::sync::Semaphore {
    static SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SEMAPHORE.get_or_init(|| tokio::sync::Semaphore::new(AUTHORITY_MAX_CONCURRENCY))
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AuthorityFailure {
    Timeout,
    Transport,
    Status,
    Oversized,
    Malformed,
    Worker,
}

async fn run_authority_blocking<T, F>(operation: &str, action: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AuthorityFailure> + Send + 'static,
{
    let permit = tokio::time::timeout(AUTHORITY_OVERALL_TIMEOUT, authority_concurrency().acquire())
        .await
        .map_err(|_| AuthorityFailure::Timeout.api_error(operation))?
        .map_err(|_| AuthorityFailure::Worker.api_error(operation))?;
    let worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        action()
    });
    tokio::time::timeout(AUTHORITY_OVERALL_TIMEOUT, worker)
        .await
        .map_err(|_| AuthorityFailure::Timeout.api_error(operation))?
        .map_err(|_| AuthorityFailure::Worker.api_error(operation))?
        .map_err(|failure| failure.api_error(operation))
}

impl AuthorityFailure {
    fn api_error(self, operation: &str) -> ApiError {
        let (status, category) = match self {
            Self::Timeout => (StatusCode::GATEWAY_TIMEOUT, "timeout"),
            Self::Transport => (StatusCode::BAD_GATEWAY, "transport"),
            Self::Status => (StatusCode::BAD_GATEWAY, "upstream-status"),
            Self::Oversized => (StatusCode::BAD_GATEWAY, "oversized-response"),
            Self::Malformed => (StatusCode::BAD_GATEWAY, "malformed-response"),
            Self::Worker => (StatusCode::SERVICE_UNAVAILABLE, "worker-unavailable"),
        };
        ApiError::new(status, format!("{operation} authority failure: {category}"))
    }
}

async fn post_authority_json<T>(
    url: &str,
    auth_header: &str,
    auth_value: &str,
    request: &serde_json::Value,
    operation: &str,
) -> Result<T, ApiError>
where
    T: for<'de> Deserialize<'de> + Send + 'static,
{
    let body = serde_json::to_string(request).map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not encode {operation} request: {error}"),
        )
    })?;
    let permit = tokio::time::timeout(AUTHORITY_OVERALL_TIMEOUT, authority_concurrency().acquire())
        .await
        .map_err(|_| AuthorityFailure::Timeout.api_error(operation))?
        .map_err(|_| AuthorityFailure::Worker.api_error(operation))?;
    let url = url.to_owned();
    let auth_header = auth_header.to_owned();
    let auth_value = auth_value.to_owned();
    let worker = tokio::task::spawn_blocking(move || -> Result<T, AuthorityFailure> {
        // Keep admission for the blocking worker's complete lifetime. An
        // overall timeout detaches spawn_blocking work, so dropping the permit
        // in the async caller would otherwise allow a stalled worker pile-up.
        let _permit = permit;
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(AUTHORITY_CONNECT_TIMEOUT)
            .timeout_read(AUTHORITY_IO_TIMEOUT)
            .timeout_write(AUTHORITY_IO_TIMEOUT)
            .build();
        let response = agent
            .post(&url)
            .set(&auth_header, &auth_value)
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(|error| match error {
                ureq::Error::Status(_, _) => AuthorityFailure::Status,
                ureq::Error::Transport(transport)
                    if transport.kind() == ureq::ErrorKind::Io
                        && transport
                            .message()
                            .is_some_and(|message| message.contains("timed out")) =>
                {
                    AuthorityFailure::Timeout
                }
                ureq::Error::Transport(_) => AuthorityFailure::Transport,
            })?;
        if response
            .header("Content-Length")
            .and_then(|length| length.parse::<u64>().ok())
            .is_some_and(|length| length > AUTHORITY_RESPONSE_MAX_BYTES)
        {
            return Err(AuthorityFailure::Oversized);
        }
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(AUTHORITY_RESPONSE_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::TimedOut {
                    AuthorityFailure::Timeout
                } else {
                    AuthorityFailure::Transport
                }
            })?;
        if bytes.len() as u64 > AUTHORITY_RESPONSE_MAX_BYTES {
            return Err(AuthorityFailure::Oversized);
        }
        serde_json::from_slice(&bytes).map_err(|_| AuthorityFailure::Malformed)
    });
    let result = tokio::time::timeout(AUTHORITY_OVERALL_TIMEOUT, worker)
        .await
        .map_err(|_| AuthorityFailure::Timeout.api_error(operation))?
        .map_err(|_| AuthorityFailure::Worker.api_error(operation))?;
    result.map_err(|failure| failure.api_error(operation))
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ResolvedIdentityResponse {
    npub: String,
    response: IdentityResponse,
}

fn resolved_identity(
    public_key: NostrPublicKey,
    nip05: Option<String>,
    relays: Vec<String>,
) -> Result<ResolvedIdentity, ApiError> {
    let npub = public_key.to_npub().map_err(nostr_identity_error)?;
    Ok(ResolvedIdentity {
        public_key,
        npub,
        hex: public_key.to_hex(),
        nip05,
        relays,
    })
}

fn nostr_identity_error(error: NostrPrimitiveError) -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        format!("invalid identity input: {error}"),
    )
}

fn identity_response_from_resolved(
    resolved: ResolvedIdentity,
    verified_at: Option<String>,
) -> IdentityResponse {
    IdentityResponse {
        display: resolved
            .nip05
            .clone()
            .unwrap_or_else(|| resolved.npub.clone()),
        npub: resolved.npub,
        hex: resolved.hex,
        nip05: resolved.nip05,
        relays: resolved.relays,
        verified_at,
    }
}

fn identity_response_from_alias(alias: IdentityAlias) -> IdentityResponse {
    IdentityResponse {
        display: alias
            .preferred_nip05
            .clone()
            .unwrap_or_else(|| alias.npub.to_string()),
        npub: alias.npub.to_string(),
        hex: alias.hex_public_key,
        nip05: alias.preferred_nip05,
        relays: alias.nip05_relays,
        verified_at: alias.nip05_verified_at,
    }
}

fn known_identity_responses(
    store: &BrainStore,
    npubs: impl IntoIterator<Item = String>,
) -> Result<Vec<IdentityResponse>, ApiError> {
    let mut ids = BTreeSet::new();
    for npub in npubs {
        if !npub.is_empty() {
            ids.insert(UserId::new(npub)?);
        }
    }
    let ids = ids.into_iter().collect::<Vec<_>>();
    let aliases = store.load_identity_aliases(&ids)?;
    Ok(aliases
        .into_iter()
        .map(identity_response_from_alias)
        .collect())
}

fn invitation_target_input(request: &CreateBrainInvitationRequest) -> Result<String, ApiError> {
    request
        .target
        .as_deref()
        .or(request.target_email.as_deref())
        .or(request.target_npub.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invitation target is required"))
}

fn email_like(value: &str) -> bool {
    let value = value.trim();
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty() && !domain.is_empty() && domain.contains('.')
}

fn finite_vip_email(value: &str) -> bool {
    canonical_email(value)
        .map(|email| email.ends_with("@finite.vip"))
        .unwrap_or(false)
}

fn canonical_email(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_ascii_lowercase();
    let Some((local, domain)) = value.split_once('@') else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "email target must be an email address",
        ));
    };
    if local.is_empty()
        || domain.is_empty()
        || value.chars().any(|c| c == '\0' || c.is_control())
        || value.len() > 320
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "email target must be a printable email address",
        ));
    }
    Ok(value)
}

fn public_invite_instructions_path(invite_code: &str) -> String {
    format!("/_admin/brain-invitation-links/{invite_code}/llms.txt")
}

fn absolute_public_url(state: &ServerState, path: &str) -> String {
    format!("{}{}", state.public_base_url.trim_end_matches('/'), path)
}

fn attach_invitation_public_url(state: &ServerState, response: &mut BrainInvitationResponse) {
    response.public_instructions_path = public_invite_instructions_path(&response.invite_code);
    response.public_instructions_url = Some(absolute_public_url(
        state,
        &response.public_instructions_path,
    ));
}

fn text_response(text: String) -> Response {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        text,
    )
        .into_response()
}

fn invite_email_payload(
    state: &ServerState,
    invited_email: &str,
    invite_code: &str,
) -> BrainInviteEmail {
    let instructions_path = public_invite_instructions_path(invite_code);
    let instructions_url = absolute_public_url(state, &instructions_path);
    BrainInviteEmail {
        to: invited_email.to_owned(),
        subject: "Brain invitation".to_owned(),
        text: format!(
            "You have a Brain invitation.\n\n\
             Start with the public agent instructions:\n{instructions_url}\n\n\
             Invite code: {invite_code}\n\n\
             This email intentionally does not include an Invite Secret or a full fragment URL. \
             Keep any URL fragment or inviteSecret value client-side, and never paste it into \
             server-visible logs, query strings, analytics redirects, or email replies."
        ),
    }
}

fn deliver_email_invitation(
    state: &ServerState,
    invitation: &StoredBrainInvitation,
) -> Result<Option<String>, ApiError> {
    let Some(invited_email) = invitation.invited_email.as_deref() else {
        return Ok(None);
    };
    let Some(mailer) = state.invite_mailer.as_ref() else {
        return Ok(Some("not_configured".to_owned()));
    };
    let email = invite_email_payload(state, invited_email, &invitation.invite_code);
    mailer(&email).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("Brain invite email delivery failed: {error}"),
        )
    })?;
    Ok(Some("sent".to_owned()))
}

fn resend_invite_mailer(api_key: String, from: String) -> BrainInviteMailer {
    Arc::new(move |email| {
        let body = serde_json::to_string(&serde_json::json!({
            "from": from,
            "to": [email.to],
            "subject": email.subject,
            "text": email.text,
        }))
        .map_err(|error| format!("could not encode Resend invite email: {error}"))?;
        ureq::post("https://api.resend.com/emails")
            .set("Authorization", &format!("Bearer {api_key}"))
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(|error| format!("Resend request failed: {error}"))?;
        Ok(())
    })
}

fn postmark_invite_mailer(server_token: String, from: String) -> BrainInviteMailer {
    Arc::new(move |email| {
        let body = serde_json::to_string(&serde_json::json!({
            "From": from,
            "To": email.to,
            "Subject": email.subject,
            "TextBody": email.text,
            "TrackOpens": false,
            "TrackLinks": "None",
        }))
        .map_err(|error| format!("could not encode Postmark invite email: {error}"))?;
        ureq::post("https://api.postmarkapp.com/email")
            .set("X-Postmark-Server-Token", &server_token)
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(|error| format!("Postmark request failed: {error}"))?;
        Ok(())
    })
}

fn public_invite_instructions_text() -> String {
    "FiniteBrain public invite instructions\n\n\
     This public page is safe to read before email proof. It intentionally omits \
     the invited email, Brain identity, Folder identity, access scope, claim state, \
     Folder Keys, bootstrap plaintext, and encrypted invite structure.\n\n\
     Workflow:\n\
     1. Prove control of the invited email through finite-identity.\n\
     2. Act with the Nostr key that will become your FiniteBrain User npub.\n\
     3. Keep any URL fragment or inviteSecret value client-side. Never paste it \
     into server-visible request bodies, query strings, logs, analytics redirects, \
     email replies, or issue trackers.\n\
     4. After email proof, request authenticated post-proof instructions from this \
     invite URL to receive the scoped claim, open, and sync steps.\n\
     5. Only a trusted FiniteBrain client or agent runtime should unwrap bootstrap \
     material and create durable claim grants.\n"
        .to_owned()
}

fn access_label(access: FolderAccessMode) -> &'static str {
    match access {
        FolderAccessMode::Owner => "owner",
        FolderAccessMode::AdminOnly => "admin_only",
        FolderAccessMode::AllMembers => "all_members",
        FolderAccessMode::Restricted => "restricted",
    }
}

fn status_label(status: LinkStatus) -> &'static str {
    match status {
        LinkStatus::Pending => "pending",
        LinkStatus::Accepted => "accepted",
        LinkStatus::Revoked => "revoked",
    }
}

fn post_proof_invite_instructions_text(
    state: &ServerState,
    invitation: &StoredBrainInvitation,
    stored: &StoredBrain,
) -> String {
    let mut text = format!(
        "FiniteBrain post-proof invite instructions\n\n\
         Invited email: {}\n\
         Claiming status: {}\n\
         Brain: {} ({})\n\
         Claim endpoint: {}{}\n\
         Public instructions: {}\n\n\
         Authorized initial Folder scope:\n",
        invitation.invited_email.as_deref().unwrap_or("unknown"),
        status_label(invitation.status),
        stored.brain.name,
        stored.brain.id,
        state.public_base_url.trim_end_matches('/'),
        invitation.accept_path,
        absolute_public_url(
            state,
            &public_invite_instructions_path(&invitation.invite_code)
        )
    );
    for scope in &invitation.bootstrap_scope {
        let name = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == scope.folder_id)
            .map(|folder| folder.name.to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        text.push_str(&format!(
            "- {} (id: {}, access: {}, expected key version: {})\n",
            name,
            scope.folder_id,
            access_label(scope.access),
            scope.key_version
        ));
    }
    text.push_str(
        "\nWorkflow:\n\
         1. Keep the Invite Secret in local client memory. Do not send it to the server.\n\
         2. Locally unwrap the bootstrap material with the Invite Secret.\n\
         3. Sign an Invite Unwrap Proof with the temporary Invite Unwrap Key.\n\
         4. Submit the claim request with emailProofCreatedAt, inviteUnwrapProofEventJson, \
         and durable npub-bound grant envelopes for exactly the Folder scope above.\n\
         5. After claim succeeds, open or reuse a Brain Working Tree intentionally, then sync.\n\n\
         This authenticated instruction response still does not include Folder Keys, \
         decrypted bootstrap payloads, auth files, or decrypted Brain content.\n",
    );
    text
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityAuthoritySatisfiesGrantResponse {
    satisfied: bool,
}

fn identity_authority_email_proof_verifier(base_url: String) -> EmailProofVerifier {
    Arc::new(move |email, actor| {
        let actor_hex = NostrPublicKey::parse(actor.as_str())
            .map_err(|error| {
                EmailProofFailure::Rejected(format!(
                    "invalid claimant npub for Identity Authority: {error}"
                ))
            })?
            .to_hex();
        let body = serde_json::to_vec(&serde_json::json!({
            "grant": email,
            "actor_pubkey": actor_hex,
        }))
        .map_err(|_| EmailProofFailure::Authority(AuthorityFailure::Malformed))?;
        let url = format!("{base_url}/api/v1/principal-resolution/satisfies-grant");
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(AUTHORITY_CONNECT_TIMEOUT)
            .timeout_read(AUTHORITY_IO_TIMEOUT)
            .timeout_write(AUTHORITY_IO_TIMEOUT)
            .redirects(0)
            .build();
        let response = agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_bytes(&body)
            .map_err(|error| {
                EmailProofFailure::Authority(match error {
                    ureq::Error::Status(_, _) => AuthorityFailure::Status,
                    ureq::Error::Transport(transport)
                        if transport.kind() == ureq::ErrorKind::Io
                            && transport
                                .message()
                                .is_some_and(|message| message.contains("timed out")) =>
                    {
                        AuthorityFailure::Timeout
                    }
                    ureq::Error::Transport(_) => AuthorityFailure::Transport,
                })
            })?;
        let mut body = Vec::new();
        response
            .into_reader()
            .take(AUTHORITY_RESPONSE_MAX_BYTES + 1)
            .read_to_end(&mut body)
            .map_err(|error| {
                EmailProofFailure::Authority(if error.kind() == std::io::ErrorKind::TimedOut {
                    AuthorityFailure::Timeout
                } else {
                    AuthorityFailure::Transport
                })
            })?;
        if body.len() as u64 > AUTHORITY_RESPONSE_MAX_BYTES {
            return Err(EmailProofFailure::Authority(AuthorityFailure::Oversized));
        }
        let response: IdentityAuthoritySatisfiesGrantResponse = serde_json::from_slice(&body)
            .map_err(|_| EmailProofFailure::Authority(AuthorityFailure::Malformed))?;
        if response.satisfied {
            Ok(())
        } else {
            Err(EmailProofFailure::Rejected(
                "Identity Authority does not confirm this npub controls the invited email"
                    .to_owned(),
            ))
        }
    })
}

async fn verify_identity_authority_email_proof(
    state: &ServerState,
    invited_email: &str,
    claimant: &UserId,
) -> Result<(), ApiError> {
    let verifier = state.email_proof_verifier.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Identity Authority email proof verifier is not configured",
        )
    })?;
    let verifier = verifier.clone();
    let invited_email = invited_email.to_owned();
    let claimant = claimant.clone();
    let verification =
        run_authority_blocking("Finite Identity email proof", move || {
            match verifier(&invited_email, &claimant) {
                Ok(()) => Ok(Ok(())),
                Err(EmailProofFailure::Authority(failure)) => Err(failure),
                Err(EmailProofFailure::Rejected(error)) => Ok(Err(error)),
            }
        })
        .await?;
    verification.map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("email proof was not accepted: {error}"),
        )
    })
}

fn selected_folder_ids(values: &[String]) -> Result<Vec<FolderId>, ApiError> {
    values
        .iter()
        .cloned()
        .map(FolderId::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ApiError::from)
}

fn email_bootstrap_scope_for_brain(
    stored: &StoredBrain,
    selected_restricted_folder_access: &[FolderId],
) -> Result<Vec<EmailInviteBootstrapScopeFolder>, ApiError> {
    let selected = selected_restricted_folder_access
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut seen_selected = BTreeSet::new();
    let mut included = BTreeSet::new();
    let mut scope = Vec::new();
    for folder in &stored.brain.folders {
        let selected_folder = selected.contains(&folder.id);
        if selected_folder {
            seen_selected.insert(folder.id.clone());
        }
        let include = match folder.access {
            FolderAccessMode::AllMembers => true,
            FolderAccessMode::Restricted => selected_folder,
            FolderAccessMode::Owner | FolderAccessMode::AdminOnly => {
                if selected_folder {
                    return Err(ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "email bootstrap initial folder access supports all-members and restricted folders only",
                    ));
                }
                false
            }
        };
        if include && included.insert(folder.id.clone()) {
            scope.push(EmailInviteBootstrapScopeFolder {
                folder_id: folder.id.clone(),
                access: folder.access,
                key_version: folder.current_key_version,
            });
        }
    }
    if seen_selected != selected {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "folder not found"));
    }
    Ok(scope)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmailInviteBootstrapAuthorizationPayload {
    version: String,
    brain_id: String,
    invited_email: String,
    invite_unwrap_npub: String,
    bootstrap_payload_hash: String,
    expires_at: String,
    folders: Vec<EmailInviteBootstrapAuthorizationFolder>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmailInviteBootstrapAuthorizationFolder {
    folder_id: String,
    access: FolderAccessMode,
    key_version: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmailInviteBootstrapClaimProofPayload {
    version: String,
    brain_id: String,
    invite_code: String,
    invited_email: String,
    claimant_npub: String,
    bootstrap_payload_hash: String,
    email_proof_created_at: String,
}

#[allow(clippy::too_many_arguments)]
fn validate_email_bootstrap_authorization(
    event_json: &str,
    admin_npub: &str,
    brain_id: &BrainId,
    invited_email: &str,
    invite_unwrap_npub: &UserId,
    bootstrap_payload_hash: &str,
    expires_at: &str,
    scope: &[EmailInviteBootstrapScopeFolder],
) -> Result<(), ApiError> {
    let event = Event::from_json(event_json).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "email bootstrap authorization event JSON did not parse",
        )
    })?;
    if event.kind.as_u16() != APP_SPECIFIC_KIND {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("email bootstrap authorization event must be kind {APP_SPECIFIC_KIND}"),
        ));
    }
    verify_event_integrity(&event).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("email bootstrap authorization event failed verification: {error}"),
        )
    })?;
    let signer = NostrPublicKey::from_protocol(event.pubkey)
        .to_npub()
        .map_err(nostr_identity_error)?;
    if signer != admin_npub {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "email bootstrap authorization signer must match the creating admin",
        ));
    }

    let payload: EmailInviteBootstrapAuthorizationPayload = serde_json::from_str(&event.content)
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "email bootstrap authorization content did not parse",
            )
        })?;
    if payload.version != "finite-email-invite-bootstrap-authorization-v1"
        || payload.brain_id != brain_id.as_str()
        || canonical_email(&payload.invited_email)? != invited_email
        || payload.invite_unwrap_npub != invite_unwrap_npub.as_str()
        || payload.bootstrap_payload_hash != bootstrap_payload_hash
        || payload.expires_at != expires_at
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "email bootstrap authorization does not match invitation metadata",
        ));
    }

    let authorized_scope = payload
        .folders
        .into_iter()
        .map(|folder| {
            Ok(EmailInviteBootstrapScopeFolder {
                folder_id: FolderId::new(folder.folder_id)?,
                access: folder.access,
                key_version: folder.key_version,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    if authorized_scope != scope {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "email bootstrap authorization does not match Folder scope",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_email_bootstrap_claim_proof(
    event_json: &str,
    invite_unwrap_npub: &UserId,
    brain_id: &BrainId,
    invite_code: &str,
    invited_email: &str,
    claimant_npub: &UserId,
    bootstrap_payload_hash: &str,
    email_proof_created_at: &str,
) -> Result<(), ApiError> {
    let event = Event::from_json(event_json).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "email bootstrap claim proof event JSON did not parse",
        )
    })?;
    if event.kind.as_u16() != APP_SPECIFIC_KIND {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("email bootstrap claim proof event must be kind {APP_SPECIFIC_KIND}"),
        ));
    }
    verify_event_integrity(&event).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("email bootstrap claim proof event failed verification: {error}"),
        )
    })?;
    let signer = NostrPublicKey::from_protocol(event.pubkey)
        .to_npub()
        .map_err(nostr_identity_error)?;
    if signer != invite_unwrap_npub.as_str() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "email bootstrap claim proof signer must match the Invite Unwrap npub",
        ));
    }

    let payload: EmailInviteBootstrapClaimProofPayload = serde_json::from_str(&event.content)
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "email bootstrap claim proof content did not parse",
            )
        })?;
    if payload.version != "finite-email-invite-bootstrap-claim-proof-v1"
        || payload.brain_id != brain_id.as_str()
        || payload.invite_code != invite_code
        || canonical_email(&payload.invited_email)? != invited_email
        || UserId::new(payload.claimant_npub)? != *claimant_npub
        || payload.bootstrap_payload_hash != bootstrap_payload_hash
        || payload.email_proof_created_at != email_proof_created_at
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "email bootstrap claim proof does not match invitation metadata",
        ));
    }
    Ok(())
}

fn validate_email_proof_window(
    invitation: &StoredBrainInvitation,
    proof_created_at: &str,
    now: &str,
) -> Result<(), ApiError> {
    let proof = OffsetDateTime::parse(proof_created_at, &Rfc3339).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "emailProofCreatedAt must be an RFC3339 timestamp",
        )
    })?;
    let created_at = OffsetDateTime::parse(&invitation.created_at, &Rfc3339).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "stored invitation timestamp did not parse",
        )
    })?;
    let now = OffsetDateTime::parse(now, &Rfc3339).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server timestamp did not parse",
        )
    })?;
    if proof < created_at {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "email proof must not be older than the invitation",
        ));
    }
    if proof > now + time::Duration::minutes(1) || now - proof > time::Duration::days(1) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "email proof must be no more than 24 hours old",
        ));
    }
    Ok(())
}

fn enrich_metadata_identities(
    store: &BrainStore,
    response: &mut BrainMetadataResponse,
) -> Result<(), ApiError> {
    let mut npubs = Vec::new();
    if let Some(owner) = &response.owner_user_id {
        npubs.push(owner.clone());
    }
    if let Some(personal_agent) = &response.personal_agent {
        npubs.push(personal_agent.agent_npub.clone());
    }
    npubs.extend(response.members.iter().cloned());
    npubs.extend(response.admins.iter().cloned());
    for folder in &response.folders {
        npubs.extend(folder.access_user_ids.iter().cloned());
    }
    response.identities = known_identity_responses(store, npubs)?;
    Ok(())
}

fn enrich_brain_invitation_identities(
    store: &BrainStore,
    response: &mut BrainInvitationResponse,
) -> Result<(), ApiError> {
    let mut npubs = Vec::new();
    if let Some(user_id) = &response.user_id {
        npubs.push(user_id.clone());
    }
    if let Some(claimed_by_npub) = &response.claimed_by_npub {
        npubs.push(claimed_by_npub.clone());
    }
    if let Some(invite_unwrap_npub) = &response.invite_unwrap_npub {
        npubs.push(invite_unwrap_npub.clone());
    }
    response.identities = known_identity_responses(store, npubs)?;
    Ok(())
}

fn enrich_share_link_identities(
    store: &BrainStore,
    response: &mut ShareLinkResponse,
) -> Result<(), ApiError> {
    response.identities = known_identity_responses(
        store,
        [
            response.recipient_npub.clone(),
            response.created_by_npub.clone(),
        ],
    )?;
    Ok(())
}

fn enrich_shared_folder_invitation_identities(
    store: &BrainStore,
    response: &mut SharedFolderInvitationResponse,
) -> Result<(), ApiError> {
    response.identities = known_identity_responses(
        store,
        [
            response.destination_admin_npub.clone(),
            response.created_by_npub.clone(),
        ],
    )?;
    Ok(())
}

fn enrich_shared_folder_connection_identities(
    store: &BrainStore,
    response: &mut SharedFolderConnectionResponse,
) -> Result<(), ApiError> {
    let mut npubs = vec![response.destination_admin_npub.clone()];
    npubs.extend(response.member_npubs.iter().cloned());
    response.identities = known_identity_responses(store, npubs)?;
    Ok(())
}

fn event_from_value(value: serde_json::Value) -> Result<Event, ApiError> {
    Event::from_json(value.to_string()).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "signed Nostr event JSON did not parse",
        )
    })
}

fn validate_admin_access_change_value(
    value: serde_json::Value,
    brain_id: &BrainId,
    admin_npub: &str,
    action: AdminAccessAction,
    folder_id: Option<&FolderId>,
    target_npub: Option<&str>,
    key_version: Option<u32>,
) -> Result<(Event, AdminAccessChangePayload), ApiError> {
    let event = event_from_value(value)?;
    if event.kind.as_u16() != APP_SPECIFIC_KIND {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("admin access-change event must be kind {APP_SPECIFIC_KIND}"),
        ));
    }
    let hint: AdminAccessChangePayload = serde_json::from_str(&event.content).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "admin access-change event content did not parse",
        )
    })?;
    let expected = AdminAccessChangeValidation {
        brain_id: brain_id.clone(),
        change_id: hint.change_id,
        action,
        admin_npub: admin_npub.to_owned(),
        folder_id: folder_id.cloned(),
        target_npub: target_npub.map(ToOwned::to_owned),
        key_version,
        note: hint.note,
        created_at: expected_created_at(&event)?,
    };
    let payload = validate_admin_access_change_event(&event, &expected)?;
    Ok((event, payload))
}

fn mutate_as_admin<F>(
    state: ServerState,
    brain_id: BrainId,
    actor_npub: String,
    event: Event,
    payload: AdminAccessChangePayload,
    mutation: F,
) -> Result<BrainMetadataResponse, ApiError>
where
    F: FnOnce(&mut BrainStore, &BrainId) -> Result<(), StoreError>,
{
    mutate_as_admin_with_grants(
        state,
        brain_id,
        actor_npub,
        event,
        payload,
        Vec::new(),
        mutation,
    )
}

fn mutate_as_admin_with_grants<F>(
    state: ServerState,
    brain_id: BrainId,
    actor_npub: String,
    event: Event,
    payload: AdminAccessChangePayload,
    grants: Vec<FolderKeyGrantMetadata>,
    mutation: F,
) -> Result<BrainMetadataResponse, ApiError>
where
    F: FnOnce(&mut BrainStore, &BrainId) -> Result<(), StoreError>,
{
    let response = {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor_npub)?;
        mutation(&mut store, &brain_id)?;
        for grant in &grants {
            append_folder_key_grant_record(&mut store, &brain_id, grant)?;
        }
        append_admin_access_change_record(&mut store, &brain_id, &actor_npub, &event, &payload)?;
        let stored = store.load_brain(&brain_id)?;
        let mut response = metadata_response(stored);
        enrich_metadata_identities(&store, &mut response)?;
        response
    };
    Ok(response)
}

fn append_folder_key_grant_record(
    store: &mut BrainStore,
    brain_id: &BrainId,
    grant: &FolderKeyGrantMetadata,
) -> Result<(), ApiError> {
    store.submit_sync_record(brain_id, &folder_key_grant_sync_record(grant)?)?;
    Ok(())
}

fn folder_key_grant_sync_record(
    grant: &FolderKeyGrantMetadata,
) -> Result<SyncRecordInput, ApiError> {
    let event = Event::from_json(grant.wrapped_event_json.clone()).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "folder key grant wrapped event JSON did not parse",
        )
    })?;
    let payload_json = serde_json::to_string(&folder_key_grant_response(grant.clone()))
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error"))?;
    Ok(SyncRecordInput::Control(ControlSyncRecord {
        record_event_id: event.id.to_hex(),
        record_type: SyncRecordType::FolderKeyGrant,
        folder_id: Some(grant.folder_id.clone()),
        actor_npub: grant.issuer_npub.clone(),
        client_created_at: grant.created_at.clone(),
        payload_json,
        record_event_kind: event.kind.as_u16(),
    }))
}

fn append_admin_access_change_record(
    store: &mut BrainStore,
    brain_id: &BrainId,
    actor_npub: &str,
    event: &Event,
    payload: &AdminAccessChangePayload,
) -> Result<(), ApiError> {
    store.submit_sync_record(
        brain_id,
        &admin_access_change_sync_record(actor_npub, event, payload)?,
    )?;
    Ok(())
}

fn admin_access_change_sync_record(
    actor_npub: &str,
    event: &Event,
    payload: &AdminAccessChangePayload,
) -> Result<SyncRecordInput, ApiError> {
    let folder_id = payload.folder_id.as_ref().map(FolderId::new).transpose()?;
    Ok(SyncRecordInput::Control(ControlSyncRecord {
        record_event_id: event.id.to_hex(),
        record_type: SyncRecordType::BrainAdminAccessChange,
        folder_id,
        actor_npub: UserId::new(actor_npub.to_owned())?,
        client_created_at: payload.created_at.clone(),
        payload_json: event.content.clone(),
        record_event_kind: event.kind.as_u16(),
    }))
}

async fn resolve_user_id_set(
    state: &ServerState,
    values: Vec<String>,
) -> Result<BTreeSet<UserId>, ApiError> {
    let mut resolved = BTreeSet::new();
    for value in values {
        let identity = resolve_and_record_identity(state, &value).await?;
        resolved.insert(UserId::new(identity.npub)?);
    }
    Ok(resolved)
}

fn grant_requests_to_metadata(
    requests: &[FolderKeyGrantRequest],
    folder_id: &FolderId,
    issuer_npub: &str,
    access_change_event_json: Option<String>,
    default_created_at: &str,
) -> Result<Vec<FolderKeyGrantMetadata>, ApiError> {
    requests
        .iter()
        .map(|request| {
            grant_request_to_metadata(
                request,
                folder_id,
                issuer_npub,
                access_change_event_json.clone(),
                default_created_at,
            )
        })
        .collect()
}

fn bootstrap_grant_requests_to_metadata(
    requests: &[CreateBrainFolderKeyGrantRequest],
    issuer_npub: &str,
    default_created_at: &str,
) -> Result<Vec<FolderKeyGrantMetadata>, ApiError> {
    requests
        .iter()
        .map(|request| {
            let folder_id = FolderId::new(request.folder_id.clone())?;
            grant_request_to_metadata(
                &request.grant,
                &folder_id,
                issuer_npub,
                None,
                default_created_at,
            )
        })
        .collect()
}

fn validate_bootstrap_grant_requests(
    requests: &[CreateBrainFolderKeyGrantRequest],
    required: &[RequiredFolderKeyGrant],
) -> Result<(), ApiError> {
    let required_set = required
        .iter()
        .map(|grant| {
            (
                grant.folder_id.to_string(),
                grant.key_version,
                grant.recipient_user_id.to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    let provided_set = requests
        .iter()
        .map(|request| {
            canonical_npub_from_public_key_input(&request.grant.recipient_npub).map(|recipient| {
                (
                    request.folder_id.clone(),
                    request.grant.key_version,
                    recipient,
                )
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if provided_set != required_set || requests.len() != required.len() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bootstrap grants must exactly match required Folder Key Grant recipients",
        ));
    }
    Ok(())
}

fn grant_request_to_metadata(
    request: &FolderKeyGrantRequest,
    folder_id: &FolderId,
    issuer_npub: &str,
    access_change_event_json: Option<String>,
    default_created_at: &str,
) -> Result<FolderKeyGrantMetadata, ApiError> {
    let recipient_npub = UserId::new(canonical_npub_from_public_key_input(
        &request.recipient_npub,
    )?)?;
    validate_folder_key_grant_wrapper(&request.wrapped_event_json, &recipient_npub)?;
    Ok(FolderKeyGrantMetadata {
        id: request.id.clone(),
        folder_id: folder_id.clone(),
        key_version: request.key_version,
        issuer_npub: UserId::new(issuer_npub.to_owned())?,
        recipient_npub,
        format: "NIP-59".to_owned(),
        wrapped_event_json: request.wrapped_event_json.clone(),
        access_change_event_json,
        created_at: request
            .created_at
            .clone()
            .unwrap_or_else(|| default_created_at.to_owned()),
    })
}

fn canonical_npub_from_public_key_input(value: &str) -> Result<String, ApiError> {
    let recipient_public_key = NostrPublicKey::parse(value).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid folder key grant recipient: {error}"),
        )
    })?;
    recipient_public_key.to_npub().map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid folder key grant recipient: {error}"),
        )
    })
}

fn validate_folder_key_grant_wrapper(
    wrapped_event_json: &str,
    recipient_npub: &UserId,
) -> Result<(), ApiError> {
    let event = Event::from_json(wrapped_event_json).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "folder key grant wrapped event JSON did not parse",
        )
    })?;
    let recipient = NostrPublicKey::parse(recipient_npub.as_str()).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid folder key grant recipient: {error}"),
        )
    })?;
    validate_gift_wrap(&event, recipient).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid folder key grant wrapper: {error}"),
        )
    })
}

fn expected_created_at(event: &Event) -> Result<String, ApiError> {
    format_unix_timestamp(event.created_at.as_secs()).ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "signed event created_at is outside RFC3339 timestamp range",
        )
    })
}

fn ensure_brain_admin(stored: &StoredBrain, actor_npub: &str) -> Result<(), ApiError> {
    if stored.brain.kind == BrainKind::Personal
        && stored
            .brain
            .owner_user_id
            .as_ref()
            .is_some_and(|owner| owner.as_str() == actor_npub)
    {
        return Ok(());
    }
    if stored.brain.kind == BrainKind::Personal
        && stored
            .personal_agent
            .as_ref()
            .is_some_and(|relationship| relationship.agent_npub.as_str() == actor_npub)
    {
        return Ok(());
    }
    let is_admin = stored
        .brain
        .admins
        .iter()
        .any(|admin| admin.as_str() == actor_npub);
    if is_admin {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "brain admin access required",
        ))
    }
}

fn ensure_direct_delete_authority(stored: &StoredBrain, actor_npub: &str) -> Result<(), ApiError> {
    ensure_brain_admin(stored, actor_npub).map_err(|_| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "permanent deletion requires brain destructive authority",
        )
    })
}

fn folder_current_key_version(stored: &StoredBrain, folder_id: &FolderId) -> Result<u32, ApiError> {
    stored
        .brain
        .folders
        .iter()
        .find(|folder| folder.id == *folder_id)
        .map(|folder| folder.current_key_version)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "folder not found"))
}

fn ensure_folder_key_version(
    stored: &StoredBrain,
    folder_id: &FolderId,
    key_version: u32,
) -> Result<(), ApiError> {
    let folder = stored
        .brain
        .folders
        .iter()
        .find(|folder| folder.id == *folder_id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "folder not found"))?;
    if folder.current_key_version == key_version {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "keyVersion does not match current Folder Key version",
        ))
    }
}

fn ensure_folder_visible(
    stored: &StoredBrain,
    folder_id: &FolderId,
    actor_npub: &str,
) -> Result<(), ApiError> {
    if folder_visible(stored, folder_id, actor_npub) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "folder access required",
        ))
    }
}

fn folder_visible(stored: &StoredBrain, folder_id: &FolderId, actor_npub: &str) -> bool {
    let Some(folder) = stored
        .brain
        .folders
        .iter()
        .find(|folder| folder.id == *folder_id)
    else {
        return false;
    };
    let is_owner = stored
        .brain
        .owner_user_id
        .as_ref()
        .is_some_and(|owner| owner.as_str() == actor_npub);
    let is_admin = stored
        .brain
        .admins
        .iter()
        .any(|admin| admin.as_str() == actor_npub);
    let is_personal_agent = stored
        .personal_agent
        .as_ref()
        .is_some_and(|relationship| relationship.agent_npub.as_str() == actor_npub);
    let is_member = stored
        .brain
        .members
        .iter()
        .any(|member| member.user_id.as_str() == actor_npub);

    if is_personal_agent {
        return true;
    }

    match folder.access {
        FolderAccessMode::Owner => is_owner,
        FolderAccessMode::AdminOnly => is_owner || is_admin,
        FolderAccessMode::AllMembers => {
            is_owner || is_admin || (stored.brain.kind == BrainKind::Organization && is_member)
        }
        FolderAccessMode::Restricted => {
            is_owner
                || is_admin
                || stored
                    .folder_access
                    .get(folder_id)
                    .is_some_and(|users| users.iter().any(|user| user.as_str() == actor_npub))
        }
    }
}

fn record_visible(stored: &StoredBrain, record: &StoredSyncRecord, actor_npub: &str) -> bool {
    let is_owner = stored
        .brain
        .owner_user_id
        .as_ref()
        .is_some_and(|owner| owner.as_str() == actor_npub);
    let is_admin = stored
        .brain
        .admins
        .iter()
        .any(|admin| admin.as_str() == actor_npub);
    let is_personal_agent = stored
        .personal_agent
        .as_ref()
        .is_some_and(|relationship| relationship.agent_npub.as_str() == actor_npub);
    match record.record_type {
        SyncRecordType::FolderObjectRevision | SyncRecordType::FolderObjectTombstone => record
            .folder_id
            .as_ref()
            .is_some_and(|folder_id| folder_visible(stored, folder_id, actor_npub)),
        SyncRecordType::FolderKeyGrant => {
            is_admin || grant_payload_recipient(&record.payload_json).as_deref() == Some(actor_npub)
        }
        SyncRecordType::BrainAdminAccessChange => {
            is_owner
                || is_admin
                || is_personal_agent
                || (is_folder_subtree_tombstone(&record.payload_json)
                    && stored
                        .folder_deletion_audience
                        .get(&record.record_event_id)
                        .is_some_and(|audience| {
                            audience.iter().any(|reader| reader.as_str() == actor_npub)
                        }))
        }
    }
}

fn is_folder_subtree_tombstone(payload_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(payload_json)
        .ok()
        .and_then(|payload| payload.get("recordType")?.as_str().map(ToOwned::to_owned))
        .as_deref()
        == Some("folder_subtree_tombstone")
}

fn grant_payload_recipient(payload_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(payload_json)
        .ok()?
        .get("recipientNpub")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn object_ciphertext(payload_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload_json)
        .ok()
        .and_then(|value| {
            value
                .get("ciphertext")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| payload_json.to_owned())
}

fn request_field(body: &[u8], field: &'static str) -> Result<String, ApiError> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get(field)
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, format!("{field} is required")))
}

fn lock_error<T>(_error: T) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "store lock poisoned")
}

fn grants_for_required(
    required: &[RequiredFolderKeyGrant],
    brain_id: &BrainId,
    issuer_npub: &str,
) -> Vec<FolderKeyGrantMetadata> {
    required
        .iter()
        .map(|required| FolderKeyGrantMetadata {
            id: format!(
                "bootstrap-{}-{}-{}-{}",
                brain_id, required.folder_id, required.key_version, required.recipient_user_id
            ),
            folder_id: required.folder_id.clone(),
            key_version: required.key_version,
            issuer_npub: UserId::new(issuer_npub).expect("issuer npub was already validated"),
            recipient_npub: required.recipient_user_id.clone(),
            format: "NIP-59".to_owned(),
            wrapped_event_json: "{\"kind\":1059}".to_owned(),
            access_change_event_json: None,
            created_at: "2026-06-23T00:00:00.000Z".to_owned(),
        })
        .collect()
}

fn shared_folder_connection_id(
    source_brain_id: &BrainId,
    source_folder_id: &FolderId,
    destination_brain_id: &BrainId,
) -> String {
    generated_link_id(
        "shared-folder-connection",
        &[
            source_brain_id.as_str(),
            source_folder_id.as_str(),
            destination_brain_id.as_str(),
        ],
        8,
    )
}

fn organization_mount_id(
    organization_brain_id: &BrainId,
    source_brain_id: &BrainId,
    source_folder_id: &FolderId,
) -> String {
    generated_link_id(
        "organization-mount",
        &[
            organization_brain_id.as_str(),
            source_brain_id.as_str(),
            source_folder_id.as_str(),
        ],
        8,
    )
}

fn ensure_metadata_visible(stored: &StoredBrain, actor_npub: &str) -> Result<(), ApiError> {
    match stored.brain.kind {
        BrainKind::Personal => {
            let is_owner = stored
                .brain
                .owner_user_id
                .as_ref()
                .is_some_and(|owner| owner.as_str() == actor_npub);
            let is_limited_member = stored
                .brain
                .members
                .iter()
                .any(|member| member.user_id.as_str() == actor_npub)
                && stored
                    .brain
                    .folders
                    .iter()
                    .any(|folder| folder_visible(stored, &folder.id, actor_npub));
            let is_personal_agent = stored
                .personal_agent
                .as_ref()
                .is_some_and(|relationship| relationship.agent_npub.as_str() == actor_npub);
            let is_deletion_recipient = stored
                .folder_deletion_audience
                .values()
                .any(|audience| audience.iter().any(|reader| reader.as_str() == actor_npub));
            if is_owner || is_personal_agent || is_limited_member || is_deletion_recipient {
                Ok(())
            } else {
                Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "brain access required",
                ))
            }
        }
        BrainKind::Organization => {
            let is_member = stored
                .brain
                .members
                .iter()
                .any(|member| member.user_id.as_str() == actor_npub);
            if is_member {
                Ok(())
            } else {
                Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "brain access required",
                ))
            }
        }
    }
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn server_timestamp(state: &ServerState) -> String {
    format_unix_timestamp(state.auth_now_unix_seconds())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned())
}

fn format_unix_timestamp(unix_seconds: u64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(unix_seconds as i64)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

fn generated_link_id(prefix: &str, parts: &[&str], hash_bytes: usize) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\n");
    }
    let hash = hasher.finalize();
    format!("{prefix}-{}", hex_prefix(&hash, hash_bytes))
}

fn hex_prefix(bytes: &[u8], len: usize) -> String {
    bytes
        .iter()
        .take(len)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use axum::http::header::{
        ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN, AUTHORIZATION, CACHE_CONTROL,
        CONTENT_TYPE, ORIGIN,
    };
    use finite_brain_core::{
        EncryptedFolderObjectEnvelope, FolderKey, FolderObjectAad,
        MAX_FOLDER_ACCESS_REMOVAL_GRANTS, MAX_PERSONAL_AGENT_ROTATION_FOLDERS,
        encrypt_folder_object_with_nonce, open_folder_object,
    };
    use finite_nostr::{
        GiftWrapValidation, HttpAuthEventRequest, build_rumor, encode_http_auth_header,
        open_gift_wrap, sign_http_auth_event, wrap_rumor,
    };
    use nostr::event::FinalizeEvent;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use tower::ServiceExt;

    const TEST_NOW: u64 = 1_780_000_000;
    const TEST_BASE_URL: &str = "http://finite.test";

    #[test]
    fn health_status_identifies_workspace_layers() {
        assert_eq!(
            health_status(),
            HealthStatus {
                service: "finite-brain".to_owned(),
                status: "ok".to_owned(),
                core_crate: "finite-brain-core".to_owned(),
                store_crate: "finite-brain-store".to_owned(),
            }
        );
    }

    #[test]
    fn server_state_defaults_to_portable_v1_auth_skew() {
        let state = ServerState::new(BrainStore::open_in_memory().unwrap(), TEST_BASE_URL);
        assert_eq!(state.max_auth_skew_seconds, 60);
        assert_eq!(state.auth_now_unix_seconds, None);
        assert_eq!(state.rate_limit.max_requests, 120);
        assert_eq!(state.rate_limit.window_seconds, 60);
        assert!(state.cors_origin_allowed(TEST_BASE_URL));

        let path_state = ServerState::new(
            BrainStore::open_in_memory().unwrap(),
            "https://finite.example/smoke",
        );
        assert!(path_state.cors_origin_allowed("https://finite.example"));
    }

    #[tokio::test]
    async fn health_route_returns_workspace_status_without_auth() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("health route response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("health body");
        let status: HealthStatus = serde_json::from_slice(&body).expect("health json");

        assert_eq!(status, health_status());
    }

    #[tokio::test]
    async fn smoke_bootstrap_route_returns_core_summary() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/smoke/bootstrap")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("bootstrap route response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), 4096)
            .await
            .expect("bootstrap body");
        let summary: BootstrapSmokeSummary = serde_json::from_slice(&body).expect("bootstrap json");

        assert_eq!(
            summary,
            finite_brain_core::smoke_bootstrap_summary().expect("smoke bootstrap summary")
        );
    }

    #[tokio::test]
    async fn smoke_ui_serves_static_assets_and_sqlite_flow_works() {
        let temp_dir = tempfile::TempDir::new().expect("temp sqlite dir");
        let db_path = temp_dir.path().join("smoke-ui.sqlite3");
        let router = sqlite_test_router(&db_path);

        let ui_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/smoke/ui")
                    .body(Body::empty())
                    .expect("valid ui request"),
            )
            .await
            .expect("ui response");
        assert_eq!(ui_response.status(), StatusCode::OK);
        let ui_body = to_bytes(ui_response.into_body(), 16 * 1024)
            .await
            .expect("ui body");
        let ui_body = std::str::from_utf8(&ui_body).expect("ui utf8");
        assert!(ui_body.contains("Development only"));
        assert!(ui_body.contains("FiniteBrain Smoke UI"));
        assert!(ui_body.contains("Invitations and Share Links"));
        assert!(ui_body.contains("Connections and mounts"));
        assert!(ui_body.contains("href=\"/client\""));
        assert!(ui_body.contains("Open client"));

        let css_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/smoke/ui.css")
                    .body(Body::empty())
                    .expect("valid css request"),
            )
            .await
            .expect("css response");
        assert_eq!(css_response.status(), StatusCode::OK);
        let css_body = to_bytes(css_response.into_body(), 16 * 1024)
            .await
            .expect("css body");
        let css_body = std::str::from_utf8(&css_body).expect("css utf8");
        assert!(css_body.contains(".topbar"));

        let js_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/smoke/ui.js")
                    .body(Body::empty())
                    .expect("valid js request"),
            )
            .await
            .expect("js response");
        assert_eq!(js_response.status(), StatusCode::OK);
        let js_body = to_bytes(js_response.into_body(), 256 * 1024)
            .await
            .expect("js body");
        let js_body = std::str::from_utf8(&js_body).expect("js utf8");
        assert!(js_body.contains("bootstrapButton"));
        assert!(js_body.contains("createShareLinkButton"));
        assert!(js_body.contains("mountsButton"));

        let keys = Keys::generate();
        let create = post_brain(
            router.clone(),
            &keys,
            &create_brain_body("smoke", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create.status(), StatusCode::OK);

        let reopened = sqlite_test_router(&db_path);
        let metadata = get_metadata(reopened.clone(), &keys, "smoke", TEST_NOW).await;
        assert_eq!(metadata.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(metadata).await;
        assert_eq!(metadata.brain_id, "smoke");
        assert!(metadata.folders.is_empty());

        let sync_bootstrap = authed_request(
            reopened,
            &keys,
            "GET",
            "/_admin/brains/smoke/sync/bootstrap",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(sync_bootstrap.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn product_client_serves_spine_assets_and_config() {
        let router = test_router();

        let client_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/client")
                    .body(Body::empty())
                    .expect("valid client request"),
            )
            .await
            .expect("client response");
        assert_eq!(client_response.status(), StatusCode::OK);
        assert_eq!(
            client_response.headers().get(CACHE_CONTROL).unwrap(),
            "no-store, max-age=0"
        );
        let client_body = to_bytes(client_response.into_body(), 64 * 1024)
            .await
            .expect("client body");
        let client_body = std::str::from_utf8(&client_body).expect("client utf8");
        assert!(client_body.contains("obsidian-shell"));
        assert!(!client_body.contains("obsidian-titlebar"));
        assert!(!client_body.contains("traffic-lights"));
        assert!(!client_body.contains("titlebarTabLabel"));
        assert!(!client_body.contains("titlebarBrainLabel"));
        assert!(!client_body.contains("pageTabButton"));
        assert!(!client_body.contains("graphTabButton"));
        assert!(!client_body.contains("titlebarNewTabButton"));
        assert!(client_body.contains("sidebar-primary-nav"));
        assert!(!client_body.contains("app-ribbon"));
        assert!(client_body.contains("file-sidebar"));
        assert!(client_body.contains("Connect securely"));
        assert!(client_body.contains("Brain locked"));
        assert!(!client_body.contains("Connect signer"));
        assert!(!client_body.contains("Connect account"));
        assert!(!client_body.contains("Session locked"));
        assert!(client_body.contains("resumeSessionButton"));
        assert!(client_body.contains("lockSessionButton"));
        assert!(!client_body.contains("Open accessible brain"));
        assert!(!client_body.contains("brainControlDetails"));
        assert!(!client_body.contains("brainSelect"));
        assert!(client_body.contains("sessionAccountBrainButton"));
        assert!(client_body.contains("brainSwitcherMenu"));
        assert!(client_body.contains("manageBrainsModal"));
        assert!(client_body.contains("settingsManageBrainsButton"));
        assert!(client_body.contains("readerFolderList"));
        assert!(client_body.contains("searchSidebarPanel"));
        assert!(client_body.contains("commandPalette"));
        assert!(client_body.contains("Quick switcher"));
        assert!(client_body.contains("graph-floating-controls"));
        assert!(client_body.contains("ribbonGraphButton"));
        assert!(!client_body.contains("editorToolbar"));
        assert!(!client_body.contains("inline-editor-toolbar"));
        assert!(!client_body.contains("data-editor-command"));
        assert!(client_body.contains("readerPageContent"));
        assert!(client_body.contains("aria-label=\"Page reader\""));
        assert!(client_body.contains("aria-label=\"Graph View\""));
        assert!(client_body.contains("aria-label=\"Search pages\""));
        assert!(!client_body.contains("graphFilterInput"));
        assert!(!client_body.contains("aria-label=\"Filter graph\""));
        assert!(client_body.contains("accessFolderButton"));
        assert!(client_body.contains("accessInspector"));
        assert!(client_body.contains("accessWhoHasList"));
        assert!(client_body.contains("accessAdvancedSection"));
        assert!(!client_body.contains("accessChangeMode"));
        assert!(!client_body.contains("accessBrainViewButton"));
        assert!(!client_body.contains("accessFolderViewButton"));
        assert!(!client_body.contains("accessBrainPanel"));
        assert!(!client_body.contains("brainSwitchList"));
        assert!(!client_body.contains("removeFolderAccessButton"));
        assert!(!client_body.contains("folderKeyInput"));
        assert!(!client_body.contains("okfBundleInput"));
        assert!(!client_body.contains("encryptDraftButton"));
        assert!(client_body.contains("createBrainInvitationButton"));
        assert!(client_body.contains("acceptBrainInvitationButton"));
        assert!(client_body.contains("revokeBrainInvitationButton"));
        assert!(client_body.contains("brainInviteUrlOutput"));
        assert!(client_body.contains("copyBrainInviteUrlButton"));
        assert!(client_body.contains("Copy private invite link"));
        assert!(client_body.contains("savePageButton"));
        assert!(!client_body.contains("readerModeButton"));
        assert!(client_body.contains("Edit Markdown"));
        assert!(!client_body.contains("syncBootstrapButton"));
        assert!(client_body.contains("Graph View"));
        assert!(client_body.contains("Zoom in"));
        assert!(client_body.contains("Reset zoom"));
        assert!(client_body.contains("Enter full screen"));
        assert!(client_body.contains("contextMenu"));
        assert!(client_body.contains("/client/app.js"));
        assert!(!client_body.contains("__FINITE_BRAIN_DISABLE_AUTOSTART__"));
        assert!(!client_body.contains("/client/smoke-nip07.js"));
        assert!(!client_body.contains("Page Loop"));
        assert!(!client_body.contains("OKF Import"));
        assert!(!client_body.contains("Plan OKF import"));

        let config_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/client/config.json")
                    .body(Body::empty())
                    .expect("valid config request"),
            )
            .await
            .expect("config response");
        assert_eq!(config_response.status(), StatusCode::OK);
        assert_eq!(
            config_response.headers().get(CACHE_CONTROL).unwrap(),
            "no-store, max-age=0"
        );
        let config: ProductClientConfigResponse = read_json(config_response).await;
        assert_eq!(
            config,
            ProductClientConfigResponse {
                public_base_url: TEST_BASE_URL.to_owned(),
                auth_scheme: "Nostr".to_owned(),
                http_auth_kind: 27_235,
                default_brain_id: "personal".to_owned(),
            }
        );

        let css_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/client/app.css")
                    .body(Body::empty())
                    .expect("valid client css request"),
            )
            .await
            .expect("client css response");
        assert_eq!(css_response.status(), StatusCode::OK);
        assert_eq!(
            css_response.headers().get(CACHE_CONTROL).unwrap(),
            "no-store, max-age=0"
        );
        let css_body = to_bytes(css_response.into_body(), 128 * 1024)
            .await
            .expect("client css body");
        let css_body = std::str::from_utf8(&css_body).expect("client css utf8");
        assert!(css_body.contains("font-family: \"Funnel Sans\""));
        assert!(css_body.contains("font-family: \"Funnel Display\""));
        assert!(css_body.contains("font-family: \"JetBrains Mono\""));
        assert!(css_body.contains("/client/fonts/funnel-sans-400.ttf"));
        assert!(css_body.contains("/client/fonts/funnel-display-600.ttf"));
        assert!(css_body.contains("/client/fonts/jetbrains-mono-400.ttf"));
        assert!(css_body.contains("@media (prefers-color-scheme: light)"));
        assert!(css_body.contains("--font-sans:"));
        assert!(css_body.contains("--font-display:"));
        assert!(css_body.contains("--font-mono:"));
        assert!(css_body.contains("--status-success:"));
        assert!(css_body.contains("--status-warning:"));
        assert!(css_body.contains("--status-error:"));
        assert!(css_body.contains(".obsidian-shell"));
        assert!(!css_body.contains(".obsidian-titlebar"));
        assert!(!css_body.contains(".traffic-light"));
        assert!(!css_body.contains(".titlebar-tab"));
        assert!(css_body.contains(".sidebar-primary-nav"));
        assert!(!css_body.contains(".app-ribbon"));
        assert!(css_body.contains(".brain-picker"));
        assert!(css_body.contains(".brain-create-row"));
        assert!(css_body.contains(".folder-option-button"));
        assert!(css_body.contains(".obsidian-folder-button"));
        assert!(css_body.contains(".context-menu"));
        assert!(css_body.contains(".graph-stage"));
        assert!(css_body.contains(".graph-floating-controls"));
        assert!(!css_body.contains(".graph-icon-button"));
        assert!(!css_body.contains(".graph-controls"));
        assert!(css_body.contains(".graph-canvas.is-hovering"));
        assert!(css_body.contains(".node.hover-active"));
        assert!(css_body.contains(".edge.hover-connected"));
        assert!(css_body.contains(".access-inspector"));
        assert!(css_body.contains(".access-badge"));
        assert!(css_body.contains(".access-content-panel"));
        assert!(css_body.contains(".brain-invite-url-output"));
        assert!(!css_body.contains(".access-view-switch"));
        assert!(!css_body.contains(".okf-controls"));
        assert!(css_body.contains(".session-security-status"));

        let js_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/client/app.js")
                    .body(Body::empty())
                    .expect("valid client js request"),
            )
            .await
            .expect("client js response");
        assert_eq!(js_response.status(), StatusCode::OK);
        assert_eq!(
            js_response.headers().get(CACHE_CONTROL).unwrap(),
            "no-store, max-age=0"
        );
        let js_body = to_bytes(js_response.into_body(), 512 * 1024)
            .await
            .expect("client js body");
        let js_body = std::str::from_utf8(&js_body).expect("client js utf8");
        assert!(js_body.contains("window.FiniteBrainProductClient"));
        assert!(js_body.contains("deriveSignerState"));
        assert!(js_body.contains("parseOkfBundle"));
        assert!(js_body.contains("prepareOkfImportWrites"));
        assert!(js_body.contains("buildAuthEventTemplate"));
        assert!(js_body.contains("buildPageWriteRequest"));
        assert!(js_body.contains("workspaceChromeState"));
        assert!(js_body.contains("visibleBrainOptions"));
        assert!(js_body.contains("personalBrainIdForPubkey"));
        assert!(js_body.contains("accessBadgesForFolder"));
        assert!(js_body.contains("accessActionRoute"));
        assert!(js_body.contains("openManageBrainsModal"));
        assert!(js_body.contains("removeFolderAccessFromPanel"));
        assert!(!js_body.contains("removeFolderAccessButton"));
        assert!(js_body.contains("readerFolderRows"));
        assert!(js_body.contains("readerPageRows"));
        assert!(js_body.contains("buildGraphProjection"));
        assert!(js_body.contains("graphLayout"));
        assert!(js_body.contains("graphStats"));
        assert!(js_body.contains("graphNeighborIds"));
        assert!(js_body.contains("setGraphHover"));
        assert!(js_body.contains("createSessionKeyring"));
        assert!(js_body.contains("clearSessionSecretsAndPlaintext"));
        assert!(js_body.contains("copyToClipboard"));
        assert!(js_body.contains("copyBrainInviteUrl"));
        assert!(js_body.contains("sessionStatusView"));
        assert!(js_body.contains("sessionGrantOpeningAllowed"));
        assert!(js_body.contains("extractPageLinks"));
        assert!(js_body.contains("openFolderObject"));
        assert!(js_body.contains("mergeSyncProjection"));
        assert!(js_body.contains("metadataFolderRows"));
        assert!(js_body.contains("kind: 27235"));
        assert!(js_body.contains("kind: APP_EVENT_KIND"));
        assert!(js_body.contains("/metadata"));

        let smoke_signer_response = router
            .oneshot(
                Request::builder()
                    .uri("/client/smoke-nip07.js")
                    .body(Body::empty())
                    .expect("valid smoke signer request"),
            )
            .await
            .expect("smoke signer response");
        assert_eq!(smoke_signer_response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn product_client_serves_local_dashboard_fonts() {
        let router = test_router();
        let fonts = [
            (
                "/client/fonts/funnel-display-500.ttf",
                32_880,
                "d820e428132e2622a7d175a74a826748bff68d113e7aec79b6f3545e86ff20f2",
            ),
            (
                "/client/fonts/funnel-display-600.ttf",
                32_864,
                "e37cbfefbb7a762fe2b69e43e12c7e840d81452d1fdc6fc3ecf0b0ec7605b3af",
            ),
            (
                "/client/fonts/funnel-display-700.ttf",
                32_812,
                "c61b735d94ac0bcd32904da436e3003f99804d09ee81ea3bea6690b180ea7a1b",
            ),
            (
                "/client/fonts/funnel-sans-400.ttf",
                32_988,
                "d9cd65b22ca457dee2310777973cb3b77e55d28866cc574018a77cd593d5d0d6",
            ),
            (
                "/client/fonts/funnel-sans-500.ttf",
                32_964,
                "ed6bdb3b1d1fbe7bf38f702e64c6f99ab8b324a30bee2a4fca591da57505289c",
            ),
            (
                "/client/fonts/funnel-sans-600.ttf",
                33_004,
                "f23f08c47901e39db4c1ae4f212c88f43ed0b6037d1252f9d589807ff6a023b5",
            ),
            (
                "/client/fonts/funnel-sans-700.ttf",
                32_892,
                "56a1277e3f904bd9543e533e1e6656c88f2e46738e1c6d1da438709323e7e87e",
            ),
            (
                "/client/fonts/jetbrains-mono-400.ttf",
                112_172,
                "44ce4a84f20d60f24539bd0cef11f79c29e38609e0f8adf18551c9794a5d9dc3",
            ),
            (
                "/client/fonts/jetbrains-mono-500.ttf",
                112_204,
                "3386a05f6ece969e4537de6be894170d20558e82f7d56c8c5d332972ef172160",
            ),
            (
                "/client/fonts/jetbrains-mono-600.ttf",
                112_160,
                "df54dbfafba61d4911eb3dab9bba2d20531fb009f01d64dd42fa96ab862584d8",
            ),
        ];

        for (path, expected_len, expected_sha256) in fonts {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("valid font request"),
                )
                .await
                .expect("font response");
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                response.headers().get(CONTENT_TYPE).unwrap(),
                "font/ttf",
                "{path}"
            );
            assert_eq!(
                response.headers().get(CACHE_CONTROL).unwrap(),
                "no-store, max-age=0",
                "{path}"
            );
            let body = to_bytes(response.into_body(), 128 * 1024)
                .await
                .expect("font body");
            assert_eq!(body.len(), expected_len, "{path}");
            assert_eq!(
                format!("{:x}", Sha256::digest(&body)),
                expected_sha256,
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn product_client_smoke_nip07_signer_is_explicitly_opt_in() {
        let router = router_with_state(
            test_state()
                .with_smoke_nip07_signer(
                    "0000000000000000000000000000000000000000000000000000000000000001",
                )
                .expect("valid smoke signer secret"),
        );

        let client_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/client")
                    .body(Body::empty())
                    .expect("valid client request"),
            )
            .await
            .expect("client response");
        assert_eq!(client_response.status(), StatusCode::OK);
        let client_body = read_text_with_limit(client_response, 64 * 1024).await;
        assert!(client_body.contains("__FINITE_BRAIN_DISABLE_AUTOSTART__"));
        assert!(client_body.contains("/client/smoke-nip07.js"));
        assert!(client_body.contains("/client/app.js"));

        let smoke_signer_response = router
            .oneshot(
                Request::builder()
                    .uri("/client/smoke-nip07.js")
                    .body(Body::empty())
                    .expect("valid smoke signer request"),
            )
            .await
            .expect("smoke signer response");
        assert_eq!(smoke_signer_response.status(), StatusCode::OK);
        let smoke_signer_body = read_text_with_limit(smoke_signer_response, 32 * 1024).await;
        assert!(smoke_signer_body.contains("createLocalNip07ProviderFromSecret"));
        assert!(smoke_signer_body.contains("__FINITE_BRAIN_SMOKE_NIP07__"));
        assert!(smoke_signer_body.contains("__FINITE_BRAIN_SET_SMOKE_NIP07_SECRET__"));
        assert!(smoke_signer_body.contains("smokeNip07Secret"));
        assert!(!smoke_signer_body.contains("sessionStorage"));
        assert!(smoke_signer_body.contains("typeof window.history?.replaceState !== \"function\""));
        assert!(smoke_signer_body.contains("window.history.replaceState"));
        assert!(!smoke_signer_body.contains("history?.replaceState?."));
        assert!(
            smoke_signer_body
                .contains("0000000000000000000000000000000000000000000000000000000000000001")
        );
    }

    #[tokio::test]
    async fn smoke_email_proof_verifier_is_explicit_and_allowlisted() {
        let actor = UserId::new(npub(&Keys::generate())).expect("valid actor npub");

        let unconfigured =
            verify_identity_authority_email_proof(&test_state(), "friend@example.com", &actor)
                .await
                .expect_err("default verifier should be absent");
        assert_eq!(unconfigured.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(unconfigured.message.contains("not configured"));

        let state = test_state()
            .with_smoke_email_proofs(" Friend@Example.com , teammate@example.com ")
            .expect("valid smoke email allowlist");
        verify_identity_authority_email_proof(&state, "friend@example.com", &actor)
            .await
            .expect("allowlisted smoke email");
        verify_identity_authority_email_proof(&state, "TEAMMATE@example.com", &actor)
            .await
            .expect("allowlisted smoke email normalizes case");

        let denied = verify_identity_authority_email_proof(&state, "other@example.com", &actor)
            .await
            .expect_err("non-allowlisted smoke email should fail");
        assert_eq!(denied.status, StatusCode::BAD_REQUEST);
        assert!(denied.message.contains("smoke email proof is not allowed"));

        assert!(test_state().with_smoke_email_proofs(" ").is_err());
        assert!(
            test_state()
                .with_smoke_email_proofs("not-an-email")
                .is_err()
        );
    }

    #[test]
    fn email_proof_window_allows_small_future_clock_skew() {
        let admin = UserId::new(npub(&Keys::generate())).expect("valid admin npub");
        let invitation = StoredBrainInvitation {
            id: "invitation-test".to_owned(),
            brain_id: BrainId::new("acme").expect("valid brain id"),
            target_kind: BrainInvitationTargetKind::EmailBootstrap,
            user_id: None,
            invited_email: Some("friend@example.com".to_owned()),
            invite_unwrap_npub: None,
            bootstrap_payload_hash: None,
            bootstrap_wrapped_event_json: None,
            bootstrap_authorization_event_json: None,
            bootstrap_scope: Vec::new(),
            claimed_by_npub: None,
            status: LinkStatus::Pending,
            invite_code: "invite-test".to_owned(),
            accept_path: "/_admin/brain-invitation-links/invite-test/claim".to_owned(),
            initial_folder_access: Vec::new(),
            created_by_npub: admin,
            expires_at: "2026-07-08T12:00:00Z".to_owned(),
            created_at: "2026-07-07T12:00:00Z".to_owned(),
            updated_at: "2026-07-07T12:00:00Z".to_owned(),
            accepted_at: None,
            duplicate_accept: false,
        };

        validate_email_proof_window(&invitation, "2026-07-07T12:00:30Z", "2026-07-07T12:00:00Z")
            .expect("small future skew should be accepted");
        let too_far_future = validate_email_proof_window(
            &invitation,
            "2026-07-07T12:02:00Z",
            "2026-07-07T12:00:00Z",
        )
        .expect_err("future skew beyond tolerance should fail");
        assert_eq!(too_far_future.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn valid_auth_creates_empty_organization_brain() {
        let keys = Keys::generate();
        let body = create_brain_body("acme", "organization");
        let router = test_router();
        let response = post_brain(router.clone(), &keys, &body, TEST_NOW, None, None, None).await;

        assert_eq!(response.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(response).await;
        assert_eq!(metadata.brain_id, "acme");
        assert_eq!(metadata.kind, BrainKind::Organization);
        assert!(metadata.folders.is_empty());
        assert_eq!(metadata.grant_count, 0);

        let response = get_metadata(router, &keys, "acme", TEST_NOW).await;
        assert_eq!(response.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(response).await;
        assert_eq!(metadata.brain_id, "acme");
        assert_eq!(metadata.members.len(), 1);
    }

    #[tokio::test]
    async fn user_created_organization_brain_atomically_adds_selected_agent_admin_by_email() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_npub = npub(&owner_keys);
        let agent_npub = npub(&agent_keys);
        let agent_key = NostrPublicKey::from_protocol(agent_keys.public_key());
        let identifier = Nip05Identifier::parse("cheater@finite.vip").unwrap();
        let document =
            serde_json::json!({ "names": { "cheater": agent_key.to_hex() } }).to_string();
        let (identity_url, identity_server) = spawn_json_authority(vec![
            (
                "/api/v1/operator/brain/agent-resolution",
                serde_json::json!({
                    "agentNpub": agent_npub,
                    "managedAgentEmail": "cheater@finite.vip",
                }),
            ),
            (
                "/api/v1/operator/brain/user-resolution",
                serde_json::json!({
                    "workosUserId": "user_workos_owner",
                    "userNpub": owner_npub,
                }),
            ),
        ]);
        let (core_url, core_server) = spawn_json_authority(vec![(
            "/api/core/v1/brain/agent-account",
            serde_json::json!({
                "workosUserId": "user_workos_owner",
                "managedAgentEmail": "cheater@finite.vip",
                "verifiedEmail": "owner@finite.computer",
                "status": "active",
            }),
        )]);
        let state = test_state()
            .with_nip05_fixture(identifier.well_known_request().url, document)
            .with_agent_bootstrap_authorities(
                core_url,
                "core-token",
                identity_url,
                "identity-token",
            );
        let router = router_with_state(state);
        let body = serde_json::json!({
            "brainId": "acme",
            "kind": "organization",
            "name": "Acme Brain",
            "initialAgentEmail": "cheater@finite.vip",
        })
        .to_string();

        let created = post_brain(
            router.clone(),
            &owner_keys,
            &body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(created.status(), StatusCode::OK);
        let created: BrainMetadataResponse = read_json(created).await;
        assert!(created.folders.is_empty());
        assert_eq!(created.members.len(), 2);
        assert_eq!(created.admins.len(), 2);
        assert!(created.admins.contains(&owner_npub));
        assert!(created.admins.contains(&agent_npub));
        assert!(created.identities.iter().any(|identity| {
            identity.npub == agent_npub && identity.nip05.as_deref() == Some("cheater@finite.vip")
        }));

        let agent_brains = authed_request(
            router,
            &agent_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 1,
        )
        .await;
        let agent_brains: VisibleBrainsResponse = read_json(agent_brains).await;
        assert_eq!(agent_brains.brains.len(), 1);
        assert_eq!(agent_brains.brains[0].role, "admin");
        identity_server.join().unwrap();
        core_server.join().unwrap();
    }

    #[tokio::test]
    async fn agent_created_organization_brain_includes_authenticated_requester() {
        let agent_keys = Keys::generate();
        let requester_keys = Keys::generate();
        let agent_npub = npub(&agent_keys);
        let requester_npub = npub(&requester_keys);
        let body = serde_json::json!({
            "brainId": "acme",
            "kind": "organization",
            "name": "Acme",
            "requestingUserNpub": requester_npub,
        })
        .to_string();
        let router = test_router();

        let response = post_brain(
            router.clone(),
            &agent_keys,
            &body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(response).await;
        assert_eq!(metadata.members.len(), 2);
        assert_eq!(metadata.admins.len(), 2);
        assert!(metadata.folders.is_empty());
        assert_eq!(metadata.grant_count, 0);
        assert!(metadata.members.contains(&agent_npub));
        assert!(metadata.members.contains(&requester_npub));
        assert!(metadata.admins.contains(&agent_npub));
        assert!(metadata.admins.contains(&requester_npub));

        let retry = post_brain(
            router.clone(),
            &agent_keys,
            &body,
            TEST_NOW + 1,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(retry.status(), StatusCode::OK);
        let retried: BrainMetadataResponse = read_json(retry).await;
        assert_eq!(retried.brain_id, metadata.brain_id);
        assert_eq!(retried.members, metadata.members);
        assert_eq!(retried.admins, metadata.admins);

        let requester_metadata =
            get_metadata(router.clone(), &requester_keys, "acme", TEST_NOW + 2).await;
        assert_eq!(requester_metadata.status(), StatusCode::OK);
        let requester_brains = authed_request(
            router.clone(),
            &requester_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 3,
        )
        .await;
        assert_eq!(requester_brains.status(), StatusCode::OK);
        let requester_brains: VisibleBrainsResponse = read_json(requester_brains).await;
        assert_eq!(requester_brains.brains.len(), 1);
        assert_eq!(requester_brains.brains[0].role, "admin");

        let requester_export = authed_request(
            router,
            &requester_keys,
            "GET",
            "/_admin/brains/acme/export",
            None,
            TEST_NOW + 3,
        )
        .await;
        assert_eq!(requester_export.status(), StatusCode::OK);
        let requester_export: EncryptedBrainExportResponse = read_json(requester_export).await;
        assert!(requester_export.key_grants.is_empty());
    }

    #[tokio::test]
    async fn requester_bootstrap_rejects_unexpected_grants_without_creating_a_brain() {
        let agent_keys = Keys::generate();
        let requester_keys = Keys::generate();
        let agent_npub = npub(&agent_keys);
        let requester_npub = npub(&requester_keys);
        let mut bootstrap_grants = ["getting-started", "restricted"]
            .into_iter()
            .flat_map(|folder_id| {
                [agent_npub.as_str(), requester_npub.as_str()].map(|recipient| {
                    serde_json::json!({
                        "folderId": folder_id,
                        "grant": real_folder_key_grant_value(
                            &format!("grant-{folder_id}-{recipient}"),
                            1,
                            &agent_keys,
                            "acme",
                            folder_id,
                            recipient,
                            &FolderKey::generate().to_base64(),
                        ),
                    })
                })
            })
            .collect::<Vec<_>>();
        bootstrap_grants.pop();
        let body = serde_json::json!({
            "brainId": "acme",
            "kind": "organization",
            "name": "Acme",
            "requestingUserNpub": requester_npub,
            "bootstrapGrants": bootstrap_grants,
        })
        .to_string();
        let router = test_router();

        let response = post_brain(
            router.clone(),
            &agent_keys,
            &body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;

        assert_error(
            response,
            StatusCode::BAD_REQUEST,
            "bootstrap grants must exactly match required Folder Key Grant recipients",
        )
        .await;
        let brains = authed_request(
            router,
            &agent_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(brains.status(), StatusCode::OK);
        let brains: VisibleBrainsResponse = read_json(brains).await;
        assert!(brains.brains.is_empty());
    }

    #[tokio::test]
    async fn requester_bootstrap_failure_leaves_no_organization_brain() {
        let agent_keys = Keys::generate();
        let agent_npub = npub(&agent_keys);
        let body = serde_json::json!({
            "brainId": "acme",
            "kind": "organization",
            "name": "Acme",
            "requestingUserNpub": agent_npub,
        })
        .to_string();
        let router = test_router();

        let response = post_brain(
            router.clone(),
            &agent_keys,
            &body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;

        assert_error(
            response,
            StatusCode::BAD_REQUEST,
            "Organization Brain creator and requester must be distinct Member Identities",
        )
        .await;
        let brains = authed_request(
            router,
            &agent_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(brains.status(), StatusCode::OK);
        let brains: VisibleBrainsResponse = read_json(brains).await;
        assert!(brains.brains.is_empty());
    }

    #[tokio::test]
    async fn invalid_requester_identity_leaves_no_organization_brain() {
        let agent_keys = Keys::generate();
        let body = serde_json::json!({
            "brainId": "acme",
            "kind": "organization",
            "name": "Acme",
            "requestingUserNpub": "devfinity@finite.computer",
        })
        .to_string();
        let router = test_router();

        let response = post_brain(
            router.clone(),
            &agent_keys,
            &body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let brains = authed_request(
            router,
            &agent_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(brains.status(), StatusCode::OK);
        let brains: VisibleBrainsResponse = read_json(brains).await;
        assert!(brains.brains.is_empty());
    }

    #[tokio::test]
    async fn personal_brain_creation_rejects_organization_requester_identity() {
        let owner_keys = Keys::generate();
        let requester_keys = Keys::generate();
        let body = serde_json::json!({
            "brainId": "personal",
            "kind": "personal",
            "name": "Personal Brain",
            "requestingUserNpub": npub(&requester_keys),
        })
        .to_string();

        let response = post_brain(
            test_router(),
            &owner_keys,
            &body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;

        assert_error(
            response,
            StatusCode::BAD_REQUEST,
            "Organization Brain requester identity is only valid for an Organization Brain",
        )
        .await;
    }

    #[tokio::test]
    async fn organization_brain_creation_rejects_personal_agent_fields() {
        let admin_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let state = test_state();
        let body = serde_json::json!({
            "brainId": "acme",
            "kind": "organization",
            "name": "Acme",
            "personalAgentNpub": npub(&agent_keys),
        })
        .to_string();

        let response = post_brain(
            router_with_state(state.clone()),
            &admin_keys,
            &body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;

        assert_error(
            response,
            StatusCode::BAD_REQUEST,
            "Personal Agent identity is only valid for a Personal Brain",
        )
        .await;
        assert!(
            state
                .store
                .lock()
                .unwrap()
                .list_visible_brains(&UserId::new(npub(&admin_keys)).unwrap())
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn owner_creates_empty_personal_brain_by_verified_agent_npub_with_email_aliases() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_npub = npub(&owner_keys);
        let agent_npub = npub(&agent_keys);
        let owner_email = "owner@finite.computer";
        let agent_email = "cheater@finite.vip";
        let (identity_url, identity_server) = spawn_json_authority(vec![
            (
                "/api/v1/operator/brain/agent-resolution",
                serde_json::json!({
                    "agentNpub": agent_npub,
                    "managedAgentEmail": agent_email,
                }),
            ),
            (
                "/api/v1/operator/brain/user-resolution",
                serde_json::json!({
                    "workosUserId": "user_workos_owner",
                    "userNpub": owner_npub,
                }),
            ),
        ]);
        let (core_url, core_server) = spawn_json_authority(vec![(
            "/api/core/v1/brain/agent-account",
            serde_json::json!({
                "workosUserId": "user_workos_owner",
                "managedAgentEmail": agent_email,
                "verifiedEmail": owner_email,
                "status": "active",
            }),
        )]);
        let router = router_with_state(test_state().with_agent_bootstrap_authorities(
            core_url,
            "core-token",
            identity_url,
            "identity-token",
        ));
        let body = serde_json::json!({
            "brainId": "personal",
            "kind": "personal",
            "name": "Personal Brain",
            "personalAgentNpub": agent_npub,
        })
        .to_string();

        let created = post_brain(
            router.clone(),
            &owner_keys,
            &body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(created.status(), StatusCode::OK);
        let created: BrainMetadataResponse = read_json(created).await;
        assert_eq!(created.kind, BrainKind::Personal);
        assert_eq!(created.owner_user_id.as_deref(), Some(owner_npub.as_str()));
        assert!(created.folders.is_empty());
        assert_eq!(created.grant_count, 0);
        assert_eq!(
            created
                .personal_agent
                .as_ref()
                .map(|relationship| relationship.agent_npub.as_str()),
            Some(agent_npub.as_str())
        );

        let agent_brains = authed_request(
            router.clone(),
            &agent_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(agent_brains.status(), StatusCode::OK);
        let agent_brains: VisibleBrainsResponse = read_json(agent_brains).await;
        assert_eq!(agent_brains.brains.len(), 1);
        assert_eq!(agent_brains.brains[0].brain_id, "personal");
        assert_eq!(agent_brains.brains[0].role, "personal_agent");
        assert!(
            created
                .identities
                .iter()
                .any(|identity| { identity.npub == owner_npub && identity.display == owner_email })
        );
        assert!(
            created
                .identities
                .iter()
                .any(|identity| { identity.npub == agent_npub && identity.display == agent_email })
        );
        identity_server.join().unwrap();
        core_server.join().unwrap();
    }

    #[tokio::test]
    async fn owner_cannot_create_a_personal_brain_without_selecting_a_personal_agent() {
        let owner_keys = Keys::generate();
        let state = test_state();
        let router = router_with_state(state.clone());
        let body = create_brain_body("personal", "personal");

        let response = post_brain(router, &owner_keys, &body, TEST_NOW, None, None, None).await;

        assert_error(
            response,
            StatusCode::BAD_REQUEST,
            "Personal Brain creation requires a Personal Agent email or npub",
        )
        .await;
        assert!(
            state
                .store
                .lock()
                .unwrap()
                .list_visible_brains(&UserId::new(npub(&owner_keys)).unwrap())
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn owner_cannot_select_an_agent_from_another_workos_account() {
        let owner_keys = Keys::generate();
        let other_owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let agent_npub = npub(&agent_keys);
        let (identity_url, identity_server) = spawn_json_authority(vec![
            (
                "/api/v1/operator/brain/agent-resolution",
                serde_json::json!({
                    "agentNpub": agent_npub,
                    "managedAgentEmail": "other-agent@finite.vip",
                }),
            ),
            (
                "/api/v1/operator/brain/user-resolution",
                serde_json::json!({
                    "workosUserId": "user_workos_other",
                    "userNpub": npub(&other_owner_keys),
                }),
            ),
        ]);
        let (core_url, core_server) = spawn_json_authority(vec![(
            "/api/core/v1/brain/agent-account",
            serde_json::json!({
                "workosUserId": "user_workos_other",
                "managedAgentEmail": "other-agent@finite.vip",
                "verifiedEmail": "other@finite.computer",
                "status": "active",
            }),
        )]);
        let state = test_state().with_agent_bootstrap_authorities(
            core_url,
            "core-token",
            identity_url,
            "identity-token",
        );
        let router = router_with_state(state.clone());
        let body = serde_json::json!({
            "brainId": "personal",
            "kind": "personal",
            "name": "Personal Brain",
            "personalAgentNpub": agent_npub,
        })
        .to_string();

        let response = post_brain(router, &owner_keys, &body, TEST_NOW, None, None, None).await;

        assert_error(
            response,
            StatusCode::FORBIDDEN,
            "selected Personal Agent does not belong to the signed owner's account",
        )
        .await;
        let store = state.store.lock().unwrap();
        assert!(
            store
                .list_visible_brains(&UserId::new(npub(&owner_keys)).unwrap())
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .load_identity_aliases(&[
                    UserId::new(npub(&owner_keys)).unwrap(),
                    UserId::new(agent_npub).unwrap(),
                ])
                .unwrap()
                .is_empty()
        );
        drop(store);
        identity_server.join().unwrap();
        core_server.join().unwrap();
    }

    #[tokio::test]
    async fn personal_agent_has_full_current_and_future_folder_access_without_a_workspace() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_npub = npub(&owner_keys);
        let agent_npub = npub(&agent_keys);
        let router = personal_test_router(&owner_keys, &agent_keys);

        for (actor, folder_id, name, now) in [
            (&owner_keys, "owner-notes", "Owner notes", TEST_NOW + 1),
            (&agent_keys, "agent-notes", "Agent notes", TEST_NOW + 2),
        ] {
            let body = serde_json::json!({
                "folderId": folder_id,
                "name": name,
                "role": "folder",
                "access": "owner",
                "parentFolderId": null,
                "path": folder_id,
                "sharedFolderSource": false,
                "accessUserIds": [],
                "grants": [
                    folder_key_grant_value(
                        &format!("grant-{folder_id}-owner-v1"),
                        1,
                        owner_npub.as_str(),
                    ),
                    folder_key_grant_value(
                        &format!("grant-{folder_id}-agent-v1"),
                        1,
                        agent_npub.as_str(),
                    ),
                ],
                "accessChangeEvent": admin_event(
                    actor,
                    "personal",
                    &format!("create-{folder_id}"),
                    AdminAccessAction::SetFolderAccessMode,
                    Some(folder_id),
                    None,
                    Some(1),
                ),
            })
            .to_string();
            let created = authed_request(
                router.clone(),
                actor,
                "POST",
                "/_admin/brains/personal/folders",
                Some(body),
                now,
            )
            .await;
            let status = created.status();
            let text = read_text(created).await;
            assert_eq!(status, StatusCode::OK, "{text}");
        }

        let agent_metadata =
            get_metadata(router.clone(), &agent_keys, "personal", TEST_NOW + 3).await;
        assert_eq!(agent_metadata.status(), StatusCode::OK);
        let agent_metadata: BrainMetadataResponse = read_json(agent_metadata).await;
        assert_eq!(agent_metadata.folders.len(), 2);
        assert_eq!(agent_metadata.grant_count, 4);
        let owner_metadata =
            get_metadata(router.clone(), &owner_keys, "personal", TEST_NOW + 3).await;
        let owner_metadata: BrainMetadataResponse = read_json(owner_metadata).await;
        assert_eq!(owner_metadata.grant_count, 4);
        assert_eq!(agent_metadata, owner_metadata);

        let retired_pairing_route = authed_request(
            router,
            &owner_keys,
            "GET",
            "/_admin/brains/personal/agent-workspace-pairings",
            None,
            TEST_NOW + 4,
        )
        .await;
        assert_eq!(retired_pairing_route.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn personal_agent_can_permanently_delete_a_folder_subtree_over_signed_http() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_npub = npub(&owner_keys);
        let agent_npub = npub(&agent_keys);
        let router = personal_test_router(&owner_keys, &agent_keys);

        let create_folder_body = serde_json::json!({
            "folderId": "agent-notes",
            "name": "Agent notes",
            "role": "folder",
            "access": "owner",
            "parentFolderId": null,
            "path": "agent-notes",
            "sharedFolderSource": false,
            "accessUserIds": [],
            "grants": [
                folder_key_grant_value("grant-agent-notes-owner-v1", 1, owner_npub.as_str()),
                folder_key_grant_value("grant-agent-notes-agent-v1", 1, agent_npub.as_str()),
            ],
            "accessChangeEvent": admin_event(
                &agent_keys,
                "personal",
                "create-agent-notes",
                AdminAccessAction::SetFolderAccessMode,
                Some("agent-notes"),
                None,
                Some(1),
            ),
        })
        .to_string();
        assert_eq!(
            authed_request(
                router.clone(),
                &agent_keys,
                "POST",
                "/_admin/brains/personal/folders",
                Some(create_folder_body),
                TEST_NOW + 1,
            )
            .await
            .status(),
            StatusCode::OK
        );

        let object_path = "/_admin/brains/personal/folders/agent-notes/objects/obj_000000000001";
        let object_body = object_write_body(
            &agent_keys,
            RevisionFixture {
                brain_id: "personal",
                folder_id: "agent-notes",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                content: "agent note",
                nonce: 21,
                record_type: false,
            },
        );
        assert_eq!(
            authed_request(
                router.clone(),
                &agent_keys,
                "PUT",
                object_path,
                Some(object_body),
                TEST_NOW + 2,
            )
            .await
            .status(),
            StatusCode::OK
        );

        let deletion_event = admin_event(
            &agent_keys,
            "personal",
            "delete-agent-notes",
            AdminAccessAction::DeleteFolder,
            Some("agent-notes"),
            None,
            Some(1),
        );
        let stale_delete_body = serde_json::json!({
            "deletionEvent": deletion_event.clone(),
            "expectedFolderIds": ["agent-notes"],
            "expectedObjectCount": 0,
        })
        .to_string();
        let stale = authed_request(
            router.clone(),
            &agent_keys,
            "DELETE",
            "/_admin/brains/personal/folders/agent-notes",
            Some(stale_delete_body),
            TEST_NOW + 3,
        )
        .await;
        assert_error(
            stale,
            StatusCode::CONFLICT,
            "sync conflict: Folder subtree changed after destructive confirmation; current revision: None",
        )
        .await;
        let metadata = get_metadata(router.clone(), &owner_keys, "personal", TEST_NOW + 4).await;
        let metadata: BrainMetadataResponse = read_json(metadata).await;
        assert_eq!(metadata.folders.len(), 1);

        let delete_body = serde_json::json!({
            "deletionEvent": deletion_event,
            "expectedFolderIds": ["agent-notes"],
            "expectedObjectCount": 1,
        })
        .to_string();
        let deleted = authed_request(
            router.clone(),
            &agent_keys,
            "DELETE",
            "/_admin/brains/personal/folders/agent-notes",
            Some(delete_body.clone()),
            TEST_NOW + 5,
        )
        .await;
        assert_eq!(deleted.status(), StatusCode::OK);
        let deleted: FolderDeleteResponse = read_json(deleted).await;
        assert_eq!(deleted.folder_count, 1);
        assert_eq!(deleted.object_count, 1);

        let retry = authed_request(
            router.clone(),
            &agent_keys,
            "DELETE",
            "/_admin/brains/personal/folders/agent-notes",
            Some(delete_body),
            TEST_NOW + 6,
        )
        .await;
        assert_eq!(retry.status(), StatusCode::OK);
        let retry: FolderDeleteResponse = read_json(retry).await;
        assert!(retry.duplicate);
        assert_eq!(retry.folder_count, deleted.folder_count);
        assert_eq!(retry.object_count, deleted.object_count);

        let metadata = get_metadata(router, &owner_keys, "personal", TEST_NOW + 7).await;
        assert_eq!(metadata.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(metadata).await;
        assert!(metadata.folders.is_empty());
    }

    #[tokio::test]
    async fn organization_member_cannot_permanently_delete_a_folder() {
        let admin_keys = Keys::generate();
        let member_keys = Keys::generate();
        let member_npub = npub(&member_keys);
        let router = router_with_test_org_folders(&admin_keys).await;
        let add_member_body = serde_json::json!({
            "targetNpub": member_npub,
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "add-delete-test-member",
                AdminAccessAction::AddMember,
                None,
                Some(member_npub.as_str()),
                None,
            ),
        })
        .to_string();
        assert_eq!(
            authed_request(
                router.clone(),
                &admin_keys,
                "POST",
                "/_admin/brains/acme/members",
                Some(add_member_body),
                TEST_NOW,
            )
            .await
            .status(),
            StatusCode::OK
        );

        let delete_body = serde_json::json!({
            "deletionEvent": admin_event(
                &member_keys,
                "acme",
                "member-delete-attempt",
                AdminAccessAction::DeleteFolder,
                Some("getting-started"),
                None,
                Some(1),
            ),
            "expectedFolderIds": ["getting-started"],
            "expectedObjectCount": 0,
        })
        .to_string();
        let denied = authed_request(
            router,
            &member_keys,
            "DELETE",
            "/_admin/brains/acme/folders/getting-started",
            Some(delete_body),
            TEST_NOW + 1,
        )
        .await;
        assert_error(
            denied,
            StatusCode::FORBIDDEN,
            "permanent deletion requires brain destructive authority",
        )
        .await;
    }

    #[tokio::test]
    async fn affected_member_receives_folder_subtree_tombstone_without_broadcasting_it() {
        let admin_keys = Keys::generate();
        let affected_keys = Keys::generate();
        let unrelated_keys = Keys::generate();
        let affected_npub = npub(&affected_keys);
        let unrelated_npub = npub(&unrelated_keys);
        let router = router_with_test_org_folders(&admin_keys).await;

        for (keys, event_id) in [
            (&affected_keys, "add-affected-delete-member"),
            (&unrelated_keys, "add-unrelated-delete-member"),
        ] {
            let member_npub = npub(keys);
            let body = serde_json::json!({
                "targetNpub": member_npub,
                "accessChangeEvent": admin_event(
                    &admin_keys,
                    "acme",
                    event_id,
                    AdminAccessAction::AddMember,
                    None,
                    Some(member_npub.as_str()),
                    None,
                ),
            })
            .to_string();
            assert_eq!(
                authed_request(
                    router.clone(),
                    &admin_keys,
                    "POST",
                    "/_admin/brains/acme/members",
                    Some(body),
                    TEST_NOW,
                )
                .await
                .status(),
                StatusCode::OK
            );
        }

        let create_folder_body = serde_json::json!({
            "folderId": "delete-restricted",
            "name": "Delete restricted",
            "role": "folder",
            "access": "restricted",
            "parentFolderId": null,
            "path": "Delete restricted",
            "sharedFolderSource": false,
            "accessUserIds": [affected_npub],
            "grants": [
                folder_key_grant_value(
                    "grant-delete-restricted-admin-v1",
                    1,
                    npub(&admin_keys).as_str(),
                ),
                folder_key_grant_value(
                    "grant-delete-restricted-member-v1",
                    1,
                    affected_npub.as_str(),
                ),
            ],
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "create-delete-restricted",
                AdminAccessAction::SetFolderAccessMode,
                Some("delete-restricted"),
                None,
                Some(1),
            ),
        })
        .to_string();
        assert_eq!(
            authed_request(
                router.clone(),
                &admin_keys,
                "POST",
                "/_admin/brains/acme/folders",
                Some(create_folder_body),
                TEST_NOW + 1,
            )
            .await
            .status(),
            StatusCode::OK
        );

        let before_delete = authed_request(
            router.clone(),
            &admin_keys,
            "GET",
            "/_admin/brains/acme/sync/bootstrap",
            None,
            TEST_NOW + 2,
        )
        .await;
        let before_delete: SyncBootstrapResponse = read_json(before_delete).await;
        let delete_body = serde_json::json!({
            "deletionEvent": admin_event(
                &admin_keys,
                "acme",
                "delete-restricted-subtree",
                AdminAccessAction::DeleteFolder,
                Some("delete-restricted"),
                None,
                Some(1),
            ),
            "expectedFolderIds": ["delete-restricted"],
            "expectedObjectCount": 0,
        })
        .to_string();
        assert_eq!(
            authed_request(
                router.clone(),
                &admin_keys,
                "DELETE",
                "/_admin/brains/acme/folders/delete-restricted",
                Some(delete_body),
                TEST_NOW + 3,
            )
            .await
            .status(),
            StatusCode::OK
        );

        for (keys, should_see_tombstone) in [(&affected_keys, true), (&unrelated_keys, false)] {
            let pull = authed_request(
                router.clone(),
                keys,
                "GET",
                &format!(
                    "/_admin/brains/acme/sync/records?after={}&limit=20",
                    before_delete.latest_sequence
                ),
                None,
                TEST_NOW + 4,
            )
            .await;
            assert_eq!(pull.status(), StatusCode::OK);
            let pull: SyncPullResponse = read_json(pull).await;
            assert_eq!(
                pull.records.iter().any(|record| {
                    record.record_type == "brain_admin_access_change"
                        && record.payload_json.contains("folder_subtree_tombstone")
                }),
                should_see_tombstone
            );

            let bootstrap = authed_request(
                router.clone(),
                keys,
                "GET",
                "/_admin/brains/acme/sync/bootstrap",
                None,
                TEST_NOW + 5,
            )
            .await;
            assert_eq!(bootstrap.status(), StatusCode::OK);
            let bootstrap: SyncBootstrapResponse = read_json(bootstrap).await;
            assert_eq!(
                bootstrap.control_records.iter().any(|record| {
                    record.record_type == "brain_admin_access_change"
                        && record.payload_json.contains("folder_subtree_tombstone")
                }),
                should_see_tombstone
            );
        }

        assert_ne!(affected_npub, unrelated_npub);
    }

    #[tokio::test]
    async fn owner_replaces_personal_agent_by_managed_email_with_durable_rotation_records() {
        let owner_keys = Keys::generate();
        let owner_npub = npub(&owner_keys);
        let old_agent_keys = Keys::generate();
        let old_agent_npub = npub(&old_agent_keys);
        let collaborator_keys = Keys::generate();
        let collaborator_npub = npub(&collaborator_keys);
        let replacement_keys = Keys::generate();
        let replacement_key = NostrPublicKey::from_protocol(replacement_keys.public_key());
        let replacement_npub = replacement_key.to_npub().unwrap();
        let identifier = Nip05Identifier::parse("replacement@finite.vip").unwrap();
        let document =
            serde_json::json!({ "names": { "replacement": replacement_key.to_hex() } }).to_string();
        let agent_resolution = serde_json::json!({
            "agentNpub": replacement_npub,
            "managedAgentEmail": "replacement@finite.vip",
        });
        let account_resolution = serde_json::json!({
            "workosUserId": "user_workos_replacement_owner",
            "managedAgentEmail": "replacement@finite.vip",
            "status": "active",
        });
        let owner_resolution = serde_json::json!({
            "workosUserId": "user_workos_replacement_owner",
            "userNpub": owner_npub,
        });
        let (identity_url, identity_server) = spawn_json_authority(vec![
            (
                "/api/v1/operator/brain/agent-resolution",
                agent_resolution.clone(),
            ),
            (
                "/api/v1/operator/brain/user-resolution",
                owner_resolution.clone(),
            ),
            ("/api/v1/operator/brain/agent-resolution", agent_resolution),
            ("/api/v1/operator/brain/user-resolution", owner_resolution),
        ]);
        let (core_url, core_server) = spawn_json_authority(vec![
            (
                "/api/core/v1/brain/agent-account",
                account_resolution.clone(),
            ),
            ("/api/core/v1/brain/agent-account", account_resolution),
        ]);
        let state = personal_test_state(&owner_keys, &old_agent_keys)
            .with_nip05_fixture(identifier.well_known_request().url, document)
            .with_agent_bootstrap_authorities(
                core_url,
                "core-token",
                identity_url,
                "identity-token",
            );
        let router = router_with_state(state.clone());

        let create_folder = serde_json::json!({
            "folderId": "personal-notes",
            "name": "Personal notes",
            "role": "folder",
            "access": "restricted",
            "parentFolderId": null,
            "path": "personal-notes",
            "sharedFolderSource": false,
            "accessUserIds": [],
            "grants": [
                folder_key_grant_value("grant-personal-notes-owner-v1", 1, owner_npub.as_str()),
                folder_key_grant_value("grant-personal-notes-agent-v1", 1, old_agent_npub.as_str()),
            ],
            "accessChangeEvent": admin_event(
                &owner_keys,
                "personal",
                "create-personal-notes",
                AdminAccessAction::SetFolderAccessMode,
                Some("personal-notes"),
                None,
                Some(1),
            ),
        })
        .to_string();
        assert_eq!(
            authed_request(
                router.clone(),
                &owner_keys,
                "POST",
                "/_admin/brains/personal/folders",
                Some(create_folder),
                TEST_NOW + 1,
            )
            .await
            .status(),
            StatusCode::OK
        );

        let grant_collaborator = serde_json::json!({
            "targetNpub": collaborator_npub,
            "grant": folder_key_grant_value(
                "grant-personal-notes-collaborator-v1",
                1,
                collaborator_npub.as_str(),
            ),
            "accessChangeEvent": admin_event(
                &owner_keys,
                "personal",
                "grant-personal-notes-collaborator",
                AdminAccessAction::GrantFolderAccess,
                Some("personal-notes"),
                Some(collaborator_npub.as_str()),
                Some(1),
            ),
        })
        .to_string();
        assert_eq!(
            authed_request(
                router.clone(),
                &owner_keys,
                "POST",
                "/_admin/brains/personal/folders/personal-notes/access",
                Some(grant_collaborator),
                TEST_NOW + 1,
            )
            .await
            .status(),
            StatusCode::OK
        );

        let replace_body = serde_json::json!({
            "agentEmail": "replacement@finite.vip",
            "rotations": [{
                "folderId": "personal-notes",
                "newKeyVersion": 2,
                "grants": [
                    folder_key_grant_value("grant-personal-notes-owner-v2", 2, owner_npub.as_str()),
                    folder_key_grant_value("grant-personal-notes-agent-v2", 2, replacement_npub.as_str()),
                    folder_key_grant_value("grant-personal-notes-collaborator-v2", 2, collaborator_npub.as_str()),
                ],
                "reencryptedRecords": [],
                "accessChangeEvent": admin_event(
                    &owner_keys,
                    "personal",
                    "replace-personal-notes-agent",
                    AdminAccessAction::RotateFolderKey,
                    Some("personal-notes"),
                    Some(replacement_npub.as_str()),
                    Some(2),
                ),
            }],
        })
        .to_string();
        let replaced = authed_request(
            router.clone(),
            &owner_keys,
            "PUT",
            "/_admin/brains/personal/personal-agent",
            Some(replace_body),
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(replaced.status(), StatusCode::OK);
        let replaced: BrainMetadataResponse = read_json(replaced).await;
        assert_eq!(
            replaced
                .personal_agent
                .as_ref()
                .map(|relationship| relationship.agent_npub.as_str()),
            Some(replacement_npub.as_str())
        );

        let old_agent_brains = authed_request(
            router.clone(),
            &old_agent_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 2,
        )
        .await;
        assert_eq!(old_agent_brains.status(), StatusCode::OK);
        let old_agent_brains: VisibleBrainsResponse = read_json(old_agent_brains).await;
        assert!(old_agent_brains.brains.is_empty());

        let replacement_brains = authed_request(
            router.clone(),
            &replacement_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 2,
        )
        .await;
        assert_eq!(replacement_brains.status(), StatusCode::OK);
        let replacement_brains: VisibleBrainsResponse = read_json(replacement_brains).await;
        assert_eq!(replacement_brains.brains.len(), 1);
        assert_eq!(replacement_brains.brains[0].role, "personal_agent");

        let replacement_sync = authed_request(
            router.clone(),
            &replacement_keys,
            "GET",
            "/_admin/brains/personal/sync/bootstrap",
            None,
            TEST_NOW + 3,
        )
        .await;
        assert_eq!(replacement_sync.status(), StatusCode::OK);
        let replacement_sync: SyncBootstrapResponse = read_json(replacement_sync).await;
        assert!(replacement_sync.control_records.iter().any(|record| {
            record.folder_id.as_deref() == Some("personal-notes")
                && record.record_type == "folder_key_grant"
                && record.payload_json.contains("\"keyVersion\":2")
        }));
        assert!(replacement_sync.control_records.iter().any(|record| {
            record.folder_id.as_deref() == Some("personal-notes")
                && record.record_type == "brain_admin_access_change"
                && record.payload_json.contains("replace-personal-notes-agent")
        }));

        let remove_body = serde_json::json!({
            "agentEmail": null,
            "rotations": [{
                "folderId": "personal-notes",
                "newKeyVersion": 3,
                "grants": [
                    folder_key_grant_value("grant-personal-notes-owner-v3", 3, owner_npub.as_str()),
                    folder_key_grant_value("grant-personal-notes-collaborator-v3", 3, collaborator_npub.as_str()),
                ],
                "reencryptedRecords": [],
                "accessChangeEvent": admin_event(
                    &owner_keys,
                    "personal",
                    "remove-personal-notes-agent",
                    AdminAccessAction::RotateFolderKey,
                    Some("personal-notes"),
                    None,
                    Some(3),
                ),
            }],
        })
        .to_string();
        let removed = authed_request(
            router.clone(),
            &owner_keys,
            "PUT",
            "/_admin/brains/personal/personal-agent",
            Some(remove_body),
            TEST_NOW + 3,
        )
        .await;
        assert_eq!(removed.status(), StatusCode::OK);
        let removed: BrainMetadataResponse = read_json(removed).await;
        assert!(removed.personal_agent.is_none());

        let reassign_body = serde_json::json!({
            "agentEmail": "replacement@finite.vip",
            "rotations": [{
                "folderId": "personal-notes",
                "newKeyVersion": 4,
                "grants": [
                    folder_key_grant_value("grant-personal-notes-owner-v4", 4, owner_npub.as_str()),
                    folder_key_grant_value("grant-personal-notes-agent-v4", 4, replacement_npub.as_str()),
                    folder_key_grant_value("grant-personal-notes-collaborator-v4", 4, collaborator_npub.as_str()),
                ],
                "reencryptedRecords": [],
                "accessChangeEvent": admin_event(
                    &owner_keys,
                    "personal",
                    "reassign-personal-notes-agent",
                    AdminAccessAction::RotateFolderKey,
                    Some("personal-notes"),
                    Some(replacement_npub.as_str()),
                    Some(4),
                ),
            }],
        })
        .to_string();
        let reassigned = authed_request(
            router,
            &owner_keys,
            "PUT",
            "/_admin/brains/personal/personal-agent",
            Some(reassign_body),
            TEST_NOW + 4,
        )
        .await;
        assert_eq!(reassigned.status(), StatusCode::OK);
        let reassigned: BrainMetadataResponse = read_json(reassigned).await;
        assert_eq!(
            reassigned
                .personal_agent
                .as_ref()
                .map(|relationship| relationship.agent_npub.as_str()),
            Some(replacement_npub.as_str())
        );
        identity_server.join().unwrap();
        core_server.join().unwrap();
    }

    #[tokio::test]
    async fn managed_agent_replacement_requires_the_brain_owners_core_account() {
        let brain_owner = Keys::generate();
        let different_owner = Keys::generate();
        let replacement = Keys::generate();
        let replacement_key = NostrPublicKey::from_protocol(replacement.public_key());
        let replacement_npub = replacement_key.to_npub().unwrap();
        let identifier = Nip05Identifier::parse("replacement@finite.vip").unwrap();
        let document =
            serde_json::json!({ "names": { "replacement": replacement_key.to_hex() } }).to_string();
        let (identity_url, identity_server) = spawn_json_authority(vec![
            (
                "/api/v1/operator/brain/agent-resolution",
                serde_json::json!({
                    "agentNpub": replacement_npub,
                    "managedAgentEmail": "replacement@finite.vip",
                }),
            ),
            (
                "/api/v1/operator/brain/user-resolution",
                serde_json::json!({
                    "workosUserId": "user_workos_different_owner",
                    "userNpub": npub(&different_owner),
                }),
            ),
        ]);
        let (core_url, core_server) = spawn_json_authority(vec![(
            "/api/core/v1/brain/agent-account",
            serde_json::json!({
                "workosUserId": "user_workos_different_owner",
                "managedAgentEmail": "replacement@finite.vip",
                "status": "active",
            }),
        )]);
        let state = personal_test_state(&brain_owner, &Keys::generate())
            .with_nip05_fixture(identifier.well_known_request().url, document)
            .with_agent_bootstrap_authorities(
                core_url,
                "core-token",
                identity_url,
                "identity-token",
            );

        let error = resolve_managed_agent_email(
            &state,
            "replacement@finite.vip",
            &UserId::new(npub(&brain_owner)).unwrap(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.status, StatusCode::FORBIDDEN);
        assert_eq!(
            error.message,
            "Managed Agent does not belong to the Personal Brain owner's account"
        );
        identity_server.join().unwrap();
        core_server.join().unwrap();
    }

    #[tokio::test]
    async fn owner_creates_personal_brain_by_managed_agent_email_without_trusting_navigation_npub()
    {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let wrong_agent_keys = Keys::generate();
        let agent_key = NostrPublicKey::from_protocol(agent_keys.public_key());
        let agent_hex = agent_key.to_hex();
        let agent_npub = agent_key.to_npub().unwrap();
        let identifier = Nip05Identifier::parse("cheater@finite.vip").unwrap();
        let document = serde_json::json!({ "names": { "cheater": agent_hex } }).to_string();
        let agent_resolution = serde_json::json!({
            "agentNpub": agent_npub,
            "managedAgentEmail": "cheater@finite.vip",
        });
        let account_resolution = serde_json::json!({
            "workosUserId": "user_workos_owner",
            "managedAgentEmail": "cheater@finite.vip",
            "verifiedEmail": "owner@finite.computer",
            "status": "active",
        });
        let (identity_url, identity_server) = spawn_json_authority(vec![
            ("/api/v1/operator/brain/agent-resolution", agent_resolution),
            (
                "/api/v1/operator/brain/user-resolution",
                serde_json::json!({
                    "workosUserId": "user_workos_owner",
                    "userNpub": npub(&owner_keys),
                }),
            ),
        ]);
        let (core_url, core_server) = spawn_json_authority(vec![(
            "/api/core/v1/brain/agent-account",
            account_resolution,
        )]);
        let state = test_state()
            .with_nip05_fixture(identifier.well_known_request().url, document)
            .with_agent_bootstrap_authorities(
                core_url,
                "core-token",
                identity_url,
                "identity-token",
            );
        let router = router_with_state(state.clone());

        let mismatch_body = serde_json::json!({
            "brainId": "personal",
            "kind": "personal",
            "name": "Personal Brain",
            "personalAgentEmail": "cheater@finite.vip",
            "personalAgentNpub": npub(&wrong_agent_keys),
        })
        .to_string();
        let mismatch = post_brain(
            router.clone(),
            &owner_keys,
            &mismatch_body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_error(
            mismatch,
            StatusCode::BAD_REQUEST,
            "personalAgentEmail and personalAgentNpub resolve to different Agent Principals",
        )
        .await;
        assert!(
            state
                .store
                .lock()
                .unwrap()
                .load_identity_aliases(&[
                    UserId::new(agent_npub.clone()).unwrap(),
                    UserId::new(npub(&wrong_agent_keys)).unwrap(),
                ])
                .unwrap()
                .is_empty(),
            "a rejected Managed Agent verification must not record identity aliases"
        );

        let owner_brains = authed_request(
            router.clone(),
            &owner_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 1,
        )
        .await;
        let owner_brains: VisibleBrainsResponse = read_json(owner_brains).await;
        assert!(owner_brains.brains.is_empty());

        let email_body = serde_json::json!({
            "brainId": "personal",
            "kind": "personal",
            "name": "Personal Brain",
            "personalAgentEmail": "cheater@finite.vip",
        })
        .to_string();
        let created = post_brain(
            router.clone(),
            &owner_keys,
            &email_body,
            TEST_NOW + 2,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(created.status(), StatusCode::OK);
        let created: BrainMetadataResponse = read_json(created).await;
        assert_eq!(
            created
                .personal_agent
                .as_ref()
                .map(|relationship| relationship.agent_npub.as_str()),
            Some(agent_npub.as_str())
        );
        assert!(created.identities.iter().any(|identity| {
            identity.npub == agent_npub && identity.nip05.as_deref() == Some("cheater@finite.vip")
        }));
        assert!(created.identities.iter().any(|identity| {
            identity.npub == npub(&owner_keys)
                && identity.nip05.as_deref() == Some("owner@finite.computer")
        }));

        let agent_brains = authed_request(
            router,
            &agent_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 3,
        )
        .await;
        let agent_brains: VisibleBrainsResponse = read_json(agent_brains).await;
        assert_eq!(agent_brains.brains.len(), 1);
        assert_eq!(agent_brains.brains[0].role, "personal_agent");
        identity_server.join().unwrap();
        core_server.join().unwrap();
    }

    #[tokio::test]
    async fn same_owner_cannot_create_multiple_personal_brains() {
        let keys = Keys::generate();
        let agent_keys = Keys::generate();
        let agent_npub = npub(&agent_keys);
        let (identity_url, identity_server) = spawn_json_authority(vec![
            (
                "/api/v1/operator/brain/agent-resolution",
                serde_json::json!({
                    "agentNpub": agent_npub,
                    "managedAgentEmail": "agent@finite.vip",
                }),
            ),
            (
                "/api/v1/operator/brain/user-resolution",
                serde_json::json!({
                    "workosUserId": "user_workos_owner",
                    "userNpub": npub(&keys),
                }),
            ),
        ]);
        let (core_url, core_server) = spawn_json_authority(vec![(
            "/api/core/v1/brain/agent-account",
            serde_json::json!({
                "workosUserId": "user_workos_owner",
                "managedAgentEmail": "agent@finite.vip",
                "verifiedEmail": "owner@finite.computer",
                "status": "active",
            }),
        )]);
        let router = router_with_state(
            personal_test_state(&keys, &agent_keys).with_agent_bootstrap_authorities(
                core_url,
                "core-token",
                identity_url,
                "identity-token",
            ),
        );
        let body = serde_json::json!({
            "brainId": "personal-b",
            "kind": "personal",
            "name": "Personal Brain",
            "personalAgentNpub": agent_npub,
        })
        .to_string();
        let second = post_brain(router, &keys, &body, TEST_NOW, None, None, None).await;

        assert_error(
            second,
            StatusCode::BAD_REQUEST,
            "user already has a personal brain",
        )
        .await;
        identity_server.join().unwrap();
        core_server.join().unwrap();
    }

    #[tokio::test]
    async fn visible_brains_lists_personal_and_member_organizations() {
        let keys = Keys::generate();
        let agent_keys = Keys::generate();
        let invited_keys = Keys::generate();
        let invited_npub = npub(&invited_keys);
        let router = personal_test_router(&keys, &agent_keys);

        let org = post_brain(
            router.clone(),
            &keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW + 1,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(org.status(), StatusCode::OK);
        add_test_org_folders(&router, &keys).await;

        let list = authed_request(
            router.clone(),
            &keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 2,
        )
        .await;
        assert_eq!(list.status(), StatusCode::OK);
        let list: VisibleBrainsResponse = read_json(list).await;
        assert_eq!(list.brains.len(), 2);
        assert_eq!(list.brains[0].brain_id, "personal");
        assert_eq!(list.brains[0].kind, BrainKind::Personal);
        assert_eq!(list.brains[0].role, "owner");
        assert_eq!(list.brains[1].brain_id, "acme");
        assert_eq!(list.brains[1].kind, BrainKind::Organization);
        assert_eq!(list.brains[1].role, "admin");

        let invite_body = serde_json::json!({
            "targetNpub": invited_npub,
            "initialFolderAccess": ["getting-started"],
            "expiresAt": "2099-06-30T00:00:00.000Z",
        })
        .to_string();
        let invite = authed_request(
            router.clone(),
            &keys,
            "POST",
            "/_admin/brains/acme/invitations",
            Some(invite_body),
            TEST_NOW + 3,
        )
        .await;
        assert_eq!(invite.status(), StatusCode::OK);
        let invitation: BrainInvitationResponse = read_json(invite).await;

        let invited_list = authed_request(
            router,
            &invited_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 4,
        )
        .await;
        assert_eq!(invited_list.status(), StatusCode::OK);
        let invited_list: VisibleBrainsResponse = read_json(invited_list).await;
        assert_eq!(invited_list.brains.len(), 1);
        assert_eq!(invited_list.brains[0].brain_id, "acme");
        assert_eq!(invited_list.brains[0].kind, BrainKind::Organization);
        assert_eq!(invited_list.brains[0].role, "invited");
        assert_eq!(
            invited_list.brains[0].invite_code.as_deref(),
            Some(invitation.invite_code.as_str())
        );
    }

    #[tokio::test]
    async fn visible_brains_does_not_list_pending_invites_for_existing_members() {
        let admin_keys = Keys::generate();
        let target_keys = Keys::generate();
        let target_npub = npub(&target_keys);
        let state = test_state();
        let router = router_with_state(state.clone());
        let create_brain = post_brain(
            router.clone(),
            &admin_keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_brain.status(), StatusCode::OK);
        add_test_org_folders(&router, &admin_keys).await;

        let invite_body = serde_json::json!({
            "targetNpub": target_npub,
            "initialFolderAccess": ["getting-started"],
            "expiresAt": "2099-06-30T00:00:00.000Z",
        })
        .to_string();
        let invite = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/invitations",
            Some(invite_body),
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(invite.status(), StatusCode::OK);
        let invitation: BrainInvitationResponse = read_json(invite).await;

        {
            let mut store = state.store.lock().unwrap();
            store
                .add_member(
                    &BrainId::new("acme").unwrap(),
                    &UserId::new(target_npub.clone()).unwrap(),
                )
                .unwrap();
        }

        let invited_list = authed_request(
            router.clone(),
            &target_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 2,
        )
        .await;
        assert_eq!(invited_list.status(), StatusCode::OK);
        let invited_list: VisibleBrainsResponse = read_json(invited_list).await;
        assert_eq!(invited_list.brains.len(), 1);
        assert_eq!(invited_list.brains[0].brain_id, "acme");
        assert_eq!(invited_list.brains[0].role, "member");
        assert!(invited_list.brains[0].invite_code.is_none());

        let accept_path = format!(
            "/_admin/brain-invitation-links/{}/accept",
            invitation.invite_code
        );
        let accept = authed_request(
            router,
            &target_keys,
            "POST",
            &accept_path,
            None,
            TEST_NOW + 3,
        )
        .await;
        assert_eq!(accept.status(), StatusCode::OK);
        let accepted: BrainInvitationResponse = read_json(accept).await;
        assert_eq!(accepted.status, "accepted");
        assert!(accepted.duplicate_accept);
    }

    #[tokio::test]
    async fn identity_resolution_persists_nip05_and_member_routes_accept_hex() {
        let admin_keys = Keys::generate();
        let target_keys = Keys::generate();
        let target_key = NostrPublicKey::from_protocol(target_keys.public_key());
        let target_hex = target_key.to_hex();
        let target_npub = target_key.to_npub().unwrap();
        let identifier = Nip05Identifier::parse("alice@example.com").unwrap();
        let document = format!(
            r#"{{
                "names": {{"alice": "{target_hex}"}},
                "relays": {{"{target_hex}": ["wss://relay.example.com"]}}
            }}"#
        );
        let router = router_with_state(
            test_state().with_nip05_fixture(identifier.well_known_request().url, document),
        );
        let create_brain = post_brain(
            router.clone(),
            &admin_keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_brain.status(), StatusCode::OK);

        let resolve_body = serde_json::json!({ "input": "alice@example.com" }).to_string();
        let resolved = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/identities/resolve",
            Some(resolve_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(resolved.status(), StatusCode::OK);
        let resolved: IdentityResponse = read_json(resolved).await;
        assert_eq!(resolved.npub, target_npub);
        assert_eq!(resolved.hex, target_hex);
        assert_eq!(resolved.nip05.as_deref(), Some("alice@example.com"));
        assert_eq!(resolved.display, "alice@example.com");

        let add_member_body = serde_json::json!({
            "targetNpub": target_hex,
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "change_add_member_hex",
                AdminAccessAction::AddMember,
                None,
                Some(target_npub.as_str()),
                None,
            ),
        })
        .to_string();
        let add_member = authed_request(
            router,
            &admin_keys,
            "POST",
            "/_admin/brains/acme/members",
            Some(add_member_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(add_member.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(add_member).await;
        assert!(metadata.members.contains(&target_npub));
        assert!(!metadata.members.contains(&target_hex));
        assert!(metadata.identities.iter().any(|identity| {
            identity.npub == target_npub
                && identity.hex == target_hex
                && identity.display == "alice@example.com"
        }));
    }

    #[tokio::test]
    async fn personal_brain_owner_can_create_owner_folder() {
        let keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_npub = npub(&keys);
        let agent_npub = npub(&agent_keys);
        let router = personal_test_router(&keys, &agent_keys);

        let body = serde_json::json!({
            "folderId": "notes",
            "name": "Notes",
            "role": "folder",
            "access": "owner",
            "parentFolderId": null,
            "path": "Notes",
            "sharedFolderSource": false,
            "accessUserIds": [],
            "grants": [
                folder_key_grant_value("grant-notes-owner-v1", 1, owner_npub.as_str()),
                folder_key_grant_value("grant-notes-agent-v1", 1, agent_npub.as_str())
            ],
            "accessChangeEvent": admin_event(
                &keys,
                "personal",
                "change-create-notes",
                AdminAccessAction::SetFolderAccessMode,
                Some("notes"),
                None,
                Some(1),
            )
        })
        .to_string();
        let response = authed_request(
            router,
            &keys,
            "POST",
            "/_admin/brains/personal/folders",
            Some(body),
            TEST_NOW + 1,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(response).await;
        assert!(metadata.folders.iter().any(|folder| folder.id == "notes"));
    }

    #[tokio::test]
    async fn personal_brain_member_sees_and_writes_only_their_restricted_folder() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let member_keys = Keys::generate();
        let owner_npub = npub(&owner_keys);
        let agent_npub = npub(&agent_keys);
        let member_npub = npub(&member_keys);
        let router = personal_test_router(&owner_keys, &agent_keys);

        let all_members_folder_body = serde_json::json!({
            "folderId": "implicit-personal-share",
            "name": "Implicit Personal Share",
            "role": "folder",
            "access": "all_members",
            "parentFolderId": null,
            "path": "Implicit Personal Share",
            "sharedFolderSource": false,
            "accessUserIds": [],
            "grants": [
                folder_key_grant_value("grant-implicit-personal-member-v1", 1, member_npub.as_str())
            ],
            "accessChangeEvent": admin_event(
                &owner_keys,
                "personal",
                "change-reject-implicit-personal-share",
                AdminAccessAction::SetFolderAccessMode,
                Some("implicit-personal-share"),
                None,
                Some(1),
            ),
        })
        .to_string();
        let all_members_folder = authed_request(
            router.clone(),
            &owner_keys,
            "POST",
            "/_admin/brains/personal/folders",
            Some(all_members_folder_body),
            TEST_NOW,
        )
        .await;
        assert_error(
            all_members_folder,
            StatusCode::BAD_REQUEST,
            "Personal Brain shared access requires a restricted Folder",
        )
        .await;

        let create_folder_body = serde_json::json!({
            "folderId": "member-workspace",
            "name": "Member Workspace",
            "role": "folder",
            "access": "restricted",
            "parentFolderId": null,
            "path": "Member Workspace",
            "sharedFolderSource": false,
            "accessUserIds": [member_npub],
            "grants": [
                folder_key_grant_value("grant-member-workspace-owner-v1", 1, owner_npub.as_str()),
                folder_key_grant_value("grant-member-workspace-agent-v1", 1, agent_npub.as_str()),
                folder_key_grant_value("grant-member-workspace-member-v1", 1, member_npub.as_str())
            ],
            "accessChangeEvent": admin_event(
                &owner_keys,
                "personal",
                "change-create-member-workspace",
                AdminAccessAction::SetFolderAccessMode,
                Some("member-workspace"),
                None,
                Some(1),
            ),
        })
        .to_string();
        let create_folder = authed_request(
            router.clone(),
            &owner_keys,
            "POST",
            "/_admin/brains/personal/folders",
            Some(create_folder_body),
            TEST_NOW,
        )
        .await;
        if create_folder.status() != StatusCode::OK {
            let error: ApiErrorBody = read_json(create_folder).await;
            panic!("personal restricted Folder create failed: {}", error.error);
        }

        let list = authed_request(
            router.clone(),
            &member_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(list.status(), StatusCode::OK);
        let list: VisibleBrainsResponse = read_json(list).await;
        assert_eq!(list.brains.len(), 1);
        assert_eq!(list.brains[0].brain_id, "personal");
        assert_eq!(list.brains[0].role, "member");

        let metadata = get_metadata(router.clone(), &member_keys, "personal", TEST_NOW + 2).await;
        assert_eq!(metadata.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(metadata).await;
        assert_eq!(metadata.owner_user_id.as_deref(), Some(owner_npub.as_str()));
        assert_eq!(metadata.members, vec![member_npub.clone()]);
        assert!(metadata.admins.is_empty());
        assert_eq!(metadata.folders.len(), 1);
        assert_eq!(metadata.folders[0].id, "member-workspace");
        assert_eq!(
            metadata.folders[0].access_user_ids,
            vec![member_npub.clone()]
        );
        assert_eq!(metadata.grant_count, 1);

        let export = authed_request(
            router.clone(),
            &member_keys,
            "GET",
            "/_admin/brains/personal/export",
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(export.status(), StatusCode::OK);
        let export: EncryptedBrainExportResponse = read_json(export).await;
        assert_eq!(export.folders.len(), 1);
        assert_eq!(export.folders[0].id, "member-workspace");
        assert_eq!(export.key_grants.len(), 1);
        assert_eq!(export.key_grants[0].recipient_npub, member_npub);

        let object_path =
            "/_admin/brains/personal/folders/member-workspace/objects/obj_000000000901";
        let object_body = object_write_body(
            &member_keys,
            RevisionFixture {
                brain_id: "personal",
                folder_id: "member-workspace",
                object_id: "obj_000000000901",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                content: "member encrypted page",
                nonce: 90,
                record_type: false,
            },
        );
        let write = authed_request(
            router.clone(),
            &member_keys,
            "PUT",
            object_path,
            Some(object_body),
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(write.status(), StatusCode::OK);

        let owner_only = authed_request(
            router.clone(),
            &member_keys,
            "GET",
            "/_admin/brains/personal/folders/getting-started/objects/obj_000000000001",
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_error(owner_only, StatusCode::FORBIDDEN, "folder access required").await;

        let add_admin_body = serde_json::json!({
            "targetNpub": member_npub,
            "accessChangeEvent": admin_event(
                &member_keys,
                "personal",
                "change-member-add-admin",
                AdminAccessAction::AddAdmin,
                None,
                Some(member_npub.as_str()),
                None,
            ),
        })
        .to_string();
        let add_admin = authed_request(
            router,
            &member_keys,
            "POST",
            "/_admin/brains/personal/admins",
            Some(add_admin_body),
            TEST_NOW + 1,
        )
        .await;
        assert_error(
            add_admin,
            StatusCode::FORBIDDEN,
            "brain admin access required",
        )
        .await;
    }

    #[tokio::test]
    async fn account_bound_agent_bootstraps_without_supplying_an_owner_or_setup_ticket() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let competing_agent_keys = Keys::generate();
        let owner_key = NostrPublicKey::from_protocol(owner_keys.public_key());
        let owner_npub = owner_key.to_npub().unwrap();
        let agent_npub = npub(&agent_keys);
        let competing_agent_npub = npub(&competing_agent_keys);
        let agent_email = "cheater-a1b2c3d4e5f60708@finite.vip";
        let competing_agent_email = "other-a1b2c3d4e5f60708@finite.vip";
        let owner_email = "owner@finite.computer";
        let (identity_url, identity_server) = spawn_json_authority(vec![
            (
                "/api/v1/operator/brain/agent-resolution",
                serde_json::json!({
                    "agentNpub": agent_npub,
                    "managedAgentEmail": agent_email,
                }),
            ),
            (
                "/api/v1/operator/brain/user-resolution",
                serde_json::json!({
                    "workosUserId": "user_workos_owner",
                    "userNpub": owner_npub,
                }),
            ),
            (
                "/api/v1/operator/brain/agent-resolution",
                serde_json::json!({
                    "agentNpub": agent_npub,
                    "managedAgentEmail": agent_email,
                }),
            ),
            (
                "/api/v1/operator/brain/user-resolution",
                serde_json::json!({
                    "workosUserId": "user_workos_owner",
                    "userNpub": owner_npub,
                }),
            ),
            (
                "/api/v1/operator/brain/agent-resolution",
                serde_json::json!({
                    "agentNpub": competing_agent_npub,
                    "managedAgentEmail": competing_agent_email,
                }),
            ),
            (
                "/api/v1/operator/brain/user-resolution",
                serde_json::json!({
                    "workosUserId": "user_workos_owner",
                    "userNpub": owner_npub,
                }),
            ),
        ]);
        let (core_url, core_server) = spawn_json_authority(vec![
            (
                "/api/core/v1/brain/agent-account",
                serde_json::json!({
                    "workosUserId": "user_workos_owner",
                    "managedAgentEmail": agent_email,
                    "verifiedEmail": owner_email,
                    "status": "active",
                }),
            ),
            (
                "/api/core/v1/brain/agent-account",
                serde_json::json!({
                    "workosUserId": "user_workos_owner",
                    "managedAgentEmail": agent_email,
                    "verifiedEmail": owner_email,
                    "status": "active",
                }),
            ),
            (
                "/api/core/v1/brain/agent-account",
                serde_json::json!({
                    "workosUserId": "user_workos_owner",
                    "managedAgentEmail": competing_agent_email,
                    "verifiedEmail": owner_email,
                    "status": "active",
                }),
            ),
        ]);
        let router = router_with_state(test_state().with_agent_bootstrap_authorities(
            core_url,
            "core-token",
            identity_url,
            "identity-token",
        ));

        let response = authed_request(
            router.clone(),
            &agent_keys,
            "POST",
            "/_admin/personal-brain-bootstrap",
            Some("{}".to_owned()),
            TEST_NOW,
        )
        .await;
        if response.status() != StatusCode::OK {
            panic!(
                "unexpected bootstrap response: {}",
                read_text(response).await
            );
        }
        let response: BootstrapPersonalBrainForAgentResponse = read_json(response).await;
        assert_eq!(
            response.brain.owner_user_id.as_deref(),
            Some(owner_npub.as_str())
        );
        assert_eq!(
            response
                .brain
                .personal_agent
                .as_ref()
                .map(|relationship| relationship.agent_npub.as_str()),
            Some(agent_npub.as_str())
        );
        assert!(response.brain.folders.is_empty());
        assert_eq!(
            response.brain.brain_id,
            format!("personal-{}", &owner_key.to_hex()[..16])
        );
        assert!(
            response
                .brain
                .identities
                .iter()
                .any(|identity| { identity.npub == owner_npub && identity.display == owner_email })
        );

        let retry = authed_request(
            router.clone(),
            &agent_keys,
            "POST",
            "/_admin/personal-brain-bootstrap",
            Some("{}".to_owned()),
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(retry.status(), StatusCode::OK);
        let retry: BootstrapPersonalBrainForAgentResponse = read_json(retry).await;
        assert_eq!(retry.brain.brain_id, response.brain.brain_id);

        let competing = authed_request(
            router.clone(),
            &competing_agent_keys,
            "POST",
            "/_admin/personal-brain-bootstrap",
            Some("{}".to_owned()),
            TEST_NOW + 1,
        )
        .await;
        assert_error(
            competing,
            StatusCode::BAD_REQUEST,
            "personal brain already has a different personal agent",
        )
        .await;

        let owner_brains = authed_request(
            router,
            &owner_keys,
            "GET",
            "/_admin/brains",
            None,
            TEST_NOW + 1,
        )
        .await;
        let owner_brains: VisibleBrainsResponse = read_json(owner_brains).await;
        assert_eq!(owner_brains.brains.len(), 1);
        assert_eq!(owner_brains.brains[0].role, "owner");
        identity_server.join().unwrap();
        core_server.join().unwrap();
    }

    #[tokio::test]
    async fn agent_bootstrap_rejects_caller_selected_authority_and_missing_configuration() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let caller_selected_owner = authed_request(
            test_router(),
            &agent_keys,
            "POST",
            "/_admin/personal-brain-bootstrap",
            Some(serde_json::json!({ "ownerNpub": npub(&owner_keys) }).to_string()),
            TEST_NOW,
        )
        .await;
        assert_error(
            caller_selected_owner,
            StatusCode::BAD_REQUEST,
            "invalid JSON request body",
        )
        .await;

        let unconfigured = authed_request(
            test_router(),
            &agent_keys,
            "POST",
            "/_admin/personal-brain-bootstrap",
            Some("{}".to_owned()),
            TEST_NOW,
        )
        .await;
        assert_error(
            unconfigured,
            StatusCode::SERVICE_UNAVAILABLE,
            "Brain account-agent authority is not configured",
        )
        .await;
    }

    #[tokio::test]
    async fn protected_create_rejects_missing_auth() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_admin/brains")
                    .header("content-type", "application/json")
                    .body(Body::from(create_brain_body("acme", "organization")))
                    .expect("valid request"),
            )
            .await
            .expect("create response");

        assert_error(
            response,
            StatusCode::FORBIDDEN,
            "valid Nostr authorization is required",
        )
        .await;
    }

    #[tokio::test]
    async fn protected_create_accepts_compatible_nostr_auth_header_aliases() {
        for header_name in [NOSTR_AUTHORIZATION_HEADER, FINITEBRAIN_NOSTR_HEADER] {
            let keys = Keys::generate();
            let body = create_brain_body(header_name.replace('-', "_").as_str(), "organization");
            let response =
                post_brain_with_header(test_router(), &keys, &body, TEST_NOW, header_name).await;

            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn protected_create_rejects_oversized_request_body() {
        let body = "x".repeat(MAX_REQUEST_BODY_BYTES + 1);
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_admin/brains")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("valid request"),
            )
            .await
            .expect("oversized response");

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn signed_rotation_routes_reject_excessive_fanout_without_mutation() {
        let keys = Keys::generate();
        let actor = UserId::new(npub(&keys)).unwrap();
        let state = test_state();
        let router = router_with_state(state.clone());

        let rotations = (0..=MAX_PERSONAL_AGENT_ROTATION_FOLDERS)
            .map(|index| {
                serde_json::json!({
                    "folderId": format!("folder-{index}"),
                    "newKeyVersion": 2,
                    "grants": [],
                    "reencryptedRecords": [],
                    "accessChangeEvent": {},
                })
            })
            .collect::<Vec<_>>();
        let personal_body = serde_json::json!({
            "agentEmail": null,
            "rotations": rotations,
        })
        .to_string();
        let personal = authed_request(
            router.clone(),
            &keys,
            "PUT",
            "/_admin/brains/personal/personal-agent",
            Some(personal_body),
            TEST_NOW,
        )
        .await;
        assert_error(
            personal,
            StatusCode::PAYLOAD_TOO_LARGE,
            "Personal Agent rotation exceeds Folder rotations limit: 101 supplied, maximum 100",
        )
        .await;

        let grants = (0..=MAX_FOLDER_ACCESS_REMOVAL_GRANTS)
            .map(|index| {
                serde_json::json!({
                    "id": format!("grant-{index}"),
                    "keyVersion": 2,
                    "recipientNpub": actor.as_str(),
                    "wrappedEventJson": "{}",
                    "createdAt": "2026-06-23T00:00:00.000Z",
                })
            })
            .collect::<Vec<_>>();
        let access_body = serde_json::json!({
            "newKeyVersion": 2,
            "grants": grants,
            "reencryptedRecords": [],
            "accessChangeEvent": {},
        })
        .to_string();
        let access = authed_request(
            router,
            &keys,
            "DELETE",
            &format!(
                "/_admin/brains/personal/folders/notes/access/{}",
                actor.as_str()
            ),
            Some(access_body),
            TEST_NOW + 1,
        )
        .await;
        assert_error(
            access,
            StatusCode::PAYLOAD_TOO_LARGE,
            "Folder access removal exceeds grants per Folder rotation limit: 1001 supplied, maximum 1000",
        )
        .await;

        assert!(
            state
                .store
                .lock()
                .unwrap()
                .list_visible_brains(&actor)
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn protected_create_rejects_stale_wrong_method_wrong_url_and_wrong_payload_auth() {
        let keys = Keys::generate();
        let body = create_brain_body("acme", "organization");

        let stale = post_brain(
            test_router(),
            &keys,
            &body,
            TEST_NOW - 1_000,
            None,
            None,
            None,
        )
        .await;
        assert_error(stale, StatusCode::FORBIDDEN, "stale Nostr event timestamp").await;

        let wrong_method = post_brain(
            test_router(),
            &keys,
            &body,
            TEST_NOW,
            Some("GET"),
            None,
            None,
        )
        .await;
        assert_error(
            wrong_method,
            StatusCode::FORBIDDEN,
            "Nostr auth method mismatch",
        )
        .await;

        let wrong_url = post_brain(
            test_router(),
            &keys,
            &body,
            TEST_NOW,
            None,
            Some("/_admin/brains/acme/metadata"),
            None,
        )
        .await;
        assert_error(wrong_url, StatusCode::FORBIDDEN, "Nostr auth URL mismatch").await;

        let wrong_payload = post_brain(
            test_router(),
            &keys,
            &body,
            TEST_NOW,
            None,
            None,
            Some(br#"{"wrong":true}"#),
        )
        .await;
        assert_error(
            wrong_payload,
            StatusCode::FORBIDDEN,
            "Nostr auth payload mismatch",
        )
        .await;
    }

    #[tokio::test]
    async fn protected_routes_reject_replayed_auth_events() {
        let keys = Keys::generate();
        let body = create_brain_body("acme", "organization");
        let router = test_router();

        let first = post_brain(router.clone(), &keys, &body, TEST_NOW, None, None, None).await;
        assert_eq!(first.status(), StatusCode::OK);

        let replay = post_brain(router, &keys, &body, TEST_NOW, None, None, None).await;
        assert_error(
            replay,
            StatusCode::FORBIDDEN,
            "replayed Nostr authorization event",
        )
        .await;
    }

    #[tokio::test]
    async fn protected_routes_enforce_configured_rate_limits() {
        let keys = Keys::generate();
        let router = router_with_state(test_state().with_rate_limit(1, 60));

        let first_body = create_brain_body("acme", "organization");
        let first = post_brain(
            router.clone(),
            &keys,
            &first_body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(first.status(), StatusCode::OK);

        let second_body = create_brain_body("beta", "organization");
        let second = post_brain(router, &keys, &second_body, TEST_NOW + 1, None, None, None).await;
        assert_error(
            second,
            StatusCode::TOO_MANY_REQUESTS,
            "protected route rate limit exceeded",
        )
        .await;
    }

    #[tokio::test]
    async fn cors_preflight_is_allowlist_driven() {
        let state =
            test_state().with_cors_allowed_origins(["https://client.finite.test".to_owned()]);
        let router = router_with_state(state);

        let allowed = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/_admin/brains")
                    .header(ORIGIN, "https://client.finite.test")
                    .header("access-control-request-method", "POST")
                    .body(Body::empty())
                    .expect("valid CORS preflight"),
            )
            .await
            .expect("allowed preflight response");
        assert_eq!(allowed.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            allowed
                .headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("https://client.finite.test")
        );
        assert_eq!(
            allowed
                .headers()
                .get(ACCESS_CONTROL_ALLOW_METHODS)
                .and_then(|value| value.to_str().ok()),
            Some("GET,POST,PUT,DELETE,PATCH,OPTIONS")
        );

        let blocked = router
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/_admin/brains")
                    .header(ORIGIN, "https://evil.example")
                    .header("access-control-request-method", "POST")
                    .body(Body::empty())
                    .expect("valid CORS preflight"),
            )
            .await
            .expect("blocked preflight response");
        assert_error(blocked, StatusCode::FORBIDDEN, "CORS origin is not allowed").await;
    }

    #[tokio::test]
    async fn invalid_bootstrap_maps_to_bad_request_after_valid_auth() {
        let keys = Keys::generate();
        let body = create_brain_body("", "organization");
        let response = post_brain(test_router(), &keys, &body, TEST_NOW, None, None, None).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn metadata_requires_brain_membership() {
        let admin_keys = Keys::generate();
        let outsider_keys = Keys::generate();
        let router = test_router();
        let body = create_brain_body("acme", "organization");
        let create = post_brain(
            router.clone(),
            &admin_keys,
            &body,
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create.status(), StatusCode::OK);

        let response = get_metadata(router, &outsider_keys, "acme", TEST_NOW).await;
        assert_error(response, StatusCode::FORBIDDEN, "brain access required").await;
    }

    #[tokio::test]
    async fn secure_object_routes_create_update_delete_and_pull_sync() {
        let keys = Keys::generate();
        let router = test_router();
        let create_brain = post_brain(
            router.clone(),
            &keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_brain.status(), StatusCode::OK);
        add_test_org_folders(&router, &keys).await;
        let setup_sequence = latest_sync_sequence(&router, &keys, "acme").await;

        let object_path = "/_admin/brains/acme/folders/getting-started/objects/obj_000000000001";
        let create_body = object_write_body(
            &keys,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                content: "created page",
                nonce: 1,
                record_type: false,
            },
        );
        let create = authed_request(
            router.clone(),
            &keys,
            "PUT",
            object_path,
            Some(create_body.clone()),
            TEST_NOW,
        )
        .await;
        assert_eq!(create.status(), StatusCode::OK);
        let create: ObjectWriteResponse = read_json(create).await;
        assert_eq!(create.sequence, setup_sequence + 1);
        assert!(!create.duplicate);
        assert_eq!(create.revision, 1);

        let get = authed_request(router.clone(), &keys, "GET", object_path, None, TEST_NOW).await;
        assert_eq!(get.status(), StatusCode::OK);
        let current: ObjectResponse = read_json(get).await;
        assert_eq!(current.revision, 1);
        assert!(!current.deleted);
        assert!(current.ciphertext.contains("\"cipher\":\"AES-256-GCM\""));

        let update_body = object_write_body(
            &keys,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Update,
                revision: 2,
                base_revision: Some(1),
                key_version: 1,
                content: "updated page",
                nonce: 2,
                record_type: true,
            },
        );
        let update = authed_request(
            router.clone(),
            &keys,
            "POST",
            "/_admin/brains/acme/sync/records",
            Some(update_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(update.status(), StatusCode::OK);
        let update: ObjectWriteResponse = read_json(update).await;
        assert_eq!(update.sequence, setup_sequence + 2);
        assert_eq!(update.revision, 2);

        let bootstrap = authed_request(
            router.clone(),
            &keys,
            "GET",
            "/_admin/brains/acme/sync/bootstrap",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(bootstrap.status(), StatusCode::OK);
        let bootstrap: SyncBootstrapResponse = read_json(bootstrap).await;
        assert_eq!(bootstrap.latest_sequence, setup_sequence + 2);
        assert_eq!(bootstrap.object_count, 1);
        assert_eq!(bootstrap.objects[0].revision, 2);

        let first_pull = authed_request(
            router.clone(),
            &keys,
            "GET",
            &format!("/_admin/brains/acme/sync/records?after={setup_sequence}&limit=1"),
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(first_pull.status(), StatusCode::OK);
        let first_pull: SyncPullResponse = read_json(first_pull).await;
        assert_eq!(first_pull.count, 1);
        assert!(first_pull.has_more);
        assert_eq!(first_pull.next_sequence, setup_sequence + 1);
        assert_eq!(first_pull.records[0].record_type, "folder_object_revision");
        assert!(
            first_pull.records[0]
                .payload_json
                .contains("\"revisionEvent\"")
        );
        assert!(
            first_pull.records[0]
                .payload_json
                .contains("\"ciphertext\"")
        );

        let second_pull = authed_request(
            router.clone(),
            &keys,
            "GET",
            &format!(
                "/_admin/brains/acme/sync/records?after={}&limit=10",
                setup_sequence + 1
            ),
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(second_pull.status(), StatusCode::OK);
        let second_pull: SyncPullResponse = read_json(second_pull).await;
        assert_eq!(second_pull.count, 1);
        assert!(!second_pull.has_more);
        assert_eq!(second_pull.records[0].revision, Some(2));
        assert!(
            second_pull.records[0]
                .payload_json
                .contains("\"revisionEvent\"")
        );

        let move_body = object_write_body(
            &keys,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Move,
                revision: 3,
                base_revision: Some(2),
                key_version: 1,
                content: "moved page",
                nonce: 11,
                record_type: false,
            },
        );
        let move_object = authed_request(
            router.clone(),
            &keys,
            "POST",
            "/_admin/brains/acme/folders/getting-started/objects/obj_000000000001/move",
            Some(move_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(move_object.status(), StatusCode::OK);
        let move_object: ObjectWriteResponse = read_json(move_object).await;
        assert_eq!(move_object.sequence, setup_sequence + 3);
        assert_eq!(move_object.revision, 3);

        let delete_body = object_delete_body(
            &keys,
            TombstoneFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000001",
                revision: 4,
                base_revision: 3,
                record_type: false,
            },
        );
        let delete = authed_request(
            router.clone(),
            &keys,
            "DELETE",
            object_path,
            Some(delete_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(delete.status(), StatusCode::OK);
        let delete: ObjectWriteResponse = read_json(delete).await;
        assert_eq!(delete.sequence, setup_sequence + 4);
        assert_eq!(delete.revision, 4);

        let get_deleted = authed_request(
            router.clone(),
            &keys,
            "GET",
            object_path,
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_error(get_deleted, StatusCode::NOT_FOUND, "object not found").await;

        let bootstrap = authed_request(
            router.clone(),
            &keys,
            "GET",
            "/_admin/brains/acme/sync/bootstrap",
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(bootstrap.status(), StatusCode::OK);
        let bootstrap: SyncBootstrapResponse = read_json(bootstrap).await;
        assert_eq!(bootstrap.latest_sequence, setup_sequence + 4);
        assert!(bootstrap.objects[0].deleted);
        let tombstone_pull = authed_request(
            router,
            &keys,
            "GET",
            &format!(
                "/_admin/brains/acme/sync/records?after={}&limit=10",
                setup_sequence + 3
            ),
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(tombstone_pull.status(), StatusCode::OK);
        let tombstone_pull: SyncPullResponse = read_json(tombstone_pull).await;
        assert_eq!(
            tombstone_pull.records[0].record_type,
            "folder_object_tombstone"
        );
        assert!(
            tombstone_pull.records[0]
                .payload_json
                .contains("\"tombstoneEvent\"")
        );
    }

    #[tokio::test]
    async fn encrypted_export_route_filters_opaque_objects_and_search_stays_client_side() {
        let admin_keys = Keys::generate();
        let member_keys = Keys::generate();
        let admin_npub = npub(&admin_keys);
        let member_npub = npub(&member_keys);
        let brain_id = BrainId::new("acme").unwrap();
        let mut store = BrainStore::open_in_memory().unwrap();
        let output = bootstrap_organization_brain("acme", "Acme", &admin_npub).unwrap();
        let grants = grants_for_required(&output.required_key_grants, &brain_id, &admin_npub);
        store.create_brain_bootstrap(&output, &grants).unwrap();
        store
            .add_member(&brain_id, &UserId::new(member_npub.clone()).unwrap())
            .unwrap();
        store
            .create_folder(
                &brain_id,
                &Folder {
                    id: FolderId::new("getting-started").unwrap(),
                    name: DisplayName::new("folder_name", "Getting Started").unwrap(),
                    role: FolderRole::General,
                    access: FolderAccessMode::AllMembers,
                    parent_folder_id: None,
                    path: SafeRelativePath::new("folder_path", "getting-started").unwrap(),
                    current_key_version: 1,
                    shared_folder_source: false,
                },
                &BTreeSet::new(),
                &[
                    FolderKeyGrantMetadata {
                        id: "grant-getting-started-admin".to_owned(),
                        folder_id: FolderId::new("getting-started").unwrap(),
                        key_version: 1,
                        issuer_npub: UserId::new(admin_npub.clone()).unwrap(),
                        recipient_npub: UserId::new(admin_npub.clone()).unwrap(),
                        format: "NIP-59".to_owned(),
                        wrapped_event_json: "{\"kind\":1059}".to_owned(),
                        access_change_event_json: Some("{\"kind\":30078}".to_owned()),
                        created_at: "2026-06-23T00:00:00.000Z".to_owned(),
                    },
                    FolderKeyGrantMetadata {
                        id: "grant-getting-started-member".to_owned(),
                        folder_id: FolderId::new("getting-started").unwrap(),
                        key_version: 1,
                        issuer_npub: UserId::new(admin_npub.clone()).unwrap(),
                        recipient_npub: UserId::new(member_npub.clone()).unwrap(),
                        format: "NIP-59".to_owned(),
                        wrapped_event_json: "{\"kind\":1059}".to_owned(),
                        access_change_event_json: Some("{\"kind\":30078}".to_owned()),
                        created_at: "2026-06-23T00:00:00.000Z".to_owned(),
                    },
                ],
            )
            .unwrap();
        store
            .create_folder(
                &brain_id,
                &Folder {
                    id: FolderId::new("strategy").unwrap(),
                    name: DisplayName::new("folder_name", "Strategy").unwrap(),
                    role: FolderRole::Folder,
                    access: FolderAccessMode::Restricted,
                    parent_folder_id: None,
                    path: SafeRelativePath::new("folder_path", "Strategy").unwrap(),
                    current_key_version: 1,
                    shared_folder_source: false,
                },
                &BTreeSet::new(),
                &[FolderKeyGrantMetadata {
                    id: "grant-strategy-admin".to_owned(),
                    folder_id: FolderId::new("strategy").unwrap(),
                    key_version: 1,
                    issuer_npub: UserId::new(admin_npub.clone()).unwrap(),
                    recipient_npub: UserId::new(admin_npub.clone()).unwrap(),
                    format: "NIP-59".to_owned(),
                    wrapped_event_json: "{\"kind\":1059}".to_owned(),
                    access_change_event_json: Some("{\"kind\":30078}".to_owned()),
                    created_at: "2026-06-23T00:00:00.000Z".to_owned(),
                }],
            )
            .unwrap();
        for (folder_id, object_id, body) in [
            (
                "getting-started",
                "obj_000000000201",
                "getting-started encrypted payload",
            ),
            ("strategy", "obj_000000000202", "secret encrypted payload"),
        ] {
            store
                .submit_sync_record(
                    &brain_id,
                    &SyncRecordInput::FolderObjectRevision(FolderObjectRevisionSyncRecord {
                        record_event_id: format!("event-{folder_id}"),
                        folder_id: FolderId::new(folder_id).unwrap(),
                        object_id: ObjectId::new(object_id).unwrap(),
                        revision: 1,
                        base_revision: None,
                        actor_npub: UserId::new(admin_npub.clone()).unwrap(),
                        client_created_at: "2026-06-23T00:00:00.000Z".to_owned(),
                        payload_json: format!("{{\"body\":\"{body}\"}}"),
                        record_event_kind: APP_SPECIFIC_KIND,
                    }),
                )
                .unwrap();
        }

        let router =
            router_with_state(ServerState::new(store, TEST_BASE_URL).with_auth_clock(TEST_NOW, 60));
        let export = authed_request(
            router.clone(),
            &member_keys,
            "GET",
            "/_admin/brains/acme/export",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(export.status(), StatusCode::OK);
        let export: EncryptedBrainExportResponse = read_json(export).await;
        assert_eq!(export.version, "finite-brain-export-v1");
        let getting_started = export
            .objects
            .iter()
            .find(|object| object.folder_id == "getting-started")
            .unwrap();
        assert!(!getting_started.opaque);
        assert!(
            getting_started
                .payload_json
                .as_ref()
                .unwrap()
                .contains("getting-started")
        );
        let strategy = export
            .objects
            .iter()
            .find(|object| object.folder_id == "strategy")
            .unwrap();
        assert!(strategy.opaque);
        assert!(strategy.payload_json.is_none());

        let search = authed_request(
            router,
            &member_keys,
            "GET",
            "/_admin/brains/acme/search?q=secret",
            None,
            TEST_NOW,
        )
        .await;
        assert_error(
            search,
            StatusCode::BAD_REQUEST,
            "plaintext search is client-side only",
        )
        .await;
    }

    #[tokio::test]
    async fn object_write_duplicate_retry_returns_original_sequence() {
        let keys = Keys::generate();
        let router = router_with_test_org_folders(&keys).await;
        let setup_sequence = latest_sync_sequence(&router, &keys, "acme").await;
        let path = "/_admin/brains/acme/folders/getting-started/objects/obj_000000000001";
        let body = object_write_body(
            &keys,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                content: "created page",
                nonce: 3,
                record_type: false,
            },
        );

        let first = authed_request(
            router.clone(),
            &keys,
            "PUT",
            path,
            Some(body.clone()),
            TEST_NOW,
        )
        .await;
        assert_eq!(first.status(), StatusCode::OK);
        let first: ObjectWriteResponse = read_json(first).await;
        assert_eq!(first.sequence, setup_sequence + 1);
        assert!(!first.duplicate);

        let retry = authed_request(router, &keys, "PUT", path, Some(body), TEST_NOW + 1).await;
        assert_eq!(retry.status(), StatusCode::OK);
        let retry: ObjectWriteResponse = read_json(retry).await;
        assert_eq!(retry.sequence, setup_sequence + 1);
        assert!(retry.duplicate);
    }

    #[tokio::test]
    async fn object_write_rejects_stale_base_bad_ciphertext_hash_and_signer_mismatch() {
        let keys = Keys::generate();
        let router = router_with_test_org_folders(&keys).await;
        let path = "/_admin/brains/acme/folders/getting-started/objects/obj_000000000001";
        let create_body = object_write_body(
            &keys,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                content: "created page",
                nonce: 4,
                record_type: false,
            },
        );
        let create = authed_request(
            router.clone(),
            &keys,
            "PUT",
            path,
            Some(create_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create.status(), StatusCode::OK);

        let update_body = object_write_body(
            &keys,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Update,
                revision: 2,
                base_revision: Some(1),
                key_version: 1,
                content: "updated page",
                nonce: 5,
                record_type: false,
            },
        );
        let update = authed_request(
            router.clone(),
            &keys,
            "PUT",
            path,
            Some(update_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(update.status(), StatusCode::OK);

        let stale_body = object_write_body(
            &keys,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Update,
                revision: 2,
                base_revision: Some(1),
                key_version: 1,
                content: "stale update",
                nonce: 6,
                record_type: false,
            },
        );
        let stale = authed_request(
            router.clone(),
            &keys,
            "PUT",
            path,
            Some(stale_body),
            TEST_NOW,
        )
        .await;
        assert_error(stale, StatusCode::CONFLICT, "baseRevision does not match").await;

        let good_envelope = object_envelope_json(
            "acme",
            "getting-started",
            "obj_000000000002",
            1,
            "good content",
            7,
        );
        let bad_envelope = object_envelope_json(
            "acme",
            "getting-started",
            "obj_000000000002",
            1,
            "bad content",
            8,
        );
        let event = revision_event_for_author(
            &keys,
            npub(&keys),
            RevisionEventFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000002",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                envelope_json: good_envelope,
            },
        );
        let bad_hash_body = serde_json::json!({
            "baseRevision": null,
            "keyVersion": 1,
            "cipher": "AES-256-GCM",
            "ciphertext": bad_envelope,
            "revisionEvent": event,
        })
        .to_string();
        let bad_hash = authed_request(
            router.clone(),
            &keys,
            "PUT",
            "/_admin/brains/acme/folders/getting-started/objects/obj_000000000002",
            Some(bad_hash_body),
            TEST_NOW,
        )
        .await;
        assert_error(
            bad_hash,
            StatusCode::BAD_REQUEST,
            "ciphertext hash mismatch",
        )
        .await;

        let signer_keys = Keys::generate();
        let envelope = object_envelope_json(
            "acme",
            "getting-started",
            "obj_000000000003",
            1,
            "signer mismatch",
            9,
        );
        let event = revision_event_for_author(
            &signer_keys,
            npub(&keys),
            RevisionEventFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000003",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                envelope_json: envelope.clone(),
            },
        );
        let signer_mismatch_body = serde_json::json!({
            "baseRevision": null,
            "keyVersion": 1,
            "cipher": "AES-256-GCM",
            "ciphertext": envelope,
            "revisionEvent": event,
        })
        .to_string();
        let signer_mismatch = authed_request(
            router,
            &keys,
            "PUT",
            "/_admin/brains/acme/folders/getting-started/objects/obj_000000000003",
            Some(signer_mismatch_body),
            TEST_NOW,
        )
        .await;
        assert_error(signer_mismatch, StatusCode::BAD_REQUEST, "signer mismatch").await;
    }

    #[tokio::test]
    async fn sync_pull_expired_cursor_requires_rebootstrap() {
        let keys = Keys::generate();
        let state = test_state();
        let router = router_with_state(state.clone());
        let create_brain = post_brain(
            router.clone(),
            &keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_brain.status(), StatusCode::OK);
        add_test_org_folders(&router, &keys).await;
        let path = "/_admin/brains/acme/folders/getting-started/objects/obj_000000000001";
        let body = object_write_body(
            &keys,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "getting-started",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                content: "created page",
                nonce: 10,
                record_type: false,
            },
        );
        let create = authed_request(router.clone(), &keys, "PUT", path, Some(body), TEST_NOW).await;
        assert_eq!(create.status(), StatusCode::OK);

        {
            let mut store = state.store.lock().unwrap();
            store
                .set_retention_floor(&BrainId::new("acme").unwrap(), 1)
                .unwrap();
        }

        let expired = authed_request(
            router,
            &keys,
            "GET",
            "/_admin/brains/acme/sync/records?after=0&limit=10",
            None,
            TEST_NOW,
        )
        .await;
        assert_error(expired, StatusCode::GONE, "rebootstrap required").await;
    }

    #[tokio::test]
    async fn concurrent_current_folder_grants_have_one_winner_and_one_truthful_no_op() {
        let admin_keys = Keys::generate();
        let member_keys = Keys::generate();
        let member_npub = npub(&member_keys);
        let router = router_with_test_org_folders(&admin_keys).await;

        let add_member_body = serde_json::json!({
            "targetNpub": member_npub,
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "change-add-concurrent-member",
                AdminAccessAction::AddMember,
                None,
                Some(member_npub.as_str()),
                None,
            ),
        })
        .to_string();
        let add_member = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/members",
            Some(add_member_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(add_member.status(), StatusCode::OK);
        let before_sequence = latest_sync_sequence(&router, &admin_keys, "acme").await;

        let grant_body = |suffix: &str| {
            serde_json::json!({
                "targetNpub": member_npub,
                "grant": folder_key_grant_value(
                    &format!("grant-restricted-member-{suffix}"),
                    1,
                    member_npub.as_str(),
                ),
                "accessChangeEvent": admin_event(
                    &admin_keys,
                    "acme",
                    &format!("change-grant-restricted-member-{suffix}"),
                    AdminAccessAction::GrantFolderAccess,
                    Some("restricted"),
                    Some(member_npub.as_str()),
                    Some(1),
                ),
            })
            .to_string()
        };
        let first_body = grant_body("first");
        let second_body = grant_body("second");
        let first = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/folders/restricted/access",
            Some(first_body),
            TEST_NOW,
        );
        let second = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/folders/restricted/access",
            Some(second_body),
            TEST_NOW,
        );
        let (first, second) = tokio::join!(first, second);

        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(second.status(), StatusCode::OK);
        let first: serde_json::Value = read_json(first).await;
        let second: serde_json::Value = read_json(second).await;
        let mut outcomes = vec![
            first["outcome"].as_str().unwrap().to_owned(),
            second["outcome"].as_str().unwrap().to_owned(),
        ];
        outcomes.sort();
        assert_eq!(outcomes, ["alreadyHasAccess", "granted"]);

        let metadata = if first["outcome"] == "granted" {
            first
        } else {
            second
        };
        assert_eq!(metadata["grantCount"], 3);
        assert_eq!(
            metadata["folders"]
                .as_array()
                .unwrap()
                .iter()
                .find(|folder| folder["id"] == "restricted")
                .unwrap()["accessUserIds"],
            serde_json::json!([member_npub])
        );
        let bootstrap = authed_request(
            router.clone(),
            &admin_keys,
            "GET",
            "/_admin/brains/acme/sync/bootstrap",
            None,
            TEST_NOW - 2,
        )
        .await;
        assert_eq!(bootstrap.status(), StatusCode::OK);
        let bootstrap: SyncBootstrapResponse = read_json(bootstrap).await;
        assert_eq!(bootstrap.latest_sequence, before_sequence + 2);
        let new_record_types = bootstrap
            .control_records
            .iter()
            .filter(|record| record.sequence > before_sequence)
            .map(|record| record.record_type.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            new_record_types,
            ["folder_key_grant", "brain_admin_access_change"]
        );
    }

    #[tokio::test]
    async fn admin_routes_create_restricted_folder_and_rotate_access_removal() {
        let admin_keys = Keys::generate();
        let member_keys = Keys::generate();
        let member_npub = npub(&member_keys);
        let router = router_with_test_org_folders(&admin_keys).await;

        let add_member_body = serde_json::json!({
            "targetNpub": member_npub,
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "change_add_member",
                AdminAccessAction::AddMember,
                None,
                Some(member_npub.as_str()),
                None,
            ),
        })
        .to_string();
        let add_member = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/members",
            Some(add_member_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(add_member.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(add_member).await;
        assert!(metadata.members.contains(&member_npub));

        let create_folder_body = serde_json::json!({
            "folderId": "strategy",
            "name": "Strategy",
            "role": "folder",
            "access": "restricted",
            "parentFolderId": null,
            "path": "Strategy",
            "accessUserIds": [member_npub],
            "grants": [
                folder_key_grant_value("grant-strategy-admin-v1", 1, npub(&admin_keys).as_str()),
                folder_key_grant_value("grant-strategy-member-v1", 1, member_npub.as_str())
            ],
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "change_create_strategy",
                AdminAccessAction::SetFolderAccessMode,
                Some("strategy"),
                None,
                Some(1),
            ),
        })
        .to_string();
        let create_folder = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/folders",
            Some(create_folder_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create_folder.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(create_folder).await;
        let strategy = metadata
            .folders
            .iter()
            .find(|folder| folder.id == "strategy")
            .expect("strategy folder metadata");
        assert_eq!(strategy.current_key_version, 1);
        assert_eq!(strategy.access_user_ids, vec![member_npub.clone()]);

        let admin_bootstrap = authed_request(
            router.clone(),
            &admin_keys,
            "GET",
            "/_admin/brains/acme/sync/bootstrap",
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(admin_bootstrap.status(), StatusCode::OK);
        let admin_bootstrap: SyncBootstrapResponse = read_json(admin_bootstrap).await;
        assert!(
            admin_bootstrap
                .control_records
                .iter()
                .filter(|record| record.record_type == "folder_key_grant")
                .count()
                >= 2
        );
        assert!(
            admin_bootstrap
                .control_records
                .iter()
                .any(|record| record.record_type == "brain_admin_access_change")
        );

        let member_pull = authed_request(
            router.clone(),
            &member_keys,
            "GET",
            "/_admin/brains/acme/sync/records?after=0&limit=20",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(member_pull.status(), StatusCode::OK);
        let member_pull: SyncPullResponse = read_json(member_pull).await;
        assert!(
            member_pull
                .records
                .iter()
                .all(|record| record.record_type != "brain_admin_access_change")
        );
        assert!(member_pull.records.iter().any(|record| {
            record.record_type == "folder_key_grant"
                && record.payload_json.contains(member_npub.as_str())
        }));

        let object_path = "/_admin/brains/acme/folders/strategy/objects/obj_000000000001";
        let create_object_body = object_write_body(
            &admin_keys,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "strategy",
                object_id: "obj_000000000001",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                content: "restricted page",
                nonce: 12,
                record_type: false,
            },
        );
        let create_object = authed_request(
            router.clone(),
            &admin_keys,
            "PUT",
            object_path,
            Some(create_object_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create_object.status(), StatusCode::OK);

        let remove_access_body = serde_json::json!({
            "newKeyVersion": 2,
            "grants": [
                folder_key_grant_value("grant-strategy-admin-v2", 2, npub(&admin_keys).as_str())
            ],
            "reencryptedRecords": [
                rotation_object_value(
                    &admin_keys,
                    "acme",
                    "strategy",
                    "obj_000000000001",
                    2,
                    Some(1),
                    2,
                    "reencrypted restricted page",
                    13,
                )
            ],
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "change_remove_strategy_access",
                AdminAccessAction::RemoveFolderAccess,
                Some("strategy"),
                Some(member_npub.as_str()),
                Some(2),
            ),
        })
        .to_string();
        let remove_access = authed_request(
            router.clone(),
            &admin_keys,
            "DELETE",
            &format!("/_admin/brains/acme/folders/strategy/access/{member_npub}"),
            Some(remove_access_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(remove_access.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(remove_access).await;
        let strategy = metadata
            .folders
            .iter()
            .find(|folder| folder.id == "strategy")
            .expect("strategy folder metadata");
        assert_eq!(strategy.current_key_version, 2);
        assert!(strategy.access_user_ids.is_empty());

        let bootstrap = authed_request(
            router,
            &admin_keys,
            "GET",
            "/_admin/brains/acme/sync/bootstrap",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(bootstrap.status(), StatusCode::OK);
        let bootstrap: SyncBootstrapResponse = read_json(bootstrap).await;
        let object = bootstrap
            .objects
            .iter()
            .find(|object| object.object_id == "obj_000000000001")
            .expect("current object");
        assert_eq!(object.revision, 2);
    }

    #[tokio::test]
    async fn organization_collaboration_rejects_legacy_alias_and_oversized_snapshot() {
        let admin_keys = Keys::generate();
        let router = router_with_test_org_folders(&admin_keys).await;

        let alias = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaboration/ensure-admin",
            Some("{}".to_owned()),
            TEST_NOW,
        )
        .await;
        assert_eq!(alias.status(), StatusCode::NOT_FOUND);

        let folders = (0..=MAX_COLLABORATION_FOLDERS)
            .map(|index| {
                serde_json::json!({
                    "folderId": format!("folder-{index}"),
                    "keyVersion": 1,
                    "path": format!("Folder {index}")
                })
            })
            .collect::<Vec<_>>();
        let oversized = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(
                serde_json::json!({
                    "targetNpub": npub(&Keys::generate()),
                    "folders": folders,
                    "grants": [],
                    "accessChangeEvent": {}
                })
                .to_string(),
            ),
            TEST_NOW,
        )
        .await;
        assert_eq!(oversized.status(), StatusCode::BAD_REQUEST);
        let body = read_text(oversized).await;
        assert!(body.contains("exceeds 1000 entries"), "{body}");
    }

    #[tokio::test]
    async fn signed_organization_collaboration_is_complete_idempotent_and_partial_safe() {
        let admin_keys = Keys::generate();
        let target_keys = Keys::generate();
        let target = npub(&target_keys);
        let router = router_with_test_org_folders(&admin_keys).await;
        let member_keys = Keys::generate();
        let member = npub(&member_keys);
        let add_member = serde_json::json!({
            "targetNpub": member,
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "collab-readiness-member",
                AdminAccessAction::AddMember,
                None,
                Some(&member),
                None,
            ),
        })
        .to_string();
        let add_member = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/members",
            Some(add_member),
            TEST_NOW,
        )
        .await;
        assert_eq!(add_member.status(), StatusCode::OK);
        let member_metadata = get_metadata(router.clone(), &member_keys, "acme", TEST_NOW).await;
        assert_eq!(member_metadata.status(), StatusCode::OK);
        let member_metadata: BrainMetadataResponse = read_json(member_metadata).await;
        assert!(
            member_metadata.collaborator_readiness.is_empty(),
            "non-admin metadata must not expose collaborator grant relationships"
        );
        let folders = serde_json::json!([
            {"folderId":"getting-started","keyVersion":1,"path":"getting-started"},
            {"folderId":"restricted","keyVersion":1,"path":"restricted"}
        ]);
        let body = serde_json::json!({
            "targetNpub": target,
            "folders": folders,
            "grants": [
                {"folderId":"getting-started", "id":"collab-getting-started", "keyVersion":1,
                 "recipientNpub":target, "wrappedEventJson":gift_wrap_event_json(&target),
                 "createdAt":"2026-06-23T00:00:00.000Z", "accessChangeEvent":admin_event(&admin_keys,"acme","collab-getting-started",AdminAccessAction::GrantFolderAccess,Some("getting-started"),Some(&target),Some(1))},
                {"folderId":"restricted", "id":"collab-restricted", "keyVersion":1,
                 "recipientNpub":target, "wrappedEventJson":gift_wrap_event_json(&target),
                 "createdAt":"2026-06-23T00:00:00.000Z", "accessChangeEvent":admin_event(&admin_keys,"acme","collab-restricted",AdminAccessAction::GrantFolderAccess,Some("restricted"),Some(&target),Some(1))}
            ],
            "accessChangeEvent":admin_event(&admin_keys,"acme","collab-admin",AdminAccessAction::AddAdmin,None,Some(&target),None)
        }).to_string();
        let first = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(body.clone()),
            TEST_NOW,
        )
        .await;
        let first_status = first.status();
        let first_text = read_text(first).await;
        assert_eq!(first_status, StatusCode::OK, "{first_text}");
        assert!(!first_text.contains("encrypted grant placeholder"));
        assert!(!first_text.contains("wrappedEventJson"));
        assert!(!first_text.contains("folderKey"));
        assert!(!first_text.contains("secretKey"));
        let first: EnsureOrganizationAdminResponse = serde_json::from_str(&first_text).unwrap();
        assert_eq!(first.state, CollaborationReceiptState::Complete);
        assert_eq!(first.ready_count, 2);
        let sequence = latest_sync_sequence(&router, &admin_keys, "acme").await;
        let retry = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(body),
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(retry.status(), StatusCode::OK);
        let retry: EnsureOrganizationAdminResponse = read_json(retry).await;
        assert_eq!(retry.state, CollaborationReceiptState::Complete);
        assert!(
            retry
                .folders
                .iter()
                .all(|folder| folder.outcome == CollaborationFolderOutcome::AlreadyReady)
        );
        assert_eq!(
            latest_sync_sequence_at(&router, &admin_keys, "acme", TEST_NOW + 2).await,
            sequence
        );

        let partial_target = npub(&Keys::generate());
        let partial_body = serde_json::json!({
            "targetNpub": partial_target,
            "folders": folders,
            "grants": [],
            "accessChangeEvent": admin_event(&admin_keys,"acme","collab-partial",AdminAccessAction::AddAdmin,None,Some(&partial_target),None)
        }).to_string();
        let partial = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(partial_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(partial.status(), StatusCode::OK);
        let partial: EnsureOrganizationAdminResponse = read_json(partial).await;
        assert_eq!(partial.state, CollaborationReceiptState::Partial);
        assert!(
            partial
                .folders
                .iter()
                .any(|folder| folder.reason.as_deref() == Some("sourceKeyUnavailable"))
        );
        let metadata = get_metadata(router.clone(), &admin_keys, "acme", TEST_NOW + 1).await;
        assert_eq!(metadata.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(metadata).await;
        let incomplete_admin = metadata
            .collaborator_readiness
            .iter()
            .find(|entry| entry.target_npub == partial_target)
            .expect("partial administrator readiness");
        assert_eq!(incomplete_admin.brain_role, "admin");
        assert_eq!(incomplete_admin.ready_count, 0);
        assert_eq!(incomplete_admin.total_count, 2);

        let repair_body = serde_json::json!({
            "targetNpub": partial_target,
            "folders": folders,
            "grants": [
                {"folderId":"getting-started", "id":"collab-repair-getting-started", "keyVersion":1,
                 "recipientNpub":partial_target, "wrappedEventJson":gift_wrap_event_json(&partial_target),
                 "createdAt":"2026-06-23T00:00:00.000Z", "accessChangeEvent":admin_event(&admin_keys,"acme","collab-repair-getting-started",AdminAccessAction::GrantFolderAccess,Some("getting-started"),Some(&partial_target),Some(1))},
                {"folderId":"restricted", "id":"collab-repair-restricted", "keyVersion":1,
                 "recipientNpub":partial_target, "wrappedEventJson":gift_wrap_event_json(&partial_target),
                 "createdAt":"2026-06-23T00:00:00.000Z", "accessChangeEvent":admin_event(&admin_keys,"acme","collab-repair-restricted",AdminAccessAction::GrantFolderAccess,Some("restricted"),Some(&partial_target),Some(1))}
            ],
            "accessChangeEvent":admin_event(&admin_keys,"acme","collab-repair-admin",AdminAccessAction::AddAdmin,None,Some(&partial_target),None)
        }).to_string();
        let repair = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(repair_body),
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(repair.status(), StatusCode::OK);
        let repair: EnsureOrganizationAdminResponse = read_json(repair).await;
        assert_eq!(repair.state, CollaborationReceiptState::Complete);
        assert_eq!(repair.brain_role, "admin");
        assert_eq!(repair.ready_count, 2);

        let ready_metadata = get_metadata(router.clone(), &admin_keys, "acme", TEST_NOW + 2).await;
        assert_eq!(ready_metadata.status(), StatusCode::OK);
        let ready_metadata: BrainMetadataResponse = read_json(ready_metadata).await;
        let ready_admin = ready_metadata
            .collaborator_readiness
            .iter()
            .find(|entry| entry.target_npub == partial_target)
            .expect("repaired administrator readiness");
        assert_eq!((ready_admin.ready_count, ready_admin.total_count), (2, 2));

        let grant_member_body = serde_json::json!({
            "targetNpub": member,
            "grant": folder_key_grant_value(
                "grant-restricted-member-before-rotation",
                1,
                member.as_str(),
            ),
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "collab-grant-member-before-rotation",
                AdminAccessAction::GrantFolderAccess,
                Some("restricted"),
                Some(member.as_str()),
                Some(1),
            ),
        })
        .to_string();
        let grant_member = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/folders/restricted/access",
            Some(grant_member_body),
            TEST_NOW + 2,
        )
        .await;
        assert_eq!(grant_member.status(), StatusCode::OK);

        let rotate_body = serde_json::json!({
            "newKeyVersion": 2,
            "grants": [
                folder_key_grant_value(
                    "grant-restricted-admin-v2",
                    2,
                    npub(&admin_keys).as_str(),
                ),
                folder_key_grant_value(
                    "grant-restricted-complete-target-v2",
                    2,
                    target.as_str(),
                ),
                folder_key_grant_value(
                    "grant-restricted-repaired-target-v2",
                    2,
                    partial_target.as_str(),
                )
            ],
            "reencryptedRecords": [],
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "collab-rotate-restricted",
                AdminAccessAction::RemoveFolderAccess,
                Some("restricted"),
                Some(member.as_str()),
                Some(2),
            ),
        })
        .to_string();
        let rotate = authed_request(
            router.clone(),
            &admin_keys,
            "DELETE",
            &format!("/_admin/brains/acme/folders/restricted/access/{member}"),
            Some(rotate_body),
            TEST_NOW + 2,
        )
        .await;
        let rotate_status = rotate.status();
        let rotate_text = read_text(rotate).await;
        assert_eq!(rotate_status, StatusCode::OK, "{rotate_text}");
        let rotated_metadata = get_metadata(router, &admin_keys, "acme", TEST_NOW + 3).await;
        assert_eq!(rotated_metadata.status(), StatusCode::OK);
        let rotated_metadata: BrainMetadataResponse = read_json(rotated_metadata).await;
        let drifted_member = rotated_metadata
            .collaborator_readiness
            .iter()
            .find(|entry| entry.target_npub == member)
            .expect("rotated collaborator readiness");
        assert_eq!(
            (drifted_member.ready_count, drifted_member.total_count),
            (0, 1),
            "a member is measured only against policy-entitled current Folders"
        );
        let ready_admin = rotated_metadata
            .collaborator_readiness
            .iter()
            .find(|entry| entry.target_npub == partial_target)
            .expect("administrator readiness after rotation");
        assert_eq!(
            (ready_admin.ready_count, ready_admin.total_count),
            (2, 2),
            "the rotated current-version grant remains authoritative"
        );
        let restricted = rotated_metadata
            .folders
            .iter()
            .find(|folder| folder.id == "restricted")
            .expect("rotated Folder metadata");
        assert_eq!(restricted.current_key_version, 2);
    }

    #[tokio::test]
    async fn collaboration_retry_guidance_names_current_grant_recipients_not_issuers() {
        let admin_keys = Keys::generate();
        let holder_keys = Keys::generate();
        let holder = npub(&holder_keys);
        let target = npub(&Keys::generate());
        let router = router_with_test_org_folders(&admin_keys).await;

        let add_holder = serde_json::json!({
            "targetNpub": holder,
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "collab-holder-member",
                AdminAccessAction::AddMember,
                None,
                Some(&holder),
                None,
            ),
        })
        .to_string();
        let response = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/members",
            Some(add_holder),
            TEST_NOW,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let add_existing_member = serde_json::json!({
            "targetNpub": target,
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "collab-existing-member",
                AdminAccessAction::AddMember,
                None,
                Some(&target),
                None,
            ),
        })
        .to_string();
        let response = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/members",
            Some(add_existing_member),
            TEST_NOW,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let holder_grant = serde_json::json!({
            "targetNpub": holder,
            "grant": folder_key_grant_value("collab-holder-grant", 1, &holder),
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "collab-holder-access",
                AdminAccessAction::GrantFolderAccess,
                Some("restricted"),
                Some(&holder),
                Some(1),
            ),
        })
        .to_string();
        let response = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/folders/restricted/access",
            Some(holder_grant),
            TEST_NOW,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let partial = serde_json::json!({
            "targetNpub": target,
            "folders": [
                {"folderId":"getting-started","keyVersion":1,"path":"getting-started"},
                {"folderId":"restricted","keyVersion":1,"path":"restricted"}
            ],
            "grants": [],
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "collab-holder-guidance",
                AdminAccessAction::AddAdmin,
                None,
                Some(&target),
                None,
            ),
        })
        .to_string();
        let response = authed_request(
            router,
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(partial),
            TEST_NOW,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let receipt: EnsureOrganizationAdminResponse = read_json(response).await;
        assert_eq!(receipt.brain_role, "admin");
        let restricted = receipt
            .folders
            .iter()
            .find(|folder| folder.folder_id == "restricted")
            .unwrap();
        assert!(
            restricted
                .key_holders
                .iter()
                .any(|candidate| candidate.npub == holder),
            "the recipient who can actually unwrap the current grant must be named"
        );
    }

    #[tokio::test]
    async fn collaboration_receipt_detects_stale_added_and_removed_snapshot_drift() {
        let admin_keys = Keys::generate();
        let target = npub(&Keys::generate());
        let router = router_with_test_org_folders(&admin_keys).await;
        let body = serde_json::json!({
            "targetNpub": target,
            "folders": [
                {"folderId":"getting-started","keyVersion":2,"path":"getting-started"},
                {"folderId":"removed-before-commit","keyVersion":1,"path":"removed-before-commit"}
            ],
            "grants": [],
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "collab-drift-admin",
                AdminAccessAction::AddAdmin,
                None,
                Some(&target),
                None,
            ),
        })
        .to_string();
        let response = authed_request(
            router,
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(body),
            TEST_NOW,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let receipt: EnsureOrganizationAdminResponse = read_json(response).await;
        assert_eq!(receipt.state, CollaborationReceiptState::Partial);
        assert!(receipt.folders.iter().any(|folder| {
            folder.folder_id == "getting-started"
                && folder.outcome == CollaborationFolderOutcome::StaleVersion
                && folder.reason.as_deref() == Some("currentKeyVersionChanged")
        }));
        assert!(receipt.folders.iter().any(|folder| {
            folder.folder_id == "restricted"
                && folder.reason.as_deref() == Some("folderAddedSinceSnapshot")
        }));
        assert!(receipt.folders.iter().any(|folder| {
            folder.folder_id == "removed-before-commit"
                && folder.reason.as_deref() == Some("folderRemovedSinceSnapshot")
        }));
    }

    #[tokio::test]
    async fn collaboration_route_accepts_the_largest_valid_grant_batch() {
        let admin_keys = Keys::generate();
        let target = npub(&Keys::generate());
        let router = router_with_test_org_folders(&admin_keys).await;
        let folder_ids = std::iter::once("getting-started".to_owned())
            .chain(std::iter::once("restricted".to_owned()))
            .chain((2..MAX_COLLABORATION_FOLDERS).map(|index| format!("folder-{index}")))
            .collect::<Vec<_>>();
        let folders = folder_ids
            .iter()
            .map(|folder_id| {
                serde_json::json!({
                    "folderId": folder_id,
                    "keyVersion": 1,
                    "path": folder_id,
                })
            })
            .collect::<Vec<_>>();
        let grants = folder_ids
            .iter()
            .enumerate()
            .map(|(index, folder_id)| {
                let mut grant =
                    folder_key_grant_value(&format!("collab-max-grant-{index}"), 1, &target);
                grant["folderId"] = serde_json::json!(folder_id);
                grant["accessChangeEvent"] = serde_json::json!(admin_event(
                    &admin_keys,
                    "acme",
                    &format!("collab-max-evidence-{index}"),
                    AdminAccessAction::GrantFolderAccess,
                    Some(folder_id),
                    Some(&target),
                    Some(1),
                ));
                grant
            })
            .collect::<Vec<_>>();
        let body = serde_json::json!({
            "targetNpub": target,
            "folders": folders,
            "grants": grants,
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "collab-max-admin",
                AdminAccessAction::AddAdmin,
                None,
                Some(&target),
                None,
            ),
        })
        .to_string();
        assert!(
            body.len() > MAX_REQUEST_BODY_BYTES,
            "the acceptance fixture must prove the collaboration-specific body limit"
        );
        assert!(body.len() <= MAX_COLLABORATION_REQUEST_BODY_BYTES);

        let response = authed_request(
            router,
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(body),
            TEST_NOW,
        )
        .await;
        let status = response.status();
        let text = read_text_with_limit(response, MAX_COLLABORATION_REQUEST_BODY_BYTES).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let receipt: EnsureOrganizationAdminResponse = serde_json::from_str(&text).unwrap();
        assert_eq!(receipt.total_count, MAX_COLLABORATION_FOLDERS);
    }

    #[tokio::test]
    async fn signed_organization_collaboration_rejects_non_admin_and_malformed_evidence() {
        let admin_keys = Keys::generate();
        let actor_keys = Keys::generate();
        let target = npub(&Keys::generate());
        let router = router_with_test_org_folders(&admin_keys).await;
        let valid_body = serde_json::json!({
            "targetNpub": target,
            "folders": [],
            "grants": [],
            "accessChangeEvent": admin_event(&actor_keys,"acme","collab-denied",AdminAccessAction::AddAdmin,None,Some(&target),None)
        })
        .to_string();
        let denied = authed_request(
            router.clone(),
            &actor_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(valid_body),
            TEST_NOW,
        )
        .await;
        let denied_status = denied.status();
        let denied_text = read_text(denied).await;
        assert_eq!(denied_status, StatusCode::FORBIDDEN, "{denied_text}");
        let malformed_body = serde_json::json!({
            "targetNpub": target,
            "folders": [],
            "grants": [],
            "accessChangeEvent": {}
        })
        .to_string();
        let malformed = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(malformed_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

        let rollback_target = npub(&Keys::generate());
        let malformed_folder_evidence = serde_json::json!({
            "targetNpub": rollback_target,
            "folders": [
                {"folderId":"getting-started","keyVersion":1,"path":"getting-started"},
                {"folderId":"restricted","keyVersion":1,"path":"restricted"}
            ],
            "grants": [
                {"folderId":"getting-started", "id":"collab-rollback-valid", "keyVersion":1,
                 "recipientNpub":rollback_target, "wrappedEventJson":gift_wrap_event_json(&rollback_target),
                 "createdAt":"2026-06-23T00:00:00.000Z", "accessChangeEvent":admin_event(&admin_keys,"acme","collab-rollback-valid",AdminAccessAction::GrantFolderAccess,Some("getting-started"),Some(&rollback_target),Some(1))},
                {"folderId":"restricted", "id":"collab-rollback-malformed", "keyVersion":1,
                 "recipientNpub":rollback_target, "wrappedEventJson":gift_wrap_event_json(&rollback_target),
                 "createdAt":"2026-06-23T00:00:00.000Z", "accessChangeEvent":{}}
            ],
            "accessChangeEvent":admin_event(&admin_keys,"acme","collab-rollback-admin",AdminAccessAction::AddAdmin,None,Some(&rollback_target),None)
        }).to_string();
        let malformed = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(malformed_folder_evidence),
            TEST_NOW,
        )
        .await;
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        let metadata = authed_request(
            router.clone(),
            &admin_keys,
            "GET",
            "/_admin/brains/acme/metadata",
            None,
            TEST_NOW + 1,
        )
        .await;
        let status = metadata.status();
        let text = read_text(metadata).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let metadata: BrainMetadataResponse = serde_json::from_str(&text).unwrap();
        assert!(!metadata.members.contains(&rollback_target));
        assert!(!metadata.admins.contains(&rollback_target));
        assert!(!metadata.folders.iter().any(|folder| {
            folder.id == "getting-started" && folder.access_user_ids.contains(&rollback_target)
        }));

        let wrapper_target = npub(&Keys::generate());
        let malformed_wrapper = serde_json::json!({
            "targetNpub": wrapper_target,
            "folders": [
                {"folderId":"getting-started","keyVersion":1,"path":"getting-started"}
            ],
            "grants": [{
                "folderId":"getting-started",
                "id":"collab-malformed-wrapper",
                "keyVersion":1,
                "recipientNpub":wrapper_target,
                "wrappedEventJson":"not-a-nostr-event",
                "createdAt":"2026-06-23T00:00:00.000Z",
                "accessChangeEvent":admin_event(&admin_keys,"acme","collab-malformed-wrapper-evidence",AdminAccessAction::GrantFolderAccess,Some("getting-started"),Some(&wrapper_target),Some(1))
            }],
            "accessChangeEvent":admin_event(&admin_keys,"acme","collab-malformed-wrapper-admin",AdminAccessAction::AddAdmin,None,Some(&wrapper_target),None)
        }).to_string();
        let malformed = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(malformed_wrapper),
            TEST_NOW,
        )
        .await;
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        let metadata = authed_request(
            router.clone(),
            &admin_keys,
            "GET",
            "/_admin/brains/acme/metadata",
            None,
            TEST_NOW + 2,
        )
        .await;
        let metadata: BrainMetadataResponse = read_json(metadata).await;
        assert!(!metadata.members.contains(&wrapper_target));
        assert!(!metadata.admins.contains(&wrapper_target));

        let limit_target = npub(&Keys::generate());
        let grant = serde_json::json!({
            "folderId":"getting-started",
            "id":"collab-limit",
            "keyVersion":1,
            "recipientNpub":limit_target,
            "wrappedEventJson":gift_wrap_event_json(&limit_target),
            "createdAt":"2026-06-23T00:00:00.000Z",
            "accessChangeEvent":admin_event(&admin_keys,"acme","collab-limit-evidence",AdminAccessAction::GrantFolderAccess,Some("getting-started"),Some(&limit_target),Some(1))
        });
        let grant_limit = serde_json::json!({
            "targetNpub": limit_target,
            "folders": [{"folderId":"getting-started","keyVersion":1,"path":"getting-started"}],
            "grants": vec![grant; MAX_COLLABORATION_GRANTS + 1],
            "accessChangeEvent": admin_event(&admin_keys,"acme","collab-limit-admin",AdminAccessAction::AddAdmin,None,Some(&limit_target),None)
        }).to_string();
        let rejected = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/collaborators/ensure-admin",
            Some(grant_limit),
            TEST_NOW,
        )
        .await;
        assert_error(
            rejected,
            StatusCode::BAD_REQUEST,
            "collaboration grants exceed 1000 entries",
        )
        .await;
        let metadata = authed_request(
            router,
            &admin_keys,
            "GET",
            "/_admin/brains/acme/metadata",
            None,
            TEST_NOW + 3,
        )
        .await;
        let metadata: BrainMetadataResponse = read_json(metadata).await;
        assert!(!metadata.members.contains(&limit_target));
        assert!(!metadata.admins.contains(&limit_target));
    }

    #[tokio::test]
    async fn finish_setup_route_repairs_empty_setup_incomplete_folder() {
        let admin_keys = Keys::generate();
        let state = test_state();
        let router = router_with_state(state.clone());
        let create_brain = post_brain(
            router.clone(),
            &admin_keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_brain.status(), StatusCode::OK);

        {
            let mut store = state.store.lock().unwrap();
            store
                .insert_setup_incomplete_folder_for_repair(
                    &BrainId::new("acme").unwrap(),
                    &test_strategy_folder(),
                    &BTreeSet::new(),
                )
                .unwrap();
        }

        let body = serde_json::json!({
            "grants": [
                folder_key_grant_value("grant-strategy-admin-v1", 1, npub(&admin_keys).as_str())
            ],
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "change_finish_strategy",
                AdminAccessAction::SetFolderAccessMode,
                Some("strategy"),
                None,
                Some(1),
            ),
        })
        .to_string();
        let finish = authed_request(
            router,
            &admin_keys,
            "POST",
            "/_admin/brains/acme/folders/strategy/finish-setup",
            Some(body),
            TEST_NOW,
        )
        .await;
        assert_eq!(finish.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(finish).await;
        let strategy = metadata
            .folders
            .iter()
            .find(|folder| folder.id == "strategy")
            .expect("strategy folder metadata");
        assert!(!strategy.setup_incomplete);
    }

    #[tokio::test]
    async fn brain_invitation_routes_are_npub_bound_single_use_and_retry_safe() {
        let admin_keys = Keys::generate();
        let target_keys = Keys::generate();
        let wrong_keys = Keys::generate();
        let target_npub = npub(&target_keys);
        let router = router_with_test_org_folders(&admin_keys).await;

        let create_body = serde_json::json!({
            "targetNpub": target_npub,
            "initialFolderAccess": ["getting-started"],
            "expiresAt": "2026-06-30T00:00:00.000Z",
        })
        .to_string();
        let create = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/invitations",
            Some(create_body),
            TEST_NOW,
        )
        .await;
        if create.status() != StatusCode::OK {
            let body: ApiErrorBody = read_json(create).await;
            panic!("email bootstrap create failed: {}", body.error);
        }
        let invitation: BrainInvitationResponse = read_json_with_limit(create, 128 * 1024).await;
        assert_eq!(invitation.status, "pending");
        assert_eq!(invitation.user_id.as_deref(), Some(target_npub.as_str()));
        assert_eq!(
            invitation.initial_folder_access,
            vec!["getting-started".to_owned()]
        );

        let list = authed_request(
            router.clone(),
            &admin_keys,
            "GET",
            "/_admin/brains/acme/invitations",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(list.status(), StatusCode::OK);
        let listed: BrainInvitationListResponse = read_json(list).await;
        assert_eq!(listed.invitations.len(), 1);
        assert_eq!(listed.invitations[0].id, invitation.id);
        assert_eq!(listed.invitations[0].status, "pending");

        let non_admin_list = authed_request(
            router.clone(),
            &target_keys,
            "GET",
            "/_admin/brains/acme/invitations",
            None,
            TEST_NOW,
        )
        .await;
        assert_error(
            non_admin_list,
            StatusCode::FORBIDDEN,
            "brain admin access required",
        )
        .await;

        let link_path = format!("/_admin/brain-invitation-links/{}", invitation.invite_code);
        let wrong_view = authed_request(
            router.clone(),
            &wrong_keys,
            "GET",
            &link_path,
            None,
            TEST_NOW,
        )
        .await;
        assert_error(
            wrong_view,
            StatusCode::NOT_FOUND,
            "brain invitation unavailable",
        )
        .await;

        let view = authed_request(
            router.clone(),
            &target_keys,
            "GET",
            &link_path,
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(view.status(), StatusCode::OK);
        let viewed: BrainInvitationResponse = read_json(view).await;
        assert_eq!(viewed.id, invitation.id);

        let accept_path = format!("{link_path}/accept");
        let accept = authed_request(
            router.clone(),
            &target_keys,
            "POST",
            &accept_path,
            None,
            TEST_NOW + 2,
        )
        .await;
        assert_eq!(accept.status(), StatusCode::OK);
        let accepted: BrainInvitationResponse = read_json(accept).await;
        assert_eq!(accepted.status, "accepted");
        assert!(!accepted.duplicate_accept);

        let retry = authed_request(
            router.clone(),
            &target_keys,
            "POST",
            &accept_path,
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(retry.status(), StatusCode::OK);
        let retry: BrainInvitationResponse = read_json(retry).await;
        assert!(retry.duplicate_accept);

        let id_accept_path = format!("/_admin/brains/acme/invitations/{}/accept", invitation.id);
        let id_retry = authed_request(
            router.clone(),
            &target_keys,
            "POST",
            &id_accept_path,
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(id_retry.status(), StatusCode::OK);
        let id_retry: BrainInvitationResponse = read_json(id_retry).await;
        assert!(id_retry.duplicate_accept);

        let metadata = get_metadata(router.clone(), &target_keys, "acme", TEST_NOW).await;
        assert_eq!(metadata.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(metadata).await;
        assert!(metadata.members.contains(&target_npub));

        let revoke_path = format!("/_admin/brains/acme/invitations/{}", invitation.id);
        let revoke = authed_request(
            router.clone(),
            &admin_keys,
            "DELETE",
            &revoke_path,
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(revoke.status(), StatusCode::OK);
        let revoked: BrainInvitationResponse = read_json(revoke).await;
        assert_eq!(revoked.status, "revoked");

        let list_after_revoke = authed_request(
            router,
            &admin_keys,
            "GET",
            "/_admin/brains/acme/invitations",
            None,
            TEST_NOW + 3,
        )
        .await;
        assert_eq!(list_after_revoke.status(), StatusCode::OK);
        let listed: BrainInvitationListResponse = read_json(list_after_revoke).await;
        assert_eq!(listed.invitations.len(), 1);
        assert_eq!(listed.invitations[0].status, "revoked");
    }

    #[tokio::test]
    async fn email_brain_invitation_creates_bootstrap_and_claims_access_without_secret() {
        let admin_keys = Keys::generate();
        let claimant_keys = Keys::generate();
        let unwrap_keys = Keys::generate();
        let claimant_npub = npub(&claimant_keys);
        let unwrap_npub = npub(&unwrap_keys);
        let expected_claimant = claimant_npub.clone();
        let delivered_invites = Arc::new(Mutex::new(Vec::<BrainInviteEmail>::new()));
        let delivered_for_mailer = delivered_invites.clone();
        let router = router_with_state(
            test_state()
                .with_email_proof_verifier(move |email, actor| {
                    if email == "friend@example.com" && actor.to_string() == expected_claimant {
                        Ok(())
                    } else {
                        Err("email proof not found".to_owned())
                    }
                })
                .with_invite_mailer(move |email| {
                    delivered_for_mailer
                        .lock()
                        .expect("delivery capture mutex")
                        .push(email.clone());
                    Ok(())
                }),
        );
        let create_brain = post_brain(
            router.clone(),
            &admin_keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_brain.status(), StatusCode::OK);
        add_test_org_folders(&router, &admin_keys).await;
        let payload_hash = "sha256-bootstrap-payload";
        let bootstrap_wrapped_event_json = gift_wrap_event_json(&unwrap_npub);
        let authorization_event_json = email_bootstrap_authorization_event(
            &admin_keys,
            "acme",
            "friend@example.com",
            &unwrap_npub,
            payload_hash,
            "2026-06-30T00:00:00.000Z",
            &[
                ("getting-started", FolderAccessMode::AllMembers, 1),
                ("restricted", FolderAccessMode::Restricted, 1),
            ],
        );

        let create_body = serde_json::json!({
            "target": "friend@example.com",
            "initialFolderAccess": ["restricted"],
            "expiresAt": "2026-06-30T00:00:00.000Z",
            "inviteUnwrapNpub": unwrap_npub,
            "bootstrapPayloadHash": payload_hash,
            "bootstrapWrappedEventJson": bootstrap_wrapped_event_json,
            "bootstrapAuthorizationEventJson": authorization_event_json,
        })
        .to_string();
        let create = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/invitations",
            Some(create_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create.status(), StatusCode::OK);
        let invitation: BrainInvitationResponse = read_json_with_limit(create, 128 * 1024).await;
        assert_eq!(invitation.target_kind, "email_bootstrap");
        assert_eq!(invitation.delivery_status.as_deref(), Some("sent"));
        assert_eq!(invitation.user_id, None);
        assert_eq!(
            invitation.invited_email.as_deref(),
            Some("friend@example.com")
        );
        assert_eq!(
            invitation.invite_unwrap_npub.as_deref(),
            Some(unwrap_npub.as_str())
        );
        assert!(invitation.accept_path.ends_with("/claim"));
        assert!(invitation.public_instructions_path.ends_with("/llms.txt"));
        let expected_public_instructions_url =
            format!("{TEST_BASE_URL}{}", invitation.public_instructions_path);
        assert_eq!(
            invitation.public_instructions_url.as_deref(),
            Some(expected_public_instructions_url.as_str())
        );
        {
            let delivered = delivered_invites.lock().expect("delivery capture mutex");
            assert_eq!(delivered.len(), 1);
            assert_eq!(delivered[0].to, "friend@example.com");
            assert!(delivered[0].text.contains(&invitation.invite_code));
            assert!(
                delivered[0]
                    .text
                    .contains(invitation.public_instructions_url.as_deref().unwrap())
            );
            assert!(!delivered[0].text.contains('#'));
            assert!(!delivered[0].text.contains(payload_hash));
        }
        assert_eq!(
            invitation
                .bootstrap_scope
                .iter()
                .map(|scope| (scope.folder_id.clone(), scope.access, scope.key_version))
                .collect::<Vec<_>>(),
            vec![
                (
                    "getting-started".to_owned(),
                    FolderAccessMode::AllMembers,
                    1
                ),
                ("restricted".to_owned(), FolderAccessMode::Restricted, 1),
            ]
        );
        assert!(
            !serde_json::to_string(&invitation)
                .unwrap()
                .to_ascii_lowercase()
                .contains("secret")
        );

        let public_instructions = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&invitation.public_instructions_path)
                    .body(Body::empty())
                    .expect("valid public instructions request"),
            )
            .await
            .expect("public instructions response");
        assert_eq!(public_instructions.status(), StatusCode::OK);
        let public_instructions = read_text(public_instructions).await;
        assert!(public_instructions.contains("FiniteBrain public invite instructions"));
        for forbidden in [
            "friend@example.com",
            "Acme",
            "getting-started",
            "restricted",
            payload_hash,
            "pending",
            "encrypted grant placeholder",
        ] {
            assert!(
                !public_instructions.contains(forbidden),
                "public instructions leaked {forbidden}"
            );
        }
        assert!(public_instructions.contains("inviteSecret"));

        let proof_created_at = format_unix_timestamp(TEST_NOW).unwrap();
        let post_proof_body = serde_json::json!({
            "email": "friend@example.com",
            "emailProofCreatedAt": proof_created_at.clone(),
        })
        .to_string();
        let post_proof = authed_request(
            router.clone(),
            &claimant_keys,
            "POST",
            &format!(
                "/_admin/brain-invitation-links/{}/instructions",
                invitation.invite_code
            ),
            Some(post_proof_body.clone()),
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(post_proof.status(), StatusCode::OK);
        let post_proof = read_text(post_proof).await;
        assert!(post_proof.contains("FiniteBrain post-proof invite instructions"));
        assert!(post_proof.contains("friend@example.com"));
        assert!(post_proof.contains("Acme"));
        assert!(post_proof.contains("getting-started"));
        assert!(post_proof.contains("restricted"));
        assert!(post_proof.contains("expected key version: 1"));
        for forbidden in [
            payload_hash,
            "encrypted grant placeholder",
            "claim-grant-getting-started",
            "claim-grant-restricted",
        ] {
            assert!(
                !post_proof.contains(forbidden),
                "post-proof instructions leaked {forbidden}"
            );
        }

        let post_proof_bootstrap = authed_request(
            router.clone(),
            &claimant_keys,
            "POST",
            &format!(
                "/_admin/brain-invitation-links/{}/bootstrap",
                invitation.invite_code
            ),
            Some(post_proof_body.clone()),
            TEST_NOW + 1,
        )
        .await;
        if post_proof_bootstrap.status() != StatusCode::OK {
            let body: ApiErrorBody = read_json(post_proof_bootstrap).await;
            panic!("post-proof bootstrap failed: {}", body.error);
        }
        let post_proof_bootstrap: BrainInvitationResponse =
            read_json_with_limit(post_proof_bootstrap, 128 * 1024).await;
        assert_eq!(
            post_proof_bootstrap.bootstrap_wrapped_event_json.as_deref(),
            Some(bootstrap_wrapped_event_json.as_str())
        );
        assert!(
            !serde_json::to_string(&post_proof_bootstrap)
                .unwrap()
                .to_ascii_lowercase()
                .contains("secret")
        );

        let wrong_claim_proof_event_json = email_bootstrap_claim_proof_event(
            &Keys::generate(),
            "acme",
            &invitation.invite_code,
            "friend@example.com",
            &claimant_npub,
            payload_hash,
            &proof_created_at,
        );
        let wrong_claim_body = serde_json::json!({
            "email": "friend@example.com",
            "emailProofCreatedAt": proof_created_at.clone(),
            "inviteUnwrapProofEventJson": wrong_claim_proof_event_json,
            "grants": [
                {
                    "folderId": "getting-started",
                    "grant": folder_key_grant_value("claim-grant-getting-started", 1, &claimant_npub)
                },
                {
                    "folderId": "restricted",
                    "grant": folder_key_grant_value("claim-grant-restricted", 1, &claimant_npub)
                }
            ]
        })
        .to_string();
        let wrong_claim = authed_request(
            router.clone(),
            &claimant_keys,
            "POST",
            &format!(
                "/_admin/brain-invitation-links/{}/claim",
                invitation.invite_code
            ),
            Some(wrong_claim_body),
            TEST_NOW + 1,
        )
        .await;
        assert_error(wrong_claim, StatusCode::BAD_REQUEST, "Invite Unwrap npub").await;

        let claim_proof_event_json = email_bootstrap_claim_proof_event(
            &unwrap_keys,
            "acme",
            &invitation.invite_code,
            "friend@example.com",
            &claimant_npub,
            payload_hash,
            &proof_created_at,
        );
        let claim_body = serde_json::json!({
            "email": "friend@example.com",
            "emailProofCreatedAt": proof_created_at.clone(),
            "inviteUnwrapProofEventJson": claim_proof_event_json,
            "grants": [
                {
                    "folderId": "getting-started",
                    "grant": folder_key_grant_value("claim-grant-getting-started", 1, &claimant_npub)
                },
                {
                    "folderId": "restricted",
                    "grant": folder_key_grant_value("claim-grant-restricted", 1, &claimant_npub)
                }
            ]
        })
        .to_string();
        let claim = authed_request(
            router.clone(),
            &claimant_keys,
            "POST",
            &format!(
                "/_admin/brain-invitation-links/{}/claim",
                invitation.invite_code
            ),
            Some(claim_body.clone()),
            TEST_NOW + 2,
        )
        .await;
        if claim.status() != StatusCode::OK {
            let body: ApiErrorBody = read_json(claim).await;
            panic!("email bootstrap claim failed: {}", body.error);
        }
        let claimed: BrainInvitationResponse = read_json(claim).await;
        assert_eq!(claimed.status, "accepted");
        assert_eq!(claimed.user_id.as_deref(), Some(claimant_npub.as_str()));
        assert_eq!(
            claimed.claimed_by_npub.as_deref(),
            Some(claimant_npub.as_str())
        );
        assert_eq!(claimed.bootstrap_wrapped_event_json, None);

        let sync_before_retry = authed_request(
            router.clone(),
            &claimant_keys,
            "GET",
            "/_admin/brains/acme/sync/records?after=0&limit=20",
            None,
            TEST_NOW + 3,
        )
        .await;
        assert_eq!(sync_before_retry.status(), StatusCode::OK);
        let sync_before_retry: SyncPullResponse = read_json(sync_before_retry).await;

        let duplicate_claim_body = serde_json::json!({
            "email": "friend@example.com",
            "emailProofCreatedAt": format_unix_timestamp(TEST_NOW - 86_500).unwrap(),
            "grants": []
        })
        .to_string();
        let duplicate_claim = authed_request(
            router.clone(),
            &claimant_keys,
            "POST",
            &format!(
                "/_admin/brain-invitation-links/{}/claim",
                invitation.invite_code
            ),
            Some(duplicate_claim_body),
            TEST_NOW + 4,
        )
        .await;
        assert_eq!(duplicate_claim.status(), StatusCode::OK);
        let duplicate_claim: BrainInvitationResponse = read_json(duplicate_claim).await;
        assert!(duplicate_claim.duplicate_accept);

        let sync_after_retry = authed_request(
            router.clone(),
            &claimant_keys,
            "GET",
            "/_admin/brains/acme/sync/records?after=0&limit=20",
            None,
            TEST_NOW + 5,
        )
        .await;
        assert_eq!(sync_after_retry.status(), StatusCode::OK);
        let sync_after_retry: SyncPullResponse = read_json(sync_after_retry).await;
        assert_eq!(
            sync_after_retry.records.len(),
            sync_before_retry.records.len()
        );

        let metadata = get_metadata(router, &claimant_keys, "acme", TEST_NOW + 6).await;
        assert_eq!(metadata.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(metadata).await;
        assert!(metadata.members.contains(&claimant_npub));
        let restricted = metadata
            .folders
            .iter()
            .find(|folder| folder.id == "restricted")
            .expect("restricted folder");
        assert!(restricted.access_user_ids.contains(&claimant_npub));
    }

    #[tokio::test]
    async fn email_brain_invitation_claim_unlocks_selected_encrypted_folders_only() {
        let admin_keys = Keys::generate();
        let claimant_keys = Keys::generate();
        let unwrap_keys = Keys::generate();
        let admin_npub = npub(&admin_keys);
        let claimant_npub = npub(&claimant_keys);
        let unwrap_npub = npub(&unwrap_keys);
        let expected_claimant = claimant_npub.clone();
        let folder_key_base64 = FolderKey::from_bytes([9; 32]).to_base64();
        let router =
            router_with_state(test_state().with_email_proof_verifier(move |email, actor| {
                if email == "friend@example.com" && actor.to_string() == expected_claimant {
                    Ok(())
                } else {
                    Err("email proof not found".to_owned())
                }
            }));

        let create_brain = post_brain(
            router.clone(),
            &admin_keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_brain.status(), StatusCode::OK);
        add_test_org_folders(&router, &admin_keys).await;

        let create_private_body = serde_json::json!({
            "folderId": "private",
            "name": "Private",
            "role": "folder",
            "access": "restricted",
            "parentFolderId": null,
            "path": "private",
            "accessUserIds": [],
            "grants": [
                real_folder_key_grant_value(
                    "grant-private-admin-v1",
                    1,
                    &admin_keys,
                    "acme",
                    "private",
                    &admin_npub,
                    &folder_key_base64,
                )
            ],
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "change_create_private",
                AdminAccessAction::SetFolderAccessMode,
                Some("private"),
                None,
                Some(1),
            ),
        })
        .to_string();
        let create_private = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/folders",
            Some(create_private_body),
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(create_private.status(), StatusCode::OK);

        for (folder_id, object_id, content, nonce) in [
            (
                "getting-started",
                "obj_000000000401",
                "shared encrypted page",
                41,
            ),
            (
                "restricted",
                "obj_000000000402",
                "selected restricted encrypted page",
                42,
            ),
            ("private", "obj_000000000403", "private encrypted page", 43),
        ] {
            let path = format!("/_admin/brains/acme/folders/{folder_id}/objects/{object_id}");
            let body = object_write_body(
                &admin_keys,
                RevisionFixture {
                    brain_id: "acme",
                    folder_id,
                    object_id,
                    operation: FolderObjectOperation::Create,
                    revision: 1,
                    base_revision: None,
                    key_version: 1,
                    content,
                    nonce,
                    record_type: false,
                },
            );
            let write = authed_request(
                router.clone(),
                &admin_keys,
                "PUT",
                &path,
                Some(body),
                TEST_NOW + 2,
            )
            .await;
            assert_eq!(write.status(), StatusCode::OK);
        }

        let bootstrap_payload = serde_json::json!({
            "version": "finite-email-invite-bootstrap-payload-v1",
            "brainId": "acme",
            "invitedEmail": "friend@example.com",
            "inviteUnwrapNpub": unwrap_npub,
            "folders": [
                {
                    "folderId": "getting-started",
                    "access": FolderAccessMode::AllMembers,
                    "keyVersion": 1,
                },
                {
                    "folderId": "restricted",
                    "access": FolderAccessMode::Restricted,
                    "keyVersion": 1,
                },
            ],
            "grants": [
                {
                    "folderId": "getting-started",
                    "grant": real_folder_key_grant_value(
                        "bootstrap-getting-started-v1",
                        1,
                        &admin_keys,
                        "acme",
                        "getting-started",
                        &unwrap_npub,
                        &folder_key_base64,
                    )
                },
                {
                    "folderId": "restricted",
                    "grant": real_folder_key_grant_value(
                        "bootstrap-restricted-v1",
                        1,
                        &admin_keys,
                        "acme",
                        "restricted",
                        &unwrap_npub,
                        &folder_key_base64,
                    )
                },
            ],
        });
        let bootstrap_payload_json = bootstrap_payload.to_string();
        let payload_hash = sha256_payload_hash(&bootstrap_payload_json);
        let bootstrap_wrapped_event_json = email_bootstrap_wrapped_event_json(
            &admin_keys,
            "acme",
            &unwrap_npub,
            &bootstrap_payload_json,
        );
        let authorization_event_json = email_bootstrap_authorization_event(
            &admin_keys,
            "acme",
            "friend@example.com",
            &unwrap_npub,
            &payload_hash,
            "2026-06-30T00:00:00.000Z",
            &[
                ("getting-started", FolderAccessMode::AllMembers, 1),
                ("restricted", FolderAccessMode::Restricted, 1),
            ],
        );
        let create_body = serde_json::json!({
            "target": "friend@example.com",
            "initialFolderAccess": ["restricted"],
            "expiresAt": "2026-06-30T00:00:00.000Z",
            "inviteUnwrapNpub": unwrap_npub,
            "bootstrapPayloadHash": payload_hash,
            "bootstrapWrappedEventJson": bootstrap_wrapped_event_json,
            "bootstrapAuthorizationEventJson": authorization_event_json,
        })
        .to_string();
        assert!(!create_body.contains("folderKey"));
        let create = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/invitations",
            Some(create_body),
            TEST_NOW + 3,
        )
        .await;
        assert_eq!(create.status(), StatusCode::OK);
        let invitation: BrainInvitationResponse = read_json_with_limit(create, 128 * 1024).await;

        let proof_created_at = format_unix_timestamp(TEST_NOW).unwrap();
        let post_proof_body = serde_json::json!({
            "email": "friend@example.com",
            "emailProofCreatedAt": proof_created_at,
        })
        .to_string();
        let wrong_email_bootstrap = authed_request(
            router.clone(),
            &claimant_keys,
            "POST",
            &format!(
                "/_admin/brain-invitation-links/{}/bootstrap",
                invitation.invite_code
            ),
            Some(
                serde_json::json!({
                    "email": "other@example.com",
                    "emailProofCreatedAt": format_unix_timestamp(TEST_NOW).unwrap(),
                })
                .to_string(),
            ),
            TEST_NOW + 4,
        )
        .await;
        assert_error(
            wrong_email_bootstrap,
            StatusCode::NOT_FOUND,
            "brain invitation unavailable",
        )
        .await;

        let wrong_actor_bootstrap = authed_request(
            router.clone(),
            &Keys::generate(),
            "POST",
            &format!(
                "/_admin/brain-invitation-links/{}/bootstrap",
                invitation.invite_code
            ),
            Some(post_proof_body.clone()),
            TEST_NOW + 4,
        )
        .await;
        assert_error(
            wrong_actor_bootstrap,
            StatusCode::BAD_REQUEST,
            "email proof was not accepted",
        )
        .await;

        let post_proof_bootstrap = authed_request(
            router.clone(),
            &claimant_keys,
            "POST",
            &format!(
                "/_admin/brain-invitation-links/{}/bootstrap",
                invitation.invite_code
            ),
            Some(post_proof_body.clone()),
            TEST_NOW + 4,
        )
        .await;
        if post_proof_bootstrap.status() != StatusCode::OK {
            let body: ApiErrorBody = read_json(post_proof_bootstrap).await;
            panic!("post-proof bootstrap failed: {}", body.error);
        }
        let post_proof_bootstrap: BrainInvitationResponse =
            read_json_with_limit(post_proof_bootstrap, 128 * 1024).await;
        let returned_bootstrap = post_proof_bootstrap
            .bootstrap_wrapped_event_json
            .as_deref()
            .expect("post-proof bootstrap ciphertext");
        let opened_bootstrap = open_gift_wrap(
            &unwrap_keys,
            &Event::from_json(returned_bootstrap).unwrap(),
            &GiftWrapValidation::new(NostrPublicKey::from_protocol(unwrap_keys.public_key()))
                .with_expected_issuer(NostrPublicKey::from_protocol(admin_keys.public_key())),
        )
        .unwrap();
        assert_eq!(
            sha256_payload_hash(&opened_bootstrap.rumor.content),
            payload_hash
        );
        let opened_payload: serde_json::Value =
            serde_json::from_str(&opened_bootstrap.rumor.content).unwrap();
        assert_eq!(
            opened_payload["grants"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["folderId"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["getting-started", "restricted"]
        );
        for entry in opened_payload["grants"].as_array().unwrap() {
            let plaintext = open_wrapped_folder_key_grant(
                &unwrap_keys,
                entry["grant"]["wrappedEventJson"].as_str().unwrap(),
            );
            assert_eq!(
                plaintext["folderKey"].as_str(),
                Some(folder_key_base64.as_str())
            );
        }

        let claim_proof_event_json = email_bootstrap_claim_proof_event(
            &unwrap_keys,
            "acme",
            &invitation.invite_code,
            "friend@example.com",
            &claimant_npub,
            &payload_hash,
            post_proof_bootstrap.created_at.as_str(),
        );
        let claim_body = serde_json::json!({
            "email": "friend@example.com",
            "emailProofCreatedAt": post_proof_bootstrap.created_at,
            "inviteUnwrapProofEventJson": claim_proof_event_json,
            "grants": [
                {
                    "folderId": "getting-started",
                    "grant": real_folder_key_grant_value(
                        "claim-getting-started-v1",
                        1,
                        &claimant_keys,
                        "acme",
                        "getting-started",
                        &claimant_npub,
                        &folder_key_base64,
                    )
                },
                {
                    "folderId": "restricted",
                    "grant": real_folder_key_grant_value(
                        "claim-restricted-v1",
                        1,
                        &claimant_keys,
                        "acme",
                        "restricted",
                        &claimant_npub,
                        &folder_key_base64,
                    )
                },
            ],
        })
        .to_string();
        assert!(!claim_body.contains("folderKey"));
        assert!(!claim_body.to_ascii_lowercase().contains("invite_secret"));
        let claim = authed_request(
            router.clone(),
            &claimant_keys,
            "POST",
            &format!(
                "/_admin/brain-invitation-links/{}/claim",
                invitation.invite_code
            ),
            Some(claim_body),
            TEST_NOW + 5,
        )
        .await;
        assert_eq!(claim.status(), StatusCode::OK);

        let tombstoned_bootstrap = authed_request(
            router.clone(),
            &claimant_keys,
            "POST",
            &format!(
                "/_admin/brain-invitation-links/{}/bootstrap",
                invitation.invite_code
            ),
            Some(post_proof_body),
            TEST_NOW + 6,
        )
        .await;
        assert_eq!(tombstoned_bootstrap.status(), StatusCode::OK);
        let tombstoned_bootstrap: BrainInvitationResponse = read_json(tombstoned_bootstrap).await;
        assert_eq!(tombstoned_bootstrap.status, "accepted");
        assert_eq!(tombstoned_bootstrap.bootstrap_wrapped_event_json, None);

        let export = authed_request(
            router.clone(),
            &claimant_keys,
            "GET",
            "/_admin/brains/acme/export",
            None,
            TEST_NOW + 6,
        )
        .await;
        assert_eq!(export.status(), StatusCode::OK);
        let export: EncryptedBrainExportResponse = read_json_with_limit(export, 128 * 1024).await;
        assert!(
            export
                .folders
                .iter()
                .find(|folder| folder.id == "getting-started")
                .unwrap()
                .accessible
        );
        assert!(
            export
                .folders
                .iter()
                .find(|folder| folder.id == "restricted")
                .unwrap()
                .accessible
        );
        assert!(
            !export
                .folders
                .iter()
                .find(|folder| folder.id == "private")
                .unwrap()
                .accessible
        );
        let getting_started_key =
            folder_key_from_export_grant(&claimant_keys, &export, "getting-started");
        let restricted_key = folder_key_from_export_grant(&claimant_keys, &export, "restricted");
        assert_eq!(getting_started_key, folder_key_base64);
        assert_eq!(restricted_key, folder_key_base64);
        let getting_started_object = export
            .objects
            .iter()
            .find(|object| object.object_id == "obj_000000000401")
            .unwrap();
        let restricted_object = export
            .objects
            .iter()
            .find(|object| object.object_id == "obj_000000000402")
            .unwrap();
        let private_object = export
            .objects
            .iter()
            .find(|object| object.object_id == "obj_000000000403")
            .unwrap();
        assert!(!getting_started_object.opaque);
        assert!(!restricted_object.opaque);
        assert!(private_object.opaque);
        assert!(private_object.payload_json.is_none());
        assert_eq!(
            open_export_object_plaintext(getting_started_object, &getting_started_key),
            "shared encrypted page"
        );
        assert_eq!(
            open_export_object_plaintext(restricted_object, &restricted_key),
            "selected restricted encrypted page"
        );

        let sync = authed_request(
            router,
            &claimant_keys,
            "GET",
            "/_admin/brains/acme/sync/bootstrap",
            None,
            TEST_NOW + 7,
        )
        .await;
        assert_eq!(sync.status(), StatusCode::OK);
        let sync: SyncBootstrapResponse = read_json_with_limit(sync, 128 * 1024).await;
        assert!(
            sync.objects
                .iter()
                .any(|object| object.object_id == "obj_000000000401")
        );
        assert!(
            sync.objects
                .iter()
                .any(|object| object.object_id == "obj_000000000402")
        );
        assert!(
            !sync
                .objects
                .iter()
                .any(|object| object.object_id == "obj_000000000403")
        );
        let synced_restricted = sync
            .objects
            .iter()
            .find(|object| object.object_id == "obj_000000000402")
            .unwrap();
        assert_eq!(
            open_sync_object_plaintext(synced_restricted, &restricted_key),
            "selected restricted encrypted page"
        );
    }

    #[tokio::test]
    async fn email_brain_invitation_creation_without_mailer_returns_manual_delivery_details() {
        let admin_keys = Keys::generate();
        let unwrap_keys = Keys::generate();
        let unwrap_npub = npub(&unwrap_keys);
        let router = router_with_test_org_folders(&admin_keys).await;
        let payload_hash = "sha256-bootstrap-payload";
        let authorization_event_json = email_bootstrap_authorization_event(
            &admin_keys,
            "acme",
            "manual@example.com",
            &unwrap_npub,
            payload_hash,
            "2026-06-30T00:00:00.000Z",
            &[
                ("getting-started", FolderAccessMode::AllMembers, 1),
                ("restricted", FolderAccessMode::Restricted, 1),
            ],
        );
        let create_body = serde_json::json!({
            "target": "manual@example.com",
            "initialFolderAccess": ["restricted"],
            "expiresAt": "2026-06-30T00:00:00.000Z",
            "inviteUnwrapNpub": unwrap_npub,
            "bootstrapPayloadHash": payload_hash,
            "bootstrapWrappedEventJson": gift_wrap_event_json(&npub(&unwrap_keys)),
            "bootstrapAuthorizationEventJson": authorization_event_json,
        })
        .to_string();
        let create = authed_request(
            router,
            &admin_keys,
            "POST",
            "/_admin/brains/acme/invitations",
            Some(create_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create.status(), StatusCode::OK);
        let invitation: BrainInvitationResponse = read_json(create).await;
        assert_eq!(
            invitation.delivery_status.as_deref(),
            Some("not_configured")
        );
        assert!(invitation.public_instructions_path.ends_with("/llms.txt"));
        assert!(
            invitation
                .public_instructions_url
                .as_deref()
                .unwrap()
                .starts_with(TEST_BASE_URL)
        );
        assert!(
            !serde_json::to_string(&invitation)
                .unwrap()
                .contains("inviteSecret")
        );
    }

    #[tokio::test]
    async fn brain_invitation_routing_keeps_active_finite_vip_nip05_on_npub_path() {
        let admin_keys = Keys::generate();
        let target_keys = Keys::generate();
        let target_npub = npub(&target_keys);
        let target_hex = NostrPublicKey::from_protocol(target_keys.public_key()).to_hex();
        let identifier = Nip05Identifier::parse("alice@finite.vip").unwrap();
        let document = serde_json::json!({
            "names": { "alice": target_hex },
        })
        .to_string()
        .into_bytes();
        let router = router_with_state(
            test_state().with_nip05_fixture(identifier.well_known_request().url, document),
        );
        let create_brain = post_brain(
            router.clone(),
            &admin_keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_brain.status(), StatusCode::OK);
        add_test_org_folders(&router, &admin_keys).await;

        let create_body = serde_json::json!({
            "target": "alice@finite.vip",
            "initialFolderAccess": ["getting-started"],
            "expiresAt": "2026-06-30T00:00:00.000Z",
        })
        .to_string();
        let create = authed_request(
            router,
            &admin_keys,
            "POST",
            "/_admin/brains/acme/invitations",
            Some(create_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create.status(), StatusCode::OK);
        let invitation: BrainInvitationResponse = read_json(create).await;
        assert_eq!(invitation.target_kind, "npub");
        assert_eq!(invitation.user_id.as_deref(), Some(target_npub.as_str()));
        assert_eq!(invitation.invited_email, None);
    }

    #[tokio::test]
    async fn configured_identity_authority_serves_finite_vip_nip05_resolution() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let bytes = stream.read(&mut request).unwrap();
            assert!(
                String::from_utf8_lossy(&request[..bytes])
                    .starts_with("GET /.well-known/nostr.json?name=cheater ")
            );
            let body = serde_json::json!({ "names": { "cheater": "77".repeat(32) } }).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let state = test_state().with_identity_authority_url(format!("http://{address}"));

        let resolved = resolve_identity_input(&state, "cheater@finite.vip")
            .await
            .unwrap();

        assert_eq!(resolved.hex, "77".repeat(32));
        assert_eq!(resolved.nip05.as_deref(), Some("cheater@finite.vip"));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn brain_invitation_create_rejects_existing_members() {
        let admin_keys = Keys::generate();
        let admin_npub = npub(&admin_keys);
        let router = router_with_test_org_folders(&admin_keys).await;

        let create_body = serde_json::json!({
            "targetNpub": admin_npub,
            "initialFolderAccess": ["getting-started"],
            "expiresAt": "2026-06-30T00:00:00.000Z",
        })
        .to_string();
        let create = authed_request(
            router,
            &admin_keys,
            "POST",
            "/_admin/brains/acme/invitations",
            Some(create_body),
            TEST_NOW,
        )
        .await;
        assert_error(
            create,
            StatusCode::BAD_REQUEST,
            "target is already a brain member",
        )
        .await;
    }

    #[tokio::test]
    async fn share_link_routes_create_access_and_optional_mount_on_accept() {
        let admin_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let wrong_keys = Keys::generate();
        let recipient_npub = npub(&recipient_keys);
        let router = router_with_test_org_folders(&admin_keys).await;

        let create_folder_body = serde_json::json!({
            "folderId": "strategy",
            "name": "Strategy",
            "role": "folder",
            "access": "restricted",
            "parentFolderId": "getting-started",
            "path": "getting-started/Strategy",
            "accessUserIds": [],
            "grants": [
                folder_key_grant_value("grant-strategy-admin-v1", 1, npub(&admin_keys).as_str())
            ],
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "change_create_strategy_share",
                AdminAccessAction::SetFolderAccessMode,
                Some("strategy"),
                None,
                Some(1),
            ),
        })
        .to_string();
        let create_folder = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/folders",
            Some(create_folder_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create_folder.status(), StatusCode::OK);

        let create_share_body = serde_json::json!({
            "recipientNpub": recipient_npub,
            "grant": folder_key_grant_value("grant-strategy-recipient-v1", 1, recipient_npub.as_str()),
            "accessChangeEvent": admin_event(
                &admin_keys,
                "acme",
                "change_share_strategy",
                AdminAccessAction::GrantFolderAccess,
                Some("strategy"),
                Some(recipient_npub.as_str()),
                Some(1),
            ),
            "expiresAt": "2026-06-30T00:00:00.000Z",
            "createPersonalMount": true,
        })
        .to_string();
        let create_share = authed_request(
            router.clone(),
            &admin_keys,
            "POST",
            "/_admin/brains/acme/folders/strategy/share-links",
            Some(create_share_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create_share.status(), StatusCode::OK);
        let share_link: ShareLinkResponse = read_json(create_share).await;
        assert_eq!(share_link.status, "pending");
        assert_eq!(share_link.recipient_npub, recipient_npub);

        let list = authed_request(
            router.clone(),
            &admin_keys,
            "GET",
            "/_admin/brains/acme/folders/strategy/share-links",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(list.status(), StatusCode::OK);
        let listed: ShareLinkListResponse = read_json(list).await;
        assert_eq!(listed.share_links.len(), 1);
        assert_eq!(listed.share_links[0].id, share_link.id);
        assert_eq!(listed.share_links[0].status, "pending");

        let non_admin_list = authed_request(
            router.clone(),
            &recipient_keys,
            "GET",
            "/_admin/brains/acme/folders/strategy/share-links",
            None,
            TEST_NOW,
        )
        .await;
        assert_error(
            non_admin_list,
            StatusCode::FORBIDDEN,
            "brain admin access required",
        )
        .await;

        let share_path = format!("/_admin/share-links/{}", share_link.id);
        let wrong_view = authed_request(
            router.clone(),
            &wrong_keys,
            "GET",
            &share_path,
            None,
            TEST_NOW,
        )
        .await;
        assert_error(wrong_view, StatusCode::NOT_FOUND, "share link unavailable").await;

        let view = authed_request(
            router.clone(),
            &recipient_keys,
            "GET",
            &share_path,
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(view.status(), StatusCode::OK);

        let accept_path = format!("{share_path}/accept");
        let accept = authed_request(
            router.clone(),
            &recipient_keys,
            "POST",
            &accept_path,
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(accept.status(), StatusCode::OK);
        let accepted: ShareLinkResponse = read_json(accept).await;
        assert_eq!(accepted.status, "accepted");
        assert!(accepted.personal_mount_id.is_some());
        assert!(!accepted.duplicate_accept);

        let retry = authed_request(
            router.clone(),
            &recipient_keys,
            "POST",
            &accept_path,
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(retry.status(), StatusCode::OK);
        let retry: ShareLinkResponse = read_json(retry).await;
        assert!(retry.duplicate_accept);

        let metadata = get_metadata(router.clone(), &recipient_keys, "acme", TEST_NOW).await;
        assert_eq!(metadata.status(), StatusCode::OK);
        let metadata: BrainMetadataResponse = read_json(metadata).await;
        assert!(metadata.members.contains(&recipient_npub));
        let strategy = metadata
            .folders
            .iter()
            .find(|folder| folder.id == "strategy")
            .expect("strategy folder metadata");
        assert_eq!(strategy.access_user_ids, vec![recipient_npub]);
        assert_eq!(metadata.grant_count, 4);

        let revoke = authed_request(
            router.clone(),
            &admin_keys,
            "DELETE",
            &share_path,
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(revoke.status(), StatusCode::OK);
        let revoked: ShareLinkResponse = read_json(revoke).await;
        assert_eq!(revoked.status, "revoked");

        let list_after_revoke = authed_request(
            router,
            &admin_keys,
            "GET",
            "/_admin/brains/acme/folders/strategy/share-links",
            None,
            TEST_NOW + 2,
        )
        .await;
        assert_eq!(list_after_revoke.status(), StatusCode::OK);
        let listed: ShareLinkListResponse = read_json(list_after_revoke).await;
        assert_eq!(listed.share_links.len(), 1);
        assert_eq!(listed.share_links[0].status, "revoked");
    }

    #[tokio::test]
    async fn shared_folder_routes_project_mounts_and_route_writes_to_source() {
        let source_admin_keys = Keys::generate();
        let destination_admin_keys = Keys::generate();
        let destination_member_keys = Keys::generate();
        let source_admin_npub = npub(&source_admin_keys);
        let destination_admin_npub = npub(&destination_admin_keys);
        let destination_member_npub = npub(&destination_member_keys);
        let router = test_router();

        let create_source = post_brain(
            router.clone(),
            &source_admin_keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_source.status(), StatusCode::OK);
        let create_destination = post_brain(
            router.clone(),
            &destination_admin_keys,
            &create_brain_body("dest", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_destination.status(), StatusCode::OK);

        let create_folder_body = serde_json::json!({
            "folderId": "strategy",
            "name": "Strategy",
            "role": "folder",
            "access": "restricted",
            "parentFolderId": null,
            "path": "Strategy",
            "accessUserIds": [],
            "grants": [
                folder_key_grant_value("grant-strategy-source-admin-v1", 1, source_admin_npub.as_str())
            ],
            "accessChangeEvent": admin_event(
                &source_admin_keys,
                "acme",
                "change_create_shared_strategy",
                AdminAccessAction::SetFolderAccessMode,
                Some("strategy"),
                None,
                Some(1),
            ),
        })
        .to_string();
        let create_folder = authed_request(
            router.clone(),
            &source_admin_keys,
            "POST",
            "/_admin/brains/acme/folders",
            Some(create_folder_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create_folder.status(), StatusCode::OK);

        let mark_source_body = serde_json::json!({
            "accessChangeEvent": admin_event(
                &source_admin_keys,
                "acme",
                "change_mark_shared_strategy",
                AdminAccessAction::SetFolderAccessMode,
                Some("strategy"),
                None,
                Some(1),
            ),
        })
        .to_string();
        let mark_source = authed_request(
            router.clone(),
            &source_admin_keys,
            "POST",
            "/_admin/brains/acme/folders/strategy/share-source",
            Some(mark_source_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(mark_source.status(), StatusCode::OK);
        let source_metadata: BrainMetadataResponse = read_json(mark_source).await;
        assert!(
            source_metadata
                .folders
                .iter()
                .find(|folder| folder.id == "strategy")
                .unwrap()
                .shared_folder_source
        );

        let create_invitation_body = serde_json::json!({
            "destinationBrainId": "dest",
            "destinationAdminNpub": destination_admin_npub,
            "grant": folder_key_grant_value("grant-strategy-dest-admin-v1", 1, destination_admin_npub.as_str()),
            "accessChangeEvent": admin_event(
                &source_admin_keys,
                "acme",
                "change_invite_dest_strategy",
                AdminAccessAction::GrantFolderAccess,
                Some("strategy"),
                Some(destination_admin_npub.as_str()),
                Some(1),
            ),
        })
        .to_string();
        let create_invitation = authed_request(
            router.clone(),
            &source_admin_keys,
            "POST",
            "/_admin/brains/acme/folders/strategy/shared-folder-invitations",
            Some(create_invitation_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create_invitation.status(), StatusCode::OK);
        let invitation: SharedFolderInvitationResponse = read_json(create_invitation).await;
        assert_eq!(invitation.status, "pending");

        let wrong_view = authed_request(
            router.clone(),
            &source_admin_keys,
            "GET",
            &format!("/_admin/shared-folder-invitations/{}", invitation.id),
            None,
            TEST_NOW,
        )
        .await;
        assert_error(
            wrong_view,
            StatusCode::NOT_FOUND,
            "shared folder invitation unavailable",
        )
        .await;

        let accept = authed_request(
            router.clone(),
            &destination_admin_keys,
            "POST",
            &format!("/_admin/shared-folder-invitations/{}/accept", invitation.id),
            None,
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(accept.status(), StatusCode::OK);
        let accepted: SharedFolderInvitationResponse = read_json(accept).await;
        assert_eq!(accepted.status, "accepted");
        assert!(!accepted.duplicate_accept);

        let accept_retry = authed_request(
            router.clone(),
            &destination_admin_keys,
            "POST",
            &format!("/_admin/shared-folder-invitations/{}/accept", invitation.id),
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(accept_retry.status(), StatusCode::OK);
        let accept_retry: SharedFolderInvitationResponse = read_json(accept_retry).await;
        assert_eq!(accept_retry.status, "accepted");
        assert!(accept_retry.duplicate_accept);

        let destination_metadata =
            get_metadata(router.clone(), &destination_admin_keys, "dest", TEST_NOW).await;
        assert_eq!(destination_metadata.status(), StatusCode::OK);
        let destination_metadata: BrainMetadataResponse = read_json(destination_metadata).await;
        assert_eq!(destination_metadata.mounted_folders.len(), 1);
        let mount = &destination_metadata.mounted_folders[0];
        assert_eq!(mount.state, "available");
        assert_eq!(mount.source_brain_id, "acme");
        assert_eq!(mount.source_folder_id, "strategy");
        let connection_id = mount.connection_id.clone();

        let source_invitations = authed_request(
            router.clone(),
            &source_admin_keys,
            "GET",
            "/_admin/brains/acme/shared-folder-invitations",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(source_invitations.status(), StatusCode::OK);
        let source_invitations: SharedFolderInvitationListResponse =
            read_json(source_invitations).await;
        assert_eq!(source_invitations.outgoing.len(), 1);
        assert_eq!(source_invitations.outgoing[0].id, invitation.id);
        assert_eq!(source_invitations.outgoing[0].status, "accepted");
        assert!(source_invitations.incoming.is_empty());

        let destination_invitations = authed_request(
            router.clone(),
            &destination_admin_keys,
            "GET",
            "/_admin/brains/dest/shared-folder-invitations",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(destination_invitations.status(), StatusCode::OK);
        let destination_invitations: SharedFolderInvitationListResponse =
            read_json(destination_invitations).await;
        assert!(destination_invitations.outgoing.is_empty());
        assert_eq!(destination_invitations.incoming.len(), 1);
        assert_eq!(destination_invitations.incoming[0].id, invitation.id);

        let source_connections = authed_request(
            router.clone(),
            &source_admin_keys,
            "GET",
            "/_admin/brains/acme/shared-folder-connections",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(source_connections.status(), StatusCode::OK);
        let source_connections: SharedFolderConnectionListResponse =
            read_json(source_connections).await;
        assert_eq!(source_connections.outgoing.len(), 1);
        assert_eq!(source_connections.outgoing[0].id, connection_id);
        assert_eq!(source_connections.outgoing[0].status, "active");
        assert!(source_connections.incoming.is_empty());

        let destination_connections = authed_request(
            router.clone(),
            &destination_admin_keys,
            "GET",
            "/_admin/brains/dest/shared-folder-connections",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(destination_connections.status(), StatusCode::OK);
        let destination_connections: SharedFolderConnectionListResponse =
            read_json(destination_connections).await;
        assert!(destination_connections.outgoing.is_empty());
        assert_eq!(destination_connections.incoming.len(), 1);
        assert_eq!(destination_connections.incoming[0].id, connection_id);

        let add_destination_member_body = serde_json::json!({
            "targetNpub": destination_member_npub,
            "accessChangeEvent": admin_event(
                &destination_admin_keys,
                "dest",
                "change_add_dest_member",
                AdminAccessAction::AddMember,
                None,
                Some(destination_member_npub.as_str()),
                None,
            ),
        })
        .to_string();
        let add_destination_member = authed_request(
            router.clone(),
            &destination_admin_keys,
            "POST",
            "/_admin/brains/dest/members",
            Some(add_destination_member_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(add_destination_member.status(), StatusCode::OK);

        let add_connection_member_body = serde_json::json!({
            "action": "add",
            "targetNpub": destination_member_npub,
            "grant": folder_key_grant_value("grant-strategy-dest-member-v1", 1, destination_member_npub.as_str()),
            "newKeyVersion": null,
            "grants": [],
            "reencryptedRecords": [],
        })
        .to_string();
        let add_connection_member = authed_request(
            router.clone(),
            &destination_admin_keys,
            "PATCH",
            &format!("/_admin/shared-folder-connections/{connection_id}/members"),
            Some(add_connection_member_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(add_connection_member.status(), StatusCode::OK);
        let connection: SharedFolderConnectionResponse = read_json(add_connection_member).await;
        assert!(connection.member_npubs.contains(&destination_member_npub));

        let destination_member_metadata =
            get_metadata(router.clone(), &destination_member_keys, "dest", TEST_NOW).await;
        assert_eq!(destination_member_metadata.status(), StatusCode::OK);
        let destination_member_metadata: BrainMetadataResponse =
            read_json(destination_member_metadata).await;
        assert_eq!(
            destination_member_metadata.mounted_folders[0].state,
            "available"
        );

        let object_path = "/_admin/brains/acme/folders/strategy/objects/obj_000000000101";
        let create_source_object_body = object_write_body(
            &destination_member_keys,
            RevisionFixture {
                brain_id: "acme",
                folder_id: "strategy",
                object_id: "obj_000000000101",
                operation: FolderObjectOperation::Create,
                revision: 1,
                base_revision: None,
                key_version: 1,
                content: "mounted write goes to source",
                nonce: 21,
                record_type: false,
            },
        );
        let create_source_object = authed_request(
            router.clone(),
            &destination_member_keys,
            "PUT",
            object_path,
            Some(create_source_object_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(create_source_object.status(), StatusCode::OK);

        let source_bootstrap = authed_request(
            router.clone(),
            &destination_member_keys,
            "GET",
            "/_admin/brains/acme/sync/bootstrap",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(source_bootstrap.status(), StatusCode::OK);
        let source_bootstrap: SyncBootstrapResponse = read_json(source_bootstrap).await;
        assert_eq!(source_bootstrap.object_count, 1);

        let destination_bootstrap = authed_request(
            router.clone(),
            &destination_member_keys,
            "GET",
            "/_admin/brains/dest/sync/bootstrap",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(destination_bootstrap.status(), StatusCode::OK);
        let destination_bootstrap: SyncBootstrapResponse = read_json(destination_bootstrap).await;
        assert_eq!(destination_bootstrap.object_count, 0);

        let remove_connection_member_body = serde_json::json!({
            "action": "remove",
            "targetNpub": destination_member_npub,
            "grant": null,
            "newKeyVersion": 2,
            "grants": [
                folder_key_grant_value("grant-strategy-source-admin-v2", 2, source_admin_npub.as_str()),
                folder_key_grant_value("grant-strategy-dest-admin-v2", 2, destination_admin_npub.as_str())
            ],
            "reencryptedRecords": [
                rotation_object_value(
                    &destination_admin_keys,
                    "acme",
                    "strategy",
                    "obj_000000000101",
                    2,
                    Some(1),
                    2,
                    "reencrypted after dest member removal",
                    22,
                )
            ],
        })
        .to_string();
        let remove_connection_member = authed_request(
            router.clone(),
            &destination_admin_keys,
            "PATCH",
            &format!("/_admin/shared-folder-connections/{connection_id}/members"),
            Some(remove_connection_member_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(remove_connection_member.status(), StatusCode::OK);

        let locked_metadata = get_metadata(
            router.clone(),
            &destination_member_keys,
            "dest",
            TEST_NOW + 1,
        )
        .await;
        assert_eq!(locked_metadata.status(), StatusCode::OK);
        let locked_metadata: BrainMetadataResponse = read_json(locked_metadata).await;
        assert_eq!(locked_metadata.mounted_folders[0].state, "locked");

        let revoke_connection_body = serde_json::json!({
            "newKeyVersion": 3,
            "grants": [
                folder_key_grant_value("grant-strategy-source-admin-v3", 3, source_admin_npub.as_str())
            ],
            "reencryptedRecords": [
                rotation_object_value(
                    &source_admin_keys,
                    "acme",
                    "strategy",
                    "obj_000000000101",
                    3,
                    Some(2),
                    3,
                    "reencrypted after source revocation",
                    23,
                )
            ],
        })
        .to_string();
        let revoke_connection = authed_request(
            router.clone(),
            &source_admin_keys,
            "DELETE",
            &format!("/_admin/shared-folder-connections/{connection_id}"),
            Some(revoke_connection_body),
            TEST_NOW,
        )
        .await;
        assert_eq!(revoke_connection.status(), StatusCode::OK);
        let revoked: SharedFolderConnectionResponse = read_json(revoke_connection).await;
        assert_eq!(revoked.status, "revoked");

        let revoked_mounts = authed_request(
            router,
            &destination_admin_keys,
            "GET",
            "/_admin/brains/dest/organization-folder-mounts",
            None,
            TEST_NOW,
        )
        .await;
        assert_eq!(revoked_mounts.status(), StatusCode::OK);
        let revoked_mounts: Vec<MountedFolderResponse> = read_json(revoked_mounts).await;
        assert_eq!(revoked_mounts[0].state, "revoked");
    }

    fn test_router() -> Router {
        router_with_state(test_state())
    }

    fn test_state() -> ServerState {
        let store = BrainStore::open_in_memory().unwrap();
        ServerState::new(store, TEST_BASE_URL).with_auth_clock(TEST_NOW, 60)
    }

    fn personal_test_state(owner_keys: &Keys, agent_keys: &Keys) -> ServerState {
        let mut store = BrainStore::open_in_memory().unwrap();
        let owner_npub = UserId::new(npub(owner_keys)).unwrap();
        let agent_npub = UserId::new(npub(agent_keys)).unwrap();
        let output =
            bootstrap_personal_brain("personal", "Personal Brain", owner_npub.as_str().to_owned())
                .unwrap();
        store
            .create_personal_brain_bootstrap(
                &output,
                &[],
                &agent_npub,
                &owner_npub,
                &test_rfc3339(),
            )
            .unwrap();
        ServerState::new(store, TEST_BASE_URL).with_auth_clock(TEST_NOW, 60)
    }

    fn personal_test_router(owner_keys: &Keys, agent_keys: &Keys) -> Router {
        router_with_state(personal_test_state(owner_keys, agent_keys))
    }

    fn spawn_json_authority(
        responses: Vec<(&'static str, serde_json::Value)>,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for (expected_path, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 4096];
                    let bytes = stream.read(&mut chunk).unwrap();
                    if bytes == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..bytes]);
                    let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                assert!(
                    request.starts_with(&format!("POST {expected_path} ")),
                    "unexpected authority request: {request}"
                );
                let body = body.to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (format!("http://{address}"), server)
    }

    enum AuthorityTestResponse {
        Status,
        Malformed,
        DeclaredOversized,
        StreamedOversized,
        MidBodyStall,
    }

    fn spawn_authority_response(
        response: AuthorityTestResponse,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            match response {
                AuthorityTestResponse::Status => stream
                    .write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 20\r\nConnection: close\r\n\r\nprivate upstream body",
                    )
                    .unwrap(),
                AuthorityTestResponse::Malformed => stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot-json",
                    )
                    .unwrap(),
                AuthorityTestResponse::DeclaredOversized => write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    AUTHORITY_RESPONSE_MAX_BYTES + 1
                )
                .unwrap(),
                AuthorityTestResponse::StreamedOversized => {
                    write!(
                        stream,
                        "HTTP/1.0 200 OK\r\nConnection: close\r\n\r\n{}",
                        "x".repeat(AUTHORITY_RESPONSE_MAX_BYTES as usize + 1)
                    )
                    .unwrap();
                }
                AuthorityTestResponse::MidBodyStall => {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 128\r\nConnection: close\r\n\r\n{",
                        )
                        .unwrap();
                    std::thread::sleep(AUTHORITY_IO_TIMEOUT + Duration::from_secs(1));
                }
            }
        });
        (format!("http://{address}/authority"), server)
    }

    #[tokio::test]
    async fn authority_boundary_classifies_status_malformed_and_oversized_without_body_leaks() {
        for (response, category) in [
            (AuthorityTestResponse::Status, "upstream-status"),
            (AuthorityTestResponse::Malformed, "malformed-response"),
            (
                AuthorityTestResponse::DeclaredOversized,
                "oversized-response",
            ),
            (
                AuthorityTestResponse::StreamedOversized,
                "oversized-response",
            ),
        ] {
            let (url, server) = spawn_authority_response(response);
            let error = post_authority_json::<serde_json::Value>(
                &url,
                "Authorization",
                "Bearer authority-secret",
                &serde_json::json!({ "request": "safe" }),
                "test",
            )
            .await
            .unwrap_err();
            assert_eq!(error.status, StatusCode::BAD_GATEWAY);
            assert!(error.message.ends_with(category), "{}", error.message);
            assert!(!error.message.contains("authority-secret"));
            assert!(!error.message.contains("private upstream body"));
            assert!(!error.message.contains(&url));
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn stalled_authority_times_out_without_blocking_local_health() {
        let (url, server) = spawn_authority_response(AuthorityTestResponse::MidBodyStall);
        let authority = tokio::spawn(async move {
            post_authority_json::<serde_json::Value>(
                &url,
                "Authorization",
                "Bearer authority-secret",
                &serde_json::json!({}),
                "test",
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let health = tokio::time::timeout(
            Duration::from_millis(250),
            test_router().oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            ),
        )
        .await
        .expect("local route was pinned by synchronous authority I/O")
        .unwrap();
        assert_eq!(health.status(), StatusCode::OK);

        let error = authority.await.unwrap().unwrap_err();
        assert_eq!(error.status, StatusCode::GATEWAY_TIMEOUT);
        assert!(error.message.ends_with("timeout"));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn inconclusive_authority_result_leaves_bootstrap_state_unchanged() {
        let agent_keys = Keys::generate();
        let agent_id = UserId::new(npub(&agent_keys)).unwrap();
        let (url, server) = spawn_authority_response(AuthorityTestResponse::Malformed);
        let state = test_state().with_agent_bootstrap_authorities(
            url.clone(),
            "core-token",
            url,
            "identity-token",
        );

        let response = authed_request(
            router_with_state(state.clone()),
            &agent_keys,
            "POST",
            "/_admin/personal-brain-bootstrap",
            Some("{}".to_owned()),
            TEST_NOW,
        )
        .await;

        assert_error(
            response,
            StatusCode::BAD_GATEWAY,
            "Finite Identity Agent Principal resolution authority failure:",
        )
        .await;
        assert!(
            state
                .store
                .lock()
                .unwrap()
                .list_visible_brains(&agent_id)
                .unwrap()
                .is_empty()
        );
        server.join().unwrap();
    }

    fn sqlite_test_router(path: &std::path::Path) -> Router {
        let store = BrainStore::open(path).unwrap();
        router_with_state(ServerState::new(store, TEST_BASE_URL).with_auth_clock(TEST_NOW, 60))
    }

    async fn router_with_test_org_folders(keys: &Keys) -> Router {
        let router = test_router();
        let create_brain = post_brain(
            router.clone(),
            keys,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(create_brain.status(), StatusCode::OK);
        add_test_org_folders(&router, keys).await;
        router
    }

    async fn add_test_org_folders(router: &Router, keys: &Keys) {
        for (folder_id, name, role, access) in [
            (
                "getting-started",
                "Getting Started",
                "general",
                "all_members",
            ),
            ("restricted", "Restricted", "folder", "restricted"),
        ] {
            let body = serde_json::json!({
                "folderId": folder_id,
                "name": name,
                "role": role,
                "access": access,
                "parentFolderId": null,
                "path": folder_id,
                "accessUserIds": [],
                "grants": [
                    folder_key_grant_value(
                        &format!("grant-{folder_id}-test-admin-v1"),
                        1,
                        npub(keys).as_str(),
                    )
                ],
                "accessChangeEvent": admin_event(
                    keys,
                    "acme",
                    &format!("change-create-{folder_id}-test"),
                    AdminAccessAction::SetFolderAccessMode,
                    Some(folder_id),
                    None,
                    Some(1),
                ),
            })
            .to_string();
            let response = authed_request(
                router.clone(),
                keys,
                "POST",
                "/_admin/brains/acme/folders",
                Some(body),
                TEST_NOW,
            )
            .await;
            let status = response.status();
            let text = read_text(response).await;
            assert_eq!(status, StatusCode::OK, "{text}");
        }
    }

    async fn latest_sync_sequence(router: &Router, keys: &Keys, brain_id: &str) -> u64 {
        latest_sync_sequence_at(router, keys, brain_id, TEST_NOW - 1).await
    }

    async fn latest_sync_sequence_at(
        router: &Router,
        keys: &Keys,
        brain_id: &str,
        created_at: u64,
    ) -> u64 {
        let response = authed_request(
            router.clone(),
            keys,
            "GET",
            &format!("/_admin/brains/{brain_id}/sync/bootstrap"),
            None,
            created_at,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        read_json::<SyncBootstrapResponse>(response)
            .await
            .latest_sequence
    }

    fn create_brain_body(brain_id: &str, kind: &str) -> String {
        serde_json::json!({
            "brainId": brain_id,
            "kind": kind,
            "name": "Acme"
        })
        .to_string()
    }

    async fn post_brain(
        router: Router,
        keys: &Keys,
        body: &str,
        created_at: u64,
        auth_method: Option<&str>,
        auth_path: Option<&str>,
        auth_body: Option<&[u8]>,
    ) -> axum::response::Response {
        let auth_method = auth_method.unwrap_or("POST");
        let auth_path = auth_path.unwrap_or("/_admin/brains");
        let auth_body = auth_body.unwrap_or(body.as_bytes());
        let auth = auth_header(keys, auth_method, auth_path, Some(auth_body), created_at);

        router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_admin/brains")
                    .header(AUTHORIZATION, auth)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_owned()))
                    .expect("valid create request"),
            )
            .await
            .expect("create response")
    }

    async fn post_brain_with_header(
        router: Router,
        keys: &Keys,
        body: &str,
        created_at: u64,
        header_name: &'static str,
    ) -> axum::response::Response {
        let auth = auth_header(
            keys,
            "POST",
            "/_admin/brains",
            Some(body.as_bytes()),
            created_at,
        );

        router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_admin/brains")
                    .header(header_name, auth)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_owned()))
                    .expect("valid create request"),
            )
            .await
            .expect("create response")
    }

    async fn get_metadata(
        router: Router,
        keys: &Keys,
        brain_id: &str,
        created_at: u64,
    ) -> axum::response::Response {
        let path = format!("/_admin/brains/{brain_id}/metadata");
        let auth = auth_header(keys, "GET", &path, None, created_at);
        router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&path)
                    .header(AUTHORIZATION, auth)
                    .body(Body::empty())
                    .expect("valid metadata request"),
            )
            .await
            .expect("metadata response")
    }

    async fn authed_request(
        router: Router,
        keys: &Keys,
        method: &str,
        path: &str,
        body: Option<String>,
        created_at: u64,
    ) -> axum::response::Response {
        let auth = auth_header(
            keys,
            method,
            path,
            body.as_deref().map(str::as_bytes),
            created_at,
        );
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(AUTHORIZATION, auth);
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }

        router
            .oneshot(
                request
                    .body(body.map_or_else(Body::empty, Body::from))
                    .expect("valid authed request"),
            )
            .await
            .expect("authed response")
    }

    #[derive(Debug, Clone)]
    struct RevisionFixture<'a> {
        brain_id: &'a str,
        folder_id: &'a str,
        object_id: &'a str,
        operation: FolderObjectOperation,
        revision: u64,
        base_revision: Option<u64>,
        key_version: u32,
        content: &'a str,
        nonce: u8,
        record_type: bool,
    }

    #[derive(Debug, Clone)]
    struct RevisionEventFixture<'a> {
        brain_id: &'a str,
        folder_id: &'a str,
        object_id: &'a str,
        operation: FolderObjectOperation,
        revision: u64,
        base_revision: Option<u64>,
        key_version: u32,
        envelope_json: String,
    }

    #[derive(Debug, Clone)]
    struct TombstoneFixture<'a> {
        brain_id: &'a str,
        folder_id: &'a str,
        object_id: &'a str,
        revision: u64,
        base_revision: u64,
        record_type: bool,
    }

    fn object_write_body(keys: &Keys, fixture: RevisionFixture<'_>) -> String {
        let envelope_json = object_envelope_json(
            fixture.brain_id,
            fixture.folder_id,
            fixture.object_id,
            fixture.key_version,
            fixture.content,
            fixture.nonce,
        );
        let event = revision_event_for_author(
            keys,
            npub(keys),
            RevisionEventFixture {
                brain_id: fixture.brain_id,
                folder_id: fixture.folder_id,
                object_id: fixture.object_id,
                operation: fixture.operation,
                revision: fixture.revision,
                base_revision: fixture.base_revision,
                key_version: fixture.key_version,
                envelope_json: envelope_json.clone(),
            },
        );
        let mut body = serde_json::json!({
            "baseRevision": fixture.base_revision,
            "keyVersion": fixture.key_version,
            "cipher": "AES-256-GCM",
            "ciphertext": envelope_json,
            "revisionEvent": event,
        });
        if fixture.record_type {
            body["recordType"] = serde_json::json!("folder_object_revision");
            body["folderId"] = serde_json::json!(fixture.folder_id);
            body["objectId"] = serde_json::json!(fixture.object_id);
        }
        body.to_string()
    }

    fn object_delete_body(keys: &Keys, fixture: TombstoneFixture<'_>) -> String {
        let event = tombstone_event(keys, &fixture);
        let mut body = serde_json::json!({
            "baseRevision": fixture.base_revision,
            "tombstoneEvent": event,
        });
        if fixture.record_type {
            body["recordType"] = serde_json::json!("folder_object_tombstone");
            body["folderId"] = serde_json::json!(fixture.folder_id);
            body["objectId"] = serde_json::json!(fixture.object_id);
        }
        body.to_string()
    }

    fn object_envelope_json(
        brain_id: &str,
        folder_id: &str,
        object_id: &str,
        key_version: u32,
        content: &str,
        nonce: u8,
    ) -> String {
        let key = FolderKey::from_bytes([9; 32]);
        let aad = FolderObjectAad {
            brain_id: BrainId::new(brain_id).unwrap(),
            folder_id: FolderId::new(folder_id).unwrap(),
            object_id: ObjectId::new(object_id).unwrap(),
            key_version,
        };
        encrypt_folder_object_with_nonce(&key, &aad, [nonce; 12], content.as_bytes())
            .unwrap()
            .canonical_json()
    }

    fn revision_event_for_author(
        signer_keys: &Keys,
        author_npub: String,
        fixture: RevisionEventFixture<'_>,
    ) -> Event {
        let expected = RevisionValidation {
            brain_id: BrainId::new(fixture.brain_id).unwrap(),
            folder_id: FolderId::new(fixture.folder_id).unwrap(),
            object_id: ObjectId::new(fixture.object_id).unwrap(),
            operation: fixture.operation,
            revision: fixture.revision,
            base_revision: fixture.base_revision,
            key_version: fixture.key_version,
            envelope_json: fixture.envelope_json,
            author_npub,
            created_at: test_rfc3339(),
        };
        let payload = FolderObjectRevisionPayload::new(&expected);
        sign_app_event(
            signer_keys,
            payload.canonical_json(),
            revision_tags(&expected),
        )
    }

    fn tombstone_event(keys: &Keys, fixture: &TombstoneFixture<'_>) -> Event {
        let expected = TombstoneValidation {
            brain_id: BrainId::new(fixture.brain_id).unwrap(),
            folder_id: FolderId::new(fixture.folder_id).unwrap(),
            object_id: ObjectId::new(fixture.object_id).unwrap(),
            revision: fixture.revision,
            base_revision: fixture.base_revision,
            author_npub: npub(keys),
            deleted_at: test_rfc3339(),
        };
        let payload = FolderObjectTombstonePayload::new(&expected);
        sign_app_event(keys, payload.canonical_json(), tombstone_tags(&expected))
    }

    fn revision_tags(input: &RevisionValidation) -> Vec<Vec<String>> {
        vec![
            vec![
                "d".to_owned(),
                format!(
                    "finite-folder-object-revision:{}:{}:{}:{}",
                    input.brain_id,
                    input.folder_id,
                    input.object_id.as_str(),
                    input.revision
                ),
            ],
            vec!["brain".to_owned(), input.brain_id.to_string()],
            vec!["folder".to_owned(), input.folder_id.to_string()],
            vec!["object".to_owned(), input.object_id.as_str().to_owned()],
            vec!["operation".to_owned(), input.operation.as_str().to_owned()],
            vec!["keyVersion".to_owned(), input.key_version.to_string()],
        ]
    }

    fn tombstone_tags(input: &TombstoneValidation) -> Vec<Vec<String>> {
        vec![
            vec![
                "d".to_owned(),
                format!(
                    "finite-folder-object-tombstone:{}:{}:{}:{}",
                    input.brain_id,
                    input.folder_id,
                    input.object_id.as_str(),
                    input.revision
                ),
            ],
            vec!["brain".to_owned(), input.brain_id.to_string()],
            vec!["folder".to_owned(), input.folder_id.to_string()],
            vec!["object".to_owned(), input.object_id.as_str().to_owned()],
            vec!["operation".to_owned(), "delete".to_owned()],
        ]
    }

    fn admin_event(
        keys: &Keys,
        brain_id: &str,
        change_id: &str,
        action: AdminAccessAction,
        folder_id: Option<&str>,
        target_npub: Option<&str>,
        key_version: Option<u32>,
    ) -> Event {
        let expected = AdminAccessChangeValidation {
            brain_id: BrainId::new(brain_id).unwrap(),
            change_id: change_id.to_owned(),
            action,
            admin_npub: npub(keys),
            folder_id: folder_id.map(FolderId::new).transpose().unwrap(),
            target_npub: target_npub.map(ToOwned::to_owned),
            key_version,
            note: None,
            created_at: test_rfc3339(),
        };
        let payload = AdminAccessChangePayload::new(&expected);
        sign_app_event(
            keys,
            payload.canonical_json(),
            admin_access_change_tags(&expected),
        )
    }

    fn admin_access_change_tags(input: &AdminAccessChangeValidation) -> Vec<Vec<String>> {
        let mut tags = vec![
            vec![
                "d".to_owned(),
                format!(
                    "finite-brain-admin-access-change:{}:{}",
                    input.brain_id, input.change_id
                ),
            ],
            vec!["brain".to_owned(), input.brain_id.to_string()],
            vec!["action".to_owned(), input.action.as_str().to_owned()],
        ];
        if let Some(folder_id) = &input.folder_id {
            tags.push(vec!["folder".to_owned(), folder_id.to_string()]);
        }
        if let Some(target_npub) = &input.target_npub {
            tags.push(vec![
                "p".to_owned(),
                NostrPublicKey::parse(target_npub).unwrap().to_hex(),
            ]);
        }
        if let Some(key_version) = input.key_version {
            tags.push(vec!["keyVersion".to_owned(), key_version.to_string()]);
        }
        tags
    }

    fn folder_key_grant_value(
        id: &str,
        key_version: u32,
        recipient_npub: &str,
    ) -> serde_json::Value {
        let gift_wrap = gift_wrap_event_json(recipient_npub);
        serde_json::json!({
            "id": id,
            "keyVersion": key_version,
            "recipientNpub": recipient_npub,
            "wrappedEventJson": gift_wrap,
            "createdAt": "2026-06-23T00:00:00.000Z",
        })
    }

    fn real_folder_key_grant_value(
        id: &str,
        key_version: u32,
        issuer_keys: &Keys,
        brain_id: &str,
        folder_id: &str,
        recipient_npub: &str,
        folder_key_base64: &str,
    ) -> serde_json::Value {
        let issuer_npub = npub(issuer_keys);
        let plaintext = serde_json::json!({
            "version": "finite-folder-key-grant-v1",
            "brainId": brain_id,
            "folderId": folder_id,
            "keyVersion": key_version,
            "folderKey": folder_key_base64,
            "issuerNpub": issuer_npub,
            "recipientNpub": recipient_npub,
            "issuedAt": test_rfc3339(),
        })
        .to_string();
        let recipient = NostrPublicKey::parse(recipient_npub).unwrap();
        let rumor = build_rumor(
            NostrPublicKey::from_protocol(issuer_keys.public_key()),
            Kind::ApplicationSpecificData,
            vec![
                nostr_tag(vec![
                    "d".to_owned(),
                    format!("finite-folder-key-grant:{brain_id}:{folder_id}:{key_version}"),
                ]),
                nostr_tag(vec!["brain".to_owned(), brain_id.to_owned()]),
                nostr_tag(vec!["folder".to_owned(), folder_id.to_owned()]),
                nostr_tag(vec!["keyVersion".to_owned(), key_version.to_string()]),
            ],
            plaintext,
            TEST_NOW,
        );
        let gift_wrap = wrap_rumor(issuer_keys, recipient, rumor).unwrap();
        serde_json::json!({
            "id": id,
            "keyVersion": key_version,
            "recipientNpub": recipient_npub,
            "wrappedEventJson": gift_wrap.as_json(),
            "createdAt": test_rfc3339(),
        })
    }

    fn email_bootstrap_wrapped_event_json(
        issuer_keys: &Keys,
        brain_id: &str,
        invite_unwrap_npub: &str,
        payload_json: &str,
    ) -> String {
        let recipient = NostrPublicKey::parse(invite_unwrap_npub).unwrap();
        let rumor = build_rumor(
            NostrPublicKey::from_protocol(issuer_keys.public_key()),
            Kind::ApplicationSpecificData,
            vec![
                nostr_tag(vec![
                    "d".to_owned(),
                    format!("finite-email-invite-bootstrap:{brain_id}"),
                ]),
                nostr_tag(vec!["brain".to_owned(), brain_id.to_owned()]),
            ],
            payload_json.to_owned(),
            TEST_NOW,
        );
        wrap_rumor(issuer_keys, recipient, rumor).unwrap().as_json()
    }

    fn sha256_payload_hash(value: &str) -> String {
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    }

    fn nostr_tag(parts: Vec<String>) -> Tag {
        Tag::parse(parts).unwrap()
    }

    fn open_wrapped_folder_key_grant(
        recipient_keys: &Keys,
        wrapped_event_json: &str,
    ) -> serde_json::Value {
        let event = Event::from_json(wrapped_event_json).unwrap();
        let opened = open_gift_wrap(
            recipient_keys,
            &event,
            &GiftWrapValidation::new(NostrPublicKey::from_protocol(recipient_keys.public_key())),
        )
        .unwrap();
        serde_json::from_str(&opened.rumor.content).unwrap()
    }

    fn folder_key_from_export_grant(
        recipient_keys: &Keys,
        export: &EncryptedBrainExportResponse,
        folder_id: &str,
    ) -> String {
        let recipient_npub = npub(recipient_keys);
        let grant = export
            .key_grants
            .iter()
            .find(|grant| grant.folder_id == folder_id && grant.recipient_npub == recipient_npub)
            .expect("recipient folder key grant");
        let plaintext = open_wrapped_folder_key_grant(recipient_keys, &grant.wrapped_event_json);
        assert_eq!(plaintext["folderId"].as_str(), Some(folder_id));
        plaintext["folderKey"].as_str().unwrap().to_owned()
    }

    fn open_export_object_plaintext(
        object: &EncryptedExportObjectResponse,
        folder_key_base64: &str,
    ) -> String {
        let payload: serde_json::Value =
            serde_json::from_str(object.payload_json.as_ref().unwrap()).unwrap();
        let envelope_json = payload["ciphertext"].as_str().unwrap();
        open_envelope_plaintext(
            folder_key_base64,
            &object.folder_id,
            &object.object_id,
            object.revision,
            envelope_json,
        )
    }

    fn open_sync_object_plaintext(object: &ObjectResponse, folder_key_base64: &str) -> String {
        open_envelope_plaintext(
            folder_key_base64,
            &object.folder_id,
            &object.object_id,
            object.revision,
            &object.ciphertext,
        )
    }

    fn open_envelope_plaintext(
        folder_key_base64: &str,
        folder_id: &str,
        object_id: &str,
        key_version: u64,
        envelope_json: &str,
    ) -> String {
        let key = FolderKey::from_base64(folder_key_base64).unwrap();
        let aad = FolderObjectAad {
            brain_id: BrainId::new("acme").unwrap(),
            folder_id: FolderId::new(folder_id).unwrap(),
            object_id: ObjectId::new(object_id).unwrap(),
            key_version: key_version as u32,
        };
        let envelope = EncryptedFolderObjectEnvelope::from_json(envelope_json).unwrap();
        String::from_utf8(open_folder_object(&key, &aad, &envelope).unwrap()).unwrap()
    }

    fn gift_wrap_event_json(recipient_npub: &str) -> String {
        let recipient = NostrPublicKey::parse(recipient_npub).unwrap();
        let gift_wrap = EventBuilder::new(Kind::GiftWrap, "encrypted grant placeholder")
            .tag(Tag::public_key(recipient.as_protocol()))
            .finalize(&Keys::generate())
            .unwrap();
        gift_wrap.as_json()
    }

    fn email_bootstrap_authorization_event(
        keys: &Keys,
        brain_id: &str,
        invited_email: &str,
        invite_unwrap_npub: &str,
        bootstrap_payload_hash: &str,
        expires_at: &str,
        folders: &[(&str, FolderAccessMode, u32)],
    ) -> String {
        let content = serde_json::json!({
            "version": "finite-email-invite-bootstrap-authorization-v1",
            "brainId": brain_id,
            "invitedEmail": invited_email,
            "inviteUnwrapNpub": invite_unwrap_npub,
            "bootstrapPayloadHash": bootstrap_payload_hash,
            "expiresAt": expires_at,
            "folders": folders
                .iter()
                .map(|(folder_id, access, key_version)| {
                    serde_json::json!({
                        "folderId": folder_id,
                        "access": access,
                        "keyVersion": key_version,
                    })
                })
                .collect::<Vec<_>>()
        })
        .to_string();
        sign_app_event(keys, content, Vec::new()).as_json()
    }

    fn email_bootstrap_claim_proof_event(
        keys: &Keys,
        brain_id: &str,
        invite_code: &str,
        invited_email: &str,
        claimant_npub: &str,
        bootstrap_payload_hash: &str,
        email_proof_created_at: &str,
    ) -> String {
        let content = serde_json::json!({
            "version": "finite-email-invite-bootstrap-claim-proof-v1",
            "brainId": brain_id,
            "inviteCode": invite_code,
            "invitedEmail": invited_email,
            "claimantNpub": claimant_npub,
            "bootstrapPayloadHash": bootstrap_payload_hash,
            "emailProofCreatedAt": email_proof_created_at,
        })
        .to_string();
        sign_app_event(keys, content, Vec::new()).as_json()
    }

    fn test_rfc3339() -> String {
        format_unix_timestamp(TEST_NOW).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn rotation_object_value(
        keys: &Keys,
        brain_id: &str,
        folder_id: &str,
        object_id: &str,
        revision: u64,
        base_revision: Option<u64>,
        key_version: u32,
        content: &str,
        nonce: u8,
    ) -> serde_json::Value {
        let envelope_json =
            object_envelope_json(brain_id, folder_id, object_id, key_version, content, nonce);
        let event = revision_event_for_author(
            keys,
            npub(keys),
            RevisionEventFixture {
                brain_id,
                folder_id,
                object_id,
                operation: FolderObjectOperation::Update,
                revision,
                base_revision,
                key_version,
                envelope_json: envelope_json.clone(),
            },
        );
        serde_json::json!({
            "objectId": object_id,
            "baseRevision": base_revision,
            "keyVersion": key_version,
            "cipher": "AES-256-GCM",
            "ciphertext": envelope_json,
            "revisionEvent": event,
        })
    }

    fn test_strategy_folder() -> Folder {
        Folder {
            id: FolderId::new("strategy").unwrap(),
            name: DisplayName::new("folder_name", "Strategy").unwrap(),
            role: FolderRole::Folder,
            access: FolderAccessMode::Restricted,
            parent_folder_id: None,
            path: SafeRelativePath::new("folder_path", "Strategy").unwrap(),
            current_key_version: 1,
            shared_folder_source: false,
        }
    }

    fn sign_app_event(keys: &Keys, content: String, tags: Vec<Vec<String>>) -> Event {
        let tags = tags
            .into_iter()
            .map(|tag| Tag::parse(tag).unwrap())
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::ApplicationSpecificData, content)
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(TEST_NOW))
            .finalize(keys)
            .unwrap()
    }

    fn auth_header(
        keys: &Keys,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        created_at: u64,
    ) -> String {
        let url = format!("{TEST_BASE_URL}{path}");
        let mut request = HttpAuthEventRequest::new(method, url, created_at);
        if let Some(body) = body {
            request = request.with_body(body.to_vec());
        }
        let event = sign_http_auth_event(keys, &request).unwrap();
        encode_http_auth_header(&event)
    }

    fn npub(keys: &Keys) -> String {
        NostrPublicKey::from_protocol(keys.public_key())
            .to_npub()
            .unwrap()
    }

    async fn read_json<T>(response: axum::response::Response) -> T
    where
        T: for<'de> Deserialize<'de>,
    {
        read_json_with_limit(response, 16 * 1024).await
    }

    async fn read_json_with_limit<T>(response: axum::response::Response, limit: usize) -> T
    where
        T: for<'de> Deserialize<'de>,
    {
        let body = to_bytes(response.into_body(), limit)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json response")
    }

    async fn read_text(response: axum::response::Response) -> String {
        read_text_with_limit(response, 16 * 1024).await
    }

    async fn read_text_with_limit(response: axum::response::Response, limit: usize) -> String {
        let body = to_bytes(response.into_body(), limit)
            .await
            .expect("response body");
        String::from_utf8(body.to_vec()).expect("utf8 response")
    }

    async fn assert_error(response: axum::response::Response, status: StatusCode, contains: &str) {
        assert_eq!(response.status(), status);
        let body: ApiErrorBody = read_json(response).await;
        assert!(
            body.error.contains(contains),
            "expected error containing {contains:?}, got {:?}",
            body.error
        );
    }
}
