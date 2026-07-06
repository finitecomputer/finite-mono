//! End-to-end exercise of the `hermes` subcommand family against a live
//! server: init → invite (URL/QR) → a user device joins via the invite
//! → poll admits the join → send/edit/activity round trips. This is the
//! same surface the Python platform plugin shells to.

use finitechat_client::{
    AppliedLogEntry, FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery,
    ReqwestHttpRuntimeTransport, RuntimeSyncOptions, SqliteClientStore, SqliteClientStoreOptions,
    finalize_invited_room, run_runtime_sync_tick, submit_invite_join_request,
};
use finitechat_core::{AppAction, AppRoomState, ChatMediaKind, FiniteChatRuntime, OpenOptions};
use finitechat_hermes::{HermesMessagePayloadV1, HermesMessageStatusV1, HermesSendKindV1};
use finitechat_http::GetEphemeralActivitiesRequest;
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::{
    DecryptedApplicationEventV1, DecryptedEphemeralActivityV1, DurableAppEventKind, InviteCodeV1,
};
use finitechat_server::{HttpServerState, http_router};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const USER_SECRET: [u8; NOSTR_SECRET_KEY_BYTES] = [41; NOSTR_SECRET_KEY_BYTES];
const USER2_SECRET: [u8; NOSTR_SECRET_KEY_BYTES] = [42; NOSTR_SECRET_KEY_BYTES];
const APP_USER_SECRET: [u8; NOSTR_SECRET_KEY_BYTES] = [43; NOSTR_SECRET_KEY_BYTES];

/// Point `FINITE_HOME` at a process-wide throwaway directory so in-process
/// CLI calls never mint or read the developer's real shared identity. The
/// agent under test shares this one identity across tests in this binary;
/// simulated user devices pass explicit secrets or run as subprocesses with
/// their own `FINITE_HOME`.
fn ensure_test_finite_home() -> PathBuf {
    use std::sync::OnceLock;
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let dir = tempfile::tempdir().expect("test FINITE_HOME tempdir");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        // SAFETY: set once before any identity resolution in this process;
        // every entry point into the CLI in this file calls this helper.
        unsafe { std::env::set_var("FINITE_HOME", &path) };
        path
    })
    .clone()
}

fn decode_wrapped_hermes_payload(plaintext: &[u8]) -> HermesMessagePayloadV1 {
    let event: DecryptedApplicationEventV1 =
        serde_json::from_slice(plaintext).expect("typed chat app event");
    assert_eq!(event.kind, DurableAppEventKind::ChatMessage);
    HermesMessagePayloadV1::decode(&event.payload)
        .expect("valid Hermes payload")
        .expect("Hermes message payload")
}

fn wrapped_hermes_payload_text_is(plaintext: &[u8], expected: &str) -> bool {
    serde_json::from_slice::<DecryptedApplicationEventV1>(plaintext)
        .ok()
        .filter(|event| event.kind == DurableAppEventKind::ChatMessage)
        .and_then(|event| {
            HermesMessagePayloadV1::decode(&event.payload)
                .ok()
                .flatten()
        })
        .is_some_and(|payload| payload.text == expected)
}

/// The account secret of the process-wide shared test identity, read from
/// the contract-v1 identity file the CLI minted.
fn shared_identity_secret_hex() -> String {
    let path = ensure_test_finite_home()
        .join("identity")
        .join("identity.json");
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    value["secret_hex"].as_str().unwrap().to_owned()
}

/// Run the real `finitechat` binary with its own `FINITE_HOME` (a distinct
/// Finite identity), for flows that need a second account on the CLI surface.
fn finitechat_bin_json(finite_home: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_finitechat"))
        .env("FINITE_HOME", finite_home)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "finitechat {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("finitechat {args:?} produced invalid JSON: {error}"))
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
    let health_url = format!("{server_url}/health");
    let client = reqwest::blocking::Client::new();
    for _ in 0..100 {
        if client
            .get(&health_url)
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

fn cli_json(args: &[&str]) -> Value {
    ensure_test_finite_home();
    let mut output = Vec::new();
    finitechat_cli::run(args.iter().map(|arg| arg.to_string()), &mut output)
        .unwrap_or_else(|error| panic!("finitechat {args:?} failed: {error}"));
    serde_json::from_slice(&output)
        .unwrap_or_else(|error| panic!("finitechat {args:?} produced invalid JSON: {error}"))
}

fn hermes(args: &[&str]) -> Value {
    cli_json(args)
}

fn hermes_ack(home_arg: &str, event: &Value) -> Value {
    hermes(&[
        "hermes",
        "--home",
        home_arg,
        "ack",
        "--request-json",
        &json!({
            "room_id": event["room_id"].clone(),
            "seq": event["seq"].clone(),
            "message_id": event["message_id"].clone(),
        })
        .to_string(),
    ])
}

fn hermes_send_text(home_arg: &str, room_id: &str, text: &str) -> Value {
    hermes(&[
        "hermes",
        "--home",
        home_arg,
        "send",
        "--request-json",
        &json!({
            "room_id": room_id,
            "conversation_id": null,
            "text": text,
            "kind": "message",
            "status": "complete",
            "reply_to_message_id": null,
        })
        .to_string(),
    ])
}

fn hermes_poll(home_arg: &str, request: Value) -> Value {
    hermes(&[
        "hermes",
        "--home",
        home_arg,
        "poll",
        "--request-json",
        &request.to_string(),
    ])
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

struct TestHermesUser {
    store: SqliteClientStore,
    device: FiniteChatDevice,
    delivery: HttpRuntimeDelivery<ReqwestHttpRuntimeTransport>,
}

impl TestHermesUser {
    fn new(
        db_path: &Path,
        server_url: &str,
        account_secret: [u8; NOSTR_SECRET_KEY_BYTES],
        device_id: &str,
    ) -> Self {
        let config = FiniteChatDeviceConfig {
            account_secret_key: NostrSecretKey::from_bytes(account_secret).unwrap(),
            device_id: device_id.to_owned(),
            now_unix_seconds: now_ms() / 1000,
            credential_not_before_unix_seconds: now_ms() / 1000 - 3600,
            credential_not_after_unix_seconds: now_ms() / 1000 + 86400,
        };
        let mut store = SqliteClientStore::open(
            db_path,
            SqliteClientStoreOptions::from_nostr_secret(
                &config.account_secret_key,
                &config.device_id,
            )
            .unwrap(),
        )
        .unwrap();
        let device = FiniteChatDevice::new(config).unwrap();
        store.save_device_state(&device).unwrap();
        let delivery =
            HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.to_owned()));
        Self {
            store,
            device,
            delivery,
        }
    }

    fn submit_invite_join(&mut self, code: &InviteCodeV1, display_name: &str) {
        submit_invite_join_request(
            &mut self.store,
            &mut self.device,
            &mut self.delivery,
            code,
            Some(display_name.to_owned()),
            now_ms(),
        )
        .unwrap();
    }

    fn finalize_invite(&mut self, code: &InviteCodeV1, options: &RuntimeSyncOptions) {
        run_runtime_sync_tick(
            &mut self.store,
            &mut self.device,
            &mut self.delivery,
            options,
        )
        .unwrap();
        finalize_invited_room(&mut self.store, &mut self.device, code).unwrap();
    }

    fn send_hermes_message(
        &mut self,
        room_id: &str,
        text: &str,
        idempotency_key: &str,
        sender_name: &str,
    ) {
        let payload = HermesMessagePayloadV1 {
            payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: None,
            text: text.to_owned(),
            kind: HermesSendKindV1::Message,
            status: HermesMessageStatusV1::Complete,
            edit_of: None,
            attachments: Vec::new(),
            reply_to_message_id: None,
            sender_name: Some(sender_name.to_owned()),
            metadata: Default::default(),
        };
        let request = self
            .device
            .create_application_request(room_id, &payload.encode().unwrap(), idempotency_key)
            .unwrap();
        self.store.save_device_state(&self.device).unwrap();
        self.delivery
            .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
            .unwrap();
    }
}

struct SmokeReport {
    name: String,
    path: Option<PathBuf>,
    started: Instant,
    facts: serde_json::Map<String, Value>,
    steps: Vec<Value>,
}

impl SmokeReport {
    fn from_env(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            path: std::env::var_os("FINITE_HERMES_SIDECAR_SMOKE_REPORT").map(PathBuf::from),
            started: Instant::now(),
            facts: serde_json::Map::new(),
            steps: Vec::new(),
        }
    }

    fn fact(&mut self, key: &str, value: Value) {
        if self.path.is_some() {
            self.facts.insert(key.to_owned(), value);
        }
    }

    fn time<T>(&mut self, name: &str, action: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let value = action();
        self.step(name, started.elapsed());
        value
    }

    fn step(&mut self, name: &str, elapsed: Duration) {
        if self.path.is_some() {
            self.steps.push(json!({
                "name": name,
                "elapsed_ms": elapsed.as_millis() as u64,
            }));
        }
    }

    fn finish(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create smoke report directory");
        }
        let report = json!({
            "status": "passed",
            "name": self.name,
            "elapsed_ms": self.started.elapsed().as_millis() as u64,
            "facts": self.facts,
            "steps": self.steps,
        });
        std::fs::write(path, serde_json::to_string_pretty(&report).unwrap())
            .expect("write Hermes sidecar smoke report");
        println!("Hermes sidecar smoke report: {}", path.display());
    }
}

#[test]
fn hermes_init_reuses_imported_shared_identity() {
    // Runs the real binary with a dedicated FINITE_HOME: `auth import`
    // writes an existing secret into the shared identity location, and
    // every later command (status, hermes init) finds the same account.
    let dir = tempfile::tempdir().unwrap();
    let finite_home = dir.path().join("finite-home");
    let home_arg = dir.path().join("agent-home").display().to_string();

    let secret_file = dir.path().join("imported.nsec");
    std::fs::write(&secret_file, format!("{}\n", "17".repeat(32))).unwrap();
    let imported = finitechat_bin_json(
        &finite_home,
        &[
            "auth",
            "import",
            "--file",
            &secret_file.display().to_string(),
        ],
    );
    assert_eq!(imported["imported"], true);
    let identity_file = finite_home.join("identity").join("identity.json");
    assert!(identity_file.is_file());
    assert_eq!(
        imported["identity_file"],
        identity_file.display().to_string()
    );

    // Importing again refuses to overwrite the single storage location.
    let rerun = Command::new(env!("CARGO_BIN_EXE_finitechat"))
        .env("FINITE_HOME", &finite_home)
        .args(["auth", "import", "--file"])
        .arg(&secret_file)
        .output()
        .unwrap();
    assert!(!rerun.status.success());
    assert!(
        String::from_utf8_lossy(&rerun.stderr).contains("refusing to overwrite"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&rerun.stderr)
    );

    let status = finitechat_bin_json(&finite_home, &["auth", "status"]);
    assert_eq!(status["account_id"], imported["account_id"]);
    assert_eq!(status["npub"], imported["npub"]);
    assert!(
        status["created_by"]
            .as_str()
            .unwrap()
            .contains("auth import")
    );

    let hermes_init = finitechat_bin_json(
        &finite_home,
        &[
            "hermes",
            "--agent-home",
            &home_arg,
            "init",
            "--server",
            "http://127.0.0.1:1",
            "--skip-agent-profile",
        ],
    );
    assert_eq!(hermes_init["account_id"], imported["account_id"]);
    assert_eq!(hermes_init["npub"], imported["npub"]);
}

#[test]
fn hermes_init_mints_shared_identity_on_fresh_start() {
    // Fresh FINITE_HOME, no identity: the first Finite tool to run mints
    // the key at the shared location, and no key material lands in the
    // agent home (hard cut of identity.env / agent.nsec).
    let dir = tempfile::tempdir().unwrap();
    let finite_home = dir.path().join("finite-home");
    let home = dir.path().join("agent-home");
    let home_arg = home.display().to_string();

    let init = finitechat_bin_json(
        &finite_home,
        &[
            "hermes",
            "--agent-home",
            &home_arg,
            "init",
            "--server",
            "http://127.0.0.1:1",
            "--skip-agent-profile",
        ],
    );
    let identity_file = finite_home.join("identity").join("identity.json");
    assert!(identity_file.is_file());
    let identity: Value =
        serde_json::from_str(&std::fs::read_to_string(&identity_file).unwrap()).unwrap();
    assert_eq!(identity["version"], 1);
    assert_eq!(identity["kind"], "nostr-secp256k1");
    assert_eq!(identity["public_key_hex"], init["account_id"]);
    assert!(
        identity["created_by"]
            .as_str()
            .unwrap()
            .starts_with("finitechat ")
    );
    assert!(!home.join("agent.nsec").exists());
    assert!(!home.join("identity.env").exists());
    assert!(!home.join("account-secret.hex").exists());

    let status = finitechat_bin_json(&finite_home, &["auth", "status"]);
    assert_eq!(status["account_id"], init["account_id"]);
    assert_eq!(status["npub"], init["npub"]);
}

#[test]
fn hermes_install_installs_plugin_into_temp_hermes_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("agent-home");
    let home_arg = home.display().to_string();
    let hermes_home = dir.path().join("hermes-home");
    let plugins_dir = hermes_home.join("plugins");
    let plugins_arg = plugins_dir.display().to_string();

    hermes(&[
        "hermes",
        "--agent-home",
        &home_arg,
        "init",
        "--server",
        "http://127.0.0.1:1",
        "--skip-agent-profile",
    ]);
    let installed = hermes(&[
        "hermes",
        "--agent-home",
        &home_arg,
        "install",
        "--plugins-dir",
        &plugins_arg,
        "--finitechat-bin",
        "/bin/finitechat",
        "--json",
    ]);

    let plugin_dir = plugins_dir.join("finitechat");
    assert_eq!(installed["plugin_name"], "finitechat");
    assert_eq!(installed["platform_name"], "finitechat");
    assert_eq!(installed["plugin_dir"], plugin_dir.display().to_string());
    assert!(plugin_dir.join("__init__.py").exists());
    assert!(plugin_dir.join("adapter.py").exists());
    assert!(plugin_dir.join("plugin.yaml").exists());
    let plugin_yaml = std::fs::read_to_string(plugin_dir.join("plugin.yaml")).unwrap();
    assert!(plugin_yaml.lines().any(|line| line == "name: finitechat"));
    assert!(
        installed["recommended_config"]
            .as_str()
            .unwrap()
            .contains("platforms:\n    finitechat:")
    );
    let env = std::fs::read_to_string(plugin_dir.join("finitechat.env")).unwrap();
    assert!(env.contains(&format!("FINITECHAT_HOME={}", home.display())));
    assert!(env.contains("FINITECHAT_BIN=/bin/finitechat"));
}

#[test]
fn hermes_serve_reports_process_health() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("agent-home");
    let home_arg = home.display().to_string();
    let ready_file = dir.path().join("serve-ready.json");
    let ready_arg = ready_file.display().to_string();

    hermes(&[
        "hermes",
        "--agent-home",
        &home_arg,
        "init",
        "--server",
        "http://127.0.0.1:1",
        "--skip-agent-profile",
    ]);

    let mut child = Command::new(env!("CARGO_BIN_EXE_finitechat"))
        .args([
            "hermes",
            "--agent-home",
            &home_arg,
            "serve",
            "--addr",
            "127.0.0.1:0",
            "--ready-file",
            &ready_arg,
            "--json",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn finitechat hermes serve");

    let started_result = wait_for_ready_file(&ready_file);
    let health_result = match &started_result {
        Ok(started) => {
            let health_url = format!("{}/healthz", started["url"].as_str().unwrap());
            reqwest::blocking::get(health_url)
                .map_err(|error| error.to_string())
                .and_then(|response| response.json::<Value>().map_err(|error| error.to_string()))
        }
        Err(error) => Err(error.clone()),
    };
    let ready_result = match &started_result {
        Ok(started) => {
            let ready_url = format!("{}/readyz", started["url"].as_str().unwrap());
            reqwest::blocking::get(ready_url)
                .map_err(|error| error.to_string())
                .and_then(|response| response.json::<Value>().map_err(|error| error.to_string()))
        }
        Err(error) => Err(error.clone()),
    };
    let recover_result = match &started_result {
        Ok(started) => reqwest::blocking::Client::new()
            .post(format!(
                "{}/v1/hermes/recover",
                started["url"].as_str().unwrap()
            ))
            .json(&json!({}))
            .send()
            .map_err(|error| error.to_string())
            .and_then(|response| response.json::<Value>().map_err(|error| error.to_string())),
        Err(error) => Err(error.clone()),
    };
    let home_channel_result = match &started_result {
        Ok(started) => reqwest::blocking::Client::new()
            .post(format!(
                "{}/v1/hermes/home-channel-show",
                started["url"].as_str().unwrap()
            ))
            .json(&json!({}))
            .send()
            .map_err(|error| error.to_string())
            .and_then(|response| response.json::<Value>().map_err(|error| error.to_string())),
        Err(error) => Err(error.clone()),
    };
    let inbound_result = match &started_result {
        Ok(started) => reqwest::blocking::get(format!(
            "{}/v1/hermes/inbound?timeout_millis=0",
            started["url"].as_str().unwrap()
        ))
        .map_err(|error| error.to_string())
        .and_then(|response| {
            let status = response.status();
            response
                .text()
                .map(|body| (status.as_u16(), body))
                .map_err(|error| error.to_string())
        }),
        Err(error) => Err(error.clone()),
    };
    let unknown_action_result = match &started_result {
        Ok(started) => reqwest::blocking::Client::new()
            .post(format!(
                "{}/v1/hermes/not-a-real-action",
                started["url"].as_str().unwrap()
            ))
            .json(&json!({}))
            .send()
            .map_err(|error| error.to_string())
            .and_then(|response| {
                let status = response.status();
                response
                    .json::<Value>()
                    .map(|body| (status.as_u16(), body))
                    .map_err(|error| error.to_string())
            }),
        Err(error) => Err(error.clone()),
    };
    let _ = child.kill();
    child.wait().expect("wait hermes service");

    let started = started_result.expect("Hermes service wrote ready file");
    let response = health_result.expect("Hermes service reported health");
    assert_eq!(response["status"], "ok");
    assert_eq!(response["service"], "finitechat-hermes");
    assert_eq!(response["agent_home"], home.display().to_string());
    assert_eq!(response["account_id"], started["account_id"]);
    let ready = ready_result.expect("Hermes service reported readiness");
    assert_eq!(ready["status"], "ready");
    assert_eq!(ready["service"], "finitechat-hermes");
    assert_eq!(ready["store"], "ok");
    assert_eq!(ready["agent_home"], home.display().to_string());
    assert_eq!(ready["account_id"], started["account_id"]);
    let recover = recover_result.expect("Hermes service handled bridge action");
    assert_eq!(recover["recovered"], 0);
    let home_channel = home_channel_result.expect("Hermes service handled home-channel action");
    assert_eq!(home_channel["home_channel"], Value::Null);
    let (inbound_status, inbound_body) =
        inbound_result.expect("Hermes service handled inbound stream action");
    assert_eq!(inbound_status, 409);
    let inbound_error: Value = serde_json::from_str(&inbound_body).unwrap();
    assert_eq!(inbound_error["ok"], false);
    assert_eq!(inbound_error["status"], "error");
    assert_eq!(inbound_error["service"], "finitechat-hermes");
    assert_eq!(inbound_error["http_status"], 409);
    assert_eq!(inbound_error["error_kind"], "hermes");
    assert_eq!(inbound_error["retryable"], false);
    assert!(!inbound_error["error"].as_str().unwrap().is_empty());
    let (unknown_status, unknown_error) =
        unknown_action_result.expect("Hermes service returned structured action error");
    assert_eq!(unknown_status, 400);
    assert_eq!(unknown_error["ok"], false);
    assert_eq!(unknown_error["status"], "error");
    assert_eq!(unknown_error["service"], "finitechat-hermes");
    assert_eq!(unknown_error["http_status"], 400);
    assert_eq!(unknown_error["error_kind"], "usage");
    assert_eq!(unknown_error["retryable"], false);
    assert!(
        unknown_error["error"]
            .as_str()
            .unwrap()
            .contains("unknown Hermes service action")
    );
}

#[test]
fn hermes_cli_app_syncs_second_message_after_read_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let home = dir.path().join("agent-home");
    let home_arg = home.display().to_string();
    let app_dir = dir.path().join("app");

    hermes(&[
        "hermes",
        "--agent-home",
        &home_arg,
        "init",
        "--server",
        &server_url,
        "--device-id",
        "agent",
    ]);
    let invite = hermes(&[
        "hermes",
        "--agent-home",
        &home_arg,
        "invite",
        "--room-name",
        "Hermes CLI Receipt Followup",
        "--max-joins",
        "1",
        "--json",
    ]);
    let room_id = invite["room_id"].as_str().unwrap().to_owned();
    let app = FiniteChatRuntime::open(OpenOptions {
        data_dir: app_dir.to_string_lossy().into_owned(),
        server_url: server_url.clone(),
        device_id: "ios-smoke".to_owned(),
        account_secret_hex: Some(hex_lower(&APP_USER_SECRET)),
        now_unix_seconds: None,
    })
    .unwrap();

    let scanned = app
        .dispatch_and_wait(AppAction::ScanTarget {
            value: invite["url"].as_str().unwrap().to_owned(),
        })
        .unwrap();
    assert!(
        scanned
            .rooms
            .iter()
            .any(|room| room.room_id == room_id && room.is_agent_chat)
    );
    app.dispatch_and_wait(AppAction::SubmitInviteJoin {
        pending_room_id: room_id.clone(),
    })
    .unwrap();
    let admitted = hermes(&[
        "hermes",
        "--agent-home",
        &home_arg,
        "poll",
        "--request-json",
        r#"{"timeout_millis":0}"#,
    ]);
    assert_eq!(admitted["joined"].as_array().unwrap().len(), 1);
    app.dispatch_and_wait(AppAction::StartRuntime).unwrap();

    hermes_send_text(&home_arg, &room_id, "first agent message");
    let first_sync = app.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    assert!(
        first_sync
            .messages
            .iter()
            .any(|message| message.text == "first agent message")
    );
    app.dispatch_and_wait(AppAction::MarkRoomRead {
        room_id: room_id.clone(),
    })
    .unwrap();
    hermes_send_text(&home_arg, &room_id, "second agent message");
    drop(app);

    let reopened = FiniteChatRuntime::open(OpenOptions {
        data_dir: app_dir.to_string_lossy().into_owned(),
        server_url,
        device_id: "ios-smoke".to_owned(),
        account_secret_hex: Some(hex_lower(&APP_USER_SECRET)),
        now_unix_seconds: None,
    })
    .unwrap();
    let final_state = reopened.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    assert!(
        final_state
            .messages
            .iter()
            .any(|message| message.text == "second agent message"),
        "app should decrypt the second Hermes CLI message after sending a read receipt"
    );
}

fn hermes_invite_status(url: &str) -> Value {
    hermes(&["hermes", "invite-status", "--url", url, "--json"])
}

#[test]
fn hermes_invite_status_tracks_open_consumed_and_expired_invites() {
    let dir = tempfile::tempdir().unwrap();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let home = dir.path().join("agent-home");
    let home_arg = home.display().to_string();

    hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "init",
        "--server",
        &server_url,
        "--device-id",
        "agent",
    ]);

    // A fresh single-use invite is open and joinable.
    let invite = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "invite",
        "--room-name",
        "Hosted Pairing",
        "--max-joins",
        "1",
        "--ttl-ms",
        "3600000",
        "--json",
    ]);
    let url = invite["url"].as_str().unwrap().to_owned();
    let code = InviteCodeV1::parse(&url).unwrap();
    let status = hermes_invite_status(&url);
    assert_eq!(status["invite_id"], code.invite_id.as_str());
    assert_eq!(status["room_id"], code.room_id.as_str());
    assert_eq!(status["state"], "open");
    assert_eq!(status["max_joins"], 1);
    assert_eq!(status["accepted_joins"], 0);
    assert_eq!(status["consumed"], false);
    assert_eq!(status["expired"], false);
    assert_eq!(status["joinable"], true);
    assert!(status["expires_at_ms"].as_u64().unwrap() > now_ms());

    // A user pairs through the invite and the agent poll admits the join:
    // the single-use invite is now consumed and closed.
    let mut user = TestHermesUser::new(
        &dir.path().join("user.sqlite3"),
        &server_url,
        USER_SECRET,
        "user_phone",
    );
    user.submit_invite_join(&code, "Paul");
    let admitted = hermes_poll(&home_arg, json!({ "timeout_millis": 0 }));
    assert_eq!(admitted["joined"].as_array().unwrap().len(), 1);

    let status = hermes_invite_status(&url);
    assert_eq!(status["state"], "closed");
    assert_eq!(status["accepted_joins"], 1);
    assert_eq!(status["consumed"], true);
    assert_eq!(status["expired"], false);
    assert_eq!(status["joinable"], false);

    // An unconsumed invite past its TTL reads expired (never consumed), so
    // callers know a re-mint is safe.
    let expired_invite = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "invite",
        "--room-id",
        &code.room_id,
        "--max-joins",
        "1",
        "--ttl-ms",
        "1",
        "--json",
    ]);
    std::thread::sleep(Duration::from_millis(5));
    let status = hermes_invite_status(expired_invite["url"].as_str().unwrap());
    assert_eq!(status["consumed"], false);
    assert_eq!(status["expired"], true);
    assert_eq!(status["joinable"], false);

    // An invite id the server has never seen reads not_found without
    // claiming consumption or expiry.
    let mut unknown = code.clone();
    unknown.invite_id = "invite-0000000000000000".to_owned();
    let status = hermes_invite_status(&unknown.encode().unwrap());
    assert_eq!(status["state"], "not_found");
    assert_eq!(status["consumed"], false);
    assert_eq!(status["expired"], false);
    assert_eq!(status["joinable"], false);
}

#[test]
fn hermes_poll_recovers_messages_already_applied_by_runtime_sync() {
    let dir = tempfile::tempdir().unwrap();
    let server_url = spawn_live_http_server(&dir.path().join("server.sqlite3"));
    let home = dir.path().join("agent-home");
    let home_arg = home.display().to_string();

    hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "init",
        "--server",
        &server_url,
        "--device-id",
        "agent",
    ]);
    let invite = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "invite",
        "--room-name",
        "Hermes Durable Recovery",
        "--max-joins",
        "1",
        "--json",
    ]);
    let code = InviteCodeV1::parse(invite["url"].as_str().unwrap()).unwrap();
    let room_id = code.room_id.clone();
    let mut user = TestHermesUser::new(
        &dir.path().join("user.sqlite3"),
        &server_url,
        USER_SECRET,
        "user_phone",
    );
    user.submit_invite_join(&code, "Paul");

    let admitted = hermes_poll(&home_arg, json!({ "timeout_millis": 0 }));
    assert_eq!(admitted["joined"].as_array().unwrap().len(), 1);

    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 4,
    };
    user.finalize_invite(&code, &options);

    user.send_hermes_message(&room_id, "cursor seed", "recovery-cursor-seed", "Paul");
    let seed = hermes_poll(&home_arg, json!({ "timeout_millis": 5000 }));
    let seed_events = seed["events"].as_array().unwrap();
    assert_eq!(seed_events.len(), 1);
    assert_eq!(seed_events[0]["text"], "cursor seed");
    assert_eq!(hermes_ack(&home_arg, &seed_events[0])["acked"], true);

    user.send_hermes_message(
        &room_id,
        "durable followup one",
        "recovery-followup-one",
        "Paul",
    );
    user.send_hermes_message(
        &room_id,
        "durable followup two",
        "recovery-followup-two",
        "Paul",
    );

    let agent_secret_hex = shared_identity_secret_hex();
    let agent_app_runtime = FiniteChatRuntime::open(OpenOptions {
        data_dir: home_arg.clone(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: Some(agent_secret_hex.clone()),
        now_unix_seconds: None,
    })
    .unwrap();
    let agent_state = agent_app_runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .unwrap();
    assert!(
        agent_state
            .messages
            .iter()
            .any(|message| message.text == "durable followup one")
    );
    assert!(
        agent_state
            .messages
            .iter()
            .any(|message| message.text == "durable followup two")
    );
    drop(agent_app_runtime);

    let recovered = hermes_poll(&home_arg, json!({ "timeout_millis": 0, "limit": 10 }));
    let recovered_events = recovered["events"].as_array().unwrap();
    assert_eq!(
        recovered_events
            .iter()
            .map(|event| event["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["durable followup one", "durable followup two"]
    );
    let first_seq = recovered_events[0]["seq"].as_u64().unwrap();
    let second_seq = recovered_events[1]["seq"].as_u64().unwrap();
    assert!(
        first_seq < second_seq,
        "Hermes must preserve ordered-log sequence when recovering from durable store"
    );

    let redelivered = hermes_poll(&home_arg, json!({ "timeout_millis": 0, "limit": 10 }));
    let redelivered_events = redelivered["events"].as_array().unwrap();
    assert_eq!(
        redelivered_events
            .iter()
            .map(|event| event["message_id"].clone())
            .collect::<Vec<_>>(),
        recovered_events
            .iter()
            .map(|event| event["message_id"].clone())
            .collect::<Vec<_>>(),
        "unacked recovered events should redeliver from the local Hermes inbox"
    );

    for event in recovered_events {
        assert_eq!(hermes_ack(&home_arg, event)["acked"], true);
    }
    let drained = hermes_poll(&home_arg, json!({ "timeout_millis": 0, "limit": 10 }));
    assert_eq!(
        drained["events"].as_array().unwrap().len(),
        0,
        "acked durable recovery events must not replay from client_app_events"
    );
}

fn wait_for_ready_file(path: &std::path::Path) -> Result<Value, String> {
    for _ in 0..100 {
        if let Ok(raw) = std::fs::read_to_string(path)
            && let Ok(value) = serde_json::from_str::<Value>(&raw)
        {
            return Ok(value);
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    Err(format!("Hermes service did not write {}", path.display()))
}

#[test]
fn hermes_cli_inits_invites_admits_and_round_trips_messages() {
    let dir = tempfile::tempdir().unwrap();
    let mut smoke =
        SmokeReport::from_env("hermes_cli_inits_invites_admits_and_round_trips_messages");
    let server_db = dir.path().join("server.sqlite3");
    let server_url = smoke.time("server_start", || spawn_live_http_server(&server_db));
    let home = dir.path().join("agent-home");
    let home_arg = home.display().to_string();

    // init: shared Finite identity (0600 at the contract location), encrypted store.
    let init = smoke.time("agent_init", || {
        hermes(&[
            "hermes",
            "--home",
            &home_arg,
            "init",
            "--server",
            &server_url,
        ])
    });
    let agent_account = init["account_id"].as_str().unwrap().to_owned();
    assert!(init["npub"].as_str().unwrap().starts_with("npub1"));
    assert_eq!(init["profile"]["account_id"], agent_account);
    assert_eq!(init["profile"]["display_name"], "Finite Agent");
    assert_eq!(
        init["profile"]["picture"],
        "https://avatars.githubusercontent.com/u/274919006?v=4"
    );
    assert_eq!(init["profile"]["bot"], true);
    assert_eq!(init["profile"]["finite_role"], "agent");
    assert_eq!(init["profile"]["saved"], true);
    smoke.fact("agent_account_id", json!(agent_account.clone()));
    smoke.fact("agent_npub", init["npub"].clone());
    let identity_file = ensure_test_finite_home()
        .join("identity")
        .join("identity.json");
    assert!(identity_file.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&identity_file)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(!home.join("agent.nsec").exists());
    assert!(!home.join("identity.env").exists());

    // invite: creates the room, prints URL + QR.
    let invite = smoke.time("invite_create", || {
        hermes(&[
            "hermes",
            "--home",
            &home_arg,
            "invite",
            "--room-name",
            "Hermes Agent",
            "--json",
        ])
    });
    let url = invite["url"].as_str().unwrap();
    let room_id = invite["room_id"].as_str().unwrap().to_owned();
    smoke.fact("room_id", json!(room_id.clone()));
    assert!(!invite["qr"].as_str().unwrap().is_empty());
    let code = InviteCodeV1::parse(url).expect("printed URL is a valid invite code");
    assert_eq!(code.inviter_account_id, agent_account);
    assert_eq!(code.room_id, room_id);
    // The user scans the QR and submits the invite proof.
    let user_config = FiniteChatDeviceConfig {
        account_secret_key: NostrSecretKey::from_bytes(USER_SECRET).unwrap(),
        device_id: "user_phone".to_owned(),
        now_unix_seconds: now_ms() / 1000,
        credential_not_before_unix_seconds: now_ms() / 1000 - 3600,
        credential_not_after_unix_seconds: now_ms() / 1000 + 86400,
    };
    let mut user_store = SqliteClientStore::open(
        dir.path().join("user.sqlite3"),
        SqliteClientStoreOptions::from_nostr_secret(
            &user_config.account_secret_key,
            &user_config.device_id,
        )
        .unwrap(),
    )
    .unwrap();
    let mut user = FiniteChatDevice::new(user_config.clone()).unwrap();
    user_store.save_device_state(&user).unwrap();
    let mut user_delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.clone()));
    smoke.time("user_submit_join", || {
        submit_invite_join_request(
            &mut user_store,
            &mut user,
            &mut user_delivery,
            &code,
            Some("Paul".to_owned()),
            now_ms(),
        )
        .unwrap();
    });

    // poll admits the verified join (and merges the agent's own commit).
    let poll = smoke.time("agent_poll_admits_join", || {
        hermes(&[
            "hermes",
            "--home",
            &home_arg,
            "poll",
            "--request-json",
            r#"{"timeout_millis":0}"#,
        ])
    });
    assert_eq!(
        poll["joined"].as_array().unwrap(),
        &vec![Value::String(user.device_ref().account_id.clone())]
    );

    // The user activates the Welcome, verifies the agent, pins the server.
    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 4,
    };
    let report = smoke.time("user_finalize_invite", || {
        let report =
            run_runtime_sync_tick(&mut user_store, &mut user, &mut user_delivery, &options)
                .unwrap();
        finalize_invited_room(&mut user_store, &mut user, &code).unwrap();
        report
    });
    assert_eq!(report.claimed_welcomes, 1);

    // Agent sends through the bridge; the user reads it decrypted.
    let send_request = json!({
        "room_id": room_id,
        "conversation_id": null,
        "text": "hello from your agent",
        "kind": "message",
        "status": "complete",
        "reply_to_message_id": null,
    });
    let sent = smoke.time("agent_send_first_message", || {
        hermes(&[
            "hermes",
            "--home",
            &home_arg,
            "send",
            "--request-json",
            &send_request.to_string(),
        ])
    });
    let agent_message_id = sent["message_id"].as_str().unwrap().to_owned();

    let report = smoke.time("user_decrypt_first_message", || {
        run_runtime_sync_tick(&mut user_store, &mut user, &mut user_delivery, &options).unwrap()
    });
    // The user's room is pinned to its room server; in this test home and
    // room server are the same process, so use the room-server tick.
    let report = if report.applied_entries.is_empty() {
        smoke.time("user_room_server_sync_first_message", || {
            finitechat_client::run_room_server_sync_tick(
                &mut user_store,
                &mut user,
                &mut user_delivery,
                &options,
                &server_url,
            )
            .unwrap()
        })
    } else {
        report
    };
    let AppliedLogEntry::Application { plaintext, sender } = &report.applied_entries[0].entry
    else {
        panic!("expected application entry");
    };
    assert_eq!(sender.account_id, agent_account);
    let payload = decode_wrapped_hermes_payload(plaintext);
    assert_eq!(payload.text, "hello from your agent");

    // A running Hermes message is tracked locally and recovered after a
    // gateway restart as a final edit on the same visible bubble.
    let running_send = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "send",
        "--request-json",
        &json!({
            "room_id": room_id,
            "conversation_id": null,
            "text": "working on it ▉",
            "kind": "tool",
            "status": "running",
            "reply_to_message_id": null,
        })
        .to_string(),
    ]);
    let running_message_id = running_send["message_id"].as_str().unwrap().to_owned();
    let recovered = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "recover",
        "--request-json",
        "{}",
    ]);
    assert_eq!(recovered["recovered"], 1);
    let recovered_again = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "recover",
        "--request-json",
        "{}",
    ]);
    assert_eq!(recovered_again["recovered"], 0);
    let report = finitechat_client::run_room_server_sync_tick(
        &mut user_store,
        &mut user,
        &mut user_delivery,
        &options,
        &server_url,
    )
    .unwrap();
    let recovered_payload = report
        .applied_entries
        .iter()
        .find_map(|entry| match &entry.entry {
            AppliedLogEntry::Application { plaintext, .. } => {
                let payload = decode_wrapped_hermes_payload(plaintext);
                (payload.edit_of.as_deref() == Some(running_message_id.as_str())).then_some(payload)
            }
            _ => None,
        })
        .expect("recovery edit payload");
    assert_eq!(recovered_payload.status, HermesMessageStatusV1::Complete);
    assert!(recovered_payload.text.contains("gateway restarted"));

    // The user replies; the bridge polls it out with the authenticated
    // sender identity.
    let reply = HermesMessagePayloadV1 {
        payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
        conversation_id: None,
        text: "hi agent".to_owned(),
        kind: HermesSendKindV1::Message,
        status: HermesMessageStatusV1::Complete,
        edit_of: None,
        attachments: Vec::new(),
        reply_to_message_id: None,
        sender_name: Some("Paul".to_owned()),
        metadata: Default::default(),
    };
    let request = user
        .create_application_request(&code.room_id, &reply.encode().unwrap(), "user-reply-1")
        .unwrap();
    user_store.save_device_state(&user).unwrap();
    user_delivery
        .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap();

    let poll = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "poll",
        "--request-json",
        r#"{"timeout_millis":5000}"#,
    ]);
    let events = poll["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["text"], "hi agent");
    assert_eq!(
        events[0]["source"]["user_id"].as_str().unwrap(),
        user.device_ref().account_id
    );
    assert_eq!(events[0]["source"]["user_name"], "Paul");
    let first_inbound = events[0].clone();

    let redelivered = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "poll",
        "--request-json",
        r#"{"timeout_millis":0}"#,
    ]);
    let redelivered_events = redelivered["events"].as_array().unwrap();
    assert_eq!(redelivered_events.len(), 1);
    assert_eq!(
        redelivered_events[0]["message_id"],
        first_inbound["message_id"]
    );
    assert_eq!(redelivered_events[0]["text"], "hi agent");

    let ack = hermes_ack(&home_arg, &first_inbound);
    assert_eq!(ack["acked"], true);
    let after_ack = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "poll",
        "--request-json",
        r#"{"timeout_millis":0}"#,
    ]);
    assert_eq!(after_ack["events"].as_array().unwrap().len(), 0);

    // Regression: a separate runtime sync can land an inbound user message
    // in the agent's durable local store before the Hermes sidecar polls it.
    // The sidecar must recover it from client_app_events instead of depending
    // on a single live bridge edge.
    let stored_reply = HermesMessagePayloadV1 {
        payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
        conversation_id: None,
        text: "already synced followup".to_owned(),
        kind: HermesSendKindV1::Message,
        status: HermesMessageStatusV1::Complete,
        edit_of: None,
        attachments: Vec::new(),
        reply_to_message_id: Some(first_inbound["message_id"].as_str().unwrap().to_owned()),
        sender_name: Some("Paul".to_owned()),
        metadata: Default::default(),
    };
    let request = user
        .create_application_request(
            &code.room_id,
            &stored_reply.encode().unwrap(),
            "user-reply-already-synced",
        )
        .unwrap();
    user_store.save_device_state(&user).unwrap();
    user_delivery
        .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap();
    let agent_secret_hex = shared_identity_secret_hex();
    let agent_app_runtime = FiniteChatRuntime::open(OpenOptions {
        data_dir: home_arg.clone(),
        server_url: server_url.clone(),
        device_id: "agent".to_owned(),
        account_secret_hex: Some(agent_secret_hex.clone()),
        now_unix_seconds: None,
    })
    .unwrap();
    let agent_app_state = agent_app_runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .unwrap();
    assert!(
        agent_app_state
            .messages
            .iter()
            .any(|message| message.text == "already synced followup")
    );
    drop(agent_app_runtime);

    let recovered = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "poll",
        "--request-json",
        r#"{"timeout_millis":0}"#,
    ]);
    let recovered_events = recovered["events"].as_array().unwrap();
    assert_eq!(recovered_events.len(), 1);
    assert_eq!(recovered_events[0]["text"], "already synced followup");
    assert_eq!(
        recovered_events[0]["source"]["user_id"].as_str().unwrap(),
        user.device_ref().account_id
    );
    let ack = hermes_ack(&home_arg, &recovered_events[0]);
    assert_eq!(ack["acked"], true);

    // The iOS app sends ordinary UTF-8 chat text, not a Hermes JSON
    // envelope. The bridge still surfaces it as an authenticated inbound
    // event for the agent.
    let request = user
        .create_application_request(&code.room_id, b"plain hello from iOS", "user-reply-plain")
        .unwrap();
    user_store.save_device_state(&user).unwrap();
    user_delivery
        .append_event(&request, DurableAppEventKind::ChatMessage.delivery_policy())
        .unwrap();

    let stream_ready_file = dir.path().join("inbound-stream-ready.json");
    let stream_ready_arg = stream_ready_file.display().to_string();
    let mut stream_child = Command::new(env!("CARGO_BIN_EXE_finitechat"))
        .args([
            "hermes",
            "--agent-home",
            &home_arg,
            "serve",
            "--addr",
            "127.0.0.1:0",
            "--ready-file",
            &stream_ready_arg,
            "--json",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn finitechat hermes serve for inbound stream");
    let stream_started = smoke.time("sidecar_ready", || {
        wait_for_ready_file(&stream_ready_file).expect("stream ready file")
    });
    smoke.fact("sidecar_url", stream_started["url"].clone());
    let stream_body = smoke.time("sidecar_inbound_plain_ios_message", || {
        let stream_response = reqwest::blocking::Client::new()
            .get(format!(
                "{}/v1/hermes/inbound",
                stream_started["url"].as_str().unwrap()
            ))
            .query(&[
                ("room_id", code.room_id.as_str()),
                ("timeout_millis", "5000"),
                ("limit", "10"),
            ])
            .send()
            .expect("inbound stream response");
        assert_eq!(stream_response.status().as_u16(), 200);
        stream_response.text().expect("inbound stream body")
    });
    let _ = stream_child.kill();
    stream_child.wait().expect("wait inbound stream service");
    let events = stream_body
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter_map(|record| {
            (record.get("type").and_then(Value::as_str) == Some("event"))
                .then(|| record.get("event").cloned())
                .flatten()
        })
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["text"], "plain hello from iOS");
    assert_eq!(
        events[0]["source"]["user_id"].as_str().unwrap(),
        user.device_ref().account_id
    );
    assert_eq!(events[0]["source"]["user_id_alt"], "user_phone");
    assert_eq!(events[0]["source"]["user_name"], Value::Null);
    smoke.time("ack_stream_event", || {
        let ack = hermes_ack(&home_arg, &events[0]);
        assert_eq!(ack["acked"], true);
    });

    let drained = smoke.time("post_stream_ack_drain", || {
        hermes(&[
            "hermes",
            "--home",
            &home_arg,
            "poll",
            "--request-json",
            r#"{"timeout_millis":0}"#,
        ])
    });
    assert_eq!(drained["events"].as_array().unwrap().len(), 0);

    let report = smoke.time("agent_reply_after_stream", || {
        hermes_send_text(&home_arg, &room_id, "reply after inbound poll");
        finitechat_client::run_room_server_sync_tick(
            &mut user_store,
            &mut user,
            &mut user_delivery,
            &options,
            &server_url,
        )
        .unwrap()
    });
    let reply_after_poll = report
        .applied_entries
        .iter()
        .find_map(|entry| match &entry.entry {
            AppliedLogEntry::Application { plaintext, .. } => {
                Some(decode_wrapped_hermes_payload(plaintext))
            }
            _ => None,
        })
        .expect("agent reply after inbound poll");
    assert_eq!(reply_after_poll.text, "reply after inbound poll");

    // Streaming edit finalization lands as a new payload superseding the
    // original message id.
    let edit_request = json!({
        "room_id": code.room_id,
        "conversation_id": null,
        "message_id": agent_message_id,
        "text": "hello from your agent (edited)",
        "status": "complete",
        "finalize": true,
    });
    hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "edit",
        "--request-json",
        &edit_request.to_string(),
    ]);
    let report = finitechat_client::run_room_server_sync_tick(
        &mut user_store,
        &mut user,
        &mut user_delivery,
        &options,
        &server_url,
    )
    .unwrap();
    let edited = report
        .applied_entries
        .iter()
        .find_map(|entry| match &entry.entry {
            AppliedLogEntry::Application { plaintext, .. } => {
                Some(decode_wrapped_hermes_payload(plaintext))
            }
            _ => None,
        })
        .expect("edit payload");
    assert_eq!(edited.edit_of.as_deref(), Some(agent_message_id.as_str()));
    assert_eq!(edited.text, "hello from your agent (edited)");

    // Typing indicator goes out encrypted under the room's exporter key.
    let activity_request = json!({
        "room_id": code.room_id,
        "conversation_id": null,
        "activity_kind": "working",
        "activity_id": null,
        "action": "set",
        "payload": {},
        "expires_in_millis": 30000,
    });
    let activity = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "activity",
        "--request-json",
        &activity_request.to_string(),
    ]);
    assert_eq!(activity["accepted"], true);

    let records = user_delivery
        .get_ephemeral_activities(&GetEphemeralActivitiesRequest {
            room_id: code.room_id.clone(),
            conversation_id: None,
            requester: user.device_ref().clone(),
            now_ms: now_ms(),
        })
        .unwrap()
        .records;
    assert_eq!(records.len(), 1);
    let plaintext = user
        .decrypt_activity_payload(&code.room_id, &records[0].payload)
        .expect("joined user decrypts Hermes activity");
    let projected: DecryptedEphemeralActivityV1 = serde_json::from_slice(&plaintext).unwrap();
    assert_eq!(projected.activity_kind, "working");
    smoke.finish();
}

#[test]
fn hermes_cli_group_room_preserves_two_human_sender_identities() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("server.sqlite3");
    let server_url = spawn_live_http_server(&server_db);
    let home = dir.path().join("agent-home");
    let home_arg = home.display().to_string();

    let init = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "init",
        "--server",
        &server_url,
    ]);
    let agent_account = init["account_id"].as_str().unwrap().to_owned();
    let invite = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "invite",
        "--room-name",
        "Friends Alpha Group",
        "--max-joins",
        "2",
        "--json",
    ]);
    let code = InviteCodeV1::parse(invite["url"].as_str().unwrap()).unwrap();
    let room_id = code.room_id.clone();
    let home_channel_before = hermes(&["hermes", "--home", &home_arg, "home-channel", "show"]);
    assert_eq!(home_channel_before["home_channel"], Value::Null);
    let home_channel_set = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "home-channel",
        "set",
        "--room-id",
        &room_id,
    ]);
    assert_eq!(home_channel_set["home_channel"]["room_id"], room_id);
    let home_channel_show = hermes(&["hermes", "--home", &home_arg, "home-channel", "show"]);
    assert_eq!(home_channel_show["home_channel"]["room_id"], room_id);

    let alice_config = FiniteChatDeviceConfig {
        account_secret_key: NostrSecretKey::from_bytes(USER_SECRET).unwrap(),
        device_id: "alice_phone".to_owned(),
        now_unix_seconds: now_ms() / 1000,
        credential_not_before_unix_seconds: now_ms() / 1000 - 3600,
        credential_not_after_unix_seconds: now_ms() / 1000 + 86400,
    };
    let bob_config = FiniteChatDeviceConfig {
        account_secret_key: NostrSecretKey::from_bytes(USER2_SECRET).unwrap(),
        device_id: "bob_phone".to_owned(),
        now_unix_seconds: now_ms() / 1000,
        credential_not_before_unix_seconds: now_ms() / 1000 - 3600,
        credential_not_after_unix_seconds: now_ms() / 1000 + 86400,
    };
    let mut alice_store = SqliteClientStore::open(
        dir.path().join("alice.sqlite3"),
        SqliteClientStoreOptions::from_nostr_secret(
            &alice_config.account_secret_key,
            &alice_config.device_id,
        )
        .unwrap(),
    )
    .unwrap();
    let mut bob_store = SqliteClientStore::open(
        dir.path().join("bob.sqlite3"),
        SqliteClientStoreOptions::from_nostr_secret(
            &bob_config.account_secret_key,
            &bob_config.device_id,
        )
        .unwrap(),
    )
    .unwrap();
    let mut alice = FiniteChatDevice::new(alice_config).unwrap();
    let mut bob = FiniteChatDevice::new(bob_config).unwrap();
    alice_store.save_device_state(&alice).unwrap();
    bob_store.save_device_state(&bob).unwrap();
    let mut alice_delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.clone()));
    let mut bob_delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.clone()));

    submit_invite_join_request(
        &mut alice_store,
        &mut alice,
        &mut alice_delivery,
        &code,
        Some("Alice".to_owned()),
        now_ms(),
    )
    .unwrap();
    submit_invite_join_request(
        &mut bob_store,
        &mut bob,
        &mut bob_delivery,
        &code,
        Some("Bob".to_owned()),
        now_ms(),
    )
    .unwrap();

    let poll = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "poll",
        "--request-json",
        r#"{"timeout_millis":5000}"#,
    ]);
    let joined = poll["joined"].as_array().unwrap();
    assert_eq!(joined.len(), 2);
    assert!(joined.contains(&Value::String(alice.device_ref().account_id.clone())));
    assert!(joined.contains(&Value::String(bob.device_ref().account_id.clone())));

    let options = RuntimeSyncOptions {
        key_package_target_available: 0,
        max_sync_pages_per_room: 4,
    };
    run_runtime_sync_tick(&mut alice_store, &mut alice, &mut alice_delivery, &options).unwrap();
    finalize_invited_room(&mut alice_store, &mut alice, &code).unwrap();
    run_runtime_sync_tick(&mut bob_store, &mut bob, &mut bob_delivery, &options).unwrap();
    finalize_invited_room(&mut bob_store, &mut bob, &code).unwrap();

    let alice_payload = HermesMessagePayloadV1 {
        payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
        conversation_id: None,
        text: "alice checking in".to_owned(),
        kind: HermesSendKindV1::Message,
        status: HermesMessageStatusV1::Complete,
        edit_of: None,
        attachments: Vec::new(),
        reply_to_message_id: None,
        sender_name: Some("Alice".to_owned()),
        metadata: Default::default(),
    };
    let alice_request = alice
        .create_application_request(&room_id, &alice_payload.encode().unwrap(), "alice-group-1")
        .unwrap();
    alice_store.save_device_state(&alice).unwrap();
    alice_delivery
        .append_event(
            &alice_request,
            DurableAppEventKind::ChatMessage.delivery_policy(),
        )
        .unwrap();

    let bob_payload = HermesMessagePayloadV1 {
        payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
        conversation_id: None,
        text: "bob checking in".to_owned(),
        kind: HermesSendKindV1::Message,
        status: HermesMessageStatusV1::Complete,
        edit_of: None,
        attachments: Vec::new(),
        reply_to_message_id: None,
        sender_name: Some("Bob".to_owned()),
        metadata: Default::default(),
    };
    let bob_request = bob
        .create_application_request(&room_id, &bob_payload.encode().unwrap(), "bob-group-1")
        .unwrap();
    bob_store.save_device_state(&bob).unwrap();
    bob_delivery
        .append_event(
            &bob_request,
            DurableAppEventKind::ChatMessage.delivery_policy(),
        )
        .unwrap();

    let poll = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "poll",
        "--request-json",
        r#"{"timeout_millis":5000}"#,
    ]);
    let events = poll["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    let alice_event = events
        .iter()
        .find(|event| event["text"] == "alice checking in")
        .expect("Alice event");
    assert_eq!(alice_event["source"]["chat_type"], "group");
    assert_eq!(
        alice_event["source"]["user_id"].as_str().unwrap(),
        alice.device_ref().account_id
    );
    assert_eq!(alice_event["source"]["user_id_alt"], "alice_phone");
    assert_eq!(alice_event["source"]["user_name"], "Alice");
    let bob_event = events
        .iter()
        .find(|event| event["text"] == "bob checking in")
        .expect("Bob event");
    assert_eq!(bob_event["source"]["chat_type"], "group");
    assert_eq!(
        bob_event["source"]["user_id"].as_str().unwrap(),
        bob.device_ref().account_id
    );
    assert_eq!(bob_event["source"]["user_id_alt"], "bob_phone");
    assert_eq!(bob_event["source"]["user_name"], "Bob");

    hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "send",
        "--request-json",
        &json!({
            "room_id": room_id,
            "conversation_id": null,
            "text": "hello group from the agent",
            "kind": "message",
            "status": "complete",
            "reply_to_message_id": null,
        })
        .to_string(),
    ]);
    let alice_report = finitechat_client::run_room_server_sync_tick(
        &mut alice_store,
        &mut alice,
        &mut alice_delivery,
        &options,
        &server_url,
    )
    .unwrap();
    let alice_saw_reply = alice_report
        .applied_entries
        .iter()
        .any(|entry| match &entry.entry {
            AppliedLogEntry::Application { plaintext, sender }
                if sender.account_id == agent_account =>
            {
                wrapped_hermes_payload_text_is(plaintext, "hello group from the agent")
            }
            _ => false,
        });
    assert!(
        alice_saw_reply,
        "Alice should receive the group agent reply"
    );

    let bob_report = finitechat_client::run_room_server_sync_tick(
        &mut bob_store,
        &mut bob,
        &mut bob_delivery,
        &options,
        &server_url,
    )
    .unwrap();
    let bob_saw_reply = bob_report
        .applied_entries
        .iter()
        .any(|entry| match &entry.entry {
            AppliedLogEntry::Application { plaintext, sender }
                if sender.account_id == agent_account =>
            {
                wrapped_hermes_payload_text_is(plaintext, "hello group from the agent")
            }
            _ => false,
        });
    assert!(bob_saw_reply, "Bob should receive the group agent reply");

    let home_channel_cleared = hermes(&["hermes", "--home", &home_arg, "home-channel", "clear"]);
    assert_eq!(home_channel_cleared["cleared"], true);
    let home_channel_after_clear = hermes(&["hermes", "--home", &home_arg, "home-channel", "show"]);
    assert_eq!(home_channel_after_clear["home_channel"], Value::Null);
}

#[test]
fn hermes_cli_round_trips_media_blob_references_with_app_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("server.sqlite3");
    let server_url = spawn_live_http_server(&server_db);
    let home = dir.path().join("agent-home");
    let home_arg = home.display().to_string();

    hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "init",
        "--server",
        &server_url,
    ]);
    let invite = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "invite",
        "--room-name",
        "Hermes Media",
        "--json",
    ]);
    let room_id = invite["room_id"].as_str().unwrap().to_owned();
    let invite_url = invite["url"].as_str().unwrap().to_owned();

    let user = FiniteChatRuntime::open(OpenOptions {
        data_dir: dir.path().join("ios-user").to_string_lossy().into_owned(),
        server_url: server_url.clone(),
        device_id: "ios-media".to_owned(),
        account_secret_hex: Some(hex_lower(&APP_USER_SECRET)),
        now_unix_seconds: Some(now_ms() / 1000),
    })
    .unwrap();
    user.dispatch_and_wait(AppAction::ScanTarget { value: invite_url })
        .unwrap();
    user.dispatch_and_wait(AppAction::SubmitInviteJoin {
        pending_room_id: room_id.clone(),
    })
    .unwrap();

    let poll = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "poll",
        "--request-json",
        r#"{"timeout_millis":0}"#,
    ]);
    assert_eq!(poll["joined"].as_array().unwrap().len(), 1);
    let joined = user.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    assert_eq!(
        joined
            .rooms
            .iter()
            .find(|room| room.room_id == room_id)
            .expect("joined room projects")
            .state,
        AppRoomState::Connected
    );

    let sent = user
        .dispatch_and_wait(AppAction::SendAttachment {
            room_id: room_id.clone(),
            filename: "diagram.png".to_owned(),
            mime_type: "image/png".to_owned(),
            kind: ChatMediaKind::Image,
            bytes: b"finitechat encrypted media fixture".to_vec(),
            caption: "see attached".to_owned(),
            reply_to_message_id: None,
        })
        .unwrap();
    let user_media = sent
        .messages
        .iter()
        .find(|message| message.text == "see attached" && !message.media.is_empty())
        .expect("app attachment projects");
    let user_attachment = user_media.media.first().unwrap();
    assert_eq!(user_attachment.kind, ChatMediaKind::Image);
    assert_eq!(user_attachment.filename, "diagram.png");

    let poll = hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "poll",
        "--request-json",
        r#"{"timeout_millis":5000}"#,
    ]);
    let events = poll["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["text"], "see attached");
    assert_eq!(events[0]["message_type"], "photo");
    let hermes_attachment = events[0]["attachments"][0].clone();
    assert_eq!(hermes_attachment["kind"], "image");
    assert_eq!(hermes_attachment["name"], "diagram.png");
    assert_eq!(hermes_attachment["mime_type"], "image/png");
    assert_eq!(
        hermes_attachment["blob"]["plaintext_sha256"],
        user_attachment.attachment_id
    );
    assert!(
        hermes_attachment["blob"]["url"]
            .as_str()
            .unwrap()
            .contains("/blobs/")
    );
    let materialized_path = hermes_attachment["path"]
        .as_str()
        .expect("Hermes poll materializes blob attachment to a local path");
    assert!(materialized_path.starts_with(&home_arg));
    assert_eq!(
        std::fs::read(materialized_path).unwrap(),
        b"finitechat encrypted media fixture"
    );

    let mut returned_attachment = hermes_attachment;
    returned_attachment["path"] = Value::Null;

    hermes(&[
        "hermes",
        "--home",
        &home_arg,
        "send",
        "--request-json",
        &json!({
            "room_id": room_id,
            "conversation_id": null,
            "text": "agent media",
            "kind": "media",
            "status": "complete",
            "attachments": [returned_attachment],
            "reply_to_message_id": null,
        })
        .to_string(),
    ]);
    let received = user.dispatch_and_wait(AppAction::StartRuntime).unwrap();
    let agent_media = received
        .messages
        .iter()
        .find(|message| message.text == "agent media" && !message.media.is_empty())
        .expect("Hermes media projects into the app transcript");
    let agent_attachment = agent_media.media.first().unwrap();
    assert_eq!(agent_attachment.kind, ChatMediaKind::Image);
    assert_eq!(agent_attachment.filename, "diagram.png");
    assert_eq!(
        agent_attachment.attachment_id,
        user_attachment.attachment_id
    );
}

#[test]
fn hermes_cli_join_command_pairs_two_agent_homes_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let server_db = dir.path().join("server.sqlite3");
    let server_url = spawn_live_http_server(&server_db);
    let agent_home = dir.path().join("agent").display().to_string();
    let user_home = dir.path().join("user").display().to_string();
    // The joining agent is a second Finite account: it runs as the real
    // binary with its own FINITE_HOME (one shared identity per environment).
    let user_finite_home = dir.path().join("user-finite-home");

    hermes(&[
        "hermes",
        "--home",
        &agent_home,
        "init",
        "--server",
        &server_url,
    ]);
    let user_init = finitechat_bin_json(
        &user_finite_home,
        &[
            "hermes",
            "--home",
            &user_home,
            "init",
            "--server",
            &server_url,
        ],
    );
    let invite = hermes(&["hermes", "--home", &agent_home, "invite", "--json"]);
    let url = invite["url"].as_str().unwrap().to_owned();

    // The joiner's request sits pending until the agent polls; run the
    // join as a subprocess while the agent admits it.
    let join_child = Command::new(env!("CARGO_BIN_EXE_finitechat"))
        .env("FINITE_HOME", &user_finite_home)
        .args([
            "hermes",
            "--home",
            &user_home,
            "join",
            "--url",
            &url,
            "--timeout-ms",
            "30000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(300));
    let poll = hermes(&[
        "hermes",
        "--home",
        &agent_home,
        "poll",
        "--request-json",
        r#"{"timeout_millis":15000}"#,
    ]);
    assert_eq!(poll["joined"].as_array().unwrap().len(), 1);

    let join_output = join_child.wait_with_output().unwrap();
    assert!(
        join_output.status.success(),
        "join failed: {}",
        String::from_utf8_lossy(&join_output.stderr)
    );
    let joined: Value = serde_json::from_slice(&join_output.stdout).unwrap();
    assert_eq!(joined["state"], "joined");
    assert_eq!(joined["room_id"], invite["room_id"]);

    // Round trip purely over the CLI surface, both directions.
    let room_id = invite["room_id"].as_str().unwrap();
    finitechat_bin_json(
        &user_finite_home,
        &[
            "hermes",
            "--home",
            &user_home,
            "send",
            "--request-json",
            &json!({
                "room_id": room_id,
                "conversation_id": null,
                "text": "hello from the cli user",
                "kind": "message",
                "status": "complete",
                "reply_to_message_id": null,
            })
            .to_string(),
        ],
    );
    let agent_poll = hermes(&[
        "hermes",
        "--home",
        &agent_home,
        "poll",
        "--request-json",
        r#"{"timeout_millis":10000}"#,
    ]);
    let events = agent_poll["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["text"], "hello from the cli user");
    assert_eq!(events[0]["source"]["user_id"], user_init["account_id"]);

    hermes(&[
        "hermes",
        "--home",
        &agent_home,
        "send",
        "--request-json",
        &json!({
            "room_id": room_id,
            "conversation_id": null,
            "text": "hello back",
            "kind": "message",
            "status": "complete",
            "reply_to_message_id": null,
        })
        .to_string(),
    ]);
    let agent_image = dir.path().join("agent-reply.png");
    std::fs::write(&agent_image, b"\x89PNG\r\n\x1a\nagent media reply").unwrap();
    hermes(&[
        "hermes",
        "--home",
        &agent_home,
        "send",
        "--request-json",
        &json!({
            "room_id": room_id,
            "conversation_id": null,
            "text": "image back",
            "kind": "media",
            "status": "complete",
            "attachments": [{
                "kind": "image",
                "path": agent_image.display().to_string(),
                "name": "agent-reply.png",
                "mime_type": "image/png",
            }],
            "reply_to_message_id": null,
        })
        .to_string(),
    ]);
    let user_poll = finitechat_bin_json(
        &user_finite_home,
        &[
            "hermes",
            "--home",
            &user_home,
            "poll",
            "--request-json",
            r#"{"timeout_millis":10000}"#,
        ],
    );
    let events = user_poll["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["text"], "hello back");
    assert_eq!(events[1]["text"], "image back");
    assert_eq!(events[1]["message_type"], "photo");
    assert_eq!(events[1]["attachments"][0]["mime_type"], "image/png");
}
