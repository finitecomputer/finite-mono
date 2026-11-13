//! The dashboard deletes an Organization Brain with the account's hosted
//! Brain key: this hosted device signs the dashboard-shaped `delete-brain`
//! event and the protected requests, and the Brain server accepts them.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use finite_brain_core::{
    AdminAccessAction, AdminAccessChangePayload, AdminAccessChangeValidation,
    BRAIN_IDENTITY_PROVIDER_VERSION, BrainId, bootstrap_organization_brain,
};
use finitechat_hosted_device::{HostedDeviceConfig, WORKOS_USER_HEADER, app};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::Digest;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;
use tower::ServiceExt;

const TOKEN: &str = "hosted-device-test-token";
const BRAIN_ORIGIN: &str = "https://brain.finite.test";

/// Shared with the dashboard test `the delete-brain template matches
/// finite-brain-core's canonical event` in `src/lib/brain-deletion.test.ts`.
const DASHBOARD_CANONICAL_DELETE_CONTENT: &str = concat!(
    r#"{"version":"finite-brain-admin-access-change-v1","brainId":"v1","changeId":"delete-brain-0123456789abcdef","#,
    r#""action":"delete-brain","adminNpub":"npub1example","note":"sha256:abababababababababababababababababababababababababababababababab","createdAt":"2026-05-28T20:26:40Z"}"#
);

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn provider(hosted: &axum::Router, operation: &str, input: Value) -> Value {
    let response = hosted
        .clone()
        .oneshot(
            Request::post("/v1/brain/identity-provider")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, "user_admin")
                .header("x-finite-brain-public-origin", BRAIN_ORIGIN)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"version": BRAIN_IDENTITY_PROVIDER_VERSION, "operation": operation, "input": input})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{operation}");
    json_body(response).await
}

/// One Brain request signed the way `hostedSignedBrainRequest` signs it.
async fn signed(
    hosted: &axum::Router,
    brain: &axum::Router,
    method: &str,
    path: &str,
    body: &str,
) -> axum::response::Response {
    let url = format!("{BRAIN_ORIGIN}{path}");
    let mut tags = vec![
        json!(["u", url]),
        json!(["method", method]),
        json!(["nonce", hex::encode(rand_nonce())]),
    ];
    if !body.is_empty() {
        tags.push(json!(["payload", hex::encode(sha2::Sha256::digest(body))]));
    }
    let event = provider(
        hosted,
        "authorizeHttpRequest",
        json!({
            "method": method,
            "url": url,
            "bodyText": body,
            "eventTemplate": {"kind": 27_235, "created_at": now(), "tags": tags, "content": ""},
        }),
    )
    .await;
    let authorization =
        finite_nostr::encode_http_auth_header(&serde_json::from_value(event).unwrap());
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", authorization);
    if !body.is_empty() {
        request = request.header("content-type", "application/json");
    }
    brain
        .clone()
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn rand_nonce() -> [u8; 16] {
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).unwrap();
    nonce
}

fn delete_content(
    brain_id: &str,
    change_id: &str,
    admin_npub: &str,
    digest: &str,
    created_at: &str,
) -> String {
    AdminAccessChangePayload::new(&AdminAccessChangeValidation {
        brain_id: BrainId::new(brain_id).unwrap(),
        change_id: change_id.to_owned(),
        action: AdminAccessAction::DeleteBrain,
        admin_npub: admin_npub.to_owned(),
        folder_id: None,
        target_npub: None,
        key_version: None,
        note: Some(digest.to_owned()),
        created_at: created_at.to_owned(),
    })
    .canonical_json()
}

#[tokio::test]
async fn hosted_device_signs_the_dashboard_brain_deletion_the_brain_server_accepts() {
    // The dashboard builds exactly the canonical content finite-brain-core
    // expects.
    assert_eq!(
        delete_content(
            "v1",
            "delete-brain-0123456789abcdef",
            "npub1example",
            &format!("sha256:{}", "ab".repeat(32)),
            "2026-05-28T20:26:40Z",
        ),
        DASHBOARD_CANONICAL_DELETE_CONTENT
    );

    let root = TempDir::new().unwrap();
    let hosted = app(HostedDeviceConfig {
        data_root: root.path().to_path_buf(),
        server_url: "http://127.0.0.1:9".to_owned(),
        public_url: "https://chat.finite.computer".to_owned(),
        api_token: TOKEN.to_owned(),
    });
    let state = hosted
        .clone()
        .oneshot(
            Request::get("/v1/app/state")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, "user_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(state.status(), StatusCode::OK);
    let npub = provider(&hosted, "identifyMember", Value::Null).await["npub"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut store = finite_brain_store::BrainStore::open_in_memory().unwrap();
    store
        .create_brain_bootstrap(
            &bootstrap_organization_brain("v1", "V1", &npub).unwrap(),
            &[],
        )
        .unwrap();
    let brain = finite_brain_server::router_with_state(finite_brain_server::ServerState::new(
        store,
        BRAIN_ORIGIN,
    ));

    let preview = signed(&hosted, &brain, "GET", "/v1/brains/v1/deletion-preview", "").await;
    assert_eq!(preview.status(), StatusCode::OK);
    let preview = json_body(preview).await;
    assert_eq!(preview["server"], BRAIN_ORIGIN);
    let digest = preview["stateDigest"].as_str().unwrap().to_owned();

    let created_at = now();
    let created_at_text = time::OffsetDateTime::from_unix_timestamp(created_at as i64)
        .unwrap()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    let change_id = "delete-brain-hosted-proof";
    let event = provider(
        &hosted,
        "authorizeBrainEvent",
        json!({
            "intent": "brain-access-change",
            "eventTemplate": {
                "kind": 30_078,
                "created_at": created_at,
                "tags": [
                    ["d", format!("finite-brain-admin-access-change:v1:{change_id}")],
                    ["brain", "v1"],
                    ["action", "delete-brain"],
                ],
                "content": delete_content("v1", change_id, &npub, &digest, &created_at_text),
            },
        }),
    )
    .await;
    let body = json!({ "deletionEvent": event }).to_string();
    let deleted = signed(&hosted, &brain, "DELETE", "/v1/brains/v1", &body).await;
    assert_eq!(deleted.status(), StatusCode::OK);
    let deleted = json_body(deleted).await;
    assert_eq!(deleted["duplicate"], false);
    assert_eq!(deleted["receipt"]["actorNpub"], npub.as_str());
    assert_eq!(deleted["receipt"]["stateDigest"], digest.as_str());
    let retry = json_body(signed(&hosted, &brain, "DELETE", "/v1/brains/v1", &body).await).await;
    assert_eq!(retry["duplicate"], true);
    assert_eq!(retry["receipt"], deleted["receipt"]);
    let gone = signed(&hosted, &brain, "GET", "/v1/brains/v1/metadata", "").await;
    assert_eq!(gone.status(), StatusCode::GONE);
}
