use super::*;

fn completion_input(
    artifact: Option<&str>,
    receipt: Option<RuntimeRetirementSnapshotReceipt>,
) -> CompleteRuntimeControlRequestInput {
    CompleteRuntimeControlRequestInput {
        request_id: "request_1".to_string(),
        runner_id: "runner-1".to_string(),
        lease_token: "lease-1".to_string(),
        runtime_artifact_id: artifact.map(str::to_string),
        state_schema_version: artifact.map(|_| "state-v1".to_string()),
        runtime_capabilities: None,
        runtime_host: artifact.map(|_| "https://runtime.example".to_string()),
        published_app_urls: artifact.map(|_| vec!["https://app.example".to_string()]),
        retirement_snapshot: receipt,
        now: None,
    }
}

fn retirement_receipt() -> RuntimeRetirementSnapshotReceipt {
    RuntimeRetirementSnapshotReceipt {
        schema: RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA.to_string(),
        request_id: "request_1".to_string(),
        project_id: "project_1".to_string(),
        agent_runtime_id: "runtime_1".to_string(),
        durable_state_id: "runtime_1".to_string(),
        runtime_artifact_id: "artifact_1".to_string(),
        backend: RUNTIME_RETIREMENT_BACKEND_BORG.to_string(),
        locator: "retirement-request_1".to_string(),
        zip_bytes: 1,
        zip_sha256: "a".repeat(64),
        manifest_sha256: "b".repeat(64),
        created_at: NOW.to_string(),
        verified_at: NOW.to_string(),
        recovery_authority_id: "finite-assisted-test".to_string(),
        retention_policy: RUNTIME_RETIREMENT_RETENTION_INDEFINITE.to_string(),
    }
}

#[test]
fn runtime_control_completion_parse_pins_the_three_shapes() {
    // Restart/Recover/Stop complete plainly; any facts are a mismatch.
    for kind in [
        RuntimeControlKind::Restart,
        RuntimeControlKind::RecoverKnownGoodChatRuntime,
        RuntimeControlKind::Stop,
    ] {
        assert_eq!(
            RuntimeControlCompletion::parse(kind, &completion_input(None, None)).unwrap(),
            RuntimeControlCompletion::Plain
        );
        assert!(matches!(
            RuntimeControlCompletion::parse(kind, &completion_input(Some("artifact_1"), None)),
            Err(CoreError::RuntimeUpgradeCompletionMismatch)
        ));
        assert!(matches!(
            RuntimeControlCompletion::parse(
                kind,
                &completion_input(None, Some(retirement_receipt()))
            ),
            Err(CoreError::RuntimeRetirementSnapshotMismatch)
        ));
    }

    // Upgrade requires the full fact set and rejects the receipt.
    assert!(matches!(
        RuntimeControlCompletion::parse(RuntimeControlKind::Upgrade, &completion_input(None, None)),
        Err(CoreError::RuntimeUpgradeCompletionMismatch)
    ));
    assert!(matches!(
        RuntimeControlCompletion::parse(
            RuntimeControlKind::Upgrade,
            &completion_input(Some("artifact_1"), Some(retirement_receipt()))
        ),
        Err(CoreError::RuntimeRetirementSnapshotMismatch)
    ));
    assert!(matches!(
        RuntimeControlCompletion::parse(
            RuntimeControlKind::Upgrade,
            &completion_input(Some("artifact_1"), None)
        ),
        Ok(RuntimeControlCompletion::Upgrade(_))
    ));

    // Destroy requires the receipt and rejects upgrade facts.
    assert!(matches!(
        RuntimeControlCompletion::parse(RuntimeControlKind::Destroy, &completion_input(None, None)),
        Err(CoreError::RuntimeRetirementSnapshotMismatch)
    ));
    assert!(matches!(
        RuntimeControlCompletion::parse(
            RuntimeControlKind::Destroy,
            &completion_input(Some("artifact_1"), Some(retirement_receipt()))
        ),
        Err(CoreError::RuntimeUpgradeCompletionMismatch)
    ));
    assert_eq!(
        RuntimeControlCompletion::parse(
            RuntimeControlKind::Destroy,
            &completion_input(None, Some(retirement_receipt()))
        )
        .unwrap(),
        RuntimeControlCompletion::Destroy(Box::new(retirement_receipt()))
    );
}

#[test]
fn lifecycle_machine_legal_chains_reach_their_terminals() {
    use crate::runtime_lifecycle::{RuntimeLifecycle, phase};

    // The up-bound chain: every up-bound operation passes through Ready
    // before it may be recorded Succeeded.
    let lifecycle = RuntimeLifecycle::<phase::Requested>::enqueue();
    assert_eq!(lifecycle.status(), RuntimeControlRequestStatus::Requested);
    let lifecycle = lifecycle.lease();
    assert_eq!(lifecycle.status(), RuntimeControlRequestStatus::Launching);
    let lifecycle = lifecycle.compute_up(&RuntimeControlCompletion::Plain);
    assert_eq!(lifecycle.status(), RuntimeControlRequestStatus::ComputeUp);
    let lifecycle = lifecycle.ready();
    assert_eq!(lifecycle.status(), RuntimeControlRequestStatus::Ready);
    let terminal = lifecycle.succeed();
    assert_eq!(terminal.status(), RuntimeControlRequestStatus::Succeeded);

    // The down-bound chain: Stop/Destroy confirm straight into Stopped.
    let terminal = RuntimeLifecycle::<phase::Requested>::enqueue()
        .lease()
        .confirm_stopped(&RuntimeControlCompletion::Plain);
    assert_eq!(terminal.status(), RuntimeControlRequestStatus::Stopped);

    // Retirement requeues from Launching back to Requested.
    let retried = RuntimeLifecycle::<phase::Requested>::enqueue()
        .lease()
        .retry();
    assert_eq!(retried.status(), RuntimeControlRequestStatus::Requested);
}

#[test]
fn lifecycle_machine_failure_is_named_from_every_non_terminal_state() {
    use crate::runtime_lifecycle::{RuntimeLifecycle, phase};

    let stages = [
        RuntimeLifecycleStage::Launch,
        RuntimeLifecycleStage::Compute,
        RuntimeLifecycleStage::Readiness,
        RuntimeLifecycleStage::Retirement,
        RuntimeLifecycleStage::Unknown,
    ];
    for stage in stages {
        let failed = RuntimeLifecycle::<phase::Requested>::enqueue().fail(stage);
        assert_eq!(failed.status(), RuntimeControlRequestStatus::Failed);
        assert_eq!(failed.stage(), stage);

        let failed = RuntimeLifecycle::<phase::Requested>::enqueue()
            .lease()
            .fail(stage);
        assert_eq!(failed.stage(), stage);

        let failed = RuntimeLifecycle::<phase::Requested>::enqueue()
            .lease()
            .compute_up(&RuntimeControlCompletion::Plain)
            .fail(stage);
        assert_eq!(failed.stage(), stage);

        let failed = RuntimeLifecycle::<phase::Requested>::enqueue()
            .lease()
            .compute_up(&RuntimeControlCompletion::Plain)
            .ready()
            .fail(stage);
        assert_eq!(failed.stage(), stage);
    }
}

#[test]
fn lifecycle_machine_rehydration_only_accepts_the_exact_phase() {
    use crate::runtime_lifecycle::{RuntimeLifecycle, phase};

    for status in [
        RuntimeControlRequestStatus::Requested,
        RuntimeControlRequestStatus::Launching,
        RuntimeControlRequestStatus::ComputeUp,
        RuntimeControlRequestStatus::Ready,
        RuntimeControlRequestStatus::Succeeded,
        RuntimeControlRequestStatus::Stopped,
        RuntimeControlRequestStatus::Failed,
    ] {
        assert_eq!(
            RuntimeLifecycle::<phase::Requested>::from_status(status).is_some(),
            status == RuntimeControlRequestStatus::Requested
        );
        assert_eq!(
            RuntimeLifecycle::<phase::Launching>::from_status(status).is_some(),
            status == RuntimeControlRequestStatus::Launching
        );
        assert_eq!(
            RuntimeLifecycle::<phase::ComputeUp>::from_status(status).is_some(),
            status == RuntimeControlRequestStatus::ComputeUp
        );
        assert_eq!(
            RuntimeLifecycle::<phase::Ready>::from_status(status).is_some(),
            status == RuntimeControlRequestStatus::Ready
        );
    }
    // Terminal states have no outgoing transitions, so they expose no
    // `from_status` rehydration into a continuing machine at all.
}

#[test]
fn lifecycle_status_terminal_and_active_sets_are_partitioned() {
    for (status, terminal) in [
        (RuntimeControlRequestStatus::Requested, false),
        (RuntimeControlRequestStatus::Launching, false),
        (RuntimeControlRequestStatus::ComputeUp, false),
        (RuntimeControlRequestStatus::Ready, false),
        (RuntimeControlRequestStatus::Succeeded, true),
        (RuntimeControlRequestStatus::Stopped, true),
        (RuntimeControlRequestStatus::Failed, true),
    ] {
        assert_eq!(status.is_terminal(), terminal, "{status:?}");
        assert_eq!(status.is_active(), !terminal, "{status:?}");
    }
    // The N-1 deploy bridge: legacy "running" parses as Launching, and
    // serialization only ever emits the canonical value.
    assert_eq!(
        parse_runtime_control_request_status("running"),
        Some(RuntimeControlRequestStatus::Launching)
    );
    assert_eq!(
        serde_json::to_value(RuntimeControlRequestStatus::Launching).unwrap(),
        serde_json::Value::String("launching".to_string())
    );
}
