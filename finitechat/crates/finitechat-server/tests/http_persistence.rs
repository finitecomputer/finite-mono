use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::DefaultBodyLimit;
use axum::http::{Method, Request, Response, StatusCode};
use finitechat_blob::BlobDescriptor;
use finitechat_delivery::{
    HTTP_SERVER_SOURCE, HttpClaimedKeyPackage, HttpCommitAdmission, HttpDeliveryPlane,
    HttpKeyPackageId, HttpKeyPackagePublication, HttpPublishReceipt, HttpPublishTarget,
    HttpSyncPage, MAX_HTTP_ID_BYTES, MAX_HTTP_SYNC_PAGE_ENTRIES,
};
use finitechat_http::{
    AckLinkPayloadRequest, AckLinkPayloadResponse, AckPushWakeRequest, AckPushWakeResponse,
    AckWelcomeRequest, AckWelcomeResponse, ApplicationEffectCountsResponse,
    ApplicationEffectRequest, BootstrapAccountRoomRequest, BootstrapAccountRoomResponse,
    ClaimKeyPackageForAccountRequest, ClaimKeyPackageRequest, ClaimKeyPackagesRequest,
    ClaimLinkPayloadRequest, ClaimLinkPayloadResponse, ClaimPushWakesRequest,
    ClaimPushWakesResponse, ClaimWelcomesRequest, CreateLinkSessionRequest, DeviceLivenessRecord,
    ErrorResponse, ExpireKeyPackageLeaseRequest, ExpireKeyPackageLeaseResponse,
    ExpireLinkSessionRequest, FailPushWakeRequest, FailPushWakeResponse,
    FiniteAccountRoomCommitProjection, GetDeviceLivenessRequest, GetDeviceLivenessResponse,
    GetKeyPackageAvailabilityRequest, GetKeyPackageAvailabilityResponse, GetLinkSessionRequest,
    GetNostrProfilesRequest, GetNostrProfilesResponse, GroupSyncRequest,
    HttpApplicationDeliveryEffect, HttpClaimedWelcome, HttpKeyPackageClaim,
    HttpKeyPackageInventory, HttpLinkSessionRecord, HttpLinkSessionState, InboxSyncRequest,
    KeyPackageInventoryRequest, LeaveRoomRequest, LeaveRoomResponse,
    ListAccountRoomDirectoryRequest, ListAccountRoomDirectoryResponse, NostrProfileRecord,
    ObserveDeviceLivenessRequest, PublishKeyPackageResponse, PublishMessageRequest, PushPlatform,
    PutNostrProfileRequest, RegisterPushTokenRequest, ReleaseLinkClaimRequest,
    ReleaseLinkClaimResponse, RemovePushTokenRequest, RemovePushTokenResponse,
    ReportInvalidCommitRequest, ReportInvalidCommitResponse, RevokeDeviceRequest,
    SaveAccountRoomRequest, SaveAccountRoomResponse, SyncHintEvent, SyncStreamRequest,
    SyncWaitInbox, SyncWaitRequest, SyncWaitResponse, SyncWaitRoom, UpdateRoomAdminsRequest,
    UpdateRoomAdminsResponse, UploadLinkPayloadRequest,
};
use finitechat_proto::{
    AccountRoomDevice, AccountRoomRecord, AppendApplicationEventRequest,
    AppendEphemeralActivityRequest, AppendEventRequest, CommitAccepted, EphemeralActivityAccepted,
    EventAccepted, RoomProtocol, SubmitCommitRequest, UploadKeyPackageRequest, WelcomeRecord,
    delivery_member_id_for_device,
};
use finitechat_proto::{
    ApplicationDeliveryPolicy, CommandInboxPolicy, DeviceRef, DurableAppEventKind, FiniteEnvelope,
    LogEntryKind, MAX_ACCOUNT_DEVICES_PER_ROOM, MAX_DEVICE_LIVENESS_EXPIRY_MILLIS,
    MAX_ENVELOPE_PAYLOAD_BYTES, MAX_EPHEMERAL_ACTIVITY_CACHE_ENTRIES_PER_ROUTE,
    MAX_KEY_PACKAGES_PER_DEVICE, MAX_LINK_SESSION_PAYLOAD_BYTES, MembershipAddV1,
    MembershipDeltaV1, MembershipRemoveV1, PushPolicy, RoomStatus, RuntimeStateProjection,
    RuntimeStateProjectionEntry, RuntimeStateProjectionError, RuntimeStateSnapshotV1,
    StagedWelcomeV1, UnreadPolicy, WelcomeState,
};
use finitechat_server::{HttpServerState, ServerHttpError, http_router};
use finitechat_transport::engine::KeyPackage;
use finitechat_transport::transport::{
    Timestamp, TransportEnvelope, TransportMessage, TransportSource,
};
use finitechat_transport::{EpochId, GroupId, MemberId, MessageId};
use futures_util::StreamExt;
use rusqlite::{Connection, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tempfile::TempDir;
use tower::ServiceExt;

#[tokio::test]
async fn sqlite_blob_upload_download_survives_restart_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let ciphertext = b"encrypted attachment ciphertext";

    let descriptor = {
        let app = persistent_app(&db_path);
        let response = put_blob(app.clone(), ciphertext).await;
        assert_eq!(response.status(), StatusCode::OK);
        let descriptor: BlobDescriptor = read_json(response).await;
        assert_eq!(descriptor.size_bytes, ciphertext.len() as u64);
        assert_eq!(descriptor.sha256.len(), 64);

        let response = get_blob(app, &descriptor.sha256).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("application/octet-stream")
        );
        assert_eq!(read_body(response).await.as_ref(), ciphertext);
        descriptor
    };

    let app = persistent_app(&db_path);
    let response = get_blob(app.clone(), &descriptor.sha256).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(read_body(response).await.as_ref(), ciphertext);

    let response = put_blob(app, ciphertext).await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed: BlobDescriptor = read_json(response).await;
    assert_eq!(replayed.sha256, descriptor.sha256);
    assert_eq!(replayed.size_bytes, descriptor.size_bytes);
}

#[tokio::test]
async fn blob_descriptor_uses_configured_public_url_for_internal_uploads() {
    let temp = TempDir::new().expect("tempdir");
    let state = persistent_state(&temp.path().join("delivery.sqlite3"))
        .with_public_url("https://chat.finite.computer/")
        .expect("public URL");
    let app = http_router(state);
    let ciphertext = b"encrypted attachment from hosted device";

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/upload")
                .header("content-type", "application/octet-stream")
                .header("host", "127.0.0.1:8788")
                .body(Body::from(ciphertext.to_vec()))
                .expect("request"),
        )
        .await
        .expect("response");
    let descriptor: BlobDescriptor = read_json(response).await;

    assert_eq!(
        descriptor.url,
        format!("https://chat.finite.computer/blobs/{}", descriptor.sha256)
    );
}

#[test]
fn configured_public_url_rejects_path_prefix() {
    let temp = TempDir::new().expect("tempdir");
    let error = persistent_state(&temp.path().join("delivery.sqlite3"))
        .with_public_url("https://chat.finite.computer/internal")
        .expect_err("public URL is an origin, not a route prefix");

    assert!(error.to_string().contains("bare origin"));
}

#[tokio::test]
async fn sqlite_public_image_blob_upload_download_survives_restart_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let png = b"\x89PNG\r\n\x1a\nprofile image bytes";

    let descriptor = {
        let app = persistent_app(&db_path);
        let response = put_blob_with_content_type(app.clone(), png, "image/png").await;
        assert_eq!(response.status(), StatusCode::OK);
        let descriptor: BlobDescriptor = read_json(response).await;
        assert_eq!(descriptor.size_bytes, png.len() as u64);
        assert_eq!(descriptor.sha256.len(), 64);
        assert!(descriptor.url.contains("/blobs/"));

        let response = get_blob(app, &descriptor.sha256).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("image/png")
        );
        assert_eq!(read_body(response).await.as_ref(), png);
        descriptor
    };

    let app = persistent_app(&db_path);
    let response = get_blob(app.clone(), &descriptor.sha256).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(read_body(response).await.as_ref(), png);

    let response = put_blob_with_content_type(app, png, "image/png").await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed: BlobDescriptor = read_json(response).await;
    assert_eq!(replayed.sha256, descriptor.sha256);
    assert_eq!(replayed.size_bytes, descriptor.size_bytes);
}

#[tokio::test]
async fn public_image_blob_upload_rejects_mismatched_image_content() {
    let temp = TempDir::new().expect("tempdir");
    let app = persistent_app(&temp.path().join("delivery.sqlite3"));

    let response = put_blob_with_content_type(app, b"not actually an image", "image/png").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn public_image_blob_upload_rejects_oversized_image_content() {
    let temp = TempDir::new().expect("tempdir");
    let app = persistent_app(&temp.path().join("delivery.sqlite3"));
    let mut png = vec![0; 8 * 1024 * 1024 + 1];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");

    let response = put_blob_with_content_type(app, &png, "image/png").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn blob_download_rejects_bad_or_missing_hash() {
    let app = http_router(HttpServerState::default());

    let response = get_blob(app.clone(), "ABC").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: ErrorResponse = read_json(response).await;
    assert_eq!(body.kind, "invalid_blob_request");

    let missing = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let response = get_blob(app, missing).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body: ErrorResponse = read_json(response).await;
    assert_eq!(body.kind, "blob_not_found");
}

#[tokio::test]
async fn sqlite_publish_idempotency_replays_original_receipt_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let group_id = group_id("idempotent-group");
    let transport_group_id = b"idempotent-transport".to_vec();
    let request = PublishMessageRequest {
        target: group_target(group_id.clone(), transport_group_id.clone(), None),
        message: group_message("idempotent-message", transport_group_id, b"first body"),
        idempotency_key: Some("idem-message-1".to_owned()),
    };

    let state = persistent_state(&db_path);
    let accepted = state
        .publish_message(request.clone())
        .expect("first publish");
    assert_eq!(accepted.seq, 1);
    assert!(!accepted.duplicate);
    drop(state);

    let state = persistent_state(&db_path);
    let replayed = state
        .publish_message(request.clone())
        .expect("idempotent replay");
    assert_eq!(replayed, accepted);
    assert!(!replayed.duplicate);
    let app = http_router(state);

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id,
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].message.id, id("idempotent-message"));
}

#[tokio::test]
async fn sqlite_publish_idempotency_rejects_same_key_with_different_body() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let group_id = group_id("idempotency-conflict-group");
    let transport_group_id = b"idempotency-conflict-transport".to_vec();
    let first = PublishMessageRequest {
        target: group_target(group_id.clone(), transport_group_id.clone(), None),
        message: group_message(
            "idempotency-conflict-a",
            transport_group_id.clone(),
            b"first",
        ),
        idempotency_key: Some("idem-conflict".to_owned()),
    };
    let conflicting = PublishMessageRequest {
        target: group_target(group_id.clone(), transport_group_id.clone(), None),
        message: group_message("idempotency-conflict-b", transport_group_id, b"second"),
        idempotency_key: Some("idem-conflict".to_owned()),
    };

    let state = persistent_state(&db_path);
    state.publish_message(first.clone()).expect("first publish");
    drop(state);

    let state = persistent_state(&db_path);
    let error = state
        .publish_message(conflicting.clone())
        .expect_err("conflicting idempotency key rejected");
    assert!(matches!(
        error,
        ServerHttpError::IdempotencyConflict { ref idempotency_key }
            if idempotency_key == "idem-conflict"
    ));
    let app = http_router(state);

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id,
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].message.id, id("idempotency-conflict-a"));
}

#[tokio::test]
async fn sqlite_log_rebuilds_key_package_claim_state_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let owner = member("durable-owner");
    let key_package_id = HttpKeyPackageId::new(b"durable-kp".to_vec());
    let publication = HttpKeyPackagePublication {
        key_package_id: key_package_id.clone(),
        owner: owner.clone(),
        key_package: KeyPackage::new(b"durable-key-package".to_vec()),
    };

    let app = persistent_app(&db_path);
    assert_eq!(
        post_json(app.clone(), "/key-packages", &publication)
            .await
            .status(),
        StatusCode::OK
    );
    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(
        claimed
            .expect("claim before restart")
            .key_package_id
            .as_slice(),
        key_package_id.as_slice()
    );

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest { owner },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(claimed, None);
}

#[tokio::test]
async fn sqlite_key_package_claim_uses_route_owner_and_preserves_opaque_payload() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let route_owner = member("bob-device");
    let untrusted_payload_owner = member("mallory-device");
    let payload_with_untrusted_claim =
        br#"{"claimed_owner":"mallory-device","claimed_device":"phone"}"#.to_vec();
    let publication = key_package_publication(
        "kp-untrusted-payload-identity",
        route_owner.clone(),
        &payload_with_untrusted_claim,
    );

    let app = persistent_app(&db_path);
    assert_eq!(
        post_json(app.clone(), "/key-packages", &publication)
            .await
            .status(),
        StatusCode::OK
    );
    assert_inventory(app.clone(), route_owner.clone(), 1, 0).await;
    assert_inventory(app, untrusted_payload_owner.clone(), 0, 0).await;

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: untrusted_payload_owner,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(claimed, None);
    assert_inventory(app.clone(), route_owner.clone(), 1, 0).await;

    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: route_owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    let claimed = claimed.expect("route owner claims package");
    assert_eq!(claimed.owner, route_owner.clone());
    assert_eq!(claimed.key_package_id, publication.key_package_id);
    assert_eq!(claimed.key_package.bytes, payload_with_untrusted_claim);
    assert_inventory(app, route_owner.clone(), 0, 1).await;

    let app = persistent_app(&db_path);
    assert_inventory(app.clone(), route_owner.clone(), 0, 1).await;
    let response = post_json(
        app,
        "/key-packages/claim",
        &ClaimKeyPackageRequest { owner: route_owner },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(claimed, None);
}

#[tokio::test]
async fn sqlite_key_package_account_claim_selects_available_unrevoked_device() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned();
    let phone = DeviceRef::new(account_id.clone(), "phone");
    let laptop = DeviceRef::new(account_id.clone(), "laptop");
    let other = DeviceRef::new(
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "phone",
    );
    let publications = [
        finite_key_package_publication(
            &phone,
            "kp-account-phone",
            "ref-phone",
            "hash-phone",
            b"phone",
        ),
        finite_key_package_publication(
            &laptop,
            "kp-account-laptop",
            "ref-laptop",
            "hash-laptop",
            b"laptop",
        ),
        finite_key_package_publication(
            &other,
            "kp-other-phone",
            "ref-other",
            "hash-other",
            b"other",
        ),
    ];

    let app = persistent_app(&db_path);
    for publication in &publications {
        assert_eq!(
            post_json(app.clone(), "/key-packages", publication)
                .await
                .status(),
            StatusCode::OK
        );
    }
    revoke_device(&app, &laptop).await;

    let response = post_json(
        app.clone(),
        "/key-packages/claim-account",
        &ClaimKeyPackageForAccountRequest {
            account_id: account_id.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    let claimed = claimed.expect("account claim finds unrevoked phone package");
    assert_eq!(claimed.owner, member_for_device(&phone));
    assert_eq!(claimed.key_package_id.as_slice(), b"kp-account-phone");
    assert_inventory(app.clone(), member_for_device(&phone), 0, 1).await;
    assert_inventory(app.clone(), member_for_device(&laptop), 1, 0).await;
    assert_inventory(app.clone(), member_for_device(&other), 1, 0).await;

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/key-packages/claim-account",
        &ClaimKeyPackageForAccountRequest { account_id },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(claimed, None);
}

#[tokio::test]
async fn sqlite_key_package_account_claim_uses_current_timestamped_package() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned();
    let phone = DeviceRef::new(account_id.clone(), "phone");
    let stale = finite_key_package_publication(
        &phone,
        "kp_ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "ref-stale",
        "hash-stale",
        b"stale",
    );
    let current = finite_key_package_publication(
        &phone,
        "kp_t00000000001800000000_0000000000000000000000000000000000000000000000000000000000000001",
        "ref-current",
        "hash-current",
        b"current",
    );

    let app = persistent_app(&db_path);
    assert_eq!(
        post_json(app.clone(), "/key-packages", &stale)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        post_json(app.clone(), "/key-packages", &current)
            .await
            .status(),
        StatusCode::OK
    );
    assert_inventory(app.clone(), member_for_device(&phone), 1, 0).await;

    let response = post_json(
        app.clone(),
        "/key-packages/claim-account",
        &ClaimKeyPackageForAccountRequest {
            account_id: account_id.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    let claimed = claimed.expect("account claim finds current package");
    assert_eq!(claimed.key_package_id, current.key_package_id);
    assert_eq!(claimed.key_package.bytes(), current.key_package.bytes());
    assert_inventory(app.clone(), member_for_device(&phone), 0, 1).await;

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/key-packages/claim-account",
        &ClaimKeyPackageForAccountRequest { account_id },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(
        claimed, None,
        "old hash-only packages retired by a timestamped publish must not reappear after restart"
    );
}

#[tokio::test]
async fn sqlite_key_package_availability_batches_accounts_without_claiming_key_packages() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new(String::from_utf8(vec![b'a'; 64]).unwrap(), "phone");
    let bob = DeviceRef::new(String::from_utf8(vec![b'b'; 64]).unwrap(), "phone");
    let carol = DeviceRef::new(String::from_utf8(vec![b'c'; 64]).unwrap(), "phone");
    let dave = DeviceRef::new(String::from_utf8(vec![b'd'; 64]).unwrap(), "phone");

    let carol_owner = member_for_device(&carol);

    let app = persistent_app(&db_path);
    for publication in [
        finite_key_package_publication(
            &alice,
            "kp-alice-available",
            "ref-alice",
            "hash-alice",
            b"alice",
        ),
        finite_key_package_publication(
            &carol,
            "kp-carol-claimed",
            "ref-carol",
            "hash-carol",
            b"carol",
        ),
        finite_key_package_publication(&dave, "kp-dave-revoked", "ref-dave", "hash-dave", b"dave"),
    ] {
        assert_eq!(
            post_json(app.clone(), "/key-packages", &publication)
                .await
                .status(),
            StatusCode::OK
        );
    }
    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: carol_owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert!(claimed.is_some());
    revoke_device(&app, &dave).await;

    let response = post_json(
        app.clone(),
        "/key-packages/availability",
        &GetKeyPackageAvailabilityRequest {
            account_ids: vec![
                alice.account_id.clone(),
                bob.account_id.clone(),
                carol.account_id.clone(),
                dave.account_id.clone(),
            ],
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let availability: GetKeyPackageAvailabilityResponse = read_json(response).await;
    assert_eq!(
        availability
            .accounts
            .into_iter()
            .map(|entry| (entry.account_id, entry.available))
            .collect::<Vec<_>>(),
        vec![
            (alice.account_id.clone(), true),
            (bob.account_id.clone(), false),
            (carol.account_id.clone(), false),
            (dave.account_id.clone(), false),
        ]
    );

    assert_inventory(app.clone(), member_for_device(&alice), 1, 0).await;
    assert_inventory(app.clone(), carol_owner, 0, 1).await;
    assert_inventory(app, member_for_device(&dave), 1, 0).await;

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/key-packages/availability",
        &GetKeyPackageAvailabilityRequest {
            account_ids: vec![alice.account_id.clone(), dave.account_id.clone()],
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let availability: GetKeyPackageAvailabilityResponse = read_json(response).await;
    assert_eq!(
        availability
            .accounts
            .into_iter()
            .map(|entry| (entry.account_id, entry.available))
            .collect::<Vec<_>>(),
        vec![(alice.account_id, true), (dave.account_id, false)]
    );
}

#[tokio::test]
async fn sqlite_key_package_inventory_tracks_available_and_claimed_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let owner = member("inventory-owner");
    let first = key_package_publication("kp-inventory-a", owner.clone(), b"inventory-a");
    let second = key_package_publication("kp-inventory-b", owner.clone(), b"inventory-b");

    let app = persistent_app(&db_path);
    assert_eq!(
        post_json(app.clone(), "/key-packages", &first)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        post_json(app.clone(), "/key-packages", &second)
            .await
            .status(),
        StatusCode::OK
    );
    assert_inventory(app.clone(), owner.clone(), 2, 0).await;

    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(
        claimed
            .as_ref()
            .expect("first package claimed")
            .key_package_id
            .as_slice(),
        b"kp-inventory-a"
    );
    assert_inventory(app, owner.clone(), 1, 1).await;

    let app = persistent_app(&db_path);
    assert_inventory(app.clone(), owner.clone(), 1, 1).await;

    assert_eq!(
        post_json(app.clone(), "/key-packages", &first)
            .await
            .status(),
        StatusCode::OK
    );
    assert_inventory(app, owner, 1, 1).await;
}

#[tokio::test]
async fn sqlite_key_package_inventory_cap_counts_claimed_and_consumed_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-key-package-inventory-cap".to_owned();
    let mls_group_id = "mls-key-package-inventory-cap".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let request = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-inventory-cap-00",
        "kp-inventory-cap-00",
    );
    let add = request
        .membership_delta
        .adds
        .first()
        .expect("add-device request has one add");

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    for index in 0..MAX_KEY_PACKAGES_PER_DEVICE {
        let (key_package_id, key_package_ref, key_package_hash) = if index == 0 {
            (
                add.key_package_id.clone(),
                add.key_package_ref.clone(),
                add.key_package_hash.clone(),
            )
        } else {
            (
                format!("kp-inventory-cap-{index:02}"),
                format!("ref-kp-inventory-cap-{index:02}"),
                format!("hash-kp-inventory-cap-{index:02}"),
            )
        };
        let response = post_json(
            app.clone(),
            "/key-packages",
            &finite_key_package_publication(
                &bob,
                &key_package_id,
                &key_package_ref,
                &key_package_hash,
                format!("payload-kp-inventory-cap-{index:02}").as_bytes(),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let inventory = key_package_inventory_for_device(&app, &bob).await;
    assert_eq!(inventory.available, MAX_KEY_PACKAGES_PER_DEVICE);
    assert_eq!(inventory.claimed, 0);

    let response = post_json(
        app.clone(),
        "/key-packages",
        &finite_key_package_publication(
            &bob,
            "kp-inventory-cap-overflow",
            "ref-kp-inventory-cap-overflow",
            "hash-kp-inventory-cap-overflow",
            b"payload-kp-inventory-cap-overflow",
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "key_package_inventory_full");

    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: member_for_device(&bob),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(
        claimed.expect("first package claimed").key_package_id,
        HttpKeyPackageId::new(add.key_package_id.as_bytes().to_vec())
    );
    let inventory = key_package_inventory_for_device(&app, &bob).await;
    assert_eq!(inventory.available, MAX_KEY_PACKAGES_PER_DEVICE - 1);
    assert_eq!(inventory.claimed, 1);

    let response = post_json(
        app.clone(),
        "/key-packages",
        &finite_key_package_publication(
            &bob,
            "kp-inventory-cap-still-full",
            "ref-kp-inventory-cap-still-full",
            "hash-kp-inventory-cap-still-full",
            b"payload-kp-inventory-cap-still-full",
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "key_package_inventory_full");

    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);
    let inventory = key_package_inventory_for_device(&app, &bob).await;
    assert_eq!(inventory.available, MAX_KEY_PACKAGES_PER_DEVICE - 1);
    assert_eq!(inventory.claimed, 0);

    let app = persistent_app(&db_path);
    let inventory = key_package_inventory_for_device(&app, &bob).await;
    assert_eq!(inventory.available, MAX_KEY_PACKAGES_PER_DEVICE - 1);
    assert_eq!(inventory.claimed, 0);
    let response = post_json(
        app.clone(),
        "/key-packages",
        &finite_key_package_publication(
            &bob,
            "kp-inventory-cap-replacement",
            "ref-kp-inventory-cap-replacement",
            "hash-kp-inventory-cap-replacement",
            b"payload-kp-inventory-cap-replacement",
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let inventory = key_package_inventory_for_device(&app, &bob).await;
    assert_eq!(inventory.available, MAX_KEY_PACKAGES_PER_DEVICE);
    assert_eq!(inventory.claimed, 0);
}

#[tokio::test]
async fn sqlite_key_package_publish_retry_and_conflict_survive_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let owner = member("publish-retry-owner");
    let original = key_package_publication("kp-publish-retry", owner.clone(), b"original-package");
    let conflicting =
        key_package_publication("kp-publish-retry", owner.clone(), b"conflicting-package");

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/key-packages", &original).await;
    assert_eq!(response.status(), StatusCode::OK);
    let published: PublishKeyPackageResponse = read_json(response).await;
    assert!(published.published);
    assert_inventory(app, owner.clone(), 1, 0).await;

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/key-packages", &original).await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed: PublishKeyPackageResponse = read_json(response).await;
    assert!(replayed.published);
    assert_inventory(app.clone(), owner.clone(), 1, 0).await;

    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    let claimed = claimed.expect("exact replay leaves one claimable KeyPackage");
    assert_eq!(claimed.key_package_id, original.key_package_id);
    assert_eq!(claimed.owner, owner.clone());
    assert_eq!(claimed.key_package, original.key_package);
    assert_inventory(app, owner.clone(), 0, 1).await;

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/key-packages", &conflicting).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "conflicting_key_package");
    assert_inventory(app.clone(), owner.clone(), 0, 1).await;

    let response = post_json(
        app,
        "/key-packages/claim",
        &ClaimKeyPackageRequest { owner },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert!(
        claimed.is_none(),
        "conflicting retry must not create a second claimable KeyPackage"
    );
}

#[tokio::test]
async fn sqlite_key_package_lease_expiry_and_reclaim_survives_restart_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let owner = member("lease-owner");
    let key_package_id = HttpKeyPackageId::new(b"kp-lease-reclaim".to_vec());
    let publication = HttpKeyPackagePublication {
        key_package_id: key_package_id.clone(),
        owner: owner.clone(),
        key_package: KeyPackage::new(b"lease-reclaim-package".to_vec()),
    };

    let app = persistent_app(&db_path);
    assert_eq!(
        post_json(app.clone(), "/key-packages", &publication)
            .await
            .status(),
        StatusCode::OK
    );
    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(
        claimed.as_ref().expect("first claim").key_package_id,
        key_package_id
    );
    assert_inventory(app.clone(), owner.clone(), 0, 1).await;
    let response = post_json(
        app.clone(),
        "/key-packages/leases/expire",
        &ExpireKeyPackageLeaseRequest {
            key_package_id: key_package_id.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let expired: ExpireKeyPackageLeaseResponse = read_json(response).await;
    assert!(expired.expired);
    assert_inventory(app.clone(), owner.clone(), 1, 0).await;

    let app = persistent_app(&db_path);
    assert_inventory(app.clone(), owner.clone(), 1, 0).await;
    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let reclaimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    let reclaimed = reclaimed.expect("reclaimed package");
    assert_eq!(reclaimed.key_package_id, key_package_id);
    assert_eq!(reclaimed.owner, owner);
    assert_eq!(reclaimed.key_package, publication.key_package);
    assert_inventory(app, member("lease-owner"), 0, 1).await;
}

#[tokio::test]
async fn sqlite_revoked_device_status_survives_restart_and_blocks_key_packages_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let bob = DeviceRef::new("bob", "bob-phone");
    let owner = member_for_device(&bob);
    let first = finite_key_package_publication(
        &bob,
        "kp-revoked-bob-1",
        "ref-revoked-one",
        "hash-revoked-one",
        b"revoked-one",
    );
    let second = finite_key_package_publication(
        &bob,
        "kp-revoked-bob-2",
        "ref-revoked-two",
        "hash-revoked-two",
        b"revoked-two",
    );

    let app = persistent_app(&db_path);
    assert_eq!(
        post_json(app.clone(), "/key-packages", &first)
            .await
            .status(),
        StatusCode::OK
    );
    assert_inventory(app.clone(), owner.clone(), 1, 0).await;

    let response = post_json(
        app.clone(),
        "/devices/revoke",
        &RevokeDeviceRequest {
            device: bob.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/key-packages", &second).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_revoked");

    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_revoked");

    let response = post_json(
        app.clone(),
        "/key-packages/claims",
        &ClaimKeyPackagesRequest {
            owners: vec![owner.clone()],
            idempotency_key: Some("revoked-owner-batch".to_owned()),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claims: Vec<HttpKeyPackageClaim> = read_json(response).await;
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].owner, owner.clone());
    assert!(claims[0].claimed.is_none());
    assert_inventory(app, owner, 1, 0).await;
}

#[tokio::test]
async fn sqlite_revoked_device_blocks_welcome_activation_and_typed_routes_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let pending_room_id = "room-revoked-pending".to_owned();
    let pending_mls_group_id = "mls-revoked-pending".to_owned();
    let active_room_id = "room-revoked-active".to_owned();
    let active_mls_group_id = "mls-revoked-active".to_owned();
    let target_room_id = "room-revoked-target".to_owned();
    let target_mls_group_id = "mls-revoked-target".to_owned();
    let pending_add = submit_add_device_request(
        &pending_room_id,
        &pending_mls_group_id,
        &alice,
        &bob,
        "welcome-revoked-pending",
        "commit-revoked-pending",
    );
    let active_add = submit_add_device_request(
        &active_room_id,
        &active_mls_group_id,
        &alice,
        &bob,
        "welcome-revoked-active",
        "commit-revoked-active",
    );

    let app = persistent_app(&db_path);
    for (room_id, mls_group_id) in [
        (&pending_room_id, &pending_mls_group_id),
        (&active_room_id, &active_mls_group_id),
    ] {
        let response = post_json(
            app.clone(),
            "/account-rooms/bootstrap",
            &BootstrapAccountRoomRequest {
                room_id: room_id.clone(),
                mls_group_id: mls_group_id.clone(),
                creator: alice.clone(),
                protocol: RoomProtocol::default(),
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    publish_and_claim_key_package_for_add(&app, &pending_add).await;
    let response = post_json(app.clone(), "/commits", &pending_add).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: member_for_device(&bob),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let pending_claims: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(pending_claims.len(), 1);
    assert_eq!(pending_claims[0].message.id, id("welcome-revoked-pending"));

    publish_and_claim_key_package_for_add(&app, &active_add).await;
    let response = post_json(app.clone(), "/commits", &active_add).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: member_for_device(&bob),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let active_claims: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(active_claims.len(), 1);
    assert_eq!(active_claims[0].message.id, id("welcome-revoked-active"));
    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-revoked-active"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    revoke_device(&app, &bob).await;

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: member_for_device(&bob),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_revoked");

    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-revoked-pending"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_revoked");
    let page = account_room_page(&app, "bob").await;
    let pending_room = page
        .rooms
        .iter()
        .find(|room| room["room_id"].as_str() == Some(pending_room_id.as_str()))
        .expect("pending room");
    let pending_bob = pending_room["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .find(|device| device["device"]["device_id"] == "bob-phone")
        .expect("pending Bob device");
    assert_eq!(pending_bob["active"], false);

    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &active_room_id,
            &active_mls_group_id,
            &bob,
            1,
            b"revoked-send",
            "revoked-send-idempotency",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_revoked");

    let remove = submit_remove_device_request(
        &active_room_id,
        &active_mls_group_id,
        &bob,
        &alice,
        1,
        "revoked-commit-idempotency",
    );
    let response = post_json(app.clone(), "/commits", &remove).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_revoked");

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: target_room_id.clone(),
            mls_group_id: target_mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let target_add = submit_add_device_request(
        &target_room_id,
        &target_mls_group_id,
        &alice,
        &bob,
        "welcome-revoked-target",
        "commit-revoked-target",
    );
    let response = post_json(app, "/commits", &target_add).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_revoked");
}

#[tokio::test]
async fn sqlite_batch_key_package_claim_replays_exact_response_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let phone = member("alice-phone");
    let laptop = member("alice-laptop");
    let missing = member("alice-tablet");
    let other = member("bob-phone");

    let app = persistent_app(&db_path);
    for publication in [
        key_package_publication("kp-phone-1", phone.clone(), b"phone-one"),
        key_package_publication("kp-phone-2", phone.clone(), b"phone-two"),
        key_package_publication("kp-laptop-1", laptop.clone(), b"laptop-one"),
        key_package_publication("kp-other-1", other.clone(), b"other-one"),
    ] {
        assert_eq!(
            post_json(app.clone(), "/key-packages", &publication)
                .await
                .status(),
            StatusCode::OK
        );
    }

    let request = ClaimKeyPackagesRequest {
        owners: vec![laptop.clone(), phone.clone(), missing.clone()],
        idempotency_key: Some("fanout-claim-replay".to_owned()),
    };
    let response = post_json(app, "/key-packages/claims", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Vec<HttpKeyPackageClaim> = read_json(response).await;
    assert_eq!(claimed.len(), 3);
    assert_eq!(claimed[0].owner, laptop);
    assert_eq!(
        claimed[0]
            .claimed
            .as_ref()
            .expect("laptop claim")
            .key_package_id
            .as_slice(),
        b"kp-laptop-1"
    );
    assert_eq!(claimed[1].owner, phone.clone());
    assert_eq!(
        claimed[1]
            .claimed
            .as_ref()
            .expect("phone claim")
            .key_package_id
            .as_slice(),
        b"kp-phone-1"
    );
    assert_eq!(claimed[2].owner, missing);
    assert_eq!(claimed[2].claimed, None);

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/key-packages/claims", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed: Vec<HttpKeyPackageClaim> = read_json(response).await;
    assert_eq!(replayed, claimed);

    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: phone.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let remaining_phone: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(
        remaining_phone
            .expect("second phone package remains available")
            .key_package_id
            .as_slice(),
        b"kp-phone-2"
    );

    let response = post_json(
        app,
        "/key-packages/claim",
        &ClaimKeyPackageRequest { owner: other },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let other_claim: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(
        other_claim
            .expect("other owner package remains available")
            .key_package_id
            .as_slice(),
        b"kp-other-1"
    );
}

#[tokio::test]
async fn sqlite_batch_key_package_claim_conflict_has_no_side_effects() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let phone = member("conflict-phone");
    let laptop = member("conflict-laptop");

    let app = persistent_app(&db_path);
    for publication in [
        key_package_publication("kp-conflict-phone", phone.clone(), b"phone"),
        key_package_publication("kp-conflict-laptop", laptop.clone(), b"laptop"),
    ] {
        assert_eq!(
            post_json(app.clone(), "/key-packages", &publication)
                .await
                .status(),
            StatusCode::OK
        );
    }

    let first = ClaimKeyPackagesRequest {
        owners: vec![phone.clone()],
        idempotency_key: Some("fanout-conflict".to_owned()),
    };
    assert_eq!(
        post_json(app.clone(), "/key-packages/claims", &first)
            .await
            .status(),
        StatusCode::OK
    );

    let conflicting = ClaimKeyPackagesRequest {
        owners: vec![laptop.clone()],
        idempotency_key: Some("fanout-conflict".to_owned()),
    };
    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/key-packages/claims", &conflicting).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "idempotency_conflict");

    let response = post_json(
        app,
        "/key-packages/claim",
        &ClaimKeyPackageRequest { owner: laptop },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let laptop_claim: Option<HttpClaimedKeyPackage> = read_json(response).await;
    assert_eq!(
        laptop_claim
            .expect("conflict must not consume laptop package")
            .key_package_id
            .as_slice(),
        b"kp-conflict-laptop"
    );
}

#[tokio::test]
async fn sqlite_link_session_state_machine_survives_restart_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let link_session_id = "link-http-session".to_owned();
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/link-sessions",
        &CreateLinkSessionRequest {
            link_session_id: link_session_id.clone(),
            pairing_public_key: "pairing-key-http".to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let record: HttpLinkSessionRecord = read_json(response).await;
    assert_eq!(record.state, HttpLinkSessionState::Created);
    assert!(record.encrypted_payload.is_none());

    let response = post_json(
        app.clone(),
        "/link-sessions",
        &CreateLinkSessionRequest {
            link_session_id: link_session_id.clone(),
            pairing_public_key: "pairing-key-http".to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "link_session_already_exists");

    let payload = b"ciphertext:server-list-and-authorization".to_vec();
    let response = post_json(
        app.clone(),
        "/link-sessions/payload",
        &UploadLinkPayloadRequest {
            link_session_id: link_session_id.clone(),
            encrypted_payload: payload.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let record: HttpLinkSessionRecord = read_json(response).await;
    assert_eq!(record.state, HttpLinkSessionState::PayloadUploaded);
    assert_eq!(record.encrypted_payload, Some(payload.clone()));

    let response = post_json(
        app.clone(),
        "/link-sessions/payload",
        &UploadLinkPayloadRequest {
            link_session_id: link_session_id.clone(),
            encrypted_payload: payload.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app.clone(),
        "/link-sessions/payload",
        &UploadLinkPayloadRequest {
            link_session_id: link_session_id.clone(),
            encrypted_payload: b"different".to_vec(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "link_session_conflict");

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/link-sessions/claim",
        &ClaimLinkPayloadRequest {
            link_session_id: link_session_id.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claim: ClaimLinkPayloadResponse = read_json(response).await;
    assert_eq!(claim.encrypted_payload, payload);
    assert!(!claim.claim_token.is_empty());

    let response = post_json(
        app.clone(),
        "/link-sessions/release",
        &ReleaseLinkClaimRequest {
            link_session_id: link_session_id.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let release: ReleaseLinkClaimResponse = read_json(response).await;
    assert!(release.released);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/link-sessions/claim",
        &ClaimLinkPayloadRequest {
            link_session_id: link_session_id.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let retry_claim: ClaimLinkPayloadResponse = read_json(response).await;
    assert_eq!(retry_claim.claim_token, claim.claim_token);

    let response = post_json(
        app.clone(),
        "/link-sessions/ack",
        &AckLinkPayloadRequest {
            link_session_id: link_session_id.clone(),
            claim_token: "wrong-token".to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "bad_link_session_claim_token");

    let response = post_json(
        app.clone(),
        "/link-sessions/ack",
        &AckLinkPayloadRequest {
            link_session_id: link_session_id.clone(),
            claim_token: retry_claim.claim_token.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let ack: AckLinkPayloadResponse = read_json(response).await;
    assert!(ack.acked);

    let response = post_json(
        app.clone(),
        "/link-sessions/payload",
        &UploadLinkPayloadRequest {
            link_session_id: link_session_id.clone(),
            encrypted_payload: b"late".to_vec(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "link_session_closed");

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/link-sessions/get",
        &GetLinkSessionRequest {
            link_session_id: link_session_id.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let record: Option<HttpLinkSessionRecord> = read_json(response).await;
    let record = record.expect("stored link session");
    assert_eq!(record.state, HttpLinkSessionState::Delivered);
    assert_eq!(record.claim_token, Some(retry_claim.claim_token));

    let response = post_json(
        app.clone(),
        "/link-sessions",
        &CreateLinkSessionRequest {
            link_session_id: "link-http-expired".to_owned(),
            pairing_public_key: "pairing-expired".to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = post_json(
        app.clone(),
        "/link-sessions/expire",
        &ExpireLinkSessionRequest {
            link_session_id: "link-http-expired".to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/link-sessions/payload",
        &UploadLinkPayloadRequest {
            link_session_id: "link-http-expired".to_owned(),
            encrypted_payload: b"late".to_vec(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "link_session_closed");
}

#[tokio::test]
async fn sqlite_link_session_payload_limit_rejects_without_persisting_payload() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let link_session_id = "link-http-payload-limit".to_owned();
    // Raise the harness body limit so this test reaches the adapter limit.
    let body_limit = (MAX_LINK_SESSION_PAYLOAD_BYTES as usize + 1) * 4;
    let app = persistent_app(&db_path).layer(DefaultBodyLimit::max(body_limit));

    let response = post_json(
        app.clone(),
        "/link-sessions",
        &CreateLinkSessionRequest {
            link_session_id: link_session_id.clone(),
            pairing_public_key: "pairing-limit".to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app.clone(),
        "/link-sessions/payload",
        &UploadLinkPayloadRequest {
            link_session_id: link_session_id.clone(),
            encrypted_payload: vec![0; MAX_LINK_SESSION_PAYLOAD_BYTES as usize + 1],
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_link_session_request");
    assert!(error.error.contains("link_session.encrypted_payload"));

    let app = persistent_app(&db_path).layer(DefaultBodyLimit::max(body_limit));
    let response = post_json(
        app,
        "/link-sessions/get",
        &GetLinkSessionRequest { link_session_id },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let record: Option<HttpLinkSessionRecord> = read_json(response).await;
    let record = record.expect("created link session survives failed upload");
    assert_eq!(record.state, HttpLinkSessionState::Created);
    assert!(record.encrypted_payload.is_none());
}

#[tokio::test]
async fn sqlite_account_room_directory_pages_and_survives_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let first_record = AccountRoomRecord {
        room_id: "room-a".to_owned(),
        mls_group_id: "mls-a".to_owned(),
        current_epoch: 1,
        last_seq: 7,
        status: RoomStatus::Open,
        devices: vec![
            AccountRoomDevice {
                device: DeviceRef {
                    account_id: "bob".to_owned(),
                    device_id: "bob-laptop".to_owned(),
                },
                active: true,
            },
            AccountRoomDevice {
                device: DeviceRef {
                    account_id: "alice".to_owned(),
                    device_id: "alice-laptop".to_owned(),
                },
                active: true,
            },
        ],
    };
    let first_expected = AccountRoomRecord {
        devices: vec![AccountRoomDevice {
            device: DeviceRef {
                account_id: "alice".to_owned(),
                device_id: "alice-laptop".to_owned(),
            },
            active: true,
        }],
        ..first_record.clone()
    };
    let second_record = AccountRoomRecord {
        room_id: "room-b".to_owned(),
        mls_group_id: "mls-b".to_owned(),
        current_epoch: 3,
        last_seq: 11,
        status: RoomStatus::Open,
        devices: vec![
            AccountRoomDevice {
                device: DeviceRef {
                    account_id: "alice".to_owned(),
                    device_id: "alice-laptop".to_owned(),
                },
                active: true,
            },
            AccountRoomDevice {
                device: DeviceRef {
                    account_id: "alice".to_owned(),
                    device_id: "alice-phone".to_owned(),
                },
                active: false,
            },
        ],
    };
    let first = SaveAccountRoomRequest {
        account_id: "alice".to_owned(),
        room_id: "room-a".to_owned(),
        record: serde_json::to_value(&first_record).expect("first record json"),
    };
    let second = SaveAccountRoomRequest {
        account_id: "alice".to_owned(),
        room_id: "room-b".to_owned(),
        record: serde_json::to_value(&second_record).expect("second record json"),
    };
    let wrong_account = SaveAccountRoomRequest {
        account_id: "alice".to_owned(),
        room_id: "room-wrong".to_owned(),
        record: serde_json::to_value(&AccountRoomRecord {
            room_id: "room-wrong".to_owned(),
            mls_group_id: "mls-wrong".to_owned(),
            current_epoch: 1,
            last_seq: 3,
            status: RoomStatus::Open,
            devices: vec![AccountRoomDevice {
                device: DeviceRef {
                    account_id: "bob".to_owned(),
                    device_id: "bob-laptop".to_owned(),
                },
                active: true,
            }],
        })
        .expect("wrong-account record json"),
    };

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/account-rooms", &second).await;
    assert_eq!(response.status(), StatusCode::OK);
    let saved: SaveAccountRoomResponse = read_json(response).await;
    assert!(saved.saved);
    assert_eq!(
        post_json(app.clone(), "/account-rooms", &first)
            .await
            .status(),
        StatusCode::OK
    );
    let response = post_json(app, "/account-rooms", &wrong_account).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_account_room_request");

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: "alice".to_owned(),
            after_room_id: None,
            limit: 1,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert_eq!(
        page.rooms,
        vec![serde_json::to_value(&first_expected).expect("first expected json")]
    );
    assert_eq!(page.next_after_room_id.as_deref(), Some("room-a"));
    assert!(page.has_more);

    let response = post_json(
        app,
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: "alice".to_owned(),
            after_room_id: Some("room-a".to_owned()),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert_eq!(
        page.rooms,
        vec![serde_json::to_value(&second_record).expect("second expected json")]
    );
    assert_eq!(page.next_after_room_id.as_deref(), Some("room-b"));
    assert!(!page.has_more);
}

#[tokio::test]
async fn sqlite_account_room_bootstrap_survives_restart_and_conflicts() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let bootstrap = BootstrapAccountRoomRequest {
        room_id: "room-bootstrap".to_owned(),
        mls_group_id: "mls-bootstrap".to_owned(),
        creator: DeviceRef {
            account_id: "alice".to_owned(),
            device_id: "alice-laptop".to_owned(),
        },
        protocol: RoomProtocol::default(),
    };

    let app = persistent_app(&db_path);
    let response = post_json(app, "/account-rooms/bootstrap", &bootstrap).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bootstrapped: BootstrapAccountRoomResponse = read_json(response).await;
    assert!(bootstrapped.bootstrapped);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: "alice".to_owned(),
            after_room_id: None,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert_eq!(page.rooms.len(), 1);
    assert_eq!(page.rooms[0]["room_id"], "room-bootstrap");
    assert_eq!(page.rooms[0]["mls_group_id"], "mls-bootstrap");
    assert_eq!(page.rooms[0]["current_epoch"], 0);
    assert_eq!(page.rooms[0]["last_seq"], 0);
    assert_eq!(page.rooms[0]["devices"][0]["device"]["account_id"], "alice");
    assert_eq!(
        page.rooms[0]["devices"][0]["device"]["device_id"],
        "alice-laptop"
    );
    assert_eq!(page.rooms[0]["devices"][0]["active"], true);

    let response = post_json(app.clone(), "/account-rooms/bootstrap", &bootstrap).await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed: BootstrapAccountRoomResponse = read_json(response).await;
    assert!(!replayed.bootstrapped);

    let response = post_json(
        app,
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            creator: DeviceRef {
                account_id: "alice".to_owned(),
                device_id: "alice-phone".to_owned(),
            },
            ..bootstrap
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "account_room_bootstrap_conflict");
}

#[tokio::test]
async fn sqlite_submit_commit_route_publishes_room_entry_and_derives_membership_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let creator = DeviceRef::new("alice", "alice-laptop");
    let phone = DeviceRef::new("alice", "alice-phone");
    let room_id = "room-submit-commit-route".to_owned();
    let mls_group_id = "mls-submit-commit-route".to_owned();
    let welcome_id = "welcome-submit-commit-route".to_owned();
    let request = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &creator,
        &phone,
        &welcome_id,
        "commit-route-idempotency",
    );
    let expected_message_id = request
        .envelope
        .message_id()
        .expect("commit envelope message id");

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: creator.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    publish_and_claim_key_package_for_add(&app, &request).await;
    let response = post_json(app, "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);
    assert_eq!(accepted.message_id, expected_message_id);
    assert_eq!(accepted.released_welcomes, vec![welcome_id.clone()]);

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed: CommitAccepted = read_json(response).await;
    assert_eq!(replayed, accepted);

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let group_page: HttpSyncPage = read_json(response).await;
    assert_eq!(group_page.entries.len(), 1);
    assert_eq!(group_page.entries[0].seq, accepted.seq);
    assert_eq!(group_page.entries[0].message.id, id(&accepted.message_id));
    let projection: FiniteAccountRoomCommitProjection =
        serde_json::from_slice(&group_page.entries[0].message.payload)
            .expect("commit projection payload");
    assert_eq!(projection.entry.message_id, accepted.message_id);
    assert_eq!(projection.entry.room_id, room_id);
    assert_eq!(projection.entry.kind, LogEntryKind::Commit);
    assert_eq!(projection.membership_delta, request.membership_delta);

    let recipient = member_for_device(&DeviceRef::new("alice", "alice-phone"));
    let response = post_json(
        app.clone(),
        "/sync/inbox",
        &InboxSyncRequest {
            recipient: recipient.clone(),
            after_seq: 0,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let inbox_page: HttpSyncPage = read_json(response).await;
    assert_eq!(inbox_page.entries.len(), 1);
    assert_eq!(inbox_page.entries[0].seq, 1);
    assert_eq!(inbox_page.entries[0].message.id, id(&welcome_id));
    let welcome: WelcomeRecord =
        serde_json::from_slice(&inbox_page.entries[0].message.payload).expect("welcome payload");
    assert_eq!(welcome.welcome_id, welcome_id);
    assert_eq!(welcome.commit_seq, accepted.seq);
    assert_eq!(welcome.recipient, DeviceRef::new("alice", "alice-phone"));
    assert_eq!(welcome.state, WelcomeState::Released);

    let response = post_json(
        app.clone(),
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: "alice".to_owned(),
            after_room_id: None,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert_eq!(page.rooms.len(), 1);
    assert_eq!(page.rooms[0]["current_epoch"], 1);
    assert_eq!(page.rooms[0]["last_seq"], accepted.seq);
    assert_eq!(page.rooms[0]["devices"][0]["active"], true);
    assert_eq!(
        page.rooms[0]["devices"][1]["device"]["device_id"],
        "alice-phone"
    );
    assert_eq!(page.rooms[0]["devices"][1]["active"], false);
}

#[tokio::test]
async fn sqlite_submit_commit_routes_welcome_to_electron_length_device_id() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let creator = DeviceRef::new(account_id, "ios-device");
    let electron = DeviceRef::new(account_id, "electron-Pauls-MacBook-Pro-2.local");
    let room_id = "room-electron-device-route".to_owned();
    let mls_group_id = "mls-electron-device-route".to_owned();
    let welcome_id = "welcome-electron-device-route".to_owned();
    let request = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &creator,
        &electron,
        &welcome_id,
        "electron-device-route",
    );

    assert_eq!(
        serde_json::to_vec(&electron).expect("device json").len(),
        130
    );
    assert!(member_for_device(&electron).as_slice().len() <= MAX_HTTP_ID_BYTES);

    let app = persistent_app(&db_path);
    bootstrap_room(&app, &room_id, &mls_group_id, &creator).await;
    publish_and_claim_key_package_for_add(&app, &request).await;

    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.released_welcomes, vec![welcome_id.clone()]);

    let response = post_json(
        app,
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: member_for_device(&electron),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claims: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].message.id, id(&welcome_id));
    let welcome: WelcomeRecord =
        serde_json::from_slice(&claims[0].message.payload).expect("welcome payload");
    assert_eq!(welcome.recipient, electron);
}

#[tokio::test]
async fn sqlite_submit_commit_validates_and_consumes_claimed_key_package_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let creator = DeviceRef::new("alice", "alice-laptop");
    let phone = DeviceRef::new("alice", "alice-phone");
    let tablet = DeviceRef::new("alice", "alice-tablet");
    let room_id = "room-submit-key-package-lifecycle".to_owned();
    let mls_group_id = "mls-submit-key-package-lifecycle".to_owned();
    let request = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &creator,
        &phone,
        "welcome-key-package-lifecycle-phone",
        "key-package-lifecycle-phone",
    );

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: creator.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_commit_request");
    assert!(
        error
            .error
            .contains("must be claimed before a typed commit"),
        "unexpected error: {}",
        error.error
    );
    assert_submit_commit_had_no_side_effects(&app, &room_id, &phone).await;

    publish_and_claim_key_package_for_add(&app, &request).await;
    let inventory = key_package_inventory_for_device(&app, &phone).await;
    assert_eq!(inventory.available, 0);
    assert_eq!(inventory.claimed, 1);

    let mut stale_ref = request.clone();
    stale_ref.membership_delta.adds[0].key_package_ref = "stale-ref".to_owned();
    let response = post_json(app.clone(), "/commits", &stale_ref).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_commit_request");
    assert!(error.error.contains("metadata does not match"));
    assert_submit_commit_had_no_side_effects(&app, &room_id, &phone).await;
    let inventory = key_package_inventory_for_device(&app, &phone).await;
    assert_eq!(inventory.available, 0);
    assert_eq!(inventory.claimed, 1);

    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);
    assert_eq!(
        accepted.released_welcomes,
        vec!["welcome-key-package-lifecycle-phone".to_owned()]
    );
    let inventory = key_package_inventory_for_device(&app, &phone).await;
    assert_eq!(inventory.available, 0);
    assert_eq!(inventory.claimed, 0);

    let app = persistent_app(&db_path);
    let inventory = key_package_inventory_for_device(&app, &phone).await;
    assert_eq!(inventory.available, 0);
    assert_eq!(inventory.claimed, 0);
    let response = post_json(
        app.clone(),
        "/key-packages/leases/expire",
        &ExpireKeyPackageLeaseRequest {
            key_package_id: HttpKeyPackageId::new(
                request.membership_delta.adds[0]
                    .key_package_id
                    .as_bytes()
                    .to_vec(),
            ),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_key_package_lease_request");
    assert!(error.error.contains("already consumed"));

    let mut reuse = submit_add_device_request_at_epoch_with_ids(
        &room_id,
        &mls_group_id,
        &creator,
        &tablet,
        1,
        "welcome-key-package-lifecycle-reuse",
        "key-package-lifecycle-reuse",
    );
    reuse.membership_delta.adds[0].key_package_id =
        request.membership_delta.adds[0].key_package_id.clone();
    reuse.membership_delta.adds[0].key_package_ref =
        request.membership_delta.adds[0].key_package_ref.clone();
    reuse.membership_delta.adds[0].key_package_hash =
        request.membership_delta.adds[0].key_package_hash.clone();
    let response = post_json(app.clone(), "/commits", &reuse).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_commit_request");
    assert!(error.error.contains("already consumed"));

    let response = post_json(
        app,
        "/sync/inbox",
        &InboxSyncRequest {
            recipient: member_for_device(&tablet),
            after_seq: 0,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
}

#[tokio::test]
async fn sqlite_submit_commit_rejects_account_device_cap_before_side_effects() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let creator = DeviceRef::new("alice", "alice-laptop");
    let room_id = "room-account-device-cap".to_owned();
    let mls_group_id = "mls-account-device-cap".to_owned();
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: creator.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    for index in 0..(MAX_ACCOUNT_DEVICES_PER_ROOM - 1) {
        let device = DeviceRef::new("alice", format!("alice-extra-{index}"));
        let request = submit_add_device_request_at_epoch_with_ids(
            &room_id,
            &mls_group_id,
            &creator,
            &device,
            u64::from(index),
            &format!("welcome-account-cap-{index}"),
            &format!("commit-account-cap-{index}"),
        );
        publish_and_claim_key_package_for_add(&app, &request).await;
        let response = post_json(app.clone(), "/commits", &request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: CommitAccepted = read_json(response).await;
        assert_eq!(accepted.seq, u64::from(index) + 1);
    }

    let overflow = DeviceRef::new("alice", "alice-extra-overflow");
    let overflow_request = submit_add_device_request_at_epoch_with_ids(
        &room_id,
        &mls_group_id,
        &creator,
        &overflow,
        u64::from(MAX_ACCOUNT_DEVICES_PER_ROOM - 1),
        "welcome-account-cap-overflow",
        "commit-account-cap-overflow",
    );
    publish_and_claim_key_package_for_add(&app, &overflow_request).await;
    let response = post_json(app.clone(), "/commits", &overflow_request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_commit_request");
    assert!(error.error.contains("room.devices_per_account"));

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: u64::from(MAX_ACCOUNT_DEVICES_PER_ROOM - 1),
            limit: 10,
            requester: Some(member_for_device(&creator)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
    assert_eq!(
        page.next_after_seq,
        u64::from(MAX_ACCOUNT_DEVICES_PER_ROOM - 1)
    );

    let page = account_room_page(&app, "alice").await;
    assert_eq!(page.rooms.len(), 1);
    assert_eq!(
        page.rooms[0]["devices"].as_array().expect("devices").len(),
        MAX_ACCOUNT_DEVICES_PER_ROOM as usize
    );
    assert!(
        !page.rooms[0]["devices"]
            .as_array()
            .expect("devices")
            .iter()
            .any(|device| device["device"]["device_id"] == "alice-extra-overflow")
    );

    let response = post_json(
        app.clone(),
        "/sync/inbox",
        &InboxSyncRequest {
            recipient: member_for_device(&overflow),
            after_seq: 0,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());

    let inventory = key_package_inventory_for_device(&app, &overflow).await;
    assert_eq!(inventory.available, 0);
    assert_eq!(inventory.claimed, 1);
}

#[tokio::test]
async fn sqlite_submit_commit_rejects_duplicate_pending_device_before_side_effects() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let creator = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let room_id = "room-duplicate-pending-add".to_owned();
    let mls_group_id = "mls-duplicate-pending-add".to_owned();
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: creator.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let add_bob = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &creator,
        &bob,
        "welcome-duplicate-pending-bob",
        "commit-duplicate-pending-bob",
    );
    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);

    let duplicate = submit_add_device_request_at_epoch_with_ids(
        &room_id,
        &mls_group_id,
        &creator,
        &bob,
        1,
        "welcome-duplicate-pending-bob-retry",
        "commit-duplicate-pending-bob-retry",
    );
    publish_and_claim_key_package_for_add(&app, &duplicate).await;
    let response = post_json(app.clone(), "/commits", &duplicate).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_commit_request");
    assert!(error.error.contains("already current or pending"));

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: accepted.seq,
            limit: 10,
            requester: Some(member_for_device(&creator)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
    assert_eq!(page.next_after_seq, accepted.seq);

    let response = post_json(
        app.clone(),
        "/sync/inbox",
        &InboxSyncRequest {
            recipient: member_for_device(&bob),
            after_seq: 0,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(
        page.entries[0].message.id,
        id("welcome-duplicate-pending-bob")
    );

    let account_page = account_room_page(&app, "bob").await;
    assert_eq!(account_page.rooms.len(), 1);
    let devices = account_page.rooms[0]["devices"]
        .as_array()
        .expect("devices");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["device"]["device_id"], "bob-phone");
    assert_eq!(devices[0]["active"], false);

    let inventory = key_package_inventory_for_device(&app, &bob).await;
    assert_eq!(inventory.available, 0);
    assert_eq!(inventory.claimed, 1);
}

#[tokio::test]
async fn sqlite_welcome_not_released_before_accepted_commit_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let creator = DeviceRef::new("alice", "alice-laptop");
    let phone = DeviceRef::new("alice", "alice-phone");
    let room_id = "room-welcome-release-coupling".to_owned();
    let mls_group_id = "mls-welcome-release-coupling".to_owned();
    let welcome_id = "welcome-release-coupling-phone".to_owned();
    let request = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &creator,
        &phone,
        &welcome_id,
        "welcome-release-coupling",
    );
    let mut rejected = request.clone();
    rejected.membership_delta.adds[0].key_package_hash = "wrong-hash".to_owned();

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: creator.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    publish_and_claim_key_package_for_add(&app, &request).await;

    let response = post_json(app.clone(), "/commits", &rejected).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_commit_request");
    assert!(error.error.contains("metadata does not match"));
    assert_submit_commit_had_no_side_effects(&app, &room_id, &phone).await;

    let app = persistent_app(&db_path);
    assert_submit_commit_had_no_side_effects(&app, &room_id, &phone).await;

    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);
    assert_eq!(accepted.released_welcomes, vec![welcome_id.clone()]);

    let response = post_json(
        app,
        "/sync/inbox",
        &InboxSyncRequest {
            recipient: member_for_device(&phone),
            after_seq: 0,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].message.id, id(&welcome_id));
    let welcome: WelcomeRecord =
        serde_json::from_slice(&page.entries[0].message.payload).expect("welcome payload");
    assert_eq!(welcome.state, WelcomeState::Released);
}

#[tokio::test]
async fn sqlite_submit_commit_replay_repairs_projection_after_partial_durable_publish() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let creator = DeviceRef::new("alice", "alice-laptop");
    let phone = DeviceRef::new("alice", "alice-phone");
    let room_id = "room-submit-partial-replay".to_owned();
    let mls_group_id = "mls-submit-partial-replay".to_owned();
    let welcome_id = "welcome-submit-partial-replay".to_owned();
    let request = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &creator,
        &phone,
        &welcome_id,
        "partial-replay-idempotency",
    );
    let message_id = request
        .envelope
        .message_id()
        .expect("commit envelope message id");
    let commit_publish = commit_publish_request_for_test(&request, &message_id);

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id,
            creator,
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // Model a process interruption after the commit publish/idempotency rows are
    // durable but before the finite projection writes run.
    insert_durable_commit_publish_without_projection(&db_path, &commit_publish, 1);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: "alice".to_owned(),
            after_room_id: None,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let before_retry: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert_eq!(before_retry.rooms.len(), 1);
    assert_eq!(before_retry.rooms[0]["current_epoch"], 0);

    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);
    assert_eq!(accepted.message_id, message_id);
    assert_eq!(accepted.released_welcomes, vec![welcome_id.clone()]);

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed: CommitAccepted = read_json(response).await;
    assert_eq!(replayed, accepted);

    let response = post_json(
        app.clone(),
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: "alice".to_owned(),
            after_room_id: None,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let after_retry: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert_eq!(after_retry.rooms.len(), 1);
    assert_eq!(after_retry.rooms[0]["current_epoch"], 1);
    assert_eq!(after_retry.rooms[0]["last_seq"], accepted.seq);
    assert_eq!(
        after_retry.rooms[0]["devices"][1]["device"]["device_id"],
        "alice-phone"
    );
    assert_eq!(after_retry.rooms[0]["devices"][1]["active"], false);

    let response = post_json(
        app,
        "/sync/inbox",
        &InboxSyncRequest {
            recipient: member_for_device(&DeviceRef::new("alice", "alice-phone")),
            after_seq: 0,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let inbox_page: HttpSyncPage = read_json(response).await;
    assert_eq!(inbox_page.entries.len(), 1);
    assert_eq!(inbox_page.entries[0].message.id, id(&welcome_id));
}

#[tokio::test]
async fn sqlite_rejected_submit_commit_replays_rejection_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let creator = DeviceRef::new("alice", "alice-laptop");
    let phone = DeviceRef::new("alice", "alice-phone");
    let tablet = DeviceRef::new("alice", "alice-tablet");
    let room_id = "room-rejected-submit-replay".to_owned();
    let mls_group_id = "mls-rejected-submit-replay".to_owned();
    let winner = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &creator,
        &phone,
        "welcome-rejected-submit-phone",
        "rejected-submit-winner",
    );
    let loser = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &creator,
        &tablet,
        "welcome-rejected-submit-tablet",
        "rejected-submit-loser",
    );

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: creator.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    publish_and_claim_key_package_for_add(&app, &winner).await;
    let response = post_json(app.clone(), "/commits", &winner).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);

    let response = post_json(app, "/commits", &loser).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let first_error: ErrorResponse = read_json(response).await;
    assert_eq!(first_error.kind, "invalid_commit_request");
    assert!(
        first_error
            .error
            .contains("commit expected epoch 0 does not match room epoch 1")
    );

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/commits", &loser).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let replayed_error: ErrorResponse = read_json(response).await;
    assert_eq!(replayed_error, first_error);

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].seq, accepted.seq);

    let response = post_json(
        app.clone(),
        "/sync/inbox",
        &InboxSyncRequest {
            recipient: member_for_device(&tablet),
            after_seq: 0,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let inbox_page: HttpSyncPage = read_json(response).await;
    assert!(inbox_page.entries.is_empty());

    let response = post_json(
        app.clone(),
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: "alice".to_owned(),
            after_room_id: None,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert_eq!(page.rooms.len(), 1);
    assert_eq!(page.rooms[0]["current_epoch"], 1);
    assert_eq!(page.rooms[0]["last_seq"], accepted.seq);
    assert_eq!(
        page.rooms[0]["devices"].as_array().expect("devices").len(),
        2
    );
    assert!(
        !page.rooms[0]["devices"]
            .as_array()
            .expect("devices")
            .iter()
            .any(|device| device["device"]["device_id"] == "alice-tablet")
    );
}

#[tokio::test]
async fn sqlite_submit_commit_crash_matrix_rolls_back_and_retry_converges() {
    for crash_point in HttpSubmitCommitCrashPoint::ALL {
        let temp = TempDir::new().expect("tempdir");
        let db_path = temp.path().join("delivery.sqlite3");
        let creator = DeviceRef::new("alice", "alice-laptop");
        let phone = DeviceRef::new("alice", "alice-phone");
        let tablet = DeviceRef::new("alice", "alice-tablet");
        let room_id = "room-http-crash-matrix".to_owned();
        let mls_group_id = "mls-http-crash-matrix".to_owned();
        let first = submit_add_device_request(
            &room_id,
            &mls_group_id,
            &creator,
            &phone,
            "welcome-http-crash-phone",
            "http-crash-first",
        );
        let crash_request = submit_add_device_request_at_epoch_with_ids(
            &room_id,
            &mls_group_id,
            &creator,
            &tablet,
            1,
            "welcome-http-crash-tablet",
            "http-crash-matrix-commit",
        );

        let app = persistent_app(&db_path);
        let response = post_json(
            app.clone(),
            "/account-rooms/bootstrap",
            &BootstrapAccountRoomRequest {
                room_id: room_id.clone(),
                mls_group_id: mls_group_id.clone(),
                creator: creator.clone(),
                protocol: RoomProtocol::default(),
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        publish_and_claim_key_package_for_add(&app, &first).await;
        let response = post_json(app.clone(), "/commits", &first).await;
        assert_eq!(response.status(), StatusCode::OK);
        let first_accepted: CommitAccepted = read_json(response).await;
        assert_eq!(first_accepted.seq, 1);

        publish_and_claim_key_package_for_add(&app, &crash_request).await;
        install_http_submit_commit_crash_trigger(&db_path, crash_point);
        let response = post_json(app, "/commits", &crash_request).await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "expected SQLite crash response at {crash_point:?}"
        );
        let error: ErrorResponse = read_json(response).await;
        assert_eq!(error.kind, "delivery_store");
        clear_http_submit_commit_crash_triggers(&db_path);

        let app = persistent_app(&db_path);
        assert_http_crash_commit_rolled_back(&app, &room_id, &tablet, first_accepted.seq).await;

        let response = post_json(app.clone(), "/commits", &crash_request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: CommitAccepted = read_json(response).await;
        assert_eq!(accepted.seq, 2);
        assert_eq!(
            accepted.released_welcomes,
            vec!["welcome-http-crash-tablet".to_owned()]
        );

        let response = post_json(app.clone(), "/commits", &crash_request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let replayed: CommitAccepted = read_json(response).await;
        assert_eq!(replayed, accepted);

        assert_http_crash_commit_converged(&app, &room_id, &tablet, accepted.seq).await;
    }
}

#[tokio::test]
async fn submit_commit_route_rejects_missing_staged_welcome_before_side_effects() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-submit-missing-welcome".to_owned();
    let mut request = submit_add_device_request(
        &room_id,
        "mls-submit-missing-welcome",
        &DeviceRef::new("alice", "alice-laptop"),
        &DeviceRef::new("alice", "alice-phone"),
        "welcome-submit-missing-welcome",
        "missing-welcome-idempotency",
    );
    request.staged_welcomes.clear();

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_commit_request");

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
}

#[tokio::test]
async fn sqlite_submit_commit_rejects_membership_delta_structural_matrix_before_side_effects() {
    struct Case {
        label: &'static str,
        mutate: fn(&mut SubmitCommitRequest),
        expected_error: &'static str,
    }

    let cases = [
        Case {
            label: "wrong-base-epoch",
            mutate: |request| request.membership_delta.base_epoch = 9,
            expected_error: "base epoch",
        },
        Case {
            label: "wrong-post-commit-epoch",
            mutate: |request| request.membership_delta.post_commit_epoch = 3,
            expected_error: "post-commit epoch",
        },
        Case {
            label: "wrong-commit-message-id",
            mutate: |request| request.membership_delta.commit_message_id = "wrong".to_owned(),
            expected_error: "commit message id",
        },
        Case {
            label: "duplicate-add",
            mutate: |request| {
                let add = request
                    .membership_delta
                    .adds
                    .first()
                    .expect("base request has add")
                    .clone();
                request.membership_delta.adds.push(add);
            },
            expected_error: "adds device more than once",
        },
        Case {
            label: "duplicate-remove",
            mutate: |request| {
                request.membership_delta.adds.clear();
                let remove = MembershipRemoveV1 {
                    device: DeviceRef::new("bob", "bob-phone"),
                    removed_leaf_index: 1,
                };
                request.membership_delta.removes = vec![remove.clone(), remove];
            },
            expected_error: "removes device more than once",
        },
        Case {
            label: "add-and-remove-same-device",
            mutate: |request| {
                request.membership_delta.removes = vec![MembershipRemoveV1 {
                    device: request.membership_delta.adds[0].device.clone(),
                    removed_leaf_index: 1,
                }];
            },
            expected_error: "adds and removes same device",
        },
        Case {
            label: "incomplete-add",
            mutate: |request| {
                request.membership_delta.adds[0].key_package_id.clear();
            },
            expected_error: "missing key package or welcome fields",
        },
    ];

    let temp = TempDir::new().expect("tempdir");
    for case in cases {
        let db_path = temp.path().join(format!("{}.sqlite3", case.label));
        let room_id = format!("room-structural-{}", case.label);
        let mls_group_id = format!("mls-structural-{}", case.label);
        let creator = DeviceRef::new("alice", "alice-laptop");
        let bob = DeviceRef::new("bob", "bob-phone");
        let app = persistent_app(&db_path);
        let mut request = submit_add_device_request(
            &room_id,
            &mls_group_id,
            &creator,
            &bob,
            &format!("welcome-structural-{}", case.label),
            &format!("commit-structural-{}", case.label),
        );
        publish_and_claim_key_package_for_add(&app, &request).await;
        (case.mutate)(&mut request);

        let response = post_json(app.clone(), "/commits", &request).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{}", case.label);
        let error: ErrorResponse = read_json(response).await;
        assert_eq!(error.kind, "invalid_commit_request", "{}", case.label);
        assert!(
            error.error.contains(case.expected_error),
            "case {} returned unexpected error: {}",
            case.label,
            error.error
        );

        let app = persistent_app(&db_path);
        assert_submit_commit_had_no_side_effects(&app, &room_id, &bob).await;

        let account_page = account_room_page(&app, "bob").await;
        assert!(account_page.rooms.is_empty(), "{}", case.label);

        let inventory = key_package_inventory_for_device(&app, &bob).await;
        assert_eq!(inventory.available, 0, "{}", case.label);
        assert_eq!(inventory.claimed, 1, "{}", case.label);
    }
}

#[tokio::test]
async fn sqlite_group_sync_filters_by_persisted_room_membership_projection() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-filtered-membership-sync".to_owned();
    let mls_group_id = "mls-filtered-membership-sync".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let carol = DeviceRef::new("carol", "carol-phone");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            b"hidden",
            "app-before-bob-idempotency",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let hidden_acceptance: EventAccepted = read_json(response).await;
    assert_eq!(hidden_acceptance.seq, 1);

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: Some(member_for_device(&bob)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bob_hidden_page: HttpSyncPage = read_json(response).await;
    assert!(bob_hidden_page.entries.is_empty());
    assert_eq!(bob_hidden_page.next_after_seq, hidden_acceptance.seq);

    let request = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-filtered-bob",
        "commit-filtered-bob",
    );
    let commit_message_id = request.envelope.message_id().expect("commit message id");
    publish_and_claim_key_package_for_add(&app, &request).await;
    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 2);

    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &bob,
            1,
            b"pending-send",
            "bob-pending-send-idempotency",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "sender_not_active");

    let mut pending_commit = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &bob,
        &carol,
        "welcome-filtered-carol",
        "bob-pending-commit-idempotency",
    );
    pending_commit.expected_epoch = 1;
    pending_commit.envelope.epoch = 1;
    let pending_commit_message_id = pending_commit
        .envelope
        .message_id()
        .expect("pending commit message id");
    pending_commit.membership_delta.base_epoch = 1;
    pending_commit.membership_delta.post_commit_epoch = 2;
    pending_commit.membership_delta.commit_message_id = pending_commit_message_id;
    let response = post_json(app.clone(), "/commits", &pending_commit).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "sender_not_active");

    let response = post_json(
        app,
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            1,
            b"visible",
            "app-after-bob-idempotency",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let visible_acceptance: EventAccepted = read_json(response).await;
    assert_eq!(visible_acceptance.seq, 3);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: bob_hidden_page.next_after_seq,
            limit: 10,
            requester: Some(member_for_device(&bob)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bob_visible_page: HttpSyncPage = read_json(response).await;
    assert_eq!(bob_visible_page.entries.len(), 2);
    assert_eq!(
        bob_visible_page.entries[0].message.id.as_slice(),
        commit_message_id.as_bytes()
    );
    assert_eq!(
        bob_visible_page.entries[1].message.id.as_slice(),
        visible_acceptance.message_id.as_bytes()
    );
    assert_eq!(bob_visible_page.next_after_seq, visible_acceptance.seq);

    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: member_for_device(&bob),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claimed.len(), 1);

    let response = post_json(
        app,
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-filtered-bob"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &bob,
            1,
            b"activated-send",
            "bob-activated-send-idempotency",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bob_acceptance: EventAccepted = read_json(response).await;
    assert_eq!(bob_acceptance.seq, 4);

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: Some(member_for_device(&carol)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let carol_page: HttpSyncPage = read_json(response).await;
    assert!(carol_page.entries.is_empty());
    assert_eq!(carol_page.next_after_seq, bob_acceptance.seq);
}

#[tokio::test]
async fn sqlite_multi_device_pending_welcome_roles_stay_separate_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-multi-device-pending-welcome".to_owned();
    let mls_group_id = "mls-multi-device-pending-welcome".to_owned();
    let bob = DeviceRef::new("bob", "bob-runtime");
    let alice_devices = [
        DeviceRef::new("alice", "alice-browser"),
        DeviceRef::new("alice", "alice-phone"),
        DeviceRef::new("alice", "alice-tablet"),
    ];
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: bob.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    for device in &alice_devices {
        let key_package_id = format!("kp-multi-{}", device.device_id);
        let response = post_json(
            app.clone(),
            "/key-packages",
            &finite_key_package_publication(
                device,
                &key_package_id,
                &format!("ref-{key_package_id}"),
                &format!("hash-{key_package_id}"),
                format!("payload-{key_package_id}").as_bytes(),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    let owners = alice_devices
        .iter()
        .map(member_for_device)
        .collect::<Vec<_>>();
    let response = post_json(
        app.clone(),
        "/key-packages/claims",
        &ClaimKeyPackagesRequest {
            owners: owners.clone(),
            idempotency_key: Some("multi-device-pending-welcome-claim".to_owned()),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Vec<HttpKeyPackageClaim> = read_json(response).await;
    assert_eq!(claimed.len(), alice_devices.len());
    for (claim, owner) in claimed.iter().zip(&owners) {
        assert_eq!(&claim.owner, owner);
        assert!(claim.claimed.is_some());
    }

    let envelope = FiniteEnvelope {
        room_id: room_id.clone(),
        mls_group_id: mls_group_id.clone(),
        epoch: 0,
        sender: bob.clone(),
        kind: LogEntryKind::Commit,
        payload: b"multi-device-pending-welcome".to_vec(),
    };
    let commit_message_id = envelope.message_id().expect("commit message id");
    let request = SubmitCommitRequest {
        room_id: room_id.clone(),
        sender: bob.clone(),
        expected_epoch: 0,
        envelope,
        membership_delta: MembershipDeltaV1 {
            base_epoch: 0,
            post_commit_epoch: 1,
            commit_message_id,
            adds: alice_devices
                .iter()
                .map(|device| {
                    let key_package_id = format!("kp-multi-{}", device.device_id);
                    MembershipAddV1 {
                        device: device.clone(),
                        key_package_id: key_package_id.clone(),
                        key_package_ref: format!("ref-{key_package_id}"),
                        key_package_hash: format!("hash-{key_package_id}"),
                        welcome_id: format!("welcome-multi-{}", device.device_id),
                    }
                })
                .collect(),
            removes: Vec::new(),
        },
        staged_welcomes: alice_devices
            .iter()
            .map(|device| StagedWelcomeV1 {
                welcome_id: format!("welcome-multi-{}", device.device_id),
                welcome_payload: format!("welcome-{}", device.device_id).into_bytes(),
                ratchet_tree_payload: format!("ratchet-{}", device.device_id).into_bytes(),
            })
            .collect(),
        idempotency_key: "multi-device-pending-welcome-commit".to_owned(),
    };
    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);

    let app = persistent_app(&db_path);
    let account_page = account_room_page(&app, &alice_devices[0].account_id).await;
    assert_eq!(account_page.rooms.len(), 1);
    for device in &alice_devices {
        assert!(!account_room_device_active(&account_page, device));
    }

    for device in &alice_devices {
        let response = post_json(
            app.clone(),
            "/welcomes/claim",
            &ClaimWelcomesRequest {
                recipient: member_for_device(device),
                limit: 10,
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
        assert_eq!(claimed.len(), 1);
        assert_eq!(
            claimed[0].message.id,
            id(&format!("welcome-multi-{}", device.device_id))
        );
    }

    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-multi-alice-phone"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let app = persistent_app(&db_path);
    let account_page = account_room_page(&app, &alice_devices[0].account_id).await;
    assert!(!account_room_device_active(
        &account_page,
        &alice_devices[0]
    ));
    assert!(account_room_device_active(&account_page, &alice_devices[1]));
    assert!(!account_room_device_active(
        &account_page,
        &alice_devices[2]
    ));

    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &alice_devices[1],
            1,
            b"phone active",
            "multi-device-phone-active",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let phone_accepted: EventAccepted = read_json(response).await;

    for (device, idempotency_key) in [
        (&alice_devices[0], "multi-device-browser-pending"),
        (&alice_devices[2], "multi-device-tablet-pending"),
    ] {
        let response = post_json(
            app.clone(),
            "/events",
            &typed_event_request(&append_application_request(
                &room_id,
                &mls_group_id,
                device,
                1,
                b"still pending",
                idempotency_key,
            )),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let error: ErrorResponse = read_json(response).await;
        assert_eq!(error.kind, "sender_not_active");
    }

    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &bob,
            1,
            b"bob after welcome",
            "multi-device-bob-after-welcome",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bob_accepted: EventAccepted = read_json(response).await;

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: accepted.seq,
            limit: 10,
            requester: Some(member_for_device(&alice_devices[2])),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tablet_page: HttpSyncPage = read_json(response).await;
    assert_eq!(tablet_page.entries.len(), 2);
    assert_eq!(
        tablet_page.entries[0].message.id.as_slice(),
        phone_accepted.message_id.as_bytes()
    );
    assert_eq!(
        tablet_page.entries[1].message.id.as_slice(),
        bob_accepted.message_id.as_bytes()
    );
    assert_eq!(tablet_page.next_after_seq, bob_accepted.seq);

    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-multi-alice-browser"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &alice_devices[0],
            1,
            b"browser active",
            "multi-device-browser-active",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let app = persistent_app(&db_path);
    let account_page = account_room_page(&app, &alice_devices[0].account_id).await;
    assert!(account_room_device_active(&account_page, &alice_devices[0]));
    assert!(account_room_device_active(&account_page, &alice_devices[1]));
    assert!(!account_room_device_active(
        &account_page,
        &alice_devices[2]
    ));

    let response = post_json(
        app,
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &alice_devices[2],
            1,
            b"tablet still pending",
            "multi-device-tablet-still-pending",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "sender_not_active");
}

#[tokio::test]
async fn sqlite_removed_device_syncs_through_removal_and_cannot_send_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-removed-device-sync".to_owned();
    let mls_group_id = "mls-removed-device-sync".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let add_bob = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-removed-sync-bob",
        "add-removed-sync-bob",
    );
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    let add_acceptance: CommitAccepted = read_json(response).await;
    assert_eq!(add_acceptance.seq, 1);

    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: member_for_device(&bob),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claimed.len(), 1);
    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-removed-sync-bob"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let remove_bob =
        submit_remove_device_request(&room_id, &mls_group_id, &alice, &bob, 1, "remove-sync-bob");
    let remove_message_id = remove_bob.envelope.message_id().expect("remove message id");
    let response = post_json(app.clone(), "/commits", &remove_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    let removal: CommitAccepted = read_json(response).await;
    assert_eq!(removal.seq, 2);
    assert_eq!(removal.message_id, remove_message_id);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: add_acceptance.seq,
            limit: 10,
            requester: Some(member_for_device(&bob)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bob_page: HttpSyncPage = read_json(response).await;
    assert_eq!(bob_page.entries.len(), 1);
    assert_eq!(bob_page.entries[0].seq, removal.seq);
    assert_eq!(
        bob_page.entries[0].message.id.as_slice(),
        remove_message_id.as_bytes()
    );
    assert_eq!(bob_page.next_after_seq, removal.seq);
    assert!(!bob_page.has_more);

    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            2,
            b"after removal",
            "alice-after-remove-idempotency",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let after_removal: EventAccepted = read_json(response).await;
    assert_eq!(after_removal.seq, 3);

    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &bob,
            2,
            b"stale send",
            "bob-stale-send-idempotency",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "sender_not_active");

    let stale_commit =
        submit_remove_device_request(&room_id, &mls_group_id, &bob, &alice, 2, "bob-stale-commit");
    let response = post_json(app.clone(), "/commits", &stale_commit).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "sender_not_active");

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: removal.seq,
            limit: 10,
            requester: Some(member_for_device(&bob)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let hidden_after_removal: HttpSyncPage = read_json(response).await;
    assert!(hidden_after_removal.entries.is_empty());
    assert_eq!(hidden_after_removal.next_after_seq, after_removal.seq);
    assert!(!hidden_after_removal.has_more);

    let response = post_json(
        app.clone(),
        "/rooms/report-invalid-commit",
        &ReportInvalidCommitRequest {
            room_id: room_id.clone(),
            reporter: bob.clone(),
            offending_seq: removal.seq,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let reported: ReportInvalidCommitResponse = read_json(response).await;
    assert!(reported.reported);

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            2,
            b"blocked after repair",
            "alice-after-removal-repair-idempotency",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "room_not_open");
}

#[tokio::test]
async fn sqlite_typed_event_rejects_oversized_payload_without_persisting_log() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-event-oversized".to_owned();
    let mls_group_id = "mls-event-oversized".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let oversized = vec![0; MAX_ENVELOPE_PAYLOAD_BYTES as usize + 1];
    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            &oversized,
            "oversized-event-idempotency",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_event_request");

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
    assert_eq!(page.next_after_seq, 0);

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
    assert_eq!(page.next_after_seq, 0);
}

#[tokio::test]
async fn sqlite_typed_event_duplicate_message_id_with_new_idempotency_key_conflicts() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-event-duplicate-message-id".to_owned();
    let mls_group_id = "mls-event-duplicate-message-id".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let first = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        b"same ciphertext",
        "first-event-idempotency",
    );
    let duplicate = AppendEventRequest {
        idempotency_key: "second-event-idempotency".to_owned(),
        ..first.clone()
    };
    let message_id = first.envelope.message_id().expect("event message id");

    let response = post_json(app.clone(), "/events", &typed_event_request(&first)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: EventAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);
    assert_eq!(accepted.message_id, message_id);

    let response = post_json(app.clone(), "/events", &typed_event_request(&first)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed: EventAccepted = read_json(response).await;
    assert_eq!(replayed, accepted);

    let response = post_json(app.clone(), "/events", &typed_event_request(&duplicate)).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "duplicate_message_id");

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/events", &typed_event_request(&first)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed_after_restart: EventAccepted = read_json(response).await;
    assert_eq!(replayed_after_restart, accepted);

    let response = post_json(app.clone(), "/events", &typed_event_request(&duplicate)).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "duplicate_message_id");

    let response = post_json(
        app,
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.next_after_seq, 1);
}

#[tokio::test]
async fn sqlite_application_delivery_effects_survive_restart_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-application-effects".to_owned();
    let mls_group_id = "mls-application-effects".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let chat = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        br#"{"type":"chat.message","text":"hello"}"#,
        "application-effect-chat",
    );
    let command = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        br#"{"type":"runtime.command.request","command":"restart"}"#,
        "application-effect-command",
    );
    let receipt = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        br#"{"type":"chat.receipt","message_id":"m1"}"#,
        "application-effect-receipt",
    );
    let chat_message_id = chat.envelope.message_id().expect("chat message id");
    let command_message_id = command.envelope.message_id().expect("command message id");
    let receipt_message_id = receipt.envelope.message_id().expect("receipt message id");

    let response = post_json(
        app.clone(),
        "/events",
        &AppendApplicationEventRequest {
            event: chat.clone(),
            delivery_policy: DurableAppEventKind::ChatMessage.delivery_policy(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted_chat: EventAccepted = read_json(response).await;
    assert_eq!(accepted_chat.seq, 1);
    assert_eq!(accepted_chat.message_id, chat_message_id);

    let response = post_json(
        app.clone(),
        "/events",
        &AppendApplicationEventRequest {
            event: chat.clone(),
            delivery_policy: DurableAppEventKind::ChatMessage.delivery_policy(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed_chat: EventAccepted = read_json(response).await;
    assert_eq!(replayed_chat, accepted_chat);

    let response = post_json(
        app.clone(),
        "/events",
        &AppendApplicationEventRequest {
            event: command.clone(),
            delivery_policy: DurableAppEventKind::RuntimeCommandRequest.delivery_policy(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted_command: EventAccepted = read_json(response).await;
    assert_eq!(accepted_command.seq, 2);
    assert_eq!(accepted_command.message_id, command_message_id);

    let response = post_json(
        app.clone(),
        "/events",
        &AppendApplicationEventRequest {
            event: receipt.clone(),
            delivery_policy: DurableAppEventKind::ChatReceipt.delivery_policy(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted_receipt: EventAccepted = read_json(response).await;
    assert_eq!(accepted_receipt.seq, 3);
    assert_eq!(accepted_receipt.message_id, receipt_message_id);

    let app = persistent_app(&db_path);
    let counts = application_effect_counts(&app).await;
    assert_eq!(counts.push_outbox, 2);
    assert_eq!(counts.unread, 1);
    assert_eq!(counts.command_inbox, 1);

    let chat_effect = application_effect(&app, &accepted_chat.message_id)
        .await
        .expect("chat effect");
    assert_eq!(chat_effect.seq, 1);
    assert_eq!(chat_effect.sender, alice);
    assert!(chat_effect.delivery_policy.creates_push());
    assert!(chat_effect.delivery_policy.creates_unread());
    assert!(!chat_effect.delivery_policy.creates_command_inbox_work());

    let command_effect = application_effect(&app, &accepted_command.message_id)
        .await
        .expect("command effect");
    assert!(command_effect.delivery_policy.creates_push());
    assert!(!command_effect.delivery_policy.creates_unread());
    assert!(command_effect.delivery_policy.creates_command_inbox_work());

    let receipt_effect = application_effect(&app, &accepted_receipt.message_id)
        .await
        .expect("receipt effect");
    assert!(!receipt_effect.delivery_policy.creates_push());
    assert!(!receipt_effect.delivery_policy.creates_unread());
    assert!(!receipt_effect.delivery_policy.creates_command_inbox_work());

    let response = post_json(
        app.clone(),
        "/events",
        &AppendApplicationEventRequest {
            event: chat,
            delivery_policy: DurableAppEventKind::ChatReceipt.delivery_policy(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "idempotency_conflict");
    assert_eq!(application_effect_counts(&app).await, counts);
}

#[tokio::test]
async fn sqlite_application_delivery_policy_matrix_survives_restart_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-application-policy-matrix".to_owned();
    let mls_group_id = "mls-application-policy-matrix".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let non_notifying_kinds = [
        DurableAppEventKind::ChatEdit,
        DurableAppEventKind::ChatReaction,
        DurableAppEventKind::ChatReceipt,
        DurableAppEventKind::RuntimeStateSnapshot,
        DurableAppEventKind::RuntimeCommandResult,
        DurableAppEventKind::RuntimeCommandCancel,
        DurableAppEventKind::ConversationSegmentStart,
    ];
    let mut accepted_message_ids = Vec::new();
    for (index, kind) in non_notifying_kinds.iter().enumerate() {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            format!(r#"{{"event_index":{index},"kind":"{kind:?}"}}"#).as_bytes(),
            &format!("application-policy-matrix-{index}"),
        );
        let response = post_json(
            app.clone(),
            "/events",
            &AppendApplicationEventRequest {
                event: request,
                delivery_policy: kind.delivery_policy(),
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: EventAccepted = read_json(response).await;
        assert_eq!(accepted.seq, u64::try_from(index).unwrap() + 1);
        accepted_message_ids.push(accepted.message_id);
    }

    let app = persistent_app(&db_path);
    assert_eq!(
        application_effect_counts(&app).await,
        ApplicationEffectCountsResponse {
            push_outbox: 0,
            unread: 0,
            command_inbox: 0,
        }
    );
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), accepted_message_ids.len());
    assert_eq!(
        page.next_after_seq,
        u64::try_from(accepted_message_ids.len()).unwrap()
    );

    for message_id in accepted_message_ids {
        let effect = application_effect(&app, &message_id)
            .await
            .expect("policy effect");
        assert!(!effect.delivery_policy.creates_push());
        assert!(!effect.delivery_policy.creates_unread());
        assert!(!effect.delivery_policy.creates_command_inbox_work());
    }
}

#[tokio::test]
async fn sqlite_runtime_state_snapshot_projects_from_http_log_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-runtime-state-projection".to_owned();
    let mls_group_id = "mls-runtime-state-projection".to_owned();
    let runtime = DeviceRef::new("runtime", "runtime-host");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: runtime.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let snapshot = RuntimeStateSnapshotV1 {
        state_key: "runtime.gateway".to_owned(),
        schema: "finitecomputer.runtime.gateway.status.v1".to_owned(),
        revision: 1,
        observed_at_ms: 1_000,
        expires_at_ms: 2_000,
        status_payload: br#"{"status":"live"}"#.to_vec(),
    };
    snapshot.validate_limits().expect("snapshot limits");
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &runtime,
        0,
        &serde_json::to_vec(&snapshot).expect("snapshot json"),
        "runtime-state-projection-snapshot",
    );
    let response = post_json(
        app.clone(),
        "/events",
        &AppendApplicationEventRequest {
            event: request,
            delivery_policy: DurableAppEventKind::RuntimeStateSnapshot.delivery_policy(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: EventAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);

    let app = persistent_app(&db_path);
    assert_eq!(
        application_effect_counts(&app).await,
        ApplicationEffectCountsResponse {
            push_outbox: 0,
            unread: 0,
            command_inbox: 0,
        }
    );
    let effect = application_effect(&app, &accepted.message_id)
        .await
        .expect("runtime state effect");
    assert!(!effect.delivery_policy.creates_push());
    assert!(!effect.delivery_policy.creates_unread());
    assert!(!effect.delivery_policy.creates_command_inbox_work());

    let response = post_json(
        app,
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: Some(member_for_device(&runtime)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].seq, accepted.seq);
    assert_eq!(
        page.entries[0].message.id.as_slice(),
        accepted.message_id.as_bytes()
    );

    let entry: finitechat_proto::RoomLogEntry =
        serde_json::from_slice(&page.entries[0].message.payload).expect("room log entry");
    assert_eq!(entry.kind, LogEntryKind::Application);
    assert_eq!(entry.sender, runtime);
    let synced_snapshot: RuntimeStateSnapshotV1 =
        serde_json::from_slice(&entry.envelope.payload).expect("runtime snapshot");
    let mut projection = RuntimeStateProjection::default();
    projection
        .apply(RuntimeStateProjectionEntry {
            room_id: entry.room_id,
            source: entry.sender,
            accepted_seq: page.entries[0].seq,
            snapshot: synced_snapshot,
        })
        .expect("projection apply");

    let status: serde_json::Value = projection
        .require_fresh_json(
            &room_id,
            &DeviceRef::new("runtime", "runtime-host"),
            "runtime.gateway",
            "finitecomputer.runtime.gateway.status.v1",
            1_500,
        )
        .expect("fresh runtime status");
    assert_eq!(status["status"], "live");

    let err = projection
        .require_fresh(
            &room_id,
            &DeviceRef::new("runtime", "runtime-host"),
            "runtime.gateway",
            "finitecomputer.runtime.gateway.status.v1",
            2_000,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        RuntimeStateProjectionError::Expired {
            now_ms: 2_000,
            expires_at_ms: 2_000,
            ..
        }
    ));
}

#[tokio::test]
async fn sqlite_runtime_command_policy_and_opaque_request_ids_survive_restart_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-runtime-command-policy".to_owned();
    let mls_group_id = "mls-runtime-command-policy".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let status_refresh_policy = ApplicationDeliveryPolicy {
        push: PushPolicy::Never,
        unread: UnreadPolicy::Never,
        command_inbox: CommandInboxPolicy::Create,
    };
    let status_refresh = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        br#"{"type":"runtime.command.request","command":"finitecomputer.runtime.status.refresh"}"#,
        "runtime-status-refresh",
    );
    let response = post_json(
        app.clone(),
        "/events",
        &AppendApplicationEventRequest {
            event: status_refresh,
            delivery_policy: status_refresh_policy,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let status_refresh: EventAccepted = read_json(response).await;
    assert_eq!(status_refresh.seq, 1);

    let first_command = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        br#"{"type":"runtime.command.request","request_id":"restart_1","body":{"attempt":1}}"#,
        "runtime-command-visible-id-1",
    );
    let duplicate_message_id = first_command
        .envelope
        .message_id()
        .expect("duplicate message id");
    let response = post_json(
        app.clone(),
        "/events",
        &AppendApplicationEventRequest {
            event: first_command.clone(),
            delivery_policy: DurableAppEventKind::RuntimeCommandRequest.delivery_policy(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let first_command_accepted: EventAccepted = read_json(response).await;
    assert_eq!(first_command_accepted.seq, 2);

    let second_command = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        br#"{"type":"runtime.command.request","request_id":"restart_1","body":{"attempt":2}}"#,
        "runtime-command-visible-id-2",
    );
    let response = post_json(
        app.clone(),
        "/events",
        &AppendApplicationEventRequest {
            event: second_command,
            delivery_policy: DurableAppEventKind::RuntimeCommandRequest.delivery_policy(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let second_command_accepted: EventAccepted = read_json(response).await;
    assert_eq!(second_command_accepted.seq, 3);
    assert_ne!(
        first_command_accepted.message_id,
        second_command_accepted.message_id
    );

    let duplicate_command = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        br#"{"type":"runtime.command.request","request_id":"restart_1","body":{"attempt":1}}"#,
        "runtime-command-duplicate-idempotency",
    );
    assert_eq!(
        duplicate_command.envelope.message_id().expect("message id"),
        duplicate_message_id
    );

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/events",
        &AppendApplicationEventRequest {
            event: duplicate_command,
            delivery_policy: DurableAppEventKind::RuntimeCommandRequest.delivery_policy(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "duplicate_message_id");

    assert_eq!(
        application_effect_counts(&app).await,
        ApplicationEffectCountsResponse {
            push_outbox: 2,
            unread: 0,
            command_inbox: 3,
        }
    );
    let status_effect = application_effect(&app, &status_refresh.message_id)
        .await
        .expect("status refresh effect");
    assert!(!status_effect.delivery_policy.creates_push());
    assert!(!status_effect.delivery_policy.creates_unread());
    assert!(status_effect.delivery_policy.creates_command_inbox_work());

    for message_id in [
        first_command_accepted.message_id,
        second_command_accepted.message_id,
    ] {
        let effect = application_effect(&app, &message_id)
            .await
            .expect("runtime command effect");
        assert!(effect.delivery_policy.creates_push());
        assert!(!effect.delivery_policy.creates_unread());
        assert!(effect.delivery_policy.creates_command_inbox_work());
    }

    let response = post_json(
        app,
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 3);
    assert_eq!(page.next_after_seq, 3);
}

#[tokio::test]
async fn sqlite_application_delivery_effect_crash_matrix_rolls_back_and_retry_converges() {
    let temp = TempDir::new().expect("tempdir");
    for point in HttpApplicationEventCrashPoint::ALL {
        let db_path = temp
            .path()
            .join(format!("application-event-{point:?}.sqlite3"));
        let room_id = "room-application-effect-crash".to_owned();
        let mls_group_id = "mls-application-effect-crash".to_owned();
        let alice = DeviceRef::new("alice", "alice-laptop");
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            br#"{"type":"chat.message","text":"application-effect-crash"}"#,
            "application-effect-crash",
        );
        let message_id = request.envelope.message_id().expect("message id");
        let policy = DurableAppEventKind::ChatMessage.delivery_policy();

        let app = persistent_app(&db_path);
        let response = post_json(
            app,
            "/account-rooms/bootstrap",
            &BootstrapAccountRoomRequest {
                room_id: room_id.clone(),
                mls_group_id: mls_group_id.clone(),
                creator: alice.clone(),
                protocol: RoomProtocol::default(),
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        install_http_application_event_crash_trigger(&db_path, point);
        let app = persistent_app(&db_path);
        let response = post_json(
            app,
            "/events",
            &AppendApplicationEventRequest {
                event: request.clone(),
                delivery_policy: policy,
            },
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{point:?}"
        );
        clear_http_application_event_crash_triggers(&db_path);

        let app = persistent_app(&db_path);
        assert_application_event_rolled_back(&app, &room_id, &message_id).await;

        let response = post_json(
            app.clone(),
            "/events",
            &AppendApplicationEventRequest {
                event: request.clone(),
                delivery_policy: policy,
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{point:?}");
        let accepted: EventAccepted = read_json(response).await;
        assert_eq!(accepted.seq, 1);
        assert_eq!(accepted.message_id, message_id);

        let app = persistent_app(&db_path);
        let response = post_json(
            app.clone(),
            "/events",
            &AppendApplicationEventRequest {
                event: request,
                delivery_policy: policy,
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{point:?}");
        let replayed: EventAccepted = read_json(response).await;
        assert_eq!(replayed, accepted);

        assert_application_event_converged(&app, &room_id, &accepted.message_id).await;
    }
}

#[tokio::test]
async fn sqlite_typed_event_sync_returns_bounded_pages_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-event-bounded-sync".to_owned();
    let mls_group_id = "mls-event-bounded-sync".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    for index in 0..=MAX_HTTP_SYNC_PAGE_ENTRIES {
        let response = post_json(
            app.clone(),
            "/events",
            &typed_event_request(&append_application_request(
                &room_id,
                &mls_group_id,
                &alice,
                0,
                format!("small-{index}").as_bytes(),
                &format!("bounded-event-{index}"),
            )),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: EventAccepted = read_json(response).await;
        assert_eq!(accepted.seq, (index as u64) + 1);
    }

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: MAX_HTTP_SYNC_PAGE_ENTRIES,
            requester: Some(member_for_device(&alice)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let first_page: HttpSyncPage = read_json(response).await;
    assert_eq!(first_page.entries.len(), MAX_HTTP_SYNC_PAGE_ENTRIES);
    assert_eq!(first_page.entries.first().unwrap().seq, 1);
    assert_eq!(
        first_page.entries.last().unwrap().seq,
        MAX_HTTP_SYNC_PAGE_ENTRIES as u64
    );
    assert_eq!(first_page.next_after_seq, MAX_HTTP_SYNC_PAGE_ENTRIES as u64);
    assert!(first_page.has_more);

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: first_page.next_after_seq,
            limit: MAX_HTTP_SYNC_PAGE_ENTRIES,
            requester: Some(member_for_device(&alice)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let second_page: HttpSyncPage = read_json(response).await;
    assert_eq!(second_page.entries.len(), 1);
    assert_eq!(
        second_page.entries[0].seq,
        (MAX_HTTP_SYNC_PAGE_ENTRIES as u64) + 1
    );
    assert_eq!(
        second_page.next_after_seq,
        (MAX_HTTP_SYNC_PAGE_ENTRIES as u64) + 1
    );
    assert!(!second_page.has_more);
}

#[tokio::test]
async fn sqlite_ephemeral_activity_over_http_does_not_persist_or_advance_sequence() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-ephemeral-activity-volatile".to_owned();
    let mls_group_id = "mls-ephemeral-activity-volatile".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let request = ephemeral_activity_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        Some("topic-activity"),
        1_000,
    );
    let response = post_json(app.clone(), "/activities", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: EphemeralActivityAccepted = read_json(response).await;
    assert_eq!(accepted.cached_events_for_route, 1);
    assert_eq!(
        accepted.route_key,
        finitechat_proto::ephemeral_activity_route_key(&room_id, Some("topic-activity"), &alice)
    );

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
    assert_eq!(page.next_after_seq, 0);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
    assert_eq!(page.next_after_seq, 0);

    let response = post_json(app, "/activities", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: EphemeralActivityAccepted = read_json(response).await;
    assert_eq!(accepted.cached_events_for_route, 1);
}

#[tokio::test]
async fn sqlite_ephemeral_activity_route_scope_and_opaque_payload_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-ephemeral-activity-scope".to_owned();
    let mls_group_id = "mls-ephemeral-activity-scope".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let topic_1_key =
        finitechat_proto::ephemeral_activity_route_key(&room_id, Some("topic-1"), &alice);
    for (index, payload) in [
        br#"{"kind":"typing","activity_id":"same-id"}"#.as_slice(),
        br#"{"kind":"working","activity_id":"same-id"}"#.as_slice(),
    ]
    .into_iter()
    .enumerate()
    {
        let mut request = ephemeral_activity_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            Some("topic-1"),
            1_000 + u64::try_from(index).unwrap(),
        );
        request.payload = payload.to_vec();
        let response = post_json(app.clone(), "/activities", &request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: EphemeralActivityAccepted = read_json(response).await;
        assert_eq!(accepted.route_key, topic_1_key);
        assert_eq!(
            accepted.cached_events_for_route,
            u32::try_from(index + 1).unwrap()
        );
    }

    let mut topic_2 =
        ephemeral_activity_request(&room_id, &mls_group_id, &alice, 0, Some("topic-2"), 2_000);
    topic_2.payload = br#"{"kind":"typing","activity_id":"same-id"}"#.to_vec();
    let response = post_json(app.clone(), "/activities", &topic_2).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: EphemeralActivityAccepted = read_json(response).await;
    assert_eq!(
        accepted.route_key,
        finitechat_proto::ephemeral_activity_route_key(&room_id, Some("topic-2"), &alice)
    );
    assert_eq!(accepted.cached_events_for_route, 1);

    let mut room_wide = ephemeral_activity_request(&room_id, &mls_group_id, &alice, 0, None, 3_000);
    room_wide.payload = br#"{"kind":"typing","activity_id":"same-id"}"#.to_vec();
    let response = post_json(app.clone(), "/activities", &room_wide).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: EphemeralActivityAccepted = read_json(response).await;
    assert_eq!(
        accepted.route_key,
        finitechat_proto::ephemeral_activity_route_key(&room_id, None, &alice)
    );
    assert_eq!(accepted.cached_events_for_route, 1);

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
    assert_eq!(page.next_after_seq, 0);

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/activities",
        &ephemeral_activity_request(&room_id, &mls_group_id, &alice, 0, Some("topic-1"), 4_000),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: EphemeralActivityAccepted = read_json(response).await;
    assert_eq!(accepted.route_key, topic_1_key);
    assert_eq!(accepted.cached_events_for_route, 1);
}

#[tokio::test]
async fn sqlite_ephemeral_activity_over_http_authorizes_members_and_bounds_cache() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-ephemeral-activity-auth".to_owned();
    let mls_group_id = "mls-ephemeral-activity-auth".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let add_bob = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-ephemeral-bob",
        "commit-ephemeral-bob",
    );
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);

    let pending = ephemeral_activity_request(&room_id, &mls_group_id, &bob, 1, None, 1_000);
    let response = post_json(app.clone(), "/activities", &pending).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "sender_not_active");

    let wrong_epoch = ephemeral_activity_request(&room_id, &mls_group_id, &alice, 0, None, 1_000);
    let response = post_json(app.clone(), "/activities", &wrong_epoch).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_activity_request");

    let expired = AppendEphemeralActivityRequest {
        expires_at_ms: 1_000,
        ..ephemeral_activity_request(&room_id, &mls_group_id, &alice, 1, None, 1_000)
    };
    let response = post_json(app.clone(), "/activities", &expired).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_activity_request");

    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: member_for_device(&bob),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claimed.len(), 1);
    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-ephemeral-bob"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    for index in 0..=MAX_EPHEMERAL_ACTIVITY_CACHE_ENTRIES_PER_ROUTE {
        let mut request = ephemeral_activity_request(
            &room_id,
            &mls_group_id,
            &bob,
            1,
            Some("topic-route"),
            2_000 + u64::from(index),
        );
        request.payload = vec![0xff, index as u8];
        let response = post_json(app.clone(), "/activities", &request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: EphemeralActivityAccepted = read_json(response).await;
        assert_eq!(
            accepted.cached_events_for_route,
            (index + 1).min(MAX_EPHEMERAL_ACTIVITY_CACHE_ENTRIES_PER_ROUTE)
        );
    }

    revoke_device(&app, &bob).await;
    let response = post_json(
        app.clone(),
        "/activities",
        &ephemeral_activity_request(&room_id, &mls_group_id, &bob, 1, Some("topic-route"), 3_000),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_revoked");

    let response = post_json(
        app,
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.next_after_seq, 1);
}

#[tokio::test]
async fn sqlite_nostr_profile_cache_survives_restart_and_reports_stale_reads() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let account_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned();

    let profile = NostrProfileRecord {
        account_id: account_id.clone(),
        name: Some("alice".to_owned()),
        display_name: Some("Alice Finite".to_owned()),
        about: Some("FiniteChat test profile".to_owned()),
        picture: Some("https://example.invalid/alice.png".to_owned()),
        bot: None,
        finite_role: None,
        metadata_json: None,
        fetched_at_ms: 1_000,
        expires_at_ms: 2_000,
    };

    {
        let app = persistent_app(&db_path);
        let response = post_json(
            app.clone(),
            "/profiles/nostr",
            &PutNostrProfileRequest {
                profile: profile.clone(),
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let response = post_json(
            app,
            "/profiles/nostr/get",
            &GetNostrProfilesRequest {
                account_ids: vec![account_id.clone()],
                now_ms: 1_500,
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let profiles: GetNostrProfilesResponse = read_json(response).await;
        assert_eq!(profiles.profiles.len(), 1);
        assert_eq!(profiles.profiles[0].profile.account_id, profile.account_id);
        assert_eq!(profiles.profiles[0].profile.name, profile.name);
        assert_eq!(
            profiles.profiles[0].profile.display_name,
            profile.display_name
        );
        assert_eq!(profiles.profiles[0].profile.about, profile.about);
        assert_eq!(profiles.profiles[0].profile.picture, profile.picture);
        assert_eq!(profiles.profiles[0].profile.bot, profile.bot);
        assert_eq!(
            profiles.profiles[0].profile.finite_role,
            profile.finite_role
        );
        assert_eq!(
            profiles.profiles[0].profile.fetched_at_ms,
            profile.fetched_at_ms
        );
        assert_eq!(
            profiles.profiles[0].profile.expires_at_ms,
            profile.expires_at_ms
        );
        let metadata: serde_json::Value = serde_json::from_str(
            profiles.profiles[0]
                .profile
                .metadata_json
                .as_deref()
                .expect("normalized profile metadata"),
        )
        .expect("metadata json");
        assert_eq!(metadata["display_name"], "Alice Finite");
        assert_eq!(metadata["picture"], "https://example.invalid/alice.png");
        assert!(!profiles.profiles[0].stale);
    }

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/profiles/nostr/get",
        &GetNostrProfilesRequest {
            account_ids: vec![
                account_id.clone(),
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned(),
            ],
            now_ms: 2_500,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let profiles: GetNostrProfilesResponse = read_json(response).await;
    assert_eq!(profiles.profiles.len(), 1);
    assert_eq!(profiles.profiles[0].profile.account_id, account_id);
    assert!(profiles.profiles[0].stale);
}

#[tokio::test]
async fn sqlite_nostr_profile_cache_preserves_unknown_metadata_fields_on_edit() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let app = persistent_app(&db_path);
    let account_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned();

    let original = NostrProfileRecord {
        account_id: account_id.clone(),
        name: Some("alice".to_owned()),
        display_name: Some("Alice".to_owned()),
        about: Some("Original profile".to_owned()),
        picture: Some("https://example.invalid/original.png".to_owned()),
        bot: Some(true),
        finite_role: Some("agent".to_owned()),
        metadata_json: Some(
            r#"{"about":"Original profile","display_name":"Alice","lud16":"alice@example.com","picture":"https://example.invalid/original.png","website":"https://alice.example"}"#.to_owned(),
        ),
        fetched_at_ms: 1_000,
        expires_at_ms: 2_000,
    };
    let response = post_json(
        app.clone(),
        "/profiles/nostr",
        &PutNostrProfileRequest {
            profile: original.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let edited = NostrProfileRecord {
        account_id: account_id.clone(),
        name: Some("alice-updated".to_owned()),
        display_name: Some("Alice Updated".to_owned()),
        about: None,
        picture: Some("https://example.invalid/updated.png".to_owned()),
        bot: None,
        finite_role: None,
        metadata_json: None,
        fetched_at_ms: 3_000,
        expires_at_ms: 4_000,
    };
    let response = post_json(
        app.clone(),
        "/profiles/nostr",
        &PutNostrProfileRequest { profile: edited },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app,
        "/profiles/nostr/get",
        &GetNostrProfilesRequest {
            account_ids: vec![account_id],
            now_ms: 3_500,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let profiles: GetNostrProfilesResponse = read_json(response).await;
    assert_eq!(profiles.profiles.len(), 1);
    let profile = &profiles.profiles[0].profile;
    assert_eq!(profile.display_name.as_deref(), Some("Alice Updated"));
    assert_eq!(
        profile.picture.as_deref(),
        Some("https://example.invalid/updated.png")
    );
    assert_eq!(profile.bot, Some(true));
    assert_eq!(profile.finite_role.as_deref(), Some("agent"));

    let metadata: serde_json::Value =
        serde_json::from_str(profile.metadata_json.as_deref().expect("metadata json"))
            .expect("metadata json object");
    assert_eq!(metadata["name"], "alice-updated");
    assert_eq!(metadata["display_name"], "Alice Updated");
    assert_eq!(metadata["picture"], "https://example.invalid/updated.png");
    assert_eq!(metadata["lud16"], "alice@example.com");
    assert_eq!(metadata["website"], "https://alice.example");
    assert_eq!(metadata["bot"], true);
    assert_eq!(metadata["finite_role"], "agent");
    assert!(metadata.get("about").is_none());
}

#[tokio::test]
async fn sqlite_nostr_profile_cache_rejects_invalid_records_without_side_effects() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let app = persistent_app(&db_path);
    let account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned();

    let response = post_json(
        app.clone(),
        "/profiles/nostr",
        &PutNostrProfileRequest {
            profile: NostrProfileRecord {
                account_id: "not-an-account".to_owned(),
                name: Some("alice".to_owned()),
                display_name: None,
                about: None,
                picture: None,
                bot: None,
                finite_role: None,
                metadata_json: None,
                fetched_at_ms: 1_000,
                expires_at_ms: 2_000,
            },
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_nostr_profile_request");

    let response = post_json(
        app.clone(),
        "/profiles/nostr",
        &PutNostrProfileRequest {
            profile: NostrProfileRecord {
                account_id: account_id.clone(),
                name: Some("alice".to_owned()),
                display_name: None,
                about: None,
                picture: Some("file:///tmp/alice.png".to_owned()),
                bot: None,
                finite_role: None,
                metadata_json: None,
                fetched_at_ms: 1_000,
                expires_at_ms: 2_000,
            },
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_nostr_profile_request");

    let response = post_json(
        app,
        "/profiles/nostr/get",
        &GetNostrProfilesRequest {
            account_ids: vec![account_id],
            now_ms: 1_500,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let profiles: GetNostrProfilesResponse = read_json(response).await;
    assert!(profiles.profiles.is_empty());
}

#[tokio::test]
async fn sqlite_device_liveness_is_volatile_and_does_not_advance_room_state() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-device-liveness".to_owned();
    let mls_group_id = "mls-device-liveness".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app.clone(),
        "/devices/liveness",
        &ObserveDeviceLivenessRequest {
            device: alice.clone(),
            observed_at_ms: 1_000,
            expires_at_ms: 1_000 + MAX_DEVICE_LIVENESS_EXPIRY_MILLIS,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let heartbeat: DeviceLivenessRecord = read_json(response).await;
    assert_eq!(heartbeat.device, alice);
    assert_eq!(heartbeat.observed_at_ms, 1_000);
    assert_eq!(
        heartbeat.expires_at_ms,
        1_000 + MAX_DEVICE_LIVENESS_EXPIRY_MILLIS
    );

    let response = post_json(
        app.clone(),
        "/devices/liveness",
        &ObserveDeviceLivenessRequest {
            device: alice.clone(),
            observed_at_ms: 1_000,
            expires_at_ms: 1_500,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let stale_replay: DeviceLivenessRecord = read_json(response).await;
    assert_eq!(stale_replay, heartbeat);

    let response = post_json(
        app.clone(),
        "/devices/liveness/get",
        &GetDeviceLivenessRequest {
            device: alice.clone(),
            now_ms: 60_999,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let live: GetDeviceLivenessResponse = read_json(response).await;
    assert_eq!(live.record, Some(heartbeat.clone()));
    assert!(live.live);

    let response = post_json(
        app.clone(),
        "/devices/liveness/get",
        &GetDeviceLivenessRequest {
            device: alice.clone(),
            now_ms: 61_000,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let expired: GetDeviceLivenessResponse = read_json(response).await;
    assert_eq!(expired.record, Some(heartbeat));
    assert!(!expired.live);

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: Some(member_for_device(&alice)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
    assert_eq!(page.next_after_seq, 0);
    assert_eq!(
        application_effect_counts(&app).await,
        ApplicationEffectCountsResponse {
            push_outbox: 0,
            unread: 0,
            command_inbox: 0,
        }
    );

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/devices/liveness/get",
        &GetDeviceLivenessRequest {
            device: alice.clone(),
            now_ms: 1_001,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let after_restart: GetDeviceLivenessResponse = read_json(response).await;
    assert_eq!(
        after_restart,
        GetDeviceLivenessResponse {
            record: None,
            live: false,
        }
    );

    let response = post_json(
        app,
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: Some(member_for_device(&alice)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
    assert_eq!(page.next_after_seq, 0);
}

#[tokio::test]
async fn sqlite_device_liveness_rejects_bad_observations_without_room_side_effects() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-device-liveness-reject".to_owned();
    let mls_group_id = "mls-device-liveness-reject".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let charlie = DeviceRef::new("charlie", "charlie-phone");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let add_bob = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-liveness-bob",
        "commit-liveness-bob",
    );
    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, 1);

    let response = post_json(
        app.clone(),
        "/devices/liveness",
        &ObserveDeviceLivenessRequest {
            device: alice.clone(),
            observed_at_ms: 1_000,
            expires_at_ms: 1_000 + MAX_DEVICE_LIVENESS_EXPIRY_MILLIS + 1,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_device_liveness_request");

    let response = post_json(
        app.clone(),
        "/devices/liveness",
        &ObserveDeviceLivenessRequest {
            device: bob.clone(),
            observed_at_ms: 1_000,
            expires_at_ms: 1_500,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_not_active");

    let response = post_json(
        app.clone(),
        "/devices/liveness",
        &ObserveDeviceLivenessRequest {
            device: charlie,
            observed_at_ms: 1_000,
            expires_at_ms: 1_500,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_not_active");

    let response = post_json(
        app.clone(),
        "/devices/revoke",
        &RevokeDeviceRequest {
            device: alice.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = post_json(
        app.clone(),
        "/devices/liveness",
        &ObserveDeviceLivenessRequest {
            device: alice,
            observed_at_ms: 2_000,
            expires_at_ms: 2_500,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "device_revoked");

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: accepted.seq,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
    assert_eq!(page.next_after_seq, accepted.seq);
    assert_eq!(
        application_effect_counts(&app).await,
        ApplicationEffectCountsResponse {
            push_outbox: 0,
            unread: 0,
            command_inbox: 0,
        }
    );
}

#[tokio::test]
async fn sqlite_invalid_commit_report_blocks_typed_mutations_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let carol = DeviceRef::new("carol", "carol-phone");
    let room_id = "room-invalid-commit-report".to_owned();
    let mls_group_id = "mls-invalid-commit-report".to_owned();
    let request = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-invalid-report-bob",
        "invalid-report-add-bob",
    );

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    publish_and_claim_key_package_for_add(&app, &request).await;
    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;

    let response = post_json(
        app.clone(),
        "/rooms/report-invalid-commit",
        &ReportInvalidCommitRequest {
            room_id: room_id.clone(),
            reporter: carol.clone(),
            offending_seq: accepted.seq,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "reporter_not_in_interval");

    let response = post_json(
        app,
        "/rooms/report-invalid-commit",
        &ReportInvalidCommitRequest {
            room_id: room_id.clone(),
            reporter: alice.clone(),
            offending_seq: accepted.seq,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let reported: ReportInvalidCommitResponse = read_json(response).await;
    assert!(reported.reported);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: "alice".to_owned(),
            after_room_id: None,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert_eq!(page.rooms.len(), 1);
    assert_eq!(page.rooms[0]["status"], "needs_repair");

    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            1,
            b"blocked",
            "invalid-report-blocked-event",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "room_not_open");

    let blocked_commit =
        submit_add_device_request_at_epoch(&room_id, &mls_group_id, &alice, &carol, 1);
    let response = post_json(app, "/commits", &blocked_commit).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "room_not_open");
}

#[tokio::test]
async fn sqlite_welcome_activation_marks_account_room_device_active_after_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let creator = DeviceRef {
        account_id: "alice".to_owned(),
        device_id: "alice-laptop".to_owned(),
    };
    let phone = DeviceRef {
        account_id: "alice".to_owned(),
        device_id: "alice-phone".to_owned(),
    };
    let room_id = "room-welcome-activation".to_owned();
    let mls_group_id = "mls-welcome-activation".to_owned();
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms",
        &SaveAccountRoomRequest {
            account_id: "alice".to_owned(),
            room_id: room_id.clone(),
            record: serde_json::to_value(&AccountRoomRecord {
                room_id: room_id.clone(),
                mls_group_id,
                current_epoch: 2,
                last_seq: 7,
                status: RoomStatus::Open,
                devices: vec![
                    AccountRoomDevice {
                        device: creator,
                        active: true,
                    },
                    AccountRoomDevice {
                        device: phone.clone(),
                        active: false,
                    },
                ],
            })
            .expect("account-room record json"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let recipient = member_for_device(&phone);
    let welcome_record = WelcomeRecord {
        welcome_id: "welcome-phone-activation".to_owned(),
        room_id: room_id.clone(),
        commit_seq: 7,
        recipient: phone.clone(),
        sender: DeviceRef {
            account_id: "alice".to_owned(),
            device_id: "alice-laptop".to_owned(),
        },
        key_package_id: "kp-phone-activation".to_owned(),
        join_epoch: 2,
        state: WelcomeState::Released,
        lease_token: Some("lease-phone-activation".to_owned()),
        welcome_payload: b"welcome-bytes".to_vec(),
        ratchet_tree_payload: b"ratchet-tree".to_vec(),
    };
    let welcome_payload = serde_json::to_vec(&welcome_record).expect("welcome record json");
    let seed_state = persistent_state(&db_path);
    seed_state
        .publish_message(PublishMessageRequest {
            target: HttpPublishTarget::Inbox {
                recipient: recipient.clone(),
            },
            message: welcome_message(
                "welcome-phone-activation",
                recipient.clone(),
                &welcome_payload,
            ),
            idempotency_key: None,
        })
        .expect("seed welcome inbox");
    drop(seed_state);
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: recipient.clone(),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claimed.len(), 1);

    let response = post_json(
        app,
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-phone-activation"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: "alice".to_owned(),
            after_room_id: None,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert_eq!(page.rooms.len(), 1);
    assert_eq!(
        page.rooms[0]["devices"][1]["device"]["device_id"],
        "alice-phone"
    );
    assert_eq!(page.rooms[0]["devices"][1]["active"], true);

    let response = post_json(
        app,
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-phone-activation"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn sqlite_delayed_welcome_syncs_forward_from_commit_seq_over_http() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-delayed-welcome-sync".to_owned();
    let mls_group_id = "mls-delayed-welcome-sync".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let add_bob = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-delayed-sync-bob",
        "commit-delayed-sync-bob",
    );
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted_add: CommitAccepted = read_json(response).await;
    assert_eq!(accepted_add.seq, 1);

    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            1,
            b"later-before-welcome-ack",
            "delayed-welcome-later-event",
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let later: EventAccepted = read_json(response).await;
    assert_eq!(later.seq, 2);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: member_for_device(&bob),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].message.id, id("welcome-delayed-sync-bob"));

    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-delayed-sync-bob"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: accepted_add.seq,
            limit: 10,
            requester: Some(member_for_device(&bob)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].seq, later.seq);
    assert_eq!(page.entries[0].message.id, id(&later.message_id));
    assert_eq!(page.next_after_seq, later.seq);
    assert!(!page.has_more);
}

#[tokio::test]
async fn sqlite_welcome_claim_survives_restart_before_ack() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let recipient = member("welcome-recipient");
    let welcome = PublishMessageRequest {
        target: HttpPublishTarget::Inbox {
            recipient: recipient.clone(),
        },
        message: welcome_message("welcome-restart", recipient.clone(), b"welcome-bytes"),
        idempotency_key: Some("idem-welcome-restart".to_owned()),
    };

    let state = persistent_state(&db_path);
    state
        .publish_message(welcome.clone())
        .expect("seed welcome");
    let app = http_router(state);

    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: recipient.clone(),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].seq, 1);
    assert_eq!(claimed[0].message.id, id("welcome-restart"));

    let response = post_json(
        app,
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: recipient.clone(),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let duplicate_claim: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert!(duplicate_claim.is_empty());

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let after_restart_claim: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert!(after_restart_claim.is_empty());

    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-restart"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let acked: AckWelcomeResponse = read_json(response).await;
    assert!(acked.acked);

    let app = persistent_app(&db_path);
    let response = post_json(
        app,
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-restart"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let acked: AckWelcomeResponse = read_json(response).await;
    assert!(acked.acked);
}

#[tokio::test]
async fn sqlite_mixed_http_operation_fuzzer_survives_restarts() {
    for seed in 1..=4 {
        run_mixed_http_operation_fuzz(seed).await;
    }
}

async fn run_mixed_http_operation_fuzz(seed: u64) {
    const STEPS: usize = 32;

    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join(format!("mixed-http-fuzz-{seed}.sqlite3"));
    let room_id = format!("room-http-fuzz-{seed}");
    let mls_group_id = format!("mls-http-fuzz-{seed}");
    let alice = DeviceRef::new("alice", format!("alice-http-fuzz-{seed}"));
    let bob = DeviceRef::new("bob", format!("bob-http-fuzz-{seed}"));
    let mut rng = HttpFuzzRng::new(seed);
    let mut last_seq: u64;
    let mut effectful_events = 0u32;
    let mut first_raw_event: Option<(AppendEventRequest, EventAccepted)> = None;

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let welcome_id = format!("welcome-http-fuzz-bob-{seed}");
    let add_bob = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        &welcome_id,
        "add-bob",
    );
    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    let add_accepted: CommitAccepted = read_json(response).await;
    last_seq = add_accepted.seq;
    assert_eq!(last_seq, 1);

    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed_add: CommitAccepted = read_json(response).await;
    assert_eq!(replayed_add, add_accepted);

    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: member_for_device(&bob),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claimed.len(), 1);

    let response = post_json(
        app,
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id(&welcome_id),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    for step in 0..STEPS {
        let app = persistent_app(&db_path);
        match rng.next_usize(8) {
            0 => {
                let sender = if rng.next_bool() { &alice } else { &bob };
                let request = append_application_request(
                    &room_id,
                    &mls_group_id,
                    sender,
                    1,
                    format!(r#"{{"type":"chat.message","seed":{seed},"step":{step}}}"#).as_bytes(),
                    &format!("mixed-http-fuzz-raw-{seed}-{step}"),
                );
                let response = post_json(app, "/events", &typed_event_request(&request)).await;
                assert_eq!(response.status(), StatusCode::OK);
                let accepted: EventAccepted = read_json(response).await;
                assert_eq!(accepted.seq, last_seq + 1);
                last_seq = accepted.seq;
                // Every fresh typed event now records delivery effects.
                effectful_events += 1;
                if first_raw_event.is_none() {
                    first_raw_event = Some((request, accepted));
                }
            }
            1 => {
                let sender = if rng.next_bool() { &alice } else { &bob };
                let request = append_application_request(
                    &room_id,
                    &mls_group_id,
                    sender,
                    1,
                    format!(r#"{{"type":"chat.message","effect":{seed},"step":{step}}}"#)
                        .as_bytes(),
                    &format!("mixed-http-fuzz-effect-{seed}-{step}"),
                );
                let response = post_json(
                    app.clone(),
                    "/events",
                    &AppendApplicationEventRequest {
                        event: request,
                        delivery_policy: DurableAppEventKind::ChatMessage.delivery_policy(),
                    },
                )
                .await;
                assert_eq!(response.status(), StatusCode::OK);
                let accepted: EventAccepted = read_json(response).await;
                assert_eq!(accepted.seq, last_seq + 1);
                last_seq = accepted.seq;
                effectful_events += 1;
                assert!(
                    application_effect(&app, &accepted.message_id)
                        .await
                        .is_some()
                );
            }
            2 => {
                let sender = if rng.next_bool() { &alice } else { &bob };
                let response = post_json(
                    app,
                    "/activities",
                    &ephemeral_activity_request(
                        &room_id,
                        &mls_group_id,
                        sender,
                        1,
                        Some("mixed-http-fuzz-topic"),
                        1_800_000_000 + step as u64,
                    ),
                )
                .await;
                assert_eq!(response.status(), StatusCode::OK);
            }
            3 => {
                let after_seq = rng.next_u64(last_seq + 1);
                let response = post_json(
                    app,
                    "/sync/group",
                    &GroupSyncRequest {
                        group_id: group_id(&room_id),
                        after_seq,
                        limit: 7,
                        requester: None,
                    },
                )
                .await;
                assert_eq!(response.status(), StatusCode::OK);
                let page: HttpSyncPage = read_json(response).await;
                assert!(page.next_after_seq >= after_seq);
                assert!(page.next_after_seq <= last_seq);
                assert!(page.entries.len() <= 7);
            }
            4 => {
                let response = post_json(
                    app,
                    "/welcomes/claim",
                    &ClaimWelcomesRequest {
                        recipient: member_for_device(&bob),
                        limit: 10,
                    },
                )
                .await;
                assert_eq!(response.status(), StatusCode::OK);
                let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
                assert!(claimed.is_empty());
            }
            5 => {
                let observed_at_ms = 1_900_000_000 + step as u64;
                let response = post_json(
                    app.clone(),
                    "/devices/liveness",
                    &ObserveDeviceLivenessRequest {
                        device: bob.clone(),
                        observed_at_ms,
                        expires_at_ms: observed_at_ms + 1_000,
                    },
                )
                .await;
                assert_eq!(response.status(), StatusCode::OK);

                let response = post_json(
                    app,
                    "/devices/liveness/get",
                    &GetDeviceLivenessRequest {
                        device: bob.clone(),
                        now_ms: observed_at_ms,
                    },
                )
                .await;
                assert_eq!(response.status(), StatusCode::OK);
                let liveness: GetDeviceLivenessResponse = read_json(response).await;
                assert!(liveness.live);
            }
            6 => {
                let response = post_json(app, "/commits", &add_bob).await;
                assert_eq!(response.status(), StatusCode::OK);
                let replayed: CommitAccepted = read_json(response).await;
                assert_eq!(replayed, add_accepted);
            }
            _ => {
                if let Some((request, accepted)) = &first_raw_event {
                    let response = post_json(app, "/events", &typed_event_request(request)).await;
                    assert_eq!(response.status(), StatusCode::OK);
                    let replayed: EventAccepted = read_json(response).await;
                    assert_eq!(&replayed, accepted);
                } else {
                    let response = post_json(
                        app,
                        "/sync/group",
                        &GroupSyncRequest {
                            group_id: group_id(&room_id),
                            after_seq: 0,
                            limit: 7,
                            requester: None,
                        },
                    )
                    .await;
                    assert_eq!(response.status(), StatusCode::OK);
                }
            }
        }
    }

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 50,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.next_after_seq, last_seq);
    assert_eq!(page.entries.len(), last_seq as usize);

    let counts = application_effect_counts(&app).await;
    assert_eq!(counts.push_outbox, effectful_events);
    assert_eq!(counts.unread, effectful_events);
    assert_eq!(counts.command_inbox, 0);

    let page = account_room_page(&app, &bob.account_id).await;
    assert!(account_room_device_active(&page, &bob));
}

struct HttpFuzzRng {
    state: u64,
}

impl HttpFuzzRng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9e37_79b9_7f4a_7c15,
        }
    }

    fn next_u64(&mut self, upper_bound: u64) -> u64 {
        assert!(upper_bound > 0);
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.state % upper_bound
    }

    fn next_usize(&mut self, upper_bound: usize) -> usize {
        self.next_u64(upper_bound as u64) as usize
    }

    fn next_bool(&mut self) -> bool {
        self.next_u64(2) == 0
    }
}

fn typed_event_request(event: &AppendEventRequest) -> AppendApplicationEventRequest {
    AppendApplicationEventRequest {
        event: event.clone(),
        delivery_policy: DurableAppEventKind::ChatMessage.delivery_policy(),
    }
}

fn persistent_app(path: &std::path::Path) -> Router {
    http_router(persistent_state(path))
}

fn persistent_state(path: &std::path::Path) -> HttpServerState {
    HttpServerState::from_sqlite_path(path).expect("persistent server state")
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

async fn put_blob(app: Router, body: &[u8]) -> Response<Body> {
    put_blob_with_content_type(app, body, "application/octet-stream").await
}

async fn put_blob_with_content_type(
    app: Router,
    body: &[u8],
    content_type: &str,
) -> Response<Body> {
    app.oneshot(
        Request::builder()
            .method(Method::PUT)
            .uri("/upload")
            .header("content-type", content_type)
            .header("host", "blob.test")
            .body(Body::from(body.to_vec()))
            .expect("request"),
    )
    .await
    .expect("response")
}

async fn get_blob(app: Router, sha256: &str) -> Response<Body> {
    app.oneshot(
        Request::builder()
            .method(Method::GET)
            .uri(format!("/blobs/{sha256}"))
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

async fn read_body(response: Response<Body>) -> Bytes {
    to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body")
}

async fn read_next_sync_hint<S>(stream: &mut S) -> SyncHintEvent
where
    S: futures_util::Stream<Item = Result<Bytes, axum::Error>> + Unpin,
{
    let mut buffer = String::new();
    loop {
        while !buffer.contains("\n\n") {
            let chunk = stream
                .next()
                .await
                .expect("SSE stream ended before event")
                .expect("SSE chunk");
            buffer.push_str(std::str::from_utf8(&chunk).expect("SSE is UTF-8"));
        }
        let Some(split_at) = buffer.find("\n\n") else {
            continue;
        };
        let raw_event = buffer[..split_at].to_owned();
        buffer = buffer[split_at + 2..].to_owned();
        let data = raw_event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        return serde_json::from_str(&data).expect("sync hint JSON");
    }
}

async fn assert_inventory(app: Router, owner: MemberId, available: u32, claimed: u32) {
    let response = post_json(
        app,
        "/key-packages/inventory",
        &KeyPackageInventoryRequest {
            owner: owner.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let inventory: HttpKeyPackageInventory = read_json(response).await;
    assert_eq!(inventory.owner, owner);
    assert_eq!(inventory.available, available);
    assert_eq!(inventory.claimed, claimed);
}

async fn application_effect_counts(app: &Router) -> ApplicationEffectCountsResponse {
    let response = post_json(
        app.clone(),
        "/application-effects/counts",
        &serde_json::json!({}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    read_json(response).await
}

async fn application_effect(
    app: &Router,
    message_id: &str,
) -> Option<HttpApplicationDeliveryEffect> {
    let response = post_json(
        app.clone(),
        "/application-effects/get",
        &ApplicationEffectRequest {
            message_id: message_id.to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    read_json(response).await
}

async fn assert_application_event_rolled_back(app: &Router, room_id: &str, message_id: &str) {
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());

    assert_eq!(
        application_effect_counts(app).await,
        ApplicationEffectCountsResponse {
            push_outbox: 0,
            unread: 0,
            command_inbox: 0,
        }
    );
    assert_eq!(application_effect(app, message_id).await, None);
}

async fn assert_application_event_converged(app: &Router, room_id: &str, message_id: &str) {
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].message.id, id(message_id));

    assert_eq!(
        application_effect_counts(app).await,
        ApplicationEffectCountsResponse {
            push_outbox: 1,
            unread: 1,
            command_inbox: 0,
        }
    );
    let effect = application_effect(app, message_id).await.expect("effect");
    assert_eq!(effect.seq, 1);
    assert_eq!(effect.message_id, message_id);
    assert!(effect.delivery_policy.creates_push());
    assert!(effect.delivery_policy.creates_unread());
    assert!(!effect.delivery_policy.creates_command_inbox_work());
}

async fn revoke_device(app: &Router, device: &DeviceRef) {
    let response = post_json(
        app.clone(),
        "/devices/revoke",
        &RevokeDeviceRequest {
            device: device.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

fn id(label: &str) -> MessageId {
    MessageId::new(label.as_bytes().to_vec())
}

fn group_id(label: &str) -> GroupId {
    GroupId::new(label.as_bytes().to_vec())
}

fn member(label: &str) -> MemberId {
    MemberId::new(label.as_bytes().to_vec())
}

fn member_for_device(device: &DeviceRef) -> MemberId {
    MemberId::new(delivery_member_id_for_device(device))
}

#[tokio::test]
async fn sqlite_room_admin_metadata_does_not_gate_membership_commits_and_survives_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-laptop");
    let carol = DeviceRef::new("carol", "carol-laptop");
    let room_id = "room-admin-authority".to_owned();
    let mls_group_id = "mls-admin-authority".to_owned();

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // The creator starts as admin metadata, but the relay does not use that
    // metadata as room authority for encrypted membership commits.
    let add_bob = submit_add_device_request_at_epoch(&room_id, &mls_group_id, &alice, &bob, 0);
    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bob_accepted: CommitAccepted = read_json(response).await;

    // Activate bob so he is an active (non-admin) member.
    let bob_recipient = member_for_device(&bob);
    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: bob_recipient.clone(),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claims: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claims.len(), 1);
    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: claims[0].message.id.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // A non-admin active member may still submit a structurally valid
    // cross-account add. Server-side admin state is not protocol authority.
    let bob_adds_carol = submit_add_device_request_at_epoch_with_ids(
        &room_id,
        &mls_group_id,
        &bob,
        &carol,
        1,
        "welcome-admin-carol-open",
        "commit-admin-carol-open",
    );
    publish_and_claim_key_package_for_add(&app, &bob_adds_carol).await;
    let response = post_json(app.clone(), "/commits", &bob_adds_carol).await;
    assert_eq!(response.status(), StatusCode::OK);
    let carol_accepted: CommitAccepted = read_json(response).await;
    assert_eq!(carol_accepted.seq, bob_accepted.seq + 1);

    // Same-account linking remains accepted as ordinary membership evolution.
    let bob_phone = DeviceRef::new("bob", "bob-phone");
    let bob_adds_own = submit_add_device_request_at_epoch_with_ids(
        &room_id,
        &mls_group_id,
        &bob,
        &bob_phone,
        2,
        "welcome-admin-bob-phone",
        "commit-admin-bob-phone",
    );
    publish_and_claim_key_package_for_add(&app, &bob_adds_own).await;
    let response = post_json(app.clone(), "/commits", &bob_adds_own).await;
    assert_eq!(response.status(), StatusCode::OK);
    let own_accepted: CommitAccepted = read_json(response).await;
    assert_eq!(own_accepted.seq, carol_accepted.seq + 1);

    // A non-admin cannot grant admin.
    let response = post_json(
        app.clone(),
        "/rooms/admins",
        &UpdateRoomAdminsRequest {
            room_id: room_id.clone(),
            sender: bob.clone(),
            grant: Some("bob".to_owned()),
            revoke: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // The admin grants bob.
    let response = post_json(
        app.clone(),
        "/rooms/admins",
        &UpdateRoomAdminsRequest {
            room_id: room_id.clone(),
            sender: alice.clone(),
            grant: Some("bob".to_owned()),
            revoke: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let granted: UpdateRoomAdminsResponse = read_json(response).await;
    assert_eq!(granted.admins, vec!["alice".to_owned(), "bob".to_owned()]);

    // The grant survives restart as advisory metadata, independent of commit
    // acceptance.
    let app = persistent_app(&db_path);
    // Admins may revoke other admins, but never the last one.
    let response = post_json(
        app.clone(),
        "/rooms/admins",
        &UpdateRoomAdminsRequest {
            room_id: room_id.clone(),
            sender: bob.clone(),
            grant: None,
            revoke: Some("alice".to_owned()),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let revoked: UpdateRoomAdminsResponse = read_json(response).await;
    assert_eq!(revoked.admins, vec!["bob".to_owned()]);
    let response = post_json(
        app.clone(),
        "/rooms/admins",
        &UpdateRoomAdminsRequest {
            room_id: room_id.clone(),
            sender: bob.clone(),
            grant: None,
            revoke: Some("bob".to_owned()),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "invalid_admin_change");
}

#[tokio::test]
async fn sqlite_leave_room_closes_account_and_later_removal_commit_completes_it() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-laptop");
    let room_id = "room-leave".to_owned();
    let mls_group_id = "mls-leave".to_owned();

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let add_bob = submit_add_device_request_at_epoch(&room_id, &mls_group_id, &alice, &bob, 0);
    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    let _add_accepted: CommitAccepted = read_json(response).await;

    // Activate bob.
    let bob_recipient = member_for_device(&bob);
    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: bob_recipient.clone(),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claims: Vec<HttpClaimedWelcome> = read_json(response).await;
    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: claims[0].message.id.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // Alice sends one more message bob can see before leaving.
    let pre_leave = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        1,
        b"before bob leaves",
        "leave-pre-message",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&pre_leave)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let pre_accepted: EventAccepted = read_json(response).await;

    // Bob leaves (whole-account, server-recognized immediately).
    let response = post_json(
        app.clone(),
        "/rooms/leave",
        &LeaveRoomRequest {
            room_id: room_id.clone(),
            sender: bob.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let left: LeaveRoomResponse = read_json(response).await;
    assert!(left.left);
    assert_eq!(left.departed_at_seq, pre_accepted.seq);

    // The leave is idempotent and survives restart.
    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/rooms/leave",
        &LeaveRoomRequest {
            room_id: room_id.clone(),
            sender: bob.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let replay: LeaveRoomResponse = read_json(response).await;
    assert!(!replay.left);

    // Departed senders cannot send.
    let post_leave_send = append_application_request(
        &room_id,
        &mls_group_id,
        &bob,
        1,
        b"after leaving",
        "leave-post-message",
    );
    let response = post_json(
        app.clone(),
        "/events",
        &typed_event_request(&post_leave_send),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Later traffic is hidden from the departed account, but history through
    // the leave seq stays syncable.
    let alice_post = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        1,
        b"after bob left",
        "leave-alice-post",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&alice_post)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 10,
            requester: Some(member_for_device(&bob)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bob_page: HttpSyncPage = read_json(response).await;
    assert!(
        bob_page
            .entries
            .iter()
            .all(|entry| entry.seq <= left.departed_at_seq)
    );

    // Bob's directory no longer lists the room.
    let response = post_json(
        app.clone(),
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: "bob".to_owned(),
            after_room_id: None,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert!(page.rooms.is_empty());

    // The admin's later MLS removal commit for the departed device is
    // accepted and completes the leave.
    let remove_bob =
        submit_remove_device_request(&room_id, &mls_group_id, &alice, &bob, 1, "leave-remove-bob");
    let response = post_json(app.clone(), "/commits", &remove_bob).await;
    assert_eq!(response.status(), StatusCode::OK);

    // The last admin cannot leave while members remain.
    let response = post_json(
        app.clone(),
        "/rooms/leave",
        &LeaveRoomRequest {
            room_id: room_id.clone(),
            sender: alice.clone(),
        },
    )
    .await;
    // Bob is fully removed now, so alice (sole member) may leave.
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn sqlite_bootstrap_rejects_unsupported_protocol_version_and_defaults_to_v1() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");

    let app = persistent_app(&db_path);
    // A future protocol version is refused with 426 before any side effects.
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: "room-protocol-future".to_owned(),
            mls_group_id: "mls-protocol-future".to_owned(),
            creator: alice.clone(),
            protocol: RoomProtocol {
                protocol_version: 999,
                required_capabilities: Vec::new(),
            },
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    let error: ErrorResponse = read_json(response).await;
    assert_eq!(error.kind, "unsupported_protocol_version");

    // Omitted protocol fields default to v1 on the wire (serde default), and
    // explicit v1 with capabilities is stored.
    let body = serde_json::json!({
        "room_id": "room-protocol-default",
        "mls_group_id": "mls-protocol-default",
        "creator": {"account_id": "alice", "device_id": "alice-laptop"},
    });
    let response = post_json(app.clone(), "/account-rooms/bootstrap", &body).await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: "room-protocol-caps".to_owned(),
            mls_group_id: "mls-protocol-caps".to_owned(),
            creator: alice.clone(),
            protocol: RoomProtocol {
                protocol_version: 1,
                required_capabilities: vec!["streams.v1".to_owned()],
            },
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // Both rooms replay idempotently after restart.
    let app = persistent_app(&db_path);
    let response = post_json(app.clone(), "/account-rooms/bootstrap", &body).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn sqlite_state_snapshot_boots_without_full_history_replay() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let room_id = "room-snapshot".to_owned();
    let mls_group_id = "mls-snapshot".to_owned();

    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let mut last_seq = 0;
    for index in 0..5 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            format!("snapshot message {index}").as_bytes(),
            &format!("snapshot-msg-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: EventAccepted = read_json(response).await;
        last_seq = accepted.seq;
    }

    state.snapshot_now().expect("snapshot");

    // Two more events form the tail the snapshot does not cover.
    for index in 5..7 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            format!("snapshot message {index}").as_bytes(),
            &format!("snapshot-msg-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: EventAccepted = read_json(response).await;
        last_seq = accepted.seq;
    }
    drop(app);
    drop(state);

    // Prove the snapshot is authoritative for its prefix: delete every
    // operation the snapshot covers (a preview of horizon compaction) and
    // the reopened server must still serve the complete ordered log.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open raw");
        let snapshot_seq: i64 = conn
            .query_row(
                "SELECT last_op_seq FROM http_state_snapshots WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("snapshot row");
        assert!(snapshot_seq > 0);
        conn.execute(
            "DELETE FROM http_delivery_ops WHERE seq <= ?1",
            rusqlite::params![snapshot_seq],
        )
        .expect("compact prefix");
    }

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 50,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), last_seq as usize);
    assert_eq!(page.next_after_seq, last_seq);

    // Idempotent replay of a pre-snapshot event still works.
    let replay = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        b"snapshot message 0",
        "snapshot-msg-0",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&replay)).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn sqlite_push_tokens_register_survive_restart_and_drop_on_revocation() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-phone");
    let bob = DeviceRef::new("bob", "bob-phone");

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/push-tokens",
        &RegisterPushTokenRequest {
            device: alice.clone(),
            platform: PushPlatform::Apns,
            token: "apns-token-alice".to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = post_json(
        app.clone(),
        "/push-tokens",
        &RegisterPushTokenRequest {
            device: bob.clone(),
            platform: PushPlatform::Fcm,
            token: "fcm-token-bob".to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // Replacement is an upsert; removal is idempotent; both survive restart.
    let response = post_json(
        app.clone(),
        "/push-tokens",
        &RegisterPushTokenRequest {
            device: alice.clone(),
            platform: PushPlatform::Apns,
            token: "apns-token-alice-2".to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/push-tokens/remove",
        &RemovePushTokenRequest {
            device: alice.clone(),
            token: Some("apns-token-alice".to_owned()),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let removed: RemovePushTokenResponse = read_json(response).await;
    assert!(
        !removed.removed,
        "stale token guard must not remove a rotated push token"
    );
    let response = post_json(
        app.clone(),
        "/push-tokens/remove",
        &RemovePushTokenRequest {
            device: alice.clone(),
            token: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let removed: RemovePushTokenResponse = read_json(response).await;
    assert!(removed.removed);
    let response = post_json(
        app.clone(),
        "/push-tokens/remove",
        &RemovePushTokenRequest {
            device: alice.clone(),
            token: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let removed: RemovePushTokenResponse = read_json(response).await;
    assert!(!removed.removed);

    // Revoking a device drops its token, and a revoked device cannot
    // re-register.
    let response = post_json(
        app.clone(),
        "/devices/revoke",
        &RevokeDeviceRequest {
            device: bob.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/push-tokens/remove",
        &RemovePushTokenRequest {
            device: bob.clone(),
            token: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let removed: RemovePushTokenResponse = read_json(response).await;
    assert!(!removed.removed, "revocation already dropped bob's token");
    let response = post_json(
        app.clone(),
        "/push-tokens",
        &RegisterPushTokenRequest {
            device: bob,
            platform: PushPlatform::Fcm,
            token: "fcm-token-bob-again".to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn sqlite_push_wakes_claim_opaque_payload_and_ack() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-phone");
    let bob = DeviceRef::new("bob", "bob-phone");
    let room_id = "room-push-wake-claim".to_owned();
    let mls_group_id = "mls-push-wake-claim".to_owned();
    let secret_text = "plaintext message body must not enter push payload";

    let app = persistent_app(&db_path);
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    add_device_to_room(
        &app,
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-push-wake-bob",
        "commit-push-wake-bob",
    )
    .await;
    register_push_token(&app, &alice, PushPlatform::Apns, "apns-token-alice").await;
    register_push_token(&app, &bob, PushPlatform::Apns, "apns-token-bob").await;

    let message = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        1,
        secret_text.as_bytes(),
        "push-wake-message",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&message)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: EventAccepted = read_json(response).await;

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/push-wakes/claim",
        &ClaimPushWakesRequest {
            now_ms: 1_000,
            lease_ms: 30_000,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: ClaimPushWakesResponse = read_json(response).await;
    assert_eq!(claimed.wakes.len(), 1);
    let wake = &claimed.wakes[0];
    assert_eq!(wake.payload.room_id, room_id);
    assert_eq!(wake.payload.seq, accepted.seq);
    assert_eq!(wake.attempt, 1);
    assert_eq!(wake.tokens.len(), 1);
    assert_eq!(wake.tokens[0].device, bob);
    assert_eq!(wake.tokens[0].token, "apns-token-bob");
    let claim_json = serde_json::to_string(&claimed).expect("claim json");
    assert!(!claim_json.contains(secret_text));
    assert!(!claim_json.contains("sender"));
    assert!(!claim_json.contains("attachment"));

    let response = post_json(
        app.clone(),
        "/push-wakes/ack",
        &AckPushWakeRequest {
            wake_id: wake.wake_id.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let ack: AckPushWakeResponse = read_json(response).await;
    assert!(ack.acked);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/push-wakes/claim",
        &ClaimPushWakesRequest {
            now_ms: 2_000,
            lease_ms: 30_000,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: ClaimPushWakesResponse = read_json(response).await;
    assert!(claimed.wakes.is_empty());

    let response = post_json(app.clone(), "/events", &typed_event_request(&message)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let replayed: EventAccepted = read_json(response).await;
    assert_eq!(replayed, accepted);
    let response = post_json(
        app,
        "/push-wakes/claim",
        &ClaimPushWakesRequest {
            now_ms: 3_000,
            lease_ms: 30_000,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: ClaimPushWakesResponse = read_json(response).await;
    assert!(
        claimed.wakes.is_empty(),
        "idempotent event replay must not recreate an acked wake"
    );
}

#[tokio::test]
async fn sqlite_push_wake_fail_retries_then_drops_after_attempt_bound() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-phone");
    let bob = DeviceRef::new("bob", "bob-phone");
    let room_id = "room-push-wake-fail".to_owned();
    let mls_group_id = "mls-push-wake-fail".to_owned();

    let app = persistent_app(&db_path);
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    add_device_to_room(
        &app,
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-push-wake-fail-bob",
        "commit-push-wake-fail-bob",
    )
    .await;
    register_push_token(&app, &bob, PushPlatform::Apns, "apns-token-bob").await;
    let message = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        1,
        b"retry-bounded-push-wake",
        "push-wake-fail-message",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&message)).await;
    assert_eq!(response.status(), StatusCode::OK);

    let mut wake_id = None;
    for attempt in 1..=5 {
        let app = persistent_app(&db_path);
        let response = post_json(
            app.clone(),
            "/push-wakes/claim",
            &ClaimPushWakesRequest {
                now_ms: 1_000 + u64::from(attempt),
                lease_ms: 30_000,
                limit: 10,
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let claimed: ClaimPushWakesResponse = read_json(response).await;
        assert_eq!(claimed.wakes.len(), 1, "attempt {attempt}");
        let wake = &claimed.wakes[0];
        assert_eq!(wake.attempt, attempt);
        wake_id = Some(wake.wake_id.clone());

        let response = post_json(
            app,
            "/push-wakes/fail",
            &FailPushWakeRequest {
                wake_id: wake.wake_id.clone(),
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let failed: FailPushWakeResponse = read_json(response).await;
        if attempt < 5 {
            assert!(failed.retry, "attempt {attempt}");
            assert!(!failed.dropped, "attempt {attempt}");
        } else {
            assert!(!failed.retry);
            assert!(failed.dropped);
        }
    }

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/push-wakes/claim",
        &ClaimPushWakesRequest {
            now_ms: 10_000,
            lease_ms: 30_000,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: ClaimPushWakesResponse = read_json(response).await;
    assert!(claimed.wakes.is_empty());

    let response = post_json(
        app,
        "/push-wakes/ack",
        &AckPushWakeRequest {
            wake_id: wake_id.expect("wake id"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let ack: AckPushWakeResponse = read_json(response).await;
    assert!(!ack.acked, "dropped wake is already gone");
}

async fn bootstrap_room(app: &Router, room_id: &str, mls_group_id: &str, creator: &DeviceRef) {
    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.to_owned(),
            mls_group_id: mls_group_id.to_owned(),
            creator: creator.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

async fn add_device_to_room(
    app: &Router,
    room_id: &str,
    mls_group_id: &str,
    sender: &DeviceRef,
    added: &DeviceRef,
    welcome_id: &str,
    idempotency_key: &str,
) -> CommitAccepted {
    let request = submit_add_device_request(
        room_id,
        mls_group_id,
        sender,
        added,
        welcome_id,
        idempotency_key,
    );
    publish_and_claim_key_package_for_add(app, &request).await;
    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted = read_json(response).await;
    let response = post_json(
        app.clone(),
        "/welcomes/claim",
        &ClaimWelcomesRequest {
            recipient: member_for_device(added),
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].message.id, id(welcome_id));
    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id(welcome_id),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let ack: AckWelcomeResponse = read_json(response).await;
    assert!(ack.acked);
    accepted
}

async fn register_push_token(
    app: &Router,
    device: &DeviceRef,
    platform: PushPlatform,
    token: &str,
) {
    let response = post_json(
        app.clone(),
        "/push-tokens",
        &RegisterPushTokenRequest {
            device: device.clone(),
            platform,
            token: token.to_owned(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

fn submit_add_device_request(
    room_id: &str,
    mls_group_id: &str,
    sender: &DeviceRef,
    added: &DeviceRef,
    welcome_id: &str,
    idempotency_key: &str,
) -> SubmitCommitRequest {
    let envelope = FiniteEnvelope {
        room_id: room_id.to_owned(),
        mls_group_id: mls_group_id.to_owned(),
        epoch: 0,
        sender: sender.clone(),
        kind: LogEntryKind::Commit,
        payload: b"commit-add-device".to_vec(),
    };
    let commit_message_id = envelope.message_id().expect("commit message id");
    let key_package_id = format!("key-package-{welcome_id}");
    SubmitCommitRequest {
        room_id: room_id.to_owned(),
        sender: sender.clone(),
        expected_epoch: 0,
        envelope,
        membership_delta: MembershipDeltaV1 {
            base_epoch: 0,
            post_commit_epoch: 1,
            commit_message_id,
            adds: vec![MembershipAddV1 {
                device: added.clone(),
                key_package_id: key_package_id.clone(),
                key_package_ref: format!("key-package-ref-{welcome_id}"),
                key_package_hash: format!("key-package-hash-{welcome_id}"),
                welcome_id: welcome_id.to_owned(),
            }],
            removes: Vec::new(),
        },
        staged_welcomes: vec![StagedWelcomeV1 {
            welcome_id: welcome_id.to_owned(),
            welcome_payload: b"welcome-add-device".to_vec(),
            ratchet_tree_payload: b"ratchet-tree-add-device".to_vec(),
        }],
        idempotency_key: idempotency_key.to_owned(),
    }
}

fn submit_remove_device_request(
    room_id: &str,
    mls_group_id: &str,
    sender: &DeviceRef,
    removed: &DeviceRef,
    epoch: u64,
    idempotency_key: &str,
) -> SubmitCommitRequest {
    let envelope = FiniteEnvelope {
        room_id: room_id.to_owned(),
        mls_group_id: mls_group_id.to_owned(),
        epoch,
        sender: sender.clone(),
        kind: LogEntryKind::Commit,
        payload: format!("commit-remove-{idempotency_key}").into_bytes(),
    };
    let commit_message_id = envelope.message_id().expect("commit message id");
    SubmitCommitRequest {
        room_id: room_id.to_owned(),
        sender: sender.clone(),
        expected_epoch: epoch,
        envelope,
        membership_delta: MembershipDeltaV1 {
            base_epoch: epoch,
            post_commit_epoch: epoch + 1,
            commit_message_id,
            adds: Vec::new(),
            removes: vec![MembershipRemoveV1 {
                device: removed.clone(),
                removed_leaf_index: 1,
            }],
        },
        staged_welcomes: Vec::new(),
        idempotency_key: idempotency_key.to_owned(),
    }
}

fn submit_add_device_request_at_epoch(
    room_id: &str,
    mls_group_id: &str,
    sender: &DeviceRef,
    added: &DeviceRef,
    epoch: u64,
) -> SubmitCommitRequest {
    let welcome_id = format!("welcome-{room_id}-{epoch}");
    submit_add_device_request_at_epoch_with_ids(
        room_id,
        mls_group_id,
        sender,
        added,
        epoch,
        &welcome_id,
        &format!("commit-{room_id}-{epoch}"),
    )
}

fn submit_add_device_request_at_epoch_with_ids(
    room_id: &str,
    mls_group_id: &str,
    sender: &DeviceRef,
    added: &DeviceRef,
    epoch: u64,
    welcome_id: &str,
    idempotency_key: &str,
) -> SubmitCommitRequest {
    let mut request = submit_add_device_request(
        room_id,
        mls_group_id,
        sender,
        added,
        welcome_id,
        idempotency_key,
    );
    request.expected_epoch = epoch;
    request.envelope.epoch = epoch;
    let commit_message_id = request.envelope.message_id().expect("commit message id");
    request.membership_delta.base_epoch = epoch;
    request.membership_delta.post_commit_epoch = epoch + 1;
    request.membership_delta.commit_message_id = commit_message_id;
    request
}

async fn publish_and_claim_key_package_for_add(app: &Router, request: &SubmitCommitRequest) {
    let add = request
        .membership_delta
        .adds
        .first()
        .expect("add-device request has one add");
    let upload = UploadKeyPackageRequest {
        key_package_id: add.key_package_id.clone(),
        owner: add.device.clone(),
        key_package_ref: add.key_package_ref.clone(),
        key_package_hash: add.key_package_hash.clone(),
        key_package_payload: format!("payload-{}", add.key_package_id).into_bytes(),
    };
    let publication = HttpKeyPackagePublication {
        key_package_id: HttpKeyPackageId::new(upload.key_package_id.as_bytes().to_vec()),
        owner: member_for_device(&upload.owner),
        key_package: KeyPackage::new(serde_json::to_vec(&upload).expect("upload json")),
    };
    let response = post_json(app.clone(), "/key-packages", &publication).await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app.clone(),
        "/key-packages/claim",
        &ClaimKeyPackageRequest {
            owner: member_for_device(&upload.owner),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: Option<HttpClaimedKeyPackage> = read_json(response).await;
    let claimed = claimed.expect("claimed KeyPackage");
    assert_eq!(claimed.key_package_id, publication.key_package_id);
    assert_eq!(claimed.owner, publication.owner);
}

async fn key_package_inventory_for_device(
    app: &Router,
    owner: &DeviceRef,
) -> HttpKeyPackageInventory {
    let response = post_json(
        app.clone(),
        "/key-packages/inventory",
        &KeyPackageInventoryRequest {
            owner: member_for_device(owner),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    read_json(response).await
}

async fn assert_submit_commit_had_no_side_effects(app: &Router, room_id: &str, added: &DeviceRef) {
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());

    let response = post_json(
        app.clone(),
        "/sync/inbox",
        &InboxSyncRequest {
            recipient: member_for_device(added),
            after_seq: 0,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert!(page.entries.is_empty());
}

#[derive(Clone, Copy, Debug)]
enum HttpSubmitCommitCrashPoint {
    CommitDeliveryOperation,
    CommitIdempotencyRecord,
    WelcomeDeliveryOperation,
    WelcomeIdempotencyRecord,
    AccountRoomProjection,
    RoomMembershipProjection,
    KeyPackageConsumedProjection,
}

impl HttpSubmitCommitCrashPoint {
    const ALL: [Self; 7] = [
        Self::CommitDeliveryOperation,
        Self::CommitIdempotencyRecord,
        Self::WelcomeDeliveryOperation,
        Self::WelcomeIdempotencyRecord,
        Self::AccountRoomProjection,
        Self::RoomMembershipProjection,
        Self::KeyPackageConsumedProjection,
    ];

    fn trigger_sql(self) -> &'static str {
        match self {
            Self::CommitDeliveryOperation => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_commit_delivery
                AFTER INSERT ON http_delivery_ops
                WHEN NEW.kind = 'publish_message'
                  AND NEW.body_json LIKE '%http-crash-matrix-commit%'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after commit delivery');
                END;
                "#
            }
            Self::CommitIdempotencyRecord => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_commit_idempotency
                AFTER INSERT ON http_publish_idempotency
                WHEN NEW.idempotency_key = 'commit:room-http-crash-matrix:http-crash-matrix-commit'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after commit idempotency');
                END;
                "#
            }
            Self::WelcomeDeliveryOperation => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_welcome_delivery
                AFTER INSERT ON http_delivery_ops
                WHEN NEW.kind = 'publish_message'
                  AND NEW.body_json LIKE '%welcome-http-crash-tablet%'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after welcome delivery');
                END;
                "#
            }
            Self::WelcomeIdempotencyRecord => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_welcome_idempotency
                AFTER INSERT ON http_publish_idempotency
                WHEN NEW.idempotency_key = 'welcome:welcome-http-crash-tablet'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after welcome idempotency');
                END;
                "#
            }
            Self::AccountRoomProjection => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_account_room_projection
                AFTER UPDATE OF record_json ON http_account_rooms
                WHEN NEW.room_id = 'room-http-crash-matrix'
                  AND NEW.record_json LIKE '%alice-tablet%'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after account-room projection');
                END;
                "#
            }
            Self::RoomMembershipProjection => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_room_membership_projection
                AFTER UPDATE OF projection_json ON http_room_memberships
                WHEN NEW.room_id = 'room-http-crash-matrix'
                  AND NEW.projection_json LIKE '%alice-tablet%'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after room-membership projection');
                END;
                "#
            }
            Self::KeyPackageConsumedProjection => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_key_package_consumed
                AFTER UPDATE OF state_json ON http_key_package_inventory
                WHEN NEW.state_json = '"Consumed"'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after KeyPackage consumed projection');
                END;
                "#
            }
        }
    }
}

fn install_http_submit_commit_crash_trigger(
    db_path: &std::path::Path,
    point: HttpSubmitCommitCrashPoint,
) {
    clear_http_submit_commit_crash_triggers(db_path);
    let conn = Connection::open(db_path).expect("sqlite connection");
    conn.execute_batch(point.trigger_sql())
        .expect("install HTTP commit crash trigger");
}

fn clear_http_submit_commit_crash_triggers(db_path: &std::path::Path) {
    let conn = Connection::open(db_path).expect("sqlite connection");
    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_commit_delivery;
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_commit_idempotency;
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_welcome_delivery;
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_welcome_idempotency;
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_account_room_projection;
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_room_membership_projection;
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_key_package_consumed;
        "#,
    )
    .expect("clear HTTP commit crash triggers");
}

#[derive(Clone, Copy, Debug)]
enum HttpApplicationEventCrashPoint {
    EventDeliveryOperation,
    EventIdempotencyRecord,
    RoomMembershipProjection,
    ApplicationEffectProjection,
}

impl HttpApplicationEventCrashPoint {
    const ALL: [Self; 4] = [
        Self::EventDeliveryOperation,
        Self::EventIdempotencyRecord,
        Self::RoomMembershipProjection,
        Self::ApplicationEffectProjection,
    ];

    fn trigger_sql(self) -> &'static str {
        match self {
            Self::EventDeliveryOperation => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_application_event_delivery
                AFTER INSERT ON http_delivery_ops
                WHEN NEW.kind = 'publish_message'
                  AND NEW.body_json LIKE '%application-effect-crash%'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after application-event delivery');
                END;
                "#
            }
            Self::EventIdempotencyRecord => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_application_event_idempotency
                AFTER INSERT ON http_publish_idempotency
                WHEN NEW.idempotency_key = 'event:room-application-effect-crash:application-effect-crash'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after application-event idempotency');
                END;
                "#
            }
            Self::RoomMembershipProjection => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_application_event_room_membership
                AFTER UPDATE OF projection_json ON http_room_memberships
                WHEN NEW.room_id = 'room-application-effect-crash'
                  AND NEW.projection_json LIKE '%"last_seq":1%'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after application-event room membership');
                END;
                "#
            }
            Self::ApplicationEffectProjection => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_application_effect_projection
                AFTER INSERT ON http_application_delivery_effects
                WHEN NEW.room_id = 'room-application-effect-crash'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after application-effect projection');
                END;
                "#
            }
        }
    }
}

fn install_http_application_event_crash_trigger(
    db_path: &std::path::Path,
    point: HttpApplicationEventCrashPoint,
) {
    clear_http_application_event_crash_triggers(db_path);
    let conn = Connection::open(db_path).expect("sqlite connection");
    conn.execute_batch(point.trigger_sql())
        .expect("install HTTP application-event crash trigger");
}

fn clear_http_application_event_crash_triggers(db_path: &std::path::Path) {
    let conn = Connection::open(db_path).expect("sqlite connection");
    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_application_event_delivery;
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_application_event_idempotency;
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_application_event_room_membership;
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_application_effect_projection;
        "#,
    )
    .expect("clear HTTP application-event crash triggers");
}

async fn assert_http_crash_commit_rolled_back(
    app: &Router,
    room_id: &str,
    tablet: &DeviceRef,
    first_seq: u64,
) {
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].seq, first_seq);

    let response = post_json(
        app.clone(),
        "/sync/inbox",
        &InboxSyncRequest {
            recipient: member_for_device(tablet),
            after_seq: 0,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let inbox_page: HttpSyncPage = read_json(response).await;
    assert!(inbox_page.entries.is_empty());

    let page = account_room_page(app, "alice").await;
    assert_eq!(page.rooms.len(), 1);
    assert_eq!(page.rooms[0]["current_epoch"], 1);
    assert_eq!(page.rooms[0]["last_seq"], first_seq);
    assert!(
        !page.rooms[0]["devices"]
            .as_array()
            .expect("devices")
            .iter()
            .any(|device| device["device"]["device_id"] == "alice-tablet")
    );

    let inventory = key_package_inventory_for_device(app, tablet).await;
    assert_eq!(inventory.available, 0);
    assert_eq!(inventory.claimed, 1);
}

async fn assert_http_crash_commit_converged(
    app: &Router,
    room_id: &str,
    tablet: &DeviceRef,
    accepted_seq: u64,
) {
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(room_id),
            after_seq: 0,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[1].seq, accepted_seq);

    let response = post_json(
        app.clone(),
        "/sync/inbox",
        &InboxSyncRequest {
            recipient: member_for_device(tablet),
            after_seq: 0,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let inbox_page: HttpSyncPage = read_json(response).await;
    assert_eq!(inbox_page.entries.len(), 1);
    assert_eq!(
        inbox_page.entries[0].message.id,
        id("welcome-http-crash-tablet")
    );

    let page = account_room_page(app, "alice").await;
    assert_eq!(page.rooms.len(), 1);
    assert_eq!(page.rooms[0]["current_epoch"], 2);
    assert_eq!(page.rooms[0]["last_seq"], accepted_seq);
    assert!(
        page.rooms[0]["devices"]
            .as_array()
            .expect("devices")
            .iter()
            .any(|device| device["device"]["device_id"] == "alice-tablet")
    );

    let inventory = key_package_inventory_for_device(app, tablet).await;
    assert_eq!(inventory.available, 0);
    assert_eq!(inventory.claimed, 0);
}

async fn account_room_page(app: &Router, account_id: &str) -> ListAccountRoomDirectoryResponse {
    let response = post_json(
        app.clone(),
        "/account-rooms/list",
        &ListAccountRoomDirectoryRequest {
            account_id: account_id.to_owned(),
            after_room_id: None,
            limit: 10,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    read_json(response).await
}

fn account_room_device_active(page: &ListAccountRoomDirectoryResponse, device: &DeviceRef) -> bool {
    page.rooms
        .iter()
        .flat_map(|room| {
            room["devices"]
                .as_array()
                .expect("account room devices")
                .iter()
        })
        .find(|entry| {
            entry["device"]["account_id"] == device.account_id
                && entry["device"]["device_id"] == device.device_id
        })
        .unwrap_or_else(|| panic!("missing account room device: {device:?}"))["active"]
        .as_bool()
        .expect("active flag")
}

fn commit_publish_request_for_test(
    request: &SubmitCommitRequest,
    message_id: &str,
) -> PublishMessageRequest {
    let transport_group_id = request.room_id.as_bytes().to_vec();
    let entry = finitechat_proto::RoomLogEntry {
        room_id: request.room_id.clone(),
        seq: 0,
        message_id: message_id.to_owned(),
        sender: request.sender.clone(),
        kind: LogEntryKind::Commit,
        epoch: request.expected_epoch,
        envelope: request.envelope.clone(),
        idempotency_key: request.idempotency_key.clone(),
        timestamp_unix_seconds: 0,
    };
    let payload = serde_json::to_vec(&FiniteAccountRoomCommitProjection {
        entry,
        membership_delta: request.membership_delta.clone(),
    })
    .expect("commit projection payload");

    PublishMessageRequest {
        target: group_target(
            group_id(&request.room_id),
            transport_group_id.clone(),
            Some(HttpCommitAdmission {
                source_epoch: EpochId(request.expected_epoch),
            }),
        ),
        message: TransportMessage {
            id: id(message_id),
            payload,
            timestamp: Timestamp(0),
            causal_deps: Vec::new(),
            source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
            envelope: TransportEnvelope::GroupMessage { transport_group_id },
        },
        idempotency_key: Some(format!(
            "commit:{}:{}",
            request.room_id, request.idempotency_key
        )),
    }
}

fn insert_durable_commit_publish_without_projection(
    db_path: &std::path::Path,
    request: &PublishMessageRequest,
    seq: u64,
) {
    let operation_json = serde_json::to_string(&serde_json::json!({
        "PublishMessage": {
            "target": &request.target,
            "message": &request.message,
            "idempotency_key": &request.idempotency_key,
        }
    }))
    .expect("persisted operation json");
    let fingerprint_json = serde_json::to_string(&serde_json::json!({
        "target": &request.target,
        "message": &request.message,
    }))
    .expect("publish fingerprint json");
    let receipt_json = serde_json::to_string(&HttpPublishReceipt {
        message_id: request.message.id.clone(),
        plane: HttpDeliveryPlane::Group,
        seq,
        duplicate: false,
    })
    .expect("publish receipt json");
    let idempotency_key = request
        .idempotency_key
        .as_deref()
        .expect("commit publish idempotency key");

    let conn = Connection::open(db_path).expect("sqlite connection");
    conn.execute(
        "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
        params!["publish_message", operation_json],
    )
    .expect("insert durable publish operation");
    conn.execute(
        "INSERT INTO http_publish_idempotency (
            idempotency_key,
            fingerprint_json,
            receipt_json
        ) VALUES (?1, ?2, ?3)",
        params![idempotency_key, fingerprint_json, receipt_json],
    )
    .expect("insert durable publish idempotency");
}

fn append_application_request(
    room_id: &str,
    mls_group_id: &str,
    sender: &DeviceRef,
    epoch: u64,
    payload: &[u8],
    idempotency_key: &str,
) -> AppendEventRequest {
    AppendEventRequest {
        room_id: room_id.to_owned(),
        sender: sender.clone(),
        envelope: FiniteEnvelope {
            room_id: room_id.to_owned(),
            mls_group_id: mls_group_id.to_owned(),
            epoch,
            sender: sender.clone(),
            kind: LogEntryKind::Application,
            payload: payload.to_vec(),
        },
        idempotency_key: idempotency_key.to_owned(),
        timestamp_unix_seconds: 1_700_000_000,
    }
}

fn ephemeral_activity_request(
    room_id: &str,
    mls_group_id: &str,
    sender: &DeviceRef,
    epoch: u64,
    conversation_id: Option<&str>,
    received_at_ms: u64,
) -> AppendEphemeralActivityRequest {
    AppendEphemeralActivityRequest {
        room_id: room_id.to_owned(),
        mls_group_id: mls_group_id.to_owned(),
        epoch,
        sender: sender.clone(),
        conversation_id: conversation_id.map(str::to_owned),
        payload: format!("activity-{}-{received_at_ms}", sender.device_id).into_bytes(),
        received_at_ms,
        expires_at_ms: received_at_ms + 1_000,
    }
}

fn key_package_publication(
    key_package_id: &str,
    owner: MemberId,
    bytes: &[u8],
) -> HttpKeyPackagePublication {
    HttpKeyPackagePublication {
        key_package_id: HttpKeyPackageId::new(key_package_id.as_bytes().to_vec()),
        owner,
        key_package: KeyPackage::new(bytes.to_vec()),
    }
}

fn finite_key_package_publication(
    owner: &DeviceRef,
    key_package_id: &str,
    key_package_ref: &str,
    key_package_hash: &str,
    payload: &[u8],
) -> HttpKeyPackagePublication {
    let upload = UploadKeyPackageRequest {
        key_package_id: key_package_id.to_owned(),
        owner: owner.clone(),
        key_package_ref: key_package_ref.to_owned(),
        key_package_hash: key_package_hash.to_owned(),
        key_package_payload: payload.to_vec(),
    };
    HttpKeyPackagePublication {
        key_package_id: HttpKeyPackageId::new(key_package_id.as_bytes().to_vec()),
        owner: member_for_device(owner),
        key_package: KeyPackage::new(serde_json::to_vec(&upload).expect("upload json")),
    }
}

fn group_target(
    group_id: GroupId,
    transport_group_id: Vec<u8>,
    commit_admission: Option<HttpCommitAdmission>,
) -> HttpPublishTarget {
    HttpPublishTarget::Group {
        group_id,
        transport_group_id,
        commit_admission,
    }
}

fn group_message(
    message_id: &str,
    transport_group_id: Vec<u8>,
    payload: &[u8],
) -> TransportMessage {
    TransportMessage {
        id: id(message_id),
        payload: payload.to_vec(),
        timestamp: Timestamp(42),
        causal_deps: Vec::new(),
        source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
        envelope: TransportEnvelope::GroupMessage { transport_group_id },
    }
}

fn welcome_message(message_id: &str, recipient: MemberId, payload: &[u8]) -> TransportMessage {
    TransportMessage {
        id: id(message_id),
        payload: payload.to_vec(),
        timestamp: Timestamp(43),
        causal_deps: Vec::new(),
        source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
        envelope: TransportEnvelope::Welcome { recipient },
    }
}

#[tokio::test]
async fn sqlite_sync_wait_wakes_on_room_publish() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-agent");
    let bob = DeviceRef::new("bob", "bob-phone");
    let room_id = "room-sync-wait".to_owned();
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: "mls-sync-wait".to_owned(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // No news: a short wait times out.
    let started = std::time::Instant::now();
    let response = post_json(
        app.clone(),
        "/sync/wait",
        &SyncWaitRequest {
            rooms: vec![SyncWaitRoom {
                room_id: room_id.clone(),
                after_seq: 0,
            }],
            wait_ms: 120,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let waited: SyncWaitResponse = read_json(response).await;
    assert!(!waited.woke);
    assert!(started.elapsed() >= std::time::Duration::from_millis(100));

    // A commit advances the room: an armed waiter wakes promptly and a
    // fresh waiter returns immediately.
    let add_bob = submit_add_device_request_at_epoch(&room_id, "mls-sync-wait", &alice, &bob, 0);
    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let waiter_app = app.clone();
    let waiter_room = room_id.clone();
    let waiter = tokio::spawn(async move {
        let started = std::time::Instant::now();
        let response = post_json(
            waiter_app,
            "/sync/wait",
            &SyncWaitRequest {
                rooms: vec![SyncWaitRoom {
                    room_id: waiter_room,
                    after_seq: 0,
                }],
                wait_ms: 10_000,
            },
        )
        .await;
        (
            read_json::<SyncWaitResponse>(response).await,
            started.elapsed(),
        )
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    let (woke, elapsed) = waiter.await.expect("waiter");
    assert!(woke.woke);
    assert_eq!(woke.reason.as_deref(), Some("room:room-sync-wait"));
    assert!(elapsed < std::time::Duration::from_secs(5));

    let response = post_json(
        app.clone(),
        "/sync/wait",
        &SyncWaitRequest {
            rooms: vec![SyncWaitRoom {
                room_id: room_id.clone(),
                after_seq: 0,
            }],
            wait_ms: 10_000,
        },
    )
    .await;
    let waited: SyncWaitResponse = read_json(response).await;
    assert!(waited.woke);
}

#[tokio::test]
async fn sqlite_sync_stream_emits_coalesced_high_watermark_hints() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-agent");
    let room_id = "room-sync-stream".to_owned();
    let mls_group_id = "mls-sync-stream".to_owned();
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app.clone(),
        "/sync/stream",
        &SyncStreamRequest {
            rooms: vec![SyncWaitRoom {
                room_id: room_id.clone(),
                after_seq: 0,
            }],
            inbox: None,
            heartbeat_ms: Some(60_000),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
    );
    let mut stream = response.into_body().into_data_stream();

    for index in 0..2 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            format!("stream payload {index}").as_bytes(),
            &format!("sync-stream-event-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    assert_eq!(
        read_next_sync_hint(&mut stream).await,
        SyncHintEvent::RoomAdvanced {
            room_id: room_id.clone(),
            seq: 2,
        }
    );
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(room_id.as_str()),
            after_seq: 0,
            limit: 100,
            requester: None,
        },
    )
    .await;
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 2);

    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        b"stream payload 2",
        "sync-stream-event-2",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        read_next_sync_hint(&mut stream).await,
        SyncHintEvent::RoomAdvanced {
            room_id: room_id.clone(),
            seq: 3,
        }
    );
}

#[tokio::test]
async fn sqlite_sync_stream_wakes_zero_room_device_for_persisted_welcome() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-agent");
    let bob = DeviceRef::new("bob", "bob-new-device");
    let room_id = "room-inbox-sync-stream".to_owned();
    let mls_group_id = "mls-inbox-sync-stream".to_owned();
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/account-rooms/bootstrap",
        &BootstrapAccountRoomRequest {
            room_id: room_id.clone(),
            mls_group_id: mls_group_id.clone(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app.clone(),
        "/sync/stream",
        &SyncStreamRequest {
            rooms: Vec::new(),
            inbox: Some(SyncWaitInbox::new(delivery_member_id_for_device(&bob), 0)),
            heartbeat_ms: Some(60_000),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut armed_stream = response.into_body().into_data_stream();

    let add_bob = submit_add_device_request_at_epoch(&room_id, &mls_group_id, &alice, &bob, 0);
    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        read_next_sync_hint(&mut armed_stream).await,
        SyncHintEvent::InboxAdvanced { seq: 1 }
    );

    // Hints carry no authority and the durable inbox survives a server
    // restart. A Device that was offline for the release gets the same wake
    // from its persisted cursor and can repair through the normal sync path.
    drop(armed_stream);
    drop(app);
    let restarted = persistent_app(&db_path);
    let response = post_json(
        restarted,
        "/sync/stream",
        &SyncStreamRequest {
            rooms: Vec::new(),
            inbox: Some(SyncWaitInbox::new(delivery_member_id_for_device(&bob), 0)),
            heartbeat_ms: Some(60_000),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut restarted_stream = response.into_body().into_data_stream();
    assert_eq!(
        read_next_sync_hint(&mut restarted_stream).await,
        SyncHintEvent::InboxAdvanced { seq: 1 }
    );
}
