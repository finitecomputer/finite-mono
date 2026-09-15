use std::fs;
use std::path::Path;
use std::process::Command;

use finitesites_blob::BlobStore;
use finitesites_proto::{ManifestFile, hex, limits::MAX_FILE_BYTES};
use finitesites_store::{Store, Visibility};
use finitesitesd::backup;
use sha2::{Digest, Sha256};

const OWNER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const HTML: &[u8] = b"<h1>Recovered private site</h1>";

fn fixture(data: &Path) -> Store {
    fs::create_dir(data).unwrap();
    let mut store = Store::open(&data.join("registry.db")).unwrap();
    store
        .create_site_with_claim("site_1", "claim_1", "hello", OWNER, 100)
        .unwrap();
    store
        .add_share("site_1", "viewer@example.com", 100)
        .unwrap();
    let sha = hex::encode(&Sha256::digest(HTML));
    BlobStore::open(&data.join("blobs"))
        .unwrap()
        .put(&sha, HTML, MAX_FILE_BYTES)
        .unwrap();
    let files = vec![ManifestFile {
        path: "/index.html".into(),
        sha256: sha.clone(),
        size: HTML.len() as u64,
    }];
    store
        .create_publish("publish_1", "site_1", &files, false, None, 100)
        .unwrap();
    store.record_blob(&sha, HTML.len() as u64, 100).unwrap();
    store
        .finalize_publish("publish_1", "version_1", &"a".repeat(64), 100)
        .unwrap();
    fs::write(data.join("cookie-secret"), hex::encode(&[9; 32])).unwrap();
    fs::create_dir(data.join("blue-green-staging")).unwrap();
    fs::write(
        data.join("blue-green-staging/legacy.tar.gz"),
        b"not active state",
    )
    .unwrap();
    store
}

#[test]
fn capture_restores_private_site_and_shared_state_without_staging_archives() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let _writer = fixture(&data);
    let repository = root.path().join("backup");
    let receipt = backup::capture(&data, &repository, 200).unwrap();
    let target = root.path().join("restored");
    backup::restore(&repository, &receipt.id, &target).unwrap();
    let restored = Store::open(&target.join("registry.db")).unwrap();
    let site = restored.site_by_name("hello").unwrap().unwrap();
    assert_eq!(site.active_version_id.as_deref(), Some("version_1"));
    assert_eq!(site.visibility, Visibility::Private);
    assert_eq!(restored.shares(&site.id).unwrap(), ["viewer@example.com"]);
    let (sha, _) = restored
        .version_file("version_1", "/index.html")
        .unwrap()
        .unwrap();
    assert_eq!(
        BlobStore::open(&target.join("blobs"))
            .unwrap()
            .get(&sha)
            .unwrap(),
        HTML
    );
    assert_eq!(
        fs::read(target.join("cookie-secret")).unwrap(),
        hex::encode(&[9; 32]).as_bytes()
    );
    assert!(!target.join("blue-green-staging").exists());
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Backup test")
        .env("GIT_AUTHOR_EMAIL", "backup@example.com")
        .env("GIT_COMMITTER_NAME", "Backup test")
        .env("GIT_COMMITTER_EMAIL", "backup@example.com")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn source_only_project_restores_git_history_and_non_deploy_branches() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let mut writer = fixture(&data);
    let project = writer
        .init_project(OWNER, "source-only", &[], 100)
        .unwrap()
        .project;
    let work = root.path().join("work");
    fs::create_dir(&work).unwrap();
    git(&work, &["init", "--initial-branch=main"]);
    fs::write(work.join("README.md"), "Original editable source").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-m", "initial"]);
    let initial = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["switch", "-c", "draft"]);
    fs::write(work.join("draft.txt"), "Unpublished work").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-m", "draft"]);
    let draft = git(&work, &["rev-parse", "HEAD"]);
    let repos = data.join("git/projects");
    fs::create_dir_all(&repos).unwrap();
    let repo = repos.join(format!("{}.git", project.id));
    git(&work, &["clone", "--bare", ".", repo.to_str().unwrap()]);
    git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    writer
        .create_git_credential(
            "cred",
            &project.id,
            &project.owner_principal_id,
            &"b".repeat(64),
            None,
            100,
        )
        .unwrap();
    for (name, sha) in [("refs/heads/main", &initial), ("refs/heads/draft", &draft)] {
        let (event, _) = writer
            .record_git_ref_event(
                &project.id,
                name,
                &"0".repeat(40),
                sha,
                &project.owner_principal_id,
                None,
                "cred",
                101,
            )
            .unwrap();
        writer.mark_git_ref_event_ignored(event.id, 101).unwrap();
    }
    let backup_dir = root.path().join("backup");
    let point = backup::capture(&data, &backup_dir, 200).unwrap();
    let target = root.path().join("restored");
    backup::restore(&backup_dir, &point.id, &target).unwrap();
    let restored_repo = target
        .join("git/projects")
        .join(format!("{}.git", project.id));
    git(&restored_repo, &["fsck", "--full"]);
    assert_eq!(git(&restored_repo, &["rev-parse", "main"]), initial);
    assert_eq!(git(&restored_repo, &["rev-parse", "draft"]), draft);
    assert_eq!(
        git(&restored_repo, &["show", "draft:draft.txt"]),
        "Unpublished work"
    );
    let recaptured = backup::capture(&target, &backup_dir, 201).unwrap();
    let restored_again = root.path().join("restored-again");
    backup::restore(&backup_dir, &recaptured.id, &restored_again).unwrap();
    let cloned = root.path().join("cloned");
    git(
        root.path(),
        &[
            "clone",
            restored_repo.to_str().unwrap(),
            cloned.to_str().unwrap(),
        ],
    );
    fs::write(cloned.join("next.txt"), "Publishing can continue").unwrap();
    git(&cloned, &["add", "."]);
    git(&cloned, &["commit", "-m", "after restore"]);
    git(&cloned, &["push", "origin", "main"]);
}

#[test]
fn missing_project_repository_is_not_certified_as_empty() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let mut writer = fixture(&data);
    let project = writer
        .init_project(OWNER, "not-initialized", &[], 100)
        .unwrap()
        .project;
    let repository = root.path().join("backup");
    assert!(backup::capture(&data, &repository, 200).is_err());
    assert_eq!(fs::read_dir(repository.join("points")).unwrap().count(), 0);
    let repo = data
        .join("git/projects")
        .join(format!("{}.git", project.id));
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--bare", "--initial-branch=main"]);
    let point = backup::capture(&data, &repository, 201).unwrap();
    backup::restore(&repository, &point.id, &root.path().join("restored")).unwrap();
}

#[test]
fn unchanged_content_is_reused_while_metadata_only_changes_restore_independently() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let mut writer = fixture(&data);
    let repository = root.path().join("backup");
    let first = backup::capture(&data, &repository, 200).unwrap();
    let replay = backup::capture(&data, &repository, 200).unwrap();
    assert_eq!(first.id, replay.id);
    assert_eq!(replay.new_objects, 0);
    writer.remove_share("site_1", "viewer@example.com").unwrap();
    let second = backup::capture(&data, &repository, 201).unwrap();
    assert_eq!(
        second.new_objects, 1,
        "only the changed registry needs another object"
    );
    let old = root.path().join("old");
    let new = root.path().join("new");
    backup::restore(&repository, &first.id, &old).unwrap();
    backup::restore(&repository, &second.id, &new).unwrap();
    assert_eq!(
        Store::open(&old.join("registry.db"))
            .unwrap()
            .shares("site_1")
            .unwrap(),
        ["viewer@example.com"]
    );
    assert!(
        Store::open(&new.join("registry.db"))
            .unwrap()
            .shares("site_1")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn missing_object_does_not_create_a_restore_target_or_replace_existing_data() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let _writer = fixture(&data);
    let repository = root.path().join("backup");
    let receipt = backup::capture(&data, &repository, 200).unwrap();
    fs::remove_file(
        repository
            .join("objects")
            .join(hex::encode(&Sha256::digest(HTML))),
    )
    .unwrap();
    let target = root.path().join("restored");
    assert!(backup::restore(&repository, &receipt.id, &target).is_err());
    assert!(!target.exists());
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep"), b"existing data").unwrap();
    assert!(backup::restore(&repository, &receipt.id, &target).is_err());
    assert_eq!(fs::read(target.join("keep")).unwrap(), b"existing data");
}

#[test]
fn manifest_cannot_omit_a_blob_required_by_its_registry() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let _writer = fixture(&data);
    let repository = root.path().join("backup");
    let receipt = backup::capture(&data, &repository, 200).unwrap();
    // Simulate a structurally valid but incomplete object-store response.
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(repository.join("points").join(&receipt.id)).unwrap())
            .unwrap();
    manifest["entries"]
        .as_array_mut()
        .unwrap()
        .retain(|entry| !entry["path"].as_str().unwrap().starts_with("blobs/"));
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let id = hex::encode(&Sha256::digest(&bytes));
    fs::write(repository.join("points").join(&id), bytes).unwrap();
    let target = root.path().join("incomplete");
    assert!(backup::restore(&repository, &id, &target).is_err());
    assert!(!target.exists());
}

#[test]
fn force_push_does_not_drop_git_objects_referenced_by_recorded_history() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let mut writer = fixture(&data);
    let project = writer
        .init_project(OWNER, "history", &[], 100)
        .unwrap()
        .project;
    writer
        .create_git_credential(
            "cred",
            &project.id,
            &project.owner_principal_id,
            &"b".repeat(64),
            None,
            100,
        )
        .unwrap();
    let repo = data
        .join("git/projects")
        .join(format!("{}.git", project.id));
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--bare", "--initial-branch=main"]);
    let tree = git(&repo, &["mktree"]);
    let original = git(&repo, &["commit-tree", &tree, "-m", "original source"]);
    let replacement = git(&repo, &["commit-tree", &tree, "-m", "replacement source"]);
    git(&repo, &["update-ref", "refs/heads/main", &replacement]);
    let (event, _) = writer
        .record_git_ref_event(
            &project.id,
            "refs/heads/main",
            &original,
            &replacement,
            &project.owner_principal_id,
            None,
            "cred",
            101,
        )
        .unwrap();
    let repository = root.path().join("backup");
    assert!(
        backup::capture(&data, &repository, 199).is_err(),
        "in-flight publication must not become a completed checkpoint"
    );
    writer.mark_git_ref_event_ignored(event.id, 101).unwrap();
    let unrecorded = git(
        &repo,
        &["commit-tree", &tree, "-m", "unrecorded transition"],
    );
    git(&repo, &["update-ref", "refs/heads/main", &unrecorded]);
    assert!(
        backup::capture(&data, &repository, 200).is_err(),
        "stable refs without a matching recorded transition are not a checkpoint"
    );
    git(&repo, &["update-ref", "refs/heads/main", &replacement]);
    let point = backup::capture(&data, &repository, 200).unwrap();
    let target = root.path().join("restored");
    backup::restore(&repository, &point.id, &target).unwrap();
    let restored_repo = target
        .join("git/projects")
        .join(format!("{}.git", project.id));
    assert_eq!(git(&restored_repo, &["rev-parse", "main"]), replacement);
    assert_eq!(
        git(&restored_repo, &["cat-file", "-t", &original]),
        "commit"
    );
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(repository.join("points").join(&point.id)).unwrap())
            .unwrap();
    manifest["projects"][0]["refs"] = serde_json::json!({});
    manifest["projects"][0]["retained"] = serde_json::json!([]);
    manifest["projects"][0]["bundle"] = serde_json::Value::Null;
    manifest["entries"]
        .as_array_mut()
        .unwrap()
        .retain(|e| !e["path"].as_str().unwrap().starts_with("git-bundles/"));
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let id = hex::encode(&Sha256::digest(&bytes));
    fs::write(repository.join("points").join(&id), bytes).unwrap();
    assert!(backup::restore(&repository, &id, &root.path().join("empty-git")).is_err());
    let (reversed, _) = writer
        .record_git_ref_event(
            &project.id,
            "refs/heads/main",
            &replacement,
            &original,
            &project.owner_principal_id,
            None,
            "cred",
            202,
        )
        .unwrap();
    writer.mark_git_ref_event_ignored(reversed.id, 202).unwrap();
    git(&repo, &["update-ref", "refs/heads/main", &original]);
    assert!(
        backup::capture(&data, &repository, 203).is_err(),
        "a cyclic deduplicated event history has no authoritative terminal state"
    );
    backup::restore(
        &repository,
        &point.id,
        &root.path().join("old-point-after-cycle"),
    )
    .unwrap();
}

#[test]
fn backup_repository_inside_source_is_rejected_without_creating_it() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let _writer = fixture(&data);
    let repository = data.join("recursive-backup");
    assert!(backup::capture(&data, &repository, 200).is_err());
    assert!(!repository.exists());
}

#[test]
fn oversized_manifest_inventory_is_rejected_before_loading_objects() {
    let root = tempfile::tempdir().unwrap();
    let repository = root.path().join("backup");
    fs::create_dir_all(repository.join("points")).unwrap();
    let project = serde_json::json!({
        "project_id": "p", "head": "refs/heads/main", "refs": {},
        "retained": [], "bundle": null
    });
    let bytes = serde_json::to_vec(&serde_json::json!({
        "format": 1, "captured_at": 200, "entries": [],
        "projects": vec![project; finitesites_proto::limits::MAX_BACKUP_OBJECTS as usize + 1]
    }))
    .unwrap();
    use sha2::{Digest, Sha256};
    let id = finitesites_proto::hex::encode(&Sha256::digest(&bytes));
    fs::write(repository.join("points").join(&id), bytes).unwrap();
    let target = root.path().join("restored");
    let error = backup::restore(&repository, &id, &target).unwrap_err();
    assert!(error.to_string().contains("oversized manifest"), "{error}");
    assert!(!target.exists());
}

#[test]
fn corrupted_source_does_not_publish_a_new_point_and_previous_point_still_restores() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let _writer = fixture(&data);
    let repository = root.path().join("backup");
    let point = backup::capture(&data, &repository, 200).unwrap();
    let hash = hex::encode(&Sha256::digest(HTML));
    fs::write(
        data.join(format!("blobs/{}/{}/{}", &hash[..2], &hash[2..4], hash)),
        b"corrupt",
    )
    .unwrap();
    assert!(backup::capture(&data, &repository, 201).is_err());
    let target = root.path().join("restored");
    backup::restore(&repository, &point.id, &target).unwrap();
    assert_eq!(
        BlobStore::open(&target.join("blobs"))
            .unwrap()
            .get(&hash)
            .unwrap(),
        HTML
    );
}

#[test]
fn operator_commands_capture_and_restore_without_overwriting_a_destination() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let _writer = fixture(&data);
    let repository = root.path().join("backup");
    let output = Command::new(env!("CARGO_BIN_EXE_finitesitesd"))
        .args([
            "backup",
            "capture",
            "--data",
            data.to_str().unwrap(),
            "--repository",
            repository.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: backup::Receipt = serde_json::from_slice(&output.stdout).unwrap();
    let target = root.path().join("restored");
    let args = [
        "backup",
        "restore",
        "--repository",
        repository.to_str().unwrap(),
        "--point",
        &receipt.id,
        "--target",
        target.to_str().unwrap(),
    ];
    assert!(
        Command::new(env!("CARGO_BIN_EXE_finitesitesd"))
            .args(args)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_finitesitesd"))
            .args(args)
            .status()
            .unwrap()
            .success()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_viewer_session_serves_restored_content_and_revocation_still_applies() {
    use finitesites_engine::{Engine, EngineConfig};
    use finitesitesd::{ServeOptions, mailer::DevMailer, server};
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("source");
    let writer = fixture(&data);
    let config = EngineConfig {
        base_domain: "sites.localhost".into(),
        site_url_scheme: "http".into(),
        site_url_port: None,
    };
    let mut engine = Engine::new(
        writer,
        BlobStore::open(&data.join("blobs")).unwrap(),
        [9; 32],
        config.clone(),
    );
    let now = server::now_unix();
    let link = engine
        .request_login("hello", "viewer@example.com", now)
        .unwrap()
        .unwrap();
    let token = link.url.split_once("?token=").unwrap().1;
    let (_, cookie) = engine.redeem_login(token, now).unwrap();
    let repository = root.path().join("backup");
    let point = backup::capture(&data, &repository, now).unwrap();
    let target = root.path().join("restored");
    backup::restore(&repository, &point.id, &target).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let key = hex::decode32(
        fs::read_to_string(target.join("cookie-secret"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    let engine = Engine::new(
        Store::open(&target.join("registry.db")).unwrap(),
        BlobStore::open(&target.join("blobs")).unwrap(),
        key,
        config,
    );
    let options = ServeOptions {
        data_dir: target.clone(),
        listen: address,
        base_domain: "sites.localhost".into(),
        api_url: format!("http://{address}"),
        git_base_url: format!("http://{address}"),
        viewer_session_service_token: None,
        account_login_url: None,
        git_hook_helper_path: env!("CARGO_BIN_EXE_finitesitesd").into(),
        git_auto_reconcile: true,
        site_url_scheme: "http".into(),
        site_url_port: Some(address.port()),
        mail_provider: None,
        mail_from: None,
    };
    let mailer = DevMailer::new(target.join("outbox")).unwrap();
    let serving = tokio::spawn(server::serve_on(
        listener,
        engine,
        Box::new(mailer),
        options,
    ));
    tokio::task::spawn_blocking(move || {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(5))
            .redirects(0)
            .build();
        let url = format!("http://{address}/");
        let get = || agent.get(&url).set("Host", "hello.sites.localhost");
        assert!(matches!(get().call(), Err(ureq::Error::Status(401, _))));
        let auth = format!("finite_site_auth={cookie}");
        let body = get()
            .set("Cookie", &auth)
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert_eq!(body.as_bytes(), HTML);
        let mut operator = Store::open(&target.join("registry.db")).unwrap();
        operator
            .remove_share("site_1", "viewer@example.com")
            .unwrap();
        assert!(matches!(
            get().set("Cookie", &auth).call(),
            Err(ureq::Error::Status(401, _))
        ));
    })
    .await
    .unwrap();
    serving.abort();
    let _ = serving.await;
}
