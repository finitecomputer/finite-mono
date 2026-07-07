use std::path::PathBuf;

/// Errors returned by this crate.
///
/// Everything fails closed: a file we do not fully understand (unknown
/// version or kind, malformed JSON, a public key that does not match the
/// secret) is an error, never a silent re-mint.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No identity file exists. `load` never mints; call
    /// [`FiniteIdentity::load_or_generate`](crate::FiniteIdentity::load_or_generate)
    /// if minting is acceptable.
    #[error("no identity file at {path}")]
    NotFound {
        /// The identity file path that was checked.
        path: PathBuf,
    },

    /// `FINITE_HOME` is unset and the platform home directory could not be
    /// resolved.
    #[error("could not resolve a home directory (set FINITE_HOME or HOME)")]
    NoHomeDir,

    /// The file declares a `version` this build does not understand.
    #[error(
        "unsupported identity file version {found} in {path} \
         (this build understands version {supported})"
    )]
    UnsupportedVersion {
        /// The version declared by the file.
        found: u64,
        /// The version this build understands.
        supported: u64,
        /// The identity file path.
        path: PathBuf,
    },

    /// The file declares a `kind` this build does not understand (e.g. a
    /// future `frostr-share`).
    #[error(
        "unsupported identity kind {found:?} in {path} \
         (this build understands {supported:?})"
    )]
    UnsupportedKind {
        /// The kind declared by the file.
        found: String,
        /// The kind this build understands.
        supported: &'static str,
        /// The identity file path.
        path: PathBuf,
    },

    /// The file is not valid JSON, or a required field is missing or has the
    /// wrong shape.
    #[error("identity file {path} is malformed: {reason}")]
    Malformed {
        /// The identity file path.
        path: PathBuf,
        /// What was wrong with it.
        reason: String,
    },

    /// The stored `public_key_hex` does not match the key derived from
    /// `secret_hex`. The file is corrupt or has been tampered with.
    #[error("public_key_hex in {path} does not match the key derived from secret_hex")]
    PublicKeyMismatch {
        /// The identity file path.
        path: PathBuf,
    },

    /// An identity file already exists.
    /// [`FiniteIdentity::import`](crate::FiniteIdentity::import) refuses to
    /// overwrite it; the existing file is left untouched.
    #[error("an identity already exists at {path}; refusing to overwrite it")]
    AlreadyExists {
        /// The existing identity file path.
        path: PathBuf,
    },

    /// A supplied secret could not be parsed or is not a valid secp256k1
    /// secret key. The message never echoes the input.
    #[error("invalid secret: {reason}")]
    InvalidSecret {
        /// Why the secret was rejected (never contains the input).
        reason: String,
    },

    /// On non-Unix platforms, secret storage cannot be permission-protected.
    /// Set `FINITE_IDENTITY_ALLOW_INSECURE=1` to proceed anyway.
    #[error(
        "refusing to store an identity without Unix file permissions; \
         set FINITE_IDENTITY_ALLOW_INSECURE=1 to override"
    )]
    InsecurePlatform,

    /// An underlying filesystem operation failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}
