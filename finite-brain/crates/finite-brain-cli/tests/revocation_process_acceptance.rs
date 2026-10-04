use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use tempfile::TempDir;

#[path = "revocation/harness.rs"]
mod harness;
use harness::{
    LostResponseProxy, candidate_binary, create_fixture, execute, execute_at, home,
    reference_authority, run, server,
};

#[test]
fn built_cli_removal_preserves_survivor_sync_and_restores_on_empty_target() {
    let scratch = TempDir::new().unwrap();
    let candidate_binary = candidate_binary();
    let binary = candidate_binary.as_path();
    let reader_binary = std::env::var_os("FBRAIN_COMPAT_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| binary.to_owned());
    let owner = home(scratch.path(), "owner");
    let survivor = home(scratch.path(), "survivor");
    let target_home = home(scratch.path(), "target");
    let database = scratch.path().join("authority.sqlite3");
    let authority = server(&database);
    let target = run(
        binary,
        &target_home,
        &target_home,
        &authority.url,
        &["signer", "public-key"],
    )["npub"]
        .as_str()
        .unwrap()
        .to_owned();
    let survivor_key = run(
        binary,
        &survivor,
        &survivor,
        &authority.url,
        &["signer", "public-key"],
    )["npub"]
        .as_str()
        .unwrap()
        .to_owned();
    create_fixture(binary, &owner, &target, &authority.url);
    run(
        binary,
        &owner,
        &owner,
        &authority.url,
        &[
            "collaborator",
            "ensure-admin",
            "--brain",
            "revocation",
            "--target",
            &survivor_key,
        ],
    );
    let owner_tree = owner.join("tree");
    run(
        binary,
        &owner,
        &owner,
        &authority.url,
        &["open", "revocation", owner_tree.to_str().unwrap()],
    );
    for (id, name) in [("first", "First"), ("second", "Second")] {
        run(
            binary,
            &owner,
            &owner_tree,
            &authority.url,
            &[
                "folder",
                "create",
                id,
                "--name",
                name,
                "--path",
                name,
                "--access",
                "restricted",
            ],
        );
    }
    run(
        binary,
        &owner,
        &owner_tree,
        &authority.url,
        &["sync", "now"],
    );
    for name in ["First", "Second"] {
        fs::write(
            owner_tree.join(name).join("wiki/keep.md"),
            "# Existing knowledge\n",
        )
        .unwrap();
    }
    run(
        binary,
        &owner,
        &owner_tree,
        &authority.url,
        &["sync", "now"],
    );
    let reader_tree = survivor.join("tree");
    run(
        &reader_binary,
        &survivor,
        &survivor,
        &authority.url,
        &["open", "revocation", reader_tree.to_str().unwrap()],
    );
    run(
        &reader_binary,
        &survivor,
        &reader_tree,
        &authority.url,
        &["sync", "now"],
    );

    if std::env::var_os("FBRAIN_COMPAT_BINARY").is_some() {
        let before = run(
            binary,
            &owner,
            &owner_tree,
            &authority.url,
            &["brain", "export"],
        );
        let unsafe_demotion = execute(
            &reader_binary,
            &owner,
            &owner_tree,
            &authority.url,
            &["admin", "role", "revoke", "admin", "--target", &target],
        );
        assert!(!unsafe_demotion.status.success());
        assert_eq!(
            before,
            run(
                binary,
                &owner,
                &owner_tree,
                &authority.url,
                &["brain", "export"]
            )
        );
    }

    let target_before = run(
        binary,
        &target_home,
        &target_home,
        &authority.url,
        &["brain", "export", "--brain", "revocation"],
    );
    assert!(
        target_before["accessState"]["admins"]
            .as_array()
            .unwrap()
            .contains(&json!(target))
    );
    let proxy = LostResponseProxy::start(&authority.url);
    let removal = execute_at(
        binary,
        &owner,
        &owner_tree,
        &proxy.url,
        &authority.url,
        &["admin", "member", "remove", "--target", &target],
    );
    assert!(
        removal.status.success(),
        "{}",
        String::from_utf8_lossy(&removal.stderr)
    );
    assert!(
        proxy.dropped.load(Ordering::Acquire),
        "fixture must drop a committed removal response"
    );
    let removed: Value = serde_json::from_slice(&removal.stdout).unwrap();
    drop(proxy);
    assert_eq!(removed["state"], "complete");
    assert_eq!(removed["outcome"], "changed");
    assert_eq!(removed["folders"].as_array().unwrap().len(), 2);
    let after = run(
        binary,
        &owner,
        &owner_tree,
        &authority.url,
        &["brain", "export"],
    );
    assert!(
        !after["accessState"]["members"]
            .as_array()
            .unwrap()
            .contains(&json!(target))
    );
    assert!(
        !after["accessState"]["admins"]
            .as_array()
            .unwrap()
            .contains(&json!(target))
    );
    assert!(
        after["folders"]
            .as_array()
            .unwrap()
            .iter()
            .all(|folder| folder["currentKeyVersion"] == 2)
    );
    assert!(
        !after["keyGrants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|grant| grant["recipientNpub"] == target && grant["keyVersion"] == 2)
    );
    let cursor = run(
        binary,
        &owner,
        &owner_tree,
        &authority.url,
        &["sync", "now"],
    )["latestSequence"]
        .clone();
    let retry = run(
        binary,
        &owner,
        &owner_tree,
        &authority.url,
        &["admin", "member", "remove", "--target", &target],
    );
    assert_eq!(retry["state"], "complete");
    assert_eq!(retry["outcome"], "alreadyComplete");
    assert_eq!(
        after,
        run(
            binary,
            &owner,
            &owner_tree,
            &authority.url,
            &["brain", "export"]
        )
    );
    assert_eq!(
        cursor,
        run(
            binary,
            &owner,
            &owner_tree,
            &authority.url,
            &["sync", "now"]
        )["latestSequence"]
    );
    run(
        &reader_binary,
        &survivor,
        &reader_tree,
        &authority.url,
        &["sync", "now"],
    );
    assert_eq!(
        fs::read_to_string(reader_tree.join("First/wiki/keep.md")).unwrap(),
        "# Existing knowledge\n"
    );
    fs::write(reader_tree.join("First/wiki/keep.md"), "# Survivor edit\n").unwrap();
    run(
        &reader_binary,
        &survivor,
        &reader_tree,
        &authority.url,
        &["sync", "now"],
    );
    run(
        binary,
        &owner,
        &owner_tree,
        &authority.url,
        &["sync", "now"],
    );
    assert_eq!(
        fs::read_to_string(owner_tree.join("First/wiki/keep.md")).unwrap(),
        "# Survivor edit\n"
    );
    if std::env::var_os("FBRAIN_COMPAT_BINARY").is_some() {
        let ordinary_home = home(scratch.path(), "ordinary-member");
        let ordinary = run(
            binary,
            &ordinary_home,
            &ordinary_home,
            &authority.url,
            &["signer", "public-key"],
        )["npub"]
            .as_str()
            .unwrap()
            .to_owned();
        run(
            binary,
            &owner,
            &owner_tree,
            &authority.url,
            &["admin", "member", "add", "--target", &ordinary],
        );
        run(
            binary,
            &owner,
            &owner_tree,
            &authority.url,
            &[
                "admin",
                "folder-access",
                "grant",
                "--folder",
                "first",
                "--target",
                &ordinary,
            ],
        );
        run(
            &reader_binary,
            &owner,
            &owner_tree,
            &authority.url,
            &["admin", "member", "remove", "--target", &ordinary],
        );
        let safe_removal = run(
            binary,
            &owner,
            &owner_tree,
            &authority.url,
            &["brain", "export"],
        );
        assert!(
            !safe_removal["accessState"]["members"]
                .as_array()
                .unwrap()
                .contains(&json!(ordinary))
        );
        assert_eq!(
            safe_removal["folders"]
                .as_array()
                .unwrap()
                .iter()
                .find(|folder| folder["id"] == "first")
                .unwrap()["currentKeyVersion"],
            3
        );
        assert!(
            !safe_removal["keyGrants"]
                .as_array()
                .unwrap()
                .iter()
                .any(|grant| grant["recipientNpub"] == ordinary && grant["keyVersion"] == 3)
        );
    }
    assert!(
        !execute(
            binary,
            &target_home,
            &target_home,
            &authority.url,
            &["brain", "export", "--brain", "revocation"]
        )
        .status
        .success()
    );
    let before_shutdown = run(
        binary,
        &owner,
        &owner_tree,
        &authority.url,
        &["brain", "export"],
    );
    drop(authority);

    // Reopen existing durable state, then restore the same offline authority
    // database and surviving Finite Home into a separate, empty location.
    let restarted = server(&database);
    assert_eq!(
        before_shutdown,
        run(
            binary,
            &owner,
            &owner,
            &restarted.url,
            &["brain", "export", "--brain", "revocation"]
        )
    );
    drop(restarted);
    let wal = database.with_file_name("authority.sqlite3-wal");
    assert!(
        !wal.exists() || fs::metadata(wal).unwrap().len() == 0,
        "offline Recovery Set requires a clean checkpoint before copying the database"
    );
    let restored_database = scratch.path().join("restored.sqlite3");
    fs::copy(&database, &restored_database).unwrap();
    let restored_home = home(scratch.path(), "restored-home");
    fs::create_dir(restored_home.join("identity")).unwrap();
    fs::copy(
        survivor.join("identity/identity.json"),
        restored_home.join("identity/identity.json"),
    )
    .unwrap();
    let restored = server(&restored_database);
    let restored_tree = restored_home.join("tree");
    run(
        &reader_binary,
        &restored_home,
        &restored_home,
        &restored.url,
        &["open", "revocation", restored_tree.to_str().unwrap()],
    );
    run(
        &reader_binary,
        &restored_home,
        &restored_tree,
        &restored.url,
        &["sync", "now"],
    );
    assert_eq!(
        fs::read_to_string(restored_tree.join("First/wiki/keep.md")).unwrap(),
        "# Survivor edit\n"
    );
    assert_eq!(
        fs::read_to_string(restored_tree.join("Second/wiki/keep.md")).unwrap(),
        "# Existing knowledge\n"
    );
    assert_eq!(
        before_shutdown,
        run(
            binary,
            &restored_home,
            &restored_tree,
            &restored.url,
            &["brain", "export"]
        )
    );
    assert!(
        !execute(
            binary,
            &target_home,
            &target_home,
            &restored.url,
            &["brain", "export", "--brain", "revocation"]
        )
        .status
        .success()
    );
}

#[test]
#[ignore = "requires FBRAIN_COMPAT_SERVER_BINARY pointing to a pinned older server"]
fn reference_server_rejects_new_revocation_before_authority_changes() {
    let reference_server = std::env::var_os("FBRAIN_COMPAT_SERVER_BINARY")
        .expect("set FBRAIN_COMPAT_SERVER_BINARY to the pinned older server");
    let scratch = TempDir::new().unwrap();
    let authority = reference_authority(
        Path::new(&reference_server),
        &scratch.path().join("authority.sqlite3"),
    );
    let candidate_binary = candidate_binary();
    let binary = candidate_binary.as_path();
    let owner = home(scratch.path(), "owner");
    let target_home = home(scratch.path(), "target");
    let target = run(
        binary,
        &target_home,
        &target_home,
        &authority.url,
        &["signer", "public-key"],
    )["npub"]
        .as_str()
        .unwrap()
        .to_owned();
    create_fixture(binary, &owner, &target, &authority.url);
    run(
        binary,
        &owner,
        &owner,
        &authority.url,
        &[
            "folder",
            "create",
            "private",
            "--name",
            "Private",
            "--brain",
            "revocation",
        ],
    );
    let before = run(
        binary,
        &owner,
        &owner,
        &authority.url,
        &["brain", "export", "--brain", "revocation"],
    );
    for args in [
        vec![
            "admin",
            "role",
            "revoke",
            "admin",
            "--target",
            &target,
            "--brain",
            "revocation",
        ],
        vec![
            "admin",
            "member",
            "remove",
            "--target",
            &target,
            "--brain",
            "revocation",
        ],
    ] {
        let rejected = execute(binary, &owner, &owner, &authority.url, &args);
        assert!(!rejected.status.success());
        let reason = String::from_utf8_lossy(&rejected.stderr);
        assert!(
            reason.contains(if args[1] == "role" {
                "server does not support rotated admin demotion"
            } else {
                "server does not support complete admin removal"
            }),
            "must reject the unsupported operation, not its arguments: {reason}"
        );
        assert_eq!(
            before,
            run(
                binary,
                &owner,
                &owner,
                &authority.url,
                &["brain", "export", "--brain", "revocation"]
            )
        );
    }
}

#[test]
#[ignore = "requires FBRAIN_COMPAT_SERVER_BINARY and FBRAIN_COMPAT_BINARY from a pinned older release"]
fn reference_legacy_removal_is_repaired_through_the_signed_cli_without_reinvite() {
    let reference_server = std::env::var_os("FBRAIN_COMPAT_SERVER_BINARY")
        .expect("set FBRAIN_COMPAT_SERVER_BINARY to the pinned older server");
    let reference_cli = std::env::var_os("FBRAIN_COMPAT_BINARY")
        .expect("set FBRAIN_COMPAT_BINARY to the pinned older CLI");
    let scratch = TempDir::new().unwrap();
    let database = scratch.path().join("authority.sqlite3");
    let old = Path::new(&reference_cli);
    let candidate_binary = candidate_binary();
    let candidate = candidate_binary.as_path();
    let owner = home(scratch.path(), "owner");
    let target_home = home(scratch.path(), "target");
    let legacy = reference_authority(Path::new(&reference_server), &database);
    let target = run(
        old,
        &target_home,
        &target_home,
        &legacy.url,
        &["signer", "public-key"],
    )["npub"]
        .as_str()
        .unwrap()
        .to_owned();
    create_fixture(old, &owner, &target, &legacy.url);
    let tree = owner.join("tree");
    run(
        old,
        &owner,
        &owner,
        &legacy.url,
        &["open", "revocation", tree.to_str().unwrap()],
    );
    for (id, name) in [("first", "First"), ("second", "Second")] {
        run(
            old,
            &owner,
            &tree,
            &legacy.url,
            &[
                "folder",
                "create",
                id,
                "--name",
                name,
                "--path",
                name,
                "--access",
                "restricted",
            ],
        );
    }
    run(old, &owner, &tree, &legacy.url, &["sync", "now"]);
    for name in ["First", "Second"] {
        fs::write(tree.join(name).join("wiki/keep.md"), "# Legacy knowledge\n").unwrap();
    }
    run(old, &owner, &tree, &legacy.url, &["sync", "now"]);
    run(
        old,
        &owner,
        &tree,
        &legacy.url,
        &["admin", "role", "revoke", "admin", "--target", &target],
    );
    run(
        old,
        &owner,
        &tree,
        &legacy.url,
        &["admin", "member", "remove", "--target", &target],
    );
    let drift = run(old, &owner, &tree, &legacy.url, &["brain", "export"]);
    assert!(
        !drift["accessState"]["members"]
            .as_array()
            .unwrap()
            .contains(&json!(target))
    );
    assert!(
        !drift["accessState"]["admins"]
            .as_array()
            .unwrap()
            .contains(&json!(target))
    );
    assert!(
        drift["folders"]
            .as_array()
            .unwrap()
            .iter()
            .all(|folder| folder["currentKeyVersion"] == 1)
    );
    assert_eq!(
        drift["keyGrants"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|grant| grant["recipientNpub"] == target && grant["keyVersion"] == 1)
            .count(),
        2
    );
    drop(legacy);

    let repaired = server(&database);
    let receipt = run(
        candidate,
        &owner,
        &tree,
        &repaired.url,
        &["admin", "member", "remove", "--target", &target],
    );
    assert_eq!(receipt["state"], "complete");
    assert_eq!(receipt["outcome"], "changed");
    assert_eq!(receipt["folders"].as_array().unwrap().len(), 2);
    let after = run(
        candidate,
        &owner,
        &tree,
        &repaired.url,
        &["brain", "export"],
    );
    assert!(
        after["folders"]
            .as_array()
            .unwrap()
            .iter()
            .all(|folder| folder["currentKeyVersion"] == 2)
    );
    assert!(
        !after["keyGrants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|grant| grant["recipientNpub"] == target && grant["keyVersion"] == 2)
    );
    let cursor =
        run(candidate, &owner, &tree, &repaired.url, &["sync", "now"])["latestSequence"].clone();
    for name in ["First", "Second"] {
        assert_eq!(
            fs::read_to_string(tree.join(name).join("wiki/keep.md")).unwrap(),
            "# Legacy knowledge\n"
        );
    }
    let retry = run(
        candidate,
        &owner,
        &tree,
        &repaired.url,
        &["admin", "member", "remove", "--target", &target],
    );
    assert_eq!(retry["outcome"], "alreadyComplete");
    assert_eq!(
        after,
        run(
            candidate,
            &owner,
            &tree,
            &repaired.url,
            &["brain", "export"]
        )
    );
    assert_eq!(
        cursor,
        run(candidate, &owner, &tree, &repaired.url, &["sync", "now"])["latestSequence"]
    );
    let fresh_home = home(scratch.path(), "fresh-owner");
    fs::create_dir(fresh_home.join("identity")).unwrap();
    fs::copy(
        owner.join("identity/identity.json"),
        fresh_home.join("identity/identity.json"),
    )
    .unwrap();
    let fresh_tree = fresh_home.join("tree");
    run(
        candidate,
        &fresh_home,
        &fresh_home,
        &repaired.url,
        &["open", "revocation", fresh_tree.to_str().unwrap()],
    );
    run(
        candidate,
        &fresh_home,
        &fresh_tree,
        &repaired.url,
        &["sync", "now"],
    );
    for name in ["First", "Second"] {
        assert_eq!(
            fs::read_to_string(fresh_tree.join(name).join("wiki/keep.md")).unwrap(),
            "# Legacy knowledge\n",
            "repair must decrypt from a fresh tree with no cached Folder keys"
        );
    }
}
