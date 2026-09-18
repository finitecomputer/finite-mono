use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use reqwest::Client;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Semaphore, watch};
use tokio::time::{sleep, timeout};

mod metrics;
use metrics::{LimiterMetrics, RequestTimer, TerminalOutcome};

const USAGE_FORMULA_VERSION: &str = "2026-05-26.v1";
const DEFAULT_MODEL: &str = "glm-5-3-flash";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DIAGNOSTIC_WRITE_CONCURRENCY: usize = 8;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct LimiterConfig {
    pub finite_usage_api_url: String,
    pub finite_usage_api_service_key: String,
    pub upstream_base_url: String,
    pub vllm_internal_api_key: String,
    pub default_model: String,
    pub upstream_model: Option<String>,
    pub model_aliases: Vec<String>,
    pub dashboard_url: String,
    pub upstream_health_path: String,
    pub usage_api_health_path: String,
    pub readiness_timeout: Duration,
    pub usage_api_timeout: Duration,
    pub upstream_first_byte_timeout: Duration,
    pub upstream_body_timeout: Duration,
    pub upstream_stream_idle_timeout: Duration,
    pub watchdog: WatchdogConfig,
    /// Degraded-admission escape hatch. `UsageApi` (the default) reserves and
    /// settles every request against the Finite usage API. `Allowlist` admits
    /// only bearer keys listed in `admission_allowlist`, performs no
    /// reservation, and skips settlement: usage during this mode is
    /// unaccounted. Every degraded response carries the
    /// `x-finite-admission: degraded-allowlist` header so the mode is
    /// observable in captured traffic. Revert by switching the mode back to
    /// `usage-api` (or removing the env) only after `FINITE_USAGE_API_URL`
    /// reaches Core. A public HTML outage page on that origin is not a usage
    /// API. No data format changes either way.
    pub admission_mode: AdmissionMode,
    pub admission_allowlist: Vec<String>,
    /// Optional bearer credential for the metrics listener. Production
    /// deployments should set this when the listener is reachable outside the
    /// container's private monitoring path.
    pub metrics_auth_token: Option<String>,
    /// When set, omitted `reasoning_effort` on `/v1/chat/completions` is
    /// filled with this value. Explicit client values are left alone.
    /// GLM-5.3-Flash's checkpoint default is `max`; Finite's product default
    /// is `high`.
    pub default_reasoning_effort: Option<String>,
    /// When set, omitted `chat_template_kwargs.enable_thinking` /
    /// `chat_template_kwargs.thinking` is filled. Explicit client values win.
    pub default_enable_thinking: Option<bool>,
}

/// Admission operating mode. See [`LimiterConfig`](struct.LimiterConfig.html)
/// field docs for the degraded-mode trade-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionMode {
    UsageApi,
    Allowlist,
}

impl AdmissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AdmissionMode::UsageApi => "usage-api",
            AdmissionMode::Allowlist => "allowlist",
        }
    }
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
            upstream_model: None,
            model_aliases: Vec::new(),
            dashboard_url,
            upstream_health_path: "/health".to_string(),
            usage_api_health_path: "/internal/finite-private/v1/health".to_string(),
            readiness_timeout: Duration::from_secs(5),
            usage_api_timeout: Duration::from_secs(15),
            upstream_first_byte_timeout: Duration::from_secs(120),
            upstream_body_timeout: Duration::from_secs(600),
            upstream_stream_idle_timeout: Duration::from_secs(600),
            watchdog: WatchdogConfig::default(),
            admission_mode: AdmissionMode::UsageApi,
            admission_allowlist: Vec::new(),
            metrics_auth_token: None,
            default_reasoning_effort: None,
            default_enable_thinking: None,
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
    metrics: LimiterMetrics,
    diagnostic_slots: Arc<Semaphore>,
}

#[derive(Debug, thiserror::Error)]
pub enum LimiterConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{0} must be greater than zero")]
    InvalidDuration(&'static str),
    #[error("failed to build HTTP client: {0}")]
    HttpClient(String),
    #[error("invalid model routing: {0}")]
    InvalidModelRouting(String),
}

impl LimiterConfig {
    fn routed_model(&self, requested_model: &str) -> Option<String> {
        let Some(upstream_model) = self.upstream_model.as_deref() else {
            return Some(requested_model.to_string());
        };
        if requested_model == upstream_model
            || requested_model == self.default_model
            || self
                .model_aliases
                .iter()
                .any(|alias| alias == requested_model)
        {
            Some(upstream_model.to_string())
        } else {
            None
        }
    }

    fn accepted_models(&self) -> Vec<String> {
        let mut models = vec![self.default_model.clone()];
        if let Some(upstream_model) = &self.upstream_model
            && !models.contains(upstream_model)
        {
            models.push(upstream_model.clone());
        }
        for alias in &self.model_aliases {
            if !models.contains(alias) {
                models.push(alias.clone());
            }
        }
        models
    }

    fn telemetry_model<'a>(&'a self, routed_model: &'a str) -> &'a str {
        // A deployment without `upstream_model` intentionally forwards the
        // requested model. Keep that legacy routing behavior while collapsing
        // arbitrary client model names into one bounded telemetry bucket.
        if self.upstream_model.is_none() && routed_model != self.default_model {
            "dynamic"
        } else {
            routed_model
        }
    }
}

pub fn app(config: LimiterConfig) -> Result<Router, LimiterConfigError> {
    validate_config(&config)?;
    let state = AppState {
        config: Arc::new(config),
        client: Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            // Core does not redirect health, reserve, or settle. Following a
            // public outage origin 307 onto HTML 200 would look like success.
            .redirect(Policy::none())
            .build()
            .map_err(|error| LimiterConfigError::HttpClient(error.to_string()))?,
        metrics: LimiterMetrics::default(),
        diagnostic_slots: Arc::new(Semaphore::new(DIAGNOSTIC_WRITE_CONCURRENCY)),
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
    if config.admission_mode == AdmissionMode::Allowlist && config.admission_allowlist.is_empty() {
        // Fail closed: an allowlist mode with no keys would either admit
        // nobody (harmless but confusing) or, if ever "optimized" into an
        // empty-means-anyone semantic, admit everybody. Refuse to start.
        return Err(LimiterConfigError::Missing(
            "FINITE_ADMISSION_ALLOWLIST (required when FINITE_ADMISSION_MODE=allowlist)",
        ));
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
    if config.upstream_model.is_none() && !config.model_aliases.is_empty() {
        return Err(LimiterConfigError::InvalidModelRouting(
            "model aliases require an upstream model".to_string(),
        ));
    }
    if let Some(upstream_model) = &config.upstream_model
        && upstream_model.trim().is_empty()
    {
        return Err(LimiterConfigError::InvalidModelRouting(
            "upstream model must not be empty".to_string(),
        ));
    }
    if config
        .model_aliases
        .iter()
        .any(|alias| alias.trim().is_empty())
    {
        return Err(LimiterConfigError::InvalidModelRouting(
            "model aliases must not contain an empty value".to_string(),
        ));
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

async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(expected) = state.config.metrics_auth_token.as_deref() else {
        return (
            [("content-type", "text/plain; version=0.0.4")],
            "finite_private_limiter_live 1\n",
        )
            .into_response();
    };
    if bearer_token(&headers).as_deref() != Some(expected) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    (
        [("content-type", "text/plain; version=0.0.4")],
        state.metrics.render(state.config.admission_mode.as_str()),
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
        HealthBody::AnySuccess,
    );
    let usage_api_check = check_component(
        &state.client,
        "usage_api",
        usage_api_target,
        Some(&state.config.finite_usage_api_service_key),
        state.config.readiness_timeout,
        HealthBody::CoreJsonOk,
    );
    let (upstream, usage_api) = tokio::join!(upstream_check, usage_api_check);
    // In allowlist mode the usage API is intentionally out of the request
    // path, so its state is reported for observability but does not gate
    // readiness.
    let degraded = state.config.admission_mode == AdmissionMode::Allowlist;
    ReadinessSnapshot {
        ok: upstream.ok && (usage_api.ok || degraded),
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

/// What a component health response must look like. Upstream inference
/// health is a status-only probe (SGLang may return plain text). The usage
/// API is Core's JSON `{"ok": true}` contract — a 2xx HTML outage page is
/// not that contract.
#[derive(Clone, Copy)]
enum HealthBody {
    AnySuccess,
    CoreJsonOk,
}

async fn check_component(
    client: &Client,
    name: &'static str,
    url: String,
    bearer_token: Option<&str>,
    timeout_duration: Duration,
    body: HealthBody,
) -> ComponentCheck {
    let started = Instant::now();
    let mut request = client.get(url.clone());
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    match timeout(timeout_duration, request.send()).await {
        Ok(Ok(response)) => {
            let status = response.status();
            let (ok, error) = match body {
                HealthBody::AnySuccess => {
                    drop(response);
                    (status.is_success(), None)
                }
                HealthBody::CoreJsonOk => match usage_api_health_ok(response).await {
                    Ok(()) => (true, None),
                    Err(error) => (false, Some(error)),
                },
            };
            ComponentCheck {
                ok,
                name,
                target_url: url,
                authenticated: bearer_token.is_some(),
                status: Some(status.as_u16()),
                latency_ms: elapsed_millis(started),
                timeout_ms: timeout_duration.as_millis(),
                error,
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

/// Core usage-API health is `GET .../internal/finite-private/v1/health` →
/// JSON `{"ok": true}`. Following a public origin 307 onto an HTML 200
/// (the finite.computer outage page) must not count as ready.
async fn usage_api_health_ok(response: reqwest::Response) -> Result<(), String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!("usage API health returned {status}"));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !content_type.contains("application/json") {
        let got = if content_type.is_empty() {
            "missing Content-Type".to_string()
        } else {
            content_type
        };
        return Err(format!(
            "usage API health returned {got} instead of application/json"
        ));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|error| format!("usage API health is not JSON: {error}"))?;
    if body.get("ok") != Some(&json!(true)) {
        return Err("usage API health JSON is not ok".to_string());
    }
    Ok(())
}

fn public_config_snapshot(config: &LimiterConfig) -> PublicConfigSnapshot {
    PublicConfigSnapshot {
        default_model: config.default_model.clone(),
        upstream_model: config.upstream_model.clone(),
        accepted_models: config.accepted_models(),
        upstream_base_url: config.upstream_base_url.clone(),
        upstream_health_path: config.upstream_health_path.clone(),
        usage_api_base_url: config.finite_usage_api_url.clone(),
        usage_api_health_path: config.usage_api_health_path.clone(),
        readiness_timeout_ms: config.readiness_timeout.as_millis(),
        usage_api_timeout_ms: config.usage_api_timeout.as_millis(),
        upstream_first_byte_timeout_ms: config.upstream_first_byte_timeout.as_millis(),
        upstream_body_timeout_ms: config.upstream_body_timeout.as_millis(),
        upstream_stream_idle_timeout_ms: config.upstream_stream_idle_timeout.as_millis(),
        admission_mode: config.admission_mode.as_str().to_string(),
        admission_allowlist_entries: config.admission_allowlist.len(),
        default_reasoning_effort: config.default_reasoning_effort.clone(),
        default_enable_thinking: config.default_enable_thinking,
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
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> Response {
    let body = match body {
        Ok(body) => body,
        Err(rejection) => {
            // Axum rejects bodies over its configured limit before entering
            // the request logic. Count that boundary explicitly while
            // preserving Axum's existing 413 response and inference path.
            state.metrics.request_observed();
            state.metrics.refusal("other");
            state.metrics.admission(false);
            state.metrics.terminal(TerminalOutcome::AdmissionError);
            return rejection.into_response();
        }
    };
    let Some(presented_api_key) = bearer_token(&headers) else {
        state.metrics.request_observed();
        state.metrics.refusal("invalid_api_key");
        state.metrics.admission(false);
        state.metrics.terminal(TerminalOutcome::AdmissionError);
        return openai_error(
            StatusCode::UNAUTHORIZED,
            "Finite Private API key is required.",
            "invalid_api_key",
            "missing_authorization",
        );
    };
    // Single decode of the request body; every consumer below reads the parsed
    // value or the extracted facts instead of re-parsing. Unparseable bodies
    // decode to Null so they are still forwarded (fail-open).
    let parsed_body = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let mut facts = RequestFacts::from_value(&parsed_body, &state.config.default_model);
    let Some(routed_model) = state.config.routed_model(&facts.model) else {
        state.metrics.request_observed();
        state.metrics.refusal("unsupported_model");
        state.metrics.admission(false);
        state.metrics.terminal(TerminalOutcome::AdmissionError);
        return openai_error(
            StatusCode::BAD_REQUEST,
            "The requested model is not available through Finite Private.",
            "invalid_request_error",
            "unsupported_model",
        );
    };
    facts.model = routed_model.clone();
    let telemetry_model = state.config.telemetry_model(&facts.model);
    let is_streaming = facts.streaming;
    if uri.path() == "/v1/responses" && is_streaming {
        state.metrics.request_observed();
        state.metrics.refusal("unsupported_streaming_endpoint");
        state.metrics.admission(false);
        state.metrics.terminal(TerminalOutcome::AdmissionError);
        return openai_error(
            StatusCode::BAD_REQUEST,
            "Finite Private streaming /v1/responses requests are not supported yet.",
            "invalid_request_error",
            "unsupported_streaming_endpoint",
        );
    }

    let request_id = new_request_id();
    let request_timer = state
        .metrics
        .request_started_for_route(telemetry_model, uri.path());
    let estimate = estimate_usage(&facts);
    let degraded_admission = state.config.admission_mode == AdmissionMode::Allowlist;
    if degraded_admission
        && !state
            .config
            .admission_allowlist
            .contains(&presented_api_key)
    {
        state.metrics.refusal("invalid_api_key");
        state
            .metrics
            .admission_for_route(telemetry_model, uri.path(), false);
        request_timer.finish(TerminalOutcome::AdmissionError);
        return openai_error(
            StatusCode::UNAUTHORIZED,
            "The provided Finite Private API key is not accepted in degraded admission mode.",
            "invalid_api_key",
            "invalid_api_key",
        );
    }
    let reservation_id = if degraded_admission {
        state
            .metrics
            .admission_for_route(telemetry_model, uri.path(), true);
        eprintln!(
            "finite-private-limiter degraded admission: request {request_id} admitted via allowlist (settlement skipped)"
        );
        format!("degraded-{request_id}")
    } else {
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
                state.metrics.refusal("usage_api_unavailable");
                state
                    .metrics
                    .admission_for_route(telemetry_model, uri.path(), false);
                request_timer.finish(TerminalOutcome::AdmissionError);
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
            state.metrics.refusal(
                reserve_decision
                    .error
                    .as_ref()
                    .map(|error| error.code.as_str())
                    .unwrap_or("other"),
            );
            state
                .metrics
                .admission_for_route(telemetry_model, uri.path(), false);
            request_timer.finish(TerminalOutcome::AdmissionError);
            return denied_response(reserve_decision);
        }

        let Some(reservation_id) = reserve_decision.reservation_id.clone() else {
            state
                .metrics
                .admission_for_route(telemetry_model, uri.path(), false);
            request_timer.finish(TerminalOutcome::AdmissionError);
            return openai_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Finite Private usage admission did not return a reservation.",
                "usage_api_invalid_response",
                "usage_api_invalid_response",
            );
        };
        state
            .metrics
            .admission_for_route(telemetry_model, uri.path(), true);
        reservation_id
    };

    let rewrite_model = state
        .config
        .upstream_model
        .as_ref()
        .map(|_| routed_model.as_str());
    let upstream_body = upstream_body_for_request(
        &uri,
        body,
        parsed_body,
        is_streaming,
        rewrite_model,
        &state.config,
    );
    if is_streaming {
        let upstream = match call_upstream_response(&state, &uri, upstream_body).await {
            Ok(response) => response,
            Err(error) => {
                request_timer.finish(if error.contains("timed out") {
                    TerminalOutcome::UpstreamTimeout
                } else {
                    TerminalOutcome::UpstreamError
                });
                eprintln!("finite-private-limiter upstream failed: {error}");
                let diagnostic_request_id = request_id.clone();
                let _settlement_result = settle_usage_with_retries(
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
                let (first_output_ms, first_answer_ms, duration_ms) = request_timer.timing();
                record_diagnostic_best_effort(
                    &state,
                    &reservation_id,
                    DiagnosticRequest {
                        request_id: diagnostic_request_id,
                        prompt_tokens: None,
                        completion_tokens: None,
                        first_output_ms,
                        first_answer_ms,
                        duration_ms,
                        termination_reason: "upstream_error".to_string(),
                        measurement_quality: "estimated_usage".to_string(),
                    },
                );
                return openai_error(
                    StatusCode::BAD_GATEWAY,
                    "Finite Private upstream is unavailable.",
                    "upstream_unavailable",
                    "upstream_unavailable",
                );
            }
        };
        return streaming_response(
            state,
            upstream,
            reservation_id,
            request_id,
            degraded_admission,
            request_timer,
        );
    }

    let upstream = match call_upstream(&state, &uri, upstream_body).await {
        Ok(response) => response,
        Err(error) => {
            request_timer.finish(if error.contains("timed out") {
                TerminalOutcome::UpstreamTimeout
            } else {
                TerminalOutcome::UpstreamError
            });
            eprintln!("finite-private-limiter upstream failed: {error}");
            let diagnostic_request_id = request_id.clone();
            let _settlement_result = settle_usage_with_retries(
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
            let (first_output_ms, first_answer_ms, duration_ms) = request_timer.timing();
            record_diagnostic_best_effort(
                &state,
                &reservation_id,
                DiagnosticRequest {
                    request_id: diagnostic_request_id,
                    prompt_tokens: None,
                    completion_tokens: None,
                    first_output_ms,
                    first_answer_ms,
                    duration_ms,
                    termination_reason: "upstream_error".to_string(),
                    measurement_quality: "estimated_usage".to_string(),
                },
            );
            return openai_error(
                StatusCode::BAD_GATEWAY,
                "Finite Private upstream is unavailable.",
                "upstream_unavailable",
                "upstream_unavailable",
            );
        }
    };

    let actual = actual_usage(&upstream.body, &state.config.default_model);
    let diagnostic_request_id = request_id.clone();
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
    // Freeze limiter-observed generation latency before Core settlement
    // retries; accounting service delay must not inflate model timing.
    request_timer.finish(if upstream.status.is_success() {
        TerminalOutcome::Success
    } else {
        TerminalOutcome::UpstreamError
    });
    request_timer.tokens(
        actual.as_ref().map(|usage| usage.prompt_tokens),
        actual.as_ref().map(|usage| usage.completion_tokens),
    );
    if let Err(error) = settle_usage_with_retries(&state, &reservation_id, settle).await {
        eprintln!("finite-private-limiter settle failed: {error}");
    }
    let (first_output_ms, first_answer_ms, duration_ms) = request_timer.timing();
    record_diagnostic_best_effort(
        &state,
        &reservation_id,
        DiagnosticRequest {
            request_id: diagnostic_request_id,
            prompt_tokens: actual.as_ref().map(|usage| usage.prompt_tokens),
            completion_tokens: actual.as_ref().map(|usage| usage.completion_tokens),
            first_output_ms,
            first_answer_ms,
            duration_ms,
            termination_reason: if upstream.status.is_success() {
                "complete"
            } else {
                "upstream_error"
            }
            .to_string(),
            measurement_quality: if actual.is_some() {
                "observed_usage"
            } else {
                "estimated_usage"
            }
            .to_string(),
        },
    );

    let mut response = Response::builder().status(upstream.status);
    if let Some(content_type) = upstream.content_type {
        response = response.header("content-type", content_type);
    }
    if degraded_admission {
        response = response.header("x-finite-admission", "degraded-allowlist");
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
    if state.config.admission_mode == AdmissionMode::Allowlist {
        // Degraded mode never settles: there is no reservation to settle and
        // the usage API is intentionally out of the request path. Usage is
        // unaccounted for the duration of this mode.
        return Ok(());
    }
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
    if state.config.admission_mode == AdmissionMode::Allowlist {
        return Ok(());
    }
    let actual = input.settlement == "actual";
    let mut last_error = String::new();
    for attempt in 1..=3 {
        match settle_usage(state, reservation_id, &input).await {
            Ok(()) => {
                state.metrics.settlement(actual, true);
                return Ok(());
            }
            Err(error) => {
                last_error = error;
                if attempt == 3 {
                    break;
                }
                sleep(Duration::from_millis(250 * attempt as u64)).await;
            }
        }
    }
    state.metrics.settlement(actual, false);
    Err(format!("Core settle failed after 3 attempts: {last_error}"))
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DiagnosticRequest {
    request_id: String,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    first_output_ms: Option<i64>,
    first_answer_ms: Option<i64>,
    duration_ms: Option<i64>,
    termination_reason: String,
    measurement_quality: String,
}

fn record_diagnostic_best_effort(state: &AppState, reservation_id: &str, input: DiagnosticRequest) {
    if state.config.admission_mode == AdmissionMode::Allowlist {
        return;
    }
    let state = state.clone();
    let reservation_id = reservation_id.to_string();
    let Ok(slot) = state.diagnostic_slots.clone().try_acquire_owned() else {
        state.metrics.diagnostics_failed();
        return;
    };
    tokio::spawn(async move {
        let _slot = slot;
        let url = format!(
            "{}/internal/finite-private/v1/request-diagnostics",
            state.config.finite_usage_api_url.trim_end_matches('/')
        );
        let request = state
            .client
            .post(url)
            .bearer_auth(&state.config.finite_usage_api_service_key)
            .json(&serde_json::json!({
                "reservationId": reservation_id,
                "requestId": input.request_id,
                "promptTokens": input.prompt_tokens,
                "completionTokens": input.completion_tokens,
                "firstOutputMs": input.first_output_ms,
                "firstAnswerMs": input.first_answer_ms,
                "durationMs": input.duration_ms,
                "terminationReason": input.termination_reason,
                "measurementQuality": input.measurement_quality,
            }))
            .send();
        let result = timeout(state.config.usage_api_timeout, request).await;
        match result {
            Ok(Ok(response)) if response.status().is_success() => {}
            _ => state.metrics.diagnostics_failed(),
        }
    });
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
    degraded_admission: bool,
    request_timer: RequestTimer,
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
    if degraded_admission {
        response = response.header("x-finite-admission", "degraded-allowlist");
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
    // Construct the guard before polling begins so a client that disconnects
    // immediately after headers still receives fallback accounting and a
    // cancellation diagnostic.
    let settlement_guard = SettlementGuard::new(settlement, fallback_settle, request_timer);
    let body_stream = async_stream::stream! {
        let mut settlement_guard = settlement_guard;
        let mut accumulator = StreamingUsageAccumulator::new(default_model);
        loop {
            let item = match timeout(idle_timeout, stream.next()).await {
                Ok(item) => item,
                Err(_) => {
                    eprintln!(
                        "finite-private-limiter upstream stream idle timeout after {}ms",
                        idle_timeout.as_millis()
                    );
                    settlement_guard.timer().finish(TerminalOutcome::UpstreamTimeout);
                    settlement_guard.preserve_terminal_diagnostic("upstream_stream_timeout");
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
                    let (first_output_ms, first_answer_ms, duration_ms) = settlement_guard.timer().timing();
                    record_diagnostic_best_effort(
                        settlement_guard.state(),
                        settlement_guard.reservation_id(),
                        DiagnosticRequest {
                            request_id: settlement_guard.request_id().to_string(),
                            prompt_tokens: None,
                            completion_tokens: None,
                            first_output_ms,
                            first_answer_ms,
                            duration_ms,
                            termination_reason: "upstream_stream_timeout".to_string(),
                            measurement_quality: "estimated_usage".to_string(),
                        },
                    );
                    settlement_guard.mark_diagnostic_recorded();
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
                    if accumulator.saw_meaningful_output() {
                        settlement_guard.timer().observe_first_output();
                    }
                    if accumulator.saw_answer_text() {
                        settlement_guard.timer().observe_first_answer();
                    }
                    let saw_done = accumulator.saw_done();
                    yield Ok::<Bytes, std::io::Error>(chunk);
                    if saw_done {
                        break;
                    }
                }
                Err(error) => {
                    eprintln!("finite-private-limiter upstream stream failed: {error}");
                    settlement_guard.timer().finish(TerminalOutcome::UpstreamError);
                    settlement_guard.preserve_terminal_diagnostic("upstream_stream_error");
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
                    let (first_output_ms, first_answer_ms, duration_ms) = settlement_guard.timer().timing();
                    record_diagnostic_best_effort(
                        settlement_guard.state(),
                        settlement_guard.reservation_id(),
                        DiagnosticRequest {
                            request_id: settlement_guard.request_id().to_string(),
                            prompt_tokens: None,
                            completion_tokens: None,
                            first_output_ms,
                            first_answer_ms,
                            duration_ms,
                            termination_reason: "upstream_stream_error".to_string(),
                            measurement_quality: "estimated_usage".to_string(),
                        },
                    );
                    settlement_guard.mark_diagnostic_recorded();
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
        settlement_guard.timer().finish(if status.is_success() {
            TerminalOutcome::Success
        } else {
            TerminalOutcome::UpstreamError
        });
        settlement_guard.timer().tokens(
            actual.as_ref().map(|usage| usage.prompt_tokens),
            actual.as_ref().map(|usage| usage.completion_tokens),
        );
        let (first_output_ms, first_answer_ms, duration_ms) = settlement_guard.timer().timing();
        let diagnostic = DiagnosticRequest {
                request_id: settlement_guard.request_id().to_string(),
                prompt_tokens: actual.as_ref().map(|usage| usage.prompt_tokens),
                completion_tokens: actual.as_ref().map(|usage| usage.completion_tokens),
                first_output_ms,
                first_answer_ms,
                duration_ms,
                termination_reason: if status.is_success() {
                    "complete"
                } else {
                    "upstream_error"
                }
                .to_string(),
                measurement_quality: if actual.is_some() {
                    "observed_usage"
                } else {
                    "estimated_usage"
                }
                .to_string(),
            };
        // Preserve known final usage if the client drops while Core retries.
        settlement_guard.pending_diagnostic = Some(diagnostic.clone());
        let settlement_ok = settlement_guard.settle(settle).await;
        if let Err(error) = &settlement_ok {
            eprintln!("finite-private-limiter streaming settle failed: {error}");
        }
        record_diagnostic_best_effort(settlement_guard.state(), settlement_guard.reservation_id(), diagnostic);
        settlement_guard.mark_diagnostic_recorded();
    };
    response.body(Body::from_stream(body_stream)).unwrap()
}

#[derive(Clone)]
struct Settlement {
    state: AppState,
    reservation_id: Arc<str>,
    request_id: Arc<str>,
    started: Arc<std::sync::atomic::AtomicBool>,
    result: watch::Sender<Option<Result<(), String>>>,
}

impl Settlement {
    fn new(state: AppState, reservation_id: String, request_id: String) -> Self {
        Self {
            state,
            reservation_id: Arc::from(reservation_id),
            request_id: Arc::from(request_id),
            started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            result: watch::channel(None).0,
        }
    }

    fn request_id(&self) -> &str {
        &self.request_id
    }

    fn reservation_id(&self) -> &str {
        &self.reservation_id
    }

    async fn settle(&self, request: SettleRequest) -> Result<(), String> {
        let mut receiver = self.result.subscribe();
        if !self.started.swap(true, Ordering::SeqCst) {
            let settlement = self.clone();
            // Accounting survives a client dropping the response while Core
            // retries. Every waiter observes this same operation and result.
            tokio::spawn(async move {
                let result = settle_usage_with_retries(
                    &settlement.state,
                    &settlement.reservation_id,
                    request,
                )
                .await;
                settlement.result.send_replace(Some(result));
            });
        }
        loop {
            if let Some(result) = receiver.borrow_and_update().clone() {
                return result;
            }
            receiver
                .changed()
                .await
                .map_err(|_| "settlement task unavailable".to_string())?;
        }
    }
}

struct SettlementGuard {
    settlement: Settlement,
    fallback: SettleRequest,
    request_timer: RequestTimer,
    diagnostic_recorded: bool,
    pending_diagnostic: Option<DiagnosticRequest>,
}

impl SettlementGuard {
    fn new(settlement: Settlement, fallback: SettleRequest, request_timer: RequestTimer) -> Self {
        Self {
            settlement,
            fallback,
            request_timer,
            diagnostic_recorded: false,
            pending_diagnostic: None,
        }
    }

    fn request_id(&self) -> &str {
        self.settlement.request_id()
    }

    fn reservation_id(&self) -> &str {
        self.settlement.reservation_id()
    }

    fn state(&self) -> &AppState {
        &self.settlement.state
    }

    fn timer(&self) -> &RequestTimer {
        &self.request_timer
    }

    fn preserve_terminal_diagnostic(&mut self, reason: &str) {
        let (first_output_ms, first_answer_ms, duration_ms) = self.request_timer.timing();
        self.pending_diagnostic = Some(DiagnosticRequest {
            request_id: self.request_id().to_string(),
            prompt_tokens: None,
            completion_tokens: None,
            first_output_ms,
            first_answer_ms,
            duration_ms,
            termination_reason: reason.to_string(),
            measurement_quality: "estimated_usage".into(),
        });
    }

    fn mark_diagnostic_recorded(&mut self) {
        self.diagnostic_recorded = true;
    }

    async fn settle(&self, request: SettleRequest) -> Result<(), String> {
        self.settlement.settle(request).await
    }
}

impl Drop for SettlementGuard {
    fn drop(&mut self) {
        if !self.diagnostic_recorded {
            self.request_timer
                .finish(TerminalOutcome::ClientCancellation);
            let (first_output_ms, first_answer_ms, duration_ms) = self.request_timer.timing();
            let diagnostic = self
                .pending_diagnostic
                .take()
                .unwrap_or_else(|| DiagnosticRequest {
                    request_id: self.request_id().to_string(),
                    prompt_tokens: None,
                    completion_tokens: None,
                    first_output_ms,
                    first_answer_ms,
                    duration_ms,
                    termination_reason: "client_disconnected_or_stream_cancelled".to_string(),
                    measurement_quality: "estimated_usage".to_string(),
                });
            let settlement = self.settlement.clone();
            let fallback = self.fallback.clone();
            tokio::spawn(async move {
                if let Err(error) = settlement.settle(fallback).await {
                    eprintln!("finite-private-limiter background settle failed: {error}");
                }
                record_diagnostic_best_effort(
                    &settlement.state,
                    settlement.reservation_id(),
                    diagnostic,
                );
            });
        }
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

fn upstream_body_for_request(
    uri: &Uri,
    body: Bytes,
    mut value: Value,
    streaming: bool,
    routed_model: Option<&str>,
    config: &LimiterConfig,
) -> Bytes {
    let is_chat = uri.path() == "/v1/chat/completions";
    let add_stream_usage = is_chat && streaming;
    let thinking_defaults = is_chat
        && (config.default_reasoning_effort.is_some() || config.default_enable_thinking.is_some());
    if !add_stream_usage && routed_model.is_none() && !thinking_defaults {
        return body;
    }
    let Some(object) = value.as_object_mut() else {
        return body;
    };
    if add_stream_usage {
        let stream_options = object.entry("stream_options").or_insert_with(|| json!({}));
        if let Some(options) = stream_options.as_object_mut() {
            options.insert("include_usage".to_string(), Value::Bool(true));
        }
    }
    if let Some(routed_model) = routed_model {
        object.insert("model".to_string(), Value::String(routed_model.to_string()));
    }
    if thinking_defaults {
        apply_thinking_defaults(
            object,
            config.default_reasoning_effort.as_deref(),
            config.default_enable_thinking,
        );
    }
    serde_json::to_vec(&value).map(Bytes::from).unwrap_or(body)
}

/// Fill omitted thinking fields. Never overwrite a client-supplied value.
fn apply_thinking_defaults(
    object: &mut serde_json::Map<String, Value>,
    default_reasoning_effort: Option<&str>,
    default_enable_thinking: Option<bool>,
) {
    if let Some(effort) = default_reasoning_effort
        && !object.contains_key("reasoning_effort")
    {
        object.insert(
            "reasoning_effort".to_string(),
            Value::String(effort.to_string()),
        );
    }
    if let Some(enable_thinking) = default_enable_thinking {
        let kwargs = object
            .entry("chat_template_kwargs")
            .or_insert_with(|| json!({}));
        let Some(kwargs) = kwargs.as_object_mut() else {
            return;
        };
        if !kwargs.contains_key("enable_thinking") && !kwargs.contains_key("thinking") {
            kwargs.insert("enable_thinking".to_string(), Value::Bool(enable_thinking));
        }
    }
}

fn estimate_usage(facts: &RequestFacts) -> EstimatedUsage {
    let prompt_tokens = (facts.prompt_chars / 4).max(1);
    let completion_tokens = facts.declared_max_completion_tokens.unwrap_or(4096).max(1);
    let usage_units = usage_units(prompt_tokens, completion_tokens, &facts.model);
    EstimatedUsage {
        model: facts.model.clone(),
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
    meaningful_output: bool,
    answer_text: bool,
    default_model: String,
}

impl StreamingUsageAccumulator {
    fn new(default_model: String) -> Self {
        Self {
            pending_line: String::new(),
            actual_usage: None,
            saw_done: false,
            meaningful_output: false,
            answer_text: false,
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
        let (meaningful_output, answer_text) = output_signals(&value);
        self.meaningful_output |= meaningful_output;
        self.answer_text |= answer_text;
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

    fn saw_meaningful_output(&self) -> bool {
        self.meaningful_output
    }

    fn saw_answer_text(&self) -> bool {
        self.answer_text
    }
}

fn output_signals(value: &Value) -> (bool, bool) {
    let mut meaningful = false;
    let mut answer = false;
    if let Some(choices) = value.get("choices").and_then(Value::as_array) {
        for choice in choices {
            if let Some(delta) = choice.get("delta") {
                let content = delta.get("content").and_then(Value::as_str);
                let reasoning = delta
                    .get("reasoning_content")
                    .or_else(|| delta.get("reasoning"))
                    .and_then(Value::as_str);
                if content.is_some_and(|text| !text.is_empty()) {
                    meaningful = true;
                    answer = true;
                }
                let tool_output = delta
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .is_some_and(|calls| {
                        calls.iter().any(|call| {
                            call.get("function")
                                .and_then(|function| {
                                    function.get("arguments").and_then(Value::as_str)
                                })
                                .is_some_and(|text| !text.is_empty())
                        })
                    });
                if reasoning.is_some_and(|text| !text.is_empty()) || tool_output {
                    meaningful = true;
                }
            }
        }
    }
    if let Some(items) = value.get("output").and_then(Value::as_array) {
        for item in items {
            let has_text = item
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty())
                || item
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|content| {
                        content.iter().any(|part| {
                            part.get("text")
                                .and_then(Value::as_str)
                                .is_some_and(|text| !text.is_empty())
                        })
                    });
            let has_tool_payload = item
                .get("arguments")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty());
            match item.get("type").and_then(Value::as_str) {
                Some("output_text") => {
                    if has_text {
                        meaningful = true;
                        answer = true;
                    }
                }
                Some("tool_call") | Some("function_call") | Some("reasoning") => {
                    if has_tool_payload || has_text {
                        meaningful = true;
                    }
                }
                _ => {}
            }
        }
    }
    (meaningful, answer)
}

/// Facts extracted from the single request-body decode in `proxy_openai`, so
/// the streaming gate, usage estimate, and upstream body mutation never
/// re-parse the body.
#[derive(Debug)]
struct RequestFacts {
    streaming: bool,
    model: String,
    prompt_chars: i64,
    declared_max_completion_tokens: Option<i64>,
}

impl RequestFacts {
    fn from_value(value: &Value, default_model: &str) -> Self {
        Self {
            streaming: value
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            model: value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(default_model)
                .to_string(),
            prompt_chars: prompt_text(value).chars().count() as i64,
            declared_max_completion_tokens: value
                .get("max_completion_tokens")
                .or_else(|| value.get("max_tokens"))
                .and_then(Value::as_i64),
        }
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
    upstream_model: Option<String>,
    accepted_models: Vec<String>,
    upstream_base_url: String,
    upstream_health_path: String,
    usage_api_base_url: String,
    usage_api_health_path: String,
    readiness_timeout_ms: u128,
    usage_api_timeout_ms: u128,
    upstream_first_byte_timeout_ms: u128,
    upstream_body_timeout_ms: u128,
    upstream_stream_idle_timeout_ms: u128,
    admission_mode: String,
    admission_allowlist_entries: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_enable_thinking: Option<bool>,
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
    use tower::ServiceExt;

    #[test]
    fn stream_output_signals_require_payload() {
        assert_eq!(
            output_signals(&json!({"choices":[{"delta":{"tool_calls":[{"index":0}]}}]})),
            (false, false)
        );
        assert_eq!(
            output_signals(
                &json!({"choices":[{"delta":{"tool_calls":[{"function":{"name":"lookup"}}]}}]})
            ),
            (false, false)
        );
        assert_eq!(
            output_signals(&json!({"output":[{"type":"output_text"}]})),
            (false, false)
        );
        assert_eq!(
            output_signals(
                &json!({"output":[{"type":"output_text","content":[{"text":"hello"}]}]})
            ),
            (true, true)
        );
    }

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
    async fn degraded_allowlist_admits_listed_key_without_core_calls() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;

        let mut config = test_config(core_url, upstream_url);
        config.admission_mode = AdmissionMode::Allowlist;
        config.admission_allowlist = vec!["fpk_degraded_key".to_string()];
        let limiter_url = spawn(app(config).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_degraded_key")
            .header("x-request-id", "req-degraded-ok")
            .json(&json!({
                "model": "glm-5-2",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-finite-admission")
                .and_then(|value| value.to_str().ok()),
            Some("degraded-allowlist")
        );
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "ok");
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        // Degraded mode must never touch the usage API.
        assert_eq!(core.reserve_calls.load(Ordering::SeqCst), 0);
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 0);

        // Health stays green in degraded mode even with the usage API down,
        // while still reporting the real usage-api component state.
        let health: Value = client
            .get(format!("{limiter_url}/health"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["ok"], json!(true));
        assert_eq!(health["config"]["admissionMode"], json!("allowlist"));
        assert_eq!(health["config"]["admissionAllowlistEntries"], json!(1));
    }

    #[tokio::test]
    async fn degraded_allowlist_rejects_unlisted_key_before_upstream() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;

        let mut config = test_config(core_url, upstream_url);
        config.admission_mode = AdmissionMode::Allowlist;
        config.admission_allowlist = vec!["fpk_degraded_key".to_string()];
        let limiter_url = spawn(app(config).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("some-other-key")
            .header("x-request-id", "req-degraded-denied")
            .json(&json!({
                "model": "glm-5-2",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], json!("invalid_api_key"));
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 0);
        assert_eq!(core.reserve_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn allowlist_mode_without_keys_fails_validation() {
        let mut config = test_config(
            "http://127.0.0.1:1".to_string(),
            "http://127.0.0.1:2".to_string(),
        );
        config.admission_mode = AdmissionMode::Allowlist;
        config.admission_allowlist = Vec::new();
        assert!(app(config).is_err());
    }

    #[tokio::test]
    async fn usage_api_health_does_not_follow_html_outage_redirect() {
        let origin = spawn(public_outage_origin_router()).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let mut config = test_config(origin, upstream_url);
        config.admission_mode = AdmissionMode::Allowlist;
        config.admission_allowlist = vec!["fpk_degraded_key".to_string()];
        let limiter_url = spawn(app(config).unwrap()).await;

        let health: Value = reqwest::Client::new()
            .get(format!("{limiter_url}/health"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        // Allowlist keeps the limiter ready, but must not report the HTML
        // origin as a healthy usage API.
        assert_eq!(health["ok"], json!(true));
        assert_eq!(health["components"]["upstream"]["ok"], json!(true));
        assert_eq!(health["components"]["usageApi"]["ok"], json!(false));
        assert_eq!(health["components"]["usageApi"]["status"], json!(307));
    }

    #[tokio::test]
    async fn usage_api_mode_is_not_ready_when_origin_is_html_outage_page() {
        let origin = spawn(public_outage_origin_router()).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let limiter_url = spawn(app(test_config(origin, upstream_url)).unwrap()).await;

        let response = reqwest::Client::new()
            .get(format!("{limiter_url}/ready"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let health: Value = response.json().await.unwrap();
        assert_eq!(health["ok"], json!(false));
        assert_eq!(health["config"]["admissionMode"], json!("usage-api"));
        assert_eq!(health["components"]["usageApi"]["ok"], json!(false));
    }

    #[tokio::test]
    async fn usage_api_health_rejects_html_200_at_the_health_path() {
        let origin = spawn(html_health_origin_router()).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let limiter_url = spawn(app(test_config(origin, upstream_url)).unwrap()).await;

        let health: Value = reqwest::Client::new()
            .get(format!("{limiter_url}/health"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["ok"], json!(false));
        assert_eq!(health["components"]["usageApi"]["ok"], json!(false));
        assert_eq!(health["components"]["usageApi"]["status"], json!(200));
        let error = health["components"]["usageApi"]["error"]
            .as_str()
            .unwrap_or_default();
        assert!(
            error.contains("application/json"),
            "expected content-type rejection, got {error:?}"
        );
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
    async fn non_streaming_body_is_forwarded_byte_identical() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let limiter_url = spawn(app(test_config(core_url, upstream_url)).unwrap()).await;

        // Whitespace and key order must survive verbatim: the proxy decodes the
        // body once for facts but forwards the original bytes untouched.
        let raw = r#"{  "max_tokens": 64, "messages": [ { "role": "user", "content": "hello" } ],  "model": "glm-5-2" }"#;
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_secret")
            .header("content-type", "application/json")
            .body(raw)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        let bodies = upstream.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(&bodies[0][..], raw.as_bytes());
    }

    #[tokio::test]
    async fn streaming_body_is_rewritten_with_include_usage() {
        let core = FakeCoreState::new("fpk_live_stream", 10_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let limiter_url = spawn(app(test_config(core_url, upstream_url)).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_stream")
            .header("content-type", "application/json")
            .body(
                r#"{"model":"glm-5-2","stream":true,"messages":[{"role":"user","content":"hello"}],"max_tokens":64}"#,
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.unwrap();
        assert!(body.contains("data: [DONE]"));
        let bodies = upstream.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        let forwarded: Value = serde_json::from_slice(&bodies[0]).unwrap();
        assert_eq!(forwarded["stream_options"]["include_usage"], true);
        assert_eq!(forwarded["stream"], true);
        assert_eq!(forwarded["model"], "glm-5-2");
    }

    #[tokio::test]
    async fn omitted_thinking_fields_take_configured_defaults() {
        let core = FakeCoreState::new("fpk_live_thinking", 10_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let mut config = test_config(core_url, upstream_url);
        config.default_reasoning_effort = Some("high".to_string());
        config.default_enable_thinking = Some(true);
        let limiter_url = spawn(app(config).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_thinking")
            .json(&json!({
                "model": "glm-5-2",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bodies = upstream.bodies.lock().unwrap();
        let forwarded: Value = serde_json::from_slice(&bodies[0]).unwrap();
        assert_eq!(forwarded["reasoning_effort"], "high");
        assert_eq!(forwarded["chat_template_kwargs"]["enable_thinking"], true);
    }

    #[tokio::test]
    async fn explicit_thinking_fields_are_not_overwritten() {
        let core = FakeCoreState::new("fpk_live_thinking_explicit", 10_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let mut config = test_config(core_url, upstream_url);
        config.default_reasoning_effort = Some("high".to_string());
        config.default_enable_thinking = Some(true);
        let limiter_url = spawn(app(config).unwrap()).await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_thinking_explicit")
            .json(&json!({
                "model": "glm-5-2",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64,
                "reasoning_effort": "max",
                "chat_template_kwargs": { "enable_thinking": false }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bodies = upstream.bodies.lock().unwrap();
        let forwarded: Value = serde_json::from_slice(&bodies[0]).unwrap();
        assert_eq!(forwarded["reasoning_effort"], "max");
        assert_eq!(forwarded["chat_template_kwargs"]["enable_thinking"], false);
    }

    #[tokio::test]
    async fn configured_model_aliases_route_to_one_upstream_model() {
        let core = FakeCoreState::new("fpk_live_aliases", 10_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let mut config = test_config(core_url, upstream_url);
        config.default_model = "glm-5-3-flash".to_string();
        config.upstream_model = Some("glm-5-3-flash".to_string());
        config.model_aliases = vec![
            "deepseek-v4-flash-0731".to_string(),
            "glm-5-2".to_string(),
            "glm-5.3-flash".to_string(),
        ];
        let limiter_url = spawn(app(config).unwrap()).await;

        let client = reqwest::Client::new();
        for requested_model in [
            "glm-5-3-flash",
            "deepseek-v4-flash-0731",
            "glm-5-2",
            "glm-5.3-flash",
        ] {
            let response = client
                .post(format!("{limiter_url}/v1/chat/completions"))
                .bearer_auth("fpk_live_aliases")
                .json(&json!({
                    "model": requested_model,
                    "messages": [{ "role": "user", "content": "hello" }],
                    "max_tokens": 64
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        let bodies = upstream.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 4);
        for body in bodies.iter() {
            let forwarded: Value = serde_json::from_slice(body).unwrap();
            assert_eq!(forwarded["model"], "glm-5-3-flash");
        }
        let reservations = core.reservations.lock().unwrap();
        assert_eq!(reservations.len(), 4);
        assert!(
            reservations
                .iter()
                .all(|reservation| reservation["model"] == "glm-5-3-flash")
        );
    }

    #[tokio::test]
    async fn configured_model_routing_rejects_unknown_model_before_reservation() {
        let core = FakeCoreState::new("fpk_live_aliases", 10_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let mut config = test_config(core_url, upstream_url);
        config.default_model = "glm-5-3-flash".to_string();
        config.upstream_model = Some("glm-5-3-flash".to_string());
        config.model_aliases = vec!["deepseek-v4-flash-0731".to_string()];
        let limiter_url = spawn(app(config).unwrap()).await;

        let response = reqwest::Client::new()
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_aliases")
            .json(&json!({
                "model": "some-other-model",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "unsupported_model");
        assert_eq!(core.reserve_calls.load(Ordering::SeqCst), 0);
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn malformed_body_is_still_proxied() {
        let core = FakeCoreState::new("fpk_live_secret", 1_000_000);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream = FakeUpstreamState::new();
        let upstream_url = spawn(fake_upstream_router(upstream.clone())).await;
        let limiter_url = spawn(app(test_config(core_url, upstream_url)).unwrap()).await;

        // Fail-open: an unparseable body is forwarded verbatim, not rejected.
        let raw = b"{ this is not json".as_ref();
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_secret")
            .header("content-type", "application/json")
            .body(raw)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 1);
        assert_eq!(core.reserve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 1);
        let bodies = upstream.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(&bodies[0][..], raw);
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
        assert_eq!(live["config"]["defaultModel"], "glm-5-3-flash");
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
        assert_eq!(ready["config"]["defaultModel"], "glm-5-3-flash");
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
        let value = serde_json::from_slice::<Value>(
            br#"{"messages":[{"role":"user","content":"hello"}],"max_tokens":8}"#,
        )
        .unwrap();
        let estimate = estimate_usage(&RequestFacts::from_value(&value, "glm-5-2"));
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

    #[tokio::test]
    async fn diagnostic_timings_ignore_roles_separate_reasoning_and_exclude_settlement_retry() {
        for answer in [true, false] {
            let core = FakeCoreState::new("fpk_live_timing", 10_000_000);
            core.settle_failures_remaining.store(1, Ordering::SeqCst);
            let core_url = spawn(fake_core_router(core.clone())).await;
            let upstream_url = spawn(Router::new().route("/v1/chat/completions", post(move || async move {
                let stream = async_stream::stream! {
                    yield Ok::<Bytes, std::io::Error>(Bytes::from_static(b": keepalive\n\ndata: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"function\":{\"name\":\"lookup\"}}]}}]}\n\n"));
                    sleep(Duration::from_millis(50)).await;
                    let delta = if answer { json!({"reasoning_content":"private reasoning fixture"}) }
                        else { json!({"tool_calls":[{"function":{"name":"","arguments":"private tool fixture"}}]}) };
                    yield Ok(Bytes::from(format!("data: {}\n\n", json!({"choices":[{"delta":delta}]}))));
                    sleep(Duration::from_millis(50)).await;
                    if answer {
                        yield Ok(Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"private answer fixture\"}}]}\n\n"));
                        yield Ok(Bytes::from_static(b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":19}}\n\n"));
                    }
                    yield Ok(Bytes::from_static(b"data: [DONE]\n\n"));
                };
                ([("content-type", "text/event-stream")], Body::from_stream(stream))
            }))).await;
            let limiter_url = spawn(app(test_config(core_url, upstream_url)).unwrap()).await;
            let started = Instant::now();
            let response = reqwest::Client::new()
                .post(format!("{limiter_url}/v1/chat/completions"))
                .bearer_auth("fpk_live_timing")
                .json(&json!({"model":"glm-5-2","stream":true,"messages":[]}))
                .send()
                .await
                .unwrap();
            let body = response.text().await.unwrap();
            assert!(body.contains("[DONE]"));
            let wall_ms = started.elapsed().as_millis() as i64;
            wait_for(|| core.diagnostics.lock().unwrap().len() == 1).await;
            let diagnostic = core.diagnostics.lock().unwrap()[0].clone();
            let first = diagnostic["firstOutputMs"].as_i64().unwrap();
            let duration = diagnostic["durationMs"].as_i64().unwrap();
            assert!(first >= 40, "headers/role-only must not count as output");
            assert!(
                wall_ms - duration >= 200,
                "settlement retry must not inflate generation duration"
            );
            if answer {
                assert!(diagnostic["firstAnswerMs"].as_i64().unwrap() - first >= 40);
                assert_eq!(diagnostic["promptTokens"], 11);
                assert_eq!(diagnostic["completionTokens"], 19);
            } else {
                assert!(diagnostic["firstAnswerMs"].is_null());
                assert!(diagnostic["promptTokens"].is_null());
                assert!(diagnostic["completionTokens"].is_null());
                assert_eq!(diagnostic["measurementQuality"], "estimated_usage");
            }
            assert!(!diagnostic.to_string().contains("private reasoning fixture"));
            assert!(!diagnostic.to_string().contains("private answer fixture"));
            assert!(!diagnostic.to_string().contains("private tool fixture"));
            assert_eq!(core.settle_calls.load(Ordering::SeqCst), 2);
        }
    }

    #[tokio::test]
    async fn unpolled_stream_drop_records_one_cancellation_and_settles() {
        let mut core = FakeCoreState::new("fpk_live_unpolled", 10_000_000);
        core.settle_delay = Duration::from_millis(250);
        let started = Instant::now();
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream_url = spawn(fake_upstream_router(FakeUpstreamState::new())).await;
        let mut config = test_config(core_url, upstream_url);
        config.usage_api_timeout = Duration::from_secs(2);
        config.metrics_auth_token = Some("synthetic-metrics".into());
        let router = app(config).unwrap();
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::post("/v1/chat/completions")
                    .header("authorization", "Bearer fpk_live_unpolled")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"model":"glm-5-2","stream":true,
                "messages":[{"role":"user","content":"synthetic"}],"max_tokens":10})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        // Deliberately never poll response.into_body(). This is the exact
        // boundary where a client can disconnect before the first SSE poll.
        drop(response);
        wait_for(|| {
            core.settle_calls.load(Ordering::SeqCst) == 1
                && core.diagnostics.lock().unwrap().len() == 1
        })
        .await;
        let diagnostic = core.diagnostics.lock().unwrap()[0].clone();
        assert!(!core.diagnostic_before_settlement.load(Ordering::SeqCst));
        assert!(
            started.elapsed().as_millis() as i64 - diagnostic["durationMs"].as_i64().unwrap()
                >= 200
        );
        assert_eq!(
            diagnostic["terminationReason"],
            "client_disconnected_or_stream_cancelled"
        );
        assert!(diagnostic["firstOutputMs"].is_null());
        assert!(diagnostic["completionTokens"].is_null());
        let response = router
            .oneshot(
                axum::http::Request::get("/metrics")
                    .header("authorization", "Bearer synthetic-metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let metrics = String::from_utf8(
            axum::body::to_bytes(response.into_body(), 100_000)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(metrics.contains("finite_private_limiter_active_requests 0"));
        assert!(
            metrics.contains(
                "finite_private_limiter_requests_total{outcome=\"client_cancellation\"} 1"
            )
        );
    }

    #[tokio::test]
    async fn client_drop_during_settlement_retry_preserves_actual_accounting_and_diagnostic_order()
    {
        let mut core = FakeCoreState::new("fpk_live_settlement_drop", 10_000_000);
        core.settle_delay = Duration::from_millis(100);
        core.settle_failures_remaining.store(1, Ordering::SeqCst);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream_url = spawn(fake_upstream_router(FakeUpstreamState::new())).await;
        let mut config = test_config(core_url, upstream_url);
        config.usage_api_timeout = Duration::from_secs(2);
        let router = app(config).unwrap();
        let response = router
            .oneshot(
                axum::http::Request::post("/v1/chat/completions")
                    .header("authorization", "Bearer fpk_live_settlement_drop")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"model":"glm-5-2","stream":true,"messages":[]}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let consuming =
            tokio::spawn(async move { axum::body::to_bytes(response.into_body(), 100_000).await });
        wait_for(|| core.settle_calls.load(Ordering::SeqCst) == 1).await;
        consuming.abort();
        let _ = consuming.await;
        wait_for(|| core.diagnostics.lock().unwrap().len() == 1).await;
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 2);
        assert_eq!(core.settlements.lock().unwrap().len(), 1);
        assert_eq!(core.settlements.lock().unwrap()[0]["settlement"], "actual");
        assert!(!core.diagnostic_before_settlement.load(Ordering::SeqCst));
        let diagnostic = core.diagnostics.lock().unwrap()[0].clone();
        assert_eq!(diagnostic["completionTokens"], 10);
        assert_eq!(diagnostic["measurementQuality"], "observed_usage");
        assert_eq!(diagnostic["terminationReason"], "complete");
    }

    #[tokio::test]
    async fn prior_core_without_diagnostic_route_keeps_inference_and_accounting_working() {
        let mut core = FakeCoreState::new("fpk_live_prior_core", 10_000_000);
        core.diagnostic_supported = false;
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream_url = spawn(fake_upstream_router(FakeUpstreamState::new())).await;
        let mut config = test_config(core_url, upstream_url);
        config.metrics_auth_token = Some("synthetic-metrics".into());
        let limiter_url = spawn(app(config).unwrap()).await;
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_prior_core")
            .json(&json!({"model":"glm-5-2","messages":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.json::<Value>().await.unwrap()["usage"]["completion_tokens"],
            20
        );
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 1);
        for _ in 0..50 {
            let metrics = client
                .get(format!("{limiter_url}/metrics"))
                .bearer_auth("synthetic-metrics")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            if metrics.contains("finite_private_limiter_diagnostic_persistence_failures_total 1") {
                assert!(
                    metrics.contains("finite_private_limiter_settlements_total{kind=\"actual\"} 1")
                );
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("unsupported diagnostics should be visible as a monitoring failure");
    }

    #[tokio::test]
    async fn slow_diagnostics_do_not_delay_inference_or_settlement() {
        let mut core = FakeCoreState::new("fpk_live_slow_diagnostics", 100_000_000);
        core.diagnostic_delay = Duration::from_secs(5);
        let core_url = spawn(fake_core_router(core.clone())).await;
        let upstream_url = spawn(fake_upstream_router(FakeUpstreamState::new())).await;
        let mut config = test_config(core_url, upstream_url);
        // Keep each diagnostic permit occupied beyond this test's response
        // deadline so cumulative arrivals also prove the concurrency bound.
        config.usage_api_timeout = Duration::from_secs(10);
        let limiter_url = spawn(app(config).unwrap()).await;
        let client = reqwest::Client::new();
        let started = Instant::now();
        let requests = (0..40).map(|_| {
            let client = client.clone();
            let url = limiter_url.clone();
            async move {
                let response = client
                    .post(format!("{url}/v1/chat/completions"))
                    .bearer_auth("fpk_live_slow_diagnostics")
                    .json(&json!({"model":"glm-5-2","messages":[],"max_tokens":10}))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let result: Value = response.json().await.unwrap();
                assert_eq!(result["choices"][0]["message"]["content"], "ok");
            }
        });
        futures_util::future::join_all(requests).await;
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "monitoring blocked inference"
        );
        assert_eq!(core.settle_calls.load(Ordering::SeqCst), 40);
        assert!(core.diagnostics.lock().unwrap().len() <= 8);
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
        diagnostics: Arc<Mutex<Vec<Value>>>,
        diagnostic_delay: Duration,
        diagnostic_supported: bool,
        settle_delay: Duration,
        diagnostic_before_settlement: Arc<AtomicBool>,
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
                diagnostics: Arc::new(Mutex::new(Vec::new())),
                diagnostic_delay: Duration::ZERO,
                diagnostic_supported: true,
                settle_delay: Duration::ZERO,
                diagnostic_before_settlement: Arc::new(AtomicBool::new(false)),
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
            .route(
                "/internal/finite-private/v1/request-diagnostics",
                post(
                    |State(state): State<FakeCoreState>,
                     headers: HeaderMap,
                     Json(body): Json<Value>| async move {
                        if state.settlements.lock().unwrap().is_empty() {
                            state
                                .diagnostic_before_settlement
                                .store(true, Ordering::SeqCst);
                        }
                        state.diagnostics.lock().unwrap().push(body);
                        assert_service_auth(&headers);
                        sleep(state.diagnostic_delay).await;
                        if state.diagnostic_supported {
                            StatusCode::OK
                        } else {
                            StatusCode::NOT_FOUND
                        }
                    },
                ),
            )
            .with_state(state)
    }

    /// Public finite.computer outage origin: usage-API path 307s to an HTML 200.
    fn public_outage_origin_router() -> Router {
        Router::new()
            .route(
                "/internal/finite-private/v1/health",
                get(|| async { axum::response::Redirect::temporary("/") }),
            )
            .route(
                "/",
                get(|| async {
                    (
                        StatusCode::OK,
                        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                        "<!doctype html><title>Finite — Service outage</title>",
                    )
                }),
            )
    }

    /// Same HTML 200 parked directly on the usage-API health path.
    fn html_health_origin_router() -> Router {
        Router::new().route(
            "/internal/finite-private/v1/health",
            get(|| async {
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    "<!doctype html><title>Finite — Service outage</title>",
                )
            }),
        )
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
        sleep(state.settle_delay).await;
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
        bodies: Arc<Mutex<Vec<Bytes>>>,
        delay_first_byte_ms: Arc<AtomicUsize>,
        stream_tail_delay_ms: Arc<AtomicUsize>,
        stream_hang_after_done: Arc<AtomicBool>,
    }

    impl FakeUpstreamState {
        fn new() -> Self {
            Self {
                health_ok: Arc::new(AtomicBool::new(true)),
                calls: Arc::new(AtomicUsize::new(0)),
                bodies: Arc::new(Mutex::new(Vec::new())),
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
        state.bodies.lock().unwrap().push(body.clone());
        let request = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
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
