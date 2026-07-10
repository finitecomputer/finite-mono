use axum::body::Body;
use axum::http::{Request, StatusCode};
use finite_identity::{FiniteIdentity, IdentityPaths};
use finitechat_core::{AppAction, FiniteChatRuntime, OpenOptions, npub_from_account_id};
use finitechat_hosted_device::{
    HostedDeviceConfig, MAX_HOSTED_ATTACHMENT_BYTES, MAX_HOSTED_ATTACHMENTS_PER_MESSAGE,
    MAX_HOSTED_MULTIPART_BODY_BYTES, WORKOS_USER_HEADER, app,
};
use finitechat_server::{HttpServerState, http_router};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use serde_json::Value;
use std::net::SocketAddr;
use std::path::Path;
use tempfile::TempDir;
use tower::ServiceExt;

const TOKEN: &str = "hosted-device-test-token";

#[tokio::test]
async fn state_requires_internal_authorization_and_verified_user() {
    let root = TempDir::new().unwrap();
    let app = test_app(&root);

    let response = app
        .clone()
        .oneshot(Request::get("/v1/app/state").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::get("/v1/app/state")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Upload authentication is checked before multipart parsing or buffering.
    let response = test_app(&root)
        .oneshot(
            Request::post("/v1/app/attachments")
                .body(Body::from("not multipart"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn users_get_isolated_devices_and_restart_reopens_the_same_identity() {
    let root = TempDir::new().unwrap();
    let first_app = test_app(&root);
    let paul = state_for(first_app.clone(), "user_paul").await;
    let alice = state_for(first_app, "user_alice").await;

    assert_ne!(
        paul["identity"]["account_id"],
        alice["identity"]["account_id"]
    );
    assert_eq!(paul["identity"]["device_id"], "hosted-web");
    assert_eq!(paul["identity"]["account_secret_hex"], "");

    let restarted_app = test_app(&root);
    let paul_after_restart = state_for(restarted_app, "user_paul").await;
    assert_eq!(
        paul["identity"]["account_id"],
        paul_after_restart["identity"]["account_id"]
    );
    assert_eq!(
        paul["identity"]["device_id"],
        paul_after_restart["identity"]["device_id"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_image_upload_returns_a_public_finitechat_blob_url() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("server.sqlite3"), None).await;
    let device = app(HostedDeviceConfig {
        data_root: root.path().join("hosted-devices"),
        server_url: server_url.clone(),
        api_token: TOKEN.to_owned(),
    });

    let response = device
        .oneshot(
            Request::post("/v1/app/images")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, "user_paul")
                .header("content-type", "image/png")
                .body(Body::from(b"\x89PNG\r\n\x1a\nprofile".to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let image_url = json["image_url"].as_str().unwrap();
    assert!(image_url.starts_with(&format!("{server_url}/blobs/")));

    server_task.abort();
    let _ = server_task.await;
}

#[tokio::test]
async fn update_stream_flushes_current_state_without_waiting_for_remote_activity() {
    let root = TempDir::new().unwrap();
    let response = test_app(&root)
        .oneshot(
            Request::get("/v1/app/updates")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, "user_paul")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let mut body = response.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(1), body.next())
        .await
        .expect("initial SSE state must be flushed immediately")
        .expect("SSE stream must yield an initial frame")
        .expect("initial SSE frame must be readable");
    let first = String::from_utf8(first.to_vec()).unwrap();
    assert!(first.contains("event: state"), "{first:?}");
    assert!(first.contains("data: {"), "{first:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosted_device_chats_with_an_agent_and_restarts_with_the_transcript() {
    let root = TempDir::new().unwrap();
    let server_db = root.path().join("server.sqlite3");
    let (server_url, server_address, server_task) = spawn_chat_server(&server_db, None).await;
    let agent_identity = FiniteIdentity::load_or_generate(
        &IdentityPaths::with_finite_home(root.path().join("agent-finite-home")),
        "finitechat-hosted-device-test/agent",
    )
    .unwrap();
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root.path().join("agent-chat").display().to_string(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: Some(hex::encode(agent_identity.expose_secret_bytes())),
        now_unix_seconds: None,
    })
    .unwrap();
    let agent_state = agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    let agent_account_id = agent_state.identity.account_id;
    let agent_npub = npub_from_account_id(agent_account_id.clone()).unwrap();

    let config = HostedDeviceConfig {
        data_root: root.path().join("hosted-devices"),
        server_url,
        api_token: TOKEN.to_owned(),
    };
    let first_app = app(config.clone());
    let first_state = state_for(first_app.clone(), "user_paul").await;
    action_for(
        first_app.clone(),
        "user_paul",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;
    let connected = action_for(
        first_app.clone(),
        "user_paul",
        serde_json::json!({
            "StartProfileChat": {
                "profile": {
                    "account_id": agent_account_id,
                    "npub": agent_npub,
                    "display_name": "Test Agent",
                    "about": "A test agent",
                    "picture": null,
                    "stale": false,
                    "is_agent": true
                },
                "display_name": "Chat with Test Agent"
            }
        }),
    )
    .await;
    let room_id = connected["rooms"][0]["room_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(connected["rooms"][0]["state"], "Connected");
    assert_eq!(connected["rooms"][0]["is_agent_chat"], true);

    agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    action_for(
        first_app.clone(),
        "user_paul",
        serde_json::json!({
            "SendMessage": { "room_id": room_id, "text": "hello from the web" }
        }),
    )
    .await;
    let agent_after_message = agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    assert!(
        agent_after_message
            .messages
            .iter()
            .any(|message| message.text == "hello from the web")
    );
    agent
        .dispatch_and_wait(AppAction::SendMessage {
            room_id: room_id.clone(),
            text: "hello from the agent".to_owned(),
        })
        .unwrap();
    let replied = action_for(
        first_app.clone(),
        "user_paul",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;
    assert!(
        replied["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["text"] == "hello from the agent")
    );

    server_task.abort();
    let _ = server_task.await;
    let (restarted_server_url, _, restarted_server_task) =
        spawn_chat_server(&server_db, Some(server_address)).await;
    assert_eq!(config.server_url, restarted_server_url);
    action_for(
        first_app.clone(),
        "user_paul",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;
    action_for(
        first_app.clone(),
        "user_paul",
        serde_json::json!({
            "SendMessage": { "room_id": room_id, "text": "after chat server restart" }
        }),
    )
    .await;
    let agent_after_server_restart = agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    assert!(
        agent_after_server_restart
            .messages
            .iter()
            .any(|message| message.text == "after chat server restart")
    );

    drop(first_app);
    let restarted = state_for(app(config), "user_paul").await;
    assert_eq!(
        first_state["identity"]["account_id"],
        restarted["identity"]["account_id"]
    );
    assert!(
        restarted["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["text"] == "hello from the agent")
    );
    restarted_server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attachment_bytes_are_isolated_redacted_and_survive_device_restart() {
    let root = TempDir::new().unwrap();
    let server_db = root.path().join("attachment-server.sqlite3");
    let (server_url, _, server_task) = spawn_chat_server(&server_db, None).await;
    let config = HostedDeviceConfig {
        data_root: root.path().join("hosted-devices"),
        server_url,
        api_token: TOKEN.to_owned(),
    };
    let first_app = app(config.clone());
    action_for(
        first_app.clone(),
        "user_paul",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;
    let room = action_for(
        first_app.clone(),
        "user_paul",
        serde_json::json!({ "CreateRoom": { "display_name": "Attachment test" } }),
    )
    .await;
    let room_id = room["selected_room_id"].as_str().unwrap().to_owned();
    let topic_id = room["selected_topic_id"].as_str().unwrap().to_owned();
    let chat_id = room["selected_chat_id"].as_str().unwrap().to_owned();
    let plaintext = b"not actually a png, but exactly the bytes the user selected".to_vec();
    let files = vec![MultipartFile {
        filename: "preview.png".to_owned(),
        content_type: "image/png".to_owned(),
        bytes: plaintext.clone(),
    }];
    let response = upload_for(
        first_app.clone(),
        "user_paul",
        &[
            ("room_id", room_id.as_str()),
            ("topic_id", topic_id.as_str()),
            ("chat_id", chat_id.as_str()),
            ("caption", "A browser attachment"),
        ],
        &files,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let uploaded: Value = serde_json::from_slice(&bytes).unwrap();
    let message = uploaded["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| {
            message["media"]
                .as_array()
                .is_some_and(|media| !media.is_empty())
        })
        .unwrap();
    let message_id = message["message_id"].as_str().unwrap().to_owned();
    let attachment_id = message["media"][0]["attachment_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(message["media"][0]["local_path"], Value::Null);
    let gallery_item = uploaded["media_gallery"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["attachment_id"] == attachment_id)
        .unwrap();
    assert_eq!(gallery_item["attachment"]["local_path"], Value::Null);

    let state = state_for(first_app.clone(), "user_paul").await;
    assert!(state["messages"].as_array().unwrap().iter().all(|message| {
        message["media"]
            .as_array()
            .unwrap()
            .iter()
            .all(|attachment| attachment["local_path"].is_null())
    }));

    let download = download_for(
        first_app.clone(),
        "user_paul",
        &room_id,
        &message_id,
        &attachment_id,
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.headers()["content-type"], "image/png");
    assert_eq!(
        download.headers()["content-disposition"],
        "inline; filename=\"preview.png\""
    );
    assert_eq!(download.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        download.into_body().collect().await.unwrap().to_bytes(),
        plaintext.as_slice()
    );

    let isolated = download_for(
        first_app.clone(),
        "user_alice",
        &room_id,
        &message_id,
        &attachment_id,
    )
    .await;
    assert_eq!(isolated.status(), StatusCode::NOT_FOUND);

    drop(first_app);
    let restarted = app(config);
    let after_restart = download_for(
        restarted,
        "user_paul",
        &room_id,
        &message_id,
        &attachment_id,
    )
    .await;
    assert_eq!(after_restart.status(), StatusCode::OK);
    assert_eq!(
        after_restart
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
        plaintext.as_slice()
    );
    server_task.abort();
}

#[tokio::test]
async fn attachment_upload_enforces_count_file_and_request_limits() {
    let root = TempDir::new().unwrap();
    let app = test_app(&root);

    let too_many = (0..=MAX_HOSTED_ATTACHMENTS_PER_MESSAGE)
        .map(|index| MultipartFile {
            filename: format!("file-{index}.txt"),
            content_type: "text/plain".to_owned(),
            bytes: vec![b'x'],
        })
        .collect::<Vec<_>>();
    let response = upload_for(
        app.clone(),
        "user_paul",
        &[("room_id", "room-test")],
        &too_many,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let oversized = vec![MultipartFile {
        filename: "too-large.bin".to_owned(),
        content_type: "application/octet-stream".to_owned(),
        bytes: vec![0; MAX_HOSTED_ATTACHMENT_BYTES + 1],
    }];
    let response = upload_for(
        app.clone(),
        "user_paul",
        &[("room_id", "room-test")],
        &oversized,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let tiny = vec![MultipartFile {
        filename: "tiny.txt".to_owned(),
        content_type: "text/plain".to_owned(),
        bytes: vec![b'x'],
    }];
    let response = upload_for(
        app,
        "user_paul",
        &[("room_id", "room-test")],
        &tiny,
        Some(MAX_HOSTED_MULTIPART_BODY_BYTES + 1),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

fn test_app(root: &TempDir) -> axum::Router {
    app(HostedDeviceConfig {
        data_root: root.path().to_path_buf(),
        server_url: "http://127.0.0.1:9".to_owned(),
        api_token: TOKEN.to_owned(),
    })
}

async fn state_for(app: axum::Router, user_id: &str) -> Value {
    let response = app
        .oneshot(
            Request::get("/v1/app/state")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, user_id)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn action_for(app: axum::Router, user_id: &str, action: Value) -> Value {
    let response = app
        .oneshot(
            Request::post("/v1/app/actions")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, user_id)
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

struct MultipartFile {
    filename: String,
    content_type: String,
    bytes: Vec<u8>,
}

async fn upload_for(
    app: axum::Router,
    user_id: &str,
    fields: &[(&str, &str)],
    files: &[MultipartFile],
    declared_content_length: Option<usize>,
) -> axum::response::Response {
    let boundary = "finitechat-hosted-device-test-boundary";
    let body = multipart_body(boundary, fields, files);
    let mut request = Request::post("/v1/app/attachments")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header(WORKOS_USER_HEADER, user_id)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        );
    if let Some(content_length) = declared_content_length {
        request = request.header("content-length", content_length);
    }
    app.oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

fn multipart_body(boundary: &str, fields: &[(&str, &str)], files: &[MultipartFile]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    for file in files {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"files\"; filename=\"{}\"\r\n",
                file.filename
            )
            .as_bytes(),
        );
        body.extend_from_slice(format!("Content-Type: {}\r\n\r\n", file.content_type).as_bytes());
        body.extend_from_slice(&file.bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

async fn download_for(
    app: axum::Router,
    user_id: &str,
    room_id: &str,
    message_id: &str,
    attachment_id: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::get(format!(
            "/v1/app/attachments/{room_id}/{message_id}/{attachment_id}"
        ))
        .header("authorization", format!("Bearer {TOKEN}"))
        .header(WORKOS_USER_HEADER, user_id)
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap()
}

async fn spawn_chat_server(
    database: &Path,
    address: Option<SocketAddr>,
) -> (String, SocketAddr, tokio::task::JoinHandle<()>) {
    let state = HttpServerState::from_sqlite_path(database).unwrap();
    let listener =
        tokio::net::TcpListener::bind(address.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap()))
            .await
            .unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, http_router(state)).await.unwrap();
    });
    (format!("http://{address}"), address, task)
}
