use super::*;

fn placement<'a>(state: &'a AppState, topic: &str, chat: &str) -> &'a AppChatPlacement {
    state
        .topics
        .iter()
        .find(|t| t.topic_id == topic)
        .unwrap()
        .chats
        .iter()
        .find(|c| c.chat_id == chat)
        .unwrap()
        .placement
        .as_ref()
        .unwrap()
}

#[test]
fn chat_organization_preserves_routes_and_replays_on_restart_and_another_device() {
    let dir = tempfile::tempdir().unwrap();
    let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
    let alice_dir = dir.path().join("alice");
    let options = |data_dir: &Path, device: &str, url: &str| {
        with_test_secret(OpenOptions {
            data_dir: data_dir.to_string_lossy().into_owned(),
            server_url: url.to_owned(),
            device_id: device.to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        })
    };
    let alice = FiniteChatRuntime::open(options(&alice_dir, "alice", &server_url)).unwrap();
    let bob =
        FiniteChatRuntime::open(options(&dir.path().join("bob"), "bob", &server_url)).unwrap();
    let created = alice
        .dispatch_and_wait(AppAction::CreateRoom {
            display_name: "Organization".into(),
        })
        .unwrap();
    let room = created.selected_room_id.unwrap();
    add_runtime_member_named(&alice, &bob, &room, "Bob");
    bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    let first = alice
        .dispatch_and_wait(AppAction::CreateTopic {
            room_id: room.clone(),
            title: "Build".into(),
        })
        .unwrap();
    let topic = first.selected_topic_id.unwrap();
    let chat = first.selected_chat_id.unwrap();
    let second = alice
        .dispatch_and_wait(AppAction::StartTopicChat {
            room_id: room.clone(),
            topic_id: topic.clone(),
            reason: None,
        })
        .unwrap();
    let second_chat = second.selected_chat_id.unwrap();
    alice
        .dispatch_and_wait(AppAction::OpenChat {
            room_id: room.clone(),
            topic_id: topic.clone(),
            chat_id: chat.clone(),
        })
        .unwrap();
    let history = alice
        .dispatch_and_wait(AppAction::SendChatMessage {
            room_id: room.clone(),
            topic_id: topic.clone(),
            chat_id: chat.clone(),
            text: "Keep this history".into(),
            metadata_json: None,
        })
        .unwrap();
    let moved = alice
        .dispatch_and_wait(AppAction::MoveChat {
            room_id: room.clone(),
            topic_id: topic.clone(),
            chat_id: chat.clone(),
            destination_topic_id: topic.clone(),
            before: Some(ChatAddress {
                topic_id: topic.clone(),
                chat_id: second_chat.clone(),
            }),
        })
        .unwrap();
    assert!(
        placement(&moved, &topic, &chat).position
            < placement(&moved, &topic, &second_chat).position
    );
    let home = alice
        .dispatch_and_wait(AppAction::MoveChat {
            room_id: room.clone(),
            topic_id: topic.clone(),
            chat_id: chat.clone(),
            destination_topic_id: HOME_TOPIC_ID.into(),
            before: None,
        })
        .unwrap();
    assert_eq!(placement(&home, &topic, &chat).topic_id, HOME_TOPIC_ID);
    assert_eq!(home.selected_topic_id, history.selected_topic_id);
    assert_eq!(home.selected_chat_id, history.selected_chat_id);
    assert_eq!(home.messages, history.messages);
    let home_placement = placement(&home, &topic, &chat).clone();
    // An original-route send cannot undo the explicit placement or split history.
    let sent = alice
        .dispatch_and_wait(AppAction::SendChatMessage {
            room_id: room.clone(),
            topic_id: topic.clone(),
            chat_id: chat.clone(),
            text: "Still the same chat".into(),
            metadata_json: None,
        })
        .unwrap();
    assert_eq!(placement(&sent, &topic, &chat), &home_placement);
    assert_eq!(sent.messages.len(), history.messages.len() + 1);
    let renamed = alice
        .dispatch_and_wait(AppAction::RenameChat {
            room_id: room.clone(),
            topic_id: topic.clone(),
            chat_id: chat.clone(),
            title: "Moved chat".into(),
        })
        .unwrap();
    assert_eq!(placement(&renamed, &topic, &chat), &home_placement);
    for archived in [true, false] {
        let state = alice
            .dispatch_and_wait(AppAction::SetChatArchived {
                room_id: room.clone(),
                topic_id: topic.clone(),
                chat_id: chat.clone(),
                archived,
            })
            .unwrap();
        assert_eq!(placement(&state, &topic, &chat), &home_placement);
    }
    let synced = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    assert_eq!(placement(&synced, &topic, &chat), &home_placement);
    // Invalid destination and an anchor moved concurrently are refused without altering metadata.
    let before = alice.state().unwrap();
    for (destination, anchor) in [
        ("missing".to_owned(), None),
        (
            HOME_TOPIC_ID.to_owned(),
            Some(ChatAddress {
                topic_id: topic.clone(),
                chat_id: second_chat,
            }),
        ),
        (
            HOME_TOPIC_ID.to_owned(),
            Some(ChatAddress {
                topic_id: topic.clone(),
                chat_id: chat.clone(),
            }),
        ),
    ] {
        assert!(
            alice
                .dispatch_and_wait(AppAction::MoveChat {
                    room_id: room.clone(),
                    topic_id: topic.clone(),
                    chat_id: chat.clone(),
                    destination_topic_id: destination,
                    before: anchor
                })
                .is_err()
        );
        assert_eq!(alice.state().unwrap().topics, before.topics);
    }
    drop(alice);
    let reopened =
        FiniteChatRuntime::open(options(&alice_dir, "alice", &unavailable_http_server_url()))
            .unwrap();
    let persisted = reopened.state().unwrap();
    assert_eq!(placement(&persisted, &topic, &chat), &home_placement);
    assert!(
        persisted
            .messages
            .iter()
            .any(|m| m.text == "Keep this history")
    );
    // A real network save failure does not publish a new placement.
    assert!(
        reopened
            .dispatch_and_wait(AppAction::MoveChat {
                room_id: room,
                topic_id: topic.clone(),
                chat_id: chat.clone(),
                destination_topic_id: topic.clone(),
                before: None
            })
            .is_err()
    );
    assert_eq!(
        placement(&reopened.state().unwrap(), &topic, &chat),
        &home_placement
    );
}

#[test]
fn chat_organization_envelope_limits_non_notifying_and_concurrent_replay() {
    let value = ChatPlacementV1 {
        topic_id: "original".into(),
        chat_id: "chat".into(),
        destination_topic_id: "home".into(),
        position: "AV".into(),
    };
    let envelope = |value: &ChatPlacementV1, topic: &str, policy| {
        serde_json::to_vec(&DecryptedApplicationEventV1 {
            kind: DurableAppEventKind::Namespaced {
                name: FINITECHAT_CHAT_PLACEMENT_EVENT_V1.into(),
                policy,
            },
            conversation_id: Some(topic.into()),
            segment_id: Some(value.chat_id.clone()),
            payload: serde_json::to_vec(value).unwrap(),
        })
        .unwrap()
    };
    assert!(matches!(
        decode_application_event(&envelope(
            &value,
            "original",
            ApplicationDeliveryPolicy::NON_NOTIFYING
        )),
        DecodedAppEvent::ChatPlacement(_)
    ));
    assert!(matches!(
        decode_application_event(&envelope(
            &value,
            "wrong",
            ApplicationDeliveryPolicy::NON_NOTIFYING
        )),
        DecodedAppEvent::Ignored
    ));
    let mut invalid = value.clone();
    for position in ["".to_owned(), "A0".into(), "!V".into(), "V".repeat(513)] {
        invalid.position = position;
        assert!(matches!(
            decode_application_event(&envelope(
                &invalid,
                "original",
                ApplicationDeliveryPolicy::NON_NOTIFYING
            )),
            DecodedAppEvent::Ignored
        ));
    }
    // Independent chats retain their moves; duplicates/stale events never overwrite newer moves.
    for order in [[1, 2, 3], [3, 1, 2], [2, 3, 1]] {
        let mut projection = ChatProjectionState::default();
        for seq in order {
            let mut event = value.clone();
            event.destination_topic_id = format!("topic-{seq}");
            projection.apply_chat_placement("room", seq, event.clone());
            projection.apply_chat_placement("room", seq, event);
        }
        let mut other = value.clone();
        other.chat_id = "other".into();
        projection.apply_chat_placement("room", 4, other);
        assert_eq!(projection.chat_placements.len(), 2);
        assert_eq!(
            projection.chat_placements[&("room".into(), "original".into(), "chat".into())]
                .1
                .topic_id,
            "topic-3"
        );
    }
}
