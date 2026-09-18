//! Native session exchange, never custom token minting. The bounded in-memory
//! cookie cache is disposable; Core's durable credentials remain authoritative.
use crate::store::hosted_hermes::HostedLogin;
use reqwest::{Client, Response, StatusCode};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
#[cfg(test)]
mod tests;

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
