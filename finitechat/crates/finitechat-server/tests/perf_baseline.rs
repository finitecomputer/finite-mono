//! Ignored timing harness for the perf plan (`docs/perf-plan.md`).
//!
//! Run with:
//! `cargo test --release -p finitechat-server --test perf_baseline -- --ignored --nocapture`
//!
//! Numbers are recorded in `docs/perf-log.md`; the tests assert nothing
//! about timing so CI never flakes on load.

use std::time::{Duration, Instant};

use finitechat_delivery::{HTTP_SERVER_SOURCE, HttpPublishTarget};
use finitechat_http::{GroupSyncRequest, PublishMessageRequest};
use finitechat_server::HttpServerState;
use finitechat_transport::transport::{
    Timestamp, TransportEnvelope, TransportMessage, TransportSource,
};
use finitechat_transport::{GroupId, MessageId};

const ROOMS: usize = 20;
const ENTRIES_PER_ROOM: usize = 500;
const HOT_ROOM_EXTRA_ENTRIES: usize = 2_000;
const PAYLOAD_BYTES: usize = 1_024;
const MEASURED_PUBLISHES: usize = 200;
const MEASURED_SYNCS: usize = 500;

fn group_message(room: usize, index: usize) -> TransportMessage {
    TransportMessage {
        id: MessageId::new(format!("perf-room-{room}-msg-{index}").into_bytes()),
        payload: vec![0xAB; PAYLOAD_BYTES],
        timestamp: Timestamp(1),
        causal_deps: Vec::new(),
        source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
        envelope: TransportEnvelope::GroupMessage {
            transport_group_id: format!("perf-transport-{room}").into_bytes(),
        },
    }
}

fn publish_request(room: usize, index: usize) -> PublishMessageRequest {
    PublishMessageRequest {
        target: HttpPublishTarget::Group {
            group_id: GroupId::new(format!("perf-room-{room}").into_bytes()),
            transport_group_id: format!("perf-transport-{room}").into_bytes(),
            commit_admission: None,
        },
        message: group_message(room, index),
        idempotency_key: None,
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

fn populate(state: &HttpServerState) {
    let started = Instant::now();
    for room in 0..ROOMS {
        for index in 0..ENTRIES_PER_ROOM {
            state
                .publish_message(publish_request(room, index))
                .expect("populate publish");
        }
    }
    for index in 0..HOT_ROOM_EXTRA_ENTRIES {
        state
            .publish_message(publish_request(0, ENTRIES_PER_ROOM + index))
            .expect("hot room publish");
    }
    let total = ROOMS * ENTRIES_PER_ROOM + HOT_ROOM_EXTRA_ENTRIES;
    println!(
        "populate: {total} entries across {ROOMS} rooms in {:?} ({:?}/publish avg)",
        started.elapsed(),
        started.elapsed() / total as u32,
    );
}

#[test]
#[ignore = "timing harness; run explicitly in release mode"]
fn server_publish_sync_and_startup_timings() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("perf.sqlite3");
    let state = HttpServerState::from_sqlite_path(&db).expect("open state");

    populate(&state);

    // Publish latency with a loaded server (hot room already deep).
    let mut samples = Vec::with_capacity(MEASURED_PUBLISHES);
    for index in 0..MEASURED_PUBLISHES {
        let request = publish_request(0, ENTRIES_PER_ROOM + HOT_ROOM_EXTRA_ENTRIES + index);
        let started = Instant::now();
        state.publish_message(request).expect("measured publish");
        samples.push(started.elapsed());
    }
    report("publish (loaded server, hot room)", samples);

    // Sync page latency near the tail of the hot room.
    let depth = (ENTRIES_PER_ROOM + HOT_ROOM_EXTRA_ENTRIES + MEASURED_PUBLISHES) as u64;
    let mut samples = Vec::with_capacity(MEASURED_SYNCS);
    for _ in 0..MEASURED_SYNCS {
        let started = Instant::now();
        let page = state
            .sync_group(GroupSyncRequest {
                group_id: GroupId::new(b"perf-room-0".to_vec()),
                after_seq: depth - 100,
                limit: 100,
                requester: None,
            })
            .expect("sync page");
        assert_eq!(page.entries.len(), 100);
        samples.push(started.elapsed());
    }
    report("sync page (100 entries at depth)", samples);

    // Sync page latency from the start of the hot room (worst-case scan).
    let mut samples = Vec::with_capacity(MEASURED_SYNCS);
    for _ in 0..MEASURED_SYNCS {
        let started = Instant::now();
        let page = state
            .sync_group(GroupSyncRequest {
                group_id: GroupId::new(b"perf-room-0".to_vec()),
                after_seq: 0,
                limit: 100,
                requester: None,
            })
            .expect("sync page");
        assert_eq!(page.entries.len(), 100);
        samples.push(started.elapsed());
    }
    report("sync page (100 entries from seq 0)", samples);

    // Startup replay cost for the accumulated op log (no snapshot yet).
    drop(state);
    let started = Instant::now();
    let reopened = HttpServerState::from_sqlite_path(&db).expect("reopen state");
    println!("startup full replay: {:?}", started.elapsed());

    // Startup with a fresh snapshot: only the (empty) tail replays.
    reopened.snapshot_now().expect("snapshot");
    drop(reopened);
    let started = Instant::now();
    let reopened = HttpServerState::from_sqlite_path(&db).expect("reopen from snapshot");
    println!("startup from snapshot: {:?}", started.elapsed());
    let page = reopened
        .sync_group(GroupSyncRequest {
            group_id: GroupId::new(b"perf-room-0".to_vec()),
            after_seq: 0,
            limit: 1,
            requester: None,
        })
        .expect("post-restart sync");
    assert_eq!(page.entries.len(), 1);
}
