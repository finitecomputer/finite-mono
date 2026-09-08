//! NIP-98 HTTP authorization: a kind-27235 nostr event, base64-encoded into
//! `Authorization: Nostr <event>`, binding the signer to one URL + method
//! (+ body hash) inside a small freshness window.
//!
//! Statement kind 1: "this key makes this HTTP request". The freshness
//! window comes from the shared [`crate::AuthPolicy`] table so every
//! verifier in the fleet applies the same skew.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};

use crate::event::NostrEvent;
use crate::{AuthnError, hex};

pub const NIP98_KIND: u32 = 27235;
pub const AUTH_SCHEME: &str = "Nostr ";

/// A claim or auth header is rejected above this size before any parsing.
pub const MAX_AUTH_HEADER_BYTES: u32 = 8 * 1024;

/// Build the value for the `Authorization` header.
pub fn build_auth_header(
    secret_key: &[u8; 32],
    url: &str,
    method: &str,
    body: Option<&[u8]>,
    now_unix: u64,
) -> Result<String, AuthnError> {
    assert!(!url.is_empty() && !method.is_empty());
    let mut tags = vec![
        vec!["u".to_string(), url.to_string()],
        vec!["method".to_string(), method.to_string()],
    ];
    if let Some(body_bytes) = body {
        let digest = Sha256::digest(body_bytes);
        tags.push(vec!["payload".to_string(), hex::encode(&digest)]);
    }
    let event = NostrEvent::sign(secret_key, now_unix, NIP98_KIND, tags, String::new())?;
    let encoded = BASE64.encode(serde_json::to_vec(&event).expect("event always serializes"));
    let header = format!("{AUTH_SCHEME}{encoded}");
    assert!(header.len() > AUTH_SCHEME.len());
    Ok(header)
}

/// Verify an `Authorization` header against the request the server actually
/// received. Returns the authenticated pubkey hex. `max_skew_seconds` is the
/// shared policy window (see [`crate::AuthPolicy::nip98_max_skew_seconds`]).
pub fn verify_auth_header(
    header: &str,
    expected_url: &str,
    expected_method: &str,
    body: Option<&[u8]>,
    now_unix: u64,
    max_skew_seconds: u64,
) -> Result<String, AuthnError> {
    assert!(!expected_url.is_empty() && !expected_method.is_empty());
    if header.len() > MAX_AUTH_HEADER_BYTES as usize {
        return Err(AuthnError::InvalidAuthHeader("header too large"));
    }
    let encoded = header
        .strip_prefix(AUTH_SCHEME)
        .ok_or(AuthnError::InvalidAuthHeader("missing Nostr scheme"))?;
    let raw = BASE64
        .decode(encoded)
        .map_err(|_| AuthnError::InvalidAuthHeader("invalid base64"))?;
    let event: NostrEvent = serde_json::from_slice(&raw)
        .map_err(|_| AuthnError::InvalidAuthHeader("invalid event json"))?;

    if event.kind != NIP98_KIND {
        return Err(AuthnError::AuthRejected("wrong event kind"));
    }
    let oldest_acceptable = now_unix.saturating_sub(max_skew_seconds);
    let newest_acceptable = now_unix.saturating_add(max_skew_seconds);
    let created_at_is_fresh =
        event.created_at >= oldest_acceptable && event.created_at <= newest_acceptable;
    if !created_at_is_fresh {
        return Err(AuthnError::AuthRejected("event timestamp outside window"));
    }
    if event.tag_value("u") != Some(expected_url) {
        return Err(AuthnError::AuthRejected("url mismatch"));
    }
    if event.tag_value("method") != Some(expected_method) {
        return Err(AuthnError::AuthRejected("method mismatch"));
    }
    match (body, event.tag_value("payload")) {
        (Some(body_bytes), Some(claimed)) => {
            let digest = hex::encode(&Sha256::digest(body_bytes));
            if digest != claimed {
                return Err(AuthnError::AuthRejected("payload hash mismatch"));
            }
        }
        (Some(body_bytes), None) => {
            // Empty bodies may omit the payload tag; non-empty must bind it.
            if !body_bytes.is_empty() {
                return Err(AuthnError::AuthRejected("missing payload tag"));
            }
        }
        (None, Some(_)) => {
            return Err(AuthnError::AuthRejected("unexpected payload tag"));
        }
        (None, None) => {}
    }

    let pubkey = event.verify()?.to_string();
    assert!(hex::is_hex32(&pubkey));
    Ok(pubkey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::pubkey_for_secret;

    const URL: &str = "http://127.0.0.1:8787/api/v1/projects/init";
    const NOW: u64 = 1_750_000_000;
    const SKEW: u64 = 60;

    fn secret(fill: u8) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        bytes[0] = fill;
        bytes[31] = 7;
        bytes
    }

    #[test]
    fn roundtrip_with_body() {
        let body = br#"{"name":"hello"}"#;
        let header = build_auth_header(&secret(1), URL, "POST", Some(body), NOW).unwrap();
        let pubkey = verify_auth_header(&header, URL, "POST", Some(body), NOW + 5, SKEW).unwrap();
        assert_eq!(pubkey, pubkey_for_secret(&secret(1)).unwrap());
    }

    #[test]
    fn roundtrip_without_body() {
        let header = build_auth_header(&secret(1), URL, "GET", None, NOW).unwrap();
        assert!(verify_auth_header(&header, URL, "GET", None, NOW, SKEW).is_ok());
    }

    #[test]
    fn rejects_url_and_method_mismatch() {
        let header = build_auth_header(&secret(1), URL, "GET", None, NOW).unwrap();
        assert_eq!(
            verify_auth_header(&header, "http://evil/", "GET", None, NOW, SKEW),
            Err(AuthnError::AuthRejected("url mismatch"))
        );
        assert_eq!(
            verify_auth_header(&header, URL, "POST", None, NOW, SKEW),
            Err(AuthnError::AuthRejected("method mismatch"))
        );
    }

    #[test]
    fn rejects_stale_and_future_events() {
        let header = build_auth_header(&secret(1), URL, "GET", None, NOW).unwrap();
        assert_eq!(
            verify_auth_header(&header, URL, "GET", None, NOW + SKEW + 1, SKEW),
            Err(AuthnError::AuthRejected("event timestamp outside window"))
        );
        assert_eq!(
            verify_auth_header(&header, URL, "GET", None, NOW - SKEW - 1, SKEW),
            Err(AuthnError::AuthRejected("event timestamp outside window"))
        );
    }

    #[test]
    fn rejects_body_tampering_and_missing_payload_tag() {
        let header = build_auth_header(&secret(1), URL, "POST", Some(b"original"), NOW).unwrap();
        assert_eq!(
            verify_auth_header(&header, URL, "POST", Some(b"tampered"), NOW, SKEW),
            Err(AuthnError::AuthRejected("payload hash mismatch"))
        );
        let unsigned = build_auth_header(&secret(1), URL, "POST", None, NOW).unwrap();
        assert_eq!(
            verify_auth_header(&unsigned, URL, "POST", Some(b"body"), NOW, SKEW),
            Err(AuthnError::AuthRejected("missing payload tag"))
        );
    }

    #[test]
    fn rejects_garbage_header() {
        assert!(verify_auth_header("Bearer xyz", URL, "GET", None, NOW, SKEW).is_err());
        assert!(verify_auth_header("Nostr not-base64!!!", URL, "GET", None, NOW, SKEW).is_err());
    }
}
