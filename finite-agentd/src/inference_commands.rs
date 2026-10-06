//! The inference contract's commands: status, v1 apply, select,
//! disconnect, and capability-gated provider commands, with the production executor host.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use finitechat_proto::RuntimeCommandRequestV1;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AgentdError;
use crate::config::{
    ConfigApplyResultV1, ConfigManager, HermesConfigRollbackV1, MODEL_CONFIG_PATH,
};
use crate::connections::{
    ConnectionManager, InferenceApplyPlan, InferenceApplyRequest, validate_model_name,
};
use crate::daemon::{
    EMPTY_REQUEST_SCHEMA, EmptyRequest, now_ms, parse_body, validate_hermes_config,
};
use crate::executor::{Executor, ExecutorHost};
use crate::facts::{CodexStateFact, FactsCache, InferenceFacts, PoolEntries, Tri};
use crate::hosted_hermes::HostedHermesHandle;
use crate::inference::{
    FinitePrivateEnv, SavedRoute, capabilities, classify_saved_route, plan_model_block,
};
use crate::intent::{
    self, Admission, AdmitCommand, IntentKind, IntentRecord, IntentRoute, IntentState,
};
use crate::openrouter::ConnectCredential;
use crate::supervisor::SupervisorHandle;

const INFERENCE_APPLY_SCHEMA: &str = "finite.agent.inference.apply.v1";
const INFERENCE_SELECT_SCHEMA: &str = "finite.agent.inference.select.v1";
const INFERENCE_DISCONNECT_SCHEMA: &str = "finite.agent.inference.disconnect.v1";
const OPENROUTER_CONNECT_SCHEMA: &str = "finite.agent.openrouter.connect.v1";
const CODEX_LOGIN_START_SCHEMA: &str = "finite.agent.codex.login.start.v1";
const CODEX_LOGIN_CANCEL_SCHEMA: &str = "finite.agent.codex.login.cancel.v1";
/// The nine commands of the inference contract. Each dispatch arm
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
    #[serde(rename = "credential")]
    _credential: ConnectCredential,
    #[serde(default)]
    #[expect(dead_code, reason = "reserved for OpenRouter connection activation")]
    activate: Option<ConnectActivation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectActivation {
    #[expect(dead_code, reason = "reserved for OpenRouter connection activation")]
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexLoginCancelRequest {
    #[serde(rename = "attempt_id")]
    _attempt_id: String,
}

/// The production executor host: the supervisor, the optional `hermes serve`,
/// the helper, and `connections.rs` for `.env`.
#[derive(Clone)]
pub(crate) struct AgentdHost {
    pub(crate) hermes_home: PathBuf,
    pub(crate) connections: ConnectionManager,
    pub(crate) supervisor: SupervisorHandle,
    pub(crate) hosted_hermes: Option<HostedHermesHandle>,
}

impl ExecutorHost for AgentdHost {
    fn validate_config(&self, deadline: Duration) -> Result<(), AgentdError> {
        validate_hermes_config(&self.hermes_home, deadline)
    }

    fn dotenv_openrouter_key(&self) -> Result<Option<String>, AgentdError> {
        self.connections.openrouter_dotenv_key()
    }

    fn environment_openrouter_key(&self) -> Option<String> {
        crate::connections::environment_openrouter_key()
    }

    fn migrate_legacy_openrouter_key(&self) -> Result<bool, AgentdError> {
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
        // No login manager exists yet; recovery of a saved disconnect still proceeds.
        Ok(())
    }
}

/// Resumes a recorded intent only once Hermes has been started, so a pending,
/// failing, or unreadable intent never delays chat. The
/// daemon detaches the task; tests await it.
pub(crate) fn resume_intent_after_hermes_starts<H: ExecutorHost + Clone>(
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

/// The inference contract's commands: status, v1 apply, select,
/// disconnect, and capability-gated provider commands.
pub(crate) struct Inference<H> {
    host: H,
    pub(crate) executor: Arc<Executor<H>>,
    connections: ConnectionManager,
    config: ConfigManager,
    hermes_home: PathBuf,
    intent_path: PathBuf,
    fp: FinitePrivateEnv,
    facts: FactsCache,
    openrouter_api_base: String,
    /// v1's reply must fit the dashboard's wait: its config check gets what is
    /// left of this from the command's receipt ...
    v1_reply_budget: Duration,
    /// ... and never less than this.
    v1_min_config_check: Duration,
    serve_restart_wait: Duration,
}

/// v1's config check ends by this long after the command arrived.
const V1_REPLY_BUDGET: Duration = Duration::from_secs(40);
/// the shortest config check v1 ever allows.
const V1_MIN_CONFIG_CHECK: Duration = Duration::from_secs(5);

/// What is left of `budget` since `received`, and at least `minimum`.
fn remaining_budget(received: Instant, budget: Duration, minimum: Duration) -> Duration {
    budget.saturating_sub(received.elapsed()).max(minimum)
}

impl<H: ExecutorHost + Clone> Inference<H> {
    pub(crate) fn new(
        host: H,
        connections: ConnectionManager,
        config: ConfigManager,
        hermes_home: PathBuf,
        intent_path: PathBuf,
        fp: FinitePrivateEnv,
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
            openrouter_api_base: crate::openrouter::api_base(),
            v1_reply_budget: V1_REPLY_BUDGET,
            v1_min_config_check: V1_MIN_CONFIG_CHECK,
            serve_restart_wait: crate::executor::SERVE_RESTART_WAIT,
        }
    }

    /// `None` when the command is not one of `INFERENCE_COMMANDS`.
    pub(crate) async fn execute(
        &self,
        request: &RuntimeCommandRequestV1,
    ) -> Option<Result<Value, AgentdError>> {
        if !INFERENCE_COMMANDS.contains(&request.command.as_str()) {
            return None;
        }
        Some(self.dispatch(request, Instant::now()).await)
    }

    async fn dispatch(
        &self,
        request: &RuntimeCommandRequestV1,
        received: Instant,
    ) -> Result<Value, AgentdError> {
        match request.command.as_str() {
            "agent.connections.status" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                self.admit(AdmitCommand::Status)?;
                self.status().await
            }
            "agent.inference.apply" => {
                let body = parse_body::<InferenceApplyRequest>(request, INFERENCE_APPLY_SCHEMA)?;
                let (_, admission) = self.admit(AdmitCommand::V1Apply)?;
                self.v1_apply(&request.request_id, body, admission, received)
                    .await
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
                parse_body::<ConnectRequest>(request, OPENROUTER_CONNECT_SCHEMA)?;
                self.admit(AdmitCommand::Connect)?;
                Err(AgentdError::UnsupportedCommand(request.command.clone()))
            }
            "agent.codex.login.start" => {
                parse_body::<EmptyRequest>(request, CODEX_LOGIN_START_SCHEMA)?;
                self.admit(AdmitCommand::CodexLoginStart)?;
                Err(AgentdError::UnsupportedCommand(request.command.clone()))
            }
            "agent.codex.login.cancel" => {
                parse_body::<CodexLoginCancelRequest>(request, CODEX_LOGIN_CANCEL_SCHEMA)?;
                self.admit(AdmitCommand::CodexLoginCancel)?;
                Err(AgentdError::UnsupportedCommand(request.command.clone()))
            }
            "agent.codex.models" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                self.admit(AdmitCommand::CodexModels)?;
                Err(AgentdError::UnsupportedCommand(request.command.clone()))
            }
            command => Err(AgentdError::UnsupportedCommand(command.to_owned())),
        }
    }

    /// The shared admission check: the current record through
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
        self.read_facts()
            .await
            .unwrap_or_else(InferenceFacts::unknown)
    }

    /// The facts through the status cache, or `None` when the read failed or
    /// timed out, including a failure remembered for 15 s.
    async fn read_facts(&self) -> Option<InferenceFacts> {
        self.facts
            .get_or_fetch(&self.hermes_home, || {
                self.host.facts(crate::helper::STATUS_FACTS_DEADLINE)
            })
            .await
    }

    /// legacy fields unchanged, plus stored facts and the operation.
    async fn status(&self) -> Result<Value, AgentdError> {
        let facts = self.facts().await;
        // Re-read after the helper ran, so the operation is current.
        let record = intent::load(&self.intent_path).unwrap_or(None);
        let operation = self.executor.operation(record.as_ref());
        let manager = self.connections.clone();
        let fp = self.fp.clone();
        let status = tokio::task::spawn_blocking(move || {
            manager.status_with_inference(&facts, &fp, operation, None, capabilities())
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

    /// v1: today's order and failure behavior, including the `.env`
    /// snapshot restore, plus the no-op, the `hermes serve` restart for a
    /// replaced key, and verification after the restart.
    async fn v1_apply(
        &self,
        request_id: &str,
        body: InferenceApplyRequest,
        admission: Admission,
        received: Instant,
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
        let result = self.apply_inference_plan(&plan, received).await?;
        self.clear_failed_record(admission);
        Ok(result)
    }

    /// The config check's deadline for a v1 apply received at `received`.
    fn v1_config_check_deadline(&self, received: Instant) -> Duration {
        remaining_budget(received, self.v1_reply_budget, self.v1_min_config_check)
    }

    async fn apply_inference_plan(
        &self,
        plan: &InferenceApplyPlan,
        received: Instant,
    ) -> Result<Value, AgentdError> {
        let credential_snapshot = self.connections.stage_inference_credential(plan)?;
        let credential_replaced = credential_snapshot.is_some();
        let proposal_id = plan.offer.proposal_id.clone();
        let manager = self.config.clone();
        let host = self.host.clone();
        let offer = plan.offer.clone();
        let deadline = self.v1_config_check_deadline(received);
        let apply = tokio::task::spawn_blocking(move || {
            manager.apply(&offer, || host.validate_config(deadline))
        })
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
            let deadline = self.v1_config_check_deadline(received);
            let rollback_result = tokio::task::spawn_blocking(move || {
                manager.rollback(&rollback, || host.validate_config(deadline))
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
            // Bounded: the verification below decides the reply.
            let wait = self.serve_restart_wait;
            if tokio::time::timeout(wait, self.host.restart_serve())
                .await
                .is_err()
            {
                eprintln!(
                    "finite-agentd: hermes serve did not confirm its restart within {wait:?}; continuing"
                );
            }
        }
        self.verify_v1(plan)?;
        Ok(serde_json::to_value(result)?)
    }

    /// For v1: one immediate read of the parsed values
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

    /// validate synchronously with no file touched, then record the
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
                if self.fp.settings().is_none() {
                    return Err(AgentdError::Config(
                        "Finite Private isn't available on this agent.".to_owned(),
                    ));
                }
                None
            }
            IntentRoute::Openrouter => {
                let model = body.model.ok_or_else(|| {
                    AgentdError::InvalidPayload("OpenRouter model is invalid".to_owned())
                })?;
                validate_model_name(&model)?;
                // Match status priority: .env, legacy config, then process environment.
                // The executor migrates a legacy key only after validation succeeds.
                let key = self
                    .connections
                    .selectable_openrouter_key(self.host.environment_openrouter_key())?
                    .ok_or_else(|| {
                        AgentdError::NotConnected("Connect OpenRouter first.".to_owned())
                    })?;
                crate::openrouter::check_key_at(&self.openrouter_api_base, &key).await?;
                Some(model)
            }
            IntentRoute::OpenaiCodex => {
                return Err(AgentdError::InvalidPayload(
                    "This agent can't use ChatGPT yet".to_owned(),
                ));
            }
        };
        let planned = plan_model_block(body.route, model.as_deref(), &self.fp)?;
        if self.config.current_value(MODEL_CONFIG_PATH)? == planned {
            self.clear_failed_record(admission);
            return Ok(json!({ "changed": false }));
        }
        self.record_intent(IntentKind::Select, body.route, model)
    }

    /// the safe-default precondition, then a new or resumed intent. Nothing is
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
        let read = self.read_facts().await;
        // Switching the agent to Finite Private needs its settings and its
        // credential known present. No live probe. A read that failed is "try
        // again"; a key the helper answered as absent or `unknown` (an
        // external secret source it does not evaluate) is not.
        if is_saved {
            if self.fp.settings().is_none() {
                return Err(AgentdError::FinitePrivateUnavailable);
            }
            match read.as_ref().map(|facts| facts.finite_private.fp_key) {
                None => return Err(AgentdError::FactsUnavailable),
                Some(Tri::Present) => {}
                Some(_) => return Err(AgentdError::FinitePrivateUnavailable),
            }
        }
        let facts = read.unwrap_or_else(InferenceFacts::unknown);
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
        self.connections.openrouter_dotenv_key()?;
        self.facts().await;
        Err(AgentdError::UnsupportedCommand(
            "agent.openrouter.usage".to_owned(),
        ))
    }
}

fn saved_route_of(route: IntentRoute) -> SavedRoute {
    match route {
        IntentRoute::FinitePrivate => SavedRoute::FinitePrivate,
        IntentRoute::Openrouter => SavedRoute::Openrouter,
        IntentRoute::OpenaiCodex => SavedRoute::OpenaiCodex,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::Mutex;

    use finitechat_proto::{
        RuntimeCommandJsonPayloadV1, RuntimeCommandPayloadKindV1, RuntimeCommandTargetV1,
    };
    use serde_json::json;
    use tokio::sync::Notify;

    use super::*;
    use crate::daemon::{failure_result, run_config_check};
    use crate::ledger::Ledger;
    use crate::openrouter::fake::{FakeOpenRouter, key_data};
    use crate::supervisor::{ProcessSpec, start_supervisor};

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
        /// A stand-in `hermes` whose `config check` runs for real.
        config_check: Arc<Mutex<Option<PathBuf>>>,
        /// The deadline each config check was given.
        check_deadlines: Arc<Mutex<Vec<Duration>>>,
        restarts: Arc<Mutex<usize>>,
        on_restart: Arc<Mutex<RestartHook>>,
        hold_restart: Arc<Mutex<Option<Arc<Notify>>>>,
        launcher_clears_overrides: Arc<Mutex<bool>>,
        /// Set to a marker path to check that Hermes started before each step.
        started_marker: Arc<Mutex<Option<PathBuf>>>,
        /// A `hermes serve` restart that never returns.
        serve_hangs: Arc<Mutex<bool>>,
        /// agentd's `OPENROUTER_API_KEY`; the tests never read the real one.
        environment_key: Arc<Mutex<Option<String>>>,
    }

    impl TestHost {
        fn event(&self, event: &str) {
            let marker = self.started_marker.lock().unwrap().clone();
            if let Some(marker) = marker {
                // The supervisor spawned Hermes before the resume began; give
                // its first line time to run on a loaded machine.
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
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

        /// The launcher's pending-disconnect step: clears only at
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
        fn validate_config(&self, deadline: Duration) -> Result<(), AgentdError> {
            self.check_deadlines.lock().unwrap().push(deadline);
            if *self.validate_fail.lock().unwrap() {
                return Err(AgentdError::Config("Hermes rejected it".to_owned()));
            }
            let program = self.config_check.lock().unwrap().clone();
            match program {
                Some(program) => run_config_check(
                    program.as_os_str(),
                    self.config_path.parent().unwrap(),
                    deadline,
                ),
                None => Ok(()),
            }
        }

        fn dotenv_openrouter_key(&self) -> Result<Option<String>, AgentdError> {
            self.connections.openrouter_dotenv_key()
        }

        fn environment_openrouter_key(&self) -> Option<String> {
            self.environment_key.lock().unwrap().clone()
        }

        fn migrate_legacy_openrouter_key(&self) -> Result<bool, AgentdError> {
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
            if *self.serve_hangs.lock().unwrap() {
                std::future::pending::<()>().await;
            }
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
            config_check: Arc::default(),
            check_deadlines: Arc::default(),
            restarts: Arc::default(),
            on_restart: Arc::new(Mutex::new(Box::new(|_| Ok(())))),
            hold_restart: Arc::default(),
            launcher_clears_overrides: Arc::new(Mutex::new(true)),
            started_marker: Arc::default(),
            serve_hangs: Arc::default(),
            environment_key: Arc::default(),
        };
        let mut inference = Inference::new(
            host.clone(),
            connections,
            config,
            hermes_home.clone(),
            intent_path,
            fp(),
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

    // ---- every dispatch arm calls `admit` -------------------------

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

    #[tokio::test]
    async fn admission_at_the_dispatch_level() {
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

            // A failed Codex disconnect refuses a new sign-in, first.
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
        // Codex actions need codex.login.v1, which is not advertised before login is implemented.
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

    // ---- Status ---------------------------------------------------

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
    async fn status_never_contains_a_secret() {
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
    async fn a_command_reads_the_facts_with_10_s_and_the_executor_with_30_s() {
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
    async fn a_failed_facts_read_refuses_a_disconnect_as_facts_unavailable() {
        let env = format!("OPENROUTER_API_KEY={OR_KEY}\n");
        let setup = new_setup(&openrouter_block(), &env);
        *setup.host.facts_fail.lock().unwrap() = true;
        let before = (setup.config_bytes(), setup.env());
        let request = request(
            "agent.inference.disconnect",
            INFERENCE_DISCONNECT_SCHEMA,
            json!({"route": "openrouter"}),
        );
        let error = setup
            .inference
            .execute(&request)
            .await
            .unwrap()
            .unwrap_err();
        // At the wire: the result the bridge delivers.
        let result = failure_result(&request, error);
        let wire = result.error.unwrap();
        assert_eq!(wire.code, "facts_unavailable");
        assert_eq!(
            wire.message,
            "The agent couldn't check its setup right now. Try again in a moment."
        );
        // The dashboard accepts a code matching ^[a-z][a-z0-9_]{0,63}$.
        assert!(wire.code.len() <= 64);
        assert!(wire.code.starts_with(|c: char| c.is_ascii_lowercase()));
        assert!(
            wire.code
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        );
        assert_eq!((setup.config_bytes(), setup.env()), before);
        assert!(setup.record().is_none(), "no intent");

        // a retry within 15 s is answered from the remembered failure,
        // with no second helper.
        *setup.host.facts_fail.lock().unwrap() = false;
        assert_eq!(
            code(setup.disconnect("openrouter").await),
            "facts_unavailable"
        );
        assert_eq!(setup.host.facts_deadlines.lock().unwrap().len(), 1);

        // Missing settings stay "not set up", whatever the facts say.
        let mut setup = new_setup(&openrouter_block(), &env);
        setup.inference.fp = FinitePrivateEnv::default();
        *setup.host.facts_fail.lock().unwrap() = true;
        assert_eq!(
            code(setup.disconnect("openrouter").await),
            "finite_private_unavailable"
        );
    }

    #[tokio::test]
    async fn the_executor_reads_the_helper_whatever_status_remembers() {
        // OpenRouter stored but not the saved default: no safe-default precondition.
        let setup = new_setup(&fp_block(), &format!("OPENROUTER_API_KEY={OR_KEY}\n"));
        *setup.host.facts_fail.lock().unwrap() = true;
        let status = setup.status().await;
        assert_eq!(status["inference"]["fallback"]["state"], "unknown");
        *setup.host.facts_fail.lock().unwrap() = false;
        // The admission read is answered from the remembered failure; the
        // executor's reads each start a helper.
        assert_eq!(
            setup.disconnect("openrouter").await.unwrap()["accepted"],
            true
        );
        assert!(setup.settled().await.is_none(), "succeeded");
        let deadlines = setup.host.facts_deadlines.lock().unwrap().clone();
        assert_eq!(deadlines[0], crate::helper::STATUS_FACTS_DEADLINE);
        assert!(deadlines.len() >= 3, "{deadlines:?}");
        assert!(
            deadlines[1..]
                .iter()
                .all(|deadline| *deadline == crate::helper::EXECUTOR_FACTS_DEADLINE),
            "one status read, then only executor reads: {deadlines:?}"
        );
    }

    #[tokio::test]
    async fn a_hung_v1_config_check_replies_config_invalid_inside_its_budget() {
        use crate::executor::tests::{all_gone, hanging_config_check, recorded_pids};
        let mut setup = new_setup(&fp_block(), "OPENROUTER_API_KEY='sk-or-v1-synthetic-old'\n");
        let (script, pids) = hanging_config_check(&setup.hermes_home);
        *setup.host.config_check.lock().unwrap() = Some(script);
        let budget = Duration::from_secs(2);
        setup.inference.v1_reply_budget = budget;
        setup.inference.v1_min_config_check = Duration::from_millis(500);
        let before = (setup.config_bytes(), setup.env());
        let started = Instant::now();
        let reply = setup
            .v1(json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL}))
            .await;
        let took = started.elapsed();
        assert_eq!(code(reply), "config_invalid");
        // The check ended by the budget; the reply follows it well inside the
        // dashboard's wait, which is 5 s longer than the budget in production.
        assert!(took < budget + Duration::from_secs(5), "{took:?}");
        let deadlines = setup.host.check_deadlines.lock().unwrap().clone();
        assert_eq!(deadlines.len(), 1);
        assert!(
            deadlines[0] <= budget && deadlines[0] >= Duration::from_millis(500),
            "{deadlines:?}"
        );
        assert_eq!((setup.config_bytes(), setup.env()), before, "restored");
        all_gone(&recorded_pids(&pids)).await;
    }

    #[test]
    fn v1_gets_what_is_left_of_40_s_and_at_least_5_s() {
        assert_eq!(V1_REPLY_BUDGET, Duration::from_secs(40));
        assert_eq!(V1_MIN_CONFIG_CHECK, Duration::from_secs(5));
        let fresh = remaining_budget(Instant::now(), V1_REPLY_BUDGET, V1_MIN_CONFIG_CHECK);
        assert!(fresh <= V1_REPLY_BUDGET && fresh > Duration::from_secs(39));
        let late = Instant::now() - Duration::from_secs(38);
        let left = remaining_budget(late, V1_REPLY_BUDGET, V1_MIN_CONFIG_CHECK);
        assert_eq!(left, V1_MIN_CONFIG_CHECK, "2 s left is raised to 5 s");
        let spent = Instant::now() - Duration::from_secs(90);
        assert_eq!(
            remaining_budget(spent, V1_REPLY_BUDGET, V1_MIN_CONFIG_CHECK),
            V1_MIN_CONFIG_CHECK
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

    // ---- v1 unchanged ---------------------------------------------

    #[tokio::test]
    async fn v1_request_and_reply_bytes() {
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
    async fn v1_no_op_and_changed_key() {
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
    async fn v1_restores_the_env_snapshot_on_check_and_restart_failure() {
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
    async fn v1_admission_and_failed_records() {
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
    async fn a_serve_restart_that_never_returns_does_not_hold_the_v1_reply() {
        let mut setup = new_setup(&fp_block(), "");
        *setup.host.serve_hangs.lock().unwrap() = true;
        setup.inference.serve_restart_wait = Duration::from_millis(100);
        let body = json!({"profile": "openrouter", "api_key": OR_KEY, "model": OR_MODEL});
        tokio::time::timeout(Duration::from_secs(30), setup.v1(body))
            .await
            .expect("the v1 reply does not wait for hermes serve")
            .unwrap();
        assert_eq!(setup.host.events(), ["gateway", "serve"]);
        assert_eq!(setup.host.model()["default"], OR_MODEL);
    }

    // ---- select preflight -----------------------------------------

    async fn assert_select_preflight(
        name: &str,
        model: &Value,
        env: &str,
        body: Value,
        status: u16,
        reply: &str,
        expected: &str,
    ) {
        let mut setup = new_setup(model, env);
        let fake = FakeOpenRouter::start(status, reply).await;
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

    #[tokio::test]
    async fn select_preflight_matrix() {
        let ok = key_data(json!({"limit": null, "limit_remaining": null}));
        let key = "OPENROUTER_API_KEY=k1234567\n";
        let or_request = json!({"route": "openrouter", "model": OR_MODEL});
        // Payload and saved-state cases all receive the same healthy provider reply.
        let cases = [
            (
                "fp",
                fp_block(),
                "",
                json!({"route": "finite_private"}),
                "changed",
            ),
            (
                "fp from openrouter",
                openrouter_block(),
                "",
                json!({"route": "finite_private", "model": null}),
                "accepted",
            ),
            (
                "fp with a model",
                openrouter_block(),
                "",
                json!({"route": "finite_private", "model": "x"}),
                "invalid_payload",
            ),
            (
                "unknown field",
                fp_block(),
                "",
                json!({"route": "finite_private", "extra": 1}),
                "invalid_payload",
            ),
            (
                "no key",
                fp_block(),
                "",
                or_request.clone(),
                "not_connected",
            ),
            (
                "bad model",
                fp_block(),
                key,
                json!({"route": "openrouter", "model": "has space"}),
                "invalid_payload",
            ),
            (
                "missing model",
                fp_block(),
                key,
                json!({"route": "openrouter"}),
                "invalid_payload",
            ),
            (
                "already saved",
                openrouter_block(),
                key,
                or_request.clone(),
                "changed",
            ),
        ];
        for (name, model, env, body, expected) in cases {
            assert_select_preflight(name, &model, env, body, 200, &ok, expected).await;
        }
        // Provider errors must not write config, credentials, or an intent.
        let exhausted = key_data(json!({"limit": 5, "limit_remaining": 0}));
        for (name, status, reply, expected) in [
            ("key ok", 200, ok.clone(), "accepted"),
            ("key rejected", 401, ok.clone(), "credential_rejected"),
            ("provider down", 500, ok.clone(), "provider_unavailable"),
            (
                "management key",
                200,
                key_data(json!({"is_management_key": true})),
                "credential_rejected",
            ),
            (
                "exhausted",
                200,
                exhausted.clone(),
                "key_allowance_exhausted",
            ),
        ] {
            assert_select_preflight(
                name,
                &fp_block(),
                key,
                or_request.clone(),
                status,
                &reply,
                expected,
            )
            .await;
        }
        let legacy = json!({"default": OR_MODEL, "provider": "openrouter", "api_key": "sk-or-v1-synthetic-legacy"});
        for (name, status, reply, expected) in [
            ("legacy key rejected", 401, ok, "credential_rejected"),
            (
                "legacy key exhausted",
                200,
                exhausted,
                "key_allowance_exhausted",
            ),
        ] {
            assert_select_preflight(
                name,
                &legacy,
                "",
                or_request.clone(),
                status,
                &reply,
                expected,
            )
            .await;
        }
    }

    #[tokio::test]
    async fn an_environment_key_that_status_shows_can_be_selected() {
        // Review B's probe: no `.env` key and no legacy key, only agentd's
        // environment key, which status shows as `environment`.
        let environment = "sk-or-v1-synthetic-environment";
        let mut setup = new_setup(&fp_block(), "KEEP=1\n");
        *setup.host.environment_key.lock().unwrap() = Some(environment.to_owned());
        {
            let mut facts = setup.host.facts.lock().unwrap();
            facts.openrouter.hermes_key = Tri::Present;
            facts.openrouter.hermes_key_fingerprint =
                Some(crate::ledger::hex_digest(environment.as_bytes()));
        }
        let status = setup.status().await;
        assert_eq!(
            status["inference"]["routes"]["openrouter"]["key_source"],
            "environment"
        );
        let fake = FakeOpenRouter::start(200, &key_data(json!({"limit": null}))).await;
        setup.inference.openrouter_api_base = fake.base.clone();
        let reply = setup
            .select(json!({"route": "openrouter", "model": OR_MODEL}))
            .await
            .unwrap();
        assert_eq!(reply["accepted"], true);
        assert_eq!(fake.requests(), ["GET /api/v1/key HTTP/1.1"]);
        assert!(setup.settled().await.is_none(), "verified and succeeded");
        assert_eq!(setup.host.model()["provider"], "openrouter");
        assert_eq!(setup.env(), "KEEP=1\n", "never copied into .env");

        // Without it, nothing is selectable.
        let setup = new_setup(&fp_block(), "");
        assert_eq!(
            code(
                setup
                    .select(json!({"route": "openrouter", "model": OR_MODEL}))
                    .await
            ),
            "not_connected"
        );
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

    // ---- async replies and the operation lifecycle ------------------

    #[tokio::test]
    async fn select_replies_accepted_and_status_follows_the_operation() {
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

    // ---- the safe-default precondition -------------------------------

    #[tokio::test]
    async fn disconnecting_the_saved_route_needs_finite_private_known_present() {
        let env = format!("OPENROUTER_API_KEY={OR_KEY}\n");
        // an answered absent or `unknown` key is "not set up"; only a
        // failed read is "could not check" (the failed-read test below).
        for (fp_key, code, message) in [
            (
                Tri::Absent,
                "finite_private_unavailable",
                "Disconnecting would leave this agent without a model: Finite Private isn't fully set up here. Choose another model first.",
            ),
            (
                // the helper answered and could not evaluate the key.
                Tri::Unknown,
                "finite_private_unavailable",
                "Disconnecting would leave this agent without a model: Finite Private isn't fully set up here. Choose another model first.",
            ),
        ] {
            let setup = new_setup(&openrouter_block(), &env);
            setup.host.facts.lock().unwrap().finite_private.fp_key = fp_key;
            let before = (setup.config_bytes(), setup.env());
            let error = setup.disconnect("openrouter").await.unwrap_err();
            assert_eq!(error.public_code(), code, "{fp_key:?}");
            assert_eq!(error.public_message(), message, "{fp_key:?}");
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
    async fn disconnect_with_finite_private_settings_missing_is_refused() {
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

    // ---- real failure paths ---------------------------------------

    #[tokio::test]
    async fn real_read_only_directories() {
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
        fn validate_config(&self, _deadline: Duration) -> Result<(), AgentdError> {
            Ok(())
        }
        fn dotenv_openrouter_key(&self) -> Result<Option<String>, AgentdError> {
            self.inner.dotenv_openrouter_key()
        }
        fn environment_openrouter_key(&self) -> Option<String> {
            self.inner.environment_openrouter_key()
        }
        fn migrate_legacy_openrouter_key(&self) -> Result<bool, AgentdError> {
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

    /// A child that sees the system directories only, never the host's PATH.
    fn sleeper_spec(name: &'static str, program: &Path) -> ProcessSpec {
        ProcessSpec {
            name,
            program: program.to_owned(),
            args: Vec::new(),
            environment: BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
        }
    }

    async fn wait_for_hermes(supervisor: &SupervisorHandle) {
        tokio::time::timeout(Duration::from_secs(30), async {
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
    async fn a_real_spawn_failure_rolls_v1_back() {
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

    /// Polls `condition` for up to 30 s; fails the test if it never holds.
    async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(30), async {
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

    // ---- partial-state kill points --------------------------

    /// Restart agentd over the state a crash left: Hermes starts first, the
    /// saved route is configured at that moment, and the operation converges
    /// or ends failed with a code.
    async fn restart_over(setup: &Setup) -> Option<IntentRecord> {
        assert!(route_configured(setup), "configured when Hermes starts");
        // The launcher step on the start after the crash.
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
    async fn select_resumes_after_each_persisted_kill_point() {
        let legacy = json!({"default": OR_MODEL, "provider": "openrouter", "api_key": OR_KEY});
        for (name, model, phase) in [
            ("intent", fp_block(), intent::IntentPhase::Accepted),
            ("credential", legacy, intent::IntentPhase::ConfigWritten),
            ("model", openrouter_block(), intent::IntentPhase::Restarting),
        ] {
            let setup = new_setup(&model, &format!("OPENROUTER_API_KEY={OR_KEY}\n"));
            crashed_at(&setup, IntentKind::Select, IntentRoute::Openrouter, phase);
            assert!(restart_over(&setup).await.is_none(), "{name}");
            assert_eq!(setup.host.model(), openrouter_block(), "{name}");
            assert_eq!(
                setup
                    .inference
                    .connections
                    .openrouter_dotenv_key()
                    .unwrap()
                    .as_deref(),
                Some(OR_KEY),
                "{name}"
            );
            if phase == intent::IntentPhase::Restarting {
                assert_eq!(
                    setup.host.events().first().map(String::as_str),
                    Some("gateway")
                );
            }
        }
    }

    #[tokio::test]
    async fn select_spawn_failure_after_the_write_restores_the_previous_route() {
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
    async fn disconnect_crashed_at_accepted_or_login_cancelled_skips_the_clears() {
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
    async fn disconnect_crashed_after_each_later_step() {
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
    async fn partial_launcher_clears_end_verify_failed() {
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
}
