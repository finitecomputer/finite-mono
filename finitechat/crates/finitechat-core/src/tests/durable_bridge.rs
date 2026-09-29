use super::*;

#[test]
fn durable_bridge_does_not_adopt_a_reply_into_a_rewound_device() {
    let dir = tempfile::tempdir().unwrap();
    let server = spawn_live_http_server(dir.path().join("server.sqlite3"));
    let home = dir.path().join("sender");
    let stale_home = dir.path().join("stale");
    let mut state = open_app_runtime_state_with_account(&home, &server, "sender", "bridge-test");
    state.create_room("Bridge".into()).unwrap();
    let room = state.app.rooms[0].room_id.clone();
    copy_client_store(&home, &stale_home);
    let payload = encode_application_event(
        DurableAppEventKind::ChatMessage,
        None,
        br#"{"text":"durable refusal"}"#,
    )
    .unwrap();
    let prepared = state.prepare_bridge_reply(room.clone(), payload).unwrap();
    let mut unsent_stale =
        open_app_runtime_state_with_account(&stale_home, &server, "sender", "bridge-test");
    let refused = unsent_stale.submit_bridge_reply(&prepared).unwrap_err();
    assert!(refused.to_string().contains("not minted"), "{refused}");
    drop(unsent_stale);
    state.submit_bridge_reply(&prepared).unwrap();
    let mut stale =
        open_app_runtime_state_with_account(&stale_home, &server, "sender", "bridge-test");
    let result = stale.submit_bridge_reply(&prepared);
    assert!(
        matches!(
            result,
            Err(FiniteChatCoreError::DeviceStateBehindServer { .. })
        ),
        "{result:?}"
    );
}

#[test]
fn durable_bridge_reopens_prepared_request_and_recovers_accepted_reply_once() {
    let dir = tempfile::tempdir().unwrap();
    let server = spawn_live_http_server(dir.path().join("server.sqlite3"));
    let home = dir.path().join("sender");
    let open = || open_app_runtime_state_with_account(&home, &server, "sender", "bridge-test");
    let mut state = open();
    state.create_room("Bridge".into()).unwrap();
    let room = state.app.rooms[0].room_id.clone();
    let baseline = room_application_entry_count(&state, &room);
    let payload = encode_application_event(
        DurableAppEventKind::ChatMessage,
        None,
        br#"{"text":"durable refusal"}"#,
    )
    .unwrap();
    let prepared = state.prepare_bridge_reply(room.clone(), payload).unwrap();
    let journal = serde_json::to_vec(&prepared).unwrap();
    assert_eq!(room_application_entry_count(&state, &room), baseline);
    drop(state);

    let mut state = open();
    let prepared: PreparedBridgeReply = serde_json::from_slice(&journal).unwrap();
    let sent = state.submit_bridge_reply(&prepared).unwrap();
    assert_eq!(room_application_entry_count(&state, &room), baseline + 1);
    drop(state); // Crash before bridge records acceptance/settles its inbox.

    let mut state = open();
    // Advance the epoch: a saved request replay is now too old, so acceptance
    // must be recovered through the existing application effect receipt.
    state.rekey_room(room.clone()).unwrap();
    let replay = state
        .submit_bridge_reply(&serde_json::from_slice(&journal).unwrap())
        .unwrap();
    assert_eq!(sent.message_id, replay.message_id);
    assert_eq!(sent.seq, replay.seq);
    assert_eq!(room_application_entry_count(&state, &room), baseline + 1);
    assert_eq!(
        state
            .app
            .messages
            .iter()
            .filter(|m| m.message_id == sent.message_id)
            .count(),
        1
    );
}

#[test]
fn durable_bridge_rejects_foreign_device_and_unknown_saved_version() {
    let dir = tempfile::tempdir().unwrap();
    let server = spawn_live_http_server(dir.path().join("server.sqlite3"));
    let mut state = open_app_runtime_state_with_account(
        dir.path().join("sender"),
        &server,
        "sender",
        "bridge-test",
    );
    state.create_room("Bridge".into()).unwrap();
    let room = state.app.rooms[0].room_id.clone();
    let payload = encode_application_event(
        DurableAppEventKind::ChatMessage,
        None,
        br#"{"text":"durable refusal"}"#,
    )
    .unwrap();
    let prepared = state.prepare_bridge_reply(room.clone(), payload).unwrap();
    let mut foreign = open_app_runtime_state_with_account(
        dir.path().join("foreign"),
        &server,
        "foreign",
        "other-account",
    );
    assert!(foreign.submit_bridge_reply(&prepared).is_err());
    let mut future = serde_json::to_value(prepared).unwrap();
    future["version"] = 3.into();
    assert!(
        state
            .submit_bridge_reply(&serde_json::from_value(future).unwrap())
            .is_err()
    );
}

fn bridge_room_with_receiver(dir: &Path) -> (AppRuntimeState, AppRuntimeState, String) {
    let server = spawn_live_http_server(dir.join("server.sqlite3"));
    bridge_room_on(dir, &server)
}

fn bridge_room_on(dir: &Path, server: &str) -> (AppRuntimeState, AppRuntimeState, String) {
    let mut sender =
        open_app_runtime_state_with_account(dir.join("sender"), server, "sender", "bridge-test");
    let mut receiver = open_app_runtime_state_with_account(
        dir.join("receiver"),
        server,
        "receiver",
        "receiver-account",
    );
    sender.create_room("Bridge".into()).unwrap();
    let room = sender.app.rooms[0].room_id.clone();
    receiver.start_runtime().unwrap();
    let account_id = receiver.app.identity.account_id.clone();
    sender
        .add_room_members(room.clone(), vec![test_profile(&account_id, "Receiver")])
        .unwrap();
    receiver.start_runtime().unwrap();
    assert!(receiver.room_is_connected(&room));
    (sender, receiver, room)
}

fn refusal_payload(text: &str) -> Vec<u8> {
    encode_application_event(DurableAppEventKind::ChatMessage, None, text.as_bytes()).unwrap()
}

fn minted_ids(state: &AppRuntimeState, room_id: &str) -> Vec<String> {
    state
        .core
        .device
        .export_state()
        .unwrap()
        .rooms
        .into_iter()
        .find(|room| room.room_id == room_id)
        .unwrap()
        .unacknowledged_own_message_ids
}

fn saved_message_id(prepared: &PreparedBridgeReply) -> String {
    serde_json::to_value(prepared).unwrap()["message"]["message_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn sees(state: &AppRuntimeState, text: &str) -> bool {
    state
        .app
        .messages
        .iter()
        .any(|message| message.text == text)
}

#[test]
fn durable_bridge_never_replays_a_reply_overtaken_by_later_own_sends() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, mut receiver, room) = bridge_room_with_receiver(dir.path());
    let accepted = sender
        .prepare_bridge_reply(room.clone(), refusal_payload("accepted refusal"))
        .unwrap();
    let sent = sender.submit_bridge_reply(&accepted).unwrap();
    // Submit fails transiently after prepare; the agent keeps talking. Each
    // accepted send leaves the minted list, so the saved id is last again.
    let stale = sender
        .prepare_bridge_reply(room.clone(), refusal_payload("stale refusal"))
        .unwrap();
    for n in 0..6 {
        sender
            .send_message(room.clone(), format!("ordinary {n}"), None, None)
            .unwrap();
    }
    receiver.start_runtime().unwrap();
    assert!(sees(&receiver, "ordinary 5"));
    // The minted list alone cannot see accepted later sends.
    assert_eq!(
        minted_ids(&sender, &room).last(),
        Some(&saved_message_id(&stale))
    );

    // Positive-effect recovery is unaffected by later sends.
    let recovered = sender.submit_bridge_reply(&accepted).unwrap();
    assert_eq!(
        (recovered.message_id, recovered.seq),
        (sent.message_id, sent.seq)
    );

    let baseline = room_application_entry_count(&sender, &room);
    let replay = sender.submit_bridge_reply(&stale);
    sender
        .send_message(room.clone(), "after refusal".into(), None, None)
        .unwrap();
    receiver.start_runtime().unwrap();
    assert!(
        sees(&receiver, "after refusal"),
        "receiver room froze behind the replayed ciphertext (replay: {replay:?})"
    );
    let refused = replay.unwrap_err();
    assert!(refused.to_string().contains("overtaken"), "{refused}");
    assert_eq!(room_application_entry_count(&sender, &room), baseline + 1);
    assert!(!sees(&receiver, "stale refusal"));
}

#[test]
fn durable_bridge_never_replays_a_reply_minted_before_a_failed_later_send() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, mut receiver, room) = bridge_room_with_receiver(dir.path());
    let stale = sender
        .prepare_bridge_reply(room.clone(), refusal_payload("stale refusal"))
        .unwrap();
    // A later send that minted its generation and then failed to append
    // leaves exactly this durable state: minted, remembered, unaccepted.
    let now = sender.core.now_unix_seconds().unwrap();
    sender
        .core
        .device
        .create_application_request_at(&room, &refusal_payload("failed"), "msg-failed", now)
        .unwrap();
    sender
        .core
        .store
        .save_device_state(&sender.core.device)
        .unwrap();

    let baseline = room_application_entry_count(&sender, &room);
    let refused = sender.submit_bridge_reply(&stale).unwrap_err();
    assert!(refused.to_string().contains("overtaken"), "{refused}");
    assert_eq!(room_application_entry_count(&sender, &room), baseline);
    // The refusal is retained by its owner; ordinary chat keeps flowing.
    assert!(minted_ids(&sender, &room).contains(&saved_message_id(&stale)));
    sender
        .send_message(room.clone(), "after failed send".into(), None, None)
        .unwrap();
    receiver.start_runtime().unwrap();
    assert!(sees(&receiver, "after failed send"));
    assert!(!sees(&receiver, "stale refusal"));
}

#[test]
fn durable_bridge_never_replays_a_reply_across_an_epoch_change() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, mut receiver, room) = bridge_room_with_receiver(dir.path());
    let stale = sender
        .prepare_bridge_reply(room.clone(), refusal_payload("stale refusal"))
        .unwrap();
    sender.rekey_room(room.clone()).unwrap();
    let baseline = room_application_entry_count(&sender, &room);
    let refused = sender.submit_bridge_reply(&stale).unwrap_err();
    assert!(
        refused.to_string().contains("previous room epoch"),
        "{refused}"
    );
    assert_eq!(room_application_entry_count(&sender, &room), baseline);
    sender
        .send_message(room.clone(), "after rekey".into(), None, None)
        .unwrap();
    receiver.start_runtime().unwrap();
    assert!(sees(&receiver, "after rekey"));
    assert!(!sees(&receiver, "stale refusal"));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyncFault {
    None,
    /// Every room page fetch fails with a delivery error.
    Fail,
    /// The next room page fetch claims the head at the requested cursor.
    HideNext,
}

/// Live HTTP server whose room page route (`/sync/group`) can fail or lie,
/// so a test can stand in for a swallowed delivery error or a bounded sync
/// that stopped short of the head.
fn spawn_sync_fault_server(path: &Path) -> (String, Arc<Mutex<SyncFault>>) {
    use axum::extract::Request;
    use axum::middleware::Next;
    use axum::response::IntoResponse;
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let fault = Arc::new(Mutex::new(SyncFault::None));
    let shared = fault.clone();
    let app = http_router(HttpServerState::from_sqlite_path(path).unwrap()).layer(
        axum::middleware::from_fn(move |request: Request, next: Next| {
            let shared = shared.clone();
            async move {
                if request.uri().path() != "/sync/group" {
                    return next.run(request).await;
                }
                let mode = {
                    let mut fault = shared.lock().unwrap();
                    let mode = *fault;
                    if mode == SyncFault::HideNext {
                        *fault = SyncFault::None;
                    }
                    mode
                };
                match mode {
                    SyncFault::None => next.run(request).await,
                    SyncFault::Fail => axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
                    SyncFault::HideNext => {
                        let body = axum::body::to_bytes(request.into_body(), usize::MAX)
                            .await
                            .unwrap();
                        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        axum::Json(serde_json::json!({
                            "entries": [],
                            "next_after_seq": request["after_seq"],
                            "has_more": false,
                        }))
                        .into_response()
                    }
                }
            }
        }),
    );
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    let server_url = format!("http://{addr}");
    wait_for_live_http_server(&server_url);
    (server_url, fault)
}

#[test]
fn durable_bridge_requires_a_successful_head_sync_before_recovery_or_replay() {
    let dir = tempfile::tempdir().unwrap();
    let (server, fault) = spawn_sync_fault_server(&dir.path().join("server.sqlite3"));
    let (mut sender, mut receiver, room) = bridge_room_on(dir.path(), &server);
    let accepted = sender
        .prepare_bridge_reply(room.clone(), refusal_payload("accepted refusal"))
        .unwrap();
    let sent = sender.submit_bridge_reply(&accepted).unwrap();
    let pending = sender
        .prepare_bridge_reply(room.clone(), refusal_payload("pending refusal"))
        .unwrap();
    let baseline = room_application_entry_count(&sender, &room);

    // The room is verified in this process, so the ordinary send gate would
    // tolerate this; the bridge must not.
    *fault.lock().unwrap() = SyncFault::Fail;
    let recovery = sender.submit_bridge_reply(&accepted).unwrap_err();
    assert!(
        matches!(recovery, FiniteChatCoreError::Delivery { .. }),
        "{recovery:?}"
    );
    let replay = sender.submit_bridge_reply(&pending).unwrap_err();
    assert!(
        matches!(replay, FiniteChatCoreError::Delivery { .. }),
        "{replay:?}"
    );
    *fault.lock().unwrap() = SyncFault::None;
    assert_eq!(room_application_entry_count(&sender, &room), baseline);

    let recovered = sender.submit_bridge_reply(&accepted).unwrap();
    assert_eq!(
        (recovered.message_id, recovered.seq),
        (sent.message_id, sent.seq)
    );
    sender.submit_bridge_reply(&pending).unwrap();

    // Ordinary sends keep their existing tolerance.
    *fault.lock().unwrap() = SyncFault::Fail;
    sender
        .send_message(
            room.clone(),
            "ordinary during sync outage".into(),
            None,
            None,
        )
        .unwrap();
    *fault.lock().unwrap() = SyncFault::None;
    receiver.start_runtime().unwrap();
    assert!(sees(&receiver, "pending refusal"));
    assert!(sees(&receiver, "ordinary during sync outage"));
}

#[test]
fn durable_bridge_fails_closed_when_the_head_probe_finds_unseen_entries() {
    let dir = tempfile::tempdir().unwrap();
    let (server, fault) = spawn_sync_fault_server(&dir.path().join("server.sqlite3"));
    let (mut sender, mut receiver, room) = bridge_room_on(dir.path(), &server);
    let pending = sender
        .prepare_bridge_reply(room.clone(), refusal_payload("pending refusal"))
        .unwrap();
    receiver
        .send_message(room.clone(), "unseen by sender".into(), None, None)
        .unwrap();
    let baseline = room_application_entry_count(&sender, &room);

    // The sync is told it reached the head; the probe sees the real log.
    *fault.lock().unwrap() = SyncFault::HideNext;
    let refused = sender.submit_bridge_reply(&pending).unwrap_err();
    assert!(refused.to_string().contains("server head"), "{refused}");
    assert_eq!(*fault.lock().unwrap(), SyncFault::None);
    assert_eq!(room_application_entry_count(&sender, &room), baseline);

    // Retry syncs past the entry and converges.
    sender.submit_bridge_reply(&pending).unwrap();
    assert!(sees(&sender, "unseen by sender"));
    receiver.start_runtime().unwrap();
    assert!(sees(&receiver, "pending refusal"));
}

#[test]
fn durable_bridge_rejects_version_1_wrappers_outright() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, mut receiver, room) = bridge_room_with_receiver(dir.path());
    let as_v1 = |prepared: &PreparedBridgeReply| -> PreparedBridgeReply {
        let mut value = serde_json::to_value(prepared).unwrap();
        value["version"] = 1.into();
        serde_json::from_value(value).unwrap()
    };
    let accepted = sender
        .prepare_bridge_reply(room.clone(), refusal_payload("accepted refusal"))
        .unwrap();
    sender.submit_bridge_reply(&accepted).unwrap();
    // No receipt recovery for v1 either.
    let refused = sender.submit_bridge_reply(&as_v1(&accepted)).unwrap_err();
    assert!(refused.to_string().contains("version"), "{refused}");
    // The pre-fence v1 shape has no mark and does not decode at all.
    let mut pre_fence = serde_json::to_value(&accepted).unwrap();
    pre_fence["version"] = 1.into();
    pre_fence
        .as_object_mut()
        .unwrap()
        .remove("own_send_high_water_seq")
        .unwrap();
    assert!(serde_json::from_value::<PreparedBridgeReply>(pre_fence).is_err());

    let pending = sender
        .prepare_bridge_reply(room.clone(), refusal_payload("pending refusal"))
        .unwrap();
    // The Runner's handoff checker agrees with this writer's real output.
    use finitechat_hermes::inbox_compatibility::{REFUSAL_V2_READER, check};
    let inbox = serde_json::to_vec(&serde_json::json!({"events": [{"lease": {
        "state": "refusal_v1",
        "operation": {"prepared": pending},
    }}]}))
    .unwrap();
    assert_eq!(check(&inbox, Some(REFUSAL_V2_READER)), Ok(()));
    assert!(check(&inbox, Some("refusal-v1")).is_err());
    let baseline = room_application_entry_count(&sender, &room);
    let refused = sender.submit_bridge_reply(&as_v1(&pending)).unwrap_err();
    assert!(refused.to_string().contains("version"), "{refused}");
    assert_eq!(room_application_entry_count(&sender, &room), baseline);
    sender.submit_bridge_reply(&pending).unwrap();
    receiver.start_runtime().unwrap();
    assert!(sees(&receiver, "pending refusal"));
}

/// Frozen persisted shape of the v2 wrapper. Any drift must bump the wrapper
/// version and the inbox reader label together.
const PREPARED_BRIDGE_REPLY_V2_FIXTURE: &str = r#"{
  "version": 2,
  "server_url": "http://127.0.0.1:8080",
  "message": {
    "room_id": "room-d0f0cf0ae1778eaf",
    "message_id": "f37806932a72d63daed38ff528aa1e9f167453a4d7cdafd4b3bb04ce6aebd6e2",
    "sender": {
      "account_id": "02af2b06347a2e2d8bf0e5efd2de986c80c14a50af7cbb7304cea86627d32978",
      "device_id": "sender"
    },
    "plaintext": [104, 105],
    "timestamp_unix_seconds": 1800000000,
    "append_request": {
      "room_id": "room-d0f0cf0ae1778eaf",
      "sender": {
        "account_id": "02af2b06347a2e2d8bf0e5efd2de986c80c14a50af7cbb7304cea86627d32978",
        "device_id": "sender"
      },
      "envelope": {
        "room_id": "room-d0f0cf0ae1778eaf",
        "mls_group_id": "mls-54fb3589403a876f",
        "epoch": 1,
        "sender": {
          "account_id": "02af2b06347a2e2d8bf0e5efd2de986c80c14a50af7cbb7304cea86627d32978",
          "device_id": "sender"
        },
        "kind": "application",
        "payload": "AAEC"
      },
      "idempotency_key": "msg-c238e21ed9621b13",
      "timestamp_unix_seconds": 1800000000
    }
  },
  "own_send_high_water_seq": 0
}"#;

#[test]
fn durable_bridge_v2_wrapper_shape_is_frozen() {
    let fixture: serde_json::Value =
        serde_json::from_str(PREPARED_BRIDGE_REPLY_V2_FIXTURE).unwrap();
    let decoded: PreparedBridgeReply = serde_json::from_value(fixture.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), fixture);

    let mut unknown = fixture.clone();
    unknown["replay_hint"] = true.into();
    assert!(serde_json::from_value::<PreparedBridgeReply>(unknown).is_err());
    let mut missing_mark = fixture;
    missing_mark
        .as_object_mut()
        .unwrap()
        .remove("own_send_high_water_seq")
        .unwrap();
    assert!(serde_json::from_value::<PreparedBridgeReply>(missing_mark).is_err());
}
