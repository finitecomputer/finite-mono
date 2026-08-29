//! Delivery-ownership settlement gates at the Hermes CLI/sidecar boundary.
//!
//! The delivery-ownership swap (commits 54427832, 8f5ea22b, 30873332, and the
//! gate drop 96de2dcd) moved reply/edit route resolution, in-flight inbox
//! ownership, and delivery idempotency from the Python adapter's shadow state
//! into this sidecar, deleting the adapter-side regression layers with it.
//! This file restores the gates whose only remaining seam is the real CLI
//! boundary (a fresh process per invocation against durable stores):
//!
//! - reply routing by `thread_id` alone resolves against the DURABLE agent
//!   store in a fresh process — "restart after route learning preserves
//!   reply scope" under the new owner (no adapter route table exists anymore);
//! - an edit with no route fields is scoped from the running-turn file across
//!   process invocations — "outbound edit route";
//! - route behavior for a `thread_id` that resolves nowhere is pinned to the
//!   EXPLICIT `FINITECHAT_HERMES_UNKNOWN_THREAD_ROUTE` value only — `home`
//!   and `default` quietly route to Core's Home default ("intentional
//!   unscoped Home send stays quiet", and the warned variant of "unknown
//!   reply route warns before Home fallback"), any other set value fails
//!   with the typed error (the strict half of that layer) — never the unset
//!   default, whose semantics are owned by the sidecar, not by this suite;
//! - after an ack, a cursor rewind plus full durable recovery never
//!   resurrects the acked entry — "restart after processing before ack
//!   suppresses duplicate turn" (the recently-acked ring owning what the
//!   adapter dedup ring used to own).
//!
//! Documented gaps (would need src changes or harness support to pin here):
//! - the sidecar's warning text when an unknown thread routes to the Home
//!   default goes to process stderr, which a parallel test binary cannot
//!   capture safely — it is pinned by inspection only (`hermes.rs`'s
//!   `UnknownThreadRoutePolicy::HomeDefault` arm);
//! - the adapter-side route-table seams the deleted layers used to exercise
//!   no longer exist (the adapter owns no state store), so these gates pin
//!   the surviving sidecar-side behavior instead.
//!
//! Release-on-cancel redelivery itself (`hermes release` returning a leased
//! entry to `Pending`) is already pinned end to end in `hermes_flow.rs`
//! (`hermes_inbox_leases_on_delivery_and_settles_with_ack_or_release`) and at
//! the unit level in `src/hermes.rs`; the Python adapter mappings
//! cancel->release and failure->ack are pinned in
//! `finitechat/tests/hermes/test_adapter_settlement_gates.py`.

use finitechat_core::{AppAction, AppRoomState, FiniteChatRuntime, OpenOptions};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const USER_SECRET: [u8; 32] = [41; 32];

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
    let app = finitechat_server::http_router(
        finitechat_server::HttpServerState::from_sqlite_path(path).unwrap(),
    );
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

/// Agent + user + one connected room, mirroring the `hermes_flow.rs` join
/// choreography. Every later `cli_json` call is a fresh resolution pass over
/// the on-disk stores: that IS the simulated restart between steps.
struct SettlementFixture {
    agent_home: String,
    room_id: String,
    user: std::sync::Arc<FiniteChatRuntime>,
}

impl Drop for SettlementFixture {
    fn drop(&mut self) {
        let _ = self.user.dispatch_and_wait(AppAction::StopRuntime);
    }
}

fn open_settlement_fixture(dir: &Path, now: u64) -> SettlementFixture {
    let now_arg = now.to_string();
    let server_url = spawn_live_http_server(&dir.join("server.sqlite3"));
    let agent_home = dir.join("agent").display().to_string();

    cli_json(&[
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
        "Settlement Room",
    ]);
    let room_id = created["selected_room_id"].as_str().unwrap().to_owned();

    let user = FiniteChatRuntime::open(OpenOptions {
        data_dir: dir.join("user").display().to_string(),
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
    let joined = user
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("user claims Welcome");
    let user_room = joined
        .rooms
        .iter()
        .find(|room| room.room_id == room_id)
        .expect("user room projects");
    assert_eq!(user_room.state, AppRoomState::Connected);

    SettlementFixture {
        agent_home,
        room_id,
        user,
    }
}

/// The user creates a topic and speaks in its default chat; the agent sidecar
/// then delivers exactly that message as one leased inbound event carrying
/// the minted `thread_id`. Returns `(topic_id, chat_id, event)`.
fn routed_inbound_event(fixture: &mut SettlementFixture, text: &str) -> (String, String, Value) {
    fixture
        .user
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("user syncs before creating topic");
    fixture
        .user
        .dispatch_and_wait(AppAction::CreateTopic {
            room_id: fixture.room_id.clone(),
            title: "Builds".to_owned(),
        })
        .expect("user creates topic");
    let synced = fixture
        .user
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("topic syncs back");
    let topic = synced
        .topics
        .iter()
        .find(|topic| topic.title == "Builds")
        .expect("created topic projects");
    let topic_id = topic.topic_id.clone();

    fixture
        .user
        .dispatch_and_wait(AppAction::SendTopicMessage {
            room_id: fixture.room_id.clone(),
            topic_id: topic_id.clone(),
            text: text.to_owned(),
            metadata_json: None,
        })
        .expect("user sends topic message");
    // Chat id: the projection starts a default chat inside the topic; read it
    // back so assertions name the exact segment the sidecar must resolve.
    fixture
        .user
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("default chat syncs");
    let topics_after = fixture.user.state().unwrap().topics.clone();
    let chat_id = topics_after
        .iter()
        .find(|topic| topic.topic_id == topic_id)
        .and_then(|topic| topic.chats.first().map(|chat| chat.chat_id.clone()))
        .expect("default chat projects");

    let polled = cli_json(&[
        "hermes",
        "--home",
        &fixture.agent_home,
        "poll",
        "--request-json",
        &json!({ "timeout_millis": 5000 }).to_string(),
    ]);
    let events = polled["events"].as_array().cloned().unwrap_or_default();
    let event = events
        .into_iter()
        .find(|event| event["text"] == text)
        .unwrap_or_else(|| panic!("routed inbound {text:?} delivered; got no such event"));
    (topic_id, chat_id, event)
}

fn poll_events(fixture: &SettlementFixture, timeout_millis: u64) -> Vec<Value> {
    cli_json(&[
        "hermes",
        "--home",
        &fixture.agent_home,
        "poll",
        "--request-json",
        &json!({ "timeout_millis": timeout_millis }).to_string(),
    ])["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Sync-and-open repeatedly until `text` projects for the user: one
/// dispatch may observe only part of several queued deliveries.
fn sync_until_message(fixture: &SettlementFixture, text: &str) -> finitechat_core::ChatMessage {
    for _ in 0..30 {
        fixture
            .user
            .dispatch_and_wait(AppAction::StartRuntime)
            .expect("user runtime syncs");
        let state = fixture
            .user
            .dispatch_and_wait(AppAction::OpenRoom {
                room_id: fixture.room_id.clone(),
            })
            .expect("user opens room");
        if let Some(message) = state.messages.iter().find(|message| message.text == text) {
            return message.clone();
        }
    }
    let projected = fixture
        .user
        .dispatch_and_wait(AppAction::OpenRoom {
            room_id: fixture.room_id.clone(),
        })
        .expect("user opens room after giving up");
    let seen = projected
        .messages
        .iter()
        .map(|message| {
            (
                message.text.as_str(),
                message.conversation_id.as_deref(),
                message.chat_id.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    panic!("`{text}` never projected for the user; saw {seen:?}");
}

/// `(text, conversation_id, segment_id)` of every decrypted Hermes chat
/// payload in the user's durable event log, read from a scratch read-only
/// store connection. Home-default deliveries are provable here even when the
/// receiving projection hides them, so the unknown-thread gates can state
/// exactly what was delivered and where.
fn user_durable_hermes_deliveries(
    user_dir: &Path,
    now: u64,
) -> Vec<(String, Option<String>, Option<String>)> {
    use finitechat_client::{FiniteChatDeviceConfig, SqliteClientStore, SqliteClientStoreOptions};
    use finitechat_mls::NostrSecretKey;

    let secret = NostrSecretKey::from_bytes(USER_SECRET).expect("user secret is a nostr key");
    let options =
        SqliteClientStoreOptions::from_nostr_secret(&secret, "ios-user").expect("store options");
    let store = SqliteClientStore::open_read_only(user_dir.join("client.sqlite3"), options)
        .expect("read-only user store opens alongside the resident writer");
    let config = FiniteChatDeviceConfig {
        account_secret_key: secret,
        device_id: "ios-user".to_owned(),
        now_unix_seconds: now,
        credential_not_before_unix_seconds: now.saturating_sub(3600),
        credential_not_after_unix_seconds: now + 90 * 24 * 60 * 60,
    };
    let device = store.load_device(config).expect("device loads");
    let events = store
        .load_app_events(device.device_ref(), u32::MAX)
        .expect("durable app events load");
    events
        .iter()
        .filter_map(|stored| {
            let event: finitechat_proto::DecryptedApplicationEventV1 =
                serde_json::from_slice(&stored.plaintext).ok()?;
            let payload =
                finitechat_hermes::HermesMessagePayloadV1::decode(&event.payload).ok()??;
            Some((payload.text, payload.conversation_id, payload.segment_id))
        })
        .collect()
}

/// An adapter-shaped `hermes send` request: only the Hermes thread identity,
/// never explicit route fields, so the sidecar must resolve the scope.
fn send_request(fixture: &SettlementFixture, thread_id: Value, text: &str) -> String {
    json!({
        "room_id": fixture.room_id,
        "conversation_id": null,
        "segment_id": null,
        "thread_id": thread_id,
        "text": text,
        "kind": "message",
        "status": "complete",
        "reply_to_message_id": null,
        "metadata": {},
    })
    .to_string()
}

/// An adapter-shaped `hermes send` whose status marks a still-running turn so
/// the running-turn file records the outbound marker.
fn running_send_request(fixture: &SettlementFixture, thread_id: &str, text: &str) -> String {
    let mut request = serde_json::from_slice::<Value>(
        send_request(fixture, Value::String(thread_id.to_owned()), text).as_bytes(),
    )
    .unwrap();
    request["status"] = json!("running");
    request.to_string()
}

/// An adapter-shaped `hermes edit` with no route fields at all: scope and
/// kind must come from the running-turn file lookup by (room, message_id).
fn edit_request(
    fixture: &SettlementFixture,
    message_id: &str,
    text: &str,
    finalize: bool,
) -> String {
    json!({
        "room_id": fixture.room_id,
        "conversation_id": null,
        "segment_id": null,
        "thread_id": null,
        "message_id": message_id,
        "text": text,
        "kind": "message",
        "status": if finalize { "complete" } else { "running" },
        "finalize": finalize,
        "metadata": {},
    })
    .to_string()
}

/// Ownership O2 restart gate ("restart after route learning preserves reply
/// scope"): the reply names ONLY the sidecar-minted `thread_id`; each fresh
/// CLI invocation resolves it against the durable agent store — the store the
/// deleted adapter SQLite route table used to shadow — and the reply lands
/// scoped in the exact originating topic chat.
#[test]
fn thread_id_reply_resolves_from_the_durable_store_across_restarts() {
    ensure_test_finite_home();
    let dir = tempfile::tempdir().unwrap();
    let now = test_now();
    let mut fixture = open_settlement_fixture(dir.path(), now);
    let (topic_id, chat_id, inbound) = routed_inbound_event(&mut fixture, "please build the fence");

    let conversation_id = inbound["conversation_id"].as_str().expect("topic route");
    let segment_id = inbound["segment_id"].as_str().expect("segment route");
    let thread_id = inbound["source"]["thread_id"]
        .as_str()
        .expect("minted thread");
    assert_eq!(
        conversation_id, topic_id,
        "inbound carries its conversation"
    );
    assert_eq!(segment_id, chat_id, "inbound carries its segment");
    assert_eq!(
        thread_id, segment_id,
        "the sidecar mints thread_id as segment_id.or(conversation_id)"
    );

    let sent = cli_json(&[
        "hermes",
        "--home",
        &fixture.agent_home,
        "send",
        "--request-json",
        &send_request(&fixture, Value::String(thread_id.to_owned()), "fence built"),
    ]);
    assert!(
        sent["message_id"].as_str().is_some_and(|id| !id.is_empty()),
        "the routed reply send reports its message id"
    );

    // The reply arrived still scoped to the origin topic chat: the persisted
    // route survived every process boundary in between.
    let reply = sync_until_message(&fixture, "fence built");
    assert_eq!(reply.conversation_id.as_deref(), Some(topic_id.as_str()));
    assert_eq!(reply.chat_id.as_deref(), Some(chat_id.as_str()));
}

/// Ownership O2 edit gate ("outbound edit route"): with no explicit route or
/// thread, every streamed revision and the finalize re-derive their scope
/// from the running-turn record of the original send, across fresh process
/// invocations — the mapping the deleted adapter outbound-route LRU owned.
#[test]
fn edit_without_route_fields_resolves_scope_from_the_running_turn_file() {
    ensure_test_finite_home();
    let dir = tempfile::tempdir().unwrap();
    let now = test_now();
    let mut fixture = open_settlement_fixture(dir.path(), now);
    let (_topic_id, chat_id, inbound) = routed_inbound_event(&mut fixture, "draft me a plan");
    let thread_id = inbound["source"]["thread_id"].as_str().unwrap().to_owned();

    // Turn start (fresh process): running status records the original.
    let started = cli_json(&[
        "hermes",
        "--home",
        &fixture.agent_home,
        "send",
        "--request-json",
        &running_send_request(&fixture, &thread_id, "plan draft v1"),
    ]);
    let sent_message_id = started["message_id"].as_str().unwrap().to_owned();

    // Streamed revision (another fresh process): no route fields at all.
    cli_json(&[
        "hermes",
        "--home",
        &fixture.agent_home,
        "edit",
        "--request-json",
        &edit_request(&fixture, &sent_message_id, "plan draft v2", false),
    ]);

    // Finalize (a third fresh process): scope STILL resolves without any
    // route fields, and the running-turn entry is consumed.
    cli_json(&[
        "hermes",
        "--home",
        &fixture.agent_home,
        "edit",
        "--request-json",
        &edit_request(&fixture, &sent_message_id, "final plan", true),
    ]);

    // Both revisions must project still scoped to the originating chat, each
    // chained onto the original send by `edit_of` — scope and chain both
    // re-derived from disk in processes that never saw the send.
    let revision = sync_until_message(&fixture, "plan draft v2");
    assert_eq!(
        revision.edit_of_message_id.as_deref(),
        Some(sent_message_id.as_str()),
        "the streamed revision chains onto the original send"
    );
    assert_eq!(revision.chat_id.as_deref(), Some(chat_id.as_str()));

    let final_message = sync_until_message(&fixture, "final plan");
    assert_eq!(
        final_message.edit_of_message_id.as_deref(),
        Some(sent_message_id.as_str()),
        "the finalize edit chains onto the original send"
    );
    assert!(
        final_message.conversation_id.is_some(),
        "the finalized turn kept its conversation scope"
    );
    assert_eq!(
        final_message.chat_id.as_deref(),
        Some(chat_id.as_str()),
        "the whole running turn stayed scoped to its originating chat"
    );
}

/// Gate ("unknown reply route warns before Home fallback" / "intentional
/// unscoped Home send stays quiet"), pinned against EXPLICIT env values only:
/// `home` and `default` route a nowhere-resolving `thread_id` to Core's Home
/// default quietly (and never let it adopt the inbound thread's scope), while
/// any other set value — spelled here as `error` — is the strict typed
/// failure. The UNSET default is deliberately never asserted: it belongs to
/// the sidecar's product policy (a sibling change flips it from strict error
/// to home fallback), and this suite must hold on either side of that flip.
/// This test solely owns the process-global policy env var; no other test in
/// this binary depends on the policy (their threads always resolve).
#[test]
fn unknown_thread_route_follows_the_explicit_env_policy_never_an_unset_default() {
    ensure_test_finite_home();
    let dir = tempfile::tempdir().unwrap();
    let now = test_now();
    let mut fixture = open_settlement_fixture(dir.path(), now);
    let (inbound_topic, _inbound_chat, inbound) = routed_inbound_event(&mut fixture, "route me");
    let known_thread = inbound["source"]["thread_id"].as_str().unwrap().to_owned();
    const POLICY_ENV: &str = "FINITECHAT_HERMES_UNKNOWN_THREAD_ROUTE";
    let unknown_thread_send = |text: &str| {
        // SAFETY: single-threaded ownership of a test-only environment
        // variable; callers below restore or overwrite it around each phase.
        unsafe { std::env::set_var(POLICY_ENV, "error") };
        let refused = finitechat_cli::run(
            [
                "hermes".to_owned(),
                "--home".to_owned(),
                fixture.agent_home.clone(),
                "send".to_owned(),
                "--request-json".to_owned(),
                send_request(&fixture, json!("chat-does-not-exist-anywhere"), text),
            ],
            &mut Vec::new(),
        )
        .expect_err("strict policy (`=error`) must fail an unknown thread closed");
        let refusal = refused.to_string();
        assert!(
            refusal.contains("unknown thread_id"),
            "refusal names the unknown thread, got: {refusal}"
        );
    };

    // Strict policy spelled explicitly: the same unknown thread fails with a
    // typed error naming it... while the KNOWN thread keeps resolving under
    // the identical policy value.
    unknown_thread_send("must not deliver");
    let resolved = cli_json(&[
        "hermes",
        "--home",
        &fixture.agent_home,
        "send",
        "--request-json",
        &send_request(&fixture, Value::String(known_thread.clone()), "scoped ok"),
    ]);
    assert!(resolved["message_id"].as_str().is_some());

    // An intentional unscoped send carries no thread at all and stays quiet
    // EVEN under the strict policy: Core's default only ever sees deliberate
    // Home traffic.
    unsafe { std::env::set_var(POLICY_ENV, "error") };
    let quiet_home = cli_json(&[
        "hermes",
        "--home",
        &fixture.agent_home,
        "send",
        "--request-json",
        &send_request(&fixture, Value::Null, "home base chatter"),
    ]);
    assert!(quiet_home["message_id"].as_str().is_some());

    // Opted-in fallback, both accepted spellings: the very same unknown
    // thread quietly routes to the Home default under `home`...
    for policy in ["home", "default"] {
        unsafe { std::env::set_var(POLICY_ENV, policy) };
        let home_routed = cli_json(&[
            "hermes",
            "--home",
            &fixture.agent_home,
            "send",
            "--request-json",
            &send_request(
                &fixture,
                json!("chat-does-not-exist-anywhere"),
                if policy == "home" {
                    "quietly home"
                } else {
                    "default spelling too"
                },
            ),
        ]);
        assert!(
            home_routed["message_id"].as_str().is_some(),
            "`{policy}` must route the unknown thread to the Home default"
        );
    }
    // ...and back under the explicit strict policy the unknown thread fails
    // again, proving the routing followed the env VALUE, not a one-way latch.
    unknown_thread_send("still refused");

    // SAFETY: restoring the pre-test state after this test owns-it window.
    unsafe { std::env::remove_var(POLICY_ENV) };

    // Ground truth in the user's DURABLE decrypted event log (projections
    // are a UI view; delivery is the contract): the known-thread reply
    // resolves to the inbound's conversation, each Home-default delivery
    // arrives WITHOUT adopting the refused thread's conversation, and the
    // strictly-refused sends exist nowhere at all.
    fixture
        .user
        .dispatch_and_wait(AppAction::StartRuntime)
        .expect("user syncs final deliveries");
    let deliveries = user_durable_hermes_deliveries(&dir.path().join("user"), now);
    let scope_of = |text: &str| {
        deliveries
            .iter()
            .find(|(delivered_text, _, _)| delivered_text == text)
            .unwrap_or_else(|| panic!("{text:?} was durably delivered"))
    };
    assert_eq!(
        scope_of("scoped ok").1.as_deref(),
        Some(inbound_topic.as_str()),
        "the known thread still resolves to its conversation under the strict policy"
    );
    for text in ["quietly home", "default spelling too", "home base chatter"] {
        let (_, conversation_id, _) = scope_of(text);
        assert_ne!(
            conversation_id.as_deref(),
            Some(inbound_topic.as_str()),
            "{text:?} must not inherit the inbound thread's conversation"
        );
    }
    assert!(
        deliveries.iter().all(
            |(delivered_text, _, _)| delivered_text != "must not deliver"
                && delivered_text != "still refused"
        ),
        "the refused thread's messages must not exist anywhere"
    );
}

/// Idempotency gate ("restart after processing before ack suppresses
/// duplicate turn"): once an entry is acked, even a cursor rewind plus the
/// full durable recovery path must not resurrect it — the recently-acked
/// ring in `hermes-inbox.json` owns what the deleted adapter SQLite dedup
/// ring owned. The negative control rewinds the cursor AND drops the ring:
/// the same recovery then redelivers, proving which mechanism suppressed it.
#[test]
fn durable_recovery_never_resurrects_an_acked_entry_even_when_the_cursor_is_behind() {
    ensure_test_finite_home();
    let dir = tempfile::tempdir().unwrap();
    let now = test_now();
    let mut fixture = open_settlement_fixture(dir.path(), now);
    let (_topic_id, _chat_id, inbound) = routed_inbound_event(&mut fixture, "acked once");
    let seq = inbound["seq"].as_u64().unwrap();
    let room_id = inbound["room_id"].as_str().unwrap().to_owned();

    cli_json(&[
        "hermes",
        "--home",
        &fixture.agent_home,
        "ack",
        "--request-json",
        &json!({
            "room_id": room_id,
            "seq": seq,
            "message_id": inbound["message_id"],
        })
        .to_string(),
    ]);
    assert!(
        poll_events(&fixture, 0).is_empty(),
        "an acked entry stays settled before any rewind"
    );

    let inbox_path = Path::new(&fixture.agent_home).join("hermes-inbox.json");
    let mut inbox: Value =
        serde_json::from_slice(&std::fs::read(&inbox_path).expect("inbox persists")).unwrap();

    // Simulated crash-with-lost-cursor: the cursor forgets everything while
    // the acked ring survives. Recovery re-reads stored events behind the
    // cursor but must not re-enqueue the acked key.
    inbox["cursors"][room_id.as_str()] = json!(0);
    std::fs::write(&inbox_path, serde_json::to_vec_pretty(&inbox).unwrap()).unwrap();
    assert!(
        poll_events(&fixture, 0).is_empty(),
        "durable recovery behind a rewound cursor must not resurrect an acked entry"
    );
    let mut inbox: Value = serde_json::from_slice(&std::fs::read(&inbox_path).unwrap()).unwrap();
    assert!(
        inbox["cursors"][room_id.as_str()].as_u64().unwrap() >= seq,
        "recovery advanced the cursor past the acked seq instead of delivering it"
    );

    // Negative control: same rewind without the ring — the entry returns,
    // proving the ring (not the cursor) was the suppressing mechanism.
    inbox["acked"] = json!([]);
    inbox["cursors"][room_id.as_str()] = json!(0);
    std::fs::write(&inbox_path, serde_json::to_vec_pretty(&inbox).unwrap()).unwrap();
    let redelivered = poll_events(&fixture, 0);
    assert!(
        redelivered
            .iter()
            .any(|event| event["message_id"] == inbound["message_id"]),
        "without the acked ring the recovered entry redelivers (control)"
    );
}
