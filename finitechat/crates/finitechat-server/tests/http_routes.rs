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
use serde_json::Value;
use std::time::Duration;
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
    assert!(body.source_fingerprint.is_some() || body.source_commit.is_some());
    if body.source_fingerprint.is_some() {
        assert_non_empty(body.source_fingerprint.as_deref());
    } else {
        assert_non_empty(body.source_commit.as_deref());
        assert_non_empty(body.source_branch.as_deref());
    }
    assert!(body.source_dirty.is_some());
}

#[tokio::test]
async fn readyz_exercises_the_durable_write_path() {
    let directory = tempfile::tempdir().expect("tempdir");
    let database = directory.path().join("finitechat.sqlite3");
    let state = HttpServerState::from_sqlite_path(&database).expect("durable state");
    let app = http_router(state);

    let response = get(app.clone(), "/readyz").await;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = read_json(response).await;
    assert_eq!(body["status"], "ready");
    assert_eq!(body["checks"]["delivery_core"]["status"], "ok");
    assert_eq!(body["checks"]["durable_store"]["status"], "ok");

    let database = rusqlite::Connection::open(&database).expect("inspect readiness evidence");
    let first_probe: String = database
        .query_row(
            "SELECT value FROM server_meta WHERE key = 'readiness_probe_ms'",
            [],
            |row| row.get(0),
        )
        .expect("readiness stamp");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let cached_response = get(app, "/readyz").await;
    assert_eq!(cached_response.status(), StatusCode::OK);
    let second_probe: String = database
        .query_row(
            "SELECT value FROM server_meta WHERE key = 'readiness_probe_ms'",
            [],
            |row| row.get(0),
        )
        .expect("cached readiness stamp");
    let probe_rows: u64 = database
        .query_row(
            "SELECT count(*) FROM server_meta WHERE key = 'readiness_probe_ms'",
            [],
            |row| row.get(0),
        )
        .expect("readiness row count");
    let delivery_entries: u64 = database
        .query_row("SELECT count(*) FROM delivery_entries", [], |row| {
            row.get(0)
        })
        .expect("delivery entry count");
    assert_eq!(
        second_probe, first_probe,
        "fresh readiness results must be cached instead of committing again"
    );
    assert_eq!(probe_rows, 1, "the readiness stamp stays a singleton");
    assert_eq!(
        delivery_entries, 0,
        "readiness must not create user delivery history"
    );
}

#[tokio::test]
async fn readyz_adds_health_evidence_without_changing_existing_delivery_state() {
    let directory = tempfile::tempdir().expect("tempdir");
    let database = directory.path().join("finitechat.sqlite3");
    let owner = member("existing-alice-device");
    let key_package_id = HttpKeyPackageId::new(b"existing-key-package".to_vec());
    let state = HttpServerState::from_sqlite_path(&database).expect("durable state");
    state
        .publish_key_package(HttpKeyPackagePublication {
            key_package_id: key_package_id.clone(),
            owner: owner.clone(),
            key_package: KeyPackage::new(b"existing-key-package-bytes".to_vec()),
        })
        .expect("persist existing delivery state");
    drop(state);

    let existing = rusqlite::Connection::open(&database).expect("existing database");
    let entries_before: u64 = existing
        .query_row("SELECT count(*) FROM delivery_entries", [], |row| {
            row.get(0)
        })
        .expect("existing entry count");
    assert_eq!(entries_before, 0);
    let payloads_before: u64 = existing
        .query_row("SELECT count(*) FROM sql_key_packages", [], |row| {
            row.get(0)
        })
        .expect("existing KeyPackage count");
    assert_eq!(payloads_before, 1);
    drop(existing);

    let state = HttpServerState::from_sqlite_path(&database).expect("reopen existing state");
    let app = http_router(state);
    let response = get(app.clone(), "/readyz").await;
    assert_eq!(response.status(), StatusCode::OK);

    let probed = rusqlite::Connection::open(&database).expect("probed database");
    let entries_after_readiness: u64 = probed
        .query_row("SELECT count(*) FROM delivery_entries", [], |row| {
            row.get(0)
        })
        .expect("entry count after readiness");
    assert_eq!(entries_after_readiness, entries_before);
    drop(probed);

    let response = post_json(
        app,
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<finitechat_delivery::HttpClaimedKeyPackage> = read_json(response).await;
    let claimed = claimed.expect("the existing KeyPackage survives the restart");
    assert_eq!(claimed.key_package_id, key_package_id);
    assert_eq!(claimed.owner, owner);
    assert_eq!(
        claimed.key_package.bytes,
        b"existing-key-package-bytes".to_vec(),
        "the durable payload home must serve claim bytes across a restart"
    );
}

#[tokio::test]
async fn readyz_fails_quickly_when_the_durable_write_path_is_locked() {
    let directory = tempfile::tempdir().expect("tempdir");
    let database = directory.path().join("finitechat.sqlite3");
    let state = HttpServerState::from_sqlite_path(&database).expect("durable state");
    let app = http_router(state);
    let lock = rusqlite::Connection::open(&database).expect("lock connection");
    lock.execute_batch("BEGIN IMMEDIATE")
        .expect("hold SQLite write lock");

    let response = tokio::time::timeout(Duration::from_secs(2), get(app.clone(), "/readyz"))
        .await
        .expect("readiness has a bounded response time");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = read_json(response).await;
    assert_eq!(body["status"], "unavailable");
    assert_eq!(body["checks"]["delivery_core"]["status"], "ok");
    assert_eq!(body["checks"]["durable_store"]["status"], "failed");

    // Liveness and the client/server version handshake stay available while
    // readiness truthfully reports that chat writes cannot be committed.
    let response = get(app, "/health").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: HealthResponse = read_json(response).await;
    assert_eq!(body.status, "ok");

    lock.execute_batch("ROLLBACK").expect("release SQLite lock");
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

async fn get(app: Router, uri: &str) -> Response<Body> {
    app.oneshot(
        Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
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
