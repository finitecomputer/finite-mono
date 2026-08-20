//! Boot-replay cost bound for `FiniteChatRuntime::open` (#596).
//!
//! Every resident start rebuilds the in-memory chat projection from the
//! durable message and event rows before the Hermes ready file is written.
//! The 2026-08-18 bridge-ready boot loop showed a ~15k-message room costing
//! ~55 CPU-bound seconds there. This test seeds a synthetic store with that
//! size class and bounds the open wall time, so a replay complexity
//! regression (for example a per-event whole-map rebuild) fails CI instead
//! of drifting back toward the cliff.
//!
//! Debug builds run the same replay without optimization, so the budget is
//! per-profile: both must stay far below the pre-fix ~55s (release) cost.

use std::time::{Duration, Instant};

use finitechat_client::{
    FiniteChatDevice, FiniteChatDeviceConfig, SqliteClientStore, SqliteClientStoreOptions,
    StoredAppEvent, StoredAppMessage,
};
use finitechat_core::{FiniteChatRuntime, OpenOptions};
use finitechat_hermes::{
    HERMES_MESSAGE_PAYLOAD_TYPE_V1, HermesMessagePayloadV1, HermesMessageStatusV1,
};
use finitechat_mls::NostrSecretKey;
use finitechat_proto::{DecryptedApplicationEventV1, DeviceRef, DurableAppEventKind};

/// Size class from the 2026-08-18 incident: ~15k messages in one room.
const MESSAGES: usize = 15_000;
const ROOM_ID: &str = "boot-replay-bench-room";
const DEVICE_ID: &str = "bench-device";

#[cfg(debug_assertions)]
const OPEN_BUDGET: Duration = Duration::from_secs(30);
#[cfg(not(debug_assertions))]
const OPEN_BUDGET: Duration = Duration::from_secs(15);

/// A fixed throwaway key: this is a synthetic local store, not a credential.
fn bench_secret() -> NostrSecretKey {
    NostrSecretKey::from_bytes([7; 32]).expect("fixed test secret is valid")
}

fn peer_sender() -> DeviceRef {
    DeviceRef {
        account_id: "aa".repeat(32),
        device_id: "peer-device".to_owned(),
    }
}

fn chat_event_plaintext(seq: usize) -> Vec<u8> {
    let hermes_payload = HermesMessagePayloadV1 {
        payload_type: HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
        conversation_id: Some("home".to_owned()),
        segment_id: None,
        text: format!("synthetic boot-replay message {seq}"),
        kind: finitechat_hermes::HermesSendKindV1::Message,
        status: HermesMessageStatusV1::Complete,
        edit_of: None,
        attachments: Vec::new(),
        reply_to_message_id: None,
        sender_name: None,
        metadata: Default::default(),
    };
    let event = DecryptedApplicationEventV1 {
        kind: DurableAppEventKind::ChatMessage,
        conversation_id: Some("home".to_owned()),
        segment_id: None,
        payload: serde_json::to_vec(&hermes_payload).expect("hermes payload serializes"),
    };
    serde_json::to_vec(&event).expect("application event serializes")
}

/// Seed a store whose boot replay cost mirrors a large hosted-agent room:
/// one room, `MESSAGES` chat messages saved as both message rows and event
/// rows, half sent by the owner and half by a peer, in room-seq order.
fn seed_store(data_dir: &std::path::Path, owner: &DeviceRef) {
    let secret = bench_secret();
    let options =
        SqliteClientStoreOptions::from_nostr_secret(&secret, DEVICE_ID).expect("store options");
    let mut store =
        SqliteClientStore::open(data_dir.join("client.sqlite3"), options).expect("store opens");

    let device = FiniteChatDevice::new(FiniteChatDeviceConfig {
        account_secret_key: secret,
        device_id: DEVICE_ID.to_owned(),
        now_unix_seconds: 1_800_000_000,
        credential_not_before_unix_seconds: 1_799_996_400,
        credential_not_after_unix_seconds: 1_803_600_000,
    })
    .expect("device mints");
    store
        .save_device_state(&device)
        .expect("device state saves");

    let peer = peer_sender();
    let mut messages = Vec::with_capacity(MESSAGES);
    let mut events = Vec::with_capacity(MESSAGES);
    for seq in 1..=MESSAGES {
        let sender = if seq % 2 == 0 {
            peer.clone()
        } else {
            owner.clone()
        };
        let message_id = format!("bench-message-{seq:06}");
        let plaintext = chat_event_plaintext(seq);
        let timestamp_unix_seconds = 1_800_000_000 + seq as u64;
        messages.push(StoredAppMessage {
            room_id: ROOM_ID.to_owned(),
            seq: seq as u64,
            message_id: message_id.clone(),
            sender: sender.clone(),
            plaintext: plaintext.clone(),
            timestamp_unix_seconds,
        });
        events.push(StoredAppEvent {
            room_id: ROOM_ID.to_owned(),
            seq: seq as u64,
            message_id,
            sender,
            plaintext,
            timestamp_unix_seconds,
        });
    }
    store
        .save_app_messages_and_events(owner, &messages, &events, u32::MAX)
        .expect("history saves");
    drop(store);
}

#[test]
fn boot_replay_bounded_for_large_room_history() {
    let dir = tempfile::tempdir().expect("temp dir");
    let secret = bench_secret();
    let account_id = hex::encode(secret.public_key().as_bytes());
    let owner = DeviceRef {
        account_id: account_id.clone(),
        device_id: DEVICE_ID.to_owned(),
    };
    seed_store(dir.path(), &owner);

    let started = Instant::now();
    let runtime = FiniteChatRuntime::open(OpenOptions {
        data_dir: dir.path().display().to_string(),
        server_url: "http://127.0.0.1:1".to_owned(),
        device_id: DEVICE_ID.to_owned(),
        account_secret_hex: Some(hex::encode(secret.as_bytes())),
        now_unix_seconds: Some(1_800_100_000),
    })
    .expect("runtime opens against the seeded store");
    let elapsed = started.elapsed();

    let state = runtime.state().expect("booted state is readable");
    assert_eq!(state.identity.account_id, account_id);

    println!("boot replay of {MESSAGES} messages took {elapsed:?} (budget {OPEN_BUDGET:?})");
    assert!(
        elapsed < OPEN_BUDGET,
        "FiniteChatRuntime::open took {elapsed:?} for a {MESSAGES}-message room; \
         boot replay has regressed past the {OPEN_BUDGET:?} budget (#596)"
    );
}
