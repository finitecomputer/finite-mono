//! End-to-end proof for `finitechat capture room-log`: a multi-entry room
//! log served by an in-process finitechat-server round-trips through the
//! capture CLI into the `CapturedRoomLogFile` diagnose input type.

use finitechat_client::rejected_entry_diagnostic::CapturedRoomLogFile;
use finitechat_client::{HttpRuntimeDelivery, ReqwestHttpRuntimeTransport};
use finitechat_delivery::MAX_HTTP_SYNC_PAGE_ENTRIES;
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::{
    AppendEventRequest, CreateRoomRequest, DeviceRef, DurableAppEventKind, EventAccepted,
    LogEntryKind, RoomProtocol, envelope,
};
use finitechat_server::{HttpServerState, http_router};

const ALICE_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [17; NOSTR_SECRET_KEY_BYTES];
const DEVICE_ID: &str = "alice_capture";
const ROOM_ID: &str = "room_capture_round_trip";
const MLS_GROUP_ID: &str = "mls_capture_round_trip";

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
        std::thread::sleep(std::time::Duration::from_millis(10));
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

fn alice_device() -> DeviceRef {
    let secret = NostrSecretKey::from_bytes(ALICE_ACCOUNT_SECRET_BYTES).unwrap();
    DeviceRef::new(hex_lower(secret.public_key().as_bytes()), DEVICE_ID)
}

fn write_account_secret_file(dir: &std::path::Path) -> String {
    let path = dir.join("account-secret.hex");
    std::fs::write(
        &path,
        format!("{}\n", hex_lower(&ALICE_ACCOUNT_SECRET_BYTES)),
    )
    .unwrap();
    path.display().to_string()
}

/// Bootstrap a room and append `count` application entries to it; returns
/// the acceptances in log order.
fn seed_room(server_url: &str, count: usize) -> Vec<EventAccepted> {
    let alice = alice_device();
    let mut delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.to_owned()));
    delivery
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: ROOM_ID.to_owned(),
            mls_group_id: MLS_GROUP_ID.to_owned(),
            creator: alice.clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    (1..=count)
        .map(|index| {
            delivery
                .append_event(
                    &AppendEventRequest {
                        room_id: ROOM_ID.to_owned(),
                        sender: alice.clone(),
                        envelope: envelope(
                            ROOM_ID,
                            MLS_GROUP_ID,
                            alice.clone(),
                            0,
                            LogEntryKind::Application,
                            format!(r#"{{"body":"captured message {index}"}}"#).into_bytes(),
                        ),
                        idempotency_key: format!("capture_idempotency_{index}"),
                        timestamp_unix_seconds: 1_800_000_000 + index as u64,
                    },
                    DurableAppEventKind::ChatMessage.delivery_policy(),
                )
                .unwrap()
        })
        .collect()
}

fn run_capture(server_url: &str, secret_file: &str, out: &str, extra_args: &[&str]) {
    let mut args = vec![
        "capture",
        "room-log",
        "--server",
        server_url,
        "--room-id",
        ROOM_ID,
        "--device-id",
        DEVICE_ID,
        "--account-secret-file",
        secret_file,
        "--out",
        out,
    ];
    args.extend_from_slice(extra_args);
    let mut output = Vec::new();
    finitechat_cli::run(args.iter().map(|arg| arg.to_string()), &mut output)
        .unwrap_or_else(|error| panic!("finitechat capture room-log failed: {error}"));
}

fn read_capture(out: &str) -> CapturedRoomLogFile {
    let bytes = std::fs::read(out).unwrap();
    // The acceptance contract: the capture must load as the diagnose input.
    serde_json::from_slice::<CapturedRoomLogFile>(&bytes)
        .unwrap_or_else(|error| panic!("capture output is not a CapturedRoomLogFile: {error}"))
}

#[test]
fn capture_room_log_round_trips_multi_entry_log_into_diagnose_input() {
    let dir = tempfile::tempdir().unwrap();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let accepted = seed_room(&server_url, 3);
    let secret_file = write_account_secret_file(dir.path());
    let out = dir.path().join("capture.json").display().to_string();

    run_capture(&server_url, &secret_file, &out, &[]);

    let capture = read_capture(&out);
    assert_eq!(capture.target_room_id, ROOM_ID);
    assert_eq!(capture.rooms.len(), 1);
    let room = &capture.rooms[0];
    assert_eq!(room.room_id, ROOM_ID);
    assert_eq!(room.entries.len(), accepted.len());
    for (entry, accepted) in room.entries.iter().zip(accepted.iter()) {
        assert_eq!(entry.seq, accepted.seq);
        assert_eq!(entry.message_id, accepted.message_id);
        assert_eq!(entry.room_id, ROOM_ID);
        assert_eq!(entry.envelope.room_id, ROOM_ID);
        assert_eq!(entry.kind, LogEntryKind::Application);
        assert_eq!(entry.sender, alice_device());
    }

    // A second capture to the same path must fail: captures are evidence.
    let mut output = Vec::new();
    let rerun = finitechat_cli::run(
        [
            "capture",
            "room-log",
            "--server",
            &server_url,
            "--room-id",
            ROOM_ID,
            "--device-id",
            DEVICE_ID,
            "--account-secret-file",
            &secret_file,
            "--out",
            &out,
        ]
        .iter()
        .map(|arg| arg.to_string()),
        &mut output,
    );
    assert!(rerun.is_err());
}

#[test]
fn capture_room_log_paginates_past_one_sync_page() {
    let dir = tempfile::tempdir().unwrap();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let count = MAX_HTTP_SYNC_PAGE_ENTRIES + 3;
    let accepted = seed_room(&server_url, count);
    let secret_file = write_account_secret_file(dir.path());
    let out = dir.path().join("capture.json").display().to_string();

    run_capture(&server_url, &secret_file, &out, &[]);

    let capture = read_capture(&out);
    let entries = &capture.rooms[0].entries;
    assert_eq!(entries.len(), count);
    for (index, entry) in entries.iter().enumerate() {
        assert_eq!(entry.seq, index as u64 + 1);
        assert_eq!(entry.message_id, accepted[index].message_id);
    }

    // Resuming from a cursor captures only the tail.
    let tail_out = dir.path().join("capture-tail.json").display().to_string();
    let after_seq = (count - 2).to_string();
    run_capture(
        &server_url,
        &secret_file,
        &tail_out,
        &["--after-seq", &after_seq],
    );
    let tail = read_capture(&tail_out);
    let tail_entries = &tail.rooms[0].entries;
    assert_eq!(tail_entries.len(), 2);
    assert_eq!(tail_entries[0].seq, count as u64 - 1);
    assert_eq!(tail_entries[1].seq, count as u64);
}

#[test]
fn capture_room_log_rejects_an_invalid_account_secret_file() {
    let dir = tempfile::tempdir().unwrap();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let secret_file = dir.path().join("bad-secret.hex");
    std::fs::write(&secret_file, "not hex\n").unwrap();
    let out = dir.path().join("capture.json").display().to_string();

    let mut output = Vec::new();
    let result = finitechat_cli::run(
        [
            "capture",
            "room-log",
            "--server",
            &server_url,
            "--room-id",
            ROOM_ID,
            "--device-id",
            DEVICE_ID,
            "--account-secret-file",
            &secret_file.display().to_string(),
            "--out",
            &out,
        ]
        .iter()
        .map(|arg| arg.to_string()),
        &mut output,
    );
    assert!(result.is_err());
    assert!(!std::path::Path::new(&out).exists());
}
