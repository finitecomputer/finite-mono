//! Local Agent Signer backed by the shared Finite identity.
//!
//! Key material comes from the Finite Identity Contract v1 file
//! (`$FINITE_HOME/identity/identity.json`, else
//! `~/.finite/identity/identity.json`) via the `finite-identity` crate.
//! Whichever Finite tool runs first mints the key; fbrain finds it.
//! Commands that need the key mint on use (`load_or_generate`); read-only
//! surfaces like `auth status`, `status`, and `doctor` never mint. The
//! secret is derived into memory per invocation and is never copied into
//! fbrain's own config store (the legacy `auth.json` is a hard cut).

use finite_identity::{FiniteIdentity, IdentityPaths, ImportSecret};
use finite_nostr::{
    HttpAuthEventRequest, NostrPublicKey, encode_http_auth_header, sign_http_auth_event,
};
use nostr::event::FinalizeEvent;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

use crate::{CliEnvironment, CliError, auth_nonce, unix_timestamp};

pub(crate) fn signed_http_auth_header(
    keys: &Keys,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
) -> Result<String, CliError> {
    let mut request =
        HttpAuthEventRequest::new(method, url, unix_timestamp()).with_nonce(auth_nonce());
    if let Some(body) = body {
        request = request.with_body(body.to_vec());
    }
    let event = sign_http_auth_event(keys, &request)
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    Ok(encode_http_auth_header(&event))
}

pub(crate) fn sign_event(
    keys: &Keys,
    kind: Kind,
    content: impl Into<String>,
    tags: Vec<Tag>,
    created_at: u64,
    _label: Option<&str>,
) -> Result<nostr::Event, CliError> {
    EventBuilder::new(kind, content)
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(keys)
        .map_err(|error| CliError::InvalidSigner(error.to_string()))
}

/// The Local Agent Signer for one CLI invocation, derived in memory from the
/// shared Finite identity. Never persisted by fbrain.
pub(crate) struct LocalSigner {
    pub(crate) npub: String,
    pub(crate) keys: Keys,
}

/// The `created_by` string recorded in `identity.json` when fbrain mints or
/// imports the shared identity.
fn identity_created_by() -> String {
    format!("fbrain/{}", env!("CARGO_PKG_VERSION"))
}

/// Resolve the shared identity location. There is deliberately no
/// fbrain-specific override: the contract makes the location a convention
/// (`FINITE_HOME` is the only environment override) so every Finite tool
/// finds the same key. `env.finite_home` is an embedder/test seam only.
pub(crate) fn identity_paths(env: &CliEnvironment) -> Result<IdentityPaths, CliError> {
    match &env.finite_home {
        Some(finite_home) => Ok(IdentityPaths::with_finite_home(finite_home)),
        None => IdentityPaths::resolve().map_err(|error| CliError::Identity(error.to_string())),
    }
}

/// Load the shared identity without minting. Returns `Ok(None)` when no
/// identity file exists yet: `auth status`, `status`, and `doctor` must
/// report instead of mint (finite-identity CLI-CONVENTIONS.md).
pub(crate) fn load_identity_optional(
    env: &CliEnvironment,
) -> Result<Option<FiniteIdentity>, CliError> {
    let paths = identity_paths(env)?;
    match FiniteIdentity::load(&paths) {
        Ok(identity) => Ok(Some(identity)),
        Err(finite_identity::Error::NotFound { .. }) => Ok(None),
        Err(error) => Err(CliError::Identity(error.to_string())),
    }
}

/// Load the shared identity, minting one (under the contract's exclusive
/// lock) if no Finite tool has minted it yet. Used by commands that need the
/// key: mint on use, not on status.
pub(crate) fn load_or_generate_identity(env: &CliEnvironment) -> Result<FiniteIdentity, CliError> {
    let paths = identity_paths(env)?;
    // Existence check is only for the first-run message; load_or_generate
    // itself is the race-safe check-and-mint.
    let existed = paths.identity_file().exists();
    let identity = FiniteIdentity::load_or_generate(&paths, &identity_created_by())
        .map_err(|error| CliError::Identity(error.to_string()))?;
    if !existed {
        eprintln!(
            "created new Finite identity at {}",
            paths.identity_file().display()
        );
    }
    Ok(identity)
}

/// Derive the Local Agent Signer from the shared identity, in memory only.
pub(crate) fn signer_for(identity: &FiniteIdentity) -> Result<LocalSigner, CliError> {
    let secret_key = nostr::SecretKey::from_slice(&identity.expose_secret_bytes())
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    let keys = Keys::new(secret_key);
    let npub = NostrPublicKey::from_protocol(keys.public_key())
        .to_npub()
        .map_err(|error| CliError::InvalidSigner(error.to_string()))?;
    // Paired invariant: the npub fbrain derives must match the identity the
    // contract crate verified against the stored file.
    assert!(npub == identity.npub());
    Ok(LocalSigner { npub, keys })
}

/// Load the Local Agent Signer, minting the shared identity on first use.
pub(crate) fn load_signer(env: &CliEnvironment) -> Result<LocalSigner, CliError> {
    let identity = load_or_generate_identity(env)?;
    signer_for(&identity)
}

pub(crate) fn signer_keys(env: &CliEnvironment) -> Result<Keys, CliError> {
    Ok(load_signer(env)?.keys)
}

/// Adopt a user-supplied secret string (`nsec1...` or 64-char hex) as the
/// shared Finite identity via the contract crate. Parsing, locking, atomic
/// write, permissions, and the refusal to overwrite an existing identity
/// (`Error::AlreadyExists`) are owned by `finite-identity`; fbrain MUST NOT
/// reimplement secret parsing or identity-file writing (CLI-CONVENTIONS.md).
pub(crate) fn import_identity(
    env: &CliEnvironment,
    secret_text: &str,
) -> Result<FiniteIdentity, CliError> {
    let secret =
        ImportSecret::parse(secret_text).map_err(|error| CliError::Identity(error.to_string()))?;
    let paths = identity_paths(env)?;
    FiniteIdentity::import(&paths, secret, &identity_created_by())
        .map_err(|error| CliError::Identity(error.to_string()))
}
