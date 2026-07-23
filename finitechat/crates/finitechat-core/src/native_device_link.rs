use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use finitechat_http::{
    AckLinkPayloadRequest, AckLinkPayloadResponse, ClaimLinkPayloadRequest,
    ClaimLinkPayloadResponse, CreateLinkSessionRequest, ErrorResponse, ExpireLinkSessionRequest,
    HttpLinkSessionRecord, ReleaseLinkClaimRequest,
};
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::FiniteChatCoreError;
use crate::device_link::{
    DEVICE_LINK_MAX_TTL_SECONDS, DeviceLinkDecryptInput, DeviceLinkPairingKey,
    create_device_link_pairing_key, decrypt_device_link_payload,
};

const MAX_LINK_RESPONSE_BYTES: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CLAIM_POLL_INTERVAL: Duration = Duration::from_millis(400);

/// A one-use native account link. Pairing and account secrets remain behind
/// this Rust object boundary and never enter `AppState`.
#[derive(uniffi::Object)]
pub struct NativeDeviceLinkSession {
    client: Client,
    server_url: String,
    dashboard_url: String,
    target_device_id: String,
    link_session_id: String,
    deadline_unix_seconds: u64,
    state: Mutex<NativeDeviceLinkState>,
}

struct NativeDeviceLinkState {
    pairing: DeviceLinkPairingKey,
    claimed: Option<NativeClaim>,
    acknowledged: bool,
}

struct NativeClaim {
    claim_token: String,
    account_secret_hex: String,
}

#[derive(Serialize)]
struct DashboardDeviceLinkRequest<'a> {
    link_session_id: &'a str,
    target_device_id: &'a str,
}

#[derive(Deserialize)]
struct DashboardDeviceLinkResponse {
    link_session_id: String,
    target_device_id: String,
    status: String,
}

#[uniffi::export]
impl NativeDeviceLinkSession {
    #[uniffi::constructor]
    pub fn create(
        server_url: String,
        dashboard_url: String,
        target_device_id: String,
    ) -> Result<Arc<Self>, FiniteChatCoreError> {
        let server_url = normalize_base_url(&server_url)?;
        let dashboard_url = normalize_base_url(&dashboard_url)?;
        validate_device_id(&target_device_id)?;
        let pairing = create_device_link_pairing_key();
        let link_session_id = random_link_session_id()?;
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| link_error("device-link request failed"))?;
        let _: HttpLinkSessionRecord = post_json(
            &client,
            &server_url,
            "/link-sessions",
            &CreateLinkSessionRequest {
                link_session_id: link_session_id.clone(),
                pairing_public_key: pairing.public_key_hex.clone(),
            },
        )?;
        let deadline_unix_seconds = now_unix_seconds()?
            .checked_add(DEVICE_LINK_MAX_TTL_SECONDS)
            .ok_or_else(|| link_error("invalid device-link configuration"))?;

        Ok(Arc::new(Self {
            client,
            server_url,
            dashboard_url,
            target_device_id,
            link_session_id,
            deadline_unix_seconds,
            state: Mutex::new(NativeDeviceLinkState {
                pairing,
                claimed: None,
                acknowledged: false,
            }),
        }))
    }

    /// Ask the authenticated dashboard account to approve this exact
    /// rendezvous. Electron uses its isolated dashboard session; native
    /// clients provide an AuthKit bearer token. A missing token is accepted
    /// only for the loopback development dashboard.
    pub fn approve_authenticated_account(
        &self,
        access_token: Option<String>,
    ) -> Result<(), FiniteChatCoreError> {
        let response = self.dashboard_post("/api/device-links/approve", access_token.as_deref())?;
        self.validate_dashboard_response(&response)?;
        if !matches!(
            response.status.as_str(),
            "awaiting_claim" | "awaiting_key_package" | "joining_rooms" | "ready"
        ) {
            return Err(link_error(
                "device-link dashboard returned an invalid response",
            ));
        }
        Ok(())
    }

    /// Poll after authenticated account approval. The plaintext crosses only
    /// this native call so Swift can write it directly to Keychain.
    pub fn claim_account_secret(&self) -> Result<String, FiniteChatCoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| FiniteChatCoreError::LockPoisoned)?;
        if let Some(claimed) = &state.claimed {
            return Ok(claimed.account_secret_hex.clone());
        }

        loop {
            let now = now_unix_seconds()?;
            if now >= self.deadline_unix_seconds {
                let _ = post_without_response(
                    &self.client,
                    &self.server_url,
                    "/link-sessions/expire",
                    &ExpireLinkSessionRequest {
                        link_session_id: self.link_session_id.clone(),
                    },
                );
                return Err(link_error("device-link request expired"));
            }

            let response = self
                .client
                .post(endpoint(&self.server_url, "/link-sessions/claim"))
                .json(&ClaimLinkPayloadRequest {
                    link_session_id: self.link_session_id.clone(),
                })
                .send();
            let response = match response {
                Ok(response)
                    if response.status() == StatusCode::CONFLICT
                        || response.status().is_server_error() =>
                {
                    std::thread::sleep(CLAIM_POLL_INTERVAL);
                    continue;
                }
                Ok(response) if response.status() == StatusCode::BAD_REQUEST => {
                    let status = response.status();
                    let bytes = bounded_response_bytes(response)?;
                    if serde_json::from_slice::<ErrorResponse>(&bytes)
                        .is_ok_and(|error| error.kind == "link_session_not_ready")
                    {
                        std::thread::sleep(CLAIM_POLL_INTERVAL);
                        continue;
                    }
                    return Err(link_error(format!(
                        "device-link server rejected the request ({})",
                        status.as_u16()
                    )));
                }
                Ok(response) => response,
                Err(_) => {
                    std::thread::sleep(CLAIM_POLL_INTERVAL);
                    continue;
                }
            };
            let claimed: ClaimLinkPayloadResponse = decode_response(response)?;
            let payload = decrypt_device_link_payload(DeviceLinkDecryptInput {
                pairing_secret_key_hex: state.pairing.secret_key_hex.clone(),
                encrypted_payload: claimed.encrypted_payload,
                expected_link_session_id: self.link_session_id.clone(),
                expected_pairing_public_key: state.pairing.public_key_hex.clone(),
                expected_target_device_id: self.target_device_id.clone(),
                expected_server_url: self.server_url.clone(),
                now_unix_seconds: now,
            });
            let payload = match payload {
                Ok(payload) => payload,
                Err(_) => {
                    let _ = post_without_response(
                        &self.client,
                        &self.server_url,
                        "/link-sessions/release",
                        &ReleaseLinkClaimRequest {
                            link_session_id: self.link_session_id.clone(),
                        },
                    );
                    return Err(link_error("device-link payload failed authentication"));
                }
            };
            let account_secret_hex = payload.account_secret_hex;
            state.claimed = Some(NativeClaim {
                claim_token: claimed.claim_token,
                account_secret_hex: account_secret_hex.clone(),
            });
            return Ok(account_secret_hex);
        }
    }

    /// Call only after Keychain durably accepts the claimed secret.
    pub fn acknowledge_stored(&self) -> Result<(), FiniteChatCoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| FiniteChatCoreError::LockPoisoned)?;
        if state.acknowledged {
            return Ok(());
        }
        let claim_token = state
            .claimed
            .as_ref()
            .map(|claim| claim.claim_token.clone())
            .ok_or_else(|| link_error("device-link payload has not been claimed"))?;
        let request = AckLinkPayloadRequest {
            link_session_id: self.link_session_id.clone(),
            claim_token,
        };
        loop {
            if now_unix_seconds()? >= self.deadline_unix_seconds {
                return Err(link_error("device-link request expired"));
            }
            let response = self
                .client
                .post(endpoint(&self.server_url, "/link-sessions/ack"))
                .json(&request)
                .send();
            let response = match response {
                Ok(response) if response.status().is_server_error() => {
                    std::thread::sleep(CLAIM_POLL_INTERVAL);
                    continue;
                }
                Ok(response) => response,
                Err(_) => {
                    std::thread::sleep(CLAIM_POLL_INTERVAL);
                    continue;
                }
            };
            let response: AckLinkPayloadResponse = decode_response(response)?;
            if !response.acked {
                return Err(link_error(
                    "device-link server returned an invalid response",
                ));
            }
            state.acknowledged = true;
            return Ok(());
        }
    }

    /// The hosted account device performs room fanout only after the native
    /// key claim is durably acknowledged. Polling this existing dashboard API
    /// is the same finalization step used by Electron.
    pub fn wait_until_ready(
        &self,
        access_token: Option<String>,
    ) -> Result<(), FiniteChatCoreError> {
        loop {
            if now_unix_seconds()? >= self.deadline_unix_seconds {
                return Err(link_error("device-link request expired"));
            }
            let response = self.dashboard_post("/api/device-links/status", access_token.as_deref());
            let response = match response {
                Ok(response) => response,
                Err(_) => {
                    std::thread::sleep(CLAIM_POLL_INTERVAL);
                    continue;
                }
            };
            self.validate_dashboard_response(&response)?;
            match response.status.as_str() {
                "ready" => return Ok(()),
                "awaiting_claim" | "awaiting_key_package" | "joining_rooms" => {
                    std::thread::sleep(CLAIM_POLL_INTERVAL);
                }
                "expired" => return Err(link_error("device-link request expired")),
                _ => {
                    return Err(link_error(
                        "device-link dashboard returned an invalid response",
                    ));
                }
            }
        }
    }

    pub fn release(&self) {
        let _ = post_without_response(
            &self.client,
            &self.server_url,
            "/link-sessions/release",
            &ReleaseLinkClaimRequest {
                link_session_id: self.link_session_id.clone(),
            },
        );
    }
}

impl NativeDeviceLinkSession {
    fn dashboard_post(
        &self,
        path: &str,
        access_token: Option<&str>,
    ) -> Result<DashboardDeviceLinkResponse, FiniteChatCoreError> {
        if access_token.is_none() && !is_loopback_http_url(&self.dashboard_url) {
            return Err(link_error(
                "device-link dashboard authentication is required",
            ));
        }
        let mut request = self.client.post(endpoint(&self.dashboard_url, path)).json(
            &DashboardDeviceLinkRequest {
                link_session_id: &self.link_session_id,
                target_device_id: &self.target_device_id,
            },
        );
        if let Some(token) = access_token {
            if token.is_empty()
                || token.len() > 16 * 1024
                || token.trim() != token
                || token.chars().any(char::is_control)
            {
                return Err(link_error(
                    "device-link dashboard authentication is invalid",
                ));
            }
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .map_err(|_| link_error("device-link dashboard request failed"))?;
        if !response.status().is_success() {
            return Err(link_error(format!(
                "device-link dashboard rejected the request ({})",
                response.status().as_u16()
            )));
        }
        let bytes = bounded_response_bytes(response)?;
        serde_json::from_slice(&bytes)
            .map_err(|_| link_error("device-link dashboard returned an invalid response"))
    }

    fn validate_dashboard_response(
        &self,
        response: &DashboardDeviceLinkResponse,
    ) -> Result<(), FiniteChatCoreError> {
        if response.link_session_id != self.link_session_id
            || response.target_device_id != self.target_device_id
        {
            return Err(link_error(
                "device-link dashboard returned a mismatched response",
            ));
        }
        Ok(())
    }
}

fn validate_device_id(value: &str) -> Result<(), FiniteChatCoreError> {
    if value.is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value == "hosted-web"
    {
        Err(link_error("invalid device-link configuration"))
    } else {
        Ok(())
    }
}

fn normalize_base_url(value: &str) -> Result<String, FiniteChatCoreError> {
    let parsed =
        reqwest::Url::parse(value).map_err(|_| link_error("invalid device-link configuration"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(link_error("invalid device-link configuration"));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

fn is_loopback_http_url(value: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(value) else {
        return false;
    };
    parsed.scheme() == "http"
        && parsed.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        })
}

fn random_link_session_id() -> Result<String, FiniteChatCoreError> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|_| link_error("device-link entropy failed"))?;
    Ok(format!("link-{}", hex::encode(entropy)))
}

fn now_unix_seconds() -> Result<u64, FiniteChatCoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| link_error("invalid device-link configuration"))
}

fn endpoint(server_url: &str, path: &str) -> String {
    format!("{}{path}", server_url.trim_end_matches('/'))
}

fn post_json<I: Serialize, O: DeserializeOwned>(
    client: &Client,
    server_url: &str,
    path: &str,
    input: &I,
) -> Result<O, FiniteChatCoreError> {
    let response = client
        .post(endpoint(server_url, path))
        .json(input)
        .send()
        .map_err(|_| link_error("device-link request failed"))?;
    decode_response(response)
}

fn post_without_response<I: Serialize>(
    client: &Client,
    server_url: &str,
    path: &str,
    input: &I,
) -> Result<(), FiniteChatCoreError> {
    let response = client
        .post(endpoint(server_url, path))
        .json(input)
        .send()
        .map_err(|_| link_error("device-link request failed"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(link_error(format!(
            "device-link server rejected the request ({})",
            response.status().as_u16()
        )))
    }
}

fn decode_response<T: DeserializeOwned>(response: Response) -> Result<T, FiniteChatCoreError> {
    if !response.status().is_success() {
        return Err(link_error(format!(
            "device-link server rejected the request ({})",
            response.status().as_u16()
        )));
    }
    let bytes = bounded_response_bytes(response)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| link_error("device-link server returned an invalid response"))
}

fn bounded_response_bytes(response: Response) -> Result<Vec<u8>, FiniteChatCoreError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_LINK_RESPONSE_BYTES as u64)
    {
        return Err(link_error(
            "device-link server returned an invalid response",
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_LINK_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| link_error("device-link server returned an invalid response"))?;
    if bytes.len() > MAX_LINK_RESPONSE_BYTES {
        return Err(link_error(
            "device-link server returned an invalid response",
        ));
    }
    Ok(bytes)
}

fn link_error(reason: impl Into<String>) -> FiniteChatCoreError {
    FiniteChatCoreError::Client {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_link_configuration_rejects_credentials_queries_and_hosted_device_id() {
        assert!(normalize_base_url("https://chat.finite.test").is_ok());
        assert!(normalize_base_url("https://user@chat.finite.test").is_err());
        assert!(normalize_base_url("https://chat.finite.test?secret=value").is_err());
        assert!(validate_device_id("ios-alpha").is_ok());
        assert!(validate_device_id("hosted-web").is_err());
        assert!(validate_device_id(" ios-alpha").is_err());
        assert!(is_loopback_http_url("http://127.0.0.1:3000"));
        assert!(is_loopback_http_url("http://[::1]:3000"));
        assert!(!is_loopback_http_url("https://127.0.0.1:3000"));
        assert!(!is_loopback_http_url("http://127.evil.example:3000"));
    }
}
