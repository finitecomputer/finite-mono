use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
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
use crate::config::{
    AeonSpecializationDesiredStateV1, ConfigManager, HermesConfigOfferV1, HermesConfigRollbackV1,
};
use crate::connections::{
    ConnectionManager, GoogleApplyRequest, InferenceApplyRequest, TelegramApproveRequest,
    TelegramConnectRequest, TelegramHomeRequest,
};
use crate::ledger::{CommandDecision, Ledger};
use crate::supervisor::{ProcessSpec, SupervisorHandle, SupervisorStatus, start_supervisor};
use crate::transport::BridgeClient;

const STATUS_SCHEMA: &str = "finite.agent.status.v1";
const STATUS_REQUEST_SCHEMA: &str = "finite.agent.status.request.v1";
const EMPTY_REQUEST_SCHEMA: &str = "finite.agent.empty.request.v1";
const CONFIG_OFFER_SCHEMA: &str = "finite.hermes.config.offer.v1";
const CONFIG_ROLLBACK_SCHEMA: &str = "finite.hermes.config.rollback.v1";
const RESULT_SCHEMA: &str = "finite.agent.command.result.v1";
const OWNER_CLAIM_COMMAND: &str = "agent.owner.claim";
const INFERENCE_APPLY_SCHEMA: &str = "finite.agent.inference.apply.v1";
const AEON_SPECIALIZATION_RECONCILE_SCHEMA: &str = "finite.agent.specialization.aeon.reconcile.v1";
const TELEGRAM_CONNECT_SCHEMA: &str = "finite.agent.telegram.connect.v1";
const TELEGRAM_APPROVE_SCHEMA: &str = "finite.agent.telegram.approve.v1";
const TELEGRAM_HOME_SCHEMA: &str = "finite.agent.telegram.home.v1";
const GOOGLE_APPLY_SCHEMA: &str = "finite.agent.google.apply.v1";

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentdStatus {
    pub service: String,
    pub version: String,
    pub account_id: String,
    pub device_id: String,
    pub authorized_principals: usize,
    pub processes: SupervisorStatus,
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
        })
    }

    fn state_dir(&self) -> PathBuf {
        self.agent_home.join("agentd")
    }

    pub fn status_path(&self) -> PathBuf {
        self.state_dir().join("status.json")
    }
}

pub async fn run_daemon(config: DaemonConfig) -> Result<(), AgentdError> {
    fs::create_dir_all(config.state_dir())?;
    fs::set_permissions(config.state_dir(), fs::Permissions::from_mode(0o700))?;
    prepare_agent_runtime(&config)?;
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
    let supervisor = start_supervisor(
        sidecar_spec(&config),
        health_spec(&config),
        hermes_spec(&config),
    );
    spawn_status_writer(
        config.status_path(),
        identity.clone(),
        ledger.clone(),
        supervisor.clone(),
    );

    wait_for_bridge(&bridge).await?;
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
    };

    let delivery_worker =
        run_delivery_loop(delivery_rx, |delivery| executor.handle_delivery(delivery));
    tokio::pin!(delivery_worker);
    tokio::select! {
        result = &mut delivery_worker => {
            result
        }
        signal = tokio::signal::ctrl_c() => {
            signal?;
            supervisor.shutdown().await;
            Ok(())
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
        match request.command.as_str() {
            "agent.status.inspect" => {
                parse_body::<EmptyRequest>(request, STATUS_REQUEST_SCHEMA)?;
                Ok(serde_json::to_value(self.current_status().await)?)
            }
            OWNER_CLAIM_COMMAND => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                Ok(json!({ "connected": true }))
            }
            "agent.hermes.restart" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                self.supervisor.restart_hermes().await?;
                Ok(json!({ "restart": "requested" }))
            }
            "agent.chat.recover" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                self.bridge.recover_chat().await
            }
            "agent.connections.status" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                Ok(serde_json::to_value(self.connection_manager.status()?)?)
            }
            "agent.inference.apply" => {
                let body = parse_body::<InferenceApplyRequest>(request, INFERENCE_APPLY_SCHEMA)?;
                let offer = self
                    .connection_manager
                    .inference_offer(&request.request_id, body)?;
                self.apply_config_offer(offer).await
            }
            "agent.specialization.aeon.reconcile" => {
                let desired = parse_body::<AeonSpecializationDesiredStateV1>(
                    request,
                    AEON_SPECIALIZATION_RECONCILE_SCHEMA,
                )?;
                let manager = self.config_manager.clone();
                let hermes_home = self.hermes_home.clone();
                let desired_for_apply = desired.clone();
                let mut result = tokio::task::spawn_blocking(move || {
                    manager.reconcile_aeon_specialization(&desired_for_apply, || {
                        validate_hermes_config(&hermes_home)
                    })
                })
                .await
                .map_err(|error| AgentdError::Config(error.to_string()))??;
                if result.applied
                    && let Err(error) = self.supervisor.restart_hermes().await
                {
                    let rollback = HermesConfigRollbackV1 {
                        proposal_id: desired.proposal_id.clone(),
                    };
                    let manager = self.config_manager.clone();
                    let hermes_home = self.hermes_home.clone();
                    tokio::task::spawn_blocking(move || {
                        manager.rollback(&rollback, || validate_hermes_config(&hermes_home))
                    })
                    .await
                    .map_err(|rollback_error| {
                        AgentdError::Config(format!(
                            "Hermes reload and specialization rollback failed: {rollback_error}"
                        ))
                    })??;
                    if let Err(restore_error) = self.supervisor.restart_hermes().await {
                        return Err(AgentdError::Supervisor(format!(
                            "Hermes specialization activation failed ({error}); previous configuration was restored on disk but could not be reactivated ({restore_error})"
                        )));
                    }
                    return Err(error);
                }
                let hermes_home = self.hermes_home.clone();
                tokio::task::spawn_blocking(move || validate_hermes_config(&hermes_home))
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                if !self.config_manager.aeon_specialization_matches(&desired)? {
                    let rollback = HermesConfigRollbackV1 {
                        proposal_id: desired.proposal_id.clone(),
                    };
                    let manager = self.config_manager.clone();
                    let hermes_home = self.hermes_home.clone();
                    tokio::task::spawn_blocking(move || {
                        manager.rollback(&rollback, || validate_hermes_config(&hermes_home))
                    })
                    .await
                    .map_err(|rollback_error| {
                        AgentdError::Config(format!(
                            "specialization read-back and rollback failed: {rollback_error}"
                        ))
                    })??;
                    self.supervisor.restart_hermes().await?;
                    return Err(AgentdError::Config(
                        "Hermes specialization read-back did not match desired state; previous configuration was restored"
                            .to_owned(),
                    ));
                }
                result.effective_matches_desired = true;
                Ok(serde_json::to_value(result)?)
            }
            "agent.telegram.connect" => {
                let body = parse_body::<TelegramConnectRequest>(request, TELEGRAM_CONNECT_SCHEMA)?;
                let offer = self
                    .connection_manager
                    .telegram_connect_offer(&request.request_id, body)?;
                self.apply_config_offer(offer).await
            }
            "agent.telegram.approve" => {
                let body = parse_body::<TelegramApproveRequest>(request, TELEGRAM_APPROVE_SCHEMA)?;
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
            "agent.hermes.config.preview" => {
                let offer = parse_body::<HermesConfigOfferV1>(request, CONFIG_OFFER_SCHEMA)?;
                Ok(serde_json::to_value(self.config_manager.preview(&offer)?)?)
            }
            "agent.hermes.config.apply" => {
                let offer = parse_body::<HermesConfigOfferV1>(request, CONFIG_OFFER_SCHEMA)?;
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
            "agent.hermes.config.rollback" => {
                let rollback =
                    parse_body::<HermesConfigRollbackV1>(request, CONFIG_ROLLBACK_SCHEMA)?;
                let manager = self.config_manager.clone();
                let hermes_home = self.hermes_home.clone();
                let result = tokio::task::spawn_blocking(move || {
                    manager.rollback(&rollback, || validate_hermes_config(&hermes_home))
                })
                .await
                .map_err(|error| AgentdError::Config(error.to_string()))??;
                if result.restart_required {
                    self.supervisor.restart_hermes().await?;
                }
                Ok(serde_json::to_value(result)?)
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
        AgentdStatus {
            service: "finite-agentd".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            account_id: self.identity.account_id.clone(),
            device_id: self.identity.device_id.clone(),
            authorized_principals: self.ledger.authorized_principal_count().unwrap_or(0),
            processes: self.supervisor.status().await,
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

fn sidecar_spec(config: &DaemonConfig) -> ProcessSpec {
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
        environment: BTreeMap::new(),
    }
}

fn health_spec(config: &DaemonConfig) -> ProcessSpec {
    ProcessSpec {
        name: "health",
        program: config.health_python.clone(),
        args: vec![config.health_script.display().to_string()],
        environment: BTreeMap::new(),
    }
}

fn hermes_spec(config: &DaemonConfig) -> ProcessSpec {
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

async fn wait_for_bridge(bridge: &BridgeClient) -> Result<(), AgentdError> {
    let mut retry = Duration::from_millis(50);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
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
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let status = AgentdStatus {
                service: "finite-agentd".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                account_id: identity.account_id.clone(),
                device_id: identity.device_id.clone(),
                authorized_principals: ledger.authorized_principal_count().unwrap_or(0),
                processes: supervisor.status().await,
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
