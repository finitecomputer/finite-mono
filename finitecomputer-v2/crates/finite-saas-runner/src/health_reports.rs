//! Runner-ferried standing readiness (2026-08 audit synthesis, H1 slice 3).
//!
//! The runner already has a provider-uniform HTTP read path into every guest
//! it launches (`/contact`, the same bounded JSON document the launch path
//! verifies). This module gives it a standing, throttled poll: once per
//! interval per runtime it reads `/contact` and posts one health report to
//! Core (`POST /api/core/v1/runtime-health-reports`), so a runtime whose
//! compute dies overnight cannot display its last lifecycle success forever.
//!
//! Design choices pinned here:
//!
//! - **Outbound-only telemetry.** This is the Runtime Management Pipe v1
//!   shape (`docs/runtime-management-contract-v1.md`): generic health facts
//!   flowing runtime-side to Core, ferried by the runner. No inbound command
//!   path is added, and a Core outage never interrupts a healthy guest.
//! - **Latest report only.** Core stores one report per runtime and projects
//!   readiness at read time; staleness (no report within 3x the cadence) reads
//!   as the named `stale` state, and a runtime that was never reported on is
//!   `unknown`. There is no sweeper and no history table.
//! - **Transport failure is reported, not skipped.** When nobody answers at
//!   the recorded endpoint the runner posts `ready: false` with reason
//!   `unreachable`, so a dead runtime reads `not_ready` immediately. Staleness
//!   then means exactly one thing — the runner stopped reporting (runner
//!   down, Core unreachable, or a pre-poller runner) — and reads `stale`.
//!   The two failure classes stay distinguishable in the fleet view.
//! - **Every runtime this host owns is registered.** A registry entry is
//!   written or refreshed on every successful up-bound completion (launch,
//!   relocation adoption, restart, recovery, upgrade), not only on fresh
//!   launch, and the registry is reconciled against Core's host-scoped
//!   target list when the runner process starts. Only an explicit stop or
//!   destroy removes an entry; a Core 404 is logged and retried on the
//!   normal cadence, never treated as permission to forget a runtime.
//! - **Identity-pinned attribution.** Ports are reallocated across stops (the
//!   2026-08-07 port-squat class), so a response is only attributed to a
//!   runtime when it presents the pinned Agent Principal npub. The pin is
//!   launch-verified where the launch path verifies identity; legacy-shaped
//!   entries (no launch pin) pin the first presented principal and enforce it
//!   thereafter. Mismatches and unbound answers are dropped, so a squatter's
//!   health never wears this runtime's name — the runtime simply goes stale.

use finite_saas_core::RuntimeHealthReportRequest;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::{AgentCreationQueue, RunnerError};

/// The fixed reason token posted when the guest cannot be fetched at all.
pub const RUNTIME_HEALTH_UNREACHABLE_REASON: &str = "unreachable";
/// Same bound the Kata launch path applies to guest HTTP bodies.
const MAX_CONTACT_RESPONSE_BYTES: u64 = 64 * 1024;
/// Guest-reported not-ready reasons are untrusted input; bound them.
const MAX_CONTACT_REASON_CHARS: usize = 256;

#[derive(Debug, Clone)]
pub struct HealthReportConfig {
    pub registry_dir: PathBuf,
    pub interval: Duration,
    pub http_timeout: Duration,
}

/// One poll target, persisted in the on-disk registry because `run_cycle`
/// rebuilds the runner every cycle. Written when a launch (or cold
/// relocation) completes, written or refreshed when any other up-bound
/// control completes and by the startup reconcile, removed only when a
/// stop/destroy completes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HealthReportTarget {
    pub agent_runtime_id: String,
    pub source_machine_id: String,
    pub contact_endpoint: String,
    /// The pinned Agent Principal npub, or `None` until the first response
    /// pins one (legacy fallback only — current launches arrive pinned).
    #[serde(default)]
    pub agent_npub: Option<String>,
    #[serde(default)]
    last_attempt_unix_ms: Option<u64>,
}

fn valid_runtime_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn target_path(registry_dir: &Path, agent_runtime_id: &str) -> Option<PathBuf> {
    if !valid_runtime_id(agent_runtime_id) {
        return None;
    }
    Some(registry_dir.join(format!("{agent_runtime_id}.json")))
}

/// Best-effort atomic write (`<runtime>.json`, tempfile + rename). A write
/// failure loses telemetry, never the launch it rides on.
fn write_target(registry_dir: &Path, target: &HealthReportTarget) -> Result<(), RunnerError> {
    let path = target_path(registry_dir, &target.agent_runtime_id).ok_or_else(|| {
        RunnerError::RuntimeLaunch(format!(
            "health report target id {} is not a simple identifier",
            target.agent_runtime_id
        ))
    })?;
    std::fs::create_dir_all(registry_dir).map_err(|error| {
        RunnerError::RuntimeLaunch(format!(
            "health report registry {} could not be created: {error}",
            registry_dir.display()
        ))
    })?;
    let temporary = registry_dir.join(format!(".{}.json.tmp", target.agent_runtime_id));
    let bytes = serde_json::to_vec(target)
        .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;
    std::fs::write(&temporary, bytes).map_err(|error| {
        RunnerError::RuntimeLaunch(format!(
            "health report target {} could not be staged: {error}",
            target.agent_runtime_id
        ))
    })?;
    std::fs::rename(&temporary, &path).map_err(|error| {
        RunnerError::RuntimeLaunch(format!(
            "health report target {} could not be persisted: {error}",
            target.agent_runtime_id
        ))
    })
}

/// Register (or replace) a poll target after a successful launch or cold
/// relocation, pinning the launch-verified Agent Principal when one was
/// verified. Best-effort: a failure loses telemetry, not the launch.
pub fn record_target(
    config: &HealthReportConfig,
    agent_runtime_id: &str,
    source_machine_id: &str,
    contact_endpoint: Option<&str>,
    launch_verified_npub: Option<&str>,
) {
    let Some(contact_endpoint) = contact_endpoint else {
        return;
    };
    let target = HealthReportTarget {
        agent_runtime_id: agent_runtime_id.to_string(),
        source_machine_id: source_machine_id.to_string(),
        contact_endpoint: contact_endpoint.to_string(),
        agent_npub: launch_verified_npub.map(str::to_string),
        last_attempt_unix_ms: None,
    };
    if let Err(error) = write_target(&config.registry_dir, &target) {
        eprintln!("warning: health report registration for {agent_runtime_id} failed: {error}");
    }
}

fn read_target(registry_dir: &Path, agent_runtime_id: &str) -> Option<HealthReportTarget> {
    let path = target_path(registry_dir, agent_runtime_id)?;
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<HealthReportTarget>(&bytes).ok())
}

/// The outcome of `ensure_target`, for logging and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureTargetOutcome {
    /// No entry existed; one was created.
    Created,
    /// The entry existed and its endpoint or machine moved.
    Refreshed,
    /// The entry already matched.
    Unchanged,
    /// Nothing to poll (no contact endpoint) or the write failed.
    Skipped,
}

/// Make sure a runtime this host owns has a poll target: create a missing
/// entry, or move an existing one to the current contact endpoint while
/// keeping its identity pin and throttle. Called on every successful
/// up-bound control completion (restart, recovery, upgrade) and by the
/// startup reconcile, so a runtime that was upgraded in place or relocated in
/// is reported on exactly like one launched fresh. A pin is only ever added,
/// never replaced: an existing launch-verified or first-seen principal wins
/// over a caller-supplied one. Best-effort: a failure loses telemetry, never
/// the lifecycle work it rides on.
pub fn ensure_target(
    config: &HealthReportConfig,
    agent_runtime_id: &str,
    source_machine_id: &str,
    contact_endpoint: Option<&str>,
    agent_npub: Option<&str>,
) -> EnsureTargetOutcome {
    let Some(contact_endpoint) = contact_endpoint else {
        eprintln!(
            "warning: health report target for {agent_runtime_id} has no contact endpoint; \
             it cannot be polled"
        );
        return EnsureTargetOutcome::Skipped;
    };
    let (target, outcome) = match read_target(&config.registry_dir, agent_runtime_id) {
        Some(mut existing) => {
            let moved = existing.contact_endpoint != contact_endpoint
                || existing.source_machine_id != source_machine_id;
            let pinned = existing.agent_npub.is_none() && agent_npub.is_some();
            if !moved && !pinned {
                return EnsureTargetOutcome::Unchanged;
            }
            if moved {
                existing.contact_endpoint = contact_endpoint.to_string();
                existing.source_machine_id = source_machine_id.to_string();
                // A moved endpoint deserves a prompt first poll.
                existing.last_attempt_unix_ms = None;
            }
            if pinned {
                existing.agent_npub = agent_npub.map(str::to_string);
            }
            (existing, EnsureTargetOutcome::Refreshed)
        }
        None => (
            HealthReportTarget {
                agent_runtime_id: agent_runtime_id.to_string(),
                source_machine_id: source_machine_id.to_string(),
                contact_endpoint: contact_endpoint.to_string(),
                agent_npub: agent_npub.map(str::to_string),
                last_attempt_unix_ms: None,
            },
            EnsureTargetOutcome::Created,
        ),
    };
    match write_target(&config.registry_dir, &target) {
        Ok(()) => outcome,
        Err(error) => {
            eprintln!("warning: health report registration for {agent_runtime_id} failed: {error}");
            EnsureTargetOutcome::Skipped
        }
    }
}

/// Reconcile the on-disk registry against Core's host-scoped list of the
/// runtimes this runner should be reporting on. Runs once per runner
/// process start. Adds missing entries and moves stale endpoints; it never
/// removes an entry, because only an explicit stop/destroy completion on
/// this host knows a runtime is deliberately gone. A Core that does not
/// serve the listing (or is unreachable) leaves the registry as it is.
pub fn reconcile_targets(
    queue: &mut dyn AgentCreationQueue,
    config: &HealthReportConfig,
    source_host_id: Option<&str>,
) {
    let listing = match queue.list_runtime_health_targets() {
        Ok(Some(listing)) => listing,
        Ok(None) => {
            eprintln!(
                "warning: health report registry reconcile skipped: Core does not list \
                 standing-health targets for this runner"
            );
            return;
        }
        Err(error) => {
            eprintln!("warning: health report registry reconcile skipped: {error}");
            return;
        }
    };
    if let Some(source_host_id) = source_host_id
        && listing.source_host_id != source_host_id
    {
        eprintln!(
            "warning: health report registry reconcile skipped: the runner credential is \
             scoped to host {} but this runner serves {source_host_id}",
            listing.source_host_id
        );
        return;
    }
    let (mut created, mut refreshed, mut skipped) = (0_usize, 0_usize, 0_usize);
    for target in &listing.targets {
        match ensure_target(
            config,
            &target.agent_runtime_id,
            &target.source_machine_id,
            target.contact_endpoint.as_deref(),
            target.agent_npub.as_deref(),
        ) {
            EnsureTargetOutcome::Created => created += 1,
            EnsureTargetOutcome::Refreshed => refreshed += 1,
            EnsureTargetOutcome::Unchanged => {}
            EnsureTargetOutcome::Skipped => skipped += 1,
        }
    }
    eprintln!(
        "health report registry reconciled against Core for host {}: {} listed, {created} \
         registered, {refreshed} refreshed, {skipped} unpollable",
        listing.source_host_id,
        listing.targets.len()
    );
}

/// Deregister a poll target after a stop/destroy completes, so a
/// deliberately offline runtime is not polled and reported unreachable
/// forever. This is the only path that removes an entry.
pub fn remove_target(config: &HealthReportConfig, agent_runtime_id: &str) {
    let Some(path) = target_path(&config.registry_dir, agent_runtime_id) else {
        return;
    };
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!(
            "warning: health report deregistration for {agent_runtime_id} failed: {error}"
        ),
    }
}

fn unix_millis_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn rfc3339_now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("RFC 3339 formatting cannot fail")
}

enum ContactRead {
    /// A parseable, size-bounded JSON body arrived (any HTTP status — the
    /// guest serves its health document even at 503 while not ready).
    Answered(serde_json::Value),
    /// Nobody answered: connection refused, reset, or timeout.
    Unreachable,
    /// A body arrived oversized or unparseable; it cannot be attributed to
    /// the runtime, so no report rides on it.
    Unattributable,
}

fn read_contact(endpoint: &str, timeout: Duration) -> ContactRead {
    let response = match ureq::get(endpoint)
        .timeout(timeout.max(Duration::from_millis(250)))
        .set("Accept", "application/json")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(ureq::Error::Transport(_)) => return ContactRead::Unreachable,
    };
    let mut body = Vec::new();
    if response
        .into_reader()
        .take(MAX_CONTACT_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .is_err()
    {
        return ContactRead::Unattributable;
    }
    if body.len() as u64 > MAX_CONTACT_RESPONSE_BYTES {
        return ContactRead::Unattributable;
    }
    match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(value) => ContactRead::Answered(value),
        Err(_) => ContactRead::Unattributable,
    }
}

/// The identity gate: a response is attributed to a runtime only when it
/// presents the pinned Agent Principal. Returns the npub to report, or `None`
/// to drop the report (the runtime then goes stale rather than wearing a
/// squatter's health).
fn gated_npub(target: &HealthReportTarget, observed: Option<&str>) -> Option<String> {
    match (target.agent_npub.as_deref(), observed) {
        (Some(pinned), Some(observed)) if pinned != observed => {
            eprintln!(
                "warning: health report identity mismatch for {}: pinned {pinned} but \
                 {} answered with {observed} — dropping report (possible port squat)",
                target.agent_runtime_id, target.contact_endpoint
            );
            None
        }
        (Some(pinned), Some(_)) => Some(pinned.to_string()),
        (Some(_), None) => {
            eprintln!(
                "warning: health report response for {} carries no Agent Principal; \
                 dropping report",
                target.agent_runtime_id
            );
            None
        }
        (None, None) => {
            // Never silently report unbound: the fleet view would show
            // identity-less data. Wait for a principal.
            eprintln!(
                "warning: health report response for {} carries no Agent Principal and \
                 none was pinned at launch; dropping report",
                target.agent_runtime_id
            );
            None
        }
        (None, Some(observed)) => Some(observed.to_string()),
    }
}

fn observed_agent_npub(value: &serde_json::Value) -> Option<String> {
    value
        .get("agent_npub")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|npub| npub.starts_with("npub1") && npub.len() <= 256)
        .map(str::to_string)
}

fn bounded_reason(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.chars().take(MAX_CONTACT_REASON_CHARS).collect())
}

/// Poll every due target once and forward one report per runtime to Core.
/// Best-effort, throttled per runtime: telemetry must never fail or slow the
/// lease cycle beyond its own bounded HTTP timeouts. `source_host_id` is the
/// host this runner serves, named in the 404 diagnostic.
pub fn forward_due_reports(
    queue: &mut dyn AgentCreationQueue,
    config: &HealthReportConfig,
    source_host_id: Option<&str>,
) {
    let Ok(entries) = std::fs::read_dir(&config.registry_dir) else {
        return;
    };
    let now_ms = unix_millis_now();
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some(mut target) = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<HealthReportTarget>(&bytes).ok())
        else {
            eprintln!(
                "warning: health report target at {} is unreadable; skipping",
                path.display()
            );
            continue;
        };
        if target
            .last_attempt_unix_ms
            .is_some_and(|last| now_ms.saturating_sub(last) < config.interval.as_millis() as u64)
        {
            continue;
        }
        target.last_attempt_unix_ms = Some(now_ms);
        if let Err(error) = write_target(&config.registry_dir, &target) {
            eprintln!(
                "warning: health report target for {} was not updated: {error}",
                target.agent_runtime_id
            );
        }
        let (ready, reason, agent_npub) = match read_contact(
            &target.contact_endpoint,
            config.http_timeout,
        ) {
            ContactRead::Unreachable => (
                false,
                Some(RUNTIME_HEALTH_UNREACHABLE_REASON.to_string()),
                target.agent_npub.clone(),
            ),
            ContactRead::Unattributable => {
                eprintln!(
                    "warning: health report response for {} was oversized or unparseable; \
                     dropping report",
                    target.agent_runtime_id
                );
                continue;
            }
            ContactRead::Answered(body) => {
                let observed = observed_agent_npub(&body);
                let Some(agent_npub) = gated_npub(&target, observed.as_deref()) else {
                    continue;
                };
                if target.agent_npub.is_none() {
                    // Legacy fallback only: no principal was pinned at launch.
                    // Pin the first presented principal and require every
                    // later read to match. Not TOFU for current launches —
                    // those arrive pinned.
                    target.agent_npub = Some(agent_npub.clone());
                    if let Err(error) = write_target(&config.registry_dir, &target) {
                        eprintln!(
                            "warning: health report identity pin for {} was not persisted: {error}",
                            target.agent_runtime_id
                        );
                    }
                }
                let ready = body
                    .get("ready")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let reason = if ready {
                    None
                } else {
                    bounded_reason(body.get("error").and_then(serde_json::Value::as_str))
                };
                (ready, reason, Some(agent_npub))
            }
        };
        let request = RuntimeHealthReportRequest {
            agent_runtime_id: target.agent_runtime_id.clone(),
            ready,
            reason,
            observed_at: rfc3339_now(),
            agent_npub,
            report_interval_seconds: Some(config.interval.as_secs() as i64),
            now: None,
        };
        if let Err(error) = queue.report_runtime_health(request) {
            // A 404 means Core does not scope this runtime to the host the
            // runner credential asserts: usually a `source_host_id` that
            // drifted after a host move, sometimes a runtime destroyed or
            // relocated away. Either way the fix is an operator's, not a
            // silent deregistration that would hide the runtime forever. The
            // target keeps its slot and its per-target throttle, so the next
            // attempt is one interval away.
            if matches!(error, RunnerError::CoreStatus { status: 404, .. }) {
                eprintln!(
                    "error: Core does not recognise runtime {} on host {} (contact {}); the \
                     runner credential's source host may not match the runtime's — keeping the \
                     health report target and retrying next interval: {error}",
                    target.agent_runtime_id,
                    source_host_id.unwrap_or("<unset>"),
                    target.contact_endpoint
                );
                continue;
            }
            eprintln!(
                "warning: health report for {} failed: {error}",
                target.agent_runtime_id
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentCreationLease, AgentCreationRequest, CompleteAgentCreationRequestInput,
        CompleteRuntimeControlRequestInput, FailAgentCreationRequestInput,
        FailRuntimeControlRequestInput, ProviderOperationEnvelope,
        ProvisionFinitePrivateRuntimeKeyInput, ProvisionFinitePrivateRuntimeKeyResult,
        RegisterAgentCreationRuntimeInput, RenewRuntimeControlRequestInput,
        RetryRuntimeControlRequestInput, RunnerLeaseCapacity, RuntimeControlLease,
        RuntimeControlRequest,
    };
    use finite_saas_core::api::RecordProviderOperationTransitionRequest;
    use finite_saas_core::{
        RuntimeHealthReportAck, RuntimeHealthTarget, RuntimeHealthTargetList, RuntimeSummaryStatus,
    };
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct RecordingQueue {
        reports: Vec<RuntimeHealthReportRequest>,
        report_status_error: Option<u16>,
        listing: Option<RuntimeHealthTargetList>,
    }

    impl AgentCreationQueue for RecordingQueue {
        fn lease_runtime_control(
            &mut self,
            _runner_id: &str,
            _lease_token: &str,
            _lease_seconds: i64,
            _source_host_id: Option<&str>,
            _runner_capacity: Option<&RunnerLeaseCapacity>,
        ) -> Result<Option<RuntimeControlLease>, RunnerError> {
            unimplemented!()
        }

        fn complete_runtime_control(
            &mut self,
            _request_id: &str,
            _input: CompleteRuntimeControlRequestInput,
        ) -> Result<RuntimeControlRequest, RunnerError> {
            unimplemented!()
        }

        fn fail_runtime_control(
            &mut self,
            _request_id: &str,
            _input: FailRuntimeControlRequestInput,
        ) -> Result<RuntimeControlRequest, RunnerError> {
            unimplemented!()
        }

        fn renew_runtime_control(
            &mut self,
            _request_id: &str,
            _input: RenewRuntimeControlRequestInput,
        ) -> Result<RuntimeControlRequest, RunnerError> {
            unimplemented!()
        }

        fn retry_runtime_control(
            &mut self,
            _request_id: &str,
            _input: RetryRuntimeControlRequestInput,
        ) -> Result<RuntimeControlRequest, RunnerError> {
            unimplemented!()
        }

        fn lease_agent_creation(
            &mut self,
            _runner_id: &str,
            _lease_token: &str,
            _lease_seconds: i64,
            _runner_capacity: Option<&RunnerLeaseCapacity>,
        ) -> Result<Option<AgentCreationLease>, RunnerError> {
            unimplemented!()
        }

        fn complete_agent_creation(
            &mut self,
            _request_id: &str,
            _input: CompleteAgentCreationRequestInput,
        ) -> Result<AgentCreationLease, RunnerError> {
            unimplemented!()
        }

        fn register_agent_creation_runtime(
            &mut self,
            _request_id: &str,
            _input: RegisterAgentCreationRuntimeInput,
        ) -> Result<AgentCreationLease, RunnerError> {
            unimplemented!()
        }

        fn record_provider_operation_transition(
            &mut self,
            _request_id: &str,
            _input: RecordProviderOperationTransitionRequest,
        ) -> Result<ProviderOperationEnvelope, RunnerError> {
            unimplemented!()
        }

        fn provision_finite_private_runtime_key(
            &mut self,
            _request_id: &str,
            _input: ProvisionFinitePrivateRuntimeKeyInput,
        ) -> Result<ProvisionFinitePrivateRuntimeKeyResult, RunnerError> {
            unimplemented!()
        }

        fn fail_agent_creation(
            &mut self,
            _request_id: &str,
            _input: FailAgentCreationRequestInput,
        ) -> Result<AgentCreationRequest, RunnerError> {
            unimplemented!()
        }

        fn report_runtime_health(
            &mut self,
            input: RuntimeHealthReportRequest,
        ) -> Result<RuntimeHealthReportAck, RunnerError> {
            if let Some(status) = self.report_status_error {
                return Err(RunnerError::CoreStatus {
                    status,
                    body: "runtime is not on this host".to_string(),
                });
            }
            let ack = RuntimeHealthReportAck {
                agent_runtime_id: input.agent_runtime_id.clone(),
                recorded_at: rfc3339_now(),
            };
            self.reports.push(input);
            Ok(ack)
        }

        fn list_runtime_health_targets(
            &mut self,
        ) -> Result<Option<RuntimeHealthTargetList>, RunnerError> {
            Ok(self.listing.clone())
        }
    }

    /// A minimal `/contact` server: one fixed body per test, one thread, and
    /// a request counter. Mirrors the Kata launch-path test double.
    struct ContactServer {
        port: u16,
        stop: Arc<AtomicBool>,
        requests: Arc<AtomicUsize>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl ContactServer {
        fn start(body: String) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(AtomicUsize::new(0));
            let stop_thread = stop.clone();
            let requests_thread = requests.clone();
            let thread = std::thread::spawn(move || {
                while let Ok((mut stream, _)) = listener.accept() {
                    // The Drop guard connects once to wake the blocking accept.
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut request = [0_u8; 2048];
                    if stream.read(&mut request).unwrap_or_default() == 0 {
                        continue;
                    }
                    requests_thread.fetch_add(1, Ordering::Relaxed);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
                }
            });
            Self {
                port,
                stop,
                requests,
                thread: Some(thread),
            }
        }

        fn endpoint(&self) -> String {
            format!("http://127.0.0.1:{}/contact", self.port)
        }

        fn request_count(&self) -> usize {
            self.requests.load(Ordering::Relaxed)
        }
    }

    impl Drop for ContactServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn test_config(registry_dir: &Path) -> HealthReportConfig {
        HealthReportConfig {
            registry_dir: registry_dir.to_path_buf(),
            interval: Duration::from_secs(60),
            http_timeout: Duration::from_secs(2),
        }
    }

    fn target(endpoint: &str, npub: Option<&str>) -> HealthReportTarget {
        HealthReportTarget {
            agent_runtime_id: "runtime-1".to_string(),
            source_machine_id: "machine-1".to_string(),
            contact_endpoint: endpoint.to_string(),
            agent_npub: npub.map(str::to_string),
            last_attempt_unix_ms: None,
        }
    }

    fn registry_with(dir: &Path, target: &HealthReportTarget) {
        write_target(dir, target).unwrap();
    }

    #[test]
    fn ready_answer_is_reported_with_the_pinned_npub() {
        let registry = tempfile::tempdir().unwrap();
        let server = ContactServer::start(
            r#"{"ready":true,"agent_npub":"npub1qqqqqqqqqqqqqqqqqqqqqqqq"}"#.to_string(),
        );
        let config = test_config(registry.path());
        registry_with(
            &config.registry_dir,
            &target(&server.endpoint(), Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq")),
        );
        let mut queue = RecordingQueue::default();
        forward_due_reports(&mut queue, &config, None);
        assert_eq!(server.request_count(), 1);
        assert_eq!(queue.reports.len(), 1);
        let report = &queue.reports[0];
        assert_eq!(report.agent_runtime_id, "runtime-1");
        assert!(report.ready);
        assert_eq!(report.reason, None);
        assert_eq!(
            report.agent_npub.as_deref(),
            Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq")
        );
        assert_eq!(report.report_interval_seconds, Some(60));
    }

    #[test]
    fn not_ready_answer_forwards_the_guest_reason() {
        let registry = tempfile::tempdir().unwrap();
        let server = ContactServer::start(
            r#"{"ready":false,"error":"model endpoint 503","agent_npub":"npub1qqqqqqqqqqqqqqqqqqqqqqqq"}"#
                .to_string(),
        );
        let config = test_config(registry.path());
        registry_with(
            &config.registry_dir,
            &target(&server.endpoint(), Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq")),
        );
        let mut queue = RecordingQueue::default();
        forward_due_reports(&mut queue, &config, None);
        assert_eq!(queue.reports.len(), 1);
        let report = &queue.reports[0];
        assert!(!report.ready);
        assert_eq!(report.reason.as_deref(), Some("model endpoint 503"));
    }

    /// The documented transport-failure choice: nobody answering at the
    /// recorded endpoint is reported `ready: false` with reason
    /// `unreachable`, so a dead runtime reads not_ready immediately and only a
    /// silent runner degrades to the named `unknown` state.
    #[test]
    fn transport_failure_posts_an_explicit_unreachable_report() {
        let registry = tempfile::tempdir().unwrap();
        // A bound-then-dropped listener leaves a port that refuses connections.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = test_config(registry.path());
        registry_with(
            &config.registry_dir,
            &target(
                &format!("http://127.0.0.1:{port}/contact"),
                Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq"),
            ),
        );
        let mut queue = RecordingQueue::default();
        forward_due_reports(&mut queue, &config, None);
        assert_eq!(queue.reports.len(), 1);
        let report = &queue.reports[0];
        assert!(!report.ready);
        assert_eq!(
            report.reason.as_deref(),
            Some(RUNTIME_HEALTH_UNREACHABLE_REASON)
        );
        assert_eq!(
            report.agent_npub.as_deref(),
            Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq")
        );
    }

    /// The anti-squat gate: a response presenting a different principal is
    /// never attributed to this runtime — the report is dropped and the
    /// runtime goes stale instead of wearing the squatter's health.
    #[test]
    fn identity_mismatch_drops_the_report() {
        let registry = tempfile::tempdir().unwrap();
        let server = ContactServer::start(
            r#"{"ready":true,"agent_npub":"npub1squattersquattersquatter"}"#.to_string(),
        );
        let config = test_config(registry.path());
        registry_with(
            &config.registry_dir,
            &target(&server.endpoint(), Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq")),
        );
        let mut queue = RecordingQueue::default();
        forward_due_reports(&mut queue, &config, None);
        assert_eq!(server.request_count(), 1);
        assert!(queue.reports.is_empty());
    }

    #[test]
    fn unpinned_target_pins_the_first_presented_principal() {
        let registry = tempfile::tempdir().unwrap();
        let server = ContactServer::start(
            r#"{"ready":true,"agent_npub":"npub1firstseenfirstseenfirstsee"}"#.to_string(),
        );
        let config = test_config(registry.path());
        registry_with(&config.registry_dir, &target(&server.endpoint(), None));
        let mut queue = RecordingQueue::default();
        forward_due_reports(&mut queue, &config, None);
        assert_eq!(queue.reports.len(), 1);
        assert_eq!(
            queue.reports[0].agent_npub.as_deref(),
            Some("npub1firstseenfirstseenfirstsee")
        );
        let persisted: HealthReportTarget = serde_json::from_slice(
            &std::fs::read(config.registry_dir.join("runtime-1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            persisted.agent_npub.as_deref(),
            Some("npub1firstseenfirstseenfirstsee")
        );
    }

    #[test]
    fn unbound_answers_are_never_reported() {
        let registry = tempfile::tempdir().unwrap();
        let server = ContactServer::start(r#"{"ready":true}"#.to_string());
        let config = test_config(registry.path());
        registry_with(&config.registry_dir, &target(&server.endpoint(), None));
        let mut queue = RecordingQueue::default();
        forward_due_reports(&mut queue, &config, None);
        assert!(queue.reports.is_empty());
    }

    #[test]
    fn oversized_or_unparseable_bodies_are_dropped() {
        let registry = tempfile::tempdir().unwrap();
        let oversized = ContactServer::start(format!(
            r#"{{"ready":true,"agent_npub":"npub1qqqqqqqqqqqqqqqqqqqqqqqq","padding":"{}"}}"#,
            "x".repeat(MAX_CONTACT_RESPONSE_BYTES as usize)
        ));
        let config = test_config(registry.path());
        registry_with(
            &config.registry_dir,
            &target(&oversized.endpoint(), Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq")),
        );
        let junk = ContactServer::start("this is not json".to_string());
        let mut junk_target = target(&junk.endpoint(), Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq"));
        junk_target.agent_runtime_id = "runtime-2".to_string();
        registry_with(&config.registry_dir, &junk_target);
        let mut queue = RecordingQueue::default();
        forward_due_reports(&mut queue, &config, None);
        assert!(queue.reports.is_empty());
    }

    #[test]
    fn throttled_targets_are_not_polled_again_within_the_interval() {
        let registry = tempfile::tempdir().unwrap();
        let server = ContactServer::start(
            r#"{"ready":true,"agent_npub":"npub1qqqqqqqqqqqqqqqqqqqqqqqq"}"#.to_string(),
        );
        let config = test_config(registry.path());
        let mut throttled = target(&server.endpoint(), Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq"));
        throttled.last_attempt_unix_ms = Some(u64::MAX);
        registry_with(&config.registry_dir, &throttled);
        let mut queue = RecordingQueue::default();
        forward_due_reports(&mut queue, &config, None);
        assert_eq!(server.request_count(), 0);
        assert!(queue.reports.is_empty());
    }

    #[test]
    fn record_refresh_and_remove_target_manage_the_registry() {
        let registry = tempfile::tempdir().unwrap();
        let config = test_config(registry.path());
        record_target(
            &config,
            "runtime-1",
            "machine-1",
            Some("http://127.0.0.1:41001/contact"),
            Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq"),
        );
        let path = registry.path().join("runtime-1.json");
        let recorded: HealthReportTarget =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(recorded.contact_endpoint, "http://127.0.0.1:41001/contact");
        assert_eq!(
            recorded.agent_npub.as_deref(),
            Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq")
        );

        // An upgrade moves the endpoint but keeps the identity pin, and a
        // weaker caller-supplied pin never replaces the recorded one.
        assert_eq!(
            ensure_target(
                &config,
                "runtime-1",
                "machine-1",
                Some("http://127.0.0.1:41009/contact"),
                Some("npub1zzzzzzzzzzzzzzzzzzzzzzzz"),
            ),
            EnsureTargetOutcome::Refreshed
        );
        let refreshed: HealthReportTarget =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(refreshed.contact_endpoint, "http://127.0.0.1:41009/contact");
        assert_eq!(
            refreshed.agent_npub.as_deref(),
            Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq")
        );
        // Restating the same facts is a no-op write.
        assert_eq!(
            ensure_target(
                &config,
                "runtime-1",
                "machine-1",
                Some("http://127.0.0.1:41009/contact"),
                None,
            ),
            EnsureTargetOutcome::Unchanged
        );

        remove_target(&config, "runtime-1");
        assert!(!path.exists());
        // Removing an absent target is a no-op.
        remove_target(&config, "runtime-1");
    }

    /// The upgrade/relocation gap: a runtime with no registry file (upgraded
    /// in place by a pre-registry runner, or adopted by a cold relocation)
    /// gets one created, not skipped.
    #[test]
    fn ensure_target_creates_a_missing_entry_and_pins_a_later_principal() {
        let registry = tempfile::tempdir().unwrap();
        let config = test_config(registry.path());
        let path = registry.path().join("runtime-1.json");
        assert!(!path.exists());
        assert_eq!(
            ensure_target(
                &config,
                "runtime-1",
                "machine-1",
                Some("http://127.0.0.1:41009/contact"),
                None,
            ),
            EnsureTargetOutcome::Created
        );
        let created: HealthReportTarget =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(created.contact_endpoint, "http://127.0.0.1:41009/contact");
        assert_eq!(created.source_machine_id, "machine-1");
        assert_eq!(created.agent_npub, None);
        // An unpinned entry accepts a pin without moving.
        assert_eq!(
            ensure_target(
                &config,
                "runtime-1",
                "machine-1",
                Some("http://127.0.0.1:41009/contact"),
                Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq"),
            ),
            EnsureTargetOutcome::Refreshed
        );
        let pinned: HealthReportTarget =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            pinned.agent_npub.as_deref(),
            Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq")
        );
        // Without an endpoint there is nothing to poll: no file is written.
        assert_eq!(
            ensure_target(&config, "runtime-2", "machine-2", None, None),
            EnsureTargetOutcome::Skipped
        );
        assert!(!registry.path().join("runtime-2.json").exists());
    }

    fn listed(
        agent_runtime_id: &str,
        contact_endpoint: Option<&str>,
        agent_npub: Option<&str>,
    ) -> RuntimeHealthTarget {
        RuntimeHealthTarget {
            agent_runtime_id: agent_runtime_id.to_string(),
            source_machine_id: format!("machine-{agent_runtime_id}"),
            contact_endpoint: contact_endpoint.map(str::to_string),
            agent_npub: agent_npub.map(str::to_string),
            lifecycle_status: RuntimeSummaryStatus::Online,
        }
    }

    /// Startup reconcile: Core's host-scoped list creates missing entries and
    /// moves stale endpoints, keeps every existing pin, and never removes an
    /// entry Core did not list.
    #[test]
    fn reconcile_registers_missing_targets_and_never_removes_active_ones() {
        let registry = tempfile::tempdir().unwrap();
        let config = test_config(registry.path());
        // Already registered with a launch-verified pin, at an old endpoint.
        registry_with(
            &config.registry_dir,
            &HealthReportTarget {
                agent_runtime_id: "runtime-known".to_string(),
                source_machine_id: "machine-runtime-known".to_string(),
                contact_endpoint: "http://127.0.0.1:41001/contact".to_string(),
                agent_npub: Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq".to_string()),
                last_attempt_unix_ms: Some(unix_millis_now()),
            },
        );
        // Registered locally but absent from Core's list (e.g. a runtime Core
        // has not caught up on): must survive the reconcile untouched.
        registry_with(
            &config.registry_dir,
            &HealthReportTarget {
                agent_runtime_id: "runtime-local-only".to_string(),
                source_machine_id: "machine-runtime-local-only".to_string(),
                contact_endpoint: "http://127.0.0.1:41003/contact".to_string(),
                agent_npub: None,
                last_attempt_unix_ms: None,
            },
        );
        let mut queue = RecordingQueue {
            listing: Some(RuntimeHealthTargetList {
                source_host_id: "host-1".to_string(),
                targets: vec![
                    listed(
                        "runtime-known",
                        Some("http://127.0.0.1:41002/contact"),
                        Some("npub1zzzzzzzzzzzzzzzzzzzzzzzz"),
                    ),
                    listed(
                        "runtime-missing",
                        Some("http://127.0.0.1:41004/contact"),
                        Some("npub1rrrrrrrrrrrrrrrrrrrrrrrr"),
                    ),
                    listed("runtime-unpollable", None, None),
                ],
            }),
            ..RecordingQueue::default()
        };
        reconcile_targets(&mut queue, &config, Some("host-1"));

        let read = |id: &str| -> Option<HealthReportTarget> {
            std::fs::read(registry.path().join(format!("{id}.json")))
                .ok()
                .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        };
        let known = read("runtime-known").unwrap();
        assert_eq!(known.contact_endpoint, "http://127.0.0.1:41002/contact");
        assert_eq!(
            known.agent_npub.as_deref(),
            Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq"),
            "the launch-verified pin beats Core's last-observed npub"
        );
        assert_eq!(
            known.last_attempt_unix_ms, None,
            "a moved endpoint polls promptly"
        );
        let missing = read("runtime-missing").unwrap();
        assert_eq!(missing.contact_endpoint, "http://127.0.0.1:41004/contact");
        assert_eq!(missing.source_machine_id, "machine-runtime-missing");
        assert_eq!(
            missing.agent_npub.as_deref(),
            Some("npub1rrrrrrrrrrrrrrrrrrrrrrrr")
        );
        assert!(
            read("runtime-local-only").is_some(),
            "reconcile never removes"
        );
        assert!(read("runtime-unpollable").is_none());

        // A host mismatch between the credential and this runner is refused
        // rather than registering another host's runtimes here.
        let before = std::fs::read_dir(registry.path()).unwrap().count();
        reconcile_targets(&mut queue, &config, Some("host-2"));
        assert_eq!(std::fs::read_dir(registry.path()).unwrap().count(), before);

        // A Core without the listing leaves the registry alone.
        let mut older_core = RecordingQueue::default();
        reconcile_targets(&mut older_core, &config, Some("host-1"));
        assert_eq!(std::fs::read_dir(registry.path()).unwrap().count(), before);
    }

    /// Core answering 404 means the runtime is not scoped to the host the
    /// runner credential asserts. That is an operator's problem to see, not
    /// a reason to forget the runtime: the target stays, the attempt is
    /// logged, and the per-target throttle spaces the retries.
    #[test]
    fn a_core_not_found_answer_keeps_the_target_and_backs_off() {
        let registry = tempfile::tempdir().unwrap();
        let server = ContactServer::start(
            r#"{"ready":true,"agent_npub":"npub1qqqqqqqqqqqqqqqqqqqqqqqq"}"#.to_string(),
        );
        let config = test_config(registry.path());
        registry_with(
            &config.registry_dir,
            &target(&server.endpoint(), Some("npub1qqqqqqqqqqqqqqqqqqqqqqqq")),
        );
        let mut queue = RecordingQueue {
            report_status_error: Some(404),
            ..RecordingQueue::default()
        };
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert!(queue.reports.is_empty());
        assert_eq!(server.request_count(), 1);
        let kept: HealthReportTarget = serde_json::from_slice(
            &std::fs::read(config.registry_dir.join("runtime-1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(kept.contact_endpoint, server.endpoint());
        assert!(kept.last_attempt_unix_ms.is_some());
        // Within the interval the target is not retried (the throttle is the
        // back-off); Core coming back later is answered on the next cadence.
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(server.request_count(), 1);
    }
}
