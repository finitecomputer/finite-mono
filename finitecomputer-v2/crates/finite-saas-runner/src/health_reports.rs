//! Runner-ferried standing readiness (2026-08 audit synthesis, H1 slice 3).
//!
//! The runner already has a provider-uniform HTTP read path into every guest
//! on its host (`/contact`, the same bounded JSON document the launch path
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
//! - **Core names the targets.** The poll target set is Core's host-scoped
//!   listing (`GET /api/core/v1/runtime-health-targets`), fetched every
//!   cycle: every live runtime Core scopes to this runner's host, with its
//!   contact endpoint and last observed Agent Principal. The runner keeps no
//!   registry of its own — Core already holds every fact a registry would
//!   duplicate (launches, upgrades, relocations, stops, destroys). Only the
//!   per-runtime throttle lives runner-side, in memory, reset on process
//!   start. A Core that does not serve the listing (N-1) turns the ferry off
//!   for that process with one log line; a transport failure skips the cycle.
//! - **Latest report only.** Core stores one report per runtime and projects
//!   readiness at read time; staleness (no report within 3x the cadence) reads
//!   as the named `stale` state, and a runtime that was never reported on is
//!   `unknown`. There is no sweeper and no history table.
//! - **Transport failure is reported, not skipped.** When nobody answers at
//!   the listed endpoint the runner posts `ready: false` with reason
//!   `unreachable`, so a dead runtime reads `not_ready` immediately. Staleness
//!   then means exactly one thing — the runner stopped reporting (runner
//!   down, Core unreachable, or a pre-poller runner) — and reads `stale`.
//!   The two failure classes stay distinguishable in the fleet view.
//! - **Identity-pinned attribution.** Ports are reallocated across stops (the
//!   2026-08-07 port-squat class), so a response is only attributed to a
//!   runtime when it presents the Agent Principal npub Core has on record for
//!   it (the one its previous reports carried). A runtime with no principal
//!   on record reports the first presented one, which Core then holds and
//!   lists as the pin. Mismatches and unbound answers are dropped, so a
//!   squatter's health never wears this runtime's name — the runtime simply
//!   goes stale.

use finite_saas_core::{RuntimeHealthReportRequest, RuntimeHealthTarget};
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
    pub interval: Duration,
    pub http_timeout: Duration,
    /// Process-lifetime poller state. `run_cycle` rebuilds the runner every
    /// cycle, so the caller holds one state for the life of the process and
    /// hands it to each cycle's config.
    pub state: Arc<HealthReportState>,
}

/// The only runner-side state the ferry keeps: the per-runtime throttle and
/// the once-per-process warning latches. Nothing here is a fact about a
/// runtime; losing it on restart costs at most one early poll per runtime.
#[derive(Debug, Default)]
pub struct HealthReportState {
    last_attempt_unix_ms: Mutex<HashMap<String, u64>>,
    listing_unavailable_warned: AtomicBool,
    host_mismatch_warned: AtomicBool,
}

impl HealthReportState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// True once this process has logged that Core does not serve the
    /// target listing (an N-1 Core). Cleared when a listing next succeeds.
    pub fn listing_unavailable_warned(&self) -> bool {
        self.listing_unavailable_warned.load(Ordering::Relaxed)
    }

    /// Record an attempt for `agent_runtime_id` now, unless one happened
    /// less than `interval` ago. Returns whether the poll is due.
    fn claim_attempt(&self, agent_runtime_id: &str, interval: Duration, now_ms: u64) -> bool {
        let mut attempts = self
            .last_attempt_unix_ms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if attempts
            .get(agent_runtime_id)
            .is_some_and(|last| now_ms.saturating_sub(*last) < interval.as_millis() as u64)
        {
            return false;
        }
        attempts.insert(agent_runtime_id.to_string(), now_ms);
        true
    }

    /// Forget runtimes Core no longer lists, so a runtime that comes back
    /// polls promptly and the map stays bounded by the host's fleet.
    fn retain_listed(&self, targets: &[RuntimeHealthTarget]) {
        let mut attempts = self
            .last_attempt_unix_ms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        attempts.retain(|id, _| targets.iter().any(|target| target.agent_runtime_id == *id));
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
/// presents the Agent Principal Core has on record for it. Returns the npub
/// to report, or `None` to drop the report (the runtime then goes stale
/// rather than wearing a squatter's health).
fn gated_npub(
    target: &RuntimeHealthTarget,
    contact_endpoint: &str,
    observed: Option<&str>,
) -> Option<String> {
    match (target.agent_npub.as_deref(), observed) {
        (Some(pinned), Some(observed)) if pinned != observed => {
            eprintln!(
                "warning: health report identity mismatch for {}: Core holds {pinned} but \
                 {contact_endpoint} answered with {observed} — dropping report (possible port \
                 squat)",
                target.agent_runtime_id
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
                 Core has none on record; dropping report",
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

/// Fetch Core's target listing for this host, poll every due target once,
/// and forward one report per runtime to Core. Best-effort, throttled per
/// runtime: telemetry must never fail or slow the lease cycle beyond its own
/// bounded HTTP timeouts. `source_host_id` is the host this runner serves;
/// a listing scoped to another host is refused.
pub fn forward_due_reports(
    queue: &mut dyn AgentCreationQueue,
    config: &HealthReportConfig,
    source_host_id: Option<&str>,
) {
    let state = &config.state;
    let listing = match queue.list_runtime_health_targets() {
        Ok(Some(listing)) => {
            state
                .listing_unavailable_warned
                .store(false, Ordering::Relaxed);
            listing
        }
        Ok(None) => {
            if !state
                .listing_unavailable_warned
                .swap(true, Ordering::Relaxed)
            {
                eprintln!(
                    "warning: standing health reports are off: Core does not serve the \
                     runtime health target listing for this runner (older Core); reports \
                     resume when it does"
                );
            }
            return;
        }
        Err(error) => {
            eprintln!(
                "warning: health reports skipped this cycle: Core target listing failed: {error}"
            );
            return;
        }
    };
    if let Some(source_host_id) = source_host_id
        && listing.source_host_id != source_host_id
    {
        if !state.host_mismatch_warned.swap(true, Ordering::Relaxed) {
            eprintln!(
                "error: standing health reports are off: the runner credential is scoped to \
                 host {} but this runner serves {source_host_id}",
                listing.source_host_id
            );
        }
        return;
    }
    state.host_mismatch_warned.store(false, Ordering::Relaxed);
    state.retain_listed(&listing.targets);
    let now_ms = unix_millis_now();
    for target in &listing.targets {
        let Some(contact_endpoint) = target.contact_endpoint.as_deref() else {
            // Core recorded no endpoint (a pre-contact-endpoint row); there is
            // nothing to poll and Core projects it `unknown`.
            continue;
        };
        if !state.claim_attempt(&target.agent_runtime_id, config.interval, now_ms) {
            continue;
        }
        let (ready, reason, agent_npub) = match read_contact(contact_endpoint, config.http_timeout)
        {
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
                let Some(agent_npub) = gated_npub(target, contact_endpoint, observed.as_deref())
                else {
                    continue;
                };
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
            // A 404 means Core stopped scoping this runtime to this host
            // between the listing and the report (destroyed, relocated
            // away, or a credential/host drift). The next listing decides
            // whether it is still ours; nothing to keep or forget here.
            if matches!(error, RunnerError::CoreStatus { status: 404, .. }) {
                eprintln!(
                    "warning: Core does not recognise runtime {} on host {} (contact \
                     {contact_endpoint}); dropping this report: {error}",
                    target.agent_runtime_id,
                    source_host_id.unwrap_or("<unset>")
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
    use finite_saas_core::{RuntimeHealthReportAck, RuntimeHealthTargetList, RuntimeSummaryStatus};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;

    const PINNED: &str = "npub1qqqqqqqqqqqqqqqqqqqqqqqq";
    const OTHER: &str = "npub1zzzzzzzzzzzzzzzzzzzzzzzz";

    /// What the poller needs from Core: the listing, and whether posting a
    /// given runtime's report is answered with an HTTP error.
    struct RecordingQueue {
        reports: Vec<RuntimeHealthReportRequest>,
        report_status_errors: HashMap<String, u16>,
        listing: Result<Option<RuntimeHealthTargetList>, String>,
        listings_served: usize,
    }

    impl Default for RecordingQueue {
        /// An older Core: no listing route.
        fn default() -> Self {
            Self {
                reports: Vec::new(),
                report_status_errors: HashMap::new(),
                listing: Ok(None),
                listings_served: 0,
            }
        }
    }

    impl RecordingQueue {
        fn listing(targets: Vec<RuntimeHealthTarget>) -> Self {
            Self {
                listing: Ok(Some(RuntimeHealthTargetList {
                    source_host_id: "host-1".to_string(),
                    targets,
                })),
                ..Self::default()
            }
        }
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
            if let Some(status) = self.report_status_errors.get(&input.agent_runtime_id) {
                return Err(RunnerError::CoreStatus {
                    status: *status,
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
            self.listings_served += 1;
            self.listing.clone().map_err(RunnerError::CoreRequest)
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

        fn ready(npub: &str) -> Self {
            Self::start(format!(r#"{{"ready":true,"agent_npub":"{npub}"}}"#))
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

    /// A port nobody listens on: bound, then released.
    fn unreachable_endpoint() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        format!("http://127.0.0.1:{port}/contact")
    }

    fn test_config(interval: Duration) -> HealthReportConfig {
        HealthReportConfig {
            interval,
            http_timeout: Duration::from_secs(2),
            state: HealthReportState::new(),
        }
    }

    fn target(id: &str, endpoint: Option<&str>, npub: Option<&str>) -> RuntimeHealthTarget {
        RuntimeHealthTarget {
            agent_runtime_id: id.to_string(),
            source_machine_id: format!("machine-{id}"),
            contact_endpoint: endpoint.map(str::to_string),
            agent_npub: npub.map(str::to_string),
            lifecycle_status: RuntimeSummaryStatus::Online,
            report_interval_seconds: None,
        }
    }

    #[test]
    fn ready_answer_is_reported_with_the_pinned_npub() {
        let server = ContactServer::ready(PINNED);
        let config = test_config(Duration::from_secs(60));
        let mut queue = RecordingQueue::listing(vec![target(
            "runtime-1",
            Some(&server.endpoint()),
            Some(PINNED),
        )]);
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(server.request_count(), 1);
        assert_eq!(queue.reports.len(), 1);
        let report = &queue.reports[0];
        assert_eq!(report.agent_runtime_id, "runtime-1");
        assert!(report.ready);
        assert_eq!(report.reason, None);
        assert_eq!(report.agent_npub.as_deref(), Some(PINNED));
        assert_eq!(report.report_interval_seconds, Some(60));
    }

    #[test]
    fn not_ready_answer_forwards_the_guest_reason() {
        let server = ContactServer::start(format!(
            r#"{{"ready":false,"error":"  model endpoint 503  ","agent_npub":"{PINNED}"}}"#
        ));
        let config = test_config(Duration::from_secs(60));
        let mut queue = RecordingQueue::listing(vec![target(
            "runtime-1",
            Some(&server.endpoint()),
            Some(PINNED),
        )]);
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(queue.reports.len(), 1);
        assert!(!queue.reports[0].ready);
        assert_eq!(
            queue.reports[0].reason.as_deref(),
            Some("model endpoint 503")
        );
    }

    #[test]
    fn transport_failure_posts_an_explicit_unreachable_report() {
        let config = test_config(Duration::from_secs(60));
        let mut queue = RecordingQueue::listing(vec![target(
            "runtime-1",
            Some(&unreachable_endpoint()),
            Some(PINNED),
        )]);
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(queue.reports.len(), 1);
        assert!(!queue.reports[0].ready);
        assert_eq!(
            queue.reports[0].reason.as_deref(),
            Some(RUNTIME_HEALTH_UNREACHABLE_REASON)
        );
        assert_eq!(queue.reports[0].agent_npub.as_deref(), Some(PINNED));
    }

    #[test]
    fn identity_mismatch_and_unbound_answers_are_dropped() {
        let squatter = ContactServer::ready(OTHER);
        let unbound = ContactServer::start(r#"{"ready":true}"#.to_string());
        let config = test_config(Duration::from_secs(60));
        let mut queue = RecordingQueue::listing(vec![
            target("runtime-pinned", Some(&squatter.endpoint()), Some(PINNED)),
            target(
                "runtime-pinned-silent",
                Some(&unbound.endpoint()),
                Some(PINNED),
            ),
            target("runtime-unpinned-silent", Some(&unbound.endpoint()), None),
        ]);
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(squatter.request_count(), 1);
        assert_eq!(unbound.request_count(), 2);
        assert!(
            queue.reports.is_empty(),
            "no report may wear another principal's name"
        );
    }

    /// Core has no principal on record yet: the first presented one is
    /// reported, and Core then lists it as the pin for later cycles.
    #[test]
    fn unpinned_target_reports_the_first_presented_principal() {
        let server = ContactServer::ready(PINNED);
        let config = test_config(Duration::from_secs(60));
        let mut queue =
            RecordingQueue::listing(vec![target("runtime-1", Some(&server.endpoint()), None)]);
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(queue.reports.len(), 1);
        assert_eq!(queue.reports[0].agent_npub.as_deref(), Some(PINNED));
    }

    #[test]
    fn oversized_or_unparseable_bodies_are_dropped() {
        let oversized = ContactServer::start("x".repeat(MAX_CONTACT_RESPONSE_BYTES as usize + 1));
        let garbage = ContactServer::start("not json".to_string());
        let config = test_config(Duration::from_secs(60));
        let mut queue = RecordingQueue::listing(vec![
            target("runtime-big", Some(&oversized.endpoint()), Some(PINNED)),
            target("runtime-garbage", Some(&garbage.endpoint()), Some(PINNED)),
        ]);
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert!(queue.reports.is_empty());
    }

    /// The throttle is the only runner-side state, and it lives in the
    /// process: a second cycle inside the interval polls nothing.
    #[test]
    fn throttled_targets_are_not_polled_again_within_the_interval() {
        let server = ContactServer::ready(PINNED);
        let config = test_config(Duration::from_secs(60));
        let mut queue = RecordingQueue::listing(vec![target(
            "runtime-1",
            Some(&server.endpoint()),
            Some(PINNED),
        )]);
        forward_due_reports(&mut queue, &config, Some("host-1"));
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(
            queue.listings_served, 2,
            "the listing is fetched every cycle"
        );
        assert_eq!(server.request_count(), 1);
        assert_eq!(queue.reports.len(), 1);
        // A fresh process (fresh state) polls promptly.
        let restarted = test_config(Duration::from_secs(60));
        forward_due_reports(&mut queue, &restarted, Some("host-1"));
        assert_eq!(server.request_count(), 2);
    }

    /// Core's listing is the target set: exactly the listed runtimes are
    /// polled, and one that drops out of the listing is not polled next
    /// cycle even though it still answers.
    #[test]
    fn poller_polls_exactly_the_listed_targets_each_cycle() {
        let listed = ContactServer::ready(PINNED);
        let unlisted = ContactServer::ready(OTHER);
        let departing = ContactServer::ready(PINNED);
        let config = test_config(Duration::ZERO);
        let mut queue = RecordingQueue::listing(vec![
            target("runtime-listed", Some(&listed.endpoint()), Some(PINNED)),
            target(
                "runtime-departing",
                Some(&departing.endpoint()),
                Some(PINNED),
            ),
            target("runtime-no-endpoint", None, Some(PINNED)),
        ]);
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(listed.request_count(), 1);
        assert_eq!(departing.request_count(), 1);
        assert_eq!(unlisted.request_count(), 0);
        assert_eq!(
            queue
                .reports
                .iter()
                .map(|report| report.agent_runtime_id.as_str())
                .collect::<Vec<_>>(),
            vec!["runtime-listed", "runtime-departing"]
        );

        // Core stops listing the departing runtime (stopped, destroyed, or
        // relocated away): it is not polled again, and nothing runner-side
        // remembers it.
        queue.listing = Ok(Some(RuntimeHealthTargetList {
            source_host_id: "host-1".to_string(),
            targets: vec![target(
                "runtime-listed",
                Some(&listed.endpoint()),
                Some(PINNED),
            )],
        }));
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(listed.request_count(), 2);
        assert_eq!(departing.request_count(), 1);
        assert_eq!(
            config
                .state
                .last_attempt_unix_ms
                .lock()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            vec!["runtime-listed"]
        );
    }

    /// An N-1 Core has no listing: nothing is polled or reported, and the
    /// condition is logged once per process, not once per cycle.
    #[test]
    fn older_core_without_a_listing_reports_nothing_and_warns_once() {
        let server = ContactServer::ready(PINNED);
        let config = test_config(Duration::ZERO);
        let mut queue = RecordingQueue::default();
        assert!(!config.state.listing_unavailable_warned());
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert!(
            config.state.listing_unavailable_warned(),
            "first cycle logs"
        );
        // The latch stays set across cycles: no second log line.
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert!(config.state.listing_unavailable_warned());
        assert_eq!(queue.listings_served, 2);
        assert_eq!(server.request_count(), 0);
        assert!(queue.reports.is_empty());
        // Core gaining the route resumes reporting and re-arms the warning.
        queue.listing = Ok(Some(RuntimeHealthTargetList {
            source_host_id: "host-1".to_string(),
            targets: vec![target("runtime-1", Some(&server.endpoint()), Some(PINNED))],
        }));
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert!(!config.state.listing_unavailable_warned());
        assert_eq!(queue.reports.len(), 1);
    }

    #[test]
    fn listing_transport_error_or_host_mismatch_skips_the_cycle() {
        let server = ContactServer::ready(PINNED);
        let config = test_config(Duration::ZERO);
        let mut queue = RecordingQueue {
            listing: Err("connection refused".to_string()),
            ..RecordingQueue::default()
        };
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(server.request_count(), 0);
        assert!(
            !config.state.listing_unavailable_warned(),
            "a transport error is not the N-1 condition"
        );
        // A listing scoped to another host is refused rather than polled.
        let mut queue = RecordingQueue::listing(vec![target(
            "runtime-1",
            Some(&server.endpoint()),
            Some(PINNED),
        )]);
        forward_due_reports(&mut queue, &config, Some("host-2"));
        assert_eq!(server.request_count(), 0);
        assert!(queue.reports.is_empty());
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(server.request_count(), 1);
        assert_eq!(queue.reports.len(), 1);
    }

    /// Core answering 404 for one runtime's report (it left this host between
    /// the listing and the report) is logged and ignored; the other targets
    /// are still reported, and the next listing decides what is ours.
    #[test]
    fn report_not_found_does_not_stop_other_reports() {
        let gone = ContactServer::ready(PINNED);
        let kept = ContactServer::ready(PINNED);
        let config = test_config(Duration::ZERO);
        let mut queue = RecordingQueue::listing(vec![
            target("runtime-gone", Some(&gone.endpoint()), Some(PINNED)),
            target("runtime-kept", Some(&kept.endpoint()), Some(PINNED)),
        ]);
        queue
            .report_status_errors
            .insert("runtime-gone".to_string(), 404);
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(gone.request_count(), 1);
        assert_eq!(kept.request_count(), 1);
        assert_eq!(queue.reports.len(), 1);
        assert_eq!(queue.reports[0].agent_runtime_id, "runtime-kept");
        // Nothing runner-side changes on a 404: the same listing polls both
        // again next cycle.
        forward_due_reports(&mut queue, &config, Some("host-1"));
        assert_eq!(gone.request_count(), 2);
        assert_eq!(kept.request_count(), 2);
    }
}
