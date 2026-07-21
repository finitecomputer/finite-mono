use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, timeout};

const USAGE_FORMULA_VERSION: &str = "2026-05-26.v1";
const DEFAULT_MODEL: &str = "glm-5-2";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct LimiterConfig {
    pub finite_usage_api_url: String,
    pub finite_usage_api_service_key: String,
    pub upstream_base_url: String,
    pub vllm_internal_api_key: String,
    pub default_model: String,
    pub dashboard_url: String,
    pub upstream_health_path: String,
    pub usage_api_health_path: String,
    pub readiness_timeout: Duration,
    pub usage_api_timeout: Duration,
    pub upstream_first_byte_timeout: Duration,
    pub upstream_body_timeout: Duration,
    pub upstream_stream_idle_timeout: Duration,
    pub watchdog: WatchdogConfig,
}

impl LimiterConfig {
    pub fn new(
        finite_usage_api_url: String,
        finite_usage_api_service_key: String,
        upstream_base_url: String,
        vllm_internal_api_key: String,
        dashboard_url: String,
    ) -> Self {
        Self {
            finite_usage_api_url,
            finite_usage_api_service_key,
            upstream_base_url,
            vllm_internal_api_key,
            default_model: DEFAULT_MODEL.to_string(),
            dashboard_url,
            upstream_health_path: "/health".to_string(),
            usage_api_health_path: "/internal/finite-private/v1/health".to_string(),
            readiness_timeout: Duration::from_secs(5),
            usage_api_timeout: Duration::from_secs(15),
            upstream_first_byte_timeout: Duration::from_secs(120),
            upstream_body_timeout: Duration::from_secs(600),
            upstream_stream_idle_timeout: Duration::from_secs(600),
            watchdog: WatchdogConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    pub enabled: bool,
    pub interval: Duration,
    pub failure_threshold: u32,
    pub restart_command: Option<String>,
    pub exit_after_failures: bool,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: Duration::from_secs(30),
            failure_threshold: 3,
            restart_command: None,
            exit_after_failures: false,
        }
    }
}

#[derive(Clone)]
struct AppState {
    config: Arc<LimiterConfig>,
    client: Client,
}

#[derive(Debug, thiserror::Error)]
pub enum LimiterConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{0} must be greater than zero")]
    InvalidDuration(&'static str),
    #[error("failed to build HTTP client: {0}")]
    HttpClient(String),
}

pub fn app(config: LimiterConfig) -> Result<Router, LimiterConfigError> {
    validate_config(&config)?;
    let state = AppState {
        config: Arc::new(config),
        client: Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .build()
            .map_err(|error| LimiterConfigError::HttpClient(error.to_string()))?,
    };
    if state.config.watchdog.enabled {
        spawn_watchdog(state.clone());
    }
    Ok(Router::new()
        .route("/live", get(live))
        .route("/health", get(health))
        .route("/ready", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/chat/completions", post(proxy_openai))
        .route("/v1/responses", post(proxy_openai))
        .with_state(state))
}

fn validate_config(config: &LimiterConfig) -> Result<(), LimiterConfigError> {
    for (name, value) in [
        ("FINITE_USAGE_API_URL", &config.finite_usage_api_url),
        (
            "FINITE_USAGE_API_SERVICE_KEY",
            &config.finite_usage_api_service_key,
        ),
        ("UPSTREAM_BASE_URL", &config.upstream_base_url),
        ("VLLM_INTERNAL_API_KEY", &config.vllm_internal_api_key),
        ("default_model", &config.default_model),
        ("DASHBOARD_URL", &config.dashboard_url),
    ] {
        if value.trim().is_empty() {
            return Err(LimiterConfigError::Missing(name));
        }
    }
    for (name, value) in [
        ("readiness_timeout", config.readiness_timeout),
        ("usage_api_timeout", config.usage_api_timeout),
        (
            "upstream_first_byte_timeout",
            config.upstream_first_byte_timeout,
        ),
        ("upstream_body_timeout", config.upstream_body_timeout),
        (
            "upstream_stream_idle_timeout",
            config.upstream_stream_idle_timeout,
        ),
    ] {
        if value.is_zero() {
            return Err(LimiterConfigError::InvalidDuration(name));
        }
    }
    if config.watchdog.enabled {
        if config.watchdog.interval.is_zero() {
            return Err(LimiterConfigError::InvalidDuration("watchdog.interval"));
        }
        if config.watchdog.failure_threshold == 0 {
            return Err(LimiterConfigError::InvalidDuration(
                "watchdog.failure_threshold",
            ));
        }
    }
    Ok(())
}

async fn live(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "service": "finite-private-limiter",
        "kind": "live",
        "checkedAtUnixMs": unix_millis(),
        "config": public_config_snapshot(&state.config)
    }))
}

async fn metrics() -> Response {
    (
        [("content-type", "text/plain; version=0.0.4")],
        "finite_private_limiter_live 1\n",
    )
        .into_response()
}

async fn health(State(state): State<AppState>) -> Response {
    let snapshot = readiness_snapshot(&state).await;
    let status = if snapshot.ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(snapshot)).into_response()
}

async fn readiness_snapshot(state: &AppState) -> ReadinessSnapshot {
    let upstream_target = join_url(
        &state.config.upstream_base_url,
        &state.config.upstream_health_path,
    );
    let usage_api_target = join_url(
        &state.config.finite_usage_api_url,
        &state.config.usage_api_health_path,
    );
    let upstream_check = check_component(
        &state.client,
        "upstream",
        upstream_target,
        None,
        state.config.readiness_timeout,
    );
    let usage_api_check = check_component(
        &state.client,
        "usage_api",
        usage_api_target,
        Some(&state.config.finite_usage_api_service_key),
        state.config.readiness_timeout,
    );
    let (upstream, usage_api) = tokio::join!(upstream_check, usage_api_check);
    ReadinessSnapshot {
        ok: upstream.ok && usage_api.ok,
        service: "finite-private-limiter",
        kind: "ready",
        checked_at_unix_ms: unix_millis(),
        config: public_config_snapshot(&state.config),
        components: ReadinessComponents {
            upstream,
            usage_api,
        },
    }
}

async fn check_component(
    client: &Client,
    name: &'static str,
    url: String,
    bearer_token: Option<&str>,
    timeout_duration: Duration,
) -> ComponentCheck {
    let started = Instant::now();
    let mut request = client.get(url.clone());
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    match timeout(timeout_duration, request.send()).await {
        Ok(Ok(response)) => {
            let status = response.status();
            ComponentCheck {
                ok: status.is_success(),
                name,
                target_url: url,
                authenticated: bearer_token.is_some(),
                status: Some(status.as_u16()),
                latency_ms: elapsed_millis(started),
                timeout_ms: timeout_duration.as_millis(),
                error: None,
            }
        }
        Ok(Err(error)) => ComponentCheck {
            ok: false,
            name,
            target_url: url,
            authenticated: bearer_token.is_some(),
            status: None,
            latency_ms: elapsed_millis(started),
            timeout_ms: timeout_duration.as_millis(),
            error: Some(error.to_string()),
        },
        Err(_) => ComponentCheck {
            ok: false,
            name,
            target_url: url,
            authenticated: bearer_token.is_some(),
            status: None,
            latency_ms: elapsed_millis(started),
            timeout_ms: timeout_duration.as_millis(),
            error: Some(format!(
                "timed out after {}ms",
                timeout_duration.as_millis()
            )),
        },
    }
}

fn public_config_snapshot(config: &LimiterConfig) -> PublicConfigSnapshot {
    PublicConfigSnapshot {
        default_model: config.default_model.clone(),
        upstream_base_url: config.upstream_base_url.clone(),
        upstream_health_path: config.upstream_health_path.clone(),
        usage_api_base_url: config.finite_usage_api_url.clone(),
        usage_api_health_path: config.usage_api_health_path.clone(),
        readiness_timeout_ms: config.readiness_timeout.as_millis(),
        usage_api_timeout_ms: config.usage_api_timeout.as_millis(),
        upstream_first_byte_timeout_ms: config.upstream_first_byte_timeout.as_millis(),
        upstream_body_timeout_ms: config.upstream_body_timeout.as_millis(),
        upstream_stream_idle_timeout_ms: config.upstream_stream_idle_timeout.as_millis(),
        required_secrets: RequiredSecretsSnapshot {
            finite_usage_api_service_key_present: !config.finite_usage_api_service_key.is_empty(),
            vllm_internal_api_key_present: !config.vllm_internal_api_key.is_empty(),
        },
    }
}

fn spawn_watchdog(state: AppState) {
    tokio::spawn(async move {
        let mut consecutive_failures = 0u32;
        loop {
            sleep(state.config.watchdog.interval).await;
            let snapshot = readiness_snapshot(&state).await;
            if snapshot.ok {
                consecutive_failures = 0;
                continue;
            }
            consecutive_failures = consecutive_failures.saturating_add(1);
            eprintln!(
                "finite-private-limiter watchdog readiness failure {}/{}: upstream={:?} usage_api={:?}",
                consecutive_failures,
                state.config.watchdog.failure_threshold,
                snapshot.components.upstream.error,
                snapshot.components.usage_api.error
            );
            if consecutive_failures < state.config.watchdog.failure_threshold {
                continue;
            }
            if let Some(command) = state.config.watchdog.restart_command.as_deref() {
                if let Err(error) = run_watchdog_command(command) {
                    eprintln!("finite-private-limiter watchdog command failed: {error}");
                }
                consecutive_failures = 0;
            }
            if state.config.watchdog.exit_after_failures {
                eprintln!(
                    "finite-private-limiter watchdog exiting after repeated readiness failures"
                );
                std::process::exit(70);
            }
        }
    });
}

fn run_watchdog_command(command: &str) -> Result<(), String> {
    let status = std::process::Command::new("sh")
        .arg("-lc")
        .arg(command)
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command exited with {status}"))
    }
}

async fn proxy_openai(
    State(state): State<AppState>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(presented_api_key) = bearer_token(&headers) else {
        return openai_error(
            StatusCode::UNAUTHORIZED,
            "Finite Private API key is required.",
            "invalid_api_key",
            "missing_authorization",
        );
    };
    let is_streaming = request_is_streaming(&body);
    if uri.path() == "/v1/responses" && is_streaming {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "Finite Private streaming /v1/responses requests are not supported yet.",
            "invalid_request_error",
            "unsupported_streaming_endpoint",
        );
    }

    let request_id = new_request_id();
    let estimate = estimate_usage(&body, &state.config.default_model);
    let reserve = ReserveRequest {
        request_id: request_id.clone(),
        presented_api_key,
        endpoint: uri.path().to_string(),
        model: estimate.model.clone(),
        estimated_prompt_tokens: estimate.prompt_tokens,
        estimated_completion_tokens: estimate.completion_tokens,
        estimated_usage_units: estimate.usage_units,
        usage_formula_version: USAGE_FORMULA_VERSION.to_string(),
        dashboard_url: state.config.dashboard_url.clone(),
    };

    let reserve_decision = match reserve_usage(&state, &reserve).await {
        Ok(decision) => decision,
        Err(error) => {
            eprintln!("finite-private-limiter reserve failed: {error}");
            return openai_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Finite Private usage admission is unavailable.",
                "usage_api_unavailable",
                "usage_api_unavailable",
            );
        }
    };

    if reserve_decision.decision != "allow" {
        return denied_response(reserve_decision);
    }

    let Some(reservation_id) = reserve_decision.reservation_id.clone() else {
        return openai_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Finite Private usage admission did not return a reservation.",
            "usage_api_invalid_response",
            "usage_api_invalid_response",
        );
    };

    let upstream_body = upstream_body_for_request(&uri, body.clone());
    if is_streaming {
        let upstream = match call_upstream_response(&state, &uri, upstream_body).await {
            Ok(response) => response,
            Err(error) => {
                eprintln!("finite-private-limiter upstream failed: {error}");
                let _ = settle_usage_with_retries(
                    &state,
                    &reservation_id,
                    SettleRequest {
                        request_id,
                        settlement: "estimate".to_string(),
                        prompt_tokens: None,
                        completion_tokens: None,
                        usage_units: None,
                        usage_formula_version: USAGE_FORMULA_VERSION.to_string(),
                        upstream_status: Some(502),
                        upstream_error_class: Some("upstream_unavailable".to_string()),
                    },
                )
                .await;
                return openai_error(
                    StatusCode::BAD_GATEWAY,
                    "Finite Private upstream is unavailable.",
                    "upstream_unavailable",
                    "upstream_unavailable",
                );
            }
        };
        return streaming_response(state, upstream, reservation_id, request_id);
    }

    let upstream = match call_upstream(&state, &uri, upstream_body).await {
        Ok(response) => response,
        Err(error) => {
            eprintln!("finite-private-limiter upstream failed: {error}");
            let _ = settle_usage_with_retries(
                &state,
                &reservation_id,
                SettleRequest {
                    request_id,
                    settlement: "estimate".to_string(),
                    prompt_tokens: None,
                    completion_tokens: None,
                    usage_units: None,
                    usage_formula_version: USAGE_FORMULA_VERSION.to_string(),
                    upstream_status: Some(502),
                    upstream_error_class: Some("upstream_unavailable".to_string()),
                },
            )
            .await;
            return openai_error(
                StatusCode::BAD_GATEWAY,
                "Finite Private upstream is unavailable.",
                "upstream_unavailable",
                "upstream_unavailable",
            );
        }
    };

    let actual = actual_usage(&upstream.body, &state.config.default_model);
    let settle = SettleRequest {
        request_id,
        settlement: if actual.is_some() {
            "actual"
        } else {
            "estimate"
        }
        .to_string(),
        prompt_tokens: actual.as_ref().map(|usage| usage.prompt_tokens),
        completion_tokens: actual.as_ref().map(|usage| usage.completion_tokens),
        usage_units: actual.as_ref().map(|usage| usage.usage_units),
        usage_formula_version: USAGE_FORMULA_VERSION.to_string(),
        upstream_status: Some(upstream.status.as_u16() as i32),
        upstream_error_class: if upstream.status.is_success() {
            None
        } else {
            Some("upstream_error".to_string())
        },
    };
    if let Err(error) = settle_usage_with_retries(&state, &reservation_id, settle).await {
        eprintln!("finite-private-limiter settle failed: {error}");
    }

    let mut response = Response::builder().status(upstream.status);
    if let Some(content_type) = upstream.content_type {
        response = response.header("content-type", content_type);
    }
    response
        .body(axum::body::Body::from(upstream.body))
        .unwrap()
}

async fn reserve_usage(
    state: &AppState,
    input: &ReserveRequest,
) -> Result<ReserveDecision, String> {
    let url = format!(
        "{}/internal/finite-private/v1/reservations",
        state.config.finite_usage_api_url.trim_end_matches('/')
    );
    let response = state
        .client
        .post(url)
        .bearer_auth(&state.config.finite_usage_api_service_key)
        .json(input)
        .send();
    let response = timeout(state.config.usage_api_timeout, response)
        .await
        .map_err(|_| {
            format!(
                "Core reserve timed out after {}ms",
                state.config.usage_api_timeout.as_millis()
            )
        })?
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Core reserve returned {}", response.status()));
    }
    response
        .json::<ReserveDecision>()
        .await
        .map_err(|error| error.to_string())
}

async fn settle_usage(
    state: &AppState,
    reservation_id: &str,
    input: &SettleRequest,
) -> Result<(), String> {
    let url = format!(
        "{}/internal/finite-private/v1/reservations/{}/settle",
        state.config.finite_usage_api_url.trim_end_matches('/'),
        reservation_id
    );
    let response = state
        .client
        .post(url)
        .bearer_auth(&state.config.finite_usage_api_service_key)
        .json(input)
        .send();
    let response = timeout(state.config.usage_api_timeout, response)
        .await
        .map_err(|_| {
            format!(
                "Core settle timed out after {}ms",
                state.config.usage_api_timeout.as_millis()
            )
        })?
        .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("Core settle returned {}", response.status()))
    }
}

async fn settle_usage_with_retries(
    state: &AppState,
    reservation_id: &str,
    input: SettleRequest,
) -> Result<(), String> {
    let mut last_error = String::new();
    for attempt in 1..=3 {
        match settle_usage(state, reservation_id, &input).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = error;
                if attempt == 3 {
                    break;
                }
                sleep(Duration::from_millis(250 * attempt as u64)).await;
            }
        }
    }
    Err(format!("Core settle failed after 3 attempts: {last_error}"))
}

async fn call_upstream(
    state: &AppState,
    uri: &Uri,
    body: Bytes,
) -> Result<UpstreamResponse, String> {
    let response = call_upstream_response(state, uri, body).await?;
    let status =
        StatusCode::from_u16(response.status().as_u16()).map_err(|error| error.to_string())?;
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| HeaderValue::from_str(value).ok());
    let body = timeout(state.config.upstream_body_timeout, response.bytes())
        .await
        .map_err(|_| {
            format!(
                "upstream body timed out after {}ms",
                state.config.upstream_body_timeout.as_millis()
            )
        })?
        .map_err(|error| error.to_string())?;
    Ok(UpstreamResponse {
        status,
        content_type,
        body,
    })
}

async fn call_upstream_response(
    state: &AppState,
    uri: &Uri,
    body: Bytes,
) -> Result<reqwest::Response, String> {
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or(uri.path());
    let url = format!(
        "{}{}",
        state.config.upstream_base_url.trim_end_matches('/'),
        path_and_query
    );
    let response = state
        .client
        .post(url)
        .bearer_auth(&state.config.vllm_internal_api_key)
        .header("content-type", "application/json")
        .body(body)
        .send();
    timeout(state.config.upstream_first_byte_timeout, response)
        .await
        .map_err(|_| {
            format!(
                "upstream first byte timed out after {}ms",
                state.config.upstream_first_byte_timeout.as_millis()
            )
        })?
        .map_err(|error| error.to_string())
}

fn streaming_response(
    state: AppState,
    upstream: reqwest::Response,
    reservation_id: String,
    request_id: String,
) -> Response {
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let upstream_status = Some(status.as_u16() as i32);
    let content_type = upstream
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| HeaderValue::from_str(value).ok());
    let mut response = Response::builder().status(status);
    if let Some(content_type) = content_type {
        response = response.header("content-type", content_type);
    }
    let mut stream = upstream.bytes_stream();
    let settlement = Settlement::new(state, reservation_id, request_id.clone());
    let fallback_settle = SettleRequest {
        request_id,
        settlement: "estimate".to_string(),
        prompt_tokens: None,
        completion_tokens: None,
        usage_units: None,
        usage_formula_version: USAGE_FORMULA_VERSION.to_string(),
        upstream_status,
        upstream_error_class: Some("client_disconnected_or_stream_cancelled".to_string()),
    };
    let idle_timeout = settlement.state.config.upstream_stream_idle_timeout;
    let default_model = settlement.state.config.default_model.clone();
    let body_stream = async_stream::stream! {
        let settlement_guard = SettlementGuard::new(settlement, fallback_settle);
        let mut accumulator = StreamingUsageAccumulator::new(default_model);
        loop {
            let item = match timeout(idle_timeout, stream.next()).await {
                Ok(item) => item,
                Err(_) => {
                    eprintln!(
                        "finite-private-limiter upstream stream idle timeout after {}ms",
                        idle_timeout.as_millis()
                    );
                    let _ = settlement_guard.settle(SettleRequest {
                        request_id: settlement_guard.request_id().to_string(),
                        settlement: "estimate".to_string(),
                        prompt_tokens: None,
                        completion_tokens: None,
                        usage_units: None,
                        usage_formula_version: USAGE_FORMULA_VERSION.to_string(),
                        upstream_status,
                        upstream_error_class: Some("upstream_stream_timeout".to_string()),
                    })
                    .await;
                    yield Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "upstream stream idle timeout",
                    ));
                    return;
                }
            };
            let Some(item) = item else {
                break;
            };
            match item {
                Ok(chunk) => {
                    accumulator.push(&chunk);
                    let saw_done = accumulator.saw_done();
                    yield Ok::<Bytes, std::io::Error>(chunk);
                    if saw_done {
                        break;
                    }
                }
                Err(error) => {
                    eprintln!("finite-private-limiter upstream stream failed: {error}");
                    let _ = settlement_guard
                        .settle(SettleRequest {
                            request_id: settlement_guard.request_id().to_string(),
                            settlement: "estimate".to_string(),
                            prompt_tokens: None,
                            completion_tokens: None,
                            usage_units: None,
                            usage_formula_version: USAGE_FORMULA_VERSION.to_string(),
                            upstream_status,
                            upstream_error_class: Some("upstream_stream_error".to_string()),
                        })
                    .await;
                    yield Err(std::io::Error::other(error.to_string()));
                    return;
                }
            }
        }
        let actual = accumulator.actual_usage();
        let settle = SettleRequest {
            request_id: settlement_guard.request_id().to_string(),
            settlement: if actual.is_some() { "actual" } else { "estimate" }.to_string(),
            prompt_tokens: actual.as_ref().map(|usage| usage.prompt_tokens),
            completion_tokens: actual.as_ref().map(|usage| usage.completion_tokens),
            usage_units: actual.as_ref().map(|usage| usage.usage_units),
            usage_formula_version: USAGE_FORMULA_VERSION.to_string(),
            upstream_status,
            upstream_error_class: if status.is_success() {
                None
            } else {
                Some("upstream_error".to_string())
            },
        };
        if let Err(error) = settlement_guard.settle(settle).await {
            eprintln!("finite-private-limiter streaming settle failed: {error}");
        }
    };
    response.body(Body::from_stream(body_stream)).unwrap()
}

#[derive(Clone)]
struct Settlement {
    state: AppState,
    reservation_id: Arc<str>,
    request_id: Arc<str>,
    completed: Arc<std::sync::atomic::AtomicBool>,
}

impl Settlement {
    fn new(state: AppState, reservation_id: String, request_id: String) -> Self {
        Self {
            state,
            reservation_id: Arc::from(reservation_id),
            request_id: Arc::from(request_id),
            completed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn request_id(&self) -> &str {
        &self.request_id
    }

    async fn settle(&self, request: SettleRequest) -> Result<(), String> {
        if self.completed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        settle_usage_with_retries(&self.state, &self.reservation_id, request).await
    }

    fn settle_in_background(&self, request: SettleRequest) {
        if self.completed.swap(true, Ordering::SeqCst) {
            return;
        }
        let settlement = self.clone();
        tokio::spawn(async move {
            if let Err(error) =
                settle_usage_with_retries(&settlement.state, &settlement.reservation_id, request)
                    .await
            {
                eprintln!("finite-private-limiter background settle failed: {error}");
            }
        });
    }
}

struct SettlementGuard {
    settlement: Settlement,
    fallback: SettleRequest,
}

impl SettlementGuard {
    fn new(settlement: Settlement, fallback: SettleRequest) -> Self {
        Self {
            settlement,
            fallback,
        }
    }

    fn request_id(&self) -> &str {
        self.settlement.request_id()
    }

    async fn settle(&self, request: SettleRequest) -> Result<(), String> {
        self.settlement.settle(request).await
    }
}

impl Drop for SettlementGuard {
    fn drop(&mut self) {
        self.settlement.settle_in_background(self.fallback.clone());
    }
}

fn denied_response(decision: ReserveDecision) -> Response {
    let status = decision
        .error
        .as_ref()
        .map(|error| match error.code.as_str() {
            "invalid_api_key" => StatusCode::UNAUTHORIZED,
            "burst_window_limit_exceeded" => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::FORBIDDEN,
        })
        .unwrap_or(StatusCode::FORBIDDEN);
    let error = decision.error.unwrap_or(ReserveError {
        message: "Finite Private request denied.".to_string(),
        error_type: "usage_limit".to_string(),
        code: "finite_private_denied".to_string(),
        retry_after: None,
        reset_at: None,
        dashboard_url: None,
        request_id: None,
    });
    (status, Json(json!({ "error": error }))).into_response()
}

fn openai_error(status: StatusCode, message: &str, error_type: &str, code: &str) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "message": message,
                "type": error_type,
                "code": code
            }
        })),
    )
        .into_response()
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn new_request_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("fp_req_{millis}_{counter}")
}

fn request_is_streaming(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("stream").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn upstream_body_for_request(uri: &Uri, body: Bytes) -> Bytes {
    if uri.path() != "/v1/chat/completions" || !request_is_streaming(&body) {
        return body;
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(&body) else {
        return body;
    };
    let Some(object) = value.as_object_mut() else {
        return body;
    };
    let stream_options = object.entry("stream_options").or_insert_with(|| json!({}));
    if let Some(options) = stream_options.as_object_mut() {
        options.insert("include_usage".to_string(), Value::Bool(true));
    }
    serde_json::to_vec(&value).map(Bytes::from).unwrap_or(body)
}

fn estimate_usage(body: &[u8], default_model: &str) -> EstimatedUsage {
    let value = serde_json::from_slice::<Value>(body).unwrap_or(Value::Null);
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(default_model)
        .to_string();
    let prompt_chars = prompt_text(&value).chars().count() as i64;
    let prompt_tokens = (prompt_chars / 4).max(1);
    let completion_tokens = value
        .get("max_completion_tokens")
        .or_else(|| value.get("max_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(4096)
        .max(1);
    let usage_units = usage_units(prompt_tokens, completion_tokens, &model);
    EstimatedUsage {
        model,
        prompt_tokens,
        completion_tokens,
        usage_units,
    }
}

fn prompt_text(value: &Value) -> String {
    if let Some(messages) = value.get("messages").and_then(Value::as_array) {
        return messages
            .iter()
            .filter_map(|message| message.get("content"))
            .map(value_text)
            .collect::<Vec<_>>()
            .join("\n");
    }
    value
        .get("input")
        .map(value_text)
        .unwrap_or_else(|| value.to_string())
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(value_text).collect::<Vec<_>>().join("\n"),
        Value::Object(object) => object
            .get("text")
            .or_else(|| object.get("content"))
            .map(value_text)
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

fn actual_usage(body: &[u8], default_model: &str) -> Option<ActualUsage> {
    let value = serde_json::from_slice::<Value>(body).ok()?;
    actual_usage_from_value(&value, default_model)
}

fn actual_usage_from_value(value: &Value, default_model: &str) -> Option<ActualUsage> {
    let usage = value.get("usage").or_else(|| {
        value
            .get("response")
            .and_then(|response| response.get("usage"))
    })?;
    let prompt_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_i64)?;
    let completion_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_i64)?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("response")
                .and_then(|response| response.get("model"))
                .and_then(Value::as_str)
        })
        .unwrap_or(default_model);
    Some(ActualUsage {
        prompt_tokens,
        completion_tokens,
        usage_units: usage_units(prompt_tokens, completion_tokens, model),
    })
}

fn usage_units(prompt_tokens: i64, completion_tokens: i64, model: &str) -> i64 {
    let base_units = prompt_tokens as f64 + (completion_tokens as f64 * 3.0);
    let context_multiplier = (1.0 + (prompt_tokens as f64 / 100_000.0)).min(3.0);
    let model_multiplier = match model {
        DEFAULT_MODEL => 1.0,
        _ => 1.0,
    };
    (base_units * context_multiplier * model_multiplier).ceil() as i64
}

struct StreamingUsageAccumulator {
    pending_line: String,
    actual_usage: Option<ActualUsage>,
    saw_done: bool,
    default_model: String,
}

impl StreamingUsageAccumulator {
    fn new(default_model: String) -> Self {
        Self {
            pending_line: String::new(),
            actual_usage: None,
            saw_done: false,
            default_model,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        self.pending_line.push_str(&String::from_utf8_lossy(chunk));
        while let Some(newline) = self.pending_line.find('\n') {
            let mut line = self.pending_line[..newline].to_string();
            self.pending_line.drain(..=newline);
            if line.ends_with('\r') {
                line.pop();
            }
            self.process_line(&line);
        }
        if self.pending_line.len() > 64 * 1024 {
            self.pending_line.clear();
        }
    }

    fn process_line(&mut self, line: &str) {
        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let data = data.trim();
        if data.is_empty() {
            return;
        }
        if data == "[DONE]" {
            self.saw_done = true;
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        if let Some(usage) = actual_usage_from_value(&value, &self.default_model) {
            self.actual_usage = Some(usage);
        }
    }

    fn actual_usage(self) -> Option<ActualUsage> {
        self.actual_usage
    }

    fn saw_done(&self) -> bool {
        self.saw_done
    }
}

#[derive(Debug)]
struct EstimatedUsage {
    model: String,
    prompt_tokens: i64,
    completion_tokens: i64,
    usage_units: i64,
}

#[derive(Debug)]
struct ActualUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
    usage_units: i64,
}

struct UpstreamResponse {
    status: StatusCode,
    content_type: Option<HeaderValue>,
    body: Bytes,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadinessSnapshot {
    ok: bool,
    service: &'static str,
    kind: &'static str,
    checked_at_unix_ms: u128,
    config: PublicConfigSnapshot,
    components: ReadinessComponents,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadinessComponents {
    upstream: ComponentCheck,
    usage_api: ComponentCheck,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComponentCheck {
    ok: bool,
    name: &'static str,
    target_url: String,
    authenticated: bool,
    status: Option<u16>,
    latency_ms: u128,
    timeout_ms: u128,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicConfigSnapshot {
    default_model: String,
    upstream_base_url: String,
    upstream_health_path: String,
    usage_api_base_url: String,
    usage_api_health_path: String,
    readiness_timeout_ms: u128,
    usage_api_timeout_ms: u128,
    upstream_first_byte_timeout_ms: u128,
    upstream_body_timeout_ms: u128,
    upstream_stream_idle_timeout_ms: u128,
    required_secrets: RequiredSecretsSnapshot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RequiredSecretsSnapshot {
    finite_usage_api_service_key_present: bool,
    vllm_internal_api_key_present: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReserveRequest {
    request_id: String,
    presented_api_key: String,
    endpoint: String,
    model: String,
    estimated_prompt_tokens: i64,
    estimated_completion_tokens: i64,
    estimated_usage_units: i64,
    usage_formula_version: String,
    dashboard_url: String,
}

#[derive(Debug, Deserialize)]
struct ReserveDecision {
    decision: String,
    reservation_id: Option<String>,
    error: Option<ReserveError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReserveError {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    code: String,
    retry_after: Option<i64>,
    reset_at: Option<String>,
    dashboard_url: Option<String>,
    request_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SettleRequest {
    request_id: String,
    settlement: String,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    usage_units: Option<i64>,
    usage_formula_version: String,
    upstream_status: Option<i32>,
    upstream_error_class: Option<String>,
}

fn join_url(base_url: &str, path: &str) -> String {
    if path.starts_with('/') {
        format!("{}{}", base_url.trim_end_matches('/'), path)
    } else {
        format!("{}/{}", base_url.trim_end_matches('/'), path)
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn elapsed_millis(started: Instant) -> u128 {
    started.elapsed().as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn limiter_reserves_proxies_settles_and_denies_before_upstream() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;

        let limiter_url = spawn(app(test_config(core_url, upstream_url)).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_secret")
            .header("x-request-id", "req-limiter-ok")
            .json(&json!({
                "model": "glm-5-2",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "ok");
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        assert_eq!(core.reserve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            core.settlements.lock().unwrap()[0]["settlement"].as_str(),
            Some("actual")
        );

        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_secret")
            .header("x-request-id", "req-limiter-denied")
            .json(&json!({
                "model": "glm-5-2",
                "messages": [{ "role": "user", "content": "too much" }],
                "max_tokens": 2_000_000
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        assert_eq!(core.reserve_calls.load(Ordering::SeqCst), 2);
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn limiter_streams_and_settles_sse_usage() {
        let core = FakeCoreState::new("fpk_live_stream", 10_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let limiter_url = spawn(app(test_config(core_url, upstream_url)).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_stream")
            .header("x-request-id", "req-stream-first")
            .json(&json!({
                "model": "glm-5-2",
                "stream": true,
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 1_500_000
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.unwrap();
        assert!(body.contains("data: [DONE]"));
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 1);

        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_stream")
            .header("x-request-id", "req-stream-second")
            .json(&json!({
                "model": "glm-5-2",
                "stream": true,
                "messages": [{ "role": "user", "content": "still under after actual settle" }],
                "max_tokens": 1_600_000
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = response.text().await.unwrap();
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 2);
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 2);
        assert!(
            core.settlements
                .lock()
                .unwrap()
                .iter()
                .all(|settlement| settlement["settlement"] == "actual")
        );
    }

    #[tokio::test]
    async fn stream_done_settles_without_waiting_for_socket_close() {
        let core = FakeCoreState::new("fpk_live_stream", 10_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        upstream
            .stream_hang_after_done
            .store(true, Ordering::SeqCst);
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let mut config = test_config(core_url, upstream_url);
        config.upstream_stream_idle_timeout = Duration::from_secs(60);
        let limiter_url = spawn(app(config).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_stream")
            .json(&json!({
                "model": "glm-5-2",
                "stream": true,
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = timeout(Duration::from_secs(2), response.text())
            .await
            .expect("stream should finish after [DONE]")
            .unwrap();
        assert!(body.contains("data: [DONE]"));

        wait_for(|| core.settle_calls.load(Ordering::SeqCst) == 1).await;
        let settlements = core.settlements.lock().unwrap();
        assert_eq!(settlements[0]["settlement"], "actual");
    }

    #[tokio::test]
    async fn readiness_reports_dependency_health() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let limiter_url =
            spawn(app(test_config(core_url.clone(), upstream_url.clone())).unwrap()).await;

        let client = reqwest::Client::new();
        let live = client
            .get(format!("{limiter_url}/live"))
            .send()
            .await
            .unwrap();
        assert_eq!(live.status(), StatusCode::OK);
        let live: Value = live.json().await.unwrap();
        assert_eq!(live["config"]["defaultModel"], "glm-5-2");
        assert_eq!(live["config"]["upstreamHealthPath"], "/health");
        assert_eq!(
            live["config"]["requiredSecrets"]["finiteUsageApiServiceKeyPresent"],
            true
        );
        assert_eq!(
            live["config"]["requiredSecrets"]["vllmInternalApiKeyPresent"],
            true
        );

        let ready = client
            .get(format!("{limiter_url}/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(ready.status(), StatusCode::OK);
        let ready: Value = ready.json().await.unwrap();
        assert_eq!(ready["ok"], true);
        assert_eq!(ready["config"]["defaultModel"], "glm-5-2");
        assert_eq!(ready["components"]["upstream"]["name"], "upstream");
        assert_eq!(ready["components"]["upstream"]["ok"], true);
        assert_eq!(
            ready["components"]["upstream"]["targetUrl"],
            format!("{upstream_url}/health")
        );
        assert_eq!(ready["components"]["upstream"]["authenticated"], false);
        assert_eq!(ready["components"]["upstream"]["timeoutMs"], 100);
        assert_eq!(ready["components"]["usageApi"]["name"], "usage_api");
        assert_eq!(ready["components"]["usageApi"]["ok"], true);
        assert_eq!(
            ready["components"]["usageApi"]["targetUrl"],
            format!("{core_url}/internal/finite-private/v1/health")
        );
        assert_eq!(ready["components"]["usageApi"]["authenticated"], true);

        upstream.health_ok.store(false, Ordering::SeqCst);
        let ready = client
            .get(format!("{limiter_url}/ready"))
            .send()
            .await
            .unwrap();
        assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
        let ready: Value = ready.json().await.unwrap();
        assert_eq!(ready["ok"], false);
        assert_eq!(ready["components"]["upstream"]["ok"], false);
        assert_eq!(ready["components"]["usageApi"]["ok"], true);
    }

    #[test]
    fn usage_fallback_model_is_configurable() {
        let estimate = estimate_usage(
            br#"{"messages":[{"role":"user","content":"hello"}],"max_tokens":8}"#,
            "glm-5-2",
        );
        assert_eq!(estimate.model, "glm-5-2");

        let actual = actual_usage(
            br#"{"usage":{"prompt_tokens":4,"completion_tokens":5,"total_tokens":9}}"#,
            "glm-5-2",
        )
        .unwrap();
        assert_eq!(actual.usage_units, usage_units(4, 5, "glm-5-2"));
    }

    #[tokio::test]
    async fn upstream_first_byte_timeout_settles_estimate() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        upstream.delay_first_byte_ms.store(200, Ordering::SeqCst);
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let mut config = test_config(core_url, upstream_url);
        config.upstream_first_byte_timeout = Duration::from_millis(20);
        let limiter_url = spawn(app(config).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_secret")
            .header("x-request-id", "req-timeout")
            .json(&json!({
                "model": "glm-5-2",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(core.reserve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 1);
        let settlements = core.settlements.lock().unwrap();
        assert_eq!(settlements[0]["settlement"], "estimate");
        assert_eq!(settlements[0]["upstreamErrorClass"], "upstream_unavailable");
    }

    #[tokio::test]
    async fn duplicate_client_request_id_still_creates_unique_reservations() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let limiter_url = spawn(app(test_config(core_url, upstream_url)).unwrap()).await;

        let client = reqwest::Client::new();
        for _ in 0..2 {
            let response = client
                .post(format!("{limiter_url}/v1/chat/completions"))
                .bearer_auth("fpk_live_secret")
                .header("x-request-id", "caller-reused-id")
                .json(&json!({
                    "model": "glm-5-2",
                    "messages": [{ "role": "user", "content": "hello" }],
                    "max_tokens": 64
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let _ = response.bytes().await.unwrap();
        }

        assert_eq!(upstream.calls.load(Ordering::SeqCst), 2);
        assert_eq!(core.reserve_calls.load(Ordering::SeqCst), 2);
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 2);
        let reservations = core.reservations.lock().unwrap();
        let first = reservations[0]["requestId"].as_str().unwrap();
        let second = reservations[1]["requestId"].as_str().unwrap();
        assert_ne!(first, "caller-reused-id");
        assert_ne!(second, "caller-reused-id");
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn settle_retries_transient_core_failures() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        core.settle_failures_remaining.store(1, Ordering::SeqCst);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let limiter_url = spawn(app(test_config(core_url, upstream_url)).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_secret")
            .json(&json!({
                "model": "glm-5-2",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = response.bytes().await.unwrap();
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 2);
        let settlements = core.settlements.lock().unwrap();
        assert_eq!(settlements.len(), 1);
        assert_eq!(settlements[0]["settlement"], "actual");
    }

    #[tokio::test]
    async fn responses_route_proxies_non_streaming_and_rejects_streaming() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let limiter_url = spawn(app(test_config(core_url, upstream_url)).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/responses"))
            .bearer_auth("fpk_live_secret")
            .json(&json!({
                "model": "glm-5-2",
                "input": "hello",
                "max_output_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = response.bytes().await.unwrap();
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        assert_eq!(core.reserve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            core.settlements.lock().unwrap()[0]["settlement"].as_str(),
            Some("actual")
        );

        let response = client
            .post(format!("{limiter_url}/v1/responses"))
            .bearer_auth("fpk_live_secret")
            .json(&json!({
                "model": "glm-5-2",
                "input": "hello",
                "stream": true
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        assert_eq!(core.reserve_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stream_idle_timeout_settles_estimate() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        upstream.stream_tail_delay_ms.store(200, Ordering::SeqCst);
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let mut config = test_config(core_url, upstream_url);
        config.upstream_stream_idle_timeout = Duration::from_millis(20);
        let limiter_url = spawn(app(config).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_secret")
            .json(&json!({
                "model": "glm-5-2",
                "stream": true,
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.bytes_stream();
        let first = stream.next().await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&first).contains("chatcmpl_stream"));
        let second = stream.next().await;
        assert!(second.is_some());
        wait_for(|| core.settle_calls.load(Ordering::SeqCst) == 1).await;
        let settlements = core.settlements.lock().unwrap();
        assert_eq!(settlements[0]["settlement"], "estimate");
        assert_eq!(
            settlements[0]["upstreamErrorClass"],
            "upstream_stream_timeout"
        );
    }

    #[tokio::test]
    async fn stream_client_disconnect_settles_estimate() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        upstream.stream_tail_delay_ms.store(500, Ordering::SeqCst);
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let mut config = test_config(core_url, upstream_url);
        config.upstream_stream_idle_timeout = Duration::from_secs(10);
        let limiter_url = spawn(app(config).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_secret")
            .json(&json!({
                "model": "glm-5-2",
                "stream": true,
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.bytes_stream();
        let first = stream.next().await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&first).contains("chatcmpl_stream"));
        drop(stream);

        wait_for(|| core.settle_calls.load(Ordering::SeqCst) == 1).await;
        let settlements = core.settlements.lock().unwrap();
        assert_eq!(settlements[0]["settlement"], "estimate");
        assert_eq!(
            settlements[0]["upstreamErrorClass"],
            "client_disconnected_or_stream_cancelled"
        );
    }

    #[derive(Clone)]
    struct FakeCoreState {
        allowed_key: &'static str,
        max_usage_units: i64,
        health_ok: Arc<AtomicBool>,
        reserve_calls: Arc<AtomicUsize>,
        settle_calls: Arc<AtomicUsize>,
        settle_failures_remaining: Arc<AtomicUsize>,
        reservations: Arc<Mutex<Vec<Value>>>,
        settlements: Arc<Mutex<Vec<Value>>>,
    }

    impl FakeCoreState {
        fn new(allowed_key: &'static str, max_usage_units: i64) -> Self {
            Self {
                allowed_key,
                max_usage_units,
                health_ok: Arc::new(AtomicBool::new(true)),
                reserve_calls: Arc::new(AtomicUsize::new(0)),
                settle_calls: Arc::new(AtomicUsize::new(0)),
                settle_failures_remaining: Arc::new(AtomicUsize::new(0)),
                reservations: Arc::new(Mutex::new(Vec::new())),
                settlements: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    fn fake_core_router(state: FakeCoreState) -> Router {
        Router::new()
            .route("/internal/finite-private/v1/health", get(fake_core_health))
            .route(
                "/internal/finite-private/v1/reservations",
                post(fake_core_reserve),
            )
            .route(
                "/internal/finite-private/v1/reservations/{reservation_id}/settle",
                post(fake_core_settle),
            )
            .with_state(state)
    }

    async fn fake_core_health(State(state): State<FakeCoreState>) -> Response {
        if state.health_ok.load(Ordering::SeqCst) {
            (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
        } else {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "ok": false })),
            )
                .into_response()
        }
    }

    async fn fake_core_reserve(
        State(state): State<FakeCoreState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        assert_service_auth(&headers);
        state.reserve_calls.fetch_add(1, Ordering::SeqCst);
        state.reservations.lock().unwrap().push(body.clone());
        let request_id = body["requestId"].as_str().unwrap_or("req");
        if body["presentedApiKey"].as_str() != Some(state.allowed_key) {
            return (
                StatusCode::OK,
                Json(json!({
                    "decision": "deny",
                    "error": {
                        "message": "Finite Private API key is invalid.",
                        "type": "invalid_api_key",
                        "code": "invalid_api_key",
                        "retry_after": null,
                        "reset_at": null,
                        "dashboard_url": null,
                        "request_id": request_id
                    }
                })),
            )
                .into_response();
        }
        if body["estimatedUsageUnits"].as_i64().unwrap_or(0) > state.max_usage_units {
            return (
                StatusCode::OK,
                Json(json!({
                    "decision": "deny",
                    "error": {
                        "message": "Finite Private burst window limit reached.",
                        "type": "usage_limit",
                        "code": "burst_window_limit_exceeded",
                        "retry_after": 3600,
                        "reset_at": "2026-05-26T13:00:00Z",
                        "dashboard_url": "https://finite.computer/dashboard",
                        "request_id": request_id
                    }
                })),
            )
                .into_response();
        }
        (
            StatusCode::OK,
            Json(json!({
                "decision": "allow",
                "reservation_id": format!("res_{request_id}")
            })),
        )
            .into_response()
    }

    async fn fake_core_settle(
        State(state): State<FakeCoreState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        assert_service_auth(&headers);
        state.settle_calls.fetch_add(1, Ordering::SeqCst);
        if state
            .settle_failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                (remaining > 0).then(|| remaining - 1)
            })
            .is_ok()
        {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "ok": false })),
            )
                .into_response();
        }
        state.settlements.lock().unwrap().push(body);
        (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
    }

    fn assert_service_auth(headers: &HeaderMap) {
        assert_eq!(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer core-token")
        );
    }

    #[derive(Clone)]
    struct FakeUpstreamState {
        health_ok: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
        delay_first_byte_ms: Arc<AtomicUsize>,
        stream_tail_delay_ms: Arc<AtomicUsize>,
        stream_hang_after_done: Arc<AtomicBool>,
    }

    impl FakeUpstreamState {
        fn new() -> Self {
            Self {
                health_ok: Arc::new(AtomicBool::new(true)),
                calls: Arc::new(AtomicUsize::new(0)),
                delay_first_byte_ms: Arc::new(AtomicUsize::new(0)),
                stream_tail_delay_ms: Arc::new(AtomicUsize::new(0)),
                stream_hang_after_done: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    fn fake_upstream_router(state: FakeUpstreamState) -> Router {
        Router::new()
            .route("/health", get(fake_upstream_health))
            .route("/v1/chat/completions", post(fake_upstream_chat))
            .route("/v1/responses", post(fake_upstream_responses))
            .with_state(state)
    }

    async fn fake_upstream_health(State(state): State<FakeUpstreamState>) -> Response {
        if state.health_ok.load(Ordering::SeqCst) {
            (StatusCode::OK, "ok").into_response()
        } else {
            (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
        }
    }

    async fn fake_upstream_chat(
        State(state): State<FakeUpstreamState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        assert_eq!(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer vllm-secret")
        );
        let delay = state.delay_first_byte_ms.load(Ordering::SeqCst);
        if delay > 0 {
            sleep(Duration::from_millis(delay as u64)).await;
        }
        state.calls.fetch_add(1, Ordering::SeqCst);
        let request = serde_json::from_slice::<Value>(&body).unwrap();
        if request["stream"].as_bool().unwrap_or(false) {
            assert_eq!(request["stream_options"]["include_usage"], true);
            if state.stream_hang_after_done.load(Ordering::SeqCst) {
                let body_stream = async_stream::stream! {
                    yield Ok::<Bytes, std::io::Error>(Bytes::from_static(
                        b"data: {\"id\":\"chatcmpl_stream\",\"model\":\"glm-5-2\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                    ));
                    yield Ok::<Bytes, std::io::Error>(Bytes::from_static(
                        b"data: {\"id\":\"chatcmpl_stream\",\"model\":\"glm-5-2\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":10,\"total_tokens\":20}}\n\n",
                    ));
                    yield Ok::<Bytes, std::io::Error>(Bytes::from_static(b"data: [DONE]\n\n"));
                    sleep(Duration::from_secs(3600)).await;
                };
                return (
                    StatusCode::OK,
                    [("content-type", "text/event-stream")],
                    Body::from_stream(body_stream),
                )
                    .into_response();
            }
            let tail_delay_ms = state.stream_tail_delay_ms.load(Ordering::SeqCst);
            if tail_delay_ms > 0 {
                let body_stream = async_stream::stream! {
                    yield Ok::<Bytes, std::io::Error>(Bytes::from_static(
                        b"data: {\"id\":\"chatcmpl_stream\",\"model\":\"glm-5-2\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                    ));
                    sleep(Duration::from_millis(tail_delay_ms as u64)).await;
                    yield Ok::<Bytes, std::io::Error>(Bytes::from_static(
                        b"data: {\"id\":\"chatcmpl_stream\",\"model\":\"glm-5-2\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":10,\"total_tokens\":20}}\n\n",
                    ));
                    yield Ok::<Bytes, std::io::Error>(Bytes::from_static(b"data: [DONE]\n\n"));
                };
                return (
                    StatusCode::OK,
                    [("content-type", "text/event-stream")],
                    Body::from_stream(body_stream),
                )
                    .into_response();
            }
            let body = [
                "data: {\"id\":\"chatcmpl_stream\",\"model\":\"glm-5-2\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                "data: {\"id\":\"chatcmpl_stream\",\"model\":\"glm-5-2\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":10,\"total_tokens\":20}}\n\n",
                "data: [DONE]\n\n",
            ]
            .concat();
            return (
                StatusCode::OK,
                [("content-type", "text/event-stream")],
                Body::from(body),
            )
                .into_response();
        }
        (
            StatusCode::OK,
            Json(json!({
                "id": "chatcmpl_test",
                "model": "glm-5-2",
                "choices": [{ "message": { "role": "assistant", "content": "ok" }}],
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 20,
                    "total_tokens": 120
                }
            })),
        )
            .into_response()
    }

    async fn fake_upstream_responses(
        State(state): State<FakeUpstreamState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        assert_eq!(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer vllm-secret")
        );
        state.calls.fetch_add(1, Ordering::SeqCst);
        let _request = serde_json::from_slice::<Value>(&body).unwrap();
        (
            StatusCode::OK,
            Json(json!({
                "id": "resp_test",
                "response": {
                    "model": "glm-5-2",
                    "usage": {
                        "input_tokens": 40,
                        "output_tokens": 8,
                        "total_tokens": 48
                    }
                },
                "output": [{ "content": [{ "type": "output_text", "text": "ok" }] }]
            })),
        )
            .into_response()
    }

    fn test_config(core_url: String, upstream_url: String) -> LimiterConfig {
        let mut config = LimiterConfig::new(
            core_url,
            "core-token".to_string(),
            upstream_url,
            "vllm-secret".to_string(),
            "https://finite.computer/dashboard".to_string(),
        );
        config.readiness_timeout = Duration::from_millis(100);
        config.usage_api_timeout = Duration::from_millis(100);
        config.upstream_first_byte_timeout = Duration::from_millis(100);
        config.upstream_body_timeout = Duration::from_millis(100);
        config.upstream_stream_idle_timeout = Duration::from_millis(100);
        config
    }

    async fn wait_for(mut condition: impl FnMut() -> bool) {
        for _ in 0..100 {
            if condition() {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
        assert!(condition());
    }

    async fn spawn(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }
}
