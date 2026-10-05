//! Brain identity descriptions v1 (FIN-122): the wire contract, input
//! validation and deployment configuration shared by the private Core routes.
//!
//! Core describes an exact public key; Brain decides that key's access. See
//! `finitecomputer-v2/docs/brain-identity-descriptions-v1.md`.

use crate::auth::CoreAuth;
use bech32::{Bech32, Hrp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use thiserror::Error;

pub const OBSERVATION_VERSION: &str = "finite-core-brain-account-observation-v1";
pub const DESCRIPTIONS_VERSION: &str = "finite-core-brain-identity-descriptions-v1";
pub const OBSERVATION_PATH: &str = "/api/core/internal/v1/brain-account-observations";
pub const DESCRIPTIONS_PATH: &str = "/api/core/internal/v1/brain-identity-descriptions";
/// Header for the trusted dashboard's observation credential. Distinct from
/// `Authorization`, which carries the account's WorkOS bearer on that route.
pub const OBSERVATION_CREDENTIAL_HEADER: &str = "x-finite-brain-observation-credential";
/// Header for the Brain server's read-only description credential.
pub const DESCRIPTION_CREDENTIAL_HEADER: &str = "x-finite-brain-description-credential";
/// The only issuer that holds the observation credential.
pub const OBSERVATION_ISSUER: &str = "finite-dashboard";

pub const MAX_DESCRIPTION_KEYS: usize = 100;
pub const MAX_DESCRIPTION_BODY_BYTES: usize = 64 * 1024;
pub const MAX_OBSERVATION_BODY_BYTES: usize = 16 * 1024;
/// Runtime rows one description batch may read before failing closed.
pub const MAX_AGENT_RUNTIME_ROWS: i64 = 2_000;
/// Independently known human keys returned for one responsible account.
pub const MAX_RESPONSIBLE_HUMAN_KEYS: usize = 8;
/// Longest email or NIP-05 name Core puts on the wire (RFC 5321 path limit).
pub const MAX_CONTACT_BYTES: usize = 254;
pub const MAX_DISPLAY_NAME_BYTES: usize = 200;
/// Serialized description response ceiling; larger responses fail closed.
pub const MAX_DESCRIPTION_RESPONSE_BYTES: usize = 256 * 1024;
pub const DESCRIPTION_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);
pub const OBSERVATION_WINDOW_SECONDS: i64 = 60;
pub const OBSERVATION_SKEW_SECONDS: i64 = 30;

const BIND_ENV: &str = "FC_CORE_BRAIN_IDENTITY_BIND";
const BRAIN_SERVER_ENV: &str = "FC_CORE_BRAIN_IDENTITY_BRAIN_SERVER";
const OBSERVATION_TOKEN_ENV: &str = "FC_CORE_BRAIN_OBSERVATION_TOKEN";
const DESCRIPTION_TOKEN_ENV: &str = "FC_CORE_BRAIN_DESCRIPTION_TOKEN";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BrainIdentityConfigError {
    #[error(
        "set {BIND_ENV}, {BRAIN_SERVER_ENV}, {OBSERVATION_TOKEN_ENV} and {DESCRIPTION_TOKEN_ENV} together, or none of them"
    )]
    Incomplete,
    #[error(
        "{BRAIN_SERVER_ENV} must be a canonical https origin such as https://brain.example.com"
    )]
    InvalidBrainServer,
    #[error("{BIND_ENV} must be a loopback or private address with an explicit port")]
    InvalidBind,
    #[error(
        "{OBSERVATION_TOKEN_ENV} and {DESCRIPTION_TOKEN_ENV} must differ from each other and from every other Core credential"
    )]
    CredentialsMustBeDistinct,
}

/// Private Brain identity listener configuration. Absent = feature off.
#[derive(Clone)]
pub struct BrainIdentityConfig {
    pub bind: String,
    pub brain_server: String,
    observation_credential: Arc<str>,
    description_credential: Arc<str>,
}

impl std::fmt::Debug for BrainIdentityConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrainIdentityConfig")
            .field("bind", &self.bind)
            .field("brain_server", &self.brain_server)
            .finish_non_exhaustive()
    }
}

impl BrainIdentityConfig {
    pub fn from_env(auth: &CoreAuth) -> Result<Option<Self>, BrainIdentityConfigError> {
        let read = |name: &str| {
            env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        Self::from_values(
            read(BIND_ENV),
            read(BRAIN_SERVER_ENV),
            read(OBSERVATION_TOKEN_ENV),
            read(DESCRIPTION_TOKEN_ENV),
            auth,
        )
    }

    pub fn from_values(
        bind: Option<String>,
        brain_server: Option<String>,
        observation_credential: Option<String>,
        description_credential: Option<String>,
        auth: &CoreAuth,
    ) -> Result<Option<Self>, BrainIdentityConfigError> {
        match (
            bind,
            brain_server,
            observation_credential,
            description_credential,
        ) {
            (None, None, None, None) => Ok(None),
            (Some(bind), Some(brain_server), Some(observation), Some(description)) => {
                if !private_bind(&bind) {
                    return Err(BrainIdentityConfigError::InvalidBind);
                }
                if canonical_brain_server(&brain_server).as_deref() != Some(brain_server.as_str()) {
                    return Err(BrainIdentityConfigError::InvalidBrainServer);
                }
                if observation == description
                    || auth.credential_in_use(&observation)
                    || auth.credential_in_use(&description)
                {
                    return Err(BrainIdentityConfigError::CredentialsMustBeDistinct);
                }
                Ok(Some(Self {
                    bind,
                    brain_server,
                    observation_credential: observation.into(),
                    description_credential: description.into(),
                }))
            }
            _ => Err(BrainIdentityConfigError::Incomplete),
        }
    }

    pub(crate) fn observation_credential_matches(&self, presented: Option<&str>) -> bool {
        presented.is_some_and(|value| token_eq(value, &self.observation_credential))
    }

    pub(crate) fn description_credential_matches(&self, presented: Option<&str>) -> bool {
        presented.is_some_and(|value| token_eq(value, &self.description_credential))
    }
}

fn token_eq(presented: &str, expected: &str) -> bool {
    let presented = Sha256::digest(presented.trim().as_bytes());
    let expected = Sha256::digest(expected.as_bytes());
    bool::from(presented.ct_eq(&expected))
}

/// The private listener must never bind a wildcard or public address.
fn private_bind(bind: &str) -> bool {
    let Ok(address) = bind.parse::<std::net::SocketAddr>() else {
        return false;
    };
    if address.port() == 0 && !cfg!(test) {
        return false;
    }
    match address.ip() {
        std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        std::net::IpAddr::V6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
    }
}

/// The single accepted form of a Brain server identity: lowercase `https`
/// scheme and host, optional non-default port, no path, query, fragment,
/// userinfo or trailing slash. Local development and CI may use
/// `http://127.0.0.1:<port>` or `http://localhost:<port>`. Anything else is
/// rejected rather than normalised, so both writers send the identical string.
pub fn canonical_brain_server(value: &str) -> Option<String> {
    if let Some(rest) = value.strip_prefix("http://") {
        let (host, port) = rest.split_once(':')?;
        let number: u16 = port.parse().ok()?;
        return (matches!(host, "127.0.0.1" | "localhost")
            && number != 0
            && !port.starts_with('0'))
        .then(|| value.to_string());
    }
    let rest = value.strip_prefix("https://")?;
    if rest.is_empty()
        || rest.len() > 253
        || rest.chars().any(|c| {
            !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | ':'))
        })
    {
        return None;
    }
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    if host.is_empty()
        || host.starts_with(['.', '-'])
        || host.ends_with(['.', '-'])
        || host.contains("..")
    {
        return None;
    }
    if let Some(port) = port {
        let number: u16 = port.parse().ok()?;
        if number == 0 || number == 443 || port.starts_with('0') {
            return None;
        }
    }
    Some(value.to_string())
}

pub fn valid_brain_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Canonical full public key: exactly 64 lowercase hex characters.
pub fn valid_public_key_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Encode a canonical hex key as the lowercase `npub` text Core stores in
/// `agent_runtimes.health_reporting_npub`, so lookups match by equality and
/// never decode stored pins.
pub fn npub_for_hex(public_key_hex: &str) -> Option<String> {
    if !valid_public_key_hex(public_key_hex) {
        return None;
    }
    let mut bytes = [0_u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&public_key_hex[2 * index..2 * index + 2], 16).ok()?;
    }
    let hrp = Hrp::parse("npub").ok()?;
    bech32::encode::<Bech32>(hrp, &bytes).ok()
}

fn valid_operation_id(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationActionKind {
    HumanHostedAction,
    OwnedAgentHostedAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrainAccountObservationRequest {
    pub version: String,
    pub operation_id: String,
    pub brain_server: String,
    pub brain_id: String,
    pub observed_at: String,
    pub action_kind: ObservationActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_human_public_key_hex: Option<String>,
    pub participating_public_key_hex: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationOutcome {
    Recorded,
    Unchanged,
}

impl ObservationOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::Unchanged => "unchanged",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "recorded" => Some(Self::Recorded),
            "unchanged" => Some(Self::Unchanged),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainAccountObservationResponse {
    pub version: String,
    pub operation_id: String,
    pub outcome: ObservationOutcome,
}

/// Why an observation request is malformed. Never echoes contact details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ObservationInputError {
    #[error("unsupported observation version")]
    Version,
    #[error("operationId must be 16-128 characters of [A-Za-z0-9_-]")]
    OperationId,
    #[error("brainServer is not this Core's configured Brain server")]
    BrainServer,
    #[error("brainId is invalid")]
    BrainId,
    #[error("observedAt must be an RFC 3339 time")]
    ObservedAt,
    #[error("public keys must be 64 lowercase hex characters")]
    PublicKey,
    #[error("a human action must observe the participating human key")]
    HumanKeyMismatch,
}

impl BrainAccountObservationRequest {
    pub fn validate(&self, configured_brain_server: &str) -> Result<(), ObservationInputError> {
        if self.version != OBSERVATION_VERSION {
            return Err(ObservationInputError::Version);
        }
        if !valid_operation_id(&self.operation_id) {
            return Err(ObservationInputError::OperationId);
        }
        if self.brain_server != configured_brain_server {
            return Err(ObservationInputError::BrainServer);
        }
        if !valid_brain_id(&self.brain_id) {
            return Err(ObservationInputError::BrainId);
        }
        if crate::parse_time(&self.observed_at).is_err() {
            return Err(ObservationInputError::ObservedAt);
        }
        if !valid_public_key_hex(&self.participating_public_key_hex)
            || self
                .observed_human_public_key_hex
                .as_deref()
                .is_some_and(|key| !valid_public_key_hex(key))
        {
            return Err(ObservationInputError::PublicKey);
        }
        if self.action_kind == ObservationActionKind::HumanHostedAction
            && self.observed_human_public_key_hex.as_deref()
                != Some(self.participating_public_key_hex.as_str())
        {
            return Err(ObservationInputError::HumanKeyMismatch);
        }
        Ok(())
    }

    /// Digest of the canonical request, used to refuse operation-id reuse with
    /// a different payload.
    pub fn payload_sha256(&self) -> String {
        let canonical = serde_json::to_vec(self).expect("observation request serializes");
        Sha256::digest(canonical)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrainIdentityDescriptionsRequest {
    pub version: String,
    pub brain_server: String,
    pub brain_id: String,
    pub requested_by_public_key_hex: String,
    pub public_keys_hex: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DescriptionsInputError {
    #[error("unsupported descriptions version")]
    Version,
    #[error("brainServer is not this Core's configured Brain server")]
    BrainServer,
    #[error("brainId is invalid")]
    BrainId,
    #[error("public keys must be 64 lowercase hex characters")]
    PublicKey,
    #[error("publicKeysHex must contain 1-100 distinct keys")]
    KeyCount,
}

impl BrainIdentityDescriptionsRequest {
    pub fn validate(&self, configured_brain_server: &str) -> Result<(), DescriptionsInputError> {
        if self.version != DESCRIPTIONS_VERSION {
            return Err(DescriptionsInputError::Version);
        }
        if self.brain_server != configured_brain_server {
            return Err(DescriptionsInputError::BrainServer);
        }
        if !valid_brain_id(&self.brain_id) {
            return Err(DescriptionsInputError::BrainId);
        }
        if !valid_public_key_hex(&self.requested_by_public_key_hex)
            || self
                .public_keys_hex
                .iter()
                .any(|key| !valid_public_key_hex(key))
        {
            return Err(DescriptionsInputError::PublicKey);
        }
        let distinct = self
            .public_keys_hex
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if self.public_keys_hex.is_empty()
            || self.public_keys_hex.len() > MAX_DESCRIPTION_KEYS
            || distinct.len() != self.public_keys_hex.len()
        {
            return Err(DescriptionsInputError::KeyCount);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DescriptionState {
    Resolved,
    Unknown,
    Ambiguous,
    NotShared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DescribedKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentLifecycle {
    Active,
    Offboarding,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DescriptionSource {
    pub kind: String,
    pub observed_at: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponsibleAccount {
    pub email: String,
    pub source: String,
    pub observed_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub human_public_keys_hex: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityDescription {
    pub public_key_hex: String,
    pub state: DescriptionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<DescribedKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<AgentLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responsible_account: Option<ResponsibleAccount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DescriptionSource>,
}

impl IdentityDescription {
    /// A row carrying only its key and state. Every `notShared` row is built
    /// here, so an unshared known account and an unknown key are identical.
    pub fn bare(public_key_hex: &str, state: DescriptionState) -> Self {
        Self {
            public_key_hex: public_key_hex.to_string(),
            state,
            kind: None,
            display_name: None,
            account_email: None,
            lifecycle: None,
            responsible_account: None,
            source: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrainIdentityDescriptionsResponse {
    pub version: String,
    pub brain_id: String,
    pub checked_at: String,
    pub results: Vec<IdentityDescription>,
}

/// Opaque, stable revision token for a source row; never exposes Core ids.
pub(crate) fn opaque_revision(kind: &str, parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_bytes());
    for part in parts {
        digest.update([0]);
        digest.update(part.as_bytes());
    }
    let hex: String = digest
        .finalize()
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{kind}-{hex}")
}

#[cfg(test)]
mod tests;
