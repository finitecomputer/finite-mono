//! Optional private Core identity descriptions client used only by the
//! access report (FIN-122). Core describes exact keys for this Brain's
//! audience; it never decides access. See
//! `finitecomputer-v2/docs/brain-identity-descriptions-v1.md`.

use std::collections::BTreeSet;
use std::io::Read;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub(crate) const DESCRIPTIONS_VERSION: &str = "finite-core-brain-identity-descriptions-v1";
const DESCRIPTIONS_PATH: &str = "/api/core/internal/v1/brain-identity-descriptions";
const DESCRIPTION_CREDENTIAL_HEADER: &str = "x-finite-brain-description-credential";
/// Keys per Core request; matches Core's batch ceiling.
pub(crate) const CORE_BATCH_KEYS: usize = 100;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const READ_TIMEOUT: Duration = Duration::from_secs(3);
/// Whole-request ceiling, shorter than the caller's async timeout, so an
/// abandoned blocking lookup still finishes and releases its shared permit.
pub(crate) const CORE_TOTAL_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE_BYTES: u64 = 256 * 1024;
const MAX_CONTACT_BYTES: usize = 254;
const MAX_DISPLAY_NAME_BYTES: usize = 200;
const MAX_TOKEN_BYTES: usize = 128;
const MAX_HUMAN_KEYS: usize = 8;

/// Accept only a literal loopback or private-network base URL with no user
/// info, path, query or fragment. A host name is refused so DNS can never
/// redirect the credential and report keys.
pub(crate) fn validate_private_base_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or("Core identity URL must be http(s)")?;
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty() || authority.contains(['/', '?', '#', '@', '\\']) {
        return Err(
            "Core identity URL must be scheme://private-ip:port with no user info, path, query, or fragment"
                .to_owned(),
        );
    }
    let address = authority
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "Core identity URL needs a literal IP address and port".to_owned())?;
    let private = match address.ip() {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
    };
    if !private || address.port() == 0 {
        return Err("Core identity URL host must be a loopback or private address".to_owned());
    }
    Ok(url.trim_end_matches('/').to_owned())
}

/// Why a Core batch could not be used. Never turned into `unknown`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum CoreLookupFailure {
    /// Core does not serve the route (older or disabled deployment).
    Unsupported,
    /// Core rejected the configured credential.
    Unauthorized,
    /// Transport, timeout, or a response that did not match the request.
    Unavailable(String),
}

impl CoreLookupFailure {
    pub(crate) fn state(&self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Unauthorized | Self::Unavailable(_) => "unavailable",
        }
    }

    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Unsupported => "Core does not offer identity descriptions".to_owned(),
            Self::Unauthorized => "Core rejected the description credential".to_owned(),
            Self::Unavailable(reason) => {
                format!("Core identity descriptions unavailable: {reason}")
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CoreSource {
    pub(crate) kind: String,
    pub(crate) observed_at: String,
    pub(crate) revision: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CoreResponsibleAccount {
    pub(crate) email: String,
    pub(crate) source: String,
    pub(crate) observed_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) human_public_keys_hex: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CoreDescription {
    pub(crate) public_key_hex: String,
    pub(crate) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) account_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) lifecycle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) responsible_account: Option<CoreResponsibleAccount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<CoreSource>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CoreDescriptionsResponse {
    pub(crate) version: String,
    pub(crate) brain_id: String,
    pub(crate) checked_at: String,
    pub(crate) results: Vec<CoreDescription>,
}

/// One description request: this Brain's identity and exact page keys.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct CoreLookupRequest {
    pub(crate) brain_server: String,
    pub(crate) brain_id: String,
    pub(crate) requested_by_hex: String,
    pub(crate) keys: Vec<String>,
}

/// Blocking lookup, called off the async runtime and never while the store
/// mutex is held.
pub(crate) type CoreDescriptionLookup = Arc<
    dyn Fn(&CoreLookupRequest) -> Result<CoreDescriptionsResponse, CoreLookupFailure> + Send + Sync,
>;

/// Build the HTTP lookup against Core's private Brain identity listener.
pub(crate) fn http_core_description_lookup(
    base_url: &str,
    credential: String,
) -> Result<CoreDescriptionLookup, String> {
    let url = format!(
        "{}{DESCRIPTIONS_PATH}",
        validate_private_base_url(base_url)?
    );
    Ok(Arc::new(move |request| {
        // Never route the credential through an ambient proxy, whatever
        // ureq features are unified into this build.
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .timeout(CORE_TOTAL_TIMEOUT)
            .redirects(0)
            .build();
        let body = serde_json::json!({
            "version": DESCRIPTIONS_VERSION,
            "brainServer": request.brain_server,
            "brainId": request.brain_id,
            "requestedByPublicKeyHex": request.requested_by_hex,
            "publicKeysHex": request.keys,
        })
        .to_string();
        let response = match agent
            .post(&url)
            .set("content-type", "application/json")
            .set(DESCRIPTION_CREDENTIAL_HEADER, &credential)
            .send_string(&body)
        {
            Ok(response) => response,
            Err(ureq::Error::Status(404 | 405, _)) => return Err(CoreLookupFailure::Unsupported),
            Err(ureq::Error::Status(401 | 403, _)) => return Err(CoreLookupFailure::Unauthorized),
            Err(ureq::Error::Status(status, _)) => {
                return Err(CoreLookupFailure::Unavailable(format!("status {status}")));
            }
            Err(ureq::Error::Transport(_)) => {
                return Err(CoreLookupFailure::Unavailable("transport".to_owned()));
            }
        };
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| CoreLookupFailure::Unavailable("read failed".to_owned()))?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(CoreLookupFailure::Unavailable(
                "response too large".to_owned(),
            ));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| CoreLookupFailure::Unavailable("malformed response".to_owned()))
    }))
}

/// Accept a batch only when it is this protocol, for this Brain, answers
/// every requested key exactly once and every row is well formed for its
/// state. Anything else makes the whole batch unusable, never partly trusted.
pub(crate) fn validate_core_response(
    request: &CoreLookupRequest,
    response: CoreDescriptionsResponse,
) -> Result<CoreDescriptionsResponse, CoreLookupFailure> {
    let invalid = |reason: &str| Err(CoreLookupFailure::Unavailable(reason.to_owned()));
    if response.version != DESCRIPTIONS_VERSION {
        return invalid("unsupported response version");
    }
    if response.brain_id != request.brain_id {
        return invalid("response is for another Brain");
    }
    if !timestamp(&response.checked_at) {
        return invalid("response checkedAt is invalid");
    }
    let requested = request.keys.iter().collect::<BTreeSet<_>>();
    let answered = response
        .results
        .iter()
        .map(|row| &row.public_key_hex)
        .collect::<BTreeSet<_>>();
    if answered.len() != response.results.len()
        || answered != requested
        || response.results.len() != request.keys.len()
    {
        return invalid("response did not answer exactly the requested keys");
    }
    for row in &response.results {
        if !row_well_formed(row) {
            return invalid("response carried a malformed description");
        }
    }
    Ok(response)
}

fn row_well_formed(row: &CoreDescription) -> bool {
    let bare = row.kind.is_none()
        && row.display_name.is_none()
        && row.account_email.is_none()
        && row.lifecycle.is_none()
        && row.responsible_account.is_none()
        && row.source.is_none();
    match row.state.as_str() {
        "unknown" | "ambiguous" | "notShared" => bare,
        "resolved" => {
            let Some(source) = &row.source else {
                return false;
            };
            let source_ok = text(&source.kind, MAX_TOKEN_BYTES)
                && text(&source.revision, MAX_TOKEN_BYTES)
                && timestamp(&source.observed_at);
            match row.kind.as_deref() {
                Some("human") => {
                    source_ok
                        && row
                            .account_email
                            .as_deref()
                            .is_some_and(|email| text(email, MAX_CONTACT_BYTES))
                        && row
                            .display_name
                            .as_deref()
                            .is_none_or(|name| text(name, MAX_DISPLAY_NAME_BYTES))
                        && row.lifecycle.is_none()
                        && row.responsible_account.is_none()
                }
                Some("agent") => {
                    source_ok
                        && row
                            .display_name
                            .as_deref()
                            .is_some_and(|name| text(name, MAX_DISPLAY_NAME_BYTES))
                        && matches!(
                            row.lifecycle.as_deref(),
                            Some("active" | "offboarding" | "retired")
                        )
                        && row.account_email.is_none()
                        && row.responsible_account.as_ref().is_some_and(|account| {
                            text(&account.email, MAX_CONTACT_BYTES)
                                && text(&account.source, MAX_TOKEN_BYTES)
                                && timestamp(&account.observed_at)
                                && account.human_public_keys_hex.len() <= MAX_HUMAN_KEYS
                                && account
                                    .human_public_keys_hex
                                    .iter()
                                    .all(|key| canonical_hex(key))
                        })
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn timestamp(value: &str) -> bool {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).is_ok()
}

pub(crate) fn canonical_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared wire fixture Core's serializer test also round-trips.
    const FIXTURE: &str = include_str!(
        "../../../../finitecomputer-v2/docs/contracts/brain-identity-descriptions-v1.example.json"
    );

    fn fixture() -> CoreDescriptionsResponse {
        serde_json::from_str(FIXTURE).expect("shared fixture parses strictly")
    }

    fn request_for(response: &CoreDescriptionsResponse) -> CoreLookupRequest {
        CoreLookupRequest {
            brain_server: "https://brain.test".to_owned(),
            brain_id: response.brain_id.clone(),
            requested_by_hex: "e".repeat(64),
            keys: response
                .results
                .iter()
                .map(|row| row.public_key_hex.clone())
                .collect(),
        }
    }

    #[test]
    fn shared_core_fixture_is_accepted_with_every_state() {
        let response = fixture();
        let request = request_for(&response);
        let accepted = validate_core_response(&request, response).unwrap();
        let states = accepted
            .results
            .iter()
            .map(|row| row.state.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            states,
            BTreeSet::from(["ambiguous", "notShared", "resolved", "unknown"])
        );
    }

    #[test]
    fn malformed_partial_or_extra_responses_are_refused_whole() {
        let base = fixture();
        let request = request_for(&base);
        let mut cases: Vec<CoreDescriptionsResponse> = Vec::new();
        let mut missing = base.clone();
        missing.results.pop();
        cases.push(missing);
        let mut duplicate = base.clone();
        duplicate.results[1] = duplicate.results[0].clone();
        cases.push(duplicate);
        let mut extra = base.clone();
        extra.results.push(CoreDescription {
            public_key_hex: "f".repeat(64),
            ..base.results[1].clone()
        });
        cases.push(extra);
        let mut version = base.clone();
        version.version = "finite-core-brain-identity-descriptions-v2".to_owned();
        cases.push(version);
        let mut other_brain = base.clone();
        other_brain.brain_id = "brain_other".to_owned();
        cases.push(other_brain);
        let mut leaking = base.clone();
        leaking.results[1].account_email = Some("hidden@example.org".to_owned());
        cases.push(leaking);
        let mut state = base.clone();
        state.results[1].state = "found".to_owned();
        cases.push(state);
        let mut long = base.clone();
        long.results[0].display_name = Some("N".repeat(201));
        cases.push(long);
        let mut control = base.clone();
        control.results[2].account_email = Some("sam\u{7}@example.org".to_owned());
        cases.push(control);
        for case in cases {
            assert!(validate_core_response(&request, case).is_err());
        }
        let unknown_field = FIXTURE.replacen(
            "\"state\": \"notShared\"",
            "\"state\": \"notShared\", \"email\": \"x\"",
            1,
        );
        assert!(serde_json::from_str::<CoreDescriptionsResponse>(&unknown_field).is_err());
    }

    #[test]
    fn core_url_must_be_a_private_literal_address() {
        for accepted in [
            "http://127.0.0.1:4202",
            "http://10.0.0.2:4202/",
            "http://[::1]:4202",
        ] {
            assert!(validate_private_base_url(accepted).is_ok(), "{accepted}");
        }
        for refused in [
            "http://localhost:4202",
            "http://64.34.80.19:4202",
            "http://0.0.0.0:4202",
            "http://127.0.0.1",
            "http://127.0.0.1:4202/path",
            "http://user@127.0.0.1:4202",
            "ftp://127.0.0.1:4202",
        ] {
            assert!(validate_private_base_url(refused).is_err(), "{refused}");
        }
    }
}
