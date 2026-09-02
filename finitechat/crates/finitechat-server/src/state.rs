//! Server state ([`HttpServerState`]), its domain methods, and the
//! server-side record types they operate on.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::net::IpAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use finitechat_blob::BlobDescriptor;
use finitechat_delivery::{
    HTTP_SERVER_SOURCE, HttpClaimedKeyPackage, HttpCommitAdmission, HttpKeyPackageId,
    HttpKeyPackagePublication, HttpPublishReceipt, HttpPublishTarget, HttpSequence,
    HttpServerError, HttpSyncPage, MAX_HTTP_SYNC_PAGE_ENTRIES,
};
use finitechat_http::{
    AckWelcomeRequest, AckWelcomeResponse, ApplicationEffectCountsResponse,
    ApplicationEffectRequest, BootstrapAccountRoomRequest, BootstrapAccountRoomResponse,
    ClaimKeyPackageForAccountRequest, ClaimKeyPackageRequest, ClaimKeyPackagesRequest,
    ClaimWelcomesRequest, CreatePairingSessionRequest, DeviceLivenessRecord,
    ExpireKeyPackageLeaseRequest, ExpireKeyPackageLeaseResponse, ExpirePairingSessionRequest,
    ExpirePairingSessionResponse, FiniteAccountRoomCommitProjection, GetDeviceLivenessRequest,
    GetDeviceLivenessResponse, GetEphemeralActivitiesRequest, GetEphemeralActivitiesResponse,
    GetKeyPackageAvailabilityRequest, GetKeyPackageAvailabilityResponse, GetNostrProfilesRequest,
    GetNostrProfilesResponse, GetPairingSessionRequest, GroupSyncRequest,
    HttpApplicationDeliveryEffect, HttpClaimedWelcome, HttpKeyPackageClaim,
    HttpKeyPackageInventory, HttpPairingEventRecord, HttpPairingSessionRecord,
    HttpPairingSessionState, KeyPackageAvailabilityEntry, KeyPackageInventoryRequest,
    LeaveRoomRequest, LeaveRoomResponse, ListAccountRoomDirectoryRequest,
    ListAccountRoomDirectoryResponse, NostrProfileCacheEntry, NostrProfileRecord,
    ObserveDeviceLivenessRequest, PublishKeyPackageResponse, PublishMessageRequest,
    PublishPairingCompleteRequest, PublishPairingOfferRequest, PublishPairingResponseRequest,
    PutNostrProfileRequest, PutNostrProfileResponse, ReportInvalidCommitRequest,
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
use finitechat_transport::{EpochId, GroupId, MemberId, MessageId};
use nostr::PublicKey as NostrPublicKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::projections::{
    HttpRoomMembershipProjection, ObservedRoomHead, apply_room_membership_delta,
    ensure_device_not_revoked_in, ensure_welcome_message_recipient_not_revoked, group_id_for_room,
    initial_room_membership_projection, member_id_for_device, room_id_for_group_id,
    transport_group_id_for_room, validate_membership_adds_for_projection,
};
use crate::store::metadata;
use crate::store::{SqlDelivery, Store};
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
    DurableStoreError, HttpServerConfigurationError, PAIRING_PROTOCOL_VERSION,
    PAIRING_SESSION_TTL_SECONDS, SNAPSHOT_INTERVAL_OPS, ServerHttpError, finite_delivery_limits,
};

/// Per-route activity records inside one `(room_id, conversation_id)` bucket,
/// keyed by the sender-inclusive wire `route_key`.
type EphemeralActivityBucket = BTreeMap<String, Vec<EphemeralActivityRecord>>;

/// RAM-only activity cache keyed for readers: every query is per room (and
/// per conversation for `/activities/get`), so records bucket by
/// `(room_id, conversation_id)`.
type EphemeralActivityCache = BTreeMap<(String, Option<String>), EphemeralActivityBucket>;

pub(crate) const READINESS_BUDGET_MILLIS: u64 = 1_000;
const READINESS_COMPONENT_BUDGET: Duration = Duration::from_millis(450);
// Cache contract: the live delivery-core sync plus committed SQLite read-back
// below are the source of truth. Completion time invalidates a result after 30
// seconds. While one replacement probe is running, concurrent callers may see
// at most 65-second-old evidence so the host and public checks coalesce; once
// that bound passes they fail closed instead of serving indefinitely stale
// green. Failures are cached too, so recovery may remain red for up to 30s.
const READINESS_CACHE_TTL: Duration = Duration::from_secs(30);
const READINESS_MAX_STALE_WHILE_IN_FLIGHT: Duration = Duration::from_secs(65);

#[derive(Clone, Debug)]
pub(crate) struct ReadinessCheckResult {
    pub(crate) elapsed: Duration,
    pub(crate) failure: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServerReadiness {
    pub(crate) delivery_core: ReadinessCheckResult,
    pub(crate) durable_store: ReadinessCheckResult,
    pub(crate) elapsed: Duration,
    pub(crate) budget_exceeded: bool,
}

impl ServerReadiness {
    pub(crate) fn is_ready(&self) -> bool {
        self.delivery_core.failure.is_none()
            && self.durable_store.failure.is_none()
            && !self.budget_exceeded
    }

    fn unavailable(reason: &'static str) -> Self {
        let failed = ReadinessCheckResult {
            elapsed: Duration::ZERO,
            failure: Some(reason),
        };
        Self {
            delivery_core: failed.clone(),
            durable_store: failed,
            elapsed: Duration::ZERO,
            budget_exceeded: false,
        }
    }
}

#[derive(Debug, Default)]
struct ReadinessCache {
    last: Option<CachedReadiness>,
    in_flight: bool,
}

#[derive(Clone, Debug)]
struct CachedReadiness {
    completed_at: Instant,
    result: ServerReadiness,
}

/// Default allowance for the public mutating routes: generous headroom over
/// what a healthy device can produce, tight enough to blunt naive abuse.
/// Device-link export legitimately publishes and claims hundreds of
/// KeyPackages per minute from one address, so the floor sits well above that.
pub const DEFAULT_RATE_LIMIT_PER_WINDOW: u32 = 1_200;
pub const DEFAULT_RATE_LIMIT_WINDOW_SECONDS: u64 = 60;

/// Cap on tracked client buckets; past it, each check sweeps expired
/// windows so a spray of spoofed/one-shot IPs cannot grow the map without
/// bound.
const MAX_RATE_LIMIT_ENTRIES: usize = 10_000;

/// Hand-rolled fixed-window per-IP rate limiter for the public mutating
/// routes (KeyPackages, pairing sessions, uploads). A
/// `Mutex<HashMap<IpAddr, (window_start, count)>>` is enough at the current
/// fleet size and adds no dependency.
#[derive(Debug)]
pub(crate) struct PublicRouteRateLimiter {
    max_requests: u32,
    window: Duration,
    max_entries: usize,
    windows: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl PublicRouteRateLimiter {
    pub(crate) fn new(max_requests: u32, window_seconds: u64) -> Self {
        Self {
            max_requests,
            window: Duration::from_secs(window_seconds),
            max_entries: MAX_RATE_LIMIT_ENTRIES,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Record one request from `ip`; false once the window allowance is spent.
    pub(crate) fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if windows.len() >= self.max_entries {
            windows.retain(|_, (started, _)| now.duration_since(*started) < self.window);
        }
        match windows.get_mut(&ip) {
            Some((started, count)) if now.duration_since(*started) < self.window => {
                if *count >= self.max_requests {
                    return false;
                }
                *count += 1;
                true
            }
            _ => {
                windows.insert(ip, (now, 1));
                true
            }
        }
    }
}

impl Default for PublicRouteRateLimiter {
    fn default() -> Self {
        Self::new(
            DEFAULT_RATE_LIMIT_PER_WINDOW,
            DEFAULT_RATE_LIMIT_WINDOW_SECONDS,
        )
    }
}

#[derive(Clone)]
pub struct HttpServerState {
    publish_idempotency: Arc<Mutex<HashMap<String, PublishIdempotencyRecord>>>,
    key_package_claim_idempotency: Arc<Mutex<HashMap<String, KeyPackageClaimIdempotencyRecord>>>,
    key_package_inventory: Arc<Mutex<HashMap<HttpKeyPackageId, KeyPackageInventoryRecord>>>,
    revoked_devices: Arc<Mutex<BTreeSet<String>>>,
    pairing_sessions: Arc<Mutex<BTreeMap<String, HttpPairingSessionRecord>>>,
    account_rooms: Arc<Mutex<BTreeMap<String, BTreeMap<String, Value>>>>,
    room_memberships: Arc<Mutex<BTreeMap<String, HttpRoomMembershipProjection>>>,
    application_effects: Arc<Mutex<BTreeMap<String, HttpApplicationDeliveryEffect>>>,
    ephemeral_activity: Arc<Mutex<EphemeralActivityCache>>,
    device_liveness: Arc<Mutex<BTreeMap<String, DeviceLivenessRecord>>>,
    nostr_profiles: Arc<Mutex<BTreeMap<String, NostrProfileRecord>>>,
    welcome_claims: Arc<Mutex<HashMap<MessageId, WelcomeClaimRecord>>>,
    /// Blob metadata only (tens of bytes per blob). Payload bytes live in
    /// SQLite and are read per request; they are never resident in RAM on a
    /// durable server.
    blob_meta: Arc<Mutex<BTreeMap<String, BlobMeta>>>,
    /// Canonical externally reachable origin used in durable blob references.
    /// Request-derived hosts remain the local-development fallback only.
    public_url: Option<String>,
    /// Mixed-version rollout gate for NIP-98 request auth on account-scoped
    /// routes. When false, requests without an `Authorization` header are
    /// still accepted (old clients) and a present-but-invalid header is
    /// logged and ignored (upgraded clients may sign a dial URL that differs
    /// from this server's public URL). When true, a missing or invalid
    /// header is rejected. A signature that validates binds the signer to
    /// the body account in both modes.
    require_signed_requests: bool,
    rate_limiter: Arc<PublicRouteRateLimiter>,
    ops_since_snapshot: Arc<Mutex<u64>>,
    /// True while a checkpoint persist runs on its background thread; op
    /// triggers that land in the meantime skip instead of stacking threads.
    snapshot_in_flight: Arc<AtomicBool>,
    /// Bounds the public readiness endpoint's SQLite write rate and coalesces
    /// concurrent internal/external probes. This must never be used by chat
    /// request paths as serving evidence; it only protects the probe itself.
    readiness_cache: Arc<Mutex<ReadinessCache>>,
    /// Long-poll wake signal (/sync/wait). A single hub: every accepted publish
    /// wakes all waiters, who re-check their own predicates. Sized for the
    /// current phase (hundreds of users); per-key channels are the documented
    /// next step if waiter counts grow.
    pub(crate) wake: Arc<tokio::sync::Notify>,
    /// The normalized delivery engine. Delivery state lives in
    /// `delivery_routes`/`delivery_entries`/`group_commit_epochs`/
    /// `sql_key_packages`, room state in the single `room_state_checkpoint`
    /// structure plus the `account_room_directory` current-state table, and
    /// shared metadata (idempotency, claims, blobs, ...) in the
    /// metadata tables behind the same [`Store`]. There is no engine flag
    /// and no second engine: a boot either folds a pre-cutover database
    /// (`crate::cutover`) or starts fresh, and every request path serves
    /// through this engine.
    sql_delivery: Arc<SqlDelivery>,
    /// The normalized engine's ordering authority: plan-then-commit publish
    /// transactions serialize behind this lock so predicted seqs cannot be
    /// invalidated between read and write.
    delivery_ordering: Arc<Mutex<()>>,
    /// The durable database's path; `None` on volatile (`new()`) servers.
    /// The readiness probe uses it to bound its own write wait without
    /// borrowing the engine's writer connection.
    sqlite_path: Option<std::path::PathBuf>,
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

/// Manual Debug: the state owns SQLite connections that are not `Debug`.
impl std::fmt::Debug for HttpServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpServerState")
            .field("engine", &"normalized")
            .field("public_url", &self.public_url)
            .finish_non_exhaustive()
    }
}

impl Default for HttpServerState {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpServerState {
    /// A volatile server backed by a private in-memory SQLite database.
    /// Tests, dev tooling, and `serve` without `--state` use this; it has
    /// the same engine and the same durability-per-request semantics as a
    /// file-backed server, and no state survives the process.
    pub fn new() -> Self {
        let store = Store::open_in_memory().expect("in-memory SQLite store");
        store
            .write(|tx| Ok(metadata::init_schema(tx)?))
            .expect("in-memory metadata schema");
        let sql_delivery = Arc::new(SqlDelivery::new(store, finite_delivery_limits()));
        Self {
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
            blob_meta: Arc::new(Mutex::new(BTreeMap::new())),
            public_url: None,
            require_signed_requests: false,
            rate_limiter: Arc::new(PublicRouteRateLimiter::default()),
            ops_since_snapshot: Arc::new(Mutex::new(0)),
            snapshot_in_flight: Arc::new(AtomicBool::new(false)),
            readiness_cache: Arc::new(Mutex::new(ReadinessCache::default())),
            wake: Arc::new(tokio::sync::Notify::new()),
            sql_delivery,
            delivery_ordering: Arc::new(Mutex::new(())),
            sqlite_path: None,
        }
    }

    pub fn with_public_url(
        mut self,
        public_url: impl AsRef<str>,
    ) -> Result<Self, HttpServerConfigurationError> {
        self.public_url = Some(normalize_public_url(public_url.as_ref())?);
        Ok(self)
    }

    pub fn with_require_signed_requests(mut self, require_signed_requests: bool) -> Self {
        self.require_signed_requests = require_signed_requests;
        self
    }

    pub fn with_rate_limit(mut self, max_requests: u32, window_seconds: u64) -> Self {
        self.rate_limiter = Arc::new(PublicRouteRateLimiter::new(max_requests, window_seconds));
        self
    }

    /// Record one request to a public mutating route from `ip`; false once
    /// the per-IP window allowance is spent.
    pub(crate) fn check_public_route_rate_limit(&self, ip: IpAddr) -> bool {
        self.rate_limiter.check(ip)
    }

    pub(crate) fn public_url(&self) -> Option<&str> {
        self.public_url.as_deref()
    }

    pub(crate) fn require_signed_requests(&self) -> bool {
        self.require_signed_requests
    }

    /// Boot a durable server from `path`. THE single boot path (chat store
    /// swap): there is no engine flag — the deploy IS the flip.
    ///
    /// 1. First boot of this build on a pre-cutover database runs the
    ///    one-time guarded fold (`crate::cutover`): the retained legacy
    ///    reader boots one final time (v2 snapshot + op-log tail + #770
    ///    reconciliation) and its state is transplanted into the normalized
    ///    tables inside ONE boot transaction with row counts and a sampled
    ///    fold asserted before the marker commits. A fresh database (no
    ///    legacy tables) skips the fold entirely and starts normalized.
    /// 2. Steady state: the account-room directory and revoked devices load
    ///    from their current-state tables, the room-membership projections
    ///    load from exactly ONE durable structure (`room_state_checkpoint`)
    ///    and re-derive from the `delivery_entries` tails, and acked
    ///    Welcomes re-activate from the claims table. The legacy tables
    ///    (`http_state_snapshots_v2`, `http_room_memberships`,
    ///    `http_account_rooms`, `http_delivery_ops`) are never read.
    /// 3. Divergence is impossible-or-blocking: a checkpoint that lags
    ///    simply replays more entries; one that disagrees with the route
    ///    heads (ahead, or a delta that will not apply) fails boot closed
    ///    instead of being absorbed — the #770 lesson.
    ///
    /// Shared metadata (pairing sessions, nostr profiles,
    /// blobs, welcome claims, application effects, publish/claim
    /// idempotency, the finite KeyPackage inventory) loads through the same
    /// [`Store`] from the tables it owns in `store::metadata`.
    pub fn from_sqlite_path(path: impl AsRef<Path>) -> Result<Self, DurableStoreError> {
        crate::print_engine_rollout_banner();
        Self::boot(path)
    }

    fn boot(path: impl AsRef<Path>) -> Result<Self, DurableStoreError> {
        let store = Store::open_file(&path)?;
        store
            .write(|tx| Ok(metadata::init_schema(tx)?))
            .map_err(|error| match error {
                crate::store::StoreWriteError::Store(error) => error,
                crate::store::StoreWriteError::Domain(error) => DurableStoreError::Replay(error),
            })?;
        if let Some(report) = crate::cutover::fold_if_needed(&store, path.as_ref())? {
            eprintln!(
                "finitechat-server: op-log fold complete ({} routes, {} entries, \
                 {} commit epochs, {} key packages, {} directory rows, {} revoked devices, \
                 {} checkpoint rooms; {} routes and {} directory rows sampled-verified)",
                report.routes,
                report.entries,
                report.commit_epochs,
                report.key_packages,
                report.directory_rows,
                report.revoked_devices,
                report.checkpoint_rooms,
                report.sampled_routes,
                report.sampled_directory_rows
            );
        }
        let sql_delivery = Arc::new(SqlDelivery::new(store, finite_delivery_limits()));

        let sql_store = sql_delivery.store();
        let (publish_idempotency, key_package_claim_idempotency, key_package_inventory) = sql_store
            .read(|conn| {
                Ok((
                    metadata::load_publish_idempotency(conn)?,
                    metadata::load_key_package_claim_idempotency(conn)?,
                    metadata::load_key_package_inventory(conn)?,
                ))
            })
            .map_err(normalized_store_error)?;
        let (pairing_sessions, nostr_profiles, welcome_claims, blob_meta) = sql_store
            .read(|conn| {
                Ok((
                    metadata::load_pairing_sessions(conn)?,
                    metadata::load_nostr_profiles(conn)?,
                    metadata::load_welcome_claims(conn)?,
                    metadata::load_blob_meta(conn)?,
                ))
            })
            .map_err(normalized_store_error)?;

        // Normalized-owned state: directory + revoked devices are current
        // state; rooms derive from the checkpoint plus entry tails.
        let revoked_devices = sql_store
            .read(|conn| Ok(crate::store::room_state::load_revoked_devices(conn)?))
            .map_err(normalized_store_error)?;
        let mut account_rooms = sql_store
            .read(|conn| Ok(crate::store::room_state::load_directory(conn)?))
            .map_err(normalized_store_error)?;
        let room_memberships = Self::derive_rooms_from_checkpoint(sql_store, &welcome_claims)
            .map_err(normalized_store_error)?;
        let directory_mutation =
            Self::activate_acked_welcomes_in_directory(&mut account_rooms, &welcome_claims);
        // Persist the boot derivation: a fresh checkpoint of the derived
        // rooms (the loaded one may have lagged the tails) plus the welcome
        // activations' directory rows, in one transaction. Failing to
        // persist it fails the boot rather than serving state the next
        // boot would re-derive differently.
        if !directory_mutation.deletes.is_empty() || !directory_mutation.upserts.is_empty() {
            sql_store
                .write(|tx| {
                    Ok(crate::store::room_state::apply_directory_mutation(
                        tx,
                        &directory_mutation,
                    )?)
                })
                .map_err(normalized_store_error)?;
        }
        Self::checkpoint_rooms(&room_memberships, sql_store).map_err(normalized_store_error)?;
        let application_effects = sql_store
            .read(|conn| Ok(metadata::load_application_effects(conn)?))
            .map_err(normalized_store_error)?;

        Ok(Self {
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
            blob_meta: Arc::new(Mutex::new(blob_meta)),
            public_url: None,
            require_signed_requests: false,
            rate_limiter: Arc::new(PublicRouteRateLimiter::default()),
            ops_since_snapshot: Arc::new(Mutex::new(0)),
            snapshot_in_flight: Arc::new(AtomicBool::new(false)),
            readiness_cache: Arc::new(Mutex::new(ReadinessCache::default())),
            wake: Arc::new(tokio::sync::Notify::new()),
            sql_delivery,
            delivery_ordering: Arc::new(Mutex::new(())),
            sqlite_path: Some(path.as_ref().to_path_buf()),
        })
    }

    /// The single room-state derivation: checkpoint rooms, replay every
    /// room's `delivery_entries` tail above the checkpoint watermark,
    /// re-activate acked Welcomes, then assert the head invariant. Any
    /// checkpoint/entry disagreement fails closed — never the #770
    /// advance-but-stay-frozen fallback.
    fn derive_rooms_from_checkpoint(
        store: &Store,
        welcome_claims: &HashMap<MessageId, WelcomeClaimRecord>,
    ) -> Result<BTreeMap<String, HttpRoomMembershipProjection>, crate::store::StoreWriteError> {
        store.read(|conn| {
            let checkpoint = crate::store::room_state::load_checkpoint(conn)?.unwrap_or_default();
            let mut rooms = checkpoint.rooms;
            for (room_id, head) in crate::store::room_state::group_room_heads(conn)? {
                let Some(projection) = rooms.get(&room_id) else {
                    // Rooms without a checkpoint entry keep the typed
                    // bootstrap path, exactly like legacy rooms without a
                    // durable projection row: the log stays authoritative
                    // and the next bootstrap derives from it.
                    continue;
                };
                if projection.last_seq > head {
                    return Err(divergence(format!(
                        "room {room_id} checkpoint is ahead of its delivery route \
                         (projection last_seq {}, route head {head})",
                        projection.last_seq
                    )));
                }
                let tail = crate::store::room_state::group_entries_after(
                    conn,
                    &room_id,
                    projection.last_seq,
                )?;
                let mls_group_id = projection.mls_group_id.clone();
                for queued in &tail {
                    let commit = serde_json::from_slice::<FiniteAccountRoomCommitProjection>(
                        &queued.message.payload,
                    )
                    .ok()
                    .filter(|commit| {
                        commit.entry.room_id == room_id && commit.entry.kind == LogEntryKind::Commit
                    });
                    match commit {
                        Some(commit) => {
                            if let Err(error) = apply_room_membership_delta(
                                &mut rooms,
                                &room_id,
                                &mls_group_id,
                                &commit.entry.sender,
                                commit.entry.epoch,
                                &commit.membership_delta,
                                queued.seq,
                            ) {
                                return Err(divergence(format!(
                                    "room {room_id} membership delta replay broke at seq {} \
                                     ({error:?}); the checkpoint disagrees with the delivery log",
                                    queued.seq
                                )));
                            }
                        }
                        None => {
                            if let Some(projection) = rooms.get_mut(&room_id) {
                                projection.last_seq = projection.last_seq.max(queued.seq);
                            }
                        }
                    }
                }
                if let Some(projection) = rooms.get(&room_id)
                    && projection.last_seq != head
                {
                    return Err(divergence(format!(
                        "room {room_id} derived head {} does not match its delivery route \
                         head {head}",
                        projection.last_seq
                    )));
                }
            }
            // Rooms with a projection but no delivery route can only be
            // bootstrap-only rooms (no entries yet); anything else is a
            // checkpoint that claims history the entries do not hold.
            for (room_id, projection) in &rooms {
                if projection.last_seq != 0
                    && crate::store::room_state::group_entries_after(conn, room_id, 0)?.is_empty()
                {
                    return Err(divergence(format!(
                        "room {room_id} checkpoint claims last_seq {} but the delivery \
                         route holds no entries",
                        projection.last_seq
                    )));
                }
            }
            // Welcome ACKs are durable delivery events, not group-log
            // entries: the delta replay above re-creates added intervals as
            // pending, so activate the ones whose Welcomes were already
            // acked. Mere claims stay pending, exactly like the live routes.
            for claim in welcome_claims.values() {
                if claim.state != WelcomeClaimState::Acked {
                    continue;
                }
                let Ok(welcome) = serde_json::from_slice::<WelcomeRecord>(&claim.message.payload)
                else {
                    continue;
                };
                if claim.message.id.as_slice() != welcome.welcome_id.as_bytes() {
                    continue;
                }
                if let Some(projection) = rooms.get_mut(&welcome.room_id) {
                    projection.activate_interval(&welcome.recipient, welcome.commit_seq);
                }
            }
            Ok(rooms)
        })
    }

    /// Mirror of the legacy boot's directory welcome-activation: mark acked
    /// recipients active in the account-room directory records that still
    /// show them pending. Returns the rows to persist (boot writes them in
    /// one transaction).
    fn activate_acked_welcomes_in_directory(
        directory: &mut BTreeMap<String, BTreeMap<String, Value>>,
        welcome_claims: &HashMap<MessageId, WelcomeClaimRecord>,
    ) -> AccountRoomDirectoryMutation {
        let mut mutation = AccountRoomDirectoryMutation::default();
        for claim in welcome_claims.values() {
            if claim.state != WelcomeClaimState::Acked {
                continue;
            }
            let Ok(welcome) = serde_json::from_slice::<WelcomeRecord>(&claim.message.payload)
            else {
                continue;
            };
            if claim.message.id.as_slice() != welcome.welcome_id.as_bytes() {
                continue;
            }
            if let Some(record) = activate_account_room_device_in_directory(
                directory,
                &welcome.recipient,
                &welcome.room_id,
            ) {
                mutation.upserts.push(record);
            }
        }
        mutation
    }

    /// Write a fresh room-state checkpoint (the normalized engine's single
    /// room-state structure) from the live map.
    fn checkpoint_rooms(
        rooms: &BTreeMap<String, HttpRoomMembershipProjection>,
        store: &Store,
    ) -> Result<(), crate::store::StoreWriteError> {
        let checkpoint = crate::store::room_state::RoomStateCheckpoint {
            rooms: rooms.clone(),
        };
        store.write(|tx| Ok(crate::store::room_state::save_checkpoint(tx, &checkpoint)?))
    }

    /// Upsert one account-room directory row (direct saves, bootstraps,
    /// welcome activations). Current state, written immediately.
    fn upsert_account_room_row(
        &self,
        record: &AccountRoomDirectoryRecord,
    ) -> Result<(), ServerHttpError> {
        let mutation = AccountRoomDirectoryMutation {
            deletes: Vec::new(),
            upserts: vec![record.clone()],
        };
        self.sql_delivery
            .store()
            .write(|tx| {
                Ok(crate::store::room_state::apply_directory_mutation(
                    tx, &mutation,
                )?)
            })
            .map_err(sql_write_error)?;
        Ok(())
    }

    /// Persist the live room-membership map as a fresh checkpoint. Called
    /// for the rare primary room mutations (bootstrap, leave, admin
    /// changes, repair) whose projection state is not re-derivable from
    /// delivery entries alone; commit-derived projection changes ride the
    /// snapshot cadence instead.
    fn normalized_checkpoint_rooms(&self) -> Result<(), ServerHttpError> {
        let rooms = self
            .room_memberships
            .lock()
            .expect("HTTP room-membership mutex")
            .clone();
        Self::checkpoint_rooms(&rooms, self.sql_delivery.store()).map_err(sql_write_error)
    }

    /// Exercise the two shared seams every delivered chat message needs:
    /// the delivery-core read contract and a committed write through the
    /// durable SQLite store.
    pub(crate) fn probe_readiness(&self) -> ServerReadiness {
        let now = Instant::now();
        let mut cache = match self.readiness_cache.lock() {
            Ok(cache) => cache,
            Err(_) => return ServerReadiness::unavailable("readiness_cache_lock_poisoned"),
        };
        if let Some(cached) = &cache.last
            && now.saturating_duration_since(cached.completed_at) <= READINESS_CACHE_TTL
        {
            return cached.result.clone();
        }
        if cache.in_flight {
            // A simultaneous host/public check may reuse older evidence while
            // the first caller performs the new bounded transaction. The
            // first caller remains responsible for publishing the fresh red
            // result if that transaction stalls or fails.
            return cache
                .last
                .as_ref()
                .filter(|cached| {
                    now.saturating_duration_since(cached.completed_at)
                        <= READINESS_MAX_STALE_WHILE_IN_FLIGHT
                })
                .map_or_else(
                    || ServerReadiness::unavailable("readiness_probe_in_flight"),
                    |cached| cached.result.clone(),
                );
        }
        cache.in_flight = true;
        drop(cache);

        let result = self.probe_readiness_uncached();
        if let Ok(mut cache) = self.readiness_cache.lock() {
            cache.in_flight = false;
            cache.last = Some(CachedReadiness {
                completed_at: Instant::now(),
                result: result.clone(),
            });
        }
        result
    }

    fn probe_readiness_uncached(&self) -> ServerReadiness {
        let started = Instant::now();
        let core_started = Instant::now();
        // The engine has no central delivery lock to contend on: the core
        // probe is the same bounded group-sync contract served from the
        // query_only read pool.
        let core_failure = {
            let probe_group = GroupId::new(b"finitechat-readiness-v1".to_vec());
            match self.sql_delivery.sync_group(&probe_group, 0, 1) {
                Ok(_) => None,
                Err(error) => {
                    eprintln!("finitechat-server: readiness delivery-core sync failed: {error:?}");
                    Some("delivery_core_sync_failed")
                }
            }
        };
        let delivery_core = ReadinessCheckResult {
            elapsed: core_started.elapsed(),
            failure: core_failure,
        };

        let store_started = Instant::now();
        // Prove a committed write through the same SQLite file user writes
        // take: a server_meta stamp written and read back in one
        // transaction. On a durable store the probe uses its OWN connection
        // with a bounded busy timeout, so an externally held write lock
        // fails the probe within the component budget instead of queueing
        // the engine's writer behind a five-second busy wait (and user
        // writes never wait behind the probe).
        let checked_at_ms = metadata::unix_now_ms();
        let written = match &self.sqlite_path {
            Some(path) => (|| -> Result<String, DurableStoreError> {
                let mut conn = rusqlite::Connection::open(path)?;
                conn.busy_timeout(READINESS_COMPONENT_BUDGET)?;
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                tx.execute(
                    "INSERT INTO server_meta (key, value) VALUES ('readiness_probe_ms', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    rusqlite::params![checked_at_ms.to_string()],
                )?;
                let observed: String = tx.query_row(
                    "SELECT value FROM server_meta WHERE key = 'readiness_probe_ms'",
                    [],
                    |row| row.get(0),
                )?;
                tx.commit()?;
                Ok(observed)
            })(),
            None => self
                .sql_delivery
                .store()
                .write(|tx| {
                    tx.execute(
                        "INSERT INTO server_meta (key, value) VALUES ('readiness_probe_ms', ?1)
                         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        rusqlite::params![checked_at_ms.to_string()],
                    )?;
                    let observed: String = tx.query_row(
                        "SELECT value FROM server_meta WHERE key = 'readiness_probe_ms'",
                        [],
                        |row| row.get(0),
                    )?;
                    Ok(observed)
                })
                .map_err(|error| match error {
                    crate::store::StoreWriteError::Store(error) => error,
                    crate::store::StoreWriteError::Domain(error) => {
                        DurableStoreError::Replay(error)
                    }
                }),
        };
        let store_failure = match written {
            Ok(observed) if observed == checked_at_ms.to_string() => None,
            Ok(_) => {
                eprintln!("finitechat-server: readiness store read-back mismatch");
                Some("read_back_mismatch")
            }
            Err(error) => {
                eprintln!("finitechat-server: readiness store commit failed: {error:?}");
                Some("commit_failed")
            }
        };
        let durable_store = ReadinessCheckResult {
            elapsed: store_started.elapsed(),
            failure: store_failure,
        };

        let elapsed = started.elapsed();
        ServerReadiness {
            delivery_core,
            durable_store,
            elapsed,
            budget_exceeded: elapsed > Duration::from_millis(READINESS_BUDGET_MILLIS),
        }
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

        self.sql_delivery
            .store()
            .write(|tx| {
                Ok(metadata::insert_blob_object(
                    tx,
                    &sha256,
                    content_type,
                    bytes,
                )?)
            })
            .map_err(sql_write_error)?;
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
        let bytes = self
            .sql_delivery
            .store()
            .read(|conn| Ok(metadata::load_blob_payload(conn, sha256)?))
            .map_err(sql_write_error)?;
        let Some(bytes) = bytes else {
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
        Ok(BlobObject {
            content_type: meta.content_type,
            bytes,
        })
    }

    /// Raw delivery-contract publish, also used by the shared delivery
    /// conformance suite against this durable server.
    ///
    /// Plan the publish read-only, then append it inside one
    /// `BEGIN IMMEDIATE` transaction under the delivery ordering lock.
    /// Digest-exact duplicate replays return the original receipt without
    /// appending; the idempotency row lands on the shared table inside the
    /// same transaction as the entry it describes, so a crash in between
    /// replays as a digest duplicate with the same seq.
    pub fn publish_message(
        &self,
        request: PublishMessageRequest,
    ) -> Result<HttpPublishReceipt, ServerHttpError> {
        self.validate_raw_commit_import(&request)?;
        let sql = &self.sql_delivery;
        let Some(idempotency_key) = request.idempotency_key.clone() else {
            let _ordering = self
                .delivery_ordering
                .lock()
                .expect("HTTP delivery ordering mutex");
            let plans = sql
                .plan_batch(std::slice::from_ref(&request))
                .map_err(sql_write_error)?;
            let receipt = plans[0].receipt.clone();
            sql.store()
                .write(|tx| SqlDelivery::append_plan_in_tx(tx, &plans))
                .map_err(sql_write_error)?;
            drop(_ordering);
            self.note_op_for_snapshot();
            return Ok(receipt);
        };

        if idempotency_key.is_empty() {
            return Err(ServerHttpError::InvalidIdempotencyKey);
        }
        let fingerprint = PublishMessageFingerprint::from_request(&request);
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

        let _ordering = self
            .delivery_ordering
            .lock()
            .expect("HTTP delivery ordering mutex");
        let plans = sql
            .plan_batch(std::slice::from_ref(&request))
            .map_err(sql_write_error)?;
        let fresh = plans[0].fresh;
        let receipt = plans[0].receipt.clone();
        let record = PublishIdempotencyRecord {
            fingerprint,
            receipt: receipt.clone(),
        };
        sql.store()
            .write(|tx| {
                SqlDelivery::append_plan_in_tx(tx, &plans)?;
                metadata::insert_publish_idempotency_in_transaction(tx, &idempotency_key, &record)
                    .map_err(crate::store::StoreTxError::Store)?;
                Ok(())
            })
            .map_err(sql_write_error)?;
        idempotency.insert(idempotency_key, record);
        drop(idempotency);
        drop(_ordering);
        if fresh {
            self.note_op_for_snapshot();
        }
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

    /// Persist the durable legs of a KeyPackage inventory mutation: the
    /// changed inventory rows (current state) plus, when the caller produced
    /// a claim idempotency record, its row — one transaction.
    fn persist_key_package_mutation(
        &self,
        idempotency: Option<(&str, &KeyPackageClaimIdempotencyRecord)>,
        changed: &[KeyPackageInventoryRecord],
    ) -> Result<(), ServerHttpError> {
        self.sql_delivery
            .store()
            .write(|tx| {
                for record in changed {
                    metadata::upsert_key_package_inventory_in_transaction(tx, record)
                        .map_err(crate::store::StoreTxError::Store)?;
                    metadata::upsert_key_package_payload(tx, record)
                        .map_err(crate::store::StoreTxError::Store)?;
                }
                if let Some((key, record)) = idempotency {
                    metadata::insert_key_package_claim_idempotency_in_transaction(tx, key, record)
                        .map_err(crate::store::StoreTxError::Store)?;
                }
                Ok(())
            })
            .map_err(sql_write_error)
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
        self.persist_key_package_mutation(None, &changed)?;
        *inventory = candidate;
        drop(inventory);
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
        self.persist_key_package_mutation(None, &changed)?;
        *inventory = candidate;
        drop(inventory);
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
        self.persist_key_package_mutation(None, &changed)?;
        *inventory = candidate;
        drop(inventory);
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
            self.persist_key_package_mutation(None, &changed)?;
            *inventory = candidate;
            drop(inventory);
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
        let record = KeyPackageClaimIdempotencyRecord {
            fingerprint,
            response: claims.clone(),
        };
        self.persist_key_package_mutation(Some((&idempotency_key, &record)), &changed)?;
        *inventory = candidate;
        idempotency.insert(idempotency_key, record);
        drop(inventory);
        drop(idempotency);
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
        // The inventory row IS the durable state.
        self.persist_key_package_mutation(None, std::slice::from_ref(&changed))?;
        *inventory = candidate;
        drop(inventory);
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
            // The revoked-device set is current state in its own table.
            self.sql_delivery
                .store()
                .write(|tx| {
                    tx.execute(
                        "INSERT OR IGNORE INTO revoked_devices (device_key) VALUES (?1)",
                        rusqlite::params![&device_key],
                    )?;
                    Ok(())
                })
                .map_err(sql_write_error)?;
            revoked_devices.insert(device_key.clone());
            drop(revoked_devices);
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
        self.sql_delivery
            .store()
            .write(|tx| Ok(metadata::upsert_nostr_profile(tx, &record)?))
            .map_err(sql_write_error)?;
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

        self.sql_delivery
            .store()
            .write(|tx| Ok(metadata::upsert_pairing_session(tx, &record)?))
            .map_err(sql_write_error)?;
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
        if let Err(error) = self
            .sql_delivery
            .store()
            .write(|tx| Ok(metadata::upsert_pairing_session(tx, &record)?))
        {
            session.source_public_key = None;
            session.events.clear();
            session.state = HttpPairingSessionState::Created;
            return Err(sql_write_error(error));
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
        if let Err(error) = self
            .sql_delivery
            .store()
            .write(|tx| Ok(metadata::upsert_pairing_session(tx, &record)?))
        {
            session.events.truncate(1);
            session.state = HttpPairingSessionState::OfferPublished;
            return Err(sql_write_error(error));
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
        if let Err(error) = self
            .sql_delivery
            .store()
            .write(|tx| Ok(metadata::upsert_pairing_session(tx, &record)?))
        {
            session.events.truncate(3);
            session.state = HttpPairingSessionState::ResponsePublished;
            return Err(sql_write_error(error));
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
        if let Err(error) = self
            .sql_delivery
            .store()
            .write(|tx| Ok(metadata::upsert_pairing_session(tx, &record)?))
        {
            session.state = prior;
            return Err(sql_write_error(error));
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
            let next_seq = self
                .sql_delivery
                .sync_inbox(&watch.recipient, watch.after_seq, 1)
                .ok()
                .and_then(|page| page.entries.first().map(|entry| entry.seq));
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
            .range((room_id.to_owned(), None)..)
            .take_while(|((bucket_room_id, _), _)| bucket_room_id == room_id)
            .flat_map(|(_, routes)| routes.values())
            .flat_map(|records| records.iter())
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
        let record = AccountRoomDirectoryRecord {
            account_id: request.account_id,
            room_id: request.room_id,
            record: value,
        };
        self.upsert_account_room_row(&record)?;
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
                let record = AccountRoomDirectoryRecord {
                    account_id: account_id.clone(),
                    room_id: request.room_id.clone(),
                    record: value,
                };
                self.upsert_account_room_row(&record)?;
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
        // Update the live map FIRST so the checkpoint persist below captures
        // the NeedsRepair status (the projection is primary state here).
        self.room_memberships
            .lock()
            .expect("HTTP room-membership mutex")
            .insert(request.room_id.clone(), projection);
        for record in &account_records {
            self.upsert_account_room_row(record)?;
        }
        self.normalized_checkpoint_rooms()?;

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

        // A bootstrapped room has no delivery entries yet, so its
        // projection is primary state: checkpoint synchronously.
        self.normalized_checkpoint_rooms()?;
        Ok(())
    }

    fn observed_room_head(
        &self,
        room_id: &str,
        mls_group_id: &str,
    ) -> Result<ObservedRoomHead, ServerHttpError> {
        let group_id = group_id_for_room(room_id);
        let sync_page = |after_seq: u64| -> Result<HttpSyncPage, ServerHttpError> {
            self.sql_delivery
                .sync_group(&group_id, after_seq, MAX_HTTP_SYNC_PAGE_ENTRIES)
                .map_err(sql_write_error)
        };
        let mut current_epoch = 0;
        let mut last_seq = 0;
        let mut after_seq = 0;
        let mut raw_commit_without_projection = false;
        loop {
            let page = sync_page(after_seq)?;
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

        self.sql_delivery
            .store()
            .write(|tx| {
                Ok(crate::store::room_state::apply_directory_mutation(
                    tx, &mutation,
                )?)
            })
            .map_err(sql_write_error)?;
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
        apply_room_membership_delta(
            &mut rooms,
            room_id,
            mls_group_id,
            sender,
            expected_epoch,
            membership_delta,
            accepted_seq,
        )?;
        drop(rooms);

        // Projection durability is the checkpoint; this repair path
        // (idempotent commit replay whose projection lagged) refreshes it
        // synchronously.
        self.normalized_checkpoint_rooms()?;
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

        self.submit_commit_fresh(request, message_id, commit_publish)
    }

    /// Fresh typed commit: the delivery appends, the commit-derived
    /// directory rows, the KeyPackage inventory consumption, and the publish
    /// idempotency rows all land in ONE `BEGIN IMMEDIATE` transaction on the
    /// normalized store, with candidate maps swapped only after the durable
    /// write commits. The room projection itself is derived state: it lands
    /// in RAM and in the room-state checkpoint on the snapshot cadence
    /// (boot re-derives it from the delivery entries committed here).
    fn submit_commit_fresh(
        &self,
        request: SubmitCommitRequest,
        message_id: String,
        commit_publish: PublishMessageRequest,
    ) -> Result<CommitAccepted, ServerHttpError> {
        let sql = &self.sql_delivery;
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

        // Idempotency half of the legacy check phase, against the same map.
        for publish in std::iter::once(&commit_publish) {
            let key = publish.idempotency_key.as_deref().unwrap_or_default();
            if let Some(record) = publish_idempotency.get(key) {
                let fingerprint = PublishMessageFingerprint::from_request(publish);
                if record.fingerprint != fingerprint {
                    return Err(ServerHttpError::IdempotencyConflict {
                        idempotency_key: key.to_owned(),
                    });
                }
            }
        }

        let _ordering = self
            .delivery_ordering
            .lock()
            .expect("HTTP delivery ordering mutex");
        let commit_plans = sql
            .plan_batch(std::slice::from_ref(&commit_publish))
            .map_err(sql_write_error)?;
        let receipt = commit_plans[0].receipt.clone();

        let mut candidate_account_rooms = account_rooms.clone();
        let mut candidate_room_memberships = room_memberships.clone();
        let mut candidate_key_package_inventory = key_package_inventory.clone();
        let account_room_mutation = apply_account_room_membership_delta(
            &mut candidate_account_rooms,
            &request.room_id,
            &request.envelope.mls_group_id,
            request.membership_delta.post_commit_epoch,
            &request.membership_delta,
            receipt.seq,
        )?;
        // The projection delta must apply cleanly against the live map; the
        // result lives in RAM (candidate swap below) and rides the checkpoint
        // cadence rather than a per-commit durable row.
        apply_room_membership_delta(
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
        let mut welcome_publishes = Vec::with_capacity(welcomes.len());
        for welcome in &welcomes {
            welcome_publishes.push(welcome_publish_request(welcome)?);
        }
        let welcome_plans = if welcome_publishes.is_empty() {
            Vec::new()
        } else {
            sql.plan_batch(&welcome_publishes)
                .map_err(sql_write_error)?
        };

        // One transaction: delivery entries (+ commit epochs), directory
        // rows derived from the delta, inventory consumption, and the
        // publish idempotency rows. A crash anywhere before the commit
        // leaves nothing behind; the client retries.
        sql.store()
            .write(|tx| {
                SqlDelivery::append_plan_in_tx(tx, &commit_plans)?;
                SqlDelivery::append_plan_in_tx(tx, &welcome_plans)?;
                crate::store::room_state::apply_directory_mutation(tx, &account_room_mutation)?;
                for record in &key_package_inventory_mutation {
                    metadata::upsert_key_package_inventory_in_transaction(tx, record)
                        .map_err(crate::store::StoreTxError::Store)?;
                }
                for (publish, plan) in welcome_publishes.iter().zip(&welcome_plans) {
                    if let Some(key) = publish.idempotency_key.as_deref() {
                        metadata::insert_publish_idempotency_in_transaction(
                            tx,
                            key,
                            &PublishIdempotencyRecord {
                                fingerprint: PublishMessageFingerprint::from_request(publish),
                                receipt: plan.receipt.clone(),
                            },
                        )
                        .map_err(crate::store::StoreTxError::Store)?;
                    }
                }
                if let Some(key) = commit_publish.idempotency_key.as_deref() {
                    metadata::insert_publish_idempotency_in_transaction(
                        tx,
                        key,
                        &PublishIdempotencyRecord {
                            fingerprint: PublishMessageFingerprint::from_request(&commit_publish),
                            receipt: receipt.clone(),
                        },
                    )
                    .map_err(crate::store::StoreTxError::Store)?;
                }
                Ok(())
            })
            .map_err(sql_write_error)?;

        // Apply phase: infallible given the transaction above committed.
        *account_rooms = candidate_account_rooms;
        *room_memberships = candidate_room_memberships;
        *key_package_inventory = candidate_key_package_inventory;
        for (publish, plan) in welcome_publishes.iter().zip(&welcome_plans) {
            if let Some(key) = publish.idempotency_key.clone() {
                publish_idempotency.insert(
                    key,
                    PublishIdempotencyRecord {
                        fingerprint: PublishMessageFingerprint::from_request(publish),
                        receipt: plan.receipt.clone(),
                    },
                );
            }
        }
        if let Some(key) = commit_publish.idempotency_key.clone() {
            publish_idempotency.insert(
                key,
                PublishIdempotencyRecord {
                    fingerprint: PublishMessageFingerprint::from_request(&commit_publish),
                    receipt: receipt.clone(),
                },
            );
        }
        drop(publish_idempotency);
        drop(account_rooms);
        drop(room_memberships);
        drop(key_package_inventory);
        drop(_ordering);
        if commit_plans[0].fresh {
            self.note_op_for_snapshot();
        }

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
        self.append_application_event_fresh(request, message_id, event_publish)
    }

    /// Fresh typed application event: plan the publish, then land the
    /// delivery entry, the effect row, and the idempotency row in ONE
    /// transaction. The projection head advance is derived state and rides
    /// the checkpoint cadence.
    fn append_application_event_fresh(
        &self,
        request: AppendApplicationEventRequest,
        message_id: String,
        event_publish: PublishMessageRequest,
    ) -> Result<EventAccepted, ServerHttpError> {
        let sql = &self.sql_delivery;
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
        // Idempotency check against the same map the legacy path used: the
        // COMPOSED publish key ("event:{room}:{key}"), which is also the
        // durable row's key — a folded database carries rows in exactly
        // this shape. An exact replay must still agree with the stored
        // delivery effect (same key, different policy is a conflict, not
        // a replay).
        let idempotency_key = event_publish
            .idempotency_key
            .clone()
            .ok_or(ServerHttpError::InvalidIdempotencyKey)?;
        if idempotency_key.is_empty() {
            return Err(ServerHttpError::InvalidIdempotencyKey);
        }
        if let Some(record) = idempotency.get(&idempotency_key) {
            let fingerprint = PublishMessageFingerprint::from_request(&event_publish);
            if record.fingerprint == fingerprint {
                let effect = HttpApplicationDeliveryEffect {
                    room_id: request.event.room_id.clone(),
                    seq: record.receipt.seq,
                    message_id: message_id.clone(),
                    sender: request.event.sender.clone(),
                    delivery_policy: request.delivery_policy,
                };
                check_application_delivery_effect(&application_effects, effect, &idempotency_key)?;
                return Ok(EventAccepted {
                    seq: record.receipt.seq,
                    message_id,
                });
            }
            return Err(ServerHttpError::IdempotencyConflict { idempotency_key });
        }

        let _ordering = self
            .delivery_ordering
            .lock()
            .expect("HTTP delivery ordering mutex");
        let plans = sql
            .plan_batch(std::slice::from_ref(&event_publish))
            .map_err(|error| match error {
                crate::store::StoreWriteError::Domain(HttpServerError::ConflictingMessageId {
                    ..
                }) => ServerHttpError::DuplicateMessageId {
                    message_id: MessageId::new(message_id.as_bytes().to_vec()),
                },
                other => sql_write_error(other),
            })?;
        let receipt = plans[0].receipt.clone();
        if !plans[0].fresh {
            // Typed events never digest-replay: a duplicate message id is a
            // client bug, exactly like the legacy check-typed path.
            return Err(ServerHttpError::DuplicateMessageId {
                message_id: MessageId::new(message_id.as_bytes().to_vec()),
            });
        }

        let room_membership_projection =
            check_room_event_acceptance(&room_memberships, &request.event.room_id, receipt.seq);
        let effect = HttpApplicationDeliveryEffect {
            room_id: request.event.room_id.clone(),
            seq: receipt.seq,
            message_id: message_id.clone(),
            sender: request.event.sender,
            delivery_policy: request.delivery_policy,
        };
        let effect_mutation =
            check_application_delivery_effect(&application_effects, effect, &idempotency_key)?;
        let idempotency_record = PublishIdempotencyRecord {
            fingerprint: PublishMessageFingerprint::from_request(&event_publish),
            receipt: receipt.clone(),
        };
        sql.store()
            .write(|tx| {
                SqlDelivery::append_plan_in_tx(tx, &plans)?;
                if let Some(effect) = &effect_mutation {
                    metadata::upsert_application_effect_in_transaction(tx, effect)
                        .map_err(crate::store::StoreTxError::Store)?;
                }
                metadata::insert_publish_idempotency_in_transaction(
                    tx,
                    &idempotency_key,
                    &idempotency_record,
                )
                .map_err(crate::store::StoreTxError::Store)?;
                Ok(())
            })
            .map_err(sql_write_error)?;

        idempotency.insert(idempotency_key, idempotency_record);
        if let Some(projection) = room_membership_projection {
            room_memberships.insert(request.event.room_id.clone(), projection);
        }
        if let Some(effect) = effect_mutation {
            application_effects.insert(effect.message_id.clone(), effect);
        }
        drop(idempotency);
        drop(room_memberships);
        drop(application_effects);
        drop(_ordering);
        self.note_op_for_snapshot();
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
        let mut unread = 0usize;
        let mut command_inbox = 0usize;
        for effect in effects.values() {
            if effect.delivery_policy.creates_unread() {
                unread += 1;
            }
            if effect.delivery_policy.creates_command_inbox_work() {
                command_inbox += 1;
            }
        }
        Ok(ApplicationEffectCountsResponse {
            unread: usize_to_u32("unread", unread)?,
            command_inbox: usize_to_u32("command_inbox", command_inbox)?,
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
        let bucket_key = (record.room_id.clone(), record.conversation_id.clone());
        let records = activity
            .entry(bucket_key)
            .or_default()
            .entry(route_key.clone())
            .or_default();
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
        let bucket_key = (request.room_id.clone(), request.conversation_id.clone());
        if let Some(routes) = activity.get_mut(&bucket_key) {
            for route_records in routes.values_mut() {
                route_records.retain(|record| record.expires_at_ms > request.now_ms);
                records.extend(route_records.iter().cloned());
            }
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

        let sync_inbox_page = |after_seq: u64| -> Result<HttpSyncPage, ServerHttpError> {
            self.sql_delivery
                .sync_inbox(&request.recipient, after_seq, MAX_HTTP_SYNC_PAGE_ENTRIES)
                .map_err(sql_write_error)
        };
        let mut claims = self
            .welcome_claims
            .lock()
            .expect("HTTP welcome claims mutex");
        let mut claimed = Vec::new();
        let mut after_seq = 0;
        loop {
            let page = sync_inbox_page(after_seq)?;
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
                self.sql_delivery
                    .store()
                    .write(|tx| Ok(metadata::upsert_welcome_claim(tx, &record)?))
                    .map_err(sql_write_error)?;
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
                self.sql_delivery
                    .store()
                    .write(|tx| Ok(metadata::upsert_welcome_claim(tx, record)?))
                    .map_err(sql_write_error)?;
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

        let record = AccountRoomDirectoryRecord {
            account_id,
            room_id: welcome.room_id,
            record: value,
        };
        self.upsert_account_room_row(&record)?;
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
        // One transaction moves the interval closes (checkpoint) and the
        // directory delete together; a crash leaves the leave
        // unconfirmed and the client retries idempotently.
        {
            let rooms = self
                .room_memberships
                .lock()
                .expect("HTTP room-membership mutex")
                .clone();
            self.sql_delivery
                .store()
                .write(|tx| {
                    tx.execute(
                        "DELETE FROM account_room_directory WHERE account_id = ?1 AND room_id = ?2",
                        rusqlite::params![&account_id, &request.room_id],
                    )?;
                    crate::store::room_state::save_checkpoint(
                        tx,
                        &crate::store::room_state::RoomStateCheckpoint { rooms },
                    )?;
                    Ok(())
                })
                .map_err(sql_write_error)?;
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

        self.normalized_checkpoint_rooms()?;
        Ok(UpdateRoomAdminsResponse {
            admins: updated.admins.iter().cloned().collect(),
        })
    }

    /// Write a fresh room-state checkpoint so the next startup replays at
    /// most each room's delivery-entry tail. Called automatically every
    /// [`SNAPSHOT_INTERVAL_OPS`] accepted operations and available for
    /// graceful shutdowns. Delivery state itself is already durable in the
    /// normalized tables — there is no op log and nothing else to snapshot.
    pub fn snapshot_now(&self) -> Result<(), ServerHttpError> {
        self.normalized_checkpoint_rooms()
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
        // triggering request neither waits for it nor fails with it. The
        // guard clears the in-flight flag on scope exit — including a panic,
        // which would otherwise leave it stuck true and silently freeze
        // snapshots for the rest of the process lifetime.
        let state = self.clone();
        let guard = SnapshotInFlightGuard(self.snapshot_in_flight.clone());
        std::thread::spawn(move || {
            let _guard = guard;
            if let Err(error) = state.snapshot_now() {
                eprintln!(
                    "finitechat-server: state snapshot persist failed; the op log keeps growing and the next interval retries: {error:?}"
                );
            }
        });
    }

    pub fn sync_inbox(
        &self,
        recipient: &MemberId,
        after_seq: u64,
        limit: usize,
    ) -> Result<HttpSyncPage, ServerHttpError> {
        self.sql_delivery
            .sync_inbox(recipient, after_seq, limit)
            .map_err(sql_write_error)
    }

    pub fn sync_group(&self, request: GroupSyncRequest) -> Result<HttpSyncPage, ServerHttpError> {
        if request.limit == 0 || request.limit > MAX_HTTP_SYNC_PAGE_ENTRIES {
            return Err(ServerHttpError::InvalidGroupSyncLimit {
                actual: request.limit,
                max: MAX_HTTP_SYNC_PAGE_ENTRIES,
            });
        }
        let page = self
            .sql_delivery
            .sync_group(&request.group_id, request.after_seq, request.limit)
            .map_err(sql_write_error)?;

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
        drop(rooms);

        // Re-derivable at boot from the claims table plus the delivery
        // entries, but cheap to keep current: checkpoint on the cadence
        // rather than synchronously here.
        self.note_op_for_snapshot();
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

/// Widen a normalized-store failure for a live request path: contract
/// rejections keep their HTTP mapping, storage failures become 500s.
fn sql_write_error(error: crate::store::StoreWriteError) -> ServerHttpError {
    match error {
        crate::store::StoreWriteError::Store(error) => error.into(),
        crate::store::StoreWriteError::Domain(error) => error.into(),
    }
}

/// Boot divergence between the room-state checkpoint and the delivery
/// entries: fail closed (the error names recovery), never absorb.
fn divergence(details: String) -> crate::store::StoreTxError {
    crate::store::StoreTxError::Store(DurableStoreError::CheckpointDivergence { details })
}

/// Widen a normalized-store failure for the boot path. A `Domain` rejection
/// during boot is a delivery-contract replay failure, which `DurableStoreError`
/// already names.
fn normalized_store_error(error: crate::store::StoreWriteError) -> DurableStoreError {
    match error {
        crate::store::StoreWriteError::Store(error) => error,
        crate::store::StoreWriteError::Domain(error) => DurableStoreError::Replay(error),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AccountRoomDirectoryMutation {
    pub(crate) deletes: Vec<(String, String)>,
    pub(crate) upserts: Vec<AccountRoomDirectoryRecord>,
}

/// Releases the snapshot in-flight flag when the background snapshot thread
/// exits, even on panic: a flag stuck at `true` would stop every future
/// snapshot attempt with no further error to observe — exactly the silent
/// durable-state freeze to guard against.
struct SnapshotInFlightGuard(Arc<AtomicBool>);

impl Drop for SnapshotInFlightGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
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

pub(crate) fn apply_account_room_membership_delta(
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

/// Boot-replay half of the Welcome-ack activation: mark the recipient's
/// device active in the account-room directory record (mirroring
/// `activate_account_room_from_welcome`) and return the record to persist,
/// or `None` when nothing changed. Corrupt or absent records are left
/// untouched — the live ack path keeps that behavior.
pub(crate) fn activate_account_room_device_in_directory(
    directory: &mut BTreeMap<String, BTreeMap<String, Value>>,
    recipient: &DeviceRef,
    room_id: &str,
) -> Option<AccountRoomDirectoryRecord> {
    let account_id = recipient.account_id.clone();
    let existing_value = directory
        .get(&account_id)
        .and_then(|rooms| rooms.get(room_id))
        .cloned()?;
    let mut record = account_scoped_account_room_record(&account_id, room_id, &existing_value)
        .ok()
        .flatten()?;
    if !record
        .devices
        .iter()
        .any(|device| device.device == *recipient && !device.active)
    {
        return None;
    }
    for device in &mut record.devices {
        if device.device == *recipient {
            device.active = true;
        }
    }
    let value = serde_json::to_value(&record).ok()?;
    directory
        .entry(account_id.clone())
        .or_default()
        .insert(room_id.to_owned(), value.clone());
    Some(AccountRoomDirectoryRecord {
        account_id,
        room_id: room_id.to_owned(),
        record: value,
    })
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
        if record.owner != publication.owner
            || (!record.key_package.bytes.is_empty()
                && record.key_package != publication.key_package)
        {
            return Err(HttpServerError::ConflictingKeyPackage {
                key_package_id: publication.key_package_id.clone(),
            }
            .into());
        }
        // A record loaded from the shared table carries no payload bytes;
        // the publication is the authority, exactly like the legacy op
        // replay's empty-bytes repair.
        if record.key_package.bytes.is_empty() {
            record.key_package = publication.key_package.clone();
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

#[cfg(test)]
mod rate_limit_tests {
    use super::PublicRouteRateLimiter;
    use std::net::{IpAddr, Ipv4Addr};

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last))
    }

    fn len(limiter: &PublicRouteRateLimiter) -> usize {
        limiter.windows.lock().expect("windows").len()
    }

    #[test]
    fn expired_windows_are_evicted_once_the_map_hits_the_cap() {
        let mut limiter = PublicRouteRateLimiter::new(120, 0);
        limiter.max_entries = 2;

        assert!(limiter.check(ip(1)));
        assert!(limiter.check(ip(2)));
        assert_eq!(len(&limiter), 2);
        // The third distinct IP triggers the sweep; with a zero-length window
        // every entry is stale, so the map shrinks instead of growing.
        assert!(limiter.check(ip(3)));
        assert_eq!(len(&limiter), 1);
    }

    #[test]
    fn live_windows_survive_the_sweep() {
        let mut limiter = PublicRouteRateLimiter::new(120, 60);
        limiter.max_entries = 2;

        assert!(limiter.check(ip(1)));
        assert!(limiter.check(ip(2)));
        // All entries are live, so the sweep evicts nothing and the cap is a
        // soft bound for genuinely concurrent clients.
        assert!(limiter.check(ip(3)));
        assert_eq!(len(&limiter), 3);
    }
}
