use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::TempDir;

#[path = "access_report/harness.rs"]
mod harness;
use harness::{
    CoreFixture, brain, candidate_binary, core_stub, execute, home, reference, run, schema_version,
};

fn hex(npub: &str) -> String {
    finite_identity::hex::encode(&finite_identity::npub::decode(npub).unwrap())
}

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

fn assert_named_folder_guest(report: &Value, key: &str, email: &str) {
    let guest = row(report, key);
    assert_eq!(guest["brainRole"], "guest");
    assert_eq!(guest["participation"]["kind"], "folderInvitationAcceptance");
    assert_eq!(guest["description"]["state"], "resolved");
    assert_eq!(guest["description"]["accountEmail"], email);
    assert_eq!(guest["folders"][0]["currentGrant"], "present");
}

#[test]
fn built_cli_reports_core_descriptions_only_for_exact_participating_keys() {
    let scratch = TempDir::new().unwrap();
    let binary = candidate_binary();
    let owner = home(scratch.path(), "owner");
    let added = home(scratch.path(), "added");
    let core = CoreFixture::default();
    let core_server = core_stub(core.clone());
    let authority = brain(
        &scratch.path().join("brain.sqlite3"),
        Some(&core_server.url),
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
    // Arbitrary-domain account contact, as Core would describe it.
    core.human(&hex(&owner_key), "owner@fixture.example");
    core.human(&hex(&added_key), "private-added@other-domain.example");
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
    assert_eq!(row(&before, &owner_key)["description"]["state"], "resolved");
    assert_eq!(
        row(&before, &owner_key)["description"]["accountEmail"],
        "owner@fixture.example"
    );
    assert_eq!(row(&before, &added_key)["brainRole"], "admin");
    assert_eq!(
        row(&before, &added_key)["description"]["state"],
        "notShared"
    );
    assert_eq!(
        row(&before, &added_key)["description"]["reason"],
        "noParticipation"
    );
    assert!(
        !before
            .to_string()
            .contains("private-added@other-domain.example")
    );
    // The admin-added key was never sent to Core.
    assert!(!core.requested().contains(&hex(&added_key)));
    assert!(core.requested().contains(&hex(&owner_key)));
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
        !String::from_utf8_lossy(&human_before.stdout)
            .contains("private-added@other-domain.example")
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
    assert_eq!(participating["description"]["state"], "resolved");
    assert_eq!(
        participating["description"]["accountEmail"],
        "private-added@other-domain.example"
    );
    assert_eq!(
        participating["participation"]["kind"],
        "authenticatedBrainAction"
    );
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
        "private-added@other-domain.example · human · admin",
        "owner@fixture.example · human",
        "current grant present",
        "identity: hostedDeviceObservation observed",
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
    core.human(&hex(&guest_key), "read-only-guest@fixture.example");
    let with_guest = report(&binary, &owner, &authority.url);
    assert_named_folder_guest(&with_guest, &guest_key, "read-only-guest@fixture.example");
    // Losing optional Core changes descriptions, never roles or grants.
    drop(core_server);
    let unavailable = report(&binary, &owner, &authority.url);
    assert_eq!(unavailable["descriptions"]["state"], "unavailable");
    assert_eq!(unavailable["currentAccessComplete"], true);
    assert_eq!(unavailable["totals"], with_guest["totals"]);
    assert_eq!(
        row(&unavailable, &added_key)["folders"],
        participating["folders"]
    );
    assert_eq!(
        row(&unavailable, &added_key)["description"]["state"],
        "unavailable"
    );
    assert_eq!(
        unavailable["identities"].as_array().unwrap().len(),
        with_guest["identities"].as_array().unwrap().len()
    );
}

#[test]
#[ignore = "requires actual baseline CLI and server via FBRAIN_COMPAT_BINARY and FBRAIN_COMPAT_SERVER_BINARY"]
fn actual_v29_server_reopens_v30_without_losing_authority_or_content() {
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
    let core = CoreFixture::default();
    let core_server = core_stub(core.clone());
    let database = scratch.path().join("authority.sqlite3");
    let old = reference(&old_server, &database);
    let tree = create_content(&old_cli, &owner, &old.url);
    let first_guest = accept_folder_as_guest(&old_cli, &owner, &tree, &guest_before, &old.url);
    core.human(&hex(&first_guest), "old-guest-before@fixture.example");
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
        "reference must be actual pre-V30 server"
    );

    let candidate = brain(&database, Some(&core_server.url));
    assert_eq!(
        before,
        run(&binary, &owner, &tree, &candidate.url, &["brain", "export"])
    );
    let migrated_report = report(&binary, &owner, &candidate.url);
    assert_eq!(migrated_report["currentAccessComplete"], true);
    assert_named_folder_guest(
        &migrated_report,
        &first_guest,
        "old-guest-before@fixture.example",
    );
    run(&binary, &owner, &tree, &candidate.url, &["sync", "now"]);
    assert_eq!(
        fs::read_to_string(tree.join("Knowledge/wiki/retained.md")).unwrap(),
        "# Retained knowledge\n"
    );
    drop(candidate);
    assert_eq!(schema_version(&database), 30);

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
    // it accepts an invitation after reopening the V30 database.
    let second_guest =
        accept_folder_as_guest(&old_cli, &owner, &fresh_tree, &guest_after, &reopened.url);
    core.human(&hex(&second_guest), "old-guest-after@fixture.example");
    fs::write(
        fresh_tree.join("Knowledge/wiki/retained.md"),
        "# Edited by baseline after V30\n",
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
    assert_eq!(schema_version(&database), 30);

    let final_server = brain(&database, Some(&core_server.url));
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
        "# Edited by baseline after V30\n"
    );
    let final_report = report(&binary, &owner, &final_server.url);
    assert_named_folder_guest(
        &final_report,
        &first_guest,
        "old-guest-before@fixture.example",
    );
    assert_named_folder_guest(
        &final_report,
        &second_guest,
        "old-guest-after@fixture.example",
    );
    assert_eq!(final_report["currentAccessComplete"], true);
    assert_eq!(final_report["totals"]["grantsMissing"], 0);
}
