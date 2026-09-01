//! The gate's own session cookie, scoped to the gate's hostname.
//!
//! This is NOT the Finite Sites viewer cookie. It only remembers that this
//! browser authenticated with the gate, so the next `/authorize` can mint a
//! vouch silently instead of sending the human through WorkOS again. The
//! signing key is domain-separated from vouch minting.

use axum::http::{HeaderMap, header};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::state::GateState;

type HmacSha256 = Hmac<Sha256>;

const SESSION_COOKIE_NAME: &str = "finite_gate_session";
const SESSION_VERSION: &str = "gate-session-v1";
/// One WorkOS login per browser per week.
const SESSION_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;

/// Derive the cookie key from the gate signing key with domain separation,
/// so the vouch signer and the cookie signer are never the same key.
fn session_key(signing_key: &[u8; 32]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(signing_key).expect("hmac accepts 32-byte keys");
    mac.update(SESSION_VERSION.as_bytes());
    let derived = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&derived);
    key
}

fn sign_email(key: &[u8; 32], email: &str, expires_at: u64) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac accepts 32-byte keys");
    mac.update(email.as_bytes());
    mac.update(&expires_at.to_be_bytes());
    BASE64URL.encode(mac.finalize().into_bytes())
}

/// Mint the gate session cookie value for an authenticated email.
/// Structure: `email.expires_at.signature`, parsed from the right because
/// local parts and domains contain dots.
pub fn mint_cookie_value(signing_key: &[u8; 32], email: &str, now: u64) -> String {
    let expires_at = now + SESSION_TTL_SECONDS;
    format!(
        "{email}.{expires_at}.{}",
        sign_email(&session_key(signing_key), email, expires_at)
    )
}

/// Verify a cookie header's session value; returns the email when valid.
pub fn verify_cookie_value(signing_key: &[u8; 32], value: &str, now: u64) -> Option<String> {
    if value.len() > 512 {
        return None;
    }
    let (head, signature) = value.rsplit_once('.')?;
    let (email, expires_text) = head.rsplit_once('.')?;
    if email.is_empty() || !email.contains('@') {
        return None;
    }
    let expires_at: u64 = expires_text.parse().ok()?;
    if now > expires_at {
        return None;
    }
    let key = session_key(signing_key);
    let claimed_bytes = BASE64URL.decode(signature).ok()?;
    let mut mac = HmacSha256::new_from_slice(&key).expect("hmac accepts 32-byte keys");
    mac.update(email.as_bytes());
    mac.update(&expires_at.to_be_bytes());
    // Constant-time comparison via the hmac crate.
    mac.verify_slice(&claimed_bytes).ok()?;
    Some(email.to_string())
}

/// The authenticated email for this request, if the gate session cookie is
/// present and valid.
pub fn read_email(state: &GateState, headers: &HeaderMap, now: u64) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in cookie_header.split(';') {
        let trimmed = pair.trim();
        if let Some(value) = trimmed.strip_prefix(SESSION_COOKIE_NAME)
            && let Some(value) = value.strip_prefix('=')
        {
            return verify_cookie_value(&state.config.signing_key, value, now);
        }
    }
    None
}

/// Attach the session cookie to a redirect response.
pub fn set_cookie(response: &mut Response, state: &GateState, email: &str, now: u64) {
    let secure = state.config.public_url.starts_with("https://");
    let cookie = format!(
        "{SESSION_COOKIE_NAME}={}; Path=/; Max-Age={SESSION_TTL_SECONDS}; HttpOnly; SameSite=Lax{}",
        mint_cookie_value(&state.config.signing_key, email, now),
        if secure { "; Secure" } else { "" }
    );
    if let Ok(value) = cookie.parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [9; 32];
    const NOW: u64 = 1_750_000_000;

    #[test]
    fn cookie_roundtrip_and_expiry() {
        let value = mint_cookie_value(&KEY, "viewer@example.com", NOW);
        assert_eq!(
            verify_cookie_value(&KEY, &value, NOW + 1),
            Some("viewer@example.com".to_string())
        );
        assert_eq!(
            verify_cookie_value(&KEY, &value, NOW + SESSION_TTL_SECONDS + 1),
            None
        );
    }

    #[test]
    fn cookie_rejects_wrong_key_and_tampering() {
        let value = mint_cookie_value(&KEY, "viewer@example.com", NOW);
        assert_eq!(verify_cookie_value(&[8; 32], &value, NOW), None);
        let tampered = format!("attacker@example.com.{}", value.split_once('.').unwrap().1);
        assert_eq!(verify_cookie_value(&KEY, &tampered, NOW), None);
        assert_eq!(verify_cookie_value(&KEY, "garbage", NOW), None);
        assert_eq!(verify_cookie_value(&KEY, "", NOW), None);
    }
}
