use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, Response, StatusCode};
use finitechat_delivery::{HttpKeyPackageId, HttpKeyPackagePublication};
use finitechat_http::{
    ClaimKeyPackageRequest, FINITECHAT_SERVER_CONTRACT_VERSION, HealthResponse,
    PublishKeyPackageResponse,
};
use finitechat_server::{HttpServerState, http_router};
use finitechat_transport::MemberId;
use finitechat_transport::engine::KeyPackage;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt;

#[tokio::test]
async fn health_reports_ok() {
    let app = http_router(HttpServerState::default());

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body: HealthResponse = read_json(response).await;
    assert_eq!(body.status, "ok");
    assert_eq!(
        body.server_contract_version,
        Some(FINITECHAT_SERVER_CONTRACT_VERSION)
    );
    assert_eq!(body.server_version.as_deref(), Some("0.1.0"));
    assert_non_empty(body.source_commit.as_deref());
    assert_non_empty(body.source_branch.as_deref());
    assert!(body.source_dirty.is_some());
}

fn assert_non_empty(value: Option<&str>) {
    assert!(value.is_some_and(|value| !value.trim().is_empty()));
}

#[tokio::test]
async fn key_package_publish_and_claim_is_single_use() {
    let app = http_router(HttpServerState::default());
    let owner = member("alice-device");
    let key_package_id = HttpKeyPackageId::new(b"kp-route-1".to_vec());
    let publication = HttpKeyPackagePublication {
        key_package_id: key_package_id.clone(),
        owner: owner.clone(),
        key_package: KeyPackage::new(b"key-package-bytes".to_vec()),
    };

    let response = post_json(app.clone(), "/key-packages", &publication).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: PublishKeyPackageResponse = read_json(response).await;
    assert!(body.published);

    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<finitechat_delivery::HttpClaimedKeyPackage> = read_json(response).await;
    let claimed = claimed.expect("published KeyPackage can be claimed once");
    assert_eq!(claimed.key_package_id, key_package_id);
    assert_eq!(claimed.owner, owner);
    assert_eq!(claimed.key_package.bytes(), b"key-package-bytes");

    let response = post_json(
        app,
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: member("alice-device"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<finitechat_delivery::HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(claimed, None);
}

async fn post_json<T: Serialize>(app: Router, uri: &str, body: &T) -> Response<Body> {
    app.oneshot(
        Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(body).expect("json body")))
            .expect("request"),
    )
    .await
    .expect("response")
}

async fn read_json<T: DeserializeOwned>(response: Response<Body>) -> T {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("json response")
}

fn member(label: &str) -> MemberId {
    MemberId::new(label.as_bytes().to_vec())
}
