use reqwest::Client;
use serde_json::{Value, json};
use std::{
    fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tempfile::TempDir;

const TOKEN: &str = "synthetic-local-test-token-not-a-secret-123456";
struct Server {
    child: Child,
    root: TempDir,
    url: String,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn private_file(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
impl Server {
    async fn start(existing: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("hermes")).unwrap();
        private_file(&root.path().join("token"), TOKEN);
        fs::write(
            root.path().join("hermes/config.yaml"),
            "model:\n  default: preserved-model\n",
        )
        .unwrap();
        if existing {
            // An old-schema database with data an older binary must still read.
            let ledger =
                finite_agentd::Ledger::open(root.path().join("agent/agentd/agentd.sqlite3"))
                    .unwrap();
            ledger.authorize_principal("old-owner").unwrap();
            let db = rusqlite::Connection::open(ledger.path()).unwrap();
            db.execute("INSERT INTO command_ledger VALUES ('old-request','old-fingerprint','terminal','old-wire-result',1)", []).unwrap();
        }
        // There is deliberately no chat config.json, chat executable, health
        // service, or prepare script. The only supervised program exits cleanly.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(env!("CARGO_BIN_EXE_finite-agentd"))
            .args([
                "control-serve",
                "--listen",
                &format!("127.0.0.1:{port}"),
                "--runtime-id",
                "runtime-test",
            ])
            .arg("--token-file")
            .arg(root.path().join("token"))
            .arg("--agent-home")
            .arg(root.path().join("agent"))
            .arg("--hermes-home")
            .arg(root.path().join("hermes"))
            .args(["--hermes-command", "/usr/bin/true"])
            .env("FINITECHAT_BIN", "/must-not-be-called/finitechat")
            .env(
                "FINITE_AGENTD_PREPARE_COMMAND",
                "/must-not-be-called/prepare",
            )
            .env("FINITE_AGENTD_BRIDGE_ADDR", "intentionally-invalid")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut server = Self {
            child,
            root,
            url: format!("http://127.0.0.1:{port}/v1/runtimes/runtime-test/connections"),
        };
        let client = Client::new();
        for _ in 0..100 {
            assert!(
                server.child.try_wait().unwrap().is_none(),
                "control server exited"
            );
            if client
                .get(&server.url)
                .bearer_auth(TOKEN)
                .send()
                .await
                .is_ok()
            {
                return server;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("listener did not start");
    }
    async fn command(&self, id: &str, name: &str) -> reqwest::Response {
        Client::new()
            .post(format!("{}/commands", self.url))
            .bearer_auth(TOKEN)
            .json(&json!({
                "request_id":id,"command":name,"schema":"finite.agent.empty.request.v1","body":{}
            }))
            .send()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn new_and_existing_agents_manage_settings_without_chat() {
    for existing in [false, true] {
        let server = Server::start(existing).await;
        let home = server.root.path().join("hermes");
        let before = fs::read(home.join("config.yaml")).unwrap();
        private_file(
            &home.join("google_token.json"),
            "synthetic-google-credential",
        );
        let status = Client::new()
            .get(&server.url)
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(status.status(), 200);
        assert_eq!(status.headers()["cache-control"], "no-store");
        let text = status.text().await.unwrap();
        assert!(!text.contains("synthetic-google-credential"));
        let result: Value = server
            .command("disconnect-1", "agent.google.disconnect")
            .await
            .json()
            .await
            .unwrap();
        assert_eq!(result, json!({"ok":true,"result":{"connected":false}}));
        assert!(!home.join("google_token.json").exists());
        // A repeated request replays its outcome, rather than deleting a newer credential.
        private_file(&home.join("google_token.json"), "new-synthetic-credential");
        let replay: Value = server
            .command("disconnect-1", "agent.google.disconnect")
            .await
            .json()
            .await
            .unwrap();
        assert_eq!(replay, result);
        assert!(home.join("google_token.json").exists());
        assert_eq!(
            server
                .command("disconnect-1", "agent.telegram.disconnect")
                .await
                .status(),
            409
        );
        assert_eq!(fs::read(home.join("config.yaml")).unwrap(), before);
        assert!(!server.root.path().join("agent/config.json").exists());
        let ledger =
            finite_agentd::Ledger::open(server.root.path().join("agent/agentd/agentd.sqlite3"))
                .unwrap();
        assert_eq!(
            ledger.authorized_principal_count().unwrap(),
            usize::from(existing)
        );
        if existing {
            let db = rusqlite::Connection::open(ledger.path()).unwrap();
            let old: String = db
                .query_row(
                    "SELECT result_json FROM command_ledger WHERE request_id='old-request'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(old, "old-wire-result");
        }
        // Simulate an interrupted operation by removing only its recorded outcome.
        let db = rusqlite::Connection::open(ledger.path()).unwrap();
        db.execute("UPDATE control_requests SET result_json=NULL", [])
            .unwrap();
        assert_eq!(
            server
                .command("disconnect-1", "agent.google.disconnect")
                .await
                .status(),
            409
        );
        assert!(home.join("google_token.json").exists());
    }
}

#[tokio::test]
async fn authorization_targeting_and_rotation_fail_closed() {
    let server = Server::start(false).await;
    let client = Client::new();
    assert_eq!(client.get(&server.url).send().await.unwrap().status(), 401);
    assert_eq!(
        client
            .get(&server.url)
            .bearer_auth("wrong")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(server.url.replace("runtime-test", "other-runtime"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{}?token={TOKEN}", server.url))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let replacement = "synthetic-rotated-local-token-123456789";
    private_file(&server.root.path().join("replacement"), replacement);
    fs::rename(
        server.root.path().join("replacement"),
        server.root.path().join("token"),
    )
    .unwrap();
    assert_eq!(
        client
            .get(&server.url)
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(&server.url)
            .bearer_auth(replacement)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    fs::remove_file(server.root.path().join("token")).unwrap();
    assert_eq!(
        client
            .get(&server.url)
            .bearer_auth(replacement)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
#[ignore = "requires FINITE_AGENTD_TEST_CADDY pointing to the pinned Caddy binary"]
async fn https_proxy_uses_verified_tls_for_real_mutation() {
    let server = Server::start(false).await;
    let caddy_binary = std::env::var("FINITE_AGENTD_TEST_CADDY").expect("pinned Caddy executable");
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let storage = server.root.path().join("caddy-storage");
    let upstream = server.url.split("/v1/").next().unwrap();
    let config = format!(
        "{{\n admin off\n auto_https disable_redirects\n skip_install_trust\n storage file_system {{\n root {}\n }}\n log default {{\n output discard\n }}\n}}\nhttps://localhost:{port} {{\n bind 127.0.0.1\n tls internal\n reverse_proxy {upstream}\n}}\n",
        storage.display()
    );
    let config_path = server.root.path().join("Caddyfile");
    fs::write(&config_path, config).unwrap();
    let child = Command::new(caddy_binary)
        .arg("run")
        .arg("--config")
        .arg(config_path)
        .args(["--adapter", "caddyfile"])
        .env("XDG_DATA_HOME", server.root.path().join("caddy-data"))
        .env("XDG_CONFIG_HOME", server.root.path().join("caddy-config"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Reuse the process guard for cleanup, including assertion failures.
    let mut proxy = Server {
        child,
        root: tempfile::tempdir().unwrap(),
        url: String::new(),
    };
    let cert_path = storage.join("pki/authorities/local/root.crt");
    for _ in 0..100 {
        assert!(proxy.child.try_wait().unwrap().is_none(), "Caddy exited");
        if cert_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let cert = reqwest::Certificate::from_pem(&fs::read(cert_path).unwrap()).unwrap();
    let client = Client::builder()
        .add_root_certificate(cert)
        .build()
        .unwrap();
    let url = format!("https://localhost:{port}/v1/runtimes/runtime-test/connections");
    let mut ready = false;
    for _ in 0..100 {
        if client.get(&url).bearer_auth(TOKEN).send().await.is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ready, "verified TLS connection did not become ready");
    assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
    let credential = server.root.path().join("hermes/google_token.json");
    private_file(&credential, "synthetic-google-credential");
    let result: Value = client.post(format!("{url}/commands")).bearer_auth(TOKEN).json(&json!({
        "request_id":"tls-disconnect", "command":"agent.google.disconnect", "schema":"finite.agent.empty.request.v1", "body":{}
    })).send().await.unwrap().json().await.unwrap();
    assert_eq!(result, json!({"ok":true,"result":{"connected":false}}));
    assert!(!credential.exists());
}
