use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

const DEFAULT_WORKOS_API_BASE_URL: &str = "https://api.workos.com";
const DEFAULT_WORKOS_ISSUER: &str = "https://api.workos.com";
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(5);

/// Authentication configuration for the Core HTTP boundary.
///
/// The three opaque service credentials are deliberately separate route
/// capabilities. WorkOS access tokens are never compared with any of them.
#[derive(Clone)]
pub struct CoreAuth {
    workos: WorkosAuthenticator,
    service_api_token: Arc<str>,
    runner_api_token: Arc<str>,
    finite_private_usage_api_token: Arc<str>,
}

impl CoreAuth {
    pub fn from_env() -> Result<Self, AuthConfigError> {
        let workos = WorkosAuthenticator::from_env()?;
        Self::new(
            workos,
            required_env("FC_CORE_API_TOKEN")?,
            required_env("FC_CORE_RUNNER_API_TOKEN")?,
            required_env("FC_FINITE_PRIVATE_USAGE_API_TOKEN")?,
        )
    }

    pub fn new(
        workos: WorkosAuthenticator,
        service_api_token: impl Into<String>,
        runner_api_token: impl Into<String>,
        finite_private_usage_api_token: impl Into<String>,
    ) -> Result<Self, AuthConfigError> {
        let service_api_token = required_value("FC_CORE_API_TOKEN", service_api_token.into())?;
        let runner_api_token = required_value("FC_CORE_RUNNER_API_TOKEN", runner_api_token.into())?;
        let finite_private_usage_api_token = required_value(
            "FC_FINITE_PRIVATE_USAGE_API_TOKEN",
            finite_private_usage_api_token.into(),
        )?;

        let distinct = [
            service_api_token.as_str(),
            runner_api_token.as_str(),
            finite_private_usage_api_token.as_str(),
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        if distinct.len() != 3 {
            return Err(AuthConfigError::ServiceCredentialsMustBeDistinct);
        }

        Ok(Self {
            workos,
            service_api_token: service_api_token.into(),
            runner_api_token: runner_api_token.into(),
            finite_private_usage_api_token: finite_private_usage_api_token.into(),
        })
    }

    pub fn workos(&self) -> &WorkosAuthenticator {
        &self.workos
    }

    pub(crate) fn service_api_token(&self) -> &str {
        &self.service_api_token
    }

    pub(crate) fn runner_api_token(&self) -> &str {
        &self.runner_api_token
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
            WorkosSource::Test(source) => source.jwks.clone(),
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
struct TestWorkosSource {
    jwks: Jwks,
    users: std::sync::RwLock<std::collections::HashMap<String, WorkosUser>>,
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use rand::rngs::OsRng;
    use rsa::RsaPrivateKey;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::traits::PublicKeyParts;
    use serde::Serialize;
    use sha2::{Digest, Sha256};
    use std::sync::OnceLock;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub(crate) const CLIENT_ID: &str = "client_test";
    pub(crate) const ISSUER: &str = "https://identity.test.invalid";
    pub(crate) const OPERATOR_ORG_ID: &str = "workos_org_internal_operator";

    struct TestKey {
        kid: String,
        encoding_key: EncodingKey,
        jwk: RsaJwk,
    }

    fn key() -> &'static TestKey {
        static KEY: OnceLock<TestKey> = OnceLock::new();
        KEY.get_or_init(|| generate_key("test-key"))
    }

    fn source() -> Arc<TestWorkosSource> {
        static SOURCE: OnceLock<Arc<TestWorkosSource>> = OnceLock::new();
        SOURCE
            .get_or_init(|| {
                Arc::new(TestWorkosSource {
                    jwks: Jwks {
                        keys: vec![key().jwk.clone()],
                    },
                    users: std::sync::RwLock::new(std::collections::HashMap::new()),
                })
            })
            .clone()
    }

    pub(crate) fn authenticator() -> WorkosAuthenticator {
        WorkosAuthenticator::for_tests(
            WorkosAuthenticatorConfig {
                client_id: CLIENT_ID.to_string(),
                issuer: ISSUER.to_string(),
                operator_org_id: OPERATOR_ORG_ID.to_string(),
                api_key: "not-used-by-test-source".to_string(),
                api_base_url: "https://api.test.invalid".to_string(),
                jwks_url: "https://api.test.invalid/sso/jwks/client_test".to_string(),
            },
            source(),
        )
        .expect("test WorkOS authenticator should be valid")
    }

    pub(crate) fn core_auth(
        service_token: impl Into<String>,
        runner_token: impl Into<String>,
        usage_token: impl Into<String>,
    ) -> CoreAuth {
        CoreAuth::new(authenticator(), service_token, runner_token, usage_token)
            .expect("test Core auth should be valid")
    }

    /// Compatibility setup for pre-boundary API behavior tests whose concern
    /// is store/route behavior rather than credential separation. Boundary
    /// tests use `core_auth` above with three distinct credentials.
    pub(crate) fn shared_route_core_auth(route_token: impl Into<String>) -> CoreAuth {
        let route_token: Arc<str> = route_token.into().into();
        CoreAuth {
            workos: authenticator(),
            service_api_token: route_token.clone(),
            runner_api_token: route_token.clone(),
            finite_private_usage_api_token: route_token,
        }
    }

    pub(crate) fn access_token(
        email: &str,
        email_verified: bool,
        organization_id: Option<&str>,
    ) -> String {
        let subject = test_subject(email, email_verified);
        access_token_with_subject(&subject, email, email_verified, organization_id)
    }

    pub(crate) fn access_token_with_subject(
        subject: &str,
        email: &str,
        email_verified: bool,
        organization_id: Option<&str>,
    ) -> String {
        let source = source();
        source
            .users
            .write()
            .expect("test WorkOS user registry should not be poisoned")
            .insert(
                subject.to_string(),
                WorkosUser {
                    id: subject.to_string(),
                    email: email.to_string(),
                    email_verified,
                },
            );
        encode_claims(TestClaims::valid(subject, organization_id), key())
    }

    fn test_subject(email: &str, verified: bool) -> String {
        let digest = Sha256::digest(email.trim().to_ascii_lowercase().as_bytes());
        let fingerprint = digest[..8]
            .iter()
            .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
        format!(
            "user_test_{}_{fingerprint:016x}",
            if verified { "verified" } else { "unverified" },
        )
    }

    #[derive(Clone, Serialize)]
    pub(crate) struct TestClaims {
        iss: String,
        sub: String,
        client_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        org_id: Option<String>,
        exp: u64,
        iat: u64,
    }

    impl TestClaims {
        pub(crate) fn valid(subject: impl Into<String>, organization_id: Option<&str>) -> Self {
            let now = now();
            Self {
                iss: ISSUER.to_string(),
                sub: subject.into(),
                client_id: CLIENT_ID.to_string(),
                org_id: organization_id.map(str::to_string),
                exp: now + 300,
                iat: now,
            }
        }

        pub(crate) fn with_issuer(mut self, issuer: &str) -> Self {
            self.iss = issuer.to_string();
            self
        }

        pub(crate) fn with_client_id(mut self, client_id: &str) -> Self {
            self.client_id = client_id.to_string();
            self
        }

        pub(crate) fn with_expiry(mut self, exp: u64) -> Self {
            self.exp = exp;
            self
        }
    }

    pub(crate) fn signed_claims(claims: TestClaims) -> String {
        encode_claims(claims, key())
    }

    pub(crate) fn invalidly_signed_claims(claims: TestClaims) -> String {
        let alternate = generate_key(&key().kid);
        encode_claims(claims, &alternate)
    }

    pub(crate) fn register_user_record(
        lookup_subject: &str,
        record_id: &str,
        email: &str,
        email_verified: bool,
    ) {
        source()
            .users
            .write()
            .expect("test WorkOS user registry should not be poisoned")
            .insert(
                lookup_subject.to_string(),
                WorkosUser {
                    id: record_id.to_string(),
                    email: email.to_string(),
                    email_verified,
                },
            );
    }

    pub(crate) fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow Unix epoch")
            .as_secs()
    }

    fn encode_claims(claims: TestClaims, key: &TestKey) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(key.kid.clone());
        encode(&header, &claims, &key.encoding_key).expect("test JWT should encode")
    }

    fn generate_key(kid: &str) -> TestKey {
        let private_key =
            RsaPrivateKey::new(&mut OsRng, 2048).expect("test RSA key generation should succeed");
        let private_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("test RSA key should encode as PKCS#8");
        let encoding_key = EncodingKey::from_rsa_pem(private_pem.as_bytes())
            .expect("test RSA PEM should be accepted");
        TestKey {
            kid: kid.to_string(),
            encoding_key,
            jwk: RsaJwk {
                kid: kid.to_string(),
                kty: "RSA".to_string(),
                alg: Some("RS256".to_string()),
                key_use: Some("sig".to_string()),
                n: URL_SAFE_NO_PAD.encode(private_key.n().to_bytes_be()),
                e: URL_SAFE_NO_PAD.encode(private_key.e().to_bytes_be()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{
        TestClaims, authenticator, invalidly_signed_claims, now, register_user_record,
        signed_claims,
    };
    use super::*;

    #[tokio::test]
    async fn accepts_valid_standard_workos_access_token() {
        let claims = TestClaims::valid("user_valid", Some(test_support::OPERATOR_ORG_ID));
        let session = authenticator()
            .verify_access_token(&signed_claims(claims))
            .await
            .unwrap();
        assert_eq!(session.subject, "user_valid");
        assert_eq!(
            session.organization_id.as_deref(),
            Some(test_support::OPERATOR_ORG_ID)
        );
    }

    #[tokio::test]
    async fn rejects_expired_access_token() {
        let claims = TestClaims::valid("user_expired", None).with_expiry(now() - 1);
        assert_eq!(
            authenticator()
                .verify_access_token(&signed_claims(claims))
                .await,
            Err(WorkosAuthError::InvalidToken)
        );
    }

    #[tokio::test]
    async fn rejects_wrong_issuer() {
        let claims = TestClaims::valid("user_wrong_issuer", None)
            .with_issuer("https://wrong-issuer.test.invalid");
        assert_eq!(
            authenticator()
                .verify_access_token(&signed_claims(claims))
                .await,
            Err(WorkosAuthError::InvalidToken)
        );
    }

    #[tokio::test]
    async fn rejects_wrong_client_id() {
        let claims = TestClaims::valid("user_wrong_client", None).with_client_id("client_wrong");
        assert_eq!(
            authenticator()
                .verify_access_token(&signed_claims(claims))
                .await,
            Err(WorkosAuthError::InvalidToken)
        );
    }

    #[tokio::test]
    async fn rejects_invalid_signature() {
        let claims = TestClaims::valid("user_invalid_signature", None);
        assert_eq!(
            authenticator()
                .verify_access_token(&invalidly_signed_claims(claims))
                .await,
            Err(WorkosAuthError::InvalidToken)
        );
    }

    #[tokio::test]
    async fn rejects_empty_subject() {
        let claims = TestClaims::valid("", None);
        assert_eq!(
            authenticator()
                .verify_access_token(&signed_claims(claims))
                .await,
            Err(WorkosAuthError::InvalidToken)
        );
    }

    #[tokio::test]
    async fn user_lookup_fails_closed_for_unknown_or_mismatched_records() {
        let authenticator = authenticator();
        let unknown = authenticator
            .verify_access_token(&signed_claims(TestClaims::valid("user_unknown", None)))
            .await
            .unwrap();
        assert_eq!(
            authenticator.verified_user(&unknown.subject).await,
            Err(WorkosAuthError::UnknownUser)
        );

        register_user_record(
            "user_expected",
            "user_different",
            "expected@finite.vip",
            true,
        );
        let mismatched = authenticator
            .verify_access_token(&signed_claims(TestClaims::valid("user_expected", None)))
            .await
            .unwrap();
        assert_eq!(
            authenticator.verified_user(&mismatched.subject).await,
            Err(WorkosAuthError::InvalidToken)
        );
    }

    #[test]
    fn service_route_credentials_must_be_distinct() {
        let result = CoreAuth::new(authenticator(), "same", "same", "usage");
        assert_eq!(
            result.err(),
            Some(AuthConfigError::ServiceCredentialsMustBeDistinct)
        );
    }
}
