use super::*;
use finitechat_server::{HttpServerState, http_router};

fn server(path: &Path) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let router = http_router(HttpServerState::from_sqlite_path(path).unwrap());
    std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), router)
                    .await
                    .unwrap();
            });
    });
    url
}

fn state(home: &Path, server_url: &str) -> HermesServiceState {
    let runtime = FiniteChatRuntime::open(OpenOptions {
        data_dir: home.to_string_lossy().into_owned(),
        server_url: server_url.into(),
        device_id: "refusal-test".into(),
        account_secret_hex: Some("01".repeat(32)),
        now_unix_seconds: None,
    })
    .unwrap();
    let identity = runtime.state().unwrap().identity;
    HermesServiceState {
        agent_home: home.into(),
        account_id: identity.account_id,
        device_id: "refusal-test".into(),
        server_url: server_url.into(),
        runtime,
        inbox_lock: Arc::new(Mutex::new(())),
        running_lock: Arc::new(Mutex::new(())),
        refusal_lock: Arc::new(Mutex::new(())),
        bridge_updates: Arc::new((Mutex::new(0), Condvar::new())),
        joined_account_ids: Arc::new(Mutex::new(Vec::new())),
        server_stream: ServerStreamMonitor::default(),
    }
}

fn enqueue(state: &HermesServiceState, room: &str, seq: u64) -> Value {
    let id = format!("input-{seq}");
    let mut event =
        HermesPollEventV1::finite_chat_text(room, seq, &id, "human", "phone", "/update").unwrap();
    event.conversation_id = Some("home".into());
    // No segment is intentional: a Topic route must not turn into a Chat.
    let mut inbox = load_hermes_inbox(&state.agent_home).unwrap();
    enqueue_hermes_inbox_event(&state.agent_home, &mut inbox, event).unwrap();
    json!({"room_id": room, "seq": seq, "message_id": id, "text": "/update isn't available in Finite"})
}

fn work(state: &HermesServiceState) -> (String, Operation) {
    let entry = load_hermes_inbox(&state.agent_home)
        .unwrap()
        .events
        .remove(0);
    match entry.lease {
        HermesInboxLease::RefusalV1 { operation } => (entry.key, *operation),
        _ => panic!("no owned refusal"),
    }
}

#[test]
fn durable_refusal_owns_the_input_and_old_reader_rejects_pending_work() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path(), "http://127.0.0.1:1");
    let request = enqueue(&state, "room", 1);
    assert_eq!(begin(&state, request.clone()).unwrap()["durable"], true);
    let mut inbox = load_hermes_inbox(dir.path()).unwrap();
    assert!(
        lease_pending_hermes_inbox_events(dir.path(), &mut inbox, None, 10)
            .unwrap()
            .is_empty()
    );
    assert!(cmd_ack(dir.path(), request.clone(), &mut Vec::new()).is_err());
    cmd_release(dir.path(), request.clone(), &mut Vec::new()).unwrap();
    let (_key, operation) = work(&state);
    assert_eq!(operation.request.conversation_id.as_deref(), Some("home"));
    assert_eq!(operation.request.segment_id, None);
    assert_eq!(operation.request.thread_id, None);
    let mut changed = request;
    changed["text"] = "different after policy upgrade".into();
    assert!(begin(&state, changed).is_err());
    // A frozen old lease reader rejects the new tag instead of losing it on write.
    #[derive(Deserialize)]
    #[serde(tag = "state", rename_all = "snake_case")]
    enum OldLease {
        Pending,
        Leased {},
    }
    let raw = serde_json::to_value(load_hermes_inbox(dir.path()).unwrap()).unwrap();
    assert!(serde_json::from_value::<OldLease>(raw["events"][0]["lease"].clone()).is_err());
    assert_eq!(work(&state).1.request.text, operation.request.text);
    recover(&state).unwrap(); // Missing room is retained, never silently acked.
    assert!(work(&state).1.last_error.is_some());
    assert_eq!(
        diagnostics(&state).unwrap()["legacy_downgrade_ready"],
        false
    );
    enqueue(&state, "room", 2);
    let mut inbox = load_hermes_inbox(dir.path()).unwrap();
    let ordinary = lease_pending_hermes_inbox_events(dir.path(), &mut inbox, None, 10).unwrap();
    assert_eq!(ordinary.len(), 1);
    assert_eq!(ordinary[0].message_id, "input-2");
}

#[test]
fn durable_refusal_recovers_lost_acceptance_and_settles_once_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let url = server(&dir.path().join("server.sqlite3"));
    let home = dir.path().join("agent");
    let state = state(&home, &url);
    let app = state
        .runtime
        .dispatch_and_wait(AppAction::CreateRoom {
            display_name: "Refusal".into(),
        })
        .unwrap();
    let room = app.rooms[0].room_id.clone();
    let request = enqueue(&state, &room, 100);
    handle_hermes_service_action(&state, "refuse-command-v1", request.clone()).unwrap();
    let (key, mut operation) = work(&state);
    let error = advance_with(&state, &key, &mut operation, || {
        Err(CliError::Hermes("lost response".into()))
    });
    assert!(error.is_err());
    assert!(work(&state).1.prepared.is_some());
    let reply = state
        .runtime
        .state()
        .unwrap()
        .messages
        .into_iter()
        .find(|m| m.text.contains("isn't available"))
        .unwrap();
    assert_eq!(reply.conversation_id.as_deref(), Some("home"));
    // Existing projection chooses Home's default chat. It must not invent a
    // chat whose id is the Topic id ("home") from a source thread fallback.
    assert_eq!(reply.chat_id.as_deref(), Some("home-chat"));
    drop(state);

    let state = self::state(&home, &url);
    // An unrelated inbox append between recovery passes must survive final settlement.
    enqueue(&state, &room, 101);
    recover(&state).unwrap();
    let inbox = load_hermes_inbox(&home).unwrap();
    assert_eq!(inbox.events.len(), 1);
    assert_eq!(inbox.events[0].message_id, "input-101");
    assert!(inbox_key_recently_acked(&inbox, &key));
    assert_eq!(begin(&state, request).unwrap()["acked"], true);
    recover(&state).unwrap();
    let messages = state.runtime.state().unwrap().messages;
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.text.contains("isn't available"))
            .count(),
        1
    );
    assert!(messages.iter().any(|m| m.message_id == reply.message_id));
    assert_eq!(diagnostics(&state).unwrap()["legacy_downgrade_ready"], true);
}

#[test]
fn durable_refusal_concurrent_retries_have_one_owner_and_one_reply() {
    let dir = tempfile::tempdir().unwrap();
    let url = server(&dir.path().join("server.sqlite3"));
    let state = state(&dir.path().join("agent"), &url);
    let app = state
        .runtime
        .dispatch_and_wait(AppAction::CreateRoom {
            display_name: "Refusal".into(),
        })
        .unwrap();
    let request = enqueue(&state, &app.rooms[0].room_id, 100);
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let state = state.clone();
            let request = request.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                begin(&state, request).unwrap();
                recover(&state).unwrap();
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert!(
        load_hermes_inbox(&state.agent_home)
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(
        state
            .runtime
            .state()
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.text.contains("isn't available"))
            .count(),
        1
    );
}
