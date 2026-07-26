use axum::body::Body;
use axum::http::{Request, StatusCode};
use finite_brain_core::{BRAIN_IDENTITY_PROVIDER_VERSION, FolderKey};
use finite_identity::{FiniteIdentity, IdentityPaths};
use finite_nostr::verify_event_integrity;
use finitechat_core::nip_ab::{
    NipAbPayloadType, NipAbSourceDescriptorV1, NipAbTargetSession, decode_finite_pairing_payload_v2,
};
use finitechat_core::{AppAction, FiniteChatRuntime, OpenOptions, npub_from_account_id};
use finitechat_hosted_device::{
    HostedDeviceConfig, HostedIdentityAuthorityConfig, MAX_HOSTED_ATTACHMENT_BYTES,
    MAX_HOSTED_ATTACHMENTS_PER_MESSAGE, MAX_HOSTED_MULTIPART_BODY_BYTES, WORKOS_USER_HEADER, app,
    app_with_final_agent_binding_persist_failures, app_with_fixed_device_link_now,
    app_with_fixed_device_link_now_and_lock_hook, app_with_identity_authority,
    app_with_profile_bootstrap_room_create_failures, app_with_profile_bootstrap_submit_failures,
};
use finitechat_http::{
    CreatePairingSessionRequest, GetPairingSessionRequest, HttpPairingSessionRecord,
    PublishPairingCompleteRequest, PublishPairingOfferRequest,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_hosted_chat_setup_registers_the_users_public_identity() {
    let root = TempDir::new().unwrap();
    let observed = std::sync::Arc::new(std::sync::Mutex::new(None));
    let observed_request = std::sync::Arc::clone(&observed);
    let identity_authority = axum::Router::new().route(
        "/api/v1/operator/account-principal-bindings",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                let observed_request = std::sync::Arc::clone(&observed_request);
                async move {
                    assert_eq!(
                        headers
                            .get("x-finite-operator-token")
                            .and_then(|value| value.to_str().ok()),
                        Some("identity-test-token")
                    );
                    *observed_request.lock().unwrap() = Some(body);
                    axum::Json(serde_json::json!({ "created": true }))
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let authority_task =
        tokio::spawn(async move { axum::serve(listener, identity_authority).await.unwrap() });
    let hosted = app_with_identity_authority(
        HostedDeviceConfig {
            data_root: root.path().to_path_buf(),
            server_url: "http://127.0.0.1:9".to_owned(),
            public_url: PUBLIC_SERVER_URL.to_owned(),
            api_token: TOKEN.to_owned(),
        },
        HostedIdentityAuthorityConfig {
            base_url: format!("http://{address}"),
            operator_token: "identity-test-token".to_owned(),
        },
    );

    let user_state = state_for(hosted, "user_paul").await;
    let expected_npub = npub_from_account_id(
        user_state["identity"]["account_id"]
            .as_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    assert_eq!(
        observed.lock().unwrap().as_ref(),
        Some(&serde_json::json!({
            "workosUserId": "user_paul",
            "userNpub": expected_npub,
        }))
    );
    authority_task.abort();
}

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

    for path in [
        "/v1/device-links/approve",
        "/v1/device-links/status",
        "/v1/device-links/enroll",
        "/v1/device-links/reconcile",
    ] {
        let response = test_app(&root)
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"pairing_session_id":"pair-a","target_device_id":"electron-a"}"#,
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

#[tokio::test]
async fn device_enrollment_authenticates_before_strict_bounded_json_parsing() {
    let root = TempDir::new().unwrap();
    let hosted = test_app(&root);

    let unauthorized = hosted
        .clone()
        .oneshot(
            Request::post("/v1/device-links/enroll")
                .header("content-type", "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    for body in [
        Body::from("not-json"),
        Body::from(
            r#"{"pairing_session_id":"pair-a","target_device_id":"electron-a","enrollment_user_id":"user-a","enrollment_capability_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","unexpected":true}"#,
        ),
        Body::from(
            r#"{"pairing_session_id":"pair-a","target_device_id":"electron-a","enrollment_user_id":"user-a","enrollment_capability_hex":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#,
        ),
    ] {
        let response = hosted
            .clone()
            .oneshot(
                Request::post("/v1/device-links/enroll")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("content-type", "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let oversized = hosted
        .oneshot(
            Request::post("/v1/device-links/enroll")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(vec![b'x'; 4 * 1024 + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn device_reconciliation_authenticates_before_strict_bounded_json_parsing() {
    let root = TempDir::new().unwrap();
    let hosted = test_app(&root);

    let unauthorized = hosted
        .clone()
        .oneshot(
            Request::post("/v1/device-links/reconcile")
                .header("content-type", "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    for body in [
        Body::from("not-json"),
        Body::from(
            r#"{"project_id":"project-one","target_device_id":"electron-one","room_id":"untrusted"}"#,
        ),
        Body::from(r#"{"project_id":"project-one"}"#),
    ] {
        let response = hosted
            .clone()
            .oneshot(
                Request::post("/v1/device-links/reconcile")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header(WORKOS_USER_HEADER, "reconcile-user")
                    .header("content-type", "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    for target_device_id in ["hosted-web".to_owned(), "x".repeat(129)] {
        let response = reconcile_device_for(
            hosted.clone(),
            "reconcile-user",
            "project-one",
            &target_device_id,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let oversized = hosted
        .oneshot(
            Request::post("/v1/device-links/reconcile")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, "reconcile-user")
                .header("content-type", "application/json")
                .body(Body::from(vec![b'x'; 4 * 1024 + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_nip_ab_approval_persists_a_target_bound_checkpoint() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("pairing-server.sqlite3"), None).await;
    let authority_requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed_authority_requests = std::sync::Arc::clone(&authority_requests);
    let identity_authority = axum::Router::new().route(
        "/api/v1/operator/account-principal-bindings",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                let observed_authority_requests =
                    std::sync::Arc::clone(&observed_authority_requests);
                async move {
                    assert_eq!(
                        headers
                            .get("x-finite-operator-token")
                            .and_then(|value| value.to_str().ok()),
                        Some("pairing-identity-test-token")
                    );
                    observed_authority_requests.lock().unwrap().push(body);
                    axum::Json(serde_json::json!({ "created": true }))
                }
            },
        ),
    );
    let authority_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let authority_address = authority_listener.local_addr().unwrap();
    let authority_task = tokio::spawn(async move {
        axum::serve(authority_listener, identity_authority)
            .await
            .unwrap()
    });
    let authority_config = HostedIdentityAuthorityConfig {
        base_url: format!("http://{authority_address}"),
        operator_token: "pairing-identity-test-token".to_owned(),
    };
    let target = NipAbTargetSession::prepare();
    let pairing_session_id = "pair-hosted-happy-path";
    let target_device_id = "ios-hosted-happy-path";
    let created = reqwest::Client::new()
        .post(format!("{server_url}/pairing-sessions"))
        .json(&CreatePairingSessionRequest {
            version: 1,
            pairing_session_id: pairing_session_id.to_owned(),
            target_device_id: target_device_id.to_owned(),
            target_public_key: target.public_key(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<HttpPairingSessionRecord>()
        .await
        .unwrap();
    assert_eq!(created.pairing_session_id, pairing_session_id);

    let data_root = root.path().join("hosted-devices");
    let hosted_config = HostedDeviceConfig {
        data_root: data_root.clone(),
        server_url: server_url.clone(),
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    };
    let existing_hosted =
        app_with_identity_authority(hosted_config.clone(), authority_config.clone());
    let existing_state = state_for(existing_hosted, "user_pairing_happy_path").await;
    let existing_account_id = existing_state["identity"]["account_id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Recreate the service over the durable account before approving. A fresh
    // fixture alone cannot catch deployment failures that occur while opening
    // an established Hosted Device and reasserting its authority binding.
    let hosted = app_with_identity_authority(hosted_config, authority_config);
    let response = device_link_for(
        hosted.clone(),
        "user_pairing_happy_path",
        "/v1/device-links/approve",
        pairing_session_id,
        target_device_id,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "awaiting_offer");
    let descriptor = NipAbSourceDescriptorV1 {
        version: json["source_descriptor"]["version"].as_u64().unwrap() as u16,
        source_public_key: json["source_descriptor"]["source_public_key"]
            .as_str()
            .unwrap()
            .to_owned(),
        session_secret_hex: json["source_descriptor"]["session_secret_hex"]
            .as_str()
            .unwrap()
            .to_owned(),
        expires_at_unix_seconds: json["source_descriptor"]["expires_at_unix_seconds"]
            .as_u64()
            .unwrap(),
    };
    NipAbTargetSession::create(target, &descriptor, test_now_unix_seconds())
        .expect("the hosted source descriptor must authenticate for the original target");
    let reopened_state = state_for(hosted, "user_pairing_happy_path").await;
    assert_eq!(
        reopened_state["identity"]["account_id"], existing_account_id,
        "service restart must reopen the same durable human account"
    );
    let authority_requests = authority_requests.lock().unwrap();
    assert_eq!(authority_requests.len(), 2);
    assert_eq!(authority_requests[0], authority_requests[1]);

    let record_path = data_root
        .join("users")
        .join(hex::encode(sha2::Sha256::digest(
            b"user_pairing_happy_path",
        )))
        .join("device-links")
        .join(format!(
            "{}.json",
            hex::encode(sha2::Sha256::digest(pairing_session_id.as_bytes()))
        ));
    assert!(
        record_path.is_file(),
        "approval must durably persist its target-bound checkpoint before returning"
    );
    authority_task.abort();
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_enrollment_capability_resumes_after_grant_expiry_without_workos() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("resume-server.sqlite3"), None).await;
    let now = test_now_unix_seconds();
    let user_id = "user_pairing_resume";
    let target_device_id = "ios-pairing-resume";
    let pairing_session_id = "pair-hosted-resume";
    let config = HostedDeviceConfig {
        data_root: root.path().join("resume-hosted"),
        server_url: server_url.clone(),
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    };
    let hosted = app_with_fixed_device_link_now(config.clone(), now);
    action_for(
        hosted.clone(),
        user_id,
        serde_json::json!({
            "CreateRoom": { "display_name": "Durable resume room" }
        }),
    )
    .await;

    let target_bootstrap = NipAbTargetSession::prepare();
    reqwest::Client::new()
        .post(format!("{server_url}/pairing-sessions"))
        .json(&CreatePairingSessionRequest {
            version: 1,
            pairing_session_id: pairing_session_id.to_owned(),
            target_device_id: target_device_id.to_owned(),
            target_public_key: target_bootstrap.public_key(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let approved = device_link_for(
        hosted.clone(),
        user_id,
        "/v1/device-links/approve",
        pairing_session_id,
        target_device_id,
    )
    .await;
    let approved: Value =
        serde_json::from_slice(&approved.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let descriptor = NipAbSourceDescriptorV1 {
        version: approved["source_descriptor"]["version"].as_u64().unwrap() as u16,
        source_public_key: approved["source_descriptor"]["source_public_key"]
            .as_str()
            .unwrap()
            .to_owned(),
        session_secret_hex: approved["source_descriptor"]["session_secret_hex"]
            .as_str()
            .unwrap()
            .to_owned(),
        expires_at_unix_seconds: approved["source_descriptor"]["expires_at_unix_seconds"]
            .as_u64()
            .unwrap(),
    };
    let (mut target_session, offer) =
        NipAbTargetSession::create(target_bootstrap, &descriptor, now).unwrap();
    reqwest::Client::new()
        .post(format!("{server_url}/pairing-sessions/offer"))
        .json(&PublishPairingOfferRequest {
            pairing_session_id: pairing_session_id.to_owned(),
            offer_event: serde_json::to_vec(&offer).unwrap(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let response = device_link_for(
        hosted.clone(),
        user_id,
        "/v1/device-links/status",
        pairing_session_id,
        target_device_id,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let record = reqwest::Client::new()
        .post(format!("{server_url}/pairing-sessions/get"))
        .json(&GetPairingSessionRequest {
            pairing_session_id: pairing_session_id.to_owned(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Option<HttpPairingSessionRecord>>()
        .await
        .unwrap()
        .unwrap();
    let confirmation: Event = serde_json::from_slice(&record.events[1].event).unwrap();
    let payload_event: Event = serde_json::from_slice(&record.events[2].event).unwrap();
    target_session
        .accept_source_confirmation(&confirmation, now)
        .unwrap();
    target_session.confirm_sas(now).unwrap();
    let (kind, payload_json) = target_session.accept_payload(&payload_event, now).unwrap();
    assert_eq!(kind, NipAbPayloadType::Custom);
    let payload = decode_finite_pairing_payload_v2(&payload_json).unwrap();
    payload
        .validate(pairing_session_id, target_device_id, PUBLIC_SERVER_URL, now)
        .unwrap();
    assert_eq!(payload.enrollment_user_id, user_id);
    let complete = target_session.complete(now).unwrap();

    // The narrow NIP-AB grant is complete before its deadline. Simulate a kill
    // after secure payload storage but before durable room enrollment starts;
    // the target comes back after the 120-second grant has expired.
    reqwest::Client::new()
        .post(format!("{server_url}/pairing-sessions/complete"))
        .json(&PublishPairingCompleteRequest {
            pairing_session_id: pairing_session_id.to_owned(),
            complete_event: serde_json::to_vec(&complete).unwrap(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    drop(hosted);
    let target = FiniteChatRuntime::open(OpenOptions {
        data_dir: root.path().join("resume-target").display().to_string(),
        server_url: server_url.clone(),
        device_id: target_device_id.to_owned(),
        account_secret_hex: Some(payload.account_secret_hex.clone()),
        now_unix_seconds: None,
    })
    .unwrap();
    target
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("resumed target publishes a KeyPackage");
    let resumed = app_with_fixed_device_link_now(config.clone(), now + 300);

    // Hold the global mutation lock with one authenticated enrollment. A
    // random capability miss must still finish immediately, proving misses
    // are rejected before they can serialize valid pairing work.
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let release_rx = std::sync::Arc::new(std::sync::Mutex::new(release_rx));
    let lock_hook = {
        let release_rx = std::sync::Arc::clone(&release_rx);
        std::sync::Arc::new(move || {
            entered_tx
                .send(())
                .expect("lock observer receives the valid enrollment");
            release_rx
                .lock()
                .unwrap()
                .recv()
                .expect("test releases valid enrollment");
        })
    };
    let contended =
        app_with_fixed_device_link_now_and_lock_hook(config.clone(), now + 300, lock_hook);
    let valid_app = contended.clone();
    let valid_capability = payload.enrollment_capability_hex.clone();
    let valid_request = tokio::spawn(async move {
        device_enrollment_for(
            valid_app,
            pairing_session_id,
            target_device_id,
            user_id,
            &valid_capability,
        )
        .await
    });
    tokio::task::spawn_blocking(move || {
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("valid enrollment acquires the mutation lock")
    })
    .await
    .unwrap();
    let miss = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        device_enrollment_for(
            contended,
            pairing_session_id,
            target_device_id,
            user_id,
            &"00".repeat(32),
        ),
    )
    .await
    .expect("random capability miss must not wait behind valid fanout");
    assert_eq!(miss.status(), StatusCode::NOT_FOUND);
    release_tx.send(()).unwrap();
    let first_resume_response = valid_request.await.unwrap();
    let first_resume_status = first_resume_response.status();
    let first_resume_body = first_resume_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(
        first_resume_status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first_resume_body)
    );
    let first_resume: Value = serde_json::from_slice(&first_resume_body).unwrap();
    assert!(
        first_resume["status"] == "joining_rooms" || first_resume["status"] == "ready",
        "durable resume must either advance enrollment or finish it: {first_resume}"
    );
    let mut ready = first_resume;
    for _ in 0..64 {
        if ready["status"] == "ready" {
            break;
        }
        target
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("resumed target advances its Welcome and complete bootstrap");
        ready = device_enrollment_json(
            resumed.clone(),
            pairing_session_id,
            target_device_id,
            user_id,
            &payload.enrollment_capability_hex,
        )
        .await;
        assert!(
            ready["status"] == "joining_rooms" || ready["status"] == "ready",
            "durable enrollment returned an invalid intermediate state: {ready}"
        );
    }
    assert_eq!(
        ready["status"], "ready",
        "durable enrollment did not converge within its bounded test budget: {ready}"
    );
    assert_eq!(ready["room_count"], 1);

    let record_path = device_link_record_path(&config.data_root, user_id, pairing_session_id);
    let completion_record: Value =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(
        completion_record["enrollment_completion"]["room_count"], 1,
        "Ready must be durable before the response is acknowledged"
    );
    let enrollment_expiry = completion_record["enrollment_expires_at_unix_seconds"]
        .as_u64()
        .unwrap();
    assert!(
        enrollment_expiry > descriptor.expires_at_unix_seconds,
        "durable enrollment needs a lifecycle distinct from the NIP-AB exchange"
    );

    let target_state = target
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("resumed target projects imported history");
    assert_eq!(target_state.rooms.len(), 1);

    // Once Ready is persisted, replay is a local tombstone lookup. It remains
    // safe and exact across restarts even if the pairing service is offline.
    drop(resumed);
    server_task.abort();
    let _ = server_task.await;
    let offline = app_with_fixed_device_link_now(config.clone(), now + 600);
    for _ in 0..2 {
        let replay = device_enrollment_json(
            offline.clone(),
            pairing_session_id,
            target_device_id,
            user_id,
            &payload.enrollment_capability_hex,
        )
        .await;
        assert_eq!(replay["status"], "ready");
        assert_eq!(replay["room_count"], 1);
    }

    // The consumed capability is retained only for its bounded replay window.
    // The first authenticated use after expiry removes the exact tombstone.
    let expired = app_with_fixed_device_link_now(config, enrollment_expiry + 1);
    let expired_response = device_enrollment_for(
        expired,
        pairing_session_id,
        target_device_id,
        user_id,
        &payload.enrollment_capability_hex,
    )
    .await;
    assert_eq!(expired_response.status(), StatusCode::NOT_FOUND);
    assert!(!record_path.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_reconciliation_requires_the_sealed_project_binding_and_resumes_fanout() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("reconcile-server.sqlite3"), None).await;
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: root.path().join("reconcile-agent").display().to_string(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: None,
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
        data_root: root.path().join("reconcile-hosted"),
        server_url: server_url.clone(),
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    };
    let hosted = app(config.clone());
    action_for(
        hosted.clone(),
        "reconcile-user",
        serde_json::json!({ "StartRuntime": null }),
    )
    .await;

    let missing = reconcile_device_for(
        hosted.clone(),
        "reconcile-user",
        "missing-project",
        "electron-reconcile",
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    binding_for(
        hosted.clone(),
        "reconcile-user",
        "/v1/app/agent-bindings/authorize-bootstrap",
        serde_json::json!({
            "project_id": "project-reconcile",
            "creation_request_id": "create-project-reconcile"
        }),
    )
    .await;
    let ensured = binding_for(
        hosted.clone(),
        "reconcile-user",
        "/v1/app/agent-bindings/ensure",
        serde_json::json!({
            "project_id": "project-reconcile",
            "agent_npub": agent_npub,
            "display_name": "Reconcile Agent"
        }),
    )
    .await;
    assert_eq!(ensured["rooms"].as_array().unwrap().len(), 1);
    action_for(
        hosted.clone(),
        "reconcile-user",
        serde_json::json!({ "CreateRoom": { "display_name": "Created before reconciliation" } }),
    )
    .await;

    let binding_root = config
        .data_root
        .join("users")
        .join(hex::encode(sha2::Sha256::digest(b"reconcile-user")))
        .join("agent-bindings");
    let valid_binding_path = binding_root.join(format!(
        "{}.json",
        hex::encode(sha2::Sha256::digest(b"project-reconcile"))
    ));
    let wrong_binding_path = binding_root.join(format!(
        "{}.json",
        hex::encode(sha2::Sha256::digest(b"wrong-project"))
    ));
    fs::copy(&valid_binding_path, &wrong_binding_path).unwrap();
    let wrong = reconcile_device_for(
        hosted.clone(),
        "reconcile-user",
        "wrong-project",
        "electron-reconcile",
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::SERVICE_UNAVAILABLE);

    let user_storage_id = hex::encode(sha2::Sha256::digest(b"reconcile-user"));
    let hosted_identity = FiniteIdentity::load(&IdentityPaths::with_finite_home(
        config
            .data_root
            .join("users")
            .join(user_storage_id)
            .join("finite-home"),
    ))
    .unwrap();
    let target = FiniteChatRuntime::open(OpenOptions {
        data_dir: root.path().join("reconcile-target").display().to_string(),
        server_url,
        device_id: "electron-reconcile".to_owned(),
        account_secret_hex: Some(hex::encode(hosted_identity.expose_secret_bytes())),
        now_unix_seconds: None,
    })
    .unwrap();

    let awaiting = reconcile_device_json(
        hosted.clone(),
        "reconcile-user",
        "project-reconcile",
        "electron-reconcile",
    )
    .await;
    assert_eq!(awaiting["status"], "awaiting_key_package");
    assert_eq!(awaiting["project_id"], "project-reconcile");
    assert_eq!(awaiting["target_device_id"], "electron-reconcile");
    assert_eq!(awaiting.as_object().unwrap().len(), 5);
    let awaiting_text = serde_json::to_string(&awaiting).unwrap();
    for forbidden in ["account_id", "room_id", "session", "secret"] {
        assert!(!awaiting_text.contains(forbidden));
    }

    target
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("target Device publishes KeyPackages");
    let joining = reconcile_device_json(
        hosted.clone(),
        "reconcile-user",
        "project-reconcile",
        "electron-reconcile",
    )
    .await;
    assert_eq!(joining["status"], "joining_rooms");
    assert_eq!(joining["room_count"], 2);
    assert_eq!(joining["active_room_count"], 0);

    let mut ready = None;
    for _ in 0..100 {
        target
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("target Device advances one bounded enrollment tick");
        let progress = reconcile_device_json(
            hosted.clone(),
            "reconcile-user",
            "project-reconcile",
            "electron-reconcile",
        )
        .await;
        if progress["status"] == "ready" {
            ready = Some(progress);
            break;
        }
        assert_eq!(progress["status"], "joining_rooms");
    }
    let ready = ready.expect("bounded source export reaches ready");
    assert_eq!(ready["status"], "ready");
    assert_eq!(ready["room_count"], 2);
    assert_eq!(ready["active_room_count"], 2);
    assert_eq!(
        reconcile_device_json(
            hosted,
            "reconcile-user",
            "project-reconcile",
            "electron-reconcile",
        )
        .await,
        ready
    );

    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_chunked_link_service_response_is_rejected() {
    let root = TempDir::new().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let oversized = || async {
        let stream = futures_util::stream::once(async {
            Ok::<_, Infallible>(axum::body::Bytes::from(vec![b'x'; 65 * 1024]))
        });
        axum::response::Response::new(Body::from_stream(stream))
    };
    let fake = axum::Router::new()
        .route("/pairing-sessions", axum::routing::post(oversized))
        .route("/pairing-sessions/get", axum::routing::post(oversized));
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
        "pairing-oversized-service",
        "electron-oversized-service",
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.len() < 1_024);
    let body_text = String::from_utf8_lossy(&body);
    assert!(body_text.contains("response is too large"), "{body_text}");
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
    let protected_url = "https://finite.computer/_admin/brains";
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
        "{{\"version\":\"finite-brain-admin-access-change-v1\",\"brainId\":\"personal\",\"changeId\":\"provider-access-change\",\"action\":\"add-member\",\"adminNpub\":\"{member_npub}\",\"targetNpub\":\"{member_npub}\",\"createdAt\":\"2026-07-13T12:00:00Z\"}}"
    );
    let access_change_input = serde_json::json!({
        "intent": "brain-access-change",
        "eventTemplate": {
            "kind": 30_078,
            "created_at": now,
            "tags": [
                ["d", "finite-brain-admin-access-change:personal:provider-access-change"],
                ["brain", "personal"],
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
                "brainId": "personal",
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
                "brainId": "personal",
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
    assert_eq!(opened["plaintext"]["brainId"], "personal");
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
                "brainId": "personal",
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
                "brainId": "personal",
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
    pairing_session_id: &str,
    target_device_id: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::post(path)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header(WORKOS_USER_HEADER, user_id)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "pairing_session_id": pairing_session_id,
                    "target_device_id": target_device_id,
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn device_enrollment_for(
    app: axum::Router,
    pairing_session_id: &str,
    target_device_id: &str,
    enrollment_user_id: &str,
    enrollment_capability_hex: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::post("/v1/device-links/enroll")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "pairing_session_id": pairing_session_id,
                    "target_device_id": target_device_id,
                    "enrollment_user_id": enrollment_user_id,
                    "enrollment_capability_hex": enrollment_capability_hex,
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn device_enrollment_json(
    app: axum::Router,
    pairing_session_id: &str,
    target_device_id: &str,
    enrollment_user_id: &str,
    enrollment_capability_hex: &str,
) -> Value {
    let response = device_enrollment_for(
        app,
        pairing_session_id,
        target_device_id,
        enrollment_user_id,
        enrollment_capability_hex,
    )
    .await;
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).unwrap()
}

fn device_link_record_path(
    data_root: &Path,
    user_id: &str,
    pairing_session_id: &str,
) -> std::path::PathBuf {
    data_root
        .join("users")
        .join(hex::encode(sha2::Sha256::digest(user_id.as_bytes())))
        .join("device-links")
        .join(format!(
            "{}.json",
            hex::encode(sha2::Sha256::digest(pairing_session_id.as_bytes()))
        ))
}

async fn reconcile_device_for(
    app: axum::Router,
    user_id: &str,
    project_id: &str,
    target_device_id: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::post("/v1/device-links/reconcile")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header(WORKOS_USER_HEADER, user_id)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "project_id": project_id,
                    "target_device_id": target_device_id,
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn reconcile_device_json(
    app: axum::Router,
    user_id: &str,
    project_id: &str,
    target_device_id: &str,
) -> Value {
    let response = reconcile_device_for(app, user_id, project_id, target_device_id).await;
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).unwrap()
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
