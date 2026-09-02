use finite_identity::{FiniteIdentity, IdentityPaths};
use finitechat_client::{
    FiniteChatDeviceConfig, FiniteChatDeviceState, SqliteClientStore, SqliteClientStoreOptions,
    StoredAppEvent, StoredAppMessage, StoredAppRoom, StoredAppState, StoredOutboundMessage,
};
use finitechat_core::{AppAction, AppRoomState, ChatMediaKind, FiniteChatRuntime, OpenOptions};
use finitechat_hermes::HermesMessagePayloadV1;
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::DecryptedApplicationEventV1;
use finitechat_server::{HttpServerState, http_router};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const USER_SECRET: [u8; NOSTR_SECRET_KEY_BYTES] = [41; NOSTR_SECRET_KEY_BYTES];

fn test_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_secs()
}

fn ensure_test_finite_home() -> PathBuf {
    use std::sync::OnceLock;
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let dir = tempfile::tempdir().expect("test FINITE_HOME tempdir");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        // SAFETY: set once before any identity resolution in this test binary.
        unsafe { std::env::set_var("FINITE_HOME", &path) };
        path
    })
    .clone()
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
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("live HTTP server did not become healthy");
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn cli_json(args: &[&str]) -> Value {
    ensure_test_finite_home();
    let mut output = Vec::new();
    finitechat_cli::run(args.iter().map(|arg| arg.to_string()), &mut output)
        .unwrap_or_else(|error| panic!("finitechat {args:?} failed: {error}"));
    serde_json::from_slice(&output)
        .unwrap_or_else(|error| panic!("finitechat {args:?} produced invalid JSON: {error}"))
}

#[test]
fn hermes_cli_uses_mls_add_welcome_and_round_trips_messages() {
    let dir = tempfile::tempdir().unwrap();
    let now = test_now();
    let now_arg = now.to_string();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let agent_home = dir.path().join("agent").display().to_string();
    let user_dir = dir.path().join("user").display().to_string();

    let init = cli_json(&[
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
    assert_eq!(init["device_id"], "agent");
    let agent_account = init["account_id"].as_str().unwrap().to_owned();
    assert!(agent_account.len() > 16);

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
        "Hermes Welcome Room",
    ]);
    let room_id = created["selected_room_id"].as_str().unwrap().to_owned();

    let user = FiniteChatRuntime::open(OpenOptions {
        data_dir: user_dir.clone(),
        server_url: server_url.clone(),
        device_id: "ios-user".to_owned(),
        account_secret_hex: Some(hex_lower(&USER_SECRET)),
        now_unix_seconds: Some(now),
    })
    .expect("user runtime opens");
    let user_account = user.state().unwrap().identity.account_id.clone();
    user.dispatch_and_wait(AppAction::StartRuntime)
        .expect("user publishes key packages");

    let added = cli_json(&[
        "app",
        "--data-dir",
        &agent_home,
        "--server",
        &server_url,
        "--device-id",
        "agent",
        "--now",
        &now_arg,
        "add-member",
        "--room-id",
        &room_id,
        "--account-id",
        &user_account,
        "--display-name",
        "iOS User",
    ]);
    assert_eq!(added["status"], "people added");

    let user_joined = user
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("user claims Welcome");
    let user_room = user_joined
        .rooms
        .iter()
        .find(|room| room.room_id == room_id)
        .expect("user room projects");
    assert_eq!(user_room.state, AppRoomState::Connected);

    user.dispatch_and_wait(AppAction::SendMessage {
        room_id: room_id.clone(),
        text: "hello hermes over welcome".to_owned(),
        metadata_json: None,
    })
    .expect("user sends");

    let poll = cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "poll",
        "--request-json",
        &json!({"timeout_millis": 1000}).to_string(),
    ]);
    let events = poll["events"].as_array().unwrap();
    assert!(
        events
            .iter()
            .any(|event| event["text"] == "hello hermes over welcome")
    );

    cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "send",
        "--request-json",
        &json!({
            "room_id": room_id,
            "conversation_id": null,
            "text": "hello back from hermes",
            "kind": "message",
            "status": "complete",
            "reply_to_message_id": null,
            "metadata": {},
        })
        .to_string(),
    ]);
    user.dispatch_and_wait(AppAction::StartRuntime)
        .expect("user syncs reply");
    let user_synced = user
        .dispatch_and_wait(AppAction::OpenRoom {
            room_id: room_id.clone(),
        })
        .expect("user opens room with reply");
    assert!(
        user_synced
            .messages
            .iter()
            .any(|message| message.text == "hello back from hermes")
    );

    let image_path = dir.path().join("agent-reply.png");
    let image_bytes = b"\x89PNG\r\n\x1a\nfinitechat hermes image";
    std::fs::write(&image_path, image_bytes).unwrap();
    cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "send",
        "--request-json",
        &json!({
            "room_id": room_id,
            "conversation_id": null,
            "text": "image back from hermes",
            "kind": "media",
            "status": "complete",
            "attachments": [{
                "kind": "image",
                "name": "agent-reply.png",
                "mime_type": "image/png",
                "path": image_path,
                "url": null,
                "blob": null
            }],
            "reply_to_message_id": null,
            "metadata": {},
        })
        .to_string(),
    ]);
    user.dispatch_and_wait(AppAction::StartRuntime)
        .expect("user syncs Hermes image reply");
    let with_image = user
        .dispatch_and_wait(AppAction::OpenRoom {
            room_id: room_id.clone(),
        })
        .expect("user opens room with image reply");
    let image_message = with_image
        .messages
        .iter()
        .find(|message| message.text == "image back from hermes")
        .expect("Hermes image message projects");
    assert_eq!(image_message.media.len(), 1);
    let media = &image_message.media[0];
    assert_eq!(media.kind, ChatMediaKind::Image);
    assert_eq!(media.filename, "agent-reply.png");
    assert_eq!(media.mime_type, "image/png");
    assert_ne!(media.attachment_id, image_path.display().to_string());
    assert!(
        media
            .url
            .as_deref()
            .is_some_and(|url| url.contains("/blobs/"))
    );
    assert_eq!(media.local_path, None);
    assert_eq!(media.upload_progress_per_mille, None);

    let event: DecryptedApplicationEventV1 =
        serde_json::from_slice(&image_message.payload).expect("typed app event decodes");
    let payload = HermesMessagePayloadV1::decode(&event.payload)
        .expect("Hermes media payload decodes")
        .expect("message is a Hermes payload");
    assert_eq!(payload.attachments.len(), 1);
    assert_eq!(payload.attachments[0].path, None);
    assert!(payload.attachments[0].blob.is_some());

    let downloaded = user
        .dispatch_and_wait(AppAction::DownloadAttachment {
            room_id: room_id.clone(),
            message_id: image_message.message_id.clone(),
            attachment_id: media.attachment_id.clone(),
        })
        .expect("user downloads and verifies Hermes image reply");
    let downloaded_image = downloaded
        .messages
        .iter()
        .find(|message| message.message_id == image_message.message_id)
        .expect("downloaded image remains projected");
    let local_path = downloaded_image.media[0]
        .local_path
        .as_ref()
        .expect("verified plaintext cache path projects");
    assert_eq!(std::fs::read(local_path).unwrap(), image_bytes);

    let mut invalid_output = Vec::new();
    let missing_path = dir.path().join("does-not-exist.png");
    let invalid_error = finitechat_cli::run(
        [
            "hermes".to_owned(),
            "--agent-home".to_owned(),
            agent_home.clone(),
            "send".to_owned(),
            "--request-json".to_owned(),
            json!({
                "room_id": room_id,
                "conversation_id": null,
                "text": "must not append",
                "kind": "media",
                "status": "complete",
                "attachments": [{
                    "kind": "image",
                    "name": "missing.png",
                    "mime_type": "image/png",
                    "path": missing_path,
                    "url": null,
                    "blob": null
                }],
                "reply_to_message_id": null,
                "metadata": {},
            })
            .to_string(),
        ],
        &mut invalid_output,
    )
    .expect_err("missing local attachment must fail before send");
    assert!(invalid_error.to_string().contains("could not open"));
    user.dispatch_and_wait(AppAction::StartRuntime)
        .expect("user syncs after rejected media send");
    let after_rejection = user
        .dispatch_and_wait(AppAction::OpenRoom {
            room_id: room_id.clone(),
        })
        .expect("user reopens room after rejected media send");
    assert!(
        after_rejection
            .messages
            .iter()
            .all(|message| message.text != "must not append")
    );

    let status = cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "room-status",
        "--room-id",
        &room_id,
        "--json",
    ]);
    assert_eq!(status["connected"], true);
    assert_eq!(status["paired"], true);

    // Operator rekey through the agent home: an ordinary self-update Commit
    // bumps the room epoch (create 0 -> add 1 -> rekey 2) and the user's
    // runtime applies it on its next sync, so traffic keeps flowing.
    let rekeyed = cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "rekey",
        "--room",
        &room_id,
        "--json",
    ]);
    assert_eq!(rekeyed["room_id"], room_id);
    assert_eq!(rekeyed["previous_epoch"], 1);
    assert_eq!(rekeyed["new_epoch"], 2);
    assert!(rekeyed["commit_seq"].as_u64().is_some_and(|seq| seq > 0));
    assert!(
        rekeyed["message_id"]
            .as_str()
            .is_some_and(|message_id| !message_id.is_empty())
    );
    let rekey_error = finitechat_cli::run(
        [
            "hermes".to_owned(),
            "--agent-home".to_owned(),
            agent_home.clone(),
            "rekey".to_owned(),
            "--room".to_owned(),
            "room-1-unknown".to_owned(),
            "--json".to_owned(),
        ],
        &mut Vec::new(),
    )
    .expect_err("rekey of an unknown room must fail closed");
    assert!(
        rekey_error
            .to_string()
            .contains("not available on this device")
    );

    cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "send",
        "--request-json",
        &json!({
            "room_id": room_id,
            "conversation_id": null,
            "text": "hello after rekey",
            "kind": "message",
            "status": "complete",
            "reply_to_message_id": null,
            "metadata": {},
        })
        .to_string(),
    ]);
    user.dispatch_and_wait(AppAction::StartRuntime)
        .expect("user applies the rekey Commit and the message after it");
    let after_rekey = user
        .dispatch_and_wait(AppAction::OpenRoom {
            room_id: room_id.clone(),
        })
        .expect("user opens room after rekey");
    assert!(
        after_rekey
            .messages
            .iter()
            .any(|message| message.text == "hello after rekey")
    );
}

/// Ownership audit O1, end to end against `hermes poll`/`ack`/`release`: an
/// inbound message is leased on delivery and not re-emitted on the next tick;
/// `ack` settles it; a fresh message can be `release`d back to the inbox and is
/// then redelivered. This is the sidecar owning in-flight state that the Python
/// adapter used to shadow with its own SQLite dedup store.
#[test]
fn hermes_inbox_leases_on_delivery_and_settles_with_ack_or_release() {
    let dir = tempfile::tempdir().unwrap();
    let now = test_now();
    let now_arg = now.to_string();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let agent_home = dir.path().join("agent").display().to_string();
    let user_dir = dir.path().join("user").display().to_string();

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
        "Lease Room",
    ]);
    let room_id = created["selected_room_id"].as_str().unwrap().to_owned();

    let user = FiniteChatRuntime::open(OpenOptions {
        data_dir: user_dir,
        server_url: server_url.clone(),
        device_id: "ios-user".to_owned(),
        account_secret_hex: Some(hex_lower(&USER_SECRET)),
        now_unix_seconds: Some(now),
    })
    .expect("user runtime opens");
    let user_account = user.state().unwrap().identity.account_id.clone();
    user.dispatch_and_wait(AppAction::StartRuntime)
        .expect("user publishes key packages");
    cli_json(&[
        "app",
        "--data-dir",
        &agent_home,
        "--server",
        &server_url,
        "--device-id",
        "agent",
        "--now",
        &now_arg,
        "add-member",
        "--room-id",
        &room_id,
        "--account-id",
        &user_account,
        "--display-name",
        "iOS User",
    ]);
    user.dispatch_and_wait(AppAction::StartRuntime)
        .expect("user claims Welcome");

    let poll = |timeout_millis: u64| -> Vec<Value> {
        cli_json(&[
            "hermes",
            "--agent-home",
            &agent_home,
            "poll",
            "--request-json",
            &json!({ "timeout_millis": timeout_millis }).to_string(),
        ])["events"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    let settle = |action: &str, event: &Value| {
        cli_json(&[
            "hermes",
            "--agent-home",
            &agent_home,
            action,
            "--request-json",
            &json!({
                "room_id": event["room_id"],
                "seq": event["seq"],
                "message_id": event["message_id"],
            })
            .to_string(),
        ]);
    };

    // First message: delivered exactly once, then leased.
    user.dispatch_and_wait(AppAction::SendMessage {
        room_id: room_id.clone(),
        text: "first".to_owned(),
        metadata_json: None,
    })
    .expect("user sends first");
    let leased = poll(2000);
    assert_eq!(leased.len(), 1, "the first poll leases the pending entry");
    assert_eq!(leased[0]["text"], "first");

    // A leased entry is not re-emitted on the next tick.
    assert!(
        poll(0).is_empty(),
        "a leased entry is not redelivered while its lease is held"
    );

    // Ack settles it; it stays gone.
    settle("ack", &leased[0]);
    assert!(poll(0).is_empty(), "an acked entry is not redelivered");

    // Second message: lease it, then release it back to the inbox.
    user.dispatch_and_wait(AppAction::SendMessage {
        room_id: room_id.clone(),
        text: "second".to_owned(),
        metadata_json: None,
    })
    .expect("user sends second");
    let second = poll(2000);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0]["text"], "second");
    assert!(
        poll(0).is_empty(),
        "the second entry is leased after delivery"
    );

    settle("release", &second[0]);
    let redelivered = poll(0);
    assert_eq!(
        redelivered.len(),
        1,
        "a released entry returns to Pending and is redelivered"
    );
    assert_eq!(redelivered[0]["text"], "second");
}

#[test]
fn app_cli_add_member_flow_uses_key_packages_and_welcomes() {
    ensure_test_finite_home();
    let dir = tempfile::tempdir().unwrap();
    let now = test_now();
    let now_arg = now.to_string();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let alice_dir = dir.path().join("alice").display().to_string();
    let bob_dir = dir.path().join("bob").display().to_string();

    let bob = FiniteChatRuntime::open(OpenOptions {
        data_dir: bob_dir,
        server_url: server_url.clone(),
        device_id: "bob-cli".to_owned(),
        account_secret_hex: Some("42".repeat(32)),
        now_unix_seconds: Some(now),
    })
    .expect("bob runtime opens");
    let bob_account_id = bob.state().unwrap().identity.account_id.clone();
    bob.dispatch_and_wait(AppAction::StartRuntime)
        .expect("bob publishes key packages");

    let created = cli_json(&[
        "app",
        "--data-dir",
        &alice_dir,
        "--server",
        &server_url,
        "--device-id",
        "alice-cli",
        "--now",
        &now_arg,
        "create-room",
        "--display-name",
        "CLI Add Flow",
    ]);
    let room_id = created["selected_room_id"].as_str().unwrap().to_owned();

    let added = cli_json(&[
        "app",
        "--data-dir",
        &alice_dir,
        "--server",
        &server_url,
        "--device-id",
        "alice-cli",
        "--now",
        &now_arg,
        "add-member",
        "--room-id",
        &room_id,
        "--account-id",
        &bob_account_id,
        "--display-name",
        "Bob",
    ]);
    assert_eq!(added["status"], "people added");

    let bob_joined = bob.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    assert_eq!(
        bob_joined
            .rooms
            .iter()
            .find(|room| room.room_id == room_id)
            .unwrap()
            .state,
        AppRoomState::Connected
    );
}

/// The full durable content of an agent store: device state (MLS ratchets
/// and KeyPackage inventory), projected rooms, persisted app state, the
/// outbox, and the stored message/event op log. A probe that writes anything
/// — a StartRuntime's KeyPackage publication, a Welcome activation, a
/// persisted selection — changes this snapshot, so equality across a probe
/// proves the probe did not write.
#[derive(Debug, PartialEq)]
struct AgentStoreSnapshot {
    device: FiniteChatDeviceState,
    rooms: Vec<StoredAppRoom>,
    app_state: StoredAppState,
    outbox: Vec<StoredOutboundMessage>,
    messages: Vec<StoredAppMessage>,
    events: Vec<StoredAppEvent>,
}

fn snapshot_agent_store(agent_home: &str, device_id: &str, now: u64) -> AgentStoreSnapshot {
    let paths = IdentityPaths::resolve().expect("identity paths resolve");
    let identity = FiniteIdentity::load(&paths).expect("shared identity loads");
    let secret = NostrSecretKey::from_bytes(identity.expose_secret_bytes())
        .expect("identity secret is a nostr key");
    let options =
        SqliteClientStoreOptions::from_nostr_secret(&secret, device_id).expect("store options");
    let store = SqliteClientStore::open_read_only(
        PathBuf::from(agent_home).join("client.sqlite3"),
        options,
    )
    .expect("read-only store opens alongside the resident writer");
    let config = FiniteChatDeviceConfig {
        account_secret_key: secret,
        device_id: device_id.to_owned(),
        now_unix_seconds: now,
        credential_not_before_unix_seconds: now.saturating_sub(3600),
        credential_not_after_unix_seconds: now + 90 * 24 * 60 * 60,
    };
    let device = store.load_device(config).expect("device loads");
    let owner = device.device_ref().clone();
    AgentStoreSnapshot {
        device: device.export_state().expect("device state exports"),
        rooms: store.load_app_rooms(&owner).expect("rooms load"),
        app_state: store.load_app_state(&owner).expect("app state loads"),
        outbox: store.load_app_outbox(&owner).expect("outbox loads"),
        messages: store
            .load_app_messages(&owner, u32::MAX)
            .expect("messages load"),
        events: store
            .load_app_events(&owner, u32::MAX)
            .expect("events load"),
    }
}

/// Regression for the poison-entry incident class: a one-shot `room-status`
/// (and a plain `app state` read) must report persisted state WITHOUT
/// dispatching StartRuntime or taking the store's writer lease, so it is safe
/// while a resident `hermes serve` owns the store.
#[test]
fn room_status_and_app_state_read_read_only_while_a_writer_holds_the_store() {
    ensure_test_finite_home();
    let dir = tempfile::tempdir().unwrap();
    let now = test_now();
    let now_arg = now.to_string();
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
        "Status Room",
    ]);
    let room_id = created["selected_room_id"].as_str().unwrap().to_owned();

    // A resident service sharing the agent home's identity holds the writer
    // lease for the rest of the test.
    let _resident = FiniteChatRuntime::open(OpenOptions {
        data_dir: agent_home.clone(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: None,
        now_unix_seconds: Some(now),
    })
    .expect("resident runtime opens");

    let before = snapshot_agent_store(&agent_home, "agent", now);

    let status = cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "room-status",
        "--room-id",
        &room_id,
        "--json",
    ]);
    assert_eq!(status["room_id"], room_id);
    assert_eq!(status["state"], "connected");
    assert_eq!(status["connected"], true);
    assert_eq!(status["member_count"], 1);

    let state = cli_json(&[
        "app",
        "--data-dir",
        &agent_home,
        "--server",
        &server_url,
        "--device-id",
        "agent",
        "--now",
        &now_arg,
        "state",
    ]);
    assert!(
        state["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|room| room["room_id"] == room_id)
    );

    // Both probes ran against the live store the resident writer holds; the
    // durable content — device ratchets, rooms, the message/event op log —
    // must be byte-for-byte what the writer left behind.
    let after = snapshot_agent_store(&agent_home, "agent", now);
    assert_eq!(
        before, after,
        "read-only probes must not write the store (no StartRuntime dispatch, no writer lease)"
    );

    // Sanity that the snapshot detects writes: once the resident releases
    // the lease, a writer command changes the durable content.
    drop(_resident);
    cli_json(&[
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
        "Writer Proof",
    ]);
    let after_write = snapshot_agent_store(&agent_home, "agent", now);
    assert_ne!(
        before, after_write,
        "the snapshot must detect a writer command's durable writes"
    );
}
