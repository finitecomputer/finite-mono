use super::*;

#[tokio::test]
async fn postgres_admin_ops_runtime_overview_and_finite_private_lifecycle() {
    with_isolated_postgres(|store| async move {
        let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
        // Unique-per-run identifiers keep this test idempotent against an
        // accumulating test database.
        let run = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let owner_email = format!("admin-ops-owner-{run}@finite.vip");
        let admin_email = format!("admin-ops-admin-{run}@finite.vip");
        let friend_email = format!("admin-ops-friend-{run}@finite.vip");
        let machine_id = format!("admin-ops-agent-{run}");

        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-admin-ops-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/finite-agent-runtime:admin-ops-v1@sha256:{}",
                    "2".repeat(64)
                ),
                version_label: "admin-ops-v1".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: Some("python:3.11-trixie".to_string()),
                recover_known_good_chat: false,
                promoted: true,
                now: None,
            })
            .await
            .unwrap();

        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: owner_email.clone(),
                workos_user_id: format!("workos_admin_ops_owner_{run}"),
                display_name: "Admin Ops Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: format!("admin-ops-{run}"),
                now: None,
            })
            .await
            .unwrap();
        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: format!("runner-admin-ops-{run}"),
                source_host_id: None,
                lease_token: format!("lease-admin-ops-{run}"),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: None,
            })
            .await
            .unwrap()
            .expect("admin ops request should lease");
        assert_eq!(lease.request.id, created.request.id);
        let provisioned_owner_key = store
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: format!("runner-admin-ops-{run}"),
                lease_token: format!("lease-admin-ops-{run}"),
                source_host_id: Some("admin-ops-host".to_string()),
                source_machine_id: Some(machine_id.clone()),
                now: None,
            })
            .await
            .unwrap();
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: format!("runner-admin-ops-{run}"),
                lease_token: format!("lease-admin-ops-{run}"),
                source_host_id: "admin-ops-host".to_string(),
                source_machine_id: machine_id.clone(),
                runtime_artifact_id: Some("artifact-admin-ops-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Admin Ops Agent".to_string()),
                hostname: None,
                runtime_host: Some("admin-ops-host".to_string()),
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

        // Provisioned-boxes overview reads back through Postgres state.
        let overviews = store.admin_runtime_overviews().await.unwrap();
        let overview = overviews
            .iter()
            .find(|overview| overview.agent_runtime_id == runtime_id)
            .expect("new runtime should appear in the admin overview");
        assert_eq!(overview.project_id, project_id);
        assert_eq!(overview.owner_email.as_deref(), Some(owner_email.as_str()));
        assert_eq!(
            overview.runtime_artifact_version_label.as_deref(),
            Some("admin-ops-v1")
        );
        assert_eq!(
            overview.runtime_capabilities,
            Some(*kata_runtime_capabilities().v1())
        );
        assert!(overview.runtime_link_active);
        let owner_account = store
            .finite_private_admin_state()
            .await
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.email == owner_email)
            .expect("provisioned owner should have a correlated Finite Private account");
        assert_eq!(owner_account.grant.id, provisioned_owner_key.grant.id);
        assert!(owner_account.projects.iter().any(|project| {
            project.id == project_id
                && project.agent_runtime_id.as_deref() == Some(runtime_id.as_str())
        }));

        // Admin restart persists a leasable control request.
        let restart = store
            .admin_request_runtime_restart(AdminRuntimeControlInput {
                admin_verified_email: admin_email.clone(),
                admin_workos_user_id: format!("workos_admin_ops_admin_{run}"),
                project_id: project_id.clone(),
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(restart.agent_runtime_id, runtime_id);
        let control_lease = store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: format!("runner-admin-ops-{run}"),
                lease_token: format!("control-lease-{run}"),
                lease_seconds: Some(60),
                source_host_id: Some("admin-ops-host".to_string()),
                runner_capacity: Some(crate::RunnerLeaseCapacity {
                    runner_classes: vec![crate::RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..crate::RunnerLeaseCapacity::default()
                }),
                now: None,
            })
            .await
            .unwrap()
            .expect("admin restart should lease");
        assert_eq!(control_lease.request.id, restart.id);
        store
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: restart.id.clone(),
                runner_id: format!("runner-admin-ops-{run}"),
                lease_token: format!("control-lease-{run}"),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: None,
            })
            .await
            .unwrap();

        // Friend key issue, rotate, and window reset persist round trips.
        let raw_key = format!("fpk_live_admin_ops_test_{run}");
        let issued = store
            .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                admin_verified_email: admin_email.clone(),
                friend_email: friend_email.clone(),
                limit_profile_id: None,
                raw_key: raw_key.clone(),
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(issued.grant.status, FinitePrivateGrantStatus::Active);
        assert_eq!(issued.api_key.status, FinitePrivateApiKeyStatus::Active);
        assert_ne!(issued.api_key.key_hash, raw_key);

        let usage = store
            .finite_private_usage_status_for_api_key(
                &raw_key,
                true,
                Some("2026-07-21T12:00:00Z".to_string()),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(usage.burst_limit_units, 100_000_000);
        assert!(usage.free_daily_reset_available);
        let daily_reset = store
            .claim_finite_private_daily_reset_for_api_key(
                &raw_key,
                Some("2026-07-21T12:01:00Z".to_string()),
            )
            .await
            .unwrap();
        assert!(daily_reset.performed);
        let repeated_reset = store
            .claim_finite_private_daily_reset_for_api_key(
                &raw_key,
                Some("2026-07-21T12:02:00Z".to_string()),
            )
            .await
            .unwrap();
        assert!(!repeated_reset.performed);

        let before_assignment = store
            .finite_private_admin_state()
            .await
            .unwrap()
            .accounts
            .into_iter()
            .find(|account| account.email == friend_email)
            .unwrap()
            .grant;
        let assigned = store
            .admin_assign_finite_private_limit_profile(AdminAssignFinitePrivateLimitProfileInput {
                admin_verified_email: admin_email.clone(),
                grant_id: issued.grant.id.clone(),
                limit_profile_id: crate::FINITE_PRIVATE_5X_LIMIT_PROFILE.to_string(),
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(
            assigned.limit_profile_id,
            crate::FINITE_PRIVATE_5X_LIMIT_PROFILE
        );
        assert_eq!(
            assigned.current_window_used_units,
            before_assignment.current_window_used_units
        );
        assert_eq!(
            assigned.burst_window_epoch,
            before_assignment.burst_window_epoch
        );
        let assigned_usage = store
            .finite_private_usage_status_for_api_key(
                &raw_key,
                true,
                Some("2026-07-21T12:03:00Z".to_string()),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(assigned_usage.burst_limit_units, 500_000_000);

        let rotated = store
            .admin_rotate_finite_private_api_key(AdminRotateFinitePrivateApiKeyInput {
                admin_verified_email: admin_email.clone(),
                key_id: issued.api_key.id.clone(),
                raw_key: format!("fpk_live_admin_ops_rotated_{run}"),
                now: None,
            })
            .await
            .unwrap();
        assert_ne!(rotated.id, issued.api_key.id);

        let admin_state = store.finite_private_admin_state().await.unwrap();
        let account = admin_state
            .accounts
            .iter()
            .find(|account| account.email == friend_email)
            .unwrap();
        assert_eq!(account.grant.id, issued.grant.id);
        assert_eq!(account.api_keys.len(), 2);
        assert!(admin_state.profiles.iter().any(|profile| {
            profile.id == crate::FINITE_PRIVATE_5X_LIMIT_PROFILE
                && profile.burst_limit_units == 500_000_000
        }));
        let old_key = admin_state
            .api_keys
            .iter()
            .find(|key| key.id == issued.api_key.id)
            .unwrap();
        assert_eq!(old_key.status, FinitePrivateApiKeyStatus::Revoked);
        let new_key = admin_state
            .api_keys
            .iter()
            .find(|key| key.id == rotated.id)
            .unwrap();
        assert_eq!(new_key.status, FinitePrivateApiKeyStatus::Active);

        let revoked = store
            .admin_revoke_finite_private_api_key(AdminRevokeFinitePrivateApiKeyInput {
                admin_verified_email: admin_email.clone(),
                key_id: rotated.id.clone(),
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(revoked.status, FinitePrivateApiKeyStatus::Revoked);

        let reset = store
            .admin_reset_finite_private_usage_window(AdminResetFinitePrivateUsageWindowInput {
                admin_verified_email: admin_email.clone(),
                grant_id: issued.grant.id.clone(),
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(reset.current_window_used_units, 0);
        assert!(reset.current_window_started_at.is_some());

        // Every admin action is durably audited with the admin actor.
        let events = store.finite_private_admin_audit_events().await.unwrap();
        let admin_actions = events
            .iter()
            .filter(|event| event.actor == admin_email)
            .map(|event| event.action.clone())
            .collect::<Vec<_>>();
        for expected in [
            "runtime.admin_restart",
            "finite_private.friend_key.admin_issue",
            "finite_private.api_key.admin_rotate",
            "finite_private.api_key.admin_revoke",
            "finite_private.grant.admin_window_reset",
            "finite_private.grant.admin_assign_limit_profile",
        ] {
            assert!(
                admin_actions.contains(&expected.to_string()),
                "missing Postgres audit action {expected}"
            );
        }

        let archive_input = |compute_absent| AdminArchiveUnrecoverableRuntimeInput {
            admin_verified_email: admin_email.clone(),
            admin_workos_user_id: format!("workos_admin_ops_admin_{run}"),
            project_id: project_id.clone(),
            expected_agent_runtime_id: runtime_id.clone(),
            expected_source_host_id: "admin-ops-host".to_string(),
            expected_source_machine_id: machine_id.clone(),
            expected_owner_email: owner_email.clone(),
            operator_observed_compute_absent: compute_absent,
            operator_observed_durable_state_absent: true,
            owner_acknowledged_unrecoverable: true,
            now: None,
        };
        assert!(matches!(
            store
                .admin_archive_unrecoverable_runtime(archive_input(false))
                .await
                .unwrap_err(),
            CoreError::UnrecoverableRuntimeArchiveAcknowledgementRequired
        ));
        let archive = store
            .admin_archive_unrecoverable_runtime(archive_input(true))
            .await
            .unwrap();
        assert_eq!(archive.agent_runtime_id, runtime_id);
        let archived_overview = store
            .admin_runtime_overviews()
            .await
            .unwrap()
            .into_iter()
            .find(|overview| overview.agent_runtime_id == runtime_id)
            .unwrap();
        assert!(!archived_overview.runtime_link_active);
        assert!(
            store
                .finite_private_admin_audit_events()
                .await
                .unwrap()
                .iter()
                .any(|event| {
                    event.action == "runtime.admin_archive_unrecoverable"
                        && event.actor == admin_email
                        && event.target_id == runtime_id
                })
        );
    })
    .await;
}
