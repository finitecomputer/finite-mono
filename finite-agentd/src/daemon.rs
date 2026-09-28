use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use finitechat_proto::{
    DeviceRef, RuntimeCommandDeliveryV1, RuntimeCommandErrorV1, RuntimeCommandInboundPayloadV1,
    RuntimeCommandJsonPayloadV1, RuntimeCommandPayloadKindV1, RuntimeCommandRequestV1,
    RuntimeCommandResultDeliveryV1, RuntimeCommandResultV1, RuntimeCommandTerminalStatusV1,
    RuntimeStateSnapshotDeliveryV1, RuntimeStateSnapshotV1,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tokio::sync::mpsc;

use crate::AgentdError;
use crate::codex::CodexState;
use crate::config::{
    ConfigApplyResultV1, ConfigManager, HermesConfigOfferV1, HermesConfigRollbackV1,
    MODEL_CONFIG_PATH,
};
use crate::connections::{
    ConnectionManager, GoogleApplyRequest, InferenceApplyPlan, InferenceApplyRequest,
    PairingApproveRequest, TelegramConnectRequest, TelegramHomeRequest, validate_model_name,
};
use crate::executor::{Executor, ExecutorHost};
use crate::facts::{CodexStateFact, FactsCache, InferenceFacts, PoolEntries, Tri};
use crate::hosted_hermes::{HostedHermesHandle, ServeGate};
use crate::inference::{
    FinitePrivateEnv, SavedRoute, capabilities, classify_saved_route, finite_private_env,
    plan_model_block,
};
use crate::intent::{
    self, Admission, AdmitCommand, IntentKind, IntentRecord, IntentRoute, IntentState,
};
use crate::ledger::{CommandDecision, Ledger};
use crate::openrouter::{ConnectCredential, OpenRouterState, SavedKey};
use crate::supervisor::{ProcessSpec, SupervisorHandle, SupervisorStatus, start_supervisor};
use crate::transport::BridgeClient;

const STATUS_SCHEMA: &str = "finite.agent.status.v1";
const STATUS_REQUEST_SCHEMA: &str = "finite.agent.status.request.v1";
const EMPTY_REQUEST_SCHEMA: &str = "finite.agent.empty.request.v1";
const RESULT_SCHEMA: &str = "finite.agent.command.result.v1";
const OWNER_CLAIM_COMMAND: &str = "agent.owner.claim";
const INFERENCE_APPLY_SCHEMA: &str = "finite.agent.inference.apply.v1";
const INFERENCE_SELECT_SCHEMA: &str = "finite.agent.inference.select.v1";
const INFERENCE_DISCONNECT_SCHEMA: &str = "finite.agent.inference.disconnect.v1";
const OPENROUTER_CONNECT_SCHEMA: &str = "finite.agent.openrouter.connect.v1";
const CODEX_LOGIN_START_SCHEMA: &str = "finite.agent.codex.login.start.v1";
const CODEX_LOGIN_CANCEL_SCHEMA: &str = "finite.agent.codex.login.cancel.v1";
/// The nine commands of the inference contract (§3.12). Each dispatch arm
/// calls `intent::admit` after its schema check and before any other work.
const INFERENCE_COMMANDS: [&str; 9] = [
    "agent.connections.status",
    "agent.inference.apply",
    "agent.inference.select",
    "agent.inference.disconnect",
    "agent.openrouter.usage",
    "agent.openrouter.connect",
    "agent.codex.login.start",
    "agent.codex.login.cancel",
    "agent.codex.models",
];
const INTENT_WRITE_FAILED: &str = "The agent couldn't record this change.";
const TELEGRAM_CONNECT_SCHEMA: &str = "finite.agent.telegram.connect.v1";
const TELEGRAM_APPROVE_SCHEMA: &str = "finite.agent.telegram.approve.v1";
const TELEGRAM_HOME_SCHEMA: &str = "finite.agent.telegram.home.v1";
const GOOGLE_APPLY_SCHEMA: &str = "finite.agent.google.apply.v1";
const BRIDGE_READY_TIMEOUT_ENV: &str = "FINITE_AGENTD_BRIDGE_READY_TIMEOUT_SECS";

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub agent_home: PathBuf,
    pub hermes_home: PathBuf,
    pub bridge_url: String,
    pub bridge_addr: String,
    pub finitechat_bin: PathBuf,
    pub prepare_command: PathBuf,
    pub hermes_command: PathBuf,
    pub health_python: PathBuf,
    pub health_script: PathBuf,
    pub authorized_accounts: BTreeSet<String>,
    /// Admission-default marker exported for the supervised chat sidecar.
    /// agentd only runs inside the container, so the marker is what
    /// distinguishes a hosted sidecar from a standalone `finitechat hermes
    /// serve`: `locked` makes a sidecar with no persisted admission policy
    /// row default to allowlist mode (seeded from the owner list and the
    /// existing rooms' counterparties) instead of the legacy allow-all.
    /// `None` when an explicit `FINITECHAT_ADMISSION_DEFAULT` is already set
    /// (the sidecar inherits it unchanged). The sidecar inherits
    /// `FINITECHAT_OWNER_NPUBS` and `FINITECHAT_WELCOME_ALLOWLIST` directly
    /// from the container environment — agentd never derives admission
    /// values; it only marks hostedness.
    pub sidecar_admission_default: Option<String>,
    /// How long startup waits for the Finite Chat bridge to serve readiness
    /// before failing the daemon. Defaults to [`DEFAULT_BRIDGE_READY_TIMEOUT`];
    /// `FINITE_AGENTD_BRIDGE_READY_TIMEOUT_SECS` overrides it.
    pub bridge_ready_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecializationBundleStatusV1 {
    pub bundle_id: Option<String>,
    pub desired: bool,
    pub effective: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentdStatus {
    pub service: String,
    pub version: String,
    pub account_id: String,
    pub device_id: String,
    pub authorized_principals: usize,
    pub processes: SupervisorStatus,
    pub specialization: SpecializationBundleStatusV1,
    pub updated_at_ms: u64,
}

#[derive(Debug, Deserialize)]
struct AgentConfigFile {
    account_id: String,
    device_id: String,
}

#[derive(Debug, Deserialize)]
struct EmptyRequest {}

impl DaemonConfig {
    pub fn from_env() -> Result<Self, AgentdError> {
        let agent_home = std::env::var("FINITECHAT_HOME")
            .or_else(|_| std::env::var("FINITE_AGENT_HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/data/agent"));
        let hermes_home = std::env::var("HERMES_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| agent_home.join("hermes-home"));
        let bridge_addr = std::env::var("FINITE_AGENTD_BRIDGE_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:37633".to_owned());
        if !bridge_addr.starts_with("127.0.0.1:") && !bridge_addr.starts_with("localhost:") {
            return Err(AgentdError::Transport(
                "FINITE_AGENTD_BRIDGE_ADDR must bind loopback".to_owned(),
            ));
        }
        let authorized_accounts = std::env::var("FINITE_AGENTD_AUTHORIZED_ACCOUNT_IDS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        Ok(Self {
            agent_home,
            hermes_home,
            bridge_url: format!("http://{bridge_addr}"),
            bridge_addr,
            finitechat_bin: PathBuf::from(
                std::env::var("FINITECHAT_BIN")
                    .unwrap_or_else(|_| "/usr/local/bin/finitechat".to_owned()),
            ),
            prepare_command: PathBuf::from(
                std::env::var("FINITE_AGENTD_PREPARE_COMMAND")
                    .unwrap_or_else(|_| "/opt/run_hermes_gateway.sh".to_owned()),
            ),
            hermes_command: PathBuf::from(
                std::env::var("FINITE_AGENTD_HERMES_COMMAND")
                    .unwrap_or_else(|_| "/opt/run_hermes_gateway.sh".to_owned()),
            ),
            health_python: PathBuf::from(
                std::env::var("FINITE_AGENTD_HEALTH_PYTHON")
                    .unwrap_or_else(|_| "python".to_owned()),
            ),
            health_script: PathBuf::from(
                std::env::var("FINITE_AGENTD_HEALTH_SCRIPT")
                    .unwrap_or_else(|_| "/opt/health_server.py".to_owned()),
            ),
            authorized_accounts,
            sidecar_admission_default: sidecar_admission_default_from_value(
                std::env::var("FINITECHAT_ADMISSION_DEFAULT")
                    .ok()
                    .as_deref(),
            ),
            bridge_ready_timeout: bridge_ready_timeout_from_value(
                std::env::var(BRIDGE_READY_TIMEOUT_ENV).ok().as_deref(),
            )?,
        })
    }

    fn state_dir(&self) -> PathBuf {
        self.agent_home.join("agentd")
    }

    pub fn status_path(&self) -> PathBuf {
        self.state_dir().join("status.json")
    }
}

fn inactive_specialization_status() -> SpecializationBundleStatusV1 {
    SpecializationBundleStatusV1 {
        bundle_id: None,
        desired: false,
        effective: false,
    }
}

pub async fn run_daemon(config: DaemonConfig) -> Result<(), AgentdError> {
    become_process_group_leader();
    fs::create_dir_all(config.state_dir())?;
    fs::set_permissions(config.state_dir(), fs::Permissions::from_mode(0o700))?;
    clear_boot_scoped_health_evidence(&config);
    prepare_agent_runtime(&config)?;
    // After prepare (the agent home and store exist) and before the
    // supervisor starts the gateway or the sidecar, so admission state is
    // settled before anything reads it and the store writer lease is still
    // uncontended. Best-effort here by design: a failure (operator typo in
    // the seed env, store hiccup) must not take the whole agent down —
    // that would turn a typo into a total chat outage in a restart loop.
    // The sidecar's own boot-time seed re-runs the step and remains the
    // hard gate: a malformed seed env crash-loops the sidecar loudly
    // instead of silently downgrading to allow-all.
    if let Err(error) = seed_chat_admission(&config) {
        eprintln!(
            "finite-agentd: chat admission seed failed ({error}); \
             the sidecar enforces admission policy at its own boot"
        );
    }
    let identity = load_agent_identity(&config.agent_home)?;
    let ledger = Ledger::open(config.state_dir().join("agentd.sqlite3"))?;
    for account_id in &config.authorized_accounts {
        ledger.authorize_principal(account_id)?;
    }
    let config_manager = ConfigManager::new(config.hermes_home.join("config.yaml"), ledger.clone());
    let connection_manager = ConnectionManager::new(
        config.agent_home.clone(),
        config.hermes_home.clone(),
        config_manager.clone(),
    );
    let bridge = BridgeClient::new(config.bridge_url.clone())?;
    let intent_path = intent::intent_path(&config.agent_home);
    // Register before any optional child starts so a stop during bridge
    // warmup still reaches its awaited shutdown path.
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    // Hermes starts first, exactly as before any inference intent existed.
    let supervisor = start_supervisor(
        sidecar_spec(&config),
        health_spec(&config),
        hermes_spec(&config, &intent_path),
        simplex_spec(&config),
    );
    let hosted_hermes =
        HostedHermesHandle::start_gated(&config.hermes_home, ServeGate::new(intent_path.clone()))?;
    let codex = Arc::new(CodexState::default());
    let inference = Arc::new(Inference::new(
        AgentdHost {
            hermes_home: config.hermes_home.clone(),
            connections: connection_manager.clone(),
            supervisor: supervisor.clone(),
            hosted_hermes: hosted_hermes.clone(),
            codex: Arc::clone(&codex),
        },
        connection_manager.clone(),
        config_manager.clone(),
        config.hermes_home.clone(),
        intent_path,
        finite_private_env(),
        codex,
    ));
    // Only after Hermes has started; a bad intent never delays chat. Detached.
    drop(resume_intent_after_hermes_starts(
        Arc::clone(&inference.executor),
        supervisor.clone(),
    ));
    spawn_status_writer(
        config.status_path(),
        identity.clone(),
        ledger.clone(),
        supervisor.clone(),
        hosted_hermes.clone(),
    );

    let readiness = tokio::select! {
        result = wait_for_bridge(&bridge, config.bridge_ready_timeout) => result.map(|()| true),
        signal = tokio::signal::ctrl_c() => signal.map(|()| false).map_err(AgentdError::from),
        _ = sigterm.recv() => Ok(false),
    };
    if !matches!(readiness, Ok(true)) {
        supervisor.shutdown().await;
        if let Some(hosted) = &hosted_hermes {
            hosted.shutdown().await;
        }
        return readiness.map(|_| ());
    }
    let (delivery_tx, delivery_rx) = mpsc::channel::<RuntimeCommandDeliveryV1>(64);
    spawn_delivery_stream(bridge.clone(), delivery_tx);
    let executor = CommandExecutor {
        identity,
        ledger,
        config_manager,
        connection_manager,
        hermes_home: config.hermes_home,
        bridge: bridge.clone(),
        supervisor: supervisor.clone(),
        hosted_hermes: hosted_hermes.clone(),
        inference,
    };

    let delivery_worker =
        run_delivery_loop(delivery_rx, |delivery| executor.handle_delivery(delivery));
    tokio::pin!(delivery_worker);
    // SIGTERM takes the same graceful path as ctrl_c: a container stop
    // reaches agentd as SIGTERM (forwarded by entrypoint.sh), and the
    // supervisor's TERM-then-KILL drain is what gives Hermes/bridge/the
    // health server their bounded window to finish mid-stream writes.
    // Backported from PR 440 (83ef3024).
    let result = tokio::select! {
        result = &mut delivery_worker => {
            result
        }
        signal = tokio::signal::ctrl_c() => {
            supervisor.shutdown().await;
            signal.map_err(AgentdError::from)
        }
        _ = sigterm.recv() => {
            supervisor.shutdown().await;
            Ok(())
        }
    };
    if let Some(hosted) = &hosted_hermes {
        hosted.shutdown().await;
    }
    result
}

/// agentd leads its own session/process group when PID 1 (entrypoint.sh) is
/// its parent, so the entrypoint's post-exit sweep can SIGKILL the whole
/// supervised tree by pgid after agentd exits — the current topology's
/// equivalent of PR 440's shell-level generation quiesce (ec46243e). Scoped
/// to parent==PID 1 so an interactive `finite-agentd serve` keeps normal
/// terminal signal behavior; a setsid failure (e.g. EPERM because the
/// spawner already made us a group leader) only degrades the sweep to a
/// no-op, never the boot.
fn become_process_group_leader() {
    let parent_is_init = rustix::process::getppid()
        .map(|parent| parent.as_raw_pid() == 1)
        .unwrap_or(false);
    if !parent_is_init {
        return;
    }
    if let Err(error) = rustix::process::setsid() {
        eprintln!(
            "finite-agentd: could not lead a process group ({error}); \
             the entrypoint post-exit orphan sweep will no-op"
        );
    }
}

/// Health evidence is boot-scoped: files written under a previous boot must
/// never describe this one. Cleared before the writers (bridge sidecar,
/// health server, Hermes) are spawned, so /healthz can only ever report on
/// this boot's processes — a stale hermes-bridge-status.json otherwise keeps
/// claiming "connected" after the adapter that wrote it died. Mirrors the
/// transient allowlist recover_chat_boot.py enforces on recovery boots
/// (bridge_health_cache, agentd_health_cache, agentd_finitechat_ready). The
/// durable startup report is NOT touched: it is the recovery journal's
/// projection, not health evidence.
fn clear_boot_scoped_health_evidence(config: &DaemonConfig) {
    for path in [
        config.agent_home.join("hermes-bridge-status.json"),
        config.status_path(),
        config.state_dir().join("finitechat-ready.json"),
    ] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!(
                "finite-agentd: could not clear stale health evidence {}: {error}",
                path.display()
            ),
        }
    }
}

async fn run_delivery_loop<H, F>(
    mut delivery_rx: mpsc::Receiver<RuntimeCommandDeliveryV1>,
    mut handle_delivery: H,
) -> Result<(), AgentdError>
where
    H: FnMut(RuntimeCommandDeliveryV1) -> F,
    F: Future<Output = Result<(), AgentdError>>,
{
    loop {
        let Some(delivery) = delivery_rx.recv().await else {
            return Err(AgentdError::Transport(
                "command delivery worker stopped".to_owned(),
            ));
        };
        if let Err(error) = handle_delivery(delivery).await {
            // handle_delivery acknowledges only after its result has been
            // accepted. On failure, leave this item in the resident durable
            // inbox for redelivery, but do not let it block later commands.
            eprintln!(
                "finite-agentd: command delivery remains queued for redelivery: {}",
                error.public_message()
            );
        }
    }
}

fn prepare_agent_runtime(config: &DaemonConfig) -> Result<(), AgentdError> {
    let status = StdCommand::new(&config.prepare_command)
        .arg("--prepare-only")
        .stdin(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(AgentdError::Supervisor(format!(
            "agent runtime preparation failed with {status}"
        )))
    }
}

#[derive(Clone)]
struct CommandExecutor {
    identity: DeviceRef,
    ledger: Ledger,
    config_manager: ConfigManager,
    connection_manager: ConnectionManager,
    hermes_home: PathBuf,
    bridge: BridgeClient,
    supervisor: SupervisorHandle,
    hosted_hermes: Option<HostedHermesHandle>,
    inference: Arc<Inference<AgentdHost>>,
}

impl CommandExecutor {
    async fn handle_delivery(&self, delivery: RuntimeCommandDeliveryV1) -> Result<(), AgentdError> {
        let RuntimeCommandInboundPayloadV1::Request(request) = &delivery.payload else {
            self.bridge.acknowledge(&delivery).await?;
            return Ok(());
        };
        if !request.target.matches_device(&self.identity) {
            self.bridge.acknowledge(&delivery).await?;
            return Ok(());
        }

        let authorized = self
            .ledger
            .principal_is_authorized(&delivery.sender.account_id)?;
        let result = if !authorized && request.command == OWNER_CLAIM_COMMAND {
            if self.ledger.authorized_principal_count()? == 0 {
                self.ledger
                    .authorize_principal(&delivery.sender.account_id)?;
                let body = json!({ "connected": true });
                let result = success_result(request, body)?;
                self.ledger.begin_command(request)?;
                self.ledger.finish_command(&request.request_id, &result)?;
                result
            } else {
                failure_result(request, AgentdError::Unauthorized)
            }
        } else if !authorized {
            failure_result(request, AgentdError::Unauthorized)
        } else {
            match self.ledger.begin_command(request) {
                Ok(CommandDecision::Replay(result)) => result,
                Ok(CommandDecision::Execute | CommandDecision::Resume) => {
                    let result = match self.execute(request).await {
                        Ok(body) => success_result(request, body)?,
                        Err(error) => failure_result(request, error),
                    };
                    self.ledger.finish_command(&request.request_id, &result)?;
                    result
                }
                Err(error) => failure_result(request, error),
            }
        };

        self.bridge
            .send_result(RuntimeCommandResultDeliveryV1 {
                room_id: delivery.room_id.clone(),
                conversation_id: delivery.conversation_id.clone(),
                result,
            })
            .await?;
        self.bridge.acknowledge(&delivery).await?;
        if let Err(error) = self
            .publish_status(&delivery.room_id, delivery.conversation_id.clone())
            .await
        {
            eprintln!(
                "finite-agentd: runtime status publish will wait for the next command: {}",
                error.public_message()
            );
        }
        Ok(())
    }

    async fn execute(&self, request: &RuntimeCommandRequestV1) -> Result<Value, AgentdError> {
        if let Some(result) = self.inference.execute(request).await {
            return result;
        }
        match request.command.as_str() {
            "agent.status.inspect" => {
                parse_body::<EmptyRequest>(request, STATUS_REQUEST_SCHEMA)?;
                Ok(serde_json::to_value(self.current_status().await)?)
            }
            OWNER_CLAIM_COMMAND => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                Ok(json!({ "connected": true }))
            }
            "agent.simplex.connect" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                let offer = self
                    .connection_manager
                    .simplex_offer(&request.request_id, true)?;
                let manager = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || manager.simplex_address())
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                self.apply_config_offer(offer).await
            }
            "agent.simplex.reset" => {
                parse_body::<EmptyRequest>(request, "finite.agent.simplex.reset.v1")?;
                let offer = self
                    .connection_manager
                    .simplex_offer(&request.request_id, false)?;
                let config = self.config_manager.clone();
                let home = self.hermes_home.clone();
                let connections = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || {
                    config.apply(&offer, || validate_hermes_config(&home))?;
                    connections.prepare_simplex_reset()
                })
                .await
                .map_err(|error| AgentdError::Config(error.to_string()))??;
                // Even already-disabled connections need cleanup. The wrapper
                // completes durable reset intent before the next gateway starts.
                self.supervisor.restart_hermes().await?;
                let connections = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || connections.wait_simplex_reset())
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))?
            }
            "agent.simplex.approve_request" => {
                let body = parse_body::<crate::connections::SimplexApproveRequest>(
                    request,
                    "finite.agent.simplex.approve-request.v1",
                )?;
                let manager = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || manager.approve_simplex_request(body))
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                Ok(json!({ "approved": true }))
            }
            "agent.telegram.connect" => {
                let body = parse_body::<TelegramConnectRequest>(request, TELEGRAM_CONNECT_SCHEMA)?;
                let offer = self
                    .connection_manager
                    .telegram_connect_offer(&request.request_id, body)?;
                self.apply_config_offer(offer).await
            }
            "agent.telegram.approve" => {
                let body = parse_body::<PairingApproveRequest>(request, TELEGRAM_APPROVE_SCHEMA)?;
                let manager = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || manager.approve_telegram(body))
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                Ok(json!({ "approved": true }))
            }
            "agent.telegram.home" => {
                let body = parse_body::<TelegramHomeRequest>(request, TELEGRAM_HOME_SCHEMA)?;
                let offer = self
                    .connection_manager
                    .telegram_home_offer(&request.request_id, body)?;
                self.apply_config_offer(offer).await
            }
            "agent.telegram.disconnect" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                let offer = self
                    .connection_manager
                    .telegram_disconnect_offer(&request.request_id)?;
                self.apply_config_offer(offer).await
            }
            "agent.google.apply" => {
                let body = parse_body::<GoogleApplyRequest>(request, GOOGLE_APPLY_SCHEMA)?;
                let manager = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || manager.apply_google(body))
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                Ok(json!({ "connected": true }))
            }
            "agent.google.disconnect" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                let manager = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || manager.disconnect_google())
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                Ok(json!({ "connected": false }))
            }
            command => Err(AgentdError::UnsupportedCommand(command.to_owned())),
        }
    }

    async fn apply_config_offer(&self, offer: HermesConfigOfferV1) -> Result<Value, AgentdError> {
        let manager = self.config_manager.clone();
        let hermes_home = self.hermes_home.clone();
        let result = tokio::task::spawn_blocking(move || {
            manager.apply(&offer, || validate_hermes_config(&hermes_home))
        })
        .await
        .map_err(|error| AgentdError::Config(error.to_string()))??;
        if result.restart_required {
            self.supervisor.restart_hermes().await?;
        }
        Ok(serde_json::to_value(result)?)
    }

    async fn current_status(&self) -> AgentdStatus {
        let mut processes = self.supervisor.status().await;
        if let Some(hosted) = &self.hosted_hermes {
            processes
                .processes
                .insert("hermes-serve".into(), hosted.status());
        }
        AgentdStatus {
            service: "finite-agentd".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            account_id: self.identity.account_id.clone(),
            device_id: self.identity.device_id.clone(),
            authorized_principals: self.ledger.authorized_principal_count().unwrap_or(0),
            processes: processes.clone(),
            specialization: inactive_specialization_status(),
            updated_at_ms: now_ms(),
        }
    }

    async fn publish_status(
        &self,
        room_id: &str,
        conversation_id: Option<String>,
    ) -> Result<(), AgentdError> {
        let status = serde_json::to_vec(&self.current_status().await)?;
        let observed_at_ms = now_ms();
        self.bridge
            .send_state(RuntimeStateSnapshotDeliveryV1 {
                room_id: room_id.to_owned(),
                conversation_id,
                snapshot: RuntimeStateSnapshotV1 {
                    state_key: "runtime.agentd".to_owned(),
                    schema: STATUS_SCHEMA.to_owned(),
                    revision: observed_at_ms,
                    observed_at_ms,
                    expires_at_ms: observed_at_ms.saturating_add(5 * 60 * 1000),
                    status_payload: status,
                },
            })
            .await
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectRequest {
    route: IntentRoute,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DisconnectRequest {
    route: IntentRoute,
}

// No Debug: the credential holds a key or an OAuth code.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectRequest {
    credential: ConnectCredential,
    #[serde(default)]
    #[expect(dead_code, reason = "wired in A2")]
    activate: Option<ConnectActivation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectActivation {
    #[expect(dead_code, reason = "wired in A2")]
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexLoginCancelRequest {
    attempt_id: String,
}

/// The production executor host: the supervisor, the optional `hermes serve`,
/// the helper, and `connections.rs` for `.env`.
#[derive(Clone)]
struct AgentdHost {
    hermes_home: PathBuf,
    connections: ConnectionManager,
    supervisor: SupervisorHandle,
    hosted_hermes: Option<HostedHermesHandle>,
    codex: Arc<CodexState>,
}

impl ExecutorHost for AgentdHost {
    fn validate_config(&self) -> Result<(), AgentdError> {
        validate_hermes_config(&self.hermes_home)
    }

    fn dotenv_openrouter_key(&self) -> Result<Option<String>, AgentdError> {
        self.connections.openrouter_dotenv_key()
    }

    fn migrate_legacy_openrouter_key(&self) -> Result<(), AgentdError> {
        self.connections.migrate_legacy_openrouter_key()
    }

    fn remove_openrouter_key(&self) -> Result<(), AgentdError> {
        self.connections.remove_openrouter_key()
    }

    async fn restart_gateway(&self) -> Result<(), AgentdError> {
        self.supervisor.restart_hermes().await
    }

    async fn restart_serve(&self) {
        if let Some(hosted) = &self.hosted_hermes {
            hosted.restart().await;
        }
    }

    async fn facts(&self, deadline: Duration) -> Result<InferenceFacts, AgentdError> {
        crate::helper::inference_facts(&self.hermes_home, deadline).await
    }

    async fn cancel_codex_login(&self) -> Result<(), AgentdError> {
        crate::codex::cancel_for_disconnect(&self.codex).await
    }
}

/// Resumes a recorded intent only once Hermes has been started, so a pending,
/// failing, or unreadable intent never delays chat (§3.11 Startup). The
/// daemon detaches the task; tests await it.
fn resume_intent_after_hermes_starts<H: ExecutorHost + Clone>(
    executor: Arc<Executor<H>>,
    supervisor: SupervisorHandle,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let started = supervisor
                .status()
                .await
                .processes
                .get("hermes")
                .is_some_and(|status| status.pid().is_some() || status.restart_count > 0);
            if started {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        executor.resume_at_startup().await;
    })
}

/// The inference contract's commands (§3): status, v1 apply, select,
/// disconnect, and the PR2/PR3 hooks.
struct Inference<H> {
    host: H,
    executor: Arc<Executor<H>>,
    connections: ConnectionManager,
    config: ConfigManager,
    hermes_home: PathBuf,
    intent_path: PathBuf,
    fp: FinitePrivateEnv,
    facts: FactsCache,
    codex: Arc<CodexState>,
    openrouter: OpenRouterState,
    openrouter_api_base: String,
}

impl<H: ExecutorHost + Clone> Inference<H> {
    fn new(
        host: H,
        connections: ConnectionManager,
        config: ConfigManager,
        hermes_home: PathBuf,
        intent_path: PathBuf,
        fp: FinitePrivateEnv,
        codex: Arc<CodexState>,
    ) -> Self {
        Self {
            executor: Arc::new(Executor::new(
                host.clone(),
                config.clone(),
                intent_path.clone(),
                fp.clone(),
            )),
            host,
            connections,
            config,
            hermes_home,
            intent_path,
            fp,
            facts: FactsCache::default(),
            codex,
            openrouter: OpenRouterState::default(),
            openrouter_api_base: crate::openrouter::api_base(),
        }
    }

    /// `None` when the command is not one of `INFERENCE_COMMANDS`.
    async fn execute(
        &self,
        request: &RuntimeCommandRequestV1,
    ) -> Option<Result<Value, AgentdError>> {
        if !INFERENCE_COMMANDS.contains(&request.command.as_str()) {
            return None;
        }
        Some(self.dispatch(request).await)
    }

    async fn dispatch(&self, request: &RuntimeCommandRequestV1) -> Result<Value, AgentdError> {
        match request.command.as_str() {
            "agent.connections.status" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                self.admit(AdmitCommand::Status)?;
                self.status().await
            }
            "agent.inference.apply" => {
                let body = parse_body::<InferenceApplyRequest>(request, INFERENCE_APPLY_SCHEMA)?;
                let (_, admission) = self.admit(AdmitCommand::V1Apply)?;
                self.v1_apply(&request.request_id, body, admission).await
            }
            "agent.inference.select" => {
                let body = parse_body::<SelectRequest>(request, INFERENCE_SELECT_SCHEMA)?;
                let (_, admission) = self.admit(AdmitCommand::Select)?;
                self.select(body, admission).await
            }
            "agent.inference.disconnect" => {
                let body = parse_body::<DisconnectRequest>(request, INFERENCE_DISCONNECT_SCHEMA)?;
                let (record, admission) = self.admit(AdmitCommand::Disconnect(body.route))?;
                self.disconnect(body.route, record, admission).await
            }
            "agent.openrouter.usage" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                self.admit(AdmitCommand::OpenRouterUsage)?;
                self.openrouter_usage().await
            }
            "agent.openrouter.connect" => {
                let body = parse_body::<ConnectRequest>(request, OPENROUTER_CONNECT_SCHEMA)?;
                self.admit(AdmitCommand::Connect)?;
                crate::openrouter::obtain_candidate(&self.openrouter, body.credential)
                    .await
                    .map(|_| Value::Null)
            }
            "agent.codex.login.start" => {
                parse_body::<EmptyRequest>(request, CODEX_LOGIN_START_SCHEMA)?;
                self.admit(AdmitCommand::CodexLoginStart)?;
                crate::codex::start(&self.codex, &self.hermes_home)
                    .await
                    .and_then(|view| Ok(serde_json::to_value(view)?))
            }
            "agent.codex.login.cancel" => {
                let body =
                    parse_body::<CodexLoginCancelRequest>(request, CODEX_LOGIN_CANCEL_SCHEMA)?;
                self.admit(AdmitCommand::CodexLoginCancel)?;
                crate::codex::cancel(&self.codex, &body.attempt_id)
                    .await
                    .and_then(|view| Ok(serde_json::to_value(view)?))
            }
            "agent.codex.models" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                self.admit(AdmitCommand::CodexModels)?;
                crate::codex::models(&self.hermes_home)
                    .await
                    .and_then(|models| Ok(serde_json::to_value(models)?))
            }
            command => Err(AgentdError::UnsupportedCommand(command.to_owned())),
        }
    }

    /// The one admission (§3.11, F2): the current record through
    /// `intent::admit`. A read failure fails a mutation closed; status and the
    /// read-only commands are still served.
    fn admit(
        &self,
        command: AdmitCommand,
    ) -> Result<(Option<IntentRecord>, Admission), AgentdError> {
        let record = match intent::load(&self.intent_path) {
            Ok(record) => record,
            Err(error)
                if matches!(
                    command,
                    AdmitCommand::Status
                        | AdmitCommand::OpenRouterUsage
                        | AdmitCommand::CodexModels
                        | AdmitCommand::CodexLoginCancel
                ) =>
            {
                eprintln!("finite-agentd: could not read the inference intent: {error}");
                None
            }
            Err(error) => return Err(error),
        };
        let admission = intent::admit(record.as_ref(), command)?;
        Ok((record, admission))
    }

    async fn facts(&self) -> InferenceFacts {
        self.facts
            .get_or_fetch(&self.hermes_home, || {
                self.host.facts(crate::helper::STATUS_FACTS_DEADLINE)
            })
            .await
    }

    /// §3.4: legacy fields unchanged, plus stored facts and the operation.
    async fn status(&self) -> Result<Value, AgentdError> {
        let facts = self.facts().await;
        // Re-read after the helper ran, so the operation is current.
        let record = intent::load(&self.intent_path).unwrap_or(None);
        let operation = self.executor.operation(record.as_ref());
        let codex = crate::codex::route_status(&self.codex, &facts);
        let manager = self.connections.clone();
        let fp = self.fp.clone();
        let status = tokio::task::spawn_blocking(move || {
            manager.status_with_inference(&facts, &fp, operation, codex, capabilities())
        })
        .await
        .map_err(|error| AgentdError::Config(error.to_string()))??;
        Ok(serde_json::to_value(status)?)
    }

    fn spawn_executor(&self) {
        let executor = Arc::clone(&self.executor);
        tokio::spawn(async move { executor.run().await });
    }

    /// Writes a new intent before the reply, so nothing can run between
    /// admission and the record.
    fn record_intent(
        &self,
        kind: IntentKind,
        route: IntentRoute,
        model: Option<String>,
    ) -> Result<Value, AgentdError> {
        let record = IntentRecord::new(kind, route, model)
            .and_then(|record| intent::store(&self.intent_path, &record).map(|()| record))
            .map_err(|_| AgentdError::Config(INTENT_WRITE_FAILED.to_owned()))?;
        self.spawn_executor();
        Ok(json!({ "accepted": true, "operation_id": record.id }))
    }

    /// `ReplaceFailed` for a command that finished without writing an intent.
    fn clear_failed_record(&self, admission: Admission) {
        if admission == Admission::ReplaceFailed
            && let Err(error) = intent::clear(&self.intent_path)
        {
            eprintln!("finite-agentd: could not delete a failed inference intent: {error}");
        }
    }

    /// v1 (§3.5): today's order and failure behavior, including the `.env`
    /// snapshot restore, plus the no-op, the `hermes serve` restart for a
    /// replaced key, and verification after the restart.
    async fn v1_apply(
        &self,
        request_id: &str,
        body: InferenceApplyRequest,
        admission: Admission,
    ) -> Result<Value, AgentdError> {
        let plan = self
            .connections
            .inference_plan_with(request_id, body, &self.fp)?;
        if self.connections.inference_plan_is_noop(&plan)? {
            self.clear_failed_record(admission);
            return Ok(serde_json::to_value(ConfigApplyResultV1 {
                proposal_id: plan.offer.proposal_id.clone(),
                path: plan.offer.path.clone(),
                applied: false,
                already_applied: true,
                restart_required: false,
            })?);
        }
        let result = self.apply_inference_plan(&plan).await?;
        self.clear_failed_record(admission);
        Ok(result)
    }

    async fn apply_inference_plan(&self, plan: &InferenceApplyPlan) -> Result<Value, AgentdError> {
        let credential_snapshot = self.connections.stage_inference_credential(plan)?;
        let credential_replaced = credential_snapshot.is_some();
        let proposal_id = plan.offer.proposal_id.clone();
        let manager = self.config.clone();
        let host = self.host.clone();
        let offer = plan.offer.clone();
        let apply =
            tokio::task::spawn_blocking(move || manager.apply(&offer, || host.validate_config()))
                .await;
        let apply = match apply {
            Ok(apply) => apply,
            Err(error) => {
                self.connections
                    .restore_inference_credential(credential_snapshot)
                    .map_err(|restore_error| {
                        AgentdError::Config(format!(
                            "inference apply task failed ({error}); previous credential could not be restored ({restore_error})"
                        ))
                    })?;
                return Err(AgentdError::Config(error.to_string()));
            }
        };
        let result = match apply {
            Ok(result) => result,
            Err(error) => {
                self.connections
                    .restore_inference_credential(credential_snapshot)?;
                return Err(error);
            }
        };
        if !result.restart_required {
            return Ok(serde_json::to_value(result)?);
        }
        if let Err(restart_error) = self.host.restart_gateway().await {
            let rollback = HermesConfigRollbackV1 { proposal_id };
            let manager = self.config.clone();
            let host = self.host.clone();
            let rollback_result = tokio::task::spawn_blocking(move || {
                manager.rollback(&rollback, || host.validate_config())
            })
            .await;
            let credential_restore = self
                .connections
                .restore_inference_credential(credential_snapshot);
            let rollback_result = rollback_result.map_err(|rollback_error| {
                AgentdError::Supervisor(format!(
                    "Hermes inference activation failed ({restart_error}); configuration rollback task failed ({rollback_error})"
                ))
            })?;
            rollback_result.map_err(|rollback_error| {
                AgentdError::Supervisor(format!(
                    "Hermes inference activation failed ({restart_error}); previous configuration could not be restored ({rollback_error})"
                ))
            })?;
            credential_restore.map_err(|restore_error| {
                AgentdError::Supervisor(format!(
                    "Hermes inference activation failed ({restart_error}); previous credential could not be restored ({restore_error})"
                ))
            })?;
            self.host.restart_gateway().await.map_err(|restore_error| {
                AgentdError::Supervisor(format!(
                    "Hermes inference activation failed ({restart_error}); previous configuration was restored but Hermes could not be reactivated ({restore_error})"
                ))
            })?;
            return Err(restart_error);
        }
        if credential_replaced {
            // A replaced key: flush any copy the native `hermes serve` holds.
            self.host.restart_serve().await;
        }
        self.verify_v1(plan)?;
        Ok(serde_json::to_value(result)?)
    }

    /// §3.10 for v1 as ruled in R8: one immediate read of the parsed values
    /// agentd owns, never bytes, so the reply is no later than before. A
    /// mismatch is `config_conflict` with the state as found: no re-apply, no
    /// second restart, and no restore, since the apply itself succeeded.
    fn verify_v1(&self, plan: &InferenceApplyPlan) -> Result<(), AgentdError> {
        let model_matches = self.config.current_value(MODEL_CONFIG_PATH)? == plan.offer.value;
        let key_matches = match plan.credential_to_persist() {
            Some(key) => self.connections.openrouter_dotenv_key()?.as_deref() == Some(key),
            None => true,
        };
        if model_matches && key_matches {
            Ok(())
        } else {
            Err(AgentdError::ConfigConflict(
                "Something else changed the agent's model setting after it was saved. Status shows it as found.".to_owned(),
            ))
        }
    }

    /// §3.6: validate synchronously with no file touched, then record the
    /// intent and reply; the executor writes, restarts, and verifies.
    async fn select(
        &self,
        body: SelectRequest,
        admission: Admission,
    ) -> Result<Value, AgentdError> {
        let model = match body.route {
            IntentRoute::FinitePrivate => {
                if body.model.is_some() {
                    return Err(AgentdError::InvalidPayload(
                        "Finite Private takes its model from the agent".to_owned(),
                    ));
                }
                None
            }
            IntentRoute::Openrouter => {
                let model = body.model.ok_or_else(|| {
                    AgentdError::InvalidPayload("OpenRouter model is invalid".to_owned())
                })?;
                validate_model_name(&model)?;
                Some(model)
            }
            IntentRoute::OpenaiCodex => {
                if !capabilities().contains(&"codex.login.v1") {
                    return Err(AgentdError::InvalidPayload(
                        "This agent can't use ChatGPT yet".to_owned(),
                    ));
                }
                let model = body
                    .model
                    .filter(|model| valid_codex_model(model))
                    .ok_or_else(|| {
                        AgentdError::InvalidPayload("ChatGPT model is invalid".to_owned())
                    })?;
                Some(model)
            }
        };
        match body.route {
            IntentRoute::FinitePrivate => {
                if self.fp.settings().is_none() {
                    return Err(AgentdError::Config(
                        "Finite Private isn't available on this agent.".to_owned(),
                    ));
                }
            }
            IntentRoute::Openrouter => {
                // The key the route will use, including a legacy-config key the
                // executor migrates only after this check passes.
                let key = self.connections.stored_openrouter_key()?.ok_or_else(|| {
                    AgentdError::NotConnected("Connect OpenRouter first.".to_owned())
                })?;
                crate::openrouter::check_key_at(&self.openrouter_api_base, &key).await?;
            }
            IntentRoute::OpenaiCodex => {
                match self.facts().await.codex.state {
                    CodexStateFact::SignedIn | CodexStateFact::QuotaLimited => {}
                    CodexStateFact::NotSignedIn => {
                        return Err(AgentdError::NotConnected(
                            "Connect ChatGPT first.".to_owned(),
                        ));
                    }
                    CodexStateFact::SignInRequired => return Err(AgentdError::SignInRequired),
                    CodexStateFact::Unknown => {
                        return Err(AgentdError::ProviderUnavailable(
                            "Couldn't read the ChatGPT sign-in on this agent.".to_owned(),
                        ));
                    }
                }
                match crate::codex::models(&self.hermes_home).await? {
                    crate::codex::CodexModels::Live { models } => {
                        if !models.iter().any(|listed| Some(listed) == model.as_ref()) {
                            return Err(AgentdError::ModelUnavailable);
                        }
                    }
                    crate::codex::CodexModels::Unavailable { .. } => {
                        return Err(AgentdError::CatalogUnavailable);
                    }
                }
            }
        }
        let planned = plan_model_block(body.route, model.as_deref(), &self.fp)?;
        if self.config.current_value(MODEL_CONFIG_PATH)? == planned {
            self.clear_failed_record(admission);
            return Ok(json!({ "changed": false }));
        }
        self.record_intent(IntentKind::Select, body.route, model)
    }

    /// §3.7: the F1 precondition, then a new or resumed intent. Nothing is
    /// removed synchronously.
    async fn disconnect(
        &self,
        route: IntentRoute,
        record: Option<IntentRecord>,
        admission: Admission,
    ) -> Result<Value, AgentdError> {
        match route {
            IntentRoute::Openrouter => {}
            IntentRoute::OpenaiCodex if capabilities().contains(&"codex.login.v1") => {}
            _ => {
                return Err(AgentdError::InvalidPayload(
                    "That connection can't be disconnected here".to_owned(),
                ));
            }
        }
        let model = self.config.current_value(MODEL_CONFIG_PATH)?;
        let is_saved =
            classify_saved_route(&model, self.fp.base_url.as_deref()) == saved_route_of(route);
        let facts = self.facts().await;
        // F1: switching the agent to Finite Private needs its settings and its
        // credential known present. No live probe.
        if is_saved && (self.fp.settings().is_none() || facts.finite_private.fp_key != Tri::Present)
        {
            return Err(AgentdError::FinitePrivateUnavailable);
        }
        if admission == Admission::ResumeFailed {
            let Some(mut record) = record else {
                return Err(AgentdError::Config(INTENT_WRITE_FAILED.to_owned()));
            };
            record.state = IntentState::Running;
            record.error_code = None;
            record.attempts = 0;
            record.updated_at_ms = now_ms();
            intent::store(&self.intent_path, &record)
                .map_err(|_| AgentdError::Config(INTENT_WRITE_FAILED.to_owned()))?;
            self.spawn_executor();
            return Ok(json!({ "accepted": true, "operation_id": record.id }));
        }
        if !is_saved && self.nothing_stored(route, &facts)? {
            self.clear_failed_record(admission);
            return Ok(json!({ "changed": false }));
        }
        self.record_intent(IntentKind::Disconnect, route, None)
    }

    /// Nothing for the route in Finite's or Hermes's storage: what a verified
    /// disconnect leaves. Any `unknown` counts as stored.
    fn nothing_stored(
        &self,
        route: IntentRoute,
        facts: &InferenceFacts,
    ) -> Result<bool, AgentdError> {
        Ok(match route {
            IntentRoute::Openrouter => {
                self.connections.stored_openrouter_key()?.is_none()
                    && facts.openrouter.dotenv_key == Tri::Absent
                    && facts.openrouter.manual_pool_entries == PoolEntries::None
                    && facts.session_overrides.openrouter == Tri::Absent
            }
            IntentRoute::OpenaiCodex => {
                facts.codex.state == CodexStateFact::NotSignedIn
                    && facts.session_overrides.openai_codex == Tri::Absent
            }
            IntentRoute::FinitePrivate => false,
        })
    }

    async fn openrouter_usage(&self) -> Result<Value, AgentdError> {
        let saved_key = self
            .connections
            .openrouter_dotenv_key()?
            .filter(|key| !key.is_empty() && !key.starts_with("${"))
            .map(|api_key| SavedKey { api_key });
        let facts = self.facts().await;
        crate::openrouter::usage(saved_key, &facts).await
    }
}

fn saved_route_of(route: IntentRoute) -> SavedRoute {
    match route {
        IntentRoute::FinitePrivate => SavedRoute::FinitePrivate,
        IntentRoute::Openrouter => SavedRoute::Openrouter,
        IntentRoute::OpenaiCodex => SavedRoute::OpenaiCodex,
    }
}

/// §3.6: 1..128 characters of `[A-Za-z0-9._:-]`.
fn valid_codex_model(model: &str) -> bool {
    (1..=128).contains(&model.len())
        && model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn parse_body<T: DeserializeOwned>(
    request: &RuntimeCommandRequestV1,
    expected_schema: &str,
) -> Result<T, AgentdError> {
    if request.body.schema != expected_schema {
        return Err(AgentdError::InvalidPayload(format!(
            "expected schema {expected_schema:?}"
        )));
    }
    serde_json::from_slice(&request.body.json_payload).map_err(|_| {
        AgentdError::InvalidPayload("request JSON did not match its schema".to_owned())
    })
}

fn success_result(
    request: &RuntimeCommandRequestV1,
    body: Value,
) -> Result<RuntimeCommandResultV1, AgentdError> {
    let result = RuntimeCommandResultV1 {
        payload_kind: RuntimeCommandPayloadKindV1::Result,
        request_id: request.request_id.clone(),
        status: RuntimeCommandTerminalStatusV1::Succeeded,
        body: Some(RuntimeCommandJsonPayloadV1 {
            schema: RESULT_SCHEMA.to_owned(),
            json_payload: serde_json::to_vec(&body)?,
        }),
        error: None,
        clears_activity: Vec::new(),
    };
    result
        .validate_structure()
        .map_err(|error| AgentdError::InvalidPayload(error.to_string()))?;
    Ok(result)
}

fn failure_result(request: &RuntimeCommandRequestV1, error: AgentdError) -> RuntimeCommandResultV1 {
    RuntimeCommandResultV1 {
        payload_kind: RuntimeCommandPayloadKindV1::Result,
        request_id: request.request_id.clone(),
        status: RuntimeCommandTerminalStatusV1::Failed,
        body: None,
        error: Some(RuntimeCommandErrorV1 {
            code: error.public_code().to_owned(),
            message: error.public_message(),
        }),
        clears_activity: Vec::new(),
    }
}

fn validate_hermes_config(hermes_home: &Path) -> Result<(), AgentdError> {
    let status = StdCommand::new("hermes")
        .arg("config")
        .arg("check")
        .env("HERMES_HOME", hermes_home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(AgentdError::Config(
            "Hermes rejected the proposed configuration; previous bytes were restored".to_owned(),
        ))
    }
}

fn load_agent_identity(agent_home: &Path) -> Result<DeviceRef, AgentdError> {
    let config =
        serde_json::from_slice::<AgentConfigFile>(&fs::read(agent_home.join("config.json"))?)?;
    Ok(DeviceRef::new(config.account_id, config.device_id))
}

fn sidecar_admission_default_from_value(admission_default: Option<&str>) -> Option<String> {
    // An explicit marker is inherited by the sidecar unchanged; otherwise
    // agentd's presence is itself the hosted marker, so the sidecar it spawns
    // defaults a row-less admission policy to locked (allowlist) mode.
    if admission_default.is_some_and(|value| !value.trim().is_empty()) {
        return None;
    }
    Some("locked".to_owned())
}

/// Seed chat admission once, before the supervisor starts any child process.
///
/// The sidecar's SQLite admission store is the single source of truth; this
/// step is its only writer at container boot. Running it here — after
/// `prepare_agent_runtime` has created the agent home and before the gateway
/// script or the sidecar can race for the store writer lease — means the
/// gateway launcher reads a current `allowed-users` mirror on its very first
/// start, including the legacy-lockdown derivation on an upgraded agent.
/// Later allowlist changes go through `chat.admission` commands, which
/// rewrite the mirror in place; they reach the gateway at its next restart.
///
/// Callers treat failure as non-fatal: log and continue. The sidecar's own
/// boot-time seed is the enforcing copy of this step and fails its boot on
/// a malformed seed env, so skipping here can never silently downgrade
/// admission to allow-all.
fn seed_chat_admission(config: &DaemonConfig) -> Result<(), AgentdError> {
    let status = StdCommand::new(&config.finitechat_bin)
        .args(["hermes", "--agent-home"])
        .arg(config.agent_home.as_os_str())
        .arg("admission")
        .arg("seed")
        .stdin(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(AgentdError::Supervisor(format!(
            "chat admission seed failed with {status}"
        )))
    }
}

fn sidecar_spec(config: &DaemonConfig) -> ProcessSpec {
    let mut environment = BTreeMap::new();
    if let Some(default) = &config.sidecar_admission_default {
        environment.insert("FINITECHAT_ADMISSION_DEFAULT".to_owned(), default.clone());
    }
    ProcessSpec {
        name: "finitechat",
        program: config.finitechat_bin.clone(),
        args: vec![
            "hermes".to_owned(),
            "--agent-home".to_owned(),
            config.agent_home.display().to_string(),
            "serve".to_owned(),
            "--addr".to_owned(),
            config.bridge_addr.clone(),
            "--ready-file".to_owned(),
            config
                .state_dir()
                .join("finitechat-ready.json")
                .display()
                .to_string(),
            "--json".to_owned(),
        ],
        environment,
    }
}

fn simplex_spec(config: &DaemonConfig) -> Option<ProcessSpec> {
    let script = crate::simplex::script_path();
    script.is_file().then(|| ProcessSpec {
        name: "simplex",
        program: config.health_python.clone(),
        args: vec![script.display().to_string(), "supervise".to_owned()],
        environment: BTreeMap::from([
            (
                "HERMES_HOME".to_owned(),
                config.hermes_home.display().to_string(),
            ),
            (
                "FINITECHAT_HOME".to_owned(),
                config.agent_home.display().to_string(),
            ),
        ]),
    })
}

fn health_spec(config: &DaemonConfig) -> ProcessSpec {
    ProcessSpec {
        name: "health",
        program: config.health_python.clone(),
        args: vec![config.health_script.display().to_string()],
        environment: BTreeMap::new(),
    }
}

fn hermes_spec(config: &DaemonConfig, intent_path: &Path) -> ProcessSpec {
    ProcessSpec {
        name: "hermes",
        program: config.hermes_command.clone(),
        args: Vec::new(),
        environment: BTreeMap::from([
            ("FINITE_AGENTD_SUPERVISED".to_owned(), "1".to_owned()),
            (
                "FINITECHAT_HERMES_SERVICE_URL".to_owned(),
                config.bridge_url.clone(),
            ),
            // The launcher's pending-disconnect step reads, never writes, it.
            (
                "FINITE_AGENTD_INTENT_PATH".to_owned(),
                intent_path.display().to_string(),
            ),
        ]),
    }
}

fn spawn_delivery_stream(bridge: BridgeClient, tx: mpsc::Sender<RuntimeCommandDeliveryV1>) {
    tokio::spawn(async move {
        let mut retry = Duration::from_millis(250);
        loop {
            if bridge.stream_deliveries(tx.clone()).await.is_ok() {
                return;
            }
            tokio::time::sleep(retry).await;
            retry = (retry * 2).min(Duration::from_secs(5));
        }
    });
}

/// Bridge cold start reprocesses retained room history before serving, so
/// time-to-ready scales with data size and host I/O, not just spawn speed.
/// 2026-08-18: a ~15k-message room needed ~55s on lat3 and boot-looped
/// against the old 30s deadline; 180s gives large rooms headroom while a
/// truly dead bridge still fails (and is retried) within minutes.
/// `FINITE_AGENTD_BRIDGE_READY_TIMEOUT_SECS` overrides this default.
const DEFAULT_BRIDGE_READY_TIMEOUT: Duration = Duration::from_secs(180);

fn bridge_ready_timeout_from_value(value: Option<&str>) -> Result<Duration, AgentdError> {
    let Some(raw) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(DEFAULT_BRIDGE_READY_TIMEOUT);
    };
    let secs = raw
        .parse::<u64>()
        .ok()
        .filter(|secs| *secs > 0)
        .ok_or_else(|| {
            AgentdError::Config(format!(
                "{BRIDGE_READY_TIMEOUT_ENV} must be a positive integer"
            ))
        })?;
    Ok(Duration::from_secs(secs))
}

async fn wait_for_bridge(bridge: &BridgeClient, timeout: Duration) -> Result<(), AgentdError> {
    let mut retry = Duration::from_millis(50);
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if bridge.wait_until_ready().await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(AgentdError::Transport(
                "Finite Chat bridge did not become ready".to_owned(),
            ));
        }
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(Duration::from_secs(1));
    }
}

fn spawn_status_writer(
    path: PathBuf,
    identity: DeviceRef,
    ledger: Ledger,
    supervisor: SupervisorHandle,
    hosted_hermes: Option<crate::hosted_hermes::HostedHermesHandle>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let mut processes = supervisor.status().await;
            if let Some(hosted) = &hosted_hermes {
                processes
                    .processes
                    .insert("hermes-serve".into(), hosted.status());
            }
            let status = AgentdStatus {
                service: "finite-agentd".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                account_id: identity.account_id.clone(),
                device_id: identity.device_id.clone(),
                authorized_principals: ledger.authorized_principal_count().unwrap_or(0),
                processes,
                specialization: inactive_specialization_status(),
                updated_at_ms: now_ms(),
            };
            let _ = write_private_json(&path, &status);
        }
    });
}

fn write_private_json(path: &Path, value: &impl Serialize) -> Result<(), AgentdError> {
    let parent = path
        .parent()
        .ok_or_else(|| AgentdError::Io(std::io::Error::other("status path has no parent")))?;
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temporary.write_all(&serde_json::to_vec_pretty(value)?)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| AgentdError::Io(error.error))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn read_status(path: &Path) -> Result<AgentdStatus, AgentdError> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use finitechat_proto::{RuntimeCommandCancelV1, RuntimeCommandPayloadKindV1};

    use super::*;

    #[test]
    fn bridge_ready_timeout_defaults_to_180s_and_validates_override() {
        assert_eq!(
            bridge_ready_timeout_from_value(None).unwrap(),
            Duration::from_secs(180)
        );
        assert_eq!(
            bridge_ready_timeout_from_value(Some("  ")).unwrap(),
            Duration::from_secs(180)
        );
        assert_eq!(
            bridge_ready_timeout_from_value(Some("45")).unwrap(),
            Duration::from_secs(45)
        );
        assert!(bridge_ready_timeout_from_value(Some("0")).is_err());
        assert!(bridge_ready_timeout_from_value(Some("-5")).is_err());
        assert!(bridge_ready_timeout_from_value(Some("soon")).is_err());
    }

    #[test]
    fn boot_scoped_health_evidence_is_cleared_but_durable_state_survives() {
        let home = tempfile::tempdir().unwrap();
        let agent_home = home.path().join("agent");
        let state_dir = agent_home.join("agentd");
        fs::create_dir_all(&state_dir).unwrap();
        for path in [
            agent_home.join("hermes-bridge-status.json"),
            state_dir.join("status.json"),
            state_dir.join("finitechat-ready.json"),
        ] {
            fs::write(path, "{}\n").unwrap();
        }
        // The recovery journal's projection and real durable state are not
        // health evidence and must survive the boot clearing.
        fs::write(agent_home.join("startup-report.json"), "{}\n").unwrap();
        fs::write(agent_home.join("config.json"), "{}\n").unwrap();

        let mut config = DaemonConfig {
            agent_home: agent_home.clone(),
            hermes_home: agent_home.join("hermes-home"),
            bridge_url: "http://127.0.0.1:37633".to_owned(),
            bridge_addr: "127.0.0.1:37633".to_owned(),
            finitechat_bin: PathBuf::from("/bin/finitechat"),
            prepare_command: PathBuf::from("/bin/true"),
            hermes_command: PathBuf::from("/bin/true"),
            health_python: PathBuf::from("python"),
            health_script: PathBuf::from("/opt/health_server.py"),
            authorized_accounts: BTreeSet::new(),
            sidecar_admission_default: None,
            bridge_ready_timeout: Duration::from_secs(1),
        };
        clear_boot_scoped_health_evidence(&config);

        assert!(!agent_home.join("hermes-bridge-status.json").exists());
        assert!(!state_dir.join("status.json").exists());
        assert!(!state_dir.join("finitechat-ready.json").exists());
        assert!(agent_home.join("startup-report.json").exists());
        assert!(agent_home.join("config.json").exists());

        // Missing files are a normal first-boot state, not an error.
        clear_boot_scoped_health_evidence(&config);
        config.agent_home = home.path().join("missing-agent-home");
        clear_boot_scoped_health_evidence(&config);
    }

    #[test]
    fn sidecar_spec_relays_only_the_admission_default_marker() {
        let mut config = DaemonConfig {
            agent_home: PathBuf::from("/data/agent"),
            hermes_home: PathBuf::from("/data/agent/hermes-home"),
            bridge_url: "http://127.0.0.1:37633".to_owned(),
            bridge_addr: "127.0.0.1:37633".to_owned(),
            finitechat_bin: PathBuf::from("/bin/finitechat"),
            prepare_command: PathBuf::from("/bin/true"),
            hermes_command: PathBuf::from("/bin/true"),
            health_python: PathBuf::from("python"),
            health_script: PathBuf::from("/opt/health_server.py"),
            authorized_accounts: BTreeSet::new(),
            sidecar_admission_default: Some("locked".to_owned()),
            bridge_ready_timeout: Duration::from_secs(1),
        };
        let spec = sidecar_spec(&config);
        assert_eq!(
            spec.environment.get("FINITECHAT_ADMISSION_DEFAULT"),
            Some(&"locked".to_owned())
        );
        // Admission values are never derived here: the sidecar inherits
        // FINITECHAT_OWNER_NPUBS and FINITECHAT_WELCOME_ALLOWLIST from the
        // container environment (spawns are additive), so no allowlist key
        // may appear in the spec.
        config.sidecar_admission_default = None;
        let spec = sidecar_spec(&config);
        assert!(spec.environment.is_empty());
    }

    #[test]
    fn sidecar_admission_default_is_locked_unless_explicitly_set() {
        assert_eq!(
            sidecar_admission_default_from_value(None),
            Some("locked".to_owned())
        );
        assert_eq!(
            sidecar_admission_default_from_value(Some("  ")),
            Some("locked".to_owned())
        );
        // An explicit marker (e.g. an operator previewing a future value) is
        // inherited by the sidecar unchanged.
        assert_eq!(sidecar_admission_default_from_value(Some("open")), None);
    }

    #[test]
    fn specialization_status_stays_inactive_after_aeon_retirement() {
        let status = inactive_specialization_status();
        assert!(status.bundle_id.is_none());
        assert!(!status.desired);
        assert!(!status.effective);
    }

    #[test]
    fn leftover_specialization_env_cannot_become_desired_or_fail_boot() {
        // from_env no longer reads FINITE_SPECIALIZATION_* or FBRAIN_EMBEDDING_*.
        // Leftover container env cannot fail boot or become desired state.
        let config = DaemonConfig {
            agent_home: PathBuf::from("/data/agent"),
            hermes_home: PathBuf::from("/data/agent/hermes-home"),
            bridge_url: "http://127.0.0.1:37633".to_owned(),
            bridge_addr: "127.0.0.1:37633".to_owned(),
            finitechat_bin: PathBuf::from("/bin/finitechat"),
            prepare_command: PathBuf::from("/bin/true"),
            hermes_command: PathBuf::from("/bin/true"),
            health_python: PathBuf::from("python"),
            health_script: PathBuf::from("/opt/health_server.py"),
            authorized_accounts: BTreeSet::new(),
            sidecar_admission_default: None,
            bridge_ready_timeout: Duration::from_secs(1),
        };
        assert!(!format!("{config:?}").contains("aeon-multimodal"));
        let json = serde_json::to_value(AgentdStatus {
            service: "finite-agentd".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            account_id: "acct".to_owned(),
            device_id: "dev".to_owned(),
            authorized_principals: 0,
            processes: SupervisorStatus::default(),
            specialization: inactive_specialization_status(),
            updated_at_ms: 0,
        })
        .unwrap();
        assert_eq!(json["specialization"]["desired"], false);
        assert_eq!(json["specialization"]["effective"], false);
        assert!(json["specialization"]["bundle_id"].is_null());
        assert_eq!(
            AgentdError::UnsupportedCommand("agent.retired.command".to_owned()).public_code(),
            "unsupported_command"
        );
    }

    fn delivery(message_id: &str, seq: u64) -> RuntimeCommandDeliveryV1 {
        RuntimeCommandDeliveryV1 {
            room_id: "room-main".to_owned(),
            conversation_id: Some("conversation-main".to_owned()),
            seq,
            message_id: message_id.to_owned(),
            sender: DeviceRef::new("user-account", "hosted-web"),
            payload: RuntimeCommandInboundPayloadV1::Cancel(RuntimeCommandCancelV1 {
                payload_kind: RuntimeCommandPayloadKindV1::Cancel,
                request_id: format!("request-{seq}"),
                reason: None,
            }),
        }
    }

    #[tokio::test]
    async fn failed_delivery_does_not_block_later_delivery_and_remains_retryable() {
        let (delivery_tx, delivery_rx) = mpsc::channel(3);
        let failed = delivery("delivery-failed", 1);
        delivery_tx.send(failed.clone()).await.unwrap();
        delivery_tx
            .send(delivery("delivery-later", 2))
            .await
            .unwrap();
        delivery_tx
            .send(failed)
            .await
            .expect("the durable inbox may redeliver an unacknowledged item");
        drop(delivery_tx);

        let attempts = Arc::new(Mutex::new(Vec::new()));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let failed_attempts = Arc::new(AtomicUsize::new(0));

        let result = run_delivery_loop(delivery_rx, {
            let attempts = Arc::clone(&attempts);
            let completed = Arc::clone(&completed);
            let failed_attempts = Arc::clone(&failed_attempts);
            move |delivery| {
                let attempts = Arc::clone(&attempts);
                let completed = Arc::clone(&completed);
                let failed_attempts = Arc::clone(&failed_attempts);
                async move {
                    let message_id = delivery.message_id;
                    attempts.lock().unwrap().push(message_id.clone());
                    if message_id == "delivery-failed"
                        && failed_attempts.fetch_add(1, Ordering::SeqCst) == 0
                    {
                        return Err(AgentdError::Transport(
                            "injected result delivery failure".to_owned(),
                        ));
                    }
                    completed.lock().unwrap().push(message_id);
                    Ok(())
                }
            }
        })
        .await;

        assert!(matches!(result, Err(AgentdError::Transport(_))));
        assert_eq!(
            *attempts.lock().unwrap(),
            ["delivery-failed", "delivery-later", "delivery-failed"],
            "a failed item must not monopolize the single delivery loop"
        );
        assert_eq!(
            *completed.lock().unwrap(),
            ["delivery-later", "delivery-failed"],
            "the later item completes before durable redelivery retries the failed item"
        );
    }
}

#[cfg(test)]
mod inference_tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    use finitechat_proto::{RuntimeCommandPayloadKindV1, RuntimeCommandTargetV1};
    use serde_json::json;
    use tokio::sync::Notify;

    use super::*;
    use crate::openrouter::fake::{FakeOpenRouter, key_data};

    const FP_URL: &str = "https://fp.example.invalid/v1";
    const OR_KEY: &str = "sk-or-v1-synthetic-agent-key";
    const OR_MODEL: &str = "anthropic/claude-sonnet-4.6";

    type RestartHook = Box<dyn FnMut(usize) -> Result<(), AgentdError> + Send>;

    /// A test host. `.env` handling is the real `ConnectionManager`; restarts,
    /// facts, and the launcher's pending-disconnect step are simulated.
    #[derive(Clone)]
    struct TestHost {
        connections: ConnectionManager,
        config_path: PathBuf,
        intent_path: PathBuf,
        events: Arc<Mutex<Vec<String>>>,
        facts: Arc<Mutex<InferenceFacts>>,
        facts_fail: Arc<Mutex<bool>>,
        /// The deadline each facts read was given.
        facts_deadlines: Arc<Mutex<Vec<Duration>>>,
        validate_fail: Arc<Mutex<bool>>,
        restarts: Arc<Mutex<usize>>,
        on_restart: Arc<Mutex<RestartHook>>,
        hold_restart: Arc<Mutex<Option<Arc<Notify>>>>,
        launcher_clears_overrides: Arc<Mutex<bool>>,
        /// Set to a marker path to check that Hermes started before each step.
        started_marker: Arc<Mutex<Option<PathBuf>>>,
    }

    impl TestHost {
        fn event(&self, event: &str) {
            let marker = self.started_marker.lock().unwrap().clone();
            if let Some(marker) = marker {
                // The supervisor spawned Hermes before the resume began; give
                // its first line time to run on a loaded machine.
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                while !marker.exists() && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
                assert!(marker.exists(), "{event} ran before Hermes started");
            }
            self.events.lock().unwrap().push(event.to_owned());
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }

        fn model(&self) -> Value {
            let text = fs::read_to_string(&self.config_path).unwrap();
            serde_yaml::from_str::<Value>(&text).unwrap()["model"].clone()
        }

        /// The launcher's pending-disconnect step (C1, F1): clears only at
        /// `cleanup`/`verifying` once the saved default is no longer the route.
        fn launcher_step(&self) -> &'static str {
            let Ok(Some(record)) = intent::load(&self.intent_path) else {
                return "skipped";
            };
            let switched =
                classify_saved_route(&self.model(), Some(FP_URL)) != saved_route_of(record.route);
            if record.kind != IntentKind::Disconnect
                || !matches!(
                    record.phase,
                    intent::IntentPhase::Cleanup | intent::IntentPhase::Verifying
                )
                || !switched
            {
                return "skipped";
            }
            let overrides = *self.launcher_clears_overrides.lock().unwrap();
            let mut facts = self.facts.lock().unwrap();
            match record.route {
                IntentRoute::Openrouter => {
                    facts.openrouter.manual_pool_entries = PoolEntries::None;
                    if overrides {
                        facts.session_overrides.openrouter = Tri::Absent;
                    }
                }
                IntentRoute::OpenaiCodex => {
                    facts.codex.state = CodexStateFact::NotSignedIn;
                    if overrides {
                        facts.session_overrides.openai_codex = Tri::Absent;
                    }
                }
                IntentRoute::FinitePrivate => {}
            }
            "applied"
        }
    }

    impl ExecutorHost for TestHost {
        fn validate_config(&self) -> Result<(), AgentdError> {
            if *self.validate_fail.lock().unwrap() {
                return Err(AgentdError::Config("Hermes rejected it".to_owned()));
            }
            Ok(())
        }

        fn dotenv_openrouter_key(&self) -> Result<Option<String>, AgentdError> {
            self.connections.openrouter_dotenv_key()
        }

        fn migrate_legacy_openrouter_key(&self) -> Result<(), AgentdError> {
            self.event("migrate");
            self.connections.migrate_legacy_openrouter_key()
        }

        fn remove_openrouter_key(&self) -> Result<(), AgentdError> {
            self.connections.remove_openrouter_key()
        }

        async fn restart_gateway(&self) -> Result<(), AgentdError> {
            self.event("gateway");
            let hold = self.hold_restart.lock().unwrap().clone();
            if let Some(hold) = hold {
                hold.notified().await;
            }
            self.launcher_step();
            let count = {
                let mut restarts = self.restarts.lock().unwrap();
                *restarts += 1;
                *restarts
            };
            (self.on_restart.lock().unwrap())(count)
        }

        async fn restart_serve(&self) {
            self.event("serve");
        }

        async fn facts(&self, deadline: Duration) -> Result<InferenceFacts, AgentdError> {
            self.facts_deadlines.lock().unwrap().push(deadline);
            if *self.facts_fail.lock().unwrap() {
                return Err(AgentdError::ProviderUnavailable("helper".to_owned()));
            }
            let mut facts = self.facts.lock().unwrap().clone();
            facts.openrouter.dotenv_key = if self.connections.openrouter_dotenv_key()?.is_some() {
                Tri::Present
            } else {
                Tri::Absent
            };
            Ok(facts)
        }

        async fn cancel_codex_login(&self) -> Result<(), AgentdError> {
            self.event("cancel-login");
            Ok(())
        }
    }

    struct Setup {
        _temp: tempfile::TempDir,
        agent_home: PathBuf,
        hermes_home: PathBuf,
        host: TestHost,
        inference: Inference<TestHost>,
    }

    fn fp() -> FinitePrivateEnv {
        FinitePrivateEnv {
            model: Some("glm-5-3-flash".to_owned()),
            base_url: Some(FP_URL.to_owned()),
            context_length: Some(393_216),
        }
    }

    fn fp_block() -> Value {
        plan_model_block(IntentRoute::FinitePrivate, None, &fp()).unwrap()
    }

    fn openrouter_block() -> Value {
        plan_model_block(IntentRoute::Openrouter, Some(OR_MODEL), &fp()).unwrap()
    }

    /// Facts of a healthy agent: FP key present, canonical backup, nothing
    /// stored for OpenRouter or Codex in Hermes.
    fn healthy_facts() -> InferenceFacts {
        let mut facts = InferenceFacts::unknown();
        facts.finite_private.fp_key = Tri::Present;
        facts.finite_private.provider_entry = crate::facts::ProviderEntryFact::Canonical;
        facts.fallback.fallback_providers = Tri::Present;
        facts.fallback.fallback_model = Tri::Absent;
        facts.fallback.effective = Some(vec![crate::facts::FallbackEntryFact {
            provider: "finite-private".to_owned(),
            model: "glm-5-3-flash".to_owned(),
            owned_canonical: crate::facts::YesNo::Yes,
        }]);
        facts.openrouter.hermes_key = Tri::Absent;
        facts.openrouter.manual_pool_entries = PoolEntries::None;
        facts.session_overrides.openrouter = Tri::Absent;
        facts.session_overrides.openai_codex = Tri::Absent;
        facts.codex.state = CodexStateFact::NotSignedIn;
        facts
    }

    fn new_setup(model: &Value, env: &str) -> Setup {
        let temp = tempfile::tempdir().unwrap();
        let agent_home = temp.path().join("agent");
        let hermes_home = agent_home.join("hermes-home");
        fs::create_dir_all(&hermes_home).unwrap();
        let config_path = hermes_home.join("config.yaml");
        fs::write(
            &config_path,
            serde_yaml::to_string(&json!({ "model": model })).unwrap(),
        )
        .unwrap();
        fs::write(hermes_home.join(".env"), env).unwrap();
        let ledger = Ledger::open(agent_home.join("agentd/agentd.sqlite3")).unwrap();
        let config = ConfigManager::new(&config_path, ledger);
        let connections = ConnectionManager::new(&agent_home, &hermes_home, config.clone());
        let intent_path = intent::intent_path(&agent_home);
        let host = TestHost {
            connections: connections.clone(),
            config_path,
            intent_path: intent_path.clone(),
            events: Arc::default(),
            facts: Arc::new(Mutex::new(healthy_facts())),
            facts_fail: Arc::default(),
            facts_deadlines: Arc::default(),
            validate_fail: Arc::default(),
            restarts: Arc::default(),
            on_restart: Arc::new(Mutex::new(Box::new(|_| Ok(())))),
            hold_restart: Arc::default(),
            launcher_clears_overrides: Arc::new(Mutex::new(true)),
            started_marker: Arc::default(),
        };
        let mut inference = Inference::new(
            host.clone(),
            connections,
            config,
            hermes_home.clone(),
            intent_path,
            fp(),
            Arc::new(CodexState::default()),
        );
        // Unreachable unless a test starts a fake OpenRouter.
        inference.openrouter_api_base = "http://127.0.0.1:9/api/v1".to_owned();
        // A read every 50 ms and a 600 ms launcher window, in place of 5 s and 60 s.
        Arc::get_mut(&mut inference.executor)
            .expect("only Inference holds the executor yet")
            .set_timing(Duration::from_millis(50), Duration::from_millis(600));
        Setup {
            _temp: temp,
            agent_home,
            hermes_home,
            host,
            inference,
        }
    }

    fn request(command: &str, schema: &str, body: Value) -> RuntimeCommandRequestV1 {
        RuntimeCommandRequestV1 {
            payload_kind: RuntimeCommandPayloadKindV1::Request,
            request_id: "request-1".to_owned(),
            command: command.to_owned(),
            target: RuntimeCommandTargetV1 {
                account_id: "acct".to_owned(),
                device_id: None,
            },
            resource_key: None,
            body: RuntimeCommandJsonPayloadV1 {
                schema: schema.to_owned(),
                json_payload: serde_json::to_vec(&body).unwrap(),
            },
        }
    }

    impl Setup {
        async fn run(
            &self,
            command: &str,
            schema: &str,
            body: Value,
        ) -> Result<Value, AgentdError> {
            self.inference
                .execute(&request(command, schema, body))
                .await
                .expect("an inference command")
        }

        async fn status(&self) -> Value {
            self.run("agent.connections.status", EMPTY_REQUEST_SCHEMA, json!({}))
                .await
                .unwrap()
        }

        async fn select(&self, body: Value) -> Result<Value, AgentdError> {
            self.run("agent.inference.select", INFERENCE_SELECT_SCHEMA, body)
                .await
        }

        async fn disconnect(&self, route: &str) -> Result<Value, AgentdError> {
            self.run(
                "agent.inference.disconnect",
                INFERENCE_DISCONNECT_SCHEMA,
                json!({ "route": route }),
            )
            .await
        }

        async fn v1(&self, body: Value) -> Result<Value, AgentdError> {
            self.run("agent.inference.apply", INFERENCE_APPLY_SCHEMA, body)
                .await
        }

        fn record(&self) -> Option<IntentRecord> {
            intent::load(&self.inference.intent_path).unwrap()
        }

        fn store(&self, kind: IntentKind, route: IntentRoute, state: IntentState) -> IntentRecord {
            let mut record = IntentRecord::new(kind, route, None).unwrap();
            if kind != IntentKind::Disconnect {
                record.model = Some(OR_MODEL.to_owned());
            }
            record.state = state;
            intent::store(&self.inference.intent_path, &record).unwrap();
            record
        }

        fn env(&self) -> String {
            fs::read_to_string(self.hermes_home.join(".env")).unwrap_or_default()
        }

        fn config_bytes(&self) -> Vec<u8> {
            fs::read(&self.host.config_path).unwrap()
        }

        /// Waits for the background operation to delete or fail its record.
        async fn settled(&self) -> Option<IntentRecord> {
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    match self.record() {
                        None => return None,
                        Some(record) if record.state == IntentState::Failed => {
                            return Some(record);
                        }
                        Some(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                    }
                }
            })
            .await
            .expect("the operation settles")
        }
    }

    fn code(result: Result<Value, AgentdError>) -> String {
        match result {
            Ok(value) => format!("ok {value}"),
            Err(error) => error.public_code().to_owned(),
        }
    }

    /// A saved route is configured: its settings and credential are stored.
    fn route_configured(setup: &Setup) -> bool {
        let model = setup.host.model();
        match classify_saved_route(&model, Some(FP_URL)) {
            SavedRoute::FinitePrivate => fp().settings().is_some(),
            SavedRoute::Openrouter => setup
                .inference
                .connections
                .stored_openrouter_key()
                .unwrap()
                .is_some(),
            SavedRoute::OpenaiCodex | SavedRoute::Other => false,
        }
    }

    // ---- T-A41: every dispatch arm calls `admit` -------------------------

    fn every_command() -> Vec<(&'static str, &'static str, Value)> {
        vec![
            ("agent.connections.status", EMPTY_REQUEST_SCHEMA, json!({})),
            (
                "agent.inference.apply",
                INFERENCE_APPLY_SCHEMA,
                json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL}),
            ),
            (
                "agent.inference.select",
                INFERENCE_SELECT_SCHEMA,
                json!({"route": "finite_private", "model": null}),
            ),
            (
                "agent.inference.disconnect",
                INFERENCE_DISCONNECT_SCHEMA,
                json!({"route": "openrouter"}),
            ),
            ("agent.openrouter.usage", EMPTY_REQUEST_SCHEMA, json!({})),
            (
                "agent.openrouter.connect",
                OPENROUTER_CONNECT_SCHEMA,
                json!({"credential": {"kind": "api_key", "api_key": OR_KEY}, "activate": null}),
            ),
            (
                "agent.codex.login.start",
                CODEX_LOGIN_START_SCHEMA,
                json!({}),
            ),
            (
                "agent.codex.login.cancel",
                CODEX_LOGIN_CANCEL_SCHEMA,
                json!({"attempt_id": format!("cxl_{}", "0".repeat(32))}),
            ),
            ("agent.codex.models", EMPTY_REQUEST_SCHEMA, json!({})),
        ]
    }

    #[test]
    fn the_table_covers_the_nine_commands() {
        let names = every_command()
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>();
        assert_eq!(names, INFERENCE_COMMANDS);
    }

    #[tokio::test]
    async fn t_a41_every_dispatch_arm_reads_the_intent_through_admit() {
        for (command, schema, body) in every_command() {
            let setup = new_setup(&fp_block(), "");
            // A corrupt record is quarantined by the only reader, `admit`.
            let path = setup.inference.intent_path.clone();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"{not a record").unwrap();
            let _ = setup.run(command, schema, body).await;
            let quarantined = fs::read_dir(path.parent().unwrap())
                .unwrap()
                .filter_map(|entry| entry.ok())
                .any(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("inference-intent.json.corrupt-")
                });
            assert!(quarantined, "{command} did not call admit");
        }
    }

    #[tokio::test]
    async fn t_a41_admission_at_the_dispatch_level() {
        for (command, schema, body) in every_command() {
            // A running operation refuses every mutation, and nothing else.
            let setup = new_setup(&fp_block(), "");
            setup.store(
                IntentKind::Select,
                IntentRoute::Openrouter,
                IntentState::Running,
            );
            let result = code(setup.run(command, schema, body.clone()).await);
            let mutating = matches!(
                command,
                "agent.inference.apply"
                    | "agent.inference.select"
                    | "agent.inference.disconnect"
                    | "agent.openrouter.connect"
            );
            assert_eq!(
                result == "operation_in_progress",
                mutating,
                "{command} with a running select: {result}"
            );

            // F2: a failed Codex disconnect refuses a new sign-in, first.
            let setup = new_setup(&fp_block(), "");
            setup.store(
                IntentKind::Disconnect,
                IntentRoute::OpenaiCodex,
                IntentState::Failed,
            );
            let result = code(setup.run(command, schema, body).await);
            match command {
                "agent.codex.login.start" => assert_eq!(result, "disconnect_in_progress"),
                "agent.inference.apply"
                | "agent.inference.select"
                | "agent.inference.disconnect"
                | "agent.openrouter.connect" => {
                    assert_eq!(result, "operation_in_progress", "{command}")
                }
                _ => assert_ne!(result, "operation_in_progress", "{command}"),
            }
        }
        // A running Codex disconnect refuses sign-in too; a failed OpenRouter
        // disconnect refuses it as another operation.
        let setup = new_setup(&fp_block(), "");
        setup.store(
            IntentKind::Disconnect,
            IntentRoute::OpenaiCodex,
            IntentState::Running,
        );
        assert_eq!(
            code(
                setup
                    .run(
                        "agent.codex.login.start",
                        CODEX_LOGIN_START_SCHEMA,
                        json!({})
                    )
                    .await
            ),
            "disconnect_in_progress"
        );
        let setup = new_setup(&fp_block(), "");
        setup.store(
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            IntentState::Failed,
        );
        assert_eq!(
            code(
                setup
                    .run(
                        "agent.codex.login.start",
                        CODEX_LOGIN_START_SCHEMA,
                        json!({})
                    )
                    .await
            ),
            "operation_in_progress"
        );
    }

    #[tokio::test]
    async fn pr2_and_pr3_hooks_answer_unsupported_until_they_land() {
        let setup = new_setup(&fp_block(), "");
        for (command, schema, body) in every_command().into_iter().skip(4) {
            assert_eq!(
                code(setup.run(command, schema, body).await),
                "unsupported_command",
                "{command}"
            );
        }
        let status = setup.status().await;
        assert_eq!(
            status["capabilities"],
            json!([
                "inference.status.v2",
                "inference.select.v1",
                "inference.disconnect.v1"
            ])
        );
        // Codex actions need codex.login.v1, which PR1 does not advertise.
        assert_eq!(
            code(setup.disconnect("openai_codex").await),
            "invalid_payload"
        );
        assert_eq!(
            code(
                setup
                    .select(json!({"route": "openai_codex", "model": "gpt-5.5"}))
                    .await
            ),
            "invalid_payload"
        );
        assert!(status["inference"]["routes"].get("openai_codex").is_none());
    }

    // ---- Status (§3.4), T-A11 --------------------------------------------

    #[tokio::test]
    async fn status_keeps_the_legacy_fields_and_adds_stored_facts_at_the_documented_places() {
        let setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
        );
        let status = setup.status().await;
        let inference = &status["inference"];
        assert_eq!(inference["profile"], "openrouter");
        assert_eq!(inference["provider"], "openrouter");
        assert_eq!(inference["model"], OR_MODEL);
        assert_eq!(inference["saved"]["route"], "openrouter");
        assert_eq!(inference["routes"]["openrouter"]["state"], "key_saved");
        assert_eq!(inference["routes"]["openrouter"]["key_source"], "agent");
        assert_eq!(
            inference["routes"]["openrouter"]["key_hash"],
            crate::ledger::hex_digest(OR_KEY.as_bytes())
        );
        assert_eq!(inference["routes"]["finite_private"]["state"], "configured");
        assert_eq!(inference["fallback"]["state"], "configured");
        assert!(inference["operation"].is_null());
        assert!(status["capabilities"].is_array());
        assert!(inference.get("capabilities").is_none());
        // The reply goes through `serde_json::Value`, so its key order is sorted,
        // today and after this change; only the values matter.
        let text = serde_json::to_string(&status).unwrap();
        for word in ["\"ready\"", "\"valid\"", "working"] {
            assert!(!text.contains(word), "{word}");
        }
    }

    #[tokio::test]
    async fn t_a11_status_never_contains_a_secret() {
        let legacy = json!({"default": "a/b", "provider": "openrouter", "api_key": "sk-or-v1-synthetic-legacy"});
        let setup = new_setup(
            &legacy,
            &format!("OPENROUTER_API_KEY='{OR_KEY}'\nFINITE_PRIVATE_API_KEY=synthetic-fp-secret\n"),
        );
        setup
            .host
            .facts
            .lock()
            .unwrap()
            .openrouter
            .hermes_key_fingerprint = Some(crate::ledger::hex_digest(OR_KEY.as_bytes()));
        setup.host.facts.lock().unwrap().openrouter.hermes_key = Tri::Present;
        let text = serde_json::to_string(&setup.status().await).unwrap();
        for secret in [
            OR_KEY,
            "sk-or-v1-synthetic-legacy",
            "synthetic-fp-secret",
            "sk-or",
        ] {
            assert!(!text.contains(secret), "{secret} in status");
        }
        assert!(text.contains("\"hermes_key\":\"saved_key\""));
    }

    #[tokio::test]
    async fn a_failing_helper_still_serves_legacy_fields_and_the_operation() {
        let setup = new_setup(&fp_block(), "");
        *setup.host.facts_fail.lock().unwrap() = true;
        let record = setup.store(
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            IntentState::Failed,
        );
        let status = setup.status().await;
        assert_eq!(status["inference"]["profile"], "finite_private");
        assert_eq!(
            status["inference"]["routes"]["finite_private"]["state"],
            "unknown"
        );
        assert_eq!(status["inference"]["fallback"]["state"], "unknown");
        assert_eq!(status["inference"]["operation"]["id"], record.id.as_str());
        assert_eq!(status["inference"]["operation"]["state"], "failed");
    }

    #[tokio::test]
    async fn r15a_a_command_reads_the_facts_with_10_s_and_the_executor_with_30_s() {
        let setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
        );
        setup.disconnect("openrouter").await.unwrap();
        assert!(setup.settled().await.is_none(), "succeeded");
        let deadlines = setup.host.facts_deadlines.lock().unwrap().clone();
        // The command's own read, whose reply the dashboard waits for, then
        // the executor's verification reads.
        assert_eq!(deadlines[0], crate::helper::STATUS_FACTS_DEADLINE);
        assert!(deadlines.len() >= 3, "{deadlines:?}");
        assert!(
            deadlines[1..]
                .iter()
                .all(|deadline| *deadline == crate::helper::EXECUTOR_FACTS_DEADLINE),
            "{deadlines:?}"
        );

        let setup = new_setup(&fp_block(), "");
        setup.status().await;
        assert_eq!(
            *setup.host.facts_deadlines.lock().unwrap(),
            [crate::helper::STATUS_FACTS_DEADLINE]
        );
    }

    #[tokio::test]
    async fn an_unreadable_intent_never_blocks_status() {
        let setup = new_setup(&fp_block(), "");
        let dir = setup.agent_home.join("agentd");
        fs::create_dir_all(&dir).unwrap();
        fs::write(setup.inference.intent_path.clone(), b"{}").unwrap();
        fs::set_permissions(
            &setup.inference.intent_path,
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        let status = setup.status().await;
        assert_eq!(status["inference"]["profile"], "finite_private");
        // A mutation fails closed instead.
        assert_eq!(code(setup.disconnect("openrouter").await), "internal_error");
        fs::set_permissions(
            &setup.inference.intent_path,
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }

    // ---- T-A10: v1 unchanged ---------------------------------------------

    #[tokio::test]
    async fn t_a10_v1_request_and_reply_bytes() {
        let setup = new_setup(&fp_block(), "UNRELATED=kept\n");
        let reply = setup
            .v1(json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL}))
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            "{\"already_applied\":false,\"applied\":true,\"path\":\"model\",\"proposal_id\":\"request-1\",\"restart_required\":true}"
        );
        assert_eq!(setup.host.model(), openrouter_block());
        assert_eq!(
            setup.env(),
            format!("UNRELATED=kept\nOPENROUTER_API_KEY='{OR_KEY}'\n")
        );
        // A new key: the gateway and `hermes serve` both restart.
        assert_eq!(setup.host.events(), ["gateway", "serve"]);

        let setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
        );
        let reply = setup
            .v1(json!({"profile": "finite_private"}))
            .await
            .unwrap();
        assert_eq!(reply["applied"], true);
        assert_eq!(
            setup.host.model(),
            fp_block(),
            "the planner's block, with context_length"
        );
        assert_eq!(setup.host.events(), ["gateway"]);
    }

    #[tokio::test]
    async fn t_a10_v1_no_op_and_changed_key() {
        let setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY='{OR_KEY}'\n"),
        );
        let reply = setup
            .v1(json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL}))
            .await
            .unwrap();
        assert_eq!(
            reply,
            json!({"proposal_id": "request-1", "path": "model", "applied": false,
                   "already_applied": true, "restart_required": false})
        );
        let reply = setup
            .v1(json!({"profile": "openrouter", "model": OR_MODEL}))
            .await
            .unwrap();
        assert_eq!(reply["already_applied"], true);
        assert!(setup.host.events().is_empty(), "a no-op never restarts");

        // A changed key on an unchanged model is not a no-op.
        let reply = setup
            .v1(json!({"profile": "openrouter", "api_key": "sk-or-v1-synthetic-rotated", "model": OR_MODEL}))
            .await
            .unwrap();
        assert_eq!(reply["restart_required"], true);
        assert_eq!(setup.host.events(), ["gateway", "serve"]);
        assert_eq!(
            setup
                .inference
                .connections
                .openrouter_dotenv_key()
                .unwrap()
                .as_deref(),
            Some("sk-or-v1-synthetic-rotated")
        );
    }

    #[tokio::test]
    async fn t_a10_v1_restores_the_env_snapshot_on_check_and_restart_failure() {
        let original_env = "OPENROUTER_API_KEY='sk-or-v1-synthetic-old'\n";
        let setup = new_setup(&fp_block(), original_env);
        let before = setup.config_bytes();
        *setup.host.validate_fail.lock().unwrap() = true;
        let result = setup
            .v1(json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL}))
            .await;
        assert_eq!(code(result), "config_invalid");
        assert_eq!(setup.config_bytes(), before);
        assert_eq!(setup.env(), original_env);

        let setup = new_setup(&fp_block(), original_env);
        *setup.host.on_restart.lock().unwrap() = Box::new(|count| {
            if count == 1 {
                Err(AgentdError::Supervisor("spawn failed".to_owned()))
            } else {
                Ok(())
            }
        });
        let result = setup
            .v1(json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL}))
            .await;
        assert_eq!(code(result), "supervisor_unavailable");
        assert_eq!(setup.config_bytes(), before);
        assert_eq!(setup.env(), original_env);
        assert_eq!(
            setup.host.events(),
            ["gateway", "gateway"],
            "restarted on the previous route"
        );
    }

    #[tokio::test]
    async fn t_a10_v1_admission_and_failed_records() {
        let body = json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL});
        let setup = new_setup(&fp_block(), "");
        setup.store(
            IntentKind::Select,
            IntentRoute::Openrouter,
            IntentState::Running,
        );
        let error = setup.v1(body.clone()).await.unwrap_err();
        assert_eq!(error.public_code(), "operation_in_progress");
        assert_eq!(
            error.public_message(),
            "Another connection change is still finishing. Try again in a moment."
        );
        assert_eq!(setup.host.model(), fp_block(), "nothing written");

        let setup = new_setup(&fp_block(), "");
        setup.store(
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            IntentState::Failed,
        );
        assert_eq!(code(setup.v1(body.clone()).await), "operation_in_progress");

        // A failed select is replaced: a v1 success deletes it...
        let setup = new_setup(&fp_block(), "");
        setup.store(
            IntentKind::Select,
            IntentRoute::Openrouter,
            IntentState::Failed,
        );
        setup.v1(body.clone()).await.unwrap();
        assert!(setup.record().is_none());
        // ...and a v1 failure keeps it.
        let setup = new_setup(&fp_block(), "");
        let failed = setup.store(
            IntentKind::Select,
            IntentRoute::Openrouter,
            IntentState::Failed,
        );
        *setup.host.validate_fail.lock().unwrap() = true;
        assert!(setup.v1(body).await.is_err());
        assert_eq!(setup.record(), Some(failed));
    }

    #[tokio::test]
    async fn r8_v1_verify_is_one_read_and_a_mismatch_is_reported_as_found() {
        let body = json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL});
        // A stale writer changes the model right after the restart.
        let setup = new_setup(&fp_block(), "");
        let path = setup.host.config_path.clone();
        *setup.host.on_restart.lock().unwrap() = Box::new(move |_| {
            let text = fs::read_to_string(&path).unwrap();
            fs::write(&path, text.replace(OR_MODEL, "stale/model")).unwrap();
            Ok(())
        });
        let found = {
            let error = setup.v1(body.clone()).await.unwrap_err();
            assert_eq!(error.public_code(), "config_conflict");
            setup.config_bytes()
        };
        // No re-apply and no second restart: one gateway restart, and the
        // key's `hermes serve` restart, then the reply.
        assert_eq!(*setup.host.restarts.lock().unwrap(), 1);
        assert_eq!(setup.host.events(), ["gateway", "serve"]);
        // Nothing restored: the file and the key are as found.
        assert_eq!(setup.host.model()["default"], "stale/model");
        assert_eq!(setup.config_bytes(), found);
        assert_eq!(
            setup
                .inference
                .connections
                .openrouter_dotenv_key()
                .unwrap()
                .as_deref(),
            Some(OR_KEY)
        );

        // A stale writer that removes the key the request carried.
        let setup = new_setup(&fp_block(), "");
        let env_path = setup.hermes_home.join(".env");
        *setup.host.on_restart.lock().unwrap() = Box::new(move |_| {
            fs::write(&env_path, "").unwrap();
            Ok(())
        });
        assert_eq!(code(setup.v1(body).await), "config_conflict");
        assert_eq!(*setup.host.restarts.lock().unwrap(), 1);
        assert_eq!(setup.env(), "", "not written again");
    }

    #[tokio::test]
    async fn r8_v1_success_makes_one_restart_and_replies_without_waiting() {
        // The reply is no later than before verification existed: exactly one
        // restart call and one immediate read. `Inference` has no verify delay
        // for v1 at all; the only waits are the restarts the host records.
        let setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
        );
        setup
            .v1(json!({"profile": "finite_private"}))
            .await
            .unwrap();
        assert_eq!(setup.host.events(), ["gateway"]);
        assert_eq!(*setup.host.restarts.lock().unwrap(), 1);

        let setup = new_setup(&fp_block(), "");
        setup
            .v1(json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL}))
            .await
            .unwrap();
        assert_eq!(setup.host.events(), ["gateway", "serve"]);
        assert_eq!(*setup.host.restarts.lock().unwrap(), 1);
    }

    #[test]
    fn dotenv_reads_take_the_last_assignment_for_every_caller() {
        let setup = new_setup(&fp_block(), "");
        fs::write(
            setup.hermes_home.join(".env"),
            "OPENROUTER_API_KEY=sk-or-v1-first\nOTHER=1\nOPENROUTER_API_KEY=sk-or-v1-last\n",
        )
        .unwrap();
        let connections = &setup.inference.connections;
        assert_eq!(
            connections.openrouter_dotenv_key().unwrap().as_deref(),
            Some("sk-or-v1-last")
        );
        // v1 reuses the key Hermes uses.
        let plan = connections
            .inference_plan_with(
                "r",
                InferenceApplyRequest {
                    profile: "openrouter".to_owned(),
                    api_key: None,
                    model: None,
                },
                &fp(),
            )
            .unwrap();
        assert!(plan.credential_to_persist().is_none());
        // Removal drops every line and keeps the others' bytes.
        fs::write(
            setup.hermes_home.join(".env"),
            "A=1\r\nOPENROUTER_API_KEY=x\nexport OPENROUTER_API_KEY='y'\n# note\nB=2",
        )
        .unwrap();
        connections.remove_openrouter_key().unwrap();
        assert_eq!(setup.env(), "A=1\r\n# note\nB=2");
        let mode = fs::metadata(setup.hermes_home.join(".env"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // ---- T-A21: select preflight -----------------------------------------

    #[tokio::test]
    async fn t_a21_select_preflight_matrix() {
        let ok = key_data(json!({"limit": null, "limit_remaining": null}));
        let cases: Vec<(&str, Value, &str, Value, u16, String, &str)> = vec![
            (
                "fp",
                fp_block(),
                "",
                json!({"route": "finite_private"}),
                200,
                ok.clone(),
                "changed",
            ),
            (
                "fp from openrouter",
                openrouter_block(),
                "",
                json!({"route": "finite_private", "model": null}),
                200,
                ok.clone(),
                "accepted",
            ),
            (
                "fp with a model",
                openrouter_block(),
                "",
                json!({"route": "finite_private", "model": "x"}),
                200,
                ok.clone(),
                "invalid_payload",
            ),
            (
                "unknown field",
                fp_block(),
                "",
                json!({"route": "finite_private", "extra": 1}),
                200,
                ok.clone(),
                "invalid_payload",
            ),
            (
                "no key",
                fp_block(),
                "",
                json!({"route": "openrouter", "model": OR_MODEL}),
                200,
                ok.clone(),
                "not_connected",
            ),
            (
                "bad model",
                fp_block(),
                "OPENROUTER_API_KEY=k1234567\n",
                json!({"route": "openrouter", "model": "has space"}),
                200,
                ok.clone(),
                "invalid_payload",
            ),
            (
                "missing model",
                fp_block(),
                "OPENROUTER_API_KEY=k1234567\n",
                json!({"route": "openrouter"}),
                200,
                ok.clone(),
                "invalid_payload",
            ),
            (
                "key ok",
                fp_block(),
                "OPENROUTER_API_KEY=k1234567\n",
                json!({"route": "openrouter", "model": OR_MODEL}),
                200,
                ok.clone(),
                "accepted",
            ),
            (
                "key rejected",
                fp_block(),
                "OPENROUTER_API_KEY=k1234567\n",
                json!({"route": "openrouter", "model": OR_MODEL}),
                401,
                ok.clone(),
                "credential_rejected",
            ),
            (
                "provider down",
                fp_block(),
                "OPENROUTER_API_KEY=k1234567\n",
                json!({"route": "openrouter", "model": OR_MODEL}),
                500,
                ok.clone(),
                "provider_unavailable",
            ),
            (
                "management key",
                fp_block(),
                "OPENROUTER_API_KEY=k1234567\n",
                json!({"route": "openrouter", "model": OR_MODEL}),
                200,
                key_data(json!({"is_management_key": true})),
                "credential_rejected",
            ),
            (
                "exhausted",
                fp_block(),
                "OPENROUTER_API_KEY=k1234567\n",
                json!({"route": "openrouter", "model": OR_MODEL}),
                200,
                key_data(json!({"limit": 5, "limit_remaining": 0})),
                "key_allowance_exhausted",
            ),
            (
                "legacy key rejected",
                json!({"default": OR_MODEL, "provider": "openrouter", "api_key": "sk-or-v1-synthetic-legacy"}),
                "",
                json!({"route": "openrouter", "model": OR_MODEL}),
                401,
                ok.clone(),
                "credential_rejected",
            ),
            (
                "legacy key exhausted",
                json!({"default": OR_MODEL, "provider": "openrouter", "api_key": "sk-or-v1-synthetic-legacy"}),
                "",
                json!({"route": "openrouter", "model": OR_MODEL}),
                200,
                key_data(json!({"limit": 5, "limit_remaining": 0})),
                "key_allowance_exhausted",
            ),
            (
                "already saved",
                openrouter_block(),
                "OPENROUTER_API_KEY=k1234567\n",
                json!({"route": "openrouter", "model": OR_MODEL}),
                200,
                ok.clone(),
                "changed",
            ),
        ];
        for (name, model, env, body, status, reply, expected) in cases {
            let mut setup = new_setup(&model, env);
            let fake = FakeOpenRouter::start(status, &reply).await;
            setup.inference.openrouter_api_base = fake.base.clone();
            // Hold the executor so the synchronous part is all that runs.
            *setup.host.hold_restart.lock().unwrap() = Some(Arc::new(Notify::new()));
            let before = (setup.config_bytes(), setup.env());
            let got = match setup.select(body).await {
                Ok(value) if value.get("accepted").is_some() => "accepted".to_owned(),
                Ok(value) if value == json!({"changed": false}) => "changed".to_owned(),
                Ok(value) => format!("unexpected {value}"),
                Err(error) => error.public_code().to_owned(),
            };
            assert_eq!(got, expected, "{name}");
            if expected != "accepted" {
                assert!(setup.record().is_none(), "{name}: no intent");
                assert_eq!(
                    (setup.config_bytes(), setup.env()),
                    before,
                    "{name}: nothing written"
                );
            }
            // Metadata only; never a completion request.
            assert!(
                fake.requests()
                    .iter()
                    .all(|line| line == "GET /api/v1/key HTTP/1.1"),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn select_finite_private_without_settings_is_config_invalid() {
        let mut setup = new_setup(&openrouter_block(), "");
        setup.inference.fp = FinitePrivateEnv::default();
        let error = setup
            .select(json!({"route": "finite_private"}))
            .await
            .unwrap_err();
        assert_eq!(error.public_code(), "config_invalid");
        assert_eq!(
            error.public_message(),
            "Finite Private isn't available on this agent."
        );
    }

    // ---- T-A20: async replies and the operation lifecycle ------------------

    #[tokio::test]
    async fn t_a20_select_replies_accepted_and_status_follows_the_operation() {
        let setup = new_setup(&fp_block(), &format!("OPENROUTER_API_KEY={OR_KEY}\n"));
        let fake = FakeOpenRouter::start(200, &key_data(json!({"limit": null}))).await;
        let mut setup = setup;
        setup.inference.openrouter_api_base = fake.base.clone();
        let hold = Arc::new(Notify::new());
        *setup.host.hold_restart.lock().unwrap() = Some(Arc::clone(&hold));
        let reply = setup
            .select(json!({"route": "openrouter", "model": OR_MODEL}))
            .await
            .unwrap();
        assert_eq!(reply["accepted"], true);
        let id = reply["operation_id"].as_str().unwrap().to_owned();
        // The record exists before the reply.
        assert_eq!(setup.record().unwrap().id, id);
        assert_eq!(setup.record().unwrap().kind, IntentKind::Select);
        let status = setup.status().await;
        assert_eq!(status["inference"]["operation"]["id"], id.as_str());
        assert_eq!(status["inference"]["operation"]["state"], "running");
        assert_eq!(status["inference"]["operation"]["kind"], "select");
        hold.notify_one();
        assert!(setup.settled().await.is_none());
        let status = setup.status().await;
        assert_eq!(status["inference"]["operation"]["id"], id.as_str());
        assert_eq!(status["inference"]["operation"]["state"], "succeeded");
        assert_eq!(status["inference"]["saved"]["route"], "openrouter");
    }

    // ---- T-A19, T-A40: the F1 precondition -------------------------------

    #[tokio::test]
    async fn t_a40_disconnecting_the_saved_route_needs_finite_private_known_present() {
        let env = format!("OPENROUTER_API_KEY={OR_KEY}\n");
        for fp_key in [Tri::Absent, Tri::Unknown] {
            let setup = new_setup(&openrouter_block(), &env);
            setup.host.facts.lock().unwrap().finite_private.fp_key = fp_key;
            let before = (setup.config_bytes(), setup.env());
            let error = setup.disconnect("openrouter").await.unwrap_err();
            assert_eq!(error.public_code(), "finite_private_unavailable");
            assert_eq!(
                error.public_message(),
                "Disconnecting would leave this agent without a model: Finite Private isn't fully set up here. Choose another model first."
            );
            assert_eq!((setup.config_bytes(), setup.env()), before);
            assert!(setup.record().is_none(), "no intent");
        }
        let setup = new_setup(&openrouter_block(), &env);
        *setup.host.hold_restart.lock().unwrap() = Some(Arc::new(Notify::new()));
        let reply = setup.disconnect("openrouter").await.unwrap();
        assert_eq!(reply["accepted"], true);
        assert_eq!(setup.record().unwrap().kind, IntentKind::Disconnect);

        // Not the saved route: no precondition, even with FP unset.
        let mut setup = new_setup(&fp_block(), &env);
        setup.inference.fp = FinitePrivateEnv::default();
        setup.host.facts.lock().unwrap().finite_private.fp_key = Tri::Absent;
        *setup.host.hold_restart.lock().unwrap() = Some(Arc::new(Notify::new()));
        assert_eq!(
            setup.disconnect("openrouter").await.unwrap()["accepted"],
            true
        );
    }

    #[tokio::test]
    async fn t_a19_disconnect_with_finite_private_settings_missing_is_refused() {
        let mut setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
        );
        setup.inference.fp = FinitePrivateEnv::default();
        assert_eq!(
            code(setup.disconnect("openrouter").await),
            "finite_private_unavailable"
        );
        assert!(setup.record().is_none());
    }

    #[tokio::test]
    async fn disconnect_with_nothing_stored_is_a_no_op_and_a_failed_one_resumes() {
        let setup = new_setup(&fp_block(), "");
        assert_eq!(
            setup.disconnect("openrouter").await.unwrap(),
            json!({"changed": false})
        );
        assert!(setup.record().is_none());

        // An unknown fact counts as something stored.
        let setup = new_setup(&fp_block(), "");
        setup
            .host
            .facts
            .lock()
            .unwrap()
            .openrouter
            .manual_pool_entries = PoolEntries::Unknown;
        *setup.host.hold_restart.lock().unwrap() = Some(Arc::new(Notify::new()));
        assert_eq!(
            setup.disconnect("openrouter").await.unwrap()["accepted"],
            true
        );

        // A failed disconnect of the same route is resumed with the same id.
        let setup = new_setup(&fp_block(), "");
        let mut failed =
            IntentRecord::new(IntentKind::Disconnect, IntentRoute::Openrouter, None).unwrap();
        failed.phase = intent::IntentPhase::Verifying;
        failed.state = IntentState::Failed;
        failed.error_code = Some("verify_failed".to_owned());
        failed.attempts = 3;
        intent::store(&setup.inference.intent_path, &failed).unwrap();
        *setup.host.hold_restart.lock().unwrap() = Some(Arc::new(Notify::new()));
        let reply = setup.disconnect("openrouter").await.unwrap();
        assert_eq!(reply["operation_id"], failed.id.as_str());
        let resumed = setup.record().unwrap();
        assert_eq!(resumed.id, failed.id);
        assert_eq!(resumed.phase, intent::IntentPhase::Verifying);
        assert_eq!(resumed.error_code, None);
        assert!(matches!(resumed.state, IntentState::Running));
    }

    // ---- T-A45: real failure paths ---------------------------------------

    #[tokio::test]
    async fn t_a45_real_read_only_directories() {
        // v1 with the config directory read-only: the write really fails and
        // nothing changes.
        let setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
        );
        let before = (setup.config_bytes(), setup.env());
        fs::set_permissions(&setup.hermes_home, fs::Permissions::from_mode(0o500)).unwrap();
        let result = setup.v1(json!({"profile": "finite_private"})).await;
        fs::set_permissions(&setup.hermes_home, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(code(result), "internal_error");
        assert_eq!((setup.config_bytes(), setup.env()), before);
        assert!(setup.host.events().is_empty(), "no restart");

        // select with the intent store read-only: nothing is written.
        let setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
        );
        let store = setup.agent_home.join("agentd");
        fs::create_dir_all(&store).unwrap();
        fs::set_permissions(&store, fs::Permissions::from_mode(0o500)).unwrap();
        let result = setup.select(json!({"route": "finite_private"})).await;
        fs::set_permissions(&store, fs::Permissions::from_mode(0o700)).unwrap();
        let error = result.unwrap_err();
        assert_eq!(error.public_code(), "config_invalid");
        assert_eq!(
            error.public_message(),
            "The agent couldn't record this change."
        );
        assert_eq!((setup.config_bytes(), setup.env()), before);
        assert!(setup.record().is_none());

        // select with the config directory read-only: accepted, then the
        // background write really fails and the config is untouched.
        let mut setup = new_setup(&fp_block(), &format!("OPENROUTER_API_KEY={OR_KEY}\n"));
        let fake = FakeOpenRouter::start(200, &key_data(json!({"limit": null}))).await;
        setup.inference.openrouter_api_base = fake.base.clone();
        let before = (setup.config_bytes(), setup.env());
        fs::set_permissions(&setup.hermes_home, fs::Permissions::from_mode(0o500)).unwrap();
        let reply = setup
            .select(json!({"route": "openrouter", "model": OR_MODEL}))
            .await
            .unwrap();
        assert_eq!(reply["accepted"], true);
        // The positive signal that the write was attempted: the executor calls
        // `migrate` and then `write_model` in one poll, with no await between,
        // on this single-threaded test runtime. Once `migrate` is recorded, the
        // write has already failed and the next attempt is 15 s away.
        let host = setup.host.clone();
        eventually("the background write attempt", || {
            host.events().contains(&"migrate".to_owned())
        })
        .await;
        let unchanged = (setup.config_bytes(), setup.env()) == before;
        let record = setup.record().unwrap();
        let events = setup.host.events();
        fs::set_permissions(&setup.hermes_home, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(unchanged, "the failed write left the config as it was");
        assert_eq!(record.phase, intent::IntentPhase::ConfigWritten);
        assert_eq!(
            record.state,
            IntentState::Running,
            "retried after its backoff"
        );
        assert_eq!(events, ["migrate"], "no restart after a failed write");
    }

    /// A host that restarts a real supervisor; the gateway program is deleted
    /// so the next spawn really fails, then put back for the rollback restart.
    #[derive(Clone)]
    struct RealSupervisorHost {
        inner: TestHost,
        supervisor: SupervisorHandle,
        program: PathBuf,
    }

    fn write_sleeper(path: &Path, marker: Option<&Path>) {
        let touch = marker
            .map(|marker| format!("touch '{}'\n", marker.display()))
            .unwrap_or_default();
        fs::write(path, format!("#!/bin/sh\n{touch}exec sleep 60\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    impl ExecutorHost for RealSupervisorHost {
        fn validate_config(&self) -> Result<(), AgentdError> {
            Ok(())
        }
        fn dotenv_openrouter_key(&self) -> Result<Option<String>, AgentdError> {
            self.inner.dotenv_openrouter_key()
        }
        fn migrate_legacy_openrouter_key(&self) -> Result<(), AgentdError> {
            self.inner.migrate_legacy_openrouter_key()
        }
        fn remove_openrouter_key(&self) -> Result<(), AgentdError> {
            self.inner.remove_openrouter_key()
        }
        async fn restart_gateway(&self) -> Result<(), AgentdError> {
            let result = self.supervisor.restart_hermes().await;
            write_sleeper(&self.program, None);
            result
        }
        async fn restart_serve(&self) {}
        async fn facts(&self, deadline: Duration) -> Result<InferenceFacts, AgentdError> {
            self.inner.facts(deadline).await
        }
        async fn cancel_codex_login(&self) -> Result<(), AgentdError> {
            Ok(())
        }
    }

    fn sleeper_spec(name: &'static str, program: &Path) -> ProcessSpec {
        ProcessSpec {
            name,
            program: program.to_owned(),
            args: Vec::new(),
            environment: BTreeMap::new(),
        }
    }

    async fn wait_for_hermes(supervisor: &SupervisorHandle) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while supervisor
                .status()
                .await
                .processes
                .get("hermes")
                .and_then(|status| status.pid())
                .is_none()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn t_a45_a_real_spawn_failure_rolls_v1_back() {
        let base = new_setup(&fp_block(), "OPENROUTER_API_KEY='sk-or-v1-synthetic-old'\n");
        let program = base.hermes_home.join("gateway");
        write_sleeper(&program, None);
        let supervisor = start_supervisor(
            sleeper_spec("finitechat", &program),
            sleeper_spec("health", &program),
            sleeper_spec("hermes", &program),
            None,
        );
        wait_for_hermes(&supervisor).await;
        let host = RealSupervisorHost {
            inner: base.host.clone(),
            supervisor: supervisor.clone(),
            program: program.clone(),
        };
        let inference = Inference::new(
            host,
            base.inference.connections.clone(),
            base.inference.config.clone(),
            base.hermes_home.clone(),
            base.inference.intent_path.clone(),
            fp(),
            Arc::new(CodexState::default()),
        );
        let before = (base.config_bytes(), base.env());
        fs::remove_file(&program).unwrap();
        let result = inference
            .execute(&request(
                "agent.inference.apply",
                INFERENCE_APPLY_SCHEMA,
                json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL}),
            ))
            .await
            .unwrap();
        assert_eq!(code(result), "supervisor_unavailable");
        assert_eq!(
            (base.config_bytes(), base.env()),
            before,
            "config and .env restored"
        );
        supervisor.shutdown().await;
    }

    // ---- Startup order and bad intents ------------------------------------

    struct Started {
        supervisor: SupervisorHandle,
        marker: PathBuf,
        resume: tokio::task::JoinHandle<()>,
    }

    /// Polls `condition` for up to 10 s; fails the test if it never holds.
    async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }

    impl Started {
        /// The positive signal that startup is done: the resume task, which
        /// runs only after the supervisor reported Hermes started, finished.
        async fn resumed(&mut self) {
            tokio::time::timeout(Duration::from_secs(30), &mut self.resume)
                .await
                .expect("the resume finishes")
                .unwrap();
        }
    }

    /// Starts a real supervisor whose Hermes touches `marker`, then the
    /// resume, exactly as `run_daemon` orders them.
    fn start_like_run_daemon(setup: &Setup) -> Started {
        let program = setup.hermes_home.join("hermes-gateway");
        let marker = setup.hermes_home.join("hermes-started");
        write_sleeper(&program, Some(&marker));
        let quiet = setup.hermes_home.join("quiet");
        write_sleeper(&quiet, None);
        *setup.host.started_marker.lock().unwrap() = Some(marker.clone());
        let supervisor = start_supervisor(
            sleeper_spec("finitechat", &quiet),
            sleeper_spec("health", &quiet),
            sleeper_spec("hermes", &program),
            None,
        );
        let resume = resume_intent_after_hermes_starts(
            Arc::clone(&setup.inference.executor),
            supervisor.clone(),
        );
        Started {
            supervisor,
            marker,
            resume,
        }
    }

    #[tokio::test]
    async fn hermes_starts_first_and_a_bad_intent_never_stops_it() {
        // Corrupt: quarantined, nothing runs.
        let setup = new_setup(&fp_block(), "");
        fs::create_dir_all(setup.agent_home.join("agentd")).unwrap();
        fs::write(&setup.inference.intent_path, b"not json").unwrap();
        let mut started = start_like_run_daemon(&setup);
        started.resumed().await;
        let marker = started.marker.clone();
        eventually("Hermes to start", || marker.exists()).await;
        let quarantined = fs::read_dir(setup.agent_home.join("agentd"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("inference-intent.json.corrupt-")
            });
        assert!(quarantined, "the resume read the record");
        assert!(setup.record().is_none());
        assert!(setup.host.events().is_empty(), "no step ran");
        started.supervisor.shutdown().await;

        // Unreadable: logged, nothing runs, Hermes keeps running.
        let setup = new_setup(&fp_block(), "");
        setup.store(
            IntentKind::Select,
            IntentRoute::FinitePrivate,
            IntentState::Running,
        );
        fs::set_permissions(
            &setup.inference.intent_path,
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        let mut started = start_like_run_daemon(&setup);
        started.resumed().await;
        let marker = started.marker.clone();
        eventually("Hermes to start", || marker.exists()).await;
        assert!(setup.host.events().is_empty(), "no step ran");
        assert!(
            started.supervisor.status().await.processes["hermes"]
                .pid()
                .is_some()
        );
        fs::set_permissions(
            &setup.inference.intent_path,
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        started.supervisor.shutdown().await;

        // Failing: ends failed; Hermes was started first and keeps running.
        let setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
        );
        setup.store(
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            IntentState::Running,
        );
        *setup.host.on_restart.lock().unwrap() =
            Box::new(|_| Err(AgentdError::Supervisor("spawn failed".to_owned())));
        let mut started = start_like_run_daemon(&setup);
        started.resumed().await;
        let failed = setup.record().unwrap();
        assert_eq!(failed.state, IntentState::Failed);
        assert_eq!(failed.error_code.as_deref(), Some("supervisor_unavailable"));
        // The first step's event() already required the marker.
        assert!(started.marker.exists());
        assert!(
            started.supervisor.status().await.processes["hermes"]
                .pid()
                .is_some()
        );
        started.supervisor.shutdown().await;
    }

    // ---- T-A18: partial-state kill points (§3.10) --------------------------

    /// Restart agentd over the state a crash left: Hermes starts first, the
    /// saved route is configured at that moment, and the operation converges
    /// or ends failed with a code.
    async fn restart_over(setup: &Setup) -> Option<IntentRecord> {
        assert!(route_configured(setup), "configured when Hermes starts");
        // The launcher step on the start after the crash (F1).
        setup.host.launcher_step();
        let started = start_like_run_daemon(setup);
        let settled = setup.settled().await;
        assert!(started.marker.exists());
        assert!(route_configured(setup), "configured after the operation");
        started.supervisor.shutdown().await;
        settled
    }

    fn crashed_at(setup: &Setup, kind: IntentKind, route: IntentRoute, phase: intent::IntentPhase) {
        let mut record = IntentRecord::new(
            kind,
            route,
            (kind != IntentKind::Disconnect && route == IntentRoute::Openrouter)
                .then(|| OR_MODEL.to_owned()),
        )
        .unwrap();
        record.phase = phase;
        intent::store(&setup.inference.intent_path, &record).unwrap();
    }

    #[tokio::test]
    async fn t_a18_select_crashed_after_the_intent_was_recorded() {
        let setup = new_setup(&fp_block(), &format!("OPENROUTER_API_KEY={OR_KEY}\n"));
        crashed_at(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            intent::IntentPhase::Accepted,
        );
        assert!(restart_over(&setup).await.is_none());
        assert_eq!(setup.host.model(), openrouter_block());
    }

    #[tokio::test]
    async fn t_a18_select_crashed_after_the_legacy_key_migration() {
        let legacy = json!({"default": OR_MODEL, "provider": "openrouter", "api_key": OR_KEY});
        let setup = new_setup(&legacy, &format!("OPENROUTER_API_KEY='{OR_KEY}'\n"));
        crashed_at(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            intent::IntentPhase::ConfigWritten,
        );
        assert!(restart_over(&setup).await.is_none());
        assert_eq!(
            setup.host.model(),
            openrouter_block(),
            "the legacy copy is gone"
        );
        assert_eq!(
            setup
                .inference
                .connections
                .openrouter_dotenv_key()
                .unwrap()
                .as_deref(),
            Some(OR_KEY)
        );
    }

    #[tokio::test]
    async fn t_a18_select_crashed_after_the_model_write() {
        let setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
        );
        crashed_at(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            intent::IntentPhase::Restarting,
        );
        assert!(restart_over(&setup).await.is_none());
        assert_eq!(
            setup.host.events().first().map(String::as_str),
            Some("gateway")
        );
    }

    #[tokio::test]
    async fn t_a18_select_spawn_failure_after_the_write_restores_the_previous_route() {
        let setup = new_setup(&fp_block(), &format!("OPENROUTER_API_KEY={OR_KEY}\n"));
        let before = setup.config_bytes();
        crashed_at(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            intent::IntentPhase::Accepted,
        );
        *setup.host.on_restart.lock().unwrap() = Box::new(|count| {
            if count == 1 {
                Err(AgentdError::Supervisor("spawn failed".to_owned()))
            } else {
                Ok(())
            }
        });
        let failed = restart_over(&setup).await.unwrap();
        assert_eq!(failed.error_code.as_deref(), Some("supervisor_unavailable"));
        assert_eq!(setup.config_bytes(), before);
    }

    #[tokio::test]
    async fn t_a18_disconnect_crashed_at_accepted_or_login_cancelled_skips_the_clears() {
        for phase in [
            intent::IntentPhase::Accepted,
            intent::IntentPhase::LoginCancelled,
        ] {
            let setup = new_setup(
                &openrouter_block(),
                &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
            );
            setup
                .host
                .facts
                .lock()
                .unwrap()
                .openrouter
                .manual_pool_entries = PoolEntries::Present;
            crashed_at(
                &setup,
                IntentKind::Disconnect,
                IntentRoute::Openrouter,
                phase,
            );
            // The start after the crash: the launcher skips, the route is intact.
            assert_eq!(setup.host.launcher_step(), "skipped", "{phase:?}");
            assert_eq!(
                setup
                    .host
                    .facts
                    .lock()
                    .unwrap()
                    .openrouter
                    .manual_pool_entries,
                PoolEntries::Present
            );
            assert!(restart_over(&setup).await.is_none(), "{phase:?}");
            assert_eq!(setup.host.model(), fp_block());
            assert_eq!(
                setup.inference.connections.openrouter_dotenv_key().unwrap(),
                None
            );
        }
    }

    #[tokio::test]
    async fn t_a18_disconnect_crashed_after_each_later_step() {
        for phase in [
            intent::IntentPhase::RouteSwitched,
            intent::IntentPhase::CredentialRemoved,
            intent::IntentPhase::Cleanup,
        ] {
            let env = if phase == intent::IntentPhase::RouteSwitched {
                format!("OPENROUTER_API_KEY={OR_KEY}\n")
            } else {
                String::new()
            };
            let setup = new_setup(&fp_block(), &env);
            setup
                .host
                .facts
                .lock()
                .unwrap()
                .openrouter
                .manual_pool_entries = PoolEntries::Present;
            crashed_at(
                &setup,
                IntentKind::Disconnect,
                IntentRoute::Openrouter,
                phase,
            );
            assert!(restart_over(&setup).await.is_none(), "{phase:?}");
            assert_eq!(
                setup.inference.connections.openrouter_dotenv_key().unwrap(),
                None
            );
        }
    }

    #[tokio::test]
    async fn t_a18_partial_launcher_clears_end_verify_failed() {
        let setup = new_setup(
            &openrouter_block(),
            &format!("OPENROUTER_API_KEY={OR_KEY}\n"),
        );
        setup
            .host
            .facts
            .lock()
            .unwrap()
            .session_overrides
            .openrouter = Tri::Present;
        *setup.host.launcher_clears_overrides.lock().unwrap() = false;
        crashed_at(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            intent::IntentPhase::Accepted,
        );
        let failed = restart_over(&setup).await.unwrap();
        assert_eq!(failed.error_code.as_deref(), Some("verify_failed"));
        assert_eq!(
            setup.host.model(),
            fp_block(),
            "Finite Private stays the saved default"
        );
    }

    #[test]
    fn the_hermes_process_is_told_where_the_intent_is() {
        let config = DaemonConfig {
            agent_home: PathBuf::from("/data/agent"),
            hermes_home: PathBuf::from("/data/agent/hermes-home"),
            bridge_url: "http://127.0.0.1:37633".to_owned(),
            bridge_addr: "127.0.0.1:37633".to_owned(),
            finitechat_bin: PathBuf::from("/usr/local/bin/finitechat"),
            prepare_command: PathBuf::from("/opt/run_hermes_gateway.sh"),
            hermes_command: PathBuf::from("/opt/run_hermes_gateway.sh"),
            health_python: PathBuf::from("python"),
            health_script: PathBuf::from("/opt/health_server.py"),
            authorized_accounts: BTreeSet::new(),
            sidecar_admission_default: None,
            bridge_ready_timeout: Duration::from_secs(1),
        };
        let spec = hermes_spec(&config, &intent::intent_path(&config.agent_home));
        assert_eq!(
            spec.environment["FINITE_AGENTD_INTENT_PATH"],
            "/data/agent/agentd/inference-intent.json"
        );
        assert_eq!(spec.environment["FINITE_AGENTD_SUPERVISED"], "1");
    }
}
