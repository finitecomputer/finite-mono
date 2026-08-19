use nostr::SecretKey;
use nostr::nips::nip44::{self as nostr_nip44, Version};

use crate::{NostrPrimitiveError, NostrPublicKey};

/// Encrypt plaintext using NIP-44 v2 with caller-provided key material.
pub fn encrypt_nip44(
    sender_secret_key: &SecretKey,
    recipient: NostrPublicKey,
    plaintext: impl AsRef<[u8]>,
) -> Result<String, NostrPrimitiveError> {
    nostr_nip44::encrypt(
        sender_secret_key,
        &recipient.as_protocol(),
        plaintext,
        Version::default(),
    )
    .map_err(|_| NostrPrimitiveError::FailedEncrypt)
}

/// Decrypt a NIP-44 payload with caller-provided key material.
pub fn decrypt_nip44(
    recipient_secret_key: &SecretKey,
    sender: NostrPublicKey,
    payload: impl AsRef<[u8]>,
) -> Result<String, NostrPrimitiveError> {
    nostr_nip44::decrypt(recipient_secret_key, &sender.as_protocol(), payload)
        .map_err(|_| NostrPrimitiveError::FailedDecrypt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::key::SecretKey;

    /// Fixed cross-language vector: this payload was produced by
    /// nostr-tools (dashboard viewer) `nip44.encrypt` with conversation key
    /// ECDH(secret 0x01*32, pubkey of secret 0x03*32). The Rust side must
    /// decrypt it — this pins the agent-wrap → browser-unwrap boundary.
    #[test]
    fn decrypts_nostr_tools_payload() {
        let recipient = SecretKey::from_slice(&[
            0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
            0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
            0x03, 0x03, 0x03, 0x03,
        ])
        .unwrap();
        let sender_hex = "1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f";
        let sender = NostrPublicKey::from_hex(sender_hex).unwrap();
        let payload = "AjeHNURa/Mw+FPLclPg9QAD43z1ZyGFQedaGkDvVSg0jbQQrAHU3U2N+OIxjei0xIcC4QuUy8gh6vYyheUopu5pS7utJQqA+emO6jFgBQpYIT1eGDq8TYcBK7r2u2n2OjUfs";
        let plaintext = decrypt_nip44(&recipient, sender, payload).unwrap();
        assert_eq!(plaintext, "finite-viewer-interop-check");
    }
}
