//! End-to-end shell tests against temp dirs and stub payloads — no Docker.
//! The stub payload's `bin/finite-agentd` is a small shell script that writes
//! the status/ready files health_server.py (and the flip health gate) reads,
//! then sleeps; the "broken" variant exits 1 immediately.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

use finite_service_directory::{
    ChannelBundleV1, PAYLOAD_BUNDLE_KIND, SERVICE_DIRECTORY_SCHEMA, ServiceDirectoryV1,
};
use finite_shell::control::{ShellRuntime, ctl_roundtrip};
use finite_shell::generations::{StageRequest, read_link_version, stage_payload};
use finite_shell::state::{FlipOutcome, FlipRecord, PollAction, SharedState};
use finite_shell::{SHELL_VERSION, ShellError, ShellSettings, health};

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[21_u8; 32])
}

fn public_key_hex() -> String {
    hex::encode(test_signing_key().verifying_key().to_bytes())
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}

/// A live "agentd is running" supervision snapshot: since the fail-open fix,
/// `ready: true` requires live supervision truth, not just fresh files.
fn running_supervision() -> finite_shell::supervise::AgentdSupervisionStatus {
    finite_shell::supervise::AgentdSupervisionStatus {
        state: "running".to_owned(),
        pid: Some(4242),
        restart_count: 0,
        last_exit: None,
        crash_looping: false,
        updated_at_ms: 0,
    }
}

/// Send one verb through the runtime's dispatch with the boot token attached
/// (the same framing `serve_socket` feeds it).
async fn authed_request(runtime: &ShellRuntime, mut request: Value) -> Value {
    request
        .as_object_mut()
        .expect("request is an object")
        .insert("token".to_owned(), json!(runtime.control_token()));
    runtime.handle_request_line(&request.to_string()).await
}

/// Build a stub payload source tree. The agentd script writes a
/// generation-marker (so tests can prove which generation ran), the agentd
/// status file, and the finitechat ready file, then sleeps.
fn build_payload_source(dir: &Path, marker: &str, broken: bool) -> PathBuf {
    let source = dir.join(format!("source-{marker}"));
    let bin = source.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let body = if broken {
        "exit 1".to_owned()
    } else {
        format!(
            r#"[ -n "$FINITECHAT_HOME" ] || exit 9
[ -n "$FINITE_PAYLOAD_ROOT" ] || exit 8
mkdir -p "$FINITECHAT_HOME/agentd"
printf '%s' "$PATH" > "$FINITECHAT_HOME/agentd/seen-path"
cat > "$FINITECHAT_HOME/agentd/status.json" <<'EOF'
{{"processes":{{"processes":{{"finitechat":{{"state":"running"}},"health":{{"state":"running"}},"hermes":{{"state":"running"}}}}}},"version":"stub-{marker}"}}
EOF
printf '{{}}' > "$FINITECHAT_HOME/agentd/finitechat-ready.json"
printf '%s' '{marker}' > "$FINITECHAT_HOME/agentd/generation-marker"
exec sleep 300"#
        )
    };
    fs::write(bin.join("finite-agentd"), format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(bin.join("finite-agentd"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(bin.join("fbrain"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(bin.join("fbrain"), fs::Permissions::from_mode(0o755)).unwrap();

    // A venv with deliberately wrong shebangs/links for the fixup to repair.
    let venv_bin = source.join("hermes-venv/bin");
    fs::create_dir_all(&venv_bin).unwrap();
    fs::write(
        source.join("hermes-venv/pyvenv.cfg"),
        "home = /opt/old-build/python/bin\nversion = 3.11.9\n",
    )
    .unwrap();
    fs::write(venv_bin.join("python3.11"), "stub interpreter").unwrap();
    std::os::unix::fs::symlink("python3.11", venv_bin.join("python")).unwrap();
    fs::write(
        venv_bin.join("hermes"),
        "#!/opt/old-build/hermes-venv/bin/python\nprint('hi')\n",
    )
    .unwrap();
    fs::set_permissions(venv_bin.join("hermes"), fs::Permissions::from_mode(0o755)).unwrap();
    source
}

fn pack_payload(
    dir: &Path,
    version: &str,
    marker: &str,
    broken: bool,
) -> finite_release::PayloadPackOutput {
    let source = build_payload_source(dir, marker, broken);
    finite_release::pack_payload_bundle(
        &finite_release::PayloadPackRequest {
            source: &source,
            artifact_id: &format!("payload-{version}"),
            version_label: version,
            min_shell_version: "0.1.0",
            source_git_sha: None,
            created_at: Some("2026-08-06T00:00:00Z"),
            out_dir: &dir.join(format!("packed-{version}")),
        },
        &test_signing_key(),
    )
    .unwrap()
}

/// Test settings: everything host-global (shim dir, shell python, agentd env)
/// redirected into the temp dir; timings shrunk.
fn test_settings(root: &Path) -> ShellSettings {
    let data_dir = root.join("data");
    let seed_dir = root.join("seed");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&seed_dir).unwrap();
    let shell_python_dir = root.join("shell-python");
    fs::create_dir_all(&shell_python_dir).unwrap();
    fs::write(shell_python_dir.join("python3"), "shell python").unwrap();
    let mut settings = ShellSettings::new(&data_dir, &seed_dir);
    settings.release_public_key = Some(public_key_hex());
    settings.shim_dir = root.join("shims");
    settings.shell_python = shell_python_dir.join("python3");
    settings.quiesce_timeout = Duration::from_secs(5);
    settings.health_timeout = Duration::from_secs(10);
    settings.health_gate_poll = Duration::from_millis(50);
    settings.restart_backoff_initial = Duration::from_millis(10);
    settings.restart_backoff_max = Duration::from_millis(50);
    settings.agentd_env.insert(
        "FINITECHAT_HOME".to_owned(),
        data_dir.join("agent").display().to_string(),
    );
    settings
}

fn install_seed(settings: &ShellSettings, packed: &finite_release::PayloadPackOutput) {
    fs::copy(
        &packed.tarball_path,
        settings.seed_dir.join("payload.tar.gz"),
    )
    .unwrap();
    fs::copy(
        &packed.manifest_path,
        settings.seed_dir.join("payload.tar.gz.manifest.json"),
    )
    .unwrap();
}

async fn boot_seeded(root: &Path, version: &str, marker: &str) -> (ShellSettings, ShellRuntime) {
    let settings = test_settings(root);
    let packed = pack_payload(root, version, marker, false);
    install_seed(&settings, &packed);
    let runtime = ShellRuntime::boot(settings.clone()).await.unwrap();
    (settings, runtime)
}

async fn wait_for_marker(settings: &ShellSettings, expected: &str) {
    let marker = settings.data_dir.join("agent/agentd/generation-marker");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if fs::read_to_string(&marker).ok().as_deref() == Some(expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("agentd generation marker never became {expected}"));
}

async fn wait_for_flip_outcome(state: &SharedState, expected: FlipOutcome) -> FlipRecord {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(flip) = state.snapshot().last_flip
                && flip.outcome == expected
            {
                return flip;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("last_flip never reached {expected:?}"))
}

fn stage_request_for(packed: &finite_release::PayloadPackOutput, force: bool) -> StageRequest {
    serde_json::from_value(json!({
        "tarballPath": packed.tarball_path,
        "manifestPath": packed.manifest_path,
        "force": force,
    }))
    .unwrap()
}

// ---------------------------------------------------------------------------
// Seed boot
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn seed_boot_unpacks_verifies_fixes_up_and_starts_agentd() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();

    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v1")
    );
    let state = runtime.state.snapshot();
    let seed = state.seed.expect("seed recorded in state.json");
    assert_eq!(seed.version_label, "v1");
    assert_eq!(seed.tree_digest.len(), 64);
    assert_eq!(
        state.current_digest.as_deref(),
        Some(seed.tree_digest.as_str()),
        "seeding must record the current generation's digest"
    );

    // Fixup ran on the seeded generation.
    let generation = layout.generation_dir("v1");
    let hermes = fs::read_to_string(generation.join("hermes-venv/bin/hermes")).unwrap();
    assert!(
        hermes.starts_with(&format!(
            "#!{}\n",
            generation.join("hermes-venv/bin/python").display()
        )),
        "seed venv shebang must point at the generation's own venv python: {hermes}"
    );
    assert_eq!(
        fs::read_link(generation.join("hermes-venv/bin/python")).unwrap(),
        settings.shell_python
    );
    let cfg = fs::read_to_string(generation.join("hermes-venv/pyvenv.cfg")).unwrap();
    assert!(cfg.contains(&format!(
        "home = {}",
        settings.shell_python.parent().unwrap().display()
    )));

    // Shims exist and exec through the `current` symlink.
    let shim = fs::read_to_string(settings.shim_dir.join("finite-agentd")).unwrap();
    assert!(shim.contains(&format!(
        "exec \"{}/bin/finite-agentd\" \"$@\"",
        layout.current_link().display()
    )));
    assert!(settings.shim_dir.join("fbrain").exists());

    // agentd runs with FINITE_PAYLOAD_ROOT + prefixed PATH.
    wait_for_marker(&settings, "one").await;
    let seen_path = fs::read_to_string(settings.data_dir.join("agent/agentd/seen-path")).unwrap();
    let real_bin = fs::canonicalize(layout.current_link()).unwrap().join("bin");
    assert!(
        seen_path.starts_with(&format!("{}:", real_bin.display())),
        "agentd PATH must be prefixed with the generation's bin: {seen_path}"
    );

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn boot_refuses_an_unwritable_data_dir_and_an_unkeyed_seed() {
    let root = tempfile::tempdir().unwrap();
    let mut settings = test_settings(root.path());
    settings.data_dir = root.path().join("does-not-exist");
    let Err(error) = ShellRuntime::boot(settings).await else {
        panic!("boot must refuse a missing /data");
    };
    assert!(matches!(error, ShellError::DataDir(_)));

    // A first boot without the release public key refuses to unpack the
    // (unverifiable) seed.
    let mut unkeyed = test_settings(root.path());
    unkeyed.release_public_key = None;
    let packed = pack_payload(root.path(), "v1", "one", false);
    install_seed(&unkeyed, &packed);
    let Err(error) = ShellRuntime::boot(unkeyed).await else {
        panic!("first boot without a release key must refuse the seed");
    };
    assert!(matches!(error, ShellError::MissingReleaseKey));
}

// ---------------------------------------------------------------------------
// Stage
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn stage_from_local_files_verifies_and_records_the_generation() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();
    let packed = pack_payload(root.path(), "v2", "two", false);

    let staged = stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&packed, false),
    )
    .await
    .unwrap();
    assert_eq!(staged.version_label, "v2");
    assert_eq!(staged.tree_digest, packed.manifest.tree_digest);
    assert!(
        layout
            .generation_dir("v2")
            .join("bin/finite-agentd")
            .is_file()
    );
    assert_eq!(runtime.state.snapshot().staged.unwrap().version_label, "v2");

    // Idempotent re-stage of the same verified version.
    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&packed, false),
    )
    .await
    .unwrap();

    // Staging the current version is refused.
    let current = pack_payload(root.path(), "v1", "one-restage", false);
    let error = stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&current, false),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ShellError::VersionIsCurrent(_)));

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stage_refuses_tampered_bytes_wrong_keys_new_shells_and_bad_versions() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();

    // Tampered tarball.
    let tampered = pack_payload(root.path(), "v2", "two", false);
    let mut bytes = fs::read(&tampered.tarball_path).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x01;
    fs::write(&tampered.tarball_path, bytes).unwrap();
    let error = stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&tampered, false),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        ShellError::Release(finite_release::ReleaseError::VerificationFailed(_))
    ));
    assert!(
        !layout.generation_dir("v2").exists(),
        "a refused bundle must not appear under generations/"
    );

    // Wrong key.
    let mut wrong_key = settings.clone();
    wrong_key.release_public_key = Some(hex::encode(
        SigningKey::from_bytes(&[22_u8; 32])
            .verifying_key()
            .to_bytes(),
    ));
    let good = pack_payload(root.path(), "v2b", "twob", false);
    let error = stage_payload(
        &wrong_key,
        &layout,
        &runtime.state,
        &stage_request_for(&good, false),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            error,
            ShellError::Release(finite_release::ReleaseError::KeyIdMismatch { .. })
        ),
        "a wrong key is a diagnosable key_id mismatch: {error}"
    );

    // min_shell_version newer than this shell: a distinct error.
    let source = build_payload_source(root.path(), "future", false);
    let future = finite_release::pack_payload_bundle(
        &finite_release::PayloadPackRequest {
            source: &source,
            artifact_id: "payload-future",
            version_label: "v-future",
            min_shell_version: "99.0.0",
            source_git_sha: None,
            created_at: Some("2026-08-06T00:00:00Z"),
            out_dir: &root.path().join("packed-future"),
        },
        &test_signing_key(),
    )
    .unwrap();
    let error = stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&future, false),
    )
    .await
    .unwrap_err();
    let ShellError::MinShellVersion { required, shell } = &error else {
        panic!("expected MinShellVersion, got {error:?}");
    };
    assert_eq!(required, "99.0.0");
    assert_eq!(shell, SHELL_VERSION);
    assert_eq!(error.code(), "min_shell_version_unsatisfied");

    // Bad-list refusal, then force override.
    let bad = pack_payload(root.path(), "v3", "three", false);
    runtime
        .state
        .update(|state| {
            state.bad.push(finite_shell::state::BadGeneration {
                version_label: "v3".to_owned(),
                reason: "health gate failed".to_owned(),
                at: "2026-08-06T00:00:00Z".to_owned(),
            })
        })
        .unwrap();
    let error = stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&bad, false),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ShellError::VersionBad(_)));
    assert_eq!(error.code(), "version_bad");
    let staged = stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&bad, true),
    )
    .await
    .unwrap();
    assert_eq!(staged.version_label, "v3");
    assert!(
        !runtime.state.snapshot().is_bad("v3"),
        "a forced re-stage clears the bad entry for a clean retry"
    );

    runtime.shutdown().await;
}

async fn serve_bytes(routes: Vec<(String, Vec<u8>)>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let routes = routes.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0_u8; 8192];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                match routes.iter().find(|(route, _)| *route == path) {
                    Some((_, body)) => {
                        let header = format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(header.as_bytes()).await;
                        let _ = stream.write_all(body).await;
                    }
                    None => {
                        let _ = stream
                            .write_all(
                                b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                            )
                            .await;
                    }
                }
                let _ = stream.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn stage_from_urls_enforces_the_release_url_policy_and_verifies() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();
    let packed = pack_payload(root.path(), "v2", "two", false);
    let base_url = serve_bytes(vec![
        (
            "/payload-v2.tar.gz".to_owned(),
            fs::read(&packed.tarball_path).unwrap(),
        ),
        (
            "/payload-v2.tar.gz.manifest.json".to_owned(),
            fs::read(&packed.manifest_path).unwrap(),
        ),
    ])
    .await;
    let request: StageRequest = serde_json::from_value(json!({
        "tarballUrl": format!("{base_url}/payload-v2.tar.gz"),
        "manifestUrl": format!("{base_url}/payload-v2.tar.gz.manifest.json"),
        "tarballSha256": packed.manifest.tarball_sha256,
    }))
    .unwrap();

    // Plain http is refused without the dev escape hatch.
    let error = stage_payload(&settings, &layout, &runtime.state, &request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "url_policy");
    assert!(error.to_string().contains("https"));

    // With the escape hatch the bundle is fetched, verified, and staged.
    let mut insecure = settings.clone();
    insecure.allow_insecure_bundle_url = true;
    let staged = stage_payload(&insecure, &layout, &runtime.state, &request)
        .await
        .unwrap();
    assert_eq!(staged.version_label, "v2");
    assert_eq!(staged.tarball_sha256, packed.manifest.tarball_sha256);

    // A caller sha256 disagreeing with the bytes is refused.
    let mut wrong_sha = request.clone();
    wrong_sha.tarball_sha256 = Some("c".repeat(64));
    let error = stage_payload(&insecure, &layout, &runtime.state, &wrong_sha)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ShellError::Release(finite_release::ReleaseError::VerificationFailed(_))
    ));

    // URL and path forms are mutually exclusive.
    let mixed: StageRequest = serde_json::from_value(json!({
        "tarballUrl": format!("{base_url}/payload-v2.tar.gz"),
        "manifestUrl": format!("{base_url}/payload-v2.tar.gz.manifest.json"),
        "tarballSha256": packed.manifest.tarball_sha256,
        "tarballPath": packed.tarball_path,
        "manifestPath": packed.manifest_path,
    }))
    .unwrap();
    let error = stage_payload(&insecure, &layout, &runtime.state, &mixed)
        .await
        .unwrap_err();
    assert!(matches!(error, ShellError::InvalidRequest(_)));

    runtime.shutdown().await;
}

// ---------------------------------------------------------------------------
// Flip + rollback
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn flip_succeeds_end_to_end_with_shims_and_venv_fixup() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();
    wait_for_marker(&settings, "one").await;

    let packed = pack_payload(root.path(), "v2", "two", false);
    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&packed, false),
    )
    .await
    .unwrap();

    let response = authed_request(&runtime, json!({ "verb": "flip" })).await;
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["result"], json!("flip_started"));
    assert_eq!(response["from"], json!("v1"));
    assert_eq!(response["to"], json!("v2"));

    let flip = wait_for_flip_outcome(&runtime.state, FlipOutcome::Success).await;
    assert_eq!(flip.from.as_deref(), Some("v1"));
    assert_eq!(flip.to, "v2");

    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v2")
    );
    assert_eq!(
        read_link_version(&layout.previous_link()).as_deref(),
        Some("v1")
    );
    let state = runtime.state.snapshot();
    assert!(state.staged.is_none(), "staged consumed");
    assert_eq!(
        state.current_digest.as_deref(),
        Some(packed.manifest.tree_digest.as_str()),
        "the flip must carry the staged digest into current_digest"
    );
    wait_for_marker(&settings, "two").await;

    // The v2 venv was fixed up for v2's real location.
    let hermes =
        fs::read_to_string(layout.generation_dir("v2").join("hermes-venv/bin/hermes")).unwrap();
    assert!(hermes.starts_with(&format!(
        "#!{}\n",
        layout.generation_dir("v2").join("hermes-venv/bin/python").display()
    )));

    // Shims still exec through `current` (content unchanged, rewritten).
    let shim = fs::read_to_string(settings.shim_dir.join("finite-agentd")).unwrap();
    assert!(shim.contains(&format!(
        "exec \"{}/bin/finite-agentd\" \"$@\"",
        layout.current_link().display()
    )));

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_health_gate_rolls_back_automatically_and_bad_lists_the_version() {
    let root = tempfile::tempdir().unwrap();
    let (mut settings, _) = {
        // Shrink the health timeout before boot so the runtime uses it.
        let settings = test_settings(root.path());
        let packed = pack_payload(root.path(), "v1", "one", false);
        install_seed(&settings, &packed);
        (settings, ())
    };
    settings.health_timeout = Duration::from_millis(1500);
    let runtime = ShellRuntime::boot(settings.clone()).await.unwrap();
    let layout = settings.layout();
    wait_for_marker(&settings, "one").await;

    let broken = pack_payload(root.path(), "v2-broken", "broken", true);
    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&broken, false),
    )
    .await
    .unwrap();
    let response = authed_request(&runtime, json!({ "verb": "flip" })).await;
    assert_eq!(response["ok"], json!(true));

    let flip = wait_for_flip_outcome(&runtime.state, FlipOutcome::RolledBack).await;
    assert!(flip.detail.unwrap().contains("health gate failed"));

    // Symlinks restored, v1 running again, v2-broken bad-listed.
    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v1")
    );
    assert_eq!(read_link_version(&layout.previous_link()), None);
    let state = runtime.state.snapshot();
    assert!(state.is_bad("v2-broken"));
    assert!(state.staged.is_none());
    wait_for_marker(&settings, "one").await;

    // The bad version cannot be staged again without force.
    let error = stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&broken, false),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ShellError::VersionBad(_)));

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rollback_flips_back_to_previous_and_never_marks_bad() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();
    wait_for_marker(&settings, "one").await;

    let packed = pack_payload(root.path(), "v2", "two", false);
    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&packed, false),
    )
    .await
    .unwrap();
    authed_request(&runtime, json!({ "verb": "flip" })).await;
    wait_for_flip_outcome(&runtime.state, FlipOutcome::Success).await;
    wait_for_marker(&settings, "two").await;

    let response = authed_request(&runtime, json!({ "verb": "rollback" })).await;
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["result"], json!("rollback_started"));
    assert_eq!(response["from"], json!("v2"));
    assert_eq!(response["to"], json!("v1"));

    let flip = wait_for_flip_outcome(&runtime.state, FlipOutcome::Success).await;
    assert_eq!(flip.to, "v1");
    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v1")
    );
    assert_eq!(
        read_link_version(&layout.previous_link()).as_deref(),
        Some("v2")
    );
    wait_for_marker(&settings, "one").await;
    let state = runtime.state.snapshot();
    assert!(state.bad.is_empty(), "rollback never marks versions bad");
    assert_eq!(
        state.current_digest, None,
        "a rollback target has no digest on record; null, not a stale digest"
    );

    // Nothing staged: flip refuses distinctly.
    let response = authed_request(&runtime, json!({ "verb": "flip" })).await;
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["error"]["code"], json!("nothing_staged"));

    runtime.shutdown().await;
}

// ---------------------------------------------------------------------------
// Crash reconcile
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn boot_reconciles_an_interrupted_flip_from_the_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();
    let packed = pack_payload(root.path(), "v2", "two", false);
    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&packed, false),
    )
    .await
    .unwrap();
    runtime.shutdown().await;

    // Simulate a shell death mid-flip, before the current swap committed:
    // state says in_progress, symlinks still say v1.
    runtime
        .state
        .update(|state| {
            state.last_flip = Some(FlipRecord {
                from: Some("v1".to_owned()),
                to: "v2".to_owned(),
                at: "2026-08-06T00:00:00Z".to_owned(),
                outcome: FlipOutcome::InProgress,
                prior_previous: None,
                detail: None,
            });
        })
        .unwrap();

    let rebooted = ShellRuntime::boot(settings.clone()).await.unwrap();
    let state = rebooted.state.snapshot();
    let flip = state.last_flip.clone().unwrap();
    assert_eq!(flip.outcome, FlipOutcome::Interrupted);
    assert!(flip.detail.unwrap().contains("current=v1"));
    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v1")
    );
    assert_eq!(
        state.staged.unwrap().version_label,
        "v2",
        "an uncommitted flip leaves the staged generation intact"
    );
    rebooted.shutdown().await;

    // Simulate death after the commit point: symlinks already say v2.
    finite_shell::generations::set_link_version(&layout.previous_link(), "v1").unwrap();
    finite_shell::generations::set_link_version(&layout.current_link(), "v2").unwrap();
    rebooted
        .state
        .update(|state| {
            if let Some(flip) = state.last_flip.as_mut() {
                flip.outcome = FlipOutcome::InProgress;
                flip.detail = None;
            }
        })
        .unwrap();
    let rebooted = ShellRuntime::boot(settings.clone()).await.unwrap();
    let state = rebooted.state.snapshot();
    let flip = state.last_flip.unwrap();
    // The commit landed but the gate never resolved: boot reruns the gate
    // against the adopted (healthy) generation and records the result
    // honestly instead of leaving an ungated candidate marked "interrupted".
    assert_eq!(flip.outcome, FlipOutcome::Success);
    assert!(flip.detail.unwrap().contains("gate rerun at boot passed"));
    assert!(
        state.staged.is_none(),
        "a committed flip consumed the staged generation"
    );
    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v2")
    );
    wait_for_marker(&settings, "two").await;
    rebooted.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn boot_reruns_the_gate_for_a_committed_broken_flip_and_rolls_back() {
    let root = tempfile::tempdir().unwrap();
    let (mut settings, _) = {
        let settings = test_settings(root.path());
        let packed = pack_payload(root.path(), "v1", "one", false);
        install_seed(&settings, &packed);
        (settings, ())
    };
    settings.health_timeout = Duration::from_millis(1500);
    let runtime = ShellRuntime::boot(settings.clone()).await.unwrap();
    let layout = settings.layout();
    wait_for_marker(&settings, "one").await;
    let broken = pack_payload(root.path(), "v2-broken", "broken", true);
    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&broken, false),
    )
    .await
    .unwrap();
    runtime.shutdown().await;

    // Simulate a crash after the commit point of a flip to the broken
    // candidate, before its health gate could resolve.
    finite_shell::generations::set_link_version(&layout.previous_link(), "v1").unwrap();
    finite_shell::generations::set_link_version(&layout.current_link(), "v2-broken").unwrap();
    runtime
        .state
        .update(|state| {
            state.last_flip = Some(FlipRecord {
                from: Some("v1".to_owned()),
                to: "v2-broken".to_owned(),
                at: "2026-08-06T00:00:00Z".to_owned(),
                outcome: FlipOutcome::InProgress,
                prior_previous: None,
                detail: None,
            });
        })
        .unwrap();

    let rebooted = ShellRuntime::boot(settings.clone()).await.unwrap();
    let state = rebooted.state.snapshot();
    let flip = state.last_flip.clone().unwrap();
    assert_eq!(
        flip.outcome,
        FlipOutcome::RolledBack,
        "the rerun gate must fail the broken candidate and restore v1: {:?}",
        flip.detail
    );
    assert!(flip.detail.unwrap().contains("boot gate rerun failed"));
    assert!(
        state.is_bad("v2-broken"),
        "the failed candidate lands on the bad list"
    );
    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v1")
    );
    wait_for_marker(&settings, "one").await;
    rebooted.shutdown().await;
}

// ---------------------------------------------------------------------------
// Socket protocol
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn every_verb_works_over_a_real_unix_socket() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();
    let listener = runtime.bind_socket().unwrap();
    let socket_path = layout.socket_path();
    let mode = fs::metadata(&socket_path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "socket must be 0600");
    let shell_dir_mode = fs::metadata(layout.shell_dir())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(shell_dir_mode & 0o777, 0o700, "shell dir must be 0700");
    tokio::spawn(runtime.clone().serve_socket(listener));

    // The token file exists (0600) beside the socket and matches the
    // runtime's token — `finite-shell ctl` reads it directly.
    let token_path = layout.control_token_path();
    let token_mode = fs::metadata(&token_path).unwrap().permissions().mode();
    assert_eq!(token_mode & 0o777, 0o600, "token file must be 0600");
    assert_eq!(
        finite_shell::control::read_control_token_near(&socket_path).as_deref(),
        Some(runtime.control_token()),
        "ctl derives the token from the file beside the socket"
    );

    let token = runtime.control_token().to_owned();
    let roundtrip = move |request: Value| {
        let socket_path = socket_path.clone();
        let mut request = request;
        request
            .as_object_mut()
            .unwrap()
            .insert("token".to_owned(), json!(token.clone()));
        tokio::task::spawn_blocking(move || ctl_roundtrip(&socket_path, &request))
    };
    let raw_roundtrip = {
        let socket_path = layout.socket_path();
        move |request: Value| {
            let socket_path = socket_path.clone();
            tokio::task::spawn_blocking(move || ctl_roundtrip(&socket_path, &request))
        }
    };

    // No token / a wrong token: refused with the distinct unauthorized code,
    // and no verb machinery runs.
    let response = raw_roundtrip(json!({ "verb": "status" }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["error"]["code"], json!("unauthorized"));
    let response = raw_roundtrip(json!({ "verb": "rollback", "token": "wrong" }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response["error"]["code"], json!("unauthorized"));

    // status
    let status = roundtrip(json!({ "verb": "status" }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status["ok"], json!(true));
    assert_eq!(status["shellVersion"], json!(SHELL_VERSION));
    assert_eq!(status["current"], json!("v1"));
    assert_eq!(status["state"]["schema"], json!("finite_shell_state.v1"));
    assert!(status["state"]["seed"].is_object());

    // set-channel (valid + invalid)
    let response = roundtrip(json!({ "verb": "set-channel", "channel": "canary" }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response, json!({ "ok": true, "channel": "canary" }));
    assert_eq!(
        fs::read_to_string(layout.channel_path()).unwrap(),
        "canary\n"
    );
    let response = roundtrip(json!({ "verb": "set-channel", "channel": "yolo" }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["error"]["code"], json!("invalid_request"));

    // stage (local form)
    let packed = pack_payload(root.path(), "v2", "two", false);
    let response = roundtrip(json!({
        "verb": "stage",
        "tarballPath": packed.tarball_path,
        "manifestPath": packed.manifest_path,
    }))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["result"], json!("staged"));
    assert_eq!(response["versionLabel"], json!("v2"));
    assert_eq!(response["treeDigest"], json!(packed.manifest.tree_digest));

    // flip responds immediately, then status resolves it (what
    // `ctl flip --wait` polls).
    let response = roundtrip(json!({ "verb": "flip" })).await.unwrap().unwrap();
    assert_eq!(response["ok"], json!(true));
    assert_eq!(response["result"], json!("flip_started"));
    wait_for_flip_outcome(&runtime.state, FlipOutcome::Success).await;
    let status = roundtrip(json!({ "verb": "status" }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status["current"], json!("v2"));
    assert_eq!(status["previous"], json!("v1"));
    assert_eq!(status["state"]["last_flip"]["outcome"], json!("success"));
    assert_eq!(status["channel"], json!("canary"));
    assert_eq!(status["agentd"]["state"], json!("running"));

    // rollback over the socket
    let response = roundtrip(json!({ "verb": "rollback" }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response["ok"], json!(true));
    wait_for_flip_outcome(&runtime.state, FlipOutcome::Success).await;

    // rollback with no previous → distinct error
    finite_shell::generations::remove_link(&layout.previous_link()).unwrap();
    let response = roundtrip(json!({ "verb": "rollback" }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response["error"]["code"], json!("no_previous_generation"));

    // unknown verb + malformed line
    let response = roundtrip(json!({ "verb": "explode" }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["error"]["code"], json!("invalid_request"));
    let response = runtime.handle_request_line("not json at all").await;
    assert_eq!(response["error"]["code"], json!("invalid_request"));

    runtime.shutdown().await;
}

// ---------------------------------------------------------------------------
// Autonomous channel polling (M4)
// ---------------------------------------------------------------------------

type SharedRoutes = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

/// Like `serve_bytes`, but the routes can be swapped between requests — the
/// fixture directory server tests repoint mid-test.
async fn serve_shared(routes: SharedRoutes) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let routes = std::sync::Arc::clone(&routes);
            tokio::spawn(async move {
                let mut buffer = vec![0_u8; 8192];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body = routes
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(route, _)| *route == path)
                    .map(|(_, body)| body.clone());
                match body {
                    Some(body) => {
                        let header = format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(header.as_bytes()).await;
                        let _ = stream.write_all(&body).await;
                    }
                    None => {
                        let _ = stream
                            .write_all(
                                b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                            )
                            .await;
                    }
                }
                let _ = stream.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}

/// Point the fixture directory at `packed` as `channel`'s payload head (or at
/// no head at all), serving the signed directory plus the bundle bytes.
fn set_directory_head(
    routes: &SharedRoutes,
    base_url: &str,
    channel: &str,
    packed: Option<&finite_release::PayloadPackOutput>,
) {
    set_directory_head_at(routes, base_url, channel, packed, &now_rfc3339());
}

/// [`set_directory_head`] with an explicit `generated_at` (the anti-replay
/// tests need control over document age).
fn set_directory_head_at(
    routes: &SharedRoutes,
    base_url: &str,
    channel: &str,
    packed: Option<&finite_release::PayloadPackOutput>,
    generated_at: &str,
) {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut channels = std::collections::BTreeMap::new();
    if let Some(packed) = packed {
        let name = format!("{}.tar.gz", packed.manifest.version_label);
        entries.push((format!("/{name}"), fs::read(&packed.tarball_path).unwrap()));
        entries.push((
            format!("/{name}.manifest.json"),
            fs::read(&packed.manifest_path).unwrap(),
        ));
        channels.insert(
            channel.to_owned(),
            std::collections::BTreeMap::from([(
                PAYLOAD_BUNDLE_KIND.to_owned(),
                ChannelBundleV1 {
                    artifact_id: packed.manifest.artifact_id.clone(),
                    version_label: packed.manifest.version_label.clone(),
                    tarball_url: format!("{base_url}/{name}"),
                    manifest_url: format!("{base_url}/{name}.manifest.json"),
                    tarball_sha256: packed.manifest.tarball_sha256.clone(),
                },
            )]),
        );
    }
    let mut directory = ServiceDirectoryV1 {
        schema: SERVICE_DIRECTORY_SCHEMA.to_owned(),
        generated_at: generated_at.to_owned(),
        services: std::collections::BTreeMap::new(),
        channels,
        key_id: String::new(),
        signature: String::new(),
    };
    directory.sign(&test_signing_key()).unwrap();
    entries.push((
        "/api/core/v1/service-directory".to_owned(),
        serde_json::to_vec(&directory).unwrap(),
    ));
    *routes.lock().unwrap() = entries;
}

/// Boot a seeded v1 shell wired to a fixture directory server whose head
/// starts as the seed itself (a converged fleet).
async fn boot_with_directory(
    root: &Path,
    poll_interval: Duration,
) -> (ShellSettings, ShellRuntime, SharedRoutes, String) {
    let routes: SharedRoutes = SharedRoutes::default();
    let base_url = serve_shared(std::sync::Arc::clone(&routes)).await;
    let mut settings = test_settings(root);
    settings.allow_insecure_bundle_url = true;
    settings.poll_interval = poll_interval;
    settings.service_directory_url = Some(format!("{base_url}/api/core/v1/service-directory"));
    let packed = pack_payload(root, "v1", "one", false);
    install_seed(&settings, &packed);
    set_directory_head(&routes, &base_url, "stable", Some(&packed));
    let runtime = ShellRuntime::boot(settings.clone()).await.unwrap();
    (settings, runtime, routes, base_url)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_poller_stages_and_flips_to_a_new_head_unattended() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime, routes, base_url) =
        boot_with_directory(root.path(), Duration::from_millis(150)).await;
    wait_for_marker(&settings, "one").await;

    // The agent is moved to canary, whose head is v2 — exactly the smoke's
    // set-channel + publish sequence, with no further driving.
    authed_request(
        &runtime,
        json!({ "verb": "set-channel", "channel": "canary" }),
    )
    .await;
    let packed_v2 = pack_payload(root.path(), "v2", "two", false);
    set_directory_head(&routes, &base_url, "canary", Some(&packed_v2));
    assert!(runtime.spawn_channel_poller());

    let flip = wait_for_flip_outcome(&runtime.state, FlipOutcome::Success).await;
    assert_eq!(flip.from.as_deref(), Some("v1"));
    assert_eq!(flip.to, "v2");
    wait_for_marker(&settings, "two").await;
    let layout = settings.layout();
    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v2")
    );
    assert_eq!(
        read_link_version(&layout.previous_link()).as_deref(),
        Some("v1")
    );

    // The tick that converged recorded flip_started; once converged the loop
    // settles on action "none" with the head still in view.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(poll) = runtime.state.snapshot().last_poll
                && poll.action == PollAction::None
                && poll.head_version.as_deref() == Some("v2")
            {
                assert_eq!(poll.channel, "canary");
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the poll loop never settled on the converged head");

    // /healthz surfaces the channel and the last poll outcome.
    let payload = health::runtime_health(&settings, &layout, &runtime.state.snapshot(), None);
    assert_eq!(payload["channel"], json!("canary"));
    assert_eq!(payload["last_poll"]["action"], json!("none"));
    assert_eq!(payload["last_poll"]["head_version"], json!("v2"));

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_poller_leaves_bad_listed_and_min_shell_unsatisfied_heads_alone() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime, routes, base_url) =
        boot_with_directory(root.path(), Duration::ZERO).await;
    wait_for_marker(&settings, "one").await;
    let layout = settings.layout();

    // A bad-listed head (a previous flip's health gate failed) is skipped —
    // the loop must not fight the rollback machinery.
    let packed_v2 = pack_payload(root.path(), "v2", "two", false);
    set_directory_head(&routes, &base_url, "stable", Some(&packed_v2));
    runtime
        .state
        .update(|state| {
            state.bad.push(finite_shell::state::BadGeneration {
                version_label: "v2".to_owned(),
                reason: "health gate failed".to_owned(),
                at: "2026-08-06T00:00:00Z".to_owned(),
            })
        })
        .unwrap();
    let record = runtime.poll_channel_once().await;
    assert_eq!(record.action, PollAction::SkippedBad);
    assert_eq!(record.head_version.as_deref(), Some("v2"));
    assert_eq!(record.channel, "stable");
    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v1")
    );
    assert!(runtime.state.snapshot().staged.is_none());

    // A head requiring a newer shell is skipped with its own action.
    let source = build_payload_source(root.path(), "future", false);
    let future = finite_release::pack_payload_bundle(
        &finite_release::PayloadPackRequest {
            source: &source,
            artifact_id: "payload-v-future",
            version_label: "v-future",
            min_shell_version: "99.0.0",
            source_git_sha: None,
            created_at: Some("2026-08-06T00:00:00Z"),
            out_dir: &root.path().join("packed-v-future"),
        },
        &test_signing_key(),
    )
    .unwrap();
    set_directory_head(&routes, &base_url, "stable", Some(&future));
    let record = runtime.poll_channel_once().await;
    assert_eq!(record.action, PollAction::SkippedMinShell);
    assert_eq!(record.head_version.as_deref(), Some("v-future"));
    assert!(!layout.generation_dir("v-future").exists());
    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v1")
    );

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_poller_no_ops_on_a_converged_or_headless_channel() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime, routes, base_url) =
        boot_with_directory(root.path(), Duration::ZERO).await;
    wait_for_marker(&settings, "one").await;

    // Head == current: nothing to do.
    let record = runtime.poll_channel_once().await;
    assert_eq!(record.action, PollAction::None);
    assert_eq!(record.head_version.as_deref(), Some("v1"));
    assert_eq!(record.channel, "stable");

    // A channel with no payload head: nothing to converge on.
    set_directory_head(&routes, &base_url, "stable", None);
    let record = runtime.poll_channel_once().await;
    assert_eq!(record.action, PollAction::None);
    assert_eq!(record.head_version, None);

    // An unreachable directory is an error tick, and the state file records
    // it atomically for /healthz.
    *routes.lock().unwrap() = Vec::new();
    let record = runtime.poll_channel_once().await;
    assert_eq!(record.action, PollAction::Error);
    let reloaded = SharedState::load(&settings.layout().state_path()).snapshot();
    assert_eq!(reloaded.last_poll.unwrap().action, PollAction::Error);

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_poll_tick_concurrent_with_a_manual_flip_skips_instead_of_double_flipping() {
    let root = tempfile::tempdir().unwrap();
    // A bespoke v1 seed whose agentd holds its TERM drain until the test
    // deletes a sentinel file: the flip stays in its quiesce phase — gate
    // held, `current` still v1 — for exactly as long as the concurrent tick
    // needs to observe it. No timing window: the tick's verdict is asserted
    // before the sentinel is released.
    let routes: SharedRoutes = SharedRoutes::default();
    let base_url = serve_shared(std::sync::Arc::clone(&routes)).await;
    let mut settings = test_settings(root.path());
    settings.allow_insecure_bundle_url = true;
    settings.poll_interval = Duration::ZERO;
    settings.service_directory_url = Some(format!("{base_url}/api/core/v1/service-directory"));
    // Generous quiesce bound: the drain is released explicitly below, so the
    // timeout only matters if the test itself is wedged.
    settings.quiesce_timeout = Duration::from_secs(30);
    let quiesce_hold = root.path().join("hold-v1-quiesce");
    fs::write(&quiesce_hold, b"").unwrap();
    let source_v1 = build_payload_source(root.path(), "one", false);
    let script_v1 = source_v1.join("bin/finite-agentd");
    let body_v1 = fs::read_to_string(&script_v1).unwrap();
    fs::write(
        &script_v1,
        body_v1.replacen(
            "exec sleep 300",
            &format!(
                "trap 'while [ -e {} ]; do sleep 0.05; done; exit 0' TERM\nsleep 300 & wait",
                quiesce_hold.display()
            ),
            1,
        ),
    )
    .unwrap();
    let packed_v1 = finite_release::pack_payload_bundle(
        &finite_release::PayloadPackRequest {
            source: &source_v1,
            artifact_id: "payload-v1",
            version_label: "v1",
            min_shell_version: "0.1.0",
            source_git_sha: None,
            created_at: Some("2026-08-06T00:00:00Z"),
            out_dir: &root.path().join("packed-v1-slow-drain"),
        },
        &test_signing_key(),
    )
    .unwrap();
    install_seed(&settings, &packed_v1);
    set_directory_head(&routes, &base_url, "stable", Some(&packed_v1));
    let runtime = ShellRuntime::boot(settings.clone()).await.unwrap();
    wait_for_marker(&settings, "one").await;
    let layout = settings.layout();

    // v2 is an ordinary payload: the deterministic window comes entirely
    // from v1's held quiesce above.
    let packed_v2 = pack_payload(root.path(), "v2", "two", false);
    set_directory_head(&routes, &base_url, "stable", Some(&packed_v2));

    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&packed_v2, false),
    )
    .await
    .unwrap();
    let response = authed_request(&runtime, json!({ "verb": "flip" })).await;
    assert_eq!(response["ok"], json!(true));

    // A tick during the manual flip must neither deadlock nor start a second
    // transition: it observes the gate and skips. The flip is pinned in its
    // quiesce phase until the sentinel below is removed, so this verdict
    // cannot race the swap.
    let record = tokio::time::timeout(Duration::from_secs(5), runtime.poll_channel_once())
        .await
        .expect("the poll tick must not deadlock against a manual flip");
    assert_eq!(record.action, PollAction::None);
    assert!(record.detail.unwrap().contains("in progress"));

    // Release v1's drain; the flip proceeds to completion.
    fs::remove_file(&quiesce_hold).unwrap();
    let flip = wait_for_flip_outcome(&runtime.state, FlipOutcome::Success).await;
    assert_eq!(flip.to, "v2");
    wait_for_marker(&settings, "two").await;
    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v2")
    );
    assert_eq!(
        read_link_version(&layout.previous_link()).as_deref(),
        Some("v1"),
        "exactly one flip happened"
    );

    // The next tick sees the manual flip already converged on the head.
    let record = runtime.poll_channel_once().await;
    assert_eq!(record.action, PollAction::None);
    assert_eq!(record.head_version.as_deref(), Some("v2"));

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn interval_zero_or_a_missing_directory_url_disables_the_poller() {
    let root = tempfile::tempdir().unwrap();
    let (_, runtime, _routes, _base_url) = boot_with_directory(root.path(), Duration::ZERO).await;
    assert!(!runtime.spawn_channel_poller(), "interval 0 disables");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(runtime.state.snapshot().last_poll.is_none());
    runtime.shutdown().await;

    let unwired_root = tempfile::tempdir().unwrap();
    let settings = test_settings(unwired_root.path());
    let packed = pack_payload(unwired_root.path(), "v1", "one", false);
    install_seed(&settings, &packed);
    let runtime = ShellRuntime::boot(settings).await.unwrap();
    assert!(
        !runtime.spawn_channel_poller(),
        "no FINITE_SERVICE_DIRECTORY_URL disables"
    );
    runtime.shutdown().await;
}

// ---------------------------------------------------------------------------
// /healthz field compatibility with health_server.py
// ---------------------------------------------------------------------------

/// The canonical NIP-19 vector from finite-identity's npub tests.
const VECTOR_PUBKEY_HEX: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
const VECTOR_NPUB: &str = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";

fn write_agent_fixture(layout: &finite_shell::DataLayout) {
    let agent = layout.agent_home();
    fs::create_dir_all(agent.join("identity")).unwrap();
    fs::create_dir_all(agent.join("agentd")).unwrap();
    fs::write(
        layout.agent_config_file(),
        json!({ "account_id": "account-1", "device_id": "agent" }).to_string(),
    )
    .unwrap();
    fs::write(
        layout.identity_file(),
        json!({
            "version": 1,
            "kind": "nostr-secp256k1",
            "secret_hex": "00".repeat(32),
            "public_key_hex": VECTOR_PUBKEY_HEX,
            "created_at": "2026-08-06T00:00:00Z",
            "created_by": "test",
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        layout.bridge_status_file(),
        json!({ "status": "running", "ok": true }).to_string(),
    )
    .unwrap();
    fs::write(
        layout.agentd_status_file(),
        json!({
            "version": "0.1.0",
            "processes": { "processes": {
                "finitechat": { "state": "running" },
                "health": { "state": "running" },
                "hermes": { "state": "running" },
            }},
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn healthz_replicates_health_server_py_fields_and_adds_shell_fields() {
    let root = tempfile::tempdir().unwrap();
    let settings = test_settings(root.path());
    let layout = settings.layout();
    write_agent_fixture(&layout);
    fs::create_dir_all(layout.generations_dir()).unwrap();
    finite_shell::generations::set_link_version(&layout.current_link(), "v2").unwrap();
    finite_shell::generations::set_link_version(&layout.previous_link(), "v1").unwrap();

    let state = SharedState::load(&layout.state_path());
    state
        .update(|shell_state| {
            shell_state.bad.push(finite_shell::state::BadGeneration {
                version_label: "v3".to_owned(),
                reason: "health gate failed".to_owned(),
                at: "2026-08-06T00:00:00Z".to_owned(),
            });
            shell_state.last_flip = Some(FlipRecord {
                from: Some("v1".to_owned()),
                to: "v2".to_owned(),
                at: "2026-08-06T00:00:00Z".to_owned(),
                outcome: FlipOutcome::Success,
                prior_previous: None,
                detail: None,
            });
            shell_state.current_digest = Some("d".repeat(64));
        })
        .unwrap();

    let supervision = running_supervision();
    let payload = health::runtime_health(&settings, &layout, &state.snapshot(), Some(&supervision));

    // The exact existing field names from health_server.py's logic.
    assert_eq!(payload["ready"], json!(true));
    assert_eq!(payload["npub"], json!(VECTOR_NPUB));
    assert_eq!(payload["agent_npub"], json!(VECTOR_NPUB));
    assert_eq!(payload["account_id"], json!("account-1"));
    assert_eq!(
        payload["bridge"],
        json!({ "status": "running", "ok": true })
    );
    assert_eq!(payload["agentd"]["status"], json!("running"));
    assert_eq!(payload["agentd"]["ok"], json!(true));
    assert_eq!(payload["agentd"]["version"], json!("0.1.0"));
    assert_eq!(
        payload["agentd"]["processes"],
        json!({ "finitechat": "running", "health": "running", "hermes": "running" })
    );
    assert!(
        payload.get("startup").is_none(),
        "no startup report file, no startup field"
    );

    // The new shell fields.
    assert_eq!(payload["shell_version"], json!(SHELL_VERSION));
    assert_eq!(payload["payload_version"], json!("v2"));
    assert_eq!(payload["payload_digest"], json!("d".repeat(64)));
    assert_eq!(
        payload["payload_generations"],
        json!({ "current": "v2", "previous": "v1", "staged": null, "bad": ["v3"] })
    );
    assert_eq!(payload["last_flip"]["outcome"], json!("success"));
    assert_eq!(payload["last_flip"]["from"], json!("v1"));
    assert_eq!(payload["last_flip"]["to"], json!("v2"));

    assert!(health::runtime_ready(&payload));
    let (status, _) = health::respond("/healthz", || payload.clone());
    assert_eq!(status, 200);
    let (status, _) = health::respond("/contact", || payload.clone());
    assert_eq!(status, 200);
    let (status, _) = health::respond("/invite", || payload.clone());
    assert_eq!(status, 200, "/invite stays a compatibility alias");
    let (status, body) = health::respond("/other", || payload.clone());
    assert_eq!(status, 404);
    assert_eq!(body, json!({ "ready": false, "error": "not found" }));
}

#[test]
fn healthz_degrades_exactly_like_health_server_py() {
    let root = tempfile::tempdir().unwrap();
    let settings = test_settings(root.path());
    let layout = settings.layout();
    let state = SharedState::load(&layout.state_path());

    // No agent home yet: identity() reports uninitialized; the whole
    // response is 503 but still carries the shell's generation fields.
    let payload = health::runtime_health(&settings, &layout, &state.snapshot(), None);
    assert_eq!(payload["ready"], json!(false));
    assert_eq!(payload["error"], json!("agent home is not initialized"));
    assert_eq!(payload["agent_npub"], json!(null));
    assert_eq!(
        payload["bridge"],
        json!({ "status": "starting", "ok": false })
    );
    assert_eq!(
        payload["agentd"],
        json!({ "status": "starting", "ok": true }),
        "FINITE_AGENTD_REQUIRED unset: a missing status file is ok=True"
    );
    assert!(!health::runtime_ready(&payload));
    let (status, _) = health::respond("/healthz", || payload.clone());
    assert_eq!(status, 503);

    // FINITE_AGENTD_REQUIRED flips the missing-status-file default.
    let mut required = settings.clone();
    required.agentd_required = true;
    let payload = health::runtime_health(&required, &layout, &state.snapshot(), None);
    assert_eq!(
        payload["agentd"],
        json!({ "status": "starting", "ok": false })
    );

    // A recovery boot intent with no startup report is the distinct
    // startup_report_missing_for_recovery_intent shape.
    let mut recovery = settings.clone();
    recovery.recovery_boot_intent_active = true;
    let payload = health::runtime_health(&recovery, &layout, &state.snapshot(), None);
    assert_eq!(
        payload["startup"],
        json!({
            "schema_version": 1,
            "status": "unavailable",
            "phase": "blocked",
            "ok": false,
            "error_code": "startup_report_missing_for_recovery_intent",
        })
    );
    assert!(!health::runtime_ready(&payload));

    // A valid startup report is sanitized field by field like the Python.
    write_agent_fixture(&layout);
    fs::write(
        layout.startup_report_file(),
        json!({
            "schema_version": 1,
            "report_kind": "finite_agent_startup",
            "boot_mode": "recover_known_good",
            "status": "completed",
            "phase": "complete",
            "operation_id_hash": format!("sha256:{}", "a".repeat(64)),
            "idempotency": {
                "same_operation_replay": "no_op_after_terminal_state",
                "resumed_after_interruption": false,
            },
            "actions": [
                { "action": "transient_cleanup", "status": "completed", "count": 2 },
                { "action": "not_a_real_action", "status": "completed" },
            ],
            "refusals": [],
            "identity": { "npub": VECTOR_NPUB },
            "state_roots": {
                "/data": { "present": true, "writable": true },
                "/somewhere/else": { "present": true, "writable": true },
            },
            "acceptance_scope": { "phala_acceptance": "not_proven", "other": "not_proven" },
        })
        .to_string(),
    )
    .unwrap();
    let supervision = running_supervision();
    let payload = health::runtime_health(&settings, &layout, &state.snapshot(), Some(&supervision));
    let startup = &payload["startup"];
    assert_eq!(startup["ok"], json!(true));
    assert_eq!(startup["status"], json!("completed"));
    assert_eq!(startup["boot_mode"], json!("recover_known_good"));
    assert_eq!(
        startup["actions"],
        json!([{ "action": "transient_cleanup", "status": "completed", "count": 2 }]),
        "unknown actions are dropped exactly like the Python sanitizer"
    );
    assert_eq!(startup["identity"], json!({ "npub": VECTOR_NPUB }));
    assert_eq!(
        startup["state_roots"],
        json!({ "/data": { "present": true, "writable": true } })
    );
    assert_eq!(
        startup["acceptance_scope"],
        json!({ "phala_acceptance": "not_proven" })
    );
    assert!(health::runtime_ready(&payload));

    // A garbled report degrades to the blocked shape.
    fs::write(layout.startup_report_file(), b"{").unwrap();
    let payload = health::runtime_health(&settings, &layout, &state.snapshot(), None);
    assert_eq!(
        payload["startup"]["error_code"],
        json!("startup_report_invalid")
    );
    assert!(!health::runtime_ready(&payload));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_http_server_serves_healthz_with_the_python_content_type_contract() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    wait_for_marker(&settings, "one").await;
    write_agent_fixture(&settings.layout());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(runtime.clone().serve_http(listener));

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nhost: test\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.contains("content-type: application/json"));
    let body: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["npub"], json!(VECTOR_NPUB));
    assert_eq!(body["payload_version"], json!("v1"));
    assert_eq!(body["agentd_supervision"]["state"], json!("running"));

    runtime.shutdown().await;
}

// ---------------------------------------------------------------------------
// Live-supervision readiness (the fail-open fix)
// ---------------------------------------------------------------------------

#[test]
fn a_dead_agentd_fails_readiness_despite_fresh_looking_status_files() {
    let root = tempfile::tempdir().unwrap();
    let settings = test_settings(root.path());
    let layout = settings.layout();
    // Every file-derived signal looks healthy: identity, bridge, agentd
    // status files are all present and fresh — exactly what a dead process
    // leaves behind.
    write_agent_fixture(&layout);
    let state = SharedState::load(&layout.state_path());

    // Dead agentd: the shell KNOWS this in memory, and it must win.
    let mut dead = running_supervision();
    dead.state = "exited".to_owned();
    dead.pid = None;
    let payload = health::runtime_health(&settings, &layout, &state.snapshot(), Some(&dead));
    assert_eq!(payload["ready"], json!(false));
    assert_eq!(payload["ready_reason"], json!("agentd_not_running"));
    // The file-derived compat fields are untouched.
    assert_eq!(payload["agentd"]["ok"], json!(true));
    assert!(!health::runtime_ready(&payload));
    let (status, _) = health::respond("/healthz", || payload.clone());
    assert_eq!(status, 503, "a dead-agentd shell serves 503");

    // Crash-looping agentd: distinct reason.
    let mut looping = running_supervision();
    looping.crash_looping = true;
    let payload = health::runtime_health(&settings, &layout, &state.snapshot(), Some(&looping));
    assert_eq!(payload["ready"], json!(false));
    assert_eq!(payload["ready_reason"], json!("agentd_crash_looping"));

    // No supervision at all (agentd slot empty mid-flip): not ready either.
    let payload = health::runtime_health(&settings, &layout, &state.snapshot(), None);
    assert_eq!(payload["ready"], json!(false));
    assert_eq!(payload["ready_reason"], json!("agentd_not_running"));

    // And a live one is ready again.
    let supervision = running_supervision();
    let payload = health::runtime_health(&settings, &layout, &state.snapshot(), Some(&supervision));
    assert_eq!(payload["ready"], json!(true));
    assert!(payload.get("ready_reason").is_none());
}

// ---------------------------------------------------------------------------
// Graceful quiesce: the flip waits for the old generation's drain
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_flip_lets_the_old_generation_drain_before_starting_the_new() {
    let root = tempfile::tempdir().unwrap();
    let settings = test_settings(root.path());
    // v1's agentd supervises a fake child; both trap TERM. On quiesce the
    // child writes its drain marker and exits cleanly, then agentd records
    // its own drain — proving the shell's bounded quiesce window is real
    // time to drain, not an instant kill.
    let source_v1 = build_payload_source(root.path(), "one", false);
    let script_v1 = source_v1.join("bin/finite-agentd");
    let body_v1 = fs::read_to_string(&script_v1).unwrap();
    fs::write(
        &script_v1,
        body_v1.replacen(
            "exec sleep 300",
            r#"(
  trap 'echo child-drained > "$FINITECHAT_HOME/agentd/drain-marker"; exit 0' TERM
  sleep 300 & wait
) &
child=$!
trap 'wait "$child"; echo agentd-drained >> "$FINITECHAT_HOME/agentd/drain-marker"; exit 0' TERM
wait "$child""#,
            1,
        ),
    )
    .unwrap();
    let packed_v1 = finite_release::pack_payload_bundle(
        &finite_release::PayloadPackRequest {
            source: &source_v1,
            artifact_id: "payload-v1",
            version_label: "v1",
            min_shell_version: "0.1.0",
            source_git_sha: None,
            created_at: Some("2026-08-06T00:00:00Z"),
            out_dir: &root.path().join("packed-v1-draining"),
        },
        &test_signing_key(),
    )
    .unwrap();
    install_seed(&settings, &packed_v1);
    let runtime = ShellRuntime::boot(settings.clone()).await.unwrap();
    wait_for_marker(&settings, "one").await;
    let layout = settings.layout();

    let packed_v2 = pack_payload(root.path(), "v2", "two", false);
    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&packed_v2, false),
    )
    .await
    .unwrap();
    let response = authed_request(&runtime, json!({ "verb": "flip" })).await;
    assert_eq!(response["ok"], json!(true));
    wait_for_flip_outcome(&runtime.state, FlipOutcome::Success).await;
    wait_for_marker(&settings, "two").await;

    let drain = fs::read_to_string(settings.data_dir.join("agent/agentd/drain-marker"))
        .expect("the drain marker must exist: the old generation got its TERM window");
    assert_eq!(
        drain, "child-drained\nagentd-drained\n",
        "both the fake child and agentd drained cleanly before the flip proceeded"
    );

    runtime.shutdown().await;
}

// ---------------------------------------------------------------------------
// Disk hygiene: sweep, prune, headroom
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_successful_flip_prunes_unreferenced_generations_but_keeps_their_bad_records() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();
    wait_for_marker(&settings, "one").await;

    // Debris: an unreferenced old generation, a bad-listed generation's
    // directory, and stale staging temp.
    fs::create_dir_all(layout.generation_dir("v0-old").join("bin")).unwrap();
    fs::create_dir_all(layout.generation_dir("v0-bad").join("bin")).unwrap();
    fs::create_dir_all(layout.generations_dir().join(".stage-tmp-junk")).unwrap();
    runtime
        .state
        .update(|state| {
            state.bad.push(finite_shell::state::BadGeneration {
                version_label: "v0-bad".to_owned(),
                reason: "health gate failed".to_owned(),
                at: "2026-08-06T00:00:00Z".to_owned(),
            })
        })
        .unwrap();

    let packed = pack_payload(root.path(), "v2", "two", false);
    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&packed, false),
    )
    .await
    .unwrap();
    assert!(
        !layout.generations_dir().join(".stage-tmp-junk").exists(),
        "staging sweeps stale temp dirs before it begins"
    );
    authed_request(&runtime, json!({ "verb": "flip" })).await;
    wait_for_flip_outcome(&runtime.state, FlipOutcome::Success).await;

    assert!(!layout.generation_dir("v0-old").exists(), "pruned");
    assert!(
        !layout.generation_dir("v0-bad").exists(),
        "a bad generation's bytes are reclaimed"
    );
    let state = runtime.state.snapshot();
    assert!(
        state.is_bad("v0-bad"),
        "the bad-list record is the memory and survives the prune"
    );
    assert!(layout.generation_dir("v1").is_dir(), "previous kept");
    assert!(layout.generation_dir("v2").is_dir(), "current kept");

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn boot_prunes_unreferenced_generations_and_sweeps_stale_temp() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();
    runtime.shutdown().await;

    fs::create_dir_all(layout.generation_dir("orphan").join("bin")).unwrap();
    fs::create_dir_all(layout.generations_dir().join(".seed-tmp")).unwrap();
    fs::create_dir_all(layout.staging_dir()).unwrap();
    fs::write(layout.staging_dir().join("payload.tar.gz"), b"junk").unwrap();

    let rebooted = ShellRuntime::boot(settings.clone()).await.unwrap();
    assert!(!layout.generation_dir("orphan").exists(), "boot prunes");
    assert!(
        !layout.generations_dir().join(".seed-tmp").exists(),
        "boot sweeps stale temp dirs"
    );
    assert!(
        !layout.staging_dir().exists(),
        "boot sweeps the staging dir"
    );
    assert!(layout.generation_dir("v1").is_dir(), "current survives");
    rebooted.shutdown().await;
}

// The disk-headroom rule is unit-tested deterministically in
// generations.rs (ensure_disk_headroom_refuses_a_tarball_larger_than_the_disk).
// The former integration test here faked a full disk with a 32 TiB sparse
// file, whose set_len a size-limited CI tmpfs rejects — the fragility, not the
// product code, was the failure.

// ---------------------------------------------------------------------------
// Shell-layer path containment
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_shell_refuses_version_labels_that_could_escape_generations() {
    use finite_shell::generations::{contained_generation_dir, validate_generation_name};

    let root = tempfile::tempdir().unwrap();
    let (_settings, runtime) = boot_seeded(root.path(), "v1", "one").await;

    for label in [
        "../x", "/abs", "a/b", "current", "previous", ".", "..", ".hidden", "",
    ] {
        let error = validate_generation_name(label).unwrap_err();
        assert!(
            matches!(error, ShellError::UnsafeVersionLabel(_)),
            "label {label:?} must be refused"
        );
        assert_eq!(error.code(), "unsafe_version_label");
        assert!(
            contained_generation_dir(&runtime.layout, label).is_err(),
            "label {label:?} must never resolve to a directory"
        );
    }
    validate_generation_name("v2.1_ok-label").unwrap();

    // A tampered state.json naming an escaping staged label is refused by
    // the flip engine before any filesystem operation.
    runtime
        .state
        .update(|state| {
            state.staged = Some(finite_shell::state::StagedGeneration {
                version_label: "../agent".to_owned(),
                artifact_id: "payload-evil".to_owned(),
                tree_digest: "a".repeat(64),
                tarball_sha256: "b".repeat(64),
                min_shell_version: "0.1.0".to_owned(),
                staged_at: "2026-08-06T00:00:00Z".to_owned(),
            });
        })
        .unwrap();
    let response = authed_request(&runtime, json!({ "verb": "flip" })).await;
    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["error"]["code"], json!("unsafe_version_label"));

    runtime.shutdown().await;
}

// ---------------------------------------------------------------------------
// Honest flip journal: rolled_back means restored; failed_open means not
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_post_commit_failure_restores_the_previous_generation_and_records_rolled_back() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();
    wait_for_marker(&settings, "one").await;

    // v2 ships an extra binary whose shim collides with a foreign file: the
    // transition errors AFTER the symlink commit (at shim rewrite), which
    // previously got relabelled "rolled_back" without any restoration.
    let source_v2 = build_payload_source(root.path(), "two", false);
    fs::write(source_v2.join("bin/v2only"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(
        source_v2.join("bin/v2only"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let packed_v2 = finite_release::pack_payload_bundle(
        &finite_release::PayloadPackRequest {
            source: &source_v2,
            artifact_id: "payload-v2",
            version_label: "v2",
            min_shell_version: "0.1.0",
            source_git_sha: None,
            created_at: Some("2026-08-06T00:00:00Z"),
            out_dir: &root.path().join("packed-v2-collide"),
        },
        &test_signing_key(),
    )
    .unwrap();
    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&packed_v2, false),
    )
    .await
    .unwrap();
    fs::write(settings.shim_dir.join("v2only"), "foreign binary").unwrap();

    let response = authed_request(&runtime, json!({ "verb": "flip" })).await;
    assert_eq!(response["ok"], json!(true));
    let flip = wait_for_flip_outcome(&runtime.state, FlipOutcome::RolledBack).await;
    assert!(flip.detail.unwrap().contains("transition failed"));

    // rolled_back is honest: the symlinks are back and v1's agentd runs.
    assert_eq!(
        read_link_version(&layout.current_link()).as_deref(),
        Some("v1")
    );
    wait_for_marker(&settings, "one").await;
    assert!(
        !runtime.state.snapshot().is_bad("v2"),
        "only health-gate failures bad-list a version"
    );

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_restoration_records_failed_open_and_healthz_surfaces_it() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime) = boot_seeded(root.path(), "v1", "one").await;
    let layout = settings.layout();
    wait_for_marker(&settings, "one").await;

    let packed_v2 = pack_payload(root.path(), "v2", "two", false);
    stage_payload(
        &settings,
        &layout,
        &runtime.state,
        &stage_request_for(&packed_v2, false),
    )
    .await
    .unwrap();
    // Both generations ship `fbrain`; clobbering its shim with a foreign
    // file makes the v2 shim write fail post-commit AND the v1 restoration
    // fail the same way: restoration is genuinely impossible.
    fs::write(settings.shim_dir.join("fbrain"), "foreign binary").unwrap();

    let response = authed_request(&runtime, json!({ "verb": "flip" })).await;
    assert_eq!(response["ok"], json!(true));
    let flip = wait_for_flip_outcome(&runtime.state, FlipOutcome::FailedOpen).await;
    let detail = flip.detail.unwrap();
    assert!(detail.contains("transition failed"));
    assert!(detail.contains("restoration failed"));

    // healthz surfaces the failed-open flip prominently: never ready, with
    // the distinct reason, regardless of any other signal.
    write_agent_fixture(&layout);
    let supervision = running_supervision();
    let payload = health::runtime_health(
        &settings,
        &layout,
        &runtime.state.snapshot(),
        Some(&supervision),
    );
    assert_eq!(payload["ready"], json!(false));
    assert_eq!(payload["ready_reason"], json!("flip_failed_open"));
    assert_eq!(payload["last_flip"]["outcome"], json!("failed_open"));
    let (status, _) = health::respond("/healthz", || payload.clone());
    assert_eq!(status, 503);

    runtime.shutdown().await;
}

// ---------------------------------------------------------------------------
// Degraded boot healthz (bootstrap_failed instead of exiting PID 1)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_corrupt_seed_serves_bootstrap_failed_healthz_instead_of_exiting() {
    use finite_shell::control::{BootHealth, boot_until_ready};

    let root = tempfile::tempdir().unwrap();
    let settings = test_settings(root.path());
    let packed = pack_payload(root.path(), "v1", "one", false);
    install_seed(&settings, &packed);
    // Corrupt the seed tarball: first boot's verification fails
    // deterministically, which used to exit PID 1 into a serve-nothing
    // restart loop.
    let seed_tarball = settings.seed_dir.join("payload.tar.gz");
    let mut bytes = fs::read(&seed_tarball).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x01;
    fs::write(&seed_tarball, bytes).unwrap();

    let boot_health = BootHealth::new();
    let boot_task = tokio::spawn(boot_until_ready(
        settings.clone(),
        boot_health.clone(),
        Duration::from_millis(50),
        Duration::from_millis(200),
    ));

    // The degraded body appears (and keeps being served while boot retries).
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if boot_health.health_body()["error"] == json!("bootstrap_failed") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("boot failures must surface as bootstrap_failed");
    let body = boot_health.health_body();
    assert_eq!(body["ready"], json!(false));
    assert_eq!(body["shell_version"], json!(SHELL_VERSION));
    assert!(
        body["detail"].as_str().unwrap().contains("verification"),
        "the sanitized failure detail names the cause: {body}"
    );

    // Served over real HTTP exactly like the booted shell would serve it.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    {
        let boot_health = boot_health.clone();
        tokio::spawn(health::serve_http(listener, move || {
            boot_health.health_body()
        }));
    }
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nhost: test\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8(response).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 503 "),
        "a bootstrap-failed shell answers 503, not connection-refused: {response}"
    );
    assert!(response.contains("bootstrap_failed"));

    boot_task.abort();
}

// ---------------------------------------------------------------------------
// Directory anti-replay: the poller's persisted floor
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_poller_refuses_replayed_directories_and_persists_the_floor() {
    let root = tempfile::tempdir().unwrap();
    let (settings, runtime, routes, base_url) =
        boot_with_directory(root.path(), Duration::ZERO).await;
    wait_for_marker(&settings, "one").await;
    // A same-label re-pack (distinct marker so the fixture tree does not
    // collide with the seed's): only its version label matters here — the
    // replayed documents below are refused before any staging.
    let packed_v1 = pack_payload(root.path(), "v1", "one-replay", false);

    // First tick accepts the fresh document and records the floor.
    let record = runtime.poll_channel_once().await;
    assert_eq!(record.action, PollAction::None);
    let floor = runtime
        .state
        .snapshot()
        .directory_floor
        .expect("an accepted directory advances the durable floor");

    // A replayed older (but validly signed) document is refused outright.
    let older = (time::OffsetDateTime::now_utc() - Duration::from_secs(3600))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    set_directory_head_at(&routes, &base_url, "stable", Some(&packed_v1), &older);
    let record = runtime.poll_channel_once().await;
    assert_eq!(record.action, PollAction::Error);
    assert!(
        record.detail.unwrap().contains("replay refused"),
        "the refusal is a distinct replay error"
    );
    assert_eq!(
        runtime.state.snapshot().directory_floor.as_deref(),
        Some(floor.as_str()),
        "a refused replay must not move the floor"
    );

    // An equal-timestamp document (Core idempotently re-serving) is fine.
    set_directory_head_at(&routes, &base_url, "stable", Some(&packed_v1), &floor);
    let record = runtime.poll_channel_once().await;
    assert_eq!(record.action, PollAction::None);

    // The floor is durable: a fresh state load sees it.
    let reloaded = SharedState::load(&settings.layout().state_path()).snapshot();
    assert_eq!(reloaded.directory_floor.as_deref(), Some(floor.as_str()));

    runtime.shutdown().await;
}
