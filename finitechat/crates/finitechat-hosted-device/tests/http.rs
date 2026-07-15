use axum::body::Body;
use axum::http::{Request, StatusCode};
use finite_brain_core::{BRAIN_IDENTITY_PROVIDER_VERSION, FolderKey};
use finite_identity::{FiniteIdentity, IdentityPaths};
use finite_nostr::verify_event_integrity;
use finitechat_core::device_link::{
    DEVICE_LINK_MAX_TTL_SECONDS, DeviceLinkDecryptInput, create_device_link_pairing_key,
    decrypt_device_link_payload,
};
use finitechat_core::{AppAction, FiniteChatRuntime, OpenOptions, npub_from_account_id};
use finitechat_hosted_device::{
    HostedDeviceConfig, MAX_HOSTED_ATTACHMENT_BYTES, MAX_HOSTED_ATTACHMENTS_PER_MESSAGE,
    MAX_HOSTED_MULTIPART_BODY_BYTES, WORKOS_USER_HEADER, app,
    app_with_final_agent_binding_persist_failures, app_with_fixed_device_link_now,
    app_with_profile_bootstrap_room_create_failures, app_with_profile_bootstrap_submit_failures,
};
use finitechat_http::{
    AckLinkPayloadRequest, AckLinkPayloadResponse, ClaimLinkPayloadRequest,
    ClaimLinkPayloadResponse, CreateLinkSessionRequest, GetLinkSessionRequest,
    HttpLinkSessionRecord, HttpLinkSessionState,
};
use finitechat_proto::{
    DecryptedApplicationEventV1, DurableAppEventKind, RuntimeCommandJsonPayloadV1,
    RuntimeCommandPayloadKindV1, RuntimeCommandRequestV1, RuntimeCommandResultV1,
    RuntimeCommandTerminalStatusV1,
};
use finitechat_server::{HttpServerState, http_router};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use nostr::Event;
use openmls::prelude::{AeadType, OpenMlsCrypto, OpenMlsProvider, OpenMlsRand};
use openmls_rust_crypto::OpenMlsRustCrypto;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
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
const PUBLIC_SERVER_URL: &str = "https://chat.finite.computer";
const TEST_AGENT_BINDING_KEY_DOMAIN: &[u8] = b"finitechat.hosted-agent-binding-key.v1";
const TEST_AGENT_BINDING_AAD_DOMAIN: &[u8] = b"finitechat.hosted-agent-binding.v1";

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
async fn device_link_uses_internal_transport_but_binds_the_public_server_url() {
    let root = TempDir::new().unwrap();
    let device_link_now = test_now_unix_seconds();
    let server_db = root.path().join("device-link-server.sqlite3");
    let (server_url, _, server_task) = spawn_chat_server(&server_db, None).await;
    assert_ne!(server_url.as_str(), PUBLIC_SERVER_URL);
    let config = HostedDeviceConfig {
        data_root: root.path().join("hosted-devices"),
        server_url: server_url.clone(),
        public_url: PUBLIC_SERVER_URL.to_owned(),
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
    let rejected_internal_url = decrypt_device_link_payload(DeviceLinkDecryptInput {
        pairing_secret_key_hex: pairing_secret_key_hex.clone(),
        encrypted_payload: encrypted_payload.clone(),
        expected_link_session_id: link_session_id.to_owned(),
        expected_pairing_public_key: pairing.public_key_hex.clone(),
        expected_target_device_id: target_device_id.to_owned(),
        expected_server_url: server_url.clone(),
        now_unix_seconds: device_link_now + 1,
    });
    assert!(
        rejected_internal_url.is_err(),
        "transport URL must not satisfy the encrypted public server binding"
    );
    let payload = decrypt_device_link_payload(DeviceLinkDecryptInput {
        pairing_secret_key_hex: pairing_secret_key_hex.clone(),
        encrypted_payload: encrypted_payload.clone(),
        expected_link_session_id: link_session_id.to_owned(),
        expected_pairing_public_key: pairing.public_key_hex,
        expected_target_device_id: target_device_id.to_owned(),
        expected_server_url: PUBLIC_SERVER_URL.to_owned(),
        now_unix_seconds: device_link_now + 1,
    })
    .unwrap();
    assert_eq!(payload.target_device_id, target_device_id);
    assert_eq!(payload.server_url, PUBLIC_SERVER_URL);

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
        public_url: server_url.clone(),
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
        public_url: PUBLIC_SERVER_URL.to_owned(),
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

    let paul_store = root
        .path()
        .join("users")
        .join(hex::encode(sha2::Sha256::digest(b"user_paul")))
        .join("chat/client.sqlite3");
    let alice_store = root
        .path()
        .join("users")
        .join(hex::encode(sha2::Sha256::digest(b"user_alice")))
        .join("chat/client.sqlite3");
    assert!(paul_store.is_file());
    assert!(alice_store.is_file());
    assert_ne!(paul_store, alice_store);

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

#[tokio::test]
async fn brain_bootstrap_authorization_is_not_exposed_as_a_direct_hosted_api() {
    let root = TempDir::new().unwrap();
    let hosted = test_app(&root);
    let response = hosted
        .oneshot(
            Request::post("/v1/brain/personal-vault-bootstrap-authorizations")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, "user_paul")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "agentNpub": "npub-not-authorized-by-chat",
                        "vaultId": "personal",
                        "workspaceFolderId": "agent-workspace",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_user_brain_setup_command_delivers_the_bounded_bundle_to_the_agent() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("brain-command-server.sqlite3"), None).await;
    let agent_identity = FiniteIdentity::load_or_generate(
        &IdentityPaths::with_finite_home(root.path().join("brain-command-agent")),
        "finitechat-hosted-device-test/brain-command-agent",
    )
    .unwrap();
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root.path().join("brain-command-chat").display().to_string(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: Some(hex::encode(agent_identity.expose_secret_bytes())),
        now_unix_seconds: None,
    })
    .unwrap();
    let agent_state = agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    let agent_account_id = agent_state.identity.account_id;
    let agent_npub = npub_from_account_id(agent_account_id.clone()).unwrap();
    let hosted = app(HostedDeviceConfig {
        data_root: root.path().join("brain-command-hosted"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    });
    let user = state_for(hosted.clone(), "user_paul").await;
    let user_account_id = user["identity"]["account_id"].as_str().unwrap().to_owned();
    let owner_npub = npub_from_account_id(user_account_id.clone()).unwrap();
    action_for(
        hosted.clone(),
        "user_paul",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;
    let connected = action_for(
        hosted.clone(),
        "user_paul",
        serde_json::json!({
            "StartProfileChat": {
                "profile": {
                    "account_id": agent_account_id,
                    "npub": agent_npub,
                    "display_name": "Brain Agent",
                    "about": "Requests bounded Brain setup",
                    "picture": null,
                    "stale": false,
                    "is_agent": true
                },
                "display_name": "Chat with Brain Agent"
            }
        }),
    )
    .await;
    let room_id = connected["selected_room_id"].as_str().unwrap().to_owned();
    agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    action_for(
        hosted.clone(),
        "user_paul",
        serde_json::json!({
            "SendMessage": {
                "room_id": room_id.clone(),
                "text": "hello agent"
            }
        }),
    )
    .await;
    let ordinary = agent.agent_bridge_poll_once().unwrap();
    assert!(!ordinary.events.iter().any(|stored| {
        serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext)
            .is_ok_and(|event| event.kind == DurableAppEventKind::RuntimeCommandRequest)
    }));
    action_for(
        hosted,
        "user_paul",
        serde_json::json!({
            "SendChatMessage": {
                "room_id": room_id,
                "topic_id": "home",
                "chat_id": "home-chat",
                "text": "/brain setup"
            }
        }),
    )
    .await;

    let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let bridge = agent.agent_bridge_poll_once().unwrap();
            if let Some(request) = bridge.events.into_iter().find_map(|stored| {
                let event =
                    serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext)
                        .ok()?;
                if event.kind != DurableAppEventKind::RuntimeCommandRequest {
                    return None;
                }
                let request =
                    serde_json::from_slice::<RuntimeCommandRequestV1>(&event.payload).ok()?;
                (request.command == "brain.personal-vault-bootstrap.authorize").then_some(request)
            }) {
                break request;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Hosted Device must deliver the Brain bootstrap bundle");
    assert_eq!(request.target.account_id, agent_account_id);
    assert_eq!(
        request.body.schema,
        "finitebrain-personal-vault-bootstrap-authorization-v1"
    );
    let bundle: Value = serde_json::from_slice(&request.body.json_payload).unwrap();
    assert_eq!(bundle["ownerNpub"], owner_npub);
    assert_eq!(bundle["agentNpub"], agent_npub);
    assert_eq!(
        bundle["vaultId"],
        format!("personal-{}", &user_account_id[..16])
    );
    assert_eq!(bundle["workspaceFolderId"], "agent-workspace");

    server_task.abort();
}

#[tokio::test]
async fn hosted_brain_identity_provider_requires_chat_setup_and_accepts_only_brain_intents() {
    let root = TempDir::new().unwrap();
    let hosted = test_app(&root);
    let provider_path = "/v1/brain/identity-provider";
    let provider_request = |operation: &str, input: Value| {
        Request::post(provider_path)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header(WORKOS_USER_HEADER, "user_paul")
            .header("x-finite-brain-public-origin", "https://finite.computer")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "version": BRAIN_IDENTITY_PROVIDER_VERSION,
                    "operation": operation,
                    "input": input,
                })
                .to_string(),
            ))
            .unwrap()
    };

    let setup_required = hosted
        .clone()
        .oneshot(provider_request("identifyMember", Value::Null))
        .await
        .unwrap();
    assert_eq!(setup_required.status(), StatusCode::PRECONDITION_REQUIRED);

    state_for(hosted.clone(), "user_paul").await;
    let identify = hosted
        .clone()
        .oneshot(provider_request("identifyMember", Value::Null))
        .await
        .unwrap();
    assert_eq!(identify.status(), StatusCode::OK);
    let identify: Value =
        serde_json::from_slice(&identify.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let public_key_hex = identify["publicKeyHex"].as_str().unwrap();
    assert_eq!(public_key_hex.len(), 64);
    assert!(identify["npub"].as_str().unwrap().starts_with("npub1"));

    let now = test_now_unix_seconds();
    let protected_url = "https://finite.computer/_admin/vaults";
    let authorized = hosted
        .clone()
        .oneshot(provider_request(
            "authorizeHttpRequest",
            serde_json::json!({
                "method": "GET",
                "url": protected_url,
                "bodyText": "",
                "eventTemplate": {
                    "kind": 27235,
                    "created_at": now,
                    "tags": [
                        ["u", protected_url],
                        ["method", "GET"],
                        ["nonce", "ab".repeat(16)],
                    ],
                    "content": "",
                },
            }),
        ))
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    let event: Event =
        serde_json::from_slice(&authorized.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    verify_event_integrity(&event).unwrap();
    assert_eq!(event.pubkey.to_hex(), public_key_hex);

    let member_npub = identify["npub"].as_str().unwrap().to_owned();
    let access_change_content = format!(
        "{{\"version\":\"finite-vault-admin-access-change-v1\",\"vaultId\":\"personal\",\"changeId\":\"provider-access-change\",\"action\":\"add-member\",\"adminNpub\":\"{member_npub}\",\"targetNpub\":\"{member_npub}\",\"createdAt\":\"2026-07-13T12:00:00Z\"}}"
    );
    let access_change_input = serde_json::json!({
        "intent": "vault-access-change",
        "eventTemplate": {
            "kind": 30_078,
            "created_at": now,
            "tags": [
                ["d", "finite-vault-admin-access-change:personal:provider-access-change"],
                ["vault", "personal"],
                ["action", "add-member"],
                ["p", public_key_hex],
            ],
            "content": access_change_content,
        },
    });
    let access_change = hosted
        .clone()
        .oneshot(provider_request(
            "authorizeBrainEvent",
            access_change_input.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(access_change.status(), StatusCode::OK);
    let mut overbroad_access_change = access_change_input;
    overbroad_access_change["eventTemplate"]["tags"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!(["extra", "ambient-authority"]));
    let overbroad_access_change = hosted
        .clone()
        .oneshot(provider_request(
            "authorizeBrainEvent",
            overbroad_access_change,
        ))
        .await
        .unwrap();
    assert_eq!(overbroad_access_change.status(), StatusCode::BAD_REQUEST);

    let folder_key = FolderKey::generate().to_base64();
    let wrapped = hosted
        .clone()
        .oneshot(provider_request(
            "wrapGrantPayload",
            serde_json::json!({
                "purpose": "folder-key-grant",
                "vaultId": "personal",
                "folderId": "restricted",
                "keyVersion": 1,
                "recipientNpub": member_npub.clone(),
                "id": "grant-restricted-owner-v1",
                "folderKey": folder_key.clone(),
                "createdAt": "2026-07-13T12:00:00Z",
                "createdAtUnixSeconds": now,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(wrapped.status(), StatusCode::OK);
    let wrapped: Value =
        serde_json::from_slice(&wrapped.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let opened = hosted
        .clone()
        .oneshot(provider_request(
            "openGrantPayload",
            serde_json::json!({
                "purpose": "folder-key-grant",
                "vaultId": "personal",
                "folderId": "restricted",
                "keyVersion": 1,
                "recipientNpub": member_npub.clone(),
                "wrappedEventJson": wrapped["grant"]["wrappedEventJson"],
            }),
        ))
        .await
        .unwrap();
    assert_eq!(opened.status(), StatusCode::OK);
    let opened: Value =
        serde_json::from_slice(&opened.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(opened["plaintext"]["vaultId"], "personal");
    assert_eq!(opened["plaintext"]["folderId"], "restricted");
    assert_eq!(opened["plaintext"]["keyVersion"], 1);
    assert_eq!(opened["plaintext"]["recipientNpub"], member_npub);
    assert_eq!(opened["plaintext"]["folderKey"], folder_key);
    let wrong_scope = hosted
        .clone()
        .oneshot(provider_request(
            "openGrantPayload",
            serde_json::json!({
                "purpose": "folder-key-grant",
                "vaultId": "personal",
                "folderId": "getting-started",
                "keyVersion": 1,
                "recipientNpub": member_npub.clone(),
                "wrappedEventJson": wrapped["grant"]["wrappedEventJson"],
            }),
        ))
        .await
        .unwrap();
    assert_eq!(wrong_scope.status(), StatusCode::BAD_REQUEST);

    for (operation, input) in [
        ("signEvent", serde_json::json!({ "kind": 1 })),
        (
            "authorizeBrainEvent",
            serde_json::json!({
                "intent": "post-to-relay",
                "eventTemplate": {
                    "kind": 1,
                    "created_at": now,
                    "tags": [],
                    "content": "arbitrary",
                },
            }),
        ),
        (
            "openGrantPayload",
            serde_json::json!({
                "purpose": "folder-key-grant",
                "vaultId": "personal",
                "folderId": "restricted",
                "keyVersion": 1,
                "recipientNpub": member_npub.clone(),
                "wrappedEventJson": "arbitrary",
            }),
        ),
    ] {
        let rejected = hosted
            .clone()
            .oneshot(provider_request(operation, input))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn hosted_sites_identity_provider_is_setup_gated_and_origin_bounded() {
    let root = TempDir::new().unwrap();
    let hosted = test_app(&root);
    let provider_request = |operation: &str, origin: &str, url: &str, return_to: &str| {
        Request::post("/v1/sites/identity-provider")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header(WORKOS_USER_HEADER, "user_paul")
            .header("x-finite-sites-public-origin", origin)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "version": "finite-sites-identity-provider-v1",
                    "operation": operation,
                    "input": {
                        "url": url,
                        "returnTo": return_to,
                        "client": "finite-dashboard",
                        "nonce": "native-owner-session-proof",
                    },
                })
                .to_string(),
            ))
            .unwrap()
    };
    let session_url = "https://hello.finite.chat/_finite/auth/native-session";

    let setup_required = hosted
        .clone()
        .oneshot(provider_request(
            "authorizeViewerSession",
            "https://hello.finite.chat",
            session_url,
            "/draft?view=full#top",
        ))
        .await
        .unwrap();
    assert_eq!(setup_required.status(), StatusCode::PRECONDITION_REQUIRED);

    state_for(hosted.clone(), "user_paul").await;
    let authorized = hosted
        .clone()
        .oneshot(provider_request(
            "authorizeViewerSession",
            "https://hello.finite.chat",
            session_url,
            "/draft?view=full#top",
        ))
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    let authorized: Value =
        serde_json::from_slice(&authorized.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(
        authorized["body_json"],
        r#"{"purpose":"finite_site_view_session","return_to":"/draft?view=full#top","client":"finite-dashboard","nonce":"native-owner-session-proof"}"#
    );
    assert!(
        authorized["authorization_header"]
            .as_str()
            .unwrap()
            .starts_with("Nostr ")
    );

    let wrong_origin = hosted
        .clone()
        .oneshot(provider_request(
            "authorizeViewerSession",
            "https://other.finite.chat",
            session_url,
            "/",
        ))
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::BAD_REQUEST);

    let external_redirect = hosted
        .clone()
        .oneshot(provider_request(
            "authorizeViewerSession",
            "https://hello.finite.chat",
            session_url,
            "https://evil.example/",
        ))
        .await
        .unwrap();
    assert_eq!(external_redirect.status(), StatusCode::BAD_REQUEST);

    let unsupported = hosted
        .oneshot(provider_request(
            "signArbitraryRequest",
            "https://hello.finite.chat",
            session_url,
            "/",
        ))
        .await
        .unwrap();
    assert_eq!(unsupported.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn partial_hosted_device_state_loss_fails_closed_without_minting_a_replacement() {
    let root = TempDir::new().unwrap();
    let before = state_for(test_app(&root), "user_paul").await;
    let user_root = root
        .path()
        .join("users")
        .join(hex::encode(sha2::Sha256::digest(b"user_paul")));
    let identity_path = user_root.join("finite-home/identity/identity.json");
    let store_path = user_root.join("chat/client.sqlite3");
    let identity_bytes = fs::read(&identity_path).unwrap();

    fs::remove_file(&identity_path).unwrap();
    let missing_identity = state_response_for(test_app(&root), "user_paul").await;
    assert_eq!(missing_identity.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        !identity_path.exists(),
        "a missing identity must never be silently replaced beside retained chat state"
    );

    fs::write(&identity_path, &identity_bytes).unwrap();
    fs::remove_file(&store_path).unwrap();
    let missing_store = state_response_for(test_app(&root), "user_paul").await;
    assert_eq!(missing_store.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(fs::read(&identity_path).unwrap(), identity_bytes);
    assert_eq!(
        before["identity"]["account_id"].as_str().unwrap().len(),
        64,
        "the original account identity was established before simulating loss"
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
        public_url: PUBLIC_SERVER_URL.to_owned(),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_users_timed_out_agent_command_does_not_block_another_users_state() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("command-isolation-server.sqlite3"), None).await;
    let agent_identity = FiniteIdentity::load_or_generate(
        &IdentityPaths::with_finite_home(root.path().join("command-isolation-agent")),
        "finitechat-hosted-device-test/command-isolation-agent",
    )
    .unwrap();
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root
            .path()
            .join("command-isolation-agent-chat")
            .display()
            .to_string(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: Some(hex::encode(agent_identity.expose_secret_bytes())),
        now_unix_seconds: None,
    })
    .unwrap();
    let agent_state = agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    let agent_account_id = agent_state.identity.account_id;
    let agent_npub = npub_from_account_id(agent_account_id.clone()).unwrap();

    let hosted = app(HostedDeviceConfig {
        data_root: root.path().join("command-isolation-hosted"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    });
    action_for(
        hosted.clone(),
        "user_paul",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;
    let connected = action_for(
        hosted.clone(),
        "user_paul",
        serde_json::json!({
            "StartProfileChat": {
                "profile": {
                    "account_id": agent_account_id,
                    "npub": agent_npub,
                    "display_name": "Unresponsive Agent",
                    "about": "Does not process platform commands in this test",
                    "picture": null,
                    "stale": false,
                    "is_agent": true
                },
                "display_name": "Chat with Unresponsive Agent"
            }
        }),
    )
    .await;
    let room_id = connected["selected_room_id"].as_str().unwrap().to_owned();
    agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();

    let stalled_hosted = hosted.clone();
    let stalled_agent_account_id = agent_account_id.clone();
    let stalled = tokio::spawn(async move {
        runtime_command_for(
            stalled_hosted,
            "user_paul",
            serde_json::json!({
                "room_id": room_id,
                "target_account_id": stalled_agent_account_id,
                "command": "agent.owner.claim",
                "resource_key": "agent.connections",
                "schema": "finite.agent.empty.request.v1",
                "body": {},
                "wait_millis": 1_000
            }),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let alice = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        state_for(hosted, "user_alice"),
    )
    .await
    .expect("one user's agent timeout must not block another user's local state");
    assert_eq!(alice["identity"]["device_id"], "hosted-web");

    let stalled_response = stalled.await.unwrap();
    assert_eq!(stalled_response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn succeeded_owner_claim_is_replayed_from_the_durable_device_log_after_restart() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("owner-claim-server.sqlite3"), None).await;
    let agent_identity = FiniteIdentity::load_or_generate(
        &IdentityPaths::with_finite_home(root.path().join("owner-claim-agent")),
        "finitechat-hosted-device-test/owner-claim-agent",
    )
    .unwrap();
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root
            .path()
            .join("owner-claim-agent-chat")
            .display()
            .to_string(),
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
        data_root: root.path().join("owner-claim-hosted"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    };
    let hosted = app(config.clone());
    action_for(
        hosted.clone(),
        "user_paul",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;
    let connected = action_for(
        hosted.clone(),
        "user_paul",
        serde_json::json!({
            "StartProfileChat": {
                "profile": {
                    "account_id": agent_account_id,
                    "npub": agent_npub,
                    "display_name": "Claim Agent",
                    "about": "Returns one owner claim result",
                    "picture": null,
                    "stale": false,
                    "is_agent": true
                },
                "display_name": "Chat with Claim Agent"
            }
        }),
    )
    .await;
    let room_id = connected["selected_room_id"].as_str().unwrap().to_owned();
    agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();

    let first_hosted = hosted.clone();
    let first_room_id = room_id.clone();
    let first_agent_account_id = agent_account_id.clone();
    let first = tokio::spawn(async move {
        runtime_command_for(
            first_hosted,
            "user_paul",
            serde_json::json!({
                "room_id": first_room_id,
                "target_account_id": first_agent_account_id,
                "command": "agent.owner.claim",
                "resource_key": "agent.connections",
                "schema": "finite.agent.empty.request.v1",
                "body": {},
                "reuse_succeeded_owner_claim": true,
                "wait_millis": 5_000
            }),
        )
        .await
    });

    let request = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let bridge = agent.agent_bridge_poll_once().unwrap();
            if let Some(request) = bridge.events.into_iter().find_map(|stored| {
                let event =
                    serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext)
                        .ok()?;
                if event.kind != DurableAppEventKind::RuntimeCommandRequest {
                    return None;
                }
                let request =
                    serde_json::from_slice::<RuntimeCommandRequestV1>(&event.payload).ok()?;
                (request.command == "agent.owner.claim").then_some(request)
            }) {
                break request;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("agent must receive the first owner claim");
    let first_request_id = request.request_id.clone();
    let result = RuntimeCommandResultV1 {
        payload_kind: RuntimeCommandPayloadKindV1::Result,
        request_id: request.request_id,
        status: RuntimeCommandTerminalStatusV1::Succeeded,
        body: Some(RuntimeCommandJsonPayloadV1 {
            schema: "finite.agent.command.result.v1".to_owned(),
            json_payload: serde_json::to_vec(&serde_json::json!({ "connected": true })).unwrap(),
        }),
        error: None,
        clears_activity: Vec::new(),
    };
    agent
        .send_runtime_command_result_and_wait(
            room_id.clone(),
            None,
            serde_json::to_vec(&result).unwrap(),
        )
        .unwrap();

    let first_response = first.await.unwrap();
    assert_eq!(first_response.status(), StatusCode::OK);
    let first_body = first_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let first_json: Value = serde_json::from_slice(&first_body).unwrap();
    assert_eq!(first_json["request_id"], first_request_id);
    drop(hosted);

    let restarted = app(config);
    let replay = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        runtime_command_for(
            restarted,
            "user_paul",
            serde_json::json!({
                "room_id": room_id,
                "target_account_id": agent_account_id,
                "command": "agent.owner.claim",
                "resource_key": "agent.connections",
                "schema": "finite.agent.empty.request.v1",
                "body": {},
                "reuse_succeeded_owner_claim": true,
                "wait_millis": 1_000
            }),
        ),
    )
    .await
    .expect("durable successful claim replay must not wait for the agent");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body = replay.into_body().collect().await.unwrap().to_bytes();
    let replay_json: Value = serde_json::from_slice(&replay_body).unwrap();
    assert_eq!(replay_json["request_id"], first_request_id);
    assert_eq!(replay_json["body"]["connected"], true);
    server_task.abort();
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
        public_url: PUBLIC_SERVER_URL.to_owned(),
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
async fn unbound_existing_agent_rooms_are_not_automatically_migrated() {
    let root = TempDir::new().unwrap();
    let server_db = root.path().join("binding-server.sqlite3");
    let (server_url, _, server_task) = spawn_chat_server(&server_db, None).await;
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root.path().join("binding-agent").display().to_string(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: None,
        now_unix_seconds: None,
    })
    .unwrap();
    let agent_state = agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    let agent_account_id = agent_state.identity.account_id;
    let agent_npub = npub_from_account_id(agent_account_id.clone()).unwrap();
    let config = HostedDeviceConfig {
        data_root: root.path().join("binding-hosted"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    };
    let hosted = app(config.clone());
    action_for(
        hosted.clone(),
        "binding-user",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;
    let profile = serde_json::json!({
        "account_id": agent_account_id,
        "npub": agent_npub,
        "display_name": "Binding Agent",
        "about": null,
        "picture": null,
        "stale": false,
        "is_agent": true
    });
    let first = action_for(
        hosted.clone(),
        "binding-user",
        serde_json::json!({
            "StartProfileChat": { "profile": profile.clone(), "display_name": "First" }
        }),
    )
    .await;
    let first_room = first["selected_room_id"].as_str().unwrap().to_owned();
    agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    let duplicate = action_for(
        hosted.clone(),
        "binding-user",
        serde_json::json!({
            "StartGroupChat": { "profiles": [profile], "display_name": "Duplicate recovery" }
        }),
    )
    .await;
    let duplicate_room = duplicate["selected_room_id"].as_str().unwrap().to_owned();
    assert_ne!(duplicate_room, first_room);

    binding_for(
        hosted.clone(),
        "binding-user",
        "/v1/app/agent-bindings/authorize-bootstrap",
        serde_json::json!({
            "project_id": "project-one",
            "creation_request_id": "create-project-one"
        }),
    )
    .await;

    let before = state_for(hosted.clone(), "binding-user").await;
    let response = hosted
        .clone()
        .oneshot(
            Request::post("/v1/app/agent-bindings/ensure")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, "binding-user")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "project_id": "project-one",
                        "agent_npub": agent_npub,
                        "display_name": "Chat with Binding Agent"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let error = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        String::from_utf8_lossy(&error).contains("automatic migration is disabled"),
        "{}",
        String::from_utf8_lossy(&error)
    );
    let after = state_for(hosted, "binding-user").await;
    assert_eq!(after, before);
    let binding_root = root
        .path()
        .join("binding-hosted/users")
        .join(hex::encode(sha2::Sha256::digest(b"binding-user")))
        .join("agent-bindings");
    let records = fs::read_dir(binding_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert!(records[0].ends_with(".authorization.json"));
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_conflicting_binding_ensures_serialize_without_last_writer_wins() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("binding-race-server.sqlite3"), None).await;
    let first_identity = FiniteIdentity::load_or_generate(
        &IdentityPaths::with_finite_home(root.path().join("binding-race-identity-a")),
        "finitechat-hosted-device-test/binding-race-a",
    )
    .unwrap();
    let second_identity = FiniteIdentity::load_or_generate(
        &IdentityPaths::with_finite_home(root.path().join("binding-race-identity-b")),
        "finitechat-hosted-device-test/binding-race-b",
    )
    .unwrap();
    let first_agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root
            .path()
            .join("binding-race-agent-a")
            .display()
            .to_string(),
        server_url: server_url.clone(),
        device_id: "agent-a".to_owned(),
        account_secret_hex: Some(hex::encode(first_identity.expose_secret_bytes())),
        now_unix_seconds: None,
    })
    .unwrap();
    let second_agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root
            .path()
            .join("binding-race-agent-b")
            .display()
            .to_string(),
        server_url: server_url.clone(),
        device_id: "agent-b".to_owned(),
        account_secret_hex: Some(hex::encode(second_identity.expose_secret_bytes())),
        now_unix_seconds: None,
    })
    .unwrap();
    let first_npub = npub_from_account_id(
        first_agent
            .dispatch_and_wait(AppAction::StartRuntime)
            .unwrap()
            .identity
            .account_id,
    )
    .unwrap();
    let second_npub = npub_from_account_id(
        second_agent
            .dispatch_and_wait(AppAction::StartRuntime)
            .unwrap()
            .identity
            .account_id,
    )
    .unwrap();
    assert_ne!(first_npub, second_npub);
    let hosted = app(HostedDeviceConfig {
        data_root: root.path().join("binding-race-hosted"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    });
    binding_for(
        hosted.clone(),
        "binding-race-user",
        "/v1/app/agent-bindings/authorize-bootstrap",
        serde_json::json!({
            "project_id": "project-race",
            "creation_request_id": "create-project-race"
        }),
    )
    .await;

    let (first, second) = tokio::join!(
        binding_response_for(
            hosted.clone(),
            "binding-race-user",
            "/v1/app/agent-bindings/ensure",
            serde_json::json!({
                "project_id": "project-race",
                "agent_npub": first_npub,
                "display_name": "First Agent"
            }),
        ),
        binding_response_for(
            hosted.clone(),
            "binding-race-user",
            "/v1/app/agent-bindings/ensure",
            serde_json::json!({
                "project_id": "project-race",
                "agent_npub": second_npub,
                "display_name": "Second Agent"
            }),
        )
    );
    let statuses = [first.status(), second.status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::SERVICE_UNAVAILABLE)
            .count(),
        1
    );
    let state = state_for(hosted, "binding-race-user").await;
    assert_eq!(state["rooms"].as_array().unwrap().len(), 1);
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_final_binding_persist_resumes_only_the_durable_intended_room() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("binding-resume-server.sqlite3"), None).await;
    let agent_identity = FiniteIdentity::load_or_generate(
        &IdentityPaths::with_finite_home(root.path().join("binding-resume-agent-identity")),
        "finitechat-hosted-device-test/binding-resume-agent",
    )
    .unwrap();
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root
            .path()
            .join("binding-resume-agent")
            .display()
            .to_string(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: Some(hex::encode(agent_identity.expose_secret_bytes())),
        now_unix_seconds: None,
    })
    .unwrap();
    let agent_npub = npub_from_account_id(
        agent
            .dispatch_and_wait(AppAction::StartRuntime)
            .unwrap()
            .identity
            .account_id,
    )
    .unwrap();
    let config = HostedDeviceConfig {
        data_root: root.path().join("binding-resume-hosted"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    };
    let hosted = app_with_final_agent_binding_persist_failures(config.clone(), 1);
    binding_for(
        hosted.clone(),
        "binding-resume-user",
        "/v1/app/agent-bindings/authorize-bootstrap",
        serde_json::json!({
            "project_id": "project-resume",
            "creation_request_id": "create-project-resume"
        }),
    )
    .await;
    let request = serde_json::json!({
        "project_id": "project-resume",
        "agent_npub": agent_npub,
        "display_name": "Resume Agent"
    });
    let failed = binding_response_for(
        hosted.clone(),
        "binding-resume-user",
        "/v1/app/agent-bindings/ensure",
        request.clone(),
    )
    .await;
    let failed_status = failed.status();
    let failed_body = failed.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        failed_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        String::from_utf8_lossy(&failed_body)
    );
    let after_failure = state_for(hosted.clone(), "binding-resume-user").await;
    assert_eq!(after_failure["rooms"].as_array().unwrap().len(), 1);
    let intended_room_id = after_failure["rooms"][0]["room_id"]
        .as_str()
        .unwrap()
        .to_owned();

    drop(hosted);
    let resumed_app = app(config.clone());
    let resumed = binding_for(
        resumed_app.clone(),
        "binding-resume-user",
        "/v1/app/agent-bindings/ensure",
        request,
    )
    .await;
    assert_eq!(resumed["rooms"].as_array().unwrap().len(), 1);
    assert_eq!(
        resumed["hosted_agent_binding"]["canonical_room_id"],
        intended_room_id
    );

    drop(resumed_app);
    let reopened = binding_for(
        app(config),
        "binding-resume-user",
        "/v1/app/agent-bindings/open",
        serde_json::json!({ "project_id": "project-resume" }),
    )
    .await;
    assert_eq!(reopened["rooms"].as_array().unwrap().len(), 1);
    assert_eq!(reopened["selected_room_id"], intended_room_id);
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_replays_exact_room_create_after_server_acceptance_before_local_save() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) = spawn_chat_server(
        &root
            .path()
            .join("binding-room-create-resume-server.sqlite3"),
        None,
    )
    .await;
    let agent_identity = FiniteIdentity::load_or_generate(
        &IdentityPaths::with_finite_home(
            root.path()
                .join("binding-room-create-resume-agent-identity"),
        ),
        "finitechat-hosted-device-test/binding-room-create-resume-agent",
    )
    .unwrap();
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root
            .path()
            .join("binding-room-create-resume-agent")
            .display()
            .to_string(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: Some(hex::encode(agent_identity.expose_secret_bytes())),
        now_unix_seconds: None,
    })
    .unwrap();
    let agent_npub = npub_from_account_id(
        agent
            .dispatch_and_wait(AppAction::StartRuntime)
            .unwrap()
            .identity
            .account_id,
    )
    .unwrap();
    let config = HostedDeviceConfig {
        data_root: root.path().join("binding-room-create-resume-hosted"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    };
    let hosted = app_with_profile_bootstrap_room_create_failures(config.clone(), 1);
    binding_for(
        hosted.clone(),
        "binding-room-create-resume-user",
        "/v1/app/agent-bindings/authorize-bootstrap",
        serde_json::json!({
            "project_id": "project-room-create-resume",
            "creation_request_id": "create-project-room-create-resume"
        }),
    )
    .await;
    let request = serde_json::json!({
        "project_id": "project-room-create-resume",
        "agent_npub": agent_npub,
        "display_name": "Room Create Resume Agent"
    });
    let failed = binding_response_for(
        hosted.clone(),
        "binding-room-create-resume-user",
        "/v1/app/agent-bindings/ensure",
        request.clone(),
    )
    .await;
    assert_eq!(failed.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        state_for(hosted.clone(), "binding-room-create-resume-user").await["rooms"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    drop(hosted);
    let resumed_app = app(config.clone());
    let resumed = binding_for(
        resumed_app.clone(),
        "binding-room-create-resume-user",
        "/v1/app/agent-bindings/ensure",
        request,
    )
    .await;
    let intended_room_id = resumed["hosted_agent_binding"]["canonical_room_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(resumed["rooms"].as_array().unwrap().len(), 1);
    assert_eq!(resumed["rooms"][0]["room_id"], intended_room_id);
    assert_eq!(resumed["selected_room_id"], intended_room_id);

    drop(resumed_app);
    let reopened = binding_for(
        app(config),
        "binding-room-create-resume-user",
        "/v1/app/agent-bindings/open",
        serde_json::json!({ "project_id": "project-room-create-resume" }),
    )
    .await;
    assert_eq!(reopened["rooms"].as_array().unwrap().len(), 1);
    assert_eq!(reopened["rooms"][0]["room_id"], intended_room_id);
    assert_eq!(reopened["selected_room_id"], intended_room_id);
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_resubmits_exact_journaled_add_after_pending_mls_state_was_saved() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) = spawn_chat_server(
        &root.path().join("binding-submit-resume-server.sqlite3"),
        None,
    )
    .await;
    let agent_identity = FiniteIdentity::load_or_generate(
        &IdentityPaths::with_finite_home(root.path().join("binding-submit-resume-agent-identity")),
        "finitechat-hosted-device-test/binding-submit-resume-agent",
    )
    .unwrap();
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root
            .path()
            .join("binding-submit-resume-agent")
            .display()
            .to_string(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: Some(hex::encode(agent_identity.expose_secret_bytes())),
        now_unix_seconds: None,
    })
    .unwrap();
    let agent_npub = npub_from_account_id(
        agent
            .dispatch_and_wait(AppAction::StartRuntime)
            .unwrap()
            .identity
            .account_id,
    )
    .unwrap();
    let config = HostedDeviceConfig {
        data_root: root.path().join("binding-submit-resume-hosted"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    };
    let hosted = app_with_profile_bootstrap_submit_failures(config.clone(), 1);
    binding_for(
        hosted.clone(),
        "binding-submit-resume-user",
        "/v1/app/agent-bindings/authorize-bootstrap",
        serde_json::json!({
            "project_id": "project-submit-resume",
            "creation_request_id": "create-project-submit-resume"
        }),
    )
    .await;
    let request = serde_json::json!({
        "project_id": "project-submit-resume",
        "agent_npub": agent_npub,
        "display_name": "Submit Resume Agent"
    });
    let failed = binding_response_for(
        hosted.clone(),
        "binding-submit-resume-user",
        "/v1/app/agent-bindings/ensure",
        request.clone(),
    )
    .await;
    assert_eq!(failed.status(), StatusCode::BAD_REQUEST);
    let after_failure = state_for(hosted.clone(), "binding-submit-resume-user").await;
    assert_eq!(after_failure["rooms"].as_array().unwrap().len(), 1);
    let intended_room_id = after_failure["rooms"][0]["room_id"]
        .as_str()
        .unwrap()
        .to_owned();

    drop(hosted);
    let resumed_app = app(config.clone());
    let resumed = binding_for(
        resumed_app.clone(),
        "binding-submit-resume-user",
        "/v1/app/agent-bindings/ensure",
        request,
    )
    .await;
    assert_eq!(resumed["rooms"].as_array().unwrap().len(), 1);
    assert_eq!(
        resumed["hosted_agent_binding"]["canonical_room_id"],
        intended_room_id
    );
    assert_eq!(resumed["selected_room_id"], intended_room_id);

    drop(resumed_app);
    let reopened = binding_for(
        app(config),
        "binding-submit-resume-user",
        "/v1/app/agent-bindings/open",
        serde_json::json!({ "project_id": "project-submit-resume" }),
    )
    .await;
    assert_eq!(reopened["rooms"].as_array().unwrap().len(), 1);
    assert_eq!(reopened["selected_room_id"], intended_room_id);
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_agent_binding_stays_unchanged_across_duplicate_selection_and_restart() {
    let root = TempDir::new().unwrap();
    let server_db = root.path().join("binding-server.sqlite3");
    let (server_url, _, server_task) = spawn_chat_server(&server_db, None).await;
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root.path().join("binding-agent").display().to_string(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: None,
        now_unix_seconds: None,
    })
    .unwrap();
    let agent_state = agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    let agent_account_id = agent_state.identity.account_id;
    let agent_npub = npub_from_account_id(agent_account_id.clone()).unwrap();
    let config = HostedDeviceConfig {
        data_root: root.path().join("binding-hosted"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    };
    let hosted = app(config.clone());
    action_for(
        hosted.clone(),
        "binding-user",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;
    let profile = serde_json::json!({
        "account_id": agent_account_id,
        "npub": agent_npub,
        "display_name": "Binding Agent",
        "about": null,
        "picture": null,
        "stale": false,
        "is_agent": true
    });

    let unauthorized_bootstrap = binding_response_for(
        hosted.clone(),
        "binding-user",
        "/v1/app/agent-bindings/ensure",
        serde_json::json!({
            "project_id": "project-one",
            "agent_npub": agent_npub,
            "display_name": "Chat with Binding Agent"
        }),
    )
    .await;
    assert_eq!(
        unauthorized_bootstrap.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(
        state_for(hosted.clone(), "binding-user").await["rooms"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let authorization = binding_for(
        hosted.clone(),
        "binding-user",
        "/v1/app/agent-bindings/authorize-bootstrap",
        serde_json::json!({
            "project_id": "project-one",
            "creation_request_id": "create-project-one"
        }),
    )
    .await;
    assert_eq!(authorization["status"], "authorized");
    let repeated_authorization = binding_for(
        hosted.clone(),
        "binding-user",
        "/v1/app/agent-bindings/authorize-bootstrap",
        serde_json::json!({
            "project_id": "project-one",
            "creation_request_id": "create-project-one"
        }),
    )
    .await;
    assert_eq!(repeated_authorization["status"], "already_authorized");
    let conflicting_authorization = binding_response_for(
        hosted.clone(),
        "binding-user",
        "/v1/app/agent-bindings/authorize-bootstrap",
        serde_json::json!({
            "project_id": "project-one",
            "creation_request_id": "different-project-creation"
        }),
    )
    .await;
    assert_eq!(
        conflicting_authorization.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );

    let ensured = binding_for(
        hosted.clone(),
        "binding-user",
        "/v1/app/agent-bindings/ensure",
        serde_json::json!({
            "project_id": "project-one",
            "agent_npub": agent_npub,
            "display_name": "Chat with Binding Agent"
        }),
    )
    .await;
    let canonical_room = ensured["hosted_agent_binding"]["canonical_room_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        ensured["hosted_agent_binding"]["associated_room_ids"],
        serde_json::json!([])
    );
    let binding_path = fs::read_dir(
        root.path()
            .join("binding-hosted/users")
            .join(hex::encode(sha2::Sha256::digest(b"binding-user")))
            .join("agent-bindings"),
    )
    .unwrap()
    .next()
    .unwrap()
    .unwrap()
    .path();
    action_for(
        hosted.clone(),
        "binding-user",
        serde_json::json!({ "CreateTopic": { "room_id": canonical_room, "title": "Retained first" } }),
    )
    .await;

    agent.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    let duplicate = action_for(
        hosted.clone(),
        "binding-user",
        serde_json::json!({
            "StartGroupChat": { "profiles": [profile], "display_name": "Duplicate recovery" }
        }),
    )
    .await;
    let duplicate_room = duplicate["selected_room_id"].as_str().unwrap().to_owned();
    assert_ne!(duplicate_room, canonical_room);
    action_for(
        hosted.clone(),
        "binding-user",
        serde_json::json!({ "CreateTopic": { "room_id": duplicate_room, "title": "Retained duplicate" } }),
    )
    .await;

    rewrite_associated_room_ids(
        &config.data_root,
        "binding-user",
        "project-one",
        &binding_path,
        std::slice::from_ref(&duplicate_room),
    );
    let original_sealed_binding = fs::read(&binding_path).unwrap();

    let opened_after_duplicate = binding_for(
        hosted.clone(),
        "binding-user",
        "/v1/app/agent-bindings/open",
        serde_json::json!({ "project_id": "project-one" }),
    )
    .await;
    assert_eq!(
        opened_after_duplicate["hosted_agent_binding"]["canonical_room_id"],
        canonical_room
    );
    assert_eq!(
        opened_after_duplicate["hosted_agent_binding"]["associated_room_ids"],
        serde_json::json!([duplicate_room])
    );
    assert_eq!(opened_after_duplicate["rooms"].as_array().unwrap().len(), 2);
    assert_eq!(
        opened_after_duplicate["topics"].as_array().unwrap().len(),
        4
    );
    let canonical_home_before = opened_after_duplicate["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|topic| topic["room_id"] == canonical_room && topic["topic_id"] == "home")
        .unwrap()["chats"]
        .as_array()
        .unwrap()
        .len();
    let legacy_new_chat = hosted
        .clone()
        .oneshot(
            Request::post("/v1/app/new-chat")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, "binding-user")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "project_id": "project-one",
                        "room_id": duplicate_room,
                        "topic_id": "home",
                        "reason": null,
                        "intent_key": "legacy-browser-new-chat"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(legacy_new_chat.status(), StatusCode::CONFLICT);
    let new_chat = serde_json::json!({
        "project_id": "project-one",
        "room_id": canonical_room,
        "topic_id": "home",
        "reason": null,
        "intent_key": "browser-new-chat-1"
    });
    let first_new_chat = binding_for(
        hosted.clone(),
        "binding-user",
        "/v1/app/new-chat",
        new_chat.clone(),
    )
    .await;
    let retried_new_chat =
        binding_for(hosted.clone(), "binding-user", "/v1/app/new-chat", new_chat).await;
    assert_eq!(
        first_new_chat["selected_chat_id"],
        retried_new_chat["selected_chat_id"]
    );
    let canonical_home_after = retried_new_chat["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|topic| topic["room_id"] == canonical_room && topic["topic_id"] == "home")
        .unwrap()["chats"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(canonical_home_after, canonical_home_before + 1);
    let sealed_binding = fs::read_to_string(&binding_path).unwrap();
    assert!(!sealed_binding.contains("project-one"));
    assert!(!sealed_binding.contains(&agent_account_id));
    assert!(!sealed_binding.contains(&canonical_room));
    assert!(!sealed_binding.contains(&duplicate_room));
    let failed_claim = runtime_command_for(
        hosted.clone(),
        "binding-user",
        serde_json::json!({
            "room_id": canonical_room,
            "target_account_id": agent_account_id,
            "command": "agent.owner.claim",
            "resource_key": "agent.connections",
            "schema": "finite.agent.empty.request.v1",
            "body": {},
            "reuse_succeeded_owner_claim": true,
            "wait_millis": 1_000
        }),
    )
    .await;
    assert_eq!(failed_claim.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let after_failed_claim = binding_for(
        hosted.clone(),
        "binding-user",
        "/v1/app/agent-bindings/open",
        serde_json::json!({ "project_id": "project-one" }),
    )
    .await;
    assert_eq!(after_failed_claim["rooms"].as_array().unwrap().len(), 2);
    assert_eq!(after_failed_claim["topics"].as_array().unwrap().len(), 4);
    assert_eq!(
        after_failed_claim["hosted_agent_binding"]["associated_room_ids"],
        serde_json::json!([duplicate_room])
    );

    action_for(
        hosted.clone(),
        "binding-user",
        serde_json::json!({ "OpenRoom": { "room_id": duplicate_room } }),
    )
    .await;
    drop(hosted);
    server_task.abort();
    let reopened = binding_for(
        app(config),
        "binding-user",
        "/v1/app/agent-bindings/open",
        serde_json::json!({ "project_id": "project-one" }),
    )
    .await;
    assert_eq!(reopened["selected_room_id"], canonical_room);
    assert_eq!(reopened["rooms"].as_array().unwrap().len(), 2);
    assert_eq!(reopened["topics"].as_array().unwrap().len(), 4);
    assert_eq!(
        reopened["hosted_agent_binding"]["associated_room_ids"],
        serde_json::json!([duplicate_room])
    );
    assert_eq!(fs::read(binding_path).unwrap(), original_sealed_binding);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attachment_bytes_are_isolated_redacted_and_survive_device_restart() {
    let root = TempDir::new().unwrap();
    let server_db = root.path().join("attachment-server.sqlite3");
    let (server_url, _, server_task) = spawn_chat_server(&server_db, None).await;
    let config = HostedDeviceConfig {
        data_root: root.path().join("hosted-devices"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
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
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    })
}

async fn state_for(app: axum::Router, user_id: &str) -> Value {
    let response = state_response_for(app, user_id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn state_response_for(app: axum::Router, user_id: &str) -> axum::response::Response {
    app.oneshot(
        Request::get("/v1/app/state")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header(WORKOS_USER_HEADER, user_id)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
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

async fn runtime_command_for(
    app: axum::Router,
    user_id: &str,
    command: Value,
) -> axum::response::Response {
    app.oneshot(
        Request::post("/v1/app/runtime-commands")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header(WORKOS_USER_HEADER, user_id)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&command).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn binding_for(app: axum::Router, user_id: &str, path: &str, body: Value) -> Value {
    let response = binding_response_for(app, user_id, path, body).await;
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

async fn binding_response_for(
    app: axum::Router,
    user_id: &str,
    path: &str,
    body: Value,
) -> axum::response::Response {
    app.oneshot(
        Request::post(path)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header(WORKOS_USER_HEADER, user_id)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[derive(Serialize, Deserialize)]
struct TestSealedAgentBinding {
    version: u16,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn rewrite_associated_room_ids(
    hosted_root: &Path,
    user_id: &str,
    project_id: &str,
    binding_path: &Path,
    associated_room_ids: &[String],
) {
    let user_storage_id = hex::encode(sha2::Sha256::digest(user_id.as_bytes()));
    let identity = FiniteIdentity::load(&IdentityPaths::with_finite_home(
        hosted_root
            .join("users")
            .join(&user_storage_id)
            .join("finite-home"),
    ))
    .unwrap();
    let mut key_hasher = sha2::Sha256::new();
    key_hasher.update(TEST_AGENT_BINDING_KEY_DOMAIN);
    key_hasher.update(identity.expose_secret_bytes());
    let key: [u8; 32] = key_hasher.finalize().into();
    let mut aad = TEST_AGENT_BINDING_AAD_DOMAIN.to_vec();
    aad.extend_from_slice(user_storage_id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(project_id.as_bytes());
    let provider = OpenMlsRustCrypto::default();
    let sealed: TestSealedAgentBinding =
        serde_json::from_slice(&fs::read(binding_path).unwrap()).unwrap();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            &key,
            &sealed.ciphertext,
            &sealed.nonce,
            &aad,
        )
        .unwrap();
    let mut binding: Value = serde_json::from_slice(&plaintext).unwrap();
    binding["associated_room_ids"] = serde_json::json!(associated_room_ids);
    let nonce: [u8; 12] = provider.rand().random_array().unwrap();
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            &key,
            &serde_json::to_vec(&binding).unwrap(),
            &nonce,
            &aad,
        )
        .unwrap();
    fs::write(
        binding_path,
        serde_json::to_vec(&TestSealedAgentBinding {
            version: 1,
            nonce: nonce.to_vec(),
            ciphertext,
        })
        .unwrap(),
    )
    .unwrap();
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
