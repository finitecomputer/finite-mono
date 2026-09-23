use super::*;
use crate::RunnerClass;
use crate::auth::test_support::{
    BOUNDARY_RUNNER_TOKEN, FULL_RUNNER_TOKEN, OPERATOR_ORG_ID, SECOND_RUNNER_TOKEN, access_token,
    access_token_with_subject, core_auth, core_auth_with_runner_credentials,
    runner_credential_config, shared_route_core_auth,
};
use crate::parse_time;
use crate::test_support::with_isolated_postgres;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
mod account_email_change;
mod admin_authorization;
mod agent_creation;
mod agent_creation_control;
mod finite_private_keys;
mod finite_private_usage;
mod hosted_access;
mod identity_authorization;
mod public_contracts;
mod runner_authorization;
mod runner_leases;
mod runtime_control;
mod runtime_health;

const TOKEN: &str = "core-token";

fn test_auth() -> CoreAuth {
    shared_route_core_auth(TOKEN)
}

fn scoped_token(scope: &str) -> String {
    let digest = Sha256::digest(format!("{TOKEN}:{scope}").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn scoped_test_auth() -> CoreAuth {
    core_auth(
        TOKEN,
        scoped_token("runner"),
        scoped_token("finite-private-usage"),
    )
}

fn runner_authorization() -> String {
    format!("Bearer {}", scoped_token("runner"))
}

fn boundary_runner_authorization() -> String {
    format!("Bearer {BOUNDARY_RUNNER_TOKEN}")
}

fn usage_authorization() -> String {
    format!("Bearer {}", scoped_token("finite-private-usage"))
}

fn runtime_capabilities_json(runtime_upgrade: bool) -> serde_json::Value {
    serde_json::to_value(RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
        native_hermes_chat: false,
        restart: true,
        recover_known_good_chat: false,
        runtime_upgrade,
        stop: true,
        runtime_retirement: false,
    }))
    .unwrap()
}

fn runner_capacity_json(runner_class: RunnerClass) -> serde_json::Value {
    serde_json::json!({
        "runnerClasses": [runner_class],
        "runtimeCapabilities": runtime_capabilities_json(runner_class == RunnerClass::Kata),
    })
}

fn assert_json_omits_keys(value: &serde_json::Value, forbidden: &[&str]) {
    match value {
        serde_json::Value::Object(object) => {
            for key in forbidden {
                assert!(
                    !object.contains_key(*key),
                    "public JSON unexpectedly contained `{key}`: {value}"
                );
            }
            for child in object.values() {
                assert_json_omits_keys(child, forbidden);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                assert_json_omits_keys(child, forbidden);
            }
        }
        _ => {}
    }
}

async fn issue_test_launch_code(store: &CoreStore) -> String {
    store
        .issue_launch_code_batch(crate::launch_codes::IssueLaunchCodeBatchInput {
            name: "Core API test batch".to_string(),
            code_count: 1,
            expires_in_hours: Some(crate::launch_codes::MAX_LAUNCH_CODE_BATCH_HOURS),
            hosting_tier: None,
            created_by_workos_user_id: "workos-test-operator".to_string(),
            now: None,
        })
        .await
        .expect("test Launch Code batch should issue")
        .codes
        .into_iter()
        .next()
        .expect("one test Launch Code should be returned")
        .code
}

fn admin_router(store: CoreStore) -> Router {
    router_with_runtime_upgrades(store, test_auth(), true)
}

fn identity_headers(email: &str, verified: &str) -> Vec<(String, String)> {
    workos_headers(email, matches!(verified, "1" | "true" | "yes"), None)
}

fn operator_identity_headers(email: &str) -> Vec<(String, String)> {
    workos_headers(email, true, Some(OPERATOR_ORG_ID))
}

fn workos_headers(
    email: &str,
    verified: bool,
    organization_id: Option<&str>,
) -> Vec<(String, String)> {
    vec![(
        "authorization".to_string(),
        format!("Bearer {}", access_token(email, verified, organization_id)),
    )]
}

async fn send_json(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(String, String)],
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| serde_json::json!({ "raw": String::from_utf8_lossy(&bytes) }))
    };
    (status, json)
}

/// Provision one hosted agent through the same HTTP flow the dashboard and
/// runner use, returning (project_id, agent_runtime_id).
async fn provision_hosted_agent(app: &Router, launch_code: &str) -> (String, String) {
    let service = [("authorization".to_string(), "Bearer core-token".to_string())];
    let (status, _) = send_json(
        app,
        "PUT",
        "/api/core/v1/runtime-artifacts/artifact-v1",
        &service,
        Some(serde_json::json!({
            "kind": "oci_image",
            "reference": format!(
                "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                "a".repeat(64)
            ),
            "versionLabel": "v1",
            "stateSchemaVersion": "state-v1",
            "baseImage": "python:3.11-trixie",
            "promoted": true,
            "now": "2026-05-25T12:00:00Z"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let owner = identity_headers("owner@finite.vip", "true");
    let (status, _) = send_json(
        app,
        "POST",
        "/api/core/v1/me/agent-creation-requests",
        &owner,
        Some(serde_json::json!({
            "displayName": "Oslo Agent",
            "launchCode": launch_code,
            "idempotencyKey": "browser-submit-1"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, lease) = send_json(
        app,
        "POST",
        "/api/core/v1/agent-creation-requests/lease",
        &service,
        Some(serde_json::json!({
            "runnerId": "runner-oslo-1",
            "leaseToken": "lease-token-1",
            "leaseSeconds": 300,
            "now": "2026-05-25T13:00:00Z"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let request_id = lease["request"]["id"].as_str().unwrap().to_string();

    let (status, completed) = send_json(
        app,
        "POST",
        &format!("/api/core/v1/agent-creation-requests/{request_id}/complete"),
        &service,
        Some(serde_json::json!({
            "runnerId": "runner-oslo-1",
            "leaseToken": "lease-token-1",
            "sourceHostId": "oslo-host-1",
            "sourceMachineId": "oslo-agent-001",
            "runtimeArtifactId": "artifact-v1",
            "runtimeHost": "oslo-host-1",
            "runtimeStatus": "online",
            "activeInferenceProfile": "finite-private",
            "hermesAvailable": true,
            "publishedAppUrls": [],
            "runtimeCapabilities": runtime_capabilities_json(true),
            "now": "2026-05-25T13:01:00Z"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (
        completed["project"]["id"].as_str().unwrap().to_string(),
        completed["request"]["agent_runtime_id"]
            .as_str()
            .unwrap()
            .to_string(),
    )
}
