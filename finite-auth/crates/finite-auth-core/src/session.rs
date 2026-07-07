use std::fmt;

use finite_nostr::NostrPublicKey;
use sha2::{Digest, Sha256};

use crate::{AuthError, Nip05Identifier};

const MIN_NONCE_LEN: usize = 16;
const MAX_NONCE_LEN: usize = 128;
const MIN_SESSION_ID_LEN: usize = 8;
const MAX_SESSION_ID_LEN: usize = 128;
const MIN_SESSION_TOKEN_LEN: usize = 32;
const MAX_SESSION_TOKEN_LEN: usize = 512;
const SESSION_HASH_HEX_LEN: usize = 64;
const MAX_HTTP_METHOD_LEN: usize = 16;
const MAX_URL_LEN: usize = 2_048;

/// Server-issued auth challenge nonce.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AuthNonce(String);

impl AuthNonce {
    /// Validate and create an auth nonce.
    pub fn new(value: impl Into<String>) -> Result<Self, AuthError> {
        validate_ascii_token("auth_nonce", value.into(), MIN_NONCE_LEN, MAX_NONCE_LEN).map(Self)
    }

    /// Borrow the nonce string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AuthNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable server-side session id.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SessionId(String);

impl SessionId {
    /// Validate and create a session id.
    pub fn new(value: impl Into<String>) -> Result<Self, AuthError> {
        validate_ascii_token(
            "session_id",
            value.into(),
            MIN_SESSION_ID_LEN,
            MAX_SESSION_ID_LEN,
        )
        .map(Self)
    }

    /// Borrow the session id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Raw bearer session token before hashing.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SessionToken(String);

impl SessionToken {
    /// Validate and create a bearer token.
    pub fn new(value: impl Into<String>) -> Result<Self, AuthError> {
        let value = value.into();
        let len = value.len();
        if !(MIN_SESSION_TOKEN_LEN..=MAX_SESSION_TOKEN_LEN).contains(&len) {
            return Err(AuthError::LimitExceeded {
                field: "session_token",
                limit: MAX_SESSION_TOKEN_LEN,
                actual: len,
            });
        }

        if !value.bytes().all(|byte| (b'!'..=b'~').contains(&byte)) {
            return Err(AuthError::InvalidInput {
                field: "session_token",
                reason: "token must be visible ASCII without whitespace".to_string(),
            });
        }

        Ok(Self(value))
    }

    /// Borrow the token.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Lowercase SHA-256 hex hash of a session token.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SessionTokenHash(String);

impl SessionTokenHash {
    /// Hash a raw bearer token for durable storage.
    pub fn from_token(token: &SessionToken) -> Self {
        let digest = Sha256::digest(token.as_str().as_bytes());
        Self(format!("{digest:x}"))
    }

    /// Parse an already-computed lowercase hex session token hash.
    pub fn parse(value: impl Into<String>) -> Result<Self, AuthError> {
        let value = value.into();
        if value.len() != SESSION_HASH_HEX_LEN
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(AuthError::InvalidInput {
                field: "session_token_hash",
                reason: "expected lowercase sha256 hex".to_string(),
            });
        }

        Ok(Self(value))
    }

    /// Borrow the lowercase hex hash.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionTokenHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bounded challenge request that can be consumed once.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AuthChallenge {
    nonce: AuthNonce,
    method: String,
    url: String,
    issued_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
    expected_public_key: Option<NostrPublicKey>,
}

impl AuthChallenge {
    /// Create a challenge with an explicit nonce, request facts, and expiry.
    pub fn new(
        nonce: AuthNonce,
        method: impl Into<String>,
        url: impl Into<String>,
        issued_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Result<Self, AuthError> {
        validate_time_window(issued_at_unix_seconds, expires_at_unix_seconds)?;
        Ok(Self {
            nonce,
            method: validate_http_method(method.into())?,
            url: validate_absolute_url("challenge_url", url.into())?,
            issued_at_unix_seconds,
            expires_at_unix_seconds,
            expected_public_key: None,
        })
    }

    /// Bind the challenge to a single expected Nostr public key.
    pub fn with_expected_public_key(mut self, public_key: NostrPublicKey) -> Self {
        self.expected_public_key = Some(public_key);
        self
    }

    /// Challenge nonce.
    pub fn nonce(&self) -> &AuthNonce {
        &self.nonce
    }

    /// Expected HTTP method.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Expected absolute URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Challenge issue time.
    pub fn issued_at_unix_seconds(&self) -> u64 {
        self.issued_at_unix_seconds
    }

    /// Challenge expiry time.
    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }

    /// Optional signer the challenge is bound to.
    pub fn expected_public_key(&self) -> Option<NostrPublicKey> {
        self.expected_public_key
    }

    /// Whether the challenge is expired at `now`.
    pub fn is_expired_at(&self, now_unix_seconds: u64) -> bool {
        now_unix_seconds >= self.expires_at_unix_seconds
    }
}

/// Authenticated Finite principal.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AuthPrincipal {
    public_key: NostrPublicKey,
    nip05: Option<Nip05Identifier>,
    authenticated_at_unix_seconds: u64,
}

impl AuthPrincipal {
    /// Create a public-key-first auth principal.
    pub fn new(public_key: NostrPublicKey, authenticated_at_unix_seconds: u64) -> Self {
        Self {
            public_key,
            nip05: None,
            authenticated_at_unix_seconds,
        }
    }

    /// Attach a current NIP-05 identification binding.
    pub fn with_nip05(mut self, nip05: Nip05Identifier) -> Self {
        self.nip05 = Some(nip05);
        self
    }

    /// Stable Nostr public key identity.
    pub fn public_key(&self) -> NostrPublicKey {
        self.public_key
    }

    /// Optional current NIP-05 identification binding.
    pub fn nip05(&self) -> Option<&Nip05Identifier> {
        self.nip05.as_ref()
    }

    /// Authentication time.
    pub fn authenticated_at_unix_seconds(&self) -> u64 {
        self.authenticated_at_unix_seconds
    }
}

/// Durable session record. The bearer token itself is never stored here.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AuthSessionRecord {
    session_id: SessionId,
    token_hash: SessionTokenHash,
    principal: AuthPrincipal,
    issued_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
    revoked_at_unix_seconds: Option<u64>,
}

impl AuthSessionRecord {
    /// Create an active session record.
    pub fn new(
        session_id: SessionId,
        token_hash: SessionTokenHash,
        principal: AuthPrincipal,
        issued_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Result<Self, AuthError> {
        validate_time_window(issued_at_unix_seconds, expires_at_unix_seconds)?;
        Ok(Self {
            session_id,
            token_hash,
            principal,
            issued_at_unix_seconds,
            expires_at_unix_seconds,
            revoked_at_unix_seconds: None,
        })
    }

    /// Return a copy with a revocation timestamp.
    pub fn revoked_at(mut self, revoked_at_unix_seconds: u64) -> Self {
        self.revoked_at_unix_seconds = Some(revoked_at_unix_seconds);
        self
    }

    /// Session id.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Session token hash.
    pub fn token_hash(&self) -> &SessionTokenHash {
        &self.token_hash
    }

    /// Authenticated principal.
    pub fn principal(&self) -> &AuthPrincipal {
        &self.principal
    }

    /// Session issue time.
    pub fn issued_at_unix_seconds(&self) -> u64 {
        self.issued_at_unix_seconds
    }

    /// Session expiry time.
    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }

    /// Session revocation time.
    pub fn revoked_at_unix_seconds(&self) -> Option<u64> {
        self.revoked_at_unix_seconds
    }

    /// Whether the session is expired at `now`.
    pub fn is_expired_at(&self, now_unix_seconds: u64) -> bool {
        now_unix_seconds >= self.expires_at_unix_seconds
    }

    /// Whether the session has been revoked.
    pub fn is_revoked(&self) -> bool {
        self.revoked_at_unix_seconds.is_some()
    }
}

pub(crate) fn validate_http_method(value: String) -> Result<String, AuthError> {
    if value.is_empty()
        || value.len() > MAX_HTTP_METHOD_LEN
        || !value.bytes().all(|byte| byte.is_ascii_uppercase())
    {
        return Err(AuthError::InvalidInput {
            field: "http_method",
            reason: "expected uppercase ASCII method".to_string(),
        });
    }

    Ok(value)
}

pub(crate) fn validate_absolute_url(
    field: &'static str,
    value: String,
) -> Result<String, AuthError> {
    if value.is_empty() || value.len() > MAX_URL_LEN {
        return Err(AuthError::LimitExceeded {
            field,
            limit: MAX_URL_LEN,
            actual: value.len(),
        });
    }

    let has_supported_scheme = value.starts_with("https://") || value.starts_with("http://");
    let has_unsafe_bytes = value.bytes().any(|byte| byte <= 0x20 || byte == 0x7f);
    if !has_supported_scheme || has_unsafe_bytes {
        return Err(AuthError::InvalidInput {
            field,
            reason: "expected absolute http or https URL without whitespace".to_string(),
        });
    }

    Ok(value)
}

fn validate_time_window(
    issued_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
) -> Result<(), AuthError> {
    if expires_at_unix_seconds <= issued_at_unix_seconds {
        return Err(AuthError::InvalidInput {
            field: "time_window",
            reason: "expires_at must be after issued_at".to_string(),
        });
    }

    Ok(())
}

fn validate_ascii_token(
    field: &'static str,
    value: String,
    min_len: usize,
    max_len: usize,
) -> Result<String, AuthError> {
    let len = value.len();
    if len < min_len || len > max_len {
        return Err(AuthError::LimitExceeded {
            field,
            limit: max_len,
            actual: len,
        });
    }

    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AuthError::InvalidInput {
            field,
            reason: "expected ASCII letters, digits, dash, underscore, or dot".to_string(),
        });
    }

    Ok(value)
}
