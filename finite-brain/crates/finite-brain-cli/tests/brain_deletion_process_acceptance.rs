//! Organization Brain deletion end to end: the built `fbrain` against a real
//! file-backed Brain server, with synthetic stand-ins for V1, content-brain
//! and marketing-comms-brain. Set `FBRAIN_BRAIN_DELETION_REPORT` to a path to
//! keep a JSON evidence report of the boundaries that passed.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use tempfile::TempDir;

#[path = "brain_deletion/harness.rs"]
mod harness;
use harness::*;

#[path = "brain_deletion/local_bookkeeping.rs"]
mod local_bookkeeping;

#[test]
fn built_fbrain_deletes_v1_and_leaves_content_and_marketing_brains_intact() {
    let scratch = TempDir::new().unwrap();
    let root = scratch.path();
    let mut evidence = Evidence {
        path: std::env::var_os("FBRAIN_BRAIN_DELETION_REPORT").map(PathBuf::from),
        facts: serde_json::Map::new(),
    };
    let admin = home(root, "admin-human", ADMIN_SECRET);
    let agent = home(root, "admin-agent", AGENT_SECRET);
    let member = home(root, "member", MEMBER_SECRET);
    let invitee = home(root, "invitee", INVITEE_SECRET);
    let outsider = home(root, "outsider", OUTSIDER_SECRET);
    let database = root.join("state/brain.sqlite3");
    fs::create_dir_all(database.parent().unwrap()).unwrap();
    let server = Server::start(&database, None);
    let url = server.url.clone();
    let url = url.as_str();

    // Three synthetic Organization Brains.
    for (id, name) in [
        ("v1", "V1"),
        ("content-brain", "Content"),
        ("marketing-comms-brain", "Marketing Comms"),
    ] {
        ok(
            &admin,
            &admin.dir,
            url,
            &[
                "brain",
                "create",
                id,
                "--kind",
                "organization",
                "--name",
                name,
            ],
        );
    }
    let v1_tree = admin.dir.join("v1");
    ok(
        &admin,
        &admin.dir,
        url,
        &["open", "v1", v1_tree.to_str().unwrap()],
    );
    ok(
        &admin,
        &v1_tree,
        url,
        &["folder", "create", "Knowledge", "--access", "all_members"],
    );
    ok(&admin, &v1_tree, url, &["folder", "create", "Strategy"]);
    for target in [&member.npub, &agent.npub] {
        ok(
            &admin,
            &v1_tree,
            url,
            &["admin", "member", "add", "--target", target],
        );
    }
    ok(
        &admin,
        &v1_tree,
        url,
        &["admin", "role", "grant", "admin", "--target", &agent.npub],
    );
    ok(&admin, &v1_tree, url, &["sync", "now"]);
    fs::write(
        v1_tree.join("Knowledge/wiki/alpha.md"),
        "# Alpha\n\nShared V1 page.\n",
    )
    .unwrap();
    fs::write(
        v1_tree.join("Strategy/wiki/plan.md"),
        "# Plan\n\nRestricted V1 page.\n",
    )
    .unwrap();
    ok(&admin, &v1_tree, url, &["sync", "now"]);
    let mut other_trees = Vec::new();
    for (id, folder, page) in [
        ("content-brain", "Notes", "content.md"),
        ("marketing-comms-brain", "Campaigns", "launch.md"),
    ] {
        let tree = admin.dir.join(id);
        ok(
            &admin,
            &admin.dir,
            url,
            &["open", id, tree.to_str().unwrap()],
        );
        ok(&admin, &tree, url, &["folder", "create", folder]);
        ok(&admin, &tree, url, &["sync", "now"]);
        fs::write(
            tree.join(format!("{folder}/wiki/{page}")),
            format!("# {id}\n\nKept.\n"),
        )
        .unwrap();
        ok(&admin, &tree, url, &["sync", "now"]);
        other_trees.push(tree);
    }
    let member_tree = member.dir.join("v1");
    ok(
        &member,
        &member.dir,
        url,
        &["open", "v1", member_tree.to_str().unwrap()],
    );
    ok(&member, &member_tree, url, &["sync", "now"]);
    assert_eq!(
        fs::read_to_string(member_tree.join("Knowledge/wiki/alpha.md")).unwrap(),
        "# Alpha\n\nShared V1 page.\n"
    );
    let invitation = ok(
        &admin,
        &v1_tree,
        url,
        &["invite", "brain", "create", "--target", &invitee.npub],
    );
    let invitation_id = invitation["id"].as_str().unwrap().to_owned();
    let token = ok(&admin, &v1_tree, url, &["invite-token", "create"]);
    let token_url = token["url"].as_str().unwrap().to_owned();
    evidence.pass(
        "fixtures",
        json!({"brains": ["v1", "content-brain", "marketing-comms-brain"], "server": url}),
    );

    // Only current admins may preview or delete; there is no bypass flag.
    assert!(
        refused(&member, &member.dir, url, &["brain", "delete", "v1"])
            .contains("destructive authority")
    );
    assert!(
        refused(&outsider, &outsider.dir, url, &["brain", "delete", "v1"])
            .contains("brain access required")
    );
    assert!(
        refused(
            &admin,
            &admin.dir,
            url,
            &["brain", "delete", "v1", "--force"]
        )
        .contains("has no --force")
    );
    evidence.pass("memberAndOutsiderDenied", json!(true));

    // The preview names the server, Brain and scope; text mode says nothing
    // was deleted.
    let first = ok(&admin, &admin.dir, url, &["brain", "delete", "v1"]);
    assert_eq!(first["state"], "confirmation-required");
    let scope = &first["preview"];
    assert_eq!(
        (
            scope["server"].as_str(),
            scope["brainId"].as_str(),
            scope["name"].as_str()
        ),
        (Some(url), Some("v1"), Some("V1"))
    );
    assert_eq!(
        (scope["folderCount"].as_u64(), scope["pageCount"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(
        (scope["memberCount"].as_u64(), scope["adminCount"].as_u64()),
        (Some(3), Some(2))
    );
    assert_eq!(
        (
            scope["pendingInvitationCount"].as_u64(),
            scope["openInviteTokenCount"].as_u64()
        ),
        (Some(1), Some(1))
    );
    let text = command(&admin.dir, &admin.dir, url)
        .args(["brain", "delete", "v1"])
        .output()
        .unwrap();
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.contains(&format!("Delete Organization Brain \"V1\" (v1) on {url}?")),
        "{text}"
    );
    assert!(text.contains("Nothing was deleted."), "{text}");
    let first_digest = scope["stateDigest"].as_str().unwrap().to_owned();

    // An edit after the preview keeps every count but invalidates it.
    fs::write(
        v1_tree.join("Strategy/wiki/plan.md"),
        "# Plan\n\nEdited after preview.\n",
    )
    .unwrap();
    ok(&admin, &v1_tree, url, &["sync", "now"]);
    let edited = ok(&admin, &admin.dir, url, &["brain", "delete", "v1"]);
    assert_eq!(edited["preview"]["pageCount"], scope["pageCount"]);
    assert_ne!(
        edited["preview"]["stateDigest"].as_str().unwrap(),
        first_digest
    );
    assert!(
        refused(
            &admin,
            &admin.dir,
            url,
            &["brain", "delete", "v1", "--confirm", &first_digest]
        )
        .contains("changed since the preview you confirmed")
    );
    evidence.pass("stalePreviewWithUnchangedCountsRefused", json!(true));

    // Active Mounts in both directions block deletion until revoked.
    let marketing_tree = &other_trees[1];
    let content_tree = &other_trees[0];
    let offer_out = ok(
        &admin,
        &v1_tree.join("Strategy"),
        url,
        &[
            "mount",
            "offer",
            "create",
            "--destination-brain",
            "marketing-comms-brain",
            "--destination-controller",
            &admin.npub,
        ],
    );
    let mount_out = ok(
        &admin,
        marketing_tree,
        url,
        &["mount", "accept", offer_out["id"].as_str().unwrap()],
    );
    let offer_in = ok(
        &admin,
        &content_tree.join("Notes"),
        url,
        &[
            "mount",
            "offer",
            "create",
            "--destination-brain",
            "v1",
            "--destination-controller",
            &admin.npub,
        ],
    );
    let mount_in = ok(
        &admin,
        &v1_tree,
        url,
        &["mount", "accept", offer_in["id"].as_str().unwrap()],
    );
    let (mount_out, mount_in) = (
        mount_out["mountId"].as_str().unwrap().to_owned(),
        mount_in["mountId"].as_str().unwrap().to_owned(),
    );
    let blocked = ok(&admin, &admin.dir, url, &["brain", "delete", "v1"]);
    assert_eq!(blocked["state"], "blocked");
    let mut directions = blocked["preview"]["blockingMounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|mount| {
            (
                mount["connectionId"].as_str().unwrap().to_owned(),
                mount["direction"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    directions.sort();
    let mut expected = vec![
        (mount_in.clone(), "incoming".to_owned()),
        (mount_out.clone(), "outgoing".to_owned()),
    ];
    expected.sort();
    assert_eq!(directions, expected);
    let blocked_digest = blocked["preview"]["stateDigest"].as_str().unwrap();
    assert!(
        refused(
            &admin,
            &admin.dir,
            url,
            &["brain", "delete", "v1", "--confirm", blocked_digest]
        )
        .contains("active cross-Brain Mounts")
    );
    ok(
        &admin,
        marketing_tree,
        url,
        &["mount", "revoke", &mount_out],
    );
    ok(&admin, &v1_tree, url, &["mount", "revoke", &mount_in]);
    evidence.pass(
        "activeMountsBothDirectionsBlocked",
        json!({"outgoing": mount_out, "incoming": mount_in}),
    );

    // Backup for the restore drill: stop, copy, restart on the same address.
    let address = server.stop();
    let backup = root.join("backup/brain.sqlite3");
    copy_database(&database, &backup);
    let server = Server::start(&database, Some(address));
    assert_eq!(server.url, url);

    // Optional older client: a pre-deletion fbrain opens its own V1 tree.
    let compat = std::env::var_os("FBRAIN_COMPAT_BINARY").map(PathBuf::from);
    let compat_tree = member.dir.join("v1-older-client");
    if let Some(binary) = &compat {
        let opened = command_with(binary, &member.dir, &member.dir, url)
            .args(["open", "v1", compat_tree.to_str().unwrap(), "--json"])
            .output()
            .unwrap();
        assert!(opened.status.success(), "{}", stderr(&opened));
        assert!(compat_tree.join("Knowledge/wiki/alpha.md").is_file());
    }

    // Unsynced local work in the Member's Working Tree.
    fs::write(
        member_tree.join("Knowledge/wiki/alpha.md"),
        "# Alpha\n\nMember edit never synced.\n",
    )
    .unwrap();
    fs::write(
        member_tree.join("Knowledge/wiki/new-idea.md"),
        "# New idea\n\nNever synced.\n",
    )
    .unwrap();

    let content_rows = brain_rows(&database, "content-brain");
    let marketing_rows = brain_rows(&database, "marketing-comms-brain");
    let content_export = ok(
        &admin,
        &admin.dir,
        url,
        &["brain", "export", "--brain", "content-brain"],
    );
    let marketing_export = ok(
        &admin,
        &admin.dir,
        url,
        &["brain", "export", "--brain", "marketing-comms-brain"],
    );

    // An Agent admin confirms the exact current preview.
    let current = ok(&agent, &agent.dir, url, &["brain", "delete", "v1"]);
    assert_eq!(current["state"], "confirmation-required");
    let digest = current["preview"]["stateDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    let deleted = ok(
        &agent,
        &agent.dir,
        url,
        &["brain", "delete", "v1", "--confirm", &digest],
    );
    assert_eq!(deleted["state"], "deleted");
    assert_eq!(deleted["duplicate"], false);
    let receipt = &deleted["receipt"];
    assert_eq!(receipt["brainId"], "v1");
    assert_eq!(receipt["actorNpub"], agent.npub.as_str());
    assert_eq!(receipt["stateDigest"], digest.as_str());
    assert_eq!(
        (
            receipt["folderCount"].as_u64(),
            receipt["pageCount"].as_u64()
        ),
        (Some(2), Some(2))
    );
    evidence.pass("agentAdminDeletedV1", receipt.clone());

    // V1 is gone from every list and from the store; the others are intact.
    for person in [&admin, &agent, &member] {
        let listed = ok(person, &person.dir, url, &["brain", "list"]);
        assert!(
            !listed["brains"]
                .as_array()
                .unwrap()
                .iter()
                .any(|brain| brain["brainId"] == "v1"),
            "{listed}"
        );
    }
    assert!(brain_rows(&database, "v1").is_empty());
    let mut retired_records = 0;
    for (brain_id, before) in [
        ("content-brain", &content_rows),
        ("marketing-comms-brain", &marketing_rows),
    ] {
        retired_records +=
            assert_only_retired_mount_records_changed(before, &brain_rows(&database, brain_id));
    }
    assert_eq!(
        ok(
            &admin,
            &admin.dir,
            url,
            &["brain", "export", "--brain", "content-brain"]
        ),
        content_export
    );
    assert_eq!(
        ok(
            &admin,
            &admin.dir,
            url,
            &["brain", "export", "--brain", "marketing-comms-brain"]
        ),
        marketing_export
    );
    evidence.pass(
        "otherBrainsIntact",
        json!({
            "encryptedExportsByteIdentical": true,
            "contentRows": content_rows.len(),
            "marketingRows": marketing_rows.len(),
            "retiredMountRecordsNamingV1": retired_records,
        }),
    );

    // Repeating the deletion reports the terminal state.
    assert!(
        refused(
            &agent,
            &agent.dir,
            url,
            &["brain", "delete", "v1", "--confirm", &digest]
        )
        .contains("was already permanently deleted")
    );
    // Invitations and Invite Tokens fail like unknown ones.
    assert!(
        !refused(
            &invitee,
            &invitee.dir,
            url,
            &["invite", "brain", "accept", "--id", &invitation_id]
        )
        .is_empty()
    );
    let token_error = refused(&invitee, &invitee.dir, url, &["invite-accept", &token_url]);
    let unknown_error = refused(
        &invitee,
        &invitee.dir,
        url,
        &[
            "invite-accept",
            &format!("{url}/v1/invite-tokens/redeem#fbit-unknownunknownunknownunknown00"),
        ],
    );
    assert_eq!(token_error, unknown_error);
    // Unrelated identities see exactly an unknown Brain.
    let outsider_v1 = refused(&outsider, &outsider.dir, url, &["brain", "metadata", "v1"]);
    let outsider_unknown = refused(
        &outsider,
        &outsider.dir,
        url,
        &["brain", "metadata", "never-existed"],
    );
    assert_eq!(outsider_v1.replace("v1", "never-existed"), outsider_unknown);
    // Former readers learn it was deleted; nobody can recreate the id.
    assert!(
        refused(&member, &member.dir, url, &["brain", "metadata", "v1"])
            .contains("Brain v1 was permanently deleted on the server")
    );
    for person in [&admin, &outsider] {
        assert!(
            refused(
                person,
                &person.dir,
                url,
                &[
                    "brain",
                    "create",
                    "v1",
                    "--kind",
                    "organization",
                    "--name",
                    "V1"
                ]
            )
            .contains("duplicate id for brain_id: v1")
        );
    }
    assert!(
        refused(
            &admin,
            &v1_tree,
            url,
            &["folder", "create", "Revived", "--brain", "v1"]
        )
        .contains("Brain v1 was permanently deleted on the server")
    );
    evidence.pass("invitationsTokensRecreateAndProbingRefused", json!(true));

    // The Member's stale client stops for good and keeps its files.
    let stopped = refused(&member, &member_tree, url, &["sync", "now"]);
    assert!(
        stopped.contains("was permanently deleted on the server"),
        "{stopped}"
    );
    assert!(stopped.contains("Knowledge/wiki/new-idea.md"), "{stopped}");
    assert_eq!(
        fs::read_to_string(member_tree.join("Knowledge/wiki/alpha.md")).unwrap(),
        "# Alpha\n\nMember edit never synced.\n"
    );
    assert!(member_tree.join("Knowledge/wiki/new-idea.md").is_file());
    let state: Value = serde_json::from_slice(
        &fs::read(member_tree.join(".finitebrain/agent-state.json")).unwrap(),
    )
    .unwrap();
    let unsynced = state["brainDeleted"]["unsyncedPaths"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        unsynced.contains(&json!("Knowledge/wiki/alpha.md"))
            && unsynced.contains(&json!("Knowledge/wiki/new-idea.md")),
        "{state}"
    );
    assert_eq!(state["brainDeleted"]["deletedAt"], receipt["deletedAt"]);
    let watched = ok(
        &member,
        &member_tree,
        url,
        &["daemon", "watch", "--max-ticks", "5", "--poll-ms", "10"],
    );
    assert_eq!(watched["ticks"], 1, "{watched}");
    let status = ok(&member, &member_tree, url, &["status"]);
    assert_eq!(status["brainDeleted"]["deletedAt"], receipt["deletedAt"]);
    evidence.pass(
        "staleClientStoppedAndKeptUnsyncedWork",
        json!({"unsyncedPaths": unsynced}),
    );

    // The deletion record survives a restart; the stale client never needs
    // the server to stay stopped.
    let address = server.stop();
    let offline = refused(&member, &member_tree, url, &["sync", "now"]);
    assert!(
        offline.contains("was permanently deleted on the server"),
        "{offline}"
    );
    let server = Server::start(&database, Some(address));
    assert!(
        refused(&admin, &admin.dir, url, &["brain", "metadata", "v1"])
            .contains("Brain v1 was permanently deleted on the server")
    );
    assert!(
        refused(
            &outsider,
            &outsider.dir,
            url,
            &[
                "brain",
                "create",
                "v1",
                "--kind",
                "organization",
                "--name",
                "V1"
            ]
        )
        .contains("duplicate id for brain_id: v1")
    );
    evidence.pass("deletionRecordDurableAcrossRestart", json!(true));

    // Optional older-client proof: a pre-deletion fbrain neither uploads its
    // dirty tree nor recreates V1, and its local files stay.
    if let Some(binary) = &compat {
        fs::write(
            compat_tree.join("Knowledge/wiki/older.md"),
            "# Older client edit\n",
        )
        .unwrap();
        let synced = command_with(binary, &member.dir, &compat_tree, url)
            .args(["sync", "now", "--json"])
            .output()
            .unwrap();
        assert!(!synced.status.success(), "older client sync must fail");
        let created = command_with(binary, &member.dir, &member.dir, url)
            .args([
                "brain",
                "create",
                "v1",
                "--kind",
                "organization",
                "--name",
                "V1",
                "--json",
            ])
            .output()
            .unwrap();
        assert!(
            !created.status.success(),
            "older client must not recreate V1"
        );
        assert!(compat_tree.join("Knowledge/wiki/older.md").is_file());
        assert!(compat_tree.join("Knowledge/wiki/alpha.md").is_file());
        assert!(brain_rows(&database, "v1").is_empty());
        evidence.pass(
            "olderClientCannotResurrect",
            json!({"binary": binary.to_string_lossy(), "syncError": stderr(&synced).trim(), "createError": stderr(&created).trim()}),
        );
    }
    drop(server);

    // Operator restore drill: the pre-deletion backup onto an empty target,
    // the admin identity key restored into an empty home, and the pages
    // decrypt again. A focused Brain drill, not the hosted Recovery Set.
    let restored_database = root.join("restore-target/brain.sqlite3");
    copy_database(&backup, &restored_database);
    let restored = Server::start(&restored_database, None);
    let restored_admin = home(root, "restored-admin", ADMIN_SECRET);
    assert_eq!(restored_admin.npub, admin.npub);
    let restored_tree = restored_admin.dir.join("v1");
    ok(
        &restored_admin,
        &restored_admin.dir,
        &restored.url,
        &["open", "v1", restored_tree.to_str().unwrap()],
    );
    ok(
        &restored_admin,
        &restored_tree,
        &restored.url,
        &["sync", "now"],
    );
    assert_eq!(
        fs::read_to_string(restored_tree.join("Knowledge/wiki/alpha.md")).unwrap(),
        "# Alpha\n\nShared V1 page.\n"
    );
    assert_eq!(
        fs::read_to_string(restored_tree.join("Strategy/wiki/plan.md")).unwrap(),
        "# Plan\n\nEdited after preview.\n"
    );
    // The Member's deleted-Brain Working Tree stays stopped even against the
    // restored server: resuming is an explicit reopen, never automatic.
    assert!(
        refused(&member, &member_tree, &restored.url, &["sync", "now"])
            .contains("was permanently deleted on the server")
    );
    evidence.pass("restoreDrillToEmptyTarget", json!({"limits": "one Brain database and one admin identity key; not the hosted five-part Recovery Set"}));
}

/// Opening a Folder to all Members after a deletion preview changes no
/// Folder or page count but still invalidates the preview, and the refused
/// deletion writes nothing. Removals before and after the opening keep the
/// Folder Key from anyone who left. A fresh deletion is terminal: neither
/// opening nor an older client brings the Brain back.
#[test]
fn opening_a_folder_invalidates_a_deletion_preview_and_deletion_stays_terminal() {
    const FORMER_SECRET: &str = "00000000000000000000000000000000000000000000000000000000000000b1";
    const LEAVER_SECRET: &str = "00000000000000000000000000000000000000000000000000000000000000b2";
    let scratch = TempDir::new().unwrap();
    let root = scratch.path();
    let admin = home(root, "admin", ADMIN_SECRET);
    let member = home(root, "member", MEMBER_SECRET);
    let former = home(root, "former", FORMER_SECRET);
    let leaver = home(root, "leaver", LEAVER_SECRET);
    let database = root.join("state/brain.sqlite3");
    fs::create_dir_all(database.parent().unwrap()).unwrap();
    let server = Server::start(&database, None);
    let url = server.url.as_str();
    let query = |sql: &str| -> Vec<String> {
        let conn = rusqlite::Connection::open(&database).unwrap();
        let mut stmt = conn.prepare(sql).unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let grants = |key_version: u32| {
        query(&format!(
            "SELECT recipient_npub FROM folder_key_grants WHERE brain_id = 'v1'
             AND folder_id = 'strategy' AND key_version = {key_version} ORDER BY 1"
        ))
    };
    let deletion_records =
        || query("SELECT brain_id FROM deleted_brains WHERE brain_id = 'v1'").len();
    let sorted = |mut npubs: Vec<String>| {
        npubs.sort();
        npubs
    };

    ok(
        &admin,
        &admin.dir,
        url,
        &[
            "brain",
            "create",
            "v1",
            "--kind",
            "organization",
            "--name",
            "V1",
        ],
    );
    let admin_tree = admin.dir.join("v1");
    ok(
        &admin,
        &admin.dir,
        url,
        &["open", "v1", admin_tree.to_str().unwrap()],
    );
    ok(
        &admin,
        &admin_tree,
        url,
        &[
            "folder",
            "create",
            "strategy",
            "--access",
            "restricted",
            "--name",
            "Strategy",
            "--path",
            "Strategy",
        ],
    );
    for person in [&member, &former, &leaver] {
        ok(
            &admin,
            &admin_tree,
            url,
            &["admin", "member", "add", "--target", &person.npub],
        );
    }
    // Removed before the opening: it must never receive the Folder Key.
    ok(
        &admin,
        &admin_tree,
        url,
        &["admin", "member", "remove", "--target", &former.npub],
    );
    ok(&admin, &admin_tree, url, &["sync", "now"]);
    fs::write(admin_tree.join("Strategy/plan.md"), "# Plan\n").unwrap();
    ok(&admin, &admin_tree, url, &["sync", "now"]);
    let member_tree = member.dir.join("v1");
    ok(
        &member,
        &member.dir,
        url,
        &["open", "v1", member_tree.to_str().unwrap()],
    );
    ok(&member, &member_tree, url, &["sync", "now"]);
    assert!(!member_tree.join("Strategy/plan.md").exists());

    let preview = ok(&admin, &admin.dir, url, &["brain", "delete", "v1"]);
    assert_eq!(preview["state"], "confirmation-required", "{preview}");
    let stale_digest = preview["preview"]["stateDigest"]
        .as_str()
        .unwrap()
        .to_owned();

    let opened = ok(
        &admin,
        &admin_tree,
        url,
        &["admin", "folder-access", "open", "--folder", "strategy"],
    );
    assert_eq!(opened["outcome"], "opened", "{opened}");
    let granted = opened["grantedRecipients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|npub| npub.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        sorted(granted),
        sorted(vec![member.npub.clone(), leaver.npub.clone()])
    );

    // The preview confirmed before the opening no longer deletes, and the
    // refusal changes no row.
    let opened_rows = brain_rows(&database, "v1");
    let error = refused(
        &admin,
        &admin.dir,
        url,
        &["brain", "delete", "v1", "--confirm", &stale_digest],
    );
    assert!(
        error.contains("changed since the preview you confirmed"),
        "{error}"
    );
    assert_eq!(brain_rows(&database, "v1"), opened_rows);
    assert_eq!(deletion_records(), 0);
    // The already-open Member's routine sync reads the opened Folder.
    ok(&member, &member_tree, url, &["sync", "now"]);
    assert_eq!(
        fs::read_to_string(member_tree.join("Strategy/plan.md")).unwrap(),
        "# Plan\n"
    );

    // Removed after the opening: its new grant is cut off by re-keying.
    let removed = ok(
        &admin,
        &admin_tree,
        url,
        &["admin", "member", "remove", "--target", &leaver.npub],
    );
    let rotated = removed["rotatedFolders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|folder| folder["folderId"] == "strategy")
        .unwrap_or_else(|| panic!("strategy not re-keyed: {removed}"))
        .clone();
    assert_eq!(rotated["reason"], "readable", "{rotated}");
    assert_eq!(rotated["newKeyVersion"], 2, "{rotated}");
    assert_eq!(
        grants(2),
        sorted(vec![admin.npub.clone(), member.npub.clone()])
    );
    assert!(
        query(&format!(
            "SELECT folder_id FROM folder_key_grants WHERE brain_id = 'v1'
             AND recipient_npub = '{}'",
            former.npub
        ))
        .is_empty()
    );

    // Optional older client: a pre-opening fbrain reads the opened Folder.
    let compat = std::env::var_os("FBRAIN_COMPAT_BINARY").map(PathBuf::from);
    let compat_tree = member.dir.join("v1-older-client");
    if let Some(binary) = &compat {
        let opened = command_with(binary, &member.dir, &member.dir, url)
            .args(["open", "v1", compat_tree.to_str().unwrap(), "--json"])
            .output()
            .unwrap();
        assert!(opened.status.success(), "{}", stderr(&opened));
        assert_eq!(
            fs::read_to_string(compat_tree.join("Strategy/plan.md")).unwrap(),
            "# Plan\n"
        );
    }

    // Same Folder and page counts, a new digest, and a deletion that holds.
    let fresh = ok(&admin, &admin.dir, url, &["brain", "delete", "v1"]);
    for count in ["folderCount", "pageCount"] {
        assert_eq!(
            fresh["preview"][count], preview["preview"][count],
            "{count}"
        );
    }
    let digest = fresh["preview"]["stateDigest"].as_str().unwrap().to_owned();
    assert_ne!(digest, stale_digest);
    let deleted = ok(
        &admin,
        &admin.dir,
        url,
        &["brain", "delete", "v1", "--confirm", &digest],
    );
    assert_eq!(deleted["state"], "deleted", "{deleted}");
    assert!(brain_rows(&database, "v1").is_empty());
    assert_eq!(deletion_records(), 1);

    let error = refused(
        &admin,
        &admin.dir,
        url,
        &[
            "admin",
            "folder-access",
            "open",
            "--folder",
            "strategy",
            "--brain",
            "v1",
        ],
    );
    assert!(error.contains("permanently deleted"), "{error}");
    let error = refused(&member, &member_tree, url, &["sync", "now"]);
    assert!(error.contains("permanently deleted"), "{error}");
    if let Some(binary) = &compat {
        let synced = command_with(binary, &member.dir, &compat_tree, url)
            .args(["sync", "now", "--json"])
            .output()
            .unwrap();
        assert!(!synced.status.success(), "older client sync must fail");
        let created = command_with(binary, &member.dir, &member.dir, url)
            .args([
                "brain",
                "create",
                "v1",
                "--kind",
                "organization",
                "--name",
                "V1",
                "--json",
            ])
            .output()
            .unwrap();
        assert!(!created.status.success(), "older client recreated V1");
        assert!(compat_tree.join("Strategy/plan.md").is_file());
    }
    assert!(brain_rows(&database, "v1").is_empty());
    assert_eq!(deletion_records(), 1);
}

#[test]
fn deletion_on_an_overridden_server_does_not_stop_the_saved_working_tree() {
    let scratch = TempDir::new().unwrap();
    let admin = home(scratch.path(), "admin", ADMIN_SECRET);
    let saved = Server::start(&scratch.path().join("saved.sqlite3"), None);
    let other = Server::start(&scratch.path().join("other.sqlite3"), None);
    for server in [&saved, &other] {
        ok(
            &admin,
            &admin.dir,
            &server.url,
            &[
                "brain",
                "create",
                "v1",
                "--kind",
                "organization",
                "--name",
                "V1",
            ],
        );
    }
    let tree = admin.dir.join("v1");
    ok(
        &admin,
        &admin.dir,
        &saved.url,
        &["open", "v1", tree.to_str().unwrap()],
    );
    let tree_state = || -> Value {
        serde_json::from_slice(&fs::read(tree.join(".finitebrain/agent-state.json")).unwrap())
            .unwrap()
    };
    let preview = ok(&admin, &admin.dir, &other.url, &["brain", "delete", "v1"]);
    ok(
        &admin,
        &tree,
        &saved.url,
        &[
            "brain",
            "delete",
            "v1",
            "--confirm",
            preview["preview"]["stateDigest"].as_str().unwrap(),
            "--server",
            &other.url,
        ],
    );
    assert!(tree_state()["brainDeleted"].is_null(), "{}", tree_state());
    let error = refused(
        &admin,
        &tree,
        &saved.url,
        &["sync", "now", "--server", &other.url],
    );
    assert!(
        error.contains("was permanently deleted on the server"),
        "{error}"
    );
    assert!(tree_state()["brainDeleted"].is_null(), "{}", tree_state());
    // The same ID is still live on the saved server and remains usable.
    ok(&admin, &tree, &saved.url, &["sync", "now"]);
    let preview = ok(&admin, &tree, &saved.url, &["brain", "delete", "v1"]);
    ok(
        &admin,
        &tree,
        &saved.url,
        &[
            "brain",
            "delete",
            "v1",
            "--confirm",
            preview["preview"]["stateDigest"].as_str().unwrap(),
        ],
    );
    assert!(tree_state()["brainDeleted"]["deletedAt"].is_string());
}

#[test]
fn supervised_working_tree_stops_on_the_live_deletion_notice() {
    let scratch = TempDir::new().unwrap();
    let root = scratch.path();
    let admin = home(root, "admin", ADMIN_SECRET);
    let member = home(root, "member", MEMBER_SECRET);
    let database = root.join("state/brain.sqlite3");
    fs::create_dir_all(database.parent().unwrap()).unwrap();
    let server = Server::start(&database, None);
    let url = server.url.as_str();
    ok(
        &admin,
        &admin.dir,
        url,
        &[
            "brain",
            "create",
            "v1",
            "--kind",
            "organization",
            "--name",
            "V1",
        ],
    );
    let admin_tree = admin.dir.join("v1");
    ok(
        &admin,
        &admin.dir,
        url,
        &["open", "v1", admin_tree.to_str().unwrap()],
    );
    ok(
        &admin,
        &admin_tree,
        url,
        &["folder", "create", "Knowledge", "--access", "all_members"],
    );
    ok(
        &admin,
        &admin_tree,
        url,
        &["admin", "member", "add", "--target", &member.npub],
    );
    ok(&admin, &admin_tree, url, &["sync", "now"]);
    fs::write(admin_tree.join("Knowledge/wiki/alpha.md"), "# Alpha\n").unwrap();
    ok(&admin, &admin_tree, url, &["sync", "now"]);

    let supervised = root.join("supervised");
    fs::create_dir_all(&supervised).unwrap();
    let member_tree = supervised.join("v1");
    ok(
        &member,
        &member.dir,
        url,
        &["open", "v1", member_tree.to_str().unwrap()],
    );
    let log_path = root.join("supervisor.log");
    let log = fs::File::create(&log_path).unwrap();
    let supervisor = command(&member.dir, &member.dir, url)
        .args([
            "daemon",
            "supervise",
            "--working-tree-root",
            supervised.to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::from(log.try_clone().unwrap()))
        .stderr(std::process::Stdio::from(log))
        .spawn()
        .unwrap();
    let supervisor = ChildGuard(supervisor);
    let agent_state = || -> Value {
        fs::read(member_tree.join(".finitebrain/agent-state.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null)
    };
    wait_until("the supervisor's first catch-up", &log_path, || {
        member_tree.join("Knowledge/wiki/alpha.md").is_file()
            && agent_state()["daemon"]["tickCount"].as_u64().unwrap_or(0) > 0
    });
    // The supervisor is idle now; nothing changes V1 between the admin's
    // preview and confirmation.
    let preview = ok(&admin, &admin.dir, url, &["brain", "delete", "v1"]);
    let digest = preview["preview"]["stateDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    let deleted = ok(
        &admin,
        &admin.dir,
        url,
        &["brain", "delete", "v1", "--confirm", &digest],
    );
    let deleted_at = deleted["receipt"]["deletedAt"].clone();

    // The live notice reaches the former Member's supervisor, which records
    // the terminal state without polling Brain contents.
    wait_until("the terminal deleted-Brain state", &log_path, || {
        agent_state()["brainDeleted"]["deletedAt"] == deleted_at
    });
    let blocked_lines = || {
        fs::read_to_string(&log_path)
            .unwrap_or_default()
            .matches("was permanently deleted on the server")
            .count()
    };
    let blocked_activity = || {
        agent_state()["activity"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| entry["kind"] == "daemon.sync_blocked")
                    .count()
            })
            .unwrap_or(0)
    };
    // Later local edits never reach the server: the worker skips the
    // deleted Brain, so the one blocked sync stays the only one.
    wait_until("the blocked sync outcome", &log_path, || {
        blocked_activity() == 1
    });
    let tick_count = || agent_state()["daemon"]["tickCount"].as_u64().unwrap_or(0);
    let before_first_edit = tick_count();
    fs::write(
        member_tree.join("Knowledge/wiki/after.md"),
        "# After deletion\n",
    )
    .unwrap();
    wait_until(
        "the worker observing the first post-deletion edit",
        &log_path,
        || tick_count() > before_first_edit,
    );
    let before_second_edit = tick_count();
    fs::write(
        member_tree.join("Knowledge/wiki/after.md"),
        "# After deletion, again\n",
    )
    .unwrap();
    wait_until(
        "the worker observing the second post-deletion edit",
        &log_path,
        || tick_count() > before_second_edit,
    );
    assert_eq!(blocked_activity(), 1, "{}", agent_state());
    assert!(blocked_lines() <= 1);
    assert!(member_tree.join("Knowledge/wiki/alpha.md").is_file());
    assert!(member_tree.join("Knowledge/wiki/after.md").is_file());
    // The supervisor holds the notification stream open; stop it first so
    // the server's graceful shutdown can finish.
    drop(supervisor);
    drop(server);
}
