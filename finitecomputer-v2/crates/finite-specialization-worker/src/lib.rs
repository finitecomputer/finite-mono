use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_SPARK_BASE_URL: &str = "https://inference.finite.computer/v1";
pub const DEFAULT_VISION_MODEL: &str = "aeon-gemma-4-12b-k4-nvfp4-unified-fast";
pub const IMAGE_PROMPT_VERSION: &str = "aeon-image-analysis-v1";
const IMAGE_CAPABILITY_PROMPT: &str = "Interpret the supplied image faithfully. Follow the user's instruction, distinguish visible facts from uncertainty, and do not invoke tools.";
const DEFAULT_MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
const DEFAULT_MAX_INLINE_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_MAX_OUTPUT_CHARS: usize = 32 * 1024;
const DEFAULT_MAX_IMAGES: usize = 8;

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub bind_host: String,
    pub bind_port: u16,
    pub worker_bearer_token: Option<String>,
    pub spark_base_url: String,
    pub spark_bearer_token: Option<String>,
    pub vision_model: String,
    pub max_image_bytes: u64,
    pub max_inline_image_bytes: u64,
    pub max_output_chars: usize,
    pub image_download_timeout_seconds: u64,
    pub request_deadline_seconds: u64,
    pub allowed_attachment_hosts: Vec<String>,
    pub image_canary_interval_seconds: Option<u64>,
}

impl WorkerConfig {
    pub fn from_env(bind_host: String, bind_port: u16) -> Self {
        Self {
            bind_host,
            bind_port,
            worker_bearer_token: non_empty_env("FINITE_SPECIALIZATION_WORKER_BEARER_TOKEN"),
            spark_base_url: non_empty_env("SPARK_BASE_URL")
                .unwrap_or_else(|| DEFAULT_SPARK_BASE_URL.to_string()),
            spark_bearer_token: non_empty_env("SPARK_GATEWAY_BEARER_TOKEN"),
            vision_model: non_empty_env("FINITE_SPECIALIZATION_VISION_MODEL")
                .unwrap_or_else(|| DEFAULT_VISION_MODEL.to_string()),
            max_image_bytes: non_empty_env("FINITE_SPECIALIZATION_MAX_IMAGE_BYTES")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(DEFAULT_MAX_IMAGE_BYTES)
                .clamp(1024, DEFAULT_MAX_IMAGE_BYTES),
            max_inline_image_bytes: non_empty_env("FINITE_SPECIALIZATION_MAX_INLINE_IMAGE_BYTES")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(DEFAULT_MAX_INLINE_IMAGE_BYTES)
                .clamp(1024, DEFAULT_MAX_INLINE_IMAGE_BYTES),
            max_output_chars: non_empty_env("FINITE_SPECIALIZATION_MAX_OUTPUT_CHARS")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(DEFAULT_MAX_OUTPUT_CHARS)
                .clamp(256, DEFAULT_MAX_OUTPUT_CHARS),
            image_download_timeout_seconds: non_empty_env(
                "FINITE_SPECIALIZATION_IMAGE_DOWNLOAD_TIMEOUT_SECONDS",
            )
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(30)
            .clamp(1, 120),
            request_deadline_seconds: non_empty_env(
                "FINITE_SPECIALIZATION_REQUEST_DEADLINE_SECONDS",
            )
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(120)
            .clamp(1, 300),
            allowed_attachment_hosts: non_empty_env("FINITE_SPECIALIZATION_ATTACHMENT_HOSTS")
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            image_canary_interval_seconds: Some(
                non_empty_env("FINITE_SPECIALIZATION_IMAGE_CANARY_INTERVAL_SECONDS")
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(300)
                    .clamp(60, 3600),
            ),
        }
    }
}

#[derive(Debug)]
struct WorkerState {
    config: WorkerConfig,
    client: Client,
    metrics: CapabilityMetrics,
}

#[derive(Debug)]
struct ImageCanaryObservation {
    health_state: &'static str,
    consecutive_failures: u64,
    last_completed_timestamp_seconds: u64,
    last_latency_milliseconds: u64,
}

impl Default for ImageCanaryObservation {
    fn default() -> Self {
        Self {
            health_state: "stale",
            consecutive_failures: 0,
            last_completed_timestamp_seconds: 0,
            last_latency_milliseconds: 0,
        }
    }
}

#[derive(Debug, Default)]
struct CapabilityMetrics {
    image_canary: Mutex<ImageCanaryObservation>,
    image_requests: AtomicU64,
    image_errors_by_code: Mutex<BTreeMap<&'static str, u64>>,
}

impl CapabilityMetrics {
    fn observe_request(&self, error_code: Option<&'static str>) {
        self.image_requests.fetch_add(1, Ordering::Relaxed);
        if let Some(error_code) = error_code {
            let mut errors = self.image_errors_by_code.lock().unwrap();
            *errors.entry(error_code).or_default() += 1;
        }
    }

    fn observe_canary(&self, succeeded: bool, latency_milliseconds: u64) {
        let mut observation = self.image_canary.lock().unwrap();
        observation.last_completed_timestamp_seconds = current_unix_timestamp();
        observation.last_latency_milliseconds = latency_milliseconds;
        if succeeded {
            observation.consecutive_failures = 0;
            observation.health_state = "healthy";
        } else {
            observation.consecutive_failures = observation.consecutive_failures.saturating_add(1);
            observation.health_state = if observation.consecutive_failures >= 3 {
                "unavailable"
            } else {
                "degraded"
            };
        }
    }

    fn render(&self, model_alias: &str) -> String {
        let observation = self.image_canary.lock().unwrap();
        let health_state = effective_canary_health(&observation);
        let model_alias = prometheus_label(model_alias);
        let mut rendered = format!(
            concat!(
                "# HELP finite_specialization_capability_health Semantic canary health for one specialization capability.\n",
                "# TYPE finite_specialization_capability_health gauge\n",
                "finite_specialization_capability_health{{capability=\"image\",health_state=\"{}\",model_alias=\"{}\",prompt_version=\"{}\",surface=\"tyk-public-frontdoor\"}} 1\n",
                "# HELP finite_specialization_canary_last_completed_timestamp_seconds Unix timestamp of the last completed semantic canary.\n",
                "# TYPE finite_specialization_canary_last_completed_timestamp_seconds gauge\n",
                "finite_specialization_canary_last_completed_timestamp_seconds{{capability=\"image\",model_alias=\"{}\",surface=\"tyk-public-frontdoor\"}} {}\n",
                "# HELP finite_specialization_canary_latency_milliseconds Last semantic canary latency.\n",
                "# TYPE finite_specialization_canary_latency_milliseconds gauge\n",
                "finite_specialization_canary_latency_milliseconds{{capability=\"image\",model_alias=\"{}\",surface=\"tyk-public-frontdoor\"}} {}\n",
                "# HELP finite_specialization_requests_total Specialization capability requests.\n",
                "# TYPE finite_specialization_requests_total counter\n",
                "finite_specialization_requests_total{{capability=\"image\",model_alias=\"{}\",surface=\"tyk-public-frontdoor\"}} {}\n",
                "# HELP finite_specialization_errors_total Specialization capability errors.\n",
                "# TYPE finite_specialization_errors_total counter\n"
            ),
            health_state,
            model_alias,
            IMAGE_PROMPT_VERSION,
            model_alias,
            observation.last_completed_timestamp_seconds,
            model_alias,
            observation.last_latency_milliseconds,
            model_alias,
            self.image_requests.load(Ordering::Relaxed),
        );
        let errors = self.image_errors_by_code.lock().unwrap();
        if errors.is_empty() {
            rendered.push_str(&format!(
                "finite_specialization_errors_total{{capability=\"image\",error_code=\"none\",model_alias=\"{model_alias}\",surface=\"tyk-public-frontdoor\"}} 0\n"
            ));
        } else {
            for (code, count) in errors.iter() {
                rendered.push_str(&format!(
                    "finite_specialization_errors_total{{capability=\"image\",error_code=\"{}\",model_alias=\"{model_alias}\",surface=\"tyk-public-frontdoor\"}} {count}\n",
                    prometheus_label(code)
                ));
            }
        }
        rendered
    }
}

fn prometheus_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ChatCompletionRequest {
    pub model: Option<String>,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub temperature: Option<Value>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_completion_tokens: Option<u64>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub finite_specialization: Option<InvocationSpecializationState>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InvocationSpecializationState {
    pub capabilities: InvocationCapabilities,
    pub prompt_versions: InvocationPromptVersions,
    pub normalization_limits: InvocationNormalizationLimits,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InvocationCapabilities {
    pub image: bool,
    pub audio: bool,
    pub video: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InvocationPromptVersions {
    pub image: String,
    pub audio: String,
    pub video: String,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
pub struct InvocationNormalizationLimits {
    pub max_images: usize,
    pub max_inline_bytes: u64,
    pub max_download_bytes: u64,
    pub max_output_chars: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: ChatContent,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    Parts(Vec<Value>),
}

pub fn app(config: WorkerConfig) -> Router {
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("worker reqwest client should build");
    app_with_client(config, client)
}

pub fn app_with_client(config: WorkerConfig, client: Client) -> Router {
    let canary_interval = config.image_canary_interval_seconds;
    let state = Arc::new(WorkerState {
        config,
        client,
        metrics: CapabilityMetrics::default(),
    });
    if let Some(interval_seconds) = canary_interval {
        tokio::spawn(run_image_canary_loop(Arc::clone(&state), interval_seconds));
    }
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/chat/completions", post(chat_completions))
        .layer(DefaultBodyLimit::max(48 * 1024 * 1024))
        .with_state(state)
}

async fn health(State(state): State<Arc<WorkerState>>) -> Json<Value> {
    let observation = state.metrics.image_canary.lock().unwrap();
    let health_state = effective_canary_health(&observation);
    Json(json!({
        "ok": true,
        "service": "finite-specialization-worker",
        "capabilities": {
            "image": {
                "health": health_state,
                "prompt_version": IMAGE_PROMPT_VERSION,
                "last_completed_timestamp_seconds": observation.last_completed_timestamp_seconds,
            }
        }
    }))
}

fn effective_canary_health(observation: &ImageCanaryObservation) -> &'static str {
    if observation.last_completed_timestamp_seconds == 0
        || current_unix_timestamp()
            > observation
                .last_completed_timestamp_seconds
                .saturating_add(15 * 60)
    {
        "stale"
    } else {
        observation.health_state
    }
}

async fn metrics(State(state): State<Arc<WorkerState>>) -> impl IntoResponse {
    (
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        state.metrics.render(&state.config.vision_model),
    )
}

async fn chat_completions(
    State(state): State<Arc<WorkerState>>,
    headers: HeaderMap,
    Json(input): Json<ChatCompletionRequest>,
) -> Result<Json<Value>, WorkerError> {
    authenticate_worker_request(&state.config, &headers)?;
    let result = tokio::time::timeout(
        Duration::from_secs(state.config.request_deadline_seconds),
        analyze_image(&state, &input),
    )
    .await
    .unwrap_or_else(|_| {
        Err(WorkerError::request_timeout(
            "image analysis deadline expired",
        ))
    });
    state
        .metrics
        .observe_request(result.as_ref().err().map(|error| error.code));
    result.map(Json)
}

async fn analyze_image(
    state: &WorkerState,
    input: &ChatCompletionRequest,
) -> Result<Value, WorkerError> {
    let started = Instant::now();
    let limits = effective_image_limits(&state.config, input)?;
    let spark_token = state
        .config
        .spark_bearer_token
        .as_deref()
        .ok_or_else(|| WorkerError::unavailable("Spark gateway token is not configured"))?;
    let normalized_input = normalize_image_inputs(state, input, limits).await?;
    let upstream_request =
        responses_request_from_chat(&normalized_input, &state.config.vision_model)?;
    let response = state
        .client
        .post(format!(
            "{}/responses",
            state.config.spark_base_url.trim_end_matches('/')
        ))
        .bearer_auth(spark_token)
        .json(&upstream_request)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                WorkerError::request_timeout("Spark image analysis timed out")
            } else {
                WorkerError::bad_gateway("Spark image analysis request failed")
            }
        })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|_| WorkerError::bad_gateway("Spark vision response was not readable"))?;
    if !status.is_success() {
        return Err(WorkerError::bad_gateway(format!(
            "Spark vision request failed with status {}",
            status.as_u16()
        )));
    }
    let upstream: Value = serde_json::from_str(&body)
        .map_err(|_| WorkerError::bad_gateway("Spark vision response was not valid JSON"))?;
    let mut output = chat_completion_response_from_responses(&upstream, &state.config.vision_model);
    let text = truncate_chars(
        extract_response_text(&upstream).unwrap_or_default(),
        limits.max_output_chars,
    );
    output["choices"][0]["message"]["content"] = json!(text);
    let request_id = upstream
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("resp_finite_specialization");
    output["specialization_result"] = json!({
        "capability": "image",
        "text": text,
        "model": state.config.vision_model,
        "request_id": request_id,
        "duration_ms": started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    });
    Ok(output)
}

fn effective_image_limits(
    config: &WorkerConfig,
    input: &ChatCompletionRequest,
) -> Result<InvocationNormalizationLimits, WorkerError> {
    let requested = input
        .finite_specialization
        .as_ref()
        .map(|state| {
            if !state.capabilities.image {
                return Err(WorkerError::unavailable("image capability is disabled"));
            }
            if state.capabilities.audio || state.capabilities.video {
                return Err(WorkerError::bad_request_with_code(
                    "audio and video capability activation is not supported by this worker",
                    "unsupported_media_format",
                ));
            }
            if state.prompt_versions.image != IMAGE_PROMPT_VERSION {
                return Err(WorkerError::bad_request_with_code(
                    "image prompt version is not supported",
                    "unsupported_media_format",
                ));
            }
            Ok(state.normalization_limits)
        })
        .transpose()?
        .unwrap_or(InvocationNormalizationLimits {
            max_images: DEFAULT_MAX_IMAGES,
            max_inline_bytes: config.max_inline_image_bytes,
            max_download_bytes: config.max_image_bytes,
            max_output_chars: config.max_output_chars,
        });
    Ok(InvocationNormalizationLimits {
        max_images: requested.max_images.clamp(1, DEFAULT_MAX_IMAGES),
        max_inline_bytes: requested
            .max_inline_bytes
            .clamp(1024, config.max_inline_image_bytes),
        max_download_bytes: requested
            .max_download_bytes
            .clamp(1024, config.max_image_bytes),
        max_output_chars: requested
            .max_output_chars
            .clamp(256, config.max_output_chars),
    })
}

fn truncate_chars(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value
    } else {
        value.chars().take(max_chars).collect()
    }
}

async fn run_image_canary_loop(state: Arc<WorkerState>, interval_seconds: u64) {
    let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
    loop {
        interval.tick().await;
        let started = Instant::now();
        let request = ChatCompletionRequest {
            model: None,
            messages: vec![ChatMessage {
                role: "user".to_owned(),
                content: ChatContent::Parts(vec![
                    json!({
                        "type": "text",
                        "text": "What is the dominant color of this image? Reply with one uppercase color word."
                    }),
                    json!({
                        "type": "image_url",
                        "image_url": { "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=" }
                    }),
                ]),
            }],
            temperature: Some(json!(0)),
            max_tokens: Some(32),
            max_completion_tokens: None,
            stream: Some(false),
            finite_specialization: None,
        };
        let passed = analyze_image(&state, &request)
            .await
            .ok()
            .and_then(|value| {
                value["specialization_result"]["text"]
                    .as_str()
                    .map(|text| text.trim().eq_ignore_ascii_case("red"))
            })
            .unwrap_or(false);
        state.metrics.observe_canary(
            passed,
            started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        );
    }
}

pub fn responses_request_from_chat(
    input: &ChatCompletionRequest,
    vision_model: &str,
) -> Result<Value, WorkerError> {
    if input.stream.unwrap_or(false) {
        return Err(WorkerError::bad_request(
            "streaming vision requests are not supported",
        ));
    }
    if input.messages.is_empty() {
        return Err(WorkerError::bad_request("messages are required"));
    }

    let mut messages = vec![json!({
        "role": "developer",
        "content": [{
            "type": "input_text",
            "text": IMAGE_CAPABILITY_PROMPT,
        }],
    })];
    for message in &input.messages {
        let role = normalize_role(&message.role)?;
        let content = responses_content_from_chat_content(&message.content)?;
        messages.push(json!({
            "role": role,
            "content": content,
        }));
    }

    let mut request = json!({
        "model": vision_model,
        "input": messages,
        "metadata": {
            "finite_capability": "image",
            "finite_prompt_version": IMAGE_PROMPT_VERSION,
        },
    });
    if let Some(temperature) = input.temperature.as_ref() {
        request["temperature"] = temperature.clone();
    }
    if let Some(max_tokens) = input.max_tokens.or(input.max_completion_tokens) {
        request["max_output_tokens"] = json!(max_tokens);
    }
    Ok(request)
}

async fn normalize_image_inputs(
    state: &WorkerState,
    input: &ChatCompletionRequest,
    limits: InvocationNormalizationLimits,
) -> Result<ChatCompletionRequest, WorkerError> {
    let mut normalized = input.clone();
    let mut image_count = 0usize;
    for message in &mut normalized.messages {
        let ChatContent::Parts(parts) = &mut message.content else {
            continue;
        };
        for part in parts {
            let part_type = part.get("type").and_then(Value::as_str).unwrap_or_default();
            if part_type != "image_url" && part_type != "input_image" {
                continue;
            }
            image_count += 1;
            if image_count > limits.max_images {
                return Err(WorkerError::payload_too_large(
                    "image count exceeds the specialization limit",
                ));
            }
            let source = image_source(part).ok_or_else(|| {
                WorkerError::bad_request_with_code(
                    "image_url content is missing url",
                    "media_fetch_failed",
                )
            })?;
            let data_uri = retrieve_and_validate_image(state, &source, limits).await?;
            *part = json!({
                "type": "image_url",
                "image_url": { "url": data_uri },
            });
        }
    }
    if image_count == 0 {
        return Err(WorkerError::bad_request_with_code(
            "image analysis requires at least one image",
            "unsupported_media_format",
        ));
    }
    Ok(normalized)
}

fn image_source(part: &Value) -> Option<String> {
    match part.get("type").and_then(Value::as_str) {
        Some("image_url") => part.get("image_url").and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("url").and_then(Value::as_str))
                .map(str::to_owned)
        }),
        Some("input_image") => part
            .get("image_url")
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

async fn retrieve_and_validate_image(
    state: &WorkerState,
    source: &str,
    limits: InvocationNormalizationLimits,
) -> Result<String, WorkerError> {
    let (declared_mime, bytes) = if source.starts_with("data:") {
        decode_image_data_uri(source, limits.max_inline_bytes)?
    } else {
        fetch_image(state, source, limits.max_download_bytes).await?
    };
    let detected_mime = detect_image_mime(&bytes).ok_or_else(|| {
        WorkerError::bad_request_with_code(
            "image bytes are not a supported image format",
            "media_decode_failed",
        )
    })?;
    let reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| media_decode_error())?;
    let (width, height) = reader.into_dimensions().map_err(|_| media_decode_error())?;
    if u64::from(width).saturating_mul(u64::from(height)) > 40_000_000 {
        return Err(WorkerError::payload_too_large(
            "decoded image dimensions exceed the specialization limit",
        ));
    }
    image::load_from_memory(&bytes).map_err(|_| media_decode_error())?;
    if let Some(declared) = declared_mime
        && declared != detected_mime
        && !(declared == "image/jpg" && detected_mime == "image/jpeg")
    {
        return Err(WorkerError::bad_request_with_code(
            "image content type does not match its bytes",
            "media_decode_failed",
        ));
    }
    Ok(format!(
        "data:{detected_mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn decode_image_data_uri(
    source: &str,
    max_bytes: u64,
) -> Result<(Option<&str>, Vec<u8>), WorkerError> {
    let (metadata, encoded) = source.split_once(',').ok_or_else(|| {
        WorkerError::bad_request_with_code("inline image is malformed", "media_decode_failed")
    })?;
    let mime = metadata
        .strip_prefix("data:")
        .and_then(|value| value.strip_suffix(";base64"))
        .filter(|value| value.starts_with("image/"));
    if mime.is_none() {
        return Err(WorkerError::bad_request_with_code(
            "inline image must use an image base64 data URI",
            "unsupported_media_format",
        ));
    }
    if encoded.len() as u64 > max_bytes.saturating_mul(2) {
        return Err(WorkerError::payload_too_large(
            "inline image exceeds the specialization limit",
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| {
            WorkerError::bad_request_with_code(
                "inline image base64 is invalid",
                "media_decode_failed",
            )
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(WorkerError::payload_too_large(
            "inline image exceeds the specialization limit",
        ));
    }
    Ok((mime, bytes))
}

fn media_decode_error() -> WorkerError {
    WorkerError::bad_request_with_code("image bytes could not be decoded", "media_decode_failed")
}

async fn fetch_image(
    state: &WorkerState,
    source: &str,
    max_download_bytes: u64,
) -> Result<(Option<&'static str>, Vec<u8>), WorkerError> {
    let mut url = reqwest::Url::parse(source).map_err(|_| {
        WorkerError::bad_request_with_code("image URL is invalid", "media_fetch_failed")
    })?;
    for redirect_count in 0..=3 {
        let client = media_client_for_url(state, &url).await?;
        let response = tokio::time::timeout(
            Duration::from_secs(state.config.image_download_timeout_seconds),
            client.get(url.clone()).send(),
        )
        .await
        .map_err(|_| WorkerError::request_timeout("image retrieval timed out"))?
        .map_err(|_| {
            WorkerError::bad_request_with_code("image retrieval failed", "media_fetch_failed")
        })?;
        if response.status().is_redirection() {
            if redirect_count == 3 {
                return Err(WorkerError::bad_request_with_code(
                    "image retrieval exceeded the redirect limit",
                    "media_fetch_failed",
                ));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    WorkerError::bad_request_with_code(
                        "image redirect is invalid",
                        "media_fetch_failed",
                    )
                })?;
            url = url.join(location).map_err(|_| {
                WorkerError::bad_request_with_code(
                    "image redirect is invalid",
                    "media_fetch_failed",
                )
            })?;
            continue;
        }
        if !response.status().is_success() {
            return Err(WorkerError::bad_request_with_code(
                "image retrieval failed",
                "media_fetch_failed",
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_download_bytes)
        {
            return Err(WorkerError::payload_too_large(
                "downloaded image exceeds the specialization limit",
            ));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                WorkerError::bad_request_with_code("image retrieval failed", "media_fetch_failed")
            })?;
            if bytes.len().saturating_add(chunk.len()) as u64 > max_download_bytes {
                return Err(WorkerError::payload_too_large(
                    "downloaded image exceeds the specialization limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok((None, bytes));
    }
    unreachable!("redirect loop always returns")
}

async fn media_client_for_url(
    state: &WorkerState,
    url: &reqwest::Url,
) -> Result<Client, WorkerError> {
    let host = url.host_str().ok_or_else(|| {
        WorkerError::bad_request_with_code("image URL has no host", "media_fetch_failed")
    })?;
    let explicitly_allowed = state
        .config
        .allowed_attachment_hosts
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(host));
    if explicitly_allowed {
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(WorkerError::bad_request_with_code(
                "image URL uses an unsupported scheme",
                "media_fetch_failed",
            ));
        }
        return Ok(state.client.clone());
    }
    if url.scheme() != "https" {
        return Err(WorkerError::bad_request_with_code(
            "image URL must use HTTPS or an allowlisted Finite attachment host",
            "media_fetch_failed",
        ));
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Err(private_media_address_error());
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses: Vec<SocketAddr> = tokio::time::timeout(
        Duration::from_secs(state.config.image_download_timeout_seconds.min(10)),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| WorkerError::request_timeout("image host resolution timed out"))?
    .map_err(|_| {
        WorkerError::bad_request_with_code("image host resolution failed", "media_fetch_failed")
    })?
    .collect();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(private_media_address_error());
    }
    Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| WorkerError::unavailable("image retrieval client could not be created"))
}

fn private_media_address_error() -> WorkerError {
    WorkerError::bad_request_with_code(
        "image URL resolves to a non-public address",
        "media_fetch_failed",
    )
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            !(address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_broadcast()
                || address.is_documentation()
                || address.is_unspecified()
                || octets[0] == 0
                || octets[0] >= 224
                || (octets[0] == 100 && (64..=127).contains(&octets[1])))
        }
        IpAddr::V6(address) => {
            !(address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_multicast())
        }
    }
}

fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn responses_content_from_chat_content(content: &ChatContent) -> Result<Vec<Value>, WorkerError> {
    match content {
        ChatContent::Text(text) => Ok(vec![json!({
            "type": "input_text",
            "text": text,
        })]),
        ChatContent::Parts(parts) => parts.iter().map(responses_part_from_chat_part).collect(),
    }
}

fn responses_part_from_chat_part(part: &Value) -> Result<Value, WorkerError> {
    let part_type = part
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    match part_type {
        "text" | "input_text" => {
            let text = part
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| WorkerError::bad_request("text content is missing text"))?;
            Ok(json!({
                "type": "input_text",
                "text": text,
            }))
        }
        "image_url" => {
            let image_url = part
                .get("image_url")
                .and_then(|value| {
                    value.as_str().map(ToOwned::to_owned).or_else(|| {
                        value
                            .get("url")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
                })
                .ok_or_else(|| WorkerError::bad_request("image_url content is missing url"))?;
            Ok(json!({
                "type": "input_image",
                "image_url": image_url,
            }))
        }
        "input_image" => {
            let image_url = part
                .get("image_url")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    WorkerError::bad_request("input_image content is missing image_url")
                })?;
            Ok(json!({
                "type": "input_image",
                "image_url": image_url,
            }))
        }
        _ => Err(WorkerError::bad_request(format!(
            "unsupported content part type '{}'",
            if part_type.is_empty() {
                "unknown"
            } else {
                part_type
            }
        ))),
    }
}

pub fn chat_completion_response_from_responses(upstream: &Value, model: &str) -> Value {
    let text = extract_response_text(upstream).unwrap_or_default();
    json!({
        "id": upstream
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("chatcmpl-finite-specialization"),
        "object": "chat.completion",
        "created": current_unix_timestamp(),
        "model": model,
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": text,
                },
                "finish_reason": "stop",
            }
        ],
    })
}

fn extract_response_text(value: &Value) -> Option<String> {
    if let Some(text) = value.get("output_text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    let mut out = Vec::new();
    for item in value.get("output")?.as_array()? {
        for content in item
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(text) = content.get("text").and_then(Value::as_str) {
                out.push(text.to_string());
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("\n"))
    }
}

fn authenticate_worker_request(
    config: &WorkerConfig,
    headers: &HeaderMap,
) -> Result<(), WorkerError> {
    let expected = config
        .worker_bearer_token
        .as_deref()
        .ok_or_else(|| WorkerError::unavailable("worker bearer token is not configured"))?;
    let actual = bearer_token(headers).ok_or_else(WorkerError::unauthorized)?;
    if actual == expected {
        Ok(())
    } else {
        Err(WorkerError::unauthorized())
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let header = headers.get("authorization")?.to_str().ok()?;
    header
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_role(role: &str) -> Result<&str, WorkerError> {
    let role = role.trim();
    match role {
        "system" | "developer" | "user" | "assistant" => Ok(role),
        _ => Err(WorkerError::bad_request("unsupported message role")),
    }
}

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug)]
pub struct WorkerError {
    status: StatusCode,
    message: String,
    error_type: &'static str,
    code: &'static str,
    param: Option<&'static str>,
}

impl WorkerError {
    fn new(
        status: StatusCode,
        message: impl Into<String>,
        error_type: &'static str,
        code: &'static str,
    ) -> Self {
        Self {
            status,
            message: bound_error(message.into()),
            error_type,
            code,
            param: None,
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::bad_request_with_code(message, "unsupported_media_format")
    }

    fn bad_request_with_code(message: impl Into<String>, code: &'static str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            message,
            "invalid_request_error",
            code,
        )
    }

    fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "authentication_error",
            "unauthorized",
        )
    }

    fn unavailable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            message,
            "server_error",
            "capability_unavailable",
        )
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_GATEWAY,
            message,
            "server_error",
            "upstream_error",
        )
    }

    fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            message,
            "invalid_request_error",
            "media_size_exceeded",
        )
    }

    fn request_timeout(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::GATEWAY_TIMEOUT,
            message,
            "server_error",
            "request_timeout",
        )
    }
}

impl IntoResponse for WorkerError {
    fn into_response(self) -> Response {
        let request_id = format!(
            "req_specialization_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        );
        (
            self.status,
            Json(json!({
                "error": {
                    "message": self.message,
                    "type": self.error_type,
                    "param": self.param,
                    "code": self.code,
                },
                "request_id": request_id,
            })),
        )
            .into_response()
    }
}

fn bound_error(mut message: String) -> String {
    message = message.replace(['\r', '\n'], " ");
    if message.len() <= 512 {
        return message;
    }
    let mut end = 512;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &message[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use axum::extract::Request;
    use axum::http::HeaderMap;
    use axum::routing::post;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    #[test]
    fn converts_chat_image_parts_to_responses_input() {
        let request = ChatCompletionRequest {
            model: Some("ignored-hermes-model".to_string()),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Parts(vec![
                    json!({ "type": "text", "text": "Describe this" }),
                    json!({
                        "type": "image_url",
                        "image_url": { "url": "data:image/png;base64,abc123" }
                    }),
                ]),
            }],
            temperature: Some(json!(0.2)),
            max_tokens: Some(100),
            max_completion_tokens: None,
            stream: None,
            finite_specialization: None,
        };

        let converted = responses_request_from_chat(&request, "qwopus-test").unwrap();

        assert_eq!(converted["model"], "qwopus-test");
        assert_eq!(converted["temperature"], json!(0.2));
        assert_eq!(converted["max_output_tokens"], json!(100));
        assert_eq!(
            converted["input"][1]["content"],
            json!([
                { "type": "input_text", "text": "Describe this" },
                { "type": "input_image", "image_url": "data:image/png;base64,abc123" }
            ])
        );
    }

    #[test]
    fn converts_responses_text_to_chat_completion_shape() {
        let upstream = json!({
            "id": "resp_123",
            "output": [
                {
                    "content": [
                        { "type": "output_text", "text": "A dark UI screenshot." }
                    ]
                }
            ]
        });

        let converted = chat_completion_response_from_responses(&upstream, "vision-model");

        assert_eq!(converted["id"], "resp_123");
        assert_eq!(converted["object"], "chat.completion");
        assert_eq!(
            converted["choices"][0]["message"]["content"],
            "A dark UI screenshot."
        );
    }

    #[tokio::test]
    async fn rejects_invalid_token_before_spark_call() {
        let config = test_config("http://127.0.0.1:9");
        let app = app(config);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer wrong")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        json!({
                            "messages": [{ "role": "user", "content": "describe" }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authenticated_chat_completion_calls_spark_responses() {
        let captured = Arc::new(Mutex::new(None::<(HeaderMap, Value)>));
        let upstream_url = spawn_spark_mock(
            captured.clone(),
            StatusCode::OK,
            json!({
                "id": "resp_ok",
                "output_text": "The image shows a dashboard."
            }),
        )
        .await;
        let worker = spawn_worker(test_config(&upstream_url)).await;
        let response: Value = reqwest::Client::new()
            .post(format!("{worker}/v1/chat/completions"))
            .bearer_auth("worker-secret")
            .json(&json!({
                "model": "hermes-vision",
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            { "type": "text", "text": "Describe it" },
                            {
                                "type": "image_url",
                                "image_url": { "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=" }
                            }
                        ]
                    }
                ]
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(
            response["choices"][0]["message"]["content"],
            "The image shows a dashboard."
        );
        assert_eq!(response["specialization_result"]["capability"], "image");
        assert_eq!(
            response["specialization_result"]["text"],
            "The image shows a dashboard."
        );
        assert_eq!(response["specialization_result"]["model"], "qwopus-test");
        assert_eq!(response["specialization_result"]["request_id"], "resp_ok");
        assert!(response["specialization_result"]["duration_ms"].is_u64());
        let captured = captured.lock().unwrap().clone().unwrap();
        assert_eq!(
            captured.0.get("authorization").unwrap().to_str().unwrap(),
            "Bearer spark-secret"
        );
        assert_eq!(captured.1["model"], "qwopus-test");
        assert_eq!(captured.1["input"][0]["role"], "developer");
        assert_eq!(
            captured.1["metadata"]["finite_prompt_version"],
            "aeon-image-analysis-v1"
        );
        assert_eq!(captured.1["input"][1]["content"][1]["type"], "input_image");
    }

    #[test]
    fn desired_state_limits_are_enforced_by_the_worker() {
        let config = test_config("http://127.0.0.1:9");
        let mut request = empty_request();
        request.finite_specialization = Some(test_desired_state());

        let limits = effective_image_limits(&config, &request).unwrap();

        assert_eq!(limits.max_images, 2);
        assert_eq!(limits.max_inline_bytes, 2048);
        assert_eq!(limits.max_download_bytes, 4096);
        assert_eq!(limits.max_output_chars, 512);
        assert_eq!(truncate_chars("abcdef".to_owned(), 3), "abc");
    }

    #[test]
    fn disabled_image_capability_fails_closed() {
        let config = test_config("http://127.0.0.1:9");
        let mut request = empty_request();
        let mut desired = test_desired_state();
        desired.capabilities.image = false;
        request.finite_specialization = Some(desired);

        let error = effective_image_limits(&config, &request).unwrap_err();

        assert_eq!(error.code, "capability_unavailable");
    }

    #[tokio::test]
    async fn request_deadline_covers_the_complete_image_operation() {
        let app = Router::new().route(
            "/responses",
            post(|| async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                Json(json!({ "id": "late", "output_text": "late" }))
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut config = test_config(&format!("http://{addr}"));
        config.request_deadline_seconds = 1;
        let worker = spawn_worker(config).await;

        let response = reqwest::Client::new()
            .post(format!("{worker}/v1/chat/completions"))
            .bearer_auth("worker-secret")
            .json(&json!({
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "image_url",
                        "image_url": { "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=" }
                    }]
                }]
            }))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();

        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(body["error"]["code"], "request_timeout");
    }

    #[tokio::test]
    async fn malformed_inline_image_returns_typed_openai_error() {
        let worker = spawn_worker(test_config("http://127.0.0.1:9")).await;
        let response = reqwest::Client::new()
            .post(format!("{worker}/v1/chat/completions"))
            .bearer_auth("worker-secret")
            .json(&json!({
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "image_url",
                        "image_url": { "url": "data:image/png;base64,bm90LWEtcG5n" }
                    }]
                }]
            }))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert_eq!(body["error"]["code"], "media_decode_failed");
        assert!(body["request_id"].as_str().is_some());
    }

    #[tokio::test]
    async fn truncated_image_with_valid_magic_is_rejected() {
        let state = test_state(test_config("http://127.0.0.1:9"));
        let truncated = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\n");

        let error = retrieve_and_validate_image(
            &state,
            &format!("data:image/png;base64,{truncated}"),
            test_limits(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, "media_decode_failed");
    }

    #[tokio::test]
    async fn public_media_url_resolving_to_private_address_is_rejected() {
        let state = test_state(test_config("http://127.0.0.1:9"));

        let error = fetch_image(
            &state,
            "https://127.0.0.1/image.png",
            DEFAULT_MAX_IMAGE_BYTES,
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, "media_fetch_failed");
        assert!(error.message.contains("non-public"));
    }

    #[tokio::test]
    async fn redirect_from_allowlisted_host_is_revalidated() {
        let app = Router::new().route(
            "/attachment.png",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [("location", "http://169.254.169.254/latest/meta-data")],
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut config = test_config("http://127.0.0.1:9");
        config.allowed_attachment_hosts = vec!["127.0.0.1".to_owned()];
        let state = test_state(config);

        let error = fetch_image(
            &state,
            &format!("http://{addr}/attachment.png"),
            DEFAULT_MAX_IMAGE_BYTES,
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, "media_fetch_failed");
    }

    #[tokio::test]
    async fn allowlisted_finite_attachment_is_downloaded_and_normalized() {
        let image_url = spawn_image_mock().await;
        let captured = Arc::new(Mutex::new(None::<(HeaderMap, Value)>));
        let upstream_url = spawn_spark_mock(
            captured.clone(),
            StatusCode::OK,
            json!({ "id": "resp_attachment", "output_text": "A red image." }),
        )
        .await;
        let mut config = test_config(&upstream_url);
        config.allowed_attachment_hosts = vec!["127.0.0.1".to_owned()];
        let worker = spawn_worker(config).await;

        let response = reqwest::Client::new()
            .post(format!("{worker}/v1/chat/completions"))
            .bearer_auth("worker-secret")
            .json(&json!({
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "image_url",
                        "image_url": { "url": image_url }
                    }]
                }]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let request = captured.lock().unwrap().clone().unwrap().1;
        let normalized = request["input"][1]["content"][0]["image_url"]
            .as_str()
            .unwrap();
        assert!(normalized.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn image_canary_health_is_capability_local_and_recovers_immediately() {
        let metrics = CapabilityMetrics::default();

        metrics.observe_canary(false, 1200);
        assert!(
            metrics
                .render("model-test")
                .contains("health_state=\"degraded\"")
        );
        metrics.observe_canary(false, 1300);
        metrics.observe_canary(false, 1400);
        assert!(
            metrics
                .render("model-test")
                .contains("health_state=\"unavailable\"")
        );
        metrics.observe_canary(true, 900);

        let rendered = metrics.render("model-test");
        assert!(rendered.contains("health_state=\"healthy\""));
        assert!(rendered.contains(
            "finite_specialization_canary_latency_milliseconds{capability=\"image\",model_alias=\"model-test\",surface=\"tyk-public-frontdoor\"} 900"
        ));
    }

    #[tokio::test]
    async fn upstream_errors_are_bounded_and_do_not_echo_tokens() {
        let captured = Arc::new(Mutex::new(None::<(HeaderMap, Value)>));
        let upstream_url = spawn_spark_mock(
            captured,
            StatusCode::BAD_GATEWAY,
            json!({ "error": "spark-secret leaked upstream details" }),
        )
        .await;
        let worker = spawn_worker(test_config(&upstream_url)).await;
        let response = reqwest::Client::new()
            .post(format!("{worker}/v1/chat/completions"))
            .bearer_auth("worker-secret")
            .json(&json!({
                "messages": [{
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "describe" },
                        {
                            "type": "image_url",
                            "image_url": { "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=" }
                        }
                    ]
                }]
            }))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(!body.contains("spark-secret"));
        assert!(body.contains("502"));
    }

    fn test_config(spark_base_url: &str) -> WorkerConfig {
        WorkerConfig {
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            worker_bearer_token: Some("worker-secret".to_string()),
            spark_base_url: spark_base_url.to_string(),
            spark_bearer_token: Some("spark-secret".to_string()),
            vision_model: "qwopus-test".to_string(),
            max_image_bytes: DEFAULT_MAX_IMAGE_BYTES,
            max_inline_image_bytes: DEFAULT_MAX_INLINE_IMAGE_BYTES,
            max_output_chars: DEFAULT_MAX_OUTPUT_CHARS,
            image_download_timeout_seconds: 5,
            request_deadline_seconds: 120,
            allowed_attachment_hosts: Vec::new(),
            image_canary_interval_seconds: None,
        }
    }

    fn empty_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: None,
            messages: Vec::new(),
            temperature: None,
            max_tokens: None,
            max_completion_tokens: None,
            stream: None,
            finite_specialization: None,
        }
    }

    fn test_desired_state() -> InvocationSpecializationState {
        InvocationSpecializationState {
            capabilities: InvocationCapabilities {
                image: true,
                audio: false,
                video: false,
            },
            prompt_versions: InvocationPromptVersions {
                image: IMAGE_PROMPT_VERSION.to_owned(),
                audio: "aeon-audio-understanding-v1".to_owned(),
                video: "aeon-video-understanding-v1".to_owned(),
            },
            normalization_limits: InvocationNormalizationLimits {
                max_images: 2,
                max_inline_bytes: 2048,
                max_download_bytes: 4096,
                max_output_chars: 512,
            },
        }
    }

    fn test_limits() -> InvocationNormalizationLimits {
        InvocationNormalizationLimits {
            max_images: DEFAULT_MAX_IMAGES,
            max_inline_bytes: DEFAULT_MAX_INLINE_IMAGE_BYTES,
            max_download_bytes: DEFAULT_MAX_IMAGE_BYTES,
            max_output_chars: DEFAULT_MAX_OUTPUT_CHARS,
        }
    }

    fn test_state(config: WorkerConfig) -> WorkerState {
        WorkerState {
            config,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            metrics: CapabilityMetrics::default(),
        }
    }

    async fn spawn_worker(config: WorkerConfig) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app(config)).await.unwrap();
        });
        format!("http://{}", addr)
    }

    async fn spawn_spark_mock(
        captured: Arc<Mutex<Option<(HeaderMap, Value)>>>,
        status: StatusCode,
        response: Value,
    ) -> String {
        let app = Router::new().route(
            "/responses",
            post(move |headers: HeaderMap, body: Bytes| {
                let captured = captured.clone();
                let response = response.clone();
                async move {
                    let payload = serde_json::from_slice::<Value>(&body).unwrap();
                    *captured.lock().unwrap() = Some((headers, payload));
                    (status, Json(response))
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{}", addr)
    }

    async fn spawn_image_mock() -> String {
        let png = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
            .unwrap();
        let app = Router::new().route(
            "/attachment.png",
            get(move || {
                let png = png.clone();
                async move { ([("content-type", "image/png")], png) }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/attachment.png")
    }
}
