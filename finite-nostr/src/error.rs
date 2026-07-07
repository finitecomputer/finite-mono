use std::error::Error;
use std::fmt;

/// Typed error returned by finite-nostr public helpers.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum NostrPrimitiveError {
    /// Input could not be parsed as a Nostr value.
    MalformedInput {
        /// Field or value that failed to parse.
        field: &'static str,
    },
    /// Input exceeded a bounded protocol limit.
    LimitExceeded {
        /// Field or value that exceeded the limit.
        field: &'static str,
        /// Maximum allowed size or count.
        limit: usize,
        /// Actual size or count.
        actual: usize,
    },
    /// Event ID is not the deterministic NIP-01 ID for the event body.
    InvalidEventId,
    /// Event signature does not verify against the event public key.
    SignatureFailure,
    /// Event signing failed before a signed event could be produced.
    SigningFailure,
    /// Event kind did not match the expected protocol kind.
    WrongEventKind {
        /// Required kind.
        expected: u16,
        /// Event kind that was supplied.
        actual: u16,
    },
    /// Event timestamp fell outside the allowed clock skew window.
    StaleTimestamp {
        /// Server/reference time in Unix seconds.
        now: u64,
        /// Event creation time in Unix seconds.
        created_at: u64,
        /// Allowed absolute skew in seconds.
        max_skew_seconds: u64,
    },
    /// Event URL tag did not match the request URL.
    UrlMismatch {
        /// Required absolute URL.
        expected: String,
        /// Event URL.
        actual: String,
    },
    /// Event method tag did not match the request method.
    MethodMismatch {
        /// Required uppercase method.
        expected: String,
        /// Event method.
        actual: String,
    },
    /// Event payload hash tag did not match the request body expectation.
    PayloadMismatch {
        /// Required lowercase SHA-256 hex hash, if a body was supplied.
        expected: Option<String>,
        /// Event payload hash tag, if present.
        actual: Option<String>,
    },
    /// Event signer did not match the expected public key.
    SignerMismatch {
        /// Expected public key hex.
        expected: String,
        /// Event public key hex.
        actual: String,
    },
    /// NIP-59 seal issuer did not match the expected sender.
    WrongIssuer {
        /// Expected public key hex.
        expected: String,
        /// Actual issuer public key hex.
        actual: String,
    },
    /// NIP-59 gift-wrap event did not include a recipient public key tag.
    MissingRecipient,
    /// NIP-59 gift-wrap recipient did not match the expected recipient.
    WrongRecipient {
        /// Expected public key hex.
        expected: String,
        /// Recipient public keys from the gift-wrap event.
        actual: Vec<String>,
    },
    /// NIP-44 decryption failed.
    FailedDecrypt,
    /// NIP-44 encryption failed.
    FailedEncrypt,
    /// Decrypted NIP-59 plaintext could not be parsed as the expected structure.
    MalformedPlaintext {
        /// Plaintext structure that failed to parse.
        field: &'static str,
    },
    /// Required protocol tag was missing.
    MissingTag {
        /// Missing tag name.
        tag: &'static str,
    },
    /// NIP-05 document did not contain the requested local name.
    Nip05NameMissing {
        /// Requested NIP-05 identifier.
        identifier: String,
    },
    /// NIP-05 document mapped the identifier to a different public key.
    Nip05PublicKeyMismatch {
        /// Requested NIP-05 identifier.
        identifier: String,
        /// Expected public key hex.
        expected: String,
        /// Public key found in the NIP-05 document.
        actual: String,
    },
    /// NIP-05 public keys must be lowercase hex.
    Nip05KeyNotLowercaseHex {
        /// Non-canonical public key value.
        value: String,
    },
}

impl fmt::Display for NostrPrimitiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedInput { field } => write!(f, "malformed Nostr input: {field}"),
            Self::LimitExceeded {
                field,
                limit,
                actual,
            } => write!(
                f,
                "Nostr input limit exceeded for {field}: limit {limit}, actual {actual}"
            ),
            Self::InvalidEventId => f.write_str("invalid Nostr event id"),
            Self::SignatureFailure => f.write_str("invalid Nostr event signature"),
            Self::SigningFailure => f.write_str("failed to sign Nostr event"),
            Self::WrongEventKind { expected, actual } => {
                write!(
                    f,
                    "wrong Nostr event kind: expected {expected}, got {actual}"
                )
            }
            Self::StaleTimestamp {
                now,
                created_at,
                max_skew_seconds,
            } => write!(
                f,
                "stale Nostr event timestamp: now {now}, created_at {created_at}, max skew {max_skew_seconds}s"
            ),
            Self::UrlMismatch { expected, actual } => {
                write!(
                    f,
                    "Nostr auth URL mismatch: expected {expected}, got {actual}"
                )
            }
            Self::MethodMismatch { expected, actual } => write!(
                f,
                "Nostr auth method mismatch: expected {expected}, got {actual}"
            ),
            Self::PayloadMismatch { expected, actual } => write!(
                f,
                "Nostr auth payload mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::SignerMismatch { expected, actual } => write!(
                f,
                "Nostr signer mismatch: expected {expected}, got {actual}"
            ),
            Self::WrongIssuer { expected, actual } => write!(
                f,
                "Nostr issuer mismatch: expected {expected}, got {actual}"
            ),
            Self::MissingRecipient => f.write_str("missing Nostr gift-wrap recipient"),
            Self::WrongRecipient { expected, actual } => write!(
                f,
                "Nostr gift-wrap recipient mismatch: expected {expected}, got {actual:?}"
            ),
            Self::FailedDecrypt => f.write_str("NIP-44 decrypt failed"),
            Self::FailedEncrypt => f.write_str("NIP-44 encrypt failed"),
            Self::MalformedPlaintext { field } => {
                write!(f, "malformed decrypted Nostr plaintext: {field}")
            }
            Self::MissingTag { tag } => write!(f, "missing Nostr auth tag: {tag}"),
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

impl Error for NostrPrimitiveError {}
