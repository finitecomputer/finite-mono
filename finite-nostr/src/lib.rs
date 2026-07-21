//! Reusable Nostr primitives for Finite Rust projects.
//!
//! This crate intentionally owns generic protocol helpers only. Product
//! policies such as Brain Access or Folder Key Grants belong in
//! application crates.

pub mod auth;
mod error;
pub mod event;
pub mod identity;
pub mod nip05;
pub mod nip44;
pub mod nip59;

#[cfg(test)]
use auth::payload_hash_hex;
pub use auth::{
    HTTP_AUTH_KIND, HTTP_AUTH_SCHEME, HttpAuthEventRequest, HttpAuthValidation,
    decode_http_auth_header, encode_http_auth_header, sign_http_auth_event,
    validate_http_auth_event,
};
pub use error::NostrPrimitiveError;
pub use event::{EventIdHex, compute_event_id, verify_event_integrity};
pub use identity::NostrPublicKey;
pub use nip05::{
    MAX_NIP05_DOCUMENT_BYTES, MAX_NIP05_RELAY_URLS, Nip05Identifier, Nip05WellKnownDocument,
    Nip05WellKnownRequest, VerifiedNip05,
};
pub use nip44::{decrypt_nip44, encrypt_nip44};
pub use nip59::{
    GIFT_WRAP_KIND, GiftWrapValidation, OpenedGiftWrap, SEAL_KIND, build_rumor, open_gift_wrap,
    seal_rumor, validate_gift_wrap, validate_rumor, validate_seal, wrap_rumor,
};

/// Returns the crate version embedded at build time.
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    use nostr::event::{EventBuilder, FinalizeEvent, FinalizeUnsignedEvent};
    use nostr::secp256k1::schnorr::Signature;
    use nostr::{Event, Keys, Kind, Tag, Timestamp, UnsignedEvent};

    const SECRET_KEY_HEX: &str = "6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";
    const RECIPIENT_SECRET_KEY_HEX: &str =
        "7b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";
    const OTHER_SECRET_KEY_HEX: &str =
        "5b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";
    const URL: &str = "https://api.finite.test/v1/folders";
    const NOW: u64 = 1_760_000_000;

    #[test]
    fn exposes_crate_version() {
        assert_eq!(crate_version(), "0.1.0");
    }

    #[test]
    fn parses_and_formats_public_keys() {
        let keys = test_keys();
        let key = NostrPublicKey::from_protocol(keys.public_key());
        let hex = key.to_hex();
        let npub = key.to_npub().unwrap();

        assert_eq!(NostrPublicKey::from_hex(&hex).unwrap(), key);
        assert_eq!(NostrPublicKey::parse(&npub).unwrap(), key);
        assert!(npub.starts_with("npub"));
    }

    #[test]
    fn rejects_malformed_identity_input() {
        assert_eq!(
            NostrPublicKey::parse("not-a-key").unwrap_err(),
            NostrPrimitiveError::MalformedInput {
                field: "public_key"
            }
        );
    }

    #[test]
    fn parses_and_verifies_nip05_documents() {
        let key = NostrPublicKey::from_protocol(test_keys().public_key());
        let identifier = Nip05Identifier::parse("alice@example.com").unwrap();
        let document = format!(
            r#"{{
                "names": {{"alice": "{}"}},
                "relays": {{"{}": ["wss://relay.example.com"]}}
            }}"#,
            key.to_hex(),
            key.to_hex()
        );

        let verified = Nip05WellKnownDocument::from_json(document.as_bytes())
            .unwrap()
            .verify(&identifier, key)
            .unwrap();

        assert_eq!(verified.identifier(), &identifier);
        assert_eq!(verified.public_key(), key);
        assert_eq!(verified.relays(), &["wss://relay.example.com".to_owned()]);
    }

    #[test]
    fn parses_nip05_domain_shorthand_as_root_name() {
        let identifier = Nip05Identifier::parse("Finite.Test").unwrap();

        assert_eq!(identifier.as_str(), "_@finite.test");
        assert_eq!(identifier.display_name(), "finite.test");
        assert_eq!(
            identifier.well_known_request(),
            Nip05WellKnownRequest {
                url: "https://finite.test/.well-known/nostr.json?name=_".to_owned(),
                max_response_bytes: MAX_NIP05_DOCUMENT_BYTES,
                follow_redirects: false,
            }
        );
    }

    #[test]
    fn computes_deterministic_event_ids() {
        let keys = test_keys();
        let unsigned = EventBuilder::new(Kind::TextNote, "portable v1")
            .custom_created_at(Timestamp::from_secs(NOW))
            .finalize_unsigned(keys.public_key());

        let first = compute_event_id(&unsigned);
        let second = compute_event_id(&unsigned);

        assert_eq!(first, second);
        assert_eq!(EventIdHex::parse(first.as_str()).unwrap(), first);
    }

    #[test]
    fn validates_http_auth_event() {
        let keys = test_keys();
        let event = signed_auth_event(&keys, "GET", URL, NOW, None);
        let expected = HttpAuthValidation::new("GET", URL, NOW, 60)
            .with_expected_signer(NostrPublicKey::from_protocol(keys.public_key()));

        let signer = validate_http_auth_event(&event, &expected).unwrap();

        assert_eq!(signer, NostrPublicKey::from_protocol(keys.public_key()));
    }

    #[test]
    fn validates_http_auth_event_with_payload_hash() {
        let keys = test_keys();
        let event = signed_auth_event(&keys, "POST", URL, NOW, Some(b"{\"name\":\"a\"}"));
        let expected =
            HttpAuthValidation::new("POST", URL, NOW, 60).with_body(b"{\"name\":\"a\"}".to_vec());

        validate_http_auth_event(&event, &expected).unwrap();
    }

    #[test]
    fn validates_http_auth_event_with_delete_method() {
        let keys = test_keys();
        let event = signed_auth_event(&keys, "DELETE", URL, NOW, Some(b"{\"delete\":true}"));
        let expected = HttpAuthValidation::new("DELETE", URL, NOW, 60)
            .with_body(b"{\"delete\":true}".to_vec());

        validate_http_auth_event(&event, &expected).unwrap();
    }

    #[test]
    fn signs_and_round_trips_http_auth_header() {
        let keys = test_keys();
        let body = b"{\"name\":\"a\"}";
        let request = HttpAuthEventRequest::new("POST", URL, NOW)
            .with_body(body.to_vec())
            .with_nonce("nonce-1");

        let event = sign_http_auth_event(&keys, &request).unwrap();
        let header = encode_http_auth_header(&event);
        let decoded = decode_http_auth_header(&header).unwrap();
        let expected = HttpAuthValidation::new("POST", URL, NOW, 60).with_body(body.to_vec());

        assert_eq!(decoded, event);
        assert_eq!(
            validate_http_auth_event(&decoded, &expected).unwrap(),
            NostrPublicKey::from_protocol(keys.public_key())
        );
        assert_eq!(tag_content(&decoded, "nonce").as_deref(), Some("nonce-1"));
    }

    #[test]
    fn rejects_malformed_http_auth_headers() {
        assert_eq!(
            decode_http_auth_header("Bearer abc").unwrap_err(),
            NostrPrimitiveError::MalformedInput {
                field: "http_auth_header"
            }
        );
        assert_eq!(
            decode_http_auth_header("Nostr not-base64").unwrap_err(),
            NostrPrimitiveError::MalformedInput {
                field: "http_auth_header"
            }
        );
    }

    #[test]
    fn rejects_wrong_method() {
        let keys = test_keys();
        let event = signed_auth_event(&keys, "GET", URL, NOW, None);
        let expected = HttpAuthValidation::new("POST", URL, NOW, 60);

        assert_eq!(
            validate_http_auth_event(&event, &expected).unwrap_err(),
            NostrPrimitiveError::MethodMismatch {
                expected: "POST".to_string(),
                actual: "GET".to_string()
            }
        );
    }

    #[test]
    fn rejects_wrong_url() {
        let keys = test_keys();
        let event = signed_auth_event(&keys, "GET", URL, NOW, None);
        let expected = HttpAuthValidation::new("GET", "https://api.finite.test/v1/other", NOW, 60);

        assert_eq!(
            validate_http_auth_event(&event, &expected).unwrap_err(),
            NostrPrimitiveError::UrlMismatch {
                expected: "https://api.finite.test/v1/other".to_string(),
                actual: URL.to_string()
            }
        );
    }

    #[test]
    fn rejects_stale_timestamp() {
        let keys = test_keys();
        let event = signed_auth_event(&keys, "GET", URL, NOW - 61, None);
        let expected = HttpAuthValidation::new("GET", URL, NOW, 60);

        assert_eq!(
            validate_http_auth_event(&event, &expected).unwrap_err(),
            NostrPrimitiveError::StaleTimestamp {
                now: NOW,
                created_at: NOW - 61,
                max_skew_seconds: 60
            }
        );
    }

    #[test]
    fn rejects_bad_payload_hash() {
        let keys = test_keys();
        let event = signed_auth_event(&keys, "POST", URL, NOW, Some(b"{\"name\":\"a\"}"));
        let expected =
            HttpAuthValidation::new("POST", URL, NOW, 60).with_body(b"{\"name\":\"b\"}".to_vec());

        assert_eq!(
            validate_http_auth_event(&event, &expected).unwrap_err(),
            NostrPrimitiveError::PayloadMismatch {
                expected: Some(payload_hash_hex(b"{\"name\":\"b\"}")),
                actual: Some(payload_hash_hex(b"{\"name\":\"a\"}"))
            }
        );
    }

    #[test]
    fn rejects_invalid_event_id() {
        let keys = test_keys();
        let mut event = signed_auth_event(&keys, "GET", URL, NOW, None);
        event.content = "changed after signing".to_string();
        let expected = HttpAuthValidation::new("GET", URL, NOW, 60);

        assert_eq!(
            validate_http_auth_event(&event, &expected).unwrap_err(),
            NostrPrimitiveError::InvalidEventId
        );
    }

    #[test]
    fn rejects_bad_signature() {
        let keys = test_keys();
        let mut event = signed_auth_event(&keys, "GET", URL, NOW, None);
        event.sig = Signature::from_slice(&[1_u8; 64]).unwrap();
        let expected = HttpAuthValidation::new("GET", URL, NOW, 60);

        assert_eq!(
            validate_http_auth_event(&event, &expected).unwrap_err(),
            NostrPrimitiveError::SignatureFailure
        );
    }

    #[test]
    fn rejects_wrong_kind() {
        let keys = test_keys();
        let event = EventBuilder::new(Kind::TextNote, "not auth")
            .custom_created_at(Timestamp::from_secs(NOW))
            .finalize(&keys)
            .unwrap();
        let expected = HttpAuthValidation::new("GET", URL, NOW, 60);

        assert_eq!(
            validate_http_auth_event(&event, &expected).unwrap_err(),
            NostrPrimitiveError::WrongEventKind {
                expected: HTTP_AUTH_KIND,
                actual: 1
            }
        );
    }

    #[test]
    fn rejects_signer_mismatch() {
        let keys = test_keys();
        let other_keys = Keys::generate();
        let event = signed_auth_event(&keys, "GET", URL, NOW, None);
        let expected = HttpAuthValidation::new("GET", URL, NOW, 60)
            .with_expected_signer(NostrPublicKey::from_protocol(other_keys.public_key()));

        assert_eq!(
            validate_http_auth_event(&event, &expected).unwrap_err(),
            NostrPrimitiveError::SignerMismatch {
                expected: other_keys.public_key().to_hex(),
                actual: keys.public_key().to_hex()
            }
        );
    }

    #[test]
    fn encrypts_and_decrypts_nip44_payloads() {
        let sender = test_keys();
        let recipient = recipient_keys();
        let ciphertext = encrypt_nip44(
            sender.secret_key(),
            NostrPublicKey::from_protocol(recipient.public_key()),
            "hello sealed world",
        )
        .unwrap();

        let plaintext = decrypt_nip44(
            recipient.secret_key(),
            NostrPublicKey::from_protocol(sender.public_key()),
            ciphertext,
        )
        .unwrap();

        assert_eq!(plaintext, "hello sealed world");
    }

    #[test]
    fn wraps_and_opens_nip59_gift_wrap() {
        let sender = test_keys();
        let recipient = recipient_keys();
        let rumor = text_rumor(&sender, "Test rumor");

        let gift_wrap = wrap_rumor(
            &sender,
            NostrPublicKey::from_protocol(recipient.public_key()),
            rumor.clone(),
        )
        .unwrap();

        let opened = open_gift_wrap(
            &recipient,
            &gift_wrap,
            &GiftWrapValidation::new(NostrPublicKey::from_protocol(recipient.public_key()))
                .with_expected_issuer(NostrPublicKey::from_protocol(sender.public_key())),
        )
        .unwrap();

        assert_eq!(
            opened.sender,
            NostrPublicKey::from_protocol(sender.public_key())
        );
        assert_eq!(
            opened.recipient,
            NostrPublicKey::from_protocol(recipient.public_key())
        );
        assert_eq!(opened.rumor, rumor);
        assert_eq!(opened.seal.kind, Kind::Seal);
        validate_gift_wrap(
            &gift_wrap,
            NostrPublicKey::from_protocol(recipient.public_key()),
        )
        .unwrap();
    }

    #[test]
    fn seals_and_validates_nip59_rumor() {
        let sender = test_keys();
        let recipient = recipient_keys();
        let rumor = text_rumor(&sender, "sealed");

        validate_rumor(
            &rumor,
            Some(NostrPublicKey::from_protocol(sender.public_key())),
        )
        .unwrap();

        let seal = seal_rumor(
            &sender,
            NostrPublicKey::from_protocol(recipient.public_key()),
            rumor,
        )
        .unwrap();

        validate_seal(
            &seal,
            Some(NostrPublicKey::from_protocol(sender.public_key())),
        )
        .unwrap();
    }

    #[test]
    fn rejects_wrong_gift_wrap_kind() {
        let sender = test_keys();
        let recipient = recipient_keys();
        let event = EventBuilder::new(Kind::TextNote, "not a gift wrap")
            .finalize(&sender)
            .unwrap();

        assert_eq!(
            validate_gift_wrap(
                &event,
                NostrPublicKey::from_protocol(recipient.public_key())
            )
            .unwrap_err(),
            NostrPrimitiveError::WrongEventKind {
                expected: GIFT_WRAP_KIND,
                actual: 1
            }
        );
    }

    #[test]
    fn rejects_wrong_gift_wrap_recipient() {
        let sender = test_keys();
        let recipient = recipient_keys();
        let other = other_keys();
        let gift_wrap = wrap_rumor(
            &sender,
            NostrPublicKey::from_protocol(recipient.public_key()),
            text_rumor(&sender, "private"),
        )
        .unwrap();

        assert_eq!(
            validate_gift_wrap(
                &gift_wrap,
                NostrPublicKey::from_protocol(other.public_key())
            )
            .unwrap_err(),
            NostrPrimitiveError::WrongRecipient {
                expected: other.public_key().to_hex(),
                actual: vec![recipient.public_key().to_hex()]
            }
        );
    }

    #[test]
    fn rejects_wrong_gift_wrap_issuer() {
        let sender = test_keys();
        let recipient = recipient_keys();
        let other = other_keys();
        let gift_wrap = wrap_rumor(
            &sender,
            NostrPublicKey::from_protocol(recipient.public_key()),
            text_rumor(&sender, "private"),
        )
        .unwrap();

        assert_eq!(
            open_gift_wrap(
                &recipient,
                &gift_wrap,
                &GiftWrapValidation::new(NostrPublicKey::from_protocol(recipient.public_key()))
                    .with_expected_issuer(NostrPublicKey::from_protocol(other.public_key())),
            )
            .unwrap_err(),
            NostrPrimitiveError::WrongIssuer {
                expected: other.public_key().to_hex(),
                actual: sender.public_key().to_hex()
            }
        );
    }

    #[test]
    fn rejects_malformed_seal_plaintext() {
        let recipient = recipient_keys();
        let gift_wrap = gift_wrap_with_plaintext_seal(&recipient, "not-json");

        assert_eq!(
            open_gift_wrap(
                &recipient,
                &gift_wrap,
                &GiftWrapValidation::new(NostrPublicKey::from_protocol(recipient.public_key())),
            )
            .unwrap_err(),
            NostrPrimitiveError::MalformedPlaintext { field: "seal" }
        );
    }

    #[test]
    fn rejects_malformed_rumor_plaintext() {
        let sender = test_keys();
        let recipient = recipient_keys();
        let gift_wrap = gift_wrap_with_plaintext_rumor(&sender, &recipient, "not-json");

        assert_eq!(
            open_gift_wrap(
                &recipient,
                &gift_wrap,
                &GiftWrapValidation::new(NostrPublicKey::from_protocol(recipient.public_key()))
                    .with_expected_issuer(NostrPublicKey::from_protocol(sender.public_key())),
            )
            .unwrap_err(),
            NostrPrimitiveError::MalformedPlaintext { field: "rumor" }
        );
    }

    #[test]
    fn rejects_nip59_decrypt_failure() {
        let sender = test_keys();
        let recipient = recipient_keys();
        let other = other_keys();
        let gift_wrap = wrap_rumor(
            &sender,
            NostrPublicKey::from_protocol(recipient.public_key()),
            text_rumor(&sender, "private"),
        )
        .unwrap();

        assert_eq!(
            open_gift_wrap(
                &other,
                &gift_wrap,
                &GiftWrapValidation::new(NostrPublicKey::from_protocol(recipient.public_key())),
            )
            .unwrap_err(),
            NostrPrimitiveError::FailedDecrypt
        );
    }

    fn test_keys() -> Keys {
        Keys::parse(SECRET_KEY_HEX).unwrap()
    }

    fn recipient_keys() -> Keys {
        Keys::parse(RECIPIENT_SECRET_KEY_HEX).unwrap()
    }

    fn other_keys() -> Keys {
        Keys::parse(OTHER_SECRET_KEY_HEX).unwrap()
    }

    fn text_rumor(keys: &Keys, content: &str) -> UnsignedEvent {
        build_rumor(
            NostrPublicKey::from_protocol(keys.public_key()),
            Kind::TextNote,
            Vec::<Tag>::new(),
            content,
            NOW,
        )
    }

    fn gift_wrap_with_plaintext_seal(recipient: &Keys, seal_plaintext: &str) -> Event {
        let ephemeral = Keys::generate();
        let content = encrypt_nip44(
            ephemeral.secret_key(),
            NostrPublicKey::from_protocol(recipient.public_key()),
            seal_plaintext,
        )
        .unwrap();

        EventBuilder::new(Kind::GiftWrap, content)
            .tag(Tag::public_key(recipient.public_key()))
            .finalize(&ephemeral)
            .unwrap()
    }

    fn gift_wrap_with_plaintext_rumor(
        sender: &Keys,
        recipient: &Keys,
        rumor_plaintext: &str,
    ) -> Event {
        let seal_content = encrypt_nip44(
            sender.secret_key(),
            NostrPublicKey::from_protocol(recipient.public_key()),
            rumor_plaintext,
        )
        .unwrap();
        let seal = EventBuilder::new(Kind::Seal, seal_content)
            .finalize(sender)
            .unwrap();

        gift_wrap_with_plaintext_seal(recipient, &seal.as_json())
    }

    fn signed_auth_event(
        keys: &Keys,
        method: &str,
        url: &str,
        created_at: u64,
        body: Option<&[u8]>,
    ) -> Event {
        let mut tags = vec![tag(["u", url]), tag(["method", method])];
        if let Some(body) = body {
            tags.push(tag(["payload", &payload_hash_hex(body)]));
        }

        EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(created_at))
            .finalize(keys)
            .unwrap()
    }

    fn tag<const N: usize>(parts: [&str; N]) -> Tag {
        Tag::parse(parts.into_iter().map(ToOwned::to_owned).collect::<Vec<_>>()).unwrap()
    }

    fn tag_content(event: &Event, kind: &str) -> Option<String> {
        event
            .tags
            .iter()
            .find(|candidate| candidate.kind() == kind)
            .and_then(Tag::content)
            .map(ToOwned::to_owned)
    }
}
