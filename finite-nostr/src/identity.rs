use std::fmt;

use nostr::PublicKey;
use nostr::nips::nip19::ToBech32;

use crate::NostrPrimitiveError;

/// Parsed Nostr public key with stable hex and npub formatting helpers.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct NostrPublicKey(PublicKey);

impl NostrPublicKey {
    /// Parse a public key from hex, npub/nprofile, or nostr: URI forms.
    pub fn parse(value: &str) -> Result<Self, NostrPrimitiveError> {
        PublicKey::parse(value)
            .map(Self)
            .map_err(|_| NostrPrimitiveError::MalformedInput {
                field: "public_key",
            })
    }

    /// Parse a public key from lowercase or uppercase hex.
    pub fn from_hex(value: &str) -> Result<Self, NostrPrimitiveError> {
        PublicKey::from_hex(value)
            .map(Self)
            .map_err(|_| NostrPrimitiveError::MalformedInput {
                field: "public_key_hex",
            })
    }

    /// Wrap a protocol crate public key.
    pub fn from_protocol(public_key: PublicKey) -> Self {
        Self(public_key)
    }

    /// Return the wrapped protocol crate public key.
    pub fn as_protocol(&self) -> PublicKey {
        self.0
    }

    /// Format this public key as lowercase hex.
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }

    /// Format this public key as NIP-19 npub.
    pub fn to_npub(&self) -> Result<String, NostrPrimitiveError> {
        self.0
            .to_bech32()
            .map_err(|_| NostrPrimitiveError::MalformedInput {
                field: "public_key",
            })
    }
}

impl fmt::Display for NostrPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}
