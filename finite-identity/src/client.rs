//! Reusable client-side Identity Authority flows for Finite products.
//!
//! These helpers deliberately do not own an HTTP stack. Product CLIs can use
//! their existing transport while sharing the body shapes, Local Identity Key
//! loading, NIP-98 request signing, and authority error classification.

use serde_json::Value;

use crate::{Error, FiniteIdentity, IdentityPaths, ImportSecret, nip98};

#[derive(Debug, Clone)]
pub struct LocalIdentityKey {
    secret_key: [u8; 32],
    pubkey: String,
}

impl LocalIdentityKey {
    pub fn load_or_generate(paths: &IdentityPaths, created_by: &str) -> Result<Self, ClientError> {
        let identity = FiniteIdentity::load_or_generate(paths, created_by)?;
        Ok(Self::from_identity(&identity))
    }

    pub fn import(
        paths: &IdentityPaths,
        secret: ImportSecret,
        created_by: &str,
    ) -> Result<Self, ClientError> {
        let identity = FiniteIdentity::import(paths, secret, created_by)?;
        Ok(Self::from_identity(&identity))
    }

    pub fn from_identity(identity: &FiniteIdentity) -> Self {
        Self {
            secret_key: identity.expose_secret_bytes(),
            pubkey: identity.public_key_hex().to_owned(),
        }
    }

    pub fn from_secret(secret_key: [u8; 32]) -> Result<Self, ClientError> {
        let pubkey = nip98::pubkey_for_secret(&secret_key)?;
        Ok(Self { secret_key, pubkey })
    }

    pub fn pubkey(&self) -> &str {
        &self.pubkey
    }
}

#[derive(Debug, Clone)]
pub struct IdentityClient {
    base_url: String,
}

impl IdentityClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    pub fn email_challenge_body(&self, email: &str) -> Result<Value, ClientError> {
        Ok(serde_json::json!({ "email": email }))
    }

    pub fn vip_email_binding_redeem(
        &self,
        key: &LocalIdentityKey,
        email: &str,
        token: &str,
        now: u64,
    ) -> Result<SignedJsonRequest, ClientError> {
        self.signed_json(
            key,
            "POST",
            "/api/v1/vip-email-bindings/redeem",
            serde_json::json!({ "email": email, "token": token }),
            now,
        )
    }

    pub fn email_only_redeem(
        &self,
        key: &LocalIdentityKey,
        email: &str,
        token: &str,
        now: u64,
    ) -> Result<SignedJsonRequest, ClientError> {
        self.signed_json(
            key,
            "POST",
            "/api/v1/email-only-principals/redeem",
            serde_json::json!({ "email": email, "token": token }),
            now,
        )
    }

    pub fn classify_authority_error(status: u16, code: &str) -> Option<AuthorityError> {
        if status < 400 {
            return None;
        }
        let kind = match code {
            "not_bound" | "principal_not_found" => AuthorityErrorKind::NotBound,
            "vip_email_already_bound" => AuthorityErrorKind::AlreadyBoundToDifferentKey,
            "unknown_or_expired_email_challenge" | "email_challenge_mismatch" => {
                AuthorityErrorKind::ExpiredOrReusedToken
            }
            "missing_authorization" | "malformed_authorization" | "nip98_rejected" => {
                AuthorityErrorKind::MissingOrInvalidNip98
            }
            "unsupported_recovery" => AuthorityErrorKind::UnsupportedRecovery,
            _ if status == 501 => AuthorityErrorKind::UnsupportedRecovery,
            _ => AuthorityErrorKind::Other,
        };
        Some(AuthorityError {
            status,
            code: code.to_owned(),
            kind,
        })
    }

    fn signed_json(
        &self,
        key: &LocalIdentityKey,
        method: &str,
        path: &str,
        body: Value,
        now: u64,
    ) -> Result<SignedJsonRequest, ClientError> {
        let body = serde_json::to_vec(&body)?;
        let authorization = nip98::build_auth_header(
            &key.secret_key,
            &format!("{}{}", self.base_url, path),
            method,
            Some(&body),
            now,
        )?;
        Ok(SignedJsonRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            body,
            authorization,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SignedJsonRequest {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
    pub authorization: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityErrorKind {
    NotBound,
    AlreadyBoundToDifferentKey,
    ExpiredOrReusedToken,
    MissingOrInvalidNip98,
    UnsupportedRecovery,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityError {
    status: u16,
    code: String,
    kind: AuthorityErrorKind,
}

impl AuthorityError {
    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn kind(&self) -> AuthorityErrorKind {
        self.kind
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Identity(#[from] Error),
    #[error(transparent)]
    Nip98(#[from] nip98::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
