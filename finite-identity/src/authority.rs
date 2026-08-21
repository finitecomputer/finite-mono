//! Finite Identity Directory HTTP contract and SQLite store.
//!
//! The Directory is the shrunken Identity Authority: it answers "what npub is
//! `name@finite.vip`?" and lets a human claim (or an operator disable) a name.
//! The Email Challenge survives only as the proof-of-control for claiming a
//! name; it proves nothing to any other product.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{OriginalUri, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::{Connection, OptionalExtension, params};
use secp256k1::rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{hex, nip98, npub};
use finite_mail::MailTransport as _;

#[derive(Debug, Clone)]
pub struct AuthorityConfig {
    pub external_base_url: String,
    pub finite_vip_domain: String,
    pub email_challenge_ttl_seconds: u64,
    pub operator_token: Option<String>,
}

impl AuthorityConfig {
    fn normalized_base_url(&self) -> String {
        self.external_base_url.trim_end_matches('/').to_owned()
    }
}

pub trait Mailer: Send + Sync + 'static {
    fn send_email_challenge(&self, email: &str, token: &str) -> Result<(), String>;
}

#[derive(Debug, Clone, Default)]
pub struct DevMailer;

impl Mailer for DevMailer {
    fn send_email_challenge(&self, email: &str, token: &str) -> Result<(), String> {
        eprintln!("finite-identityd dev email challenge for {email}: {token}");
        Ok(())
    }
}

/// Production mail delivery through the shared `finite-mail` Resend
/// transport. The API key comes from `RESEND_API_KEY`; delivery is an
/// adapter (ADR 0012), identity semantics stay in this crate.
pub struct HttpMailer {
    resend: finite_mail::ResendMailer,
}

impl HttpMailer {
    pub fn new(api_key: String, from_address: String) -> Self {
        Self {
            resend: finite_mail::ResendMailer::new(api_key, from_address),
        }
    }
}

impl Mailer for HttpMailer {
    fn send_email_challenge(&self, email: &str, token: &str) -> Result<(), String> {
        self.resend
            .send_text_email(&finite_mail::TextEmail {
                to: email,
                subject: email_challenge_subject(),
                text: &email_challenge_text(email, token),
            })
            .map_err(|error| error.to_string())
    }
}

fn email_challenge_subject() -> &'static str {
    "Your Finite Identity email challenge"
}

fn email_challenge_text(email: &str, token: &str) -> String {
    format!(
        "Use this token to verify {email} with the Finite tool that requested it:\n\n\
         {token}\n\n\
         The token works once and expires in 15 minutes. If you did not \
         request it, you can ignore this email.\n"
    )
}

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> u64;
}

#[derive(Debug, Clone, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        assert!(now > 0);
        now as u64
    }
}

#[derive(Debug, Clone)]
pub struct FixedClock {
    now: Arc<AtomicU64>,
}

impl FixedClock {
    pub fn new(now: u64) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(now)),
        }
    }

    pub fn set(&self, now: u64) {
        self.now.store(now, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub struct AuthorityState {
    store: IdentityStore,
    mailer: Arc<dyn Mailer>,
    clock: Arc<dyn Clock>,
    config: AuthorityConfig,
}

impl AuthorityState {
    pub fn new(
        store: IdentityStore,
        mailer: Arc<dyn Mailer>,
        clock: impl Clock,
        config: AuthorityConfig,
    ) -> Self {
        Self {
            store,
            mailer,
            clock: Arc::new(clock),
            config,
        }
    }
}

#[derive(Clone)]
pub struct IdentityStore {
    conn: Arc<Mutex<Connection>>,
}

impl IdentityStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(StoreError::Io)?;
        }
        let conn = Connection::open(path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self, StoreError> {
        let store = Self {
            conn: Arc::new(Mutex::new(Connection::open_in_memory()?)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn bind_vip_email(&self, email: &str, pubkey: &str, now: u64) -> Result<(), StoreError> {
        if !hex::is_hex32(pubkey) {
            return Err(StoreError::Validation("malformed pubkey"));
        }
        let parsed = parse_email(email).ok_or(StoreError::Validation("malformed email"))?;
        let mut conn = self.conn.lock().expect("store mutex never poisoned");
        let tx = conn.transaction()?;
        Self::bind_vip_email_in_transaction(&tx, &parsed, pubkey, now)?;
        tx.commit()?;
        Ok(())
    }

    fn bind_vip_email_in_transaction(
        tx: &rusqlite::Transaction<'_>,
        parsed: &ParsedEmail,
        pubkey: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        let existing: Option<String> = tx
            .query_row(
                "SELECT pubkey FROM vip_email_bindings WHERE email = ?1",
                params![&parsed.email],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing_pubkey) = existing {
            if existing_pubkey != pubkey {
                return Err(StoreError::Conflict("vip_email_already_bound"));
            }
        } else {
            tx.execute(
                "INSERT INTO vip_email_bindings
                   (email, localpart, domain, pubkey, created_at, disabled_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                params![
                    &parsed.email,
                    &parsed.localpart,
                    &parsed.domain,
                    pubkey,
                    now
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO native_principals (pubkey, created_at)
             VALUES (?1, ?2)
             ON CONFLICT(pubkey) DO NOTHING",
            params![pubkey, now],
        )?;
        Ok(())
    }

    pub fn disable_vip_email(&self, email: &str, now: u64) -> Result<(), StoreError> {
        let parsed = parse_email(email).ok_or(StoreError::Validation("malformed email"))?;
        self.conn
            .lock()
            .expect("store mutex never poisoned")
            .execute(
                "UPDATE vip_email_bindings
             SET disabled_at = COALESCE(disabled_at, ?2)
             WHERE email = ?1",
                params![parsed.email, now],
            )?;
        Ok(())
    }

    fn migrate(&self) -> Result<(), StoreError> {
        // Plain exec schema: tables are created if missing and never altered.
        // The directory shrink retired `principal_links`,
        // `workos_account_principals`, `email_only_principals`,
        // `mailbox_proofs`, and `notification_deliveries`. Databases created
        // before the shrink keep those tables on disk, unread and unwritten:
        // with no migration mechanism in this codebase there is nothing to
        // drop them with, and the rows they hold are no longer load-bearing
        // for any product.
        self.conn
            .lock()
            .expect("store mutex never poisoned")
            .execute_batch(
                "
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS native_principals (
              pubkey TEXT PRIMARY KEY,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS vip_email_bindings (
              email TEXT PRIMARY KEY,
              localpart TEXT NOT NULL,
              domain TEXT NOT NULL,
              pubkey TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              disabled_at INTEGER
            );
            CREATE UNIQUE INDEX IF NOT EXISTS vip_email_bindings_name
              ON vip_email_bindings(localpart, domain);
            CREATE TABLE IF NOT EXISTS email_challenges (
              token_hash TEXT PRIMARY KEY,
              email TEXT NOT NULL,
              expires_at INTEGER NOT NULL,
              used_at INTEGER,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS managed_agent_nip05_bindings (
              name TEXT PRIMARY KEY,
              pubkey TEXT NOT NULL,
              created_at INTEGER NOT NULL
            );
            ",
            )?;
        Ok(())
    }

    fn create_email_challenge(
        &self,
        email: &str,
        token_hash: &str,
        expires_at: u64,
        now: u64,
    ) -> Result<(), StoreError> {
        self.conn
            .lock()
            .expect("store mutex never poisoned")
            .execute(
                "INSERT INTO email_challenges (token_hash, email, expires_at, used_at, created_at)
             VALUES (?1, ?2, ?3, NULL, ?4)",
                params![token_hash, email, expires_at, now],
            )?;
        Ok(())
    }

    fn redeem_email_challenge(&self, token_hash: &str, now: u64) -> Result<String, StoreError> {
        let mut conn = self.conn.lock().expect("store mutex never poisoned");
        let tx = conn.transaction()?;
        let row: Option<(String, u64, Option<u64>)> = tx
            .query_row(
                "SELECT email, expires_at, used_at
                 FROM email_challenges
                 WHERE token_hash = ?1",
                params![token_hash],
                |row| Ok((row.get(0)?, row.get::<_, u64>(1)?, row.get(2)?)),
            )
            .optional()?;
        let (email, expires_at, used_at) =
            row.ok_or(StoreError::Validation("unknown_or_expired_email_challenge"))?;
        if used_at.is_some() || now > expires_at {
            return Err(StoreError::Validation("unknown_or_expired_email_challenge"));
        }
        tx.execute(
            "UPDATE email_challenges SET used_at = ?1 WHERE token_hash = ?2",
            params![now, token_hash],
        )?;
        tx.commit()?;
        Ok(email)
    }

    fn nip05_pubkey(&self, localpart: &str, domain: &str) -> Result<Option<String>, StoreError> {
        self.conn
            .lock()
            .expect("store mutex never poisoned")
            .query_row(
                "SELECT pubkey FROM vip_email_bindings
                 WHERE localpart = ?1 AND domain = ?2 AND disabled_at IS NULL",
                params![localpart, domain],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
    }

    fn bind_managed_agent_nip05(
        &self,
        name: &str,
        pubkey: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        if !hex::is_hex32(pubkey) {
            return Err(StoreError::Validation("malformed pubkey"));
        }
        let parsed = parse_email(name).ok_or(StoreError::Validation("malformed email"))?;
        let mut conn = self.conn.lock().expect("store mutex never poisoned");
        let tx = conn.transaction()?;
        Self::bind_vip_email_in_transaction(&tx, &parsed, pubkey, now)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT pubkey FROM managed_agent_nip05_bindings WHERE name = ?1",
                params![&parsed.email],
                |row| row.get(0),
            )
            .optional()?;
        if existing
            .as_deref()
            .is_some_and(|existing| existing != pubkey)
        {
            return Err(StoreError::Conflict("managed_agent_nip05_already_bound"));
        }
        tx.execute(
            "INSERT INTO managed_agent_nip05_bindings (name, pubkey, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO NOTHING",
            params![&parsed.email, pubkey, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn is_managed_agent_nip05(&self, name: &str, pubkey: &str) -> Result<bool, StoreError> {
        let found: Option<String> = self
            .conn
            .lock()
            .expect("store mutex never poisoned")
            .query_row(
                "SELECT pubkey FROM managed_agent_nip05_bindings
                 WHERE name = ?1 AND pubkey = ?2",
                params![name, pubkey],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    fn vip_binding_by_email(
        &self,
        email: &str,
    ) -> Result<Option<VipEmailBindingRecord>, StoreError> {
        self.conn
            .lock()
            .expect("store mutex never poisoned")
            .query_row(
                "SELECT email, localpart, domain, pubkey, created_at, disabled_at
                 FROM vip_email_bindings
                 WHERE email = ?1",
                params![email],
                VipEmailBindingRecord::from_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    fn vip_bindings_by_pubkey(
        &self,
        pubkey: &str,
    ) -> Result<Vec<VipEmailBindingRecord>, StoreError> {
        let conn = self.conn.lock().expect("store mutex never poisoned");
        let mut statement = conn.prepare(
            "SELECT email, localpart, domain, pubkey, created_at, disabled_at
             FROM vip_email_bindings
             WHERE pubkey = ?1
             ORDER BY email",
        )?;
        let records = statement
            .query_map(params![pubkey], VipEmailBindingRecord::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    fn email_challenge_audits_by_email(
        &self,
        email: &str,
    ) -> Result<Vec<EmailChallengeAuditRecord>, StoreError> {
        let conn = self.conn.lock().expect("store mutex never poisoned");
        let mut statement = conn.prepare(
            "SELECT email, expires_at, used_at, created_at
             FROM email_challenges
             WHERE email = ?1
             ORDER BY created_at DESC
             LIMIT 20",
        )?;
        let records = statement
            .query_map(params![email], EmailChallengeAuditRecord::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }
}

#[derive(Debug, Clone, Serialize)]
struct VipEmailBindingRecord {
    email: String,
    localpart: String,
    domain: String,
    pubkey: String,
    created_at: u64,
    disabled_at: Option<u64>,
}

impl VipEmailBindingRecord {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            email: row.get(0)?,
            localpart: row.get(1)?,
            domain: row.get(2)?,
            pubkey: row.get(3)?,
            created_at: row.get(4)?,
            disabled_at: row.get(5)?,
        })
    }

    fn disabled(&self) -> bool {
        self.disabled_at.is_some()
    }
}

#[derive(Debug, Clone, Serialize)]
struct EmailChallengeAuditRecord {
    email: String,
    expires_at: u64,
    used_at: Option<u64>,
    created_at: u64,
}

impl EmailChallengeAuditRecord {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            email: row.get(0)?,
            expires_at: row.get(1)?,
            used_at: row.get(2)?,
            created_at: row.get(3)?,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(std::io::Error),
    #[error("validation error: {0}")]
    Validation(&'static str),
    #[error("conflict: {0}")]
    Conflict(&'static str),
}

/// Routes every public caller may reach. Each either returns public data or
/// authenticates its caller (Email Challenge token or NIP-98 signature).
fn public_routes() -> Router<AuthorityState> {
    Router::new()
        .route("/health", get(health))
        .route("/.well-known/nostr.json", get(nip05))
        .route("/api/v1/email-challenges", post(request_email_challenge))
        .route(
            "/api/v1/vip-email-bindings/redeem",
            post(redeem_vip_email_binding),
        )
        .route("/api/v1/nip05-resolution", post(resolve_nip05))
}

/// The service-owned public route surface. The edge proxies this router
/// verbatim and keeps no route list of its own; adding a route here is the
/// only way to expose one publicly.
pub fn public_router(state: AuthorityState) -> Router {
    public_routes().with_state(state)
}

/// The full loopback router: the public surface plus the operator routes
/// trusted services reach by network position.
pub fn router(state: AuthorityState) -> Router {
    public_routes()
        .route("/api/v1/operator/inspect", post(operator_inspect))
        .route(
            "/api/v1/operator/agent-email-bindings",
            post(operator_bind_agent_email),
        )
        .route(
            "/api/v1/operator/disable-binding",
            post(operator_disable_binding),
        )
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "service": "finite-identity",
        "status": "ok",
    }))
}

async fn nip05(
    State(state): State<AuthorityState>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    // NIP-05 clients fetch cross-origin. The service owns this header so the
    // edge can stay a plain reverse proxy with no per-route behavior.
    let body = match query.get("name") {
        Some(name) if valid_nip05_localpart(name) => {
            match state
                .store
                .nip05_pubkey(name, &state.config.finite_vip_domain.to_ascii_lowercase())
            {
                Ok(Some(pubkey)) => serde_json::json!({ "names": { name: pubkey } }),
                _ => serde_json::json!({ "names": {} }),
            }
        }
        _ => serde_json::json!({ "names": {} }),
    };
    ([(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")], Json(body))
}

async fn request_email_challenge(
    State(state): State<AuthorityState>,
    Json(request): Json<EmailChallengeRequest>,
) -> impl IntoResponse {
    let Some(email) = normalize_invited_email(&request.email, &state.config.finite_vip_domain)
    else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_invited_email");
    };
    let token = random_token();
    let now = state.clock.now();
    let token_hash = token_hash(&token);
    if let Err(error) = state.store.create_email_challenge(
        &email,
        &token_hash,
        now + state.config.email_challenge_ttl_seconds,
        now,
    ) {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
    }
    if state.mailer.send_email_challenge(&email, &token).is_err() {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "mail_delivery_failed");
    }
    Json(EmailChallengeResponse { email }).into_response()
}

async fn redeem_vip_email_binding(
    State(state): State<AuthorityState>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let actor = match authenticate(&state, &headers, "POST", &original_uri, Some(&body)) {
        Ok(actor) => actor,
        Err(error) => return api_error(error.status, error.code),
    };
    let request: VipEmailRedeemRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "invalid_json"),
    };
    let Some(email) = normalize_finite_vip_email(&request.email, &state.config.finite_vip_domain)
    else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_finite_vip_email");
    };
    let now = state.clock.now();
    let token_email = match state
        .store
        .redeem_email_challenge(&token_hash(&request.token), now)
    {
        Ok(token_email) => token_email,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, store_error_code(&error)),
    };
    if token_email != email {
        return api_error(StatusCode::BAD_REQUEST, "email_challenge_mismatch");
    }
    match state.store.bind_vip_email(&email, &actor, now) {
        Ok(()) => Json(VipEmailRedeemResponse {
            email: email.clone(),
            pubkey: actor,
            nip05: email,
        })
        .into_response(),
        Err(StoreError::Conflict(code)) => api_error(StatusCode::CONFLICT, code),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error)),
    }
}

async fn resolve_nip05(
    State(state): State<AuthorityState>,
    Json(request): Json<Nip05ResolutionRequest>,
) -> impl IntoResponse {
    let Some(name) = normalize_finite_vip_email(&request.name, &state.config.finite_vip_domain)
    else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_finite_nip05_name");
    };
    let binding = match state.store.vip_binding_by_email(&name) {
        Ok(Some(binding)) if !binding.disabled() => binding,
        Ok(_) => return api_error(StatusCode::NOT_FOUND, "nip05_name_not_found"),
        Err(error) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
        }
    };
    let managed_agent = match state.store.is_managed_agent_nip05(&name, &binding.pubkey) {
        Ok(managed_agent) => managed_agent,
        Err(error) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
        }
    };
    let Some(pubkey_bytes) = hex::decode32(&binding.pubkey) else {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "invalid_stored_pubkey");
    };
    Json(Nip05ResolutionResponse {
        name,
        pubkey: binding.pubkey,
        npub: npub::encode(&pubkey_bytes),
        kind: if managed_agent {
            "managed_agent"
        } else {
            "mailbox"
        },
    })
    .into_response()
}

async fn operator_inspect(
    State(state): State<AuthorityState>,
    headers: HeaderMap,
    Json(request): Json<OperatorInspectRequest>,
) -> impl IntoResponse {
    if let Err(error) = require_operator(&state, &headers) {
        return api_error(error.status, error.code);
    }
    let identifier = request.identifier.trim();
    if let Some(email) = parse_email(identifier) {
        let normalized = if email.domain == state.config.finite_vip_domain.to_ascii_lowercase() {
            let Some(email) =
                normalize_finite_vip_email(identifier, &state.config.finite_vip_domain)
            else {
                return api_error(StatusCode::BAD_REQUEST, "invalid_finite_vip_email");
            };
            email
        } else {
            email.email
        };
        let email_challenges = match state.store.email_challenge_audits_by_email(&normalized) {
            Ok(records) => records,
            Err(error) => {
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
            }
        };
        let binding = match state.store.vip_binding_by_email(&normalized) {
            Ok(binding) => binding,
            Err(error) => {
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
            }
        };
        let Some(binding) = binding else {
            return api_error(StatusCode::NOT_FOUND, "principal_not_found");
        };
        let nip05 = if binding.disabled() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(binding.email.clone())
        };
        return Json(serde_json::json!({
            "kind": "vip_email",
            "email": binding.email,
            "localpart": binding.localpart,
            "domain": binding.domain,
            "pubkey": binding.pubkey,
            "created_at": binding.created_at,
            "disabled": binding.disabled(),
            "disabled_at": binding.disabled_at,
            "nip05": nip05,
            "email_challenges": email_challenges,
        }))
        .into_response();
    }

    let pubkey = if let Ok(bytes) = npub::decode(identifier) {
        hex::encode(&bytes)
    } else if hex::is_hex32(identifier) {
        identifier.to_ascii_lowercase()
    } else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_identifier");
    };
    let vip_emails = match state.store.vip_bindings_by_pubkey(&pubkey) {
        Ok(records) => records,
        Err(error) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
        }
    };
    if vip_emails.is_empty() {
        return api_error(StatusCode::NOT_FOUND, "principal_not_found");
    }
    Json(serde_json::json!({
        "kind": "native",
        "pubkey": pubkey,
        "vip_emails": vip_emails
            .into_iter()
            .map(|binding| serde_json::json!({
                "email": binding.email,
                "localpart": binding.localpart,
                "domain": binding.domain,
                "created_at": binding.created_at,
                "disabled": binding.disabled(),
                "disabled_at": binding.disabled_at,
            }))
            .collect::<Vec<_>>(),
    }))
    .into_response()
}

async fn operator_bind_agent_email(
    State(state): State<AuthorityState>,
    headers: HeaderMap,
    Json(request): Json<OperatorBindAgentEmailRequest>,
) -> impl IntoResponse {
    if let Err(error) = require_operator(&state, &headers) {
        return api_error(error.status, error.code);
    }
    let Some(email) = normalize_finite_vip_email(&request.email, &state.config.finite_vip_domain)
    else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_finite_vip_email");
    };
    let pubkey_bytes = match npub::decode(request.agent_npub.trim()) {
        Ok(pubkey) => pubkey,
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "invalid_agent_npub"),
    };
    let pubkey = hex::encode(&pubkey_bytes);
    match state.store.vip_binding_by_email(&email) {
        Ok(Some(binding)) if binding.disabled() => {
            return api_error(StatusCode::CONFLICT, "vip_email_binding_disabled");
        }
        Ok(_) => {}
        Err(error) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
        }
    }
    match state
        .store
        .bind_managed_agent_nip05(&email, &pubkey, state.clock.now())
    {
        Ok(()) => Json(OperatorBindAgentEmailResponse {
            email: email.clone(),
            agent_npub: npub::encode(&pubkey_bytes),
            nip05: email,
        })
        .into_response(),
        Err(StoreError::Conflict(code)) => api_error(StatusCode::CONFLICT, code),
        Err(StoreError::Validation(code)) => api_error(StatusCode::BAD_REQUEST, code),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error)),
    }
}

async fn operator_disable_binding(
    State(state): State<AuthorityState>,
    headers: HeaderMap,
    Json(request): Json<OperatorDisableBindingRequest>,
) -> impl IntoResponse {
    if let Err(error) = require_operator(&state, &headers) {
        return api_error(error.status, error.code);
    }
    let Some(email) = normalize_finite_vip_email(&request.email, &state.config.finite_vip_domain)
    else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_finite_vip_email");
    };
    let binding = match state.store.vip_binding_by_email(&email) {
        Ok(binding) => binding,
        Err(error) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
        }
    };
    if binding.is_none() {
        return api_error(StatusCode::NOT_FOUND, "principal_not_found");
    }
    let now = state.clock.now();
    if let Err(error) = state.store.disable_vip_email(&email, now) {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
    }
    Json(serde_json::json!({
        "email": email,
        "disabled": true,
    }))
    .into_response()
}

fn authenticate(
    state: &AuthorityState,
    headers: &HeaderMap,
    method: &str,
    original_uri: &OriginalUri,
    body: Option<&[u8]>,
) -> Result<String, ApiFailure> {
    let Some(header_value) = headers.get(header::AUTHORIZATION) else {
        return Err(ApiFailure::new(
            StatusCode::UNAUTHORIZED,
            "missing_authorization",
        ));
    };
    let Ok(header_value) = header_value.to_str() else {
        return Err(ApiFailure::new(
            StatusCode::UNAUTHORIZED,
            "malformed_authorization",
        ));
    };
    let path_and_query = original_uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let url = format!("{}{}", state.config.normalized_base_url(), path_and_query);
    nip98::verify_auth_header(header_value, &url, method, body, state.clock.now())
        .map_err(|_| ApiFailure::new(StatusCode::UNAUTHORIZED, "nip98_rejected"))
}

fn require_operator(state: &AuthorityState, headers: &HeaderMap) -> Result<(), ApiFailure> {
    let Some(expected) = state.config.operator_token.as_deref() else {
        return Err(ApiFailure::new(
            StatusCode::UNAUTHORIZED,
            "operator_api_disabled",
        ));
    };
    let Some(actual) = headers.get("x-finite-operator-token") else {
        return Err(ApiFailure::new(
            StatusCode::UNAUTHORIZED,
            "missing_operator_token",
        ));
    };
    let Ok(actual) = actual.to_str() else {
        return Err(ApiFailure::new(
            StatusCode::UNAUTHORIZED,
            "malformed_operator_token",
        ));
    };
    if actual != expected {
        return Err(ApiFailure::new(
            StatusCode::UNAUTHORIZED,
            "invalid_operator_token",
        ));
    }
    Ok(())
}

fn api_error(status: StatusCode, code: &'static str) -> axum::response::Response {
    (
        status,
        Json(serde_json::json!({
            "error": code,
        })),
    )
        .into_response()
}

struct ApiFailure {
    status: StatusCode,
    code: &'static str,
}

impl ApiFailure {
    fn new(status: StatusCode, code: &'static str) -> Self {
        Self { status, code }
    }
}

fn store_error_code(error: &StoreError) -> &'static str {
    match error {
        StoreError::Validation(code) | StoreError::Conflict(code) => code,
        StoreError::Sqlite(_) | StoreError::Io(_) => "store_error",
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    secp256k1::rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(&bytes)
}

fn token_hash(token: &str) -> String {
    hex::encode(&Sha256::digest(token.as_bytes()))
}

fn normalize_finite_vip_email(email: &str, finite_vip_domain: &str) -> Option<String> {
    let parsed = parse_email(email)?;
    (parsed.domain == finite_vip_domain.to_ascii_lowercase()
        && valid_nip05_localpart(&parsed.localpart))
    .then_some(parsed.email)
}

fn normalize_invited_email(email: &str, finite_vip_domain: &str) -> Option<String> {
    let parsed = parse_email(email)?;
    if parsed.domain == finite_vip_domain.to_ascii_lowercase()
        && !valid_nip05_localpart(&parsed.localpart)
    {
        return None;
    }
    Some(parsed.email)
}

#[derive(Debug)]
struct ParsedEmail {
    email: String,
    localpart: String,
    domain: String,
}

fn parse_email(email: &str) -> Option<ParsedEmail> {
    let email = email.trim();
    if !email.is_ascii() {
        return None;
    }
    let email = email.to_ascii_lowercase();
    let (localpart, domain) = email.split_once('@')?;
    if localpart.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || !valid_email_localpart(localpart)
        || !valid_email_domain(domain)
    {
        return None;
    }
    let localpart = localpart.to_owned();
    let domain = domain.to_owned();
    Some(ParsedEmail {
        email,
        localpart,
        domain,
    })
}

fn valid_nip05_localpart(localpart: &str) -> bool {
    !localpart.is_empty()
        && localpart
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.'))
}

fn valid_email_localpart(localpart: &str) -> bool {
    localpart.len() <= 128
        && !localpart.starts_with('.')
        && !localpart.ends_with('.')
        && !localpart.contains("..")
        && localpart.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z'
                    | b'0'..=b'9'
                    | b'.'
                    | b'_'
                    | b'%'
                    | b'+'
                    | b'-'
            )
        })
}

fn valid_email_domain(domain: &str) -> bool {
    domain.len() <= 253
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        })
}

#[derive(Debug, Deserialize)]
struct EmailChallengeRequest {
    email: String,
}

#[derive(Debug, Serialize)]
struct EmailChallengeResponse {
    email: String,
}

#[derive(Debug, Deserialize)]
struct VipEmailRedeemRequest {
    email: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct OperatorBindAgentEmailRequest {
    email: String,
    agent_npub: String,
}

#[derive(Debug, Serialize)]
struct OperatorBindAgentEmailResponse {
    email: String,
    agent_npub: String,
    nip05: String,
}

#[derive(Debug, Serialize)]
struct VipEmailRedeemResponse {
    email: String,
    pubkey: String,
    nip05: String,
}

#[derive(Debug, Deserialize)]
struct Nip05ResolutionRequest {
    name: String,
}

#[derive(Debug, Serialize)]
struct Nip05ResolutionResponse {
    name: String,
    pubkey: String,
    npub: String,
    kind: &'static str,
}

#[derive(Debug, Deserialize)]
struct OperatorInspectRequest {
    identifier: String,
}

#[derive(Debug, Deserialize)]
struct OperatorDisableBindingRequest {
    email: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resend_api_key_env_name_is_stable() {
        // Referenced by infra env files and NixOS modules; a rename would
        // break production mail delivery at restart.
        assert_eq!(finite_mail::RESEND_API_KEY_ENV_VAR, "RESEND_API_KEY");
    }

    #[test]
    fn email_challenge_message_names_the_email_and_token() {
        let subject = email_challenge_subject();
        let text = email_challenge_text("paul@finite.vip", "token-123");
        assert_eq!(subject, "Your Finite Identity email challenge");
        assert!(text.contains("token-123"));
        assert!(text.contains("paul@finite.vip"));
        assert!(text.contains("expires in 15 minutes"));
    }

    #[test]
    fn migrate_creates_only_directory_tables() {
        // The shrink retires tables without a drop migration: fresh databases
        // must contain exactly the directory tables, and nothing else.
        let store = IdentityStore::open_memory().expect("open memory store");
        let conn = store.conn.lock().expect("store mutex never poisoned");
        let mut statement = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("prepare sqlite_master query");
        let tables = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query tables")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect tables");
        assert_eq!(
            tables,
            [
                "email_challenges",
                "managed_agent_nip05_bindings",
                "native_principals",
                "vip_email_bindings",
            ]
        );
    }
}
