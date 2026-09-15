use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use finitesites_proto::{hex, ids};

fn success(output: Output) -> Vec<u8> {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn borg(client: &Path, repository: &Path, passphrase: &str, cwd: &Path, args: &[&str]) -> Output {
    // A test must never inherit the operator's repository, keys, or SSH settings.
    Command::new("borg")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", client)
        .env("BORG_BASE_DIR", client)
        .env("BORG_REPO", repository)
        .env("BORG_PASSPHRASE", passphrase)
        .env("BORG_UNKNOWN_UNENCRYPTED_REPO_ACCESS_IS_OK", "no")
        .stdin(Stdio::null())
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("Borg is required; enter the pinned Nix development shell")
}

fn restore(repository: &Path, point: &str, target: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_finitesitesd"))
        .args(["backup", "restore", "--repository"])
        .arg(repository)
        .args(["--point", point, "--target"])
        .arg(target)
        .output()
        .unwrap()
}

pub fn restore_through_borg(data: &Path, target: &Path) {
    let root = tempfile::tempdir().unwrap();
    let local = root.path().join("recovery-points");
    let output = Command::new(env!("CARGO_BIN_EXE_finitesitesd"))
        .args(["backup", "capture", "--data"])
        .arg(data)
        .arg("--repository")
        .arg(&local)
        .output()
        .unwrap();
    let receipt: finitesitesd::backup::Receipt = serde_json::from_slice(&success(output)).unwrap();
    let remote = root.path().join("borg-repository");
    let writer = root.path().join("writer");
    fs::create_dir(&writer).unwrap();
    let passphrase = hex::encode(&ids::random_32());
    success(borg(
        &writer,
        &remote,
        &passphrase,
        root.path(),
        &["init", "--encryption=repokey-blake2"],
    ));
    let archive = format!("::sites-{}", receipt.id);
    let point = format!("points/{}", receipt.id);
    let create = [
        "create",
        "--compression=auto,zstd",
        "--files-cache=disabled",
        &archive,
        "objects",
        &point,
    ];
    success(borg(&writer, &remote, &passphrase, &local, &create));
    let archive_info: serde_json::Value = serde_json::from_slice(&success(borg(
        &writer,
        &remote,
        &passphrase,
        &local,
        &["info", "--json", &archive],
    )))
    .unwrap();
    assert_eq!(archive_info["encryption"]["mode"], "repokey-blake2");
    assert_eq!(
        archive_info["archives"][0]["id"].as_str().unwrap().len(),
        64
    );
    let replay = borg(&writer, &remote, &passphrase, &local, &create);
    assert!(
        !replay.status.success(),
        "an archive must not be overwritten"
    );
    assert!(String::from_utf8_lossy(&replay.stderr).contains("already exists"));
    let escrow = root.path().join("exported-repokey");
    success(borg(
        &writer,
        &remote,
        &passphrase,
        root.path(),
        &["key", "export", "::", escrow.to_str().unwrap()],
    ));
    assert!(fs::metadata(&escrow).unwrap().len() > 0);

    // Recovery must not depend on the original checkpoint or Borg client cache.
    fs::remove_dir_all(&local).unwrap();
    fs::remove_dir_all(&writer).unwrap();
    let reader = root.path().join("reader");
    let extracted = root.path().join("extracted");
    fs::create_dir(&reader).unwrap();
    fs::create_dir(&extracted).unwrap();
    let wrong_passphrase = hex::encode(&ids::random_32());
    let denied = borg(
        &reader,
        &remote,
        &wrong_passphrase,
        &extracted,
        &["extract", &archive],
    );
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr)
            .to_ascii_lowercase()
            .contains("passphrase"),
        "{}",
        String::from_utf8_lossy(&denied.stderr)
    );
    assert_eq!(fs::read_dir(&extracted).unwrap().count(), 0);
    success(borg(
        &reader,
        &remote,
        &passphrase,
        &extracted,
        &["check", "--verify-data", &archive],
    ));
    success(borg(
        &reader,
        &remote,
        &passphrase,
        &extracted,
        &["extract", &archive],
    ));
    let manifest = extracted.join(&point);
    let bytes = fs::read(&manifest).unwrap();
    fs::write(&manifest, b"damaged after extraction").unwrap();
    assert!(!restore(&extracted, &receipt.id, target).status.success());
    assert!(
        !target.exists(),
        "a failed restore must leave the target absent"
    );
    fs::write(&manifest, bytes).unwrap();
    success(restore(&extracted, &receipt.id, target));
    assert!(!restore(&extracted, &receipt.id, target).status.success());
}
