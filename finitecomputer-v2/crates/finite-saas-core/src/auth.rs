use crate::{RunnerClass, normalize_source_host_id};
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

const DEFAULT_WORKOS_API_BASE_URL: &str = "https://api.workos.com";

const DEFAULT_WORKOS_ISSUER: &str = "https://api.workos.com";

const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(5);

const RUNNER_CREDENTIALS_ENV: &str = "FC_CORE_RUNNER_CREDENTIALS_JSON";

const RUNNER_TOKEN_ENV_PREFIX: &str = "FC_CORE_RUNNER_CREDENTIAL_TOKEN_";

const LEGACY_KATA_CREDENTIAL_ID: &str = "legacy-finite-kata-runner-1";

const LEGACY_KATA_RUNNER_ID: &str = "finite-kata-runner-1";

const LEGACY_KATA_SOURCE_HOST_ID: &str = "finite-lat-1";

/// One rotatable Runner credential definition. This deliberately does not
/// implement `Debug`: callers must never make bearer material printable.
#[derive(Clone)]
pub struct RunnerCredentialConfig {
    pub credential_id: String,
    pub token: String,
    pub runner_id: String,
    pub runner_classes: Vec<RunnerClass>,
    pub source_host_id: String,
    pub revoked: bool,
}

#[derive(Clone)]
struct RunnerCredentialCandidate {
    config: RunnerCredentialConfig,
    legacy_kata_compatibility: bool,
}

#[derive(Clone)]
struct RunnerCredential {
    credential_id: Arc<str>,
    token_digest: [u8; 32],
    runner_id: Arc<str>,
    runner_classes: Arc<[RunnerClass]>,
    source_host_id: Arc<str>,
    revoked: bool,
    legacy_kata_compatibility: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedRunnerCredential {
    pub credential_id: String,
    pub runner_id: String,
    pub runner_classes: Vec<RunnerClass>,
    pub source_host_id: String,
    pub legacy_kata_compatibility: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunnerCredentialEnvRecord {
    credential_id: String,
    token_env: String,
    runner_id: String,
    runner_classes: Vec<RunnerClass>,
    source_host_id: String,
    #[serde(default)]
    revoked: bool,
}

/// Authentication configuration for the Core HTTP boundary.
///
/// The three opaque service credentials are deliberately separate route
/// capabilities. WorkOS access tokens are never compared with any of them.
#[derive(Clone)]
pub struct CoreAuth {
    workos: WorkosAuthenticator,
    service_api_token: Arc<str>,
    runner_credentials: Arc<[RunnerCredential]>,
    finite_private_usage_api_token: Arc<str>,
}

impl CoreAuth {
    pub fn from_env() -> Result<Self, AuthConfigError> {
        let workos = WorkosAuthenticator::from_env()?;
        let mut runner_credentials = optional_env(RUNNER_CREDENTIALS_ENV)
            .map(|metadata| {
                runner_credential_configs_from_metadata(&metadata, |name| env::var(name).ok())
            })
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .map(|config| RunnerCredentialCandidate {
                config,
                legacy_kata_compatibility: false,
            })
            .collect::<Vec<_>>();
        if let Some(token) = optional_env("FC_CORE_RUNNER_API_TOKEN") {
            runner_credentials.push(legacy_kata_runner_credential(token));
        }
        Self::new_internal(
            workos,
            required_env("FC_CORE_API_TOKEN")?,
            runner_credentials,
            required_env("FC_FINITE_PRIVATE_USAGE_API_TOKEN")?,
        )
    }

    pub fn new(
        workos: WorkosAuthenticator,
        service_api_token: impl Into<String>,
        runner_api_token: impl Into<String>,
        finite_private_usage_api_token: impl Into<String>,
    ) -> Result<Self, AuthConfigError> {
        Self::new_internal(
            workos,
            service_api_token,
            vec![legacy_kata_runner_credential(runner_api_token.into())],
            finite_private_usage_api_token,
        )
    }

    pub fn new_with_runner_credentials(
        workos: WorkosAuthenticator,
        service_api_token: impl Into<String>,
        runner_credentials: Vec<RunnerCredentialConfig>,
        finite_private_usage_api_token: impl Into<String>,
    ) -> Result<Self, AuthConfigError> {
        Self::new_internal(
            workos,
            service_api_token,
            runner_credentials
                .into_iter()
                .map(|config| RunnerCredentialCandidate {
                    config,
                    legacy_kata_compatibility: false,
                })
                .collect(),
            finite_private_usage_api_token,
        )
    }

    fn new_internal(
        workos: WorkosAuthenticator,
        service_api_token: impl Into<String>,
        runner_credentials: Vec<RunnerCredentialCandidate>,
        finite_private_usage_api_token: impl Into<String>,
    ) -> Result<Self, AuthConfigError> {
        let service_api_token = required_value("FC_CORE_API_TOKEN", service_api_token.into())?;
        let finite_private_usage_api_token = required_value(
            "FC_FINITE_PRIVATE_USAGE_API_TOKEN",
            finite_private_usage_api_token.into(),
        )?;
        if runner_credentials.is_empty() {
            return Err(AuthConfigError::MissingRunnerCredentials);
        }

        let mut credential_ids = BTreeSet::new();
        if service_api_token == finite_private_usage_api_token {
            return Err(AuthConfigError::ServiceCredentialsMustBeDistinct);
        }
        let mut runner_tokens = BTreeSet::new();

        let mut validated = Vec::with_capacity(runner_credentials.len());
        for candidate in runner_credentials {
            let credential_id = candidate.config.credential_id.trim().to_string();
            let token = candidate.config.token.trim().to_string();
            let runner_id = candidate.config.runner_id.trim().to_string();
            let source_host_id = normalize_source_host_id(&candidate.config.source_host_id)
                .map_err(|_| AuthConfigError::InvalidRunnerCredentialKeyring)?;
            if credential_id.is_empty()
                || token.is_empty()
                || runner_id.is_empty()
                || candidate.config.runner_classes.is_empty()
                || !credential_ids.insert(credential_id.clone())
                || runner_classes_have_duplicates(&candidate.config.runner_classes)
            {
                return Err(AuthConfigError::InvalidRunnerCredentialKeyring);
            }
            if token == service_api_token || token == finite_private_usage_api_token {
                return Err(AuthConfigError::ServiceCredentialsMustBeDistinct);
            }
            if !runner_tokens.insert(token.clone()) {
                return Err(AuthConfigError::InvalidRunnerCredentialKeyring);
            }
            validated.push(RunnerCredential {
                credential_id: credential_id.into(),
                token_digest: Sha256::digest(token.as_bytes()).into(),
                runner_id: runner_id.into(),
                runner_classes: candidate.config.runner_classes.into(),
                source_host_id: source_host_id.into(),
                revoked: candidate.config.revoked,
                legacy_kata_compatibility: candidate.legacy_kata_compatibility,
            });
        }

        Ok(Self {
            workos,
            service_api_token: service_api_token.into(),
            runner_credentials: validated.into(),
            finite_private_usage_api_token: finite_private_usage_api_token.into(),
        })
    }

    pub fn workos(&self) -> &WorkosAuthenticator {
        &self.workos
    }

    /// Operator placement may name only a live, host-bound Kata credential.
    pub fn has_kata_host(&self, source_host_id: &str) -> bool {
        self.runner_credentials.iter().any(|credential| {
            !credential.revoked
                && !credential.legacy_kata_compatibility
                && credential.source_host_id.as_ref() == source_host_id
                && credential.runner_classes.contains(&RunnerClass::Kata)
        })
    }

    pub(crate) fn service_api_token(&self) -> &str {
        &self.service_api_token
    }

    pub(crate) fn verify_runner_credential(
        &self,
        presented_token: &str,
    ) -> Option<VerifiedRunnerCredential> {
        let presented_digest: [u8; 32] = Sha256::digest(presented_token.as_bytes()).into();
        let mut matched = None;
        for credential in self.runner_credentials.iter() {
            if bool::from(presented_digest.ct_eq(&credential.token_digest)) {
                matched = Some(credential);
            }
        }
        let credential = matched.filter(|credential| !credential.revoked)?;
        Some(VerifiedRunnerCredential {
            credential_id: credential.credential_id.to_string(),
            runner_id: credential.runner_id.to_string(),
            runner_classes: credential.runner_classes.to_vec(),
            source_host_id: credential.source_host_id.to_string(),
            legacy_kata_compatibility: credential.legacy_kata_compatibility,
        })
    }

    pub(crate) fn finite_private_usage_api_token(&self) -> &str {
        &self.finite_private_usage_api_token
    }
}

#[derive(Clone)]
pub struct WorkosAuthenticator {
    config: Arc<WorkosAuthConfig>,
    client: Client,
    jwks: Arc<RwLock<Vec<RsaJwk>>>,
    refresh_lock: Arc<Mutex<()>>,
    source: WorkosSource,
}

#[derive(Clone)]
enum WorkosSource {
    Remote,
    #[cfg(test)]
    Test(Arc<TestWorkosSource>),
}

struct WorkosAuthConfig {
    client_id: String,
    issuer: String,
    operator_org_id: String,
    api_key: String,
    api_base_url: Url,
    jwks_url: Url,
}

impl WorkosAuthenticator {
    pub fn from_env() -> Result<Self, AuthConfigError> {
        let client_id = required_env("WORKOS_CLIENT_ID")?;
        let api_base_url = optional_env("WORKOS_API_BASE_URL")
            .unwrap_or_else(|| DEFAULT_WORKOS_API_BASE_URL.to_string());
        let jwks_url = optional_env("WORKOS_JWKS_URL")
            .unwrap_or_else(|| format!("{api_base_url}/sso/jwks/{client_id}"));
        Self::new(WorkosAuthenticatorConfig {
            client_id,
            issuer: optional_env("WORKOS_ISSUER")
                .unwrap_or_else(|| DEFAULT_WORKOS_ISSUER.to_string()),
            operator_org_id: required_env("FC_WORKOS_OPERATOR_ORG_ID")?,
            api_key: required_env("WORKOS_API_KEY")?,
            api_base_url,
            jwks_url,
        })
    }

    pub fn new(config: WorkosAuthenticatorConfig) -> Result<Self, AuthConfigError> {
        let config = WorkosAuthConfig {
            client_id: required_value("WORKOS_CLIENT_ID", config.client_id)?,
            issuer: required_value("WORKOS_ISSUER", config.issuer)?,
            operator_org_id: required_value("FC_WORKOS_OPERATOR_ORG_ID", config.operator_org_id)?,
            api_key: required_value("WORKOS_API_KEY", config.api_key)?,
            api_base_url: parse_url("WORKOS_API_BASE_URL", &config.api_base_url)?,
            jwks_url: parse_url("WORKOS_JWKS_URL", &config.jwks_url)?,
        };
        let client = Client::builder()
            .timeout(DEFAULT_HTTP_TIMEOUT)
            .build()
            .map_err(|_| AuthConfigError::InvalidHttpClient)?;
        Ok(Self {
            config: Arc::new(config),
            client,
            jwks: Arc::new(RwLock::new(Vec::new())),
            refresh_lock: Arc::new(Mutex::new(())),
            source: WorkosSource::Remote,
        })
    }

    pub fn operator_org_id(&self) -> &str {
        &self.config.operator_org_id
    }

    pub async fn verify_access_token(
        &self,
        access_token: &str,
    ) -> Result<VerifiedWorkosSession, WorkosAuthError> {
        let header = decode_header(access_token).map_err(|_| {
            tracing::warn!(reason = "malformed_header", "WorkOS JWT validation failed");
            WorkosAuthError::InvalidToken
        })?;
        if header.alg != Algorithm::RS256 {
            tracing::warn!(
                reason = "unexpected_algorithm",
                "WorkOS JWT validation failed"
            );
            return Err(WorkosAuthError::InvalidToken);
        }
        let kid = header
            .kid
            .as_deref()
            .map(str::trim)
            .filter(|kid| !kid.is_empty())
            .ok_or_else(|| {
                tracing::warn!(reason = "missing_key_id", "WorkOS JWT validation failed");
                WorkosAuthError::InvalidToken
            })?;

        let key = self.key_for(kid).await.map_err(|error| {
            tracing::warn!(
                reason = if error == WorkosAuthError::InvalidToken {
                    "no_matching_jwk"
                } else {
                    "jwks_unavailable"
                },
                "WorkOS JWT validation failed"
            );
            error
        })?;
        match self.decode_with_key(access_token, &key) {
            Ok(session) => Ok(session),
            Err(DecodeFailure::InvalidSignature) => {
                // A key may rotate while retaining its id. Refresh once before
                // treating a bad signature as final.
                self.refresh_jwks().await?;
                let key = self
                    .cached_key(kid)
                    .await
                    .ok_or(WorkosAuthError::InvalidToken)?;
                self.decode_with_key(access_token, &key)
                    .map_err(|_| WorkosAuthError::InvalidToken)
            }
            Err(DecodeFailure::InvalidToken) => Err(WorkosAuthError::InvalidToken),
        }
    }

    pub async fn verified_user(
        &self,
        subject: &str,
    ) -> Result<VerifiedWorkosUser, WorkosAuthError> {
        let subject = subject.trim();
        if subject.is_empty() {
            return Err(WorkosAuthError::InvalidToken);
        }

        let user = match &self.source {
            WorkosSource::Remote => self.fetch_remote_user(subject).await?,
            #[cfg(test)]
            WorkosSource::Test(source) => source
                .users
                .read()
                .map_err(|_| WorkosAuthError::Unavailable)?
                .get(subject)
                .cloned()
                .ok_or(WorkosAuthError::UnknownUser)?,
        };
        if user.id != subject {
            return Err(WorkosAuthError::InvalidToken);
        }
        if !user.email_verified {
            return Err(WorkosAuthError::UnverifiedUser);
        }
        let email = user.email.trim();
        if email.is_empty() {
            return Err(WorkosAuthError::InvalidUser);
        }
        Ok(VerifiedWorkosUser {
            id: user.id,
            email: email.to_string(),
        })
    }

    fn decode_with_key(
        &self,
        access_token: &str,
        key: &DecodingKey,
    ) -> Result<VerifiedWorkosSession, DecodeFailure> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[self.config.issuer.as_str()]);
        validation.required_spec_claims = ["exp", "iss", "sub"]
            .into_iter()
            .map(str::to_string)
            .collect();
        validation.leeway = 0;
        // AuthKit access tokens identify the application with `client_id`,
        // not the optional OAuth `aud` claim.
        validation.validate_aud = false;

        let claims = decode::<WorkosClaims>(access_token, key, &validation)
            .map_err(|error| {
                tracing::warn!(reason = ?error.kind(), "WorkOS JWT validation failed");
                match error.kind() {
                    ErrorKind::InvalidSignature => DecodeFailure::InvalidSignature,
                    _ => DecodeFailure::InvalidToken,
                }
            })?
            .claims;
        let subject = claims.sub.trim();
        if subject.is_empty() {
            tracing::warn!(reason = "empty_subject", "WorkOS JWT validation failed");
            return Err(DecodeFailure::InvalidToken);
        }
        if claims.client_id != self.config.client_id {
            tracing::warn!(
                reason = "client_id_mismatch",
                "WorkOS JWT validation failed"
            );
            return Err(DecodeFailure::InvalidToken);
        }
        Ok(VerifiedWorkosSession {
            subject: subject.to_string(),
            organization_id: claims
                .org_id
                .map(|org_id| org_id.trim().to_string())
                .filter(|org_id| !org_id.is_empty()),
        })
    }

    async fn key_for(&self, kid: &str) -> Result<DecodingKey, WorkosAuthError> {
        if let Some(key) = self.cached_key(kid).await {
            return Ok(key);
        }
        self.refresh_jwks().await?;
        self.cached_key(kid)
            .await
            .ok_or(WorkosAuthError::InvalidToken)
    }

    async fn cached_key(&self, kid: &str) -> Option<DecodingKey> {
        self.jwks
            .read()
            .await
            .iter()
            .find(|key| key.usable_for(kid))
            .and_then(RsaJwk::decoding_key)
    }

    async fn refresh_jwks(&self) -> Result<(), WorkosAuthError> {
        let _refresh = self.refresh_lock.lock().await;
        let set = match &self.source {
            WorkosSource::Remote => {
                let response = self
                    .client
                    .get(self.config.jwks_url.clone())
                    .send()
                    .await
                    .map_err(|_| WorkosAuthError::Unavailable)?;
                if !response.status().is_success() {
                    return Err(WorkosAuthError::Unavailable);
                }
                response
                    .json::<Jwks>()
                    .await
                    .map_err(|_| WorkosAuthError::Unavailable)?
            }
            #[cfg(test)]
            WorkosSource::Test(source) => source
                .jwks
                .read()
                .map_err(|_| WorkosAuthError::Unavailable)?
                .clone(),
        };
        if set.keys.is_empty() {
            return Err(WorkosAuthError::Unavailable);
        }
        *self.jwks.write().await = set.keys;
        Ok(())
    }

    async fn fetch_remote_user(&self, subject: &str) -> Result<WorkosUser, WorkosAuthError> {
        let mut url = self.config.api_base_url.clone();
        url.path_segments_mut()
            .map_err(|_| WorkosAuthError::Unavailable)?
            .clear()
            .extend(["user_management", "users", subject]);
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(|_| WorkosAuthError::Unavailable)?;
        match response.status() {
            status if status.is_success() => response
                .json::<WorkosUser>()
                .await
                .map_err(|_| WorkosAuthError::Unavailable),
            StatusCode::NOT_FOUND => Err(WorkosAuthError::UnknownUser),
            _ => Err(WorkosAuthError::Unavailable),
        }
    }

    #[cfg(test)]
    fn for_tests(
        config: WorkosAuthenticatorConfig,
        source: Arc<TestWorkosSource>,
    ) -> Result<Self, AuthConfigError> {
        let mut authenticator = Self::new(config)?;
        authenticator.source = WorkosSource::Test(source);
        Ok(authenticator)
    }
}

#[derive(Clone)]
pub struct WorkosAuthenticatorConfig {
    pub client_id: String,
    pub issuer: String,
    pub operator_org_id: String,
    pub api_key: String,
    pub api_base_url: String,
    pub jwks_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedWorkosSession {
    pub subject: String,
    pub organization_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedWorkosUser {
    pub id: String,
    pub email: String,
}

#[derive(Debug, Deserialize)]
struct WorkosClaims {
    sub: String,
    client_id: String,
    #[serde(default)]
    org_id: Option<String>,
}

enum DecodeFailure {
    InvalidSignature,
    InvalidToken,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkosUser {
    id: String,
    email: String,
    email_verified: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct Jwks {
    keys: Vec<RsaJwk>,
}

#[derive(Debug, Clone, Deserialize)]
struct RsaJwk {
    kid: String,
    kty: String,
    #[serde(default)]
    alg: Option<String>,
    #[serde(default, rename = "use")]
    key_use: Option<String>,
    n: String,
    e: String,
}

impl RsaJwk {
    fn usable_for(&self, kid: &str) -> bool {
        self.kid == kid
            && self.kty == "RSA"
            && self.alg.as_deref().is_none_or(|alg| alg == "RS256")
            && self
                .key_use
                .as_deref()
                .is_none_or(|key_use| key_use == "sig")
    }

    fn decoding_key(&self) -> Option<DecodingKey> {
        DecodingKey::from_rsa_components(&self.n, &self.e).ok()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkosAuthError {
    #[error("invalid WorkOS access token")]
    InvalidToken,
    #[error("WorkOS identity service is unavailable")]
    Unavailable,
    #[error("WorkOS user does not exist")]
    UnknownUser,
    #[error("WorkOS user email is not verified")]
    UnverifiedUser,
    #[error("WorkOS user record is invalid")]
    InvalidUser,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),
    #[error("invalid URL in {0}")]
    InvalidUrl(&'static str),
    #[error("could not construct WorkOS HTTP client")]
    InvalidHttpClient,
    #[error("Core service credentials must be non-empty and distinct")]
    ServiceCredentialsMustBeDistinct,
    #[error("at least one bound Runner credential is required")]
    MissingRunnerCredentials,
    #[error("Runner credential keyring is invalid")]
    InvalidRunnerCredentialKeyring,
}

fn legacy_kata_runner_credential(token: String) -> RunnerCredentialCandidate {
    RunnerCredentialCandidate {
        config: RunnerCredentialConfig {
            credential_id: LEGACY_KATA_CREDENTIAL_ID.to_string(),
            token,
            runner_id: LEGACY_KATA_RUNNER_ID.to_string(),
            runner_classes: vec![RunnerClass::Kata],
            source_host_id: LEGACY_KATA_SOURCE_HOST_ID.to_string(),
            revoked: false,
        },
        legacy_kata_compatibility: true,
    }
}

fn runner_credential_configs_from_metadata(
    metadata: &str,
    token_lookup: impl Fn(&str) -> Option<String>,
) -> Result<Vec<RunnerCredentialConfig>, AuthConfigError> {
    let records = serde_json::from_str::<Vec<RunnerCredentialEnvRecord>>(metadata)
        .map_err(|_| AuthConfigError::InvalidRunnerCredentialKeyring)?;
    records
        .into_iter()
        .map(|record| {
            let token_env = record.token_env.trim();
            if !token_env.starts_with(RUNNER_TOKEN_ENV_PREFIX)
                || !token_env.chars().all(|character| {
                    character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
                })
            {
                return Err(AuthConfigError::InvalidRunnerCredentialKeyring);
            }
            let token = token_lookup(token_env)
                .and_then(|token| required_runner_token(token).ok())
                .ok_or(AuthConfigError::InvalidRunnerCredentialKeyring)?;
            Ok(RunnerCredentialConfig {
                credential_id: record.credential_id,
                token,
                runner_id: record.runner_id,
                runner_classes: record.runner_classes,
                source_host_id: record.source_host_id,
                revoked: record.revoked,
            })
        })
        .collect()
}

fn required_runner_token(value: String) -> Result<String, AuthConfigError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(AuthConfigError::InvalidRunnerCredentialKeyring)
    } else {
        Ok(value)
    }
}

fn runner_classes_have_duplicates(classes: &[RunnerClass]) -> bool {
    classes
        .iter()
        .enumerate()
        .any(|(index, class)| classes[index + 1..].contains(class))
}

fn required_env(name: &'static str) -> Result<String, AuthConfigError> {
    required_value(name, env::var(name).unwrap_or_default())
}

fn required_value(name: &'static str, value: String) -> Result<String, AuthConfigError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(AuthConfigError::Missing(name))
    } else {
        Ok(value)
    }
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_url(name: &'static str, value: &str) -> Result<Url, AuthConfigError> {
    Url::parse(value).map_err(|_| AuthConfigError::InvalidUrl(name))
}

#[cfg(test)]
pub(crate) struct TestWorkosSource {
    jwks: std::sync::RwLock<Jwks>,
    users: std::sync::RwLock<std::collections::HashMap<String, WorkosUser>>,
}
