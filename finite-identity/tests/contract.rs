//! Integration tests for the Finite Identity Contract v1 behaviors that need
//! a real filesystem: mint/load round-trips, concurrency, fail-closed
//! parsing, and path resolution.

use std::fs;
use std::sync::Arc;

use finite_identity::{Error, FiniteIdentity, IdentityPaths, ImportSecret};

fn temp_paths() -> (tempfile::TempDir, IdentityPaths) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let paths = IdentityPaths::with_finite_home(dir.path());
    (dir, paths)
}

#[test]
fn generate_then_load_round_trips() {
    let (_dir, paths) = temp_paths();

    let minted = FiniteIdentity::load_or_generate(&paths, "test-tool/1.0.0").expect("mint");
    let loaded = FiniteIdentity::load(&paths).expect("load");

    assert_eq!(minted.public_key_hex(), loaded.public_key_hex());
    assert_eq!(minted.npub(), loaded.npub());
    assert_eq!(minted.expose_secret_bytes(), loaded.expose_secret_bytes());
    assert_eq!(loaded.created_by(), "test-tool/1.0.0");
    assert_eq!(minted.created_at(), loaded.created_at());

    // Second load_or_generate finds the existing key, never re-mints.
    let again = FiniteIdentity::load_or_generate(&paths, "other-tool/2.0.0").expect("reload");
    assert_eq!(again.public_key_hex(), minted.public_key_hex());
    assert_eq!(again.created_by(), "test-tool/1.0.0");
}

#[test]
fn load_never_mints() {
    let (_dir, paths) = temp_paths();

    match FiniteIdentity::load(&paths) {
        Err(Error::NotFound { path }) => assert_eq!(path, paths.identity_file()),
        other => panic!("expected NotFound, got {other:?}"),
    }
    // load must not have created anything on disk.
    assert!(!paths.root().exists());
}

#[test]
fn concurrent_load_or_generate_converges_on_one_key() {
    let (_dir, paths) = temp_paths();
    let paths = Arc::new(paths);

    let handles: Vec<_> = (0..16)
        .map(|i| {
            let paths = Arc::clone(&paths);
            std::thread::spawn(move || {
                let identity =
                    FiniteIdentity::load_or_generate(&paths, &format!("racer/{i}")).expect("mint");
                identity.public_key_hex().to_owned()
            })
        })
        .collect();

    let keys: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let first = &keys[0];
    assert!(
        keys.iter().all(|k| k == first),
        "all racers must converge on one key, got {keys:?}"
    );

    // Exactly one identity file, no leftover temp files.
    let leftovers: Vec<_> = fs::read_dir(paths.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "identity.json" && name != ".lock")
        .collect();
    assert!(leftovers.is_empty(), "unexpected files: {leftovers:?}");
}

#[test]
fn import_hex_and_nsec_inputs_produce_the_same_identity() {
    let mut secret_bytes = [0u8; 32];
    secret_bytes[31] = 3;
    let nsec = finite_identity::nsec::encode(&secret_bytes);

    let inputs = [
        SECRET_HEX.to_owned(),
        SECRET_HEX.to_uppercase(),
        nsec,
        format!("  {SECRET_HEX}\n"), // stdin input keeps its trailing newline
    ];
    let mut npubs = Vec::new();
    for input in &inputs {
        let (_dir, paths) = temp_paths();
        let secret = ImportSecret::parse(input).expect("parse");
        let imported = FiniteIdentity::import(&paths, secret, "test-tool/1.0.0").expect("import");
        assert_eq!(imported.public_key_hex(), PUBLIC_HEX);
        assert_eq!(imported.expose_secret_bytes(), secret_bytes);
        assert_eq!(imported.created_by(), "test-tool/1.0.0");
        npubs.push(imported.npub());
    }
    assert!(npubs.iter().all(|n| n == &npubs[0]));

    // Raw bytes are accepted directly via From<[u8; 32]>.
    let (_dir, paths) = temp_paths();
    let imported =
        FiniteIdentity::import(&paths, secret_bytes.into(), "test-tool/1.0.0").expect("import");
    assert_eq!(imported.npub(), npubs[0]);
}

#[test]
fn import_then_load_round_trips() {
    let (_dir, paths) = temp_paths();
    let secret = ImportSecret::parse(SECRET_HEX).unwrap();
    let imported = FiniteIdentity::import(&paths, secret, "importer/1.0.0").expect("import");

    let loaded = FiniteIdentity::load(&paths).expect("load");
    assert_eq!(loaded.public_key_hex(), imported.public_key_hex());
    assert_eq!(loaded.created_by(), "importer/1.0.0");
    assert_eq!(loaded.created_at(), imported.created_at());

    // load_or_generate adopts the imported identity, never re-mints.
    let adopted = FiniteIdentity::load_or_generate(&paths, "other/2.0.0").expect("adopt");
    assert_eq!(adopted.public_key_hex(), imported.public_key_hex());
    assert_eq!(adopted.created_by(), "importer/1.0.0");
}

#[test]
fn import_refuses_to_overwrite_and_leaves_file_untouched() {
    let (_dir, paths) = temp_paths();
    let minted = FiniteIdentity::load_or_generate(&paths, "first/1.0.0").expect("mint");
    let before = fs::read_to_string(paths.identity_file()).unwrap();

    let secret = ImportSecret::parse(SECRET_HEX).unwrap();
    match FiniteIdentity::import(&paths, secret, "importer/1.0.0") {
        Err(Error::AlreadyExists { path }) => assert_eq!(path, paths.identity_file()),
        other => panic!("expected AlreadyExists, got {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(paths.identity_file()).unwrap(),
        before,
        "existing identity file must be left untouched"
    );
    let loaded = FiniteIdentity::load(&paths).unwrap();
    assert_eq!(loaded.public_key_hex(), minted.public_key_hex());
}

#[test]
fn import_secret_parse_rejects_bad_input() {
    // Wrong bech32 hrp: an npub (public key) is never a secret.
    let npub = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";
    assert!(matches!(
        ImportSecret::parse(npub),
        Err(Error::InvalidSecret { .. })
    ));
    // Wrong hex length.
    assert!(matches!(
        ImportSecret::parse(&SECRET_HEX[..63]),
        Err(Error::InvalidSecret { .. })
    ));
    assert!(matches!(
        ImportSecret::parse(&format!("{SECRET_HEX}00")),
        Err(Error::InvalidSecret { .. })
    ));
    // Mixed garbage, and garbage that never echoes back in the error.
    for garbage in [
        "",
        "hello world",
        "nsec1zzzz",
        "0xdeadbeef",
        "g".repeat(64).as_str(),
    ] {
        match ImportSecret::parse(garbage) {
            Err(Error::InvalidSecret { reason }) => {
                if !garbage.is_empty() {
                    assert!(
                        !reason.contains(garbage),
                        "error must not echo the input: {reason}"
                    );
                }
            }
            other => panic!("expected InvalidSecret for {garbage:?}, got {other:?}"),
        }
    }
}

#[test]
fn import_rejects_invalid_secp256k1_scalar() {
    // All-zero bytes parse as hex but are not a valid secret key; the check
    // happens at import time and nothing is written.
    let (_dir, paths) = temp_paths();
    let zeros = ImportSecret::parse(&"0".repeat(64)).expect("parse accepts 64 hex chars");
    match FiniteIdentity::import(&paths, zeros, "test/0.0.0") {
        Err(Error::InvalidSecret { .. }) => {}
        other => panic!("expected InvalidSecret, got {other:?}"),
    }
    assert!(!paths.identity_file().exists());
}

// Racing import against load_or_generate must have exactly one winner under
// the shared lock: if generation wins, import fails with AlreadyExists; if
// import wins, load_or_generate adopts the imported key.
#[test]
fn concurrent_import_and_load_or_generate_have_one_winner() {
    let (_dir, paths) = temp_paths();
    let paths = Arc::new(paths);

    let importers: Vec<_> = (0..8)
        .map(|i| {
            let paths = Arc::clone(&paths);
            std::thread::spawn(move || {
                let secret = ImportSecret::parse(SECRET_HEX).unwrap();
                FiniteIdentity::import(&paths, secret, &format!("importer/{i}"))
                    .map(|id| id.public_key_hex().to_owned())
            })
        })
        .collect();
    let generators: Vec<_> = (0..8)
        .map(|i| {
            let paths = Arc::clone(&paths);
            std::thread::spawn(move || {
                FiniteIdentity::load_or_generate(&paths, &format!("racer/{i}"))
                    .expect("load_or_generate never fails in this race")
                    .public_key_hex()
                    .to_owned()
            })
        })
        .collect();

    let import_results: Vec<_> = importers.into_iter().map(|h| h.join().unwrap()).collect();
    let generated_keys: Vec<_> = generators.into_iter().map(|h| h.join().unwrap()).collect();

    let final_key = FiniteIdentity::load(&paths)
        .unwrap()
        .public_key_hex()
        .to_owned();
    // Every load_or_generate converged on the single on-disk key.
    assert!(generated_keys.iter().all(|k| k == &final_key));

    let mut winners = 0;
    for result in &import_results {
        match result {
            Ok(key) => {
                winners += 1;
                assert_eq!(key, &final_key, "a winning import defines the key");
                assert_eq!(key, PUBLIC_HEX);
            }
            Err(Error::AlreadyExists { .. }) => {}
            Err(other) => panic!("expected Ok or AlreadyExists, got {other:?}"),
        }
    }
    assert!(winners <= 1, "at most one import can win");
    if final_key == PUBLIC_HEX {
        assert_eq!(winners, 1, "if the imported key is on disk, an import won");
    }

    // Exactly one identity file, no leftover temp files.
    let leftovers: Vec<_> = fs::read_dir(paths.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "identity.json" && name != ".lock")
        .collect();
    assert!(leftovers.is_empty(), "unexpected files: {leftovers:?}");
}

#[cfg(unix)]
#[test]
fn imported_file_permissions_are_restrictive() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, paths) = temp_paths();
    let secret = ImportSecret::parse(SECRET_HEX).unwrap();
    FiniteIdentity::import(&paths, secret, "test/0.0.0").unwrap();

    let dir_mode = fs::metadata(paths.root()).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "identity root must be 0700");
    let file_mode = fs::metadata(paths.identity_file())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600, "imported identity.json must be 0600");
}

fn write_identity_file(paths: &IdentityPaths, contents: &str) {
    fs::create_dir_all(paths.root()).unwrap();
    fs::write(paths.identity_file(), contents).unwrap();
}

// Secret 0x...03 and its x-only public key, for handcrafted files.
const SECRET_HEX: &str = "0000000000000000000000000000000000000000000000000000000000000003";
const PUBLIC_HEX: &str = "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";

fn identity_json(version: u64, kind: &str, public_key_hex: &str) -> String {
    format!(
        r#"{{
  "version": {version},
  "kind": "{kind}",
  "secret_hex": "{SECRET_HEX}",
  "public_key_hex": "{public_key_hex}",
  "created_at": "2026-07-04T00:00:00Z",
  "created_by": "test/0.0.0"
}}"#
    )
}

#[test]
fn unknown_version_is_rejected_and_never_re_minted() {
    let (_dir, paths) = temp_paths();
    write_identity_file(&paths, &identity_json(2, "nostr-secp256k1", PUBLIC_HEX));

    match FiniteIdentity::load(&paths) {
        Err(Error::UnsupportedVersion { found: 2, .. }) => {}
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
    // load_or_generate must also fail closed, not silently re-mint.
    match FiniteIdentity::load_or_generate(&paths, "test/0.0.0") {
        Err(Error::UnsupportedVersion { found: 2, .. }) => {}
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
    let raw = fs::read_to_string(paths.identity_file()).unwrap();
    assert!(raw.contains(SECRET_HEX), "file must be left untouched");
}

#[test]
fn unknown_kind_is_rejected() {
    let (_dir, paths) = temp_paths();
    write_identity_file(&paths, &identity_json(1, "frostr-share", PUBLIC_HEX));

    match FiniteIdentity::load(&paths) {
        Err(Error::UnsupportedKind { found, .. }) => assert_eq!(found, "frostr-share"),
        other => panic!("expected UnsupportedKind, got {other:?}"),
    }
    match FiniteIdentity::load_or_generate(&paths, "test/0.0.0") {
        Err(Error::UnsupportedKind { .. }) => {}
        other => panic!("expected UnsupportedKind, got {other:?}"),
    }
}

#[test]
fn public_key_mismatch_is_rejected() {
    let (_dir, paths) = temp_paths();
    let wrong_public = "0000000000000000000000000000000000000000000000000000000000000001";
    write_identity_file(&paths, &identity_json(1, "nostr-secp256k1", wrong_public));

    match FiniteIdentity::load(&paths) {
        Err(Error::PublicKeyMismatch { .. }) => {}
        other => panic!("expected PublicKeyMismatch, got {other:?}"),
    }
    // Never a silent re-mint over a corrupt file.
    match FiniteIdentity::load_or_generate(&paths, "test/0.0.0") {
        Err(Error::PublicKeyMismatch { .. }) => {}
        other => panic!("expected PublicKeyMismatch, got {other:?}"),
    }
    let raw = fs::read_to_string(paths.identity_file()).unwrap();
    assert!(raw.contains(wrong_public), "file must be left untouched");
}

#[test]
fn file_with_unknown_fields_loads() {
    let (_dir, paths) = temp_paths();
    let raw = format!(
        r#"{{
  "version": 1,
  "kind": "nostr-secp256k1",
  "secret_hex": "{SECRET_HEX}",
  "public_key_hex": "{PUBLIC_HEX}",
  "created_at": "2026-07-04T00:00:00Z",
  "created_by": "test/0.0.0",
  "future_field": {{"nested": true}}
}}"#
    );
    write_identity_file(&paths, &raw);

    let identity = FiniteIdentity::load(&paths).expect("unknown fields are ignored on read");
    assert_eq!(identity.public_key_hex(), PUBLIC_HEX);
}

#[test]
fn npub_matches_known_nip19_vector() {
    // Vector from finitesites-proto / finitechat-proto (fiatjaf's pubkey).
    let hex = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
    let npub = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";

    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
    }
    assert_eq!(finite_identity::npub::encode(&bytes), npub);
    assert_eq!(finite_identity::npub::decode(npub).unwrap(), bytes);

    // And a minted identity's npub decodes back to its own hex pubkey.
    let (_dir, paths) = temp_paths();
    let identity = FiniteIdentity::load_or_generate(&paths, "test/0.0.0").unwrap();
    let decoded = finite_identity::npub::decode(&identity.npub()).unwrap();
    let mut expected = [0u8; 32];
    for (i, byte) in expected.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&identity.public_key_hex()[2 * i..2 * i + 2], 16).unwrap();
    }
    assert_eq!(decoded, expected);
}

#[test]
fn sign_schnorr_verifies_and_is_deterministic() {
    use secp256k1::{Message, XOnlyPublicKey, schnorr::Signature};

    let (_dir, paths) = temp_paths();
    let identity = FiniteIdentity::load_or_generate(&paths, "test/0.0.0").unwrap();

    let digest = [0x42u8; 32];
    let sig_bytes = identity.sign_schnorr(&digest);

    // Deterministic: no aux rand, same digest -> same signature.
    assert_eq!(identity.sign_schnorr(&digest), sig_bytes);
    // Reloading from disk signs identically.
    let reloaded = FiniteIdentity::load(&paths).unwrap();
    assert_eq!(reloaded.sign_schnorr(&digest), sig_bytes);

    // Verifies under secp256k1 against the x-only pubkey from the file.
    let mut pk_bytes = [0u8; 32];
    for (i, byte) in pk_bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&identity.public_key_hex()[2 * i..2 * i + 2], 16).unwrap();
    }
    let pubkey = XOnlyPublicKey::from_slice(&pk_bytes).unwrap();
    let signature = Signature::from_slice(&sig_bytes).unwrap();
    secp256k1::global::SECP256K1
        .verify_schnorr(&signature, &Message::from_digest(digest), &pubkey)
        .expect("signature must verify");
}

#[cfg(unix)]
#[test]
fn unix_permissions_are_restrictive() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, paths) = temp_paths();
    FiniteIdentity::load_or_generate(&paths, "test/0.0.0").unwrap();

    let dir_mode = fs::metadata(paths.root()).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "identity root must be 0700");
    let file_mode = fs::metadata(paths.identity_file())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600, "identity.json must be 0600");
}

#[test]
fn finite_home_env_overrides_resolution() {
    // Set and unset FINITE_HOME in one test so nothing races on the process
    // environment. Rust test binaries run tests in threads; this is the only
    // test in this binary that touches the environment.
    unsafe { std::env::set_var("FINITE_HOME", "/data/agent") };
    let overridden = IdentityPaths::resolve().unwrap();
    assert_eq!(
        overridden.root(),
        std::path::Path::new("/data/agent/identity")
    );
    assert_eq!(
        overridden.identity_file(),
        std::path::Path::new("/data/agent/identity/identity.json")
    );

    unsafe { std::env::remove_var("FINITE_HOME") };
    let default = IdentityPaths::resolve().unwrap();
    let home = dirs::home_dir().unwrap();
    assert_eq!(default.root(), home.join(".finite").join("identity"));

    // Explicit-root constructor is exactly FINITE_HOME=dir.
    assert_eq!(IdentityPaths::with_finite_home("/data/agent"), overridden);
}
