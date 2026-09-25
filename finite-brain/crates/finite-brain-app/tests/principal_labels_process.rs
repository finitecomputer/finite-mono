//! Real exporter entrypoint against retained source columns and private output.
#![cfg(unix)]
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use finite_brain_server::{ServerState, principal_labels::LabelProjection, router_with_state};
use finite_brain_store::BrainStore;
use finite_nostr::NostrPublicKey;
use finite_nostr::{
    HttpAuthEventRequest, decode_http_auth_header, sign_http_auth_header_with_secret,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_postgres::NoTls;
use tower::ServiceExt;

#[tokio::test]
async fn built_label_exporter_uses_narrow_sources_and_preserves_output_on_failure() {
    let url = std::env::var("FC_CORE_POSTGRES_TEST_URL").expect("run through devfinity");
    let (admin, connection) = tokio_postgres::connect(&url, NoTls).await.unwrap();
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("brain_labels_process_{unique}");
    let role = format!("brain_labels_reader_{unique}");
    admin.batch_execute(&format!("CREATE SCHEMA {schema}; SET search_path TO {schema};
        CREATE TABLE users (workos_user_id TEXT, normalized_email TEXT, link_status TEXT, secret TEXT);
        CREATE TABLE projects (id TEXT, display_name TEXT);
        CREATE TABLE agent_runtimes (project_id TEXT, health_reporting_npub TEXT);
        INSERT INTO users VALUES ('verified', 'alex@example.com', 'linked', 'not-read');
        CREATE ROLE {role}; GRANT USAGE ON SCHEMA {schema} TO {role};
        GRANT SELECT (workos_user_id, normalized_email, link_status) ON users TO {role};
        GRANT SELECT (id, display_name) ON projects TO {role};
        GRANT SELECT (project_id, health_reporting_npub) ON agent_runtimes TO {role};
        SET ROLE {role};")).await.unwrap();
    assert!(
        admin
            .query("SELECT workos_user_id FROM users", &[])
            .await
            .is_ok()
    );
    assert!(admin.query("SELECT secret FROM users", &[]).await.is_err());
    assert!(admin.execute("DELETE FROM users", &[]).await.is_err());
    admin.batch_execute("RESET ROLE").await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let brain_db = dir.path().join("brain.sqlite3");
    let secret = [1_u8; 32];
    let proof = sign_http_auth_header_with_secret(
        &secret,
        &HttpAuthEventRequest::new("GET", "http://brain.test", 100),
    )
    .unwrap();
    let hex = decode_http_auth_header(&proof).unwrap().pubkey.to_hex();
    let npub = NostrPublicKey::parse(&hex).unwrap().to_npub().unwrap();
    let mut store = BrainStore::open(&brain_db).unwrap();
    store
        .create_brain_bootstrap(
            &finite_brain_core::bootstrap_organization_brain("org", "Org", &npub).unwrap(),
            &[],
        )
        .unwrap();
    drop(store);
    let hosted = dir.path().join("hosted");
    let chat = hosted
        .join("users")
        .join(format!("{:x}", Sha256::digest(b"verified")))
        .join("chat/client.sqlite3");
    fs::create_dir_all(chat.parent().unwrap()).unwrap();
    let conn = Connection::open(&chat).unwrap();
    // Existing hosted schema public columns; the encryption payload is opaque.
    conn.execute_batch("CREATE TABLE client_device_states (account_id TEXT NOT NULL, device_id TEXT NOT NULL, nonce BLOB NOT NULL, ciphertext BLOB NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY(account_id, device_id));").unwrap();
    conn.execute(
        "INSERT INTO client_device_states VALUES (?1, 'hosted-web', X'00', X'01', 'existing')",
        [&hex],
    )
    .unwrap();
    drop(conn);
    let output = dir.path().join("labels.json");
    let scoped_url = format!(
        "{url}{}options=-csearch_path%3D{schema}%20-crole%3D{role}",
        if url.contains('?') { '&' } else { '?' }
    );
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_finite-brain-labels"))
            .env_clear()
            .env("FINITE_BRAIN_LABEL_DATABASE_URL", &scoped_url)
            .env("FINITE_BRAIN_DB", &brain_db)
            .env("FINITECHAT_HOSTED_DATA_ROOT", &hosted)
            .env("FINITE_BRAIN_PRINCIPAL_LABELS", &output)
            .output()
            .unwrap()
    };
    let before_brain = fs::read(&brain_db).unwrap();
    let before_chat = fs::read(&chat).unwrap();
    let success = run();
    assert!(
        success.status.success(),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    let bytes = fs::read(&output).unwrap();
    let projection: LabelProjection = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        projection.labels(projection.generated_at).unwrap()[&npub].display(),
        "alex@example.com (human)"
    );
    assert_eq!(fs::read(&brain_db).unwrap(), before_brain);
    assert_eq!(fs::read(&chat).unwrap(), before_chat);
    assert!(!String::from_utf8_lossy(&success.stdout).contains("alex@example.com"));
    // Run the real daemon and authorized metadata entrypoint. No discovery on
    // startup; a cold response returns before work, then a later read has names.
    let daemon_output = dir.path().join("demand.json");
    let socket_path = dir.path().join("refresh.sock");
    struct Worker(std::process::Child);
    impl Drop for Worker {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut worker = Worker(
        Command::new(env!("CARGO_BIN_EXE_finite-brain-labels"))
            .env_clear()
            .env("FINITE_BRAIN_LABEL_DATABASE_URL", &scoped_url)
            .env("FINITE_BRAIN_DB", &brain_db)
            .env("FINITECHAT_HOSTED_DATA_ROOT", &hosted)
            .env("FINITE_BRAIN_PRINCIPAL_LABELS", &daemon_output)
            .env("FINITE_BRAIN_LABEL_SOCKET", &socket_path)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let started = std::time::Instant::now();
    while !socket_path.exists() {
        assert!(worker.0.try_wait().unwrap().is_none());
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !daemon_output.exists(),
        "idle service does not scan or publish"
    );
    let router = router_with_state(
        ServerState::new(BrainStore::open(&brain_db).unwrap(), "http://brain.test")
            .with_principal_labels_path(&daemon_output)
            .with_principal_labels_socket(&socket_path),
    );
    let read = |nonce: &str| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let auth = sign_http_auth_header_with_secret(
            &secret,
            &HttpAuthEventRequest::new("GET", "http://brain.test/v1/brains/org/metadata", now)
                .with_nonce(nonce),
        )
        .unwrap();
        router.clone().oneshot(
            Request::builder()
                .uri("/v1/brains/org/metadata")
                .header("Authorization", auth)
                .body(Body::empty())
                .unwrap(),
        )
    };
    assert!(read("cold").await.unwrap().status().is_success());
    while !daemon_output.exists() {
        assert!(worker.0.try_wait().unwrap().is_none());
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let response = read("warm").await.unwrap();
    assert!(response.status().is_success());
    let metadata: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(
        metadata["identities"][0]["display"],
        "alex@example.com (human)"
    );
    let published = fs::read(&daemon_output).unwrap();
    // New requests reuse the projection instead of querying even changed names.
    admin
        .execute(
            "UPDATE users SET normalized_email='changed@example.com'",
            &[],
        )
        .await
        .unwrap();
    for nonce in 0..20 {
        assert!(
            read(&format!("burst-{nonce}"))
                .await
                .unwrap()
                .status()
                .is_success()
        );
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(fs::read(&daemon_output).unwrap(), published);
    drop(worker);
    let unavailable = read("worker-down").await.unwrap();
    assert!(unavailable.status().is_success());
    let metadata: serde_json::Value = serde_json::from_slice(
        &to_bytes(unavailable.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        metadata["identities"][0]["display"],
        "alex@example.com (human)"
    );
    assert_eq!(fs::read(&chat).unwrap(), before_chat);
    // The long-lived worker must survive source errors and release its DB
    // driver, preserve old evidence, and recover on later demand after backoff.
    admin
        .batch_execute(&format!(
            "REVOKE SELECT (workos_user_id, normalized_email, link_status) ON users FROM {role}"
        ))
        .await
        .unwrap();
    let application = format!("brain_labels_daemon_{unique}");
    let daemon_url = format!("{scoped_url}&application_name={application}");
    let errors = dir.path().join("worker-errors.log");
    let mut recovering = Worker(
        Command::new(env!("CARGO_BIN_EXE_finite-brain-labels"))
            .env_clear()
            .env("FINITE_BRAIN_LABEL_DATABASE_URL", &daemon_url)
            .env("FINITE_BRAIN_DB", &brain_db)
            .env("FINITECHAT_HOSTED_DATA_ROOT", &hosted)
            .env("FINITE_BRAIN_PRINCIPAL_LABELS", &daemon_output)
            .env("FINITE_BRAIN_LABEL_SOCKET", &socket_path)
            .stdout(std::process::Stdio::null())
            .stderr(fs::File::create(&errors).unwrap())
            .spawn()
            .unwrap(),
    );
    let signal = std::os::unix::net::UnixDatagram::unbound().unwrap();
    signal.set_nonblocking(true).unwrap();
    let failure_started = std::time::Instant::now();
    loop {
        assert!(recovering.0.try_wait().unwrap().is_none());
        assert!(failure_started.elapsed() < std::time::Duration::from_secs(15));
        let _ = signal.send_to(b"refresh", &socket_path);
        if fs::read_to_string(&errors)
            .unwrap()
            .contains("Brain label refresh failed")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(fs::read(&daemon_output).unwrap(), published);
    assert!(read("source-failed").await.unwrap().status().is_success());
    let cleanup_started = std::time::Instant::now();
    loop {
        let active: i64 = admin
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name=$1",
                &[&application],
            )
            .await
            .unwrap()
            .get(0);
        if active == 0 {
            break;
        }
        assert!(
            cleanup_started.elapsed() < std::time::Duration::from_secs(5),
            "failed refresh leaked a connection"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    admin
        .batch_execute(&format!(
            "GRANT SELECT (workos_user_id, normalized_email, link_status) ON users TO {role}"
        ))
        .await
        .unwrap();
    // Use the actual production first-failure backoff, not a test-only setting.
    tokio::time::sleep(std::time::Duration::from_secs(31)).await;
    let recovery_started = std::time::Instant::now();
    loop {
        assert!(recovering.0.try_wait().unwrap().is_none());
        assert!(recovery_started.elapsed() < std::time::Duration::from_secs(15));
        let _ = signal.send_to(b"refresh", &socket_path);
        let current: LabelProjection =
            serde_json::from_slice(&fs::read(&daemon_output).unwrap()).unwrap();
        if current
            .bindings
            .iter()
            .any(|binding| binding.label.name == "changed@example.com")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let response = read("recovered").await.unwrap();
    assert!(response.status().is_success());
    let metadata: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(
        metadata["identities"][0]["display"],
        "changed@example.com (human)"
    );
    assert!(!fs::read_to_string(&errors).unwrap().contains("example.com"));
    drop(recovering);
    // Wrong-schema or corrupt existing hosted sources must not overwrite a
    // previously verified projection or leak identity/connection data to logs.
    let conn = Connection::open(&chat).unwrap();
    conn.execute_batch("DROP TABLE client_device_states")
        .unwrap();
    drop(conn);
    assert!(!run().status.success());
    assert_eq!(fs::read(&output).unwrap(), bytes);
    fs::write(&chat, b"not a database").unwrap();
    assert!(!run().status.success());
    assert_eq!(fs::read(&output).unwrap(), bytes);
    admin
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE; DROP ROLE {role};"))
        .await
        .unwrap();
    connection.abort();
}
