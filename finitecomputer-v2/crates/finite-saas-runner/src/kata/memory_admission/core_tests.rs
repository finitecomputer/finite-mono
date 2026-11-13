//! Real Runner -> HTTP Core -> Postgres proof. Only VM execution and the
//! changing host observations are synthetic; queue and credential state are real.
use super::*;
use crate::*;
use std::cell::Cell;
mod fixture;
use fixture::CoreFixture;

#[derive(Clone, Copy)]
enum Refusal {
    Memory,
    Unknown,
    Contention,
}

struct Launcher {
    config: KataConfig,
    refusal: Refusal,
    recovered: Cell<bool>,
    advertised: Cell<bool>,
    launches: usize,
}

impl RuntimeLauncher for Launcher {
    fn validate_ready(&self) -> Result<(), RunnerError> {
        Ok(())
    }
    fn runner_class(&self) -> RunnerClass {
        RunnerClass::Kata
    }
    fn runtime_capabilities(&self) -> RuntimeCapabilitiesEnvelope {
        state_preserving_runtime_capabilities(false)
    }
    fn source_host_id(&self) -> Option<&str> {
        Some("memory-host")
    }
    fn runner_capacity(&self) -> RunnerLeaseCapacity {
        // Capacity was healthy when advertised; it changes while Core leases.
        self.advertised.set(true);
        RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Kata],
            max_sandbox_count: Some(32),
            active_sandbox_count: Some(6),
            available_memory_bytes: Some(64 * GIB),
            ..RunnerLeaseCapacity::default()
        }
    }
    fn admit_creation(
        &self,
        _: &AgentCreationLease,
    ) -> Result<CreationAdmissionGuard, RunnerError> {
        assert!(self.advertised.get());
        admit(&self.config, || MemoryAdmission {
            available_memory_bytes: match (self.recovered.get(), self.refusal) {
                (false, Refusal::Memory) => Some(0),
                (false, Refusal::Unknown) => None,
                _ => Some(64 * GIB),
            },
            available_shared_memory_bytes: Some(64 * GIB),
        })
    }
    fn launch(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        assert!(
            lock_creation(&self.config).is_err(),
            "guard must survive provisioning through launch"
        );
        assert!(options.finite_private.is_some());
        assert!(
            options
                .secret_environment
                .contains_key("FINITE_CORE_CREDENTIAL")
        );
        self.launches += 1;
        let spec = creation_runtime_spec(lease, RunnerClass::Kata)?.unwrap();
        Ok(RuntimeLaunchFacts {
            source_host_id: "memory-host".into(),
            source_machine_id: "memory-agent".into(),
            runtime_artifact_id: Some(spec.runtime_artifact_id.clone()),
            state_schema_version: Some(spec.state_schema_version.clone()),
            provider_runtime_handle: None,
            contact_endpoint: Some("http://127.0.0.1:8080/contact".into()),
            display_name: Some(lease.project.display_name.clone()),
            hostname: None,
            runtime_host: Some("http://127.0.0.1:8080".into()),
            runtime_status: RuntimeSummaryStatus::Online,
            active_inference_profile: None,
            hermes_available: Some(true),
            published_app_urls: vec![],
        })
    }
}

fn proves_retry_after(refusal: Refusal) {
    let core = CoreFixture::new();
    let temp = tempfile::tempdir().unwrap();
    let config = KataConfig {
        work_root: temp.path().into(),
        ..KataConfig::default()
    };
    let competing = matches!(refusal, Refusal::Contention).then(|| lock_creation(&config).unwrap());
    // Box also verifies the production trait adapter forwards admission.
    let launcher = Box::new(Launcher {
        config,
        refusal,
        recovered: Cell::new(false),
        advertised: Cell::new(false),
        launches: 0,
    });
    let mut runner = AgentCreationRunner::new(
        CoreHttpAgentCreationQueue::new(&core.base_url, &core.runner_token).unwrap(),
        launcher,
        RandomLeaseTokenSource,
        "memory-runner",
        300,
    )
    .unwrap()
    .with_default_finite_private_inference(FinitePrivateRuntimeDefaults {
        base_url: DEFAULT_FINITE_PRIVATE_BASE_URL.into(),
        model: DEFAULT_FINITE_PRIVATE_MODEL.into(),
        api_key_override: None,
    })
    .with_runtime_core_bootstrap(core.base_url.clone())
    .unwrap();
    let outcome = runner.run_once().unwrap();
    assert!(
        matches!(outcome, RunOnceOutcome::LaunchDeferred { ref request_id, .. } if request_id == &core.request_id),
        "{outcome:?}"
    );
    let before = core.request_state();
    assert_eq!(before["status"], "launching");
    assert!(before["failure_message"].is_null());
    assert_eq!(
        core.key_counts(),
        (0, 0),
        "no credentials minted for deferred admission"
    );
    assert_eq!(runner.launcher.launches, 0);
    assert_eq!(
        runner.run_once().unwrap(),
        RunOnceOutcome::Idle,
        "a still-valid lease cannot be stolen"
    );
    drop(competing);
    assert!(
        lock_creation(&runner.launcher.config).is_ok(),
        "refusal must release its guard"
    );
    runner.launcher.recovered.set(true);
    // Advance only this synthetic request's expiry, rather than sleeping 5 min.
    core.expire_lease();
    let outcome = runner.run_once().unwrap();
    assert!(
        matches!(outcome, RunOnceOutcome::Launched { ref request_id, .. } if request_id == &core.request_id),
        "{outcome:?}"
    );
    assert_eq!(runner.launcher.launches, 1);
    assert_eq!(core.request_state()["status"], "running");
    assert_eq!(
        core.key_counts(),
        (1, 1),
        "one successful attempt's credentials only"
    );
    assert!(
        lock_creation(&runner.launcher.config).is_ok(),
        "completion releases admission"
    );
}

#[test]
fn memory_admission_core_retries_pressure_after_lease_without_orphan_keys() {
    proves_retry_after(Refusal::Memory);
}
#[test]
fn memory_admission_core_retries_unreadable_memory_without_orphan_keys() {
    proves_retry_after(Refusal::Unknown);
}
#[test]
fn memory_admission_core_retries_lock_contention_without_orphan_keys() {
    proves_retry_after(Refusal::Contention);
}
