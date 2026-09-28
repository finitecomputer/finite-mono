//! The background executor for the inference intent record (§3.11).
//!
//! It never creates a record. It advances the existing one through its kind's
//! phases, writing each phase before that phase's step, and deletes it on
//! success. Every step is idempotent, so a resume re-runs the recorded phase's
//! step and everything after it. Nothing here runs before Hermes starts, and a
//! failure leaves the record `failed` for status and the launcher; it never
//! stops chat from starting.
//!
//! Mutations are forward-only. The one rollback is the existing spawn-failure
//! config restore, under the byte guard, before any chat turn could run.

use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::AgentdError;
use crate::config::{ConfigManager, MODEL_CONFIG_PATH, ModelWrite, WrittenConfig};
use crate::facts::{CodexStateFact, InferenceFacts, PoolEntries, Tri};
use crate::helper::EXECUTOR_FACTS_DEADLINE;
use crate::inference::{
    FinitePrivateEnv, OperationState, OperationStatus, SavedRoute, classify_saved_route,
    plan_model_block,
};
use crate::intent::{self, IntentKind, IntentPhase, IntentRecord, IntentRoute, IntentState};

/// Delay before each attempt; three attempts per agentd process.
const BACKOFF: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_secs(15),
    Duration::from_secs(60),
];
/// Verification reads again this long after the restart reached `Running`.
const VERIFY_DELAY: Duration = Duration::from_secs(5);
/// How many times a mismatch found by verification is re-applied.
const MAX_REAPPLIES: usize = 2;
/// R14: after a disconnect's cleanup restart, how long verification waits for
/// the launcher's pending-disconnect step before calling it a mismatch. It
/// polls every `VERIFY_DELAY` within this window and restarts nothing; a read
/// that fails in it is only "not yet" (R15b).
const LAUNCHER_WAIT: Duration = Duration::from_secs(60);
/// R18: how long `hermes config check` may take before it is killed and the
/// attempt is `config_invalid`.
pub(crate) const CONFIG_CHECK_DEADLINE: Duration = Duration::from_secs(60);
/// How long status shows a succeeded operation after its record is deleted.
const RESULT_TTL: Duration = Duration::from_secs(10 * 60);

/// What the executor needs from the rest of agentd. `.env` handling belongs to
/// `connections.rs`; restarts to the supervisor and `hosted_hermes.rs`.
pub(crate) trait ExecutorHost: Clone + Send + Sync + 'static {
    /// `hermes config check` after a config write, killed with its process
    /// group at `deadline` (R18). Blocking: callers run it off the async
    /// workers.
    fn validate_config(&self, deadline: Duration) -> Result<(), AgentdError>;
    /// The value of the last `OPENROUTER_API_KEY` line in `.env`, if any.
    fn dotenv_openrouter_key(&self) -> Result<Option<String>, AgentdError>;
    /// §3.6 background step 1: copy a legacy `model.api_key` into `.env` when
    /// `.env` has no key.
    fn migrate_legacy_openrouter_key(&self) -> Result<(), AgentdError>;
    /// §3.7 step 3: remove every `OPENROUTER_API_KEY` line from `.env`.
    fn remove_openrouter_key(&self) -> Result<(), AgentdError>;
    /// Restart the gateway; `Ok` once it is `Running` again.
    fn restart_gateway(&self) -> impl Future<Output = Result<(), AgentdError>> + Send;
    /// Stop the agentd-launched `hermes serve`. It starts again when its gate
    /// allows: at once, or after the disconnect record is deleted.
    fn restart_serve(&self) -> impl Future<Output = ()> + Send;
    /// Fresh helper facts, bypassing the status cache, within `deadline`.
    fn facts(
        &self,
        deadline: Duration,
    ) -> impl Future<Output = Result<InferenceFacts, AgentdError>> + Send;
    /// `codex::cancel_for_disconnect`.
    fn cancel_codex_login(&self) -> impl Future<Output = Result<(), AgentdError>> + Send;
}

/// Why an attempt stopped. `code` is the §3.2 background code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Failure {
    code: &'static str,
    /// Retrying would repeat work the step already retried, or undo a restore.
    terminal: bool,
}

impl Failure {
    const CONFIG_CONFLICT: Self = Self {
        code: "config_conflict",
        terminal: true,
    };
    const VERIFY_FAILED: Self = Self {
        code: "verify_failed",
        terminal: true,
    };
    const SUPERVISOR_UNAVAILABLE: Self = Self {
        code: "supervisor_unavailable",
        terminal: true,
    };
    const CONFIG_INVALID: Self = Self {
        code: "config_invalid",
        terminal: false,
    };
}

impl From<AgentdError> for Failure {
    fn from(error: AgentdError) -> Self {
        match error {
            AgentdError::ConfigConflict(_) => Self::CONFIG_CONFLICT,
            AgentdError::Supervisor(_) => Self::SUPERVISOR_UNAVAILABLE,
            AgentdError::ProviderUnavailable(_) => Self {
                code: "helper_unavailable",
                terminal: false,
            },
            _ => Self::CONFIG_INVALID,
        }
    }
}

pub(crate) struct Executor<H> {
    host: H,
    config: ConfigManager,
    intent_path: PathBuf,
    fp: FinitePrivateEnv,
    backoff: [Duration; 3],
    verify_delay: Duration,
    launcher_wait: Duration,
    config_check_deadline: Duration,
    last: std::sync::Mutex<Option<(OperationStatus, Instant)>>,
    running: tokio::sync::Mutex<()>,
}

impl<H: ExecutorHost> Executor<H> {
    pub(crate) fn new(
        host: H,
        config: ConfigManager,
        intent_path: PathBuf,
        fp: FinitePrivateEnv,
    ) -> Self {
        Self {
            host,
            config,
            intent_path,
            fp,
            backoff: BACKOFF,
            verify_delay: VERIFY_DELAY,
            launcher_wait: LAUNCHER_WAIT,
            config_check_deadline: CONFIG_CHECK_DEADLINE,
            last: std::sync::Mutex::new(None),
            running: tokio::sync::Mutex::new(()),
        }
    }

    /// Shorter waits for tests elsewhere in the crate.
    #[cfg(test)]
    pub(crate) fn set_timing(&mut self, verify_delay: Duration, launcher_wait: Duration) {
        self.verify_delay = verify_delay;
        self.launcher_wait = launcher_wait;
    }

    /// At startup, after Hermes has started: re-arm a failed disconnect with
    /// a fresh budget, then run a running record. A failed select or activate
    /// stays exactly as it is (R12): the user may have chosen another model
    /// since, and only their next change replaces it.
    pub(crate) async fn resume_at_startup(&self) {
        {
            let _running = self.running.lock().await;
            match intent::load(&self.intent_path) {
                Ok(Some(mut record))
                    if record.state == IntentState::Failed
                        && record.kind == IntentKind::Disconnect =>
                {
                    record.state = IntentState::Running;
                    record.error_code = None;
                    record.attempts = 0;
                    record.updated_at_ms = now_ms();
                    if let Err(error) = intent::store(&self.intent_path, &record) {
                        log_failure(&record, &format!("could not re-arm ({error})"));
                        return;
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("finite-agentd: could not read the inference intent: {error}");
                    return;
                }
            }
        }
        self.run().await;
    }

    /// Runs a `running` record to success or `failed`. A handler calls this
    /// (in a spawned task) after writing a record.
    pub(crate) async fn run(&self) {
        let _running = self.running.lock().await;
        let mut record = match intent::load(&self.intent_path) {
            Ok(Some(record)) if record.state == IntentState::Running => record,
            Ok(_) => return,
            Err(error) => {
                eprintln!("finite-agentd: could not read the inference intent: {error}");
                return;
            }
        };
        for (attempt, delay) in self.backoff.iter().enumerate() {
            tokio::time::sleep(*delay).await;
            record.attempts = attempt as u32 + 1;
            let result = match self.save(&mut record) {
                Ok(()) => self.drive(&mut record).await,
                Err(failure) => Err(failure),
            };
            match result {
                Ok(()) => {
                    self.succeed(&record).await;
                    return;
                }
                Err(failure) if failure.terminal || attempt + 1 == self.backoff.len() => {
                    record.state = IntentState::Failed;
                    record.error_code = Some(failure.code.to_owned());
                    log_failure(&record, failure.code);
                    if let Err(error) = self.save(&mut record) {
                        log_failure(&record, &format!("could not record failure ({error:?})"));
                    }
                    return;
                }
                Err(failure) => log_failure(&record, failure.code),
            }
        }
    }

    /// `inference.operation` in status: the record, or the last success for
    /// ten minutes after its record was deleted.
    pub(crate) fn operation(&self, record: Option<&IntentRecord>) -> Option<OperationStatus> {
        if let Some(record) = record {
            return Some(OperationStatus::from_record(record));
        }
        let last = self.last.lock().unwrap_or_else(|error| error.into_inner());
        last.as_ref()
            .filter(|(_, at)| at.elapsed() < RESULT_TTL)
            .map(|(status, _)| status.clone())
    }

    async fn succeed(&self, record: &IntentRecord) {
        if let Err(error) = intent::clear(&self.intent_path) {
            log_failure(record, &format!("could not delete the record ({error})"));
            return;
        }
        let mut status = OperationStatus::from_record(record);
        status.state = OperationState::Succeeded;
        status.error_code = None;
        status.updated_at_ms = now_ms();
        *self.last.lock().unwrap_or_else(|error| error.into_inner()) =
            Some((status, Instant::now()));
        if record.kind == IntentKind::Disconnect {
            // Verified and deleted: `hermes serve` may start again, after the
            // restart that ran the launcher's clears.
            self.host.restart_serve().await;
        }
    }

    async fn drive(&self, record: &mut IntentRecord) -> Result<(), Failure> {
        match record.kind {
            IntentKind::Select | IntentKind::Activate => self.drive_select(record).await,
            IntentKind::Disconnect => self.drive_disconnect(record).await,
        }
    }

    /// `accepted → config_written → restarting → verifying`.
    async fn drive_select(&self, record: &mut IntentRecord) -> Result<(), Failure> {
        let planned = plan_model_block(record.route, record.model.as_deref(), &self.fp)?;
        let mut written = None;
        for phase in self.remaining(record) {
            self.enter(record, phase)?;
            match phase {
                IntentPhase::ConfigWritten => {
                    written = self.write_select(record, &planned).await?;
                }
                IntentPhase::Restarting => {
                    if self.host.restart_gateway().await.is_err() {
                        return Err(self.restore_after_spawn_failure(written.as_ref()).await);
                    }
                    if record.kind == IntentKind::Activate {
                        // A replaced credential: flush any copy `hermes serve` holds.
                        self.host.restart_serve().await;
                    }
                }
                IntentPhase::Verifying => {
                    let mut reapplies = 0;
                    while !self.select_verified(record, &planned).await? {
                        if reapplies == MAX_REAPPLIES {
                            return Err(Failure::CONFIG_CONFLICT);
                        }
                        reapplies += 1;
                        self.write_select(record, &planned).await?;
                        self.host.restart_gateway().await?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn write_select(
        &self,
        record: &IntentRecord,
        planned: &Value,
    ) -> Result<Option<WrittenConfig>, Failure> {
        if record.route == IntentRoute::Openrouter {
            self.host.migrate_legacy_openrouter_key()?;
        }
        match self.write_model(planned).await? {
            ModelWrite::Written(written) => Ok(Some(written)),
            ModelWrite::Unchanged => Ok(None),
        }
    }

    /// Writes the `model` block, validated by `hermes config check` on a
    /// blocking thread and within `config_check_deadline` (R18). A check that
    /// fails or times out restores the previous bytes and is `config_invalid`.
    async fn write_model(&self, planned: &Value) -> Result<ModelWrite, Failure> {
        let config = self.config.clone();
        let host = self.host.clone();
        let planned = planned.clone();
        let deadline = self.config_check_deadline;
        tokio::task::spawn_blocking(move || {
            config.write_model(&planned, || host.validate_config(deadline))
        })
        .await
        .map_err(|_| Failure::CONFIG_INVALID)?
        .map_err(Failure::from)
    }

    /// §3.10: after a spawn failure, restore the bytes this run wrote (only if
    /// they are still intact) and bring the previous route back up.
    async fn restore_after_spawn_failure(&self, written: Option<&WrittenConfig>) -> Failure {
        let Some(written) = written else {
            return Failure::SUPERVISOR_UNAVAILABLE;
        };
        if self.config.restore_if_unchanged(written).is_err() {
            return Failure::CONFIG_CONFLICT;
        }
        let _ = self.host.restart_gateway().await;
        Failure::SUPERVISOR_UNAVAILABLE
    }

    /// Values, never bytes (G1): the parsed `model` mapping equals the planned
    /// block, and an OpenRouter route has a stored key. Checked after
    /// `Running` and again after `VERIFY_DELAY`.
    async fn select_verified(
        &self,
        record: &IntentRecord,
        planned: &Value,
    ) -> Result<bool, Failure> {
        for check in 0..2 {
            if check > 0 {
                tokio::time::sleep(self.verify_delay).await;
            }
            let model_matches = self.config.current_value(MODEL_CONFIG_PATH)? == *planned;
            let key_matches = record.route != IntentRoute::Openrouter
                || stored_key(self.host.dotenv_openrouter_key()?.as_deref());
            if !(model_matches && key_matches) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `accepted → login_cancelled → route_switched → credential_removed →
    /// cleanup → verifying`.
    async fn drive_disconnect(&self, record: &mut IntentRecord) -> Result<(), Failure> {
        let route = record.route;
        for phase in self.remaining(record) {
            self.enter(record, phase)?;
            match phase {
                IntentPhase::LoginCancelled => {
                    if route == IntentRoute::OpenaiCodex {
                        self.host.cancel_codex_login().await?;
                    }
                }
                IntentPhase::RouteSwitched => self.switch_route_away(route).await?,
                IntentPhase::CredentialRemoved => self.remove_credential(route)?,
                IntentPhase::Cleanup => self.cleanup_restart().await?,
                IntentPhase::Verifying => {
                    let mut reruns = 0;
                    while !self.disconnect_verified(route).await? {
                        if reruns == MAX_REAPPLIES {
                            return Err(Failure::VERIFY_FAILED);
                        }
                        reruns += 1;
                        // Steps 2–5 again; the phase stays `verifying`, so the
                        // launcher still applies its clears on this restart.
                        self.switch_route_away(route).await?;
                        self.remove_credential(route)?;
                        self.cleanup_restart().await?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// §3.7 step 2: if the saved default is the route being removed, write the
    /// Finite Private block and confirm the switch by re-reading.
    async fn switch_route_away(&self, route: IntentRoute) -> Result<(), Failure> {
        if self.saved_route()? != saved_route_of(route) {
            return Ok(());
        }
        let planned = plan_model_block(IntentRoute::FinitePrivate, None, &self.fp)?;
        self.write_model(&planned).await?;
        if self.saved_route()? == saved_route_of(route) {
            return Err(Failure::CONFIG_CONFLICT);
        }
        Ok(())
    }

    fn remove_credential(&self, route: IntentRoute) -> Result<(), Failure> {
        if route != IntentRoute::Openrouter {
            return Ok(());
        }
        self.host.remove_openrouter_key()?;
        if self.host.dotenv_openrouter_key()?.is_some() {
            return Err(Failure::CONFIG_INVALID);
        }
        Ok(())
    }

    /// §3.7 step 4: stop `hermes serve` (it stays stopped while the record
    /// exists), then restart the gateway, whose launcher applies the clears.
    async fn cleanup_restart(&self) -> Result<(), Failure> {
        self.host.restart_serve().await;
        self.host.restart_gateway().await?;
        Ok(())
    }

    /// §3.7 step 5 as ruled in R14 and R15b. The launcher's clears land some
    /// seconds after the restart, so the facts are read every `verify_delay`
    /// until `launcher_wait` has passed. Verified at the first read that shows
    /// everything cleared and is confirmed by the next one. During the wait an
    /// `unknown` fact or a failed read is only "not yet". Once the window has
    /// passed, the last read decides: not cleared is the mismatch, and a
    /// failed read is `helper_unavailable`. A cleared read inside the window
    /// still gets its confirmation. No restart happens inside it.
    async fn disconnect_verified(&self, route: IntentRoute) -> Result<bool, Failure> {
        let deadline = tokio::time::Instant::now() + self.launcher_wait;
        let mut cleared_before = false;
        loop {
            let read = self.host.facts(EXECUTOR_FACTS_DEADLINE).await;
            let cleared = match &read {
                Ok(facts) => self.disconnect_cleared(route, facts)?,
                Err(_) => false,
            };
            if cleared && cleared_before {
                return Ok(true);
            }
            if !cleared && tokio::time::Instant::now() >= deadline {
                read?;
                return Ok(false);
            }
            cleared_before = cleared;
            tokio::time::sleep(self.verify_delay).await;
        }
    }

    /// Whether one read shows the route removed. Any `unknown` counts as not
    /// cleared.
    fn disconnect_cleared(
        &self,
        route: IntentRoute,
        facts: &InferenceFacts,
    ) -> Result<bool, Failure> {
        let removed = match route {
            IntentRoute::Openrouter => {
                facts.openrouter.dotenv_key == Tri::Absent
                    && facts.openrouter.manual_pool_entries == PoolEntries::None
                    && facts.session_overrides.openrouter == Tri::Absent
            }
            IntentRoute::OpenaiCodex => {
                facts.codex.state == CodexStateFact::NotSignedIn
                    && facts.session_overrides.openai_codex == Tri::Absent
            }
            IntentRoute::FinitePrivate => false,
        };
        Ok(removed && self.saved_route()? != saved_route_of(route))
    }

    fn saved_route(&self) -> Result<SavedRoute, Failure> {
        let model = self.config.current_value(MODEL_CONFIG_PATH)?;
        Ok(classify_saved_route(&model, self.fp.base_url.as_deref()))
    }

    /// The record's phase and every later phase of its kind.
    fn remaining(&self, record: &IntentRecord) -> Vec<IntentPhase> {
        let phases = record.kind.phases();
        let start = phases
            .iter()
            .position(|phase| *phase == record.phase)
            .unwrap_or_default();
        phases[start..].to_vec()
    }

    /// Writes `phase` to the record before its step runs.
    fn enter(&self, record: &mut IntentRecord, phase: IntentPhase) -> Result<(), Failure> {
        if record.phase != phase {
            record.phase = phase;
            self.save(record)?;
        }
        Ok(())
    }

    fn save(&self, record: &mut IntentRecord) -> Result<(), Failure> {
        record.updated_at_ms = now_ms();
        intent::store(&self.intent_path, record).map_err(|_| Failure::CONFIG_INVALID)
    }
}

fn saved_route_of(route: IntentRoute) -> SavedRoute {
    match route {
        IntentRoute::FinitePrivate => SavedRoute::FinitePrivate,
        IntentRoute::Openrouter => SavedRoute::Openrouter,
        IntentRoute::OpenaiCodex => SavedRoute::OpenaiCodex,
    }
}

fn stored_key(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.is_empty() && !value.starts_with("${"))
}

fn log_failure(record: &IntentRecord, detail: &str) {
    eprintln!(
        "finite-agentd: inference operation {} ({:?} {:?}, phase {:?}, attempt {}): {detail}",
        record.id, record.kind, record.route, record.phase, record.attempts
    );
}

fn now_ms() -> u64 {
    crate::supervisor::now_ms()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::ledger::Ledger;

    const FP_URL: &str = "https://fp.example.invalid/v1";
    const OPENROUTER_MODEL: &str = "anthropic/claude-sonnet-4.6";

    #[derive(Debug, Clone, PartialEq)]
    enum Event {
        Validate(Option<IntentPhase>),
        Migrate(Option<IntentPhase>),
        RemoveKey(Option<IntentPhase>),
        RestartGateway(Option<IntentPhase>),
        /// Whether a disconnect record existed, i.e. whether the gate kept it stopped.
        RestartServe {
            phase: Option<IntentPhase>,
            gated: bool,
        },
        Facts(Option<IntentPhase>),
        CancelLogin(Option<IntentPhase>),
    }

    type RestartHook = Box<dyn FnMut(usize, &Path) -> Result<(), AgentdError> + Send>;
    type ReadHook = Box<dyn FnOnce(&Path) + Send>;

    /// Hermes-side state the launcher's pending-disconnect step clears.
    #[derive(Debug, Clone, Copy)]
    struct HermesSide {
        pool: PoolEntries,
        override_openrouter: Tri,
        override_codex: Tri,
        codex: CodexStateFact,
        /// When false, the fake launcher leaves the overrides behind.
        launcher_clears_overrides: bool,
    }

    struct Fake {
        home: PathBuf,
        intent_path: PathBuf,
        events: Mutex<Vec<Event>>,
        restarts: Mutex<usize>,
        on_restart: Mutex<RestartHook>,
        /// Runs once, at the first `.env` key read during `verifying`: between
        /// the first check's model read and the second check.
        on_verify_key_read: Mutex<Option<ReadHook>>,
        /// How many facts reads fail first: `usize::MAX` fails every one.
        failed_reads: Mutex<usize>,
        /// Every facts read: the attempt it belonged to, whether it failed,
        /// and when it started.
        reads: Mutex<Vec<(u32, bool, Instant)>>,
        hermes: Mutex<HermesSide>,
        /// How many facts polls after a restart the launcher's clears take to
        /// land: 0 lands them with the restart, `usize::MAX` never does.
        launcher_polls: Mutex<usize>,
        /// A started launcher step whose clears have not landed yet:
        /// (route, polls left).
        pending_clear: Mutex<Option<(IntentRoute, usize)>>,
        /// How many facts polls report the override as `unknown` first.
        unknown_polls: Mutex<usize>,
        /// How many launcher steps hit their 20 s limit after clearing the
        /// pool entry and before clearing the override.
        cut_short_steps: Mutex<usize>,
        restart_times: Mutex<Vec<Instant>>,
        /// A stand-in `hermes` whose `config check` the fake runs for real.
        config_check: Mutex<Option<PathBuf>>,
        /// The deadline each config check was given.
        check_deadlines: Mutex<Vec<Duration>>,
    }

    impl Fake {
        fn apply_clears(&self, route: IntentRoute, overrides: bool) {
            let mut hermes = self.hermes.lock().unwrap();
            let overrides = overrides && hermes.launcher_clears_overrides;
            match route {
                IntentRoute::Openrouter => {
                    hermes.pool = PoolEntries::None;
                    if overrides {
                        hermes.override_openrouter = Tri::Absent;
                    }
                }
                IntentRoute::OpenaiCodex => {
                    hermes.codex = CodexStateFact::NotSignedIn;
                    if overrides {
                        hermes.override_codex = Tri::Absent;
                    }
                }
                IntentRoute::FinitePrivate => {}
            }
        }

        fn config_path(&self) -> PathBuf {
            self.home.join("config.yaml")
        }

        fn env_path(&self) -> PathBuf {
            self.home.join(".env")
        }

        fn phase(&self) -> Option<IntentPhase> {
            intent::load(&self.intent_path)
                .unwrap()
                .map(|record| record.phase)
        }

        fn record(&self, event: Event) {
            self.events.lock().unwrap().push(event);
        }

        fn events(&self) -> Vec<Event> {
            self.events.lock().unwrap().clone()
        }

        fn model(&self) -> Value {
            let text = fs::read_to_string(self.config_path()).unwrap();
            serde_yaml::from_str::<Value>(&text).unwrap()["model"].clone()
        }

        fn env_lines(&self) -> Vec<String> {
            fs::read_to_string(self.env_path())
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    impl ExecutorHost for Arc<Fake> {
        fn validate_config(&self, deadline: Duration) -> Result<(), AgentdError> {
            self.record(Event::Validate(self.phase()));
            self.check_deadlines.lock().unwrap().push(deadline);
            let program = self.config_check.lock().unwrap().clone();
            match program {
                Some(program) => {
                    crate::daemon::run_config_check(program.as_os_str(), &self.home, deadline)
                }
                None => Ok(()),
            }
        }

        fn dotenv_openrouter_key(&self) -> Result<Option<String>, AgentdError> {
            if self.phase() == Some(IntentPhase::Verifying) {
                let hook = self.on_verify_key_read.lock().unwrap().take();
                if let Some(hook) = hook {
                    hook(&self.config_path());
                }
            }
            Ok(self
                .env_lines()
                .iter()
                .rev()
                .find_map(|line| line.strip_prefix("OPENROUTER_API_KEY="))
                .map(str::to_owned))
        }

        fn migrate_legacy_openrouter_key(&self) -> Result<(), AgentdError> {
            self.record(Event::Migrate(self.phase()));
            let legacy = self.model()["api_key"].as_str().map(str::to_owned);
            if self.dotenv_openrouter_key()?.is_none()
                && let Some(key) = legacy
            {
                let mut lines = self.env_lines();
                lines.push(format!("OPENROUTER_API_KEY={key}"));
                fs::write(self.env_path(), lines.join("\n") + "\n")?;
            }
            Ok(())
        }

        fn remove_openrouter_key(&self) -> Result<(), AgentdError> {
            self.record(Event::RemoveKey(self.phase()));
            let lines = self
                .env_lines()
                .into_iter()
                .filter(|line| !line.starts_with("OPENROUTER_API_KEY="))
                .collect::<Vec<_>>();
            fs::write(self.env_path(), lines.join("\n") + "\n")?;
            Ok(())
        }

        async fn restart_gateway(&self) -> Result<(), AgentdError> {
            let phase = self.phase();
            self.record(Event::RestartGateway(phase));
            self.restart_times.lock().unwrap().push(Instant::now());
            // The launcher's pending-disconnect step (F1).
            if let Ok(Some(record)) = intent::load(&self.intent_path)
                && record.kind == IntentKind::Disconnect
                && matches!(record.phase, IntentPhase::Cleanup | IntentPhase::Verifying)
            {
                let saved = classify_saved_route(&self.model(), Some(FP_URL));
                if saved != saved_route_of(record.route) {
                    let polls = *self.launcher_polls.lock().unwrap();
                    if polls == 0 {
                        let cut_short = {
                            let mut steps = self.cut_short_steps.lock().unwrap();
                            let cut_short = *steps > 0;
                            *steps = steps.saturating_sub(1);
                            cut_short
                        };
                        self.apply_clears(record.route, !cut_short);
                    } else {
                        // A restart while a step is still running stops it.
                        *self.pending_clear.lock().unwrap() = Some((record.route, polls));
                    }
                }
            }
            let count = {
                let mut restarts = self.restarts.lock().unwrap();
                *restarts += 1;
                *restarts
            };
            (self.on_restart.lock().unwrap())(count, &self.config_path())
        }

        async fn restart_serve(&self) {
            let gated = crate::hosted_hermes::ServeGate::new(self.intent_path.clone()).blocked();
            self.record(Event::RestartServe {
                phase: self.phase(),
                gated,
            });
        }

        async fn facts(&self, deadline: Duration) -> Result<InferenceFacts, AgentdError> {
            let at = Instant::now();
            // R15a: every executor read gets the executor's deadline.
            assert_eq!(deadline, EXECUTOR_FACTS_DEADLINE);
            self.record(Event::Facts(self.phase()));
            // The slow launcher step lands its clears after its polls, whether
            // or not the read that marks the time succeeds.
            let landed = {
                let mut pending = self.pending_clear.lock().unwrap();
                match pending.as_mut() {
                    Some((route, left)) if *left <= 1 => {
                        let route = *route;
                        *pending = None;
                        Some(route)
                    }
                    Some((_, left)) => {
                        *left = left.saturating_sub(1);
                        None
                    }
                    None => None,
                }
            };
            if let Some(route) = landed {
                self.apply_clears(route, true);
            }
            let failed = {
                let mut left = self.failed_reads.lock().unwrap();
                let failed = *left > 0;
                *left = left.saturating_sub(1);
                failed
            };
            let attempt = intent::load(&self.intent_path)
                .unwrap()
                .map_or(0, |record| record.attempts);
            self.reads.lock().unwrap().push((attempt, failed, at));
            if failed {
                return Err(AgentdError::ProviderUnavailable("helper".to_owned()));
            }
            let hermes = *self.hermes.lock().unwrap();
            let mut facts = InferenceFacts::unknown();
            facts.openrouter.dotenv_key = if self.dotenv_openrouter_key()?.is_some() {
                Tri::Present
            } else {
                Tri::Absent
            };
            facts.openrouter.manual_pool_entries = hermes.pool;
            facts.session_overrides.openrouter = hermes.override_openrouter;
            facts.session_overrides.openai_codex = hermes.override_codex;
            facts.codex.state = hermes.codex;
            let mut unknown = self.unknown_polls.lock().unwrap();
            if *unknown > 0 {
                *unknown = unknown.saturating_sub(1);
                facts.session_overrides.openrouter = Tri::Unknown;
                facts.session_overrides.openai_codex = Tri::Unknown;
            }
            Ok(facts)
        }

        async fn cancel_codex_login(&self) -> Result<(), AgentdError> {
            self.record(Event::CancelLogin(self.phase()));
            Ok(())
        }
    }

    struct Setup {
        _temp: tempfile::TempDir,
        fake: Arc<Fake>,
        executor: Executor<Arc<Fake>>,
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
        plan_model_block(IntentRoute::Openrouter, Some(OPENROUTER_MODEL), &fp()).unwrap()
    }

    fn new_setup(model: &Value, env: &str) -> Setup {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("hermes-home");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("config.yaml"),
            serde_yaml::to_string(&json!({"model": model, "toolsets": ["hermes-cli"]})).unwrap(),
        )
        .unwrap();
        fs::write(home.join(".env"), env).unwrap();
        let intent_path = intent::intent_path(temp.path());
        let fake = Arc::new(Fake {
            home: home.clone(),
            intent_path: intent_path.clone(),
            events: Mutex::new(Vec::new()),
            restarts: Mutex::new(0),
            on_restart: Mutex::new(Box::new(|_, _| Ok(()))),
            on_verify_key_read: Mutex::new(None),
            launcher_polls: Mutex::new(0),
            pending_clear: Mutex::new(None),
            unknown_polls: Mutex::new(0),
            cut_short_steps: Mutex::new(0),
            restart_times: Mutex::new(Vec::new()),
            config_check: Mutex::new(None),
            check_deadlines: Mutex::new(Vec::new()),
            failed_reads: Mutex::new(0),
            reads: Mutex::new(Vec::new()),
            hermes: Mutex::new(HermesSide {
                pool: PoolEntries::Present,
                override_openrouter: Tri::Present,
                override_codex: Tri::Present,
                codex: CodexStateFact::SignedIn,
                launcher_clears_overrides: true,
            }),
        });
        let ledger = Ledger::open(temp.path().join("agentd.sqlite3")).unwrap();
        let mut executor = Executor::new(
            Arc::clone(&fake),
            ConfigManager::new(home.join("config.yaml"), ledger),
            intent_path,
            fp(),
        );
        executor.backoff = [Duration::ZERO; 3];
        // Production proportions: a read every tick, twelve ticks a window.
        executor.verify_delay = Duration::from_millis(50);
        executor.launcher_wait = executor.verify_delay * 12;
        Setup {
            _temp: temp,
            fake,
            executor,
        }
    }

    fn arm(
        setup: &Setup,
        kind: IntentKind,
        route: IntentRoute,
        model: Option<&str>,
    ) -> IntentRecord {
        let record = IntentRecord::new(kind, route, model.map(str::to_owned)).unwrap();
        intent::store(&setup.fake.intent_path, &record).unwrap();
        record
    }

    fn at_phase(setup: &Setup, mut record: IntentRecord, phase: IntentPhase) {
        record.phase = phase;
        intent::store(&setup.fake.intent_path, &record).unwrap();
    }

    fn record_now(setup: &Setup) -> Option<IntentRecord> {
        intent::load(&setup.fake.intent_path).unwrap()
    }

    fn serve_events(events: &[Event]) -> Vec<(Option<IntentPhase>, bool)> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::RestartServe { phase, gated } => Some((*phase, *gated)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_backoff_is_zero_fifteen_and_sixty_seconds() {
        assert_eq!(
            BACKOFF,
            [
                Duration::ZERO,
                Duration::from_secs(15),
                Duration::from_secs(60)
            ]
        );
        assert_eq!(VERIFY_DELAY, Duration::from_secs(5));
        assert_eq!(LAUNCHER_WAIT, Duration::from_secs(60));
    }

    fn gateway_restarts(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, Event::RestartGateway(_)))
            .count()
    }

    fn facts_reads(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, Event::Facts(_)))
            .count()
    }

    /// A setup whose launcher window is long enough that a scheduling delay
    /// on a loaded machine never ends it before the reads a test counts.
    fn roomy(route: IntentRoute) -> Setup {
        let mut setup = disconnect_setup(route);
        setup.executor.verify_delay = Duration::from_millis(20);
        setup.executor.launcher_wait = Duration::from_secs(5);
        setup
    }

    #[tokio::test]
    async fn r14_a_launcher_that_clears_after_20_s_is_waited_for() {
        // Reads are 5 s apart in production; this launcher's clears land at
        // the fifth read, 20 s after the restart.
        let setup = roomy(IntentRoute::Openrouter);
        *setup.fake.launcher_polls.lock().unwrap() = 5;
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        assert!(record_now(&setup).is_none(), "succeeded");
        let events = setup.fake.events();
        // Exactly one restart after cleanup: the launcher was never stopped
        // while its step ran.
        assert_eq!(gateway_restarts(&events), 1, "{events:?}");
        assert!(
            setup.fake.pending_clear.lock().unwrap().is_none(),
            "the step finished"
        );
        // Four reads before the clears, the clearing read, its confirmation.
        assert_eq!(facts_reads(&events), 6);
        let operation = setup.executor.operation(None).unwrap();
        assert_eq!(operation.state, OperationState::Succeeded);
        assert_eq!(operation.attempts, 1);
    }

    #[tokio::test]
    async fn r14_a_launcher_that_never_clears_fails_after_full_windows() {
        let setup = disconnect_setup(IntentRoute::Openrouter);
        *setup.fake.launcher_polls.lock().unwrap() = usize::MAX;
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        let finished = Instant::now();
        let failed = record_now(&setup).unwrap();
        assert_eq!(failed.state, IntentState::Failed);
        assert_eq!(failed.error_code.as_deref(), Some("verify_failed"));
        let events = setup.fake.events();
        assert_eq!(gateway_restarts(&events), 1 + MAX_REAPPLIES);
        // No re-run, and no failure, before a whole window has passed.
        let mut ends = setup.fake.restart_times.lock().unwrap().clone();
        ends.push(finished);
        for pair in ends.windows(2) {
            assert!(
                pair[1] - pair[0] >= setup.executor.launcher_wait,
                "a window ended after {:?}",
                pair[1] - pair[0]
            );
        }
        // Each window read the facts more than once while it waited.
        assert!(facts_reads(&events) >= (1 + MAX_REAPPLIES) * 2);
    }

    #[tokio::test]
    async fn r14_unknown_during_the_wait_is_not_a_mismatch() {
        let setup = roomy(IntentRoute::Openrouter);
        *setup.fake.unknown_polls.lock().unwrap() = 6;
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        assert!(record_now(&setup).is_none(), "succeeded");
        let events = setup.fake.events();
        assert_eq!(gateway_restarts(&events), 1, "no re-run while unknown");
        assert_eq!(
            facts_reads(&events),
            8,
            "six unknown reads, then cleared and confirmed"
        );
    }

    #[tokio::test]
    async fn r14_unknown_at_the_deadline_is_a_mismatch() {
        let setup = disconnect_setup(IntentRoute::Openrouter);
        *setup.fake.unknown_polls.lock().unwrap() = usize::MAX;
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        let failed = record_now(&setup).unwrap();
        assert_eq!(failed.error_code.as_deref(), Some("verify_failed"));
        assert_eq!(gateway_restarts(&setup.fake.events()), 1 + MAX_REAPPLIES);
    }

    /// The start time of each facts read in `attempt`, and whether it failed.
    fn reads_in(setup: &Setup, attempt: u32) -> Vec<(bool, Instant)> {
        setup
            .fake
            .reads
            .lock()
            .unwrap()
            .iter()
            .filter(|(of, _, _)| *of == attempt)
            .map(|(_, failed, at)| (*failed, *at))
            .collect()
    }

    #[tokio::test]
    async fn r15_a_cut_short_step_and_a_slow_helper_end_succeeded_after_one_more_start() {
        // The E-0 run R15 comes from: the first start's launcher step hit its
        // limit after clearing the pool entry and before the override, and
        // the attempt's first facts reads failed.
        let mut setup = disconnect_setup(IntentRoute::Openrouter);
        setup.executor.verify_delay = Duration::from_millis(20);
        setup.executor.launcher_wait = Duration::from_secs(1);
        *setup.fake.cut_short_steps.lock().unwrap() = 1;
        *setup.fake.failed_reads.lock().unwrap() = 5;
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        assert!(record_now(&setup).is_none(), "succeeded");
        let operation = setup.executor.operation(None).unwrap();
        assert_eq!(operation.state, OperationState::Succeeded);
        assert_eq!(operation.attempts, 1, "never helper_unavailable");
        let events = setup.fake.events();
        assert_eq!(gateway_restarts(&events), 2, "one re-run: {events:?}");
        // The re-run came once the window had passed, and a working read
        // found the override still there.
        let restarts = setup.fake.restart_times.lock().unwrap().clone();
        assert!(restarts[1] - restarts[0] >= setup.executor.launcher_wait);
        let before_rerun = reads_in(&setup, 1)
            .into_iter()
            .filter(|(_, at)| *at < restarts[1])
            .collect::<Vec<_>>();
        assert!(before_rerun[..5].iter().all(|(failed, _)| *failed));
        assert!(!before_rerun.last().unwrap().0, "{before_rerun:?}");
        // The second start finished the clears.
        let hermes = *setup.fake.hermes.lock().unwrap();
        assert_eq!(hermes.pool, PoolEntries::None);
        assert_eq!(hermes.override_openrouter, Tri::Absent);
        assert_eq!(setup.fake.env_lines(), ["KEEP=1"]);
        assert_eq!(setup.fake.model(), fp_block());
    }

    #[tokio::test]
    async fn r15_a_helper_that_fails_every_window_is_helper_unavailable_after_the_retries() {
        let setup = disconnect_setup(IntentRoute::Openrouter);
        *setup.fake.failed_reads.lock().unwrap() = usize::MAX;
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        let finished = Instant::now();
        let failed = record_now(&setup).unwrap();
        assert_eq!(failed.state, IntentState::Failed);
        assert_eq!(failed.error_code.as_deref(), Some("helper_unavailable"));
        assert_eq!(failed.attempts, 3);
        // Only the cleanup restart: a failed read never restarts the gateway.
        assert_eq!(gateway_restarts(&setup.fake.events()), 1);
        // Each attempt read through its whole window before it counted.
        let mut ends = (2..=3)
            .map(|attempt| reads_in(&setup, attempt)[0].1)
            .collect::<Vec<_>>();
        ends.push(finished);
        for (attempt, end) in (1..=3).zip(ends) {
            let reads = reads_in(&setup, attempt);
            assert!(reads.len() >= 2, "attempt {attempt}");
            assert!(reads.iter().all(|(failed, _)| *failed));
            assert!(
                end - reads[0].1 >= setup.executor.launcher_wait,
                "attempt {attempt} ended after {:?}",
                end - reads[0].1
            );
        }
    }

    #[tokio::test]
    async fn r15_a_failed_read_then_a_cleared_one_inside_the_window_succeeds() {
        let setup = roomy(IntentRoute::Openrouter);
        *setup.fake.failed_reads.lock().unwrap() = 2;
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        assert!(record_now(&setup).is_none(), "succeeded");
        let operation = setup.executor.operation(None).unwrap();
        assert_eq!(operation.state, OperationState::Succeeded);
        assert_eq!(operation.attempts, 1);
        let events = setup.fake.events();
        assert_eq!(gateway_restarts(&events), 1, "{events:?}");
        let reads = reads_in(&setup, 1)
            .into_iter()
            .map(|(failed, _)| failed)
            .collect::<Vec<_>>();
        assert_eq!(reads, [true, true, false, false], "cleared and confirmed");
    }

    /// A stand-in `hermes` whose `config check` hangs. It uses absolute paths
    /// only, and appends "<its pid> <its sleeping child's pid>" to `pids`.
    pub(crate) fn hanging_config_check(dir: &Path) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("hanging-hermes");
        let pids = dir.join("config-check.pids");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nPATH=/usr/bin:/bin\n/bin/sleep 600 &\necho \"$$ $!\" >> '{}'\nwait\n",
                pids.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        (script, pids)
    }

    /// Every pid the stand-in recorded, from complete lines only.
    pub(crate) fn recorded_pids(pids: &Path) -> Vec<i32> {
        let text = fs::read_to_string(pids).unwrap_or_default();
        text.split_inclusive('\n')
            .filter(|line| line.ends_with('\n'))
            .flat_map(|line| line.split_whitespace())
            .filter_map(|pid| pid.parse().ok())
            .collect()
    }

    /// Polls (bounded, 30 s) until every pid is gone.
    pub(crate) async fn all_gone(pids: &[i32]) {
        tokio::time::timeout(Duration::from_secs(30), async {
            while pids.iter().any(|pid| {
                rustix::process::test_kill_process(rustix::process::Pid::from_raw(*pid).unwrap())
                    .is_ok()
            }) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("a hung config check outlived its deadline: {pids:?}"));
    }

    #[tokio::test]
    async fn r18_a_hung_config_check_is_config_invalid_and_retried() {
        let mut setup = new_setup(&fp_block(), "OPENROUTER_API_KEY=sk-or-v1-synthetic\n");
        let (script, pids) = hanging_config_check(&setup.fake.home);
        *setup.fake.config_check.lock().unwrap() = Some(script);
        let deadline = Duration::from_secs(1);
        setup.executor.config_check_deadline = deadline;
        let before = fs::read(setup.fake.config_path()).unwrap();
        arm(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            Some(OPENROUTER_MODEL),
        );
        let started = Instant::now();
        setup.executor.run().await;
        let failed = record_now(&setup).unwrap();
        assert_eq!(failed.state, IntentState::Failed);
        assert_eq!(failed.error_code.as_deref(), Some("config_invalid"));
        assert_eq!(failed.attempts, 3, "the bounded retry ran");
        assert_eq!(*setup.fake.check_deadlines.lock().unwrap(), [deadline; 3]);
        assert!(
            started.elapsed() >= deadline * 3,
            "each check ran to its deadline"
        );
        assert_eq!(
            fs::read(setup.fake.config_path()).unwrap(),
            before,
            "restored"
        );
        assert_eq!(gateway_restarts(&setup.fake.events()), 0);
        // Each hung check and its sleeping child were killed with the group.
        // A check killed before its script got to record is killed all the
        // same; the ones that recorded prove it.
        let recorded = recorded_pids(&pids);
        assert!(
            recorded.len() >= 2 && recorded.len().is_multiple_of(2),
            "{recorded:?}"
        );
        all_gone(&recorded).await;
        assert_eq!(CONFIG_CHECK_DEADLINE, Duration::from_secs(60));
    }

    fn failed_record(setup: &Setup, kind: IntentKind, phase: IntentPhase) -> Vec<u8> {
        let mut record = IntentRecord::new(
            kind,
            IntentRoute::Openrouter,
            Some(OPENROUTER_MODEL.to_owned()),
        )
        .unwrap();
        record.phase = phase;
        record.state = IntentState::Failed;
        record.error_code = Some("config_conflict".to_owned());
        record.attempts = 1;
        intent::store(&setup.fake.intent_path, &record).unwrap();
        fs::read(&setup.fake.intent_path).unwrap()
    }

    #[tokio::test]
    async fn r12_a_failed_select_or_activate_is_left_alone_at_startup() {
        for kind in [IntentKind::Select, IntentKind::Activate] {
            // The user chose another model in chat after the operation failed.
            let users_choice = json!({"default": "someone/else", "provider": "openrouter"});
            let setup = new_setup(&users_choice, "OPENROUTER_API_KEY=sk-or-v1-synthetic\n");
            let record = failed_record(&setup, kind, IntentPhase::Verifying);
            let config = fs::read(setup.fake.config_path()).unwrap();
            setup.executor.resume_at_startup().await;
            assert_eq!(
                fs::read(&setup.fake.intent_path).unwrap(),
                record,
                "{kind:?}"
            );
            assert!(setup.fake.events().is_empty(), "{kind:?}: no host call");
            assert_eq!(
                fs::read(setup.fake.config_path()).unwrap(),
                config,
                "{kind:?}"
            );
        }
    }

    #[tokio::test]
    async fn r12_a_failed_disconnect_is_resumed_at_startup() {
        let setup = disconnect_setup(IntentRoute::Openrouter);
        let mut record =
            IntentRecord::new(IntentKind::Disconnect, IntentRoute::Openrouter, None).unwrap();
        record.state = IntentState::Failed;
        record.error_code = Some("helper_unavailable".to_owned());
        intent::store(&setup.fake.intent_path, &record).unwrap();
        setup.executor.resume_at_startup().await;
        assert!(record_now(&setup).is_none(), "completed");
        assert_eq!(setup.fake.model(), fp_block());
        assert_eq!(setup.fake.env_lines(), ["KEEP=1"]);
        assert_eq!(
            setup.executor.operation(None).unwrap().id,
            record.id,
            "the same operation"
        );
    }

    #[tokio::test]
    async fn r12_a_select_beaten_by_another_writer_does_not_come_back_after_a_restart() {
        // Another writer (for example `/model ... --global`) keeps changing
        // the model, so the select ends `config_conflict`.
        let setup = new_setup(
            &openrouter_block(),
            "OPENROUTER_API_KEY=sk-or-v1-synthetic\n",
        );
        *setup.fake.on_restart.lock().unwrap() = Box::new(|_, path| {
            let text = fs::read_to_string(path).unwrap();
            fs::write(path, text.replace("glm-5-3-flash", "users-own-choice")).unwrap();
            Ok(())
        });
        arm(&setup, IntentKind::Select, IntentRoute::FinitePrivate, None);
        setup.executor.run().await;
        let failed = fs::read(&setup.fake.intent_path).unwrap();
        assert_eq!(
            record_now(&setup).unwrap().error_code.as_deref(),
            Some("config_conflict")
        );
        assert_eq!(setup.fake.model()["default"], "users-own-choice");
        let restarts = *setup.fake.restarts.lock().unwrap();

        // agentd restarts.
        *setup.fake.on_restart.lock().unwrap() = Box::new(|_, _| Ok(()));
        setup.executor.resume_at_startup().await;
        assert_eq!(setup.fake.model()["default"], "users-own-choice");
        assert_eq!(fs::read(&setup.fake.intent_path).unwrap(), failed);
        assert_eq!(
            *setup.fake.restarts.lock().unwrap(),
            restarts,
            "nothing re-applied"
        );
    }

    #[tokio::test]
    async fn with_no_record_the_executor_does_nothing() {
        let setup = new_setup(&fp_block(), "");
        setup.executor.run().await;
        setup.executor.resume_at_startup().await;
        assert!(setup.fake.events().is_empty());
        assert!(record_now(&setup).is_none());
        assert!(setup.executor.operation(None).is_none());
    }

    #[tokio::test]
    async fn t_a7_select_writes_each_phase_before_its_step() {
        let setup = new_setup(&fp_block(), "OPENROUTER_API_KEY=sk-or-v1-synthetic\n");
        let record = arm(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            Some(OPENROUTER_MODEL),
        );
        setup.executor.run().await;
        assert_eq!(
            setup.fake.events(),
            [
                Event::Migrate(Some(IntentPhase::ConfigWritten)),
                Event::Validate(Some(IntentPhase::ConfigWritten)),
                Event::RestartGateway(Some(IntentPhase::Restarting)),
            ]
        );
        assert_eq!(setup.fake.model(), openrouter_block());
        assert!(record_now(&setup).is_none());
        let operation = setup.executor.operation(None).unwrap();
        assert_eq!(operation.id, record.id);
        assert_eq!(operation.state, OperationState::Succeeded);
        assert_eq!(operation.attempts, 1);
    }

    #[tokio::test]
    async fn t_a7_select_resumes_from_each_phase() {
        for phase in IntentKind::Select.phases() {
            for kind in [IntentKind::Select, IntentKind::Activate] {
                // Past `config_written` the block may already be on disk.
                let written = !matches!(phase, IntentPhase::Accepted | IntentPhase::ConfigWritten);
                let start = if written {
                    openrouter_block()
                } else {
                    fp_block()
                };
                let setup = new_setup(&start, "OPENROUTER_API_KEY=sk-or-v1-synthetic\n");
                let record = arm(
                    &setup,
                    kind,
                    IntentRoute::Openrouter,
                    Some(OPENROUTER_MODEL),
                );
                at_phase(&setup, record, *phase);
                setup.executor.run().await;
                assert!(record_now(&setup).is_none(), "{kind:?} {phase:?}");
                assert_eq!(setup.fake.model(), openrouter_block());
                let events = setup.fake.events();
                let restarts = events
                    .iter()
                    .filter(|event| matches!(event, Event::RestartGateway(_)))
                    .count();
                assert_eq!(
                    restarts,
                    usize::from(*phase != IntentPhase::Verifying),
                    "{kind:?} {phase:?}: {events:?}"
                );
                // Only the steps from the recorded phase on run.
                let wrote = events
                    .iter()
                    .any(|event| matches!(event, Event::Migrate(_)));
                assert_eq!(wrote, !written, "{kind:?} {phase:?}");
            }
        }
    }

    fn disconnect_setup(route: IntentRoute) -> Setup {
        let (model, env) = match route {
            IntentRoute::Openrouter => (
                openrouter_block(),
                "KEEP=1\nOPENROUTER_API_KEY=sk-or-v1-synthetic\n",
            ),
            _ => (
                json!({"default": "gpt-5.5", "provider": "openai-codex"}),
                "KEEP=1\n",
            ),
        };
        new_setup(&model, env)
    }

    #[tokio::test]
    async fn t_a7_disconnect_runs_every_step_in_its_phase() {
        let setup = disconnect_setup(IntentRoute::OpenaiCodex);
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::OpenaiCodex,
            None,
        );
        setup.executor.run().await;
        assert_eq!(
            setup.fake.events(),
            [
                Event::CancelLogin(Some(IntentPhase::LoginCancelled)),
                Event::Validate(Some(IntentPhase::RouteSwitched)),
                Event::RestartServe {
                    phase: Some(IntentPhase::Cleanup),
                    gated: true
                },
                Event::RestartGateway(Some(IntentPhase::Cleanup)),
                Event::Facts(Some(IntentPhase::Verifying)),
                Event::Facts(Some(IntentPhase::Verifying)),
                Event::RestartServe {
                    phase: None,
                    gated: false
                },
            ]
        );
        assert_eq!(setup.fake.model(), fp_block());
        assert!(record_now(&setup).is_none());

        let setup = disconnect_setup(IntentRoute::Openrouter);
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        let events = setup.fake.events();
        assert_eq!(
            events[..3],
            [
                Event::Validate(Some(IntentPhase::RouteSwitched)),
                Event::RemoveKey(Some(IntentPhase::CredentialRemoved)),
                Event::RestartServe {
                    phase: Some(IntentPhase::Cleanup),
                    gated: true
                },
            ]
        );
        assert_eq!(setup.fake.env_lines(), ["KEEP=1"]);
        assert!(record_now(&setup).is_none());
    }

    #[tokio::test]
    async fn t_a7_disconnect_resumes_from_each_phase() {
        for route in [IntentRoute::Openrouter, IntentRoute::OpenaiCodex] {
            for phase in IntentKind::Disconnect.phases() {
                let setup = disconnect_setup(route);
                let record = arm(&setup, IntentKind::Disconnect, route, None);
                at_phase(&setup, record, *phase);
                setup.executor.run().await;
                let events = setup.fake.events();
                assert!(
                    record_now(&setup).is_none(),
                    "{route:?} {phase:?}: {events:?}"
                );
                // Every restart that ran the launcher step saw a gated serve.
                let serve = serve_events(&events);
                assert_eq!(serve.last(), Some(&(None, false)), "{route:?} {phase:?}");
                assert!(serve[..serve.len() - 1].iter().all(|(_, gated)| *gated));
                assert!(
                    !events
                        .iter()
                        .any(|event| matches!(event, Event::CancelLogin(_)))
                        || route == IntentRoute::OpenaiCodex
                );
            }
        }
    }

    #[tokio::test]
    async fn t_a7_bounded_retry_then_failed_and_kept() {
        let setup = disconnect_setup(IntentRoute::OpenaiCodex);
        let record = arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::OpenaiCodex,
            None,
        );
        *setup.fake.failed_reads.lock().unwrap() = usize::MAX;
        setup.executor.run().await;
        let failed = record_now(&setup).expect("a failed record is kept");
        assert_eq!(failed.id, record.id);
        assert_eq!(failed.state, IntentState::Failed);
        assert_eq!(failed.error_code.as_deref(), Some("helper_unavailable"));
        assert_eq!(failed.attempts, 3);
        assert_eq!(failed.phase, IntentPhase::Verifying);
        // R15b: a failed read is only "not yet", so each attempt kept reading
        // through its launcher window before it counted.
        for attempt in 1..=3 {
            assert!(reads_in(&setup, attempt).len() >= 2, "attempt {attempt}");
        }
        // A failed disconnect keeps `hermes serve` stopped.
        assert!(
            serve_events(&setup.fake.events())
                .iter()
                .all(|(_, gated)| *gated)
        );
        assert_eq!(
            setup.executor.operation(Some(&failed)).unwrap().state,
            OperationState::Failed
        );

        // A later run does not touch a failed record; startup re-arms it.
        setup.executor.run().await;
        assert_eq!(record_now(&setup).unwrap(), failed);
        *setup.fake.failed_reads.lock().unwrap() = 0;
        setup.executor.resume_at_startup().await;
        assert!(record_now(&setup).is_none());
    }

    #[tokio::test]
    async fn backoff_delays_each_retry() {
        let mut setup = disconnect_setup(IntentRoute::OpenaiCodex);
        setup.executor.backoff = [
            Duration::ZERO,
            Duration::from_millis(150),
            Duration::from_millis(300),
        ];
        let record = arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::OpenaiCodex,
            None,
        );
        at_phase(&setup, record, IntentPhase::Verifying);
        *setup.fake.failed_reads.lock().unwrap() = usize::MAX;
        let started = Instant::now();
        setup.executor.run().await;
        assert!(started.elapsed() >= Duration::from_millis(450));
        assert_eq!(record_now(&setup).unwrap().state, IntentState::Failed);
    }

    #[tokio::test]
    async fn t_a15_serve_restarts_on_credential_changes_only() {
        let count = |events: Vec<Event>| serve_events(&events).len();

        let setup = new_setup(&fp_block(), "OPENROUTER_API_KEY=sk-or-v1-synthetic\n");
        arm(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            Some(OPENROUTER_MODEL),
        );
        setup.executor.run().await;
        assert_eq!(count(setup.fake.events()), 0, "a plain select");

        let setup = new_setup(&fp_block(), "OPENROUTER_API_KEY=sk-or-v1-synthetic\n");
        arm(
            &setup,
            IntentKind::Activate,
            IntentRoute::Openrouter,
            Some(OPENROUTER_MODEL),
        );
        setup.executor.run().await;
        assert_eq!(
            serve_events(&setup.fake.events()),
            [(Some(IntentPhase::Restarting), false)],
            "a replaced key"
        );

        let setup = disconnect_setup(IntentRoute::Openrouter);
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        assert_eq!(count(setup.fake.events()), 2, "a removed key");
    }

    #[tokio::test]
    async fn t_a43_serve_starts_only_after_the_restart_that_ran_the_launcher_step() {
        let setup = disconnect_setup(IntentRoute::Openrouter);
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        let events = setup.fake.events();
        let position = |wanted: &Event| events.iter().position(|event| event == wanted).unwrap();
        let stop = position(&Event::RestartServe {
            phase: Some(IntentPhase::Cleanup),
            gated: true,
        });
        let gateway = position(&Event::RestartGateway(Some(IntentPhase::Cleanup)));
        let verify = position(&Event::Facts(Some(IntentPhase::Verifying)));
        let start = position(&Event::RestartServe {
            phase: None,
            gated: false,
        });
        assert!(
            stop < gateway && gateway < verify && verify < start,
            "{events:?}"
        );
        // The launcher step saw the route already switched and cleared.
        let hermes = *setup.fake.hermes.lock().unwrap();
        assert_eq!(hermes.pool, PoolEntries::None);
        assert_eq!(hermes.override_openrouter, Tri::Absent);
    }

    #[tokio::test]
    async fn disconnect_repeats_steps_then_reports_verify_failed() {
        let setup = disconnect_setup(IntentRoute::Openrouter);
        setup.fake.hermes.lock().unwrap().launcher_clears_overrides = false;
        arm(
            &setup,
            IntentKind::Disconnect,
            IntentRoute::Openrouter,
            None,
        );
        setup.executor.run().await;
        let failed = record_now(&setup).unwrap();
        assert_eq!(failed.state, IntentState::Failed);
        assert_eq!(failed.error_code.as_deref(), Some("verify_failed"));
        let gateway_restarts = setup
            .fake
            .events()
            .iter()
            .filter(|event| matches!(event, Event::RestartGateway(_)))
            .count();
        assert_eq!(gateway_restarts, 1 + MAX_REAPPLIES);
        // The re-runs happen at phase `verifying`; the phase never moves back.
        assert!(setup.fake.events().iter().all(|event| !matches!(
            event,
            Event::RestartGateway(Some(
                IntentPhase::RouteSwitched | IntentPhase::CredentialRemoved
            ))
        )));
    }

    /// Rewrites `config.yaml` the way the real gateway does on a first turn (G1).
    fn first_turn_rewrite(path: &Path, change_model: bool) {
        let mut document =
            serde_yaml::from_str::<Value>(&fs::read_to_string(path).unwrap()).unwrap();
        document["onboarding"] = json!({"seen": {"profile_build_offered": true}});
        document["agent"] = json!({});
        if change_model {
            document["model"]["default"] = json!("someone/else");
        }
        let text = serde_yaml::to_string(&document)
            .unwrap()
            .replace("\n- ", "\n  - ");
        fs::write(
            path,
            format!("{text}# fallback_model:\n#   provider: openrouter\n"),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn t_a42_a_first_turn_rewrite_is_not_a_mismatch() {
        let setup = new_setup(
            &openrouter_block(),
            "OPENROUTER_API_KEY=sk-or-v1-synthetic\n",
        );
        *setup.fake.on_restart.lock().unwrap() = Box::new(|_, path| {
            first_turn_rewrite(path, false);
            Ok(())
        });
        arm(&setup, IntentKind::Select, IntentRoute::FinitePrivate, None);
        setup.executor.run().await;
        assert!(record_now(&setup).is_none());
        assert_eq!(*setup.fake.restarts.lock().unwrap(), 1, "no re-apply");
        let text = fs::read_to_string(setup.fake.config_path()).unwrap();
        assert!(text.contains("profile_build_offered"));

        let setup = new_setup(
            &openrouter_block(),
            "OPENROUTER_API_KEY=sk-or-v1-synthetic\n",
        );
        *setup.fake.on_restart.lock().unwrap() = Box::new(|count, path| {
            first_turn_rewrite(path, count == 1);
            Ok(())
        });
        arm(&setup, IntentKind::Select, IntentRoute::FinitePrivate, None);
        setup.executor.run().await;
        assert!(record_now(&setup).is_none());
        assert_eq!(*setup.fake.restarts.lock().unwrap(), 2, "one re-apply");
        assert_eq!(setup.fake.model(), fp_block());
    }

    #[tokio::test]
    async fn t_a14_a_stale_writer_is_reapplied_then_reported_as_found() {
        // Clobbered once, after the first check passed: the second read
        // (5 s later in production) catches it. Deterministic: the clobber
        // runs inside the first check, after its model read.
        let setup = new_setup(&fp_block(), "OPENROUTER_API_KEY=sk-or-v1-synthetic\n");
        *setup.fake.on_verify_key_read.lock().unwrap() = Some(Box::new(|path| {
            let text = fs::read_to_string(path).unwrap();
            fs::write(path, text.replace(OPENROUTER_MODEL, "stale-model")).unwrap();
        }));
        arm(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            Some(OPENROUTER_MODEL),
        );
        setup.executor.run().await;
        assert!(record_now(&setup).is_none());
        assert_eq!(*setup.fake.restarts.lock().unwrap(), 2);
        assert_eq!(setup.fake.model(), openrouter_block());

        // Clobbered after every restart: two re-applies, then the state as found.
        let setup = new_setup(
            &openrouter_block(),
            "OPENROUTER_API_KEY=sk-or-v1-synthetic\n",
        );
        *setup.fake.on_restart.lock().unwrap() = Box::new(|_, path| {
            let text = fs::read_to_string(path).unwrap();
            fs::write(path, text.replace("glm-5-3-flash", "stale-model")).unwrap();
            Ok(())
        });
        arm(&setup, IntentKind::Select, IntentRoute::FinitePrivate, None);
        setup.executor.run().await;
        let failed = record_now(&setup).unwrap();
        assert_eq!(failed.state, IntentState::Failed);
        assert_eq!(failed.error_code.as_deref(), Some("config_conflict"));
        assert_eq!(*setup.fake.restarts.lock().unwrap(), 1 + MAX_REAPPLIES);
        assert_eq!(setup.fake.model()["default"], "stale-model");
    }

    #[tokio::test]
    async fn a_spawn_failure_restores_intact_bytes_and_fails() {
        let setup = new_setup(&fp_block(), "OPENROUTER_API_KEY=sk-or-v1-synthetic\n");
        let before = fs::read(setup.fake.config_path()).unwrap();
        *setup.fake.on_restart.lock().unwrap() = Box::new(|count, _| {
            if count == 1 {
                Err(AgentdError::Supervisor("spawn failed".to_owned()))
            } else {
                Ok(())
            }
        });
        arm(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            Some(OPENROUTER_MODEL),
        );
        setup.executor.run().await;
        let failed = record_now(&setup).unwrap();
        assert_eq!(failed.error_code.as_deref(), Some("supervisor_unavailable"));
        assert_eq!(failed.attempts, 1, "not retried");
        assert_eq!(fs::read(setup.fake.config_path()).unwrap(), before);
        assert_eq!(
            *setup.fake.restarts.lock().unwrap(),
            2,
            "previous route restarted"
        );

        // Another writer touched the file: it is left as found.
        let setup = new_setup(&fp_block(), "OPENROUTER_API_KEY=sk-or-v1-synthetic\n");
        *setup.fake.on_restart.lock().unwrap() = Box::new(|_, path| {
            let mut text = fs::read_to_string(path).unwrap();
            text.push_str("agent: {}\n");
            fs::write(path, text).unwrap();
            Err(AgentdError::Supervisor("spawn failed".to_owned()))
        });
        arm(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            Some(OPENROUTER_MODEL),
        );
        setup.executor.run().await;
        assert_eq!(
            record_now(&setup).unwrap().error_code.as_deref(),
            Some("config_conflict")
        );
        assert_eq!(setup.fake.model(), openrouter_block());
    }

    #[tokio::test]
    async fn a_spawn_that_really_fails_goes_through_the_real_supervisor() {
        use crate::supervisor::{ProcessSpec, SupervisorHandle, start_supervisor};

        #[derive(Clone)]
        struct Real {
            fake: Arc<Fake>,
            supervisor: SupervisorHandle,
            program: PathBuf,
        }
        impl ExecutorHost for Real {
            fn validate_config(&self, deadline: Duration) -> Result<(), AgentdError> {
                self.fake.validate_config(deadline)
            }
            fn dotenv_openrouter_key(&self) -> Result<Option<String>, AgentdError> {
                self.fake.dotenv_openrouter_key()
            }
            fn migrate_legacy_openrouter_key(&self) -> Result<(), AgentdError> {
                self.fake.migrate_legacy_openrouter_key()
            }
            fn remove_openrouter_key(&self) -> Result<(), AgentdError> {
                self.fake.remove_openrouter_key()
            }
            async fn restart_gateway(&self) -> Result<(), AgentdError> {
                let result = self.supervisor.restart_hermes().await;
                // Put the program back so the restore's restart can succeed.
                write_sleeper(&self.program);
                result
            }
            async fn restart_serve(&self) {}
            async fn facts(&self, deadline: Duration) -> Result<InferenceFacts, AgentdError> {
                self.fake.facts(deadline).await
            }
            async fn cancel_codex_login(&self) -> Result<(), AgentdError> {
                Ok(())
            }
        }
        fn write_sleeper(path: &Path) {
            use std::os::unix::fs::PermissionsExt;
            fs::write(path, "#!/bin/sh\nexec sleep 60\n").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let sleeper = |name: &'static str, program: &Path| ProcessSpec {
            name,
            program: program.to_owned(),
            args: Vec::new(),
            // The system directories only, never the host's PATH.
            environment: std::collections::BTreeMap::from([(
                "PATH".to_owned(),
                "/usr/bin:/bin".to_owned(),
            )]),
        };

        let setup = new_setup(&fp_block(), "OPENROUTER_API_KEY=sk-or-v1-synthetic\n");
        let program = setup.fake.home.join("gateway");
        write_sleeper(&program);
        let supervisor = start_supervisor(
            sleeper("finitechat", &program),
            sleeper("health", &program),
            sleeper("hermes", &program),
            None,
        );
        tokio::time::timeout(Duration::from_secs(30), async {
            while supervisor
                .status()
                .await
                .processes
                .get("hermes")
                .and_then(|s| s.pid())
                .is_none()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        // The next gateway spawn fails for real: its program is gone.
        fs::remove_file(&program).unwrap();
        let before = fs::read(setup.fake.config_path()).unwrap();
        let mut executor = Executor::new(
            Real {
                fake: Arc::clone(&setup.fake),
                supervisor: supervisor.clone(),
                program,
            },
            setup.executor.config.clone(),
            setup.fake.intent_path.clone(),
            fp(),
        );
        executor.backoff = [Duration::ZERO; 3];
        arm(
            &setup,
            IntentKind::Select,
            IntentRoute::Openrouter,
            Some(OPENROUTER_MODEL),
        );
        executor.run().await;
        let failed = record_now(&setup).unwrap();
        assert_eq!(failed.error_code.as_deref(), Some("supervisor_unavailable"));
        assert_eq!(fs::read(setup.fake.config_path()).unwrap(), before);
        supervisor.shutdown().await;
    }
}
