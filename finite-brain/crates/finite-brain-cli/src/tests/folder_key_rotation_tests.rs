//! `fbrain admin folder-access rotate` against a scripted Brain server.

use super::*;

const ADMIN_SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const DEMOTED_SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000002";

/// How the scripted server answers the rotation POST.
#[derive(Clone, Copy)]
enum RotationReply {
    /// Commit and return a receipt that matches the prepared rotation.
    Committed,
    /// Return a receipt that also re-wrapped the demoted key.
    RewrappedStaleHolder,
    /// An older server: its unknown-route 404.
    UnknownRoute,
}

fn metadata(admin: &str, demoted: &str, key_version: u32) -> Value {
    serde_json::json!({
        "brainId": "acme",
        "kind": "organization",
        "name": "Acme",
        "ownerUserId": null,
        "members": [admin, demoted],
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

fn start_rotation_server(
    admin: String,
    demoted: String,
    export_grant: Value,
    reply: RotationReply,
    expected_requests: usize,
) -> (String, thread::JoinHandle<Vec<(String, String)>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let started = Instant::now();
        let mut requests = Vec::new();
        while requests.len() < expected_requests && started.elapsed() < Duration::from_secs(5) {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            let (request_line, body) = read_http_request(&mut stream);
            let (status, response_body) = if request_line.contains("/metadata") {
                ("200 OK", metadata(&admin, &demoted, 1).to_string())
            } else if request_line.contains("/export") {
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
                    "accessState": { "members": [admin, demoted], "admins": [admin] }
                });
                ("200 OK", export.to_string())
            } else {
                match reply {
                    RotationReply::UnknownRoute => (
                        "404 Not Found",
                        serde_json::json!({
                            "error": "Brain API route not found; this client may use a retired Brain protocol. Upgrade fbrain and retry"
                        })
                        .to_string(),
                    ),
                    RotationReply::Committed | RotationReply::RewrappedStaleHolder => {
                        let mut response = metadata(&admin, &demoted, 2);
                        let recipients = match reply {
                            RotationReply::Committed => vec![admin.clone()],
                            _ => vec![admin.clone(), demoted.clone()],
                        };
                        response["rotation"] = serde_json::json!({
                            "folderId": "restricted",
                            "previousKeyVersion": 1,
                            "newKeyVersion": 2,
                            "recipientNpubs": recipients,
                            "cutOffNpubs": [demoted],
                        });
                        ("200 OK", response.to_string())
                    }
                }
            };
            requests.push((request_line, body));
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        requests
    });
    (url, handle)
}

struct RotationRun {
    result: Result<(), CliError>,
    output: String,
    requests: Vec<(String, String)>,
    admin: String,
    demoted: String,
}

/// Run `admin folder-access rotate` as the admin. `grant_for_admin` decides
/// whether the export carries a current wrap this key can open.
fn run_rotate(reply: RotationReply, grant_for_admin: bool, json: bool) -> RotationRun {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, ADMIN_SECRET);
    let env = env_for(&tmp);
    let admin = run(&tmp, &["signer", "public-key"]).trim().to_owned();
    let demoted = npub_for_secret(DEMOTED_SECRET);
    let grant_recipient = if grant_for_admin { &admin } else { &demoted };
    let export_grant = export_grant_for_test(
        &env,
        "acme",
        "restricted",
        1,
        &FolderKey::from_bytes([9; 32]),
        grant_recipient,
    );
    let expected_requests = if grant_for_admin { 3 } else { 2 };
    let (server_url, server) = start_rotation_server(
        admin.clone(),
        demoted.clone(),
        export_grant,
        reply,
        expected_requests,
    );
    let mut args = vec![
        "admin",
        "folder-access",
        "rotate",
        "--brain",
        "acme",
        "--folder",
        "restricted",
        "--server",
        &server_url,
    ];
    if json {
        args.push("--json");
    }
    let mut output = Vec::new();
    let result = run_with_env(args, env, &mut output);
    RotationRun {
        result,
        output: String::from_utf8(output).unwrap(),
        requests: server.join().unwrap(),
        admin,
        demoted,
    }
}

#[test]
fn rotate_prepares_signed_material_and_prints_a_verified_receipt() {
    let run = run_rotate(RotationReply::Committed, true, true);
    run.result.unwrap();
    let receipt: Value = serde_json::from_str(&run.output).unwrap();
    assert_eq!(receipt["state"], "complete");
    assert_eq!(receipt["operation"], "admin-folder-access-rotate");
    assert_eq!(receipt["brainId"], "acme");
    assert_eq!(receipt["folderId"], "restricted");
    assert_eq!(receipt["previousKeyVersion"], 1);
    assert_eq!(receipt["newKeyVersion"], 2);
    assert_eq!(receipt["recipientNpubs"], serde_json::json!([run.admin]));
    assert_eq!(receipt["cutOffNpubs"], serde_json::json!([run.demoted]));

    let (request, body) = run.requests.last().unwrap();
    assert!(
        request.starts_with("POST /v1/admin/brains/acme/folders/restricted/key-rotations"),
        "{request}"
    );
    let body: Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["newKeyVersion"], 2);
    let grants = body["grants"].as_array().unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0]["recipientNpub"], run.admin);
    assert_eq!(grants[0]["keyVersion"], 2);
    let event = Event::from_json(body["accessChangeEvent"].to_string()).unwrap();
    let payload: Value = serde_json::from_str(&event.content).unwrap();
    assert_eq!(payload["action"], "rotate-folder-key");
    assert_eq!(payload["folderId"], "restricted");
    assert_eq!(payload["keyVersion"], 2);
    assert!(payload.get("targetNpub").is_none());
    assert!(body.get("folderKey").is_none());
}

#[test]
fn rotate_prints_a_text_receipt_naming_cut_off_keys() {
    let run = run_rotate(RotationReply::Committed, true, false);
    run.result.unwrap();
    assert!(
        run.output.starts_with(
            "complete Folder Key rotation brain=acme folder=restricted keyVersion=1->2 recipients=1 cutOff=1"
        ),
        "{}",
        run.output
    );
    assert!(run.output.contains(&format!("cut off {}", run.demoted)));
}

#[test]
fn rotate_rejects_a_receipt_that_does_not_match_the_prepared_recipients() {
    let run = run_rotate(RotationReply::RewrappedStaleHolder, true, true);
    let error = run.result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Folder Key rotation postcondition was not authoritative"),
        "{error}"
    );
    assert!(run.output.is_empty());
}

#[test]
fn rotate_explains_an_older_server_without_the_route() {
    let run = run_rotate(RotationReply::UnknownRoute, true, true);
    let error = run.result.unwrap_err();
    assert!(matches!(error, CliError::Unsupported(_)), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("does not support Folder Key rotation yet"),
        "{message}"
    );
    assert!(message.contains("nothing was changed"), "{message}");
}

#[test]
fn rotate_refuses_before_posting_when_this_key_cannot_open_the_current_folder_key() {
    let run = run_rotate(RotationReply::Committed, false, true);
    let error = run.result.unwrap_err();
    assert!(
        matches!(error, CliError::GrantOpening { .. }),
        "unexpected error: {error}"
    );
    assert!(error.to_string().contains("Folder Key rotation"), "{error}");
    assert!(
        run.requests
            .iter()
            .all(|(request, _)| !request.starts_with("POST")),
        "rotation material must not be submitted"
    );
}

#[test]
fn rotate_requires_a_folder_outside_a_working_tree() {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, ADMIN_SECRET);
    let error = run_with_env(
        ["admin", "folder-access", "rotate", "--brain", "acme"],
        env_for(&tmp),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        matches!(error, CliError::MissingArgument("--folder")),
        "{error}"
    );
}

fn revoke_metadata(admins: &[&str], members: &[&str], access: &[&str], key_version: u32) -> Value {
    serde_json::json!({
        "brainId": "acme",
        "kind": "organization",
        "name": "Acme",
        "ownerUserId": null,
        "members": members,
        "guests": [],
        "admins": admins,
        "folders": [{
            "id": "restricted",
            "name": "restricted",
            "role": "folder",
            "access": "restricted",
            "parentFolderId": null,
            "path": "restricted",
            "accessUserIds": access,
            "currentKeyVersion": key_version,
            "setupIncomplete": false
        }],
        "mountedFolders": [],
        "grantCount": 2
    })
}

/// Serves `before` for metadata, a readable export, and `after` for the
/// DELETE; returns the captured request lines.
fn start_revoke_server(
    before: Value,
    after: Value,
    export_grant: Value,
    expected_requests: usize,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let started = Instant::now();
        let mut requests = Vec::new();
        while requests.len() < expected_requests && started.elapsed() < Duration::from_secs(5) {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            let (request_line, _) = read_http_request(&mut stream);
            let body = if request_line.contains("/metadata") {
                before.to_string()
            } else if request_line.contains("/export") {
                serde_json::json!({
                    "brain": { "id": "acme", "kind": "organization", "name": "Acme", "ownerUserId": null },
                    "folders": [{ "id": "restricted", "path": "restricted", "access": "restricted", "currentKeyVersion": 1, "accessible": true }],
                    "keyGrants": [export_grant],
                    "objects": [],
                    "accessState": { "members": before["members"], "admins": before["admins"] }
                })
                .to_string()
            } else {
                after.to_string()
            };
            requests.push(request_line);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        requests
    });
    (url, handle)
}

fn run_revoke(
    before: Value,
    after: Value,
    expected_requests: usize,
) -> (Result<(), CliError>, Vec<String>) {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, ADMIN_SECRET);
    let env = env_for(&tmp);
    let admin = run(&tmp, &["signer", "public-key"]).trim().to_owned();
    let export_grant = export_grant_for_test(
        &env,
        "acme",
        "restricted",
        1,
        &FolderKey::from_bytes([9; 32]),
        &admin,
    );
    let (server_url, server) = start_revoke_server(before, after, export_grant, expected_requests);
    let target = npub_for_secret(DEMOTED_SECRET);
    let result = run_with_env(
        [
            "admin",
            "folder-access",
            "revoke",
            "--brain",
            "acme",
            "--folder",
            "restricted",
            "--target",
            &target,
            "--server",
            &server_url,
            "--json",
        ],
        env,
        &mut Vec::new(),
    );
    (result, server.join().unwrap())
}

#[test]
fn revoke_refuses_before_preparing_when_the_target_keeps_the_key_by_role() {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, ADMIN_SECRET);
    let admin = run(&tmp, &["signer", "public-key"]).trim().to_owned();
    let promoted = npub_for_secret(DEMOTED_SECRET);
    // An explicit grant from before the key was promoted to admin.
    let before = revoke_metadata(&[&admin, &promoted], &[&admin, &promoted], &[&promoted], 1);
    let (result, requests) = run_revoke(before.clone(), before, 1);
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("through its Brain role"),
        "{error}"
    );
    assert!(error.to_string().contains("Nothing was changed"), "{error}");
    assert!(
        requests
            .iter()
            .all(|request| !request.starts_with("DELETE")),
        "{requests:?}"
    );
}

#[test]
fn revoke_rejects_a_result_that_still_wraps_the_target() {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, ADMIN_SECRET);
    let admin = run(&tmp, &["signer", "public-key"]).trim().to_owned();
    let target = npub_for_secret(DEMOTED_SECRET);
    let before = revoke_metadata(&[&admin], &[&admin, &target], &[&target], 1);
    // The row is gone and the version moved, but the target is an admin in
    // the authoritative result, so the new key was wrapped for it.
    let after = revoke_metadata(&[&admin, &target], &[&admin, &target], &[], 2);
    let (result, requests) = run_revoke(before, after, 3);
    let error = result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Folder access revocation postcondition was not authoritative"),
        "{error}"
    );
    assert!(
        requests.last().unwrap().starts_with("DELETE"),
        "{requests:?}"
    );
}

#[test]
fn rotation_requests_over_the_limit_are_refused_before_sending() {
    let body = serde_json::json!({ "reencryptedRecords": ["x".repeat(64)] });
    let size = serde_json::to_vec(&body).unwrap().len();
    ensure_rotation_request_fits("Folder restricted Key rotation", &body, size).unwrap();
    let error = ensure_rotation_request_fits("Folder restricted Key rotation", &body, size - 1)
        .unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("Folder restricted Key rotation"),
        "{message}"
    );
    assert!(
        message.contains(&format!("{size}-byte rotation request")),
        "{message}"
    );
    assert!(message.contains("nothing was sent"), "{message}");
    // The CLI enforces the same bound the server applies to rotation routes.
    assert_eq!(
        finite_brain_core::MAX_FOLDER_ROTATION_REQUEST_BODY_BYTES,
        64 * 1024 * 1024
    );
}
