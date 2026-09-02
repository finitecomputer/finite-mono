use finitechat_delivery::{
    HttpClaimedKeyPackage, HttpKeyPackageId, HttpPublishTarget, HttpSequence,
};
use finitechat_proto::{
    AppendApplicationEventRequest, AppendEphemeralActivityRequest, ApplicationDeliveryPolicy,
    DeviceRef, EphemeralActivityRecord, MembershipDeltaV1, RoomLogEntry, RoomProtocol,
    SubmitCommitRequest,
};
use finitechat_transport::transport::TransportMessage;
use finitechat_transport::{GroupId, MemberId, MessageId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
/// Exact HTTP delivery/admission contract spoken by this build.
///
/// Bump this when client, Hermes bridge, or server behavior changes in a way
/// that must not silently interoperate with an older deployed server.
pub const FINITECHAT_SERVER_CONTRACT_VERSION: u32 = 6;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_contract_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_fingerprint: Option<String>,
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
    pub unread: u32,
    pub command_inbox: u32,
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

/// Per-device inbox cursor watched by the realtime hint stream. Inbox hints
/// carry no payload or authority: they only wake the normal bounded
/// claim/activate/ack sync path when a Welcome arrives for a Device that may
/// not belong to any rooms yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncWaitInbox {
    pub recipient: MemberId,
    pub after_seq: HttpSequence,
}

impl SyncWaitInbox {
    pub fn new(recipient: impl Into<Vec<u8>>, after_seq: HttpSequence) -> Self {
        Self {
            recipient: MemberId::new(recipient.into()),
            after_seq,
        }
    }
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
    pub inbox: Option<SyncWaitInbox>,
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
    InboxAdvanced {
        seq: HttpSequence,
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

/// Binds an account-scoped request body to the account id its NIP-98-style
/// `Authorization` header must be signed by. The server extractor rejects the
/// request when the event signer does not match this id.
pub trait AccountScopedRequest {
    fn signer_account_id(&self) -> &str;
}

impl AccountScopedRequest for AppendApplicationEventRequest {
    fn signer_account_id(&self) -> &str {
        &self.event.sender.account_id
    }
}

impl AccountScopedRequest for SubmitCommitRequest {
    fn signer_account_id(&self) -> &str {
        &self.sender.account_id
    }
}

impl AccountScopedRequest for AppendEphemeralActivityRequest {
    fn signer_account_id(&self) -> &str {
        &self.sender.account_id
    }
}

impl AccountScopedRequest for GetEphemeralActivitiesRequest {
    fn signer_account_id(&self) -> &str {
        &self.requester.account_id
    }
}

impl AccountScopedRequest for RevokeDeviceRequest {
    fn signer_account_id(&self) -> &str {
        &self.device.account_id
    }
}

impl AccountScopedRequest for ObserveDeviceLivenessRequest {
    fn signer_account_id(&self) -> &str {
        &self.device.account_id
    }
}

impl AccountScopedRequest for GetDeviceLivenessRequest {
    fn signer_account_id(&self) -> &str {
        &self.device.account_id
    }
}

impl AccountScopedRequest for PutNostrProfileRequest {
    fn signer_account_id(&self) -> &str {
        &self.profile.account_id
    }
}

impl AccountScopedRequest for BootstrapAccountRoomRequest {
    fn signer_account_id(&self) -> &str {
        &self.creator.account_id
    }
}

impl AccountScopedRequest for SaveAccountRoomRequest {
    fn signer_account_id(&self) -> &str {
        &self.account_id
    }
}

impl AccountScopedRequest for ListAccountRoomDirectoryRequest {
    fn signer_account_id(&self) -> &str {
        &self.account_id
    }
}

impl AccountScopedRequest for LeaveRoomRequest {
    fn signer_account_id(&self) -> &str {
        &self.sender.account_id
    }
}

impl AccountScopedRequest for UpdateRoomAdminsRequest {
    fn signer_account_id(&self) -> &str {
        &self.sender.account_id
    }
}

impl AccountScopedRequest for ReportInvalidCommitRequest {
    fn signer_account_id(&self) -> &str {
        &self.reporter.account_id
    }
}
