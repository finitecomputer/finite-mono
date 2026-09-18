use super::*;

/// The destroy lifecycle records every phase forward: enqueue writes
/// retirement_requested, and one completion transaction carries the
/// runtime through receipt_verified, compute_removed, and
/// link_deactivated to the terminal archived. The phase never regresses:
/// a backward write fails closed and names both phases, while restating
/// the recorded phase is an idempotent no-op for replayed completions.
#[tokio::test]
async fn postgres_destroy_completion_records_forward_only_offboarding_phases() {
    with_isolated_postgres(|store| async move {
        let run = "phase-forward";
        let host = "phase-forward-host";
        let (project_id, runtime_id, destroy, durable_state_id, spec_artifact_id) =
            stage_retirement_in_flight(&store, run, host).await;

        // Enqueueing the destroy recorded the first phase.
        assert_eq!(
            offboarding_phase_of(&store, &runtime_id).await,
            json!("retirement_requested")
        );

        // A duplicate request dedupes to the same destroy and keeps the
        // phase.
        let deduped = store
            .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                admin_verified_email: format!("{run}-admin@finite.vip"),
                admin_workos_user_id: format!("workos_{run}_admin"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: format!("{run}-agent-001"),
                now: Some("2026-07-21T12:01:20Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(deduped.id, destroy.id);
        assert_eq!(
            offboarding_phase_of(&store, &runtime_id).await,
            json!("retirement_requested")
        );

        let receipt = RuntimeRetirementSnapshotReceipt {
            schema: crate::RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA.to_string(),
            request_id: destroy.id.clone(),
            project_id: project_id.clone(),
            agent_runtime_id: runtime_id.clone(),
            durable_state_id,
            runtime_artifact_id: spec_artifact_id,
            backend: crate::RUNTIME_RETIREMENT_BACKEND_BORG.to_string(),
            locator: crate::runtime_retirement_archive_locator(&destroy.id),
            zip_bytes: 8192,
            zip_sha256: "a".repeat(64),
            manifest_sha256: "b".repeat(64),
            created_at: "2026-07-21T12:02:00Z".to_string(),
            verified_at: "2026-07-21T12:03:00Z".to_string(),
            recovery_authority_id: "finite-assisted-test".to_string(),
            retention_policy: crate::RUNTIME_RETIREMENT_RETENTION_INDEFINITE.to_string(),
        };
        let completion = CompleteRuntimeControlRequestInput {
            request_id: destroy.id.clone(),
            runner_id: format!("runner-{run}"),
            lease_token: format!("ctl-destroy-{run}"),
            runtime_artifact_id: None,
            state_schema_version: None,
            runtime_capabilities: None,
            runtime_host: None,
            published_app_urls: None,
            retirement_snapshot: Some(receipt),
            now: Some("2026-07-21T12:04:00Z".to_string()),
        };
        store
            .complete_runtime_control_request(completion.clone())
            .await
            .unwrap();

        // One transaction carried the runtime to the terminal phase.
        assert_eq!(
            offboarding_phase_of(&store, &runtime_id).await,
            json!("archived")
        );
        assert!(
            store
                .active_runtime_for_project(&project_id)
                .await
                .is_none()
        );

        // A replayed identical completion stays idempotent and terminal.
        store
            .complete_runtime_control_request(completion)
            .await
            .expect("identical completion replay must be idempotent");
        assert_eq!(
            offboarding_phase_of(&store, &runtime_id).await,
            json!("archived")
        );

        // The phase never moves backward; restating it is a no-op.
        let client = store.connection().await.unwrap();
        let regression = set_offboarding_phase(
            &**client,
            &runtime_id,
            OffboardingPhase::RetirementRequested,
            "2026-07-21T12:05:00Z",
        )
        .await
        .unwrap_err();
        assert!(matches!(
            regression,
            CoreError::OffboardingPhaseRegression {
                current: OffboardingPhase::Archived,
                attempted: OffboardingPhase::RetirementRequested,
            }
        ));
        set_offboarding_phase(
            &**client,
            &runtime_id,
            OffboardingPhase::Archived,
            "2026-07-21T12:05:00Z",
        )
        .await
        .expect("restating the recorded phase must be an idempotent no-op");
        drop(client);
        assert_eq!(
            offboarding_phase_of(&store, &runtime_id).await,
            json!("archived")
        );
    })
    .await;
}

/// The half-retired ghost from the audit: the destroy stored a verified
/// receipt and removed compute under a pre-phase-machine Core, but the
/// offboarding never ran (link still active). The 0020 backfill classifies
/// the row from its legacy flags, `runtime-retire-exact` resumes from the
/// recorded phase instead of minting a new destroy (the uncapped retry
/// wedge is unrepresentable), and `runtime-offboard-retired-exact`
/// completes the offboarding through the same phase machine.
#[tokio::test]
async fn postgres_retire_exact_resumes_a_partially_retired_runtime_from_its_phase() {
    with_isolated_postgres(|store| async move {
        let run = "phase-resume";
        let host = "phase-resume-host";
        let owner_email = format!("{run}-owner@finite.vip");
        let admin_email = format!("{run}-admin@finite.vip");
        let machine_id = format!("{run}-agent-001");
        let (project_id, runtime_id, destroy, durable_state_id, spec_artifact_id) =
            stage_retirement_in_flight(&store, run, host).await;

        // The legacy ghost: the destroy succeeded and its verified receipt
        // is stored, but the offboarding never ran. Clearing the phase
        // simulates a row written before the phase column existed.
        store
            .exec(&format!(
                "UPDATE runtime_control_requests \
                     SET status = 'succeeded', lease_token = NULL, lease_expires_at = NULL, \
                         completed_at = CURRENT_TIMESTAMP \
                     WHERE id = '{}'",
                destroy.id
            ))
            .await;
        store
            .exec(&format!(
                "INSERT INTO runtime_retirement_snapshots (
                       request_id, project_id, agent_runtime_id, durable_state_id,
                       runtime_artifact_id, schema_version, backend, locator,
                       zip_bytes, zip_sha256, manifest_sha256, created_at,
                       verified_at, recovery_authority_id, retention_policy, stored_at
                     ) VALUES (
                       '{}', '{}', '{}', '{}',
                       '{}', 'runtime_retirement_snapshot.v1', 'borg', '{}',
                       8192, '{}', '{}', '2026-07-21T12:02:00Z',
                       '2026-07-21T12:03:00Z', 'finite-assisted-test',
                       'indefinite_until_purge', CURRENT_TIMESTAMP
                     )",
                destroy.id,
                project_id,
                runtime_id,
                durable_state_id,
                spec_artifact_id,
                crate::runtime_retirement_archive_locator(&destroy.id),
                "a".repeat(64),
                "b".repeat(64),
            ))
            .await;
        store
            .exec(&format!(
                "UPDATE agent_runtimes SET offboarding_phase = NULL WHERE id = '{runtime_id}'"
            ))
            .await;

        // Re-applying the migration maps the legacy flags exactly once:
        // receipt stored plus an active link is compute_removed.
        store
            .exec(include_str!(
                "../../../migrations/0020_runtime_offboarding_phases.sql"
            ))
            .await;
        assert_eq!(
            offboarding_phase_of(&store, &runtime_id).await,
            json!("compute_removed")
        );

        // runtime-retire-exact resumes from the recorded phase: it refuses
        // to mint a new destroy and names the resume point instead of
        // looping against the absent container.
        let resume = store
            .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                admin_verified_email: admin_email.clone(),
                admin_workos_user_id: format!("workos_{run}_admin"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: machine_id.clone(),
                now: Some("2026-07-21T12:06:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            resume,
            CoreError::RuntimeOffboardingResumeRequired {
                phase: OffboardingPhase::ComputeRemoved,
            }
        ));
        assert_eq!(store.all_runtime_control_requests().await.len(), 1);

        // The owner-facing destroy path is gated by the same phase.
        let owner_resume = store
            .request_runtime_destroy(RequestRuntimeDestroyInput {
                verified_email: owner_email.clone(),
                workos_user_id: format!("workos_{run}_owner"),
                project_id: project_id.clone(),
                now: Some("2026-07-21T12:06:30Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            owner_resume,
            CoreError::RuntimeOffboardingResumeRequired {
                phase: OffboardingPhase::ComputeRemoved,
            }
        ));
        assert_eq!(store.all_runtime_control_requests().await.len(), 1);

        // The resume command completes the offboarding boundary and the
        // phase machine records the terminal archived phase.
        let receipt = store
            .admin_offboard_retired_runtime(AdminOffboardRetiredRuntimeInput {
                admin_verified_email: admin_email.clone(),
                admin_workos_user_id: format!("workos_{run}_admin"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: machine_id.clone(),
                expected_owner_email: owner_email.clone(),
                operator_observed_compute_absent: true,
                now: Some("2026-07-21T12:07:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(receipt.retirement_request_id, destroy.id);
        assert_eq!(
            offboarding_phase_of(&store, &runtime_id).await,
            json!("archived")
        );
        assert!(
            store
                .active_runtime_for_project(&project_id)
                .await
                .is_none()
        );

        // A rerun fails closed and the terminal phase is untouched.
        assert!(matches!(
            store
                .admin_offboard_retired_runtime(AdminOffboardRetiredRuntimeInput {
                    admin_verified_email: admin_email.clone(),
                    admin_workos_user_id: format!("workos_{run}_admin"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine_id.clone(),
                    expected_owner_email: owner_email.clone(),
                    operator_observed_compute_absent: true,
                    now: Some("2026-07-21T12:08:00Z".to_string()),
                })
                .await
                .unwrap_err(),
            CoreError::ProjectRuntimeNotFound
        ));
        assert_eq!(
            offboarding_phase_of(&store, &runtime_id).await,
            json!("archived")
        );
    })
    .await;
}

/// The unrecoverable-archive boundary crosses no receipt phases, but the
/// runtime record still lands on the single terminal state.
#[tokio::test]
async fn postgres_archive_unrecoverable_records_the_terminal_phase() {
    with_isolated_postgres(|store| async move {
        let run = "phase-archive";
        let owner_email = format!("{run}-owner@finite.vip");
        let admin_email = format!("{run}-admin@finite.vip");
        let machine_id = format!("{run}-agent-001");
        let host = "phase-archive-host";
        let launch_code = issue_test_launch_code(&store, "2026-07-21T12:00:00Z").await;
        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: format!("artifact-{run}-v1"),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/finite-agent-runtime:{run}-v1@sha256:{}",
                    "7".repeat(64)
                ),
                version_label: format!("{run}-v1"),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: None,
                canary_runtime_id: None,
                recover_known_good_chat: false,
                promoted: true,
                now: None,
            })
            .await
            .unwrap();
        store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: owner_email.clone(),
                workos_user_id: format!("workos_{run}_owner"),
                display_name: format!("{run} Agent"),
                launch_code,
                idempotency_key: format!("{run}-submit"),
                now: None,
            })
            .await
            .unwrap();
        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: format!("runner-{run}"),
                source_host_id: None,
                lease_token: format!("lease-{run}"),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: None,
            })
            .await
            .unwrap()
            .expect("creation request should lease");
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: format!("runner-{run}"),
                lease_token: format!("lease-{run}"),
                source_host_id: host.to_string(),
                source_machine_id: machine_id.clone(),
                runtime_artifact_id: Some(format!("artifact-{run}-v1")),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some(format!("{run} Agent")),
                hostname: None,
                runtime_host: Some(host.to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();
        let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
        let project_id = completed.project.id.clone();
        assert_eq!(
            offboarding_phase_of(&store, &runtime_id).await,
            serde_json::Value::Null
        );

        store
            .admin_archive_unrecoverable_runtime(AdminArchiveUnrecoverableRuntimeInput {
                admin_verified_email: admin_email.clone(),
                admin_workos_user_id: format!("workos_{run}_admin"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: machine_id.clone(),
                expected_owner_email: owner_email.clone(),
                operator_observed_compute_absent: true,
                operator_observed_durable_state_absent: true,
                owner_acknowledged_unrecoverable: true,
                now: Some("2026-07-21T12:05:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(
            offboarding_phase_of(&store, &runtime_id).await,
            json!("archived")
        );
    })
    .await;
}

/// The read-only pre-deploy census query from the PR body, executed
/// verbatim against synthetic fixtures: a live runtime lands in the
/// (no receipt, no active destroy, active link) bucket and a
/// half-retired ghost in the (receipt, no active destroy, active link)
/// bucket.
#[tokio::test]
async fn postgres_offboarding_census_groups_legacy_flag_combinations() {
    with_isolated_postgres(|store| async move {
            // The half-retired ghost: the destroy succeeded with a stored
            // verified receipt, but its offboarding never ran (link active).
            let (ghost_project_id, ghost_runtime_id, ghost_destroy, _, _) =
                stage_retirement_in_flight(&store, "census-ghost", "census-ghost-host").await;
            store
                .exec(&format!(
                    "UPDATE runtime_control_requests \
                     SET status = 'succeeded', lease_token = NULL, lease_expires_at = NULL, \
                         completed_at = CURRENT_TIMESTAMP \
                     WHERE id = '{}'",
                    ghost_destroy.id
                ))
                .await;
            store
                .exec(&format!(
                    "INSERT INTO runtime_retirement_snapshots (
                       request_id, project_id, agent_runtime_id, durable_state_id,
                       runtime_artifact_id, schema_version, backend, locator,
                       zip_bytes, zip_sha256, manifest_sha256, created_at,
                       verified_at, recovery_authority_id, retention_policy, stored_at
                     ) VALUES (
                       '{}', '{}', '{}', 'census-durable-state',
                       'artifact-census-ghost-v1', 'runtime_retirement_snapshot.v1', 'borg', '{}',
                       8192, '{}', '{}', '2026-07-21T12:02:00Z',
                       '2026-07-21T12:03:00Z', 'finite-assisted-test',
                       'indefinite_until_purge', CURRENT_TIMESTAMP
                     )",
                    ghost_destroy.id,
                    ghost_project_id,
                    ghost_runtime_id,
                    crate::runtime_retirement_archive_locator(&ghost_destroy.id),
                    "a".repeat(64),
                    "b".repeat(64),
                ))
                .await;

            // A live runtime with no offboarding evidence at all. The ghost
            // fixture already promoted the artifact this launch binds.
            let live_launch_code = issue_test_launch_code(&store, "2026-07-21T12:00:00Z").await;
            store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "census-live-owner@finite.vip".to_string(),
                    workos_user_id: "workos_census_live_owner".to_string(),
                    display_name: "Census Live Agent".to_string(),
                    launch_code: live_launch_code,
                    idempotency_key: "census-live-submit".to_string(),
                    now: None,
                })
                .await
                .unwrap();
            let live_lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-census-live".to_string(),
                    source_host_id: None,
                    lease_token: "lease-census-live".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: None,
                })
                .await
                .unwrap()
                .expect("live creation request should lease");
            store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: live_lease.request.id.clone(),
                    runner_id: "runner-census-live".to_string(),
                    lease_token: "lease-census-live".to_string(),
                    source_host_id: "census-live-host".to_string(),
                    source_machine_id: "census-live-agent-001".to_string(),
                    runtime_artifact_id: Some("artifact-census-ghost-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:41006/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Census Live Agent".to_string()),
                    hostname: None,
                    runtime_host: Some("census-live-host".to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: vec!["http://127.0.0.1:41006/contact".to_string()],
                    agent_npub: None,
                    now: None,
                })
                .await
                .unwrap();

            // The exact read-only census query shipped in the PR body.
            let client = store.connection().await.unwrap();
            let rows = client
                .query(
                    "SELECT
                       EXISTS (SELECT 1 FROM runtime_retirement_snapshots s
                               WHERE s.agent_runtime_id = r.id) AS has_verified_receipt,
                       EXISTS (SELECT 1 FROM runtime_control_requests c
                               WHERE c.agent_runtime_id = r.id
                                 AND c.kind = 'destroy'
                                 AND c.status IN ('requested', 'running')) AS destroy_request_active,
                       EXISTS (SELECT 1 FROM project_runtime_links l
                               WHERE l.agent_runtime_id = r.id AND l.active) AS link_active,
                       EXISTS (SELECT 1 FROM project_runtime_links l
                               WHERE l.agent_runtime_id = r.id) AS any_link_exists,
                       EXISTS (SELECT 1 FROM project_runtime_links l
                               WHERE l.project_id = r.project_id AND l.active) AS project_has_active_link,
                       count(*) AS runtimes
                     FROM agent_runtimes r
                     GROUP BY 1, 2, 3, 4, 5
                     ORDER BY 1, 2, 3, 4, 5",
                    &[],
                )
                .await
                .unwrap();
            let census: Vec<(bool, bool, bool, bool, bool, i64)> = rows
                .iter()
                .map(|row| {
                    (
                        row.get("has_verified_receipt"),
                        row.get("destroy_request_active"),
                        row.get("link_active"),
                        row.get("any_link_exists"),
                        row.get("project_has_active_link"),
                        row.get("runtimes"),
                    )
                })
                .collect();
            assert_eq!(
                census,
                vec![
                    // The live runtime: no offboarding evidence -> stays NULL.
                    (false, false, true, true, true, 1),
                    // The half-retired ghost: receipt verified, no active
                    // destroy, link still active -> maps to compute_removed.
                    (true, false, true, true, true, 1),
                ]
            );
        })
        .await;
}
