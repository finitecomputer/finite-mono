use super::*;

/// Unrecoverable archive is fail-closed on every guard, and a successful
/// archive retains history.
///
/// Each rejected attempt leaves state untouched, so they run in sequence
/// against one database instead of forking an in-memory snapshot.
#[tokio::test]
async fn cold_relocation_is_stopped_exact_targeted_and_failure_preserves_source_runtime() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "canary@finite.vip",
            "workos-canary",
            "canary-create",
            "finite-kata-canary",
            "artifact-v1",
            "2026-05-25T13:00:00Z",
        )
        .await;
        let project_id = db
            .agent_runtime(&runtime_id)
            .await
            .unwrap()
            .project_id
            .clone();
        let running = db
            .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: "oslo-host-1".to_string(),
                expected_source_machine_id: "finite-kata-canary".to_string(),
                target_source_host_id: "oslo-host-3".to_string(),
                expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                durable_state_manifest_sha256: "b".repeat(64),
                operator_observed_compute_absent: false,
                now: Some("2026-05-25T13:01:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(running, CoreError::RuntimeControlUnsupported));

        // Stopping is the precondition for relocation, not the subject of
        // this test. The owner path reaches the same state; the admin
        // variant was an in-memory-only helper.
        let stop = db
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: "canary@finite.vip".to_string(),
                workos_user_id: "workos-canary".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .await
            .unwrap();
        let capacity = RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Kata],
            runtime_capabilities: Some(kata_runtime_capabilities()),
            ..RunnerLeaseCapacity::default()
        };
        let stop_lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "stop-lease".to_string(),
                lease_seconds: Some(300),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(capacity.clone()),
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stop_lease.request.id, stop.id);
        db.complete_runtime_control_request(CompleteRuntimeControlRequestInput {
            request_id: stop.id,
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "stop-lease".to_string(),
            runtime_artifact_id: None,
            state_schema_version: None,
            runtime_capabilities: None,
            runtime_host: None,
            published_app_urls: None,
            retirement_snapshot: None,
            now: Some("2026-05-25T13:04:00Z".to_string()),
        })
        .await
        .unwrap();

        let relocation = db
            .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: "oslo-host-1".to_string(),
                expected_source_machine_id: "finite-kata-canary".to_string(),
                target_source_host_id: "oslo-host-3".to_string(),
                expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                durable_state_manifest_sha256: "b".repeat(64),
                operator_observed_compute_absent: false,
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(
            relocation.target_source_host_id.as_deref(),
            Some("oslo-host-3")
        );
        assert_eq!(
            relocation.agent_runtime_id.as_deref(),
            Some(runtime_id.as_str())
        );
        assert!(relocation.relocation.is_some());

        assert!(
            db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "wrong-host".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(capacity.clone()),
                source_host_id: Some("oslo-host-1".to_string()),
                now: Some("2026-05-25T13:06:00Z".to_string()),
            })
            .await
            .unwrap()
            .is_none()
        );
        let lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-3".to_string(),
                lease_token: "relocate-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(capacity),
                source_host_id: Some("oslo-host-3".to_string()),
                now: Some("2026-05-25T13:06:00Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease.request.id, relocation.id);
        db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
            request_id: relocation.id.clone(),
            runner_id: "runner-oslo-3".to_string(),
            lease_token: "relocate-lease".to_string(),
            source_host_id: "oslo-host-3".to_string(),
            source_machine_id: "finite-kata-canary".to_string(),
            runtime_artifact_id: Some("artifact-v1".to_string()),
            state_schema_version: Some("db-v1".to_string()),
            provider_runtime_handle: None,
            contact_endpoint: Some("http://oslo-host-3:4201/contact".to_string()),
            runtime_capabilities: Some(kata_runtime_capabilities()),
            display_name: None,
            hostname: None,
            runtime_host: Some("http://oslo-host-3:4201".to_string()),
            runtime_status: Some(RuntimeSummaryStatus::Unknown),
            active_inference_profile: Some("finite-private".to_string()),
            hermes_available: Some(true),
            published_app_urls: Vec::new(),
            now: Some("2026-05-25T13:06:30Z".to_string()),
        })
        .await
        .unwrap();
        let still_source = db.agent_runtime(&runtime_id).await.unwrap();
        assert_eq!(still_source.source_host_id, "oslo-host-1");
        assert_eq!(
            still_source.host_facts.runtime_status,
            RuntimeSummaryStatus::Offline
        );
        db.fail_agent_creation_request(FailAgentCreationRequestInput {
            request_id: relocation.id.clone(),
            runner_id: "runner-oslo-3".to_string(),
            lease_token: "relocate-lease".to_string(),
            failure_message: "synthetic target launch failure".to_string(),
            provisioned_finite_private_api_key_id: None,
            now: Some("2026-05-25T13:07:00Z".to_string()),
        })
        .await
        .unwrap();
        let source = db.agent_runtime(&runtime_id).await.unwrap();
        assert_eq!(source.source_host_id, "oslo-host-1");
        assert_eq!(
            source.host_facts.runtime_status,
            RuntimeSummaryStatus::Offline
        );
        assert_eq!(
            db.agent_creation_request(&relocation.id)
                .await
                .unwrap()
                .agent_runtime_id
                .as_deref(),
            Some(runtime_id.as_str())
        );
        assert!(db.all("project_runtime_links").await.iter().any(|link| {
            link["project_id"] == project_id.as_str()
                && link["agent_runtime_id"] == runtime_id.as_str()
                && link["active"] == true
        }));
    })
    .await;
}

#[tokio::test]
async fn cold_relocation_with_absent_compute_accepts_stale_and_waives_stop_receipt() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "eddie-case@finite.vip",
            "workos-eddie-case",
            "absent-create",
            "finite-kata-absent",
            "artifact-v1",
            "2026-08-12T13:00:00Z",
        )
        .await;
        let project_id = db
            .agent_runtime(&runtime_id)
            .await
            .unwrap()
            .project_id
            .clone();
        let relocate_input = |absent: bool, now: &str| AdminRuntimeRelocateExactInput {
            admin_verified_email: "admin@finite.vip".to_string(),
            admin_workos_user_id: "workos-admin".to_string(),
            project_id: project_id.clone(),
            expected_agent_runtime_id: runtime_id.clone(),
            expected_source_host_id: "oslo-host-1".to_string(),
            expected_source_machine_id: "finite-kata-absent".to_string(),
            target_source_host_id: "oslo-host-3".to_string(),
            expected_agent_npub: format!("npub1{}", "q".repeat(58)),
            durable_state_manifest_sha256: "c".repeat(64),
            operator_observed_compute_absent: absent,
            now: Some(now.to_string()),
        };

        // Without the attestation an online runtime may still be
        // running, so the exact relocation must refuse; the
        // attested-online acceptance and its survival of the
        // registration-time validation are pinned by the sibling
        // test below.
        let online = db
            .admin_request_runtime_relocate_exact(relocate_input(false, "2026-08-12T13:01:00Z"))
            .await
            .unwrap_err();
        assert!(matches!(online, CoreError::RuntimeControlUnsupported));

        // Reach `stale` the way absent compute does in production: a
        // control request that fails at the provider.
        let restart = db
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "eddie-case@finite.vip".to_string(),
                workos_user_id: "workos-eddie-case".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-08-12T13:02:00Z".to_string()),
            })
            .await
            .unwrap();
        let capacity = RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Kata],
            runtime_capabilities: Some(kata_runtime_capabilities()),
            ..RunnerLeaseCapacity::default()
        };
        let restart_lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease".to_string(),
                lease_seconds: Some(300),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(capacity),
                now: Some("2026-08-12T13:03:00Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restart_lease.request.id, restart.id);
        db.fail_runtime_control_request(FailRuntimeControlRequestInput {
            request_id: restart.id,
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "restart-lease".to_string(),
            failure_message: "no such object finite-kata-absent".to_string(),
            failure_stage: Some(RuntimeLifecycleStage::Compute),
            now: Some("2026-08-12T13:04:00Z".to_string()),
        })
        .await
        .unwrap();
        assert_eq!(
            db.agent_runtime(&runtime_id)
                .await
                .unwrap()
                .host_facts
                .runtime_status,
            RuntimeSummaryStatus::Stale
        );
        let stale_overview = db
            .admin_runtime_overviews()
            .await
            .unwrap()
            .into_iter()
            .find(|overview| overview.agent_runtime_id == runtime_id)
            .unwrap();
        assert_eq!(
            stale_overview.lifecycle_status,
            db.agent_runtime(&runtime_id)
                .await
                .unwrap()
                .host_facts
                .runtime_status
        );

        // Without the attestation, `stale` (and the missing stop
        // receipt) keep refusing — the existing posture is pinned.
        let unattested = db
            .admin_request_runtime_relocate_exact(relocate_input(false, "2026-08-12T13:05:00Z"))
            .await
            .unwrap_err();
        assert!(matches!(unattested, CoreError::RuntimeControlUnsupported));

        // With it, the enqueue succeeds despite `stale` and despite the
        // runtime never having a succeeded stop, and the attestation
        // rides the envelope for lease-time validation.
        let relocation = db
            .admin_request_runtime_relocate_exact(relocate_input(true, "2026-08-12T13:06:00Z"))
            .await
            .unwrap();
        assert_eq!(
            relocation.agent_runtime_id.as_deref(),
            Some(runtime_id.as_str())
        );
        assert_eq!(
            relocation.target_source_host_id.as_deref(),
            Some("oslo-host-3")
        );
        let envelope = relocation.relocation.as_ref().unwrap().v1();
        assert!(envelope.source_compute_absent);
        assert_eq!(envelope.source_machine_id, "finite-kata-absent");
    })
    .await;
}

#[tokio::test]
async fn cold_relocation_with_absent_compute_accepts_attested_online_through_registration() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "online-case@finite.vip",
            "workos-online-case",
            "absent-online-create",
            "finite-kata-absent-online",
            "artifact-v1",
            "2026-08-12T14:00:00Z",
        )
        .await;
        let project_id = db
            .agent_runtime(&runtime_id)
            .await
            .unwrap()
            .project_id
            .clone();
        let relocate_input = |absent: bool, now: &str| AdminRuntimeRelocateExactInput {
            admin_verified_email: "admin@finite.vip".to_string(),
            admin_workos_user_id: "workos-admin".to_string(),
            project_id: project_id.clone(),
            expected_agent_runtime_id: runtime_id.clone(),
            expected_source_host_id: "oslo-host-1".to_string(),
            expected_source_machine_id: "finite-kata-absent-online".to_string(),
            target_source_host_id: "oslo-host-3".to_string(),
            expected_agent_npub: format!("npub1{}", "q".repeat(58)),
            durable_state_manifest_sha256: "c".repeat(64),
            operator_observed_compute_absent: absent,
            now: Some(now.to_string()),
        };

        // Without the attestation the pre-death `online` report is not
        // frozen: the runtime may still be running, so refuse.
        let unattested = db
            .admin_request_runtime_relocate_exact(relocate_input(false, "2026-08-12T14:01:00Z"))
            .await
            .unwrap_err();
        assert!(matches!(unattested, CoreError::RuntimeControlUnsupported));

        // Under the attestation `online` is exactly as frozen as the
        // attested `stale` in the test above: the attestation rides the
        // envelope for the target runner's lease-time and
        // registration/completion-time validation.
        let relocation = db
            .admin_request_runtime_relocate_exact(relocate_input(true, "2026-08-12T14:02:00Z"))
            .await
            .unwrap();
        assert_eq!(
            relocation.agent_runtime_id.as_deref(),
            Some(runtime_id.as_str())
        );
        assert_eq!(
            relocation.target_source_host_id.as_deref(),
            Some("oslo-host-3")
        );
        let envelope = relocation.relocation.as_ref().unwrap().v1();
        assert!(envelope.source_compute_absent);
        assert_eq!(envelope.source_machine_id, "finite-kata-absent-online");

        let capacity = RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Kata],
            runtime_capabilities: Some(kata_runtime_capabilities()),
            ..RunnerLeaseCapacity::default()
        };
        assert!(
            db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "wrong-host".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(capacity.clone()),
                source_host_id: Some("oslo-host-1".to_string()),
                now: Some("2026-08-12T14:03:00Z".to_string()),
            })
            .await
            .unwrap()
            .is_none()
        );
        let lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-3".to_string(),
                lease_token: "relocate-online-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(capacity),
                source_host_id: Some("oslo-host-3".to_string()),
                now: Some("2026-08-12T14:03:30Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease.request.id, relocation.id);

        // Registration on the target re-validates the envelope while the
        // source is still `online`: only the recorded attestation keeps
        // that status frozen. Registration stays non-mutating, so the
        // source binding survives untouched.
        db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
            request_id: relocation.id.clone(),
            runner_id: "runner-oslo-3".to_string(),
            lease_token: "relocate-online-lease".to_string(),
            source_host_id: "oslo-host-3".to_string(),
            source_machine_id: "finite-kata-absent-online".to_string(),
            runtime_artifact_id: Some("artifact-v1".to_string()),
            state_schema_version: Some("db-v1".to_string()),
            provider_runtime_handle: None,
            contact_endpoint: Some("http://oslo-host-3:4201/contact".to_string()),
            runtime_capabilities: Some(kata_runtime_capabilities()),
            display_name: None,
            hostname: None,
            runtime_host: Some("http://oslo-host-3:4201".to_string()),
            runtime_status: Some(RuntimeSummaryStatus::Unknown),
            active_inference_profile: Some("finite-private".to_string()),
            hermes_available: Some(true),
            published_app_urls: Vec::new(),
            now: Some("2026-08-12T14:04:00Z".to_string()),
        })
        .await
        .unwrap();
        let still_source = db.agent_runtime(&runtime_id).await.unwrap();
        assert_eq!(still_source.source_host_id, "oslo-host-1");
        assert_eq!(
            still_source.host_facts.runtime_status,
            RuntimeSummaryStatus::Online
        );

        // Completion re-validates the same envelope a second time and is
        // the single transaction that replaces the source binding.
        let completed = db
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: relocation.id.clone(),
                runner_id: "runner-oslo-3".to_string(),
                lease_token: "relocate-online-lease".to_string(),
                source_host_id: "oslo-host-3".to_string(),
                source_machine_id: "finite-kata-absent-online".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: Some("db-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://oslo-host-3:4201/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: Some("http://oslo-host-3:4201".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: Some("2026-08-12T14:05:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(
            completed.request.status,
            AgentCreationRequestStatus::Running
        );
        let relocated = db.agent_runtime(&runtime_id).await.unwrap();
        assert_eq!(relocated.source_host_id, "oslo-host-3");
        assert_eq!(
            relocated.host_facts.runtime_status,
            RuntimeSummaryStatus::Online
        );
    })
    .await;
}
