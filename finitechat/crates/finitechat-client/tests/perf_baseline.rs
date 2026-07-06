//! Ignored timing harness for the perf plan (`docs/perf-plan.md`).
//!
//! Run with:
//! `cargo test --release -p finitechat-client --test perf_baseline -- --ignored --nocapture`

use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use finitechat_client::{
    FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery, HttpRuntimeTransport,
    RuntimeDelivery, RuntimeSyncOptions, SqliteClientStore, SqliteClientStoreOptions,
    run_runtime_sync_tick,
};
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::{CreateRoomRequest, DurableAppEventKind, RoomProtocol};
use finitechat_server::{HttpServerState, http_router};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt;

const MESSAGES: usize = 300;
const SAVE_SAMPLES: usize = 100;
const NOW: u64 = 1_800_000_000;

struct BenchTransport {
    app: Router,
    runtime: tokio::runtime::Runtime,
}

#[derive(Debug)]
struct BenchTransportError(String);

impl std::fmt::Display for BenchTransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl HttpRuntimeTransport for BenchTransport {
    type Error = BenchTransportError;

    fn post_json<T, R>(&mut self, uri: &str, body: &T) -> Result<R, Self::Error>
    where
        T: Serialize,
        R: DeserializeOwned,
    {
        self.runtime.block_on(async {
            let request = Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(body)
                        .map_err(|error| BenchTransportError(error.to_string()))?,
                ))
                .map_err(|error| BenchTransportError(error.to_string()))?;
            let response = self
                .app
                .clone()
                .oneshot(request)
                .await
                .map_err(|error| BenchTransportError(error.to_string()))?;
            let status = response.status();
            let bytes = to_bytes(response.into_body(), usize::MAX)
                .await
                .map_err(|error| BenchTransportError(error.to_string()))?;
            if status != StatusCode::OK {
                return Err(BenchTransportError(format!(
                    "status {status}: {}",
                    String::from_utf8_lossy(&bytes)
                )));
            }
            serde_json::from_slice(&bytes).map_err(|error| BenchTransportError(error.to_string()))
        })
    }
}

fn delivery(path: &std::path::Path) -> HttpRuntimeDelivery<BenchTransport> {
    HttpRuntimeDelivery::new(BenchTransport {
        app: http_router(HttpServerState::from_sqlite_path(path).unwrap()),
        runtime: tokio::runtime::Runtime::new().unwrap(),
    })
}

fn config(secret: u8, device_id: &str) -> FiniteChatDeviceConfig {
    FiniteChatDeviceConfig {
        account_secret_key: NostrSecretKey::from_bytes([secret; NOSTR_SECRET_KEY_BYTES]).unwrap(),
        device_id: device_id.to_string(),
        now_unix_seconds: NOW,
        credential_not_before_unix_seconds: NOW - 60,
        credential_not_after_unix_seconds: NOW + 60,
    }
}

fn percentile(sorted: &[Duration], percentile: f64) -> Duration {
    let index = ((sorted.len() as f64 - 1.0) * percentile).round() as usize;
    sorted[index]
}

fn report(label: &str, mut samples: Vec<Duration>) {
    samples.sort();
    println!(
        "{label}: n={} p50={:?} p90={:?} p99={:?} max={:?}",
        samples.len(),
        percentile(&samples, 0.50),
        percentile(&samples, 0.90),
        percentile(&samples, 0.99),
        samples.last().unwrap(),
    );
}

#[test]
#[ignore = "timing harness; run explicitly in release mode"]
fn client_sync_tick_and_save_timings() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("perf-server.sqlite3");
    let room_id = "perf_room";
    let mls_group_id = "perf_mls_group";

    let alice_config = config(17, "alice_perf");
    let mut alice_store = SqliteClientStore::open(
        dir.path().join("alice.sqlite3"),
        SqliteClientStoreOptions::from_nostr_secret(
            &alice_config.account_secret_key,
            &alice_config.device_id,
        )
        .unwrap(),
    )
    .unwrap();
    let mut alice = FiniteChatDevice::new(alice_config.clone()).unwrap();
    let mut bob = FiniteChatDevice::new(config(19, "bob_perf")).unwrap();

    let mut server = delivery(&server_db);

    // Bootstrap a two-member room over the HTTP routes.
    bob.create_group_state(room_id, mls_group_id).unwrap();
    server
        .bootstrap_account_room(&CreateRoomRequest {
            room_id: room_id.to_owned(),
            mls_group_id: mls_group_id.to_owned(),
            creator: bob.device_ref().clone(),
            protocol: RoomProtocol::default(),
        })
        .unwrap();
    server
        .upload_key_package(alice.upload_key_package_request("kp_perf_alice").unwrap())
        .unwrap();
    let claimed = server
        .claim_key_package_for_device(alice.device_ref())
        .unwrap()
        .expect("alice key package");
    let prepared = bob
        .prepare_add_member_commit(room_id, &claimed, "welcome_perf_alice", "perf_add_alice")
        .unwrap();
    let accepted = server.submit_commit(prepared.request).unwrap();
    let page = server.sync_events(room_id, bob.device_ref(), 0).unwrap();
    bob.merge_pending_commit_from_log(room_id, &page.entries, &prepared.message_id)
        .unwrap();

    // Alice joins via the welcome through one sync tick.
    alice_store.save_device_state(&alice).unwrap();
    let join_options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 1,
    };
    let join =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut server, &join_options).unwrap();
    assert_eq!(join.claimed_welcomes, 1);
    assert_eq!(alice.last_applied_seq(room_id).unwrap(), accepted.seq);

    // Bob publishes the backlog alice will sync.
    let populate_started = Instant::now();
    for index in 0..MESSAGES {
        let plaintext = format!(r#"{{"type":"finitecomputer.command.v1","body":{{"n":{index}}}}}"#);
        let request = bob
            .create_application_request(room_id, plaintext.as_bytes(), format!("perf_msg_{index}"))
            .unwrap();
        server
            .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
            .unwrap();
    }
    println!(
        "populate: {MESSAGES} MLS messages in {:?} ({:?}/message avg)",
        populate_started.elapsed(),
        populate_started.elapsed() / MESSAGES as u32,
    );

    // The measured tick: alice applies the backlog (decrypt + persist).
    let sync_options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 8,
    };
    let started = Instant::now();
    let report_tick =
        run_runtime_sync_tick(&mut alice_store, &mut alice, &mut server, &sync_options).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(report_tick.applied_entries.len(), MESSAGES);
    println!(
        "sync tick: applied {MESSAGES} entries in {:?} ({:?}/entry avg)",
        elapsed,
        elapsed / MESSAGES as u32,
    );

    // Isolated cost of one full-state save at this state size.
    let mut samples = Vec::with_capacity(SAVE_SAMPLES);
    for _ in 0..SAVE_SAMPLES {
        let started = Instant::now();
        alice_store.save_device_state(&alice).unwrap();
        samples.push(started.elapsed());
    }
    report("save_device_state (joined room, post-sync state)", samples);
}
