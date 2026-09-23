//! Opt-in real-provider proof: isolated Postgres and two account identities,
//! with the shipped Runner binary, canonical image, and native authentication.
use super::*;
mod capacity;
mod dashboard;
mod predecessor;
mod recovery;
mod sites;
mod stalled_boot;
mod worker_loss;
use crate::auth::test_support::{core_auth_with_runner_credentials, runner_credential_config};
use crate::launch_codes::IssueLaunchCodeBatchInput;
use crate::{
    RuntimeArtifactKind, RuntimePlacement, RuntimeResourceClass, UpsertRuntimeArtifactInput,
};
use dashboard::dashboard_creation;
use std::time::Duration;
use worker_loss::crash_local_worker;

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn owner_session(index: usize) -> String {
    let user = format!("substrate-proof-{index}");
    access_token_with_subject(&user, &format!("{user}@finite.test"), true, None)
}

async fn native_grant(app: &axum::Router, path: &str, owner: &str) -> Value {
    let mut last_failure = Vec::new();
    for _ in 0..60 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("authorization", format!("Bearer {owner}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        if response.status() == StatusCode::OK {
            return serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                .unwrap();
        }
        last_failure = format!("{}\n", response.status()).into_bytes();
        last_failure.extend_from_slice(&to_bytes(response.into_body(), 65536).await.unwrap());
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    use std::io::Write;
    let mut log = tempfile::NamedTempFile::new().unwrap();
    log.write_all(&last_failure).unwrap();
    let (_, path) = log.keep().unwrap();
    panic!(
        "native readiness and Core session grant; private diagnostics: {}",
        path.display()
    );
}

// Exercise the same hosted Device control API used by dashboard Connections.
// Its identity must be the owner admitted by Core, not a synthetic account ID.
async fn device_request(
    device: &axum::Router,
    user: &str,
    path: &str,
    body: Option<Value>,
) -> Value {
    let response = device
        .clone()
        .oneshot(
            Request::builder()
                .method(if body.is_some() { "POST" } else { "GET" })
                .uri(path)
                .header("authorization", "Bearer disposable-control-proof")
                .header(finitechat_hosted_device::WORKOS_USER_HEADER, user)
                .header("content-type", "application/json")
                .body(Body::from(
                    body.map(|value| value.to_string()).unwrap_or_default(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    if status != StatusCode::OK {
        use std::io::Write;
        let mut log = tempfile::NamedTempFile::new().unwrap();
        log.write_all(&bytes).unwrap();
        let (_, diagnostic) = log.keep().unwrap();
        eprintln!(
            "private hosted Device diagnostics: {}",
            diagnostic.display()
        );
    }
    assert_eq!(status, StatusCode::OK, "hosted Device {path}");
    serde_json::from_slice(&bytes).unwrap()
}

async fn connection_command(
    device: &axum::Router,
    index: usize,
    binding: &Value,
    command: &str,
) -> Value {
    let result = device_request(
        device,
        &format!("substrate-proof-{index}"),
        "/v1/app/runtime-commands",
        Some(json!({
            "room_id": binding["canonical_room_id"],
            "target_account_id": binding["agent_account_id"],
            "command": command, "resource_key": "agent.connections",
            "schema": "finite.agent.empty.request.v1", "body": {}, "wait_millis": 45000,
        })),
    )
    .await;
    if result["status"] != "succeeded" {
        use std::io::Write;
        let mut log = tempfile::NamedTempFile::new().unwrap();
        log.write_all(result.to_string().as_bytes()).unwrap();
        let (_, diagnostic) = log.keep().unwrap();
        eprintln!(
            "private connection command diagnostics: {}",
            diagnostic.display()
        );
    }
    assert_eq!(
        result["status"], "succeeded",
        "managed connection control {command}"
    );
    result["body"].clone()
}

async fn simplex_peer(input: Value) -> Value {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../scripts/proofs/substrate-simplex-peer.mjs");
    let mut child = tokio::process::Command::new("node")
        .arg(script)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(150), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.status.success(),
        "SimpleX peer proof failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn local_substrate_context() -> String {
    let context = std::env::var("FC_TEST_SUBSTRATE_CONTEXT")
        .unwrap_or_else(|_| "kind-finite-hermes-spike".into());
    assert!(
        matches!(
            context.as_str(),
            "kind-finite-hermes-spike" | "kind-finite-hermes-restore"
        ),
        "fault injection requires an explicitly supported disposable kind cluster"
    );
    context
}

fn local_substrate_resource(kind: &str, name: &str) -> Value {
    let output = std::process::Command::new(required("FC_TEST_SUBSTRATE_ATE_CLI"))
        .args([
            "--kubeconfig",
            &required("FC_TEST_SUBSTRATE_CRASH_KUBECONFIG"),
            "--context",
            &local_substrate_context(),
            "get",
            kind,
            name,
            "-a",
            "finite-hermes-spike",
            "-o",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "read disposable provider resource");
    serde_json::from_slice(&output.stdout).unwrap()
}

// Explicit opt-in fault injection for the disposable kind cluster only.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires disposable Substrate, local TLS ingress, canonical image and a test inference key"]
async fn substrate_two_owner_launch_and_native_access() {
    with_isolated_postgres(|db| async move {
        let sites = sites::Sites::start().await;
        let image = required("FC_TEST_SUBSTRATE_IMAGE_DIGEST");
        let public_origin = required("FC_TEST_SUBSTRATE_PUBLIC_ORIGIN");
        let runtime_core_origin = required("FC_TEST_SUBSTRATE_CORE_ORIGIN");
        let runner_binary = required("FC_TEST_SUBSTRATE_RUNNER_BINARY");
        let inference_key =
            std::fs::read_to_string(required("FC_TEST_SUBSTRATE_INFERENCE_KEY_FILE")).unwrap();
        let runner_token = crate::store::runtime_credentials::new_secret().unwrap();
        let predecessor_token = crate::store::runtime_credentials::new_secret().unwrap();
        let source_host = "substrate-proof";
        let runner_id = "substrate-proof";
        let upgrade_image = std::env::var("FC_TEST_SUBSTRATE_UPGRADE_IMAGE_DIGEST").ok();
        let failed_image = std::env::var("FC_TEST_SUBSTRATE_FAILED_IMAGE_DIGEST").ok();
        assert!(failed_image.is_none() || upgrade_image.is_some());
        if upgrade_image.is_some() || std::env::var_os("FC_TEST_SUBSTRATE_AUTO_RECOVERY").is_some() {
            required("FC_TEST_SUBSTRATE_ATE_CLI");
            required("FC_TEST_SUBSTRATE_CRASH_KUBECONFIG");
        }
        assert!(std::env::var_os("FC_TEST_SUBSTRATE_INFLIGHT_CRASH").is_none()
            || std::env::var_os("FC_TEST_SUBSTRATE_AUTO_RECOVERY").is_some(), "in-flight fault requires automatic recovery");
        let mut artifacts = Vec::new();
        if let Some(image) = &upgrade_image { artifacts.push(("substrate-upgrade-proof", image.clone())); }
        if let Some(image) = &failed_image { artifacts.push(("substrate-failed-proof", image.clone())); }
        let register_artifact = |id: &str, image: String| db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
            id: id.into(),
            kind: RuntimeArtifactKind::OciImage,
            reference: image,
            version_label: "substrate-proof".into(),
            source_git_sha: None,
            finitec_version: None,
            hermes_source_ref: None,
            finite_platform_plugin_ref: None,
            state_schema_version: "state-v1".into(),
            base_image: None,
            canary_runtime_id: None,
            recover_known_good_chat: false,
            promoted: true,
            now: None,
        });
        register_artifact("substrate-proof", image).await.unwrap();
        let mut environment = BTreeMap::from([
            ("FINITE_DESKTOP_ENABLED".into(), "1".into()),
            ("FINITE_SUBSTRATE_ENV_PROOF".into(), "initial".into()),
            ("FINITE_SUBSTRATE_ENV_REMOVAL".into(), "present".into()),
        ]);
        if std::env::var_os("FC_TEST_SUBSTRATE_REQUEST_TRACE").is_some() {
            environment.insert("HERMES_DUMP_REQUESTS".into(), "1".into());
        }
        let store = db.store.clone().with_runtime_environment(environment.clone()).unwrap();
        let origins =
            HostedHermesOrigins::from_json(&json!({source_host: public_origin}).to_string())
                .unwrap();
        let auth = core_auth_with_runner_credentials(
            "proof-service",
            vec![runner_credential_config(
                "proof",
                &runner_token,
                runner_id,
                &[crate::RunnerClass::Substrate],
                source_host,
                false,
            ), runner_credential_config("predecessor", &predecessor_token, "predecessor",
                &[crate::RunnerClass::LocalDocker], source_host, false)],
            "proof-usage",
        );
        let app = router_with_runtime_upgrades_and_agent_creation_placement(
            store.clone(),
            auth,
            upgrade_image.is_some(),
            false,
            Some(RuntimePlacement {
                runner_class: crate::RunnerClass::Substrate,
                // Two real reservations fit the disposable 16-GiB local node.
                runtime_resource_class: RuntimeResourceClass::Vcpu2Memory4Gib,
            }),
            origins.clone(),
        );
        let account_listener = tokio::net::TcpListener::bind("127.0.0.1:18420")
            .await
            .unwrap();
        let runtime_listener = tokio::net::TcpListener::bind("127.0.0.1:18422")
            .await
            .unwrap();
        let chat_listener = tokio::net::TcpListener::bind("127.0.0.1:18426")
            .await
            .unwrap();
        let chat_server = tokio::spawn(
            axum::serve(
                chat_listener,
                finitechat_server::http_router(
                    finitechat_server::HttpServerState::new().with_require_signed_requests(true),
                )
                .into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .into_future(),
        );
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_token = crate::store::runtime_credentials::new_secret().unwrap();
        let identity_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let identity_origin = format!("http://{}", identity_listener.local_addr().unwrap());
        let identity_server = tokio::spawn(
            axum::serve(
                identity_listener,
                finite_identity::authority::router(
                    finite_identity::authority::AuthorityState::new(
                        finite_identity::authority::IdentityStore::open(
                            identity_dir.path().join("identity.db"),
                        )
                        .unwrap(),
                        std::sync::Arc::new(finite_identity::authority::DevMailer),
                        finite_identity::authority::SystemClock,
                        finite_identity::authority::AuthorityConfig {
                            external_base_url: identity_origin.clone(),
                            finite_vip_domain: "finite.vip".into(),
                            email_challenge_ttl_seconds: 900,
                            operator_token: Some(identity_token.clone()),
                        },
                    ),
                ),
            )
            .into_future(),
        );
        // A test-only transport barrier can kill Runner after provider success
        // but before Core sees completion; production has no fault-injection API.
        let interrupt_completion = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let completion_arrived = std::sync::Arc::new(tokio::sync::Notify::new());
        let release_completion = std::sync::Arc::new(tokio::sync::Notify::new());
        let account_app = app.clone().layer(axum::middleware::from_fn({
            let armed = interrupt_completion.clone();
            let arrived = completion_arrived.clone();
            let release = release_completion.clone();
            move |request: Request<Body>, next: axum::middleware::Next| {
                let (armed, arrived, release) = (armed.clone(), arrived.clone(), release.clone());
                async move {
                    if request.uri().path().starts_with("/api/core/v1/runtime-control-requests/")
                        && request.uri().path().ends_with("/complete")
                        && armed.swap(false, std::sync::atomic::Ordering::SeqCst)
                    {
                        arrived.notify_one();
                        release.notified().await;
                        return axum::response::IntoResponse::into_response(StatusCode::SERVICE_UNAVAILABLE);
                    }
                    next.run(request).await
                }
            }
        }));
        let account_server = tokio::spawn(axum::serve(account_listener, account_app).into_future());
        let stalled_boot = stalled_boot::Fault::new();
        let mut runtime_server = tokio::spawn(
            axum::serve(runtime_listener, stalled_boot.layer(store, origins.clone(), &environment)).into_future(),
        );
        let codes = db
            .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "disposable Substrate qualification".into(),
                code_count: 2,
                expires_in_hours: Some(24),
                hosting_tier: None,
                created_by_workos_user_id: "proof-operator".into(),
                now: None,
            })
            .await
            .unwrap();
        let runner_command = || {
            let mut command = std::process::Command::new(&runner_binary);
            command
                .arg("run-once")
                .env("FC_CORE_URL", "http://127.0.0.1:18420")
                .env("FC_CORE_RUNNER_API_TOKEN", &runner_token)
                .env("FC_RUNNER_ID", runner_id)
                .env("FC_RUNNER_SOURCE_HOST_ID", source_host)
                .env("FC_RUNNER_CLASS", "substrate")
                .env("FINITE_IDENTITY_AUTHORITY", &identity_origin)
                .env("FINITE_IDENTITY_OPERATOR_TOKEN", &identity_token)
                .env(
                    "FC_RUNNER_FINITECHAT_SERVER_URL",
                    required("FC_TEST_SUBSTRATE_CHAT_ORIGIN"),
                )
                .env("FC_RUNNER_RUNTIME_ARTIFACT_ID", "substrate-proof")
                .env("FC_RUNNER_RUNTIME_CORE_URL", &runtime_core_origin)
                .env(
                    "FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE",
                    inference_key.trim(),
                )
                .env("FC_RUNNER_HEALTH_REPORT_INTERVAL_SECS", "5");
            command
        };
        let device_dir = tempfile::tempdir().unwrap();
        let device = finitechat_hosted_device::app(finitechat_hosted_device::HostedDeviceConfig {
            data_root: device_dir.path().to_owned(),
            server_url: "http://127.0.0.1:18426".into(),
            public_url: required("FC_TEST_SUBSTRATE_CHAT_ORIGIN"),
            api_token: "disposable-control-proof".into(),
        });
        let dashboard_proof = std::env::var_os("FC_TEST_SUBSTRATE_DASHBOARD").is_some();
        let device_server = if dashboard_proof {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:18428").await.unwrap();
            Some(tokio::spawn(axum::serve(listener, device.clone()).into_future()))
        } else { None };
        let mut bindings = Vec::new();
        let mut projects = Vec::new();
        let mut runtimes = Vec::new();
        let mut owners = Vec::new();
        for (index, code) in codes.codes.iter().enumerate() {
            let user = format!("substrate-proof-{index}");
            let owner =
                access_token_with_subject(&user, &format!("{user}@finite.test"), true, None);
            let request = if dashboard_proof {
                dashboard_creation(&owner, &user, &code.code).await
            } else {
            let identity = device_request(&device, &user, "/v1/app/state", None).await;
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/core/v1/me/agent-creation-requests")
                        .header("authorization", format!("Bearer {owner}"))
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({"displayName": user, "launchCode": code.code,
                    "idempotencyKey": user, "ownerChatAccountId": identity["identity"]["account_id"]})
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let created: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            created["request"]["id"].as_str().unwrap().to_owned()
            };
            predecessor::idle(runner_command(), &predecessor_token).await;
            let stalled_identity = if index == 0 {
                stalled_boot.interrupt(&db, &request, runner_command()).await
            } else { None };
            let capacity_wait = if index == 1 {
                capacity::exhaust(&db, &request, runner_command()).await
            } else { None };
            // Golden-template workers are released asynchronously. Exercise the
            // normal runner polling contract while Core retains one creation.
            let mut attempts = 0;
            let (output, creation) = loop {
                let mut command = runner_command();
                let output = tokio::task::spawn_blocking(move || command.output().unwrap())
                    .await
                    .unwrap();
                let creation = db.agent_creation_request(&request).await.unwrap();
                attempts += 1;
                if matches!(
                    creation.status,
                    crate::AgentCreationRequestStatus::Running
                        | crate::AgentCreationRequestStatus::Failed
                ) || attempts == 30
                  || (capacity_wait.is_some() && std::env::var_os("FC_TEST_SUBSTRATE_CREATION_CRASH").is_some())
                {
                    break (output, creation);
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            };
            // Private tempfile retains diagnostics without echoing credentials.
            if !output.status.success()
                || creation.status != crate::AgentCreationRequestStatus::Running
            {
                use std::io::Write;
                let mut log = tempfile::NamedTempFile::new().unwrap();
                log.write_all(&output.stdout).unwrap();
                log.write_all(&output.stderr).unwrap();
                let (_, path) = log.keep().unwrap();
                eprintln!("private runner diagnostics: {}", path.display());
            }
            assert!(
                output.status.success(),
                "runner failed; inspect the isolated creation journal"
            );
            assert_eq!(serde_json::to_value(creation.status).unwrap(), "running");
            assert_eq!(creation.desired_runtime_artifact_id.as_deref(), Some("substrate-proof"), "proof must start on the original image");
            if let Some(wait) = capacity_wait { wait.verify(&db, &request).await; }
            if let Some(before) = stalled_identity {
                assert_eq!(before, capacity::identity(&db, &request).await, "stalled launch changed durable identity");
                eprintln!("stalled bootstrap recovered under the same creation identity");
            }
            let runtime = creation.agent_runtime_id.unwrap();
            eprintln!("qualified launch for synthetic owner {index}: {runtime}");
            device_request(&device, &user, "/v1/app/actions", Some(json!({"StartRuntime": null}))).await;
            device_request(&device, &user, "/v1/app/agent-bindings/authorize-bootstrap", Some(json!({
                "project_id": creation.project_id, "creation_request_id": request,
            }))).await;
            let contact: Value = reqwest::Client::new()
                .get(format!("{}/runtimes/{runtime}/contact", required("FC_SUBSTRATE_RUNTIME_ORIGIN")))
                .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
            let bound = device_request(&device, &user, "/v1/app/agent-bindings/ensure", Some(json!({
                "project_id": creation.project_id, "agent_npub": contact["agent_npub"],
                "display_name": format!("Chat with {user}"),
            }))).await;
            bindings.push(bound["hosted_agent_binding"].clone());
            projects.push(creation.project_id);
            runtimes.push(runtime);
            owners.push(owner);
        }
        // Publish targets only after both agents have launched the original
        // image. Otherwise latest-promoted admission would start on the target.
        for (id, image) in artifacts { register_artifact(id, image).await.unwrap(); }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        // Launch/agentd readiness precedes native credential installation. Both
        // targets must be chat-ready before cross-agent authentication probes.
        for (runtime, owner) in runtimes.iter().zip(&owners) {
            native_grant(
                &app,
                &format!("/api/core/v1/me/runtimes/{runtime}/hosted-hermes-session"),
                owner,
            )
            .await;
        }
        let mut histories = vec![Value::Null; 2];
        let prove_simplex = std::env::var_os("FC_TEST_SUBSTRATE_SIMPLEX").is_some();
        let mut simplex_addresses = [String::new(), String::new()];
        let peer_home = tempfile::tempdir().unwrap();
        let mut peers = Vec::new();
        let mut peer_ports = Vec::new();
        let mut peer_contacts = [Value::Null, Value::Null];
        if prove_simplex {
            for index in 0..2 {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let port = listener.local_addr().unwrap().port();
                drop(listener);
                peers.push(tokio::process::Command::new(required("FC_TEST_SUBSTRATE_SIMPLEX_BIN"))
                    .args(["-d", peer_home.path().join(format!("peer{index}")).to_str().unwrap(), "-p", &port.to_string(), "--mute", "--user-display-name", &format!("FiniteProof{index}")])
                    .kill_on_drop(true).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
                    .spawn().unwrap());
                peer_ports.push(port);
            }
        }
        for pass in 0..(2 + usize::from(upgrade_image.is_some()) + usize::from(failed_image.is_some())) {
            if pass == 1 {
                recovery::wait_for_cluster_restore(&db.url, &runtimes).await;
                // Replace Core's boot configuration authority, leaving the
                // installed provider templates and creation specs untouched.
                runtime_server.abort();
                let _ = (&mut runtime_server).await;
                environment.insert("FINITE_SUBSTRATE_ENV_PROOF".into(), "updated".into());
                environment.remove("FINITE_SUBSTRATE_ENV_REMOVAL");
                let refreshed = db.store.clone().with_runtime_environment(environment.clone()).unwrap();
                let listener = tokio::net::TcpListener::bind("127.0.0.1:18422").await.unwrap();
                runtime_server = tokio::spawn(
                    axum::serve(listener, runtime_router(refreshed, origins.clone())).into_future(),
                );
            }
            for (index, runtime) in runtimes.iter().enumerate() {
                if pass >= 2 {
                    let actor_name = runtime.replacen("runtime_", "runtime-", 1);
                    let before = local_substrate_resource("actor", &actor_name);
                    let prior_runtime = db.agent_runtime(runtime).await.unwrap();
                    let operator = access_token_with_subject("proof-operator", "proof-operator@finite.vip", true,
                        Some(crate::auth::test_support::OPERATOR_ORG_ID));
                    let target = if pass == 2 { "substrate-upgrade-proof" } else { "substrate-failed-proof" };
                    let response = app.clone().oneshot(Request::builder().method("POST")
                        .uri(format!("/api/core/v1/admin/projects/{}/runtime/upgrade", projects[index]))
                        .header("authorization", format!("Bearer {operator}"))
                        .header("content-type", "application/json")
                        .body(Body::from(json!({"targetRuntimeArtifactId": target}).to_string())).unwrap()).await.unwrap();
                    assert_eq!(response.status(), StatusCode::OK, "enqueue Substrate image upgrade");
                    let control: Value = serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
                    if pass == 2 && index == 0 && std::env::var_os("FC_TEST_SUBSTRATE_INTERRUPT_UPGRADE").is_some() {
                        interrupt_completion.store(true, std::sync::atomic::Ordering::SeqCst);
                        let log = tempfile::NamedTempFile::new().unwrap();
                        let mut command = tokio::process::Command::from(runner_command());
                        let mut child = command.env("FC_RUNNER_LEASE_SECONDS", "60").kill_on_drop(true)
                            .stdout(log.as_file().try_clone().unwrap()).stderr(log.as_file().try_clone().unwrap())
                            .spawn().unwrap();
                        let (_, path) = log.keep().unwrap();
                        eprintln!("private interrupted Runner diagnostics: {}", path.display());
                        tokio::time::timeout(Duration::from_secs(120), completion_arrived.notified()).await
                            .expect("Runner must reach completion after real provider upgrade");
                        child.kill().await.unwrap();
                        assert!(!child.wait().await.unwrap().success());
                        release_completion.notify_one();
                        let pending = db.runtime_control_request(control["id"].as_str().unwrap()).await.unwrap();
                        assert_eq!(pending.status, crate::RuntimeControlRequestStatus::Launching);
                        assert_eq!(db.agent_runtime(runtime).await.unwrap().runtime_artifact_id, prior_runtime.runtime_artifact_id);
                        let interrupted = local_substrate_resource("actor", &actor_name);
                        assert_eq!(interrupted["metadata"]["uid"], before["metadata"]["uid"]);
                        assert_eq!(interrupted["status"]["actorVolumes"], before["status"]["actorVolumes"]);
                        assert_eq!(interrupted["status"]["state"], "ACTOR_STATE_RUNNING");
                        let installed = local_substrate_resource("actor-template", interrupted["actorTemplate"]["name"].as_str().unwrap());
                        assert_eq!(installed["containers"][0]["image"].as_str(), upgrade_image.as_deref());
                        let expires = time::OffsetDateTime::parse(pending.lease_expires_at.as_deref().unwrap(),
                            &time::format_description::well_known::Rfc3339).unwrap();
                        let remaining = (expires - time::OffsetDateTime::now_utc()).whole_milliseconds().max(0) as u64;
                        tokio::time::sleep(Duration::from_millis(remaining + 100)).await;
                        eprintln!("Runner killed before completion; reclaiming the same expired upgrade request");
                    }
                    let mut command = runner_command();
                    let output = tokio::task::spawn_blocking(move || command.output().unwrap()).await.unwrap();
                    if !output.status.success() {
                        use std::io::Write;
                        let mut log = tempfile::NamedTempFile::new().unwrap();
                        log.write_all(&output.stdout).unwrap();
                        log.write_all(&output.stderr).unwrap();
                        let (_, path) = log.keep().unwrap();
                        eprintln!("private upgrade runner diagnostics: {}", path.display());
                    }
                    assert!(output.status.success(), "upgrade runner process failed");
                    let request = db.runtime_control_request(control["id"].as_str().unwrap()).await.unwrap();
                    assert_eq!(request.status, if pass == 2 { crate::RuntimeControlRequestStatus::Succeeded } else { crate::RuntimeControlRequestStatus::Failed },
                        "image upgrade outcome: {:?}", request.failure_message);
                    let committed_runtime = db.agent_runtime(runtime).await.unwrap();
                    assert_eq!(committed_runtime.runtime_artifact_id.as_deref(), Some("substrate-upgrade-proof"),
                        "Core must retain the healthy artifact after a rejected upgrade");
                    assert_eq!(committed_runtime.contact_endpoint, prior_runtime.contact_endpoint);
                    assert_eq!(committed_runtime.host_facts.runtime_host, prior_runtime.host_facts.runtime_host);
                    let after = local_substrate_resource("actor", &actor_name);
                    assert_eq!(after["metadata"]["uid"], before["metadata"]["uid"]);
                    assert_eq!(after["status"]["actorVolumes"], before["status"]["actorVolumes"]);
                    assert_eq!(after["status"]["state"], "ACTOR_STATE_RUNNING");
                    let template = local_substrate_resource("actor-template", after["actorTemplate"]["name"].as_str().unwrap());
                    assert_eq!(template["containers"][0]["image"].as_str(), upgrade_image.as_deref());
                    assert_eq!(after["status"]["currentActorTemplateUid"], template["metadata"]["uid"]);
                }
                if pass == 1 {
                    let automatic = std::env::var_os("FC_TEST_SUBSTRATE_AUTO_RECOVERY").is_some();
                    if index == 0 {
                        if automatic {
                            assert!(db.request_substrate_recovery("other-host", runtime).await.unwrap().is_none());
                        }
                        let before = automatic.then(|| local_substrate_resource("actor", &runtime.replace('_', "-")));
                        crash_local_worker(runtime).await;
                        if let Some(before) = before {
                            let mut command = runner_command();
                            command.env("FC_RUNNER_DRAIN", "true");
                            let output = tokio::task::spawn_blocking(move || command.output().unwrap()).await.unwrap();
                            assert!(output.status.success(), "automatic recovery runner failed");
                            let after = local_substrate_resource("actor", &runtime.replace('_', "-"));
                            assert_eq!(after["metadata"]["uid"], before["metadata"]["uid"]);
                            assert_eq!(after["status"]["actorVolumes"], before["status"]["actorVolumes"]);
                            assert_eq!(after["status"]["state"], "ACTOR_STATE_RUNNING", "a normal Runner cycle must recover worker loss without an owner restart request");
                            let automatic_requests = db.query_json("SELECT jsonb_build_object('kind',kind,'status',status,'user',requested_by_user_id) FROM runtime_control_requests WHERE agent_runtime_id=$1 AND requested_by_user_id IS NULL", &[runtime]).await;
                            assert_eq!(automatic_requests, vec![serde_json::json!({"kind":"restart", "status":"succeeded", "user":null})]);
                            // Simulate a delayed crash observation after recovery.
                            let duplicate = db.request_substrate_recovery(source_host, runtime).await.unwrap().unwrap();
                            assert!(duplicate.requested_by_user_id.is_none());
                            predecessor::idle(runner_command(), &predecessor_token).await;
                            assert!(db.request_substrate_recovery(source_host, runtime).await.unwrap().is_none());
                            let mut command = runner_command();
                            let output = tokio::task::spawn_blocking(move || command.output().unwrap()).await.unwrap();
                            assert!(output.status.success());
                            let unchanged = local_substrate_resource("actor", &runtime.replace('_', "-"));
                            assert_eq!(unchanged["metadata"]["version"], after["metadata"]["version"], "a delayed observation must not restart healthy compute");
                        }
                    }
                    let actions: &[&str] = if index == 0 && automatic && std::env::var_os("FC_TEST_SUBSTRATE_INFLIGHT_CRASH").is_some() {
                        &[] // Prove native readiness after automatic recovery alone.
                    } else if index == 1 || automatic {
                        &["stop", "restart"]
                    } else {
                        // Recovery must remain stoppable after a lost worker.
                        &["restart", "stop", "restart"]
                    };
                    for action in actions {
                        owners[index] = owner_session(index);
                        let response = app
                            .clone()
                            .oneshot(
                                Request::builder()
                                    .method("POST")
                                    .uri(format!(
                                        "/api/core/v1/me/projects/{}/runtime/{action}",
                                        projects[index]
                                    ))
                                    .header("authorization", format!("Bearer {}", owners[index]))
                                    .header("content-type", "application/json")
                                    .body(Body::from("{}"))
                                    .unwrap(),
                            )
                            .await
                            .unwrap();
                        assert_eq!(response.status(), StatusCode::OK);
                        let control: Value = serde_json::from_slice(
                            &to_bytes(response.into_body(), 65536).await.unwrap(),
                        )
                        .unwrap();
                        if automatic {
                            assert!(db.request_substrate_recovery(source_host, runtime).await.unwrap().is_none(), "pending owner control must fence automatic recovery");
                        }
                        let mut command = runner_command();
                        let output = tokio::task::spawn_blocking(move || command.output().unwrap())
                            .await
                            .unwrap();
                        assert!(output.status.success(), "runtime control runner failed");
                        let request = db
                            .runtime_control_request(control["id"].as_str().unwrap())
                            .await
                            .unwrap();
                        assert_eq!(
                            request.status,
                            if *action == "stop" {
                                crate::RuntimeControlRequestStatus::Stopped
                            } else {
                                crate::RuntimeControlRequestStatus::Succeeded
                            },
                            "{action} failed: {:?}",
                            request.failure_message
                        );
                        if *action == "stop" {
                            if automatic {
                                assert!(db.request_substrate_recovery(source_host, runtime).await.unwrap().is_none(), "offline lifecycle must reject automatic recovery");
                                let mut command = runner_command();
                                let output = tokio::task::spawn_blocking(move || command.output().unwrap()).await.unwrap();
                                assert!(output.status.success());
                                assert_eq!(local_substrate_resource("actor", &runtime.replace('_', "-"))["status"]["state"], "ACTOR_STATE_SUSPENDED", "automatic recovery must honor a completed stop");
                            }
                            for _ in 0..3 {
                                assert_eq!(
                                    http.get(format!(
                                        "{public_origin}/runtimes/{runtime}/api/auth/me"
                                    ))
                                    .send()
                                    .await
                                    .unwrap()
                                    .status(),
                                    StatusCode::SERVICE_UNAVAILABLE,
                                    "traffic must not resume an explicitly stopped agent"
                                );
                            }
                        }
                    }
                }

                let path = format!("/api/core/v1/me/runtimes/{runtime}/hosted-hermes-session");
                for command in ["agent.owner.claim", "agent.connections.status"] {
                    connection_command(&device, index, &bindings[index], command).await;
                }
                eprintln!("owner-authorized connection controls passed for synthetic owner {index}, restart pass {pass}");
                if prove_simplex {
                    if pass == 0 {
                        connection_command(&device, index, &bindings[index], "agent.simplex.connect").await;
                    }
                    let mut ready = false;
                    for _ in 0..30 {
                        let status = connection_command(&device, index, &bindings[index], "agent.connections.status").await;
                        let simplex = &status["simplex"];
                        if simplex["enabled"] == true && simplex["ready"] == true {
                            let address = simplex["address"].as_str().expect("managed SimpleX address");
                            assert!(!address.is_empty());
                            if pass == 0 {
                                simplex_addresses[index] = address.to_owned();
                            } else {
                                assert!(simplex_addresses[index] == address, "SimpleX identity changed across restart");
                            }
                            ready = true;
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    assert!(ready, "managed SimpleX did not become ready");
                    eprintln!("managed SimpleX identity ready for synthetic owner {index}, restart pass {pass}");
                    if pass == 0 {
                        let paired = simplex_peer(json!({"port": peer_ports[index], "address": simplex_addresses[index], "mode": "pair"})).await;
                        peer_contacts[index] = paired["contactId"].clone();
                        // Relay connection and gateway pairing are asynchronous.
                        // Wait for the request; never choose among multiple contacts.
                        let pending = tokio::time::timeout(Duration::from_secs(30), async {
                            for _ in 0..30 {
                                let status = connection_command(&device, index, &bindings[index], "agent.connections.status").await;
                                let pending = status["simplex"]["pending"].as_array().expect("pending SimpleX requests");
                                if !pending.is_empty() {
                                    assert_eq!(pending.len(), 1, "only the disposable contact may be approved");
                                    return pending.clone();
                                }
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                            panic!("disposable SimpleX pairing request did not arrive");
                        }).await.expect("SimpleX pairing request deadline");
                        let result = device_request(&device, &format!("substrate-proof-{index}"), "/v1/app/runtime-commands", Some(json!({
                            "room_id": bindings[index]["canonical_room_id"], "target_account_id": bindings[index]["agent_account_id"],
                            "command": "agent.simplex.approve_request", "resource_key": "agent.connections",
                            "schema": "finite.agent.simplex.approve-request.v1", "body": {"request_id": pending[0]["request_id"]}, "wait_millis": 45000,
                        }))).await;
                        assert_eq!(result["status"], "succeeded", "owner approves exact pending SimpleX request");
                    }
                    let reply = simplex_peer(json!({"port": peer_ports[index], "contactId": peer_contacts[index], "mode": "message"})).await;
                    assert_eq!(reply["replyVerified"], true);
                    eprintln!("approved SimpleX inference reply passed for synthetic owner {index}, restart pass {pass}");
                }
                if pass >= 2 {
                    let readiness = db.query_json("SELECT jsonb_build_object('activated',c.activated,'revoked',c.revoked,'enabled',c.hosted_enabled,'generation',c.hosted_generation,'applied_generation',c.hosted_applied_generation,'apply_status',c.hosted_apply_status,'creation_status',q.status) FROM runtime_core_credentials c JOIN agent_creation_requests q ON q.id=c.creation_request_id WHERE c.agent_runtime_id=$1", &[runtime]).await;
                    eprintln!("native readiness before grant, owner {index}, pass {pass}: {readiness:?}");
                }
                // Account sessions expire independently of native sessions. Long
                // fault injection must renew the same owners, as the dashboard does.
                for (index, owner) in owners.iter_mut().enumerate() {
                    *owner = owner_session(index);
                }
                let grant = native_grant(&app, &path, &owners[index]).await;
                let base = grant["baseUrl"].as_str().unwrap();
                assert_eq!(
                    http.get(format!("{base}api/auth/me"))
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::UNAUTHORIZED
                );
                let denied = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri(&path)
                            .header("authorization", format!("Bearer {}", owners[1 - index]))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(denied.status(), StatusCode::NOT_FOUND);
                let mut native_grant = grant;
                // Match dashboard requests: each native HTTP operation obtains
                // a fresh grant; model/tool work can exceed the 60-second TTL.
                native_grant["grantEndpoint"] = json!(format!("http://127.0.0.1:18420{path}"));
                native_grant["ownerToken"] = json!(owners[index]);
                native_grant["previous"] = histories[index].clone();
                native_grant["environmentValue"] = json!(if pass == 0 { "initial" } else { "updated" });
                native_grant["proveInterrupt"] = Value::Bool(index == 0 && pass == 0);
                native_grant["proveDesktop"] = Value::Bool(index == 1 && pass == 0);
                native_grant["prepareCrash"] = Value::Bool(index == 0 && pass == 0 && std::env::var_os("FC_TEST_SUBSTRATE_INFLIGHT_CRASH").is_some());
                native_grant["proveBrowser"] = Value::Bool(index == 0 && pass == 0 && std::env::var_os("FC_TEST_SUBSTRATE_BROWSER").is_some());
                if pass >= 2 {
                    native_grant["upgradeMarker"] = json!(required("FC_TEST_SUBSTRATE_UPGRADE_MARKER"));
                }
                if pass == 0 && let Some(sites) = &sites {
                    native_grant["sitesProof"] = sites.grant(&bindings[index], index).await;
                }
                let native_grant = native_grant.to_string();
                let chat = tokio::task::spawn_blocking(move || {
                    use std::io::Write;
                    let mut child = std::process::Command::new("node")
                        .arg(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../../../scripts/proofs/substrate-native-chat.mjs"
                        ))
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                        .unwrap();
                    child
                        .stdin
                        .take()
                        .unwrap()
                        .write_all(native_grant.as_bytes())
                        .unwrap();
                    child.wait_with_output().unwrap()
                })
                .await
                .unwrap();
                assert!(
                    chat.status.success(),
                    "native model turn failed: {}",
                    String::from_utf8_lossy(&chat.stderr)
                );
                histories[index] = serde_json::from_slice(&chat.stdout).unwrap();
                if index == 0 && pass == 0 {
                    if std::env::var_os("FC_TEST_SUBSTRATE_BROWSER").is_some() {
                        assert_eq!(histories[index]["browserVerified"], true);
                        eprintln!("real native browser turn and reload history passed");
                    }
                    if std::env::var_os("FC_TEST_SUBSTRATE_IMAGES").is_some() {
                        assert_eq!(histories[index]["imageVerified"], true);
                        eprintln!("turn-scoped native image inference and plain-text follow-up passed");
                    }
                    assert_eq!(histories[index]["interruptionVerified"], true);
                    assert_eq!(histories[index]["clarificationVerified"], true);
                    assert_eq!(histories[index]["approvalVerified"], true);
                    assert_eq!(histories[index]["reconnectVerified"], true);
                    eprintln!("native provider reconnect, interruption, clarification, approval, and subsequent turn passed");
                }
                if index == 1 {
                    assert!(histories[index]["desktop"]["sha256"].as_str().is_some());
                }
                if let Some(path) = histories[index]["desktop"]["artifact"].as_str() {
                    eprintln!("desktop screenshot artifact: {path}");
                }
                eprintln!(
                    "native model turn passed for synthetic owner {index}, restart pass {pass}"
                );
            }
            // A recovery drill restores both actors suspended. Check cross-agent
            // authentication only after both have passed their owner restart.
            for (index, runtime) in runtimes.iter().enumerate() {
                let owner = owner_session(index);
                let path = format!("/api/core/v1/me/runtimes/{runtime}/hosted-hermes-session");
                let grant = native_grant(&app, &path, &owner).await;
                let base = grant["baseUrl"].as_str().unwrap();
                let token = grant["accessToken"].as_str().unwrap();
                // Alternate identities repeatedly: a single successful pair missed
                // pooled CONNECT filter-state leakage in upstream Envoy.
                for _ in 0..12 {
                    let authenticated = http
                        .get(format!("{base}api/auth/me"))
                        .bearer_auth(token)
                        .send()
                        .await
                        .unwrap();
                    let authenticated_status = authenticated.status();
                    if authenticated_status != StatusCode::OK {
                        let reason = authenticated.json::<Value>().await.ok().and_then(|body| {
                            body.get("reason")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        });
                        panic!("native grant rejected: {authenticated_status}, reason: {reason:?}");
                    }
                    let other = format!(
                        "{public_origin}/runtimes/{}/api/auth/me",
                        runtimes[1 - index]
                    );
                    assert_eq!(
                        http.get(&other)
                            .bearer_auth(token)
                            .send()
                            .await
                            .unwrap()
                            .status(),
                        StatusCode::UNAUTHORIZED
                    );
                }
            }
            eprintln!("bidirectional native owner isolation passed, restart pass {pass}");
        }
        if let Some(server) = device_server { server.abort(); }
        identity_server.abort();
        chat_server.abort();
        account_server.abort();
        runtime_server.abort();
        // Actors and CSI volumes intentionally remain for recovery inspection;
        // deleting an actor would also destroy its durable volume upstream.
    })
    .await;
}
