//! Run only against the pinned fsite/v0.5.3 source; see fixture README.
use std::{collections::BTreeMap, fs, path::PathBuf};

use finitesites_blob::BlobStore;
use finitesites_engine::{Engine, EngineConfig, ViewAccess};
use finitesites_proto::{
    ManifestFile,
    dto::{ProjectInitRequest, SharingRequest},
    hex,
    project_config::{ProjectConfig, ProjectOutputConfig, ProjectOutputKind, ProjectSection},
};
use finitesites_store::Store;
use sha2::{Digest, Sha256};

const OWNER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const NOW: u64 = 1_750_000_000;

#[test]
fn write_legacy_email_fixture() {
    let output =
        PathBuf::from(std::env::var_os("SITES_LEGACY_FIXTURE_OUTPUT").expect("output directory"));
    fs::create_dir(&output).expect("use a new empty output directory");
    let store = Store::open(&output.join("registry.db")).unwrap();
    let blobs = BlobStore::open(&output.join("blobs")).unwrap();
    // Public synthetic test key, never a production cookie secret.
    let mut engine = Engine::new(
        store,
        blobs,
        [42; 32],
        EngineConfig {
            base_domain: "finite.chat".into(),
            document_base_domain: "docs.finite.chat".into(),
            site_url_scheme: "https".into(),
            site_url_port: None,
        },
    );
    engine
        .store_mut()
        .allow_pubkey(OWNER, "synthetic publisher", NOW)
        .unwrap();
    engine
        .store_mut()
        .register_sites_authorized_key("owner@example.com", OWNER, NOW)
        .unwrap();
    let request = ProjectInitRequest {
        config: ProjectConfig {
            project: ProjectSection {
                slug: "legacy-email".into(),
            },
            outputs: BTreeMap::from([(
                "site".into(),
                ProjectOutputConfig {
                    kind: ProjectOutputKind::Site,
                    site_name: Some("legacy-email".into()),
                    document_name: None,
                    branch: "main".into(),
                    path: ".".into(),
                    entry: None,
                    spa: false,
                    start: None,
                },
            )]),
        },
        dry_run: false,
        requesting_user_npub: None,
        owner_email: Some("owner@example.com".into()),
        hosted_requester_assertion: None,
    };
    let result = engine
        .init_project(
            OWNER,
            &request,
            "https://git.finite.chat/legacy-email.git".into(),
            NOW,
        )
        .unwrap();
    let site_id = result.outputs[0].site_id.as_ref().unwrap();
    let bytes = b"<h1>published by fsite/v0.5.3</h1>";
    engine
        .commit_project_output_version(
            site_id,
            vec![(
                ManifestFile {
                    path: "/index.html".into(),
                    sha256: hex::encode(&Sha256::digest(bytes)),
                    size: bytes.len() as u64,
                },
                bytes.to_vec(),
            )],
            false,
            NOW + 1,
        )
        .unwrap();
    engine
        .set_sharing(
            OWNER,
            "legacy-email",
            &SharingRequest {
                visibility: Some("shared".into()),
                confirm_public: false,
                add_emails: vec!["Friend@Example.com".into()],
                remove_emails: vec![],
                add_npubs: vec![],
                remove_npubs: vec![],
            },
            NOW + 2,
        )
        .unwrap();
    let site = engine.resolve_site("legacy-email").unwrap().unwrap();
    assert!(
        engine
            .email_can_view_site(&site, "owner@example.com")
            .unwrap()
    );
    assert!(
        engine
            .email_can_view_site(&site, "friend@example.com")
            .unwrap()
    );
    assert!(
        !engine
            .email_can_view_site(&site, "stranger@example.com")
            .unwrap()
    );
    let link = engine
        .request_login_for_site(&site, "friend@example.com", NOW + 3)
        .unwrap()
        .unwrap();
    let token = link.url.split("token=").nth(1).unwrap();
    let (_, cookie) = engine.redeem_login(token, NOW + 4).unwrap();
    assert_eq!(
        engine.view_access(&site, Some(&cookie), NOW + 5).unwrap(),
        ViewAccess::Allowed
    );
    fs::write(output.join("friend-cookie.txt"), cookie).unwrap();
    fs::write(output.join("site-id.txt"), &site.id).unwrap();
    fs::write(
        output.join("active-version-id.txt"),
        site.active_version_id.as_ref().unwrap(),
    )
    .unwrap();
    // Dropping the old writer closes SQLite and checkpoints its WAL before archiving.
    drop(engine);
}
