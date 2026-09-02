use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Method, Request, Response, StatusCode};
use finitechat_blob::BlobDescriptor;
use finitechat_delivery::{
    HTTP_SERVER_SOURCE, HttpClaimedKeyPackage, HttpCommitAdmission, HttpKeyPackageId,
    HttpKeyPackagePublication, HttpPublishTarget, HttpSyncPage, MAX_HTTP_ID_BYTES,
    MAX_HTTP_SYNC_PAGE_ENTRIES,
};
use finitechat_http::{
    AckWelcomeRequest, AckWelcomeResponse,
    ApplicationEffectCountsResponse, ApplicationEffectRequest, BootstrapAccountRoomRequest,
    BootstrapAccountRoomResponse, ClaimKeyPackageForAccountRequest, ClaimKeyPackageRequest,
    ClaimKeyPackagesRequest, ClaimWelcomesRequest,
    CreatePairingSessionRequest, DeviceLivenessRecord, ErrorResponse, ExpireKeyPackageLeaseRequest,
    ExpireKeyPackageLeaseResponse, FiniteAccountRoomCommitProjection, GetDeviceLivenessRequest, GetDeviceLivenessResponse,
    GetEphemeralActivitiesRequest, GetEphemeralActivitiesResponse,
    GetKeyPackageAvailabilityRequest, GetKeyPackageAvailabilityResponse, GetNostrProfilesRequest,
    GetNostrProfilesResponse, GetPairingSessionRequest, GroupSyncRequest,
    HttpApplicationDeliveryEffect, HttpClaimedWelcome, HttpKeyPackageClaim,
    HttpKeyPackageInventory, HttpPairingSessionRecord, HttpPairingSessionState, InboxSyncRequest,
    KeyPackageInventoryRequest, LeaveRoomRequest, LeaveRoomResponse,
    ListAccountRoomDirectoryRequest, ListAccountRoomDirectoryResponse, NostrProfileRecord,
    ObserveDeviceLivenessRequest, PublishKeyPackageResponse, PublishMessageRequest,
    PublishPairingCompleteRequest, PublishPairingOfferRequest, PublishPairingResponseRequest,
    PutNostrProfileRequest, ReportInvalidCommitRequest, ReportInvalidCommitResponse,
    RevokeDeviceRequest, SaveAccountRoomRequest, SaveAccountRoomResponse, SyncHintEvent,
    SyncStreamRequest, SyncWaitInbox, SyncWaitRequest, SyncWaitResponse, SyncWaitRoom,
    UpdateRoomAdminsRequest, UpdateRoomAdminsResponse,
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
    MAX_KEY_PACKAGES_PER_DEVICE, MembershipAddV1, MembershipDeltaV1, MembershipRemoveV1,
    PushPolicy, RoomStatus, RuntimeStateProjection, RuntimeStateProjectionEntry,
    RuntimeStateProjectionError, RuntimeStateSnapshotV1, StagedWelcomeV1, UnreadPolicy,
    WelcomeState,
};
use finitechat_server::{DurableStoreError, HttpServerState, ServerHttpError, http_router};
use finitechat_transport::engine::KeyPackage;
use finitechat_transport::transport::{
    Timestamp, TransportEnvelope, TransportMessage, TransportSource,
};
use finitechat_transport::{EpochId, GroupId, MemberId, MessageId};
use futures_util::StreamExt;
use nostr::event::FinalizeEvent;
use nostr::{EventBuilder, Keys, Kind, PublicKey, Tag, Timestamp as NostrTimestamp};
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
async fn sqlite_blob_meta_backfills_for_databases_written_before_the_meta_table() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let ciphertext = b"pre-meta-table attachment ciphertext";

    let descriptor = {
        let app = persistent_app(&db_path);
        let response = put_blob(app, ciphertext).await;
        assert_eq!(response.status(), StatusCode::OK);
        let descriptor: BlobDescriptor = read_json(response).await;
        descriptor
    };

    // Reduce the database to its pre-http_blob_meta shape: payload rows only,
    // exactly what a production store written before this table looked like.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open raw");
        conn.execute("DELETE FROM http_blob_meta", [])
            .expect("drop meta rows");
    }

    let app = persistent_app(&db_path);
    let response = get_blob(app.clone(), &descriptor.sha256).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(read_body(response).await.as_ref(), ciphertext);

    // The backfilled meta row is durable, not a read-through side effect.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open raw");
        let (size_bytes, backend): (u64, String) = conn
            .query_row(
                "SELECT size_bytes, backend FROM http_blob_meta WHERE sha256 = ?1",
                rusqlite::params![descriptor.sha256],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("backfilled meta row");
        assert_eq!(size_bytes, ciphertext.len() as u64);
        assert_eq!(backend, "sqlite");
    }
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

fn pairing_event(keys: &Keys, recipient: PublicKey, content: &str) -> Vec<u8> {
    let event = EventBuilder::new(Kind::Custom(24_134), content)
        .tags([Tag::public_key(recipient)])
        .custom_created_at(NostrTimestamp::now())
        .finalize(keys)
        .expect("signed pairing event");
    serde_json::to_vec(&event).expect("pairing event JSON")
}

#[tokio::test]
async fn sqlite_pairing_events_are_bound_idempotent_and_survive_restart() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let pairing_session_id = "pair-http-session".to_owned();
    let target = Keys::generate();
    let source = Keys::generate();
    let attacker = Keys::generate();
    let offer = pairing_event(&target, source.public_key(), "offer-ciphertext");
    let attacker_offer = pairing_event(&attacker, source.public_key(), "attacker-offer");
    let confirmation = pairing_event(&source, target.public_key(), "confirm-ciphertext");
    let payload = pairing_event(&source, target.public_key(), "payload-ciphertext");
    let complete = pairing_event(&target, source.public_key(), "complete-ciphertext");
    let app = persistent_app(&db_path);

    let response = post_json(
        app.clone(),
        "/pairing-sessions",
        &CreatePairingSessionRequest {
            version: 1,
            pairing_session_id: pairing_session_id.clone(),
            target_device_id: "ios-test".to_owned(),
            target_public_key: target.public_key().to_hex(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let created: HttpPairingSessionRecord = read_json(response).await;
    assert_eq!(created.state, HttpPairingSessionState::Created);
    assert!(created.events.is_empty());

    let response = post_json(
        app.clone(),
        "/pairing-sessions/offer",
        &PublishPairingOfferRequest {
            pairing_session_id: pairing_session_id.clone(),
            offer_event: attacker_offer,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = post_json(
        app.clone(),
        "/pairing-sessions/get",
        &GetPairingSessionRequest {
            pairing_session_id: pairing_session_id.clone(),
        },
    )
    .await;
    let unchanged: Option<HttpPairingSessionRecord> = read_json(response).await;
    assert_eq!(
        unchanged
            .expect("attacker rejection preserves session")
            .state,
        HttpPairingSessionState::Created
    );

    let response = post_json(
        app.clone(),
        "/pairing-sessions/offer",
        &PublishPairingOfferRequest {
            pairing_session_id: pairing_session_id.clone(),
            offer_event: offer.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/pairing-sessions/offer",
        &PublishPairingOfferRequest {
            pairing_session_id: pairing_session_id.clone(),
            offer_event: offer,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let offered: HttpPairingSessionRecord = read_json(response).await;
    assert_eq!(offered.events.len(), 1);

    let response = post_json(
        app.clone(),
        "/pairing-sessions/response",
        &PublishPairingResponseRequest {
            pairing_session_id: pairing_session_id.clone(),
            source_confirmation_event: confirmation.clone(),
            payload_event: payload.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let responded: HttpPairingSessionRecord = read_json(response).await;
    assert_eq!(responded.state, HttpPairingSessionState::ResponsePublished);
    assert_eq!(responded.events.len(), 3);

    let altered_payload = pairing_event(&source, target.public_key(), "different-payload");
    let response = post_json(
        app.clone(),
        "/pairing-sessions/response",
        &PublishPairingResponseRequest {
            pairing_session_id: pairing_session_id.clone(),
            source_confirmation_event: confirmation.clone(),
            payload_event: altered_payload,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/pairing-sessions/response",
        &PublishPairingResponseRequest {
            pairing_session_id: pairing_session_id.clone(),
            source_confirmation_event: confirmation,
            payload_event: payload,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app.clone(),
        "/pairing-sessions/complete",
        &PublishPairingCompleteRequest {
            pairing_session_id: pairing_session_id.clone(),
            complete_event: complete.clone(),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
        "/pairing-sessions/complete",
        &PublishPairingCompleteRequest {
            pairing_session_id: pairing_session_id.clone(),
            complete_event: complete,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let completed: HttpPairingSessionRecord = read_json(response).await;
    assert_eq!(completed.state, HttpPairingSessionState::Completed);
    assert_eq!(completed.events.len(), 4);

    let response = post_json(
        app,
        "/pairing-sessions/get",
        &GetPairingSessionRequest { pairing_session_id },
    )
    .await;
    let stored: Option<HttpPairingSessionRecord> = read_json(response).await;
    assert_eq!(stored.expect("persisted pairing"), completed);
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

    let app = persistent_app(&db_path);
    let response = post_json(
        app.clone(),
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

    // Model the normalized crash window: the commit transaction (delivery
    // entry + idempotency + directory rows) is durable, but the
    // room-membership projection rides the checkpoint cadence and has not
    // caught up — exactly the shape every boot derives from.
    publish_and_claim_key_package_for_add(&app, &request).await;
    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let durable: CommitAccepted = read_json(response).await;
    assert_eq!(durable.seq, 1);

    // Boot derivation repairs the projection from the delivery-entry tail
    // before any request runs (the frozen-table incident fix), so the
    // directory already reflects the durable commit.
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
    assert_eq!(before_retry.rooms[0]["current_epoch"], 1);
    assert_eq!(before_retry.rooms[0]["last_seq"], 1);
    assert_eq!(
        before_retry.rooms[0]["devices"][1]["device"]["device_id"],
        "alice-phone"
    );
    assert_eq!(before_retry.rooms[0]["devices"][1]["active"], false);

    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
    assert_eq!(accepted.seq, durable.seq);
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
    assert_eq!(counts.unread, 1);
    assert_eq!(counts.command_inbox, 1);

    let chat_effect = application_effect(&app, &accepted_chat.message_id)
        .await
        .expect("chat effect");
    assert_eq!(chat_effect.seq, 1);
    assert_eq!(chat_effect.sender, alice);
    assert!(chat_effect.delivery_policy.creates_unread());
    assert!(!chat_effect.delivery_policy.creates_command_inbox_work());

    let command_effect = application_effect(&app, &accepted_command.message_id)
        .await
        .expect("command effect");
    assert!(!command_effect.delivery_policy.creates_unread());
    assert!(command_effect.delivery_policy.creates_command_inbox_work());

    let receipt_effect = application_effect(&app, &accepted_receipt.message_id)
        .await
        .expect("receipt effect");
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
            unread: 0,
            command_inbox: 0,
        }
    );
    let effect = application_effect(&app, &accepted.message_id)
        .await
        .expect("runtime state effect");
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
            unread: 0,
            command_inbox: 3,
        }
    );
    let status_effect = application_effect(&app, &status_refresh.message_id)
        .await
        .expect("status refresh effect");
    assert!(!status_effect.delivery_policy.creates_unread());
    assert!(status_effect.delivery_policy.creates_command_inbox_work());

    for message_id in [
        first_command_accepted.message_id,
        second_command_accepted.message_id,
    ] {
        let effect = application_effect(&app, &message_id)
            .await
            .expect("runtime command effect");
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

    // Eviction is per route and drops the oldest record first; the response
    // stays sorted by received_at_ms.
    let response = post_json(
        app.clone(),
        "/activities/get",
        &GetEphemeralActivitiesRequest {
            room_id: room_id.clone(),
            conversation_id: Some("topic-route".to_owned()),
            requester: alice.clone(),
            now_ms: 2_000,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let activities: GetEphemeralActivitiesResponse = read_json(response).await;
    let received: Vec<u64> = activities
        .records
        .iter()
        .map(|record| record.received_at_ms)
        .collect();
    let expected: Vec<u64> = (1..=MAX_EPHEMERAL_ACTIVITY_CACHE_ENTRIES_PER_ROUTE)
        .map(|index| 2_000 + u64::from(index))
        .collect();
    assert_eq!(received, expected);

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
async fn sqlite_ephemeral_activity_query_spans_routes_and_prunes_queried_room() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let room_id = "room-ephemeral-activity-query".to_owned();
    let mls_group_id = "mls-ephemeral-activity-query".to_owned();
    let other_room_id = "room-ephemeral-activity-query-other".to_owned();
    let other_mls_group_id = "mls-ephemeral-activity-query-other".to_owned();
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let add_bob = submit_add_device_request(
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-ephemeral-query-bob",
        "commit-ephemeral-query-bob",
    );
    let app = persistent_app(&db_path);

    for (room_id, mls_group_id) in [
        (room_id.clone(), mls_group_id.clone()),
        (other_room_id.clone(), other_mls_group_id.clone()),
    ] {
        let response = post_json(
            app.clone(),
            "/account-rooms/bootstrap",
            &BootstrapAccountRoomRequest {
                room_id,
                mls_group_id,
                creator: alice.clone(),
                protocol: RoomProtocol::default(),
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    publish_and_claim_key_package_for_add(&app, &add_bob).await;
    let response = post_json(app.clone(), "/commits", &add_bob).await;
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
    let claimed: Vec<HttpClaimedWelcome> = read_json(response).await;
    assert_eq!(claimed.len(), 1);
    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id("welcome-ephemeral-query-bob"),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // Two routes (senders) in one room+conversation; each sender's first
    // record is expired at query time.
    for (sender, received_at_ms) in [
        (&alice, 1_000),
        (&bob, 1_500),
        (&alice, 4_100),
        (&alice, 4_500),
        (&bob, 4_500),
    ] {
        let response = post_json(
            app.clone(),
            "/activities",
            &ephemeral_activity_request(
                &room_id,
                &mls_group_id,
                sender,
                1,
                Some("topic-shared"),
                received_at_ms,
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    // Same room, room-wide route: never part of the conversation query.
    let response = post_json(
        app.clone(),
        "/activities",
        &ephemeral_activity_request(&room_id, &mls_group_id, &alice, 1, None, 4_200),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    // Other room: expired at the same query instant, but pruned only by a
    // query against its own room.
    let response = post_json(
        app.clone(),
        "/activities",
        &ephemeral_activity_request(&other_room_id, &other_mls_group_id, &alice, 0, None, 1_000),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = post_json(
        app.clone(),
        "/activities/get",
        &GetEphemeralActivitiesRequest {
            room_id: room_id.clone(),
            conversation_id: Some("topic-shared".to_owned()),
            requester: alice.clone(),
            now_ms: 5_000,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let activities: GetEphemeralActivitiesResponse = read_json(response).await;
    let observed: Vec<(DeviceRef, u64)> = activities
        .records
        .iter()
        .map(|record| (record.sender.clone(), record.received_at_ms))
        .collect();
    assert_eq!(
        observed,
        vec![
            (alice.clone(), 4_100),
            (alice.clone(), 4_500),
            (bob.clone(), 4_500)
        ]
    );

    // The first query pruned only its own room: the other room's record,
    // expired at the same instant, is still cached and still served.
    let response = post_json(
        app,
        "/activities/get",
        &GetEphemeralActivitiesRequest {
            room_id: other_room_id.clone(),
            conversation_id: None,
            requester: alice.clone(),
            now_ms: 1_500,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let activities: GetEphemeralActivitiesResponse = read_json(response).await;
    let observed: Vec<(DeviceRef, u64)> = activities
        .records
        .iter()
        .map(|record| (record.sender.clone(), record.received_at_ms))
        .collect();
    assert_eq!(observed, vec![(alice.clone(), 1_000)]);
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
async fn sqlite_nostr_profile_cache_rejects_invalid_metadata_json_without_side_effects() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let app = persistent_app(&db_path);
    let account_id = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_owned();

    let profile = |metadata_json: String| NostrProfileRecord {
        account_id: account_id.clone(),
        name: Some("alice".to_owned()),
        display_name: None,
        about: None,
        picture: None,
        bot: None,
        finite_role: None,
        metadata_json: Some(metadata_json),
        fetched_at_ms: 1_000,
        expires_at_ms: 2_000,
    };

    let oversized = format!(r#"{{"big":"{}"}}"#, "x".repeat(16 * 1024));
    let response = post_json(
        app.clone(),
        "/profiles/nostr",
        &PutNostrProfileRequest {
            profile: profile(oversized),
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
            profile: profile("not json".to_owned()),
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
            profile: profile(r#"["not","an","object"]"#.to_owned()),
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
            unread: 1,
            command_inbox: 0,
        }
    );
    let effect = application_effect(app, message_id).await.expect("effect");
    assert_eq!(effect.seq, 1);
    assert_eq!(effect.message_id, message_id);
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

// The legacy uncompressed snapshot table is no longer minted by the store's
// DDL; tests that reproduce a database written by pre-v2 builds recreate it
// exactly as those builds left it behind.
const LEGACY_SNAPSHOT_TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS http_state_snapshots (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    last_op_seq INTEGER NOT NULL,
    snapshot_json TEXT NOT NULL
)";

#[tokio::test]
async fn sqlite_fresh_database_never_mints_legacy_tables() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");

    let state = persistent_state(&db_path);
    drop(state);

    // Single-deploy gate: a fresh database must not create ANY legacy engine
    // table — their existence is what tells the next boot to run the fold.
    let conn = Connection::open(&db_path).expect("open raw");
    for table in [
        "http_state_snapshots",
        "http_state_snapshots_v2",
        "http_delivery_ops",
        "http_room_memberships",
        "http_account_rooms",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .expect("count legacy tables");
        assert_eq!(count, 0, "a fresh database must not mint {table}");
    }
}

#[tokio::test]
async fn sqlite_legacy_snapshot_row_without_v2_successor_fails_boot_closed() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let room_id = "room-legacy-snapshot".to_owned();
    let mls_group_id = "mls-legacy-snapshot".to_owned();

    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    for index in 0..3 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            format!("legacy message {index}").as_bytes(),
            &format!("legacy-msg-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    drop(app);
    drop(state);

    // Fabricate the pre-cutover shape with a live v2 snapshot...
    defold_into_legacy_shape(&db_path, Some(2));

    // ...then rewrite the snapshot into the legacy uncompressed table,
    // exactly as a pre-v2 build persisted it, and remove the v2 row (the
    // op-log prefix is already pruned below the horizon, as the old
    // cross-generation MIN() prune would have left it). The fold's reader
    // cannot interpret the v1 row: replaying from what remains of the log
    // could silently discard history, so the boot must fail closed.
    let legacy_seq = {
        let conn = Connection::open(&db_path).expect("open raw");
        let (seq, compressed): (i64, Vec<u8>) = conn
            .query_row(
                "SELECT last_op_seq, snapshot_zstd FROM http_state_snapshots_v2 WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("v2 snapshot row");
        let json = zstd::decode_all(compressed.as_slice()).expect("decompress snapshot");
        conn.execute_batch(LEGACY_SNAPSHOT_TABLE_DDL)
            .expect("recreate legacy table");
        conn.execute(
            "INSERT INTO http_state_snapshots (id, last_op_seq, snapshot_json)
             VALUES (1, ?1, ?2)",
            params![seq, String::from_utf8(json).expect("snapshot utf8")],
        )
        .expect("write legacy row");
        conn.execute("DELETE FROM http_state_snapshots_v2", [])
            .expect("remove v2 row");
        seq
    };
    assert!(legacy_seq > 0);

    let error = HttpServerState::from_sqlite_path(&db_path)
        .expect_err("a v1 row with no v2 successor must fail boot closed");
    assert!(matches!(
        error,
        DurableStoreError::LegacySnapshotWithoutV2Successor { last_op_seq }
            if last_op_seq == legacy_seq
    ));
    assert!(
        error.to_string().contains("http_state_snapshots"),
        "the refusal must name the offending table: {error}"
    );

    // Failing closed mutates nothing: the legacy row and the empty v2 table
    // survive the refusal exactly as the failed boot found them.
    let conn = Connection::open(&db_path).expect("open raw");
    let surviving_legacy_seq: i64 = conn
        .query_row(
            "SELECT last_op_seq FROM http_state_snapshots WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("legacy row survives the refusal");
    assert_eq!(surviving_legacy_seq, legacy_seq);
    let v2_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM http_state_snapshots_v2", [], |row| {
            row.get(0)
        })
        .expect("count v2 rows");
    assert_eq!(v2_rows, 0);
}

#[tokio::test]
async fn sqlite_stale_legacy_snapshot_row_is_inert_when_v2_exists() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let room_id = "room-stale-legacy-corpse".to_owned();
    let mls_group_id = "mls-stale-legacy-corpse".to_owned();

    // The lat2 shape at the cutover: a live v2 snapshot next to a
    // months-stale v1 corpse row no build has written since v2 landed.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    let mut last_seq = 0;
    for index in 0..3 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            format!("stale corpse message {index}").as_bytes(),
            &format!("stale-corpse-msg-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: EventAccepted = read_json(response).await;
        last_seq = accepted.seq;
    }
    drop(app);
    drop(state);

    defold_into_legacy_shape(&db_path, Some(2));
    {
        let conn = Connection::open(&db_path).expect("open raw");
        conn.execute_batch(LEGACY_SNAPSHOT_TABLE_DDL)
            .expect("recreate legacy table");
        // The corpse row carries a bogus, HIGHER watermark than the v2
        // snapshot — the shape that misled forensics on 2026-08-29 and that
        // #779 pinned as inert while v2 exists. The payload is never read
        // again; even garbage at a misleading watermark must not disturb a
        // fold that has a v2 snapshot to read.
        let v2_seq: i64 = conn
            .query_row(
                "SELECT last_op_seq FROM http_state_snapshots_v2 WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("v2 snapshot row");
        conn.execute(
            "INSERT INTO http_state_snapshots (id, last_op_seq, snapshot_json)
             VALUES (1, ?1, '{\"stale\":\"corpse\"}')",
            rusqlite::params![v2_seq + 500],
        )
        .expect("write stale legacy row");
    }

    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
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

    // The corpse row is inert, not cleaned up: dropping it is a deliberate
    // operator step, not a boot side effect.
    let conn = Connection::open(&db_path).expect("open raw");
    let legacy_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM http_state_snapshots", [], |row| {
            row.get(0)
        })
        .expect("count legacy rows");
    assert_eq!(legacy_rows, 1);
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

/// Crash points across the normalized commit transaction's durable legs.
/// The legacy per-commit room-membership upsert has no normalized
/// equivalent (the projection rides the checkpoint cadence), so that crash
/// point retired with the engine.
#[derive(Clone, Copy, Debug)]
enum HttpSubmitCommitCrashPoint {
    CommitDeliveryOperation,
    CommitIdempotencyRecord,
    WelcomeDeliveryOperation,
    WelcomeIdempotencyRecord,
    AccountRoomProjection,
    KeyPackageConsumedProjection,
}

impl HttpSubmitCommitCrashPoint {
    const ALL: [Self; 6] = [
        Self::CommitDeliveryOperation,
        Self::CommitIdempotencyRecord,
        Self::WelcomeDeliveryOperation,
        Self::WelcomeIdempotencyRecord,
        Self::AccountRoomProjection,
        Self::KeyPackageConsumedProjection,
    ];

    fn trigger_sql(self) -> &'static str {
        match self {
            Self::CommitDeliveryOperation => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_commit_delivery
                AFTER INSERT ON delivery_entries
                WHEN CAST(NEW.payload AS TEXT) LIKE '%http-crash-matrix-commit%'
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
                AFTER INSERT ON delivery_entries
                WHEN CAST(NEW.payload AS TEXT) LIKE '%welcome-http-crash-tablet%'
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
                AFTER UPDATE OF record_json ON account_room_directory
                WHEN NEW.room_id = 'room-http-crash-matrix'
                  AND NEW.record_json LIKE '%alice-tablet%'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after account-room projection');
                END;
                CREATE TRIGGER finitechat_http_test_crash_after_account_room_projection_insert
                AFTER INSERT ON account_room_directory
                WHEN NEW.room_id = 'room-http-crash-matrix'
                  AND NEW.record_json LIKE '%alice-tablet%'
                BEGIN
                  SELECT RAISE(ROLLBACK, 'finitechat http test crash after account-room projection');
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
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_account_room_projection_insert;
        DROP TRIGGER IF EXISTS finitechat_http_test_crash_after_key_package_consumed;
        "#,
    )
    .expect("clear HTTP commit crash triggers");
}

/// Crash points across the normalized event transaction's durable legs.
/// The legacy per-event room-membership upsert has no normalized equivalent
/// (the projection rides the checkpoint cadence), so that crash point
/// retired with the engine.
#[derive(Clone, Copy, Debug)]
enum HttpApplicationEventCrashPoint {
    EventDeliveryOperation,
    EventIdempotencyRecord,
    ApplicationEffectProjection,
}

impl HttpApplicationEventCrashPoint {
    const ALL: [Self; 3] = [
        Self::EventDeliveryOperation,
        Self::EventIdempotencyRecord,
        Self::ApplicationEffectProjection,
    ];

    fn trigger_sql(self) -> &'static str {
        match self {
            Self::EventDeliveryOperation => {
                r#"
                CREATE TRIGGER finitechat_http_test_crash_after_application_event_delivery
                AFTER INSERT ON delivery_entries
                WHEN CAST(NEW.payload AS TEXT) LIKE '%application-effect-crash%'
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
async fn sqlite_sync_stream_does_not_replay_old_activity_after_reconnect() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("delivery.sqlite3");
    let alice = DeviceRef::new("alice", "alice-agent");
    let room_id = "room-sync-stream-activity".to_owned();
    let mls_group_id = "mls-sync-stream-activity".to_owned();
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

    let first_activity =
        ephemeral_activity_request(&room_id, &mls_group_id, &alice, 0, None, 1_000);
    let response = post_json(app.clone(), "/activities", &first_activity).await;
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
    let mut stream = response.into_body().into_data_stream();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            read_next_sync_hint(&mut stream),
        )
        .await
        .is_err(),
        "activity that predates the stream must be the initial baseline"
    );

    let second_activity =
        ephemeral_activity_request(&room_id, &mls_group_id, &alice, 0, None, 2_000);
    let response = post_json(app.clone(), "/activities", &second_activity).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        read_next_sync_hint(&mut stream).await,
        SyncHintEvent::ActivityChanged {
            room_id: room_id.clone(),
            received_at_ms: 2_000,
        }
    );
    drop(stream);

    let response = post_json(
        app,
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
    let mut reconnected = response.into_body().into_data_stream();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            read_next_sync_hint(&mut reconnected),
        )
        .await
        .is_err(),
        "reconnecting must not replay the same activity hint"
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

// Production shape (lat2, 2026-08-29): the durable op log and the replayed
// delivery service run far ahead of a FROZEN `http_room_memberships` table and
// a stale `http_state_snapshots_v2` row (both stuck at the op-198825 era,
// 2026-08-27 ~02:37 UTC, since the platform-wave deploy b9254c81). A process
// that boots from that split state must keep serving: new publishes above the
// pre-boot head, hints for already-ahead clients, and durable projection +
// snapshot persistence. These tests freeze the table on purpose and prove the
// boot reconciles it.
#[tokio::test]
async fn sqlite_boot_from_frozen_room_projection_serves_and_persists_new_publishes() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let room_id = "room-frozen-projection-boot".to_owned();
    let mls_group_id = "mls-frozen-projection-boot".to_owned();

    // Era 1 (pre-freeze): bootstrap + one event. The frozen row knows only
    // alice at this head.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        b"pre-freeze message",
        "pre-freeze-msg",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let frozen_projection_json: String =
        read_normalized_checkpoint(&db_path)["rooms"][room_id.as_str()].to_string();

    // Era 2 (the frozen window): a membership commit adds bob (epoch 0 -> 1),
    // bob claims+acks the Welcome (his interval activates), and bob chats at
    // epoch 1. The delivery log advances; in production the durable
    // projection table did not follow.
    add_device_to_room(
        &app,
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-frozen-projection-boot",
        "commit-frozen-projection-boot",
    )
    .await;
    let mut pre_boot_head = 0;
    for index in 0..3 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &bob,
            1,
            format!("frozen-window message {index}").as_bytes(),
            &format!("frozen-window-msg-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: EventAccepted = read_json(response).await;
        pre_boot_head = accepted.seq;
    }
    drop(app);
    drop(state);

    // Fabricate the legacy shape, then freeze the durable projection at its
    // era-1 value, exactly as the lat2 store looked after the 2026-08-27
    // deploy: op log runs ahead while http_room_memberships lags behind.
    defold_into_legacy_shape(&db_path, None);
    {
        let conn = Connection::open(&db_path).expect("open raw");
        conn.execute(
            "UPDATE http_room_memberships SET projection_json = ?1 WHERE room_id = ?2",
            params![frozen_projection_json, room_id],
        )
        .expect("freeze projection row");
    }

    // The cutover boot: the fold's reader boots from the op log, reconciles
    // the frozen row, and the normalized engine serves.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());

    // Sanity: the replayed log still serves the tail above the frozen
    // projection's last_seq (clients' cursors are already at the head).
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: pre_boot_head - 1,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(
        page.entries.last().map(|entry| entry.seq),
        Some(pre_boot_head)
    );

    // A client at the current room epoch publishes a NEW message above the
    // pre-boot head. On the stale boot this is exactly the fleet-wide stuck
    // outbox: the frozen projection rejects the current epoch/sender.
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &bob,
        1,
        b"post-boot message",
        "post-boot-msg",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "boot from a frozen projection must accept current-epoch publishes"
    );
    let accepted: EventAccepted = read_json(response).await;
    assert!(accepted.seq > pre_boot_head);

    // The new message is visible to an ahead client above its cursor.
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: pre_boot_head,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].seq, accepted.seq);

    // RoomAdvanced hints fire again: the projection head is above the cursor.
    let response = post_json(
        app.clone(),
        "/sync/wait",
        &SyncWaitRequest {
            rooms: vec![SyncWaitRoom {
                room_id: room_id.clone(),
                after_seq: pre_boot_head,
            }],
            wait_ms: 250,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let waited: SyncWaitResponse = read_json(response).await;
    assert!(waited.woke, "hints must advance for ahead clients");

    // Durable: the reconciliation inside the fold's reader unfroze the
    // legacy membership row past its era-1 value, and the normalized
    // checkpoint carries the repaired projection.
    {
        let conn = Connection::open(&db_path).expect("open raw");
        let projection_json: String = conn
            .query_row(
                "SELECT projection_json FROM http_room_memberships WHERE room_id = ?1",
                params![room_id],
                |row| row.get(0),
            )
            .expect("post-boot projection row");
        let frozen: serde_json::Value =
            serde_json::from_str(&frozen_projection_json).expect("frozen projection json");
        let repaired: serde_json::Value =
            serde_json::from_str(&projection_json).expect("repaired projection json");
        assert!(
            repaired["last_seq"].as_u64().unwrap_or(0) > frozen["last_seq"].as_u64().unwrap_or(0),
            "projection row must persist past the frozen last_seq"
        );
    }
    state.snapshot_now().expect("post-boot checkpoint");
    let checkpoint = read_normalized_checkpoint(&db_path);
    assert!(
        checkpoint["rooms"][room_id.as_str()]["last_seq"]
            .as_u64()
            .expect("checkpoint head")
            >= accepted.seq,
        "the normalized checkpoint must carry the repaired head"
    );
}

#[tokio::test]
async fn sqlite_boot_from_frozen_projection_serves_devices_added_in_frozen_window() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let room_id = "room-frozen-added-device".to_owned();
    let mls_group_id = "mls-frozen-added-device".to_owned();

    // Era 1: bootstrap; the frozen row knows only alice.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    let frozen_projection_json: String =
        read_normalized_checkpoint(&db_path)["rooms"][room_id.as_str()].to_string();

    // Era 2 (frozen window): bob joins (Welcome claimed+acked) and sends; the
    // legacy membership table never learns.
    add_device_to_room(
        &app,
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-frozen-added-device",
        "commit-frozen-added-device",
    )
    .await;
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &bob,
        1,
        b"frozen window from bob",
        "frozen-window-bob",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let pre_boot_head: EventAccepted = read_json(response).await;
    drop(app);
    drop(state);

    defold_into_legacy_shape(&db_path, None);
    {
        let conn = Connection::open(&db_path).expect("open raw");
        conn.execute(
            "UPDATE http_room_memberships SET projection_json = ?1 WHERE room_id = ?2",
            params![frozen_projection_json, room_id],
        )
        .expect("freeze projection row");
    }

    // Boot: bob (added inside the frozen window, missing from the frozen row)
    // must be able to publish and read back above the head.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &bob,
        1,
        b"post-boot from bob",
        "post-boot-bob",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "devices added during the frozen window must be sendable after boot repair"
    );
    let accepted: EventAccepted = read_json(response).await;
    assert!(accepted.seq > pre_boot_head.seq);

    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: pre_boot_head.seq,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].seq, accepted.seq);
}

// The 2026-08-27..29 durable freeze: `http_state_snapshots_v2` stuck at
// last_op_seq=198825 while ~8000 ops accumulated, because the snapshot
// cadence only counted /commits and /events successes — key-package,
// lease, and revoke ops grow the log without ever tripping the snapshot
// interval, and once stale projections reject typed traffic the counter
// never moves again. These tests pin the unfrozen contract: every appended
// op counts toward the cadence, persists survive restarts, and the
// readiness probe never stalls them.
#[tokio::test]
async fn sqlite_snapshot_cadence_counts_ops_from_every_delivery_path() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let room_id = "room-cadence".to_owned();
    let mls_group_id = "mls-cadence".to_owned();

    // The 2026-08-27..29 freeze happened because only some paths counted
    // toward the snapshot cadence. On the normalized engine the cadence
    // refreshes the room-state checkpoint, so drive the room's tail with
    // typed events (the path whose state the checkpoint carries) and also
    // raw delivery-contract publishes, then prove the background interval
    // catches the checkpoint up WITHOUT a restart.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;

    let interval_ops = 4_096usize;
    let baseline: u64 = read_normalized_checkpoint(&db_path)["rooms"][room_id.as_str()]["last_seq"]
        .as_u64()
        .unwrap_or(0);
    let mut head = 0u64;
    // Raw publishes grow the route tail but not the projection watermark;
    // the checkpoint's `last_seq` tracks typed events, so keep the typed
    // head separately for the catch-up assertion.
    let mut typed_head = 0u64;
    for index in 0..interval_ops {
        if index % 2 == 0 {
            let request = append_application_request(
                &room_id,
                &mls_group_id,
                &alice,
                0,
                format!("cadence event {index}").as_bytes(),
                &format!("cadence-event-{index}"),
            );
            let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
            assert_eq!(response.status(), StatusCode::OK);
            let accepted: EventAccepted = read_json(response).await;
            head = accepted.seq;
            typed_head = accepted.seq;
        } else {
            // Raw delivery-contract publish: grows the route tail without a
            // typed projection update — the op class the frozen build
            // missed.
            state
                .publish_message(PublishMessageRequest {
                    target: group_target(group_id(&room_id), room_id.as_bytes().to_vec(), None),
                    message: group_message(
                        &format!("cadence-raw-{index}"),
                        room_id.as_bytes().to_vec(),
                        format!("cadence raw body {index}").as_bytes(),
                    ),
                    idempotency_key: None,
                })
                .expect("raw group publish");
            head += 1;
        }
    }

    // The interval trigger runs on its own background thread and fires when
    // the counted ops cross the interval (the HTTP routes count one op per
    // accepted typed request on top of the state-layer count, so the fire
    // lands mid-loop). Wait for the durable checkpoint to move past its
    // bootstrap-era baseline without any restart.
    assert!(baseline < typed_head);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut caught_up = false;
    while std::time::Instant::now() < deadline {
        let checkpoint = read_normalized_checkpoint(&db_path);
        let stored = checkpoint["rooms"][room_id.as_str()]["last_seq"]
            .as_u64()
            .unwrap_or(0);
        if stored > baseline {
            caught_up = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        caught_up,
        "the cadence must refresh the checkpoint within {interval_ops} ops \
         without a restart (baseline {baseline}, typed head {typed_head}, head {head})"
    );

    // More ops after the interval: the counter keeps counting from the
    // refresh, and an explicit snapshot_now persists the current map.
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        b"post-interval event",
        "cadence-post-interval",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: EventAccepted = read_json(response).await;
    state.snapshot_now().expect("post-interval checkpoint");
    let checkpoint = read_normalized_checkpoint(&db_path);
    assert_eq!(
        checkpoint["rooms"][room_id.as_str()]["last_seq"]
            .as_u64()
            .expect("checkpoint head"),
        accepted.seq,
        "an explicit checkpoint persists the live head"
    );
}

#[tokio::test]
async fn sqlite_readiness_probes_concurrent_with_publishes_do_not_stall_persistence() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let room_id = "room-readyz-concurrent".to_owned();
    let mls_group_id = "mls-readyz-concurrent".to_owned();

    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        b"before probes",
        "readyz-concurrent-before",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    state.snapshot_now().expect("baseline checkpoint");
    let baseline_seq: i64 = {
        let checkpoint = read_normalized_checkpoint(&db_path);
        checkpoint["rooms"][room_id.as_str()]["last_seq"]
            .as_i64()
            .expect("baseline checkpoint head")
    };

    // Interleave public readiness probes (the semantic-serving readiness
    // path from 2026-08-26, which writes through the shared SQLite
    // connection under the ordering-authoritative service lock) with typed
    // publishes. Every publish must be accepted and the durable state must
    // keep advancing.
    let probe_app = app.clone();
    let probes = tokio::spawn(async move {
        for _ in 0..40 {
            let response = probe_app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri("/readyz")
                        .body(Body::empty())
                        .expect("readyz request"),
                )
                .await
                .expect("readyz response");
            assert_eq!(response.status(), StatusCode::OK);
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    });
    for index in 0..40 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            format!("during probes {index}").as_bytes(),
            &format!("readyz-concurrent-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "publishes must not be stalled by readiness probes"
        );
    }
    probes.await.expect("probe loop");

    state.snapshot_now().expect("post-probe checkpoint");
    let final_seq: i64 = {
        let checkpoint = read_normalized_checkpoint(&db_path);
        checkpoint["rooms"][room_id.as_str()]["last_seq"]
            .as_i64()
            .expect("final checkpoint head")
    };
    assert!(final_seq > baseline_seq);

    // And the served log is intact above the baseline cursor.
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 1,
            limit: 100,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 40);
}

// Paul's review of the boot-reconciliation fix: the repaired room
// projections and the account-room directory rows must move in ONE SQLite
// transaction. If the membership rows advance first (autocommit) and the
// directory writes then fail or the process dies, the next boot loads the
// advanced projection and skips the replayed publishes
// (`publish.seq <= projection.last_seq`), so the directory repair is never
// replayable again — http_account_rooms is stranded stale forever. This
// test injects a directory-write failure at boot and proves nothing
// persists (fail-closed), then proves the retry boot converges both
// tables.
#[tokio::test]
async fn sqlite_boot_reconciliation_persists_membership_and_directory_atomically() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let room_id = "room-reconcile-atomic".to_owned();
    let mls_group_id = "mls-reconcile-atomic".to_owned();

    // Era 1 (pre-freeze): bootstrap + one event; capture the frozen
    // projection.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        b"pre-freeze message",
        "reconcile-atomic-pre",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let frozen_projection_json: String =
        read_normalized_checkpoint(&db_path)["rooms"][room_id.as_str()].to_string();

    // Era 2 (the frozen window): bob joins (Welcome claimed+acked) and
    // sends; then the legacy membership table is frozen at its era-1 value.
    let add_accepted = add_device_to_room(
        &app,
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-reconcile-atomic",
        "commit-reconcile-atomic",
    )
    .await;
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &bob,
        1,
        b"frozen window message",
        "reconcile-atomic-window",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let pre_boot_head: EventAccepted = read_json(response).await;
    drop(app);
    drop(state);

    defold_into_legacy_shape(&db_path, None);
    {
        let conn = Connection::open(&db_path).expect("open raw");
        conn.execute(
            "UPDATE http_room_memberships SET projection_json = ?1 WHERE room_id = ?2",
            params![frozen_projection_json, room_id],
        )
        .expect("freeze projection row");
    }

    // Fault injection: every http_account_rooms write aborts, modeling a
    // crash or SQLite failure after the membership rows would have advanced
    // under a non-atomic write order.
    {
        let conn = Connection::open(&db_path).expect("open raw");
        conn.execute_batch(
            r#"
            CREATE TRIGGER finitechat_http_test_fail_account_rooms_insert
            BEFORE INSERT ON http_account_rooms
            BEGIN
                SELECT RAISE(ABORT, 'finitechat http test: injected directory write failure');
            END;
            CREATE TRIGGER finitechat_http_test_fail_account_rooms_update
            BEFORE UPDATE ON http_account_rooms
            BEGIN
                SELECT RAISE(ABORT, 'finitechat http test: injected directory write failure');
            END;
            "#,
        )
        .expect("install directory fault trigger");
    }

    // Boot fails closed — and, the crash-safety contract, NOTHING persisted:
    // the membership row must still be exactly the frozen era-1 row, or a
    // later boot would skip the replayed publishes and strand the directory
    // stale forever.
    let failed_boot = HttpServerState::from_sqlite_path(&db_path);
    assert!(
        failed_boot.is_err(),
        "boot must fail when the reconciliation transaction cannot commit"
    );
    {
        let conn = Connection::open(&db_path).expect("open raw");
        let membership_json: String = conn
            .query_row(
                "SELECT projection_json FROM http_room_memberships WHERE room_id = ?1",
                params![room_id],
                |row| row.get(0),
            )
            .expect("membership row survives failed boot");
        assert_eq!(
            membership_json, frozen_projection_json,
            "a failed reconciliation must not advance the membership watermark"
        );
    }

    // Clear the fault: the retry boot converges BOTH tables in one pass.
    {
        let conn = Connection::open(&db_path).expect("open raw");
        conn.execute_batch(
            "DROP TRIGGER finitechat_http_test_fail_account_rooms_insert;
             DROP TRIGGER finitechat_http_test_fail_account_rooms_update;",
        )
        .expect("clear directory fault trigger");
    }
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());

    // Serving works above the pre-boot head for the current-epoch sender.
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &bob,
        1,
        b"post-repair message",
        "reconcile-atomic-post",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: EventAccepted = read_json(response).await;
    assert!(accepted.seq > pre_boot_head.seq);
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: pre_boot_head.seq,
            limit: 10,
            requester: None,
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].seq, accepted.seq);

    // Both durable tables moved together: the membership projection is past
    // the frozen row, and the directory reflects the replayed room head.
    {
        let conn = Connection::open(&db_path).expect("open raw");
        let membership_json: String = conn
            .query_row(
                "SELECT projection_json FROM http_room_memberships WHERE room_id = ?1",
                params![room_id],
                |row| row.get(0),
            )
            .expect("repaired membership row");
        assert_ne!(membership_json, frozen_projection_json);
        let repaired: serde_json::Value =
            serde_json::from_str(&membership_json).expect("repaired projection json");
        // The legacy tables freeze at fold time: the reconciliation brought
        // the row to the log head the fold consumed.
        assert_eq!(
            repaired["last_seq"].as_u64().expect("last_seq"),
            pre_boot_head.seq
        );
        let directory_json: String = conn
            .query_row(
                "SELECT record_json FROM http_account_rooms WHERE account_id = 'bob' AND room_id = ?1",
                params![room_id],
                |row| row.get(0),
            )
            .expect("bob directory row");
        let record: serde_json::Value =
            serde_json::from_str(&directory_json).expect("directory record json");
        assert_eq!(record["current_epoch"], 1);
        // Directory rows advance on commits (the live /events path never
        // writes them), so the replayed add commit is the expected head.
        assert_eq!(
            record["last_seq"].as_u64().expect("directory last_seq"),
            add_accepted.seq
        );
    }
    // The live head (past the frozen window) lands in the normalized
    // checkpoint — the structure the next boot derives from.
    state.snapshot_now().expect("post-repair checkpoint");
    let checkpoint = read_normalized_checkpoint(&db_path);
    assert!(
        checkpoint["rooms"][room_id.as_str()]["last_seq"]
            .as_u64()
            .expect("checkpoint head")
            > pre_boot_head.seq,
        "the live head must persist past the pre-boot head"
    );
}

// ---------------------------------------------------------------------------
// Chat store swap (PR 1): the normalized-engine cutover, single deploy.
//
// There is no engine flag: the deploy IS the flip. A durable boot either
// folds a pre-cutover (op-log) database — marker-gated, see the reader in
// `src/cutover.rs` — or starts fresh on the normalized tables. These tests
// prove that contract end to end on real SQLite files:
//
// * `defold_into_legacy_shape` fabricates pre-cutover databases for the
//   tests: it serves an era through the normalized engine, then rewrites
//   the file into the exact legacy shape (op log, v2 snapshot with a pruned
//   prefix, projection rows, directory rows) and empties the normalized
//   tables. The legacy SERVING engine is gone, so the fixture factory is
//   the only writer of legacy tables left in the tree — test code, not
//   product code.
// * The fold tests prove full-history survival, the frozen-#770-shape
//   repair inside the fold's reader, and the single-path steady boot.
// * The fail-closed tests prove divergence blocks (checkpoint ahead of the
//   entries, undecodable checkpoint, v1 snapshot without a v2 successor).
// * The replay-diff proves fold(de-fold(N)) == N on the synthetic
//   4,200-op fixture, and — with REPLAY_DIFF_DB=/path/to/copy — runs the
//   same diff against a production database copy.
// ---------------------------------------------------------------------------

/// Byte-copy a database (and its WAL sidecars) to another path. The source
/// must not have live writers when this runs.
fn copy_database(source: &std::path::Path, target: &std::path::Path) {
    std::fs::copy(source, target).expect("copy database file");
    for sidecar in ["-wal", "-shm"] {
        let from = source.with_file_name(format!(
            "{}{sidecar}",
            source.file_name().expect("db file name").to_string_lossy()
        ));
        if from.exists() {
            let to = target.with_file_name(format!(
                "{}{sidecar}",
                target.file_name().expect("db file name").to_string_lossy()
            ));
            std::fs::copy(from, to).expect("copy database sidecar");
        }
    }
}

fn read_normalized_checkpoint(path: &std::path::Path) -> serde_json::Value {
    let conn = Connection::open(path).expect("open normalized db");
    let compressed: Vec<u8> = conn
        .query_row(
            "SELECT state_zstd FROM room_state_checkpoint WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("checkpoint row");
    let plain = zstd::decode_all(compressed.as_slice()).expect("decompress checkpoint");
    serde_json::from_slice(&plain).expect("checkpoint json")
}

fn write_normalized_checkpoint(path: &std::path::Path, checkpoint: &serde_json::Value) {
    let plain = serde_json::to_vec(checkpoint).expect("checkpoint json");
    let compressed = zstd::encode_all(plain.as_slice(), 3).expect("compress checkpoint");
    let conn = Connection::open(path).expect("open normalized db");
    conn.execute(
        "UPDATE room_state_checkpoint SET state_zstd = ?1 WHERE id = 1",
        params![compressed],
    )
    .expect("tamper checkpoint");
}

async fn sync_all_group_entries(app: &Router, room: &str) -> Vec<(u64, String, Vec<u8>)> {
    let mut entries = Vec::new();
    let mut after_seq = 0;
    loop {
        let response = post_json(
            app.clone(),
            "/sync/group",
            &GroupSyncRequest {
                group_id: group_id(room),
                after_seq,
                limit: 100,
                requester: None,
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let page: HttpSyncPage = read_json(response).await;
        for entry in &page.entries {
            entries.push((
                entry.seq,
                String::from_utf8(entry.message.id.as_slice().to_vec()).expect("id"),
                entry.message.payload.clone(),
            ));
        }
        if !page.has_more || page.entries.is_empty() {
            return entries;
        }
        after_seq = page.next_after_seq;
    }
}

async fn sync_all_inbox_entries(app: &Router, recipient: &MemberId) -> Vec<(u64, Vec<u8>)> {
    let mut entries = Vec::new();
    let mut after_seq = 0;
    loop {
        let response = post_json(
            app.clone(),
            "/sync/inbox",
            &InboxSyncRequest {
                recipient: recipient.clone(),
                after_seq,
                limit: 100,
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let page: HttpSyncPage = read_json(response).await;
        for entry in &page.entries {
            entries.push((entry.seq, entry.message.payload.clone()));
        }
        if !page.has_more || page.entries.is_empty() {
            return entries;
        }
        after_seq = page.next_after_seq;
    }
}

/// One delivery entry read back from `delivery_entries`, with everything
/// the de-fold needs to rebuild its legacy op row.
struct DefoldEntry {
    /// (plane, route key) of the owning route.
    plane: String,
    route_key: Vec<u8>,
    seq: u64,
    message: TransportMessage,
}

/// Rewrite a normalized-served database into the exact shape a pre-cutover
/// (legacy op-log) build would have left behind, then EMPTY the normalized
/// tables: the next boot sees no fold marker and legacy tables present, so
/// the fold runs. This is test fixture fabrication — the legacy serving
/// engine that once wrote these tables is deleted.
///
/// `prefix_in_snapshot` folds that many leading ops into a v2 snapshot
/// (pruning them from the replayed log, exactly like the legacy snapshot
/// writer did); `None` leaves a never-snapshotted full op log.
///
/// Fidelity notes, so the round-trip stays honest:
/// * Delivery entries become `publish_message` ops in per-route seq order
///   (any interleaving preserving per-route order replays to the same
///   queues); commit admissions ride the op targets from
///   `group_commit_epochs`.
/// * KeyPackages are seeded directly into the snapshot's service (they are
///   not re-derived as publish ops), and the snapshot's inventory section
///   carries the current shared-table rows verbatim — the fold re-seeds
///   that table from the replay, so it must round-trip byte-equal.
/// * The membership/directory rows are copied from the checkpoint and the
///   normalized directory table unchanged.
fn defold_into_legacy_shape(path: &std::path::Path, prefix_in_snapshot: Option<usize>) -> usize {
    let conn = Connection::open(path).expect("open normalized db");

    // 1. Read every route's entries (per-route seq order).
    let mut statement = conn
        .prepare(
            "SELECT r.plane, r.route_key, e.seq, e.message_id, e.payload, e.ts,
                    e.causal_deps_json, e.source, e.envelope_kind, e.envelope_ref
             FROM delivery_entries e
             JOIN delivery_routes r ON r.route_id = e.route_id
             ORDER BY r.plane ASC, r.route_key ASC, e.seq ASC",
        )
        .expect("prepare entry read");
    let rows = statement
        .query_map([], |row| {
            Ok(DefoldEntry {
                plane: row.get(0)?,
                route_key: row.get(1)?,
                seq: u64::try_from(row.get::<_, i64>(2)?).expect("seq"),
                message: TransportMessage {
                    id: MessageId::new(row.get::<_, Vec<u8>>(3)?),
                    payload: row.get::<_, Vec<u8>>(4)?,
                    timestamp: Timestamp(u64::try_from(row.get::<_, i64>(5)?).expect("ts")),
                    causal_deps: serde_json::from_str(&row.get::<_, String>(6)?)
                        .expect("causal deps"),
                    source: TransportSource(row.get::<_, String>(7)?),
                    envelope: match row.get::<_, i64>(8)? {
                        0 => TransportEnvelope::GroupMessage {
                            transport_group_id: row.get::<_, Vec<u8>>(9)?,
                        },
                        _ => TransportEnvelope::Welcome {
                            recipient: MemberId::new(row.get::<_, Vec<u8>>(9)?),
                        },
                    },
                },
            })
        })
        .expect("entry rows");
    let entries = rows.collect::<Result<Vec<_>, _>>().expect("entries");
    let total_ops = entries.len();

    // 2. Commit admissions per (route, seq).
    let mut admissions: std::collections::BTreeMap<(String, Vec<u8>, u64), u64> =
        std::collections::BTreeMap::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT r.plane, r.route_key, e.seq, k.source_epoch
                 FROM group_commit_epochs k
                 JOIN delivery_routes r ON r.route_id = k.route_id
                 JOIN delivery_entries e ON e.route_id = k.route_id AND e.seq = k.seq",
            )
            .expect("prepare epoch read");
        let rows = statement
            .query_map([], |row| {
                Ok((
                    (
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        u64::try_from(row.get::<_, i64>(2)?).expect("seq"),
                    ),
                    u64::try_from(row.get::<_, i64>(3)?).expect("epoch"),
                ))
            })
            .expect("epoch rows");
        for row in rows {
            let ((plane, route_key, seq), epoch) = row.expect("epoch row");
            admissions.insert((plane, route_key, seq), epoch);
        }
    }

    // 3. Rebuild the target of each entry from its envelope (+ admission).
    let target_of = |entry: &DefoldEntry| -> HttpPublishTarget {
        let admission = admissions
            .get(&(entry.plane.clone(), entry.route_key.clone(), entry.seq))
            .map(|source_epoch| HttpCommitAdmission {
                source_epoch: EpochId(*source_epoch),
            });
        match &entry.message.envelope {
            TransportEnvelope::GroupMessage { transport_group_id } => HttpPublishTarget::Group {
                group_id: GroupId::new(entry.route_key.clone()),
                transport_group_id: transport_group_id.clone(),
                commit_admission: admission,
            },
            TransportEnvelope::Welcome { recipient } => HttpPublishTarget::Inbox {
                recipient: recipient.clone(),
            },
        }
    };

    // 4. Shared-table rows that must survive the fold byte-equal: the
    //    inventory triples (the fold re-seeds the table from the replayed
    //    state, so the snapshot carries them verbatim) and the revoked set.
    let inventory_rows: Vec<(String, String, String)> = {
        let mut statement = conn
            .prepare(
                "SELECT key_package_id_json, owner_json, state_json
                 FROM http_key_package_inventory",
            )
            .expect("prepare inventory read");
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("inventory rows");
        rows.collect::<Result<Vec<_>, _>>().expect("inventory")
    };
    let revoked_devices: Vec<String> = {
        let mut statement = conn
            .prepare("SELECT device_key FROM revoked_devices ORDER BY device_key ASC")
            .expect("prepare revoked read");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("revoked rows");
        rows.collect::<Result<Vec<_>, _>>().expect("revoked")
    };
    let key_packages: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> = {
        let mut statement = conn
            .prepare(
                "SELECT key_package_id, owner, key_package_bytes
                 FROM sql_key_packages
                 ORDER BY key_package_id ASC",
            )
            .expect("prepare kp read");
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })
            .expect("kp rows");
        rows.collect::<Result<Vec<_>, _>>().expect("kp")
    };

    // 5. Replay the prefix through a real HttpDeliveryService — the same
    //    RAM core the legacy engine snapshotted — and publish the core
    //    KeyPackages into it (their ops are not re-derived; see doc).
    let prefix_len = prefix_in_snapshot.unwrap_or(0).min(total_ops);
    let mut service = finitechat_delivery::HttpDeliveryService::with_limits(
        finitechat_server::finite_delivery_limits(),
    );
    for entry in &entries[..prefix_len] {
        service
            .publish(target_of(entry), entry.message.clone())
            .expect("prefix replay publish");
    }
    for (key_package_id, owner, bytes) in &key_packages {
        service
            .publish_key_package(HttpKeyPackagePublication {
                key_package_id: HttpKeyPackageId::new(key_package_id.clone()),
                owner: MemberId::new(owner.clone()),
                key_package: KeyPackage::new(bytes.clone()),
            })
            .expect("prefix key package");
    }

    // 6. Write the legacy tables.
    conn.execute_batch(
        "CREATE TABLE http_delivery_ops (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            body_json TEXT NOT NULL
        );
        CREATE TABLE http_state_snapshots_v2 (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            last_op_seq INTEGER NOT NULL,
            snapshot_zstd BLOB NOT NULL
        );
        CREATE TABLE http_account_rooms (
            account_id TEXT NOT NULL,
            room_id TEXT NOT NULL,
            record_json TEXT NOT NULL,
            PRIMARY KEY(account_id, room_id)
        );
        CREATE TABLE http_room_memberships (
            room_id TEXT PRIMARY KEY,
            projection_json TEXT NOT NULL
        );",
    )
    .expect("create legacy tables");

    for entry in &entries {
        let target = target_of(entry);
        let body = serde_json::json!({
            "PublishMessage": {
                "target": target,
                "message": entry.message,
            }
        });
        conn.execute(
            "INSERT INTO http_delivery_ops (kind, body_json) VALUES ('publish_message', ?1)",
            params![body.to_string()],
        )
        .expect("write op row");
    }

    if prefix_len > 0 {
        // The snapshot document mirrors the legacy DurableStateSnapshot:
        // the replayed service, the current inventory rows (so the fold's
        // re-seed round-trips), and the revoked set. Inventory records keep
        // their stored JSON verbatim; the fields the table does not carry
        // (payload bytes, finite metadata) are exactly what the serving
        // engine re-synthesizes as empty when it loads the table.
        let inventory_json: Vec<serde_json::Value> = inventory_rows
            .iter()
            .map(|(key_package_id, owner, state)| {
                serde_json::json!({
                    "key_package_id": serde_json::from_str::<serde_json::Value>(key_package_id)
                        .expect("stored id json"),
                    "owner": serde_json::from_str::<serde_json::Value>(owner)
                        .expect("stored owner json"),
                    "key_package": {"bytes": []},
                    "state": serde_json::from_str::<serde_json::Value>(state)
                        .expect("stored state json"),
                    "finite_metadata": null,
                })
            })
            .collect();
        let snapshot = serde_json::json!({
            "service": service,
            "key_package_inventory": inventory_json,
            "revoked_devices": revoked_devices,
        });
        let compressed =
            zstd::encode_all(snapshot.to_string().as_bytes(), 3).expect("compress snapshot");
        conn.execute(
            "INSERT INTO http_state_snapshots_v2 (id, last_op_seq, snapshot_zstd)
             VALUES (1, ?1, ?2)",
            params![prefix_len as i64, compressed],
        )
        .expect("write v2 snapshot");
        // The legacy writer pruned the covered prefix.
        conn.execute(
            "DELETE FROM http_delivery_ops WHERE seq <= ?1",
            params![prefix_len as i64],
        )
        .expect("prune covered prefix");
    }

    // Memberships + directory: copied verbatim from the normalized homes.
    let checkpoint = read_normalized_checkpoint(path);
    let rooms = checkpoint["rooms"].as_object().expect("checkpoint rooms");
    for (room_id, projection) in rooms {
        conn.execute(
            "INSERT INTO http_room_memberships (room_id, projection_json) VALUES (?1, ?2)",
            params![room_id, projection.to_string()],
        )
        .expect("write membership row");
    }
    {
        let mut statement = conn
            .prepare("SELECT account_id, room_id, record_json FROM account_room_directory")
            .expect("prepare directory read");
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("directory rows");
        for row in rows {
            let (account_id, room_id, record_json) = row.expect("directory row");
            conn.execute(
                "INSERT INTO http_account_rooms (account_id, room_id, record_json)
                 VALUES (?1, ?2, ?3)",
                params![account_id, room_id, record_json],
            )
            .expect("write directory row");
        }
    }

    // 7. Empty the normalized tables and clear the fold marker: the file
    //    now looks exactly like a pre-cutover database.
    conn.execute_batch(
        "DELETE FROM delivery_entries;
         DELETE FROM group_commit_epochs;
         DELETE FROM delivery_routes;
         DELETE FROM sql_key_packages;
         DELETE FROM account_room_directory;
         DELETE FROM revoked_devices;
         DELETE FROM room_state_checkpoint;
         DELETE FROM server_meta WHERE key = 'op_log_fold_complete';",
    )
    .expect("clear normalized tables");
    total_ops
}

/// Boot the normalized engine on a fabricated legacy database (running the
/// one-time fold), then prove the full history, the membership projection,
/// the directory, the KeyPackage inventory, revocation, seq continuation,
/// and the frozen legacy tables — then boot again to prove the
/// steady-state single load path.
#[tokio::test]
async fn chat_store_swap_fold_serves_full_legacy_history_on_the_normalized_engine() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let carol = DeviceRef::new("carol", "carol-tablet");
    let room_id = "room-cutover-fold".to_owned();
    let mls_group_id = "mls-cutover-fold".to_owned();

    // The pre-cutover era, served by the (only) engine: bootstrap, chat, a
    // membership commit with a claimed + acked Welcome, more chat, then a
    // second membership commit.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    for index in 0..3 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            format!("pre-snapshot message {index}").as_bytes(),
            &format!("pre-snapshot-msg-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let add_bob = add_device_to_room(
        &app,
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-cutover-fold-1",
        "commit-cutover-fold-1",
    )
    .await;
    for index in 0..2 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &bob,
            1,
            format!("post-add message {index}").as_bytes(),
            &format!("post-add-msg-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    state.snapshot_now().expect("cutover-era checkpoint");
    drop(app);
    drop(state);
    let legacy_entry_count =
        sync_all_group_entries(&http_router(persistent_state(&db_path)), &room_id)
            .await
            .len();
    // Fabricate the legacy shape with a snapshot covering all but the last
    // two entries: the fold must transplant the snapshot base and replay the
    // pruned tail.
    let total_ops = defold_into_legacy_shape(&db_path, Some(legacy_entry_count - 2));
    assert!(
        total_ops > legacy_entry_count,
        "the fixture must include at least one inbox welcome op ({total_ops} ops)"
    );

    // Cutover boot: the fold runs inside this boot and the normalized
    // engine serves.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());

    // Full history survived the fold: every legacy entry, same order.
    let entries = sync_all_group_entries(&app, &room_id).await;
    assert_eq!(entries.len(), legacy_entry_count);
    let add_carol = add_device_to_room_at_epoch(
        &app,
        &room_id,
        &mls_group_id,
        &alice,
        &carol,
        1,
        "welcome-cutover-fold-2",
        "commit-cutover-fold-2",
    )
    .await;
    assert_eq!(
        entries.last().expect("head entry").0 + 1,
        add_carol.seq,
        "seq assignment continues from the folded head"
    );
    // The first three entries predate the v2 snapshot horizon: their op-log
    // rows were pruned, so they could only have come through the fold's
    // snapshot transplant.
    let prefix_payload = String::from_utf8_lossy(&entries[2].2).to_string();
    assert!(
        prefix_payload.contains("pre-snapshot-msg-2"),
        "the pruned prefix must survive via the snapshot transplant"
    );

    // The membership projection survived: bob (added at the commit) sees the
    // commit and everything after, but not the pre-add entries.
    let response = post_json(
        app.clone(),
        "/sync/group",
        &GroupSyncRequest {
            group_id: group_id(&room_id),
            after_seq: 0,
            limit: 100,
            requester: Some(member_for_device(&bob)),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: HttpSyncPage = read_json(response).await;
    let bob_first = page.entries.first().expect("bob sees his commit").seq;
    assert_eq!(bob_first, add_bob.seq);

    // The account-room directory survived with both devices activated.
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
    let listed: ListAccountRoomDirectoryResponse = read_json(response).await;
    assert_eq!(listed.rooms.len(), 1);
    assert_eq!(
        listed.rooms[0]["current_epoch"].as_u64().expect("epoch"),
        2,
        "both add commits advanced the directory epoch"
    );
    // The directory is account-scoped: each member's own record must exist
    // with that account's device active (the claimed-and-acked welcomes).
    for (account_id, device_id) in [
        ("alice", "alice-laptop"),
        ("bob", "bob-phone"),
        ("carol", "carol-tablet"),
    ] {
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
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "directory for {account_id}"
        );
        let listed: ListAccountRoomDirectoryResponse = read_json(response).await;
        assert_eq!(listed.rooms.len(), 1, "{account_id} still lists the room");
        let devices = listed.rooms[0]["devices"].as_array().expect("devices");
        assert!(
            devices.iter().any(|device| {
                device["device"]["device_id"] == device_id
                    && device["active"].as_bool().unwrap_or(false)
            }),
            "{account_id}/{device_id} must be active after the fold"
        );
    }

    // Seq assignment continues from the folded head and the room accepts a
    // current-epoch publish.
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &carol,
        2,
        b"post-fold message",
        "post-fold-msg",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the normalized engine must accept current-epoch publishes after the fold"
    );
    let accepted: EventAccepted = read_json(response).await;
    assert_eq!(accepted.seq, add_carol.seq + 1);

    // The legacy tables are frozen: no op-log rows appear for normalized
    // writes.
    let ops_after_fold = {
        let conn = Connection::open(&db_path).expect("open db");
        conn.query_row("SELECT COUNT(*) FROM http_delivery_ops", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("op count")
    };

    drop(app);
    drop(state);

    // Steady-state boot: checkpoint + delivery-entry tail replay (the fold
    // marker prevents a re-fold). The post-fold entry must be there and the
    // next seq continues.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    let entries = sync_all_group_entries(&app, &room_id).await;
    assert_eq!(entries.len(), legacy_entry_count + 2);
    assert_eq!(entries.last().expect("head").0, add_carol.seq + 1);
    let ops_after_restart = {
        let conn = Connection::open(&db_path).expect("open db");
        conn.query_row("SELECT COUNT(*) FROM http_delivery_ops", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("op count")
    };
    assert_eq!(
        ops_after_fold, ops_after_restart,
        "the normalized engine never appends to the legacy op log"
    );
    assert!(ops_after_restart > 0, "the legacy era wrote ops");
}

/// Review #799 blocking case: a KeyPackage published AFTER the legacy v2
/// snapshot horizon exists only as a `PublishKeyPackage` op in the tail —
/// the fold's service rebuild does not replay those ops, so its payload
/// bytes live nowhere but the replayed wrapper inventory. The fold must
/// still give it a durable `sql_key_packages` row: after the cutover boot
/// and a restart, an account-scoped claim returns the ORIGINAL bytes and
/// account-scoped availability answers from them.
#[tokio::test]
async fn chat_store_swap_fold_preserves_a_post_snapshot_key_package_payload() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let room_id = "room-kp-tail".to_owned();
    let mls_group_id = "mls-kp-tail".to_owned();

    // The legacy era: enough history that the fabricated snapshot covers a
    // strict prefix of the op log.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    for index in 0..3 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            format!("kp-tail message {index}").as_bytes(),
            &format!("kp-tail-msg-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    drop(app);
    drop(state);

    // Snapshot at N: the fabricated legacy shape keeps the first two ops in
    // the v2 snapshot and prunes them from the log; the rest is the tail.
    let total_ops = defold_into_legacy_shape(&db_path, Some(2));
    assert!(total_ops > 2, "the fixture must have a post-snapshot tail");

    // A KeyPackage published after the horizon, appended straight to the
    // legacy op log exactly as the legacy engine would have. The payload is
    // an UploadKeyPackageRequest, so the fold can re-derive the finite
    // (account-scoped) metadata from the bytes.
    let account_id = String::from_utf8(vec![b'a'; 64]).expect("account id");
    let other_account_id = String::from_utf8(vec![b'b'; 64]).expect("other account id");
    let device = DeviceRef::new(account_id.clone(), "phone");
    let publication = finite_key_package_publication(
        &device,
        "kp-post-snapshot-tail",
        "ref-post-snapshot",
        "hash-post-snapshot",
        b"post-snapshot key package payload",
    );
    {
        let conn = Connection::open(&db_path).expect("open raw");
        let body = serde_json::json!({
            "PublishKeyPackage": { "publication": publication }
        });
        conn.execute(
            "INSERT INTO http_delivery_ops (kind, body_json) VALUES ('publish_key_package', ?1)",
            params![body.to_string()],
        )
        .expect("append post-snapshot publish op");
    }

    // Cutover boot: the fold runs here.
    drop(persistent_state(&db_path));

    // The durable payload row exists with the original bytes.
    {
        let conn = Connection::open(&db_path).expect("open raw");
        let bytes: Vec<u8> = conn
            .query_row(
                "SELECT key_package_bytes FROM sql_key_packages WHERE key_package_id = ?1",
                params![publication.key_package_id.as_slice()],
                |row| row.get(0),
            )
            .expect("post-snapshot key package must have a durable payload row");
        assert_eq!(bytes, publication.key_package.bytes());
    }

    // Restart: the steady-state load path (shared inventory triples enriched
    // from sql_key_packages) must serve the same package.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());

    // Account-scoped availability answers from the folded payload: the
    // publishing account is available, an account with no package is not.
    let response = post_json(
        app.clone(),
        "/key-packages/availability",
        &GetKeyPackageAvailabilityRequest {
            account_ids: vec![account_id.clone(), other_account_id.clone()],
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
        vec![(account_id.clone(), true), (other_account_id, false)]
    );

    // The claim returns the ORIGINAL bytes.
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
    let claimed = claimed.expect("the post-snapshot key package survives the fold");
    assert_eq!(claimed.key_package_id, publication.key_package_id);
    assert_eq!(claimed.owner, member_for_device(&device));
    assert_eq!(claimed.key_package.bytes(), publication.key_package.bytes());

    // A further restart does not resurrect the consumed package.
    drop(app);
    drop(state);
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
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

#[allow(clippy::too_many_arguments)]
async fn add_device_to_room_at_epoch(
    app: &Router,
    room_id: &str,
    mls_group_id: &str,
    sender: &DeviceRef,
    added: &DeviceRef,
    epoch: u64,
    welcome_id: &str,
    idempotency_key: &str,
) -> CommitAccepted {
    let request = submit_add_device_request_at_epoch_with_ids(
        room_id,
        mls_group_id,
        sender,
        added,
        epoch,
        welcome_id,
        idempotency_key,
    );
    publish_and_claim_key_package_for_add(app, &request).await;
    let response = post_json(app.clone(), "/commits", &request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let accepted: CommitAccepted = read_json(response).await;
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
    let response = post_json(
        app.clone(),
        "/welcomes/ack",
        &AckWelcomeRequest {
            message_id: id(welcome_id),
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    accepted
}

/// A checkpoint that merely lags (the crash window between an accepted
/// publish and the cadence refresh) is absorbed safely: boot replays the
/// delivery-entry tails and converges. This is the safe direction of the
/// #770 invariant — a stale checkpoint can never serve stale state.
#[tokio::test]
async fn chat_store_swap_boot_absorbs_a_stale_checkpoint_by_replaying_entry_tails() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let bob = DeviceRef::new("bob", "bob-phone");
    let room_id = "room-stale-checkpoint".to_owned();
    let mls_group_id = "mls-stale-checkpoint".to_owned();

    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    let add_bob = add_device_to_room(
        &app,
        &room_id,
        &mls_group_id,
        &alice,
        &bob,
        "welcome-stale-checkpoint",
        "commit-stale-checkpoint",
    )
    .await;

    // Advance the entries far past the checkpoint WITHOUT letting the
    // cadence refresh it (the cadence is thousands of ops; a handful of
    // publishes never triggers it). This is the synthetic freeze: the
    // durable room structure stays at its last value while the delivery
    // log runs ahead — exactly the 2026-08-29 lat2 shape.
    let mut head = add_bob.seq;
    for index in 0..5 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &bob,
            1,
            format!("frozen-window message {index}").as_bytes(),
            &format!("frozen-window-msg-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted: EventAccepted = read_json(response).await;
        head = accepted.seq;
    }
    drop(app);
    drop(state);
    {
        let checkpoint = read_normalized_checkpoint(&db_path);
        let stored_head = checkpoint["rooms"][room_id.as_str()]["last_seq"]
            .as_u64()
            .expect("checkpoint last_seq");
        assert!(
            stored_head < head,
            "checkpoint must lag the entries for this test (stored {stored_head}, head {head})"
        );
    }

    // Boot: the tails replay, the projection converges to the entry head,
    // and the boot persists the refreshed checkpoint.
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    let entries = sync_all_group_entries(&app, room_id.as_str()).await;
    assert_eq!(entries.last().expect("head").0, head);
    drop(app);
    drop(state);
    let checkpoint = read_normalized_checkpoint(&db_path);
    assert_eq!(
        checkpoint["rooms"][room_id.as_str()]["last_seq"]
            .as_u64()
            .expect("refreshed"),
        head,
        "boot must persist the converged checkpoint"
    );
}

/// A checkpoint that claims history the delivery entries do not hold fails
/// boot closed. Divergence is impossible-or-blocking, never absorbable —
/// the #770 fault-injection carried forward.
#[tokio::test]
async fn chat_store_swap_boot_fails_closed_when_the_checkpoint_is_ahead_of_the_entries() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let room_id = "room-checkpoint-ahead".to_owned();
    let mls_group_id = "mls-checkpoint-ahead".to_owned();

    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    let request = append_application_request(
        &room_id,
        &mls_group_id,
        &alice,
        0,
        b"checkpoint-ahead message",
        "checkpoint-ahead-msg",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    drop(app);
    drop(state);
    drop(persistent_state(&db_path));

    // Freeze the checkpoint AHEAD of the route head: claim the room saw
    // five more entries than the delivery log holds.
    {
        let mut checkpoint = read_normalized_checkpoint(&db_path);
        let stored = checkpoint["rooms"][room_id.as_str()]["last_seq"]
            .as_u64()
            .expect("checkpoint last_seq") as i64;
        checkpoint["rooms"][room_id.as_str()]["last_seq"] = serde_json::json!(stored + 5);
        write_normalized_checkpoint(&db_path, &checkpoint);
    }

    let error = HttpServerState::from_sqlite_path(&db_path)
        .expect_err("boot must refuse a checkpoint ahead of the entries");
    assert!(
        matches!(error, DurableStoreError::CheckpointDivergence { .. }),
        "expected CheckpointDivergence, got {error:?}"
    );
}

/// An undecodable checkpoint is corruption, not an empty one: boot refuses
/// rather than silently re-deriving from nothing.
#[tokio::test]
async fn chat_store_swap_boot_fails_closed_on_an_undecodable_checkpoint() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let alice = DeviceRef::new("alice", "alice-laptop");
    let room_id = "room-checkpoint-corrupt".to_owned();
    let mls_group_id = "mls-checkpoint-corrupt".to_owned();

    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    drop(app);
    drop(state);
    drop(persistent_state(&db_path));

    {
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute(
            "UPDATE room_state_checkpoint SET state_zstd = ?1 WHERE id = 1",
            params![b"not-zstd-at-all".to_vec()],
        )
        .expect("corrupt checkpoint");
    }

    let error = HttpServerState::from_sqlite_path(&db_path)
        .expect_err("boot must refuse an undecodable checkpoint");
    // Fail-closed is the contract; the corrupt blob surfaces as a Json/Io
    // decode failure inside the checkpoint load.
    let rendered = format!("{error:?}");
    assert!(
        rendered.contains("Io") || rendered.contains("Sqlite") || rendered.contains("Json"),
        "unexpected error for a corrupt checkpoint: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// Replay-diff: fold fidelity.
//
// Synthetic mode (CI): serve the 4,200-op mixed fixture through the
// normalized engine, fabricate the legacy shape from it, then fold a copy
// and diff every observable — fold(de-fold(N)) must equal N. Byte-equal or
// it does not ship.
//
// Set REPLAY_DIFF_DB=/path/to/database-copy to run the same proof against a
// production database copy (never the live file): there the legacy shape is
// real, so the diff proves the fold's output against the legacy tables
// themselves — the checkpoint must equal the (reconciled) projection rows,
// the directory must be row-for-row equal, and the folded entries must
// equal an INDEPENDENT replay of the v2 snapshot + op tail rebuilt in this
// test from the public delivery contract.
// ---------------------------------------------------------------------------

struct ReplayDiffCoverage {
    rooms: usize,
    room_entries: usize,
    inbox_recipients: usize,
    inbox_entries: usize,
    inventory_owners: usize,
    directory_rows: usize,
}

async fn build_replay_diff_fixture(path: &std::path::Path) {
    let rooms = [
        (
            "diff-room-alpha".to_owned(),
            "diff-mls-alpha".to_owned(),
            DeviceRef::new("alpha", "alpha-laptop"),
        ),
        (
            "diff-room-beta".to_owned(),
            "diff-mls-beta".to_owned(),
            DeviceRef::new("beta", "beta-laptop"),
        ),
        (
            "diff-room-gamma".to_owned(),
            "diff-mls-gamma".to_owned(),
            DeviceRef::new("gamma", "gamma-laptop"),
        ),
    ];
    let mut epochs = [0u64, 0, 0];
    let mut added_devices: Vec<DeviceRef> = Vec::new();

    let state = persistent_state(path);
    let app = http_router(state.clone());
    for (room_id, mls_group_id, creator) in &rooms {
        bootstrap_room(&app, room_id, mls_group_id, creator).await;
    }

    const OPS: usize = 4_200;
    for index in 0..OPS {
        let room = index % rooms.len();
        let (room_id, mls_group_id, creator) = &rooms[room];
        if index % 90 == 40 {
            // Membership commit: publish + claim a KeyPackage, commit the
            // add, then claim + ack the Welcome.
            let device = DeviceRef::new(format!("member-{index}"), format!("member-{index}-phone"));
            let epoch = epochs[room];
            add_device_to_room_at_epoch(
                &app,
                room_id,
                mls_group_id,
                creator,
                &device,
                epoch,
                &format!("welcome-diff-{index}"),
                &format!("commit-diff-{index}"),
            )
            .await;
            epochs[room] += 1;
            added_devices.push(device);
        } else if index % 140 == 70 {
            // KeyPackage churn + revocation for a device that never joins a
            // room.
            let device = DeviceRef::new(format!("churn-{index}"), format!("churn-{index}-laptop"));
            publish_key_package_for_device(&app, &device, &format!("kp-churn-{index}")).await;
            revoke_device(&app, &device).await;
        } else if index % 11 == 3 {
            // Raw delivery-contract group publish (below the typed layer).
            let request = PublishMessageRequest {
                target: group_target(group_id(room_id), room_id.as_bytes().to_vec(), None),
                message: group_message(
                    &format!("raw-group-{index}"),
                    room_id.as_bytes().to_vec(),
                    format!("raw group body {index}").as_bytes(),
                ),
                idempotency_key: None,
            };
            state.publish_message(request).expect("raw group publish");
        } else if index % 11 == 8 {
            // Raw inbox publish to a member that received a Welcome.
            let Some(recipient) = added_devices.last() else {
                continue;
            };
            let member = member_for_device(recipient);
            let request = PublishMessageRequest {
                target: HttpPublishTarget::Inbox {
                    recipient: member.clone(),
                },
                message: welcome_message(
                    &format!("raw-inbox-{index}"),
                    member,
                    format!("raw inbox body {index}").as_bytes(),
                ),
                idempotency_key: None,
            };
            state.publish_message(request).expect("raw inbox publish");
        } else if index % 6 == 5 {
            // Unclaimed KeyPackage inventory churn.
            let device = DeviceRef::new(format!("stock-{index}"), format!("stock-{index}-tablet"));
            publish_key_package_for_device(&app, &device, &format!("kp-stock-{index}")).await;
        } else {
            // Typed application event from the (never-removed) creator.
            let request = append_application_request(
                room_id,
                mls_group_id,
                creator,
                epochs[room],
                format!("diff message {index}").as_bytes(),
                &format!("diff-msg-{index}"),
            );
            let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "fixture event {index} in {room_id} at epoch {}",
                epochs[room]
            );
        }
    }
    // Persist the live room map so the de-fold fabricates membership rows
    // at the serving head (the legacy live path upserted per commit; a
    // stale checkpoint would fabricate the frozen-#770 shape instead).
    state.snapshot_now().expect("fixture checkpoint");
    drop(app);
    drop(state);
}

async fn publish_key_package_for_device(app: &Router, device: &DeviceRef, id: &str) {
    let upload = UploadKeyPackageRequest {
        key_package_id: id.to_owned(),
        owner: device.clone(),
        key_package_ref: format!("ref-{id}"),
        key_package_hash: format!("hash-{id}"),
        key_package_payload: format!("payload-{id}").into_bytes(),
    };
    let publication = HttpKeyPackagePublication {
        key_package_id: HttpKeyPackageId::new(id.as_bytes().to_vec()),
        owner: member_for_device(&upload.owner),
        key_package: KeyPackage::new(serde_json::to_vec(&upload).expect("upload json")),
    };
    let response = post_json(app.clone(), "/key-packages", &publication).await;
    assert_eq!(response.status(), StatusCode::OK);
}

fn read_legacy_room_projections(
    path: &std::path::Path,
) -> std::collections::BTreeMap<String, serde_json::Value> {
    let conn = Connection::open(path).expect("open legacy copy");
    let mut statement = conn
        .prepare("SELECT room_id, projection_json FROM http_room_memberships ORDER BY room_id")
        .expect("prepare membership read");
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("membership rows");
    let mut rooms = std::collections::BTreeMap::new();
    for row in rows {
        let (room_id, json) = row.expect("row");
        rooms.insert(
            room_id,
            serde_json::from_str(&json).expect("projection json"),
        );
    }
    rooms
}

fn read_directory_rows(
    path: &std::path::Path,
    table: &str,
) -> std::collections::BTreeMap<(String, String), serde_json::Value> {
    let conn = Connection::open(path).expect("open copy");
    let mut statement = conn
        .prepare(&format!(
            "SELECT account_id, room_id, record_json FROM {table} ORDER BY account_id, room_id"
        ))
        .expect("prepare directory read");
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("directory rows");
    let mut directory = std::collections::BTreeMap::new();
    for row in rows {
        let (account_id, room_id, json) = row.expect("row");
        directory.insert(
            (account_id, room_id),
            serde_json::from_str(&json).expect("directory record json"),
        );
    }
    directory
}

fn read_inventory_owners(path: &std::path::Path) -> Vec<MemberId> {
    let conn = Connection::open(path).expect("open copy");
    let mut statement = conn
        .prepare("SELECT owner_json FROM http_key_package_inventory")
        .expect("prepare inventory owners");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("inventory owner rows");
    let mut owners = Vec::new();
    for row in rows {
        owners.push(serde_json::from_str(&row.expect("row")).expect("owner json"));
    }
    owners
}

fn read_welcome_recipients(path: &std::path::Path) -> Vec<MemberId> {
    let conn = Connection::open(path).expect("open copy");
    let mut statement = conn
        .prepare("SELECT recipient_json FROM http_welcome_claims")
        .expect("prepare welcome recipients");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("welcome recipient rows");
    let mut recipients = Vec::new();
    for row in rows {
        recipients.push(serde_json::from_str(&row.expect("row")).expect("recipient json"));
    }
    recipients
}

/// Independently rebuild the pre-cutover delivery state from the legacy
/// tables using ONLY the public delivery contract: decode the v2 snapshot's
/// service, replay the op tail into it, and return its route snapshots.
/// This is a second implementation of the reader in `src/cutover.rs` — the
/// replay-diff's ground truth.
fn independent_legacy_replay(
    path: &std::path::Path,
) -> Vec<finitechat_delivery::HttpRouteSnapshot> {
    let conn = Connection::open(path).expect("open legacy copy");
    let horizon: i64 = conn
        .query_row(
            "SELECT COALESCE((SELECT last_op_seq FROM http_state_snapshots_v2 WHERE id = 1), 0)",
            [],
            |row| row.get(0),
        )
        .expect("snapshot horizon");
    let mut service = if horizon > 0 {
        let compressed: Vec<u8> = conn
            .query_row(
                "SELECT snapshot_zstd FROM http_state_snapshots_v2 WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("snapshot blob");
        let plain = zstd::decode_all(compressed.as_slice()).expect("decompress snapshot");
        let document: serde_json::Value = serde_json::from_slice(&plain).expect("snapshot json");
        serde_json::from_value(document["service"].clone()).expect("snapshot service")
    } else {
        finitechat_delivery::HttpDeliveryService::with_limits(
            finitechat_server::finite_delivery_limits(),
        )
    };
    let mut statement = conn
        .prepare("SELECT body_json FROM http_delivery_ops WHERE seq > ?1 ORDER BY seq ASC")
        .expect("prepare op read");
    let rows = statement
        .query_map(params![horizon], |row| row.get::<_, String>(0))
        .expect("op rows");
    for row in rows {
        let body: serde_json::Value = serde_json::from_str(&row.expect("op row")).expect("op json");
        let Some(variant) = body
            .as_object()
            .and_then(|object| object.keys().next().cloned())
        else {
            continue;
        };
        if variant != "PublishMessage" {
            // KeyPackage/revocation ops do not move delivery queues.
            continue;
        }
        let target: HttpPublishTarget =
            serde_json::from_value(body["PublishMessage"]["target"].clone()).expect("op target");
        let message: TransportMessage =
            serde_json::from_value(body["PublishMessage"]["message"].clone()).expect("op message");
        service.publish(target, message).expect("replay publish");
    }
    service.route_snapshots()
}

#[tokio::test]
async fn chat_store_swap_replay_diff_fold_is_the_identity_on_the_legacy_shape() {
    let temp = TempDir::new().expect("tempdir");
    let production_copy = std::env::var("REPLAY_DIFF_DB")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(std::path::PathBuf::from);
    let (source_path, synthetic) = match &production_copy {
        Some(value) => {
            eprintln!("replay-diff: production copy mode ({})", value.display());
            (value.clone(), false)
        }
        None => {
            eprintln!("replay-diff: synthetic fixture mode");
            let fixture = temp.path().join("fixture.sqlite3");
            build_replay_diff_fixture(&fixture).await;
            (fixture, true)
        }
    };

    // In synthetic mode the source is a normalized database: fabricate the
    // legacy shape on a copy first, with the last ~1/14th of the ops as the
    // pruned-tail (mirroring the old fixture's periodic snapshots).
    let legacy_path = temp.path().join("legacy-shape.sqlite3");
    copy_database(&source_path, &legacy_path);
    if synthetic {
        let total_entries: i64 = {
            let conn = Connection::open(&legacy_path).expect("open fixture");
            conn.query_row("SELECT COUNT(*) FROM delivery_entries", [], |row| {
                row.get(0)
            })
            .expect("entry count")
        };
        defold_into_legacy_shape(&legacy_path, Some((total_entries as usize) * 13 / 14));
    }

    // The independent ground truth from the legacy shape, computed BEFORE
    // the fold mutates anything (the fold's reader persists its #770
    // reconciliation into the legacy tables).
    let ground_truth_routes = independent_legacy_replay(&legacy_path);

    // Fold a copy and serve it.
    let normalized_path = temp.path().join("folded.sqlite3");
    copy_database(&legacy_path, &normalized_path);
    let state = persistent_state(&normalized_path);
    let app = http_router(state.clone());

    // 1. Room-membership projections: the legacy tables' post-reconciliation
    //    rows vs the normalized checkpoint. Same room set, and every
    //    projection (intervals, epochs, heads, admins, departed) equal as
    //    JSON documents.
    let legacy_rooms = read_legacy_room_projections(&legacy_path);
    let normalized_checkpoint = read_normalized_checkpoint(&normalized_path);
    let normalized_rooms_value = normalized_checkpoint
        .get("rooms")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let normalized_rooms = normalized_rooms_value
        .as_object()
        .expect("checkpoint rooms object");
    let legacy_room_ids: std::collections::BTreeSet<&String> = legacy_rooms.keys().collect();
    let normalized_room_ids: std::collections::BTreeSet<&String> =
        normalized_rooms.keys().collect();
    assert_eq!(
        legacy_room_ids, normalized_room_ids,
        "room projection sets differ between the legacy tables and the fold"
    );
    for (room_id, legacy_projection) in &legacy_rooms {
        assert_eq!(
            Some(legacy_projection),
            normalized_rooms.get(room_id),
            "room {room_id} membership projection differs after the fold"
        );
    }

    // 2. Per-account directory: identical rows.
    let legacy_directory = read_directory_rows(&legacy_path, "http_account_rooms");
    let normalized_directory = read_directory_rows(&normalized_path, "account_room_directory");
    assert_eq!(
        legacy_directory, normalized_directory,
        "account-room directory differs after the fold"
    );

    // 3. Delivery entries: the folded routes must equal the independent
    //    replay byte-for-byte (seq, message id, payload).
    let folded_routes = {
        let conn = Connection::open(&normalized_path).expect("open folded copy");
        let mut statement = conn
            .prepare(
                "SELECT r.plane, r.route_key, e.seq, e.message_id, e.payload
                 FROM delivery_entries e
                 JOIN delivery_routes r ON r.route_id = e.route_id
                 ORDER BY r.plane ASC, r.route_key ASC, e.seq ASC",
            )
            .expect("prepare folded entry read");
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    u64::try_from(row.get::<_, i64>(2)?).expect("seq"),
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            })
            .expect("folded entry rows");
        rows.collect::<Result<Vec<_>, _>>().expect("folded entries")
    };
    let mut folded_index = 0usize;
    let mut room_entries_total = 0usize;
    for route in &ground_truth_routes {
        let plane = match route.plane {
            finitechat_delivery::HttpDeliveryPlane::Group => "group",
            finitechat_delivery::HttpDeliveryPlane::Inbox => "inbox",
        };
        let mut folded_for_route = Vec::new();
        while folded_index < folded_routes.len()
            && folded_routes[folded_index].0 == plane
            && folded_routes[folded_index].1 == route.route_key
        {
            folded_for_route.push(folded_routes[folded_index].clone());
            folded_index += 1;
        }
        assert_eq!(
            folded_for_route.len(),
            route.entries.len(),
            "entry count for {plane} route {:?} differs after the fold",
            route.route_key
        );
        for (folded, queued) in folded_for_route.iter().zip(&route.entries) {
            assert_eq!(folded.2, queued.seq, "seq differs on {plane} route");
            assert_eq!(
                folded.3,
                queued.message.id.as_slice(),
                "message id differs on {plane} route at seq {}",
                queued.seq
            );
            assert_eq!(
                folded.4, queued.message.payload,
                "payload differs on {plane} route at seq {}",
                queued.seq
            );
        }
        if plane == "group" {
            room_entries_total += route.entries.len();
        }
    }
    assert_eq!(
        folded_index,
        folded_routes.len(),
        "every folded entry must be matched by an independently replayed route"
    );

    // 4. Inbox observability through the serving contract: every recipient
    //    with a durable welcome claim sees the same inbox the independent
    //    replay holds.
    let mut recipients = read_welcome_recipients(&legacy_path);
    recipients.sort_by(|left, right| left.as_slice().cmp(right.as_slice()));
    recipients.dedup();
    let mut inbox_recipients = 0usize;
    let mut inbox_entries_total = 0usize;
    for recipient in &recipients {
        let served = sync_all_inbox_entries(&app, recipient).await;
        let truth = ground_truth_routes
            .iter()
            .find(|route| {
                route.plane == finitechat_delivery::HttpDeliveryPlane::Inbox
                    && route.route_key == recipient.as_slice()
            })
            .map(|route| {
                route
                    .entries
                    .iter()
                    .map(|entry| (entry.seq, entry.message.payload.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert_eq!(
            served,
            truth,
            "inbox for {:?} differs after the fold",
            recipient.as_slice()
        );
        inbox_recipients += 1;
        inbox_entries_total += served.len();
    }

    // 5. KeyPackage state: per-owner inventory counts through the public
    //    contract. In synthetic mode the fold must reproduce the fixture's
    //    pre-defold counts exactly (the snapshot carried them verbatim); on
    //    a production copy the pre-fold cache may legitimately lag what the
    //    replay re-seeds, so only the coverage count is asserted below.
    let expected_inventory_counts = if synthetic {
        let conn = Connection::open(&source_path).expect("open source fixture");
        let mut statement = conn
            .prepare(
                "SELECT owner_json,
                        SUM(CASE WHEN state_json LIKE '%Available%' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN state_json LIKE '%Claimed%' THEN 1 ELSE 0 END)
                 FROM http_key_package_inventory GROUP BY owner_json",
            )
            .expect("prepare fixture inventory read");
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .expect("fixture inventory rows");
        rows.collect::<Result<Vec<_>, _>>()
            .expect("fixture inventory")
            .into_iter()
            .map(|(owner, available, claimed)| (owner, (available as u32, claimed as u32)))
            .collect::<std::collections::HashMap<String, (u32, u32)>>()
    } else {
        std::collections::HashMap::new()
    };
    let mut owners = read_inventory_owners(&legacy_path);
    owners.sort_by(|left, right| left.as_slice().cmp(right.as_slice()));
    owners.dedup();
    for owner in &owners {
        let response = post_json(
            app.clone(),
            "/key-packages/inventory",
            &KeyPackageInventoryRequest {
                owner: owner.clone(),
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let inventory: HttpKeyPackageInventory = read_json(response).await;
        if synthetic {
            let owner_key = serde_json::to_string(owner).expect("owner json");
            let expected = expected_inventory_counts
                .get(&owner_key)
                .copied()
                .unwrap_or((0, 0));
            assert_eq!(
                (inventory.available, inventory.claimed),
                expected,
                "inventory for owner {:?} differs after the fold",
                owner.as_slice()
            );
        }
    }

    let coverage = ReplayDiffCoverage {
        rooms: legacy_rooms.len(),
        room_entries: room_entries_total,
        inbox_recipients,
        inbox_entries: inbox_entries_total,
        inventory_owners: owners.len(),
        directory_rows: legacy_directory.len(),
    };
    eprintln!(
        "replay-diff: {} rooms, {} room entries, {} inbox recipients ({} entries), \
         {} inventory owners, {} directory rows — all observables equal",
        coverage.rooms,
        coverage.room_entries,
        coverage.inbox_recipients,
        coverage.inbox_entries,
        coverage.inventory_owners,
        coverage.directory_rows
    );
    // The synthetic fixture must actually exercise the shapes it exists for.
    if synthetic {
        assert!(
            coverage.room_entries > 3_000,
            "fixture must build thousands of ops"
        );
        assert!(coverage.rooms >= 3);
        assert!(coverage.directory_rows > 30);
    } else {
        // The verified production replay-diff (lat2 copy, 2026-08-31). If
        // these move, the fold changed — that is a review-level event.
        assert_eq!(coverage.rooms, 290, "rooms");
        assert_eq!(coverage.room_entries, 214_794, "room entries");
        assert_eq!(coverage.directory_rows, 545, "directory rows");
        assert_eq!(coverage.inventory_owners, 495, "inventory owners");
    }
}

/// Legacy era → fabricated pre-cutover shape → cutover boot (the fold). The
/// shared setup for the `rollback-check` guard tests below; returns the
/// number of group entries the legacy era wrote.
async fn fold_rollback_fixture(db_path: &std::path::Path) -> usize {
    let alice = DeviceRef::new("alice", "alice-laptop");
    let room_id = "room-rollback-check".to_owned();
    let mls_group_id = "mls-rollback-check".to_owned();
    let state = persistent_state(db_path);
    let app = http_router(state.clone());
    bootstrap_room(&app, &room_id, &mls_group_id, &alice).await;
    for index in 0..3 {
        let request = append_application_request(
            &room_id,
            &mls_group_id,
            &alice,
            0,
            format!("rollback-check message {index}").as_bytes(),
            &format!("rollback-check-msg-{index}"),
        );
        let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let entries = sync_all_group_entries(&app, &room_id).await.len();
    drop(app);
    drop(state);
    let total_ops = defold_into_legacy_shape(db_path, Some(2));
    assert!(total_ops > 2, "the fixture must have a post-snapshot tail");
    // Cutover boot: the fold runs here and records the pre-fold head.
    drop(persistent_state(db_path));
    entries
}

fn run_rollback_check_binary(db_path: &std::path::Path) -> (bool, serde_json::Value) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_finitechat-server"))
        .args(["rollback-check", "--sqlite"])
        .arg(db_path)
        .output()
        .expect("run finitechat-server rollback-check");
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let verdict: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("rollback-check prints one JSON line");
    (output.status.success(), verdict)
}

/// (a) Fold, then no writes: the deploy window. Restoring the pre-fold
/// backup rewinds no client, so the guard passes — from the library and
/// from the CLI (exit 0, JSON verdict).
#[tokio::test]
async fn rollback_check_passes_right_after_the_fold_with_no_writes() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    let entries = fold_rollback_fixture(&db_path).await;

    let check = finitechat_server::rollback_check(&db_path).expect("rollback check");
    assert!(check.fold_complete);
    assert!(check.rollback_allowed, "{}", check.reason);
    assert_eq!(check.pre_fold_head, Some(check.current_head));
    assert!(
        check.current_head >= entries as u64,
        "the head covers every folded group entry (plus inbox welcomes)"
    );
    assert!(check.reason.contains("no post-fold delivery writes"));

    // A plain steady-state boot (housekeeping writes, no delivery writes)
    // keeps the window open.
    drop(persistent_state(&db_path));
    let check = finitechat_server::rollback_check(&db_path).expect("rollback check");
    assert!(check.rollback_allowed, "{}", check.reason);

    let (ok, verdict) = run_rollback_check_binary(&db_path);
    assert!(ok, "the CLI exits 0 inside the window: {verdict}");
    assert_eq!(verdict["fold_complete"], true);
    assert_eq!(verdict["rollback_allowed"], true);
    assert_eq!(verdict["pre_fold_head"], verdict["current_head"]);
}

/// (b) Fold, then ONE accepted publish: a client may hold a cursor above the
/// pre-fold head, so the restore is refused (exit non-zero).
#[tokio::test]
async fn rollback_check_refuses_after_one_post_fold_publish() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    fold_rollback_fixture(&db_path).await;
    let before = finitechat_server::rollback_check(&db_path).expect("rollback check");
    assert!(before.rollback_allowed, "{}", before.reason);

    let alice = DeviceRef::new("alice", "alice-laptop");
    let state = persistent_state(&db_path);
    let app = http_router(state.clone());
    let request = append_application_request(
        "room-rollback-check",
        "mls-rollback-check",
        &alice,
        0,
        b"post-fold message",
        "rollback-check-post-fold",
    );
    let response = post_json(app.clone(), "/events", &typed_event_request(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    drop(app);
    drop(state);

    let check = finitechat_server::rollback_check(&db_path).expect("rollback check");
    assert!(check.fold_complete);
    assert!(!check.rollback_allowed);
    assert_eq!(check.pre_fold_head, before.pre_fold_head);
    assert_eq!(check.current_head, before.current_head + 1);
    assert!(
        check.reason.contains("post-fold writes exist"),
        "{}",
        check.reason
    );

    let (ok, verdict) = run_rollback_check_binary(&db_path);
    assert!(!ok, "the CLI exits non-zero once a post-fold write exists");
    assert_eq!(verdict["rollback_allowed"], false);
    assert_eq!(verdict["fold_complete"], true);
}

/// (c) Marker set but the head key missing (a database folded by a build
/// that did not record it): the pre-fold head is unknown, so fail closed.
#[tokio::test]
async fn rollback_check_refuses_when_the_pre_fold_head_is_unknown() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    fold_rollback_fixture(&db_path).await;
    {
        let conn = Connection::open(&db_path).expect("open raw");
        let deleted = conn
            .execute("DELETE FROM server_meta WHERE key = 'op_log_fold_head'", [])
            .expect("delete head key");
        assert_eq!(deleted, 1, "the fold recorded the head key");
    }

    let check = finitechat_server::rollback_check(&db_path).expect("rollback check");
    assert!(check.fold_complete);
    assert_eq!(check.pre_fold_head, None);
    assert!(!check.rollback_allowed);
    assert!(
        check.reason.contains("unknown pre-fold head"),
        "{}",
        check.reason
    );

    let (ok, verdict) = run_rollback_check_binary(&db_path);
    assert!(!ok);
    assert_eq!(verdict["pre_fold_head"], serde_json::Value::Null);
}

/// (d) A fresh database that never folded: there is no pre-fold backup, so
/// the restore is refused with the "no fold" reason.
#[tokio::test]
async fn rollback_check_refuses_an_unfolded_fresh_database() {
    let temp = TempDir::new().expect("tempdir");
    let db_path = temp.path().join("server.sqlite3");
    drop(persistent_state(&db_path));

    let check = finitechat_server::rollback_check(&db_path).expect("rollback check");
    assert!(!check.fold_complete);
    assert_eq!(check.pre_fold_head, None);
    assert!(!check.rollback_allowed);
    assert!(check.reason.starts_with("no fold"), "{}", check.reason);

    let (ok, verdict) = run_rollback_check_binary(&db_path);
    assert!(!ok);
    assert_eq!(verdict["fold_complete"], false);
    assert_eq!(verdict["rollback_allowed"], false);
}
