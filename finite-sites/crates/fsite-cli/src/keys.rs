//! Key material handling for the CLI.
//!
//! Two kinds of keys:
//! - the User Key: the shared Finite identity per the Finite Identity
//!   Contract v1, at `$FINITE_HOME/identity/identity.json` when `FINITE_HOME`
//!   is set and `~/.finite/identity/identity.json` otherwise. Whichever
//!   Finite tool runs first mints it; `fsite` finds it. The secret is derived
//!   into memory per invocation and is never copied into fsite's own config
//!   store. The legacy `~/.config/finite-sites/identity.env` location is a
//!   hard cut: it is not read (see `fsite auth import`).
//! - one email key per verified External Principal email, still tool-specific
//!   under `~/.config/finite-sites/emails/`.
//!
//! Email key files are `KEY=hex` env-style, created with 0600 permissions,
//! and must never be committed or included in deploy artifacts.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use finite_identity::{FiniteIdentity, IdentityPaths, ImportSecret};
use finitesites_proto::{event, hex, ids};

use crate::CliError;

/// Env-file key name for the legacy pre-contract identity file, understood
/// only by `fsite auth import --file`.
const LEGACY_IDENTITY_KEY_NAME: &str = "FINITE_SITES_USER_SECRET";
const EMAIL_KEY_NAME: &str = "FINITE_SITES_EMAIL_SECRET";
const PENDING_EMAIL_LINK_PUBKEY_NAME: &str = "FINITE_SITES_LINK_PUBKEY";

pub struct KeyFile {
    pub secret: [u8; 32],
    pub pubkey: String,
}

fn parse_env_file(content: &str, wanted_key: &str) -> Option<String> {
    // Bounded: key files are a handful of lines.
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix(wanted_key)
            && let Some(value) = value.strip_prefix('=')
        {
            return Some(value.trim().to_string());
        }
    }
    None
}

fn load_key_file(path: &Path, key_name: &str) -> Result<KeyFile, CliError> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| CliError::Io(format!("cannot read {}: {error}", path.display())))?;
    let secret_hex = parse_env_file(&content, key_name)
        .ok_or_else(|| CliError::Key(format!("{} is missing {key_name}", path.display())))?;
    let secret = hex::decode32(&secret_hex)
        .map_err(|_| CliError::Key(format!("{} has a malformed secret", path.display())))?;
    let pubkey = event::pubkey_for_secret(&secret)
        .map_err(|_| CliError::Key(format!("{} secret is not a valid key", path.display())))?;
    Ok(KeyFile { secret, pubkey })
}

fn write_key_file(path: &Path, key_name: &str, secret: &[u8; 32]) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            CliError::Io(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    let content = format!("{key_name}={}\n", hex::encode(secret));
    std::fs::write(path, content)
        .map_err(|error| CliError::Io(format!("cannot write {}: {error}", path.display())))?;
    set_owner_only_permissions(path)?;
    // Paired check: read the key back before trusting it was stored.
    let reread = load_key_file(path, key_name)?;
    assert!(reread.secret == *secret);
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &Path) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| CliError::Io(format!("cannot chmod {}: {error}", path.display())))
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &Path) -> Result<(), CliError> {
    Ok(())
}

/// Resolve the shared Finite identity location from the environment
/// (`FINITE_HOME` else the platform home directory). There is deliberately
/// no fsite-specific override: the contract makes the location a convention
/// so every Finite tool finds the same key.
pub fn identity_paths() -> Result<IdentityPaths, CliError> {
    IdentityPaths::resolve().map_err(|error| CliError::Key(error.to_string()))
}

/// The `created_by` string recorded in `identity.json` when fsite mints.
fn identity_created_by() -> String {
    format!("fsite/{}", env!("CARGO_PKG_VERSION"))
}

/// Load the shared Finite user identity, minting one (under the contract's
/// exclusive lock) if no Finite tool has minted it yet.
/// Load the shared identity without minting. Returns Ok(None) when no
/// identity file exists yet ("auth status" must never mint per
/// CLI-CONVENTIONS.md).
pub fn load_identity(paths: &IdentityPaths) -> Result<Option<FiniteIdentity>, CliError> {
    match FiniteIdentity::load(paths) {
        Ok(identity) => Ok(Some(identity)),
        Err(finite_identity::Error::NotFound { .. }) => Ok(None),
        Err(error) => Err(CliError::Key(error.to_string())),
    }
}

pub fn load_or_generate_identity(paths: &IdentityPaths) -> Result<FiniteIdentity, CliError> {
    // Existence check is only for the first-run message; load_or_generate
    // itself is the race-safe check-and-mint.
    let existed = paths.identity_file().exists();
    let identity = FiniteIdentity::load_or_generate(paths, &identity_created_by())
        .map_err(|error| CliError::Key(error.to_string()))?;
    if !existed {
        eprintln!(
            "created new Finite identity at {}",
            paths.identity_file().display()
        );
    }
    Ok(identity)
}

/// Derive the CLI signing key from the shared identity, in memory only.
/// Per the contract, the secret is never copied into fsite's own config
/// store; it lives in this process for the duration of one command.
pub fn user_key_for(identity: &FiniteIdentity) -> Result<KeyFile, CliError> {
    let secret = identity.expose_secret_bytes();
    let derived_pubkey = event::pubkey_for_secret(&secret)
        .map_err(|_| CliError::Key("shared identity secret is not a valid key".to_string()))?;
    // Paired invariant: the pubkey fsite derives must match the pubkey the
    // identity crate verified against the stored file.
    assert!(derived_pubkey == identity.public_key_hex());
    Ok(KeyFile {
        secret,
        pubkey: derived_pubkey,
    })
}

/// Load the User Key for signing: the shared Finite identity, minted on
/// first use by whichever Finite tool runs first.
pub fn load_or_generate_user_key() -> Result<KeyFile, CliError> {
    let paths = identity_paths()?;
    let identity = load_or_generate_identity(&paths)?;
    user_key_for(&identity)
}

/// Adopt a user-supplied secret string (`nsec1...` or 64-char hex) as the
/// shared Finite identity via the contract crate. Locking, atomic write,
/// permissions, and the refusal to overwrite an existing identity
/// (`Error::AlreadyExists`) are owned by `finite-identity`; fsite MUST NOT
/// reimplement secret parsing or identity-file writing (CLI-CONVENTIONS.md).
pub fn import_identity(
    paths: &IdentityPaths,
    secret_text: &str,
) -> Result<FiniteIdentity, CliError> {
    let secret =
        ImportSecret::parse(secret_text).map_err(|error| CliError::Key(error.to_string()))?;
    let identity = FiniteIdentity::import(paths, secret, &identity_created_by())
        .map_err(|error| CliError::Key(error.to_string()))?;
    // Paired check: the contract loader must accept what import wrote and
    // derive the same key.
    let reloaded = FiniteIdentity::load(paths).map_err(|error| CliError::Key(error.to_string()))?;
    assert!(reloaded.public_key_hex() == identity.public_key_hex());
    Ok(identity)
}

/// Interpret the content of a file handed to `fsite auth import --file`.
///
/// If the file is a legacy pre-contract `identity.env`
/// (`FINITE_SITES_USER_SECRET=hex`), extract that value so the documented
/// one-liner keeps working. Otherwise the whole file is the secret string
/// itself (an `nsec1...` or hex secret); `ImportSecret::parse` trims and
/// validates it.
pub fn import_secret_text_from_file(content: &str) -> String {
    match parse_env_file(content, LEGACY_IDENTITY_KEY_NAME) {
        Some(legacy_secret_hex) => legacy_secret_hex,
        None => content.trim().to_string(),
    }
}

pub fn email_key_path(email: &str) -> Result<PathBuf, CliError> {
    let home = std::env::var("HOME")
        .map_err(|_| CliError::Key("HOME is not set; cannot store email key".to_string()))?;
    Ok(email_scoped_config_path(&home, "emails", email, ".env"))
}

pub fn load_or_create_email_key(email: &str) -> Result<KeyFile, CliError> {
    let path = email_key_path(email)?;
    if path.exists() {
        return load_key_file(&path, EMAIL_KEY_NAME);
    }
    let secret = ids::random_32();
    write_key_file(&path, EMAIL_KEY_NAME, &secret)?;
    eprintln!("created email key at {}", path.display());
    load_key_file(&path, EMAIL_KEY_NAME)
}

pub fn pending_email_link_path(email: &str) -> Result<PathBuf, CliError> {
    let home = std::env::var("HOME")
        .map_err(|_| CliError::Key("HOME is not set; cannot store email link".to_string()))?;
    Ok(email_scoped_config_path(
        &home,
        "email-links",
        email,
        ".env",
    ))
}

pub fn write_pending_email_link(email: &str, pubkey: &str) -> Result<(), CliError> {
    if !hex::is_hex32(pubkey) {
        return Err(CliError::Key(
            "pending email link pubkey is malformed".to_string(),
        ));
    }
    let path = pending_email_link_path(email)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            CliError::Io(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    std::fs::write(&path, pending_email_link_content(pubkey))
        .map_err(|error| CliError::Io(format!("cannot write {}: {error}", path.display())))?;
    set_owner_only_permissions(&path)?;
    Ok(())
}

pub fn pending_email_link_pubkey(email: &str) -> Result<Option<String>, CliError> {
    let path = pending_email_link_path(email)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| CliError::Io(format!("cannot read {}: {error}", path.display())))?;
    parse_pending_email_link(&content, &path).map(Some)
}

pub fn clear_pending_email_link(email: &str) -> Result<(), CliError> {
    let path = pending_email_link_path(email)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliError::Io(format!(
            "cannot remove {}: {error}",
            path.display()
        ))),
    }
}

fn email_scoped_config_path(home: &str, directory: &str, email: &str, suffix: &str) -> PathBuf {
    let digest = hex::encode(&Sha256::digest(
        email.trim().to_ascii_lowercase().as_bytes(),
    ));
    PathBuf::from(home)
        .join(".config/finite-sites")
        .join(directory)
        .join(format!("{}{}", &digest[..16], suffix))
}

fn pending_email_link_content(pubkey: &str) -> String {
    format!("{PENDING_EMAIL_LINK_PUBKEY_NAME}={pubkey}\n")
}

fn parse_pending_email_link(content: &str, path: &Path) -> Result<String, CliError> {
    let pubkey = parse_env_file(content, PENDING_EMAIL_LINK_PUBKEY_NAME)
        .ok_or_else(|| CliError::Key(format!("{} is missing pending pubkey", path.display())))?;
    if !hex::is_hex32(&pubkey) {
        return Err(CliError::Key(format!(
            "{} has a malformed pending pubkey",
            path.display()
        )));
    }
    Ok(pubkey)
}

#[cfg(test)]
mod tests {
    use super::*;

    use finitesites_proto::nip98;

    fn legacy_env_content(secret: &[u8; 32]) -> String {
        format!("{LEGACY_IDENTITY_KEY_NAME}={}\n", hex::encode(secret))
    }

    #[test]
    fn fresh_finite_home_mints_shared_identity() {
        let dir = tempfile::tempdir().unwrap();
        let paths = IdentityPaths::with_finite_home(dir.path());
        assert!(!paths.identity_file().exists());
        let identity = load_or_generate_identity(&paths).unwrap();
        assert!(paths.identity_file().exists());
        // The freshly minted identity signs for the pubkey it reports.
        let key = user_key_for(&identity).unwrap();
        assert_eq!(key.pubkey, identity.public_key_hex());
    }

    #[test]
    fn existing_shared_identity_is_found_and_keeps_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let paths = IdentityPaths::with_finite_home(dir.path());
        let first = load_or_generate_identity(&paths).unwrap();
        let second = load_or_generate_identity(&paths).unwrap();
        assert_eq!(first.public_key_hex(), second.public_key_hex());
        assert_eq!(first.npub(), second.npub());
    }

    #[test]
    fn nip98_round_trips_through_the_shared_identity_key() {
        let dir = tempfile::tempdir().unwrap();
        let paths = IdentityPaths::with_finite_home(dir.path());
        let identity = load_or_generate_identity(&paths).unwrap();
        let key = user_key_for(&identity).unwrap();

        let url = "http://127.0.0.1:8787/api/v1/projects/init";
        let now: u64 = 1_750_000_000;
        let body = br#"{"hello":"world"}"#;
        let header = nip98::build_auth_header(&key.secret, url, "POST", Some(body), now).unwrap();
        let verified = nip98::verify_auth_header(&header, url, "POST", Some(body), now).unwrap();
        assert_eq!(verified, identity.public_key_hex());

        // Tampered body must not verify (negative path).
        assert!(nip98::verify_auth_header(&header, url, "POST", Some(b"evil"), now).is_err());
    }

    #[test]
    fn import_adopts_a_hex_secret_as_the_shared_identity() {
        let dir = tempfile::tempdir().unwrap();
        let paths = IdentityPaths::with_finite_home(dir.path());
        let mut secret = [0u8; 32];
        secret[31] = 9;
        let expected_pubkey = event::pubkey_for_secret(&secret).unwrap();

        let identity = import_identity(&paths, &hex::encode(&secret)).unwrap();
        assert_eq!(identity.public_key_hex(), expected_pubkey);

        // The contract loader finds the imported key on the next load.
        let reloaded = load_or_generate_identity(&paths).unwrap();
        assert_eq!(reloaded.public_key_hex(), expected_pubkey);
    }

    #[test]
    fn import_refuses_to_overwrite_an_existing_shared_identity() {
        let dir = tempfile::tempdir().unwrap();
        let paths = IdentityPaths::with_finite_home(dir.path());
        let existing = load_or_generate_identity(&paths).unwrap();

        let mut secret = [0u8; 32];
        secret[31] = 9;
        let result = import_identity(&paths, &hex::encode(&secret));
        assert!(matches!(
            result,
            Err(CliError::Key(message)) if message.contains("already exists")
        ));

        // Replay must not have touched the existing identity.
        let reloaded = load_or_generate_identity(&paths).unwrap();
        assert_eq!(reloaded.public_key_hex(), existing.public_key_hex());
    }

    #[test]
    fn import_rejects_a_malformed_secret_without_writing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let paths = IdentityPaths::with_finite_home(dir.path());
        assert!(matches!(
            import_identity(&paths, "not-a-secret"),
            Err(CliError::Key(_))
        ));
        // Failed imports must not create an identity file.
        assert!(!paths.identity_file().exists());
    }

    #[test]
    fn file_content_yields_the_legacy_env_value_or_the_raw_secret() {
        let mut secret = [0u8; 32];
        secret[31] = 9;
        // Legacy identity.env format: the KEY= line wins.
        let legacy = legacy_env_content(&secret);
        assert_eq!(import_secret_text_from_file(&legacy), hex::encode(&secret));
        // Anything else is the secret string itself, trimmed.
        assert_eq!(
            import_secret_text_from_file("  nsec1notchecked\n"),
            "nsec1notchecked"
        );
        assert_eq!(
            import_secret_text_from_file(&format!("{}\n", hex::encode(&secret))),
            hex::encode(&secret)
        );
    }

    #[test]
    fn parse_env_file_finds_key() {
        let content = "# comment\nFINITE_SITES_TEST_SECRET=abc123\nOTHER=x\n";
        assert_eq!(
            parse_env_file(content, "FINITE_SITES_TEST_SECRET").as_deref(),
            Some("abc123")
        );
        assert_eq!(parse_env_file(content, "MISSING"), None);
        // A key that is a prefix of another must not match it.
        let tricky = "FINITE_SITES_TEST_SECRET_OLD=zzz\nFINITE_SITES_TEST_SECRET=good\n";
        assert_eq!(
            parse_env_file(tricky, "FINITE_SITES_TEST_SECRET").as_deref(),
            Some("good")
        );
    }

    #[test]
    fn pending_email_link_content_validates_pubkey() {
        let mut secret = [0u8; 32];
        secret[31] = 7;
        let pubkey = event::pubkey_for_secret(&secret).unwrap();
        let content = pending_email_link_content(&pubkey);
        assert_eq!(
            parse_pending_email_link(&content, Path::new("pending.env")).unwrap(),
            pubkey
        );
        assert!(parse_pending_email_link("", Path::new("pending.env")).is_err());
        assert!(
            parse_pending_email_link(
                "FINITE_SITES_LINK_PUBKEY=not-a-pubkey\n",
                Path::new("pending.env")
            )
            .is_err()
        );
    }

    #[test]
    fn email_scoped_paths_are_normalized() {
        let a = email_scoped_config_path("/tmp/home", "emails", "Skyler@Example.com", ".env");
        let b = email_scoped_config_path("/tmp/home", "emails", " skyler@example.com ", ".env");
        assert_eq!(a, b);
        assert!(a.starts_with("/tmp/home/.config/finite-sites/emails"));
        assert_eq!(
            a.file_name().unwrap().to_string_lossy().len(),
            "0123456789abcdef.env".len()
        );
    }
}
