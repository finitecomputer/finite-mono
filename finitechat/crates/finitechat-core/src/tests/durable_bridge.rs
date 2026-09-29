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
    future["version"] = 2.into();
    assert!(
        state
            .submit_bridge_reply(&serde_json::from_value(future).unwrap())
            .is_err()
    );
}
