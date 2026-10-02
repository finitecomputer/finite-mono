use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::process::{Child, Command, Output};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use finite_brain_core::{
    BrainId, FolderId, FolderKey, FolderKeyGrantPayload, FolderObjectAad, ObjectId,
    encrypt_folder_object,
};
use finite_nostr::{NostrPublicKey, build_rumor, wrap_rumor};
use nostr::{Keys, Kind};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

struct CollaborationSmokeReport {
    path: Option<PathBuf>,
    current_boundary: &'static str,
    passed_boundaries: Vec<&'static str>,
    completed: bool,
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl CollaborationSmokeReport {
    fn from_environment() -> Self {
        Self {
            path: std::env::var_os("FINITE_BRAIN_COLLABORATION_SMOKE_REPORT").map(PathBuf::from),
            current_boundary: "fixtureSetup",
            passed_boundaries: Vec::new(),
            completed: false,
        }
    }

    fn enter(&mut self, boundary: &'static str) {
        self.current_boundary = boundary;
    }

    fn pass(&mut self) {
        self.passed_boundaries.push(self.current_boundary);
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for CollaborationSmokeReport {
    fn drop(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let report = json!({
            "format": "finite.brain.organization-collaboration-smoke.v1",
            "status": if self.completed { "passed" } else { "failed" },
            "failedBoundary": if self.completed {
                Value::Null
            } else {
                Value::String(self.current_boundary.to_owned())
            },
            "passedBoundaries": self.passed_boundaries,
            "facts": {
                "collaborationState": if self.passed_boundaries.contains(&"npubCollaboration") {
                    Value::String("complete".to_owned())
                } else {
                    Value::Null
                },
                "independentFiniteHomes": self.passed_boundaries.contains(&"fixtureSetup"),
                "targetForm": "npub",
                "emailInvitationsRetired": self
                    .passed_boundaries
                    .contains(&"npubFolderGuestInvitation"),
                "existingRestrictedKnowledge": self
                    .passed_boundaries
                    .contains(&"restrictedKnowledgeBeforeCollaboration"),
                "recipientRead": self.passed_boundaries.contains(&"betaOpenAndRead"),
                "recipientEditAndSync": self.passed_boundaries.contains(&"betaEditAndSync"),
                "inviterObservedRecipientEdit": self
                    .passed_boundaries
                    .contains(&"alphaSyncAndObserve"),
                "recordsCredentialsKeysGrantPlaintextCommandsOrToolOutput": false
            }
        });
        let _ = fs::write(path, serde_json::to_vec_pretty(&report).unwrap());
    }
}

fn spawn_real_brain_server(
    target_npub: &str,
    personal_agent_npub: &str,
    owner_npub: &str,
    requester_npub: &str,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    thread::JoinHandle<()>,
) {
    // The Brain server consults no account authority and fetches NIP-05 over
    // the public internet (auth kernel cut); tests name npubs directly.
    let _ = (target_npub, requester_npub);
    let (url_tx, url_rx) = mpsc::channel();
    let personal_agent_npub = personal_agent_npub.to_owned();
    let owner_npub = owner_npub.to_owned();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let thread = thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let mut store = finite_brain_store::BrainStore::open_in_memory().unwrap();
            let personal = finite_brain_core::bootstrap_personal_brain(
                "personal-a",
                "Personal A",
                &owner_npub,
            )
            .unwrap();
            store
                .create_personal_brain_bootstrap(
                    &personal,
                    &[],
                    &finite_brain_core::UserId::new(personal_agent_npub).unwrap(),
                    &finite_brain_core::UserId::new(owner_npub).unwrap(),
                    &OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
                )
                .unwrap();
            let state = finite_brain_server::ServerState::new(store, url.clone())
                .with_dev_invite_mailer()
                .with_auth_clock(OffsetDateTime::now_utc().unix_timestamp() as u64, 300);
            let router = finite_brain_server::router_with_state(state);
            url_tx.send(url).unwrap();
            tokio::select! {
                result = axum::serve(listener, router) => result.unwrap(),
                _ = shutdown_rx => {}
            }
        });
    });
    (url_rx.recv().unwrap(), shutdown_tx, thread)
}

/// Authorities for the CLI invite/approval roundtrip: Bob's account resolves
/// to his human Principal plus one active managed agent; the agent npub is the
/// CLI home the test drives.
fn spawn_file_backed_brain_server(
    owner_npub: &str,
    database_path: std::path::PathBuf,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    thread::JoinHandle<()>,
) {
    spawn_clocked_file_backed_brain_server(owner_npub, database_path, None)
}

/// Like `spawn_file_backed_brain_server`, optionally pinning the server clock
/// (with a 300 second auth skew) so a test controls client and server time.
fn spawn_clocked_file_backed_brain_server(
    owner_npub: &str,
    database_path: std::path::PathBuf,
    server_now_unix: Option<u64>,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    thread::JoinHandle<()>,
) {
    let (url_tx, url_rx) = mpsc::channel();
    let owner_npub = owner_npub.to_owned();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let thread = thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let mut store = finite_brain_store::BrainStore::open(&database_path).unwrap();
            let organization = finite_brain_core::bootstrap_organization_brain(
                "roundtrip-org",
                "Roundtrip Org",
                &owner_npub,
            )
            .unwrap();
            let brain_exists = store
                .load_brain(&finite_brain_core::BrainId::new("roundtrip-org").unwrap())
                .is_ok();
            if !brain_exists {
                store.create_brain_bootstrap(&organization, &[]).unwrap();
            }
            let mut state = finite_brain_server::ServerState::new(store, url.clone());
            if let Some(now) = server_now_unix {
                state = state.with_auth_clock(now, 300);
            }
            url_tx.send(url).unwrap();
            let router = finite_brain_server::router_with_state(state);
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
        });
    });
    let url = url_rx.recv().unwrap();
    (url, shutdown_tx, thread)
}

fn spawn_brain_updates_404_proxy(
    upstream_url: &str,
) -> (
    String,
    Arc<AtomicUsize>,
    Arc<AtomicBool>,
    thread::JoinHandle<()>,
) {
    let upstream = upstream_url
        .strip_prefix("http://")
        .expect("test Brain server uses HTTP")
        .to_owned();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let notification_requests = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let request_counter = Arc::clone(&notification_requests);
    let thread_stop = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        while !thread_stop.load(Ordering::SeqCst) {
            let (mut client, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("old-server proxy accept failed: {error}"),
            };
            let upstream = upstream.clone();
            let request_counter = Arc::clone(&request_counter);
            thread::spawn(move || {
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 8192];
                    let bytes = client.read(&mut chunk).unwrap_or(0);
                    if bytes == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..bytes]);
                    let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
                let request_line = String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                if request_line.contains(" /v1/brain-updates ") {
                    request_counter.fetch_add(1, Ordering::SeqCst);
                    let body = r#"{"error":"not_found"}"#;
                    write!(
                        client,
                        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    return;
                }
                let header_end = request
                    .windows(4)
                    .position(|part| part == b"\r\n\r\n")
                    .unwrap();
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let mut forwarded_request = headers
                    .lines()
                    .filter(|line| !line.to_ascii_lowercase().starts_with("connection:"))
                    .collect::<Vec<_>>()
                    .join("\r\n")
                    .into_bytes();
                forwarded_request.extend_from_slice(b"\r\nConnection: close\r\n\r\n");
                forwarded_request.extend_from_slice(&request[header_end + 4..]);
                let mut server = TcpStream::connect(&upstream).unwrap();
                server.write_all(&forwarded_request).unwrap();
                let _ = std::io::copy(&mut server, &mut client);
            });
        }
    });
    (url, notification_requests, stop, handle)
}

fn fbrain() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fbrain"))
}

#[test]
fn built_fbrain_rename_preserves_an_open_tree_and_survives_server_restart() {
    let scratch = TempDir::new().unwrap();
    let home = scratch.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let signer = run(&home, &home, &["signer", "public-key", "--json"]);
    assert!(
        signer.status.success(),
        "{}",
        String::from_utf8_lossy(&signer.stderr)
    );
    let signer: Value = serde_json::from_slice(&signer.stdout).unwrap();
    let owner = signer["npub"].as_str().unwrap();
    let db = scratch.path().join("brain.sqlite3");
    let (server, shutdown, thread) = spawn_file_backed_brain_server(owner, db.clone());
    let tree = home.join("unchanged-tree-path");
    let execute = |cwd: &Path, args: &[&str], server: &str| {
        let result = command(&home, cwd)
            .env("FINITE_BRAIN_SERVER_URL", server)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice::<Value>(&result.stdout).unwrap()
    };
    execute(
        &home,
        &["open", "roundtrip-org", tree.to_str().unwrap(), "--json"],
        &server,
    );
    execute(&tree, &["folder", "create", "Knowledge", "--json"], &server);
    execute(&tree, &["sync", "now", "--json"], &server);
    let page = tree.join("Knowledge/wiki/keep.md");
    let contents = "# Existing knowledge\n\nKeep this content across rename.\n";
    fs::write(&page, contents).unwrap();
    execute(&tree, &["sync", "now", "--json"], &server);
    let prior_agent_state: Value =
        serde_json::from_slice(&fs::read(tree.join(".finitebrain/agent-state.json")).unwrap())
            .unwrap();
    let renamed = execute(
        &home,
        &[
            "brain",
            "rename",
            "Renamed Knowledge",
            "--brain",
            "roundtrip-org",
            "--json",
        ],
        &server,
    );
    assert_eq!(renamed["brainId"], "roundtrip-org");
    assert_eq!(renamed["name"], "Renamed Knowledge");

    // Optional pre-change client proof: the caller supplies a built old CLI.
    // It reads existing content and catches up through its original sync path.
    if let Some(binary) = std::env::var_os("FBRAIN_COMPAT_BINARY") {
        let old = Command::new(binary)
            .current_dir(&tree)
            .env_clear()
            .env("HOME", &home)
            .env("FINITE_HOME", home.join("finite-home"))
            .env("FBRAIN_CONFIG_DIR", home.join("fbrain-config"))
            .env("FINITE_BRAIN_SERVER_URL", &server)
            .args(["sync", "now", "--json"])
            .output()
            .unwrap();
        assert!(
            old.status.success(),
            "{}",
            String::from_utf8_lossy(&old.stderr)
        );
        let export: Value = serde_json::from_slice(
            &fs::read(tree.join(".finitebrain/encrypted-sync/export.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(export["brain"]["name"], "Renamed Knowledge");
        assert_eq!(fs::read_to_string(&page).unwrap(), contents);
    }

    execute(&tree, &["sync", "now", "--json"], &server);
    let export: Value = serde_json::from_slice(
        &fs::read(tree.join(".finitebrain/encrypted-sync/export.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(export["brain"]["name"], "Renamed Knowledge");
    let directory: Value =
        serde_json::from_slice(&fs::read(tree.join(".finitebrain/brain-directory.json")).unwrap())
            .unwrap();
    assert_eq!(directory["brain"]["name"], "Renamed Knowledge");
    assert_eq!(directory["brain"]["id"], "roundtrip-org");
    assert_eq!(fs::read_to_string(&page).unwrap(), contents);
    let agent_state: Value =
        serde_json::from_slice(&fs::read(tree.join(".finitebrain/agent-state.json")).unwrap())
            .unwrap();
    assert_eq!(agent_state["brainId"], prior_agent_state["brainId"]);
    // A second rename from the open tree exercises the remembered Brain target.
    execute(&tree, &["brain", "rename", "Final name", "--json"], &server);
    // Distinct commands may deliberately restore earlier names, even with
    // the fixed FBRAIN_NOW used by this process fixture.
    let restored = execute(
        &tree,
        &["brain", "rename", "Renamed Knowledge", "--json"],
        &server,
    );
    assert_eq!(restored["name"], "Renamed Knowledge");
    let final_name = execute(&tree, &["brain", "rename", "Final name", "--json"], &server);
    assert_eq!(final_name["name"], "Final name");
    execute(&tree, &["sync", "now", "--json"], &server);
    shutdown.send(()).unwrap();
    thread.join().unwrap();
    let (server, shutdown, thread) = spawn_file_backed_brain_server(owner, db.clone());
    let metadata = execute(
        &home,
        &["brain", "metadata", "roundtrip-org", "--json"],
        &server,
    );
    assert_eq!(metadata["name"], "Final name");
    assert_eq!(metadata["brainId"], "roundtrip-org");
    assert_eq!(fs::read_to_string(&page).unwrap(), contents);
    shutdown.send(()).unwrap();
    thread.join().unwrap();

    if let Some(binary) = std::env::var_os("FBRAIN_COMPAT_SERVER_BINARY") {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = reservation.local_addr().unwrap();
        drop(reservation);
        let server = format!("http://{addr}");
        let _old_server = ChildGuard(
            Command::new(binary)
                .env_clear()
                .env("FINITE_BRAIN_ADDR", addr.to_string())
                .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server)
                .env("FINITE_BRAIN_DB", &db)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect(addr).is_err() {
            assert!(Instant::now() < deadline, "old server did not start");
            thread::sleep(Duration::from_millis(20));
        }
        let metadata = execute(
            &home,
            &["brain", "metadata", "roundtrip-org", "--json"],
            &server,
        );
        assert_eq!(metadata["name"], "Final name");
        let rejected = command(&home, &home)
            .env("FINITE_BRAIN_SERVER_URL", &server)
            .args([
                "brain",
                "rename",
                "Must not apply",
                "--brain",
                "roundtrip-org",
                "--json",
            ])
            .output()
            .unwrap();
        assert!(!rejected.status.success());
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("404"));
        let metadata = execute(
            &home,
            &["brain", "metadata", "roundtrip-org", "--json"],
            &server,
        );
        assert_eq!(metadata["name"], "Final name");
        execute(
            &tree,
            &["sync", "now", "--server", &server, "--json"],
            &server,
        );
        assert_eq!(fs::read_to_string(&page).unwrap(), contents);
    }
}

fn command(home: &Path, cwd: &Path) -> Command {
    command_for(&fbrain(), home, cwd)
}

fn command_for(binary: &Path, home: &Path, cwd: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(cwd)
        .env_clear()
        .env("HOME", home)
        .env("FINITE_HOME", home.join("finite-home"))
        .env("FBRAIN_CONFIG_DIR", home.join("fbrain-config"))
        .env("FBRAIN_NOW", "2026-07-22T18:00:00Z");
    command
}

fn run(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    command(home, cwd).args(args).output().unwrap()
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn write_requester_context(finite_home: &Path, session_key: &str, requesting_user_id: &str) {
    let digest = Sha256::digest(session_key.as_bytes());
    let path = finite_home
        .join("requester-context-v1")
        .join(format!("{digest:x}.json"));
    let expires_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    write_json(
        &path,
        &json!({
            "version": 1,
            "session_key": session_key,
            "platform": "finitechat",
            "requesting_user_id": requesting_user_id,
            "expires_at_unix": expires_at_unix,
        }),
    );
}

fn assert_canonical_folder_projection(folder_root: &Path, folder_id: &str) -> String {
    let instructions = fs::read_to_string(folder_root.join("AGENTS.md")).unwrap();
    assert_eq!(
        instructions,
        format!(
            "# Folder Agent Instructions\n\nFolder id: `{folder_id}`\n\nUse `raw/` for Markdown source captures and Asset Source Notes, `wiki/` for durable synthesized pages, `inventory/` for source candidates and open questions, `datasets/` for manifests and query recipes, and `output/` for generated artifacts. Keep non-Markdown Asset bytes outside the Brain. Represent each Asset with one Markdown Source Note under `raw/` whose frontmatter includes `type`, `title`, and its canonical `resource` URI, then cite that note from synthesized work.\n"
        )
    );
    for marker in [
        "raw/.keep",
        "wiki/.keep",
        "inventory/.keep",
        "datasets/.keep",
        "output/.keep",
    ] {
        assert!(
            folder_root.join(marker).is_file(),
            "canonical marker {marker} missing from {}",
            folder_root.display()
        );
    }
    assert!(!folder_root.join("raw/assets").exists());
    assert!(!folder_root.join("compiled/.keep").exists());
    instructions
}

fn setup_tree(scratch: &TempDir) -> PathBuf {
    let secret = scratch.path().join("identity-secret");
    fs::write(
        &secret,
        "0000000000000000000000000000000000000000000000000000000000000001\n",
    )
    .unwrap();
    let imported = run(
        scratch.path(),
        scratch.path(),
        &[
            "auth",
            "import",
            "--file",
            secret.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );

    let tree = scratch.path().join("brain");
    let opened = run(
        scratch.path(),
        scratch.path(),
        &["open", "brain", tree.to_str().unwrap(), "--json"],
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    fs::create_dir_all(tree.join("General/nested")).unwrap();
    fs::create_dir_all(tree.join("Research")).unwrap();
    fs::create_dir_all(tree.join("Locked")).unwrap();
    fs::write(
        tree.join("General/nested/strong-a.md"),
        "# Cobalt cobalt cobalt\n\nCobalt cobalt cobalt cobalt durable evidence.\n",
    )
    .unwrap();
    fs::write(
        tree.join("General/strong-b.md"),
        "# Cobalt analysis\n\nCobalt cobalt cobalt repeated evidence.\n",
    )
    .unwrap();
    fs::write(
        tree.join("Research/weak.md"),
        "# Notes\n\nOne passing cobalt reference.\n",
    )
    .unwrap();
    fs::write(
        tree.join("Locked/hidden.md"),
        "# Secret\n\nuniquelockedterm must never be indexed.\n",
    )
    .unwrap();
    fs::write(
        tree.join("General/removed.md"),
        "# Removed\n\ntransientremoved evidence.\n",
    )
    .unwrap();
    let synced = fs::read(tree.join("General/nested/strong-a.md")).unwrap();
    let synced_hash = format!("{:x}", Sha256::digest(&synced));
    write_json(
        &tree.join(".finitebrain/working-tree-state.json"),
        &json!({
            "version": "finite-brain-working-tree-state-v1",
            "folderRoots": [
                {
                    "folderId": "general",
                    "sourceBrainId": null,
                    "path": "General",
                    "canRead": true,
                    "metadataOnly": false
                },
                {
                    "folderId": "research",
                    "sourceBrainId": null,
                    "path": "Research",
                    "canRead": true,
                    "metadataOnly": false
                },
                {
                    "folderId": "locked",
                    "sourceBrainId": null,
                    "path": "Locked",
                    "canRead": false,
                    "metadataOnly": true
                }
            ],
            "objects": [{
                "folderId": "general",
                "sourceBrainId": null,
                "path": "nested/strong-a.md",
                "objectId": "obj_synced_process_1",
                "revision": 1,
                "keyVersion": 1,
                "contentType": "text/markdown",
                "contentHash": synced_hash
            }],
            "sync": { "latestSequence": 0 }
        }),
    );
    let agent_state_path = tree.join(".finitebrain/agent-state.json");
    let mut agent_state: Value =
        serde_json::from_slice(&fs::read(&agent_state_path).unwrap()).unwrap();
    agent_state["conflicts"] = json!([{
        "id": "conflict-process-1",
        "folderId": "general",
        "path": "strong-b.md",
        "reason": "process acceptance conflict",
        "state": "open",
        "createdAt": "2026-07-22T18:00:00Z",
        "resolvedAt": null
    }]);
    write_json(&agent_state_path, &agent_state);
    tree
}

fn setup_access_loss_tree(scratch: &TempDir) -> PathBuf {
    let tree = setup_tree(scratch);
    // Leave one clean readable Folder so `sync now` reaches the remote access
    // transition without first attempting to upload unrelated local edits.
    fs::remove_file(tree.join("General/strong-b.md")).unwrap();
    fs::remove_file(tree.join("General/removed.md")).unwrap();
    fs::remove_dir_all(tree.join("Research")).unwrap();
    // Cache an empty bootstrap at the cursor so these scenarios exercise the
    // incremental records path; without it sync bootstraps directly.
    let bootstrap_path = tree.join(".finitebrain/encrypted-sync/bootstrap.json");
    write_json(
        &bootstrap_path,
        &json!({ "latestSequence": 0, "objects": [] }),
    );
    #[cfg(unix)]
    fs::set_permissions(&bootstrap_path, fs::Permissions::from_mode(0o600)).unwrap();
    let state_path = tree.join(".finitebrain/working-tree-state.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    state["folderRoots"]
        .as_array_mut()
        .unwrap()
        .retain(|folder| matches!(folder["folderId"].as_str(), Some("general" | "locked")));
    write_json(&state_path, &state);
    let agent_path = tree.join(".finitebrain/agent-state.json");
    let mut agent: Value = serde_json::from_slice(&fs::read(&agent_path).unwrap()).unwrap();
    agent["conflicts"] = json!([]);
    write_json(&agent_path, &agent);
    tree
}

#[test]
fn supervisor_keeps_local_sync_when_old_server_has_no_notification_route() {
    let scratch = TempDir::new().unwrap();
    let owner_home = scratch.path().join("owner-home");
    let agent_home = scratch.path().join("agent-home");
    fs::create_dir_all(&owner_home).unwrap();
    fs::create_dir_all(&agent_home).unwrap();
    let owner_secret = scratch.path().join("owner-secret");
    let agent_secret = scratch.path().join("agent-secret");
    fs::write(
        &owner_secret,
        "0000000000000000000000000000000000000000000000000000000000000001\n",
    )
    .unwrap();
    fs::write(
        &agent_secret,
        "0000000000000000000000000000000000000000000000000000000000000003\n",
    )
    .unwrap();
    for (home, secret) in [(&owner_home, &owner_secret), (&agent_home, &agent_secret)] {
        let imported = run(
            home,
            home,
            &[
                "auth",
                "import",
                "--file",
                secret.to_str().unwrap(),
                "--json",
            ],
        );
        assert!(
            imported.status.success(),
            "{}",
            String::from_utf8_lossy(&imported.stderr)
        );
    }
    let public_key = |home: &Path| {
        let output = run(home, home, &["signer", "public-key", "--json"]);
        assert!(output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        value["npub"].as_str().unwrap().to_owned()
    };
    let owner_npub = public_key(&owner_home);
    let agent_npub = public_key(&agent_home);
    let (server_url, server_shutdown, server_thread) =
        spawn_real_brain_server(&agent_npub, &agent_npub, &owner_npub, &owner_npub);
    let (proxy_url, notification_requests, proxy_stop, proxy_thread) =
        spawn_brain_updates_404_proxy(&server_url);

    let working_tree_root = scratch.path().join("supervised-trees");
    fs::create_dir_all(&working_tree_root).unwrap();
    let supervisor_log_path = scratch.path().join("supervisor.log");
    let supervisor_log = fs::File::create(&supervisor_log_path).unwrap();
    let supervisor = command(&agent_home, &agent_home)
        .env("FINITE_BRAIN_SERVER_URL", &proxy_url)
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
        .env("FBRAIN_WORKING_TREE_ROOT", &working_tree_root)
        .args(["daemon", "supervise"])
        .stdout(Stdio::from(supervisor_log.try_clone().unwrap()))
        .stderr(Stdio::from(supervisor_log))
        .spawn()
        .unwrap();
    let mut supervisor = ChildGuard(supervisor);
    let deadline = Instant::now() + Duration::from_secs(10);
    while notification_requests.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(notification_requests.load(Ordering::SeqCst), 1);

    let run_server = |cwd: &Path, args: &[&str]| {
        command(&agent_home, cwd)
            .env("FINITE_BRAIN_SERVER_URL", &proxy_url)
            .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
            .args(args)
            .output()
            .unwrap()
    };
    let tree = working_tree_root.join("personal-a");
    let opened = run_server(
        &agent_home,
        &["open", "personal-a", tree.to_str().unwrap(), "--json"],
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    let folder = run_server(&tree, &["folder", "create", "Notes", "--json"]);
    assert!(
        folder.status.success(),
        "{}",
        String::from_utf8_lossy(&folder.stderr)
    );
    let mirror = scratch.path().join("mirror");
    let opened_mirror = run_server(
        &agent_home,
        &["open", "personal-a", mirror.to_str().unwrap(), "--json"],
    );
    assert!(
        opened_mirror.status.success(),
        "{}",
        String::from_utf8_lossy(&opened_mirror.stderr)
    );

    let expected = "# Automatic local sync\n\nOld-server compatibility proof.\n";
    fs::write(tree.join("Notes/automatic.md"), expected).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let synced = run_server(&mirror, &["sync", "now", "--json"]);
        assert!(
            synced.status.success(),
            "{}",
            String::from_utf8_lossy(&synced.stderr)
        );
        if fs::read_to_string(mirror.join("Notes/automatic.md"))
            .ok()
            .as_deref()
            == Some(expected)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "supervised local edit did not reach a second Working Tree\nlog:\n{}\nstate:\n{}",
            fs::read_to_string(&supervisor_log_path).unwrap_or_default(),
            fs::read_to_string(tree.join(".finitebrain/agent-state.json")).unwrap_or_default(),
        );
        thread::sleep(Duration::from_millis(100));
    }

    fs::rename(
        &working_tree_root,
        scratch.path().join("retired-supervised-trees"),
    )
    .unwrap();
    fs::create_dir_all(&working_tree_root).unwrap();
    let reopened = run_server(
        &agent_home,
        &["open", "personal-a", tree.to_str().unwrap(), "--json"],
    );
    assert!(
        reopened.status.success(),
        "{}",
        String::from_utf8_lossy(&reopened.stderr)
    );
    let reset_expected = "# Automatic local sync after reset\n\nRoot lifecycle proof.\n";
    fs::write(tree.join("Notes/after-root-reset.md"), reset_expected).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let synced = run_server(&mirror, &["sync", "now", "--json"]);
        assert!(
            synced.status.success(),
            "{}",
            String::from_utf8_lossy(&synced.stderr)
        );
        if fs::read_to_string(mirror.join("Notes/after-root-reset.md"))
            .ok()
            .as_deref()
            == Some(reset_expected)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "supervisor did not recover after its entire Working Tree root was replaced\nlog:\n{}\nstate:\n{}",
            fs::read_to_string(&supervisor_log_path).unwrap_or_default(),
            fs::read_to_string(tree.join(".finitebrain/agent-state.json")).unwrap_or_default(),
        );
        thread::sleep(Duration::from_millis(100));
    }
    assert!(supervisor.0.try_wait().unwrap().is_none());
    assert_eq!(notification_requests.load(Ordering::SeqCst), 1);
    let state: Value =
        serde_json::from_slice(&fs::read(tree.join(".finitebrain/agent-state.json")).unwrap())
            .unwrap();
    assert!(
        !state["sync"]["status"]
            .as_str()
            .unwrap_or_default()
            .starts_with("blocked:")
    );
    assert!(state["daemon"]["lastError"].is_null());

    drop(supervisor);
    proxy_stop.store(true, Ordering::SeqCst);
    proxy_thread.join().unwrap();
    server_shutdown.send(()).unwrap();
    server_thread.join().unwrap();
}

#[test]
fn supervisor_catches_up_remote_updates_after_repeated_working_tree_root_replacement() {
    let scratch = TempDir::new().unwrap();
    let owner_home = scratch.path().join("owner-home");
    let agent_home = scratch.path().join("agent-home");
    fs::create_dir_all(&owner_home).unwrap();
    fs::create_dir_all(&agent_home).unwrap();
    let owner_secret = scratch.path().join("owner-secret");
    let agent_secret = scratch.path().join("agent-secret");
    fs::write(
        &owner_secret,
        "0000000000000000000000000000000000000000000000000000000000000001\n",
    )
    .unwrap();
    fs::write(
        &agent_secret,
        "0000000000000000000000000000000000000000000000000000000000000003\n",
    )
    .unwrap();
    for (home, secret) in [(&owner_home, &owner_secret), (&agent_home, &agent_secret)] {
        let imported = run(
            home,
            home,
            &[
                "auth",
                "import",
                "--file",
                secret.to_str().unwrap(),
                "--json",
            ],
        );
        assert!(
            imported.status.success(),
            "{}",
            String::from_utf8_lossy(&imported.stderr)
        );
    }
    let public_key = |home: &Path| {
        let output = run(home, home, &["signer", "public-key", "--json"]);
        assert!(output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        value["npub"].as_str().unwrap().to_owned()
    };
    let owner_npub = public_key(&owner_home);
    let agent_npub = public_key(&agent_home);
    let (server_url, server_shutdown, server_thread) =
        spawn_real_brain_server(&agent_npub, &agent_npub, &owner_npub, &owner_npub);
    let run_server = |home: &Path, cwd: &Path, args: &[&str]| {
        command(home, cwd)
            .env("FINITE_BRAIN_SERVER_URL", &server_url)
            .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
            .args(args)
            .output()
            .unwrap()
    };

    let working_tree_root = scratch.path().join("supervised-trees");
    fs::create_dir_all(&working_tree_root).unwrap();
    let supervisor_log_path = scratch.path().join("supervisor.log");
    let supervisor_log = fs::File::create(&supervisor_log_path).unwrap();
    let supervisor = command(&agent_home, &agent_home)
        .env("FINITE_BRAIN_SERVER_URL", &server_url)
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
        .env("FBRAIN_WORKING_TREE_ROOT", &working_tree_root)
        .args(["daemon", "supervise"])
        .stdout(Stdio::from(supervisor_log.try_clone().unwrap()))
        .stderr(Stdio::from(supervisor_log))
        .spawn()
        .unwrap();
    let mut supervisor = ChildGuard(supervisor);

    let tree = working_tree_root.join("personal-a");
    for reset in 0..3 {
        if reset > 0 {
            fs::rename(
                &working_tree_root,
                scratch
                    .path()
                    .join(format!("retired-supervised-trees-{reset}")),
            )
            .unwrap();
            fs::create_dir_all(&working_tree_root).unwrap();
        }
        let opened = run_server(
            &agent_home,
            &agent_home,
            &["open", "personal-a", tree.to_str().unwrap(), "--json"],
        );
        assert!(
            opened.status.success(),
            "{}",
            String::from_utf8_lossy(&opened.stderr)
        );
    }

    let owner_tree = scratch.path().join("owner-tree");
    let opened_owner = run_server(
        &owner_home,
        &owner_home,
        &["open", "personal-a", owner_tree.to_str().unwrap(), "--json"],
    );
    assert!(
        opened_owner.status.success(),
        "{}",
        String::from_utf8_lossy(&opened_owner.stderr)
    );
    let created = run_server(
        &owner_home,
        &owner_tree,
        &["folder", "create", "Remote Notification Folder", "--json"],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );

    let deadline = Instant::now() + Duration::from_secs(15);
    while !tree.join("Remote Notification Folder").is_dir() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(
        tree.join("Remote Notification Folder").is_dir(),
        "supervisor did not catch up a remote update after repeated Working Tree root replacement\nlog:\n{}\nstate:\n{}",
        fs::read_to_string(&supervisor_log_path).unwrap_or_default(),
        fs::read_to_string(tree.join(".finitebrain/agent-state.json")).unwrap_or_default(),
    );
    assert!(supervisor.0.try_wait().unwrap().is_none());

    drop(supervisor);
    server_shutdown.send(()).unwrap();
    server_thread.join().unwrap();
}

#[test]
fn supervisor_runs_with_builtin_working_tree_root_default_and_flag_override() {
    let scratch = TempDir::new().unwrap();
    let home = scratch.path().join("home");
    fs::create_dir_all(&home).unwrap();

    // Neither FBRAIN_WORKING_TREE_ROOT nor a flag: the supervisor falls back
    // to the hosted CLI default (current directory) instead of hard-erroring
    // on the unset env var. The unreachable loopback server keeps the
    // notification reconnect loop inert for the single handled event.
    let defaulted = command(&home, &home)
        .env("FINITE_BRAIN_SERVER_URL", "http://127.0.0.1:9")
        .args(["daemon", "supervise", "--max-events", "1"])
        .output()
        .unwrap();
    assert!(
        defaulted.status.success(),
        "{}",
        String::from_utf8_lossy(&defaulted.stderr)
    );
    assert!(
        String::from_utf8_lossy(&defaulted.stdout).contains("daemon supervise stopped events=1"),
        "{}",
        String::from_utf8_lossy(&defaulted.stdout)
    );

    // An explicit --working-tree-root flag overrides the built-in default.
    let flagged_root = scratch.path().join("flagged-trees");
    let flagged = command(&home, &home)
        .env("FINITE_BRAIN_SERVER_URL", "http://127.0.0.1:9")
        .args([
            "daemon",
            "supervise",
            "--working-tree-root",
            flagged_root.to_str().unwrap(),
            "--max-events",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        flagged.status.success(),
        "{}",
        String::from_utf8_lossy(&flagged.stderr)
    );
    assert!(flagged_root.is_dir());
}

#[test]
fn supervisor_quiesces_after_catch_up_when_nothing_changes() {
    let scratch = TempDir::new().unwrap();
    let home = scratch.path().join("home");
    fs::create_dir_all(&home).unwrap();
    // Pre-create the supervised root: when the supervisor creates it itself,
    // FSEvents can deliver that creation to the freshly started watcher,
    // which is a legitimate lifecycle event unrelated to this check.
    let working_tree_root = scratch.path().join("supervised-trees");
    fs::create_dir_all(&working_tree_root).unwrap();
    let secret = scratch.path().join("secret");
    fs::write(
        &secret,
        "0000000000000000000000000000000000000000000000000000000000000007\n",
    )
    .unwrap();
    let imported = run(
        &home,
        &home,
        &[
            "auth",
            "import",
            "--file",
            secret.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );

    // The server accepts the notification stream connection and then never
    // answers, so the stream thread blocks inside the HTTP request: the
    // filesystem watcher is the only remaining event source.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => held.push(stream),
                Err(_) => break,
            }
        }
    });

    let supervisor_log_path = scratch.path().join("supervisor.log");
    let supervisor_log = fs::File::create(&supervisor_log_path).unwrap();
    let supervisor = command(&home, &home)
        .env("FINITE_BRAIN_SERVER_URL", &server_url)
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
        .args([
            "daemon",
            "supervise",
            "--working-tree-root",
            working_tree_root.to_str().unwrap(),
            // startup_catch_up is the only legitimate pending event; the
            // stream thread is blocked inside its unanswered request. A second
            // handled event means the supervisor manufactured work by itself:
            // before observation traffic was filtered, every handled event
            // re-scanned the Working Tree root and the scan's directory reads
            // re-armed the watcher, so the process exited almost immediately.
            "--max-events",
            "2",
        ])
        .stdout(Stdio::from(supervisor_log.try_clone().unwrap()))
        .stderr(Stdio::from(supervisor_log))
        .spawn()
        .unwrap();
    let mut supervisor = ChildGuard(supervisor);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        assert!(
            supervisor.0.try_wait().unwrap().is_none(),
            "supervisor handled more events than exist without any local change; \
             its own observation traffic is feeding the filesystem watcher: {}",
            fs::read_to_string(&supervisor_log_path).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn built_fbrain_process_brain_restore_drill() {
    // The #459/#527 drill, service level: populate a file-backed Brain,
    // revoke a principal, stop the server, copy the database, destroy the
    // original, and restore onto an empty target. The restored server must
    // preserve memberships with provenance and accepted invitations — and a
    // still-pending npub invitation must remain acceptable after the restore.
    // Clients hold the keys; the server only ever sees ciphertext and access
    // facts.
    let scratch = TempDir::new().unwrap();
    let home_alice = scratch.path().join("home-alice");
    let home_member = scratch.path().join("home-member");
    let home_bob = scratch.path().join("home-bob");
    let home_bob_human = scratch.path().join("home-bob-human");
    for home in [&home_alice, &home_member, &home_bob, &home_bob_human] {
        fs::create_dir_all(home).unwrap();
    }
    for (home, secret) in [
        (
            &home_alice,
            "0000000000000000000000000000000000000000000000000000000000000001",
        ),
        (
            &home_bob,
            "0000000000000000000000000000000000000000000000000000000000000002",
        ),
        (
            &home_member,
            "0000000000000000000000000000000000000000000000000000000000000003",
        ),
        (
            &home_bob_human,
            "0000000000000000000000000000000000000000000000000000000000000005",
        ),
    ] {
        let secret_file = scratch.path().join(format!("secret-{}", &secret[..2]));
        fs::write(&secret_file, format!("{secret}\n")).unwrap();
        assert!(
            run(
                home,
                home,
                &[
                    "auth",
                    "import",
                    "--file",
                    secret_file.to_str().unwrap(),
                    "--json"
                ]
            )
            .status
            .success()
        );
    }
    let bob_human_keys =
        nostr::Keys::parse("0000000000000000000000000000000000000000000000000000000000000005")
            .unwrap();
    let bob_human_npub = NostrPublicKey::from_protocol(bob_human_keys.public_key())
        .to_npub()
        .unwrap();
    let npub_of = |home: &Path| -> String {
        let output = run(home, home, &["signer", "public-key", "--json"]);
        assert!(output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        value["npub"].as_str().unwrap().to_owned()
    };
    let alice_npub = npub_of(&home_alice);
    let member_npub = npub_of(&home_member);
    let bob_agent_npub = npub_of(&home_bob);

    let database_path = scratch.path().join("state").join("brain-a.sqlite3");
    fs::create_dir_all(database_path.parent().unwrap()).unwrap();
    let (server_a_url, shutdown_a, server_a) =
        spawn_file_backed_brain_server(&alice_npub, database_path.clone());
    let run_against = |home: &Path, server_url: &str, args: &[&str]| {
        let now = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
        command(home, home)
            .env("FBRAIN_NOW", now)
            .env("FINITE_BRAIN_SERVER_URL", server_url)
            .env("FINITE_BRAIN_PUBLIC_BASE_URL", server_url)
            .args(args)
            .output()
            .unwrap()
    };
    let json_of = |label: &str, output: &Output| -> Value {
        assert!(
            output.status.success(),
            "{label} failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };

    // Populate: a member without admin standing can no longer file an
    // email-targeted invite (retired with the identity-proof flow); alice
    // invites bob's npub directly and bob accepts.
    assert!(
        run_against(
            &home_alice,
            &server_a_url,
            &[
                "admin",
                "member",
                "add",
                "--brain",
                "roundtrip-org",
                "--target",
                &member_npub,
                "--json"
            ]
        )
        .status
        .success()
    );
    let retired_invite = run_against(
        &home_member,
        &server_a_url,
        &[
            "invite",
            "brain",
            "create",
            "--brain",
            "roundtrip-org",
            "--target",
            "bob@example.com",
            "--json",
        ],
    );
    assert!(
        !retired_invite.status.success(),
        "email-targeted invitation must be retired"
    );
    json_of(
        "alice invites bob",
        &run_against(
            &home_alice,
            &server_a_url,
            &[
                "invite",
                "brain",
                "create",
                "--brain",
                "roundtrip-org",
                "--target",
                &bob_agent_npub,
                "--json",
            ],
        ),
    );
    let bob_invitations = json_of(
        "bob list",
        &run_against(
            &home_bob,
            &server_a_url,
            &["invite", "brain", "list", "--json"],
        ),
    );
    let accepted_invitation_id = bob_invitations["invitations"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|invitation| {
            (invitation["brainId"].as_str() == Some("roundtrip-org"))
                .then(|| invitation["id"].as_str().unwrap().to_owned())
        })
        .expect("bob has an invitation");
    json_of(
        "bob accept",
        &run_against(
            &home_bob,
            &server_a_url,
            &[
                "invite",
                "brain",
                "accept",
                "--id",
                &accepted_invitation_id,
                "--json",
            ],
        ),
    );
    // A pending npub invitation that must survive the restore unaccepted and
    // stay acceptable afterwards.
    let pending = json_of(
        "pending invite for bob's other key",
        &run_against(
            &home_alice,
            &server_a_url,
            &[
                "invite",
                "brain",
                "create",
                "--brain",
                "roundtrip-org",
                "--target",
                &bob_human_npub,
                "--json",
            ],
        ),
    );
    let pending_id = pending["id"].as_str().unwrap().to_owned();

    // Revocation: the member is removed and must stay removed after restore.
    assert!(
        run_against(
            &home_alice,
            &server_a_url,
            &[
                "admin",
                "member",
                "remove",
                "--brain",
                "roundtrip-org",
                "--target",
                &member_npub,
                "--json"
            ]
        )
        .status
        .success()
    );

    // Backup, destroy, restore onto an empty target. The clean shutdown
    // closes the WAL so the file copy is a complete backup, exactly like a
    // Litestream restore point.
    drop(shutdown_a);
    server_a.join().unwrap();
    let backup_path = scratch.path().join("backup").join("brain-restored.sqlite3");
    fs::create_dir_all(backup_path.parent().unwrap()).unwrap();
    fs::copy(&database_path, &backup_path).unwrap();
    fs::remove_file(&database_path).unwrap();
    for wal in ["-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{wal}", database_path.display()));
    }
    let restored_path = scratch.path().join("empty-target").join("brain.sqlite3");
    fs::create_dir_all(restored_path.parent().unwrap()).unwrap();
    fs::copy(&backup_path, &restored_path).unwrap();
    let (server_b_url, shutdown_b, server_b) =
        spawn_file_backed_brain_server(&alice_npub, restored_path);

    // Memberships and provenance survived; the revoked member did not.
    let metadata = json_of(
        "restored metadata",
        &run_against(
            &home_alice,
            &server_b_url,
            &["brain", "metadata", "--brain", "roundtrip-org", "--json"],
        ),
    );
    let members: Vec<&str> = metadata["members"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|member| member.as_str())
        .collect();
    assert!(members.contains(&bob_agent_npub.as_str()));
    assert!(!members.contains(&member_npub.as_str()));

    // The restored server still answers for the accepted principal.
    let bob_brains = json_of(
        "restored bob brain list",
        &run_against(&home_bob, &server_b_url, &["brain", "list", "--json"]),
    );
    assert!(
        bob_brains["brains"]
            .as_array()
            .unwrap()
            .iter()
            .any(|brain| brain["brainId"].as_str() == Some("roundtrip-org"))
    );

    // The pending invitation survived unaccepted and is still acceptable:
    // bob's human key sees it and accepts it against the restored server.
    let human_pending = json_of(
        "restored pending list",
        &run_against(
            &home_bob_human,
            &server_b_url,
            &["invite", "brain", "list", "--json"],
        ),
    );
    assert!(
        human_pending["invitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|invitation| invitation["id"].as_str() == Some(pending_id.as_str()))
    );
    json_of(
        "restored accept",
        &run_against(
            &home_bob_human,
            &server_b_url,
            &["invite", "brain", "accept", "--id", &pending_id, "--json"],
        ),
    );

    // The revoked member's standing did not survive as access.
    let member_list = run_against(&home_member, &server_b_url, &["brain", "list", "--json"]);
    if member_list.status.success() {
        let member_brains: Value = serde_json::from_slice(&member_list.stdout).unwrap();
        assert!(
            !member_brains["brains"]
                .as_array()
                .unwrap()
                .iter()
                .any(|brain| brain["brainId"].as_str() == Some("roundtrip-org"))
        );
    }

    // No restore path ever exposed plaintext or keys: the backup file is the
    // same ciphertext-bearing SQLite the server held all along.
    drop(shutdown_b);
    server_b.join().unwrap();
}

#[test]
fn built_fbrain_invite_brain_create_accepts_one_hour_from_a_lagging_client() {
    // FIN-147: the CLI computes `expiresAt` from its own clock (`FBRAIN_NOW`)
    // and the server stamps `createdAt` on receipt. Pin the server clock and
    // run the CLI clock 30 seconds behind it, as request latency or clock
    // offset does in production.
    let scratch = TempDir::new().unwrap();
    let home = scratch.path().join("home-admin");
    fs::create_dir_all(&home).unwrap();
    let secret_file = scratch.path().join("secret-admin");
    fs::write(
        &secret_file,
        "0000000000000000000000000000000000000000000000000000000000000001\n",
    )
    .unwrap();
    let imported = run(
        &home,
        &home,
        &[
            "auth",
            "import",
            "--file",
            secret_file.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(imported.status.success());
    let public_key = run(&home, &home, &["signer", "public-key", "--json"]);
    assert!(public_key.status.success());
    let public_key: Value = serde_json::from_slice(&public_key.stdout).unwrap();
    let admin_npub = public_key["npub"].as_str().unwrap().to_owned();
    let target_npub = |secret: &str| {
        NostrPublicKey::from_protocol(Keys::parse(secret).unwrap().public_key())
            .to_npub()
            .unwrap()
    };

    let server_now = OffsetDateTime::now_utc().unix_timestamp();
    let (server_url, shutdown, server) = spawn_clocked_file_backed_brain_server(
        &admin_npub,
        scratch.path().join("brain.sqlite3"),
        Some(server_now as u64),
    );
    let invite = |client_now: OffsetDateTime, target: &str, expires_in: &str| {
        command(&home, &home)
            .env("FBRAIN_NOW", client_now.format(&Rfc3339).unwrap())
            .env("FINITE_BRAIN_SERVER_URL", &server_url)
            .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
            .args([
                "invite",
                "brain",
                "create",
                "--brain",
                "roundtrip-org",
                "--target",
                target,
                "--expires-in",
                expires_in,
                "--json",
            ])
            .output()
            .unwrap()
    };

    let lagging_client_now = OffsetDateTime::from_unix_timestamp(server_now - 30).unwrap();
    let created = invite(
        lagging_client_now,
        &target_npub("0000000000000000000000000000000000000000000000000000000000000003"),
        "1h",
    );
    assert!(
        created.status.success(),
        "invite brain create --expires-in 1h failed: {}{}",
        String::from_utf8_lossy(&created.stdout),
        String::from_utf8_lossy(&created.stderr),
    );
    let created: Value = serde_json::from_slice(&created.stdout).unwrap();
    // The server stores exactly what the CLI asked for; nothing extends it.
    assert_eq!(
        created["expiresAt"].as_str().unwrap(),
        (lagging_client_now + time::Duration::hours(1))
            .format(&Rfc3339)
            .unwrap()
    );

    // The thirty-day ceiling stays strict for a client whose clock leads.
    let leading_client_now = OffsetDateTime::from_unix_timestamp(server_now + 30).unwrap();
    let too_long = invite(
        leading_client_now,
        &target_npub("0000000000000000000000000000000000000000000000000000000000000004"),
        "30d",
    );
    assert!(!too_long.status.success());
    assert!(
        String::from_utf8_lossy(&too_long.stderr)
            .contains("invitation expiry must be between 55 minutes and thirty days from creation"),
        "unexpected rejection: {}",
        String::from_utf8_lossy(&too_long.stderr),
    );

    drop(shutdown);
    server.join().unwrap();
}

#[test]
fn built_fbrain_process_restores_demoted_admin_folder_access_with_retained_key() {
    let scratch = TempDir::new().unwrap();
    let owner_home = scratch.path().join("owner");
    let member_home = scratch.path().join("member");
    let mut npubs = Vec::new();
    for home in [&owner_home, &member_home] {
        fs::create_dir_all(home).unwrap();
        let keys = Keys::generate();
        let secret = home.join("import-key");
        fs::write(&secret, keys.secret_key().to_secret_hex()).unwrap();
        let imported = run(
            home,
            home,
            &[
                "auth",
                "import",
                "--file",
                secret.to_str().unwrap(),
                "--json",
            ],
        );
        assert!(
            imported.status.success(),
            "{}",
            String::from_utf8_lossy(&imported.stderr)
        );
        fs::remove_file(secret).unwrap();
        npubs.push(
            NostrPublicKey::from_protocol(keys.public_key())
                .to_npub()
                .unwrap(),
        );
    }
    let owner = &npubs[0];
    let member = &npubs[1];
    let (server_url, shutdown, server_thread) =
        spawn_real_brain_server(member, member, owner, owner);
    let run_json = |home: &Path, cwd: &Path, args: &[&str]| -> Value {
        let output = command(home, cwd)
            .env(
                "FBRAIN_NOW",
                OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
            )
            .env("FINITE_BRAIN_SERVER_URL", &server_url)
            .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    run_json(
        &owner_home,
        &owner_home,
        &["brain", "create", "organization", "Repair"],
    );
    let owner_tree = owner_home.join("tree");
    run_json(
        &owner_home,
        &owner_home,
        &["open", "repair", owner_tree.to_str().unwrap()],
    );
    run_json(
        &owner_home,
        &owner_tree,
        &["admin", "member", "add", "--target", member],
    );
    run_json(
        &owner_home,
        &owner_tree,
        &["admin", "role", "grant", "admin", "--target", member],
    );
    // Creating the folders while the recipient is an admin gives them a key
    // through native access, without a direct folder permission.
    for name in ["Shared", "Unrelated"] {
        run_json(&owner_home, &owner_tree, &["folder", "create", name]);
    }
    run_json(&owner_home, &owner_tree, &["sync", "now"]);
    fs::write(owner_tree.join("Shared/note.md"), "# Retained key proof\n").unwrap();
    fs::write(owner_tree.join("Unrelated/private.md"), "# Unrelated\n").unwrap();
    run_json(&owner_home, &owner_tree, &["sync", "now"]);
    let member_tree = member_home.join("tree");
    run_json(
        &member_home,
        &member_home,
        &["open", "repair", member_tree.to_str().unwrap()],
    );
    run_json(&member_home, &member_tree, &["sync", "now"]);
    assert_eq!(
        fs::read_to_string(member_tree.join("Shared/note.md")).unwrap(),
        "# Retained key proof\n"
    );

    run_json(
        &owner_home,
        &owner_tree,
        &["admin", "role", "revoke", "admin", "--target", member],
    );
    run_json(&member_home, &member_tree, &["sync", "now"]);
    // The client preserves old downloaded bytes after access loss. Check the
    // server's authority, then require a new revision to prove restored reads.
    let denied = run_json(&member_home, &member_tree, &["brain", "export"]);
    for folder_id in ["shared", "unrelated"] {
        let folder = denied["folders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|folder| folder["id"] == folder_id)
            .unwrap();
        assert_eq!(folder["accessible"], false);
    }
    fs::write(
        owner_tree.join("Shared/note.md"),
        "# Written after demotion\n",
    )
    .unwrap();
    fs::write(
        owner_tree.join("Unrelated/private.md"),
        "# Still private after demotion\n",
    )
    .unwrap();
    run_json(&owner_home, &owner_tree, &["sync", "now"]);
    let before = run_json(&owner_home, &owner_tree, &["brain", "export"]);
    let repair = [
        "admin",
        "folder-access",
        "grant",
        "--folder",
        "shared",
        "--target",
        member,
    ];
    let restored = run_json(&owner_home, &owner_tree, &repair);
    assert_eq!(restored["outcome"], "granted");
    assert_eq!(restored["admins"], json!([owner]));
    let retry = run_json(&owner_home, &owner_tree, &repair);
    assert_eq!(retry["outcome"], "alreadyHasAccess");
    let after = run_json(&owner_home, &owner_tree, &["brain", "export"]);
    assert_eq!(after["keyGrants"], before["keyGrants"]);
    run_json(&member_home, &member_tree, &["sync", "now"]);
    assert_eq!(
        fs::read_to_string(member_tree.join("Shared/note.md")).unwrap(),
        "# Written after demotion\n"
    );
    assert_eq!(
        fs::read_to_string(member_tree.join("Unrelated/private.md")).unwrap(),
        "# Unrelated\n"
    );

    // A fresh local tree must also bootstrap and decrypt the retained grant.
    let fresh_tree = member_home.join("fresh-tree");
    run_json(
        &member_home,
        &member_home,
        &["open", "repair", fresh_tree.to_str().unwrap()],
    );
    run_json(&member_home, &fresh_tree, &["sync", "now"]);
    assert_eq!(
        fs::read_to_string(fresh_tree.join("Shared/note.md")).unwrap(),
        "# Written after demotion\n"
    );
    assert!(!fresh_tree.join("Unrelated/private.md").exists());
    shutdown.send(()).unwrap();
    server_thread.join().unwrap();
}

#[test]
fn built_fbrain_process_two_independent_homes_open_restricted_collaboration() {
    let mut smoke = CollaborationSmokeReport::from_environment();
    let scratch = TempDir::new().unwrap();
    let home_a = scratch.path().join("home-a");
    let home_b = scratch.path().join("home-b");
    fs::create_dir_all(&home_a).unwrap();
    fs::create_dir_all(&home_b).unwrap();
    let secret_a = scratch.path().join("secret-a");
    let secret_b = scratch.path().join("secret-b");
    fs::write(
        &secret_a,
        "0000000000000000000000000000000000000000000000000000000000000001\n",
    )
    .unwrap();
    fs::write(
        &secret_b,
        "0000000000000000000000000000000000000000000000000000000000000002\n",
    )
    .unwrap();
    assert!(
        run(
            &home_a,
            &home_a,
            &[
                "auth",
                "import",
                "--file",
                secret_a.to_str().unwrap(),
                "--json"
            ]
        )
        .status
        .success()
    );
    assert!(
        run(
            &home_b,
            &home_b,
            &[
                "auth",
                "import",
                "--file",
                secret_b.to_str().unwrap(),
                "--json"
            ]
        )
        .status
        .success()
    );
    smoke.pass();

    smoke.enter("signedBrainHttp");
    let signer_b = run(&home_b, &home_b, &["signer", "public-key", "--json"]);
    assert!(
        signer_b.status.success(),
        "{}",
        String::from_utf8_lossy(&signer_b.stderr)
    );
    let signer_b: Value = serde_json::from_slice(&signer_b.stdout).unwrap();
    let target_npub = signer_b["npub"].as_str().unwrap().to_owned();
    let signer_a = run(&home_a, &home_a, &["signer", "public-key", "--json"]);
    assert!(signer_a.status.success());
    let signer_a: Value = serde_json::from_slice(&signer_a.stdout).unwrap();
    let owner_npub = signer_a["npub"].as_str().unwrap().to_owned();
    let requester_keys =
        nostr::Keys::parse("0000000000000000000000000000000000000000000000000000000000000004")
            .unwrap();
    let requester_hex = requester_keys.public_key().to_hex();
    let requester_npub = NostrPublicKey::from_protocol(requester_keys.public_key())
        .to_npub()
        .unwrap();
    let requester_home = scratch.path().join("requester-home");
    fs::create_dir_all(&requester_home).unwrap();
    let requester_secret = scratch.path().join("requester-secret");
    fs::write(
        &requester_secret,
        "0000000000000000000000000000000000000000000000000000000000000004\n",
    )
    .unwrap();
    assert!(
        run(
            &requester_home,
            &requester_home,
            &[
                "auth",
                "import",
                "--file",
                requester_secret.to_str().unwrap(),
                "--json",
            ],
        )
        .status
        .success()
    );
    let personal_agent_home = scratch.path().join("personal-agent-home");
    fs::create_dir_all(&personal_agent_home).unwrap();
    let personal_agent_secret = scratch.path().join("personal-agent-secret");
    fs::write(
        &personal_agent_secret,
        "0000000000000000000000000000000000000000000000000000000000000003\n",
    )
    .unwrap();
    assert!(
        run(
            &personal_agent_home,
            &personal_agent_home,
            &[
                "auth",
                "import",
                "--file",
                personal_agent_secret.to_str().unwrap(),
                "--json",
            ],
        )
        .status
        .success()
    );
    let personal_agent = run(
        &personal_agent_home,
        &personal_agent_home,
        &["signer", "public-key", "--json"],
    );
    assert!(personal_agent.status.success());
    let personal_agent: Value = serde_json::from_slice(&personal_agent.stdout).unwrap();
    let personal_agent_npub = personal_agent["npub"].as_str().unwrap().to_owned();
    let (server_url, shutdown, server_thread) = spawn_real_brain_server(
        &target_npub,
        &personal_agent_npub,
        &owner_npub,
        &requester_npub,
    );
    let run = |home: &Path, cwd: &Path, args: &[&str]| {
        let now = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
        command(home, cwd)
            .env("FBRAIN_NOW", now)
            .env("FINITE_BRAIN_SERVER_URL", &server_url)
            .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
            .args(args)
            .output()
            .unwrap()
    };
    let doctor = run(&home_a, &home_a, &["doctor", "--json"]);
    assert!(
        doctor.status.success(),
        "{}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let doctor: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(doctor["server"]["state"], "ok");
    let discovered = run(&home_a, &home_a, &["brain", "list", "--json"]);
    assert!(discovered.status.success());
    let missing_requester = command(&home_a, &home_a)
        .env("HERMES_SESSION_PLATFORM", "finitechat")
        .env("HERMES_SESSION_KEY", "missing-requester-session")
        .env("HERMES_SESSION_USER_ID", &requester_hex)
        .env("FINITE_BRAIN_SERVER_URL", &server_url)
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
        .args([
            "brain",
            "create",
            "organization",
            "Must Not Exist",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!missing_requester.status.success());
    assert!(
        String::from_utf8_lossy(&missing_requester.stderr)
            .contains("authenticated Finite Chat requester context is unavailable")
    );
    let after_missing_requester = run(&home_a, &home_a, &["brain", "list", "--json"]);
    assert!(after_missing_requester.status.success());
    let after_missing_requester: Value =
        serde_json::from_slice(&after_missing_requester.stdout).unwrap();
    assert!(
        after_missing_requester["brains"]
            .as_array()
            .unwrap()
            .iter()
            .all(|brain| brain["brainId"] != "must-not-exist")
    );
    let missing_folder_context = run(
        &home_b,
        &home_b,
        &["folder", "create", "Must Not Exist", "--json"],
    );
    assert!(!missing_folder_context.status.success());
    let missing_folder_context = String::from_utf8_lossy(&missing_folder_context.stderr);
    assert!(missing_folder_context.contains("No active Brain Working Tree was found"));
    assert!(missing_folder_context.contains("fbrain brain list"));
    assert!(missing_folder_context.contains("open the intended Brain"));
    let direct_human = run(
        &requester_home,
        &requester_home,
        &["brain", "create", "organization", "Human Direct", "--json"],
    );
    assert!(
        direct_human.status.success(),
        "{}",
        String::from_utf8_lossy(&direct_human.stderr)
    );
    let direct_human: Value = serde_json::from_slice(&direct_human.stdout).unwrap();
    assert_eq!(direct_human["brainId"], "human-direct");
    assert_eq!(direct_human["admins"], json!([requester_npub]));
    let session_key = "brain-create-session-a";
    write_requester_context(&home_a.join("finite-home"), session_key, &requester_hex);
    let create = command(&home_a, &home_a)
        .env("HERMES_SESSION_PLATFORM", "finitechat")
        .env("HERMES_SESSION_KEY", session_key)
        .env("HERMES_SESSION_USER_ID", &requester_hex)
        .env("FINITE_BRAIN_SERVER_URL", &server_url)
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
        .args(["brain", "create", "organization", "Acme", "--json"])
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    let created: Value = serde_json::from_slice(&create.stdout).unwrap();
    assert_eq!(created["brainId"], "acme");
    assert_eq!(created["name"], "Acme");
    assert!(created["folders"].as_array().unwrap().is_empty());
    let admins = created["admins"].as_array().unwrap();
    assert!(admins.iter().any(|admin| admin == &owner_npub));
    assert!(admins.iter().any(|admin| admin == &requester_npub));
    let duplicate_brain = run(
        &home_a,
        &home_a,
        &["brain", "create", "organization", "Acme", "--json"],
    );
    assert!(!duplicate_brain.status.success());
    let duplicate_brain_error = String::from_utf8_lossy(&duplicate_brain.stderr);
    assert!(duplicate_brain_error.contains("Brain already exists"));
    assert!(duplicate_brain_error.contains("id=acme"));
    smoke.pass();

    smoke.enter("restrictedKnowledgeBeforeCollaboration");
    let tree_a = home_a.join("tree-a");
    let opened_a = run(
        &home_a,
        &home_a,
        &["open", "acme", tree_a.to_str().unwrap(), "--json"],
    );
    assert!(
        opened_a.status.success(),
        "{}",
        String::from_utf8_lossy(&opened_a.stderr)
    );
    let folder = run(
        &home_a,
        &tree_a,
        &["folder", "create", "Restricted", "--json"],
    );
    assert!(
        folder.status.success(),
        "{}",
        String::from_utf8_lossy(&folder.stderr)
    );
    let duplicate_folder = run(
        &home_a,
        &tree_a,
        &["folder", "create", "Restricted", "--json"],
    );
    assert!(!duplicate_folder.status.success());
    let duplicate_folder_error = String::from_utf8_lossy(&duplicate_folder.stderr);
    assert!(duplicate_folder_error.contains("Folder already exists"));
    assert!(duplicate_folder_error.contains("id=restricted"));
    let folders_after_duplicate = run(&home_a, &tree_a, &["folder", "list", "--json"]);
    assert!(folders_after_duplicate.status.success());
    let folders_after_duplicate: Value =
        serde_json::from_slice(&folders_after_duplicate.stdout).unwrap();
    assert_eq!(
        folders_after_duplicate
            .as_array()
            .expect("Folder list must be an array")
            .iter()
            .filter(|folder| folder["id"] == "restricted")
            .count(),
        1
    );
    let unrelated_folder = run(
        &home_a,
        &tree_a,
        &["folder", "create", "Unrelated", "--json"],
    );
    assert!(
        unrelated_folder.status.success(),
        "{}",
        String::from_utf8_lossy(&unrelated_folder.stderr)
    );
    let safe_path_folder = run(
        &home_a,
        &tree_a,
        &["folder", "create", "Research: Primary Sources", "--json"],
    );
    assert!(
        safe_path_folder.status.success(),
        "{}",
        String::from_utf8_lossy(&safe_path_folder.stderr)
    );
    let safe_path_folder: Value = serde_json::from_slice(&safe_path_folder.stdout).unwrap();
    let safe_path_folder = safe_path_folder["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|folder| folder["id"] == "research-primary-sources")
        .unwrap();
    assert_eq!(safe_path_folder["name"], "Research: Primary Sources");
    assert_eq!(safe_path_folder["path"], "Research Primary Sources");
    let admin_only_folder = run(
        &home_a,
        &tree_a,
        &[
            "folder",
            "create",
            "admin-only",
            "--access",
            "admin_only",
            "--name",
            "Admin Only",
            "--path",
            "Admin Only",
            "--json",
        ],
    );
    assert!(
        admin_only_folder.status.success(),
        "{}",
        String::from_utf8_lossy(&admin_only_folder.stderr)
    );
    let synced_empty_a = run(&home_a, &tree_a, &["sync", "now", "--json"]);
    assert!(
        synced_empty_a.status.success(),
        "{}",
        String::from_utf8_lossy(&synced_empty_a.stderr)
    );
    let restricted_instructions =
        assert_canonical_folder_projection(&tree_a.join("Restricted"), "restricted");
    assert_canonical_folder_projection(
        &tree_a.join("Research Primary Sources"),
        "research-primary-sources",
    );
    fs::write(
        tree_a.join("Restricted/secret.md"),
        "# Restricted\n\nRecipient-readable proof.\n",
    )
    .unwrap();
    fs::write(
        tree_a.join("Unrelated/other.md"),
        "# Unrelated\n\nMust remain private from a Folder Guest.\n",
    )
    .unwrap();
    fs::write(
        tree_a.join("Admin Only/admin.md"),
        "# Admin Only\n\nExplicit Guest invitation proof.\n",
    )
    .unwrap();
    let synced_a = run(&home_a, &tree_a, &["sync", "now", "--json"]);
    assert!(
        synced_a.status.success(),
        "{}",
        String::from_utf8_lossy(&synced_a.stderr)
    );
    assert_eq!(
        assert_canonical_folder_projection(&tree_a.join("Restricted"), "restricted"),
        restricted_instructions
    );
    assert_eq!(
        fs::read_to_string(tree_a.join("Restricted/secret.md")).unwrap(),
        "# Restricted\n\nRecipient-readable proof.\n"
    );
    let repeated_sync_a = run(&home_a, &tree_a, &["sync", "now", "--json"]);
    assert!(
        repeated_sync_a.status.success(),
        "{}",
        String::from_utf8_lossy(&repeated_sync_a.stderr)
    );
    assert_eq!(
        assert_canonical_folder_projection(&tree_a.join("Restricted"), "restricted"),
        restricted_instructions
    );
    assert_eq!(
        fs::read_to_string(tree_a.join("Restricted/secret.md")).unwrap(),
        "# Restricted\n\nRecipient-readable proof.\n"
    );
    smoke.pass();

    smoke.enter("npubFolderGuestInvitation");
    // Email-targeted invitations are retired (auth kernel cut): an email that
    // does not resolve through public NIP-05 is rejected with guidance toward
    // capability Invite Tokens.
    let retired_brain_invite = run(
        &home_a,
        &tree_a,
        &[
            "invite",
            "brain",
            "create",
            "--target",
            "future-user@example.com",
            "--json",
        ],
    );
    assert!(
        !retired_brain_invite.status.success(),
        "email-targeted Brain invitations are retired"
    );
    assert!(
        String::from_utf8_lossy(&retired_brain_invite.stderr)
            .contains("does not resolve to an npub")
    );
    let retired_folder_invite = run(
        &home_a,
        &tree_a.join("Admin Only"),
        &[
            "invite",
            "folder",
            "create",
            "--target",
            "future-user@example.com",
            "--json",
        ],
    );
    assert!(
        !retired_folder_invite.status.success(),
        "email-targeted Folder invitations are retired"
    );
    assert!(
        String::from_utf8_lossy(&retired_folder_invite.stderr)
            .contains("does not resolve to an npub")
    );

    // Folder guest access is an npub-targeted share link, created by an admin
    // and accepted by the guest key.
    let folder_invitation = run(
        &home_a,
        &tree_a.join("Admin Only"),
        &[
            "invite",
            "folder",
            "create",
            "--target",
            &target_npub,
            "--json",
        ],
    );
    assert!(
        folder_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&folder_invitation.stderr)
    );
    let folder_invitation: Value = serde_json::from_slice(&folder_invitation.stdout).unwrap();
    let folder_invitation_id = folder_invitation["id"].as_str().unwrap();
    let listed_folder_invitations = run(
        &home_a,
        &tree_a.join("Admin Only"),
        &["invite", "folder", "list", "--json"],
    );
    assert!(
        listed_folder_invitations.status.success(),
        "{}",
        String::from_utf8_lossy(&listed_folder_invitations.stderr)
    );
    let listed_folder_invitations: Value =
        serde_json::from_slice(&listed_folder_invitations.stdout).unwrap();
    assert!(
        listed_folder_invitations["invitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|invitation| invitation["id"] == folder_invitation["id"]),
        "a pending Folder Invitation must remain visible through the Folder collection"
    );
    let accepted_folder_invitation = run(
        &home_b,
        &home_b,
        &["invite", "folder", "accept", folder_invitation_id, "--json"],
    );
    assert!(
        accepted_folder_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted_folder_invitation.stderr)
    );
    let cancel_accepted = run(
        &home_a,
        &tree_a.join("Admin Only"),
        &["invite", "folder", "revoke", folder_invitation_id, "--json"],
    );
    assert!(
        !cancel_accepted.status.success(),
        "an accepted Folder Invitation must not be cancellable"
    );
    // A pending share link revokes cleanly and is then unacceptable.
    let stale_npub = NostrPublicKey::from_protocol(Keys::generate().public_key())
        .to_npub()
        .unwrap();
    let pending_invitation = run(
        &home_a,
        &tree_a.join("Admin Only"),
        &[
            "invite",
            "folder",
            "create",
            "--target",
            &stale_npub,
            "--json",
        ],
    );
    assert!(
        pending_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&pending_invitation.stderr)
    );
    let pending_invitation: Value = serde_json::from_slice(&pending_invitation.stdout).unwrap();
    let cancelled = run(
        &home_a,
        &tree_a.join("Admin Only"),
        &[
            "invite",
            "folder",
            "revoke",
            pending_invitation["id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        cancelled.status.success(),
        "{}",
        String::from_utf8_lossy(&cancelled.stderr)
    );
    let cancelled: Value = serde_json::from_slice(&cancelled.stdout).unwrap();
    assert_eq!(cancelled["status"], "revoked");
    let cancelled_accept = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "folder",
            "accept",
            pending_invitation["id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        !cancelled_accept.status.success(),
        "a revoked pending Folder Invitation must not be acceptable"
    );
    let guest_metadata = run(
        &home_b,
        &home_b,
        &["brain", "metadata", "--brain", "acme", "--json"],
    );
    assert!(guest_metadata.status.success());
    let guest_metadata: Value = serde_json::from_slice(&guest_metadata.stdout).unwrap();
    assert!(
        guest_metadata["members"]
            .as_array()
            .unwrap()
            .iter()
            .all(|member| member != &target_npub)
    );
    assert!(
        guest_metadata["guests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|guest| guest == &target_npub)
    );
    let tree_b = home_b.join("tree-b");
    let guest_open = run(
        &home_b,
        &home_b,
        &["open", "acme", tree_b.to_str().unwrap(), "--json"],
    );
    assert!(
        guest_open.status.success(),
        "{}",
        String::from_utf8_lossy(&guest_open.stderr)
    );
    let guest_sync = run(&home_b, &tree_b, &["sync", "now", "--json"]);
    assert!(
        guest_sync.status.success(),
        "{}",
        String::from_utf8_lossy(&guest_sync.stderr)
    );
    assert_eq!(
        fs::read_to_string(tree_b.join("Admin Only/admin.md")).unwrap(),
        "# Admin Only\n\nExplicit Guest invitation proof.\n"
    );
    assert!(!tree_b.join("Restricted/secret.md").exists());
    assert!(!tree_b.join("Unrelated/other.md").exists());
    fs::write(
        tree_b.join("Admin Only/guest-edit.md"),
        "# Guest edit\n\nBounded Folder write proof.\n",
    )
    .unwrap();
    let guest_push = run(&home_b, &tree_b, &["sync", "now", "--json"]);
    assert!(
        guest_push.status.success(),
        "{}",
        String::from_utf8_lossy(&guest_push.stderr)
    );
    smoke.pass();

    smoke.enter("npubCollaboration");
    let ensure = run(
        &home_a,
        &tree_a,
        &[
            "collaborator",
            "ensure-admin",
            "--target",
            &target_npub,
            "--json",
        ],
    );
    assert!(
        ensure.status.success(),
        "{}",
        String::from_utf8_lossy(&ensure.stderr)
    );
    let receipt: Value = serde_json::from_slice(&ensure.stdout).unwrap();
    assert_eq!(receipt["state"], "complete", "{receipt}");
    smoke.pass();

    smoke.enter("betaOpenAndRead");
    let synced_b = run(&home_b, &tree_b, &["sync", "now", "--json"]);
    assert!(
        synced_b.status.success(),
        "{}",
        String::from_utf8_lossy(&synced_b.stderr)
    );
    assert_eq!(
        fs::read_to_string(tree_b.join("Restricted/secret.md")).unwrap(),
        "# Restricted\n\nRecipient-readable proof.\n"
    );
    let org_instructions = fs::read_to_string(tree_b.join("AGENTS.md")).unwrap();
    assert!(org_instructions.contains("FiniteBrain Organization Brain Working Tree"));
    assert!(org_instructions.contains(&format!("Acting Member Identity: `{target_npub}`")));
    assert!(org_instructions.contains("Acting Brain role: `admin`"));
    assert_eq!(
        assert_canonical_folder_projection(&tree_b.join("Restricted"), "restricted"),
        restricted_instructions
    );
    smoke.pass();

    smoke.enter("betaEditAndSync");
    fs::write(
        tree_b.join("Restricted/beta-edit.md"),
        "# Beta edit\n\nBeta collaboration write proof.\n",
    )
    .unwrap();
    let asset_source_note = "---\ntype: source\ntitle: Shared demo PDF\nresource: https://docs.example.test/shared-demo.pdf\ndescription: Canonical external demo source.\nfinite_asset:\n  content_type: application/pdf\n---\n\n# Shared demo PDF\n\nThe bytes remain at the canonical resource.\n";
    fs::write(
        tree_b.join("Restricted/raw/shared-demo-pdf.md"),
        asset_source_note,
    )
    .unwrap();
    let beta_push = run(&home_b, &tree_b, &["sync", "now", "--json"]);
    assert!(
        beta_push.status.success(),
        "{}",
        String::from_utf8_lossy(&beta_push.stderr)
    );
    smoke.pass();

    smoke.enter("alphaSyncAndObserve");
    let alpha_pull = run(&home_a, &tree_a, &["sync", "now", "--json"]);
    assert!(
        alpha_pull.status.success(),
        "{}",
        String::from_utf8_lossy(&alpha_pull.stderr)
    );
    assert_eq!(
        fs::read_to_string(tree_a.join("Restricted/beta-edit.md")).unwrap(),
        "# Beta edit\n\nBeta collaboration write proof.\n"
    );
    assert_eq!(
        fs::read_to_string(tree_a.join("Restricted/raw/shared-demo-pdf.md")).unwrap(),
        asset_source_note
    );
    assert_eq!(
        fs::read_to_string(tree_a.join("Admin Only/guest-edit.md")).unwrap(),
        "# Guest edit\n\nBounded Folder write proof.\n"
    );
    smoke.pass();

    smoke.enter("personalDiscoveryAndBrainInvitation");
    let opened_personal = run(&home_a, &home_a, &["open", "personal", "--json"]);
    assert!(
        opened_personal.status.success(),
        "{}",
        String::from_utf8_lossy(&opened_personal.stderr)
    );
    let opened_personal: Value = serde_json::from_slice(&opened_personal.stdout).unwrap();
    assert_eq!(opened_personal["brainId"], "personal-a");
    assert_eq!(
        opened_personal["nextCommandWorkingDirectory"],
        opened_personal["path"]
    );
    let tree_personal_a = PathBuf::from(
        opened_personal["path"]
            .as_str()
            .expect("personal Working Tree path"),
    );
    for (id, access, name) in [
        ("personal-team", "all_members", "Personal Team"),
        ("personal-private", "restricted", "Personal Private"),
    ] {
        let created = run(
            &home_a,
            &tree_personal_a,
            &[
                "folder", "create", id, "--access", access, "--name", name, "--path", name,
                "--json",
            ],
        );
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
    }
    let personal_owner_folder = run(
        &home_a,
        &tree_personal_a,
        &["folder", "create", "Personal Owner", "--json"],
    );
    assert!(
        personal_owner_folder.status.success(),
        "{}",
        String::from_utf8_lossy(&personal_owner_folder.stderr)
    );
    let personal_owner_folder: Value =
        serde_json::from_slice(&personal_owner_folder.stdout).unwrap();
    let personal_owner_folder = personal_owner_folder["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|folder| folder["id"] == "personal-owner")
        .unwrap();
    assert_eq!(personal_owner_folder["path"], "Personal Owner");
    assert_eq!(personal_owner_folder["access"], "owner");
    let synced_personal_folders = run(&home_a, &tree_personal_a, &["sync", "now", "--json"]);
    assert!(
        synced_personal_folders.status.success(),
        "{}",
        String::from_utf8_lossy(&synced_personal_folders.stderr)
    );
    assert_canonical_folder_projection(&tree_personal_a.join("Personal Team"), "personal-team");
    let personal_instructions = fs::read_to_string(tree_personal_a.join("AGENTS.md")).unwrap();
    assert!(personal_instructions.contains("FiniteBrain Personal Brain Working Tree"));
    assert!(personal_instructions.contains(&format!("Acting Member Identity: `{owner_npub}`")));
    assert!(personal_instructions.contains("Acting Brain role: `owner`"));
    assert_canonical_folder_projection(
        &tree_personal_a.join("Personal Private"),
        "personal-private",
    );
    assert_canonical_folder_projection(&tree_personal_a.join("Personal Owner"), "personal-owner");
    let personal_folder_invitation = run(
        &home_a,
        &tree_personal_a.join("Personal Team"),
        &[
            "invite",
            "folder",
            "create",
            "--target",
            &target_npub,
            "--json",
        ],
    );
    assert!(
        personal_folder_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&personal_folder_invitation.stderr)
    );
    let personal_folder_invitation: Value =
        serde_json::from_slice(&personal_folder_invitation.stdout).unwrap();
    let accepted_personal_folder = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "folder",
            "accept",
            personal_folder_invitation["id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        accepted_personal_folder.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted_personal_folder.stderr)
    );
    let opened_personal_b = run(&home_b, &home_b, &["open", "personal-a", "--json"]);
    assert!(
        opened_personal_b.status.success(),
        "{}",
        String::from_utf8_lossy(&opened_personal_b.stderr)
    );
    let opened_personal_b: Value = serde_json::from_slice(&opened_personal_b.stdout).unwrap();
    let tree_personal_b = PathBuf::from(
        opened_personal_b["path"]
            .as_str()
            .expect("recipient Personal tree"),
    );
    let guest_metadata = run(&home_b, &tree_personal_b, &["brain", "metadata", "--json"]);
    assert!(
        guest_metadata.status.success(),
        "{}",
        String::from_utf8_lossy(&guest_metadata.stderr)
    );
    let guest_metadata: Value = serde_json::from_slice(&guest_metadata.stdout).unwrap();
    assert!(
        !guest_metadata["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|member| member == &target_npub)
    );
    assert!(
        guest_metadata["guests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|guest| guest == &target_npub)
    );
    assert!(
        guest_metadata["folders"]
            .as_array()
            .unwrap()
            .iter()
            .any(|folder| folder["id"] == "personal-team")
    );
    assert!(
        !guest_metadata["folders"]
            .as_array()
            .unwrap()
            .iter()
            .any(|folder| folder["id"] == "personal-private"),
        "a Folder Guest must not inherit unrelated Personal Brain folders"
    );
    let brain_invitation = run(
        &home_a,
        &tree_personal_a,
        &[
            "invite",
            "brain",
            "create",
            "--target",
            &target_npub,
            "--json",
        ],
    );
    assert!(
        brain_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&brain_invitation.stderr)
    );
    let brain_invitation: Value = serde_json::from_slice(&brain_invitation.stdout).unwrap();
    let brain_invitation_id = brain_invitation["id"].as_str().unwrap();
    let invitee_pending = run(&home_b, &home_b, &["invite", "brain", "list", "--json"]);
    assert!(
        invitee_pending.status.success(),
        "{}",
        String::from_utf8_lossy(&invitee_pending.stderr)
    );
    let invitee_pending: Value = serde_json::from_slice(&invitee_pending.stdout).unwrap();
    let invitee_pending_entries = invitee_pending["invitations"].as_array().unwrap();
    assert!(
        invitee_pending_entries
            .iter()
            .any(|invitation| invitation["id"] == brain_invitation_id),
        "an invitee without a Working Tree must see pending invitations addressed to them"
    );
    assert!(
        invitee_pending_entries
            .iter()
            .all(|invitation| invitation["brainDisplayName"].is_string()
                && invitation["expiresAt"].is_string()
                && invitation["inviteCode"].is_string()),
        "the invitee list must carry the fields needed to accept"
    );
    let accepted_brain = run(
        &home_b,
        &home_b,
        &["invite", "brain", "accept", brain_invitation_id, "--json"],
    );
    assert!(
        accepted_brain.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted_brain.stderr)
    );
    let create_invitation_org = run(
        &requester_home,
        &requester_home,
        &[
            "brain",
            "create",
            "organization",
            "Invitation Org",
            "--json",
        ],
    );
    assert!(
        create_invitation_org.status.success(),
        "{}",
        String::from_utf8_lossy(&create_invitation_org.stderr)
    );
    let org_brain_invitation = run(
        &requester_home,
        &requester_home,
        &[
            "invite",
            "brain",
            "create",
            "--brain",
            "invitation-org",
            "--target",
            &target_npub,
            "--json",
        ],
    );
    assert!(
        org_brain_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&org_brain_invitation.stderr)
    );
    let org_brain_invitation: Value = serde_json::from_slice(&org_brain_invitation.stdout).unwrap();
    let accepted_org_brain = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "brain",
            "accept",
            org_brain_invitation["id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        accepted_org_brain.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted_org_brain.stderr)
    );
    let invitation_org_tree = requester_home.join("invitation-org-tree");
    let opened_invitation_org = run(
        &requester_home,
        &requester_home,
        &[
            "open",
            "invitation-org",
            invitation_org_tree.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        opened_invitation_org.status.success(),
        "{}",
        String::from_utf8_lossy(&opened_invitation_org.stderr)
    );
    for name in ["Member Scope", "Member Unrelated"] {
        let created = run(
            &requester_home,
            &invitation_org_tree,
            &["folder", "create", name, "--json"],
        );
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let created: Value = serde_json::from_slice(&created.stdout).unwrap();
        let created_folder = created["folders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|folder| folder["name"] == name)
            .unwrap();
        assert_eq!(created_folder["access"], "restricted");
    }
    let member_folders_initial_sync = run(
        &requester_home,
        &invitation_org_tree,
        &["sync", "now", "--json"],
    );
    assert!(
        member_folders_initial_sync.status.success(),
        "{}",
        String::from_utf8_lossy(&member_folders_initial_sync.stderr)
    );
    fs::write(
        invitation_org_tree.join("Member Scope/invited.md"),
        "# Member Scope\n\nInvited Member access proof.\n",
    )
    .unwrap();
    fs::write(
        invitation_org_tree.join("Member Unrelated/private.md"),
        "# Member Unrelated\n\nMust remain unreadable.\n",
    )
    .unwrap();
    let member_folders_content_sync = run(
        &requester_home,
        &invitation_org_tree,
        &["sync", "now", "--json"],
    );
    assert!(
        member_folders_content_sync.status.success(),
        "{}",
        String::from_utf8_lossy(&member_folders_content_sync.stderr)
    );
    let member_folder_invitation = run(
        &requester_home,
        &invitation_org_tree.join("Member Scope"),
        &[
            "invite",
            "folder",
            "create",
            "--target",
            &target_npub,
            "--json",
        ],
    );
    assert!(
        member_folder_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&member_folder_invitation.stderr)
    );
    let member_folder_invitation: Value =
        serde_json::from_slice(&member_folder_invitation.stdout).unwrap();
    let claimed_member_invitation = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "folder",
            "accept",
            member_folder_invitation["id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        claimed_member_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&claimed_member_invitation.stderr)
    );
    let member_metadata = run(
        &home_b,
        &home_b,
        &["brain", "metadata", "--brain", "invitation-org", "--json"],
    );
    assert!(
        member_metadata.status.success(),
        "{}",
        String::from_utf8_lossy(&member_metadata.stderr)
    );
    let member_metadata: Value = serde_json::from_slice(&member_metadata.stdout).unwrap();
    assert!(
        member_metadata["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|member| member == &target_npub)
    );
    assert!(
        member_metadata["guests"]
            .as_array()
            .unwrap()
            .iter()
            .all(|guest| guest != &target_npub)
    );
    assert!(
        member_metadata["folders"]
            .as_array()
            .unwrap()
            .iter()
            .any(|folder| folder["id"] == "member-scope")
    );
    let member_tree = home_b.join("invitation-org-member-tree");
    let opened_member_tree = run(
        &home_b,
        &home_b,
        &[
            "open",
            "invitation-org",
            member_tree.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        opened_member_tree.status.success(),
        "{}",
        String::from_utf8_lossy(&opened_member_tree.stderr)
    );
    let synced_member_tree = run(&home_b, &member_tree, &["sync", "now", "--json"]);
    assert!(
        synced_member_tree.status.success(),
        "{}",
        String::from_utf8_lossy(&synced_member_tree.stderr)
    );
    assert_eq!(
        fs::read_to_string(member_tree.join("Member Scope/invited.md")).unwrap(),
        "# Member Scope\n\nInvited Member access proof.\n"
    );
    let member_instructions = fs::read_to_string(member_tree.join("AGENTS.md")).unwrap();
    assert!(member_instructions.contains("FiniteBrain Organization Brain Working Tree"));
    assert!(member_instructions.contains(&format!("Acting Member Identity: `{target_npub}`")));
    assert!(member_instructions.contains("Acting Brain role: `member`"));
    assert!(
        !member_tree.join("Member Unrelated/private.md").exists(),
        "an existing Member must not receive an unrelated restricted Folder key grant"
    );
    smoke.pass();

    smoke.enter("mountOfferParticipantWriteAndRevoke");
    let mount_offer = run(
        &home_a,
        &tree_a.join("Restricted"),
        &[
            "mount",
            "offer",
            "create",
            "--destination-brain",
            "personal-a",
            "--destination-controller",
            &owner_npub,
            "--json",
        ],
    );
    assert!(
        mount_offer.status.success(),
        "{}",
        String::from_utf8_lossy(&mount_offer.stderr)
    );
    let mount_offer: Value = serde_json::from_slice(&mount_offer.stdout).unwrap();
    let initial_participants = mount_offer["initialParticipantNpubs"].as_array().unwrap();
    assert_eq!(initial_participants.len(), 2);
    assert!(
        initial_participants
            .iter()
            .any(|value| value == &owner_npub)
    );
    assert!(
        initial_participants
            .iter()
            .any(|value| value == &personal_agent_npub)
    );
    let mount_offer_id = mount_offer["id"].as_str().unwrap();
    let accepted_mount = run(
        &home_a,
        &tree_personal_a,
        &["mount", "accept", mount_offer_id, "--json"],
    );
    assert!(
        accepted_mount.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted_mount.stderr)
    );
    let accepted_mount: Value = serde_json::from_slice(&accepted_mount.stdout).unwrap();
    let mount_id = accepted_mount["mountId"].as_str().unwrap();
    let added_participant = run(
        &home_a,
        &tree_personal_a,
        &[
            "mount",
            "participant",
            "add",
            mount_id,
            &target_npub,
            "--json",
        ],
    );
    assert!(
        added_participant.status.success(),
        "{}",
        String::from_utf8_lossy(&added_participant.stderr)
    );
    let personal_member_metadata = run(
        &home_b,
        &tree_personal_b,
        &["brain", "metadata", "--brain", "personal-a", "--json"],
    );
    assert!(
        personal_member_metadata.status.success(),
        "{}",
        String::from_utf8_lossy(&personal_member_metadata.stderr)
    );
    let personal_member_metadata: Value =
        serde_json::from_slice(&personal_member_metadata.stdout).unwrap();
    assert!(
        personal_member_metadata["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|member| member == &target_npub)
    );
    let source_member_metadata = run(
        &home_b,
        &tree_personal_b,
        &["brain", "metadata", "--brain", "acme", "--json"],
    );
    assert!(
        source_member_metadata.status.success(),
        "{}",
        String::from_utf8_lossy(&source_member_metadata.stderr)
    );

    let synced_personal_b = run(&home_b, &tree_personal_b, &["sync", "now", "--json"]);
    assert!(
        synced_personal_b.status.success(),
        "{}",
        String::from_utf8_lossy(&synced_personal_b.stderr)
    );
    let tree_state: Value = serde_json::from_slice(
        &fs::read(tree_personal_b.join(".finitebrain/working-tree-state.json")).unwrap(),
    )
    .unwrap();
    let mounted_path = tree_state["folderRoots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|folder| folder["sourceBrainId"] == "acme")
        .and_then(|folder| folder["path"].as_str())
        .unwrap_or_else(|| panic!("mounted source Folder path missing from {tree_state}"));
    let mounted_root = tree_personal_b.join(mounted_path);
    assert_eq!(
        assert_canonical_folder_projection(&mounted_root, "restricted"),
        restricted_instructions
    );
    fs::write(
        mounted_root.join("beta-mounted-edit.md"),
        "# Mounted edit\n\nBeta source-backed write proof.\n",
    )
    .unwrap();
    let beta_mount_push = run(&home_b, &tree_personal_b, &["sync", "now", "--json"]);
    assert!(
        beta_mount_push.status.success(),
        "{}",
        String::from_utf8_lossy(&beta_mount_push.stderr)
    );
    let alpha_mount_pull = run(&home_a, &tree_a, &["sync", "now", "--json"]);
    assert!(
        alpha_mount_pull.status.success(),
        "{}",
        String::from_utf8_lossy(&alpha_mount_pull.stderr)
    );
    assert_eq!(
        fs::read_to_string(tree_a.join("Restricted/beta-mounted-edit.md")).unwrap(),
        "# Mounted edit\n\nBeta source-backed write proof.\n"
    );
    let removed_participant = run(
        &home_a,
        &tree_personal_a,
        &[
            "mount",
            "participant",
            "remove",
            mount_id,
            &personal_agent_npub,
            "--json",
        ],
    );
    assert!(
        removed_participant.status.success(),
        "{}",
        String::from_utf8_lossy(&removed_participant.stderr)
    );
    let revoked_mount = run(
        &home_a,
        &tree_personal_a,
        &["mount", "revoke", mount_id, "--json"],
    );
    assert!(
        revoked_mount.status.success(),
        "{}",
        String::from_utf8_lossy(&revoked_mount.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&revoked_mount.stdout).unwrap()["status"],
        "revoked"
    );
    let beta_after_mount_revoke = run(&home_b, &tree_b, &["sync", "now", "--json"]);
    assert!(
        beta_after_mount_revoke.status.success(),
        "{}",
        String::from_utf8_lossy(&beta_after_mount_revoke.stderr)
    );
    assert_eq!(
        fs::read_to_string(tree_b.join("Restricted/secret.md")).unwrap(),
        "# Restricted\n\nRecipient-readable proof.\n"
    );
    smoke.pass();

    shutdown.send(()).unwrap();
    server_thread.join().unwrap();
    smoke.complete();
}

fn spawn_provider(expected_requests: usize) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let started = Instant::now();
        let mut captured = Vec::new();
        while captured.len() < expected_requests && started.elapsed() < Duration::from_secs(10) {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0_u8; 4096];
                let bytes = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..bytes]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                assert!(headers.starts_with("POST /v1/embeddings "));
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer process-token")
                );
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap();
                if request.len() < header_end + 4 + length {
                    continue;
                }
                let body: Value =
                    serde_json::from_slice(&request[header_end + 4..header_end + 4 + length])
                        .unwrap();
                let vectors = body["inputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|input| json!({ "id": input["id"], "embedding": [1.0, 0.0, 0.0] }))
                    .collect::<Vec<_>>();
                captured.push(body);
                let response = json!({
                    "model": "process-embed",
                    "modelVersion": "process-embed-v1",
                    "dimensions": 3,
                    "vectors": vectors
                })
                .to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                )
                .unwrap();
                break;
            }
        }
        captured
    });
    (endpoint, worker)
}

fn read_provider_request(stream: &mut std::net::TcpStream) -> Value {
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let bytes = stream.read(&mut chunk).unwrap();
        request.extend_from_slice(&chunk[..bytes]);
        let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("authorization: bearer process-token")
        );
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap();
        if request.len() >= header_end + 4 + length {
            return serde_json::from_slice(&request[header_end + 4..header_end + 4 + length])
                .unwrap();
        }
    }
}

fn write_provider_response(stream: &mut std::net::TcpStream, request: &Value, model_version: &str) {
    let vectors = request["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|input| json!({ "id": input["id"], "embedding": [1.0, 0.0, 0.0] }))
        .collect::<Vec<_>>();
    let response = json!({
        "model": "process-embed",
        "modelVersion": model_version,
        "dimensions": 3,
        "vectors": vectors
    })
    .to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    )
    .unwrap();
}

fn read_http_request_line(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let bytes = stream.read(&mut chunk).unwrap();
        assert!(
            bytes > 0,
            "HTTP peer closed before sending complete headers"
        );
        request.extend_from_slice(&chunk[..bytes]);
        if request.windows(4).any(|part| part == b"\r\n\r\n") {
            return String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap()
                .to_owned();
        }
    }
}

fn spawn_access_loss_sync_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let started = Instant::now();
        let mut requests = Vec::new();
        while requests.len() < 2 && started.elapsed() < Duration::from_secs(10) {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            let request_line = read_http_request_line(&mut stream);
            let body = if request_line.contains("/export") {
                json!({
                    "brain": {
                        "id": "brain",
                        "kind": "personal",
                        "name": "Brain",
                        "ownerUserId": null
                    },
                    "folders": [
                        {
                            "id": "general",
                            "path": "General",
                            "access": "owner",
                            "currentKeyVersion": 1,
                            "accessible": false
                        },
                        {
                            "id": "locked",
                            "path": "Locked",
                            "access": "owner",
                            "currentKeyVersion": 1,
                            "accessible": false
                        }
                    ],
                    "keyGrants": [],
                    "accessState": { "members": [], "admins": [] }
                })
                .to_string()
            } else if request_line.contains("/sync/records") {
                json!({
                    "brainId": "brain",
                    "afterSequence": 0,
                    "latestSequence": 0,
                    "records": [],
                    "count": 0,
                    "hasMore": false,
                    "nextSequence": 0
                })
                .to_string()
            } else {
                panic!("unexpected access-loss sync request: {request_line}");
            };
            requests.push(request_line);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        requests
    });
    (endpoint, worker)
}

fn spawn_brain_access_revoked_server() -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request_line = read_http_request_line(&mut stream);
        let body = r#"{"error":"brain access required"}"#;
        write!(
            stream,
            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        request_line
    });
    (endpoint, worker)
}

fn large_bootstrap_markdown_fixture() -> (Value, String) {
    let keys =
        Keys::parse("0000000000000000000000000000000000000000000000000000000000000001").unwrap();
    let recipient = NostrPublicKey::from_protocol(keys.public_key());
    let recipient_npub = recipient.to_npub().unwrap();
    let folder_key = FolderKey::from_bytes([11; 32]);
    let grant_payload = FolderKeyGrantPayload {
        version: "finite-folder-key-grant-v1".to_owned(),
        brain_id: "brain".to_owned(),
        folder_id: "general".to_owned(),
        key_version: 1,
        folder_key: folder_key.to_base64(),
        issuer_npub: recipient_npub.clone(),
        recipient_npub: recipient_npub.clone(),
        created_at: "2026-07-22T18:00:00Z".to_owned(),
    };
    let rumor = build_rumor(
        NostrPublicKey::from_protocol(keys.public_key()),
        Kind::Custom(30_101),
        Vec::new(),
        grant_payload.canonical_json(),
        1_753_206_400,
    );
    let wrapped = wrap_rumor(&keys, recipient, rumor).unwrap();
    let grant = json!({
        "folderId": "general",
        "keyVersion": 1,
        "issuerNpub": recipient_npub,
        "recipientNpub": grant_payload.recipient_npub,
        "wrappedEventJson": wrapped.as_json()
    });
    let object_id = ObjectId::new("obj_largebootstrap1").unwrap();
    let plaintext = json!({
        "version": "finite-folder-object-page-v1",
        "path": "remote-large-bootstrap.md",
        "markdown": "# Remote large bootstrap\n\nDelivered inside the oversized bootstrap response.\n"
    })
    .to_string();
    let envelope = encrypt_folder_object(
        &folder_key,
        &FolderObjectAad {
            brain_id: BrainId::new("brain").unwrap(),
            folder_id: FolderId::new("general").unwrap(),
            object_id,
            key_version: 1,
        },
        plaintext,
    )
    .unwrap();
    (grant, envelope.canonical_json())
}

fn spawn_large_bootstrap_sync_server(
    export_grant: Value,
    ciphertext: String,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let started = Instant::now();
        let mut requests = Vec::new();
        while requests.len() < 3 && started.elapsed() < Duration::from_secs(10) {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            let request_line = read_http_request_line(&mut stream);
            let (status, body) = if request_line.contains("/export") {
                (
                    "200 OK",
                    json!({
                        "brain": {
                            "id": "brain",
                            "kind": "personal",
                            "name": "Brain",
                            "ownerUserId": null
                        },
                        "folders": [{
                            "id": "general",
                            "path": "General",
                            "access": "owner",
                            "currentKeyVersion": 1,
                            "accessible": true
                        }],
                        "keyGrants": [export_grant],
                        "accessState": { "members": [], "admins": [] }
                    })
                    .to_string(),
                )
            } else if request_line.contains("/sync/records") {
                (
                    "410 Gone",
                    json!({ "error": "rebootstrap required from retention floor 1" }).to_string(),
                )
            } else if request_line.contains("/sync/bootstrap") {
                let padding = "x".repeat(11 * 1024 * 1024);
                (
                    "200 OK",
                    json!({
                        "latestSequence": 9,
                        "objects": [{
                            "folderId": "general",
                            "objectId": "obj_largebootstrap1",
                            "revision": 1,
                            "ciphertext": ciphertext,
                            "deleted": false
                        }],
                        "forwardCompatiblePadding": padding
                    })
                    .to_string(),
                )
            } else {
                panic!("unexpected large-bootstrap sync request: {request_line}");
            };
            requests.push(request_line);
            let headers = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).unwrap();
            stream.write_all(body.as_bytes()).unwrap();
        }
        requests
    });
    (endpoint, worker)
}

fn spawn_oversize_bootstrap_sync_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let started = Instant::now();
        let mut requests = Vec::new();
        while requests.len() < 3 && started.elapsed() < Duration::from_secs(10) {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            let request_line = read_http_request_line(&mut stream);
            requests.push(request_line.clone());
            if request_line.contains("/export") {
                let body = json!({
                    "brain": {
                        "id": "brain",
                        "kind": "personal",
                        "name": "Brain",
                        "ownerUserId": null
                    },
                    "folders": [],
                    "keyGrants": [],
                    "accessState": { "members": [], "admins": [] }
                })
                .to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            } else if request_line.contains("/sync/records") {
                let body = json!({
                    "error": "rebootstrap required from retention floor 1"
                })
                .to_string();
                write!(
                    stream,
                    "HTTP/1.1 410 Gone\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            } else if request_line.contains("/sync/bootstrap") {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    128 * 1024 * 1024 + 1
                )
                .unwrap();
                break;
            } else {
                panic!("unexpected oversize-bootstrap sync request: {request_line}");
            }
        }
        requests
    });
    (endpoint, worker)
}

enum QueryProviderResponse {
    Malformed,
    RateLimited,
    Delay,
    ModelV1,
    ModelV2,
}

fn spawn_query_provider(response: QueryProviderResponse) -> (String, thread::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_provider_request(&mut stream);
        assert_eq!(request["inputs"][0]["kind"], "query");
        match response {
            QueryProviderResponse::Malformed => stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot-json",
                )
                .unwrap(),
            QueryProviderResponse::RateLimited => stream
                .write_all(
                    b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap(),
            QueryProviderResponse::Delay => thread::sleep(Duration::from_millis(1_500)),
            QueryProviderResponse::ModelV1 => {
                write_provider_response(&mut stream, &request, "process-embed-v1")
            }
            QueryProviderResponse::ModelV2 => {
                write_provider_response(&mut stream, &request, "process-embed-v2")
            }
        }
        request
    });
    (endpoint, worker)
}

fn spawn_held_provider(
    expected_kind: &'static str,
) -> (
    String,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
    thread::JoinHandle<Value>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (seen_tx, seen_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_provider_request(&mut stream);
        assert!(
            request["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|input| input["kind"] == expected_kind)
        );
        seen_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        write_provider_response(&mut stream, &request, "process-embed-v1");
        request
    });
    (endpoint, seen_rx, release_tx, worker)
}

#[test]
fn built_fbrain_process_proves_global_ranking_output_and_safe_fallback() {
    let scratch = TempDir::new().unwrap();
    let tree = setup_tree(&scratch);
    let nested_cwd = tree.join("General/nested");

    let lexical = run(
        scratch.path(),
        &nested_cwd,
        &["search", "cobalt", "--lexical-only", "--json"],
    );
    assert!(
        lexical.status.success(),
        "{}",
        String::from_utf8_lossy(&lexical.stderr)
    );
    assert!(lexical.stderr.is_empty());
    let lexical: Value = serde_json::from_slice(&lexical.stdout).unwrap();
    assert_eq!(lexical["mode"], "lexical");
    assert_eq!(lexical["results"][0]["pagePath"], "nested/strong-a.md");
    assert_eq!(lexical["results"][0]["disposition"], "synced");
    assert_eq!(lexical["results"][1]["pagePath"], "strong-b.md");
    assert_eq!(lexical["results"][1]["disposition"], "conflicted");
    assert_eq!(lexical["results"][2]["pagePath"], "weak.md");

    fs::write(
        tree.join("Research/weak.md"),
        "# Notes\n\nA newly saved multiblue offline edit.\n",
    )
    .unwrap();
    let offline_edit = run(
        scratch.path(),
        &nested_cwd,
        &["search", "multiblue", "--lexical-only", "--json"],
    );
    assert!(
        offline_edit.status.success(),
        "{}",
        String::from_utf8_lossy(&offline_edit.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&offline_edit.stdout).unwrap()["results"][0]["pagePath"],
        "weak.md"
    );

    let hidden = run(
        scratch.path(),
        &tree,
        &["search", "uniquelockedterm", "--json"],
    );
    assert!(hidden.status.success());
    let hidden: Value = serde_json::from_slice(&hidden.stdout).unwrap();
    assert!(hidden["results"].as_array().unwrap().is_empty());

    let removed_before = run(
        scratch.path(),
        &tree,
        &["search", "transientremoved", "--lexical-only", "--json"],
    );
    assert!(removed_before.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&removed_before.stdout).unwrap()["results"][0]["pagePath"],
        "removed.md"
    );
    fs::remove_file(tree.join("General/removed.md")).unwrap();
    let refreshed = run(
        scratch.path(),
        &tree,
        &["search-index", "status", "--folder", "general", "--json"],
    );
    assert!(refreshed.status.success());
    let removed_after = run(
        scratch.path(),
        &tree,
        &["search", "transientremoved", "--lexical-only", "--json"],
    );
    assert!(removed_after.status.success());
    assert!(
        serde_json::from_slice::<Value>(&removed_after.stdout).unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let human = run(scratch.path(), &tree, &["search", "cobalt"]);
    assert!(human.status.success());
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains("General/nested/strong-a.md"));
    assert!(human.contains("[synced; lexical]"));

    let invalid = run(
        scratch.path(),
        &tree,
        &["search", "cobalt", "--limit", "51", "--json"],
    );
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("--limit must be an integer from 1 to 50"),
        "{}",
        String::from_utf8_lossy(&invalid.stderr)
    );

    let restarted = run(
        scratch.path(),
        &tree,
        &["search", "cobalt", "--lexical-only", "--json"],
    );
    assert!(restarted.status.success());
    let restarted: Value = serde_json::from_slice(&restarted.stdout).unwrap();
    assert_eq!(
        restarted["results"],
        json!([lexical["results"][0].clone(), lexical["results"][1].clone()])
    );
}

#[test]
fn built_fbrain_process_uses_provider_and_does_not_repeat_idle_embedding_work() {
    let scratch = TempDir::new().unwrap();
    let tree = setup_tree(&scratch);
    let enabled = run(
        scratch.path(),
        &tree,
        &["search-index", "enable", "--folder", "general", "--json"],
    );
    assert!(
        enabled.status.success(),
        "{}",
        String::from_utf8_lossy(&enabled.stderr)
    );

    let disabled_research = run(
        scratch.path(),
        &tree,
        &["search-index", "disable", "--folder", "research", "--json"],
    );
    assert!(disabled_research.status.success());

    let (endpoint, provider) = spawn_provider(2);
    let mut daemon = command(scratch.path(), &tree);
    daemon
        .env("FBRAIN_EMBEDDING_ENDPOINT", &endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .env("FBRAIN_EMBEDDING_TIMEOUT_SECONDS", "2")
        .args([
            "daemon",
            "watch",
            "--once",
            "--server",
            "http://127.0.0.1:9",
            "--json",
        ]);
    let daemon = daemon.output().unwrap();
    assert!(
        daemon.status.success(),
        "{}",
        String::from_utf8_lossy(&daemon.stderr)
    );

    let status = run(
        scratch.path(),
        &tree,
        &["search-index", "status", "--folder", "general", "--json"],
    );
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["folders"][0]["lifecycle"], "ready", "{status}");

    let mut search = command(scratch.path(), &tree);
    search
        .env("FBRAIN_EMBEDDING_ENDPOINT", &endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .env("FBRAIN_EMBEDDING_TIMEOUT_SECONDS", "2")
        .args(["search", "cobalt", "--json"]);
    let search = search.output().unwrap();
    assert!(
        search.status.success(),
        "{}",
        String::from_utf8_lossy(&search.stderr)
    );
    let report: Value = serde_json::from_slice(&search.stdout).unwrap();
    let captured = provider.join().unwrap();
    assert_eq!(captured.len(), 2, "{captured:?}");
    assert!(
        captured[..1].iter().all(|request| request["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|input| input["kind"] == "section")),
        "{captured:?}"
    );
    let section_wire = captured[0].to_string();
    for forbidden in [
        "general",
        "research",
        "strong-a.md",
        "obj_synced_process_1",
        "process-token",
        "revision",
    ] {
        assert!(!section_wire.contains(forbidden), "{section_wire}");
    }
    assert!(!section_wire.contains("One passing cobalt reference"));
    assert_eq!(captured[1]["inputs"][0]["kind"], "query", "{captured:?}");
    assert_eq!(captured[1]["inputs"][0]["text"], "cobalt");
    assert_eq!(
        report["mode"], "hybrid",
        "report={report} captured={captured:?}"
    );
    assert_eq!(
        report["results"][0]["signals"],
        json!(["lexical", "semantic"])
    );

    let idle = run(
        scratch.path(),
        &tree,
        &[
            "daemon",
            "watch",
            "--max-ticks",
            "2",
            "--poll-ms",
            "10",
            "--remote-poll-ticks",
            "0",
            "--server",
            "http://127.0.0.1:9",
            "--json",
        ],
    );
    assert!(idle.status.success());
    let state: Value =
        serde_json::from_slice(&fs::read(tree.join(".finitebrain/agent-state.json")).unwrap())
            .unwrap();
    assert!(state["activity"].as_array().unwrap().len() <= 256);
}

#[test]
fn built_fbrain_process_falls_back_for_provider_failures_and_recovers() {
    let scratch = TempDir::new().unwrap();
    let tree = setup_tree(&scratch);
    assert!(
        run(
            scratch.path(),
            &tree,
            &["search-index", "disable", "--folder", "research", "--json"],
        )
        .status
        .success()
    );
    let (build_endpoint, build_provider) = spawn_provider(1);
    let mut build = command(scratch.path(), &tree);
    let build = build
        .env("FBRAIN_EMBEDDING_ENDPOINT", &build_endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .args([
            "daemon",
            "watch",
            "--once",
            "--server",
            "http://127.0.0.1:9",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(build_provider.join().unwrap().len(), 1);

    for response in [
        QueryProviderResponse::Malformed,
        QueryProviderResponse::RateLimited,
        QueryProviderResponse::Delay,
    ] {
        let (endpoint, provider) = spawn_query_provider(response);
        let mut search = command(scratch.path(), &tree);
        let search = search
            .env("FBRAIN_EMBEDDING_ENDPOINT", endpoint)
            .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
            .env("FBRAIN_EMBEDDING_TIMEOUT_SECONDS", "1")
            .args(["search", "cobalt", "--json"])
            .output()
            .unwrap();
        assert!(
            search.status.success(),
            "{}",
            String::from_utf8_lossy(&search.stderr)
        );
        assert!(search.stderr.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&search.stdout).unwrap()["mode"],
            "lexical"
        );
        provider.join().unwrap();
    }

    let unavailable_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let unavailable_endpoint = format!("http://{}", unavailable_listener.local_addr().unwrap());
    drop(unavailable_listener);
    let mut unavailable = command(scratch.path(), &tree);
    let unavailable = unavailable
        .env("FBRAIN_EMBEDDING_ENDPOINT", unavailable_endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .args(["search", "cobalt", "--json"])
        .output()
        .unwrap();
    assert!(unavailable.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&unavailable.stdout).unwrap()["mode"],
        "lexical"
    );

    let (recovery_endpoint, recovery_provider) =
        spawn_query_provider(QueryProviderResponse::ModelV1);
    let mut recovered = command(scratch.path(), &tree);
    let recovered = recovered
        .env("FBRAIN_EMBEDDING_ENDPOINT", recovery_endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .args(["search", "cobalt", "--json"])
        .output()
        .unwrap();
    recovery_provider.join().unwrap();
    assert!(recovered.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&recovered.stdout).unwrap()["mode"],
        "hybrid"
    );

    let (changed_endpoint, changed_provider) = spawn_query_provider(QueryProviderResponse::ModelV2);
    let mut changed = command(scratch.path(), &tree);
    let changed = changed
        .env("FBRAIN_EMBEDDING_ENDPOINT", changed_endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .args(["search", "cobalt", "--json"])
        .output()
        .unwrap();
    changed_provider.join().unwrap();
    assert!(changed.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&changed.stdout).unwrap()["mode"],
        "lexical"
    );
    let status = run(
        scratch.path(),
        &tree,
        &["search-index", "status", "--folder", "general", "--json"],
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["folders"][0]["lifecycle"],
        "stale"
    );
}

#[test]
fn built_fbrain_disable_drains_admitted_provider_io_before_returning() {
    let scratch = TempDir::new().unwrap();
    let tree = setup_tree(&scratch);
    assert!(
        run(
            scratch.path(),
            &tree,
            &["search-index", "disable", "--folder", "research", "--json"],
        )
        .status
        .success()
    );
    let (endpoint, seen, release, provider) = spawn_held_provider("section");
    let mut daemon = command(scratch.path(), &tree);
    daemon
        .env("FBRAIN_EMBEDDING_ENDPOINT", endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "daemon",
            "watch",
            "--once",
            "--server",
            "http://127.0.0.1:9",
            "--json",
        ]);
    let daemon = daemon.spawn().unwrap();
    seen.recv_timeout(Duration::from_secs(5)).unwrap();

    let mut disable_command = command(scratch.path(), &tree);
    disable_command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(["search-index", "disable", "--folder", "general", "--json"]);
    let mut disable = disable_command.spawn().unwrap();
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        disable.try_wait().unwrap().is_none(),
        "disable returned before admitted provider I/O drained"
    );
    release.send(()).unwrap();
    let disabled = disable.wait_with_output().unwrap();
    assert!(
        disabled.status.success(),
        "{}",
        String::from_utf8_lossy(&disabled.stderr)
    );

    provider.join().unwrap();
    let daemon = daemon.wait_with_output().unwrap();
    assert!(
        daemon.status.success(),
        "{}",
        String::from_utf8_lossy(&daemon.stderr)
    );
    let disabled: Value = serde_json::from_slice(&disabled.stdout).unwrap();
    assert_eq!(disabled["folders"][0]["enabled"], false);
    assert_eq!(disabled["folders"][0]["currentVectors"], 0);

    let lexical = run(
        scratch.path(),
        &tree,
        &["search", "cobalt", "--folder", "general", "--json"],
    );
    assert!(lexical.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&lexical.stdout).unwrap()["mode"],
        "lexical"
    );
}

#[test]
fn built_fbrain_disable_drains_admitted_query_embedding_before_returning() {
    let scratch = TempDir::new().unwrap();
    let tree = setup_tree(&scratch);
    assert!(
        run(
            scratch.path(),
            &tree,
            &["search-index", "disable", "--folder", "research", "--json"],
        )
        .status
        .success()
    );
    let (build_endpoint, build_provider) = spawn_provider(1);
    let mut build = command(scratch.path(), &tree);
    let built = build
        .env("FBRAIN_EMBEDDING_ENDPOINT", build_endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .args([
            "daemon",
            "watch",
            "--once",
            "--server",
            "http://127.0.0.1:9",
            "--json",
        ])
        .output()
        .unwrap();
    build_provider.join().unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let (endpoint, seen, release, provider) = spawn_held_provider("query");
    let mut search = command(scratch.path(), &tree);
    search
        .env("FBRAIN_EMBEDDING_ENDPOINT", endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(["search", "cobalt", "--json"]);
    let search = search.spawn().unwrap();
    seen.recv_timeout(Duration::from_secs(5)).unwrap();

    let mut disable = command(scratch.path(), &tree);
    disable.stdout(Stdio::piped()).stderr(Stdio::piped()).args([
        "search-index",
        "disable",
        "--folder",
        "general",
        "--json",
    ]);
    let mut disable = disable.spawn().unwrap();
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        disable.try_wait().unwrap().is_none(),
        "disable returned before admitted query embedding drained"
    );
    release.send(()).unwrap();
    let searched = search.wait_with_output().unwrap();
    let disabled = disable.wait_with_output().unwrap();
    provider.join().unwrap();
    assert!(
        searched.status.success(),
        "{}",
        String::from_utf8_lossy(&searched.stderr)
    );
    assert!(
        disabled.status.success(),
        "{}",
        String::from_utf8_lossy(&disabled.stderr)
    );
}

#[test]
fn built_fbrain_full_brain_access_loss_pauses_without_deleting_local_work() {
    let scratch = TempDir::new().unwrap();
    let tree = setup_access_loss_tree(&scratch);
    let preserved = tree.join("General/unsynced-after-revocation.md");
    fs::write(&preserved, "# Preserved\n\nUnsynced local work.\n").unwrap();
    let (endpoint, server) = spawn_brain_access_revoked_server();

    let sync = run(
        scratch.path(),
        &tree,
        &["sync", "now", "--server", &endpoint, "--json"],
    );
    let request = server.join().unwrap();
    assert!(request.contains("/v1/brains/brain/export"), "{request}");
    assert!(!sync.status.success());
    assert!(String::from_utf8_lossy(&sync.stderr).contains("brain access required"));
    assert_eq!(
        fs::read_to_string(&preserved).unwrap(),
        "# Preserved\n\nUnsynced local work.\n"
    );
    assert!(tree.join("General/nested/strong-a.md").is_file());

    let state: Value =
        serde_json::from_slice(&fs::read(tree.join(".finitebrain/agent-state.json")).unwrap())
            .unwrap();
    assert_eq!(state["sync"]["status"], "paused-access-revoked");
    assert!(
        state["activity"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["kind"] == "daemon.access_paused")
    );

    let status = run(scratch.path(), &tree, &["daemon", "status", "--json"]);
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["state"], "paused");
}

#[test]
fn built_fbrain_sync_accepts_bootstrap_larger_than_ureq_default_limit() {
    let scratch = TempDir::new().unwrap();
    let tree = setup_access_loss_tree(&scratch);
    let (grant, ciphertext) = large_bootstrap_markdown_fixture();
    let (endpoint, server) = spawn_large_bootstrap_sync_server(grant, ciphertext);

    let sync = run(
        scratch.path(),
        &tree,
        &["sync", "now", "--server", &endpoint, "--json"],
    );
    let requests = server.join().unwrap();

    assert!(
        sync.status.success(),
        "{}; requests={requests:?}",
        String::from_utf8_lossy(&sync.stderr),
    );
    let report: Value = serde_json::from_slice(&sync.stdout).unwrap();
    assert_eq!(report["status"], "applied-remote-records");
    assert_eq!(report["latestSequence"], 9);
    let state: Value = serde_json::from_slice(
        &fs::read(tree.join(".finitebrain/working-tree-state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["sync"]["latestSequence"], 9);
    assert_eq!(
        fs::read_to_string(tree.join("General/remote-large-bootstrap.md")).unwrap(),
        "# Remote large bootstrap\n\nDelivered inside the oversized bootstrap response.\n"
    );
    assert_eq!(requests.len(), 3);
    assert!(requests[2].contains("/v1/brains/brain/sync/bootstrap"));
}

#[test]
fn built_fbrain_sync_rejects_bootstrap_over_128_mib_without_changing_projection() {
    let scratch = TempDir::new().unwrap();
    let tree = setup_access_loss_tree(&scratch);
    let unresolved_path = tree.join("General/unresolved.md");
    fs::write(&unresolved_path, "# Unresolved local edit\n").unwrap();
    let manifest_path = tree.join(".finitebrain/working-tree-state.json");
    let agent_state_path = tree.join(".finitebrain/agent-state.json");
    let bootstrap_path = tree.join(".finitebrain/encrypted-sync/bootstrap.json");
    let materialized_path = tree.join("General/nested/strong-a.md");
    let cached_bootstrap = json!({ "latestSequence": 0, "objects": [] });
    write_json(&bootstrap_path, &cached_bootstrap);
    #[cfg(unix)]
    fs::set_permissions(&bootstrap_path, fs::Permissions::from_mode(0o600)).unwrap();
    let manifest_before = fs::read(&manifest_path).unwrap();
    let agent_state_before: Value =
        serde_json::from_slice(&fs::read(&agent_state_path).unwrap()).unwrap();
    let bootstrap_before = fs::read(&bootstrap_path).unwrap();
    let materialized_before = fs::read(&materialized_path).unwrap();
    let (endpoint, server) = spawn_oversize_bootstrap_sync_server();

    let sync = run(
        scratch.path(),
        &tree,
        &["sync", "now", "--server", &endpoint, "--json"],
    );
    let requests = server.join().unwrap();

    assert!(!sync.status.success());
    assert!(
        String::from_utf8_lossy(&sync.stderr)
            .contains("response body exceeds the configured 134217728-byte limit"),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);
    let agent_state_after: Value =
        serde_json::from_slice(&fs::read(&agent_state_path).unwrap()).unwrap();
    assert_eq!(
        agent_state_after["conflicts"], agent_state_before["conflicts"],
        "bootstrap rejection must happen before local conflict bookkeeping"
    );
    assert_eq!(fs::read(&bootstrap_path).unwrap(), bootstrap_before);
    assert_eq!(fs::read(&materialized_path).unwrap(), materialized_before);
    assert_eq!(
        fs::read_to_string(&unresolved_path).unwrap(),
        "# Unresolved local edit\n"
    );
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains("/v1/brains/brain/sync/bootstrap"));
}

#[test]
fn built_fbrain_access_loss_drains_provider_io_and_restarts_fail_closed() {
    let scratch = TempDir::new().unwrap();
    let tree = setup_access_loss_tree(&scratch);
    let state_path = tree.join(".finitebrain/working-tree-state.json");

    let enabled = run(
        scratch.path(),
        &tree,
        &["search-index", "enable", "--folder", "general", "--json"],
    );
    assert!(
        enabled.status.success(),
        "{}",
        String::from_utf8_lossy(&enabled.stderr)
    );
    let index_root = tree.join(".finitebrain/search-indexes");
    let general_index_directory = fs::read_dir(&index_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();

    let (provider_endpoint, provider_seen, provider_release, provider) =
        spawn_held_provider("section");
    let mut daemon = command(scratch.path(), &tree);
    daemon
        .env("FBRAIN_EMBEDDING_ENDPOINT", provider_endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "daemon",
            "watch",
            "--once",
            "--server",
            "http://127.0.0.1:9",
            "--json",
        ]);
    let daemon = daemon.spawn().unwrap();
    provider_seen.recv_timeout(Duration::from_secs(5)).unwrap();

    // A corrupt derived index for an unrelated, already unreadable Folder
    // must not prevent the selected readable Folder from losing access.
    let corrupt_unrelated = index_root.join("unrelated-locked-folder");
    fs::create_dir(&corrupt_unrelated).unwrap();
    let corrupt_unrelated_index = corrupt_unrelated.join("index.sqlite3");
    fs::write(&corrupt_unrelated_index, b"not sqlite").unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&corrupt_unrelated, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&corrupt_unrelated_index, fs::Permissions::from_mode(0o600)).unwrap();
    }

    let (sync_endpoint, sync_server) = spawn_access_loss_sync_server();
    let mut sync = command(scratch.path(), &tree);
    sync.stdout(Stdio::piped()).stderr(Stdio::piped()).args([
        "sync",
        "now",
        "--server",
        &sync_endpoint,
        "--json",
    ]);
    let mut sync = sync.spawn().unwrap();
    std::thread::sleep(Duration::from_millis(250));
    if sync.try_wait().unwrap().is_some() {
        let early = sync.wait_with_output().unwrap();
        panic!(
            "access-loss sync returned before admitted provider I/O drained: stdout={} stderr={}",
            String::from_utf8_lossy(&early.stdout),
            String::from_utf8_lossy(&early.stderr)
        );
    }

    provider_release.send(()).unwrap();
    let synced = sync.wait_with_output().unwrap();
    let daemon = daemon.wait_with_output().unwrap();
    provider.join().unwrap();
    let requests = sync_server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        synced.status.success(),
        "{}",
        String::from_utf8_lossy(&synced.stderr)
    );
    assert!(
        daemon.status.success(),
        "{}",
        String::from_utf8_lossy(&daemon.stderr)
    );

    let state: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    let general = state["folderRoots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|folder| folder["folderId"] == "general")
        .unwrap();
    assert_eq!(general["canRead"], false);
    assert_eq!(general["metadataOnly"], true);
    assert!(!general_index_directory.exists());
    assert!(!corrupt_unrelated.exists());
    assert!(tree.join("General/nested/strong-a.md").is_file());
    assert!(tree.join("Locked/hidden.md").is_file());

    // A fresh executable invocation after the transition cannot search the
    // revoked Folder or recreate any plaintext-derived index state.
    let restarted = run(
        scratch.path(),
        &tree.join("General/nested"),
        &["search", "cobalt", "--json"],
    );
    assert!(
        restarted.status.success(),
        "{}",
        String::from_utf8_lossy(&restarted.stderr)
    );
    let restarted: Value = serde_json::from_slice(&restarted.stdout).unwrap();
    assert!(restarted["results"].as_array().unwrap().is_empty());
    assert!(!index_root.exists() || fs::read_dir(&index_root).unwrap().next().is_none());
}

#[test]
fn built_fbrain_access_loss_crash_restarts_fail_closed_and_retries() {
    let scratch = TempDir::new().unwrap();
    let tree = setup_access_loss_tree(&scratch);
    let state_path = tree.join(".finitebrain/working-tree-state.json");
    let enabled = run(
        scratch.path(),
        &tree,
        &["search-index", "enable", "--folder", "general", "--json"],
    );
    assert!(
        enabled.status.success(),
        "{}",
        String::from_utf8_lossy(&enabled.stderr)
    );
    let general_index_directory = fs::read_dir(tree.join(".finitebrain/search-indexes"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();

    let (provider_endpoint, provider_seen, provider_release, provider) =
        spawn_held_provider("section");
    let mut daemon = command(scratch.path(), &tree);
    daemon
        .env("FBRAIN_EMBEDDING_ENDPOINT", provider_endpoint)
        .env("FBRAIN_EMBEDDING_BEARER_TOKEN", "process-token")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "daemon",
            "watch",
            "--once",
            "--server",
            "http://127.0.0.1:9",
            "--json",
        ]);
    let daemon = daemon.spawn().unwrap();
    provider_seen.recv_timeout(Duration::from_secs(5)).unwrap();

    let (sync_endpoint, sync_server) = spawn_access_loss_sync_server();
    let mut sync = command(scratch.path(), &tree);
    sync.stdout(Stdio::piped()).stderr(Stdio::piped()).args([
        "sync",
        "now",
        "--server",
        &sync_endpoint,
        "--json",
    ]);
    let mut sync = sync.spawn().unwrap();
    let revocation_marker = general_index_directory.join("access-revoked");
    let started = Instant::now();
    while !revocation_marker.is_file() && started.elapsed() < Duration::from_secs(3) {
        assert!(
            sync.try_wait().unwrap().is_none(),
            "access-loss sync exited before persisting revocation intent"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(revocation_marker.is_file());
    assert!(sync.try_wait().unwrap().is_none());

    // SIGKILL the real control process at the durable drain boundary. The old
    // readable manifest remains, but the persisted intent must keep a fresh
    // executable from reopening lexical or semantic derived state.
    sync.kill().unwrap();
    let killed = sync.wait_with_output().unwrap();
    assert!(!killed.status.success());
    sync_server.join().unwrap();
    let state: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(state["folderRoots"][0]["canRead"], true);

    provider_release.send(()).unwrap();
    provider.join().unwrap();
    let daemon = daemon.wait_with_output().unwrap();
    assert!(
        daemon.status.success(),
        "{}",
        String::from_utf8_lossy(&daemon.stderr)
    );
    let restarted = run(
        scratch.path(),
        &tree,
        &["search", "cobalt", "--folder", "general", "--json"],
    );
    assert!(!restarted.status.success());
    assert!(general_index_directory.join("index.sqlite3").is_file());

    // Replaying the same public sync resumes the interrupted transition and
    // reaches the normal unreadable/no-derived-state postcondition.
    let (retry_endpoint, retry_server) = spawn_access_loss_sync_server();
    let retried = run(
        scratch.path(),
        &tree,
        &["sync", "now", "--server", &retry_endpoint, "--json"],
    );
    retry_server.join().unwrap();
    assert!(
        retried.status.success(),
        "{}",
        String::from_utf8_lossy(&retried.stderr)
    );
    let state: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(state["folderRoots"][0]["canRead"], false);
    assert_eq!(state["folderRoots"][0]["metadataOnly"], true);
    assert!(!general_index_directory.exists());
}

#[test]
fn built_fbrain_pending_wraps_complete_on_admin_sync_unlock_invited_member() {
    let scratch = TempDir::new().unwrap();
    let home_a = scratch.path().join("home-a");
    let home_b = scratch.path().join("home-b");
    fs::create_dir_all(&home_a).unwrap();
    fs::create_dir_all(&home_b).unwrap();
    for (home, secret, suffix) in [(&home_a, "secret-a", "0001"), (&home_b, "secret-b", "0002")] {
        let secret_path = scratch.path().join(secret);
        fs::write(
            &secret_path,
            format!("000000000000000000000000000000000000000000000000000000000000{suffix}\n"),
        )
        .unwrap();
        assert!(
            run(
                home,
                home,
                &[
                    "auth",
                    "import",
                    "--file",
                    secret_path.to_str().unwrap(),
                    "--json"
                ]
            )
            .status
            .success()
        );
    }
    let npub_of = |home: &Path| {
        let signer = run(home, home, &["signer", "public-key", "--json"]);
        assert!(
            signer.status.success(),
            "{}",
            String::from_utf8_lossy(&signer.stderr)
        );
        let signer: Value = serde_json::from_slice(&signer.stdout).unwrap();
        signer["npub"].as_str().unwrap().to_owned()
    };
    let owner_npub = npub_of(&home_a);
    let target_npub = npub_of(&home_b);
    let personal_agent_keys =
        nostr::Keys::parse("0000000000000000000000000000000000000000000000000000000000000003")
            .unwrap();
    let personal_agent_npub = NostrPublicKey::from_protocol(personal_agent_keys.public_key())
        .to_npub()
        .unwrap();
    let requester_keys =
        nostr::Keys::parse("0000000000000000000000000000000000000000000000000000000000000004")
            .unwrap();
    let requester_npub = NostrPublicKey::from_protocol(requester_keys.public_key())
        .to_npub()
        .unwrap();
    let (server_url, shutdown, server_thread) = spawn_real_brain_server(
        &target_npub,
        &personal_agent_npub,
        &owner_npub,
        &requester_npub,
    );
    let run = |home: &Path, cwd: &Path, args: &[&str]| {
        let now = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
        command(home, cwd)
            .env("FBRAIN_NOW", now)
            .env("FINITE_BRAIN_SERVER_URL", &server_url)
            .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
            .args(args)
            .output()
            .unwrap()
    };

    // The owner opens their Personal Brain, creates a restricted Folder, and
    // publishes content into it.
    let tree_a = home_a.join("personal-a-tree");
    let opened = run(
        &home_a,
        &home_a,
        &["open", "personal-a", tree_a.to_str().unwrap(), "--json"],
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    let created = run(
        &home_a,
        &tree_a,
        &[
            "folder",
            "create",
            "team-folder",
            "--access",
            "restricted",
            "--name",
            "Team Folder",
            "--path",
            "Team Folder",
            "--json",
        ],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let synced = run(&home_a, &tree_a, &["sync", "now", "--json"]);
    assert!(
        synced.status.success(),
        "{}",
        String::from_utf8_lossy(&synced.stderr)
    );
    fs::write(
        tree_a.join("Team Folder/notes.md"),
        "# Team Folder\n\nShared with the invited member.\n",
    )
    .unwrap();
    let pushed = run(&home_a, &tree_a, &["sync", "now", "--json"]);
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );

    // Invite the second home; the invitee accepts and opens the Brain. The
    // historical failure: entitlement without a wrapped Folder Key, so the
    // Folder stays locked until someone wraps for them.
    let invitation = run(
        &home_a,
        &tree_a,
        &[
            "invite",
            "brain",
            "create",
            "--target",
            &target_npub,
            "--folder",
            "team-folder",
            "--json",
        ],
    );
    assert!(
        invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&invitation.stderr)
    );
    let invitation: Value = serde_json::from_slice(&invitation.stdout).unwrap();
    let accepted = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "brain",
            "accept",
            invitation["id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let member_tree = home_b.join("personal-a-member-tree");
    let opened_member = run(
        &home_b,
        &home_b,
        &[
            "open",
            "personal-a",
            member_tree.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        opened_member.status.success(),
        "{}",
        String::from_utf8_lossy(&opened_member.stderr)
    );
    let first_sync = run(&home_b, &member_tree, &["sync", "now", "--json"]);
    assert!(
        first_sync.status.success(),
        "{}",
        String::from_utf8_lossy(&first_sync.stderr)
    );
    assert!(
        !member_tree.join("Team Folder/notes.md").exists(),
        "before any key holder syncs, the invited Folder must still be locked"
    );

    // The owner's metadata surfaces the pending wraps.
    let metadata = run(
        &home_a,
        &tree_a,
        &["brain", "metadata", "--brain", "personal-a", "--json"],
    );
    assert!(
        metadata.status.success(),
        "{}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    let metadata: Value = serde_json::from_slice(&metadata.stdout).unwrap();
    assert!(
        metadata["pendingWraps"].as_array().map_or(0, Vec::len) >= 1,
        "owner metadata must carry pendingWraps: {metadata}"
    );

    // The owner's status surfaces the pending wraps.
    let status = run(&home_a, &tree_a, &["status", "--json"]);
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(
        status["pendingWraps"].as_u64().unwrap_or(0) >= 1,
        "owner status must surface the pending wraps: {status}"
    );

    // The owner's next sync completes the pending wraps opportunistically and
    // notes each one in --summary output.
    let owner_sync = run(&home_a, &tree_a, &["sync", "now", "--summary"]);
    assert!(
        owner_sync.status.success(),
        "{}",
        String::from_utf8_lossy(&owner_sync.stderr)
    );
    let owner_summary = String::from_utf8_lossy(&owner_sync.stdout);
    assert!(
        owner_summary.contains("wrapped grants:"),
        "owner sync summary must note the completed wraps: {owner_summary}"
    );
    assert!(
        owner_summary.contains(&format!("wrapped team-folder key for {target_npub}")),
        "owner sync summary must name the wrapped Folder and recipient: {owner_summary}"
    );

    // The invitee's next sync opens the delivered grant and materializes the
    // previously locked content: the original onboarding failure is dead.
    let second_sync = run(&home_b, &member_tree, &["sync", "now", "--json"]);
    assert!(
        second_sync.status.success(),
        "{}",
        String::from_utf8_lossy(&second_sync.stderr)
    );
    assert_eq!(
        fs::read_to_string(member_tree.join("Team Folder/notes.md")).unwrap(),
        "# Team Folder\n\nShared with the invited member.\n"
    );

    // The markers are gone; owner status no longer carries the field.
    let status = run(&home_a, &tree_a, &["status", "--json"]);
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(
        status.get("pendingWraps").is_none(),
        "completed wraps must clear the admin signal: {status}"
    );

    shutdown.send(()).unwrap();
    server_thread.join().unwrap();
}

/// One step of the invited-member journey observed live in FIN-146.
#[derive(Clone, Copy, Debug)]
enum InviteJourneyStep {
    /// The invited member opens the Brain; open runs the first sync.
    MemberOpens,
    /// The invited member runs an ordinary sync.
    MemberSync,
    /// The owner's ordinary sync wraps the pending Folder Key for the member.
    OwnerDeliversKey,
    /// The owner rewrites the verification marker and appends the log.
    OwnerRevisesPages(&'static str),
    /// The owner creates a second restricted Folder the member has no grant for.
    OwnerCreatesControlFolder,
}

const INVITED_FOLDER_ID: &str = "team-folder";
const CONTROL_FOLDER_ID: &str = "control-folder";

fn verification_page(marker: &str) -> String {
    format!("# Verification\n\nMarker {marker}.\n")
}

/// Owner and invited member Finite Homes against one real Brain server. The
/// member has accepted a Folder-limited member invitation but has not opened
/// the Brain; nobody has wrapped the Folder Key for them yet.
struct InvitedMemberJourney {
    _scratch: TempDir,
    home_a: PathBuf,
    home_b: PathBuf,
    tree_a: PathBuf,
    member_tree: PathBuf,
    target_npub: String,
    server_url: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    server_thread: Option<thread::JoinHandle<()>>,
}

impl InvitedMemberJourney {
    fn start() -> Self {
        let scratch = TempDir::new().unwrap();
        let home_a = scratch.path().join("home-a");
        let home_b = scratch.path().join("home-b");
        let mut npubs = Vec::new();
        for (home, suffix) in [(&home_a, "0001"), (&home_b, "0002")] {
            fs::create_dir_all(home).unwrap();
            let secret_path = home.join("import-key");
            fs::write(
                &secret_path,
                format!("000000000000000000000000000000000000000000000000000000000000{suffix}\n"),
            )
            .unwrap();
            let imported = run(
                home,
                home,
                &[
                    "auth",
                    "import",
                    "--file",
                    secret_path.to_str().unwrap(),
                    "--json",
                ],
            );
            assert!(
                imported.status.success(),
                "{}",
                String::from_utf8_lossy(&imported.stderr)
            );
            fs::remove_file(secret_path).unwrap();
            let signer = run(home, home, &["signer", "public-key", "--json"]);
            assert!(signer.status.success());
            let signer: Value = serde_json::from_slice(&signer.stdout).unwrap();
            npubs.push(signer["npub"].as_str().unwrap().to_owned());
        }
        let fixed_npub = |secret: &str| {
            NostrPublicKey::from_protocol(Keys::parse(secret).unwrap().public_key())
                .to_npub()
                .unwrap()
        };
        let (server_url, shutdown, server_thread) = spawn_real_brain_server(
            &npubs[1],
            &fixed_npub("0000000000000000000000000000000000000000000000000000000000000003"),
            &npubs[0],
            &fixed_npub("0000000000000000000000000000000000000000000000000000000000000004"),
        );
        let journey = Self {
            _scratch: scratch,
            tree_a: home_a.join("personal-a-tree"),
            member_tree: home_b.join("personal-a-member-tree"),
            home_a,
            home_b,
            target_npub: npubs[1].clone(),
            server_url,
            shutdown: Some(shutdown),
            server_thread: Some(server_thread),
        };

        // The owner creates a restricted Folder with three pages and syncs.
        journey.owner_json(&[
            "open",
            "personal-a",
            journey.tree_a.to_str().unwrap(),
            "--json",
        ]);
        journey.owner_json(&[
            "folder",
            "create",
            INVITED_FOLDER_ID,
            "--access",
            "restricted",
            "--name",
            "Team Folder",
            "--path",
            "Team Folder",
            "--json",
        ]);
        journey.owner_json(&["sync", "now", "--json"]);
        let folder = journey.tree_a.join("Team Folder");
        fs::write(folder.join("index.md"), "# Team Folder\n\nInvite test.\n").unwrap();
        fs::write(folder.join("log.md"), "# Log\n\n- created amberorchidone\n").unwrap();
        fs::write(
            folder.join("wiki/verification.md"),
            verification_page("amberorchidone"),
        )
        .unwrap();
        journey.owner_json(&["sync", "now", "--json"]);

        // A member invitation limited to that Folder; the member accepts.
        let invitation = journey.owner_json(&[
            "invite",
            "brain",
            "create",
            "--target",
            &journey.target_npub,
            "--folder",
            INVITED_FOLDER_ID,
            "--json",
        ]);
        let accepted = journey.run(
            &fbrain(),
            &journey.home_b,
            &journey.home_b,
            &[
                "invite",
                "brain",
                "accept",
                invitation["id"].as_str().unwrap(),
                "--json",
            ],
        );
        assert!(
            accepted.status.success(),
            "{}",
            String::from_utf8_lossy(&accepted.stderr)
        );
        journey
    }

    fn run(&self, binary: &Path, home: &Path, cwd: &Path, args: &[&str]) -> Output {
        command_for(binary, home, cwd)
            .env(
                "FBRAIN_NOW",
                OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
            )
            .env("FINITE_BRAIN_SERVER_URL", &self.server_url)
            .env("FINITE_BRAIN_PUBLIC_BASE_URL", &self.server_url)
            .args(args)
            .output()
            .unwrap()
    }

    fn owner_json(&self, args: &[&str]) -> Value {
        let cwd = if self.tree_a.exists() {
            &self.tree_a
        } else {
            &self.home_a
        };
        let output = self.run(&fbrain(), &self.home_a, cwd, args);
        assert!(
            output.status.success(),
            "owner {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or(Value::Null)
    }

    fn member_open(&self, binary: &Path, tree: &Path) {
        let opened = self.run(
            binary,
            &self.home_b,
            &self.home_b,
            &["open", "personal-a", tree.to_str().unwrap(), "--json"],
        );
        assert!(
            opened.status.success(),
            "{}",
            String::from_utf8_lossy(&opened.stderr)
        );
    }

    fn member_sync(&self, binary: &Path, tree: &Path) -> Value {
        let synced = self.run(binary, &self.home_b, tree, &["sync", "now", "--json"]);
        assert!(
            synced.status.success(),
            "{}",
            String::from_utf8_lossy(&synced.stderr)
        );
        serde_json::from_slice(&synced.stdout).unwrap()
    }

    fn step(&self, member_binary: &Path, step: InviteJourneyStep) {
        match step {
            InviteJourneyStep::MemberOpens => {
                self.member_open(member_binary, &self.member_tree);
            }
            InviteJourneyStep::MemberSync => {
                self.member_sync(member_binary, &self.member_tree);
            }
            InviteJourneyStep::OwnerDeliversKey => {
                let owner_sync = self.run(
                    &fbrain(),
                    &self.home_a,
                    &self.tree_a,
                    &["sync", "now", "--summary"],
                );
                assert!(
                    owner_sync.status.success(),
                    "{}",
                    String::from_utf8_lossy(&owner_sync.stderr)
                );
                let summary = String::from_utf8_lossy(&owner_sync.stdout);
                assert!(
                    summary.contains(&format!(
                        "wrapped {INVITED_FOLDER_ID} key for {}",
                        self.target_npub
                    )),
                    "the owner's ordinary sync must wrap the pending key: {summary}"
                );
            }
            InviteJourneyStep::OwnerRevisesPages(marker) => {
                let folder = self.tree_a.join("Team Folder");
                fs::write(
                    folder.join("wiki/verification.md"),
                    verification_page(marker),
                )
                .unwrap();
                let mut log = fs::read_to_string(folder.join("log.md")).unwrap();
                log.push_str(&format!("- revised to {marker}\n"));
                fs::write(folder.join("log.md"), log).unwrap();
                let pushed = self.owner_json(&["sync", "now", "--json"]);
                assert_eq!(pushed["conflicts"], json!([]), "owner revision: {pushed}");
                assert_eq!(
                    pushed["localChanges"].as_array().map(Vec::len),
                    Some(2),
                    "owner revision: {pushed}"
                );
            }
            InviteJourneyStep::OwnerCreatesControlFolder => {
                self.owner_json(&[
                    "folder",
                    "create",
                    CONTROL_FOLDER_ID,
                    "--access",
                    "restricted",
                    "--name",
                    "Control Folder",
                    "--path",
                    "Control Folder",
                    "--json",
                ]);
                self.owner_json(&["sync", "now", "--json"]);
                fs::write(
                    self.tree_a.join("Control Folder/wiki/control-page.md"),
                    "# Control\n\nMarker controlonly.\n",
                )
                .unwrap();
                self.owner_json(&["sync", "now", "--json"]);
            }
        }
    }

    fn tree_state(tree: &Path) -> Value {
        serde_json::from_slice(
            &fs::read(tree.join(".finitebrain/working-tree-state.json")).unwrap(),
        )
        .unwrap()
    }

    fn invited_root(state: &Value) -> Value {
        state["folderRoots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|root| root["folderId"] == INVITED_FOLDER_ID)
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn invited_revisions(objects: &Value) -> std::collections::BTreeMap<String, u64> {
        objects
            .as_array()
            .unwrap()
            .iter()
            .filter(|object| object["folderId"] == INVITED_FOLDER_ID && object["deleted"] != true)
            .map(|object| {
                (
                    object["objectId"].as_str().unwrap().to_owned(),
                    object["revision"].as_u64().unwrap(),
                )
            })
            .collect()
    }

    /// The member reads the current revision of every invited page, finds the
    /// current marker through lexical search, and has nothing of the control
    /// Folder. Cached plaintext alone never satisfies this: the manifest
    /// revisions must match the owner's authoritative export.
    fn assert_member_reads_current_folder(&self, tree: &Path, moment: &str) {
        let state = Self::tree_state(tree);
        let invited = Self::invited_root(&state);
        assert_eq!(
            (invited["canRead"].clone(), invited["metadataOnly"].clone()),
            (json!(true), json!(false)),
            "{moment}: invited Folder must be readable: {invited}"
        );
        // A Working Tree may name the control Folder as metadata-only, but
        // never readable and never with its content.
        assert!(
            state["folderRoots"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|root| root["folderId"] == CONTROL_FOLDER_ID)
                .all(|root| root["canRead"] == false && root["metadataOnly"] == true),
            "{moment}: control Folder must stay excluded: {state}"
        );
        assert!(
            state["objects"]
                .as_array()
                .unwrap()
                .iter()
                .all(|object| object["folderId"] != CONTROL_FOLDER_ID),
            "{moment}: {state}"
        );
        assert!(
            !tree.join("Control Folder/wiki/control-page.md").exists(),
            "{moment}"
        );
        let export = self.owner_json(&["brain", "export", "--json"]);
        assert_eq!(
            Self::invited_revisions(&state["objects"]),
            Self::invited_revisions(&export["objects"]),
            "{moment}: member manifest must hold the current revisions"
        );
        for page in ["index.md", "log.md", "wiki/verification.md"] {
            assert_eq!(
                fs::read_to_string(tree.join("Team Folder").join(page)).unwrap(),
                fs::read_to_string(self.tree_a.join("Team Folder").join(page)).unwrap(),
                "{moment}: {page} must match the current revision"
            );
        }
        let verification =
            fs::read_to_string(self.tree_a.join("Team Folder/wiki/verification.md")).unwrap();
        let marker = verification
            .split("Marker ")
            .nth(1)
            .unwrap()
            .trim_end()
            .trim_end_matches('.');
        let search = self.run(
            &fbrain(),
            &self.home_b,
            tree,
            &["search", marker, "--lexical-only", "--json"],
        );
        assert!(
            search.status.success(),
            "{moment}: lexical search failed: {}",
            String::from_utf8_lossy(&search.stderr)
        );
        let search: Value = serde_json::from_slice(&search.stdout).unwrap();
        assert_eq!(
            search["searchedFolders"],
            json!([INVITED_FOLDER_ID]),
            "{moment}: {search}"
        );
        assert_eq!(
            search["results"][0]["folderId"], INVITED_FOLDER_ID,
            "{moment}: {search}"
        );
        let control = self.run(
            &fbrain(),
            &self.home_b,
            tree,
            &["search", "controlonly", "--lexical-only", "--json"],
        );
        assert!(control.status.success(), "{moment}");
        let control: Value = serde_json::from_slice(&control.stdout).unwrap();
        assert_eq!(control["results"], json!([]), "{moment}: {control}");
    }

    /// The steps run, then readability, current revisions,
    /// search and control-Folder exclusion must hold across repeated syncs, a
    /// later writer revision and a fresh Working Tree. Every fbrain invocation
    /// is a new process that trusts only persisted client state, so each sync
    /// here is also a client restart.
    fn run_ordering(&self, steps: &[InviteJourneyStep]) {
        let binary = fbrain();
        for step in steps {
            self.step(&binary, *step);
        }
        for round in 1..=3 {
            self.member_sync(&binary, &self.member_tree);
            self.assert_member_reads_current_folder(
                &self.member_tree,
                &format!("{steps:?}: member sync {round} after the journey"),
            );
        }
        self.step(
            &binary,
            InviteJourneyStep::OwnerRevisesPages("amberorchidthree"),
        );
        self.member_sync(&binary, &self.member_tree);
        self.assert_member_reads_current_folder(
            &self.member_tree,
            &format!("{steps:?}: member sync after a later revision"),
        );
        let fresh_tree = self.home_b.join("fresh-member-tree");
        self.member_open(&binary, &fresh_tree);
        self.member_sync(&binary, &fresh_tree);
        self.assert_member_reads_current_folder(&fresh_tree, &format!("{steps:?}: fresh tree"));
    }
}

impl Drop for InvitedMemberJourney {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(server_thread) = self.server_thread.take() {
            let _ = server_thread.join();
        }
    }
}

/// FIN-146 as reported: the member opens and syncs before key delivery, the
/// owner delivers, revises and adds a control Folder, then the member syncs.
#[test]
fn built_fbrain_invited_member_reads_later_revisions_after_live_ordering() {
    use InviteJourneyStep::*;
    InvitedMemberJourney::start().run_ordering(&[
        MemberOpens,
        MemberSync,
        OwnerDeliversKey,
        OwnerRevisesPages("amberorchidtwo"),
        OwnerCreatesControlFolder,
    ]);
}

/// The live ordering with a member sync (for example the daemon) between key
/// delivery and the writer revision: revision 1 materializes first.
#[test]
fn built_fbrain_invited_member_reads_later_revisions_after_interleaved_sync() {
    use InviteJourneyStep::*;
    InvitedMemberJourney::start().run_ordering(&[
        MemberOpens,
        MemberSync,
        OwnerDeliversKey,
        MemberSync,
        OwnerRevisesPages("amberorchidtwo"),
        OwnerCreatesControlFolder,
    ]);
}

/// Repeated member syncs after delivery with no writer change in between.
#[test]
fn built_fbrain_invited_member_stays_readable_across_idle_syncs_after_delivery() {
    use InviteJourneyStep::*;
    InvitedMemberJourney::start().run_ordering(&[
        MemberOpens,
        OwnerDeliversKey,
        MemberSync,
        MemberSync,
    ]);
}

/// The member first opens only after the key was delivered, so the first
/// export it caches already carries the grant.
#[test]
fn built_fbrain_invited_member_opened_after_delivery_reads_later_revisions() {
    use InviteJourneyStep::*;
    InvitedMemberJourney::start().run_ordering(&[
        OwnerDeliversKey,
        MemberOpens,
        OwnerRevisesPages("amberorchidtwo"),
        OwnerCreatesControlFolder,
    ]);
}

impl InvitedMemberJourney {
    fn member_json_file(&self, name: &str) -> Value {
        serde_json::from_slice(
            &fs::read(
                self.member_tree
                    .join(".finitebrain/encrypted-sync")
                    .join(name),
            )
            .unwrap(),
        )
        .unwrap()
    }

    /// Drive the member into the on-disk shape fbrain 0.5.0 leaves behind and
    /// that the live FIN-146 tree reported: the cached export predates the
    /// grant, the cached bootstrap holds the grant record and revision-2
    /// ciphertext at the latest cursor, and the Working Tree marks the Folder
    /// metadata-only over revision-1 manifest entries and plaintext.
    fn assert_stuck_shape(&self) {
        let export = self.member_json_file("export.json");
        assert!(
            export["keyGrants"]
                .as_array()
                .unwrap()
                .iter()
                .all(|grant| grant["folderId"] != INVITED_FOLDER_ID),
            "stuck shape: cached export must predate the grant: {}",
            export["keyGrants"]
        );
        let bootstrap = self.member_json_file("bootstrap.json");
        let grant_records = bootstrap["controlRecords"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|record| {
                record["recordType"] == "folder_key_grant"
                    && serde_json::from_str::<Value>(record["payloadJson"].as_str().unwrap())
                        .unwrap()["recipientNpub"]
                        == self.target_npub.as_str()
            })
            .count();
        assert_eq!(grant_records, 1, "stuck shape: cached grant record");
        assert!(
            Self::invited_revisions(&bootstrap["objects"])
                .values()
                .any(|revision| *revision == 2),
            "stuck shape: cached bootstrap must hold revision-2 ciphertext"
        );
        let state = Self::tree_state(&self.member_tree);
        assert_eq!(
            state["sync"]["latestSequence"], bootstrap["latestSequence"],
            "stuck shape: cursor at the cached bootstrap"
        );
        let invited = Self::invited_root(&state);
        assert_eq!(
            (invited["canRead"].clone(), invited["metadataOnly"].clone()),
            (json!(false), json!(true)),
            "stuck shape: {invited}"
        );
        assert!(
            Self::invited_revisions(&state["objects"])
                .values()
                .all(|revision| *revision == 1),
            "stuck shape: revision-1 manifest: {state}"
        );
        assert_eq!(
            fs::read_to_string(self.member_tree.join("Team Folder/wiki/verification.md")).unwrap(),
            verification_page("amberorchidone")
        );
    }

    /// One ordinary sync with the current CLI recovers the stuck tree; the
    /// recovered state then holds across syncs, a later revision and search.
    fn assert_ordinary_sync_recovers(&self) {
        let binary = fbrain();
        let recovered = self.member_sync(&binary, &self.member_tree);
        assert_eq!(recovered["conflicts"], json!([]));
        self.assert_member_reads_current_folder(&self.member_tree, "first sync after recovery");
        self.member_sync(&binary, &self.member_tree);
        self.assert_member_reads_current_folder(&self.member_tree, "second sync after recovery");
        self.step(
            &binary,
            InviteJourneyStep::OwnerRevisesPages("amberorchidthree"),
        );
        self.member_sync(&binary, &self.member_tree);
        self.assert_member_reads_current_folder(&self.member_tree, "later revision after recovery");
    }
}

/// A tree already stuck by fbrain 0.5.0 recovers on the next ordinary sync.
/// The current CLI never produces the stuck shape, so this rebuilds it from
/// the member's own persisted files exactly as 0.5.0 leaves them: the export
/// cached at first open, the bootstrap the member cached at the latest cursor,
/// and the Working Tree state and plaintext from the revision-1 sync with the
/// Folder marked metadata-only.
#[test]
fn built_fbrain_sync_recovers_invited_folder_stuck_by_stale_cached_export() {
    use InviteJourneyStep::*;
    let journey = InvitedMemberJourney::start();
    let binary = fbrain();
    let sync_dir = journey.member_tree.join(".finitebrain/encrypted-sync");
    let state_path = journey
        .member_tree
        .join(".finitebrain/working-tree-state.json");
    journey.member_open(&binary, &journey.member_tree);
    journey.step(&binary, MemberSync);
    let export_before_delivery = fs::read(sync_dir.join("export.json")).unwrap();
    journey.step(&binary, OwnerDeliversKey);
    journey.step(&binary, MemberSync);
    journey.assert_member_reads_current_folder(&journey.member_tree, "revision 1 delivered");
    let revision_one_state = InvitedMemberJourney::tree_state(&journey.member_tree);
    let revision_one_pages = ["index.md", "log.md", "wiki/verification.md"].map(|page| {
        let path = journey.member_tree.join("Team Folder").join(page);
        (path.clone(), fs::read(path).unwrap())
    });
    journey.step(&binary, OwnerRevisesPages("amberorchidtwo"));
    journey.step(&binary, OwnerCreatesControlFolder);
    journey.step(&binary, MemberSync);

    fs::write(sync_dir.join("export.json"), export_before_delivery).unwrap();
    let bootstrap = journey.member_json_file("bootstrap.json");
    let mut stuck_state = revision_one_state;
    stuck_state["sync"]["latestSequence"] = bootstrap["latestSequence"].clone();
    for root in stuck_state["folderRoots"].as_array_mut().unwrap() {
        if root["folderId"] == INVITED_FOLDER_ID {
            root["canRead"] = json!(false);
            root["metadataOnly"] = json!(true);
        }
    }
    write_json(&state_path, &stuck_state);
    for (path, bytes) in &revision_one_pages {
        fs::write(path, bytes).unwrap();
    }
    journey.assert_stuck_shape();

    journey.assert_ordinary_sync_recovers();
}

/// Recovery must not silently overwrite an unsynced edit to a page retained
/// in the stuck, metadata-only Folder. The edit reaches conflict handling
/// against the newer server revision instead of being replaced by it.
#[test]
fn built_fbrain_sync_recovery_preserves_unsynced_edit_in_stuck_folder() {
    use InviteJourneyStep::*;
    let journey = InvitedMemberJourney::start();
    let binary = fbrain();
    let sync_dir = journey.member_tree.join(".finitebrain/encrypted-sync");
    let state_path = journey
        .member_tree
        .join(".finitebrain/working-tree-state.json");
    journey.member_open(&binary, &journey.member_tree);
    journey.step(&binary, MemberSync);
    let export_before_delivery = fs::read(sync_dir.join("export.json")).unwrap();
    journey.step(&binary, OwnerDeliversKey);
    journey.step(&binary, MemberSync);
    let revision_one_state = InvitedMemberJourney::tree_state(&journey.member_tree);
    let revision_one_pages = ["index.md", "log.md", "wiki/verification.md"].map(|page| {
        let path = journey.member_tree.join("Team Folder").join(page);
        (path.clone(), fs::read(path).unwrap())
    });
    journey.step(&binary, OwnerRevisesPages("amberorchidtwo"));
    journey.step(&binary, MemberSync);

    fs::write(sync_dir.join("export.json"), export_before_delivery).unwrap();
    let bootstrap = journey.member_json_file("bootstrap.json");
    let mut stuck_state = revision_one_state;
    stuck_state["sync"]["latestSequence"] = bootstrap["latestSequence"].clone();
    for root in stuck_state["folderRoots"].as_array_mut().unwrap() {
        if root["folderId"] == INVITED_FOLDER_ID {
            root["canRead"] = json!(false);
            root["metadataOnly"] = json!(true);
        }
    }
    write_json(&state_path, &stuck_state);
    for (path, bytes) in &revision_one_pages {
        fs::write(path, bytes).unwrap();
    }
    let edited = journey.member_tree.join("Team Folder/wiki/verification.md");
    let marker = "unsynced-member-note-vermilionquartz";
    fs::write(
        &edited,
        format!(
            "{}\n{marker}\n",
            String::from_utf8_lossy(&revision_one_pages[2].1)
        ),
    )
    .unwrap();

    let result = journey.member_sync(&binary, &journey.member_tree);
    assert_ne!(
        result["conflicts"],
        json!([]),
        "an edit against revision 1 must conflict with server revision 2: {result}"
    );
    let mut preserved = Vec::new();
    let mut pending = vec![journey.member_tree.clone()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if fs::read(&path)
                .map(|bytes| String::from_utf8_lossy(&bytes).contains(marker))
                .unwrap_or(false)
            {
                preserved.push(path);
            }
        }
    }
    assert!(
        preserved.iter().any(|path| !path.starts_with(&sync_dir)),
        "the unsynced edit must remain in the Working Tree: {preserved:?}"
    );
}

/// Cross-version proof: the member runs the pre-fix CLI named by
/// `FBRAIN_PRE_FIX_BIN` (for example fbrain 0.5.0 built from
/// `fbrain/v0.5.0`) against the current server until it reaches the FIN-146
/// stuck shape on its own, then one ordinary sync with the current CLI
/// recovers it.
#[test]
#[ignore = "needs FBRAIN_PRE_FIX_BIN pointing at a pre-fix fbrain build"]
fn built_fbrain_sync_recovers_invited_folder_stuck_by_pre_fix_cli() {
    use InviteJourneyStep::*;
    let pre_fix = PathBuf::from(
        std::env::var_os("FBRAIN_PRE_FIX_BIN").expect("FBRAIN_PRE_FIX_BIN must name a binary"),
    );
    let journey = InvitedMemberJourney::start();
    journey.member_open(&pre_fix, &journey.member_tree);
    for step in [
        MemberSync,
        OwnerDeliversKey,
        MemberSync,
        OwnerRevisesPages("amberorchidtwo"),
        OwnerCreatesControlFolder,
        MemberSync,
        MemberSync,
    ] {
        journey.step(&pre_fix, step);
    }
    journey.assert_stuck_shape();

    journey.assert_ordinary_sync_recovers();
}

impl InvitedMemberJourney {
    /// Removing the member's Folder access removes readable state: the
    /// rotated revision stays undecryptable and search no longer admits the
    /// Folder. Restoring access then delivers the rotated key and the member
    /// reads the current revision again.
    fn assert_folder_access_revoke_and_regrant(&self) {
        let binary = fbrain();
        self.assert_member_reads_current_folder(&self.member_tree, "before revoke");
        let folder_access = |action: &str| {
            self.owner_json(&[
                "admin",
                "folder-access",
                action,
                "--folder",
                INVITED_FOLDER_ID,
                "--target",
                &self.target_npub,
                "--json",
            ])
        };
        assert_eq!(folder_access("revoke")["state"], "complete");
        // Revocation rotates the key and re-encrypts the Folder's objects.
        self.owner_json(&["sync", "now", "--json"]);
        self.step(
            &binary,
            InviteJourneyStep::OwnerRevisesPages("amberorchidrevoked"),
        );
        for round in 1..=2 {
            self.member_sync(&binary, &self.member_tree);
            let state = Self::tree_state(&self.member_tree);
            let invited = Self::invited_root(&state);
            assert_eq!(
                (invited["canRead"].clone(), invited["metadataOnly"].clone()),
                (json!(false), json!(true)),
                "member sync {round} after revoke must not keep the Folder readable"
            );
            assert_ne!(
                fs::read_to_string(self.member_tree.join("Team Folder/wiki/verification.md"))
                    .unwrap_or_default(),
                verification_page("amberorchidrevoked"),
                "member sync {round} after revoke must not decrypt the rotated revision"
            );
            let search = self.run(
                &binary,
                &self.home_b,
                &self.member_tree,
                &["search", "amberorchidone", "--lexical-only", "--json"],
            );
            assert!(
                search.status.success(),
                "member sync {round} after revoke: {}",
                String::from_utf8_lossy(&search.stderr)
            );
            let search: Value = serde_json::from_slice(&search.stdout).unwrap();
            assert_eq!(
                search["searchedFolders"],
                json!([]),
                "member sync {round} after revoke: {search}"
            );
            assert_eq!(search["results"], json!([]));
        }

        folder_access("grant");
        for round in 1..=2 {
            self.member_sync(&binary, &self.member_tree);
            self.assert_member_reads_current_folder(
                &self.member_tree,
                &format!("member sync {round} after regrant"),
            );
        }
    }
}

/// Revocation reaches a member whose key arrived as a sync record.
#[test]
fn built_fbrain_invited_member_revoke_and_regrant_after_record_delivered_key() {
    use InviteJourneyStep::*;
    let journey = InvitedMemberJourney::start();
    for step in [MemberOpens, OwnerDeliversKey, MemberSync, MemberSync] {
        journey.step(&fbrain(), step);
    }
    journey.assert_folder_access_revoke_and_regrant();
}

/// Revocation reaches a member whose first export already held the key.
#[test]
fn built_fbrain_invited_member_revoke_and_regrant_after_export_delivered_key() {
    use InviteJourneyStep::*;
    let journey = InvitedMemberJourney::start();
    for step in [OwnerDeliversKey, MemberOpens, MemberSync] {
        journey.step(&fbrain(), step);
    }
    journey.assert_folder_access_revoke_and_regrant();
}
