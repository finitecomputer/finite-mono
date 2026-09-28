//! E-0 host harness (DESIGN.md §13.3): the real `finite-agentd serve` in a
//! scratch home, behind a fake Finite Chat bridge, with an HWD-compatible
//! `/v1/app/runtime-commands` endpoint the real dashboard can drive
//! (`FC_DESIGN_RUNTIME_COMMANDS_URL`).
//!
//! The Hermes gateway is `examples/harness-hermes-stub.sh`: the real
//! reconciler and the real pending-disconnect step, then `exec sleep`. Helper
//! facts come from the packaged helper in the patched Hermes env. No real
//! gateway, provider, or production service is ever contacted: OpenRouter's
//! `/key` and Core's hosted-Hermes desired state are local fakes on the same
//! loopback port.
//!
//! `smoke` runs the seven E-0 proofs and exits non-zero if any fails. `serve`
//! keeps the agent up for the dashboard until Ctrl-C.
//!
//! Run from the repository root, inside the Nix dev shell (the launcher step
//! needs coreutils `timeout`):
//!
//! ```text
//! scripts/with-dev-env bash -c 'cargo build -p finite-agentd --bins --examples && \
//!   target/debug/examples/inference_host_harness smoke'
//! ```

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{Parser, ValueEnum};
use finitechat_proto::{
    DeviceRef, RuntimeCommandDeliveryAckV1, RuntimeCommandDeliveryV1,
    RuntimeCommandInboundPayloadV1, RuntimeCommandJsonPayloadV1, RuntimeCommandPayloadKindV1,
    RuntimeCommandRequestV1, RuntimeCommandResultDeliveryV1, RuntimeCommandResultV1,
    RuntimeCommandTargetV1,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};

const OWNER_ACCOUNT: &str = "e0-owner-account";
const AGENT_ACCOUNT: &str = "e0-agent-account";
const AGENT_DEVICE: &str = "e0-agent-device";
const DEFAULT_ROOM: &str = "e0-room";
const FP_KEY: &str = "e0-fake-finite-private-key";
const FP_BASE_URL: &str = "https://fp.e0.invalid/v1";
const FP_MODEL: &str = "glm-5-3-flash";
const OR_MODEL: &str = "anthropic/claude-sonnet-4.6";
/// Production probes the native `hermes serve` here (`hosted_hermes_pull.rs`);
/// the harness holds the port so the probe can reach nothing else.
const NATIVE_PORT: u16 = 8642;
/// The stubs exec these, so the harness can recognize its own leftovers.
const GATEWAY_SLEEP: &str = "sleep 1000000";
const QUIET_SLEEP: &str = "sleep 1000001";
const SERVE_SLEEP: &str = "sleep 1000002";

#[derive(Parser)]
#[command(about = "E-0: the real finite-agentd behind a fake bridge")]
struct Args {
    #[arg(value_enum, default_value = "smoke")]
    mode: Mode,
    /// Loopback port for the bridge, HWD, OpenRouter and Core fakes.
    #[arg(long, default_value_t = 18990)]
    port: u16,
    /// The `finite-agentd` binary (default: next to this example's directory).
    #[arg(long)]
    agentd: Option<PathBuf>,
    /// The patched Hermes env (default: ~/.finite-scratch/fin-129-130/hermes-env-patched).
    #[arg(long)]
    hermes_env: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Mode {
    Smoke,
    Serve,
}

/// Fake bridge, HWD, OpenRouter, and Core state.
struct Fakes {
    /// Deliveries not yet acknowledged; a new stream gets all of them again.
    unacked: Mutex<Vec<RuntimeCommandDeliveryV1>>,
    results: Mutex<HashMap<String, RuntimeCommandResultV1>>,
    seq: AtomicU64,
    streams: AtomicU64,
    openrouter_requests: Mutex<Vec<String>>,
    hwd_headers: Mutex<Vec<(bool, bool)>>,
    serve_enabled: bool,
}

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Deserialize)]
struct HwdCommand {
    #[serde(default)]
    room_id: Option<String>,
    #[serde(default)]
    conversation_id: Option<String>,
    command: String,
    #[serde(default)]
    resource_key: Option<String>,
    schema: String,
    body: Value,
    #[serde(default)]
    wait_millis: Option<u64>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// ---- A minimal HTTP/1.1 server (loopback, one request per connection) ----

async fn read_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    let header_end = loop {
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        if buffer.len() > 64 * 1024 {
            return None;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split_whitespace();
    let method = request_line.next()?.to_owned();
    let target = request_line.next()?.to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<HashMap<_, _>>();
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if length > 1024 * 1024 {
        return None;
    }
    while buffer.len() < header_end + length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let path = target.split('?').next().unwrap_or_default().to_owned();
    Some(HttpRequest {
        method,
        path,
        headers,
        body: buffer[header_end..header_end + length].to_vec(),
    })
}

async fn respond(stream: &mut TcpStream, status: u16, body: &Value) {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {status} E0\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&bytes).await;
    let _ = stream.shutdown().await;
}

async fn serve_http(listener: TcpListener, fakes: Arc<Fakes>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let fakes = Arc::clone(&fakes);
        tokio::spawn(async move { handle_connection(stream, fakes).await });
    }
}

async fn handle_connection(mut stream: TcpStream, fakes: Arc<Fakes>) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/readyz") => respond(&mut stream, 200, &json!({"ready": true})).await,
        ("GET", "/v1/agentd/inbound") => stream_deliveries(stream, fakes).await,
        ("POST", "/v1/agentd/ack") => {
            if let Ok(ack) = serde_json::from_slice::<RuntimeCommandDeliveryAckV1>(&request.body) {
                fakes
                    .unacked
                    .lock()
                    .unwrap()
                    .retain(|delivery| delivery.message_id != ack.message_id);
            }
            respond(&mut stream, 200, &json!({})).await;
        }
        ("POST", "/v1/agentd/result") => {
            if let Ok(delivery) =
                serde_json::from_slice::<RuntimeCommandResultDeliveryV1>(&request.body)
            {
                fakes
                    .results
                    .lock()
                    .unwrap()
                    .insert(delivery.result.request_id.clone(), delivery.result);
            }
            respond(&mut stream, 200, &json!({})).await;
        }
        ("POST", "/v1/agentd/state") => respond(&mut stream, 200, &json!({})).await,
        ("POST", "/v1/app/runtime-commands") => {
            let (status, body) = hwd_command(&fakes, &request).await;
            respond(&mut stream, status, &body).await;
        }
        ("GET", "/api/v1/key") => {
            fakes
                .openrouter_requests
                .lock()
                .unwrap()
                .push(format!("GET {}", request.path));
            let body = json!({"data": {"label": "sk-or-v1-e0...", "limit": null,
                "limit_remaining": null, "usage": 0.0, "is_free_tier": false}});
            respond(&mut stream, 200, &body).await;
        }
        ("GET", "/api/core/v1/runtime/hosted-hermes") if fakes.serve_enabled => {
            let desired = json!({"runtimeId": "e0-runtime", "generation": 1, "enabled": true,
                "publicUrl": "https://agents.e0.invalid/runtimes/e0/", "username": "e0-user",
                "password": "e0-fake-native-password", "signingSecret": "e0-fake-native-secret",
                "accessTtlSeconds": 60});
            respond(&mut stream, 200, &desired).await;
        }
        ("POST", "/api/core/v1/runtime/hosted-hermes/report") if fakes.serve_enabled => {
            respond(&mut stream, 200, &json!({})).await;
        }
        (method, path) => {
            if path.starts_with("/api/v1/") {
                fakes
                    .openrouter_requests
                    .lock()
                    .unwrap()
                    .push(format!("{method} {path}"));
            }
            respond(&mut stream, 404, &json!({"error": "not found"})).await;
        }
    }
}

/// The bridge's inbound stream: every unacknowledged delivery, then new ones
/// as they arrive, one JSON line each. It stays open until the peer goes away.
async fn stream_deliveries(mut stream: TcpStream, fakes: Arc<Fakes>) {
    fakes.streams.fetch_add(1, Ordering::SeqCst);
    let head = "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n";
    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    let mut sent = HashSet::new();
    loop {
        let pending = fakes
            .unacked
            .lock()
            .unwrap()
            .iter()
            .filter(|delivery| !sent.contains(&delivery.message_id))
            .cloned()
            .collect::<Vec<_>>();
        for delivery in pending {
            let mut line = serde_json::to_vec(&delivery).unwrap_or_default();
            line.push(b'\n');
            if stream.write_all(&line).await.is_err() {
                return;
            }
            sent.insert(delivery.message_id);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// `/v1/app/runtime-commands` as the Hosted Web Device serves it: wrap the
/// command in a request, deliver it to agentd, and wait for its result.
async fn hwd_command(fakes: &Fakes, request: &HttpRequest) -> (u16, Value) {
    fakes.hwd_headers.lock().unwrap().push((
        request.headers.contains_key("authorization"),
        request.headers.contains_key("x-finite-workos-user-id"),
    ));
    let Ok(command) = serde_json::from_slice::<HwdCommand>(&request.body) else {
        return (400, json!({"error": "The runtime command was malformed."}));
    };
    let seq = fakes.seq.fetch_add(1, Ordering::SeqCst) + 1;
    let request_id = format!("runtime-e0-{}-{seq}", now_ms());
    let Ok(json_payload) = serde_json::to_vec(&command.body) else {
        return (
            400,
            json!({"error": "The runtime command body was malformed."}),
        );
    };
    // E-0 has one agent: the target is always it, whatever the caller named.
    let delivery = RuntimeCommandDeliveryV1 {
        room_id: command
            .room_id
            .filter(|room| !room.is_empty())
            .unwrap_or_else(|| DEFAULT_ROOM.to_owned()),
        conversation_id: command.conversation_id,
        seq,
        message_id: format!("e0-message-{seq}"),
        sender: DeviceRef::new(OWNER_ACCOUNT, "e0-hosted-web"),
        payload: RuntimeCommandInboundPayloadV1::Request(RuntimeCommandRequestV1 {
            payload_kind: RuntimeCommandPayloadKindV1::Request,
            request_id: request_id.clone(),
            command: command.command,
            target: RuntimeCommandTargetV1 {
                account_id: AGENT_ACCOUNT.to_owned(),
                device_id: None,
            },
            resource_key: command.resource_key,
            body: RuntimeCommandJsonPayloadV1 {
                schema: command.schema,
                json_payload,
            },
        }),
    };
    fakes.unacked.lock().unwrap().push(delivery);
    let wait = Duration::from_millis(command.wait_millis.unwrap_or(45_000).clamp(1_000, 60_000));
    let started = Instant::now();
    while started.elapsed() < wait {
        if let Some(result) = fakes.results.lock().unwrap().remove(&request_id) {
            let body = result
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice::<Value>(&body.json_payload).ok());
            return (
                200,
                json!({"request_id": result.request_id, "status": result.status,
                       "body": body, "error": result.error}),
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    (
        504,
        json!({"error": "The agent did not respond in time. Try again.", "error_kind": "timeout",
               "retryable": true}),
    )
}

// ---- The scratch run: layout, agentd process, observations ----

struct Run {
    dir: PathBuf,
    agent_home: PathBuf,
    hermes_home: PathBuf,
    env: BTreeMap<String, String>,
    agentd_bin: PathBuf,
    port: u16,
    fakes: Arc<Fakes>,
    client: reqwest::Client,
    agentd: Option<(Child, u32)>,
    started_pids: Vec<u32>,
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write a harness script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod a harness script");
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

impl Run {
    fn prepare(args: &Args, fakes: Arc<Fakes>) -> Self {
        let scratch =
            PathBuf::from(std::env::var("HOME").expect("HOME")).join(".finite-scratch/fin-129-130");
        let dir = scratch.join("e0").join(format!("run-{}", now_ms()));
        let hermes_env = args
            .hermes_env
            .clone()
            .unwrap_or_else(|| scratch.join("hermes-env-patched"));
        let repo = std::env::current_dir().expect("current dir");
        let stub = repo.join("finite-agentd/examples/harness-hermes-stub.sh");
        assert!(
            stub.is_file(),
            "run from the repository root (no {})",
            stub.display()
        );
        assert!(
            hermes_env.join("bin/python3").is_file(),
            "no patched Hermes env at {}",
            hermes_env.display()
        );
        let agentd_bin = args.agentd.clone().unwrap_or_else(|| {
            let exe = std::env::current_exe().expect("current exe");
            exe.parent()
                .and_then(Path::parent)
                .expect("target dir")
                .join("finite-agentd")
        });
        assert!(
            agentd_bin.is_file(),
            "no finite-agentd at {} (cargo build -p finite-agentd --bins)",
            agentd_bin.display()
        );
        let home = dir.join("home");
        let agent_home = dir.join("agent");
        let hermes_home = agent_home.join("hermes-home");
        let bin = dir.join("bin");
        let codex_home = dir.join("codex-empty");
        for path in [&home, &hermes_home, &bin, &codex_home] {
            fs::create_dir_all(path).expect("scratch layout");
        }
        fs::write(
            agent_home.join("config.json"),
            json!({"account_id": AGENT_ACCOUNT, "device_id": AGENT_DEVICE}).to_string(),
        )
        .expect("agent identity");
        write_executable(
            &bin.join("gateway"),
            &format!("#!/bin/sh\nexec bash '{}' \"$@\"\n", stub.display()),
        );
        // The chat sidecar and health server are not under test: they sleep.
        // `admission seed` must return, because agentd runs it synchronously.
        write_executable(
            &bin.join("finitechat"),
            &format!("#!/bin/sh\ncase \"$*\" in *admission*) exit 0 ;; esac\nexec {QUIET_SLEEP}\n"),
        );
        write_executable(
            &bin.join("health"),
            &format!("#!/bin/sh\nexec {QUIET_SLEEP}\n"),
        );
        // `hermes config check` is real; `hermes serve` is a stand-in that
        // records its start and sleeps.
        write_executable(
            &bin.join("hermes"),
            &format!(
                "#!/bin/sh\ncase \"${{1:-}}\" in\n  config) exec '{}' \"$@\" ;;\n  serve) echo \"$(python -c 'import time; print(int(time.time()*1000))') serve-start pid=$$\" >> '{}'; exec {SERVE_SLEEP} ;;\n  *) echo \"e0: hermes $* is not part of E-0\" >&2; exit 64 ;;\nesac\n",
                hermes_env.join("bin/hermes").display(),
                dir.join("events.log").display()
            ),
        );
        let path = format!(
            "{}:{}:{}",
            bin.display(),
            hermes_env.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut env = BTreeMap::new();
        let mut set = |name: &str, value: String| {
            env.insert(name.to_owned(), value);
        };
        set("PATH", path);
        set("HOME", home.display().to_string());
        set(
            "XDG_STATE_HOME",
            home.join(".local/state").display().to_string(),
        );
        set(
            "XDG_CONFIG_HOME",
            home.join(".config").display().to_string(),
        );
        set(
            "XDG_DATA_HOME",
            home.join(".local/share").display().to_string(),
        );
        set("XDG_CACHE_HOME", home.join(".cache").display().to_string());
        set(
            "HERMES_KANBAN_HOME",
            home.join("kanban").display().to_string(),
        );
        set("CODEX_HOME", codex_home.display().to_string());
        set("FINITECHAT_HOME", agent_home.display().to_string());
        set("HERMES_HOME", hermes_home.display().to_string());
        set(
            "FINITE_AGENTD_BRIDGE_ADDR",
            format!("127.0.0.1:{}", args.port),
        );
        set("FINITE_AGENTD_BRIDGE_READY_TIMEOUT_SECS", "30".to_owned());
        set(
            "FINITECHAT_BIN",
            bin.join("finitechat").display().to_string(),
        );
        set(
            "FINITE_AGENTD_PREPARE_COMMAND",
            bin.join("gateway").display().to_string(),
        );
        set(
            "FINITE_AGENTD_HERMES_COMMAND",
            bin.join("gateway").display().to_string(),
        );
        set(
            "FINITE_AGENTD_HEALTH_PYTHON",
            bin.join("health").display().to_string(),
        );
        set("FINITE_AGENTD_HEALTH_SCRIPT", "/dev/null".to_owned());
        set(
            "FINITE_AGENTD_SIMPLEX_SCRIPT",
            dir.join("no-simplex").display().to_string(),
        );
        set(
            "FINITE_AGENTD_AUTHORIZED_ACCOUNT_IDS",
            OWNER_ACCOUNT.to_owned(),
        );
        set(
            "FINITE_DEFAULT_INFERENCE_PROFILE",
            "finite-private".to_owned(),
        );
        set("FINITE_PRIVATE_API_KEY", FP_KEY.to_owned());
        // The Runner's alias; the launch rule must drop it.
        set("OPENAI_API_KEY", FP_KEY.to_owned());
        set("FINITE_PRIVATE_MODEL", FP_MODEL.to_owned());
        set("FINITE_PRIVATE_BASE_URL", FP_BASE_URL.to_owned());
        set("FINITE_PRIVATE_CONTEXT_LENGTH", "393216".to_owned());
        set(
            "FINITE_PRIVATE_CONTROL_URL",
            format!("http://127.0.0.1:{}/fp-control", args.port),
        );
        set(
            "FINITE_AGENTD_OPENROUTER_API_BASE",
            format!("http://127.0.0.1:{}/api/v1", args.port),
        );
        set(
            "FINITE_AGENTD_INFERENCE_HELPER_PYTHON",
            hermes_env.join("bin/python3").display().to_string(),
        );
        if fakes.serve_enabled {
            set(
                "FINITE_CORE_URL",
                format!("http://127.0.0.1:{}/", args.port),
            );
            set("FINITE_CORE_CREDENTIAL", "e0".repeat(32));
        }
        set("E0_REPO", repo.display().to_string());
        set("E0_RUN_DIR", dir.display().to_string());
        Self {
            dir,
            agent_home,
            hermes_home,
            env,
            agentd_bin,
            port: args.port,
            fakes,
            client: reqwest::Client::new(),
            agentd: None,
            started_pids: Vec::new(),
        }
    }

    fn intent_path(&self) -> PathBuf {
        self.agent_home.join("agentd/inference-intent.json")
    }

    fn intent(&self) -> Option<Value> {
        fs::read(self.intent_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    fn set_stub_mode(&self, mode: &str) {
        fs::write(self.dir.join("stub-mode"), mode).expect("stub mode");
    }

    /// Gateway and serve start lines from the stubs.
    fn events(&self) -> Vec<(u64, String)> {
        fs::read_to_string(self.dir.join("events.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let (ms, rest) = line.split_once(' ')?;
                Some((ms.parse().ok()?, rest.to_owned()))
            })
            .collect()
    }

    /// Stub lines of one kind (`gateway-spawn`, `gateway-ready`,
    /// `gateway-stopped`) at or after `since`.
    fn stub_lines_since(&self, since: u64, kind: &str) -> Vec<(u64, String)> {
        self.events()
            .into_iter()
            .filter(|(ms, line)| *ms >= since && line.starts_with(kind))
            .collect()
    }

    /// Gateway starts attributed to an operation by the intent they saw when
    /// spawned (for example `intent=select/`).
    fn spawns_seeing(&self, since: u64, intent: &str) -> Vec<(u64, String)> {
        self.stub_lines_since(since, "gateway-spawn")
            .into_iter()
            .filter(|(_, line)| line.contains(intent))
            .collect()
    }

    /// How the start with this spawn line ended: its ready or stopped line.
    fn outcome_of(&self, spawn: &str) -> String {
        let pid = spawn
            .split_whitespace()
            .find(|part| part.starts_with("pid="))
            .unwrap_or_default()
            .to_owned();
        self.events()
            .into_iter()
            .find(|(_, line)| {
                (line.starts_with("gateway-ready") || line.starts_with("gateway-stopped"))
                    && line.split_whitespace().any(|part| part == pid)
            })
            .map(|(_, line)| line)
            .unwrap_or_else(|| "no ready or stopped line".to_owned())
    }

    /// Every stub line between two times, for the evidence of a race.
    fn stub_story(&self, since: u64, until: u64) -> Vec<String> {
        self.events()
            .into_iter()
            .filter(|(ms, line)| *ms >= since && *ms <= until && line.starts_with("gateway-"))
            .map(|(ms, line)| format!("+{}ms {line}", ms - since))
            .collect()
    }

    /// The stand-in for a first chat turn (G1): what the real gateway writes
    /// into config.yaml, with the model values unchanged.
    fn first_turn_rewrite(&self) {
        let path = self.hermes_home.join("config.yaml");
        let mut config: serde_yaml::Value =
            serde_yaml::from_str(&fs::read_to_string(&path).expect("config")).expect("config yaml");
        let mapping = config.as_mapping_mut().expect("config mapping");
        mapping.insert(
            "onboarding".into(),
            serde_yaml::from_str("seen: {profile_build_offered: true}").expect("yaml"),
        );
        mapping.insert("agent".into(), serde_yaml::Mapping::new().into());
        let text = serde_yaml::to_string(&config).expect("yaml");
        fs::write(
            &path,
            format!(
                "{text}# fallback_model:\n#   provider: openrouter\n#   model: example/model\n"
            ),
        )
        .expect("rewrite config");
    }

    fn serve_starts(&self) -> Vec<(u64, u32)> {
        self.events()
            .into_iter()
            .filter_map(|(ms, line)| {
                let pid = line.strip_prefix("serve-start pid=")?.trim().parse().ok()?;
                Some((ms, pid))
            })
            .collect()
    }

    async fn start_agentd(&mut self) {
        let before = self.fakes.streams.load(Ordering::SeqCst);
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("agentd.log"))
            .expect("agentd log");
        let child = Command::new(&self.agentd_bin)
            .arg("serve")
            .env_clear()
            .envs(&self.env)
            .current_dir(&self.dir)
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone().expect("log"))
            .stderr(log)
            .spawn()
            .expect("spawn finite-agentd");
        let pid = child.id().expect("agentd pid");
        self.started_pids.push(pid);
        self.agentd = Some((child, pid));
        // Connected: agentd opened the inbound stream after bridge readiness.
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.fakes.streams.load(Ordering::SeqCst) == before {
            assert!(
                Instant::now() < deadline,
                "agentd never connected to the bridge"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Graceful stop: SIGTERM, then agentd drains its own children.
    async fn stop_agentd(&mut self) {
        let Some((mut child, pid)) = self.agentd.take() else {
            return;
        };
        signal(pid, rustix::process::Signal::TERM);
        if tokio::time::timeout(Duration::from_secs(40), child.wait())
            .await
            .is_err()
        {
            signal(pid, rustix::process::Signal::KILL);
            let _ = child.wait().await;
        }
        self.sweep();
    }

    /// A crash: SIGKILL agentd alone, then remove what it left behind, the
    /// way the container entrypoint's orphan sweep does.
    async fn kill_agentd(&mut self) {
        let Some((mut child, pid)) = self.agentd.take() else {
            return;
        };
        signal(pid, rustix::process::Signal::KILL);
        let _ = child.wait().await;
        self.sweep();
    }

    /// Kills the harness's own sleepers left behind: only processes whose
    /// command line is one of the stubs' unique sleeps.
    fn sweep(&self) {
        let output = std::process::Command::new("ps")
            .args(["-axo", "pid=,command="])
            .output()
            .expect("ps");
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let line = line.trim();
            let Some((pid, command)) = line.split_once(' ') else {
                continue;
            };
            let command = command.trim();
            if [GATEWAY_SLEEP, QUIET_SLEEP, SERVE_SLEEP].contains(&command)
                && let Ok(pid) = pid.parse::<u32>()
                && self.owns(pid)
            {
                signal(pid, rustix::process::Signal::KILL);
            }
        }
    }

    /// A sleeper is ours when its pid is one the stubs recorded, or its
    /// environment names this run (`ps eww` shows it for our own user).
    fn owns(&self, pid: u32) -> bool {
        let recorded = self.events().iter().any(|(_, line)| {
            line.contains(&format!("pid={pid} ")) || line.ends_with(&format!("pid={pid}"))
        });
        if recorded {
            return true;
        }
        let output = std::process::Command::new("ps")
            .args(["eww", "-o", "command=", "-p", &pid.to_string()])
            .output();
        output.is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("E0_RUN_DIR={}", self.dir.display()))
        })
    }

    async fn command(&self, name: &str, schema: &str, body: Value) -> Value {
        let response = self
            .client
            .post(format!(
                "http://127.0.0.1:{}/v1/app/runtime-commands",
                self.port
            ))
            .bearer_auth("web-design-hosted-device-token")
            .header("x-finite-workos-user-id", "user_web_design")
            .json(
                &json!({"room_id": DEFAULT_ROOM, "target_account_id": AGENT_ACCOUNT,
                          "command": name, "schema": schema, "body": body, "wait_millis": 60_000}),
            )
            .timeout(Duration::from_secs(70))
            .send()
            .await
            .expect("HWD request");
        response.json().await.expect("HWD reply")
    }

    async fn status(&self) -> Value {
        let reply = self
            .command(
                "agent.connections.status",
                "finite.agent.empty.request.v1",
                json!({}),
            )
            .await;
        assert_eq!(reply["status"], "succeeded", "status failed: {reply}");
        reply["body"].clone()
    }

    async fn v1_openrouter(&self, key: &str, model: &str) -> Value {
        self.command(
            "agent.inference.apply",
            "finite.agent.inference.apply.v1",
            json!({"profile": "openrouter", "api_key": key, "model": model}),
        )
        .await
    }

    async fn select(&self, route: &str, model: Option<&str>) -> Value {
        self.command(
            "agent.inference.select",
            "finite.agent.inference.select.v1",
            json!({"route": route, "model": model}),
        )
        .await
    }

    async fn disconnect(&self, route: &str) -> Value {
        self.command(
            "agent.inference.disconnect",
            "finite.agent.inference.disconnect.v1",
            json!({"route": route}),
        )
        .await
    }

    /// Polls status until the operation `id` is no longer running.
    async fn operation_end(&self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(240);
        loop {
            let status = self.status().await;
            let operation = &status["inference"]["operation"];
            if operation["id"] == id && operation["state"] != "running" {
                return status;
            }
            assert!(Instant::now() < deadline, "operation {id} never finished");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// The real helper's facts on the scratch files, with the environment
    /// agentd gives it.
    fn helper_facts(&self) -> Value {
        let output = std::process::Command::new(&self.env["FINITE_AGENTD_INFERENCE_HELPER_PYTHON"])
            .args([
                "-m",
                "hermes_cli.finite_inference_helper",
                "inference-facts",
            ])
            .env_clear()
            .envs(&self.env)
            .env("CODEX_HOME", "/dev/null/finite-codex-home-disabled")
            .env("FINITE_CONFIG_FP_MODEL", FP_MODEL)
            .env("FINITE_CONFIG_FP_BASE_URL", FP_BASE_URL)
            .env("FINITE_CONFIG_FP_CONTEXT_LENGTH", "393216")
            .env_remove("OPENAI_API_KEY")
            .output()
            .expect("run the helper");
        serde_json::from_slice(&output.stdout).expect("helper facts")
    }

    /// Hermes-side state for OpenRouter, written through Hermes's own code: a
    /// manual pool entry in `auth.json` and a conversation override.
    fn seed_hermes_openrouter_state(&self) {
        let auth_path = self.hermes_home.join("auth.json");
        let mut store = fs::read(&auth_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or_else(|| json!({"version": 1}));
        store["credential_pool"]["openrouter"] = json!([
            {"id": "e0-pool", "source": "manual", "access_token": "sk-or-v1-e0-fake-pool"}
        ]);
        fs::write(&auth_path, serde_json::to_vec_pretty(&store).expect("auth")).expect("auth.json");
        fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600)).expect("auth mode");
        let script = "from gateway import config as gc, session as gs\n\
            cfg = gc.load_gateway_config()\n\
            store = gs.SessionStore(cfg.sessions_dir, cfg)\n\
            source = gs.SessionSource(platform=gc.Platform.LOCAL, chat_id='e0-room', user_id='e0-user')\n\
            entry = store.get_or_create_session(source)\n\
            store.set_model_override(entry.session_key, {'provider': 'openrouter', 'model': 'e0/model', 'base_url': 'https://openrouter.ai/api/v1'})\n";
        let status = std::process::Command::new(&self.env["FINITE_AGENTD_INFERENCE_HELPER_PYTHON"])
            .args(["-c", script])
            .env_clear()
            .envs(&self.env)
            .current_dir(&self.dir)
            .status()
            .expect("seed a session override");
        assert!(status.success(), "seeding the override failed");
    }

    /// Writes a disconnect intent as a crash right after `accepted` leaves it.
    fn write_crashed_disconnect(&self) -> String {
        let id = format!("op_{}", &sha256_hex(now_ms().to_string().as_bytes())[..32]);
        let now = now_ms();
        let record = json!({"v": 1, "id": id, "kind": "disconnect", "route": "openrouter",
            "model": null, "phase": "accepted", "state": "running", "error_code": null,
            "attempts": 0, "created_at_ms": now, "updated_at_ms": now});
        let path = self.intent_path();
        fs::write(&path, serde_json::to_vec_pretty(&record).expect("intent")).expect("intent");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("intent mode");
        id
    }

    /// One gateway start by the stub with agentd stopped: waits for its ready
    /// (or stopped) line, then stops it. Returns that line.
    async fn launcher_start_alone(&mut self) -> String {
        let mut child = Command::new(self.dir.join("bin/gateway"))
            .env_clear()
            .envs(&self.env)
            .env("FINITE_AGENTD_SUPERVISED", "1")
            .env("FINITE_AGENTD_INTENT_PATH", self.intent_path())
            .current_dir(&self.dir)
            .stdin(std::process::Stdio::null())
            .spawn()
            .expect("spawn the stub");
        let pid = child.id().expect("stub pid");
        self.started_pids.push(pid);
        let deadline = Instant::now() + Duration::from_secs(120);
        let line = loop {
            if let Some((_, line)) = self.events().into_iter().find(|(_, line)| {
                (line.starts_with("gateway-ready") || line.starts_with("gateway-stopped"))
                    && line
                        .split_whitespace()
                        .any(|part| part == format!("pid={pid}"))
            }) {
                break line;
            }
            assert!(Instant::now() < deadline, "the stub never became ready");
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        signal(pid, rustix::process::Signal::TERM);
        let _ = child.wait().await;
        line
    }

    fn dotenv(&self) -> String {
        fs::read_to_string(self.hermes_home.join(".env")).unwrap_or_default()
    }

    fn config_text(&self) -> String {
        fs::read_to_string(self.hermes_home.join("config.yaml")).unwrap_or_default()
    }
}

fn signal(pid: u32, signal: rustix::process::Signal) {
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process(pid, signal);
    }
}

fn alive(pid: u32) -> bool {
    rustix::process::Pid::from_raw(pid as i32)
        .is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
}

// ---- Observation of the intent record and `hermes serve` over time ----

#[derive(Default)]
struct Timeline {
    /// (observed ms, "kind/phase/state" or "absent", the record's
    /// `updated_at_ms`) on each change of the file's content.
    intent: Vec<(u64, String, u64)>,
    /// (ms, serve pid, alive) on each change.
    serve: Vec<(u64, u32, bool)>,
}

fn observe(run_dir: PathBuf, intent_path: PathBuf, timeline: Arc<Mutex<Timeline>>) {
    tokio::spawn(async move {
        let mut last_intent = (String::new(), 0);
        let mut serve_alive: HashMap<u32, bool> = HashMap::new();
        loop {
            let intent = fs::read(&intent_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .map(|record| {
                    (
                        format!(
                            "{}/{}/{}",
                            record["kind"].as_str().unwrap_or("?"),
                            record["phase"].as_str().unwrap_or("?"),
                            record["state"].as_str().unwrap_or("?")
                        ),
                        record["updated_at_ms"].as_u64().unwrap_or(0),
                    )
                })
                .unwrap_or_else(|| ("absent".to_owned(), 0));
            if intent != last_intent {
                timeline
                    .lock()
                    .unwrap()
                    .intent
                    .push((now_ms(), intent.0.clone(), intent.1));
                last_intent = intent;
            }
            let starts = fs::read_to_string(run_dir.join("events.log")).unwrap_or_default();
            for line in starts.lines() {
                if let Some(pid) = line
                    .split_once(" serve-start pid=")
                    .and_then(|(_, pid)| pid.trim().parse::<u32>().ok())
                {
                    let now = alive(pid);
                    if serve_alive.insert(pid, now) != Some(now) {
                        timeline.lock().unwrap().serve.push((now_ms(), pid, now));
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    });
}

// ---- The seven proofs ----

struct Proofs {
    results: Vec<(String, bool, Vec<String>)>,
    observations: Vec<(String, Vec<String>)>,
}

impl Proofs {
    /// Evidence that is not a pass/fail check.
    fn observe(&mut self, name: &str, lines: Vec<String>) {
        println!("\n-- observed: {name}");
        for line in &lines {
            println!("   {line}");
        }
        self.observations.push((name.to_owned(), lines));
    }

    fn record(&mut self, name: &str, checks: Vec<(bool, String)>) {
        let passed = checks.iter().all(|(ok, _)| *ok);
        let lines = checks
            .into_iter()
            .map(|(ok, line)| format!("[{}] {line}", if ok { "ok" } else { "FAIL" }))
            .collect::<Vec<_>>();
        println!("\n== {name}: {}", if passed { "PASS" } else { "FAIL" });
        for line in &lines {
            println!("   {line}");
        }
        self.results.push((name.to_owned(), passed, lines));
    }
}

fn check(ok: bool, what: impl Into<String>) -> (bool, String) {
    (ok, what.into())
}

fn is_operation_id(value: &Value) -> bool {
    value.as_str().is_some_and(|id| {
        id.len() == 35
            && id.starts_with("op_")
            && id[3..].bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

async fn smoke(run: &mut Run, timeline: Arc<Mutex<Timeline>>) -> Proofs {
    let mut proofs = Proofs {
        results: Vec::new(),
        observations: Vec::new(),
    };
    let key_one = "sk-or-v1-e0-fake-key-one";

    // P7 (first half): facts on the files the real reconciler seeded.
    run.start_agentd().await;
    let status = run.status().await;
    let facts = run.helper_facts();
    let inference = &status["inference"];
    let mut p7 = vec![
        check(
            inference["saved"]["route"] == "finite_private",
            format!("saved route {}", inference["saved"]["route"]),
        ),
        check(
            inference["routes"]["finite_private"]["state"] == "configured",
            format!("finite_private {}", inference["routes"]["finite_private"]),
        ),
        check(
            inference["fallback"]["state"] == "configured"
                && inference["fallback"]["model"] == FP_MODEL,
            format!("fallback {}", inference["fallback"]),
        ),
        check(
            facts["finite_private"]["provider_entry"] == "canonical",
            format!(
                "helper provider_entry {}",
                facts["finite_private"]["provider_entry"]
            ),
        ),
        check(
            facts["alias_present"] == "no" && facts["codex_home_neutral"] == "yes",
            format!(
                "helper alias_present {} codex_home_neutral {}",
                facts["alias_present"], facts["codex_home_neutral"]
            ),
        ),
        check(
            inference["routes"]["openrouter"]["state"] == "no_key",
            format!("openrouter {}", inference["routes"]["openrouter"]),
        ),
        check(
            status["capabilities"]
                == json!([
                    "inference.status.v2",
                    "inference.select.v1",
                    "inference.disconnect.v1"
                ]),
            format!("capabilities {}", status["capabilities"]),
        ),
    ];

    // v1 with a fake key: key facts through the real helper on the real .env.
    let reply = run.v1_openrouter(key_one, OR_MODEL).await;
    let status = run.status().await;
    let openrouter = &status["inference"]["routes"]["openrouter"];
    p7.push(check(
        reply["status"] == "succeeded",
        format!("v1 apply {}", reply["body"]),
    ));
    p7.push(check(
        openrouter["key_source"] == "agent"
            && openrouter["key_hash"] == sha256_hex(key_one.as_bytes()),
        format!(
            "openrouter key_source {} key_hash matches sha256 of the written key",
            openrouter["key_source"]
        ),
    ));
    p7.push(check(
        openrouter["hermes_key"] == "saved_key",
        format!(
            "hermes_key {} (helper fingerprint of .env)",
            openrouter["hermes_key"]
        ),
    ));

    // P1 + P4a: select and disconnect reply accepted; status follows each to
    // its end. While the select verifies (after `Running`), a stand-in first
    // chat turn rewrites config.yaml.
    let before = now_ms();
    let reply = run.select("finite_private", None).await;
    let id = reply["body"]["operation_id"].clone();
    let running = run.status().await;
    let deadline = Instant::now() + Duration::from_secs(60);
    while run
        .intent()
        .is_some_and(|record| record["phase"] != "verifying")
    {
        assert!(
            Instant::now() < deadline,
            "the select never reached verifying"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    run.first_turn_rewrite();
    let ended = run.operation_end(id.as_str().unwrap_or_default()).await;
    let starts = run.spawns_seeing(before, "intent=select/");
    let mut p1 = vec![
        check(
            reply["body"]["accepted"] == true && is_operation_id(&id),
            format!("select reply {}", reply["body"]),
        ),
        check(
            running["inference"]["operation"]["id"] == id
                && running["inference"]["operation"]["kind"] == "select",
            format!("status right after: {}", running["inference"]["operation"]),
        ),
        check(
            ended["inference"]["operation"]["state"] == "succeeded"
                && ended["inference"]["saved"]["route"] == "finite_private",
            format!(
                "select ended {} saved {}",
                ended["inference"]["operation"]["state"], ended["inference"]["saved"]["route"]
            ),
        ),
    ];
    let p4 = vec![
        check(
            starts.len() == 1,
            format!(
                "first-turn rewrite during verification: {} gateway start(s) for the select (1 = no re-apply)",
                starts.len()
            ),
        ),
        check(
            run.config_text().contains("profile_build_offered")
                && run.config_text().contains("# fallback_model:"),
            "the rewrite (onboarding flag, trailing comment block) is still on disk: agentd wrote nothing after it",
        ),
        check(
            ended["inference"]["operation"]["state"] == "succeeded",
            "verification compared values and passed",
        ),
    ];
    let disconnect_at = now_ms();
    let reply = run.disconnect("openrouter").await;
    let id = reply["body"]["operation_id"].clone();
    let ended = run.operation_end(id.as_str().unwrap_or_default()).await;
    proofs.observe(
        "P1 disconnect (not the saved route), gateway starts",
        run.stub_story(disconnect_at, now_ms()),
    );
    p1.push(check(
        reply["body"]["accepted"] == true && is_operation_id(&id),
        format!("disconnect reply {}", reply["body"]),
    ));
    p1.push(check(
        ended["inference"]["operation"]["state"] == "succeeded"
            && ended["inference"]["operation"]["kind"] == "disconnect",
        format!("disconnect ended {}", ended["inference"]["operation"]),
    ));
    p1.push(check(
        !run.dotenv().contains("OPENROUTER_API_KEY")
            && ended["inference"]["routes"]["openrouter"]["state"] == "no_key",
        "the .env key is gone and status says no_key",
    ));
    let reply = run.select("openrouter", Some(OR_MODEL)).await;
    p1.push(check(
        reply["status"] == "failed" && reply["error"]["code"] == "not_connected",
        format!("select OpenRouter with no key: {}", reply["error"]),
    ));
    proofs.record("P1 async command contract", p1);

    // P2 + P3: a killed agentd leaves the intent; the restart runs Hermes
    // first, then the executor finishes the operation.
    // The kill lands mid-way through a select to OpenRouter, whose key check
    // goes to the fake `/key`.
    run.v1_openrouter("sk-or-v1-e0-fake-key-two", OR_MODEL)
        .await;
    let reply = run.select("finite_private", None).await;
    run.operation_end(reply["body"]["operation_id"].as_str().unwrap_or_default())
        .await;
    let reply = run.select("openrouter", Some(OR_MODEL)).await;
    let id = reply["body"]["operation_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !run
        .intent()
        .is_some_and(|record| matches!(record["phase"].as_str(), Some("restarting" | "verifying")))
    {
        assert!(
            Instant::now() < deadline,
            "the select never reached restarting"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    run.kill_agentd().await;
    let left = run.intent().unwrap_or(Value::Null);
    let killed_at = now_ms();
    run.start_agentd().await;
    let ended = run.operation_end(&id).await;
    let restart_start = run
        .stub_lines_since(killed_at, "gateway-spawn")
        .first()
        .cloned();
    let restart_outcome = restart_start
        .as_ref()
        .map(|(_, line)| run.outcome_of(line))
        .unwrap_or_default();
    // The new executor's first write carries its own timestamp.
    let resumed_at = timeline
        .lock()
        .unwrap()
        .intent
        .iter()
        .find(|(_, state, updated)| state != "absent" && *updated > killed_at)
        .map(|(_, _, updated)| *updated);
    proofs.record(
        "P2 intent across a killed agentd",
        vec![
            check(
                left["id"] == id && left["state"] == "running",
                format!(
                    "after SIGKILL the record is on disk: {}/{} ({})",
                    left["kind"], left["phase"], left["state"]
                ),
            ),
            check(
                ended["inference"]["operation"]["id"] == id
                    && ended["inference"]["operation"]["state"] == "succeeded",
                format!("after restart: {}", ended["inference"]["operation"]),
            ),
            check(
                ended["inference"]["saved"]["route"] == "openrouter" && run.intent().is_none(),
                "the route converged and the record was deleted",
            ),
        ],
    );
    proofs.record(
        "P3 startup order: Hermes before the executor",
        vec![
            check(
                restart_start
                    .as_ref()
                    .is_some_and(|(_, line)| line.contains("intent=select/")),
                format!(
                    "first gateway start after the restart saw the record: {restart_start:?}; it ended: {restart_outcome}"
                ),
            ),
            check(
                match (&restart_start, resumed_at) {
                    (Some((gateway, _)), Some(resumed)) => *gateway <= resumed,
                    _ => false,
                },
                format!(
                    "gateway spawned at {:?} ≤ the resumed executor's first write at {resumed_at:?}",
                    restart_start.as_ref().map(|(ms, _)| ms)
                ),
            ),
        ],
    );

    // P5 + P6: the launcher step skips at `accepted`, clears at `cleanup`;
    // `hermes serve` never starts while the record exists.
    run.v1_openrouter("sk-or-v1-e0-fake-key-three", OR_MODEL)
        .await;
    run.seed_hermes_openrouter_state();
    let seeded = run.helper_facts();
    let serve_before = run.serve_starts();
    run.stop_agentd().await;
    let op = run.write_crashed_disconnect();
    // The start after a crash at `accepted`: the real launcher step, run by
    // the stub with the record on disk and no agentd, must change nothing.
    let skip = run.launcher_start_alone().await;
    let skipped = run.helper_facts();
    let restarted_at = now_ms();
    run.start_agentd().await;
    let ended = run.operation_end(&op).await;
    let finished_at = now_ms();
    let readies = run.stub_lines_since(restarted_at, "gateway-ready");
    let after = run.helper_facts();
    let deleted_at = timeline
        .lock()
        .unwrap()
        .intent
        .iter()
        .find(|(ms, state, _)| *ms > restarted_at && state == "absent")
        .map(|(ms, _, _)| *ms);
    let applied = readies
        .iter()
        .find(|(_, line)| line.contains("\"applied\":\"yes\""))
        .map(|(_, line)| line.clone())
        .unwrap_or_default();
    proofs.observe(
        "P5 disconnect (resumed at accepted) gateway starts",
        run.stub_story(restarted_at, finished_at),
    );
    proofs.record(
        "P5 disconnect clears in the launcher step (F1)",
        vec![
            check(
                seeded["openrouter"]["manual_pool_entries"] == "present"
                    && seeded["session_overrides"]["openrouter"] == "present",
                format!(
                    "seeded: pool {} override {}",
                    seeded["openrouter"]["manual_pool_entries"],
                    seeded["session_overrides"]["openrouter"]
                ),
            ),
            check(
                skip.contains("intent=disconnect/accepted/running")
                    && skip.contains("phase_before_cleanup")
                    && skip.contains("hermes_state_changed=no"),
                format!("start at phase accepted: {skip}"),
            ),
            check(
                skipped["openrouter"]["manual_pool_entries"] == "present"
                    && skipped["session_overrides"]["openrouter"] == "present",
                "after that start the pool entry and the override are still there",
            ),
            check(
                !applied.is_empty(),
                format!("a launcher step during the operation reported applied: {applied}"),
            ),
            check(
                after["openrouter"]["manual_pool_entries"] == "none"
                    && after["session_overrides"]["openrouter"] == "absent"
                    && after["openrouter"]["dotenv_key"] == "absent",
                format!(
                    "after: pool {} override {} .env {}",
                    after["openrouter"]["manual_pool_entries"],
                    after["session_overrides"]["openrouter"],
                    after["openrouter"]["dotenv_key"]
                ),
            ),
            check(
                ended["inference"]["operation"]["state"] == "succeeded"
                    && ended["inference"]["saved"]["route"] == "finite_private",
                format!("operation {}", ended["inference"]["operation"]),
            ),
        ],
    );
    let mut p6 = Vec::new();
    if run.fakes.serve_enabled {
        let serve_after = run.serve_starts();
        let new_starts = serve_after
            .iter()
            .filter(|start| !serve_before.contains(start))
            .collect::<Vec<_>>();
        p6.push(check(
            !serve_before.is_empty(),
            format!("serve ran before the disconnect: {serve_before:?}"),
        ));
        p6.push(check(
            deleted_at.is_some_and(|deleted| new_starts.iter().all(|(ms, _)| *ms >= deleted)) && !new_starts.is_empty(),
            format!("with the record present at startup, serve started only after deletion ({deleted_at:?}): {new_starts:?}"),
        ));
        // A running serve is stopped at cleanup and restarted after deletion.
        run.v1_openrouter("sk-or-v1-e0-fake-key-four", OR_MODEL)
            .await;
        let deadline = Instant::now() + Duration::from_secs(30);
        let running = loop {
            if let Some((_, pid)) = run
                .serve_starts()
                .last()
                .copied()
                .filter(|(_, pid)| alive(*pid))
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "serve never ran");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let started_at = now_ms();
        let reply = run.disconnect("openrouter").await;
        let id = reply["body"]["operation_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        run.operation_end(&id).await;
        proofs.observe(
            "P6 disconnect with serve running, gateway starts",
            run.stub_story(started_at, now_ms()),
        );
        let (stopped, phase_at_stop, deleted) = {
            let timeline = timeline.lock().unwrap();
            let stopped = timeline
                .serve
                .iter()
                .find(|(ms, pid, now)| *ms >= started_at && *pid == running && !now)
                .map(|(ms, _, _)| *ms);
            let phase_at_stop = stopped.and_then(|stop| {
                timeline
                    .intent
                    .iter()
                    .rev()
                    .find(|(ms, _, _)| *ms <= stop)
                    .map(|(_, state, _)| state.clone())
            });
            let deleted = timeline
                .intent
                .iter()
                .find(|(ms, state, _)| *ms > started_at && state == "absent")
                .map(|(ms, _, _)| *ms);
            (stopped, phase_at_stop, deleted)
        };
        // The pull loop starts the child on its next reconcile after the
        // record is gone: wait for it (bounded) before checking the order.
        let deadline = Instant::now() + Duration::from_secs(30);
        let restarted = loop {
            let restarted = run
                .serve_starts()
                .into_iter()
                .filter(|(ms, _)| *ms > started_at)
                .collect::<Vec<_>>();
            if !restarted.is_empty() || Instant::now() >= deadline {
                break restarted;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        p6.push(check(
            stopped.is_some()
                && phase_at_stop.as_deref().is_some_and(|state| {
                    state.starts_with("disconnect/cleanup")
                        || state.starts_with("disconnect/credential_removed")
                }),
            format!(
                "running serve {running} stopped at {stopped:?}, intent then {phase_at_stop:?}"
            ),
        ));
        p6.push(check(
            !restarted.is_empty()
                && deleted.is_some_and(|deleted| restarted.iter().all(|(ms, _)| *ms >= deleted)),
            format!("restarted only after deletion ({deleted:?}): {restarted:?}"),
        ));
    } else {
        p6.push(check(
            false,
            format!("not run: port {NATIVE_PORT} is in use, so serve stayed disabled"),
        ));
    }
    proofs.record("P6 hermes serve gating", p6);

    // P4b: a stale writer re-adds the .env key after every start.
    run.v1_openrouter("sk-or-v1-e0-fake-key-five", OR_MODEL)
        .await;
    run.set_stub_mode("readd-env-key");
    let before = now_ms();
    let reply = run.disconnect("openrouter").await;
    let id = reply["body"]["operation_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let ended = run.operation_end(&id).await;
    let starts = run.spawns_seeing(before, "intent=disconnect/");
    proofs.observe(
        "P4b disconnect with a stale writer, gateway starts",
        run.stub_story(before, now_ms()),
    );
    run.set_stub_mode("none");
    let retry = run.disconnect("openrouter").await;
    let retried = run.operation_end(&id).await;
    let mut p4 = p4;
    p4.push(check(
        ended["inference"]["operation"]["state"] == "failed"
            && ended["inference"]["operation"]["error_code"] == "verify_failed",
        format!(
            "re-added key: operation {}",
            ended["inference"]["operation"]
        ),
    ));
    p4.push(check(
        starts.len() == 3,
        format!(
            "{} gateway starts for the disconnect (1 + 2 re-runs) before verify_failed",
            starts.len()
        ),
    ));
    p4.push(check(
        retry["body"]["operation_id"] == id
            && retried["inference"]["operation"]["state"] == "succeeded",
        format!(
            "the failed disconnect resumes under the same id and succeeds: {}",
            retried["inference"]["operation"]
        ),
    ));
    proofs.record("P4 value-based verification", p4);

    let status = run.status().await;
    let facts = run.helper_facts();
    p7.push(check(
        status["inference"]["routes"]["openrouter"]["other_pool_keys"] == "none"
            && facts["openrouter"]["manual_pool_entries"] == "none",
        format!(
            "after disconnects: other_pool_keys {}",
            status["inference"]["routes"]["openrouter"]["other_pool_keys"]
        ),
    ));
    let requests = run.fakes.openrouter_requests.lock().unwrap().clone();
    p7.push(check(
        !requests.is_empty() && requests.iter().all(|line| line == "GET /api/v1/key"),
        format!(
            "OpenRouter fake saw only GET /key ({} requests)",
            requests.len()
        ),
    ));
    proofs.record("P7 helper facts on real files", p7);
    proofs
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    assert!(
        port_is_free(args.port),
        "port {} is in use; pass --port",
        args.port
    );
    let native_free = port_is_free(NATIVE_PORT);
    let fakes = Arc::new(Fakes {
        unacked: Mutex::new(Vec::new()),
        results: Mutex::new(HashMap::new()),
        seq: AtomicU64::new(0),
        streams: AtomicU64::new(0),
        openrouter_requests: Mutex::new(Vec::new()),
        hwd_headers: Mutex::new(Vec::new()),
        serve_enabled: native_free,
    });
    let listener = TcpListener::bind(("127.0.0.1", args.port))
        .await
        .expect("bind the E-0 port");
    tokio::spawn(serve_http(listener, Arc::clone(&fakes)));
    if native_free {
        // Hold the native port: agentd's readiness probe reaches only us.
        let native = TcpListener::bind(("127.0.0.1", NATIVE_PORT))
            .await
            .expect("bind the native port");
        tokio::spawn(async move {
            loop {
                if let Ok((mut stream, _)) = native.accept().await {
                    let _ = read_request(&mut stream).await;
                    respond(
                        &mut stream,
                        503,
                        &json!({"error": "E-0 has no native Hermes"}),
                    )
                    .await;
                }
            }
        });
    }
    let mut run = Run::prepare(&args, Arc::clone(&fakes));
    println!("E-0 run directory: {}", run.dir.display());
    let timeline = Arc::new(Mutex::new(Timeline::default()));
    observe(run.dir.clone(), run.intent_path(), Arc::clone(&timeline));

    let passed = match args.mode {
        Mode::Serve => {
            run.start_agentd().await;
            println!(
                "E-0 ready. Point the design fixture at it:\n  FC_DESIGN_RUNTIME_COMMANDS_URL=http://127.0.0.1:{} just dev web-design\nCtrl-C stops agentd and everything it started.",
                args.port
            );
            let _ = tokio::signal::ctrl_c().await;
            true
        }
        Mode::Smoke => {
            let proofs = smoke(&mut run, Arc::clone(&timeline)).await;
            let observations = proofs
                .observations
                .iter()
                .map(|(name, lines)| {
                    format!(
                        "observed: {name}\n{}",
                        lines
                            .iter()
                            .map(|line| format!("   {line}\n"))
                            .collect::<String>()
                    )
                })
                .collect::<String>();
            let summary = proofs
                .results
                .iter()
                .map(|(name, passed, lines)| {
                    format!(
                        "{name}: {}\n{}",
                        if *passed { "PASS" } else { "FAIL" },
                        lines
                            .iter()
                            .map(|line| format!("   {line}\n"))
                            .collect::<String>()
                    )
                })
                .collect::<String>();
            fs::write(
                run.dir.join("summary.txt"),
                format!("{summary}\n{observations}"),
            )
            .expect("summary");
            proofs.results.iter().all(|(_, passed, _)| *passed)
        }
    };
    run.stop_agentd().await;
    let leftovers = run
        .started_pids
        .iter()
        .filter(|pid| alive(**pid))
        .collect::<Vec<_>>();
    let headers = fakes.hwd_headers.lock().unwrap().clone();
    println!(
        "\nE-0 stopped. agentd processes still alive: {leftovers:?}. HWD calls with the fixture's headers: {}/{}.",
        headers.iter().filter(|(auth, user)| *auth && *user).count(),
        headers.len()
    );
    println!("Summary: {}", run.dir.join("summary.txt").display());
    std::process::exit(if passed && leftovers.is_empty() { 0 } else { 1 });
}
