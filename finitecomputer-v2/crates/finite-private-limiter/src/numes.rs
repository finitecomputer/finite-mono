//! Optional marketplace admission, independent of Finite subscription accounting.
//! Tinfoil's shim owns EHBP; this module only sees in-enclave plaintext.
use super::*;
use hmac::{Hmac, Mac};
use http_body::Frame;
use http_body_util::StreamBody;
use sha2::Sha256;
use std::sync::Mutex;
use tokio::sync::{mpsc, oneshot};

mod usage;
use usage::{StreamUsage, Usage};

const USAGE_HEADER: &str = "x-tinfoil-usage-metrics";
const MAX_CONCURRENT: usize = 1024;

#[derive(Clone)]
pub struct NumesConfig {
    pub api_key: String,
    pub control_api_key: String,
    /// Initial cap after process start. Zero pauses marketplace admission.
    pub max_concurrent: usize,
    pub max_output_tokens: u64,
    pub request_timeout: Duration,
}

impl std::fmt::Debug for NumesConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NumesConfig")
            .field("max_concurrent", &self.max_concurrent)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("request_timeout", &self.request_timeout)
            .finish_non_exhaustive()
    }
}

impl NumesConfig {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let vars = [
            "NUMES_API_KEY",
            "NUMES_CONTROL_API_KEY",
            "NUMES_MAX_CONCURRENT",
            "NUMES_MAX_OUTPUT_TOKENS",
            "NUMES_REQUEST_TIMEOUT_SECS",
        ];
        if vars.iter().all(|name| std::env::var_os(name).is_none()) {
            return Ok(None);
        }
        let number = |name: &str, default: u64| -> anyhow::Result<u64> {
            match std::env::var(name) {
                Ok(value) => value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("{name} must be an unsigned integer")),
                Err(std::env::VarError::NotPresent) => Ok(default),
                Err(_) => anyhow::bail!("{name} must be valid UTF-8"),
            }
        };
        Ok(Some(Self {
            api_key: std::env::var("NUMES_API_KEY").unwrap_or_default(),
            control_api_key: std::env::var("NUMES_CONTROL_API_KEY").unwrap_or_default(),
            max_concurrent: usize::try_from(number("NUMES_MAX_CONCURRENT", 0)?)?,
            max_output_tokens: number("NUMES_MAX_OUTPUT_TOKENS", 4096)?,
            request_timeout: Duration::from_secs(number("NUMES_REQUEST_TIMEOUT_SECS", 120)?),
        }))
    }

    pub(super) fn validate(&self) -> Result<(), LimiterConfigError> {
        for (name, key) in [
            ("NUMES_API_KEY", &self.api_key),
            ("NUMES_CONTROL_API_KEY", &self.control_api_key),
        ] {
            if key.len() < 32 || !key.bytes().all(|b| b.is_ascii_graphic()) {
                return Err(LimiterConfigError::InvalidNumesConfig(format!(
                    "{name} must contain at least 32 printable non-space ASCII characters"
                )));
            }
        }
        if self.api_key == self.control_api_key
            || self.max_concurrent > MAX_CONCURRENT
            || self.max_output_tokens == 0
            || self.max_output_tokens > 1_000_000
            || self.request_timeout.is_zero()
            || self.request_timeout > Duration::from_secs(3600)
        {
            return Err(LimiterConfigError::InvalidNumesConfig(
                "invalid Numes credentials, capacity, output limit or deadline".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Admission {
    capacity: Arc<Mutex<Capacity>>,
    config: NumesConfig,
}

#[derive(Serialize)]
struct Capacity {
    max_concurrent: usize,
    in_flight: usize,
}

struct Slot(Arc<Mutex<Capacity>>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0
            .lock()
            .expect("Numes capacity mutex poisoned")
            .in_flight -= 1;
    }
}

// HMAC verification compares fixed-size tags in constant time; neither key is logged.
fn matches_key(expected: &str, presented: &str) -> bool {
    let mut expected_mac = Hmac::<Sha256>::new_from_slice(b"finite-numes-auth-v1").unwrap();
    expected_mac.update(expected.as_bytes());
    let tag = expected_mac.finalize().into_bytes();
    let mut candidate = Hmac::<Sha256>::new_from_slice(b"finite-numes-auth-v1").unwrap();
    candidate.update(presented.as_bytes());
    candidate.verify_slice(&tag).is_ok()
}

impl Admission {
    pub(super) fn new(config: &NumesConfig) -> Self {
        Self {
            config: config.clone(),
            capacity: Arc::new(Mutex::new(Capacity {
                max_concurrent: config.max_concurrent,
                in_flight: 0,
            })),
        }
    }

    pub(super) fn authenticates(&self, key: &str) -> bool {
        matches_key(&self.config.api_key, key)
    }

    fn acquire(&self) -> Option<Slot> {
        let mut capacity = self.capacity.lock().expect("Numes capacity mutex poisoned");
        if capacity.in_flight >= capacity.max_concurrent {
            return None;
        }
        capacity.in_flight += 1;
        Some(Slot(self.capacity.clone()))
    }

    pub(super) fn metrics(&self) -> String {
        let capacity = self.capacity.lock().expect("Numes capacity mutex poisoned");
        format!(
            "finite_private_numes_max_concurrent {}\nfinite_private_numes_in_flight {}\n",
            capacity.max_concurrent, capacity.in_flight
        )
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapacityUpdate {
    max_concurrent: usize,
}

pub(super) async fn capacity(State(state): State<AppState>, headers: HeaderMap) -> Response {
    control(&state, &headers, None)
}

pub(super) async fn set_capacity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<CapacityUpdate>,
) -> Response {
    control(&state, &headers, Some(update.max_concurrent))
}

fn control(state: &AppState, headers: &HeaderMap, update: Option<usize>) -> Response {
    let Some(admission) = &state.numes else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !bearer_token(headers)
        .is_some_and(|key| matches_key(&admission.config.control_api_key, &key))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if update.is_some_and(|n| n > MAX_CONCURRENT) {
        return error(
            StatusCode::BAD_REQUEST,
            "numes_invalid_capacity",
            "Capacity must be between 0 and 1024.",
        );
    }
    let mut capacity = admission
        .capacity
        .lock()
        .expect("Numes capacity mutex poisoned");
    if let Some(limit) = update {
        capacity.max_concurrent = limit;
    }
    Json(
        json!({"max_concurrent": capacity.max_concurrent, "in_flight": capacity.in_flight,
        "initial_max_concurrent": admission.config.max_concurrent}),
    )
    .into_response()
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    openai_error(status, message, "numes_error", code)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

pub(super) async fn serve(
    state: AppState,
    admission: Admission,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if uri.path() != "/v1/chat/completions" {
        return error(
            StatusCode::BAD_REQUEST,
            "numes_unsupported_endpoint",
            "Numes supports chat completions only.",
        );
    }
    let model = &state.config.default_model;
    let Ok(mut value) = serde_json::from_slice::<Value>(&body) else {
        return error(
            StatusCode::BAD_REQUEST,
            "numes_invalid_request",
            "A JSON chat request is required.",
        );
    };
    if value.get("model").and_then(Value::as_str) != Some(model)
        || header(&headers, "x-tinfoil-model") != Some(model)
        || header(&headers, "x-tinfoil-request-usage-metrics") != Some("true")
    {
        return error(
            StatusCode::BAD_REQUEST,
            "numes_invalid_model",
            "The body and X-Tinfoil-Model must name the canonical model; usage metrics must be requested.",
        );
    }
    // Tinfoil relays need not supply an application-specific request ID.
    // Generate correlation metadata locally without changing the usage format.
    let request_id = if headers.contains_key("x-request-id") {
        match header(&headers, "x-request-id").filter(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
        }) {
            Some(id) => id.to_owned(),
            None => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "numes_invalid_request_id",
                    "When supplied, X-Request-Id must be a bounded opaque identifier.",
                );
            }
        }
    } else {
        new_request_id()
    };
    let Some(object) = value.as_object_mut() else {
        unreachable!()
    };
    if !object.get("messages").is_some_and(Value::is_array)
        || object.get("stream").is_some_and(|v| !v.is_boolean())
        || object.get("n").is_some_and(|v| v.as_u64() != Some(1))
    {
        return error(
            StatusCode::BAD_REQUEST,
            "numes_invalid_request",
            "Messages, boolean stream and at most one completion are required.",
        );
    }
    // Bound every accepted request even when the encrypted body omits a limit.
    // A relay may impose a smaller limit in the visible Tinfoil header.
    let mut output_limit = admission.config.max_output_tokens;
    for name in ["max_tokens", "max_completion_tokens"] {
        if let Some(v) = object.get(name) {
            let Some(n) = v.as_u64().filter(|n| *n > 0 && *n <= output_limit) else {
                return error(
                    StatusCode::BAD_REQUEST,
                    "numes_invalid_output_limit",
                    "Output exceeds Finite's marketplace limit.",
                );
            };
            output_limit = n;
        }
    }
    if headers.contains_key("x-tinfoil-max-output-tokens") {
        let Some(n) = header(&headers, "x-tinfoil-max-output-tokens")
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|n| *n > 0 && *n <= admission.config.max_output_tokens)
        else {
            return error(
                StatusCode::BAD_REQUEST,
                "numes_invalid_output_limit",
                "Invalid X-Tinfoil-Max-Output-Tokens.",
            );
        };
        output_limit = output_limit.min(n);
    }
    if headers.contains_key("x-tinfoil-completion-count")
        && header(&headers, "x-tinfoil-completion-count") != Some("1")
    {
        return error(
            StatusCode::BAD_REQUEST,
            "numes_invalid_request",
            "Only one completion is supported.",
        );
    }
    object.remove("max_completion_tokens");
    object.insert("max_tokens".into(), json!(output_limit));
    object.insert("n".into(), json!(1));
    let streaming = object
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if streaming {
        object.insert("stream_options".into(), json!({"include_usage": true}));
    }
    apply_thinking_defaults(
        object,
        state.config.default_reasoning_effort.as_deref(),
        state.config.default_enable_thinking,
    );
    let Some(slot) = admission.acquire() else {
        let mut response = error(
            StatusCode::TOO_MANY_REQUESTS,
            "numes_at_capacity",
            "Finite's Numes capacity is unavailable.",
        );
        response
            .headers_mut()
            .insert("retry-after", HeaderValue::from_static("1"));
        return response;
    };
    // Own the slot in a detached worker BEFORE prefill. A dropped handler or
    // body must never release GPU capacity while generation is still running.
    let (reply_tx, reply_rx) = oneshot::channel();
    let body = Bytes::from(serde_json::to_vec(&value).expect("JSON value serialization"));
    tokio::spawn(async move {
        let _slot = slot;
        let deadline = admission.config.request_timeout;
        let work = generate(
            &state,
            uri,
            body,
            streaming,
            request_id,
            output_limit,
            reply_tx,
        );
        if timeout(deadline, work).await.is_err() {
            // Dropping the upstream cancels the request. Never fabricate usage.
            eprintln!("finite-private Numes request deadline exceeded");
        }
    });
    reply_rx.await.unwrap_or_else(|_| {
        error(
            StatusCode::GATEWAY_TIMEOUT,
            "numes_upstream_incomplete",
            "Numes generation did not complete.",
        )
    })
}

async fn generate(
    state: &AppState,
    uri: Uri,
    body: Bytes,
    streaming: bool,
    request_id: String,
    output_limit: u64,
    reply: oneshot::Sender<Response>,
) {
    let upstream = match call_upstream_response(state, &uri, body).await {
        Ok(upstream) if upstream.status().is_success() => upstream,
        _ => {
            let _ = reply.send(error(
                StatusCode::BAD_GATEWAY,
                "numes_upstream_error",
                "Marketplace inference failed.",
            ));
            return;
        }
    };
    let model = &state.config.default_model;
    if !streaming {
        let Ok(body) = upstream.bytes().await else {
            return;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&body) else {
            return;
        };
        let Some(usage) = Usage::from_value(&value, model, output_limit) else {
            let _ = reply.send(error(
                StatusCode::BAD_GATEWAY,
                "numes_missing_usage",
                "No valid final usage was reported.",
            ));
            return;
        };
        let response = Response::builder()
            .header("content-type", "application/json")
            .header("x-request-id", request_id)
            .header(USAGE_HEADER, usage.format(model))
            .body(Body::from(body))
            .unwrap();
        let _ = reply.send(response);
        return;
    }
    let (tx, mut rx) = mpsc::channel::<Result<Frame<Bytes>, std::io::Error>>(8);
    let frames = async_stream::stream! {
        while let Some(frame) = rx.recv().await {
            let terminal = match &frame {
                Ok(frame) => frame.is_trailers(),
                Err(_) => true,
            };
            yield frame;
            if terminal {
                return;
            }
        }
        // A deadline drops the worker and sender. Expose that as an incomplete
        // HTTP body, not a normal EOF that an encrypting proxy could finalize.
        yield Err(std::io::Error::other("Numes stream ended without final usage"));
    };
    let response = Response::builder()
        .header("content-type", "text/event-stream")
        .header("x-request-id", request_id)
        .header("trailer", USAGE_HEADER)
        .body(Body::new(StreamBody::new(frames)))
        .unwrap();
    let _ = reply.send(response);
    let mut stream = upstream.bytes_stream();
    let mut usage = StreamUsage::new(model, output_limit);
    while let Some(item) = stream.next().await {
        let Ok(chunk) = item else {
            let _ = tx
                .send(Err(std::io::Error::other("Numes upstream stream failed")))
                .await;
            return;
        };
        usage.push(&chunk);
        // Closed receiver means customer delivery ended. Keep draining upstream
        // to obtain final counts, holding the slot until completion/deadline.
        let _ = tx.send(Ok(Frame::data(chunk))).await;
        // Drain through HTTP EOF, even after [DONE]. A later transport failure
        // or contradictory event must prevent issuing a successful receipt.
    }
    if let Some(usage) = usage.finish() {
        let mut trailers = HeaderMap::new();
        trailers.insert(
            USAGE_HEADER,
            HeaderValue::from_str(&usage.format(model)).unwrap(),
        );
        let _ = tx.send(Ok(Frame::trailers(trailers))).await;
    } else {
        let _ = tx
            .send(Err(std::io::Error::other(
                "Numes stream ended without valid final usage",
            )))
            .await;
    }
}

#[cfg(test)]
mod tests;
