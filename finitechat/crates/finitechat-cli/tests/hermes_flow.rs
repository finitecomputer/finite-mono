use finitechat_core::{AppAction, AppRoomState, ChatMediaKind, FiniteChatRuntime, OpenOptions};
use finitechat_hermes::HermesMessagePayloadV1;
use finitechat_mls::NOSTR_SECRET_KEY_BYTES;
use finitechat_proto::DecryptedApplicationEventV1;
use finitechat_server::{HttpServerState, http_router};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

const USER_SECRET: [u8; NOSTR_SECRET_KEY_BYTES] = [41; NOSTR_SECRET_KEY_BYTES];
const TEST_NOW: u64 = 1_800_000_000;
const TEST_NOW_ARG: &str = "1800000000";

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
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let agent_home = dir.path().join("agent").display().to_string();
    let user_dir = dir.path().join("user").display().to_string();

    let init = cli_json(&[
        "hermes",
        "--home",
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
        TEST_NOW_ARG,
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
        now_unix_seconds: Some(TEST_NOW),
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
        TEST_NOW_ARG,
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
    })
    .expect("user sends");

    let poll = cli_json(&[
        "hermes",
        "--home",
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
        "--home",
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
        "--home",
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
            "--home".to_owned(),
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
        "--home",
        &agent_home,
        "room-status",
        "--room-id",
        &room_id,
        "--json",
    ]);
    assert_eq!(status["connected"], true);
    assert_eq!(status["paired"], true);
}

#[test]
fn app_cli_add_member_flow_uses_key_packages_and_welcomes() {
    ensure_test_finite_home();
    let dir = tempfile::tempdir().unwrap();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let alice_dir = dir.path().join("alice").display().to_string();
    let bob_dir = dir.path().join("bob").display().to_string();

    let bob = FiniteChatRuntime::open(OpenOptions {
        data_dir: bob_dir,
        server_url: server_url.clone(),
        device_id: "bob-cli".to_owned(),
        account_secret_hex: Some("42".repeat(32)),
        now_unix_seconds: Some(TEST_NOW),
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
        TEST_NOW_ARG,
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
        TEST_NOW_ARG,
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
