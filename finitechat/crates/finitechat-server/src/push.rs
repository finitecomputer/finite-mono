use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use finitechat_http::{
    AckPushWakeRequest, AckPushWakeResponse, ClaimPushWakesRequest, ClaimPushWakesResponse,
    FailPushWakeRequest, FailPushWakeResponse, PushPlatform, PushTokenRecord, PushWakeDelivery,
    PushWakePayload, RemovePushTokenRequest, RemovePushTokenResponse,
};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use thiserror::Error;

const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8787";
const DEFAULT_BATCH_LIMIT: usize = 25;
const DEFAULT_LEASE_MS: u64 = 30_000;
const DEFAULT_INTERVAL_MS: u64 = 1_000;

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
            topic: optional_value(topic),
            team_id: optional_value(team_id),
            key_id: optional_value(key_id),
            private_key_path: optional_value(private_key_path),
            base_url,
        },
        once,
        interval_ms,
        limit,
        lease_ms,
    })
}

pub fn run_push_drain(command: PushDrainCommand) -> Result<(), PushDrainError> {
    eprintln!(
        "finitechat-server: push-drain is running with a stub APNs sender; \
         wake-only pushes are NOT delivered to Apple"
    );
    let mut api = HttpPushWakeApi::new(command.server_url.clone());
    let mut sender = ApnsStubSender::new(command.apns.clone());

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
    // `Delivered` and `InvalidToken` are only produced by a real APNs
    // sender; the stub below always reports `Retry`. They are exercised by
    // the drain tests and by the future `a2` integration.
    // TODO: adopt the `a2` crate before shipping push.
    #[allow(dead_code)]
    Delivered,
    #[allow(dead_code)]
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

/// Placeholder APNs sender used while push is unshipped. It never contacts
/// Apple: each claimed wake is logged and reported as retryable so the
/// push-wake outbox is never falsely acked and no token is ever pruned on a
/// delivery that did not happen.
// TODO: adopt the `a2` crate before shipping push.
pub struct ApnsStubSender {
    #[allow(dead_code)]
    options: ApnsOptions,
}

impl ApnsStubSender {
    pub fn new(options: ApnsOptions) -> Self {
        Self { options }
    }
}

impl ApnsSender for ApnsStubSender {
    fn send_push(
        &mut self,
        token: &PushTokenRecord,
        payload: &PushWakePayload,
    ) -> Result<ApnsSendOutcome, PushDrainError> {
        eprintln!(
            "finitechat-server: stub APNs sender skipping wake push \
             device={:?} platform={:?} room_id={} seq={} (push not shipped)",
            token.device, token.platform, payload.room_id, payload.seq
        );
        Ok(ApnsSendOutcome::Retry)
    }
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

fn optional_value(value: Option<String>) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_default()
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
    fn stub_sender_never_reports_delivery_or_invalid_token() {
        let bob = DeviceRef::new("bob", "phone");
        let mut sender = ApnsStubSender::new(ApnsOptions {
            topic: "computer.finite.finitechat".to_owned(),
            team_id: String::new(),
            key_id: String::new(),
            private_key_path: String::new(),
            base_url: "https://api.sandbox.push.apple.com".to_owned(),
        });

        let outcome = sender
            .send_push(
                &apns_token(&bob, "token-bob"),
                &PushWakePayload {
                    room_id: "room-main".to_owned(),
                    seq: 42,
                },
            )
            .unwrap();

        assert_eq!(outcome, ApnsSendOutcome::Retry);
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
}
