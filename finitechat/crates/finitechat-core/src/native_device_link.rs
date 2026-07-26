use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use finitechat_http::{
    CreatePairingSessionRequest, ExpirePairingSessionRequest, GetPairingSessionRequest,
    HttpNipAbSourceDescriptorV1, HttpPairingSessionRecord, PublishPairingCompleteRequest,
    PublishPairingOfferRequest,
};
use nostr::Event;
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::FiniteChatCoreError;
use crate::native_authkit::NativeAuthKitSession;
use crate::nip_ab::{
    FinitePairingPayloadDecodeError, NIP_AB_VERSION, NipAbPayloadType, NipAbSourceDescriptorV1,
    NipAbTargetSession, decode_finite_pairing_payload_v2,
};

const MAX_PAIRING_RESPONSE_BYTES: usize = 128 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const PAIRING_POLL_INTERVAL: Duration = Duration::from_millis(400);
const ENROLLMENT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// A one-use native account pairing. The target ephemeral secret, WorkOS
/// token, source descriptor, and account secret never enter Swift view state.
#[derive(uniffi::Object)]
pub struct NativeDeviceLinkSession {
    client: Client,
    server_url: String,
    dashboard_url: String,
    target_device_id: String,
    pairing_session_id: String,
    deadline_unix_seconds: u64,
    state: Mutex<NativeDeviceLinkState>,
}

struct NativeDeviceLinkState {
    bootstrap: Option<crate::nip_ab::NipAbTargetBootstrap>,
    target: Option<NipAbTargetSession>,
    access_token: Option<String>,
    account_secret_hex: Option<Zeroizing<String>>,
    enrollment_grant: Option<NativeDeviceEnrollmentGrant>,
    acknowledged: bool,
}

#[derive(Serialize)]
struct DashboardPairingRequest<'a> {
    pairing_session_id: &'a str,
    target_device_id: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct NativeDeviceEnrollmentGrant {
    pub pairing_session_id: String,
    pub target_device_id: String,
    pub account_id: String,
    pub enrollment_user_id: String,
    pub enrollment_capability_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct NativeDeviceEnrollmentManifest {
    pub bootstrap_id: String,
    pub room_id: String,
    pub manifest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct NativeDeviceEnrollmentReady {
    pub room_count: u32,
    pub manifests: Vec<NativeDeviceEnrollmentManifest>,
}

#[derive(Serialize)]
struct DashboardEnrollmentRequest<'a> {
    pairing_session_id: &'a str,
    target_device_id: &'a str,
    enrollment_user_id: &'a str,
    enrollment_capability_hex: &'a str,
}

#[derive(Deserialize)]
struct DashboardPairingResponse {
    pairing_session_id: String,
    target_device_id: String,
    status: String,
    room_count: u32,
    active_room_count: u32,
    #[serde(default)]
    bootstrap_manifests: Vec<NativeDeviceEnrollmentManifest>,
    #[serde(default)]
    source_descriptor: Option<HttpNipAbSourceDescriptorV1>,
}

enum EnrollmentPollError {
    Retryable,
    Terminal(FiniteChatCoreError),
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
        let pairing_session_id = random_pairing_session_id()?;
        let bootstrap = NipAbTargetSession::prepare();
        let target_public_key = bootstrap.public_key();
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| pairing_error("pairing request failed"))?;
        let record: HttpPairingSessionRecord = post_json(
            &client,
            &server_url,
            "/pairing-sessions",
            &CreatePairingSessionRequest {
                version: NIP_AB_VERSION,
                pairing_session_id: pairing_session_id.clone(),
                target_device_id: target_device_id.clone(),
                target_public_key: target_public_key.clone(),
            },
        )?;
        if record.pairing_session_id != pairing_session_id
            || record.target_device_id != target_device_id
            || record.target_public_key != target_public_key
        {
            return Err(pairing_error(
                "pairing server returned a mismatched response",
            ));
        }

        Ok(Arc::new(Self {
            client,
            server_url,
            dashboard_url,
            target_device_id,
            pairing_session_id,
            deadline_unix_seconds: record.expires_at_unix_seconds,
            state: Mutex::new(NativeDeviceLinkState {
                bootstrap: Some(bootstrap),
                target: None,
                access_token: None,
                account_secret_hex: None,
                enrollment_grant: None,
                acknowledged: false,
            }),
        }))
    }

    pub fn approve_authenticated_account(
        &self,
        access_token: Option<String>,
    ) -> Result<(), FiniteChatCoreError> {
        validate_access_token(&self.dashboard_url, access_token.as_deref())?;
        let response = self.dashboard_post("/api/device-links/approve", access_token.as_deref())?;
        self.validate_dashboard_response(&response)?;
        let descriptor = response
            .source_descriptor
            .ok_or_else(|| pairing_error("pairing approval omitted its source descriptor"))?;
        let descriptor = NipAbSourceDescriptorV1 {
            version: descriptor.version,
            source_public_key: descriptor.source_public_key,
            session_secret_hex: descriptor.session_secret_hex,
            expires_at_unix_seconds: descriptor.expires_at_unix_seconds,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| FiniteChatCoreError::LockPoisoned)?;
        if state.target.is_none() {
            let bootstrap = state
                .bootstrap
                .take()
                .ok_or_else(|| pairing_error("pairing target is unavailable"))?;
            let (target, offer) =
                NipAbTargetSession::create(bootstrap, &descriptor, now_unix_seconds()?)
                    .map_err(|_| pairing_error("pairing descriptor failed authentication"))?;
            let _: HttpPairingSessionRecord = post_json(
                &self.client,
                &self.server_url,
                "/pairing-sessions/offer",
                &PublishPairingOfferRequest {
                    pairing_session_id: self.pairing_session_id.clone(),
                    offer_event: event_bytes(&offer)?,
                },
            )?;
            state.target = Some(target);
        }
        state.access_token = access_token;
        Ok(())
    }

    pub fn approve_with_authkit(
        &self,
        authkit: Arc<NativeAuthKitSession>,
    ) -> Result<(), FiniteChatCoreError> {
        self.approve_authenticated_account(Some(authkit.access_token()?))
    }

    /// Poll after authenticated approval. Swift receives only the validated
    /// account secret so it can commit it to Keychain.
    pub fn claim_account_secret(&self) -> Result<String, FiniteChatCoreError> {
        loop {
            let now = now_unix_seconds()?;
            if now >= self.deadline_unix_seconds {
                self.expire();
                return Err(pairing_error("pairing request expired"));
            }
            let access_token = self
                .state
                .lock()
                .map_err(|_| FiniteChatCoreError::LockPoisoned)?
                .access_token
                .clone();
            let _ = self.dashboard_post("/api/device-links/status", access_token.as_deref());
            let record: Option<HttpPairingSessionRecord> = post_json(
                &self.client,
                &self.server_url,
                "/pairing-sessions/get",
                &GetPairingSessionRequest {
                    pairing_session_id: self.pairing_session_id.clone(),
                },
            )?;
            let record = record.ok_or_else(|| pairing_error("pairing request was not found"))?;
            if let Some(result) = self.accept_pairing_response(&record, now)? {
                return Ok(result);
            }
            std::thread::sleep(PAIRING_POLL_INTERVAL);
        }
    }

    /// Call only after Keychain durably stores and reads back the exact secret.
    pub fn acknowledge_stored(&self) -> Result<(), FiniteChatCoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| FiniteChatCoreError::LockPoisoned)?;
        if state.acknowledged {
            return Ok(());
        }
        if state.account_secret_hex.is_none() {
            return Err(pairing_error("pairing payload has not been received"));
        }
        let complete = state
            .target
            .as_mut()
            .ok_or_else(|| pairing_error("pairing target is unavailable"))?
            .complete(now_unix_seconds()?)
            .map_err(|_| pairing_error("pairing completion failed"))?;
        let _: HttpPairingSessionRecord = post_json(
            &self.client,
            &self.server_url,
            "/pairing-sessions/complete",
            &PublishPairingCompleteRequest {
                pairing_session_id: self.pairing_session_id.clone(),
                complete_event: event_bytes(&complete)?,
            },
        )?;
        state.acknowledged = true;
        Ok(())
    }

    pub fn enrollment_grant(&self) -> Result<NativeDeviceEnrollmentGrant, FiniteChatCoreError> {
        self.state
            .lock()
            .map_err(|_| FiniteChatCoreError::LockPoisoned)?
            .enrollment_grant
            .clone()
            .ok_or_else(|| pairing_error("pairing payload has not been received"))
    }

    pub fn release(&self) {
        self.expire();
    }
}

impl NativeDeviceLinkSession {
    fn accept_pairing_response(
        &self,
        record: &HttpPairingSessionRecord,
        now: u64,
    ) -> Result<Option<String>, FiniteChatCoreError> {
        if record.pairing_session_id != self.pairing_session_id
            || record.target_device_id != self.target_device_id
        {
            return Err(pairing_error(
                "pairing server returned a mismatched response",
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| FiniteChatCoreError::LockPoisoned)?;
        if let Some(secret) = &state.account_secret_hex {
            return Ok(Some(secret.to_string()));
        }
        if record.events.len() < 3 {
            return Ok(None);
        }
        let confirmation = parse_event(&record.events[1].event)?;
        let payload_event = parse_event(&record.events[2].event)?;
        let target = state
            .target
            .as_mut()
            .ok_or_else(|| pairing_error("pairing target is unavailable"))?;
        target
            .accept_source_confirmation(&confirmation, now)
            .map_err(|_| pairing_error("pairing transcript failed authentication"))?;
        target
            .confirm_sas(now)
            .map_err(|_| pairing_error("pairing confirmation failed"))?;
        let (payload_type, encoded) = target
            .accept_payload(&payload_event, now)
            .map_err(|_| pairing_error("pairing payload failed authentication"))?;
        if payload_type != NipAbPayloadType::Custom {
            return Err(pairing_error("pairing payload type is invalid"));
        }
        let payload = decode_finite_pairing_payload_v2(&encoded).map_err(|error| match error {
            FinitePairingPayloadDecodeError::IncompatibleVersion => {
                pairing_error("pairing source is not compatible with this app version")
            }
            FinitePairingPayloadDecodeError::Invalid => pairing_error("pairing payload is invalid"),
        })?;
        payload
            .validate(
                &self.pairing_session_id,
                &self.target_device_id,
                &self.server_url,
                now,
            )
            .map_err(|_| pairing_error("pairing payload is invalid"))?;
        let secret = Zeroizing::new(payload.account_secret_hex.clone());
        let result = secret.to_string();
        state.enrollment_grant = Some(NativeDeviceEnrollmentGrant {
            pairing_session_id: payload.pairing_session_id.clone(),
            target_device_id: payload.target_device_id.clone(),
            account_id: payload.account_id.clone(),
            enrollment_user_id: payload.enrollment_user_id.clone(),
            enrollment_capability_hex: payload.enrollment_capability_hex.clone(),
        });
        state.account_secret_hex = Some(secret);
        Ok(Some(result))
    }

    fn dashboard_post(
        &self,
        path: &str,
        access_token: Option<&str>,
    ) -> Result<DashboardPairingResponse, FiniteChatCoreError> {
        validate_access_token(&self.dashboard_url, access_token)?;
        let mut request =
            self.client
                .post(endpoint(&self.dashboard_url, path))
                .json(&DashboardPairingRequest {
                    pairing_session_id: &self.pairing_session_id,
                    target_device_id: &self.target_device_id,
                });
        if let Some(token) = access_token {
            request = request.bearer_auth(token);
        }
        decode_response(
            request
                .send()
                .map_err(|_| pairing_error("pairing dashboard request failed"))?,
        )
    }

    fn validate_dashboard_response(
        &self,
        response: &DashboardPairingResponse,
    ) -> Result<(), FiniteChatCoreError> {
        if response.pairing_session_id != self.pairing_session_id
            || response.target_device_id != self.target_device_id
            || response.active_room_count > response.room_count
        {
            return Err(pairing_error(
                "pairing dashboard returned a mismatched response",
            ));
        }
        Ok(())
    }

    fn expire(&self) {
        let _ = post_without_response(
            &self.client,
            &self.server_url,
            "/pairing-sessions/expire",
            &ExpirePairingSessionRequest {
                pairing_session_id: self.pairing_session_id.clone(),
            },
        );
    }
}

#[derive(uniffi::Object)]
pub struct NativeDeviceEnrollmentSession {
    client: Client,
    dashboard_url: String,
    grant: NativeDeviceEnrollmentGrant,
}

#[uniffi::export]
impl NativeDeviceEnrollmentSession {
    #[uniffi::constructor]
    pub fn resume(
        dashboard_url: String,
        grant: NativeDeviceEnrollmentGrant,
    ) -> Result<Arc<Self>, FiniteChatCoreError> {
        let dashboard_url = normalize_base_url(&dashboard_url)?;
        validate_enrollment_grant(&grant)?;
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| pairing_error("enrollment request failed"))?;
        Ok(Arc::new(Self {
            client,
            dashboard_url,
            grant,
        }))
    }

    pub fn wait_until_ready(&self) -> Result<NativeDeviceEnrollmentReady, FiniteChatCoreError> {
        let deadline = Instant::now() + ENROLLMENT_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err(pairing_error(
                    "device enrollment timed out before complete history arrived",
                ));
            }
            match self.enrollment_post() {
                Ok(response) => {
                    self.validate_response(&response)?;
                    match response.status.as_str() {
                        "ready"
                            if response.room_count > 0
                                && response.bootstrap_manifests.len()
                                    == response.room_count as usize =>
                        {
                            return Ok(NativeDeviceEnrollmentReady {
                                room_count: response.room_count,
                                manifests: response.bootstrap_manifests,
                            });
                        }
                        "ready" => {
                            return Err(pairing_error(
                                "linked account has no available agent rooms",
                            ));
                        }
                        "awaiting_key_package" | "joining_rooms" => {}
                        _ => return Err(pairing_error("invalid enrollment response")),
                    }
                }
                Err(EnrollmentPollError::Retryable) => {}
                Err(EnrollmentPollError::Terminal(error)) => return Err(error),
            }
            std::thread::sleep(PAIRING_POLL_INTERVAL);
        }
    }
}

impl NativeDeviceEnrollmentSession {
    fn enrollment_post(&self) -> Result<DashboardPairingResponse, EnrollmentPollError> {
        let response = self
            .client
            .post(endpoint(&self.dashboard_url, "/api/device-links/enroll"))
            .json(&DashboardEnrollmentRequest {
                pairing_session_id: &self.grant.pairing_session_id,
                target_device_id: &self.grant.target_device_id,
                enrollment_user_id: &self.grant.enrollment_user_id,
                enrollment_capability_hex: &self.grant.enrollment_capability_hex,
            })
            .send()
            .map_err(|_| EnrollmentPollError::Retryable)?;
        let status = response.status();
        if enrollment_status_is_retryable(status) {
            return Err(EnrollmentPollError::Retryable);
        }
        if !status.is_success() {
            return Err(EnrollmentPollError::Terminal(pairing_error(format!(
                "enrollment dashboard rejected the request ({})",
                status.as_u16()
            ))));
        }
        let bytes = bounded_response_bytes(response).map_err(EnrollmentPollError::Terminal)?;
        serde_json::from_slice(&bytes).map_err(|_| {
            EnrollmentPollError::Terminal(pairing_error(
                "enrollment dashboard returned invalid JSON",
            ))
        })
    }

    fn validate_response(
        &self,
        response: &DashboardPairingResponse,
    ) -> Result<(), FiniteChatCoreError> {
        let manifests_valid = response.bootstrap_manifests.iter().all(|manifest| {
            !manifest.bootstrap_id.is_empty()
                && !manifest.room_id.is_empty()
                && manifest.manifest_sha256.len() == 64
                && manifest
                    .manifest_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        });
        let unique_manifests = response
            .bootstrap_manifests
            .iter()
            .map(|manifest| (manifest.bootstrap_id.as_str(), manifest.room_id.as_str()))
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == response.bootstrap_manifests.len();
        if response.pairing_session_id != self.grant.pairing_session_id
            || response.target_device_id != self.grant.target_device_id
            || response.active_room_count > response.room_count
            || !manifests_valid
            || !unique_manifests
            || (response.status == "ready"
                && response.bootstrap_manifests.len() != response.room_count as usize)
        {
            return Err(pairing_error(
                "enrollment dashboard returned a mismatched response",
            ));
        }
        Ok(())
    }
}

fn enrollment_status_is_retryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn validate_enrollment_grant(
    grant: &NativeDeviceEnrollmentGrant,
) -> Result<(), FiniteChatCoreError> {
    validate_token("pairing session", &grant.pairing_session_id)?;
    validate_device_id(&grant.target_device_id)?;
    if grant.account_id.len() != 64
        || !grant
            .account_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(pairing_error("invalid enrollment account"));
    }
    validate_token("enrollment user", &grant.enrollment_user_id)?;
    if grant.enrollment_capability_hex.len() != 64
        || !grant
            .enrollment_capability_hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(pairing_error("invalid enrollment capability"));
    }
    Ok(())
}

fn validate_device_id(value: &str) -> Result<(), FiniteChatCoreError> {
    if value.is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value == "hosted-web"
    {
        Err(pairing_error("invalid pairing configuration"))
    } else {
        Ok(())
    }
}

fn validate_token(field: &str, value: &str) -> Result<(), FiniteChatCoreError> {
    if value.is_empty()
        || value.len() > 512
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(pairing_error(format!("invalid {field}")))
    } else {
        Ok(())
    }
}

fn validate_access_token(
    dashboard_url: &str,
    access_token: Option<&str>,
) -> Result<(), FiniteChatCoreError> {
    if access_token.is_none() && !is_loopback_http_url(dashboard_url) {
        return Err(pairing_error(
            "pairing dashboard authentication is required",
        ));
    }
    if let Some(token) = access_token
        && (token.is_empty()
            || token.len() > 16 * 1024
            || token.trim() != token
            || token.chars().any(char::is_control))
    {
        return Err(pairing_error("pairing dashboard authentication is invalid"));
    }
    Ok(())
}

fn normalize_base_url(value: &str) -> Result<String, FiniteChatCoreError> {
    let parsed =
        reqwest::Url::parse(value).map_err(|_| pairing_error("invalid pairing configuration"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(pairing_error("invalid pairing configuration"));
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

fn random_pairing_session_id() -> Result<String, FiniteChatCoreError> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|_| pairing_error("pairing entropy failed"))?;
    Ok(format!("pair-{}", hex::encode(entropy)))
}

fn now_unix_seconds() -> Result<u64, FiniteChatCoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| pairing_error("invalid pairing configuration"))
}

fn endpoint(server_url: &str, path: &str) -> String {
    format!("{}{path}", server_url.trim_end_matches('/'))
}

fn event_bytes(event: &Event) -> Result<Vec<u8>, FiniteChatCoreError> {
    serde_json::to_vec(event).map_err(|_| pairing_error("pairing event serialization failed"))
}

fn parse_event(bytes: &[u8]) -> Result<Event, FiniteChatCoreError> {
    serde_json::from_slice(bytes).map_err(|_| pairing_error("pairing event is invalid"))
}

fn post_json<I: Serialize, O: DeserializeOwned>(
    client: &Client,
    server_url: &str,
    path: &str,
    input: &I,
) -> Result<O, FiniteChatCoreError> {
    decode_response(
        client
            .post(endpoint(server_url, path))
            .json(input)
            .send()
            .map_err(|_| pairing_error("pairing request failed"))?,
    )
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
        .map_err(|_| pairing_error("pairing request failed"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(pairing_error(format!(
            "pairing server rejected the request ({})",
            response.status().as_u16()
        )))
    }
}

fn decode_response<O: DeserializeOwned>(response: Response) -> Result<O, FiniteChatCoreError> {
    let status = response.status();
    let bytes = bounded_response_bytes(response)?;
    if !status.is_success() {
        return Err(pairing_error(format!(
            "pairing server rejected the request ({})",
            status.as_u16()
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| pairing_error("pairing server returned invalid JSON"))
}

fn bounded_response_bytes(response: Response) -> Result<Vec<u8>, FiniteChatCoreError> {
    let mut bytes = Vec::new();
    response
        .take((MAX_PAIRING_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| pairing_error("pairing response failed"))?;
    if bytes.len() > MAX_PAIRING_RESPONSE_BYTES {
        return Err(pairing_error("pairing response is too large"));
    }
    Ok(bytes)
}

fn pairing_error(message: impl Into<String>) -> FiniteChatCoreError {
    FiniteChatCoreError::Client {
        reason: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NativeDeviceEnrollmentGrant, enrollment_status_is_retryable, validate_enrollment_grant,
    };
    use reqwest::StatusCode;

    #[test]
    fn enrollment_retries_only_throttling_and_server_failures() {
        for status in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert!(enrollment_status_is_retryable(status), "{status}");
        }
        for status in [
            StatusCode::OK,
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::CONFLICT,
            StatusCode::GONE,
        ] {
            assert!(!enrollment_status_is_retryable(status), "{status}");
        }
    }

    #[test]
    fn enrollment_grant_is_exact_and_lowercase() {
        let valid = NativeDeviceEnrollmentGrant {
            pairing_session_id: "pair-test".to_owned(),
            target_device_id: "ios-test".to_owned(),
            account_id: "11".repeat(32),
            enrollment_user_id: "user_test".to_owned(),
            enrollment_capability_hex: "ab".repeat(32),
        };
        validate_enrollment_grant(&valid).unwrap();
        assert!(
            validate_enrollment_grant(&NativeDeviceEnrollmentGrant {
                enrollment_capability_hex: "AB".repeat(32),
                ..valid.clone()
            })
            .is_err()
        );
        assert!(
            validate_enrollment_grant(&NativeDeviceEnrollmentGrant {
                enrollment_user_id: " user_test".to_owned(),
                ..valid
            })
            .is_err()
        );
    }
}
