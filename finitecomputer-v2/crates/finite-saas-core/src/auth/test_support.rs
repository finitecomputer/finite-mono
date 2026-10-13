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

pub(crate) const BOUNDARY_RUNNER_TOKEN: &str = "runner-auth-boundary-token";

pub(crate) const FULL_RUNNER_TOKEN: &str = "runner-oslo-full-token";

pub(crate) const SECOND_RUNNER_TOKEN: &str = "runner-oslo-2-token";

pub(crate) struct TestKey {
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
                jwks: std::sync::RwLock::new(Jwks {
                    keys: vec![key().jwk.clone()],
                }),
                users: std::sync::RwLock::new(std::collections::HashMap::new()),
            })
        })
        .clone()
}

/// A private WorkOS source for tests that mutate the served JWKS (key
/// rotation, degraded endpoints). The shared `source()` registry must
/// never be mutated because crate tests run in parallel against it.
pub(crate) fn isolated_source() -> Arc<TestWorkosSource> {
    Arc::new(TestWorkosSource {
        jwks: std::sync::RwLock::new(Jwks {
            keys: vec![key().jwk.clone()],
        }),
        users: std::sync::RwLock::new(std::collections::HashMap::new()),
    })
}

pub(crate) fn standard_kid() -> String {
    key().kid.clone()
}

/// Replace the JWKS served by a test source, simulating WorkOS key
/// rotation or a degraded (empty) key set.
pub(crate) fn install_jwks_keys(source: &TestWorkosSource, keys: Vec<TestKey>) {
    *source
        .jwks
        .write()
        .expect("test JWKS registry should not be poisoned") = Jwks {
        keys: keys.into_iter().map(|key| key.jwk).collect(),
    };
}

pub(crate) fn authenticator_for_source(source: Arc<TestWorkosSource>) -> WorkosAuthenticator {
    WorkosAuthenticator::for_tests(
        WorkosAuthenticatorConfig {
            client_id: CLIENT_ID.to_string(),
            issuer: ISSUER.to_string(),
            operator_org_id: OPERATOR_ORG_ID.to_string(),
            api_key: "not-used-by-test-source".to_string(),
            api_base_url: "https://api.test.invalid".to_string(),
            jwks_url: "https://api.test.invalid/sso/jwks/client_test".to_string(),
        },
        source,
    )
    .expect("test WorkOS authenticator should be valid")
}

pub(crate) fn authenticator() -> WorkosAuthenticator {
    authenticator_for_source(source())
}

pub(crate) fn core_auth(
    service_token: impl Into<String>,
    runner_token: impl Into<String>,
    usage_token: impl Into<String>,
) -> CoreAuth {
    CoreAuth::new_with_runner_credentials(
        authenticator(),
        service_token,
        vec![
            runner_credential_config(
                "runner-oslo-1-current",
                runner_token,
                "runner-oslo-1",
                &[RunnerClass::Kata],
                "oslo-host-1",
                false,
            ),
            runner_credential_config(
                "runner-auth-boundary-current",
                BOUNDARY_RUNNER_TOKEN,
                "runner-auth-boundary",
                &[RunnerClass::Kata],
                "source-auth-boundary",
                false,
            ),
        ],
        usage_token,
    )
    .expect("test Core auth should be valid")
}

pub(crate) fn core_auth_with_runner_credentials(
    service_token: impl Into<String>,
    runner_credentials: Vec<RunnerCredentialConfig>,
    usage_token: impl Into<String>,
) -> CoreAuth {
    CoreAuth::new_with_runner_credentials(
        authenticator(),
        service_token,
        runner_credentials,
        usage_token,
    )
    .expect("test Core auth should be valid")
}

pub(crate) fn runner_credential_config(
    credential_id: impl Into<String>,
    token: impl Into<String>,
    runner_id: impl Into<String>,
    runner_classes: &[RunnerClass],
    source_host_id: impl Into<String>,
    revoked: bool,
) -> RunnerCredentialConfig {
    RunnerCredentialConfig {
        credential_id: credential_id.into(),
        token: token.into(),
        runner_id: runner_id.into(),
        runner_classes: runner_classes.to_vec(),
        source_host_id: source_host_id.into(),
        revoked,
    }
}

/// Compatibility setup for pre-boundary API behavior tests whose concern
/// is store/route behavior rather than credential separation. Boundary
/// tests use `core_auth` above with three distinct credentials.
pub(crate) fn shared_route_core_auth(route_token: impl Into<String>) -> CoreAuth {
    let route_token = route_token.into();
    let route_token_arc: Arc<str> = route_token.clone().into();
    CoreAuth {
        workos: authenticator(),
        service_api_token: route_token_arc.clone(),
        runner_credentials: vec![
            runner_credential_for_tests(
                "runner-oslo-1-compatibility",
                &route_token,
                "runner-oslo-1",
                &[RunnerClass::Kata],
                "oslo-host-1",
                true,
            ),
            runner_credential_for_tests(
                "runner-oslo-full-current",
                FULL_RUNNER_TOKEN,
                "runner-oslo-full",
                &[RunnerClass::Kata],
                "oslo-host-1",
                false,
            ),
            runner_credential_for_tests(
                "runner-oslo-2-current",
                SECOND_RUNNER_TOKEN,
                "runner-oslo-2",
                &[RunnerClass::Kata],
                "oslo-host-1",
                false,
            ),
        ]
        .into(),
        finite_private_usage_api_token: route_token_arc,
    }
}

fn runner_credential_for_tests(
    credential_id: &str,
    token: &str,
    runner_id: &str,
    runner_classes: &[RunnerClass],
    source_host_id: &str,
    legacy_kata_compatibility: bool,
) -> RunnerCredential {
    RunnerCredential {
        credential_id: credential_id.into(),
        token_digest: Sha256::digest(token.as_bytes()).into(),
        runner_id: runner_id.into(),
        runner_classes: runner_classes.into(),
        source_host_id: source_host_id.into(),
        revoked: false,
        legacy_kata_compatibility,
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

pub(crate) fn encode_claims(claims: TestClaims, key: &TestKey) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(&header, &claims, &key.encoding_key).expect("test JWT should encode")
}

/// Generate an independent RSA key for JWKS-rotation tests. Passing the
/// standard key id simulates a same-kid key replacement.
pub(crate) fn generate_key(kid: &str) -> TestKey {
    let private_key =
        RsaPrivateKey::new(&mut OsRng, 2048).expect("test RSA key generation should succeed");
    let private_pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("test RSA key should encode as PKCS#8");
    let encoding_key =
        EncodingKey::from_rsa_pem(private_pem.as_bytes()).expect("test RSA PEM should be accepted");
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
