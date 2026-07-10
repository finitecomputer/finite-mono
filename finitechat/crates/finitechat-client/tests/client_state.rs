use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use finitechat_client::{
    AppliedLogEntry, ClientError, ClientStoreError, FiniteChatDevice, FiniteChatDeviceConfig,
    HttpRuntimeDelivery, HttpRuntimeDeliveryError, HttpRuntimeTransport, LinkFanoutRoomStatus,
    ReqwestHttpRuntimeTransport, ReqwestHttpRuntimeTransportError, RuntimeDelivery,
    RuntimeLinkFanoutOptions, RuntimeSyncOptions, RuntimeWorkerError, SqliteClientStore,
    SqliteClientStoreOptions, run_link_fanout_tick, run_runtime_sync_tick,
};
use finitechat_delivery::MAX_HTTP_SYNC_PAGE_ENTRIES;
use finitechat_http::{SyncHintEvent, SyncStreamRequest, SyncWaitRoom};
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::LogEntryKind;
use finitechat_proto::{
    AppendEventRequest, CreateRoomRequest, DurableAppEventKind, EventAccepted,
    ListAccountRoomsRequest, RoomProtocol, RoomSyncProjection, WelcomeRecord, envelope,
    lease_token_for,
};
use finitechat_server::{HttpServerState, http_router};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt;

const ALICE_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [17; NOSTR_SECRET_KEY_BYTES];
const BOB_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [19; NOSTR_SECRET_KEY_BYTES];
const CHARLIE_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [23; NOSTR_SECRET_KEY_BYTES];
const DANA_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [29; NOSTR_SECRET_KEY_BYTES];
const ROOM_ID: &str = "room_client_direct";
const MLS_GROUP_ID: &str = "mls_client_direct";
const BOB_KEY_PACKAGE_ID: &str = "kp_bob_client_1";
const BOB_WELCOME_ID: &str = "welcome_bob_client_1";
const NOW: u64 = 1_800_000_000;

#[test]
fn sqlite_client_store_encrypts_state_and_rejects_wrong_or_tampered_key_material() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("encrypted.sqlite3");
    let config = test_config(BOB_ACCOUNT_SECRET_BYTES, "bob_secure_store");
    let mut device = FiniteChatDevice::new(config.clone()).unwrap();
    device
        .create_group_state("room_secure_store", "mls_secure_store")
        .unwrap();
    let exported_state = device.export_state().unwrap();
    let mut store = sqlite_client_store(&path, &config);

    store.save_device_state(&device).unwrap();
    let conn = Connection::open(&path).unwrap();
    let legacy_tables: u64 = conn
        .query_row(
            r#"
            SELECT COUNT(*)
            FROM sqlite_master
            WHERE type = 'table'
              AND name IN ('client_profiles', 'client_rooms', 'client_openmls_storage')
            "#,
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_tables, 0);

    let (nonce, ciphertext): (Vec<u8>, Vec<u8>) = conn
        .query_row(
            r#"
            SELECT nonce, ciphertext
            FROM client_device_states
            WHERE account_id = ?1 AND device_id = ?2
            "#,
            params![
                hex_lower(config.account_secret_key.public_key().as_bytes()),
                &config.device_id,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(nonce.len(), 12);
    assert!(
        !contains_subsequence(&ciphertext, &exported_state.credential_identity),
        "credential identity should only appear inside encrypted state"
    );
    let storage_value = exported_state
        .openmls_storage_records
        .iter()
        .find(|record| record.value.len() >= 16)
        .expect("OpenMLS should persist at least one non-trivial secret row");
    assert!(
        !contains_subsequence(&ciphertext, &storage_value.value),
        "OpenMLS storage values should only appear inside encrypted state"
    );
    drop(conn);

    let wrong_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "bob_secure_store");
    let wrong_store = SqliteClientStore::open(
        &path,
        SqliteClientStoreOptions::from_nostr_secret(
            &wrong_config.account_secret_key,
            &config.device_id,
        )
        .unwrap(),
    )
    .unwrap();
    let wrong_key_error = match wrong_store.load_device(config.clone()) {
        Ok(_) => panic!("wrong local store key should not decrypt client state"),
        Err(error) => error,
    };
    assert!(matches!(wrong_key_error, ClientStoreError::DecryptState));

    let mut tampered = ciphertext;
    tampered[0] ^= 0x01;
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        r#"
        UPDATE client_device_states
        SET ciphertext = ?1
        WHERE account_id = ?2 AND device_id = ?3
        "#,
        params![
            tampered,
            hex_lower(config.account_secret_key.public_key().as_bytes()),
            &config.device_id,
        ],
    )
    .unwrap();
    drop(conn);
    let tamper_error = match store.load_device(config) {
        Ok(_) => panic!("tampered local store ciphertext should not decrypt"),
        Err(error) => error,
    };
    assert!(matches!(tamper_error, ClientStoreError::DecryptState));
}

#[test]
fn runtime_sync_tick_replenishes_key_packages_over_finitechat_http_routes() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_runtime_worker");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut alice = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let options = RuntimeSyncOptions {
        key_package_target_available: 2,
        max_sync_pages_per_room: 4,
    };
    alice_store.save_device_state(&alice).unwrap();

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let report =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(report.uploaded_key_packages, 2);
    assert_eq!(report.claimed_welcomes, 0);
    assert_eq!(report.activated_welcome_acks_sent, 0);
    assert!(report.applied_entries.is_empty());
    let inventory = delivery.key_package_inventory(alice.device_ref()).unwrap();
    assert_eq!(inventory.available, 2);
    assert_eq!(inventory.leased, 0);

    let mut alice = alice_store.load_device(alice_config).unwrap();
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let replay =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(replay.uploaded_key_packages, 0);
    let inventory = delivery.key_package_inventory(alice.device_ref()).unwrap();
    assert_eq!(inventory.available, 2);
    assert_eq!(inventory.leased, 0);
}

#[test]
fn runtime_sync_tick_replenishes_after_key_package_claim() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_claim_replenish");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut alice = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let options = RuntimeSyncOptions {
        key_package_target_available: 1,
        max_sync_pages_per_room: 4,
    };
    alice_store.save_device_state(&alice).unwrap();

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let initial =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(initial.uploaded_key_packages, 1);
    let claimed = delivery
        .claim_key_package_for_device(alice.device_ref())
        .unwrap();
    assert!(claimed.is_some());
    let inventory = delivery.key_package_inventory(alice.device_ref()).unwrap();
    assert_eq!(inventory.available, 0);
    assert_eq!(inventory.leased, 1);

    let mut alice = alice_store.load_device(alice_config).unwrap();
    let replenished =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(replenished.uploaded_key_packages, 1);
    let inventory = delivery.key_package_inventory(alice.device_ref()).unwrap();
    assert_eq!(inventory.available, 1);
    assert_eq!(inventory.leased, 1);
}

#[test]
fn runtime_delivery_claims_key_package_metadata_over_finitechat_http_routes() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_key_package_claim");
    let request = bob.upload_key_package_request("kp_http_claim_bob").unwrap();

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    delivery.upload_key_package(request.clone()).unwrap();
    let claimed = delivery
        .claim_key_package_for_device(&request.owner)
        .unwrap()
        .expect("uploaded package can be claimed");
    assert_eq!(claimed.key_package_id, request.key_package_id);
    assert_eq!(claimed.owner, request.owner);
    assert_eq!(claimed.key_package_ref, request.key_package_ref);
    assert_eq!(claimed.key_package_hash, request.key_package_hash);
    assert_eq!(claimed.key_package_payload, request.key_package_payload);
    assert_eq!(
        claimed.lease_token,
        lease_token_for(&claimed.key_package_id, &claimed.owner)
    );

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let replay = delivery
        .claim_key_package_for_device(&claimed.owner)
        .unwrap();
    assert_eq!(replay, None);
}

#[test]
fn reqwest_http_runtime_delivery_claims_key_package_over_live_server() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http-live.sqlite3");
    let server_url = spawn_live_http_server(&server_db);
    let bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_live_key_package_claim");
    let request = bob
        .upload_key_package_request("kp_http_live_claim_bob")
        .unwrap();

    let mut delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.clone()));
    delivery.upload_key_package(request.clone()).unwrap();
    let claimed = delivery
        .claim_key_package_for_device(&request.owner)
        .unwrap()
        .expect("uploaded package can be claimed over live HTTP");
    assert_eq!(claimed.key_package_id, request.key_package_id);
    assert_eq!(claimed.owner, request.owner);
    assert_eq!(claimed.key_package_ref, request.key_package_ref);
    assert_eq!(claimed.key_package_hash, request.key_package_hash);
    assert_eq!(claimed.key_package_payload, request.key_package_payload);

    let mut delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(format!("{server_url}/")));
    let replay = delivery
        .claim_key_package_for_device(&claimed.owner)
        .unwrap();
    assert_eq!(replay, None);

    let mut transport = ReqwestHttpRuntimeTransport::new(server_url);
    let missing = transport
        .post_json::<_, serde_json::Value>("/missing", &serde_json::json!({"probe": true}));
    assert!(matches!(
        missing,
        Err(ReqwestHttpRuntimeTransportError::Server {
            status: StatusCode::NOT_FOUND,
            ..
        })
    ));
}

#[test]
fn reqwest_http_runtime_delivery_reads_sync_stream_hints_over_live_server() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http-live-sse.sqlite3");
    let server_url = spawn_live_http_server(&server_db);
    let room_id = "room_http_live_sse";
    let mls_group_id = "mls_http_live_sse";
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_live_sse");
    let mut alice = FiniteChatDevice::new(alice_config).unwrap();
    alice.create_group_state(room_id, mls_group_id).unwrap();

    let mut delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.clone()));
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: room_id.to_owned(),
            mls_group_id: mls_group_id.to_owned(),
            creator: alice.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    let mut stream = delivery
        .sync_stream(&SyncStreamRequest {
            rooms: vec![SyncWaitRoom {
                room_id: room_id.to_owned(),
                after_seq: 0,
            }],
            inbox: None,
            heartbeat_ms: Some(60_000),
        })
        .unwrap();

    let request = alice
        .create_application_request(room_id, b"hello over sse", "app_http_live_sse")
        .unwrap();
    let accepted = delivery
        .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap();
    assert_eq!(
        stream.next_hint().unwrap(),
        SyncHintEvent::RoomAdvanced {
            room_id: room_id.to_owned(),
            seq: accepted.seq,
        }
    );
}

#[test]
fn reqwest_http_runtime_sync_tick_syncs_room_pages_over_live_server() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http-live-room-sync.sqlite3");
    let server_url = spawn_live_http_server(&server_db);
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_live_room_sync");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut alice = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_live_room_sync");
    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 4,
    };

    let mut delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.clone()));
    let add_accepted_seq = create_group_room_with_member(
        &mut delivery,
        &mut alice,
        &mut bob,
        GroupMemberSetup {
            room_id: ROOM_ID,
            mls_group_id: MLS_GROUP_ID,
            key_package_id: "kp_http_live_room_bob",
            welcome_id: "welcome_http_live_room_bob",
            idempotency_key: "commit_http_live_room_bob",
        },
    );
    alice_store
        .advance_room_cursor_and_save(&mut alice, ROOM_ID, add_accepted_seq)
        .unwrap();

    let plaintext =
        br#"{"type":"finitecomputer.command.v1","body":{"text":"live http room sync"}}"#;
    let message = bob
        .create_application_request(ROOM_ID, plaintext, "app_http_live_room_sync")
        .unwrap();
    let message_accepted = delivery
        .append_event(&message, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap();
    assert_eq!(message_accepted.seq, add_accepted_seq + 1);

    let report =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(report.sync_pages, 1);
    assert_eq!(report.applied_entries.len(), 1);
    assert_eq!(report.applied_entries[0].room_id, ROOM_ID);
    assert_eq!(report.applied_entries[0].seq, message_accepted.seq);
    assert_eq!(
        report.applied_entries[0].entry,
        AppliedLogEntry::Application {
            plaintext: plaintext.to_vec(),
            sender: bob.device_ref().clone(),
        }
    );

    let mut alice = alice_store.load_device(alice_config).unwrap();
    let mut delivery = HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url));
    let replay =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert!(replay.applied_entries.is_empty());
}

#[test]
fn runtime_sync_tick_claims_and_acks_welcomes_over_finitechat_http_routes() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_welcome_worker");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut alice = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_welcome_sender");
    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 4,
    };
    alice_store.save_device_state(&alice).unwrap();

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    bob.create_group_state(ROOM_ID, MLS_GROUP_ID).unwrap();
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: ROOM_ID.to_owned(),
            mls_group_id: MLS_GROUP_ID.to_owned(),
            creator: bob.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    delivery
        .upload_key_package(
            alice
                .upload_key_package_request("kp_http_welcome_alice")
                .unwrap(),
        )
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(alice.device_ref())
        .unwrap()
        .expect("alice package");
    let prepared = bob
        .prepare_add_member_commit(
            ROOM_ID,
            &claimed_key_package,
            "welcome_http_runtime_alice",
            "commit_http_runtime_alice",
        )
        .unwrap();
    let accepted = delivery.submit_commit(prepared.request).unwrap();

    let report =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(report.uploaded_key_packages, 0);
    assert_eq!(report.claimed_welcomes, 1);
    assert_eq!(report.activated_welcome_acks_sent, 1);
    assert_eq!(alice.group_epoch(ROOM_ID).unwrap(), 1);
    assert_eq!(alice.last_applied_seq(ROOM_ID).unwrap(), accepted.seq);
    assert_eq!(alice.pending_welcome_count(), 0);
    assert_eq!(alice.pending_welcome_ack_count(), 0);

    let mut alice = alice_store.load_device(alice_config).unwrap();
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let replay =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(replay.claimed_welcomes, 0);
    assert_eq!(replay.activated_welcome_acks_sent, 0);
    delivery.ack_welcome("welcome_http_runtime_alice").unwrap();
}

#[test]
fn runtime_sync_tick_syncs_room_pages_over_finitechat_http_routes() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_room_sync");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut alice = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_room_sync");
    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 4,
    };

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let add_accepted_seq = create_group_room_with_member(
        &mut delivery,
        &mut alice,
        &mut bob,
        GroupMemberSetup {
            room_id: ROOM_ID,
            mls_group_id: MLS_GROUP_ID,
            key_package_id: "kp_http_room_bob",
            welcome_id: "welcome_http_room_bob",
            idempotency_key: "commit_http_room_bob",
        },
    );
    alice_store
        .advance_room_cursor_and_save(&mut alice, ROOM_ID, add_accepted_seq)
        .unwrap();

    let plaintext = br#"{"type":"finitecomputer.command.v1","body":{"text":"http room sync"}}"#;
    let message = bob
        .create_application_request(ROOM_ID, plaintext, "app_http_room_sync")
        .unwrap();
    let message_accepted = delivery
        .append_event(&message, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap();
    assert_eq!(message_accepted.seq, add_accepted_seq + 1);

    let report =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(report.sync_pages, 1);
    assert_eq!(report.applied_entries.len(), 1);
    assert_eq!(report.applied_entries[0].room_id, ROOM_ID);
    assert_eq!(report.applied_entries[0].seq, message_accepted.seq);
    assert_eq!(
        report.applied_entries[0].entry,
        AppliedLogEntry::Application {
            plaintext: plaintext.to_vec(),
            sender: bob.device_ref().clone(),
        }
    );
    assert_eq!(
        alice.last_applied_seq(ROOM_ID).unwrap(),
        message_accepted.seq
    );

    let mut alice = alice_store.load_device(alice_config).unwrap();
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let replay =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert!(replay.applied_entries.is_empty());
    assert_eq!(
        alice.last_applied_seq(ROOM_ID).unwrap(),
        message_accepted.seq
    );
}

#[test]
fn runtime_sync_tick_repairs_partial_pull_pages_over_finitechat_http_routes() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http-partial-pull.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_partial_pull");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut alice = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_partial_pull");
    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 1,
    };

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let add_accepted_seq = create_group_room_with_member(
        &mut delivery,
        &mut alice,
        &mut bob,
        GroupMemberSetup {
            room_id: ROOM_ID,
            mls_group_id: MLS_GROUP_ID,
            key_package_id: "kp_http_partial_pull_bob",
            welcome_id: "welcome_http_partial_pull_bob",
            idempotency_key: "commit_http_partial_pull_bob",
        },
    );
    alice_store
        .advance_room_cursor_and_save(&mut alice, ROOM_ID, add_accepted_seq)
        .unwrap();

    let mut sent_plaintexts = Vec::new();
    let mut next_message_index = 0;
    send_bob_messages(
        &mut delivery,
        &mut bob,
        90,
        &mut next_message_index,
        MAX_HTTP_SYNC_PAGE_ENTRIES + 1,
        &mut sent_plaintexts,
    );
    assert_eq!(sent_plaintexts.len(), MAX_HTTP_SYNC_PAGE_ENTRIES + 1);

    let first_report =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(first_report.sync_pages, 1);
    assert_eq!(
        first_report.applied_entries.len(),
        MAX_HTTP_SYNC_PAGE_ENTRIES
    );
    assert_eq!(first_report.applied_entries[0].seq, sent_plaintexts[0].seq);
    assert_eq!(
        first_report.applied_entries[0].entry,
        AppliedLogEntry::Application {
            plaintext: sent_plaintexts[0].plaintext.clone(),
            sender: bob.device_ref().clone(),
        }
    );
    assert_eq!(
        first_report.applied_entries.last().unwrap().seq,
        sent_plaintexts[MAX_HTTP_SYNC_PAGE_ENTRIES - 1].seq
    );
    assert_eq!(
        alice.last_applied_seq(ROOM_ID).unwrap(),
        sent_plaintexts[MAX_HTTP_SYNC_PAGE_ENTRIES - 1].seq
    );

    let mut alice = alice_store.load_device(alice_config.clone()).unwrap();
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let repair_report =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(repair_report.sync_pages, 1);
    assert_eq!(repair_report.applied_entries.len(), 1);
    assert_eq!(
        repair_report.applied_entries[0].seq,
        sent_plaintexts[MAX_HTTP_SYNC_PAGE_ENTRIES].seq
    );
    assert_eq!(
        repair_report.applied_entries[0].entry,
        AppliedLogEntry::Application {
            sender: bob.device_ref().clone(),
            plaintext: sent_plaintexts[MAX_HTTP_SYNC_PAGE_ENTRIES]
                .plaintext
                .clone()
        }
    );
    assert_eq!(
        alice.last_applied_seq(ROOM_ID).unwrap(),
        sent_plaintexts[MAX_HTTP_SYNC_PAGE_ENTRIES].seq
    );

    let mut alice = alice_store.load_device(alice_config).unwrap();
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let replay =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut delivery, &options).unwrap();
    assert_eq!(replay.sync_pages, 1);
    assert!(replay.applied_entries.is_empty());
}

#[test]
fn sync_projection_advances_only_from_finitechat_http_pull_pages() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http-sync-projection.sqlite3");
    let room_id = "room_http_sync_projection";
    let mls_group_id = "mls_http_sync_projection";
    let alice = test_device(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_sync_projection");

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: room_id.to_owned(),
            mls_group_id: mls_group_id.to_owned(),
            creator: alice.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();

    for index in 1..=3 {
        let idempotency_key = format!("http_sync_projection_msg_{index}");
        delivery
            .append_event(
                &AppendEventRequest {
                    room_id: room_id.to_owned(),
                    sender: alice.device_ref().clone(),
                    envelope: envelope(
                        room_id.to_owned(),
                        mls_group_id.to_owned(),
                        alice.device_ref().clone(),
                        0,
                        LogEntryKind::Application,
                        format!(r#"{{"body":"message {index}"}}"#).into_bytes(),
                    ),
                    idempotency_key,
                    timestamp_unix_seconds: 1_700_000_000 + index,
                },
                DurableAppEventKind::ChatMessage.delivery_policy(),
            )
            .unwrap();
    }

    let source_page = delivery
        .sync_events(room_id, alice.device_ref(), 0)
        .unwrap();
    assert_eq!(source_page.entries.len(), 3);
    let expected_message_ids = source_page
        .entries
        .iter()
        .map(|entry| entry.message_id.clone())
        .collect::<Vec<_>>();

    let mut projection = RoomSyncProjection::default();
    assert!(
        projection
            .observe_stream_hint(room_id, source_page.entries[1].seq)
            .unwrap()
    );
    assert!(
        projection
            .observe_stream_hint(room_id, source_page.entries[0].seq)
            .unwrap()
    );
    assert!(
        projection
            .observe_stream_hint(room_id, source_page.entries[2].seq)
            .unwrap()
    );
    assert_eq!(projection.server_cursor(), 0);
    assert_eq!(projection.highest_stream_hint(), source_page.next_after_seq);
    assert!(projection.applied_message_ids().is_empty());

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let pulled_page = delivery
        .sync_events(room_id, alice.device_ref(), 0)
        .unwrap();
    let applied = projection.apply_page(room_id, &pulled_page).unwrap();
    assert_eq!(applied.applied_entries, 3);
    assert_eq!(applied.server_cursor, source_page.next_after_seq);
    assert!(!applied.needs_more_pull);
    assert_eq!(projection.server_cursor(), source_page.next_after_seq);
    assert_eq!(
        projection.applied_message_ids(),
        expected_message_ids.as_slice()
    );

    assert!(
        !projection
            .observe_stream_hint(room_id, source_page.entries[0].seq)
            .unwrap()
    );
    assert_eq!(projection.server_cursor(), source_page.next_after_seq);
}

#[test]
fn client_merges_pending_commit_only_after_finitechat_http_log_observation() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir
        .path()
        .join("finitechat-http-pending-observation.sqlite3");
    let room_id = "room_http_pending_observation";
    let mls_group_id = "mls_http_pending_observation";
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let mut alice = test_device(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_pending_observation");
    let bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_pending_observation");

    alice.create_group_state(room_id, mls_group_id).unwrap();
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: room_id.to_owned(),
            mls_group_id: mls_group_id.to_owned(),
            creator: alice.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    delivery
        .upload_key_package(
            bob.upload_key_package_request("kp_bob_http_pending_observation")
                .unwrap(),
        )
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(bob.device_ref())
        .unwrap()
        .expect("bob key package");
    let prepared = alice
        .prepare_add_member_commit(
            room_id,
            &claimed_key_package,
            "welcome_bob_http_pending_observation",
            "commit_bob_http_pending_observation",
        )
        .unwrap();
    assert!(alice.has_pending_commit(room_id).unwrap());
    assert!(matches!(
        alice.create_application_request(room_id, b"too early", "alice_too_early_http_pending"),
        Err(ClientError::PendingCommitMustBeMerged(rejected_room)) if rejected_room == room_id
    ));

    let accepted = delivery.submit_commit(prepared.request.clone()).unwrap();
    assert_eq!(accepted.seq, 1);
    assert_eq!(accepted.message_id, prepared.message_id);
    let unobserved = alice
        .merge_pending_commit_from_log(room_id, &[], &prepared.message_id)
        .unwrap_err();
    assert!(matches!(
        unobserved,
        ClientError::PendingCommitNotObserved(message_id) if message_id == prepared.message_id
    ));
    assert_eq!(alice.group_epoch(room_id).unwrap(), 0);
    assert!(alice.has_pending_commit(room_id).unwrap());

    let page = delivery
        .sync_events(room_id, alice.device_ref(), 0)
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].seq, accepted.seq);
    assert_eq!(page.entries[0].message_id, prepared.message_id);
    alice
        .merge_pending_commit_from_log(room_id, &page.entries, &prepared.message_id)
        .unwrap();
    assert_eq!(alice.group_epoch(room_id).unwrap(), 1);
    assert!(!alice.has_pending_commit(room_id).unwrap());

    let request = alice
        .create_application_request(
            room_id,
            b"after observed commit",
            "alice_after_http_pending_observation",
        )
        .unwrap();
    let accepted_event = delivery
        .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap();
    assert_eq!(accepted_event.seq, 2);
}

#[test]
fn runtime_later_device_history_starts_at_add_commit_over_finitechat_http() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http-history-policy.sqlite3");
    let phone_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_history_phone");
    let mut phone_store = sqlite_client_store(dir.path().join("phone.sqlite3"), &phone_config);
    let mut alice_phone = FiniteChatDevice::new(phone_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_history");
    let room_id = "room_http_history_policy";
    let mls_group_id = "mls_http_history_policy";
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);

    bob.create_group_state(room_id, mls_group_id).unwrap();
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: room_id.to_owned(),
            mls_group_id: mls_group_id.to_owned(),
            creator: bob.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    let prior_plaintext =
        br#"{"type":"finitecomputer.command.v1","body":{"text":"before http add"}}"#;
    let prior = bob
        .create_application_request(room_id, prior_plaintext, "history_http_before_add")
        .unwrap();
    let prior = delivery
        .append_event(&prior, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap();
    assert_eq!(prior.seq, 1);

    phone_store.save_device_state(&alice_phone).unwrap();
    delivery
        .upload_key_package(
            alice_phone
                .upload_key_package_request("kp_history_http_alice_phone")
                .unwrap(),
        )
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(alice_phone.device_ref())
        .unwrap()
        .expect("alice phone key package");
    let prepared = bob
        .prepare_add_member_commit(
            room_id,
            &claimed_key_package,
            "welcome_history_http_alice_phone",
            "history_http_add_alice_phone",
        )
        .unwrap();
    let accepted = delivery.submit_commit(prepared.request).unwrap();
    assert_eq!(accepted.seq, 2);

    let bob_page = delivery.sync_events(room_id, bob.device_ref(), 0).unwrap();
    assert_eq!(bob_page.entries.len(), 2);
    bob.merge_pending_commit_from_log(room_id, &bob_page.entries, &prepared.message_id)
        .unwrap();
    assert_eq!(bob.group_epoch(room_id).unwrap(), 1);

    let post_plaintext =
        br#"{"type":"finitecomputer.command.v1","body":{"text":"after http add"}}"#;
    let post = bob
        .create_application_request(room_id, post_plaintext, "history_http_after_add")
        .unwrap();
    let post = delivery
        .append_event(&post, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap();
    assert_eq!(post.seq, 3);

    let report = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(report.claimed_welcomes, 1);
    assert_eq!(report.activated_welcome_acks_sent, 1);
    assert_eq!(report.applied_entries.len(), 1);
    assert_eq!(report.applied_entries[0].seq, post.seq);
    assert_eq!(
        report.applied_entries[0].entry,
        AppliedLogEntry::Application {
            plaintext: post_plaintext.to_vec(),
            sender: bob.device_ref().clone(),
        }
    );
    assert_eq!(alice_phone.group_epoch(room_id).unwrap(), 1);
    assert_eq!(alice_phone.last_applied_seq(room_id).unwrap(), post.seq);

    let full_page = delivery
        .sync_events(room_id, alice_phone.device_ref(), 0)
        .unwrap();
    assert!(
        full_page
            .entries
            .iter()
            .all(|entry| entry.seq >= accepted.seq)
    );
    assert!(
        !full_page
            .entries
            .iter()
            .any(|entry| entry.message_id == prior.message_id)
    );
    assert_eq!(full_page.entries.len(), 2);
    assert_eq!(full_page.entries[0].kind, LogEntryKind::Commit);
    assert_eq!(full_page.entries[0].seq, accepted.seq);
    assert_eq!(full_page.entries[1].kind, LogEntryKind::Application);
    assert_eq!(full_page.entries[1].message_id, post.message_id);
}

#[test]
fn runtime_removed_device_processes_removal_but_not_future_http_ciphertext() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http-removed-device.sqlite3");
    let charlie_config = test_config(CHARLIE_ACCOUNT_SECRET_BYTES, "charlie_http_removed");
    let mut charlie_store =
        sqlite_client_store(dir.path().join("charlie.sqlite3"), &charlie_config);
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_removed");
    let mut charlie = FiniteChatDevice::new(charlie_config.clone()).unwrap();
    let room_id = "room_http_removed_device";
    let mls_group_id = "mls_http_removed_device";
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);

    bob.create_group_state(room_id, mls_group_id).unwrap();
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: room_id.to_owned(),
            mls_group_id: mls_group_id.to_owned(),
            creator: bob.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    charlie_store.save_device_state(&charlie).unwrap();
    delivery
        .upload_key_package(
            charlie
                .upload_key_package_request("kp_charlie_http_removed")
                .unwrap(),
        )
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(charlie.device_ref())
        .unwrap()
        .expect("charlie key package");
    let add_charlie = bob
        .prepare_add_member_commit(
            room_id,
            &claimed_key_package,
            "welcome_charlie_http_removed",
            "add_charlie_http_removed",
        )
        .unwrap();
    let add_acceptance = delivery.submit_commit(add_charlie.request).unwrap();
    assert_eq!(add_acceptance.seq, 1);
    let bob_add_page = delivery.sync_events(room_id, bob.device_ref(), 0).unwrap();
    bob.merge_pending_commit_from_log(room_id, &bob_add_page.entries, &add_charlie.message_id)
        .unwrap();

    let join_report = run_runtime_sync_tick(
        &mut charlie_store,
        &mut charlie,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(join_report.claimed_welcomes, 1);
    assert_eq!(join_report.activated_welcome_acks_sent, 1);
    assert_eq!(charlie.group_epoch(room_id).unwrap(), 1);
    let stale_charlie_state = charlie.export_state().unwrap();
    let mut stale_charlie =
        FiniteChatDevice::from_state(charlie_config, stale_charlie_state).unwrap();

    let remove_charlie = bob
        .prepare_remove_member_commit(room_id, stale_charlie.device_ref(), "remove_charlie_http")
        .unwrap();
    let remove_acceptance = delivery.submit_commit(remove_charlie.request).unwrap();
    assert_eq!(remove_acceptance.seq, 2);
    let bob_remove_page = delivery
        .sync_events(room_id, bob.device_ref(), add_acceptance.seq)
        .unwrap();
    bob.merge_pending_commit_from_log(
        room_id,
        &bob_remove_page.entries,
        &remove_charlie.message_id,
    )
    .unwrap();
    assert_eq!(bob.group_epoch(room_id).unwrap(), 2);

    let stale_page = delivery
        .sync_events(room_id, stale_charlie.device_ref(), add_acceptance.seq)
        .unwrap();
    assert_eq!(stale_page.entries.len(), 1);
    assert_eq!(stale_page.entries[0].seq, remove_acceptance.seq);
    let stale_old_epoch_send = stale_charlie
        .create_application_request(room_id, b"stale", "charlie_removed_old_epoch_http")
        .unwrap();
    let err = delivery
        .append_event(
            &stale_old_epoch_send,
            DurableAppEventKind::ChatMessage.delivery_policy(),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        HttpRuntimeDeliveryError::Transport(InProcessHttpTransportError::HttpStatus(
            StatusCode::BAD_REQUEST,
            body,
        ))
            if body.contains("invalid_event_request")
    ));
    assert_eq!(
        stale_charlie
            .apply_log_entry(room_id, &stale_page.entries[0])
            .unwrap(),
        AppliedLogEntry::Commit {
            sender: bob.device_ref().clone(),
            epoch: 2,
        }
    );
    assert!(matches!(
        stale_charlie.create_application_request(room_id, b"removed", "charlie_removed_http"),
        Err(ClientError::CreateApplicationMessage)
    ));
    let forged_new_epoch_send = AppendEventRequest {
        room_id: room_id.to_owned(),
        sender: stale_charlie.device_ref().clone(),
        envelope: envelope(
            room_id.to_owned(),
            mls_group_id.to_owned(),
            stale_charlie.device_ref().clone(),
            2,
            LogEntryKind::Application,
            b"forged removed sender".to_vec(),
        ),
        idempotency_key: "charlie_removed_new_epoch_http".to_owned(),
        timestamp_unix_seconds: 1_700_000_000,
    };
    let err = delivery
        .append_event(
            &forged_new_epoch_send,
            DurableAppEventKind::ChatMessage.delivery_policy(),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        HttpRuntimeDeliveryError::Transport(InProcessHttpTransportError::HttpStatus(
            StatusCode::FORBIDDEN,
            body,
        ))
            if body.contains("sender_not_active")
    ));

    let plaintext =
        br#"{"type":"finitecomputer.command.v1","body":{"text":"not for removed over http"}}"#;
    let post_removal = bob
        .create_application_request(room_id, plaintext, "bob_after_http_remove")
        .unwrap();
    let post_removal = delivery
        .append_event(
            &post_removal,
            DurableAppEventKind::ChatMessage.delivery_policy(),
        )
        .unwrap();
    assert_eq!(post_removal.seq, 3);
    let bob_post_page = delivery
        .sync_events(room_id, bob.device_ref(), remove_acceptance.seq)
        .unwrap();
    assert_eq!(bob_post_page.entries.len(), 1);
    assert!(matches!(
        stale_charlie.decrypt_application_entry(room_id, &bob_post_page.entries[0]),
        Err(ClientError::ProcessMessage { .. })
    ));
    let stale_future_page = delivery
        .sync_events(room_id, stale_charlie.device_ref(), remove_acceptance.seq)
        .unwrap();
    assert!(stale_future_page.entries.is_empty());
    assert_eq!(stale_future_page.next_after_seq, post_removal.seq);
}

#[test]
fn http_runtime_delivery_filters_membership_and_rejects_pending_sends() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let room_id = "room_http_membership_filter";
    let mls_group_id = "mls_http_membership_filter";
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_membership_filter");
    let mut alice = test_device(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_membership_filter");

    bob.create_group_state(room_id, mls_group_id).unwrap();
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: room_id.to_owned(),
            mls_group_id: mls_group_id.to_owned(),
            creator: bob.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();

    let before_add = bob
        .create_application_request(room_id, b"before add", "bob_before_http_filter")
        .unwrap();
    let before_add = delivery
        .append_event(
            &before_add,
            DurableAppEventKind::ChatMessage.delivery_policy(),
        )
        .unwrap();
    assert_eq!(before_add.seq, 1);

    let alice_hidden = delivery
        .sync_events(room_id, alice.device_ref(), 0)
        .unwrap();
    assert!(alice_hidden.entries.is_empty());
    assert_eq!(alice_hidden.next_after_seq, before_add.seq);

    delivery
        .upload_key_package(
            alice
                .upload_key_package_request("kp_alice_http_membership_filter")
                .unwrap(),
        )
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(alice.device_ref())
        .unwrap()
        .expect("alice key package");
    let prepared = bob
        .prepare_add_member_commit(
            room_id,
            &claimed_key_package,
            "welcome_alice_http_membership_filter",
            "bob_add_alice_http_membership_filter",
        )
        .unwrap();
    let accepted = delivery.submit_commit(prepared.request).unwrap();
    assert_eq!(accepted.seq, 2);

    let bob_page = delivery.sync_events(room_id, bob.device_ref(), 0).unwrap();
    assert_eq!(bob_page.entries.len(), 2);
    bob.merge_pending_commit_from_log(room_id, &bob_page.entries, &prepared.message_id)
        .unwrap();

    let pending_send = AppendEventRequest {
        room_id: room_id.to_owned(),
        sender: alice.device_ref().clone(),
        envelope: envelope(
            room_id.to_owned(),
            mls_group_id.to_owned(),
            alice.device_ref().clone(),
            1,
            LogEntryKind::Application,
            b"pending send".to_vec(),
        ),
        idempotency_key: "alice_pending_http_filter".to_owned(),
        timestamp_unix_seconds: 1_700_000_000,
    };
    let err = delivery
        .append_event(
            &pending_send,
            DurableAppEventKind::ChatMessage.delivery_policy(),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        HttpRuntimeDeliveryError::Transport(InProcessHttpTransportError::HttpStatus(
            StatusCode::FORBIDDEN,
            body,
        ))
            if body.contains("sender_not_active")
    ));

    let alice_pending_page = delivery
        .sync_events(room_id, alice.device_ref(), alice_hidden.next_after_seq)
        .unwrap();
    assert_eq!(alice_pending_page.entries.len(), 1);
    assert_eq!(alice_pending_page.entries[0].kind, LogEntryKind::Commit);

    let claimed_welcomes = delivery.claim_welcomes(alice.device_ref()).unwrap();
    assert_eq!(claimed_welcomes.len(), 1);
    alice
        .activate_welcome(
            room_id,
            &claimed_welcomes[0].welcome_payload,
            &claimed_welcomes[0].ratchet_tree_payload,
        )
        .unwrap();
    delivery
        .ack_welcome("welcome_alice_http_membership_filter")
        .unwrap();

    let after_activation = alice
        .create_application_request(
            room_id,
            b"after activation",
            "alice_after_http_filter_activation",
        )
        .unwrap();
    let after_activation = delivery
        .append_event(
            &after_activation,
            DurableAppEventKind::ChatMessage.delivery_policy(),
        )
        .unwrap();
    assert_eq!(after_activation.seq, 3);

    let bob_after = delivery
        .sync_events(room_id, bob.device_ref(), accepted.seq)
        .unwrap();
    assert_eq!(bob_after.entries.len(), 1);
    assert_eq!(
        bob.decrypt_application_entry(room_id, &bob_after.entries[0])
            .unwrap()
            .plaintext,
        b"after activation"
    );
}

#[test]
fn runtime_link_fanout_discovers_account_rooms_over_finitechat_http_routes() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_account_rooms");
    let phone_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "phone_http_account_rooms");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut alice = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut alice_phone = FiniteChatDevice::new(phone_config).unwrap();
    let room_id = "room_http_account_directory";
    let group_id = "mls_http_account_directory";
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    create_group_room_with_member(
        &mut delivery,
        &mut alice,
        &mut alice_phone,
        GroupMemberSetup {
            room_id,
            mls_group_id: group_id,
            key_package_id: "kp_phone_http_account_directory",
            welcome_id: "welcome_phone_http_account_directory",
            idempotency_key: "commit_phone_http_account_directory",
        },
    );
    let account_id = alice.device_ref().account_id.clone();
    let account_rooms = delivery
        .list_account_rooms(ListAccountRoomsRequest {
            account_id: account_id.clone(),
            after_room_id: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(account_rooms.rooms.len(), 1);
    assert_eq!(account_rooms.rooms[0].room_id, room_id);
    assert!(
        account_rooms.rooms[0]
            .devices
            .iter()
            .any(|device| device.device == *alice_phone.device_ref())
    );

    alice_store.save_device_state(&alice).unwrap();
    alice_store
        .start_link_fanout_and_save(
            &mut alice,
            "fanout_http_account_directory",
            alice_phone.device_ref().clone(),
        )
        .unwrap();
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let report = run_link_fanout_tick(
        &mut alice_store,
        &mut alice,
        &mut delivery,
        "fanout_http_account_directory",
        &RuntimeLinkFanoutOptions {
            max_discovery_pages_per_tick: 2,
            max_commit_rooms_per_tick: 1,
            max_completion_sync_pages_per_room: 1,
        },
    )
    .unwrap();
    assert_eq!(report.discovery_pages, 1);
    assert_eq!(report.queued_rooms, 0);
    assert_eq!(report.claimed_key_packages, 0);
    assert_eq!(report.prepared_commits, 0);
    assert_eq!(report.submitted_commits, 0);
    assert!(report.complete);
    assert_eq!(
        alice
            .link_fanout_room_count("fanout_http_account_directory")
            .unwrap(),
        0
    );
}

#[test]
fn runtime_link_fanout_tick_links_later_device_over_finitechat_http_routes() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_link_fanout");
    let phone_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "phone_http_link_fanout");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut phone_store = sqlite_client_store(dir.path().join("phone.sqlite3"), &phone_config);
    let mut alice_browser = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut alice_phone = FiniteChatDevice::new(phone_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_link_fanout");
    let room_id = "room_http_link_fanout";
    let group_id = "mls_http_link_fanout";

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let bob_join_seq = create_group_room_with_member(
        &mut delivery,
        &mut alice_browser,
        &mut bob,
        GroupMemberSetup {
            room_id,
            mls_group_id: group_id,
            key_package_id: "kp_bob_http_link_fanout",
            welcome_id: "welcome_bob_http_link_fanout",
            idempotency_key: "add_bob_http_link_fanout",
        },
    );
    alice_store.save_device_state(&alice_browser).unwrap();
    alice_store
        .advance_room_cursor_and_save(&mut alice_browser, room_id, bob_join_seq)
        .unwrap();
    phone_store.save_device_state(&alice_phone).unwrap();

    let account_id = alice_browser.device_ref().account_id.clone();
    let account_rooms = delivery
        .list_account_rooms(ListAccountRoomsRequest {
            account_id: account_id.clone(),
            after_room_id: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(account_rooms.rooms.len(), 1);
    assert_eq!(account_rooms.rooms[0].room_id, room_id);
    assert_eq!(account_rooms.rooms[0].current_epoch, 1);
    assert_eq!(account_rooms.rooms[0].last_seq, bob_join_seq);
    assert!(
        account_rooms.rooms[0]
            .devices
            .iter()
            .any(|device| device.device == *alice_browser.device_ref() && device.active)
    );
    assert!(
        !account_rooms.rooms[0]
            .devices
            .iter()
            .any(|device| device.device == *alice_phone.device_ref())
    );

    let phone_replenish = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 1,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_replenish.uploaded_key_packages, 1);

    alice_store
        .start_link_fanout_and_save(
            &mut alice_browser,
            "fanout_http_link_phone",
            alice_phone.device_ref().clone(),
        )
        .unwrap();
    let report = run_link_fanout_tick(
        &mut alice_store,
        &mut alice_browser,
        &mut delivery,
        "fanout_http_link_phone",
        &RuntimeLinkFanoutOptions {
            max_discovery_pages_per_tick: 2,
            max_commit_rooms_per_tick: 1,
            max_completion_sync_pages_per_room: 2,
        },
    )
    .unwrap();
    assert_eq!(report.discovery_pages, 1);
    assert_eq!(report.queued_rooms, 1);
    assert_eq!(report.claimed_key_packages, 1);
    assert_eq!(report.prepared_commits, 1);
    assert_eq!(report.submitted_commits, 1);
    assert_eq!(report.completed_rooms, 1);
    assert!(report.complete);
    assert_eq!(report.applied_entries.len(), 1);
    assert_eq!(report.applied_entries[0].room_id, room_id);
    assert_eq!(report.applied_entries[0].seq, bob_join_seq + 1);
    assert!(!report.applied_entries[0].message_id.is_empty());
    assert_eq!(
        report.applied_entries[0].entry,
        AppliedLogEntry::Commit {
            sender: alice_browser.device_ref().clone(),
            epoch: 2,
        }
    );
    let LinkFanoutRoomStatus::Done { accepted_seq } = alice_browser
        .link_fanout_room_status("fanout_http_link_phone", room_id)
        .unwrap()
    else {
        panic!("HTTP fanout room did not complete");
    };
    assert_eq!(accepted_seq, bob_join_seq + 1);

    delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let projected_rooms = delivery
        .list_account_rooms(ListAccountRoomsRequest {
            account_id: account_id.clone(),
            after_room_id: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(projected_rooms.rooms.len(), 1);
    let projected_room = &projected_rooms.rooms[0];
    assert_eq!(projected_room.room_id, room_id);
    assert_eq!(projected_room.current_epoch, 2);
    assert_eq!(projected_room.last_seq, accepted_seq);
    assert!(
        projected_room
            .devices
            .iter()
            .any(|device| { device.device == *alice_phone.device_ref() && !device.active })
    );

    let bob_page = delivery
        .sync_events(room_id, bob.device_ref(), bob_join_seq)
        .unwrap();
    assert_eq!(bob_page.entries.len(), 1);
    assert_eq!(
        bob.apply_log_entry(room_id, &bob_page.entries[0]).unwrap(),
        AppliedLogEntry::Commit {
            sender: alice_browser.device_ref().clone(),
            epoch: 2,
        }
    );

    let mut alice_phone = phone_store.load_device(phone_config).unwrap();
    let phone_join = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_join.claimed_welcomes, 1);
    assert_eq!(phone_join.activated_welcome_acks_sent, 1);
    assert_eq!(alice_phone.group_epoch(room_id).unwrap(), 2);

    delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let activated_rooms = delivery
        .list_account_rooms(ListAccountRoomsRequest {
            account_id,
            after_room_id: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(activated_rooms.rooms.len(), 1);
    assert!(
        activated_rooms.rooms[0]
            .devices
            .iter()
            .any(|device| { device.device == *alice_phone.device_ref() && device.active })
    );
}

#[test]
fn reqwest_runtime_link_fanout_tick_links_later_device_over_live_server() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http-live-link.sqlite3");
    let server_url = spawn_live_http_server(&server_db);
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_live_link_fanout");
    let phone_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "phone_http_live_link_fanout");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut phone_store = sqlite_client_store(dir.path().join("phone.sqlite3"), &phone_config);
    let mut alice_browser = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut alice_phone = FiniteChatDevice::new(phone_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_live_link_fanout");
    let room_id = "room_http_live_link_fanout";
    let group_id = "mls_http_live_link_fanout";

    let mut delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.clone()));
    let bob_join_seq = create_group_room_with_member(
        &mut delivery,
        &mut alice_browser,
        &mut bob,
        GroupMemberSetup {
            room_id,
            mls_group_id: group_id,
            key_package_id: "kp_bob_http_live_link_fanout",
            welcome_id: "welcome_bob_http_live_link_fanout",
            idempotency_key: "add_bob_http_live_link_fanout",
        },
    );
    alice_store.save_device_state(&alice_browser).unwrap();
    alice_store
        .advance_room_cursor_and_save(&mut alice_browser, room_id, bob_join_seq)
        .unwrap();
    phone_store.save_device_state(&alice_phone).unwrap();

    let phone_replenish = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 1,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_replenish.uploaded_key_packages, 1);

    alice_store
        .start_link_fanout_and_save(
            &mut alice_browser,
            "fanout_http_live_link_phone",
            alice_phone.device_ref().clone(),
        )
        .unwrap();
    let report = run_link_fanout_tick(
        &mut alice_store,
        &mut alice_browser,
        &mut delivery,
        "fanout_http_live_link_phone",
        &RuntimeLinkFanoutOptions {
            max_discovery_pages_per_tick: 2,
            max_commit_rooms_per_tick: 1,
            max_completion_sync_pages_per_room: 2,
        },
    )
    .unwrap();
    assert_eq!(report.discovery_pages, 1);
    assert_eq!(report.queued_rooms, 1);
    assert_eq!(report.claimed_key_packages, 1);
    assert_eq!(report.prepared_commits, 1);
    assert_eq!(report.submitted_commits, 1);
    assert_eq!(report.completed_rooms, 1);
    assert!(report.complete);
    let LinkFanoutRoomStatus::Done { accepted_seq } = alice_browser
        .link_fanout_room_status("fanout_http_live_link_phone", room_id)
        .unwrap()
    else {
        panic!("live HTTP fanout room did not complete");
    };
    assert_eq!(accepted_seq, bob_join_seq + 1);

    let mut delivery = HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url));
    let bob_page = delivery
        .sync_events(room_id, bob.device_ref(), bob_join_seq)
        .unwrap();
    assert_eq!(bob_page.entries.len(), 1);
    assert_eq!(
        bob.apply_log_entry(room_id, &bob_page.entries[0]).unwrap(),
        AppliedLogEntry::Commit {
            sender: alice_browser.device_ref().clone(),
            epoch: 2,
        }
    );

    let mut alice_phone = phone_store.load_device(phone_config).unwrap();
    let phone_join = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_join.claimed_welcomes, 1);
    assert_eq!(phone_join.activated_welcome_acks_sent, 1);
    assert_eq!(alice_phone.group_epoch(room_id).unwrap(), 2);
}

#[test]
fn runtime_submit_commit_removes_account_room_over_finitechat_http_routes() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let mut world = active_alice_bob_charlie_room(&mut delivery);
    let charlie_ref = world.charlie.device_ref().clone();
    let charlie_account_id = charlie_ref.account_id.clone();

    let account_rooms = delivery
        .list_account_rooms(ListAccountRoomsRequest {
            account_id: charlie_account_id.clone(),
            after_room_id: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(account_rooms.rooms.len(), 1);
    assert!(
        account_rooms.rooms[0]
            .devices
            .iter()
            .any(|device| device.device == charlie_ref && device.active)
    );

    // Only an admin may remove another account's devices (ADR 0003 §2);
    // alice created the room, so she authors the removal.
    let prepared = world
        .alice
        .prepare_remove_member_commit(ROOM_ID, &charlie_ref, "alice_http_remove_charlie")
        .unwrap();
    let accepted = delivery.submit_commit(prepared.request).unwrap();
    assert_eq!(accepted.seq, world.last_seq + 1);
    assert_eq!(accepted.message_id, prepared.message_id);
    let bob_page = delivery
        .sync_events(ROOM_ID, world.bob.device_ref(), world.last_seq)
        .unwrap();
    assert_eq!(bob_page.entries.len(), 1);
    assert_eq!(bob_page.entries[0].seq, accepted.seq);
    assert_eq!(bob_page.entries[0].kind, LogEntryKind::Commit);

    delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let projected_rooms = delivery
        .list_account_rooms(ListAccountRoomsRequest {
            account_id: charlie_account_id,
            after_room_id: None,
            limit: 10,
        })
        .unwrap();
    assert!(projected_rooms.rooms.is_empty());
    assert_eq!(projected_rooms.next_after_room_id, None);
    assert!(!projected_rooms.has_more);
}

#[test]
fn runtime_link_fanout_retries_http_submit_response_loss_without_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_link_retry");
    let phone_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "phone_http_link_retry");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut phone_store = sqlite_client_store(dir.path().join("phone.sqlite3"), &phone_config);
    let mut alice_browser = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut alice_phone = FiniteChatDevice::new(phone_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_link_retry");
    let room_id = "room_http_link_retry";
    let group_id = "mls_http_link_retry";

    let mut setup_delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let bob_join_seq = create_group_room_with_member(
        &mut setup_delivery,
        &mut alice_browser,
        &mut bob,
        GroupMemberSetup {
            room_id,
            mls_group_id: group_id,
            key_package_id: "kp_bob_http_link_retry",
            welcome_id: "welcome_bob_http_link_retry",
            idempotency_key: "add_bob_http_link_retry",
        },
    );
    drop(setup_delivery);
    alice_store.save_device_state(&alice_browser).unwrap();
    alice_store
        .advance_room_cursor_and_save(&mut alice_browser, room_id, bob_join_seq)
        .unwrap();
    phone_store.save_device_state(&alice_phone).unwrap();

    let mut delivery = HttpRuntimeDelivery::with_submit_response_loss_from_sqlite_path(&server_db);
    let phone_replenish = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 1,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_replenish.uploaded_key_packages, 1);

    alice_store
        .start_link_fanout_and_save(
            &mut alice_browser,
            "fanout_http_retry_phone",
            alice_phone.device_ref().clone(),
        )
        .unwrap();
    let options = RuntimeLinkFanoutOptions {
        max_discovery_pages_per_tick: 2,
        max_commit_rooms_per_tick: 1,
        max_completion_sync_pages_per_room: 2,
    };
    let err = run_link_fanout_tick(
        &mut alice_store,
        &mut alice_browser,
        &mut delivery,
        "fanout_http_retry_phone",
        &options,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        RuntimeWorkerError::Delivery(HttpRuntimeDeliveryError::Transport(
            InProcessHttpTransportError::InjectedSubmitAfterAccept
        ))
    ));

    let mut alice_browser = alice_store.load_device(alice_config.clone()).unwrap();
    assert!(matches!(
        alice_browser
            .link_fanout_room_status("fanout_http_retry_phone", room_id)
            .unwrap(),
        LinkFanoutRoomStatus::Prepared { .. }
    ));
    let after_failure = delivery
        .sync_events(room_id, bob.device_ref(), bob_join_seq)
        .unwrap();
    assert_eq!(after_failure.entries.len(), 1);
    assert_eq!(after_failure.entries[0].seq, bob_join_seq + 1);
    assert_eq!(after_failure.entries[0].kind, LogEntryKind::Commit);

    let report = run_link_fanout_tick(
        &mut alice_store,
        &mut alice_browser,
        &mut delivery,
        "fanout_http_retry_phone",
        &options,
    )
    .unwrap();
    assert_eq!(report.discovery_pages, 0);
    assert_eq!(report.claimed_key_packages, 0);
    assert_eq!(report.prepared_commits, 0);
    assert_eq!(report.submitted_commits, 1);
    assert_eq!(report.completed_rooms, 1);
    assert!(report.complete);

    let after_retry = delivery
        .sync_events(room_id, bob.device_ref(), bob_join_seq)
        .unwrap();
    assert_eq!(after_retry.entries.len(), 1);
    assert_eq!(after_retry.entries[0].seq, bob_join_seq + 1);
    assert_eq!(
        bob.apply_log_entry(room_id, &after_retry.entries[0])
            .unwrap(),
        AppliedLogEntry::Commit {
            sender: alice_browser.device_ref().clone(),
            epoch: 2,
        }
    );

    let mut alice_phone = phone_store.load_device(phone_config).unwrap();
    let phone_join = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_join.claimed_welcomes, 1);
    assert_eq!(phone_join.activated_welcome_acks_sent, 1);
    assert_eq!(alice_phone.group_epoch(room_id).unwrap(), 2);
    let replay = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(replay.claimed_welcomes, 0);
    assert_eq!(replay.activated_welcome_acks_sent, 0);
}

#[test]
fn runtime_link_fanout_tick_links_multiple_rooms_over_finitechat_http_routes() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_multi_link");
    let phone_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "phone_http_multi_link");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut phone_store = sqlite_client_store(dir.path().join("phone.sqlite3"), &phone_config);
    let mut alice_browser = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut alice_phone = FiniteChatDevice::new(phone_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_multi_link");
    let mut dana = test_device(DANA_ACCOUNT_SECRET_BYTES, "dana_http_multi_link");
    let room_a = "room_http_multi_link_a";
    let group_a = "mls_http_multi_link_a";
    let room_b = "room_http_multi_link_b";
    let group_b = "mls_http_multi_link_b";

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let bob_join_seq = create_group_room_with_member(
        &mut delivery,
        &mut alice_browser,
        &mut bob,
        GroupMemberSetup {
            room_id: room_a,
            mls_group_id: group_a,
            key_package_id: "kp_bob_http_multi_link_a",
            welcome_id: "welcome_bob_http_multi_link_a",
            idempotency_key: "add_bob_http_multi_link_a",
        },
    );
    let dana_join_seq = create_group_room_with_member(
        &mut delivery,
        &mut alice_browser,
        &mut dana,
        GroupMemberSetup {
            room_id: room_b,
            mls_group_id: group_b,
            key_package_id: "kp_dana_http_multi_link_b",
            welcome_id: "welcome_dana_http_multi_link_b",
            idempotency_key: "add_dana_http_multi_link_b",
        },
    );
    alice_store.save_device_state(&alice_browser).unwrap();
    alice_store
        .advance_room_cursor_and_save(&mut alice_browser, room_a, bob_join_seq)
        .unwrap();
    alice_store
        .advance_room_cursor_and_save(&mut alice_browser, room_b, dana_join_seq)
        .unwrap();
    phone_store.save_device_state(&alice_phone).unwrap();

    let account_id = alice_browser.device_ref().account_id.clone();
    let account_rooms = delivery
        .list_account_rooms(ListAccountRoomsRequest {
            account_id: account_id.clone(),
            after_room_id: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(account_rooms.rooms.len(), 2);
    assert!(account_rooms.rooms.iter().all(|room| {
        !room
            .devices
            .iter()
            .any(|device| device.device == *alice_phone.device_ref())
    }));

    let phone_replenish = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 2,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_replenish.uploaded_key_packages, 2);

    alice_store
        .start_link_fanout_and_save(
            &mut alice_browser,
            "fanout_http_multi_phone",
            alice_phone.device_ref().clone(),
        )
        .unwrap();
    let report = run_link_fanout_tick(
        &mut alice_store,
        &mut alice_browser,
        &mut delivery,
        "fanout_http_multi_phone",
        &RuntimeLinkFanoutOptions {
            max_discovery_pages_per_tick: 4,
            max_commit_rooms_per_tick: 4,
            max_completion_sync_pages_per_room: 2,
        },
    )
    .unwrap();
    assert_eq!(report.discovery_pages, 2);
    assert_eq!(report.queued_rooms, 2);
    assert_eq!(report.claimed_key_packages, 2);
    assert_eq!(report.prepared_commits, 2);
    assert_eq!(report.submitted_commits, 2);
    assert_eq!(report.completed_rooms, 2);
    assert!(report.complete);

    let status_a = alice_browser
        .link_fanout_room_status("fanout_http_multi_phone", room_a)
        .unwrap();
    let LinkFanoutRoomStatus::Done {
        accepted_seq: accepted_a_seq,
    } = status_a
    else {
        panic!("HTTP multi-room fanout did not complete room a");
    };
    let status_b = alice_browser
        .link_fanout_room_status("fanout_http_multi_phone", room_b)
        .unwrap();
    let LinkFanoutRoomStatus::Done {
        accepted_seq: accepted_b_seq,
    } = status_b
    else {
        panic!("HTTP multi-room fanout did not complete room b");
    };
    assert_eq!(accepted_a_seq, bob_join_seq + 1);
    assert_eq!(accepted_b_seq, dana_join_seq + 1);

    let bob_page = delivery
        .sync_events(room_a, bob.device_ref(), bob_join_seq)
        .unwrap();
    assert_eq!(bob_page.entries.len(), 1);
    assert_eq!(
        bob.apply_log_entry(room_a, &bob_page.entries[0]).unwrap(),
        AppliedLogEntry::Commit {
            sender: alice_browser.device_ref().clone(),
            epoch: 2,
        }
    );
    let dana_page = delivery
        .sync_events(room_b, dana.device_ref(), dana_join_seq)
        .unwrap();
    assert_eq!(dana_page.entries.len(), 1);
    assert_eq!(
        dana.apply_log_entry(room_b, &dana_page.entries[0]).unwrap(),
        AppliedLogEntry::Commit {
            sender: alice_browser.device_ref().clone(),
            epoch: 2,
        }
    );

    let mut alice_phone = phone_store.load_device(phone_config).unwrap();
    let phone_join = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_join.claimed_welcomes, 2);
    assert_eq!(phone_join.activated_welcome_acks_sent, 2);
    assert_eq!(alice_phone.group_epoch(room_a).unwrap(), 2);
    assert_eq!(alice_phone.group_epoch(room_b).unwrap(), 2);
}

#[test]
fn runtime_link_fanout_retries_only_failed_room_over_finitechat_http_routes() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_partial_link");
    let phone_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "phone_http_partial_link");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut phone_store = sqlite_client_store(dir.path().join("phone.sqlite3"), &phone_config);
    let mut alice_browser = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut alice_phone = FiniteChatDevice::new(phone_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_partial_link");
    let mut dana = test_device(DANA_ACCOUNT_SECRET_BYTES, "dana_http_partial_link");
    let room_a = "room_http_partial_link_a";
    let group_a = "mls_http_partial_link_a";
    let room_b = "room_http_partial_link_b";
    let group_b = "mls_http_partial_link_b";

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let bob_join_seq = create_group_room_with_member(
        &mut delivery,
        &mut alice_browser,
        &mut bob,
        GroupMemberSetup {
            room_id: room_a,
            mls_group_id: group_a,
            key_package_id: "kp_bob_http_partial_link_a",
            welcome_id: "welcome_bob_http_partial_link_a",
            idempotency_key: "add_bob_http_partial_link_a",
        },
    );
    let dana_join_seq = create_group_room_with_member(
        &mut delivery,
        &mut alice_browser,
        &mut dana,
        GroupMemberSetup {
            room_id: room_b,
            mls_group_id: group_b,
            key_package_id: "kp_dana_http_partial_link_b",
            welcome_id: "welcome_dana_http_partial_link_b",
            idempotency_key: "add_dana_http_partial_link_b",
        },
    );
    alice_store.save_device_state(&alice_browser).unwrap();
    alice_store
        .advance_room_cursor_and_save(&mut alice_browser, room_a, bob_join_seq)
        .unwrap();
    alice_store
        .advance_room_cursor_and_save(&mut alice_browser, room_b, dana_join_seq)
        .unwrap();
    phone_store.save_device_state(&alice_phone).unwrap();

    let phone_replenish = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 2,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_replenish.uploaded_key_packages, 2);

    alice_store
        .start_link_fanout_and_save(
            &mut alice_browser,
            "fanout_http_partial_phone",
            alice_phone.device_ref().clone(),
        )
        .unwrap();
    let one_room_options = RuntimeLinkFanoutOptions {
        max_discovery_pages_per_tick: 4,
        max_commit_rooms_per_tick: 1,
        max_completion_sync_pages_per_room: 2,
    };

    let first_report = run_link_fanout_tick(
        &mut alice_store,
        &mut alice_browser,
        &mut delivery,
        "fanout_http_partial_phone",
        &one_room_options,
    )
    .unwrap();
    assert_eq!(first_report.discovery_pages, 2);
    assert_eq!(first_report.queued_rooms, 2);
    assert_eq!(first_report.submitted_commits, 1);
    assert_eq!(first_report.completed_rooms, 1);
    assert!(!first_report.complete);

    let status_a = alice_browser
        .link_fanout_room_status("fanout_http_partial_phone", room_a)
        .unwrap();
    let LinkFanoutRoomStatus::Done {
        accepted_seq: accepted_a_seq,
    } = status_a
    else {
        panic!("HTTP partial fanout did not complete room a");
    };
    assert_eq!(accepted_a_seq, bob_join_seq + 1);
    assert!(matches!(
        alice_browser
            .link_fanout_room_status("fanout_http_partial_phone", room_b)
            .unwrap(),
        LinkFanoutRoomStatus::Pending
    ));

    let mut delivery =
        HttpRuntimeDelivery::with_submit_before_accept_failure_from_sqlite_path(&server_db);
    let err = run_link_fanout_tick(
        &mut alice_store,
        &mut alice_browser,
        &mut delivery,
        "fanout_http_partial_phone",
        &one_room_options,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        RuntimeWorkerError::Delivery(HttpRuntimeDeliveryError::Transport(
            InProcessHttpTransportError::InjectedSubmitBeforeAccept
        ))
    ));

    let mut alice_browser = alice_store.load_device(alice_config.clone()).unwrap();
    assert!(matches!(
        alice_browser
            .link_fanout_room_status("fanout_http_partial_phone", room_a)
            .unwrap(),
        LinkFanoutRoomStatus::Done { .. }
    ));
    assert!(matches!(
        alice_browser
            .link_fanout_room_status("fanout_http_partial_phone", room_b)
            .unwrap(),
        LinkFanoutRoomStatus::Prepared { .. }
    ));

    let mut delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let retry_report = run_link_fanout_tick(
        &mut alice_store,
        &mut alice_browser,
        &mut delivery,
        "fanout_http_partial_phone",
        &one_room_options,
    )
    .unwrap();
    assert_eq!(retry_report.submitted_commits, 1);
    assert_eq!(retry_report.completed_rooms, 1);
    assert!(retry_report.complete);

    let LinkFanoutRoomStatus::Done {
        accepted_seq: retry_accepted_a_seq,
    } = alice_browser
        .link_fanout_room_status("fanout_http_partial_phone", room_a)
        .unwrap()
    else {
        panic!("room a lost Done status after room b retry");
    };
    assert_eq!(retry_accepted_a_seq, accepted_a_seq);
    let LinkFanoutRoomStatus::Done {
        accepted_seq: accepted_b_seq,
    } = alice_browser
        .link_fanout_room_status("fanout_http_partial_phone", room_b)
        .unwrap()
    else {
        panic!("room b fanout did not complete after retry");
    };
    assert_eq!(accepted_b_seq, dana_join_seq + 1);

    let room_a_page = delivery
        .sync_events(room_a, bob.device_ref(), bob_join_seq)
        .unwrap();
    assert_eq!(room_a_page.entries.len(), 1);
    assert_eq!(room_a_page.entries[0].seq, accepted_a_seq);
    let room_b_page = delivery
        .sync_events(room_b, dana.device_ref(), dana_join_seq)
        .unwrap();
    assert_eq!(room_b_page.entries.len(), 1);
    assert_eq!(room_b_page.entries[0].seq, accepted_b_seq);

    let mut alice_phone = phone_store.load_device(phone_config).unwrap();
    let phone_join = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_join.claimed_welcomes, 2);
    assert_eq!(phone_join.activated_welcome_acks_sent, 2);
    assert_eq!(alice_phone.group_epoch(room_a).unwrap(), 2);
    assert_eq!(alice_phone.group_epoch(room_b).unwrap(), 2);
}

#[test]
fn runtime_link_fanout_reprepares_after_http_same_epoch_loss() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("finitechat-http.sqlite3");
    let alice_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "alice_http_link_race");
    let phone_config = test_config(ALICE_ACCOUNT_SECRET_BYTES, "phone_http_link_race");
    let mut alice_store = sqlite_client_store(dir.path().join("alice.sqlite3"), &alice_config);
    let mut phone_store = sqlite_client_store(dir.path().join("phone.sqlite3"), &phone_config);
    let mut alice_browser = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut alice_phone = FiniteChatDevice::new(phone_config.clone()).unwrap();
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_http_link_race");
    let room_id = "room_http_link_race";
    let group_id = "mls_http_link_race";

    let mut setup_delivery = HttpRuntimeDelivery::from_sqlite_path(&server_db);
    let bob_join_seq = create_group_room_with_member(
        &mut setup_delivery,
        &mut alice_browser,
        &mut bob,
        GroupMemberSetup {
            room_id,
            mls_group_id: group_id,
            key_package_id: "kp_bob_http_link_race",
            welcome_id: "welcome_bob_http_link_race",
            idempotency_key: "add_bob_http_link_race",
        },
    );
    drop(setup_delivery);
    alice_store.save_device_state(&alice_browser).unwrap();
    alice_store
        .advance_room_cursor_and_save(&mut alice_browser, room_id, bob_join_seq)
        .unwrap();
    phone_store.save_device_state(&alice_phone).unwrap();

    let mut delivery =
        HttpRuntimeDelivery::with_submit_before_accept_failure_from_sqlite_path(&server_db);
    let phone_replenish = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 1,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_replenish.uploaded_key_packages, 1);

    alice_store
        .start_link_fanout_and_save(
            &mut alice_browser,
            "fanout_http_race_phone",
            alice_phone.device_ref().clone(),
        )
        .unwrap();
    let options = RuntimeLinkFanoutOptions {
        max_discovery_pages_per_tick: 2,
        max_commit_rooms_per_tick: 1,
        max_completion_sync_pages_per_room: 4,
    };
    let err = run_link_fanout_tick(
        &mut alice_store,
        &mut alice_browser,
        &mut delivery,
        "fanout_http_race_phone",
        &options,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        RuntimeWorkerError::Delivery(HttpRuntimeDeliveryError::Transport(
            InProcessHttpTransportError::InjectedSubmitBeforeAccept
        ))
    ));
    let mut alice_browser = alice_store.load_device(alice_config.clone()).unwrap();
    assert!(alice_browser.has_pending_commit(room_id).unwrap());
    assert!(matches!(
        alice_browser
            .link_fanout_room_status("fanout_http_race_phone", room_id)
            .unwrap(),
        LinkFanoutRoomStatus::Prepared { .. }
    ));

    let bob_winner = bob
        .prepare_self_update_commit(room_id, "bob_http_link_race_wins")
        .unwrap();
    let bob_winner_message_id = bob_winner.message_id.clone();
    let bob_accepted = delivery.submit_commit(bob_winner.request).unwrap();
    let bob_page = delivery
        .sync_events(room_id, bob.device_ref(), bob_join_seq)
        .unwrap();
    assert_eq!(bob_page.entries.len(), 1);
    bob.merge_pending_commit_from_log(room_id, &bob_page.entries, &bob_winner_message_id)
        .unwrap();

    let page = delivery
        .sync_events(room_id, alice_browser.device_ref(), bob_join_seq)
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(
        alice_store
            .apply_log_entry_and_save(&mut alice_browser, room_id, &page.entries[0])
            .unwrap(),
        Some(AppliedLogEntry::Commit {
            sender: bob.device_ref().clone(),
            epoch: 2,
        })
    );
    assert_eq!(bob_accepted.seq, bob_join_seq + 1);
    assert!(!alice_browser.has_pending_commit(room_id).unwrap());

    let report = run_link_fanout_tick(
        &mut alice_store,
        &mut alice_browser,
        &mut delivery,
        "fanout_http_race_phone",
        &options,
    )
    .unwrap();
    assert_eq!(report.claimed_key_packages, 0);
    assert_eq!(report.prepared_commits, 1);
    assert_eq!(report.submitted_commits, 1);
    assert_eq!(report.completed_rooms, 1);
    assert!(report.complete);
    let LinkFanoutRoomStatus::Done {
        accepted_seq: phone_add_seq,
    } = alice_browser
        .link_fanout_room_status("fanout_http_race_phone", room_id)
        .unwrap()
    else {
        panic!("HTTP same-epoch fanout did not complete");
    };
    assert_eq!(phone_add_seq, bob_accepted.seq + 1);

    let bob_after = delivery
        .sync_events(room_id, bob.device_ref(), bob_accepted.seq)
        .unwrap();
    assert_eq!(bob_after.entries.len(), 1);
    assert_eq!(
        bob.apply_log_entry(room_id, &bob_after.entries[0]).unwrap(),
        AppliedLogEntry::Commit {
            sender: alice_browser.device_ref().clone(),
            epoch: 3,
        }
    );

    let mut alice_phone = phone_store.load_device(phone_config).unwrap();
    let phone_join = run_runtime_sync_tick(
        &mut phone_store,
        &mut alice_phone,
        &mut delivery,
        &RuntimeSyncOptions {
            key_package_target_available: 0,
            max_sync_pages_per_room: 4,
        },
    )
    .unwrap();
    assert_eq!(phone_join.claimed_welcomes, 1);
    assert_eq!(phone_join.activated_welcome_acks_sent, 1);
    assert_eq!(alice_phone.group_epoch(room_id).unwrap(), 3);
}

fn test_device(
    account_secret_bytes: [u8; NOSTR_SECRET_KEY_BYTES],
    device_id: &str,
) -> FiniteChatDevice {
    FiniteChatDevice::new(test_config(account_secret_bytes, device_id)).unwrap()
}

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

fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
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

fn claim_and_activate<T: HttpRuntimeTransport>(
    delivery: &mut HttpRuntimeDelivery<T>,
    device: &mut FiniteChatDevice,
    welcome_id: &str,
) -> u64
where
    T::Error: std::fmt::Debug,
{
    claim_and_activate_room_record(delivery, device, ROOM_ID, welcome_id).commit_seq
}

fn claim_and_activate_room_record<T: HttpRuntimeTransport>(
    delivery: &mut HttpRuntimeDelivery<T>,
    device: &mut FiniteChatDevice,
    room_id: &str,
    welcome_id: &str,
) -> WelcomeRecord
where
    T::Error: std::fmt::Debug,
{
    let claimed_welcomes = delivery.claim_welcomes(device.device_ref()).unwrap();
    let welcome = claimed_welcomes
        .into_iter()
        .find(|welcome| welcome.welcome_id == welcome_id)
        .unwrap();
    device
        .activate_welcome(
            room_id,
            &welcome.welcome_payload,
            &welcome.ratchet_tree_payload,
        )
        .unwrap();
    delivery.ack_welcome(welcome_id).unwrap();
    welcome
}

struct GroupMemberSetup<'a> {
    room_id: &'a str,
    mls_group_id: &'a str,
    key_package_id: &'a str,
    welcome_id: &'a str,
    idempotency_key: &'a str,
}

/// Bootstrap a typed room over the Finite Chat HTTP routes and add one member:
/// typed account-room bootstrap, KeyPackage upload/claim, typed `/commits`
/// submit (which releases the Welcome server-side), creator merge from the
/// ordered log, and member Welcome claim/activate/ack. Returns the member's
/// join sequence.
fn create_group_room_with_member<T: HttpRuntimeTransport>(
    delivery: &mut HttpRuntimeDelivery<T>,
    alice: &mut FiniteChatDevice,
    member: &mut FiniteChatDevice,
    setup: GroupMemberSetup<'_>,
) -> u64
where
    T::Error: std::fmt::Debug,
{
    alice
        .create_group_state(setup.room_id, setup.mls_group_id)
        .unwrap();
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: setup.room_id.to_string(),
            mls_group_id: setup.mls_group_id.to_string(),
            creator: alice.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    delivery
        .upload_key_package(
            member
                .upload_key_package_request(setup.key_package_id)
                .unwrap(),
        )
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(member.device_ref())
        .unwrap()
        .expect("member key package");
    let prepared = alice
        .prepare_add_member_commit(
            setup.room_id,
            &claimed_key_package,
            setup.welcome_id,
            setup.idempotency_key,
        )
        .unwrap();
    let accepted = delivery.submit_commit(prepared.request).unwrap();
    let alice_page = delivery
        .sync_events(setup.room_id, alice.device_ref(), 0)
        .unwrap();
    alice
        .merge_pending_commit_from_log(setup.room_id, &alice_page.entries, &prepared.message_id)
        .unwrap();
    let welcome = claim_and_activate_room_record(delivery, member, setup.room_id, setup.welcome_id);
    assert_eq!(welcome.commit_seq, accepted.seq);
    assert_eq!(alice.group_epoch(setup.room_id).unwrap(), 1);
    assert_eq!(member.group_epoch(setup.room_id).unwrap(), 1);
    accepted.seq
}

struct ActiveThreeMemberRoom {
    alice: FiniteChatDevice,
    bob: FiniteChatDevice,
    charlie: FiniteChatDevice,
    last_seq: u64,
}

fn active_alice_bob_charlie_room(delivery: &mut TestHttpRuntimeDelivery) -> ActiveThreeMemberRoom {
    let mut alice = test_device(ALICE_ACCOUNT_SECRET_BYTES, "alice_browser");
    let mut bob = test_device(BOB_ACCOUNT_SECRET_BYTES, "bob_runtime");
    let bob_join_seq = create_group_room_with_member(
        delivery,
        &mut alice,
        &mut bob,
        GroupMemberSetup {
            room_id: ROOM_ID,
            mls_group_id: MLS_GROUP_ID,
            key_package_id: BOB_KEY_PACKAGE_ID,
            welcome_id: BOB_WELCOME_ID,
            idempotency_key: "activate_bob_helper",
        },
    );
    let mut charlie = test_device(CHARLIE_ACCOUNT_SECRET_BYTES, "charlie_phone");

    delivery
        .upload_key_package(
            charlie
                .upload_key_package_request("kp_active_charlie_1")
                .unwrap(),
        )
        .unwrap();
    let claimed_key_package = delivery
        .claim_key_package_for_device(charlie.device_ref())
        .unwrap()
        .expect("charlie key package");
    let prepared = alice
        .prepare_add_member_commit(
            ROOM_ID,
            &claimed_key_package,
            "welcome_active_charlie_1",
            "alice_add_active_charlie",
        )
        .unwrap();
    let accepted = delivery.submit_commit(prepared.request).unwrap();
    assert_eq!(
        apply_one_commit(delivery, &mut alice, bob_join_seq),
        AppliedLogEntry::Commit {
            sender: alice.device_ref().clone(),
            epoch: 2,
        }
    );
    assert_eq!(
        apply_one_commit(delivery, &mut bob, bob_join_seq),
        AppliedLogEntry::Commit {
            sender: alice.device_ref().clone(),
            epoch: 2,
        }
    );
    let charlie_join_seq = claim_and_activate(delivery, &mut charlie, "welcome_active_charlie_1");
    assert_eq!(charlie_join_seq, accepted.seq);
    assert_eq!(alice.group_epoch(ROOM_ID).unwrap(), 2);
    assert_eq!(bob.group_epoch(ROOM_ID).unwrap(), 2);
    assert_eq!(charlie.group_epoch(ROOM_ID).unwrap(), 2);

    ActiveThreeMemberRoom {
        alice,
        bob,
        charlie,
        last_seq: accepted.seq,
    }
}

fn apply_one_commit(
    delivery: &mut TestHttpRuntimeDelivery,
    device: &mut FiniteChatDevice,
    after_seq: u64,
) -> AppliedLogEntry {
    let page = delivery
        .sync_events(ROOM_ID, device.device_ref(), after_seq)
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].kind, LogEntryKind::Commit);
    device.apply_log_entry(ROOM_ID, &page.entries[0]).unwrap()
}

#[derive(Debug)]
struct SentPlaintext {
    seq: u64,
    plaintext: Vec<u8>,
}

fn send_bob_messages<T: HttpRuntimeTransport>(
    delivery: &mut HttpRuntimeDelivery<T>,
    bob: &mut FiniteChatDevice,
    scenario_index: usize,
    next_message_index: &mut usize,
    count: usize,
    sent_plaintexts: &mut Vec<SentPlaintext>,
) where
    T::Error: std::fmt::Debug,
{
    for _ in 0..count {
        *next_message_index += 1;
        let plaintext = format!(
            r#"{{"type":"finitecomputer.command.v1","body":{{"scenario":{scenario_index},"message":{}}}}}"#,
            *next_message_index
        )
        .into_bytes();
        let request = bob
            .create_application_request(
                ROOM_ID,
                &plaintext,
                format!("bob_msg_{scenario_index}_{}", *next_message_index),
            )
            .unwrap();
        let accepted = delivery
            .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
            .unwrap();
        assert_application_acceptance(&accepted, sent_plaintexts);
        sent_plaintexts.push(SentPlaintext {
            seq: accepted.seq,
            plaintext,
        });
    }
}

#[derive(Debug, PartialEq, Eq)]
enum InProcessHttpTransportError {
    Json(String),
    HttpStatus(StatusCode, String),
    Router(String),
    InjectedSubmitBeforeAccept,
    InjectedSubmitAfterAccept,
    InjectedKeyPackageInventory,
}

impl std::fmt::Display for InProcessHttpTransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "JSON error: {error}"),
            Self::HttpStatus(status, body) => {
                write!(formatter, "HTTP status {status}: {body}")
            }
            Self::Router(error) => write!(formatter, "router error: {error}"),
            Self::InjectedSubmitBeforeAccept => write!(formatter, "injected submit before accept"),
            Self::InjectedSubmitAfterAccept => write!(formatter, "injected submit after accept"),
            Self::InjectedKeyPackageInventory => {
                write!(formatter, "injected KeyPackage inventory failure")
            }
        }
    }
}

type TestHttpRuntimeDelivery = HttpRuntimeDelivery<InProcessHttpTransport>;

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
    wait_for_live_http_server(&server_url);
    server_url
}

fn wait_for_live_http_server(server_url: &str) {
    let health_url = format!("{}/health", server_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::new();
    for _ in 0..100 {
        if client
            .get(&health_url)
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("live HTTP test server did not become healthy at {health_url}");
}

struct InProcessHttpTransport {
    app: Router,
    runtime: tokio::runtime::Runtime,
    fail_next_submit_before_accept: bool,
    fail_next_submit_after_accept: bool,
    fail_key_package_inventory: bool,
}

impl InProcessHttpTransport {
    fn from_sqlite_path(path: &std::path::Path) -> Self {
        Self {
            app: http_router(HttpServerState::from_sqlite_path(path).unwrap()),
            runtime: tokio::runtime::Runtime::new().unwrap(),
            fail_next_submit_before_accept: false,
            fail_next_submit_after_accept: false,
            fail_key_package_inventory: false,
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
        if uri == "/commits" && self.fail_next_submit_before_accept {
            self.fail_next_submit_before_accept = false;
            return Err(InProcessHttpTransportError::InjectedSubmitBeforeAccept);
        }
        if self.fail_key_package_inventory
            && matches!(
                uri,
                "/key-packages" | "/key-packages/claim" | "/key-packages/claims"
            )
        {
            return Err(InProcessHttpTransportError::InjectedKeyPackageInventory);
        }
        let result = self.runtime.block_on(async {
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
        });
        if uri == "/commits" && self.fail_next_submit_after_accept {
            self.fail_next_submit_after_accept = false;
            result?;
            return Err(InProcessHttpTransportError::InjectedSubmitAfterAccept);
        }
        result
    }
}

trait TestHttpRuntimeDeliveryExt {
    fn from_sqlite_path(path: &std::path::Path) -> Self;
    fn with_submit_before_accept_failure_from_sqlite_path(path: &std::path::Path) -> Self;
    fn with_submit_response_loss_from_sqlite_path(path: &std::path::Path) -> Self;
}

impl TestHttpRuntimeDeliveryExt for TestHttpRuntimeDelivery {
    fn from_sqlite_path(path: &std::path::Path) -> Self {
        Self::new(InProcessHttpTransport::from_sqlite_path(path))
    }

    fn with_submit_before_accept_failure_from_sqlite_path(path: &std::path::Path) -> Self {
        let mut transport = InProcessHttpTransport::from_sqlite_path(path);
        transport.fail_next_submit_before_accept = true;
        Self::new(transport)
    }

    fn with_submit_response_loss_from_sqlite_path(path: &std::path::Path) -> Self {
        let mut transport = InProcessHttpTransport::from_sqlite_path(path);
        transport.fail_next_submit_after_accept = true;
        Self::new(transport)
    }
}

fn assert_application_acceptance(accepted: &EventAccepted, sent_plaintexts: &[SentPlaintext]) {
    let expected_seq = sent_plaintexts
        .last()
        .map(|message| message.seq + 1)
        .unwrap_or(2);
    assert_eq!(accepted.seq, expected_seq);
}
