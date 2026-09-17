use super::*;

#[tokio::test]
async fn runtime_artifact_promotion_does_not_mutate_healthy_running_agent() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_a = complete_self_serve_agent(
            &db,
            "a@finite.vip",
            "user_workos_a",
            "agent-a",
            "oslo-agent-a",
            "artifact-v1",
            "2026-05-25T13:02:00Z",
        )
        .await;
        let runtime_a_before = db.agent_runtime(&runtime_a).await.unwrap().clone();

        promote_runtime_artifact_version(
            &db,
            "artifact-v2",
            &format!(
                "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
                "b".repeat(64)
            ),
            "v2",
            "db-v1",
            "2026-05-25T14:00:00Z",
        )
        .await;

        assert_eq!(
            db.agent_runtime(&runtime_a).await.unwrap(),
            runtime_a_before
        );
        assert_eq!(
            db.agent_runtime(&runtime_a)
                .await
                .unwrap()
                .runtime_artifact_id
                .as_deref(),
            Some("artifact-v1")
        );

        let runtime_b = complete_self_serve_agent(
            &db,
            "b@finite.vip",
            "user_workos_b",
            "agent-b",
            "oslo-agent-b",
            "artifact-v2",
            "2026-05-25T14:05:00Z",
        )
        .await;
        assert_eq!(
            db.agent_runtime(&runtime_b)
                .await
                .unwrap()
                .runtime_artifact_id
                .as_deref(),
            Some("artifact-v2")
        );
        assert_eq!(
            db.agent_runtime(&runtime_a)
                .await
                .unwrap()
                .runtime_artifact_id
                .as_deref(),
            Some("artifact-v1")
        );
    })
    .await;
}

#[tokio::test]
async fn promoted_or_runtime_referenced_artifact_material_is_immutable() {
    with_isolated_postgres(|db| async move {
        let input = UpsertRuntimeArtifactInput {
            id: "artifact-immutable".to_string(),
            kind: RuntimeArtifactKind::OciImage,
            reference: format!("ghcr.io/finite/runtime@sha256:{}", "a".repeat(64)),
            version_label: "v1".to_string(),
            source_git_sha: Some("git-v1".to_string()),
            finitec_version: Some("finitec-v1".to_string()),
            hermes_source_ref: Some("hermes-v1".to_string()),
            finite_platform_plugin_ref: Some("plugin-v1".to_string()),
            state_schema_version: "db-v1".to_string(),
            base_image: Some("base-v1".to_string()),
            recover_known_good_chat: false,
            promoted: false,
            now: Some(NOW.to_string()),
        };
        db.upsert_runtime_artifact(input.clone()).await.unwrap();

        let mut before_promotion = input.clone();
        before_promotion.version_label = "v1-corrected".to_string();
        db.upsert_runtime_artifact(before_promotion.clone())
            .await
            .unwrap();
        before_promotion.promoted = true;
        db.upsert_runtime_artifact(before_promotion.clone())
            .await
            .unwrap();

        let mut exact_retry = before_promotion.clone();
        exact_retry.now = Some(LATER.to_string());
        db.upsert_runtime_artifact(exact_retry).await.unwrap();
        let mut mutation = before_promotion;
        mutation.reference = format!("ghcr.io/finite/runtime@sha256:{}", "b".repeat(64));
        assert!(matches!(
            db.upsert_runtime_artifact(mutation).await.unwrap_err(),
            CoreError::RuntimeArtifactImmutable
        ));

        // Same invariant for an UNPROMOTED artifact that a Runtime
        // references. Create a real Runtime and repoint it, rather than
        // fabricating a row: `agent_runtimes.runtime_artifact_id` is a
        // foreign key, and the invariant is about the reference existing,
        // not about how it got there.
        // The agent leases the most recently promoted artifact, which is
        // `artifact-immutable`, so the Runtime references it directly.
        complete_self_serve_agent(
            &db,
            "immutable@finite.vip",
            "workos_immutable",
            "immutable-key",
            "immutable-machine",
            "artifact-immutable",
            NOW,
        )
        .await;
        db.exec("UPDATE runtime_artifacts SET promoted_at = NULL WHERE id = 'artifact-immutable'")
            .await;

        let mut referenced_mutation = input;
        referenced_mutation.version_label = "mutated".to_string();
        assert!(matches!(
            db.upsert_runtime_artifact(referenced_mutation)
                .await
                .unwrap_err(),
            CoreError::RuntimeArtifactImmutable
        ));
    })
    .await;
}

#[tokio::test]
async fn self_serve_agent_creation_requires_promoted_runtime_artifact() {
    with_isolated_postgres(|db| async move {
        // The shared harness seeds one promoted artifact so creation tests
        // can lease. This test is about having NO launchable artifact.
        db.exec("UPDATE runtime_artifacts SET promoted_at = NULL")
            .await;
        let launch_code = issue_test_launch_code(&db).await;
        db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
            id: "artifact-v1".to_string(),
            kind: RuntimeArtifactKind::OciImage,
            reference: "ghcr.io/finitecomputer/finite-agent-runtime:v1".to_string(),
            version_label: "v1".to_string(),
            source_git_sha: None,
            finitec_version: None,
            hermes_source_ref: None,
            finite_platform_plugin_ref: None,
            state_schema_version: "db-v1".to_string(),
            base_image: Some("python:3.11-trixie".to_string()),
            recover_known_good_chat: false,
            promoted: false,
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();
        let requested = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let error = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap_err();

        assert!(matches!(error, CoreError::RuntimeArtifactUnavailable));
        assert!(db.all_agent_runtimes().await.is_empty());
        assert_eq!(
            db.agent_creation_request(&requested.request.id)
                .await
                .unwrap()
                .status,
            AgentCreationRequestStatus::Requested
        );
    })
    .await;
}

#[tokio::test]
async fn oci_runtime_artifacts_support_hosted_runtime_control() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "new@finite.vip",
            "user_workos_new",
            "first-submit",
            "docker-agent-001",
            "artifact-v1",
            "2026-05-25T13:02:00Z",
        )
        .await;
        let project_id = db
            .agent_runtime(&runtime_id)
            .await
            .unwrap()
            .project_id
            .clone();

        let restart = db
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(restart.agent_runtime_id, runtime_id);
        assert_eq!(restart.kind, RuntimeControlKind::Restart);
    })
    .await;
}
