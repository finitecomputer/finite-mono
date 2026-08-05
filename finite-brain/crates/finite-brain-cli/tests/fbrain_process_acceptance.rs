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

use finite_nostr::NostrPublicKey;
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
                "collaborationState": if self
                    .passed_boundaries
                    .contains(&"nativeEmailCollaboration")
                {
                    Value::String("complete".to_owned())
                } else {
                    Value::Null
                },
                "independentFiniteHomes": self.passed_boundaries.contains(&"fixtureSetup"),
                "targetForm": "canonicalManagedAgentEmail",
                "unregisteredEmailFolderBootstrap": self
                    .passed_boundaries
                    .contains(&"unregisteredEmailFolderInvitationBootstrap"),
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
    let (url_tx, url_rx) = mpsc::channel();
    let nip05_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let nip05_url = format!("http://{}", nip05_listener.local_addr().unwrap());
    let target_hex = NostrPublicKey::parse(target_npub).unwrap().to_hex();
    let personal_agent_npub = personal_agent_npub.to_owned();
    let owner_npub = owner_npub.to_owned();
    let (identity_authority_url, core_authority_url) =
        spawn_requester_authorities(owner_npub.clone(), requester_npub.to_owned());
    thread::spawn(move || {
        if let Ok((mut stream, _)) = nip05_listener.accept() {
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let body = format!(r#"{{"names":{{"beta":"{target_hex}"}}}}"#);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        }
    });
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
                .with_identity_authority_url(nip05_url)
                .with_agent_bootstrap_authorities(
                    core_authority_url,
                    "process-core-token",
                    identity_authority_url,
                    "process-identity-token",
                )
                .with_dev_invite_mailer()
                .with_smoke_email_proofs(
                    "future-user@example.com,future-personal@example.com,existing-member@example.com,stale-user@example.com",
                )
                .unwrap()
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

fn spawn_requester_authorities(
    managed_agent_npub: String,
    requester_npub: String,
) -> (String, String) {
    fn serve(listener: TcpListener, responder: impl Fn(&str) -> (u16, Value) + Send + 'static) {
        thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(mut stream) = incoming else {
                    break;
                };
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 4096];
                    let bytes = stream.read(&mut chunk).unwrap_or(0);
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
                let request = String::from_utf8_lossy(&request);
                let (status, body) = responder(&request);
                let reason = if status == 200 { "OK" } else { "Not Found" };
                let body = body.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
    }

    let identity = TcpListener::bind("127.0.0.1:0").unwrap();
    let identity_url = format!("http://{}", identity.local_addr().unwrap());
    let identity_agent = managed_agent_npub.clone();
    let identity_owner = requester_npub.clone();
    serve(identity, move |request| {
        match request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/")
        {
            "/api/v1/operator/brain/agent-resolution" if request.contains(&identity_agent) => (
                200,
                json!({
                    "agentNpub": identity_agent,
                    "managedAgentEmail": "alpha@finite.vip",
                }),
            ),
            "/api/v1/operator/brain/user-resolution" => (
                200,
                json!({
                    "workosUserId": "process-owner",
                    "userNpub": identity_owner,
                }),
            ),
            _ => (404, json!({ "error": "not_found" })),
        }
    });

    let core = TcpListener::bind("127.0.0.1:0").unwrap();
    let core_url = format!("http://{}", core.local_addr().unwrap());
    serve(core, move |request| {
        match request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/")
        {
            "/api/core/v1/brain/agent-account" => (
                200,
                json!({
                    "workosUserId": "process-owner",
                    "managedAgentEmail": "alpha@finite.vip",
                    "verifiedEmail": "owner@example.com",
                    "status": "active",
                }),
            ),
            _ => (404, json!({ "error": "not_found" })),
        }
    });
    (identity_url, core_url)
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

fn command(home: &Path, cwd: &Path) -> Command {
    let mut command = Command::new(fbrain());
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
            "# Folder Agent Instructions\n\nFolder id: `{folder_id}`\n\nUse `raw/` for source captures, `raw/assets/` for non-Markdown Assets, `wiki/` for durable synthesized pages, `inventory/` for source candidates and open questions, `datasets/` for manifests and query recipes, and `output/` for generated artifacts. Pair every Asset with a Markdown Source Note before citing it from synthesized work.\n"
        )
    );
    for marker in [
        "raw/.keep",
        "raw/assets/.keep",
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
    let target_hex = NostrPublicKey::parse(&target_npub).unwrap().to_hex();
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
    let forged_session = "forged-requester-session";
    write_requester_context(&home_a.join("finite-home"), forged_session, &target_hex);
    let forged_requester = command(&home_a, &home_a)
        .env("HERMES_SESSION_PLATFORM", "finitechat")
        .env("HERMES_SESSION_KEY", forged_session)
        .env("HERMES_SESSION_USER_ID", &target_hex)
        .env("FINITE_BRAIN_SERVER_URL", &server_url)
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", &server_url)
        .args([
            "brain",
            "create",
            "organization",
            "Forged Requester",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!forged_requester.status.success());
    let downgraded_agent = run(
        &home_a,
        &home_a,
        &[
            "brain",
            "create",
            "organization",
            "Downgraded Agent",
            "--json",
        ],
    );
    assert!(!downgraded_agent.status.success());
    assert!(
        String::from_utf8_lossy(&downgraded_agent.stderr)
            .contains("requires authenticated requester context")
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

    smoke.enter("unregisteredEmailFolderInvitationBootstrap");
    let email_brain_invitation = run(
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
        email_brain_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&email_brain_invitation.stderr)
    );
    let email_brain_invitation: Value =
        serde_json::from_slice(&email_brain_invitation.stdout).unwrap();
    assert_eq!(email_brain_invitation["folderOnly"], json!(false));
    assert_eq!(
        email_brain_invitation["invitedEmail"],
        "future-user@example.com"
    );
    assert_eq!(email_brain_invitation["deliveryStatus"], "sent");

    let email_folder_invitation = run(
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
        email_folder_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&email_folder_invitation.stderr)
    );
    let email_folder_invitation: Value =
        serde_json::from_slice(&email_folder_invitation.stdout).unwrap();
    assert_eq!(email_folder_invitation["folderOnly"], json!(true));
    assert_eq!(
        email_folder_invitation["invitedEmail"],
        "future-user@example.com"
    );
    assert_eq!(email_folder_invitation["deliveryStatus"], "sent");
    assert_eq!(
        email_folder_invitation["initialFolderAccess"],
        json!(["admin-only"])
    );
    assert_eq!(
        email_folder_invitation["bootstrapScope"]
            .as_array()
            .unwrap()
            .iter()
            .map(|scope| scope["folderId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["admin-only"]
    );
    assert!(
        email_folder_invitation["inviteUrl"]
            .as_str()
            .unwrap()
            .contains("#inviteSecret=")
    );
    let listed_email_folder_invitations = run(
        &home_a,
        &tree_a.join("Admin Only"),
        &["invite", "folder", "list", "--json"],
    );
    assert!(
        listed_email_folder_invitations.status.success(),
        "{}",
        String::from_utf8_lossy(&listed_email_folder_invitations.stderr)
    );
    let listed_email_folder_invitations: Value =
        serde_json::from_slice(&listed_email_folder_invitations.stdout).unwrap();
    assert!(
        listed_email_folder_invitations["invitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|invitation| invitation["id"] == email_folder_invitation["id"]),
        "an email-targeted Folder Invitation must remain visible through the Folder collection"
    );
    let listed_brain_invitations = run(&home_a, &tree_a, &["invite", "brain", "list", "--json"]);
    assert!(
        listed_brain_invitations.status.success(),
        "{}",
        String::from_utf8_lossy(&listed_brain_invitations.stderr)
    );
    let listed_brain_invitations: Value =
        serde_json::from_slice(&listed_brain_invitations.stdout).unwrap();
    assert!(
        listed_brain_invitations["invitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|invitation| invitation["id"] == email_brain_invitation["id"]),
        "the pending Brain Invitation must remain visible through the Brain collection"
    );
    assert!(
        listed_brain_invitations["invitations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|invitation| invitation["id"] != email_folder_invitation["id"]),
        "a Folder Invitation must not leak into the Brain Invitation collection"
    );
    let invite_code = email_folder_invitation["inviteCode"].as_str().unwrap();
    let invite_secret = email_folder_invitation["inviteSecret"].as_str().unwrap();
    let invite_secret_file = home_b.join("folder-invite-secret");
    fs::write(&invite_secret_file, invite_secret).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&invite_secret_file, fs::Permissions::from_mode(0o600)).unwrap();
    let wrong_email_claim = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "folder",
            "claim",
            invite_code,
            "--email",
            "not-the-invited-user@example.com",
            "--invite-secret-file",
            invite_secret_file.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!wrong_email_claim.status.success());
    let claimed_email_invitation = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "folder",
            "claim",
            invite_code,
            "--email",
            "future-user@example.com",
            "--invite-secret-file",
            invite_secret_file.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        claimed_email_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&claimed_email_invitation.stderr)
    );
    let claimed_email_invitation: Value =
        serde_json::from_slice(&claimed_email_invitation.stdout).unwrap();
    assert_eq!(claimed_email_invitation["status"], "accepted");
    assert_eq!(claimed_email_invitation["folderOnly"], true);
    let duplicate_email_claim = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "folder",
            "claim",
            invite_code,
            "--email",
            "future-user@example.com",
            "--invite-secret-file",
            invite_secret_file.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        !duplicate_email_claim.status.success(),
        "a consumed Folder Invitation bootstrap must reject a duplicate claim"
    );
    assert!(
        String::from_utf8_lossy(&duplicate_email_claim.stderr)
            .contains("invitation bootstrap is unavailable")
    );
    let revoked_brain_invitation = run(
        &home_a,
        &tree_a,
        &[
            "invite",
            "brain",
            "revoke",
            email_brain_invitation["id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        revoked_brain_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&revoked_brain_invitation.stderr)
    );
    let revoked_brain_invitation: Value =
        serde_json::from_slice(&revoked_brain_invitation.stdout).unwrap();
    assert_eq!(revoked_brain_invitation["status"], "revoked");
    let cancel_accepted = run(
        &home_a,
        &tree_a.join("Admin Only"),
        &[
            "invite",
            "folder",
            "revoke",
            email_folder_invitation["id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        !cancel_accepted.status.success(),
        "an accepted Folder Invitation must not be cancellable"
    );
    let pending_cancellation = run(
        &home_a,
        &tree_a.join("Admin Only"),
        &[
            "invite",
            "folder",
            "create",
            "--target",
            "stale-user@example.com",
            "--json",
        ],
    );
    assert!(
        pending_cancellation.status.success(),
        "{}",
        String::from_utf8_lossy(&pending_cancellation.stderr)
    );
    let pending_cancellation: Value = serde_json::from_slice(&pending_cancellation.stdout).unwrap();
    let cancelled_secret_file = home_b.join("cancelled-folder-invite-secret");
    fs::write(
        &cancelled_secret_file,
        pending_cancellation["inviteSecret"].as_str().unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(&cancelled_secret_file, fs::Permissions::from_mode(0o600)).unwrap();
    let cancelled = run(
        &home_a,
        &tree_a.join("Admin Only"),
        &[
            "invite",
            "folder",
            "revoke",
            pending_cancellation["id"].as_str().unwrap(),
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
    let cancelled_claim = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "folder",
            "claim",
            pending_cancellation["inviteCode"].as_str().unwrap(),
            "--email",
            "stale-user@example.com",
            "--invite-secret-file",
            cancelled_secret_file.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        !cancelled_claim.status.success(),
        "a cancelled pending Folder Invitation must not be claimable"
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
        tree_b.join("Admin Only/email-guest-edit.md"),
        "# Email guest edit\n\nBounded Folder write proof.\n",
    )
    .unwrap();
    let guest_push = run(&home_b, &tree_b, &["sync", "now", "--json"]);
    assert!(
        guest_push.status.success(),
        "{}",
        String::from_utf8_lossy(&guest_push.stderr)
    );
    smoke.pass();

    smoke.enter("nativeEmailCollaboration");
    let target = "beta@finite.vip";
    let ensure = run(
        &home_a,
        &tree_a,
        &["collaborator", "ensure-admin", "--target", target, "--json"],
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
        fs::read_to_string(tree_a.join("Admin Only/email-guest-edit.md")).unwrap(),
        "# Email guest edit\n\nBounded Folder write proof.\n"
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
            "future-personal@example.com",
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
    let personal_invite_secret_file = home_b.join("personal-folder-invite-secret");
    fs::write(
        &personal_invite_secret_file,
        personal_folder_invitation["inviteSecret"].as_str().unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(
        &personal_invite_secret_file,
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let accepted_personal_folder = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "folder",
            "claim",
            personal_folder_invitation["inviteCode"].as_str().unwrap(),
            "--email",
            "future-personal@example.com",
            "--invite-secret-file",
            personal_invite_secret_file.to_str().unwrap(),
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
    let member_email_invitation = run(
        &requester_home,
        &invitation_org_tree.join("Member Scope"),
        &[
            "invite",
            "folder",
            "create",
            "--target",
            "existing-member@example.com",
            "--json",
        ],
    );
    assert!(
        member_email_invitation.status.success(),
        "{}",
        String::from_utf8_lossy(&member_email_invitation.stderr)
    );
    let member_email_invitation: Value =
        serde_json::from_slice(&member_email_invitation.stdout).unwrap();
    let member_invite_secret_file = home_b.join("member-folder-invite-secret");
    fs::write(
        &member_invite_secret_file,
        member_email_invitation["inviteSecret"].as_str().unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(
        &member_invite_secret_file,
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let claimed_member_invitation = run(
        &home_b,
        &home_b,
        &[
            "invite",
            "folder",
            "claim",
            member_email_invitation["inviteCode"].as_str().unwrap(),
            "--email",
            "existing-member@example.com",
            "--invite-secret-file",
            member_invite_secret_file.to_str().unwrap(),
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
