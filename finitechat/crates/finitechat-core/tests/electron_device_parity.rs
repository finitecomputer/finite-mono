use std::collections::HashSet;
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use finitechat_core::{
    AppAction, AppProfileSummary, AppRoomState, AppState, FiniteChatRuntime, HOME_CHAT_ID,
    HOME_TOPIC_ID, OpenOptions, npub_from_account_id,
};
use finitechat_hermes::HermesMessagePayloadV1;
use finitechat_proto::{DecryptedApplicationEventV1, DurableAppEventKind};
use finitechat_server::{HttpServerState, http_router};

const USER_SECRET: &str = "6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";
const AGENT_SECRET: &str = "4242424242424242424242424242424242424242424242424242424242424242";
const PEER_SECRET: &str = "3131313131313131313131313131313131313131313131313131313131313131";

#[test]
fn hosted_web_and_electron_continue_one_agent_chat_as_distinct_devices() {
    let root = tempfile::tempdir().unwrap();
    let server_url = spawn_chat_server(root.path().join("server.sqlite3"));
    let web_store = root.path().join("hosted-web");
    let electron_store = root.path().join("electron");
    let agent_store = root.path().join("agent");
    let peer_store = root.path().join("peer");

    let web = open_runtime(&web_store, &server_url, "hosted-web", USER_SECRET);
    let mut electron = open_runtime(
        &electron_store,
        &server_url,
        "electron-alpha-test",
        USER_SECRET,
    );
    let agent = open_runtime(&agent_store, &server_url, "agent", AGENT_SECRET);
    let peer = open_runtime(&peer_store, &server_url, "peer", PEER_SECRET);

    let web_identity = sync(&web).identity;
    let electron_identity = sync(&electron).identity;
    let agent_identity = sync(&agent).identity;
    let peer_identity = sync(&peer).identity;
    assert_eq!(web_identity.account_id, electron_identity.account_id);
    assert_ne!(web_identity.device_id, electron_identity.device_id);
    assert_ne!(web_identity.account_id, agent_identity.account_id);

    let first_room_id = web
        .dispatch_and_wait(AppAction::CreateRoom {
            display_name: "First parity room".to_owned(),
        })
        .unwrap()
        .selected_room_id
        .unwrap();
    let second_room_id = web
        .dispatch_and_wait(AppAction::CreateRoom {
            display_name: "Second parity room".to_owned(),
        })
        .unwrap()
        .selected_room_id
        .unwrap();
    // A fresh linked Device sorts generic rooms by their raw id. Make the
    // smaller id the legacy/non-agent room so the old first-room behavior is
    // guaranteed to select the wrong target.
    let (legacy_room_id, agent_room_id) = if first_room_id < second_room_id {
        (first_room_id, second_room_id)
    } else {
        (second_room_id, first_room_id)
    };
    save_room_name(&web, &legacy_room_id, "Legacy team room");
    save_room_name(&web, &agent_room_id, "Canonical agent room");

    add_member(
        &web,
        &peer,
        &legacy_room_id,
        profile(&peer_identity.account_id, "Parity Teammate", false),
    );
    add_member(
        &web,
        &agent,
        &agent_room_id,
        profile(&agent_identity.account_id, "Parity Agent", true),
    );

    web.dispatch_and_wait(AppAction::OpenRoom {
        room_id: agent_room_id.clone(),
    })
    .unwrap();
    let created_topic = web
        .dispatch_and_wait(AppAction::CreateTopic {
            room_id: agent_room_id.clone(),
            title: "Release investigation".to_owned(),
        })
        .unwrap();
    let topic_id = created_topic.selected_topic_id.unwrap();
    let chat_id = created_topic.selected_chat_id.unwrap();
    web.dispatch_and_wait(AppAction::RenameChat {
        room_id: agent_room_id.clone(),
        topic_id: topic_id.clone(),
        chat_id: chat_id.clone(),
        title: "Desktop continuation".to_owned(),
    })
    .unwrap();

    // Put the conversation foundations more than one bootstrap event window
    // behind the live transcript. The bootstrap must still retain the topic
    // metadata, selected segment start, and rename.
    for index in 0..66 {
        send_to_chat(
            &web,
            &agent_room_id,
            &topic_id,
            &chat_id,
            &format!("pre-link transcript {index:02}"),
        );
    }
    send_to_chat(
        &web,
        &agent_room_id,
        &topic_id,
        &chat_id,
        "latest history before Electron enrollment",
    );
    sync(&agent);

    // The target publishes its own KeyPackages. Fanout still uses normal MLS
    // Add/Welcome; the bounded application snapshot is sent only after those
    // room memberships exist.
    sync(&electron);
    let fanout = web
        .link_device_and_wait(
            "electron-device-parity-link".to_owned(),
            "electron-alpha-test".to_owned(),
        )
        .unwrap();
    assert!(fanout.fanout_complete);
    assert_eq!(fanout.room_count, 2);
    let electron_after_join = sync(&electron);
    assert_room_connected(&electron_after_join, &legacy_room_id);
    assert_room_connected(&electron_after_join, &agent_room_id);
    let activated = web
        .link_device_and_wait(
            "electron-device-parity-link".to_owned(),
            "electron-alpha-test".to_owned(),
        )
        .unwrap();
    assert_eq!(activated.active_room_count, 2);
    assert_hydrated_agent_conversation(&electron_after_join, &agent_room_id, &topic_id, &chat_id);
    assert_eq!(
        electron_after_join
            .rooms
            .iter()
            .find(|room| room.room_id == legacy_room_id)
            .unwrap()
            .display_name,
        "Legacy team room"
    );

    // Exercise the actual product route: the Electron composer reads these
    // selected ids, Hermes/agent bridge observes the same route, and the reply
    // is appended to it rather than to the legacy room.
    send_selected(&electron, "turn from paired Electron");
    let inbound = agent.agent_bridge_poll_once().unwrap();
    let (bridge_room_id, bridge_topic_id, bridge_chat_id) =
        bridge_route_for_text(&inbound.events, "turn from paired Electron");
    assert_eq!(bridge_room_id, agent_room_id);
    assert_eq!(bridge_topic_id, topic_id);
    assert_eq!(bridge_chat_id, chat_id);
    send_to_chat(
        &agent,
        &bridge_room_id,
        &bridge_topic_id,
        &bridge_chat_id,
        "agent reply to paired Electron",
    );
    let web_after_reply = sync(&web);
    let electron_after_reply = sync(&electron);
    assert_message_route(
        &electron_after_reply,
        "agent reply to paired Electron",
        "agent",
        false,
        &topic_id,
        &chat_id,
    );
    assert_shared_order(
        &web_after_reply,
        &electron_after_reply,
        &[
            "turn from paired Electron",
            "agent reply to paired Electron",
        ],
    );

    // A real user navigation after bootstrap owns the selection. Routine sync
    // and an idempotent fanout reconciliation must not switch it back.
    electron
        .dispatch_and_wait(AppAction::OpenRoom {
            room_id: legacy_room_id.clone(),
        })
        .unwrap();
    web.link_device_and_wait(
        "electron-device-parity-link".to_owned(),
        "electron-alpha-test".to_owned(),
    )
    .unwrap();
    assert_selection(
        &sync(&electron),
        &legacy_room_id,
        HOME_TOPIC_ID,
        HOME_CHAT_ID,
    );

    // Simulate Paul's pre-bootstrap persisted state: MLS rooms and identity are
    // already linked, but local room metadata is the raw id and the arbitrary
    // first room was persisted. Reopening the upgraded build requests a fresh
    // typed snapshot from the existing Hosted Web Device—no re-pairing.
    save_room_name(&electron, &legacy_room_id, &legacy_room_id);
    save_room_name(&electron, &agent_room_id, &agent_room_id);

    drop(electron);
    electron = open_runtime(
        &electron_store,
        &server_url,
        "electron-alpha-test",
        USER_SECRET,
    );
    let legacy_reopened = sync(&electron);
    assert_eq!(
        legacy_reopened.selected_room_id.as_deref(),
        Some(legacy_room_id.as_str())
    );
    sync(&web); // consumes the encrypted requests and emits target-bound replies
    let repaired = sync(&electron);
    assert_hydrated_agent_conversation(&repaired, &agent_room_id, &topic_id, &chat_id);
    assert_unique_message_ids(&repaired);

    // Duplicate/no-op sync is idempotent, and a later explicit selection stays
    // selected across another process restart.
    sync(&web);
    assert_unique_message_ids(&sync(&electron));
    electron
        .dispatch_and_wait(AppAction::OpenRoom {
            room_id: legacy_room_id.clone(),
        })
        .unwrap();
    drop(electron);
    electron = open_runtime(
        &electron_store,
        &server_url,
        "electron-alpha-test",
        USER_SECRET,
    );
    assert_selection(
        &sync(&electron),
        &legacy_room_id,
        HOME_TOPIC_ID,
        HOME_CHAT_ID,
    );
}

fn open_runtime(
    data_dir: &Path,
    server_url: &str,
    device_id: &str,
    account_secret_hex: &str,
) -> Arc<FiniteChatRuntime> {
    FiniteChatRuntime::open(OpenOptions {
        data_dir: data_dir.display().to_string(),
        server_url: server_url.to_owned(),
        device_id: device_id.to_owned(),
        account_secret_hex: Some(account_secret_hex.to_owned()),
        now_unix_seconds: None,
    })
    .unwrap()
}

fn sync(runtime: &Arc<FiniteChatRuntime>) -> AppState {
    runtime.dispatch_and_wait(AppAction::StartRuntime).unwrap()
}

fn add_member(
    owner: &Arc<FiniteChatRuntime>,
    member: &Arc<FiniteChatRuntime>,
    room_id: &str,
    profile: AppProfileSummary,
) {
    sync(member);
    owner
        .dispatch_and_wait(AppAction::AddRoomMembers {
            room_id: room_id.to_owned(),
            profiles: vec![profile],
        })
        .unwrap();
    assert_room_connected(&sync(member), room_id);
}

fn save_room_name(runtime: &Arc<FiniteChatRuntime>, room_id: &str, display_name: &str) {
    runtime
        .dispatch_and_wait(AppAction::SaveRoomMetadata {
            room_id: room_id.to_owned(),
            display_name: display_name.to_owned(),
            picture: None,
        })
        .unwrap();
}

fn send_to_chat(
    runtime: &Arc<FiniteChatRuntime>,
    room_id: &str,
    topic_id: &str,
    chat_id: &str,
    text: &str,
) {
    runtime
        .dispatch_and_wait(AppAction::SendChatMessage {
            room_id: room_id.to_owned(),
            topic_id: topic_id.to_owned(),
            chat_id: chat_id.to_owned(),
            text: text.to_owned(),
        })
        .unwrap();
}

fn send_selected(runtime: &Arc<FiniteChatRuntime>, text: &str) {
    let selected = runtime.state().unwrap();
    send_to_chat(
        runtime,
        selected.selected_room_id.as_deref().unwrap(),
        selected.selected_topic_id.as_deref().unwrap(),
        selected.selected_chat_id.as_deref().unwrap(),
        text,
    );
}

fn bridge_route_for_text(
    events: &[finitechat_core::AppBridgeAppliedEvent],
    expected_text: &str,
) -> (String, String, String) {
    for event in events {
        let Ok(application) =
            serde_json::from_slice::<DecryptedApplicationEventV1>(&event.plaintext)
        else {
            continue;
        };
        if application.kind != DurableAppEventKind::ChatMessage {
            continue;
        }
        let Ok(Some(message)) = HermesMessagePayloadV1::decode(&application.payload) else {
            continue;
        };
        if message.text == expected_text {
            return (
                event.room_id.clone(),
                application.conversation_id.unwrap(),
                application.segment_id.unwrap(),
            );
        }
    }
    panic!("agent bridge did not receive {expected_text:?}");
}

fn profile(account_id: &str, display_name: &str, is_agent: bool) -> AppProfileSummary {
    AppProfileSummary {
        account_id: account_id.to_owned(),
        npub: npub_from_account_id(account_id.to_owned()).unwrap(),
        display_name: display_name.to_owned(),
        about: None,
        picture: None,
        stale: false,
        is_agent,
    }
}

fn assert_room_connected(state: &AppState, room_id: &str) {
    let room = state
        .rooms
        .iter()
        .find(|room| room.room_id == room_id)
        .unwrap_or_else(|| panic!("missing room {room_id}"));
    assert_eq!(room.state, AppRoomState::Connected);
}

fn assert_selection(state: &AppState, room_id: &str, topic_id: &str, chat_id: &str) {
    assert_eq!(state.selected_room_id.as_deref(), Some(room_id));
    assert_eq!(state.selected_topic_id.as_deref(), Some(topic_id));
    assert_eq!(state.selected_chat_id.as_deref(), Some(chat_id));
}

fn assert_message_route(
    state: &AppState,
    text: &str,
    sender_device_id: &str,
    is_mine: bool,
    topic_id: &str,
    chat_id: &str,
) {
    let message = state
        .messages
        .iter()
        .find(|message| message.text == text)
        .unwrap_or_else(|| panic!("missing message {text:?}"));
    assert_eq!(message.sender_device_id, sender_device_id);
    assert_eq!(message.is_mine, is_mine);
    assert_eq!(message.conversation_id.as_deref(), Some(topic_id));
    assert_eq!(message.chat_id.as_deref(), Some(chat_id));
}

fn assert_hydrated_agent_conversation(
    state: &AppState,
    agent_room_id: &str,
    topic_id: &str,
    chat_id: &str,
) {
    assert_selection(state, agent_room_id, topic_id, chat_id);
    assert_eq!(state.rooms.len(), 2);
    let agent_room = state
        .rooms
        .iter()
        .find(|room| room.room_id == agent_room_id)
        .unwrap();
    assert_eq!(agent_room.display_name, "Canonical agent room");
    assert!(agent_room.is_agent_chat);
    assert_eq!(
        state
            .topics
            .iter()
            .filter(|topic| topic.room_id == agent_room_id && topic.topic_id == HOME_TOPIC_ID)
            .count(),
        1,
        "the canonical room must project exactly one Home topic"
    );
    let topic = state
        .topics
        .iter()
        .find(|topic| topic.room_id == agent_room_id && topic.topic_id == topic_id)
        .unwrap();
    assert_eq!(topic.title, "Release investigation");
    assert_eq!(topic.active_chat_id.as_deref(), Some(chat_id));
    assert_eq!(
        topic
            .chats
            .iter()
            .find(|chat| chat.chat_id == chat_id)
            .unwrap()
            .title,
        "Desktop continuation"
    );
    assert_message_route(
        state,
        "latest history before Electron enrollment",
        "hosted-web",
        true,
        topic_id,
        chat_id,
    );
}

fn assert_shared_order(left: &AppState, right: &AppState, expected: &[&str]) {
    let project = |state: &AppState| {
        state
            .messages
            .iter()
            .filter(|message| expected.contains(&message.text.as_str()))
            .map(|message| {
                (
                    message.seq,
                    message.message_id.clone(),
                    message.text.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let left = project(left);
    let right = project(right);
    assert_eq!(left, right, "both Devices must project one ordered Chat");
    assert_eq!(
        left.iter()
            .map(|(_, _, text)| text.as_str())
            .collect::<Vec<_>>(),
        expected
    );
}

fn assert_unique_message_ids(state: &AppState) {
    let mut ids = HashSet::new();
    for message in &state.messages {
        assert!(
            ids.insert(message.message_id.clone()),
            "duplicate projected message id {}",
            message.message_id
        );
    }
}

fn spawn_chat_server(path: impl AsRef<Path>) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let app = http_router(HttpServerState::from_sqlite_path(path).unwrap());
    thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    let server_url = format!("http://{address}");
    let health = format!("{server_url}/health");
    for _ in 0..100 {
        if reqwest::blocking::get(&health)
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            return server_url;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("Finite Chat server did not become ready at {health}");
}
