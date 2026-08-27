//! NIP-98 request-auth and per-IP rate-limit behavior for the HTTP surface.

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode};
use finitechat_http::{
    ErrorResponse, GetDeviceLivenessRequest, GetDeviceLivenessResponse, KeyPackageInventoryRequest,
    NostrProfileRecord, PutNostrProfileRequest,
};
use finitechat_proto::DeviceRef;
use finitechat_server::{HttpServerState, http_router};
use finitechat_transport::MemberId;
use nostr::{Keys, SecretKey};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt;

/// Host header the signed test requests carry; the server reconstructs the
/// signed absolute URL as `http://{HOST}{uri}` from it.
const HOST: &str = "auth.test";
const ALICE_SECRET: [u8; 32] = [11; 32];
const BOB_SECRET: [u8; 32] = [22; 32];

const LIVENESS_GET_URI: &str = "/devices/liveness/get";

fn account_id(secret: &[u8; 32]) -> String {
    Keys::new(SecretKey::from_slice(secret).expect("valid secret"))
        .public_key()
        .to_hex()
}

fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Sign `body` for `uri` exactly the way the production client transport
/// does: method, absolute URL, and payload hash.
fn auth_header(secret: &[u8; 32], uri: &str, body: &[u8], created_at: u64) -> String {
    let request =
        finite_nostr::HttpAuthEventRequest::new("POST", format!("http://{HOST}{uri}"), created_at)
            .with_body(body.to_vec());
    finite_nostr::sign_http_auth_header_with_secret(secret, &request).expect("sign auth header")
}

fn liveness_get(secret: &[u8; 32]) -> GetDeviceLivenessRequest {
    GetDeviceLivenessRequest {
        device: DeviceRef::new(account_id(secret), "phone"),
        now_ms: 0,
    }
}

async fn post_signed<T: Serialize>(
    app: Router,
    uri: &str,
    body: &T,
    secret: &[u8; 32],
) -> Response<Body> {
    let bytes = serde_json::to_vec(body).expect("json body");
    let authorization = auth_header(secret, uri, &bytes, now_unix_seconds());
    post_raw(app, uri, bytes, Some(authorization), None).await
}

async fn post_unsigned<T: Serialize>(app: Router, uri: &str, body: &T) -> Response<Body> {
    post_raw(
        app,
        uri,
        serde_json::to_vec(body).expect("json body"),
        None,
        None,
    )
    .await
}

async fn post_raw(
    app: Router,
    uri: &str,
    body: Vec<u8>,
    authorization: Option<String>,
    client_ip: Option<&str>,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .header("host", HOST);
    if let Some(authorization) = authorization {
        builder = builder.header("authorization", authorization);
    }
    if let Some(client_ip) = client_ip {
        builder = builder.header("x-forwarded-for", client_ip);
    }
    app.oneshot(builder.body(Body::from(body)).expect("request"))
        .await
        .expect("response")
}

async fn read_json<T: DeserializeOwned>(response: Response<Body>) -> T {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("json response")
}

#[tokio::test]
async fn missing_header_is_accepted_when_signed_requests_not_required() {
    let app = http_router(HttpServerState::default());

    let response = post_unsigned(app, LIVENESS_GET_URI, &liveness_get(&ALICE_SECRET)).await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn missing_header_is_rejected_when_signed_requests_required() {
    let state = HttpServerState::default().with_require_signed_requests(true);
    let app = http_router(state);

    let response = post_unsigned(app, LIVENESS_GET_URI, &liveness_get(&ALICE_SECRET)).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body: ErrorResponse = read_json(response).await;
    assert_eq!(body.kind, "unauthorized");
}

#[tokio::test]
async fn valid_signature_is_accepted_when_signed_requests_required() {
    let state = HttpServerState::default().with_require_signed_requests(true);
    let app = http_router(state);

    let response = post_signed(
        app,
        LIVENESS_GET_URI,
        &liveness_get(&ALICE_SECRET),
        &ALICE_SECRET,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body: GetDeviceLivenessResponse = read_json(response).await;
    assert!(!body.live);
}

#[tokio::test]
async fn valid_signature_for_the_wrong_account_is_rejected() {
    let app = http_router(HttpServerState::default());

    // Bob signs a body that names Alice's account.
    let response = post_signed(
        app,
        LIVENESS_GET_URI,
        &liveness_get(&ALICE_SECRET),
        &BOB_SECRET,
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body: ErrorResponse = read_json(response).await;
    assert_eq!(body.kind, "unauthorized");
}

#[tokio::test]
async fn tampered_body_is_rejected() {
    let app = http_router(HttpServerState::default());
    let signed_bytes = serde_json::to_vec(&liveness_get(&ALICE_SECRET)).expect("json body");
    let authorization = auth_header(
        &ALICE_SECRET,
        LIVENESS_GET_URI,
        &signed_bytes,
        now_unix_seconds(),
    );
    let mut tampered: serde_json::Value = serde_json::from_slice(&signed_bytes).expect("json");
    tampered["now_ms"] = serde_json::json!(1);
    let tampered = serde_json::to_vec(&tampered).expect("tampered json body");

    let response = post_raw(app, LIVENESS_GET_URI, tampered, Some(authorization), None).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stale_created_at_is_rejected() {
    let app = http_router(HttpServerState::default());
    let body = serde_json::to_vec(&liveness_get(&ALICE_SECRET)).expect("json body");
    let authorization = auth_header(
        &ALICE_SECRET,
        LIVENESS_GET_URI,
        &body,
        now_unix_seconds() - 3_600,
    );

    let response = post_raw(app, LIVENESS_GET_URI, body, Some(authorization), None).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invalid_header_is_rejected_even_when_signed_requests_not_required() {
    let app = http_router(HttpServerState::default());
    let body = serde_json::to_vec(&liveness_get(&ALICE_SECRET)).expect("json body");

    let response = post_raw(
        app,
        LIVENESS_GET_URI,
        body,
        Some("Nostr not-base64".to_owned()),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn signed_mutating_route_accepts_the_matching_account() {
    let state = HttpServerState::default().with_require_signed_requests(true);
    let app = http_router(state);
    let put = PutNostrProfileRequest {
        profile: NostrProfileRecord {
            account_id: account_id(&ALICE_SECRET),
            name: Some("alice".to_owned()),
            display_name: None,
            about: None,
            picture: None,
            bot: None,
            finite_role: None,
            metadata_json: None,
            fetched_at_ms: 1_000,
            expires_at_ms: 60_000,
        },
    };

    let response = post_signed(app, "/profiles/nostr", &put, &ALICE_SECRET).await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn rate_limit_rejects_requests_beyond_the_window_allowance() {
    let state = HttpServerState::default().with_rate_limit(2, 60);
    let app = http_router(state);
    let inventory = KeyPackageInventoryRequest {
        owner: MemberId::new(b"inventory-owner".to_vec()),
    };
    let body = serde_json::to_vec(&inventory).expect("json body");

    for expected in [
        StatusCode::OK,
        StatusCode::OK,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let response = post_raw(
            app.clone(),
            "/key-packages/inventory",
            body.clone(),
            None,
            Some("203.0.113.1"),
        )
        .await;
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn rate_limit_tracks_client_ips_independently() {
    let state = HttpServerState::default().with_rate_limit(1, 60);
    let app = http_router(state);
    let inventory = KeyPackageInventoryRequest {
        owner: MemberId::new(b"inventory-owner".to_vec()),
    };
    let body = serde_json::to_vec(&inventory).expect("json body");

    let first = post_raw(
        app.clone(),
        "/key-packages/inventory",
        body.clone(),
        None,
        Some("203.0.113.1"),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = post_raw(
        app.clone(),
        "/key-packages/inventory",
        body.clone(),
        None,
        Some("203.0.113.1"),
    )
    .await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    // A different client IP has its own allowance.
    let other_ip = post_raw(
        app,
        "/key-packages/inventory",
        body,
        None,
        Some("203.0.113.2"),
    )
    .await;
    assert_eq!(other_ip.status(), StatusCode::OK);
}

#[tokio::test]
async fn rate_limit_ignores_spoofed_first_hops_of_the_forwarded_header() {
    let state = HttpServerState::default().with_rate_limit(1, 60);
    let app = http_router(state);
    let inventory = KeyPackageInventoryRequest {
        owner: MemberId::new(b"inventory-owner".to_vec()),
    };
    let body = serde_json::to_vec(&inventory).expect("json body");

    // The harness drives the router in-process, so the peer resolves to
    // loopback and X-Forwarded-For is trusted — as it is behind host-local
    // Caddy, which appends the observed client address as the LAST hop.
    let first = post_raw(
        app.clone(),
        "/key-packages/inventory",
        body.clone(),
        None,
        Some("198.51.100.1, 203.0.113.9"),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    // A fresh spoofed first hop must not buy a fresh bucket: the last hop
    // (the address the proxy observed) is what keys the limiter.
    let spoofed = post_raw(
        app.clone(),
        "/key-packages/inventory",
        body.clone(),
        None,
        Some("198.51.100.2, 203.0.113.9"),
    )
    .await;
    assert_eq!(spoofed.status(), StatusCode::TOO_MANY_REQUESTS);
    // A genuinely different client (different last hop) still has allowance.
    let other_client = post_raw(
        app,
        "/key-packages/inventory",
        body,
        None,
        Some("198.51.100.1, 203.0.113.10"),
    )
    .await;
    assert_eq!(other_client.status(), StatusCode::OK);
}

#[tokio::test]
async fn rate_limit_resets_after_the_window() {
    let state = HttpServerState::default().with_rate_limit(1, 1);
    let app = http_router(state);
    let inventory = KeyPackageInventoryRequest {
        owner: MemberId::new(b"inventory-owner".to_vec()),
    };
    let body = serde_json::to_vec(&inventory).expect("json body");

    let first = post_raw(
        app.clone(),
        "/key-packages/inventory",
        body.clone(),
        None,
        Some("203.0.113.1"),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = post_raw(
        app.clone(),
        "/key-packages/inventory",
        body.clone(),
        None,
        Some("203.0.113.1"),
    )
    .await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    let third = post_raw(
        app,
        "/key-packages/inventory",
        body,
        None,
        Some("203.0.113.1"),
    )
    .await;
    assert_eq!(third.status(), StatusCode::OK);
}
