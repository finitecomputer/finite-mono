use finitechat_delivery::{
    HttpClaimedKeyPackage, HttpKeyPackageId, HttpPublishTarget, HttpSequence,
};
use finitechat_proto::{
    ApplicationDeliveryPolicy, DeviceRef, EphemeralActivityRecord, MembershipDeltaV1, RoomLogEntry,
    RoomProtocol,
};
use finitechat_transport::transport::TransportMessage;
use finitechat_transport::{GroupId, MemberId, MessageId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
/// Exact HTTP delivery/admission contract spoken by this build.
///
/// Bump this when client, Hermes bridge, or server behavior changes in a way
/// that must not silently interoperate with an older deployed server.
pub const FINITECHAT_SERVER_CONTRACT_VERSION: u32 = 4;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_contract_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_dirty: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishMessageRequest {
    pub target: HttpPublishTarget,
    pub message: TransportMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiniteAccountRoomCommitProjection {
    pub entry: RoomLogEntry,
    pub membership_delta: MembershipDeltaV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationEffectRequest {
    pub message_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpApplicationDeliveryEffect {
    pub room_id: String,
    pub seq: HttpSequence,
    pub message_id: String,
    pub sender: DeviceRef,
    pub delivery_policy: ApplicationDeliveryPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationEffectCountsResponse {
    pub push_outbox: u32,
    pub unread: u32,
    pub command_inbox: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushWakePayload {
    pub room_id: String,
    pub seq: HttpSequence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushWakeDelivery {
    pub wake_id: String,
    pub payload: PushWakePayload,
    pub tokens: Vec<PushTokenRecord>,
    pub attempt: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimPushWakesRequest {
    pub now_ms: u64,
    pub lease_ms: u64,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimPushWakesResponse {
    pub wakes: Vec<PushWakeDelivery>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckPushWakeRequest {
    pub wake_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckPushWakeResponse {
    pub acked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailPushWakeRequest {
    pub wake_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailPushWakeResponse {
    pub retry: bool,
    pub dropped: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSyncRequest {
    pub group_id: GroupId,
    pub after_seq: HttpSequence,
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester: Option<MemberId>,
}

/// Long-poll wake hint (ADR 0003 §5 wake contract over HTTP): returns when
/// any watched room log advances past the supplied cursor or when `wait_ms`
/// elapses. Purely advisory — hints never advance state; callers re-sync to
/// observe actual entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncWaitRequest {
    #[serde(default)]
    pub rooms: Vec<SyncWaitRoom>,
    pub wait_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncWaitRoom {
    pub room_id: String,
    pub after_seq: HttpSequence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncWaitResponse {
    pub woke: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// SSE wake-hint request. This watches the same scopes as `/sync/wait`, but
/// streams high-watermark hint events until the client disconnects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStreamRequest {
    #[serde(default)]
    pub rooms: Vec<SyncWaitRoom>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncHintEvent {
    RoomAdvanced {
        room_id: String,
        seq: HttpSequence,
    },
    ActivityChanged {
        room_id: String,
        received_at_ms: u64,
    },
    Heartbeat,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxSyncRequest {
    pub recipient: MemberId,
    pub after_seq: HttpSequence,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeDeviceRequest {
    pub device: DeviceRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeDeviceResponse {
    pub revoked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObserveDeviceLivenessRequest {
    pub device: DeviceRef,
    pub observed_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLivenessRecord {
    pub device: DeviceRef,
    pub observed_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetDeviceLivenessRequest {
    pub device: DeviceRef,
    pub now_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetDeviceLivenessResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<DeviceLivenessRecord>,
    pub live: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEphemeralActivitiesRequest {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub requester: DeviceRef,
    pub now_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEphemeralActivitiesResponse {
    #[serde(default)]
    pub records: Vec<EphemeralActivityRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NostrProfileRecord {
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finite_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_json: Option<String>,
    pub fetched_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutNostrProfileRequest {
    pub profile: NostrProfileRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutNostrProfileResponse {
    pub saved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetNostrProfilesRequest {
    pub account_ids: Vec<String>,
    pub now_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NostrProfileCacheEntry {
    pub profile: NostrProfileRecord,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetNostrProfilesResponse {
    pub profiles: Vec<NostrProfileCacheEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetKeyPackageAvailabilityRequest {
    pub account_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyPackageAvailabilityEntry {
    pub account_id: String,
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetKeyPackageAvailabilityResponse {
    pub accounts: Vec<KeyPackageAvailabilityEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimKeyPackageRequest {
    pub owner: MemberId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimKeyPackageForAccountRequest {
    pub account_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpireKeyPackageLeaseRequest {
    pub key_package_id: HttpKeyPackageId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpireKeyPackageLeaseResponse {
    pub expired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimKeyPackagesRequest {
    pub owners: Vec<MemberId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyPackageInventoryRequest {
    pub owner: MemberId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpKeyPackageInventory {
    pub owner: MemberId,
    pub available: u32,
    pub claimed: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpKeyPackageClaim {
    pub owner: MemberId,
    pub claimed: Option<HttpClaimedKeyPackage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateLinkSessionRequest {
    pub link_session_id: String,
    pub pairing_public_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetLinkSessionRequest {
    pub link_session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadLinkPayloadRequest {
    pub link_session_id: String,
    pub encrypted_payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimLinkPayloadRequest {
    pub link_session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimLinkPayloadResponse {
    pub encrypted_payload: Vec<u8>,
    pub claim_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckLinkPayloadRequest {
    pub link_session_id: String,
    pub claim_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckLinkPayloadResponse {
    pub acked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseLinkClaimRequest {
    pub link_session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseLinkClaimResponse {
    pub released: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpireLinkSessionRequest {
    pub link_session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpireLinkSessionResponse {
    pub expired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpLinkSessionRecord {
    pub link_session_id: String,
    pub pairing_public_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_payload: Option<Vec<u8>>,
    pub state: HttpLinkSessionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpLinkSessionState {
    Created,
    PayloadUploaded,
    Claimed,
    Delivered,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveAccountRoomRequest {
    pub account_id: String,
    pub room_id: String,
    pub record: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveAccountRoomResponse {
    pub saved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapAccountRoomRequest {
    pub room_id: String,
    pub mls_group_id: String,
    pub creator: DeviceRef,
    #[serde(default)]
    pub protocol: RoomProtocol,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapAccountRoomResponse {
    pub bootstrapped: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListAccountRoomDirectoryRequest {
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_room_id: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListAccountRoomDirectoryResponse {
    pub rooms: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after_room_id: Option<String>,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimWelcomesRequest {
    pub recipient: MemberId,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpClaimedWelcome {
    pub seq: HttpSequence,
    pub message: TransportMessage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckWelcomeRequest {
    pub message_id: MessageId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckWelcomeResponse {
    pub acked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushPlatform {
    Apns,
    Fcm,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterPushTokenRequest {
    pub device: DeviceRef,
    pub platform: PushPlatform,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterPushTokenResponse {
    pub registered: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovePushTokenRequest {
    pub device: DeviceRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovePushTokenResponse {
    pub removed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTokenRecord {
    pub device: DeviceRef,
    pub platform: PushPlatform,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaveRoomRequest {
    pub room_id: String,
    pub sender: DeviceRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaveRoomResponse {
    pub left: bool,
    pub departed_at_seq: HttpSequence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRoomAdminsRequest {
    pub room_id: String,
    pub sender: DeviceRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoke: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRoomAdminsResponse {
    pub admins: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportInvalidCommitRequest {
    pub room_id: String,
    pub reporter: DeviceRef,
    pub offending_seq: HttpSequence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportInvalidCommitResponse {
    pub reported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishKeyPackageResponse {
    pub published: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub kind: String,
    pub error: String,
}
