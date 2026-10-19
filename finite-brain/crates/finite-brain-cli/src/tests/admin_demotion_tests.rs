//! `fbrain admin role revoke admin` against a scripted Brain server.

use super::*;

const ADMIN_SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const DEMOTED_SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000002";

/// How the scripted server answers the demotion POST.
#[derive(Clone, Copy)]
enum DemotionReply {
    /// Commit and return metadata plus a receipt that match what was prepared.
    Committed,
    /// Claim success while the key is still an admin.
    KeptAdmin,
    /// An older server: its unknown-route 404.
    UnknownRoute,
}

fn folder(id: &str, access: &str, access_user_ids: &[&str], key_version: u32) -> Value {
    serde_json::json!({
        "id": id,
        "name": id,
        "role": "folder",
        "access": access,
        "parentFolderId": null,
        "path": id,
        "accessUserIds": access_user_ids,
        "currentKeyVersion": key_version,
        "setupIncomplete": false
    })
}

/// `restricted` is reached by the admin role alone, `team` through
/// All-Members membership and `explicit` through an explicit access row.
fn metadata(admin: &str, demoted: &str, admins: &[&str], restricted_version: u32) -> Value {
    serde_json::json!({
        "brainId": "acme",
        "kind": "organization",
        "name": "Acme",
        "ownerUserId": null,
        "members": [admin, demoted],
        "guests": [],
        "admins": admins,
        "folders": [
            folder("restricted", "restricted", &[], restricted_version),
            folder("team", "all_members", &[], 1),
            folder("explicit", "restricted", &[demoted], 1),
        ],
        "mountedFolders": [],
        "grantCount": 6
    })
}

fn start_demotion_server(
    admin: String,
    demoted: String,
    export_grant: Value,
    reply: DemotionReply,
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
                (
                    "200 OK",
                    metadata(&admin, &demoted, &[&admin, &demoted], 1).to_string(),
                )
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
                    "accessState": { "members": [admin, demoted], "admins": [admin, demoted] }
                });
                ("200 OK", export.to_string())
            } else {
                match reply {
                    DemotionReply::UnknownRoute => (
                        "404 Not Found",
                        serde_json::json!({
                            "error": "Brain API route not found; this client may use a retired Brain protocol. Upgrade fbrain and retry"
                        })
                        .to_string(),
                    ),
                    DemotionReply::Committed | DemotionReply::KeptAdmin => {
                        let admins: Vec<&str> = match reply {
                            DemotionReply::Committed => vec![&admin],
                            _ => vec![&admin, &demoted],
                        };
                        let mut response = metadata(&admin, &demoted, &admins, 2);
                        response["pendingApprovals"] =
                            serde_json::json!([{ "id": "approval-kept" }]);
                        response["rotations"] = serde_json::json!([{
                            "folderId": "restricted",
                            "previousKeyVersion": 1,
                            "newKeyVersion": 2,
                            "recipientNpubs": [admin],
                            "cutOffNpubs": [demoted],
                        }]);
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

struct DemotionRun {
    result: Result<(), CliError>,
    output: String,
    requests: Vec<(String, String)>,
    admin: String,
    demoted: String,
}

/// Run `admin role revoke admin` as the admin. `grant_for_admin` decides
/// whether the export carries a restricted wrap this key can open.
fn run_demotion(reply: DemotionReply, grant_for_admin: bool, json: bool) -> DemotionRun {
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
    let (server_url, server) = start_demotion_server(
        admin.clone(),
        demoted.clone(),
        export_grant,
        reply,
        expected_requests,
    );
    let mut args = vec![
        "admin".to_owned(),
        "role".to_owned(),
        "revoke".to_owned(),
        "admin".to_owned(),
        "--brain".to_owned(),
        "acme".to_owned(),
        "--target".to_owned(),
        demoted.clone(),
        "--server".to_owned(),
        server_url,
    ];
    if json {
        args.push("--json".to_owned());
    }
    let mut output = Vec::new();
    let result = run_with_env(args, env, &mut output);
    DemotionRun {
        result,
        output: String::from_utf8(output).unwrap(),
        requests: server.join().unwrap(),
        admin,
        demoted,
    }
}

fn event_payload(event: &Value) -> Value {
    let event = Event::from_json(event.to_string()).unwrap();
    serde_json::from_str(&event.content).unwrap()
}

#[test]
fn demotion_rotates_only_folders_reached_through_the_role() {
    let admin = "npub-admin";
    let demoted = "npub-demoted";
    let view: BrainMetadataView =
        serde_json::from_value(metadata(admin, demoted, &[admin, demoted], 1)).unwrap();
    assert_eq!(
        admin_demotion_rotation_folders(&view, demoted).unwrap(),
        vec!["restricted".to_owned()]
    );
    let mut with_admin_only = view.clone();
    with_admin_only.folders[1].access = "admin_only".to_owned();
    assert_eq!(
        admin_demotion_rotation_folders(&with_admin_only, demoted).unwrap(),
        vec!["restricted".to_owned(), "team".to_owned()]
    );
    // A key that is not an admin reaches nothing through the role.
    assert!(
        admin_demotion_rotation_folders(&view, "npub-member")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn demotion_submits_signed_rotations_and_prints_a_verified_receipt() {
    let run = run_demotion(DemotionReply::Committed, true, true);
    run.result.unwrap();
    let receipt: Value = serde_json::from_str(&run.output).unwrap();
    assert_eq!(receipt["state"], "complete");
    assert_eq!(receipt["operation"], "admin-role-revoke");
    assert_eq!(receipt["targetNpub"], run.demoted);
    assert_eq!(receipt["admins"], serde_json::json!([run.admin]));
    // The server's full response passes through, not only the typed view.
    assert_eq!(
        receipt["pendingApprovals"],
        serde_json::json!([{ "id": "approval-kept" }])
    );
    assert_eq!(
        receipt["rotations"],
        serde_json::json!([{
            "folderId": "restricted",
            "previousKeyVersion": 1,
            "newKeyVersion": 2,
            "recipientNpubs": [run.admin],
            "cutOffNpubs": [run.demoted],
        }])
    );

    let (request, body) = run.requests.last().unwrap();
    assert!(
        request.starts_with(&format!(
            "POST /v1/admin/brains/acme/roles/admin/{}/demotion",
            run.demoted
        )),
        "{request}"
    );
    let body: Value = serde_json::from_str(body).unwrap();
    let demotion = event_payload(&body["accessChangeEvent"]);
    assert_eq!(demotion["action"], "remove-admin");
    assert_eq!(demotion["targetNpub"], run.demoted);
    let rotations = body["rotations"].as_array().unwrap();
    assert_eq!(rotations.len(), 1);
    assert_eq!(rotations[0]["folderId"], "restricted");
    assert_eq!(rotations[0]["newKeyVersion"], 2);
    let grants = rotations[0]["grants"].as_array().unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0]["recipientNpub"], run.admin);
    assert_eq!(grants[0]["keyVersion"], 2);
    let rotation = event_payload(&rotations[0]["accessChangeEvent"]);
    assert_eq!(rotation["action"], "rotate-folder-key");
    assert_eq!(rotation["folderId"], "restricted");
    assert_eq!(rotation["keyVersion"], 2);
    assert!(rotation.get("targetNpub").is_none());
}

#[test]
fn demotion_prints_a_text_receipt_per_rotated_folder() {
    let run = run_demotion(DemotionReply::Committed, true, false);
    run.result.unwrap();
    assert_eq!(
        run.output,
        format!(
            "complete admin demotion brain=acme target={} rotatedFolders=1\nrotated folder=restricted keyVersion=1->2 recipients=1 cutOff=1\n",
            run.demoted
        )
    );
}

#[test]
fn demotion_refuses_before_sending_when_a_folder_key_cannot_be_opened() {
    let run = run_demotion(DemotionReply::Committed, false, true);
    let error = run.result.unwrap_err().to_string();
    assert!(
        error.contains("Folder restricted must be rotated"),
        "{error}"
    );
    assert!(error.contains("nothing was changed"), "{error}");
    assert!(
        run.requests
            .iter()
            .all(|(request, _)| !request.starts_with("POST")),
        "demotion must not be submitted"
    );
}

#[test]
fn demotion_rejects_a_receipt_that_keeps_the_admin() {
    let run = run_demotion(DemotionReply::KeptAdmin, true, true);
    let error = run.result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("admin demotion postcondition was not authoritative"),
        "{error}"
    );
    assert!(run.output.is_empty());
}

#[test]
fn demotion_against_an_older_server_fails_closed() {
    let run = run_demotion(DemotionReply::UnknownRoute, true, true);
    let error = run.result.unwrap_err();
    assert!(matches!(error, CliError::Unsupported(_)), "{error}");
    let message = error.to_string();
    assert!(message.contains("nothing was changed"), "{message}");
    assert!(
        message.contains("must rotate the Folder Keys of [restricted]"),
        "{message}"
    );
    // Exactly one mutation attempt: no fallback to the rotation-free route.
    assert_eq!(
        run.requests
            .iter()
            .filter(|(request, _)| !request.starts_with("GET"))
            .count(),
        1
    );
}

#[test]
fn demotion_refuses_a_request_over_the_server_body_limit_naming_the_folders() {
    let body = serde_json::json!({
        "accessChangeEvent": {},
        "rotations": [
            { "folderId": "small", "reencryptedRecords": [] },
            { "folderId": "large", "reencryptedRecords": ["x".repeat(400)] },
        ],
    });
    let size = serde_json::to_vec(&body).unwrap().len();
    let subject = demotion_request_subject("npub-demoted", &body);
    ensure_rotation_request_fits(&subject, &body, size).unwrap();
    let error = ensure_rotation_request_fits(&subject, &body, size - 1)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Admin demotion of npub-demoted, rotating [small ("),
        "{error}"
    );
    assert!(error.contains("large ("), "{error}");
    assert!(error.contains(&format!("needs a {size}-byte")), "{error}");
    assert!(error.contains("nothing was sent"), "{error}");
}
