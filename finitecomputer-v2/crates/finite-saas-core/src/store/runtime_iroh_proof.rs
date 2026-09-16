// Explicit external acceptance gate: real Core/Postgres, production agentd
// subprocess, native Iroh over HTTPS relay, and an unmodified Hermes binary.
use super::*;
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl,
    endpoint::{Connection, RecvStream, SendStream, presets},
};
use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::AsyncReadExt;

async fn header(recv: &mut RecvStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(
            tokio::time::timeout(Duration::from_secs(10), recv.read_u8())
                .await
                .unwrap()
                .unwrap(),
        );
        assert!(bytes.len() <= 16384);
    }
    bytes
}
fn admission_closed(error: iroh::endpoint::ConnectionError) {
    match error {
        iroh::endpoint::ConnectionError::ApplicationClosed(close) => {
            assert_eq!(close.error_code, 403u32.into());
            assert_eq!(close.reason.as_ref(), b"admission ended");
        }
        other => panic!("expected explicit admission revocation, got {other:?}"),
    }
}
async fn tunnel(connection: &Connection, port: u16) -> (SendStream, RecvStream, Vec<u8>) {
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    send.write_all(
        format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let response = header(&mut recv).await;
    (send, recv, response)
}

#[tokio::test]
#[ignore = "requires FINITE_TEST_AGENTD_BIN, FINITE_TEST_HERMES_BIN and HTTPS relay network; run explicitly"]
async fn real_core_agentd_iroh_hermes_acceptance() {
    let daemon_bin =
        std::env::var("FINITE_TEST_AGENTD_BIN").expect("real built agentd binary required");
    let hermes_bin =
        std::env::var("FINITE_TEST_HERMES_BIN").expect("real Hermes serve binary required");
    let relay: RelayUrl = std::env::var("FINITE_TEST_IROH_RELAY_URL")
        .unwrap_or_else(|_| "https://use1-1.relay.n0.iroh.link./".into())
        .parse()
        .unwrap();
    with_isolated_postgres(|db| prove(db, daemon_bin, hermes_bin, relay)).await;
}

async fn prove(db: TestDb, daemon_bin: String, hermes_bin: String, relay: RelayUrl) {
    let request = requested(&db).await;
    let secret = db
        .provision_runtime_credential(provision(&request))
        .await
        .unwrap()
        .secret;
    register(&db, &request).await;
    let launch = complete(&db, &request).await.unwrap();
    let project = launch.project.id;
    let auth = crate::auth::test_support::core_auth("service", "runner", "usage");
    let polls = Arc::new(AtomicUsize::new(0));
    let observed = polls.clone();
    let app = crate::api::router(db.store.clone(), auth).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let observed = observed.clone();
            async move {
                let admission = request.uri().path().ends_with("/iroh-admissions");
                let response = next.run(request).await;
                if admission && response.status().is_success() {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
                response
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let core_url = format!("http://{}", listener.local_addr().unwrap());
    let (stop_core, stopped) = tokio::sync::oneshot::channel();
    let core = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap()
    });
    let home = tempfile::tempdir().unwrap();
    let skill = home.path().join("hermes/skills/finite-iroh-acceptance");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "---\nname: finite-iroh-acceptance\ndescription: Real agent-local skills acceptance fixture\n---\n# Acceptance skill\nRead-only browser proof.\n").unwrap();
    let mut hermes_command = tokio::process::Command::new(hermes_bin);
    hermes_command
        .args([
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            "8642",
            "--no-open",
        ])
        .env("HERMES_HOME", home.path().join("hermes"))
        .env_remove("HERMES_DASHBOARD_SESSION_TOKEN")
        .env("HERMES_TUI_WS_ORPHAN_REAP_GRACE_S", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut hermes = hermes_command.spawn().unwrap();
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            assert!(
                hermes.try_wait().unwrap().is_none(),
                "Hermes exited before readiness"
            );
            if http.get("http://127.0.0.1:8642/").send().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap();
    let mut daemon_command = tokio::process::Command::new(daemon_bin);
    daemon_command.arg("iroh")
        .env("FINITECHAT_HOME", home.path())
        .env("FINITE_CORE_URL", &core_url)
        .env("FINITE_CORE_CREDENTIAL", &secret)
        .env("FINITE_IROH_RELAY_URL", relay.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut daemon = daemon_command.spawn().unwrap();
    let binding = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            assert!(
                daemon.try_wait().unwrap().is_none(),
                "Iroh process exited before registration"
            );
            if let Ok(value) = db
                .runtime_hermes_endpoint(&project, "runtime-auth-user", false)
                .await
            {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    // Optional real dashboard/browser leg of this explicit external gate.
    // Only the existing test WorkOS key/user source is a fixture; Core HTTP,
    // admissions, Postgres, agentd, Hermes and browser WASM are production code.
    if let Ok(script) = std::env::var("FINITE_TEST_BROWSER_SCRIPT") {
        let account_token = crate::auth::test_support::access_token_with_subject(
            "runtime-auth-user", "runtime-auth@finite.test", true,
            Some(crate::auth::test_support::OPERATOR_ORG_ID),
        );
        let mut browser = tokio::process::Command::new("node")
            .args(["--import", "tsx", &script])
            .current_dir(std::env::var("FINITE_TEST_DASHBOARD_DIR").unwrap())
            .env("FC_CORE_BASE_URL", &core_url)
            .env("FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN", account_token)
            .env("FC_WORKOS_OPERATOR_ORG_ID", crate::auth::test_support::OPERATOR_ORG_ID)
            .env("FINITE_TEST_HERMES_RESTART_DIR", home.path())
            .env("FINITE_TEST_PROJECT_ID", &project)
            .env("FINITE_TEST_RUNTIME_ID", launch.request.agent_runtime_id.as_ref().unwrap())
            .kill_on_drop(true).spawn().unwrap();
        let result = tokio::time::timeout(Duration::from_secs(180), async {
            loop {
                tokio::select! {
                    result = browser.wait() => break result.unwrap(),
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        let request = home.path().join("restart-hermes");
                        if request.exists() {
                            std::fs::remove_file(request).unwrap();
                            hermes.kill().await.unwrap();
                            hermes.wait().await.unwrap();
                            hermes = hermes_command.spawn().unwrap();
                            // Readiness is rechecked by the browser through Iroh.
                            std::fs::write(home.path().join("hermes-restarted"), b"ok").unwrap();
                        }
                    }
                }
            }
        }).await.expect("browser acceptance timed out");
        assert!(result.success(), "dashboard/browser acceptance failed");
    }
    // Hermes owns its process token; native test clients discover it too.
    // The browser leg obtains this independently through its admitted tunnel.
    let bootstrap = http.get("http://127.0.0.1:8642/").send().await.unwrap()
        .text().await.unwrap();
    let assignment = bootstrap.split_once("window.__HERMES_SESSION_TOKEN__=")
        .expect("native bootstrap token assignment").1;
    let token: String = serde_json::from_str(assignment.split_once(';').unwrap().0).unwrap();
    let endpoint: EndpointId = binding["endpointId"].as_str().unwrap().parse().unwrap();
    let client = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::Custom(relay.clone().into()))
        .bind()
        .await
        .unwrap();
    let denied = client
        .connect(
            EndpointAddr::new(endpoint).with_relay_url(relay.clone()),
            b"iroh-http-proxy/1",
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), denied.closed())
        .await
        .expect("default-off must reject an unadmitted peer");
    let peer = crate::store::runtime_admissions::PeerAdmissionRequest {
        generation: binding["generation"].as_i64().unwrap(),
        endpoint_id: endpoint.to_string(),
        peer_id: client.id().to_string(),
    };
    assert!(
        db.admit_runtime_peer(&project, "runtime-auth-user", false, &peer)
            .await
            .is_err()
    );
    db.set_runtime_hosted_access(&project, &peer, true)
        .await
        .unwrap();
    db.admit_runtime_peer(&project, "runtime-auth-user", false, &peer)
        .await
        .unwrap();
    let before = polls.load(Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(15), async {
        while polls.load(Ordering::SeqCst) < before + 2 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let address = EndpointAddr::new(endpoint).with_relay_url(relay.clone());
    let connection = tokio::time::timeout(
        Duration::from_secs(30),
        client.connect(address.clone(), b"iroh-http-proxy/1"),
    )
    .await
    .unwrap()
    .unwrap();
    let (mut send, mut recv, response) = tunnel(&connection, 8642).await;
    assert!(response.starts_with(b"HTTP/1.1 200"));
    send.write_all(format!("GET /api/sessions?limit=1 HTTP/1.1\r\nHost: localhost\r\nX-Hermes-Session-Token: {token}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    assert!(header(&mut recv).await.starts_with(b"HTTP/1.1 200"));
    let (mut ws_send, mut ws_recv, response) = tunnel(&connection, 8642).await;
    assert!(response.starts_with(b"HTTP/1.1 200"));
    ws_send.write_all(format!("GET /api/ws?token={token} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").as_bytes()).await.unwrap();
    assert!(header(&mut ws_recv).await.starts_with(b"HTTP/1.1 101"));
    let (_, _, denied) = tunnel(&connection, 8643).await;
    assert!(denied.starts_with(b"HTTP/1.1 403"));
    let stranger = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::Custom(relay.clone().into()))
        .bind()
        .await
        .unwrap();
    let unadmitted = stranger
        .connect(address.clone(), b"iroh-http-proxy/1")
        .await
        .unwrap();
    match tokio::time::timeout(Duration::from_secs(10), unadmitted.closed())
        .await
        .unwrap()
    {
        iroh::endpoint::ConnectionError::ApplicationClosed(close) => {
            assert_eq!(close.error_code, 403u32.into())
        }
        other => panic!("unadmitted peer must be rejected by agent, got {other:?}"),
    }
    stranger.close().await;
    // Restart the actual networking process with the same Core credential and
    // durable ledger. Hermes remains the same live process. The new ephemeral
    // endpoint must preserve enablement but inherit no browser admissions.
    let hermes_pid = hermes.id();
    daemon.kill().await.unwrap();
    daemon.wait().await.unwrap();
    daemon = daemon_command.spawn().unwrap();
    let replacement = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            assert!(daemon.try_wait().unwrap().is_none());
            let current = db.runtime_hermes_endpoint(&project, "runtime-auth-user", false).await.unwrap();
            if current["generation"].as_i64().unwrap() > peer.generation {
                break current;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }).await.unwrap();
    assert_eq!(replacement["enabled"], true);
    assert_ne!(replacement["endpointId"], peer.endpoint_id);
    assert!(db.admit_runtime_peer(&project, "runtime-auth-user", false, &peer).await.is_err());
    assert!(db.set_runtime_hosted_access(&project, &peer, false).await.is_err());
    let peer = crate::store::runtime_admissions::PeerAdmissionRequest {
        generation: replacement["generation"].as_i64().unwrap(),
        endpoint_id: replacement["endpointId"].as_str().unwrap().into(),
        peer_id: client.id().to_string(),
    };
    let address = EndpointAddr::new(peer.endpoint_id.parse().unwrap()).with_relay_url(relay);
    let denied = client.connect(address.clone(), b"iroh-http-proxy/1").await.unwrap();
    match tokio::time::timeout(Duration::from_secs(10), denied.closed()).await.unwrap() {
        iroh::endpoint::ConnectionError::ApplicationClosed(close) => assert_eq!(close.error_code, 403u32.into()),
        other => panic!("old browser admission survived process restart: {other:?}"),
    }
    db.admit_runtime_peer(&project, "runtime-auth-user", false, &peer).await.unwrap();
    let before = polls.load(Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(15), async {
        while polls.load(Ordering::SeqCst) < before + 2 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }).await.unwrap();
    let connection = client.connect(address.clone(), b"iroh-http-proxy/1").await.unwrap();
    let (mut send, mut recv, response) = tunnel(&connection, 8642).await;
    assert!(response.starts_with(b"HTTP/1.1 200"));
    send.write_all(format!("GET /api/sessions?limit=1 HTTP/1.1\r\nHost: localhost\r\nX-Hermes-Session-Token: {token}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    assert!(header(&mut recv).await.starts_with(b"HTTP/1.1 200"));
    assert!(hermes.try_wait().unwrap().is_none());
    assert_eq!(hermes.id(), hermes_pid);
    let (mut ws_send, mut ws_recv, response) = tunnel(&connection, 8642).await;
    assert!(response.starts_with(b"HTTP/1.1 200"));
    ws_send.write_all(format!("GET /api/ws?token={token} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").as_bytes()).await.unwrap();
    assert!(header(&mut ws_recv).await.starts_with(b"HTTP/1.1 101"));
    db.remove_runtime_peer(&project, "runtime-auth-user", false, &peer)
        .await
        .unwrap();
    admission_closed(
        tokio::time::timeout(Duration::from_secs(12), connection.closed())
            .await
            .expect("revocation must close existing WebSocket transport"),
    );
    assert!(daemon.try_wait().unwrap().is_none());
    assert!(hermes.try_wait().unwrap().is_none());
    db.admit_runtime_peer(&project, "runtime-auth-user", false, &peer)
        .await
        .unwrap();
    db.connection().await.unwrap().execute("UPDATE runtime_peer_admissions SET expires_at=clock_timestamp()+INTERVAL '20 seconds' WHERE creation_request_id=$1", &[&request]).await.unwrap();
    let before = polls.load(Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(15), async {
        while polls.load(Ordering::SeqCst) < before + 2 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let connection = client.connect(address, b"iroh-http-proxy/1").await.unwrap();
    let (_send, _recv, response) = tunnel(&connection, 8642).await;
    assert!(response.starts_with(b"HTTP/1.1 200"));
    stop_core.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), core)
        .await
        .unwrap()
        .unwrap();
    let final_polls = polls.load(Ordering::SeqCst);
    admission_closed(
        tokio::time::timeout(Duration::from_secs(22), connection.closed())
            .await
            .expect("local deadline must close transport during Core outage"),
    );
    assert!(daemon.try_wait().unwrap().is_none());
    assert!(
        hermes.try_wait().unwrap().is_none(),
        "access expiry must not terminate Hermes"
    );
    assert_eq!(
        polls.load(Ordering::SeqCst),
        final_polls,
        "Core served admissions after shutdown"
    );
    client.close().await;
    for process in [&mut daemon, &mut hermes] {
        process.kill().await.unwrap();
        process.wait().await.unwrap();
    }
    writable_directories(home.path());
}

// Nix-packaged Hermes seeds read-only skill directories. Make only this
// synthetic test home's directories removable; never follow symlinks.
fn writable_directories(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            writable_directories(&entry.path());
        }
    }
}
