//! The `finitechat hermes` subcommand family: the JSON bridge
//! the Hermes platform plugin shells to (ADR 0002), plus agent onboarding
//! (`init` publishes the agent identity and KeyPackages; rooms admit members
//! through MLS add/welcome).
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
use std::sync::{Arc, Mutex};
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
    BlossomDownloadHttpResponse, finish_blossom_download_http_response,
    prepare_blossom_download_http_request, sha256_hex,
};
use finitechat_client::{
    FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery, ReqwestHttpRuntimeTransport,
    SqliteClientStore, SqliteClientStoreOptions, StoredAppEvent,
};
use finitechat_core::{
    AppAction, AppBridgeActivityInput, AppRoomState, AppSentMessage, AppState, FiniteChatCoreError,
    FiniteChatRuntime, OpenOptions,
};
use finitechat_hermes::{
    HermesAckRequestV1, HermesActivityRequestV1, HermesEditRequestV1, HermesMessagePayloadV1,
    HermesMessageStatusV1, HermesPollEventV1, HermesSendRequestV1, MAX_HERMES_POLL_TIMEOUT_MILLIS,
};
use finitechat_http::{NostrProfileRecord, SyncWaitRequest, SyncWaitRoom};
use finitechat_mls::NostrSecretKey;
use finitechat_proto::{
    AttachmentBlobReferenceV1, DecryptedApplicationEventV1, DurableAppEventKind,
    EphemeralActivityActionV1, npub_encode,
};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::CliError;

const CONFIG_FILE: &str = "config.json";
const HERMES_INBOX_FILE: &str = "hermes-inbox.json";
const HERMES_RUNNING_FILE: &str = "hermes-running.json";
const HERMES_HOME_CHANNEL_FILE: &str = "hermes-home-channel.json";
const BACKUP_ACTIVITY_FILE: &str = ".finitechat-backup-active";
const STORE_FILE: &str = "client.sqlite3";
const ATTACHMENT_CACHE_DIR: &str = "attachments";
const HERMES_PLUGIN_INSTALL_NAME: &str = "finitechat";
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
const DEFAULT_HERMES_SERVICE_ADDR: &str = "127.0.0.1:0";
const DEFAULT_DEVICE_ID: &str = "agent";
const DEFAULT_AGENT_PROFILE_NAME: &str = "Finite Agent";
const DEFAULT_AGENT_PROFILE_ABOUT: &str = "A Finite Computer agent you can chat with.";
const DEFAULT_AGENT_PROFILE_PICTURE: &str = "https://avatars.githubusercontent.com/u/274919006?v=4";
const CREDENTIAL_VALIDITY_SECONDS: u64 = 90 * 24 * 60 * 60;
const POLL_SLEEP_MS: u64 = 300;
const HERMES_STORED_EVENT_RECOVERY_LIMIT: u32 = 5_000;
const HERMES_SERVICE_HEARTBEAT_MILLIS: u64 = 250;

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

pub(crate) fn run<W: Write>(args: Vec<String>, output: &mut W) -> Result<(), CliError> {
    let mut args = args;
    let home_dir = resolve_home(&mut args)?;
    let json_mode = take_flag(&mut args, "--json");
    let request_json = crate::take_option(&mut args, "--request-json")?;
    let Some(command) = args.first().cloned() else {
        return Err(CliError::Usage(hermes_usage()));
    };
    let rest = args[1..].to_vec();

    match command.as_str() {
        "init" => cmd_init(&home_dir, rest, output),
        "install" => cmd_install(&home_dir, rest, json_mode, output),
        "serve" => cmd_serve(&home_dir, rest, json_mode, output),
        "home-channel" => cmd_home_channel(&home_dir, rest, output),
        "room-status" => cmd_room_status(&home_dir, rest, json_mode, output),
        "poll" => with_backup_activity(&home_dir, "poll", || {
            cmd_poll(&home_dir, read_request(request_json)?, output)
        }),
        "ack" => with_backup_activity(&home_dir, "ack", || {
            cmd_ack(&home_dir, read_request(request_json)?, output)
        }),
        "send" => with_backup_activity(&home_dir, "send", || {
            cmd_send(&home_dir, read_request(request_json)?, output)
        }),
        "edit" => with_backup_activity(&home_dir, "edit", || {
            cmd_edit(&home_dir, read_request(request_json)?, output)
        }),
        "recover" => with_backup_activity(&home_dir, "recover", || {
            cmd_recover(&home_dir, read_request(request_json)?, output)
        }),
        "activity" => with_backup_activity(&home_dir, "activity", || {
            cmd_activity(&home_dir, read_request(request_json)?, output)
        }),
        _ => Err(CliError::Usage(hermes_usage())),
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
    mut args: Vec<String>,
    json_mode: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let plugin_dir_arg = crate::take_option(&mut args, "--plugin-dir")?;
    let plugins_dir_arg = crate::take_option(&mut args, "--plugins-dir")?;
    let plugin_name = crate::take_option(&mut args, "--plugin-name")?
        .unwrap_or_else(|| HERMES_PLUGIN_INSTALL_NAME.to_owned());
    let finitechat_bin_arg = crate::take_option(&mut args, "--finitechat-bin")?;
    let service_url = crate::take_option(&mut args, "--service-url")?;
    let force = take_flag(&mut args, "--force");
    crate::reject_extra_args(&args)?;

    validate_plugin_name(&plugin_name)?;
    if plugin_dir_arg.is_some() && plugins_dir_arg.is_some() {
        return Err(CliError::Usage(
            "pass either --plugin-dir or --plugins-dir, not both".to_owned(),
        ));
    }
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
    mut args: Vec<String>,
    json_mode: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let addr = crate::take_option(&mut args, "--addr")?
        .unwrap_or_else(|| DEFAULT_HERMES_SERVICE_ADDR.to_owned())
        .parse::<SocketAddr>()
        .map_err(|error| CliError::Usage(format!("invalid --addr: {error}")))?;
    let ready_file = crate::take_option(&mut args, "--ready-file")?.map(PathBuf::from);
    crate::reject_extra_args(&args)?;

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
    let state = HermesServiceState {
        agent_home: home_dir.to_path_buf(),
        account_id: home.config.account_id.clone(),
        device_id: home.config.device_id.clone(),
        server_url: home.config.server_url.clone(),
        runtime,
        inbox_lock: Arc::new(Mutex::new(())),
        running_lock: Arc::new(Mutex::new(())),
    };
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
        .with_state(state)
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

fn handle_hermes_service_action(
    state: &HermesServiceState,
    action: &str,
    payload: Value,
) -> Result<Value, CliError> {
    match action {
        "poll" => handle_hermes_service_poll(state, payload),
        "ack" => {
            let _guard = lock_service_mutex(&state.inbox_lock)?;
            output_json_value(|output| cmd_ack(&state.agent_home, payload, output))
        }
        "send" => {
            let request: HermesSendRequestV1 =
                serde_json::from_value(payload).map_err(CliError::Json)?;
            let _guard = lock_service_mutex(&state.running_lock)?;
            let sent = send_hermes_request_with_runtime(&state.runtime, &request)?;
            update_running_after_send(&state.agent_home, &request, &sent.message_id)?;
            Ok(sent_message_value(&sent))
        }
        "edit" => {
            let request: HermesEditRequestV1 =
                serde_json::from_value(payload).map_err(CliError::Json)?;
            let _guard = lock_service_mutex(&state.running_lock)?;
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
        let payload = collect_hermes_service_inbound_payload(state, &home, &request, None)?;
        if hermes_inbound_payload_has_records(&payload) || started.elapsed() >= timeout {
            return Ok(payload);
        }

        let remaining = timeout.saturating_sub(started.elapsed()).as_millis() as u64;
        let bridge = wait_for_hermes_bridge_update_or_poll(state, remaining)?;
        let payload = collect_hermes_service_inbound_payload(state, &home, &request, Some(bridge))?;
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
            message_id: message.message_id.clone(),
            text: "Hermes gateway restarted before this turn completed.".to_owned(),
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
    let request: HermesActivityRequestV1 =
        serde_json::from_value(payload).map_err(CliError::Json)?;
    let activity_payload = if matches!(request.action, EphemeralActivityActionV1::Set) {
        serde_json::to_vec(&request.payload).map_err(CliError::Serialize)?
    } else {
        Vec::new()
    };
    let accepted = state
        .runtime
        .append_ephemeral_activity_and_wait(AppBridgeActivityInput {
            room_id: request.room_id,
            conversation_id: request.conversation_id,
            activity_kind: request.activity_kind,
            activity_id: request.activity_id,
            action: request.action,
            payload: activity_payload,
            expires_in_millis: request.expires_in_millis,
        })
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
        let payload = collect_hermes_service_inbound_payload(&state, &home, &request, None)?;
        if !send_hermes_inbound_payload(&tx, &payload)? {
            return Ok(());
        }
        if hermes_inbound_payload_has_records(&payload) {
            continue;
        }

        let bridge = wait_for_hermes_bridge_update_or_poll(&state, timeout_millis)?;
        let payload =
            collect_hermes_service_inbound_payload(&state, &home, &request, Some(bridge))?;
        if !send_hermes_inbound_payload(&tx, &payload)? {
            return Ok(());
        }
    }
}

fn wait_for_hermes_bridge_update_or_poll(
    state: &HermesServiceState,
    timeout_millis: u64,
) -> Result<finitechat_core::AppBridgeSync, CliError> {
    match state.runtime.agent_bridge_wait_for_update(timeout_millis) {
        Ok(bridge) => Ok(bridge),
        Err(_) => {
            let fallback_sleep_ms = timeout_millis.min(POLL_SLEEP_MS);
            if fallback_sleep_ms > 0 {
                std::thread::sleep(Duration::from_millis(fallback_sleep_ms));
            }
            state
                .runtime
                .agent_bridge_poll_once()
                .map_err(map_core_hermes_error)
        }
    }
}

fn collect_hermes_service_inbound_payload(
    state: &HermesServiceState,
    home: &AgentHome,
    request: &PollRequest,
    bridge: Option<finitechat_core::AppBridgeSync>,
) -> Result<Value, CliError> {
    let limit = normalized_hermes_poll_limit(request);
    let _guard = lock_service_mutex(&state.inbox_lock)?;
    let mut inbox = load_hermes_inbox(&state.agent_home)?;
    initialize_hermes_inbox_cursors(&state.agent_home, home, &mut inbox)?;
    let mut joined = Vec::<String>::new();

    if let Some(bridge) = bridge {
        joined = bridge.joined_account_ids;
        for applied in &bridge.events {
            if let Some(room_filter) = &request.room_id
                && room_filter != &applied.room_id
            {
                continue;
            }
            if applied.sender_account_id == state.account_id {
                continue;
            }
            let context = HermesPollEventContext {
                home_dir: &state.agent_home,
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
                enqueue_hermes_inbox_event(&state.agent_home, &mut inbox, event)?;
            }
        }
    }

    recover_stored_hermes_events(
        &state.agent_home,
        home,
        &state.account_id,
        request.room_id.as_deref(),
        &mut inbox,
    )?;
    let events = pending_hermes_inbox_events(&inbox, request.room_id.as_deref(), limit);
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
    service_error(
        status,
        cli_error_kind(&error),
        cli_error_retryable(&error),
        error.to_string(),
    )
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

fn cli_error_kind(error: &CliError) -> &'static str {
    match error {
        CliError::Usage(_) => "usage",
        CliError::Serialize(_) => "serialize",
        CliError::Json(_) => "json",
        CliError::Http(_) => "http",
        CliError::Server { .. } => "server",
        CliError::Output(_) => "output",
        CliError::Hermes(_) => "hermes",
        CliError::Identity(_) => "identity",
        CliError::Runtime(_) => "runtime",
    }
}

fn cli_error_retryable(error: &CliError) -> bool {
    match error {
        CliError::Http(_) => true,
        CliError::Server { status, .. } => {
            status.is_server_error()
                || *status == reqwest::StatusCode::REQUEST_TIMEOUT
                || *status == reqwest::StatusCode::TOO_MANY_REQUESTS
        }
        CliError::Usage(_)
        | CliError::Serialize(_)
        | CliError::Json(_)
        | CliError::Output(_)
        | CliError::Hermes(_)
        | CliError::Identity(_)
        | CliError::Runtime(_) => false,
    }
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
    mut args: Vec<String>,
    output: &mut W,
) -> Result<(), CliError> {
    let Some(command) = args.first().cloned() else {
        return Err(CliError::Usage(hermes_usage()));
    };
    let rest = args.split_off(1);
    match command.as_str() {
        "show" => {
            crate::reject_extra_args(&rest)?;
            write_home_channel_show(home_dir, output)
        }
        "set" => cmd_home_channel_set(home_dir, rest, output),
        "clear" => {
            crate::reject_extra_args(&rest)?;
            clear_home_channel(home_dir)?;
            crate::write_pretty_json(output, &json!({ "cleared": true, "home_channel": null }))
        }
        _ => Err(CliError::Usage(hermes_usage())),
    }
}

fn cmd_home_channel_set<W: Write>(
    home_dir: &Path,
    mut args: Vec<String>,
    output: &mut W,
) -> Result<(), CliError> {
    let room_id = crate::required_option(&mut args, "--room-id")?;
    let conversation_id = crate::take_option(&mut args, "--conversation-id")?;
    crate::reject_extra_args(&args)?;
    set_home_channel(home_dir, room_id, conversation_id, output)
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
    mut args: Vec<String>,
    output: &mut W,
) -> Result<(), CliError> {
    let server_url = crate::required_option(&mut args, "--server")?;
    let device_id = crate::take_option(&mut args, "--device-id")?
        .unwrap_or_else(|| DEFAULT_DEVICE_ID.to_owned());
    let agent_name = crate::take_option(&mut args, "--agent-name")?
        .unwrap_or_else(|| DEFAULT_AGENT_PROFILE_NAME.to_owned());
    let agent_about = crate::take_option(&mut args, "--agent-about")?
        .unwrap_or_else(|| DEFAULT_AGENT_PROFILE_ABOUT.to_owned());
    let agent_picture = crate::take_option(&mut args, "--agent-picture-url")?
        .unwrap_or_else(|| DEFAULT_AGENT_PROFILE_PICTURE.to_owned());
    let skip_agent_profile = take_flag(&mut args, "--skip-agent-profile");
    crate::reject_extra_args(&args)?;
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
    mut args: Vec<String>,
    json_mode: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let room_id = crate::required_option(&mut args, "--room-id")?;
    crate::reject_extra_args(&args)?;

    let home = load_home(home_dir)?;
    let runtime = open_agent_runtime(&home)?;
    runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(map_core_hermes_error)?;
    let state = runtime
        .dispatch_and_wait(AppAction::OpenRoom {
            room_id: room_id.clone(),
        })
        .map_err(map_core_hermes_error)?;
    let summary = hermes_room_status_summary(&state, &room_id);

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

fn hermes_room_status_summary(state: &AppState, room_id: &str) -> HermesRoomStatusSummary {
    let room = state.rooms.iter().find(|room| room.room_id == room_id);
    let connected = room
        .map(|room| room.state == AppRoomState::Connected)
        .unwrap_or(false);
    let details = state
        .room_details
        .as_ref()
        .filter(|details| details.room_id == room_id);
    let member_count = details.map(|details| details.members.len()).unwrap_or(0) as u32;
    let other_member_count = details
        .map(|details| {
            details
                .members
                .iter()
                .filter(|member| !member.current_device)
                .count() as u32
        })
        .unwrap_or(0);
    let has_counterparty_messages = state
        .messages
        .iter()
        .any(|message| message.room_id == room_id && !message.is_mine);

    HermesRoomStatusSummary {
        room_id: room_id.to_owned(),
        state: room
            .map(|room| app_room_state_label(&room.state).to_owned())
            .unwrap_or_else(|| "unknown".to_owned()),
        status: room
            .map(|room| room.status.clone())
            .unwrap_or_else(|| "not_found".to_owned()),
        connected,
        paired: connected && (other_member_count > 0 || has_counterparty_messages),
        member_count,
        other_member_count,
    }
}

fn app_room_state_label(state: &AppRoomState) -> &'static str {
    match state {
        AppRoomState::Connected => "connected",
        AppRoomState::WaitingForApproval => "waiting_for_approval",
        AppRoomState::Joining => "joining",
        AppRoomState::UnavailableOnDevice => "unavailable_on_device",
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HermesInboxEvent {
    key: String,
    room_id: String,
    seq: u64,
    message_id: String,
    created_at_ms: u64,
    event: HermesPollEventV1,
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
    let mut events = pending_hermes_inbox_events(&inbox, request.room_id.as_deref(), limit);
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
        events = pending_hermes_inbox_events(&inbox, request.room_id.as_deref(), limit);

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
    if inbox.events.len() != before {
        save_hermes_inbox(home_dir, &inbox)?;
    }
    crate::write_pretty_json(
        output,
        &json!({ "acked": inbox.events.len() != before, "room_id": request.room_id, "seq": request.seq, "message_id": request.message_id }),
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
    advance_hermes_inbox_cursor(inbox, &event.room_id, event.seq);
    inbox.events.push(HermesInboxEvent {
        key,
        room_id: event.room_id.clone(),
        seq: event.seq,
        message_id: event.message_id.clone(),
        created_at_ms: now_ms(),
        event,
    });
    save_hermes_inbox(home_dir, inbox)
}

fn initialize_hermes_inbox_cursors(
    home_dir: &Path,
    home: &AgentHome,
    inbox: &mut HermesInboxState,
) -> Result<(), CliError> {
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
                .map_or(true, |seq| event.seq < *seq)
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
    let mut cursor_changed = false;
    for stored in load_recent_agent_app_events(home)? {
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

fn pending_hermes_inbox_events(
    inbox: &HermesInboxState,
    room_filter: Option<&str>,
    limit: usize,
) -> Vec<HermesPollEventV1> {
    inbox
        .events
        .iter()
        .filter(|entry| match room_filter {
            Some(room_id) => room_id == entry.room_id,
            None => true,
        })
        .take(limit)
        .map(|entry| entry.event.clone())
        .collect()
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
    if let Some(payload) = HermesMessagePayloadV1::decode(payload)
        .map_err(|error| CliError::Hermes(error.to_string()))?
    {
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
        materialize_poll_event_attachments(context.home_dir, &mut event)?;
        return Ok(Some(event));
    }

    if typed_chat_message && payload_is_typed_json(payload) {
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
    event: &mut HermesPollEventV1,
) -> Result<(), CliError> {
    for attachment in &mut event.attachments {
        if attachment.path.is_some() {
            continue;
        }
        let Some(reference) = attachment.blob.clone() else {
            continue;
        };
        let path = materialize_blob_attachment(home_dir, &reference)?;
        attachment.path = Some(path.to_string_lossy().into_owned());
    }
    Ok(())
}

fn materialize_blob_attachment(
    home_dir: &Path,
    reference: &AttachmentBlobReferenceV1,
) -> Result<PathBuf, CliError> {
    let path = hermes_attachment_cache_path(home_dir, reference);
    if let Ok(existing) = fs::read(&path)
        && existing.len() as u64 == reference.plaintext_size
        && sha256_hex(&existing) == reference.plaintext_sha256
    {
        return Ok(path);
    }

    let request = prepare_blossom_download_http_request(reference)
        .map_err(|error| CliError::Hermes(error.to_string()))?;
    let response = reqwest::blocking::Client::new()
        .get(request.url)
        .send()
        .map_err(|error| CliError::Hermes(format!("attachment download failed: {error}")))?;
    let status = response.status().as_u16();
    let body = response
        .bytes()
        .map_err(|error| CliError::Hermes(format!("attachment download failed: {error}")))?
        .to_vec();
    let downloaded = finish_blossom_download_http_response(
        reference,
        BlossomDownloadHttpResponse {
            status,
            body: &body,
        },
    )
    .map_err(|error| CliError::Hermes(format!("attachment verification failed: {error}")))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| CliError::Hermes(error.to_string()))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &downloaded.plaintext).map_err(|error| CliError::Hermes(error.to_string()))?;
    fs::rename(&tmp, &path).map_err(|error| CliError::Hermes(error.to_string()))?;
    Ok(path)
}

fn hermes_attachment_cache_path(home_dir: &Path, reference: &AttachmentBlobReferenceV1) -> PathBuf {
    home_dir
        .join(ATTACHMENT_CACHE_DIR)
        .join(&reference.plaintext_sha256)
        .join(sanitized_attachment_filename(&reference.metadata.filename))
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

fn payload_is_typed_json(payload: &[u8]) -> bool {
    serde_json::from_slice::<Value>(payload)
        .ok()
        .and_then(|value| value.get("type").and_then(Value::as_str).map(str::to_owned))
        .is_some()
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
    let request: HermesSendRequestV1 = serde_json::from_value(request).map_err(CliError::Json)?;
    let home = load_home(home_dir)?;
    let runtime = open_agent_runtime(&home)?;
    runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(map_core_hermes_error)?;
    let sent = send_hermes_request_with_runtime(&runtime, &request)?;
    update_running_after_send(home_dir, &request, &sent.message_id)?;
    write_sent_message(output, &sent)
}

fn cmd_edit<W: Write>(home_dir: &Path, request: Value, output: &mut W) -> Result<(), CliError> {
    let request: HermesEditRequestV1 = serde_json::from_value(request).map_err(CliError::Json)?;
    let home = load_home(home_dir)?;
    let runtime = open_agent_runtime(&home)?;
    runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(map_core_hermes_error)?;
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
            message_id: message.message_id.clone(),
            text: "Hermes gateway restarted before this turn completed.".to_owned(),
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

fn send_hermes_request_with_runtime(
    runtime: &FiniteChatRuntime,
    request: &HermesSendRequestV1,
) -> Result<AppSentMessage, CliError> {
    let hermes_payload = HermesMessagePayloadV1::from_send(request)
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
    let request: HermesActivityRequestV1 =
        serde_json::from_value(request).map_err(CliError::Json)?;
    let home = load_home(home_dir)?;
    let payload = if matches!(request.action, EphemeralActivityActionV1::Set) {
        serde_json::to_vec(&request.payload).map_err(CliError::Serialize)?
    } else {
        Vec::new()
    };
    let runtime = open_agent_runtime(&home)?;
    runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(map_core_hermes_error)?;
    let accepted = runtime
        .append_ephemeral_activity_and_wait(AppBridgeActivityInput {
            room_id: request.room_id,
            conversation_id: request.conversation_id,
            activity_kind: request.activity_kind,
            activity_id: request.activity_id,
            action: request.action,
            payload,
            expires_in_millis: request.expires_in_millis,
        })
        .map_err(map_core_hermes_error)?;
    crate::write_pretty_json(output, &json!({ "accepted": true, "result": accepted }))
}

// --- agent home plumbing ---

/// Resolve the agent home (durable agent state — never the identity, whose
/// location is fixed by the Finite Identity Contract).
fn resolve_home(args: &mut Vec<String>) -> Result<PathBuf, CliError> {
    resolve_home_with(
        args,
        |name| std::env::var_os(name).map(PathBuf::from),
        || std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn resolve_home_with(
    args: &mut Vec<String>,
    env_path: impl Fn(&str) -> Option<PathBuf>,
    home_dir: impl Fn() -> Option<PathBuf>,
) -> Result<PathBuf, CliError> {
    if let Some(path) = crate::take_option(args, "--agent-home")? {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = crate::take_option(args, "--home")? {
        return Ok(PathBuf::from(path));
    }
    for name in ["FINITE_AGENT_HOME", "FINITECHAT_HOME"] {
        if let Some(path) = env_path(name) {
            return Ok(path);
        }
    }
    let Some(home) = home_dir() else {
        return Err(CliError::Usage(
            "pass --agent-home DIR, set FINITE_AGENT_HOME, or set HOME".to_owned(),
        ));
    };
    Ok(home.join(".finite").join("agent"))
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
    FiniteChatRuntime::open(OpenOptions {
        data_dir: home.dir.to_string_lossy().into_owned(),
        server_url: home.config.server_url.clone(),
        device_id: home.config.device_id.clone(),
        account_secret_hex: Some(hex_lower(home.secret.as_bytes())),
        now_unix_seconds: Some(now_secs()),
    })
    .map_err(map_core_hermes_error)
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
    fs::write(&path, contents).map_err(|error| CliError::Hermes(error.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| CliError::Hermes(error.to_string()))?;
    }
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

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(index) = args.iter().position(|arg| arg == name) {
        args.remove(index);
        return true;
    }
    false
}

pub(crate) fn hermes_usage() -> String {
    "hermes commands:\n  finitechat hermes [--agent-home DIR] init --server URL [--device-id ID] [--agent-name NAME] [--agent-about TEXT] [--agent-picture-url URL] [--skip-agent-profile]\n  finitechat hermes [--agent-home DIR] install [--plugins-dir DIR | --plugin-dir DIR] [--plugin-name NAME] [--finitechat-bin PATH] [--service-url URL] [--force] [--json]\n  finitechat hermes [--agent-home DIR] serve [--addr HOST:PORT] [--ready-file PATH] [--json]\n  finitechat hermes [--agent-home DIR] home-channel show|clear\n  finitechat hermes [--agent-home DIR] home-channel set --room-id ID [--conversation-id ID]\n  finitechat hermes [--agent-home DIR] room-status --room-id ID [--json]\n  finitechat hermes [--agent-home DIR] poll --json   (stdin: {room_id?, limit?, timeout_millis?})\n  finitechat hermes [--agent-home DIR] ack --json    (stdin: HermesAckRequestV1)\n  finitechat hermes [--agent-home DIR] send --json   (stdin: HermesSendRequestV1)\n  finitechat hermes [--agent-home DIR] edit --json   (stdin: HermesEditRequestV1)\n  finitechat hermes [--agent-home DIR] recover --json\n  finitechat hermes [--agent-home DIR] activity --json (stdin: HermesActivityRequestV1)\n  (--home is accepted as a compatibility alias; FINITE_AGENT_HOME, FINITECHAT_HOME, or ~/.finite/agent may replace --agent-home; the account key is the shared Finite identity under $FINITE_HOME/identity or ~/.finite/identity — see finitechat auth; --request-json JSON may replace stdin)".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn app_state_with_room_members(member_current_device_flags: Vec<bool>) -> AppState {
        serde_json::from_value(json!({
            "rev": 1,
            "identity": {
                "account_id": "agent-account",
                "device_id": "agent-device",
                "account_secret_hex": "00"
            },
            "rooms": [{
                "room_id": "room-main",
                "display_name": "Main",
                "picture": null,
                "state": "Connected",
                "status": "connected",
                "user_status_text": "Connected",
                "last_message_preview": "",
                "unread_count": 0,
                "can_load_older": false,
                "is_agent_chat": false
            }],
            "selected_room_id": "room-main",
            "topics": [],
            "selected_topic_id": null,
            "active_profile_id": null,
            "status": "ready",
            "toast": null,
            "messages": [],
            "media_gallery": null,
            "room_details": {
                "room_id": "room-main",
                "display_name": "Main",
                "picture": null,
                "state": "Connected",
                "status": "connected",
                "user_status_text": "Connected",
                "media_item_count": 0,
                "members": member_current_device_flags
                    .into_iter()
                    .enumerate()
                    .map(|(index, current_device)| {
                        json!({
                            "account_id": format!("account-{index}"),
                            "device_id": format!("device-{index}"),
                            "npub": format!("npub-{index}"),
                            "display_name": format!("Member {index}"),
                            "picture": null,
                            "current_device": current_device
                        })
                    })
                    .collect::<Vec<_>>(),
                "devices": []
            },
            "profiles": [],
            "devices": [],
            "typing_members": [],
            "flow": {
                "notice_text": null,
                "notice_busy": false,
                "scan_in_flight": false,
                "scan_result": "None",
                "image_upload_url": null
            }
        }))
        .unwrap()
    }

    #[test]
    fn room_status_summary_pairs_connected_room_with_other_member() {
        let state = app_state_with_room_members(vec![true, false]);

        let summary = hermes_room_status_summary(&state, "room-main");

        assert!(summary.connected);
        assert!(summary.paired);
        assert_eq!(summary.member_count, 2);
        assert_eq!(summary.other_member_count, 1);
        assert_eq!(summary.state, "connected");
    }

    #[test]
    fn room_status_summary_does_not_pair_without_other_member() {
        let state = app_state_with_room_members(vec![true]);

        let summary = hermes_room_status_summary(&state, "room-main");

        assert!(summary.connected);
        assert!(!summary.paired);
        assert_eq!(summary.member_count, 1);
        assert_eq!(summary.other_member_count, 0);
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
            vec![
                "--plugin-dir".to_owned(),
                plugin_dir.display().to_string(),
                "--finitechat-bin".to_owned(),
                "/usr/local/bin/finitechat".to_owned(),
                "--service-url".to_owned(),
                "http://127.0.0.1:4321".to_owned(),
            ],
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
        assert!(plugin_dir.join("plugin.yaml").exists());
        assert!(plugin_dir.join(HERMES_PLUGIN_ENV_FILE).exists());

        let plugin_yaml = fs::read_to_string(plugin_dir.join("plugin.yaml")).unwrap();
        assert!(plugin_yaml.lines().any(|line| line == "name: finitechat"));
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
            vec![
                "--plugins-dir".to_owned(),
                plugins_dir.display().to_string(),
                "--finitechat-bin".to_owned(),
                "/usr/local/bin/finitechat".to_owned(),
            ],
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
        let args = vec![
            "--plugin-dir".to_owned(),
            plugin_dir.display().to_string(),
            "--finitechat-bin".to_owned(),
            "/bin/finitechat".to_owned(),
        ];

        let mut output = Vec::new();
        cmd_install(&home, args.clone(), true, &mut output).expect("first install");
        output.clear();
        cmd_install(&home, args.clone(), true, &mut output).expect("same install is idempotent");

        fs::write(plugin_dir.join("adapter.py"), "# local edit\n").unwrap();
        let error = cmd_install(&home, args.clone(), true, &mut output)
            .expect_err("local edit requires --force");
        assert!(error.to_string().contains("--force"));

        let mut force_args = args;
        force_args.push("--force".to_owned());
        cmd_install(&home, force_args, true, &mut output).expect("force overwrites managed file");
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
            vec![
                "--plugin-dir".to_owned(),
                plugin_dir.display().to_string(),
                "--finitechat-bin".to_owned(),
                "/bin/finitechat".to_owned(),
            ],
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
            vec![
                "--server".to_owned(),
                "http://127.0.0.1:1".to_owned(),
                "--skip-agent-profile".to_owned(),
            ],
            &mut output,
        )
        .expect("init");
        output.clear();

        let error = cmd_home_channel_set(
            &home,
            vec!["--room-id".to_owned(), "missing-room".to_owned()],
            &mut output,
        )
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

    #[test]
    fn inbox_initialization_does_not_consume_first_counterparty_message() {
        let home = tempfile::tempdir().unwrap();
        let mut inbox = HermesInboxState::default();
        let events = vec![
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
}
