//! Bridge-path latency benchmark (ignored; run in release):
//!
//! ```
//! cargo test -p finitechat-cli --test perf_bridge --release -- --ignored --nocapture
//! ```
//!
//! Measures every leg a hermes message crosses — MLS encrypt, HTTP publish
//! (JSON + WAL), wake hint, sync + decrypt — in-process against a live
//! server, then the same path through the real `hermes` subprocess bridge
//! the Python plugin shells to. Prints p50/p99 per leg so hot spots and
//! pathological scaling are visible against the theoretical floor
//! (loopback RTTs + crypto + fsync).

use finitechat_client::{
    AppliedLogEntry, FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery,
    ReqwestHttpRuntimeTransport, RuntimeSyncOptions, SqliteClientStore, SqliteClientStoreOptions,
    finalize_invited_room, run_room_server_sync_tick, run_runtime_sync_tick,
    submit_invite_join_request,
};
use finitechat_http::{SyncWaitRequest, SyncWaitRoom};
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::{DurableAppEventKind, InviteCodeV1};
use finitechat_server::{HttpServerState, http_router};
use serde_json::{Value, json};
use std::io::Write as _;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const AGENT_SECRET: [u8; NOSTR_SECRET_KEY_BYTES] = [51; NOSTR_SECRET_KEY_BYTES];
const USER_SECRET: [u8; NOSTR_SECRET_KEY_BYTES] = [53; NOSTR_SECRET_KEY_BYTES];

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
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
    panic!("server never became healthy");
}

struct Timings(Vec<Duration>);

impl Timings {
    fn report(mut self, label: &str) -> Duration {
        self.0.sort_unstable();
        let p50 = self.0[self.0.len() / 2];
        let p99 = self.0[(self.0.len() * 99) / 100];
        println!("{label:<58} p50 {p50:>10.2?}   p99 {p99:>10.2?}");
        p50
    }
}

fn time_n(n: usize, mut f: impl FnMut(usize)) -> Timings {
    let mut out = Vec::with_capacity(n);
    for index in 0..n {
        let started = Instant::now();
        f(index);
        out.push(started.elapsed());
    }
    Timings(out)
}

fn device(secret: [u8; NOSTR_SECRET_KEY_BYTES], device_id: &str) -> FiniteChatDevice {
    let now = now_ms() / 1000;
    FiniteChatDevice::new(FiniteChatDeviceConfig {
        account_secret_key: NostrSecretKey::from_bytes(secret).unwrap(),
        device_id: device_id.to_owned(),
        now_unix_seconds: now,
        credential_not_before_unix_seconds: now - 3600,
        credential_not_after_unix_seconds: now + 7 * 86400,
    })
    .unwrap()
}

#[test]
#[ignore = "release-mode latency benchmark; run with --ignored --nocapture"]
fn bridge_path_latency_breakdown() {
    let dir = tempfile::tempdir().unwrap();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let bin = env!("CARGO_BIN_EXE_finitechat");

    // --- Pair an agent (CLI home) with an in-process user device. ---
    let agent_home = dir.path().join("agent").display().to_string();
    // The agent's shared Finite identity lives in a benchmark-local
    // FINITE_HOME, never the developer's real ~/.finite.
    let finite_home = dir.path().join("finite-home");
    let cli = |args: &[&str]| -> Value {
        let output = std::process::Command::new(bin)
            .env("FINITE_HOME", &finite_home)
            .args(["hermes", "--home", &agent_home])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    cli(&["init", "--server", &server_url]);
    let invite = cli(&["invite", "--json"]);
    let room_id = invite["room_id"].as_str().unwrap().to_owned();
    let code = InviteCodeV1::parse(invite["url"].as_str().unwrap()).unwrap();

    let mut user = device(USER_SECRET, "bench_user");
    let mut user_store = SqliteClientStore::open(
        dir.path().join("user.sqlite3"),
        SqliteClientStoreOptions::from_nostr_secret(
            &NostrSecretKey::from_bytes(USER_SECRET).unwrap(),
            "bench_user",
        )
        .unwrap(),
    )
    .unwrap();
    user_store.save_device_state(&user).unwrap();
    let mut user_delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.clone()));
    let pairing_started = Instant::now();
    submit_invite_join_request(
        &mut user_store,
        &mut user,
        &mut user_delivery,
        &code,
        None,
        now_ms(),
    )
    .unwrap();
    cli(&["poll", "--request-json", r#"{"timeout_millis":5000}"#]);
    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 8,
    };
    run_runtime_sync_tick(&mut user_store, &mut user, &mut user_delivery, &options).unwrap();
    finalize_invited_room(&mut user_store, &mut user, &code).unwrap();
    println!(
        "\npairing (invite already printed → joined+verified)        total {:>10.2?}\n",
        pairing_started.elapsed()
    );

    // --- An in-process "agent" device for isolating legs. ---
    let mut agent2 = device(AGENT_SECRET, "bench_sender");
    let mut agent2_store = SqliteClientStore::open(
        dir.path().join("agent2.sqlite3"),
        SqliteClientStoreOptions::from_nostr_secret(
            &NostrSecretKey::from_bytes(AGENT_SECRET).unwrap(),
            "bench_sender",
        )
        .unwrap(),
    )
    .unwrap();
    agent2_store.save_device_state(&agent2).unwrap();
    let mut agent2_delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.clone()));
    let join2 = submit_invite_join_request(
        &mut agent2_store,
        &mut agent2,
        &mut agent2_delivery,
        &code,
        None,
        now_ms(),
    )
    .unwrap();
    let _ = join2;
    cli(&["poll", "--request-json", r#"{"timeout_millis":5000}"#]);
    run_runtime_sync_tick(
        &mut agent2_store,
        &mut agent2,
        &mut agent2_delivery,
        &options,
    )
    .unwrap();
    finalize_invited_room(&mut agent2_store, &mut agent2, &code).unwrap();
    run_room_server_sync_tick(
        &mut user_store,
        &mut user,
        &mut user_delivery,
        &options,
        &code.server_url,
    )
    .unwrap();

    const N: usize = 64;
    let payload_1k = vec![0x42u8; 1024];
    let payload_32k = vec![0x42u8; 32 * 1024];

    // Leg 1: MLS application encryption (in-memory).
    let mut encrypted = Vec::new();
    time_n(N, |i| {
        encrypted.push(
            agent2
                .create_application_request(&room_id, &payload_1k, format!("bench-enc-{i}"))
                .unwrap(),
        );
    })
    .report("leg 1  MLS encrypt 1 KiB (create_application_request)");

    // Leg 1b: full client persist after one encrypt (the per-send save).
    time_n(N, |_| {
        agent2_store.save_device_state(&agent2).unwrap();
    })
    .report("leg 1b client store save (full encrypted snapshot)");

    // Leg 2: HTTP publish (JSON serialize + loopback + WAL persist).
    let wire = serde_json::to_vec(&encrypted[0]).unwrap();
    println!(
        "        wire size for 1 KiB ciphertext payload: {} bytes (JSON number-array tax ×{:.1})",
        wire.len(),
        wire.len() as f64 / 1024.0
    );
    let policy = DurableAppEventKind::ChatMessage.delivery_policy();
    let mut published_seq = 0;
    time_n(N, |i| {
        published_seq = agent2_delivery
            .append_event(&encrypted[i], policy)
            .unwrap()
            .seq;
    })
    .report("leg 2  HTTP POST /events (serialize+loopback+WAL)");

    // Leg 3: sync + decrypt on the receiving side (page already waiting).
    let mut applied = 0usize;
    time_n(1, |_| {
        let report = run_room_server_sync_tick(
            &mut user_store,
            &mut user,
            &mut user_delivery,
            &RuntimeSyncOptions {
                key_package_target_available: 0,
                max_sync_pages_per_room: 64,
            },
            &code.server_url,
        )
        .unwrap();
        applied = report.applied_entries.len();
    })
    .report(&format!(
        "leg 3  receiver sync+decrypt+persist ({applied} entries, total)"
    ));

    // Leg 4: wake-hint latency — /sync/wait armed, then a publish lands.
    let waiter_url = server_url.clone();
    let waiter_room = room_id.clone();
    let after = published_seq;
    time_n(8, |i| {
        let url = waiter_url.clone();
        let watch = waiter_room.clone();
        let armed = std::thread::spawn(move || {
            let mut delivery = HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(url));
            delivery
                .sync_wait(&SyncWaitRequest {
                    rooms: vec![SyncWaitRoom {
                        room_id: watch,
                        after_seq: after + i as u64,
                    }],
                    invites: Vec::new(),
                    wait_ms: 10_000,
                })
                .unwrap()
        });
        std::thread::sleep(Duration::from_millis(20));
        let request = agent2
            .create_application_request(&room_id, &payload_1k, format!("bench-wake-{i}"))
            .unwrap();
        let send_at = Instant::now();
        agent2_delivery.append_event(&request, policy).unwrap();
        let woke = armed.join().unwrap();
        assert!(woke.woke);
        // Report publish→wake, excluding the staged 20ms arming sleep.
        print!("        publish→wake {:>10.2?}\r", send_at.elapsed());
        std::io::stdout().flush().unwrap();
    })
    .report("leg 4  arm wait + publish + wake (incl. 20ms arming)");

    // Leg 5: 32 KiB payload through the same publish path.
    let mut big = Vec::new();
    for i in 0..16 {
        big.push(
            agent2
                .create_application_request(&room_id, &payload_32k, format!("bench-big-{i}"))
                .unwrap(),
        );
    }
    let wire_big = serde_json::to_vec(&big[0]).unwrap();
    println!(
        "        wire size for 32 KiB ciphertext payload: {} bytes (×{:.1})",
        wire_big.len(),
        wire_big.len() as f64 / (32.0 * 1024.0)
    );
    time_n(16, |i| {
        agent2_delivery.append_event(&big[i], policy).unwrap();
    })
    .report("leg 5  HTTP POST /events with 32 KiB payload");

    // Leg 6: the real bridge — `hermes send` subprocess per message,
    // exactly what the Python adapter does today.
    time_n(16, |i| {
        cli(&[
            "send",
            "--request-json",
            &json!({
                "room_id": room_id,
                "conversation_id": null,
                "text": format!("bench message {i}"),
                "kind": "message",
                "status": "complete",
                "reply_to_message_id": null,
            })
            .to_string(),
        ]);
    })
    .report("leg 6  `hermes send` subprocess (spawn+store+encrypt+POST)");

    // Leg 6b: subprocess floor — the cheapest store-opening command.
    time_n(16, |_| {
        cli(&["invite", "--room-id", &room_id, "--json"]);
    })
    .report("leg 6b `hermes invite --json` subprocess (spawn+store open only)");

    // Leg 7: end-to-end wake-to-delivered — user holds a poll-shaped wait
    // then syncs, while the agent CLI sends.
    let mut e2e = Vec::with_capacity(8);
    for i in 0..8 {
        let after_seq = user
            .room_sync_cursors()
            .into_iter()
            .find(|cursor| cursor.room_id == room_id)
            .unwrap()
            .after_seq;
        let url = server_url.clone();
        let watch = room_id.clone();
        let armed = std::thread::spawn(move || {
            let mut delivery = HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(url));
            delivery
                .sync_wait(&SyncWaitRequest {
                    rooms: vec![SyncWaitRoom {
                        room_id: watch,
                        after_seq,
                    }],
                    invites: Vec::new(),
                    wait_ms: 10_000,
                })
                .unwrap()
        });
        std::thread::sleep(Duration::from_millis(20));
        let started = Instant::now();
        cli(&[
            "send",
            "--request-json",
            &json!({
                "room_id": room_id,
                "conversation_id": null,
                "text": format!("e2e {i}"),
                "kind": "message",
                "status": "complete",
                "reply_to_message_id": null,
            })
            .to_string(),
        ]);
        armed.join().unwrap();
        let report = run_room_server_sync_tick(
            &mut user_store,
            &mut user,
            &mut user_delivery,
            &options,
            &code.server_url,
        )
        .unwrap();
        e2e.push(started.elapsed());
        assert!(report.applied_entries.iter().any(|entry| matches!(
            &entry.entry,
            AppliedLogEntry::Application { plaintext, .. }
                if String::from_utf8_lossy(plaintext).contains("e2e")
        )));
    }
    Timings(e2e).report("leg 7  E2E hermes-send → user wake+sync+decrypted");

    println!(
        "\ntheoretical floor: 2× loopback RTT (~0.1 ms) + MLS encrypt+decrypt (leg 1 ×2) + WAL persist (leg 2 − serialize) — everything above that is overhead.\n"
    );
}
