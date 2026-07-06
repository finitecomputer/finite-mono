//! Integration tests for the shared Finite identity wiring: environment
//! resolution (FINITE_HOME else ~/.finite), the hard cut away from the
//! legacy ~/.config/finite-sites/identity.env location, and the
//! `fsite auth status` / `fsite auth import` command surface. Each test
//! drives the real binary with a controlled HOME/FINITE_HOME so tests never
//! touch the developer's key.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use finite_identity::nsec;
use finitesites_proto::{event, hex, npub};

fn fsite_command(home: &Path, finite_home: Option<&Path>, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fsite"));
    command.args(args);
    command.env("HOME", home);
    match finite_home {
        Some(dir) => {
            command.env("FINITE_HOME", dir);
        }
        None => {
            command.env_remove("FINITE_HOME");
        }
    }
    command
}

fn fsite(home: &Path, finite_home: Option<&Path>, args: &[&str]) -> Output {
    fsite_command(home, finite_home, args)
        .output()
        .expect("fsite runs")
}

/// Run fsite with the given line piped to stdin, the way an agent pipes a
/// secret into `fsite auth import`.
fn fsite_with_stdin(home: &Path, args: &[&str], stdin_line: &str) -> Output {
    let mut command = fsite_command(home, None, args);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("fsite spawns");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin_line.as_bytes())
        .expect("stdin accepts the secret");
    child.wait_with_output().expect("fsite runs")
}

fn stdout_json(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "fsite failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("fsite prints valid json")
}

fn npub_for_secret(secret: &[u8; 32]) -> String {
    let pubkey = event::pubkey_for_secret(secret).unwrap();
    npub::encode_npub(&pubkey).unwrap()
}

fn write_legacy_identity_env(home: &Path, secret: &[u8; 32]) {
    let config_dir = home.join(".config/finite-sites");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("identity.env"),
        format!("FINITE_SITES_USER_SECRET={}\n", hex::encode(secret)),
    )
    .unwrap();
}

fn test_secret(fill: u8) -> [u8; 32] {
    let mut secret = [0u8; 32];
    secret[0] = fill;
    secret[31] = 7;
    secret
}

#[test]
fn status_never_mints_and_finds_an_imported_identity() {
    let home = tempfile::tempdir().unwrap();
    let expected_file = home.path().join(".finite/identity/identity.json");

    // First run: status reports the missing identity and creates nothing.
    let first = stdout_json(&fsite(
        home.path(),
        None,
        &["auth", "status", "--output", "json"],
    ));
    assert!(!first["exists"].as_bool().unwrap());
    assert_eq!(
        first["file"].as_str().unwrap(),
        expected_file.to_str().unwrap()
    );
    assert!(!expected_file.exists());
    assert!(!home.path().join(".finite").exists());

    // Mint through the documented paths (here: import), then status finds it.
    let secret = test_secret(9);
    let imported = stdout_json(&fsite_with_stdin(
        home.path(),
        &["auth", "import", "--output", "json"],
        &hex::encode(&secret),
    ));
    let second = stdout_json(&fsite(
        home.path(),
        None,
        &["auth", "status", "--output", "json"],
    ));
    assert!(second["exists"].as_bool().unwrap());
    assert_eq!(second["npub"], imported["npub"]);
    assert!(expected_file.exists());
}

#[test]
fn identity_minted_by_another_finite_tool_is_found_with_the_same_npub() {
    let home = tempfile::tempdir().unwrap();
    let secret = test_secret(3);
    let pubkey = event::pubkey_for_secret(&secret).unwrap();
    let identity_dir = home.path().join(".finite/identity");
    std::fs::create_dir_all(&identity_dir).unwrap();
    std::fs::write(
        identity_dir.join("identity.json"),
        format!(
            "{{\n  \"version\": 1,\n  \"kind\": \"nostr-secp256k1\",\n  \"secret_hex\": \"{}\",\n  \"public_key_hex\": \"{pubkey}\",\n  \"created_at\": \"2026-07-04T00:00:00Z\",\n  \"created_by\": \"finitechat/0.4.2\"\n}}\n",
            hex::encode(&secret)
        ),
    )
    .unwrap();

    let shown = stdout_json(&fsite(
        home.path(),
        None,
        &["auth", "status", "--output", "json"],
    ));
    assert_eq!(shown["npub"].as_str().unwrap(), npub_for_secret(&secret));
    assert_eq!(shown["created_by"].as_str().unwrap(), "finitechat/0.4.2");
}

#[test]
fn finite_home_overrides_the_default_identity_location() {
    let home = tempfile::tempdir().unwrap();
    let finite_home = tempfile::tempdir().unwrap();
    let missing = stdout_json(&fsite(
        home.path(),
        Some(finite_home.path()),
        &["auth", "status", "--output", "json"],
    ));
    let expected_file = finite_home.path().join("identity/identity.json");
    assert!(!missing["exists"].as_bool().unwrap());
    assert_eq!(
        missing["file"].as_str().unwrap(),
        expected_file.to_str().unwrap()
    );
    let mut import = fsite_command(home.path(), Some(finite_home.path()), &["auth", "import"]);
    import.stdin(Stdio::piped());
    let mut child = import.spawn().unwrap();
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(hex::encode(&test_secret(11)).as_bytes())
        .unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    let shown = stdout_json(&fsite(
        home.path(),
        Some(finite_home.path()),
        &["auth", "status", "--output", "json"],
    ));
    assert_eq!(
        shown["file"].as_str().unwrap(),
        expected_file.to_str().unwrap()
    );
    assert!(expected_file.exists());
    // Nothing may leak into the default location when FINITE_HOME is set.
    assert!(!home.path().join(".finite").exists());
}

#[test]
fn legacy_identity_env_is_dead_and_a_new_identity_is_minted() {
    let home = tempfile::tempdir().unwrap();
    let legacy_secret = test_secret(5);
    write_legacy_identity_env(home.path(), &legacy_secret);

    // Status is read-only and does not see (or adopt) the legacy key.
    let missing = stdout_json(&fsite(
        home.path(),
        None,
        &["auth", "status", "--output", "json"],
    ));
    assert!(!missing["exists"].as_bool().unwrap());

    // The normal first-run mint (any identity-using command, here whoami)
    // ignores the planted legacy key and mints fresh.
    assert!(fsite(home.path(), None, &["whoami"]).status.success());
    let shown = stdout_json(&fsite(
        home.path(),
        None,
        &["auth", "status", "--output", "json"],
    ));
    assert_ne!(
        shown["npub"].as_str().unwrap(),
        npub_for_secret(&legacy_secret)
    );
    assert!(home.path().join(".finite/identity/identity.json").exists());
    // The legacy file itself is left untouched for manual migration.
    assert!(
        home.path()
            .join(".config/finite-sites/identity.env")
            .exists()
    );
}

#[test]
fn auth_import_reads_an_nsec_from_stdin() {
    let home = tempfile::tempdir().unwrap();
    let secret = test_secret(11);

    let output = fsite_with_stdin(
        home.path(),
        &["auth", "import", "--output", "json"],
        &format!("{}\n", nsec::encode(&secret)),
    );
    let imported = stdout_json(&output);
    assert_eq!(imported["npub"].as_str().unwrap(), npub_for_secret(&secret));
    // The secret must never be echoed back on any stream.
    let all_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!all_output.contains(&hex::encode(&secret)));
    assert!(!all_output.contains(&nsec::encode(&secret)));

    // The imported identity is what every later command sees.
    let shown = stdout_json(&fsite(
        home.path(),
        None,
        &["auth", "status", "--output", "json"],
    ));
    assert_eq!(shown["npub"], imported["npub"]);
}

#[test]
fn auth_import_reads_a_legacy_identity_env_via_file() {
    let home = tempfile::tempdir().unwrap();
    let legacy_secret = test_secret(9);
    write_legacy_identity_env(home.path(), &legacy_secret);
    let env_file = home.path().join(".config/finite-sites/identity.env");

    let imported = stdout_json(&fsite(
        home.path(),
        None,
        &[
            "auth",
            "import",
            "--file",
            env_file.to_str().unwrap(),
            "--output",
            "json",
        ],
    ));
    assert_eq!(
        imported["npub"].as_str().unwrap(),
        npub_for_secret(&legacy_secret)
    );
    assert_eq!(
        imported["imported_from"].as_str().unwrap(),
        env_file.to_str().unwrap()
    );

    // The imported identity is what every later command sees.
    let shown = stdout_json(&fsite(
        home.path(),
        None,
        &["auth", "status", "--output", "json"],
    ));
    assert_eq!(shown["npub"], imported["npub"]);
}

#[test]
fn auth_import_reads_a_bare_nsec_via_file() {
    let home = tempfile::tempdir().unwrap();
    let secret = test_secret(13);
    let nsec_file = home.path().join("old-key.txt");
    std::fs::write(&nsec_file, format!("{}\n", nsec::encode(&secret))).unwrap();

    let imported = stdout_json(&fsite(
        home.path(),
        None,
        &[
            "auth",
            "import",
            "--file",
            nsec_file.to_str().unwrap(),
            "--output",
            "json",
        ],
    ));
    assert_eq!(imported["npub"].as_str().unwrap(), npub_for_secret(&secret));
}

#[test]
fn auth_import_refuses_to_overwrite_an_existing_identity() {
    let home = tempfile::tempdir().unwrap();
    // The normal first-run path mints an identity.
    assert!(fsite(home.path(), None, &["whoami"]).status.success());
    let existing = stdout_json(&fsite(
        home.path(),
        None,
        &["auth", "status", "--output", "json"],
    ));

    let secret = test_secret(9);
    let replay = fsite_with_stdin(
        home.path(),
        &["auth", "import"],
        &format!("{}\n", hex::encode(&secret)),
    );
    assert!(!replay.status.success());
    assert!(String::from_utf8_lossy(&replay.stderr).contains("already exists"));

    // The refusal must not have touched the existing identity.
    let shown = stdout_json(&fsite(
        home.path(),
        None,
        &["auth", "status", "--output", "json"],
    ));
    assert_eq!(shown["npub"], existing["npub"]);
}

#[test]
fn whoami_reports_the_shared_identity_file() {
    let home = tempfile::tempdir().unwrap();
    let output = fsite(home.path(), None, &["whoami"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("npub:   npub1"));
    assert!(stdout.contains(".finite/identity/identity.json"));
    assert!(!stdout.contains("identity.env"));
}
