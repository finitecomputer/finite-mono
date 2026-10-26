//! Organization Brain deletion end to end: the built `fbrain` against a real
//! file-backed Brain server, with synthetic stand-ins for V1, content-brain
//! and marketing-comms-brain. Set `FBRAIN_BRAIN_DELETION_REPORT` to a path to
//! keep a JSON evidence report of the boundaries that passed.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;

use serde_json::{Value, json};
use tempfile::TempDir;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const ADMIN_SECRET: &str = "00000000000000000000000000000000000000000000000000000000000000a1";
const AGENT_SECRET: &str = "00000000000000000000000000000000000000000000000000000000000000a2";
const MEMBER_SECRET: &str = "00000000000000000000000000000000000000000000000000000000000000a3";
const INVITEE_SECRET: &str = "00000000000000000000000000000000000000000000000000000000000000a4";
const OUTSIDER_SECRET: &str = "00000000000000000000000000000000000000000000000000000000000000a5";

/// A real Brain server on a file-backed SQLite store, restartable on the
/// same address so clients keep their configured server.
struct Server {
    url: String,
    address: SocketAddr,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(database: &Path, address: Option<SocketAddr>) -> Self {
        let database = database.to_path_buf();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let bind = address.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
                let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
                let address = listener.local_addr().unwrap();
                let url = format!("http://{address}");
                let store = finite_brain_store::BrainStore::open(&database).unwrap();
                let state = finite_brain_server::ServerState::new(store, url.clone());
                ready_tx.send((url, address)).unwrap();
                axum::serve(listener, finite_brain_server::router_with_state(state))
                    .with_graceful_shutdown(async move {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .unwrap();
            });
        });
        let (url, address) = ready_rx.recv().unwrap();
        Self {
            url,
            address,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        }
    }

    fn stop(mut self) -> SocketAddr {
        self.halt();
        self.address
    }

    fn halt(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.halt();
    }
}

struct Home {
    dir: PathBuf,
    npub: String,
}

fn home(scratch: &Path, name: &str, secret: &str) -> Home {
    let dir = scratch.join(name);
    fs::create_dir_all(&dir).unwrap();
    let secret_file = scratch.join(format!("{name}.secret"));
    fs::write(&secret_file, format!("{secret}\n")).unwrap();
    let imported = command(&dir, &dir, "http://127.0.0.1:9")
        .args(["auth", "import", "--file", secret_file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(imported.status.success(), "{}", stderr(&imported));
    fs::remove_file(secret_file).unwrap();
    let signer = command(&dir, &dir, "http://127.0.0.1:9")
        .args(["signer", "public-key", "--json"])
        .output()
        .unwrap();
    let npub = serde_json::from_slice::<Value>(&signer.stdout).unwrap()["npub"]
        .as_str()
        .unwrap()
        .to_owned();
    Home { dir, npub }
}

fn command(home: &Path, cwd: &Path, server: &str) -> Command {
    command_with(Path::new(env!("CARGO_BIN_EXE_fbrain")), home, cwd, server)
}

fn command_with(binary: &Path, home: &Path, cwd: &Path, server: &str) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(cwd)
        .env_clear()
        .env("HOME", home)
        .env("FINITE_HOME", home.join("finite-home"))
        .env("FBRAIN_CONFIG_DIR", home.join("fbrain-config"))
        .env(
            "FBRAIN_NOW",
            OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
        )
        .env("FINITE_BRAIN_SERVER_URL", server)
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", server);
    command
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Run a command that must succeed; returns its `--json` output.
fn ok(home: &Home, cwd: &Path, server: &str, args: &[&str]) -> Value {
    let output = command(&home.dir, cwd, server)
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}: {}", stderr(&output));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Run a command that must fail; returns its error text.
fn refused(home: &Home, cwd: &Path, server: &str, args: &[&str]) -> String {
    let output = command(&home.dir, cwd, server).args(args).output().unwrap();
    assert!(
        !output.status.success(),
        "{args:?} unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    stderr(&output)
}

/// Every row in every table that names `brain_id`, outside the permanent
/// deletion record, as sorted text.
fn brain_rows(database: &Path, brain_id: &str) -> Vec<String> {
    let conn = rusqlite::Connection::open(database).unwrap();
    let tables = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut rows = Vec::new();
    for table in tables {
        if table == "deleted_brains" || table == "deleted_brain_audience" {
            continue;
        }
        let columns = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let keyed = columns
            .iter()
            .filter(|column| {
                matches!(
                    column.as_str(),
                    "brain_id" | "source_brain_id" | "destination_brain_id" | "vault_id"
                ) || (table == "brains" && column.as_str() == "id")
            })
            .map(|column| format!("\"{column}\" = ?1"))
            .collect::<Vec<_>>();
        if keyed.is_empty() {
            continue;
        }
        let mut stmt = conn
            .prepare(&format!(
                "SELECT * FROM \"{table}\" WHERE {}",
                keyed.join(" OR ")
            ))
            .unwrap();
        let width = stmt.column_count();
        let mut found = stmt.query([brain_id]).unwrap();
        while let Some(row) = found.next().unwrap() {
            let values = (0..width)
                .map(|index| match row.get_ref(index).unwrap() {
                    rusqlite::types::ValueRef::Text(text) => {
                        String::from_utf8_lossy(text).into_owned()
                    }
                    other => format!("{other:?}"),
                })
                .collect::<Vec<_>>();
            rows.push(format!("{table}|{}", values.join("|")));
        }
    }
    rows.sort();
    rows
}

/// Another Brain keeps every row except the revoked Mount records that named
/// V1 (removed so no stale offer can restore access) and its derived
/// capacity counters. Returns how many retired records were removed.
fn assert_only_retired_mount_records_changed(before: &[String], after: &[String]) -> usize {
    let retired = |row: &String| {
        (row.starts_with("folder_mounts|") || row.starts_with("shared_folder_"))
            && row.split('|').any(|field| field == "v1")
    };
    let derived = |row: &String| row.starts_with("brain_capacity_counts|");
    let kept = |rows: &[String]| {
        rows.iter()
            .filter(|row| !retired(row) && !derived(row))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(kept(after), kept(before));
    assert!(!after.iter().any(retired), "{after:#?}");
    before.iter().filter(|row| retired(row)).count()
}

fn copy_database(from: &Path, to: &Path) {
    fs::create_dir_all(to.parent().unwrap()).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let source = PathBuf::from(format!("{}{suffix}", from.display()));
        if source.exists() {
            fs::copy(&source, format!("{}{suffix}", to.display())).unwrap();
        }
    }
}

/// Boundaries that passed, written as JSON when the report path is set.
struct Evidence {
    path: Option<PathBuf>,
    facts: serde_json::Map<String, Value>,
}

impl Evidence {
    fn pass(&mut self, boundary: &str, detail: Value) {
        self.facts.insert(boundary.to_owned(), detail);
    }
}

impl Drop for Evidence {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let report = json!({
                "format": "finite.brain.organization-brain-deletion-acceptance.v1",
                "status": if thread::panicking() { "failed" } else { "passed" },
                "passedBoundaries": self.facts,
                "syntheticOnly": true,
            });
            fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
    }
}

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
        refused(&member, &member.dir, url, &["brain", "metadata", "v1"]).contains("brain_deleted")
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
        .contains("brain_deleted")
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
        refused(&admin, &admin.dir, url, &["brain", "metadata", "v1"]).contains("brain_deleted")
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

struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_until(what: &str, log: &Path, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}: {}",
            fs::read_to_string(log).unwrap_or_default()
        );
        thread::sleep(std::time::Duration::from_millis(100));
    }
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
    fs::write(
        member_tree.join("Knowledge/wiki/draft.md"),
        "# Draft never synced\n",
    )
    .unwrap();
    // Let the supervisor push nothing yet: the draft is written after its
    // catch-up and deletion follows at once, so it may or may not be pushed.
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
    fs::write(
        member_tree.join("Knowledge/wiki/after.md"),
        "# After deletion\n",
    )
    .unwrap();
    thread::sleep(std::time::Duration::from_secs(2));
    fs::write(
        member_tree.join("Knowledge/wiki/after.md"),
        "# After deletion, again\n",
    )
    .unwrap();
    thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(blocked_activity(), 1, "{}", agent_state());
    assert!(blocked_lines() <= 1);
    assert!(member_tree.join("Knowledge/wiki/alpha.md").is_file());
    assert!(member_tree.join("Knowledge/wiki/after.md").is_file());
    assert!(member_tree.join("Knowledge/wiki/draft.md").is_file());
    // The supervisor holds the notification stream open; stop it first so
    // the server's graceful shutdown can finish.
    drop(supervisor);
    drop(server);
}
