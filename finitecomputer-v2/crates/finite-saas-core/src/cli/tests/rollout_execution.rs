use super::*;

#[tokio::test]
async fn rollout_plan_only_performs_no_writes() {
    let initial = vec![rollout_overview(
        "project-a",
        "artifact-v1",
        true,
        true,
        RuntimeSummaryStatus::Online,
    )];
    let store = FakeRolloutStore::new(initial.clone(), initial, BTreeMap::new());
    let report = runtime_artifact_rollout(
        &store,
        rollout_input(
            RuntimeArtifactRolloutScope::Projects(vec!["project-a".to_string()]),
            true,
        ),
        Duration::from_millis(1),
    )
    .await
    .unwrap();

    assert_eq!(report.planned.len(), 1);
    assert!(report.outcomes.is_empty());
    assert!(store.enqueued_projects.lock().unwrap().is_empty());
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["target_artifact_id"], "artifact-v2");
    assert_eq!(json["planned"][0]["project_id"], "project-a");
    assert_eq!(json["planned"][0]["agent_runtime_id"], "runtime-project-a");
    assert_eq!(
        json["planned"][0]["project_display_name"],
        "Agent project-a"
    );
    assert_eq!(
        json["planned"][0]["source_machine_id"],
        "finite-kata-project-a"
    );
    assert_eq!(json["planned"][0]["source_host_id"], "lat1");
    assert_eq!(json["planned"][0]["target_artifact_id"], "artifact-v2");
}

#[tokio::test]
async fn rollout_rejects_runtime_replaced_between_plan_and_enqueue() {
    let initial = vec![rollout_overview(
        "project-a",
        "artifact-v1",
        true,
        true,
        RuntimeSummaryStatus::Online,
    )];
    let store = FakeRolloutStore::new(initial.clone(), initial, BTreeMap::new())
        .with_enqueue_runtime_override("project-a", "runtime-replacement");
    let report = runtime_artifact_rollout(
        &store,
        rollout_input(
            RuntimeArtifactRolloutScope::Projects(vec!["project-a".to_string()]),
            false,
        ),
        Duration::from_millis(1),
    )
    .await
    .unwrap();

    assert!(report.halted);
    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(
        report.outcomes[0].status,
        RuntimeArtifactRolloutOutcomeStatus::EnqueueFailed
    );
    assert!(store.enqueued_projects.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rollout_stops_before_next_enqueue_on_failure() {
    let initial = vec![
        rollout_overview(
            "project-a",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        ),
        rollout_overview(
            "project-b",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        ),
    ];
    let store = FakeRolloutStore::new(
        initial.clone(),
        initial,
        BTreeMap::from([("project-a".to_string(), RuntimeControlRequestStatus::Failed)]),
    );
    let report = runtime_artifact_rollout(
        &store,
        rollout_input(
            RuntimeArtifactRolloutScope::Projects(vec![
                "project-a".to_string(),
                "project-b".to_string(),
            ]),
            false,
        ),
        Duration::from_millis(1),
    )
    .await
    .unwrap();

    assert!(report.halted);
    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(
        report.outcomes[0].status,
        RuntimeArtifactRolloutOutcomeStatus::RequestFailed
    );
    assert_eq!(*store.enqueued_projects.lock().unwrap(), vec!["project-a"]);
}

#[tokio::test]
async fn rollout_stops_before_next_enqueue_when_postcondition_is_not_met() {
    let initial = vec![
        rollout_overview(
            "project-a",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        ),
        rollout_overview(
            "project-b",
            "artifact-v1",
            true,
            true,
            RuntimeSummaryStatus::Online,
        ),
    ];
    let store = FakeRolloutStore::new(initial.clone(), initial, BTreeMap::new());
    let report = runtime_artifact_rollout(
        &store,
        rollout_input(
            RuntimeArtifactRolloutScope::Projects(vec![
                "project-a".to_string(),
                "project-b".to_string(),
            ]),
            false,
        ),
        Duration::from_millis(1),
    )
    .await
    .unwrap();

    assert!(report.halted);
    assert_eq!(
        report.outcomes[0].status,
        RuntimeArtifactRolloutOutcomeStatus::PostconditionFailed
    );
    assert_eq!(*store.enqueued_projects.lock().unwrap(), vec!["project-a"]);
}

#[tokio::test]
async fn rollout_timeout_reports_live_request_without_cancellation_claim() {
    let initial = vec![rollout_overview(
        "project-a",
        "artifact-v1",
        true,
        true,
        RuntimeSummaryStatus::Online,
    )];
    let store = FakeRolloutStore::new(
        initial.clone(),
        initial,
        BTreeMap::from([(
            "project-a".to_string(),
            RuntimeControlRequestStatus::Launching,
        )]),
    );
    let mut input = rollout_input(
        RuntimeArtifactRolloutScope::Projects(vec!["project-a".to_string()]),
        false,
    );
    input.wait_timeout = Duration::from_millis(1);
    let report = runtime_artifact_rollout(&store, input, Duration::from_millis(1))
        .await
        .unwrap();

    assert_eq!(
        report.outcomes[0].request_id.as_deref(),
        Some("request-project-a")
    );
    assert_eq!(
        report.outcomes[0].status,
        RuntimeArtifactRolloutOutcomeStatus::TimedOut
    );
    assert!(
        report.outcomes[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("was not cancelled")
    );
}
