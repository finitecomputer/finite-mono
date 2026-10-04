//! `fbrain access list` against scripted synthetic report pages.

use super::*;

const SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000001";

fn own_npub() -> String {
    let keys = nostr::Keys::parse(SECRET).unwrap();
    finite_nostr::NostrPublicKey::from_protocol(keys.public_key())
        .to_npub()
        .unwrap()
}

fn fingerprint(seed: char) -> String {
    seed.to_string().repeat(64)
}

fn identity(npub: &str, role: &str, mut name: Value, kind: &str, mut folders: Value) -> Value {
    let type_value = if kind == "Managed Agent" {
        "managedAgent"
    } else {
        "notConfirmed"
    };
    if matches!(name["state"].as_str(), Some("verified" | "domainClaimed")) {
        name["matchedKey"] = "00".into();
    }
    for folder in folders.as_array_mut().unwrap() {
        if folder["currentGrant"] == "present" {
            if !folder["grant"].is_object() {
                folder["grant"] = serde_json::json!({"signedAudit": "notStored"});
            }
            folder["grant"]["issuedBy"] = "npub1syntheticissuer".into();
            folder["grant"]["issuedAt"] = "2026-06-24T20:46:36Z".into();
        }
    }
    let missing = folders
        .as_array()
        .unwrap()
        .iter()
        .filter(|folder| folder["state"] == "grantMissing")
        .map(|folder| folder["folderId"].clone())
        .collect::<Vec<_>>();
    let revoked = folders
        .as_array()
        .unwrap()
        .iter()
        .filter(|folder| folder["state"] == "revocationIncomplete")
        .map(|folder| folder["folderId"].clone())
        .collect::<Vec<_>>();
    serde_json::json!({
        "npub": npub,
        "hex": "00",
        "brainRole": role,
        "roleSources": ["brainMember"],
        "name": name,
        "identityType": {
            "value": type_value,
            "label": kind,
            "evidence": "synthetic evidence",
        },
        "folders": folders,
        "missingCurrentGrants": missing,
        "revocationIncomplete": revoked,
    })
}

struct Page {
    seed: char,
    identities: Vec<Value>,
    after: Option<String>,
    next: Option<String>,
}

fn render(page: Page) -> String {
    let authority = fingerprint(page.seed);
    let mut body = serde_json::json!({
        "version": "finite-brain-access-report-v1",
        "brainId": "acme",
        "brainKind": "organization",
        "brainName": "Synthetic Org",
        "authoritySequence": 7,
        "authorityFingerprint": authority,
        "checkedAt": "2026-06-24T20:46:36Z",
        "actingKey": { "npub": own_npub(), "brainRole": "admin" },
        "coverage": {
            "members": { "state": "complete" },
            "guests": { "state": "complete" },
            "mounts": { "state": "unverified", "reason": "1 Folder(s) are mounted into this Brain" },
            "currentGrants": { "state": "complete" },
            "accessHistory": { "state": "unsupported", "reason": "not reconstructed" },
        },
        "currentAccessComplete": false,
        "directory": { "state": "checked", "checkedKeys": 1 },
        "totals": { "identities": 3, "revocationIncomplete": 1, "grantsMissing": 0 },
        "incomingMounts": [{
            "mountId": "mount-synthetic", "displayName": "Partner Notes",
            "sourceBrainId": "partner", "sourceFolderId": "notes",
            "connectionStatus": "active", "participantDetail": "complete", "participants": 1,
        }],
        "page": { "limit": 100, "returned": page.identities.len() },
        "identities": page.identities,
    });
    if let Some(after) = page.after {
        body["page"]["after"] = after.into();
    }
    if let Some(next) = page.next {
        body["page"]["next"] = format!("{authority}.{next}").into();
    }
    body.to_string()
}

fn first_rows() -> Vec<Value> {
    vec![
        identity(
            "npub1syntheticadmin",
            "admin",
            serde_json::json!({
                "state": "verified", "display": "admin-name@finite.vip",
                "label": "admin-name@finite.vip", "source": "identityDirectory:finite_vip_binding",
                "checkedAt": "2026-06-24T20:46:36Z",
            }),
            "Type not confirmed",
            serde_json::json!([{
                "folderId": "general", "path": "general", "accessMode": "all_members",
                "keyVersion": 1, "entitled": true, "entitlementSources": ["brainAdmin"],
                "currentGrant": "present", "state": "ready",
                "grant": { "signedAudit": "verified" },
            }]),
        ),
        identity(
            "npub1syntheticagent",
            "member",
            serde_json::json!({
                "state": "verified", "display": "agent-bot@finite.vip",
                "label": "agent-bot@finite.vip", "source": "identityDirectory:finite_vip_binding",
                "checkedAt": "2026-06-24T20:46:36Z",
            }),
            "Managed Agent",
            serde_json::json!([]),
        ),
    ]
}

fn second_rows() -> Vec<Value> {
    let mut removed = identity(
        "npub1syntheticremoved",
        "noCurrentRole",
        serde_json::json!({
            "state": "storedNotRechecked",
            "display": "Stored name, not rechecked: old-name@example.test",
            "label": "old-name@example.test", "storedVerifiedAt": "2026-05-01T00:00:00Z",
        }),
        "Type not confirmed",
        serde_json::json!([{
            "folderId": "general", "path": "general", "accessMode": "all_members",
            "keyVersion": 1, "entitled": false, "entitlementSources": [],
            "currentGrant": "present", "state": "revocationIncomplete",
        }]),
    );
    removed["incomingMounts"] = serde_json::json!([{
        "mountId": "mount-synthetic", "displayName": "Partner Notes",
        "sourceBrainId": "partner", "sourceFolderId": "notes",
        "mountAccess": true, "currentGrant": "missing", "state": "grantMissing",
    }]);
    vec![removed]
}

fn first_page(seed: char) -> String {
    render(Page {
        seed,
        identities: first_rows(),
        after: None,
        next: Some("npub1syntheticagent".to_owned()),
    })
}

fn second_page(seed: char, cursor_seed: char) -> String {
    render(Page {
        seed,
        identities: second_rows(),
        after: Some(format!("{}.npub1syntheticagent", fingerprint(cursor_seed))),
        next: None,
    })
}

fn second_request(seed: char) -> String {
    format!(
        "GET /v1/brains/acme/access-report?limit=100&after={}.npub1syntheticagent HTTP/1.1",
        fingerprint(seed)
    )
}

fn run_list(server_url: &str, tmp: &TempDir, json: bool) -> Result<Vec<u8>, CliError> {
    let mut output = Vec::new();
    let mut args = vec!["access", "list", "--brain", "acme", "--server", server_url];
    if json {
        args.push("--json");
    }
    run_with_env(args, env_for(tmp), &mut output).map(|()| output)
}

#[test]
fn access_list_json_and_human_render_the_same_paged_report() {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, SECRET);
    let (server_url, server) = start_scripted_capture_server(vec![
        (200, first_page('a')),
        (200, second_page('a', 'a')),
        (200, first_page('a')),
        (200, second_page('a', 'a')),
    ]);

    let json_output = run_list(&server_url, &tmp, true).unwrap();
    let report: Value = serde_json::from_slice(&json_output).unwrap();
    assert_eq!(report["identities"].as_array().unwrap().len(), 3);
    assert_eq!(report["pages"], 2);
    assert!(report.get("page").is_none());
    assert_eq!(report["authoritySequence"], 7);
    assert_eq!(report["authorityFingerprint"], fingerprint('a'));
    assert_eq!(report["directory"]["checkedKeys"], 2);

    let human = String::from_utf8(run_list(&server_url, &tmp, false).unwrap()).unwrap();
    for row in report["identities"].as_array().unwrap() {
        let npub = row["npub"].as_str().unwrap();
        let role = row["brainRole"].as_str().unwrap();
        assert!(human.contains(&format!("key: {npub}")), "{human}");
        assert!(human.contains(&format!("role: {role} (")), "{human}");
        assert!(human.contains(row["identityType"]["label"].as_str().unwrap()));
    }
    let coverage = &report["coverage"];
    assert!(human.contains(&format!(
        "Coverage: members {}; guests {}; mounts {}; current grants {}; access history {}",
        coverage["members"]["state"].as_str().unwrap(),
        coverage["guests"]["state"].as_str().unwrap(),
        coverage["mounts"]["state"].as_str().unwrap(),
        coverage["currentGrants"]["state"].as_str().unwrap(),
        coverage["accessHistory"]["state"].as_str().unwrap(),
    )));
    assert!(human.contains(&format!("(authority {})", fingerprint('a'))));
    assert!(human.contains(&format!("as {} (admin)", own_npub())));
    assert!(human.contains("Current access: NOT complete"));
    assert!(human.contains("Mounted in: Partner Notes from Brain partner Folder notes"));
    assert!(human.contains("mounted Partner Notes from partner/notes: grantMissing"));
    assert!(human.contains("[Managed Agent]"));
    assert!(human.contains("[brainAdmin]; signed audit verified"));
    assert!(human.contains(
        "grant issued by npub1syntheticissuer at 2026-06-24T20:46:36Z (stored provenance)"
    ));
    assert!(human.contains("Stored name, not rechecked: old-name@example.test"));
    assert!(human.contains("Revocation incomplete: current grant without entitlement"));
    assert!(human.contains("Names are labels, not authority"));

    let requests = server.join().unwrap();
    let lines = requests
        .iter()
        .map(|(line, _)| line.clone())
        .collect::<Vec<_>>();
    let first = "GET /v1/brains/acme/access-report?limit=100 HTTP/1.1".to_owned();
    assert_eq!(
        lines,
        vec![
            first.clone(),
            second_request('a'),
            first,
            second_request('a')
        ]
    );
}

#[test]
fn access_list_fails_clearly_on_an_old_server_without_weaker_fallback() {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, SECRET);
    let (server_url, server) = start_scripted_capture_server(vec![(
        404,
        serde_json::json!({
            "error": "Brain API route not found; this client may use a retired Brain protocol. Upgrade fbrain and retry"
        })
        .to_string(),
    )]);
    let error = run_list(&server_url, &tmp, true).unwrap_err();
    match &error {
        CliError::Unsupported(reason) => {
            assert!(reason.contains("does not provide the named access report"));
            assert!(reason.contains("no weaker metadata summary was substituted"));
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1, "no metadata fallback request");
    assert!(requests[0].0.contains("/access-report"));
}

#[test]
fn access_list_rereads_when_authority_changes_between_pages() {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, SECRET);
    // A different fingerprint on page two, then a 409 for a stale cursor:
    // both restart from page one; the final report is one authority.
    let (server_url, server) = start_scripted_capture_server(vec![
        (200, first_page('a')),
        (200, second_page('b', 'a')),
        (200, first_page('b')),
        (
            409,
            serde_json::json!({ "error": "Brain access changed" }).to_string(),
        ),
    ]);
    let error = run_list(&server_url, &tmp, true).unwrap_err();
    assert!(
        error.to_string().contains("Brain access changed"),
        "{error}"
    );
    assert_eq!(server.join().unwrap().len(), 4);

    let (server_url, server) = start_scripted_capture_server(vec![
        (200, first_page('a')),
        (
            409,
            serde_json::json!({ "error": "Brain access changed" }).to_string(),
        ),
        (200, first_page('c')),
        (200, second_page('c', 'c')),
    ]);
    let report: Value =
        serde_json::from_slice(&run_list(&server_url, &tmp, true).unwrap()).unwrap();
    assert_eq!(report["authorityFingerprint"], fingerprint('c'));
    assert_eq!(report["identities"].as_array().unwrap().len(), 3);
    let requests = server.join().unwrap();
    assert_eq!(requests[3].0, second_request('c'));
}

#[test]
fn access_list_refuses_partial_foreign_or_inconsistent_pages() {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, SECRET);
    let mutate = |page: String, change: &dyn Fn(&mut Value)| {
        let mut value: Value = serde_json::from_str(&page).unwrap();
        change(&mut value);
        value.to_string()
    };
    let cases: Vec<(&str, Vec<String>)> = vec![
        (
            "partial report",
            vec![mutate(
                render(Page {
                    seed: 'a',
                    identities: Vec::new(),
                    after: None,
                    next: None,
                }),
                &|page| page["directory"]["checkedKeys"] = 0.into(),
            )],
        ),
        (
            "different Brain",
            vec![mutate(first_page('a'), &|page| {
                page["brainId"] = "other".into()
            })],
        ),
        (
            "different calling key",
            vec![mutate(first_page('a'), &|page| {
                page["actingKey"]["npub"] = "npub1someoneelse".into()
            })],
        ),
        (
            "invalid authority fingerprint",
            vec![mutate(first_page('a'), &|page| {
                page["authorityFingerprint"] = "abc".into()
            })],
        ),
        (
            "next-page cursor does not match",
            vec![mutate(first_page('a'), &|page| {
                page["page"]["next"] = format!("{}.npub1elsewhere", fingerprint('a')).into()
            })],
        ),
        (
            "does not continue from the requested cursor",
            vec![first_page('a'), second_page('a', 'b')],
        ),
        (
            "strictly ordered and unique",
            vec![
                first_page('a'),
                mutate(second_page('a', 'a'), &|page| {
                    page["identities"][0]["npub"] = "npub1syntheticadmin".into()
                }),
            ],
        ),
        (
            "pages disagree on totals",
            vec![
                first_page('a'),
                mutate(second_page('a', 'a'), &|page| {
                    page["totals"]["identities"] = 4.into()
                }),
            ],
        ),
    ];
    for (expected, pages) in cases {
        let (server_url, server) =
            start_scripted_capture_server(pages.into_iter().map(|page| (200, page)).collect());
        let error = run_list(&server_url, &tmp, false).unwrap_err();
        assert!(error.to_string().contains(expected), "{expected}: {error}");
        server.join().unwrap();
    }
}

#[test]
fn access_list_refuses_malformed_or_contradictory_complete_reports() {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, SECRET);
    for field in [
        "coverage",
        "totals",
        "currentAccessComplete",
        "checkedAt",
        "directory",
        "incomingMounts",
        "page",
    ] {
        let mut page: Value = serde_json::from_str(&first_page('a')).unwrap();
        page.as_object_mut().unwrap().remove(field);
        let (url, server) = start_scripted_capture_server(vec![(200, page.to_string())]);
        assert!(
            run_list(&url, &tmp, false)
                .unwrap_err()
                .to_string()
                .contains("malformed access report"),
            "{field}"
        );
        server.join().unwrap();
    }
    let mut contradictions: Vec<Value> = Vec::new();
    let base: Value = serde_json::from_str(&first_page('a')).unwrap();
    let mut complete = base.clone();
    complete["currentAccessComplete"] = true.into();
    contradictions.push(complete);
    let mut missing_rows = base.clone();
    missing_rows["identities"][0]
        .as_object_mut()
        .unwrap()
        .remove("folders");
    contradictions.push(missing_rows);
    let mut wrong_type = base.clone();
    wrong_type["currentAccessComplete"] = "true".into();
    contradictions.push(wrong_type);
    let mut wrong_count = base.clone();
    wrong_count["page"]["returned"] = 0.into();
    contradictions.push(wrong_count);
    let mut wrong_folder = base.clone();
    wrong_folder["identities"][0]["folders"][0]["entitled"] = false.into();
    contradictions.push(wrong_folder);
    let mut wrong_name = base.clone();
    wrong_name["identities"][0]["name"]["matchedKey"] = "ff".into();
    contradictions.push(wrong_name);
    let mut missing_issuer = base;
    missing_issuer["identities"][0]["folders"][0]["grant"]
        .as_object_mut()
        .unwrap()
        .remove("issuedBy");
    contradictions.push(missing_issuer);
    for page in contradictions {
        let (url, server) = start_scripted_capture_server(vec![(200, page.to_string())]);
        assert!(
            run_list(&url, &tmp, false)
                .unwrap_err()
                .to_string()
                .contains("malformed access report")
        );
        server.join().unwrap();
    }
    // The original empty response could previously default a missing total to zero.
    let forged = serde_json::json!({"version": "finite-brain-access-report-v1", "brainId": "acme",
        "actingKey": {"npub": own_npub()}, "authorityFingerprint": fingerprint('a'),
        "identities": [], "currentAccessComplete": true});
    let (url, server) = start_scripted_capture_server(vec![(200, forged.to_string())]);
    assert!(run_list(&url, &tmp, false).is_err());
    server.join().unwrap();
}

#[test]
fn access_list_retries_first_page_drift_and_accepts_content_sequence_changes() {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, SECRET);
    let mut second: Value = serde_json::from_str(&second_page('a', 'a')).unwrap();
    second["authoritySequence"] = 8.into();
    let (url, server) = start_scripted_capture_server(vec![
        (
            409,
            serde_json::json!({"error": "Brain access changed"}).to_string(),
        ),
        (200, first_page('a')),
        (200, second.to_string()),
    ]);
    let report: Value = serde_json::from_slice(&run_list(&url, &tmp, true).unwrap()).unwrap();
    assert_eq!(report["identities"].as_array().unwrap().len(), 3);
    assert_eq!(server.join().unwrap().len(), 3);
}

#[test]
fn access_list_renders_forward_names_as_domain_claims() {
    let tmp = TempDir::new().unwrap();
    import_identity_secret(&tmp, SECRET);
    let mut first: Value = serde_json::from_str(&first_page('a')).unwrap();
    first["identities"][0]["name"]["state"] = "domainClaimed".into();
    first["identities"][0]["name"]["source"] = "nip05Forward".into();
    let (url, server) =
        start_scripted_capture_server(vec![(200, first.to_string()), (200, second_page('a', 'a'))]);
    let text = String::from_utf8(run_list(&url, &tmp, false).unwrap()).unwrap();
    assert!(text.contains("domain claims this exact key via nip05Forward"));
    assert!(text.contains("not independently verified"));
    server.join().unwrap();
}
