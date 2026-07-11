use axum::body::Body;
use axum::http::{Request, StatusCode};
use finite_identity::{FiniteIdentity, IdentityPaths};
use finitechat_core::device_link::{
    DEVICE_LINK_MAX_TTL_SECONDS, DeviceLinkDecryptInput, create_device_link_pairing_key,
    decrypt_device_link_payload,
};
use finitechat_core::{AppAction, FiniteChatRuntime, OpenOptions, npub_from_account_id};
use finitechat_hosted_device::{
    HostedDeviceConfig, MAX_HOSTED_ATTACHMENT_BYTES, MAX_HOSTED_ATTACHMENTS_PER_MESSAGE,
    MAX_HOSTED_MULTIPART_BODY_BYTES, WORKOS_USER_HEADER, app, app_with_fixed_device_link_now,
};
use finitechat_http::{
    AckLinkPayloadRequest, AckLinkPayloadResponse, ClaimLinkPayloadRequest,
    ClaimLinkPayloadResponse, CreateLinkSessionRequest, GetLinkSessionRequest,
    HttpLinkSessionRecord, HttpLinkSessionState,
};
use finitechat_server::{HttpServerState, http_router};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::Digest;
use std::convert::Infallible;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
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

    for path in ["/v1/device-links/approve", "/v1/device-links/status"] {
        let response = test_app(&root)
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"link_session_id":"link-a","target_device_id":"electron-a"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let unauthorized_malformed = test_app(&root)
        .oneshot(
            Request::post("/v1/device-links/approve")
                .header("content-type", "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized_malformed.status(), StatusCode::UNAUTHORIZED);

    let oversized = test_app(&root)
        .oneshot(
            Request::post("/v1/device-links/approve")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, "user_paul")
                .header("content-type", "application/json")
                .body(Body::from(vec![b'x'; 4 * 1024 + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workos_approved_device_link_is_isolated_durable_and_fans_out() {
    let root = TempDir::new().unwrap();
    let device_link_now = test_now_unix_seconds();
    let server_db = root.path().join("device-link-server.sqlite3");
    let (server_url, _, server_task) = spawn_chat_server(&server_db, None).await;
    let config = HostedDeviceConfig {
        data_root: root.path().join("hosted-devices"),
        server_url: server_url.clone(),
        api_token: TOKEN.to_owned(),
    };
    let hosted = app_with_fixed_device_link_now(config.clone(), device_link_now);
    action_for(
        hosted.clone(),
        "user_paul",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;
    let room = action_for(
        hosted.clone(),
        "user_paul",
        serde_json::json!({ "CreateRoom": { "display_name": "Device parity" } }),
    )
    .await;
    let room_id = room["selected_room_id"].as_str().unwrap().to_owned();

    let pairing = create_device_link_pairing_key();
    let link_session_id = "link-workos-paul";
    let target_device_id = "electron-paul-alpha";
    let created: HttpLinkSessionRecord = chat_post(
        &server_url,
        "/link-sessions",
        &CreateLinkSessionRequest {
            link_session_id: link_session_id.to_owned(),
            pairing_public_key: pairing.public_key_hex.clone(),
        },
    )
    .await;
    assert_eq!(created.state, HttpLinkSessionState::Created);

    let approved = device_link_for(
        hosted.clone(),
        "user_paul",
        "/v1/device-links/approve",
        link_session_id,
        target_device_id,
    )
    .await;
    assert_eq!(approved.status(), StatusCode::OK);
    let approved_body = approved.into_body().collect().await.unwrap().to_bytes();
    let approved_text = String::from_utf8(approved_body.to_vec()).unwrap();
    let approved_json: Value = serde_json::from_str(&approved_text).unwrap();
    assert_eq!(approved_json["status"], "awaiting_claim");
    for forbidden in [
        "account_secret",
        "nsec",
        "encrypted_payload",
        "pairing_public_key",
    ] {
        assert!(
            !approved_text.contains(forbidden),
            "response leaked {forbidden}"
        );
    }

    let uploaded: Option<HttpLinkSessionRecord> = chat_post(
        &server_url,
        "/link-sessions/get",
        &GetLinkSessionRequest {
            link_session_id: link_session_id.to_owned(),
        },
    )
    .await;
    let uploaded = uploaded.unwrap();
    assert_eq!(uploaded.state, HttpLinkSessionState::PayloadUploaded);
    let encrypted_payload = uploaded.encrypted_payload.clone().unwrap();
    let pairing_secret_key_hex = pairing.secret_key_hex.clone();
    let payload = decrypt_device_link_payload(DeviceLinkDecryptInput {
        pairing_secret_key_hex: pairing_secret_key_hex.clone(),
        encrypted_payload: encrypted_payload.clone(),
        expected_link_session_id: link_session_id.to_owned(),
        expected_pairing_public_key: pairing.public_key_hex,
        expected_target_device_id: target_device_id.to_owned(),
        expected_server_url: server_url.clone(),
        now_unix_seconds: device_link_now + 1,
    })
    .unwrap();
    assert_eq!(payload.target_device_id, target_device_id);

    let persisted_path = config
        .data_root
        .join("users")
        .join(hex::encode(sha2::Sha256::digest(b"user_paul")))
        .join("device-links")
        .join(format!(
            "{}.json",
            hex::encode(sha2::Sha256::digest(link_session_id.as_bytes()))
        ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&persisted_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let persisted = fs::read_to_string(&persisted_path).unwrap();
    assert!(!persisted.contains(&payload.account_secret_hex));
    assert!(!persisted.contains(&pairing_secret_key_hex));

    let claimed: ClaimLinkPayloadResponse = chat_post(
        &server_url,
        "/link-sessions/claim",
        &ClaimLinkPayloadRequest {
            link_session_id: link_session_id.to_owned(),
        },
    )
    .await;
    assert_eq!(claimed.encrypted_payload, encrypted_payload);

    let electron = FiniteChatRuntime::open(OpenOptions {
        data_dir: root.path().join("electron").display().to_string(),
        server_url: server_url.clone(),
        device_id: target_device_id.to_owned(),
        account_secret_hex: Some(payload.account_secret_hex),
        now_unix_seconds: Some(device_link_now),
    })
    .unwrap();
    electron
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("linked Electron Device publishes KeyPackages");
    let acked: AckLinkPayloadResponse = chat_post(
        &server_url,
        "/link-sessions/ack",
        &AckLinkPayloadRequest {
            link_session_id: link_session_id.to_owned(),
            claim_token: claimed.claim_token,
        },
    )
    .await;
    assert!(acked.acked);

    let joining = device_link_json(
        hosted.clone(),
        "user_paul",
        "/v1/device-links/status",
        link_session_id,
        target_device_id,
    )
    .await;
    assert_eq!(joining["status"], "joining_rooms");
    assert_eq!(joining["room_count"], 1);
    electron
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("linked Electron Device activates its Welcome");
    let electron_state = electron.state().unwrap();
    assert!(
        electron_state
            .rooms
            .iter()
            .any(|room| room.room_id == room_id)
    );

    let ready = device_link_json(
        hosted.clone(),
        "user_paul",
        "/v1/device-links/status",
        link_session_id,
        target_device_id,
    )
    .await;
    assert_eq!(ready["status"], "ready");
    assert_eq!(ready["active_room_count"], 1);

    let mut tampered: Value = serde_json::from_str(&persisted).unwrap();
    let first_byte = tampered["encrypted_payload"][0].as_u64().unwrap();
    tampered["encrypted_payload"][0] = Value::from(first_byte ^ 1);
    fs::write(&persisted_path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    let rejected_tamper = device_link_for(
        hosted.clone(),
        "user_paul",
        "/v1/device-links/status",
        link_session_id,
        target_device_id,
    )
    .await;
    assert_eq!(rejected_tamper.status(), StatusCode::CONFLICT);
    fs::write(&persisted_path, persisted).unwrap();

    let isolated = device_link_for(
        hosted.clone(),
        "user_alice",
        "/v1/device-links/status",
        link_session_id,
        target_device_id,
    )
    .await;
    assert_eq!(isolated.status(), StatusCode::NOT_FOUND);
    let substituted_target = device_link_for(
        hosted,
        "user_paul",
        "/v1/device-links/approve",
        link_session_id,
        "electron-other",
    )
    .await;
    assert_eq!(substituted_target.status(), StatusCode::NOT_FOUND);

    let restarted = app_with_fixed_device_link_now(config, device_link_now + 2);
    let resumed = device_link_json(
        restarted.clone(),
        "user_paul",
        "/v1/device-links/status",
        link_session_id,
        target_device_id,
    )
    .await;
    assert_eq!(resumed["status"], "ready");
    let repeated = device_link_json(
        restarted,
        "user_paul",
        "/v1/device-links/approve",
        link_session_id,
        target_device_id,
    )
    .await;
    assert_eq!(repeated["status"], "ready");

    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_device_link_is_closed_and_stays_expired_after_restart() {
    let root = TempDir::new().unwrap();
    let device_link_now = test_now_unix_seconds();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("expiry-server.sqlite3"), None).await;
    let config = HostedDeviceConfig {
        data_root: root.path().join("hosted-devices"),
        server_url: server_url.clone(),
        api_token: TOKEN.to_owned(),
    };
    let pairing = create_device_link_pairing_key();
    let link_session_id = "link-expiry-test";
    let _: HttpLinkSessionRecord = chat_post(
        &server_url,
        "/link-sessions",
        &CreateLinkSessionRequest {
            link_session_id: link_session_id.to_owned(),
            pairing_public_key: pairing.public_key_hex,
        },
    )
    .await;
    let current = app_with_fixed_device_link_now(config.clone(), device_link_now);
    let approved = device_link_json(
        current,
        "user_paul",
        "/v1/device-links/approve",
        link_session_id,
        "electron-expiry-test",
    )
    .await;
    assert_eq!(approved["status"], "awaiting_claim");

    let expired =
        app_with_fixed_device_link_now(config, device_link_now + DEVICE_LINK_MAX_TTL_SECONDS + 1);
    for _ in 0..2 {
        let status = device_link_json(
            expired.clone(),
            "user_paul",
            "/v1/device-links/status",
            link_session_id,
            "electron-expiry-test",
        )
        .await;
        assert_eq!(status["status"], "expired");
    }
    let server_record: Option<HttpLinkSessionRecord> = chat_post(
        &server_url,
        "/link-sessions/get",
        &GetLinkSessionRequest {
            link_session_id: link_session_id.to_owned(),
        },
    )
    .await;
    assert_eq!(server_record.unwrap().state, HttpLinkSessionState::Expired);
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_chunked_link_service_response_is_rejected() {
    let root = TempDir::new().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let fake = axum::Router::new().route(
        "/link-sessions/get",
        axum::routing::post(|| async {
            let stream = futures_util::stream::once(async {
                Ok::<_, Infallible>(axum::body::Bytes::from(vec![b'x'; 65 * 1024]))
            });
            axum::response::Response::new(Body::from_stream(stream))
        }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
    let device = app(HostedDeviceConfig {
        data_root: root.path().join("hosted-devices"),
        server_url: format!("http://{address}"),
        api_token: TOKEN.to_owned(),
    });
    let response = device_link_for(
        device,
        "user_paul",
        "/v1/device-links/approve",
        "link-oversized-service",
        "electron-oversized-service",
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.len() < 1_024);
    assert!(String::from_utf8_lossy(&body).contains("response is too large"));
    task.abort();
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

async fn device_link_for(
    app: axum::Router,
    user_id: &str,
    path: &str,
    link_session_id: &str,
    target_device_id: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::post(path)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header(WORKOS_USER_HEADER, user_id)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "link_session_id": link_session_id,
                    "target_device_id": target_device_id,
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn device_link_json(
    app: axum::Router,
    user_id: &str,
    path: &str,
    link_session_id: &str,
    target_device_id: &str,
) -> Value {
    let response = device_link_for(app, user_id, path, link_session_id, target_device_id).await;
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).unwrap()
}

async fn chat_post<I: Serialize, O: DeserializeOwned>(
    server_url: &str,
    path: &str,
    input: &I,
) -> O {
    let response = reqwest::Client::new()
        .post(format!("{server_url}{path}"))
        .json(input)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json().await.unwrap()
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

fn test_now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
