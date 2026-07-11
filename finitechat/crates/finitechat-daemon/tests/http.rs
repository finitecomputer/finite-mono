use std::net::SocketAddr;
use std::path::Path;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use finitechat_core::{AppAction, AppState, FiniteChatRuntime, OpenOptions};
use finitechat_daemon::app;
use finitechat_server::{HttpServerState, http_router};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const WRONG_TOKEN: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
const ACCOUNT_SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000003";

#[tokio::test]
async fn every_route_rejects_missing_and_wrong_authorization() {
    let root = TempDir::new().unwrap();
    let daemon = test_app(&root, "http://127.0.0.1:9");
    let requests = [
        Request::get("/v1/healthz").body(Body::empty()).unwrap(),
        Request::get("/v1/app/state").body(Body::empty()).unwrap(),
        Request::get("/v1/app/updates").body(Body::empty()).unwrap(),
        Request::post("/v1/app/actions")
            .header("content-type", "application/json")
            .body(Body::from("not-json"))
            .unwrap(),
    ];

    for request in requests {
        let response = daemon.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    for uri in ["/v1/healthz", "/v1/app/state", "/v1/app/updates"] {
        let response = daemon
            .clone()
            .oneshot(
                Request::get(uri)
                    .header("authorization", format!("Bearer {WRONG_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn authorized_state_is_redacted_and_updates_flush_immediately() {
    let root = TempDir::new().unwrap();
    let daemon = test_app(&root, "http://127.0.0.1:9");

    let health = request(daemon.clone(), Request::get("/v1/healthz")).await;
    assert_eq!(health.status(), StatusCode::OK);

    let state = request(daemon.clone(), Request::get("/v1/app/state")).await;
    assert_eq!(state.status(), StatusCode::OK);
    let state: AppState = read_json(state).await;
    assert_eq!(state.identity.device_id, "electron-test");
    assert!(state.identity.account_secret_hex.is_empty());

    let updates = request(daemon, Request::get("/v1/app/updates")).await;
    assert_eq!(updates.status(), StatusCode::OK);
    let mut body = updates.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(1), body.next())
        .await
        .expect("initial state must not wait for a remote update")
        .expect("SSE stream must contain an initial frame")
        .expect("initial frame must be readable");
    let first = String::from_utf8(first.to_vec()).unwrap();
    assert!(first.contains("event: state"), "{first:?}");
    assert!(first.contains("data: {"), "{first:?}");
    assert!(
        !first.contains(ACCOUNT_SECRET),
        "secret leaked in SSE frame"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_actions_and_restart_reopen_the_same_selected_chat() {
    let root = TempDir::new().unwrap();
    let (server_url, server_task) = spawn_chat_server(&root.path().join("server.sqlite3")).await;
    let data_dir = root.path().join("electron-store");

    let first_runtime = open_runtime(&data_dir, &server_url);
    let first = app(first_runtime, TOKEN).unwrap();
    let created: AppState = action(
        first.clone(),
        AppAction::CreateRoom {
            display_name: "Device Parity".to_owned(),
        },
    )
    .await;
    let room_id = created.selected_room_id.expect("created room selected");
    let topic: AppState = action(
        first.clone(),
        AppAction::CreateTopic {
            room_id: room_id.clone(),
            title: "Alpha".to_owned(),
        },
    )
    .await;
    let topic_id = topic.selected_topic_id.expect("created topic selected");
    let chat_id = topic.selected_chat_id.expect("created chat selected");
    let sent: AppState = action(
        first,
        AppAction::SendChatMessage {
            room_id: room_id.clone(),
            topic_id: topic_id.clone(),
            chat_id: chat_id.clone(),
            text: "survives an Electron daemon restart".to_owned(),
        },
    )
    .await;
    let before_identity = sent.identity.clone();
    assert!(
        sent.messages
            .iter()
            .any(|message| message.text == "survives an Electron daemon restart")
    );
    drop(sent);

    let restarted = app(open_runtime(&data_dir, &server_url), TOKEN).unwrap();
    let state_response = request(restarted, Request::get("/v1/app/state")).await;
    let after: AppState = read_json(state_response).await;
    assert_eq!(after.identity.account_id, before_identity.account_id);
    assert_eq!(after.identity.device_id, before_identity.device_id);
    assert_eq!(after.selected_room_id.as_deref(), Some(room_id.as_str()));
    assert_eq!(after.selected_topic_id.as_deref(), Some(topic_id.as_str()));
    assert_eq!(after.selected_chat_id.as_deref(), Some(chat_id.as_str()));
    assert!(
        after
            .messages
            .iter()
            .any(|message| message.text == "survives an Electron daemon restart")
    );

    server_task.abort();
    let _ = server_task.await;
}

fn test_app(root: &TempDir, server_url: &str) -> axum::Router {
    app(open_runtime(&root.path().join("device"), server_url), TOKEN).unwrap()
}

fn open_runtime(data_dir: &Path, server_url: &str) -> std::sync::Arc<FiniteChatRuntime> {
    FiniteChatRuntime::open(OpenOptions {
        data_dir: data_dir.display().to_string(),
        server_url: server_url.to_owned(),
        device_id: "electron-test".to_owned(),
        account_secret_hex: Some(ACCOUNT_SECRET.to_owned()),
        now_unix_seconds: Some(1_900_000_000),
    })
    .unwrap()
}

async fn request(
    daemon: axum::Router,
    request: axum::http::request::Builder,
) -> axum::response::Response {
    daemon
        .oneshot(
            request
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn action(daemon: axum::Router, action: AppAction) -> AppState {
    let response = daemon
        .oneshot(
            Request::post("/v1/app/actions")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&action).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

async fn read_json<T: serde::de::DeserializeOwned>(response: axum::response::Response) -> T {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn spawn_chat_server(database: &Path) -> (String, tokio::task::JoinHandle<()>) {
    let state = HttpServerState::from_sqlite_path(database).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, http_router(state)).await.unwrap();
    });
    (format!("http://{address}"), task)
}
