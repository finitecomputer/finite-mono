use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use finitechat_core::nip_ab::{
    FinitePairingPayloadV1, NIP_AB_SESSION_TTL_SECONDS, NIP_AB_VERSION, NipAbPayloadType,
    NipAbSourceDescriptorV1, NipAbTargetSession,
};
use finitechat_http::{
    CreatePairingSessionRequest, ExpirePairingSessionRequest, GetPairingSessionRequest,
    HttpPairingSessionRecord, PublishPairingCompleteRequest, PublishPairingOfferRequest,
};
use nostr::Event;
use reqwest::{Client, Response, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

const MAX_PAIRING_RESPONSE_BYTES: usize = 128 * 1024;
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
            timeout: Duration::from_secs(NIP_AB_SESSION_TTL_SECONDS),
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeviceLinkReady {
    pub event: &'static str,
    pub pairing_session_id: String,
    pub target_device_id: String,
}

/// The authenticated dashboard returns this object through a supervisor-only
/// pipe. It must never be passed through argv, stdout, stderr, or the renderer.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairingSourceDescriptor {
    pub version: u16,
    pub source_public_key: String,
    pub session_secret_hex: String,
    pub expires_at_unix_seconds: u64,
}

pub struct PendingDeviceLinkSession {
    client: Client,
    server_url: String,
    target_device_id: String,
    pairing_session_id: String,
    bootstrap: crate::device_link::TargetBootstrap,
    deadline_unix_seconds: u64,
    poll_interval: Duration,
    ready: DeviceLinkReady,
}

/// Private helper wrapper so the ephemeral target key has no accidental
/// Debug/Serialize surface in the public daemon state.
mod private {
    pub struct TargetBootstrap(pub finitechat_core::nip_ab::NipAbTargetBootstrap);
}
use private::TargetBootstrap;

pub struct WaitingDeviceLinkSession {
    client: Client,
    server_url: String,
    target_device_id: String,
    pairing_session_id: String,
    target: NipAbTargetSession,
    deadline_unix_seconds: u64,
    poll_interval: Duration,
}

pub struct ClaimedDeviceLink {
    client: Client,
    server_url: String,
    pairing_session_id: String,
    target: NipAbTargetSession,
    account_secret_hex: Zeroizing<String>,
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
        || options.timeout > Duration::from_secs(NIP_AB_SESSION_TTL_SECONDS)
    {
        return Err(DeviceLinkBootstrapError::InvalidConfiguration);
    }
    if options.poll_interval.is_zero() {
        options.poll_interval = DEFAULT_POLL_INTERVAL;
    }
    let bootstrap = NipAbTargetSession::prepare();
    let target_public_key = bootstrap.public_key();
    let pairing_session_id = random_pairing_session_id()?;
    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| DeviceLinkBootstrapError::Request)?;
    let record: HttpPairingSessionRecord = post_json(
        &client,
        &server_url,
        "/pairing-sessions",
        &CreatePairingSessionRequest {
            version: NIP_AB_VERSION,
            pairing_session_id: pairing_session_id.clone(),
            target_device_id: options.target_device_id.clone(),
            target_public_key: target_public_key.clone(),
        },
    )
    .await?;
    if record.pairing_session_id != pairing_session_id
        || record.target_device_id != options.target_device_id
        || record.target_public_key != target_public_key
    {
        return Err(DeviceLinkBootstrapError::InvalidResponse);
    }
    let ready = DeviceLinkReady {
        event: "pairing_ready",
        pairing_session_id: pairing_session_id.clone(),
        target_device_id: options.target_device_id.clone(),
    };
    Ok(PendingDeviceLinkSession {
        client,
        server_url,
        target_device_id: options.target_device_id,
        pairing_session_id,
        bootstrap: TargetBootstrap(bootstrap),
        deadline_unix_seconds: record.expires_at_unix_seconds,
        poll_interval: options.poll_interval,
        ready,
    })
}

impl PendingDeviceLinkSession {
    pub fn ready(&self) -> &DeviceLinkReady {
        &self.ready
    }

    pub async fn accept_source_descriptor(
        self,
        descriptor: PairingSourceDescriptor,
    ) -> Result<WaitingDeviceLinkSession, DeviceLinkBootstrapError> {
        let descriptor = NipAbSourceDescriptorV1 {
            version: descriptor.version,
            source_public_key: descriptor.source_public_key,
            session_secret_hex: descriptor.session_secret_hex,
            expires_at_unix_seconds: descriptor.expires_at_unix_seconds,
        };
        let (target, offer) =
            NipAbTargetSession::create(self.bootstrap.0, &descriptor, now_unix_seconds()?)
                .map_err(|_| DeviceLinkBootstrapError::PayloadRejected)?;
        let _: HttpPairingSessionRecord = post_json(
            &self.client,
            &self.server_url,
            "/pairing-sessions/offer",
            &PublishPairingOfferRequest {
                pairing_session_id: self.pairing_session_id.clone(),
                offer_event: event_bytes(&offer)?,
            },
        )
        .await?;
        Ok(WaitingDeviceLinkSession {
            client: self.client,
            server_url: self.server_url,
            target_device_id: self.target_device_id,
            pairing_session_id: self.pairing_session_id,
            target,
            deadline_unix_seconds: self.deadline_unix_seconds,
            poll_interval: self.poll_interval,
        })
    }
}

impl WaitingDeviceLinkSession {
    pub async fn wait_for_claim(mut self) -> Result<ClaimedDeviceLink, DeviceLinkBootstrapError> {
        loop {
            let now = now_unix_seconds()?;
            if now >= self.deadline_unix_seconds {
                self.expire().await;
                return Err(DeviceLinkBootstrapError::Expired);
            }
            let record: Option<HttpPairingSessionRecord> = post_json(
                &self.client,
                &self.server_url,
                "/pairing-sessions/get",
                &GetPairingSessionRequest {
                    pairing_session_id: self.pairing_session_id.clone(),
                },
            )
            .await?;
            let Some(record) = record else {
                return Err(DeviceLinkBootstrapError::InvalidResponse);
            };
            if record.pairing_session_id != self.pairing_session_id
                || record.target_device_id != self.target_device_id
            {
                return Err(DeviceLinkBootstrapError::InvalidResponse);
            }
            if record.events.len() < 3 {
                tokio::time::sleep(self.poll_interval).await;
                continue;
            }
            let confirmation = parse_event(&record.events[1].event)?;
            let payload_event = parse_event(&record.events[2].event)?;
            self.target
                .accept_source_confirmation(&confirmation, now)
                .and_then(|_| self.target.confirm_sas(now).map(|_| ()))
                .map_err(|_| DeviceLinkBootstrapError::PayloadRejected)?;
            let (kind, encoded) = self
                .target
                .accept_payload(&payload_event, now)
                .map_err(|_| DeviceLinkBootstrapError::PayloadRejected)?;
            if kind != NipAbPayloadType::Custom {
                return Err(DeviceLinkBootstrapError::PayloadRejected);
            }
            let payload: FinitePairingPayloadV1 = serde_json::from_str(&encoded)
                .map_err(|_| DeviceLinkBootstrapError::PayloadRejected)?;
            payload
                .validate(
                    &self.pairing_session_id,
                    &self.target_device_id,
                    &self.server_url,
                    now,
                )
                .map_err(|_| DeviceLinkBootstrapError::PayloadRejected)?;
            return Ok(ClaimedDeviceLink {
                client: self.client,
                server_url: self.server_url,
                pairing_session_id: self.pairing_session_id,
                target: self.target,
                account_secret_hex: Zeroizing::new(payload.account_secret_hex.clone()),
                deadline_unix_seconds: self.deadline_unix_seconds,
                poll_interval: self.poll_interval,
            });
        }
    }

    async fn expire(&self) {
        let _ = post_without_response(
            &self.client,
            &self.server_url,
            "/pairing-sessions/expire",
            &ExpirePairingSessionRequest {
                pairing_session_id: self.pairing_session_id.clone(),
            },
        )
        .await;
    }
}

impl ClaimedDeviceLink {
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

    pub async fn acknowledge_stored(mut self) -> Result<(), DeviceLinkBootstrapError> {
        let complete = self
            .target
            .complete(now_unix_seconds()?)
            .map_err(|_| DeviceLinkBootstrapError::PayloadRejected)?;
        let request = PublishPairingCompleteRequest {
            pairing_session_id: self.pairing_session_id,
            complete_event: event_bytes(&complete)?,
        };
        loop {
            if now_unix_seconds()? >= self.deadline_unix_seconds {
                return Err(DeviceLinkBootstrapError::Expired);
            }
            match post_json::<_, HttpPairingSessionRecord>(
                &self.client,
                &self.server_url,
                "/pairing-sessions/complete",
                &request,
            )
            .await
            {
                Ok(_) => return Ok(()),
                Err(DeviceLinkBootstrapError::Request)
                | Err(DeviceLinkBootstrapError::ServerStatus(500..=599)) => {
                    tokio::time::sleep(self.poll_interval).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub async fn release(self) {
        let _ = post_without_response(
            &self.client,
            &self.server_url,
            "/pairing-sessions/expire",
            &ExpirePairingSessionRequest {
                pairing_session_id: self.pairing_session_id,
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
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(DeviceLinkBootstrapError::InvalidConfiguration);
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

fn random_pairing_session_id() -> Result<String, DeviceLinkBootstrapError> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|_| DeviceLinkBootstrapError::Entropy)?;
    Ok(format!("pair-{}", hex::encode(entropy)))
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

fn event_bytes(event: &Event) -> Result<Vec<u8>, DeviceLinkBootstrapError> {
    serde_json::to_vec(event).map_err(|_| DeviceLinkBootstrapError::InvalidResponse)
}

fn parse_event(bytes: &[u8]) -> Result<Event, DeviceLinkBootstrapError> {
    serde_json::from_slice(bytes).map_err(|_| DeviceLinkBootstrapError::InvalidResponse)
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
    response: Response,
) -> Result<T, DeviceLinkBootstrapError> {
    let status = response.status();
    if !status.is_success() {
        return Err(DeviceLinkBootstrapError::ServerStatus(status.as_u16()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| DeviceLinkBootstrapError::Request)?;
    if bytes.len() > MAX_PAIRING_RESPONSE_BYTES {
        return Err(DeviceLinkBootstrapError::InvalidResponse);
    }
    serde_json::from_slice(&bytes).map_err(|_| DeviceLinkBootstrapError::InvalidResponse)
}
