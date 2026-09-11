//! Compatibility proof from a registry written by the deployed fsite/v0.5.3.
//! The archived fixture and its generator are documented beside the fixture.
use std::{fs, path::Path};

use finitesites_blob::BlobStore;
use finitesites_engine::{Engine, EngineConfig, ViewAccess};
use finitesites_proto::dto::SharingRequest;
use finitesites_store::{SiteStatus, Store, Visibility};

const OWNER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const NOW: u64 = 1_750_000_010;

fn open_candidate(data: &Path) -> Engine {
    Engine::new(
        Store::open(&data.join("registry.db")).unwrap(),
        BlobStore::open(&data.join("blobs")).unwrap(),
        [42; 32], // Public synthetic key shared with the legacy fixture writer.
        EngineConfig {
            base_domain: "finite.site".into(),
            site_url_scheme: "https".into(),
            site_url_port: None,
        },
    )
}

#[test]
fn legacy_email_grants_survive_new_domain_and_revocation_after_restart() {
    let data = tempfile::tempdir().unwrap();
    let archive = flate2::read::GzDecoder::new(
        &include_bytes!("fixtures/legacy-email-v053/state.tar.gz")[..],
    );
    tar::Archive::new(archive).unpack(data.path()).unwrap();
    let expected_id = fs::read_to_string(data.path().join("site-id.txt")).unwrap();
    let expected_version = fs::read_to_string(data.path().join("active-version-id.txt")).unwrap();
    let old_cookie = fs::read_to_string(data.path().join("friend-cookie.txt")).unwrap();
    let mut engine = open_candidate(data.path());
    let site = engine.resolve_site("legacy-email").unwrap().unwrap();
    assert_eq!(site.id, expected_id);
    assert_eq!(
        site.active_version_id.as_deref(),
        Some(expected_version.as_str())
    );
    assert_eq!(site.visibility, Visibility::Shared);
    assert_eq!(site.status, SiteStatus::Published);
    assert_eq!(
        engine.site_url("legacy-email"),
        "https://legacy-email.finite.site/"
    );
    let file = engine.lookup_file(&site, "/").unwrap().unwrap();
    assert_eq!(
        engine.read_blob(&file.sha256).unwrap(),
        b"<h1>published by fsite/v0.5.3</h1>"
    );
    assert_eq!(
        engine.view_access(&site, None, NOW).unwrap(),
        ViewAccess::NeedsLogin
    );

    // The legacy owner authority is a publisher-email FK, not a new share row.
    for email in ["owner@example.com", "friend@example.com"] {
        assert!(engine.email_can_view_site(&site, email).unwrap());
        let cookie = handoff_cookie(&mut engine, &site, email, NOW);
        assert_eq!(
            engine.view_access(&site, Some(&cookie), NOW).unwrap(),
            ViewAccess::Allowed
        );
    }
    assert!(
        !engine
            .email_can_view_site(&site, "stranger@example.com")
            .unwrap()
    );
    let stranger = handoff_cookie(&mut engine, &site, "stranger@example.com", NOW);
    assert_eq!(
        engine.view_access(&site, Some(&stranger), NOW).unwrap(),
        ViewAccess::NeedsLogin
    );
    // Browser cookies do not cross domains. This additionally proves the old
    // cookie representation remains readable if used on its original domain.
    assert_eq!(
        engine.view_access(&site, Some(&old_cookie), NOW).unwrap(),
        ViewAccess::Allowed
    );
    let new_cookie = handoff_cookie(&mut engine, &site, "friend@example.com", NOW);
    engine
        .set_sharing(
            OWNER,
            "legacy-email",
            &SharingRequest {
                visibility: None,
                confirm_public: false,
                add_emails: vec![],
                remove_emails: vec!["friend@example.com".into()],
                add_npubs: vec![],
                remove_npubs: vec![],
            },
            NOW + 1,
        )
        .unwrap();
    drop(engine);

    let mut engine = open_candidate(data.path());
    let site = engine.resolve_site("legacy-email").unwrap().unwrap();
    assert_eq!(site.id, expected_id);
    assert_eq!(
        site.active_version_id.as_deref(),
        Some(expected_version.as_str())
    );
    assert!(
        !engine
            .email_can_view_site(&site, "friend@example.com")
            .unwrap()
    );
    for cookie in [&old_cookie, &new_cookie] {
        assert_eq!(
            engine.view_access(&site, Some(cookie), NOW + 2).unwrap(),
            ViewAccess::NeedsLogin
        );
    }
    let owner = handoff_cookie(&mut engine, &site, "owner@example.com", NOW + 2);
    assert_eq!(
        engine.view_access(&site, Some(&owner), NOW + 2).unwrap(),
        ViewAccess::Allowed
    );
}

fn handoff_cookie(
    engine: &mut Engine,
    site: &finitesites_store::SiteRecord,
    email: &str,
    now: u64,
) -> String {
    let link = engine.request_viewer_handoff(site, email, now).unwrap();
    let token = link.url.split("session_token=").nth(1).unwrap();
    // The bridge token cannot be replayed as a reusable email link.
    assert!(engine.redeem_login(token, now).is_err());
    let (_, cookie, _) = engine.redeem_viewer_handoff(site, token, now).unwrap();
    assert!(engine.redeem_viewer_handoff(site, token, now).is_err());
    cookie
}

#[test]
fn account_handoff_expiry_and_consumption_survive_reopening() {
    let data = tempfile::tempdir().unwrap();
    tar::Archive::new(flate2::read::GzDecoder::new(
        &include_bytes!("fixtures/legacy-email-v053/state.tar.gz")[..],
    ))
    .unpack(data.path())
    .unwrap();
    let mut engine = open_candidate(data.path());
    let site = engine.resolve_site("legacy-email").unwrap().unwrap();
    let expired = engine
        .request_viewer_handoff(&site, "friend@example.com", NOW)
        .unwrap();
    let valid = engine
        .request_viewer_handoff(&site, "friend@example.com", NOW + 2)
        .unwrap();
    drop(engine);
    let mut engine = open_candidate(data.path());
    let token = |link: &str| link.split("session_token=").nth(1).unwrap().to_string();
    assert!(
        engine
            .redeem_viewer_handoff(&site, &token(&expired.url), NOW + 61)
            .is_err()
    );
    assert!(
        engine
            .redeem_viewer_handoff(&site, &token(&valid.url), NOW + 61)
            .is_ok()
    );
    drop(engine);
    let mut engine = open_candidate(data.path());
    assert!(
        engine
            .redeem_viewer_handoff(&site, &token(&valid.url), NOW + 61)
            .is_err()
    );
}

#[test]
fn concurrent_handoff_redemption_has_exactly_one_winner() {
    let data = tempfile::tempdir().unwrap();
    tar::Archive::new(flate2::read::GzDecoder::new(
        &include_bytes!("fixtures/legacy-email-v053/state.tar.gz")[..],
    ))
    .unpack(data.path())
    .unwrap();
    let mut engine = open_candidate(data.path());
    let site = engine.resolve_site("legacy-email").unwrap().unwrap();
    let link = engine
        .request_viewer_handoff(&site, "friend@example.com", NOW)
        .unwrap();
    let token = link.url.split("session_token=").nth(1).unwrap().to_string();
    drop(engine);
    let engines = [open_candidate(data.path()), open_candidate(data.path())];
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads = engines.map(|mut engine| {
        let barrier = barrier.clone();
        let site = site.clone();
        let token = token.clone();
        std::thread::spawn(move || {
            barrier.wait();
            engine.redeem_viewer_handoff(&site, &token, NOW).is_ok()
        })
    });
    assert_eq!(
        threads
            .into_iter()
            .filter_map(|thread| thread.join().unwrap().then_some(()))
            .count(),
        1
    );
}
