//! Access report through the public signed router. Every identity, name, and
//! domain here is synthetic.

use super::*;
use crate::core_descriptions::{
    CORE_BATCH_KEYS, CoreDescription, CoreDescriptionsResponse, CoreLookupFailure,
    CoreLookupRequest, CoreResponsibleAccount, CoreSource,
};
use finite_brain_core::sha256_hex;
use std::sync::atomic::{AtomicUsize, Ordering};

const REPORT_BODY_LIMIT: usize = 1024 * 1024;

type CoreCalls = Arc<Mutex<Vec<Vec<String>>>>;
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
    lookup: impl Fn(&CoreLookupRequest) -> Result<CoreDescriptionsResponse, CoreLookupFailure>
    + Send
    + Sync
    + 'static,
) -> Option<CoreDescriptionClient> {
    Some(CoreDescriptionClient {
        brain_server: TEST_BASE_URL.trim_end_matches('/').to_owned(),
        lookup: Arc::new(lookup),
    })
}

fn hex_of(keys: &Keys) -> String {
    NostrPublicKey::from_protocol(keys.public_key()).to_hex()
}

fn user(keys: &Keys) -> UserId {
    UserId::new(npub(keys)).unwrap()
}

/// What synthetic Core knows about a key for this Brain's audience.
#[derive(Clone)]
enum Known {
    Human(&'static str),
    Agent(&'static str, &'static str),
    Ambiguous,
}

/// A complete Core answer: described keys as known, `notShared` for the rest.
fn core_answer(
    request: &CoreLookupRequest,
    known: &BTreeMap<String, Known>,
) -> CoreDescriptionsResponse {
    let source = |kind: &str| CoreSource {
        kind: kind.to_owned(),
        observed_at: "2026-05-01T00:00:00Z".to_owned(),
        revision: "synthetic-revision".to_owned(),
    };
    let bare = |key: &String, state: &str| CoreDescription {
        public_key_hex: key.clone(),
        state: state.to_owned(),
        kind: None,
        display_name: None,
        account_email: None,
        lifecycle: None,
        responsible_account: None,
        source: None,
    };
    CoreDescriptionsResponse {
        version: crate::core_descriptions::DESCRIPTIONS_VERSION.to_owned(),
        brain_id: request.brain_id.clone(),
        checked_at: "2026-05-02T00:00:00Z".to_owned(),
        results: request
            .keys
            .iter()
            .map(|key| match known.get(key) {
                None => bare(key, "notShared"),
                Some(Known::Ambiguous) => bare(key, "ambiguous"),
                Some(Known::Human(email)) => CoreDescription {
                    kind: Some("human".to_owned()),
                    account_email: Some((*email).to_owned()),
                    source: Some(source("hostedDeviceObservation")),
                    ..bare(key, "resolved")
                },
                Some(Known::Agent(name, owner)) => CoreDescription {
                    kind: Some("agent".to_owned()),
                    display_name: Some((*name).to_owned()),
                    lifecycle: Some("active".to_owned()),
                    responsible_account: Some(CoreResponsibleAccount {
                        email: (*owner).to_owned(),
                        source: "coreAccountContact".to_owned(),
                        observed_at: "2026-05-01T00:00:00Z".to_owned(),
                        human_public_keys_hex: Vec::new(),
                    }),
                    source: Some(source("finiteRuntimeRecord")),
                    ..bare(key, "resolved")
                },
            })
            .collect(),
    }
}

fn core_fixture(
    cast: &Cast,
) -> (
    impl Fn(&CoreLookupRequest) -> Result<CoreDescriptionsResponse, CoreLookupFailure>
    + Send
    + Sync
    + 'static,
    CoreCalls,
) {
    let calls = CoreCalls::default();
    let recorded = calls.clone();
    let known = BTreeMap::from([
        (
            hex_of(&cast.mailbox),
            Known::Human("mailbox-member@acme.example"),
        ),
        (
            hex_of(&cast.agent),
            Known::Agent("Agent Bot", "owner@acme.example"),
        ),
        (hex_of(&cast.shared), Known::Ambiguous),
        (
            hex_of(&cast.mount_admin),
            Known::Human("mount-controller@dest.example"),
        ),
        // Would leak if the outsider's key were ever sent.
        (
            hex_of(&cast.outsider),
            Known::Human("outsider-core@example.org"),
        ),
    ]);
    let lookup = move |request: &CoreLookupRequest| {
        recorded.lock().unwrap().push(request.keys.clone());
        Ok(core_answer(request, &known))
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
async fn synthetic_brain(cast: &Cast) -> (ServerState, Router, tempfile::TempDir) {
    let scratch = tempfile::TempDir::new().unwrap();
    let database = scratch.path().join("legacy-report-fixture.sqlite3");
    let store = BrainStore::open(&database).unwrap();
    let state = ServerState::new(store, TEST_BASE_URL).with_auth_clock(TEST_NOW, 60);
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
        // Reproduce a historical database left by the pre-safe-removal writer.
        // Current removal APIs intentionally reject this retained-grant drift;
        // only this synthetic fixture uses raw role deletes. Foreign keys and
        // capacity-counter triggers remain enabled, and grants are untouched.
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        assert_eq!(
            connection
                .execute(
                    "DELETE FROM brain_members WHERE brain_id = ?1 AND user_id = ?2",
                    rusqlite::params![brain.as_str(), user(&cast.replaced).as_str()],
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .execute(
                    "DELETE FROM brain_admins WHERE brain_id = ?1 AND user_id = ?2",
                    rusqlite::params![brain.as_str(), user(&cast.demoted).as_str()],
                )
                .unwrap(),
            1
        );
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
    (state, router, scratch)
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
    let (mut state, _, _scratch) = synthetic_brain(&cast).await;
    let (lookup, core_calls) = core_fixture(&cast);
    let nip05_urls = nip05_fixture(
        &mut state,
        vec![
            ("unconfirmed-name", hex_of(&cast.unconfirmed)),
            ("replacement-name", hex_of(&cast.replacement)),
            ("outsider-name", hex_of(&cast.outsider)),
        ],
    );
    let state = state.with_core_description_fixture(lookup);
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
    assert_eq!(report["descriptions"]["state"], "checked");
    assert_eq!(report["descriptions"]["checkedAt"], "2026-05-02T00:00:00Z");
    assert_eq!(report["totals"]["identities"], 12);
    assert_eq!(report["totals"]["revocationIncomplete"], 2);
    assert_eq!(report["totals"]["grantsMissing"], 7);
    assert_eq!(report["identities"].as_array().unwrap().len(), 12);
    assert!(report["page"].get("next").is_none());

    let mailbox = row(&report, &cast.mailbox);
    assert_eq!(mailbox["brainRole"], "member");
    assert_eq!(mailbox["description"]["state"], "resolved");
    assert_eq!(mailbox["description"]["kind"], "human");
    assert_eq!(
        mailbox["description"]["accountEmail"],
        "mailbox-member@acme.example"
    );
    assert_eq!(
        mailbox["description"]["source"]["kind"],
        "hostedDeviceObservation"
    );
    assert_eq!(mailbox["participation"]["kind"], "inviteTokenRedemption");
    assert_eq!(mailbox["membership"]["origin"], "invitation");
    assert_eq!(
        folder(mailbox, "getting-started").unwrap()["state"],
        "grantMissing"
    );

    let agent = row(&report, &cast.agent);
    assert_eq!(agent["description"]["kind"], "agent");
    assert_eq!(agent["description"]["displayName"], "Agent Bot");
    assert_eq!(agent["description"]["lifecycle"], "active");
    assert_eq!(
        agent["description"]["responsibleAccount"]["email"],
        "owner@acme.example"
    );

    let shared = row(&report, &cast.shared);
    assert_eq!(shared["brainRole"], "member");
    // One exact key, conflicting Core records: no selected owner or contact.
    assert_eq!(shared["description"]["state"], "ambiguous");
    assert!(shared["description"].get("accountEmail").is_none());
    let shared_folder = folder(shared, "getting-started").unwrap();
    assert_eq!(shared_folder["state"], "ready");
    assert_eq!(shared_folder["currentGrant"], "present");
    assert_eq!(shared_folder["grant"]["issuedBy"], npub(&cast.admin));
    assert_eq!(shared_folder["grant"]["provenance"]["origin"], "direct");
    assert!(shared_folder["grant"].get("signedAudit").is_none());

    // Participating, unshared in Core: its stored public alias is shown as
    // a dated stored claim, never rechecked and never a mailbox.
    let unconfirmed = row(&report, &cast.unconfirmed);
    assert_eq!(unconfirmed["description"]["state"], "notShared");
    assert_eq!(
        unconfirmed["storedNip05"]["name"],
        "unconfirmed-name@example.test"
    );
    assert_eq!(
        unconfirmed["storedNip05"]["storedAt"],
        "2026-05-01T00:00:00Z"
    );
    assert_eq!(
        unconfirmed["participation"]["kind"],
        "inviteTokenRedemption"
    );

    // Admin-added keys never get a description or alias, even when Core or a
    // stored alias knows them: adding a key must not unlock discovery.
    for keys in [
        &cast.replaced,
        &cast.replacement,
        &cast.arbitrary,
        &cast.outsider,
    ] {
        let identity = row(&report, keys);
        assert!(identity.get("participation").is_none());
        assert_eq!(identity["description"]["state"], "notShared");
        assert_eq!(identity["description"]["reason"], "noParticipation");
        assert!(identity.get("storedNip05").is_none());
    }
    let text = report.to_string();
    for leaked in [
        "outsider-name@example.test",
        "outsider-core@example.org",
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
    assert_eq!(mount["participation"]["kind"], "mountOfferAcceptance");
    assert_eq!(mount["description"]["state"], "resolved");
    assert_eq!(
        mount["description"]["accountEmail"],
        "mount-controller@dest.example"
    );
    let mount_folder = folder(mount, "restricted").unwrap();
    let sources = mount_folder["entitlementSources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert!(sources[0].as_str().unwrap().starts_with("mount:"));
    assert!(
        mount["roleSources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source.as_str().unwrap().ends_with("@dest"))
    );

    // Only keys with recorded participation by that exact key reached Core.
    let requested = core_calls
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
    // The report makes no outbound NIP-05 request at all.
    assert!(nip05_urls.lock().unwrap().is_empty());

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
    let (mut state, _, _scratch) = synthetic_brain(&cast).await;
    let (lookup, core_calls) = core_fixture(&cast);
    let nip05_urls = nip05_fixture(&mut state, Vec::new());
    let state = state.with_core_description_fixture(lookup);
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

    assert!(core_calls.lock().unwrap().is_empty());
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
async fn access_report_keeps_every_row_when_core_cannot_describe() {
    let cast = Cast::new();
    let (base, _, _scratch) = synthetic_brain(&cast).await;
    let wrong_key = hex_of(&Keys::generate());
    let cases = vec![
        ("notConfigured", None),
        (
            "unsupported",
            lookup_fn(|_| Err(CoreLookupFailure::Unsupported)),
        ),
        (
            "unavailable",
            lookup_fn(|_| Err(CoreLookupFailure::Unauthorized)),
        ),
        (
            "unavailable",
            lookup_fn(|_| {
                Err(CoreLookupFailure::Unavailable(
                    "synthetic outage".to_owned(),
                ))
            }),
        ),
        // A foreign key in place of the requested ones.
        (
            "unavailable",
            lookup_fn(move |request| {
                let foreign = CoreLookupRequest {
                    keys: vec![wrong_key.clone()],
                    ..request.clone()
                };
                Ok(core_answer(
                    &foreign,
                    &BTreeMap::from([(
                        wrong_key.clone(),
                        Known::Human("someone-else@example.org"),
                    )]),
                ))
            }),
        ),
        // A partial answer: only the first requested key, falsely described.
        (
            "unavailable",
            lookup_fn(|request| {
                let partial = CoreLookupRequest {
                    keys: request.keys[..1].to_vec(),
                    ..request.clone()
                };
                let known = BTreeMap::from([(
                    request.keys[0].clone(),
                    Known::Human("someone-else@example.org"),
                )]);
                Ok(core_answer(&partial, &known))
            }),
        ),
        // Another protocol version.
        (
            "unavailable",
            lookup_fn(|request| {
                let mut answer = core_answer(request, &BTreeMap::new());
                answer.version = "finite-core-brain-identity-descriptions-v2".to_owned();
                Ok(answer)
            }),
        ),
    ];
    let full = {
        let mut state = base.clone();
        state.core_descriptions = lookup_fn(|request| Ok(core_answer(request, &BTreeMap::new())));
        let (status, report) =
            get_report(&router_with_state(state), &cast.admin, "", TEST_NOW + 29).await;
        assert_eq!(status, StatusCode::OK, "{report}");
        report
    };
    for (index, (expected, lookup)) in cases.into_iter().enumerate() {
        let mut state = base.clone();
        state.core_descriptions = lookup;
        let nip05_urls = nip05_fixture(&mut state, Vec::new());
        let router = router_with_state(state);
        let (status, report) =
            get_report(&router, &cast.admin, "", TEST_NOW + 30 + index as u64).await;
        assert_eq!(status, StatusCode::OK, "{report}");
        assert_eq!(report["descriptions"]["state"], expected, "{report}");
        assert_eq!(report["currentAccessComplete"], true);
        // Every access fact is identical whatever happens to descriptions.
        let strip = |report: &serde_json::Value| {
            report["identities"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    let mut row = row.clone();
                    row.as_object_mut().unwrap().remove("description");
                    row
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(strip(&report), strip(&full));
        assert_eq!(report["totals"], full["totals"]);
        // Participating keys are unavailable, never `unknown` or an empty
        // roster; non-participants stay withheld.
        for identity in report["identities"].as_array().unwrap() {
            let state = identity["description"]["state"].as_str().unwrap();
            if identity.get("participation").is_some() {
                assert_eq!(state, "unavailable", "{identity}");
            } else {
                assert_eq!(state, "notShared", "{identity}");
            }
        }
        // Stored aliases stay separate dated evidence during outages.
        assert_eq!(
            row(&report, &cast.unconfirmed)["storedNip05"]["name"],
            "unconfirmed-name@example.test"
        );
        assert!(row(&report, &cast.outsider).get("storedNip05").is_none());
        let text = report.to_string();
        assert!(!text.contains("someone-else@example.org"));
        assert!(!text.contains("outsider-name@example.test"));
        assert!(nip05_urls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn access_report_rebuilds_when_a_token_redemption_lands_during_name_lookup() {
    let cast = Cast::new();
    let (base, _, _scratch) = synthetic_brain(&cast).await;
    let acme = BrainId::new("acme").unwrap();
    let sequence_before = base.store.lock().unwrap().latest_sequence(&acme).unwrap();
    let late_admin = Keys::generate();

    // Core "pauses" while an admin-role token is redeemed: that
    // writes Membership and Admin rows but appends no Brain record.
    let store = base.store.clone();
    let admin = cast.admin.clone();
    let late = user(&late_admin);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mut state =
        base.clone()
            .with_core_description_fixture(move |request: &CoreLookupRequest| {
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
                Ok(core_answer(request, &BTreeMap::new()))
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
    let mut state = base.with_core_description_fixture(move |request: &CoreLookupRequest| {
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
        Ok(core_answer(request, &BTreeMap::new()))
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
    let (base, _, _scratch) = synthetic_brain(&cast).await;
    // Seventy more participating members; Core batches never exceed its bound.
    let extra = (0..70).map(|_| Keys::generate()).collect::<Vec<_>>();
    for (index, keys) in extra.iter().enumerate() {
        redeem_token(&base, &cast.admin, keys, &format!("bulk-{index}"));
    }
    let calls = CoreCalls::default();
    let recorded = calls.clone();
    let mut state = base.with_core_description_fixture(move |request: &CoreLookupRequest| {
        recorded.lock().unwrap().push(request.keys.clone());
        Ok(core_answer(request, &BTreeMap::new()))
    });
    nip05_fixture(&mut state, Vec::new());
    let router = router_with_state(state.clone());

    let (status, full) = get_report(&router, &cast.admin, "?limit=100", TEST_NOW + 50).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(full["totals"]["identities"], 82);
    assert_eq!(full["page"]["returned"], 82);
    let batches = calls.lock().unwrap().clone();
    assert!(!batches.is_empty());
    assert!(batches.iter().all(|batch| batch.len() <= CORE_BATCH_KEYS));

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
    let (mut state, _, _scratch) = synthetic_brain(&cast).await;
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

#[tokio::test]
async fn existing_access_metadata_route_keeps_its_shape() {
    let cast = Cast::new();
    let (_, router, _scratch) = synthetic_brain(&cast).await;
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
async fn failed_non_admin_lookup_leaves_only_a_stored_alias_never_a_description() {
    let cast = Cast::new();
    let (mut state, _, _scratch) = synthetic_brain(&cast).await;
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
    // The globally written alias is only a dated stored claim beside the row;
    // it never becomes the key's description.
    let mailbox = row(&report, &cast.mailbox);
    assert_eq!(
        mailbox["storedNip05"]["name"],
        "attacker-label@evil.example"
    );
    assert_eq!(report["descriptions"]["state"], "notConfigured");
    assert_eq!(mailbox["description"]["state"], "unavailable");
    assert!(mailbox["description"].get("accountEmail").is_none());
}

#[tokio::test]
async fn access_report_does_not_retry_or_expire_cursor_for_ordinary_content_writes() {
    use finite_brain_store::{FolderObjectRevisionSyncRecord, SyncRecordInput};
    let cast = Cast::new();
    let (base, _, _scratch) = synthetic_brain(&cast).await;
    let store = base.store.clone();
    let actor = user(&cast.admin);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mut state = base.with_core_description_fixture(move |request: &CoreLookupRequest| {
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
        Ok(core_answer(request, &BTreeMap::new()))
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
    let (base, _, _scratch) = synthetic_brain(&cast).await;
    let brain = BrainId::new("acme").unwrap();
    base.store
        .lock()
        .unwrap()
        .add_admin(&brain, &user(&cast.mailbox))
        .unwrap();
    let database = _scratch.path().join("legacy-report-fixture.sqlite3");
    let caller = user(&cast.admin);
    let mut state = base.with_core_description_fixture(move |request: &CoreLookupRequest| {
        // Model a concurrent historical writer losing the caller's role while
        // name lookup is in flight. This tests the report's fresh auth check,
        // without asking safe current mutation APIs to create unsafe drift.
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        assert_eq!(
            connection
                .execute(
                    "DELETE FROM brain_admins WHERE brain_id = ?1 AND user_id = ?2",
                    rusqlite::params![brain.as_str(), caller.as_str()],
                )
                .unwrap(),
            1
        );
        let remaining: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM brain_admins WHERE brain_id = ?1 AND user_id = ?2",
                rusqlite::params![brain.as_str(), caller.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
        Ok(core_answer(request, &BTreeMap::new()))
    });
    nip05_fixture(&mut state, Vec::new());
    let (status, report) =
        get_report(&router_with_state(state), &cast.admin, "", TEST_NOW + 11).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{report}");
    assert!(report.get("identities").is_none());
}

#[tokio::test]
async fn accepted_folder_invitation_enables_only_its_exact_recipient_description() {
    let admin = Keys::generate();
    let recipient = Keys::generate();
    let pending = Keys::generate();
    let wrong = Keys::generate();
    let calls = CoreCalls::default();
    let recorded = calls.clone();
    let known = BTreeMap::from([
        (
            hex_of(&recipient),
            Known::Human("accepted-folder@example.org"),
        ),
        (hex_of(&pending), Known::Human("pending-folder@example.org")),
        (hex_of(&wrong), Known::Human("admin-added@example.org")),
    ]);
    let state = test_state().with_core_description_fixture(move |request: &CoreLookupRequest| {
        recorded.lock().unwrap().push(request.keys.clone());
        Ok(core_answer(request, &known))
    });
    let router = router_with_state(state.clone());
    assert_eq!(
        post_brain(
            router.clone(),
            &admin,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    add_test_org_folders(&router, &admin).await;
    // These keys already appear in the report. Neither admin-added roles nor
    // pending invitations may expose their Core descriptions.
    for keys in [&pending, &wrong] {
        state
            .store
            .lock()
            .unwrap()
            .add_member(&BrainId::new("acme").unwrap(), &user(keys))
            .unwrap();
    }
    let mut invitations = Vec::new();
    for (index, keys) in [&recipient, &pending].into_iter().enumerate() {
        let target = npub(keys);
        let change_id = format!("folder-accept-evidence-{index}");
        let created = authed_request(router.clone(), &admin, "POST", "/v1/brains/acme/folders/restricted/invitations", Some(serde_json::json!({
            "recipientNpub": target,
            "grant": folder_key_grant_value(&change_id, 1, &target),
            "accessChangeEvent": admin_event(&admin, "acme", &change_id, AdminAccessAction::GrantFolderAccess, Some("restricted"), Some(&target), Some(1)),
            "expiresAt": "2026-06-04T20:26:40Z",
        }).to_string()), TEST_NOW + index as u64 + 1).await;
        assert_eq!(created.status(), StatusCode::OK);
        invitations.push(read_json::<FolderInvitationResponse>(created).await);
    }
    let (status, before) = get_report(&router, &admin, "", TEST_NOW + 3).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    for keys in [&pending, &wrong] {
        assert!(row(&before, keys).get("participation").is_none());
        assert_eq!(row(&before, keys)["description"]["state"], "notShared");
    }
    assert!(
        !before["identities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["npub"] == npub(&recipient))
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .any(|key| key == &hex_of(&recipient))
    );
    let accept_path = format!("/v1/invitations/{}/accept", invitations[0].id);
    let denied = authed_request(
        router.clone(),
        &wrong,
        "POST",
        &accept_path,
        None,
        TEST_NOW + 4,
    )
    .await;
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);
    let accepted = authed_request(
        router.clone(),
        &recipient,
        "POST",
        &accept_path,
        None,
        TEST_NOW + 5,
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted: FolderInvitationResponse = read_json(accepted).await;
    calls.lock().unwrap().clear();
    let before_report = state
        .store
        .lock()
        .unwrap()
        .latest_sequence(&BrainId::new("acme").unwrap())
        .unwrap();
    let (status, after) = get_report(&router, &admin, "", TEST_NOW + 6).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    let named = row(&after, &recipient);
    assert_eq!(named["brainRole"], "guest");
    assert_eq!(named["participation"]["kind"], "folderInvitationAcceptance");
    assert_eq!(
        named["participation"]["recordedAt"],
        serde_json::json!(accepted.accepted_at)
    );
    assert_eq!(named["description"]["state"], "resolved");
    assert_eq!(
        named["description"]["accountEmail"],
        "accepted-folder@example.org"
    );
    assert_eq!(
        folder(named, "restricted").unwrap()["currentGrant"],
        "present"
    );
    let requested = calls
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(requested.contains(&hex_of(&recipient)));
    for keys in [&pending, &wrong] {
        assert!(!requested.contains(&hex_of(keys)));
        assert!(row(&after, keys).get("participation").is_none());
        assert_eq!(row(&after, keys)["description"]["state"], "notShared");
    }
    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .latest_sequence(&BrainId::new("acme").unwrap())
            .unwrap(),
        before_report
    );
}

#[tokio::test]
async fn accepted_personal_mount_describes_only_the_accepting_controller_in_source_brain() {
    let admin = Keys::generate();
    let owner = Keys::generate();
    let agent = Keys::generate();
    let calls = CoreCalls::default();
    let recorded = calls.clone();
    let known = BTreeMap::from([
        (hex_of(&owner), Known::Human("mount-owner@example.org")),
        (
            hex_of(&agent),
            Known::Agent("Automatic Agent", "mount-owner@example.org"),
        ),
    ]);
    let state = personal_test_state(&owner, &agent).with_core_description_fixture(
        move |request: &CoreLookupRequest| {
            recorded.lock().unwrap().push(request.keys.clone());
            Ok(core_answer(request, &known))
        },
    );
    let router = router_with_state(state.clone());
    assert_eq!(
        post_brain(
            router.clone(),
            &admin,
            &create_brain_body("acme", "organization"),
            TEST_NOW,
            None,
            None,
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    add_test_org_folders(&router, &admin).await;
    // Make both keys visible before acceptance without letting either act.
    for keys in [&owner, &agent] {
        state
            .store
            .lock()
            .unwrap()
            .add_member(&BrainId::new("acme").unwrap(), &user(keys))
            .unwrap();
    }
    let target = npub(&owner);
    let created = authed_request(router.clone(), &admin, "POST", "/v1/brains/acme/folders/restricted/mount-offers", Some(serde_json::json!({
        "destinationBrainId": "personal", "destinationControllerNpub": target,
        "grant": folder_key_grant_value("mount-evidence-owner", 1, &target),
        "accessChangeEvent": admin_event(&admin, "acme", "mount-evidence-offer", AdminAccessAction::GrantFolderAccess, Some("restricted"), Some(&target), Some(1)),
        "expiresAt": "2026-06-04T20:26:40Z",
    }).to_string()), TEST_NOW + 1).await;
    assert_eq!(created.status(), StatusCode::OK);
    let offer: MountOfferResponse = read_json(created).await;
    let (status, before) = get_report(&router, &admin, "", TEST_NOW + 2).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    for keys in [&owner, &agent] {
        assert!(row(&before, keys).get("participation").is_none());
        assert_eq!(row(&before, keys)["description"]["state"], "notShared");
    }
    let accept_path = format!("/v1/mount-offers/{}/accept", offer.id);
    // The Agent also controls the Personal Brain but is not the addressed
    // controller. Controller capability alone does not prove acceptance.
    let wrong = authed_request(
        router.clone(),
        &agent,
        "POST",
        &accept_path,
        Some(String::new()),
        TEST_NOW + 3,
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::NOT_FOUND);
    let accepted = authed_request(
        router.clone(),
        &owner,
        "POST",
        &accept_path,
        Some(
            serde_json::json!({
                "grants": [folder_key_grant_value("mount-evidence-agent", 1, &npub(&agent))],
            })
            .to_string(),
        ),
        TEST_NOW + 4,
    )
    .await;
    let status = accepted.status();
    let text = read_text(accepted).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    calls.lock().unwrap().clear();
    let (status, after) = get_report(&router, &admin, "", TEST_NOW + 5).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(
        row(&after, &owner)["participation"]["kind"],
        "mountOfferAcceptance"
    );
    assert_eq!(row(&after, &owner)["description"]["state"], "resolved");
    assert_eq!(
        row(&after, &owner)["description"]["accountEmail"],
        "mount-owner@example.org"
    );
    assert!(row(&after, &agent).get("participation").is_none());
    assert_eq!(row(&after, &agent)["description"]["state"], "notShared");
    assert_eq!(
        folder(row(&after, &agent), "restricted").unwrap()["currentGrant"],
        "present"
    );
    let requested = calls
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(requested.contains(&hex_of(&owner)));
    assert!(!requested.contains(&hex_of(&agent)));
    let (status, destination) = get_report_for(&router, &owner, "personal", "", TEST_NOW + 6).await;
    assert_eq!(status, StatusCode::OK, "{destination}");
    assert!(row(&destination, &owner).get("participation").is_none());
}

#[tokio::test]
async fn applied_approval_signer_becomes_eligible_and_its_target_does_not() {
    let cast = Cast::new();
    let (base, _, _scratch) = synthetic_brain(&cast).await;
    let calls = CoreCalls::default();
    let recorded = calls.clone();
    let known = BTreeMap::from([
        (
            hex_of(&cast.arbitrary),
            Known::Human("approver@acme.example"),
        ),
        (
            hex_of(&cast.replacement),
            Known::Human("target@acme.example"),
        ),
    ]);
    let state = base.with_core_description_fixture(move |request: &CoreLookupRequest| {
        recorded.lock().unwrap().push(request.keys.clone());
        Ok(core_answer(request, &known))
    });
    let router = router_with_state(state.clone());
    let requested = |calls: &CoreCalls| {
        calls
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>()
    };

    // Admin-added, never acted: withheld and never sent to Core.
    let (status, before) = get_report(&router, &cast.admin, "", TEST_NOW + 50).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!(
        row(&before, &cast.arbitrary)["description"]["reason"],
        "noParticipation"
    );
    assert!(!requested(&calls).contains(&hex_of(&cast.arbitrary)));

    // The existing approval writer records its exact signer; its target
    // (here a member granted admin) is not a signer.
    {
        let acme = BrainId::new("acme").unwrap();
        let mut store = state.store.lock().unwrap();
        store
            .grant_admin_with_provenance(
                &acme,
                &user(&cast.replacement),
                &finite_brain_store::MemberProvenance::approval(
                    user(&cast.arbitrary),
                    "approval-event".to_owned(),
                ),
            )
            .unwrap();
        store
            .record_brain_approval_nonce(
                &acme,
                "approval-nonce",
                "approval-event",
                &user(&cast.arbitrary),
                finite_brain_core::BRAIN_APPROVAL_ACTION_DELEGATION_GRANT,
                &test_rfc3339(),
            )
            .unwrap();
    }
    calls.lock().unwrap().clear();
    let (status, after) = get_report(&router, &cast.admin, "", TEST_NOW + 51).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    let approver = row(&after, &cast.arbitrary);
    assert_eq!(
        approver["participation"]["kind"],
        "authenticatedBrainAction"
    );
    assert_eq!(
        approver["description"]["accountEmail"],
        "approver@acme.example"
    );
    let target = row(&after, &cast.replacement);
    assert_eq!(target["brainRole"], "admin");
    assert!(target.get("participation").is_none());
    assert_eq!(target["description"]["state"], "notShared");
    let sent = requested(&calls);
    assert!(sent.contains(&hex_of(&cast.arbitrary)));
    assert!(!sent.contains(&hex_of(&cast.replacement)));
    assert!(!after.to_string().contains("target@acme.example"));
}
