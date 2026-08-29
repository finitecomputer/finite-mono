//! NIP-98 HTTP authorization: a kind-27235 nostr event, base64-encoded into
//! `Authorization: Nostr <event>`, binding the signer to one URL + method
//! (+ body hash) inside a small freshness window.
//!
//! Signing and validation are the canonical implementations in
//! `finite-nostr` (`auth` module). This module is only the Sites wire
//! policy on top: the `finitesitesd`/`fsite` error surface, the header
//! size cap, the fixed 60-second freshness window, and one leniency —
//! an empty request body may omit the `payload` tag (external
//! spec-following signers do), while non-empty bodies must bind their
//! hash and a body-less request must not carry a tag.

use finite_nostr::{
    HttpAuthEventRequest, HttpAuthValidation, NostrPrimitiveError, decode_http_auth_header,
    sign_http_auth_header_with_secret, validate_http_auth_event,
};

use crate::ProtoError;
use crate::limits::{MAX_AUTH_HEADER_BYTES, NIP98_MAX_SKEW_SECONDS};

pub const NIP98_KIND: u32 = 27235;
pub const AUTH_SCHEME: &str = "Nostr ";

/// Build the value for the `Authorization` header.
pub fn build_auth_header(
    secret_key: &[u8; 32],
    url: &str,
    method: &str,
    body: Option<&[u8]>,
    now_unix: u64,
) -> Result<String, ProtoError> {
    assert!(!url.is_empty() && !method.is_empty());
    let mut request = HttpAuthEventRequest::new(method, url, now_unix);
    if let Some(body_bytes) = body {
        request = request.with_body(body_bytes);
    }
    let header =
        sign_http_auth_header_with_secret(secret_key, &request).map_err(|error| match error {
            NostrPrimitiveError::MalformedInput {
                field: "http_auth_secret_key",
            } => ProtoError::InvalidEvent("invalid secret key"),
            _ => ProtoError::InvalidEvent("signing failed"),
        })?;
    assert!(header.len() > AUTH_SCHEME.len());
    Ok(header)
}

/// Verify an `Authorization` header against the request the server actually
/// received. Returns the authenticated pubkey hex.
pub fn verify_auth_header(
    header: &str,
    expected_url: &str,
    expected_method: &str,
    body: Option<&[u8]>,
    now_unix: u64,
) -> Result<String, ProtoError> {
    assert!(!expected_url.is_empty() && !expected_method.is_empty());
    if header.len() > MAX_AUTH_HEADER_BYTES as usize {
        return Err(ProtoError::InvalidAuthHeader("header too large"));
    }
    header
        .strip_prefix(AUTH_SCHEME)
        .ok_or(ProtoError::InvalidAuthHeader("missing Nostr scheme"))?;
    let event = decode_http_auth_header(header)
        .map_err(|_| ProtoError::InvalidAuthHeader("invalid base64 or event json"))?;

    let mut validation = HttpAuthValidation::new(
        expected_method,
        expected_url,
        now_unix,
        NIP98_MAX_SKEW_SECONDS,
    );
    if let Some(body_bytes) = body {
        validation = validation.with_body(body_bytes);
    }
    match validate_http_auth_event(&event, &validation) {
        Ok(signer) => Ok(signer.to_hex()),
        // Sites wire policy: an empty request body may omit the payload tag.
        // Retry once without the body expectation; every other check
        // (kind, integrity, freshness, url, method) still applies.
        Err(NostrPrimitiveError::PayloadMismatch {
            expected: Some(_),
            actual: None,
        }) if body.is_some_and(|bytes| bytes.is_empty()) => {
            let lax = HttpAuthValidation::new(
                expected_method,
                expected_url,
                now_unix,
                NIP98_MAX_SKEW_SECONDS,
            );
            validate_http_auth_event(&event, &lax)
                .map(|signer| signer.to_hex())
                .map_err(map_validation_error)
        }
        Err(error) => Err(map_validation_error(error)),
    }
}

/// Map canonical validation failures onto the Sites error surface. Reason
/// strings are wire-visible in 401 bodies, so they are part of the contract.
fn map_validation_error(error: NostrPrimitiveError) -> ProtoError {
    match error {
        NostrPrimitiveError::WrongEventKind { .. } => ProtoError::AuthRejected("wrong event kind"),
        NostrPrimitiveError::StaleTimestamp { .. } => {
            ProtoError::AuthRejected("event timestamp outside window")
        }
        NostrPrimitiveError::UrlMismatch { .. }
        | NostrPrimitiveError::MissingTag { tag: "u" }
        | NostrPrimitiveError::MalformedInput {
            field: "http_auth_url",
        } => ProtoError::AuthRejected("url mismatch"),
        NostrPrimitiveError::MethodMismatch { .. }
        | NostrPrimitiveError::MissingTag { tag: "method" } => {
            ProtoError::AuthRejected("method mismatch")
        }
        NostrPrimitiveError::PayloadMismatch {
            expected: Some(_),
            actual: None,
        } => ProtoError::AuthRejected("missing payload tag"),
        NostrPrimitiveError::PayloadMismatch {
            expected: None,
            actual: Some(_),
        } => ProtoError::AuthRejected("unexpected payload tag"),
        NostrPrimitiveError::PayloadMismatch { .. } => {
            ProtoError::AuthRejected("payload hash mismatch")
        }
        NostrPrimitiveError::InvalidEventId => ProtoError::InvalidEvent("id does not match fields"),
        NostrPrimitiveError::SignatureFailure => ProtoError::InvalidSignature,
        // validate_http_auth_event only emits the variants above for this
        // input shape; anything else is a bug, not an auth decision.
        _ => ProtoError::AuthRejected("auth event rejected"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::pubkey_for_secret;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    const URL: &str = "http://127.0.0.1:8787/api/v1/projects/init";
    const NOW: u64 = 1_750_000_000;

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
        let pubkey = verify_auth_header(&header, URL, "POST", Some(body), NOW + 5).unwrap();
        assert_eq!(pubkey, pubkey_for_secret(&secret(1)).unwrap());
    }

    #[test]
    fn roundtrip_without_body() {
        let header = build_auth_header(&secret(1), URL, "GET", None, NOW).unwrap();
        assert!(verify_auth_header(&header, URL, "GET", None, NOW).is_ok());
    }

    #[test]
    fn empty_body_may_omit_the_payload_tag() {
        // Sites wire leniency: spec-following signers omit the payload tag
        // for empty bodies. The daemon verifies empty POST bodies as
        // Some(b""), so this must pass.
        let header = build_auth_header(&secret(1), URL, "POST", None, NOW).unwrap();
        assert!(verify_auth_header(&header, URL, "POST", Some(b""), NOW).is_ok());
    }

    #[test]
    fn empty_body_may_also_bind_the_empty_hash() {
        // Our own signer binds empty bodies with sha256(b""); both forms
        // are accepted.
        let header = build_auth_header(&secret(1), URL, "POST", Some(b""), NOW).unwrap();
        assert!(verify_auth_header(&header, URL, "POST", Some(b""), NOW).is_ok());
    }

    #[test]
    fn rejects_url_mismatch() {
        let header = build_auth_header(&secret(1), URL, "GET", None, NOW).unwrap();
        assert_eq!(
            verify_auth_header(&header, "http://evil/", "GET", None, NOW),
            Err(ProtoError::AuthRejected("url mismatch"))
        );
    }

    #[test]
    fn rejects_method_mismatch() {
        let header = build_auth_header(&secret(1), URL, "GET", None, NOW).unwrap();
        assert_eq!(
            verify_auth_header(&header, URL, "POST", None, NOW),
            Err(ProtoError::AuthRejected("method mismatch"))
        );
    }

    #[test]
    fn rejects_stale_and_future_events() {
        let header = build_auth_header(&secret(1), URL, "GET", None, NOW).unwrap();
        let too_late = NOW + NIP98_MAX_SKEW_SECONDS + 1;
        let too_early = NOW - NIP98_MAX_SKEW_SECONDS - 1;
        assert_eq!(
            verify_auth_header(&header, URL, "GET", None, too_late),
            Err(ProtoError::AuthRejected("event timestamp outside window"))
        );
        assert_eq!(
            verify_auth_header(&header, URL, "GET", None, too_early),
            Err(ProtoError::AuthRejected("event timestamp outside window"))
        );
    }

    #[test]
    fn rejects_body_tampering() {
        let header = build_auth_header(&secret(1), URL, "POST", Some(b"original"), NOW).unwrap();
        assert_eq!(
            verify_auth_header(&header, URL, "POST", Some(b"tampered"), NOW),
            Err(ProtoError::AuthRejected("payload hash mismatch"))
        );
    }

    #[test]
    fn rejects_missing_payload_tag_for_nonempty_body() {
        let header = build_auth_header(&secret(1), URL, "POST", None, NOW).unwrap();
        assert_eq!(
            verify_auth_header(&header, URL, "POST", Some(b"body"), NOW),
            Err(ProtoError::AuthRejected("missing payload tag"))
        );
    }

    #[test]
    fn rejects_unexpected_payload_tag_on_bodyless_request() {
        let header = build_auth_header(&secret(1), URL, "GET", Some(b"body"), NOW).unwrap();
        assert_eq!(
            verify_auth_header(&header, URL, "GET", None, NOW),
            Err(ProtoError::AuthRejected("unexpected payload tag"))
        );
    }

    #[test]
    fn rejects_wrong_kind() {
        // The kind is checked before the signature, so placeholder id/sig
        // suffice to reach the wrong-kind path.
        let event_json = format!(
            r#"{{"id":"{}","pubkey":"{}","created_at":{NOW},"kind":1,"tags":[["u","{URL}"],["method","GET"]],"content":"","sig":"{}"}}"#,
            "0".repeat(64),
            "1".repeat(64),
            "2".repeat(128),
        );
        let header = format!("{AUTH_SCHEME}{}", BASE64.encode(event_json.as_bytes()));
        assert_eq!(
            verify_auth_header(&header, URL, "GET", None, NOW),
            Err(ProtoError::AuthRejected("wrong event kind"))
        );
    }

    #[test]
    fn rejects_garbage_header() {
        assert!(verify_auth_header("Bearer xyz", URL, "GET", None, NOW).is_err());
        assert!(verify_auth_header("Nostr not-base64!!!", URL, "GET", None, NOW).is_err());
    }

    #[test]
    fn rejects_oversized_header() {
        let pad = "A".repeat(MAX_AUTH_HEADER_BYTES as usize);
        let header = format!("{AUTH_SCHEME}{pad}");
        assert_eq!(
            verify_auth_header(&header, URL, "GET", None, NOW),
            Err(ProtoError::InvalidAuthHeader("header too large"))
        );
    }

    #[test]
    fn rejects_tampered_event_content() {
        // Mutating the signed content invalidates the event id; the kind is
        // still 27235, so the id check is what must fire.
        let header = build_auth_header(&secret(1), URL, "GET", None, NOW).unwrap();
        let mut json: serde_json::Value =
            serde_json::from_str(&decode_raw(&header)).expect("event json");
        json["content"] = serde_json::json!("changed after signing");
        let raw = serde_json::to_vec(&json).unwrap();
        let tampered_header = format!("{AUTH_SCHEME}{}", BASE64.encode(&raw));
        assert_eq!(
            verify_auth_header(&tampered_header, URL, "GET", None, NOW),
            Err(ProtoError::InvalidEvent("id does not match fields"))
        );
    }

    /// Decode the base64 payload of a header into raw event JSON text.
    fn decode_raw(header: &str) -> String {
        let encoded = header.strip_prefix(AUTH_SCHEME).expect("scheme");
        let raw = BASE64.decode(encoded).expect("valid base64");
        String::from_utf8(raw).expect("event json is utf8")
    }
}
