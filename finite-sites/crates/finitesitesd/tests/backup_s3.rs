//! Synthetic S3 wire contract, exercised only through operator capture/restore.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use finitesites_blob::BlobStore;
use finitesites_proto::{ManifestFile, hex, limits::MAX_FILE_BYTES};
use finitesites_store::{Store, Visibility};
use sha2::{Digest, Sha256};

const HTML: &[u8] = b"<h1>Synthetic S3 recovery</h1>";

#[derive(Default)]
struct Bucket {
    requests: u64,
    objects: BTreeMap<String, Vec<Vec<u8>>>,
    lifecycle: Option<String>,
    encryption: Option<&'static str>,
    fail_put: Option<String>,
    conflict_winner: bool,
    suspended: bool,
    response_version: Option<&'static str>,
    corrupt_upload: bool,
    lose_completion_response: bool,
}

async fn s3(
    State(bucket): State<Arc<Mutex<Bucket>>>,
    method: Method,
    uri: Uri,
    Query(query): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let mut bucket = bucket.lock().unwrap();
    bucket.requests += 1;
    if query.contains_key("versioning") {
        if bucket.suspended {
            return "<VersioningConfiguration><Status>Suspended</Status></VersioningConfiguration>"
                .into_response();
        }
        return "<VersioningConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Status>Enabled</Status></VersioningConfiguration>".into_response();
    }
    if query.contains_key("lifecycle") {
        return match &bucket.lifecycle {
            Some(xml) => xml.clone().into_response(),
            None => (
                StatusCode::NOT_FOUND,
                "<Error><Code>NoSuchLifecycleConfiguration</Code></Error>",
            )
                .into_response(),
        };
    }
    let key = uri.path().to_string();
    match method {
        Method::GET => {
            let Some(versions) = bucket.objects.get(&key) else {
                return (
                    StatusCode::NOT_FOUND,
                    "<Error><Code>NoSuchKey</Code></Error>",
                )
                    .into_response();
            };
            let version = query
                .get("versionId")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(versions.len());
            let Some(bytes) = version.checked_sub(1).and_then(|i| versions.get(i)) else {
                return (
                    StatusCode::NOT_FOUND,
                    "<Error><Code>NoSuchVersion</Code></Error>",
                )
                    .into_response();
            };
            let version = bucket
                .response_version
                .map(str::to_string)
                .unwrap_or_else(|| version.to_string());
            encrypted_response(
                ([("x-amz-version-id", version)], bytes.clone()).into_response(),
                bucket.encryption,
            )
        }
        Method::PUT => {
            if bucket.fail_put.as_ref() == Some(&key) {
                return (
                    StatusCode::FORBIDDEN,
                    "<Error><Code>AccessDenied</Code></Error>",
                )
                    .into_response();
            }
            if headers.get("if-none-match").and_then(|v| v.to_str().ok()) != Some("*") {
                return StatusCode::BAD_REQUEST.into_response();
            }
            if bucket.objects.contains_key(&key) {
                return (
                    StatusCode::PRECONDITION_FAILED,
                    "<Error><Code>PreconditionFailed</Code></Error>",
                )
                    .into_response();
            }
            let lose_response = bucket.lose_completion_response && key.contains("/points/");
            let bytes = if bucket.corrupt_upload {
                vec![b'x'; bytes.len()]
            } else {
                bytes.to_vec()
            };
            bucket.objects.insert(key, vec![bytes]);
            if lose_response {
                return (
                    StatusCode::FORBIDDEN,
                    "<Error><Code>AccessDenied</Code></Error>",
                )
                    .into_response();
            }
            if bucket.conflict_winner {
                return (
                    StatusCode::PRECONDITION_FAILED,
                    "<Error><Code>PreconditionFailed</Code></Error>",
                )
                    .into_response();
            }
            encrypted_response(
                (
                    [("x-amz-version-id", bucket.response_version.unwrap_or("1"))],
                    "",
                )
                    .into_response(),
                bucket.encryption,
            )
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

fn encrypted_response(mut response: Response, encryption: Option<&'static str>) -> Response {
    if response
        .headers()
        .get("x-amz-version-id")
        .is_some_and(|v| v.is_empty())
    {
        response.headers_mut().remove("x-amz-version-id");
    }
    if let Some(encryption) = encryption {
        response
            .headers_mut()
            .insert("x-amz-server-side-encryption", encryption.parse().unwrap());
    }
    response
}

struct S3Server {
    endpoint: String,
    bucket: Arc<Mutex<Bucket>>,
    task: tokio::task::JoinHandle<()>,
}

impl S3Server {
    async fn capture(&self, home: &Path, repository: &Path, point: &str) -> Output {
        self.command(
            home,
            &[
                "capture-s3",
                "--repository",
                repository.to_str().unwrap(),
                "--point",
                point,
            ],
        )
        .await
    }

    async fn restore(&self, home: &Path, receipt: &serde_json::Value, target: &Path) -> Output {
        self.command(
            home,
            &[
                "restore-s3",
                "--point",
                receipt["id"].as_str().unwrap(),
                "--version-id",
                receipt["version_id"].as_str().unwrap(),
                "--target",
                target.to_str().unwrap(),
            ],
        )
        .await
    }

    async fn start() -> Self {
        let bucket = Arc::new(Mutex::new(Bucket {
            encryption: Some("AES256"),
            ..Bucket::default()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = axum::Router::new().fallback(s3).with_state(bucket.clone());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            endpoint,
            bucket,
            task,
        }
    }

    async fn command(&self, home: &Path, args: &[&str]) -> Output {
        self.command_with_env(home, args, &[]).await
    }

    async fn command_with_env(
        &self,
        home: &Path,
        args: &[&str],
        overrides: &[(&str, &str)],
    ) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_finitesitesd"));
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", home)
            .env("AWS_ACCESS_KEY_ID", "synthetic-access")
            .env("AWS_SECRET_ACCESS_KEY", "synthetic-secret")
            .env("AWS_EC2_METADATA_DISABLED", "true")
            .envs(overrides.iter().copied())
            .args(["backup"])
            .args(args)
            .args([
                "--bucket",
                "synthetic",
                "--prefix",
                "recovery",
                "--region",
                "us-east-1",
                "--endpoint-url",
                &self.endpoint,
            ]);
        tokio::task::spawn_blocking(move || cmd.output().unwrap())
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn explicit_loopback_endpoint_wins_over_inherited_shared_and_service_endpoints() {
    let selected = S3Server::start().await;
    let inherited = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    let shared = ("AWS_ENDPOINT_URL", inherited.endpoint.as_str());
    let service = ("AWS_ENDPOINT_URL_S3", inherited.endpoint.as_str());
    for (name, overrides) in [
        ("shared", vec![shared]),
        ("service", vec![service]),
        ("both", vec![shared, service]),
    ] {
        let receipt = success(
            selected
                .command_with_env(
                    root.path(),
                    &[
                        "capture-s3",
                        "--repository",
                        repository.to_str().unwrap(),
                        "--point",
                        &point.id,
                    ],
                    &overrides,
                )
                .await,
        );
        let target = root.path().join(name);
        success(
            selected
                .command_with_env(
                    root.path(),
                    &[
                        "restore-s3",
                        "--point",
                        receipt["id"].as_str().unwrap(),
                        "--version-id",
                        receipt["version_id"].as_str().unwrap(),
                        "--target",
                        target.to_str().unwrap(),
                    ],
                    &overrides,
                )
                .await,
        );
        assert_eq!(
            Store::open(&target.join("registry.db"))
                .unwrap()
                .shares("site_1")
                .unwrap(),
            ["viewer@example.com"]
        );
    }
    assert!(selected.bucket.lock().unwrap().requests > 0);
    assert_eq!(
        inherited.bucket.lock().unwrap().requests,
        0,
        "inherited endpoints must receive neither reads nor writes"
    );
}

#[tokio::test]
async fn capture_s3_refuses_expiration_that_can_delete_reused_dependencies() {
    let server = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    for expiration in [
        "<Expiration><Days>30</Days></Expiration>",
        "<NoncurrentVersionExpiration><NoncurrentDays>30</NoncurrentDays></NoncurrentVersionExpiration>",
    ] {
        server.bucket.lock().unwrap().lifecycle = Some(format!(
            "<LifecycleConfiguration><Rule><ID>expire</ID><Filter><Prefix></Prefix></Filter><Status>Enabled</Status>{expiration}</Rule></LifecycleConfiguration>"
        ));
        let output = server.capture(root.path(), &repository, &point.id).await;
        assert!(
            !output.status.success(),
            "capture must reject expiration before uploading"
        );
        assert!(server.bucket.lock().unwrap().objects.is_empty());
    }
}

#[tokio::test]
async fn capture_s3_requires_recognized_server_encryption_on_new_and_reused_objects() {
    let server = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    server.bucket.lock().unwrap().encryption = None;
    assert!(
        !server
            .capture(root.path(), &repository, &point.id)
            .await
            .status
            .success()
    );
    assert!(
        server
            .bucket
            .lock()
            .unwrap()
            .objects
            .keys()
            .all(|key| !key.contains("/points/"))
    );
    server.bucket.lock().unwrap().encryption = Some("aws:kms");
    let receipt = success(server.capture(root.path(), &repository, &point.id).await);
    success(
        server
            .restore(root.path(), &receipt, &root.path().join("restored"))
            .await,
    );
    server.bucket.lock().unwrap().encryption = Some("unknown");
    assert!(
        !server
            .capture(root.path(), &repository, &point.id)
            .await
            .status
            .success()
    );
    let target = root.path().join("failed");
    assert!(
        !server
            .restore(root.path(), &receipt, &target)
            .await
            .status
            .success()
    );
    assert!(!target.exists());
}

#[tokio::test]
async fn interrupted_capture_reuses_objects_and_preserves_metadata_only_recovery_points() {
    let server = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    let blob_key = format!(
        "/synthetic/recovery/objects/{}",
        hex::encode(&Sha256::digest(HTML))
    );
    server.bucket.lock().unwrap().fail_put = Some(blob_key.clone());
    assert!(
        !server
            .capture(root.path(), &repository, &point.id)
            .await
            .status
            .success()
    );
    {
        let bucket = server.bucket.lock().unwrap();
        assert!(
            !bucket.objects.is_empty(),
            "interruption left reusable dependencies"
        );
        assert!(
            bucket.objects.keys().all(|key| !key.contains("/points/")),
            "no incomplete point is selectable"
        );
    }
    server.bucket.lock().unwrap().fail_put = None;
    server.bucket.lock().unwrap().conflict_winner = true;
    let first = success(server.capture(root.path(), &repository, &point.id).await);
    let before_replay = server.bucket.lock().unwrap().objects.clone();
    let replay = success(server.capture(root.path(), &repository, &point.id).await);
    assert_eq!(replay, first);
    assert_eq!(server.bucket.lock().unwrap().objects, before_replay);
    Store::open(&source.join("registry.db"))
        .unwrap()
        .remove_share("site_1", "viewer@example.com")
        .unwrap();
    let changed = finitesitesd::backup::capture(&source, &repository, 201).unwrap();
    let second = success(server.capture(root.path(), &repository, &changed.id).await);
    assert_ne!(first["id"], second["id"]);
    assert_eq!(
        server.bucket.lock().unwrap().objects[&blob_key],
        [HTML.to_vec()]
    );
    for (receipt, name, expected) in [
        (&first, "old", vec!["viewer@example.com"]),
        (&second, "new", vec![]),
    ] {
        let target = root.path().join(name);
        success(server.restore(root.path(), receipt, &target).await);
        assert_eq!(
            Store::open(&target.join("registry.db"))
                .unwrap()
                .shares("site_1")
                .unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn restore_pins_every_version_and_fails_closed_for_corrupt_or_missing_dependencies() {
    let server = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    let receipt = success(server.capture(root.path(), &repository, &point.id).await);
    for versions in server.bucket.lock().unwrap().objects.values_mut() {
        versions.push(b"wrong latest object".to_vec());
    }
    success(
        server
            .restore(root.path(), &receipt, &root.path().join("pinned"))
            .await,
    );
    let blob_key = format!(
        "/synthetic/recovery/objects/{}",
        hex::encode(&Sha256::digest(HTML))
    );
    server
        .bucket
        .lock()
        .unwrap()
        .objects
        .get_mut(&blob_key)
        .unwrap()[0] = vec![b'x'; HTML.len()];
    let corrupt = root.path().join("corrupt");
    assert!(
        !server
            .restore(root.path(), &receipt, &corrupt)
            .await
            .status
            .success()
    );
    assert!(!corrupt.exists());
    server.bucket.lock().unwrap().objects.remove(&blob_key);
    let missing = root.path().join("missing");
    assert!(
        !server
            .restore(root.path(), &receipt, &missing)
            .await
            .status
            .success()
    );
    assert!(!missing.exists());
    fs::create_dir(&missing).unwrap();
    fs::write(missing.join("keep"), b"existing state").unwrap();
    assert!(
        !server
            .restore(root.path(), &receipt, &missing)
            .await
            .status
            .success()
    );
    assert_eq!(fs::read(missing.join("keep")).unwrap(), b"existing state");
}

#[tokio::test]
async fn capture_and_restore_require_enabled_versioning_and_exact_nonnull_versions() {
    let server = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    server.bucket.lock().unwrap().suspended = true;
    assert!(
        !server
            .capture(root.path(), &repository, &point.id)
            .await
            .status
            .success()
    );
    assert!(server.bucket.lock().unwrap().objects.is_empty());
    server.bucket.lock().unwrap().suspended = false;
    for version in ["null", ""] {
        server.bucket.lock().unwrap().response_version = Some(version);
        assert!(
            !server
                .capture(root.path(), &repository, &point.id)
                .await
                .status
                .success()
        );
        assert!(
            server
                .bucket
                .lock()
                .unwrap()
                .objects
                .keys()
                .all(|key| !key.contains("/points/"))
        );
    }
    server.bucket.lock().unwrap().response_version = None;
    let receipt = success(server.capture(root.path(), &repository, &point.id).await);
    for (name, version) in [("null", "null"), ("missing", ""), ("wrong", "999")] {
        server.bucket.lock().unwrap().response_version = Some(version);
        let target = root.path().join(name);
        assert!(
            !server
                .restore(root.path(), &receipt, &target)
                .await
                .status
                .success()
        );
        assert!(!target.exists());
    }
}

#[tokio::test]
async fn capture_refuses_corrupt_local_sets_upload_readback_and_reused_objects() {
    let server = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    let blob = hex::encode(&Sha256::digest(HTML));
    fs::write(repository.join("objects").join(&blob), b"damaged").unwrap();
    assert!(
        !server
            .capture(root.path(), &repository, &point.id)
            .await
            .status
            .success()
    );
    assert!(server.bucket.lock().unwrap().objects.is_empty());
    fs::write(repository.join("objects").join(&blob), HTML).unwrap();
    server.bucket.lock().unwrap().corrupt_upload = true;
    assert!(
        !server
            .capture(root.path(), &repository, &point.id)
            .await
            .status
            .success()
    );
    assert!(
        server
            .bucket
            .lock()
            .unwrap()
            .objects
            .keys()
            .all(|key| !key.contains("/points/"))
    );
    server.bucket.lock().unwrap().corrupt_upload = false;
    server.bucket.lock().unwrap().objects.clear();
    let receipt = success(server.capture(root.path(), &repository, &point.id).await);
    server
        .bucket
        .lock()
        .unwrap()
        .objects
        .get_mut(&format!("/synthetic/recovery/objects/{blob}"))
        .unwrap()[0] = vec![b'x'; HTML.len()];
    assert!(
        !server
            .capture(root.path(), &repository, &point.id)
            .await
            .status
            .success()
    );
    let target = root.path().join("invalid");
    assert!(
        !server
            .restore(root.path(), &receipt, &target)
            .await
            .status
            .success()
    );
    assert!(!target.exists());
}

#[tokio::test]
async fn retry_recovers_a_lost_completion_receipt_without_rewriting_the_recovery_set() {
    let server = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    server.bucket.lock().unwrap().lose_completion_response = true;
    let failed = server.capture(root.path(), &repository, &point.id).await;
    assert!(!failed.status.success());
    assert!(
        failed.stdout.is_empty(),
        "a failed command must not emit a success receipt"
    );
    let before_retry = server.bucket.lock().unwrap().objects.clone();
    server.bucket.lock().unwrap().lose_completion_response = false;
    let receipt = success(server.capture(root.path(), &repository, &point.id).await);
    assert_eq!(server.bucket.lock().unwrap().objects, before_retry);
    success(
        server
            .restore(root.path(), &receipt, &root.path().join("restored"))
            .await,
    );
}

#[tokio::test]
async fn s3_restore_preserves_git_history_and_subsequent_local_push() {
    let server = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let mut store = Store::open(&source.join("registry.db")).unwrap();
    let project = store
        .init_project(&"1".repeat(64), "source-only", &[], 100)
        .unwrap()
        .project;
    let work = root.path().join("work");
    fs::create_dir(&work).unwrap();
    git(&work, &["init", "--initial-branch=main"]);
    fs::write(work.join("README.md"), "Synthetic editable source").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-m", "synthetic source"]);
    let head = git(&work, &["rev-parse", "HEAD"]);
    let repo_path = format!("git/projects/{}.git", project.id);
    fs::create_dir_all(source.join("git/projects")).unwrap();
    git(
        &work,
        &[
            "clone",
            "--bare",
            ".",
            source.join(&repo_path).to_str().unwrap(),
        ],
    );
    store
        .create_git_credential(
            "credential",
            &project.id,
            &project.owner_principal_id,
            &"b".repeat(64),
            None,
            100,
        )
        .unwrap();
    let (event, _) = store
        .record_git_ref_event(
            &project.id,
            "refs/heads/main",
            &"0".repeat(40),
            &head,
            &project.owner_principal_id,
            None,
            "credential",
            101,
        )
        .unwrap();
    store.mark_git_ref_event_ignored(event.id, 101).unwrap();
    drop(store);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    let receipt = success(server.capture(root.path(), &repository, &point.id).await);
    let target = root.path().join("restored");
    success(server.restore(root.path(), &receipt, &target).await);
    let restored_repo = target.join(&repo_path);
    assert_eq!(git(&restored_repo, &["rev-parse", "main"]), head);
    let cloned = root.path().join("cloned");
    git(
        root.path(),
        &[
            "clone",
            restored_repo.to_str().unwrap(),
            cloned.to_str().unwrap(),
        ],
    );
    fs::write(cloned.join("next.txt"), "After restore").unwrap();
    git(&cloned, &["add", "."]);
    git(&cloned, &["commit", "-m", "after restore"]);
    git(&cloned, &["push", "origin", "main"]);
    assert_ne!(git(&restored_repo, &["rev-parse", "main"]), head);
    assert_eq!(git(&source.join(repo_path), &["rev-parse", "main"]), head);
}

#[tokio::test]
async fn restore_rejects_a_hashed_completion_manifest_with_a_missing_dependency_version() {
    let server = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    let mut receipt = success(server.capture(root.path(), &repository, &point.id).await);
    {
        let mut bucket = server.bucket.lock().unwrap();
        let key = format!(
            "/synthetic/recovery/points/{}",
            receipt["id"].as_str().unwrap()
        );
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&bucket.objects[&key][0]).unwrap();
        manifest["objects"]
            .as_object_mut()
            .unwrap()
            .remove(&hex::encode(&Sha256::digest(HTML)));
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let hash = hex::encode(&Sha256::digest(&bytes));
        bucket
            .objects
            .insert(format!("/synthetic/recovery/points/{hash}"), vec![bytes]);
        receipt["id"] = hash.into();
    }
    let target = root.path().join("incomplete");
    assert!(
        !server
            .restore(root.path(), &receipt, &target)
            .await
            .status
            .success()
    );
    assert!(!target.exists());
}

fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Recovery test")
        .env("GIT_COMMITTER_NAME", "Recovery test")
        .env("GIT_AUTHOR_EMAIL", "recovery@example.com")
        .env("GIT_COMMITTER_EMAIL", "recovery@example.com")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

impl Drop for S3Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn fixture(data: &Path) {
    fs::create_dir(data).unwrap();
    let mut store = Store::open(&data.join("registry.db")).unwrap();
    store
        .create_site_with_claim("site_1", "claim_1", "hello", &"1".repeat(64), 100)
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
}

fn success(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
async fn capture_s3_restores_private_content_and_shares_on_an_empty_target() {
    let server = S3Server::start().await;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fixture(&source);
    let repository = root.path().join("local");
    let point = finitesitesd::backup::capture(&source, &repository, 200).unwrap();
    let receipt = success(
        server
            .command(
                root.path(),
                &[
                    "capture-s3",
                    "--repository",
                    repository.to_str().unwrap(),
                    "--point",
                    &point.id,
                ],
            )
            .await,
    );
    let target = root.path().join("restored");
    success(
        server
            .command(
                root.path(),
                &[
                    "restore-s3",
                    "--point",
                    receipt["id"].as_str().unwrap(),
                    "--version-id",
                    receipt["version_id"].as_str().unwrap(),
                    "--target",
                    target.to_str().unwrap(),
                ],
            )
            .await,
    );
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
    assert!(!server.bucket.lock().unwrap().objects.is_empty());
}
