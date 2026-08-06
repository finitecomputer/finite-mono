//! Synthetic proofs for the Track B rejected-entry classifier.
//!
//! Every fixture is local: an in-process axum server, fabricated devices,
//! and fabricated store copies. No production contact of any kind.

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use finitechat_client::rejected_entry_diagnostic::{
    CapturedRoomLog, CapturedRoomLogFile, RejectedEntryDiagnostic, RejectedEntryDiagnosticRequest,
    RejectedEntryErrorClass, RejectedEntryKind, RepairDisposition, ReplayOutcome,
    run_rejected_entry_diagnostic,
};
use finitechat_client::{
    FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery, HttpRuntimeTransport,
    RuntimeDelivery, SqliteClientStore, SqliteClientStoreOptions,
};
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::{
    CreateRoomRequest, DurableAppEventKind, LogEntryKind, RoomLogEntry, RoomProtocol,
};
use finitechat_server::{HttpServerState, http_router};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const ALICE_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [41; NOSTR_SECRET_KEY_BYTES];
const HOSTED_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [43; NOSTR_SECRET_KEY_BYTES];
const CHARLIE_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [47; NOSTR_SECRET_KEY_BYTES];
const NOW: u64 = 1_800_000_000;
const INCIDENT_ALIAS: &str = "incident-track-b-synthetic";

fn test_config(
    account_secret_bytes: [u8; NOSTR_SECRET_KEY_BYTES],
    device_id: &str,
) -> FiniteChatDeviceConfig {
    FiniteChatDeviceConfig {
        account_secret_key: NostrSecretKey::from_bytes(account_secret_bytes).unwrap(),
        device_id: device_id.to_string(),
        now_unix_seconds: NOW,
        credential_not_before_unix_seconds: NOW - 60,
        credential_not_after_unix_seconds: NOW + 60,
    }
}

fn sqlite_client_store(
    path: impl AsRef<std::path::Path>,
    config: &FiniteChatDeviceConfig,
) -> SqliteClientStore {
    SqliteClientStore::open(
        path,
        SqliteClientStoreOptions::from_nostr_secret(&config.account_secret_key, &config.device_id)
            .unwrap(),
    )
    .unwrap()
}

#[derive(Debug, PartialEq, Eq)]
enum InProcessHttpTransportError {
    Json(String),
    HttpStatus(StatusCode, String),
    Router(String),
}

impl std::fmt::Display for InProcessHttpTransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "JSON error: {error}"),
            Self::HttpStatus(status, body) => write!(formatter, "HTTP status {status}: {body}"),
            Self::Router(error) => write!(formatter, "router error: {error}"),
        }
    }
}

impl std::error::Error for InProcessHttpTransportError {}

struct InProcessHttpTransport {
    app: Router,
    runtime: tokio::runtime::Runtime,
}

impl InProcessHttpTransport {
    fn from_sqlite_path(path: &std::path::Path) -> Self {
        Self {
            app: http_router(HttpServerState::from_sqlite_path(path).unwrap()),
            runtime: tokio::runtime::Runtime::new().unwrap(),
        }
    }
}

impl HttpRuntimeTransport for InProcessHttpTransport {
    type Error = InProcessHttpTransportError;

    fn post_json<T, R>(&mut self, uri: &str, body: &T) -> Result<R, Self::Error>
    where
        T: Serialize,
        R: DeserializeOwned,
    {
        self.runtime.block_on(async {
            let request = Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(body).map_err(|error| {
                    InProcessHttpTransportError::Json(error.to_string())
                })?))
                .map_err(|error| InProcessHttpTransportError::Router(error.to_string()))?;
            let response = self
                .app
                .clone()
                .oneshot(request)
                .await
                .map_err(|error| InProcessHttpTransportError::Router(error.to_string()))?;
            let status = response.status();
            let bytes = to_bytes(response.into_body(), usize::MAX)
                .await
                .map_err(|error| InProcessHttpTransportError::Router(error.to_string()))?;
            if status != StatusCode::OK {
                return Err(InProcessHttpTransportError::HttpStatus(
                    status,
                    String::from_utf8_lossy(&bytes).into_owned(),
                ));
            }
            serde_json::from_slice(&bytes)
                .map_err(|error| InProcessHttpTransportError::Json(error.to_string()))
        })
    }
}

type TestDelivery = HttpRuntimeDelivery<InProcessHttpTransport>;

fn test_delivery(path: &std::path::Path) -> TestDelivery {
    HttpRuntimeDelivery::new(InProcessHttpTransport::from_sqlite_path(path))
}

fn hex_lower(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(TABLE[(byte >> 4) as usize] as char);
        out.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

struct RoomSetup<'a> {
    room_id: &'a str,
    mls_group_id: &'a str,
    key_package_id: &'a str,
    welcome_id: &'a str,
}

/// Bootstrap a room, add the hosted device as a member, and persist the
/// hosted device's durable state at the join (add-commit) cursor. Returns
/// the join sequence.
fn create_room_with_hosted_member(
    delivery: &mut TestDelivery,
    hosted_store: &mut SqliteClientStore,
    alice: &mut FiniteChatDevice,
    hosted: &mut FiniteChatDevice,
    setup: RoomSetup<'_>,
) -> u64 {
    let RoomSetup {
        room_id,
        mls_group_id,
        key_package_id,
        welcome_id,
    } = setup;
    alice.create_group_state(room_id, mls_group_id).unwrap();
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: room_id.to_string(),
            mls_group_id: mls_group_id.to_string(),
            creator: alice.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    delivery
        .upload_key_package(hosted.upload_key_package_request(key_package_id).unwrap())
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(hosted.device_ref())
        .unwrap()
        .expect("hosted key package");
    let prepared = alice
        .prepare_add_member_commit(room_id, &claimed_key_package, welcome_id, "add-hosted")
        .unwrap();
    let accepted = delivery.submit_commit(prepared.request).unwrap();
    let alice_page = delivery
        .sync_events(room_id, alice.device_ref(), 0)
        .unwrap();
    alice
        .merge_pending_commit_from_log(room_id, &alice_page.entries, &prepared.message_id)
        .unwrap();
    let claimed_welcomes = delivery.claim_welcomes(hosted.device_ref()).unwrap();
    let welcome = claimed_welcomes
        .into_iter()
        .find(|welcome| welcome.welcome_id == welcome_id)
        .unwrap();
    hosted_store
        .activate_welcome_and_save(
            hosted,
            welcome_id,
            room_id,
            &welcome.welcome_payload,
            &welcome.ratchet_tree_payload,
            accepted.seq,
        )
        .unwrap();
    delivery.ack_welcome(welcome_id).unwrap();
    accepted.seq
}

fn append_alice_message(
    delivery: &mut TestDelivery,
    alice: &mut FiniteChatDevice,
    room_id: &str,
    plaintext: &[u8],
    idempotency_key: &str,
) -> u64 {
    let request = alice
        .create_application_request(room_id, plaintext, idempotency_key)
        .unwrap();
    delivery
        .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap()
        .seq
}

/// Append an application entry whose payload is corrupted after creation
/// into bytes that no longer deserialize as an MLS message; the opaque
/// server stores it as-is (the same corruption shape as the production
/// quarantine regression test). Returns the corrupted payload bytes and
/// the accepted sequence.
fn append_corrupted_alice_message(
    delivery: &mut TestDelivery,
    alice: &mut FiniteChatDevice,
    room_id: &str,
    idempotency_key: &str,
) -> (Vec<u8>, u64) {
    let mut request = alice
        .create_application_request(room_id, b"plaintext never stored", idempotency_key)
        .unwrap();
    let corrupted_payload = vec![0xff; 32];
    request.envelope.payload = corrupted_payload.clone();
    let seq = delivery
        .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap()
        .seq;
    (corrupted_payload, seq)
}

/// Read-only capture of a room's ordered log from the fixture server.
fn capture_room_log(
    delivery: &mut TestDelivery,
    room_id: &str,
    requester: &FiniteChatDevice,
) -> CapturedRoomLog {
    let mut entries: Vec<RoomLogEntry> = Vec::new();
    let mut after_seq = 0;
    loop {
        let page = delivery
            .sync_events(room_id, requester.device_ref(), after_seq)
            .unwrap();
        after_seq = page.next_after_seq;
        let has_more = page.has_more;
        entries.extend(page.entries);
        if !has_more {
            break;
        }
    }
    CapturedRoomLog {
        room_id: room_id.to_string(),
        entries,
    }
}

/// Two-room fixture: a broken room whose log contains one corrupted
/// application ciphertext between two valid ones, and a healthy room with
/// one valid pending message. The hosted store copy sits at both join
/// cursors. Returns everything the proofs need.
struct TwoRoomFixture {
    hosted_config: FiniteChatDeviceConfig,
    hosted_store_path: std::path::PathBuf,
    target: CapturedRoomLog,
    healthy: CapturedRoomLog,
    corrupted_payload: Vec<u8>,
    corrupted_seq: u64,
    hosted_cursor_before: u64,
    healthy_cursor_before: u64,
    plaintext_marker: String,
}

fn two_room_fixture(dir: &std::path::Path) -> TwoRoomFixture {
    let server_db = dir.join("fixture-server.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_fixture_operator");
    let hosted_config = test_config(HOSTED_ACCOUNT_SECRET_BYTES, "hosted_web_device");
    let mut alice = FiniteChatDevice::new(alice_config).unwrap();
    let mut hosted = FiniteChatDevice::new(hosted_config.clone()).unwrap();
    let mut delivery = test_delivery(&server_db);

    // The hosted device is caught up through both join commits; each
    // activation persists exactly that durable state as the "store copy"
    // under diagnosis.
    let hosted_store_path = dir.join("hosted-store.sqlite3");
    let mut hosted_store = sqlite_client_store(&hosted_store_path, &hosted_config);
    let broken_join_seq = create_room_with_hosted_member(
        &mut delivery,
        &mut hosted_store,
        &mut alice,
        &mut hosted,
        RoomSetup {
            room_id: "room_track_b_broken",
            mls_group_id: "mls_track_b_broken",
            key_package_id: "kp_hosted_broken",
            welcome_id: "welcome_hosted_broken",
        },
    );
    let healthy_join_seq = create_room_with_hosted_member(
        &mut delivery,
        &mut hosted_store,
        &mut alice,
        &mut hosted,
        RoomSetup {
            room_id: "room_track_b_healthy",
            mls_group_id: "mls_track_b_healthy",
            key_package_id: "kp_hosted_healthy",
            welcome_id: "welcome_hosted_healthy",
        },
    );
    drop(hosted_store);

    // Broken room: valid, corrupted, valid. The rejected entry is NOT
    // cursor + 1, so only exact attribution can find it.
    let plaintext_marker =
        "SECRET-PLAINTEXT-MARKER https://secret.invalid/SECRET-FILENAME.pdf".to_owned();
    let first_valid_seq = append_alice_message(
        &mut delivery,
        &mut alice,
        "room_track_b_broken",
        plaintext_marker.as_bytes(),
        "broken-valid-before",
    );
    assert_eq!(first_valid_seq, broken_join_seq + 1);
    let (corrupted_payload, corrupted_seq) = append_corrupted_alice_message(
        &mut delivery,
        &mut alice,
        "room_track_b_broken",
        "broken-corrupted-entry",
    );
    assert_eq!(corrupted_seq, broken_join_seq + 2);
    let second_valid_seq = append_alice_message(
        &mut delivery,
        &mut alice,
        "room_track_b_broken",
        b"valid trailing message",
        "broken-valid-after",
    );
    assert_eq!(second_valid_seq, broken_join_seq + 3);

    // Healthy room: one valid message the hosted device has not seen.
    let healthy_message_seq = append_alice_message(
        &mut delivery,
        &mut alice,
        "room_track_b_healthy",
        b"healthy room progress",
        "healthy-valid",
    );
    assert_eq!(healthy_message_seq, healthy_join_seq + 1);

    let target = capture_room_log(&mut delivery, "room_track_b_broken", &alice);
    let healthy = capture_room_log(&mut delivery, "room_track_b_healthy", &alice);

    TwoRoomFixture {
        hosted_config,
        hosted_store_path,
        target,
        healthy,
        corrupted_payload,
        corrupted_seq,
        hosted_cursor_before: broken_join_seq,
        healthy_cursor_before: healthy_join_seq,
        plaintext_marker,
    }
}

fn diagnostic_request(
    fixture: &TwoRoomFixture,
    work_dir: &std::path::Path,
) -> RejectedEntryDiagnosticRequest {
    RejectedEntryDiagnosticRequest {
        source_db_path: fixture.hosted_store_path.clone(),
        work_dir: work_dir.to_path_buf(),
        config: fixture.hosted_config.clone(),
        incident_alias: INCIDENT_ALIAS.to_owned(),
        target: fixture.target.clone(),
        other_rooms: vec![fixture.healthy.clone()],
    }
}

#[test]
fn corrupted_application_ciphertext_reports_exact_entry_binding_and_class() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = two_room_fixture(dir.path());

    let record =
        run_rejected_entry_diagnostic(&diagnostic_request(&fixture, &dir.path().join("work")))
            .unwrap();

    assert_eq!(record.schema_version, 1);
    assert_eq!(record.incident_alias, INCIDENT_ALIAS);
    assert_eq!(record.cursor_before, fixture.hosted_cursor_before);
    assert_eq!(record.replay_outcome, ReplayOutcome::Rejected);
    let rejected = record.rejected.expect("a rejected entry is attributed");
    assert_eq!(
        rejected.seq, fixture.corrupted_seq,
        "the exact rejected sequence, not cursor + 1"
    );
    assert_ne!(rejected.seq, record.cursor_before + 1);
    assert_eq!(rejected.kind, RejectedEntryKind::Application);
    assert_eq!(
        rejected.sha256,
        sha256_hex(&fixture.corrupted_payload),
        "SHA-256 of the exact opaque ciphertext bytes"
    );
    assert_eq!(
        record.error_class,
        Some(RejectedEntryErrorClass::ProtocolEnvelopeParsing),
        "bytes that no longer deserialize as an MLS message"
    );
    assert_eq!(
        record.repair_disposition,
        Some(RepairDisposition::ClassificationOnly)
    );
    assert_eq!(record.cursor_after, fixture.hosted_cursor_before);
}

#[test]
fn failed_replay_leaves_durable_cursor_and_mls_state_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = two_room_fixture(dir.path());
    let (baseline_cursor, baseline_epoch) = {
        let store = sqlite_client_store(&fixture.hosted_store_path, &fixture.hosted_config);
        let device = store.load_device(fixture.hosted_config.clone()).unwrap();
        (
            device.last_applied_seq(&fixture.target.room_id).unwrap(),
            device.group_epoch(&fixture.target.room_id).unwrap(),
        )
    };

    let record =
        run_rejected_entry_diagnostic(&diagnostic_request(&fixture, &dir.path().join("work")))
            .unwrap();

    assert_eq!(record.replay_outcome, ReplayOutcome::Rejected);
    assert!(
        !record.device_state_candidate_persisted,
        "fail closed: the target-room replay persists no device-state candidate"
    );
    assert_eq!(record.cursor_after, record.cursor_before);

    // Independently verify the replay copy's durable target-room state:
    // cursor and MLS group epoch are exactly what they were before the
    // failed replay. (Other rooms may legitimately advance on the copy;
    // that is proven separately.)
    let replay_copy = dir
        .path()
        .join("work")
        .join("rejected-entry-replay-store.sqlite3");
    let store = sqlite_client_store(&replay_copy, &fixture.hosted_config);
    let reloaded = store.load_device(fixture.hosted_config.clone()).unwrap();
    assert_eq!(
        reloaded.last_applied_seq(&fixture.target.room_id).unwrap(),
        baseline_cursor,
        "durable target-room cursor is unchanged"
    );
    assert_eq!(
        reloaded.group_epoch(&fixture.target.room_id).unwrap(),
        baseline_epoch,
        "durable target-room MLS epoch is unchanged"
    );
    assert_eq!(baseline_cursor, fixture.hosted_cursor_before);
}

/// One-room fixture with a membership change the hosted device has not
/// processed: join commit, two epoch-1 application messages, an add-member
/// commit, and one epoch-2 application message. The hosted store copy sits
/// at the join cursor.
struct MembershipFixture {
    hosted_config: FiniteChatDeviceConfig,
    hosted_store_path: std::path::PathBuf,
    target: CapturedRoomLog,
    app_b_seq: u64,
    commit_seq: u64,
}

fn membership_room_fixture(dir: &std::path::Path) -> MembershipFixture {
    let server_db = dir.join("fixture-server.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_commit_operator");
    let hosted_config = test_config(HOSTED_ACCOUNT_SECRET_BYTES, "hosted_web_device");
    let charlie_config = test_config(CHARLIE_ACCOUNT_SECRET_BYTES, "charlie_fixture_device");
    let mut alice = FiniteChatDevice::new(alice_config).unwrap();
    let mut hosted = FiniteChatDevice::new(hosted_config.clone()).unwrap();
    let charlie = FiniteChatDevice::new(charlie_config).unwrap();
    let mut delivery = test_delivery(&server_db);

    let hosted_store_path = dir.join("hosted-store.sqlite3");
    let mut hosted_store = sqlite_client_store(&hosted_store_path, &hosted_config);
    let join_seq = create_room_with_hosted_member(
        &mut delivery,
        &mut hosted_store,
        &mut alice,
        &mut hosted,
        RoomSetup {
            room_id: "room_track_b_commit",
            mls_group_id: "mls_track_b_commit",
            key_package_id: "kp_hosted_commit",
            welcome_id: "welcome_hosted_commit",
        },
    );
    drop(hosted_store);

    append_alice_message(
        &mut delivery,
        &mut alice,
        "room_track_b_commit",
        b"application message before the commit",
        "commit-room-app-a",
    );
    let app_b_seq = append_alice_message(
        &mut delivery,
        &mut alice,
        "room_track_b_commit",
        b"second application message before the commit",
        "commit-room-app-b",
    );

    // A later membership commit the hosted device has not processed.
    delivery
        .upload_key_package(
            charlie
                .upload_key_package_request("kp_charlie_commit")
                .unwrap(),
        )
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(charlie.device_ref())
        .unwrap()
        .expect("charlie key package");
    let prepared = alice
        .prepare_add_member_commit(
            "room_track_b_commit",
            &claimed_key_package,
            "welcome_charlie_commit",
            "add-charlie",
        )
        .unwrap();
    let commit_seq = delivery.submit_commit(prepared.request).unwrap().seq;
    assert!(commit_seq > join_seq);
    let alice_page = delivery
        .sync_events("room_track_b_commit", alice.device_ref(), join_seq)
        .unwrap();
    alice
        .merge_pending_commit_from_log(
            "room_track_b_commit",
            &alice_page.entries,
            &prepared.message_id,
        )
        .unwrap();
    // A valid application message behind the commit proves the classifier
    // never routes around a failed Commit into an application-gap path.
    append_alice_message(
        &mut delivery,
        &mut alice,
        "room_track_b_commit",
        b"application message after the commit",
        "commit-room-app-after",
    );

    let target = capture_room_log(&mut delivery, "room_track_b_commit", &alice);
    MembershipFixture {
        hosted_config,
        hosted_store_path,
        target,
        app_b_seq,
        commit_seq,
    }
}

fn membership_request(
    fixture: &MembershipFixture,
    work_dir: &std::path::Path,
    target: CapturedRoomLog,
) -> RejectedEntryDiagnosticRequest {
    RejectedEntryDiagnosticRequest {
        source_db_path: fixture.hosted_store_path.clone(),
        work_dir: work_dir.to_path_buf(),
        config: fixture.hosted_config.clone(),
        incident_alias: INCIDENT_ALIAS.to_owned(),
        target,
        other_rooms: Vec::new(),
    }
}

#[test]
fn malformed_commit_is_non_skippable_and_enters_no_gap_path() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = membership_room_fixture(dir.path());

    let mut target = fixture.target.clone();
    let commit_entry = target
        .entries
        .iter_mut()
        .find(|entry| entry.seq == fixture.commit_seq)
        .expect("capture contains the membership commit");
    assert_eq!(commit_entry.kind, LogEntryKind::Commit);
    // The capture is corrupted in transit/at rest into bytes that no
    // longer deserialize: an unprocessable Commit. Recompute the derived
    // message id so the capture stays self-consistent (the failure is the
    // payload, not envelope shape).
    commit_entry.envelope.payload = vec![0xff; 32];
    commit_entry.message_id = commit_entry.envelope.message_id().unwrap();

    let record = run_rejected_entry_diagnostic(&membership_request(
        &fixture,
        &dir.path().join("work"),
        target,
    ))
    .unwrap();

    assert_eq!(record.replay_outcome, ReplayOutcome::Rejected);
    let rejected = record
        .rejected
        .as_ref()
        .expect("the Commit is attributed exactly");
    assert_eq!(rejected.seq, fixture.commit_seq);
    assert_eq!(rejected.kind, RejectedEntryKind::Commit);
    assert_eq!(
        record.error_class,
        Some(RejectedEntryErrorClass::ProtocolEnvelopeParsing),
        "the Commit payload no longer deserializes"
    );
    assert_eq!(
        record.repair_disposition,
        Some(RepairDisposition::NonSkippableCommit),
        "a malformed Commit is non-skippable"
    );
    assert_eq!(
        record.cursor_after, record.cursor_before,
        "no application gap is opened: the cursor cannot move past the Commit"
    );
    assert!(!record.device_state_candidate_persisted);
    let json = serde_json::to_value(&record).unwrap();
    assert!(
        !serde_json::to_string(&json).unwrap().contains("gap"),
        "the schema has no application-gap path at all"
    );
}

#[test]
fn commit_entry_with_application_content_is_commit_proposal_membership() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = membership_room_fixture(dir.path());

    // The Commit entry's payload is swapped for a well-formed application
    // ciphertext: MLS processing succeeds but the content is not a Commit.
    // The donor entry is removed so its ratchet generation stays
    // unconsumed, and the derived message id is recomputed.
    let mut target = fixture.target.clone();
    let donor_payload = target
        .entries
        .iter()
        .find(|entry| entry.seq == fixture.app_b_seq)
        .unwrap()
        .envelope
        .payload
        .clone();
    target
        .entries
        .retain(|entry| entry.seq != fixture.app_b_seq);
    let commit_entry = target
        .entries
        .iter_mut()
        .find(|entry| entry.seq == fixture.commit_seq)
        .unwrap();
    commit_entry.envelope.payload = donor_payload;
    commit_entry.message_id = commit_entry.envelope.message_id().unwrap();

    let record = run_rejected_entry_diagnostic(&membership_request(
        &fixture,
        &dir.path().join("work"),
        target,
    ))
    .unwrap();

    assert_eq!(record.replay_outcome, ReplayOutcome::Rejected);
    let rejected = record.rejected.as_ref().expect("rejected entry");
    assert_eq!(rejected.seq, fixture.commit_seq);
    assert_eq!(rejected.kind, RejectedEntryKind::Commit);
    assert_eq!(
        record.error_class,
        Some(RejectedEntryErrorClass::CommitProposalMembership)
    );
    assert_eq!(
        record.repair_disposition,
        Some(RepairDisposition::NonSkippableCommit)
    );
    assert_eq!(record.cursor_after, record.cursor_before);
}

#[test]
fn application_entry_with_commit_content_is_mls_application_ciphertext() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = membership_room_fixture(dir.path());

    // The membership Commit is relabeled as an Application entry: it
    // decrypts cleanly but the MLS content is a Commit, not application
    // data. The derived message id is recomputed.
    let mut target = fixture.target.clone();
    let commit_entry = target
        .entries
        .iter_mut()
        .find(|entry| entry.seq == fixture.commit_seq)
        .unwrap();
    commit_entry.kind = LogEntryKind::Application;
    commit_entry.envelope.kind = LogEntryKind::Application;
    commit_entry.message_id = commit_entry.envelope.message_id().unwrap();

    let record = run_rejected_entry_diagnostic(&membership_request(
        &fixture,
        &dir.path().join("work"),
        target,
    ))
    .unwrap();

    assert_eq!(record.replay_outcome, ReplayOutcome::Rejected);
    let rejected = record.rejected.as_ref().expect("rejected entry");
    assert_eq!(rejected.seq, fixture.commit_seq);
    assert_eq!(rejected.kind, RejectedEntryKind::Application);
    assert_eq!(
        record.error_class,
        Some(RejectedEntryErrorClass::MlsApplicationCiphertext)
    );
    assert_eq!(
        record.repair_disposition,
        Some(RepairDisposition::ClassificationOnly)
    );
    assert_eq!(record.cursor_after, record.cursor_before);
}

#[test]
fn commit_at_unexpected_epoch_is_mls_epoch_or_state_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = membership_room_fixture(dir.path());

    // The Commit claims an epoch the local group is not at: the signature
    // of a device whose local MLS state diverged from the log.
    let mut target = fixture.target.clone();
    let commit_entry = target
        .entries
        .iter_mut()
        .find(|entry| entry.seq == fixture.commit_seq)
        .unwrap();
    let claimed_epoch = commit_entry.epoch + 1;
    commit_entry.epoch = claimed_epoch;
    commit_entry.envelope.epoch = claimed_epoch;
    commit_entry.message_id = commit_entry.envelope.message_id().unwrap();

    let record = run_rejected_entry_diagnostic(&membership_request(
        &fixture,
        &dir.path().join("work"),
        target,
    ))
    .unwrap();

    assert_eq!(record.replay_outcome, ReplayOutcome::Rejected);
    let rejected = record.rejected.as_ref().expect("rejected entry");
    assert_eq!(rejected.seq, fixture.commit_seq);
    assert_eq!(rejected.kind, RejectedEntryKind::Commit);
    assert_eq!(
        record.error_class,
        Some(RejectedEntryErrorClass::MlsEpochOrStateMismatch)
    );
    assert_eq!(
        record.repair_disposition,
        Some(RepairDisposition::NonSkippableCommit)
    );
    assert_eq!(record.cursor_after, record.cursor_before);
}

#[test]
fn broken_room_does_not_block_healthy_room_progress() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = two_room_fixture(dir.path());

    let record =
        run_rejected_entry_diagnostic(&diagnostic_request(&fixture, &dir.path().join("work")))
            .unwrap();

    assert_eq!(record.replay_outcome, ReplayOutcome::Rejected);
    assert_eq!(record.other_rooms_attempted, 1);
    assert_eq!(record.other_rooms_advanced, 1);
    assert!(
        record.later_rooms_continued,
        "one broken room does not block a healthy room's independent progress"
    );

    // The healthy room really advanced on the throwaway copy.
    let replay_copy = dir
        .path()
        .join("work")
        .join("rejected-entry-replay-store.sqlite3");
    let store = sqlite_client_store(&replay_copy, &fixture.hosted_config);
    let reloaded = store.load_device(fixture.hosted_config.clone()).unwrap();
    assert!(
        reloaded.last_applied_seq(&fixture.healthy.room_id).unwrap()
            > fixture.healthy_cursor_before
    );
    assert_eq!(
        reloaded.last_applied_seq(&fixture.target.room_id).unwrap(),
        fixture.hosted_cursor_before
    );
}

#[test]
fn replaying_the_same_snapshot_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = two_room_fixture(dir.path());

    let first: RejectedEntryDiagnostic =
        run_rejected_entry_diagnostic(&diagnostic_request(&fixture, &dir.path().join("work-a")))
            .unwrap();
    let second =
        run_rejected_entry_diagnostic(&diagnostic_request(&fixture, &dir.path().join("work-b")))
            .unwrap();

    assert_eq!(first, second, "same snapshot, same record");
    assert_eq!(
        first.rejected.as_ref().map(|entry| entry.sha256.clone()),
        second.rejected.as_ref().map(|entry| entry.sha256.clone()),
        "same entry binding"
    );
    assert_eq!(first.error_class, second.error_class, "same class");
}

#[test]
fn diagnostic_output_contains_no_seeded_plaintext_identifiers_or_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = two_room_fixture(dir.path());

    let record =
        run_rejected_entry_diagnostic(&diagnostic_request(&fixture, &dir.path().join("work")))
            .unwrap();
    let json = serde_json::to_string_pretty(&record).unwrap();

    let hosted_account_id = hex_lower(
        fixture
            .hosted_config
            .account_secret_key
            .public_key()
            .as_bytes(),
    );
    let alice_account_id = hex_lower(
        test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_fixture_operator")
            .account_secret_key
            .public_key()
            .as_bytes(),
    );
    let forbidden = [
        fixture.plaintext_marker.clone(),
        "SECRET-PLAINTEXT-MARKER".to_owned(),
        "SECRET-FILENAME.pdf".to_owned(),
        "secret.invalid".to_owned(),
        "valid trailing message".to_owned(),
        "healthy room progress".to_owned(),
        fixture.target.room_id.clone(),
        fixture.healthy.room_id.clone(),
        "mls_track_b_broken".to_owned(),
        "mls_track_b_healthy".to_owned(),
        hosted_account_id,
        alice_account_id,
        "hosted_web_device".to_owned(),
        "alice_fixture_operator".to_owned(),
        hex_lower(&HOSTED_ACCOUNT_SECRET_BYTES),
        hex_lower(&fixture.corrupted_payload),
        "broken-corrupted-entry".to_owned(),
        "welcome_hosted_broken".to_owned(),
    ];
    for marker in &forbidden {
        assert!(
            !json.contains(marker.as_str()),
            "diagnostic output must not contain {marker:?}"
        );
    }
    // Non-vacuous control: the binding digest is present.
    let rejected = record.rejected.expect("rejected entry");
    assert!(json.contains(&rejected.sha256));
    assert!(json.contains(INCIDENT_ALIAS));
}

#[test]
fn diagnostic_never_writes_source_store_or_log_capture() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = two_room_fixture(dir.path());

    // Write the capture to disk as the operator would.
    let capture_path = dir.path().join("room-log-capture.json");
    let capture_file = CapturedRoomLogFile {
        target_room_id: fixture.target.room_id.clone(),
        rooms: vec![fixture.target.clone(), fixture.healthy.clone()],
    };
    std::fs::write(
        &capture_path,
        serde_json::to_vec_pretty(&capture_file).unwrap(),
    )
    .unwrap();

    let hash_files = |root: &std::path::Path| {
        let mut hashes: Vec<(std::path::PathBuf, String)> = Vec::new();
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() && !path.ends_with("work") {
                hashes.push((path.clone(), sha256_hex(&std::fs::read(&path).unwrap())));
            }
        }
        hashes.sort();
        hashes
    };
    let before = hash_files(dir.path());

    run_rejected_entry_diagnostic(&diagnostic_request(&fixture, &dir.path().join("work"))).unwrap();

    let after = hash_files(dir.path());
    assert_eq!(
        before, after,
        "source store, fixture server log, and capture file are byte-identical"
    );
}

#[test]
fn unsupported_entry_kind_stops_without_selecting_a_repair() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = two_room_fixture(dir.path());

    // Turn the corrupted-entry position into an unsupported Proposal entry
    // (kind "other"): the classifier must stop, not classify-and-continue.
    let mut target = fixture.target.clone();
    let proposal = target
        .entries
        .iter_mut()
        .find(|entry| entry.seq == fixture.corrupted_seq)
        .unwrap();
    proposal.kind = LogEntryKind::Proposal;
    proposal.envelope.kind = LogEntryKind::Proposal;

    let record = run_rejected_entry_diagnostic(&RejectedEntryDiagnosticRequest {
        target,
        ..diagnostic_request(&fixture, &dir.path().join("work"))
    })
    .unwrap();

    assert_eq!(record.replay_outcome, ReplayOutcome::Rejected);
    let rejected = record.rejected.expect("the unsupported entry is bound");
    assert_eq!(rejected.seq, fixture.corrupted_seq);
    assert_eq!(rejected.kind, RejectedEntryKind::Other);
    assert_eq!(
        record.error_class,
        Some(RejectedEntryErrorClass::UnsupportedUnclassified)
    );
    assert_eq!(
        record.repair_disposition,
        Some(RepairDisposition::StopUnclassified),
        "unsupported/unclassified failures stop rather than selecting a repair"
    );
    assert_eq!(record.cursor_after, record.cursor_before);
    assert!(!record.device_state_candidate_persisted);
    assert!(
        record.later_rooms_continued,
        "even an unclassified target failure stays quarantined from other rooms"
    );
}

#[test]
fn invalid_incident_alias_is_rejected_before_any_replay() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = two_room_fixture(dir.path());
    for alias in [
        "",
        "operator@example.com",
        "https://chat.finite.computer/room",
        "../hosted-store.sqlite3",
        "alias with spaces",
    ] {
        let request = RejectedEntryDiagnosticRequest {
            incident_alias: alias.to_owned(),
            ..diagnostic_request(&fixture, &dir.path().join("work"))
        };
        assert!(
            run_rejected_entry_diagnostic(&request).is_err(),
            "alias {alias:?} is rejected"
        );
    }
}
