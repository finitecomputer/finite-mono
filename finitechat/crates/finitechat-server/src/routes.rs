//! Axum handlers and the HTTP router for the finite chat server.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use finitechat_blob::BlobDescriptor;
use finitechat_delivery::{HttpClaimedKeyPackage, HttpKeyPackagePublication, HttpSyncPage};
use finitechat_http::{
    AckPushWakeRequest, AckPushWakeResponse, AckWelcomeRequest, AckWelcomeResponse,
    ApplicationEffectCountsResponse, ApplicationEffectRequest, BootstrapAccountRoomRequest,
    BootstrapAccountRoomResponse, ClaimKeyPackageForAccountRequest, ClaimKeyPackageRequest,
    ClaimKeyPackagesRequest, ClaimPushWakesRequest, ClaimPushWakesResponse, ClaimWelcomesRequest,
    CreatePairingSessionRequest, DeviceLivenessRecord, ExpireKeyPackageLeaseRequest,
    ExpireKeyPackageLeaseResponse, ExpirePairingSessionRequest, ExpirePairingSessionResponse,
    FINITECHAT_SERVER_CONTRACT_VERSION, FailPushWakeRequest, FailPushWakeResponse,
    GetDeviceLivenessRequest, GetDeviceLivenessResponse, GetEphemeralActivitiesRequest,
    GetEphemeralActivitiesResponse, GetKeyPackageAvailabilityRequest,
    GetKeyPackageAvailabilityResponse, GetNostrProfilesRequest, GetNostrProfilesResponse,
    GetPairingSessionRequest, GroupSyncRequest, HealthResponse, HttpApplicationDeliveryEffect,
    HttpClaimedWelcome, HttpKeyPackageClaim, HttpKeyPackageInventory, HttpPairingSessionRecord,
    InboxSyncRequest, KeyPackageInventoryRequest, LeaveRoomRequest, LeaveRoomResponse,
    ListAccountRoomDirectoryRequest, ListAccountRoomDirectoryResponse,
    ObserveDeviceLivenessRequest, PublishKeyPackageResponse, PublishPairingCompleteRequest,
    PublishPairingOfferRequest, PublishPairingResponseRequest, PutNostrProfileRequest,
    PutNostrProfileResponse, RegisterPushTokenRequest, RegisterPushTokenResponse,
    RemovePushTokenRequest, RemovePushTokenResponse, ReportInvalidCommitRequest,
    ReportInvalidCommitResponse, RevokeDeviceRequest, RevokeDeviceResponse, SaveAccountRoomRequest,
    SaveAccountRoomResponse, SyncHintEvent, SyncStreamRequest, SyncWaitRequest, SyncWaitResponse,
    UpdateRoomAdminsRequest, UpdateRoomAdminsResponse,
};
use finitechat_proto::{
    AppendApplicationEventRequest, AppendEphemeralActivityRequest, CommitAccepted,
    EphemeralActivityAccepted, EventAccepted, SubmitCommitRequest,
};

use crate::state::{
    HttpServerState, SyncStreamCursors, SyncStreamInboxCursor, SyncStreamLoop, SyncStreamRoomCursor,
};
use crate::validate::{
    DEFAULT_SYNC_STREAM_HEARTBEAT_MILLIS, MAX_SYNC_STREAM_HEARTBEAT_MILLIS, MAX_SYNC_WAIT_MILLIS,
    MIN_SYNC_STREAM_HEARTBEAT_MILLIS, validate_sync_stream_request, validate_sync_wait_request,
};
use crate::{MAX_HTTP_BLOB_UPLOAD_BODY_BYTES, ServerHttpError};

pub fn http_router(state: HttpServerState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/events", post(append_application_event))
        .route("/application-effects/get", post(get_application_effect))
        .route(
            "/application-effects/counts",
            post(get_application_effect_counts),
        )
        .route("/activities", post(append_ephemeral_activity))
        .route("/activities/get", post(get_ephemeral_activities))
        .route(
            "/upload",
            put(upload_blob_object).layer(DefaultBodyLimit::max(MAX_HTTP_BLOB_UPLOAD_BODY_BYTES)),
        )
        .route("/blobs/{sha256}", get(download_blob_object))
        .route("/commits", post(submit_commit))
        .route("/sync/group", post(sync_group))
        .route("/sync/inbox", post(sync_inbox))
        .route("/sync/stream", post(sync_stream))
        .route("/sync/wait", post(sync_wait))
        .route("/devices/revoke", post(revoke_device))
        .route("/devices/liveness", post(observe_device_liveness))
        .route("/devices/liveness/get", post(get_device_liveness))
        .route("/profiles/nostr", post(put_nostr_profile))
        .route("/profiles/nostr/get", post(get_nostr_profiles))
        .route(
            "/key-packages/availability",
            post(get_key_package_availability),
        )
        .route("/key-packages", post(publish_key_package))
        .route("/key-packages/inventory", post(key_package_inventory))
        .route("/key-packages/claim", post(claim_key_package))
        .route(
            "/key-packages/claim-account",
            post(claim_key_package_for_account),
        )
        .route("/key-packages/claims", post(claim_key_packages))
        .route(
            "/key-packages/leases/expire",
            post(expire_key_package_lease),
        )
        .route("/pairing-sessions", post(create_pairing_session))
        .route("/pairing-sessions/get", post(get_pairing_session))
        .route("/pairing-sessions/offer", post(publish_pairing_offer))
        .route("/pairing-sessions/response", post(publish_pairing_response))
        .route("/pairing-sessions/complete", post(publish_pairing_complete))
        .route("/pairing-sessions/expire", post(expire_pairing_session))
        .route("/account-rooms/bootstrap", post(bootstrap_account_room))
        .route("/account-rooms", post(save_account_room))
        .route("/account-rooms/list", post(list_account_rooms))
        .route("/push-tokens", post(register_push_token))
        .route("/push-tokens/remove", post(remove_push_token))
        .route("/push-wakes/claim", post(claim_push_wakes))
        .route("/push-wakes/ack", post(ack_push_wake))
        .route("/push-wakes/fail", post(fail_push_wake))
        .route("/rooms/leave", post(leave_room))
        .route("/rooms/admins", post(update_room_admins))
        .route("/rooms/report-invalid-commit", post(report_invalid_commit))
        .route("/welcomes/claim", post(claim_welcomes))
        .route("/welcomes/ack", post(ack_welcome))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_owned(),
        server_contract_version: Some(FINITECHAT_SERVER_CONTRACT_VERSION),
        server_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        source_fingerprint: non_empty_build_value(option_env!("FINITECHAT_BUILD_FINGERPRINT")),
        source_commit: non_empty_build_value(option_env!("FINITECHAT_BUILD_COMMIT")),
        source_branch: non_empty_build_value(option_env!("FINITECHAT_BUILD_BRANCH")),
        source_dirty: option_env!("FINITECHAT_BUILD_DIRTY").map(|value| value == "true"),
    })
}

fn non_empty_build_value(value: Option<&'static str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

async fn append_application_event(
    State(state): State<HttpServerState>,
    Json(request): Json<AppendApplicationEventRequest>,
) -> Result<Json<EventAccepted>, ServerHttpError> {
    let response = state.append_application_event(request)?;
    state.note_op_for_snapshot();
    state.wake.notify_waiters();
    Ok(Json(response))
}

async fn get_application_effect(
    State(state): State<HttpServerState>,
    Json(request): Json<ApplicationEffectRequest>,
) -> Result<Json<Option<HttpApplicationDeliveryEffect>>, ServerHttpError> {
    Ok(Json(state.application_effect(request)?))
}

async fn get_application_effect_counts(
    State(state): State<HttpServerState>,
) -> Result<Json<ApplicationEffectCountsResponse>, ServerHttpError> {
    Ok(Json(state.application_effect_counts()?))
}

async fn append_ephemeral_activity(
    State(state): State<HttpServerState>,
    Json(request): Json<AppendEphemeralActivityRequest>,
) -> Result<Json<EphemeralActivityAccepted>, ServerHttpError> {
    let response = state.append_ephemeral_activity(request)?;
    state.wake.notify_waiters();
    Ok(Json(response))
}

async fn get_ephemeral_activities(
    State(state): State<HttpServerState>,
    Json(request): Json<GetEphemeralActivitiesRequest>,
) -> Result<Json<GetEphemeralActivitiesResponse>, ServerHttpError> {
    Ok(Json(state.get_ephemeral_activities(request)?))
}

async fn upload_blob_object(
    State(state): State<HttpServerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<BlobDescriptor>, ServerHttpError> {
    // Blob payloads go to SQLite (hashing + fsync of up to 32 MiB): run on
    // the blocking pool so the async runtime is never parked on them.
    let descriptor = tokio::task::spawn_blocking(move || state.put_blob_object(&headers, &body))
        .await
        .expect("blob upload task")?;
    Ok(Json(descriptor))
}

async fn download_blob_object(
    State(state): State<HttpServerState>,
    AxumPath(sha256): AxumPath<String>,
) -> Result<impl IntoResponse, ServerHttpError> {
    // Payload reads come from SQLite with a per-read hash verification of up
    // to 32 MiB: blocking-pool work, same as uploads.
    let object = tokio::task::spawn_blocking(move || state.get_blob_object(&sha256))
        .await
        .expect("blob download task")?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, object.content_type)],
        object.bytes,
    ))
}

async fn submit_commit(
    State(state): State<HttpServerState>,
    Json(request): Json<SubmitCommitRequest>,
) -> Result<Json<CommitAccepted>, ServerHttpError> {
    let response = state.submit_commit(request)?;
    state.note_op_for_snapshot();
    state.wake.notify_waiters();
    Ok(Json(response))
}

async fn sync_group(
    State(state): State<HttpServerState>,
    Json(request): Json<GroupSyncRequest>,
) -> Result<Json<HttpSyncPage>, ServerHttpError> {
    Ok(Json(state.sync_group(request)?))
}

async fn sync_inbox(
    State(state): State<HttpServerState>,
    Json(request): Json<InboxSyncRequest>,
) -> Result<Json<HttpSyncPage>, ServerHttpError> {
    let page = state.sync_inbox(&request.recipient, request.after_seq, request.limit)?;
    Ok(Json(page))
}

async fn sync_stream(
    State(state): State<HttpServerState>,
    Json(request): Json<SyncStreamRequest>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ServerHttpError> {
    validate_sync_stream_request(&request)?;
    let heartbeat_ms = request
        .heartbeat_ms
        .unwrap_or(DEFAULT_SYNC_STREAM_HEARTBEAT_MILLIS)
        .clamp(
            MIN_SYNC_STREAM_HEARTBEAT_MILLIS,
            MAX_SYNC_STREAM_HEARTBEAT_MILLIS,
        );
    let cursors = SyncStreamCursors {
        rooms: request
            .rooms
            .into_iter()
            .map(|room| SyncStreamRoomCursor {
                seen_activity_received_at_ms: state.activity_highwater_for_room(&room.room_id),
                room_id: room.room_id,
                after_seq: room.after_seq,
            })
            .collect(),
        inbox: request.inbox.map(|inbox| SyncStreamInboxCursor {
            recipient: inbox.recipient,
            after_seq: inbox.after_seq,
        }),
    };
    let stream = futures_util::stream::unfold(
        SyncStreamLoop {
            state,
            cursors,
            pending: VecDeque::new(),
            heartbeat_ms,
        },
        |mut stream| async move {
            loop {
                if let Some(event) = stream.pending.pop_front() {
                    return Some((Ok(sync_sse_event(event)), stream));
                }

                stream
                    .pending
                    .extend(stream.state.collect_sync_hints(&mut stream.cursors));
                if let Some(event) = stream.pending.pop_front() {
                    return Some((Ok(sync_sse_event(event)), stream));
                }

                let wake = Arc::clone(&stream.state.wake);
                let notified = wake.notified();
                tokio::select! {
                    _ = notified => continue,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(stream.heartbeat_ms)) => {
                        return Some((Ok(sync_sse_event(SyncHintEvent::Heartbeat)), stream));
                    }
                }
            }
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn sync_sse_event(event: SyncHintEvent) -> Event {
    let name = match &event {
        SyncHintEvent::RoomAdvanced { .. } => "room_advanced",
        SyncHintEvent::ActivityChanged { .. } => "activity_changed",
        SyncHintEvent::InboxAdvanced { .. } => "inbox_advanced",
        SyncHintEvent::Heartbeat => "heartbeat",
    };
    Event::default()
        .event(name)
        .data(serde_json::to_string(&event).expect("SyncHintEvent serialization cannot fail"))
}

async fn sync_wait(
    State(state): State<HttpServerState>,
    Json(request): Json<SyncWaitRequest>,
) -> Result<Json<SyncWaitResponse>, ServerHttpError> {
    validate_sync_wait_request(&request)?;
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_millis(request.wait_ms.min(MAX_SYNC_WAIT_MILLIS));
    loop {
        // Arm the notification before checking so a publish that lands
        // between the check and the await still wakes this waiter.
        let notified = state.wake.notified();
        if let Some(reason) = state.check_wait_signal(&request) {
            return Ok(Json(SyncWaitResponse {
                woke: true,
                reason: Some(reason),
            }));
        }
        tokio::select! {
            _ = notified => continue,
            _ = tokio::time::sleep_until(deadline) => {
                return Ok(Json(SyncWaitResponse {
                    woke: false,
                    reason: None,
                }));
            }
        }
    }
}

async fn revoke_device(
    State(state): State<HttpServerState>,
    Json(request): Json<RevokeDeviceRequest>,
) -> Result<Json<RevokeDeviceResponse>, ServerHttpError> {
    let response = state.revoke_device(request)?;
    Ok(Json(response))
}

async fn observe_device_liveness(
    State(state): State<HttpServerState>,
    Json(request): Json<ObserveDeviceLivenessRequest>,
) -> Result<Json<DeviceLivenessRecord>, ServerHttpError> {
    let response = state.observe_device_liveness(request)?;
    Ok(Json(response))
}

async fn get_device_liveness(
    State(state): State<HttpServerState>,
    Json(request): Json<GetDeviceLivenessRequest>,
) -> Result<Json<GetDeviceLivenessResponse>, ServerHttpError> {
    let response = state.get_device_liveness(request)?;
    Ok(Json(response))
}

async fn put_nostr_profile(
    State(state): State<HttpServerState>,
    Json(request): Json<PutNostrProfileRequest>,
) -> Result<Json<PutNostrProfileResponse>, ServerHttpError> {
    let response = state.put_nostr_profile(request)?;
    Ok(Json(response))
}

async fn get_nostr_profiles(
    State(state): State<HttpServerState>,
    Json(request): Json<GetNostrProfilesRequest>,
) -> Result<Json<GetNostrProfilesResponse>, ServerHttpError> {
    let response = state.get_nostr_profiles(request)?;
    Ok(Json(response))
}

async fn get_key_package_availability(
    State(state): State<HttpServerState>,
    Json(request): Json<GetKeyPackageAvailabilityRequest>,
) -> Result<Json<GetKeyPackageAvailabilityResponse>, ServerHttpError> {
    let response = state.get_key_package_availability(request)?;
    Ok(Json(response))
}

async fn publish_key_package(
    State(state): State<HttpServerState>,
    Json(publication): Json<HttpKeyPackagePublication>,
) -> Result<Json<PublishKeyPackageResponse>, ServerHttpError> {
    let response = state.publish_key_package(publication)?;
    state.note_op_for_snapshot();
    Ok(Json(response))
}

async fn claim_key_package(
    State(state): State<HttpServerState>,
    Json(request): Json<ClaimKeyPackageRequest>,
) -> Result<Json<Option<HttpClaimedKeyPackage>>, ServerHttpError> {
    let claimed = state.claim_key_package(request)?;
    Ok(Json(claimed))
}

async fn claim_key_package_for_account(
    State(state): State<HttpServerState>,
    Json(request): Json<ClaimKeyPackageForAccountRequest>,
) -> Result<Json<Option<HttpClaimedKeyPackage>>, ServerHttpError> {
    let claimed = state.claim_key_package_for_account(request)?;
    Ok(Json(claimed))
}

async fn key_package_inventory(
    State(state): State<HttpServerState>,
    Json(request): Json<KeyPackageInventoryRequest>,
) -> Result<Json<HttpKeyPackageInventory>, ServerHttpError> {
    let inventory = state.key_package_inventory(request)?;
    Ok(Json(inventory))
}

async fn claim_key_packages(
    State(state): State<HttpServerState>,
    Json(request): Json<ClaimKeyPackagesRequest>,
) -> Result<Json<Vec<HttpKeyPackageClaim>>, ServerHttpError> {
    let claimed = state.claim_key_packages(request)?;
    Ok(Json(claimed))
}

async fn expire_key_package_lease(
    State(state): State<HttpServerState>,
    Json(request): Json<ExpireKeyPackageLeaseRequest>,
) -> Result<Json<ExpireKeyPackageLeaseResponse>, ServerHttpError> {
    let response = state.expire_key_package_lease(request)?;
    Ok(Json(response))
}

async fn create_pairing_session(
    State(state): State<HttpServerState>,
    Json(request): Json<CreatePairingSessionRequest>,
) -> Result<Json<HttpPairingSessionRecord>, ServerHttpError> {
    let record = state.create_pairing_session(request)?;
    Ok(Json(record))
}

async fn get_pairing_session(
    State(state): State<HttpServerState>,
    Json(request): Json<GetPairingSessionRequest>,
) -> Result<Json<Option<HttpPairingSessionRecord>>, ServerHttpError> {
    let record = state.get_pairing_session(request)?;
    Ok(Json(record))
}

async fn publish_pairing_offer(
    State(state): State<HttpServerState>,
    Json(request): Json<PublishPairingOfferRequest>,
) -> Result<Json<HttpPairingSessionRecord>, ServerHttpError> {
    let record = state.publish_pairing_offer(request)?;
    Ok(Json(record))
}

async fn publish_pairing_response(
    State(state): State<HttpServerState>,
    Json(request): Json<PublishPairingResponseRequest>,
) -> Result<Json<HttpPairingSessionRecord>, ServerHttpError> {
    let record = state.publish_pairing_response(request)?;
    Ok(Json(record))
}

async fn publish_pairing_complete(
    State(state): State<HttpServerState>,
    Json(request): Json<PublishPairingCompleteRequest>,
) -> Result<Json<HttpPairingSessionRecord>, ServerHttpError> {
    let record = state.publish_pairing_complete(request)?;
    Ok(Json(record))
}

async fn expire_pairing_session(
    State(state): State<HttpServerState>,
    Json(request): Json<ExpirePairingSessionRequest>,
) -> Result<Json<ExpirePairingSessionResponse>, ServerHttpError> {
    let response = state.expire_pairing_session(request)?;
    Ok(Json(response))
}

async fn save_account_room(
    State(state): State<HttpServerState>,
    Json(request): Json<SaveAccountRoomRequest>,
) -> Result<Json<SaveAccountRoomResponse>, ServerHttpError> {
    let response = state.save_account_room(request)?;
    Ok(Json(response))
}

async fn bootstrap_account_room(
    State(state): State<HttpServerState>,
    Json(request): Json<BootstrapAccountRoomRequest>,
) -> Result<Json<BootstrapAccountRoomResponse>, ServerHttpError> {
    let response = state.bootstrap_account_room(request)?;
    Ok(Json(response))
}

async fn list_account_rooms(
    State(state): State<HttpServerState>,
    Json(request): Json<ListAccountRoomDirectoryRequest>,
) -> Result<Json<ListAccountRoomDirectoryResponse>, ServerHttpError> {
    let page = state.list_account_rooms(request)?;
    Ok(Json(page))
}

async fn register_push_token(
    State(state): State<HttpServerState>,
    Json(request): Json<RegisterPushTokenRequest>,
) -> Result<Json<RegisterPushTokenResponse>, ServerHttpError> {
    let response = state.register_push_token(request)?;
    Ok(Json(response))
}

async fn remove_push_token(
    State(state): State<HttpServerState>,
    Json(request): Json<RemovePushTokenRequest>,
) -> Result<Json<RemovePushTokenResponse>, ServerHttpError> {
    let response = state.remove_push_token(request)?;
    Ok(Json(response))
}

async fn claim_push_wakes(
    State(state): State<HttpServerState>,
    Json(request): Json<ClaimPushWakesRequest>,
) -> Result<Json<ClaimPushWakesResponse>, ServerHttpError> {
    let response = state.claim_push_wakes(request)?;
    Ok(Json(response))
}

async fn ack_push_wake(
    State(state): State<HttpServerState>,
    Json(request): Json<AckPushWakeRequest>,
) -> Result<Json<AckPushWakeResponse>, ServerHttpError> {
    let response = state.ack_push_wake(request)?;
    Ok(Json(response))
}

async fn fail_push_wake(
    State(state): State<HttpServerState>,
    Json(request): Json<FailPushWakeRequest>,
) -> Result<Json<FailPushWakeResponse>, ServerHttpError> {
    let response = state.fail_push_wake(request)?;
    Ok(Json(response))
}

async fn leave_room(
    State(state): State<HttpServerState>,
    Json(request): Json<LeaveRoomRequest>,
) -> Result<Json<LeaveRoomResponse>, ServerHttpError> {
    let response = state.leave_room(request)?;
    Ok(Json(response))
}

async fn update_room_admins(
    State(state): State<HttpServerState>,
    Json(request): Json<UpdateRoomAdminsRequest>,
) -> Result<Json<UpdateRoomAdminsResponse>, ServerHttpError> {
    let response = state.update_room_admins(request)?;
    Ok(Json(response))
}

async fn report_invalid_commit(
    State(state): State<HttpServerState>,
    Json(request): Json<ReportInvalidCommitRequest>,
) -> Result<Json<ReportInvalidCommitResponse>, ServerHttpError> {
    let response = state.report_invalid_commit(request)?;
    Ok(Json(response))
}

async fn claim_welcomes(
    State(state): State<HttpServerState>,
    Json(request): Json<ClaimWelcomesRequest>,
) -> Result<Json<Vec<HttpClaimedWelcome>>, ServerHttpError> {
    let claimed = state.claim_welcomes(request)?;
    Ok(Json(claimed))
}

async fn ack_welcome(
    State(state): State<HttpServerState>,
    Json(request): Json<AckWelcomeRequest>,
) -> Result<Json<AckWelcomeResponse>, ServerHttpError> {
    let acked = state.ack_welcome(request)?;
    Ok(Json(acked))
}
