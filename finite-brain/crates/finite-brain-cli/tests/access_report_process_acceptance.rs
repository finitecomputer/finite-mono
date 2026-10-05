use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use finite_identity::authority::{IdentityStore, NAME_LOOKUP_PATH, NAME_LOOKUP_TOKEN_HEADER};
use serde_json::{Value, json};
use tempfile::TempDir;

#[path = "access_report/harness.rs"]
mod harness;
use harness::{
    LOOKUP_TOKEN, OPERATOR_TOKEN, brain, candidate_binary, directory, execute, home, reference,
    run, schema_version,
};

fn create_content(binary: &Path, owner: &Path, url: &str) -> PathBuf {
    run(
        binary,
        owner,
        owner,
        url,
        &[
            "brain",
            "create",
            "access-proof",
            "--kind",
            "organization",
            "--name",
            "Access proof",
        ],
    );
    let tree = owner.join("tree");
    run(
        binary,
        owner,
        owner,
        url,
        &["open", "access-proof", tree.to_str().unwrap()],
    );
    run(
        binary,
        owner,
        &tree,
        url,
        &[
            "folder",
            "create",
            "knowledge",
            "--name",
            "Knowledge",
            "--path",
            "Knowledge",
            "--access",
            "restricted",
        ],
    );
    run(binary, owner, &tree, url, &["sync", "now"]);
    fs::write(
        tree.join("Knowledge/wiki/retained.md"),
        "# Retained knowledge\n",
    )
    .unwrap();
    run(binary, owner, &tree, url, &["sync", "now"]);
    tree
}

fn report(binary: &Path, owner: &Path, url: &str) -> Value {
    run(
        binary,
        owner,
        owner,
        url,
        &["access", "list", "--brain", "access-proof"],
    )
}

fn row<'a>(report: &'a Value, npub: &str) -> &'a Value {
    report["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["npub"] == npub)
        .unwrap()
}

fn accept_folder_as_guest(
    binary: &Path,
    owner: &Path,
    tree: &Path,
    guest: &Path,
    url: &str,
) -> String {
    let key = run(binary, guest, guest, url, &["signer", "public-key"])["npub"]
        .as_str()
        .unwrap()
        .to_owned();
    let invitation = run(
        binary,
        owner,
        tree,
        url,
        &[
            "invite",
            "folder",
            "create",
            "--brain",
            "access-proof",
            "--folder",
            "knowledge",
            "--target",
            &key,
        ],
    );
    let accepted = run(
        binary,
        guest,
        guest,
        url,
        &[
            "invite",
            "folder",
            "accept",
            "--id",
            invitation["id"].as_str().unwrap(),
        ],
    );
    assert_eq!(accepted["status"], "accepted");
    assert!(accepted["acceptedAt"].as_str().is_some());
    key
}

fn assert_named_folder_guest(report: &Value, key: &str, name: &str) {
    let guest = row(report, key);
    assert_eq!(guest["brainRole"], "guest");
    assert_eq!(guest["participation"]["kind"], "folderInvitationAcceptance");
    assert_eq!(guest["name"]["state"], "verified");
    assert_eq!(guest["name"]["display"], name);
    assert_eq!(guest["folders"][0]["currentGrant"], "present");
}

#[test]
fn built_cli_uses_private_directory_and_withholds_names_until_exact_key_participates() {
    let scratch = TempDir::new().unwrap();
    let binary = candidate_binary();
    let owner = home(scratch.path(), "owner");
    let added = home(scratch.path(), "added");
    let store = IdentityStore::open_memory().unwrap();
    let private_directory = directory(store.clone(), false);
    let public_directory = directory(store.clone(), true);
    let authority = brain(
        &scratch.path().join("brain.sqlite3"),
        Some(&private_directory.url),
    );
    let owner_key = run(
        &binary,
        &owner,
        &owner,
        &authority.url,
        &["signer", "public-key"],
    )["npub"]
        .as_str()
        .unwrap()
        .to_owned();
    let added_key = run(
        &binary,
        &added,
        &added,
        &authority.url,
        &["signer", "public-key"],
    )["npub"]
        .as_str()
        .unwrap()
        .to_owned();
    for (name, key) in [
        ("owner@fixture.invalid", &owner_key),
        ("private-added@fixture.invalid", &added_key),
    ] {
        let hex = finite_identity::hex::encode(&finite_identity::npub::decode(key).unwrap());
        store.bind_vip_email(name, &hex, 1_780_000_000).unwrap();
    }
    let http = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();
    let payload = json!({"pubkeys": [added_key]}).to_string();
    // The same exact name is available with the dedicated private credential,
    // but neither the public router nor an operator credential grants access.
    let direct: Value = http
        .post(&format!("{}{NAME_LOOKUP_PATH}", private_directory.url))
        .set(NAME_LOOKUP_TOKEN_HEADER, LOOKUP_TOKEN)
        .set("Content-Type", "application/json")
        .send_string(&payload)
        .unwrap()
        .into_json()
        .unwrap();
    assert_eq!(
        direct["results"][0]["names"][0]["name"],
        "private-added@fixture.invalid"
    );
    for (url, token, expected) in [
        (&public_directory.url, LOOKUP_TOKEN, 404),
        (&private_directory.url, OPERATOR_TOKEN, 401),
    ] {
        let error = http
            .post(&format!("{url}{NAME_LOOKUP_PATH}"))
            .set(NAME_LOOKUP_TOKEN_HEADER, token)
            .set("Content-Type", "application/json")
            .send_string(&payload)
            .unwrap_err();
        assert!(matches!(error, ureq::Error::Status(status, _) if status == expected));
    }
    let tree = create_content(&binary, &owner, &authority.url);
    run(
        &binary,
        &owner,
        &owner,
        &authority.url,
        &[
            "collaborator",
            "ensure-admin",
            "--brain",
            "access-proof",
            "--target",
            &added_key,
        ],
    );
    let export_before = run(&binary, &owner, &tree, &authority.url, &["brain", "export"]);
    let before = report(&binary, &owner, &authority.url);
    assert_eq!(before["version"], "finite-brain-access-report-v1");
    assert_eq!(before["currentAccessComplete"], true);
    assert_eq!(before["totals"]["identities"], 2);
    assert_eq!(before["totals"]["grantsMissing"], 0);
    assert_eq!(row(&before, &owner_key)["name"]["state"], "verified");
    assert_eq!(
        row(&before, &owner_key)["name"]["display"],
        "owner@fixture.invalid"
    );
    assert_eq!(row(&before, &added_key)["brainRole"], "admin");
    assert_eq!(row(&before, &added_key)["name"]["state"], "unknown");
    assert!(!before.to_string().contains("private-added@fixture.invalid"));
    let human_before = execute(
        &binary,
        &owner,
        &owner,
        &authority.url,
        &["access", "list", "--brain", "access-proof"],
        false,
    );
    assert!(
        human_before.status.success(),
        "{}",
        String::from_utf8_lossy(&human_before.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&human_before.stdout).contains("private-added@fixture.invalid")
    );
    assert_eq!(
        export_before,
        run(&binary, &owner, &tree, &authority.url, &["brain", "export"])
    );

    let added_tree = added.join("tree");
    run(
        &binary,
        &added,
        &added,
        &authority.url,
        &["open", "access-proof", added_tree.to_str().unwrap()],
    );
    run(
        &binary,
        &added,
        &added_tree,
        &authority.url,
        &["sync", "now"],
    );
    assert_eq!(
        fs::read_to_string(added_tree.join("Knowledge/wiki/retained.md")).unwrap(),
        "# Retained knowledge\n"
    );
    fs::write(
        added_tree.join("Knowledge/wiki/participation.md"),
        "# Signed participation\n",
    )
    .unwrap();
    run(
        &binary,
        &added,
        &added_tree,
        &authority.url,
        &["sync", "now"],
    );
    let after = report(&binary, &owner, &authority.url);
    let participating = row(&after, &added_key);
    assert_eq!(participating["name"]["state"], "verified");
    assert_eq!(
        participating["name"]["display"],
        "private-added@fixture.invalid"
    );
    assert_eq!(participating["name"]["matchedKey"], participating["hex"]);
    assert_eq!(participating["identityType"]["value"], "notConfirmed");
    assert_eq!(participating["folders"][0]["entitled"], true);
    assert_eq!(participating["folders"][0]["currentGrant"], "present");
    assert_eq!(participating["folders"][0]["state"], "ready");
    assert_eq!(participating["folders"][0]["grant"]["issuedBy"], owner_key);
    let human = execute(
        &binary,
        &owner,
        &owner,
        &authority.url,
        &["access", "list", "--brain", "access-proof"],
        false,
    );
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let human = String::from_utf8(human.stdout).unwrap();
    for expected in [
        "Current access: complete",
        "private-added@fixture.invalid",
        "owner@fixture.invalid",
        "current grant present",
        "verified for this exact key",
        &format!("grant issued by {owner_key}"),
        participating["folders"][0]["grant"]["issuedAt"]
            .as_str()
            .unwrap(),
    ] {
        assert!(human.contains(expected), "missing {expected:?}: {human}");
    }
    // The ordinary process suite covers a Guest who only accepts and reads,
    // without needing baseline binaries or a recipient content write.
    let guest = home(scratch.path(), "read-only-guest");
    let guest_key = accept_folder_as_guest(&binary, &owner, &tree, &guest, &authority.url);
    let guest_hex =
        finite_identity::hex::encode(&finite_identity::npub::decode(&guest_key).unwrap());
    store
        .bind_vip_email("read-only-guest@fixture.invalid", &guest_hex, 1_780_000_000)
        .unwrap();
    let with_guest = report(&binary, &owner, &authority.url);
    assert_named_folder_guest(&with_guest, &guest_key, "read-only-guest@fixture.invalid");
    // Losing the optional Directory changes evidence, never roles or grants.
    drop(private_directory);
    let unavailable = report(&binary, &owner, &authority.url);
    assert_eq!(unavailable["directory"]["state"], "unavailable");
    assert_eq!(unavailable["currentAccessComplete"], true);
    assert_eq!(unavailable["totals"], with_guest["totals"]);
    assert_eq!(
        row(&unavailable, &added_key)["folders"],
        participating["folders"]
    );
    assert_ne!(row(&unavailable, &added_key)["name"]["state"], "verified");
}

#[test]
#[ignore = "requires actual baseline CLI and server via FBRAIN_COMPAT_BINARY and FBRAIN_COMPAT_SERVER_BINARY"]
fn actual_v29_server_reopens_v31_without_losing_authority_or_content() {
    let old_cli =
        PathBuf::from(std::env::var_os("FBRAIN_COMPAT_BINARY").expect("set actual baseline CLI"));
    let old_server = PathBuf::from(
        std::env::var_os("FBRAIN_COMPAT_SERVER_BINARY").expect("set actual baseline server"),
    );
    assert!(old_cli.is_file() && old_server.is_file());
    let binary = candidate_binary();
    let scratch = TempDir::new().unwrap();
    let owner = home(scratch.path(), "owner");
    let guest_before = home(scratch.path(), "guest-before-migration");
    let guest_after = home(scratch.path(), "guest-after-migration");
    let names = IdentityStore::open_memory().unwrap();
    let private_directory = directory(names.clone(), false);
    let database = scratch.path().join("authority.sqlite3");
    let old = reference(&old_server, &database);
    let tree = create_content(&old_cli, &owner, &old.url);
    let first_guest = accept_folder_as_guest(&old_cli, &owner, &tree, &guest_before, &old.url);
    let first_hex =
        finite_identity::hex::encode(&finite_identity::npub::decode(&first_guest).unwrap());
    names
        .bind_vip_email(
            "old-guest-before@fixture.invalid",
            &first_hex,
            1_780_000_000,
        )
        .unwrap();
    let before = run(&old_cli, &owner, &tree, &old.url, &["brain", "export"]);
    let unsupported = execute(
        &binary,
        &owner,
        &owner,
        &old.url,
        &["access", "list", "--brain", "access-proof"],
        true,
    );
    assert!(!unsupported.status.success());
    let error = String::from_utf8_lossy(&unsupported.stderr);
    assert!(
        error.contains("Upgrade") || error.contains("upgrade"),
        "{error}"
    );
    assert!(!String::from_utf8_lossy(&unsupported.stdout).contains("currentAccessComplete"));
    assert_eq!(
        before,
        run(&old_cli, &owner, &tree, &old.url, &["brain", "export"])
    );
    drop(old);
    assert_eq!(
        schema_version(&database),
        29,
        "reference must be actual pre-V31 server"
    );

    let candidate = brain(&database, Some(&private_directory.url));
    assert_eq!(
        before,
        run(&binary, &owner, &tree, &candidate.url, &["brain", "export"])
    );
    let migrated_report = report(&binary, &owner, &candidate.url);
    assert_eq!(migrated_report["currentAccessComplete"], true);
    assert_named_folder_guest(
        &migrated_report,
        &first_guest,
        "old-guest-before@fixture.invalid",
    );
    run(&binary, &owner, &tree, &candidate.url, &["sync", "now"]);
    assert_eq!(
        fs::read_to_string(tree.join("Knowledge/wiki/retained.md")).unwrap(),
        "# Retained knowledge\n"
    );
    drop(candidate);
    assert_eq!(schema_version(&database), 31);

    let reopened = reference(&old_server, &database);
    assert_eq!(
        before,
        run(&old_cli, &owner, &tree, &reopened.url, &["brain", "export"])
    );
    let fresh_tree = owner.join("fresh-old-reader");
    run(
        &old_cli,
        &owner,
        &owner,
        &reopened.url,
        &["open", "access-proof", fresh_tree.to_str().unwrap()],
    );
    run(
        &old_cli,
        &owner,
        &fresh_tree,
        &reopened.url,
        &["sync", "now"],
    );
    assert_eq!(
        fs::read_to_string(fresh_tree.join("Knowledge/wiki/retained.md")).unwrap(),
        "# Retained knowledge\n"
    );
    // The actual old writer also maintains the new acceptance indexes when
    // it accepts an invitation after reopening the V31 database.
    let second_guest =
        accept_folder_as_guest(&old_cli, &owner, &fresh_tree, &guest_after, &reopened.url);
    let second_hex =
        finite_identity::hex::encode(&finite_identity::npub::decode(&second_guest).unwrap());
    names
        .bind_vip_email(
            "old-guest-after@fixture.invalid",
            &second_hex,
            1_780_000_000,
        )
        .unwrap();
    fs::write(
        fresh_tree.join("Knowledge/wiki/retained.md"),
        "# Edited by baseline after V31\n",
    )
    .unwrap();
    run(
        &old_cli,
        &owner,
        &fresh_tree,
        &reopened.url,
        &["sync", "now"],
    );
    let after_old_write = run(
        &old_cli,
        &owner,
        &fresh_tree,
        &reopened.url,
        &["brain", "export"],
    );
    assert_ne!(before, after_old_write);
    drop(reopened);
    assert_eq!(schema_version(&database), 31);

    let final_server = brain(&database, Some(&private_directory.url));
    assert_eq!(
        after_old_write,
        run(
            &binary,
            &owner,
            &tree,
            &final_server.url,
            &["brain", "export"]
        )
    );
    let fresh_candidate = owner.join("fresh-candidate-reader");
    run(
        &binary,
        &owner,
        &owner,
        &final_server.url,
        &["open", "access-proof", fresh_candidate.to_str().unwrap()],
    );
    run(
        &binary,
        &owner,
        &fresh_candidate,
        &final_server.url,
        &["sync", "now"],
    );
    assert_eq!(
        fs::read_to_string(fresh_candidate.join("Knowledge/wiki/retained.md")).unwrap(),
        "# Edited by baseline after V31\n"
    );
    let final_report = report(&binary, &owner, &final_server.url);
    assert_named_folder_guest(
        &final_report,
        &first_guest,
        "old-guest-before@fixture.invalid",
    );
    assert_named_folder_guest(
        &final_report,
        &second_guest,
        "old-guest-after@fixture.invalid",
    );
    assert_eq!(final_report["currentAccessComplete"], true);
    assert_eq!(final_report["totals"]["grantsMissing"], 0);
}
