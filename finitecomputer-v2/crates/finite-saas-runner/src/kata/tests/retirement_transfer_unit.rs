//! A retirement whose Borg upload outlasts a run completes across runs,
//! while the host's other control work keeps running. In production systemd
//! ends each run after an hour (`RuntimeMaxSec=1h`) and an upload of a 2 GB
//! home at 113 KB/s takes about five; here a fake upload outlasts a short
//! budget that every run must return within.
use super::*;
use crate::held_control::HeldControlStore;
use crate::{AgentCreationQueue, AgentCreationRunner, RandomLeaseTokenSource, RunOnceOutcome};
use finite_saas_core::{
    AgentCreationRequest, CompleteAgentCreationRequestInput, CompleteRuntimeControlRequestInput,
    FailAgentCreationRequestInput, FailRuntimeControlRequestInput, ProviderOperationEnvelope,
    ProvisionFinitePrivateRuntimeKeyInput, ProvisionFinitePrivateRuntimeKeyResult,
    RegisterAgentCreationRuntimeInput, RenewRuntimeControlRequestInput,
    RetryRuntimeControlRequestInput, RunnerLeaseCapacity, RuntimeControlRequest,
    RuntimeHealthReportAck, RuntimeHealthReportRequest,
    api::RecordProviderOperationTransitionRequest,
};

/// Generous for a run that does a whole small retirement on a loaded host
/// (about 3 s here at load 28); the fake upload outlasts it.
const RUN_BUDGET: Duration = Duration::from_secs(10);
const OTHER_CONTAINER: &str = "finite-kata-other-agent";
const OTHER_RUNTIME: &str = "runtime-2";
const CREATED_CONTAINER: &str = "finite-kata-created-meanwhile";

/// Core's runtime control queue, as its lease SQL behaves: the oldest
/// request nobody holds whose kind the Runner offers is leased; renew,
/// retry and complete need the current holder's token (409 otherwise).
#[derive(Default)]
struct SimCore {
    requests: Vec<(RuntimeControlLease, Option<String>, bool)>,
    leased: Vec<(String, String, i64)>,
    offered: Vec<RunnerLeaseCapacity>,
    renewed: Vec<(String, String)>,
    completed: Vec<CompleteRuntimeControlRequestInput>,
    retried: Vec<RetryRuntimeControlRequestInput>,
    failed: Vec<FailRuntimeControlRequestInput>,
}

impl SimCore {
    fn push(&mut self, lease: RuntimeControlLease) {
        self.requests.push((lease, None, false));
    }

    fn holder(&self, request_id: &str) -> Option<String> {
        self.requests
            .iter()
            .find(|(lease, _, _)| lease.request.id == request_id)
            .and_then(|(_, holder, _)| holder.clone())
    }

    /// The lease lapsed: Core leases the request again when it is due.
    fn lapse(&mut self, request_id: &str) {
        for (lease, holder, _) in &mut self.requests {
            if lease.request.id == request_id {
                *holder = None;
            }
        }
    }

    fn held_by(
        &mut self,
        request_id: &str,
        token: &str,
    ) -> Result<&mut (RuntimeControlLease, Option<String>, bool), RunnerError> {
        self.requests
            .iter_mut()
            .find(|(lease, holder, done)| {
                lease.request.id == request_id && !done && holder.as_deref() == Some(token)
            })
            .ok_or_else(|| RunnerError::CoreStatus {
                status: 409,
                body: "runtime control request lease conflict".to_string(),
            })
    }
}

impl AgentCreationQueue for SimCore {
    fn lease_runtime_control(
        &mut self,
        runner_id: &str,
        lease_token: &str,
        lease_seconds: i64,
        _source_host_id: Option<&str>,
        runner_capacity: Option<&RunnerLeaseCapacity>,
    ) -> Result<Option<RuntimeControlLease>, RunnerError> {
        let capacity = runner_capacity.cloned().unwrap_or_default();
        self.offered.push(capacity.clone());
        let Some((lease, holder, _)) = self.requests.iter_mut().find(|(lease, holder, done)| {
            !done && holder.is_none() && capacity.supports_runtime_control(lease.request.kind)
        }) else {
            return Ok(None);
        };
        *holder = Some(lease_token.to_string());
        let mut leased = lease.clone();
        leased.request.runner_id = Some(runner_id.to_string());
        leased.request.lease_token = Some(lease_token.to_string());
        self.leased.push((
            leased.request.id.clone(),
            lease_token.to_string(),
            lease_seconds,
        ));
        Ok(Some(leased))
    }

    fn complete_runtime_control(
        &mut self,
        request_id: &str,
        input: CompleteRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        let (lease, _, done) = self.held_by(request_id, &input.lease_token)?;
        *done = true;
        let request = lease.request.clone();
        self.completed.push(input);
        Ok(request)
    }

    fn fail_runtime_control(
        &mut self,
        request_id: &str,
        input: FailRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        let (lease, _, done) = self.held_by(request_id, &input.lease_token)?;
        *done = true;
        let request = lease.request.clone();
        self.failed.push(input);
        Ok(request)
    }

    fn renew_runtime_control(
        &mut self,
        request_id: &str,
        input: RenewRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        let request = self
            .held_by(request_id, &input.lease_token)?
            .0
            .request
            .clone();
        self.renewed
            .push((request_id.to_string(), input.lease_token));
        Ok(request)
    }

    fn retry_runtime_control(
        &mut self,
        request_id: &str,
        input: RetryRuntimeControlRequestInput,
    ) -> Result<RuntimeControlRequest, RunnerError> {
        let (lease, holder, _) = self.held_by(request_id, &input.lease_token)?;
        *holder = None;
        let request = lease.request.clone();
        self.retried.push(input);
        Ok(request)
    }

    fn lease_agent_creation(
        &mut self,
        _runner_id: &str,
        _lease_token: &str,
        _lease_seconds: i64,
        _runner_capacity: Option<&RunnerLeaseCapacity>,
    ) -> Result<Option<AgentCreationLease>, RunnerError> {
        Ok(None)
    }

    fn complete_agent_creation(
        &mut self,
        _request_id: &str,
        _input: CompleteAgentCreationRequestInput,
    ) -> Result<AgentCreationLease, RunnerError> {
        unreachable!("no creation is leased")
    }

    fn register_agent_creation_runtime(
        &mut self,
        _request_id: &str,
        _input: RegisterAgentCreationRuntimeInput,
    ) -> Result<AgentCreationLease, RunnerError> {
        unreachable!("no creation is leased")
    }

    fn record_provider_operation_transition(
        &mut self,
        _request_id: &str,
        _input: RecordProviderOperationTransitionRequest,
    ) -> Result<ProviderOperationEnvelope, RunnerError> {
        unreachable!("no creation is leased")
    }

    fn provision_finite_private_runtime_key(
        &mut self,
        _request_id: &str,
        _input: ProvisionFinitePrivateRuntimeKeyInput,
    ) -> Result<ProvisionFinitePrivateRuntimeKeyResult, RunnerError> {
        unreachable!("no creation is leased")
    }

    fn fail_agent_creation(
        &mut self,
        _request_id: &str,
        _input: FailAgentCreationRequestInput,
    ) -> Result<AgentCreationRequest, RunnerError> {
        unreachable!("no creation is leased")
    }

    fn report_runtime_health(
        &mut self,
        _input: RuntimeHealthReportRequest,
    ) -> Result<RuntimeHealthReportAck, RunnerError> {
        unreachable!("health reports are off")
    }
}

type Runner = AgentCreationRunner<SimCore, KataLauncher, RandomLeaseTokenSource>;

struct Host {
    temp: tempfile::TempDir,
    plan: KataLaunchPlan,
    fake_state: PathBuf,
    retire: RuntimeControlLease,
    runner: Runner,
}

const IMAGE: &str = "registry.example/runtime@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// A host whose stopped canonical runtime is being retired, beside another
/// agent that is running, with Core holding the retirement request.
fn host(port: u16) -> Host {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, fake_state) = test_launcher(&temp, port);
    configure_test_retirement(&mut launcher, temp.path());
    write_fake_container(
        &fake_state,
        &plan.container_name,
        "runtime:v1",
        "artifact-v1",
        "",
        port,
        &plan.state_root,
    );
    std::fs::write(
        fake_state.join(format!("{}.status", plan.container_name)),
        "exited",
    )
    .unwrap();
    std::fs::create_dir_all(plan.state_root.join("workspace")).unwrap();
    std::fs::write(plan.state_root.join("workspace/note"), "preserved").unwrap();

    let other = kata_launch_plan_for_source_machine(
        &launcher.config,
        OTHER_CONTAINER,
        Some(OTHER_RUNTIME),
        launcher.config.container_port,
    )
    .unwrap();
    std::fs::create_dir_all(&other.state_root).unwrap();
    std::fs::create_dir_all(&other.metadata_root).unwrap();
    write_fake_container(
        &fake_state,
        OTHER_CONTAINER,
        "runtime:v1",
        "artifact-v1",
        "",
        port + 1,
        &other.state_root,
    );
    std::fs::write(
        fake_state.join(format!("{OTHER_CONTAINER}.source")),
        OTHER_CONTAINER,
    )
    .unwrap();

    let mut retire = recovery_lease("runtime_ctl_retire_slow_upload", IMAGE, port);
    retire.request.kind = RuntimeControlKind::Destroy;
    retire.request.created_at = "2026-11-18T16:00:00Z".to_string();
    let RuntimeSpecEnvelope::V1(spec) = retire.runtime_spec.as_mut().unwrap();
    spec.boot_intent = RuntimeBootIntent::Normal;
    let mut core = SimCore::default();
    core.push(retire.clone());
    let runner =
        AgentCreationRunner::new(core, launcher, RandomLeaseTokenSource, "runner-1", 300).unwrap();
    Host {
        temp,
        plan,
        fake_state,
        retire,
        runner,
    }
}

/// Another agent's stop, requested while the retirement runs.
fn stop_of_the_other_agent(port: u16) -> RuntimeControlLease {
    let mut stop = recovery_lease("runtime_ctl_stop_other_agent", IMAGE, port);
    stop.request.kind = RuntimeControlKind::Stop;
    stop.request.created_at = "2026-11-18T16:05:00Z".to_string();
    stop.request.agent_runtime_id = OTHER_RUNTIME.to_string();
    stop.request.source_machine_id = OTHER_CONTAINER.to_string();
    stop.runtime.id = OTHER_RUNTIME.to_string();
    stop.runtime.source_machine_id = OTHER_CONTAINER.to_string();
    let RuntimeSpecEnvelope::V1(spec) = stop.runtime_spec.as_mut().unwrap();
    spec.agent_runtime_id = OTHER_RUNTIME.to_string();
    spec.durable_state_id = OTHER_RUNTIME.to_string();
    spec.boot_intent = RuntimeBootIntent::Normal;
    stop
}

/// One run, which must return within its budget as systemd requires.
fn run(runner: &mut Runner) -> RunOnceOutcome {
    let started = Instant::now();
    let outcome = runner.run_once().unwrap();
    let took = started.elapsed();
    assert!(
        took < RUN_BUDGET,
        "a run took {took:?}, past its {RUN_BUDGET:?} budget, and returned {outcome:?}"
    );
    outcome
}

fn run_until_settled(runner: &mut Runner, request_id: &str) -> RunOnceOutcome {
    for _ in 0..400 {
        match run(runner) {
            RunOnceOutcome::RuntimeDestroyRunning {
                request_id: running,
            } => {
                assert_eq!(running, request_id);
                thread::sleep(Duration::from_millis(25));
            }
            outcome => return outcome,
        }
    }
    panic!("the retirement never settled");
}

fn unit_file(host: &Host, step: &str) -> PathBuf {
    host.temp.path().join("units").join(format!(
        "finite-retirement-{}-{step}.service",
        sanitize_sandbox_name(&host.retire.request.id)
    ))
}

fn held(host: &Host) -> Vec<(String, String)> {
    HeldControlStore::new(host.runner.launcher.held_control_root().unwrap())
        .scan()
        .held
        .into_iter()
        .map(|held| (held.lease.request.id, held.lease_token))
        .collect()
}

#[test]
fn a_retirement_upload_longer_than_a_run_completes_across_runs_while_the_host_keeps_working() {
    let mut host = host(41330);
    std::fs::write(host.temp.path().join("systemd-run-background"), "").unwrap();
    std::fs::write(host.temp.path().join("borg-hold-upload"), "").unwrap();
    let request_id = host.retire.request.id.clone();

    // Run 1 leases the retirement, snapshots `/data` and starts the upload
    // in its own unit, then returns.
    let upload_started = Instant::now();
    assert_eq!(
        run(&mut host.runner),
        RunOnceOutcome::RuntimeDestroyRunning {
            request_id: request_id.clone()
        }
    );
    let token = host.runner.queue.holder(&request_id).unwrap();
    assert_eq!(held(&host), [(request_id.clone(), token.clone())]);
    // Until the hold is written the lease is only the Runner's short grant
    // (FC_RUNNER_LEASE_SECONDS, 600 s by default); renewals extend it.
    assert_eq!(host.runner.queue.leased[0].2, 300);
    assert_eq!(
        std::fs::read_to_string(unit_file(&host, "upload")).unwrap(),
        "running\n"
    );
    // Borg resumes from checkpoints, may wait for another host's lock, and
    // a stop makes it commit a checkpoint first.
    let argv =
        std::fs::read_to_string(unit_file(&host, "upload").with_extension("service.argv")).unwrap();
    for expected in [
        "--property=RemainAfterExit=yes",
        "--property=KillSignal=SIGINT",
        "--property=KillMode=mixed",
        "--checkpoint-interval 300",
        "--lock-wait 3600",
        "--setenv=BORG_RSH=ssh -i",
        "--setenv=BORG_PASSCOMMAND=cat",
        "--setenv=HOME=",
    ] {
        assert!(argv.contains(expected), "{expected} missing from {argv}");
    }
    let staging = host
        .runner
        .launcher
        .retirement_staging_paths(&request_id)
        .unwrap()
        .0;
    assert!(argv.contains(&format!("--working-directory={}", staging.display())));

    // Another agent's stop on this host runs while the upload does.
    host.runner.queue.push(stop_of_the_other_agent(41340));
    assert!(matches!(
        run(&mut host.runner),
        RunOnceOutcome::RuntimeStopped { ref request_id, .. }
            if request_id == "runtime_ctl_stop_other_agent"
    ));
    assert_eq!(
        std::fs::read_to_string(host.fake_state.join(format!("{OTHER_CONTAINER}.status"))).unwrap(),
        "exited"
    );
    // A new agent is created on this host while the upload runs.
    let created_root = host.temp.path().join("runner/kata/runtime-3");
    std::fs::create_dir_all(&created_root).unwrap();
    write_fake_container(
        &host.fake_state,
        CREATED_CONTAINER,
        "runtime:v1",
        "artifact-v1",
        "",
        41342,
        &created_root,
    );
    // Meanwhile Core offers this host no second Borg transfer.
    let offered = host.runner.queue.offered.last().unwrap();
    assert!(offered.supports_runtime_control(RuntimeControlKind::Stop));
    assert!(!offered.supports_runtime_control(RuntimeControlKind::Destroy));
    assert!(!offered.supports_runtime_control(RuntimeControlKind::ArchiveTrial));

    // The upload outlives a run's budget; every run renews the same lease.
    while upload_started.elapsed() < RUN_BUDGET + Duration::from_millis(500) {
        assert_eq!(
            run(&mut host.runner),
            RunOnceOutcome::RuntimeDestroyRunning {
                request_id: request_id.clone()
            }
        );
        thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(
        std::fs::read_to_string(unit_file(&host, "upload")).unwrap(),
        "running\n"
    );
    assert!(
        host.runner
            .launcher
            .inspect(&host.plan.container_name)
            .unwrap()
            .is_some()
    );
    assert_eq!(host.runner.queue.holder(&request_id), Some(token.clone()));
    assert!(
        host.runner
            .queue
            .renewed
            .iter()
            .filter(|(id, renewed)| *id == request_id && *renewed == token)
            .count()
            >= 5
    );

    // The upload ends. Later runs read the archive back in its own unit,
    // prove it, remove compute and settle in Core under the same lease.
    std::fs::remove_file(host.temp.path().join("borg-hold-upload")).unwrap();
    let outcome = run_until_settled(&mut host.runner, &request_id);
    assert_eq!(
        outcome,
        RunOnceOutcome::RuntimeDestroyed {
            request_id: request_id.clone(),
            runtime_id: host.retire.runtime.id.clone(),
        }
    );
    let completed = host.runner.queue.completed.last().unwrap();
    assert_eq!(completed.lease_token, token);
    let receipt = completed.retirement_snapshot.as_ref().unwrap();
    assert_eq!(receipt.request_id, request_id);
    assert!(host.runner.queue.retried.is_empty());
    assert!(host.runner.queue.failed.is_empty());
    assert_eq!(
        host.runner
            .queue
            .leased
            .iter()
            .filter(|(id, _, _)| *id == request_id)
            .count(),
        1
    );
    assert!(
        host.runner
            .launcher
            .inspect(&host.plan.container_name)
            .unwrap()
            .is_none()
    );
    assert!(
        host.runner
            .launcher
            .inspect(OTHER_CONTAINER)
            .unwrap()
            .is_some()
    );
    assert!(
        host.runner
            .launcher
            .inspect(CREATED_CONTAINER)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        std::fs::read_to_string(host.plan.state_root.join("workspace/note")).unwrap(),
        "preserved"
    );
    assert!(!staging.exists());
    assert!(held(&host).is_empty());
    assert!(!unit_file(&host, "upload").exists());
    assert!(!unit_file(&host, "readback").exists());
}

#[test]
fn a_run_killed_right_after_its_lease_was_granted_leaves_it_to_the_next_run() {
    let mut host = host(41332);
    std::fs::write(host.temp.path().join("systemd-run-background"), "").unwrap();
    let request_id = host.retire.request.id.clone();
    // The killed run: Core granted it the lease and it recorded the hold,
    // then it died before any work.
    let capacity = RunnerLeaseCapacity {
        runner_classes: vec![RunnerClass::Kata],
        runtime_capabilities: Some(host.runner.launcher.runtime_capabilities()),
        ..RunnerLeaseCapacity::default()
    };
    let lease = host
        .runner
        .queue
        .lease_runtime_control("runner-1", "killed-run", 3600, None, Some(&capacity))
        .unwrap()
        .unwrap();
    HeldControlStore::new(host.runner.launcher.held_control_root().unwrap())
        .hold(&lease, "killed-run")
        .unwrap();

    // The next run (a new process) continues that lease; Core never has to
    // let it lapse.
    let Host { runner, .. } = host;
    let AgentCreationRunner {
        queue, launcher, ..
    } = runner;
    host.runner =
        AgentCreationRunner::new(queue, launcher, RandomLeaseTokenSource, "runner-1", 300).unwrap();
    let outcome = run_until_settled(&mut host.runner, &request_id);

    assert!(matches!(outcome, RunOnceOutcome::RuntimeDestroyed { .. }));
    assert_eq!(host.runner.queue.completed[0].lease_token, "killed-run");
    assert_eq!(host.runner.queue.leased.len(), 1);
    assert!(held(&host).is_empty());
}

#[test]
fn a_lease_core_let_lapse_is_dropped_and_the_new_lease_resumes_the_running_upload() {
    let mut host = host(41334);
    std::fs::write(host.temp.path().join("systemd-run-background"), "").unwrap();
    std::fs::write(host.temp.path().join("borg-hold-upload"), "").unwrap();
    let request_id = host.retire.request.id.clone();
    assert!(matches!(
        run(&mut host.runner),
        RunOnceOutcome::RuntimeDestroyRunning { .. }
    ));
    let first = host.runner.queue.holder(&request_id).unwrap();
    let upload_argv =
        std::fs::read_to_string(unit_file(&host, "upload").with_extension("service.argv")).unwrap();

    // The runner was away past the lease (Core refuses its token now).
    host.runner.queue.lapse(&request_id);
    assert!(matches!(
        run(&mut host.runner),
        RunOnceOutcome::RuntimeDestroyRunning { .. }
    ));
    let second = host.runner.queue.holder(&request_id).unwrap();
    assert_ne!(first, second);
    assert_eq!(held(&host), [(request_id.clone(), second.clone())]);
    // The upload kept running: it was neither stopped nor started again.
    assert_eq!(
        std::fs::read_to_string(unit_file(&host, "upload")).unwrap(),
        "running\n"
    );
    assert_eq!(
        std::fs::read_to_string(unit_file(&host, "upload").with_extension("service.argv")).unwrap(),
        upload_argv
    );

    std::fs::remove_file(host.temp.path().join("borg-hold-upload")).unwrap();
    assert!(matches!(
        run_until_settled(&mut host.runner, &request_id),
        RunOnceOutcome::RuntimeDestroyed { .. }
    ));
    assert_eq!(host.runner.queue.completed[0].lease_token, second);
}

#[test]
fn a_failed_upload_unit_requeues_the_retirement_and_the_next_lease_uploads_again() {
    let mut host = host(41336);
    std::fs::write(host.temp.path().join("systemd-run-background"), "").unwrap();
    std::fs::write(host.temp.path().join("borg-fail-create"), "").unwrap();
    let request_id = host.retire.request.id.clone();

    let outcome = run_until_settled(&mut host.runner, &request_id);
    assert!(
        matches!(
            outcome,
            RunOnceOutcome::RuntimeDestroyFailed { ref failure_message, .. }
                if failure_message.contains("Borg retirement upload failed")
        ),
        "{outcome:?}"
    );
    assert_eq!(host.runner.queue.retried.len(), 1);
    assert!(held(&host).is_empty());
    // The failed unit is consumed, so the retry can run the step again.
    assert!(!unit_file(&host, "upload").exists());
    assert!(
        host.runner
            .launcher
            .inspect(&host.plan.container_name)
            .unwrap()
            .is_some()
    );

    std::fs::remove_file(host.temp.path().join("borg-fail-create")).unwrap();
    assert!(matches!(
        run_until_settled(&mut host.runner, &request_id),
        RunOnceOutcome::RuntimeDestroyed { .. }
    ));
    assert_eq!(host.runner.queue.leased.len(), 2);
}

#[test]
fn a_transfer_unit_without_retirement_progress_is_refused_before_a_new_snapshot() {
    let mut host = host(41338);
    let units = host.temp.path().join("units");
    std::fs::create_dir_all(&units).unwrap();
    std::fs::write(unit_file(&host, "upload"), "exited 0\n").unwrap();
    let request_id = host.retire.request.id.clone();

    let outcome = run(&mut host.runner);

    assert!(
        matches!(
            outcome,
            RunOnceOutcome::RuntimeDestroyFailed { ref failure_message, .. }
                if failure_message.contains("without retirement progress")
        ),
        "{outcome:?}"
    );
    // Nothing was snapshotted or stopped for it.
    assert!(
        !host
            .runner
            .launcher
            .retirement_staging_paths(&request_id)
            .unwrap()
            .0
            .exists()
    );
    assert_eq!(
        std::fs::read_to_string(
            host.fake_state
                .join(format!("{}.explicitly-stopped", host.plan.container_name))
        )
        .unwrap_or_default(),
        ""
    );
}

#[test]
fn a_run_killed_before_its_hold_strands_only_the_short_grant_and_the_host_keeps_working() {
    let mut host = host(41352);
    std::fs::write(host.temp.path().join("systemd-run-background"), "").unwrap();
    let request_id = host.retire.request.id.clone();
    // The killed run: Core granted the lease the Runner asks for, and the run
    // died before it recorded the hold.
    let capacity = RunnerLeaseCapacity {
        runner_classes: vec![RunnerClass::Kata],
        runtime_capabilities: Some(host.runner.launcher.runtime_capabilities()),
        ..RunnerLeaseCapacity::default()
    };
    let lease_seconds = host.runner.lease_seconds;
    host.runner
        .queue
        .lease_runtime_control(
            "runner-1",
            "killed-before-hold",
            lease_seconds,
            None,
            Some(&capacity),
        )
        .unwrap()
        .unwrap();
    assert!(held(&host).is_empty());

    // Nothing works on that grant until it expires, but nothing waits on it.
    host.runner.queue.push(stop_of_the_other_agent(41354));
    assert!(matches!(
        run(&mut host.runner),
        RunOnceOutcome::RuntimeStopped { .. }
    ));

    // Core lets the short grant lapse; the next run leases and finishes it.
    host.runner.queue.lapse(&request_id);
    assert!(matches!(
        run_until_settled(&mut host.runner, &request_id),
        RunOnceOutcome::RuntimeDestroyed { .. }
    ));
    assert_ne!(
        host.runner.queue.completed.last().unwrap().lease_token,
        "killed-before-hold"
    );
}

#[test]
fn a_readback_from_another_recovery_set_is_refused_and_read_back_again() {
    let mut host = host(41356);
    std::fs::write(host.temp.path().join("systemd-run-background"), "").unwrap();
    std::fs::write(host.temp.path().join("borg-hold-extract"), "").unwrap();
    let request_id = host.retire.request.id.clone();
    let readback_running = |host: &Host| {
        std::fs::read_to_string(unit_file(host, "readback")).is_ok_and(|state| state == "running\n")
    };
    for _ in 0..400 {
        if readback_running(&host) {
            break;
        }
        assert!(matches!(
            run(&mut host.runner),
            RunOnceOutcome::RuntimeDestroyRunning { .. }
        ));
        thread::sleep(Duration::from_millis(25));
    }
    assert!(readback_running(&host));

    // A deploy changes the recovery set while that readback runs.
    let retirement = host.runner.launcher.config.retirement.as_mut().unwrap();
    retirement.repository = "ssh://other.example/./retired".to_string();
    retirement.recovery_authority_id = "finite-assisted-other".to_string();
    std::fs::remove_file(host.temp.path().join("borg-hold-extract")).unwrap();

    let outcome = run_until_settled(&mut host.runner, &request_id);
    assert!(
        matches!(
            outcome,
            RunOnceOutcome::RuntimeDestroyFailed { ref failure_message, .. }
                if failure_message.contains("another recovery set")
        ),
        "{outcome:?}"
    );
    assert!(
        host.runner
            .launcher
            .inspect(&host.plan.container_name)
            .unwrap()
            .is_some()
    );
    assert!(!unit_file(&host, "readback").exists());

    // The retry reads back from the set the receipt names.
    assert!(matches!(
        run_until_settled(&mut host.runner, &request_id),
        RunOnceOutcome::RuntimeDestroyed { .. }
    ));
    let receipt = host
        .runner
        .queue
        .completed
        .last()
        .unwrap()
        .retirement_snapshot
        .clone()
        .unwrap();
    assert_eq!(receipt.recovery_authority_id, "finite-assisted-other");
}

#[test]
fn an_unreadable_hold_pauses_borg_transfers_but_not_the_hosts_other_controls() {
    let mut host = host(41358);
    let root = host.runner.launcher.held_control_root().unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let unreadable = root.join("runtime_ctl_unknown.json");
    std::fs::write(&unreadable, b"{ not json").unwrap();
    let request_id = host.retire.request.id.clone();
    host.runner.queue.push(stop_of_the_other_agent(41360));

    // The retirement is older, but only the stop is offered and leased.
    assert!(matches!(
        run(&mut host.runner),
        RunOnceOutcome::RuntimeStopped { ref request_id, .. }
            if request_id == "runtime_ctl_stop_other_agent"
    ));
    let offered = host.runner.queue.offered.last().unwrap();
    assert!(!offered.supports_runtime_control(RuntimeControlKind::Destroy));
    assert_eq!(host.runner.queue.holder(&request_id), None);
    assert_eq!(run(&mut host.runner), RunOnceOutcome::Idle);
    // The evidence stays for an operator.
    assert_eq!(std::fs::read(&unreadable).unwrap(), b"{ not json");

    // Once it is resolved, the retirement runs.
    std::fs::remove_file(&unreadable).unwrap();
    assert!(matches!(
        run_until_settled(&mut host.runner, &request_id),
        RunOnceOutcome::RuntimeDestroyed { .. }
    ));
}
