//! OpenRouter key validation and provider command contracts. Usage and
//! connection commands remain unsupported and unadvertised until implemented.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::AgentdError;

const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";
/// Test-only: points agentd at a fake OpenRouter.
const API_BASE_OVERRIDE: &str = "FINITE_AGENTD_OPENROUTER_API_BASE";
const KEY_CHECK_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_KEY_RESPONSE_BYTES: usize = 64 * 1024;

/// Allowance metadata returned by a successful `/key` validation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct KeyInfo {
    pub limit_usd: Option<f64>,
    pub limit_remaining_usd: Option<f64>,
}

/// `credential` in `finite.agent.openrouter.connect.v1`. No `Debug`:
/// it holds a key or an OAuth code.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[expect(
    dead_code,
    reason = "reserved for OpenRouter connection and usage support"
)]
pub(crate) enum ConnectCredential {
    ApiKey {
        api_key: String,
    },
    OauthCode {
        code: String,
        code_verifier: String,
        attempt_id: String,
    },
}

/// The OpenRouter API base. The test override is honored only for
/// `https://…` or `http://127.0.0.1:<port>`; anything else uses OpenRouter.
pub(crate) fn api_base() -> String {
    std::env::var(API_BASE_OVERRIDE)
        .ok()
        .filter(|value| valid_api_base(value))
        .map(|value| value.trim_end_matches('/').to_owned())
        .unwrap_or_else(|| OPENROUTER_API_BASE.to_owned())
}

fn valid_api_base(value: &str) -> bool {
    if value.starts_with("https://") {
        return true;
    }
    value.strip_prefix("http://127.0.0.1:").is_some_and(|rest| {
        let port = rest.split('/').next().unwrap_or_default();
        !port.is_empty() && port.parse::<u16>().is_ok()
    })
}

/// Validate a key against an explicit API base. This is metadata only: no
/// completion request is ever sent to test a key. The key never appears
/// in an error.
pub(crate) async fn check_key_at(api_base: &str, api_key: &str) -> Result<KeyInfo, AgentdError> {
    let client = reqwest::Client::builder()
        .timeout(KEY_CHECK_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| unreachable_provider())?;
    let mut response = client
        .get(format!("{api_base}/key"))
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|_| unreachable_provider())?;
    let status = response.status();
    if matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    ) {
        return Err(AgentdError::CredentialRejected(
            "OpenRouter didn't accept this key.".to_owned(),
        ));
    }
    if status != reqwest::StatusCode::OK {
        return Err(unreachable_provider());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unreachable_provider())? {
        if body.len() + chunk.len() > MAX_KEY_RESPONSE_BYTES {
            return Err(unreachable_provider());
        }
        body.extend_from_slice(&chunk);
    }
    let document = serde_json::from_slice::<Value>(&body).map_err(|_| unreachable_provider())?;
    let data = document
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(unreachable_provider)?;
    let flag = |name: &str| data.get(name).and_then(Value::as_bool) == Some(true);
    if flag("is_management_key") || flag("is_provisioning_key") {
        return Err(AgentdError::CredentialRejected(
            "That's an OpenRouter management key. Use an ordinary API key.".to_owned(),
        ));
    }
    let limit_usd = data.get("limit").and_then(Value::as_f64);
    let limit_remaining_usd = data.get("limit_remaining").and_then(Value::as_f64);
    if limit_usd.is_some() && limit_remaining_usd.is_some_and(|remaining| remaining <= 0.0) {
        return Err(AgentdError::KeyAllowanceExhausted);
    }
    Ok(KeyInfo {
        limit_usd,
        limit_remaining_usd,
    })
}

fn unreachable_provider() -> AgentdError {
    AgentdError::ProviderUnavailable("Couldn't reach OpenRouter to check the key.".to_owned())
}

/// A loopback fake of OpenRouter's `/key` for tests. It records every request
/// line, so a test can prove no completion request was sent.
#[cfg(test)]
pub(crate) mod fake {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    pub(crate) struct FakeOpenRouter {
        pub base: String,
        requests: Arc<Mutex<Vec<String>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl FakeOpenRouter {
        /// Every request gets `status` and `body`.
        pub(crate) async fn start(status: u16, body: &str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&requests);
            let body = body.to_owned();
            let task = tokio::spawn(async move {
                loop {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        return;
                    };
                    let mut buffer = vec![0_u8; 16 * 1024];
                    let size = socket.read(&mut buffer).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buffer[..size]).into_owned();
                    recorded
                        .lock()
                        .unwrap()
                        .push(head.lines().next().unwrap_or_default().to_owned());
                    let reply = format!(
                        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                }
            });
            Self {
                base,
                requests,
                task,
            }
        }

        pub(crate) fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Drop for FakeOpenRouter {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    pub(crate) fn key_data(data: serde_json::Value) -> String {
        serde_json::json!({ "data": data }).to_string()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::fake::{FakeOpenRouter, key_data};
    use super::*;

    const KEY: &str = "sk-or-v1-synthetic-test-key";

    async fn check(status: u16, body: &str) -> (Result<KeyInfo, AgentdError>, Vec<String>) {
        let server = FakeOpenRouter::start(status, body).await;
        let result = check_key_at(&server.base, KEY).await;
        (result, server.requests())
    }

    fn code(result: &Result<KeyInfo, AgentdError>) -> &'static str {
        match result {
            Ok(_) => "ok",
            Err(error) => error.public_code(),
        }
    }

    #[tokio::test]
    async fn check_key_rules_in_order() {
        let ordinary = json!({"limit": null, "limit_remaining": null, "usage": 0.5});
        let cases = [
            (401, key_data(ordinary.clone()), "credential_rejected"),
            (403, key_data(ordinary.clone()), "credential_rejected"),
            (500, key_data(ordinary.clone()), "provider_unavailable"),
            (429, key_data(ordinary.clone()), "provider_unavailable"),
            (200, "not json".to_owned(), "provider_unavailable"),
            (
                200,
                json!({"error": "x"}).to_string(),
                "provider_unavailable",
            ),
            (200, json!({"data": []}).to_string(), "provider_unavailable"),
            (
                200,
                key_data(json!({"is_management_key": true})),
                "credential_rejected",
            ),
            (
                200,
                key_data(json!({"is_provisioning_key": true})),
                "credential_rejected",
            ),
            // A management key with nothing remaining is still named a management key.
            (
                200,
                key_data(json!({"is_management_key": true, "limit": 10, "limit_remaining": 0})),
                "credential_rejected",
            ),
            (
                200,
                key_data(json!({"limit": 10, "limit_remaining": 0})),
                "key_allowance_exhausted",
            ),
            (
                200,
                key_data(json!({"limit": 10, "limit_remaining": -0.5})),
                "key_allowance_exhausted",
            ),
            (
                200,
                key_data(json!({"limit": null, "limit_remaining": 0})),
                "ok",
            ),
            (200, key_data(json!({"limit": 10})), "ok"),
            (
                200,
                key_data(json!({"limit": 10, "limit_remaining": "0"})),
                "ok",
            ),
            (
                200,
                key_data(json!({"limit": 10, "limit_remaining": 2.5})),
                "ok",
            ),
            (200, key_data(json!({"is_management_key": false})), "ok"),
        ];
        for (status, body, expected) in cases {
            let (result, requests) = check(status, &body).await;
            assert_eq!(code(&result), expected, "{status} {body}");
            // Metadata only: exactly one GET /key, never a completion request.
            assert_eq!(requests, ["GET /api/v1/key HTTP/1.1"], "{status} {body}");
            if let Err(error) = &result {
                assert!(!error.public_message().contains(KEY));
            }
        }
        let (result, _) = check(200, &key_data(json!({"is_management_key": true}))).await;
        assert_eq!(
            result.unwrap_err().public_message(),
            "That's an OpenRouter management key. Use an ordinary API key."
        );
        let (result, _) = check(200, &key_data(json!({"limit": 10, "limit_remaining": 0}))).await;
        assert_eq!(
            result.unwrap_err().public_message(),
            "This key has no remaining allowance. Raise its limit at openrouter.ai/keys, or use another key."
        );
        let (result, _) = check(200, &key_data(json!({"limit": 10, "limit_remaining": 2.5}))).await;
        assert_eq!(
            result.unwrap(),
            KeyInfo {
                limit_usd: Some(10.0),
                limit_remaining_usd: Some(2.5)
            }
        );
    }

    #[tokio::test]
    async fn an_unreachable_provider_is_unavailable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
        drop(listener);
        let result = check_key_at(&base, KEY).await;
        assert_eq!(code(&result), "provider_unavailable");
    }

    #[test]
    fn the_test_base_accepts_only_https_or_loopback_ports() {
        for valid in [
            "https://openrouter.example.test/api/v1",
            "http://127.0.0.1:8080",
            "http://127.0.0.1:8080/api/v1",
        ] {
            assert!(valid_api_base(valid), "{valid}");
        }
        for invalid in [
            "http://openrouter.ai/api/v1",
            "http://127.0.0.1",
            "http://127.0.0.1:/x",
            "http://127.0.0.1:99999",
            "http://localhost:8080",
            "ftp://127.0.0.1:21",
            "",
        ] {
            assert!(!valid_api_base(invalid), "{invalid}");
        }
    }
}
