//! End-to-end proofs for `finitechat repair heal-behind-server`, the
//! one-shot that clears durable currency-gate rewind evidence a rekey
//! already healed. The fixtures reproduce the 2026-09-19→24 lat4 wedge
//! exactly: a store restored from an older copy while the server already
//! accepted the agent's later send (evidence recorded at epoch 1), then a
//! rekey run by an image that minted and merged its own Commit without
//! clearing the evidence — healed crypto, stale flag, mute agent.

use finitechat_client::{
    ClientError, FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery,
    ReqwestHttpRuntimeTransport, RuntimeDelivery, RuntimeSyncOptions, RuntimeWorkerError,
    SqliteClientStore, SqliteClientStoreOptions, run_runtime_sync_tick,
};
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::{CreateRoomRequest, DurableAppEventKind, RoomProtocol};
use finitechat_server::{HttpServerState, http_router};
use serde_json::Value;

const ALICE_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [71; NOSTR_SECRET_KEY_BYTES];
const HOSTED_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [73; NOSTR_SECRET_KEY_BYTES];
const ROOM_ID: &str = "room_repair_heal_behind_server";
const MLS_GROUP_ID: &str = "mls_repair_heal_behind_server";
const HOSTED_DEVICE_ID: &str = "hosted_heal_repair_device";

fn test_config(
    account_secret_bytes: [u8; NOSTR_SECRET_KEY_BYTES],
    device_id: &str,
) -> FiniteChatDeviceConfig {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    FiniteChatDeviceConfig {
        account_secret_key: NostrSecretKey::from_bytes(account_secret_bytes).unwrap(),
        device_id: device_id.to_string(),
        now_unix_seconds: now,
        credential_not_before_unix_seconds: now - 60,
        credential_not_after_unix_seconds: now + 60,
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

fn spawn_live_http_server(path: &std::path::Path) -> String {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let app = http_router(HttpServerState::from_sqlite_path(path).unwrap());
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    let server_url = format!("http://{addr}");
    let client = reqwest::blocking::Client::new();
    for _ in 0..100 {
        if client
            .get(format!("{server_url}/health"))
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            return server_url;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("live HTTP server did not become healthy");
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

type TestDelivery = HttpRuntimeDelivery<ReqwestHttpRuntimeTransport>;

fn test_delivery(server_url: &str) -> TestDelivery {
    HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.to_owned()))
}

/// The production send shape: encrypt, append, record the accept, save.
fn send_recorded(
    delivery: &mut TestDelivery,
    device: &mut FiniteChatDevice,
    store: &mut SqliteClientStore,
    text: &str,
    idempotency_key: &str,
) -> u64 {
    let plaintext = format!(r#"{{"type":"finitecomputer.command.v1","body":{{"text":"{text}"}}}}"#)
        .into_bytes();
    let request = device
        .create_application_request(ROOM_ID, &plaintext, idempotency_key)
        .unwrap();
    let accepted = delivery
        .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap();
    device
        .record_own_send_accepted(ROOM_ID, accepted.seq, &accepted.message_id)
        .unwrap();
    store.save_device_state(device).unwrap();
    accepted.seq
}

fn room_cursor(device: &FiniteChatDevice) -> finitechat_client::RoomSyncCursor {
    device
        .room_sync_cursors()
        .into_iter()
        .find(|cursor| cursor.room_id == ROOM_ID)
        .expect("fixture room cursor")
}

fn durable_behind_server(
    store_path: &std::path::Path,
    config: &FiniteChatDeviceConfig,
) -> Option<finitechat_client::BehindServerEvidence> {
    let store = sqlite_client_store(store_path, config);
    let device = store.load_device(config.clone()).unwrap();
    room_cursor(&device).behind_server
}

/// A main-file copy taken after a clean close (no WAL sidecar exists), so
/// restoring it is the production rewind: any later server-accepted sends
/// are above the restored store's own-send mark.
fn snapshot_closed_store(store_path: &std::path::Path, snapshot_path: &std::path::Path) {
    std::fs::copy(store_path, snapshot_path).unwrap();
}

fn restore_snapshot(snapshot_path: &std::path::Path, store_path: &std::path::Path) {
    std::fs::copy(snapshot_path, store_path).unwrap();
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(store_path.with_file_name(format!(
            "{}{suffix}",
            store_path.file_name().unwrap().to_string_lossy()
        )));
    }
}

/// Reproduce the pre-heal-fix rekey one-shot (the lat4 operators' tool on
/// the then-current image): mint, merge, and save the device's own rekey
/// Commit with the cursor landing on it, WITHOUT clearing behind-server
/// evidence — healed crypto, stale durable flag.
fn rekey_like_the_pre_fix_image(
    store: &mut SqliteClientStore,
    device: &mut FiniteChatDevice,
    delivery: &mut TestDelivery,
    idempotency_key: &str,
) {
    let prepared = device
        .prepare_self_update_commit(ROOM_ID, idempotency_key)
        .unwrap();
    store.save_device_state(device).unwrap();
    let accepted = delivery.submit_commit(prepared.request).unwrap();
    let page = delivery
        .sync_events(ROOM_ID, device.device_ref(), accepted.seq - 1)
        .unwrap();
    device
        .merge_pending_commit_from_log(ROOM_ID, &page.entries, &prepared.message_id)
        .unwrap();
    store
        .advance_room_cursor_and_save(device, ROOM_ID, accepted.seq)
        .unwrap();
}

struct HealFixture {
    delivery: TestDelivery,
    alice: FiniteChatDevice,
    alice_store: SqliteClientStore,
    hosted_config: FiniteChatDeviceConfig,
    hosted_secret_hex: String,
    hosted_store_path: std::path::PathBuf,
    /// The rewound store's own-send mark (the evidence's `local_mark`).
    first_seq: u64,
    /// The server-accepted send the rewound store never recorded (the
    /// evidence's `observed_seq`).
    later_seq: u64,
}

enum HealShape {
    /// The lat4 store: rewound (evidence durable at epoch 1) and then
    /// rekeyed by the pre-fix image — stale evidence, healed crypto.
    RekeyedStaleEvidence,
    /// Rewound only: the evidence is still current (no rekey ran).
    CurrentEvidence,
    /// A healthy store that never tripped the gate.
    Healthy,
}

fn build_heal_fixture(dir: &std::path::Path, shape: HealShape) -> HealFixture {
    let server_url = spawn_live_http_server(&dir.join("heal-server.sqlite3"));
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_heal_operator");
    let hosted_config = test_config(HOSTED_ACCOUNT_SECRET_BYTES, HOSTED_DEVICE_ID);
    let mut alice = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut hosted = FiniteChatDevice::new(hosted_config.clone()).unwrap();
    let mut delivery = test_delivery(&server_url);

    let hosted_store_path = dir.join("hosted-store.sqlite3");
    let mut hosted_store = sqlite_client_store(&hosted_store_path, &hosted_config);
    let mut alice_store = sqlite_client_store(dir.join("alice-store.sqlite3"), &alice_config);

    // Bootstrap the room: alice owns it, the hosted device is a member.
    alice.create_group_state(ROOM_ID, MLS_GROUP_ID).unwrap();
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: ROOM_ID.to_string(),
            mls_group_id: MLS_GROUP_ID.to_string(),
            creator: alice.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    delivery
        .upload_key_package(hosted.upload_key_package_request("kp_hosted_heal").unwrap())
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(hosted.device_ref())
        .unwrap()
        .expect("hosted key package");
    let prepared = alice
        .prepare_add_member_commit(
            ROOM_ID,
            &claimed_key_package,
            "welcome_hosted_heal",
            "add-hosted-heal",
        )
        .unwrap();
    let accepted = delivery.submit_commit(prepared.request).unwrap();
    let alice_page = delivery
        .sync_events(ROOM_ID, alice.device_ref(), 0)
        .unwrap();
    alice
        .merge_pending_commit_from_log(ROOM_ID, &alice_page.entries, &prepared.message_id)
        .unwrap();
    alice_store
        .advance_room_cursor_and_save(&mut alice, ROOM_ID, accepted.seq)
        .unwrap();
    let claimed_welcomes = delivery.claim_welcomes(hosted.device_ref()).unwrap();
    let welcome = claimed_welcomes
        .into_iter()
        .find(|welcome| welcome.welcome_id == "welcome_hosted_heal")
        .unwrap();
    hosted_store
        .activate_welcome_and_save(
            &mut hosted,
            "welcome_hosted_heal",
            ROOM_ID,
            &welcome.welcome_payload,
            &welcome.ratchet_tree_payload,
            accepted.seq,
        )
        .unwrap();
    delivery.ack_welcome("welcome_hosted_heal").unwrap();

    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 8,
    };
    // One completed tick flips the room to enforcement before any rewind,
    // so a later rewind trips instead of bootstrapping the mark.
    run_runtime_sync_tick(&mut hosted_store, &mut hosted, &mut delivery, &options).unwrap();

    let first_seq = send_recorded(
        &mut delivery,
        &mut hosted,
        &mut hosted_store,
        "heal first",
        "heal_first",
    );

    let later_seq = match shape {
        HealShape::Healthy => first_seq,
        shape @ (HealShape::RekeyedStaleEvidence | HealShape::CurrentEvidence) => {
            // Rewind: snapshot the closed store at `first`, send `later`
            // from the live store, then restore the snapshot over it. The
            // server now holds an own entry above the restored mark.
            let snapshot_path = dir.join("hosted-store-snapshot.sqlite3");
            drop(hosted_store);
            snapshot_closed_store(&hosted_store_path, &snapshot_path);
            let mut hosted_store = sqlite_client_store(&hosted_store_path, &hosted_config);
            let mut hosted = hosted_store.load_device(hosted_config.clone()).unwrap();
            // The reopened process must verify currency once before it
            // may send (the gate's other half).
            run_runtime_sync_tick(&mut hosted_store, &mut hosted, &mut delivery, &options).unwrap();
            let later_seq = send_recorded(
                &mut delivery,
                &mut hosted,
                &mut hosted_store,
                "heal later",
                "heal_later",
            );
            drop(hosted_store);
            restore_snapshot(&snapshot_path, &hosted_store_path);
            let mut hosted_store = sqlite_client_store(&hosted_store_path, &hosted_config);
            let mut hosted = hosted_store.load_device(hosted_config.clone()).unwrap();

            // The first sync tick records the durable evidence.
            let tripped =
                run_runtime_sync_tick(&mut hosted_store, &mut hosted, &mut delivery, &options)
                    .unwrap_err();
            assert!(
                matches!(
                    &tripped,
                    RuntimeWorkerError::ClientStore(finitechat_client::ClientStoreError::Client(
                        ClientError::DeviceStateBehindServer { .. }
                    )) | RuntimeWorkerError::Client(ClientError::DeviceStateBehindServer { .. })
                ),
                "expected the currency gate to trip, got {tripped:?}"
            );
            let evidence = durable_behind_server(&hosted_store_path, &hosted_config)
                .expect("durable rewind evidence");
            assert_eq!(evidence.local_mark, first_seq);
            assert_eq!(evidence.observed_seq, later_seq);
            assert_eq!(evidence.evidence_epoch, 1);

            if matches!(shape, HealShape::RekeyedStaleEvidence) {
                // The pre-fix rekey: heals the crypto, leaves the flag.
                rekey_like_the_pre_fix_image(
                    &mut hosted_store,
                    &mut hosted,
                    &mut delivery,
                    "heal-pre-fix-rekey",
                );
                assert_eq!(hosted.group_epoch(ROOM_ID).unwrap(), 2);
                assert!(
                    durable_behind_server(&hosted_store_path, &hosted_config).is_some(),
                    "the pre-fix rekey leaves the evidence in place (the wedge)"
                );
            }
            later_seq
        }
    };
    HealFixture {
        delivery,
        alice,
        alice_store,
        hosted_config,
        hosted_secret_hex: hex_lower(&HOSTED_ACCOUNT_SECRET_BYTES),
        hosted_store_path,
        first_seq,
        later_seq,
    }
}

fn run_heal(
    fixture: &HealFixture,
    audit_path: &std::path::Path,
    extra_args: &[&str],
) -> Result<String, String> {
    let mut args = vec![
        "repair".to_owned(),
        "heal-behind-server".to_owned(),
        "--store".to_owned(),
        fixture.hosted_store_path.display().to_string(),
        "--device-id".to_owned(),
        HOSTED_DEVICE_ID.to_owned(),
        "--account-secret-hex".to_owned(),
        fixture.hosted_secret_hex.clone(),
        "--audit-log".to_owned(),
        audit_path.display().to_string(),
    ];
    args.extend(extra_args.iter().map(|arg| arg.to_string()));
    let mut output = Vec::new();
    finitechat_cli::run(args, &mut output)
        .map(|()| String::from_utf8(output).expect("utf8 output"))
        .map_err(|error| error.to_string())
}

fn read_audit_lines(audit_path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(audit_path)
        .expect("audit log exists")
        .lines()
        .map(|line| serde_json::from_str(line).expect("audit line is JSON"))
        .collect()
}

/// The lat4 heal, end to end: the repair clears the stale evidence from
/// the REAL store (targeting the room), the durable flag is gone, and the
/// healed agent sends again with the owner decrypting.
#[test]
fn heals_stale_evidence_on_the_real_store_and_sends_resume() {
    let dir = tempfile::tempdir().unwrap();
    let mut fixture = build_heal_fixture(dir.path(), HealShape::RekeyedStaleEvidence);
    let audit_path = dir.path().join("audit.jsonl");

    let stdout = run_heal(&fixture, &audit_path, &["--room", ROOM_ID]).expect("heal command runs");
    let record: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(record["schema_version"], 1);
    assert_eq!(record["repair_disposition"], "applied");
    assert!(record.get("refusal_reason").is_none());
    let healed = record["healed"].as_array().expect("healed list");
    assert_eq!(healed.len(), 1);
    assert_eq!(healed[0]["room_id"], ROOM_ID);
    assert_eq!(healed[0]["evidence_epoch"], 1);
    assert_eq!(healed[0]["group_epoch"], 2);
    assert_eq!(healed[0]["local_mark"], fixture.first_seq);
    assert_eq!(healed[0]["observed_seq"], fixture.later_seq);
    assert_eq!(
        record["remaining_flagged_rooms"].as_array().unwrap().len(),
        0
    );

    // The durable evidence is gone.
    assert!(durable_behind_server(&fixture.hosted_store_path, &fixture.hosted_config).is_none());

    // Durable audit: one healed line plus one apply summary.
    let audit = read_audit_lines(&audit_path);
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[0]["record"], "heal-behind-server");
    assert_eq!(audit[0]["phase"], "healed");
    assert_eq!(audit[0]["room_id"], ROOM_ID);
    assert_eq!(audit[0]["evidence_epoch"], 1);
    assert_eq!(audit[0]["group_epoch"], 2);
    assert!(audit[0]["recorded_at_unix_seconds"].as_u64().unwrap() > 0);
    assert_eq!(audit[1]["record"], "heal-behind-server");
    assert_eq!(audit[1]["phase"], "apply");
    assert_eq!(audit[1]["flagged"], 1);
    assert_eq!(audit[1]["healed"], 1);

    // The audit log is created private.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&audit_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    // Privacy lock: no account ids, device ids, or secrets in stdout.
    let hosted_account_id = hex_lower(
        fixture
            .hosted_config
            .account_secret_key
            .public_key()
            .as_bytes(),
    );
    for forbidden in [
        hosted_account_id,
        HOSTED_DEVICE_ID.to_owned(),
        fixture.hosted_secret_hex.clone(),
        "heal later".to_owned(),
    ] {
        assert!(
            !stdout.contains(&forbidden),
            "heal output must not contain {forbidden:?}"
        );
    }

    // The healed agent sends again and the owner decrypts.
    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 8,
    };
    let mut hosted_store = sqlite_client_store(&fixture.hosted_store_path, &fixture.hosted_config);
    let mut hosted = hosted_store
        .load_device(fixture.hosted_config.clone())
        .unwrap();
    run_runtime_sync_tick(
        &mut hosted_store,
        &mut hosted,
        &mut fixture.delivery,
        &options,
    )
    .unwrap();
    let healed_seq = send_recorded(
        &mut fixture.delivery,
        &mut hosted,
        &mut hosted_store,
        "heal recovered",
        "heal_recovered",
    );
    drop(hosted_store);
    let owner_report = run_runtime_sync_tick(
        &mut fixture.alice_store,
        &mut fixture.alice,
        &mut fixture.delivery,
        &options,
    )
    .unwrap();
    assert!(
        owner_report
            .applied_entries
            .iter()
            .any(|entry| entry.seq == healed_seq)
    );
}

/// Fail closed: evidence that is still current (no rekey advanced the
/// epoch) refuses the run, the store is untouched, and the refusal names
/// the rekey as the prerequisite.
#[test]
fn refuses_and_changes_nothing_while_the_evidence_is_current() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = build_heal_fixture(dir.path(), HealShape::CurrentEvidence);
    let audit_path = dir.path().join("audit.jsonl");

    let stdout = run_heal(&fixture, &audit_path, &[]).expect("heal command runs");
    let record: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(record["repair_disposition"], "refused");
    assert!(
        record["refusal_reason"]
            .as_str()
            .expect("refusal reason")
            .starts_with("room_not_healed")
    );
    assert_eq!(record["healed"].as_array().unwrap().len(), 0);
    assert_eq!(
        record["remaining_flagged_rooms"]
            .as_array()
            .unwrap()
            .iter()
            .map(|room| room.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![ROOM_ID]
    );
    let refused = &record["refused"];
    assert_eq!(refused["room_id"], ROOM_ID);
    assert_eq!(refused["evidence_epoch"], 1);
    assert_eq!(refused["group_epoch"], 1);

    // The durable evidence survives the refusal untouched.
    let evidence = durable_behind_server(&fixture.hosted_store_path, &fixture.hosted_config)
        .expect("refusal left the evidence in place");
    assert_eq!(evidence.evidence_epoch, 1);

    let audit = read_audit_lines(&audit_path);
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[0]["phase"], "refused");
    assert_eq!(audit[0]["room_id"], ROOM_ID);
    assert_eq!(audit[1]["phase"], "refused");
    assert_eq!(audit[1]["flagged"], 1);
    assert_eq!(audit[1]["healed"], 0);
}

/// A healthy store is an honest no-op: nothing flagged, nothing written.
#[test]
fn healthy_store_reports_nothing_to_heal() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = build_heal_fixture(dir.path(), HealShape::Healthy);
    let audit_path = dir.path().join("audit.jsonl");

    let stdout = run_heal(&fixture, &audit_path, &[]).expect("heal command runs");
    let record: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(record["repair_disposition"], "applied");
    assert_eq!(record["healed"].as_array().unwrap().len(), 0);
    assert_eq!(
        record["remaining_flagged_rooms"].as_array().unwrap().len(),
        0
    );

    let audit = read_audit_lines(&audit_path);
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["phase"], "apply");
    assert_eq!(audit[0]["flagged"], 0);
    assert_eq!(audit[0]["healed"], 0);
}
