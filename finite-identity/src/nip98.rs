//! NIP-98 HTTP authorization helpers for the Identity Contract.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use secp256k1::global::SECP256K1;
use secp256k1::schnorr::Signature;
use secp256k1::{Keypair, Message, SecretKey, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::hex;

pub const NIP98_KIND: u32 = 27235;
pub const AUTH_SCHEME: &str = "Nostr ";
const MAX_AUTH_HEADER_BYTES: usize = 16 * 1024;
const MAX_SKEW_SECONDS: u64 = 60;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid secret key")]
    InvalidSecret,
    #[error("invalid auth header: {0}")]
    InvalidHeader(&'static str),
    #[error("auth rejected: {0}")]
    Rejected(&'static str),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NostrEvent {
    id: String,
    pubkey: String,
    created_at: u64,
    kind: u32,
    tags: Vec<Vec<String>>,
    content: String,
    sig: String,
}

/// Derive the x-only public key hex for a 32-byte secret key.
pub fn pubkey_for_secret(secret_key: &[u8; 32]) -> Result<String, Error> {
    let secret = SecretKey::from_slice(secret_key).map_err(|_| Error::InvalidSecret)?;
    let keypair = Keypair::from_secret_key(SECP256K1, &secret);
    Ok(hex::encode(&keypair.x_only_public_key().0.serialize()))
}

/// Build a NIP-98 `Authorization` header for one HTTP request.
pub fn build_auth_header(
    secret_key: &[u8; 32],
    url: &str,
    method: &str,
    body: Option<&[u8]>,
    now_unix: u64,
) -> Result<String, Error> {
    assert!(!url.is_empty() && !method.is_empty());
    let secret = SecretKey::from_slice(secret_key).map_err(|_| Error::InvalidSecret)?;
    let keypair = Keypair::from_secret_key(SECP256K1, &secret);
    let pubkey = hex::encode(&keypair.x_only_public_key().0.serialize());
    let mut tags = vec![
        vec!["u".to_owned(), url.to_owned()],
        vec!["method".to_owned(), method.to_owned()],
    ];
    if let Some(body_bytes) = body {
        tags.push(vec![
            "payload".to_owned(),
            hex::encode(&Sha256::digest(body_bytes)),
        ]);
    }
    let mut event = NostrEvent {
        id: String::new(),
        pubkey,
        created_at: now_unix,
        kind: NIP98_KIND,
        tags,
        content: String::new(),
        sig: String::new(),
    };
    let id = event_id(&event);
    let message = Message::from_digest(id);
    let signature = SECP256K1.sign_schnorr_no_aux_rand(&message, &keypair);
    event.id = hex::encode(&id);
    event.sig = hex::encode(signature.as_ref());
    Ok(format!(
        "{AUTH_SCHEME}{}",
        BASE64.encode(serde_json::to_vec(&event).expect("event serializes"))
    ))
}

/// Verify a NIP-98 `Authorization` header and return the signer pubkey hex.
pub fn verify_auth_header(
    header: &str,
    expected_url: &str,
    expected_method: &str,
    body: Option<&[u8]>,
    now_unix: u64,
) -> Result<String, Error> {
    assert!(!expected_url.is_empty() && !expected_method.is_empty());
    if header.len() > MAX_AUTH_HEADER_BYTES {
        return Err(Error::InvalidHeader("header too large"));
    }
    let encoded = header
        .strip_prefix(AUTH_SCHEME)
        .ok_or(Error::InvalidHeader("missing Nostr scheme"))?;
    let raw = BASE64
        .decode(encoded)
        .map_err(|_| Error::InvalidHeader("invalid base64"))?;
    let event: NostrEvent =
        serde_json::from_slice(&raw).map_err(|_| Error::InvalidHeader("invalid event json"))?;

    if event.kind != NIP98_KIND {
        return Err(Error::Rejected("wrong event kind"));
    }
    let oldest = now_unix.saturating_sub(MAX_SKEW_SECONDS);
    let newest = now_unix.saturating_add(MAX_SKEW_SECONDS);
    if event.created_at < oldest || event.created_at > newest {
        return Err(Error::Rejected("event timestamp outside window"));
    }
    if tag_value(&event, "u") != Some(expected_url) {
        return Err(Error::Rejected("url mismatch"));
    }
    if tag_value(&event, "method") != Some(expected_method) {
        return Err(Error::Rejected("method mismatch"));
    }
    match (body, tag_value(&event, "payload")) {
        (Some(body_bytes), Some(claimed)) => {
            if hex::encode(&Sha256::digest(body_bytes)) != claimed {
                return Err(Error::Rejected("payload hash mismatch"));
            }
        }
        (Some(body_bytes), None) => {
            if !body_bytes.is_empty() {
                return Err(Error::Rejected("missing payload tag"));
            }
        }
        (None, Some(_)) => return Err(Error::Rejected("unexpected payload tag")),
        (None, None) => {}
    }

    let claimed_id = hex::decode32(&event.id).ok_or(Error::InvalidHeader("invalid id"))?;
    if claimed_id != event_id(&event) {
        return Err(Error::Rejected("event id mismatch"));
    }
    let public_key = XOnlyPublicKey::from_slice(
        &hex::decode32(&event.pubkey).ok_or(Error::InvalidHeader("invalid pubkey"))?,
    )
    .map_err(|_| Error::InvalidHeader("invalid pubkey"))?;
    let signature = Signature::from_slice(
        &hex::decode64(&event.sig).ok_or(Error::InvalidHeader("invalid signature"))?,
    )
    .map_err(|_| Error::InvalidHeader("invalid signature"))?;
    let message = Message::from_digest(claimed_id);
    SECP256K1
        .verify_schnorr(&signature, &message, &public_key)
        .map_err(|_| Error::Rejected("signature verification failed"))?;
    Ok(event.pubkey)
}

fn tag_value<'a>(event: &'a NostrEvent, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        if tag.first().is_some_and(|value| value == name) {
            tag.get(1).map(String::as_str)
        } else {
            None
        }
    })
}

fn event_id(event: &NostrEvent) -> [u8; 32] {
    let preimage = serde_json::json!([
        0,
        event.pubkey,
        event.created_at,
        event.kind,
        event.tags,
        event.content
    ]);
    Sha256::digest(serde_json::to_vec(&preimage).expect("event id preimage serializes")).into()
}
