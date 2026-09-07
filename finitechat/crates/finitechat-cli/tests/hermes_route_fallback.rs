//! An unresolvable `thread_id` falls back to the Home default (with a loud
//! warning) so a topic archived mid-session never silently consumes the
//! user's message. There is one behaviour and no policy switch.

use finitechat_server::{HttpServerState, http_router};
use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn test_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_secs()
}

fn ensure_test_finite_home() {
    static HOME: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let dir = tempfile::tempdir().expect("test FINITE_HOME tempdir");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        // SAFETY: set once before any identity resolution in this test binary.
        unsafe { std::env::set_var("FINITE_HOME", &path) };
    });
}

fn spawn_live_http_server(path: &Path) -> String {
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
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("live HTTP server did not become healthy");
}

fn cli_json(args: &[&str]) -> Value {
    ensure_test_finite_home();
    let mut output = Vec::new();
    finitechat_cli::run(args.iter().map(|arg| arg.to_string()), &mut output)
        .unwrap_or_else(|error| panic!("finitechat {args:?} failed: {error}"));
    serde_json::from_slice(&output)
        .unwrap_or_else(|error| panic!("finitechat {args:?} produced invalid JSON: {error}"))
}

/// One agent home with its own room on its own live server; returns
/// `(agent home dir string, room id)`. The room has no topics, so any
/// `thread_id` is unresolvable.
fn setup_agent_room() -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let now_arg = test_now().to_string();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let agent_home = dir.path().join("agent").display().to_string();

    cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "init",
        "--server",
        &server_url,
        "--device-id",
        "agent",
        "--skip-agent-profile",
        "--json",
    ]);
    let created = cli_json(&[
        "app",
        "--data-dir",
        &agent_home,
        "--server",
        &server_url,
        "--device-id",
        "agent",
        "--now",
        &now_arg,
        "create-room",
        "--display-name",
        "Route Fallback Room",
    ]);
    let room_id = created["selected_room_id"].as_str().unwrap().to_owned();
    // The fixture's stores and running state are read for the rest of the test.
    std::mem::forget(dir);
    (agent_home, room_id)
}

fn send_with_unknown_thread(home: &str, room_id: &str) -> Value {
    cli_json(&[
        "hermes",
        "--agent-home",
        home,
        "send",
        "--request-json",
        &json!({
            "room_id": room_id,
            "conversation_id": null,
            "segment_id": null,
            "thread_id": "topic-archived-mid-session",
            "text": "reply that must not vanish",
            "kind": "message",
            // `running` makes the sidecar record the resolved route fields
            // in hermes-running.json where the assertions can read them.
            "status": "running",
            "reply_to_message_id": null,
            "metadata": {},
        })
        .to_string(),
    ])
}

fn recorded_running_route(home: &str) -> (String, Value, Value) {
    let raw = std::fs::read_to_string(Path::new(home).join("hermes-running.json"))
        .expect("a successful send with status=running records its route");
    let running: Value = serde_json::from_str(&raw).expect("valid hermes-running.json");
    let messages = running["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 1, "exactly one running turn is recorded");
    let entry = &messages[0];
    (
        entry["message_id"].as_str().unwrap().to_owned(),
        entry["conversation_id"].clone(),
        entry["segment_id"].clone(),
    )
}

#[test]
fn unknown_thread_resolves_to_home_fallback_fields() {
    let (agent_home, room_id) = setup_agent_room();

    // Deliverable by default: an unknown thread id falls back to the Home
    // default instead of failing the reply.
    let sent = send_with_unknown_thread(&agent_home, &room_id);

    let (message_id, conversation_id, segment_id) = recorded_running_route(&agent_home);
    assert_eq!(
        message_id,
        sent["message_id"].as_str().expect("sent reply has an id"),
        "the recorded running turn is this reply"
    );
    assert_eq!(
        conversation_id,
        Value::Null,
        "unknown thread falls back to Core's Home default (no explicit conversation)"
    );
    assert_eq!(
        segment_id,
        Value::Null,
        "unknown thread falls back to Core's Home default (no explicit segment)"
    );
}
