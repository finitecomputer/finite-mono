use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use finitechat_blob::{BLOB_CIPHERTEXT_CONTENT_TYPE, BlobDescriptor, BlobPutRequest};
use finitechat_delivery::{
    HTTP_SERVER_SOURCE, HttpClaimedKeyPackage, HttpCommitAdmission, HttpDeliveryLimits,
    HttpDeliveryService, HttpKeyPackageId, HttpKeyPackagePublication, HttpPublishCheck,
    HttpPublishReceipt, HttpPublishTarget, HttpSequence, HttpServerError, HttpSyncPage,
    MAX_HTTP_ID_BYTES, MAX_HTTP_SYNC_PAGE_ENTRIES,
};
pub use finitechat_http::{
    AckLinkPayloadRequest, AckLinkPayloadResponse, AckPushWakeRequest, AckPushWakeResponse,
    AckWelcomeRequest, AckWelcomeResponse, ApplicationEffectCountsResponse,
    ApplicationEffectRequest, BootstrapAccountRoomRequest, BootstrapAccountRoomResponse,
    ClaimKeyPackageForAccountRequest, ClaimKeyPackageRequest, ClaimKeyPackagesRequest,
    ClaimLinkPayloadRequest, ClaimLinkPayloadResponse, ClaimPushWakesRequest,
    ClaimPushWakesResponse, ClaimWelcomesRequest, CreateLinkSessionRequest, DeviceLivenessRecord,
    ErrorResponse, ExpireKeyPackageLeaseRequest, ExpireKeyPackageLeaseResponse,
    ExpireLinkSessionRequest, ExpireLinkSessionResponse, FINITECHAT_SERVER_CONTRACT_VERSION,
    FailPushWakeRequest, FailPushWakeResponse, FiniteAccountRoomCommitProjection,
    GetDeviceLivenessRequest, GetDeviceLivenessResponse, GetEphemeralActivitiesRequest,
    GetEphemeralActivitiesResponse, GetKeyPackageAvailabilityRequest,
    GetKeyPackageAvailabilityResponse, GetLinkSessionRequest, GetNostrProfilesRequest,
    GetNostrProfilesResponse, GroupSyncRequest, HealthResponse, HttpApplicationDeliveryEffect,
    HttpClaimedWelcome, HttpKeyPackageClaim, HttpKeyPackageInventory, HttpLinkSessionRecord,
    HttpLinkSessionState, InboxSyncRequest, KeyPackageAvailabilityEntry,
    KeyPackageInventoryRequest, LeaveRoomRequest, LeaveRoomResponse,
    ListAccountRoomDirectoryRequest, ListAccountRoomDirectoryResponse, NostrProfileCacheEntry,
    NostrProfileRecord, ObserveDeviceLivenessRequest, PublishKeyPackageResponse,
    PublishMessageRequest, PushTokenRecord, PushWakeDelivery, PushWakePayload,
    PutNostrProfileRequest, PutNostrProfileResponse, RegisterPushTokenRequest,
    RegisterPushTokenResponse, ReleaseLinkClaimRequest, ReleaseLinkClaimResponse,
    RemovePushTokenRequest, RemovePushTokenResponse, ReportInvalidCommitRequest,
    ReportInvalidCommitResponse, RevokeDeviceRequest, RevokeDeviceResponse, SaveAccountRoomRequest,
    SaveAccountRoomResponse, SyncHintEvent, SyncStreamRequest, SyncWaitRequest, SyncWaitResponse,
    UpdateRoomAdminsRequest, UpdateRoomAdminsResponse, UploadLinkPayloadRequest,
};
use finitechat_proto::{
    AccountRoomDevice, AccountRoomRecord, AppendApplicationEventRequest,
    AppendEphemeralActivityRequest, AppendEventRequest, CommitAccepted, DeviceMembership,
    EphemeralActivityAccepted, EphemeralActivityRecord, EventAccepted, MembershipInterval,
    SubmitCommitRequest, UploadKeyPackageRequest, WelcomeRecord, delivery_member_id_for_device,
    lease_token_for, staged_welcomes_by_id, validate_activity_expiry,
};
use finitechat_proto::{
    DeviceRef, LogEntryKind, MAX_ACCOUNT_DEVICES_PER_ROOM, MAX_ATTACHMENT_CIPHERTEXT_BYTES,
    MAX_DEVICE_LIVENESS_EXPIRY_MILLIS, MAX_EPHEMERAL_ACTIVITY_CACHE_ENTRIES_PER_ROUTE,
    MAX_KEY_PACKAGES_PER_DEVICE, MAX_LINK_SESSION_PAYLOAD_BYTES, MAX_OBJECT_ID_BYTES,
    MIN_SUPPORTED_PROTOCOL_VERSION, MembershipAddV1, MembershipDeltaV1, PROTOCOL_VERSION_V1,
    RoomLogEntry, RoomProtocol, RoomStatus, WelcomeState, validate_bytes_len,
    validate_bytes_non_empty, validate_string_bytes,
};
use finitechat_transport::engine::KeyPackage;
use finitechat_transport::transport::{
    Timestamp, TransportEnvelope, TransportMessage, TransportSource,
};
use finitechat_transport::{EpochId, GroupId, MemberId, MessageId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

const MAX_HTTP_ACCOUNT_ROOM_ID_BYTES: usize = 128;
const MAX_HTTP_BLOB_UPLOAD_BODY_BYTES: usize = MAX_ATTACHMENT_CIPHERTEXT_BYTES as usize;
const MAX_KEY_PACKAGE_AVAILABILITY_BATCH: usize = MAX_HTTP_SYNC_PAGE_ENTRIES;
const MAX_NOSTR_PROFILE_BATCH: usize = 64;
const MAX_NOSTR_PROFILE_NAME_BYTES: usize = 128;
const MAX_NOSTR_PROFILE_ABOUT_BYTES: usize = 4 * 1024;
const MAX_NOSTR_PROFILE_PICTURE_BYTES: usize = 2 * 1024;
const MAX_NOSTR_PROFILE_METADATA_JSON_BYTES: usize = 16 * 1024;
const MAX_PUBLIC_IMAGE_BLOB_BYTES: usize = 8 * 1024 * 1024;
const MAX_PUSH_WAKE_CLAIM_BATCH: usize = 100;
const MAX_PUSH_WAKE_LEASE_MS: u64 = 5 * 60 * 1_000;
const MAX_PUSH_WAKE_ATTEMPTS: u32 = 5;

/// Capacity limits for the durable finite chat server.
///
/// The upstream defaults are sized for tests. These are sized for the current
/// product phase (hundreds of active users, dozens of long chats each); they
/// must be applied before op-log replay so reopening a large server never
/// trips a smaller cap than the one it was written under.
/// How many accepted operations may accumulate before the durable state
/// snapshot refreshes. Startup replays at most this many ops on top of the
/// snapshot.
const SNAPSHOT_INTERVAL_OPS: u64 = 4_096;

pub fn finite_delivery_limits() -> HttpDeliveryLimits {
    HttpDeliveryLimits {
        max_groups: 65_536,
        max_recipient_inboxes: 65_536,
        max_queue_entries_per_route: 262_144,
        max_key_packages_per_account: 4_096,
    }
}

#[derive(Clone, Debug, Default)]
pub struct HttpServerState {
    service: Arc<Mutex<HttpDeliveryService>>,
    publish_idempotency: Arc<Mutex<HashMap<String, PublishIdempotencyRecord>>>,
    key_package_claim_idempotency: Arc<Mutex<HashMap<String, KeyPackageClaimIdempotencyRecord>>>,
    key_package_inventory: Arc<Mutex<HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>>>,
    revoked_devices: Arc<Mutex<BTreeSet<String>>>,
    link_sessions: Arc<Mutex<BTreeMap<String, HttpLinkSessionRecord>>>,
    account_rooms: Arc<Mutex<BTreeMap<String, BTreeMap<String, Value>>>>,
    room_memberships: Arc<Mutex<BTreeMap<String, HttpRoomMembershipProjection>>>,
    application_effects: Arc<Mutex<BTreeMap<String, HttpApplicationDeliveryEffect>>>,
    ephemeral_activity: Arc<Mutex<BTreeMap<String, Vec<EphemeralActivityRecord>>>>,
    device_liveness: Arc<Mutex<BTreeMap<String, DeviceLivenessRecord>>>,
    nostr_profiles: Arc<Mutex<BTreeMap<String, NostrProfileRecord>>>,
    welcome_claims: Arc<Mutex<HashMap<MessageId, WelcomeClaimRecord>>>,
    push_tokens: Arc<Mutex<BTreeMap<String, PushTokenRecord>>>,
    push_wakes: Arc<Mutex<BTreeMap<String, PushWakeOutboxRecord>>>,
    blob_objects: Arc<Mutex<BTreeMap<String, BlobObject>>>,
    /// Canonical externally reachable origin used in durable blob references.
    /// Request-derived hosts remain the local-development fallback only.
    public_url: Option<String>,
    ops_since_snapshot: Arc<Mutex<u64>>,
    /// Long-poll wake signal (/sync/wait). A single hub: every accepted publish
    /// wakes all waiters, who re-check their own predicates. Sized for the
    /// current phase (hundreds of users); per-key channels are the documented
    /// next step if waiter counts grow.
    wake: Arc<tokio::sync::Notify>,
    store: Option<Arc<SqliteHttpDeliveryStore>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BlobObject {
    content_type: String,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct SyncStreamCursors {
    rooms: Vec<SyncStreamRoomCursor>,
    inbox: Option<SyncStreamInboxCursor>,
}

#[derive(Clone)]
struct SyncStreamRoomCursor {
    room_id: String,
    after_seq: u64,
    seen_activity_received_at_ms: u64,
}

#[derive(Clone)]
struct SyncStreamInboxCursor {
    recipient: MemberId,
    after_seq: u64,
}

struct SyncStreamLoop {
    state: HttpServerState,
    cursors: SyncStreamCursors,
    pending: VecDeque<SyncHintEvent>,
    heartbeat_ms: u64,
}

impl HttpServerState {
    pub fn new(service: HttpDeliveryService) -> Self {
        Self {
            service: Arc::new(Mutex::new(service)),
            publish_idempotency: Arc::new(Mutex::new(HashMap::new())),
            key_package_claim_idempotency: Arc::new(Mutex::new(HashMap::new())),
            key_package_inventory: Arc::new(Mutex::new(HashMap::new())),
            revoked_devices: Arc::new(Mutex::new(BTreeSet::new())),
            link_sessions: Arc::new(Mutex::new(BTreeMap::new())),
            account_rooms: Arc::new(Mutex::new(BTreeMap::new())),
            room_memberships: Arc::new(Mutex::new(BTreeMap::new())),
            application_effects: Arc::new(Mutex::new(BTreeMap::new())),
            ephemeral_activity: Arc::new(Mutex::new(BTreeMap::new())),
            device_liveness: Arc::new(Mutex::new(BTreeMap::new())),
            nostr_profiles: Arc::new(Mutex::new(BTreeMap::new())),
            welcome_claims: Arc::new(Mutex::new(HashMap::new())),
            push_tokens: Arc::new(Mutex::new(BTreeMap::new())),
            push_wakes: Arc::new(Mutex::new(BTreeMap::new())),
            blob_objects: Arc::new(Mutex::new(BTreeMap::new())),
            public_url: None,
            ops_since_snapshot: Arc::new(Mutex::new(0)),
            wake: Arc::new(tokio::sync::Notify::new()),
            store: None,
        }
    }

    pub fn with_public_url(
        mut self,
        public_url: impl AsRef<str>,
    ) -> Result<Self, HttpServerConfigurationError> {
        self.public_url = Some(normalize_public_url(public_url.as_ref())?);
        Ok(self)
    }

    pub fn from_sqlite_path(path: impl AsRef<Path>) -> Result<Self, DurableStoreError> {
        let store = Arc::new(SqliteHttpDeliveryStore::open(path)?);
        // Boot from the latest snapshot plus the operation-log tail; full
        // replay only happens for stores that have never snapshotted.
        let (mut service, mut key_package_inventory, mut revoked_devices, snapshot_seq) =
            match store.load_state_snapshot()? {
                Some((seq, snapshot)) => (
                    snapshot.service,
                    snapshot
                        .key_package_inventory
                        .into_iter()
                        .map(|record| (record.key_package_id.clone(), record))
                        .collect(),
                    snapshot.revoked_devices,
                    seq,
                ),
                None => (
                    HttpDeliveryService::with_limits(finite_delivery_limits()),
                    HashMap::new(),
                    BTreeSet::new(),
                    0,
                ),
            };
        let operations = store.load_operations_after(snapshot_seq)?;
        for operation in operations.iter().cloned() {
            replay_operation(&mut service, operation)?;
        }
        apply_operations_to_key_package_inventory(&mut key_package_inventory, &operations);
        apply_operations_to_revoked_devices(&mut revoked_devices, &operations);
        let publish_idempotency = store.load_publish_idempotency()?;
        let key_package_claim_idempotency = store.load_key_package_claim_idempotency()?;
        if snapshot_seq == 0
            && !key_package_inventory_cache_matches(
                &store.load_key_package_inventory()?,
                &key_package_inventory,
            )
        {
            for record in key_package_inventory.values() {
                store.upsert_key_package_inventory(record)?;
            }
        }
        let link_sessions = store.load_link_sessions()?;
        let account_rooms = store.load_account_room_directory()?;
        let room_memberships = store.load_room_memberships()?;
        let application_effects = store.load_application_effects()?;
        let nostr_profiles = store.load_nostr_profiles()?;
        let welcome_claims = store.load_welcome_claims()?;
        let push_tokens = store.load_push_tokens()?;
        let push_wakes = store.load_push_wakes()?;
        let blob_objects = store.load_blob_objects()?;
        Ok(Self {
            service: Arc::new(Mutex::new(service)),
            publish_idempotency: Arc::new(Mutex::new(publish_idempotency)),
            key_package_claim_idempotency: Arc::new(Mutex::new(key_package_claim_idempotency)),
            key_package_inventory: Arc::new(Mutex::new(key_package_inventory)),
            revoked_devices: Arc::new(Mutex::new(revoked_devices)),
            link_sessions: Arc::new(Mutex::new(link_sessions)),
            account_rooms: Arc::new(Mutex::new(account_rooms)),
            room_memberships: Arc::new(Mutex::new(room_memberships)),
            application_effects: Arc::new(Mutex::new(application_effects)),
            ephemeral_activity: Arc::new(Mutex::new(BTreeMap::new())),
            device_liveness: Arc::new(Mutex::new(BTreeMap::new())),
            nostr_profiles: Arc::new(Mutex::new(nostr_profiles)),
            welcome_claims: Arc::new(Mutex::new(welcome_claims)),
            push_tokens: Arc::new(Mutex::new(push_tokens)),
            push_wakes: Arc::new(Mutex::new(push_wakes)),
            blob_objects: Arc::new(Mutex::new(blob_objects)),
            public_url: None,
            ops_since_snapshot: Arc::new(Mutex::new(0)),
            wake: Arc::new(tokio::sync::Notify::new()),
            store: Some(store),
        })
    }

    pub fn put_blob_object(
        &self,
        headers: &HeaderMap,
        bytes: &[u8],
    ) -> Result<BlobDescriptor, ServerHttpError> {
        let content_type = normalize_blob_upload_content_type(blob_content_type(headers)?)?;
        validate_blob_upload(bytes, content_type)?;

        let sha256 = sha256_hex(bytes);
        let mut objects = self.blob_objects.lock().expect("HTTP blob objects mutex");
        if let Some(existing) = objects.get(&sha256) {
            if existing.bytes.as_slice() == bytes {
                return Ok(BlobDescriptor {
                    url: blob_url(self.public_url.as_deref(), headers, &sha256),
                    sha256,
                    size_bytes: bytes.len() as u64,
                });
            }
            return Err(ServerHttpError::BlobConflict { sha256 });
        }

        if let Some(store) = &self.store {
            store.upsert_blob_object(&sha256, content_type, bytes)?;
        }
        objects.insert(
            sha256.clone(),
            BlobObject {
                content_type: content_type.to_owned(),
                bytes: bytes.to_vec(),
            },
        );
        Ok(BlobDescriptor {
            url: blob_url(self.public_url.as_deref(), headers, &sha256),
            sha256,
            size_bytes: bytes.len() as u64,
        })
    }

    fn get_blob_object(&self, sha256: &str) -> Result<BlobObject, ServerHttpError> {
        validate_blob_sha256(sha256)?;
        let objects = self.blob_objects.lock().expect("HTTP blob objects mutex");
        objects
            .get(sha256)
            .cloned()
            .ok_or_else(|| ServerHttpError::BlobNotFound {
                sha256: sha256.to_owned(),
            })
    }

    /// Raw delivery-contract publish, also used by the shared delivery
    /// conformance suite against this durable server.
    pub fn publish_message(
        &self,
        request: PublishMessageRequest,
    ) -> Result<HttpPublishReceipt, ServerHttpError> {
        self.validate_raw_commit_import(&request)?;
        let Some(idempotency_key) = request.idempotency_key.clone() else {
            let mut service = self.service.lock().expect("HTTP delivery service mutex");
            let receipt = match service.check_publish(&request.target, &request.message)? {
                HttpPublishCheck::DuplicateReplay(receipt) => return Ok(receipt),
                HttpPublishCheck::Fresh(receipt) => receipt,
            };
            if let Some(store) = &self.store {
                store.append_operation(&PersistedOperation::PublishMessage {
                    target: request.target.clone(),
                    message: request.message.clone(),
                    idempotency_key: None,
                })?;
            }
            // The dry run admitted this publish under the held lock, so the
            // apply cannot fail; `?` keeps the impossible path a 500 instead
            // of a panic.
            let published = service.publish(request.target, request.message)?;
            debug_assert_eq!(published, receipt);
            return Ok(published);
        };

        if idempotency_key.is_empty() {
            return Err(ServerHttpError::InvalidIdempotencyKey);
        }

        let fingerprint = PublishMessageFingerprint::from_request(&request);
        let mut service = self.service.lock().expect("HTTP delivery service mutex");
        let mut idempotency = self
            .publish_idempotency
            .lock()
            .expect("HTTP publish idempotency mutex");
        if let Some(record) = idempotency.get(&idempotency_key) {
            if record.fingerprint == fingerprint {
                return Ok(record.receipt.clone());
            }
            return Err(ServerHttpError::IdempotencyConflict { idempotency_key });
        }

        let receipt = match service.check_publish(&request.target, &request.message)? {
            HttpPublishCheck::DuplicateReplay(receipt) => receipt,
            HttpPublishCheck::Fresh(receipt) => receipt,
        };
        let operation = (!receipt.duplicate).then_some(PersistedOperation::PublishMessage {
            target: request.target.clone(),
            message: request.message.clone(),
            idempotency_key: Some(idempotency_key.clone()),
        });
        let record = PublishIdempotencyRecord {
            fingerprint,
            receipt: receipt.clone(),
        };
        if let Some(store) = &self.store {
            store.append_publish_mutation(operation.as_ref(), Some((&idempotency_key, &record)))?;
        }
        if !receipt.duplicate {
            let published = service.publish(request.target, request.message)?;
            debug_assert_eq!(published, receipt);
        }
        idempotency.insert(idempotency_key, record);
        Ok(receipt)
    }

    fn validate_raw_commit_import(
        &self,
        request: &PublishMessageRequest,
    ) -> Result<(), ServerHttpError> {
        if !matches!(&request.target, HttpPublishTarget::Group { .. })
            || serde_json::from_slice::<FiniteAccountRoomCommitProjection>(&request.message.payload)
                .is_ok()
        {
            return Ok(());
        }
        let Some(entry) = room_log_entry_from_payload(&request.message.payload) else {
            return Ok(());
        };
        if entry.kind != LogEntryKind::Commit
            || entry.envelope.kind != LogEntryKind::Commit
            || entry.envelope.room_id != entry.room_id
            || request.message.id.as_slice() != entry.message_id.as_bytes()
        {
            return Ok(());
        }

        let rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let Some(projection) = rooms.get(&entry.room_id) else {
            return Ok(());
        };
        if projection.mls_group_id == entry.envelope.mls_group_id && projection.membership_complete
        {
            return Err(ServerHttpError::InvalidRawCommitImport {
                room_id: entry.room_id,
                reason: "raw commit import for a typed room must carry membership_delta projection"
                    .to_owned(),
            });
        }
        Ok(())
    }

    pub fn publish_key_package(
        &self,
        publication: HttpKeyPackagePublication,
    ) -> Result<PublishKeyPackageResponse, ServerHttpError> {
        if let Some(metadata) = finite_key_package_metadata(&publication) {
            self.ensure_device_not_revoked(&metadata.owner)?;
        }
        let mut inventory = self
            .key_package_inventory
            .lock()
            .expect("HTTP KeyPackage inventory mutex");
        let mut candidate = inventory.clone();
        let Some(_) = record_key_package_publication(&mut candidate, &publication)? else {
            return Ok(PublishKeyPackageResponse { published: true });
        };
        let changed = changed_key_package_inventory_records(&inventory, &candidate);
        let operation = PersistedOperation::PublishKeyPackage { publication };
        if let Some(store) = &self.store {
            store.append_key_package_claim_mutation(Some(&operation), None, &changed)?;
        }
        *inventory = candidate;
        Ok(PublishKeyPackageResponse { published: true })
    }

    pub fn claim_key_package(
        &self,
        request: ClaimKeyPackageRequest,
    ) -> Result<Option<HttpClaimedKeyPackage>, ServerHttpError> {
        let mut inventory = self
            .key_package_inventory
            .lock()
            .expect("HTTP KeyPackage inventory mutex");
        let revoked_devices = self.revoked_device_keys();
        if let Some(device) = available_finite_owner_revoked_in_inventory(
            &inventory,
            &request.owner,
            &revoked_devices,
        ) {
            return Err(ServerHttpError::DeviceRevoked { device });
        }
        let mut candidate = inventory.clone();
        let claimed =
            claim_next_key_package_from_inventory(&mut candidate, &request.owner, &revoked_devices);
        let changed = claimed
            .as_ref()
            .and_then(|package| candidate.get(&package.key_package_id).cloned());
        let changed = changed.into_iter().collect::<Vec<_>>();
        let operation = claimed
            .is_some()
            .then_some(PersistedOperation::ClaimKeyPackage {
                owner: request.owner,
            });
        if let Some(store) = &self.store {
            store.append_key_package_claim_mutation(
                operation.as_ref(),
                None,
                changed.as_slice(),
            )?;
        }
        *inventory = candidate;
        Ok(claimed)
    }

    pub fn claim_key_package_for_account(
        &self,
        request: ClaimKeyPackageForAccountRequest,
    ) -> Result<Option<HttpClaimedKeyPackage>, ServerHttpError> {
        validate_key_package_availability_account_id(&request.account_id)?;
        let mut inventory = self
            .key_package_inventory
            .lock()
            .expect("HTTP KeyPackage inventory mutex");
        let revoked_devices = self.revoked_device_keys();
        let mut candidate = inventory.clone();
        let claimed = claim_next_key_package_for_account_from_inventory(
            &mut candidate,
            &request.account_id,
            &revoked_devices,
        );
        let changed = claimed
            .as_ref()
            .and_then(|package| candidate.get(&package.key_package_id).cloned());
        let changed = changed.into_iter().collect::<Vec<_>>();
        let operation = claimed
            .as_ref()
            .map(|package| PersistedOperation::ClaimKeyPackage {
                owner: package.owner.clone(),
            });
        if let Some(store) = &self.store {
            store.append_key_package_claim_mutation(
                operation.as_ref(),
                None,
                changed.as_slice(),
            )?;
        }
        *inventory = candidate;
        Ok(claimed)
    }

    fn claim_key_packages(
        &self,
        request: ClaimKeyPackagesRequest,
    ) -> Result<Vec<HttpKeyPackageClaim>, ServerHttpError> {
        validate_key_package_claim_batch(&request.owners)?;
        let Some(idempotency_key) = request.idempotency_key.clone() else {
            let mut inventory = self
                .key_package_inventory
                .lock()
                .expect("HTTP KeyPackage inventory mutex");
            let revoked_devices = self.revoked_device_keys();
            let mut candidate = inventory.clone();
            let claims = claim_key_packages_from_inventory(
                &mut candidate,
                &request.owners,
                &revoked_devices,
            );
            let changed = key_package_claim_inventory_records(&candidate, &claims);
            let operation = claims
                .iter()
                .any(|claim| claim.claimed.is_some())
                .then_some(PersistedOperation::ClaimKeyPackages {
                    owners: request.owners,
                });
            if let Some(store) = &self.store {
                store.append_key_package_claim_mutation(
                    operation.as_ref(),
                    None,
                    changed.as_slice(),
                )?;
            }
            *inventory = candidate;
            return Ok(claims);
        };

        if idempotency_key.is_empty() {
            return Err(ServerHttpError::InvalidIdempotencyKey);
        }

        let fingerprint = KeyPackageClaimFingerprint {
            owners: request.owners.clone(),
        };
        let mut inventory = self
            .key_package_inventory
            .lock()
            .expect("HTTP KeyPackage inventory mutex");
        let revoked_devices = self.revoked_device_keys();
        let mut idempotency = self
            .key_package_claim_idempotency
            .lock()
            .expect("HTTP KeyPackage claim idempotency mutex");
        if let Some(record) = idempotency.get(&idempotency_key) {
            if record.fingerprint == fingerprint {
                return Ok(record.response.clone());
            }
            return Err(ServerHttpError::IdempotencyConflict { idempotency_key });
        }

        let mut candidate = inventory.clone();
        let claims =
            claim_key_packages_from_inventory(&mut candidate, &request.owners, &revoked_devices);
        let changed = key_package_claim_inventory_records(&candidate, &claims);
        let operation = claims
            .iter()
            .any(|claim| claim.claimed.is_some())
            .then_some(PersistedOperation::ClaimKeyPackages {
                owners: request.owners,
            });
        let record = KeyPackageClaimIdempotencyRecord {
            fingerprint,
            response: claims.clone(),
        };
        if let Some(store) = &self.store {
            store.append_key_package_claim_mutation(
                operation.as_ref(),
                Some((&idempotency_key, &record)),
                changed.as_slice(),
            )?;
        }
        *inventory = candidate;
        idempotency.insert(idempotency_key, record);
        Ok(claims)
    }

    fn expire_key_package_lease(
        &self,
        request: ExpireKeyPackageLeaseRequest,
    ) -> Result<ExpireKeyPackageLeaseResponse, ServerHttpError> {
        let mut inventory = self
            .key_package_inventory
            .lock()
            .expect("HTTP KeyPackage inventory mutex");
        let mut candidate = inventory.clone();
        let record = candidate.get_mut(&request.key_package_id).ok_or_else(|| {
            ServerHttpError::InvalidKeyPackageLeaseRequest {
                reason: format!("KeyPackage {:?} was not published", request.key_package_id),
            }
        })?;
        match record.state {
            KeyPackageInventoryState::Claimed => {
                record.state = KeyPackageInventoryState::Available;
            }
            KeyPackageInventoryState::Available => {
                return Err(ServerHttpError::InvalidKeyPackageLeaseRequest {
                    reason: format!("KeyPackage {:?} is not claimed", request.key_package_id),
                });
            }
            KeyPackageInventoryState::Consumed => {
                return Err(ServerHttpError::InvalidKeyPackageLeaseRequest {
                    reason: format!(
                        "KeyPackage {:?} is already consumed",
                        request.key_package_id
                    ),
                });
            }
        }
        let changed = record.clone();
        let operation = PersistedOperation::ExpireKeyPackageLease {
            key_package_id: request.key_package_id,
        };
        if let Some(store) = &self.store {
            store.append_key_package_inventory_operation(&operation, &changed)?;
        }
        *inventory = candidate;
        Ok(ExpireKeyPackageLeaseResponse { expired: true })
    }

    fn revoke_device(
        &self,
        request: RevokeDeviceRequest,
    ) -> Result<RevokeDeviceResponse, ServerHttpError> {
        request.device.validate_limits().map_err(|error| {
            ServerHttpError::InvalidDeviceRequest {
                reason: error.to_string(),
            }
        })?;
        let device_key = DeviceMembership::key(&request.device);
        let mut revoked_devices = self.revoked_devices.lock().expect("HTTP device mutex");
        if !revoked_devices.contains(&device_key) {
            let operation = PersistedOperation::RevokeDevice {
                device: request.device.clone(),
            };
            if let Some(store) = &self.store {
                store.append_operation(&operation)?;
            }
            revoked_devices.insert(device_key.clone());
            drop(revoked_devices);
            // A revoked device must never be woken again.
            let mut tokens = self.push_tokens.lock().expect("HTTP push-token mutex");
            if tokens.remove(&device_key).is_some()
                && let Some(store) = &self.store
            {
                store.delete_push_token(&device_key)?;
            }
        }
        Ok(RevokeDeviceResponse { revoked: true })
    }

    fn observe_device_liveness(
        &self,
        request: ObserveDeviceLivenessRequest,
    ) -> Result<DeviceLivenessRecord, ServerHttpError> {
        validate_device_liveness_request(&request)?;
        self.ensure_device_not_revoked(&request.device)?;
        if !self.device_active_in_any_room(&request.device) {
            return Err(ServerHttpError::DeviceNotActive {
                device: request.device,
            });
        }

        let key = DeviceMembership::key(&request.device);
        let mut records = self
            .device_liveness
            .lock()
            .expect("HTTP device-liveness mutex");
        if let Some(current) = records.get(&key)
            && request.observed_at_ms <= current.observed_at_ms
        {
            return Ok(current.clone());
        }

        let record = DeviceLivenessRecord {
            device: request.device,
            observed_at_ms: request.observed_at_ms,
            expires_at_ms: request.expires_at_ms,
        };
        records.insert(key, record.clone());
        Ok(record)
    }

    fn get_device_liveness(
        &self,
        request: GetDeviceLivenessRequest,
    ) -> Result<GetDeviceLivenessResponse, ServerHttpError> {
        request.device.validate_limits().map_err(|error| {
            ServerHttpError::InvalidDeviceLivenessRequest {
                reason: error.to_string(),
            }
        })?;
        let key = DeviceMembership::key(&request.device);
        let record = self
            .device_liveness
            .lock()
            .expect("HTTP device-liveness mutex")
            .get(&key)
            .cloned();
        let live = record
            .as_ref()
            .is_some_and(|record| request.now_ms < record.expires_at_ms)
            && self.device_active_in_any_room(&request.device)
            && self.ensure_device_not_revoked(&request.device).is_ok();
        Ok(GetDeviceLivenessResponse { record, live })
    }

    fn put_nostr_profile(
        &self,
        request: PutNostrProfileRequest,
    ) -> Result<PutNostrProfileResponse, ServerHttpError> {
        let record = {
            let profiles = self
                .nostr_profiles
                .lock()
                .expect("HTTP nostr-profile mutex");
            let existing = profiles.get(&request.profile.account_id);
            normalize_nostr_profile_record(request.profile, existing)?
        };
        validate_nostr_profile_record(&record)?;
        let mut profiles = self
            .nostr_profiles
            .lock()
            .expect("HTTP nostr-profile mutex");
        profiles.insert(record.account_id.clone(), record.clone());
        if let Some(store) = &self.store {
            store.upsert_nostr_profile(&record)?;
        }
        Ok(PutNostrProfileResponse { saved: true })
    }

    fn get_nostr_profiles(
        &self,
        request: GetNostrProfilesRequest,
    ) -> Result<GetNostrProfilesResponse, ServerHttpError> {
        validate_nostr_profile_batch(&request.account_ids)?;
        let profiles = self
            .nostr_profiles
            .lock()
            .expect("HTTP nostr-profile mutex");
        let mut response = Vec::with_capacity(request.account_ids.len());
        for account_id in request.account_ids {
            if let Some(profile) = profiles.get(&account_id) {
                response.push(NostrProfileCacheEntry {
                    profile: profile.clone(),
                    stale: request.now_ms >= profile.expires_at_ms,
                });
            }
        }
        Ok(GetNostrProfilesResponse { profiles: response })
    }

    fn get_key_package_availability(
        &self,
        request: GetKeyPackageAvailabilityRequest,
    ) -> Result<GetKeyPackageAvailabilityResponse, ServerHttpError> {
        validate_key_package_availability_batch(&request.account_ids)?;
        let requested: BTreeSet<&str> = request.account_ids.iter().map(String::as_str).collect();
        let revoked_devices = self.revoked_devices.lock().expect("HTTP device mutex");
        let inventory = self
            .key_package_inventory
            .lock()
            .expect("HTTP KeyPackage inventory mutex");
        let mut available_accounts = BTreeSet::<String>::new();
        for record in inventory.values() {
            if record.state != KeyPackageInventoryState::Available {
                continue;
            }
            let Some(metadata) = &record.finite_metadata else {
                continue;
            };
            if !requested.contains(metadata.owner.account_id.as_str()) {
                continue;
            }
            if revoked_devices.contains(&DeviceMembership::key(&metadata.owner)) {
                continue;
            }
            available_accounts.insert(metadata.owner.account_id.clone());
        }
        let accounts = request
            .account_ids
            .into_iter()
            .map(|account_id| KeyPackageAvailabilityEntry {
                available: available_accounts.contains(&account_id),
                account_id,
            })
            .collect();
        Ok(GetKeyPackageAvailabilityResponse { accounts })
    }

    fn device_active_in_any_room(&self, device: &DeviceRef) -> bool {
        self.room_memberships
            .lock()
            .expect("HTTP room-membership mutex")
            .values()
            .any(|projection| projection.device_active_at_head(device))
    }

    fn revoked_device_keys(&self) -> BTreeSet<String> {
        self.revoked_devices
            .lock()
            .expect("HTTP device mutex")
            .clone()
    }

    fn ensure_device_not_revoked(&self, device: &DeviceRef) -> Result<(), ServerHttpError> {
        let revoked_devices = self.revoked_devices.lock().expect("HTTP device mutex");
        ensure_device_not_revoked_in(&revoked_devices, device)
    }

    fn key_package_inventory(
        &self,
        request: KeyPackageInventoryRequest,
    ) -> Result<HttpKeyPackageInventory, ServerHttpError> {
        let inventory = self
            .key_package_inventory
            .lock()
            .expect("HTTP KeyPackage inventory mutex");
        let mut available = 0usize;
        let mut claimed = 0usize;
        for record in inventory.values() {
            if record.owner != request.owner {
                continue;
            }
            match record.state {
                KeyPackageInventoryState::Available => available += 1,
                KeyPackageInventoryState::Claimed => claimed += 1,
                KeyPackageInventoryState::Consumed => {}
            }
        }
        Ok(HttpKeyPackageInventory {
            owner: request.owner,
            available: usize_to_u32("available", available)?,
            claimed: usize_to_u32("claimed", claimed)?,
        })
    }

    fn create_link_session(
        &self,
        request: CreateLinkSessionRequest,
    ) -> Result<HttpLinkSessionRecord, ServerHttpError> {
        validate_link_session_id(&request.link_session_id)?;
        validate_link_pairing_public_key(&request.pairing_public_key)?;
        let mut sessions = self.link_sessions.lock().expect("HTTP link-session mutex");
        if sessions.contains_key(&request.link_session_id) {
            return Err(ServerHttpError::LinkSessionAlreadyExists {
                link_session_id: request.link_session_id,
            });
        }
        let record = HttpLinkSessionRecord {
            link_session_id: request.link_session_id,
            pairing_public_key: request.pairing_public_key,
            encrypted_payload: None,
            state: HttpLinkSessionState::Created,
            claim_token: None,
        };
        sessions.insert(record.link_session_id.clone(), record.clone());
        drop(sessions);

        if let Some(store) = &self.store {
            store.upsert_link_session(&record)?;
        }
        Ok(record)
    }

    fn get_link_session(
        &self,
        request: GetLinkSessionRequest,
    ) -> Result<Option<HttpLinkSessionRecord>, ServerHttpError> {
        validate_link_session_id(&request.link_session_id)?;
        let sessions = self.link_sessions.lock().expect("HTTP link-session mutex");
        Ok(sessions.get(&request.link_session_id).cloned())
    }

    fn upload_link_payload(
        &self,
        request: UploadLinkPayloadRequest,
    ) -> Result<HttpLinkSessionRecord, ServerHttpError> {
        validate_link_session_id(&request.link_session_id)?;
        validate_link_payload(&request.encrypted_payload)?;
        let mut sessions = self.link_sessions.lock().expect("HTTP link-session mutex");
        let session = sessions.get_mut(&request.link_session_id).ok_or_else(|| {
            ServerHttpError::LinkSessionNotFound {
                link_session_id: request.link_session_id.clone(),
            }
        })?;
        match session.state {
            HttpLinkSessionState::Created => {
                session.encrypted_payload = Some(request.encrypted_payload);
                session.state = HttpLinkSessionState::PayloadUploaded;
            }
            HttpLinkSessionState::PayloadUploaded
                if session.encrypted_payload.as_deref()
                    == Some(request.encrypted_payload.as_slice()) => {}
            HttpLinkSessionState::PayloadUploaded => {
                return Err(ServerHttpError::LinkSessionConflict {
                    link_session_id: request.link_session_id,
                    reason: "encrypted payload differs from existing payload".to_owned(),
                });
            }
            HttpLinkSessionState::Claimed
            | HttpLinkSessionState::Delivered
            | HttpLinkSessionState::Expired => {
                return Err(ServerHttpError::LinkSessionClosed {
                    link_session_id: request.link_session_id,
                });
            }
        }
        let record = session.clone();
        drop(sessions);

        if let Some(store) = &self.store {
            store.upsert_link_session(&record)?;
        }
        Ok(record)
    }

    fn claim_link_payload(
        &self,
        request: ClaimLinkPayloadRequest,
    ) -> Result<ClaimLinkPayloadResponse, ServerHttpError> {
        validate_link_session_id(&request.link_session_id)?;
        let mut sessions = self.link_sessions.lock().expect("HTTP link-session mutex");
        let session = sessions.get_mut(&request.link_session_id).ok_or_else(|| {
            ServerHttpError::LinkSessionNotFound {
                link_session_id: request.link_session_id.clone(),
            }
        })?;
        let newly_claimed = match session.state {
            HttpLinkSessionState::PayloadUploaded => true,
            // The claim token is deterministic and the payload is encrypted to
            // the linker's ephemeral pairing key. Replaying this exact claim
            // lets a linker recover when the first successful HTTP response is
            // lost without opening the payload to anyone new.
            HttpLinkSessionState::Claimed => false,
            _ => {
                return Err(ServerHttpError::LinkSessionNotReady {
                    link_session_id: request.link_session_id,
                });
            }
        };
        let encrypted_payload = session.encrypted_payload.clone().ok_or_else(|| {
            ServerHttpError::LinkSessionNotReady {
                link_session_id: request.link_session_id.clone(),
            }
        })?;
        let claim_token = if newly_claimed {
            let claim_token = link_session_claim_token(session);
            session.state = HttpLinkSessionState::Claimed;
            session.claim_token = Some(claim_token.clone());
            claim_token
        } else {
            session
                .claim_token
                .clone()
                .ok_or_else(|| ServerHttpError::LinkSessionNotReady {
                    link_session_id: request.link_session_id.clone(),
                })?
        };
        let record = session.clone();
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_link_session(&record)
        {
            if newly_claimed {
                session.state = HttpLinkSessionState::PayloadUploaded;
                session.claim_token = None;
            }
            return Err(error.into());
        }
        drop(sessions);
        Ok(ClaimLinkPayloadResponse {
            encrypted_payload,
            claim_token,
        })
    }

    fn ack_link_payload(
        &self,
        request: AckLinkPayloadRequest,
    ) -> Result<AckLinkPayloadResponse, ServerHttpError> {
        validate_link_session_id(&request.link_session_id)?;
        validate_link_claim_token(&request.claim_token)?;
        let mut sessions = self.link_sessions.lock().expect("HTTP link-session mutex");
        let session = sessions.get_mut(&request.link_session_id).ok_or_else(|| {
            ServerHttpError::LinkSessionNotFound {
                link_session_id: request.link_session_id.clone(),
            }
        })?;
        if session.claim_token.as_deref() != Some(request.claim_token.as_str()) {
            return Err(ServerHttpError::BadLinkSessionClaimToken {
                link_session_id: request.link_session_id,
            });
        }
        let newly_delivered = match session.state {
            HttpLinkSessionState::Claimed => {
                session.state = HttpLinkSessionState::Delivered;
                true
            }
            HttpLinkSessionState::Delivered => false,
            _ => {
                return Err(ServerHttpError::LinkSessionNotReady {
                    link_session_id: request.link_session_id,
                });
            }
        };
        let record = session.clone();
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_link_session(&record)
        {
            if newly_delivered {
                session.state = HttpLinkSessionState::Claimed;
            }
            return Err(error.into());
        }
        drop(sessions);
        Ok(AckLinkPayloadResponse { acked: true })
    }

    fn release_link_claim(
        &self,
        request: ReleaseLinkClaimRequest,
    ) -> Result<ReleaseLinkClaimResponse, ServerHttpError> {
        validate_link_session_id(&request.link_session_id)?;
        let mut sessions = self.link_sessions.lock().expect("HTTP link-session mutex");
        let session = sessions.get_mut(&request.link_session_id).ok_or_else(|| {
            ServerHttpError::LinkSessionNotFound {
                link_session_id: request.link_session_id.clone(),
            }
        })?;
        if session.state != HttpLinkSessionState::Claimed {
            return Err(ServerHttpError::LinkSessionNotReady {
                link_session_id: request.link_session_id,
            });
        }
        session.state = HttpLinkSessionState::PayloadUploaded;
        session.claim_token = None;
        let record = session.clone();
        drop(sessions);

        if let Some(store) = &self.store {
            store.upsert_link_session(&record)?;
        }
        Ok(ReleaseLinkClaimResponse { released: true })
    }

    fn expire_link_session(
        &self,
        request: ExpireLinkSessionRequest,
    ) -> Result<ExpireLinkSessionResponse, ServerHttpError> {
        validate_link_session_id(&request.link_session_id)?;
        let mut sessions = self.link_sessions.lock().expect("HTTP link-session mutex");
        let session = sessions.get_mut(&request.link_session_id).ok_or_else(|| {
            ServerHttpError::LinkSessionNotFound {
                link_session_id: request.link_session_id.clone(),
            }
        })?;
        if session.state == HttpLinkSessionState::Delivered {
            return Err(ServerHttpError::LinkSessionClosed {
                link_session_id: request.link_session_id,
            });
        }
        session.state = HttpLinkSessionState::Expired;
        let record = session.clone();
        drop(sessions);

        if let Some(store) = &self.store {
            store.upsert_link_session(&record)?;
        }
        Ok(ExpireLinkSessionResponse { expired: true })
    }

    /// The /sync/wait predicate: any watched room advanced past its cursor.
    fn check_wait_signal(&self, request: &SyncWaitRequest) -> Option<String> {
        {
            let rooms = self
                .room_memberships
                .lock()
                .expect("HTTP room-membership mutex");
            for watch in &request.rooms {
                if let Some(projection) = rooms.get(&watch.room_id)
                    && projection.last_seq > watch.after_seq
                {
                    return Some(format!("room:{}", watch.room_id));
                }
            }
        }
        None
    }

    fn collect_sync_hints(&self, cursors: &mut SyncStreamCursors) -> Vec<SyncHintEvent> {
        let mut events = Vec::new();
        {
            let rooms = self
                .room_memberships
                .lock()
                .expect("HTTP room-membership mutex");
            for watch in &mut cursors.rooms {
                let Some(projection) = rooms.get(&watch.room_id) else {
                    continue;
                };
                if projection.last_seq > watch.after_seq {
                    watch.after_seq = projection.last_seq;
                    events.push(SyncHintEvent::RoomAdvanced {
                        room_id: watch.room_id.clone(),
                        seq: projection.last_seq,
                    });
                }
            }
        }

        for watch in &mut cursors.rooms {
            let highwater = self.activity_highwater_for_room(&watch.room_id);
            if highwater > watch.seen_activity_received_at_ms {
                watch.seen_activity_received_at_ms = highwater;
                events.push(SyncHintEvent::ActivityChanged {
                    room_id: watch.room_id.clone(),
                    received_at_ms: highwater,
                });
            }
        }

        if let Some(watch) = &mut cursors.inbox {
            let next_seq = {
                let service = self.service.lock().expect("HTTP delivery service mutex");
                service
                    .sync_inbox(&watch.recipient, watch.after_seq, 1)
                    .ok()
                    .and_then(|page| page.entries.first().map(|entry| entry.seq))
            };
            if let Some(seq) = next_seq {
                watch.after_seq = seq;
                events.push(SyncHintEvent::InboxAdvanced { seq });
            }
        }

        events
    }

    fn activity_highwater_for_room(&self, room_id: &str) -> u64 {
        let activity = self
            .ephemeral_activity
            .lock()
            .expect("HTTP ephemeral activity mutex");
        activity
            .values()
            .flat_map(|records| records.iter())
            .filter(|record| record.room_id == room_id)
            .map(|record| record.received_at_ms)
            .max()
            .unwrap_or_default()
    }

    fn save_account_room(
        &self,
        request: SaveAccountRoomRequest,
    ) -> Result<SaveAccountRoomResponse, ServerHttpError> {
        validate_account_room_id("account_id", &request.account_id)?;
        validate_account_room_id("room_id", &request.room_id)?;
        let Some(record) = account_scoped_account_room_record(
            &request.account_id,
            &request.room_id,
            &request.record,
        )?
        else {
            return Err(ServerHttpError::InvalidAccountRoomRequest {
                reason: format!(
                    "record has no current devices for account {}",
                    request.account_id
                ),
            });
        };
        let value = serde_json::to_value(&record)
            .map_err(|error| ServerHttpError::ProjectionJson(error.to_string()))?;

        let mut directory = self
            .account_rooms
            .lock()
            .expect("HTTP account-room directory mutex");
        directory
            .entry(request.account_id.clone())
            .or_default()
            .insert(request.room_id.clone(), value.clone());
        if let Some(store) = &self.store {
            store.upsert_account_room(&AccountRoomDirectoryRecord {
                account_id: request.account_id,
                room_id: request.room_id,
                record: value,
            })?;
        }
        Ok(SaveAccountRoomResponse { saved: true })
    }

    fn bootstrap_account_room(
        &self,
        request: BootstrapAccountRoomRequest,
    ) -> Result<BootstrapAccountRoomResponse, ServerHttpError> {
        validate_account_room_id("room_id", &request.room_id)?;
        validate_account_room_id("mls_group_id", &request.mls_group_id)?;
        request.creator.validate_limits().map_err(|error| {
            ServerHttpError::InvalidAccountRoomRequest {
                reason: error.to_string(),
            }
        })?;

        request.protocol.validate_limits().map_err(|error| {
            ServerHttpError::InvalidAccountRoomRequest {
                reason: error.to_string(),
            }
        })?;
        if request.protocol.protocol_version < MIN_SUPPORTED_PROTOCOL_VERSION
            || request.protocol.protocol_version > PROTOCOL_VERSION_V1
        {
            return Err(ServerHttpError::UnsupportedProtocolVersion {
                requested: request.protocol.protocol_version,
                min: MIN_SUPPORTED_PROTOCOL_VERSION,
                max: PROTOCOL_VERSION_V1,
            });
        }
        let account_id = request.creator.account_id.clone();
        validate_account_room_id("account_id", &account_id)?;
        let mut bootstrapped = false;
        {
            let mut directory = self
                .account_rooms
                .lock()
                .expect("HTTP account-room directory mutex");
            if let Some(existing_value) = directory
                .get(&account_id)
                .and_then(|rooms| rooms.get(&request.room_id))
            {
                let existing_record =
                    serde_json::from_value::<AccountRoomRecord>(existing_value.clone()).map_err(
                        |error| ServerHttpError::AccountRoomBootstrapConflict {
                            account_id: account_id.clone(),
                            room_id: request.room_id.clone(),
                            reason: format!(
                                "existing record is not a Finite account-room record: {error}"
                            ),
                        },
                    )?;
                let has_creator = existing_record
                    .devices
                    .iter()
                    .any(|device| device.device == request.creator && device.active);
                if existing_record.mls_group_id != request.mls_group_id || !has_creator {
                    return Err(ServerHttpError::AccountRoomBootstrapConflict {
                        account_id,
                        room_id: request.room_id,
                        reason: "existing account-room record differs from bootstrap request"
                            .to_owned(),
                    });
                }
            } else {
                let record = AccountRoomRecord {
                    room_id: request.room_id.clone(),
                    mls_group_id: request.mls_group_id.clone(),
                    current_epoch: 0,
                    last_seq: 0,
                    status: RoomStatus::Open,
                    devices: vec![AccountRoomDevice {
                        device: request.creator.clone(),
                        active: true,
                    }],
                };
                record.validate_limits().map_err(|error| {
                    ServerHttpError::InvalidAccountRoomRequest {
                        reason: error.to_string(),
                    }
                })?;
                let value = serde_json::to_value(&record)
                    .map_err(|error| ServerHttpError::ProjectionJson(error.to_string()))?;
                directory
                    .entry(account_id.clone())
                    .or_default()
                    .insert(request.room_id.clone(), value.clone());
                if let Some(store) = &self.store {
                    store.upsert_account_room(&AccountRoomDirectoryRecord {
                        account_id: account_id.clone(),
                        room_id: request.room_id.clone(),
                        record: value,
                    })?;
                }
                bootstrapped = true;
            }
        }

        self.bootstrap_room_membership(&request)?;
        Ok(BootstrapAccountRoomResponse { bootstrapped })
    }

    fn list_account_rooms(
        &self,
        request: ListAccountRoomDirectoryRequest,
    ) -> Result<ListAccountRoomDirectoryResponse, ServerHttpError> {
        validate_account_room_id("account_id", &request.account_id)?;
        if let Some(after_room_id) = &request.after_room_id {
            validate_account_room_id("after_room_id", after_room_id)?;
        }
        if request.limit == 0 || request.limit > MAX_HTTP_SYNC_PAGE_ENTRIES {
            return Err(ServerHttpError::InvalidAccountRoomListLimit {
                actual: request.limit,
                max: MAX_HTTP_SYNC_PAGE_ENTRIES,
            });
        }

        let directory = self
            .account_rooms
            .lock()
            .expect("HTTP account-room directory mutex");
        let mut rooms = Vec::new();
        let mut next_after_room_id = None;
        let mut has_more = false;
        if let Some(account_rooms) = directory.get(&request.account_id) {
            for (room_id, record) in account_rooms {
                if let Some(after_room_id) = &request.after_room_id
                    && room_id <= after_room_id
                {
                    continue;
                }
                let Some(record) =
                    account_scoped_account_room_record(&request.account_id, room_id, record)?
                else {
                    continue;
                };
                if rooms.len() == request.limit {
                    has_more = true;
                    break;
                }
                rooms.push(
                    serde_json::to_value(&record)
                        .map_err(|error| ServerHttpError::ProjectionJson(error.to_string()))?,
                );
                next_after_room_id = Some(room_id.clone());
            }
        }
        Ok(ListAccountRoomDirectoryResponse {
            rooms,
            next_after_room_id,
            has_more,
        })
    }

    fn report_invalid_commit(
        &self,
        request: ReportInvalidCommitRequest,
    ) -> Result<ReportInvalidCommitResponse, ServerHttpError> {
        validate_account_room_id("room_id", &request.room_id)?;
        request.reporter.validate_limits().map_err(|error| {
            ServerHttpError::InvalidRepairReport {
                reason: error.to_string(),
            }
        })?;

        let mut projection = {
            let rooms = self
                .room_memberships
                .lock()
                .expect("HTTP room-membership mutex");
            rooms.get(&request.room_id).cloned().ok_or_else(|| {
                ServerHttpError::RoomMembershipConflict {
                    room_id: request.room_id.clone(),
                    reason: "invalid commit report requires a room-membership projection"
                        .to_owned(),
                }
            })?
        };
        if !projection.device_was_member_for_seq(&request.reporter, request.offending_seq) {
            return Err(ServerHttpError::ReporterNotInInterval {
                reporter: request.reporter,
                offending_seq: request.offending_seq,
            });
        }
        projection.status = RoomStatus::NeedsRepair;

        let account_records = self.account_room_repair_records(&request.room_id)?;
        if let Some(store) = &self.store {
            store.upsert_room_repair_state(&projection, &account_records)?;
        }

        let mut rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        rooms.insert(request.room_id.clone(), projection);
        drop(rooms);

        let mut directory = self
            .account_rooms
            .lock()
            .expect("HTTP account-room directory mutex");
        for record in account_records {
            directory
                .entry(record.account_id)
                .or_default()
                .insert(record.room_id, record.record);
        }

        Ok(ReportInvalidCommitResponse { reported: true })
    }

    fn account_room_repair_records(
        &self,
        room_id: &str,
    ) -> Result<Vec<AccountRoomDirectoryRecord>, ServerHttpError> {
        let directory = self
            .account_rooms
            .lock()
            .expect("HTTP account-room directory mutex");
        let mut records = Vec::new();
        for (account_id, rooms) in directory.iter() {
            let Some(value) = rooms.get(room_id) else {
                continue;
            };
            let Some(mut record) = account_scoped_account_room_record(account_id, room_id, value)?
            else {
                continue;
            };
            record.status = RoomStatus::NeedsRepair;
            let value = serde_json::to_value(&record)
                .map_err(|error| ServerHttpError::ProjectionJson(error.to_string()))?;
            records.push(AccountRoomDirectoryRecord {
                account_id: account_id.clone(),
                room_id: room_id.to_owned(),
                record: value,
            });
        }
        Ok(records)
    }

    fn bootstrap_room_membership(
        &self,
        request: &BootstrapAccountRoomRequest,
    ) -> Result<(), ServerHttpError> {
        let mut rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        if let Some(existing) = rooms.get(&request.room_id) {
            let creator_is_active = existing
                .membership
                .get(&DeviceMembership::key(&request.creator))
                .is_some_and(|membership| {
                    membership.intervals.iter().any(|interval| {
                        interval.active && interval.start_seq == 0 && interval.end_seq.is_none()
                    })
                });
            if existing.mls_group_id != request.mls_group_id || !creator_is_active {
                return Err(ServerHttpError::RoomMembershipConflict {
                    room_id: request.room_id.clone(),
                    reason: "existing room-membership projection differs from bootstrap request"
                        .to_owned(),
                });
            }
            return Ok(());
        }

        let observed = self.observed_room_head(&request.room_id, &request.mls_group_id)?;
        if observed.raw_commit_without_projection {
            return Err(ServerHttpError::RoomMembershipConflict {
                room_id: request.room_id.clone(),
                reason: "typed bootstrap requires existing raw commit history to carry membership_delta projection wrappers".to_owned(),
            });
        }
        let projection = initial_room_membership_projection(
            &request.room_id,
            &request.mls_group_id,
            &request.creator,
            observed.current_epoch,
            observed.last_seq,
            true,
            request.protocol.clone(),
        );
        rooms.insert(request.room_id.clone(), projection.clone());
        drop(rooms);

        if let Some(store) = &self.store {
            store.upsert_room_membership(&projection)?;
        }
        Ok(())
    }

    fn observed_room_head(
        &self,
        room_id: &str,
        mls_group_id: &str,
    ) -> Result<ObservedRoomHead, ServerHttpError> {
        let group_id = group_id_for_room(room_id);
        let service = self.service.lock().expect("HTTP delivery service mutex");
        let mut current_epoch = 0;
        let mut last_seq = 0;
        let mut after_seq = 0;
        let mut raw_commit_without_projection = false;
        loop {
            let page = service.sync_group(&group_id, after_seq, MAX_HTTP_SYNC_PAGE_ENTRIES)?;
            for queued in &page.entries {
                last_seq = last_seq.max(queued.seq);
                let has_membership_delta = serde_json::from_slice::<
                    FiniteAccountRoomCommitProjection,
                >(&queued.message.payload)
                .is_ok();
                let Some(entry) = room_log_entry_from_payload(&queued.message.payload) else {
                    continue;
                };
                if entry.room_id == room_id
                    && entry.envelope.mls_group_id == mls_group_id
                    && entry.kind == LogEntryKind::Commit
                {
                    current_epoch = current_epoch.max(entry.epoch.saturating_add(1));
                    if !has_membership_delta {
                        raw_commit_without_projection = true;
                    }
                }
            }
            if !page.has_more || page.next_after_seq <= after_seq {
                break;
            }
            after_seq = page.next_after_seq;
        }
        Ok(ObservedRoomHead {
            current_epoch,
            last_seq,
            raw_commit_without_projection,
        })
    }

    fn record_submit_commit_projection(
        &self,
        request: &SubmitCommitRequest,
        accepted_seq: HttpSequence,
    ) -> Result<(), ServerHttpError> {
        self.record_account_room_membership_delta(
            &request.room_id,
            &request.envelope.mls_group_id,
            request.membership_delta.post_commit_epoch,
            &request.membership_delta,
            accepted_seq,
        )?;
        self.record_room_membership_delta(
            &request.room_id,
            &request.envelope.mls_group_id,
            &request.sender,
            request.expected_epoch,
            &request.membership_delta,
            accepted_seq,
        )
    }

    fn ensure_submit_commit_projection(
        &self,
        request: &SubmitCommitRequest,
        accepted_seq: HttpSequence,
    ) -> Result<(), ServerHttpError> {
        let rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let projection_is_current = rooms.get(&request.room_id).is_some_and(|projection| {
            projection.mls_group_id == request.envelope.mls_group_id
                && projection.current_epoch >= request.membership_delta.post_commit_epoch
                && projection.last_seq >= accepted_seq
        });
        drop(rooms);

        if projection_is_current {
            return Ok(());
        }

        self.record_submit_commit_projection(request, accepted_seq)
    }

    fn record_account_room_membership_delta(
        &self,
        room_id: &str,
        mls_group_id: &str,
        current_epoch: u64,
        membership_delta: &MembershipDeltaV1,
        accepted_seq: HttpSequence,
    ) -> Result<(), ServerHttpError> {
        let mut directory = self
            .account_rooms
            .lock()
            .expect("HTTP account-room directory mutex");
        let mutation = apply_account_room_membership_delta(
            &mut directory,
            room_id,
            mls_group_id,
            current_epoch,
            membership_delta,
            accepted_seq,
        )?;
        drop(directory);

        if let Some(store) = &self.store {
            for (account_id, room_id) in mutation.deletes {
                store.delete_account_room(&account_id, &room_id)?;
            }
            for record in mutation.upserts {
                store.upsert_account_room(&record)?;
            }
        }
        Ok(())
    }

    fn record_room_membership_delta(
        &self,
        room_id: &str,
        mls_group_id: &str,
        sender: &DeviceRef,
        expected_epoch: u64,
        membership_delta: &MembershipDeltaV1,
        accepted_seq: HttpSequence,
    ) -> Result<(), ServerHttpError> {
        let mut rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let projection = apply_room_membership_delta(
            &mut rooms,
            room_id,
            mls_group_id,
            sender,
            expected_epoch,
            membership_delta,
            accepted_seq,
        )?;
        drop(rooms);

        if let Some(store) = &self.store {
            store.upsert_room_membership(&projection)?;
        }
        Ok(())
    }

    fn submit_commit(
        &self,
        request: SubmitCommitRequest,
    ) -> Result<CommitAccepted, ServerHttpError> {
        validate_submit_commit_request(&request)?;
        let message_id = request.envelope.message_id().map_err(|error| {
            ServerHttpError::InvalidCommitRequest {
                reason: error.to_string(),
            }
        })?;
        let commit_publish = commit_publish_request(&request, &message_id)?;
        if let Some(receipt) = self.replayed_publish_receipt(&commit_publish) {
            self.ensure_submit_commit_projection(&request, receipt.seq)?;
            let welcomes = released_welcome_records_for_commit(&request, receipt.seq)?;
            for welcome in &welcomes {
                self.publish_message(welcome_publish_request(welcome)?)?;
            }
            return Ok(CommitAccepted {
                seq: receipt.seq,
                message_id,
                released_welcomes: welcomes
                    .into_iter()
                    .map(|welcome| welcome.welcome_id)
                    .collect(),
            });
        }

        self.ensure_device_not_revoked(&request.sender)?;
        for add in &request.membership_delta.adds {
            self.ensure_device_not_revoked(&add.device)?;
        }
        self.validate_commit_room_membership(&request)?;

        // Fresh typed commits must publish the commit, release Welcomes, and update
        // Finite projections as one candidate snapshot before the durable swap.
        let mut service = self.service.lock().expect("HTTP delivery service mutex");
        let mut publish_idempotency = self
            .publish_idempotency
            .lock()
            .expect("HTTP publish idempotency mutex");
        let mut account_rooms = self
            .account_rooms
            .lock()
            .expect("HTTP account-room directory mutex");
        let mut room_memberships = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let mut key_package_inventory = self
            .key_package_inventory
            .lock()
            .expect("HTTP KeyPackage inventory mutex");

        // Commit and Welcome publishes are dry-run checked against live
        // state (the delivery service is never cloned); only the small
        // projection maps keep the candidate pattern.
        let mut candidate_account_rooms = account_rooms.clone();
        let mut candidate_room_memberships = room_memberships.clone();
        let mut candidate_key_package_inventory = key_package_inventory.clone();

        let commit_check = check_publish_request(&service, &publish_idempotency, &commit_publish)?;
        let receipt = commit_check.receipt.clone();
        let mut checked_publishes = vec![(commit_publish, commit_check)];
        let account_room_mutation = apply_account_room_membership_delta(
            &mut candidate_account_rooms,
            &request.room_id,
            &request.envelope.mls_group_id,
            request.membership_delta.post_commit_epoch,
            &request.membership_delta,
            receipt.seq,
        )?;
        let room_membership_projection = apply_room_membership_delta(
            &mut candidate_room_memberships,
            &request.room_id,
            &request.envelope.mls_group_id,
            &request.sender,
            request.expected_epoch,
            &request.membership_delta,
            receipt.seq,
        )?;
        let key_package_inventory_mutation = consume_claimed_key_packages_for_commit(
            &mut candidate_key_package_inventory,
            &request,
        )?;

        let welcomes = released_welcome_records_for_commit(&request, receipt.seq)?;
        for welcome in &welcomes {
            let publish = welcome_publish_request(welcome)?;
            let check = check_publish_request(&service, &publish_idempotency, &publish)?;
            checked_publishes.push((publish, check));
        }
        let publish_mutations = checked_publishes
            .iter()
            .filter_map(|(_, check)| check.mutation.clone())
            .collect::<Vec<_>>();

        if let Some(store) = &self.store {
            store.append_submit_commit_mutation(
                &publish_mutations,
                &account_room_mutation,
                &room_membership_projection,
                &key_package_inventory_mutation,
            )?;
        }

        for (publish, check) in checked_publishes {
            if check.fresh {
                let published = service.publish(publish.target, publish.message)?;
                debug_assert_eq!(published, check.receipt);
            }
            if let Some(mutation) = check.mutation {
                publish_idempotency.insert(mutation.idempotency_key, mutation.record);
            }
        }
        *account_rooms = candidate_account_rooms;
        *room_memberships = candidate_room_memberships;
        *key_package_inventory = candidate_key_package_inventory;
        drop(service);
        drop(publish_idempotency);
        drop(account_rooms);
        drop(room_memberships);
        drop(key_package_inventory);

        Ok(CommitAccepted {
            seq: receipt.seq,
            message_id,
            released_welcomes: welcomes
                .into_iter()
                .map(|welcome| welcome.welcome_id)
                .collect(),
        })
    }

    fn replayed_publish_receipt(
        &self,
        request: &PublishMessageRequest,
    ) -> Option<HttpPublishReceipt> {
        let idempotency_key = request.idempotency_key.as_ref()?;
        let fingerprint = PublishMessageFingerprint::from_request(request);
        let idempotency = self
            .publish_idempotency
            .lock()
            .expect("HTTP publish idempotency mutex");
        idempotency
            .get(idempotency_key)
            .filter(|record| record.fingerprint == fingerprint)
            .map(|record| record.receipt.clone())
    }

    fn validate_commit_room_membership(
        &self,
        request: &SubmitCommitRequest,
    ) -> Result<(), ServerHttpError> {
        let rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let Some(projection) = rooms.get(&request.room_id) else {
            return Ok(());
        };
        if projection.mls_group_id != request.envelope.mls_group_id {
            return Err(ServerHttpError::InvalidCommitRequest {
                reason: "commit envelope MLS group does not match room projection".to_owned(),
            });
        }
        if projection.status != RoomStatus::Open {
            return Err(ServerHttpError::RoomNotOpen {
                room_id: request.room_id.clone(),
                status: projection.status,
            });
        }
        if request.expected_epoch != projection.current_epoch {
            return Err(ServerHttpError::InvalidCommitRequest {
                reason: format!(
                    "commit expected epoch {} does not match room epoch {}",
                    request.expected_epoch, projection.current_epoch
                ),
            });
        }
        let tracks_sender = projection.tracks_device(&request.sender);
        if (tracks_sender || projection.membership_complete)
            && !projection.device_active_at_head(&request.sender)
        {
            return Err(ServerHttpError::SenderNotActive {
                sender: request.sender.clone(),
            });
        }
        validate_membership_adds_for_projection(projection, &request.membership_delta.adds)?;
        Ok(())
    }

    fn append_application_event(
        &self,
        request: AppendApplicationEventRequest,
    ) -> Result<EventAccepted, ServerHttpError> {
        validate_append_event_request(&request.event)?;
        if request.event.envelope.kind != LogEntryKind::Application {
            return Err(ServerHttpError::InvalidEventRequest {
                reason: "/events accepts only application envelopes".to_owned(),
            });
        }
        self.ensure_device_not_revoked(&request.event.sender)?;
        self.validate_event_room_membership(&request.event)?;
        let message_id = request.event.envelope.message_id().map_err(|error| {
            ServerHttpError::InvalidEventRequest {
                reason: error.to_string(),
            }
        })?;
        let event_publish = event_publish_request(&request.event, &message_id)?;

        let mut service = self.service.lock().expect("HTTP delivery service mutex");
        let mut idempotency = self
            .publish_idempotency
            .lock()
            .expect("HTTP publish idempotency mutex");
        let mut room_memberships = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let mut application_effects = self
            .application_effects
            .lock()
            .expect("HTTP application-effects mutex");
        let mut push_wakes = self.push_wakes.lock().expect("HTTP push-wake mutex");

        // Check phase: every admission rule runs read-only against live
        // state, producing exactly the rows to persist.
        let (receipt, publish_mutation) =
            check_typed_event_publish(&service, &idempotency, &event_publish, &message_id)?;
        let room_membership_projection =
            check_room_event_acceptance(&room_memberships, &request.event.room_id, receipt.seq);
        let effect = HttpApplicationDeliveryEffect {
            room_id: request.event.room_id.clone(),
            seq: receipt.seq,
            message_id: message_id.clone(),
            sender: request.event.sender,
            delivery_policy: request.delivery_policy,
        };
        let effect_mutation = check_application_delivery_effect(
            &application_effects,
            effect,
            &request.event.idempotency_key,
        )?;
        let push_wake_mutation = effect_mutation
            .as_ref()
            .and_then(PushWakeOutboxRecord::from_effect);

        // Persist phase: one SQLite transaction, before any in-memory state
        // changes, so an injected failure rolls back with nothing to undo.
        if let Some(store) = &self.store {
            store.append_application_event_mutation(
                publish_mutation.as_ref(),
                room_membership_projection.as_ref(),
                effect_mutation.as_ref(),
                push_wake_mutation.as_ref(),
            )?;
        }

        // Apply phase: infallible given the checks above ran under the held
        // locks.
        if let Some(mutation) = publish_mutation {
            let published =
                service.publish(event_publish.target.clone(), event_publish.message.clone())?;
            debug_assert_eq!(published, receipt);
            idempotency.insert(mutation.idempotency_key, mutation.record);
        }
        if let Some(projection) = room_membership_projection {
            room_memberships.insert(request.event.room_id.clone(), projection);
        }
        if let Some(effect) = effect_mutation {
            application_effects.insert(effect.message_id.clone(), effect);
        }
        if let Some(wake) = push_wake_mutation {
            push_wakes.insert(wake.wake_id.clone(), wake);
        }
        Ok(EventAccepted {
            seq: receipt.seq,
            message_id,
        })
    }

    fn validate_event_room_membership(
        &self,
        request: &AppendEventRequest,
    ) -> Result<(), ServerHttpError> {
        let rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let projection =
            rooms
                .get(&request.room_id)
                .ok_or_else(|| ServerHttpError::RoomMembershipConflict {
                    room_id: request.room_id.clone(),
                    reason: "typed event requires a room-membership projection".to_owned(),
                })?;
        if projection.mls_group_id != request.envelope.mls_group_id {
            return Err(ServerHttpError::InvalidEventRequest {
                reason: "event envelope MLS group does not match room projection".to_owned(),
            });
        }
        if projection.status != RoomStatus::Open {
            return Err(ServerHttpError::RoomNotOpen {
                room_id: request.room_id.clone(),
                status: projection.status,
            });
        }
        if request.envelope.epoch != projection.current_epoch {
            return Err(ServerHttpError::InvalidEventRequest {
                reason: format!(
                    "event envelope epoch {} does not match room epoch {}",
                    request.envelope.epoch, projection.current_epoch
                ),
            });
        }
        let tracks_sender = projection.tracks_device(&request.sender);
        if (tracks_sender || projection.membership_complete)
            && !projection.device_active_at_head(&request.sender)
        {
            return Err(ServerHttpError::SenderNotActive {
                sender: request.sender.clone(),
            });
        }
        Ok(())
    }

    fn application_effect(
        &self,
        request: ApplicationEffectRequest,
    ) -> Result<Option<HttpApplicationDeliveryEffect>, ServerHttpError> {
        validate_string_bytes("message_id", &request.message_id, MAX_OBJECT_ID_BYTES).map_err(
            |error| ServerHttpError::InvalidEventRequest {
                reason: error.to_string(),
            },
        )?;
        let effects = self
            .application_effects
            .lock()
            .expect("HTTP application-effects mutex");
        Ok(effects.get(&request.message_id).cloned())
    }

    fn application_effect_counts(
        &self,
    ) -> Result<ApplicationEffectCountsResponse, ServerHttpError> {
        let effects = self
            .application_effects
            .lock()
            .expect("HTTP application-effects mutex");
        let mut push_outbox = 0usize;
        let mut unread = 0usize;
        let mut command_inbox = 0usize;
        for effect in effects.values() {
            if effect.delivery_policy.creates_push() {
                push_outbox += 1;
            }
            if effect.delivery_policy.creates_unread() {
                unread += 1;
            }
            if effect.delivery_policy.creates_command_inbox_work() {
                command_inbox += 1;
            }
        }
        Ok(ApplicationEffectCountsResponse {
            push_outbox: usize_to_u32("push_outbox", push_outbox)?,
            unread: usize_to_u32("unread", unread)?,
            command_inbox: usize_to_u32("command_inbox", command_inbox)?,
        })
    }

    fn claim_push_wakes(
        &self,
        request: ClaimPushWakesRequest,
    ) -> Result<ClaimPushWakesResponse, ServerHttpError> {
        let limit = request.limit.min(MAX_PUSH_WAKE_CLAIM_BATCH);
        if limit == 0 {
            return Ok(ClaimPushWakesResponse { wakes: Vec::new() });
        }
        if request.lease_ms == 0 || request.lease_ms > MAX_PUSH_WAKE_LEASE_MS {
            return Err(ServerHttpError::InvalidDeviceRequest {
                reason: format!("push wake lease_ms must be 1..={MAX_PUSH_WAKE_LEASE_MS}"),
            });
        }

        let mut push_wakes = self.push_wakes.lock().expect("HTTP push-wake mutex");
        let mut claimable: Vec<(HttpSequence, String, PushWakeOutboxRecord)> = push_wakes
            .iter()
            .filter(|(_, record)| record.claimable_at(request.now_ms))
            .map(|(wake_id, record)| (record.seq, wake_id.clone(), record.clone()))
            .collect();
        claimable.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

        let claimed: Vec<PushWakeOutboxRecord> = claimable
            .into_iter()
            .take(limit)
            .map(|(_, _, record)| record.claimed(request.now_ms, request.lease_ms))
            .collect();
        if claimed.is_empty() {
            return Ok(ClaimPushWakesResponse { wakes: Vec::new() });
        }

        if let Some(store) = &self.store {
            store.upsert_push_wakes(&claimed)?;
        }
        for record in &claimed {
            push_wakes.insert(record.wake_id.clone(), record.clone());
        }
        drop(push_wakes);

        let tokens = self.push_tokens.lock().expect("HTTP push-token mutex");
        let rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let revoked = self.revoked_devices.lock().expect("HTTP device mutex");
        let wakes = claimed
            .iter()
            .map(|record| PushWakeDelivery {
                wake_id: record.wake_id.clone(),
                payload: PushWakePayload {
                    room_id: record.room_id.clone(),
                    seq: record.seq,
                },
                tokens: push_tokens_for_wake(record, &tokens, &rooms, &revoked),
                attempt: record.attempts(),
            })
            .collect();
        Ok(ClaimPushWakesResponse { wakes })
    }

    fn ack_push_wake(
        &self,
        request: AckPushWakeRequest,
    ) -> Result<AckPushWakeResponse, ServerHttpError> {
        validate_string_bytes("wake_id", &request.wake_id, MAX_OBJECT_ID_BYTES).map_err(
            |error| ServerHttpError::InvalidDeviceRequest {
                reason: error.to_string(),
            },
        )?;
        let mut push_wakes = self.push_wakes.lock().expect("HTTP push-wake mutex");
        let acked = push_wakes.contains_key(&request.wake_id);
        if acked {
            if let Some(store) = &self.store {
                store.delete_push_wake(&request.wake_id)?;
            }
            push_wakes.remove(&request.wake_id);
        }
        Ok(AckPushWakeResponse { acked })
    }

    fn fail_push_wake(
        &self,
        request: FailPushWakeRequest,
    ) -> Result<FailPushWakeResponse, ServerHttpError> {
        validate_string_bytes("wake_id", &request.wake_id, MAX_OBJECT_ID_BYTES).map_err(
            |error| ServerHttpError::InvalidDeviceRequest {
                reason: error.to_string(),
            },
        )?;
        let mut push_wakes = self.push_wakes.lock().expect("HTTP push-wake mutex");
        let Some(record) = push_wakes.get(&request.wake_id).cloned() else {
            return Ok(FailPushWakeResponse {
                retry: false,
                dropped: false,
            });
        };

        if record.attempts() >= MAX_PUSH_WAKE_ATTEMPTS {
            if let Some(store) = &self.store {
                store.delete_push_wake(&request.wake_id)?;
            }
            push_wakes.remove(&request.wake_id);
            return Ok(FailPushWakeResponse {
                retry: false,
                dropped: true,
            });
        }

        let retry = record.released_for_retry();
        if let Some(store) = &self.store {
            store.upsert_push_wakes(std::slice::from_ref(&retry))?;
        }
        push_wakes.insert(retry.wake_id.clone(), retry);
        Ok(FailPushWakeResponse {
            retry: true,
            dropped: false,
        })
    }

    fn append_ephemeral_activity(
        &self,
        request: AppendEphemeralActivityRequest,
    ) -> Result<EphemeralActivityAccepted, ServerHttpError> {
        validate_append_ephemeral_activity_request(&request)?;
        self.ensure_device_not_revoked(&request.sender)?;
        {
            let rooms = self
                .room_memberships
                .lock()
                .expect("HTTP room-membership mutex");
            let projection = rooms.get(&request.room_id).ok_or_else(|| {
                ServerHttpError::RoomMembershipConflict {
                    room_id: request.room_id.clone(),
                    reason: "ephemeral activity requires a room-membership projection".to_owned(),
                }
            })?;
            if projection.mls_group_id != request.mls_group_id {
                return Err(ServerHttpError::InvalidActivityRequest {
                    reason: "activity MLS group does not match room projection".to_owned(),
                });
            }
            if projection.status != RoomStatus::Open {
                return Err(ServerHttpError::RoomNotOpen {
                    room_id: request.room_id.clone(),
                    status: projection.status,
                });
            }
            if request.epoch != projection.current_epoch {
                return Err(ServerHttpError::InvalidActivityRequest {
                    reason: format!(
                        "activity epoch {} does not match room epoch {}",
                        request.epoch, projection.current_epoch
                    ),
                });
            }
            let tracks_sender = projection.tracks_device(&request.sender);
            if (tracks_sender || projection.membership_complete)
                && !projection.device_active_at_head(&request.sender)
            {
                return Err(ServerHttpError::SenderNotActive {
                    sender: request.sender.clone(),
                });
            }
        }

        let route_key = finitechat_proto::ephemeral_activity_route_key(
            &request.room_id,
            request.conversation_id.as_deref(),
            &request.sender,
        );
        let record = EphemeralActivityRecord {
            room_id: request.room_id,
            mls_group_id: request.mls_group_id,
            epoch: request.epoch,
            sender: request.sender,
            conversation_id: request.conversation_id,
            payload: request.payload,
            received_at_ms: request.received_at_ms,
            expires_at_ms: request.expires_at_ms,
        };
        let mut activity = self
            .ephemeral_activity
            .lock()
            .expect("HTTP ephemeral activity mutex");
        let records = activity.entry(route_key.clone()).or_default();
        records.retain(|record| record.expires_at_ms > record.received_at_ms);
        records.push(record);
        while records.len() > MAX_EPHEMERAL_ACTIVITY_CACHE_ENTRIES_PER_ROUTE as usize {
            records.remove(0);
        }
        let cached_events_for_route =
            u32::try_from(records.len()).map_err(|_| ServerHttpError::CounterOverflow)?;
        Ok(EphemeralActivityAccepted {
            route_key,
            cached_events_for_route,
        })
    }

    fn get_ephemeral_activities(
        &self,
        request: GetEphemeralActivitiesRequest,
    ) -> Result<GetEphemeralActivitiesResponse, ServerHttpError> {
        validate_get_ephemeral_activities_request(&request)?;
        self.ensure_device_not_revoked(&request.requester)?;
        {
            let rooms = self
                .room_memberships
                .lock()
                .expect("HTTP room-membership mutex");
            let projection = rooms.get(&request.room_id).ok_or_else(|| {
                ServerHttpError::RoomMembershipConflict {
                    room_id: request.room_id.clone(),
                    reason: "ephemeral activity read requires a room-membership projection"
                        .to_owned(),
                }
            })?;
            if projection.status != RoomStatus::Open {
                return Err(ServerHttpError::RoomNotOpen {
                    room_id: request.room_id.clone(),
                    status: projection.status,
                });
            }
            let tracks_requester = projection.tracks_device(&request.requester);
            if (tracks_requester || projection.membership_complete)
                && !projection.device_active_at_head(&request.requester)
            {
                return Err(ServerHttpError::SenderNotActive {
                    sender: request.requester.clone(),
                });
            }
        }

        let mut activity = self
            .ephemeral_activity
            .lock()
            .expect("HTTP ephemeral activity mutex");
        let mut records = Vec::new();
        for route_records in activity.values_mut() {
            route_records.retain(|record| record.expires_at_ms > request.now_ms);
            records.extend(
                route_records
                    .iter()
                    .filter(|record| {
                        record.room_id == request.room_id
                            && record.conversation_id == request.conversation_id
                    })
                    .cloned(),
            );
        }
        records.sort_by(|left, right| {
            left.received_at_ms
                .cmp(&right.received_at_ms)
                .then_with(|| left.sender.account_id.cmp(&right.sender.account_id))
                .then_with(|| left.sender.device_id.cmp(&right.sender.device_id))
        });
        Ok(GetEphemeralActivitiesResponse { records })
    }

    fn claim_welcomes(
        &self,
        request: ClaimWelcomesRequest,
    ) -> Result<Vec<HttpClaimedWelcome>, ServerHttpError> {
        if request.limit == 0 || request.limit > MAX_HTTP_SYNC_PAGE_ENTRIES {
            return Err(ServerHttpError::InvalidWelcomeClaimLimit {
                actual: request.limit,
                max: MAX_HTTP_SYNC_PAGE_ENTRIES,
            });
        }
        let revoked_devices = self.revoked_device_keys();

        let service = self.service.lock().expect("HTTP delivery service mutex");
        let mut claims = self
            .welcome_claims
            .lock()
            .expect("HTTP welcome claims mutex");
        let mut claimed = Vec::new();
        let mut after_seq = 0;
        loop {
            let page =
                service.sync_inbox(&request.recipient, after_seq, MAX_HTTP_SYNC_PAGE_ENTRIES)?;
            for entry in page.entries {
                if claimed.len() >= request.limit {
                    break;
                }
                if !matches!(entry.message.envelope, TransportEnvelope::Welcome { .. }) {
                    continue;
                }
                ensure_welcome_message_recipient_not_revoked(&revoked_devices, &entry.message)?;
                if claims.contains_key(&entry.message.id) {
                    continue;
                }
                let record = WelcomeClaimRecord {
                    recipient: request.recipient.clone(),
                    seq: entry.seq,
                    message: entry.message,
                    state: WelcomeClaimState::Claimed,
                };
                if let Some(store) = &self.store {
                    store.upsert_welcome_claim(&record)?;
                }
                claims.insert(record.message.id.clone(), record.clone());
                claimed.push(record.into_claimed_welcome());
            }
            if claimed.len() >= request.limit || !page.has_more {
                break;
            }
            after_seq = page.next_after_seq;
        }
        Ok(claimed)
    }

    fn ack_welcome(
        &self,
        request: AckWelcomeRequest,
    ) -> Result<AckWelcomeResponse, ServerHttpError> {
        let activation_message;
        let mut claims = self
            .welcome_claims
            .lock()
            .expect("HTTP welcome claims mutex");
        let Some(record) = claims.get_mut(&request.message_id) else {
            return Err(ServerHttpError::WelcomeNotFound {
                message_id: request.message_id,
            });
        };
        ensure_welcome_message_recipient_not_revoked(&self.revoked_device_keys(), &record.message)?;
        match record.state {
            WelcomeClaimState::Claimed => {
                record.state = WelcomeClaimState::Acked;
                if let Some(store) = &self.store {
                    store.upsert_welcome_claim(record)?;
                }
                activation_message = Some(record.message.clone());
            }
            // A failed activation never reaches the server: the device simply
            // retries, so a repeated ack is an idempotent activation replay.
            WelcomeClaimState::Acked => {
                activation_message = Some(record.message.clone());
            }
        }
        drop(claims);

        if let Some(message) = activation_message {
            self.activate_account_room_from_welcome(&message)?;
            self.activate_room_membership_from_welcome(&message)?;
        }
        Ok(AckWelcomeResponse { acked: true })
    }

    fn activate_account_room_from_welcome(
        &self,
        message: &TransportMessage,
    ) -> Result<(), ServerHttpError> {
        let Ok(welcome) = serde_json::from_slice::<WelcomeRecord>(&message.payload) else {
            return Ok(());
        };
        if message.id.as_slice() != welcome.welcome_id.as_bytes() {
            return Ok(());
        }
        validate_account_room_id("room_id", &welcome.room_id)?;
        welcome.recipient.validate_limits().map_err(|error| {
            ServerHttpError::InvalidAccountRoomRequest {
                reason: error.to_string(),
            }
        })?;

        let account_id = welcome.recipient.account_id.clone();
        let mut directory = self
            .account_rooms
            .lock()
            .expect("HTTP account-room directory mutex");
        let Some(existing_value) = directory
            .get(&account_id)
            .and_then(|rooms| rooms.get(&welcome.room_id))
            .cloned()
        else {
            return Ok(());
        };
        let Some(mut record) =
            account_scoped_account_room_record(&account_id, &welcome.room_id, &existing_value)?
        else {
            return Ok(());
        };

        let mut changed = false;
        for device in &mut record.devices {
            if device.device == welcome.recipient && !device.active {
                device.active = true;
                changed = true;
            }
        }
        if !changed {
            return Ok(());
        }
        let value = serde_json::to_value(&record)
            .map_err(|error| ServerHttpError::ProjectionJson(error.to_string()))?;
        directory
            .entry(account_id.clone())
            .or_default()
            .insert(welcome.room_id.clone(), value.clone());
        drop(directory);

        if let Some(store) = &self.store {
            store.upsert_account_room(&AccountRoomDirectoryRecord {
                account_id,
                room_id: welcome.room_id,
                record: value,
            })?;
        }
        Ok(())
    }

    fn leave_room(&self, request: LeaveRoomRequest) -> Result<LeaveRoomResponse, ServerHttpError> {
        self.ensure_device_not_revoked(&request.sender)?;
        let mut rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let Some(projection) = rooms.get_mut(&request.room_id) else {
            return Err(ServerHttpError::RoomMembershipConflict {
                room_id: request.room_id.clone(),
                reason: "leave requires a room-membership projection".to_owned(),
            });
        };
        if projection.status != RoomStatus::Open {
            return Err(ServerHttpError::RoomNotOpen {
                room_id: request.room_id.clone(),
                status: projection.status,
            });
        }
        let account_id = request.sender.account_id.clone();
        let departed_at_seq = projection.last_seq;
        if projection.departed.contains(&account_id)
            || projection.current_or_pending_device_count_for_account(&account_id) == 0
        {
            // Idempotent replay: the account already left (or was removed).
            return Ok(LeaveRoomResponse {
                left: false,
                departed_at_seq,
            });
        }
        if !projection.device_active_at_head(&request.sender) {
            return Err(ServerHttpError::SenderNotActive {
                sender: request.sender.clone(),
            });
        }

        // Whole-account leave (ADR 0003 §3): close every open interval the
        // account holds; delivery filtering takes over immediately. The MLS
        // removal commit follows asynchronously from an admin device.
        for membership in projection.membership.values_mut() {
            if membership.device.account_id != account_id {
                continue;
            }
            for interval in membership.intervals.iter_mut() {
                if interval.end_seq.is_none() {
                    interval.end_seq = Some(departed_at_seq);
                }
            }
        }
        projection.departed.insert(account_id.clone());
        // The last admin cannot leave a room that still has other members —
        // that would strand the room with no one able to manage membership.
        // They must grant another admin first (or remove everyone).
        if projection.admins.contains(&account_id) && projection.admins.len() == 1 {
            let remaining_accounts = projection
                .membership
                .values()
                .filter(|membership| membership.device.account_id != account_id)
                .filter(|membership| {
                    membership
                        .intervals
                        .iter()
                        .any(|interval| interval.end_seq.is_none())
                })
                .count();
            if remaining_accounts > 0 {
                // Re-open the intervals we just closed and refuse: the last
                // admin must hand off (or remove everyone) before leaving.
                for membership in projection.membership.values_mut() {
                    if membership.device.account_id != account_id {
                        continue;
                    }
                    for interval in membership.intervals.iter_mut() {
                        if interval.end_seq == Some(departed_at_seq) {
                            interval.end_seq = None;
                        }
                    }
                }
                projection.departed.remove(&account_id);
                return Err(ServerHttpError::InvalidAdminChange {
                    reason: "the last admin must grant another admin before leaving".to_owned(),
                });
            }
        }
        projection.admins.remove(&account_id);
        let updated = projection.clone();
        drop(rooms);

        // Drop the room from the departing account's directory.
        {
            let mut directory = self
                .account_rooms
                .lock()
                .expect("HTTP account-room directory mutex");
            if let Some(rooms_for_account) = directory.get_mut(&account_id) {
                rooms_for_account.remove(&request.room_id);
            }
        }
        if let Some(store) = &self.store {
            store.upsert_room_membership(&updated)?;
            store.delete_account_room(&account_id, &request.room_id)?;
        }
        Ok(LeaveRoomResponse {
            left: true,
            departed_at_seq,
        })
    }

    fn update_room_admins(
        &self,
        request: UpdateRoomAdminsRequest,
    ) -> Result<UpdateRoomAdminsResponse, ServerHttpError> {
        let (grant, target) = match (&request.grant, &request.revoke) {
            (Some(account), None) => (true, account.clone()),
            (None, Some(account)) => (false, account.clone()),
            _ => {
                return Err(ServerHttpError::InvalidAdminChange {
                    reason: "exactly one of grant or revoke is required".to_owned(),
                });
            }
        };
        self.ensure_device_not_revoked(&request.sender)?;

        let mut rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let Some(projection) = rooms.get_mut(&request.room_id) else {
            return Err(ServerHttpError::RoomMembershipConflict {
                room_id: request.room_id.clone(),
                reason: "admin change requires a room-membership projection".to_owned(),
            });
        };
        if projection.status != RoomStatus::Open {
            return Err(ServerHttpError::RoomNotOpen {
                room_id: request.room_id.clone(),
                status: projection.status,
            });
        }
        if !projection.device_active_at_head(&request.sender) {
            return Err(ServerHttpError::SenderNotActive {
                sender: request.sender.clone(),
            });
        }
        if !projection.admins.contains(&request.sender.account_id) {
            return Err(ServerHttpError::CommitAuthorityRequired {
                sender: request.sender.clone(),
            });
        }

        if grant {
            if projection.current_or_pending_device_count_for_account(&target) == 0 {
                return Err(ServerHttpError::InvalidAdminChange {
                    reason: format!("account {target} has no devices in the room"),
                });
            }
            projection.admins.insert(target);
        } else {
            if !projection.admins.contains(&target) {
                return Err(ServerHttpError::InvalidAdminChange {
                    reason: format!("account {target} is not an admin"),
                });
            }
            if projection.admins.len() == 1 {
                return Err(ServerHttpError::InvalidAdminChange {
                    reason: "cannot revoke the last admin".to_owned(),
                });
            }
            projection.admins.remove(&target);
        }
        let updated = projection.clone();
        drop(rooms);

        if let Some(store) = &self.store {
            store.upsert_room_membership(&updated)?;
        }
        Ok(UpdateRoomAdminsResponse {
            admins: updated.admins.iter().cloned().collect(),
        })
    }

    fn register_push_token(
        &self,
        request: RegisterPushTokenRequest,
    ) -> Result<RegisterPushTokenResponse, ServerHttpError> {
        request.device.validate_limits().map_err(|error| {
            ServerHttpError::InvalidDeviceRequest {
                reason: error.to_string(),
            }
        })?;
        if request.token.is_empty() || request.token.len() > 4_096 {
            return Err(ServerHttpError::InvalidDeviceRequest {
                reason: "push token must be 1..=4096 bytes".to_owned(),
            });
        }
        self.ensure_device_not_revoked(&request.device)?;
        let record = PushTokenRecord {
            device: request.device.clone(),
            platform: request.platform,
            token: request.token,
        };
        let mut tokens = self.push_tokens.lock().expect("HTTP push-token mutex");
        if let Some(store) = &self.store {
            store.upsert_push_token(&record)?;
        }
        tokens.insert(DeviceMembership::key(&request.device), record);
        Ok(RegisterPushTokenResponse { registered: true })
    }

    fn remove_push_token(
        &self,
        request: RemovePushTokenRequest,
    ) -> Result<RemovePushTokenResponse, ServerHttpError> {
        request.device.validate_limits().map_err(|error| {
            ServerHttpError::InvalidDeviceRequest {
                reason: error.to_string(),
            }
        })?;
        let key = DeviceMembership::key(&request.device);
        let mut tokens = self.push_tokens.lock().expect("HTTP push-token mutex");
        let removed = match (tokens.get(&key), request.token.as_deref()) {
            (None, _) => false,
            (Some(record), Some(expected_token)) if record.token != expected_token => false,
            (Some(_), _) => tokens.remove(&key).is_some(),
        };
        if removed && let Some(store) = &self.store {
            store.delete_push_token(&key)?;
        }
        Ok(RemovePushTokenResponse { removed })
    }

    /// Write a fresh durable-state snapshot so the next startup replays only
    /// the operation-log tail. Called automatically every
    /// [`SNAPSHOT_INTERVAL_OPS`] accepted operations and available for
    /// graceful shutdowns.
    pub fn snapshot_now(&self) -> Result<(), ServerHttpError> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        // Lock order matches submit_commit (service before inventory); the
        // revoked set is copied last. Holding these blocks op appends, so the
        // MAX(seq) read is consistent with the captured state.
        let service = self.service.lock().expect("HTTP delivery service mutex");
        let inventory = self
            .key_package_inventory
            .lock()
            .expect("HTTP KeyPackage inventory mutex");
        let revoked = self
            .revoked_devices
            .lock()
            .expect("HTTP revoked device mutex");
        let snapshot = DurableStateSnapshot {
            service: service.clone(),
            key_package_inventory: inventory.values().cloned().collect(),
            revoked_devices: revoked.clone(),
        };
        let last_op_seq = store.max_operation_seq()?;
        store.save_state_snapshot(last_op_seq, &snapshot)?;
        *self
            .ops_since_snapshot
            .lock()
            .expect("snapshot counter mutex") = 0;
        Ok(())
    }

    fn note_op_for_snapshot(&self) {
        let due = {
            let mut counter = self
                .ops_since_snapshot
                .lock()
                .expect("snapshot counter mutex");
            *counter += 1;
            *counter >= SNAPSHOT_INTERVAL_OPS
        };
        if due {
            // Snapshotting is an optimization; a failure here must not fail
            // the request that triggered it.
            if self.snapshot_now().is_err() {
                // The next interval will retry.
            }
        }
    }

    pub fn sync_inbox(
        &self,
        recipient: &MemberId,
        after_seq: u64,
        limit: usize,
    ) -> Result<HttpSyncPage, ServerHttpError> {
        let service = self.service.lock().expect("HTTP delivery service mutex");
        Ok(service.sync_inbox(recipient, after_seq, limit)?)
    }

    pub fn sync_group(&self, request: GroupSyncRequest) -> Result<HttpSyncPage, ServerHttpError> {
        if request.limit == 0 || request.limit > MAX_HTTP_SYNC_PAGE_ENTRIES {
            return Err(ServerHttpError::InvalidGroupSyncLimit {
                actual: request.limit,
                max: MAX_HTTP_SYNC_PAGE_ENTRIES,
            });
        }
        let service = self.service.lock().expect("HTTP delivery service mutex");
        let page = service.sync_group(&request.group_id, request.after_seq, request.limit)?;
        drop(service);

        let Some(requester) = &request.requester else {
            return Ok(page);
        };
        let room_id = room_id_for_group_id(&request.group_id)?;
        let rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let Some(projection) = rooms.get(&room_id) else {
            return Ok(page);
        };
        let Some(requester) = projection.device_for_member_id(requester).cloned() else {
            return Ok(HttpSyncPage {
                entries: Vec::new(),
                next_after_seq: page.next_after_seq,
                has_more: page.has_more,
            });
        };

        let mut entries = Vec::new();
        let mut scanned_to_seq = request.after_seq;
        for entry in page.entries {
            scanned_to_seq = entry.seq;
            if projection.device_was_member_for_seq(&requester, entry.seq) {
                entries.push(entry);
            }
        }
        let next_after_seq = entries
            .last()
            .map(|entry| entry.seq)
            .unwrap_or(scanned_to_seq);
        Ok(HttpSyncPage {
            entries,
            next_after_seq,
            has_more: page.has_more,
        })
    }

    fn activate_room_membership_from_welcome(
        &self,
        message: &TransportMessage,
    ) -> Result<(), ServerHttpError> {
        let Ok(welcome) = serde_json::from_slice::<WelcomeRecord>(&message.payload) else {
            return Ok(());
        };
        if message.id.as_slice() != welcome.welcome_id.as_bytes() {
            return Ok(());
        }
        let mut rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex");
        let Some(projection) = rooms.get_mut(&welcome.room_id) else {
            return Ok(());
        };
        if !projection.activate_interval(&welcome.recipient, welcome.commit_seq) {
            return Ok(());
        }
        let projection = projection.clone();
        drop(rooms);

        if let Some(store) = &self.store {
            store.upsert_room_membership(&projection)?;
        }
        Ok(())
    }
}

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
        .route("/link-sessions", post(create_link_session))
        .route("/link-sessions/get", post(get_link_session))
        .route("/link-sessions/payload", post(upload_link_payload))
        .route("/link-sessions/claim", post(claim_link_payload))
        .route("/link-sessions/ack", post(ack_link_payload))
        .route("/link-sessions/release", post(release_link_claim))
        .route("/link-sessions/expire", post(expire_link_session))
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
    Ok(Json(state.put_blob_object(&headers, &body)?))
}

async fn download_blob_object(
    State(state): State<HttpServerState>,
    AxumPath(sha256): AxumPath<String>,
) -> Result<impl IntoResponse, ServerHttpError> {
    let object = state.get_blob_object(&sha256)?;
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
                room_id: room.room_id,
                after_seq: room.after_seq,
                seen_activity_received_at_ms: 0,
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

async fn create_link_session(
    State(state): State<HttpServerState>,
    Json(request): Json<CreateLinkSessionRequest>,
) -> Result<Json<HttpLinkSessionRecord>, ServerHttpError> {
    let record = state.create_link_session(request)?;
    Ok(Json(record))
}

async fn get_link_session(
    State(state): State<HttpServerState>,
    Json(request): Json<GetLinkSessionRequest>,
) -> Result<Json<Option<HttpLinkSessionRecord>>, ServerHttpError> {
    let record = state.get_link_session(request)?;
    Ok(Json(record))
}

async fn upload_link_payload(
    State(state): State<HttpServerState>,
    Json(request): Json<UploadLinkPayloadRequest>,
) -> Result<Json<HttpLinkSessionRecord>, ServerHttpError> {
    let record = state.upload_link_payload(request)?;
    Ok(Json(record))
}

async fn claim_link_payload(
    State(state): State<HttpServerState>,
    Json(request): Json<ClaimLinkPayloadRequest>,
) -> Result<Json<ClaimLinkPayloadResponse>, ServerHttpError> {
    let response = state.claim_link_payload(request)?;
    Ok(Json(response))
}

async fn ack_link_payload(
    State(state): State<HttpServerState>,
    Json(request): Json<AckLinkPayloadRequest>,
) -> Result<Json<AckLinkPayloadResponse>, ServerHttpError> {
    let response = state.ack_link_payload(request)?;
    Ok(Json(response))
}

async fn release_link_claim(
    State(state): State<HttpServerState>,
    Json(request): Json<ReleaseLinkClaimRequest>,
) -> Result<Json<ReleaseLinkClaimResponse>, ServerHttpError> {
    let response = state.release_link_claim(request)?;
    Ok(Json(response))
}

async fn expire_link_session(
    State(state): State<HttpServerState>,
    Json(request): Json<ExpireLinkSessionRequest>,
) -> Result<Json<ExpireLinkSessionResponse>, ServerHttpError> {
    let response = state.expire_link_session(request)?;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum PersistedOperation {
    PublishMessage {
        target: HttpPublishTarget,
        message: finitechat_transport::transport::TransportMessage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        idempotency_key: Option<String>,
    },
    PublishKeyPackage {
        publication: HttpKeyPackagePublication,
    },
    RevokeDevice {
        device: DeviceRef,
    },
    ClaimKeyPackage {
        owner: MemberId,
    },
    ClaimKeyPackages {
        owners: Vec<MemberId>,
    },
    ExpireKeyPackageLease {
        key_package_id: HttpKeyPackageId,
    },
}

impl PersistedOperation {
    fn kind(&self) -> &'static str {
        match self {
            Self::PublishMessage { .. } => "publish_message",
            Self::PublishKeyPackage { .. } => "publish_key_package",
            Self::RevokeDevice { .. } => "revoke_device",
            Self::ClaimKeyPackage { .. } => "claim_key_package",
            Self::ClaimKeyPackages { .. } => "claim_key_packages",
            Self::ExpireKeyPackageLease { .. } => "expire_key_package_lease",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PublishMessageFingerprint {
    target: HttpPublishTarget,
    message: finitechat_transport::transport::TransportMessage,
}

impl PublishMessageFingerprint {
    fn from_request(request: &PublishMessageRequest) -> Self {
        Self {
            target: request.target.clone(),
            message: request.message.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PublishIdempotencyRecord {
    fingerprint: PublishMessageFingerprint,
    receipt: HttpPublishReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct KeyPackageClaimFingerprint {
    owners: Vec<MemberId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct KeyPackageClaimIdempotencyRecord {
    fingerprint: KeyPackageClaimFingerprint,
    response: Vec<HttpKeyPackageClaim>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct KeyPackageInventoryRecord {
    key_package_id: HttpKeyPackageId,
    owner: MemberId,
    key_package: KeyPackage,
    state: KeyPackageInventoryState,
    finite_metadata: Option<FiniteKeyPackageMetadata>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum KeyPackageInventoryState {
    Available,
    Claimed,
    Consumed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FiniteKeyPackageMetadata {
    owner: DeviceRef,
    key_package_ref: String,
    key_package_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct WelcomeClaimRecord {
    recipient: MemberId,
    seq: HttpSequence,
    message: TransportMessage,
    state: WelcomeClaimState,
}

impl WelcomeClaimRecord {
    fn into_claimed_welcome(self) -> HttpClaimedWelcome {
        HttpClaimedWelcome {
            seq: self.seq,
            message: self.message,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WelcomeClaimState {
    Claimed,
    Acked,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct AccountRoomDirectoryRecord {
    account_id: String,
    room_id: String,
    record: Value,
}

/// Everything `from_sqlite_path` otherwise derives by replaying the full
/// operation log. Snapshotting it makes startup snapshot + tail replay, per
/// the standing constraint that full-history replay is a rare recovery
/// action (ADR 0003).
#[derive(Serialize, Deserialize)]
struct DurableStateSnapshot {
    service: HttpDeliveryService,
    // Stored as a list: JSON maps need string keys, and the record carries
    // its own id.
    key_package_inventory: Vec<KeyPackageInventoryRecord>,
    revoked_devices: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct AccountRoomDirectoryMutation {
    deletes: Vec<(String, String)>,
    upserts: Vec<AccountRoomDirectoryRecord>,
}

#[derive(Clone, Debug, PartialEq)]
struct PublishMutation {
    operation: Option<PersistedOperation>,
    idempotency_key: String,
    record: PublishIdempotencyRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PushWakeOutboxRecord {
    wake_id: String,
    room_id: String,
    seq: HttpSequence,
    sender: DeviceRef,
    state: PushWakeOutboxState,
}

impl PushWakeOutboxRecord {
    fn from_effect(effect: &HttpApplicationDeliveryEffect) -> Option<Self> {
        effect
            .delivery_policy
            .creates_push()
            .then(|| PushWakeOutboxRecord {
                wake_id: effect.message_id.clone(),
                room_id: effect.room_id.clone(),
                seq: effect.seq,
                sender: effect.sender.clone(),
                state: PushWakeOutboxState::Pending { attempts: 0 },
            })
    }

    fn attempts(&self) -> u32 {
        match self.state {
            PushWakeOutboxState::Pending { attempts }
            | PushWakeOutboxState::Leased { attempts, .. } => attempts,
        }
    }

    fn claimable_at(&self, now_ms: u64) -> bool {
        match self.state {
            PushWakeOutboxState::Pending { .. } => true,
            PushWakeOutboxState::Leased {
                lease_expires_at_ms,
                ..
            } => lease_expires_at_ms <= now_ms,
        }
    }

    fn claimed(&self, now_ms: u64, lease_ms: u64) -> Self {
        let mut next = self.clone();
        let attempts = self.attempts().saturating_add(1);
        next.state = PushWakeOutboxState::Leased {
            lease_expires_at_ms: now_ms.saturating_add(lease_ms),
            attempts,
        };
        next
    }

    fn released_for_retry(&self) -> Self {
        let mut next = self.clone();
        next.state = PushWakeOutboxState::Pending {
            attempts: self.attempts(),
        };
        next
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PushWakeOutboxState {
    Pending {
        attempts: u32,
    },
    Leased {
        lease_expires_at_ms: u64,
        attempts: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct HttpRoomMembershipProjection {
    room_id: String,
    mls_group_id: String,
    current_epoch: u64,
    last_seq: HttpSequence,
    status: RoomStatus,
    #[serde(default = "default_membership_complete")]
    membership_complete: bool,
    /// Accounts allowed to change membership for other accounts (ADR 0003 §2
    /// as amended by ADR 0004 §4). Creator-initialized at typed bootstrap.
    #[serde(default)]
    admins: BTreeSet<String>,
    /// Accounts that left (ADR 0003 §3) and still await the MLS removal
    /// commit. The server already filters their delivery; this marker lets
    /// member workers discover the pending cryptographic cleanup.
    #[serde(default)]
    departed: BTreeSet<String>,
    /// Per-room protocol slots (ADR 0003 §1).
    #[serde(default)]
    protocol: RoomProtocol,
    #[serde(default)]
    membership: BTreeMap<String, DeviceMembership>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ObservedRoomHead {
    current_epoch: u64,
    last_seq: HttpSequence,
    raw_commit_without_projection: bool,
}

impl HttpRoomMembershipProjection {
    fn device_for_member_id(&self, member_id: &MemberId) -> Option<&DeviceRef> {
        self.membership
            .values()
            .map(|membership| &membership.device)
            .find(|device| {
                member_id_for_device(device)
                    .ok()
                    .is_some_and(|candidate| candidate == *member_id)
            })
    }

    fn tracks_device(&self, device: &DeviceRef) -> bool {
        self.membership.contains_key(&DeviceMembership::key(device))
    }

    fn device_active_at_head(&self, device: &DeviceRef) -> bool {
        self.membership
            .get(&DeviceMembership::key(device))
            .map(|membership| {
                membership.intervals.iter().any(|interval| {
                    interval.active
                        && interval.start_seq <= self.last_seq
                        && interval.end_seq.is_none()
                })
            })
            .unwrap_or(false)
    }

    fn device_was_member_for_seq(&self, device: &DeviceRef, seq: HttpSequence) -> bool {
        self.membership
            .get(&DeviceMembership::key(device))
            .map(|membership| {
                membership.intervals.iter().any(|interval| {
                    interval.start_seq <= seq && interval.end_seq.is_none_or(|end| seq <= end)
                })
            })
            .unwrap_or(false)
    }

    fn current_or_pending_device_count_for_account(&self, account_id: &str) -> usize {
        self.membership
            .values()
            .filter(|membership| membership.device.account_id == account_id)
            .filter(|membership| {
                membership
                    .intervals
                    .iter()
                    .any(|interval| interval.end_seq.is_none())
            })
            .count()
    }

    fn device_current_or_pending_at_head(&self, device: &DeviceRef) -> bool {
        self.membership
            .get(&DeviceMembership::key(device))
            .map(|membership| {
                membership
                    .intervals
                    .iter()
                    .any(|interval| interval.end_seq.is_none())
            })
            .unwrap_or(false)
    }

    fn activate_interval(&mut self, device: &DeviceRef, start_seq: HttpSequence) -> bool {
        let Some(membership) = self.membership.get_mut(&DeviceMembership::key(device)) else {
            return false;
        };
        let Some(interval) = membership
            .intervals
            .iter_mut()
            .find(|interval| interval.start_seq == start_seq && !interval.active)
        else {
            return false;
        };
        interval.active = true;
        true
    }
}

#[derive(Debug)]
struct SqliteHttpDeliveryStore {
    conn: Mutex<Connection>,
}

impl SqliteHttpDeliveryStore {
    fn open(path: impl AsRef<Path>) -> Result<Self, rusqlite::Error> {
        let store = Self {
            conn: Mutex::new(Connection::open(path.as_ref())?),
        };
        let conn = store.connection();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
            PRAGMA synchronous = FULL;
            PRAGMA busy_timeout = 5000;
            CREATE TABLE IF NOT EXISTS http_delivery_ops (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                body_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_push_tokens (
                device_key TEXT PRIMARY KEY,
                record_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_push_wakes (
                wake_id TEXT PRIMARY KEY,
                record_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_state_snapshots (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                last_op_seq INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_publish_idempotency (
                idempotency_key TEXT PRIMARY KEY,
                fingerprint_json TEXT NOT NULL,
                receipt_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_key_package_claim_idempotency (
                idempotency_key TEXT PRIMARY KEY,
                fingerprint_json TEXT NOT NULL,
                response_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_key_package_inventory (
                key_package_id_json TEXT PRIMARY KEY,
                owner_json TEXT NOT NULL,
                state_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_link_sessions (
                link_session_id TEXT PRIMARY KEY,
                record_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_nostr_profiles (
                account_id TEXT PRIMARY KEY,
                record_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_account_rooms (
                account_id TEXT NOT NULL,
                room_id TEXT NOT NULL,
                record_json TEXT NOT NULL,
                PRIMARY KEY(account_id, room_id)
            );
            CREATE TABLE IF NOT EXISTS http_room_memberships (
                room_id TEXT PRIMARY KEY,
                projection_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_application_delivery_effects (
                message_id TEXT PRIMARY KEY,
                room_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                sender_json TEXT NOT NULL,
                delivery_policy_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_welcome_claims (
                message_id_json TEXT PRIMARY KEY,
                recipient_json TEXT NOT NULL,
                seq INTEGER NOT NULL,
                message_json TEXT NOT NULL,
                state_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS http_blob_objects (
                sha256 TEXT PRIMARY KEY,
                size_bytes INTEGER NOT NULL,
                content_type TEXT NOT NULL,
                ciphertext BLOB NOT NULL
            );",
        )?;
        ensure_blob_content_type_column(&conn)?;
        drop(conn);
        Ok(store)
    }

    fn append_operation(&self, operation: &PersistedOperation) -> Result<(), DurableStoreError> {
        let body_json = serde_json::to_string(operation)?;
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
            params![operation.kind(), body_json],
        )?;
        Ok(())
    }

    fn append_publish_mutation(
        &self,
        operation: Option<&PersistedOperation>,
        idempotency: Option<(&str, &PublishIdempotencyRecord)>,
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        if let Some(operation) = operation {
            transaction.execute(
                "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
                params![operation.kind(), serde_json::to_string(operation)?],
            )?;
        }
        if let Some((idempotency_key, record)) = idempotency {
            transaction.execute(
                "INSERT INTO http_publish_idempotency (
                    idempotency_key,
                    fingerprint_json,
                    receipt_json
                ) VALUES (?1, ?2, ?3)",
                params![
                    idempotency_key,
                    serde_json::to_string(&record.fingerprint)?,
                    serde_json::to_string(&record.receipt)?,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn append_submit_commit_mutation(
        &self,
        publish_mutations: &[PublishMutation],
        account_room_mutation: &AccountRoomDirectoryMutation,
        room_membership_projection: &HttpRoomMembershipProjection,
        key_package_inventory_mutation: &[KeyPackageInventoryRecord],
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        for mutation in publish_mutations {
            if let Some(operation) = &mutation.operation {
                transaction.execute(
                    "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
                    params![operation.kind(), serde_json::to_string(operation)?],
                )?;
            }
            transaction.execute(
                "INSERT INTO http_publish_idempotency (
                    idempotency_key,
                    fingerprint_json,
                    receipt_json
                ) VALUES (?1, ?2, ?3)",
                params![
                    mutation.idempotency_key,
                    serde_json::to_string(&mutation.record.fingerprint)?,
                    serde_json::to_string(&mutation.record.receipt)?,
                ],
            )?;
        }
        for (account_id, room_id) in &account_room_mutation.deletes {
            transaction.execute(
                "DELETE FROM http_account_rooms WHERE account_id = ?1 AND room_id = ?2",
                params![account_id, room_id],
            )?;
        }
        for record in &account_room_mutation.upserts {
            transaction.execute(
                "INSERT INTO http_account_rooms (account_id, room_id, record_json)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, room_id) DO UPDATE SET
                    record_json = excluded.record_json",
                params![
                    record.account_id,
                    record.room_id,
                    serde_json::to_string(&record.record)?,
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO http_room_memberships (room_id, projection_json)
             VALUES (?1, ?2)
             ON CONFLICT(room_id) DO UPDATE SET
                projection_json = excluded.projection_json",
            params![
                room_membership_projection.room_id,
                serde_json::to_string(room_membership_projection)?,
            ],
        )?;
        for record in key_package_inventory_mutation {
            upsert_key_package_inventory_in_transaction(&transaction, record)?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn append_application_event_mutation(
        &self,
        publish_mutation: Option<&PublishMutation>,
        room_membership_projection: Option<&HttpRoomMembershipProjection>,
        effect: Option<&HttpApplicationDeliveryEffect>,
        push_wake: Option<&PushWakeOutboxRecord>,
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        if let Some(mutation) = publish_mutation {
            if let Some(operation) = &mutation.operation {
                transaction.execute(
                    "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
                    params![operation.kind(), serde_json::to_string(operation)?],
                )?;
            }
            transaction.execute(
                "INSERT INTO http_publish_idempotency (
                    idempotency_key,
                    fingerprint_json,
                    receipt_json
                ) VALUES (?1, ?2, ?3)",
                params![
                    mutation.idempotency_key,
                    serde_json::to_string(&mutation.record.fingerprint)?,
                    serde_json::to_string(&mutation.record.receipt)?,
                ],
            )?;
        }
        if let Some(projection) = room_membership_projection {
            transaction.execute(
                "INSERT INTO http_room_memberships (room_id, projection_json)
                 VALUES (?1, ?2)
                 ON CONFLICT(room_id) DO UPDATE SET
                    projection_json = excluded.projection_json",
                params![projection.room_id, serde_json::to_string(projection)?],
            )?;
        }
        if let Some(effect) = effect {
            upsert_application_effect_in_transaction(&transaction, effect)?;
        }
        if let Some(push_wake) = push_wake {
            upsert_push_wake_in_transaction(&transaction, push_wake)?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn append_key_package_claim_mutation(
        &self,
        operation: Option<&PersistedOperation>,
        idempotency: Option<(&str, &KeyPackageClaimIdempotencyRecord)>,
        inventory_records: &[KeyPackageInventoryRecord],
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        if let Some(operation) = operation {
            transaction.execute(
                "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
                params![operation.kind(), serde_json::to_string(operation)?],
            )?;
        }
        if let Some((idempotency_key, record)) = idempotency {
            transaction.execute(
                "INSERT INTO http_key_package_claim_idempotency (
                    idempotency_key,
                    fingerprint_json,
                    response_json
                ) VALUES (?1, ?2, ?3)",
                params![
                    idempotency_key,
                    serde_json::to_string(&record.fingerprint)?,
                    serde_json::to_string(&record.response)?,
                ],
            )?;
        }
        for record in inventory_records {
            upsert_key_package_inventory_in_transaction(&transaction, record)?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn append_key_package_inventory_operation(
        &self,
        operation: &PersistedOperation,
        inventory_record: &KeyPackageInventoryRecord,
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        transaction.execute(
            "INSERT INTO http_delivery_ops (kind, body_json) VALUES (?1, ?2)",
            params![operation.kind(), serde_json::to_string(operation)?],
        )?;
        upsert_key_package_inventory_in_transaction(&transaction, inventory_record)?;
        transaction.commit()?;
        Ok(())
    }

    fn load_operations_after(
        &self,
        after_seq: i64,
    ) -> Result<Vec<PersistedOperation>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn
            .prepare("SELECT body_json FROM http_delivery_ops WHERE seq > ?1 ORDER BY seq ASC")?;
        let rows = statement.query_map(params![after_seq], |row| row.get::<_, String>(0))?;
        let mut operations = Vec::new();
        for row in rows {
            operations.push(serde_json::from_str(&row?)?);
        }
        Ok(operations)
    }

    fn max_operation_seq(&self) -> Result<i64, DurableStoreError> {
        let conn = self.connection();
        let max: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM http_delivery_ops",
            [],
            |row| row.get(0),
        )?;
        Ok(max)
    }

    fn load_push_tokens(&self) -> Result<BTreeMap<String, PushTokenRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare("SELECT device_key, record_json FROM http_push_tokens")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut tokens = BTreeMap::new();
        for row in rows {
            let (key, json) = row?;
            tokens.insert(key, serde_json::from_str(&json)?);
        }
        Ok(tokens)
    }

    fn load_push_wakes(&self) -> Result<BTreeMap<String, PushWakeOutboxRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement =
            conn.prepare("SELECT wake_id, record_json FROM http_push_wakes ORDER BY wake_id ASC")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut wakes = BTreeMap::new();
        for row in rows {
            let (wake_id, json) = row?;
            wakes.insert(wake_id, serde_json::from_str(&json)?);
        }
        Ok(wakes)
    }

    fn load_blob_objects(&self) -> Result<BTreeMap<String, BlobObject>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT sha256, size_bytes, content_type, ciphertext
             FROM http_blob_objects
             ORDER BY sha256 ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?;
        let mut objects = BTreeMap::new();
        for row in rows {
            let (sha256, size_bytes, content_type, bytes) = row?;
            if size_bytes != bytes.len() as u64 || sha256 != sha256_hex(&bytes) {
                return Err(DurableStoreError::BlobObjectCorrupt { sha256 });
            }
            objects.insert(
                sha256,
                BlobObject {
                    content_type,
                    bytes,
                },
            );
        }
        Ok(objects)
    }

    fn upsert_blob_object(
        &self,
        sha256: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_blob_objects (sha256, size_bytes, content_type, ciphertext)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(sha256) DO NOTHING",
            params![sha256, bytes.len() as u64, content_type, bytes],
        )?;
        Ok(())
    }

    fn upsert_push_token(&self, record: &PushTokenRecord) -> Result<(), DurableStoreError> {
        let json = serde_json::to_string(record)?;
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_push_tokens (device_key, record_json)
             VALUES (?1, ?2)
             ON CONFLICT(device_key) DO UPDATE SET record_json = excluded.record_json",
            params![DeviceMembership::key(&record.device), json],
        )?;
        Ok(())
    }

    fn delete_push_token(&self, device_key: &str) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM http_push_tokens WHERE device_key = ?1",
            params![device_key],
        )?;
        Ok(())
    }

    fn upsert_push_wakes(&self, records: &[PushWakeOutboxRecord]) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        for record in records {
            upsert_push_wake_in_transaction(&transaction, record)?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn delete_push_wake(&self, wake_id: &str) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM http_push_wakes WHERE wake_id = ?1",
            params![wake_id],
        )?;
        Ok(())
    }

    fn load_state_snapshot(
        &self,
    ) -> Result<Option<(i64, DurableStateSnapshot)>, DurableStoreError> {
        let conn = self.connection();
        let row = conn
            .query_row(
                "SELECT last_op_seq, snapshot_json FROM http_state_snapshots WHERE id = 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        match row {
            Some((seq, json)) => Ok(Some((seq, serde_json::from_str(&json)?))),
            None => Ok(None),
        }
    }

    fn save_state_snapshot(
        &self,
        last_op_seq: i64,
        snapshot: &DurableStateSnapshot,
    ) -> Result<(), DurableStoreError> {
        let json = serde_json::to_string(snapshot)?;
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_state_snapshots (id, last_op_seq, snapshot_json)
             VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
                 last_op_seq = excluded.last_op_seq,
                 snapshot_json = excluded.snapshot_json",
            params![last_op_seq, json],
        )?;
        Ok(())
    }

    fn load_publish_idempotency(
        &self,
    ) -> Result<HashMap<String, PublishIdempotencyRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT idempotency_key, fingerprint_json, receipt_json FROM http_publish_idempotency",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut idempotency = HashMap::new();
        for row in rows {
            let (key, fingerprint_json, receipt_json) = row?;
            idempotency.insert(
                key,
                PublishIdempotencyRecord {
                    fingerprint: serde_json::from_str(&fingerprint_json)?,
                    receipt: serde_json::from_str(&receipt_json)?,
                },
            );
        }
        Ok(idempotency)
    }

    fn load_key_package_claim_idempotency(
        &self,
    ) -> Result<HashMap<String, KeyPackageClaimIdempotencyRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT idempotency_key, fingerprint_json, response_json
             FROM http_key_package_claim_idempotency",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut idempotency = HashMap::new();
        for row in rows {
            let (key, fingerprint_json, response_json) = row?;
            idempotency.insert(
                key,
                KeyPackageClaimIdempotencyRecord {
                    fingerprint: serde_json::from_str(&fingerprint_json)?,
                    response: serde_json::from_str(&response_json)?,
                },
            );
        }
        Ok(idempotency)
    }

    fn upsert_key_package_inventory(
        &self,
        record: &KeyPackageInventoryRecord,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_key_package_inventory (
                key_package_id_json,
                owner_json,
                state_json
            ) VALUES (?1, ?2, ?3)
            ON CONFLICT(key_package_id_json) DO UPDATE SET
                owner_json = excluded.owner_json,
                state_json = excluded.state_json",
            params![
                serde_json::to_string(&record.key_package_id)?,
                serde_json::to_string(&record.owner)?,
                serde_json::to_string(&record.state)?,
            ],
        )?;
        Ok(())
    }

    fn load_key_package_inventory(
        &self,
    ) -> Result<HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT key_package_id_json, owner_json, state_json FROM http_key_package_inventory",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut inventory = HashMap::new();
        for row in rows {
            let (key_package_id_json, owner_json, state_json) = row?;
            let key_package_id: HttpKeyPackageId = serde_json::from_str(&key_package_id_json)?;
            inventory.insert(
                key_package_id.clone(),
                KeyPackageInventoryRecord {
                    key_package_id,
                    owner: serde_json::from_str(&owner_json)?,
                    key_package: KeyPackage::new(Vec::new()),
                    state: serde_json::from_str(&state_json)?,
                    finite_metadata: None,
                },
            );
        }
        Ok(inventory)
    }

    fn upsert_link_session(&self, record: &HttpLinkSessionRecord) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_link_sessions (link_session_id, record_json)
             VALUES (?1, ?2)
             ON CONFLICT(link_session_id) DO UPDATE SET
                record_json = excluded.record_json",
            params![record.link_session_id, serde_json::to_string(record)?],
        )?;
        Ok(())
    }

    fn load_link_sessions(
        &self,
    ) -> Result<BTreeMap<String, HttpLinkSessionRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT link_session_id, record_json
             FROM http_link_sessions
             ORDER BY link_session_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut sessions = BTreeMap::new();
        for row in rows {
            let (link_session_id, record_json) = row?;
            sessions.insert(link_session_id, serde_json::from_str(&record_json)?);
        }
        Ok(sessions)
    }

    fn upsert_nostr_profile(&self, record: &NostrProfileRecord) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_nostr_profiles (account_id, record_json)
             VALUES (?1, ?2)
             ON CONFLICT(account_id) DO UPDATE SET
                record_json = excluded.record_json",
            params![record.account_id, serde_json::to_string(record)?],
        )?;
        Ok(())
    }

    fn load_nostr_profiles(
        &self,
    ) -> Result<BTreeMap<String, NostrProfileRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT account_id, record_json
             FROM http_nostr_profiles
             ORDER BY account_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut profiles = BTreeMap::new();
        for row in rows {
            let (account_id, record_json) = row?;
            profiles.insert(account_id, serde_json::from_str(&record_json)?);
        }
        Ok(profiles)
    }

    fn upsert_account_room(
        &self,
        record: &AccountRoomDirectoryRecord,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_account_rooms (account_id, room_id, record_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id, room_id) DO UPDATE SET
                record_json = excluded.record_json",
            params![
                record.account_id,
                record.room_id,
                serde_json::to_string(&record.record)?,
            ],
        )?;
        Ok(())
    }

    fn load_account_room_directory(
        &self,
    ) -> Result<BTreeMap<String, BTreeMap<String, Value>>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT account_id, room_id, record_json
             FROM http_account_rooms
             ORDER BY account_id ASC, room_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut directory = BTreeMap::new();
        for row in rows {
            let (account_id, room_id, record_json) = row?;
            directory
                .entry(account_id)
                .or_insert_with(BTreeMap::new)
                .insert(room_id, serde_json::from_str(&record_json)?);
        }
        Ok(directory)
    }

    fn delete_account_room(
        &self,
        account_id: &str,
        room_id: &str,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM http_account_rooms WHERE account_id = ?1 AND room_id = ?2",
            params![account_id, room_id],
        )?;
        Ok(())
    }

    fn upsert_room_membership(
        &self,
        projection: &HttpRoomMembershipProjection,
    ) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_room_memberships (room_id, projection_json)
             VALUES (?1, ?2)
             ON CONFLICT(room_id) DO UPDATE SET
                projection_json = excluded.projection_json",
            params![&projection.room_id, serde_json::to_string(projection)?,],
        )?;
        Ok(())
    }

    fn upsert_room_repair_state(
        &self,
        projection: &HttpRoomMembershipProjection,
        account_records: &[AccountRoomDirectoryRecord],
    ) -> Result<(), DurableStoreError> {
        let mut conn = self.connection();
        let transaction = conn.transaction()?;
        transaction.execute(
            "INSERT INTO http_room_memberships (room_id, projection_json)
             VALUES (?1, ?2)
             ON CONFLICT(room_id) DO UPDATE SET
                projection_json = excluded.projection_json",
            params![projection.room_id, serde_json::to_string(projection)?],
        )?;
        for record in account_records {
            transaction.execute(
                "INSERT INTO http_account_rooms (account_id, room_id, record_json)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, room_id) DO UPDATE SET
                    record_json = excluded.record_json",
                params![
                    record.account_id,
                    record.room_id,
                    serde_json::to_string(&record.record)?,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn load_room_memberships(
        &self,
    ) -> Result<BTreeMap<String, HttpRoomMembershipProjection>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT room_id, projection_json
             FROM http_room_memberships
             ORDER BY room_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut rooms = BTreeMap::new();
        for row in rows {
            let (room_id, projection_json) = row?;
            rooms.insert(room_id, serde_json::from_str(&projection_json)?);
        }
        Ok(rooms)
    }

    fn load_application_effects(
        &self,
    ) -> Result<BTreeMap<String, HttpApplicationDeliveryEffect>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT message_id, room_id, seq, sender_json, delivery_policy_json
             FROM http_application_delivery_effects
             ORDER BY message_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut effects = BTreeMap::new();
        for row in rows {
            let (message_id, room_id, seq, sender_json, delivery_policy_json) = row?;
            effects.insert(
                message_id.clone(),
                HttpApplicationDeliveryEffect {
                    room_id,
                    seq,
                    message_id,
                    sender: serde_json::from_str(&sender_json)?,
                    delivery_policy: serde_json::from_str(&delivery_policy_json)?,
                },
            );
        }
        Ok(effects)
    }

    fn upsert_welcome_claim(&self, record: &WelcomeClaimRecord) -> Result<(), DurableStoreError> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO http_welcome_claims (
                message_id_json,
                recipient_json,
                seq,
                message_json,
                state_json
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(message_id_json) DO UPDATE SET
                recipient_json = excluded.recipient_json,
                seq = excluded.seq,
                message_json = excluded.message_json,
                state_json = excluded.state_json",
            params![
                serde_json::to_string(&record.message.id)?,
                serde_json::to_string(&record.recipient)?,
                record.seq,
                serde_json::to_string(&record.message)?,
                serde_json::to_string(&record.state)?,
            ],
        )?;
        Ok(())
    }

    fn load_welcome_claims(
        &self,
    ) -> Result<HashMap<MessageId, WelcomeClaimRecord>, DurableStoreError> {
        let conn = self.connection();
        let mut statement = conn.prepare(
            "SELECT message_id_json, recipient_json, seq, message_json, state_json
             FROM http_welcome_claims",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut claims = HashMap::new();
        for row in rows {
            let (message_id_json, recipient_json, seq, message_json, state_json) = row?;
            let message_id = serde_json::from_str(&message_id_json)?;
            claims.insert(
                message_id,
                WelcomeClaimRecord {
                    recipient: serde_json::from_str(&recipient_json)?,
                    seq,
                    message: serde_json::from_str(&message_json)?,
                    state: serde_json::from_str(&state_json)?,
                },
            );
        }
        Ok(claims)
    }

    fn connection(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .expect("HTTP delivery store connection mutex")
    }
}

fn ensure_blob_content_type_column(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut statement = conn.prepare("PRAGMA table_info(http_blob_objects)")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == "content_type" {
            return Ok(());
        }
    }
    conn.execute(
        "ALTER TABLE http_blob_objects
         ADD COLUMN content_type TEXT NOT NULL DEFAULT 'application/octet-stream'",
        [],
    )?;
    Ok(())
}

fn upsert_key_package_inventory_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    record: &KeyPackageInventoryRecord,
) -> Result<(), DurableStoreError> {
    transaction.execute(
        "INSERT INTO http_key_package_inventory (
            key_package_id_json,
            owner_json,
            state_json
        ) VALUES (?1, ?2, ?3)
        ON CONFLICT(key_package_id_json) DO UPDATE SET
            owner_json = excluded.owner_json,
            state_json = excluded.state_json",
        params![
            serde_json::to_string(&record.key_package_id)?,
            serde_json::to_string(&record.owner)?,
            serde_json::to_string(&record.state)?,
        ],
    )?;
    Ok(())
}

fn upsert_application_effect_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    effect: &HttpApplicationDeliveryEffect,
) -> Result<(), DurableStoreError> {
    transaction.execute(
        "INSERT INTO http_application_delivery_effects (
            message_id,
            room_id,
            seq,
            sender_json,
            delivery_policy_json
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(message_id) DO NOTHING",
        params![
            &effect.message_id,
            &effect.room_id,
            effect.seq,
            serde_json::to_string(&effect.sender)?,
            serde_json::to_string(&effect.delivery_policy)?,
        ],
    )?;
    Ok(())
}

fn upsert_push_wake_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    record: &PushWakeOutboxRecord,
) -> Result<(), DurableStoreError> {
    transaction.execute(
        "INSERT INTO http_push_wakes (wake_id, record_json)
         VALUES (?1, ?2)
         ON CONFLICT(wake_id) DO UPDATE SET record_json = excluded.record_json",
        params![&record.wake_id, serde_json::to_string(record)?],
    )?;
    Ok(())
}

fn blob_content_type(headers: &HeaderMap) -> Result<&str, ServerHttpError> {
    let Some(value) = headers.get(header::CONTENT_TYPE) else {
        return Err(ServerHttpError::InvalidBlobRequest {
            reason: "blob upload must include a content-type header".to_owned(),
        });
    };
    let content_type = value
        .to_str()
        .map_err(|_| ServerHttpError::InvalidBlobRequest {
            reason: "blob upload content-type header is not valid UTF-8".to_owned(),
        })?;
    Ok(content_type.split(';').next().unwrap_or_default().trim())
}

fn blob_url(public_url: Option<&str>, headers: &HeaderMap, sha256: &str) -> String {
    if let Some(public_url) = public_url {
        return format!("{public_url}/blobs/{sha256}");
    }
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|value| *value == "http" || *value == "https")
        .unwrap_or("http");
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("localhost");
    format!("{scheme}://{host}/blobs/{sha256}")
}

fn normalize_public_url(public_url: &str) -> Result<String, HttpServerConfigurationError> {
    let trimmed = public_url.trim();
    let parsed = reqwest::Url::parse(trimmed).map_err(|error| {
        HttpServerConfigurationError::InvalidPublicUrl {
            reason: error.to_string(),
        }
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(HttpServerConfigurationError::InvalidPublicUrl {
            reason: "scheme must be http or https".to_owned(),
        });
    }
    if parsed.host_str().is_none() {
        return Err(HttpServerConfigurationError::InvalidPublicUrl {
            reason: "host is required".to_owned(),
        });
    }
    if parsed.path() != "/" {
        return Err(HttpServerConfigurationError::InvalidPublicUrl {
            reason: "URL must be a bare origin without a path".to_owned(),
        });
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(HttpServerConfigurationError::InvalidPublicUrl {
            reason: "credentials are not allowed".to_owned(),
        });
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(HttpServerConfigurationError::InvalidPublicUrl {
            reason: "query and fragment are not allowed".to_owned(),
        });
    }
    Ok(trimmed.trim_end_matches('/').to_owned())
}

fn normalize_blob_upload_content_type(content_type: &str) -> Result<&'static str, ServerHttpError> {
    match content_type.trim().to_ascii_lowercase().as_str() {
        BLOB_CIPHERTEXT_CONTENT_TYPE => Ok(BLOB_CIPHERTEXT_CONTENT_TYPE),
        "image/jpeg" | "image/jpg" => Ok("image/jpeg"),
        "image/png" => Ok("image/png"),
        "image/gif" => Ok("image/gif"),
        "image/webp" => Ok("image/webp"),
        other => Err(ServerHttpError::InvalidBlobRequest {
            reason: format!("blob upload content type is not supported: {other}"),
        }),
    }
}

fn validate_blob_upload(bytes: &[u8], content_type: &str) -> Result<(), ServerHttpError> {
    if content_type == BLOB_CIPHERTEXT_CONTENT_TYPE {
        return BlobPutRequest {
            ciphertext: bytes,
            content_type,
        }
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidBlobRequest {
            reason: error.to_string(),
        });
    }
    validate_bytes_non_empty("blob.bytes", bytes.len()).map_err(|error| {
        ServerHttpError::InvalidBlobRequest {
            reason: error.to_string(),
        }
    })?;
    validate_bytes_len(
        "blob.bytes",
        bytes.len(),
        MAX_PUBLIC_IMAGE_BLOB_BYTES as u32,
    )
    .map_err(|error| ServerHttpError::InvalidBlobRequest {
        reason: error.to_string(),
    })?;
    if public_image_blob_magic_matches(bytes, content_type) {
        return Ok(());
    }
    Err(ServerHttpError::InvalidBlobRequest {
        reason: format!("blob bytes do not match {content_type}"),
    })
}

fn public_image_blob_magic_matches(bytes: &[u8], content_type: &str) -> bool {
    match content_type {
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

fn validate_blob_sha256(sha256: &str) -> Result<(), ServerHttpError> {
    if sha256.len() != 64 {
        return Err(ServerHttpError::InvalidBlobRequest {
            reason: format!(
                "blob sha256 must be 64 lowercase hex chars, got {}",
                sha256.len()
            ),
        });
    }
    if sha256
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Ok(());
    }
    Err(ServerHttpError::InvalidBlobRequest {
        reason: "blob sha256 must use lowercase hex".to_owned(),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Result of a read-only publish admission check inside a typed commit.
struct CheckedPublish {
    receipt: HttpPublishReceipt,
    /// True when the publish must be applied to the live service after the
    /// durable rows are persisted; false for exact replays.
    fresh: bool,
    mutation: Option<PublishMutation>,
}

/// Read-only form of the old candidate publish: validates one publish inside
/// a typed commit against live state and returns the receipt it would
/// produce, whether it still needs applying, and the durable rows to
/// persist. Distinct queues and idempotency keys per publish are guaranteed
/// by typed-commit validation (duplicate adds are rejected), so a batch of
/// these checks against the same live state predicts seqs correctly.
fn check_publish_request(
    service: &HttpDeliveryService,
    idempotency: &HashMap<String, PublishIdempotencyRecord>,
    request: &PublishMessageRequest,
) -> Result<CheckedPublish, ServerHttpError> {
    let Some(idempotency_key) = request.idempotency_key.clone() else {
        let (receipt, fresh) = match service.check_publish(&request.target, &request.message)? {
            HttpPublishCheck::DuplicateReplay(receipt) => (receipt, false),
            HttpPublishCheck::Fresh(receipt) => (receipt, true),
        };
        return Ok(CheckedPublish {
            receipt,
            fresh,
            mutation: None,
        });
    };
    if idempotency_key.is_empty() {
        return Err(ServerHttpError::InvalidIdempotencyKey);
    }

    let fingerprint = PublishMessageFingerprint::from_request(request);
    if let Some(record) = idempotency.get(&idempotency_key) {
        if record.fingerprint == fingerprint {
            return Ok(CheckedPublish {
                receipt: record.receipt.clone(),
                fresh: false,
                mutation: None,
            });
        }
        return Err(ServerHttpError::IdempotencyConflict { idempotency_key });
    }

    let (receipt, fresh) = match service.check_publish(&request.target, &request.message)? {
        HttpPublishCheck::DuplicateReplay(receipt) => (receipt, false),
        HttpPublishCheck::Fresh(receipt) => (receipt, true),
    };
    let operation = fresh.then(|| PersistedOperation::PublishMessage {
        target: request.target.clone(),
        message: request.message.clone(),
        idempotency_key: Some(idempotency_key.clone()),
    });
    let record = PublishIdempotencyRecord {
        fingerprint,
        receipt: receipt.clone(),
    };

    Ok(CheckedPublish {
        receipt,
        fresh,
        mutation: Some(PublishMutation {
            operation,
            idempotency_key,
            record,
        }),
    })
}

/// Read-only admission check for a typed event publish. Returns the receipt
/// the publish would produce plus the durable mutation to persist before
/// applying. Returns `(receipt, None)` for an exact idempotent replay.
fn check_typed_event_publish(
    service: &HttpDeliveryService,
    idempotency: &HashMap<String, PublishIdempotencyRecord>,
    request: &PublishMessageRequest,
    message_id: &str,
) -> Result<(HttpPublishReceipt, Option<PublishMutation>), ServerHttpError> {
    let Some(idempotency_key) = request.idempotency_key.clone() else {
        return Err(ServerHttpError::InvalidIdempotencyKey);
    };
    if idempotency_key.is_empty() {
        return Err(ServerHttpError::InvalidIdempotencyKey);
    }

    let fingerprint = PublishMessageFingerprint::from_request(request);
    if let Some(record) = idempotency.get(&idempotency_key) {
        if record.fingerprint == fingerprint {
            return Ok((record.receipt.clone(), None));
        }
        return Err(ServerHttpError::IdempotencyConflict { idempotency_key });
    }

    let typed_message_id = MessageId::new(message_id.as_bytes().to_vec());
    let receipt = match service.check_publish(&request.target, &request.message) {
        Ok(HttpPublishCheck::Fresh(receipt)) => receipt,
        Ok(HttpPublishCheck::DuplicateReplay(_))
        | Err(HttpServerError::ConflictingMessageId { .. }) => {
            return Err(ServerHttpError::DuplicateMessageId {
                message_id: typed_message_id,
            });
        }
        Err(error) => return Err(error.into()),
    };

    let operation = PersistedOperation::PublishMessage {
        target: request.target.clone(),
        message: request.message.clone(),
        idempotency_key: Some(idempotency_key.clone()),
    };
    let record = PublishIdempotencyRecord {
        fingerprint,
        receipt: receipt.clone(),
    };

    Ok((
        receipt,
        Some(PublishMutation {
            operation: Some(operation),
            idempotency_key,
            record,
        }),
    ))
}

/// Compute the room-membership `last_seq` advance for an accepted typed
/// event: returns the updated projection to persist and later insert,
/// without touching the map.
fn check_room_event_acceptance(
    rooms: &BTreeMap<String, HttpRoomMembershipProjection>,
    room_id: &str,
    accepted_seq: HttpSequence,
) -> Option<HttpRoomMembershipProjection> {
    let projection = rooms.get(room_id)?;
    if projection.last_seq >= accepted_seq {
        return None;
    }
    let mut updated = projection.clone();
    updated.last_seq = accepted_seq;
    Some(updated)
}

/// Validate a delivery effect against the stored projection and return the
/// row to persist and later insert, without touching the map. Exact replays
/// return `None`; conflicting policies for the same message id are rejected.
fn check_application_delivery_effect(
    effects: &BTreeMap<String, HttpApplicationDeliveryEffect>,
    effect: HttpApplicationDeliveryEffect,
    idempotency_key: &str,
) -> Result<Option<HttpApplicationDeliveryEffect>, ServerHttpError> {
    if let Some(existing) = effects.get(&effect.message_id) {
        if existing == &effect {
            return Ok(None);
        }
        return Err(ServerHttpError::IdempotencyConflict {
            idempotency_key: idempotency_key.to_owned(),
        });
    }
    Ok(Some(effect))
}

fn push_tokens_for_wake(
    record: &PushWakeOutboxRecord,
    tokens: &BTreeMap<String, PushTokenRecord>,
    rooms: &BTreeMap<String, HttpRoomMembershipProjection>,
    revoked: &BTreeSet<String>,
) -> Vec<PushTokenRecord> {
    let Some(projection) = rooms.get(&record.room_id) else {
        return Vec::new();
    };
    let mut recipients: Vec<PushTokenRecord> = projection
        .membership
        .values()
        .filter(|membership| membership.device != record.sender)
        .filter(|membership| projection.device_active_at_head(&membership.device))
        .filter_map(|membership| {
            let key = DeviceMembership::key(&membership.device);
            if revoked.contains(&key) {
                return None;
            }
            tokens.get(&key).cloned()
        })
        .collect();
    recipients.sort_by(|left, right| {
        left.device
            .account_id
            .cmp(&right.device.account_id)
            .then_with(|| left.device.device_id.cmp(&right.device.device_id))
    });
    recipients
}

fn apply_account_room_membership_delta(
    directory: &mut BTreeMap<String, BTreeMap<String, Value>>,
    room_id: &str,
    mls_group_id: &str,
    current_epoch: u64,
    membership_delta: &MembershipDeltaV1,
    accepted_seq: HttpSequence,
) -> Result<AccountRoomDirectoryMutation, ServerHttpError> {
    let mut account_ids = BTreeSet::new();
    for (account_id, rooms) in directory.iter() {
        if rooms.contains_key(room_id) {
            account_ids.insert(account_id.clone());
        }
    }
    for add in &membership_delta.adds {
        account_ids.insert(add.device.account_id.clone());
    }
    for remove in &membership_delta.removes {
        account_ids.insert(remove.device.account_id.clone());
    }

    let mut mutation = AccountRoomDirectoryMutation::default();
    for account_id in account_ids {
        let empty_record = || AccountRoomRecord {
            room_id: room_id.to_owned(),
            mls_group_id: mls_group_id.to_owned(),
            current_epoch,
            last_seq: accepted_seq,
            status: RoomStatus::Open,
            devices: Vec::new(),
        };
        let existing_record = directory
            .get(&account_id)
            .and_then(|rooms| rooms.get(room_id))
            .cloned();
        let mut record = match existing_record {
            Some(value) => match account_scoped_account_room_record(&account_id, room_id, &value) {
                Ok(Some(record)) => record,
                Ok(None) => empty_record(),
                Err(_) => continue,
            },
            None => empty_record(),
        };

        if record.room_id != room_id {
            continue;
        }
        record.mls_group_id = mls_group_id.to_owned();
        record.current_epoch = current_epoch;
        record.last_seq = accepted_seq;
        for remove in membership_delta
            .removes
            .iter()
            .filter(|remove| remove.device.account_id == account_id)
        {
            record
                .devices
                .retain(|device| device.device != remove.device);
        }
        for add in membership_delta
            .adds
            .iter()
            .filter(|add| add.device.account_id == account_id)
        {
            if !record
                .devices
                .iter()
                .any(|device| device.device == add.device)
            {
                record.devices.push(AccountRoomDevice {
                    device: add.device.clone(),
                    active: false,
                });
            }
        }
        record
            .devices
            .sort_by(|left, right| left.device.device_id.cmp(&right.device.device_id));

        if record.devices.is_empty() {
            if let Some(rooms) = directory.get_mut(&account_id) {
                rooms.remove(room_id);
                if rooms.is_empty() {
                    directory.remove(&account_id);
                }
            }
            mutation.deletes.push((account_id, room_id.to_owned()));
            continue;
        }

        let value = serde_json::to_value(&record)
            .map_err(|error| ServerHttpError::ProjectionJson(error.to_string()))?;
        directory
            .entry(account_id.clone())
            .or_default()
            .insert(room_id.to_owned(), value.clone());
        mutation.upserts.push(AccountRoomDirectoryRecord {
            account_id,
            room_id: room_id.to_owned(),
            record: value,
        });
    }
    Ok(mutation)
}

fn validate_membership_adds_for_projection(
    projection: &HttpRoomMembershipProjection,
    adds: &[MembershipAddV1],
) -> Result<(), ServerHttpError> {
    let mut added_devices_by_account = BTreeMap::<String, usize>::new();
    for add in adds {
        let current_devices =
            projection.current_or_pending_device_count_for_account(&add.device.account_id);
        let added_devices = added_devices_by_account
            .entry(add.device.account_id.clone())
            .or_insert(0);
        *added_devices += 1;
        let proposed = current_devices + *added_devices;
        if proposed > MAX_ACCOUNT_DEVICES_PER_ROOM as usize {
            return Err(ServerHttpError::InvalidCommitRequest {
                reason: format!(
                    "room.devices_per_account has {proposed} items, max {MAX_ACCOUNT_DEVICES_PER_ROOM}"
                ),
            });
        }
        if projection.device_current_or_pending_at_head(&add.device) {
            return Err(ServerHttpError::InvalidCommitRequest {
                reason: format!(
                    "device {:?} is already current or pending in room",
                    add.device
                ),
            });
        }
    }
    Ok(())
}

fn apply_room_membership_delta(
    rooms: &mut BTreeMap<String, HttpRoomMembershipProjection>,
    room_id: &str,
    mls_group_id: &str,
    sender: &DeviceRef,
    expected_epoch: u64,
    membership_delta: &MembershipDeltaV1,
    accepted_seq: HttpSequence,
) -> Result<HttpRoomMembershipProjection, ServerHttpError> {
    let projection = rooms.entry(room_id.to_owned()).or_insert_with(|| {
        initial_room_membership_projection(
            room_id,
            mls_group_id,
            sender,
            expected_epoch,
            0,
            expected_epoch == 0,
            RoomProtocol::default(),
        )
    });
    if projection.room_id != room_id || projection.mls_group_id != mls_group_id {
        return Err(ServerHttpError::RoomMembershipConflict {
            room_id: room_id.to_owned(),
            reason: "membership delta targets a different room or MLS group".to_owned(),
        });
    }
    if projection.current_epoch != expected_epoch {
        return Err(ServerHttpError::RoomMembershipConflict {
            room_id: room_id.to_owned(),
            reason: format!(
                "membership delta expected epoch {expected_epoch}, projection is at {}",
                projection.current_epoch
            ),
        });
    }

    validate_membership_adds_for_projection(projection, &membership_delta.adds)?;

    for remove in &membership_delta.removes {
        if let Some(membership) = projection
            .membership
            .get_mut(&DeviceMembership::key(&remove.device))
            && let Some(interval) = membership
                .intervals
                .iter_mut()
                .rev()
                .find(|interval| interval.active && interval.end_seq.is_none())
        {
            interval.end_seq = Some(accepted_seq);
        }
        // The MLS removal commit for a departed account completes the leave.
        projection.departed.remove(&remove.device.account_id);
    }
    for add in &membership_delta.adds {
        projection
            .membership
            .entry(DeviceMembership::key(&add.device))
            .or_insert_with(|| DeviceMembership {
                device: add.device.clone(),
                intervals: Vec::new(),
            })
            .intervals
            .push(MembershipInterval {
                start_seq: accepted_seq,
                end_seq: None,
                active: false,
            });
    }
    projection.current_epoch = membership_delta.post_commit_epoch;
    projection.last_seq = accepted_seq;
    Ok(projection.clone())
}

fn replay_operation(
    service: &mut HttpDeliveryService,
    operation: PersistedOperation,
) -> Result<(), DurableStoreError> {
    match operation {
        PersistedOperation::PublishMessage {
            target, message, ..
        } => {
            service.publish(target, message)?;
        }
        // KeyPackage lease/reclaim/consume state is rebuilt in the finite wrapper
        // inventory; Finite Chat's core store has no claimed lease state.
        PersistedOperation::PublishKeyPackage { .. } => {}
        PersistedOperation::RevokeDevice { .. } => {}
        PersistedOperation::ClaimKeyPackage { .. }
        | PersistedOperation::ClaimKeyPackages { .. }
        | PersistedOperation::ExpireKeyPackageLease { .. } => {}
    }
    Ok(())
}

fn apply_operations_to_revoked_devices(
    revoked: &mut BTreeSet<String>,
    operations: &[PersistedOperation],
) {
    for operation in operations {
        if let PersistedOperation::RevokeDevice { device } = operation {
            revoked.insert(DeviceMembership::key(device));
        }
    }
}

fn apply_operations_to_key_package_inventory(
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    operations: &[PersistedOperation],
) {
    for operation in operations {
        match operation {
            PersistedOperation::PublishKeyPackage { publication } => {
                if let Some(metadata) = finite_key_package_metadata(publication) {
                    retire_older_finite_key_packages(
                        inventory,
                        &metadata.owner,
                        &publication.key_package_id,
                    );
                }
                let record = inventory
                    .entry(publication.key_package_id.clone())
                    .or_insert_with(|| KeyPackageInventoryRecord {
                        key_package_id: publication.key_package_id.clone(),
                        owner: publication.owner.clone(),
                        key_package: publication.key_package.clone(),
                        state: KeyPackageInventoryState::Available,
                        finite_metadata: finite_key_package_metadata(publication),
                    });
                if record.key_package.bytes().is_empty() {
                    record.key_package = publication.key_package.clone();
                }
                if record.finite_metadata.is_none() {
                    record.finite_metadata = finite_key_package_metadata(publication);
                }
            }
            PersistedOperation::ClaimKeyPackage { owner } => {
                mark_next_key_package_claimed(inventory, owner);
            }
            PersistedOperation::ClaimKeyPackages { owners } => {
                for owner in owners {
                    mark_next_key_package_claimed(inventory, owner);
                }
            }
            PersistedOperation::ExpireKeyPackageLease { key_package_id } => {
                if let Some(record) = inventory.get_mut(key_package_id)
                    && record.state == KeyPackageInventoryState::Claimed
                {
                    record.state = KeyPackageInventoryState::Available;
                }
            }
            PersistedOperation::PublishMessage { message, .. } => {
                consume_key_packages_from_persisted_message(inventory, message);
            }
            PersistedOperation::RevokeDevice { .. } => {}
        }
    }
}

fn key_package_inventory_cache_matches(
    cached: &HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    rebuilt: &HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
) -> bool {
    cached.len() == rebuilt.len()
        && rebuilt.iter().all(|(key_package_id, rebuilt_record)| {
            cached.get(key_package_id).is_some_and(|cached_record| {
                cached_record.owner == rebuilt_record.owner
                    && cached_record.state == rebuilt_record.state
            })
        })
}

fn mark_next_key_package_claimed(
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    owner: &MemberId,
) {
    let selected = inventory
        .iter()
        .filter(|(_, record)| {
            record.owner == *owner && record.state == KeyPackageInventoryState::Available
        })
        .map(|(key_package_id, _)| key_package_id.clone())
        .min_by(|left, right| left.as_slice().cmp(right.as_slice()));
    if let Some(key_package_id) = selected {
        inventory
            .get_mut(&key_package_id)
            .expect("selected KeyPackage must exist before claim")
            .state = KeyPackageInventoryState::Claimed;
    }
}

fn consume_claimed_key_packages_for_commit(
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    request: &SubmitCommitRequest,
) -> Result<Vec<KeyPackageInventoryRecord>, ServerHttpError> {
    let mut changed = Vec::new();
    for add in &request.membership_delta.adds {
        if let Some(record) = validate_claimed_key_package_for_add(inventory, add)? {
            record.state = KeyPackageInventoryState::Consumed;
            changed.push(record.clone());
            continue;
        }
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: format!(
                "KeyPackage {} must be claimed before a typed commit can add {:?}",
                add.key_package_id, add.device
            ),
        });
    }
    Ok(changed)
}

fn validate_claimed_key_package_for_add<'a>(
    inventory: &'a mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    add: &MembershipAddV1,
) -> Result<Option<&'a mut KeyPackageInventoryRecord>, ServerHttpError> {
    let key_package_id = HttpKeyPackageId::new(add.key_package_id.as_bytes().to_vec());
    let Some(record) = inventory.get_mut(&key_package_id) else {
        return Ok(None);
    };
    match record.state {
        KeyPackageInventoryState::Claimed => {}
        KeyPackageInventoryState::Available => {
            return Err(ServerHttpError::InvalidCommitRequest {
                reason: format!(
                    "KeyPackage {} must be claimed before a typed commit can add {:?}",
                    add.key_package_id, add.device
                ),
            });
        }
        KeyPackageInventoryState::Consumed => {
            return Err(ServerHttpError::InvalidCommitRequest {
                reason: format!("KeyPackage {} is already consumed", add.key_package_id),
            });
        }
    }

    let expected_owner = member_id_for_device(&add.device)?;
    if record.owner != expected_owner {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: format!(
                "KeyPackage {} owner does not match added device",
                add.key_package_id
            ),
        });
    }
    let Some(metadata) = &record.finite_metadata else {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: format!(
                "KeyPackage {} does not contain Finite upload metadata",
                add.key_package_id
            ),
        });
    };
    if metadata.owner != add.device {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: format!(
                "KeyPackage {} metadata owner does not match added device",
                add.key_package_id
            ),
        });
    }
    if metadata.key_package_ref != add.key_package_ref
        || metadata.key_package_hash != add.key_package_hash
    {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: format!(
                "KeyPackage {} metadata does not match membership add",
                add.key_package_id
            ),
        });
    }
    Ok(Some(record))
}

fn consume_key_packages_from_persisted_message(
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    message: &TransportMessage,
) {
    let Ok(projection) =
        serde_json::from_slice::<FiniteAccountRoomCommitProjection>(&message.payload)
    else {
        return;
    };
    for add in &projection.membership_delta.adds {
        let key_package_id = HttpKeyPackageId::new(add.key_package_id.as_bytes().to_vec());
        let Ok(owner) = member_id_for_device(&add.device) else {
            continue;
        };
        let record =
            inventory
                .entry(key_package_id.clone())
                .or_insert_with(|| KeyPackageInventoryRecord {
                    key_package_id,
                    owner: owner.clone(),
                    key_package: KeyPackage::new(Vec::new()),
                    state: KeyPackageInventoryState::Claimed,
                    finite_metadata: Some(FiniteKeyPackageMetadata {
                        owner: add.device.clone(),
                        key_package_ref: add.key_package_ref.clone(),
                        key_package_hash: add.key_package_hash.clone(),
                    }),
                });
        if record.owner != owner {
            continue;
        }
        if record.finite_metadata.is_none() {
            record.finite_metadata = Some(FiniteKeyPackageMetadata {
                owner: add.device.clone(),
                key_package_ref: add.key_package_ref.clone(),
                key_package_hash: add.key_package_hash.clone(),
            });
        }
        record.state = KeyPackageInventoryState::Consumed;
    }
}

fn finite_key_package_metadata(
    publication: &HttpKeyPackagePublication,
) -> Option<FiniteKeyPackageMetadata> {
    let request =
        serde_json::from_slice::<UploadKeyPackageRequest>(publication.key_package.bytes()).ok()?;
    if publication.key_package_id.as_slice() != request.key_package_id.as_bytes() {
        return None;
    }
    if member_id_for_device(&request.owner).ok()? != publication.owner {
        return None;
    }
    Some(FiniteKeyPackageMetadata {
        owner: request.owner,
        key_package_ref: request.key_package_ref,
        key_package_hash: request.key_package_hash,
    })
}

fn validate_submit_commit_request(request: &SubmitCommitRequest) -> Result<(), ServerHttpError> {
    request
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidCommitRequest {
            reason: error.to_string(),
        })?;
    let message_id =
        request
            .envelope
            .message_id()
            .map_err(|error| ServerHttpError::InvalidCommitRequest {
                reason: error.to_string(),
            })?;
    if request.envelope.kind != LogEntryKind::Commit {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: "commit request envelope must be a commit".to_owned(),
        });
    }
    if request.envelope.room_id != request.room_id {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: format!(
                "commit envelope room_id {} does not match request room_id {}",
                request.envelope.room_id, request.room_id
            ),
        });
    }
    if request.envelope.epoch != request.expected_epoch {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: format!(
                "commit envelope epoch {} does not match expected epoch {}",
                request.envelope.epoch, request.expected_epoch
            ),
        });
    }
    if request.envelope.sender != request.sender {
        return Err(ServerHttpError::InvalidCommitRequest {
            reason: "commit envelope sender does not match request sender".to_owned(),
        });
    }
    request
        .membership_delta
        .validate_structure(request.expected_epoch, &message_id)
        .map_err(|error| ServerHttpError::InvalidCommitRequest {
            reason: error.to_string(),
        })?;
    staged_welcomes_by_id(&request.membership_delta, &request.staged_welcomes).map_err(
        |error| ServerHttpError::InvalidCommitRequest {
            reason: error.to_string(),
        },
    )?;
    Ok(())
}

fn validate_append_event_request(request: &AppendEventRequest) -> Result<(), ServerHttpError> {
    request
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidEventRequest {
            reason: error.to_string(),
        })?;
    if request.envelope.kind == LogEntryKind::Commit {
        return Err(ServerHttpError::InvalidEventRequest {
            reason: "commit events must be submitted through /commits".to_owned(),
        });
    }
    if request.envelope.room_id != request.room_id {
        return Err(ServerHttpError::InvalidEventRequest {
            reason: format!(
                "event envelope room_id {} does not match request room_id {}",
                request.envelope.room_id, request.room_id
            ),
        });
    }
    if request.envelope.sender != request.sender {
        return Err(ServerHttpError::InvalidEventRequest {
            reason: "event envelope sender does not match request sender".to_owned(),
        });
    }
    request
        .envelope
        .message_id()
        .map_err(|error| ServerHttpError::InvalidEventRequest {
            reason: error.to_string(),
        })?;
    Ok(())
}

fn validate_append_ephemeral_activity_request(
    request: &AppendEphemeralActivityRequest,
) -> Result<(), ServerHttpError> {
    request
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidActivityRequest {
            reason: error.to_string(),
        })?;
    validate_activity_expiry(request.received_at_ms, request.expires_at_ms).map_err(|error| {
        ServerHttpError::InvalidActivityRequest {
            reason: error.to_string(),
        }
    })
}

fn validate_get_ephemeral_activities_request(
    request: &GetEphemeralActivitiesRequest,
) -> Result<(), ServerHttpError> {
    validate_string_bytes("activity.room_id", &request.room_id, MAX_OBJECT_ID_BYTES).map_err(
        |error| ServerHttpError::InvalidActivityRequest {
            reason: error.to_string(),
        },
    )?;
    if let Some(conversation_id) = &request.conversation_id {
        validate_bytes_non_empty("activity.conversation_id", conversation_id.len()).map_err(
            |error| ServerHttpError::InvalidActivityRequest {
                reason: error.to_string(),
            },
        )?;
        validate_string_bytes(
            "activity.conversation_id",
            conversation_id,
            MAX_OBJECT_ID_BYTES,
        )
        .map_err(|error| ServerHttpError::InvalidActivityRequest {
            reason: error.to_string(),
        })?;
    }
    request
        .requester
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidActivityRequest {
            reason: error.to_string(),
        })
}

fn validate_device_liveness_request(
    request: &ObserveDeviceLivenessRequest,
) -> Result<(), ServerHttpError> {
    request.device.validate_limits().map_err(|error| {
        ServerHttpError::InvalidDeviceLivenessRequest {
            reason: error.to_string(),
        }
    })?;
    if request.expires_at_ms <= request.observed_at_ms {
        return Err(ServerHttpError::InvalidDeviceLivenessRequest {
            reason:
                "device_liveness.expires_at_ms must be greater than device_liveness.observed_at_ms"
                    .to_owned(),
        });
    }
    let window = request.expires_at_ms - request.observed_at_ms;
    if window > MAX_DEVICE_LIVENESS_EXPIRY_MILLIS {
        return Err(ServerHttpError::InvalidDeviceLivenessRequest {
            reason: format!(
                "device_liveness.expiry_window_millis has {window} ms, max {MAX_DEVICE_LIVENESS_EXPIRY_MILLIS}"
            ),
        });
    }
    Ok(())
}

fn normalize_nostr_profile_record(
    mut incoming: NostrProfileRecord,
    existing: Option<&NostrProfileRecord>,
) -> Result<NostrProfileRecord, ServerHttpError> {
    if incoming.bot.is_none() {
        incoming.bot = existing.and_then(|record| record.bot);
    }
    if incoming.finite_role.is_none() {
        incoming.finite_role = existing.and_then(|record| record.finite_role.clone());
    }
    incoming.metadata_json = Some(patched_nostr_profile_metadata_json(&incoming, existing)?);
    Ok(incoming)
}

fn patched_nostr_profile_metadata_json(
    incoming: &NostrProfileRecord,
    existing: Option<&NostrProfileRecord>,
) -> Result<String, ServerHttpError> {
    let mut object = existing
        .and_then(|record| record.metadata_json.as_deref())
        .or(incoming.metadata_json.as_deref())
        .map(nostr_profile_metadata_object)
        .transpose()?
        .unwrap_or_default();

    patch_json_string_field(&mut object, "name", incoming.name.as_deref());
    patch_json_string_field(
        &mut object,
        "display_name",
        incoming.display_name.as_deref(),
    );
    object.remove("displayName");
    patch_json_string_field(&mut object, "about", incoming.about.as_deref());
    patch_json_string_field(&mut object, "picture", incoming.picture.as_deref());
    object.remove("picture_url");
    if let Some(bot) = incoming.bot {
        object.insert("bot".to_owned(), serde_json::Value::Bool(bot));
    }
    patch_json_string_field(&mut object, "finite_role", incoming.finite_role.as_deref());
    object.remove("finiteRole");

    serde_json::to_string(&serde_json::Value::Object(object)).map_err(|error| {
        ServerHttpError::InvalidNostrProfileRequest {
            reason: format!("profile.metadata_json could not be encoded: {error}"),
        }
    })
}

fn nostr_profile_metadata_object(
    metadata_json: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, ServerHttpError> {
    let value: serde_json::Value = serde_json::from_str(metadata_json).map_err(|error| {
        ServerHttpError::InvalidNostrProfileRequest {
            reason: format!("profile.metadata_json must be valid JSON: {error}"),
        }
    })?;
    match value {
        serde_json::Value::Object(object) => Ok(object),
        _ => Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: "profile.metadata_json must be a JSON object".to_owned(),
        }),
    }
}

fn patch_json_string_field(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        object.insert(key.to_owned(), serde_json::Value::String(value.to_owned()));
    } else {
        object.remove(key);
    }
}

fn validate_nostr_profile_record(record: &NostrProfileRecord) -> Result<(), ServerHttpError> {
    validate_nostr_account_id(&record.account_id)?;
    validate_optional_profile_text(
        "profile.name",
        record.name.as_deref(),
        MAX_NOSTR_PROFILE_NAME_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.display_name",
        record.display_name.as_deref(),
        MAX_NOSTR_PROFILE_NAME_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.about",
        record.about.as_deref(),
        MAX_NOSTR_PROFILE_ABOUT_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.picture",
        record.picture.as_deref(),
        MAX_NOSTR_PROFILE_PICTURE_BYTES,
    )?;
    validate_optional_profile_text(
        "profile.finite_role",
        record.finite_role.as_deref(),
        MAX_NOSTR_PROFILE_NAME_BYTES,
    )?;
    if let Some(picture) = &record.picture
        && !picture.starts_with("http://")
        && !picture.starts_with("https://")
    {
        return Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: "profile.picture must be http(s)".to_owned(),
        });
    }
    if record.expires_at_ms <= record.fetched_at_ms {
        return Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: "profile.expires_at_ms must be greater than profile.fetched_at_ms".to_owned(),
        });
    }
    validate_nostr_profile_metadata_json(record.metadata_json.as_deref())?;
    Ok(())
}

fn validate_nostr_profile_metadata_json(
    metadata_json: Option<&str>,
) -> Result<(), ServerHttpError> {
    let Some(metadata_json) = metadata_json else {
        return Ok(());
    };
    validate_bytes_len(
        "profile.metadata_json",
        metadata_json.len(),
        MAX_NOSTR_PROFILE_METADATA_JSON_BYTES as u32,
    )
    .map_err(|error| ServerHttpError::InvalidNostrProfileRequest {
        reason: error.to_string(),
    })?;
    let value: serde_json::Value = serde_json::from_str(metadata_json).map_err(|error| {
        ServerHttpError::InvalidNostrProfileRequest {
            reason: format!("profile.metadata_json must be valid JSON: {error}"),
        }
    })?;
    if !value.is_object() {
        return Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: "profile.metadata_json must be a JSON object".to_owned(),
        });
    }
    Ok(())
}

fn validate_nostr_profile_batch(account_ids: &[String]) -> Result<(), ServerHttpError> {
    if account_ids.is_empty() || account_ids.len() > MAX_NOSTR_PROFILE_BATCH {
        return Err(ServerHttpError::InvalidNostrProfileBatch {
            actual: account_ids.len(),
            max: MAX_NOSTR_PROFILE_BATCH,
        });
    }
    let mut seen = BTreeSet::new();
    for account_id in account_ids {
        validate_nostr_account_id(account_id)?;
        if !seen.insert(account_id) {
            return Err(ServerHttpError::InvalidNostrProfileRequest {
                reason: format!("duplicate profile account_id {account_id}"),
            });
        }
    }
    Ok(())
}

fn validate_key_package_availability_batch(account_ids: &[String]) -> Result<(), ServerHttpError> {
    if account_ids.is_empty() || account_ids.len() > MAX_KEY_PACKAGE_AVAILABILITY_BATCH {
        return Err(ServerHttpError::InvalidKeyPackageAvailabilityBatch {
            actual: account_ids.len(),
            max: MAX_KEY_PACKAGE_AVAILABILITY_BATCH,
        });
    }
    for account_id in account_ids {
        validate_key_package_availability_account_id(account_id)?;
    }
    Ok(())
}

fn validate_nostr_account_id(account_id: &str) -> Result<(), ServerHttpError> {
    if account_id.len() != 64
        || !account_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: "profile.account_id must be 64 lowercase hex characters".to_owned(),
        });
    }
    Ok(())
}

fn validate_key_package_availability_account_id(account_id: &str) -> Result<(), ServerHttpError> {
    if account_id.len() != 64
        || !account_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ServerHttpError::InvalidKeyPackageAvailabilityRequest {
            reason: "key_package_availability.account_id must be 64 lowercase hex characters"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_optional_profile_text(
    field: &'static str,
    value: Option<&str>,
    max_bytes: usize,
) -> Result<(), ServerHttpError> {
    if let Some(value) = value
        && value.len() > max_bytes
    {
        return Err(ServerHttpError::InvalidNostrProfileRequest {
            reason: format!("{field} must be at most {max_bytes} bytes"),
        });
    }
    Ok(())
}

fn commit_publish_request(
    request: &SubmitCommitRequest,
    message_id: &str,
) -> Result<PublishMessageRequest, ServerHttpError> {
    let transport_group_id = transport_group_id_for_room(&request.room_id);
    let placeholder_entry = RoomLogEntry {
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
    Ok(PublishMessageRequest {
        target: HttpPublishTarget::Group {
            group_id: group_id_for_room(&request.room_id),
            transport_group_id: transport_group_id.clone(),
            commit_admission: Some(HttpCommitAdmission {
                source_epoch: EpochId(request.expected_epoch),
            }),
        },
        message: TransportMessage {
            id: MessageId::new(message_id.as_bytes().to_vec()),
            payload: serde_json::to_vec(&FiniteAccountRoomCommitProjection {
                entry: placeholder_entry,
                membership_delta: request.membership_delta.clone(),
            })
            .map_err(|error| ServerHttpError::ProjectionJson(error.to_string()))?,
            timestamp: Timestamp(0),
            causal_deps: Vec::new(),
            source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
            envelope: TransportEnvelope::GroupMessage { transport_group_id },
        },
        idempotency_key: Some(format!(
            "commit:{}:{}",
            request.room_id, request.idempotency_key
        )),
    })
}

fn event_publish_request(
    request: &AppendEventRequest,
    message_id: &str,
) -> Result<PublishMessageRequest, ServerHttpError> {
    let transport_group_id = transport_group_id_for_room(&request.room_id);
    let placeholder_entry = RoomLogEntry {
        room_id: request.room_id.clone(),
        seq: 0,
        message_id: message_id.to_owned(),
        sender: request.sender.clone(),
        kind: request.envelope.kind,
        epoch: request.envelope.epoch,
        envelope: request.envelope.clone(),
        idempotency_key: request.idempotency_key.clone(),
        timestamp_unix_seconds: request.timestamp_unix_seconds,
    };
    Ok(PublishMessageRequest {
        target: HttpPublishTarget::Group {
            group_id: group_id_for_room(&request.room_id),
            transport_group_id: transport_group_id.clone(),
            commit_admission: None,
        },
        message: TransportMessage {
            id: MessageId::new(message_id.as_bytes().to_vec()),
            payload: serde_json::to_vec(&placeholder_entry)
                .map_err(|error| ServerHttpError::ProjectionJson(error.to_string()))?,
            timestamp: Timestamp(request.timestamp_unix_seconds),
            causal_deps: Vec::new(),
            source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
            envelope: TransportEnvelope::GroupMessage { transport_group_id },
        },
        idempotency_key: Some(format!(
            "event:{}:{}",
            request.room_id, request.idempotency_key
        )),
    })
}

fn room_log_entry_from_payload(payload: &[u8]) -> Option<RoomLogEntry> {
    if let Ok(projection) = serde_json::from_slice::<FiniteAccountRoomCommitProjection>(payload) {
        return Some(projection.entry);
    }
    serde_json::from_slice(payload).ok()
}

fn released_welcome_records_for_commit(
    request: &SubmitCommitRequest,
    commit_seq: u64,
) -> Result<Vec<WelcomeRecord>, ServerHttpError> {
    let staged = staged_welcomes_by_id(&request.membership_delta, &request.staged_welcomes)
        .map_err(|error| ServerHttpError::InvalidCommitRequest {
            reason: error.to_string(),
        })?;
    request
        .membership_delta
        .adds
        .iter()
        .map(|add| {
            let staged = staged
                .get(&add.welcome_id)
                .expect("validated staged welcome must exist");
            Ok(WelcomeRecord {
                welcome_id: add.welcome_id.clone(),
                room_id: request.room_id.clone(),
                commit_seq,
                recipient: add.device.clone(),
                sender: request.sender.clone(),
                key_package_id: add.key_package_id.clone(),
                join_epoch: request.membership_delta.post_commit_epoch,
                state: WelcomeState::Released,
                lease_token: Some(lease_token_for(&add.welcome_id, &add.device)),
                welcome_payload: staged.welcome_payload.clone(),
                ratchet_tree_payload: staged.ratchet_tree_payload.clone(),
            })
        })
        .collect()
}

fn welcome_publish_request(
    welcome: &WelcomeRecord,
) -> Result<PublishMessageRequest, ServerHttpError> {
    let recipient = member_id_for_device(&welcome.recipient)?;
    Ok(PublishMessageRequest {
        target: HttpPublishTarget::Inbox {
            recipient: recipient.clone(),
        },
        message: TransportMessage {
            id: MessageId::new(welcome.welcome_id.as_bytes().to_vec()),
            payload: serde_json::to_vec(welcome)
                .map_err(|error| ServerHttpError::ProjectionJson(error.to_string()))?,
            timestamp: Timestamp(0),
            causal_deps: Vec::new(),
            source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
            envelope: TransportEnvelope::Welcome { recipient },
        },
        idempotency_key: Some(format!("welcome:{}", welcome.welcome_id)),
    })
}

fn member_id_for_device(device: &DeviceRef) -> Result<MemberId, ServerHttpError> {
    Ok(MemberId::new(delivery_member_id_for_device(device)))
}

fn ensure_device_not_revoked_in(
    revoked_devices: &BTreeSet<String>,
    device: &DeviceRef,
) -> Result<(), ServerHttpError> {
    if revoked_devices.contains(&DeviceMembership::key(device)) {
        Err(ServerHttpError::DeviceRevoked {
            device: device.clone(),
        })
    } else {
        Ok(())
    }
}

fn ensure_welcome_message_recipient_not_revoked(
    revoked_devices: &BTreeSet<String>,
    message: &TransportMessage,
) -> Result<(), ServerHttpError> {
    let Ok(welcome) = serde_json::from_slice::<WelcomeRecord>(&message.payload) else {
        return Ok(());
    };
    ensure_device_not_revoked_in(revoked_devices, &welcome.recipient)
}

fn group_id_for_room(room_id: &str) -> GroupId {
    GroupId::new(room_id.as_bytes().to_vec())
}

fn room_id_for_group_id(group_id: &GroupId) -> Result<String, ServerHttpError> {
    String::from_utf8(group_id.as_slice().to_vec()).map_err(|error| {
        ServerHttpError::InvalidGroupSyncRequest {
            reason: format!("group_id must be a UTF-8 Finite room_id: {error}"),
        }
    })
}

fn transport_group_id_for_room(room_id: &str) -> Vec<u8> {
    room_id.as_bytes().to_vec()
}

fn initial_room_membership_projection(
    room_id: &str,
    mls_group_id: &str,
    creator: &DeviceRef,
    current_epoch: u64,
    last_seq: HttpSequence,
    membership_complete: bool,
    protocol: RoomProtocol,
) -> HttpRoomMembershipProjection {
    let mut membership = BTreeMap::new();
    membership.insert(
        DeviceMembership::key(creator),
        DeviceMembership {
            device: creator.clone(),
            intervals: vec![MembershipInterval {
                start_seq: 0,
                end_seq: None,
                active: true,
            }],
        },
    );
    HttpRoomMembershipProjection {
        room_id: room_id.to_owned(),
        mls_group_id: mls_group_id.to_owned(),
        current_epoch,
        last_seq,
        status: RoomStatus::Open,
        membership_complete,
        admins: BTreeSet::from([creator.account_id.clone()]),
        departed: BTreeSet::new(),
        protocol,
        membership,
    }
}

fn default_membership_complete() -> bool {
    true
}

fn validate_link_session_id(link_session_id: &str) -> Result<(), ServerHttpError> {
    validate_string_bytes("link_session_id", link_session_id, MAX_OBJECT_ID_BYTES).map_err(
        |error| ServerHttpError::InvalidLinkSessionRequest {
            reason: error.to_string(),
        },
    )
}

fn validate_link_pairing_public_key(pairing_public_key: &str) -> Result<(), ServerHttpError> {
    validate_string_bytes(
        "pairing_public_key",
        pairing_public_key,
        MAX_OBJECT_ID_BYTES,
    )
    .map_err(|error| ServerHttpError::InvalidLinkSessionRequest {
        reason: error.to_string(),
    })
}

fn validate_link_payload(payload: &[u8]) -> Result<(), ServerHttpError> {
    validate_bytes_len(
        "link_session.encrypted_payload",
        payload.len(),
        MAX_LINK_SESSION_PAYLOAD_BYTES,
    )
    .map_err(|error| ServerHttpError::InvalidLinkSessionRequest {
        reason: error.to_string(),
    })
}

fn validate_link_claim_token(claim_token: &str) -> Result<(), ServerHttpError> {
    validate_string_bytes("link_session.claim_token", claim_token, MAX_OBJECT_ID_BYTES).map_err(
        |error| ServerHttpError::InvalidLinkSessionRequest {
            reason: error.to_string(),
        },
    )
}

const MAX_SYNC_WAIT_MILLIS: u64 = 25_000;
const MAX_SYNC_WAIT_ROOMS: usize = 256;
const DEFAULT_SYNC_STREAM_HEARTBEAT_MILLIS: u64 = 15_000;
const MIN_SYNC_STREAM_HEARTBEAT_MILLIS: u64 = 1_000;
const MAX_SYNC_STREAM_HEARTBEAT_MILLIS: u64 = 60_000;

fn validate_sync_wait_request(request: &SyncWaitRequest) -> Result<(), ServerHttpError> {
    validate_sync_watch_bounds(&request.rooms, "sync_wait")
}

fn validate_sync_stream_request(request: &SyncStreamRequest) -> Result<(), ServerHttpError> {
    validate_sync_watch_bounds(&request.rooms, "sync_stream")?;
    if let Some(inbox) = &request.inbox {
        let recipient_len = inbox.recipient.as_slice().len();
        if recipient_len == 0 || recipient_len > MAX_HTTP_ID_BYTES {
            return Err(ServerHttpError::InvalidSyncRequest {
                reason: format!(
                    "sync_stream inbox recipient must contain 1..={MAX_HTTP_ID_BYTES} bytes"
                ),
            });
        }
    }
    Ok(())
}

fn validate_sync_watch_bounds(
    rooms: &[finitechat_http::SyncWaitRoom],
    route: &str,
) -> Result<(), ServerHttpError> {
    if rooms.len() > MAX_SYNC_WAIT_ROOMS {
        return Err(ServerHttpError::InvalidSyncRequest {
            reason: format!("{route} watches at most {MAX_SYNC_WAIT_ROOMS} rooms"),
        });
    }
    for room in rooms {
        validate_sync_room_id(&room.room_id)?;
    }
    Ok(())
}

fn validate_sync_room_id(room_id: &str) -> Result<(), ServerHttpError> {
    validate_string_bytes("sync.room_id", room_id, MAX_OBJECT_ID_BYTES).map_err(|error| {
        ServerHttpError::InvalidSyncRequest {
            reason: error.to_string(),
        }
    })
}

fn link_session_claim_token(session: &HttpLinkSessionRecord) -> String {
    lease_token_for(
        &session.link_session_id,
        &DeviceRef {
            account_id: "link".to_owned(),
            device_id: session.pairing_public_key.clone(),
        },
    )
}

fn validate_account_room_id(field: &'static str, value: &str) -> Result<(), ServerHttpError> {
    if value.is_empty() || value.len() > MAX_HTTP_ACCOUNT_ROOM_ID_BYTES {
        return Err(ServerHttpError::InvalidAccountRoomRequest {
            reason: format!(
                "{field} must contain between 1 and {MAX_HTTP_ACCOUNT_ROOM_ID_BYTES} bytes"
            ),
        });
    }
    Ok(())
}

fn account_scoped_account_room_record(
    account_id: &str,
    room_id: &str,
    value: &Value,
) -> Result<Option<AccountRoomRecord>, ServerHttpError> {
    let mut record =
        serde_json::from_value::<AccountRoomRecord>(value.clone()).map_err(|error| {
            ServerHttpError::InvalidAccountRoomRequest {
                reason: format!("record must be a Finite account-room record: {error}"),
            }
        })?;
    record
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidAccountRoomRequest {
            reason: error.to_string(),
        })?;
    if record.room_id != room_id {
        return Err(ServerHttpError::InvalidAccountRoomRequest {
            reason: format!(
                "record room_id {} does not match directory room_id {room_id}",
                record.room_id
            ),
        });
    }

    record
        .devices
        .retain(|device| device.device.account_id == account_id);
    record
        .devices
        .sort_by(|left, right| left.device.device_id.cmp(&right.device.device_id));
    if record.devices.is_empty() {
        return Ok(None);
    }
    record
        .validate_limits()
        .map_err(|error| ServerHttpError::InvalidAccountRoomRequest {
            reason: error.to_string(),
        })?;
    Ok(Some(record))
}

fn claim_key_packages_from_inventory(
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    owners: &[MemberId],
    revoked_devices: &BTreeSet<String>,
) -> Vec<HttpKeyPackageClaim> {
    owners
        .iter()
        .map(|owner| {
            let claimed = claim_next_key_package_from_inventory(inventory, owner, revoked_devices);
            HttpKeyPackageClaim {
                owner: owner.clone(),
                claimed,
            }
        })
        .collect()
}

fn record_key_package_publication(
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    publication: &HttpKeyPackagePublication,
) -> Result<Option<KeyPackageInventoryRecord>, ServerHttpError> {
    if let Some(record) = inventory.get_mut(&publication.key_package_id) {
        if record.owner != publication.owner || record.key_package != publication.key_package {
            return Err(HttpServerError::ConflictingKeyPackage {
                key_package_id: publication.key_package_id.clone(),
            }
            .into());
        }
        if record.finite_metadata.is_none() {
            record.finite_metadata = finite_key_package_metadata(publication);
            return Ok(Some(record.clone()));
        }
        return Ok(None);
    }

    if let Some(metadata) = finite_key_package_metadata(publication) {
        retire_older_finite_key_packages(inventory, &metadata.owner, &publication.key_package_id);
    }

    let unconsumed = inventory
        .values()
        .filter(|record| {
            record.owner == publication.owner
                && matches!(
                    record.state,
                    KeyPackageInventoryState::Available | KeyPackageInventoryState::Claimed
                )
        })
        .count();
    if unconsumed >= MAX_KEY_PACKAGES_PER_DEVICE as usize {
        return Err(HttpServerError::KeyPackageInventoryFull {
            owner: publication.owner.clone(),
            max: MAX_KEY_PACKAGES_PER_DEVICE as usize,
        }
        .into());
    }

    let record = KeyPackageInventoryRecord {
        key_package_id: publication.key_package_id.clone(),
        owner: publication.owner.clone(),
        key_package: publication.key_package.clone(),
        state: KeyPackageInventoryState::Available,
        finite_metadata: finite_key_package_metadata(publication),
    };
    inventory.insert(publication.key_package_id.clone(), record.clone());
    Ok(Some(record))
}

fn changed_key_package_inventory_records(
    before: &HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    after: &HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
) -> Vec<KeyPackageInventoryRecord> {
    after
        .values()
        .filter(|record| {
            before
                .get(&record.key_package_id)
                .is_none_or(|previous| previous != *record)
        })
        .cloned()
        .collect()
}

fn claim_next_key_package_from_inventory(
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    owner: &MemberId,
    revoked_devices: &BTreeSet<String>,
) -> Option<HttpClaimedKeyPackage> {
    let selected = inventory
        .iter()
        .filter(|(_, record)| {
            record.owner == *owner
                && record.state == KeyPackageInventoryState::Available
                && !record_finite_owner_is_revoked(record, revoked_devices)
        })
        .map(|(key_package_id, _)| key_package_id.clone())
        .min_by(|left, right| left.as_slice().cmp(right.as_slice()));
    let key_package_id = selected?;
    let record = inventory
        .get_mut(&key_package_id)
        .expect("selected KeyPackage must exist before claim");
    record.state = KeyPackageInventoryState::Claimed;
    Some(HttpClaimedKeyPackage {
        key_package_id,
        owner: record.owner.clone(),
        key_package: record.key_package.clone(),
    })
}

fn claim_next_key_package_for_account_from_inventory(
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    account_id: &str,
    revoked_devices: &BTreeSet<String>,
) -> Option<HttpClaimedKeyPackage> {
    let selected = inventory
        .iter()
        .filter(|(_, record)| {
            if record.state != KeyPackageInventoryState::Available {
                return false;
            }
            let Some(metadata) = &record.finite_metadata else {
                return false;
            };
            metadata.owner.account_id == account_id
                && !revoked_devices.contains(&DeviceMembership::key(&metadata.owner))
        })
        .map(|(key_package_id, _)| key_package_id.clone())
        .max_by(|left, right| {
            key_package_freshness_rank(left.as_slice())
                .cmp(&key_package_freshness_rank(right.as_slice()))
                .then_with(|| left.as_slice().cmp(right.as_slice()))
        });
    let key_package_id = selected?;
    let record = inventory
        .get_mut(&key_package_id)
        .expect("selected KeyPackage must exist before account claim");
    record.state = KeyPackageInventoryState::Claimed;
    Some(HttpClaimedKeyPackage {
        key_package_id,
        owner: record.owner.clone(),
        key_package: record.key_package.clone(),
    })
}

fn key_package_freshness_rank(key_package_id: &[u8]) -> (u8, u64) {
    let Some(rest) = key_package_id.strip_prefix(b"kp_t") else {
        return (0, 0);
    };
    let Some((timestamp, _suffix)) = rest.split_first_chunk::<20>() else {
        return (0, 0);
    };
    let Ok(timestamp) = std::str::from_utf8(timestamp) else {
        return (0, 0);
    };
    if !timestamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return (0, 0);
    }
    timestamp
        .parse::<u64>()
        .map(|value| (1, value))
        .unwrap_or((0, 0))
}

fn retire_older_finite_key_packages(
    inventory: &mut HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    owner: &DeviceRef,
    new_key_package_id: &HttpKeyPackageId,
) {
    let new_rank = key_package_freshness_rank(new_key_package_id.as_slice());
    if new_rank.0 == 0 {
        return;
    }
    for record in inventory.values_mut() {
        if record.key_package_id == *new_key_package_id {
            continue;
        }
        if record.state != KeyPackageInventoryState::Available {
            continue;
        }
        let Some(metadata) = &record.finite_metadata else {
            continue;
        };
        if metadata.owner == *owner
            && key_package_freshness_rank(record.key_package_id.as_slice()) < new_rank
        {
            record.state = KeyPackageInventoryState::Consumed;
        }
    }
}

fn record_finite_owner_is_revoked(
    record: &KeyPackageInventoryRecord,
    revoked_devices: &BTreeSet<String>,
) -> bool {
    record
        .finite_metadata
        .as_ref()
        .is_some_and(|metadata| revoked_devices.contains(&DeviceMembership::key(&metadata.owner)))
}

fn available_finite_owner_revoked_in_inventory(
    inventory: &HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    owner: &MemberId,
    revoked_devices: &BTreeSet<String>,
) -> Option<DeviceRef> {
    inventory
        .values()
        .filter(|record| {
            record.owner == *owner && record.state == KeyPackageInventoryState::Available
        })
        .filter_map(|record| record.finite_metadata.as_ref())
        .find(|metadata| revoked_devices.contains(&DeviceMembership::key(&metadata.owner)))
        .map(|metadata| metadata.owner.clone())
}

fn key_package_claim_inventory_records(
    inventory: &HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>,
    claims: &[HttpKeyPackageClaim],
) -> Vec<KeyPackageInventoryRecord> {
    claims
        .iter()
        .filter_map(|claim| {
            claim
                .claimed
                .as_ref()
                .and_then(|package| inventory.get(&package.key_package_id))
                .cloned()
        })
        .collect()
}

fn validate_key_package_claim_batch(owners: &[MemberId]) -> Result<(), ServerHttpError> {
    if owners.is_empty() || owners.len() > MAX_HTTP_SYNC_PAGE_ENTRIES {
        return Err(ServerHttpError::InvalidKeyPackageClaimBatch {
            actual: owners.len(),
            max: MAX_HTTP_SYNC_PAGE_ENTRIES,
        });
    }

    let mut seen = HashSet::new();
    for owner in owners {
        if !seen.insert(owner) {
            return Err(ServerHttpError::DuplicateKeyPackageClaimOwner {
                owner: owner.clone(),
            });
        }
    }
    Ok(())
}

fn usize_to_u32(field: &'static str, value: usize) -> Result<u32, ServerHttpError> {
    u32::try_from(value)
        .map_err(|_| ServerHttpError::KeyPackageInventoryCountOverflow { field, value })
}

#[derive(Debug, Error)]
pub enum HttpServerConfigurationError {
    #[error("invalid Finite Chat public URL: {reason}")]
    InvalidPublicUrl { reason: String },
}

#[derive(Debug, Error)]
pub enum DurableStoreError {
    #[error("SQLite delivery store error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("delivery store JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("persisted delivery operation failed replay: {0}")]
    Replay(#[from] HttpServerError),
    #[error("persisted blob object is corrupt: {sha256}")]
    BlobObjectCorrupt { sha256: String },
}

#[derive(Debug)]
pub enum ServerHttpError {
    Delivery(HttpServerError),
    IdempotencyConflict {
        idempotency_key: String,
    },
    InvalidIdempotencyKey,
    InvalidKeyPackageClaimBatch {
        actual: usize,
        max: usize,
    },
    InvalidKeyPackageLeaseRequest {
        reason: String,
    },
    InvalidDeviceRequest {
        reason: String,
    },
    DeviceRevoked {
        device: DeviceRef,
    },
    InvalidDeviceLivenessRequest {
        reason: String,
    },
    InvalidNostrProfileRequest {
        reason: String,
    },
    InvalidNostrProfileBatch {
        actual: usize,
        max: usize,
    },
    InvalidKeyPackageAvailabilityRequest {
        reason: String,
    },
    InvalidKeyPackageAvailabilityBatch {
        actual: usize,
        max: usize,
    },
    DeviceNotActive {
        device: DeviceRef,
    },
    DuplicateKeyPackageClaimOwner {
        owner: MemberId,
    },
    InventoryConflict {
        key_package_id: HttpKeyPackageId,
    },
    KeyPackageInventoryCountOverflow {
        field: &'static str,
        value: usize,
    },
    CounterOverflow,
    InvalidCommitRequest {
        reason: String,
    },
    InvalidRawCommitImport {
        room_id: String,
        reason: String,
    },
    InvalidEventRequest {
        reason: String,
    },
    DuplicateMessageId {
        message_id: MessageId,
    },
    InvalidActivityRequest {
        reason: String,
    },
    SenderNotActive {
        sender: DeviceRef,
    },
    CommitAuthorityRequired {
        sender: DeviceRef,
    },
    InvalidAdminChange {
        reason: String,
    },
    UnsupportedProtocolVersion {
        requested: u32,
        min: u32,
        max: u32,
    },
    InvalidRepairReport {
        reason: String,
    },
    ReporterNotInInterval {
        reporter: DeviceRef,
        offending_seq: HttpSequence,
    },
    RoomNotOpen {
        room_id: String,
        status: RoomStatus,
    },
    InvalidFanoutRequest {
        reason: String,
    },
    FanoutLimitExceeded {
        fanout_id: String,
        actual: usize,
        max: usize,
    },
    FanoutConflict {
        fanout_id: String,
        reason: String,
    },
    FanoutNotFound {
        fanout_id: String,
    },
    FanoutRoomNotFound {
        fanout_id: String,
        room_id: GroupId,
    },
    InvalidLinkSessionRequest {
        reason: String,
    },
    LinkSessionAlreadyExists {
        link_session_id: String,
    },
    LinkSessionNotFound {
        link_session_id: String,
    },
    LinkSessionConflict {
        link_session_id: String,
        reason: String,
    },
    LinkSessionClosed {
        link_session_id: String,
    },
    LinkSessionNotReady {
        link_session_id: String,
    },
    BadLinkSessionClaimToken {
        link_session_id: String,
    },
    InvalidSyncRequest {
        reason: String,
    },
    InvalidAccountRoomRequest {
        reason: String,
    },
    AccountRoomBootstrapConflict {
        account_id: String,
        room_id: String,
        reason: String,
    },
    DirectRoomConflict {
        room_id: String,
        reason: String,
    },
    ProjectionJson(String),
    InvalidGroupSyncRequest {
        reason: String,
    },
    InvalidGroupSyncLimit {
        actual: usize,
        max: usize,
    },
    RoomMembershipConflict {
        room_id: String,
        reason: String,
    },
    InvalidAccountRoomListLimit {
        actual: usize,
        max: usize,
    },
    InvalidWelcomeClaimLimit {
        actual: usize,
        max: usize,
    },
    Store(DurableStoreError),
    WelcomeNotFound {
        message_id: MessageId,
    },
    InvalidBlobRequest {
        reason: String,
    },
    BlobNotFound {
        sha256: String,
    },
    BlobConflict {
        sha256: String,
    },
}

impl From<HttpServerError> for ServerHttpError {
    fn from(error: HttpServerError) -> Self {
        Self::Delivery(error)
    }
}

impl From<DurableStoreError> for ServerHttpError {
    fn from(error: DurableStoreError) -> Self {
        Self::Store(error)
    }
}

impl IntoResponse for ServerHttpError {
    fn into_response(self) -> Response {
        let (status, kind, error) = match self {
            Self::Delivery(error) => (
                status_for_error(&error),
                kind_for_error(&error).to_owned(),
                error.to_string(),
            ),
            Self::Store(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "delivery_store".to_owned(),
                error.to_string(),
            ),
            Self::IdempotencyConflict { idempotency_key } => (
                StatusCode::CONFLICT,
                "idempotency_conflict".to_owned(),
                format!("conflicting request for idempotency key '{idempotency_key}'"),
            ),
            Self::InvalidIdempotencyKey => (
                StatusCode::BAD_REQUEST,
                "invalid_idempotency_key".to_owned(),
                "idempotency key must not be empty".to_owned(),
            ),
            Self::InvalidKeyPackageClaimBatch { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_key_package_claim_batch".to_owned(),
                format!(
                    "KeyPackage claim batch must contain between 1 and {max} owners, got {actual}"
                ),
            ),
            Self::InvalidKeyPackageLeaseRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_key_package_lease_request".to_owned(),
                reason,
            ),
            Self::InvalidDeviceRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_device_request".to_owned(),
                reason,
            ),
            Self::DeviceRevoked { device } => (
                StatusCode::FORBIDDEN,
                "device_revoked".to_owned(),
                format!("device {device:?} is revoked"),
            ),
            Self::InvalidDeviceLivenessRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_device_liveness_request".to_owned(),
                reason,
            ),
            Self::InvalidNostrProfileRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_nostr_profile_request".to_owned(),
                reason,
            ),
            Self::InvalidNostrProfileBatch { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_nostr_profile_batch".to_owned(),
                format!(
                    "Nostr profile batch must contain between 1 and {max} accounts, got {actual}"
                ),
            ),
            Self::InvalidKeyPackageAvailabilityRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_key_package_availability_request".to_owned(),
                reason,
            ),
            Self::InvalidKeyPackageAvailabilityBatch { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_key_package_availability_batch".to_owned(),
                format!(
                    "KeyPackage availability batch must contain between 1 and {max} accounts, got {actual}"
                ),
            ),
            Self::DeviceNotActive { device } => (
                StatusCode::FORBIDDEN,
                "device_not_active".to_owned(),
                format!("device {device:?} is not active in any room"),
            ),
            Self::DuplicateKeyPackageClaimOwner { owner } => (
                StatusCode::BAD_REQUEST,
                "duplicate_key_package_claim_owner".to_owned(),
                format!("KeyPackage claim batch contains duplicate owner {owner:?}"),
            ),
            Self::InventoryConflict { key_package_id } => (
                StatusCode::CONFLICT,
                "key_package_inventory_conflict".to_owned(),
                format!("KeyPackage inventory has a conflicting owner for {key_package_id:?}"),
            ),
            Self::KeyPackageInventoryCountOverflow { field, value } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "key_package_inventory_count_overflow".to_owned(),
                format!("KeyPackage inventory field {field} does not fit in u32: {value}"),
            ),
            Self::CounterOverflow => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "counter_overflow".to_owned(),
                "counter value does not fit in u32".to_owned(),
            ),
            Self::InvalidCommitRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_commit_request".to_owned(),
                reason,
            ),
            Self::InvalidRawCommitImport { room_id, reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_raw_commit_import".to_owned(),
                format!("raw commit import for {room_id} is invalid: {reason}"),
            ),
            Self::InvalidEventRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_event_request".to_owned(),
                reason,
            ),
            Self::DuplicateMessageId { message_id } => (
                StatusCode::CONFLICT,
                "duplicate_message_id".to_owned(),
                format!("duplicate typed event message id {message_id}"),
            ),
            Self::InvalidActivityRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_activity_request".to_owned(),
                reason,
            ),
            Self::SenderNotActive { sender } => (
                StatusCode::FORBIDDEN,
                "sender_not_active".to_owned(),
                format!("sender {sender:?} is not active in the room"),
            ),
            Self::CommitAuthorityRequired { sender } => (
                StatusCode::FORBIDDEN,
                "commit_authority_required".to_owned(),
                format!(
                    "sender {sender:?} must be a room admin to change another account's membership"
                ),
            ),
            Self::InvalidAdminChange { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_admin_change".to_owned(),
                reason,
            ),
            Self::UnsupportedProtocolVersion {
                requested,
                min,
                max,
            } => (
                StatusCode::UPGRADE_REQUIRED,
                "unsupported_protocol_version".to_owned(),
                format!(
                    "room protocol version {requested} is outside the supported range {min}..={max}"
                ),
            ),
            Self::InvalidRepairReport { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_repair_report".to_owned(),
                reason,
            ),
            Self::ReporterNotInInterval {
                reporter,
                offending_seq,
            } => (
                StatusCode::FORBIDDEN,
                "reporter_not_in_interval".to_owned(),
                format!("reporter {reporter:?} was not a member for seq {offending_seq}"),
            ),
            Self::RoomNotOpen { room_id, status } => (
                StatusCode::CONFLICT,
                "room_not_open".to_owned(),
                format!("room {room_id} is {status:?}"),
            ),
            Self::InvalidFanoutRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_fanout_request".to_owned(),
                reason,
            ),
            Self::FanoutLimitExceeded {
                fanout_id,
                actual,
                max,
            } => (
                StatusCode::TOO_MANY_REQUESTS,
                "fanout_limit_exceeded".to_owned(),
                format!("fanout {fanout_id} has {actual} rooms, max {max}"),
            ),
            Self::FanoutConflict { fanout_id, reason } => (
                StatusCode::CONFLICT,
                "fanout_conflict".to_owned(),
                format!("fanout {fanout_id} conflict: {reason}"),
            ),
            Self::FanoutNotFound { fanout_id } => (
                StatusCode::NOT_FOUND,
                "fanout_not_found".to_owned(),
                format!("fanout {fanout_id} was not found"),
            ),
            Self::FanoutRoomNotFound { fanout_id, room_id } => (
                StatusCode::NOT_FOUND,
                "fanout_room_not_found".to_owned(),
                format!("fanout {fanout_id} has no room {room_id:?}"),
            ),
            Self::InvalidLinkSessionRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_link_session_request".to_owned(),
                reason,
            ),
            Self::LinkSessionAlreadyExists { link_session_id } => (
                StatusCode::CONFLICT,
                "link_session_already_exists".to_owned(),
                format!("link session {link_session_id} already exists"),
            ),
            Self::LinkSessionNotFound { link_session_id } => (
                StatusCode::NOT_FOUND,
                "link_session_not_found".to_owned(),
                format!("link session {link_session_id} was not found"),
            ),
            Self::LinkSessionConflict {
                link_session_id,
                reason,
            } => (
                StatusCode::CONFLICT,
                "link_session_conflict".to_owned(),
                format!("link session {link_session_id} conflict: {reason}"),
            ),
            Self::LinkSessionClosed { link_session_id } => (
                StatusCode::BAD_REQUEST,
                "link_session_closed".to_owned(),
                format!("link session {link_session_id} is closed"),
            ),
            Self::LinkSessionNotReady { link_session_id } => (
                StatusCode::BAD_REQUEST,
                "link_session_not_ready".to_owned(),
                format!("link session {link_session_id} is not ready"),
            ),
            Self::BadLinkSessionClaimToken { link_session_id } => (
                StatusCode::BAD_REQUEST,
                "bad_link_session_claim_token".to_owned(),
                format!("link session {link_session_id} claim token does not match"),
            ),
            Self::InvalidSyncRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_sync_request".to_owned(),
                reason,
            ),
            Self::InvalidAccountRoomRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_account_room_request".to_owned(),
                reason,
            ),
            Self::AccountRoomBootstrapConflict {
                account_id,
                room_id,
                reason,
            } => (
                StatusCode::CONFLICT,
                "account_room_bootstrap_conflict".to_owned(),
                format!("account-room bootstrap conflict for {account_id}/{room_id}: {reason}"),
            ),
            Self::DirectRoomConflict { room_id, reason } => (
                StatusCode::CONFLICT,
                "direct_room_conflict".to_owned(),
                format!("direct-room conflict for {room_id}: {reason}"),
            ),
            Self::ProjectionJson(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "finite_projection_json".to_owned(),
                error,
            ),
            Self::InvalidGroupSyncRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_group_sync_request".to_owned(),
                reason,
            ),
            Self::InvalidGroupSyncLimit { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_group_sync_limit".to_owned(),
                format!("group sync limit must be between 1 and {max}, got {actual}"),
            ),
            Self::RoomMembershipConflict { room_id, reason } => (
                StatusCode::CONFLICT,
                "room_membership_conflict".to_owned(),
                format!("room-membership projection conflict for {room_id}: {reason}"),
            ),
            Self::InvalidAccountRoomListLimit { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_account_room_list_limit".to_owned(),
                format!("account-room list limit must be between 1 and {max}, got {actual}"),
            ),
            Self::InvalidWelcomeClaimLimit { actual, max } => (
                StatusCode::BAD_REQUEST,
                "invalid_welcome_claim_limit".to_owned(),
                format!("welcome claim limit must be between 1 and {max}, got {actual}"),
            ),
            Self::WelcomeNotFound { message_id } => (
                StatusCode::NOT_FOUND,
                "welcome_not_found".to_owned(),
                format!("welcome {message_id} was not claimed"),
            ),
            Self::InvalidBlobRequest { reason } => (
                StatusCode::BAD_REQUEST,
                "invalid_blob_request".to_owned(),
                reason,
            ),
            Self::BlobNotFound { sha256 } => (
                StatusCode::NOT_FOUND,
                "blob_not_found".to_owned(),
                format!("blob object {sha256} was not found"),
            ),
            Self::BlobConflict { sha256 } => (
                StatusCode::CONFLICT,
                "blob_conflict".to_owned(),
                format!("blob object {sha256} already exists with different bytes"),
            ),
        };
        let body = ErrorResponse { kind, error };
        (status, Json(body)).into_response()
    }
}

fn status_for_error(error: &HttpServerError) -> StatusCode {
    match error {
        HttpServerError::ConflictingMessageId { .. }
        | HttpServerError::StaleEpoch { .. }
        | HttpServerError::ConflictingKeyPackage { .. } => StatusCode::CONFLICT,
        HttpServerError::QueueFull { .. }
        | HttpServerError::GroupLimitExceeded { .. }
        | HttpServerError::InboxLimitExceeded { .. }
        | HttpServerError::KeyPackageInventoryFull { .. } => StatusCode::TOO_MANY_REQUESTS,
        HttpServerError::Empty { .. }
        | HttpServerError::TooLarge { .. }
        | HttpServerError::PublishTargetMismatch
        | HttpServerError::InvalidPageLimit { .. } => StatusCode::BAD_REQUEST,
    }
}

fn kind_for_error(error: &HttpServerError) -> &'static str {
    match error {
        HttpServerError::Empty { .. } => "empty",
        HttpServerError::TooLarge { .. } => "too_large",
        HttpServerError::PublishTargetMismatch => "publish_target_mismatch",
        HttpServerError::ConflictingMessageId { .. } => "conflicting_message_id",
        HttpServerError::StaleEpoch { .. } => "stale_epoch",
        HttpServerError::QueueFull { .. } => "queue_full",
        HttpServerError::GroupLimitExceeded { .. } => "group_limit_exceeded",
        HttpServerError::InboxLimitExceeded { .. } => "inbox_limit_exceeded",
        HttpServerError::InvalidPageLimit { .. } => "invalid_page_limit",
        HttpServerError::ConflictingKeyPackage { .. } => "conflicting_key_package",
        HttpServerError::KeyPackageInventoryFull { .. } => "key_package_inventory_full",
    }
}
