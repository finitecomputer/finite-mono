//! Relocation completion under interruption, run against a stateful model of
//! Core and of the target compute. The model mirrors Core's rules for the
//! relocation row: completion and failure need the exact lease on a launching
//! request, and a cancelled relocation keeps its lease until the target Runner
//! releases it with a failure record. Every step asserts the single-copy
//! invariant: target compute never runs while Core admits source controls.
use super::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Absent,
    Running,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Completion {
    Commits,
    CommitsResponseLost,
    RollsBackResponseLost,
    Rejected,
}

#[derive(Debug)]
struct World {
    status: AgentCreationRequestStatus,
    lease: Option<String>,
    lease_live: bool,
    target: Target,
    /// Compute the provider lost track of that still writes the target tree.
    /// Removing or stopping the named container does not end it.
    orphan_running: bool,
    /// Another copy of the Agent runs: the restarted source, or the Agent
    /// after a newer relocation.
    source_running: bool,
    completion: Completion,
    drop_failure_response: bool,
    failure_record_lost: bool,
    cancel_on_boot: bool,
    stop_failures: usize,
    remove_fails: bool,
    /// Another worker acts while this one is paused after a refused
    /// failure record.
    on_refused_failure: Option<fn(&mut World)>,
    calls: Vec<&'static str>,
}

type SharedWorld = Rc<RefCell<World>>;

impl World {
    fn new(completion: Completion) -> SharedWorld {
        Rc::new(RefCell::new(World {
            status: AgentCreationRequestStatus::Requested,
            lease: None,
            lease_live: false,
            target: Target::Absent,
            orphan_running: false,
            source_running: false,
            completion,
            drop_failure_response: false,
            failure_record_lost: false,
            cancel_on_boot: false,
            stop_failures: 0,
            remove_fails: false,
            on_refused_failure: None,
            calls: Vec::new(),
        }))
    }

    fn target_running(&self) -> bool {
        self.target == Target::Running || self.orphan_running
    }

    /// Core admits controls against the old source binding once the
    /// relocation failed, or was cancelled and its target Runner released it.
    /// A running relocation moved the binding to the target instead.
    fn source_controls_admitted(&self) -> bool {
        match self.status {
            AgentCreationRequestStatus::Failed => true,
            AgentCreationRequestStatus::Cancelled => self.lease.is_none(),
            _ => false,
        }
    }

    fn record(&mut self, call: &'static str) {
        self.calls.push(call);
        assert!(
            !(self.target_running() && self.source_controls_admitted()),
            "after {call}: target compute runs while Core admits source controls: {self:?}"
        );
        assert!(
            !(self.target_running() && self.source_running),
            "after {call}: two copies of the Agent run: {self:?}"
        );
    }

    fn holds(&self, lease_token: &str) -> bool {
        self.lease.as_deref() == Some(lease_token)
    }

    /// An owner restart of the source, admitted only when Core admits it.
    fn owner_restart(&mut self) -> bool {
        if self.source_controls_admitted() {
            self.source_running = true;
        }
        self.record("owner restart");
        self.source_running
    }

    /// Real Core keeps the expired token until a new lease replaces it.
    fn expire_lease(&mut self) {
        assert_eq!(self.status, AgentCreationRequestStatus::Launching);
        self.lease_live = false;
    }

    /// The attested operator release of a cancelled relocation's lease,
    /// after the operator has stopped every trace of target compute.
    fn operator_release(&mut self) {
        assert_eq!(self.status, AgentCreationRequestStatus::Cancelled);
        assert!(self.lease.is_some() && !self.target_running());
        self.lease = None;
        self.record("operator release");
    }

    fn position(&self, call: &str) -> Option<usize> {
        self.calls.iter().position(|recorded| *recorded == call)
    }

    /// Both calls happened, the first one earlier.
    fn before(&self, first: &str, second: &str) -> bool {
        matches!(
            (self.position(first), self.position(second)),
            (Some(first), Some(second)) if first < second
        )
    }

    fn called_after(&self, earlier: &str, call: &str) -> bool {
        let Some(start) = self.position(earlier) else {
            return false;
        };
        self.calls[start..].contains(&call)
    }
}

fn not_launching() -> RunnerError {
    RunnerError::CoreStatus {
        status: 409,
        body: r#"{"error":"agent creation request is not launching"}"#.to_string(),
    }
}

fn response_lost() -> RunnerError {
    RunnerError::CoreRequest("connection reset by peer".to_string())
}

#[derive(Debug)]
struct ModelQueue {
    world: SharedWorld,
    lease: AgentCreationLease,
}

impl ModelQueue {
    fn request(&self) -> AgentCreationRequest {
        let mut request = self.lease.request.clone();
        request.status = self.world.borrow().status;
        request
    }
}

impl AgentCreationQueue for ModelQueue {
    fn lease_runtime_control(
        &mut self,
        _runner_id: &str,
        _lease_token: &str,
        _lease_seconds: i64,
        _source_host_id: Option<&str>,
        _runner_capacity: Option<&RunnerLeaseCapacity>,
    ) -> Result<Option<RuntimeControlLease>, RunnerError> {
        Ok(None)
    }

    fn complete_runtime_control(
        &mut self,
        _request_id: &str,
        _input: CompleteRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        unreachable!("the relocation model has no runtime controls")
    }

    fn fail_runtime_control(
        &mut self,
        _request_id: &str,
        _input: FailRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        unreachable!("the relocation model has no runtime controls")
    }

    fn renew_runtime_control(
        &mut self,
        _request_id: &str,
        _input: RenewRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        unreachable!("the relocation model has no runtime controls")
    }

    fn retry_runtime_control(
        &mut self,
        _request_id: &str,
        _input: RetryRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        unreachable!("the relocation model has no runtime controls")
    }

    fn lease_agent_creation(
        &mut self,
        _runner_id: &str,
        lease_token: &str,
        _lease_seconds: i64,
        _runner_capacity: Option<&RunnerLeaseCapacity>,
    ) -> Result<Option<AgentCreationLease>, RunnerError> {
        let mut world = self.world.borrow_mut();
        let leasable = world.status == AgentCreationRequestStatus::Requested
            || (world.status == AgentCreationRequestStatus::Launching && !world.lease_live);
        if !leasable {
            return Ok(None);
        }
        world.status = AgentCreationRequestStatus::Launching;
        world.lease = Some(lease_token.to_string());
        world.lease_live = true;
        world.record("lease");
        let mut lease = self.lease.clone();
        lease.request.status = AgentCreationRequestStatus::Launching;
        lease.request.lease_token = Some(lease_token.to_string());
        Ok(Some(lease))
    }

    fn complete_agent_creation(
        &mut self,
        _request_id: &str,
        input: CompleteAgentCreationRequestInput,
    ) -> Result<AgentCreationLease, RunnerError> {
        let mut world = self.world.borrow_mut();
        let accepted = world.status == AgentCreationRequestStatus::Launching
            && world.lease_live
            && world.holds(&input.lease_token);
        let completion = world.completion;
        let commits = accepted
            && matches!(
                completion,
                Completion::Commits | Completion::CommitsResponseLost
            );
        if commits {
            world.status = AgentCreationRequestStatus::Running;
            world.lease = None;
        }
        world.record("complete");
        match completion {
            Completion::CommitsResponseLost | Completion::RollsBackResponseLost => {
                Err(response_lost())
            }
            _ if commits => {
                let mut lease = self.lease.clone();
                lease.request.status = AgentCreationRequestStatus::Running;
                Ok(lease)
            }
            _ => Err(not_launching()),
        }
    }

    fn register_agent_creation_runtime(
        &mut self,
        _request_id: &str,
        input: RegisterAgentCreationRuntimeInput,
    ) -> Result<AgentCreationLease, RunnerError> {
        let mut world = self.world.borrow_mut();
        world.record("register");
        if world.status == AgentCreationRequestStatus::Launching && world.holds(&input.lease_token)
        {
            Ok(self.lease.clone())
        } else {
            Err(not_launching())
        }
    }

    fn record_provider_operation_transition(
        &mut self,
        _request_id: &str,
        _input: RecordProviderOperationTransitionRequest,
    ) -> Result<ProviderOperationEnvelope, RunnerError> {
        unreachable!("Kata relocations have no provider operation")
    }

    fn provision_finite_private_runtime_key(
        &mut self,
        _request_id: &str,
        _input: ProvisionFinitePrivateRuntimeKeyInput,
    ) -> Result<ProvisionFinitePrivateRuntimeKeyResult, RunnerError> {
        Ok(sample_finite_private_key())
    }

    fn fail_agent_creation(
        &mut self,
        _request_id: &str,
        input: FailAgentCreationRequestInput,
    ) -> Result<AgentCreationRequest, RunnerError> {
        let accepted = {
            let mut world = self.world.borrow_mut();
            if world.failure_record_lost {
                // The Runner crashed or the request never reached Core.
                world.record("failure record lost");
                return Err(response_lost());
            }
            let accepted = world.holds(&input.lease_token)
                && matches!(
                    world.status,
                    AgentCreationRequestStatus::Launching | AgentCreationRequestStatus::Cancelled
                );
            if accepted {
                if world.status == AgentCreationRequestStatus::Launching {
                    world.status = AgentCreationRequestStatus::Failed;
                }
                world.lease = None;
            }
            world.record("fail");
            if !accepted && let Some(act) = world.on_refused_failure.take() {
                act(&mut world);
            }
            if world.drop_failure_response {
                return Err(response_lost());
            }
            accepted
        };
        if accepted {
            Ok(self.request())
        } else {
            Err(not_launching())
        }
    }

    fn report_runtime_health(
        &mut self,
        _input: RuntimeHealthReportRequest,
    ) -> Result<RuntimeHealthReportAck, RunnerError> {
        unreachable!("health reporting is not configured in the relocation model")
    }
}

#[derive(Debug)]
struct ModelLauncher {
    world: SharedWorld,
}

impl RuntimeLauncher for ModelLauncher {
    fn validate_ready(&self) -> Result<(), RunnerError> {
        Ok(())
    }

    fn runtime_capabilities(&self) -> RuntimeCapabilitiesEnvelope {
        kata_runtime_capabilities_with_retirement(false)
    }

    fn runner_class(&self) -> RunnerClass {
        RunnerClass::Kata
    }

    fn runner_capacity(&self) -> RunnerLeaseCapacity {
        RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Kata],
            ..RunnerLeaseCapacity::default()
        }
    }

    fn source_host_id(&self) -> Option<&str> {
        Some("new-host")
    }

    fn launch(
        &mut self,
        _lease: &AgentCreationLease,
        _options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        let mut world = self.world.borrow_mut();
        if world.target != Target::Absent {
            world.record("launch refused");
            return Err(RunnerError::RuntimeLaunch(
                "cold relocation requires staged state with identity and no target compute"
                    .to_string(),
            ));
        }
        world.target = Target::Running;
        if world.cancel_on_boot {
            // An operator cancels while the target boots. Core keeps the
            // target Runner's lease, so source controls stay refused.
            world.status = AgentCreationRequestStatus::Cancelled;
        }
        world.record("launch");
        Ok(RuntimeLaunchFacts::sample())
    }

    fn cleanup_failed_launch(&mut self, _facts: &RuntimeLaunchFacts) -> Result<(), RunnerError> {
        let mut world = self.world.borrow_mut();
        if world.remove_fails {
            world.record("remove failed");
            return Err(RunnerError::RuntimeLaunch("rm failed".to_string()));
        }
        world.target = Target::Absent;
        world.record("remove");
        Ok(())
    }

    fn stop_relocation_target(&mut self, _lease: &AgentCreationLease) -> Result<(), RunnerError> {
        let mut world = self.world.borrow_mut();
        if world.stop_failures > 0 {
            world.stop_failures -= 1;
            world.record("stop failed");
            return Err(RunnerError::RuntimeLaunch("stop timed out".to_string()));
        }
        if world.target == Target::Running {
            world.target = Target::Stopped;
        }
        // The adapter proves shutdown from the durable tree; its Kata test
        // shows a live orphan writer makes that proof fail.
        if world.orphan_running {
            world.record("shutdown unproved");
            return Err(RunnerError::RuntimeLaunch(
                "relocation target shutdown is unproved".to_string(),
            ));
        }
        world.record("stop");
        Ok(())
    }
}

/// One Runner process: the Runner keeps no memory between runs.
fn run_cycle(world: &SharedWorld, lease_token: &str) -> serde_json::Value {
    let mut runner = AgentCreationRunner::new(
        ModelQueue {
            world: world.clone(),
            lease: sample_relocation_lease("agent_request_model"),
        },
        ModelLauncher {
            world: world.clone(),
        },
        FixedLeaseTokens::new([lease_token]),
        "runner-1",
        300,
    )
    .unwrap();
    match runner.run_once() {
        Ok(outcome) => serde_json::to_value(outcome).unwrap(),
        Err(error) => serde_json::json!({ "status": "cycle_error", "error": error.to_string() }),
    }
}

fn later_typed_restart_then_stop(world: &mut World) {
    world.target = Target::Running;
    world.record("typed restart");
    world.target = Target::Stopped;
    world.record("typed stop");
}

fn newer_relocation_elsewhere(world: &mut World) {
    world.source_running = true;
    world.record("newer relocation");
}

#[test]
fn lost_failure_record_leaves_the_target_stopped_before_the_source_restarts() {
    let world = World::new(Completion::RollsBackResponseLost);
    world.borrow_mut().drop_failure_response = true;
    let outcome = run_cycle(&world, "lease-1");
    assert_eq!(outcome["status"], "completion_unconfirmed", "{outcome}");
    let mut world = world.borrow_mut();
    // Core committed the failure record; only its response was lost.
    assert_eq!(world.status, AgentCreationRequestStatus::Failed);
    assert_eq!(world.target, Target::Stopped);
    assert!(world.before("stop", "fail"));
    assert!(world.owner_restart(), "Core admits the source restart");
}

#[test]
fn target_is_stopped_or_removed_while_source_controls_stay_refused() {
    // A clear rejection keeps the old order: remove, then record failure.
    let world = World::new(Completion::Rejected);
    let outcome = run_cycle(&world, "lease-1");
    assert_eq!(outcome["status"], "launch_failed", "{outcome}");
    let world = world.borrow();
    assert!(world.before("remove", "fail"));
    assert_eq!(
        (world.status, world.target),
        (AgentCreationRequestStatus::Failed, Target::Absent)
    );
    drop(world);

    // When removal fails, the target is stopped before failure is recorded.
    let world = World::new(Completion::Rejected);
    world.borrow_mut().remove_fails = true;
    let outcome = run_cycle(&world, "lease-1");
    assert_eq!(outcome["status"], "launch_failed", "{outcome}");
    let world = world.borrow();
    assert!(world.before("stop", "fail"));
    assert_eq!(
        (world.status, world.target),
        (AgentCreationRequestStatus::Failed, Target::Stopped)
    );
    drop(world);

    // An ambiguous result stops the target first and removes it only after
    // Core accepted the failure record.
    let world = World::new(Completion::RollsBackResponseLost);
    let outcome = run_cycle(&world, "lease-1");
    assert_eq!(outcome["status"], "launch_failed", "{outcome}");
    let world = world.borrow();
    assert!(world.before("stop", "fail"));
    assert!(world.before("fail", "remove"));
    assert_eq!(
        (world.status, world.target),
        (AgentCreationRequestStatus::Failed, Target::Absent)
    );
}

#[test]
fn unproved_shutdown_sends_no_failure_record_and_the_source_stays_refused() {
    // Removing or stopping the named container leaves an orphan writing the
    // tree. No failure record may follow, whichever path led here.
    for completion in [Completion::Rejected, Completion::RollsBackResponseLost] {
        let world = World::new(completion);
        world.borrow_mut().orphan_running = true;
        let outcome = run_cycle(&world, "lease-1");
        assert_eq!(
            outcome["status"], "relocation_target_shutdown_unproved",
            "{completion:?}: {outcome}"
        );
        let mut world = world.borrow_mut();
        assert_eq!(world.position("fail"), None, "{completion:?}");
        assert_eq!(world.status, AgentCreationRequestStatus::Launching);
        assert!(
            !world.owner_restart(),
            "{completion:?}: the source stays refused"
        );
    }
}

#[test]
fn failed_stop_of_a_launching_relocation_is_retried_by_the_next_run() {
    let world = World::new(Completion::RollsBackResponseLost);
    world.borrow_mut().stop_failures = 1;
    let outcome = run_cycle(&world, "lease-1");
    assert_eq!(
        outcome["status"], "relocation_target_shutdown_unproved",
        "{outcome}"
    );
    {
        let mut world = world.borrow_mut();
        assert_eq!(
            world.position("fail"),
            None,
            "no failure record without a stopped target"
        );
        assert_eq!(world.status, AgentCreationRequestStatus::Launching);
        assert_eq!(world.target, Target::Running);
        assert!(
            !world.owner_restart(),
            "a launching relocation refuses the source restart"
        );
        world.expire_lease();
        world.calls.clear();
    }

    // Only a launching request is leased again. The next Runner process finds
    // the earlier target and stops it before recording failure.
    let outcome = run_cycle(&world, "lease-2");
    assert_eq!(outcome["status"], "launch_failed", "{outcome}");
    let mut world = world.borrow_mut();
    assert!(world.before("launch refused", "stop"));
    assert!(world.before("stop", "fail"));
    assert_eq!(
        (world.status, world.target),
        (AgentCreationRequestStatus::Failed, Target::Stopped)
    );
    assert!(world.owner_restart());
}

#[test]
fn committed_completion_with_a_lost_response_leaves_the_target_stopped_for_a_typed_restart() {
    let world = World::new(Completion::CommitsResponseLost);
    let outcome = run_cycle(&world, "lease-1");
    assert_eq!(outcome["status"], "completion_unconfirmed", "{outcome}");
    let world = world.borrow();
    assert!(world.before("stop", "fail"));
    assert_eq!(
        world.position("start"),
        None,
        "the Runner never starts compute"
    );
    assert_eq!(
        (world.status, world.target),
        (AgentCreationRequestStatus::Running, Target::Stopped)
    );
}

#[test]
fn delayed_worker_starts_nothing_after_a_later_stop_or_a_newer_relocation() {
    for (later, expected) in [
        (
            later_typed_restart_then_stop as fn(&mut World),
            "typed stop",
        ),
        (newer_relocation_elsewhere, "newer relocation"),
    ] {
        let world = World::new(Completion::CommitsResponseLost);
        world.borrow_mut().on_refused_failure = Some(later);
        let outcome = run_cycle(&world, "lease-1");
        assert_eq!(
            outcome["status"], "completion_unconfirmed",
            "{expected}: {outcome}"
        );
        let world = world.borrow();
        assert!(world.position(expected).is_some());
        assert!(
            !world.called_after(expected, "start"),
            "{expected}: {:?}",
            world.calls
        );
        assert!(
            !world.called_after(expected, "remove"),
            "{expected}: {:?}",
            world.calls
        );
        assert_eq!(world.target, Target::Stopped, "{expected}");
    }
}

#[test]
fn cancel_of_a_booted_relocation_releases_only_after_the_target_is_down() {
    for remove_fails in [false, true] {
        let world = World::new(Completion::Commits);
        world.borrow_mut().cancel_on_boot = true;
        world.borrow_mut().remove_fails = remove_fails;
        let outcome = run_cycle(&world, "lease-1");
        assert_eq!(outcome["status"], "launch_failed", "{outcome}");
        let mut world = world.borrow_mut();
        // The Runner learned of the cancel when registration was refused.
        assert!(world.before("register", "fail"));
        assert_eq!(world.status, AgentCreationRequestStatus::Cancelled);
        assert_eq!(world.lease, None, "the failure record released the lease");
        let expected = if remove_fails {
            Target::Stopped
        } else {
            Target::Absent
        };
        assert_eq!(world.target, expected);
        assert!(world.owner_restart());
    }
}

#[test]
fn cancelled_relocation_without_a_release_waits_for_an_operator() {
    // Shutdown unproved after a cancel, or the failure record never arrived.
    for (orphan_running, failure_record_lost) in [(true, false), (false, true)] {
        let world = World::new(Completion::Commits);
        {
            let mut world = world.borrow_mut();
            world.cancel_on_boot = true;
            world.orphan_running = orphan_running;
            world.failure_record_lost = failure_record_lost;
        }
        run_cycle(&world, "lease-1");
        {
            let mut world = world.borrow_mut();
            assert_eq!(world.status, AgentCreationRequestStatus::Cancelled);
            assert!(world.lease.is_some(), "the barrier holds");
            assert!(!world.owner_restart());
        }
        // A cancelled request is never leased again: no automatic retry.
        let outcome = run_cycle(&world, "lease-2");
        assert_eq!(outcome["status"], "idle", "{outcome}");
        let mut world = world.borrow_mut();
        assert!(!world.owner_restart());
        // The operator stops every trace of target compute, then releases.
        world.orphan_running = false;
        world.operator_release();
        assert!(world.owner_restart());
    }
}
