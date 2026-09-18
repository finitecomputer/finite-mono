use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use finite_private_limiter::{LimiterConfig, app};
use tower::ServiceExt;

fn config() -> LimiterConfig {
    LimiterConfig::new(
        "http://127.0.0.1:1".into(),
        "synthetic-accounting-credential".into(),
        "http://127.0.0.1:1".into(),
        "synthetic-model-credential".into(),
        "https://dashboard.invalid".into(),
    )
}

#[tokio::test]
async fn old_deployment_configuration_exposes_only_existing_liveness() {
    let response = app(config())
        .unwrap()
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap().as_ref(),
        b"finite_private_limiter_live 1\n"
    );
}

#[tokio::test]
async fn only_monitoring_credential_can_read_request_metrics() {
    let mut configuration = config();
    configuration.metrics_auth_token = Some("synthetic-monitoring-credential".into());
    let router = app(configuration).unwrap();
    for credential in [
        None,
        Some("synthetic-accounting-credential"),
        Some("synthetic-model-credential"),
        Some("synthetic-inference-key"),
    ] {
        let mut request = Request::get("/metrics");
        if let Some(credential) = credential {
            request = request.header("Authorization", format!("Bearer {credential}"));
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = router
        .oneshot(
            Request::get("/metrics")
                .header("Authorization", "Bearer synthetic-monitoring-credential")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body = std::str::from_utf8(&body).unwrap();
    assert!(body.contains("finite_private_limiter_requests_total"));
    assert!(!body.contains("synthetic-"));
}

#[tokio::test]
async fn missing_authorization_is_counted_once_and_probes_are_excluded() {
    let mut configuration = config();
    configuration.metrics_auth_token = Some("synthetic-monitoring-credential".into());
    let router = app(configuration).unwrap();
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    let response = router
        .clone()
        .oneshot(
            Request::post("/v1/chat/completions")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    r#"{"model":"synthetic-private-model","messages":[]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(
                Request::get("/metrics")
                    .header("Authorization", "Bearer synthetic-monitoring-credential")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let metrics = std::str::from_utf8(&bytes).unwrap();
        let total = |name: &str, matching_label: &str| -> u64 {
            metrics
                .lines()
                .filter(|line| line.starts_with(name) && line.contains(matching_label))
                .map(|line| line.rsplit_once(' ').unwrap().1.parse::<u64>().unwrap())
                .sum()
        };
        assert_eq!(total("finite_private_limiter_requests_total{", ""), 1);
        assert_eq!(
            total("finite_private_limiter_requests_started_total ", ""),
            1
        );
        assert_eq!(
            total(
                "finite_private_limiter_admissions_total{",
                "outcome=\"refused\""
            ),
            1
        );
        assert_eq!(total("finite_private_limiter_active_requests ", ""), 0);
        assert!(!metrics.contains("synthetic-private-model"));
    }
}

#[tokio::test]
async fn legacy_unrestricted_routing_does_not_create_unbounded_model_labels() {
    let mut configuration = config();
    configuration.metrics_auth_token = Some("synthetic-monitoring-credential".into());
    let router = app(configuration).unwrap();
    for index in 0..12 {
        let response = router
            .clone()
            .oneshot(
                Request::post("/v1/chat/completions")
                    .header("Content-Type", "application/json")
                    .header("Authorization", "Bearer synthetic-inference-key")
                    .body(Body::from(format!(
                        r#"{{"model":"untrusted-client-model-{index}","messages":[]}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Legacy routing still attempts normal Core admission. The synthetic
        // Core is unavailable; telemetry must not change admission behavior.
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    let response = router
        .oneshot(
            Request::get("/metrics")
                .header("Authorization", "Bearer synthetic-monitoring-credential")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let metrics = std::str::from_utf8(&bytes).unwrap();
    assert!(!metrics.contains("untrusted-client-model-"));
    assert!(!metrics.contains("synthetic-inference-key"));
}

#[tokio::test]
async fn oversized_inference_body_is_counted_without_changing_its_rejection() {
    let mut configuration = config();
    configuration.metrics_auth_token = Some("synthetic-monitoring-credential".into());
    let router = app(configuration).unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::post("/v1/chat/completions")
                .header("Authorization", "Bearer synthetic-inference-key")
                .body(Body::from(vec![b' '; 2 * 1024 * 1024 + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let response = router
        .oneshot(
            Request::get("/metrics")
                .header("Authorization", "Bearer synthetic-monitoring-credential")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let metrics = std::str::from_utf8(&bytes).unwrap();
    let refused: u64 = metrics
        .lines()
        .filter(|line| {
            line.starts_with("finite_private_limiter_admissions_total{")
                && line.contains("outcome=\"refused\"")
        })
        .map(|line| line.rsplit_once(' ').unwrap().1.parse::<u64>().unwrap())
        .sum();
    assert_eq!(refused, 1);
}
