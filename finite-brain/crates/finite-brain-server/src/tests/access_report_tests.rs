//! Access report through the public signed router. Every identity, name, and
//! domain here is synthetic.

use super::*;
use crate::directory_names::{
    DirectoryKeyNames, DirectoryLookupFailure, DirectoryLookupResponse, DirectoryName,
    DirectoryNameLookup,
};
use finite_brain_core::sha256_hex;
use std::sync::atomic::{AtomicUsize, Ordering};

const REPORT_BODY_LIMIT: usize = 1024 * 1024;

type DirectoryCalls = Arc<Mutex<Vec<Vec<String>>>>;
type Nip05Urls = Arc<Mutex<Vec<String>>>;

struct Cast {
    admin: Keys,
    mailbox: Keys,
    agent: Keys,
    shared: Keys,
    unconfirmed: Keys,
    replaced: Keys,
    replacement: Keys,
    arbitrary: Keys,
    outsider: Keys,
    guest: Keys,
    demoted: Keys,
    mount_admin: Keys,
}

impl Cast {
    fn new() -> Self {
        Self {
            admin: Keys::generate(),
            mailbox: Keys::generate(),
            agent: Keys::generate(),
            shared: Keys::generate(),
            unconfirmed: Keys::generate(),
            replaced: Keys::generate(),
            replacement: Keys::generate(),
            arbitrary: Keys::generate(),
            outsider: Keys::generate(),
            guest: Keys::generate(),
            demoted: Keys::generate(),
            mount_admin: Keys::generate(),
        }
    }

    fn all(&self) -> [&Keys; 12] {
        [
            &self.admin,
            &self.mailbox,
            &self.agent,
            &self.shared,
            &self.unconfirmed,
            &self.replaced,
            &self.replacement,
            &self.arbitrary,
            &self.outsider,
            &self.guest,
            &self.demoted,
            &self.mount_admin,
        ]
    }
}

fn lookup_fn(
    lookup: impl Fn(&[String]) -> Result<DirectoryLookupResponse, DirectoryLookupFailure>
    + Send
    + Sync
    + 'static,
) -> Option<DirectoryNameLookup> {
    Some(Arc::new(lookup))
}

fn hex_of(keys: &Keys) -> String {
    NostrPublicKey::from_protocol(keys.public_key()).to_hex()
}

fn user(keys: &Keys) -> UserId {
    UserId::new(npub(keys)).unwrap()
}

/// A complete Directory answer: `found` for known keys, `not_found` for the rest.
fn directory_answer(
    keys: &[String],
    known: &BTreeMap<String, (&'static str, &'static str)>,
) -> DirectoryLookupResponse {
    DirectoryLookupResponse {
        checked_at: Some(TEST_NOW),
        results: keys
            .iter()
            .map(|key| {
                let names = known
                    .get(key)
                    .map(|(name, kind)| {
                        vec![DirectoryName {
                            name: (*name).to_owned(),
                            kind: (*kind).to_owned(),
                            source: Some("finite_vip_binding".to_owned()),
                            bound_at: Some(TEST_NOW - 100),
                        }]
                    })
                    .unwrap_or_default();
                DirectoryKeyNames {
                    pubkey: key.clone(),
                    status: if names.is_empty() {
                        "not_found"
                    } else {
                        "found"
                    }
                    .to_owned(),
                    names,
                    more_names: false,
                }
            })
            .collect(),
    }
}

fn directory_fixture(
    cast: &Cast,
) -> (
    impl Fn(&[String]) -> Result<DirectoryLookupResponse, DirectoryLookupFailure>
    + Send
    + Sync
    + 'static,
    DirectoryCalls,
) {
    let calls = DirectoryCalls::default();
    let recorded = calls.clone();
    let known = BTreeMap::from([
        (
            hex_of(&cast.mailbox),
            ("mailbox-member@finite.vip", "mailbox"),
        ),
        (
            hex_of(&cast.agent),
            ("agent-bot@finite.vip", "managed_agent"),
        ),
        (
            hex_of(&cast.shared),
            ("shared-runtime@finite.vip", "mailbox"),
        ),
        // Would leak if the outsider's key were ever sent.
        (
            hex_of(&cast.outsider),
            ("outsider-directory@finite.vip", "mailbox"),
        ),
    ]);
    let lookup = move |keys: &[String]| {
        recorded.lock().unwrap().push(keys.to_vec());
        Ok(directory_answer(keys, &known))
    };
    (lookup, calls)
}

/// NIP-05 fixture: synthetic stored names resolve forward to the given keys;
/// records every fetched URL so tests can prove which names were checked.
fn nip05_fixture(state: &mut ServerState, names: Vec<(&'static str, String)>) -> Nip05Urls {
    let urls = Nip05Urls::default();
    let recorded = urls.clone();
    state.nip05_fetcher = Arc::new(move |request| {
        recorded.lock().unwrap().push(request.url.clone());
        for (local, hex) in &names {
            if request.url.ends_with(&format!("?name={local}")) {
                let mut document = serde_json::Map::new();
                document.insert((*local).to_owned(), serde_json::Value::String(hex.clone()));
                return Ok(serde_json::json!({ "names": document })
                    .to_string()
                    .into_bytes());
            }
        }
        Err("synthetic NIP-05 outage".to_owned())
    });
    urls
}

async fn get_report_for(
    router: &Router,
    keys: &Keys,
    brain: &str,
    query: &str,
    created_at: u64,
) -> (StatusCode, serde_json::Value) {
    let response = authed_request(
        router.clone(),
        keys,
        "GET",
        &format!("/v1/brains/{brain}/access-report{query}"),
        None,
        created_at,
    )
    .await;
    let status = response.status();
    let body = read_json_with_limit::<serde_json::Value>(response, REPORT_BODY_LIMIT).await;
    (status, body)
}

async fn get_report(
    router: &Router,
    keys: &Keys,
    query: &str,
    created_at: u64,
) -> (StatusCode, serde_json::Value) {
    get_report_for(router, keys, "acme", query, created_at).await
}

async fn put_folder_grant(router: &Router, admin: &Keys, folder: &str, target: &str, at: u64) {
    let change_id = format!("grant-{folder}-{}", &target[target.len() - 8..]);
    let response = authed_request(
        router.clone(),
        admin,
        "PUT",
        &format!("/v1/admin/brains/acme/folders/{folder}/access/{target}"),
        Some(
            serde_json::json!({
                "grant": folder_key_grant_value(&change_id, 1, target),
                "accessChangeEvent": admin_event(admin, "acme", &change_id,
                    AdminAccessAction::GrantFolderAccess, Some(folder), Some(target), Some(1)),
            })
            .to_string(),
        ),
        at,
    )
    .await;
    let status = response.status();
    let text = read_text(response).await;
    assert_eq!(status, StatusCode::OK, "{text}");
}

fn redeem_token_in(
    store: &mut BrainStore,
    admin: &Keys,
    redeemer: &UserId,
    role: BrainInviteTokenRole,
    label: &str,
) {
    let brain = BrainId::new("acme").unwrap();
    let hash = sha256_hex(format!("synthetic-token-{label}"));
    store
        .create_brain_invite_token(
            &brain,
            &hash,
            role,
            &user(admin),
            &format_unix_timestamp(TEST_NOW + 86_400).unwrap(),
            &test_rfc3339(),
        )
        .unwrap();
    store
        .redeem_brain_invite_token(&hash, redeemer, &test_rfc3339())
        .unwrap();
}

fn redeem_token(state: &ServerState, admin: &Keys, redeemer: &Keys, label: &str) {
    let mut store = state.store.lock().unwrap();
    redeem_token_in(
        &mut store,
        admin,
        &user(redeemer),
        BrainInviteTokenRole::Member,
        label,
    );
}

fn record_alias(state: &ServerState, keys: &Keys, nip05: &str) {
    state
        .store
        .lock()
        .unwrap()
        .record_identity_alias(&IdentityAlias {
            npub: user(keys),
            hex_public_key: hex_of(keys),
            preferred_nip05: Some(nip05.to_owned()),
            nip05_verified_at: Some("2026-05-01T00:00:00Z".to_owned()),
            nip05_relays: Vec::new(),
            updated_at: "2026-05-01T00:00:00Z".to_owned(),
        })
        .unwrap();
}

/// Build the synthetic Brain: mailbox and managed-agent members, one key used
/// by two runtimes, a participating key known only by a stored name, replaced
/// keys, admin-added keys (one with a preexisting stored alias), a direct
/// Guest, a demoted admin, a removed member, and a Mount participant.
async fn synthetic_brain(cast: &Cast) -> (ServerState, Router) {
    let state = test_state();
    let router = router_with_state(state.clone());
    assert_eq!(
        post_brain(
            router.clone(),
            &cast.admin,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await
        .status(),
        StatusCode::OK
    );
    add_test_org_folders(&router, &cast.admin).await;
    let brain = BrainId::new("acme").unwrap();

    redeem_token(&state, &cast.admin, &cast.mailbox, "mailbox");
    redeem_token(&state, &cast.admin, &cast.agent, "agent");
    redeem_token(&state, &cast.admin, &cast.unconfirmed, "unconfirmed");
    // Two runtimes sharing one key both redeem: still one Member Identity.
    redeem_token(&state, &cast.admin, &cast.shared, "shared-runtime-one");
    redeem_token(&state, &cast.admin, &cast.shared, "shared-runtime-two");
    {
        let mut store = state.store.lock().unwrap();
        for keys in [
            &cast.replaced,
            &cast.replacement,
            &cast.arbitrary,
            &cast.outsider,
            &cast.demoted,
        ] {
            store.add_member(&brain, &user(keys)).unwrap();
        }
        store.add_admin(&brain, &user(&cast.demoted)).unwrap();
    }
    put_folder_grant(
        &router,
        &cast.admin,
        "getting-started",
        &npub(&cast.shared),
        TEST_NOW + 1,
    )
    .await;
    put_folder_grant(
        &router,
        &cast.admin,
        "getting-started",
        &npub(&cast.replaced),
        TEST_NOW + 2,
    )
    .await;
    put_folder_grant(
        &router,
        &cast.admin,
        "restricted",
        &npub(&cast.guest),
        TEST_NOW + 3,
    )
    .await;
    put_folder_grant(
        &router,
        &cast.admin,
        "restricted",
        &npub(&cast.demoted),
        TEST_NOW + 4,
    )
    .await;
    {
        let mut store = state.store.lock().unwrap();
        // Removed and demoted without rotation: their current grants remain.
        store.remove_member(&brain, &user(&cast.replaced)).unwrap();
        store.remove_admin(&brain, &user(&cast.demoted)).unwrap();
    }

    // A destination Organization Brain mounts the restricted Folder.
    assert_eq!(
        post_brain(
            router.clone(),
            &cast.mount_admin,
            &create_brain_body("dest", "organization"),
            TEST_NOW,
            None,
            None,
            None,
        )
        .await
        .status(),
        StatusCode::OK
    );
    let mount_admin = npub(&cast.mount_admin);
    let offer = authed_request(
        router.clone(),
        &cast.admin,
        "POST",
        "/v1/brains/acme/folders/restricted/mount-offers",
        Some(
            serde_json::json!({
                "destinationBrainId": "dest",
                "destinationControllerNpub": mount_admin,
                "grant": folder_key_grant_value("grant-restricted-mount-v1", 1, &mount_admin),
                "accessChangeEvent": admin_event(&cast.admin, "acme", "mount-offer-restricted",
                    AdminAccessAction::GrantFolderAccess, Some("restricted"), Some(&mount_admin), Some(1)),
                "expiresAt": "2026-06-04T20:26:40Z",
            })
            .to_string(),
        ),
        TEST_NOW + 5,
    )
    .await;
    assert_eq!(offer.status(), StatusCode::OK);
    let offer: MountOfferResponse = read_json(offer).await;
    let accept = authed_request(
        router.clone(),
        &cast.mount_admin,
        "POST",
        &format!("/v1/mount-offers/{}/accept", offer.id),
        Some(String::new()),
        TEST_NOW + 6,
    )
    .await;
    assert_eq!(accept.status(), StatusCode::OK);

    // Stored labels last, so route-side alias refreshes cannot overwrite
    // them. The outsider's alias predates the report: anyone's lookup may
    // have written it, and it must never surface for an admin-added key.
    record_alias(&state, &cast.unconfirmed, "unconfirmed-name@example.test");
    record_alias(&state, &cast.replaced, "replaced-name@example.test");
    record_alias(&state, &cast.replacement, "replacement-name@example.test");
    record_alias(&state, &cast.outsider, "outsider-name@example.test");
    (state, router)
}

fn row<'a>(report: &'a serde_json::Value, keys: &Keys) -> &'a serde_json::Value {
    let npub = npub(keys);
    let rows = report["identities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["npub"] == npub.as_str())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "exactly one row per exact key: {npub}");
    rows[0]
}

fn folder<'a>(row: &'a serde_json::Value, folder_id: &str) -> Option<&'a serde_json::Value> {
    row["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["folderId"] == folder_id)
}

fn durable_state(state: &ServerState, cast: &Cast) -> impl PartialEq + std::fmt::Debug {
    let brain = BrainId::new("acme").unwrap();
    let store = state.store.lock().unwrap();
    let keys = cast.all().into_iter().map(user).collect::<Vec<_>>();
    (
        store.load_brain(&brain).unwrap(),
        store.load_identity_aliases(&keys).unwrap(),
        store.latest_sequence(&brain).unwrap(),
        store.pull_sync_records(&brain, 0, 1_000).unwrap(),
        store.list_brain_invite_tokens(&brain).unwrap(),
    )
}

#[tokio::test]
async fn access_report_names_exact_keys_with_evidence_and_honest_coverage() {
    let cast = Cast::new();
    let (mut state, _) = synthetic_brain(&cast).await;
    let (lookup, directory_calls) = directory_fixture(&cast);
    let nip05_urls = nip05_fixture(
        &mut state,
        vec![
            ("unconfirmed-name", hex_of(&cast.unconfirmed)),
            ("replacement-name", hex_of(&cast.replacement)),
            ("outsider-name", hex_of(&cast.outsider)),
        ],
    );
    let state = state.with_directory_name_fixture(lookup);
    let router = router_with_state(state.clone());
    let before = durable_state(&state, &cast);

    let (status, report) = get_report(&router, &cast.admin, "", TEST_NOW + 10).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["version"], "finite-brain-access-report-v1");
    assert_eq!(report["brainId"], "acme");
    assert_eq!(report["actingKey"]["npub"], npub(&cast.admin));
    assert_eq!(report["actingKey"]["brainRole"], "admin");
    assert!(report["authoritySequence"].as_u64().unwrap() > 0);
    assert_eq!(report["authorityFingerprint"].as_str().unwrap().len(), 64);
    assert_eq!(
        report["checkedAt"],
        format_unix_timestamp(TEST_NOW).unwrap()
    );
    // acme only shares out through an active, fully sourced Mount.
    for scope in ["members", "guests", "mounts", "currentGrants"] {
        assert_eq!(report["coverage"][scope]["state"], "complete", "{scope}");
    }
    assert_eq!(report["coverage"]["accessHistory"]["state"], "unsupported");
    assert_eq!(report["currentAccessComplete"], true);
    assert_eq!(report["incomingMounts"], serde_json::json!([]));
    assert_eq!(report["directory"]["state"], "checked");
    assert_eq!(report["totals"]["identities"], 12);
    assert_eq!(report["totals"]["revocationIncomplete"], 2);
    assert_eq!(report["totals"]["grantsMissing"], 7);
    assert_eq!(report["identities"].as_array().unwrap().len(), 12);
    assert!(report["page"].get("next").is_none());

    let mailbox = row(&report, &cast.mailbox);
    assert_eq!(mailbox["brainRole"], "member");
    assert_eq!(mailbox["name"]["state"], "verified");
    assert_eq!(mailbox["name"]["label"], "mailbox-member@finite.vip");
    assert_eq!(mailbox["name"]["kind"], "mailbox");
    assert_eq!(mailbox["name"]["matchedKey"], hex_of(&cast.mailbox));
    assert_eq!(mailbox["identityType"]["label"], "Type not confirmed");
    assert_eq!(mailbox["participation"]["kind"], "inviteTokenRedemption");
    assert_eq!(mailbox["membership"]["origin"], "invitation");
    assert_eq!(
        folder(mailbox, "getting-started").unwrap()["state"],
        "grantMissing"
    );

    let agent = row(&report, &cast.agent);
    assert_eq!(agent["identityType"]["label"], "Managed Agent");
    assert_eq!(agent["identityType"]["value"], "managedAgent");
    assert_eq!(agent["name"]["label"], "agent-bot@finite.vip");

    let shared = row(&report, &cast.shared);
    assert_eq!(shared["brainRole"], "member");
    assert_eq!(shared["name"]["label"], "shared-runtime@finite.vip");
    let shared_folder = folder(shared, "getting-started").unwrap();
    assert_eq!(shared_folder["state"], "ready");
    assert_eq!(shared_folder["currentGrant"], "present");
    assert_eq!(shared_folder["grant"]["issuedBy"], npub(&cast.admin));
    // The stored signed change grants exactly this Folder, key, and version.
    assert_eq!(shared_folder["grant"]["signedAudit"], "verified");
    assert_eq!(
        shared_folder["grant"]["signedAuditActor"],
        npub(&cast.admin)
    );
    assert_eq!(shared_folder["grant"]["provenance"]["origin"], "direct");

    // The admin's own grant was written at Folder creation, under a
    // set-folder-access-mode change at most: never verified as a grant.
    let admin_row = row(&report, &cast.admin);
    let creation_audit = &folder(admin_row, "getting-started").unwrap()["grant"]["signedAudit"];
    assert!(
        creation_audit == "mismatched" || creation_audit == "notStored",
        "{creation_audit}"
    );

    // Participating, no Directory name: its stored name is checked forward.
    let unconfirmed = row(&report, &cast.unconfirmed);
    assert_eq!(unconfirmed["name"]["state"], "domainClaimed");
    assert_eq!(unconfirmed["name"]["source"], "nip05Forward");
    assert_eq!(unconfirmed["identityType"]["value"], "notConfirmed");
    assert_eq!(
        unconfirmed["participation"]["kind"],
        "inviteTokenRedemption"
    );

    // Admin-added keys never get a name, even with a stored alias that
    // would verify forward: adding a key must not unlock name discovery.
    for keys in [
        &cast.replaced,
        &cast.replacement,
        &cast.arbitrary,
        &cast.outsider,
    ] {
        let identity = row(&report, keys);
        assert!(identity.get("participation").is_none());
        assert_eq!(identity["name"]["state"], "unknown");
        assert_eq!(identity["name"]["display"], "Unknown name");
        assert_eq!(identity["identityType"]["value"], "notConfirmed");
    }
    let text = report.to_string();
    for leaked in [
        "outsider-name@example.test",
        "outsider-directory@finite.vip",
        "replaced-name@example.test",
        "replacement-name@example.test",
    ] {
        assert!(!text.contains(leaked), "{leaked}");
    }
    let replaced = row(&report, &cast.replaced);
    assert_eq!(replaced["brainRole"], "noCurrentRole");
    assert_eq!(
        replaced["roleSources"],
        serde_json::json!(["currentGrantOnly"])
    );
    assert_eq!(
        replaced["revocationIncomplete"],
        serde_json::json!(["getting-started"])
    );
    assert_eq!(
        folder(replaced, "getting-started").unwrap()["state"],
        "revocationIncomplete"
    );
    assert_eq!(row(&report, &cast.replacement)["brainRole"], "member");

    let guest = row(&report, &cast.guest);
    assert_eq!(guest["brainRole"], "guest");
    let guest_folder = folder(guest, "restricted").unwrap();
    assert_eq!(
        guest_folder["entitlementSources"],
        serde_json::json!(["explicitAccess"])
    );
    assert_eq!(guest_folder["state"], "ready");
    assert!(folder(guest, "getting-started").is_none());

    let demoted = row(&report, &cast.demoted);
    assert_eq!(demoted["brainRole"], "member");
    assert_eq!(
        demoted["revocationIncomplete"],
        serde_json::json!(["restricted"])
    );
    assert_eq!(
        demoted["missingCurrentGrants"],
        serde_json::json!(["getting-started"])
    );
    let demoted_restricted = folder(demoted, "restricted").unwrap();
    assert_eq!(demoted_restricted["entitled"], false);
    assert_eq!(demoted_restricted["currentGrant"], "present");

    // Mount participation comes from the recorded access source.
    let mount = row(&report, &cast.mount_admin);
    assert_eq!(mount["brainRole"], "mountParticipant");
    let mount_folder = folder(mount, "restricted").unwrap();
    let sources = mount_folder["entitlementSources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert!(sources[0].as_str().unwrap().starts_with("mount:"));
    assert_eq!(mount_folder["grant"]["signedAudit"], "verified");
    assert!(
        mount["roleSources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source.as_str().unwrap().ends_with("@dest"))
    );

    // Only keys with recorded participation by that exact key reached the
    // Directory or a stored-name recheck.
    let requested = directory_calls
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>();
    for keys in [
        &cast.mailbox,
        &cast.agent,
        &cast.shared,
        &cast.unconfirmed,
        &cast.admin,
    ] {
        assert!(requested.contains(&hex_of(keys)));
    }
    for keys in [
        &cast.replaced,
        &cast.replacement,
        &cast.arbitrary,
        &cast.outsider,
        &cast.guest,
        &cast.demoted,
    ] {
        assert!(
            !requested.contains(&hex_of(keys)),
            "no reverse lookup for admin-added keys"
        );
    }
    let fetched = nip05_urls.lock().unwrap().clone();
    assert!(
        fetched
            .iter()
            .any(|url| url.ends_with("?name=unconfirmed-name"))
    );
    assert!(
        fetched
            .iter()
            .all(|url| !url.contains("outsider") && !url.contains("replace")),
        "{fetched:?}"
    );

    // A refresh observes the same authority and leaves durable state
    // byte-identical: no alias writes, no history, no grant changes.
    let (status, again) = get_report(&router, &cast.admin, "", TEST_NOW + 11).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        again["authorityFingerprint"],
        report["authorityFingerprint"]
    );
    assert_eq!(again["identities"], report["identities"]);
    assert_eq!(durable_state(&state, &cast), before);

    // The report's entitlement rule is the server's folder_visible rule, and
    // stored aliases are read only for participating page keys.
    let store = state.store.lock().unwrap();
    let acme = BrainId::new("acme").unwrap();
    let snapshot = store.access_report_snapshot(&acme, None, 100).unwrap();
    assert!(crate::access_report::entitlement_matches_folder_visible(
        &snapshot.authority
    ));
    assert!(!snapshot.aliases.contains_key(&user(&cast.outsider)));
    assert!(snapshot.aliases.contains_key(&user(&cast.unconfirmed)));
}

#[tokio::test]
async fn access_report_denies_non_admins_before_any_lookup() {
    let cast = Cast::new();
    let (mut state, _) = synthetic_brain(&cast).await;
    let (lookup, directory_calls) = directory_fixture(&cast);
    let nip05_urls = nip05_fixture(&mut state, Vec::new());
    let state = state.with_directory_name_fixture(lookup);
    let router = router_with_state(state.clone());
    let before = durable_state(&state, &cast);

    // An Organization managed-agent bot without the admin role is denied
    // like any other non-admin: no account-owner fallback.
    let outsider = Keys::generate();
    for (index, keys) in [
        &outsider,
        &cast.mailbox,
        &cast.agent,
        &cast.guest,
        &cast.demoted,
    ]
    .into_iter()
    .enumerate()
    {
        let (status, body) = get_report(&router, keys, "", TEST_NOW + 20 + index as u64).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body.get("identities").is_none());
    }
    let unsigned = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/brains/acme/access-report")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsigned.status(), StatusCode::FORBIDDEN);

    assert!(directory_calls.lock().unwrap().is_empty());
    assert!(nip05_urls.lock().unwrap().is_empty());
    assert_eq!(durable_state(&state, &cast), before);
}

#[tokio::test]
async fn personal_brain_report_admits_owner_and_its_delegated_personal_agent_only() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let router = router_with_state(personal_test_state(&owner, &agent));
    // The Personal Agent's standing is the canonical Personal Brain
    // delegation ensure_brain_admin already grants it, not account fallback.
    for (index, (keys, role)) in [(&owner, "owner"), (&agent, "personalAgent")]
        .into_iter()
        .enumerate()
    {
        let (status, report) =
            get_report_for(&router, keys, "personal", "", TEST_NOW + 1 + index as u64).await;
        assert_eq!(status, StatusCode::OK, "{report}");
        assert_eq!(report["actingKey"]["brainRole"], role);
        assert_eq!(report["brainKind"], "personal");
        // Clean native Brain with no Mounts: complete coverage.
        assert_eq!(report["coverage"]["mounts"]["state"], "complete");
        assert_eq!(report["currentAccessComplete"], true);
        assert_eq!(row(&report, &agent)["brainRole"], "personalAgent");
    }
    let (status, _) =
        get_report_for(&router, &Keys::generate(), "personal", "", TEST_NOW + 5).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn access_report_degrades_names_honestly_when_the_directory_cannot_answer() {
    let cast = Cast::new();
    let (base, _) = synthetic_brain(&cast).await;
    let wrong_key = hex_of(&Keys::generate());
    let cases = vec![
        ("notConfigured", None),
        (
            "unsupported",
            lookup_fn(|_| Err(DirectoryLookupFailure::Unsupported)),
        ),
        (
            "unavailable",
            lookup_fn(|_| {
                Err(DirectoryLookupFailure::Unavailable(
                    "synthetic outage".to_owned(),
                ))
            }),
        ),
        // A foreign key in place of a requested one.
        (
            "unavailable",
            lookup_fn(move |_| {
                Ok(DirectoryLookupResponse {
                    checked_at: Some(TEST_NOW),
                    results: vec![DirectoryKeyNames {
                        pubkey: wrong_key.clone(),
                        status: "found".to_owned(),
                        names: vec![DirectoryName {
                            name: "someone-else@finite.vip".to_owned(),
                            kind: "managed_agent".to_owned(),
                            source: None,
                            bound_at: None,
                        }],
                        more_names: false,
                    }],
                })
            }),
        ),
        // A partial answer: only the first requested key, falsely named.
        (
            "unavailable",
            lookup_fn(|keys| {
                let known = BTreeMap::from([(
                    keys[0].clone(),
                    ("someone-else@finite.vip", "managed_agent"),
                )]);
                let mut answer = directory_answer(&keys[..1], &known);
                answer.checked_at = Some(TEST_NOW);
                Ok(answer)
            }),
        ),
    ];
    for (index, (expected, lookup)) in cases.into_iter().enumerate() {
        let mut state = base.clone();
        state.directory_names = lookup;
        // Forward rechecks are down too: stored names stay stored, not verified.
        nip05_fixture(&mut state, Vec::new());
        let router = router_with_state(state);
        let (status, report) =
            get_report(&router, &cast.admin, "", TEST_NOW + 30 + index as u64).await;
        assert_eq!(status, StatusCode::OK, "{report}");
        assert_eq!(report["directory"]["state"], expected, "{report}");
        assert_eq!(report["currentAccessComplete"], true);
        // Authority facts stay visible whatever the names do.
        assert_eq!(
            row(&report, &cast.demoted)["revocationIncomplete"],
            serde_json::json!(["restricted"])
        );
        assert_eq!(row(&report, &cast.agent)["brainRole"], "member");
        // Nothing becomes a Managed Agent, and no name is invented.
        for identity in report["identities"].as_array().unwrap() {
            assert_eq!(identity["identityType"]["value"], "notConfirmed");
            assert_ne!(identity["name"]["state"], "verified");
        }
        let unconfirmed = row(&report, &cast.unconfirmed);
        assert_eq!(unconfirmed["name"]["state"], "storedNotRechecked");
        assert_eq!(
            unconfirmed["name"]["display"],
            "Stored name, not rechecked: unconfirmed-name@example.test"
        );
        assert_eq!(
            unconfirmed["name"]["storedVerifiedAt"],
            "2026-05-01T00:00:00Z"
        );
        // A non-participant's stored alias stays hidden during outages too.
        assert_eq!(row(&report, &cast.outsider)["name"]["state"], "unknown");
        let text = report.to_string();
        assert!(!text.contains("someone-else@finite.vip"));
        assert!(!text.contains("outsider-name@example.test"));
    }
}

#[tokio::test]
async fn access_report_rebuilds_when_a_token_redemption_lands_during_name_lookup() {
    let cast = Cast::new();
    let (base, _) = synthetic_brain(&cast).await;
    let acme = BrainId::new("acme").unwrap();
    let sequence_before = base.store.lock().unwrap().latest_sequence(&acme).unwrap();
    let late_admin = Keys::generate();

    // The Directory "pauses" while an admin-role token is redeemed: that
    // writes Membership and Admin rows but appends no Brain record.
    let store = base.store.clone();
    let admin = cast.admin.clone();
    let late = user(&late_admin);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mut state = base
        .clone()
        .with_directory_name_fixture(move |keys: &[String]| {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                let mut store = store.lock().unwrap();
                redeem_token_in(
                    &mut store,
                    &admin,
                    &late,
                    BrainInviteTokenRole::Admin,
                    "late-admin",
                );
            }
            Ok(directory_answer(keys, &BTreeMap::new()))
        });
    nip05_fixture(&mut state, Vec::new());
    let router = router_with_state(state.clone());

    let (status, report) = get_report(&router, &cast.admin, "", TEST_NOW + 40).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(
        state.store.lock().unwrap().latest_sequence(&acme).unwrap(),
        sequence_before,
        "the race appended no record, so a sequence check could not see it"
    );
    assert_eq!(report["authoritySequence"], sequence_before);
    // The report was rebuilt from the new authority, not returned stale.
    assert_eq!(row(&report, &late_admin)["brainRole"], "admin");
    assert_eq!(report["totals"]["identities"], 13);
    assert!(calls.load(Ordering::SeqCst) >= 2);

    // If authority keeps changing, the caller is told to retry.
    let store = base.store.clone();
    let admin = cast.admin.clone();
    let churn = Arc::new(AtomicUsize::new(0));
    let mut state = base.with_directory_name_fixture(move |keys: &[String]| {
        let round = churn.fetch_add(1, Ordering::SeqCst);
        let key = UserId::new(npub(&Keys::generate())).unwrap();
        let mut store = store.lock().unwrap();
        redeem_token_in(
            &mut store,
            &admin,
            &key,
            BrainInviteTokenRole::Member,
            &format!("churn-{round}"),
        );
        Ok(directory_answer(keys, &BTreeMap::new()))
    });
    nip05_fixture(&mut state, Vec::new());
    let router = router_with_state(state);
    let (status, body) = get_report(&router, &cast.admin, "", TEST_NOW + 41).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.get("identities").is_none());
}

#[tokio::test]
async fn access_report_pages_are_stable_bounded_and_bound_to_one_authority() {
    let cast = Cast::new();
    let (base, _) = synthetic_brain(&cast).await;
    // Seventy more participating members force two Directory batches.
    let extra = (0..70).map(|_| Keys::generate()).collect::<Vec<_>>();
    for (index, keys) in extra.iter().enumerate() {
        redeem_token(&base, &cast.admin, keys, &format!("bulk-{index}"));
    }
    let calls = DirectoryCalls::default();
    let recorded = calls.clone();
    let mut state = base.with_directory_name_fixture(move |keys: &[String]| {
        recorded.lock().unwrap().push(keys.to_vec());
        Ok(directory_answer(keys, &BTreeMap::new()))
    });
    nip05_fixture(&mut state, Vec::new());
    let router = router_with_state(state.clone());

    let (status, full) = get_report(&router, &cast.admin, "?limit=100", TEST_NOW + 50).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(full["totals"]["identities"], 82);
    assert_eq!(full["page"]["returned"], 82);
    let batches = calls.lock().unwrap().clone();
    assert!(batches.len() >= 2);
    assert!(batches.iter().all(|batch| batch.len() <= 64));

    let fingerprint = full["authorityFingerprint"].as_str().unwrap().to_owned();
    let mut paged = Vec::new();
    let mut after = String::new();
    for page in 0.. {
        assert!(page < 20, "pagination must terminate");
        let query = if after.is_empty() {
            "?limit=7".to_owned()
        } else {
            format!("?limit=7&after={after}")
        };
        let (status, body) = get_report(&router, &cast.admin, &query, TEST_NOW + 10 + page).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["totals"], full["totals"]);
        assert_eq!(body["authorityFingerprint"], fingerprint);
        assert!(body["page"]["returned"].as_u64().unwrap() <= 7);
        paged.extend(
            body["identities"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["npub"].as_str().unwrap().to_owned()),
        );
        match body["page"]["next"].as_str() {
            Some(next) => {
                assert!(next.starts_with(&format!("{fingerprint}.")));
                after = next.to_owned();
            }
            None => break,
        }
    }
    let expected = full["identities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["npub"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(paged, expected, "pages are disjoint, ordered, and complete");

    // Per-key evidence is read for the page only, never the whole Brain.
    {
        let store = state.store.lock().unwrap();
        let snapshot = store
            .access_report_snapshot(&BrainId::new("acme").unwrap(), None, 7)
            .unwrap();
        assert_eq!(snapshot.evidence_keys.len(), 7);
        assert!(
            snapshot
                .verified_participation
                .keys()
                .all(|key| snapshot.evidence_keys.contains(key))
        );
        assert!(
            snapshot
                .aliases
                .keys()
                .all(|key| snapshot.verified_participation.contains_key(key))
        );
        assert!(
            snapshot
                .member_provenance
                .keys()
                .all(|key| snapshot.evidence_keys.contains(key))
        );
    }

    // A cursor issued under older authority is refused, even though no
    // record was appended (a Member added directly writes none).
    let (_, first) = get_report(&router, &cast.admin, "?limit=7", TEST_NOW + 35).await;
    let stale = first["page"]["next"].as_str().unwrap().to_owned();
    state
        .store
        .lock()
        .unwrap()
        .add_member(&BrainId::new("acme").unwrap(), &user(&Keys::generate()))
        .unwrap();
    let (status, body) = get_report(
        &router,
        &cast.admin,
        &format!("?limit=7&after={stale}"),
        TEST_NOW + 36,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let forged = format!("{}.{}", "0".repeat(64), stale.split_once('.').unwrap().1);
    let (status, _) = get_report(
        &router,
        &cast.admin,
        &format!("?limit=7&after={forged}"),
        TEST_NOW + 37,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let bare_npub = format!("?after={}", npub(&cast.admin));
    for (index, query) in [
        "?limit=0",
        "?limit=101",
        "?after=not-a-cursor",
        bare_npub.as_str(),
    ]
    .into_iter()
    .enumerate()
    {
        let (status, _) =
            get_report(&router, &cast.admin, query, TEST_NOW + 40 + index as u64).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
    }
}

#[tokio::test]
async fn incoming_mounts_are_reported_and_never_claimed_complete() {
    let cast = Cast::new();
    let (mut state, _) = synthetic_brain(&cast).await;
    nip05_fixture(&mut state, Vec::new());
    let router = router_with_state(state);

    let (status, report) =
        get_report_for(&router, &cast.mount_admin, "dest", "", TEST_NOW + 60).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["coverage"]["mounts"]["state"], "unverified");
    let reason = report["coverage"]["mounts"]["reason"].as_str().unwrap();
    assert!(reason.contains("mounted into this Brain"), "{reason}");
    assert!(reason.contains("acme/restricted"), "{reason}");
    assert_eq!(report["currentAccessComplete"], false);

    let mounts = report["incomingMounts"].as_array().unwrap();
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0]["sourceBrainId"], "acme");
    assert_eq!(mounts[0]["sourceFolderId"], "restricted");
    assert_eq!(mounts[0]["connectionStatus"], "active");
    assert_eq!(mounts[0]["participantDetail"], "complete");
    assert_eq!(mounts[0]["participants"], 1);

    let participant = row(&report, &cast.mount_admin);
    let entry = &participant["incomingMounts"][0];
    assert_eq!(entry["mountAccess"], true);
    assert_eq!(entry["currentGrant"], "present");
    assert_eq!(entry["state"], "ready");
    assert!(
        participant["roleSources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source.as_str().unwrap().starts_with("incomingMount:"))
    );

    // Source-Brain identities unrelated to the Mount never appear.
    let text = report.to_string();
    for keys in [&cast.admin, &cast.guest, &cast.demoted, &cast.mailbox] {
        assert!(!text.contains(&npub(keys)));
    }
}

#[test]
fn signed_grant_audit_verifies_only_an_exactly_matching_grant_change() {
    use crate::access_report::{SignedAudit, signed_grant_audit};
    use finite_brain_store::{AccessReportGrant, GrantProvenance};

    let admin = Keys::generate();
    let recipient = npub(&Keys::generate());
    let other = npub(&Keys::generate());
    let grant = |event: Option<String>| AccessReportGrant {
        id: "grant-synthetic".to_owned(),
        folder_id: FolderId::new("restricted").unwrap(),
        key_version: 2,
        issuer_npub: user(&admin),
        recipient_npub: UserId::new(recipient.clone()).unwrap(),
        access_change_event_json: event,
        created_at: test_rfc3339(),
        provenance: GrantProvenance::direct(),
    };
    let event = |brain: &str, action, folder: &str, target: &str, version| {
        Some(
            admin_event(
                &admin,
                brain,
                "change-synthetic",
                action,
                Some(folder),
                Some(target),
                Some(version),
            )
            .as_json(),
        )
    };
    let grant_action = AdminAccessAction::GrantFolderAccess;

    let exact = signed_grant_audit(
        "acme",
        &grant(event("acme", grant_action, "restricted", &recipient, 2)),
    );
    assert!(matches!(exact, SignedAudit::Verified { ref signer, .. } if *signer == npub(&admin)));

    for mismatched in [
        event("acme", grant_action, "restricted", &other, 2),
        event("acme", grant_action, "getting-started", &recipient, 2),
        event("acme", grant_action, "restricted", &recipient, 1),
        event("other-brain", grant_action, "restricted", &recipient, 2),
        event(
            "acme",
            AdminAccessAction::RemoveFolderAccess,
            "restricted",
            &recipient,
            2,
        ),
    ] {
        assert_eq!(
            signed_grant_audit("acme", &grant(mismatched)).state(),
            "mismatched"
        );
    }

    let mut forged_issuer = grant(event("acme", grant_action, "restricted", &recipient, 2));
    forged_issuer.issuer_npub = UserId::new(other).unwrap();
    assert_eq!(
        signed_grant_audit("acme", &forged_issuer).state(),
        "mismatched"
    );
    forged_issuer.provenance.delegated_by_npub = Some(user(&admin));
    assert_eq!(
        signed_grant_audit("acme", &forged_issuer).state(),
        "verified"
    );

    assert_eq!(
        signed_grant_audit("acme", &grant(None)).state(),
        "notStored"
    );
    let mut tampered: serde_json::Value =
        serde_json::from_str(&event("acme", grant_action, "restricted", &recipient, 2).unwrap())
            .unwrap();
    tampered["content"] = tampered["content"]
        .as_str()
        .unwrap()
        .replace("restricted", "restrictex")
        .into();
    for unreadable in [Some("not json".to_owned()), Some(tampered.to_string())] {
        assert_eq!(
            signed_grant_audit("acme", &grant(unreadable)).state(),
            "unreadable"
        );
    }
}

#[tokio::test]
async fn existing_access_metadata_route_keeps_its_shape() {
    let cast = Cast::new();
    let (_, router) = synthetic_brain(&cast).await;
    let response = authed_request(
        router,
        &cast.admin,
        "GET",
        "/v1/brains/acme/access",
        None,
        TEST_NOW + 20,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let metadata: BrainMetadataResponse = read_json_with_limit(response, REPORT_BODY_LIMIT).await;
    assert_eq!(metadata.brain_id, "acme");
}

#[tokio::test]
async fn failed_non_admin_lookup_cannot_turn_domain_claim_into_verified_identity() {
    let cast = Cast::new();
    let (mut state, _) = synthetic_brain(&cast).await;
    nip05_fixture(&mut state, vec![("attacker-label", hex_of(&cast.mailbox))]);
    let router = router_with_state(state.clone());
    // Existing identity resolution records a global alias before the route
    // checks admin standing. That label is a domain claim, even for a member
    // whose participation was independently recorded.
    let body = serde_json::json!({ "accessChangeEvent": admin_event(
        &cast.outsider, "acme", "untrusted-label-write", AdminAccessAction::AddMember,
        None, Some(&npub(&cast.mailbox)), None,
    ) });
    let response = authed_request(
        router.clone(),
        &cast.outsider,
        "PUT",
        "/v1/admin/brains/acme/members/attacker-label@evil.example",
        Some(body.to_string()),
        TEST_NOW + 7,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let alias = state
        .store
        .lock()
        .unwrap()
        .load_identity_aliases(&[user(&cast.mailbox)])
        .unwrap();
    assert_eq!(
        alias
            .iter()
            .find(|alias| alias.npub == user(&cast.mailbox))
            .unwrap()
            .preferred_nip05
            .as_deref(),
        Some("attacker-label@evil.example")
    );
    let (status, report) = get_report(&router, &cast.admin, "", TEST_NOW + 8).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    let evidence = &row(&report, &cast.mailbox)["name"];
    assert_eq!(evidence["state"], "domainClaimed");
    assert!(
        evidence["reason"]
            .as_str()
            .unwrap()
            .contains("key holder has not confirmed")
    );
}

#[tokio::test]
async fn access_report_does_not_retry_or_expire_cursor_for_ordinary_content_writes() {
    use finite_brain_store::{FolderObjectRevisionSyncRecord, SyncRecordInput};
    let cast = Cast::new();
    let (base, _) = synthetic_brain(&cast).await;
    let store = base.store.clone();
    let actor = user(&cast.admin);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mut state = base.with_directory_name_fixture(move |keys: &[String]| {
        let index = counted.fetch_add(1, Ordering::SeqCst);
        // Store submissions are the validated content writer's persistence
        // boundary. They append real projected objects and records, without
        // modifying access authority.
        store
            .lock()
            .unwrap()
            .submit_sync_record(
                &BrainId::new("acme").unwrap(),
                &SyncRecordInput::FolderObjectRevision(FolderObjectRevisionSyncRecord {
                    record_event_id: format!("content-report-race-{index}"),
                    folder_id: FolderId::new("getting-started").unwrap(),
                    object_id: ObjectId::new(format!("obj_{index:012}")).unwrap(),
                    revision: 1,
                    base_revision: None,
                    actor_npub: actor.clone(),
                    client_created_at: test_rfc3339(),
                    payload_json: "{\"ciphertext\":\"synthetic\"}".to_owned(),
                    record_event_kind: APP_SPECIFIC_KIND,
                }),
            )
            .unwrap();
        Ok(directory_answer(keys, &BTreeMap::new()))
    });
    nip05_fixture(&mut state, Vec::new());
    let router = router_with_state(state);
    let (status, first) = get_report(&router, &cast.admin, "?limit=7", TEST_NOW + 9).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "content edits must not force lookup retries"
    );
    let query = format!("?limit=7&after={}", first["page"]["next"].as_str().unwrap());
    let (status, second) = get_report(&router, &cast.admin, &query, TEST_NOW + 10).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(
        first["authorityFingerprint"],
        second["authorityFingerprint"]
    );
    assert_ne!(first["authoritySequence"], second["authoritySequence"]);
}

#[tokio::test]
async fn caller_losing_admin_during_lookup_receives_no_report() {
    let cast = Cast::new();
    let (base, _) = synthetic_brain(&cast).await;
    let brain = BrainId::new("acme").unwrap();
    base.store
        .lock()
        .unwrap()
        .add_admin(&brain, &user(&cast.mailbox))
        .unwrap();
    let store = base.store.clone();
    let caller = user(&cast.admin);
    let mut state = base.with_directory_name_fixture(move |keys: &[String]| {
        store.lock().unwrap().remove_admin(&brain, &caller).unwrap();
        Ok(directory_answer(keys, &BTreeMap::new()))
    });
    nip05_fixture(&mut state, Vec::new());
    let (status, report) =
        get_report(&router_with_state(state), &cast.admin, "", TEST_NOW + 11).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{report}");
    assert!(report.get("identities").is_none());
}

#[test]
fn default_nip05_transport_has_total_deadline_even_when_response_drips() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0u8; 4096];
        assert!(stream.read(&mut request).unwrap() > 0);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n")
            .unwrap();
        for _ in 0..28 {
            if stream.write_all(b" ").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    let request = Nip05WellKnownRequest {
        url: format!("http://{address}/.well-known/nostr.json?name=synthetic"),
        max_response_bytes: 4096,
        follow_redirects: false,
    };
    let started = std::time::Instant::now();
    assert!(fetch_nip05_document(&request, &request.url).is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    peer.join().unwrap();
}
