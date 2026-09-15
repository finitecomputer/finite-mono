//! Raw x-only public key derivation for Sites key files.
//!
//! NIP-98 event encoding, signing, and verification live in the canonical
//! `finite-nostr` implementation (see `crate::nip98`); this module only
//! derives the pubkey a 32-byte secret signs with, which the CLI and tests
//! use when displaying and cross-checking key identities.

use secp256k1::{Keypair, Secp256k1};

use crate::{ProtoError, hex};

/// Derive the x-only public key hex for a secret key.
pub fn pubkey_for_secret(secret_key: &[u8; 32]) -> Result<String, ProtoError> {
    let secp = Secp256k1::new();
    let keypair = Keypair::from_seckey_slice(&secp, secret_key)
        .map_err(|_| ProtoError::InvalidEvent("invalid secret key"))?;
    let (xonly, _) = keypair.x_only_public_key();
    Ok(hex::encode(&xonly.serialize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use finite_nostr::{
        HttpAuthEventRequest, decode_http_auth_header, sign_http_auth_header_with_secret,
    };

    fn test_secret() -> [u8; 32] {
        let mut secret = [0u8; 32];
        secret[31] = 1;
        secret
    }

    #[test]
    fn pubkey_for_secret_matches_canonical_signed_events() {
        // The derivation must agree with the pubkey the canonical
        // implementation signs with for the same secret.
        let request = HttpAuthEventRequest::new("GET", "https://finite.test/", 1_700_000_000);
        let header = sign_http_auth_header_with_secret(&test_secret(), &request).unwrap();
        let event = decode_http_auth_header(&header).unwrap();
        assert_eq!(
            pubkey_for_secret(&test_secret()).unwrap(),
            event.pubkey.to_hex()
        );
    }

    #[test]
    fn rejects_invalid_secret_key() {
        // A secret that is not a valid secp256k1 scalar fails closed.
        let mut secret = [0xffu8; 32];
        secret[0] = 0xff;
        assert_eq!(
            pubkey_for_secret(&secret),
            Err(ProtoError::InvalidEvent("invalid secret key"))
        );
    }
}
