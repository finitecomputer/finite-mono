//! The `finitechat auth` subcommand family: the CLI surface over the shared
//! Finite identity (Finite Identity Contract v1).
//!
//! The account key lives at `$FINITE_HOME/identity/identity.json` (or
//! `~/.finite/identity/identity.json`) and is shared by every Finite tool.
//! `auth status` reports the identity; `auth import` writes an existing nsec
//! into the shared location (the only storage location — finitechat keeps no
//! copy of the secret in its own stores).

use std::io::{Read, Write};

use finite_identity::{FiniteIdentity, IdentityPaths, ImportSecret};
use serde::Serialize;

use crate::{CliError, reject_extra_args, take_option, take_positional, write_pretty_json};

/// `created_by` recorded in identities written by `auth import`.
const IMPORT_CREATED_BY: &str = concat!("finitechat ", env!("CARGO_PKG_VERSION"), " (auth import)");

#[derive(Debug, Serialize, PartialEq, Eq)]
struct AuthStatusSummary {
    identity_file: String,
    account_id: String,
    npub: String,
    created_at: String,
    created_by: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct AuthImportSummary {
    identity_file: String,
    account_id: String,
    npub: String,
    imported: bool,
}

pub(crate) fn run<W: Write>(mut args: Vec<String>, output: &mut W) -> Result<(), CliError> {
    let Some(command) = take_positional(&mut args) else {
        return Err(CliError::Usage(usage()));
    };
    match command.as_str() {
        "status" => {
            reject_extra_args(&args)?;
            cmd_status(output)
        }
        "import" => {
            let file = take_option(&mut args, "--file")?;
            reject_extra_args(&args)?;
            cmd_import(file.as_deref(), &mut std::io::stdin(), output)
        }
        _ => Err(CliError::Usage(usage())),
    }
}

fn cmd_status<W: Write>(output: &mut W) -> Result<(), CliError> {
    let paths = identity_paths()?;
    let identity = FiniteIdentity::load(&paths).map_err(|error| {
        CliError::Identity(format!(
            "no usable Finite identity at {}: {error} (run any Finite tool to mint one, or `finitechat auth import`)",
            paths.identity_file().display()
        ))
    })?;
    write_pretty_json(
        output,
        &AuthStatusSummary {
            identity_file: paths.identity_file().display().to_string(),
            account_id: identity.public_key_hex().to_owned(),
            npub: identity.npub(),
            created_at: identity.created_at().to_owned(),
            created_by: identity.created_by().to_owned(),
        },
    )
}

fn cmd_import<W: Write>(
    file: Option<&str>,
    stdin: &mut impl Read,
    output: &mut W,
) -> Result<(), CliError> {
    // The secret is read from stdin or a file, never from an argv value
    // (argv leaks through process listings and shell history).
    let raw = match file {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|error| CliError::Identity(format!("failed to read {path}: {error}")))?,
        None => {
            let mut buffer = String::new();
            stdin
                .read_to_string(&mut buffer)
                .map_err(|error| CliError::Identity(format!("failed to read stdin: {error}")))?;
            buffer
        }
    };
    let secret = parse_secret_input(&raw)?;

    let paths = identity_paths()?;
    // The crate owns the contract-v1 write: exclusive create under the shared
    // lock (refuses to overwrite an existing identity), 0600/0700 permissions,
    // atomic rename, secp256k1 validation.
    let identity = FiniteIdentity::import(&paths, secret, IMPORT_CREATED_BY)
        .map_err(|error| CliError::Identity(error.to_string()))?;
    write_pretty_json(
        output,
        &AuthImportSummary {
            identity_file: paths.identity_file().display().to_string(),
            account_id: identity.public_key_hex().to_owned(),
            npub: identity.npub(),
            imported: true,
        },
    )
}

pub(crate) fn identity_paths() -> Result<IdentityPaths, CliError> {
    IdentityPaths::resolve().map_err(|error| CliError::Identity(error.to_string()))
}

fn parse_secret_input(raw: &str) -> Result<ImportSecret, CliError> {
    // `ImportSecret::parse` accepts nsec1... or 64 hex chars; the `nostr:`
    // URI prefix is a CLI-surface convenience, stripped here.
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_prefix("nostr:").unwrap_or(trimmed);
    ImportSecret::parse(trimmed).map_err(|error| CliError::Identity(error.to_string()))
}

pub(crate) fn usage() -> String {
    "auth commands:\n  finitechat auth status\n  finitechat auth import [--file PATH]   (reads an nsec or 64-hex secret from PATH or stdin)\n  (identity location: $FINITE_HOME/identity/identity.json, else ~/.finite/identity/identity.json)".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
    use finitechat_proto::{npub_encode, nsec_encode};

    fn secret_hex() -> String {
        "17".repeat(NOSTR_SECRET_KEY_BYTES)
    }

    /// The public key finitechat's own crypto derives for [`secret_hex`];
    /// the imported identity must agree with it.
    fn expected_public_key_hex() -> String {
        let secret = NostrSecretKey::from_bytes([0x17; NOSTR_SECRET_KEY_BYTES]).unwrap();
        secret
            .public_key()
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Import `raw` into a fresh FINITE_HOME and return the resulting
    /// identity's public key hex.
    fn imported_public_key(raw: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let paths = IdentityPaths::with_finite_home(dir.path());
        let secret = parse_secret_input(raw).expect("secret parses");
        FiniteIdentity::import(&paths, secret, IMPORT_CREATED_BY)
            .expect("import writes")
            .public_key_hex()
            .to_owned()
    }

    #[test]
    fn import_writes_contract_v1_identity_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let paths = IdentityPaths::with_finite_home(dir.path());

        let secret = parse_secret_input(&secret_hex()).unwrap();
        FiniteIdentity::import(&paths, secret, IMPORT_CREATED_BY).expect("import writes");
        let identity = FiniteIdentity::load(&paths).expect("crate loads imported identity");
        assert_eq!(identity.public_key_hex(), expected_public_key_hex());
        assert_eq!(
            identity.npub(),
            npub_encode(identity.public_key_hex()).unwrap()
        );
        assert!(identity.created_by().contains("auth import"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(paths.identity_file())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn import_refuses_to_overwrite_existing_identity() {
        let dir = tempfile::tempdir().unwrap();
        let paths = IdentityPaths::with_finite_home(dir.path());
        let existing = FiniteIdentity::load_or_generate(&paths, "test/0.0.0").unwrap();

        let secret = parse_secret_input(&secret_hex()).unwrap();
        let error = FiniteIdentity::import(&paths, secret, IMPORT_CREATED_BY)
            .expect_err("refuses overwrite");
        assert!(error.to_string().contains("refusing to overwrite"));

        let reloaded = FiniteIdentity::load(&paths).unwrap();
        assert_eq!(reloaded.public_key_hex(), existing.public_key_hex());
    }

    #[test]
    fn secret_input_accepts_nsec_and_hex_forms() {
        let hex_form = secret_hex();
        let nsec_form = nsec_encode(&hex_form).unwrap();
        let expected = expected_public_key_hex();

        assert_eq!(imported_public_key(&format!("{hex_form}\n")), expected);
        assert_eq!(imported_public_key(&format!("  {nsec_form}  ")), expected);
        assert_eq!(imported_public_key(&format!("nostr:{nsec_form}")), expected);

        assert!(parse_secret_input("nsec1notvalid").is_err());
        assert!(parse_secret_input("abc123").is_err());
    }
}
