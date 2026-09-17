use super::*;
use axum::http::{HeaderMap, HeaderValue};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn native_expiry_tolerates_bounded_server_clock_skew() {
    for expiry in [971, 1000, 1060, 1090] {
        assert!(valid_native_expiry(expiry, 1000));
    }
    for expiry in [970, 1091, 4600] {
        assert!(!valid_native_expiry(expiry, 1000));
    }
}

#[tokio::test]
async fn native_exchange_reuses_cookies_projects_only_access_and_fences_generation() {
    #[derive(Clone, Default)]
    struct Counts {
        login: Arc<AtomicUsize>,
        identity: Arc<AtomicUsize>,
    }
    async fn login(State(c): State<Counts>) -> (HeaderMap, Json<serde_json::Value>) {
        c.login.fetch_add(1, Ordering::SeqCst);
        let mut headers = HeaderMap::new();
        for cookie in [
            "hermes_session_at=synthetic-access; HttpOnly",
            "hermes_session_rt=synthetic-refresh; HttpOnly",
            "hermes_session_provider=basic; HttpOnly",
        ] {
            headers.append("set-cookie", HeaderValue::from_static(cookie));
        }
        (headers, Json(serde_json::json!({"ok":true})))
    }
    async fn identity(State(c): State<Counts>, headers: HeaderMap) -> Json<serde_json::Value> {
        c.identity.fetch_add(1, Ordering::SeqCst);
        assert!(
            headers["cookie"]
                .to_str()
                .unwrap()
                .contains("hermes_session_rt=synthetic-refresh")
        );
        Json(
            serde_json::json!({"provider":"basic","user_id":"native-user","expires_at":time::OffsetDateTime::now_utc().unix_timestamp()+60}),
        )
    }
    let counts = Counts::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/auth/password-login", post(login))
        .route("/api/auth/me", get(identity))
        .with_state(counts.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut binding = HostedLogin {
        runtime_id: "runtime".into(),
        creation_request_id: "creation".into(),
        generation: 1,
        base_url: url,
        username: "native-user".into(),
        password: "synthetic-password".into(),
    };
    let sessions = NativeSessions::default();
    for _ in 0..3 {
        let grant = sessions.grant(&binding).await.unwrap();
        let serialized = serde_json::to_string(&grant).unwrap();
        assert!(
            !serialized.contains("synthetic-refresh") && !serialized.contains("synthetic-password")
        );
    }
    assert_eq!(counts.login.load(Ordering::SeqCst), 1);
    binding.generation = 2;
    sessions.grant(&binding).await.unwrap();
    assert_eq!(counts.login.load(Ordering::SeqCst), 2);
    assert_eq!(counts.identity.load(Ordering::SeqCst), 4);
    server.abort();
}

#[tokio::test]
#[ignore = "requires repo-pinned HERMES_NATIVE_TEST_PYTHON and HERMES_NATIVE_TEST_SOURCE"]
async fn actual_native_session_expires_and_renews_without_browser_refresh_credentials() {
    struct Process(std::process::Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let python = std::env::var("HERMES_NATIVE_TEST_PYTHON").unwrap();
    let source = std::env::var("HERMES_NATIVE_TEST_SOURCE").unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.yaml"),
        "plugins:\n  disabled: [dashboard_auth/nous]\n",
    )
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let password = crate::store::runtime_credentials::new_secret().unwrap();
    let signing = crate::store::runtime_credentials::new_secret().unwrap();
    let log = std::fs::File::create(home.path().join("native.log")).unwrap();
    let _process = Process(
        std::process::Command::new(python)
            .args([
                "-m",
                "hermes_cli.main",
                "serve",
                "--isolated",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--no-open",
            ])
            .env("PYTHONPATH", &source)
            .env("HERMES_HOME", home.path())
            .env("HERMES_BUNDLED_PLUGINS", format!("{source}/plugins"))
            .env("HERMES_DASHBOARD_PUBLIC_URL", "https://native-test.invalid")
            .env("HERMES_DASHBOARD_BASIC_AUTH_USERNAME", "native-user")
            .env("HERMES_DASHBOARD_BASIC_AUTH_PASSWORD", &password)
            .env("HERMES_DASHBOARD_BASIC_AUTH_SECRET", signing)
            .env("HERMES_DASHBOARD_BASIC_AUTH_TTL_SECONDS", "60")
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let binding = HostedLogin {
        runtime_id: "runtime".into(),
        creation_request_id: "creation".into(),
        generation: 1,
        base_url: format!("http://127.0.0.1:{port}/"),
        username: "native-user".into(),
        password,
    };
    let sessions = NativeSessions::default();
    let client = Client::new();
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            if client
                .get(format!("{}api/auth/me", binding.base_url))
                .send()
                .await
                .is_ok_and(|r| r.status() == StatusCode::UNAUTHORIZED)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("native Hermes did not become ready");
    let first = sessions.grant(&binding).await.unwrap();
    assert_eq!(
        client
            .get(format!("{}api/auth/me", first.base_url))
            .bearer_auth(&first.access_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    // Real expiration; no custom token mutation and no model inference.
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(
        client
            .get(format!("{}api/auth/me", first.base_url))
            .bearer_auth(&first.access_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let renewed = sessions.grant(&binding).await.unwrap();
    assert!(renewed.access_token != first.access_token);
    assert_eq!(
        client
            .get(format!("{}api/auth/me", renewed.base_url))
            .bearer_auth(&renewed.access_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}
