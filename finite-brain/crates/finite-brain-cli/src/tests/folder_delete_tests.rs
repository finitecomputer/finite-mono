//! `fbrain folder delete` confirms its scope from the server's deletion
//! preview and never downloads the whole Brain to count objects.

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000001";
/// Larger than the CLI's 10 MiB JSON response cap.
const OVERSIZED_BOOTSTRAP_BYTES: usize = 11 * 1024 * 1024;

enum PreviewReply {
    Scope(Value),
    /// Status line and raw body, for older servers and malformed answers.
    Raw(&'static str, String),
}

fn route_not_found() -> PreviewReply {
    PreviewReply::Raw(
        "404 Not Found",
        serde_json::json!({
            "error": "Brain API route not found; this client may use a retired Brain protocol. Upgrade fbrain and retry"
        })
        .to_string(),
    )
}

struct FolderDeleteServer {
    url: String,
    stop: Arc<AtomicBool>,
    handle: thread::JoinHandle<Vec<(String, String)>>,
}

impl FolderDeleteServer {
    fn start(preview: PreviewReply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut requests = Vec::new();
            while !stop_flag.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(15) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (request_line, body) = read_http_request(&mut stream);
                stream.set_nonblocking(false).unwrap();
                let (status, response_body) = if request_line.contains("/sync/bootstrap") {
                    // What a real Brain answers: every object's ciphertext.
                    let filler = "c".repeat(OVERSIZED_BOOTSTRAP_BYTES);
                    let bootstrap = serde_json::json!({
                        "brainId": "acme",
                        "latestSequence": 41,
                        "objects": [{
                            "folderId": "general",
                            "objectId": "obj_000000000001",
                            "revision": 1,
                            "ciphertext": filler,
                            "deleted": false
                        }]
                    });
                    ("200 OK", bootstrap.to_string())
                } else if request_line.contains("/deletion-preview") {
                    match &preview {
                        PreviewReply::Scope(scope) => ("200 OK", scope.to_string()),
                        PreviewReply::Raw(status, body) => (*status, body.clone()),
                    }
                } else if request_line.starts_with("DELETE ") {
                    (
                        "200 OK",
                        serde_json::json!({
                            "sequence": 42,
                            "duplicate": false,
                            "folderCount": 2,
                            "objectCount": 3,
                            "deletedFolderIds": ["archive", "archive-child"]
                        })
                        .to_string(),
                    )
                } else if request_line.contains("/metadata") {
                    ("200 OK", metadata().to_string())
                } else {
                    (
                        "404 Not Found",
                        serde_json::json!({ "error": "not found" }).to_string(),
                    )
                };
                requests.push((request_line, body));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
            requests
        });
        Self { url, stop, handle }
    }

    fn finish(self) -> Vec<(String, String)> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.join().unwrap()
    }
}

fn folder(id: &str, parent: Option<&str>, key_version: u32) -> Value {
    serde_json::json!({
        "id": id,
        "name": id,
        "role": "folder",
        "access": "admin_only",
        "parentFolderId": parent,
        "path": id,
        "accessUserIds": [],
        "currentKeyVersion": key_version,
        "setupIncomplete": false
    })
}

fn metadata() -> Value {
    serde_json::json!({
        "brainId": "acme",
        "kind": "organization",
        "name": "Acme",
        "ownerUserId": null,
        "members": [],
        "admins": ["npub-admin"],
        "folders": [
            folder("archive", None, 2),
            folder("archive-child", Some("archive"), 1),
            folder("general", None, 1),
        ],
        "mountedFolders": [],
        "grantCount": 0,
        "collaboratorReadiness": []
    })
}

fn preview(folder_ids: &[&str], object_count: usize, key_version: u32) -> PreviewReply {
    PreviewReply::Scope(serde_json::json!({
        "brainId": "acme",
        "folderId": "archive",
        "folderIds": folder_ids,
        "objectCount": object_count,
        "currentKeyVersion": key_version
    }))
}

fn delete_archive(server_url: &str) -> (Result<(), CliError>, Vec<u8>) {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, SECRET);
    let mut output = Vec::new();
    let result = run_with_env(
        [
            "folder", "delete", "archive", "--brain", "acme", "--server", server_url, "--json",
        ],
        env_for(&tmp),
        &mut output,
    );
    (result, output)
}

fn assert_never_downloaded_or_deleted(requests: &[(String, String)]) {
    assert!(
        !requests
            .iter()
            .any(|(line, _)| line.contains("/sync/bootstrap")),
        "{requests:?}"
    );
    assert!(
        !requests.iter().any(|(line, _)| line.starts_with("DELETE ")),
        "{requests:?}"
    );
}

#[test]
fn folder_delete_echoes_the_server_preview_without_downloading_the_brain() {
    let server = FolderDeleteServer::start(preview(&["archive", "archive-child"], 3, 2));
    let (result, output) = delete_archive(&server.url);
    let requests = server.finish();
    result.unwrap();

    let response: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        response["deletedFolderIds"],
        serde_json::json!(["archive", "archive-child"])
    );
    assert!(
        !requests
            .iter()
            .any(|(line, _)| line.contains("/sync/bootstrap")),
        "{requests:?}"
    );
    assert!(requests.iter().any(|(line, _)| {
        line.starts_with("GET /v1/brains/acme/folders/archive/deletion-preview ")
    }));
    let (_, delete_body) = requests
        .iter()
        .find(|(line, _)| line.starts_with("DELETE /v1/brains/acme/folders/archive "))
        .expect("DELETE was sent");
    let delete_body: Value = serde_json::from_str(delete_body).unwrap();
    assert_eq!(
        delete_body["expectedFolderIds"],
        serde_json::json!(["archive", "archive-child"])
    );
    assert_eq!(delete_body["expectedObjectCount"], 3);
}

#[test]
fn folder_delete_fails_closed_when_the_server_has_no_preview_route() {
    for (label, reply) in [
        ("unknown route", route_not_found()),
        (
            "method not allowed",
            PreviewReply::Raw("405 Method Not Allowed", String::new()),
        ),
    ] {
        let server = FolderDeleteServer::start(reply);
        let (result, _) = delete_archive(&server.url);
        let requests = server.finish();

        let error = result.unwrap_err();
        assert!(
            matches!(error, CliError::Unsupported(_)),
            "{label}: {error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains("Upgrade the Brain server"),
            "{label}: {message}"
        );
        assert!(
            message.contains("nothing was deleted"),
            "{label}: {message}"
        );
        assert_never_downloaded_or_deleted(&requests);
    }
}

#[test]
fn folder_delete_rejects_a_preview_that_names_a_different_folder() {
    for (label, brain_id, folder_id) in [
        ("other Brain", "other", "archive"),
        ("other Folder", "acme", "archive-child"),
    ] {
        let server = FolderDeleteServer::start(PreviewReply::Scope(serde_json::json!({
            "brainId": brain_id,
            "folderId": folder_id,
            "folderIds": ["archive", "archive-child"],
            "objectCount": 0,
            "currentKeyVersion": 2
        })));
        let (result, _) = delete_archive(&server.url);
        let requests = server.finish();

        let error = result.unwrap_err();
        assert!(
            matches!(&error, CliError::HttpResponseDecode(reason) if reason.contains("different Folder")),
            "{label}: {error:?}"
        );
        assert_never_downloaded_or_deleted(&requests);
    }
}

#[test]
fn folder_delete_rejects_a_malformed_preview() {
    for (label, body) in [
        (
            "missing count",
            serde_json::json!({
                "brainId": "acme",
                "folderId": "archive",
                "folderIds": ["archive", "archive-child"],
                "currentKeyVersion": 2
            }),
        ),
        (
            "negative count",
            serde_json::json!({
                "brainId": "acme",
                "folderId": "archive",
                "folderIds": ["archive", "archive-child"],
                "objectCount": -1,
                "currentKeyVersion": 2
            }),
        ),
        (
            "Folder ids not a list",
            serde_json::json!({
                "brainId": "acme",
                "folderId": "archive",
                "folderIds": "archive",
                "objectCount": 0,
                "currentKeyVersion": 2
            }),
        ),
    ] {
        let server = FolderDeleteServer::start(PreviewReply::Scope(body));
        let (result, _) = delete_archive(&server.url);
        let requests = server.finish();

        let error = result.unwrap_err();
        assert!(
            matches!(&error, CliError::HttpResponseDecode(reason) if reason.contains("malformed")),
            "{label}: {error:?}"
        );
        assert_never_downloaded_or_deleted(&requests);
    }
}

#[test]
fn folder_delete_refuses_a_preview_that_disagrees_with_local_metadata() {
    for (label, reply) in [
        ("missing descendant", preview(&["archive"], 0, 2)),
        (
            "extra Folder",
            preview(&["archive", "archive-child", "general"], 0, 2),
        ),
        (
            "duplicate Folder",
            preview(&["archive", "archive", "archive-child"], 0, 2),
        ),
        ("rotated key", preview(&["archive", "archive-child"], 0, 3)),
    ] {
        let server = FolderDeleteServer::start(reply);
        let (result, _) = delete_archive(&server.url);
        let requests = server.finish();

        let error = result.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("changed while preparing its deletion"),
            "{label}: {error}"
        );
        assert_never_downloaded_or_deleted(&requests);
    }
}
