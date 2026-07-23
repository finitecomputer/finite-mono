use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

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
                            "sharedFolderSource": false,
                            "accessible": false
                        },
                        {
                            "id": "locked",
                            "path": "Locked",
                            "access": "owner",
                            "currentKeyVersion": 1,
                            "sharedFolderSource": false,
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
