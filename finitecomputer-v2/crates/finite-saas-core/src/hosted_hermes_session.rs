//! Native session exchange, never custom token minting. The bounded in-memory
//! cookie cache is disposable; Core's durable credentials remain authoritative.
use crate::store::hosted_hermes::HostedLogin;
use reqwest::{Client, Response, StatusCode};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedSession {
    pub base_url: String,
    pub access_token: String,
    pub expires_at: i64,
}
#[derive(Clone, Default)]
pub struct NativeSessions {
    entries: Arc<Mutex<BTreeMap<String, Arc<Mutex<Cached>>>>>,
    #[cfg(test)]
    test_client: Option<Client>,
}
struct Cached {
    binding: Option<HostedLogin>,
    cookies: BTreeMap<String, String>,
    touched: Instant,
}
impl Default for Cached {
    fn default() -> Self {
        Self {
            binding: None,
            cookies: BTreeMap::new(),
            touched: Instant::now(),
        }
    }
}

impl NativeSessions {
    #[cfg(test)]
    pub(crate) fn with_test_client(client: Client) -> Self {
        Self {
            test_client: Some(client),
            ..Self::default()
        }
    }

    pub async fn grant(&self, binding: &HostedLogin) -> Result<HostedSession, &'static str> {
        #[cfg(test)]
        if let Some(client) = &self.test_client {
            return self.grant_with_client(binding, client).await;
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "native client unavailable")?;
        self.grant_with_client(binding, &client).await
    }

    pub(crate) async fn grant_with_client(
        &self,
        binding: &HostedLogin,
        client: &Client,
    ) -> Result<HostedSession, &'static str> {
        let entry = {
            let mut entries = self.entries.lock().await;
            entries.retain(|_, entry| {
                Arc::strong_count(entry) > 1
                    || entry
                        .try_lock()
                        .is_ok_and(|e| e.touched.elapsed() < Duration::from_secs(600))
            });
            if !entries.contains_key(&binding.runtime_id) && entries.len() >= 1024 {
                return Err("native session capacity reached");
            }
            entries
                .entry(binding.runtime_id.clone())
                .or_default()
                .clone()
        };
        // Single-flight per runtime; independent runtimes do not wait on its HTTP.
        let mut cache = entry.lock().await;
        cache.touched = Instant::now();
        if cache.binding.as_ref() != Some(binding) {
            cache.cookies.clear();
            cache.binding = Some(binding.clone());
        }
        let identity_url = format!("{}api/auth/me", binding.base_url);
        let mut response = if cache.cookies.is_empty() {
            None
        } else {
            Some(
                client
                    .get(&identity_url)
                    .header("cookie", cookie_header(&cache.cookies))
                    .send()
                    .await
                    .map_err(|_| "native session unavailable")?,
            )
        };
        if response
            .as_ref()
            .is_some_and(|r| r.status() == StatusCode::UNAUTHORIZED)
        {
            cache.cookies.clear();
            response = None;
        }
        if response.is_none() {
            let login = client.post(format!("{}auth/password-login", binding.base_url))
                .json(&serde_json::json!({"provider":"basic","username":binding.username,"password":binding.password}))
                .send().await.map_err(|_| "native login unavailable")?;
            if login.status() != StatusCode::OK {
                return Err("native login rejected");
            }
            update_cookies(&mut cache.cookies, &login)?;
            if access_token(&cache.cookies).is_none() {
                return Err("native login missing session");
            }
            response = Some(
                client
                    .get(&identity_url)
                    .header("cookie", cookie_header(&cache.cookies))
                    .send()
                    .await
                    .map_err(|_| "native session unavailable")?,
            );
        }
        let mut response = response.ok_or("native session unavailable")?;
        if response.status() != StatusCode::OK {
            return Err("native session rejected");
        }
        update_cookies(&mut cache.cookies, &response)?;
        let mut identity = bounded_json(response).await?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        if identity
            .get("expires_at")
            .and_then(|v| v.as_i64())
            .is_some_and(|expiry| expiry <= now + 5)
        {
            // Let native cookie middleware refresh. Never decode/mint native tokens.
            cache
                .cookies
                .retain(|name, _| bare_name(name) != Some("hermes_session_at"));
            response = client
                .get(&identity_url)
                .header("cookie", cookie_header(&cache.cookies))
                .send()
                .await
                .map_err(|_| "native renewal unavailable")?;
            if response.status() != StatusCode::OK {
                cache.cookies.clear();
                return Err("native renewal rejected");
            }
            update_cookies(&mut cache.cookies, &response)?;
            identity = bounded_json(response).await?;
        }
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let expires_at = identity
            .get("expires_at")
            .and_then(|v| v.as_i64())
            .ok_or("native expiry missing")?;
        if identity.get("provider").and_then(|v| v.as_str()) != Some("basic")
            || identity.get("user_id").and_then(|v| v.as_str()) != Some(binding.username.as_str())
            || !valid_native_expiry(expires_at, now)
        {
            cache.cookies.clear();
            return Err("native session identity or lifetime mismatch");
        }
        Ok(HostedSession {
            base_url: binding.base_url.clone(),
            access_token: access_token(&cache.cookies)
                .ok_or("native access missing")?
                .to_string(),
            expires_at,
        })
    }
}

// The protected native read already proves the token is currently accepted.
// Bound server clock skew without asking the browser's wall clock to authorize it.
fn valid_native_expiry(expires_at: i64, now: i64) -> bool {
    expires_at > now - 30 && expires_at <= now + 90
}

fn bare_name(name: &str) -> Option<&str> {
    let bare = name
        .strip_prefix("__Host-")
        .or_else(|| name.strip_prefix("__Secure-"))
        .unwrap_or(name);
    matches!(
        bare,
        "hermes_session_at" | "hermes_session_rt" | "hermes_session_provider"
    )
    .then_some(bare)
}
fn update_cookies(
    cookies: &mut BTreeMap<String, String>,
    response: &Response,
) -> Result<(), &'static str> {
    for header in response.headers().get_all(reqwest::header::SET_COOKIE) {
        let value = header.to_str().map_err(|_| "invalid native cookie")?;
        if value.len() > 8192 {
            return Err("oversized native cookie");
        }
        let Some((name, value)) = value.split(';').next().and_then(|v| v.split_once('=')) else {
            continue;
        };
        let Some(bare) = bare_name(name) else {
            continue;
        };
        if value.chars().any(char::is_control) {
            return Err("invalid native cookie");
        }
        cookies.retain(|n, _| bare_name(n) != Some(bare));
        if !value.is_empty() {
            cookies.insert(name.into(), value.into());
        }
    }
    Ok(())
}
fn access_token(cookies: &BTreeMap<String, String>) -> Option<&str> {
    cookies
        .iter()
        .find(|(name, _)| bare_name(name) == Some("hermes_session_at"))
        .map(|(_, value)| value.as_str())
}
fn cookie_header(cookies: &BTreeMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}
async fn bounded_json(mut response: Response) -> Result<serde_json::Value, &'static str> {
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "native response unavailable")?
    {
        if data.len() + chunk.len() > 32768 {
            return Err("oversized native response");
        }
        data.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&data).map_err(|_| "invalid native response")
}

#[cfg(test)]
mod tests {
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
                !serialized.contains("synthetic-refresh")
                    && !serialized.contains("synthetic-password")
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
}
