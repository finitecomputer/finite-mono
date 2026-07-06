//! End-to-end check of the chained-local-limiter topology using the same
//! in-process pattern as the finite-private-limiter tests: a real Core
//! (admission/metering), the in-tree limiter with the chained config, and a
//! fake "deployed limiter" upstream that only accepts the one operator key.

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use finite_saas_core::api::router as core_router;
use finite_saas_core::store::CoreStore;
use finite_saas_core::{ApproveFinitePrivateGrantInput, IssueFinitePrivateApiKeyInput};
use finite_saas_local::{ChainedLimiterInputs, chained_limiter_config};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::TcpListener;

const OPERATOR_UPSTREAM_KEY: &str = "fpk_live_operator_upstream";
const LOCAL_RUNTIME_KEY: &str = "fpk_local_runtime_key";

#[derive(Clone)]
struct DeployedLimiterDouble {
    calls: Arc<AtomicUsize>,
}

/// Stands in for the deployed limiter: mounted at /v1/chat/completions under
/// the host root (the chained limiter must strip the /v1 base-URL suffix to
/// hit it) and rejects everything except the operator upstream key.
async fn deployed_limiter_double(
    State(state): State<DeployedLimiterDouble>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if authorization != format!("Bearer {OPERATOR_UPSTREAM_KEY}") {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({ "error": { "code": "invalid_api_key" } })),
        )
            .into_response();
    }
    let request = serde_json::from_slice::<Value>(&body).unwrap();
    assert_eq!(request["model"], "glm-5-2");
    state.calls.fetch_add(1, Ordering::SeqCst);
    axum::Json(json!({
        "id": "chatcmpl_chained",
        "model": "glm-5-2",
        "choices": [{ "message": { "role": "assistant", "content": "ok" }}],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "total_tokens": 120
        }
    }))
    .into_response()
}

#[tokio::test]
async fn chained_local_limiter_admits_local_keys_and_forwards_with_operator_key() {
    // Local Core with a runtime key issued the same way runner provisioning
    // issues them.
    let core_store = CoreStore::memory();
    let grant = core_store
        .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
            verified_email: "local-canary@finite.computer".to_string(),
            workos_user_id: Some("user_local_canary".to_string()),
            limit_profile_id: None,
            now: Some("2026-07-02T12:00:00Z".to_string()),
        })
        .await
        .unwrap();
    core_store
        .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id,
            raw_key: LOCAL_RUNTIME_KEY.to_string(),
            project_id: None,
            agent_runtime_id: None,
            now: Some("2026-07-02T12:00:00Z".to_string()),
        })
        .await
        .unwrap();
    let core_url = spawn(core_router(core_store, "local-core-token")).await;

    let upstream_calls = Arc::new(AtomicUsize::new(0));
    let deployed_limiter_url = spawn(
        Router::new()
            .route("/v1/chat/completions", post(deployed_limiter_double))
            .with_state(DeployedLimiterDouble {
                calls: upstream_calls.clone(),
            }),
    )
    .await;

    // Exactly the config finite-saas-local assembles: the upstream is given
    // in agent-facing `.../v1` form and must be reduced to the host root.
    let config = chained_limiter_config(&ChainedLimiterInputs {
        core_url: core_url.clone(),
        core_api_token: "local-core-token".to_string(),
        upstream_base_url: format!("{deployed_limiter_url}/v1"),
        upstream_api_key: Some(OPERATOR_UPSTREAM_KEY.to_string()),
        dashboard_url: "http://127.0.0.1:13002/dashboard".to_string(),
    })
    .unwrap();
    assert_eq!(config.upstream_base_url, deployed_limiter_url);
    let chained_limiter_url = spawn(finite_private_limiter::app(config).unwrap()).await;

    // A locally provisioned key completes a real round trip through both
    // limiter hops.
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{chained_limiter_url}/v1/chat/completions"))
        .bearer_auth(LOCAL_RUNTIME_KEY)
        .header("x-request-id", "req-chained-ok")
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
    assert_eq!(body["model"], "glm-5-2");
    assert_eq!(upstream_calls.load(Ordering::SeqCst), 1);

    // A key unknown to local Core is denied locally, before the upstream hop.
    let response = client
        .post(format!("{chained_limiter_url}/v1/chat/completions"))
        .bearer_auth("fpk_not_provisioned_here")
        .header("x-request-id", "req-chained-denied")
        .json(&json!({
            "model": "glm-5-2",
            "messages": [{ "role": "user", "content": "hello" }],
            "max_tokens": 64
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(upstream_calls.load(Ordering::SeqCst), 1);
}

async fn spawn(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}
