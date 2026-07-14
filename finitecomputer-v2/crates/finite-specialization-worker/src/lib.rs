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
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::sync::Semaphore;

pub const DEFAULT_SPARK_BASE_URL: &str = "https://inference.finite.computer/v1";
pub const DEFAULT_VISION_MODEL: &str = "aeon-gemma-4-12b-k4-nvfp4-unified-fast";
pub const IMAGE_PROMPT_VERSION: &str = "aeon-image-analysis-v1";
pub const AUDIO_PROMPT_VERSION: &str = "aeon-audio-understanding-v1";
pub const VIDEO_PROMPT_VERSION: &str = "aeon-video-understanding-v1";
const IMAGE_CAPABILITY_PROMPT: &str = "Interpret the supplied image faithfully. Follow the user's instruction, distinguish visible facts from uncertainty, and do not invoke tools.";
const AUDIO_CAPABILITY_PROMPT: &str = "Interpret the supplied audio semantically rather than returning a transcript alone. Follow the user's instruction, distinguish audible facts from uncertainty, and do not invoke tools.";
const VIDEO_CAPABILITY_PROMPT: &str = "Interpret the chronological timestamped frames as one visual sequence. Follow the user's instruction, distinguish visible facts from uncertainty, do not infer unheard audio, and do not invoke tools.";
const DEFAULT_MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
const DEFAULT_MAX_INLINE_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_MAX_OUTPUT_CHARS: usize = 32 * 1024;
const DEFAULT_MAX_IMAGES: usize = 8;
const DEFAULT_MAX_AUDIO_DURATION_SECONDS: u64 = 900;
const DEFAULT_MAX_VIDEO_DURATION_SECONDS: u64 = 600;
const DEFAULT_MAX_VIDEO_FRAMES: usize = 16;

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
    pub audio_canary_interval_seconds: Option<u64>,
    pub video_canary_interval_seconds: Option<u64>,
    pub max_audio_duration_seconds: u64,
    pub max_video_duration_seconds: u64,
    pub max_video_frames: usize,
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
    pub media_temp_dir: PathBuf,
    pub media_concurrency: usize,
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
            audio_canary_interval_seconds: Some(
                non_empty_env("FINITE_SPECIALIZATION_AUDIO_CANARY_INTERVAL_SECONDS")
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(300)
                    .clamp(60, 3600),
            ),
            video_canary_interval_seconds: Some(
                non_empty_env("FINITE_SPECIALIZATION_VIDEO_CANARY_INTERVAL_SECONDS")
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(300)
                    .clamp(60, 3600),
            ),
            max_audio_duration_seconds: non_empty_env(
                "FINITE_SPECIALIZATION_MAX_AUDIO_DURATION_SECONDS",
            )
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_MAX_AUDIO_DURATION_SECONDS)
            .clamp(1, DEFAULT_MAX_AUDIO_DURATION_SECONDS),
            max_video_duration_seconds: non_empty_env(
                "FINITE_SPECIALIZATION_MAX_VIDEO_DURATION_SECONDS",
            )
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_MAX_VIDEO_DURATION_SECONDS)
            .clamp(1, DEFAULT_MAX_VIDEO_DURATION_SECONDS),
            max_video_frames: non_empty_env("FINITE_SPECIALIZATION_MAX_VIDEO_FRAMES")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(DEFAULT_MAX_VIDEO_FRAMES)
                .clamp(1, DEFAULT_MAX_VIDEO_FRAMES),
            ffmpeg_path: PathBuf::from(
                non_empty_env("FINITE_SPECIALIZATION_FFMPEG_PATH")
                    .unwrap_or_else(|| "ffmpeg".to_owned()),
            ),
            ffprobe_path: PathBuf::from(
                non_empty_env("FINITE_SPECIALIZATION_FFPROBE_PATH")
                    .unwrap_or_else(|| "ffprobe".to_owned()),
            ),
            media_temp_dir: PathBuf::from(
                non_empty_env("FINITE_SPECIALIZATION_MEDIA_TEMP_DIR")
                    .unwrap_or_else(|| "/tmp".to_owned()),
            ),
            media_concurrency: non_empty_env("FINITE_SPECIALIZATION_MEDIA_CONCURRENCY")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(2)
                .clamp(1, 8),
        }
    }
}

#[derive(Debug)]
struct WorkerState {
    config: WorkerConfig,
    client: Client,
    metrics: CapabilityMetrics,
    media_slots: Arc<Semaphore>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capability {
    Image = 0,
    Audio = 1,
    Video = 2,
}

impl Capability {
    const ALL: [Self; 3] = [Self::Image, Self::Audio, Self::Video];

    fn name(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }

    fn prompt_version(self) -> &'static str {
        match self {
            Self::Image => IMAGE_PROMPT_VERSION,
            Self::Audio => AUDIO_PROMPT_VERSION,
            Self::Video => VIDEO_PROMPT_VERSION,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug)]
struct CanaryObservation {
    health_state: &'static str,
    consecutive_failures: u64,
    last_completed_timestamp_seconds: u64,
    last_latency_milliseconds: u64,
}

#[derive(Debug)]
struct FailureObservation {
    request_identifier: u64,
    duration_milliseconds: u64,
    error_code: &'static str,
    state_version: &'static str,
}

impl Default for CanaryObservation {
    fn default() -> Self {
        Self {
            health_state: "stale",
            consecutive_failures: 0,
            last_completed_timestamp_seconds: 0,
            last_latency_milliseconds: 0,
        }
    }
}

#[derive(Debug)]
struct CapabilityMetrics {
    canaries: [Mutex<CanaryObservation>; 3],
    requests: [AtomicU64; 3],
    errors_by_code: [Mutex<BTreeMap<&'static str, u64>>; 3],
    last_failures: [Mutex<Option<FailureObservation>>; 3],
}

impl Default for CapabilityMetrics {
    fn default() -> Self {
        Self {
            canaries: std::array::from_fn(|_| Mutex::new(CanaryObservation::default())),
            requests: std::array::from_fn(|_| AtomicU64::new(0)),
            errors_by_code: std::array::from_fn(|_| Mutex::new(BTreeMap::new())),
            last_failures: std::array::from_fn(|_| Mutex::new(None)),
        }
    }
}

impl CapabilityMetrics {
    fn observe_request(
        &self,
        capability: Capability,
        error_code: Option<&'static str>,
        request_id: &str,
        duration_milliseconds: u64,
    ) {
        self.requests[capability.index()].fetch_add(1, Ordering::Relaxed);
        if let Some(error_code) = error_code {
            let mut errors = self.errors_by_code[capability.index()].lock().unwrap();
            *errors.entry(error_code).or_default() += 1;
            *self.last_failures[capability.index()].lock().unwrap() = Some(FailureObservation {
                request_identifier: bounded_request_identifier(request_id),
                duration_milliseconds,
                error_code,
                state_version: capability.prompt_version(),
            });
        }
    }

    fn observe_canary(&self, capability: Capability, succeeded: bool, latency_milliseconds: u64) {
        let mut observation = self.canaries[capability.index()].lock().unwrap();
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
        let model_alias = prometheus_label(model_alias);
        let mut rendered = concat!(
            "# HELP finite_specialization_capability_health Semantic canary health for one specialization capability.\n",
            "# TYPE finite_specialization_capability_health gauge\n",
            "# HELP finite_specialization_canary_last_completed_timestamp_seconds Unix timestamp of the last completed semantic canary.\n",
            "# TYPE finite_specialization_canary_last_completed_timestamp_seconds gauge\n",
            "# HELP finite_specialization_canary_latency_milliseconds Last semantic canary latency.\n",
            "# TYPE finite_specialization_canary_latency_milliseconds gauge\n",
            "# HELP finite_specialization_requests_total Specialization capability requests.\n",
            "# TYPE finite_specialization_requests_total counter\n",
            "# HELP finite_specialization_errors_total Specialization capability errors.\n",
            "# TYPE finite_specialization_errors_total counter\n",
            "# HELP finite_specialization_last_failure_duration_milliseconds Most recent failure timing and safe correlation fields for one capability.\n",
            "# TYPE finite_specialization_last_failure_duration_milliseconds gauge\n",
            "# HELP finite_specialization_last_failure_request_identifier Numeric identifier returned with the most recent failure for one capability.\n",
            "# TYPE finite_specialization_last_failure_request_identifier gauge\n"
        )
        .to_owned();
        for capability in Capability::ALL {
            let observation = self.canaries[capability.index()].lock().unwrap();
            let health_state = effective_canary_health(&observation);
            rendered.push_str(&format!(
                "finite_specialization_capability_health{{capability=\"{}\",health_state=\"{}\",model_alias=\"{}\",prompt_version=\"{}\",surface=\"tyk-public-frontdoor\"}} 1\n",
                capability.name(), health_state, model_alias, capability.prompt_version()
            ));
            rendered.push_str(&format!(
                "finite_specialization_canary_last_completed_timestamp_seconds{{capability=\"{}\",model_alias=\"{}\",surface=\"tyk-public-frontdoor\"}} {}\n",
                capability.name(), model_alias, observation.last_completed_timestamp_seconds
            ));
            rendered.push_str(&format!(
                "finite_specialization_canary_latency_milliseconds{{capability=\"{}\",model_alias=\"{}\",surface=\"tyk-public-frontdoor\"}} {}\n",
                capability.name(), model_alias, observation.last_latency_milliseconds
            ));
            rendered.push_str(&format!(
                "finite_specialization_requests_total{{capability=\"{}\",model_alias=\"{}\",surface=\"tyk-public-frontdoor\"}} {}\n",
                capability.name(), model_alias, self.requests[capability.index()].load(Ordering::Relaxed)
            ));
            let errors = self.errors_by_code[capability.index()].lock().unwrap();
            if errors.is_empty() {
                rendered.push_str(&format!(
                    "finite_specialization_errors_total{{capability=\"{}\",error_code=\"none\",model_alias=\"{}\",surface=\"tyk-public-frontdoor\"}} 0\n",
                    capability.name(), model_alias
                ));
            } else {
                for (code, count) in errors.iter() {
                    rendered.push_str(&format!(
                        "finite_specialization_errors_total{{capability=\"{}\",error_code=\"{}\",model_alias=\"{}\",surface=\"tyk-public-frontdoor\"}} {}\n",
                        capability.name(), prometheus_label(code), model_alias, count
                    ));
                }
            }
            if let Some(failure) = self.last_failures[capability.index()]
                .lock()
                .unwrap()
                .as_ref()
            {
                rendered.push_str(&format!(
                    "finite_specialization_last_failure_duration_milliseconds{{capability=\"{}\",error_class=\"{}\",model_alias=\"{}\",state_version=\"{}\",surface=\"tyk-public-frontdoor\"}} {}\n",
                    capability.name(),
                    prometheus_label(failure.error_code),
                    model_alias,
                    failure.state_version,
                    failure.duration_milliseconds,
                ));
                rendered.push_str(&format!(
                    "finite_specialization_last_failure_request_identifier{{capability=\"{}\",error_class=\"{}\",model_alias=\"{}\",state_version=\"{}\",surface=\"tyk-public-frontdoor\"}} {}\n",
                    capability.name(),
                    prometheus_label(failure.error_code),
                    model_alias,
                    failure.state_version,
                    failure.request_identifier,
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
    let image_canary_interval = config.image_canary_interval_seconds;
    let audio_canary_interval = config.audio_canary_interval_seconds;
    let video_canary_interval = config.video_canary_interval_seconds;
    let media_concurrency = config.media_concurrency;
    let state = Arc::new(WorkerState {
        config,
        client,
        metrics: CapabilityMetrics::default(),
        media_slots: Arc::new(Semaphore::new(media_concurrency)),
    });
    if let Some(interval_seconds) = image_canary_interval {
        tokio::spawn(run_image_canary_loop(Arc::clone(&state), interval_seconds));
    }
    if let Some(interval_seconds) = audio_canary_interval {
        tokio::spawn(run_audio_canary_loop(Arc::clone(&state), interval_seconds));
    }
    if let Some(interval_seconds) = video_canary_interval {
        tokio::spawn(run_video_canary_loop(Arc::clone(&state), interval_seconds));
    }
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/chat/completions", post(chat_completions))
        .layer(DefaultBodyLimit::max(48 * 1024 * 1024))
        .with_state(state)
}

async fn health(State(state): State<Arc<WorkerState>>) -> Json<Value> {
    let capabilities = Capability::ALL.into_iter().fold(
        serde_json::Map::new(),
        |mut capabilities, capability| {
            let observation = state.metrics.canaries[capability.index()].lock().unwrap();
            capabilities.insert(
                capability.name().to_owned(),
                json!({
                    "health": effective_canary_health(&observation),
                    "prompt_version": capability.prompt_version(),
                    "last_completed_timestamp_seconds": observation.last_completed_timestamp_seconds,
                }),
            );
            capabilities
        },
    );
    Json(json!({
        "ok": true,
        "service": "finite-specialization-worker",
        "capabilities": capabilities,
    }))
}

fn effective_canary_health(observation: &CanaryObservation) -> &'static str {
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
    let capability = classify_request(&input)?;
    let started = Instant::now();
    let request_id = correlation_request_id(&headers);
    let mut result = tokio::time::timeout(
        Duration::from_secs(state.config.request_deadline_seconds),
        analyze_capability(&state, &input, capability, &request_id),
    )
    .await
    .unwrap_or_else(|_| {
        Err(WorkerError::request_timeout(format!(
            "{} analysis deadline expired",
            capability.name()
        )))
    });
    if let Err(error) = &mut result {
        error.request_id = Some(request_id.clone());
    }
    state.metrics.observe_request(
        capability,
        result.as_ref().err().map(|error| error.code),
        &request_id,
        started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    );
    result.map(Json)
}

fn correlation_request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
        .map(str::to_owned)
        .unwrap_or_else(generated_request_id)
}

fn generated_request_id() -> String {
    format!(
        "req_specialization_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    )
}

fn bounded_request_identifier(request_id: &str) -> u64 {
    let hash = request_id
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    (hash & ((1_u64 << 53) - 1)).max(1)
}

async fn analyze_capability(
    state: &WorkerState,
    input: &ChatCompletionRequest,
    capability: Capability,
    correlation_id: &str,
) -> Result<Value, WorkerError> {
    match capability {
        Capability::Image => analyze_image(state, input, correlation_id).await,
        Capability::Audio => analyze_audio(state, input, correlation_id).await,
        Capability::Video => analyze_video(state, input, correlation_id).await,
    }
}

fn classify_request(input: &ChatCompletionRequest) -> Result<Capability, WorkerError> {
    let mut counts = [0usize; 3];
    for message in &input.messages {
        let ChatContent::Parts(parts) = &message.content else {
            continue;
        };
        for part in parts {
            let capability = match part.get("type").and_then(Value::as_str) {
                Some("image_url" | "input_image") => Some(Capability::Image),
                Some("input_audio" | "audio_url") => Some(Capability::Audio),
                Some("video_url" | "input_video") => Some(Capability::Video),
                _ => None,
            };
            if let Some(capability) = capability {
                counts[capability.index()] += 1;
            }
        }
    }
    let active: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| counts[capability.index()] > 0)
        .collect();
    if active.len() > 1 {
        return Err(WorkerError::bad_request_with_code(
            "one specialization request must contain one primary media type",
            "mixed_media_requires_decomposition",
        ));
    }
    let capability = active.into_iter().next().ok_or_else(|| {
        WorkerError::bad_request_with_code(
            "specialization analysis requires one primary media item",
            "unsupported_media_format",
        )
    })?;
    if matches!(capability, Capability::Audio | Capability::Video)
        && counts[capability.index()] != 1
    {
        return Err(WorkerError::bad_request_with_code(
            "audio and video requests require exactly one media item",
            "unsupported_media_format",
        ));
    }
    Ok(capability)
}

async fn analyze_image(
    state: &WorkerState,
    input: &ChatCompletionRequest,
    correlation_id: &str,
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
        .header("x-request-id", correlation_id)
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
        required_upstream_text(extract_response_text(&upstream), Capability::Image)?,
        limits.max_output_chars,
    );
    output["choices"][0]["message"]["content"] = json!(text);
    let upstream_request_id = upstream
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WorkerError::bad_gateway("Spark image response omitted its request ID"))?;
    output["specialization_result"] = json!({
        "capability": "image",
        "text": text,
        "model": state.config.vision_model,
        "request_id": correlation_id,
        "upstream_request_id": upstream_request_id,
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

fn chat_request_for_audio(
    input: &ChatCompletionRequest,
    model: &str,
) -> Result<Value, WorkerError> {
    if input.stream.unwrap_or(false) {
        return Err(WorkerError::bad_request(
            "streaming audio interpretation is not supported",
        ));
    }
    let mut messages = vec![json!({
        "role": "developer",
        "content": AUDIO_CAPABILITY_PROMPT,
    })];
    for message in &input.messages {
        let role = normalize_role(&message.role)?;
        let content = match &message.content {
            ChatContent::Text(text) => Value::String(text.clone()),
            ChatContent::Parts(parts) => Value::Array(
                parts
                    .iter()
                    .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                        Some("text" | "input_text" | "input_audio") => Some(part.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
        };
        messages.push(json!({ "role": role, "content": content }));
    }
    let mut request = json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "temperature": input.temperature.clone().unwrap_or_else(|| json!(0)),
    });
    if let Some(max_tokens) = input.max_tokens.or(input.max_completion_tokens) {
        request["max_tokens"] = json!(max_tokens);
    }
    Ok(request)
}

fn uniform_video_timestamps(duration_seconds: f64, max_frames: usize) -> Vec<f64> {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 || max_frames == 0 {
        return Vec::new();
    }
    let last = (duration_seconds - 0.001).max(0.0);
    if duration_seconds <= max_frames as f64 {
        return (0..=last.floor() as usize)
            .map(|second| second as f64)
            .collect();
    }
    if max_frames == 1 {
        return vec![0.0];
    }
    (0..max_frames)
        .map(|index| last * index as f64 / (max_frames - 1) as f64)
        .collect()
}

#[derive(Debug)]
enum MediaSource {
    DataUri(String),
    Inline { mime: &'static str, encoded: String },
    Url(String),
}

async fn analyze_audio(
    state: &WorkerState,
    input: &ChatCompletionRequest,
    correlation_id: &str,
) -> Result<Value, WorkerError> {
    let started = Instant::now();
    let limits = effective_media_limits(state, input, Capability::Audio)?;
    let spark_token = state
        .config
        .spark_bearer_token
        .as_deref()
        .ok_or_else(|| WorkerError::unavailable("Spark gateway token is not configured"))?;
    let normalized = normalize_audio_input(state, input, limits).await?;
    let upstream_request = chat_request_for_audio(&normalized, &state.config.vision_model)?;
    let response = state
        .client
        .post(format!(
            "{}/chat/completions",
            state.config.spark_base_url.trim_end_matches('/')
        ))
        .bearer_auth(spark_token)
        .header("x-request-id", correlation_id)
        .json(&upstream_request)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                WorkerError::request_timeout("Spark audio interpretation timed out")
            } else {
                WorkerError::bad_gateway("Spark audio interpretation request failed")
            }
        })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|_| WorkerError::bad_gateway("Spark audio response was not readable"))?;
    if !status.is_success() {
        return Err(WorkerError::bad_gateway(format!(
            "Spark audio request failed with status {}",
            status.as_u16()
        )));
    }
    let upstream: Value = serde_json::from_str(&body)
        .map_err(|_| WorkerError::bad_gateway("Spark audio response was not valid JSON"))?;
    let text = truncate_chars(
        required_upstream_text(extract_chat_completion_text(&upstream), Capability::Audio)?,
        limits.max_output_chars,
    );
    let upstream_request_id = upstream
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WorkerError::bad_gateway("Spark audio response omitted its request ID"))?
        .to_owned();
    let mut output = upstream;
    if output.pointer("/choices/0/message").is_none() {
        return Err(WorkerError::bad_gateway(
            "Spark audio response did not contain an assistant message",
        ));
    }
    output["choices"][0]["message"]["content"] = json!(text);
    output["specialization_result"] = specialization_result(
        Capability::Audio,
        text,
        &state.config.vision_model,
        correlation_id,
        &upstream_request_id,
        started,
    );
    Ok(output)
}

async fn analyze_video(
    state: &WorkerState,
    input: &ChatCompletionRequest,
    correlation_id: &str,
) -> Result<Value, WorkerError> {
    let started = Instant::now();
    let limits = effective_media_limits(state, input, Capability::Video)?;
    let spark_token = state
        .config
        .spark_bearer_token
        .as_deref()
        .ok_or_else(|| WorkerError::unavailable("Spark gateway token is not configured"))?;
    let normalized = normalize_video_input(state, input, limits).await?;
    let upstream_request = responses_request_for_video(&normalized, &state.config.vision_model)?;
    let response = state
        .client
        .post(format!(
            "{}/responses",
            state.config.spark_base_url.trim_end_matches('/')
        ))
        .bearer_auth(spark_token)
        .header("x-request-id", correlation_id)
        .json(&upstream_request)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                WorkerError::request_timeout("Spark sampled-video analysis timed out")
            } else {
                WorkerError::bad_gateway("Spark sampled-video analysis request failed")
            }
        })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|_| WorkerError::bad_gateway("Spark video response was not readable"))?;
    if !status.is_success() {
        return Err(WorkerError::bad_gateway(format!(
            "Spark video request failed with status {}",
            status.as_u16()
        )));
    }
    let upstream: Value = serde_json::from_str(&body)
        .map_err(|_| WorkerError::bad_gateway("Spark video response was not valid JSON"))?;
    let text = truncate_chars(
        required_upstream_text(extract_response_text(&upstream), Capability::Video)?,
        limits.max_output_chars,
    );
    let upstream_request_id = upstream
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WorkerError::bad_gateway("Spark video response omitted its request ID"))?;
    let mut output = chat_completion_response_from_responses(&upstream, &state.config.vision_model);
    output["choices"][0]["message"]["content"] = json!(text);
    output["specialization_result"] = specialization_result(
        Capability::Video,
        text,
        &state.config.vision_model,
        correlation_id,
        upstream_request_id,
        started,
    );
    Ok(output)
}

fn specialization_result(
    capability: Capability,
    text: String,
    model: &str,
    request_id: &str,
    upstream_request_id: &str,
    started: Instant,
) -> Value {
    json!({
        "capability": capability.name(),
        "text": text,
        "model": model,
        "request_id": request_id,
        "upstream_request_id": upstream_request_id,
        "duration_ms": started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

fn required_upstream_text(
    text: Option<String>,
    capability: Capability,
) -> Result<String, WorkerError> {
    text.filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            WorkerError::bad_gateway(format!(
                "Spark {} response omitted assistant text",
                capability.name()
            ))
        })
}

fn effective_media_limits(
    state: &WorkerState,
    input: &ChatCompletionRequest,
    capability: Capability,
) -> Result<InvocationNormalizationLimits, WorkerError> {
    let requested = input
        .finite_specialization
        .as_ref()
        .map(|desired| {
            let enabled = match capability {
                Capability::Image => desired.capabilities.image,
                Capability::Audio => desired.capabilities.audio,
                Capability::Video => desired.capabilities.video,
            };
            if !enabled {
                return Err(WorkerError::unavailable(format!(
                    "{} capability is disabled",
                    capability.name()
                )));
            }
            let prompt = match capability {
                Capability::Image => &desired.prompt_versions.image,
                Capability::Audio => &desired.prompt_versions.audio,
                Capability::Video => &desired.prompt_versions.video,
            };
            if prompt != capability.prompt_version() {
                return Err(WorkerError::bad_request_with_code(
                    format!("{} prompt version is not supported", capability.name()),
                    "unsupported_media_format",
                ));
            }
            Ok(desired.normalization_limits)
        })
        .transpose()?
        .unwrap_or(InvocationNormalizationLimits {
            max_images: DEFAULT_MAX_IMAGES,
            max_inline_bytes: state.config.max_inline_image_bytes,
            max_download_bytes: state.config.max_image_bytes,
            max_output_chars: state.config.max_output_chars,
        });
    Ok(InvocationNormalizationLimits {
        max_images: requested.max_images.clamp(1, DEFAULT_MAX_IMAGES),
        max_inline_bytes: requested
            .max_inline_bytes
            .clamp(1024, state.config.max_inline_image_bytes),
        max_download_bytes: requested
            .max_download_bytes
            .clamp(1024, state.config.max_image_bytes),
        max_output_chars: requested
            .max_output_chars
            .clamp(256, state.config.max_output_chars),
    })
}

async fn normalize_audio_input(
    state: &WorkerState,
    input: &ChatCompletionRequest,
    limits: InvocationNormalizationLimits,
) -> Result<ChatCompletionRequest, WorkerError> {
    let source = media_source(input, Capability::Audio)?;
    let bytes = retrieve_media(state, source, Capability::Audio, limits).await?;
    let wav = normalize_audio_bytes(state, bytes, state.config.max_image_bytes).await?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(wav);
    replace_primary_media(
        input,
        Capability::Audio,
        vec![json!({
            "type": "input_audio",
            "input_audio": { "data": encoded, "format": "wav" },
        })],
    )
}

async fn normalize_video_input(
    state: &WorkerState,
    input: &ChatCompletionRequest,
    limits: InvocationNormalizationLimits,
) -> Result<ChatCompletionRequest, WorkerError> {
    let source = media_source(input, Capability::Video)?;
    let bytes = retrieve_media(state, source, Capability::Video, limits).await?;
    let frames = sample_video_frames(state, bytes).await?;
    let mut parts = Vec::with_capacity(frames.len() * 2);
    for (timestamp, png) in frames {
        parts.push(json!({
            "type": "text",
            "text": format!("Frame at {timestamp:.3} seconds"),
        }));
        parts.push(json!({
            "type": "image_url",
            "image_url": {
                "url": format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(png)
                )
            },
        }));
    }
    replace_primary_media(input, Capability::Video, parts)
}

fn media_source(
    input: &ChatCompletionRequest,
    capability: Capability,
) -> Result<MediaSource, WorkerError> {
    for message in &input.messages {
        let ChatContent::Parts(parts) = &message.content else {
            continue;
        };
        for part in parts {
            let part_type = part.get("type").and_then(Value::as_str).unwrap_or_default();
            match (capability, part_type) {
                (Capability::Audio, "input_audio") => {
                    let audio = part.get("input_audio").ok_or_else(|| {
                        WorkerError::bad_request_with_code(
                            "input_audio content is missing input_audio",
                            "media_decode_failed",
                        )
                    })?;
                    if let Some(url) = audio.get("url").and_then(Value::as_str) {
                        return Ok(source_from_url_or_data(url));
                    }
                    let encoded = audio.get("data").and_then(Value::as_str).ok_or_else(|| {
                        WorkerError::bad_request_with_code(
                            "input_audio content is missing data",
                            "media_decode_failed",
                        )
                    })?;
                    let format = audio.get("format").and_then(Value::as_str).unwrap_or("wav");
                    let mime = audio_format_mime(format)?;
                    return Ok(MediaSource::Inline {
                        mime,
                        encoded: encoded.to_owned(),
                    });
                }
                (Capability::Audio, "audio_url") => {
                    let url = nested_url(part, "audio_url")?;
                    return Ok(source_from_url_or_data(&url));
                }
                (Capability::Video, "video_url") | (Capability::Video, "input_video") => {
                    let key = if part_type == "video_url" {
                        "video_url"
                    } else {
                        "input_video"
                    };
                    let url = nested_url(part, key)?;
                    return Ok(source_from_url_or_data(&url));
                }
                _ => {}
            }
        }
    }
    Err(WorkerError::bad_request_with_code(
        format!("{} source is missing", capability.name()),
        "unsupported_media_format",
    ))
}

fn nested_url(part: &Value, key: &str) -> Result<String, WorkerError> {
    part.get(key)
        .and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("url").and_then(Value::as_str))
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            WorkerError::bad_request_with_code(
                format!("{key} content is missing url"),
                "media_fetch_failed",
            )
        })
}

fn source_from_url_or_data(value: &str) -> MediaSource {
    if value.starts_with("data:") {
        MediaSource::DataUri(value.to_owned())
    } else {
        MediaSource::Url(value.to_owned())
    }
}

fn audio_format_mime(format: &str) -> Result<&'static str, WorkerError> {
    match format.trim().to_ascii_lowercase().as_str() {
        "wav" | "wave" => Ok("audio/wav"),
        "mp3" | "mpeg" => Ok("audio/mpeg"),
        "ogg" | "opus" => Ok("audio/ogg"),
        "flac" => Ok("audio/flac"),
        "m4a" | "mp4" => Ok("audio/mp4"),
        _ => Err(WorkerError::bad_request_with_code(
            "input_audio format is unsupported",
            "unsupported_media_format",
        )),
    }
}

async fn retrieve_media(
    state: &WorkerState,
    source: MediaSource,
    capability: Capability,
    limits: InvocationNormalizationLimits,
) -> Result<Vec<u8>, WorkerError> {
    let bytes = match source {
        MediaSource::DataUri(source) => {
            decode_media_data_uri(&source, capability, limits.max_inline_bytes)?
        }
        MediaSource::Inline { mime, encoded } => {
            decode_inline_media(mime, &encoded, capability, limits.max_inline_bytes)?
        }
        MediaSource::Url(url) => {
            fetch_binary_media(state, &url, capability, limits.max_download_bytes).await?
        }
    };
    if bytes.is_empty() {
        return Err(WorkerError::bad_request_with_code(
            format!("{} media is empty", capability.name()),
            "media_decode_failed",
        ));
    }
    Ok(bytes)
}

fn decode_media_data_uri(
    source: &str,
    capability: Capability,
    max_bytes: u64,
) -> Result<Vec<u8>, WorkerError> {
    let (metadata, encoded) = source.split_once(',').ok_or_else(|| {
        WorkerError::bad_request_with_code("inline media is malformed", "media_decode_failed")
    })?;
    let mime = metadata
        .strip_prefix("data:")
        .and_then(|value| value.strip_suffix(";base64"))
        .ok_or_else(|| {
            WorkerError::bad_request_with_code(
                "inline media must use a base64 data URI",
                "unsupported_media_format",
            )
        })?;
    let valid_mime = match capability {
        Capability::Audio => mime.starts_with("audio/"),
        Capability::Video => mime.starts_with("video/"),
        Capability::Image => mime.starts_with("image/"),
    };
    if !valid_mime {
        return Err(WorkerError::bad_request_with_code(
            format!("inline {} has the wrong media type", capability.name()),
            "unsupported_media_format",
        ));
    }
    decode_inline_media(mime, encoded, capability, max_bytes)
}

fn decode_inline_media(
    _mime: &str,
    encoded: &str,
    capability: Capability,
    max_bytes: u64,
) -> Result<Vec<u8>, WorkerError> {
    if encoded.len() as u64 > max_bytes.saturating_mul(2) {
        return Err(WorkerError::payload_too_large(format!(
            "inline {} exceeds the specialization limit",
            capability.name()
        )));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| {
            WorkerError::bad_request_with_code(
                format!("inline {} base64 is invalid", capability.name()),
                "media_decode_failed",
            )
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(WorkerError::payload_too_large(format!(
            "inline {} exceeds the specialization limit",
            capability.name()
        )));
    }
    Ok(bytes)
}

async fn fetch_binary_media(
    state: &WorkerState,
    source: &str,
    capability: Capability,
    max_download_bytes: u64,
) -> Result<Vec<u8>, WorkerError> {
    let mut url = reqwest::Url::parse(source).map_err(|_| {
        WorkerError::bad_request_with_code("media URL is invalid", "media_fetch_failed")
    })?;
    for redirect_count in 0..=3 {
        let client = media_client_for_url(state, &url).await?;
        let response = tokio::time::timeout(
            Duration::from_secs(state.config.image_download_timeout_seconds),
            client.get(url.clone()).send(),
        )
        .await
        .map_err(|_| WorkerError::request_timeout("media retrieval timed out"))?
        .map_err(|_| {
            WorkerError::bad_request_with_code("media retrieval failed", "media_fetch_failed")
        })?;
        if response.status().is_redirection() {
            if redirect_count == 3 {
                return Err(WorkerError::bad_request_with_code(
                    "media retrieval exceeded the redirect limit",
                    "media_fetch_failed",
                ));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    WorkerError::bad_request_with_code(
                        "media redirect is invalid",
                        "media_fetch_failed",
                    )
                })?;
            url = url.join(location).map_err(|_| {
                WorkerError::bad_request_with_code(
                    "media redirect is invalid",
                    "media_fetch_failed",
                )
            })?;
            continue;
        }
        if !response.status().is_success() {
            return Err(WorkerError::bad_request_with_code(
                "media retrieval failed",
                "media_fetch_failed",
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_download_bytes)
        {
            return Err(WorkerError::payload_too_large(format!(
                "downloaded {} exceeds the specialization limit",
                capability.name()
            )));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                WorkerError::bad_request_with_code("media retrieval failed", "media_fetch_failed")
            })?;
            if bytes.len().saturating_add(chunk.len()) as u64 > max_download_bytes {
                return Err(WorkerError::payload_too_large(format!(
                    "downloaded {} exceeds the specialization limit",
                    capability.name()
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(bytes);
    }
    unreachable!("redirect loop always returns")
}

async fn normalize_audio_bytes(
    state: &WorkerState,
    bytes: Vec<u8>,
    max_output_bytes: u64,
) -> Result<Vec<u8>, WorkerError> {
    let _permit = state
        .media_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| WorkerError::capacity("media normalization capacity is exhausted"))?;
    let directory = tempfile::Builder::new()
        .prefix("finite-audio-")
        .tempdir_in(&state.config.media_temp_dir)
        .map_err(|_| WorkerError::unavailable("audio workspace could not be created"))?;
    let input = directory.path().join("input.media");
    let output = directory.path().join("normalized.wav");
    tokio::fs::write(&input, bytes)
        .await
        .map_err(|_| WorkerError::unavailable("audio input could not be staged"))?;
    let duration = probe_media(state, &input, Capability::Audio).await?;
    if duration > state.config.max_audio_duration_seconds as f64 {
        return Err(WorkerError::duration_exceeded(
            "audio duration exceeds the specialization limit",
        ));
    }
    let status = Command::new(&state.config.ffmpeg_path)
        .args(["-v", "error", "-y", "-i"])
        .arg(&input)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .map_err(|_| WorkerError::unavailable("audio decoder is unavailable"))?;
    if !status.success() {
        return Err(media_decode_failure(Capability::Audio));
    }
    let normalized = tokio::fs::read(output)
        .await
        .map_err(|_| media_decode_failure(Capability::Audio))?;
    if normalized.len() as u64 > max_output_bytes {
        return Err(WorkerError::payload_too_large(
            "normalized audio exceeds the specialization limit",
        ));
    }
    Ok(normalized)
}

async fn sample_video_frames(
    state: &WorkerState,
    bytes: Vec<u8>,
) -> Result<Vec<(f64, Vec<u8>)>, WorkerError> {
    let _permit = state
        .media_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| WorkerError::capacity("media normalization capacity is exhausted"))?;
    let directory = tempfile::Builder::new()
        .prefix("finite-video-")
        .tempdir_in(&state.config.media_temp_dir)
        .map_err(|_| WorkerError::unavailable("video workspace could not be created"))?;
    let input = directory.path().join("input.media");
    tokio::fs::write(&input, bytes)
        .await
        .map_err(|_| WorkerError::unavailable("video input could not be staged"))?;
    let duration = probe_media(state, &input, Capability::Video).await?;
    if duration > state.config.max_video_duration_seconds as f64 {
        return Err(WorkerError::duration_exceeded(
            "video duration exceeds the specialization limit",
        ));
    }
    let timestamps = uniform_video_timestamps(duration, state.config.max_video_frames);
    if timestamps.is_empty() {
        return Err(media_decode_failure(Capability::Video));
    }
    let mut frames = Vec::with_capacity(timestamps.len());
    for (index, timestamp) in timestamps.into_iter().enumerate() {
        let output = directory.path().join(format!("frame-{index:02}.png"));
        let status = Command::new(&state.config.ffmpeg_path)
            .args(["-v", "error", "-y", "-ss"])
            .arg(format!("{timestamp:.3}"))
            .arg("-i")
            .arg(&input)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "scale=1280:-2:force_original_aspect_ratio=decrease",
            ])
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await
            .map_err(|_| WorkerError::unavailable("video decoder is unavailable"))?;
        if !status.success() {
            return Err(media_decode_failure(Capability::Video));
        }
        let png = tokio::fs::read(output)
            .await
            .map_err(|_| media_decode_failure(Capability::Video))?;
        image::load_from_memory(&png).map_err(|_| media_decode_failure(Capability::Video))?;
        frames.push((timestamp, png));
    }
    Ok(frames)
}

async fn probe_media(
    state: &WorkerState,
    input: &Path,
    capability: Capability,
) -> Result<f64, WorkerError> {
    let output = Command::new(&state.config.ffprobe_path)
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(input)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|_| WorkerError::unavailable("media probe is unavailable"))?;
    if !output.status.success() {
        return Err(media_decode_failure(capability));
    }
    let probe: Value =
        serde_json::from_slice(&output.stdout).map_err(|_| media_decode_failure(capability))?;
    let expected_stream = match capability {
        Capability::Audio => "audio",
        Capability::Video => "video",
        Capability::Image => "video",
    };
    let has_stream = probe
        .get("streams")
        .and_then(Value::as_array)
        .is_some_and(|streams| {
            streams.iter().any(|stream| {
                stream.get("codec_type").and_then(Value::as_str) == Some(expected_stream)
            })
        });
    let duration = probe
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0);
    if !has_stream || duration.is_none() {
        return Err(media_decode_failure(capability));
    }
    Ok(duration.unwrap_or_default())
}

fn replace_primary_media(
    input: &ChatCompletionRequest,
    capability: Capability,
    replacement: Vec<Value>,
) -> Result<ChatCompletionRequest, WorkerError> {
    let mut normalized = input.clone();
    let mut replaced = false;
    for message in &mut normalized.messages {
        let ChatContent::Parts(parts) = &mut message.content else {
            continue;
        };
        let mut output = Vec::new();
        for part in parts.iter() {
            let is_primary = matches!(
                (capability, part.get("type").and_then(Value::as_str)),
                (Capability::Audio, Some("input_audio" | "audio_url"))
                    | (Capability::Video, Some("video_url" | "input_video"))
            );
            if is_primary {
                if replaced {
                    return Err(WorkerError::bad_request_with_code(
                        "audio and video requests require exactly one media item",
                        "unsupported_media_format",
                    ));
                }
                output.extend(replacement.clone());
                replaced = true;
            } else {
                output.push(part.clone());
            }
        }
        *parts = output;
    }
    if !replaced {
        return Err(WorkerError::bad_request_with_code(
            format!("{} source is missing", capability.name()),
            "unsupported_media_format",
        ));
    }
    Ok(normalized)
}

fn responses_request_for_video(
    input: &ChatCompletionRequest,
    model: &str,
) -> Result<Value, WorkerError> {
    let mut request = responses_request_from_chat(input, model)?;
    request["input"][0]["content"][0]["text"] = json!(VIDEO_CAPABILITY_PROMPT);
    request["metadata"]["finite_capability"] = json!(Capability::Video.name());
    request["metadata"]["finite_prompt_version"] = json!(VIDEO_PROMPT_VERSION);
    Ok(request)
}

fn extract_chat_completion_text(value: &Value) -> Option<String> {
    value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn media_decode_failure(capability: Capability) -> WorkerError {
    WorkerError::bad_request_with_code(
        format!("{} bytes could not be decoded", capability.name()),
        "media_decode_failed",
    )
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
                        "image_url": { "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAQAAAAEAQMAAACTPww9AAAAA1BMVEX/AAAZ4gk3AAAAC0lEQVQI12NggAAAAAgAAS8g3TEAAAAASUVORK5CYII=" }
                    }),
                ]),
            }],
            temperature: Some(json!(0)),
            max_tokens: Some(32),
            max_completion_tokens: None,
            stream: Some(false),
            finite_specialization: None,
        };
        let passed = analyze_image(&state, &request, &generated_request_id())
            .await
            .ok()
            .and_then(|value| {
                value["specialization_result"]["text"]
                    .as_str()
                    .map(|text| text.trim().eq_ignore_ascii_case("red"))
            })
            .unwrap_or(false);
        state.metrics.observe_canary(
            Capability::Image,
            passed,
            started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        );
    }
}

async fn run_audio_canary_loop(state: Arc<WorkerState>, interval_seconds: u64) {
    let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
    loop {
        interval.tick().await;
        let started = Instant::now();
        let request = audio_canary_request();
        let passed = analyze_audio(&state, &request, &generated_request_id())
            .await
            .ok()
            .and_then(|value| {
                value["specialization_result"]["text"]
                    .as_str()
                    .map(|text| text.to_ascii_lowercase().contains("tone"))
            })
            .unwrap_or(false);
        state.metrics.observe_canary(
            Capability::Audio,
            passed,
            started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        );
    }
}

async fn run_video_canary_loop(state: Arc<WorkerState>, interval_seconds: u64) {
    let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
    loop {
        interval.tick().await;
        let started = Instant::now();
        let passed = match video_canary_request(&state).await {
            Ok(request) => analyze_video(&state, &request, &generated_request_id())
                .await
                .ok()
                .and_then(|value| {
                    value["specialization_result"]["text"]
                        .as_str()
                        .map(video_canary_passed)
                })
                .unwrap_or(false),
            Err(_) => false,
        };
        state.metrics.observe_canary(
            Capability::Video,
            passed,
            started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        );
    }
}

fn audio_canary_request() -> ChatCompletionRequest {
    let sample_rate = 16_000_u32;
    let sample_count = sample_rate / 2;
    let mut pcm = Vec::with_capacity(sample_count as usize * 2);
    for sample in 0..sample_count {
        let phase = sample as f32 * 440.0 * std::f32::consts::TAU / sample_rate as f32;
        pcm.extend_from_slice(&((phase.sin() * 8_000.0) as i16).to_le_bytes());
    }
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36_u32 + pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(&pcm);
    let mut request = request_with_media_parts(vec![
        json!({
            "type": "text",
            "text": "Describe the sound in this short audio clip. Mention whether you hear a tone."
        }),
        json!({
            "type": "input_audio",
            "input_audio": {
                "data": base64::engine::general_purpose::STANDARD.encode(wav),
                "format": "wav"
            }
        }),
    ]);
    request.max_tokens = Some(96);
    request
}

async fn video_canary_request(state: &WorkerState) -> Result<ChatCompletionRequest, WorkerError> {
    let file = tempfile::Builder::new()
        .prefix("finite-video-canary-")
        .suffix(".mp4")
        .tempfile_in(&state.config.media_temp_dir)
        .map_err(|_| WorkerError::unavailable("video canary workspace is unavailable"))?;
    let path = file.path().to_owned();
    let status = Command::new(&state.config.ffmpeg_path)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=64x64:d=1:r=4",
            "-f",
            "lavfi",
            "-i",
            "color=c=green:s=64x64:d=1:r=4",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=64x64:d=1:r=4",
        ])
        .args([
            "-filter_complex",
            "[0:v][1:v][2:v]concat=n=3:v=1:a=0,format=yuv420p[v]",
            "-map",
            "[v]",
            "-an",
            "-c:v",
            "mpeg4",
        ])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .map_err(|_| WorkerError::unavailable("video canary generation failed"))?;
    if !status.success() {
        return Err(WorkerError::unavailable("video canary generation failed"));
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| WorkerError::unavailable("video canary could not be read"))?;
    Ok(request_with_media_parts(vec![
        json!({
            "type": "text",
            "text": "Name the three full-frame colors in chronological order. Reply with three uppercase color words separated by THEN."
        }),
        json!({
            "type": "video_url",
            "video_url": { "url": format!("data:video/mp4;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)) }
        }),
    ]))
}

fn video_canary_passed(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    let Some(red) = normalized.find("red") else {
        return false;
    };
    let Some(green) = normalized[red + 3..].find("green") else {
        return false;
    };
    let green = red + 3 + green;
    normalized[green + 5..].contains("blue")
}

fn request_with_media_parts(parts: Vec<Value>) -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: None,
        messages: vec![ChatMessage {
            role: "user".to_owned(),
            content: ChatContent::Parts(parts),
        }],
        temperature: Some(json!(0)),
        max_tokens: Some(32),
        max_completion_tokens: None,
        stream: Some(false),
        finite_specialization: None,
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
    request_id: Option<String>,
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
            request_id: None,
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

    fn duration_exceeded(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            message,
            "invalid_request_error",
            "media_duration_exceeded",
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

    fn capacity(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            message,
            "server_error",
            "capacity_exceeded",
        )
    }
}

impl IntoResponse for WorkerError {
    fn into_response(self) -> Response {
        let request_id = self.request_id.unwrap_or_else(generated_request_id);
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
                "request_identifier": bounded_request_identifier(&request_id),
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
    fn classifies_one_primary_media_and_rejects_mixed_media() {
        let audio = request_with_media_parts(vec![
            json!({ "type": "text", "text": "What is the speaker implying?" }),
            json!({
                "type": "input_audio",
                "input_audio": { "data": "UklGRg==", "format": "wav" }
            }),
        ]);
        assert_eq!(classify_request(&audio).unwrap(), Capability::Audio);

        let video = request_with_media_parts(vec![json!({
            "type": "video_url",
            "video_url": { "url": "data:video/mp4;base64,AAAA" }
        })]);
        assert_eq!(classify_request(&video).unwrap(), Capability::Video);

        let mixed = request_with_media_parts(vec![
            json!({
                "type": "image_url",
                "image_url": { "url": "data:image/png;base64,AAAA" }
            }),
            json!({
                "type": "input_audio",
                "input_audio": { "data": "UklGRg==", "format": "wav" }
            }),
        ]);
        let error = classify_request(&mixed).unwrap_err();
        assert_eq!(error.code, "mixed_media_requires_decomposition");
    }

    #[test]
    fn audio_request_uses_canonical_chat_completions_input_audio() {
        let request = request_with_media_parts(vec![
            json!({ "type": "text", "text": "Interpret the emotional tone." }),
            json!({
                "type": "input_audio",
                "input_audio": { "data": "UklGRg==", "format": "wav" }
            }),
        ]);

        let upstream = chat_request_for_audio(&request, "aeon-test").unwrap();

        assert_eq!(upstream["model"], "aeon-test");
        assert_eq!(upstream["stream"], false);
        assert_eq!(upstream["messages"][0]["role"], "developer");
        assert_eq!(
            upstream["messages"][1]["content"][1],
            json!({
                "type": "input_audio",
                "input_audio": { "data": "UklGRg==", "format": "wav" }
            })
        );
    }

    #[test]
    fn audio_canary_uses_descriptive_semantic_prompt_with_output_headroom() {
        let request = audio_canary_request();
        let ChatContent::Parts(parts) = &request.messages[0].content else {
            panic!("audio canary must use multipart content");
        };

        assert_eq!(
            parts[0]["text"],
            "Describe the sound in this short audio clip. Mention whether you hear a tone."
        );
        assert_eq!(request.max_tokens, Some(96));
    }

    #[tokio::test]
    async fn audio_round_trip_normalizes_wav_and_calls_chat_completions() {
        if !media_tools_available().await {
            return;
        }
        let captured = Arc::new(Mutex::new(None::<(HeaderMap, Value)>));
        let upstream_url = spawn_spark_mock(
            captured.clone(),
            StatusCode::OK,
            json!({
                "id": "chatcmpl_audio",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "A steady tone." },
                    "finish_reason": "stop"
                }]
            }),
        )
        .await;
        let worker = spawn_worker(test_config(&upstream_url)).await;
        let response = reqwest::Client::new()
            .post(format!("{worker}/v1/chat/completions"))
            .bearer_auth("worker-secret")
            .json(&audio_canary_request())
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["specialization_result"]["capability"], "audio");
        assert_eq!(body["specialization_result"]["text"], "A steady tone.");
        let request = captured.lock().unwrap().clone().unwrap().1;
        assert_eq!(request["model"], "qwopus-test");
        assert_eq!(request["messages"][0]["content"], AUDIO_CAPABILITY_PROMPT);
        let normalized = request["messages"][1]["content"][1]["input_audio"]["data"]
            .as_str()
            .unwrap();
        let wav = base64::engine::general_purpose::STANDARD
            .decode(normalized)
            .unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[tokio::test]
    async fn corrupt_audio_and_video_return_shared_typed_decode_errors() {
        if !media_tools_available().await {
            return;
        }
        let state = test_state(test_config("http://127.0.0.1:9"));
        let audio = request_with_media_parts(vec![json!({
            "type": "input_audio",
            "input_audio": { "data": "bm90LW1lZGlh", "format": "wav" }
        })]);
        let video = request_with_media_parts(vec![json!({
            "type": "video_url",
            "video_url": { "url": "data:video/mp4;base64,bm90LW1lZGlh" }
        })]);

        let audio_error = analyze_audio(&state, &audio, "req-audio-corrupt")
            .await
            .unwrap_err();
        let video_error = analyze_video(&state, &video, "req-video-corrupt")
            .await
            .unwrap_err();

        assert_eq!(audio_error.status, StatusCode::BAD_REQUEST);
        assert_eq!(audio_error.code, "media_decode_failed");
        assert_eq!(video_error.status, StatusCode::BAD_REQUEST);
        assert_eq!(video_error.code, "media_decode_failed");
    }

    #[tokio::test]
    async fn exhausted_media_capacity_returns_typed_retryable_boundary() {
        let mut config = test_config("http://127.0.0.1:9");
        config.media_concurrency = 1;
        let state = test_state(config);
        let _permit = state.media_slots.clone().acquire_owned().await.unwrap();

        let error = normalize_audio_input(&state, &audio_canary_request(), test_limits())
            .await
            .unwrap_err();

        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.code, "capacity_exceeded");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn decoded_duration_limits_use_the_shared_duration_error_code() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let probe = directory.path().join("long-ffprobe.sh");
        std::fs::write(
            &probe,
            "#!/bin/sh\ncase \"$*\" in *finite-audio-*) codec=audio ;; *) codec=video ;; esac\nprintf '{\"format\":{\"duration\":\"30\"},\"streams\":[{\"codec_type\":\"%s\"}]}' \"$codec\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut config = test_config("http://127.0.0.1:9");
        config.ffprobe_path = probe;
        config.max_audio_duration_seconds = 1;
        config.max_video_duration_seconds = 1;
        let state = test_state(config);

        let audio_error = normalize_audio_bytes(&state, vec![0; 16], 1024)
            .await
            .unwrap_err();
        let video_error = sample_video_frames(&state, vec![0; 16]).await.unwrap_err();

        assert_eq!(audio_error.status, StatusCode::BAD_REQUEST);
        assert_eq!(audio_error.code, "media_duration_exceeded");
        assert_eq!(video_error.status, StatusCode::BAD_REQUEST);
        assert_eq!(video_error.code, "media_duration_exceeded");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn audio_deadline_kills_in_flight_media_probe() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("probe.pid");
        let probe = directory.path().join("blocking-ffprobe.sh");
        std::fs::write(
            &probe,
            format!(
                "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut config = test_config("http://127.0.0.1:9");
        config.ffprobe_path = probe;
        config.request_deadline_seconds = 2;
        let worker = spawn_worker(config).await;

        let response = reqwest::Client::new()
            .post(format!("{worker}/v1/chat/completions"))
            .bearer_auth("worker-secret")
            .json(&audio_canary_request())
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();

        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(body["error"]["code"], "request_timeout");
        let pid = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Ok(pid) = tokio::fs::read_to_string(&pid_file).await {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("ffprobe fixture never started");
        let mut stopped = false;
        for _ in 0..20 {
            if !Command::new("kill")
                .args(["-0", pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .is_ok_and(|status| status.success())
            {
                stopped = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(stopped, "deadline left ffprobe process {pid} running");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn caller_cancellation_kills_in_flight_media_probe() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("probe.pid");
        let probe = directory.path().join("blocking-ffprobe.sh");
        std::fs::write(
            &probe,
            format!(
                "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut config = test_config("http://127.0.0.1:9");
        config.ffprobe_path = probe;
        let worker = spawn_worker(config).await;
        let request = audio_canary_request();
        let operation = tokio::spawn(async move {
            reqwest::Client::new()
                .post(format!("{worker}/v1/chat/completions"))
                .bearer_auth("worker-secret")
                .json(&request)
                .send()
                .await
        });

        for _ in 0..100 {
            if pid_file.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let pid = tokio::fs::read_to_string(&pid_file).await.unwrap();
        operation.abort();
        let _ = operation.await;

        let mut stopped = false;
        for _ in 0..40 {
            if !Command::new("kill")
                .args(["-0", pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .is_ok_and(|status| status.success())
            {
                stopped = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            stopped,
            "caller cancellation left ffprobe process {pid} running"
        );
    }

    #[test]
    fn uniform_video_sampling_covers_short_and_long_clips() {
        assert_eq!(uniform_video_timestamps(3.2, 16), vec![0.0, 1.0, 2.0, 3.0]);

        let long = uniform_video_timestamps(60.0, 16);
        assert_eq!(long.len(), 16);
        assert_eq!(long[0], 0.0);
        assert!((long[15] - 59.999).abs() < 0.01);
        assert!(long.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[tokio::test]
    async fn video_round_trip_samples_frames_in_chronological_responses_input() {
        if !media_tools_available().await {
            return;
        }
        let captured = Arc::new(Mutex::new(None::<(HeaderMap, Value)>));
        let upstream_url = spawn_spark_mock(
            captured.clone(),
            StatusCode::OK,
            json!({
                "id": "resp_video",
                "output_text": "The frames are red."
            }),
        )
        .await;
        let config = test_config(&upstream_url);
        let request = video_canary_request(&test_state(config.clone()))
            .await
            .unwrap();
        let worker = spawn_worker(config).await;
        let response = reqwest::Client::new()
            .post(format!("{worker}/v1/chat/completions"))
            .bearer_auth("worker-secret")
            .json(&request)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["specialization_result"]["capability"], "video");
        assert_eq!(body["specialization_result"]["text"], "The frames are red.");
        let request = captured.lock().unwrap().clone().unwrap().1;
        assert_eq!(request["metadata"]["finite_capability"], "video");
        assert_eq!(
            request["input"][0]["content"][0]["text"],
            VIDEO_CAPABILITY_PROMPT
        );
        let content = request["input"][1]["content"].as_array().unwrap();
        let frame_markers = content
            .iter()
            .filter_map(|part| {
                (part["type"] == "input_text")
                    .then(|| part["text"].as_str())
                    .flatten()
                    .filter(|text| text.starts_with("Frame at "))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            frame_markers,
            [
                "Frame at 0.000 seconds",
                "Frame at 1.000 seconds",
                "Frame at 2.000 seconds"
            ]
        );
        let images = content
            .iter()
            .filter(|part| part["type"] == "input_image")
            .collect::<Vec<_>>();
        assert_eq!(images.len(), frame_markers.len());
        assert!(
            images[0]["image_url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }

    #[test]
    fn capability_health_is_independent() {
        let metrics = CapabilityMetrics::default();
        metrics.observe_canary(Capability::Image, true, 100);
        metrics.observe_canary(Capability::Audio, false, 200);
        metrics.observe_canary(Capability::Video, true, 300);

        let rendered = metrics.render("model-test");
        assert!(rendered.contains("capability=\"image\",health_state=\"healthy\""));
        assert!(rendered.contains("capability=\"audio\",health_state=\"degraded\""));
        assert!(rendered.contains("capability=\"video\",health_state=\"healthy\""));
    }

    #[test]
    fn capability_health_becomes_stale_after_fifteen_minutes() {
        let observation = CanaryObservation {
            health_state: "healthy",
            consecutive_failures: 0,
            last_completed_timestamp_seconds: current_unix_timestamp().saturating_sub(901),
            last_latency_milliseconds: 100,
        };

        assert_eq!(effective_canary_health(&observation), "stale");
    }

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
        let request_id = response["specialization_result"]["request_id"]
            .as_str()
            .unwrap();
        assert!(request_id.starts_with("req_specialization_"));
        assert_eq!(
            response["specialization_result"]["upstream_request_id"],
            "resp_ok"
        );
        assert!(response["specialization_result"]["duration_ms"].is_u64());
        let captured = captured.lock().unwrap().clone().unwrap();
        assert_eq!(
            captured.0.get("authorization").unwrap().to_str().unwrap(),
            "Bearer spark-secret"
        );
        assert_eq!(
            captured.0.get("x-request-id").unwrap().to_str().unwrap(),
            request_id
        );
        assert_eq!(captured.1["model"], "qwopus-test");
        assert_eq!(captured.1["input"][0]["role"], "developer");
        assert_eq!(
            captured.1["metadata"]["finite_prompt_version"],
            "aeon-image-analysis-v1"
        );
        assert_eq!(captured.1["input"][1]["content"][1]["type"], "input_image");
    }

    #[tokio::test]
    async fn malformed_success_is_rejected_for_every_capability() {
        let image = request_with_media_parts(vec![
            json!({ "type": "text", "text": "Describe" }),
            json!({
                "type": "image_url",
                "image_url": { "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=" }
            }),
        ]);
        let video_config = test_config("http://127.0.0.1:9");
        let video = video_canary_request(&test_state(video_config))
            .await
            .unwrap();
        let requests = [
            (Capability::Image, image),
            (Capability::Audio, audio_canary_request()),
            (Capability::Video, video),
        ];

        for (capability, request) in requests {
            let upstream_url = spawn_spark_mock(
                Arc::new(Mutex::new(None::<(HeaderMap, Value)>)),
                StatusCode::OK,
                json!({}),
            )
            .await;
            let worker = spawn_worker(test_config(&upstream_url)).await;
            let response = reqwest::Client::new()
                .post(format!("{worker}/v1/chat/completions"))
                .bearer_auth("worker-secret")
                .json(&request)
                .send()
                .await
                .unwrap();
            let status = response.status();
            let body: Value = response.json().await.unwrap();

            assert_eq!(
                status,
                StatusCode::BAD_GATEWAY,
                "{}: {body}",
                capability.name()
            );
            assert_eq!(
                body["error"]["code"],
                "upstream_error",
                "{}: {body}",
                capability.name()
            );
        }
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
            .header("x-request-id", "req-malformed-image")
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
        assert_eq!(body["request_id"], "req-malformed-image");
        assert_eq!(
            body["request_identifier"],
            bounded_request_identifier("req-malformed-image")
        );
        let metrics = reqwest::get(format!("{worker}/metrics"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(metrics.contains(
            "finite_specialization_last_failure_duration_milliseconds{capability=\"image\",error_class=\"media_decode_failed\",model_alias=\"qwopus-test\",state_version=\"aeon-image-analysis-v1\",surface=\"tyk-public-frontdoor\"}"
        ));
        assert!(metrics.contains(&format!(
            "finite_specialization_last_failure_request_identifier{{capability=\"image\",error_class=\"media_decode_failed\",model_alias=\"qwopus-test\",state_version=\"aeon-image-analysis-v1\",surface=\"tyk-public-frontdoor\"}} {}",
            bounded_request_identifier("req-malformed-image")
        )));
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

        for capability in [Capability::Audio, Capability::Video] {
            let error = fetch_binary_media(
                &state,
                "https://127.0.0.1/media",
                capability,
                test_limits().max_download_bytes,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, "media_fetch_failed", "{}", capability.name());
            assert!(
                error.message.contains("non-public"),
                "{}",
                capability.name()
            );
        }
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

        for capability in [Capability::Audio, Capability::Video] {
            let error = fetch_binary_media(
                &state,
                &format!("http://{addr}/attachment.png"),
                capability,
                test_limits().max_download_bytes,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, "media_fetch_failed", "{}", capability.name());
        }
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

        metrics.observe_canary(Capability::Image, false, 1200);
        assert!(
            metrics
                .render("model-test")
                .contains("health_state=\"degraded\"")
        );
        metrics.observe_canary(Capability::Image, false, 1300);
        metrics.observe_canary(Capability::Image, false, 1400);
        assert!(
            metrics
                .render("model-test")
                .contains("health_state=\"unavailable\"")
        );
        metrics.observe_canary(Capability::Image, true, 900);

        let rendered = metrics.render("model-test");
        assert!(rendered.contains("health_state=\"healthy\""));
        assert!(rendered.contains(
            "finite_specialization_canary_latency_milliseconds{capability=\"image\",model_alias=\"model-test\",surface=\"tyk-public-frontdoor\"} 900"
        ));
    }

    #[test]
    fn video_canary_requires_the_complete_chronological_marker() {
        assert!(video_canary_passed("RED then GREEN then BLUE"));
        assert!(video_canary_passed(
            "The sequence is red, green, and then blue."
        ));
        assert!(!video_canary_passed("blue, green, red"));
        assert!(!video_canary_passed("red and blue"));
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
            audio_canary_interval_seconds: None,
            video_canary_interval_seconds: None,
            max_audio_duration_seconds: DEFAULT_MAX_AUDIO_DURATION_SECONDS,
            max_video_duration_seconds: DEFAULT_MAX_VIDEO_DURATION_SECONDS,
            max_video_frames: DEFAULT_MAX_VIDEO_FRAMES,
            ffmpeg_path: PathBuf::from("ffmpeg"),
            ffprobe_path: PathBuf::from("ffprobe"),
            media_temp_dir: std::env::temp_dir(),
            media_concurrency: 2,
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
        let media_concurrency = config.media_concurrency;
        WorkerState {
            config,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            metrics: CapabilityMetrics::default(),
            media_slots: Arc::new(Semaphore::new(media_concurrency)),
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
        let responses_captured = captured.clone();
        let responses_body = response.clone();
        let app = Router::new()
            .route(
                "/responses",
                post(move |headers: HeaderMap, body: Bytes| {
                    let captured = responses_captured.clone();
                    let response = responses_body.clone();
                    async move {
                        let payload = serde_json::from_slice::<Value>(&body).unwrap();
                        *captured.lock().unwrap() = Some((headers, payload));
                        (status, Json(response))
                    }
                }),
            )
            .route(
                "/chat/completions",
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

    async fn media_tools_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|status| status.success())
            && Command::new("ffprobe")
                .arg("-version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .is_ok_and(|status| status.success())
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
