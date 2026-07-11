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
use finitechat_server::{HttpServerState, http_router};

const USER_SECRET: &str = "6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";
const AGENT_SECRET: &str = "4242424242424242424242424242424242424242424242424242424242424242";

#[test]
fn hosted_web_and_electron_continue_one_agent_chat_as_distinct_devices() {
    let root = tempfile::tempdir().unwrap();
    let server_url = spawn_chat_server(root.path().join("server.sqlite3"));
    let web_store = root.path().join("hosted-web");
    let electron_store = root.path().join("electron");
    let agent_store = root.path().join("agent");

    let mut web = open_runtime(&web_store, &server_url, "hosted-web", USER_SECRET);
    let mut electron = open_runtime(
        &electron_store,
        &server_url,
        "electron-alpha-test",
        USER_SECRET,
    );
    let agent = open_runtime(&agent_store, &server_url, "agent", AGENT_SECRET);

    let web_identity = sync(&web).identity;
    let electron_identity = sync(&electron).identity;
    let agent_identity = sync(&agent).identity;
    assert_eq!(web_identity.account_id, electron_identity.account_id);
    assert_ne!(web_identity.device_id, electron_identity.device_id);
    assert_ne!(web_identity.account_id, agent_identity.account_id);

    let created = web
        .dispatch_and_wait(AppAction::CreateRoom {
            display_name: "Device parity agent chat".to_owned(),
        })
        .unwrap();
    let room_id = created.selected_room_id.clone().unwrap();
    assert_eq!(created.selected_topic_id.as_deref(), Some(HOME_TOPIC_ID));
    assert_eq!(created.selected_chat_id.as_deref(), Some(HOME_CHAT_ID));

    add_member(
        &web,
        &agent,
        &room_id,
        profile(&agent_identity.account_id, "Parity Agent", true),
    );

    send(&web, &room_id, "history before Electron enrollment");
    let agent_state = sync(&agent);
    assert_message(
        &agent_state,
        "history before Electron enrollment",
        "hosted-web",
        false,
    );

    // The product link fanout claims this exact Device's KeyPackage and uses
    // normal MLS Add/Welcome. It does not copy the Hosted Web Device's store.
    sync(&electron);
    let fanout = web
        .link_device_and_wait(
            "electron-device-parity-link".to_owned(),
            "electron-alpha-test".to_owned(),
        )
        .unwrap();
    assert!(fanout.fanout_complete);
    assert_eq!(fanout.room_count, 1);
    let electron_after_join = sync(&electron);
    assert_room_connected(&electron_after_join, &room_id);
    let activated = web
        .link_device_and_wait(
            "electron-device-parity-link".to_owned(),
            "electron-alpha-test".to_owned(),
        )
        .unwrap();
    assert_eq!(activated.active_room_count, 1);
    assert!(
        electron_after_join
            .messages
            .iter()
            .all(|message| message.text != "history before Electron enrollment"),
        "a newly enrolled Device must not pretend it received pre-admission history"
    );

    send(&web, &room_id, "turn one from Hosted Web");
    let electron_after_web = sync(&electron);
    let agent_after_web = sync(&agent);
    assert_message(
        &electron_after_web,
        "turn one from Hosted Web",
        "hosted-web",
        true,
    );
    assert_message(
        &agent_after_web,
        "turn one from Hosted Web",
        "hosted-web",
        false,
    );

    send(&agent, &room_id, "agent reply to Hosted Web");
    let web_after_agent = sync(&web);
    let electron_after_agent = sync(&electron);
    assert_message(
        &web_after_agent,
        "agent reply to Hosted Web",
        "agent",
        false,
    );
    assert_message(
        &electron_after_agent,
        "agent reply to Hosted Web",
        "agent",
        false,
    );

    send(&electron, &room_id, "turn two from Electron");
    let web_after_electron = sync(&web);
    let agent_after_electron = sync(&agent);
    assert_message(
        &web_after_electron,
        "turn two from Electron",
        "electron-alpha-test",
        true,
    );
    assert_message(
        &agent_after_electron,
        "turn two from Electron",
        "electron-alpha-test",
        false,
    );

    send(&agent, &room_id, "agent reply to Electron");
    let web_before_restart = sync(&web);
    let electron_before_restart = sync(&electron);
    assert_shared_order(
        &web_before_restart,
        &electron_before_restart,
        &[
            "turn one from Hosted Web",
            "agent reply to Hosted Web",
            "turn two from Electron",
            "agent reply to Electron",
        ],
    );

    drop(electron);
    electron = open_runtime(
        &electron_store,
        &server_url,
        "electron-alpha-test",
        USER_SECRET,
    );
    let electron_restarted = sync(&electron);
    assert_selection(&electron_restarted, &room_id);
    assert_shared_order(
        &web_before_restart,
        &electron_restarted,
        &[
            "turn one from Hosted Web",
            "agent reply to Hosted Web",
            "turn two from Electron",
            "agent reply to Electron",
        ],
    );

    drop(web);
    web = open_runtime(&web_store, &server_url, "hosted-web", USER_SECRET);
    let web_restarted = sync(&web);
    assert_selection(&web_restarted, &room_id);
    assert_shared_order(
        &web_restarted,
        &electron_restarted,
        &[
            "turn one from Hosted Web",
            "agent reply to Hosted Web",
            "turn two from Electron",
            "agent reply to Electron",
        ],
    );

    send(&electron, &room_id, "post-restart turn from Electron");
    assert_message(
        &sync(&agent),
        "post-restart turn from Electron",
        "electron-alpha-test",
        false,
    );
    send(&agent, &room_id, "post-restart agent reply");
    let web_final = sync(&web);
    let electron_final = sync(&electron);
    assert_shared_order(
        &web_final,
        &electron_final,
        &[
            "turn one from Hosted Web",
            "agent reply to Hosted Web",
            "turn two from Electron",
            "agent reply to Electron",
            "post-restart turn from Electron",
            "post-restart agent reply",
        ],
    );
    assert_unique_message_ids(&web_final);
    assert_unique_message_ids(&electron_final);
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

fn send(runtime: &Arc<FiniteChatRuntime>, room_id: &str, text: &str) {
    runtime
        .dispatch_and_wait(AppAction::SendChatMessage {
            room_id: room_id.to_owned(),
            topic_id: HOME_TOPIC_ID.to_owned(),
            chat_id: HOME_CHAT_ID.to_owned(),
            text: text.to_owned(),
        })
        .unwrap();
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

fn assert_selection(state: &AppState, room_id: &str) {
    assert_eq!(state.selected_room_id.as_deref(), Some(room_id));
    assert_eq!(state.selected_topic_id.as_deref(), Some(HOME_TOPIC_ID));
    assert_eq!(state.selected_chat_id.as_deref(), Some(HOME_CHAT_ID));
}

fn assert_message(state: &AppState, text: &str, sender_device_id: &str, is_mine: bool) {
    let message = state
        .messages
        .iter()
        .find(|message| message.text == text)
        .unwrap_or_else(|| panic!("missing message {text:?}"));
    assert_eq!(message.sender_device_id, sender_device_id);
    assert_eq!(message.is_mine, is_mine);
    assert_eq!(message.conversation_id.as_deref(), Some(HOME_TOPIC_ID));
    assert_eq!(message.chat_id.as_deref(), Some(HOME_CHAT_ID));
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
