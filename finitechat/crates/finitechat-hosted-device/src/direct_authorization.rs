//! User-signed request assertions for the direct Finite Chat transport.
//!
//! In direct mode the dashboard talks to the Agent's own chat bridge, which
//! verifies a kind-27235 event signed by the user's existing chat account key
//! on every viewer request, so code inside the Agent container cannot act as
//! its owner. This route signs exactly one bounded bridge request description:
//! it never signs caller-supplied templates and never mints an identity.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::{CACHE_CONTROL, HeaderValue};
use axum::response::{IntoResponse, Response};
use finite_nostr::HTTP_AUTH_KIND;
use nostr::event::FinalizeEvent;
use nostr::{Event, EventBuilder, Keys, Kind, SecretKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{HostedDeviceError, HostedDeviceState, authorized_user, existing_chat_identity};

/// The request carries only bounded identifiers, a path, and a digest.
pub(crate) const MAX_DIRECT_AUTHORIZATION_REQUEST_BYTES: usize = 8 * 1024;
const DIRECT_AUTHORIZATION_VERSION: u16 = 1;
const DIRECT_URL_SCHEME: &str = "finitechat-direct://";
const AGENT_ID_PREFIX: &str = "agent_";
const MAX_AGENT_ID_SUFFIX_BYTES: usize = 120;
const BRIDGE_PATH_PREFIX: &str = "/v1/app/";
const MAX_BRIDGE_PATH_BYTES: usize = 2048;
const SHA256_HEX_BYTES: usize = 64;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectAuthorizationRequest {
    version: u16,
    agent_id: String,
    method: String,
    path: String,
    body_sha256: Option<String>,
}

#[derive(Serialize)]
struct DirectAuthorizationResponse {
    event: Event,
    account_id: String,
}

pub(crate) async fn authorize_direct_request(
    State(state): State<HostedDeviceState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, HostedDeviceError> {
    let user_id = authorized_user(&state, &headers)?;
    let request: DirectAuthorizationRequest = serde_json::from_slice(&body)
        .map_err(|error| HostedDeviceError::InvalidDirectAuthorization(error.to_string()))?;
    let tags = direct_authorization_tags(&request)?;
    let response = tokio::task::spawn_blocking(move || -> Result<_, HostedDeviceError> {
        let identity = existing_chat_identity(&state, &user_id)?
            .ok_or(HostedDeviceError::ChatIdentityMissing)?;
        let secret_bytes = Zeroizing::new(identity.expose_secret_bytes());
        let keys = SecretKey::from_slice(&secret_bytes[..])
            .map(Keys::new)
            .map_err(|_| {
                HostedDeviceError::Task("Finite Chat identity key is invalid".to_owned())
            })?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| HostedDeviceError::Task(format!("system clock is invalid: {error}")))?
            .as_secs();
        let event = EventBuilder::new(Kind::from(HTTP_AUTH_KIND), "")
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(now))
            .finalize(&keys)
            .map_err(|_| {
                HostedDeviceError::Task("direct chat authorization could not be signed".to_owned())
            })?;
        let account_id = event.pubkey.to_hex();
        Ok(DirectAuthorizationResponse { event, account_id })
    })
    .await
    .map_err(|error| HostedDeviceError::Task(error.to_string()))??;
    let mut response = Json(response).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// Builds the exact tags the bridge verifies: `u` naming the Agent and bridge
/// path, `method`, then `payload` (the request body digest) for POST only.
fn direct_authorization_tags(
    request: &DirectAuthorizationRequest,
) -> Result<Vec<Tag>, HostedDeviceError> {
    if request.version != DIRECT_AUTHORIZATION_VERSION {
        return Err(invalid("unsupported version"));
    }
    if !is_agent_id(&request.agent_id) {
        return Err(invalid("agent_id must match agent_[A-Za-z0-9_-]{1,120}"));
    }
    if !is_bridge_path(&request.path) {
        return Err(invalid(
            "path must be a /v1/app/ bridge path of at most 2048 visible ASCII bytes without a fragment or dot segment",
        ));
    }
    let payload = match (request.method.as_str(), request.body_sha256.as_deref()) {
        ("GET", None) => None,
        ("POST", Some(digest)) if is_sha256_hex(digest) => Some(digest),
        ("GET", Some(_)) => return Err(invalid("body_sha256 must be null for GET")),
        ("POST", _) => return Err(invalid("POST requires body_sha256 as 64 lowercase hex")),
        _ => return Err(invalid("method must be GET or POST")),
    };
    let mut tags = vec![
        vec![
            "u".to_owned(),
            format!("{DIRECT_URL_SCHEME}{}{}", request.agent_id, request.path),
        ],
        vec!["method".to_owned(), request.method.clone()],
    ];
    if let Some(digest) = payload {
        tags.push(vec!["payload".to_owned(), digest.to_owned()]);
    }
    // Identical requests in the same second must still be distinct events:
    // the bridge rejects a reused event id.
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| invalid("randomness is unavailable"))?;
    tags.push(vec!["nonce".to_owned(), hex::encode(nonce)]);
    tags.into_iter()
        .map(|tag| Tag::parse(tag).map_err(|_| invalid("request tag is invalid")))
        .collect()
}

fn is_agent_id(value: &str) -> bool {
    value.strip_prefix(AGENT_ID_PREFIX).is_some_and(|suffix| {
        (1..=MAX_AGENT_ID_SUFFIX_BYTES).contains(&suffix.len())
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    })
}

/// A request target is visible ASCII, so control characters, whitespace, and
/// unencoded non-ASCII are rejected along with fragments. Dot segments are
/// rejected so a normalizing hop cannot move a signed path outside `/v1/app/`.
fn is_bridge_path(path: &str) -> bool {
    path.starts_with(BRIDGE_PATH_PREFIX)
        && path.len() <= MAX_BRIDGE_PATH_BYTES
        && path
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'#')
        && path
            .split('?')
            .next()
            .unwrap_or_default()
            .split('/')
            .all(|segment| segment != "." && segment != "..")
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == SHA256_HEX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid(reason: &str) -> HostedDeviceError {
    HostedDeviceError::InvalidDirectAuthorization(reason.to_owned())
}
