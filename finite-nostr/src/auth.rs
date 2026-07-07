use std::str;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use nostr::event::{FinalizeEvent, SignEvent};
use nostr::hashes::Hash;
use nostr::hashes::sha256::Hash as Sha256Hash;
use nostr::key::GetPublicKey;
use nostr::{Event, EventBuilder, Kind, Tag, Timestamp, Url};

use crate::{NostrPrimitiveError, NostrPublicKey, verify_event_integrity};

/// HTTP auth events are NIP-98 kind 27235.
pub const HTTP_AUTH_KIND: u16 = 27_235;

/// HTTP authorization scheme for NIP-98-style auth headers.
pub const HTTP_AUTH_SCHEME: &str = "Nostr";

/// Request facts used to create a NIP-98-style HTTP auth event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HttpAuthEventRequest {
    method: String,
    url: String,
    created_at: Timestamp,
    body: Option<Vec<u8>>,
    nonce: Option<String>,
}

/// Expected request data for validating a NIP-98 style event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HttpAuthValidation {
    method: String,
    url: String,
    now: Timestamp,
    max_skew_seconds: u64,
    body: Option<Vec<u8>>,
    expected_signer: Option<NostrPublicKey>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ParsedHttpAuth {
    url: String,
    method: String,
    payload: Option<String>,
}

impl HttpAuthEventRequest {
    /// Create a request for signing an auth event.
    pub fn new<M, U>(method: M, url: U, created_at_unix_seconds: u64) -> Self
    where
        M: Into<String>,
        U: Into<String>,
    {
        Self {
            method: method.into(),
            url: url.into(),
            created_at: Timestamp::from_secs(created_at_unix_seconds),
            body: None,
            nonce: None,
        }
    }

    /// Include a request body hash in the signed event.
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Include a caller-provided nonce tag in the signed event.
    pub fn with_nonce(mut self, nonce: impl Into<String>) -> Self {
        self.nonce = Some(nonce.into());
        self
    }
}

impl HttpAuthValidation {
    /// Create an expected auth request.
    pub fn new<M, U>(method: M, url: U, now_unix_seconds: u64, max_skew_seconds: u64) -> Self
    where
        M: Into<String>,
        U: Into<String>,
    {
        Self {
            method: method.into(),
            url: url.into(),
            now: Timestamp::from_secs(now_unix_seconds),
            max_skew_seconds,
            body: None,
            expected_signer: None,
        }
    }

    /// Require the event payload tag to match this request body.
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Require the event signer to match this public key.
    pub fn with_expected_signer(mut self, signer: NostrPublicKey) -> Self {
        self.expected_signer = Some(signer);
        self
    }
}

/// Sign a NIP-98-style HTTP auth event.
pub fn sign_http_auth_event<S>(
    signer: &S,
    request: &HttpAuthEventRequest,
) -> Result<Event, NostrPrimitiveError>
where
    S: GetPublicKey + SignEvent,
{
    let mut tags = vec![
        auth_tag(["u", request.url.as_str()])?,
        auth_tag(["method", request.method.as_str()])?,
    ];
    if let Some(nonce) = &request.nonce {
        tags.push(auth_tag(["nonce", nonce.as_str()])?);
    }
    if let Some(body) = &request.body {
        let payload = payload_hash_hex(body);
        tags.push(auth_tag(["payload", payload.as_str()])?);
    }

    EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .custom_created_at(request.created_at)
        .finalize(signer)
        .map_err(|_| NostrPrimitiveError::SigningFailure)
}

/// Encode a NIP-98-style event into an HTTP authorization header value.
pub fn encode_http_auth_header(event: &Event) -> String {
    format!(
        "{HTTP_AUTH_SCHEME} {}",
        BASE64_STANDARD.encode(event.as_json())
    )
}

/// Decode a NIP-98-style HTTP authorization header into a signed event.
pub fn decode_http_auth_header(header: &str) -> Result<Event, NostrPrimitiveError> {
    let encoded = header
        .strip_prefix("Nostr ")
        .ok_or(NostrPrimitiveError::MalformedInput {
            field: "http_auth_header",
        })?;
    let event_json =
        BASE64_STANDARD
            .decode(encoded)
            .map_err(|_| NostrPrimitiveError::MalformedInput {
                field: "http_auth_header",
            })?;
    let event_json =
        str::from_utf8(&event_json).map_err(|_| NostrPrimitiveError::MalformedInput {
            field: "http_auth_header",
        })?;
    Event::from_json(event_json).map_err(|_| NostrPrimitiveError::MalformedInput {
        field: "http_auth_header",
    })
}

/// Validate a signed NIP-98-style HTTP auth event against request facts.
pub fn validate_http_auth_event(
    event: &Event,
    expected: &HttpAuthValidation,
) -> Result<NostrPublicKey, NostrPrimitiveError> {
    if event.kind != Kind::HttpAuth {
        return Err(NostrPrimitiveError::WrongEventKind {
            expected: HTTP_AUTH_KIND,
            actual: event.kind.as_u16(),
        });
    }

    verify_event_integrity(event)?;
    validate_timestamp(event.created_at, expected.now, expected.max_skew_seconds)?;

    let http_data = parse_http_data(event)?;
    validate_url(&http_data, &expected.url)?;
    validate_method(&http_data, &expected.method)?;
    validate_payload(&http_data, expected.body.as_deref())?;

    let signer = NostrPublicKey::from_protocol(event.pubkey);
    if let Some(expected_signer) = expected.expected_signer
        && signer != expected_signer
    {
        return Err(NostrPrimitiveError::SignerMismatch {
            expected: expected_signer.to_hex(),
            actual: signer.to_hex(),
        });
    }

    Ok(signer)
}

pub(crate) fn payload_hash_hex(body: &[u8]) -> String {
    Sha256Hash::hash(body).to_string()
}

fn auth_tag<const N: usize>(parts: [&str; N]) -> Result<Tag, NostrPrimitiveError> {
    Tag::parse(parts).map_err(|_| NostrPrimitiveError::MalformedInput {
        field: "http_auth_tag",
    })
}

fn parse_http_data(event: &Event) -> Result<ParsedHttpAuth, NostrPrimitiveError> {
    let url = tag_content(event, "u").ok_or(NostrPrimitiveError::MissingTag { tag: "u" })?;
    Url::parse(&url).map_err(|_| NostrPrimitiveError::MalformedInput {
        field: "http_auth_url",
    })?;
    let method =
        tag_content(event, "method").ok_or(NostrPrimitiveError::MissingTag { tag: "method" })?;
    let payload = tag_content(event, "payload");

    Ok(ParsedHttpAuth {
        url,
        method,
        payload,
    })
}

fn tag_content(event: &Event, tag: &'static str) -> Option<String> {
    event
        .tags
        .iter()
        .find(|candidate| candidate.kind() == tag)
        .and_then(Tag::content)
        .map(ToOwned::to_owned)
}

fn validate_timestamp(
    created_at: Timestamp,
    now: Timestamp,
    max_skew_seconds: u64,
) -> Result<(), NostrPrimitiveError> {
    let created_at_secs = created_at.as_secs();
    let now_secs = now.as_secs();
    let delta = created_at_secs.abs_diff(now_secs);

    if delta > max_skew_seconds {
        return Err(NostrPrimitiveError::StaleTimestamp {
            now: now_secs,
            created_at: created_at_secs,
            max_skew_seconds,
        });
    }

    Ok(())
}

fn validate_url(http_data: &ParsedHttpAuth, expected_url: &str) -> Result<(), NostrPrimitiveError> {
    if http_data.url != expected_url {
        return Err(NostrPrimitiveError::UrlMismatch {
            expected: expected_url.to_string(),
            actual: http_data.url.clone(),
        });
    }

    Ok(())
}

fn validate_method(
    http_data: &ParsedHttpAuth,
    expected_method: &str,
) -> Result<(), NostrPrimitiveError> {
    if http_data.method != expected_method {
        return Err(NostrPrimitiveError::MethodMismatch {
            expected: expected_method.to_string(),
            actual: http_data.method.clone(),
        });
    }

    Ok(())
}

fn validate_payload(
    http_data: &ParsedHttpAuth,
    body: Option<&[u8]>,
) -> Result<(), NostrPrimitiveError> {
    let expected = body.map(payload_hash_hex);
    let actual = http_data.payload.clone();

    if actual != expected {
        return Err(NostrPrimitiveError::PayloadMismatch { expected, actual });
    }

    Ok(())
}
