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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const USAGE_FORMULA_VERSION: &str = "2026-05-26.v1";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct LimiterConfig {
    pub finite_usage_api_url: String,
    pub finite_usage_api_service_key: String,
    pub upstream_base_url: String,
    pub vllm_internal_api_key: String,
    pub dashboard_url: String,
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
    #[error("failed to build HTTP client: {0}")]
    HttpClient(String),
}

pub fn app(config: LimiterConfig) -> Result<Router, LimiterConfigError> {
    validate_config(&config)?;
    let state = AppState {
        config: Arc::new(config),
        client: Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_REQUEST_TIMEOUT)
            .build()
            .map_err(|error| LimiterConfigError::HttpClient(error.to_string()))?,
    };
    Ok(Router::new()
        .route("/health", get(health))
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
        ("DASHBOARD_URL", &config.dashboard_url),
    ] {
        if value.trim().is_empty() {
            return Err(LimiterConfigError::Missing(name));
        }
    }
    Ok(())
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
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
    // Accounting idempotency belongs to this limiter attempt, not to an
    // untrusted client correlation header. Reusing a caller supplied id would
    // let a second upstream attempt reuse the first reservation in Core.
    let request_id = request_id();
    let estimate = estimate_usage(&body);
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

    let is_streaming = request_is_streaming(&body);
    let upstream_body = upstream_body_for_request(&uri, body.clone());
    if is_streaming {
        let upstream = match call_upstream_response(&state, &uri, upstream_body).await {
            Ok(response) => response,
            Err(error) => {
                eprintln!("finite-private-limiter upstream failed: {error}");
                let _ = settle_usage(
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
            let _ = settle_usage(
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

    let actual = actual_usage(&upstream.body);
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
    if let Err(error) = settle_usage(&state, &reservation_id, settle).await {
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
        .send()
        .await
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
    input: SettleRequest,
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
        .json(&input)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("Core settle returned {}", response.status()))
    }
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
    let body = response.bytes().await.map_err(|error| error.to_string())?;
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
    state
        .client
        .post(url)
        .bearer_auth(&state.config.vllm_internal_api_key)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
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
    let body_stream = async_stream::stream! {
        let mut accumulator = StreamingUsageAccumulator::default();
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => {
                    accumulator.push(&chunk);
                    yield Ok::<Bytes, std::io::Error>(chunk);
                }
                Err(error) => {
                    eprintln!("finite-private-limiter upstream stream failed: {error}");
                    let _ = settle_usage(
                        &state,
                        &reservation_id,
                        SettleRequest {
                            request_id: request_id.clone(),
                            settlement: "estimate".to_string(),
                            prompt_tokens: None,
                            completion_tokens: None,
                            usage_units: None,
                            usage_formula_version: USAGE_FORMULA_VERSION.to_string(),
                            upstream_status,
                            upstream_error_class: Some("upstream_stream_error".to_string()),
                        },
                    )
                    .await;
                    yield Err(std::io::Error::other(error.to_string()));
                    return;
                }
            }
        }
        let actual = accumulator.actual_usage();
        let settle = SettleRequest {
            request_id: request_id.clone(),
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
        if let Err(error) = settle_usage(&state, &reservation_id, settle).await {
            eprintln!("finite-private-limiter streaming settle failed: {error}");
        }
    };
    response.body(Body::from_stream(body_stream)).unwrap()
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

fn request_id() -> String {
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

fn estimate_usage(body: &[u8]) -> EstimatedUsage {
    let value = serde_json::from_slice::<Value>(body).unwrap_or(Value::Null);
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("kimi-k2-6")
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

fn actual_usage(body: &[u8]) -> Option<ActualUsage> {
    let value = serde_json::from_slice::<Value>(body).ok()?;
    actual_usage_from_value(&value)
}

fn actual_usage_from_value(value: &Value) -> Option<ActualUsage> {
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
        .unwrap_or("kimi-k2-6");
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
        "kimi-k2-6" => 1.0,
        _ => 1.0,
    };
    (base_units * context_multiplier * model_multiplier).ceil() as i64
}

#[derive(Default)]
struct StreamingUsageAccumulator {
    pending_line: String,
    actual_usage: Option<ActualUsage>,
}

impl StreamingUsageAccumulator {
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
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        if let Some(usage) = actual_usage_from_value(&value) {
            self.actual_usage = Some(usage);
        }
    }

    fn actual_usage(self) -> Option<ActualUsage> {
        self.actual_usage
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

#[derive(Debug, Serialize)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, State};
    use finite_saas_core::api::router as core_router;
    use finite_saas_core::auth::{CoreAuth, WorkosAuthenticator, WorkosAuthenticatorConfig};
    use finite_saas_core::store::CoreStore;
    use finite_saas_core::{ApproveFinitePrivateGrantInput, IssueFinitePrivateApiKeyInput};
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use tokio::net::TcpListener;

    fn test_core_auth() -> CoreAuth {
        let workos = WorkosAuthenticator::new(WorkosAuthenticatorConfig {
            client_id: "client_limiter_test".to_string(),
            issuer: "https://identity.limiter.invalid".to_string(),
            operator_org_id: "org_limiter_operator".to_string(),
            api_key: "limiter-test-workos-key".to_string(),
            api_base_url: "https://identity.limiter.invalid".to_string(),
            jwks_url: "https://identity.limiter.invalid/jwks".to_string(),
        })
        .unwrap();
        CoreAuth::new(
            workos,
            "core-service-token",
            "core-runner-token",
            "core-token",
        )
        .unwrap()
    }

    #[tokio::test]
    async fn limiter_reserves_proxies_settles_and_denies_before_upstream() {
        let core_store = CoreStore::memory();
        let grant = core_store
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "private@finite.vip".to_string(),
                workos_user_id: Some("user_workos_private".to_string()),
                limit_profile_id: None,
                now: Some("2026-05-26T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        core_store
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id,
                raw_key: "fpk_live_secret".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some("2026-05-26T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        let core_url = spawn(core_router(core_store, test_core_auth())).await;

        #[derive(Clone)]
        struct FakeUpstreamState {
            calls: Arc<AtomicUsize>,
        }
        async fn fake_upstream(
            State(state): State<FakeUpstreamState>,
            headers: HeaderMap,
        ) -> Response {
            assert_eq!(
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer vllm-secret")
            );
            state.calls.fetch_add(1, Ordering::SeqCst);
            (
                StatusCode::OK,
                Json(json!({
                    "id": "chatcmpl_test",
                    "model": "kimi-k2-6",
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
        let calls = Arc::new(AtomicUsize::new(0));
        let upstream_url = spawn(
            Router::new()
                .route("/v1/chat/completions", post(fake_upstream))
                .with_state(FakeUpstreamState {
                    calls: calls.clone(),
                }),
        )
        .await;

        let limiter_url = spawn(
            app(LimiterConfig {
                finite_usage_api_url: core_url,
                finite_usage_api_service_key: "core-token".to_string(),
                upstream_base_url: upstream_url,
                vllm_internal_api_key: "vllm-secret".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
            })
            .unwrap(),
        )
        .await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_secret")
            .header("x-request-id", "req-limiter-ok")
            .json(&json!({
                "model": "kimi-k2-6",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 64
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_secret")
            .header("x-request-id", "req-limiter-denied")
            .json(&json!({
                "model": "kimi-k2-6",
                "messages": [{ "role": "user", "content": "too much" }],
                "max_tokens": 2_000_000
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn limiter_streams_and_settles_sse_usage() {
        let core_store = CoreStore::memory();
        let grant = core_store
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "stream@finite.vip".to_string(),
                workos_user_id: Some("user_workos_stream".to_string()),
                limit_profile_id: None,
                now: Some("2026-05-26T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        core_store
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id,
                raw_key: "fpk_live_stream".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some("2026-05-26T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        let core_url = spawn(core_router(core_store, test_core_auth())).await;

        #[derive(Clone)]
        struct FakeStreamingUpstreamState {
            calls: Arc<AtomicUsize>,
        }
        async fn fake_streaming_upstream(
            State(state): State<FakeStreamingUpstreamState>,
            headers: HeaderMap,
            body: Bytes,
        ) -> Response {
            assert_eq!(
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer vllm-secret")
            );
            let request = serde_json::from_slice::<Value>(&body).unwrap();
            assert_eq!(request["stream"], true);
            assert_eq!(request["stream_options"]["include_usage"], true);
            state.calls.fetch_add(1, Ordering::SeqCst);
            let body = [
                "data: {\"id\":\"chatcmpl_stream\",\"model\":\"kimi-k2-6\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                "data: {\"id\":\"chatcmpl_stream\",\"model\":\"kimi-k2-6\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":10,\"total_tokens\":20}}\n\n",
                "data: [DONE]\n\n",
            ]
            .concat();
            (
                StatusCode::OK,
                [("content-type", "text/event-stream")],
                Body::from(body),
            )
                .into_response()
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let upstream_url = spawn(
            Router::new()
                .route("/v1/chat/completions", post(fake_streaming_upstream))
                .with_state(FakeStreamingUpstreamState {
                    calls: calls.clone(),
                }),
        )
        .await;

        let limiter_url = spawn(
            app(LimiterConfig {
                finite_usage_api_url: core_url,
                finite_usage_api_service_key: "core-token".to_string(),
                upstream_base_url: upstream_url,
                vllm_internal_api_key: "vllm-secret".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
            })
            .unwrap(),
        )
        .await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_stream")
            .header("x-request-id", "req-stream-first")
            .json(&json!({
                "model": "kimi-k2-6",
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
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let response = client
            .post(format!("{limiter_url}/v1/chat/completions"))
            .bearer_auth("fpk_live_stream")
            .header("x-request-id", "req-stream-second")
            .json(&json!({
                "model": "kimi-k2-6",
                "stream": true,
                "messages": [{ "role": "user", "content": "still under after actual settle" }],
                "max_tokens": 1_600_000
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = response.text().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn limiter_uses_a_fresh_internal_accounting_id_for_duplicate_client_ids() {
        #[derive(Clone, Default)]
        struct UsageApiState {
            reserve_request_ids: Arc<Mutex<Vec<String>>>,
            settle_request_ids: Arc<Mutex<Vec<String>>>,
        }
        async fn reserve(
            State(state): State<UsageApiState>,
            Json(input): Json<Value>,
        ) -> Json<Value> {
            let request_id = input["requestId"].as_str().unwrap().to_string();
            state
                .reserve_request_ids
                .lock()
                .unwrap()
                .push(request_id.clone());
            Json(json!({
                "decision": "allow",
                "reservation_id": format!("reservation-{request_id}"),
            }))
        }
        async fn settle(
            State(state): State<UsageApiState>,
            Path(_reservation_id): Path<String>,
            Json(input): Json<Value>,
        ) -> Json<Value> {
            state
                .settle_request_ids
                .lock()
                .unwrap()
                .push(input["requestId"].as_str().unwrap().to_string());
            Json(json!({ "settled": true }))
        }

        let usage_api_state = UsageApiState::default();
        let usage_api_url = spawn(
            Router::new()
                .route("/internal/finite-private/v1/reservations", post(reserve))
                .route(
                    "/internal/finite-private/v1/reservations/{reservation_id}/settle",
                    post(settle),
                )
                .with_state(usage_api_state.clone()),
        )
        .await;
        let upstream_url = spawn(Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                Json(json!({
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 10,
                        "total_tokens": 20,
                    }
                }))
            }),
        ))
        .await;
        let limiter_url = spawn(
            app(LimiterConfig {
                finite_usage_api_url: usage_api_url,
                finite_usage_api_service_key: "core-token".to_string(),
                upstream_base_url: upstream_url,
                vllm_internal_api_key: "vllm-secret".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
            })
            .unwrap(),
        )
        .await;

        let client = reqwest::Client::new();
        for _ in 0..2 {
            let response = client
                .post(format!("{limiter_url}/v1/chat/completions"))
                .bearer_auth("fpk_live_duplicate_client_id")
                .header("x-request-id", "caller-reused-id")
                .json(&json!({
                    "model": "kimi-k2-6",
                    "messages": [{ "role": "user", "content": "hello" }],
                    "max_tokens": 64,
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        let reserved = usage_api_state.reserve_request_ids.lock().unwrap().clone();
        let settled = usage_api_state.settle_request_ids.lock().unwrap().clone();
        assert_eq!(reserved.len(), 2);
        assert_eq!(settled, reserved);
        assert_ne!(reserved[0], reserved[1]);
        assert!(reserved.iter().all(|id| id != "caller-reused-id"));
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
