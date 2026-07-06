use std::fs;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use finitechat_http::{
    AckPushWakeRequest, AckPushWakeResponse, ClaimPushWakesRequest, ClaimPushWakesResponse,
    FailPushWakeRequest, FailPushWakeResponse, PushPlatform, PushTokenRecord, PushWakeDelivery,
    PushWakePayload, RemovePushTokenRequest, RemovePushTokenResponse,
};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::DecodePrivateKey;
use reqwest::StatusCode;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use thiserror::Error;

const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8787";
const DEFAULT_BATCH_LIMIT: usize = 25;
const DEFAULT_LEASE_MS: u64 = 30_000;
const DEFAULT_INTERVAL_MS: u64 = 1_000;
const APNS_TOKEN_CACHE_SECONDS: u64 = 20 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushDrainCommand {
    pub server_url: String,
    pub apns: ApnsOptions,
    pub once: bool,
    pub interval_ms: u64,
    pub limit: usize,
    pub lease_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApnsOptions {
    pub topic: String,
    pub team_id: String,
    pub key_id: String,
    pub private_key_path: String,
    pub base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushDrainReport {
    pub claimed: usize,
    pub tokens_sent: usize,
    pub wakes_acked: usize,
    pub wakes_failed: usize,
    pub stale_tokens_removed: usize,
    pub unsupported_tokens: usize,
}

#[derive(Debug, Error)]
pub enum PushDrainError {
    #[error("missing required push-drain option: {0}")]
    MissingOption(&'static str),
    #[error("invalid push-drain option: {0}")]
    InvalidOption(String),
    #[error("push API request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("push API returned {status}: {body}")]
    HttpStatus { status: StatusCode, body: String },
    #[error("failed to read APNs private key: {0}")]
    PrivateKeyRead(std::io::Error),
    #[error("invalid APNs private key: {0}")]
    PrivateKey(String),
    #[error("failed to encode APNs provider token: {0}")]
    ProviderToken(String),
    #[error("system clock is before unix epoch")]
    Clock,
}

pub fn parse_push_drain_command(args: &[String]) -> Result<PushDrainCommand, PushDrainError> {
    let mut server_url = std::env::var("FINITECHAT_PUSH_SERVER_URL").ok();
    let mut topic = std::env::var("FINITECHAT_APNS_TOPIC").ok();
    let mut team_id = std::env::var("FINITECHAT_APNS_TEAM_ID").ok();
    let mut key_id = std::env::var("FINITECHAT_APNS_KEY_ID").ok();
    let mut private_key_path = std::env::var("FINITECHAT_APNS_PRIVATE_KEY_PATH").ok();
    let mut apns_base_url = std::env::var("FINITECHAT_APNS_BASE_URL").ok();
    let mut apns_environment = std::env::var("FINITECHAT_APNS_ENV").ok();
    let mut once = false;
    let mut interval_ms = DEFAULT_INTERVAL_MS;
    let mut limit = DEFAULT_BATCH_LIMIT;
    let mut lease_ms = DEFAULT_LEASE_MS;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--server-url" => server_url = Some(take_value(args, &mut index, "--server-url")?),
            "--apns-topic" => topic = Some(take_value(args, &mut index, "--apns-topic")?),
            "--apns-team-id" => team_id = Some(take_value(args, &mut index, "--apns-team-id")?),
            "--apns-key-id" => key_id = Some(take_value(args, &mut index, "--apns-key-id")?),
            "--apns-private-key" => {
                private_key_path = Some(take_value(args, &mut index, "--apns-private-key")?)
            }
            "--apns-base-url" => {
                apns_base_url = Some(take_value(args, &mut index, "--apns-base-url")?)
            }
            "--apns-env" => apns_environment = Some(take_value(args, &mut index, "--apns-env")?),
            "--once" => once = true,
            "--interval-ms" => {
                interval_ms = parse_u64(
                    "interval-ms",
                    &take_value(args, &mut index, "--interval-ms")?,
                )?
            }
            "--limit" => limit = parse_usize("limit", &take_value(args, &mut index, "--limit")?)?,
            "--lease-ms" => {
                lease_ms = parse_u64("lease-ms", &take_value(args, &mut index, "--lease-ms")?)?
            }
            value => {
                return Err(PushDrainError::InvalidOption(format!(
                    "unknown push-drain option '{value}'"
                )));
            }
        }
        index += 1;
    }

    let base_url =
        apns_base_url.unwrap_or_else(|| match apns_environment.as_deref().unwrap_or("sandbox") {
            "production" | "prod" => "https://api.push.apple.com".to_owned(),
            _ => "https://api.sandbox.push.apple.com".to_owned(),
        });

    Ok(PushDrainCommand {
        server_url: server_url.unwrap_or_else(|| DEFAULT_SERVER_URL.to_owned()),
        apns: ApnsOptions {
            topic: require_option(topic, "apns-topic or FINITECHAT_APNS_TOPIC")?,
            team_id: require_option(team_id, "apns-team-id or FINITECHAT_APNS_TEAM_ID")?,
            key_id: require_option(key_id, "apns-key-id or FINITECHAT_APNS_KEY_ID")?,
            private_key_path: require_option(
                private_key_path,
                "apns-private-key or FINITECHAT_APNS_PRIVATE_KEY_PATH",
            )?,
            base_url,
        },
        once,
        interval_ms,
        limit,
        lease_ms,
    })
}

pub fn run_push_drain(command: PushDrainCommand) -> Result<(), PushDrainError> {
    let private_key_pem = fs::read_to_string(&command.apns.private_key_path)
        .map_err(PushDrainError::PrivateKeyRead)?;
    let mut api = HttpPushWakeApi::new(command.server_url.clone());
    let mut sender = ApnsHttpSender::new(command.apns.clone(), private_key_pem)?;

    loop {
        let report = drain_push_wakes_once(
            &mut api,
            &mut sender,
            DrainOnceOptions {
                now_ms: current_unix_millis()?,
                lease_ms: command.lease_ms,
                limit: command.limit,
            },
        )?;
        if report.claimed > 0 {
            println!(
                "finitechat-server: push drain claimed={} sent={} acked={} failed={} stale_removed={} unsupported={}",
                report.claimed,
                report.tokens_sent,
                report.wakes_acked,
                report.wakes_failed,
                report.stale_tokens_removed,
                report.unsupported_tokens
            );
        }
        if command.once {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(command.interval_ms));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainOnceOptions {
    pub now_ms: u64,
    pub lease_ms: u64,
    pub limit: usize,
}

pub fn drain_push_wakes_once<A, S>(
    api: &mut A,
    sender: &mut S,
    options: DrainOnceOptions,
) -> Result<PushDrainReport, PushDrainError>
where
    A: PushWakeApi,
    S: ApnsSender,
{
    let claimed = api.claim_push_wakes(ClaimPushWakesRequest {
        now_ms: options.now_ms,
        lease_ms: options.lease_ms,
        limit: options.limit,
    })?;
    let mut report = PushDrainReport {
        claimed: claimed.wakes.len(),
        tokens_sent: 0,
        wakes_acked: 0,
        wakes_failed: 0,
        stale_tokens_removed: 0,
        unsupported_tokens: 0,
    };

    for wake in claimed.wakes {
        let outcome = deliver_wake(api, sender, &wake, &mut report)?;
        match outcome {
            WakeDeliveryOutcome::Ack => {
                api.ack_push_wake(AckPushWakeRequest {
                    wake_id: wake.wake_id,
                })?;
                report.wakes_acked += 1;
            }
            WakeDeliveryOutcome::Fail => {
                api.fail_push_wake(FailPushWakeRequest {
                    wake_id: wake.wake_id,
                })?;
                report.wakes_failed += 1;
            }
        }
    }

    Ok(report)
}

fn deliver_wake<A, S>(
    api: &mut A,
    sender: &mut S,
    wake: &PushWakeDelivery,
    report: &mut PushDrainReport,
) -> Result<WakeDeliveryOutcome, PushDrainError>
where
    A: PushWakeApi,
    S: ApnsSender,
{
    let mut should_fail = false;
    for token in &wake.tokens {
        if token.platform != PushPlatform::Apns {
            report.unsupported_tokens += 1;
            continue;
        }
        match sender.send_push(token, &wake.payload)? {
            ApnsSendOutcome::Delivered => {
                report.tokens_sent += 1;
            }
            ApnsSendOutcome::InvalidToken => {
                api.remove_push_token(RemovePushTokenRequest {
                    device: token.device.clone(),
                    token: Some(token.token.clone()),
                })?;
                report.stale_tokens_removed += 1;
            }
            ApnsSendOutcome::Retry => {
                should_fail = true;
            }
        }
    }
    if should_fail {
        Ok(WakeDeliveryOutcome::Fail)
    } else {
        Ok(WakeDeliveryOutcome::Ack)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WakeDeliveryOutcome {
    Ack,
    Fail,
}

pub trait PushWakeApi {
    fn claim_push_wakes(
        &mut self,
        request: ClaimPushWakesRequest,
    ) -> Result<ClaimPushWakesResponse, PushDrainError>;
    fn ack_push_wake(
        &mut self,
        request: AckPushWakeRequest,
    ) -> Result<AckPushWakeResponse, PushDrainError>;
    fn fail_push_wake(
        &mut self,
        request: FailPushWakeRequest,
    ) -> Result<FailPushWakeResponse, PushDrainError>;
    fn remove_push_token(
        &mut self,
        request: RemovePushTokenRequest,
    ) -> Result<RemovePushTokenResponse, PushDrainError>;
}

pub trait ApnsSender {
    fn send_push(
        &mut self,
        token: &PushTokenRecord,
        payload: &PushWakePayload,
    ) -> Result<ApnsSendOutcome, PushDrainError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApnsSendOutcome {
    Delivered,
    InvalidToken,
    Retry,
}

pub struct HttpPushWakeApi {
    server_url: String,
    client: reqwest::blocking::Client,
}

impl HttpPushWakeApi {
    pub fn new(server_url: String) -> Self {
        Self {
            server_url,
            client: reqwest::blocking::Client::new(),
        }
    }

    fn post_json<T, R>(&mut self, path: &str, body: &T) -> Result<R, PushDrainError>
    where
        T: serde::Serialize,
        R: DeserializeOwned,
    {
        let response = self
            .client
            .post(format!(
                "{}/{}",
                self.server_url.trim_end_matches('/'),
                path.trim_start_matches('/')
            ))
            .json(body)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text()?;
            return Err(PushDrainError::HttpStatus { status, body });
        }
        response.json().map_err(PushDrainError::Http)
    }
}

impl PushWakeApi for HttpPushWakeApi {
    fn claim_push_wakes(
        &mut self,
        request: ClaimPushWakesRequest,
    ) -> Result<ClaimPushWakesResponse, PushDrainError> {
        self.post_json("/push-wakes/claim", &request)
    }

    fn ack_push_wake(
        &mut self,
        request: AckPushWakeRequest,
    ) -> Result<AckPushWakeResponse, PushDrainError> {
        self.post_json("/push-wakes/ack", &request)
    }

    fn fail_push_wake(
        &mut self,
        request: FailPushWakeRequest,
    ) -> Result<FailPushWakeResponse, PushDrainError> {
        self.post_json("/push-wakes/fail", &request)
    }

    fn remove_push_token(
        &mut self,
        request: RemovePushTokenRequest,
    ) -> Result<RemovePushTokenResponse, PushDrainError> {
        self.post_json("/push-tokens/remove", &request)
    }
}

pub struct ApnsHttpSender {
    options: ApnsOptions,
    signer: ApnsProviderTokenSigner,
    client: reqwest::blocking::Client,
}

impl ApnsHttpSender {
    pub fn new(options: ApnsOptions, private_key_pem: String) -> Result<Self, PushDrainError> {
        Ok(Self {
            signer: ApnsProviderTokenSigner::new(
                options.team_id.clone(),
                options.key_id.clone(),
                private_key_pem,
            )?,
            options,
            client: reqwest::blocking::Client::builder().build()?,
        })
    }
}

impl ApnsSender for ApnsHttpSender {
    fn send_push(
        &mut self,
        token: &PushTokenRecord,
        payload: &PushWakePayload,
    ) -> Result<ApnsSendOutcome, PushDrainError> {
        let now_seconds = current_unix_seconds()?;
        let provider_token = self.signer.provider_token(now_seconds)?;
        let response = self
            .client
            .post(format!(
                "{}/3/device/{}",
                self.options.base_url.trim_end_matches('/'),
                token.token
            ))
            .bearer_auth(provider_token)
            .header("apns-topic", &self.options.topic)
            .header("apns-push-type", "background")
            .header("apns-priority", "5")
            .header("apns-expiration", "0")
            .json(&json!({
                "aps": {
                    "content-available": 1,
                },
                "room_id": payload.room_id,
                "seq": payload.seq,
            }))
            .send()?;

        let status = response.status();
        if status.is_success() {
            return Ok(ApnsSendOutcome::Delivered);
        }

        let body = response.text()?;
        let reason = apns_error_reason(&body);
        eprintln!(
            "finitechat-server: APNs push rejected status={} reason={}",
            status,
            reason.as_deref().unwrap_or("unknown")
        );
        if is_invalid_apns_token(status, reason.as_deref()) {
            return Ok(ApnsSendOutcome::InvalidToken);
        }
        Ok(ApnsSendOutcome::Retry)
    }
}

struct ApnsProviderTokenSigner {
    team_id: String,
    key_id: String,
    signing_key: SigningKey,
    cached: Option<CachedProviderToken>,
}

struct CachedProviderToken {
    issued_at_seconds: u64,
    token: String,
}

impl ApnsProviderTokenSigner {
    fn new(
        team_id: String,
        key_id: String,
        private_key_pem: String,
    ) -> Result<Self, PushDrainError> {
        let signing_key = SigningKey::from_pkcs8_pem(&private_key_pem)
            .map_err(|error| PushDrainError::PrivateKey(error.to_string()))?;
        Ok(Self {
            team_id,
            key_id,
            signing_key,
            cached: None,
        })
    }

    fn provider_token(&mut self, now_seconds: u64) -> Result<String, PushDrainError> {
        if let Some(cached) = &self.cached
            && now_seconds.saturating_sub(cached.issued_at_seconds) < APNS_TOKEN_CACHE_SECONDS
        {
            return Ok(cached.token.clone());
        }

        let header = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "alg": "ES256",
                "kid": self.key_id,
            }))
            .map_err(|error| PushDrainError::ProviderToken(error.to_string()))?,
        );
        let claims = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "iss": self.team_id,
                "iat": now_seconds,
            }))
            .map_err(|error| PushDrainError::ProviderToken(error.to_string()))?,
        );
        let signing_input = format!("{header}.{claims}");
        let signature: Signature = self.signing_key.sign(signing_input.as_bytes());
        let token = format!(
            "{}.{}",
            signing_input,
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        );
        self.cached = Some(CachedProviderToken {
            issued_at_seconds: now_seconds,
            token: token.clone(),
        });
        Ok(token)
    }
}

#[derive(Deserialize)]
struct ApnsErrorBody {
    reason: String,
}

fn apns_error_reason(body: &str) -> Option<String> {
    serde_json::from_str::<ApnsErrorBody>(body)
        .ok()
        .map(|body| body.reason)
}

fn is_invalid_apns_token(status: StatusCode, reason: Option<&str>) -> bool {
    status == StatusCode::GONE
        || matches!(
            reason,
            Some("BadDeviceToken" | "DeviceTokenNotForTopic" | "Unregistered")
        )
}

fn current_unix_seconds() -> Result<u64, PushDrainError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PushDrainError::Clock)?
        .as_secs())
}

fn current_unix_millis() -> Result<u64, PushDrainError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PushDrainError::Clock)?;
    Ok(duration.as_millis().try_into().unwrap_or(u64::MAX))
}

fn take_value(
    args: &[String],
    index: &mut usize,
    option: &'static str,
) -> Result<String, PushDrainError> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or(PushDrainError::MissingOption(option))
}

fn require_option(value: Option<String>, name: &'static str) -> Result<String, PushDrainError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or(PushDrainError::MissingOption(name))
}

fn parse_u64(name: &str, value: &str) -> Result<u64, PushDrainError> {
    value
        .parse()
        .map_err(|_| PushDrainError::InvalidOption(format!("{name} must be an integer")))
}

fn parse_usize(name: &str, value: &str) -> Result<usize, PushDrainError> {
    value
        .parse()
        .map_err(|_| PushDrainError::InvalidOption(format!("{name} must be an integer")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use finitechat_proto::DeviceRef;

    #[test]
    fn drain_success_acks_after_apns_delivery() {
        let bob = DeviceRef::new("bob", "phone");
        let wake = wake("wake-1", &[apns_token(&bob, "token-bob")]);
        let mut api = FakePushWakeApi::with_wakes(vec![wake]);
        let mut apns = FakeApnsSender::with_outcomes(vec![ApnsSendOutcome::Delivered]);

        let report = drain_push_wakes_once(&mut api, &mut apns, test_options()).unwrap();

        assert_eq!(report.claimed, 1);
        assert_eq!(report.tokens_sent, 1);
        assert_eq!(report.wakes_acked, 1);
        assert_eq!(api.acked, vec!["wake-1"]);
        assert!(api.failed.is_empty());
    }

    #[test]
    fn drain_retryable_apns_error_fails_wake_for_retry() {
        let bob = DeviceRef::new("bob", "phone");
        let wake = wake("wake-1", &[apns_token(&bob, "token-bob")]);
        let mut api = FakePushWakeApi::with_wakes(vec![wake]);
        let mut apns = FakeApnsSender::with_outcomes(vec![ApnsSendOutcome::Retry]);

        let report = drain_push_wakes_once(&mut api, &mut apns, test_options()).unwrap();

        assert_eq!(report.wakes_failed, 1);
        assert_eq!(api.failed, vec!["wake-1"]);
        assert!(api.acked.is_empty());
    }

    #[test]
    fn drain_invalid_apns_token_removes_with_token_guard_and_acks() {
        let bob = DeviceRef::new("bob", "phone");
        let wake = wake("wake-1", &[apns_token(&bob, "stale-token")]);
        let mut api = FakePushWakeApi::with_wakes(vec![wake]);
        let mut apns = FakeApnsSender::with_outcomes(vec![ApnsSendOutcome::InvalidToken]);

        let report = drain_push_wakes_once(&mut api, &mut apns, test_options()).unwrap();

        assert_eq!(report.stale_tokens_removed, 1);
        assert_eq!(report.wakes_acked, 1);
        assert_eq!(api.removed.len(), 1);
        assert_eq!(api.removed[0].device, bob);
        assert_eq!(api.removed[0].token.as_deref(), Some("stale-token"));
        assert_eq!(api.acked, vec!["wake-1"]);
    }

    #[test]
    fn drain_empty_wake_is_acked() {
        let wake = wake("wake-1", &[]);
        let mut api = FakePushWakeApi::with_wakes(vec![wake]);
        let mut apns = FakeApnsSender::with_outcomes(vec![]);

        let report = drain_push_wakes_once(&mut api, &mut apns, test_options()).unwrap();

        assert_eq!(report.claimed, 1);
        assert_eq!(report.wakes_acked, 1);
        assert!(apns.sent.is_empty());
    }

    #[test]
    fn provider_token_is_three_base64url_segments_and_cached() {
        let mut signer = ApnsProviderTokenSigner::new(
            "TEAMID1234".to_owned(),
            "KEYID12345".to_owned(),
            TEST_P8.to_owned(),
        )
        .unwrap();

        let first = signer.provider_token(1_800_000_000).unwrap();
        let second = signer.provider_token(1_800_000_100).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.split('.').count(), 3);
        assert!(!first.contains('='));
    }

    fn wake(wake_id: &str, tokens: &[PushTokenRecord]) -> PushWakeDelivery {
        PushWakeDelivery {
            wake_id: wake_id.to_owned(),
            payload: PushWakePayload {
                room_id: "room-main".to_owned(),
                seq: 42,
            },
            tokens: tokens.to_vec(),
            attempt: 1,
        }
    }

    fn apns_token(device: &DeviceRef, token: &str) -> PushTokenRecord {
        PushTokenRecord {
            device: device.clone(),
            platform: PushPlatform::Apns,
            token: token.to_owned(),
        }
    }

    fn test_options() -> DrainOnceOptions {
        DrainOnceOptions {
            now_ms: 1_000,
            lease_ms: 30_000,
            limit: 10,
        }
    }

    struct FakePushWakeApi {
        wakes: Vec<PushWakeDelivery>,
        acked: Vec<String>,
        failed: Vec<String>,
        removed: Vec<RemovePushTokenRequest>,
    }

    impl FakePushWakeApi {
        fn with_wakes(wakes: Vec<PushWakeDelivery>) -> Self {
            Self {
                wakes,
                acked: Vec::new(),
                failed: Vec::new(),
                removed: Vec::new(),
            }
        }
    }

    impl PushWakeApi for FakePushWakeApi {
        fn claim_push_wakes(
            &mut self,
            _request: ClaimPushWakesRequest,
        ) -> Result<ClaimPushWakesResponse, PushDrainError> {
            Ok(ClaimPushWakesResponse {
                wakes: std::mem::take(&mut self.wakes),
            })
        }

        fn ack_push_wake(
            &mut self,
            request: AckPushWakeRequest,
        ) -> Result<AckPushWakeResponse, PushDrainError> {
            self.acked.push(request.wake_id);
            Ok(AckPushWakeResponse { acked: true })
        }

        fn fail_push_wake(
            &mut self,
            request: FailPushWakeRequest,
        ) -> Result<FailPushWakeResponse, PushDrainError> {
            self.failed.push(request.wake_id);
            Ok(FailPushWakeResponse {
                retry: true,
                dropped: false,
            })
        }

        fn remove_push_token(
            &mut self,
            request: RemovePushTokenRequest,
        ) -> Result<RemovePushTokenResponse, PushDrainError> {
            self.removed.push(request);
            Ok(RemovePushTokenResponse { removed: true })
        }
    }

    struct FakeApnsSender {
        outcomes: Vec<ApnsSendOutcome>,
        sent: Vec<(String, PushWakePayload)>,
    }

    impl FakeApnsSender {
        fn with_outcomes(outcomes: Vec<ApnsSendOutcome>) -> Self {
            Self {
                outcomes,
                sent: Vec::new(),
            }
        }
    }

    impl ApnsSender for FakeApnsSender {
        fn send_push(
            &mut self,
            token: &PushTokenRecord,
            payload: &PushWakePayload,
        ) -> Result<ApnsSendOutcome, PushDrainError> {
            self.sent.push((token.token.clone(), payload.clone()));
            if self.outcomes.is_empty() {
                return Ok(ApnsSendOutcome::Delivered);
            }
            Ok(self.outcomes.remove(0))
        }
    }

    const TEST_P8: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg3iykXJjuhFiWUwSZ
TfZjMF0SuiTsuQdMeyzW9M6eF+uhRANCAAQ0AtC+LJJMXAWCAwOU+J81knblk6yP
qpVrzfDboa8rkcoCLHTyHbj7zEW4FEhAnjAIto4vX/U85oNfiYV57sve
-----END PRIVATE KEY-----"#;
}
