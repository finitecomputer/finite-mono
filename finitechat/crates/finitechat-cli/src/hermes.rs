//! The `finitechat hermes` subcommand family: the JSON bridge
//! the Hermes platform plugin shells to (ADR 0002), plus agent onboarding
//! (`init` publishes the Agent Principal profile; the resident service starts
//! the Device runtime and publishes KeyPackages; rooms admit members through
//! MLS Add + Welcome).
//!
//! The agent's account key is the shared Finite identity at
//! `$FINITE_HOME/identity/identity.json` (else `~/.finite/identity/`),
//! minted by whichever Finite tool runs first and never copied into the
//! agent home. The agent's durable home lives under `--home` /
//! `$FINITECHAT_HOME`: `config.json`, the encrypted client store
//! `client.sqlite3`, and sidecar state files.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path as AxumPath, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use finite_identity::{FiniteIdentity, IdentityPaths};
use finitechat_blob::{
    AttachmentBlobError, BlossomDownloadHttpResponse, finish_blossom_download_http_response,
    prepare_blossom_download_http_request, sha256_hex,
};
use finitechat_client::{
    ClientStoreError, FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery,
    ReqwestHttpRuntimeTransport, SqliteClientStore, SqliteClientStoreOptions, StoredAppEvent,
    StoredAppMessage, StoredAppRoom, StoredAppRoomState,
};
use finitechat_core::{
    AppAction, AppBridgeActivityInput, AppBridgeSync, AppRoomState, AppSentMessage,
    AppTopicSummary, ChatMediaKind, FiniteChatCoreError, FiniteChatRuntime, OpenOptions,
    OutboundAttachment,
};
use finitechat_hermes::{
    HERMES_MESSAGE_PAYLOAD_TYPE_V1, HermesAckRequestV1, HermesActivityRequestV1,
    HermesEditRequestV1, HermesMessagePayloadV1, HermesMessageStatusV1, HermesMessageTypeV1,
    HermesPollEventV1, HermesSendRequestV1, MAX_HERMES_METADATA_BYTES,
    MAX_HERMES_POLL_TIMEOUT_MILLIS, MAX_HERMES_TEXT_BYTES,
};
use finitechat_http::{NostrProfileRecord, SyncWaitRequest, SyncWaitRoom};
use finitechat_mls::NostrSecretKey;
use finitechat_proto::{
    AttachmentBlobReferenceV1, DecryptedApplicationEventV1, DeviceRef, DurableAppEventKind,
    EphemeralActivityActionV1, MAX_ATTACHMENT_PLAINTEXT_BYTES, MAX_RUNTIME_COMMAND_LEDGER_RECORDS,
    ProtocolLimitError, RuntimeCommandCancelV1, RuntimeCommandDeliveryAckV1,
    RuntimeCommandDeliveryV1, RuntimeCommandInboundPayloadV1, RuntimeCommandRequestV1,
    RuntimeCommandResultDeliveryV1, RuntimeStateSnapshotDeliveryV1, npub_encode,
};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::CliError;
use crate::cli::{
    HermesArgs, HermesCommand, HermesHomeChannelCommand, HermesHomeChannelSetArgs, HermesInitArgs,
    HermesInstallArgs, HermesRoomStatusArgs, HermesServeArgs,
};

const CONFIG_FILE: &str = "config.json";
const HERMES_INBOX_FILE: &str = "hermes-inbox.json";
const AGENTD_INBOX_FILE: &str = "agentd-inbox.json";
const HERMES_RUNNING_FILE: &str = "hermes-running.json";
const HERMES_HOME_CHANNEL_FILE: &str = "hermes-home-channel.json";
const BACKUP_ACTIVITY_FILE: &str = ".finitechat-backup-active";
const STORE_FILE: &str = "client.sqlite3";
const ATTACHMENT_CACHE_DIR: &str = "attachments";
const LEGACY_HERMES_PLUGIN_NAME: &str = "finite-platform";
const AMBIGUOUS_HERMES_PLUGIN_NAME: &str = "finite";
const HERMES_PLATFORM_NAME: &str = "finitechat";
const HERMES_PLUGIN_INIT: &str =
    include_str!("../../../integrations/hermes/finitechat/__init__.py");
const HERMES_PLUGIN_ADAPTER: &str =
    include_str!("../../../integrations/hermes/finitechat/adapter.py");
const HERMES_PLUGIN_YAML: &str =
    include_str!("../../../integrations/hermes/finitechat/plugin.yaml");
const HERMES_PLUGIN_ENV_FILE: &str = "finitechat.env";
const RESIDENT_BRIDGE_SYNC_INTERVAL_MILLIS: u64 = 10_000;
const CREDENTIAL_VALIDITY_SECONDS: u64 = 90 * 24 * 60 * 60;
const POLL_SLEEP_MS: u64 = 300;
const HERMES_STORED_EVENT_RECOVERY_LIMIT: u32 = 5_000;
/// How long a leased inbox entry stays owned by its consumer before the
/// sidecar returns it to `Pending` and re-delivers it. Generous on purpose:
/// a lease only expires when a consumer took an entry and neither acked nor
/// released it (a crash mid-turn), so the ceiling need only be larger than a
/// normal turn. Override with `FINITECHAT_HERMES_LEASE_TTL_MILLIS`.
const DEFAULT_HERMES_INBOX_LEASE_TTL_MILLIS: u64 = 15 * 60 * 1000;
const HERMES_INBOX_LEASE_TTL_ENV: &str = "FINITECHAT_HERMES_LEASE_TTL_MILLIS";
/// Policy for a `thread_id` that resolves to no conversation or segment in the
/// agent store. Unset (the default) routes unknown threads to Core's Home
/// default with a loud warning — the pre-d6a8b424 product semantics: a topic
/// archived mid-session must never silently consume the user's message, so the
/// reply stays deliverable and the routing decision is visible in the log.
/// Values: `home`/`default` spell the fallback explicitly; `error` (and any
/// other value) restores the strict typed error so a typo fails closed instead
/// of silently rerouting replies.
const HERMES_UNKNOWN_THREAD_ROUTE_ENV: &str = "FINITECHAT_HERMES_UNKNOWN_THREAD_ROUTE";
/// Bounded ring of recently-acked entry keys. It makes a post-restart
/// duplicate ack a no-op and blocks a redelivery of an already-acked entry
/// from durable recovery even if its seq is re-observed, so the sidecar is
/// idempotent without the Python dedup set that used to shadow it.
const MAX_HERMES_INBOX_ACKED_KEYS: usize = 4_096;
/// Monotonic source of lease ids within this process. Combined with the lease
/// timestamp it identifies exactly which delivery leased an entry.
static HERMES_LEASE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const HERMES_SERVICE_HEARTBEAT_MILLIS: u64 = 250;
const HERMES_ATTACHMENT_CONNECT_TIMEOUT_SECS: u64 = 5;
const HERMES_ATTACHMENT_DOWNLOAD_TIMEOUT_SECS: u64 = 30;
const HERMES_ATTACHMENT_UNAVAILABLE_NOTICE: &str =
    "An attachment could not be opened. Ask the user to resend it if you need to inspect it.";
/// Match the hosted-device batch ceiling so one Hermes send cannot make the
/// resident bridge buffer the protocol's per-file maximum sixteen times over.
const MAX_HERMES_LOCAL_ATTACHMENT_TOTAL_BYTES: usize = 64 * 1024 * 1024;
static PRIVATE_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const _: () = {
    assert!(MAX_HERMES_LOCAL_ATTACHMENT_TOTAL_BYTES >= MAX_ATTACHMENT_PLAINTEXT_BYTES as usize);
};

#[derive(Debug, Serialize, Deserialize)]
struct AgentConfig {
    server_url: String,
    device_id: String,
    account_id: String,
}

struct AgentHome {
    dir: PathBuf,
    config: AgentConfig,
    secret: NostrSecretKey,
}

pub(crate) fn run<W: Write>(args: HermesArgs, output: &mut W) -> Result<(), CliError> {
    let home_dir = resolve_home(args.agent_home)?;
    let json_mode = args.json;
    let request_json = args.request_json;

    match args.command {
        HermesCommand::Init(args) => cmd_init(&home_dir, args, output),
        HermesCommand::Install(args) => cmd_install(&home_dir, args, json_mode, output),
        HermesCommand::Serve(args) => cmd_serve(&home_dir, args, json_mode, output),
        HermesCommand::HomeChannel { command } => cmd_home_channel(&home_dir, command, output),
        HermesCommand::RoomStatus(args) => cmd_room_status(&home_dir, args, json_mode, output),
        HermesCommand::Poll => with_backup_activity(&home_dir, "poll", || {
            cmd_poll(&home_dir, read_request(request_json)?, output)
        }),
        HermesCommand::Ack => with_backup_activity(&home_dir, "ack", || {
            cmd_ack(&home_dir, read_request(request_json)?, output)
        }),
        HermesCommand::Release => with_backup_activity(&home_dir, "release", || {
            cmd_release(&home_dir, read_request(request_json)?, output)
        }),
        HermesCommand::Send => with_backup_activity(&home_dir, "send", || {
            cmd_send(&home_dir, read_request(request_json)?, output)
        }),
        HermesCommand::Edit => with_backup_activity(&home_dir, "edit", || {
            cmd_edit(&home_dir, read_request(request_json)?, output)
        }),
        HermesCommand::Recover => with_backup_activity(&home_dir, "recover", || {
            cmd_recover(&home_dir, read_request(request_json)?, output)
        }),
        HermesCommand::Activity => with_backup_activity(&home_dir, "activity", || {
            cmd_activity(&home_dir, read_request(request_json)?, output)
        }),
    }
}

#[derive(Debug, Serialize)]
struct HermesInstallSummary {
    plugin_name: String,
    platform_name: String,
    plugin_dir: String,
    agent_home: String,
    finitechat_bin: String,
    files: Vec<String>,
    recommended_config: String,
    warnings: Vec<String>,
    legacy_plugin_conflicts: Vec<HermesInstallLegacyPluginConflict>,
    legacy_config_conflicts: Vec<HermesInstallLegacyConfigConflict>,
}

#[derive(Debug, Serialize)]
struct HermesInstallLegacyPluginConflict {
    plugin_name: String,
    plugin_dir: String,
    reason: String,
}

#[derive(Debug, Serialize)]
struct HermesInstallLegacyConfigConflict {
    config_path: String,
    enabled_plugin: String,
    replacement_plugin: String,
}

fn cmd_install<W: Write>(
    home_dir: &Path,
    args: HermesInstallArgs,
    json_mode: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let HermesInstallArgs {
        plugin_dir: plugin_dir_arg,
        plugins_dir: plugins_dir_arg,
        plugin_name,
        finitechat_bin: finitechat_bin_arg,
        service_url,
        force,
    } = args;

    validate_plugin_name(&plugin_name)?;
    // clap already rejects passing both --plugin-dir and --plugins-dir.
    if !home_dir.join(CONFIG_FILE).exists() {
        return Err(CliError::Hermes(format!(
            "agent home {} is not initialized (run finitechat hermes init first)",
            home_dir.display()
        )));
    }

    let (plugin_dir, plugins_dir_for_audit) = match plugin_dir_arg {
        Some(path) => {
            let plugin_dir = PathBuf::from(path);
            let plugins_dir = plugin_dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            (plugin_dir, plugins_dir)
        }
        None => {
            let plugins_dir = plugins_dir_arg
                .map(PathBuf::from)
                .map(Ok)
                .unwrap_or_else(default_hermes_plugins_dir)?;
            let plugin_dir = plugins_dir.join(&plugin_name);
            (plugin_dir, plugins_dir)
        }
    };
    let finitechat_bin = match finitechat_bin_arg {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe().map_err(|error| {
            CliError::Hermes(format!("could not resolve current executable: {error}"))
        })?,
    };

    fs::create_dir_all(&plugin_dir).map_err(|error| CliError::Hermes(error.to_string()))?;
    let mut installed = Vec::new();
    write_managed_plugin_file(
        &plugin_dir.join("__init__.py"),
        HERMES_PLUGIN_INIT,
        force,
        &mut installed,
    )?;
    write_managed_plugin_file(
        &plugin_dir.join("adapter.py"),
        HERMES_PLUGIN_ADAPTER,
        force,
        &mut installed,
    )?;
    let plugin_yaml = hermes_plugin_yaml_for_install(&plugin_name);
    write_managed_plugin_file(
        &plugin_dir.join("plugin.yaml"),
        &plugin_yaml,
        force,
        &mut installed,
    )?;
    let env_contents =
        hermes_plugin_env_contents(home_dir, &finitechat_bin, service_url.as_deref())?;
    write_managed_private_file(
        &plugin_dir.join(HERMES_PLUGIN_ENV_FILE),
        &env_contents,
        force,
        &mut installed,
    )?;

    let legacy_plugin_conflicts =
        detect_legacy_plugin_conflicts(&plugins_dir_for_audit, &plugin_dir, &plugin_name);
    let legacy_config_conflicts =
        detect_legacy_config_conflicts(&plugins_dir_for_audit, &plugin_name);
    let mut warnings = Vec::new();
    for conflict in &legacy_plugin_conflicts {
        warnings.push(format!(
            "found legacy Hermes plugin '{}' at {}; {}",
            conflict.plugin_name, conflict.plugin_dir, conflict.reason
        ));
    }
    for conflict in &legacy_config_conflicts {
        warnings.push(format!(
            "{} enables legacy plugin '{}'; change plugins.enabled to '{}'",
            conflict.config_path, conflict.enabled_plugin, conflict.replacement_plugin
        ));
    }

    let summary = HermesInstallSummary {
        plugin_name: plugin_name.clone(),
        platform_name: HERMES_PLATFORM_NAME.to_owned(),
        plugin_dir: plugin_dir.display().to_string(),
        agent_home: home_dir.display().to_string(),
        finitechat_bin: finitechat_bin.display().to_string(),
        files: installed,
        recommended_config: hermes_recommended_config(&plugin_name, home_dir),
        warnings,
        legacy_plugin_conflicts,
        legacy_config_conflicts,
    };
    if json_mode {
        crate::write_pretty_json(output, &summary)
    } else {
        writeln!(
            output,
            "Installed Finite Chat Hermes plugin '{}' at {}",
            summary.plugin_name, summary.plugin_dir
        )
        .map_err(CliError::Output)?;
        writeln!(output, "Agent home: {}", summary.agent_home).map_err(CliError::Output)?;
        writeln!(output, "finitechat binary: {}", summary.finitechat_bin)
            .map_err(CliError::Output)?;
        writeln!(output, "Enable with:\n{}", summary.recommended_config)
            .map_err(CliError::Output)?;
        for warning in &summary.warnings {
            writeln!(output, "WARNING: {warning}").map_err(CliError::Output)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
struct HermesServiceStarted {
    service: &'static str,
    version: &'static str,
    url: String,
    addr: String,
    agent_home: String,
    account_id: String,
    device_id: String,
    server_url: String,
    pid: u32,
}

#[derive(Clone)]
struct HermesServiceState {
    agent_home: PathBuf,
    account_id: String,
    device_id: String,
    server_url: String,
    runtime: Arc<FiniteChatRuntime>,
    inbox_lock: Arc<Mutex<()>>,
    running_lock: Arc<Mutex<()>>,
    bridge_updates: Arc<(Mutex<u64>, Condvar)>,
    joined_account_ids: Arc<Mutex<Vec<String>>>,
}

#[derive(Debug, Serialize)]
struct HermesServiceHealth {
    status: &'static str,
    service: &'static str,
    version: &'static str,
    agent_home: String,
    account_id: String,
    device_id: String,
    server_url: String,
}

#[derive(Debug, Deserialize, Default)]
struct HermesInboundQuery {
    #[serde(default)]
    room_id: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    timeout_millis: Option<u64>,
}

struct PreparedHermesService {
    listener: tokio::net::TcpListener,
    state: HermesServiceState,
    started: HermesServiceStarted,
}

fn cmd_serve<W: Write>(
    home_dir: &Path,
    args: HermesServeArgs,
    json_mode: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let HermesServeArgs {
        addr,
        ready_file: ready_file_arg,
    } = args;
    let ready_file = ready_file_arg.map(PathBuf::from);

    let home = load_home(home_dir)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            CliError::Hermes(format!("could not start Hermes service runtime: {error}"))
        })?;
    let prepared = runtime.block_on(prepare_hermes_service(home_dir, &home, addr, ready_file))?;
    if json_mode {
        crate::write_pretty_json(output, &prepared.started)?;
    } else {
        writeln!(
            output,
            "finitechat hermes service listening on {}",
            prepared.started.url
        )
        .map_err(CliError::Output)?;
    }
    output.flush().map_err(CliError::Output)?;
    runtime.block_on(serve_prepared_hermes_service(prepared))
}

async fn prepare_hermes_service(
    home_dir: &Path,
    home: &AgentHome,
    addr: SocketAddr,
    ready_file: Option<PathBuf>,
) -> Result<PreparedHermesService, CliError> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| CliError::Hermes(format!("could not bind Hermes service: {error}")))?;
    let bound_addr = listener
        .local_addr()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let url = format!("http://{bound_addr}");
    let runtime = open_agent_runtime(home)?;
    // Service readiness means this Device is actually reachable for the
    // Welcome-first product flow, not merely that its local SQLite file opens.
    // StartRuntime publishes/replenishes KeyPackages and activates pending
    // Welcomes on every resident-service start, which also makes restart the
    // natural healing path after Chat/server interruption.
    runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(map_core_hermes_error)?;
    let state = HermesServiceState {
        agent_home: home_dir.to_path_buf(),
        account_id: home.config.account_id.clone(),
        device_id: home.config.device_id.clone(),
        server_url: home.config.server_url.clone(),
        runtime,
        inbox_lock: Arc::new(Mutex::new(())),
        running_lock: Arc::new(Mutex::new(())),
        bridge_updates: Arc::new((Mutex::new(0), Condvar::new())),
        joined_account_ids: Arc::new(Mutex::new(Vec::new())),
    };
    start_resident_bridge_sync(state.clone())?;
    let started = HermesServiceStarted {
        service: "finitechat-hermes",
        version: env!("CARGO_PKG_VERSION"),
        url,
        addr: bound_addr.to_string(),
        agent_home: state.agent_home.display().to_string(),
        account_id: state.account_id.clone(),
        device_id: state.device_id.clone(),
        server_url: state.server_url.clone(),
        pid: std::process::id(),
    };
    if let Some(path) = ready_file {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| CliError::Hermes(error.to_string()))?;
        }
        write_private(
            path,
            &serde_json::to_string_pretty(&started).map_err(CliError::Serialize)?,
        )?;
    }
    Ok(PreparedHermesService {
        listener,
        state,
        started,
    })
}

async fn serve_prepared_hermes_service(prepared: PreparedHermesService) -> Result<(), CliError> {
    axum::serve(
        prepared.listener,
        hermes_service_router(prepared.state).into_make_service(),
    )
    .await
    .map_err(|error| CliError::Hermes(format!("Hermes service failed: {error}")))
}

fn hermes_service_router(state: HermesServiceState) -> Router {
    Router::new()
        .route("/healthz", get(hermes_service_healthz))
        .route("/readyz", get(hermes_service_readyz))
        .route("/v1/hermes/inbound", get(hermes_service_inbound))
        .route("/v1/hermes/{action}", post(hermes_service_action))
        .route("/v1/agentd/inbound", get(agentd_service_inbound))
        .route("/v1/agentd/ack", post(agentd_service_ack))
        .route("/v1/agentd/result", post(agentd_service_result))
        .route("/v1/agentd/state", post(agentd_service_state))
        .with_state(state)
}

fn start_resident_bridge_sync(state: HermesServiceState) -> Result<(), CliError> {
    std::thread::Builder::new()
        .name("finitechat-resident-sync".to_owned())
        .spawn(move || {
            let mut retry_millis = 250u64;
            let mut sync_immediately = true;
            loop {
                let result = if sync_immediately {
                    state.runtime.agent_bridge_poll_once()
                } else {
                    state
                        .runtime
                        .agent_bridge_wait_for_update(RESIDENT_BRIDGE_SYNC_INTERVAL_MILLIS)
                };
                match result {
                    Ok(bridge) => {
                        retry_millis = 250;
                        sync_immediately = false;
                        let has_updates = bridge_sync_has_updates(&bridge);
                        if !bridge.joined_account_ids.is_empty()
                            && let Ok(mut joined) = state.joined_account_ids.lock()
                        {
                            joined.extend(bridge.joined_account_ids);
                            joined.sort();
                            joined.dedup();
                        }
                        if has_updates {
                            signal_bridge_update(&state);
                        }
                    }
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(retry_millis));
                        retry_millis = (retry_millis.saturating_mul(2)).min(5_000);
                        sync_immediately = true;
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|error| CliError::Hermes(format!("could not start resident sync: {error}")))
}

fn bridge_sync_has_updates(bridge: &AppBridgeSync) -> bool {
    !bridge.joined_account_ids.is_empty() || !bridge.events.is_empty()
}

fn signal_bridge_update(state: &HermesServiceState) {
    let (generation, wake) = &*state.bridge_updates;
    if let Ok(mut generation) = generation.lock() {
        *generation = generation.wrapping_add(1);
        wake.notify_all();
    }
}

fn bridge_generation(state: &HermesServiceState) -> u64 {
    state
        .bridge_updates
        .0
        .lock()
        .map(|generation| *generation)
        .unwrap_or(0)
}

fn wait_for_bridge_update(
    state: &HermesServiceState,
    observed_generation: u64,
    timeout_millis: u64,
) {
    let (generation, wake) = &*state.bridge_updates;
    let Ok(guard) = generation.lock() else {
        return;
    };
    if *guard != observed_generation {
        return;
    }
    let _ = wake.wait_timeout_while(
        guard,
        Duration::from_millis(timeout_millis.max(1)),
        |generation| *generation == observed_generation,
    );
}

fn take_joined_accounts(state: &HermesServiceState) -> Vec<String> {
    let Ok(mut joined) = state.joined_account_ids.lock() else {
        return Vec::new();
    };
    std::mem::take(&mut *joined)
}

async fn hermes_service_healthz(
    State(state): State<HermesServiceState>,
) -> Json<HermesServiceHealth> {
    Json(HermesServiceHealth {
        status: "ok",
        service: "finitechat-hermes",
        version: env!("CARGO_PKG_VERSION"),
        agent_home: state.agent_home.display().to_string(),
        account_id: state.account_id,
        device_id: state.device_id,
        server_url: state.server_url,
    })
}

async fn hermes_service_readyz(
    State(state): State<HermesServiceState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let result = tokio::task::spawn_blocking(move || {
        let app = state.runtime.state().map_err(map_core_hermes_error)?;
        Ok(json!({
            "status": "ready",
            "service": "finitechat-hermes",
            "version": env!("CARGO_PKG_VERSION"),
            "agent_home": state.agent_home.display().to_string(),
            "account_id": state.account_id,
            "device_id": state.device_id,
            "server_url": state.server_url,
            "store": "ok",
            "store_file": state.agent_home.join(STORE_FILE).display().to_string(),
            "rooms": app.rooms.len(),
            "messages": app.messages.len(),
        }))
    })
    .await
    .map_err(|error| service_internal_error(error.to_string()))?;
    result.map(Json).map_err(service_cli_error)
}

async fn hermes_service_action(
    State(state): State<HermesServiceState>,
    AxumPath(action): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let result =
        tokio::task::spawn_blocking(move || handle_hermes_service_action(&state, &action, payload))
            .await
            .map_err(|error| service_internal_error(error.to_string()))?;
    result.map(Json).map_err(service_cli_error)
}

async fn hermes_service_inbound(
    State(state): State<HermesServiceState>,
    Query(query): Query<HermesInboundQuery>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, Infallible>>(32);
    std::thread::spawn(move || {
        if let Err(error) = run_hermes_inbound_stream(state, query, tx.clone()) {
            let record = json!({
                "type": "error",
                "error": error.to_string(),
            });
            if let Ok(line) = serde_json::to_string(&record) {
                let _ = tx.blocking_send(Ok(Bytes::from(format!("{line}\n"))));
            }
        }
    });

    let body_stream = stream::unfold(
        (
            rx,
            tokio::time::interval(Duration::from_millis(HERMES_SERVICE_HEARTBEAT_MILLIS)),
        ),
        |(mut rx, mut heartbeat)| async move {
            tokio::select! {
                item = rx.recv() => item.map(|bytes| (bytes, (rx, heartbeat))),
                _ = heartbeat.tick() => Some((Ok(Bytes::from_static(b"\n")), (rx, heartbeat))),
            }
        },
    );

    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::from_stream(body_stream),
    )
        .into_response())
}

async fn agentd_service_inbound(
    State(state): State<HermesServiceState>,
    Query(query): Query<HermesInboundQuery>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, Infallible>>(32);
    std::thread::spawn(move || {
        if let Err(error) = run_agentd_inbound_stream(state, query, tx.clone()) {
            let record = json!({
                "type": "error",
                "error": error.to_string(),
            });
            if let Ok(line) = serde_json::to_string(&record) {
                let _ = tx.blocking_send(Ok(Bytes::from(format!("{line}\n"))));
            }
        }
    });

    let body_stream = stream::unfold(
        (
            rx,
            tokio::time::interval(Duration::from_millis(HERMES_SERVICE_HEARTBEAT_MILLIS)),
        ),
        |(mut rx, mut heartbeat)| async move {
            tokio::select! {
                item = rx.recv() => item.map(|bytes| (bytes, (rx, heartbeat))),
                _ = heartbeat.tick() => Some((Ok(Bytes::from_static(b"\n")), (rx, heartbeat))),
            }
        },
    );

    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::from_stream(body_stream),
    )
        .into_response())
}

async fn agentd_service_ack(
    State(state): State<HermesServiceState>,
    Json(ack): Json<RuntimeCommandDeliveryAckV1>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let result = tokio::task::spawn_blocking(move || handle_agentd_ack(&state, ack))
        .await
        .map_err(|error| service_internal_error(error.to_string()))?;
    result.map(Json).map_err(service_cli_error)
}

async fn agentd_service_result(
    State(state): State<HermesServiceState>,
    Json(delivery): Json<RuntimeCommandResultDeliveryV1>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let result = tokio::task::spawn_blocking(move || handle_agentd_result(&state, delivery))
        .await
        .map_err(|error| service_internal_error(error.to_string()))?;
    result.map(Json).map_err(service_cli_error)
}

async fn agentd_service_state(
    State(state): State<HermesServiceState>,
    Json(delivery): Json<RuntimeStateSnapshotDeliveryV1>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let result = tokio::task::spawn_blocking(move || handle_agentd_state(&state, delivery))
        .await
        .map_err(|error| service_internal_error(error.to_string()))?;
    result.map(Json).map_err(service_cli_error)
}

fn handle_agentd_ack(
    state: &HermesServiceState,
    ack: RuntimeCommandDeliveryAckV1,
) -> Result<Value, CliError> {
    ack.validate_limits()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let _guard = lock_service_mutex(&state.inbox_lock)?;
    let mut inbox = load_agentd_inbox(&state.agent_home)?;
    let key = agentd_inbox_key(&ack.room_id, ack.seq, &ack.message_id);
    let before = inbox.events.len();
    inbox.events.retain(|event| event.key != key);
    if inbox.events.len() != before {
        save_agentd_inbox(&state.agent_home, &inbox)?;
        signal_bridge_update(state);
    }
    Ok(json!({
        "acked": inbox.events.len() != before,
        "room_id": ack.room_id,
        "seq": ack.seq,
        "message_id": ack.message_id,
    }))
}

fn handle_agentd_result(
    state: &HermesServiceState,
    delivery: RuntimeCommandResultDeliveryV1,
) -> Result<Value, CliError> {
    delivery
        .validate_structure()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let payload = serde_json::to_vec(&delivery.result).map_err(CliError::Serialize)?;
    let message_id = state
        .runtime
        .send_runtime_command_result_and_wait(delivery.room_id, delivery.conversation_id, payload)
        .map_err(map_core_hermes_error)?;
    Ok(json!({ "message_id": message_id }))
}

fn handle_agentd_state(
    state: &HermesServiceState,
    delivery: RuntimeStateSnapshotDeliveryV1,
) -> Result<Value, CliError> {
    delivery
        .validate_limits()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let payload = serde_json::to_vec(&delivery.snapshot).map_err(CliError::Serialize)?;
    let message_id = state
        .runtime
        .send_runtime_state_snapshot_and_wait(delivery.room_id, delivery.conversation_id, payload)
        .map_err(map_core_hermes_error)?;
    Ok(json!({ "message_id": message_id }))
}

fn handle_hermes_service_action(
    state: &HermesServiceState,
    action: &str,
    payload: Value,
) -> Result<Value, CliError> {
    match action {
        "poll" => handle_hermes_service_poll(state, payload),
        "ack" => {
            let result = {
                let _guard = lock_service_mutex(&state.inbox_lock)?;
                output_json_value(|output| cmd_ack(&state.agent_home, payload, output))
            };
            if result.is_ok() {
                signal_bridge_update(state);
            }
            result
        }
        "release" => {
            let result = {
                let _guard = lock_service_mutex(&state.inbox_lock)?;
                output_json_value(|output| cmd_release(&state.agent_home, payload, output))
            };
            // A released entry is Pending again; wake the stream so it is
            // redelivered promptly instead of waiting out the poll interval.
            if result.is_ok() {
                signal_bridge_update(state);
            }
            result
        }
        "send" => {
            let mut request: HermesSendRequestV1 =
                serde_json::from_value(payload).map_err(CliError::Json)?;
            let _guard = lock_service_mutex(&state.running_lock)?;
            resolve_hermes_send_route(&state.runtime, &mut request)?;
            let sent = send_hermes_request_with_runtime(&state.runtime, &request)?;
            update_running_after_send(&state.agent_home, &request, &sent.message_id)?;
            Ok(sent_message_value(&sent))
        }
        "edit" => {
            let mut request: HermesEditRequestV1 =
                serde_json::from_value(payload).map_err(CliError::Json)?;
            let _guard = lock_service_mutex(&state.running_lock)?;
            resolve_hermes_edit_route(&state.agent_home, &state.runtime, &mut request)?;
            let sent = edit_hermes_request_with_runtime(&state.runtime, &request)?;
            update_running_after_edit(&state.agent_home, &request)?;
            Ok(sent_message_value(&sent))
        }
        "recover" => handle_hermes_service_recover(state),
        "activity" => handle_hermes_service_activity(state, payload),
        "home-channel-show" => {
            output_json_value(|output| write_home_channel_show(&state.agent_home, output))
        }
        "home-channel-set" => {
            let request: HermesHomeChannelSetRequest =
                serde_json::from_value(payload).map_err(CliError::Json)?;
            output_json_value(|output| {
                set_home_channel(
                    &state.agent_home,
                    request.room_id,
                    request.conversation_id,
                    output,
                )
            })
        }
        "home-channel-clear" => {
            clear_home_channel(&state.agent_home)?;
            Ok(json!({ "cleared": true, "home_channel": null }))
        }
        _ => Err(CliError::Usage(format!(
            "unknown Hermes service action {action:?}"
        ))),
    }
}

fn output_json_value(
    f: impl FnOnce(&mut Vec<u8>) -> Result<(), CliError>,
) -> Result<Value, CliError> {
    let mut output = Vec::new();
    f(&mut output)?;
    serde_json::from_slice(&output).map_err(CliError::Json)
}

fn handle_hermes_service_poll(
    state: &HermesServiceState,
    payload: Value,
) -> Result<Value, CliError> {
    let request: PollRequest = serde_json::from_value(payload).map_err(CliError::Json)?;
    let timeout = normalized_hermes_poll_timeout(&request);
    let started = Instant::now();
    let home = load_home(&state.agent_home)?;

    loop {
        let observed_generation = bridge_generation(state);
        let payload = collect_hermes_service_inbound_payload(state, &home, &request)?;
        if hermes_inbound_payload_has_records(&payload) || started.elapsed() >= timeout {
            return Ok(payload);
        }

        let remaining = timeout.saturating_sub(started.elapsed()).as_millis() as u64;
        wait_for_bridge_update(state, observed_generation, remaining);
        let payload = collect_hermes_service_inbound_payload(state, &home, &request)?;
        if hermes_inbound_payload_has_records(&payload) || started.elapsed() >= timeout {
            return Ok(payload);
        }
    }
}

fn handle_hermes_service_recover(state: &HermesServiceState) -> Result<Value, CliError> {
    let _guard = lock_service_mutex(&state.running_lock)?;
    let running = load_hermes_running(&state.agent_home)?;
    let mut recovered = 0usize;
    for message in &running.messages {
        let recovery = HermesEditRequestV1 {
            room_id: message.room_id.clone(),
            conversation_id: message.conversation_id.clone(),
            segment_id: message.segment_id.clone(),
            thread_id: None,
            message_id: message.message_id.clone(),
            text: "Hermes gateway restarted before this turn completed.".to_owned(),
            kind: message.kind,
            status: HermesMessageStatusV1::Complete,
            finalize: true,
            metadata: BTreeMap::new(),
        };
        edit_hermes_request_with_runtime(&state.runtime, &recovery)?;
        recovered += 1;
    }
    if recovered > 0 {
        save_hermes_running(&state.agent_home, &HermesRunningState::default())?;
    }
    Ok(json!({ "recovered": recovered }))
}

fn handle_hermes_service_activity(
    state: &HermesServiceState,
    payload: Value,
) -> Result<Value, CliError> {
    let mut request: HermesActivityRequestV1 =
        serde_json::from_value(payload).map_err(CliError::Json)?;
    resolve_hermes_activity_route(&state.runtime, &mut request)?;
    let input = app_bridge_activity_input(request)?;
    let accepted = state
        .runtime
        .append_ephemeral_activity_and_wait(input)
        .map_err(map_core_hermes_error)?;
    Ok(json!({ "accepted": true, "result": accepted }))
}

fn run_hermes_inbound_stream(
    state: HermesServiceState,
    query: HermesInboundQuery,
    tx: tokio::sync::mpsc::Sender<Result<Bytes, Infallible>>,
) -> Result<(), CliError> {
    let home = load_home(&state.agent_home)?;
    let request = PollRequest {
        room_id: query.room_id,
        limit: query.limit,
        timeout_millis: query.timeout_millis,
    };
    let timeout_millis = normalized_hermes_poll_timeout(&request).as_millis() as u64;

    loop {
        let observed_generation = bridge_generation(&state);
        let payload = collect_hermes_service_inbound_payload(&state, &home, &request)?;
        if !send_hermes_inbound_payload(&tx, &payload)? {
            return Ok(());
        }
        wait_for_bridge_update(&state, observed_generation, timeout_millis);
    }
}

fn run_agentd_inbound_stream(
    state: HermesServiceState,
    query: HermesInboundQuery,
    tx: tokio::sync::mpsc::Sender<Result<Bytes, Infallible>>,
) -> Result<(), CliError> {
    let home = load_home(&state.agent_home)?;
    let room_filter = query.room_id.as_deref();
    let limit = query.limit.unwrap_or(32).clamp(1, 64) as usize;
    let timeout_millis = query
        .timeout_millis
        .unwrap_or(30_000)
        .clamp(1_000, MAX_HERMES_POLL_TIMEOUT_MILLIS);

    loop {
        let observed_generation = bridge_generation(&state);
        let deliveries = collect_agentd_inbound(&state, &home, room_filter, limit)?;
        for delivery in deliveries {
            let line = serde_json::to_string(&delivery).map_err(CliError::Serialize)?;
            if tx
                .blocking_send(Ok(Bytes::from(format!("{line}\n"))))
                .is_err()
            {
                return Ok(());
            }
        }
        wait_for_bridge_update(&state, observed_generation, timeout_millis);
    }
}

fn collect_agentd_inbound(
    state: &HermesServiceState,
    home: &AgentHome,
    room_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<RuntimeCommandDeliveryV1>, CliError> {
    let _guard = lock_service_mutex(&state.inbox_lock)?;
    let mut inbox = load_agentd_inbox(&state.agent_home)?;
    recover_stored_agentd_events(home, &state.account_id, room_filter, &mut inbox)?;
    Ok(inbox
        .events
        .iter()
        .filter(|event| room_filter.is_none_or(|room_id| event.room_id == room_id))
        .take(limit)
        .map(|event| event.delivery.clone())
        .collect())
}

fn collect_hermes_service_inbound_payload(
    state: &HermesServiceState,
    home: &AgentHome,
    request: &PollRequest,
) -> Result<Value, CliError> {
    let limit = normalized_hermes_poll_limit(request);
    let _guard = lock_service_mutex(&state.inbox_lock)?;
    let mut inbox = load_hermes_inbox(&state.agent_home)?;
    let recent_events = load_recent_agent_app_events(home)?;
    initialize_hermes_inbox_cursors_from_events(
        &state.agent_home,
        &mut inbox,
        &home.config.account_id,
        recent_events.iter(),
    )?;
    let joined = take_joined_accounts(state);

    recover_stored_hermes_events_from_events(
        &state.agent_home,
        home,
        &state.account_id,
        request.room_id.as_deref(),
        &mut inbox,
        recent_events,
    )?;
    let events = lease_pending_hermes_inbox_events(
        &state.agent_home,
        &mut inbox,
        request.room_id.as_deref(),
        limit,
    )?;
    Ok(json!({ "events": events, "joined": joined }))
}

fn send_hermes_inbound_payload(
    tx: &tokio::sync::mpsc::Sender<Result<Bytes, Infallible>>,
    payload: &Value,
) -> Result<bool, CliError> {
    let body = hermes_inbound_ndjson(payload)?;
    if body.is_empty() {
        return Ok(true);
    }
    Ok(tx.blocking_send(Ok(Bytes::from(body))).is_ok())
}

fn hermes_inbound_payload_has_records(payload: &Value) -> bool {
    payload
        .get("joined")
        .and_then(Value::as_array)
        .is_some_and(|joined| !joined.is_empty())
        || payload
            .get("events")
            .and_then(Value::as_array)
            .is_some_and(|events| !events.is_empty())
}

fn normalized_hermes_poll_limit(request: &PollRequest) -> usize {
    request.limit.unwrap_or(10).clamp(1, 32) as usize
}

fn normalized_hermes_poll_timeout(request: &PollRequest) -> Duration {
    Duration::from_millis(
        request
            .timeout_millis
            .unwrap_or(0)
            .min(MAX_HERMES_POLL_TIMEOUT_MILLIS),
    )
}

fn lock_service_mutex(mutex: &Mutex<()>) -> Result<std::sync::MutexGuard<'_, ()>, CliError> {
    mutex
        .lock()
        .map_err(|_| CliError::Hermes("Hermes service state lock poisoned".to_owned()))
}

struct BackupActivityGuard {
    path: PathBuf,
}

impl BackupActivityGuard {
    fn enter(home_dir: &Path, action: &str) -> Result<Self, CliError> {
        fs::create_dir_all(home_dir).map_err(|error| CliError::Hermes(error.to_string()))?;
        let path = home_dir.join(BACKUP_ACTIVITY_FILE);
        let marker = json!({
            "pid": std::process::id(),
            "action": action,
            "started_at_ms": now_ms(),
        });
        write_private(
            path.clone(),
            &serde_json::to_string(&marker).map_err(CliError::Serialize)?,
        )?;
        Ok(Self { path })
    }
}

impl Drop for BackupActivityGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn with_backup_activity<T>(
    home_dir: &Path,
    action: &str,
    f: impl FnOnce() -> Result<T, CliError>,
) -> Result<T, CliError> {
    let _guard = BackupActivityGuard::enter(home_dir, action)?;
    f()
}

fn hermes_inbound_ndjson(payload: &Value) -> Result<String, CliError> {
    let mut lines = String::new();
    if let Some(joined) = payload.get("joined").and_then(Value::as_array) {
        for account_id in joined {
            let record = json!({
                "type": "joined",
                "account_id": account_id,
            });
            lines.push_str(&serde_json::to_string(&record).map_err(CliError::Serialize)?);
            lines.push('\n');
        }
    }
    if let Some(events) = payload.get("events").and_then(Value::as_array) {
        for event in events {
            let record = json!({
                "type": "event",
                "event": event,
            });
            lines.push_str(&serde_json::to_string(&record).map_err(CliError::Serialize)?);
            lines.push('\n');
        }
    }
    Ok(lines)
}

fn status_for_cli_error(error: &CliError) -> StatusCode {
    match error {
        CliError::Usage(_) | CliError::Json(_) => StatusCode::BAD_REQUEST,
        CliError::Hermes(_) | CliError::Identity(_) => StatusCode::CONFLICT,
        CliError::Serialize(_)
        | CliError::Http(_)
        | CliError::Server { .. }
        | CliError::Output(_)
        | CliError::Runtime(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn service_cli_error(error: CliError) -> (StatusCode, Json<Value>) {
    let status = status_for_cli_error(&error);
    service_error(status, error.kind(), error.retryable(), error.to_string())
}

fn service_internal_error(error: String) -> (StatusCode, Json<Value>) {
    service_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", true, error)
}

fn service_error(
    status: StatusCode,
    error_kind: &'static str,
    retryable: bool,
    error: String,
) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({
            "ok": false,
            "status": "error",
            "service": "finitechat-hermes",
            "version": env!("CARGO_PKG_VERSION"),
            "http_status": status.as_u16(),
            "error_kind": error_kind,
            "retryable": retryable,
            "error": error,
        })),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HermesHomeChannel {
    room_id: String,
    #[serde(default)]
    conversation_id: Option<String>,
    set_at_ms: u64,
}

#[derive(Debug, Deserialize)]
struct HermesHomeChannelSetRequest {
    room_id: String,
    #[serde(default)]
    conversation_id: Option<String>,
}

fn cmd_home_channel<W: Write>(
    home_dir: &Path,
    command: HermesHomeChannelCommand,
    output: &mut W,
) -> Result<(), CliError> {
    match command {
        HermesHomeChannelCommand::Show => write_home_channel_show(home_dir, output),
        HermesHomeChannelCommand::Set(HermesHomeChannelSetArgs {
            room_id,
            conversation_id,
        }) => set_home_channel(home_dir, room_id, conversation_id, output),
        HermesHomeChannelCommand::Clear => {
            clear_home_channel(home_dir)?;
            crate::write_pretty_json(output, &json!({ "cleared": true, "home_channel": null }))
        }
    }
}

fn set_home_channel<W: Write>(
    home_dir: &Path,
    room_id: String,
    conversation_id: Option<String>,
    output: &mut W,
) -> Result<(), CliError> {
    let room_id = non_empty_home_channel_value("room_id", room_id)?;
    let conversation_id = conversation_id
        .map(|value| non_empty_home_channel_value("conversation_id", value))
        .transpose()?;
    ensure_agent_room_available(home_dir, &room_id)?;
    let channel = HermesHomeChannel {
        room_id,
        conversation_id,
        set_at_ms: now_ms(),
    };
    save_home_channel(home_dir, &channel)?;
    crate::write_pretty_json(output, &json!({ "home_channel": channel }))
}

fn write_home_channel_show<W: Write>(home_dir: &Path, output: &mut W) -> Result<(), CliError> {
    let channel = load_home_channel(home_dir)?;
    crate::write_pretty_json(output, &json!({ "home_channel": channel }))
}

fn non_empty_home_channel_value(name: &str, value: String) -> Result<String, CliError> {
    let trimmed = value.trim().to_owned();
    if trimmed.is_empty() {
        return Err(CliError::Hermes(format!("{name} cannot be empty")));
    }
    Ok(trimmed)
}

fn ensure_agent_room_available(home_dir: &Path, room_id: &str) -> Result<(), CliError> {
    let home = load_home(home_dir)?;
    let runtime = open_agent_runtime(&home)?;
    let state = runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(map_core_hermes_error)?;
    if state
        .rooms
        .iter()
        .any(|room| room.room_id == room_id && room.state == AppRoomState::Connected)
    {
        return Ok(());
    }
    Err(CliError::Hermes(format!(
        "home channel room {room_id} is not available to this agent"
    )))
}

fn load_home_channel(home_dir: &Path) -> Result<Option<HermesHomeChannel>, CliError> {
    let path = home_dir.join(HERMES_HOME_CHANNEL_FILE);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(CliError::Hermes(error.to_string())),
    };
    serde_json::from_str(&raw).map(Some).map_err(CliError::Json)
}

fn save_home_channel(home_dir: &Path, channel: &HermesHomeChannel) -> Result<(), CliError> {
    write_private(
        home_dir.join(HERMES_HOME_CHANNEL_FILE),
        &serde_json::to_string_pretty(channel).map_err(CliError::Serialize)?,
    )
}

fn clear_home_channel(home_dir: &Path) -> Result<(), CliError> {
    match fs::remove_file(home_dir.join(HERMES_HOME_CHANNEL_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliError::Hermes(error.to_string())),
    }
}

fn cmd_init<W: Write>(
    home_dir: &Path,
    args: HermesInitArgs,
    output: &mut W,
) -> Result<(), CliError> {
    let HermesInitArgs {
        server: server_url,
        device_id,
        agent_name,
        agent_about,
        agent_picture_url: agent_picture,
        skip_agent_profile,
    } = args;
    if home_dir.join(CONFIG_FILE).exists() {
        return Err(CliError::Hermes(format!(
            "agent home {} is already initialized",
            home_dir.display()
        )));
    }
    fs::create_dir_all(home_dir).map_err(|error| CliError::Hermes(error.to_string()))?;

    // Account-key acquisition per the Finite Identity Contract v1: load the
    // shared identity, minting it if this is the first Finite tool to run.
    // The secret stays in memory; nothing key-shaped is written to the home.
    let secret = load_or_generate_agent_secret()?;
    let runtime = FiniteChatRuntime::open(OpenOptions {
        data_dir: home_dir.to_string_lossy().into_owned(),
        server_url: server_url.clone(),
        device_id: device_id.clone(),
        account_secret_hex: Some(hex_lower(secret.as_bytes())),
        now_unix_seconds: Some(now_secs()),
    })
    .map_err(map_core_hermes_error)?;
    let state = runtime.state().map_err(map_core_hermes_error)?;

    let config = AgentConfig {
        server_url,
        device_id,
        account_id: state.identity.account_id,
    };
    write_private(
        home_dir.join(CONFIG_FILE),
        &serde_json::to_string_pretty(&config).map_err(CliError::Serialize)?,
    )?;

    let npub = npub_encode(&config.account_id)
        .map_err(|error| CliError::Hermes(format!("npub encoding failed: {error}")))?;
    let profile = if skip_agent_profile {
        None
    } else {
        Some(publish_agent_profile(
            &config,
            normalize_agent_profile_text("--agent-name", agent_name)?,
            normalize_agent_profile_text("--agent-about", agent_about)?,
            normalize_agent_profile_picture(agent_picture)?,
        )?)
    };
    crate::write_pretty_json(
        output,
        &json!({
            "home": home_dir.display().to_string(),
            "server_url": config.server_url,
            "device_id": config.device_id,
            "account_id": config.account_id,
            "npub": npub,
            "profile": profile,
        }),
    )
}

#[derive(Debug, Clone, Serialize)]
struct HermesAgentProfileSummary {
    account_id: String,
    display_name: String,
    about: String,
    picture: String,
    bot: bool,
    finite_role: String,
    saved: bool,
}

fn publish_agent_profile(
    config: &AgentConfig,
    display_name: String,
    about: String,
    picture: String,
) -> Result<HermesAgentProfileSummary, CliError> {
    let now = now_ms();
    let profile = NostrProfileRecord {
        account_id: config.account_id.clone(),
        name: Some(display_name.clone()),
        display_name: Some(display_name.clone()),
        about: Some(about.clone()),
        picture: Some(picture.clone()),
        bot: Some(true),
        finite_role: Some("agent".to_owned()),
        metadata_json: None,
        fetched_at_ms: now,
        expires_at_ms: now + CREDENTIAL_VALIDITY_SECONDS * 1000,
    };
    let mut delivery =
        HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(config.server_url.clone()));
    let response = delivery
        .put_nostr_profile(&profile)
        .map_err(|error| CliError::Hermes(format!("could not publish agent profile: {error}")))?;
    Ok(HermesAgentProfileSummary {
        account_id: profile.account_id,
        display_name,
        about,
        picture,
        bot: true,
        finite_role: "agent".to_owned(),
        saved: response.saved,
    })
}

fn normalize_agent_profile_text(name: &str, value: String) -> Result<String, CliError> {
    let trimmed = value.trim().to_owned();
    if trimmed.is_empty() {
        return Err(CliError::Usage(format!("{name} cannot be empty")));
    }
    Ok(trimmed)
}

fn normalize_agent_profile_picture(value: String) -> Result<String, CliError> {
    let trimmed = normalize_agent_profile_text("--agent-picture-url", value)?;
    let url = reqwest::Url::parse(&trimmed)
        .map_err(|error| CliError::Usage(format!("invalid --agent-picture-url: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(CliError::Usage(
            "--agent-picture-url must be an http(s) URL".to_owned(),
        ));
    }
    Ok(trimmed)
}

#[derive(Debug, Serialize)]
struct HermesRoomStatusSummary {
    room_id: String,
    state: String,
    status: String,
    connected: bool,
    paired: bool,
    member_count: u32,
    other_member_count: u32,
}

fn cmd_room_status<W: Write>(
    home_dir: &Path,
    args: HermesRoomStatusArgs,
    json_mode: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let room_id = args.room_id;

    let home = load_home(home_dir)?;
    // Read-only on purpose, and typed that way: a resident `hermes serve`
    // owns the store's writer lease, and a status read must never sync or
    // send. `ReadOnlyAgentStore` can only be built from the read-only open,
    // so this command cannot name the writer open — a one-shot StartRuntime
    // against the resident service's store is exactly the ratchet-divergence
    // incident this command used to cause.
    let store = ReadOnlyAgentStore::open(&home.dir, &home.secret, &home.config.device_id)?;
    let summary = read_only_room_status(&store, &home, &room_id)?;

    if json_mode {
        crate::write_pretty_json(output, &summary)
    } else {
        writeln!(
            output,
            "Room {}: {} (connected: {}, paired: {}, members: {})",
            summary.room_id,
            summary.status,
            summary.connected,
            summary.paired,
            summary.member_count,
        )
        .map_err(CliError::Output)
    }
}

/// How many recent stored messages `room-status` scans for counterparty
/// evidence; member counts are the primary pairing signal.
const ROOM_STATUS_MESSAGE_SCAN_LIMIT: u32 = 512;

fn read_only_room_status(
    store: &ReadOnlyAgentStore,
    home: &AgentHome,
    room_id: &str,
) -> Result<HermesRoomStatusSummary, CliError> {
    let config = device_config(&home.secret, &home.config.device_id, now_secs());
    let device = store
        .load_device(config)
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let owner = device.device_ref().clone();
    let has_mls_room = device.room_mls_group_id(room_id).is_ok();
    let stored_rooms = store
        .load_app_rooms(&owner)
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let stored_room = stored_rooms.iter().find(|room| room.room_id == room_id);
    let (state, status) = stored_room_state_and_status(stored_room, has_mls_room);
    let connected = state == "connected";
    let (member_count, other_member_count) = if connected {
        match device.room_members(room_id) {
            Ok(members) => {
                let other = members
                    .iter()
                    .filter(|member| member.device_id != owner.device_id)
                    .count();
                (members.len() as u32, other as u32)
            }
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    };
    let has_counterparty_messages = store
        .load_app_messages(&owner, ROOM_STATUS_MESSAGE_SCAN_LIMIT)
        .map_err(|error| CliError::Hermes(error.to_string()))?
        .iter()
        .any(|message| message.room_id == room_id && message.sender != owner);

    Ok(hermes_room_status_summary(
        room_id,
        state,
        status,
        member_count,
        other_member_count,
        has_counterparty_messages,
    ))
}

/// Mirror core's `app_room_from_stored` (finitechat-core): a stored
/// Connected/Unavailable room without local MLS state is unavailable on
/// this device, and a stored Unavailable room with MLS state has since
/// connected; a room known only to MLS materializes as connected.
fn stored_room_state_and_status(
    stored_room: Option<&StoredAppRoom>,
    has_mls_room: bool,
) -> (String, String) {
    match stored_room {
        None if has_mls_room => ("connected".to_owned(), "connected".to_owned()),
        None => ("unknown".to_owned(), "not_found".to_owned()),
        Some(room) => {
            let mut state = stored_app_room_state_label(room.state).to_owned();
            let mut status = room.status.clone();
            if matches!(
                room.state,
                StoredAppRoomState::Connected | StoredAppRoomState::UnavailableOnDevice
            ) && !has_mls_room
            {
                state = "unavailable_on_device".to_owned();
                status = "room is not available on this device".to_owned();
            } else if room.state == StoredAppRoomState::UnavailableOnDevice && has_mls_room {
                state = "connected".to_owned();
                status = "connected".to_owned();
            }
            (state, status)
        }
    }
}

fn hermes_room_status_summary(
    room_id: &str,
    state: String,
    status: String,
    member_count: u32,
    other_member_count: u32,
    has_counterparty_messages: bool,
) -> HermesRoomStatusSummary {
    let connected = state == "connected";
    HermesRoomStatusSummary {
        room_id: room_id.to_owned(),
        state,
        status,
        connected,
        paired: connected && (other_member_count > 0 || has_counterparty_messages),
        member_count,
        other_member_count,
    }
}

fn stored_app_room_state_label(state: StoredAppRoomState) -> &'static str {
    match state {
        StoredAppRoomState::Connected => "connected",
        StoredAppRoomState::WaitingForApproval => "waiting_for_approval",
        StoredAppRoomState::Joining => "joining",
        StoredAppRoomState::UnavailableOnDevice => "unavailable_on_device",
    }
}

#[derive(Debug, Deserialize)]
struct PollRequest {
    #[serde(default)]
    room_id: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    timeout_millis: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HermesInboxState {
    #[serde(default)]
    events: Vec<HermesInboxEvent>,
    #[serde(default)]
    cursors: BTreeMap<String, u64>,
    /// Bounded, oldest-first ring of recently-acked entry keys. It survives in
    /// `hermes-inbox.json`, so a duplicate ack after a restart is a no-op and
    /// an already-acked entry can never be re-enqueued from durable recovery.
    #[serde(default)]
    acked: Vec<HermesInboxAckedKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HermesInboxAckedKey {
    key: String,
    acked_at_ms: u64,
}

/// In-flight ownership of one inbox entry. The stream / `poll` / `inbound`
/// deliver only `Pending` entries and flip them to `Leased`; `ack` settles a
/// leased (or pending) entry, `release` returns it to `Pending`, and a lease
/// older than the TTL is swept back to `Pending` for redelivery. Serialized
/// with `#[serde(default)]` so an entry written before leases existed loads as
/// `Pending` — existing `hermes-inbox.json` files are unchanged on disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
enum HermesInboxLease {
    #[default]
    Pending,
    Leased {
        lease_id: String,
        leased_at_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HermesInboxEvent {
    key: String,
    room_id: String,
    seq: u64,
    message_id: String,
    created_at_ms: u64,
    event: HermesPollEventV1,
    #[serde(default)]
    lease: HermesInboxLease,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AgentdInboxState {
    #[serde(default)]
    events: Vec<AgentdInboxEvent>,
    #[serde(default)]
    cursors: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentdInboxEvent {
    key: String,
    room_id: String,
    seq: u64,
    message_id: String,
    delivery: RuntimeCommandDeliveryV1,
}

fn cmd_poll<W: Write>(home_dir: &Path, request: Value, output: &mut W) -> Result<(), CliError> {
    let request: PollRequest = serde_json::from_value(request).map_err(CliError::Json)?;
    let limit = request.limit.unwrap_or(10).clamp(1, 32) as usize;
    let timeout = Duration::from_millis(
        request
            .timeout_millis
            .unwrap_or(0)
            .min(MAX_HERMES_POLL_TIMEOUT_MILLIS),
    );
    let home = load_home(home_dir)?;
    let runtime = open_agent_runtime(&home)?;
    let started = Instant::now();
    let own_account = home.config.account_id.clone();
    let mut inbox = load_hermes_inbox(home_dir)?;
    initialize_hermes_inbox_cursors(home_dir, &home, &mut inbox)?;
    let mut events =
        lease_pending_hermes_inbox_events(home_dir, &mut inbox, request.room_id.as_deref(), limit)?;
    let mut joined: Vec<String> = Vec::new();

    while events.is_empty() {
        let bridge = runtime
            .agent_bridge_poll_once()
            .map_err(map_core_hermes_error)?;
        joined.extend(bridge.joined_account_ids);
        joined.sort();
        joined.dedup();

        for applied in &bridge.events {
            if let Some(room_filter) = &request.room_id
                && room_filter != &applied.room_id
            {
                continue;
            }
            if applied.sender_account_id == own_account {
                continue;
            }
            let context = HermesPollEventContext {
                home_dir,
                server_url: &home.config.server_url,
                room_id: &applied.room_id,
                seq: applied.seq,
                message_id: &applied.message_id,
                sender_account_id: &applied.sender_account_id,
                sender_device_id: &applied.sender_device_id,
                conversation_id: None,
                segment_id: None,
            };
            if let Some(event) =
                hermes_poll_event_from_application_plaintext(context, &applied.plaintext)?
            {
                enqueue_hermes_inbox_event(home_dir, &mut inbox, event)?;
            }
        }
        recover_stored_hermes_events(
            home_dir,
            &home,
            &own_account,
            request.room_id.as_deref(),
            &mut inbox,
        )?;
        events = lease_pending_hermes_inbox_events(
            home_dir,
            &mut inbox,
            request.room_id.as_deref(),
            limit,
        )?;

        if !events.is_empty() || !joined.is_empty() || started.elapsed() >= timeout {
            break;
        }
        let remaining = timeout.saturating_sub(started.elapsed()).as_millis() as u64;
        let (_store, device, mut delivery) = open_agent(&home)?;
        wait_for_hermes_sync_hint(&home, &mut delivery, &device, remaining);
    }

    crate::write_pretty_json(output, &json!({ "events": events, "joined": joined }))
}

fn cmd_ack<W: Write>(home_dir: &Path, request: Value, output: &mut W) -> Result<(), CliError> {
    let request: HermesAckRequestV1 = serde_json::from_value(request).map_err(CliError::Json)?;
    request
        .validate_limits()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let mut inbox = load_hermes_inbox(home_dir)?;
    let key = hermes_inbox_key(&request.room_id, request.seq, &request.message_id);
    let before = inbox.events.len();
    inbox.events.retain(|event| event.key != key);
    let removed = inbox.events.len() != before;
    // Idempotent: a duplicate ack (e.g. after a restart) for an entry already
    // settled finds it in the recently-acked ring and is still reported acked,
    // without a second inbox write.
    let previously_acked = inbox_key_recently_acked(&inbox, &key);
    if removed {
        record_hermes_inbox_acked(&mut inbox, &key, now_ms());
        save_hermes_inbox(home_dir, &inbox)?;
    }
    crate::write_pretty_json(
        output,
        &json!({ "acked": removed || previously_acked, "room_id": request.room_id, "seq": request.seq, "message_id": request.message_id }),
    )
}

fn cmd_release<W: Write>(home_dir: &Path, request: Value, output: &mut W) -> Result<(), CliError> {
    let request: HermesAckRequestV1 = serde_json::from_value(request).map_err(CliError::Json)?;
    request
        .validate_limits()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let mut inbox = load_hermes_inbox(home_dir)?;
    let key = hermes_inbox_key(&request.room_id, request.seq, &request.message_id);
    let released = release_hermes_inbox_entry(home_dir, &mut inbox, &key)?;
    crate::write_pretty_json(
        output,
        &json!({ "released": released, "room_id": request.room_id, "seq": request.seq, "message_id": request.message_id }),
    )
}

fn load_hermes_inbox(home_dir: &Path) -> Result<HermesInboxState, CliError> {
    let path = home_dir.join(HERMES_INBOX_FILE);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(CliError::Hermes(error.to_string())),
    };
    serde_json::from_str(&raw).map_err(CliError::Json)
}

fn save_hermes_inbox(home_dir: &Path, inbox: &HermesInboxState) -> Result<(), CliError> {
    write_private(
        home_dir.join(HERMES_INBOX_FILE),
        &serde_json::to_string_pretty(inbox).map_err(CliError::Serialize)?,
    )
}

fn load_agentd_inbox(home_dir: &Path) -> Result<AgentdInboxState, CliError> {
    let path = home_dir.join(AGENTD_INBOX_FILE);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Default::default());
        }
        Err(error) => return Err(CliError::Hermes(error.to_string())),
    };
    serde_json::from_str(&raw).map_err(CliError::Json)
}

fn save_agentd_inbox(home_dir: &Path, inbox: &AgentdInboxState) -> Result<(), CliError> {
    write_private(
        home_dir.join(AGENTD_INBOX_FILE),
        &serde_json::to_string_pretty(inbox).map_err(CliError::Serialize)?,
    )
}

fn recover_stored_agentd_events(
    home: &AgentHome,
    own_account: &str,
    room_filter: Option<&str>,
    inbox: &mut AgentdInboxState,
) -> Result<(), CliError> {
    let mut changed = false;
    for stored in load_recent_agent_app_events(home)? {
        if let Some(room_id) = room_filter
            && room_id != stored.room_id
        {
            continue;
        }
        if stored.seq <= agentd_inbox_cursor(inbox, &stored.room_id) {
            continue;
        }
        if stored.sender.account_id == own_account {
            changed |= advance_agentd_inbox_cursor(inbox, &stored.room_id, stored.seq);
            continue;
        }
        let delivery = runtime_command_delivery_from_stored(&stored);
        changed |= advance_agentd_inbox_cursor(inbox, &stored.room_id, stored.seq);
        let Some(delivery) = delivery else {
            continue;
        };
        let key = agentd_inbox_key(&stored.room_id, stored.seq, &stored.message_id);
        if inbox.events.iter().any(|event| event.key == key) {
            continue;
        }
        if inbox.events.len() >= MAX_RUNTIME_COMMAND_LEDGER_RECORDS as usize {
            return Err(CliError::Hermes(format!(
                "agentd inbox capacity exceeded: max {MAX_RUNTIME_COMMAND_LEDGER_RECORDS}"
            )));
        }
        inbox.events.push(AgentdInboxEvent {
            key,
            room_id: stored.room_id,
            seq: stored.seq,
            message_id: stored.message_id,
            delivery,
        });
        changed = true;
    }
    if changed {
        save_agentd_inbox(&home.dir, inbox)?;
    }
    Ok(())
}

fn runtime_command_delivery_from_stored(
    stored: &StoredAppEvent,
) -> Option<RuntimeCommandDeliveryV1> {
    let event = serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext).ok()?;
    if event.validate_limits().is_err() {
        return None;
    }
    let payload = match event.kind {
        DurableAppEventKind::RuntimeCommandRequest => {
            let request = serde_json::from_slice::<RuntimeCommandRequestV1>(&event.payload).ok()?;
            if request.validate_structure().is_err() {
                return None;
            }
            RuntimeCommandInboundPayloadV1::Request(request)
        }
        DurableAppEventKind::RuntimeCommandCancel => {
            let cancel = serde_json::from_slice::<RuntimeCommandCancelV1>(&event.payload).ok()?;
            if cancel.validate_structure().is_err() {
                return None;
            }
            RuntimeCommandInboundPayloadV1::Cancel(cancel)
        }
        _ => return None,
    };
    let delivery = RuntimeCommandDeliveryV1 {
        room_id: stored.room_id.clone(),
        conversation_id: event.conversation_id,
        seq: stored.seq,
        message_id: stored.message_id.clone(),
        sender: stored.sender.clone(),
        payload,
    };
    delivery.validate_structure().ok()?;
    Some(delivery)
}

fn agentd_inbox_cursor(inbox: &AgentdInboxState, room_id: &str) -> u64 {
    inbox.cursors.get(room_id).copied().unwrap_or(0)
}

fn advance_agentd_inbox_cursor(inbox: &mut AgentdInboxState, room_id: &str, seq: u64) -> bool {
    let cursor = inbox.cursors.entry(room_id.to_owned()).or_default();
    if seq <= *cursor {
        return false;
    }
    *cursor = seq;
    true
}

fn agentd_inbox_key(room_id: &str, seq: u64, message_id: &str) -> String {
    format!("{room_id}:{seq}:{message_id}")
}

fn enqueue_hermes_inbox_event(
    home_dir: &Path,
    inbox: &mut HermesInboxState,
    event: HermesPollEventV1,
) -> Result<(), CliError> {
    let key = hermes_inbox_key(&event.room_id, event.seq, &event.message_id);
    if inbox.events.iter().any(|existing| existing.key == key) {
        if advance_hermes_inbox_cursor(inbox, &event.room_id, event.seq) {
            save_hermes_inbox(home_dir, inbox)?;
        }
        return Ok(());
    }
    if event.seq <= hermes_inbox_cursor(inbox, &event.room_id) {
        return Ok(());
    }
    if inbox_key_recently_acked(inbox, &key) {
        // An already-acked entry must never be re-enqueued from durable
        // recovery, even if its seq is re-observed before the cursor caught up.
        if advance_hermes_inbox_cursor(inbox, &event.room_id, event.seq) {
            save_hermes_inbox(home_dir, inbox)?;
        }
        return Ok(());
    }
    advance_hermes_inbox_cursor(inbox, &event.room_id, event.seq);
    inbox.events.push(HermesInboxEvent {
        key,
        room_id: event.room_id.clone(),
        seq: event.seq,
        message_id: event.message_id.clone(),
        created_at_ms: now_ms(),
        event,
        lease: HermesInboxLease::Pending,
    });
    save_hermes_inbox(home_dir, inbox)
}

fn initialize_hermes_inbox_cursors(
    home_dir: &Path,
    home: &AgentHome,
    inbox: &mut HermesInboxState,
) -> Result<(), CliError> {
    if !inbox.cursors.is_empty() {
        return Ok(());
    }
    let recent_events = load_recent_agent_app_events(home)?;
    initialize_hermes_inbox_cursors_from_events(
        home_dir,
        inbox,
        &home.config.account_id,
        recent_events.iter(),
    )
}

fn initialize_hermes_inbox_cursors_from_events<'a>(
    home_dir: &Path,
    inbox: &mut HermesInboxState,
    own_account: &str,
    recent_events: impl IntoIterator<Item = &'a StoredAppEvent>,
) -> Result<(), CliError> {
    let recent_events = recent_events.into_iter().collect::<Vec<_>>();
    if !inbox.cursors.is_empty() {
        return Ok(());
    }
    let mut changed = false;
    let pending = inbox
        .events
        .iter()
        .map(|event| (event.room_id.clone(), event.seq))
        .collect::<Vec<_>>();
    for (room_id, seq) in pending {
        changed |= advance_hermes_inbox_cursor(inbox, &room_id, seq);
    }
    if !inbox.events.is_empty() {
        if changed {
            save_hermes_inbox(home_dir, inbox)?;
        }
        return Ok(());
    }

    let mut first_counterparty_seq_by_room = BTreeMap::<&str, u64>::new();
    for event in &recent_events {
        if event.sender.account_id != own_account {
            first_counterparty_seq_by_room
                .entry(event.room_id.as_str())
                .and_modify(|seq| *seq = (*seq).min(event.seq))
                .or_insert(event.seq);
        }
    }

    for event in recent_events {
        if event.sender.account_id == own_account
            && first_counterparty_seq_by_room
                .get(event.room_id.as_str())
                .is_none_or(|seq| event.seq < *seq)
        {
            changed |= advance_hermes_inbox_cursor(inbox, &event.room_id, event.seq);
        }
    }
    if changed {
        save_hermes_inbox(home_dir, inbox)?;
    }
    Ok(())
}

fn recover_stored_hermes_events(
    home_dir: &Path,
    home: &AgentHome,
    own_account: &str,
    room_filter: Option<&str>,
    inbox: &mut HermesInboxState,
) -> Result<(), CliError> {
    let recent_events = load_recent_agent_app_events(home)?;
    recover_stored_hermes_events_from_events(
        home_dir,
        home,
        own_account,
        room_filter,
        inbox,
        recent_events,
    )
}

fn recover_stored_hermes_events_from_events(
    home_dir: &Path,
    home: &AgentHome,
    own_account: &str,
    room_filter: Option<&str>,
    inbox: &mut HermesInboxState,
    recent_events: Vec<StoredAppEvent>,
) -> Result<(), CliError> {
    let mut cursor_changed = false;
    for stored in recent_events {
        if let Some(room_id) = room_filter
            && room_id != stored.room_id
        {
            continue;
        }
        if stored.sender.account_id == own_account {
            continue;
        }
        if stored.seq <= hermes_inbox_cursor(inbox, &stored.room_id) {
            continue;
        }
        let context = HermesPollEventContext {
            home_dir,
            server_url: &home.config.server_url,
            room_id: &stored.room_id,
            seq: stored.seq,
            message_id: &stored.message_id,
            sender_account_id: &stored.sender.account_id,
            sender_device_id: &stored.sender.device_id,
            conversation_id: None,
            segment_id: None,
        };
        match hermes_poll_event_from_application_plaintext(context, &stored.plaintext)? {
            Some(event) => enqueue_hermes_inbox_event(home_dir, inbox, event)?,
            None => {
                cursor_changed |= advance_hermes_inbox_cursor(inbox, &stored.room_id, stored.seq);
            }
        }
    }
    if cursor_changed {
        save_hermes_inbox(home_dir, inbox)?;
    }
    Ok(())
}

fn load_recent_agent_app_events(home: &AgentHome) -> Result<Vec<StoredAppEvent>, CliError> {
    let (store, device, _) = open_agent(home)?;
    store
        .load_app_events(device.device_ref(), HERMES_STORED_EVENT_RECOVERY_LIMIT)
        .map_err(|error| CliError::Hermes(error.to_string()))
}

fn hermes_inbox_cursor(inbox: &HermesInboxState, room_id: &str) -> u64 {
    inbox.cursors.get(room_id).copied().unwrap_or(0)
}

fn advance_hermes_inbox_cursor(inbox: &mut HermesInboxState, room_id: &str, seq: u64) -> bool {
    let cursor = inbox.cursors.entry(room_id.to_owned()).or_default();
    if seq <= *cursor {
        return false;
    }
    *cursor = seq;
    true
}

/// Non-mutating peek at the entries a delivery would hand out right now:
/// `Pending` entries plus any whose lease has already expired. Delivery itself
/// goes through `lease_pending_hermes_inbox_events`, which flips them to
/// `Leased` and persists; this peek only decides whether there is anything to
/// deliver (poll/stream idle checks) and backs the inbox unit tests.
#[cfg(test)]
fn pending_hermes_inbox_events(
    inbox: &HermesInboxState,
    room_filter: Option<&str>,
    limit: usize,
) -> Vec<HermesPollEventV1> {
    let now = now_ms();
    let ttl = hermes_inbox_lease_ttl_ms();
    inbox
        .events
        .iter()
        .filter(|entry| match room_filter {
            Some(room_id) => room_id == entry.room_id,
            None => true,
        })
        .filter(|entry| entry.lease.is_deliverable(now, ttl))
        .take(limit)
        .map(|entry| entry.event.clone())
        .collect()
}

/// Deliver up to `limit` deliverable entries, flipping each to `Leased` and
/// persisting the change so the next tick does not re-emit them. A leased
/// entry only comes back through `release`, `ack` (settles it), or the lease
/// TTL sweeping it back to `Pending`.
fn lease_pending_hermes_inbox_events(
    home_dir: &Path,
    inbox: &mut HermesInboxState,
    room_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<HermesPollEventV1>, CliError> {
    let now = now_ms();
    let ttl = hermes_inbox_lease_ttl_ms();
    let mut leased = Vec::new();
    let mut changed = false;
    for entry in inbox.events.iter_mut() {
        if leased.len() >= limit {
            break;
        }
        if let Some(room_id) = room_filter
            && room_id != entry.room_id
        {
            continue;
        }
        if !entry.lease.is_deliverable(now, ttl) {
            continue;
        }
        entry.lease = HermesInboxLease::Leased {
            lease_id: next_hermes_lease_id(now),
            leased_at_ms: now,
        };
        changed = true;
        leased.push(entry.event.clone());
    }
    if changed {
        save_hermes_inbox(home_dir, inbox)?;
    }
    Ok(leased)
}

impl HermesInboxLease {
    /// A `Pending` entry is deliverable; a `Leased` one becomes deliverable
    /// again only once its lease is at least `ttl_ms` old (a consumer that took
    /// it and neither acked nor released it, e.g. crashed mid-turn).
    fn is_deliverable(&self, now_ms: u64, ttl_ms: u64) -> bool {
        match self {
            HermesInboxLease::Pending => true,
            HermesInboxLease::Leased { leased_at_ms, .. } => {
                now_ms.saturating_sub(*leased_at_ms) >= ttl_ms
            }
        }
    }
}

fn hermes_inbox_lease_ttl_ms() -> u64 {
    std::env::var(HERMES_INBOX_LEASE_TTL_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_HERMES_INBOX_LEASE_TTL_MILLIS)
}

fn next_hermes_lease_id(now_ms: u64) -> String {
    let sequence = HERMES_LEASE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{now_ms}-{sequence}")
}

/// Return a leased entry to `Pending` so it is redelivered on the next tick.
/// A pending entry is left as-is; an unknown entry is a no-op. Returns whether
/// an entry was actually released.
fn release_hermes_inbox_entry(
    home_dir: &Path,
    inbox: &mut HermesInboxState,
    key: &str,
) -> Result<bool, CliError> {
    let mut released = false;
    for entry in inbox.events.iter_mut() {
        if entry.key == key && matches!(entry.lease, HermesInboxLease::Leased { .. }) {
            entry.lease = HermesInboxLease::Pending;
            released = true;
        }
    }
    if released {
        save_hermes_inbox(home_dir, inbox)?;
    }
    Ok(released)
}

fn inbox_key_recently_acked(inbox: &HermesInboxState, key: &str) -> bool {
    inbox.acked.iter().any(|acked| acked.key == key)
}

/// Record an acked entry key in the bounded, oldest-first ring so a duplicate
/// ack or a durable redelivery of the same entry is idempotent.
fn record_hermes_inbox_acked(inbox: &mut HermesInboxState, key: &str, now_ms: u64) {
    if inbox_key_recently_acked(inbox, key) {
        return;
    }
    inbox.acked.push(HermesInboxAckedKey {
        key: key.to_owned(),
        acked_at_ms: now_ms,
    });
    let overflow = inbox
        .acked
        .len()
        .saturating_sub(MAX_HERMES_INBOX_ACKED_KEYS);
    if overflow > 0 {
        inbox.acked.drain(0..overflow);
    }
}

fn hermes_inbox_key(room_id: &str, seq: u64, message_id: &str) -> String {
    format!("{room_id}\x1f{seq}\x1f{message_id}")
}

fn wait_for_hermes_sync_hint(
    home: &AgentHome,
    delivery: &mut AgentDelivery,
    device: &FiniteChatDevice,
    wait_ms: u64,
) {
    if wait_ms == 0 {
        return;
    }
    let cursors = device.room_sync_cursors();
    let (home_rooms, remote_rooms) = group_sync_wait_rooms(
        &home.config.server_url,
        cursors
            .into_iter()
            .map(|cursor| (cursor.room_id, cursor.after_seq, cursor.server_url)),
    );
    let wait_target_count = usize::from(!home_rooms.is_empty()) + remote_rooms.len();
    if wait_target_count == 0 {
        std::thread::sleep(Duration::from_millis(wait_ms.min(POLL_SLEEP_MS)));
        return;
    }
    let per_target_wait_ms = if wait_target_count == 1 {
        wait_ms
    } else {
        wait_ms.min(1_000)
    };
    let started = Instant::now();

    if !home_rooms.is_empty() {
        let target_wait_ms = bounded_remaining_wait_ms(wait_ms, per_target_wait_ms, started);
        let wait = SyncWaitRequest {
            rooms: home_rooms,
            wait_ms: target_wait_ms,
        };
        sync_wait_or_sleep(delivery, &wait, target_wait_ms);
    }
    for (server_url, rooms) in remote_rooms {
        let target_wait_ms = bounded_remaining_wait_ms(wait_ms, per_target_wait_ms, started);
        if target_wait_ms == 0 {
            break;
        }
        let mut room_delivery =
            HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url));
        let wait = SyncWaitRequest {
            rooms,
            wait_ms: target_wait_ms,
        };
        sync_wait_or_sleep(&mut room_delivery, &wait, target_wait_ms);
    }
}

fn bounded_remaining_wait_ms(wait_ms: u64, per_target_wait_ms: u64, started: Instant) -> u64 {
    let elapsed = started.elapsed().as_millis() as u64;
    wait_ms.saturating_sub(elapsed).min(per_target_wait_ms)
}

fn sync_wait_or_sleep(delivery: &mut AgentDelivery, wait: &SyncWaitRequest, fallback_wait_ms: u64) {
    if delivery.sync_wait(wait).is_err() {
        std::thread::sleep(Duration::from_millis(fallback_wait_ms.min(POLL_SLEEP_MS)));
    }
}

fn group_sync_wait_rooms<I>(
    home_server_url: &str,
    cursors: I,
) -> (Vec<SyncWaitRoom>, BTreeMap<String, Vec<SyncWaitRoom>>)
where
    I: IntoIterator<Item = (String, u64, Option<String>)>,
{
    let mut home_rooms = Vec::new();
    let mut remote_rooms: BTreeMap<String, Vec<SyncWaitRoom>> = BTreeMap::new();
    for (room_id, after_seq, server_url) in cursors {
        let room = SyncWaitRoom { room_id, after_seq };
        match server_url {
            Some(server_url) if server_url != home_server_url => {
                remote_rooms.entry(server_url).or_default().push(room);
            }
            Some(_) | None => home_rooms.push(room),
        }
    }
    (home_rooms, remote_rooms)
}

#[derive(Clone, Copy)]
struct HermesPollEventContext<'a> {
    home_dir: &'a Path,
    server_url: &'a str,
    room_id: &'a str,
    seq: u64,
    message_id: &'a str,
    sender_account_id: &'a str,
    sender_device_id: &'a str,
    conversation_id: Option<&'a str>,
    segment_id: Option<&'a str>,
}

fn hermes_poll_event_from_application_plaintext(
    context: HermesPollEventContext<'_>,
    plaintext: &[u8],
) -> Result<Option<HermesPollEventV1>, CliError> {
    if let Ok(event) = serde_json::from_slice::<DecryptedApplicationEventV1>(plaintext) {
        if event.validate_limits().is_err() {
            return Ok(None);
        }
        return match event.kind {
            DurableAppEventKind::ChatMessage => {
                let context = HermesPollEventContext {
                    conversation_id: event.conversation_id.as_deref(),
                    segment_id: event.segment_id.as_deref(),
                    ..context
                };
                hermes_poll_event_from_chat_payload(context, &event.payload, true)
            }
            DurableAppEventKind::ConversationCreate
            | DurableAppEventKind::ConversationUpdate
            | DurableAppEventKind::ConversationArchive
            | DurableAppEventKind::ConversationSegmentStart
            | DurableAppEventKind::ChatEdit
            | DurableAppEventKind::ChatReaction
            | DurableAppEventKind::ChatReceipt
            | DurableAppEventKind::RuntimeStateSnapshot
            | DurableAppEventKind::RuntimeCommandRequest
            | DurableAppEventKind::RuntimeCommandResult
            | DurableAppEventKind::RuntimeCommandCancel
            | DurableAppEventKind::StreamStart
            | DurableAppEventKind::StreamFinish
            | DurableAppEventKind::Namespaced { .. } => Ok(None),
        };
    }

    hermes_poll_event_from_chat_payload(context, plaintext, false)
}

fn hermes_poll_event_from_chat_payload(
    context: HermesPollEventContext<'_>,
    payload: &[u8],
    typed_chat_message: bool,
) -> Result<Option<HermesPollEventV1>, CliError> {
    // Single JSON pass over the payload: parse once, dispatch on the "type"
    // tag, and convert only the hermes form. Twin of the chat projection's
    // resolve_chat_message_payload; the payload previously got one full Value
    // parse per probe (the hermes decode, then the typed-JSON sniff).
    let value = serde_json::from_slice::<Value>(payload).ok();
    let payload_type = value
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let decoded = match value {
        Some(value) if payload_type.as_deref() == Some(HERMES_MESSAGE_PAYLOAD_TYPE_V1) => {
            HermesMessagePayloadV1::decode_value(value)
                .map_err(|error| CliError::Hermes(error.to_string()))?
        }
        _ => None,
    };
    if let Some(payload) = decoded {
        let mut event = payload.into_poll_event(
            context.room_id.to_owned(),
            context.seq,
            context.message_id.to_owned(),
            context.sender_account_id.to_owned(),
            context.sender_device_id.to_owned(),
        );
        if event.conversation_id.is_none() {
            event.conversation_id = context.conversation_id.map(ToOwned::to_owned);
            event.source.thread_id = event.conversation_id.clone();
        }
        if event.segment_id.is_none() {
            event.segment_id = context.segment_id.map(ToOwned::to_owned);
        }
        if event.segment_id.is_some() {
            event.source.thread_id = event.segment_id.clone();
        }
        materialize_poll_event_attachments(context.home_dir, context.server_url, &mut event)?;
        return Ok(Some(event));
    }

    if typed_chat_message && payload_type.is_some() {
        return Ok(None);
    }

    let Ok(text) = std::str::from_utf8(payload) else {
        return Ok(None);
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let mut event = HermesPollEventV1::finite_chat_text(
        context.room_id.to_owned(),
        context.seq,
        context.message_id.to_owned(),
        context.sender_account_id.to_owned(),
        context.sender_device_id.to_owned(),
        text.to_owned(),
    )
    .map_err(|error| CliError::Hermes(error.to_string()))?;
    event.conversation_id = context.conversation_id.map(ToOwned::to_owned);
    event.segment_id = context.segment_id.map(ToOwned::to_owned);
    event.source.thread_id = event.segment_id.clone().or(event.conversation_id.clone());
    event
        .validate_limits()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    Ok(Some(event))
}

fn materialize_poll_event_attachments(
    home_dir: &Path,
    server_url: &str,
    event: &mut HermesPollEventV1,
) -> Result<(), CliError> {
    let mut unavailable = false;
    for attachment in &mut event.attachments {
        if attachment.path.is_some() {
            continue;
        }
        let Some(reference) = attachment.blob.clone() else {
            continue;
        };
        match materialize_blob_attachment(home_dir, server_url, &reference) {
            Ok(path) => attachment.path = Some(path.to_string_lossy().into_owned()),
            Err(AttachmentMaterializationError::Retryable(error)) => {
                // Recovery owns the cursor until the verified local media is
                // ready. Returning before enqueue means the resident stream
                // reconnects and retries this same durable event; it is never
                // exposed to Hermes or acked while transport is unhealthy.
                return Err(CliError::Hermes(format!(
                    "attachment is temporarily unavailable: {error}"
                )));
            }
            Err(AttachmentMaterializationError::Permanent(_)) => {
                // A deterministically missing or corrupt blob must not poison
                // this room's durable Hermes cursor. Keep the encrypted
                // reference in the raw event so the chat UI can still retry or
                // resend it, but never expose ciphertext as model media.
                attachment.path = None;
                attachment.url = None;
                unavailable = true;
            }
        }
    }

    if unavailable {
        add_attachment_unavailable_notice(event);
        if event
            .attachments
            .iter()
            .all(|attachment| attachment.path.is_none() && attachment.url.is_none())
        {
            event.message_type = HermesMessageTypeV1::Text;
        }
        event
            .validate_limits()
            .map_err(|error| CliError::Hermes(error.to_string()))?;
    }
    Ok(())
}

fn add_attachment_unavailable_notice(event: &mut HermesPollEventV1) {
    let separator = if event.text.trim().is_empty() {
        ""
    } else {
        "\n\n"
    };
    let notice = format!("{separator}{HERMES_ATTACHMENT_UNAVAILABLE_NOTICE}");
    if event.text.len() + notice.len() <= MAX_HERMES_TEXT_BYTES as usize {
        event.text.push_str(&notice);
        return;
    }

    // Preserve a maximum-sized user caption byte-for-byte. If an existing
    // channel prompt has room, retain it too; otherwise the short recovery
    // instruction wins so the model never mistakes missing media for success.
    let existing = event.channel_prompt.take().unwrap_or_default();
    let separator = if existing.trim().is_empty() {
        ""
    } else {
        "\n\n"
    };
    let combined = format!("{existing}{separator}{HERMES_ATTACHMENT_UNAVAILABLE_NOTICE}");
    event.channel_prompt = Some(if combined.len() <= MAX_HERMES_METADATA_BYTES as usize {
        combined
    } else {
        HERMES_ATTACHMENT_UNAVAILABLE_NOTICE.to_owned()
    });
}

#[derive(Debug, PartialEq, Eq)]
enum AttachmentMaterializationError {
    Retryable(String),
    Permanent(String),
}

fn materialize_blob_attachment(
    home_dir: &Path,
    server_url: &str,
    reference: &AttachmentBlobReferenceV1,
) -> Result<PathBuf, AttachmentMaterializationError> {
    let path = hermes_attachment_cache_path(home_dir, reference);
    if let Ok(existing) = fs::read(&path)
        && existing.len() as u64 == reference.plaintext_size
        && sha256_hex(&existing) == reference.plaintext_sha256
    {
        return Ok(path);
    }

    let request = prepare_blossom_download_http_request(reference).map_err(|error| {
        AttachmentMaterializationError::Permanent(format!(
            "attachment reference is invalid: {error}"
        ))
    })?;
    let download_url = hermes_attachment_download_url(request.url, server_url, reference)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(HERMES_ATTACHMENT_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(HERMES_ATTACHMENT_DOWNLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|error| {
            AttachmentMaterializationError::Retryable(format!("attachment client failed: {error}"))
        })?;
    let response = client.get(download_url).send().map_err(|error| {
        AttachmentMaterializationError::Retryable(format!("attachment download failed: {error}"))
    })?;
    let status = response.status().as_u16();
    if !(200..=299).contains(&status) {
        return Err(classify_attachment_http_error(status));
    }
    let body = response
        .bytes()
        .map_err(|error| {
            AttachmentMaterializationError::Retryable(format!(
                "attachment download failed: {error}"
            ))
        })?
        .to_vec();
    let downloaded = finish_blossom_download_http_response(
        reference,
        BlossomDownloadHttpResponse {
            status,
            body: &body,
        },
    )
    .map_err(classify_attachment_verification_error)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AttachmentMaterializationError::Retryable(format!(
                "attachment cache directory failed: {error}"
            ))
        })?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &downloaded.plaintext).map_err(|error| {
        AttachmentMaterializationError::Retryable(format!("attachment cache write failed: {error}"))
    })?;
    fs::rename(&tmp, &path).map_err(|error| {
        AttachmentMaterializationError::Retryable(format!(
            "attachment cache commit failed: {error}"
        ))
    })?;
    Ok(path)
}

fn classify_attachment_http_error(status: u16) -> AttachmentMaterializationError {
    let error = format!("attachment server returned HTTP {status}");
    if status == StatusCode::REQUEST_TIMEOUT.as_u16()
        || status == StatusCode::TOO_EARLY.as_u16()
        || status == StatusCode::TOO_MANY_REQUESTS.as_u16()
        || !(400..=499).contains(&status)
    {
        // Redirects without a usable target, throttles, request timeouts, and
        // server failures may heal without consuming the encrypted message.
        AttachmentMaterializationError::Retryable(error)
    } else {
        // Authentication, authorization, missing-object, and other stable
        // client failures cannot heal by replaying the same durable event.
        AttachmentMaterializationError::Permanent(error)
    }
}

fn classify_attachment_verification_error(
    error: AttachmentBlobError,
) -> AttachmentMaterializationError {
    let retryable = matches!(
        &error,
        AttachmentBlobError::Protocol(ProtocolLimitError::BytesEmpty { field })
            if field == "attachment.ciphertext"
    ) || matches!(
        &error,
        AttachmentBlobError::CiphertextSizeMismatch { expected, actual }
            if actual < expected
    ) || matches!(&error, AttachmentBlobError::HttpStatus { .. });
    let message = format!("attachment verification failed: {error}");
    if retryable {
        AttachmentMaterializationError::Retryable(message)
    } else {
        // Exact-size ciphertext hash failures, AEAD failures, plaintext
        // verification failures, and invalid references are deterministic for
        // this encrypted message and cannot heal by replaying the same bytes.
        AttachmentMaterializationError::Permanent(message)
    }
}

fn hermes_attachment_cache_path(home_dir: &Path, reference: &AttachmentBlobReferenceV1) -> PathBuf {
    home_dir
        .join(ATTACHMENT_CACHE_DIR)
        .join(&reference.plaintext_sha256)
        .join(sanitized_attachment_filename(&reference.metadata.filename))
}

/// Older Hosted Web Devices uploaded through the server's loopback address,
/// which caused that process-local origin to be written into the encrypted
/// attachment reference. A different device (including a Kata guest) must use
/// its trusted Finite Chat server origin for the same content-addressed blob.
/// The ciphertext hash and AEAD verification still authenticate the bytes.
fn hermes_attachment_download_url(
    reference_url: &str,
    server_url: &str,
    reference: &AttachmentBlobReferenceV1,
) -> Result<String, AttachmentMaterializationError> {
    let reference_url = reqwest::Url::parse(reference_url).map_err(|error| {
        AttachmentMaterializationError::Permanent(format!("attachment URL is invalid: {error}"))
    })?;
    if !matches!(reference_url.scheme(), "http" | "https") {
        return Err(AttachmentMaterializationError::Permanent(format!(
            "attachment URL scheme {:?} is not supported",
            reference_url.scheme()
        )));
    }
    let expected_path = format!("/blobs/{}", reference.ciphertext_sha256);
    if !url_host_is_loopback(&reference_url) || reference_url.path() != expected_path {
        return Ok(reference_url.to_string());
    }

    let mut canonical = reqwest::Url::parse(server_url).map_err(|error| {
        AttachmentMaterializationError::Retryable(format!(
            "Finite Chat server URL is invalid: {error}"
        ))
    })?;
    if !matches!(canonical.scheme(), "http" | "https") || canonical.host_str().is_none() {
        return Err(AttachmentMaterializationError::Retryable(
            "Finite Chat server is not an HTTP origin".to_owned(),
        ));
    }
    canonical.set_path(&expected_path);
    canonical.set_query(None);
    canonical.set_fragment(None);
    Ok(canonical.to_string())
}

fn url_host_is_loopback(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn sanitized_attachment_filename(filename: &str) -> String {
    let leaf = Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment")
        .trim();
    let sanitized = leaf
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "attachment".to_owned()
    } else {
        sanitized
    }
}

fn encode_application_event(
    kind: DurableAppEventKind,
    conversation_id: Option<String>,
    segment_id: Option<String>,
    payload: &[u8],
) -> Result<Vec<u8>, CliError> {
    let event = DecryptedApplicationEventV1 {
        kind,
        conversation_id,
        segment_id,
        payload: payload.to_vec(),
    };
    event
        .validate_limits()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    serde_json::to_vec(&event).map_err(CliError::Serialize)
}

fn cmd_send<W: Write>(home_dir: &Path, request: Value, output: &mut W) -> Result<(), CliError> {
    let mut request: HermesSendRequestV1 =
        serde_json::from_value(request).map_err(CliError::Json)?;
    let home = load_home(home_dir)?;
    let runtime = open_agent_runtime(&home)?;
    runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(map_core_hermes_error)?;
    resolve_hermes_send_route(&runtime, &mut request)?;
    let sent = send_hermes_request_with_runtime(&runtime, &request)?;
    update_running_after_send(home_dir, &request, &sent.message_id)?;
    write_sent_message(output, &sent)
}

fn cmd_edit<W: Write>(home_dir: &Path, request: Value, output: &mut W) -> Result<(), CliError> {
    let mut request: HermesEditRequestV1 =
        serde_json::from_value(request).map_err(CliError::Json)?;
    let home = load_home(home_dir)?;
    let runtime = open_agent_runtime(&home)?;
    runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(map_core_hermes_error)?;
    resolve_hermes_edit_route(home_dir, &runtime, &mut request)?;
    let sent = edit_hermes_request_with_runtime(&runtime, &request)?;
    update_running_after_edit(home_dir, &request)?;
    write_sent_message(output, &sent)
}

fn cmd_recover<W: Write>(home_dir: &Path, _request: Value, output: &mut W) -> Result<(), CliError> {
    let running = load_hermes_running(home_dir)?;
    let mut recovered = 0usize;
    for message in &running.messages {
        let recovery = HermesEditRequestV1 {
            room_id: message.room_id.clone(),
            conversation_id: message.conversation_id.clone(),
            segment_id: message.segment_id.clone(),
            thread_id: None,
            message_id: message.message_id.clone(),
            text: "Hermes gateway restarted before this turn completed.".to_owned(),
            kind: message.kind,
            status: HermesMessageStatusV1::Complete,
            finalize: true,
            metadata: BTreeMap::new(),
        };
        let hermes_payload = HermesMessagePayloadV1::from_edit(&recovery)
            .encode()
            .map_err(|error| CliError::Hermes(error.to_string()))?;
        let app_payload = encode_application_event(
            DurableAppEventKind::ChatMessage,
            recovery.conversation_id.clone(),
            recovery.segment_id.clone(),
            &hermes_payload,
        )?;
        let home = load_home(home_dir)?;
        let runtime = open_agent_runtime(&home)?;
        runtime
            .dispatch_and_wait(AppAction::StartRuntime)
            .map_err(map_core_hermes_error)?;
        append_payload_to_room_with_runtime(
            &runtime,
            &recovery.room_id,
            app_payload,
            recovery.text.clone(),
        )?;
        recovered += 1;
    }
    if recovered > 0 {
        save_hermes_running(home_dir, &HermesRunningState::default())?;
    }
    crate::write_pretty_json(output, &json!({ "recovered": recovered }))
}

/// What a send/edit/activity does when its `thread_id` matches nothing in the
/// agent store; selected by [`HERMES_UNKNOWN_THREAD_ROUTE_ENV`].
enum UnknownThreadRoutePolicy {
    /// Fail closed: a typed non-retryable error (HTTP 409 from the resident
    /// service, `error_kind: "hermes"`, `retryable: false`).
    Error,
    /// Deliver anyway: clear the route fields so Core applies its Home
    /// default, after logging a loud routing warning.
    HomeDefault,
}

fn unknown_thread_route_policy() -> UnknownThreadRoutePolicy {
    match std::env::var(HERMES_UNKNOWN_THREAD_ROUTE_ENV)
        .ok()
        .as_deref()
        .map(str::trim)
    {
        // Unset is the deliverable-by-default product behavior; only an
        // explicitly set value selects strict (so `=error` opts in and any
        // unrecognized value fails closed rather than rerouting silently).
        None => UnknownThreadRoutePolicy::HomeDefault,
        Some("home") | Some("default") => UnknownThreadRoutePolicy::HomeDefault,
        Some(_) => UnknownThreadRoutePolicy::Error,
    }
}

/// Resolve a Hermes `thread_id` against the agent store: it is either a
/// conversation (topic) id, or a segment (chat) id inside one of the room's
/// topics. Returns `(conversation_id, segment_id)`. `hermes.rs`'s inbound path
/// mints `thread_id = segment_id.or(conversation_id)`, so this is the inverse
/// the Python adapter used to persist in its own route table.
fn resolve_thread_id_route(
    topics: &[AppTopicSummary],
    room_id: &str,
    thread_id: &str,
) -> Option<(String, Option<String>)> {
    if topics
        .iter()
        .any(|topic| topic.room_id == room_id && topic.topic_id == thread_id && !topic.archived)
    {
        return Some((thread_id.to_owned(), None));
    }
    for topic in topics
        .iter()
        .filter(|topic| topic.room_id == room_id && !topic.archived)
    {
        if topic.chats.iter().any(|chat| chat.chat_id == thread_id) {
            return Some((topic.topic_id.clone(), Some(thread_id.to_owned())));
        }
    }
    None
}

/// Turn the request's optional `thread_id` into concrete route fields. An
/// explicit `conversation_id`/`segment_id` always wins (thread_id is only a
/// resolver); with neither route nor thread_id the send is an intentional Home
/// message (Core applies its default). An unknown thread_id falls back to the
/// Home default with a loud warning by default so replies stay deliverable;
/// setting `FINITECHAT_HERMES_UNKNOWN_THREAD_ROUTE` to `error` restores the
/// strict typed failure.
fn resolve_route_fields(
    runtime: &FiniteChatRuntime,
    room_id: &str,
    conversation_id: Option<String>,
    segment_id: Option<String>,
    thread_id: Option<&str>,
    context: &str,
) -> Result<(Option<String>, Option<String>), CliError> {
    if conversation_id.is_some() || segment_id.is_some() {
        return Ok((conversation_id, segment_id));
    }
    let Some(thread_id) = thread_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok((None, None));
    };
    let state = runtime.state().map_err(map_core_hermes_error)?;
    if let Some((conversation_id, segment_id)) =
        resolve_thread_id_route(&state.topics, room_id, thread_id)
    {
        return Ok((Some(conversation_id), segment_id));
    }
    match unknown_thread_route_policy() {
        UnknownThreadRoutePolicy::Error => Err(CliError::Hermes(format!(
            "unknown thread_id {thread_id:?} for room {room_id} ({context}): \
             no matching conversation or segment in the agent store"
        ))),
        UnknownThreadRoutePolicy::HomeDefault => {
            eprintln!(
                "[finitechat] hermes {context}: unknown thread_id {thread_id:?} for room \
                 {room_id}; falling back to the Home default so this reply stays deliverable \
                 ({HERMES_UNKNOWN_THREAD_ROUTE_ENV}=error restores the strict typed error)"
            );
            Ok((None, None))
        }
    }
}

fn resolve_hermes_send_route(
    runtime: &FiniteChatRuntime,
    request: &mut HermesSendRequestV1,
) -> Result<(), CliError> {
    let (conversation_id, segment_id) = resolve_route_fields(
        runtime,
        &request.room_id,
        request.conversation_id.take(),
        request.segment_id.take(),
        request.thread_id.as_deref(),
        "send",
    )?;
    request.conversation_id = conversation_id;
    request.segment_id = segment_id;
    Ok(())
}

fn resolve_hermes_activity_route(
    runtime: &FiniteChatRuntime,
    request: &mut HermesActivityRequestV1,
) -> Result<(), CliError> {
    let (conversation_id, segment_id) = resolve_route_fields(
        runtime,
        &request.room_id,
        request.conversation_id.take(),
        request.segment_id.take(),
        request.thread_id.as_deref(),
        "activity",
    )?;
    request.conversation_id = conversation_id;
    request.segment_id = segment_id;
    Ok(())
}

fn lookup_running_message(
    home_dir: &Path,
    room_id: &str,
    message_id: &str,
) -> Result<Option<HermesRunningMessage>, CliError> {
    let running = load_hermes_running(home_dir)?;
    Ok(running
        .messages
        .into_iter()
        .find(|message| message.room_id == room_id && message.message_id == message_id))
}

/// Resolve an edit's route. An explicit route wins; otherwise a `thread_id` is
/// resolved against the store; otherwise the original message's route is looked
/// up by `(room_id, message_id)` from the running-turn file, so the adapter no
/// longer needs to remember outbound message routes.
fn resolve_hermes_edit_route(
    home_dir: &Path,
    runtime: &FiniteChatRuntime,
    request: &mut HermesEditRequestV1,
) -> Result<(), CliError> {
    if request.conversation_id.is_some() || request.segment_id.is_some() {
        // A route override must name a real conversation, so a wrong override
        // is a typed error rather than a silent misroute to a phantom topic.
        if let Some(conversation_id) = request.conversation_id.as_deref() {
            let state = runtime.state().map_err(map_core_hermes_error)?;
            let exists = state.topics.iter().any(|topic| {
                topic.room_id == request.room_id
                    && topic.topic_id == conversation_id
                    && !topic.archived
            });
            if !exists {
                return Err(CliError::Hermes(format!(
                    "edit conversation_id {conversation_id:?} override does not name a \
                     conversation in room {}",
                    request.room_id
                )));
            }
        }
        return Ok(());
    }
    if request
        .thread_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        let (conversation_id, segment_id) = resolve_route_fields(
            runtime,
            &request.room_id,
            None,
            None,
            request.thread_id.as_deref(),
            "edit",
        )?;
        request.conversation_id = conversation_id;
        request.segment_id = segment_id;
        return Ok(());
    }
    if let Some(message) = lookup_running_message(home_dir, &request.room_id, &request.message_id)?
    {
        // The running-turn file is authoritative for an in-flight edit's route
        // and kind, so the adapter no longer remembers outbound message routes
        // or kinds (ownership audit O2).
        request.conversation_id = message.conversation_id;
        request.segment_id = message.segment_id;
        request.kind = message.kind;
    }
    Ok(())
}

fn send_hermes_request_with_runtime(
    runtime: &FiniteChatRuntime,
    request: &HermesSendRequestV1,
) -> Result<AppSentMessage, CliError> {
    let request = prepare_hermes_send_attachments(runtime, request)?;
    let hermes_payload = HermesMessagePayloadV1::from_send(&request)
        .encode()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let app_payload = encode_application_event(
        DurableAppEventKind::ChatMessage,
        request.conversation_id.clone(),
        request.segment_id.clone(),
        &hermes_payload,
    )?;
    append_payload_to_room_with_runtime(
        runtime,
        &request.room_id,
        app_payload,
        request.text.clone(),
    )
}

struct LoadedHermesAttachment {
    index: usize,
    outbound: OutboundAttachment,
}

/// Resolve every raw local path before uploading anything. A missing,
/// unreadable, empty, or oversized file therefore cannot append a broken
/// media message (and cannot leave earlier files from the same request
/// partially uploaded just because a later path was invalid).
fn prepare_hermes_send_attachments(
    runtime: &FiniteChatRuntime,
    request: &HermesSendRequestV1,
) -> Result<HermesSendRequestV1, CliError> {
    request
        .validate_limits()
        .map_err(|error| CliError::Hermes(error.to_string()))?;

    let mut prepared = request.clone();
    let mut loaded = Vec::new();
    let mut total_bytes = 0usize;
    for (index, attachment) in request.attachments.iter().enumerate() {
        if let Some(blob) = &attachment.blob {
            // Inbound bridge events materialize verified blobs for Hermes and
            // therefore carry both `path` and `blob`. If Hermes sends that
            // attachment back, retain only the durable reference; never leak
            // an agent-local cache path into the E2EE room log.
            prepared.attachments[index].path = None;
            prepared.attachments[index].url = Some(blob.url.clone());
            continue;
        }
        let Some(path) = attachment.path.as_deref() else {
            continue;
        };
        let bytes = read_bounded_hermes_attachment(path, total_bytes)?;
        total_bytes = total_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| CliError::Hermes("local attachment byte total overflowed".to_owned()))?;
        loaded.push(LoadedHermesAttachment {
            index,
            outbound: OutboundAttachment {
                filename: attachment.name.clone(),
                mime_type: attachment.mime_type.clone(),
                kind: match attachment.kind {
                    finitechat_hermes::HermesAttachmentKindV1::Image => ChatMediaKind::Image,
                    finitechat_hermes::HermesAttachmentKindV1::Video => ChatMediaKind::Video,
                    finitechat_hermes::HermesAttachmentKindV1::Audio => ChatMediaKind::VoiceNote,
                    finitechat_hermes::HermesAttachmentKindV1::File => ChatMediaKind::File,
                },
                bytes,
            },
        });
    }

    for attachment in loaded {
        prepared.attachments[attachment.index] = runtime
            .upload_bridge_attachment_and_wait(request.room_id.clone(), attachment.outbound)
            .map_err(map_core_hermes_error)?;
    }
    prepared
        .validate_limits()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    Ok(prepared)
}

fn read_bounded_hermes_attachment(path: &str, prior_bytes: usize) -> Result<Vec<u8>, CliError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        return Err(CliError::Hermes(
            "local attachment path must not be empty".to_owned(),
        ));
    }
    let mut file = fs::File::open(path).map_err(|error| {
        CliError::Hermes(format!(
            "could not open local attachment {}: {error}",
            path.display()
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        CliError::Hermes(format!(
            "could not inspect local attachment {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(CliError::Hermes(format!(
            "local attachment {} is not a regular file",
            path.display()
        )));
    }
    if metadata.len() == 0 {
        return Err(CliError::Hermes(format!(
            "local attachment {} is empty",
            path.display()
        )));
    }
    if metadata.len() > u64::from(MAX_ATTACHMENT_PLAINTEXT_BYTES) {
        return Err(CliError::Hermes(format!(
            "local attachment {} exceeds the {} byte limit",
            path.display(),
            MAX_ATTACHMENT_PLAINTEXT_BYTES
        )));
    }
    let remaining_total = MAX_HERMES_LOCAL_ATTACHMENT_TOTAL_BYTES
        .checked_sub(prior_bytes)
        .ok_or_else(|| CliError::Hermes("local attachment batch is too large".to_owned()))?;
    if metadata.len() > remaining_total as u64 {
        return Err(CliError::Hermes(format!(
            "local attachments exceed the {} byte batch limit",
            MAX_HERMES_LOCAL_ATTACHMENT_TOTAL_BYTES
        )));
    }

    let expected_len = usize::try_from(metadata.len())
        .map_err(|_| CliError::Hermes("local attachment is too large for this host".to_owned()))?;
    let allowed = remaining_total.min(MAX_ATTACHMENT_PLAINTEXT_BYTES as usize);
    let mut bytes = Vec::with_capacity(expected_len.min(allowed));
    Read::take(&mut file, (allowed as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::Hermes(format!(
                "could not read local attachment {}: {error}",
                path.display()
            ))
        })?;
    if bytes.len() > MAX_ATTACHMENT_PLAINTEXT_BYTES as usize {
        return Err(CliError::Hermes(format!(
            "local attachment {} exceeds the {} byte limit",
            path.display(),
            MAX_ATTACHMENT_PLAINTEXT_BYTES
        )));
    }
    if bytes.len() > remaining_total {
        return Err(CliError::Hermes(format!(
            "local attachments exceed the {} byte batch limit",
            MAX_HERMES_LOCAL_ATTACHMENT_TOTAL_BYTES
        )));
    }
    if bytes.is_empty() {
        return Err(CliError::Hermes(format!(
            "local attachment {} is empty",
            path.display()
        )));
    }
    Ok(bytes)
}

fn edit_hermes_request_with_runtime(
    runtime: &FiniteChatRuntime,
    request: &HermesEditRequestV1,
) -> Result<AppSentMessage, CliError> {
    let hermes_payload = HermesMessagePayloadV1::from_edit(request)
        .encode()
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let app_payload = encode_application_event(
        DurableAppEventKind::ChatMessage,
        request.conversation_id.clone(),
        request.segment_id.clone(),
        &hermes_payload,
    )?;
    append_payload_to_room_with_runtime(
        runtime,
        &request.room_id,
        app_payload,
        request.text.clone(),
    )
}

fn append_payload_to_room_with_runtime(
    runtime: &FiniteChatRuntime,
    room_id: &str,
    payload: Vec<u8>,
    preview: String,
) -> Result<AppSentMessage, CliError> {
    runtime
        .send_encoded_chat_message_and_wait(room_id.to_owned(), payload, preview)
        .map_err(map_core_hermes_error)
}

fn write_sent_message<W: Write>(output: &mut W, sent: &AppSentMessage) -> Result<(), CliError> {
    crate::write_pretty_json(output, &sent_message_value(sent))
}

fn sent_message_value(sent: &AppSentMessage) -> Value {
    json!({ "message_id": &sent.message_id, "seq": sent.seq })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HermesRunningState {
    #[serde(default)]
    messages: Vec<HermesRunningMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HermesRunningMessage {
    room_id: String,
    conversation_id: Option<String>,
    #[serde(default)]
    segment_id: Option<String>,
    #[serde(default)]
    kind: finitechat_hermes::HermesSendKindV1,
    message_id: String,
}

fn load_hermes_running(home_dir: &Path) -> Result<HermesRunningState, CliError> {
    let path = home_dir.join(HERMES_RUNNING_FILE);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(CliError::Hermes(error.to_string())),
    };
    serde_json::from_str(&raw).map_err(CliError::Json)
}

fn save_hermes_running(home_dir: &Path, running: &HermesRunningState) -> Result<(), CliError> {
    write_private(
        home_dir.join(HERMES_RUNNING_FILE),
        &serde_json::to_string_pretty(running).map_err(CliError::Serialize)?,
    )
}

fn update_running_after_send(
    home_dir: &Path,
    request: &HermesSendRequestV1,
    message_id: &str,
) -> Result<(), CliError> {
    if request.status != HermesMessageStatusV1::Running {
        return Ok(());
    }
    upsert_hermes_running_message(
        home_dir,
        HermesRunningMessage {
            room_id: request.room_id.clone(),
            conversation_id: request.conversation_id.clone(),
            segment_id: request.segment_id.clone(),
            kind: request.kind,
            message_id: message_id.to_owned(),
        },
    )
}

fn update_running_after_edit(
    home_dir: &Path,
    request: &HermesEditRequestV1,
) -> Result<(), CliError> {
    if request.finalize || request.status == HermesMessageStatusV1::Complete {
        return remove_hermes_running_message(home_dir, &request.room_id, &request.message_id);
    }
    upsert_hermes_running_message(
        home_dir,
        HermesRunningMessage {
            room_id: request.room_id.clone(),
            conversation_id: request.conversation_id.clone(),
            segment_id: request.segment_id.clone(),
            kind: request.kind,
            message_id: request.message_id.clone(),
        },
    )
}

fn upsert_hermes_running_message(
    home_dir: &Path,
    message: HermesRunningMessage,
) -> Result<(), CliError> {
    let mut running = load_hermes_running(home_dir)?;
    running.messages.retain(|existing| {
        existing.room_id != message.room_id || existing.message_id != message.message_id
    });
    running.messages.push(message);
    save_hermes_running(home_dir, &running)
}

fn remove_hermes_running_message(
    home_dir: &Path,
    room_id: &str,
    message_id: &str,
) -> Result<(), CliError> {
    let mut running = load_hermes_running(home_dir)?;
    let before = running.messages.len();
    running
        .messages
        .retain(|message| message.room_id != room_id || message.message_id != message_id);
    if running.messages.len() != before {
        save_hermes_running(home_dir, &running)?;
    }
    Ok(())
}

fn cmd_activity<W: Write>(home_dir: &Path, request: Value, output: &mut W) -> Result<(), CliError> {
    let mut request: HermesActivityRequestV1 =
        serde_json::from_value(request).map_err(CliError::Json)?;
    let home = load_home(home_dir)?;
    let runtime = open_agent_runtime(&home)?;
    runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(map_core_hermes_error)?;
    resolve_hermes_activity_route(&runtime, &mut request)?;
    let input = app_bridge_activity_input(request)?;
    let accepted = runtime
        .append_ephemeral_activity_and_wait(input)
        .map_err(map_core_hermes_error)?;
    crate::write_pretty_json(output, &json!({ "accepted": true, "result": accepted }))
}

fn app_bridge_activity_input(
    request: HermesActivityRequestV1,
) -> Result<AppBridgeActivityInput, CliError> {
    let payload = if matches!(request.action, EphemeralActivityActionV1::Set) {
        serde_json::to_vec(&request.payload).map_err(CliError::Serialize)?
    } else {
        Vec::new()
    };
    Ok(AppBridgeActivityInput {
        room_id: request.room_id,
        conversation_id: request.conversation_id,
        segment_id: request.segment_id,
        activity_kind: request.activity_kind,
        activity_id: request.activity_id,
        action: request.action,
        payload,
        expires_in_millis: request.expires_in_millis,
    })
}

// --- agent home plumbing ---

/// Resolve the agent home (durable agent state — never the identity, whose
/// location is fixed by the Finite Identity Contract). Precedence:
/// `--agent-home`/`--home` (parsed by clap), then `$FINITE_AGENT_HOME`,
/// then `$FINITECHAT_HOME`, then `~/.finite/agent`.
fn resolve_home(agent_home: Option<String>) -> Result<PathBuf, CliError> {
    if let Some(path) = agent_home {
        return Ok(PathBuf::from(path));
    }
    for name in ["FINITE_AGENT_HOME", "FINITECHAT_HOME"] {
        if let Some(path) = std::env::var_os(name) {
            return Ok(PathBuf::from(path));
        }
    }
    let Some(home) = std::env::var_os("HOME") else {
        return Err(CliError::Usage(
            "pass --agent-home DIR, set FINITE_AGENT_HOME, or set HOME".to_owned(),
        ));
    };
    Ok(PathBuf::from(home).join(".finite").join("agent"))
}

fn default_hermes_plugins_dir() -> Result<PathBuf, CliError> {
    if let Some(path) = std::env::var_os("HERMES_PLUGINS_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("HERMES_HOME") {
        return Ok(PathBuf::from(path).join("plugins"));
    }
    if let Some(path) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(path).join(".hermes").join("plugins"));
    }
    Err(CliError::Usage(
        "pass --plugins-dir DIR, --plugin-dir DIR, set HERMES_HOME, or set HOME".to_owned(),
    ))
}

fn validate_plugin_name(name: &str) -> Result<(), CliError> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(CliError::Usage(format!(
            "--plugin-name must be a single YAML-safe directory name, got {name:?}"
        )));
    }
    Ok(())
}

fn hermes_plugin_yaml_for_install(plugin_name: &str) -> String {
    HERMES_PLUGIN_YAML.replacen("name: finitechat", &format!("name: {plugin_name}"), 1)
}

fn hermes_recommended_config(plugin_name: &str, home_dir: &Path) -> String {
    format!(
        "plugins:\n  enabled:\n    - {plugin_name}\ngateway:\n  platforms:\n    {platform_name}:\n      enabled: true\n      extra:\n        home: {home}\n",
        platform_name = HERMES_PLATFORM_NAME,
        home = home_dir.display()
    )
}

fn detect_legacy_plugin_conflicts(
    plugins_dir: &Path,
    installed_plugin_dir: &Path,
    plugin_name: &str,
) -> Vec<HermesInstallLegacyPluginConflict> {
    let mut conflicts = Vec::new();
    for (candidate_name, reason) in [
        (
            LEGACY_HERMES_PLUGIN_NAME,
            "this is the legacy plaintext bridge",
        ),
        (
            AMBIGUOUS_HERMES_PLUGIN_NAME,
            "this is the old generic Finite plugin name",
        ),
    ] {
        let candidate_dir = plugins_dir.join(candidate_name);
        if same_path(&candidate_dir, installed_plugin_dir) {
            continue;
        }
        let yaml = candidate_dir.join("plugin.yaml");
        if !yaml.exists() {
            continue;
        }
        let yaml_name = plugin_yaml_name(&yaml);
        if yaml_name.as_deref() == Some(candidate_name)
            || yaml_name.as_deref() == Some(LEGACY_HERMES_PLUGIN_NAME)
        {
            conflicts.push(HermesInstallLegacyPluginConflict {
                plugin_name: candidate_name.to_owned(),
                plugin_dir: candidate_dir.display().to_string(),
                reason: format!("{reason}; enable '{plugin_name}' for Finite Chat"),
            });
        }
    }
    conflicts
}

fn detect_legacy_config_conflicts(
    plugins_dir: &Path,
    plugin_name: &str,
) -> Vec<HermesInstallLegacyConfigConflict> {
    let mut configs = Vec::new();
    if let Some(hermes_home) = plugins_dir.parent() {
        configs.push(hermes_home.join("config.yaml"));
        configs.push(hermes_home.join("config.yml"));
    }
    if let Some(home) = std::env::var_os("HERMES_HOME") {
        let home = PathBuf::from(home);
        configs.push(home.join("config.yaml"));
        configs.push(home.join("config.yml"));
    }
    configs.sort();
    configs.dedup();

    configs
        .into_iter()
        .flat_map(|path| {
            config_enabled_conflicting_plugins(&path)
                .into_iter()
                .map(move |enabled_plugin| HermesInstallLegacyConfigConflict {
                    config_path: path.display().to_string(),
                    enabled_plugin,
                    replacement_plugin: plugin_name.to_owned(),
                })
        })
        .collect()
}

fn config_enabled_conflicting_plugins(path: &Path) -> Vec<String> {
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for candidate in [LEGACY_HERMES_PLUGIN_NAME, AMBIGUOUS_HERMES_PLUGIN_NAME] {
        if contents
            .lines()
            .any(|line| yaml_line_is_value(line, candidate))
        {
            found.push(candidate.to_owned());
        }
    }
    found
}

fn yaml_line_is_value(line: &str, value: &str) -> bool {
    let trimmed = line.trim();
    trimmed == format!("- {value}")
        || trimmed == value
        || trimmed == format!("\"{value}\"")
        || trimmed == format!("'{value}'")
}

fn plugin_yaml_name(path: &Path) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    contents.lines().find_map(|line| {
        let trimmed = line.trim();
        let value = trimmed.strip_prefix("name:")?.trim();
        Some(value.trim_matches('"').trim_matches('\'').trim().to_owned())
    })
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || match (fs::canonicalize(left), fs::canonicalize(right)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
}

fn hermes_plugin_env_contents(
    home_dir: &Path,
    finitechat_bin: &Path,
    service_url: Option<&str>,
) -> Result<String, CliError> {
    let home = env_file_value("FINITECHAT_HOME", home_dir)?;
    let bin = env_file_value("FINITECHAT_BIN", finitechat_bin)?;
    let mut contents = format!("FINITECHAT_HOME={home}\nFINITECHAT_BIN={bin}\n");
    // Hosted runtimes pin the shared identity location with FINITE_HOME
    // (e.g. the durable /data/agent mount); propagate it so the plugin
    // shells finitechat against the same identity.
    if let Some(finite_home) = std::env::var_os("FINITE_HOME") {
        let finite_home = env_file_value("FINITE_HOME", Path::new(&finite_home))?;
        if !finite_home.trim().is_empty() {
            contents.push_str(&format!("FINITE_HOME={finite_home}\n"));
        }
    }
    if let Some(service_url) = service_url {
        let service_url = env_string_value("FINITECHAT_HERMES_SERVICE_URL", service_url)?;
        if !service_url.trim().is_empty() {
            contents.push_str(&format!("FINITECHAT_HERMES_SERVICE_URL={service_url}\n"));
        }
    }
    Ok(contents)
}

fn env_file_value(name: &str, path: &Path) -> Result<String, CliError> {
    env_string_value(name, &path.display().to_string())
}

fn env_string_value(name: &str, value: &str) -> Result<String, CliError> {
    if value.contains('\n') || value.contains('\r') || value.contains('\0') {
        return Err(CliError::Hermes(format!(
            "{name} contains a character that cannot be written to finitechat.env"
        )));
    }
    Ok(value.to_owned())
}

fn write_managed_plugin_file(
    path: &Path,
    contents: &str,
    force: bool,
    installed: &mut Vec<String>,
) -> Result<(), CliError> {
    write_managed_file(path, contents, force, false, installed)
}

fn write_managed_private_file(
    path: &Path,
    contents: &str,
    force: bool,
    installed: &mut Vec<String>,
) -> Result<(), CliError> {
    write_managed_file(path, contents, force, true, installed)
}

fn write_managed_file(
    path: &Path,
    contents: &str,
    force: bool,
    private: bool,
    installed: &mut Vec<String>,
) -> Result<(), CliError> {
    match fs::read(path) {
        Ok(existing) if existing == contents.as_bytes() => {
            installed.push(path.display().to_string());
            return Ok(());
        }
        Ok(_) if !force => {
            return Err(CliError::Hermes(format!(
                "{} already exists with different contents; pass --force to overwrite the managed Hermes plugin file",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(CliError::Hermes(error.to_string())),
    }
    if private {
        write_private(path.to_path_buf(), contents)?;
    } else {
        fs::write(path, contents).map_err(|error| CliError::Hermes(error.to_string()))?;
    }
    installed.push(path.display().to_string());
    Ok(())
}

fn load_home(dir: &Path) -> Result<AgentHome, CliError> {
    let config: AgentConfig =
        serde_json::from_str(&fs::read_to_string(dir.join(CONFIG_FILE)).map_err(|_| {
            CliError::Hermes(format!(
                "agent home {} is not initialized (run hermes init)",
                dir.display()
            ))
        })?)
        .map_err(CliError::Json)?;
    // Bridge commands never mint: the shared identity must already exist and
    // must be the account this agent home was initialized with.
    let paths = identity_paths()?;
    let identity = FiniteIdentity::load(&paths).map_err(|error| {
        CliError::Hermes(format!(
            "shared Finite identity unavailable at {}: {error}",
            paths.identity_file().display()
        ))
    })?;
    if identity.public_key_hex() != config.account_id {
        return Err(CliError::Hermes(format!(
            "shared Finite identity at {} is account {}, but agent home {} was initialized with account {}",
            paths.identity_file().display(),
            identity.public_key_hex(),
            dir.display(),
            config.account_id
        )));
    }
    let secret = NostrSecretKey::from_bytes(identity.expose_secret_bytes())
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    Ok(AgentHome {
        dir: dir.to_path_buf(),
        config,
        secret,
    })
}

fn identity_paths() -> Result<IdentityPaths, CliError> {
    IdentityPaths::resolve().map_err(|error| CliError::Hermes(error.to_string()))
}

/// `finitechat hermes init` acquisition: load the shared Finite identity,
/// minting under the contract's exclusive lock when absent.
fn load_or_generate_agent_secret() -> Result<NostrSecretKey, CliError> {
    let paths = identity_paths()?;
    let identity =
        FiniteIdentity::load_or_generate(&paths, concat!("finitechat ", env!("CARGO_PKG_VERSION")))
            .map_err(|error| CliError::Hermes(error.to_string()))?;
    NostrSecretKey::from_bytes(identity.expose_secret_bytes())
        .map_err(|error| CliError::Hermes(error.to_string()))
}

type AgentDelivery = HttpRuntimeDelivery<ReqwestHttpRuntimeTransport>;

fn open_agent_runtime(home: &AgentHome) -> Result<Arc<FiniteChatRuntime>, CliError> {
    FiniteChatRuntime::open(agent_runtime_open_options(home)).map_err(map_core_hermes_error)
}

fn agent_runtime_open_options(home: &AgentHome) -> OpenOptions {
    OpenOptions {
        data_dir: home.dir.to_string_lossy().into_owned(),
        server_url: home.config.server_url.clone(),
        device_id: home.config.device_id.clone(),
        account_secret_hex: Some(hex_lower(home.secret.as_bytes())),
        now_unix_seconds: None,
    }
}

fn map_core_hermes_error(error: FiniteChatCoreError) -> CliError {
    CliError::Hermes(error.to_string())
}

fn open_agent(
    home: &AgentHome,
) -> Result<(SqliteClientStore, FiniteChatDevice, AgentDelivery), CliError> {
    let store = open_store(&home.dir, &home.secret, &home.config.device_id)?;
    let config = device_config(&home.secret, &home.config.device_id, now_secs());
    let device = store
        .load_device(config)
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let delivery = HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(
        home.config.server_url.clone(),
    ));
    // The store mutably borrows during ticks; return all three.
    Ok((store, device, delivery))
}

fn open_store(
    dir: &Path,
    secret: &NostrSecretKey,
    device_id: &str,
) -> Result<SqliteClientStore, CliError> {
    let options = SqliteClientStoreOptions::from_nostr_secret(secret, device_id)
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    SqliteClientStore::open(dir.join(STORE_FILE), options)
        .map_err(|error| CliError::Hermes(error.to_string()))
}

/// The store handle for read-only status probes (the typed read/write
/// command class at this dispatch boundary): constructible only through the
/// read-only open mode, so a probe command cannot name the writer open —
/// routing one through `open_store` is a compile error, not a convention
/// slip. Never takes the writer lease, runs no migrations, and the store
/// rejects any write through it, so it is safe alongside a resident
/// `hermes serve` that owns the store.
struct ReadOnlyAgentStore(SqliteClientStore);

impl ReadOnlyAgentStore {
    fn open(dir: &Path, secret: &NostrSecretKey, device_id: &str) -> Result<Self, CliError> {
        let options = SqliteClientStoreOptions::from_nostr_secret(secret, device_id)
            .map_err(|error| CliError::Hermes(error.to_string()))?;
        SqliteClientStore::open_read_only(dir.join(STORE_FILE), options)
            .map(Self)
            .map_err(|error| CliError::Hermes(error.to_string()))
    }

    fn load_device(
        &self,
        config: FiniteChatDeviceConfig,
    ) -> Result<FiniteChatDevice, ClientStoreError> {
        self.0.load_device(config)
    }

    fn load_app_rooms(&self, owner: &DeviceRef) -> Result<Vec<StoredAppRoom>, ClientStoreError> {
        self.0.load_app_rooms(owner)
    }

    fn load_app_messages(
        &self,
        owner: &DeviceRef,
        limit: u32,
    ) -> Result<Vec<StoredAppMessage>, ClientStoreError> {
        self.0.load_app_messages(owner, limit)
    }
}

fn device_config(
    secret: &NostrSecretKey,
    device_id: &str,
    now_secs: u64,
) -> FiniteChatDeviceConfig {
    FiniteChatDeviceConfig {
        account_secret_key: secret.clone(),
        device_id: device_id.to_owned(),
        now_unix_seconds: now_secs,
        credential_not_before_unix_seconds: now_secs.saturating_sub(3600),
        credential_not_after_unix_seconds: now_secs + CREDENTIAL_VALIDITY_SECONDS,
    }
}

fn write_private(path: PathBuf, contents: &str) -> Result<(), CliError> {
    let parent = path.parent().ok_or_else(|| {
        CliError::Hermes(format!("private file {} has no parent", path.display()))
    })?;
    let file_name = path.file_name().ok_or_else(|| {
        CliError::Hermes(format!("private file {} has no file name", path.display()))
    })?;
    let temporary = parent.join(format!(
        ".{}.{}.{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        PRIVATE_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let staged = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = staged {
        let _ = fs::remove_file(&temporary);
        return Err(CliError::Hermes(error.to_string()));
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(CliError::Hermes(error.to_string()));
    }
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn read_request(request_json: Option<String>) -> Result<Value, CliError> {
    match request_json {
        Some(raw) => serde_json::from_str(&raw).map_err(CliError::Json),
        None => {
            let mut raw = String::new();
            std::io::stdin()
                .read_to_string(&mut raw)
                .map_err(|error| CliError::Hermes(error.to_string()))?;
            if raw.trim().is_empty() {
                return Ok(Value::Object(serde_json::Map::new()));
            }
            serde_json::from_str(&raw).map_err(CliError::Json)
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn now_secs() -> u64 {
    now_ms() / 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;
    use finitechat_blob::{MemoryBlobStore, upload_attachment};
    use finitechat_hermes::HermesMessageTypeV1;

    /// `hermes install` requires an initialized agent home; plant the
    /// config marker without running init (install only checks existence).
    fn write_test_agent_config(home: &Path) {
        fs::create_dir_all(home).unwrap();
        fs::write(
            home.join(CONFIG_FILE),
            r#"{"server_url":"http://127.0.0.1:1","device_id":"agent","account_id":"00"}"#,
        )
        .unwrap();
    }

    /// Parse `hermes install` arguments exactly as the CLI would.
    fn install_args(args: &[&str]) -> crate::cli::HermesInstallArgs {
        crate::cli::HermesInstallArgs::try_parse_from(
            std::iter::once("install").chain(args.iter().copied()),
        )
        .expect("hermes install args parse")
    }

    /// Parse `hermes init` arguments exactly as the CLI would.
    fn init_args(args: &[&str]) -> crate::cli::HermesInitArgs {
        crate::cli::HermesInitArgs::try_parse_from(
            std::iter::once("init").chain(args.iter().copied()),
        )
        .expect("hermes init args parse")
    }

    #[test]
    fn private_journal_replacement_is_complete_and_leaves_no_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HERMES_RUNNING_FILE);
        write_private(path.clone(), r#"{"messages":[{"message_id":"before"}]}"#).unwrap();
        write_private(path.clone(), r#"{"messages":[{"message_id":"after"}]}"#).unwrap();

        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["messages"][0]["message_id"], "after");
        let siblings = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(siblings, vec![HERMES_RUNNING_FILE]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    fn encrypted_test_attachment() -> (Vec<u8>, AttachmentBlobReferenceV1) {
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(
            &mut store,
            b"finite image bytes",
            finitechat_proto::AttachmentBlobMetadataV1 {
                mime_type: "image/png".to_owned(),
                filename: "image.png".to_owned(),
                dimensions: None,
            },
        )
        .expect("encrypt attachment");
        (uploaded.ciphertext, uploaded.reference)
    }

    #[test]
    fn resident_agent_runtime_uses_the_live_clock() {
        let dir = tempfile::tempdir().unwrap();
        let home = AgentHome {
            dir: dir.path().join("agent-home"),
            config: AgentConfig {
                server_url: "http://127.0.0.1:1".to_owned(),
                device_id: "agent-device".to_owned(),
                account_id: "agent-account".to_owned(),
            },
            secret: NostrSecretKey::from_bytes([0x17; 32]).unwrap(),
        };

        let options = agent_runtime_open_options(&home);

        assert_eq!(options.now_unix_seconds, None);
        assert_eq!(options.data_dir, home.dir.to_string_lossy());
        assert_eq!(options.server_url, home.config.server_url);
        assert_eq!(options.device_id, home.config.device_id);
        assert_eq!(
            options.account_secret_hex,
            Some(hex_lower(home.secret.as_bytes()))
        );
    }

    #[test]
    fn resident_bridge_only_wakes_consumers_for_observed_updates() {
        assert!(!bridge_sync_has_updates(&AppBridgeSync::default()));
        assert!(bridge_sync_has_updates(&AppBridgeSync {
            joined_account_ids: vec!["joined-account".to_owned()],
            events: Vec::new(),
        }));
        assert!(bridge_sync_has_updates(&AppBridgeSync {
            joined_account_ids: Vec::new(),
            events: vec![finitechat_core::AppBridgeAppliedEvent {
                room_id: "room-main".to_owned(),
                seq: 1,
                message_id: "message-1".to_owned(),
                sender_account_id: "sender-account".to_owned(),
                sender_device_id: "sender-device".to_owned(),
                plaintext: b"event".to_vec(),
            }],
        }));
    }

    #[test]
    fn initialized_hermes_inbox_does_not_reopen_event_store() {
        let dir = tempfile::tempdir().unwrap();
        let agent_home = dir.path().join("store-must-remain-unopened");
        let home = AgentHome {
            dir: agent_home.clone(),
            config: AgentConfig {
                server_url: "http://127.0.0.1:1".to_owned(),
                device_id: "agent-device".to_owned(),
                account_id: "agent-account".to_owned(),
            },
            secret: NostrSecretKey::from_bytes([0x19; 32]).unwrap(),
        };
        let mut inbox = HermesInboxState::default();
        inbox.cursors.insert("room-main".to_owned(), 7);

        initialize_hermes_inbox_cursors(&agent_home, &home, &mut inbox).unwrap();

        assert!(
            !agent_home.exists(),
            "an initialized inbox should not open or scan the encrypted Agent store"
        );
    }

    fn encoded_media_payload(reference: AttachmentBlobReferenceV1, text: &str) -> Vec<u8> {
        HermesMessagePayloadV1 {
            payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: None,
            segment_id: None,
            text: text.to_owned(),
            kind: finitechat_hermes::HermesSendKindV1::Media,
            status: HermesMessageStatusV1::Complete,
            edit_of: None,
            attachments: vec![finitechat_hermes::HermesAttachmentV1 {
                kind: finitechat_hermes::HermesAttachmentKindV1::Image,
                name: "image.png".to_owned(),
                mime_type: "image/png".to_owned(),
                path: None,
                url: Some(reference.url.clone()),
                blob: Some(reference),
            }],
            reply_to_message_id: None,
            sender_name: None,
            metadata: BTreeMap::new(),
        }
        .encode()
        .expect("encode media payload")
    }

    fn serve_one_blob_response(
        listener: std::net::TcpListener,
        status: &str,
        body: Vec<u8>,
    ) -> std::thread::JoinHandle<()> {
        let status = status.to_owned();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept blob request");
            let mut request = [0_u8; 2048];
            let size = stream.read(&mut request).expect("read blob request");
            assert!(
                String::from_utf8_lossy(&request[..size]).starts_with("GET /blobs/"),
                "materializer must request the content-addressed blob path"
            );
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write blob response headers");
            stream.write_all(&body).expect("write blob response body");
        })
    }

    fn stored_main_room(state: StoredAppRoomState, status: &str) -> StoredAppRoom {
        StoredAppRoom {
            room_id: "room-main".to_owned(),
            display_name: "Main".to_owned(),
            picture: None,
            state,
            status: status.to_owned(),
            local_read_seq: 0,
        }
    }

    #[test]
    fn room_status_summary_pairs_connected_room_with_other_member() {
        let stored = stored_main_room(StoredAppRoomState::Connected, "connected");
        let (state, status) = stored_room_state_and_status(Some(&stored), true);

        let summary = hermes_room_status_summary("room-main", state, status, 2, 1, false);

        assert!(summary.connected);
        assert!(summary.paired);
        assert_eq!(summary.member_count, 2);
        assert_eq!(summary.other_member_count, 1);
        assert_eq!(summary.state, "connected");
    }

    #[test]
    fn room_status_summary_does_not_pair_without_other_member() {
        let stored = stored_main_room(StoredAppRoomState::Connected, "connected");
        let (state, status) = stored_room_state_and_status(Some(&stored), true);

        let summary = hermes_room_status_summary("room-main", state, status, 1, 0, false);

        assert!(summary.connected);
        assert!(!summary.paired);
        assert_eq!(summary.member_count, 1);
        assert_eq!(summary.other_member_count, 0);
    }

    #[test]
    fn room_status_summary_marks_stored_room_without_mls_state_unavailable() {
        let stored = stored_main_room(StoredAppRoomState::Connected, "connected");

        let (state, status) = stored_room_state_and_status(Some(&stored), false);

        assert_eq!(state, "unavailable_on_device");
        assert_eq!(status, "room is not available on this device");
    }

    #[test]
    fn room_status_summary_reports_unknown_room_as_not_found() {
        let (state, status) = stored_room_state_and_status(None, false);

        assert_eq!(state, "unknown");
        assert_eq!(status, "not_found");
    }

    #[test]
    fn install_writes_embedded_plugin_and_local_env_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("agent-home");
        let plugin_dir = dir.path().join("hermes").join("plugins").join("finitechat");
        write_test_agent_config(&home);

        let mut output = Vec::new();
        cmd_install(
            &home,
            install_args(&[
                "--plugin-dir",
                &plugin_dir.display().to_string(),
                "--finitechat-bin",
                "/usr/local/bin/finitechat",
                "--service-url",
                "http://127.0.0.1:4321",
            ]),
            true,
            &mut output,
        )
        .expect("install succeeds");

        let summary: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(summary["plugin_name"], "finitechat");
        assert_eq!(summary["platform_name"], "finitechat");
        assert_eq!(summary["plugin_dir"], plugin_dir.display().to_string());
        assert_eq!(summary["warnings"].as_array().unwrap().len(), 0);
        assert!(plugin_dir.join("__init__.py").exists());
        assert!(plugin_dir.join("adapter.py").exists());
        assert!(!plugin_dir.join("specialization.py").exists());
        assert!(plugin_dir.join("plugin.yaml").exists());
        assert!(plugin_dir.join(HERMES_PLUGIN_ENV_FILE).exists());

        let plugin_yaml = fs::read_to_string(plugin_dir.join("plugin.yaml")).unwrap();
        assert!(plugin_yaml.lines().any(|line| line == "name: finitechat"));
        assert!(plugin_yaml.contains("  - pre_tool_call\n  - post_tool_call"));
        let adapter = fs::read_to_string(plugin_dir.join("adapter.py")).unwrap();
        assert!(adapter.contains("_RequesterContextBroker"));
        assert!(
            summary["recommended_config"]
                .as_str()
                .unwrap()
                .contains("gateway:\n  platforms:\n    finitechat:")
        );
        let env = fs::read_to_string(plugin_dir.join(HERMES_PLUGIN_ENV_FILE)).unwrap();
        assert!(env.contains(&format!("FINITECHAT_HOME={}", home.display())));
        assert!(env.contains("FINITECHAT_BIN=/usr/local/bin/finitechat"));
        assert!(env.contains("FINITECHAT_HERMES_SERVICE_URL=http://127.0.0.1:4321"));
    }

    #[test]
    fn install_reports_legacy_plaintext_plugin_and_config_collision() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("agent-home");
        let hermes_home = dir.path().join("hermes");
        let plugins_dir = hermes_home.join("plugins");
        let legacy_dir = plugins_dir.join(LEGACY_HERMES_PLUGIN_NAME);
        let ambiguous_dir = plugins_dir.join(AMBIGUOUS_HERMES_PLUGIN_NAME);
        write_test_agent_config(&home);
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("plugin.yaml"),
            "name: finite-platform\nkind: platform\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::create_dir_all(&ambiguous_dir).unwrap();
        fs::write(
            ambiguous_dir.join("plugin.yaml"),
            "name: finite-platform\nkind: platform\nversion: 0.2.0\n",
        )
        .unwrap();
        fs::write(
            hermes_home.join("config.yaml"),
            "plugins:\n  enabled:\n    - finite-platform\n    - finite\ngateway:\n  platforms:\n    finite:\n      enabled: true\n",
        )
        .unwrap();

        let mut output = Vec::new();
        cmd_install(
            &home,
            install_args(&[
                "--plugins-dir",
                &plugins_dir.display().to_string(),
                "--finitechat-bin",
                "/usr/local/bin/finitechat",
            ]),
            true,
            &mut output,
        )
        .expect("install succeeds with warnings");

        let summary: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(summary["plugin_name"], "finitechat");
        assert_eq!(
            summary["plugin_dir"],
            plugins_dir.join("finitechat").display().to_string()
        );
        let legacy_plugins = summary["legacy_plugin_conflicts"].as_array().unwrap();
        assert_eq!(legacy_plugins.len(), 2);
        assert_eq!(legacy_plugins[0]["plugin_name"], "finite-platform");
        assert!(
            legacy_plugins[0]["reason"]
                .as_str()
                .unwrap()
                .contains("legacy plaintext bridge")
        );
        assert_eq!(legacy_plugins[1]["plugin_name"], "finite");
        assert!(
            legacy_plugins[1]["reason"]
                .as_str()
                .unwrap()
                .contains("old generic Finite plugin name")
        );
        let legacy_configs = summary["legacy_config_conflicts"].as_array().unwrap();
        assert_eq!(legacy_configs.len(), 2);
        assert_eq!(legacy_configs[0]["enabled_plugin"], "finite-platform");
        assert_eq!(legacy_configs[0]["replacement_plugin"], "finitechat");
        assert_eq!(legacy_configs[1]["enabled_plugin"], "finite");
        assert_eq!(legacy_configs[1]["replacement_plugin"], "finitechat");
        assert!(
            summary["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|warning| warning
                    .as_str()
                    .unwrap()
                    .contains("change plugins.enabled to 'finitechat'"))
        );
    }

    #[test]
    fn backup_activity_guard_marks_and_unmarks_agent_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("agent-home");
        let marker = home.join(BACKUP_ACTIVITY_FILE);
        {
            let _guard = BackupActivityGuard::enter(&home, "send").unwrap();
            assert!(marker.exists());
            let value: Value = serde_json::from_str(&fs::read_to_string(&marker).unwrap()).unwrap();
            assert_eq!(value["action"], "send");
        }
        assert!(!marker.exists());
    }

    #[test]
    fn install_is_idempotent_but_refuses_to_overwrite_local_edits_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("agent-home");
        let plugin_dir = dir.path().join("finite-plugin");
        write_test_agent_config(&home);
        let plugin_dir_arg = plugin_dir.display().to_string();
        let base_args = [
            "--plugin-dir",
            plugin_dir_arg.as_str(),
            "--finitechat-bin",
            "/bin/finitechat",
        ];

        let mut output = Vec::new();
        cmd_install(&home, install_args(&base_args), true, &mut output).expect("first install");
        output.clear();
        cmd_install(&home, install_args(&base_args), true, &mut output)
            .expect("same install is idempotent");

        fs::write(plugin_dir.join("adapter.py"), "# local edit\n").unwrap();
        let error = cmd_install(&home, install_args(&base_args), true, &mut output)
            .expect_err("local edit requires --force");
        assert!(error.to_string().contains("--force"));

        let mut force_args = base_args.to_vec();
        force_args.push("--force");
        cmd_install(&home, install_args(&force_args), true, &mut output)
            .expect("force overwrites managed file");
        let adapter = fs::read_to_string(plugin_dir.join("adapter.py")).unwrap();
        assert!(adapter.contains("Finite Chat platform plugin for Hermes"));
    }

    #[test]
    fn install_fails_when_agent_home_is_not_initialized() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("agent-home");
        let plugin_dir = dir.path().join("finite-plugin");
        let mut output = Vec::new();

        let error = cmd_install(
            &home,
            install_args(&[
                "--plugin-dir",
                &plugin_dir.display().to_string(),
                "--finitechat-bin",
                "/bin/finitechat",
            ]),
            true,
            &mut output,
        )
        .expect_err("uninitialized agent home fails");
        assert!(error.to_string().contains("not initialized"));
        assert!(!plugin_dir.exists());
    }

    #[test]
    fn home_channel_rejects_room_not_available_to_agent() {
        crate::ensure_test_finite_home();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("agent-home");
        let mut output = Vec::new();
        cmd_init(
            &home,
            init_args(&["--server", "http://127.0.0.1:1", "--skip-agent-profile"]),
            &mut output,
        )
        .expect("init");
        output.clear();

        let error = set_home_channel(&home, "missing-room".to_owned(), None, &mut output)
            .expect_err("unknown room cannot become home channel");
        assert!(error.to_string().contains("not available"));
    }

    #[test]
    fn poll_decoder_unwraps_typed_chat_message_but_ignores_non_hermes_typed_payloads() {
        let home = tempfile::tempdir().unwrap();
        let wrapped_poll = DecryptedApplicationEventV1 {
            kind: DurableAppEventKind::ChatMessage,
            conversation_id: None,
            segment_id: None,
            payload: br#"{"type":"finitechat.chat.poll.v1","question":"Lunch?","options":[]}"#
                .to_vec(),
        };
        let plaintext = serde_json::to_vec(&wrapped_poll).unwrap();
        let event = hermes_poll_event_from_application_plaintext(
            HermesPollEventContext {
                home_dir: home.path(),
                server_url: "https://chat.finite.computer",
                room_id: "room-main",
                seq: 1,
                message_id: "message-1",
                sender_account_id: "alice",
                sender_device_id: "ios",
                conversation_id: None,
                segment_id: None,
            },
            &plaintext,
        )
        .unwrap();
        assert!(
            event.is_none(),
            "typed non-Hermes payloads must not leak to agents as JSON text"
        );

        let wrapped_text = DecryptedApplicationEventV1 {
            kind: DurableAppEventKind::ChatMessage,
            conversation_id: None,
            segment_id: None,
            payload: b"plain hello".to_vec(),
        };
        let plaintext = serde_json::to_vec(&wrapped_text).unwrap();
        let event = hermes_poll_event_from_application_plaintext(
            HermesPollEventContext {
                home_dir: home.path(),
                server_url: "https://chat.finite.computer",
                room_id: "room-main",
                seq: 2,
                message_id: "message-2",
                sender_account_id: "alice",
                sender_device_id: "ios",
                conversation_id: None,
                segment_id: None,
            },
            &plaintext,
        )
        .unwrap()
        .expect("typed plain-text chat is still bridge-visible");
        assert_eq!(event.text, "plain hello");
        assert_eq!(event.message_type, HermesMessageTypeV1::Text);
    }

    #[test]
    fn poll_decoder_degrades_malformed_json_payload_to_lossy_text() {
        let home = tempfile::tempdir().unwrap();
        let malformed = br#"{"type":"finitechat.hermes.message.v1","#.to_vec();
        let wrapped = DecryptedApplicationEventV1 {
            kind: DurableAppEventKind::ChatMessage,
            conversation_id: None,
            segment_id: None,
            payload: malformed.clone(),
        };
        let plaintext = serde_json::to_vec(&wrapped).unwrap();
        let event = hermes_poll_event_from_application_plaintext(
            HermesPollEventContext {
                home_dir: home.path(),
                server_url: "https://chat.finite.computer",
                room_id: "room-main",
                seq: 1,
                message_id: "message-malformed",
                sender_account_id: "alice",
                sender_device_id: "ios",
                conversation_id: None,
                segment_id: None,
            },
            &plaintext,
        )
        .unwrap()
        .expect("malformed JSON degrades to lossy text like any plain payload");
        assert_eq!(event.text, String::from_utf8(malformed).unwrap());
        assert_eq!(event.message_type, HermesMessageTypeV1::Text);

        let wrapped_binary = DecryptedApplicationEventV1 {
            kind: DurableAppEventKind::ChatMessage,
            conversation_id: None,
            segment_id: None,
            payload: vec![0xff, 0xfe],
        };
        let plaintext = serde_json::to_vec(&wrapped_binary).unwrap();
        let event = hermes_poll_event_from_application_plaintext(
            HermesPollEventContext {
                home_dir: home.path(),
                server_url: "https://chat.finite.computer",
                room_id: "room-main",
                seq: 2,
                message_id: "message-binary",
                sender_account_id: "alice",
                sender_device_id: "ios",
                conversation_id: None,
                segment_id: None,
            },
            &plaintext,
        )
        .unwrap();
        assert!(event.is_none(), "non-UTF-8 payloads stay undeliverable");
    }

    #[test]
    fn hermes_routes_historical_loopback_blob_reference_through_trusted_server() {
        let hash = "d0311404f15ff0cb53859640000453741bed701ff3df2b5b7f867904c164fb39";
        let reference = AttachmentBlobReferenceV1 {
            scheme: "finitechat.attachment.blob.v1".to_owned(),
            url: format!("http://127.0.0.1:8788/blobs/{hash}"),
            ciphertext_sha256: hash.to_owned(),
            plaintext_sha256: "a".repeat(64),
            plaintext_size: 1,
            ciphertext_size: 17,
            encryption: finitechat_proto::AttachmentBlobEncryptionV1 {
                algorithm: "aes-256-gcm".to_owned(),
                key_hex: "b".repeat(64),
                nonce_hex: "c".repeat(24),
            },
            metadata: finitechat_proto::AttachmentBlobMetadataV1 {
                mime_type: "image/png".to_owned(),
                filename: "image.png".to_owned(),
                dimensions: None,
            },
        };

        let routed = hermes_attachment_download_url(
            &reference.url,
            "https://chat.finite.computer",
            &reference,
        )
        .expect("route loopback reference");

        assert_eq!(routed, format!("https://chat.finite.computer/blobs/{hash}"));
    }

    #[test]
    fn hermes_preserves_non_loopback_blob_store_reference() {
        let hash = "d0311404f15ff0cb53859640000453741bed701ff3df2b5b7f867904c164fb39";
        let reference = AttachmentBlobReferenceV1 {
            scheme: "finitechat.attachment.blob.v1".to_owned(),
            url: format!("https://blobs.example/blobs/{hash}"),
            ciphertext_sha256: hash.to_owned(),
            plaintext_sha256: "a".repeat(64),
            plaintext_size: 1,
            ciphertext_size: 17,
            encryption: finitechat_proto::AttachmentBlobEncryptionV1 {
                algorithm: "aes-256-gcm".to_owned(),
                key_hex: "b".repeat(64),
                nonce_hex: "c".repeat(24),
            },
            metadata: finitechat_proto::AttachmentBlobMetadataV1 {
                mime_type: "image/png".to_owned(),
                filename: "image.png".to_owned(),
                dimensions: None,
            },
        };

        let routed = hermes_attachment_download_url(
            &reference.url,
            "https://chat.finite.computer",
            &reference,
        )
        .expect("keep external reference");

        assert_eq!(routed, reference.url);
    }

    #[test]
    fn attachment_http_status_classification_matches_retry_policy() {
        let cases = [
            (302, true),
            (307, true),
            (400, false),
            (401, false),
            (403, false),
            (404, false),
            (408, true),
            (410, false),
            (425, true),
            (429, true),
            (500, true),
            (503, true),
        ];

        for (status, expected_retryable) in cases {
            let actual = classify_attachment_http_error(status);
            assert_eq!(
                matches!(actual, AttachmentMaterializationError::Retryable(_)),
                expected_retryable,
                "unexpected classification for HTTP {status}"
            );
        }
    }

    #[test]
    fn attachment_redirect_is_followed_and_final_client_error_is_permanent() {
        let (_, mut reference) = encrypted_test_attachment();
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind redirect blob server");
        let address = listener.local_addr().expect("redirect blob server address");
        reference.url = format!("http://{address}/blobs/{}", reference.ciphertext_sha256);
        let server_url = format!("http://{address}");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept initial blob request");
            let mut request = [0_u8; 2048];
            let size = stream
                .read(&mut request)
                .expect("read initial blob request");
            assert!(
                String::from_utf8_lossy(&request[..size]).starts_with("GET /blobs/"),
                "materializer must request the content-addressed blob path"
            );
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: /missing\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .expect("write redirect response");

            let (mut stream, _) = listener.accept().expect("accept redirected blob request");
            let size = stream
                .read(&mut request)
                .expect("read redirected blob request");
            assert!(
                String::from_utf8_lossy(&request[..size]).starts_with("GET /missing "),
                "materializer must follow the redirect exactly once"
            );
            write!(
                stream,
                "HTTP/1.1 410 Gone\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .expect("write final response");
        });

        let home = tempfile::tempdir().unwrap();
        let error = materialize_blob_attachment(home.path(), &server_url, &reference)
            .expect_err("final deterministic client error must degrade");
        server.join().expect("redirect blob server");

        assert!(matches!(
            error,
            AttachmentMaterializationError::Permanent(_)
        ));
    }

    #[test]
    fn permanently_missing_noncanonical_reference_degrades_instead_of_wedging() {
        let (_, mut reference) = encrypted_test_attachment();
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind missing blob server");
        let address = listener.local_addr().expect("missing blob server address");
        reference.url = format!("http://{address}/objects/{}", reference.ciphertext_sha256);
        let payload = encoded_media_payload(reference, "Keep this caption");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept missing blob request");
            let mut request = [0_u8; 2048];
            let size = stream
                .read(&mut request)
                .expect("read missing blob request");
            assert!(
                String::from_utf8_lossy(&request[..size]).starts_with("GET /objects/"),
                "the exact noncanonical reference must be requested"
            );
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .expect("write missing blob response");
        });
        let home = tempfile::tempdir().unwrap();

        let degraded = hermes_poll_event_from_application_plaintext(
            HermesPollEventContext {
                home_dir: home.path(),
                server_url: "https://chat.finite.computer",
                room_id: "room-main",
                seq: 1,
                message_id: "message-missing-media",
                sender_account_id: "alice",
                sender_device_id: "ios",
                conversation_id: None,
                segment_id: None,
            },
            &payload,
        )
        .expect("exact-reference 404 is permanent")
        .expect("missing media caption remains deliverable");
        server.join().expect("missing blob server");

        assert_eq!(
            degraded.text,
            format!("Keep this caption\n\n{HERMES_ATTACHMENT_UNAVAILABLE_NOTICE}")
        );
        assert_eq!(degraded.message_type, HermesMessageTypeV1::Text);
        assert!(degraded.attachments[0].path.is_none());
        assert!(degraded.attachments[0].url.is_none());
    }

    #[test]
    fn hermes_materializes_historical_loopback_reference_from_canonical_server() {
        let plaintext = b"finite image bytes";
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(
            &mut store,
            plaintext,
            finitechat_proto::AttachmentBlobMetadataV1 {
                mime_type: "image/png".to_owned(),
                filename: "image.png".to_owned(),
                dimensions: None,
            },
        )
        .expect("encrypt attachment");
        let ciphertext = uploaded.ciphertext.clone();
        let mut reference = uploaded.reference;
        reference.url = format!("http://127.0.0.1:1/blobs/{}", reference.ciphertext_sha256);

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test blob server");
        let server_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept blob request");
            let mut request = [0_u8; 2048];
            let size = stream.read(&mut request).expect("read blob request");
            assert!(
                String::from_utf8_lossy(&request[..size]).starts_with("GET /blobs/"),
                "materializer must request the canonical content-addressed path"
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                ciphertext.len()
            )
            .expect("write blob response headers");
            stream
                .write_all(&ciphertext)
                .expect("write blob response body");
        });

        let home = tempfile::tempdir().unwrap();
        let path = materialize_blob_attachment(home.path(), &server_url, &reference)
            .expect("materialize historical reference");

        assert_eq!(fs::read(path).unwrap(), plaintext);
        server.join().expect("blob server");
    }

    #[test]
    fn transient_attachment_server_outage_retries_without_advancing_cursor() {
        let (ciphertext, mut reference) = encrypted_test_attachment();
        reference.url = format!("http://127.0.0.1:1/blobs/{}", reference.ciphertext_sha256);

        let reservation =
            std::net::TcpListener::bind("127.0.0.1:0").expect("reserve test blob address");
        let address = reservation.local_addr().expect("reserved address");
        drop(reservation);
        let server_url = format!("http://{address}");
        let media_payload = encoded_media_payload(reference, "Please inspect this");
        let home = tempfile::tempdir().unwrap();
        let context = HermesPollEventContext {
            home_dir: home.path(),
            server_url: &server_url,
            room_id: "room-main",
            seq: 1,
            message_id: "message-retry-media",
            sender_account_id: "alice",
            sender_device_id: "ios",
            conversation_id: None,
            segment_id: None,
        };
        let mut inbox = HermesInboxState::default();

        let first = hermes_poll_event_from_application_plaintext(context, &media_payload)
            .expect_err("server outage keeps the durable event pending");
        assert!(first.to_string().contains("temporarily unavailable"));
        assert_eq!(hermes_inbox_cursor(&inbox, "room-main"), 0);
        assert!(pending_hermes_inbox_events(&inbox, None, 10).is_empty());

        let listener = std::net::TcpListener::bind(address).expect("restore test blob server");
        let server = serve_one_blob_response(listener, "200 OK", ciphertext);
        let recovered = hermes_poll_event_from_application_plaintext(context, &media_payload)
            .expect("reconnected stream retries the same event")
            .expect("media event materializes after recovery");
        server.join().expect("recovered blob server");

        assert_eq!(recovered.text, "Please inspect this");
        let path = recovered.attachments[0]
            .path
            .as_deref()
            .expect("verified local media path");
        assert_eq!(fs::read(path).unwrap(), b"finite image bytes");
        enqueue_hermes_inbox_event(home.path(), &mut inbox, recovered).unwrap();
        assert_eq!(hermes_inbox_cursor(&inbox, "room-main"), 1);
        assert_eq!(pending_hermes_inbox_events(&inbox, None, 10).len(), 1);
    }

    #[test]
    fn truncated_attachment_download_is_retryable() {
        let (ciphertext, reference) = encrypted_test_attachment();
        let truncated = &ciphertext[..ciphertext.len() - 1];
        let error = finish_blossom_download_http_response(
            &reference,
            BlossomDownloadHttpResponse {
                status: 200,
                body: truncated,
            },
        )
        .expect_err("truncated ciphertext fails verification");

        assert!(matches!(
            classify_attachment_verification_error(error),
            AttachmentMaterializationError::Retryable(_)
        ));
    }

    #[test]
    fn permanently_corrupt_attachment_degrades_and_does_not_block_following_text() {
        let (mut corrupt_ciphertext, mut reference) = encrypted_test_attachment();
        corrupt_ciphertext[0] ^= 0xff;
        reference.url = format!("http://127.0.0.1:1/blobs/{}", reference.ciphertext_sha256);

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind bad blob server");
        let server_url = format!("http://{}", listener.local_addr().unwrap());
        let server = serve_one_blob_response(listener, "200 OK", corrupt_ciphertext);
        let media_payload = encoded_media_payload(reference, "Please inspect this");
        let home = tempfile::tempdir().unwrap();
        let degraded = hermes_poll_event_from_application_plaintext(
            HermesPollEventContext {
                home_dir: home.path(),
                server_url: &server_url,
                room_id: "room-main",
                seq: 1,
                message_id: "message-bad-media",
                sender_account_id: "alice",
                sender_device_id: "ios",
                conversation_id: None,
                segment_id: None,
            },
            &media_payload,
        )
        .expect("bad attachment degrades instead of failing the stream")
        .expect("caption remains deliverable");
        server.join().expect("bad blob server");

        assert_eq!(
            degraded.text,
            format!("Please inspect this\n\n{HERMES_ATTACHMENT_UNAVAILABLE_NOTICE}")
        );
        assert_eq!(degraded.message_type, HermesMessageTypeV1::Text);
        assert!(degraded.attachments[0].path.is_none());
        assert!(degraded.attachments[0].url.is_none());
        assert!(degraded.attachments[0].blob.is_some());

        let following = hermes_poll_event_from_application_plaintext(
            HermesPollEventContext {
                home_dir: home.path(),
                server_url: &server_url,
                room_id: "room-main",
                seq: 2,
                message_id: "message-following-text",
                sender_account_id: "alice",
                sender_device_id: "ios",
                conversation_id: None,
                segment_id: None,
            },
            b"Following text still arrives",
        )
        .expect("following text decodes")
        .expect("following text is deliverable");

        let mut inbox = HermesInboxState::default();
        enqueue_hermes_inbox_event(home.path(), &mut inbox, degraded).unwrap();
        enqueue_hermes_inbox_event(home.path(), &mut inbox, following).unwrap();
        let pending = pending_hermes_inbox_events(&inbox, None, 10);
        assert_eq!(
            pending
                .iter()
                .map(|event| event.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["message-bad-media", "message-following-text"]
        );

        let mut output = Vec::new();
        cmd_ack(
            home.path(),
            serde_json::to_value(HermesAckRequestV1 {
                room_id: "room-main".to_owned(),
                seq: 1,
                message_id: "message-bad-media".to_owned(),
            })
            .unwrap(),
            &mut output,
        )
        .unwrap();
        let inbox = load_hermes_inbox(home.path()).unwrap();
        let pending = pending_hermes_inbox_events(&inbox, None, 10);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].message_id, "message-following-text");
    }

    #[test]
    fn attachment_recovery_guidance_preserves_maximum_caption() {
        let caption = "x".repeat(MAX_HERMES_TEXT_BYTES as usize);
        let mut event = HermesPollEventV1::finite_chat_text(
            "room-main",
            1,
            "message-max-caption",
            "alice",
            "ios",
            caption.clone(),
        )
        .expect("maximum caption");

        add_attachment_unavailable_notice(&mut event);

        assert_eq!(event.text, caption);
        assert_eq!(
            event.channel_prompt.as_deref(),
            Some(HERMES_ATTACHMENT_UNAVAILABLE_NOTICE)
        );
        event.validate_limits().expect("bounded recovery guidance");
    }

    #[test]
    fn hermes_activity_bridge_preserves_topic_and_segment_scope() {
        let input = app_bridge_activity_input(HermesActivityRequestV1 {
            room_id: "room-main".to_owned(),
            conversation_id: Some("topic-build".to_owned()),
            segment_id: Some("segment-7".to_owned()),
            thread_id: None,
            activity_kind: finitechat_proto::FINITECHAT_ACTIVITY_KIND_WORKING.to_owned(),
            activity_id: None,
            action: EphemeralActivityActionV1::Set,
            payload: json!({"phase": "tool"}),
            expires_in_millis: 15_000,
        })
        .unwrap();

        assert_eq!(input.conversation_id.as_deref(), Some("topic-build"));
        assert_eq!(input.segment_id.as_deref(), Some("segment-7"));
        assert_eq!(input.payload, br#"{"phase":"tool"}"#);
    }

    #[test]
    fn wrapped_chat_event_conversation_id_reaches_poll_event() {
        let home = tempfile::tempdir().unwrap();
        let hermes_payload = HermesMessagePayloadV1 {
            payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: None,
            segment_id: None,
            text: "topic hello".to_owned(),
            kind: finitechat_hermes::HermesSendKindV1::Message,
            status: HermesMessageStatusV1::Complete,
            edit_of: None,
            attachments: Vec::new(),
            reply_to_message_id: None,
            sender_name: None,
            metadata: BTreeMap::new(),
        }
        .encode()
        .unwrap();
        let wrapped = DecryptedApplicationEventV1 {
            kind: DurableAppEventKind::ChatMessage,
            conversation_id: Some("topic-main".to_owned()),
            segment_id: None,
            payload: hermes_payload,
        };
        let plaintext = serde_json::to_vec(&wrapped).unwrap();
        let event = hermes_poll_event_from_application_plaintext(
            HermesPollEventContext {
                home_dir: home.path(),
                server_url: "https://chat.finite.computer",
                room_id: "room-main",
                seq: 3,
                message_id: "message-3",
                sender_account_id: "alice",
                sender_device_id: "electron",
                conversation_id: None,
                segment_id: None,
            },
            &plaintext,
        )
        .unwrap()
        .expect("topic chat is bridge-visible");
        assert_eq!(event.text, "topic hello");
        assert_eq!(event.conversation_id.as_deref(), Some("topic-main"));
        assert_eq!(event.source.thread_id.as_deref(), Some("topic-main"));
    }

    #[test]
    fn inbox_cursor_redelivers_until_ack_then_blocks_stale_recovery() {
        let home = tempfile::tempdir().unwrap();
        let mut inbox = HermesInboxState::default();
        let first = HermesPollEventV1::finite_chat_text(
            "room-a",
            10,
            "msg-10",
            "account-a",
            "phone",
            "one",
        )
        .unwrap();

        enqueue_hermes_inbox_event(home.path(), &mut inbox, first.clone()).unwrap();
        enqueue_hermes_inbox_event(home.path(), &mut inbox, first.clone()).unwrap();
        assert_eq!(hermes_inbox_cursor(&inbox, "room-a"), 10);
        assert_eq!(pending_hermes_inbox_events(&inbox, None, 10).len(), 1);

        let mut output = Vec::new();
        cmd_ack(
            home.path(),
            serde_json::to_value(HermesAckRequestV1 {
                room_id: "room-a".to_owned(),
                seq: 10,
                message_id: "msg-10".to_owned(),
            })
            .unwrap(),
            &mut output,
        )
        .unwrap();
        let acked: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(acked["acked"], true);

        let mut inbox = load_hermes_inbox(home.path()).unwrap();
        assert_eq!(hermes_inbox_cursor(&inbox, "room-a"), 10);
        assert!(pending_hermes_inbox_events(&inbox, None, 10).is_empty());

        enqueue_hermes_inbox_event(home.path(), &mut inbox, first).unwrap();
        assert!(
            pending_hermes_inbox_events(&inbox, None, 10).is_empty(),
            "an acked seq must not be re-enqueued from durable recovery"
        );

        let second = HermesPollEventV1::finite_chat_text(
            "room-a",
            11,
            "msg-11",
            "account-a",
            "phone",
            "two",
        )
        .unwrap();
        enqueue_hermes_inbox_event(home.path(), &mut inbox, second).unwrap();
        let pending = pending_hermes_inbox_events(&inbox, None, 10);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].message_id, "msg-11");
        assert_eq!(hermes_inbox_cursor(&inbox, "room-a"), 11);
    }

    fn sample_inbox_event(seq: u64, message_id: &str) -> HermesPollEventV1 {
        HermesPollEventV1::finite_chat_text("room-a", seq, message_id, "account-a", "phone", "hi")
            .unwrap()
    }

    #[test]
    fn lease_is_deliverable_respects_pending_and_ttl() {
        assert!(HermesInboxLease::Pending.is_deliverable(100, 50));
        let leased = HermesInboxLease::Leased {
            lease_id: "l".to_owned(),
            leased_at_ms: 100,
        };
        // Fresh lease: not redeliverable.
        assert!(!leased.is_deliverable(120, 50));
        // Lease older than the TTL: redeliverable.
        assert!(leased.is_deliverable(200, 50));
    }

    #[test]
    fn leased_entry_is_not_redelivered_on_the_next_tick() {
        let home = tempfile::tempdir().unwrap();
        let mut inbox = HermesInboxState::default();
        enqueue_hermes_inbox_event(home.path(), &mut inbox, sample_inbox_event(5, "msg-5"))
            .unwrap();

        let first = lease_pending_hermes_inbox_events(home.path(), &mut inbox, None, 10).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].message_id, "msg-5");

        // The entry is now leased; the next tick delivers nothing.
        let second = lease_pending_hermes_inbox_events(home.path(), &mut inbox, None, 10).unwrap();
        assert!(second.is_empty());

        // The lease survives a restart (it is persisted in hermes-inbox.json).
        let reloaded = load_hermes_inbox(home.path()).unwrap();
        assert!(matches!(
            reloaded.events[0].lease,
            HermesInboxLease::Leased { .. }
        ));
        assert!(pending_hermes_inbox_events(&reloaded, None, 10).is_empty());
    }

    #[test]
    fn release_returns_a_leased_entry_to_pending_for_redelivery() {
        let home = tempfile::tempdir().unwrap();
        let mut inbox = HermesInboxState::default();
        enqueue_hermes_inbox_event(home.path(), &mut inbox, sample_inbox_event(5, "msg-5"))
            .unwrap();
        lease_pending_hermes_inbox_events(home.path(), &mut inbox, None, 10).unwrap();

        let mut output = Vec::new();
        cmd_release(
            home.path(),
            serde_json::to_value(HermesAckRequestV1 {
                room_id: "room-a".to_owned(),
                seq: 5,
                message_id: "msg-5".to_owned(),
            })
            .unwrap(),
            &mut output,
        )
        .unwrap();
        let released: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(released["released"], true);

        // Released entries are redelivered on the next tick.
        let inbox = load_hermes_inbox(home.path()).unwrap();
        assert!(matches!(inbox.events[0].lease, HermesInboxLease::Pending));
        let mut inbox = inbox;
        let again = lease_pending_hermes_inbox_events(home.path(), &mut inbox, None, 10).unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].message_id, "msg-5");
    }

    #[test]
    fn stale_lease_is_swept_back_to_pending_and_redelivered() {
        let home = tempfile::tempdir().unwrap();
        let mut inbox = HermesInboxState::default();
        enqueue_hermes_inbox_event(home.path(), &mut inbox, sample_inbox_event(5, "msg-5"))
            .unwrap();
        lease_pending_hermes_inbox_events(home.path(), &mut inbox, None, 10).unwrap();

        // Simulate a lease older than any TTL (a consumer crashed mid-turn).
        inbox.events[0].lease = HermesInboxLease::Leased {
            lease_id: "stale".to_owned(),
            leased_at_ms: 0,
        };
        let swept = lease_pending_hermes_inbox_events(home.path(), &mut inbox, None, 10).unwrap();
        assert_eq!(swept.len(), 1);
        assert_eq!(swept[0].message_id, "msg-5");
    }

    #[test]
    fn duplicate_ack_after_restart_is_an_idempotent_no_op() {
        let home = tempfile::tempdir().unwrap();
        let mut inbox = HermesInboxState::default();
        enqueue_hermes_inbox_event(home.path(), &mut inbox, sample_inbox_event(5, "msg-5"))
            .unwrap();
        lease_pending_hermes_inbox_events(home.path(), &mut inbox, None, 10).unwrap();

        let ack_request = || {
            serde_json::to_value(HermesAckRequestV1 {
                room_id: "room-a".to_owned(),
                seq: 5,
                message_id: "msg-5".to_owned(),
            })
            .unwrap()
        };

        let mut first = Vec::new();
        cmd_ack(home.path(), ack_request(), &mut first).unwrap();
        let first: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(first["acked"], true);

        // Reload as if the sidecar restarted; the entry is gone and its key is
        // in the recently-acked ring.
        let reloaded = load_hermes_inbox(home.path()).unwrap();
        assert!(reloaded.events.is_empty());
        let key = hermes_inbox_key("room-a", 5, "msg-5");
        assert!(inbox_key_recently_acked(&reloaded, &key));

        // A duplicate ack for the same entry is still reported acked.
        let mut second = Vec::new();
        cmd_ack(home.path(), ack_request(), &mut second).unwrap();
        let second: Value = serde_json::from_slice(&second).unwrap();
        assert_eq!(second["acked"], true);
    }

    #[test]
    fn acked_key_blocks_redelivery_even_when_the_cursor_is_behind() {
        let home = tempfile::tempdir().unwrap();
        let mut inbox = HermesInboxState::default();
        let event = sample_inbox_event(5, "msg-5");
        enqueue_hermes_inbox_event(home.path(), &mut inbox, event.clone()).unwrap();
        lease_pending_hermes_inbox_events(home.path(), &mut inbox, None, 10).unwrap();

        let mut output = Vec::new();
        cmd_ack(
            home.path(),
            serde_json::to_value(HermesAckRequestV1 {
                room_id: "room-a".to_owned(),
                seq: 5,
                message_id: "msg-5".to_owned(),
            })
            .unwrap(),
            &mut output,
        )
        .unwrap();

        // Force the ring (not the cursor) to be the thing that blocks a
        // redelivery: rewind the cursor as if durable recovery re-observed the
        // acked seq before the cursor caught up. A crash between the ack write
        // and any downstream dedup must not resurrect the entry.
        let mut reloaded = load_hermes_inbox(home.path()).unwrap();
        reloaded.cursors.insert("room-a".to_owned(), 0);
        enqueue_hermes_inbox_event(home.path(), &mut reloaded, event).unwrap();
        assert!(
            pending_hermes_inbox_events(&reloaded, None, 10).is_empty(),
            "an acked entry must not be re-enqueued even when the cursor is behind"
        );
    }

    fn topic_with_chats(room_id: &str, topic_id: &str, chat_ids: &[&str]) -> AppTopicSummary {
        use finitechat_core::AppChatSummary;
        AppTopicSummary {
            room_id: room_id.to_owned(),
            topic_id: topic_id.to_owned(),
            title: topic_id.to_owned(),
            description: None,
            last_message_preview: String::new(),
            unread_count: 0,
            message_count: 0,
            created_seq: 0,
            updated_seq: 0,
            archived: false,
            active_chat_id: chat_ids.first().map(|id| (*id).to_owned()),
            chats: chat_ids
                .iter()
                .map(|id| AppChatSummary {
                    chat_id: (*id).to_owned(),
                    title: (*id).to_owned(),
                    last_message_preview: String::new(),
                    unread_count: 0,
                    message_count: 0,
                    started_seq: 0,
                    updated_seq: 0,
                    active: false,
                    archived: false,
                })
                .collect(),
        }
    }

    #[test]
    fn resolve_thread_id_route_matches_segment_then_conversation() {
        let topics = vec![topic_with_chats(
            "room-a",
            "topic-build",
            &["chat-build-1", "chat-build-2"],
        )];
        // A segment (chat) id resolves to its parent topic plus itself.
        assert_eq!(
            resolve_thread_id_route(&topics, "room-a", "chat-build-1"),
            Some(("topic-build".to_owned(), Some("chat-build-1".to_owned())))
        );
        // A conversation (topic) id resolves to itself with no segment.
        assert_eq!(
            resolve_thread_id_route(&topics, "room-a", "topic-build"),
            Some(("topic-build".to_owned(), None))
        );
        // An unknown thread id resolves to nothing (the caller then applies the
        // configured unknown-thread policy).
        assert_eq!(
            resolve_thread_id_route(&topics, "room-a", "chat-unknown"),
            None
        );
        // A chat id in another room does not leak across rooms.
        assert_eq!(
            resolve_thread_id_route(&topics, "room-b", "chat-build-1"),
            None
        );
    }

    #[test]
    fn resolve_thread_id_route_ignores_archived_topics() {
        let mut topics = vec![topic_with_chats("room-a", "topic-old", &["chat-old"])];
        topics[0].archived = true;
        assert_eq!(resolve_thread_id_route(&topics, "room-a", "chat-old"), None);
        assert_eq!(
            resolve_thread_id_route(&topics, "room-a", "topic-old"),
            None
        );
    }

    #[test]
    fn unknown_thread_route_policy_defaults_to_home_fallback() {
        // The env var is process-global; this test owns it and restores it.
        // SAFETY: single-threaded mutation of a test-only environment variable.
        unsafe {
            std::env::remove_var(HERMES_UNKNOWN_THREAD_ROUTE_ENV);
        }
        assert!(matches!(
            unknown_thread_route_policy(),
            UnknownThreadRoutePolicy::HomeDefault
        ));
        unsafe {
            std::env::set_var(HERMES_UNKNOWN_THREAD_ROUTE_ENV, "home");
        }
        assert!(matches!(
            unknown_thread_route_policy(),
            UnknownThreadRoutePolicy::HomeDefault
        ));
        unsafe {
            std::env::set_var(HERMES_UNKNOWN_THREAD_ROUTE_ENV, "error");
        }
        assert!(matches!(
            unknown_thread_route_policy(),
            UnknownThreadRoutePolicy::Error
        ));
        unsafe {
            std::env::remove_var(HERMES_UNKNOWN_THREAD_ROUTE_ENV);
        }
    }

    #[test]
    fn agentd_delivery_accepts_only_typed_runtime_commands_and_keeps_chat_separate() {
        let request = RuntimeCommandRequestV1 {
            payload_kind: finitechat_proto::RuntimeCommandPayloadKindV1::Request,
            request_id: "request-1".to_owned(),
            command: "agent.status.inspect".to_owned(),
            target: finitechat_proto::RuntimeCommandTargetV1 {
                account_id: "agent-account".to_owned(),
                device_id: Some("agent-device".to_owned()),
            },
            resource_key: None,
            body: finitechat_proto::RuntimeCommandJsonPayloadV1 {
                schema: "finite.agent.status.request.v1".to_owned(),
                json_payload: br#"{}"#.to_vec(),
            },
        };
        let wrapped = DecryptedApplicationEventV1 {
            kind: DurableAppEventKind::RuntimeCommandRequest,
            conversation_id: Some("topic-main".to_owned()),
            segment_id: None,
            payload: serde_json::to_vec(&request).unwrap(),
        };
        let stored = StoredAppEvent {
            room_id: "room-main".to_owned(),
            seq: 8,
            message_id: "message-8".to_owned(),
            sender: finitechat_proto::DeviceRef::new("user-account", "hosted-web"),
            plaintext: serde_json::to_vec(&wrapped).unwrap(),
            timestamp_unix_seconds: 10,
        };

        let delivery = runtime_command_delivery_from_stored(&stored).unwrap();
        assert_eq!(delivery.room_id, "room-main");
        assert_eq!(delivery.conversation_id.as_deref(), Some("topic-main"));
        assert_eq!(delivery.sender.account_id, "user-account");
        assert!(matches!(
            delivery.payload,
            RuntimeCommandInboundPayloadV1::Request(value)
                if value.command == "agent.status.inspect"
        ));

        let chat = DecryptedApplicationEventV1 {
            kind: DurableAppEventKind::ChatMessage,
            conversation_id: Some("topic-main".to_owned()),
            segment_id: None,
            payload: b"normal chat remains Hermes input".to_vec(),
        };
        let mut chat_stored = stored;
        chat_stored.plaintext = serde_json::to_vec(&chat).unwrap();
        assert!(runtime_command_delivery_from_stored(&chat_stored).is_none());
    }

    #[test]
    fn agentd_inbox_cursor_advances_past_non_command_events_without_losing_pending_commands() {
        let mut inbox = AgentdInboxState::default();
        assert!(advance_agentd_inbox_cursor(&mut inbox, "room-a", 4));
        inbox.events.push(AgentdInboxEvent {
            key: agentd_inbox_key("room-a", 5, "command-5"),
            room_id: "room-a".to_owned(),
            seq: 5,
            message_id: "command-5".to_owned(),
            delivery: RuntimeCommandDeliveryV1 {
                room_id: "room-a".to_owned(),
                conversation_id: None,
                seq: 5,
                message_id: "command-5".to_owned(),
                sender: finitechat_proto::DeviceRef::new("user-account", "hosted-web"),
                payload: RuntimeCommandInboundPayloadV1::Cancel(RuntimeCommandCancelV1 {
                    payload_kind: finitechat_proto::RuntimeCommandPayloadKindV1::Cancel,
                    request_id: "request-5".to_owned(),
                    reason: Some("user_requested".to_owned()),
                }),
            },
        });
        assert!(advance_agentd_inbox_cursor(&mut inbox, "room-a", 5));
        assert!(!advance_agentd_inbox_cursor(&mut inbox, "room-a", 3));
        assert_eq!(agentd_inbox_cursor(&inbox, "room-a"), 5);
        assert_eq!(inbox.events.len(), 1, "cursor movement is not an ack");
    }

    #[test]
    fn inbox_initialization_does_not_consume_first_counterparty_message() {
        let home = tempfile::tempdir().unwrap();
        let mut inbox = HermesInboxState::default();
        let events = [
            StoredAppEvent {
                room_id: "room-a".to_owned(),
                seq: 1,
                message_id: "agent-setup".to_owned(),
                sender: finitechat_proto::DeviceRef::new("agent-account", "agent-device"),
                plaintext: b"agent setup".to_vec(),
                timestamp_unix_seconds: 1,
            },
            StoredAppEvent {
                room_id: "room-a".to_owned(),
                seq: 2,
                message_id: "user-first".to_owned(),
                sender: finitechat_proto::DeviceRef::new("user-account", "electron"),
                plaintext: b"hello agent".to_vec(),
                timestamp_unix_seconds: 2,
            },
            StoredAppEvent {
                room_id: "room-a".to_owned(),
                seq: 3,
                message_id: "agent-after".to_owned(),
                sender: finitechat_proto::DeviceRef::new("agent-account", "agent-device"),
                plaintext: b"agent after".to_vec(),
                timestamp_unix_seconds: 3,
            },
        ];

        initialize_hermes_inbox_cursors_from_events(
            home.path(),
            &mut inbox,
            "agent-account",
            events.iter(),
        )
        .unwrap();
        assert_eq!(
            hermes_inbox_cursor(&inbox, "room-a"),
            1,
            "first run must not advance past unseen counterparty messages"
        );

        let user_event = HermesPollEventV1::finite_chat_text(
            "room-a",
            2,
            "user-first",
            "user-account",
            "electron",
            "hello agent",
        )
        .unwrap();
        enqueue_hermes_inbox_event(home.path(), &mut inbox, user_event).unwrap();
        let pending = pending_hermes_inbox_events(&inbox, None, 10);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].message_id, "user-first");
    }

    #[test]
    fn inbound_ndjson_encodes_joined_accounts_and_events() {
        let payload = json!({
            "joined": ["alice"],
            "events": [
                {
                    "room_id": "room-main",
                    "seq": 7,
                    "message_id": "message-7",
                    "text": "hello"
                }
            ]
        });

        let ndjson = hermes_inbound_ndjson(&payload).expect("encode inbound stream records");
        let lines = ndjson
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["type"], "joined");
        assert_eq!(lines[0]["account_id"], "alice");
        assert_eq!(lines[1]["type"], "event");
        assert_eq!(lines[1]["event"]["message_id"], "message-7");
        assert_eq!(lines[1]["event"]["text"], "hello");
    }

    #[test]
    fn sync_wait_grouping_keeps_same_server_pinned_rooms_on_home_wait() {
        let (home_rooms, remote_rooms) = group_sync_wait_rooms(
            "https://chat.finite.computer",
            vec![
                ("room-home".to_owned(), 4, None),
                (
                    "room-pinned-same".to_owned(),
                    9,
                    Some("https://chat.finite.computer".to_owned()),
                ),
            ],
        );

        assert_eq!(home_rooms.len(), 2);
        assert_eq!(home_rooms[0].room_id, "room-home");
        assert_eq!(home_rooms[1].room_id, "room-pinned-same");
        assert!(remote_rooms.is_empty());
    }

    #[test]
    fn sync_wait_grouping_sends_other_server_rooms_to_that_server() {
        let (home_rooms, remote_rooms) = group_sync_wait_rooms(
            "https://chat.finite.computer",
            vec![(
                "room-remote".to_owned(),
                7,
                Some("https://other.example".to_owned()),
            )],
        );

        assert!(home_rooms.is_empty());
        let rooms = remote_rooms.get("https://other.example").unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].room_id, "room-remote");
        assert_eq!(rooms[0].after_seq, 7);
    }

    #[test]
    fn local_attachment_reader_is_bounded_and_rejects_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let valid = dir.path().join("valid.png");
        fs::write(&valid, b"small image bytes").unwrap();
        assert_eq!(
            read_bounded_hermes_attachment(valid.to_str().unwrap(), 0).unwrap(),
            b"small image bytes"
        );

        let missing = dir.path().join("missing.png");
        let missing_error =
            read_bounded_hermes_attachment(missing.to_str().unwrap(), 0).unwrap_err();
        assert!(missing_error.to_string().contains("could not open"));

        let empty = dir.path().join("empty.png");
        fs::write(&empty, []).unwrap();
        let empty_error = read_bounded_hermes_attachment(empty.to_str().unwrap(), 0).unwrap_err();
        assert!(empty_error.to_string().contains("is empty"));

        let directory_error =
            read_bounded_hermes_attachment(dir.path().to_str().unwrap(), 0).unwrap_err();
        assert!(
            directory_error
                .to_string()
                .contains("is not a regular file")
        );

        let oversized = dir.path().join("oversized.bin");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(u64::from(MAX_ATTACHMENT_PLAINTEXT_BYTES) + 1)
            .unwrap();
        let oversized_error =
            read_bounded_hermes_attachment(oversized.to_str().unwrap(), 0).unwrap_err();
        assert!(oversized_error.to_string().contains("exceeds the"));

        let batch_error = read_bounded_hermes_attachment(
            valid.to_str().unwrap(),
            MAX_HERMES_LOCAL_ATTACHMENT_TOTAL_BYTES - 1,
        )
        .unwrap_err();
        assert!(batch_error.to_string().contains("batch limit"));
    }
}
