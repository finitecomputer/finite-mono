//! Real exporter entrypoint against retained source columns and private output.
use finite_brain_server::principal_labels::LabelProjection;
use finite_brain_store::BrainStore;
use finite_nostr::NostrPublicKey;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_postgres::NoTls;

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
    let hex = "01".repeat(32);
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
