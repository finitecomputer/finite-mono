//! Gate vouches: statement kind 2 — "the gate authenticated this human for
//! this origin just now".
//!
//! A vouch is a short-lived, origin-bound, single-use, versioned signed
//! statement minted by the Auth Gate after WorkOS verifies a human, and
//! consumed by exactly one browser redirect back to the output origin, whose
//! daemon verifies it OFFLINE against a pinned gate public key.
//!
//! Wire format (deliberately JWT-shaped but minimal):
//!
//! ```text
//! vouch := base64url(payload_json) "." base64url(schnorr_signature)
//! payload_json := {
//!   "v": 1,                 // format version; npub claims arrive as v1 additions
//!   "iss": "finite-gate",
//!   "aud": "<output origin>",   // e.g. "https://hello.finite.chat" — a vouch for
//!                               // one output is not a passport to others
//!   "email": "<verified email attribute>",
//!   "npub": null,               // reserved; unset in v1
//!   "iat": <unix seconds>,
//!   "exp": <unix seconds>,      // iat + policy.vouch_ttl_seconds (~60s)
//!   "jti": "<32-byte hex nonce>"// consumer-side replay rejection
//! }
//! ```
//!
//! The signature is schnorr (secp256k1, same primitive as NIP-98 events)
//! over `sha256(payload_segment)` where `payload_segment` is the exact
//! base64url string transmitted. Verification never re-serializes the
//! claims, so no canonicalization ambiguity exists.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use secp256k1::schnorr::Signature;
use secp256k1::{Keypair, Message, Secp256k1, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AuthPolicy;
use crate::{AuthnError, hex};

/// Current vouch format version.
pub const VOUCH_VERSION: u32 = 1;
/// `iss` value for vouches minted by the Finite Auth Gate.
pub const VOUCH_ISSUER: &str = "finite-gate";
/// Whole-envelope bound; a vouch is a few bounded strings, never a document.
pub const MAX_VOUCH_BYTES: usize = 2 * 1024;
pub const MAX_VOUCH_AUDIENCE_BYTES: usize = 2 * 1024;
pub const MAX_VOUCH_EMAIL_BYTES: usize = 254;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VouchClaims {
    pub v: u32,
    pub iss: String,
    pub aud: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npub: Option<String>,
    pub iat: u64,
    pub exp: u64,
    pub jti: String,
}

/// The x-only public key (64 lowercase hex chars) a verifier pins for the
/// gate; the counterpart of the gate's signing secret.
pub fn gate_pubkey_for_secret(secret_key: &[u8; 32]) -> Result<String, AuthnError> {
    crate::event::pubkey_for_secret(secret_key)
}

/// Mint a vouch for one verified email on one output origin. `nonce` is OS
/// randomness supplied by the caller (the crate stays pure); it becomes the
/// `jti` replay token.
pub fn mint_vouch(
    gate_secret_key: &[u8; 32],
    audience: &str,
    email: &str,
    now_unix: u64,
    policy: &AuthPolicy,
    nonce: [u8; 32],
) -> Result<String, AuthnError> {
    if audience.is_empty() || audience.len() > MAX_VOUCH_AUDIENCE_BYTES {
        return Err(AuthnError::InvalidVouch("audience out of bounds"));
    }
    if email.is_empty() || email.len() > MAX_VOUCH_EMAIL_BYTES {
        return Err(AuthnError::InvalidVouch("email out of bounds"));
    }
    let claims = VouchClaims {
        v: VOUCH_VERSION,
        iss: VOUCH_ISSUER.to_string(),
        aud: audience.to_string(),
        email: email.to_string(),
        npub: None,
        iat: now_unix,
        exp: now_unix + policy.vouch_ttl_seconds,
        jti: hex::encode(&nonce),
    };
    let payload = serde_json::to_string(&claims).map_err(|_| AuthnError::InvalidVouch("encode"))?;
    let payload_segment = BASE64URL.encode(payload.as_bytes());
    let signature = sign_payload(gate_secret_key, payload_segment.as_bytes())?;
    let vouch = format!("{payload_segment}.{signature}");
    if vouch.len() > MAX_VOUCH_BYTES {
        return Err(AuthnError::InvalidVouch("vouch out of bounds"));
    }
    Ok(vouch)
}

/// Verify a vouch offline against the pinned gate public key. The audience
/// must match the verifier's own output origin exactly, and the vouch must
/// be inside its TTL (+ skew window). Returns the claims (with the `jti`
/// the caller must replay-check).
pub fn verify_vouch(
    vouch: &str,
    gate_pubkey: &str,
    expected_audience: &str,
    now_unix: u64,
    policy: &AuthPolicy,
) -> Result<VouchClaims, AuthnError> {
    if vouch.len() > MAX_VOUCH_BYTES {
        return Err(AuthnError::InvalidVouch("vouch too large"));
    }
    let Some((payload_segment, signature_segment)) = vouch.split_once('.') else {
        return Err(AuthnError::InvalidVouch("missing signature"));
    };
    let payload_bytes = BASE64URL
        .decode(payload_segment)
        .map_err(|_| AuthnError::InvalidVouch("invalid payload encoding"))?;
    let signature_bytes = BASE64URL
        .decode(signature_segment)
        .map_err(|_| AuthnError::InvalidVouch("invalid signature encoding"))?;
    let claims: VouchClaims = serde_json::from_slice(&payload_bytes)
        .map_err(|_| AuthnError::InvalidVouch("invalid claims"))?;
    if claims.v != VOUCH_VERSION {
        return Err(AuthnError::InvalidVouch("unsupported version"));
    }
    if claims.iss != VOUCH_ISSUER {
        return Err(AuthnError::InvalidVouch("unknown issuer"));
    }
    if claims.aud != expected_audience {
        return Err(AuthnError::InvalidVouch("audience mismatch"));
    }
    if claims.email.is_empty() || claims.email.len() > MAX_VOUCH_EMAIL_BYTES {
        return Err(AuthnError::InvalidVouch("email out of bounds"));
    }
    if claims.jti.len() != 64 || !hex::is_hex32(&claims.jti) {
        return Err(AuthnError::InvalidVouch("invalid nonce"));
    }
    if claims.exp < claims.iat || claims.exp > claims.iat + policy.vouch_ttl_seconds {
        return Err(AuthnError::InvalidVouch("expansion outside policy"));
    }
    let oldest_acceptable = now_unix.saturating_sub(policy.vouch_max_skew_seconds);
    let newest_acceptable = now_unix.saturating_add(policy.vouch_max_skew_seconds);
    if claims.iat > newest_acceptable || claims.exp < oldest_acceptable {
        return Err(AuthnError::InvalidVouch("outside freshness window"));
    }

    let pubkey_bytes = hex::decode32(gate_pubkey)?;
    let xonly = XOnlyPublicKey::from_slice(&pubkey_bytes)
        .map_err(|_| AuthnError::InvalidVouch("gate pubkey is not a valid x-only point"))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| AuthnError::InvalidVouch("invalid signature"))?;
    let digest = Sha256::digest(payload_segment.as_bytes());
    let message = Message::from_digest(digest.into());
    let secp = Secp256k1::verification_only();
    if secp.verify_schnorr(&signature, &message, &xonly).is_err() {
        return Err(AuthnError::InvalidVouch("signature does not verify"));
    }
    Ok(claims)
}

fn sign_payload(secret_key: &[u8; 32], payload: &[u8]) -> Result<String, AuthnError> {
    let secp = Secp256k1::new();
    let keypair = Keypair::from_seckey_slice(&secp, secret_key)
        .map_err(|_| AuthnError::InvalidVouch("invalid gate key"))?;
    let digest = Sha256::digest(payload);
    let message = Message::from_digest(digest.into());
    let signature = secp.sign_schnorr_no_aux_rand(&message, &keypair);
    Ok(BASE64URL.encode(signature.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GATE_SECRET: [u8; 32] = [0x0b; 32];
    const OTHER_SECRET: [u8; 32] = [0x0c; 32];
    const AUD: &str = "https://hello.finite.chat";
    const EMAIL: &str = "viewer@example.com";
    const NOW: u64 = 1_750_000_000;

    fn nonce() -> [u8; 32] {
        [0x42; 32]
    }

    fn mint(aud: &str, email: &str, now: u64) -> String {
        mint_vouch(
            &GATE_SECRET,
            aud,
            email,
            now,
            &AuthPolicy::default(),
            nonce(),
        )
        .unwrap()
    }

    fn verify(vouch: &str, aud: &str, now: u64) -> Result<VouchClaims, AuthnError> {
        let pubkey = gate_pubkey_for_secret(&GATE_SECRET).unwrap();
        verify_vouch(vouch, &pubkey, aud, now, &AuthPolicy::default())
    }

    #[test]
    fn mint_verify_roundtrip() {
        let vouch = mint(AUD, EMAIL, NOW);
        let claims = verify(&vouch, AUD, NOW + 10).unwrap();
        assert_eq!(claims.email, EMAIL);
        assert_eq!(claims.aud, AUD);
        assert_eq!(claims.exp - claims.iat, 60);
        assert_eq!(claims.jti.len(), 64);
        assert_eq!(claims.iss, "finite-gate");
        assert_eq!(claims.v, 1);
    }

    #[test]
    fn a_vouch_for_one_origin_is_not_a_passport_to_others() {
        let vouch = mint(AUD, EMAIL, NOW);
        assert_eq!(
            verify(&vouch, "https://other.finite.chat", NOW),
            Err(AuthnError::InvalidVouch("audience mismatch"))
        );
        // Scheme and port are part of the origin.
        assert_eq!(
            verify(&vouch, "http://hello.finite.chat", NOW),
            Err(AuthnError::InvalidVouch("audience mismatch"))
        );
    }

    #[test]
    fn expires_after_ttl_and_skew() {
        let vouch = mint(AUD, EMAIL, NOW);
        // TTL (60) + skew (60): at +119 the redeem window is still open.
        assert!(verify(&vouch, AUD, NOW + 119).is_ok());
        assert_eq!(
            verify(&vouch, AUD, NOW + 121),
            Err(AuthnError::InvalidVouch("outside freshness window"))
        );
    }

    #[test]
    fn rejects_future_mints() {
        let vouch = mint(AUD, EMAIL, NOW + 10_000);
        assert_eq!(
            verify(&vouch, AUD, NOW),
            Err(AuthnError::InvalidVouch("outside freshness window"))
        );
    }

    #[test]
    fn rejects_wrong_gate_key_and_tampering() {
        let vouch = mint(AUD, EMAIL, NOW);
        let other_pubkey = gate_pubkey_for_secret(&OTHER_SECRET).unwrap();
        assert_eq!(
            verify_vouch(&vouch, &other_pubkey, AUD, NOW, &AuthPolicy::default()),
            Err(AuthnError::InvalidVouch("signature does not verify"))
        );

        // Tamper with the payload (swap in another email).
        let (payload_segment, signature) = vouch.split_once('.').unwrap();
        let payload = BASE64URL.decode(payload_segment).unwrap();
        let mut claims: VouchClaims = serde_json::from_slice(&payload).unwrap();
        claims.email = "attacker@example.com".into();
        let tampered_payload = BASE64URL.encode(serde_json::to_vec(&claims).unwrap());
        let tampered = format!("{tampered_payload}.{signature}");
        assert_eq!(
            verify(&tampered, AUD, NOW),
            Err(AuthnError::InvalidVouch("signature does not verify"))
        );

        assert_eq!(
            verify("garbage", AUD, NOW),
            Err(AuthnError::InvalidVouch("missing signature"))
        );
        assert_eq!(
            verify("!!!.???", AUD, NOW),
            Err(AuthnError::InvalidVouch("invalid payload encoding"))
        );
    }

    #[test]
    fn rejects_stretched_expiry_outside_policy() {
        // A gate bug or malicious mint must not be able to extend the TTL
        // beyond the policy even with a valid signature.
        let claims = VouchClaims {
            v: 1,
            iss: VOUCH_ISSUER.into(),
            aud: AUD.into(),
            email: EMAIL.into(),
            npub: None,
            iat: NOW,
            exp: NOW + 3_600,
            jti: hex::encode(&nonce()),
        };
        let payload_segment = BASE64URL.encode(serde_json::to_vec(&claims).unwrap());
        let signature = sign_payload(&GATE_SECRET, payload_segment.as_bytes()).unwrap();
        let vouch = format!("{payload_segment}.{signature}");
        assert_eq!(
            verify(&vouch, AUD, NOW + 120),
            Err(AuthnError::InvalidVouch("expansion outside policy"))
        );
    }

    #[test]
    fn different_nonces_mint_distinct_vouches() {
        let a = mint_vouch(
            &GATE_SECRET,
            AUD,
            EMAIL,
            NOW,
            &AuthPolicy::default(),
            [1; 32],
        )
        .unwrap();
        let b = mint_vouch(
            &GATE_SECRET,
            AUD,
            EMAIL,
            NOW,
            &AuthPolicy::default(),
            [2; 32],
        )
        .unwrap();
        assert_ne!(a, b);
        assert_ne!(
            verify(&a, AUD, NOW).unwrap().jti,
            verify(&b, AUD, NOW).unwrap().jti
        );
    }
}
