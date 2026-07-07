//! Finite auth domain types and Nostr validation policy.
//!
//! This crate owns Finite authentication policy. Reusable Nostr protocol
//! validation belongs in `finite-nostr`.

mod error;
mod frostr;
mod http;
mod nip05;
mod session;

pub use error::AuthError;
pub use frostr::{
    AgentNostrKeyBinding, FINITE_FROSTR_MEMBER_COUNT, FINITE_FROSTR_THRESHOLD, FrostrKeysetPlan,
    FrostrKeysetRecord, FrostrKeysetStatus, FrostrSharePackageRef, FrostrSharePlacement,
    FrostrShareRole,
};
pub use http::{NostrHttpAuthRequest, VerifiedNostrAuth, authenticate_nostr_http_header};
pub use nip05::{
    MAX_NIP05_DOCUMENT_BYTES, MAX_NIP05_RELAY_URLS, Nip05Identifier, Nip05WellKnownDocument,
    Nip05WellKnownRequest, VerifiedNip05,
};
pub use session::{
    AuthChallenge, AuthNonce, AuthPrincipal, AuthSessionRecord, SessionId, SessionToken,
    SessionTokenHash,
};

/// Returns the crate name used in workspace status surfaces.
pub fn crate_name() -> &'static str {
    "finite-auth-core"
}

#[cfg(test)]
mod tests {
    use super::*;

    use finite_nostr::{HttpAuthEventRequest, encode_http_auth_header, sign_http_auth_event};
    use nostr::Keys;

    const SECRET_KEY_HEX: &str = "6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";
    const AGENT_SECRET_KEY_HEX: &str =
        "5b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";
    const URL: &str = "https://api.finite.test/v1/auth/session";
    const NOW: u64 = 1_760_000_000;
    const NONCE: &str = "auth-nonce-00001";

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "finite-auth-core");
    }

    #[test]
    fn authenticates_nostr_http_header_with_expected_nonce() {
        let keys = Keys::parse(SECRET_KEY_HEX).unwrap();
        let signer = finite_nostr::NostrPublicKey::from_protocol(keys.public_key());
        let body = br#"{"intent":"login"}"#;
        let sign_request = HttpAuthEventRequest::new("POST", URL, NOW)
            .with_body(body.to_vec())
            .with_nonce(NONCE);
        let event = sign_http_auth_event(&keys, &sign_request).unwrap();
        let header = encode_http_auth_header(&event);
        let expected = NostrHttpAuthRequest::new("POST", URL, NOW, 60)
            .unwrap()
            .with_body(body.to_vec())
            .with_expected_nonce(AuthNonce::new(NONCE).unwrap());

        let verified = authenticate_nostr_http_header(&header, &expected).unwrap();

        assert_eq!(verified.signer(), signer);
        assert_eq!(verified.nonce().unwrap().as_str(), NONCE);
        assert_eq!(verified.created_at_unix_seconds(), NOW);
    }

    #[test]
    fn rejects_nostr_http_header_with_wrong_nonce() {
        let keys = Keys::parse(SECRET_KEY_HEX).unwrap();
        let sign_request = HttpAuthEventRequest::new("GET", URL, NOW).with_nonce(NONCE);
        let event = sign_http_auth_event(&keys, &sign_request).unwrap();
        let header = encode_http_auth_header(&event);
        let expected = NostrHttpAuthRequest::new("GET", URL, NOW, 60)
            .unwrap()
            .with_expected_nonce(AuthNonce::new("auth-nonce-00002").unwrap());

        assert_eq!(
            authenticate_nostr_http_header(&header, &expected).unwrap_err(),
            AuthError::NonceMismatch {
                expected: "auth-nonce-00002".to_string(),
                actual: Some(NONCE.to_string())
            }
        );
    }

    #[test]
    fn verifies_nip05_document_binding() {
        let keys = Keys::parse(SECRET_KEY_HEX).unwrap();
        let public_key = finite_nostr::NostrPublicKey::from_protocol(keys.public_key());
        let identifier = Nip05Identifier::parse("alice@example.com").unwrap();
        let document = format!(
            r#"{{
                "names": {{"alice": "{}"}},
                "relays": {{"{}": ["wss://relay.example.com"]}}
            }}"#,
            public_key.to_hex(),
            public_key.to_hex()
        );

        let verified = Nip05WellKnownDocument::from_json(document.as_bytes())
            .unwrap()
            .verify(&identifier, public_key)
            .unwrap();

        assert_eq!(verified.identifier(), &identifier);
        assert_eq!(verified.public_key(), public_key);
        assert_eq!(verified.relays(), &["wss://relay.example.com".to_string()]);
    }

    #[test]
    fn keeps_nip05_identifier_from_replacing_public_key() {
        let identifier = Nip05Identifier::parse("_@finite.test").unwrap();

        assert_eq!(identifier.as_str(), "_@finite.test");
        assert_eq!(identifier.display_name(), "finite.test");
        assert_eq!(
            identifier.well_known_request(),
            Nip05WellKnownRequest {
                url: "https://finite.test/.well-known/nostr.json?name=_".to_string(),
                max_response_bytes: MAX_NIP05_DOCUMENT_BYTES,
                follow_redirects: false
            }
        );
    }

    #[test]
    fn accepts_fixed_two_of_three_frostr_share_plan() {
        let group_key = test_public_key(SECRET_KEY_HEX);
        let plan = test_frostr_plan(group_key);

        assert_eq!(plan.group_public_key(), group_key);
        assert_eq!(plan.threshold(), FINITE_FROSTR_THRESHOLD);
        assert_eq!(plan.member_count(), FINITE_FROSTR_MEMBER_COUNT);
        assert_eq!(
            plan.share_for_role(FrostrShareRole::Server)
                .unwrap()
                .member_index(),
            1
        );
    }

    #[test]
    fn rejects_frostr_plan_missing_required_role() {
        let group_key = test_public_key(SECRET_KEY_HEX);
        let shares = vec![
            FrostrSharePlacement::new(
                FrostrShareRole::Server,
                1,
                FrostrSharePackageRef::new("server-share-ref-0001").unwrap(),
            )
            .unwrap(),
            FrostrSharePlacement::new(
                FrostrShareRole::UserClient,
                2,
                FrostrSharePackageRef::new("client-share-ref-0001").unwrap(),
            )
            .unwrap(),
        ];

        assert_eq!(
            FrostrKeysetPlan::new(group_key, shares).unwrap_err(),
            AuthError::InvalidInput {
                field: "frostr_share_roles",
                reason: "expected server, user_client, and native_secure_storage shares"
                    .to_string()
            }
        );
    }

    #[test]
    fn rejects_short_frostr_share_package_ref_with_precise_error() {
        assert_eq!(
            FrostrSharePackageRef::new("short").unwrap_err(),
            AuthError::InvalidInput {
                field: "frostr_share_package_ref",
                reason: "expected at least 8 characters".to_string()
            }
        );
    }

    #[test]
    fn rejects_agent_key_matching_user_key() {
        let group_key = test_public_key(SECRET_KEY_HEX);

        assert_eq!(
            AgentNostrKeyBinding::new(group_key, group_key, NOW).unwrap_err(),
            AuthError::InvalidInput {
                field: "agent_public_key",
                reason: "agent key must differ from user primary key".to_string()
            }
        );
    }

    #[test]
    fn creates_agent_key_binding_for_user_primary_key() {
        let group_key = test_public_key(SECRET_KEY_HEX);
        let agent_key = test_public_key(AGENT_SECRET_KEY_HEX);
        let binding = AgentNostrKeyBinding::new(group_key, agent_key, NOW).unwrap();

        assert_eq!(binding.user_public_key(), group_key);
        assert_eq!(binding.agent_public_key(), agent_key);
        assert_eq!(binding.created_at_unix_seconds(), NOW);
        assert!(!binding.is_revoked());
    }

    fn test_frostr_plan(group_key: finite_nostr::NostrPublicKey) -> FrostrKeysetPlan {
        FrostrKeysetPlan::new(
            group_key,
            vec![
                FrostrSharePlacement::new(
                    FrostrShareRole::Server,
                    1,
                    FrostrSharePackageRef::new("server-share-ref-0001").unwrap(),
                )
                .unwrap(),
                FrostrSharePlacement::new(
                    FrostrShareRole::UserClient,
                    2,
                    FrostrSharePackageRef::new("client-share-ref-0001").unwrap(),
                )
                .unwrap(),
                FrostrSharePlacement::new(
                    FrostrShareRole::NativeSecureStorage,
                    3,
                    FrostrSharePackageRef::new("native-share-ref-0001").unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn test_public_key(secret_key_hex: &str) -> finite_nostr::NostrPublicKey {
        let keys = Keys::parse(secret_key_hex).unwrap();
        finite_nostr::NostrPublicKey::from_protocol(keys.public_key())
    }
}
