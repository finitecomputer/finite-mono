//! Disposable Sites registry for native model-turn parity; no production state.
use super::*;

pub(super) struct Sites {
    binary: String,
    data: tempfile::TempDir,
    token: String,
    _child: tokio::process::Child,
}

impl Sites {
    pub async fn start() -> Option<Self> {
        let binary = std::env::var("FC_TEST_SUBSTRATE_SITES_BINARY").ok()?;
        let data = tempfile::tempdir().unwrap();
        let token = "a1".repeat(32); // Disposable local service credential.
        let mut child = tokio::process::Command::new(&binary)
            .args(["serve", "--data"])
            .arg(data.path())
            .args([
                "--listen",
                "0.0.0.0:18429",
                "--api-url",
                "http://host.docker.internal:18429",
                "--mailer",
                "dev",
            ])
            .env("FINITE_SITES_VIEWER_SESSION_TOKEN", &token)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..50 {
            assert!(child.try_wait().unwrap().is_none(), "local Sites exited");
            if tokio::net::TcpStream::connect("127.0.0.1:18429")
                .await
                .is_ok()
            {
                return Some(Self {
                    binary,
                    data,
                    token,
                    _child: child,
                });
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("local Sites did not listen");
    }

    pub async fn grant(&self, binding: &Value, index: usize) -> Value {
        let agent = binding["agent_npub"].as_str().unwrap();
        let owner = binding["human_account_id"].as_str().unwrap();
        let output = tokio::process::Command::new(&self.binary)
            .args(["allow", "--data"])
            .arg(self.data.path())
            .arg(agent)
            .output()
            .await
            .unwrap();
        assert!(output.status.success(), "local Sites admission failed");
        let email = format!("substrate-proof-{index}@finite.test");
        let assertion: Value = reqwest::Client::new()
            .post("http://127.0.0.1:18429/internal/v1/hosted-requester-assertions")
            .bearer_auth(&self.token)
            .json(&json!({"email": email, "requester_npub": owner, "agent_npub": agent}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        json!({
            "api": "http://host.docker.internal:18429",
            "slug": format!("native-owner-{index}"),
            "requester": {"userId": owner, "email": email, "sitesAssertion": assertion["assertion"], "expiresAt": assertion["expires_at"]}
        })
    }
}
