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
