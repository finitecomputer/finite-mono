//! Server state ([`HttpServerState`]), its domain methods, and the
//! server-side record types they operate on.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::http::HeaderMap;
use finitechat_blob::BlobDescriptor;
use finitechat_delivery::{
    HTTP_SERVER_SOURCE, HttpClaimedKeyPackage, HttpCommitAdmission, HttpDeliveryService,
    HttpKeyPackageId, HttpKeyPackagePublication, HttpPublishCheck, HttpPublishReceipt,
    HttpPublishTarget, HttpSequence, HttpServerError, HttpSyncPage, MAX_HTTP_SYNC_PAGE_ENTRIES,
};
use finitechat_http::{
    AckPushWakeRequest, AckPushWakeResponse, AckWelcomeRequest, AckWelcomeResponse,
    ApplicationEffectCountsResponse, ApplicationEffectRequest, BootstrapAccountRoomRequest,
    BootstrapAccountRoomResponse, ClaimKeyPackageForAccountRequest, ClaimKeyPackageRequest,
    ClaimKeyPackagesRequest, ClaimPushWakesRequest, ClaimPushWakesResponse, ClaimWelcomesRequest,
    CreatePairingSessionRequest, DeviceLivenessRecord, ExpireKeyPackageLeaseRequest,
    ExpireKeyPackageLeaseResponse, ExpirePairingSessionRequest, ExpirePairingSessionResponse,
    FailPushWakeRequest, FailPushWakeResponse, FiniteAccountRoomCommitProjection,
    GetDeviceLivenessRequest, GetDeviceLivenessResponse, GetEphemeralActivitiesRequest,
    GetEphemeralActivitiesResponse, GetKeyPackageAvailabilityRequest,
    GetKeyPackageAvailabilityResponse, GetNostrProfilesRequest, GetNostrProfilesResponse,
    GetPairingSessionRequest, GroupSyncRequest, HttpApplicationDeliveryEffect, HttpClaimedWelcome,
    HttpKeyPackageClaim, HttpKeyPackageInventory, HttpPairingEventRecord, HttpPairingSessionRecord,
    HttpPairingSessionState, KeyPackageAvailabilityEntry, KeyPackageInventoryRequest,
    LeaveRoomRequest, LeaveRoomResponse, ListAccountRoomDirectoryRequest,
    ListAccountRoomDirectoryResponse, NostrProfileCacheEntry, NostrProfileRecord,
    ObserveDeviceLivenessRequest, PublishKeyPackageResponse, PublishMessageRequest,
    PublishPairingCompleteRequest, PublishPairingOfferRequest, PublishPairingResponseRequest,
    PushTokenRecord, PushWakeDelivery, PushWakePayload, PutNostrProfileRequest,
    PutNostrProfileResponse, RegisterPushTokenRequest, RegisterPushTokenResponse,
    RemovePushTokenRequest, RemovePushTokenResponse, ReportInvalidCommitRequest,
    ReportInvalidCommitResponse, RevokeDeviceRequest, RevokeDeviceResponse, SaveAccountRoomRequest,
    SaveAccountRoomResponse, SyncHintEvent, SyncWaitRequest, UpdateRoomAdminsRequest,
    UpdateRoomAdminsResponse,
};
use finitechat_proto::{
    AccountRoomDevice, AccountRoomRecord, AppendApplicationEventRequest,
    AppendEphemeralActivityRequest, AppendEventRequest, CommitAccepted, DeviceMembership,
    DeviceRef, EphemeralActivityAccepted, EphemeralActivityRecord, EventAccepted, LogEntryKind,
    MAX_EPHEMERAL_ACTIVITY_CACHE_ENTRIES_PER_ROUTE, MAX_KEY_PACKAGES_PER_DEVICE,
    MAX_OBJECT_ID_BYTES, MIN_SUPPORTED_PROTOCOL_VERSION, MembershipAddV1, MembershipDeltaV1,
    PROTOCOL_VERSION_V1, RoomLogEntry, RoomStatus, SubmitCommitRequest, UploadKeyPackageRequest,
    WelcomeRecord, WelcomeState, lease_token_for, staged_welcomes_by_id, validate_string_bytes,
};
use finitechat_transport::engine::KeyPackage;
use finitechat_transport::transport::{
    Timestamp, TransportEnvelope, TransportMessage, TransportSource,
};
use finitechat_transport::{EpochId, MemberId, MessageId};
use nostr::PublicKey as NostrPublicKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::legacy_store::{
    PersistedOperation, SqliteHttpDeliveryStore, apply_operations_to_key_package_inventory,
    apply_operations_to_revoked_devices, replay_operation,
};
use crate::projections::{
    HttpRoomMembershipProjection, ObservedRoomHead, apply_room_membership_delta,
    ensure_device_not_revoked_in, ensure_welcome_message_recipient_not_revoked, group_id_for_room,
    initial_room_membership_projection, member_id_for_device, room_id_for_group_id,
    transport_group_id_for_room, validate_membership_adds_for_projection,
};
use crate::validate::{
    blob_content_type, blob_url, ensure_pairing_session_open, normalize_blob_upload_content_type,
    normalize_nostr_profile_record, normalize_public_url, pairing_conflict, pairing_corrupt,
    pairing_invalid, pairing_now, pairing_recipient, sha256_hex, usize_to_u32,
    validate_account_room_id, validate_append_ephemeral_activity_request,
    validate_append_event_request, validate_blob_sha256, validate_blob_upload,
    validate_device_liveness_request, validate_get_ephemeral_activities_request,
    validate_key_package_availability_account_id, validate_key_package_availability_batch,
    validate_key_package_claim_batch, validate_nostr_profile_batch, validate_nostr_profile_record,
    validate_pairing_device_id, validate_pairing_event, validate_pairing_public_key,
    validate_pairing_session_id, validate_submit_commit_request,
};
use crate::{
    DurableStoreError, HttpServerConfigurationError, MAX_PUSH_WAKE_ATTEMPTS,
    MAX_PUSH_WAKE_CLAIM_BATCH, MAX_PUSH_WAKE_LEASE_MS, PAIRING_PROTOCOL_VERSION,
    PAIRING_SESSION_TTL_SECONDS, SNAPSHOT_INTERVAL_OPS, ServerHttpError, finite_delivery_limits,
};

#[derive(Clone, Debug, Default)]
pub struct HttpServerState {
    service: Arc<Mutex<HttpDeliveryService>>,
    publish_idempotency: Arc<Mutex<HashMap<String, PublishIdempotencyRecord>>>,
    key_package_claim_idempotency: Arc<Mutex<HashMap<String, KeyPackageClaimIdempotencyRecord>>>,
    key_package_inventory: Arc<Mutex<HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>>>,
    revoked_devices: Arc<Mutex<BTreeSet<String>>>,
    pairing_sessions: Arc<Mutex<BTreeMap<String, HttpPairingSessionRecord>>>,
    account_rooms: Arc<Mutex<BTreeMap<String, BTreeMap<String, Value>>>>,
    room_memberships: Arc<Mutex<BTreeMap<String, HttpRoomMembershipProjection>>>,
    application_effects: Arc<Mutex<BTreeMap<String, HttpApplicationDeliveryEffect>>>,
    ephemeral_activity: Arc<Mutex<BTreeMap<String, Vec<EphemeralActivityRecord>>>>,
    device_liveness: Arc<Mutex<BTreeMap<String, DeviceLivenessRecord>>>,
    nostr_profiles: Arc<Mutex<BTreeMap<String, NostrProfileRecord>>>,
    welcome_claims: Arc<Mutex<HashMap<MessageId, WelcomeClaimRecord>>>,
    push_tokens: Arc<Mutex<BTreeMap<String, PushTokenRecord>>>,
    push_wakes: Arc<Mutex<BTreeMap<String, PushWakeOutboxRecord>>>,
    /// Blob metadata only (tens of bytes per blob). Payload bytes live in
    /// SQLite and are read per request; they are never resident in RAM on a
    /// durable server.
    blob_meta: Arc<Mutex<BTreeMap<String, BlobMeta>>>,
    /// Payload fallback for store-less servers (`HttpServerState::default()`
    /// in tests and dev tooling). A durable server never inserts here.
    blob_bytes_in_memory: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    /// Canonical externally reachable origin used in durable blob references.
    /// Request-derived hosts remain the local-development fallback only.
    public_url: Option<String>,
    ops_since_snapshot: Arc<Mutex<u64>>,
    /// True while a snapshot persist runs on its background thread; op
    /// triggers that land in the meantime skip instead of stacking threads.
    snapshot_in_flight: Arc<AtomicBool>,
    /// Long-poll wake signal (/sync/wait). A single hub: every accepted publish
    /// wakes all waiters, who re-check their own predicates. Sized for the
    /// current phase (hundreds of users); per-key channels are the documented
    /// next step if waiter counts grow.
    pub(crate) wake: Arc<tokio::sync::Notify>,
    store: Option<Arc<SqliteHttpDeliveryStore>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BlobObject {
    pub(crate) content_type: String,
    pub(crate) bytes: Vec<u8>,
}

/// Which storage backend holds a blob's payload bytes. Only `Sqlite` is
/// written today; `Object` is the S3 phase's marker, parsed now so a rolled-
/// forward database never fails to load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlobBackend {
    Sqlite,
    Object,
}

impl BlobBackend {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "sqlite" => Some(Self::Sqlite),
            "object" => Some(Self::Object),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BlobMeta {
    pub(crate) size_bytes: u64,
    pub(crate) content_type: String,
    pub(crate) backend: BlobBackend,
}

#[derive(Clone)]
pub(crate) struct SyncStreamCursors {
    pub(crate) rooms: Vec<SyncStreamRoomCursor>,
    pub(crate) inbox: Option<SyncStreamInboxCursor>,
}

#[derive(Clone)]
pub(crate) struct SyncStreamRoomCursor {
    pub(crate) room_id: String,
    pub(crate) after_seq: u64,
    pub(crate) seen_activity_received_at_ms: u64,
}

#[derive(Clone)]
pub(crate) struct SyncStreamInboxCursor {
    pub(crate) recipient: MemberId,
    pub(crate) after_seq: u64,
}

pub(crate) struct SyncStreamLoop {
    pub(crate) state: HttpServerState,
    pub(crate) cursors: SyncStreamCursors,
    pub(crate) pending: VecDeque<SyncHintEvent>,
    pub(crate) heartbeat_ms: u64,
}

impl HttpServerState {
    pub fn new(service: HttpDeliveryService) -> Self {
        Self {
            service: Arc::new(Mutex::new(service)),
            publish_idempotency: Arc::new(Mutex::new(HashMap::new())),
            key_package_claim_idempotency: Arc::new(Mutex::new(HashMap::new())),
            key_package_inventory: Arc::new(Mutex::new(HashMap::new())),
            revoked_devices: Arc::new(Mutex::new(BTreeSet::new())),
            pairing_sessions: Arc::new(Mutex::new(BTreeMap::new())),
            account_rooms: Arc::new(Mutex::new(BTreeMap::new())),
            room_memberships: Arc::new(Mutex::new(BTreeMap::new())),
            application_effects: Arc::new(Mutex::new(BTreeMap::new())),
            ephemeral_activity: Arc::new(Mutex::new(BTreeMap::new())),
            device_liveness: Arc::new(Mutex::new(BTreeMap::new())),
            nostr_profiles: Arc::new(Mutex::new(BTreeMap::new())),
            welcome_claims: Arc::new(Mutex::new(HashMap::new())),
            push_tokens: Arc::new(Mutex::new(BTreeMap::new())),
            push_wakes: Arc::new(Mutex::new(BTreeMap::new())),
            blob_meta: Arc::new(Mutex::new(BTreeMap::new())),
            blob_bytes_in_memory: Arc::new(Mutex::new(BTreeMap::new())),
            public_url: None,
            ops_since_snapshot: Arc::new(Mutex::new(0)),
            snapshot_in_flight: Arc::new(AtomicBool::new(false)),
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
        let pairing_sessions = store.load_pairing_sessions()?;
        let account_rooms = store.load_account_room_directory()?;
        let room_memberships = store.load_room_memberships()?;
        let application_effects = store.load_application_effects()?;
        let nostr_profiles = store.load_nostr_profiles()?;
        let welcome_claims = store.load_welcome_claims()?;
        let push_tokens = store.load_push_tokens()?;
        let push_wakes = store.load_push_wakes()?;
        // Meta only: payload bytes stay in SQLite and are read per request,
        // so boot cost and RSS no longer scale with stored blob volume.
        let blob_meta = store.load_blob_meta()?;
        Ok(Self {
            service: Arc::new(Mutex::new(service)),
            publish_idempotency: Arc::new(Mutex::new(publish_idempotency)),
            key_package_claim_idempotency: Arc::new(Mutex::new(key_package_claim_idempotency)),
            key_package_inventory: Arc::new(Mutex::new(key_package_inventory)),
            revoked_devices: Arc::new(Mutex::new(revoked_devices)),
            pairing_sessions: Arc::new(Mutex::new(pairing_sessions)),
            account_rooms: Arc::new(Mutex::new(account_rooms)),
            room_memberships: Arc::new(Mutex::new(room_memberships)),
            application_effects: Arc::new(Mutex::new(application_effects)),
            ephemeral_activity: Arc::new(Mutex::new(BTreeMap::new())),
            device_liveness: Arc::new(Mutex::new(BTreeMap::new())),
            nostr_profiles: Arc::new(Mutex::new(nostr_profiles)),
            welcome_claims: Arc::new(Mutex::new(welcome_claims)),
            push_tokens: Arc::new(Mutex::new(push_tokens)),
            push_wakes: Arc::new(Mutex::new(push_wakes)),
            blob_meta: Arc::new(Mutex::new(blob_meta)),
            blob_bytes_in_memory: Arc::new(Mutex::new(BTreeMap::new())),
            public_url: None,
            ops_since_snapshot: Arc::new(Mutex::new(0)),
            snapshot_in_flight: Arc::new(AtomicBool::new(false)),
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
        let mut meta = self.blob_meta.lock().expect("HTTP blob meta mutex");
        if let Some(existing) = meta.get(&sha256) {
            // Content addressing makes matching digest + length an identity
            // match. The old byte-for-byte comparison required every payload
            // resident in RAM and could only diverge from this check on a
            // sha256 collision.
            if existing.size_bytes == bytes.len() as u64 {
                return Ok(BlobDescriptor {
                    url: blob_url(self.public_url.as_deref(), headers, &sha256),
                    sha256,
                    size_bytes: bytes.len() as u64,
                });
            }
            return Err(ServerHttpError::BlobConflict { sha256 });
        }

        if let Some(store) = &self.store {
            store.insert_blob_object(&sha256, content_type, bytes)?;
        } else {
            self.blob_bytes_in_memory
                .lock()
                .expect("HTTP blob bytes mutex")
                .insert(sha256.clone(), bytes.to_vec());
        }
        meta.insert(
            sha256.clone(),
            BlobMeta {
                size_bytes: bytes.len() as u64,
                content_type: content_type.to_owned(),
                backend: BlobBackend::Sqlite,
            },
        );
        Ok(BlobDescriptor {
            url: blob_url(self.public_url.as_deref(), headers, &sha256),
            sha256,
            size_bytes: bytes.len() as u64,
        })
    }

    pub(crate) fn get_blob_object(&self, sha256: &str) -> Result<BlobObject, ServerHttpError> {
        validate_blob_sha256(sha256)?;
        let meta = {
            let metas = self.blob_meta.lock().expect("HTTP blob meta mutex");
            metas.get(sha256).cloned()
        };
        let Some(meta) = meta else {
            return Err(ServerHttpError::BlobNotFound {
                sha256: sha256.to_owned(),
            });
        };
        let bytes = if let Some(store) = &self.store {
            let Some(bytes) = store.load_blob_payload(sha256)? else {
                // Meta says present; a missing payload row is corruption, not
                // a 404 a client could mistake for permanent deletion.
                return Err(DurableStoreError::BlobObjectCorrupt {
                    sha256: sha256.to_owned(),
                }
                .into());
            };
            // Boot used to verify every stored blob up front; the same
            // verification now runs per read on just the requested object.
            if bytes.len() as u64 != meta.size_bytes || sha256 != sha256_hex(&bytes) {
                return Err(DurableStoreError::BlobObjectCorrupt {
                    sha256: sha256.to_owned(),
                }
                .into());
            }
            bytes
        } else {
            self.blob_bytes_in_memory
                .lock()
                .expect("HTTP blob bytes mutex")
                .get(sha256)
                .cloned()
                .ok_or_else(|| ServerHttpError::BlobNotFound {
                    sha256: sha256.to_owned(),
                })?
        };
        Ok(BlobObject {
            content_type: meta.content_type,
            bytes,
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

    pub(crate) fn claim_key_packages(
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

    pub(crate) fn expire_key_package_lease(
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

    pub(crate) fn revoke_device(
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

    pub(crate) fn observe_device_liveness(
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

    pub(crate) fn get_device_liveness(
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

    pub(crate) fn put_nostr_profile(
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

    pub(crate) fn get_nostr_profiles(
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

    pub(crate) fn get_key_package_availability(
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

    pub(crate) fn key_package_inventory(
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

    pub(crate) fn create_pairing_session(
        &self,
        request: CreatePairingSessionRequest,
    ) -> Result<HttpPairingSessionRecord, ServerHttpError> {
        if request.version != PAIRING_PROTOCOL_VERSION {
            return Err(pairing_invalid("unsupported pairing protocol version"));
        }
        validate_pairing_session_id(&request.pairing_session_id)?;
        validate_pairing_device_id(&request.target_device_id)?;
        let target_public_key = validate_pairing_public_key(&request.target_public_key)?;
        let issued_at_unix_seconds = pairing_now();
        let mut sessions = self
            .pairing_sessions
            .lock()
            .expect("HTTP pairing-session mutex");
        if sessions.contains_key(&request.pairing_session_id) {
            return Err(ServerHttpError::PairingSessionAlreadyExists {
                pairing_session_id: request.pairing_session_id,
            });
        }
        let record = HttpPairingSessionRecord {
            version: PAIRING_PROTOCOL_VERSION,
            pairing_session_id: request.pairing_session_id,
            target_device_id: request.target_device_id,
            target_public_key: target_public_key.to_hex(),
            issued_at_unix_seconds,
            expires_at_unix_seconds: issued_at_unix_seconds
                .saturating_add(PAIRING_SESSION_TTL_SECONDS),
            source_public_key: None,
            events: Vec::new(),
            state: HttpPairingSessionState::Created,
        };
        sessions.insert(record.pairing_session_id.clone(), record.clone());
        drop(sessions);

        if let Some(store) = &self.store {
            store.upsert_pairing_session(&record)?;
        }
        Ok(record)
    }

    pub(crate) fn get_pairing_session(
        &self,
        request: GetPairingSessionRequest,
    ) -> Result<Option<HttpPairingSessionRecord>, ServerHttpError> {
        validate_pairing_session_id(&request.pairing_session_id)?;
        let sessions = self
            .pairing_sessions
            .lock()
            .expect("HTTP pairing-session mutex");
        Ok(sessions.get(&request.pairing_session_id).cloned())
    }

    pub(crate) fn publish_pairing_offer(
        &self,
        request: PublishPairingOfferRequest,
    ) -> Result<HttpPairingSessionRecord, ServerHttpError> {
        validate_pairing_session_id(&request.pairing_session_id)?;
        let event = validate_pairing_event(&request.offer_event)?;
        let mut sessions = self
            .pairing_sessions
            .lock()
            .expect("HTTP pairing-session mutex");
        let session = sessions
            .get_mut(&request.pairing_session_id)
            .ok_or_else(|| ServerHttpError::PairingSessionNotFound {
                pairing_session_id: request.pairing_session_id.clone(),
            })?;
        ensure_pairing_session_open(session)?;
        if session.state != HttpPairingSessionState::Expired
            && session
                .events
                .first()
                .is_some_and(|stored| stored.event == request.offer_event)
        {
            return Ok(session.clone());
        }
        if session.state != HttpPairingSessionState::Created {
            return Err(ServerHttpError::PairingSessionClosed {
                pairing_session_id: request.pairing_session_id,
            });
        }
        let target = NostrPublicKey::from_hex(&session.target_public_key)
            .map_err(|_| pairing_corrupt("stored target public key is invalid"))?;
        if event.pubkey != target {
            return Err(pairing_conflict(
                &request.pairing_session_id,
                "offer sender does not match the bound target",
            ));
        }
        let source = pairing_recipient(&event)?;
        if source == target {
            return Err(pairing_conflict(
                &request.pairing_session_id,
                "source and target pairing keys must differ",
            ));
        }
        session.source_public_key = Some(source.to_hex());
        session.events.push(HttpPairingEventRecord {
            seq: 1,
            event: request.offer_event,
        });
        session.state = HttpPairingSessionState::OfferPublished;
        let record = session.clone();
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_pairing_session(&record)
        {
            session.source_public_key = None;
            session.events.clear();
            session.state = HttpPairingSessionState::Created;
            return Err(error.into());
        }
        drop(sessions);
        Ok(record)
    }

    pub(crate) fn publish_pairing_response(
        &self,
        request: PublishPairingResponseRequest,
    ) -> Result<HttpPairingSessionRecord, ServerHttpError> {
        validate_pairing_session_id(&request.pairing_session_id)?;
        let confirmation = validate_pairing_event(&request.source_confirmation_event)?;
        let payload = validate_pairing_event(&request.payload_event)?;
        if confirmation.id == payload.id {
            return Err(pairing_conflict(
                &request.pairing_session_id,
                "pairing response events must be distinct",
            ));
        }
        let mut sessions = self
            .pairing_sessions
            .lock()
            .expect("HTTP pairing-session mutex");
        let session = sessions
            .get_mut(&request.pairing_session_id)
            .ok_or_else(|| ServerHttpError::PairingSessionNotFound {
                pairing_session_id: request.pairing_session_id.clone(),
            })?;
        if session.state != HttpPairingSessionState::Expired
            && session
                .events
                .get(1)
                .is_some_and(|stored| stored.event == request.source_confirmation_event)
            && session
                .events
                .get(2)
                .is_some_and(|stored| stored.event == request.payload_event)
        {
            return Ok(session.clone());
        }
        if session.state != HttpPairingSessionState::OfferPublished {
            return Err(ServerHttpError::PairingSessionClosed {
                pairing_session_id: request.pairing_session_id,
            });
        }
        let source = session
            .source_public_key
            .as_deref()
            .and_then(|value| NostrPublicKey::from_hex(value).ok())
            .ok_or_else(|| pairing_corrupt("stored source public key is invalid"))?;
        let target = NostrPublicKey::from_hex(&session.target_public_key)
            .map_err(|_| pairing_corrupt("stored target public key is invalid"))?;
        for event in [&confirmation, &payload] {
            if event.pubkey != source || pairing_recipient(event)? != target {
                return Err(pairing_conflict(
                    &request.pairing_session_id,
                    "pairing response is not bound to this source and target",
                ));
            }
        }
        session.events.push(HttpPairingEventRecord {
            seq: 2,
            event: request.source_confirmation_event,
        });
        session.events.push(HttpPairingEventRecord {
            seq: 3,
            event: request.payload_event,
        });
        session.state = HttpPairingSessionState::ResponsePublished;
        let record = session.clone();
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_pairing_session(&record)
        {
            session.events.truncate(1);
            session.state = HttpPairingSessionState::OfferPublished;
            return Err(error.into());
        }
        drop(sessions);
        Ok(record)
    }

    pub(crate) fn publish_pairing_complete(
        &self,
        request: PublishPairingCompleteRequest,
    ) -> Result<HttpPairingSessionRecord, ServerHttpError> {
        validate_pairing_session_id(&request.pairing_session_id)?;
        let complete = validate_pairing_event(&request.complete_event)?;
        let mut sessions = self
            .pairing_sessions
            .lock()
            .expect("HTTP pairing-session mutex");
        let session = sessions
            .get_mut(&request.pairing_session_id)
            .ok_or_else(|| ServerHttpError::PairingSessionNotFound {
                pairing_session_id: request.pairing_session_id.clone(),
            })?;
        ensure_pairing_session_open(session)?;
        if session.state == HttpPairingSessionState::Completed
            && session
                .events
                .get(3)
                .is_some_and(|stored| stored.event == request.complete_event)
        {
            return Ok(session.clone());
        }
        if session.state != HttpPairingSessionState::ResponsePublished {
            return Err(ServerHttpError::PairingSessionClosed {
                pairing_session_id: request.pairing_session_id,
            });
        }
        let source = session
            .source_public_key
            .as_deref()
            .and_then(|value| NostrPublicKey::from_hex(value).ok())
            .ok_or_else(|| pairing_corrupt("stored source public key is invalid"))?;
        let target = NostrPublicKey::from_hex(&session.target_public_key)
            .map_err(|_| pairing_corrupt("stored target public key is invalid"))?;
        if complete.pubkey != target || pairing_recipient(&complete)? != source {
            return Err(pairing_conflict(
                &request.pairing_session_id,
                "completion is not bound to this source and target",
            ));
        }
        session.events.push(HttpPairingEventRecord {
            seq: 4,
            event: request.complete_event,
        });
        session.state = HttpPairingSessionState::Completed;
        let record = session.clone();
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_pairing_session(&record)
        {
            session.events.truncate(3);
            session.state = HttpPairingSessionState::ResponsePublished;
            return Err(error.into());
        }
        drop(sessions);
        Ok(record)
    }

    pub(crate) fn expire_pairing_session(
        &self,
        request: ExpirePairingSessionRequest,
    ) -> Result<ExpirePairingSessionResponse, ServerHttpError> {
        validate_pairing_session_id(&request.pairing_session_id)?;
        let mut sessions = self
            .pairing_sessions
            .lock()
            .expect("HTTP pairing-session mutex");
        let session = sessions
            .get_mut(&request.pairing_session_id)
            .ok_or_else(|| ServerHttpError::PairingSessionNotFound {
                pairing_session_id: request.pairing_session_id.clone(),
            })?;
        if session.state == HttpPairingSessionState::Completed {
            return Err(ServerHttpError::PairingSessionClosed {
                pairing_session_id: request.pairing_session_id,
            });
        }
        let prior = session.state.clone();
        session.state = HttpPairingSessionState::Expired;
        let record = session.clone();
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_pairing_session(&record)
        {
            session.state = prior;
            return Err(error.into());
        }
        drop(sessions);
        Ok(ExpirePairingSessionResponse { expired: true })
    }

    /// The /sync/wait predicate: any watched room advanced past its cursor.
    pub(crate) fn check_wait_signal(&self, request: &SyncWaitRequest) -> Option<String> {
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

    pub(crate) fn collect_sync_hints(&self, cursors: &mut SyncStreamCursors) -> Vec<SyncHintEvent> {
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

    pub(crate) fn activity_highwater_for_room(&self, room_id: &str) -> u64 {
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

    pub(crate) fn save_account_room(
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

    pub(crate) fn bootstrap_account_room(
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

    pub(crate) fn list_account_rooms(
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

    pub(crate) fn report_invalid_commit(
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

    pub(crate) fn submit_commit(
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

    pub(crate) fn append_application_event(
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

    pub(crate) fn application_effect(
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

    pub(crate) fn application_effect_counts(
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

    pub(crate) fn claim_push_wakes(
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

    pub(crate) fn ack_push_wake(
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

    pub(crate) fn fail_push_wake(
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

    pub(crate) fn append_ephemeral_activity(
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

    pub(crate) fn get_ephemeral_activities(
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

    pub(crate) fn claim_welcomes(
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

    pub(crate) fn ack_welcome(
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

    pub(crate) fn leave_room(
        &self,
        request: LeaveRoomRequest,
    ) -> Result<LeaveRoomResponse, ServerHttpError> {
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

    pub(crate) fn update_room_admins(
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

    pub(crate) fn register_push_token(
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

    pub(crate) fn remove_push_token(
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
        let (snapshot, last_op_seq) = {
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
            (snapshot, last_op_seq)
        };
        // Serialization and the SQLite write run after the state locks drop,
        // so request handlers only ever wait behind the capture clone.
        store.save_state_snapshot(last_op_seq, &snapshot)?;
        *self
            .ops_since_snapshot
            .lock()
            .expect("snapshot counter mutex") = 0;
        Ok(())
    }

    pub(crate) fn note_op_for_snapshot(&self) {
        let due = {
            let mut counter = self
                .ops_since_snapshot
                .lock()
                .expect("snapshot counter mutex");
            *counter += 1;
            if *counter < SNAPSHOT_INTERVAL_OPS {
                false
            } else {
                // Reset at attempt start, not on success: a failing snapshot
                // retries once per interval instead of on every following op.
                *counter = 0;
                true
            }
        };
        if !due {
            return;
        }
        if self
            .snapshot_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        // Snapshotting is an optimization; it runs on its own thread so the
        // triggering request neither waits for it nor fails with it.
        let state = self.clone();
        std::thread::spawn(move || {
            if let Err(error) = state.snapshot_now() {
                eprintln!("finitechat-server: state snapshot failed: {error:?}");
            }
            state.snapshot_in_flight.store(false, Ordering::Release);
        });
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PublishMessageFingerprint {
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
pub(crate) struct PublishIdempotencyRecord {
    pub(crate) fingerprint: PublishMessageFingerprint,
    pub(crate) receipt: HttpPublishReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KeyPackageClaimFingerprint {
    owners: Vec<MemberId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KeyPackageClaimIdempotencyRecord {
    pub(crate) fingerprint: KeyPackageClaimFingerprint,
    pub(crate) response: Vec<HttpKeyPackageClaim>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KeyPackageInventoryRecord {
    pub(crate) key_package_id: HttpKeyPackageId,
    pub(crate) owner: MemberId,
    pub(crate) key_package: KeyPackage,
    pub(crate) state: KeyPackageInventoryState,
    pub(crate) finite_metadata: Option<FiniteKeyPackageMetadata>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum KeyPackageInventoryState {
    Available,
    Claimed,
    Consumed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FiniteKeyPackageMetadata {
    pub(crate) owner: DeviceRef,
    key_package_ref: String,
    key_package_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WelcomeClaimRecord {
    pub(crate) recipient: MemberId,
    pub(crate) seq: HttpSequence,
    pub(crate) message: TransportMessage,
    pub(crate) state: WelcomeClaimState,
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
pub(crate) struct AccountRoomDirectoryRecord {
    pub(crate) account_id: String,
    pub(crate) room_id: String,
    pub(crate) record: Value,
}

/// Everything `from_sqlite_path` otherwise derives by replaying the full
/// operation log. Snapshotting it makes startup snapshot + tail replay, per
/// the standing constraint that full-history replay is a rare recovery
/// action (ADR 0003).
#[derive(Serialize, Deserialize)]
pub(crate) struct DurableStateSnapshot {
    service: HttpDeliveryService,
    // Stored as a list: JSON maps need string keys, and the record carries
    // its own id.
    key_package_inventory: Vec<KeyPackageInventoryRecord>,
    revoked_devices: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AccountRoomDirectoryMutation {
    pub(crate) deletes: Vec<(String, String)>,
    pub(crate) upserts: Vec<AccountRoomDirectoryRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PublishMutation {
    pub(crate) operation: Option<PersistedOperation>,
    pub(crate) idempotency_key: String,
    pub(crate) record: PublishIdempotencyRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PushWakeOutboxRecord {
    pub(crate) wake_id: String,
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

pub(crate) fn mark_next_key_package_claimed(
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

pub(crate) fn consume_key_packages_from_persisted_message(
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

pub(crate) fn finite_key_package_metadata(
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

pub(crate) fn retire_older_finite_key_packages(
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
