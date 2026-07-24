use std::io::{self, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use finitechat_core::device_link::{
    DEVICE_LINK_MAX_TTL_SECONDS, DeviceLinkDecryptInput, DeviceLinkPairingKey,
    create_device_link_pairing_key, decrypt_device_link_payload,
};
use finitechat_http::{
    AckLinkPayloadRequest, AckLinkPayloadResponse, ClaimLinkPayloadRequest,
    ClaimLinkPayloadResponse, CreateLinkSessionRequest, ErrorResponse, ExpireLinkSessionRequest,
    ReleaseLinkClaimRequest,
};
use reqwest::{Client, StatusCode, Url};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

const MAX_LINK_RESPONSE_BYTES: usize = 64 * 1024;
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(400);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct DeviceLinkBootstrapOptions {
    pub server_url: String,
    pub target_device_id: String,
    pub timeout: Duration,
    pub poll_interval: Duration,
}

impl DeviceLinkBootstrapOptions {
    pub fn internal_alpha(
        server_url: impl Into<String>,
        target_device_id: impl Into<String>,
    ) -> Self {
        Self {
            server_url: server_url.into(),
            target_device_id: target_device_id.into(),
            timeout: Duration::from_secs(DEVICE_LINK_MAX_TTL_SECONDS),
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeviceLinkReady {
    pub event: &'static str,
    pub link_session_id: String,
    pub target_device_id: String,
}

/// A created one-use rendezvous. The pairing secret deliberately has no
/// Debug/Serialize surface and never enters stdout, renderer state, or argv.
pub struct PendingDeviceLinkSession {
    client: Client,
    server_url: String,
    target_device_id: String,
    link_session_id: String,
    pairing: DeviceLinkPairingKey,
    deadline_unix_seconds: u64,
    poll_interval: Duration,
    ready: DeviceLinkReady,
}

/// A claimed and authenticated account bootstrap awaiting confirmation that
/// Electron main stored it with `safeStorage`. Only then may the rendezvous be
/// acknowledged as delivered.
pub struct ClaimedDeviceLink {
    client: Client,
    server_url: String,
    link_session_id: String,
    claim_token: String,
    account_secret_hex: String,
    deadline_unix_seconds: u64,
    poll_interval: Duration,
}

#[derive(Debug, Error)]
pub enum DeviceLinkBootstrapError {
    #[error("invalid device-link configuration")]
    InvalidConfiguration,
    #[error("device-link entropy generation failed")]
    Entropy,
    #[error("device-link server request failed")]
    Request,
    #[error("device-link server rejected the request ({0})")]
    ServerStatus(u16),
    #[error("device-link server returned an invalid response")]
    InvalidResponse,
    #[error("device-link request expired")]
    Expired,
    #[error("device-link payload failed authentication")]
    PayloadRejected,
    #[error("device-link result pipe failed")]
    ResultPipe,
}

pub async fn create_device_link_session(
    mut options: DeviceLinkBootstrapOptions,
) -> Result<PendingDeviceLinkSession, DeviceLinkBootstrapError> {
    let server_url = normalize_base_url(&options.server_url)?;
    validate_device_id(&options.target_device_id)?;
    if options.timeout.is_zero()
        || options.timeout > Duration::from_secs(DEVICE_LINK_MAX_TTL_SECONDS)
    {
        return Err(DeviceLinkBootstrapError::InvalidConfiguration);
    }
    if options.poll_interval.is_zero() {
        options.poll_interval = DEFAULT_POLL_INTERVAL;
    }

    let pairing = create_device_link_pairing_key();
    let link_session_id = random_link_session_id()?;
    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| DeviceLinkBootstrapError::Request)?;
    let _: finitechat_http::HttpLinkSessionRecord = post_json(
        &client,
        &server_url,
        "/link-sessions",
        &CreateLinkSessionRequest {
            link_session_id: link_session_id.clone(),
            pairing_public_key: pairing.public_key_hex.clone(),
        },
    )
    .await?;

    let deadline_unix_seconds = now_unix_seconds()?
        .checked_add(options.timeout.as_secs())
        .ok_or(DeviceLinkBootstrapError::InvalidConfiguration)?;
    let ready = DeviceLinkReady {
        event: "link_ready",
        link_session_id: link_session_id.clone(),
        target_device_id: options.target_device_id.clone(),
    };

    Ok(PendingDeviceLinkSession {
        client,
        server_url,
        target_device_id: options.target_device_id,
        link_session_id,
        pairing,
        deadline_unix_seconds,
        poll_interval: options.poll_interval,
        ready,
    })
}

impl PendingDeviceLinkSession {
    pub fn ready(&self) -> &DeviceLinkReady {
        &self.ready
    }

    pub async fn wait_for_claim(self) -> Result<ClaimedDeviceLink, DeviceLinkBootstrapError> {
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
                )
                .await;
                return Err(DeviceLinkBootstrapError::Expired);
            }

            let response = match self
                .client
                .post(endpoint(&self.server_url, "/link-sessions/claim"))
                .json(&ClaimLinkPayloadRequest {
                    link_session_id: self.link_session_id.clone(),
                })
                .send()
                .await
            {
                Ok(response) => response,
                Err(_) => {
                    tokio::time::sleep(self.poll_interval).await;
                    continue;
                }
            };
            if response.status() == StatusCode::CONFLICT || response.status().is_server_error() {
                tokio::time::sleep(self.poll_interval).await;
                continue;
            }
            if response.status() == StatusCode::BAD_REQUEST {
                let status = response.status();
                let bytes = bounded_response_bytes(response).await?;
                if serde_json::from_slice::<ErrorResponse>(&bytes)
                    .is_ok_and(|error| error.kind == "link_session_not_ready")
                {
                    tokio::time::sleep(self.poll_interval).await;
                    continue;
                }
                return Err(DeviceLinkBootstrapError::ServerStatus(status.as_u16()));
            }
            let claimed: ClaimLinkPayloadResponse = decode_response(response).await?;
            let payload = decrypt_device_link_payload(DeviceLinkDecryptInput {
                pairing_secret_key_hex: self.pairing.secret_key_hex,
                encrypted_payload: claimed.encrypted_payload,
                expected_link_session_id: self.link_session_id.clone(),
                expected_pairing_public_key: self.pairing.public_key_hex,
                expected_target_device_id: self.target_device_id,
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
                    )
                    .await;
                    return Err(DeviceLinkBootstrapError::PayloadRejected);
                }
            };
            return Ok(ClaimedDeviceLink {
                client: self.client,
                server_url: self.server_url,
                link_session_id: self.link_session_id,
                claim_token: claimed.claim_token,
                account_secret_hex: payload.account_secret_hex,
                deadline_unix_seconds: self.deadline_unix_seconds,
                poll_interval: self.poll_interval,
            });
        }
    }
}

impl ClaimedDeviceLink {
    /// Write the plaintext account bootstrap only to the supervisor-owned
    /// private result pipe. This method must never receive stdout/stderr.
    pub fn write_secret_result(
        &self,
        mut writer: impl Write,
    ) -> Result<(), DeviceLinkBootstrapError> {
        #[derive(Serialize)]
        struct SecretResult<'a> {
            account_secret: &'a str,
        }
        serde_json::to_writer(
            &mut writer,
            &SecretResult {
                account_secret: &self.account_secret_hex,
            },
        )
        .map_err(|_| DeviceLinkBootstrapError::ResultPipe)?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|_| DeviceLinkBootstrapError::ResultPipe)
    }

    pub async fn acknowledge_stored(self) -> Result<(), DeviceLinkBootstrapError> {
        let request = AckLinkPayloadRequest {
            link_session_id: self.link_session_id,
            claim_token: self.claim_token,
        };
        loop {
            if now_unix_seconds()? >= self.deadline_unix_seconds {
                return Err(DeviceLinkBootstrapError::Expired);
            }
            let response = self
                .client
                .post(endpoint(&self.server_url, "/link-sessions/ack"))
                .json(&request)
                .send()
                .await;
            let response = match response {
                Ok(response) if response.status().is_server_error() => {
                    tokio::time::sleep(self.poll_interval).await;
                    continue;
                }
                Ok(response) => response,
                Err(_) => {
                    tokio::time::sleep(self.poll_interval).await;
                    continue;
                }
            };
            let response: AckLinkPayloadResponse = decode_response(response).await?;
            return if response.acked {
                Ok(())
            } else {
                Err(DeviceLinkBootstrapError::InvalidResponse)
            };
        }
    }

    pub async fn release(self) {
        let _ = post_without_response(
            &self.client,
            &self.server_url,
            "/link-sessions/release",
            &ReleaseLinkClaimRequest {
                link_session_id: self.link_session_id,
            },
        )
        .await;
    }
}

fn validate_device_id(value: &str) -> Result<(), DeviceLinkBootstrapError> {
    if value.is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value == "hosted-web"
    {
        Err(DeviceLinkBootstrapError::InvalidConfiguration)
    } else {
        Ok(())
    }
}

fn normalize_base_url(value: &str) -> Result<String, DeviceLinkBootstrapError> {
    let parsed = Url::parse(value).map_err(|_| DeviceLinkBootstrapError::InvalidConfiguration)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(DeviceLinkBootstrapError::InvalidConfiguration);
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

fn random_link_session_id() -> Result<String, DeviceLinkBootstrapError> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|_| DeviceLinkBootstrapError::Entropy)?;
    Ok(format!("link-{}", hex::encode(entropy)))
}

fn now_unix_seconds() -> Result<u64, DeviceLinkBootstrapError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| DeviceLinkBootstrapError::InvalidConfiguration)
}

fn endpoint(server_url: &str, path: &str) -> String {
    format!("{}{path}", server_url.trim_end_matches('/'))
}

async fn post_json<I: Serialize, O: DeserializeOwned>(
    client: &Client,
    server_url: &str,
    path: &str,
    input: &I,
) -> Result<O, DeviceLinkBootstrapError> {
    let response = client
        .post(endpoint(server_url, path))
        .json(input)
        .send()
        .await
        .map_err(|_| DeviceLinkBootstrapError::Request)?;
    decode_response(response).await
}

async fn post_without_response<I: Serialize>(
    client: &Client,
    server_url: &str,
    path: &str,
    input: &I,
) -> Result<(), DeviceLinkBootstrapError> {
    let response = client
        .post(endpoint(server_url, path))
        .json(input)
        .send()
        .await
        .map_err(|_| DeviceLinkBootstrapError::Request)?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(DeviceLinkBootstrapError::ServerStatus(
            response.status().as_u16(),
        ))
    }
}

async fn decode_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, DeviceLinkBootstrapError> {
    if !response.status().is_success() {
        return Err(DeviceLinkBootstrapError::ServerStatus(
            response.status().as_u16(),
        ));
    }
    let bytes = bounded_response_bytes(response).await?;
    serde_json::from_slice(&bytes).map_err(|_| DeviceLinkBootstrapError::InvalidResponse)
}

async fn bounded_response_bytes(
    response: reqwest::Response,
) -> Result<Vec<u8>, DeviceLinkBootstrapError> {
    let bytes = response
        .bytes()
        .await
        .map_err(|_| DeviceLinkBootstrapError::InvalidResponse)?;
    if bytes.len() > MAX_LINK_RESPONSE_BYTES {
        return Err(DeviceLinkBootstrapError::InvalidResponse);
    }
    Ok(bytes.to_vec())
}

impl From<io::Error> for DeviceLinkBootstrapError {
    fn from(_: io::Error) -> Self {
        Self::ResultPipe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finitechat_core::device_link::{DeviceLinkEncryptInput, encrypt_device_link_payload};
    use finitechat_http::UploadLinkPayloadRequest;
    use finitechat_server::{HttpServerState, http_router};

    #[test]
    fn public_ready_record_contains_only_rendezvous_coordinates() {
        let ready = DeviceLinkReady {
            event: "link_ready",
            link_session_id: "link-public-test".to_owned(),
            target_device_id: "electron-public-test".to_owned(),
        };
        let encoded = serde_json::to_value(ready).unwrap();
        assert_eq!(encoded["event"], "link_ready");
        assert_eq!(encoded.as_object().unwrap().len(), 3);
        assert!(encoded.get("account_secret").is_none());
        assert!(encoded.get("pairing_secret").is_none());
        assert!(encoded.get("pairing_public_key").is_none());
    }

    #[tokio::test]
    async fn claim_poll_waits_for_delayed_automatic_approval() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test link server");
        let address = listener.local_addr().expect("test link address");
        let server_url = format!("http://{address}");
        let server = tokio::spawn(async move {
            axum::serve(listener, http_router(HttpServerState::default()))
                .await
                .expect("serve test link server");
        });

        let mut options = DeviceLinkBootstrapOptions::internal_alpha(
            server_url.clone(),
            "electron-delayed-approval",
        );
        options.timeout = Duration::from_secs(5);
        options.poll_interval = Duration::from_millis(20);
        let pending = create_device_link_session(options)
            .await
            .expect("create pending link");
        let session_id = pending.link_session_id.clone();
        let pairing_public_key = pending.pairing.public_key_hex.clone();
        let expires_at = pending.deadline_unix_seconds;
        let waiter = tokio::spawn(pending.wait_for_claim());

        // Let more than one normal Created -> not-ready poll happen before the
        // automatic dashboard approval uploads its encrypted payload.
        tokio::time::sleep(Duration::from_millis(75)).await;
        let account_secret_hex = "11".repeat(32);
        let encrypted_payload = encrypt_device_link_payload(DeviceLinkEncryptInput {
            account_secret_hex: account_secret_hex.clone(),
            pairing_public_key,
            link_session_id: session_id.clone(),
            target_device_id: "electron-delayed-approval".to_owned(),
            server_url: server_url.clone(),
            issued_at_unix_seconds: now_unix_seconds().expect("test clock"),
            expires_at_unix_seconds: expires_at,
        })
        .expect("encrypt delayed approval");
        let response = Client::new()
            .post(endpoint(&server_url, "/link-sessions/payload"))
            .json(&UploadLinkPayloadRequest {
                link_session_id: session_id,
                encrypted_payload,
            })
            .send()
            .await
            .expect("upload delayed approval");
        assert!(response.status().is_success());

        let claimed = waiter
            .await
            .expect("claim waiter task")
            .expect("claim after delayed approval");
        assert_eq!(claimed.account_secret_hex, account_secret_hex);
        claimed.release().await;
        server.abort();
    }

    #[test]
    fn secret_result_is_written_only_to_the_explicit_private_writer() {
        let secret = "0000000000000000000000000000000000000000000000000000000000000003";
        let claimed = ClaimedDeviceLink {
            client: Client::new(),
            server_url: "https://chat.finite.test".to_owned(),
            link_session_id: "link-private-test".to_owned(),
            claim_token: "claim-private-test".to_owned(),
            account_secret_hex: secret.to_owned(),
            deadline_unix_seconds: u64::MAX,
            poll_interval: DEFAULT_POLL_INTERVAL,
        };
        let mut private_pipe = Vec::new();
        claimed.write_secret_result(&mut private_pipe).unwrap();
        let decoded: serde_json::Value = serde_json::from_slice(&private_pipe).unwrap();
        assert_eq!(decoded["account_secret"], secret);
        assert_eq!(decoded.as_object().unwrap().len(), 1);
    }

    #[test]
    fn link_configuration_rejects_credentials_queries_and_hosted_device_id() {
        assert!(normalize_base_url("https://chat.finite.test").is_ok());
        assert!(normalize_base_url("https://user@chat.finite.test").is_err());
        assert!(normalize_base_url("https://chat.finite.test?secret=value").is_err());
        assert!(validate_device_id("electron-alpha").is_ok());
        assert!(validate_device_id("hosted-web").is_err());
        assert!(validate_device_id(" electron-alpha").is_err());
    }
}
