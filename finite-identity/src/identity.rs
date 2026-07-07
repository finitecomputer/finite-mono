use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use secp256k1::global::SECP256K1;
use secp256k1::{Keypair, Message, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::{Error, IdentityPaths, npub, nsec};

/// The identity file format version this build reads and writes.
pub const FORMAT_VERSION: u64 = 1;

/// The only key-material `kind` understood in contract v1.
pub const KIND_NOSTR_SECP256K1: &str = "nostr-secp256k1";

/// On non-Unix platforms, setting this environment variable to `1` allows
/// storing the identity without Unix file permissions.
pub const ALLOW_INSECURE_ENV: &str = "FINITE_IDENTITY_ALLOW_INSECURE";

/// On-disk shape of `identity.json`. Unknown fields land in `extra` and are
/// preserved verbatim on rewrite.
#[derive(Serialize, Deserialize)]
struct Wire {
    version: u64,
    kind: String,
    secret_hex: String,
    public_key_hex: String,
    created_at: String,
    created_by: String,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// The user's Nostr identity, loaded into memory once.
///
/// The secp256k1 keypair is cached in the struct: signing never touches the
/// filesystem. Construct via [`FiniteIdentity::load`] (never mints) or
/// [`FiniteIdentity::load_or_generate`] (mints under an exclusive lock if no
/// identity exists yet).
pub struct FiniteIdentity {
    keypair: Keypair,
    public_key_hex: String,
    created_at: String,
    created_by: String,
    /// Unknown JSON fields from the file, preserved for forward
    /// compatibility when the file is rewritten.
    extra: Map<String, Value>,
}

impl std::fmt::Debug for FiniteIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FiniteIdentity")
            .field("public_key_hex", &self.public_key_hex)
            .field("created_at", &self.created_at)
            .field("created_by", &self.created_by)
            .finish_non_exhaustive()
    }
}

/// A secret key handed to [`FiniteIdentity::import`]: 32 raw bytes, or a
/// user-supplied string parsed via [`ImportSecret::parse`].
///
/// Deliberately opaque: there is no accessor for the bytes and the [`Debug`]
/// impl is redacted, so an `ImportSecret` cannot leak through logging.
pub struct ImportSecret([u8; 32]);

impl ImportSecret {
    /// Parse a user-supplied secret string.
    ///
    /// Accepts NIP-19 `nsec1...` (bech32, HRP `nsec`) or 64 hex chars
    /// (lowercase or uppercase). Leading/trailing whitespace is trimmed.
    /// Error messages never echo the input; whether the bytes form a valid
    /// secp256k1 secret key is checked by [`FiniteIdentity::import`].
    pub fn parse(s: &str) -> Result<Self, Error> {
        let s = s.trim();
        let bytes = if let Some(bytes) = hex::decode32(s) {
            bytes
        } else {
            nsec::decode(s).map_err(|reason| Error::InvalidSecret {
                reason: format!("expected an nsec1... string or 64 hex chars ({reason})"),
            })?
        };
        Ok(Self(bytes))
    }
}

impl From<[u8; 32]> for ImportSecret {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl std::fmt::Debug for ImportSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ImportSecret(<redacted>)")
    }
}

impl FiniteIdentity {
    /// Load an existing identity. Never mints: if no identity file exists,
    /// returns [`Error::NotFound`].
    pub fn load(paths: &IdentityPaths) -> Result<Self, Error> {
        let path = paths.identity_file();
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(Error::NotFound { path });
            }
            Err(error) => return Err(io_error(&path, error)),
        };
        Self::from_json(&raw, &path)
    }

    /// Load the identity, minting a fresh key if none exists.
    ///
    /// Follows the contract's concurrency rules: takes an exclusive advisory
    /// lock on `<identity-root>/.lock` around the check-and-mint, writes to a
    /// temp file in the same directory, fsyncs, and renames into place. Two
    /// tools racing on a fresh machine converge on one key.
    ///
    /// `created_by` names the minting tool (e.g. `"finitechat/0.4.2"`) and is
    /// recorded in the file if this call mints.
    pub fn load_or_generate(paths: &IdentityPaths, created_by: &str) -> Result<Self, Error> {
        ensure_secure_platform()?;
        create_identity_root(paths.root())?;

        let lock_path = paths.lock_file();
        let lock = open_lock_file(&lock_path)?;
        fs4::fs_std::FileExt::lock_exclusive(&lock).map_err(|error| io_error(&lock_path, error))?;
        let result = match Self::load(paths) {
            Ok(identity) => Ok(identity),
            Err(Error::NotFound { .. }) => Self::generate(paths, created_by),
            Err(error) => Err(error),
        };
        let _ = fs4::fs_std::FileExt::unlock(&lock);
        result
    }

    /// Adopt an existing secret as the Finite identity and atomically write
    /// it, following the same locking/write rules as
    /// [`load_or_generate`](Self::load_or_generate).
    ///
    /// Refuses to overwrite: if an identity file already exists (even one
    /// this build cannot parse), returns [`Error::AlreadyExists`] and leaves
    /// the file untouched. Racing `import` against `load_or_generate` has
    /// exactly one winner under the shared lock: if generation wins, `import`
    /// fails with `AlreadyExists`; if `import` wins, `load_or_generate`
    /// adopts the imported key.
    ///
    /// The public key is derived from the secret and stored; `created_by`
    /// names the importing tool and `created_at` records the import time.
    /// Returns [`Error::InvalidSecret`] if the bytes are not a valid
    /// secp256k1 secret key.
    pub fn import(
        paths: &IdentityPaths,
        secret: ImportSecret,
        created_by: &str,
    ) -> Result<Self, Error> {
        ensure_secure_platform()?;
        create_identity_root(paths.root())?;

        let lock_path = paths.lock_file();
        let lock = open_lock_file(&lock_path)?;
        fs4::fs_std::FileExt::lock_exclusive(&lock).map_err(|error| io_error(&lock_path, error))?;
        let result = Self::import_locked(paths, secret, created_by);
        let _ = fs4::fs_std::FileExt::unlock(&lock);
        result
    }

    /// Check-and-write half of [`import`](Self::import). Caller must hold
    /// the exclusive lock.
    fn import_locked(
        paths: &IdentityPaths,
        secret: ImportSecret,
        created_by: &str,
    ) -> Result<Self, Error> {
        let dest = paths.identity_file();
        match fs::symlink_metadata(&dest) {
            Ok(_) => return Err(Error::AlreadyExists { path: dest }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&dest, error)),
        }

        let secret_key = SecretKey::from_slice(&secret.0).map_err(|_| Error::InvalidSecret {
            reason: "not a valid secp256k1 secret key".to_owned(),
        })?;
        let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
        let public_key_hex = hex::encode(&keypair.x_only_public_key().0.serialize());
        let created_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .expect("UTC datetime always formats as RFC3339");

        let identity = Self {
            keypair,
            public_key_hex,
            created_at,
            created_by: created_by.to_owned(),
            extra: Map::new(),
        };
        identity.write_atomic(paths)?;
        Ok(identity)
    }

    /// Mint a fresh identity and atomically write it. Caller must hold the
    /// exclusive lock.
    fn generate(paths: &IdentityPaths, created_by: &str) -> Result<Self, Error> {
        let keypair = Keypair::new(SECP256K1, &mut secp256k1::rand::thread_rng());
        let public_key_hex = hex::encode(&keypair.x_only_public_key().0.serialize());
        let created_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .expect("UTC datetime always formats as RFC3339");

        let identity = Self {
            keypair,
            public_key_hex,
            created_at,
            created_by: created_by.to_owned(),
            extra: Map::new(),
        };
        identity.write_atomic(paths)?;
        Ok(identity)
    }

    /// Parse and validate the file contents, failing closed on anything this
    /// build does not fully understand.
    fn from_json(raw: &str, path: &Path) -> Result<Self, Error> {
        let malformed = |reason: String| Error::Malformed {
            path: path.to_path_buf(),
            reason,
        };

        let value: Value =
            serde_json::from_str(raw).map_err(|error| malformed(error.to_string()))?;
        let object = value
            .as_object()
            .ok_or_else(|| malformed("not a JSON object".to_owned()))?;

        // Gate on version and kind before interpreting anything else, so a
        // v2 file with a different shape is UnsupportedVersion, not a parse
        // error.
        let version = object
            .get("version")
            .and_then(Value::as_u64)
            .ok_or_else(|| malformed("missing or non-integer \"version\"".to_owned()))?;
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion {
                found: version,
                supported: FORMAT_VERSION,
                path: path.to_path_buf(),
            });
        }
        let kind = object
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("missing or non-string \"kind\"".to_owned()))?;
        if kind != KIND_NOSTR_SECP256K1 {
            return Err(Error::UnsupportedKind {
                found: kind.to_owned(),
                supported: KIND_NOSTR_SECP256K1,
                path: path.to_path_buf(),
            });
        }

        let wire: Wire =
            serde_json::from_value(value).map_err(|error| malformed(error.to_string()))?;

        let secret_bytes = hex::decode32(&wire.secret_hex)
            .ok_or_else(|| malformed("secret_hex is not 64 hex chars".to_owned()))?;
        let secret_key = SecretKey::from_slice(&secret_bytes)
            .map_err(|_| malformed("secret_hex is not a valid secp256k1 secret key".to_owned()))?;
        let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);

        // The stored public key is derived data; verify it against the
        // secret and fail closed on mismatch (corrupt or tampered file).
        let derived_hex = hex::encode(&keypair.x_only_public_key().0.serialize());
        if !wire.public_key_hex.eq_ignore_ascii_case(&derived_hex) {
            return Err(Error::PublicKeyMismatch {
                path: path.to_path_buf(),
            });
        }

        Ok(Self {
            keypair,
            public_key_hex: derived_hex,
            created_at: wire.created_at,
            created_by: wire.created_by,
            extra: wire.extra,
        })
    }

    fn to_wire(&self) -> Wire {
        Wire {
            version: FORMAT_VERSION,
            kind: KIND_NOSTR_SECP256K1.to_owned(),
            secret_hex: hex::encode(&self.keypair.secret_bytes()),
            public_key_hex: self.public_key_hex.clone(),
            created_at: self.created_at.clone(),
            created_by: self.created_by.clone(),
            extra: self.extra.clone(),
        }
    }

    /// Serialize and atomically replace the identity file: temp file in the
    /// same directory (0600), write, fsync, rename into place, fsync the
    /// directory. Unknown fields captured at load time are written back out.
    fn write_atomic(&self, paths: &IdentityPaths) -> Result<(), Error> {
        let mut json = serde_json::to_string_pretty(&self.to_wire())
            .expect("identity wire format always serializes");
        json.push('\n');

        let dest = paths.identity_file();
        let temp = paths
            .root()
            .join(format!(".identity.json.tmp.{}", std::process::id()));

        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp).map_err(|e| io_error(&temp, e))?;
            file.write_all(json.as_bytes())
                .map_err(|e| io_error(&temp, e))?;
            file.sync_all().map_err(|e| io_error(&temp, e))?;
            drop(file);
            fs::rename(&temp, &dest).map_err(|e| io_error(&dest, e))?;
            #[cfg(unix)]
            {
                // Persist the rename itself.
                File::open(paths.root())
                    .and_then(|dir| dir.sync_all())
                    .map_err(|e| io_error(paths.root(), e))?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    /// The x-only public key as 64 lowercase hex chars.
    pub fn public_key_hex(&self) -> &str {
        &self.public_key_hex
    }

    /// The public key in NIP-19 `npub1...` display form.
    pub fn npub(&self) -> String {
        npub::encode(&self.keypair.x_only_public_key().0.serialize())
    }

    /// Sign a 32-byte message digest with BIP340 Schnorr.
    ///
    /// Deterministic (no auxiliary randomness): signing the same digest
    /// always yields the same 64-byte signature. Uses the in-memory keypair;
    /// never touches the filesystem.
    pub fn sign_schnorr(&self, digest: &[u8; 32]) -> [u8; 64] {
        let message = Message::from_digest(*digest);
        let signature = SECP256K1.sign_schnorr_no_aux_rand(&message, &self.keypair);
        *signature.as_ref()
    }

    /// The raw 32-byte secret key.
    ///
    /// This is sensitive material. It exists for callers that must derive
    /// from the account key at runtime (e.g. finitechat's HKDF
    /// domain-separated derivation). Per the contract, tools MUST NOT copy
    /// the secret into their own config stores.
    pub fn expose_secret_bytes(&self) -> [u8; 32] {
        self.keypair.secret_bytes()
    }

    /// RFC3339 timestamp recorded when the identity was minted.
    pub fn created_at(&self) -> &str {
        &self.created_at
    }

    /// Tool name/version that minted the identity.
    pub fn created_by(&self) -> &str {
        &self.created_by
    }
}

/// Create the identity root (and parents, e.g. `~/.finite`) with mode 0700.
fn create_identity_root(root: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)
            .map_err(|error| io_error(root, error))
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(root).map_err(|error| io_error(root, error))
    }
}

fn open_lock_file(path: &Path) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|error| io_error(path, error))
}

/// On non-Unix platforms the identity cannot be permission-protected;
/// creation fails closed unless `FINITE_IDENTITY_ALLOW_INSECURE=1`.
fn ensure_secure_platform() -> Result<(), Error> {
    #[cfg(unix)]
    {
        Ok(())
    }
    #[cfg(not(unix))]
    {
        if std::env::var_os(ALLOW_INSECURE_ENV).is_some_and(|value| value == "1") {
            Ok(())
        } else {
            Err(Error::InsecurePlatform)
        }
    }
}

fn io_error(path: &Path, source: io::Error) -> Error {
    Error::Io {
        path: PathBuf::from(path),
        source,
    }
}

/// Minimal lowercase-hex helpers for 32-byte keys; not worth a dependency.
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(char::from_digit((byte >> 4) as u32, 16).expect("nibble < 16"));
            out.push(char::from_digit((byte & 0x0f) as u32, 16).expect("nibble < 16"));
        }
        out
    }

    pub fn decode32(hex: &str) -> Option<[u8; 32]> {
        if hex.len() != 64 || !hex.is_ascii() {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A file with fields this build does not know about must round-trip them.
    #[test]
    fn unknown_fields_are_preserved_on_rewrite() {
        let secret_hex = "0000000000000000000000000000000000000000000000000000000000000003";
        let secret = SecretKey::from_slice(&hex::decode32(secret_hex).unwrap()).unwrap();
        let keypair = Keypair::from_secret_key(SECP256K1, &secret);
        let public_key_hex = hex::encode(&keypair.x_only_public_key().0.serialize());

        let raw = format!(
            r#"{{
                "version": 1,
                "kind": "nostr-secp256k1",
                "secret_hex": "{secret_hex}",
                "public_key_hex": "{public_key_hex}",
                "created_at": "2026-07-04T00:00:00Z",
                "created_by": "test/0.0.0",
                "future_field": {{"nested": [1, 2, 3]}},
                "another": "kept"
            }}"#
        );
        let identity = FiniteIdentity::from_json(&raw, Path::new("test.json")).unwrap();

        let rewritten = serde_json::to_value(identity.to_wire()).unwrap();
        assert_eq!(
            rewritten["future_field"],
            serde_json::json!({"nested": [1, 2, 3]})
        );
        assert_eq!(rewritten["another"], "kept");
        assert_eq!(rewritten["secret_hex"], secret_hex);
        assert_eq!(rewritten["version"], 1);
    }

    // Neither FiniteIdentity nor ImportSecret may leak key material through
    // their Debug impls.
    #[test]
    fn debug_impls_are_redacted() {
        let secret_hex = "0000000000000000000000000000000000000000000000000000000000000003";
        let secret = ImportSecret::parse(secret_hex).unwrap();
        assert_eq!(format!("{secret:?}"), "ImportSecret(<redacted>)");

        let key = SecretKey::from_slice(&hex::decode32(secret_hex).unwrap()).unwrap();
        let identity = FiniteIdentity {
            keypair: Keypair::from_secret_key(SECP256K1, &key),
            public_key_hex: "irrelevant".to_owned(),
            created_at: "2026-07-04T00:00:00Z".to_owned(),
            created_by: "test/0.0.0".to_owned(),
            extra: Map::new(),
        };
        let debug = format!("{identity:?}");
        assert!(!debug.contains(secret_hex), "Debug must not leak secret");
        let nsec = crate::nsec::encode(&key.secret_bytes());
        assert!(!debug.contains(&nsec), "Debug must not leak secret");
    }

    #[test]
    fn hex_round_trips() {
        let bytes: [u8; 32] = std::array::from_fn(|i| (i * 7 + 1) as u8);
        let encoded = hex::encode(&bytes);
        assert_eq!(encoded.len(), 64);
        assert_eq!(hex::decode32(&encoded).unwrap(), bytes);
        assert!(hex::decode32("zz").is_none());
        assert!(hex::decode32(&"g".repeat(64)).is_none());
    }
}
