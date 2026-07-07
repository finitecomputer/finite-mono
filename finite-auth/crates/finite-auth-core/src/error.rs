use std::error::Error;
use std::fmt;

use finite_nostr::NostrPrimitiveError;

/// Typed error returned by finite-auth core validation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AuthError {
    /// A reusable Nostr primitive rejected protocol input.
    Nostr(NostrPrimitiveError),
    /// Input failed a Finite auth boundary check.
    InvalidInput {
        /// Field that failed validation.
        field: &'static str,
        /// Stable failure reason.
        reason: String,
    },
    /// Bounded input exceeded an explicit limit.
    LimitExceeded {
        /// Field or collection that exceeded the limit.
        field: &'static str,
        /// Configured upper bound.
        limit: usize,
        /// Observed size.
        actual: usize,
    },
    /// A signed auth event did not include a required nonce.
    MissingNonce,
    /// A signed auth event included a nonce that did not match the challenge.
    NonceMismatch {
        /// Expected challenge nonce.
        expected: String,
        /// Actual event nonce, if present.
        actual: Option<String>,
    },
    /// NIP-05 well-known document was not valid JSON for this scaffold.
    MalformedNip05Document {
        /// Parser or shape failure.
        reason: String,
    },
    /// NIP-05 document did not contain the requested local name.
    Nip05NameMissing {
        /// Full identifier that was requested.
        identifier: String,
    },
    /// NIP-05 document mapped the identifier to a different public key.
    Nip05PublicKeyMismatch {
        /// Full identifier that was requested.
        identifier: String,
        /// Public key required by the caller.
        expected: String,
        /// Public key found in the NIP-05 document.
        actual: String,
    },
    /// NIP-05 public keys must be lowercase hex.
    Nip05KeyNotLowercaseHex {
        /// Rejected key material.
        value: String,
    },
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nostr(error) => write!(f, "{error}"),
            Self::InvalidInput { field, reason } => {
                write!(f, "invalid auth input for {field}: {reason}")
            }
            Self::LimitExceeded {
                field,
                limit,
                actual,
            } => write!(
                f,
                "auth input exceeded limit for {field}: limit {limit}, actual {actual}"
            ),
            Self::MissingNonce => f.write_str("missing auth challenge nonce"),
            Self::NonceMismatch { expected, actual } => write!(
                f,
                "auth challenge nonce mismatch: expected {expected}, got {actual:?}"
            ),
            Self::MalformedNip05Document { reason } => {
                write!(f, "malformed NIP-05 document: {reason}")
            }
            Self::Nip05NameMissing { identifier } => {
                write!(f, "NIP-05 name missing for {identifier}")
            }
            Self::Nip05PublicKeyMismatch {
                identifier,
                expected,
                actual,
            } => write!(
                f,
                "NIP-05 public key mismatch for {identifier}: expected {expected}, got {actual}"
            ),
            Self::Nip05KeyNotLowercaseHex { value } => {
                write!(f, "NIP-05 public key is not lowercase hex: {value}")
            }
        }
    }
}

impl Error for AuthError {}

impl From<NostrPrimitiveError> for AuthError {
    fn from(value: NostrPrimitiveError) -> Self {
        Self::Nostr(value)
    }
}
