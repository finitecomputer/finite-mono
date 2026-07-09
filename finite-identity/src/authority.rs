//! Finite Identity Authority HTTP contract and SQLite store.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailProvider {
    Resend,
    Postmark,
}

impl MailProvider {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "resend" => Some(Self::Resend),
            "postmark" => Some(Self::Postmark),
            _ => None,
        }
    }

    pub fn api_key_env_var(&self) -> &'static str {
        match self {
            Self::Resend => "RESEND_API_KEY",
            Self::Postmark => "POSTMARK_SERVER_TOKEN",
        }
    }

    fn endpoint(&self) -> &'static str {
        match self {
            Self::Resend => "https://api.resend.com/emails",
            Self::Postmark => "https://api.postmarkapp.com/email",
        }
    }

    fn auth_header(&self) -> &'static str {
        match self {
            Self::Resend => "Authorization",
            Self::Postmark => "X-Postmark-Server-Token",
        }
    }
}

pub struct HttpMailer {
    provider: MailProvider,
    api_key: String,
    from_address: String,
    agent: ureq::Agent,
}

impl HttpMailer {
    pub fn new(provider: MailProvider, api_key: String, from_address: String) -> Self {
        assert!(!api_key.is_empty());
        assert!(from_address.contains('@'));
        Self {
            provider,
            api_key,
            from_address,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .build(),
        }
    }

    fn send_payload(&self, payload: serde_json::Value) -> Result<(), String> {
        let auth_value = match self.provider {
            MailProvider::Resend => format!("Bearer {}", self.api_key),
            MailProvider::Postmark => self.api_key.clone(),
        };
        let result = self
            .agent
            .post(self.provider.endpoint())
            .set(self.provider.auth_header(), &auth_value)
            .set("Accept", "application/json")
            .send_json(payload);
        match result {
            Ok(_response) => Ok(()),
            Err(ureq::Error::Status(code, response)) => {
                let body = response
                    .into_string()
                    .unwrap_or_else(|_| "unreadable body".to_owned());
                let truncated: String = body.chars().take(500).collect();
                Err(format!("provider returned {code}: {truncated}"))
            }
            Err(error) => Err(format!("transport error: {error}")),
        }
    }
}

impl Mailer for HttpMailer {
    fn send_email_challenge(&self, email: &str, token: &str) -> Result<(), String> {
        self.send_payload(email_challenge_payload(
            self.provider,
            &self.from_address,
            email,
            token,
        ))
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

fn email_challenge_payload(
    provider: MailProvider,
    from_address: &str,
    email: &str,
    token: &str,
) -> serde_json::Value {
    let subject = email_challenge_subject();
    let text = email_challenge_text(email, token);
    match provider {
        MailProvider::Resend => serde_json::json!({
            "from": from_address,
            "to": [email],
            "subject": subject,
            "text": text,
        }),
        MailProvider::Postmark => serde_json::json!({
            "From": from_address,
            "To": email,
            "Subject": subject,
            "TextBody": text,
            "MessageStream": "outbound",
        }),
    }
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
        let existing: Option<String> = tx
            .query_row(
                "SELECT pubkey FROM vip_email_bindings WHERE email = ?1",
                params![parsed.email],
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
                params![parsed.email, parsed.localpart, parsed.domain, pubkey, now],
            )?;
        }
        tx.execute(
            "INSERT INTO native_principals (pubkey, created_at)
             VALUES (?1, ?2)
             ON CONFLICT(pubkey) DO NOTHING",
            params![pubkey, now],
        )?;
        tx.execute(
            "INSERT INTO principal_links (email, pubkey, verified_at, revoked_at)
             VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT(email) DO UPDATE SET
                pubkey = excluded.pubkey,
                verified_at = excluded.verified_at,
                revoked_at = NULL",
            params![parsed.email, pubkey, now],
        )?;
        tx.execute(
            "UPDATE email_only_principals
             SET revoked_at = COALESCE(revoked_at, ?2)
             WHERE email = ?1",
            params![parsed.email, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn verify_email_only_principal(
        &self,
        email: &str,
        pubkey: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        if !hex::is_hex32(pubkey) {
            return Err(StoreError::Validation("malformed pubkey"));
        }
        let parsed = parse_email(email).ok_or(StoreError::Validation("malformed email"))?;
        self.conn
            .lock()
            .expect("store mutex never poisoned")
            .execute(
                "INSERT INTO email_only_principals (email, pubkey, verified_at, revoked_at)
                 VALUES (?1, ?2, ?3, NULL)
                 ON CONFLICT(email, pubkey) DO UPDATE SET
                    verified_at = excluded.verified_at,
                    revoked_at = NULL",
                params![parsed.email, pubkey, now],
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
            CREATE TABLE IF NOT EXISTS principal_links (
              email TEXT PRIMARY KEY,
              pubkey TEXT NOT NULL,
              verified_at INTEGER NOT NULL,
              revoked_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS email_only_principals (
              email TEXT NOT NULL,
              pubkey TEXT NOT NULL,
              verified_at INTEGER NOT NULL,
              revoked_at INTEGER,
              PRIMARY KEY(email, pubkey)
            );
            CREATE INDEX IF NOT EXISTS email_only_principals_email
              ON email_only_principals(email);
            CREATE INDEX IF NOT EXISTS email_only_principals_pubkey
              ON email_only_principals(pubkey);
            CREATE TABLE IF NOT EXISTS email_challenges (
              token_hash TEXT PRIMARY KEY,
              email TEXT NOT NULL,
              expires_at INTEGER NOT NULL,
              used_at INTEGER,
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

    fn active_binding_pubkey(&self, email: &str) -> Result<Option<String>, StoreError> {
        self.conn
            .lock()
            .expect("store mutex never poisoned")
            .query_row(
                "SELECT pubkey FROM vip_email_bindings
                 WHERE email = ?1 AND disabled_at IS NULL",
                params![email],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
    }

    fn active_email_only_principal(&self, email: &str, pubkey: &str) -> Result<bool, StoreError> {
        let found: Option<String> = self
            .conn
            .lock()
            .expect("store mutex never poisoned")
            .query_row(
                "SELECT pubkey FROM email_only_principals
                 WHERE email = ?1 AND pubkey = ?2 AND revoked_at IS NULL",
                params![email, pubkey],
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

    fn email_only_principals_by_email(
        &self,
        email: &str,
    ) -> Result<Vec<EmailOnlyPrincipalRecord>, StoreError> {
        let conn = self.conn.lock().expect("store mutex never poisoned");
        let mut statement = conn.prepare(
            "SELECT email, pubkey, verified_at, revoked_at
             FROM email_only_principals
             WHERE email = ?1
             ORDER BY pubkey",
        )?;
        let records = statement
            .query_map(params![email], EmailOnlyPrincipalRecord::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    fn email_only_principals_by_pubkey(
        &self,
        pubkey: &str,
    ) -> Result<Vec<EmailOnlyPrincipalRecord>, StoreError> {
        let conn = self.conn.lock().expect("store mutex never poisoned");
        let mut statement = conn.prepare(
            "SELECT email, pubkey, verified_at, revoked_at
             FROM email_only_principals
             WHERE pubkey = ?1
             ORDER BY email",
        )?;
        let records = statement
            .query_map(params![pubkey], EmailOnlyPrincipalRecord::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    fn principal_link_by_email(
        &self,
        email: &str,
    ) -> Result<Option<PrincipalLinkRecord>, StoreError> {
        self.conn
            .lock()
            .expect("store mutex never poisoned")
            .query_row(
                "SELECT email, pubkey, verified_at, revoked_at
                 FROM principal_links
                 WHERE email = ?1",
                params![email],
                PrincipalLinkRecord::from_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    fn principal_links_by_pubkey(
        &self,
        pubkey: &str,
    ) -> Result<Vec<PrincipalLinkRecord>, StoreError> {
        let conn = self.conn.lock().expect("store mutex never poisoned");
        let mut statement = conn.prepare(
            "SELECT email, pubkey, verified_at, revoked_at
             FROM principal_links
             WHERE pubkey = ?1
             ORDER BY email",
        )?;
        let records = statement
            .query_map(params![pubkey], PrincipalLinkRecord::from_row)?
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
struct EmailOnlyPrincipalRecord {
    email: String,
    pubkey: String,
    verified_at: u64,
    revoked_at: Option<u64>,
}

impl EmailOnlyPrincipalRecord {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            email: row.get(0)?,
            pubkey: row.get(1)?,
            verified_at: row.get(2)?,
            revoked_at: row.get(3)?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct PrincipalLinkRecord {
    email: String,
    pubkey: String,
    verified_at: u64,
    revoked_at: Option<u64>,
}

impl PrincipalLinkRecord {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            email: row.get(0)?,
            pubkey: row.get(1)?,
            verified_at: row.get(2)?,
            revoked_at: row.get(3)?,
        })
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

pub fn router(state: AuthorityState) -> Router {
    Router::new()
        .route("/.well-known/nostr.json", get(nip05))
        .route("/api/v1/email-challenges", post(request_email_challenge))
        .route(
            "/api/v1/vip-email-bindings/redeem",
            post(redeem_vip_email_binding),
        )
        .route(
            "/api/v1/email-only-principals/redeem",
            post(redeem_email_only_principal),
        )
        .route(
            "/api/v1/principal-resolution/satisfies-grant",
            post(satisfies_grant),
        )
        .route("/api/v1/operator/inspect", post(operator_inspect))
        .route(
            "/api/v1/operator/disable-binding",
            post(operator_disable_binding),
        )
        .with_state(state)
}

async fn nip05(
    State(state): State<AuthorityState>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(name) = query.get("name") else {
        return Json(serde_json::json!({ "names": {} }));
    };
    if !valid_nip05_localpart(name) {
        return Json(serde_json::json!({ "names": {} }));
    }
    match state
        .store
        .nip05_pubkey(name, &state.config.finite_vip_domain.to_ascii_lowercase())
    {
        Ok(Some(pubkey)) => Json(serde_json::json!({ "names": { name: pubkey } })),
        Ok(None) => Json(serde_json::json!({ "names": {} })),
        Err(_) => Json(serde_json::json!({ "names": {} })),
    }
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

async fn redeem_email_only_principal(
    State(state): State<AuthorityState>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let actor = match authenticate(&state, &headers, "POST", &original_uri, Some(&body)) {
        Ok(actor) => actor,
        Err(error) => return api_error(error.status, error.code),
    };
    let request: EmailOnlyRedeemRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "invalid_json"),
    };
    let Some(email) = normalize_invited_email(&request.email, &state.config.finite_vip_domain)
    else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_invited_email");
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
    match state.store.verify_email_only_principal(&email, &actor, now) {
        Ok(()) => Json(EmailOnlyRedeemResponse {
            email: email.clone(),
            pubkey: actor.clone(),
            principal: PrincipalResponse {
                kind: "email_only",
                pubkey: actor,
                email: Some(email),
            },
        })
        .into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error)),
    }
}

async fn satisfies_grant(
    State(state): State<AuthorityState>,
    Json(request): Json<SatisfiesGrantRequest>,
) -> impl IntoResponse {
    if !hex::is_hex32(&request.actor_pubkey) {
        return api_error(StatusCode::BAD_REQUEST, "invalid_actor_pubkey");
    }
    let resolved = resolve_grant(&state, &request.grant, &request.actor_pubkey)
        .ok()
        .flatten();
    let satisfied = resolved.is_some();
    let principal = match resolved {
        Some(ResolvedPrincipal::Native { pubkey }) => Some(PrincipalResponse {
            kind: "native",
            pubkey,
            email: None,
        }),
        Some(ResolvedPrincipal::EmailOnly { email, pubkey }) => Some(PrincipalResponse {
            kind: "email_only",
            pubkey,
            email: Some(email),
        }),
        None => None,
    };
    Json(SatisfiesGrantResponse {
        satisfied,
        principal,
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
        if let Some(binding) = match state.store.vip_binding_by_email(&normalized) {
            Ok(binding) => binding,
            Err(error) => {
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
            }
        } {
            let nip05 = if binding.disabled() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(binding.email.clone())
            };
            let principal_link = match state.store.principal_link_by_email(&normalized) {
                Ok(link) => link,
                Err(error) => {
                    return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
                }
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
                "principal_link": principal_link,
                "email_challenges": email_challenges,
            }))
            .into_response();
        }
        let email_only = match state.store.email_only_principals_by_email(&normalized) {
            Ok(records) => records,
            Err(error) => {
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
            }
        };
        if email_only.is_empty() {
            return api_error(StatusCode::NOT_FOUND, "principal_not_found");
        }
        return Json(serde_json::json!({
            "kind": "email_only",
            "email": normalized,
            "principals": email_only,
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
    let email_only_emails = match state.store.email_only_principals_by_pubkey(&pubkey) {
        Ok(records) => records,
        Err(error) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
        }
    };
    let principal_links = match state.store.principal_links_by_pubkey(&pubkey) {
        Ok(records) => records,
        Err(error) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, store_error_code(&error));
        }
    };
    if vip_emails.is_empty() && email_only_emails.is_empty() {
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
        "email_only_emails": email_only_emails,
        "principal_links": principal_links,
    }))
    .into_response()
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

fn resolve_grant(
    state: &AuthorityState,
    grant: &str,
    actor_pubkey: &str,
) -> Result<Option<ResolvedPrincipal>, StoreError> {
    let trimmed = grant.trim();
    if let Ok(pubkey) = npub::decode(trimmed) {
        if hex::encode(&pubkey) == actor_pubkey {
            return Ok(Some(ResolvedPrincipal::Native {
                pubkey: actor_pubkey.to_owned(),
            }));
        }
        return Ok(None);
    }
    if hex::is_hex32(trimmed) {
        if trimmed.eq_ignore_ascii_case(actor_pubkey) {
            return Ok(Some(ResolvedPrincipal::Native {
                pubkey: actor_pubkey.to_owned(),
            }));
        }
        return Ok(None);
    }
    let Some(email) = parse_email(trimmed) else {
        return Ok(None);
    };
    let active_binding = if email.domain == state.config.finite_vip_domain.to_ascii_lowercase() {
        state.store.active_binding_pubkey(&email.email)?
    } else {
        None
    };
    if let Some(pubkey) = active_binding {
        if pubkey == actor_pubkey {
            return Ok(Some(ResolvedPrincipal::Native { pubkey }));
        }
        return Ok(None);
    }
    if state
        .store
        .active_email_only_principal(&email.email, actor_pubkey)?
    {
        return Ok(Some(ResolvedPrincipal::EmailOnly {
            email: email.email,
            pubkey: actor_pubkey.to_owned(),
        }));
    }
    Ok(None)
}

enum ResolvedPrincipal {
    Native { pubkey: String },
    EmailOnly { email: String, pubkey: String },
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

#[derive(Debug, Serialize)]
struct VipEmailRedeemResponse {
    email: String,
    pubkey: String,
    nip05: String,
}

#[derive(Debug, Deserialize)]
struct EmailOnlyRedeemRequest {
    email: String,
    token: String,
}

#[derive(Debug, Serialize)]
struct EmailOnlyRedeemResponse {
    email: String,
    pubkey: String,
    principal: PrincipalResponse,
}

#[derive(Debug, Deserialize)]
struct SatisfiesGrantRequest {
    grant: String,
    actor_pubkey: String,
}

#[derive(Debug, Serialize)]
struct SatisfiesGrantResponse {
    satisfied: bool,
    principal: Option<PrincipalResponse>,
}

#[derive(Debug, Serialize)]
struct PrincipalResponse {
    kind: &'static str,
    pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
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
    fn mail_provider_parsing_and_env_names_are_stable() {
        assert_eq!(MailProvider::parse("resend"), Some(MailProvider::Resend));
        assert_eq!(
            MailProvider::parse("postmark"),
            Some(MailProvider::Postmark)
        );
        assert_eq!(MailProvider::parse("smtp"), None);
        assert_eq!(MailProvider::Resend.api_key_env_var(), "RESEND_API_KEY");
        assert_eq!(
            MailProvider::Postmark.api_key_env_var(),
            "POSTMARK_SERVER_TOKEN"
        );
    }

    #[test]
    fn email_challenge_payloads_match_provider_shapes() {
        let resend = email_challenge_payload(
            MailProvider::Resend,
            "Finite <identity@finite.chat>",
            "paul@finite.vip",
            "token-123",
        );
        assert_eq!(resend["from"], "Finite <identity@finite.chat>");
        assert_eq!(resend["to"][0], "paul@finite.vip");
        assert_eq!(resend["subject"], email_challenge_subject());
        assert!(resend["text"].as_str().unwrap().contains("token-123"));
        assert!(resend["text"].as_str().unwrap().contains("paul@finite.vip"));

        let postmark = email_challenge_payload(
            MailProvider::Postmark,
            "Finite <identity@finite.chat>",
            "paul@finite.vip",
            "token-123",
        );
        assert_eq!(postmark["From"], "Finite <identity@finite.chat>");
        assert_eq!(postmark["To"], "paul@finite.vip");
        assert_eq!(postmark["Subject"], email_challenge_subject());
        assert_eq!(postmark["MessageStream"], "outbound");
        assert!(postmark["TextBody"].as_str().unwrap().contains("token-123"));
    }
}
