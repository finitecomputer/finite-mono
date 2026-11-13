use super::*;
use axum::http::Request;
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

const KEY: &str = "synthetic-numes-source-key-000000000000";
const ADMIN: &str = "synthetic-finite-control-key-0000000000";
const MODEL: &str = "glm-5-3-flash";

#[derive(Clone)]
struct Engine {
    calls: Arc<AtomicUsize>,
    release: Arc<Semaphore>,
    prefill: bool,
    final_usage: bool,
    fail: bool,
    after_done: Option<Arc<Semaphore>>,
    fail_after_done: bool,
    last_request: Arc<Mutex<Option<Value>>>,
}

impl Engine {
    fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            release: Arc::new(Semaphore::new(0)),
            prefill: false,
            final_usage: true,
            fail: false,
            after_done: None,
            fail_after_done: false,
            last_request: Arc::new(Mutex::new(None)),
        }
    }
}

async fn engine(
    State(engine): State<Engine>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_eq!(headers["authorization"], "Bearer synthetic-engine-key");
    engine.calls.fetch_add(1, Ordering::SeqCst);
    *engine.last_request.lock().unwrap() = Some(body.clone());
    if engine.fail {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    if engine.prefill {
        engine.release.acquire().await.unwrap().forget();
    }
    if body["stream"] != true {
        let usage = engine
            .final_usage
            .then(|| json!({"prompt_tokens": 10, "completion_tokens": 2, "total_tokens":12}));
        return Json(
            json!({"model":MODEL, "choices":[{"message":{"content":"ok"}}], "usage":usage}),
        )
        .into_response();
    }
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["max_tokens"], 64);
    let stream = async_stream::stream! {
        yield Ok::<_, std::io::Error>(Bytes::from(format!("data: {{\"model\":\"{MODEL}\",\"choices\":[{{\"delta\":{{\"content\":\"ok\"}}}}]}}\n\n")));
        engine.release.acquire().await.unwrap().forget();
        if engine.final_usage {
            yield Ok(Bytes::from(format!("data: {{\"model\":\"{MODEL}\",\"choices\":[],\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12}}}}\n\n")));
        }
        yield Ok(Bytes::from_static(b"data: [DONE]\n\n"));
        if let Some(gate) = engine.after_done {
            gate.acquire().await.unwrap().forget();
        }
        if engine.fail_after_done {
            yield Err(std::io::Error::other("synthetic failure after DONE"));
        }
    };
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
        .into_response()
}

struct Harness {
    app: Router,
    engine: Engine,
    core_calls: Arc<AtomicUsize>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Harness {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn server(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }),
    )
}

fn config(core: String, upstream: String) -> LimiterConfig {
    let mut config = LimiterConfig::new(
        core,
        "synthetic-core-service-key".into(),
        upstream,
        "synthetic-engine-key".into(),
        "https://finite.example/dashboard".into(),
    );
    config.upstream_model = Some(MODEL.into());
    config.numes = Some(NumesConfig {
        api_key: KEY.into(),
        control_api_key: ADMIN.into(),
        max_concurrent: 1,
        max_output_tokens: 64,
        request_timeout: Duration::from_secs(5),
    });
    config
}

async fn harness(engine_state: Engine, deadline: Duration) -> Harness {
    let core_calls = Arc::new(AtomicUsize::new(0));
    let counter = core_calls.clone();
    let core = Router::new().fallback(move |headers: HeaderMap| {
        counter.fetch_add(1, Ordering::SeqCst);
        async move {
            assert_eq!(
                headers["authorization"],
                "Bearer synthetic-core-service-key"
            );
            // Real subscription authentication still owns all other credentials.
            Json(
                json!({"decision":"deny", "error":{"message":"synthetic denied",
                "type":"invalid_api_key", "code":"invalid_api_key"}}),
            )
        }
    });
    let (core, core_task) = server(core).await;
    let (upstream, engine_task) = server(
        Router::new()
            .route("/v1/chat/completions", post(engine))
            .with_state(engine_state.clone()),
    )
    .await;
    let mut cfg = config(core, upstream);
    cfg.numes.as_mut().unwrap().request_timeout = deadline;
    Harness {
        app: app(cfg).unwrap(),
        engine: engine_state,
        core_calls,
        tasks: vec![core_task, engine_task],
    }
}

fn request(key: &str, streaming: bool) -> Request<Body> {
    Request::post("/v1/chat/completions")
        .header("authorization", format!("Bearer {key}"))
        .header("content-type", "application/json")
        .header("x-tinfoil-model", MODEL)
        .header("x-request-id", "synthetic-request-1")
        .header("x-tinfoil-request-usage-metrics", "true")
        .body(Body::from(
            json!({"model":MODEL,"messages":[],"stream":streaming}).to_string(),
        ))
        .unwrap()
}

async fn capacity(app: &Router, key: &str, limit: Option<usize>) -> Response {
    let mut request = Request::builder()
        .uri("/admin/numes/capacity")
        .header("authorization", format!("Bearer {key}"));
    let body = if let Some(n) = limit {
        request = request
            .method("POST")
            .header("content-type", "application/json");
        Body::from(json!({"max_concurrent":n}).to_string())
    } else {
        Body::empty()
    };
    app.clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

async fn inflight(app: &Router) -> usize {
    let response = capacity(app, ADMIN, None).await;
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice::<Value>(&bytes).unwrap()["in_flight"]
        .as_u64()
        .unwrap() as usize
}

async fn wait_for(mut predicate: impl AsyncFnMut() -> bool) {
    timeout(Duration::from_secs(2), async {
        while !predicate().await {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("synthetic condition timed out");
}

#[tokio::test]
async fn marketplace_returns_raw_usage_without_touching_core() {
    let h = harness(Engine::new(), Duration::from_secs(5)).await;
    let response = h.app.clone().oneshot(request(KEY, false)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[USAGE_HEADER],
        "prompt=10,completion=2,total=12,model=glm-5-3-flash"
    );
    assert_eq!(response.headers()["x-request-id"], "synthetic-request-1");
    assert_eq!(h.core_calls.load(Ordering::SeqCst), 0);
    assert_eq!(inflight(&h.app).await, 0);
}

#[tokio::test]
async fn tinfoil_relay_without_request_ids_gets_generated_ids_and_unchanged_usage() {
    let h = harness(Engine::new(), Duration::from_secs(5)).await;
    let mut ids = Vec::new();
    for streaming in [false, true] {
        let mut request = request(KEY, streaming);
        request.headers_mut().remove("x-request-id");
        let response = h.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(id.starts_with("fp_req_"));
        ids.push(id);
        let expected = "prompt=10,completion=2,total=12,model=glm-5-3-flash";
        if streaming {
            h.engine.release.add_permits(1);
            let collected = response.into_body().collect().await.unwrap();
            assert_eq!(collected.trailers().unwrap()[USAGE_HEADER], expected);
        } else {
            assert_eq!(response.headers()[USAGE_HEADER], expected);
        }
        wait_for(async || inflight(&h.app).await == 0).await;
    }
    assert_ne!(ids[0], ids[1]);
    assert_eq!(h.core_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn invalid_or_ambiguous_supplied_request_ids_fail_before_dispatch() {
    let h = harness(Engine::new(), Duration::from_secs(5)).await;
    for value in ["".to_owned(), "contains spaces".to_owned(), "a".repeat(129)] {
        let mut request = request(KEY, false);
        request
            .headers_mut()
            .insert("x-request-id", HeaderValue::from_str(&value).unwrap());
        assert_eq!(
            h.app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    let mut duplicate = request(KEY, false);
    duplicate
        .headers_mut()
        .append("x-request-id", HeaderValue::from_static("second-id"));
    assert_eq!(
        h.app.clone().oneshot(duplicate).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(h.engine.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.core_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn live_capacity_control_is_finite_only_and_drains_without_cancelling() {
    let h = harness(Engine::new(), Duration::from_secs(5)).await;
    let response = h.app.clone().oneshot(request(KEY, true)).await.unwrap();
    assert_eq!(inflight(&h.app).await, 1);
    for key in [KEY, "synthetic-finite-subscription-key", "wrong"] {
        assert_eq!(
            capacity(&h.app, key, Some(100)).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        capacity(&h.app, ADMIN, Some(0)).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(request(KEY, false))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(inflight(&h.app).await, 1);
    h.engine.release.add_permits(1);
    let collected = response.into_body().collect().await.unwrap();
    assert_eq!(
        collected.trailers().unwrap()[USAGE_HEADER],
        "prompt=10,completion=2,total=12,model=glm-5-3-flash"
    );
    wait_for(async || inflight(&h.app).await == 0).await;
    assert_eq!(
        h.app
            .clone()
            .oneshot(request(KEY, false))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        capacity(&h.app, ADMIN, Some(2)).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(request(KEY, false))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(h.core_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn disconnected_stream_holds_capacity_until_final_usage_is_collected() {
    let h = harness(Engine::new(), Duration::from_secs(5)).await;
    let response = h.app.clone().oneshot(request(KEY, true)).await.unwrap();
    drop(response);
    assert_eq!(
        h.app
            .clone()
            .oneshot(request(KEY, false))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(h.engine.calls.load(Ordering::SeqCst), 1);
    h.engine.release.add_permits(1);
    wait_for(async || inflight(&h.app).await == 0).await;
    assert_eq!(
        h.app
            .clone()
            .oneshot(request(KEY, false))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn raising_and_lowering_a_live_cap_is_atomic_across_inflight_requests() {
    let h = harness(Engine::new(), Duration::from_secs(5)).await;
    drop(h.app.clone().oneshot(request(KEY, true)).await.unwrap());
    capacity(&h.app, ADMIN, Some(2)).await;
    drop(h.app.clone().oneshot(request(KEY, true)).await.unwrap());
    assert_eq!(inflight(&h.app).await, 2);
    capacity(&h.app, ADMIN, Some(1)).await;
    h.engine.release.add_permits(1);
    wait_for(async || inflight(&h.app).await == 1).await;
    assert_eq!(
        h.app
            .clone()
            .oneshot(request(KEY, false))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    h.engine.release.add_permits(1);
    wait_for(async || inflight(&h.app).await == 0).await;
    assert_eq!(
        h.app
            .clone()
            .oneshot(request(KEY, false))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn streaming_usage_survives_real_http1_chunked_trailers() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let h = harness(Engine::new(), Duration::from_secs(5)).await;
    h.engine.release.add_permits(1);
    let (url, task) = server(h.app.clone()).await;
    let address = url.strip_prefix("http://").unwrap();
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = json!({"model":MODEL,"messages":[],"stream":true}).to_string();
    let wire = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {KEY}\r\nContent-Type: application/json\r\nX-Tinfoil-Model: {MODEL}\r\nX-Tinfoil-Request-Usage-Metrics: true\r\nX-Request-Id: synthetic-wire\r\nTE: trailers\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(wire.as_bytes()).await.unwrap();
    let mut received = Vec::new();
    timeout(Duration::from_secs(2), socket.read_to_end(&mut received))
        .await
        .unwrap()
        .unwrap();
    let received = String::from_utf8(received).unwrap();
    assert!(received.starts_with("HTTP/1.1 200"));
    assert!(
        received
            .to_lowercase()
            .contains("trailer: x-tinfoil-usage-metrics\r\n")
    );
    assert!(received.contains("data: [DONE]"));
    assert!(received.contains(
        "0\r\nx-tinfoil-usage-metrics: prompt=10,completion=2,total=12,model=glm-5-3-flash\r\n\r\n"
    ));
    task.abort();
}

#[tokio::test]
async fn a_saturated_marketplace_does_not_block_subscription_admission_or_settlement() {
    let engine_state = Engine::new();
    let (upstream, upstream_task) = server(
        Router::new()
            .route("/v1/chat/completions", post(engine))
            .with_state(engine_state.clone()),
    )
    .await;
    let reservations = Arc::new(AtomicUsize::new(0));
    let settlements = Arc::new(AtomicUsize::new(0));
    let reserve_count = reservations.clone();
    let settle_count = settlements.clone();
    let (core, core_task) = server(
        Router::new()
            .route(
                "/internal/finite-private/v1/reservations",
                post(move |Json(body): Json<Value>| {
                    reserve_count.fetch_add(1, Ordering::SeqCst);
                    async move {
                        assert_eq!(body["presentedApiKey"], "synthetic-subscription-key");
                        Json(json!({"decision":"allow", "reservation_id":"synthetic-reservation"}))
                    }
                }),
            )
            .route(
                "/internal/finite-private/v1/reservations/{id}/settle",
                post(move || {
                    settle_count.fetch_add(1, Ordering::SeqCst);
                    async { Json(json!({"ok":true})) }
                }),
            ),
    )
    .await;
    let app = app(config(core, upstream)).unwrap();
    let market = app.clone().oneshot(request(KEY, true)).await.unwrap();
    let response = app
        .clone()
        .oneshot(request("synthetic-subscription-key", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(reservations.load(Ordering::SeqCst), 1);
    assert_eq!(settlements.load(Ordering::SeqCst), 1);
    assert_eq!(inflight(&app).await, 1);
    drop(market);
    engine_state.release.add_permits(1);
    wait_for(async || inflight(&app).await == 0).await;
    core_task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn cancelled_prefill_handler_keeps_its_slot_and_never_queues_excess_requests() {
    let mut engine = Engine::new();
    engine.prefill = true;
    let h = harness(engine, Duration::from_secs(5)).await;
    let app = h.app.clone();
    let caller = tokio::spawn(async move { app.oneshot(request(KEY, false)).await });
    wait_for(async || h.engine.calls.load(Ordering::SeqCst) == 1).await;
    caller.abort();
    let mut attempts = Vec::new();
    for _ in 0..20 {
        let app = h.app.clone();
        attempts.push(tokio::spawn(async move {
            app.oneshot(request(KEY, false)).await.unwrap().status()
        }));
    }
    for attempt in attempts {
        assert_eq!(attempt.await.unwrap(), StatusCode::TOO_MANY_REQUESTS);
    }
    assert_eq!(h.engine.calls.load(Ordering::SeqCst), 1);
    h.engine.release.add_permits(1);
    wait_for(async || inflight(&h.app).await == 0).await;
}

#[tokio::test]
async fn finite_requests_and_invalid_marketplace_credentials_still_use_core() {
    let h = harness(Engine::new(), Duration::from_secs(5)).await;
    capacity(&h.app, ADMIN, Some(0)).await;
    for key in ["synthetic-subscription-key", "invalid-numes-key", ADMIN] {
        assert_eq!(
            h.app
                .clone()
                .oneshot(request(key, false))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(h.core_calls.load(Ordering::SeqCst), 3);
    assert_eq!(h.engine.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn deadline_releases_stalled_generation_without_fabricated_usage() {
    let h = harness(Engine::new(), Duration::from_millis(100)).await;
    let response = h.app.clone().oneshot(request(KEY, true)).await.unwrap();
    assert!(response.into_body().collect().await.is_err());
    wait_for(async || inflight(&h.app).await == 0).await;
    assert_eq!(
        h.app
            .clone()
            .oneshot(request(KEY, false))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn missing_usage_and_upstream_errors_cannot_create_a_charge_or_leak_slots() {
    for streaming in [false, true] {
        let mut engine = Engine::new();
        engine.final_usage = false;
        engine.release.add_permits(1);
        let h = harness(engine, Duration::from_secs(5)).await;
        let response = h
            .app
            .clone()
            .oneshot(request(KEY, streaming))
            .await
            .unwrap();
        if streaming {
            assert!(response.into_body().collect().await.is_err());
        } else {
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
            assert!(!response.headers().contains_key(USAGE_HEADER));
        }
        wait_for(async || inflight(&h.app).await == 0).await;
        assert_eq!(h.core_calls.load(Ordering::SeqCst), 0);
    }
    let mut engine = Engine::new();
    engine.fail = true;
    let h = harness(engine, Duration::from_secs(5)).await;
    assert_eq!(
        h.app
            .clone()
            .oneshot(request(KEY, true))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(inflight(&h.app).await, 0);
}

#[tokio::test]
async fn mismatched_model_and_oversized_generation_fail_before_dispatch() {
    let h = harness(Engine::new(), Duration::from_secs(5)).await;
    let mut mismatch = request(KEY, false);
    mismatch
        .headers_mut()
        .insert("x-tinfoil-model", HeaderValue::from_static("other-model"));
    let mut duplicate = request(KEY, false);
    duplicate
        .headers_mut()
        .append("x-tinfoil-model", HeaderValue::from_static(MODEL));
    let mut oversized = request(KEY, false);
    *oversized.body_mut() =
        Body::from(json!({"model":MODEL,"messages":[],"max_tokens":65}).to_string());
    let mut choices = request(KEY, false);
    *choices.body_mut() = Body::from(json!({"model":MODEL,"messages":[],"n":2}).to_string());
    for request in [mismatch, duplicate, oversized, choices] {
        assert_eq!(
            h.app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(h.engine.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.core_calls.load(Ordering::SeqCst), 0);
    assert_eq!(inflight(&h.app).await, 0);
}

#[test]
fn malformed_usage_is_never_reported_as_billable() {
    let good =
        json!({"model":MODEL,"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}});
    for invalid in [
        json!(-1),
        json!(1.5),
        json!("10"),
        json!(9_007_199_254_740_992_u64),
    ] {
        let mut value = good.clone();
        value["usage"]["prompt_tokens"] = invalid;
        assert!(Usage::from_value(&value, MODEL, 64).is_none());
    }
    let mut other_model = good.clone();
    other_model["model"] = json!("other-model");
    assert!(Usage::from_value(&other_model, MODEL, 64).is_none());
    assert!(Usage::from_value(&good, MODEL, 1).is_none());
    let wire = format!("data: {good}\r\n\r\ndata: [DONE]\r\n\r\n");
    let mut parser = StreamUsage::new(MODEL, 64);
    for chunk in wire.as_bytes().chunks(3) {
        parser.push(chunk);
    }
    assert!(parser.finish().is_some());
    let mut parser = StreamUsage::new(MODEL, 64);
    parser.push(format!("data: {good}\n\n").as_bytes());
    assert!(parser.finish().is_none()); // interrupted stream
}

#[test]
fn configuration_is_opt_in_and_credentials_are_dedicated_and_redacted() {
    let mut cfg = config("http://core".into(), "http://engine".into());
    assert!(!format!("{:?}", cfg.numes).contains(KEY));
    cfg.numes.as_mut().unwrap().max_concurrent = 0;
    assert!(app(cfg.clone()).is_ok());
    cfg.numes.as_mut().unwrap().control_api_key = KEY.into();
    assert!(app(cfg.clone()).is_err());
    cfg.numes.as_mut().unwrap().control_api_key = ADMIN.into();
    cfg.vllm_internal_api_key = KEY.into();
    assert!(app(cfg.clone()).is_err());
    cfg.numes = None;
    assert!(app(cfg).is_ok());
}

mod transport;
