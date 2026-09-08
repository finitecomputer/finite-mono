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
        let cookie = engine.mint_email_viewer_cookie(&site, email, NOW).unwrap();
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
    let stranger = engine
        .mint_email_viewer_cookie(&site, "stranger@example.com", NOW)
        .unwrap();
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
    let new_cookie = engine
        .mint_email_viewer_cookie(&site, "friend@example.com", NOW)
        .unwrap();
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

    let engine = open_candidate(data.path());
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
    let owner = engine
        .mint_email_viewer_cookie(&site, "owner@example.com", NOW + 2)
        .unwrap();
    assert_eq!(
        engine.view_access(&site, Some(&owner), NOW + 2).unwrap(),
        ViewAccess::Allowed
    );
}
