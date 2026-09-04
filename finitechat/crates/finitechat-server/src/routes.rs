//! Axum handlers and the HTTP router for the finite chat server.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path as AxumPath, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use finitechat_blob::BlobDescriptor;
use finitechat_delivery::{HttpClaimedKeyPackage, HttpKeyPackagePublication, HttpSyncPage};
use finitechat_http::{
    AckWelcomeRequest, AckWelcomeResponse, ApplicationEffectCountsResponse,
    ApplicationEffectRequest, BootstrapAccountRoomRequest, BootstrapAccountRoomResponse,
    ClaimKeyPackageForAccountRequest, ClaimKeyPackageRequest, ClaimKeyPackagesRequest,
    ClaimWelcomesRequest, DeviceLivenessRecord, ExpireKeyPackageLeaseRequest,
    ExpireKeyPackageLeaseResponse, FINITECHAT_SERVER_CONTRACT_VERSION, GetDeviceLivenessRequest,
    GetDeviceLivenessResponse, GetEphemeralActivitiesRequest, GetEphemeralActivitiesResponse,
    GetKeyPackageAvailabilityRequest, GetKeyPackageAvailabilityResponse, GetNostrProfilesRequest,
    GetNostrProfilesResponse, GroupSyncRequest, HealthResponse, HttpApplicationDeliveryEffect,
    HttpClaimedWelcome, HttpKeyPackageClaim, HttpKeyPackageInventory, InboxSyncRequest,
    KeyPackageInventoryRequest, LeaveRoomRequest, LeaveRoomResponse,
    ListAccountRoomDirectoryRequest, ListAccountRoomDirectoryResponse,
    ObserveDeviceLivenessRequest, PublishKeyPackageResponse, PutNostrProfileRequest,
    PutNostrProfileResponse, ReportInvalidCommitRequest, ReportInvalidCommitResponse,
    RevokeDeviceRequest, RevokeDeviceResponse, SaveAccountRoomRequest, SaveAccountRoomResponse,
    SyncHintEvent, SyncStreamRequest, SyncWaitRequest, SyncWaitResponse, UpdateRoomAdminsRequest,
    UpdateRoomAdminsResponse,
};
use finitechat_proto::{
    AppendApplicationEventRequest, AppendEphemeralActivityRequest, CommitAccepted,
    EphemeralActivityAccepted, EventAccepted, SubmitCommitRequest,
};
use serde::Serialize;

use crate::auth::SignedJson;
use crate::state::{
    HttpServerState, READINESS_BUDGET_MILLIS, ReadinessCheckResult, ServerReadiness,
    SyncStreamCursors, SyncStreamInboxCursor, SyncStreamLoop, SyncStreamRoomCursor,
};
use crate::validate::{
    DEFAULT_SYNC_STREAM_HEARTBEAT_MILLIS, MAX_SYNC_STREAM_HEARTBEAT_MILLIS, MAX_SYNC_WAIT_MILLIS,
    MIN_SYNC_STREAM_HEARTBEAT_MILLIS, validate_sync_stream_request, validate_sync_wait_request,
};
use crate::{MAX_HTTP_BLOB_UPLOAD_BODY_BYTES, ServerHttpError};

pub fn http_router(state: HttpServerState) -> Router {
    // Stranger-accessible mutating routes (no account secret exists to sign
    // them with) get a per-IP fixed-window rate limit instead of NIP-98 auth.
    let public_mutating = Router::new()
        .route(
            "/upload",
            put(upload_blob_object).layer(DefaultBodyLimit::max(MAX_HTTP_BLOB_UPLOAD_BODY_BYTES)),
        )
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
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_public_routes,
        ));

    Router::new()
        .route("/health", get(health))
        .route("/readyz", get(readyz))
        .route("/events", post(append_application_event))
        .route("/application-effects/get", post(get_application_effect))
        .route(
            "/application-effects/counts",
            post(get_application_effect_counts),
        )
        .route("/activities", post(append_ephemeral_activity))
        .route("/activities/get", post(get_ephemeral_activities))
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
        .route("/account-rooms/bootstrap", post(bootstrap_account_room))
        .route("/account-rooms", post(save_account_room))
        .route("/account-rooms/list", post(list_account_rooms))
        .route("/rooms/leave", post(leave_room))
        .route("/rooms/admins", post(update_room_admins))
        .route("/rooms/report-invalid-commit", post(report_invalid_commit))
        .route("/welcomes/claim", post(claim_welcomes))
        .route("/welcomes/ack", post(ack_welcome))
        .merge(public_mutating)
        .with_state(state)
}

async fn rate_limit_public_routes(
    State(state): State<HttpServerState>,
    request: Request,
    next: Next,
) -> Response {
    // Direct loopback callers (host-local services, no proxy in front) share
    // one address and one bucket; device-link export alone runs hundreds of
    // KeyPackage publishes a minute from exactly that path. They are trusted
    // host-local traffic and are exempt. Everything else — XFF-attributed
    // clients behind the host-local proxy, or direct remote peers — is
    // limited per address.
    match client_ip(&request) {
        Some(ip) if !state.check_public_route_rate_limit(ip) => {
            (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response()
        }
        _ => next.run(request).await,
    }
}

/// Client IP for rate limiting; `None` marks a direct loopback caller, which
/// is exempt (host-local traffic, no per-client address to attribute). The
/// edge proxy (Caddy) is host-local, so X-Forwarded-For is only trusted when
/// the direct peer is loopback; the trusted value is the LAST hop, the
/// address Caddy observed and appended — the first hop is attacker-
/// controlled whenever the client supplies the header. A non-loopback peer
/// is the client itself; XFF is ignored.
fn client_ip(request: &Request) -> Option<IpAddr> {
    client_ip_from(
        request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip()),
        request
            .headers()
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok()),
    )
}

fn client_ip_from(peer: Option<IpAddr>, x_forwarded_for: Option<&str>) -> Option<IpAddr> {
    let peer = peer.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    if !peer.is_loopback() {
        return Some(peer);
    }
    // No proxy attribution and the peer is the host itself: exempt rather
    // than bucket every local caller under 127.0.0.1.
    x_forwarded_for
        .and_then(|value| value.split(',').next_back())
        .map(str::trim)
        .filter(|hop| !hop.is_empty())
        .and_then(|hop| hop.parse().ok())
}

#[cfg(test)]
mod client_ip_tests {
    use super::client_ip_from;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn loopback_peer_trusts_only_the_last_forwarded_hop() {
        // Caddy appends the observed peer to any client-supplied header, so
        // the last hop is the real client even when the first is spoofed.
        assert_eq!(
            client_ip_from(
                Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
                Some("198.51.100.1, 203.0.113.9"),
            ),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)))
        );
    }

    #[test]
    fn non_loopback_peer_ignores_forwarded_header() {
        let peer = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7));
        assert_eq!(client_ip_from(Some(peer), Some("203.0.113.9")), Some(peer));
        assert_eq!(client_ip_from(Some(peer), None), Some(peer));
    }

    #[test]
    fn loopback_peer_without_header_is_exempt() {
        // Direct host-local callers share 127.0.0.1 and cannot be attributed
        // per client; they are exempt from the limiter entirely.
        assert_eq!(
            client_ip_from(Some(IpAddr::V6(Ipv6Addr::LOCALHOST)), None),
            None
        );
        assert_eq!(
            client_ip_from(Some(IpAddr::V4(Ipv4Addr::LOCALHOST)), None),
            None
        );
        // An unparsable header cannot attribute a client either; for a
        // loopback peer that means exemption, not a shared bucket.
        assert_eq!(
            client_ip_from(Some(IpAddr::V4(Ipv4Addr::LOCALHOST)), Some("not-an-ip")),
            None
        );
    }
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

#[derive(Serialize)]
struct ReadinessResponse {
    status: &'static str,
    budget_ms: u64,
    latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    checks: ReadinessChecksResponse,
}

#[derive(Serialize)]
struct ReadinessChecksResponse {
    delivery_core: ReadinessCheckResponse,
    durable_store: ReadinessCheckResponse,
}

#[derive(Serialize)]
struct ReadinessCheckResponse {
    status: &'static str,
    latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
}

async fn readyz(State(state): State<HttpServerState>) -> impl IntoResponse {
    let probe = tokio::task::spawn_blocking(move || state.probe_readiness()).await;
    let (status, body) = match probe {
        Ok(probe) => readiness_response(probe),
        Err(error) => {
            eprintln!("finitechat-server: readiness task failed: {error}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ReadinessResponse {
                    status: "unavailable",
                    budget_ms: READINESS_BUDGET_MILLIS,
                    latency_ms: 0,
                    reason: Some("probe_task_failed"),
                    checks: ReadinessChecksResponse {
                        delivery_core: failed_readiness_check("probe_task_failed"),
                        durable_store: failed_readiness_check("probe_task_failed"),
                    },
                },
            )
        }
    };
    (status, Json(body))
}

fn readiness_response(probe: ServerReadiness) -> (StatusCode, ReadinessResponse) {
    let ready = probe.is_ready();
    let reason = if probe.budget_exceeded {
        Some("readiness_budget_exceeded")
    } else if !ready {
        Some("component_failed")
    } else {
        None
    };
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        ReadinessResponse {
            status: if ready { "ready" } else { "unavailable" },
            budget_ms: READINESS_BUDGET_MILLIS,
            latency_ms: duration_millis(probe.elapsed),
            reason,
            checks: ReadinessChecksResponse {
                delivery_core: readiness_check_response(probe.delivery_core),
                durable_store: readiness_check_response(probe.durable_store),
            },
        },
    )
}

fn readiness_check_response(check: ReadinessCheckResult) -> ReadinessCheckResponse {
    ReadinessCheckResponse {
        status: if check.failure.is_none() {
            "ok"
        } else {
            "failed"
        },
        latency_ms: duration_millis(check.elapsed),
        reason: check.failure,
    }
}

fn failed_readiness_check(reason: &'static str) -> ReadinessCheckResponse {
    ReadinessCheckResponse {
        status: "failed",
        latency_ms: 0,
        reason: Some(reason),
    }
}

fn duration_millis(duration: std::time::Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn non_empty_build_value(value: Option<&'static str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

async fn append_application_event(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<AppendApplicationEventRequest>,
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
    SignedJson(request): SignedJson<AppendEphemeralActivityRequest>,
) -> Result<Json<EphemeralActivityAccepted>, ServerHttpError> {
    let response = state.append_ephemeral_activity(request)?;
    state.wake.notify_waiters();
    Ok(Json(response))
}

async fn get_ephemeral_activities(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<GetEphemeralActivitiesRequest>,
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
    SignedJson(request): SignedJson<SubmitCommitRequest>,
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
    SignedJson(request): SignedJson<RevokeDeviceRequest>,
) -> Result<Json<RevokeDeviceResponse>, ServerHttpError> {
    let response = state.revoke_device(request)?;
    Ok(Json(response))
}

async fn observe_device_liveness(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<ObserveDeviceLivenessRequest>,
) -> Result<Json<DeviceLivenessRecord>, ServerHttpError> {
    let response = state.observe_device_liveness(request)?;
    Ok(Json(response))
}

async fn get_device_liveness(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<GetDeviceLivenessRequest>,
) -> Result<Json<GetDeviceLivenessResponse>, ServerHttpError> {
    let response = state.get_device_liveness(request)?;
    Ok(Json(response))
}

async fn put_nostr_profile(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<PutNostrProfileRequest>,
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

async fn save_account_room(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<SaveAccountRoomRequest>,
) -> Result<Json<SaveAccountRoomResponse>, ServerHttpError> {
    let response = state.save_account_room(request)?;
    Ok(Json(response))
}

async fn bootstrap_account_room(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<BootstrapAccountRoomRequest>,
) -> Result<Json<BootstrapAccountRoomResponse>, ServerHttpError> {
    let response = state.bootstrap_account_room(request)?;
    Ok(Json(response))
}

async fn list_account_rooms(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<ListAccountRoomDirectoryRequest>,
) -> Result<Json<ListAccountRoomDirectoryResponse>, ServerHttpError> {
    let page = state.list_account_rooms(request)?;
    Ok(Json(page))
}

async fn leave_room(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<LeaveRoomRequest>,
) -> Result<Json<LeaveRoomResponse>, ServerHttpError> {
    let response = state.leave_room(request)?;
    Ok(Json(response))
}

async fn update_room_admins(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<UpdateRoomAdminsRequest>,
) -> Result<Json<UpdateRoomAdminsResponse>, ServerHttpError> {
    let response = state.update_room_admins(request)?;
    Ok(Json(response))
}

async fn report_invalid_commit(
    State(state): State<HttpServerState>,
    SignedJson(request): SignedJson<ReportInvalidCommitRequest>,
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
