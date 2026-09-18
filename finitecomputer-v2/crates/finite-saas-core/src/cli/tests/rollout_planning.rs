use super::*;

#[test]
fn rollout_planner_is_scoped_deterministic_and_canary_first() {
    let mut other_host = rollout_overview(
        "project-other-host",
        "artifact-v1",
        true,
        true,
        RuntimeSummaryStatus::Online,
    );
    other_host.source_host_id = "lat2".to_string();
    let overviews = vec![
        rollout_overview(
            "project-b",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        ),
        rollout_overview(
            "project-z-canary",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        ),
        rollout_overview(
            "project-inactive",
            "artifact-v1",
            false,
            true,
            RuntimeSummaryStatus::Offline,
        ),
        rollout_overview(
            "project-unsupported",
            "artifact-v1",
            true,
            false,
            RuntimeSummaryStatus::Online,
        ),
        rollout_overview(
            "project-current",
            "artifact-v2",
            true,
            true,
            RuntimeSummaryStatus::Online,
        ),
        other_host,
    ];
    let plan = plan_runtime_artifact_rollout(
        overviews.clone(),
        &RuntimeArtifactRolloutScope::All {
            canary_project_id: "project-z-canary".to_string(),
        },
        "lat1",
        "artifact-v2",
    );

    assert_eq!(
        plan.planned
            .iter()
            .map(|entry| entry.project_id.as_str())
            .collect::<Vec<_>>(),
        vec!["project-z-canary", "project-b"]
    );
    assert_eq!(
        plan.skipped
            .iter()
            .map(|entry| (entry.project_id.as_str(), entry.reason.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("project-current", "already_on_target_artifact"),
            ("project-inactive", "inactive_runtime_link"),
            ("project-unsupported", "runtime_upgrade_not_supported"),
        ]
    );
    assert!(plan.execution_blocked_reason.is_none());

    let selected = plan_runtime_artifact_rollout(
        overviews,
        &RuntimeArtifactRolloutScope::Projects(vec![
            "project-z-canary".to_string(),
            "project-b".to_string(),
            "project-missing".to_string(),
        ]),
        "lat1",
        "artifact-v2",
    );
    assert_eq!(
        selected
            .planned
            .iter()
            .map(|entry| entry.project_id.as_str())
            .collect::<Vec<_>>(),
        vec!["project-z-canary", "project-b"]
    );
    assert_eq!(selected.skipped.len(), 1);
    assert_eq!(selected.skipped[0].project_id, "project-missing");
    assert_eq!(selected.skipped[0].reason, "project_not_found");
    assert!(
        selected
            .execution_blocked_reason
            .as_deref()
            .unwrap()
            .contains("project-missing")
    );
}

/// A partially retired Runtime (verified receipt, link still active — the
/// Sites Canary 0715 ghost shape) is classified by its recorded
/// offboarding phase, never planned, and needs no manual exclusion list.
/// Every non-terminal phase is skipped with its exact phase; a terminal
/// archived Runtime keeps the ordinary inactive-link classification.
#[test]
fn rollout_planner_skips_in_progress_offboardings_by_phase() {
    let mut ghost = rollout_overview(
        "project-ghost",
        "artifact-v1",
        true,
        true,
        RuntimeSummaryStatus::Online,
    );
    ghost.offboarding_phase = Some(OffboardingPhase::ComputeRemoved);
    let mut retiring = rollout_overview(
        "project-retiring",
        "artifact-v1",
        true,
        true,
        RuntimeSummaryStatus::Online,
    );
    retiring.offboarding_phase = Some(OffboardingPhase::RetirementRequested);
    let mut receipt_only = rollout_overview(
        "project-receipt",
        "artifact-v1",
        true,
        true,
        RuntimeSummaryStatus::Offline,
    );
    receipt_only.offboarding_phase = Some(OffboardingPhase::ReceiptVerified);
    let mut deactivating = rollout_overview(
        "project-deactivating",
        "artifact-v1",
        false,
        true,
        RuntimeSummaryStatus::Offline,
    );
    deactivating.offboarding_phase = Some(OffboardingPhase::LinkDeactivated);
    let mut archived = rollout_overview(
        "project-archived",
        "artifact-v1",
        false,
        true,
        RuntimeSummaryStatus::Offline,
    );
    archived.offboarding_phase = Some(OffboardingPhase::Archived);
    let overviews = vec![
        rollout_overview(
            "project-canary",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        ),
        ghost,
        retiring,
        receipt_only,
        deactivating,
        archived,
    ];
    let plan = plan_runtime_artifact_rollout(
        overviews,
        &RuntimeArtifactRolloutScope::All {
            canary_project_id: "project-canary".to_string(),
        },
        "lat1",
        "artifact-v2",
    );

    assert_eq!(
        plan.planned
            .iter()
            .map(|entry| entry.project_id.as_str())
            .collect::<Vec<_>>(),
        vec!["project-canary"]
    );
    assert_eq!(
        plan.skipped
            .iter()
            .map(|entry| (entry.project_id.as_str(), entry.reason.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("project-archived", "inactive_runtime_link"),
            ("project-deactivating", "offboarding_link_deactivated"),
            ("project-ghost", "offboarding_compute_removed"),
            ("project-receipt", "offboarding_receipt_verified"),
            ("project-retiring", "offboarding_retirement_requested"),
        ]
    );
    assert!(plan.execution_blocked_reason.is_none());
}
