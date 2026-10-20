//! `fbrain admin member remove` against a scripted Brain server: the plan
//! drives which Folders are rotated, and every refusal happens before the
//! mutating DELETE or fails the postcondition after it.

use super::*;

const ADMIN_SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const REMOVED_SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000002";

/// How the scripted server behaves.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Script {
    /// Plan the stale-grant Folder at version 1 and commit the removal.
    Committed,
    /// The plan reports version 2: another admin rotated after the metadata
    /// fetch, so the locally prepared rotation is out of date.
    PlanVersionMoved,
    /// The DELETE answers with the Folder still at version 1.
    FolderNotAdvanced,
    /// An older server without the plan route.
    OldServer,
    /// Commit the removal, but answer the DELETE only after 11 seconds, past
    /// the ordinary request timeout, like a server queueing it for a permit.
    SlowDelete,
}

impl Script {
    /// Requests the client must make: metadata, plan, export, mounts,
    /// DELETE, mounts; each refusal stops earlier.
    fn expected_requests(self) -> usize {
        match self {
            Self::Committed | Self::SlowDelete => 6,
            Self::FolderNotAdvanced => 5,
            Self::PlanVersionMoved => 3,
            Self::OldServer => 2,
        }
    }
}

fn metadata(members: &[&str], admin: &str, key_version: u32) -> Value {
    serde_json::json!({
        "brainId": "acme",
        "kind": "organization",
        "name": "Acme",
        "ownerUserId": null,
        "members": members,
        "guests": [],
        "admins": [admin],
        "folders": [{
            "id": "restricted",
            "name": "restricted",
            "role": "folder",
            "access": "restricted",
            "parentFolderId": null,
            "path": "restricted",
            "accessUserIds": [],
            "currentKeyVersion": key_version,
            "setupIncomplete": false
        }],
        "mountedFolders": [],
        "grantCount": 2
    })
}

fn reply(
    script: Script,
    request_line: &str,
    admin: &str,
    removed: &str,
    export_grant: &Value,
) -> (&'static str, String) {
    if request_line.contains("/removal-plan") {
        if script == Script::OldServer {
            return (
                "404 Not Found",
                serde_json::json!({
                    "error": "Brain API route not found; this client may use a retired Brain protocol. Upgrade fbrain and retry"
                })
                .to_string(),
            );
        }
        let planned_version = if script == Script::PlanVersionMoved {
            2
        } else {
            1
        };
        let plan = serde_json::json!({
            "brainId": "acme",
            "targetNpub": removed,
            "folderRotations": [{
                "folderId": "restricted",
                "currentKeyVersion": planned_version,
                "reason": "staleCurrentGrant"
            }]
        });
        return ("200 OK", plan.to_string());
    }
    if request_line.starts_with("DELETE") {
        if script == Script::SlowDelete {
            thread::sleep(Duration::from_secs(11));
        }
        let version = if script == Script::FolderNotAdvanced {
            1
        } else {
            2
        };
        return ("200 OK", metadata(&[admin], admin, version).to_string());
    }
    if request_line.contains("/metadata") {
        return ("200 OK", metadata(&[admin, removed], admin, 1).to_string());
    }
    if request_line.contains("/export") {
        let export = serde_json::json!({
            "brain": { "id": "acme", "kind": "organization", "name": "Acme", "ownerUserId": null },
            "folders": [{
                "id": "restricted",
                "path": "restricted",
                "access": "restricted",
                "currentKeyVersion": 1,
                "accessible": true
            }],
            "keyGrants": [export_grant],
            "objects": [],
            "accessState": { "members": [admin, removed], "admins": [admin] }
        });
        return ("200 OK", export.to_string());
    }
    if request_line.contains("/mounts") {
        return (
            "200 OK",
            serde_json::json!({ "incoming": [], "outgoing": [] }).to_string(),
        );
    }
    ("500 Internal Server Error", "{}".to_owned())
}

struct RemovalRun {
    result: Result<(), CliError>,
    output: String,
    requests: Vec<(String, String)>,
    admin: String,
    removed: String,
}

impl RemovalRun {
    fn sent_delete(&self) -> bool {
        self.requests
            .iter()
            .any(|(request, _)| request.starts_with("DELETE"))
    }
}

fn run_remove(script: Script) -> RemovalRun {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, ADMIN_SECRET);
    let env = env_for(&tmp);
    let admin = run(&tmp, &["signer", "public-key"]).trim().to_owned();
    let removed = npub_for_secret(REMOVED_SECRET);
    let export_grant = export_grant_for_test(
        &env,
        "acme",
        "restricted",
        1,
        &FolderKey::from_bytes([9; 32]),
        &admin,
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let server_url = format!("http://{}", listener.local_addr().unwrap());
    let (server_admin, server_removed) = (admin.clone(), removed.clone());
    let server = thread::spawn(move || {
        let started = Instant::now();
        let mut requests = Vec::new();
        while requests.len() < script.expected_requests()
            && started.elapsed() < Duration::from_secs(30)
        {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            let (request_line, body) = read_http_request(&mut stream);
            let (status, response_body) = reply(
                script,
                &request_line,
                &server_admin,
                &server_removed,
                &export_grant,
            );
            requests.push((request_line, body));
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        requests
    });
    let mut output = Vec::new();
    let result = run_with_env(
        [
            "admin",
            "member",
            "remove",
            "--brain",
            "acme",
            "--target",
            &removed,
            "--server",
            &server_url,
            "--json",
        ],
        env,
        &mut output,
    );
    RemovalRun {
        result,
        output: String::from_utf8(output).unwrap(),
        requests: server.join().unwrap(),
        admin,
        removed,
    }
}

#[test]
fn removal_rotates_the_planned_stale_grant_folder_and_reports_it() {
    let run = run_remove(Script::Committed);
    run.result.unwrap();
    let receipt: Value = serde_json::from_str(&run.output).unwrap();
    assert_eq!(
        receipt["rotatedFolders"],
        serde_json::json!([{
            "folderId": "restricted",
            "reason": "staleCurrentGrant",
            "previousKeyVersion": 1,
            "newKeyVersion": 2
        }])
    );
    assert!(
        run.requests
            .iter()
            .any(|(request, _)| request.starts_with(&format!(
                "GET /v1/admin/brains/acme/members/{}/removal-plan",
                run.removed
            ))),
        "{:?}",
        run.requests
    );
    let (request, body) = run
        .requests
        .iter()
        .find(|(request, _)| request.starts_with("DELETE"))
        .unwrap();
    assert!(
        request.starts_with(&format!(
            "DELETE /v1/admin/brains/acme/members/{}",
            run.removed
        )),
        "{request}"
    );
    let body: Value = serde_json::from_str(body).unwrap();
    let rotations = body["rotations"].as_array().unwrap();
    assert_eq!(rotations.len(), 1);
    assert_eq!(rotations[0]["folderId"], "restricted");
    assert_eq!(rotations[0]["newKeyVersion"], 2);
    let grants = rotations[0]["grants"].as_array().unwrap();
    assert_eq!(grants.len(), 1, "only the remaining admin is re-wrapped");
    assert_eq!(grants[0]["recipientNpub"], run.admin);
    let event = Event::from_json(body["accessChangeEvent"].to_string()).unwrap();
    let payload: Value = serde_json::from_str(&event.content).unwrap();
    assert_eq!(payload["action"], "remove-member");
    assert_eq!(payload["targetNpub"], run.removed);
}

#[test]
fn removal_refuses_without_deleting_when_the_planned_key_version_moved() {
    let run = run_remove(Script::PlanVersionMoved);
    let sent_delete = run.sent_delete();
    let error = run.result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("changed while member removal was prepared"),
        "{error}"
    );
    assert!(!sent_delete, "{:?}", run.requests);
    assert!(run.output.is_empty());
}

#[test]
fn removal_fails_its_postcondition_when_a_planned_folder_did_not_advance() {
    let run = run_remove(Script::FolderNotAdvanced);
    let sent_delete = run.sent_delete();
    let error = run.result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Member removal postcondition was not authoritative"),
        "{error}"
    );
    assert!(sent_delete);
    assert!(run.output.is_empty());
}

#[test]
fn removal_against_an_older_server_stops_before_the_delete() {
    let run = run_remove(Script::OldServer);
    let sent_delete = run.sent_delete();
    let error = run.result.unwrap_err();
    assert!(matches!(error, CliError::Unsupported(_)), "{error}");
    let message = error.to_string();
    assert!(message.contains("nothing was changed"), "{message}");
    assert!(message.contains("Upgrade the Brain server"), "{message}");
    assert!(!sent_delete, "{:?}", run.requests);
    assert!(run.output.is_empty());
}

/// The DELETE carries rotation material, so it uses the rotation timeout and
/// outlasts a server that holds it longer than an ordinary request.
#[test]
fn removal_delete_is_answered_when_the_server_holds_it_past_ten_seconds() {
    let run = run_remove(Script::SlowDelete);
    run.result.unwrap();
    let receipt: Value = serde_json::from_str(&run.output).unwrap();
    assert_eq!(receipt["rotatedFolders"][0]["newKeyVersion"], 2);
    assert!(
        run.requests
            .iter()
            .any(|(request, _)| request.starts_with("DELETE")),
        "{:?}",
        run.requests
    );
}
