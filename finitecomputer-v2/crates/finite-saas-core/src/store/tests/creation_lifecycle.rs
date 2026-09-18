use super::*;

/// Timestamp reads do not depend on the database server's timezone.
///
/// A bare `col::text` renders Postgres's display format in the SERVER's
/// zone, so the same row read on a UTC box and an Asia/Kolkata box produced
/// different strings -- and neither was RFC3339. `core_rfc3339` pins the
/// rendering to UTC. This drives the session timezone directly so the
/// guarantee is checked rather than assumed from wherever CI happens to run.
#[tokio::test]
async fn postgres_timestamp_reads_are_independent_of_server_timezone() {
    with_isolated_postgres(|db| async move {
        let written = "2026-05-25T12:00:00Z";
        db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
            id: "artifact-tz".to_string(),
            kind: crate::RuntimeArtifactKind::OciImage,
            reference: format!(
                "ghcr.io/finitecomputer/agent-runtime:tz@sha256:{}",
                "c".repeat(64)
            ),
            version_label: "tz".to_string(),
            source_git_sha: None,
            finitec_version: None,
            hermes_source_ref: None,
            finite_platform_plugin_ref: None,
            state_schema_version: "state-v1".to_string(),
            base_image: None,
            canary_runtime_id: None,
            recover_known_good_chat: false,
            promoted: false,
            now: Some(written.to_string()),
        })
        .await
        .unwrap();

        for zone in ["UTC", "America/Chicago", "Asia/Kolkata"] {
            let client = db.store.connection().await.unwrap();
            client
                .batch_execute(&format!("SET TIME ZONE '{zone}'"))
                .await
                .unwrap();
            let artifact = select_runtime_artifact(&**client, "artifact-tz")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                artifact.created_at, written,
                "read under TimeZone={zone} must match what was written"
            );
            // And Core must be able to read its own output back.
            crate::parse_time(&artifact.created_at)
                .unwrap_or_else(|_| panic!("unparsable under TimeZone={zone}"));
        }
    })
    .await;
}

/// Artifact selection orders by the TIMESTAMPTZ column, not its rendered
/// text.
///
/// `SELECT core_rfc3339(promoted_at) AS promoted_at ... ORDER BY
/// promoted_at` binds the output column in Postgres, so a bare name sorts
/// lexicographically. RFC3339 only sorts correctly as text at a FIXED
/// precision, and `current_time_iso` trims trailing zeros, so a whole
/// second ("…:02Z") sorts after a fractional one ("…:02.5Z") -- 'Z' > '.'.
/// That would launch new agents on the older artifact.
#[tokio::test]
async fn postgres_launchable_artifact_orders_by_instant_not_rendered_text() {
    with_isolated_postgres(|db| async move {
        // Same second, differing fractional precision. Lexicographically
        // "…:02Z" > "…:02.500000Z"; chronologically it is earlier.
        for (id, promoted) in [
            ("artifact-frac", "2030-01-01T00:00:02.5Z"),
            ("artifact-whole", "2030-01-01T00:00:02Z"),
        ] {
            db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: id.to_string(),
                kind: crate::RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/agent-runtime:{id}@sha256:{}",
                    "a".repeat(64)
                ),
                version_label: id.to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: None,
                canary_runtime_id: None,
                recover_known_good_chat: false,
                promoted: true,
                now: Some(promoted.to_string()),
            })
            .await
            .unwrap();
        }

        let client = db.store.connection().await.unwrap();
        let latest = select_latest_launchable_runtime_artifact(&**client)
            .await
            .unwrap();
        assert_eq!(
            latest.id, "artifact-frac",
            "the later instant must win, even though it sorts earlier as text"
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_row_native_create_lease_complete_and_visible_reads() {
    with_isolated_postgres(|store| async move {
        let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-row-native-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/finite-agent-runtime:row-native-v1@sha256:{}",
                    "1".repeat(64)
                ),
                version_label: "row-native-v1".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: Some("python:3.11-trixie".to_string()),
                canary_runtime_id: None,
                recover_known_good_chat: false,
                promoted: true,
                now: Some("2026-05-28T12:00:00Z".to_string()),
            })
            .await
            .unwrap();

        let create = RequestAgentCreationInput {
            verified_email: "row-native@finite.vip".to_string(),
            workos_user_id: "workos_row_native".to_string(),
            display_name: "Row Native Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "browser-submit-row-native".to_string(),
            now: Some("2026-05-28T12:01:00Z".to_string()),
        };
        let (first, second) = tokio::join!(
            store.request_agent_creation(create.clone()),
            store.request_agent_creation(create)
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(first.request.id, second.request.id);
        assert!(first.reused ^ second.reused);

        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-row-native-1".to_string(),
                source_host_id: None,
                lease_token: "lease-row-native-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some("2026-05-28T12:02:00Z".to_string()),
            })
            .await
            .unwrap()
            .expect("row-native request should lease");
        assert_eq!(lease.request.status, AgentCreationRequestStatus::Launching);

        let visible_before = store
            .visible_projects_for_workos_user("workos_row_native")
            .await
            .unwrap();
        assert_eq!(visible_before.len(), 1);
        assert!(visible_before[0].runtime.is_none());

        let provisioned = store
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-row-native-1".to_string(),
                lease_token: "lease-row-native-1".to_string(),
                source_host_id: Some("row-native-host".to_string()),
                source_machine_id: Some("row-native-agent-001".to_string()),
                now: Some("2026-05-28T12:02:15Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(provisioned.grant.status, FinitePrivateGrantStatus::Active);
        assert_eq!(
            provisioned.api_key.status,
            FinitePrivateApiKeyStatus::Active
        );

        store
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-row-native-1".to_string(),
                lease_token: "lease-row-native-1".to_string(),
                source_host_id: "row-native-host".to_string(),
                source_machine_id: "row-native-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-row-native-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Row Native Agent".to_string()),
                hostname: None,
                runtime_host: Some("row-native-host".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: Some("2026-05-28T12:02:30Z".to_string()),
            })
            .await
            .unwrap();

        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-row-native-1".to_string(),
                lease_token: "lease-row-native-1".to_string(),
                source_host_id: "row-native-host".to_string(),
                source_machine_id: "row-native-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-row-native-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Row Native Agent".to_string()),
                hostname: None,
                runtime_host: Some("row-native-host".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: Some("2026-05-28T12:03:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(
            completed.request.status,
            AgentCreationRequestStatus::Running
        );

        let visible_after = store
            .visible_projects_for_workos_user("workos_row_native")
            .await
            .unwrap();
        assert_eq!(visible_after.len(), 1);
        assert_eq!(
            visible_after[0].runtime.as_ref().unwrap().source_machine_id,
            "row-native-agent-001"
        );
        let requests = store
            .agent_creation_requests_for_workos_user("workos_row_native")
            .await
            .unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].status, AgentCreationRequestStatus::Running);
    })
    .await;
}
