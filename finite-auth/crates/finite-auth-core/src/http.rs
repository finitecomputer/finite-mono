use finite_nostr::{
    EventIdHex, HttpAuthValidation, NostrPublicKey, decode_http_auth_header,
    validate_http_auth_event,
};
use nostr::{Event, Tag};

use crate::session::{validate_absolute_url, validate_http_method};
use crate::{AuthError, AuthNonce};

/// Expected request facts for a Nostr HTTP authorization event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NostrHttpAuthRequest {
    method: String,
    url: String,
    now_unix_seconds: u64,
    max_skew_seconds: u64,
    body: Option<Vec<u8>>,
    expected_nonce: Option<AuthNonce>,
    expected_signer: Option<NostrPublicKey>,
}

impl NostrHttpAuthRequest {
    /// Create expected request facts for auth validation.
    pub fn new(
        method: impl Into<String>,
        url: impl Into<String>,
        now_unix_seconds: u64,
        max_skew_seconds: u64,
    ) -> Result<Self, AuthError> {
        Ok(Self {
            method: validate_http_method(method.into())?,
            url: validate_absolute_url("auth_url", url.into())?,
            now_unix_seconds,
            max_skew_seconds,
            body: None,
            expected_nonce: None,
            expected_signer: None,
        })
    }

    /// Require the event payload hash to match this body.
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Require a Finite challenge nonce tag.
    pub fn with_expected_nonce(mut self, nonce: AuthNonce) -> Self {
        self.expected_nonce = Some(nonce);
        self
    }

    /// Require a specific Nostr signer.
    pub fn with_expected_signer(mut self, signer: NostrPublicKey) -> Self {
        self.expected_signer = Some(signer);
        self
    }
}

/// Accepted Nostr HTTP auth event after protocol and challenge checks.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VerifiedNostrAuth {
    signer: NostrPublicKey,
    event_id: EventIdHex,
    nonce: Option<AuthNonce>,
    created_at_unix_seconds: u64,
}

impl VerifiedNostrAuth {
    /// Event signer.
    pub fn signer(&self) -> NostrPublicKey {
        self.signer
    }

    /// Deterministic NIP-01 event id.
    pub fn event_id(&self) -> &EventIdHex {
        &self.event_id
    }

    /// Optional Finite challenge nonce.
    pub fn nonce(&self) -> Option<&AuthNonce> {
        self.nonce.as_ref()
    }

    /// Event creation time.
    pub fn created_at_unix_seconds(&self) -> u64 {
        self.created_at_unix_seconds
    }
}

/// Validate a NIP-98-style HTTP auth header and optional Finite nonce.
pub fn authenticate_nostr_http_header(
    header: &str,
    expected: &NostrHttpAuthRequest,
) -> Result<VerifiedNostrAuth, AuthError> {
    let event = decode_http_auth_header(header)?;
    let signer = validate_event_request_facts(&event, expected)?;
    let event_nonce = parse_event_nonce(&event)?;
    validate_expected_nonce(event_nonce.as_ref(), expected.expected_nonce.as_ref())?;

    Ok(VerifiedNostrAuth {
        signer,
        event_id: EventIdHex::parse(&event.id.to_string())?,
        nonce: event_nonce,
        created_at_unix_seconds: event.created_at.as_secs(),
    })
}

fn validate_event_request_facts(
    event: &Event,
    expected: &NostrHttpAuthRequest,
) -> Result<NostrPublicKey, AuthError> {
    let mut validation = HttpAuthValidation::new(
        expected.method.as_str(),
        expected.url.as_str(),
        expected.now_unix_seconds,
        expected.max_skew_seconds,
    );
    if let Some(body) = &expected.body {
        validation = validation.with_body(body.clone());
    }
    if let Some(signer) = expected.expected_signer {
        validation = validation.with_expected_signer(signer);
    }

    validate_http_auth_event(event, &validation).map_err(AuthError::from)
}

fn parse_event_nonce(event: &Event) -> Result<Option<AuthNonce>, AuthError> {
    tag_content(event, "nonce").map(AuthNonce::new).transpose()
}

fn validate_expected_nonce(
    actual: Option<&AuthNonce>,
    expected: Option<&AuthNonce>,
) -> Result<(), AuthError> {
    if let Some(expected) = expected {
        let Some(actual) = actual else {
            return Err(AuthError::MissingNonce);
        };

        if actual != expected {
            return Err(AuthError::NonceMismatch {
                expected: expected.as_str().to_string(),
                actual: Some(actual.as_str().to_string()),
            });
        }
    }

    Ok(())
}

fn tag_content(event: &Event, tag: &'static str) -> Option<String> {
    event
        .tags
        .iter()
        .find(|candidate| candidate.kind() == tag)
        .and_then(Tag::content)
        .map(ToOwned::to_owned)
}
